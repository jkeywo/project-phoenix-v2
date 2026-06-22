use bevy::prelude::*;
use std::collections::HashMap;

use crate::control_source::{ControlSourceResolver, ControlTickPolicy};
use crate::entity_spawner::RegionEffectsSection;
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{
    ClientMessage, CoordinationPayload, ModifierSlot, StationId, SystemControlPayload,
};
use crate::modifiers::ShipModifiers;
use crate::region_effects::RegionEffectKind;
use crate::region_plugin::RegionMembership;
use crate::ship::config::ShipConfig;
use crate::ship::coordination;
use crate::ship::coordination::{CoordinationLagQueue, QueuedCoordination};
use crate::ship::control_source::ControlSource;
use crate::ship::rating;
use crate::ship_physics::{compute_physics, ShipPhysicsConfig, ShipPhysicsInput, ShipPhysicsState};
use crate::ship_state::ShipState;
use crate::simulation::{Ship, ShipBoost, ShipHullIntegrity, ShipImpulse};

// Ã¢â€â‚¬Ã¢â€â‚¬ Resources Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

#[derive(Resource)]
struct HelmInputTimer(Timer);

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct LastHelmInput {
    pub thrust: f32,
    pub steering: f32,
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct ShipSystemControlSources(pub ControlSourceResolver);

/// The parsed `ShipConfig` defining stations, systems, and per-station rating
/// tables. Populated once at startup from the embedded ship TOML.
#[derive(Resource, Clone)]
pub struct ShipConfigResource(pub ShipConfig);

/// Tracks the currently active rating name for each station.
/// Updated when a player sends `SetStationRating`.
#[derive(Resource, Clone, Debug, Default)]
pub struct ActiveStationRatings(pub HashMap<StationId, String>);

/// Channel-3 coordination lag queue. Holds pending coordination messages
/// until their due time, at which point they are routed by the delivery-time
/// matrix (issue #494).
#[derive(Resource, Clone, Debug, Default)]
pub struct CoordinationQueueResource(pub CoordinationLagQueue);

/// Server-side enqueue event for channel-3 coordination messages.
/// AI controllers fire this to send delayed advisories to human operators.
#[derive(Message, Clone, Debug)]
pub struct CoordinationEnqueue {
    pub sender_origin: ControlSource,
    pub target: crate::messages::SystemId,
    pub payload: CoordinationPayload,
    pub sender_label: String,
}

impl Default for ShipConfigResource {
    fn default() -> Self {
        // Default config matches the test TOML in ship/config.rs.
        // TODO: replace with the real ship config loading once the
        // station/system migration lands on player_ship.toml.
        let toml = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."
short_code = "CPT"
console = "captain"

[[station.rating]]
name = "Assisted"
automated_systems = ["red-alert"]

[[station.rating]]
name = "Manual"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons and threat response."
rank = "Ltn."
short_code = "TAC"
console = "tactical"

[[station.rating]]
name = "Assisted"
automated_systems = ["torpedo-magazine", "torpedo-tube-fore-port"]

[power_groups.ops]
label = "Operations"
default_level = 2
min_level = 1
max_level = 4

[power_groups.weapons]
label = "Weapons"
default_level = 2
min_level = 1
max_level = 4

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
power_group = "ops"

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
power_group = "weapons"

[system.config]
facing_deg = 0
fire_arc_deg = 270

[[system]]
id = "torpedo-magazine"
kind = "torpedo_magazine"
station = "tactical"
power_group = "weapons"

[[system]]
id = "torpedo-tube-fore-port"
kind = "torpedo_tube"
station = "tactical"
power_group = "weapons"

[[system]]
id = "viewscreen"
kind = "viewscreen"
ai_only = true
power_group = "ops"
"#;
        const KINDS: &[&str] = &[
            "red_alert",
            "helm",
            "phaser_bank",
            "torpedo_magazine",
            "torpedo_tube",
            "viewscreen",
        ];
        let config = ShipConfig::from_toml(toml, KINDS)
            .expect("default ship config must parse");
        Self(config)
    }
}

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct HelmAiController {
    pub thrust: f32,
    pub steering: f32,
}

impl Default for HelmAiController {
    fn default() -> Self {
        Self {
            thrust: 0.5,
            steering: 0.0,
        }
    }
}

impl HelmAiController {
    fn operate(self) -> LastHelmInput {
        LastHelmInput {
            thrust: self.thrust,
            steering: self.steering,
        }
    }
}

/// Runtime ship physics config, loaded from `[helm_console]` in the entity TOML.
/// When absent, `ShipPhysicsConfig::new()` defaults are used.
#[derive(Resource, Clone)]
pub struct ShipPhysicsConfigResource(pub crate::ship_physics::ShipPhysicsConfig);

/// Runtime impulse drive config, loaded from `[helm_console]` in the entity TOML.
/// Charge duration and speed multiplier can be overridden per ship.
#[derive(Resource, Clone)]
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
#[derive(Resource, Clone)]
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
#[derive(Resource, Clone)]
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

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmInputTimer(Timer::from_seconds(
            1.0 / 30.0,
            TimerMode::Repeating,
        )))
        .init_resource::<LastHelmInput>()
        .init_resource::<ShipSystemControlSources>()
        .init_resource::<ShipConfigResource>()
        .init_resource::<ActiveStationRatings>()
        .init_resource::<HelmAiController>()
        .init_resource::<ImpulseConfigResource>()
        .init_resource::<BoostConfigResource>()
        .init_resource::<ShipBoost>()
        .init_resource::<BankConfigResource>()
        .init_resource::<CoordinationQueueResource>()
        .add_message::<CoordinationEnqueue>()
        .add_systems(
            Update,
            (
                process_helm_inputs.in_set(crate::sim_sets::SimSet::Physics),
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
            )
                .after(crate::lobby::process_lobby),
        );
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Systems Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

fn process_helm_inputs(
    time: Res<Time>,
    mut timer: ResMut<HelmInputTimer>,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut ship: ResMut<ShipState>,
    mut last_input: ResMut<LastHelmInput>,
    modifiers: Res<ShipModifiers>,
    ship_physics_config: Option<Res<ShipPhysicsConfigResource>>,
    impulse: Res<ShipImpulse>,
    impulse_config: Res<ImpulseConfigResource>,
    boost: Res<ShipBoost>,
    boost_config: Res<BoostConfigResource>,
    bank_config: Res<BankConfigResource>,
    control_sources: Res<ShipSystemControlSources>,
    helm_ai: Res<HelmAiController>,
    mut prev_phase: Local<Option<crate::impulse::ImpulsePhase>>,
) {
    // Edge-detect Idle → Charging (or any → Charging) and zero out the
    // last cached helm input so a stale steering/thrust value can't
    // resurface the moment impulse cancels or the autopilot disengages.
    // Mirrors the `prev_phase` Local pattern in
    // `modifiers/coordination.rs::translate_impulse_modifiers`.
    let current_phase = impulse.0.phase;
    if Some(current_phase) != *prev_phase {
        if current_phase == crate::impulse::ImpulsePhase::Charging {
            last_input.thrust = 0.0;
            last_input.steering = 0.0;
        }
        *prev_phase = Some(current_phase);
    }

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let policy = control_sources
        .0
        .policy_for(&crate::system_registry::helm_system_id());
    let helm_token = sessions.0.console_holder(crate::messages::Console::Helm);
    if policy.accept_human_input && helm_token.is_none() {
        return;
    }

    for ev in reader.read() {
        if !policy.accept_human_input {
            continue;
        }
        if helm_token != Some(ev.token.as_str()) {
            continue;
        }
        if let Some(input) = helm_input_from_message(&ev.msg) {
            *last_input = input;
        }
    }
    if policy.operate_ai {
        *last_input = helm_ai.operate();
    }

    let dt = timer.0.duration().as_secs_f32();
    let state = ShipPhysicsState {
        x: ship.x,
        z: ship.z,
        yaw: ship.yaw,
        forward_speed: ship.forward_speed,
    };
    let impulse_active = impulse.0.is_active();
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
    let mut config = match ship_physics_config.as_deref() {
        Some(cfg) => cfg.0,
        None => ShipPhysicsConfig::new(),
    };
    config.max_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
    config.max_reverse_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
    config.max_yaw_rate *= modifiers.get(&ModifierSlot::MaxYawRate);
    if impulse_active {
        // Mirror `ship/impulse.rs::apply_to_physics`: a non-positive
        // multiplier (e.g. an unset TOML field defaulting to 0) falls
        // back to the const instead of nuking acceleration entirely.
        let mult = if impulse_config.acceleration_multiplier > 0.0 {
            impulse_config.acceleration_multiplier
        } else {
            crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER
        };
        config.acceleration *= mult;
    }
    // Boost drive: while engaged, multiply max speed and acceleration. Only
    // applies when the ship's TOML enabled the feature.
    if boost_config.enabled && boost.0.is_active() {
        config.max_speed *= boost_config.multiplier;
        config.max_reverse_speed *= boost_config.multiplier;
        config.acceleration *= boost_config.multiplier;
        config.max_yaw_rate *= boost_config.steering_multiplier;
    }
    let result = compute_physics(state, input, dt, &config);

    ship.x = result.x;
    ship.z = result.z;
    ship.yaw = result.yaw;
    ship.forward_speed = result.forward_speed;

    // Visual banking: lerp roll toward target based on steering
    let max_bank_rad = bank_config.max_bank_deg.to_radians();
    let target_roll = if impulse_active {
        0.0
    } else {
        -input.steering * max_bank_rad
    };
    let lerp_factor = (bank_config.bank_lerp_rate * dt).min(1.0);
    ship.roll = ship.roll + (target_roll - ship.roll) * lerp_factor;
}

fn helm_control_policy(sources: &ShipSystemControlSources) -> ControlTickPolicy {
    sources.0.policy_for(&crate::system_registry::helm_system_id())
}

fn helm_input_from_message(msg: &ClientMessage) -> Option<LastHelmInput> {
    match msg {
        ClientMessage::HelmInput { thrust, steering } => Some(LastHelmInput {
            thrust: *thrust,
            steering: *steering,
        }),
        ClientMessage::ControlSystem { target, payload }
            if target.0 == crate::system_registry::HELM_SYSTEM_ID =>
        {
            match payload {
                SystemControlPayload::HelmInput { thrust, steering } => Some(LastHelmInput {
                    thrust: *thrust,
                    steering: *steering,
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

fn helm_payload_from_message(msg: &ClientMessage) -> Option<&SystemControlPayload> {
    match msg {
        ClientMessage::ControlSystem { target, payload }
            if target.0 == crate::system_registry::HELM_SYSTEM_ID =>
        {
            Some(payload)
        }
        _ => None,
    }
}

fn sync_ship_position(ship: Res<ShipState>, mut ship_query: Query<&mut Transform, With<Ship>>) {
    let Ok(mut transform) = ship_query.single_mut() else {
        return;
    };

    transform.translation.x = ship.x;
    transform.translation.z = ship.z;
    transform.rotation = Quat::from_euler(EulerRot::YXZ, ship.yaw, 0.0, ship.roll);
}

pub fn handle_impulse_messages(
    mut reader: MessageReader<InboundMessage>,
    mut impulse: ResMut<ShipImpulse>,
    hull: Res<ShipHullIntegrity>,
    mut last_hull_hp: Local<f32>,
    membership: Option<Res<RegionMembership>>,
    region_query: Query<&RegionEffectsSection>,
    ship_query: Query<Entity, With<Ship>>,
    control_sources: Res<ShipSystemControlSources>,
) {
    if *last_hull_hp == 0.0 && (hull.0.total_current() - hull.0.total_max()).abs() < 1e-6 {
        *last_hull_hp = hull.0.total_max();
    }

    let current_hp = hull.0.total_current();
    if current_hp < *last_hull_hp {
        impulse.0.cancel_charge();
    }
    *last_hull_hp = current_hp;

    let policy = helm_control_policy(&control_sources);
    for msg in reader.read() {
        match &msg.msg {
            ClientMessage::StartImpulseCharge
                if policy.accept_human_input
                    && !is_inside_blocks_impulse(&membership, &region_query, &ship_query) =>
            {
                impulse.0.start_charge();
            }
            ClientMessage::ControlSystem { .. }
                if policy.accept_human_input
                    && matches!(
                        helm_payload_from_message(&msg.msg),
                        Some(SystemControlPayload::StartImpulseCharge)
                    )
                    && !is_inside_blocks_impulse(&membership, &region_query, &ship_query) =>
            {
                impulse.0.start_charge();
            }
            ClientMessage::CancelImpulse if policy.accept_human_input => {
                impulse.0.cancel_charge();
            }
            ClientMessage::ControlSystem { .. }
                if policy.accept_human_input
                    && matches!(
                        helm_payload_from_message(&msg.msg),
                        Some(SystemControlPayload::CancelImpulse)
                    ) =>
            {
                impulse.0.cancel_charge();
            }
            _ => {}
        }
    }
}

fn tick_impulse(
    time: Res<Time>,
    mut impulse: ResMut<ShipImpulse>,
    config: Res<ImpulseConfigResource>,
) {
    impulse.0.tick(time.delta_secs(), config.charge_duration);
}

/// Toggle the boost drive in response to Helm boost controls. No-op when the
/// feature is disabled or Helm is currently AI-operated.
pub fn handle_boost_messages(
    mut reader: MessageReader<InboundMessage>,
    mut boost: ResMut<ShipBoost>,
    config: Res<BoostConfigResource>,
    control_sources: Res<ShipSystemControlSources>,
) {
    if !config.enabled {
        return;
    }
    let policy = helm_control_policy(&control_sources);
    if !policy.accept_human_input {
        for _ in reader.read() {}
        return;
    }
    for msg in reader.read() {
        match &msg.msg {
            ClientMessage::ToggleBoost => boost.0.toggle(),
            ClientMessage::ControlSystem { .. }
                if matches!(
                    helm_payload_from_message(&msg.msg),
                    Some(SystemControlPayload::ToggleBoost)
                ) =>
            {
                boost.0.toggle();
            }
            ClientMessage::SetBoost { active } => {
                if *active { boost.0.activate(); } else { boost.0.deactivate(); }
            }
            ClientMessage::ControlSystem { .. } => {
                if let Some(SystemControlPayload::SetBoost { active }) =
                    helm_payload_from_message(&msg.msg)
                {
                    if *active { boost.0.activate(); } else { boost.0.deactivate(); }
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
    mut boost: ResMut<ShipBoost>,
    config: Res<BoostConfigResource>,
    last_input: Res<LastHelmInput>,
    sessions: Res<Sessions>,
    impulse: Res<ShipImpulse>,
    control_sources: Res<ShipSystemControlSources>,
) {
    if !config.enabled {
        return;
    }
    let policy = helm_control_policy(&control_sources);
    let has_helm = sessions
        .0
        .console_holder(crate::messages::Console::Helm)
        .is_some()
        || policy.operate_ai;
    let drain_factor = if !has_helm {
        0.0
    } else if impulse.0.is_active() {
        normalized_boost_drain_factor(1.0, 0.0)
    } else {
        normalized_boost_drain_factor(last_input.thrust, last_input.steering)
    };
    boost.0.tick_with_drain_factor(
        time.delta_secs(),
        config.active_duration,
        config.recharge_duration,
        drain_factor,
    );
}

fn is_inside_blocks_impulse(
    membership: &Option<Res<RegionMembership>>,
    region_query: &Query<&RegionEffectsSection>,
    ship_query: &Query<Entity, With<Ship>>,
) -> bool {
    let Some(membership) = membership else {
        return false;
    };
    let Ok(ship_entity) = ship_query.single() else {
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
    ship_config: Res<ShipConfigResource>,
    mut control_sources: ResMut<ShipSystemControlSources>,
    mut active_ratings: ResMut<ActiveStationRatings>,
) {
    for ev in reader.read() {
        let ClientMessage::SetStationRating { rating_name } = &ev.msg else {
            continue;
        };

        // Find the player
        let player = match sessions.0.players().iter().find(|p| p.token == ev.token) {
            Some(p) => p,
            None => continue,
        };

        // Determine which station the player holds via their consoles
        let mut station_id: Option<StationId> = None;
        for console in &player.consoles {
            if let Some(station) = ship_config
                .0
                .station_for_console(console.station_console_id())
            {
                station_id = Some(station.id.clone());
                break;
            }
        }
        let Some(station_id) = station_id else {
            continue;
        };

        // Apply the rating
        rating::apply_rating(
            &ship_config.0,
            &station_id,
            rating_name,
            &mut control_sources.0,
        );

        // Track the active rating
        active_ratings.0.insert(station_id, rating_name.clone());
    }
}

pub fn handle_coordination_enqueue(
    mut queue: ResMut<CoordinationQueueResource>,
    ship_config: Res<ShipConfigResource>,
    mut events: MessageReader<CoordinationEnqueue>,
    mut inbound: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    let lag = ship_config.0.coordination_lag_secs;

    for ev in events.read() {
        queue.0.enqueue(QueuedCoordination {
            sender_origin: ev.sender_origin,
            target: ev.target.clone(),
            payload: ev.payload.clone(),
            sender_label: ev.sender_label.clone(),
            due_time: now + lag,
        });
    }

    for msg in inbound.read() {
        let ClientMessage::SendCoordination { target, payload } = &msg.msg else {
            continue;
        };

        let player = match sessions.0.players().iter().find(|p| p.token == msg.token) {
            Some(p) => p,
            None => continue,
        };
        let sender_origin = if player.consoles.is_empty() {
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
    mut queue: ResMut<CoordinationQueueResource>,
    control_sources: Res<ShipSystemControlSources>,
    sessions: Res<Sessions>,
    mut outbox: ResMut<crate::lobby::LobbyOutbox>,
    ship_config: Res<ShipConfigResource>,
) {
    let now = time.elapsed_secs();
    let due = queue.0.due_messages(now);
    if due.is_empty() {
        return;
    }

    for msg in due {
        let target_control = control_sources.0.source_for(&msg.target);
        let action = coordination::route_coordination(msg.sender_origin, target_control);

        match action {
            coordination::DeliverAction::Consume => {}
            coordination::DeliverAction::Suppress => {}
            coordination::DeliverAction::Popup => {
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
                        let token: Option<String> = [crate::messages::Console::CaptainChair, crate::messages::Console::Helm, crate::messages::Console::Tactical, crate::messages::Console::Repair, crate::messages::Console::Sensors, crate::messages::Console::Shields, crate::messages::Console::Navigation, crate::messages::Console::Power, crate::messages::Console::Comms]
                            .iter()
                            .find(|c| c.station_console_id() == console_id)
                            .and_then(|console| {
                                sessions.0.console_holder(console.clone()).map(|t| t.to_string())
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

pub fn handle_coordination_messages(
    mut reader: MessageReader<InboundMessage>,
) {
    for msg in reader.read() {
        let ClientMessage::SendCoordination { .. } = &msg.msg else {
            continue;
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::EntityConfig;
    use crate::entity_spawner::spawn_entity;
    use crate::impulse::{ImpulsePhase, IMPULSE_CHARGE_DURATION};
    use crate::lobby::LobbyPlugin;
    use crate::messages::ClientMessage;
    use crate::control_source::ControlSource;
    use crate::modifiers::ShipModifiers;
    use crate::region_effects::{BlocksImpulseEffect, RegionEffectsConfig};
    use crate::region_shape::RegionShape;
    use crate::regions::server::RegionPlugin;
    use crate::messages::StationId;
    use crate::ship::rating;
    use crate::ship_state::ShipState;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(crate::damage::ConsoleHull::from_config(
                &[
                    (crate::messages::Console::Helm, 25.0),
                    (crate::messages::Console::Tactical, 25.0),
                    (crate::messages::Console::Power, 25.0),
                    (crate::messages::Console::Shields, 25.0),
                ],
            )))
            .insert_resource(crate::simulation::ShipShields(
                crate::shield::ShieldSystem::default(),
            ))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(ShipModifiers::new())
            .add_plugins(ShipPlugin);
        app
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
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn set_helm_control_source(app: &mut App, source: ControlSource) {
        app.world_mut()
            .resource_mut::<ShipSystemControlSources>()
            .0
            .set(crate::system_registry::helm_system_id(), source);
    }

    // ── Helm system control-source tests ───────────────────────────────────

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
            *app.world().resource::<LastHelmInput>(),
            LastHelmInput {
                thrust: 1.0,
                steering: 0.25
            }
        );
        assert!(app.world().resource::<ShipState>().forward_speed > 0.0);
    }

    #[test]
    fn ai_helm_operates_without_human_holder() {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick_twice(&mut app);

        assert_eq!(
            *app.world().resource::<LastHelmInput>(),
            LastHelmInput {
                thrust: 0.5,
                steering: 0.0
            }
        );
        assert!(app.world().resource::<ShipState>().forward_speed > 0.0);
    }

    #[test]
    fn ai_helm_ignores_human_input() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);
        set_helm_control_source(&mut app, ControlSource::Ai);
        app.insert_resource(HelmAiController {
            thrust: 0.25,
            steering: 0.0,
        });

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

        assert_eq!(
            *app.world().resource::<LastHelmInput>(),
            LastHelmInput {
                thrust: 0.25,
                steering: 0.0
            }
        );
    }

    #[test]
    fn human_helm_suppresses_ai_operate() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);
        app.insert_resource(HelmAiController {
            thrust: 1.0,
            steering: 0.0,
        });

        tick(&mut app);

        assert_eq!(*app.world().resource::<LastHelmInput>(), LastHelmInput::default());
        assert_eq!(app.world().resource::<ShipState>().forward_speed, 0.0);
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ Impulse Drive / Damage Cancellation tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn hull_damage_cancels_charging_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Charging,
            "impulse should be charging after StartImpulseCharge"
        );

        {
            let mut rng = rand::rng();
            app.world_mut()
                .resource_mut::<ShipHullIntegrity>()
                .0
                .apply_damage(10.0, &mut rng);
        }
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
            "impulse charge should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn hull_damage_cancels_active_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        {
            let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
            imp.0.start_charge();
            imp.0.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        }
        assert!(
            app.world().resource::<ShipImpulse>().0.is_active(),
            "impulse should be active before damage"
        );

        {
            let mut rng = rand::rng();
            app.world_mut()
                .resource_mut::<ShipHullIntegrity>()
                .0
                .apply_damage(10.0, &mut rng);
        }
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
            "active impulse should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn no_hull_damage_does_not_cancel_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Charging,
            "impulse should still be charging when no damage occurred"
        );
    }

    #[test]
    fn start_impulse_charge_message_begins_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
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
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Charging,
        );
    }

    #[test]
    fn cancel_impulse_message_cancels_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);
        push(&mut app, "helm", ClientMessage::CancelImpulse);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
        );
    }

    #[test]
    fn control_system_cancel_impulse_cancels_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
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
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
        );
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ BlocksImpulse region gating tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    fn blocks_impulse_test_app() -> App {
        let mut app = test_app();
        app.add_plugins(RegionPlugin);
        app.world_mut().spawn((Ship, Transform::default()));
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
            shields: None,
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
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
            "impulse should be idle before StartImpulseCharge"
        );

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
            "StartImpulseCharge should be ignored inside BlocksImpulse region"
        );
    }

    #[test]
    fn start_impulse_charge_works_outside_blocks_impulse_region() {
        let mut app = blocks_impulse_test_app();

        let _region = spawn_blocks_impulse_region(&mut app, 500.0, 0.0, 50.0);

        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
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
            let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
            imp.0.start_charge();
            imp.0.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        }

        // Player tries to fight the autopilot: zero thrust, hard right steer.
        // The server must ignore both and force thrust=1.0, steering=0.0.
        push(
            &mut app,
            "helm",
            ClientMessage::HelmInput {
                thrust: 0.0,
                steering: 1.0,
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipState>();
        // With 5x boost, expect ≈1.39; without boost ≈0.28. Require >=1.0 to
        // clearly distinguish the boosted path.
        assert!(
            ship.forward_speed >= 1.0,
            "active impulse should autopilot with boosted accel; got forward_speed={}",
            ship.forward_speed
        );
        // Steering must be ignored — yaw should be essentially unchanged.
        assert!(
            ship.yaw.abs() < 1e-3,
            "active impulse must zero steering; got yaw={}",
            ship.yaw
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
            ClientMessage::HelmInput {
                thrust: 1.0,
                steering: 0.0,
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipState>();
        // Base accel ≈ 8.33, dt = 1/30 → expected ≈ 0.28. Cap at 2.0 to
        // catch any accidental boost.
        assert!(
            ship.forward_speed < 2.0,
            "idle impulse must not boost accel; got forward_speed={}",
            ship.forward_speed
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
            let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
            imp.0.start_charge();
            imp.0.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        }
        tick(&mut app);

        let ship = app.world().resource::<ShipState>();
        // Const is 5.0 → expect ≈ 1.39/tick (dt=1/30). Without the fallback,
        // forward_speed would be ~0 (0× accel during impulse).
        assert!(
            ship.forward_speed >= 1.0,
            "zero acceleration_multiplier must fall back to const; \
             got forward_speed={}",
            ship.forward_speed
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
            ClientMessage::HelmInput {
                thrust: 1.0,
                steering: 0.0,
            },
        );
        tick(&mut app);
        let boosted = app.world().resource::<ShipState>().forward_speed;

        // Baseline: identical run with boost left disabled.
        let mut base = test_app();
        start_game_with_helm_and_science(&mut base);
        push(
            &mut base,
            "helm",
            ClientMessage::HelmInput {
                thrust: 1.0,
                steering: 0.0,
            },
        );
        tick(&mut base);
        let baseline = base.world().resource::<ShipState>().forward_speed;

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
            ClientMessage::HelmInput {
                thrust: 0.0,
                steering: 1.0,
            },
        );
        tick(&mut app);
        let boosted_yaw = app.world().resource::<ShipState>().yaw;

        let mut base = test_app();
        start_game_with_helm_and_science(&mut base);
        push(
            &mut base,
            "helm",
            ClientMessage::HelmInput {
                thrust: 0.0,
                steering: 1.0,
            },
        );
        tick(&mut base);
        let baseline_yaw = base.world().resource::<ShipState>().yaw;

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
            let mut last_input = app.world_mut().resource_mut::<LastHelmInput>();
            last_input.thrust = 1.0;
            last_input.steering = 1.0;
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

        push(&mut app, "helm", ClientMessage::ToggleBoost);
        tick(&mut app);
        assert!(app.world().resource::<ShipBoost>().0.is_active());

        push(&mut app, "helm", ClientMessage::ToggleBoost);
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

        push(&mut app, "helm", ClientMessage::ToggleBoost);
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
            ClientMessage::HelmInput {
                thrust: 0.0,
                steering: 1.0,
            },
        );
        tick(&mut app);

        // Press IMPULSE → starts charging. `LastHelmInput` must be cleared.
        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Charging,
            "impulse should be charging after StartImpulseCharge"
        );
        let last = app.world().resource::<LastHelmInput>();
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
        let yaw_before = app.world().resource::<ShipState>().yaw;
        push(&mut app, "helm", ClientMessage::CancelImpulse);
        tick(&mut app);
        let yaw_after = app.world().resource::<ShipState>().yaw;
        assert!(
            (yaw_after - yaw_before).abs() < 1e-3,
            "post-cancel tick must not autopilot a phantom turn; \
             yaw drifted by {}",
            yaw_after - yaw_before
        );
    }

    // ── Station Rating tests ─────────────────────────────────────────────

    #[test]
    fn set_station_rating_sets_ai_for_automated_systems() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        // Captain station config maps red-alert as assisted system.
        // The captain player holds Console::CaptainChair which maps to
        // the "captain" station in the default ShipConfig.
        push(
            &mut app,
            "captain",
            ClientMessage::SetStationRating {
                rating_name: "Assisted".into(),
            },
        );
        tick_twice(&mut app);

        let sources = app.world().resource::<ShipSystemControlSources>();
        assert_eq!(
            sources.0.source_for(&crate::system_registry::red_alert_system_id()),
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

        let sources = app.world().resource::<ShipSystemControlSources>();
        assert_eq!(
            sources.0.source_for(&crate::system_registry::red_alert_system_id()),
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

        let sources = app.world().resource::<ShipSystemControlSources>();
        assert_eq!(
            sources.0.source_for(&crate::system_registry::red_alert_system_id()),
            ControlSource::Ai
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

        let sources = app.world().resource::<ShipSystemControlSources>();
        // Default is Human
        assert_eq!(
            sources.0.source_for(&crate::system_registry::red_alert_system_id()),
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

        let active = app.world().resource::<ActiveStationRatings>();
        assert_eq!(
            active.0.get(&StationId("captain".into())).map(|s| s.as_str()),
            Some("Assisted")
        );
    }
}
