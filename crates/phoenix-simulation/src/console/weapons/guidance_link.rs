//! Source command knowledge and actual paired equipment links. Physical flights
//! stay in their existing owner; no UI or AI projection enumerates remote rounds.
use super::TorpedoSystemResource;
use crate::{
    core::messages::{AdmittedCommands, SystemControlPayload, SystemId},
    entities::spawner::EntityUuid,
    science::broad::{BroadTracks, SensingControls},
    server_app::{Ship, ShipSystemBlackboards},
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::{ecs::system::SystemParam, prelude::*};
use phoenix_model::{
    guided_guidance::{GuidanceObservation, GuidanceReceiverProfile, GuidanceSource},
    guided_orders::{GuidedSourceKnowledge, IssuedGuidanceOrder, KnownGuidedLaunch},
};
use phoenix_sim_gameplay::weapons::{
    guidance::{link_reaches, receiver_accepts},
    guided_orders,
};

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ShipGuidanceKnowledge(pub GuidedSourceKnowledge);

/// Only this consuming System's explicitly bound, current observations may be
/// forwarded. Co-hosted Consoles never pool information.
#[derive(SystemParam)]
pub struct GuidanceMeasurements<'w, 's> {
    scopes: Query<
        'w,
        's,
        (
            &'static EntityUuid,
            &'static ShipConfigComponent,
            &'static ShipSystemBlackboards,
            &'static BroadTracks,
            Option<&'static SensingControls>,
        ),
    >,
    tick: Option<Res<'w, crate::sim_tick::SimTick>>,
    runtime: Option<Res<'w, crate::world::server::WorldContentRuntime>>,
}

impl GuidanceMeasurements<'_, '_> {
    fn concealed(&self, observer: &str, target: &str) -> bool {
        self.runtime.as_ref().is_some_and(|runtime| {
            crate::gm::operations::contact::mode(&runtime.contact_overrides, observer, target)
                == crate::gm::operations::contact::ContactMode::Conceal
        })
    }
    pub fn alias_for_route(&self, entity: Entity, owner: &SystemId, route: &str) -> Option<String> {
        let (_, config, boards, tracks, _) = self.scopes.get(entity).ok()?;
        crate::science::contact_routing::alias_for_source(
            &config.0,
            boards,
            Some(tracks),
            owner,
            route,
        )
    }

    pub fn sample(
        &self,
        entity: Entity,
        owner: &SystemId,
        contact: &str,
    ) -> Option<GuidanceObservation> {
        let (observer, config, boards, tracks, controls) = self.scopes.get(entity).ok()?;
        let route = crate::science::contact_routing::resolve_machinery(
            Some(&config.0),
            Some(boards),
            Some(tracks),
            controls,
            owner,
            contact,
        )?;
        if self.runtime.as_ref().is_some_and(|runtime| {
            crate::gm::operations::contact::mode(&runtime.contact_overrides, &observer.0, &route)
                == crate::gm::operations::contact::ContactMode::Conceal
        }) {
            return None;
        }
        let permitted = config.0.observations_for_system(owner, &boards.0);
        let mut records = boards
            .0
            .keys()
            .filter_map(|key| permitted.get(key))
            .filter_map(crate::science::contact_routing::observations)
            .flatten()
            .filter(|record| {
                record.contact_id == contact && record.selectable && !record.predicted
            });
        let record = records.next()?;
        if !records.all(|other| other == record) {
            return None;
        }
        let height = record.elevation?;
        height.validate().ok()?;
        let sampled_tick = if let Some(report) = &record.observation {
            report.observed_tick?
        } else {
            record.observed_tick
        };
        let result = GuidanceObservation {
            target_uuid: route,
            position: [record.position[0], height.height, record.position[1]],
            sampled_tick,
            source: GuidanceSource::Console {
                ship_uuid: observer.0.clone(),
                console: config.0.console_for_system(owner)?.id.clone(),
                system: owner.clone(),
                samples: record.samples.clone(),
                observation: record.observation.clone(),
            },
        };
        result.validate(self.tick.as_deref()?.0).ok()?;
        Some(result)
    }
}

struct SentOrder {
    source: Entity,
    source_uuid: String,
    tube: SystemId,
    round_uuid: String,
    origin: [f32; 3],
    sender: GuidanceReceiverProfile,
    order: IssuedGuidanceOrder,
    observation: Option<GuidanceObservation>,
}

type Sources<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static EntityUuid,
        &'static ShipPhysics,
        &'static ShipSystemControlSources,
        &'static AdmittedCommands,
        &'static mut ShipGuidanceKnowledge,
        &'static mut ShipPowerSystem,
        Option<&'static ShipDamage>,
    ),
    With<Ship>,
>;

/// Ordinary admitted controls: a paid local transmission can remain unresolved
/// whether its remote round was intercepted, out of range, or simply unheard.
/// No hidden remote refusal reason becomes action feedback.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_guidance_commands(
    mut sources: Sources,
    mut flights: Query<(Entity, &mut TorpedoSystemResource)>,
    measurements: GuidanceMeasurements,
    aims: super::assignments::WeaponAimInputs,
    paths: crate::regions::signal_path::RegionSignalQuery,
    tick: Res<crate::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    let mut sent = Vec::new();
    let mut ordered: Vec<_> = sources
        .iter()
        .map(|row| (row.1 .0.clone(), row.0))
        .collect();
    ordered.sort();
    for (_, entity) in ordered {
        let Ok((_, uuid, pose, controls, admitted, mut knowledge, mut power, damage)) =
            sources.get_mut(entity)
        else {
            continue;
        };
        let source_equipment = flights.get(entity).ok();
        for command in &admitted.0 {
            let (round_uuid, target) = match &command.payload {
                SystemControlPayload::RetargetGuidedRound {
                    round_uuid,
                    target_uuid,
                } => (round_uuid, Some(target_uuid)),
                SystemControlPayload::StopGuidedRoundGuidance { round_uuid } => (round_uuid, None),
                _ => continue,
            };
            let Some((sender, filter)) = source_equipment.as_ref().and_then(|(_, system)| {
                let sender = system.0.config.receiver?;
                let tube = system.0.tubes.iter().find(|tube| {
                    crate::ship::system_registry::torpedo_tube_system_id(&tube.id).as_ref()
                        == Some(&command.target)
                })?;
                Some((sender, tube.targets.as_deref()))
            }) else {
                super::finish_action_feedback(
                    command,
                    &mut outbound,
                    super::WeaponActionResult::Refused(super::WeaponActionRefusal::UnknownMount),
                );
                continue;
            };
            let known = knowledge.0.launches.get(round_uuid);
            let allowed = target.is_none_or(|contact| {
                aims.resolve_reference(entity, &command.target, contact, |_| None)
                    .is_some_and(|aim| aim.allows(filter))
            });
            let permitted = controls.0.policy_for(&command.target);
            let mut result =
                super::WeaponActionResult::Refused(super::WeaponActionRefusal::MissingTarget);
            let observation =
                target.and_then(|target| measurements.sample(entity, &command.target, target));
            if allowed
                && super::shared::system_is_registered(controls, &command.target)
                && (permitted.accept_human_input || permitted.operate_ai)
                && known
                    .is_some_and(|known| known.tube == command.target && known.receiver.is_some())
                && target.is_none_or(|_| observation.is_some())
                && damage.is_none_or(|damage| {
                    damage.factor(
                        &command.target,
                        phoenix_model::damage::DamageBinding::Operation,
                    ) > 0.0
                })
            {
                let mut candidate = knowledge.0.clone();
                if let Some(order) = guided_orders::issue(
                    &mut candidate,
                    &command.target,
                    round_uuid,
                    target.cloned(),
                    observation.as_ref().map(|reading| reading.sampled_tick),
                    tick.0,
                ) {
                    if matches!(power.0.try_work_discrete(&command.target), Ok(true)) {
                        knowledge.0 = candidate;
                        result = super::WeaponActionResult::Applied;
                        sent.push(SentOrder {
                            source: entity,
                            source_uuid: uuid.0.clone(),
                            tube: command.target.clone(),
                            round_uuid: round_uuid.clone(),
                            origin: [pose.x, pose.y, pose.z],
                            sender,
                            order,
                            observation,
                        });
                    } else {
                        result = super::WeaponActionResult::Refused(
                            super::WeaponActionRefusal::EnergyUnavailable,
                        );
                    }
                }
            }
            super::finish_action_feedback(command, &mut outbound, result);
        }
    }
    for message in sent {
        // Membership is private actuator routing, never a UI/AI availability query.
        let matches: Vec<_> = flights
            .iter()
            .filter_map(|(entity, owner)| {
                owner
                    .0
                    .in_flight
                    .iter()
                    .find(|round| {
                        round.uuid == message.round_uuid
                            && round.source_uuid.as_deref() == Some(message.source_uuid.as_str())
                            && crate::ship::system_registry::torpedo_tube_system_id(&round.tube_id)
                                .as_ref()
                                == Some(&message.tube)
                    })
                    .map(|_| entity)
            })
            .collect();
        if matches.len() != 1 {
            continue;
        }
        let Ok((_, mut owner)) = flights.get_mut(matches[0]) else {
            continue;
        };
        let Some(round) = owner
            .0
            .in_flight
            .iter_mut()
            .find(|round| round.uuid == message.round_uuid)
        else {
            continue;
        };
        let Some(receiver) = round.profile.receiver else {
            continue;
        };
        let position = [round.x, round.y, round.z];
        let outbound_path = paths.transmission(
            Vec3::from_array(message.origin),
            Vec3::from_array(position),
            crate::regions::signal_path::SignalClass::Comms,
        );
        let link = link_reaches(&message.sender, position, message.origin, outbound_path)
            && link_reaches(&receiver, position, message.origin, outbound_path)
            && message.observation.as_ref().is_none_or(|reading| {
                receiver_accepts(
                    &message.sender,
                    position,
                    message.origin,
                    outbound_path,
                    reading,
                    tick.0,
                    fixed.timestep().as_secs_f64(),
                ) && receiver_accepts(
                    &receiver,
                    position,
                    message.origin,
                    outbound_path,
                    reading,
                    tick.0,
                    fixed.timestep().as_secs_f64(),
                )
            });
        if !link {
            continue;
        }
        let concealed = message
            .observation
            .as_ref()
            .is_some_and(|reading| measurements.concealed(&round.uuid, &reading.target_uuid));
        if !concealed {
            round.receive_guidance(message.observation.as_ref());
        }
        // Receipt means this actual order was heard, never proof that a remote
        // controller accepted/applied it. Concealment supplies no refusal oracle.
        let reverse = paths.transmission(
            Vec3::from_array(position),
            Vec3::from_array(message.origin),
            crate::regions::signal_path::SignalClass::Comms,
        );
        if !link_reaches(&message.sender, message.origin, position, reverse)
            || !link_reaches(&receiver, message.origin, position, reverse)
        {
            continue;
        }
        if let Ok((_, _, _, _, _, mut knowledge, _, _)) = sources.get_mut(message.source) {
            guided_orders::acknowledge(
                &mut knowledge.0,
                &message.tube,
                &message.round_uuid,
                &message.order,
                tick.0,
            );
        }
    }
}

/// Called only at the actual own emission, never by a projection or alive scan.
pub(crate) fn record_emitted(
    knowledge: &mut ShipGuidanceKnowledge,
    round: &crate::weapons::torpedo::Torpedo,
    public_target: Option<String>,
    sampled_tick: Option<u64>,
    tick: u64,
) {
    let Some(tube) = crate::ship::system_registry::torpedo_tube_system_id(&round.tube_id) else {
        return;
    };
    guided_orders::note_launch(
        &mut knowledge.0,
        KnownGuidedLaunch {
            round_uuid: round.uuid.clone(),
            tube,
            launched_tick: tick,
            lifespan_secs: round.profile.lifespan,
            receiver: round.profile.receiver,
            issued: IssuedGuidanceOrder {
                revision: 0,
                issued_tick: tick,
                target: public_target,
                sampled_tick,
            },
            acknowledged: None,
        },
        tick,
    );
}

/// Only local own machinery/endurance participates; no remote survival query.
pub(crate) fn refresh_source_knowledge(
    tick: Res<crate::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    mut sources: Query<(&mut ShipGuidanceKnowledge, Option<&TorpedoSystemResource>), With<Ship>>,
) {
    for (mut knowledge, machinery) in &mut sources {
        guided_orders::retire(&mut knowledge.0, tick.0, fixed.timestep().as_secs_f64());
        let pending: std::collections::BTreeSet<_> = machinery
            .into_iter()
            .flat_map(|system| &system.0.burst_states)
            .map(|burst| &burst.launch_uuid)
            .collect();
        knowledge
            .0
            .launch_commands
            .retain(|id, _| pending.contains(id));
    }
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::sim_sets::{FixedStep, SimSet};
    app.declare_state::<ShipGuidanceKnowledge>(StateClass::Folded, "guided-source-order-knowledge")
        .add_systems(
            FixedUpdate,
            (
                handle_guidance_commands
                    .in_set(SimSet::Physics)
                    .in_set(FixedStep::HandleGuidanceCommands)
                    // Both human and the ordinary tube AI issue after this
                    // step's movement. New orders steer on the next advance.
                    .after(FixedStep::AiTorpedoAutoFire)
                    .after(FixedStep::AdvanceTorpedoFlights)
                    .before(FixedStep::HandleFireTorpedo),
                refresh_source_knowledge
                    .in_set(SimSet::Physics)
                    .in_set(FixedStep::RefreshGuidanceKnowledge)
                    .after(FixedStep::HandleFireTorpedo)
                    .after(FixedStep::AdvanceTorpedoFlights),
            ),
        );
}

#[cfg(test)]
#[path = "guidance_link_tests.rs"]
mod tests;
