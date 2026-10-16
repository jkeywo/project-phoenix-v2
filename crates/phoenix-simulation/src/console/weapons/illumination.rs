//! An independently controlled paid emitter; no target intent or flight owner
//! is copied from Tactical. Generic channels are the sole command state.
use super::guidance_link::GuidanceMeasurements;
use crate::{
    core::messages::{AdmittedCommands, SystemControlPayload as Control, SystemId},
    entities::spawner::EntityUuid,
    server_app::Ship,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::{math::DVec3, prelude::*};
use phoenix_sim_gameplay::{
    ship::composition::{illumination_composition, CommandChannelValue, SystemCommandChannels},
    weapons::{
        guidance::{link_reaches, receiver_accepts},
        illumination::IlluminationConfig,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipIllumination(pub BTreeMap<SystemId, SystemCommandChannels>);
/// Own last emission only. No receiver list/count, application or survival fact.
#[derive(Component, Clone, Debug, Default)]
pub struct IlluminationEmissions(pub BTreeSet<SystemId>);
impl ShipIllumination {
    pub fn from_ship(ship: &crate::ship::config::ShipConfig) -> Option<Self> {
        let values: BTreeMap<_, _> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::ILLUMINATION_KIND)
            .map(|system| {
                (
                    system.id.clone(),
                    SystemCommandChannels::from_definitions(&system.channels),
                )
            })
            .collect();
        (!values.is_empty()).then_some(Self(values))
    }
    pub fn validate(&self, ship: &crate::ship::config::ShipConfig) -> Result<(), String> {
        let owners: BTreeSet<_> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::ILLUMINATION_KIND)
            .map(|system| &system.id)
            .collect();
        if owners != self.0.keys().collect() {
            return Err("illumination owner roster differs from accepted emitters".into());
        }
        for (owner, channels) in &self.0 {
            let system = ship
                .system(owner)
                .ok_or("illumination owner is not an authored System")?;
            phoenix_sim_gameplay::ship::composition::validate_composition(system)?;
            phoenix_sim_gameplay::weapons::illumination::validate_channels(
                channels,
                &system.channels,
            )?;
        }
        Ok(())
    }
    pub fn enabled(&self, ship: &crate::ship::config::ShipConfig, owner: &SystemId) -> bool {
        illumination_composition(
            ship.system(owner)
                .map(|system| system.components.as_slice())
                .unwrap_or_default(),
        )
        .zip(self.0.get(owner))
        .is_some_and(|(binding, channels)| channels.boolean(binding.enabled_channel))
    }
    pub fn target<'a>(
        &'a self,
        ship: &crate::ship::config::ShipConfig,
        owner: &SystemId,
    ) -> Option<&'a str> {
        let binding = illumination_composition(&ship.system(owner)?.components)?;
        self.0.get(owner)?.target(binding.target_channel)
    }
}

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(state) = config
        .ship_config
        .as_ref()
        .and_then(ShipIllumination::from_ship)
    {
        commands.insert((state, IlluminationEmissions::default()));
    }
}

pub(crate) fn handle_commands(
    measurements: GuidanceMeasurements,
    mut owners: Query<(
        Entity,
        &AdmittedCommands,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &mut ShipIllumination,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::server::OutboundMessage>>>,
) {
    for (entity, admitted, config, sources, mut state) in &mut owners {
        for command in &admitted.0 {
            let Some(system) = config
                .0
                .system(&command.target)
                .filter(|system| system.kind == crate::ship::system_registry::ILLUMINATION_KIND)
            else {
                continue;
            };
            let Some(binding) = illumination_composition(&system.components) else {
                continue;
            };
            let policy = sources.0.policy_for(&command.target);
            let permitted = sources
                .0
                .entries()
                .any(|(owner, _)| owner == &command.target)
                && (policy.accept_human_input || policy.operate_ai);
            let value = match &command.payload {
                Control::SetIlluminationEnabled { enabled } => Some((
                    binding.enabled_channel,
                    CommandChannelValue::Boolean(*enabled),
                )),
                Control::SetIlluminationTarget { target_uuid }
                    if target_uuid.as_ref().is_none_or(|contact| {
                        measurements
                            .sample(entity, &command.target, contact)
                            .is_some()
                    }) =>
                {
                    Some((
                        binding.target_channel,
                        CommandChannelValue::EntityTarget(target_uuid.clone()),
                    ))
                }
                Control::SetIlluminationTarget { .. } => None,
                _ => continue,
            };
            let changed = if permitted {
                value
                    .and_then(|(channel, value)| {
                        state
                            .0
                            .get_mut(&command.target)
                            .map(|channels| channels.set(channel, value).is_ok())
                    })
                    .unwrap_or(false)
            } else {
                false
            };
            crate::command_admission::finish_admitted_action_feedback(
                &mut outbound,
                command,
                if changed {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
    }
}

struct Emission {
    origin: [f32; 3],
    config: IlluminationConfig,
    observation: phoenix_model::guided_guidance::GuidanceObservation,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn tick(
    fixed: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    measurements: GuidanceMeasurements,
    paths: crate::regions::signal_path::RegionSignalQuery,
    mut owners: Query<
        (
            Entity,
            &EntityUuid,
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &ShipIllumination,
            &ShipPhysics,
            &mut ShipPowerSystem,
            Option<&ShipDamage>,
            Option<&crate::weapons::mounts::MountProfiles>,
            Option<&crate::weapons::mounts::ShipMounts>,
            &mut IlluminationEmissions,
        ),
        With<Ship>,
    >,
    mut flights: Query<&mut super::TorpedoSystemResource>,
) {
    let dt = fixed.timestep().as_secs_f32();
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let mut order: Vec<_> = owners.iter().map(|row| (row.1 .0.clone(), row.0)).collect();
    order.sort();
    let mut emissions = Vec::new();
    for (_, entity) in order {
        let Ok((
            _,
            _,
            config,
            sources,
            state,
            own,
            mut power,
            damage,
            profiles,
            mounts,
            mut readout,
        )) = owners.get_mut(entity)
        else {
            continue;
        };
        readout.0.clear();
        for owner in state.0.keys() {
            if !state.enabled(&config.0, owner)
                || sources.0.is_offline(owner)
                || sources.0.source_for(owner)
                    == crate::ship::control_source::ControlSource::Offline
                || !sources.0.entries().any(|(id, _)| id == owner)
            {
                continue;
            }
            let Some(system) = config.0.system(owner) else {
                continue;
            };
            let Some(binding) = illumination_composition(&system.components) else {
                continue;
            };
            let Some(reading) = state
                .target(&config.0, owner)
                .and_then(|target| measurements.sample(entity, owner, target))
            else {
                continue;
            };
            let origin = [own.x, own.y, own.z];
            let distance = (DVec3::from_array(reading.position.map(f64::from))
                - DVec3::from_array(origin.map(f64::from)))
            .length();
            let geometry = crate::weapons::mounts::geometry(
                profiles,
                mounts,
                owner,
                &binding.config.mount,
                [reading.position[0], reading.position[2]],
                [own.x, own.z],
                own.yaw,
            );
            if !geometry.in_arc
                || !geometry.in_range
                || distance > f64::from(binding.config.mount.range)
            {
                continue;
            }
            let path = paths.transmission(
                Vec3::from_array(origin),
                Vec3::from_array(reading.position),
                crate::regions::signal_path::SignalClass::Sensors,
            );
            if !path.is_finite() || path < binding.config.minimum_transmission || path > 1.0 {
                continue;
            }
            let factor = damage.map_or(1.0, |damage| {
                damage.factor(owner, phoenix_model::damage::DamageBinding::Operation)
            });
            if !factor.is_finite() || factor <= 0.0 {
                continue;
            }
            // One fixed interval of emission is staged against the real owner.
            // A partial grant cannot produce a full-quality free observation.
            if !power
                .0
                .work_continuous_if(owner, dt, 1.0 / factor, |grant| {
                    Ok((grant.fraction >= 1.0 && grant.granted > 0.0).then_some(()))
                })
                .is_ok_and(|accepted| accepted.is_some())
            {
                continue;
            }
            readout.0.insert(owner.clone());
            emissions.push(Emission {
                origin,
                config: *binding.config,
                observation: reading,
            });
        }
    }
    for emission in emissions {
        for mut provider in &mut flights {
            for round in &mut provider.0.in_flight {
                if round.target_uuid.as_deref() != Some(emission.observation.target_uuid.as_str()) {
                    continue;
                }
                let Some(receiver) = round
                    .profile
                    .receiver
                    .filter(|receiver| receiver.accept_external_illumination)
                else {
                    continue;
                };
                if measurements.concealed(&round.uuid, &emission.observation.target_uuid)
                    || round.source_uuid.as_ref().is_some_and(|source| {
                        measurements.concealed(source, &emission.observation.target_uuid)
                    })
                {
                    continue;
                }
                let position = [round.x, round.y, round.z];
                let path = paths.transmission(
                    Vec3::from_array(emission.origin),
                    Vec3::from_array(position),
                    crate::regions::signal_path::SignalClass::Comms,
                );
                // Only the emitter's physical envelope is replaced here. Age
                // remains exactly the emitted receiver's frozen original bound.
                let transmitter = phoenix_model::guided_guidance::GuidanceReceiverProfile {
                    range: emission.config.link_range,
                    minimum_transmission: emission.config.minimum_transmission,
                    ..receiver
                };
                if link_reaches(&transmitter, position, emission.origin, path)
                    && receiver_accepts(
                        &receiver,
                        position,
                        emission.origin,
                        path,
                        &emission.observation,
                        tick.0,
                        fixed.timestep().as_secs_f64(),
                    )
                {
                    round.assist_guidance(&emission.observation);
                }
            }
        }
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn operate_ai(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    measurements: GuidanceMeasurements,
    mut owners: Query<(
        Entity,
        Option<&EntityUuid>,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &crate::server_app::ShipSystemBlackboards,
        &crate::science::broad::BroadTracks,
        &ShipIllumination,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (
        entity,
        uuid,
        config,
        sources,
        boards,
        tracks,
        state,
        programs,
        mut runtimes,
        mut admitted,
    ) in &mut owners
    {
        for owner in state.0.keys() {
            if !crate::ai::host::ai_operates(&sources.0, owner) {
                continue;
            }
            let Some(system) = config.0.system(owner) else {
                continue;
            };
            let Some(binding) = illumination_composition(&system.components) else {
                continue;
            };
            let Some(program) = programs.0.get(owner) else {
                continue;
            };
            let Some(runtime) = runtimes.0.get_mut(owner) else {
                continue;
            };
            let enabled = state.enabled(&config.0, owner);
            let target = state.target(&config.0, owner);
            let available =
                target.is_some_and(|target| measurements.sample(entity, owner, target).is_some());
            let permitted = config.0.observations_for_system(owner, &boards.0);
            let request = if crate::science::reading::mission_text_ready(
                runtime, &config.0, boards, owner, tick.0,
            ) {
                match permitted.get(&crate::ship::system_registry::viewscreen_system_id()) {
                    Some(crate::core::messages::SystemBlackboard::Viewscreen(board)) => {
                        crate::objectives::top_operate_directive(
                            &board.scored_objectives,
                            crate::core::messages::SystemAffinity::Sensors,
                            |directive| {
                                matches!(
                                    directive,
                                    crate::core::messages::AiDirective::Illuminate { .. }
                                )
                            },
                        )
                        .and_then(|directive| match directive {
                            crate::core::messages::AiDirective::Illuminate { target } => {
                                Some(target)
                            }
                            _ => None,
                        })
                    }
                    _ => None,
                }
            } else {
                None
            };
            let requested = request
                .and_then(|request| {
                    crate::science::contact_routing::request_contact(
                        &config.0, boards, tracks, owner, request,
                    )
                })
                .map(|(alias, _)| alias)
                .filter(|alias| measurements.sample(entity, owner, alias).is_some());
            let facts = BTreeMap::from([
                ("enabled".into(), enabled),
                ("has_target".into(), target.is_some()),
                ("target_available".into(), available),
                ("request_available".into(), requested.is_some()),
            ]);
            let pending = BTreeMap::from([
                (binding.enabled_channel.into(), enabled),
                (binding.target_channel.into(), false),
            ]);
            let Ok(TreeHostOutcome::Evaluated(result)) = decide_tree(
                &sources.0,
                owner,
                Some(program),
                &facts,
                &pending,
                runtime,
                tick.0,
            ) else {
                continue;
            };
            if result.intents.get(binding.target_channel) == Some(&true) {
                if let Some(requested) =
                    requested.filter(|requested| Some(requested.as_str()) != target)
                {
                    crate::command_admission::ai_emit::emit_ai_command(
                        uuid,
                        owner.clone(),
                        Control::SetIlluminationTarget {
                            target_uuid: Some(requested),
                        },
                        sources,
                        &sessions,
                        Some(config),
                        &mut admitted,
                    );
                }
            }
            if let Some(next) = result
                .intents
                .get(binding.enabled_channel)
                .filter(|next| **next != enabled)
            {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
                    owner.clone(),
                    Control::SetIlluminationEnabled { enabled: *next },
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
    }
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    use crate::sim_sets::{FixedStep, SimSet};
    app.declare_state::<ShipIllumination>(StateClass::Folded, "ship-illumination-intent")
        .declare_state::<IlluminationEmissions>(
            StateClass::Derived,
            "illumination-emission-readout",
        );
    crate::ai::cadence::register_ai_cadence(app);
    for (payload, feedback) in [
        (
            Payload::SetIlluminationTarget,
            &[Payload::SetIlluminationTarget][..],
        ),
        (
            Payload::SetIlluminationEnabled,
            &[Payload::SetIlluminationEnabled][..],
        ),
    ] {
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(crate::ship::system_registry::ILLUMINATION_KIND, payload)
                .with_feedback(
                    crate::command_admission::FeedbackAddress::DeclaredKindOnly,
                    feedback,
                )
                .with_capability_ownership(),
        );
    }
    app.add_systems(
        FixedUpdate,
        (
            operate_ai
                .in_set(SimSet::Input)
                .in_set(FixedStep::OperateIlluminationAi)
                .after(FixedStep::ReadSystemText)
                .before(handle_commands)
                .run_if(crate::ai::cadence::ai_tick_ready),
            handle_commands
                .in_set(SimSet::Input)
                .in_set(FixedStep::HandleIlluminationCommands),
            tick.in_set(SimSet::Physics)
                .in_set(FixedStep::TickIllumination)
                .after(FixedStep::HandleGuidanceCommands)
                .before(FixedStep::HandleFireTorpedo),
        ),
    );
}

#[cfg(test)]
#[path = "illumination_tests.rs"]
mod tests;

pub(crate) fn project(
    world: &World,
    entity: Entity,
    owner: &SystemId,
) -> Option<phoenix_model::illumination_ui::IlluminationControlsView> {
    let config = world.get::<ShipConfigComponent>(entity)?;
    let state = world.get::<ShipIllumination>(entity)?;
    if !state.0.contains_key(owner) {
        return None;
    }
    let controllable = world
        .get::<ShipSystemControlSources>(entity)
        .is_some_and(|sources| sources.0.policy_for(owner).accept_human_input);
    let mut view = phoenix_model::illumination_ui::IlluminationControlsView {
        enabled: state.enabled(&config.0, owner),
        target: state.target(&config.0, owner).map(str::to_owned),
        emitting: world
            .get::<IlluminationEmissions>(entity)
            .is_some_and(|readout| readout.0.contains(owner)),
        controllable,
        targets: Vec::new(),
    };
    let Some((observer, boards, tracks, tick)) = world
        .get::<EntityUuid>(entity)
        .zip(world.get::<crate::server_app::ShipSystemBlackboards>(entity))
        .zip(world.get::<crate::science::broad::BroadTracks>(entity))
        .zip(world.get_resource::<crate::sim_tick::SimTick>())
        .map(|(((a, b), c), d)| (a, b, c, d.0))
    else {
        return Some(view);
    };
    let scope = super::guidance_link::GuidanceContext {
        observer,
        config,
        boards,
        tracks,
        controls: world.get::<crate::science::broad::SensingControls>(entity),
        tick,
        runtime: world.get_resource::<crate::world::server::WorldContentRuntime>(),
    };
    let permitted = config.0.observations_for_system(owner, &boards.0);
    let mut seen = BTreeSet::new();
    for record in boards
        .0
        .keys()
        .filter_map(|key| permitted.get(key))
        .filter_map(crate::science::contact_routing::observations)
        .flatten()
    {
        if !seen.insert(record.contact_id.clone())
            || scope.sample(owner, &record.contact_id).is_none()
        {
            continue;
        }
        view.targets
            .push(phoenix_model::guided_orders::GuidanceTargetOption {
                contact: record.contact_id.clone(),
                name: record
                    .identity
                    .as_ref()
                    .map(|identity| identity.name.clone()),
            });
    }
    view.targets.sort_by(|a, b| a.contact.cmp(&b.contact));
    Some(view)
}

#[cfg(test)]
#[path = "illumination_payment_reference.rs"]
mod payment_reference;
