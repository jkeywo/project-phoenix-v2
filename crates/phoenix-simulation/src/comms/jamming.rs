//! One independently commanded paid jammer. Canonical generic channels retain
//! intent; current receiver influence is recomputed, never saved as work credit.
use super::server::CommsRuntime;
use crate::console::weapons::guidance_link::GuidanceMeasurements;
use crate::{
    core::messages::{AdmittedCommands, SystemControlPayload as Control, SystemId},
    entities::spawner::EntityUuid,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_sim_gameplay::{
    electronic::jamming::{exposure, JammingConfig},
    ship::composition::{jamming_composition, CommandChannelValue, SystemCommandChannels},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipJamming(pub BTreeMap<SystemId, SystemCommandChannels>);
#[derive(Clone, Debug)]
struct PaidEmission {
    origin: [f32; 3],
    heading: f32,
    profile: JammingConfig,
    fraction: f32,
}
/// Ephemeral physical current-step emissions, not a receiver census or credit.
#[derive(Component, Clone, Debug, Default)]
pub struct JammingEmissions {
    processed_tick: Option<u64>,
    entries: BTreeMap<SystemId, PaidEmission>,
}
impl JammingEmissions {
    #[cfg(test)]
    pub(crate) fn is_unprocessed(&self) -> bool {
        self.processed_tick.is_none() && self.entries.is_empty()
    }
}
impl ShipJamming {
    pub fn from_ship(ship: &crate::ship::config::ShipConfig) -> Option<Self> {
        let values: BTreeMap<_, _> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::JAMMING_KIND)
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
            .filter(|system| system.kind == crate::ship::system_registry::JAMMING_KIND)
            .map(|system| &system.id)
            .collect();
        if owners != self.0.keys().collect() {
            return Err("jamming owner roster differs from accepted emitters".into());
        }
        for (owner, channels) in &self.0 {
            let system = ship.system(owner).ok_or("jamming has no actual System")?;
            phoenix_sim_gameplay::ship::composition::validate_composition(system)?;
            phoenix_sim_gameplay::weapons::illumination::validate_channels(
                channels,
                &system.channels,
            )
            .map_err(|reason| format!("jamming intent: {reason}"))?;
        }
        Ok(())
    }
    pub fn enabled(&self, ship: &crate::ship::config::ShipConfig, owner: &SystemId) -> bool {
        ship.system(owner)
            .and_then(|system| jamming_composition(&system.components))
            .zip(self.0.get(owner))
            .is_some_and(|(binding, channels)| channels.boolean(binding.enabled_channel))
    }
    pub fn target<'a>(
        &'a self,
        ship: &crate::ship::config::ShipConfig,
        owner: &SystemId,
    ) -> Option<&'a str> {
        self.0
            .get(owner)?
            .target(jamming_composition(&ship.system(owner)?.components)?.target_channel)
    }
}
pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(state) = config.ship_config.as_ref().and_then(ShipJamming::from_ship) {
        commands.insert((state, JammingEmissions::default()));
    }
}

pub(crate) fn handle_commands(
    measurements: crate::console::weapons::guidance_link::GuidanceMeasurements,
    tick: Res<crate::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    mut owners: Query<(
        Entity,
        &AdmittedCommands,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &mut ShipJamming,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::server::OutboundMessage>>>,
) {
    for (entity, admitted, config, sources, mut state) in &mut owners {
        for command in &admitted.0 {
            let Some(system) = config
                .0
                .system(&command.target)
                .filter(|system| system.kind == crate::ship::system_registry::JAMMING_KIND)
            else {
                continue;
            };
            let Some(binding) = jamming_composition(&system.components) else {
                continue;
            };
            let policy = sources.0.policy_for(&command.target);
            let permitted = sources.0.entries().any(|(id, _)| id == &command.target)
                && (policy.accept_human_input || policy.operate_ai);
            let value = match &command.payload {
                Control::SetJammingEnabled { enabled } => Some((
                    binding.enabled_channel,
                    CommandChannelValue::Boolean(*enabled),
                )),
                Control::SetJammingTarget { target_uuid }
                    if target_uuid.as_ref().is_none_or(|contact| {
                        measurements
                            .sample(entity, &command.target, contact)
                            .is_some_and(|sample| {
                                binding.config.sample_fresh(
                                    sample.sampled_tick,
                                    tick.0,
                                    fixed.timestep().as_secs_f64(),
                                )
                            })
                    }) =>
                {
                    Some((
                        binding.target_channel,
                        CommandChannelValue::EntityTarget(target_uuid.clone()),
                    ))
                }
                Control::SetJammingTarget { .. } => None,
                _ => continue,
            };
            let applied = permitted
                && value.is_some_and(|(channel, value)| {
                    state
                        .0
                        .get_mut(&command.target)
                        .is_some_and(|channels| channels.set(channel, value).is_ok())
                });
            crate::command_admission::finish_admitted_action_feedback(
                &mut outbound,
                command,
                if applied {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
    }
}

/// Input precedes Hail. Physics traversal changes NEXT boundary's actual bearing.
/// One actual grant bounds all physical receivers for this one step.
pub(crate) fn tick(
    fixed: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    measurements: crate::console::weapons::guidance_link::GuidanceMeasurements,
    mut owners: Query<(
        Entity,
        &EntityUuid,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipJamming,
        Option<&ShipPhysics>,
        Option<&Transform>,
        Option<&mut ShipPowerSystem>,
        Option<&ShipDamage>,
        Option<&crate::weapons::mounts::MountProfiles>,
        Option<&crate::weapons::mounts::ShipMounts>,
        (
            &mut JammingEmissions,
            Option<&crate::resources::fitting_server::CompiledShipFitting>,
            Option<&crate::resources::fitting_server::ShipFitting>,
        ),
    )>,
) {
    let mut order: Vec<_> = owners.iter().map(|row| (row.1 .0.clone(), row.0)).collect();
    order.sort();
    for (_, entity) in order {
        let Ok((
            _,
            _,
            config,
            sources,
            state,
            own,
            transform,
            power,
            profiles_damage,
            profiles,
            mounts,
            (mut readout, fitting_profile, fitting_state),
        )) = owners.get_mut(entity)
        else {
            continue;
        };
        if readout.processed_tick == Some(tick.0) {
            continue;
        }
        readout.entries.clear();
        readout.processed_tick = Some(tick.0);
        let dt = fixed.timestep().as_secs_f32();
        if !dt.is_finite() || dt <= 0.0 {
            continue;
        }
        let Some(mut power) = power else {
            continue;
        };
        let pose = own.map(|own| ([own.x, own.y, own.z], own.yaw)).or_else(|| {
            transform.map(|transform| {
                let (_, yaw, _) = transform.rotation.to_euler(EulerRot::XYZ);
                (transform.translation.to_array(), -yaw)
            })
        });
        let Some((origin, own_yaw)) = pose else {
            continue;
        };
        for owner in state.0.keys() {
            if fitting_profile
                .zip(fitting_state)
                .is_some_and(|(profile, state)| {
                    crate::resources::fitting_server::unavailable(profile, state, owner)
                })
                || !state.enabled(&config.0, owner)
                || sources.0.is_offline(owner)
                || sources.0.source_for(owner)
                    == crate::ship::control_source::ControlSource::Offline
                || !sources.0.entries().any(|(id, _)| id == owner)
            {
                continue;
            }
            let Some(binding) = config
                .0
                .system(owner)
                .and_then(|system| jamming_composition(&system.components))
            else {
                continue;
            };
            let Some(sample) = state
                .target(&config.0, owner)
                .and_then(|target| measurements.sample(entity, owner, target))
                .filter(|sample| {
                    binding.config.sample_fresh(
                        sample.sampled_tick,
                        tick.0,
                        fixed.timestep().as_secs_f64(),
                    )
                })
            else {
                continue;
            };
            let Some(mount) = crate::weapons::mounts::sensor_mount(
                profiles,
                mounts,
                owner,
                &binding.config.mount,
            ) else {
                continue;
            };
            let heading = own_yaw + mount.facing_deg.to_radians();
            if !phoenix_sim_gameplay::weapons::mount::contains_3d(
                origin,
                heading,
                0.0,
                sample.position,
                mount.range,
                mount.fire_arc_deg,
            ) {
                continue;
            }
            let factor = profiles_damage.map_or(1.0, |damage| {
                damage.factor(owner, phoenix_model::damage::DamageBinding::Operation)
            });
            if !factor.is_finite() || factor <= 0.0 {
                continue;
            }
            let Ok(grant) = power.0.work_continuous(owner, dt, 1.0 / factor) else {
                continue;
            };
            if grant.granted <= 0.0 || !grant.fraction.is_finite() || grant.fraction <= 0.0 {
                continue;
            }
            readout.entries.insert(
                owner.clone(),
                PaidEmission {
                    origin,
                    heading,
                    profile: *binding.config,
                    fraction: grant.fraction.clamp(0.0, 1.0),
                },
            );
        }
    }
}

/// Only the shared physical signal query calls this. No target/source identities
/// or membership escape to controller facts or the receiving Console.
pub(crate) fn receiver_exposure(
    mut sources: Vec<(&EntityUuid, &JammingEmissions)>,
    tick: Option<u64>,
    paths: &crate::regions::signal_path::RegionSignalQuery,
    receiver: Vec3,
    effect: phoenix_sim_gameplay::electronic::jamming::JammingEffect,
) -> (f64, bool) {
    sources.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    let mut total = 0.0_f64;
    let mut complete = true;
    for (_, readout) in sources {
        if tick.is_none_or(|tick| readout.processed_tick != Some(tick)) {
            // No paid current work is no physical contribution. Incomplete
            // knowledge cannot create global interference or cancel paid peers.
            complete = false;
            continue;
        }
        for emission in readout
            .entries
            .values()
            .filter(|emission| emission.profile.effect == effect)
        {
            let path = paths.transmission(
                Vec3::from_array(emission.origin),
                receiver,
                match effect {
                    phoenix_sim_gameplay::electronic::jamming::JammingEffect::Comms => {
                        crate::regions::signal_path::SignalClass::Comms
                    }
                    phoenix_sim_gameplay::electronic::jamming::JammingEffect::Sensors => {
                        crate::regions::signal_path::SignalClass::Sensors
                    }
                },
            );
            total += exposure(
                &emission.profile,
                emission.origin,
                emission.heading,
                receiver.to_array(),
                path,
                emission.fraction,
            );
        }
    }
    (total, complete)
}
pub(crate) fn has_emitters(owners: Query<&ShipJamming>) -> bool {
    !owners.is_empty()
}

#[allow(clippy::type_complexity)]
pub(crate) fn operate_ai(
    tick: Res<crate::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    sessions: Res<crate::lobby::Sessions>,
    measurements: GuidanceMeasurements,
    mut owners: Query<(
        Entity,
        Option<&EntityUuid>,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &crate::server_app::ShipSystemBlackboards,
        &crate::science::broad::BroadTracks,
        &ShipJamming,
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
            let Some(binding) = jamming_composition(&system.components) else {
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
            let available = target.is_some_and(|target| {
                measurements
                    .sample(entity, owner, target)
                    .is_some_and(|sample| {
                        binding.config.sample_fresh(
                            sample.sampled_tick,
                            tick.0,
                            fixed.timestep().as_secs_f64(),
                        )
                    })
            });
            let permitted = config.0.observations_for_system(owner, &boards.0);
            let request = if crate::science::reading::mission_text_ready(
                runtime, &config.0, boards, owner, tick.0,
            ) {
                match permitted.get(&crate::ship::system_registry::viewscreen_system_id()) {
                    Some(crate::core::messages::SystemBlackboard::Viewscreen(board)) => {
                        crate::objectives::top_operate_directive(
                            &board.scored_objectives,
                            crate::core::messages::SystemAffinity::Comms,
                            |directive| {
                                matches!(directive, crate::core::messages::AiDirective::Jam { .. })
                            },
                        )
                        .and_then(|directive| match directive {
                            crate::core::messages::AiDirective::Jam { target } => Some(target),
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
                .filter(|alias| {
                    measurements
                        .sample(entity, owner, alias)
                        .is_some_and(|sample| {
                            binding.config.sample_fresh(
                                sample.sampled_tick,
                                tick.0,
                                fixed.timestep().as_secs_f64(),
                            )
                        })
                });
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
                        Control::SetJammingTarget {
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
                    Control::SetJammingEnabled { enabled: *next },
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
    app.declare_state::<ShipJamming>(StateClass::Folded, "ship-jamming-intent")
        .declare_state::<JammingEmissions>(StateClass::Derived, "jamming-paid-emission-view");
    if !app.is_plugin_added::<crate::weapons::mounts::MountsPlugin>() {
        app.add_plugins(crate::weapons::mounts::MountsPlugin);
    }
    crate::ai::cadence::register_ai_cadence(app);
    for (payload, feedback) in [
        (
            Payload::SetJammingTarget,
            &[Payload::SetJammingTarget] as &'static [_],
        ),
        (
            Payload::SetJammingEnabled,
            &[Payload::SetJammingEnabled] as &'static [_],
        ),
    ] {
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(crate::ship::system_registry::JAMMING_KIND, payload)
                .with_feedback(
                    crate::command_admission::FeedbackAddress::DeclaredKindOnly,
                    feedback,
                ),
        );
    }
    app.add_systems(
        FixedUpdate,
        (
            operate_ai
                .in_set(SimSet::Input)
                .in_set(FixedStep::OperateJammingAi)
                .after(FixedStep::ReadSystemText)
                .before(handle_commands)
                .run_if(crate::ai::cadence::ai_tick_ready),
            handle_commands
                .in_set(SimSet::Input)
                .in_set(FixedStep::HandleJammingCommands)
                .before(tick),
            tick.in_set(SimSet::Input)
                .in_set(FixedStep::TickJamming)
                .before(FixedStep::RefreshJammedCommsRange),
            refresh_jammed_comms_range
                .in_set(SimSet::Input)
                .in_set(FixedStep::RefreshJammedCommsRange)
                .before(FixedStep::HandleHail)
                .run_if(has_emitters),
        ),
    );
}

fn refresh_jammed_comms_range(
    comms: ResMut<CommsRuntime>,
    ship_q: Query<
        (
            &Transform,
            Option<&crate::comms::CommsRange>,
            Option<&EntityUuid>,
            Option<&crate::comms::radio::RadioProfile>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    // Every hull the frozen roster put in the world, for the host-neutral half
    // of the same measurement (issue #1343). Read-only, like the two queries
    // beside it, and deliberately a SECOND pass rather than a widening of
    // `ship_q`: the local pass below owns the roster, the open-hail prune and
    // the broadcast stamp, all of which are this host's own screen.
    fleet_q: Query<(
        &crate::lockstep::FleetSlotOf,
        &Transform,
        Option<&crate::comms::CommsRange>,
        Option<&EntityUuid>,
        Option<&crate::comms::radio::RadioProfile>,
    )>,
    entity_q: Query<(
        &crate::entities::spawner::EntityUuid,
        &Transform,
        &crate::comms::CommsRange,
        Option<&crate::comms::CommsHailable>,
        Option<&crate::entities::spawner::EntityName>,
        Option<&crate::comms::radio::RadioProfile>,
    )>,
    signals: crate::regions::signal_path::RegionSignalQuery,
) {
    super::server::update_comms_range_flags(comms, ship_q, fleet_q, entity_q, signals);
}

#[cfg(test)]
#[path = "jamming_tests.rs"]
pub(crate) mod tests;

pub(crate) fn project(
    world: &World,
    entity: Entity,
    owner: &SystemId,
) -> Option<phoenix_model::jamming_ui::JammingControlsView> {
    let config = world.get::<ShipConfigComponent>(entity)?;
    let state = world.get::<ShipJamming>(entity)?;
    if !state.0.contains_key(owner) {
        return None;
    }
    let controllable = world
        .get::<ShipSystemControlSources>(entity)
        .is_some_and(|sources| sources.0.policy_for(owner).accept_human_input);
    let mut view = phoenix_model::jamming_ui::JammingControlsView {
        enabled: state.enabled(&config.0, owner),
        target: state.target(&config.0, owner).map(str::to_owned),
        emitting: world
            .get::<JammingEmissions>(entity)
            .is_some_and(|readout| {
                readout.processed_tick
                    == world
                        .get_resource::<crate::sim_tick::SimTick>()
                        .map(|tick| tick.0)
                    && readout.entries.contains_key(owner)
            }),
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
    let scope = crate::console::weapons::guidance_link::GuidanceContext {
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
            || scope
                .sample(owner, &record.contact_id)
                .is_none_or(|sample| {
                    let Some(binding) = config
                        .0
                        .system(owner)
                        .and_then(|system| jamming_composition(&system.components))
                    else {
                        return true;
                    };
                    world.get_resource::<Time<Fixed>>().is_none_or(|fixed| {
                        !binding.config.sample_fresh(
                            sample.sampled_tick,
                            tick,
                            fixed.timestep().as_secs_f64(),
                        )
                    })
                })
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
