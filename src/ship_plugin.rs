use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::collections::HashMap;

use crate::control_source::{ControlSourceResolver, ControlTickPolicy};
use crate::entity_spawner::RegionEffectsSection;
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{
    AdmittedCommands, ClientMessage, CoordinationPayload, ModifierSlot, StationId,
    SystemControlPayload,
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
#[derive(Message, Clone, Debug)]
pub struct CoordinationEnqueue {
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

/// Physics and drive config resources bundled so `process_helm_inputs` stays
/// under Bevy's 16-parameter system-function limit.
#[derive(SystemParam)]
struct HelmDriveParams<'w> {
    ship_physics_config: Option<Res<'w, ShipPhysicsConfigResource>>,
    impulse: Res<'w, ShipImpulse>,
    impulse_config: Res<'w, ImpulseConfigResource>,
    boost: Res<'w, ShipBoost>,
    boost_config: Res<'w, BoostConfigResource>,
    bank_config: Res<'w, BankConfigResource>,
}

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::messages::AdmittedCommands>();
        app.insert_resource(HelmInputTimer(Timer::from_seconds(
            1.0 / 30.0,
            TimerMode::Repeating,
        )))
        .init_resource::<LastHelmInput>()
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
                player_ship_helm_ai
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(operate_helm_ai),
                process_helm_inputs
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(player_ship_helm_ai),
                detect_player_ship_objective_completion.in_set(crate::sim_sets::SimSet::Broadcast),
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
    admitted: Res<AdmittedCommands>,
    mut ship: ResMut<ShipState>,
    mut last_input: ResMut<LastHelmInput>,
    modifiers: Res<ShipModifiers>,
    drive: HelmDriveParams,
    mut prev_phase: Local<Option<crate::impulse::ImpulsePhase>>,
) {
    // Edge-detect Idle → Charging (or any → Charging) and zero out the
    // last cached helm input so a stale steering/thrust value can't
    // resurface the moment impulse cancels or the autopilot disengages.
    let current_phase = drive.impulse.0.phase;
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

    for cmd in admitted.for_target(crate::system_registry::HELM_SYSTEM_ID) {
        if let SystemControlPayload::HelmInput { thrust, steering } = &cmd.payload {
            last_input.thrust = *thrust;
            last_input.steering = *steering;
        }
    }
    let dt = timer.0.duration().as_secs_f32();
    let state = ShipPhysicsState {
        x: ship.x,
        z: ship.z,
        yaw: ship.yaw,
        forward_speed: ship.forward_speed,
    };
    let impulse_active = drive.impulse.0.is_active();
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
    let mut config = match drive.ship_physics_config.as_deref() {
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
        let mult = if drive.impulse_config.acceleration_multiplier > 0.0 {
            drive.impulse_config.acceleration_multiplier
        } else {
            crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER
        };
        config.acceleration *= mult;
    }
    // Boost drive: while engaged, multiply max speed and acceleration. Only
    // applies when the ship's TOML enabled the feature.
    if drive.boost_config.enabled && drive.boost.0.is_active() {
        config.max_speed *= drive.boost_config.multiplier;
        config.max_reverse_speed *= drive.boost_config.multiplier;
        config.acceleration *= drive.boost_config.multiplier;
        config.max_yaw_rate *= drive.boost_config.steering_multiplier;
    }
    let result = compute_physics(state, input, dt, &config);

    ship.x = result.x;
    ship.z = result.z;
    ship.yaw = result.yaw;
    ship.forward_speed = result.forward_speed;

    // Visual banking: lerp roll toward target based on steering
    let max_bank_rad = drive.bank_config.max_bank_deg.to_radians();
    let target_roll = if impulse_active {
        0.0
    } else {
        -input.steering * max_bank_rad
    };
    let lerp_factor = (drive.bank_config.bank_lerp_rate * dt).min(1.0);
    ship.roll = ship.roll + (target_roll - ship.roll) * lerp_factor;
}

fn helm_control_policy(sources: &ShipSystemControlSources) -> ControlTickPolicy {
    sources
        .0
        .policy_for(&crate::system_registry::helm_system_id())
}

/// Per-kind AI plugin for helm. Runs after the AI tick so `last_helm_intent`
/// is fresh.
///
/// Two paths:
/// - Player ship (no `AiControllerComponent`): writes `LastHelmInput` when Backfill.
/// - NPC ships (`AiControllerComponent` present): applies physics directly to their
///   `Transform`, keeping NPC motion fully per-entity and out of `server.rs`.
fn operate_helm_ai(
    time: Res<Time>,
    mut last_input: ResMut<LastHelmInput>,
    player_ships: Query<
        &ShipSystemControlSources,
        (
            With<Ship>,
            Without<crate::ai::server::AiControllerComponent>,
        ),
    >,
    mut npc_ships: Query<
        (
            &mut Transform,
            &mut crate::ai::server::AiControllerComponent,
            &ShipSystemControlSources,
            Option<&crate::entities::spawner::HelmConsoleSection>,
        ),
        With<crate::ai::server::AiControllerComponent>,
    >,
) {
    // Player ship Backfill path: when helm is AI-controlled but there is no
    // behaviour tree to generate intent, hold the ship at zero thrust/steering
    // so it decelerates to a stop rather than keeping the last human input.
    for sources in &player_ships {
        let policy = helm_control_policy(sources);
        if !policy.operate_ai {
            continue;
        }
        *last_input = LastHelmInput {
            thrust: 0.0,
            steering: 0.0,
        };
    }

    // NPC ship path: translate `last_helm_intent` into physics on the entity's
    // own Transform. `server.rs` only writes intent; no physics lives there.
    let dt = time.delta_secs();
    for (mut transform, mut ctrl, sources, helm_section) in &mut npc_ships {
        let policy = helm_control_policy(sources);
        if !policy.operate_ai {
            continue;
        }
        let (thrust, steering) = ctrl.last_helm_intent.unwrap_or((0.0, 0.0));
        let physics_config = helm_section
            .map(|hc| ShipPhysicsConfig {
                max_speed: hc.0.max_speed,
                max_reverse_speed: hc.0.max_reverse_speed,
                acceleration: hc.0.acceleration,
                deceleration: hc.0.deceleration,
                max_yaw_rate: hc.0.max_yaw_rate,
            })
            .unwrap_or_else(ShipPhysicsConfig::new);
        let pos = transform.translation;
        let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
        let result = compute_physics(
            ShipPhysicsState {
                x: pos.x,
                z: pos.z,
                yaw,
                forward_speed: ctrl.forward_speed,
            },
            ShipPhysicsInput { thrust, steering },
            dt,
            &physics_config,
        );
        transform.translation.x = result.x;
        transform.translation.z = result.z;
        let max_bank_rad = helm_section
            .map(|h| h.0.max_bank_deg.to_radians())
            .unwrap_or(0.0);
        let current_roll = transform.rotation.to_euler(EulerRot::YXZ).2;
        let target_roll = -steering * max_bank_rad;
        let lerp_factor = (5.0_f32 * dt).min(1.0);
        let new_roll = current_roll + (target_roll - current_roll) * lerp_factor;
        transform.rotation = Quat::from_euler(EulerRot::YXZ, result.yaw, 0.0, new_roll);
        ctrl.forward_speed = result.forward_speed;
    }
}

/// Drive the player ship toward its top-scoring Reach mission objective.
///
/// Runs after `operate_helm_ai` (which writes the Backfill zero-stop) and
/// before `process_helm_inputs` (which applies `LastHelmInput` to physics).
/// When a Reach objective is active, overwrites the Backfill stop with
/// real thrust/steering from `operate_helm`.
fn player_ship_helm_ai(
    ship: Res<ShipState>,
    mut last_input: ResMut<LastHelmInput>,
    mut memory: Option<ResMut<crate::server_app::PlayerAiMemory>>,
    blackboards: Option<Res<crate::server_app::SystemBlackboards>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    player_ships: Query<
        (
            &ShipSystemControlSources,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
        ),
        (
            With<Ship>,
            Without<crate::ai::server::AiControllerComponent>,
        ),
    >,
    entity_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::EntityName>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::EntityConsoleHull>,
            Option<&crate::entities::spawner::ColliderSection>,
        ),
        Without<Ship>,
    >,
) {
    let Ok((sources, player_uuid, player_faction, player_collider)) = player_ships.single() else {
        return;
    };
    if !helm_control_policy(sources).operate_ai {
        return;
    }

    let Some(blackboards) = blackboards else {
        return;
    };
    let scored: Vec<crate::messages::ScoredObjective> = match blackboards
        .0
        .get(&crate::system_registry::viewscreen_system_id())
    {
        Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
        _ => return,
    };

    let has_helm_objective = scored
        .iter()
        .any(|o| o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm));
    if !has_helm_objective {
        return;
    }

    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();
    let runtime = runtime.as_deref();
    let player_uuid = player_uuid.map(|u| u.0.as_str());
    let entities = entity_q
        .iter()
        .filter(|(uuid, _, _, _, _, _)| Some(uuid.0.as_str()) != player_uuid)
        .map(|(uuid, transform, name, faction, hull, collider)| {
            let runtime_name = runtime.and_then(|rt| {
                rt.name_to_uuid
                    .iter()
                    .find_map(|(name, mapped_uuid)| (mapped_uuid == &uuid.0).then(|| name.clone()))
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
        .collect();

    let world_view = crate::ai::WorldView {
        entity_pos: [ship.x, 0.0, ship.z],
        entity_yaw: ship.yaw,
        anchors: anchors.clone(),
        entities,
        self_faction: player_faction.map(|f| f.0),
        self_radius: player_collider.map(|c| c.0.radius).unwrap_or(0.0),
        ..crate::ai::WorldView::default()
    };

    let (thrust, steering) = if let Some(mem) = memory.as_mut() {
        crate::ai::operate_helm(
            &mut mem.0,
            &world_view,
            &scored,
            &[],
            &anchors,
            crate::ai::WAYPOINT_ARRIVAL_RADIUS,
            crate::ai::AVOIDANCE_BUFFER,
            crate::ai::AVOIDANCE_LOOK_AHEAD_SECS,
            ship.forward_speed,
            &crate::faction::FactionRegistry::default(),
        )
    } else {
        (0.0, 0.0)
    };

    *last_input = LastHelmInput { thrust, steering };
}

/// Mark Reach objectives complete once the player ship arrives within
/// `WAYPOINT_ARRIVAL_RADIUS` of the objective's anchor.
///
/// Runs in `Broadcast` (after `PublishAggregate` so `scored_objectives` is
/// fresh) and only when the helm system is AI-controlled.
fn detect_player_ship_objective_completion(
    ship: Res<ShipState>,
    blackboards: Option<Res<crate::server_app::SystemBlackboards>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    objectives: Option<ResMut<crate::world::server::ObjectiveManagerRes>>,
    player_ships: Query<
        &ShipSystemControlSources,
        (
            With<Ship>,
            Without<crate::ai::server::AiControllerComponent>,
        ),
    >,
) {
    let Ok(sources) = player_ships.single() else {
        return;
    };
    if !helm_control_policy(sources).operate_ai {
        return;
    }

    let Some(blackboards) = blackboards else {
        return;
    };
    let Some(mut objectives) = objectives else {
        return;
    };

    let scored: Vec<crate::messages::ScoredObjective> = match blackboards
        .0
        .get(&crate::system_registry::viewscreen_system_id())
    {
        Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
        _ => return,
    };

    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();

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
        let dx = target[0] - ship.x;
        let dz = target[2] - ship.z;
        if (dx * dx + dz * dz).sqrt() < crate::ai::WAYPOINT_ARRIVAL_RADIUS {
            objectives.0.complete(&obj.snapshot.id);
        }
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
    admitted: Res<AdmittedCommands>,
    mut impulse: ResMut<ShipImpulse>,
    hull: Res<ShipHullIntegrity>,
    mut last_hull_hp: Local<f32>,
    membership: Option<Res<RegionMembership>>,
    region_query: Query<&RegionEffectsSection>,
    ship_query: Query<Entity, With<Ship>>,
) {
    if *last_hull_hp == 0.0 && (hull.0.total_current() - hull.0.total_max()).abs() < 1e-6 {
        *last_hull_hp = hull.0.total_max();
    }

    let current_hp = hull.0.total_current();
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
    mut impulse: ResMut<ShipImpulse>,
    config: Res<ImpulseConfigResource>,
) {
    impulse.0.tick(time.delta_secs(), config.charge_duration);
}

/// Toggle the boost drive in response to Helm boost controls. No-op when
/// the feature is disabled.
pub fn handle_boost_messages(
    admitted: Res<AdmittedCommands>,
    mut boost: ResMut<ShipBoost>,
    config: Res<BoostConfigResource>,
) {
    if !config.enabled {
        return;
    }
    for cmd in admitted.for_target(crate::system_registry::HELM_SYSTEM_ID) {
        match &cmd.payload {
            SystemControlPayload::ToggleBoost => {
                boost.0.toggle();
            }
            SystemControlPayload::SetBoost { active } => {
                if *active {
                    boost.0.activate();
                } else {
                    boost.0.deactivate();
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
    ship_components: Query<(&ShipConfigComponent, &ShipSystemControlSources), With<Ship>>,
) {
    let Ok((ship_config, control_sources)) = ship_components.single() else {
        return;
    };
    if !config.enabled {
        return;
    }
    let policy = helm_control_policy(&control_sources);
    let has_helm = sessions
        .0
        .console_holder(&crate::messages::Console::Helm, &ship_config.0)
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
    mut ship_components: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<Ship>,
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
    mut ship_components: Query<(&ShipConfigComponent, &mut CoordinationQueue), With<Ship>>,
    mut events: MessageReader<CoordinationEnqueue>,
    mut inbound: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    let coord_events: Vec<_> = events.read().collect();
    let inbound_msgs: Vec<_> = inbound.read().collect();
    for (ship_config, mut queue) in ship_components.iter_mut() {
        let lag = ship_config.0.coordination_lag_secs;
        for ev in coord_events.iter() {
            queue.0.enqueue(QueuedCoordination {
                sender_origin: ev.sender_origin,
                target: ev.target.clone(),
                payload: ev.payload.clone(),
                sender_label: ev.sender_label.clone(),
                due_time: now + lag,
            });
        }
        for msg in inbound_msgs.iter() {
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
}

pub fn process_coordination_lag(
    time: Res<Time>,
    mut ship_components: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &mut CoordinationQueue,
        ),
        With<Ship>,
    >,
    sessions: Res<Sessions>,
    mut outbox: ResMut<crate::lobby::LobbyOutbox>,
) {
    let now = time.elapsed_secs();
    for (ship_config, control_sources, mut queue) in ship_components.iter_mut() {
        let due = queue.0.due_messages(now);
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
    use crate::ship_state::ShipState;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
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
        app.world_mut().spawn((
            Ship,
            Transform::default(),
            ShipConfigComponent::default(),
            ShipSystemControlSources::default(),
            ActiveStationRatings::default(),
            CoordinationQueue::default(),
        ));
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
                thrust: 0.0,
                steering: 0.0
            }
        );
        assert_eq!(app.world().resource::<ShipState>().forward_speed, 0.0);
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
            *app.world().resource::<LastHelmInput>(),
            LastHelmInput::default()
        );
    }

    #[test]
    fn human_helm_suppresses_ai_operate() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        tick(&mut app);

        assert_eq!(
            *app.world().resource::<LastHelmInput>(),
            LastHelmInput::default()
        );
        assert_eq!(app.world().resource::<ShipState>().forward_speed, 0.0);
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
            app.world().resource::<ShipImpulse>().0.phase,
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
            app.world().resource::<ShipImpulse>().0.phase,
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
            app.world().resource::<ShipImpulse>().0.phase,
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 0.0,
                    steering: 1.0,
                },
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 1.0,
                    steering: 0.0,
                },
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 1.0,
                    steering: 0.0,
                },
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 1.0,
                    steering: 0.0,
                },
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 0.0,
                    steering: 1.0,
                },
            },
        );
        tick(&mut app);
        let boosted_yaw = app.world().resource::<ShipState>().yaw;

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
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::CancelImpulse,
            },
        );
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

    fn viewscreen_with_objectives(
        objectives: Vec<crate::messages::ScoredObjective>,
    ) -> crate::server_app::SystemBlackboards {
        use crate::messages::{SystemBlackboard, ViewscreenBlackboard};
        use crate::server_app::SystemBlackboards;
        let mut bbs = SystemBlackboards::default();
        let mut vb = ViewscreenBlackboard::default();
        vb.scored_objectives = objectives;
        bbs.0.insert(
            crate::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(vb),
        );
        bbs
    }

    fn viewscreen_with_reach(anchor: &str, score: f32) -> crate::server_app::SystemBlackboards {
        viewscreen_with_objectives(vec![reach_scored_objective(anchor, score)])
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
        app.insert_resource(viewscreen_with_reach(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        app.insert_resource(crate::server_app::PlayerAiMemory::default());
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = *app.world().resource::<LastHelmInput>();
        assert!(
            last.thrust > 0.0,
            "AI helm must apply positive thrust toward Reach anchor; got {last:?}"
        );
    }

    #[test]
    fn player_ship_helm_ai_patrols_from_viewscreen_objective() {
        let mut app = test_app();
        let anchor = "starbase_patrol_east";
        app.insert_resource(viewscreen_with_objectives(vec![patrol_scored_objective(
            vec![anchor],
            20.0,
        )]));
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        app.insert_resource(crate::server_app::PlayerAiMemory::default());
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = *app.world().resource::<LastHelmInput>();
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
        app.insert_resource(viewscreen_with_objectives(vec![destroy_scored_objective(
            "wave_1", 80.0,
        )]));
        app.insert_resource(crate::server_app::PlayerAiMemory::default());
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = *app.world().resource::<LastHelmInput>();
        assert!(
            last.thrust > 0.0,
            "AI helm must pursue named Destroy objective target; got {last:?}"
        );
    }

    #[test]
    fn player_ship_helm_ai_does_nothing_when_helm_human() {
        let mut app = test_app();
        let anchor = "station-alpha";
        app.insert_resource(viewscreen_with_reach(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        app.insert_resource(crate::server_app::PlayerAiMemory::default());
        // helm stays Human (default)

        tick(&mut app);

        let last = *app.world().resource::<LastHelmInput>();
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
            SystemAffinity, SystemBlackboard, ViewscreenBlackboard,
        };
        use crate::server_app::SystemBlackboards;
        let mut bbs = SystemBlackboards::default();
        let mut vb = ViewscreenBlackboard::default();
        vb.scored_objectives = vec![ScoredObjective {
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
        }];
        bbs.0.insert(
            crate::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(vb),
        );
        app.insert_resource(bbs);
        app.insert_resource(crate::server_app::PlayerAiMemory::default());
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        // `operate_helm_ai` wrote zero; unresolved Destroy target leaves it there.
        let last = *app.world().resource::<LastHelmInput>();
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
        app.insert_resource(viewscreen_with_reach(anchor, 8.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
        app.insert_resource(crate::server_app::PlayerAiMemory::default());
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
        app.insert_resource(viewscreen_with_reach(anchor, 8.0));
        app.insert_resource(world_config_with_anchor(anchor, [500.0, 0.0, 0.0]));
        app.insert_resource(crate::server_app::PlayerAiMemory::default());
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
        app.insert_resource(viewscreen_with_reach(anchor, 8.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
        app.insert_resource(crate::server_app::PlayerAiMemory::default());
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
}
