//! Deliberate Navigation sharing through its existing waypoint and public chart.
use super::{NavigationWaypoint, WaypointMode};
use crate::{
    core::messages::{AdmittedCommands, SystemControlPayload, SystemId},
    entities::spawner::{EntityUuid, FactionComponent},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
pub use phoenix_sim_gameplay::science::navigation_sharing::*;

#[derive(Component, Clone, Debug)]
pub struct NavigationSharing(pub NavigationSharingState);
#[derive(Component, Clone, Debug)]
pub struct NavigationSharingProfile {
    pub radar: crate::radar_config::RadarConfig,
}

pub fn install(config: &crate::entities::config::EntityConfig, commands: &mut EntityCommands) {
    let Some(nav) = config
        .navigation_console
        .as_ref()
        .filter(|nav| nav.sharing.is_some())
    else {
        return;
    };
    validate_config(config).expect("accepted Navigation sharing owner");
    commands.insert((
        NavigationSharing(NavigationSharingState::new(nav.sharing.as_ref().unwrap())),
        NavigationSharingProfile {
            radar: nav.system_chart.clone(),
        },
    ));
}
pub(super) fn operating(sources: &ShipSystemControlSources, owner: &SystemId) -> bool {
    sources.0.entries().any(|(id, _)| id == owner)
        && !sources.0.is_offline(owner)
        && sources.0.source_for(owner) != crate::ship::control_source::ControlSource::Offline
}
pub(crate) fn apply_commands(
    mut ships: Query<(&AdmittedCommands, &mut NavigationSharing)>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    for (commands, mut sharing) in &mut ships {
        for command in &commands.0 {
            if command.target != sharing.0.owner {
                continue;
            }
            let SystemControlPayload::SetRadarSharing {
                share_own_faction,
                receive,
            } = &command.payload
            else {
                continue;
            };
            sharing.0.share_own_faction = *share_own_faction;
            sharing.0.receive = *receive;
            if let Some(feedback) = crate::command_admission::feedback::admitted_action_feedback(
                command,
                crate::core::messages::ActionFeedbackOutcome::Applied,
            ) {
                if let Some(outbound) = &mut outbound {
                    outbound.write(feedback);
                }
            }
        }
    }
}

fn visible_chart<'a>(
    world: Option<&'a crate::lobby::WorldResource>,
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    observer: &str,
    radar: &crate::radar_config::RadarConfig,
    pose: Vec3,
) -> Vec<&'a crate::core::messages::EntitySnapshot> {
    world
        .into_iter()
        .flat_map(|world| &world.0.entities)
        .filter(|entity| {
            entity
                .tags
                .iter()
                .any(|tag| tag == crate::console_delivery::CHARTED_TAG)
                && entity
                    .tags
                    .iter()
                    .any(|tag| radar.shows.iter().any(|allowed| allowed.as_str() == tag))
                && entity.position.is_some_and(|position| {
                    position.iter().all(|x| x.is_finite())
                        && pose.distance(Vec3::from_array(position)) <= radar.range
                })
                && runtime.is_none_or(|runtime| {
                    crate::gm::operations::contact::mode(
                        &runtime.contact_overrides,
                        observer,
                        &entity.uuid,
                    ) != crate::gm::operations::contact::ContactMode::Conceal
                })
        })
        .collect()
}

fn measured_anchor(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
    tracks: Option<&crate::science::broad::BroadTracks>,
    owner: &SystemId,
    source: &str,
) -> Option<NavigationSamplePoint> {
    let tracks = tracks?;
    let permitted = config.observations_for_system(owner, &boards.0);
    tracks.0.iter().find_map(|(instrument, state)| {
        crate::science::contact_routing::observations(permitted.get(instrument)?)?
            .iter()
            .find(|record| {
                !record.predicted
                    && record.selectable
                    && state.resolve_current(&record.contact_id) == Some(source)
            })
            .map(|record| NavigationSamplePoint {
                kind: NavigationSharedKind::Waypoint,
                position: record.position,
                observed_tick: Some(record.observed_tick),
                observer_position: record
                    .samples
                    .first()
                    .and_then(|sample| sample.observer_position),
            })
    })
}

#[derive(Clone)]
struct Feed {
    ship: String,
    faction: uuid::Uuid,
    pose: Vec3,
    range: f32,
    samples: Vec<NavigationSharedObservation>,
}

/// Publishable inputs are own waypoint information and explicitly charted
/// landmarks inside this Navigation instrument's authored chart coverage.
/// Neither the foreign waypoint anchor nor any mobile world roster is copied.
#[allow(clippy::type_complexity)]
pub(crate) fn tick(
    tick: Res<crate::sim_tick::SimTick>,
    signals: crate::regions::signal_path::RegionSignalQuery,
    world: Option<Res<crate::lobby::WorldResource>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut ships: Query<(
        &EntityUuid,
        &Transform,
        Option<&crate::ship::state::ShipPhysics>,
        &FactionComponent,
        &crate::comms::CommsRange,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &crate::server_app::ShipSystemBlackboards,
        &NavigationWaypoint,
        &NavigationSharingProfile,
        Option<&crate::science::broad::BroadTracks>,
        Option<&crate::modifiers::ShipModifiers>,
        &mut NavigationSharing,
    )>,
) {
    let mut feeds = Vec::new();
    let mut ordered: Vec<_> = ships.iter_mut().collect();
    ordered.sort_by(|a, b| a.0.0.cmp(&b.0.0));
    for (
        uuid,
        transform,
        physics,
        faction,
        range,
        config,
        sources,
        boards,
        waypoint,
        profile,
        tracks,
        modifiers,
        sharing,
    ) in &mut ordered
    {
        if !operating(sources, &sharing.0.owner) {
            continue;
        }
        let pose = physics.map_or(transform.translation, |physics| {
            Vec3::new(physics.x, physics.y, physics.z)
        });
        let chart = visible_chart(
            world.as_deref(),
            runtime.as_deref(),
            &uuid.0,
            &profile.radar,
            pose,
        );
        let mut points: Vec<_> = chart
            .iter()
            .filter_map(|entity| {
                Some(NavigationSamplePoint {
                    kind: NavigationSharedKind::Landmark {
                        id: entity.uuid.clone(),
                        name: entity.name.clone().unwrap_or_else(|| entity.uuid.clone()),
                    },
                    position: [entity.position?[0], entity.position?[2]],
                    observed_tick: None,
                    observer_position: None,
                })
            })
            .collect();
        let waypoint = match waypoint.mode() {
            Some(WaypointMode::Free { x, z }) => Some(NavigationSamplePoint {
                kind: NavigationSharedKind::Waypoint,
                position: [*x, *z],
                observed_tick: None,
                observer_position: None,
            }),
            Some(WaypointMode::Anchored { source_uuid, .. }) => chart
                .iter()
                .find(|entity| &entity.uuid == source_uuid)
                .and_then(|entity| {
                    entity.position.map(|position| NavigationSamplePoint {
                        kind: NavigationSharedKind::Waypoint,
                        position: [position[0], position[2]],
                        observed_tick: None,
                        observer_position: None,
                    })
                })
                .or_else(|| {
                    measured_anchor(&config.0, boards, *tracks, &sharing.0.owner, source_uuid)
                }),
            None => None,
        };
        if let Some(waypoint) = waypoint {
            points.push(waypoint);
        }
        if let Err(error) = sharing.0.sample(&uuid.0, tick.0, [pose.x, pose.z], &points) {
            warn!("Navigation sharing sample refused: {error}");
            continue;
        }
        if sharing.0.share_own_faction
            && crate::comms::server::information_endpoint_available(config, sources, *modifiers)
        {
            feeds.push(Feed {
                ship: uuid.0.clone(),
                faction: faction.0,
                pose,
                range: range.0,
                samples: sharing.0.exports(),
            });
        }
    }
    // All original inputs are staged before any receiving owner changes.
    for (
        uuid,
        transform,
        physics,
        faction,
        range,
        config,
        sources,
        _,
        _,
        _,
        _,
        modifiers,
        sharing,
    ) in &mut ordered
    {
        if !operating(sources, &sharing.0.owner)
            || !crate::comms::server::information_endpoint_available(config, sources, *modifiers)
        {
            continue;
        }
        let pose = physics.map_or(transform.translation, |physics| {
            Vec3::new(physics.x, physics.y, physics.z)
        });
        let samples = feeds
            .iter()
            .filter(|feed| {
                feed.ship != uuid.0
                    && feed.faction == faction.0
                    && crate::comms::server::signal_comms_in_range(
                        &signals, feed.pose, pose, feed.range, range.0,
                    )
            })
            .flat_map(|feed| feed.samples.iter())
            .filter(|sample| match &sample.kind {
                NavigationSharedKind::Landmark { id, .. } => {
                    runtime.as_ref().is_none_or(|runtime| {
                        crate::gm::operations::contact::mode(
                            &runtime.contact_overrides,
                            &uuid.0,
                            id,
                        ) != crate::gm::operations::contact::ContactMode::Conceal
                    })
                }
                NavigationSharedKind::Waypoint => true,
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Err(error) = sharing.0.accept(&uuid.0, tick.0, &samples) {
            warn!("Navigation sharing receipt refused: {error}");
        }
    }
}

#[cfg(test)]
#[path = "sharing_tests.rs"]
mod tests;
