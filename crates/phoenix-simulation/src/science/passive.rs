//! Passive reception measures actual paid machinery emissions. Controllers see
//! only their Console's published bearings, never this producer's scene inputs.
use crate::core::messages::{AdmittedCommands, SystemBlackboard, SystemControlPayload as Control};
use crate::entities::spawner::EntityUuid;
use crate::ship::power::ShipPowerSystem;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use bevy::prelude::*;
use phoenix_model::sensing::{PassiveBlackboard, PassiveConfig, PassiveSignal, PassiveSignature};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Component, Clone)]
pub struct PassiveProfile(pub BTreeMap<crate::core::messages::SystemId, PassiveConfig>);
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassiveState(pub BTreeMap<crate::core::messages::SystemId, PassiveReceiverState>);
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassiveReceiverState {
    pub enabled: bool,
    pub last_tick: Option<u64>,
    pub last_sample_tick: Option<u64>,
    pub signals: Vec<PassiveSignal>,
}
impl PassiveProfile {
    pub fn from_config(config: &crate::entities::config::EntityConfig) -> Option<Self> {
        let profiles: BTreeMap<_, _> = config
            .sensing
            .as_ref()?
            .passive
            .iter()
            .map(|profile| (profile.system.clone(), profile.clone()))
            .collect();
        (!profiles.is_empty()).then_some(Self(profiles))
    }
}
impl PassiveState {
    pub fn new(profile: &PassiveProfile) -> Self {
        Self(
            profile
                .0
                .keys()
                .map(|id| {
                    (
                        id.clone(),
                        PassiveReceiverState {
                            enabled: true,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
        )
    }
    pub fn validate(
        &self,
        profile: &PassiveProfile,
        observer: &str,
        tick: u64,
    ) -> Result<(), String> {
        if !self.0.keys().eq(profile.0.keys()) {
            return Err("passive instrument owner set disagrees with fitted configuration".into());
        }
        for (owner, state) in &self.0 {
            state.validate(&profile.0[owner], observer, tick)?;
        }
        Ok(())
    }
}
impl PassiveReceiverState {
    pub(crate) fn validate(
        &self,
        profile: &PassiveConfig,
        observer: &str,
        tick: u64,
    ) -> Result<(), String> {
        if self.last_tick.is_some_and(|last| last > tick)
            || self
                .last_sample_tick
                .is_some_and(|sample| self.last_tick.is_none_or(|last| sample > last))
            || self.last_sample_tick.is_none() && !self.signals.is_empty()
        {
            return Err("invalid passive observation frontier".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for signal in &self.signals {
            let prefix = format!(
                "{}/{}/{}/",
                observer, profile.system.0, signal.observed_tick
            );
            if Some(signal.observed_tick) != self.last_sample_tick
                || !signal.sample_id.starts_with(&prefix)
                || signal
                    .sample_id
                    .strip_prefix(&prefix)
                    .is_none_or(|index| index.parse::<usize>().is_err())
                || !ids.insert(&signal.sample_id)
                || !signal.bearing_rad.is_finite()
                || signal.bearing_rad.abs() > std::f32::consts::PI
                || signal.bearing_error_rad != profile.bearing_error_rad
                || !signal.strength.is_finite()
                || signal.strength <= 0.0
                || signal.strength < profile.min_signal_strength
            {
                return Err("invalid permitted passive sample or provenance".into());
            }
        }
        Ok(())
    }
}
pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(profile) = PassiveProfile::from_config(config) {
        let state = PassiveState::new(&profile);
        commands.insert((profile, state));
    }
}
pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    app.declare_state::<PassiveProfile>(StateClass::Derived, "independent-passive-receiver")
        .declare_state::<PassiveState>(StateClass::Folded, "independent-passive-receiver")
        .register_admitted_consumer(
            ConsumerMatcher::kind(crate::ship::system_registry::PASSIVE_RECEIVER_KIND)
                .with_feedback(
                    FeedbackAddress::DeclaredKindOnly,
                    &[crate::core::messages::SystemControlPayloadDiscriminants::SetPassiveEnabled],
                ),
        );
    app.add_systems(
        FixedUpdate,
        operate_ai
            .in_set(crate::sim_sets::SimSet::Input)
            .run_if(crate::ai::cadence::ai_tick_ready),
    );
    app.add_systems(
        FixedUpdate,
        tick.in_set(crate::sim_sets::SimSet::Modifiers)
            .after(super::server::tick_scans),
    );
    app.add_systems(
        FixedUpdate,
        publish.in_set(crate::sim_sets::SimSet::Publish),
    );
}
fn operate_ai(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        Option<&EntityUuid>,
        &PassiveProfile,
        &PassiveState,
        &ShipSystemControlSources,
        &ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (uuid, profile, state, sources, config, boards, programs, mut runtimes, mut admitted) in
        &mut ships
    {
        for (system, profile) in &profile.0 {
            let Some(state) = state.0.get(system) else {
                continue;
            };
            if !crate::ai::host::ai_operates(&sources.0, &profile.system) {
                continue;
            }
            let permitted = config.0.observations_for_system(&profile.system, &boards.0);
            let Some(SystemBlackboard::Passive(board)) = permitted.get(&profile.system) else {
                continue;
            };
            let Some(runtime) = runtimes.0.get_mut(&profile.system) else {
                continue;
            };
            let facts = BTreeMap::from([
                ("enabled".into(), board.enabled),
                ("can_operate".into(), board.can_operate),
            ]);
            let channels = BTreeMap::from([("operate".into(), false)]);
            let Ok(TreeHostOutcome::Evaluated(result)) = decide_tree(
                &sources.0,
                &profile.system,
                programs.0.get(&profile.system),
                &facts,
                &channels,
                runtime,
                tick.0,
            ) else {
                continue;
            };
            let enabled = result.intents.get("operate") == Some(&true);
            if enabled != state.enabled {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
                    profile.system.clone(),
                    Control::SetPassiveEnabled { enabled },
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
    }
}

type Emitters<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        &'static Transform,
        &'static ShipPowerSystem,
        Option<&'static ShipConfigComponent>,
    ),
>;
type Receivers<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        &'static Transform,
        &'static PassiveProfile,
        &'static mut PassiveState,
        &'static mut ShipPowerSystem,
        &'static AdmittedCommands,
        &'static ShipSystemControlSources,
        Option<&'static crate::ship::compartment_damage::ShipDamage>,
        Option<&'static crate::ship::state::ShipPhysics>,
        Option<&'static crate::weapons::mounts::MountProfiles>,
        Option<&'static crate::weapons::mounts::ShipMounts>,
    ),
>;

fn tick(
    time: Res<Time>,
    fixed: Option<Res<Time<Fixed>>>,
    tick: Res<crate::sim_tick::SimTick>,
    paths: crate::regions::signal_path::RegionSignalQuery,
    mut scene: ParamSet<(Emitters, Receivers)>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    let dt = fixed.map_or_else(|| time.delta_secs(), |fixed| fixed.timestep().as_secs_f32());
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let mut emissions: Vec<_> = scene
        .p0()
        .iter()
        .filter_map(|(uuid, transform, power, config)| {
            let mut strength = if power.0.network().generation_enabled {
                f64::from(power.0.network().output_actual_per_sec)
            } else {
                0.0
            };
            let mut signature = PassiveSignature::Reactor;
            // The strongest actual machinery cause labels this bearing. Local
            // reserves can operate equipment with reactor generation disabled.
            if let Some(config) = config {
                for system in &config.0.systems {
                    let kind = system.kind.as_str();
                    let measured = power.0.emitted_work(&system.id) / f64::from(dt);
                    let cause = if [
                        crate::ship::system_registry::HELM_ENGINE_KIND,
                        crate::ship::system_registry::HELM_THRUST_KIND,
                        crate::ship::system_registry::HELM_BOOST_KIND,
                    ]
                    .contains(&kind)
                    {
                        PassiveSignature::Propulsion
                    } else if kind == crate::ship::system_registry::SENSORS_KIND {
                        PassiveSignature::ActiveScan
                    } else if [
                        crate::ship::system_registry::PHASER_BANK_KIND,
                        crate::ship::system_registry::BLASTER_BANK_KIND,
                        crate::ship::system_registry::TORPEDO_TUBE_KIND,
                    ]
                    .contains(&kind)
                    {
                        PassiveSignature::Weapon
                    } else {
                        continue;
                    };
                    if measured > strength {
                        strength = measured;
                        signature = cause;
                    }
                }
            }
            (strength > 0.0).then(|| {
                (
                    uuid.0.clone(),
                    transform.translation,
                    strength.min(f64::from(f32::MAX)) as f32,
                    signature,
                )
            })
        })
        .collect();
    emissions.sort_by(|a, b| {
        a.1.x
            .total_cmp(&b.1.x)
            .then(a.1.z.total_cmp(&b.1.z))
            .then(a.1.y.total_cmp(&b.1.y))
            .then(a.2.total_cmp(&b.2))
            .then((a.3 as u8).cmp(&(b.3 as u8)))
            .then(a.0.cmp(&b.0))
    });
    let mut receivers = scene.p1();
    let mut order: Vec<_> = receivers.iter().map(|(uuid, ..)| uuid.0.clone()).collect();
    order.sort();
    for id in order {
        let Some((
            uuid,
            transform,
            profile,
            mut state,
            mut power,
            admitted,
            sources,
            damage,
            physics,
            mount_profiles,
            mount_states,
        )) = receivers.iter_mut().find(|(uuid, ..)| uuid.0 == id)
        else {
            continue;
        };
        for (system, profile) in &profile.0 {
            let state = state
                .0
                .get_mut(system)
                .expect("installed passive instrument owns state");
            for command in admitted.for_target(&profile.system.0) {
                if let Control::SetPassiveEnabled { enabled } = command.payload {
                    state.enabled = enabled;
                    crate::command_admission::finish_action_feedback(
                        command,
                        &mut outbound,
                        crate::core::messages::ActionFeedbackOutcome::Applied,
                    );
                }
            }
            if state.last_tick == Some(tick.0) {
                continue;
            }
            state.last_tick = Some(tick.0);
            let factor = damage.map_or(1.0, |damage| {
                damage.factor(
                    &profile.system,
                    phoenix_model::damage::DamageBinding::Operation,
                )
            });
            if !state.enabled
                || factor <= 0.0
                || !sources.0.entries().any(|(id, _)| id == system)
                || sources.0.is_offline(&profile.system)
                || sources.0.source_for(system)
                    == crate::ship::control_source::ControlSource::Offline
            {
                continue;
            }
            let static_mount = phoenix_sim_gameplay::science::sensing::mount(&profile.mount);
            let Some(mut mount) = crate::weapons::mounts::sensor_mount(
                mount_profiles,
                mount_states,
                system,
                &static_mount,
            ) else {
                continue;
            };
            let paid = power
                .0
                .work_continuous(&profile.system, dt, 1.0)
                .map_or(0.0, |grant| grant.fraction * factor);
            if paid <= 0.0
                || state.last_sample_tick.is_some_and(|last| {
                    tick.0.saturating_sub(last) as f64 * (dt as f64)
                        < profile.sample_interval_secs as f64
                })
            {
                continue;
            }
            mount.range *=
                paid * paths.sensors_receiver_factor(transform.translation, &uuid.0, system);
            let mut signals = Vec::new();
            for (source, position, emitted, signature) in &emissions {
                if source == &uuid.0 {
                    continue;
                }
                let geometry = mount.geometry(
                    position.x,
                    position.z,
                    transform.translation.x,
                    transform.translation.z,
                    physics.map_or(0.0, |physics| physics.yaw),
                );
                if !geometry.in_arc || !geometry.in_range {
                    continue;
                }
                let distance_squared = (position.as_dvec3() - transform.translation.as_dvec3())
                    .length_squared()
                    .max(f64::from(f32::EPSILON));
                let strength = ((*emitted as f64 / distance_squared)
                    * paths.transmission(
                        transform.translation,
                        *position,
                        crate::regions::signal_path::SignalClass::Sensors,
                    ) as f64)
                    .min(f32::MAX as f64) as f32;
                if strength <= 0.0 || strength < profile.min_signal_strength {
                    continue;
                }
                let delta = *position - transform.translation;
                let bearing = crate::simmath::atan2(delta.x, -delta.z);
                let width = 2.0 * profile.bearing_error_rad;
                let bearing = if width > 0.0 {
                    ((bearing / width).floor() + 0.5) * width
                } else {
                    bearing
                };
                let bearing = wrap_bearing(bearing);
                signals.push(PassiveSignal {
                    sample_id: String::new(),
                    observed_tick: tick.0,
                    bearing_rad: bearing,
                    bearing_error_rad: profile.bearing_error_rad,
                    strength,
                    signature: *signature,
                });
            }
            // Anonymous provenance must not encode the rank of a hidden actor ID.
            signals.sort_by(|a, b| {
                a.bearing_rad
                    .total_cmp(&b.bearing_rad)
                    .then(a.strength.total_cmp(&b.strength))
                    .then((a.signature as u8).cmp(&(b.signature as u8)))
            });
            for (index, signal) in signals.iter_mut().enumerate() {
                signal.sample_id = format!("{}/{}/{}/{}", uuid.0, profile.system.0, tick.0, index);
            }
            state.last_sample_tick = Some(tick.0);
            state.signals = signals;
        }
    }
}
fn wrap_bearing(angle: f32) -> f32 {
    let pi = std::f32::consts::PI;
    (angle + pi).rem_euclid(2.0 * pi) - pi
}

#[cfg(test)]
#[path = "passive_tests.rs"]
mod tests;
pub(crate) fn publish(
    mut ships: Query<(
        &PassiveProfile,
        &PassiveState,
        &ShipPowerSystem,
        &ShipSystemControlSources,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        &mut crate::server_app::ShipSystemBlackboards,
        Option<&crate::weapons::mounts::MountProfiles>,
        Option<&crate::weapons::mounts::ShipMounts>,
    )>,
) {
    for (profile, state, power, sources, damage, mut boards, mount_profiles, mount_states) in
        &mut ships
    {
        for (system, profile) in &profile.0 {
            let Some(state) = state.0.get(system) else {
                continue;
            };
            let actual_mount = crate::weapons::mounts::sensor_mount(
                mount_profiles,
                mount_states,
                system,
                &phoenix_sim_gameplay::science::sensing::mount(&profile.mount),
            );
            let board = SystemBlackboard::Passive(PassiveBlackboard {
                enabled: state.enabled,
                can_operate: actual_mount.is_some()
                    && sources.0.entries().any(|(id, _)| id == system)
                    && sources.0.source_for(system)
                        != crate::ship::control_source::ControlSource::Offline
                    && !sources.0.is_offline(&profile.system)
                    && damage.is_none_or(|damage| damage.operational(&profile.system))
                    && power
                        .0
                        .network()
                        .consumers
                        .get(&profile.system)
                        .is_some_and(|energy| energy.energy > 0.0 && !energy.thermal_stopped),
                mount: actual_mount.map(|mount| phoenix_model::sensing::SensorMountConfig {
                    facing_deg: mount.facing_deg,
                    arc_deg: mount.fire_arc_deg,
                    range: mount.range,
                }),
                last_sample_tick: state.last_sample_tick,
                signals: state.signals.clone(),
            });
            if boards.0.get(&profile.system) != Some(&board) {
                boards.0.insert(profile.system.clone(), board);
            }
        }
    }
}

#[cfg(test)]
#[path = "passive_interference_tests.rs"]
mod interference_tests;
