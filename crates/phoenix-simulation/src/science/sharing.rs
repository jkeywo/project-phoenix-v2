//! Host-neutral delivery of deliberate faction radar feeds. No transport queue.
use super::broad::{BroadProfile, BroadTracks};
use crate::core::messages::{AdmittedCommands, SystemControlPayload, SystemId};
use crate::entities::spawner::{EntityUuid, FactionComponent};
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use bevy::prelude::*;
pub(crate) use phoenix_sim_gameplay::science::sharing::projection_live;
use phoenix_sim_gameplay::science::sharing::{
    export_samples, receive_samples_live, SharedRadarSample,
};
pub use phoenix_sim_gameplay::science::sharing::{projection, ReceivedRadarSample};
use std::collections::BTreeMap;

pub(crate) fn register(app: &mut App) {
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    for kind in [
        crate::ship::system_registry::SENSOR_RADAR_KIND,
        crate::ship::system_registry::TACTICAL_RADAR_KIND,
    ] {
        app.register_admitted_consumer(ConsumerMatcher::kind(kind).with_feedback(
            FeedbackAddress::DeclaredKindOnly,
            &[crate::core::messages::SystemControlPayloadDiscriminants::SetRadarSharing],
        ));
    }
    app.add_systems(
        FixedUpdate,
        (
            apply_commands.in_set(crate::sim_sets::SimSet::Physics),
            deliver
                .in_set(crate::sim_sets::SimSet::Modifiers)
                .after(super::broad::tick),
        ),
    );
}

fn supported(kind: &str) -> bool {
    matches!(
        kind,
        crate::ship::system_registry::SENSOR_RADAR_KIND
            | crate::ship::system_registry::TACTICAL_RADAR_KIND
    )
}
fn operating(controls: &ShipSystemControlSources, owner: &SystemId) -> bool {
    controls.0.entries().any(|(id, _)| id == owner)
        && !controls.0.is_offline(owner)
        && controls.0.source_for(owner) != crate::ship::control_source::ControlSource::Offline
}

pub(crate) fn apply_commands(
    mut ships: Query<(
        &EntityUuid,
        &BroadProfile,
        &ShipConfigComponent,
        &AdmittedCommands,
        &mut BroadTracks,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    let mut ordered: Vec<_> = ships.iter_mut().collect();
    ordered.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    for (_, profiles, config, admitted, mut tracks) in ordered {
        for command in &admitted.0 {
            let SystemControlPayload::SetRadarSharing {
                share_own_faction,
                receive,
            } = &command.payload
            else {
                continue;
            };
            let valid = profiles.0.contains_key(&command.target)
                && config
                    .0
                    .system(&command.target)
                    .is_some_and(|owner| supported(&owner.kind));
            let applied = if valid {
                tracks
                    .0
                    .get_mut(&command.target)
                    .map(|state| {
                        state.share_own_faction = *share_own_faction;
                        state.receive_shared = *receive;
                    })
                    .is_some()
            } else {
                false
            };
            if let Some(feedback) = crate::command_admission::feedback::admitted_action_feedback(
                command,
                if applied {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            ) {
                if let Some(outbound) = &mut outbound {
                    outbound.write(feedback);
                }
            }
        }
    }
}

#[derive(Clone)]
struct Feed {
    sender: String,
    owner: SystemId,
    position: Vec3,
    range: f32,
    radio: Option<SystemId>,
    faction: uuid::Uuid,
    kind: String,
    samples: Vec<SharedRadarSample>,
}

type Observer = (
    Entity,
    &'static EntityUuid,
    &'static Transform,
    Option<&'static crate::ship::state::ShipPhysics>,
    &'static FactionComponent,
    &'static crate::comms::CommsRange,
    &'static ShipConfigComponent,
    &'static ShipSystemControlSources,
    &'static BroadProfile,
    &'static BroadTracks,
    Option<&'static crate::modifiers::ShipModifiers>,
    Option<&'static crate::comms::radio::RadioProfile>,
);

/// Stage all outgoing originals before mutating any recipient. Relaying cannot
/// cause same-tick flood ordering to depend on ECS spawn/archetype order.
#[allow(clippy::type_complexity)]
pub(crate) fn deliver(
    fixed: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    signals: crate::regions::signal_path::RegionSignalQuery,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut ships: ParamSet<(Query<Observer>, Query<&mut BroadTracks>)>,
) {
    let dt = fixed.timestep().as_secs_f32();
    // Broad has already processed these owners this tick. With no deliberate
    // sender, every receive would be an empty transaction at the same frontier:
    // no route renewal, receipt or allocation can occur. Avoid cloning complete
    // recipient configurations/profiles for this common default case.
    if ships
        .p0()
        .iter()
        .all(|(_, _, _, _, _, _, _, _, _, tracks, _, _)| {
            tracks
                .0
                .values()
                .all(|state| !state.share_own_faction && state.last_tick == Some(tick.0))
        })
    {
        return;
    }
    let mut feeds = Vec::new();
    let mut recipients = Vec::new();
    for (
        entity,
        uuid,
        pose,
        physics,
        faction,
        range,
        config,
        controls,
        profiles,
        tracks,
        modifiers,
        radio,
    ) in &ships.p0()
    {
        if !crate::comms::server::information_endpoint_available(config, controls, modifiers) {
            continue;
        }
        let position = super::broad::physical_position(physics, pose);
        recipients.push((
            uuid.0.clone(),
            entity,
            position,
            range.0,
            radio.map(|radio| radio.0.system.clone()),
            faction.0,
            profiles.clone(),
            config
                .0
                .systems
                .iter()
                .map(|system| (system.id.clone(), system.kind.clone()))
                .collect::<BTreeMap<_, _>>(),
            tracks
                .0
                .keys()
                .filter(|owner| operating(controls, owner))
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
        ));
        for (owner, state) in &tracks.0 {
            let Some(system) = config
                .0
                .system(owner)
                .filter(|system| supported(&system.kind))
            else {
                continue;
            };
            if !state.share_own_faction || !operating(controls, owner) {
                continue;
            }
            let Some(profile) = profiles.0.get(owner) else {
                continue;
            };
            if let Ok(samples) = export_samples(&uuid.0, state, &profile.profile, tick.0, dt) {
                feeds.push(Feed {
                    sender: uuid.0.clone(),
                    owner: owner.clone(),
                    position,
                    range: range.0,
                    radio: radio.map(|radio| radio.0.system.clone()),
                    faction: faction.0,
                    kind: system.kind.clone(),
                    samples,
                });
            }
        }
    }
    feeds.sort_by(|a, b| a.sender.cmp(&b.sender).then_with(|| a.owner.cmp(&b.owner)));
    recipients.sort_by(|a, b| a.0.cmp(&b.0));
    let mut owners = ships.p1();
    for (uuid, entity, position, range, radio, faction, profiles, kinds, operating) in recipients {
        let Ok(mut tracks) = owners.get_mut(entity) else {
            continue;
        };
        for (owner, state) in &mut tracks.0 {
            if !operating.contains(owner) {
                continue;
            }
            let Some(kind) = kinds.get(owner).filter(|kind| supported(kind)) else {
                continue;
            };
            let Some(profile) = profiles.0.get(owner) else {
                continue;
            };
            let samples = feeds
                .iter()
                .filter(|feed| {
                    feed.sender != uuid
                        && feed.faction == faction
                        && &feed.kind == kind
                        && crate::comms::server::signal_comms_in_range(
                            &signals,
                            crate::comms::server::SignalEndpoint {
                                point: feed.position,
                                range: feed.range,
                                receiver: feed
                                    .radio
                                    .as_ref()
                                    .map(|owner| (feed.sender.as_str(), owner)),
                            },
                            crate::comms::server::SignalEndpoint {
                                point: position,
                                range,
                                receiver: radio.as_ref().map(|owner| (uuid.as_str(), owner)),
                            },
                        )
                })
                .flat_map(|feed| feed.samples.iter().cloned())
                .filter(|sample| {
                    sample.route.as_ref().is_none_or(|target| {
                        runtime.as_ref().is_none_or(|runtime| {
                            crate::gm::operations::contact::mode(
                                &runtime.contact_overrides,
                                &uuid,
                                target,
                            ) != crate::gm::operations::contact::ContactMode::Conceal
                        })
                    })
                })
                .collect::<Vec<_>>();
            // Source profiles and stored rows are already validated. Refusal
            // leaves this complete recipient owner unchanged.
            if let Err(error) =
                receive_samples_live(&uuid, state, &profile.profile, tick.0, dt, &samples)
            {
                warn!("radar sharing refused malformed observation: {error}");
            }
        }
    }
}

pub fn sharing_payload(
    owner: &SystemId,
    state: &phoenix_sim_gameplay::science::tracks::BroadTrackState,
    can_operate: bool,
) -> phoenix_model::sensing_sharing::RadarSharingView {
    phoenix_model::sensing_sharing::RadarSharingView {
        owner: owner.clone(),
        share_own_faction: state.share_own_faction,
        receive: state.receive_shared,
        received_samples: state
            .tracks
            .values()
            .filter(|track| track.received.is_some())
            .count()
            .min(u32::MAX as usize) as u32,
        can_operate,
    }
}

#[cfg(test)]
#[path = "sharing_tests.rs"]
mod tests;
