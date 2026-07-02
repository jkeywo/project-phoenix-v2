use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::collections::HashMap;

use crate::control_source::{ControlSourceResolver, ControlTickPolicy};
use crate::damage::DamageTier;
use crate::entity_spawner::RegionEffectsSection;
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{
    AdmittedCommands, ClientMessage, CoordinationPayload, InterSystemMsg, InterSystemPayload,
    InterSystemQueue, ModifierSlot, StationId, SystemControlPayload,
};
use crate::modifiers::ShipModifiers;
use crate::region_effects::RegionEffectKind;
use crate::region_plugin::RegionMembership;
use crate::ship::config::ShipConfig;
use crate::ship::control_source::ControlSource;
use crate::ship::coordination;
use crate::ship::coordination::{CoordinationLagQueue, QueuedCoordination};
use crate::ship::rating;
use crate::ship_physics::{compute_physics, ShipPhysicsConfig, ShipPhysicsInput, ShipPhysicsState};
use crate::ship_state::ShipPhysics;
use crate::simulation::{ShipBoost, ShipImpulse};
use crate::server_app::{LocalShip, Ship};

// Ã¢â€â‚¬Ã¢â€â‚¬ Resources Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

#[derive(Resource)]
struct HelmInputTimer(Timer);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct LastHelmInput {
    pub thrust: f32,
    pub steering: f32,
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ShipSystemControlSources(pub ControlSourceResolver);

/// The parsed `ShipConfig` defining stations, systems, and per-station rating
/// tables. Populated once at startup from the embedded ship TOML.
#[derive(Component, Clone)]
pub struct ShipConfigComponent(pub ShipConfig);

/// Tracks the currently active rating name for each station.
/// Updated when a player sends `SetStationRating`.
#[derive(Component, Clone, Debug, Default)]
pub struct ActiveStationRatings(pub HashMap<StationId, String>);

/// Channel-3 coordination lag queue. Holds pending coordination messages
/// until their due time, at which point they are routed by the delivery-time
/// matrix (issue #494).
#[derive(Component, Clone, Debug, Default)]
pub struct CoordinationQueue(pub CoordinationLagQueue);

/// Newtype resource used by the WASM bridge to pass a custom `ShipConfig`
/// before the ship entity is spawned.  Consumed during ship spawn and then
/// removed from the world.
#[derive(Resource)]
pub struct PendingShipConfig(pub ShipConfig);

/// Server-side enqueue event for channel-3 coordination messages.
/// AI controllers fire this to send delayed advisories to human operators.
///
/// `source_entity` identifies the ship the coordination belongs to. At
/// delivery time, the message will be enqueued into that ship's own
/// `CoordinationQueue` component and routed against that ship's
/// `ShipSystemControlSources` + `ShipConfigComponent`. NPC ships (no
/// `LocalShip` marker) drain silently — popups are only emitted for the
/// LocalShip because that's the only ship with a human console holder.
#[derive(Message, Clone, Debug)]
pub struct CoordinationEnqueue {
    pub source_entity: Entity,
    pub sender_origin: ControlSource,
    pub target: crate::messages::SystemId,
    pub payload: CoordinationPayload,
    pub sender_label: String,
}

/// Load `ShipConfigComponent` from `assets/entities/player_ship.toml` (embedded at compile time).
///
/// Panics if the file fails validation — the server cannot start without a valid ship
/// configuration.
pub(crate) fn load_ship_config_from_disk() -> ShipConfigComponent {
    let toml_str = include_str!("../assets/entities/player_ship.toml");
    let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
        .expect("core system registry must be valid");
    let kinds: Vec<&str> = registry.kinds().collect();
    match crate::ship::config::parse_and_validate(&toml_str, &kinds) {
        Ok(config) => {
            bevy::log::info!(
                "ship_config: loaded {} stations, {} systems",
                config.stations.len(),
                config.systems.len()
            );
            ShipConfigComponent(config)
        }
        Err(e) => panic!("ship_config: failed validation: {e}"),
    }
}

impl Default for ShipConfigComponent {
    fn default() -> Self {
        load_ship_config_from_disk()
    }
}

/// Runtime ship physics config, loaded from `[helm_console]` in the entity TOML.
/// When absent, `ShipPhysicsConfig::new()` defaults are used.
/// Dual-derives `Resource` (for tests + global fallback) and `Component`
/// (per-entity component on each ship — PR 4 migration, see PRD #597).
#[derive(Resource, Component, Clone)]
pub struct ShipPhysicsConfigResource(pub crate::ship_physics::ShipPhysicsConfig);

/// Runtime impulse drive config, loaded from `[helm_console]` in the entity TOML.
/// Charge duration and speed multiplier can be overridden per ship.
/// Dual-derives `Resource` (for tests + global fallback) and `Component`
/// (per-entity component on each ship — PR 4 migration, see PRD #597).
#[derive(Resource, Component, Clone)]
pub struct ImpulseConfigResource {
    pub charge_duration: f32,
    pub speed_multiplier: f32,
    pub acceleration_multiplier: f32,
}

impl Default for ImpulseConfigResource {
    fn default() -> Self {
        Self {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER,
        }
    }
}

/// Runtime boost drive config, loaded from `[helm_console.boost]` in the entity
/// TOML. `enabled` is false (the default) when the TOML omits the table, which
/// disables the feature entirely.
/// Dual-derives `Resource` (for tests + global fallback) and `Component`
/// (per-entity component on each ship — PR 4 migration, see PRD #597).
#[derive(Resource, Component, Clone)]
pub struct BoostConfigResource {
    pub enabled: bool,
    pub multiplier: f32,
    pub steering_multiplier: f32,
    pub active_duration: f32,
    pub recharge_duration: f32,
}

impl Default for BoostConfigResource {
    fn default() -> Self {
        Self {
            enabled: false,
            multiplier: crate::boost::BOOST_MULTIPLIER,
            steering_multiplier: crate::boost::BOOST_STEERING_MULTIPLIER,
            active_duration: crate::boost::BOOST_ACTIVE_DURATION,
            recharge_duration: crate::boost::BOOST_RECHARGE_DURATION,
        }
    }
}

/// Runtime banking config, loaded from `[helm_console] max_bank_deg` in the entity TOML.
/// Dual-derives `Resource` (for tests + global fallback) and `Component`
/// (per-entity component on each ship — PR 4 migration, see PRD #597).
#[derive(Resource, Component, Clone)]
pub struct BankConfigResource {
    pub max_bank_deg: f32,
    pub bank_lerp_rate: f32,
}

impl Default for BankConfigResource {
    fn default() -> Self {
        Self {
            max_bank_deg: 0.0,
            bank_lerp_rate: BANK_LERP_RATE,
        }
    }
}

/// How quickly the ship's visual roll lerps toward the target bank angle.
/// Used as the serde default for `HelmConsoleConfig::bank_lerp_rate`.
pub const BANK_LERP_RATE: f32 = 5.0;

// Ã¢â€â‚¬Ã¢â€â‚¬ Plugin Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

pub struct ShipPlugin;

/// Physics and drive config components bundled so `process_helm_inputs` stays
/// under Bevy's 16-parameter system-function limit.
///
/// PR 4 (PRD #597): configs are now per-entity Components on each ship entity.
/// The Resource variants are kept as fallbacks for test environments that still
/// use `insert_resource` without inserting a LocalShip entity with components.
#[derive(SystemParam)]
struct HelmDriveParams<'w, 's> {
    /// Per-entity drive configs on the LocalShip entity (PR 4 primary path).
    config_q: Query<
        'w,
        's,
        (
            Option<&'static ShipPhysicsConfigResource>,
            Option<&'static ImpulseConfigResource>,
            Option<&'static BoostConfigResource>,
            Option<&'static BankConfigResource>,
        ),
        With<LocalShip>,
    >,
    /// Resource fallbacks (legacy path; used by tests that insert_resource
    /// without spawning a ship entity with the component).
    physics_cfg_res: Option<Res<'w, ShipPhysicsConfigResource>>,
    impulse_cfg_res: Option<Res<'w, ImpulseConfigResource>>,
    boost_cfg_res: Option<Res<'w, BoostConfigResource>>,
    bank_cfg_res: Option<Res<'w, BankConfigResource>>,
    /// Per-entity impulse state on the LocalShip (Component, not Resource).
    impulse_q: Query<'w, 's, &'static ShipImpulse, With<LocalShip>>,
    boost: Res<'w, ShipBoost>,
}

impl HelmDriveParams<'_, '_> {
    /// Effective impulse config: per-entity component takes priority over Resource.
    fn impulse_cfg(&self) -> ImpulseConfigResource {
        let entity = self.config_q.single().ok().and_then(|(_, ic, _, _)| ic.cloned());
        entity
            .or_else(|| self.impulse_cfg_res.as_deref().cloned())
            .unwrap_or_default()
    }

    /// Effective boost config: per-entity component takes priority over Resource.
    fn boost_cfg(&self) -> BoostConfigResource {
        let entity = self.config_q.single().ok().and_then(|(_, _, bc, _)| bc.cloned());
        entity
            .or_else(|| self.boost_cfg_res.as_deref().cloned())
            .unwrap_or_default()
    }

    /// Effective bank config: per-entity component takes priority over Resource.
    fn bank_cfg(&self) -> BankConfigResource {
        let entity = self.config_q.single().ok().and_then(|(_, _, _, bk)| bk.cloned());
        entity
            .or_else(|| self.bank_cfg_res.as_deref().cloned())
            .unwrap_or_default()
    }

    /// Effective physics config: per-entity component takes priority over Resource.
    fn physics_cfg(&self) -> Option<ShipPhysicsConfigResource> {
        let entity = self.config_q.single().ok().and_then(|(pc, _, _, _)| pc.cloned());
        entity.or_else(|| self.physics_cfg_res.as_deref().cloned())
    }
}

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmInputTimer(Timer::from_seconds(
            1.0 / 30.0,
            TimerMode::Repeating,
        )))
        .init_resource::<ImpulseConfigResource>()
        .init_resource::<BoostConfigResource>()
        .init_resource::<ShipBoost>()
        .init_resource::<BankConfigResource>()
        .add_message::<CoordinationEnqueue>()
        .add_systems(
            Update,
            (
                operate_helm_ai
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel),
                process_helm_inputs
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(operate_helm_ai),
                publish_joystick_to_engines
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(process_helm_inputs),
                operate_helm_engine_ai
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(operate_helm_ai),
                detect_reached_objective_completion.in_set(crate::sim_sets::SimSet::Broadcast),
                tick_impulse.in_set(crate::sim_sets::SimSet::Physics),
                tick_boost.in_set(crate::sim_sets::SimSet::Physics),
                sync_ship_position
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .before(crate::sim_sets::AiTickLabel),
                handle_impulse_messages.in_set(crate::sim_sets::SimSet::Input),
                handle_boost_messages.in_set(crate::sim_sets::SimSet::Input),
                handle_station_rating_change.in_set(crate::sim_sets::SimSet::Input),
                handle_coordination_enqueue.in_set(crate::sim_sets::SimSet::Input),
                handle_coordination_messages.in_set(crate::sim_sets::SimSet::Input),
                process_coordination_lag.in_set(crate::sim_sets::SimSet::Modifiers),
                sync_console_damage_tiers.in_set(crate::sim_sets::SimSet::Damage),
            )
                .after(crate::lobby::process_lobby),
        );
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Systems Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

fn process_helm_inputs(
    time: Res<Time>,
    mut timer: ResMut<HelmInputTimer>,
    ship_query: Query<(&AdmittedCommands, &ShipSystemControlSources), With<LocalShip>>,
    mut physics_query: Query<&mut ShipPhysics, With<LocalShip>>,
    mut last_input_q: Query<&mut LastHelmInput, With<LocalShip>>,
    modifiers_q: Query<&ShipModifiers, With<LocalShip>>,
    modifiers_res: Option<Res<ShipModifiers>>,
    drive: HelmDriveParams,
    mut prev_phase: Local<Option<crate::impulse::ImpulsePhase>>,
) {
    // Prefer per-entity ShipModifiers component on LocalShip; fall back to
    // the global Resource for tests that only insert the Resource form.
    let default_modifiers;
    let modifiers: &ShipModifiers = match modifiers_q.single() {
        Ok(m) => m,
        Err(_) => match modifiers_res.as_deref() {
            Some(m) => m,
            None => {
                default_modifiers = ShipModifiers::new();
                &default_modifiers
            }
        },
    };
    let Some(mut last_input) = last_input_q.iter_mut().next() else {
        return;
    };
    // Edge-detect Idle → Charging (or any → Charging) and zero out the
    // last cached helm input so a stale steering/thrust value can't
    // resurface the moment impulse cancels or the autopilot disengages.
    let current_phase = drive.impulse_q.iter().next().map(|i| i.0.phase)
        .unwrap_or(crate::impulse::ImpulsePhase::Idle);
    if Some(current_phase) != *prev_phase {
        if current_phase == crate::impulse::ImpulsePhase::Charging {
            last_input.thrust = 0.0;
            last_input.steering = 0.0;
        }
        *prev_phase = Some(current_phase);
    }

    let Some((admitted, sources)) = ship_query.iter().next() else {
        return;
    };
    let Some(mut physics) = physics_query.iter_mut().next() else {
        return;
    };

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    // When helm is under AI control (Backfill), `operate_helm_ai` has already
    // integrated physics this frame.  Running `compute_physics` here a second
    // time with a *different* dt (the 30 Hz timer duration) would double-move
    // the player ship relative to NPC ships driven only by `operate_helm_ai`.
    // Skip the physics integration; the AI-written LastHelmInput is still
    // preserved so the SimState broadcast reflects the correct intent.
    if helm_control_policy(sources).operate_ai {
        return;
    }

    // Read per-entity config components from the LocalShip entity.
    // Falls back to Resources when the component is absent (test environments
    // that don't insert a LocalShip entity with drive config components).
    let impulse_cfg = drive.impulse_cfg();
    let boost_cfg = drive.boost_cfg();
    let bank_cfg = drive.bank_cfg();
    let physics_cfg = drive.physics_cfg();

    for cmd in admitted.for_target(crate::system_registry::HELM_SYSTEM_ID) {
        if let SystemControlPayload::HelmInput { thrust, steering } = &cmd.payload {
            last_input.thrust = *thrust;
            last_input.steering = *steering;
        }
    }
    let dt = timer.0.duration().as_secs_f32();
    let state = ShipPhysicsState {
        x: physics.x,
        z: physics.z,
        yaw: physics.yaw,
        forward_speed: physics.forward_speed,
    };
    let impulse_active = drive.impulse_q.iter().next().map(|i| i.0.is_active()).unwrap_or(false);
    let input = if impulse_active {
        // Autopilot: full forward thrust, zero steering. Player input is ignored.
        ShipPhysicsInput {
            thrust: 1.0,
            steering: 0.0,
        }
    } else {
        ShipPhysicsInput {
            thrust: last_input.thrust,
            steering: last_input.steering,
        }
    };

    // ── Engine-damage thrust scaling (issue #511) ─────────────────────────
    // Count how many fine engine systems are online. Each offline engine
    // removes 50% of the computed thrust. If both engines are offline, thrust
    // is zeroed. Uses `offline_systems` set on `ShipSystemControlSources`
    // (populated by `sync_console_damage_tiers` in `SimSet::Damage`).
    let port_offline = sources.0.offline_systems.contains(
        &crate::system_registry::helm_engine_port_system_id(),
    );
    let stbd_offline = sources.0.offline_systems.contains(
        &crate::system_registry::helm_engine_starboard_system_id(),
    );
    // Fraction of engines online: 0 engines = 0.0, 1 engine = 0.5, 2 = 1.0.
    // Only scale when at least one fine engine system is known (i.e. both IDs
    // are registered in the control sources; if neither ID is present in
    // offline_systems at all, it means no fine systems exist → no scaling).
    let engine_thrust_scale: f32 = match (port_offline, stbd_offline) {
        (true, true) => 0.0,
        (true, false) | (false, true) => 0.5,
        (false, false) => 1.0,
    };
    let scaled_input = ShipPhysicsInput {
        thrust: input.thrust * engine_thrust_scale,
        steering: input.steering,
    };

    let mut config = match physics_cfg {
        Some(ref cfg) => cfg.0,
        None => ShipPhysicsConfig::new(),
    };
    config.max_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
    config.max_reverse_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
    config.max_yaw_rate *= modifiers.get(&ModifierSlot::MaxYawRate);
    if impulse_active {
        // Mirror `ship/impulse.rs::apply_to_physics`: a non-positive
        // multiplier (e.g. an unset TOML field defaulting to 0) falls
        // back to the const instead of nuking acceleration entirely.
        let mult = if impulse_cfg.acceleration_multiplier > 0.0 {
            impulse_cfg.acceleration_multiplier
        } else {
            crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER
        };
        config.acceleration *= mult;
    }
    // Boost drive: while engaged, multiply max speed and acceleration. Only
    // applies when the ship's TOML enabled the feature.
    if boost_cfg.enabled && drive.boost.0.is_active() {
        config.max_speed *= boost_cfg.multiplier;
        config.max_reverse_speed *= boost_cfg.multiplier;
        config.acceleration *= boost_cfg.multiplier;
        config.max_yaw_rate *= boost_cfg.steering_multiplier;
    }
    let result = compute_physics(state, scaled_input, dt, &config);

    physics.x = result.x;
    physics.z = result.z;
    physics.yaw = result.yaw;
    physics.forward_speed = result.forward_speed;

    // Visual banking: lerp roll toward target based on steering (use the unscaled
    // input.steering so roll reflects intent, not engine count).
    let max_bank_rad = bank_cfg.max_bank_deg.to_radians();
    let target_roll = if impulse_active {
        0.0
    } else {
        -input.steering * max_bank_rad
    };
    let lerp_factor = (bank_cfg.bank_lerp_rate * dt).min(1.0);
    physics.roll = physics.roll + (target_roll - physics.roll) * lerp_factor;
}

fn helm_control_policy(sources: &ShipSystemControlSources) -> ControlTickPolicy {
    sources
        .0
        .policy_for(&crate::system_registry::helm_system_id())
}

/// Unified per-entity helm AI. Runs after the AI tick so `last_helm_intent`
/// is fresh for NPC ships.
///
/// For every ship entity where the helm system is `ControlSource::Ai`:
///  - Reads `ShipSystemBlackboards` viewscreen entry for scored objectives.
///  - Builds a `WorldView` from `WorldSnapshot` (all other entities for avoidance),
///    falling back to a direct ECS query when `WorldSnapshot` is absent (tests).
///  - Calls `operate_helm(memory, ...)` and writes the result to `ShipPhysics`.
///  - For `LocalShip` entities: also writes to `LastHelmInput` so
///    `process_helm_inputs` applies the correct physics this tick.
///  - For NPC ships with `AiControllerComponent`: also updates `last_helm_intent`
///    for backward compatibility until `tick_ai_controllers` is retired (#595).
///
/// This replaces both the old player-ship-only `player_ship_helm_ai` and the NPC
/// helm path in `tick_ai_controllers`.
#[allow(clippy::too_many_arguments)]
fn operate_helm_ai(
    time: Res<Time>,
    mut local_ship_input: Query<&mut LastHelmInput, With<LocalShip>>,
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    entity_fallback_q: Query<(
        &crate::entity_spawner::EntityUuid,
        &Transform,
        Option<&crate::entities::spawner::EntityName>,
        Option<&crate::entities::spawner::FactionComponent>,
        Option<&crate::entities::spawner::EntityConsoleHull>,
        Option<&crate::entities::spawner::ColliderSection>,
    )>,
    mut ships: Query<(
        Entity,
        &ShipSystemControlSources,
        &mut ShipPhysics,
        &mut crate::ai_plugin::ShipAiMemory,
        &crate::server_app::ShipSystemBlackboards,
        Option<&crate::entity_spawner::EntityUuid>,
        Option<&crate::entities::spawner::FactionComponent>,
        Option<&crate::entities::spawner::ColliderSection>,
        Option<&crate::entities::spawner::HelmConsoleSection>,
        Option<&crate::entities::spawner::BehaviourSection>,
        Has<crate::server_app::LocalShip>,
    )>,
) {
    let dt = time.delta_secs();
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();
    let runtime_ref = runtime.as_deref();

    // Snapshot world entities for avoidance (read-only pass before mutating ships).
    // Use WorldSnapshot when available (production); fall back to inline query (tests).
    let snapshot_entities: Vec<crate::ai::AiWorldEntity> =
        if let Some(ws) = world_snapshot.as_ref() {
            ws.entities.clone()
        } else {
            // Fallback path for tests that don't register AiPlugin.
            entity_fallback_q
                .iter()
                .map(|(uuid, transform, name, faction, hull, collider)| {
                    let runtime_name = runtime_ref.and_then(|rt| {
                        rt.name_to_uuid.iter().find_map(|(n, mapped)| {
                            (mapped == &uuid.0).then(|| n.clone())
                        })
                    });
                    let hull_fraction = hull.and_then(|h| {
                        let max = h.0.total_max();
                        (max > 0.0).then(|| h.0.total_current() / max)
                    });
                    crate::ai::AiWorldEntity {
                        uuid: uuid::Uuid::parse_str(&uuid.0).unwrap_or_default(),
                        name: runtime_name.or_else(|| name.map(|n| n.0.clone())),
                        position: [
                            transform.translation.x,
                            transform.translation.y,
                            transform.translation.z,
                        ],
                        faction: faction.map(|f| f.0),
                        hull_fraction,
                        yaw: Some(transform.rotation.to_euler(EulerRot::YXZ).0),
                        radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
                        ..Default::default()
                    }
                })
                .collect()
        };

    for (
        _entity,
        sources,
        mut physics,
        mut ai_memory,
        blackboards,
        entity_uuid,
        faction,
        collider,
        helm_section,
        behaviour_section,
        is_local,
    ) in ships.iter_mut()
    {
        let policy = helm_control_policy(sources);
        if !policy.operate_ai {
            continue;
        }

        // ── Read scored objectives from this entity's viewscreen blackboard ──
        let scored: Vec<crate::messages::ScoredObjective> = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
            _ => vec![],
        };

        let has_helm_objective = scored
            .iter()
            .any(|o| o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm));

        if !has_helm_objective {
            // No objectives → zero out intent (decelerate to stop).
            if is_local {
                if let Some(mut li) = local_ship_input.iter_mut().next() {
                    *li = LastHelmInput::default();
                }
            }
            continue;
        }

        // ── Build WorldView (exclude self) ───────────────────────────────────
        let self_uuid_str = entity_uuid.map(|u| u.0.as_str()).unwrap_or("");
        let entities: Vec<crate::ai::AiWorldEntity> = snapshot_entities
            .iter()
            .filter(|e| e.uuid.to_string() != self_uuid_str)
            .cloned()
            .collect();

        let world_view = crate::ai::WorldView {
            entity_pos: [physics.x, 0.0, physics.z],
            entity_yaw: physics.yaw,
            anchors: anchors.clone(),
            entities,
            self_faction: faction.map(|f| f.0),
            self_radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
            ..crate::ai::WorldView::default()
        };

        let (thrust, steering) = crate::ai::operate_helm(
            &mut ai_memory.0,
            &world_view,
            &scored,
            behaviour_section
                .map(|b| b.0.doctrine.as_slice())
                .unwrap_or(&[]),
            &anchors,
            crate::ai::WAYPOINT_ARRIVAL_RADIUS,
            crate::ai::AVOIDANCE_BUFFER,
            crate::ai::AVOIDANCE_LOOK_AHEAD_SECS,
            physics.forward_speed,
            faction_registry
                .as_deref()
                .map(|r| &r.0)
                .unwrap_or(&crate::faction::FactionRegistry::default()),
        );

        // ── Apply physics ────────────────────────────────────────────────────
        let physics_config = helm_section
            .map(|hc| ShipPhysicsConfig {
                max_speed: hc.0.max_speed,
                max_reverse_speed: hc.0.max_reverse_speed,
                acceleration: hc.0.acceleration,
                deceleration: hc.0.deceleration,
                max_yaw_rate: hc.0.max_yaw_rate,
            })
            .unwrap_or_else(ShipPhysicsConfig::new);

        let result = compute_physics(
            ShipPhysicsState {
                x: physics.x,
                z: physics.z,
                yaw: physics.yaw,
                forward_speed: physics.forward_speed,
            },
            ShipPhysicsInput { thrust, steering },
            dt,
            &physics_config,
        );

        physics.x = result.x;
        physics.z = result.z;
        physics.yaw = result.yaw;
        physics.forward_speed = result.forward_speed;

        // For the player ship: also write LastHelmInput so process_helm_inputs
        // sees the AI-driven intent (though it will re-apply physics anyway).
        if is_local {
            if let Some(mut li) = local_ship_input.iter_mut().next() {
                *li = LastHelmInput { thrust, steering };
            }
        }
    }
}

/// Mark Reach objectives complete once any ship arrives within
/// `WAYPOINT_ARRIVAL_RADIUS` of the objective's anchor.
///
/// Runs in `Broadcast` (after `PublishAggregate` so `scored_objectives` is
/// fresh) and only counts ships whose helm system is AI-controlled.
/// Iterates every ship (player + NPC) so any ship pursuing a shared
/// world Reach objective can complete it. The `ObjectiveManagerRes` is a
/// single world-level resource, so multiple ships arriving at the same
/// anchor complete the shared objective once (idempotent complete()).
fn detect_reached_objective_completion(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    objectives: Option<ResMut<crate::world::server::ObjectiveManagerRes>>,
    ships: Query<
        (&ShipSystemControlSources, &ShipPhysics, &crate::server_app::ShipSystemBlackboards),
        With<crate::server_app::Ship>,
    >,
) {
    let Some(mut objectives) = objectives else {
        return;
    };
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();

    for (sources, physics, blackboards) in ships.iter() {
        if !helm_control_policy(sources).operate_ai {
            continue;
        }

        let scored: Vec<crate::messages::ScoredObjective> = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => {
                bb.scored_objectives.clone()
            }
            _ => continue,
        };

        for obj in &scored {
            if obj.score <= 0.0 {
                continue;
            }
            let crate::messages::AiDirective::Reach { anchor } = &obj.directive else {
                continue;
            };
            let Some(&target) = anchors.get(anchor.as_str()) else {
                continue;
            };
            let dx = target[0] - physics.x;
            let dz = target[2] - physics.z;
            if (dx * dx + dz * dz).sqrt() < crate::ai::WAYPOINT_ARRIVAL_RADIUS {
                objectives.0.complete(&obj.snapshot.id);
            }
        }
    }
}

// ── Fine-grained Helm systems: channel-1 joystick → engines (issue #511) ──────

/// Forwards the current joystick state from the Helm Joystick fine system to
/// both Helm Engine fine systems via the `InterSystemQueue` (channel 1).
///
/// Runs in `SimSet::Physics` AFTER `process_helm_inputs` so `LastHelmInput`
/// has been populated from admitted commands this tick. Both engine instances
/// receive the same joystick payload; each engine independently gates on its
/// own online state when interpreting the message.
fn publish_joystick_to_engines(
    ships: Query<(&ShipSystemControlSources, &LastHelmInput), With<LocalShip>>,
    mut inter_system: ResMut<InterSystemQueue>,
) {
    for (sources, last_input) in ships.iter() {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::helm_joystick_system_id());
        // Only publish when the joystick system can operate (human or AI).
        if !policy.accept_human_input && !policy.operate_ai {
            continue;
        }
        let port_id = crate::system_registry::helm_engine_port_system_id();
        let stbd_id = crate::system_registry::helm_engine_starboard_system_id();
        for target in [port_id, stbd_id] {
            inter_system.0.push(InterSystemMsg {
                target,
                payload: InterSystemPayload::JoystickState {
                    thrust: last_input.thrust,
                    steering: last_input.steering,
                },
                source_entity: None,
            });
        }
    }
}

/// Per-engine AI bookkeeping (issue #511). Mirrors what the joystick publishes
/// but for ships where an engine is under AI control.
///
/// The coarse `operate_helm_ai` already drives physics; this system only
/// ensures the fine engine systems reflect AI-controlled thrust in the
/// blackboard so the GUI can show AUTO badges correctly.
fn operate_helm_engine_ai(
    ships: Query<(&ShipSystemControlSources, &LastHelmInput), With<LocalShip>>,
    mut inter_system: ResMut<InterSystemQueue>,
) {
    for (sources, last_input) in ships.iter() {
        // `publish_joystick_to_engines` already covers the normal case where
        // the joystick system is operable (human or AI). This system only
        // needs to push engine messages when the joystick itself is offline
        // (e.g. joystick damaged/disabled) but an individual engine is still
        // AI-controlled. This prevents a double-push on every Backfill tick.
        let joystick_policy = sources
            .0
            .policy_for(&crate::system_registry::helm_joystick_system_id());
        let joystick_publishing = joystick_policy.accept_human_input || joystick_policy.operate_ai;
        if joystick_publishing {
            // `publish_joystick_to_engines` will cover both engines this tick.
            continue;
        }

        let port_policy = sources
            .0
            .policy_for(&crate::system_registry::helm_engine_port_system_id());
        let stbd_policy = sources
            .0
            .policy_for(&crate::system_registry::helm_engine_starboard_system_id());

        // Joystick is offline; push for any engine that is still AI-operable.
        if port_policy.operate_ai {
            inter_system.0.push(InterSystemMsg {
                target: crate::system_registry::helm_engine_port_system_id(),
                payload: InterSystemPayload::JoystickState {
                    thrust: last_input.thrust,
                    steering: last_input.steering,
                },
                source_entity: None,
            });
        }
        if stbd_policy.operate_ai {
            inter_system.0.push(InterSystemMsg {
                target: crate::system_registry::helm_engine_starboard_system_id(),
                payload: InterSystemPayload::JoystickState {
                    thrust: last_input.thrust,
                    steering: last_input.steering,
                },
                source_entity: None,
            });
        }
    }
}

fn sync_ship_position(
    mut ship_query: Query<(&ShipPhysics, &mut Transform)>,
) {
    for (physics, mut transform) in ship_query.iter_mut() {
        transform.translation.x = physics.x;
        transform.translation.z = physics.z;
        transform.rotation = Quat::from_euler(EulerRot::YXZ, physics.yaw, 0.0, physics.roll);
    }
}

pub fn handle_impulse_messages(
    ship_ac_query: Query<&AdmittedCommands, With<LocalShip>>,
    mut impulse_q: Query<&mut ShipImpulse, With<LocalShip>>,
    hull_q: Query<&crate::entity_spawner::EntityConsoleHull, With<LocalShip>>,
    mut last_hull_hp: Local<f32>,
    membership: Option<Res<RegionMembership>>,
    region_query: Query<&RegionEffectsSection>,
    ship_query: Query<Entity, With<LocalShip>>,
) {
    let Some(admitted) = ship_ac_query.iter().next() else {
        return;
    };
    let Some(mut impulse) = impulse_q.iter_mut().next() else {
        return;
    };
    let hull_total = hull_q.single().map(|h| (h.0.total_current(), h.0.total_max())).unwrap_or((100.0, 100.0));
    if *last_hull_hp == 0.0 && (hull_total.0 - hull_total.1).abs() < 1e-6 {
        *last_hull_hp = hull_total.1;
    }

    let current_hp = hull_total.0;
    if current_hp < *last_hull_hp {
        impulse.0.cancel_charge();
    }
    *last_hull_hp = current_hp;

    for cmd in admitted.for_target(crate::system_registry::HELM_SYSTEM_ID) {
        match &cmd.payload {
            SystemControlPayload::StartImpulseCharge
                if !is_inside_blocks_impulse(&membership, &region_query, &ship_query) =>
            {
                impulse.0.start_charge();
            }
            SystemControlPayload::CancelImpulse => {
                impulse.0.cancel_charge();
            }
            _ => {}
        }
    }
}

fn tick_impulse(
    time: Res<Time>,
    mut ships_q: Query<(&mut ShipImpulse, Option<&ImpulseConfigResource>), With<Ship>>,
    config_res: Option<Res<ImpulseConfigResource>>,
) {
    let dt = time.delta_secs();
    let fallback_duration = config_res
        .as_deref()
        .map(|c| c.charge_duration)
        .unwrap_or(crate::impulse::IMPULSE_CHARGE_DURATION);
    for (mut impulse, entity_cfg) in ships_q.iter_mut() {
        let charge_duration = entity_cfg
            .map(|c| c.charge_duration)
            .unwrap_or(fallback_duration);
        impulse.0.tick(dt, charge_duration);
    }
}

/// Toggle the boost drive in response to Helm boost controls. No-op when
/// the feature is disabled.
pub fn handle_boost_messages(
    mut ship_query: Query<
        (&AdmittedCommands, Option<&BoostConfigResource>, Option<&mut ShipBoost>),
        With<LocalShip>,
    >,
    mut boost_res: ResMut<ShipBoost>,
    config_res: Option<Res<BoostConfigResource>>,
) {
    let Some((admitted, entity_cfg, entity_boost)) = ship_query.iter_mut().next() else {
        return;
    };
    // Per-entity component takes priority over the Resource fallback.
    let enabled = entity_cfg
        .map(|c| c.enabled)
        .or_else(|| config_res.as_deref().map(|c| c.enabled))
        .unwrap_or(false);
    if !enabled {
        return;
    }
    // Determine which boost state to mutate: entity component when present
    // (production path), resource as fallback (legacy test path).
    let has_entity_boost = entity_boost.is_some();
    let mut entity_boost = entity_boost;
    for cmd in admitted.for_target(crate::system_registry::HELM_SYSTEM_ID) {
        match &cmd.payload {
            SystemControlPayload::ToggleBoost => {
                if let Some(ref mut b) = entity_boost {
                    b.0.toggle();
                }
                if !has_entity_boost {
                    boost_res.0.toggle();
                } else {
                    // Keep resource in sync so process_helm_inputs (Res<ShipBoost>) sees it.
                    if let Some(ref b) = entity_boost {
                        boost_res.0 = b.0.clone();
                    }
                }
            }
            SystemControlPayload::SetBoost { active } => {
                if let Some(ref mut b) = entity_boost {
                    if *active {
                        b.0.activate();
                    } else {
                        b.0.deactivate();
                    }
                }
                if !has_entity_boost {
                    if *active {
                        boost_res.0.activate();
                    } else {
                        boost_res.0.deactivate();
                    }
                } else if let Some(ref b) = entity_boost {
                    boost_res.0 = b.0.clone();
                }
            }
            _ => {}
        }
    }
}

fn normalized_boost_drain_factor(thrust: f32, steering: f32) -> f32 {
    thrust.clamp(-1.0, 1.0).abs() + steering.clamp(-1.0, 1.0).abs()
}

fn tick_boost(
    time: Res<Time>,
    mut boost_res: ResMut<ShipBoost>,
    // Per-entity component takes priority; Resource is the fallback.
    mut boost_entity_q: Query<(Option<&BoostConfigResource>, Option<&mut ShipBoost>), With<LocalShip>>,
    boost_cfg_res: Option<Res<BoostConfigResource>>,
    last_input_q: Query<&LastHelmInput, With<LocalShip>>,
    sessions: Res<Sessions>,
    impulse_q: Query<&ShipImpulse, With<LocalShip>>,
    ship_components: Query<(&ShipConfigComponent, &ShipSystemControlSources), With<LocalShip>>,
) {
    let Some((ship_config, control_sources)) = ship_components.iter().next() else {
        return;
    };
    // Per-entity component takes priority over the Resource fallback.
    let Some((entity_cfg_opt, entity_boost_opt)) = boost_entity_q.iter_mut().next() else {
        return;
    };
    let entity_cfg = entity_cfg_opt.cloned();
    let config = entity_cfg
        .or_else(|| boost_cfg_res.as_deref().cloned())
        .unwrap_or_default();
    if !config.enabled {
        return;
    }
    let last_input = last_input_q.single().copied().unwrap_or_default();
    let policy = helm_control_policy(&control_sources);
    let has_helm = sessions
        .0
        .console_holder(&crate::messages::Console::Helm, &ship_config.0)
        .is_some()
        || policy.operate_ai;
    let impulse_active = impulse_q.iter().next().map(|i| i.0.is_active()).unwrap_or(false);
    let drain_factor = if !has_helm {
        0.0
    } else if impulse_active {
        normalized_boost_drain_factor(1.0, 0.0)
    } else {
        normalized_boost_drain_factor(last_input.thrust, last_input.steering)
    };
    if let Some(mut entity_boost) = entity_boost_opt {
        // Production path: entity component takes priority.
        entity_boost.0.tick_with_drain_factor(
            time.delta_secs(),
            config.active_duration,
            config.recharge_duration,
            drain_factor,
        );
        // Keep resource in sync so process_helm_inputs (Res<ShipBoost>) sees it.
        boost_res.0 = entity_boost.0.clone();
    } else {
        // Fallback: legacy test path with no entity component.
        boost_res.0.tick_with_drain_factor(
            time.delta_secs(),
            config.active_duration,
            config.recharge_duration,
            drain_factor,
        );
    }
}

fn is_inside_blocks_impulse(
    membership: &Option<Res<RegionMembership>>,
    region_query: &Query<&RegionEffectsSection>,
    ship_query: &Query<Entity, With<LocalShip>>,
) -> bool {
    let Some(membership) = membership else {
        return false;
    };
    let Some(ship_entity) = ship_query.iter().next() else {
        return false;
    };
    let Some(inside) = membership.inside.get(&ship_entity) else {
        return false;
    };
    for &region_entity in inside {
        if let Ok(effects) = region_query.get(region_entity) {
            if effects.0.contains(&RegionEffectKind::BlocksImpulse) {
                return true;
            }
        }
    }
    false
}

// ── Station Rating Handler ──────────────────────────────────────────────────

/// System that processes `SetStationRating` messages from players mid-game.
/// Resolves the sender's station from their held consoles, looks up the
/// rating in the ship config, and updates `ShipSystemControlSources` and
/// `ActiveStationRatings` accordingly.
pub fn handle_station_rating_change(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut ship_components: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<LocalShip>,
    >,
    mut outbox: ResMut<crate::lobby::LobbyOutbox>,
) {
    let messages: Vec<_> = reader.read().collect();
    for (ship_config, mut control_sources, mut active_ratings) in ship_components.iter_mut() {
        for ev in messages.iter() {
            let ClientMessage::SetStationRating { rating_name } = &ev.msg else {
                continue;
            };

            let station_id = sessions.0.station_for_token(&ev.token).cloned();
            let Some(station_id) = station_id else {
                continue;
            };

            rating::apply_rating(
                &ship_config.0,
                &station_id,
                rating_name,
                &mut control_sources.0,
            );

            active_ratings
                .0
                .insert(station_id.clone(), rating_name.clone());

            outbox.0.push((
                crate::lobby_handler::Target::All,
                crate::messages::ServerMessage::RatingChanged {
                    station_id,
                    rating_name: rating_name.clone(),
                },
            ));
        }
    }
}

pub fn handle_coordination_enqueue(
    mut ship_components: Query<
        (Entity, &ShipConfigComponent, &mut CoordinationQueue),
        With<crate::server_app::Ship>,
    >,
    local_ship_q: Query<Entity, With<LocalShip>>,
    mut events: MessageReader<CoordinationEnqueue>,
    mut inbound: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    let coord_events: Vec<_> = events.read().cloned().collect();
    let inbound_msgs: Vec<_> = inbound.read().cloned().collect();

    // Route typed CoordinationEnqueue events to their source ship's queue.
    for ev in &coord_events {
        let Ok((_e, ship_config, mut queue)) = ship_components.get_mut(ev.source_entity) else {
            // Source ship despawned or lacks a CoordinationQueue — silently drop.
            continue;
        };
        let lag = ship_config.0.coordination_lag_secs;
        queue.0.enqueue(QueuedCoordination {
            sender_origin: ev.sender_origin,
            target: ev.target.clone(),
            payload: ev.payload.clone(),
            sender_label: ev.sender_label.clone(),
            due_time: now + lag,
        });
    }

    // Route human `SendCoordination` messages to the LocalShip only.
    // `SendCoordination` is always a ClientMessage from a human, always
    // scoped to that human's own ship.
    let Some(local_entity) = local_ship_q.iter().next() else {
        return;
    };
    let Ok((_e, ship_config, mut queue)) = ship_components.get_mut(local_entity) else {
        return;
    };
    let lag = ship_config.0.coordination_lag_secs;
    for msg in &inbound_msgs {
        let ClientMessage::SendCoordination { target, payload } = &msg.msg else {
            continue;
        };
        let player = match sessions.0.players().iter().find(|p| p.token == msg.token) {
            Some(p) => p,
            None => continue,
        };
        let sender_origin = if player.station.is_none() {
            ControlSource::Ai
        } else {
            ControlSource::Human
        };
        queue.0.enqueue(QueuedCoordination {
            sender_origin,
            target: target.clone(),
            payload: payload.clone(),
            sender_label: player.name.clone(),
            due_time: now + lag,
        });
    }
}

pub fn process_coordination_lag(
    time: Res<Time>,
    mut ship_components: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &mut CoordinationQueue,
            Has<LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    sessions: Res<Sessions>,
    mut outbox: ResMut<crate::lobby::LobbyOutbox>,
) {
    let now = time.elapsed_secs();
    for (ship_config, control_sources, mut queue, is_local) in ship_components.iter_mut() {
        let due = queue.0.due_messages(now);
        for msg in due {
            let target_control = control_sources.0.source_for(&msg.target);
            let action = coordination::route_coordination(msg.sender_origin, target_control);

            match action {
                coordination::DeliverAction::Consume => {}
                coordination::DeliverAction::Suppress => {}
                coordination::DeliverAction::Popup => {
                    // Popups require a browser-connected console holder.
                    // Only the LocalShip has one — NPCs drain silently.
                    if !is_local {
                        continue;
                    }
                    let label = if msg.sender_label.is_empty() {
                        "AI".to_string()
                    } else {
                        msg.sender_label
                    };

                    let system = ship_config.0.system(&msg.target);
                    let station_opt = system.and_then(|s| s.station.as_ref());

                    if let Some(station_id) = station_opt {
                        if let Some(station) = ship_config.0.station(station_id) {
                            let console_id = &station.console;
                            let token: Option<String> = crate::messages::Console::from_console_id(
                                console_id,
                            )
                            .and_then(|console| {
                                sessions
                                    .0
                                    .console_holder(&console, &ship_config.0)
                                    .map(|t| t.to_string())
                            });

                            if let Some(token) = token {
                                outbox.0.push((
                                    crate::lobby_handler::Target::Token(token),
                                    crate::messages::ServerMessage::CoordinationPopup {
                                        target: msg.target.clone(),
                                        payload: msg.payload.clone(),
                                        sender_label: label,
                                    },
                                ));
                            }
                        }
                    } else {
                        outbox.0.push((
                            crate::lobby_handler::Target::All,
                            crate::messages::ServerMessage::CoordinationPopup {
                                target: msg.target.clone(),
                                payload: msg.payload.clone(),
                                sender_label: label,
                            },
                        ));
                    }
                }
            }
        }
    }
}

pub fn handle_coordination_messages(mut reader: MessageReader<InboundMessage>) {
    for msg in reader.read() {
        let ClientMessage::SendCoordination { .. } = &msg.msg else {
            continue;
        };
    }
}

// ── Damage-tier → control gate sync ──────────────────────────────────────────

/// Bevy system that synchronises `ControlSourceResolver.offline_systems` with
/// the current damage tiers of each console in the ship hull.
///
/// Runs in `SimSet::Damage` (after hull damage is applied). For every ship that
/// carries both `EntityConsoleHull` and `ShipSystemControlSources`:
///
/// - Consoles in `Disabled` or `Destroyed` tier: their corresponding `SystemId`
///   is added to `offline_systems`.
/// - Consoles in `Operational` or `Damaged` tier: their corresponding
///   `SystemId` is removed from `offline_systems` (restoring normal gating).
///
/// The `SystemId` for a console is derived from `Console::station_console_id()`,
/// which matches the `id` field of the `[[system]]` entries in the TOML.
pub fn sync_console_damage_tiers(
    mut ships: Query<(
        &crate::entity_spawner::EntityConsoleHull,
        &mut ShipSystemControlSources,
    )>,
) {
    for (hull_component, mut control_sources) in ships.iter_mut() {
        let hull = &hull_component.0;
        for (console, _cur, _max) in hull.entries() {
            let system_id = crate::messages::SystemId(
                console.station_console_id().to_string(),
            );
            let tier = hull.tier_for(console.clone());
            match tier {
                DamageTier::Disabled | DamageTier::Destroyed => {
                    control_sources.0.offline_systems.insert(system_id);
                }
                DamageTier::Operational | DamageTier::Damaged => {
                    control_sources.0.offline_systems.remove(&system_id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_source::ControlSource;
    use crate::entity_config::EntityConfig;
    use crate::entity_spawner::spawn_entity;
    use crate::impulse::{ImpulsePhase, IMPULSE_CHARGE_DURATION};
    use crate::lobby::LobbyPlugin;
    use crate::messages::ClientMessage;
    use crate::messages::StationId;
    use crate::modifiers::ShipModifiers;
    use crate::region_effects::{BlocksImpulseEffect, RegionEffectsConfig};
    use crate::region_shape::RegionShape;
    use crate::regions::server::RegionPlugin;
    use crate::ship::rating;
    use crate::simulation::{LocalShip, Ship};

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            // TODO: remove, ShipImpulse is a Component (not a Resource) since #b4b0605;
            // kept here only to avoid breaking any remaining Resource-based readers in tests.
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(ShipModifiers::new())
            .add_plugins(ShipPlugin);
        let hull_config = &[
            (crate::messages::Console::Helm, 25.0_f32),
            (crate::messages::Console::Tactical, 25.0),
            (crate::messages::Console::Power, 25.0),
            (crate::messages::Console::Shields, 25.0),
        ];
        app.world_mut().spawn((
            Ship,
            LocalShip,
            Transform::default(),
            ShipPhysics::default(),
            ShipConfigComponent::default(),
            ShipSystemControlSources::default(),
            ActiveStationRatings::default(),
            CoordinationQueue::default(),
            crate::messages::AdmittedCommands::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ai_plugin::ShipAiMemory::default(),
            crate::entity_spawner::EntityConsoleHull(
                crate::damage::ConsoleHull::from_config(hull_config),
            ),
            LastHelmInput::default(),
            crate::simulation::ShipShields(crate::shield::ShieldSystem::default()),
            ShipImpulse(crate::impulse::ImpulseState::new()),
        ));
        app
    }

    fn apply_hull_damage(app: &mut App, amount: f32) {
        let mut rng = rand::rng();
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<crate::entity_spawner::EntityConsoleHull>()
            .unwrap()
            .0
            .apply_damage(amount, &mut rng);
    }

    fn get_last_helm_input(app: &mut App) -> LastHelmInput {
        app.world_mut()
            .query_filtered::<&LastHelmInput, With<LocalShip>>()
            .single(app.world())
            .copied()
            .unwrap_or_default()
    }

    fn set_last_helm_input(app: &mut App, val: LastHelmInput) {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut().entity_mut(ship).insert(val);
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) {
        app.update();
    }

    fn tick_twice(app: &mut App) {
        tick(app);
        tick(app);
    }

    fn start_game_with_helm_and_science(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Hikaru".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "helm", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn set_helm_control_source(app: &mut App, source: ControlSource) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(crate::system_registry::helm_system_id(), source);
        }
    }

    fn get_ship_physics(app: &mut App) -> ShipPhysics {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipPhysics, With<Ship>>();
        q.single(app.world())
            .expect("expected Ship entity with ShipPhysics")
            .clone()
    }

    fn set_ship_physics(app: &mut App, physics: ShipPhysics) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipPhysics, With<Ship>>();
        let mut p = q.single_mut(app.world_mut()).expect("expected Ship with ShipPhysics");
        *p = physics;
    }

    fn get_ship_control_sources(app: &mut App) -> ShipSystemControlSources {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemControlSources, With<Ship>>();
        q.single(app.world())
            .expect("expected Ship entity with ShipSystemControlSources")
            .clone()
    }

    fn get_ship_active_ratings(app: &mut App) -> ActiveStationRatings {
        let mut q = app
            .world_mut()
            .query_filtered::<&ActiveStationRatings, With<Ship>>();
        q.single(app.world())
            .expect("expected Ship entity with ActiveStationRatings")
            .clone()
    }

    // ── Helm system control-source tests ───────────────────────────────────

    fn get_ship_impulse(app: &mut App) -> crate::impulse::ImpulseState {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipImpulse, With<LocalShip>>();
        q.single(app.world())
            .expect("expected LocalShip entity with ShipImpulse")
            .0
            .clone()
    }

    fn set_ship_impulse(app: &mut App, state: crate::impulse::ImpulseState) {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ShipImpulse>()
            .unwrap()
            .0 = state;
    }

    #[test]
    fn control_system_helm_input_updates_last_input_and_moves_ship() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 1.0,
                    steering: 0.25,
                },
            },
        );
        tick_twice(&mut app);

        assert_eq!(
            get_last_helm_input(&mut app),
            LastHelmInput {
                thrust: 1.0,
                steering: 0.25
            }
        );
        assert!(get_ship_physics(&mut app).forward_speed > 0.0);
    }

    #[test]
    fn ai_helm_operates_without_human_holder() {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick_twice(&mut app);

        assert_eq!(
            get_last_helm_input(&mut app),
            LastHelmInput {
                thrust: 0.0,
                steering: 0.0
            }
        );
        assert_eq!(get_ship_physics(&mut app).forward_speed, 0.0);
    }

    #[test]
    fn ai_helm_ignores_human_input() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);
        set_helm_control_source(&mut app, ControlSource::Ai);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: -1.0,
                    steering: 1.0,
                },
            },
        );
        tick_twice(&mut app);

        // Human input must be ignored when policy is AI; no AiControllerComponent
        // on the player ship yet, so LastHelmInput stays at default.
        assert_eq!(
            get_last_helm_input(&mut app),
            LastHelmInput::default()
        );
    }

    #[test]
    fn human_helm_suppresses_ai_operate() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        tick(&mut app);

        assert_eq!(
            get_last_helm_input(&mut app),
            LastHelmInput::default()
        );
        assert_eq!(get_ship_physics(&mut app).forward_speed, 0.0);
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ Impulse Drive / Damage Cancellation tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn hull_damage_cancels_charging_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
            "impulse should be charging after StartImpulseCharge"
        );

        apply_hull_damage(&mut app, 10.0);
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
            "impulse charge should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn hull_damage_cancels_active_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);
        // One tick to let handle_impulse_messages initialise last_hull_hp from the
        // current (undamaged) hull, so a subsequent damage event is detected.
        tick(&mut app);

        {
            let active = {
                let mut s = crate::impulse::ImpulseState::new();
                s.start_charge();
                s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
                s
            };
            set_ship_impulse(&mut app, active);
        }
        assert!(
            get_ship_impulse(&mut app).is_active(),
            "impulse should be active before damage"
        );

        apply_hull_damage(&mut app, 10.0);
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
            "active impulse should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn no_hull_damage_does_not_cancel_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
            "impulse should still be charging when no damage occurred"
        );
    }

    #[test]
    fn start_impulse_charge_message_begins_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
        );
    }

    #[test]
    fn control_system_start_impulse_charge_begins_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
        );
    }

    #[test]
    fn cancel_impulse_message_cancels_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::CancelImpulse,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
        );
    }

    #[test]
    fn control_system_cancel_impulse_cancels_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::CancelImpulse,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
        );
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ BlocksImpulse region gating tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    fn blocks_impulse_test_app() -> App {
        let mut app = test_app();
        app.add_plugins(RegionPlugin);
        app
    }

    fn spawn_blocks_impulse_region(
        app: &mut App,
        x: f32,
        z: f32,
        radius: f32,
    ) -> bevy::ecs::entity::Entity {
        let config = EntityConfig {
            name: None,
            light: Vec::new(),
            ship_config: None,
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: Some(RegionEffectsConfig {
                blocks_impulse: Some(BlocksImpulseEffect {}),
                ..Default::default()
            }),
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            comms: None,
            sensors_console: None,
            navigation_console: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            star: None,
            target: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    #[test]
    fn start_impulse_charge_ignored_inside_blocks_impulse_region() {
        let mut app = blocks_impulse_test_app();

        let _region = spawn_blocks_impulse_region(&mut app, 0.0, 0.0, 50.0);

        start_game_with_helm_and_science(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
            "impulse should be idle before StartImpulseCharge"
        );

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
            "StartImpulseCharge should be ignored inside BlocksImpulse region"
        );
    }

    #[test]
    fn start_impulse_charge_works_outside_blocks_impulse_region() {
        let mut app = blocks_impulse_test_app();

        let _region = spawn_blocks_impulse_region(&mut app, 500.0, 0.0, 50.0);

        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
            "StartImpulseCharge should work when outside BlocksImpulse region"
        );
    }

    // ── Impulse autopilot tests ───────────────────────────────────────

    /// While the impulse drive is Active, the server should ignore any helm
    /// input from the player and autopilot the ship: full forward thrust,
    /// zero steering. The configured `acceleration_multiplier` boosts the
    /// base acceleration so the ship ramps up to the boosted top speed.
    #[test]
    fn active_impulse_autopilots_with_boosted_acceleration() {
        let mut app = test_app();
        // 5x boost: base accel = 25/3 ≈ 8.33; boosted = ~41.67 per second.
        // Timer fires at 30 Hz (dt ≈ 1/30 s), so the first tick gives
        // forward_speed ≈ 41.67/30 ≈ 1.39.
        app.insert_resource(ImpulseConfigResource {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: 5.0,
        });
        start_game_with_helm_and_science(&mut app);

        // Activate impulse directly (bypass charge).
        {
            let mut s = crate::impulse::ImpulseState::new();
            s.start_charge();
            s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
            set_ship_impulse(&mut app, s);
        }

        // Player tries to fight the autopilot: zero thrust, hard right steer.
        // The server must ignore both and force thrust=1.0, steering=0.0.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 0.0,
                    steering: 1.0,
                },
            },
        );
        tick(&mut app);

        let physics = get_ship_physics(&mut app);
        // With 5x boost, expect ≈1.39; without boost ≈0.28. Require >=1.0 to
        // clearly distinguish the boosted path.
        assert!(
            physics.forward_speed >= 1.0,
            "active impulse should autopilot with boosted accel; got forward_speed={}",
            physics.forward_speed
        );
        // Steering must be ignored — yaw should be essentially unchanged.
        assert!(
            physics.yaw.abs() < 1e-3,
            "active impulse must zero steering; got yaw={}",
            physics.yaw
        );
    }

    /// While the impulse drive is Idle, the configured
    /// `acceleration_multiplier` must have no effect — it applies only
    /// during the Active phase.
    #[test]
    fn idle_impulse_does_not_boost_acceleration() {
        let mut app = test_app();
        app.insert_resource(ImpulseConfigResource {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: 5.0,
        });
        start_game_with_helm_and_science(&mut app);

        // Impulse stays Idle; helm asks for full thrust.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 1.0,
                    steering: 0.0,
                },
            },
        );
        tick(&mut app);

        let physics = get_ship_physics(&mut app);
        // Base accel ≈ 8.33, dt = 1/30 → expected ≈ 0.28. Cap at 2.0 to
        // catch any accidental boost.
        assert!(
            physics.forward_speed < 2.0,
            "idle impulse must not boost accel; got forward_speed={}",
            physics.forward_speed
        );
    }

    /// A non-positive `acceleration_multiplier` (e.g. an unconfigured TOML
    /// field that defaults to 0.0) must fall back to the const
    /// `IMPULSE_ACCELERATION_MULTIPLIER` instead of nuking acceleration
    /// during impulse. Mirrors the `speed_multiplier <= 0` fallback in
    /// `ship/impulse.rs::apply_to_physics`.
    #[test]
    fn zero_acceleration_multiplier_falls_back_to_const() {
        let mut app = test_app();
        app.insert_resource(ImpulseConfigResource {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: 0.0,
        });
        start_game_with_helm_and_science(&mut app);

        // Activate impulse directly (bypass charge).
        {
            let mut s = crate::impulse::ImpulseState::new();
            s.start_charge();
            s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
            set_ship_impulse(&mut app, s);
        }
        tick(&mut app);

        let physics = get_ship_physics(&mut app);
        // Const is 5.0 → expect ≈ 1.39/tick (dt=1/30). Without the fallback,
        // forward_speed would be ~0 (0× accel during impulse).
        assert!(
            physics.forward_speed >= 1.0,
            "zero acceleration_multiplier must fall back to const; \
             got forward_speed={}",
            physics.forward_speed
        );
    }

    // ── Boost drive tests ─────────────────────────────────────────────

    fn enabled_boost_config() -> BoostConfigResource {
        BoostConfigResource {
            enabled: true,
            multiplier: 3.0,
            steering_multiplier: 2.0,
            active_duration: 4.0,
            recharge_duration: 20.0,
        }
    }

    /// With boost enabled and engaged, the ship accelerates ~3× faster than the
    /// un-boosted baseline (multiplier applies to both accel and max speed).
    #[test]
    fn active_boost_triples_acceleration() {
        let mut app = test_app();
        app.insert_resource(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);
        app.world_mut().resource_mut::<ShipBoost>().0.toggle(); // engage
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 1.0,
                    steering: 0.0,
                },
            },
        );
        tick(&mut app);
        let boosted = get_ship_physics(&mut app).forward_speed;

        // Baseline: identical run with boost left disabled.
        let mut base = test_app();
        start_game_with_helm_and_science(&mut base);
        push(
            &mut base,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 1.0,
                    steering: 0.0,
                },
            },
        );
        tick(&mut base);
        let baseline = get_ship_physics(&mut base).forward_speed;

        assert!(baseline > 0.0, "baseline should move; got {baseline}");
        assert!(
            (boosted - baseline * 3.0).abs() < baseline * 0.1,
            "boosted ({boosted}) should be ~3× baseline ({baseline})"
        );
    }

    /// With boost enabled and engaged, steering uses the separate configured
    /// yaw-rate multiplier.
    #[test]
    fn active_boost_multiplies_steering_rate() {
        let mut app = test_app();
        app.insert_resource(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);
        app.world_mut().resource_mut::<ShipBoost>().0.toggle();
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 0.0,
                    steering: 1.0,
                },
            },
        );
        tick(&mut app);
        let boosted_yaw = get_ship_physics(&mut app).yaw;

        let mut base = test_app();
        start_game_with_helm_and_science(&mut base);
        push(
            &mut base,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 0.0,
                    steering: 1.0,
                },
            },
        );
        tick(&mut base);
        let baseline_yaw = get_ship_physics(&mut base).yaw;

        assert!(
            baseline_yaw > 0.0,
            "baseline should turn; got {baseline_yaw}"
        );
        assert!(
            (boosted_yaw - baseline_yaw * 2.0).abs() < baseline_yaw * 0.1,
            "boosted yaw ({boosted_yaw}) should be ~2× baseline ({baseline_yaw})"
        );
    }

    #[test]
    fn active_boost_battery_drain_scales_with_helm_demand() {
        let mut app = test_app();
        app.insert_resource(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        {
            let mut boost = app.world_mut().resource_mut::<ShipBoost>();
            boost.0.toggle();
        }
        {
            set_last_helm_input(&mut app, LastHelmInput { thrust: 1.0, steering: 1.0 });
        }

        tick(&mut app);

        let battery = app.world().resource::<ShipBoost>().0.battery;
        assert!(
            (battery - 0.9).abs() < 0.001,
            "full thrust + full steering should drain twice the base rate; got {battery}"
        );
    }

    #[test]
    fn active_boost_battery_does_not_drain_with_idle_helm() {
        let mut app = test_app();
        app.insert_resource(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        {
            let mut boost = app.world_mut().resource_mut::<ShipBoost>();
            boost.0.toggle();
        }

        tick(&mut app);

        let battery = app.world().resource::<ShipBoost>().0.battery;
        assert!(
            (battery - 1.0).abs() < f32::EPSILON,
            "idle helm should not spend boost battery; got {battery}"
        );
    }

    /// A `ToggleBoost` message engages the drive only when the feature is
    /// enabled for this ship.
    #[test]
    fn toggle_boost_engages_when_enabled() {
        let mut app = test_app();
        app.insert_resource(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<ShipBoost>().0.is_active());

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(!app.world().resource::<ShipBoost>().0.is_active());
    }

    #[test]
    fn control_system_toggle_boost_engages_when_enabled() {
        let mut app = test_app();
        app.insert_resource(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<ShipBoost>().0.is_active());
    }

    #[test]
    fn control_system_set_boost_sets_active_state() {
        let mut app = test_app();
        app.insert_resource(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::SetBoost { active: true },
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<ShipBoost>().0.is_active());

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::SetBoost { active: false },
            },
        );
        tick(&mut app);
        assert!(!app.world().resource::<ShipBoost>().0.is_active());
    }

    /// When boost is disabled (no TOML), `ToggleBoost` is ignored and the
    /// multiplier never applies even if the state were somehow active.
    #[test]
    fn toggle_boost_ignored_when_disabled() {
        let mut app = test_app(); // BoostConfigResource defaults to disabled
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(
            !app.world().resource::<ShipBoost>().0.is_active(),
            "ToggleBoost must be a no-op when boost is disabled"
        );
    }

    /// REGRESSION (review finding #1 / #5): when impulse starts charging,
    /// the server's `LastHelmInput` must be cleared so a stale steering
    /// value can't immediately fly the ship the moment impulse cancels
    /// (or in the post-active autopilot-disengage frame).
    ///
    /// Reproduce: send `HelmInput { steering: 1.0 }`, then
    /// `StartImpulseCharge`, then `CancelImpulse`, then tick. Without the
    /// fix the post-cancel tick will read the stale `steering=1.0` and
    /// rotate the ship.
    #[test]
    fn stale_helm_steering_cleared_when_impulse_starts_charging() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        // Player jams the steering hard right before pressing IMPULSE.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 0.0,
                    steering: 1.0,
                },
            },
        );
        tick(&mut app);

        // Press IMPULSE → starts charging. `LastHelmInput` must be cleared.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);
        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
            "impulse should be charging after StartImpulseCharge"
        );
        let last = get_last_helm_input(&mut app);
        assert_eq!(
            (last.thrust, last.steering),
            (0.0, 0.0),
            "LastHelmInput must be zeroed on Charging transition; got \
             thrust={}, steering={}",
            last.thrust,
            last.steering
        );

        // Snapshot yaw, then cancel and tick once. With the bug, the
        // post-cancel tick replays steering=1.0 and yaw changes.
        let yaw_before = get_ship_physics(&mut app).yaw;
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::CancelImpulse,
            },
        );
        tick(&mut app);
        let yaw_after = get_ship_physics(&mut app).yaw;
        assert!(
            (yaw_after - yaw_before).abs() < 1e-3,
            "post-cancel tick must not autopilot a phantom turn; \
             yaw drifted by {}",
            yaw_after - yaw_before
        );
    }

    // ── Station Rating tests ─────────────────────────────────────────────

    /// Build a ShipConfig with a captain station that has an "Assisted" rating
    /// declaring red-alert as automated. Used by rating-mechanism tests that
    /// need automation to be configured independently of player_ship.toml.
    fn ship_config_with_assisted_captain() -> ShipConfigComponent {
        const TOML: &str = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Captain"
rank = "Cpt."
short_code = "CPT"
console = "captain"

[[station.rating]]
name = "Assisted"
automated_systems = ["red-alert"]

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "helm"
name = "Helm"
description = "Helm"
rank = "Ltn."
short_code = "HLM"
console = "helm"

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "captain"
kind = "captain"
station = "captain"

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"

[[system]]
id = "viewscreen"
kind = "viewscreen"
ai_only = true

[[system]]
id = "helm"
kind = "helm"
station = "helm"
"#;
        const KINDS: &[&str] = &["captain", "red_alert", "viewscreen", "helm"];
        ShipConfigComponent(
            crate::ship::config::parse_and_validate(TOML, KINDS)
                .expect("test config must be valid"),
        )
    }

    #[test]
    fn set_station_rating_sets_ai_for_automated_systems() {
        let mut app = test_app();
        // Apply the custom config directly on the Ship entity — PendingShipConfig
        // is consumed by spawn_game_start_entities which is not in the test app.
        let custom_config = ship_config_with_assisted_captain();
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipConfigComponent, With<Ship>>();
        for mut cfg in q.iter_mut(app.world_mut()) {
            *cfg = custom_config.clone();
        }
        start_game_with_helm_and_science(&mut app);

        // Captain "Assisted" rating has red-alert in automated_systems.
        push(
            &mut app,
            "captain",
            ClientMessage::SetStationRating {
                rating_name: "Assisted".into(),
            },
        );
        tick_twice(&mut app);

        let sources = get_ship_control_sources(&mut app);
        assert_eq!(
            sources
                .0
                .source_for(&crate::system_registry::red_alert_system_id()),
            ControlSource::Ai
        );
    }

    #[test]
    fn set_station_rating_manual_leaves_all_systems_human() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::SetStationRating {
                rating_name: "Manual".into(),
            },
        );
        tick_twice(&mut app);

        let sources = get_ship_control_sources(&mut app);
        assert_eq!(
            sources
                .0
                .source_for(&crate::system_registry::red_alert_system_id()),
            ControlSource::Human
        );
    }

    #[test]
    fn set_station_rating_backfill_automates_all_station_systems() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::SetStationRating {
                rating_name: rating::BACKFILL_RATING.into(),
            },
        );
        tick_twice(&mut app);

        let sources = get_ship_control_sources(&mut app);
        assert_eq!(
            sources
                .0
                .source_for(&crate::system_registry::red_alert_system_id()),
            ControlSource::Ai
        );
        // viewscreen is now owned by the captain station, so backfill must automate it too.
        assert_eq!(
            sources
                .0
                .source_for(&crate::system_registry::viewscreen_system_id()),
            ControlSource::Ai,
            "backfill should also automate the captain-owned viewscreen system"
        );
    }

    #[test]
    fn set_station_rating_from_non_holder_is_ignored() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        // "helm" player holds Helm console, not captain.
        // SetStationRating for the captain station should be ignored.
        push(
            &mut app,
            "helm",
            ClientMessage::SetStationRating {
                rating_name: "Assisted".into(),
            },
        );
        tick_twice(&mut app);

        let sources = get_ship_control_sources(&mut app);
        // Default is Human
        assert_eq!(
            sources
                .0
                .source_for(&crate::system_registry::red_alert_system_id()),
            ControlSource::Human
        );
    }

    #[test]
    fn set_station_rating_updates_active_ratings() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::SetStationRating {
                rating_name: "Assisted".into(),
            },
        );
        tick_twice(&mut app);

        let active = get_ship_active_ratings(&mut app);
        assert_eq!(
            active
                .0
                .get(&StationId("captain".into()))
                .map(|s| s.as_str()),
            Some("Assisted")
        );
    }

    #[test]
    fn set_station_rating_emits_rating_changed() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::SetStationRating {
                rating_name: "Assisted".into(),
            },
        );
        tick_twice(&mut app);

        let outbox = app.world().resource::<crate::lobby::LobbyOutbox>();
        let has_rating_changed = outbox.0.iter().any(|(_, msg)| {
            matches!(
                msg,
                crate::messages::ServerMessage::RatingChanged {
                    station_id,
                    rating_name,
                } if station_id.0 == "captain" && rating_name == "Assisted"
            )
        });
        assert!(has_rating_changed, "expected RatingChanged in outbox");
    }

    // ── #575: player ship AI helm navigation ──────────────────────────────────

    fn reach_scored_objective(anchor: &str, score: f32) -> crate::messages::ScoredObjective {
        crate::messages::ScoredObjective {
            id: format!("reach-{anchor}"),
            score,
            directive: crate::messages::AiDirective::Reach {
                anchor: anchor.into(),
            },
            source: crate::messages::ObjectiveSource::Mission,
            relevance: vec![crate::messages::SystemAffinity::Helm],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: format!("reach-{anchor}"),
                text: format!("Reach {anchor}"),
                mandatory: true,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::messages::ObjectiveSource::Mission,
            },
        }
    }

    fn patrol_scored_objective(anchors: Vec<&str>, score: f32) -> crate::messages::ScoredObjective {
        crate::messages::ScoredObjective {
            id: "obj-defend".into(),
            score,
            directive: crate::messages::AiDirective::Patrol {
                anchors: anchors.into_iter().map(str::to_string).collect(),
                loop_path: true,
            },
            source: crate::messages::ObjectiveSource::Mission,
            relevance: vec![crate::messages::SystemAffinity::Helm],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "obj-defend".into(),
                text: "Defend Starbase Alpha".into(),
                mandatory: true,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec!["Starbase Alpha".into()],
                source: crate::messages::ObjectiveSource::Mission,
            },
        }
    }

    fn destroy_scored_objective(target: &str, score: f32) -> crate::messages::ScoredObjective {
        crate::messages::ScoredObjective {
            id: format!("destroy-{target}"),
            score,
            directive: crate::messages::AiDirective::Destroy {
                target: target.into(),
            },
            source: crate::messages::ObjectiveSource::Mission,
            relevance: vec![
                crate::messages::SystemAffinity::Helm,
                crate::messages::SystemAffinity::Weapons,
                crate::messages::SystemAffinity::Captain,
            ],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: format!("destroy-{target}"),
                text: format!("Destroy {target}"),
                mandatory: true,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![target.into()],
                source: crate::messages::ObjectiveSource::Mission,
            },
        }
    }

    fn set_ship_blackboard_objectives(
        app: &mut App,
        objectives: Vec<crate::messages::ScoredObjective>,
    ) {
        use crate::messages::{SystemBlackboard, ViewscreenBlackboard};
        let mut vb = ViewscreenBlackboard::default();
        vb.scored_objectives = objectives;
        let entry = (
            crate::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(vb),
        );
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<Ship>>();
        let mut bbs = q
            .single_mut(app.world_mut())
            .expect("expected Ship with ShipSystemBlackboards");
        bbs.0.insert(entry.0, entry.1);
    }

    fn world_config_with_anchor(anchor: &str, pos: [f32; 3]) -> crate::world::config::WorldConfig {
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.anchors.insert(anchor.into(), pos);
        cfg
    }

    #[test]
    fn player_ship_helm_ai_navigates_toward_reach_objective() {
        let mut app = test_app();
        // Place anchor 100 units ahead (positive X) — ship starts at origin.
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(
            &mut app,
            vec![reach_scored_objective(anchor, 10.0)],
        );
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "AI helm must apply positive thrust toward Reach anchor; got {last:?}"
        );
    }

    #[test]
    fn player_ship_helm_ai_patrols_from_viewscreen_objective() {
        let mut app = test_app();
        let anchor = "starbase_patrol_east";
        set_ship_blackboard_objectives(
            &mut app,
            vec![patrol_scored_objective(vec![anchor], 20.0)],
        );
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "AI helm must apply positive thrust toward Patrol anchor; got {last:?}"
        );
    }

    #[test]
    fn player_ship_helm_ai_pursues_named_destroy_objective() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
        app.insert_resource(runtime);
        set_ship_blackboard_objectives(
            &mut app,
            vec![destroy_scored_objective("wave_1", 80.0)],
        );
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "AI helm must pursue named Destroy objective target; got {last:?}"
        );
    }

    #[test]
    fn player_ship_helm_ai_does_nothing_when_helm_human() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(
            &mut app,
            vec![reach_scored_objective(anchor, 10.0)],
        );
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        // helm stays Human (default)

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert_eq!(
            last,
            LastHelmInput::default(),
            "helm AI must not overwrite LastHelmInput when helm is human; got {last:?}"
        );
    }

    #[test]
    fn player_ship_helm_ai_stays_zero_when_destroy_target_missing() {
        let mut app = test_app();
        // Blackboard has a Destroy directive, but no live entity resolves to it.
        use crate::messages::{
            AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
            SystemAffinity,
        };
        set_ship_blackboard_objectives(
            &mut app,
            vec![ScoredObjective {
                id: "destroy-pirates".into(),
                score: 5.0,
                directive: AiDirective::Destroy {
                    target: "pirate".into(),
                },
                source: ObjectiveSource::Mission,
                relevance: vec![SystemAffinity::Helm],
                snapshot: ObjectiveSnapshot {
                    id: "destroy-pirates".into(),
                    text: "Destroy pirates".into(),
                    mandatory: true,
                    status: ObjectiveStatus::Active,
                    targets: vec![],
                    source: ObjectiveSource::Mission,
                },
            }],
        );
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        // operate_helm_ai: unresolved Destroy target → zero thrust remains.
        let last = get_last_helm_input(&mut app);
        assert_eq!(
            last,
            LastHelmInput::default(),
            "missing Destroy target means Backfill zero should remain; got {last:?}"
        );
    }

    #[test]
    fn detect_reach_completion_marks_objective_complete() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig};
        use crate::world::server::ObjectiveManagerRes;

        let mut app = test_app();
        let anchor = "dock-alpha";
        // Anchor at origin — ship also starts at origin, so distance == 0.
        // detect_reached_objective_completion reads from ShipSystemBlackboards component.
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "reach-dock-alpha",
            "Dock at Alpha",
            true,
            vec![],
            AiDirective::Reach {
                anchor: anchor.into(),
            },
            UtilityConfig::default(),
            ObjectiveSource::Mission,
        );
        app.insert_resource(ObjectiveManagerRes(mgr));

        tick(&mut app);

        let res = app.world().resource::<ObjectiveManagerRes>();
        let obj = res
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == "reach-dock-alpha");
        assert!(
            obj.map(|o| o.status == crate::messages::ObjectiveStatus::Completed)
                .unwrap_or(false),
            "Reach objective should be completed when ship is within arrival radius"
        );
    }

    #[test]
    fn detect_reach_completion_does_not_complete_when_far() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig};
        use crate::world::server::ObjectiveManagerRes;

        let mut app = test_app();
        let anchor = "dock-far";
        // Anchor 500 units away — ship starts at origin.
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [500.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "reach-dock-far",
            "Dock at Far",
            true,
            vec![],
            AiDirective::Reach {
                anchor: anchor.into(),
            },
            UtilityConfig::default(),
            ObjectiveSource::Mission,
        );
        app.insert_resource(ObjectiveManagerRes(mgr));

        tick(&mut app);

        let res = app.world().resource::<ObjectiveManagerRes>();
        let obj = res
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == "reach-dock-far");
        assert!(
            obj.map(|o| o.status == crate::messages::ObjectiveStatus::Active)
                .unwrap_or(false),
            "Reach objective must remain Active when ship is far from the anchor"
        );
    }

    #[test]
    fn detect_reach_completion_does_not_complete_when_helm_human() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig};
        use crate::world::server::ObjectiveManagerRes;

        let mut app = test_app();
        let anchor = "dock-beta";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
        // helm stays Human — completion system must not fire

        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "reach-dock-beta",
            "Dock at Beta",
            true,
            vec![],
            AiDirective::Reach {
                anchor: anchor.into(),
            },
            UtilityConfig::default(),
            ObjectiveSource::Mission,
        );
        app.insert_resource(ObjectiveManagerRes(mgr));

        tick(&mut app);

        let res = app.world().resource::<ObjectiveManagerRes>();
        let obj = res
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == "reach-dock-beta");
        assert!(
            obj.map(|o| o.status == crate::messages::ObjectiveStatus::Active)
                .unwrap_or(false),
            "Reach completion must not fire when helm is human-controlled"
        );
    }

    // ── E5 smoke tests (#553) ─────────────────────────────────────────────────

    // (a) Pirate raider — verifies that an NPC ship with full Ai control has
    // `operate_ai = true` for helm, which is the gate that makes `operate_helm_ai`
    // apply Transform physics for that ship instead of the player ship path.
    #[test]
    fn pirate_raider_ai_helm_policy_routes_through_npc_path() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut resolver = ControlSourceResolver::new();
        for system_id in [
            crate::system_registry::helm_system_id(),
            crate::system_registry::tactical_system_id(),
        ] {
            resolver.set(system_id, ControlSource::Ai);
        }
        let sources = ShipSystemControlSources(resolver);
        let policy = helm_control_policy(&sources);
        assert!(
            policy.operate_ai,
            "NPC raider helm must route through operate_helm_ai"
        );
        assert!(
            !policy.accept_human_input,
            "NPC raider must not accept human helm input"
        );
    }

    // (b) All-Backfill player ship — verifies that when the player ship has all
    // systems on Ai control but no AiControllerComponent (no behaviour tree),
    // `helm_control_policy` still returns `operate_ai = true` so the player-ship
    // path in `operate_helm_ai` writes zero thrust/steering (safe deceleration).
    #[test]
    fn all_backfill_player_ship_helm_policy_gates_operate_ai() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut resolver = ControlSourceResolver::new();
        resolver.set(crate::system_registry::helm_system_id(), ControlSource::Ai);
        let sources = ShipSystemControlSources(resolver);
        let policy = helm_control_policy(&sources);
        assert!(
            policy.operate_ai,
            "Backfill player helm must satisfy the operate_ai gate"
        );
        assert!(
            !policy.accept_human_input,
            "Backfill player helm must not accept human input"
        );
    }

    // (c) Player ship Backfill runs full operate_helm (avoidance + doctrine).
    // Verifies that the player ship on Backfill uses the same unified operate_helm_ai
    // loop as NPC ships — not a Reach-only stub — satisfying issue #587 AC.
    #[test]
    fn player_ship_backfill_runs_full_operate_helm_with_objectives() {
        let mut app = test_app();
        // Give the ship a Destroy objective (non-Reach) pointing at an entity.
        let target_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.clone()),
            crate::entities::spawner::EntityName("enemy_fighter".into()),
            Transform::from_xyz(80.0, 0.0, 0.0),
        ));
        set_ship_blackboard_objectives(
            &mut app,
            vec![destroy_scored_objective("enemy_fighter", 60.0)],
        );
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        // The Destroy directive targets an entity at (80, 0). Full operate_helm
        // should produce non-zero thrust to pursue it.
        assert!(
            last.thrust > 0.0 || last.steering.abs() > 0.0,
            "player ship Backfill must run full operate_helm (non-Reach); \
             got thrust={}, steering={}",
            last.thrust,
            last.steering
        );
    }

    // (d) NPC helm finds nearest hostile via the loaded FactionRegistry.
    //
    // Regression guard for the #587 regression where `operate_helm_ai` was
    // passing `FactionRegistry::default()` (empty) instead of the live
    // `FactionRegistryResource`. With an empty registry `find_nearest_hostile`
    // never finds anyone, so NPC ships with `Destroy { target: "" }` doctrine
    // sat stationary even when enemies were present (combat_test.toml).
    //
    // Drives `operate_helm` (the pure core function) with a real vs empty
    // registry to confirm the fix works and documents the regression shape.
    #[test]
    fn npc_helm_finds_hostile_via_faction_registry() {
        use crate::faction::{FactionConfig, FactionRegistry};
        use crate::messages::{
            AiDirective, ObjectiveSource, ObjectiveSnapshot, ObjectiveStatus, ScoredObjective,
            SystemAffinity,
        };

        let fed_uuid =
            uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap();
        let harrow_uuid =
            uuid::Uuid::parse_str("cccccccc-3333-4333-8333-cccccccccccc").unwrap();
        let target_uuid = uuid::Uuid::new_v4();

        // Registry: Harrow lists Federation as an enemy (matches combat_test.toml
        // `add_faction_enemy { faction = "Harrow", enemy = "Federation" }`).
        let mut registry = FactionRegistry::new();
        registry.insert(FactionConfig {
            uuid: harrow_uuid,
            name: "Harrow".into(),
            enemies: vec![fed_uuid],
        });
        registry.insert(FactionConfig {
            uuid: fed_uuid,
            name: "Federation".into(),
            enemies: vec![],
        });

        // Doctrine pool matching pirate_raider.toml: `Destroy { target: "" }`
        // scored 35. With no explicit target, helm_destroy falls through to
        // `find_nearest_hostile` which consults the registry.
        let scored_pool = vec![ScoredObjective {
            id: "destroy-hostiles".into(),
            score: 35.0,
            directive: AiDirective::Destroy { target: String::new() },
            source: ObjectiveSource::Doctrine,
            relevance: vec![SystemAffinity::Helm, SystemAffinity::Weapons],
            snapshot: ObjectiveSnapshot {
                id: "destroy-hostiles".into(),
                text: "Engage and destroy hostile ships".into(),
                mandatory: false,
                status: ObjectiveStatus::Active,
                targets: vec![],
                source: ObjectiveSource::Doctrine,
            },
        }];

        // World view: NPC Harrow ship at origin, Federation target 100 units away.
        let world_view = crate::ai::WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_faction: Some(harrow_uuid),
            entities: vec![crate::ai::AiWorldEntity {
                uuid: target_uuid,
                faction: Some(fed_uuid),
                position: [100.0, 0.0, 0.0],
                ..Default::default()
            }],
            ..Default::default()
        };

        // With the real registry: Harrow finds and pursues the Federation target.
        let (thrust, _) = crate::ai::operate_helm(
            &mut crate::ai::AiMemory::default(),
            &world_view,
            &scored_pool,
            &[],
            &Default::default(),
            crate::ai::WAYPOINT_ARRIVAL_RADIUS,
            crate::ai::AVOIDANCE_BUFFER,
            crate::ai::AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &registry,
        );
        assert!(
            thrust > 0.0,
            "With real FactionRegistry, NPC must produce non-zero thrust toward hostile; \
             got thrust={}",
            thrust
        );

        // With an empty registry (the pre-fix regression): no target found, zero thrust.
        let (thrust_empty, _) = crate::ai::operate_helm(
            &mut crate::ai::AiMemory::default(),
            &world_view,
            &scored_pool,
            &[],
            &Default::default(),
            crate::ai::WAYPOINT_ARRIVAL_RADIUS,
            crate::ai::AVOIDANCE_BUFFER,
            crate::ai::AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &FactionRegistry::default(),
        );
        assert_eq!(
            thrust_empty, 0.0,
            "Empty FactionRegistry (regression) must produce zero thrust; got {}",
            thrust_empty
        );
    }

    // ── sync_console_damage_tiers integration tests ───────────────────────────

    /// Helper: get the policy for a system from the ship's ControlSourceResolver.
    fn get_policy(app: &mut App, system_id: &str) -> ControlTickPolicy {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemControlSources, With<Ship>>();
        let sources = q.single(app.world()).expect("Ship with ShipSystemControlSources");
        sources.0.policy_for(&crate::messages::SystemId(system_id.into()))
    }

    fn set_console_hp(app: &mut App, console: crate::messages::Console, hp: f32) {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut binding = app.world_mut().entity_mut(ship);
        let mut hull_component = binding
            .get_mut::<crate::entity_spawner::EntityConsoleHull>()
            .unwrap();
        // Wipe then restore to exact HP.
        hull_component.0.apply_damage(1_000_000.0, &mut rand::rng());
        hull_component.0.restore(console, hp);
    }

    #[test]
    fn disabled_console_gates_human_and_ai_input() {
        let mut app = test_app();
        // Helm console max_hp = 25. Disabled threshold = 25 % = 6.25 HP.
        // Set Helm to 5 HP (below disabled threshold) → Disabled tier.
        set_console_hp(&mut app, crate::messages::Console::Helm, 5.0);
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            !policy.accept_human_input,
            "Disabled console must not accept human input"
        );
        assert!(
            !policy.operate_ai,
            "Disabled console must not operate AI"
        );
    }

    #[test]
    fn destroyed_console_gates_human_and_ai_input() {
        let mut app = test_app();
        // Wipe helm to 0 HP → Destroyed tier.
        set_console_hp(&mut app, crate::messages::Console::Helm, 0.0);
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            !policy.accept_human_input,
            "Destroyed console must not accept human input"
        );
        assert!(
            !policy.operate_ai,
            "Destroyed console must not operate AI"
        );
    }

    #[test]
    fn restored_console_re_enables_input() {
        let mut app = test_app();
        // First disable helm.
        set_console_hp(&mut app, crate::messages::Console::Helm, 5.0);
        tick(&mut app);
        // Verify it is gated.
        assert!(!get_policy(&mut app, "helm").accept_human_input);

        // Now restore to operational HP.
        set_console_hp(&mut app, crate::messages::Console::Helm, 25.0);
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            policy.accept_human_input,
            "Restored console must accept human input again"
        );
    }

    #[test]
    fn damaged_tier_does_not_gate_input() {
        let mut app = test_app();
        // Helm at 50% = 12.5 HP → Damaged tier (25 % < 50 % < 75 %).
        // Damaged tier must NOT block input — only Disabled and Destroyed do.
        set_console_hp(&mut app, crate::messages::Console::Helm, 12.5);
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            policy.accept_human_input,
            "Damaged (but not Disabled) console must still accept human input"
        );
    }

    /// When helm is under AI control (`operate_ai = true`), `process_helm_inputs`
    /// must NOT apply a second physics integration step.
    ///
    /// `operate_helm_ai` already runs `compute_physics` and writes the result
    /// into `ShipPhysics` every Bevy frame.  If `process_helm_inputs` then runs
    /// `compute_physics` again using the updated state as its starting point
    /// (with a *different* fixed dt of 1/30 s), the player ship ends up moving
    /// ~3× faster than intended relative to NPC ships that are only driven by
    /// `operate_helm_ai`.
    ///
    /// Regression guard: set helm to AI, write a non-zero intent into
    /// `LastHelmInput`, tick the app, and assert that `ShipPhysics.x` does NOT
    /// advance by more than one single `operate_helm_ai` step.
    #[test]
    fn process_helm_inputs_skips_physics_when_helm_is_ai_controlled() {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Set a non-zero last input so that if process_helm_inputs incorrectly
        // runs compute_physics it will produce a non-trivial displacement.
        set_last_helm_input(&mut app, LastHelmInput { thrust: 1.0, steering: 0.0 });

        // Snapshot physics before the tick.
        let before = get_ship_physics(&mut app);

        tick(&mut app);

        let after = get_ship_physics(&mut app);

        // operate_helm_ai has no objectives in this test (blackboard empty), so
        // it zeros out LastHelmInput and skips to `continue` — physics stays put.
        // If process_helm_inputs fired anyway it would have used the stale
        // thrust=1.0 from before the tick and moved the ship.
        assert_eq!(
            after.x, before.x,
            "ShipPhysics.x must not advance when helm is AI-controlled: \
             process_helm_inputs must skip physics integration"
        );
        assert_eq!(
            after.forward_speed, before.forward_speed,
            "forward_speed must not change when process_helm_inputs skips physics"
        );
    }

    // ── Fine Helm system tests (issue #511) ───────────────────────────────────

    /// Build an app that includes HelmEnginePort + HelmEngineStarboard hull
    /// entries alongside the usual coarse consoles. Used for engine-damage tests.
    fn test_app_with_engine_hull() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(ShipModifiers::new())
            .add_plugins(ShipPlugin);
        let hull_config = &[
            (crate::messages::Console::Helm, 25.0_f32),
            (crate::messages::Console::Tactical, 25.0),
            (crate::messages::Console::Power, 25.0),
            (crate::messages::Console::Shields, 25.0),
            (crate::messages::Console::HelmEnginePort, 15.0),
            (crate::messages::Console::HelmEngineStarboard, 15.0),
        ];
        app.world_mut().spawn((
            Ship,
            LocalShip,
            Transform::default(),
            ShipPhysics::default(),
            ShipConfigComponent::default(),
            ShipSystemControlSources::default(),
            ActiveStationRatings::default(),
            CoordinationQueue::default(),
            crate::messages::AdmittedCommands::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ai_plugin::ShipAiMemory::default(),
            crate::entity_spawner::EntityConsoleHull(
                crate::damage::ConsoleHull::from_config(hull_config),
            ),
            LastHelmInput::default(),
            crate::simulation::ShipShields(crate::shield::ShieldSystem::default()),
            ShipImpulse(crate::impulse::ImpulseState::new()),
        ));
        app
    }

    /// Set the HP of a specific console on the LocalShip hull to `new_hp`.
    /// Delegates to `ConsoleHull::set_console_hp` which directly sets the value.
    fn set_console_hp_direct(app: &mut App, console: crate::messages::Console, new_hp: f32) {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut entity_mut = app.world_mut().entity_mut(ship);
        let mut hull = entity_mut
            .get_mut::<crate::entity_spawner::EntityConsoleHull>()
            .unwrap();
        hull.0.set_console_hp(&console, new_hp);
    }

    #[test]
    fn joystick_publishes_state_to_engines_via_inter_system() {
        let mut app = test_app_with_engine_hull();
        // Ensure InterSystemQueue is initialised.
        app.init_resource::<InterSystemQueue>();

        // Set a known LastHelmInput before ticking.
        set_last_helm_input(
            &mut app,
            LastHelmInput {
                thrust: 0.75,
                steering: 0.25,
            },
        );

        tick(&mut app);

        let queue = app.world().resource::<InterSystemQueue>();
        let port_id = crate::system_registry::helm_engine_port_system_id();
        let stbd_id = crate::system_registry::helm_engine_starboard_system_id();

        let port_msgs: Vec<_> = queue
            .for_target(port_id.0.as_str())
            .collect();
        let stbd_msgs: Vec<_> = queue
            .for_target(stbd_id.0.as_str())
            .collect();

        // `publish_joystick_to_engines` and `operate_helm_engine_ai` may both push.
        // At least one message must arrive for each engine.
        assert!(
            !port_msgs.is_empty(),
            "expected at least one JoystickState message for helm-engine-port"
        );
        assert!(
            !stbd_msgs.is_empty(),
            "expected at least one JoystickState message for helm-engine-starboard"
        );

        // The first message should carry the joystick values.
        let InterSystemPayload::JoystickState { thrust, steering } =
            &port_msgs[0].payload
        else {
            panic!("expected JoystickState payload for port engine");
        };
        assert!(
            (*thrust - 0.75).abs() < 0.01,
            "port engine thrust should match joystick thrust"
        );
        assert!(
            (*steering - 0.25).abs() < 0.01,
            "port engine steering should match joystick steering"
        );
    }

    #[test]
    fn engine_port_hull_damage_gates_engine_offline() {
        let mut app = test_app_with_engine_hull();

        // Zero out the port engine HP (destroyed tier).
        set_console_hp_direct(&mut app, crate::messages::Console::HelmEnginePort, 0.0);
        tick(&mut app);

        // After sync_console_damage_tiers, offline_systems should contain helm-engine-port.
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let control_sources = app
            .world()
            .entity(ship)
            .get::<ShipSystemControlSources>()
            .unwrap();
        let port_id = crate::system_registry::helm_engine_port_system_id();
        assert!(
            control_sources.0.offline_systems.contains(&port_id),
            "helm-engine-port should be in offline_systems when HP = 0"
        );
    }

    #[test]
    fn engine_port_offline_reduces_thrust_compared_to_both_online() {
        // With both engines online, terminal velocity = max_speed (25 m/s by default).
        // With one engine offline, effective thrust = 0.5, so terminal = 0.5 * max_speed = 12.5.
        // We run enough ticks to approach terminal velocity at the 50%-thrust case,
        // then verify the one-engine-offline ship is slower than the both-online ship.
        const TICK_MS: u64 = 34; // slightly above 1/30s so timer fires once per tick
        const TICKS: usize = 120; // 120 ticks × 34ms ≈ 4s, enough to reach ~12.5 m/s terminal

        let make_app = || {
            let mut app = test_app_with_engine_hull();
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(TICK_MS),
            ));
            app
        };

        // ── Both engines online ────────────────────────────────────────────
        let mut app_both = make_app();
        set_last_helm_input(&mut app_both, LastHelmInput { thrust: 1.0, steering: 0.0 });
        for _ in 0..TICKS { tick(&mut app_both); }
        let speed_both = app_both
            .world_mut()
            .query_filtered::<&ShipPhysics, With<LocalShip>>()
            .single(app_both.world())
            .unwrap()
            .forward_speed;

        // ── Port engine disabled ───────────────────────────────────────────
        // Zero the port engine HP, tick once so sync_console_damage_tiers runs
        // (populating offline_systems), then drive at full thrust for TICKS more.
        let mut app_one = make_app();
        set_console_hp_direct(&mut app_one, crate::messages::Console::HelmEnginePort, 0.0);
        tick(&mut app_one); // let Damage tier propagate
        set_last_helm_input(&mut app_one, LastHelmInput { thrust: 1.0, steering: 0.0 });
        for _ in 0..TICKS { tick(&mut app_one); }
        let speed_one = app_one
            .world_mut()
            .query_filtered::<&ShipPhysics, With<LocalShip>>()
            .single(app_one.world())
            .unwrap()
            .forward_speed;

        // With enough ticks, app_both should be near 25 m/s and app_one near 12.5 m/s.
        assert!(
            speed_one < speed_both,
            "forward_speed with one engine offline ({speed_one:.4}) should be less than \
             with both engines online ({speed_both:.4})"
        );
    }
}

