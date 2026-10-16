//! One physical turret per actual composed System. Target/enabled intent stays
//! with its existing System controller; no mount target or AI copy is saved.
use crate::core::messages::SystemId;
use bevy::prelude::*;
use phoenix_sim_gameplay::weapons::{
    mount::MountConfig,
    turret::{TurretConfig, TurretState},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Component, Clone, Debug, PartialEq)]
pub struct MountProfiles(pub BTreeMap<SystemId, TurretConfig>);
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipMounts(pub BTreeMap<SystemId, TurretState>);

/// Weapon-only and Science-only hosts install the same physical owner once.
pub struct MountsPlugin;
impl Plugin for MountsPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::ship::emp::ComponentEffectsPlugin>() {
            app.add_plugins(crate::ship::emp::ComponentEffectsPlugin);
        }
        register(app);
    }
}

impl MountProfiles {
    pub fn from_ship(config: &crate::ship::config::ShipConfig) -> Result<Option<Self>, String> {
        let mut profiles = BTreeMap::new();
        for system in &config.systems {
            crate::ship::composition::validate_composition(system)?;
            if let Some(binding) = crate::ship::composition::turret_composition(&system.components)
            {
                profiles.insert(system.id.clone(), binding.config.clone());
            }
        }
        Ok((!profiles.is_empty()).then_some(Self(profiles)))
    }
}
impl ShipMounts {
    pub fn new(profiles: &MountProfiles) -> Self {
        Self(
            profiles
                .0
                .keys()
                .cloned()
                .map(|owner| (owner, TurretState::default()))
                .collect(),
        )
    }
    pub fn validate(&self, profiles: &MountProfiles, tick: u64) -> Result<(), String> {
        if !self.0.keys().eq(profiles.0.keys()) {
            return Err("turret owner set disagrees with fitted components".into());
        }
        for (owner, state) in &self.0 {
            state.validate(&profiles.0[owner], tick)?;
        }
        Ok(())
    }
}

/// A missing state for authored hardware refuses readiness. An actual System
/// without a turret retains its original fixed mount geometry exactly.
pub fn geometry(
    profiles: Option<&MountProfiles>,
    states: Option<&ShipMounts>,
    owner: &SystemId,
    mount: &MountConfig,
    target: [f32; 2],
    own: [f32; 2],
    yaw: f32,
) -> crate::core::messages::WeaponTargetGeometry {
    let Some(config) = profiles.and_then(|profiles| profiles.0.get(owner)) else {
        return mount.geometry(target[0], target[1], own[0], own[1], yaw);
    };
    let Some(state) = states.and_then(|states| states.0.get(owner)) else {
        let mut result = mount.geometry(target[0], target[1], own[0], own[1], yaw);
        result.in_arc = false;
        return result;
    };
    state.geometry(config, mount, target, own, yaw)
}

/// A receiver's existing field of view rotates with the same physical mount;
/// it is not narrowed to a weapon's pointing tolerance for every signal.
pub fn sensor_mount(
    profiles: Option<&MountProfiles>,
    states: Option<&ShipMounts>,
    owner: &SystemId,
    mount: &MountConfig,
) -> Option<MountConfig> {
    let Some(config) = profiles.and_then(|profiles| profiles.0.get(owner)) else {
        return Some(*mount);
    };
    let state = states?.0.get(owner)?;
    Some(MountConfig {
        facing_deg: state.bearing_deg(config),
        ..*mount
    })
}

pub fn measured_point(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
    tracks: &crate::science::broad::BroadTracks,
    owner: &SystemId,
    contact: &str,
) -> Option<[f32; 2]> {
    measured_contact(config, boards, tracks, owner, contact).map(|(_, point, _)| point)
}

pub fn measured_elevation(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
    tracks: &crate::science::broad::BroadTracks,
    owner: &SystemId,
    contact: &str,
) -> Option<phoenix_model::sensing_tracks::ObservedElevation> {
    measured_contact(config, boards, tracks, owner, contact)?.2
}

fn measured_contact(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
    tracks: &crate::science::broad::BroadTracks,
    owner: &SystemId,
    contact: &str,
) -> Option<(
    String,
    [f32; 2],
    Option<phoenix_model::sensing_tracks::ObservedElevation>,
)> {
    let view = config.observations_for_system(owner, &boards.0);
    let mut found = None;
    for (instrument, state) in &tracks.0 {
        let Some(public) = view
            .get(instrument)
            .and_then(crate::science::contact_routing::observations)
        else {
            continue;
        };
        for record in public
            .iter()
            .filter(|record| record.contact_id == contact && !record.predicted && record.selectable)
        {
            let Some(route) = state.resolve_current(contact) else {
                continue;
            };
            if record.position.iter().any(|value| !value.is_finite())
                || record
                    .elevation
                    .as_ref()
                    .is_some_and(|value| value.validate().is_err())
            {
                return None;
            }
            let candidate = (
                route.to_owned(),
                record.position,
                record.velocity,
                record.elevation,
            );
            if found
                .as_ref()
                .is_some_and(|previous| previous != &candidate)
            {
                return None;
            }
            found = Some(candidate);
        }
    }
    found.map(|(route, point, _, elevation)| (route, point, elevation))
}

fn current_point(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
    tracks: &crate::science::broad::BroadTracks,
    owner: &SystemId,
    contact: &str,
    observer: &str,
    overrides: Option<&crate::gm::operations::contact::ContactOverrides>,
) -> Option<[f32; 2]> {
    let (source, point, _) = measured_contact(config, boards, tracks, owner, contact)?;
    if overrides.is_some_and(|overrides| {
        crate::gm::operations::contact::mode(overrides, observer, &source)
            == crate::gm::operations::contact::ContactMode::Conceal
    }) {
        return None;
    }
    Some(point)
}

/// Stage energy and bearing together. No payment for a replay, disabled owner,
/// zero movement or absent hardware; no queued movement credit is retained.
#[allow(clippy::too_many_arguments)]
pub fn advance_system(
    profiles: &MountProfiles,
    states: &mut ShipMounts,
    owner: &SystemId,
    tick: u64,
    dt: f32,
    enabled: bool,
    desired_deg: Option<f32>,
    factor: f32,
    power: &mut crate::ship::power::ShipPowerSystem,
) -> Result<(), String> {
    let config = profiles.0.get(owner).ok_or("unknown mounted System")?;
    let state = states.0.get(owner).ok_or("missing physical turret state")?;
    state.validate(config, tick)?;
    if !dt.is_finite() || dt < 0.0 || !factor.is_finite() || !(0.0..=1.0).contains(&factor) {
        return Err("invalid turret duration or actual operation fraction".into());
    }
    let movement = state.movement_deg(config, desired_deg)?.abs();
    if state.last_tick == Some(tick) {
        return Ok(());
    }
    let offered = (movement / f64::from(config.traverse_deg_per_sec)).min(f64::from(dt)) as f32;
    let mut next = state.clone();
    if !enabled || factor == 0.0 || offered == 0.0 {
        next.advance(config, tick, 0.0, false, desired_deg, 0.0)?;
        states.0.insert(owner.clone(), next);
        return Ok(());
    }
    let next = power
        .0
        .work_continuous_if(owner, offered, 1.0, |grant| {
            next.advance(
                config,
                tick,
                offered,
                true,
                desired_deg,
                grant.fraction * factor,
            )?;
            Ok(Some(next))
        })?
        .expect("successful turret advance accepts its grant");
    states.0.insert(owner.clone(), next);
    Ok(())
}

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(profiles) = config
        .ship_config
        .as_ref()
        .and_then(|ship| MountProfiles::from_ship(ship).expect("accepted turret composition"))
    {
        let states = ShipMounts::new(&profiles);
        commands.insert((profiles, states));
    }
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<MountProfiles>(StateClass::Derived, "finite-system-turrets")
        .declare_state::<ShipMounts>(StateClass::Folded, "finite-system-turrets")
        .add_systems(
            FixedUpdate,
            tick.in_set(crate::sim_sets::SimSet::Physics)
                .in_set(crate::sim_sets::FixedStep::TickMounts),
        );
}

/// Borrow the receiver's ordinary Science designation through its actual feed,
/// then take aim from the corresponding held measured point, never a scene pose.
fn passive_contact(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
    tracks: Option<&crate::science::broad::BroadTracks>,
    controls: Option<&crate::science::broad::SensingControls>,
    owner: &SystemId,
) -> Option<String> {
    let science = crate::science::broad::science_owner(controls, tracks.is_some())?;
    let view = config.observations_for_system(owner, &boards.0);
    let crate::core::messages::SystemBlackboard::Sensors(board) = view.get(&science)? else {
        return None;
    };
    let selected = board.science_target_uuid.as_deref()?;
    let tracks = tracks?;
    if crate::science::contact_routing::resolve(config, boards, Some(tracks), owner, selected)
        .is_some()
    {
        return Some(selected.to_owned());
    }
    crate::science::contact_routing::alias_for_source(config, boards, Some(tracks), owner, selected)
}

#[allow(clippy::type_complexity)]
fn tick(
    time: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    blasters: Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::console::weapons::BlasterSystemResource,
    )>,
    mut ships: Query<(
        &crate::entities::spawner::EntityUuid,
        &MountProfiles,
        &mut ShipMounts,
        &crate::ship_plugin::ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        &crate::ship_plugin::ShipSystemControlSources,
        &mut crate::ship::power::ShipPowerSystem,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        (Option<&crate::ship::state::ShipPhysics>, &Transform),
        Option<&crate::science::broad::BroadTracks>,
        Option<&crate::science::broad::SensingControls>,
        Option<&crate::console::weapons::assignments::ShipWeaponTargets>,
        (
            Option<&crate::science::passive::PassiveState>,
            Option<&crate::console::weapons::illumination::ShipIllumination>,
            Option<&crate::comms::jamming::ShipJamming>,
        ),
        &crate::core::messages::AdmittedCommands,
        Option<(
            &crate::resources::fitting_server::CompiledShipFitting,
            &crate::resources::fitting_server::ShipFitting,
        )>,
    )>,
) {
    let dt = time.timestep().as_secs_f32();
    let mut order: Vec<_> = ships.iter().map(|(uuid, ..)| uuid.0.clone()).collect();
    order.sort();
    for uuid in order {
        let Some((
            _,
            profiles,
            mut states,
            config,
            boards,
            sources,
            mut power,
            damage,
            (physics, transform),
            tracks,
            controls,
            assignments,
            (passive, illumination, jamming),
            admitted,
            fitting,
        )) = ships.iter_mut().find(|(id, ..)| id.0 == uuid)
        else {
            continue;
        };
        let own = physics.map_or(
            [transform.translation.x, transform.translation.z],
            |physics| [physics.x, physics.z],
        );
        let yaw = physics.map_or_else(
            || transform.rotation.to_euler(EulerRot::YXZ).0,
            |physics| physics.yaw,
        );
        for owner in profiles.0.keys() {
            let is_passive = config.0.system(owner).is_some_and(|system| {
                system.kind == crate::ship::system_registry::PASSIVE_RECEIVER_KIND
            });
            let is_illumination = config.0.system(owner).is_some_and(|system| {
                system.kind == crate::ship::system_registry::ILLUMINATION_KIND
            });
            let is_jamming = config
                .0
                .system(owner)
                .is_some_and(|system| system.kind == crate::ship::system_registry::JAMMING_KIND);
            let contact = if is_jamming {
                jamming
                    .and_then(|state| state.target(&config.0, owner))
                    .map(str::to_owned)
            } else if is_illumination {
                illumination
                    .and_then(|state| state.target(&config.0, owner))
                    .map(str::to_owned)
            } else if is_passive {
                passive_contact(&config.0, boards, tracks, controls, owner)
            } else {
                let filter = blasters
                    .iter()
                    .find(|(id, _)| id.0 == uuid)
                    .and_then(|(_, banks)| {
                        banks.0.iter().find(|bank| {
                            crate::ship::system_registry::blaster_bank_system_id(&bank.config.id)
                                .as_ref()
                                == Some(owner)
                        })
                    })
                    .and_then(|bank| bank.config.targets.as_deref());
                crate::console::weapons::assignments::effective_weapon_contact_filtered(
                    &config.0,
                    owner,
                    boards,
                    tracks,
                    controls,
                    assignments,
                    filter,
                )
            };
            let point = contact.as_deref().and_then(|contact| {
                if is_jamming {
                    let binding = phoenix_sim_gameplay::ship::composition::jamming_composition(
                        &config.0.system(owner)?.components,
                    )?;
                    let observer = crate::entities::spawner::EntityUuid(uuid.clone());
                    let context = crate::console::weapons::guidance_link::GuidanceContext {
                        observer: &observer,
                        config,
                        boards,
                        tracks: tracks?,
                        controls,
                        tick: tick.0,
                        runtime: runtime.as_deref(),
                    };
                    return context
                        .sample(owner, contact)
                        .filter(|sample| {
                            binding.config.sample_fresh(
                                sample.sampled_tick,
                                tick.0,
                                time.timestep().as_secs_f64(),
                            )
                        })
                        .map(|sample| [sample.position[0], sample.position[2]]);
                }
                current_point(
                    &config.0,
                    boards,
                    tracks?,
                    owner,
                    contact,
                    &uuid,
                    runtime.as_ref().map(|runtime| &runtime.contact_overrides),
                )
            });
            let desired = point.map(|point| {
                let (x, y) = phoenix_sim_gameplay::weapons::mount::ship_local(
                    point[0], point[1], own[0], own[1], yaw,
                );
                crate::simmath::atan2(x, y).to_degrees()
            });
            let receiver_enabled = !is_passive
                || admitted
                    .for_target(&owner.0)
                    .filter_map(|command| {
                        if let crate::core::messages::SystemControlPayload::SetPassiveEnabled {
                            enabled,
                        } = command.payload
                        {
                            Some(enabled)
                        } else {
                            None
                        }
                    })
                    .last()
                    .or_else(|| {
                        passive
                            .and_then(|state| state.0.get(owner))
                            .map(|state| state.enabled)
                    })
                    .unwrap_or(false);
            let emitter_enabled = !is_illumination
                || illumination.is_some_and(|state| state.enabled(&config.0, owner));
            let jammer_enabled =
                !is_jamming || jamming.is_some_and(|state| state.enabled(&config.0, owner));
            let enabled = receiver_enabled
                && emitter_enabled
                && jammer_enabled
                && fitting.is_none_or(|(profile, state)| {
                    !crate::resources::fitting_server::unavailable(profile, state, owner)
                })
                && sources.0.entries().any(|(id, _)| id == owner)
                && !sources.0.is_offline(owner)
                && sources.0.source_for(owner)
                    != crate::ship::control_source::ControlSource::Offline;
            let factor = damage.map_or(1.0, |damage| {
                damage.factor(owner, phoenix_model::damage::DamageBinding::Operation)
            });
            if let Err(error) = advance_system(
                profiles,
                &mut states,
                owner,
                tick.0,
                dt,
                enabled,
                desired,
                factor,
                &mut power,
            ) {
                warn!(system=%owner.0, %error, "turret advancement refused");
            }
        }
    }
}

#[cfg(test)]
#[path = "mounts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "mounts_payment_reference.rs"]
mod payment_reference;
