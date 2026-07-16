use bevy::prelude::*;
use std::collections::HashMap;

use crate::console_bridge::AiChatterEvent;
use crate::control_source::{ControlSourceResolver, ControlTickPolicy};
use crate::damage::DamageTier;
use crate::entity_spawner::RegionEffectsSection;
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{
    AdmittedCommands, ClientMessage, CoordinationPayload, InterSystemMsg, InterSystemPayload,
    InterSystemQueue, ModifierSlot, StationId, SystemControlPayload, SystemId,
};
use crate::modifiers::ShipModifiers;
use crate::region_effects::RegionEffectKind;
use crate::region_plugin::RegionMembership;
use crate::server_app::{LocalShip, Ship};
use crate::ship::config::ShipConfig;
use crate::ship::control_source::ControlSource;
use crate::ship::coordination;
use crate::ship::coordination::{CoordinationLagQueue, QueuedCoordination};
use crate::ship::helm::{
    BoostCommand, ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput,
};
use crate::ship::rating;
use crate::ship_physics::{compute_physics, ShipPhysicsConfig, ShipPhysicsInput, ShipPhysicsState};
use crate::ship_state::ShipPhysics;
use crate::simulation::{ShipBoost, ShipImpulse};

// Ã¢â€â‚¬Ã¢â€â‚¬ Resources Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

#[derive(Resource)]
struct HelmInputTimer(Timer);

const HELM_AI_MAX_DT_SECS: f32 = 1.0 / 30.0;

#[derive(Resource)]
struct AiLateralThrustTimer(Timer);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct LastHelmInput {
    pub thrust: f32,
    pub steering: f32,
    pub lateral: f32,
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

/// Pending Weapons->Helm arc-bearing request, delivered via the channel-3
/// coordination bus (issue #677). Set by `process_coordination_lag` when a
/// `CoordinationPayload::ArcBearingRequest` is consumed by an AI-controlled
/// Helm; read (and cleared once the requested entity is no longer visible)
/// by `operate_helm_ai` to bias steering toward the requested bearing.
#[derive(Component, Clone, Debug, Default)]
pub struct PendingArcBearingRequest(pub Option<uuid::Uuid>);

/// Tracks the last-seen `DamageTier` per system per ship for detecting
/// crossings to worse tiers (issue #682). Initialised during ship spawn;
/// each tick of the tier-crossing detector updates entries for every system.
#[derive(Component, Clone, Debug, Default)]
pub struct LastSystemTiers(pub std::collections::HashMap<SystemId, DamageTier>);

/// Tracks which stations have already been flagged for human repair popups
/// (issue #682). Key: station_id. Value: the worst tier already alerted for.
/// Prevents re-popup every tick for Operational->Damaged crossings.
#[derive(Component, Clone, Debug, Default)]
pub struct RepairHumanAlerted(pub std::collections::HashMap<String, DamageTier>);

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

/// Load `ShipConfigComponent` from `assets/entities/alliance_battleship.toml` (embedded at compile time).
///
/// Panics if the file fails validation — the server cannot start without a valid ship
/// configuration.
pub(crate) fn load_ship_config_from_disk() -> ShipConfigComponent {
    let toml_str = include_str!("../assets/entities/alliance_battleship.toml");
    let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
        .expect("core system registry must be valid");
    let kinds: Vec<&str> = registry.kinds().collect();
    match crate::ship::config::parse_and_validate(toml_str, &kinds) {
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
/// Per-entity `Component` on each ship (issue #606: component is the sole
/// source of truth; no Resource fallback).
#[derive(Component, Clone)]
pub struct ImpulseConfigResource {
    pub charge_duration: f32,
    pub speed_multiplier: f32,
    pub acceleration_multiplier: f32,
    pub engage_distance: f32,
    pub cancel_distance: f32,
}

impl Default for ImpulseConfigResource {
    fn default() -> Self {
        Self {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER,
            engage_distance: 200.0,
            cancel_distance: 40.0,
        }
    }
}

/// Runtime boost drive config, loaded from `[helm_console.boost]` in the entity
/// TOML. `enabled` is false (the default) when the TOML omits the table, which
/// disables the feature entirely.
/// Per-entity `Component` on each ship (issue #606: component is the sole
/// source of truth; no Resource fallback).
#[derive(Component, Clone)]
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

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmInputTimer(Timer::from_seconds(
            1.0 / 30.0,
            TimerMode::Repeating,
        )))
        .init_resource::<BankConfigResource>()
        .add_message::<CoordinationEnqueue>()
        .add_message::<AiChatterEvent>()
        .insert_resource(AiLateralThrustTimer(Timer::from_seconds(
            1.0 / 30.0,
            TimerMode::Repeating,
        )))
        .add_systems(
            Update,
            (
                operate_helm_ai
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel),
                operate_lateral_thrust_ai
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(operate_helm_ai)
                    .before(process_helm_inputs),
                // Applies commanded impulse/boost phase transitions (issue
                // #695). Must run before `process_helm_inputs` — whose
                // stale-input edge-detection reads `ShipImpulse.phase` and
                // needs to see this tick's transition, not last tick's —
                // and before `tick_impulse`/`tick_boost` so a
                // freshly-started charge/engagement begins progressing the
                // same tick it was commanded, mirroring the old fused
                // `process_helm_inputs`/`handle_impulse_messages` ordering.
                apply_helm_commands
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(operate_helm_ai)
                    .after(handle_impulse_messages)
                    .after(handle_boost_messages)
                    .before(process_helm_inputs)
                    .before(tick_impulse)
                    .before(tick_boost),
                process_helm_inputs
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(operate_helm_ai)
                    .after(tick_impulse),
                publish_joystick_to_engines
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(process_helm_inputs),
                operate_helm_engine_ai
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(operate_helm_ai),
                detect_reached_objective_completion.in_set(crate::sim_sets::SimSet::Broadcast),
                tick_impulse.in_set(crate::sim_sets::SimSet::Physics),
                tick_boost.in_set(crate::sim_sets::SimSet::Physics),
                handle_impulse_messages.in_set(crate::sim_sets::SimSet::Input),
                handle_boost_messages.in_set(crate::sim_sets::SimSet::Input),
                // Sole helm-path writer of ShipPhysics (issues #695, #699):
                // reads the intent components written by
                // `process_helm_inputs`
                // (human/admission) and `operate_helm_ai` (AI decision),
                // plus the post-transition `ShipImpulse`/`ShipBoost` state
                // applied by `apply_helm_commands`, then performs the
                // actual physics integration for whichever ship (LocalShip
                // or promoted NPC) has fresh values this tick. Ordered
                // after every writer of those intents and after
                // `tick_impulse` so it reads this tick's freshly-ticked
                // impulse phase, mirroring the old fused
                // `process_helm_inputs` ordering.
                integrate_ship_physics
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(operate_helm_ai)
                    .after(process_helm_inputs)
                    .after(apply_helm_commands)
                    .after(tick_impulse)
                    .after(tick_boost),
                sync_ship_position
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(process_helm_inputs)
                    .after(operate_helm_ai)
                    .after(integrate_ship_physics),
                handle_station_rating_change.in_set(crate::sim_sets::SimSet::Input),
                handle_coordination_enqueue.in_set(crate::sim_sets::SimSet::Input),
                handle_coordination_messages.in_set(crate::sim_sets::SimSet::Input),
                process_coordination_lag.in_set(crate::sim_sets::SimSet::Modifiers),
                sync_console_damage_tiers.in_set(crate::sim_sets::SimSet::Damage),
                detect_damage_tier_crossings.in_set(crate::sim_sets::SimSet::Damage),
            )
                .after(crate::lobby::process_lobby),
        );

        // Per-axis helm AI (issue #701). Registered separately from the tuple
        // above purely because that tuple is at Bevy's 20-element limit.
        //
        // `.after(operate_helm_ai)` is load-bearing since #800. These two now
        // gate on their own axis alone and run for the same ship the monolith
        // does, so the order fixes (a) who commits `AiMemory` — the last system
        // to run owns it — and (b) that both read pre-commit memory and so reach
        // the same decision the monolith did. Per-component write exclusion is
        // enforced by the monolith's stand-down, not by this order; see the
        // module note on `ai_helm_thrust`.
        //
        // Ordered AFTER `process_helm_inputs`, unlike
        // `operate_lateral_thrust_ai`. Lateral thrust has its own
        // `LateralThrustInput` payload, so making its fine system AI-operated
        // sets `accept_human_input = false` and no lateral command is ever
        // admitted — nothing for `process_helm_inputs` to clobber it with.
        // Thrust and steering instead share the coarse `helm` `HelmInput`
        // payload, which stays admissible while a human holds the coarse helm
        // (exactly the case these systems run in). Running last makes the
        // per-axis AI the authoritative writer of the axis it owns,
        // deterministically rather than by set-order luck.
        //
        // `ai_helm_thrust` runs before `ai_helm_steering` so that the commit
        // rule in the module note below ("the last helm-AI system that actually
        // runs owns the `AiMemory` commit") has a well-defined "last".
        //
        // Both systems also write `LastHelmInput.{thrust,steering}` for the
        // player ship, one field each. Every reader of that *pair* in
        // `SimSet::Physics` must therefore be ordered after BOTH of them, or
        // it can observe a torn pair — this tick's AI throttle next to last
        // tick's stale human steering. Since #800 the monolith writes
        // `LastHelmInput` field-wise too (it stands down per axis rather than
        // per ship), so no single writer covers the pair on its own; what
        // rules out a tear is that the union of the writers covers every field
        // before any pair-reader runs. The pair readers are
        // `publish_joystick_to_engines`, `operate_helm_engine_ai` and
        // `tick_boost`; `helm_ai_last_input_pair_is_not_torn` pins the result.
        //
        // (`operate_lateral_thrust_ai` also writes `LastHelmInput`, but only
        // the disjoint `.lateral` field, and it is already
        // `.before(process_helm_inputs)` — hence already before these two.
        // `ai_power_allocation` reads `.thrust` alone, so it cannot see a torn
        // pair; it is left ordered exactly as it is against the monolith.)
        app.add_systems(
            Update,
            (ai_helm_thrust.before(ai_helm_steering), ai_helm_steering)
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(crate::lobby::process_lobby)
                .after(operate_helm_ai)
                .after(process_helm_inputs)
                .before(publish_joystick_to_engines)
                .before(operate_helm_engine_ai)
                .before(tick_boost)
                .before(integrate_ship_physics),
        );

        // Debug-only helm-path single-writer tripwire (issue #699). The frame
        // stamp is bumped once in `First` so every writer in a given frame
        // observes the same value; `integrate_ship_physics` stamps each ship
        // it integrates and panics if that ship was already stamped this
        // frame. Compiled out entirely in release builds.
        #[cfg(debug_assertions)]
        app.init_resource::<crate::ship::helm::HelmPhysicsFrame>()
            .add_systems(First, crate::ship::helm::tick_helm_physics_frame);
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Systems Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// Human-admission path (issue #695): turns `AdmittedCommands` into
/// `LastHelmInput` (kept for broadcast/back-compat consumers) and the
/// shared `ThrustInput`/`SteeringInput`/`LateralThrustInput` intent
/// components. Physics integration itself now lives in
/// `integrate_ship_physics`, which reads those intent components for both
/// the player ship and any AI-promoted NPC.
fn process_helm_inputs(
    time: Res<Time>,
    mut timer: ResMut<HelmInputTimer>,
    ship_query: Query<(&AdmittedCommands, &ShipSystemControlSources), With<LocalShip>>,
    mut last_input_q: Query<&mut LastHelmInput, With<LocalShip>>,
    mut intent_q: Query<
        (
            Option<&mut ThrustInput>,
            Option<&mut SteeringInput>,
            Option<&mut LateralThrustInput>,
        ),
        With<LocalShip>,
    >,
    impulse_q: Query<&ShipImpulse, With<LocalShip>>,
    mut prev_phase: Local<Option<crate::impulse::ImpulsePhase>>,
) {
    let Some(mut last_input) = last_input_q.iter_mut().next() else {
        return;
    };
    // Edge-detect Idle → Charging (or any → Charging) and zero out the
    // last cached helm input so a stale steering/thrust value can't
    // resurface the moment impulse cancels or the autopilot disengages.
    let current_phase = impulse_q
        .iter()
        .next()
        .map(|i| i.0.phase)
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

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    // When helm is under AI control (Backfill), `operate_helm_ai` is the
    // authoritative writer of the intent components this tick. Skip
    // admission entirely so a stale/human-admitted value can't clobber the
    // AI's decision — mirrors the old physics-skip semantics, just for the
    // admission decision now rather than the physics integration itself.
    if helm_control_policy(sources).operate_ai {
        return;
    }

    for cmd in admitted.for_target(crate::system_registry::HELM_SYSTEM_ID) {
        if let SystemControlPayload::HelmInput { thrust, steering } = &cmd.payload {
            last_input.thrust = *thrust;
            last_input.steering = *steering;
        }
    }

    for cmd in admitted.for_target(&crate::system_registry::lateral_thrust_system_id().0) {
        if let SystemControlPayload::LateralThrustInput { lateral } = &cmd.payload {
            last_input.lateral = *lateral;
        }
    }

    if let Some((thrust_in, steering_in, lateral_in)) = intent_q.iter_mut().next() {
        if let Some(mut t) = thrust_in {
            t.0 = last_input.thrust;
        }
        if let Some(mut s) = steering_in {
            s.0 = last_input.steering;
        }
        if let Some(mut l) = lateral_in {
            l.0 = last_input.lateral;
        }
    }
}

fn helm_control_policy(sources: &ShipSystemControlSources) -> ControlTickPolicy {
    sources
        .0
        .policy_for(&crate::system_registry::helm_system_id())
}

// ── Shared helm-AI decision inputs (issue #701) ───────────────────────────────
//
// `operate_helm_ai` (coarse) and the per-axis `ai_helm_thrust` /
// `ai_helm_steering` all need the same three inputs: the world entity list,
// the entity's scored objectives, and a `WorldView`. These helpers are the
// single implementation of each, so the per-axis systems cannot silently
// drift from the monolith they replace in #704.

/// The read-only entity query the helm AI falls back to when `WorldSnapshot`
/// is absent (tests that don't register `AiPlugin`).
type HelmAiFallbackQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static crate::entity_spawner::EntityUuid,
        &'static Transform,
        Option<&'static crate::entities::spawner::EntityName>,
        Option<&'static crate::entities::spawner::FactionComponent>,
        Option<&'static crate::entities::spawner::EntitySystemHull>,
        Option<&'static crate::entities::spawner::ColliderSection>,
    ),
>;

/// Snapshot every world entity for avoidance / target resolution.
///
/// Uses `WorldSnapshot` when available (production); falls back to a direct
/// ECS query for tests that don't register `AiPlugin`.
fn helm_ai_snapshot_entities(
    world_snapshot: Option<&crate::ai::server::WorldSnapshot>,
    runtime_ref: Option<&crate::world::server::WorldContentRuntime>,
    entity_fallback_q: &HelmAiFallbackQuery,
) -> Vec<crate::ai::AiWorldEntity> {
    if let Some(ws) = world_snapshot {
        return ws.entities.clone();
    }
    entity_fallback_q
        .iter()
        .map(|(uuid, transform, name, faction, hull, collider)| {
            let runtime_name = runtime_ref.and_then(|rt| {
                rt.name_to_uuid
                    .iter()
                    .find_map(|(n, mapped)| (mapped == &uuid.0).then(|| n.clone()))
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
}

/// Read this entity's scored objectives out of its viewscreen blackboard.
fn helm_ai_scored_objectives(
    blackboards: &crate::server_app::ShipSystemBlackboards,
) -> Vec<crate::messages::ScoredObjective> {
    match blackboards
        .0
        .get(&crate::system_registry::viewscreen_system_id())
    {
        Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
        _ => vec![],
    }
}

/// True when any scored objective is live and Helm-relevant. When false the
/// helm AI has nothing to pursue and zeroes its intent.
fn has_helm_objective(scored: &[crate::messages::ScoredObjective]) -> bool {
    scored
        .iter()
        .any(|o| o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm))
}

/// This ship's damage-scaled helm radar range (issue #674).
///
/// Prefers the live value from the ship's own Helm blackboard entry; falls
/// back to static config for NPC ships (no Helm blackboard entry) and for the
/// player before the blackboard is first published.
fn helm_ai_radar_range(
    blackboards: &crate::server_app::ShipSystemBlackboards,
    helm_section: Option<&crate::entities::spawner::HelmConsoleSection>,
    ship_client_config: Option<&crate::lobby::server::ShipClientConfigResource>,
    is_local: bool,
) -> f32 {
    let from_blackboard = match blackboards.0.get(&crate::system_registry::helm_system_id()) {
        Some(crate::messages::SystemBlackboard::Helm(bb)) if bb.radar_range > 0.0 => {
            Some(bb.radar_range)
        }
        _ => None,
    };
    from_blackboard.unwrap_or_else(|| {
        if is_local {
            ship_client_config
                .map(|c| c.0.helm_radar_range)
                .unwrap_or(0.0)
        } else {
            helm_section
                .map(|hc| hc.0.effective_radar_range())
                .unwrap_or(0.0)
        }
    })
}

/// Build the `WorldView` the helm AI reasons over: every snapshot entity
/// except self, gated by this ship's damage-scaled radar range.
#[allow(clippy::too_many_arguments)]
fn helm_ai_world_view(
    physics: &ShipPhysics,
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    faction: Option<&crate::entities::spawner::FactionComponent>,
    collider: Option<&crate::entities::spawner::ColliderSection>,
    helm_section: Option<&crate::entities::spawner::HelmConsoleSection>,
    blackboards: &crate::server_app::ShipSystemBlackboards,
    ship_client_config: Option<&crate::lobby::server::ShipClientConfigResource>,
    is_local: bool,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    snapshot_entities: &[crate::ai::AiWorldEntity],
) -> crate::ai::WorldView {
    let self_uuid_str = entity_uuid.map(|u| u.0.as_str()).unwrap_or("");
    let self_filtered: Vec<crate::ai::AiWorldEntity> = snapshot_entities
        .iter()
        .filter(|e| e.uuid.to_string() != self_uuid_str)
        .cloned()
        .collect();

    let radar_range = helm_ai_radar_range(blackboards, helm_section, ship_client_config, is_local);
    let entity_pos = [physics.x, 0.0, physics.z];
    let entities = crate::ai::visible_entities(entity_pos, radar_range, &self_filtered);

    crate::ai::WorldView {
        entity_pos,
        entity_yaw: physics.yaw,
        anchors: anchors.clone(),
        entities,
        self_faction: faction.map(|f| f.0),
        self_radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
        ..crate::ai::WorldView::default()
    }
}

/// Call the pure `crate::ai::operate_helm` with this ship's TOML-authored
/// behaviour tuning, returning `(thrust, steering)`.
///
/// Both per-axis systems call this and keep only their own axis (see the
/// module note on `ai_helm_thrust`). Every tunable it passes down — arrival
/// radius, avoidance buffer, avoidance look-ahead, nav-handoff speed — comes
/// from the entity's `[behaviour]` TOML section. The `crate::ai::*` constants
/// below appear only as `unwrap_or` fallbacks for an entity that has no
/// `[behaviour]` section at all; every one of them is the same value the
/// matching serde `default =` fn supplies, so an entity that omits the field
/// and an entity that omits the whole section behave identically.
fn helm_ai_decision(
    memory: &mut crate::ai::AiMemory,
    world_view: &crate::ai::WorldView,
    scored: &[crate::messages::ScoredObjective],
    behaviour_section: Option<&crate::entities::spawner::BehaviourSection>,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    forward_speed: f32,
    faction_registry: Option<&crate::entities::config_cache::FactionRegistryResource>,
) -> (f32, f32) {
    crate::ai::operate_helm(
        memory,
        world_view,
        scored,
        behaviour_section
            .map(|b| b.0.doctrine.as_slice())
            .unwrap_or(&[]),
        anchors,
        // Authored per entity template in TOML (`[behaviour]
        // waypoint_arrival_radius`), same as the cursor evaluator reads —
        // the helm's turn-at-waypoint decision must not disagree with the
        // arrival that fires the scenario trigger.
        behaviour_section
            .map(|b| b.0.waypoint_arrival_radius)
            .unwrap_or(crate::ai::WAYPOINT_ARRIVAL_RADIUS),
        behaviour_section
            .map(|b| b.0.avoidance_buffer)
            .unwrap_or(crate::ai::AVOIDANCE_BUFFER),
        behaviour_section
            .map(|b| b.0.avoidance_look_ahead_secs)
            .unwrap_or(crate::ai::AVOIDANCE_LOOK_AHEAD_SECS),
        forward_speed,
        faction_registry
            .map(|r| &r.0)
            .unwrap_or(&crate::faction::FactionRegistry::default()),
        behaviour_section
            .map(|b| b.0.nav_handoff_speed)
            .unwrap_or(crate::ai::NAV_HANDOFF_SPEED),
    )
}

/// Apply the Weapons→Helm arc-bearing request (issue #677) to `steering`.
///
/// Biases steering to face the requested target so the phaser firing arc can
/// bear on it, without disturbing the thrust/range-holding decision
/// `operate_helm` already made. Cleared once the requested entity is no
/// longer visible (destroyed or out of radar range), OR once the ship's
/// current facing already brings some bank's arc onto the target — the same
/// `in_arc` check Weapons uses to decide whether to ask at all — so the bias
/// never persists after the request has been satisfied or outlives the
/// situation that created it.
fn apply_arc_bearing_request(
    steering: &mut f32,
    pending: Option<&mut PendingArcBearingRequest>,
    world_view: &crate::ai::WorldView,
    physics: &ShipPhysics,
    combat_config: Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
) {
    let Some(pending) = pending else { return };
    let Some(bearing_uuid) = pending.0 else {
        return;
    };
    match world_view.entities.iter().find(|e| e.uuid == bearing_uuid) {
        Some(target_entity) => {
            let arc_satisfied = combat_config.is_some_and(|cfg| {
                cfg.0.banks.iter().any(|b| {
                    let (rx, ry) = crate::weapons::phaser::ship_local(
                        target_entity.position[0],
                        target_entity.position[2],
                        physics.x,
                        physics.z,
                        physics.yaw,
                    );
                    crate::weapons::phaser::in_arc(rx, ry, b.facing_deg, b.auto_arc_deg)
                })
            });

            if arc_satisfied {
                pending.0 = None;
            } else {
                let dx = target_entity.position[0] - world_view.entity_pos[0];
                let dz = target_entity.position[2] - world_view.entity_pos[2];
                let dist = (dx * dx + dz * dz).sqrt();
                if dist > 1.0 {
                    *steering = crate::ai::steer_toward(
                        physics.yaw,
                        [dx / dist, dz / dist],
                        crate::ai::PATROL_DEADBAND_RAD,
                        crate::ai::PATROL_FULL_STEER_RAD,
                    );
                }
            }
        }
        None => pending.0 = None,
    }
}

/// Unified per-entity helm AI. Runs after the AI tick so `last_helm_intent`
/// is fresh for NPC ships.
///
/// For every ship entity where the helm system is `ControlSource::Ai`:
///  - Reads `ShipSystemBlackboards` viewscreen entry for scored objectives.
///  - Builds a `WorldView` from `WorldSnapshot` (all other entities for avoidance),
///    falling back to a direct ECS query when `WorldSnapshot` is absent (tests).
///  - Calls `operate_helm(memory, ...)` and writes the result to the shared
///    `ThrustInput`/`SteeringInput`/`LateralThrustInput` intent components —
///    the same ones the admitted human path writes. It takes `&ShipPhysics`
///    **read-only** and never advances physics itself; `integrate_ship_physics`
///    is the sole helm-path writer (issues #695, #699).
///  - For `LocalShip` entities: also writes to `LastHelmInput` so
///    broadcast/back-compat consumers see the AI's decision.
///  - For NPC ships with `AiControllerComponent`: also updates `last_helm_intent`
///    for backward compatibility until `tick_ai_controllers` is retired (#595).
///
/// This replaces both the old player-ship-only `helm_ai` and the NPC
/// helm path in `tick_ai_controllers`.
#[allow(clippy::too_many_arguments)]
fn operate_helm_ai(
    mut local_ship_input: Query<&mut LastHelmInput, With<LocalShip>>,
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    entity_fallback_q: HelmAiFallbackQuery,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            &mut crate::ai_plugin::ShipAiMemory,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Has<crate::server_app::LocalShip>,
            Option<&ShipImpulse>,
            Option<&ImpulseConfigResource>,
            Option<&mut PendingArcBearingRequest>,
            (
                Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
                &mut ThrustInput,
                &mut SteeringInput,
                &mut LateralThrustInput,
                &mut ImpulseCommand,
            ),
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();
    let runtime_ref = runtime.as_deref();

    // Snapshot world entities for avoidance (read-only pass before mutating ships).
    let snapshot_entities =
        helm_ai_snapshot_entities(world_snapshot.as_deref(), runtime_ref, &entity_fallback_q);

    for (
        _entity,
        sources,
        physics,
        mut ai_memory,
        blackboards,
        entity_uuid,
        faction,
        collider,
        helm_section,
        behaviour_section,
        is_local,
        impulse_comp,
        impulse_cfg,
        mut pending_bearing,
        (combat_config_opt, mut thrust_in, mut steering_in, mut lateral_in, mut impulse_cmd),
    ) in ships.iter_mut()
    {
        let policy = helm_control_policy(sources);
        if !policy.operate_ai {
            continue;
        }

        // ── Per-axis stand-down (issue #800) ─────────────────────────────────
        // Now that `helm-thrust` / `helm-steering` are declared in shipped
        // TOMLs, `ai_helm_thrust` / `ai_helm_steering` gate on their own axis
        // alone and run for the same ship this system does. Each axis has
        // exactly one writer per tick: the axis's own system when it is
        // AI-operated, this one otherwise. Lateral thrust and impulse have no
        // per-axis successor yet, so this system keeps owning them outright —
        // standing the whole ship down would silently kill obstacle avoidance
        // and AI impulse on every shipped hull.
        let thrust_is_ai = sources
            .0
            .policy_for(&crate::system_registry::helm_thrust_system_id())
            .operate_ai;
        let steering_is_ai = sources
            .0
            .policy_for(&crate::system_registry::helm_steering_system_id())
            .operate_ai;

        // ── Read scored objectives from this entity's viewscreen blackboard ──
        let scored = helm_ai_scored_objectives(blackboards);

        if !has_helm_objective(&scored) {
            // No objectives → zero out intent (decelerate to stop). Written
            // for every AiHighFidelity ship (not just the player) so the
            // shared `integrate_ship_physics` step decelerates it via the
            // normal physics curve instead of coasting on a stale intent.
            // The per-axis systems zero the axes they own (to the same 0.0)
            // in their own no-objective branch.
            if !thrust_is_ai {
                thrust_in.0 = 0.0;
            }
            if !steering_is_ai {
                steering_in.0 = 0.0;
            }
            lateral_in.0 = 0.0;
            if is_local {
                if let Some(mut li) = local_ship_input.iter_mut().next() {
                    if !thrust_is_ai {
                        li.thrust = 0.0;
                    }
                    if !steering_is_ai {
                        li.steering = 0.0;
                    }
                    li.lateral = 0.0;
                }
            }
            continue;
        }

        // ── Build WorldView (exclude self, gate by radar range) ───────────────
        let world_view = helm_ai_world_view(
            physics,
            entity_uuid,
            faction,
            collider,
            helm_section,
            blackboards,
            ship_client_config.as_deref(),
            is_local,
            &anchors,
            &snapshot_entities,
        );

        // AiMemory commit rule — see the module note on `ai_helm_thrust`. The
        // last helm-AI system to run for this ship owns the commit, and the
        // registered order is this system → `ai_helm_thrust` → `ai_helm_steering`.
        // So we commit only when neither per-axis system will run; otherwise we
        // work on a scratch clone and let the later one commit. All three read
        // the same pre-commit memory and `helm_ai_decision` is pure given it, so
        // they reach the same decision and the axes agree.
        let mut memory_scratch;
        let memory: &mut crate::ai::AiMemory = if thrust_is_ai || steering_is_ai {
            memory_scratch = ai_memory.0.clone();
            &mut memory_scratch
        } else {
            &mut ai_memory.0
        };
        let (thrust, mut steering) = helm_ai_decision(
            memory,
            &world_view,
            &scored,
            behaviour_section,
            &anchors,
            physics.forward_speed,
            faction_registry.as_deref(),
        );

        // ── Weapons->Helm arc-bearing request (issue #677) ────────────────────
        // Part of the steering axis: it biases `steering` and can clear the
        // pending request (`pending.0 = None`), so we skip it when
        // `ai_helm_steering` owns steering. Today this is defensive rather than
        // load-bearing — `ai_helm_steering` would compute the identical bias
        // from the identical inputs, and the request is only cleared in the arc
        // -satisfied / target-gone cases where neither of us applies a bias. But
        // `PendingArcBearingRequest` is a component like any other, and leaving
        // the monolith writing it for a ship whose steering axis it does not own
        // is precisely the latent second writer #800 exists to remove.
        if !steering_is_ai {
            apply_arc_bearing_request(
                &mut steering,
                pending_bearing.as_deref_mut(),
                &world_view,
                physics,
                combat_config_opt,
            );
        }

        // ── Compute lateral thrust for obstacle avoidance (AI only) ──────────
        // Same TOML-authored avoidance tuning `helm_ai_decision` passes to
        // `operate_helm`: the dodge and the yaw must agree about how much
        // clearance this hull wants, or the ship sidesteps an obstacle its
        // steering has already decided is not a threat.
        let lateral = crate::ai::operate_lateral_thrust(
            &world_view,
            &scored,
            behaviour_section
                .map(|b| b.0.avoidance_buffer)
                .unwrap_or(crate::ai::AVOIDANCE_BUFFER),
            behaviour_section
                .map(|b| b.0.avoidance_look_ahead_secs)
                .unwrap_or(crate::ai::AVOIDANCE_LOOK_AHEAD_SECS),
            physics.forward_speed,
        );

        // ── Write intent components ────────────────────────────────────────────
        // Physics integration itself now happens in the shared
        // `integrate_ship_physics` system, which reads these intent
        // components for both the player ship and any AI-promoted NPC.
        // Each axis is written by its own system when that system is
        // AI-operated (issue #800) and by this one otherwise — never both.
        if !thrust_is_ai {
            thrust_in.0 = thrust;
        }
        if !steering_is_ai {
            steering_in.0 = steering;
        }
        lateral_in.0 = lateral;

        // ── AI Impulse decision ──────────────────────────────────────────────
        if let (Some(impulse), Some(cfg)) = (impulse_comp, impulse_cfg) {
            // Reads post-decision memory (target selection happens in
            // `helm_ai_decision`), which is `memory` — the scratch clone when a
            // per-axis system owns the commit, the real thing otherwise. Either
            // way it holds this tick's selection.
            let target_pos = resolve_helm_target_position(&scored, &world_view, &anchors, memory);
            if let Some(tp) = target_pos {
                // Find the matching doctrine to check use_impulse flag.
                let top_obj = scored.iter().find(|o| {
                    o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm)
                });
                let use_impulse = top_obj
                    .and_then(|obj| {
                        behaviour_section.and_then(|b| b.0.doctrine.iter().find(|d| d.id == obj.id))
                    })
                    .map(|d| d.effective_use_impulse())
                    .unwrap_or(false);
                if use_impulse {
                    let decision = crate::ai::decide_impulse(&crate::ai::ImpulseDecisionInput {
                        pos: [physics.x, physics.z],
                        yaw: physics.yaw,
                        target_pos: tp,
                        phase: impulse.0.phase,
                        engage_distance: cfg.engage_distance,
                        cancel_distance: cfg.cancel_distance,
                        angle_tolerance: crate::ai::IMPULSE_ANGLE_TOLERANCE_RAD,
                    });
                    match decision {
                        crate::ai::ImpulseDecision::Engage => {
                            impulse_cmd.0 = crate::impulse::ImpulsePhase::Charging;
                        }
                        crate::ai::ImpulseDecision::Cancel => {
                            impulse_cmd.0 = crate::impulse::ImpulsePhase::Idle;
                        }
                        crate::ai::ImpulseDecision::NoChange => {}
                    }
                }
            }
        }

        // For the player ship: also write LastHelmInput so downstream
        // consumers (broadcast, fine-engine bookkeeping) see the AI-driven
        // intent. Field-wise rather than wholesale, so the axes this system
        // stood down from are left to their own system to mirror (issue #800).
        if is_local {
            if let Some(mut li) = local_ship_input.iter_mut().next() {
                if !thrust_is_ai {
                    li.thrust = thrust;
                }
                if !steering_is_ai {
                    li.steering = steering;
                }
                li.lateral = lateral;
            }
        }
    }
}

/// Resolve the target position from the highest-scored Helm objective.
fn resolve_helm_target_position(
    scored: &[crate::messages::ScoredObjective],
    world_view: &crate::ai::WorldView,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    memory: &crate::ai::AiMemory,
) -> Option<[f32; 3]> {
    use crate::messages::{AiDirective, SystemAffinity};
    let top = scored
        .iter()
        .find(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm))?;
    match &top.directive {
        AiDirective::Reach { anchor } => anchors.get(anchor.as_str()).copied(),
        AiDirective::Retreat { anchor } => Some(
            // Resolve the retreat anchor by name, falling back to the ship's
            // home/spawn position when the anchor is empty or unknown (the
            // synthetic hull-triggered Retreat carries an empty anchor). Mirrors
            // the Retreat arm in `operate_helm`.
            anchors
                .get(anchor.as_str())
                .copied()
                .unwrap_or(memory.home_position),
        ),
        AiDirective::Destroy { target } => {
            let uuid = uuid::Uuid::parse_str(target).ok()?;
            world_view
                .entities
                .iter()
                .find(|e| e.uuid == uuid)
                .map(|e| e.position)
        }
        AiDirective::Patrol {
            anchors: waypoints, ..
        } => {
            let idx = memory.waypoint_index;
            waypoints
                .get(idx)
                .and_then(|wp| anchors.get(wp.as_str()))
                .copied()
        }
        _ => None,
    }
}

/// Mark Reach objectives complete once any ship arrives within its
/// TOML-authored `[behaviour] waypoint_arrival_radius` of the objective's
/// anchor (falling back to `WAYPOINT_ARRIVAL_RADIUS` for ships without a
/// behaviour section).
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
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entities::spawner::BehaviourSection>,
        ),
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

    for (sources, physics, blackboards, behaviour_section) in ships.iter() {
        if !helm_control_policy(sources).operate_ai {
            continue;
        }

        let arrival_radius = behaviour_section
            .map(|b| b.0.waypoint_arrival_radius)
            .unwrap_or(crate::ai::WAYPOINT_ARRIVAL_RADIUS);

        let scored: Vec<crate::messages::ScoredObjective> = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
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
            if (dx * dx + dz * dz).sqrt() < arrival_radius {
                objectives.0.complete(&obj.snapshot.id);
            }
        }
    }
}

// ── Per-axis helm AI (issue #701) ─────────────────────────────────────────────
//
// `ai_helm_thrust` and `ai_helm_steering` are the per-axis successors to the
// `operate_helm_ai` monolith: one owns `ThrustInput`, the other owns
// `SteeringInput`. Each gates on its own axis alone:
//
//     if !<own axis>.operate_ai { continue; }
//
// Until #800 both also carried a coarse half (`if helm_control_policy(sources)
// .operate_ai { continue; }`) which made them mutually exclusive with the
// monolith *per ship*. That was only tenable while `helm-thrust` /
// `helm-steering` were declared in zero TOMLs: their policy defaulted to Human,
// so these two systems never fired in shipped content and the monolith did all
// the work. #800 declares both axes on all nine shipped hulls, which turns them
// on — and the coarse half off.
//
// **Mutual exclusion is now per component, not per ship.** All three systems run
// for the same ship in the same tick, but each intent component still has
// exactly one writer:
//
//   ThrustInput   ← `ai_helm_thrust` iff T, else `operate_helm_ai` iff C
//   SteeringInput ← `ai_helm_steering` iff S, else `operate_helm_ai` iff C
//
// (T/S/C = the helm-thrust / helm-steering / coarse-helm `operate_ai` policies.)
// `operate_helm_ai` skips the write — and, for steering, the whole arc-bearing
// step, which *consumes* the pending request — for any axis whose own system
// owns it. So Bevy's arbitrary intra-set ordering still never decides the
// outcome (the #697 failure mode); `helm_ai_writers_are_mutually_exclusive`
// pins this over every (C, T, S) combination.
//
// The monolith keeps `LateralThrustInput` and `ImpulseCommand` outright: they
// have no per-axis successor, and `operate_lateral_thrust_ai`'s own gate is
// `L && !C`, so standing the whole ship down would leave obstacle avoidance and
// AI impulse with *no* writer on every shipped hull. #704 deletes the monolith
// and must rehome those two rather than simply dropping them.
//
// Per the owner's ruling both systems call the pure `crate::ai::operate_helm`
// and each keeps only its own axis, duplicating the `WorldView` build per ship
// per tick — the same duplication `operate_lateral_thrust_ai` already accepts.
// There is deliberately no shared cached `HelmDecision` component; that would
// re-create the mini-monolith this split exists to remove.
//
// **AiMemory ownership**: `operate_helm` mutates `AiMemory` (patrol waypoint
// advance, Destroy target selection, nav_goal clearing). Committing that twice
// in one tick would double-advance the patrol waypoint on arrival; committing
// it zero times freezes the waypoint. (`ShipAiMemory.target` is no longer the
// weapons AI's bridge: `ai_torpedo_auto_fire` reads `WeaponsTarget` alone
// post-#698, and post-#703 `ai_target_selection` acquires the nearest hostile
// itself, leaving `ai_phaser_auto_fire`'s memory fallback vestigial.)
// So the rule is:
//
//     **The last helm-AI system that actually runs owns the commit.**
//
// The registered order is `operate_helm_ai` → `ai_helm_thrust` →
// `ai_helm_steering`, so concretely: `ai_helm_steering` always commits when it
// runs; `ai_helm_thrust` commits only when helm-steering is *not* AI-operated;
// `operate_helm_ai` commits only when *neither* axis is. Whoever does not
// commit works on a scratch clone and discards it. That keys off policies all
// three already read, so it needs no shared cache.
//
// Exactly-once over every (coarse C, thrust T, steering S) combination, where
// each system's body now runs iff its own axis is AI (the coarse half of the
// per-axis gates came off in #800):
//
//   C=0, T=0, S=0 → nothing runs; nothing to commit.                        0
//   C=0, T=1, S=0 → thrust runs, sees `!S` → commits.                       1
//   C=0, T=0, S=1 → steering runs → commits.                                1
//   C=0, T=1, S=1 → thrust scratches; steering commits.                     1
//   C=1, T=0, S=0 → monolith runs, sees `!T && !S` → commits.               1
//   C=1, T=1, S=0 → monolith scratches; thrust commits.                     1
//   C=1, T=0, S=1 → monolith scratches; steering commits.                   1
//   C=1, T=1, S=1 → monolith and thrust scratch; steering commits.          1
//
// The shipped-hull case is the last row: every hull declares all three, and an
// unmanned station backfills all three to AI.
//
// Because every system that runs reads memory *before* any commit lands, and
// `helm_ai_decision` is pure given (memory, world_view, scored, tuning), all
// three reach the identical decision — so the axes never disagree and the
// per-axis result is bit-identical to what the monolith alone produced before
// #800. The "no Helm-relevant objective" early-`continue` short-circuits before
// `operate_helm` is called at all, on every path — zero commits everywhere, not
// an asymmetry.
// `ai_helm_thrust_commits_memory_when_steering_is_human` and
// `per_axis_gates_are_independent` pin the mixed cases;
// `helm_ai_memory_commits_exactly_once` and
// `coarse_helm_ai_commits_memory_exactly_once` pin that the both-AI and
// shipped-hull cases do not double-advance.

/// Per-axis helm AI: throttle. Writes `ThrustInput` for ships whose
/// helm-thrust system is AI-operated, whatever the coarse helm is doing —
/// `operate_helm_ai` stands down from the thrust axis for exactly these ships
/// (issue #800).
///
/// `AiHighFidelity`-scoped: `ThrustInput` only exists on ships carrying that
/// marker (`lod_ai_ships` inserts/removes it with the intent bundle), so
/// unlike `operate_lateral_thrust_ai` this system can take `&mut ThrustInput`
/// directly rather than `Option<&mut _>`.
///
/// Commits `AiMemory` when helm-steering is *not* AI-operated, i.e. when
/// `ai_helm_steering` will not run for this ship this tick and this system is
/// the first (and only) per-axis system that does — see the module note.
///
/// Takes `&ShipPhysics` read-only and never advances physics itself;
/// `integrate_ship_physics` is the sole helm-path writer (issues #695, #699).
#[allow(clippy::too_many_arguments)]
fn ai_helm_thrust(
    mut local_ship_input: Query<&mut LastHelmInput, With<crate::server_app::LocalShip>>,
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    entity_fallback_q: HelmAiFallbackQuery,
    mut ships: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            &mut crate::ai_plugin::ShipAiMemory,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Has<crate::server_app::LocalShip>,
            &mut ThrustInput,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();
    let snapshot_entities = helm_ai_snapshot_entities(
        world_snapshot.as_deref(),
        runtime.as_deref(),
        &entity_fallback_q,
    );

    for (
        sources,
        physics,
        mut ai_memory,
        blackboards,
        entity_uuid,
        faction,
        collider,
        helm_section,
        behaviour_section,
        is_local,
        mut thrust_in,
    ) in ships.iter_mut()
    {
        // Gate on our own axis alone (issue #800) — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_thrust_system_id())
            .operate_ai
        {
            continue;
        }

        let scored = helm_ai_scored_objectives(blackboards);
        if !has_helm_objective(&scored) {
            // No objectives → zero throttle (decelerate to stop) via the
            // shared physics curve rather than coasting on a stale intent.
            thrust_in.0 = 0.0;
            if is_local {
                if let Some(mut li) = local_ship_input.iter_mut().next() {
                    li.thrust = 0.0;
                }
            }
            continue;
        }

        let world_view = helm_ai_world_view(
            physics,
            entity_uuid,
            faction,
            collider,
            helm_section,
            blackboards,
            ship_client_config.as_deref(),
            is_local,
            &anchors,
            &snapshot_entities,
        );

        // AiMemory commit rule — see the module note. `ai_helm_steering` runs
        // after us and commits whenever it runs, so we must not: we'd
        // double-advance the patrol waypoint. But when helm-steering is *not*
        // AI-operated it never runs, and we are the last helm-AI system to run
        // on this ship this tick — so we commit, otherwise `waypoint_index` and
        // `target` would freeze forever in the thrust-AI/steering-human config.
        // `operate_helm_ai` ran before us and has already yielded the commit to
        // us whenever we run at all, so we never double-commit against it.
        let steering_is_ai = sources
            .0
            .policy_for(&crate::system_registry::helm_steering_system_id())
            .operate_ai;
        let mut memory_scratch;
        let memory: &mut crate::ai::AiMemory = if steering_is_ai {
            memory_scratch = ai_memory.0.clone();
            &mut memory_scratch
        } else {
            &mut ai_memory.0
        };
        let (thrust, _steering) = helm_ai_decision(
            memory,
            &world_view,
            &scored,
            behaviour_section,
            &anchors,
            physics.forward_speed,
            faction_registry.as_deref(),
        );

        thrust_in.0 = thrust;

        // Mirror to LastHelmInput for the player ship so broadcast /
        // fine-engine bookkeeping consumers see the AI's throttle, matching
        // what `operate_helm_ai` does for the coarse path.
        if is_local {
            if let Some(mut li) = local_ship_input.iter_mut().next() {
                li.thrust = thrust;
            }
        }
    }
}

/// Per-axis helm AI: steering. Writes `SteeringInput` for ships whose
/// helm-steering system is AI-operated, whatever the coarse helm is doing —
/// `operate_helm_ai` stands down from the steering axis (including the
/// arc-bearing step) for exactly these ships (issue #800).
///
/// Steers toward the selected waypoint/target chosen by the pure
/// `crate::ai::operate_helm`, which resolves the top-scored Helm-relevant
/// directive. That includes the **Retreat consumer** (issue #688): when
/// `AiDirective::Retreat` is the top-scored directive — as it is once
/// `aggregate_doctrine_blackboards` injects the synthetic hull-triggered
/// Retreat below the entity's TOML `[behaviour] retreat_hull_threshold` —
/// `operate_helm`'s Retreat arm resolves the anchor by name, falling back to
/// the ship's home position when the anchor is empty (which the synthetic one
/// always is), and steers toward it. `ai_helm_steering_retreats_toward_anchor`
/// and `ai_helm_steering_retreat_falls_back_to_home_position` pin that
/// behaviour through this system.
///
/// Commits `AiMemory` whenever it runs. Since it is `.after(ai_helm_thrust)`,
/// and `ai_helm_thrust` yields the commit to it precisely when helm-steering is
/// AI-operated, exactly one of the two commits per tick — see the module note
/// above. Takes `&ShipPhysics` read-only; `integrate_ship_physics` is the
/// sole helm-path physics writer (issues #695, #699).
#[allow(clippy::too_many_arguments)]
fn ai_helm_steering(
    mut local_ship_input: Query<&mut LastHelmInput, With<crate::server_app::LocalShip>>,
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    entity_fallback_q: HelmAiFallbackQuery,
    mut ships: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            &mut crate::ai_plugin::ShipAiMemory,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Has<crate::server_app::LocalShip>,
            Option<&mut PendingArcBearingRequest>,
            Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
            &mut SteeringInput,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();
    let snapshot_entities = helm_ai_snapshot_entities(
        world_snapshot.as_deref(),
        runtime.as_deref(),
        &entity_fallback_q,
    );

    for (
        sources,
        physics,
        mut ai_memory,
        blackboards,
        entity_uuid,
        faction,
        collider,
        helm_section,
        behaviour_section,
        is_local,
        mut pending_bearing,
        combat_config_opt,
        mut steering_in,
    ) in ships.iter_mut()
    {
        // Gate on our own axis alone (issue #800) — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_steering_system_id())
            .operate_ai
        {
            continue;
        }

        let scored = helm_ai_scored_objectives(blackboards);
        if !has_helm_objective(&scored) {
            steering_in.0 = 0.0;
            if is_local {
                if let Some(mut li) = local_ship_input.iter_mut().next() {
                    li.steering = 0.0;
                }
            }
            continue;
        }

        let world_view = helm_ai_world_view(
            physics,
            entity_uuid,
            faction,
            collider,
            helm_section,
            blackboards,
            ship_client_config.as_deref(),
            is_local,
            &anchors,
            &snapshot_entities,
        );

        // Commits AiMemory: waypoint advance / target selection happen here.
        // We are the last helm-AI system to run, and both `operate_helm_ai` and
        // `ai_helm_thrust` defer the commit whenever helm-steering is AI —
        // which it plainly is here — so this is the tick's one and only commit.
        // See the module note.
        let (_thrust, mut steering) = helm_ai_decision(
            &mut ai_memory.0,
            &world_view,
            &scored,
            behaviour_section,
            &anchors,
            physics.forward_speed,
            faction_registry.as_deref(),
        );

        // ── Weapons->Helm arc-bearing request (issue #677) ────────────────────
        apply_arc_bearing_request(
            &mut steering,
            pending_bearing.as_deref_mut(),
            &world_view,
            physics,
            combat_config_opt,
        );

        steering_in.0 = steering;

        // Mirror to LastHelmInput for the player ship — see `ai_helm_thrust`.
        if is_local {
            if let Some(mut li) = local_ship_input.iter_mut().next() {
                li.steering = steering;
            }
        }
    }
}

// ── Dedicated AI lateral thrust (issue #697) ──────────────────────────────────
// Runs when the lateral thrust system is under AI control (e.g. "Simplified"
// rating) but the main helm is human-controlled. In the "Backfill" or full-AI
// case, `operate_helm_ai` already handles lateral thrust — this system fills
// the gap for partial automation where only lateral thrust is AI-driven.

/// Runs AI lateral thrust for ships where the lateral thrust system is
/// under AI control but the main helm is human-controlled.
///
/// When the main helm is also AI-controlled, `operate_helm_ai` already
/// handles lateral thrust for obstacle avoidance. This system fills the
/// gap for the "Simplified" rating pattern where only the lateral thrust
/// system is automated.
fn operate_lateral_thrust_ai(
    time: Res<Time>,
    mut timer: ResMut<AiLateralThrustTimer>,
    mut local_ship_input: Query<&mut LastHelmInput, With<crate::server_app::LocalShip>>,
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    mut ships: Query<(
        &ShipSystemControlSources,
        &crate::server_app::ShipSystemBlackboards,
        &ShipPhysics,
        Option<&crate::entities::spawner::ColliderSection>,
        Option<&crate::entities::spawner::EntityUuid>,
        Option<&crate::entities::spawner::FactionComponent>,
        Has<crate::server_app::LocalShip>,
        // Only present under `AiHighFidelity` (issue #695). This system is
        // not itself `AiHighFidelity`-scoped — it can match a demoted NPC
        // that has lost the component — so the write below must guard on
        // `Some`/skip gracefully rather than assume presence.
        Option<&mut LateralThrustInput>,
        // Optional, so it does not filter the iteration set: a ship without a
        // `[behaviour]` section still runs AI lateral thrust, on the
        // `crate::ai::*` fallbacks that match the serde defaults.
        Option<&crate::entities::spawner::BehaviourSection>,
    )>,
) {
    let Some(ref snapshot) = world_snapshot else {
        return;
    };
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let snapshot_entities: Vec<crate::ai::AiWorldEntity> = snapshot.entities.clone();

    for (
        sources,
        blackboards,
        physics,
        collider,
        entity_uuid,
        faction,
        is_local,
        lateral_intent,
        behaviour_section,
    ) in ships.iter_mut()
    {
        // Only run when lateral thrust is AI-controlled but the main helm is not
        // (if helm is also AI, operate_helm_ai already handles it).
        let lt_policy = sources
            .0
            .policy_for(&crate::system_registry::lateral_thrust_system_id());
        if !lt_policy.operate_ai {
            continue;
        }
        if helm_control_policy(sources).operate_ai {
            continue;
        }

        use crate::messages::{ScoredObjective, SystemAffinity, SystemBlackboard};

        // Get scored objectives from the viewscreen blackboard.
        let scored: Vec<ScoredObjective> = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
            _ => Vec::new(),
        };

        let has_helm_objective = scored
            .iter()
            .any(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm));
        if !has_helm_objective {
            continue;
        }

        // Build a world view from the snapshot, filtering out self.
        let self_pos = [physics.x, 0.0, physics.z];
        let self_uuid_str = entity_uuid.map(|u| u.0.as_str()).unwrap_or("");
        let self_filtered: Vec<crate::ai::AiWorldEntity> = snapshot_entities
            .iter()
            .filter(|e| e.uuid.to_string() != self_uuid_str)
            .cloned()
            .collect();

        let entities = crate::ai::visible_entities(self_pos, 0.0, &self_filtered);
        let world_view = crate::ai::WorldView {
            entity_pos: self_pos,
            entity_yaw: physics.yaw,
            entities,
            self_faction: faction.map(|f| f.0),
            self_radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
            ..Default::default()
        };

        // TOML-authored avoidance tuning, same as `operate_helm_ai` and
        // `helm_ai_decision` use: how much clearance this hull wants is a
        // property of the hull, not of which system happens to be automated.
        let lateral = crate::ai::operate_lateral_thrust(
            &world_view,
            &scored,
            behaviour_section
                .map(|b| b.0.avoidance_buffer)
                .unwrap_or(crate::ai::AVOIDANCE_BUFFER),
            behaviour_section
                .map(|b| b.0.avoidance_look_ahead_secs)
                .unwrap_or(crate::ai::AVOIDANCE_LOOK_AHEAD_SECS),
            physics.forward_speed,
        );

        // For the player ship: write to LastHelmInput so process_helm_inputs
        // picks up the AI-driven lateral intent.
        if is_local {
            if let Some(mut li) = local_ship_input.iter_mut().next() {
                li.lateral = lateral;
            }
        }
        // Write the shared intent component too, for whichever ship this is
        // (local or NPC), so `integrate_ship_physics` sees the AI-driven
        // lateral value. Guarded: only present while `AiHighFidelity`.
        if let Some(mut intent) = lateral_intent {
            intent.0 = lateral;
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

fn sync_ship_position(mut ship_query: Query<(&ShipPhysics, &mut Transform)>) {
    for (physics, mut transform) in ship_query.iter_mut() {
        transform.translation.x = physics.x;
        transform.translation.z = physics.z;
        transform.rotation = Quat::from_euler(EulerRot::YXZ, -physics.yaw, 0.0, physics.roll);
    }
}

/// Admission-only (issue #695): turns hull-damage auto-cancel and admitted
/// `StartImpulseCharge`/`CancelImpulse` messages into an `ImpulseCommand`
/// intent, rather than mutating `ShipImpulse` directly. The shared
/// `integrate_ship_physics` system applies the actual `start_charge`/
/// `cancel_charge` transition. Only writes the intent when something
/// actually happened this tick (hull damage or an admitted command) —
/// otherwise leaves the persisted intent alone, mirroring the old
/// direct-mutation code which likewise only called `start_charge`/
/// `cancel_charge` on those same triggers. Hull-damage cancellation is
/// evaluated before the admitted-command loop, then the loop can still
/// override it within the same tick — matching the old sequential
/// direct-mutation order exactly.
pub fn handle_impulse_messages(
    ship_ac_query: Query<&AdmittedCommands, With<LocalShip>>,
    impulse_q: Query<&ShipImpulse, With<LocalShip>>,
    mut impulse_cmd_q: Query<&mut ImpulseCommand, With<LocalShip>>,
    hull_q: Query<&crate::entity_spawner::EntitySystemHull, With<LocalShip>>,
    mut last_hull_hp: Local<f32>,
    membership: Option<Res<RegionMembership>>,
    region_query: Query<&RegionEffectsSection>,
    ship_query: Query<Entity, With<LocalShip>>,
) {
    let Some(admitted) = ship_ac_query.iter().next() else {
        return;
    };
    // Guard: only proceed when the LocalShip actually carries `ShipImpulse`
    // (matches the old direct-mutation code's implicit guard).
    if impulse_q.iter().next().is_none() {
        return;
    }
    let hull_total = hull_q
        .single()
        .map(|h| (h.0.total_current(), h.0.total_max()))
        .unwrap_or((100.0, 100.0));
    if *last_hull_hp == 0.0 && (hull_total.0 - hull_total.1).abs() < 1e-6 {
        *last_hull_hp = hull_total.1;
    }

    let current_hp = hull_total.0;
    let mut desired: Option<crate::impulse::ImpulsePhase> = None;
    if current_hp < *last_hull_hp {
        desired = Some(crate::impulse::ImpulsePhase::Idle);
    }
    *last_hull_hp = current_hp;

    for cmd in admitted.for_target(crate::system_registry::HELM_SYSTEM_ID) {
        match &cmd.payload {
            SystemControlPayload::StartImpulseCharge
                if !is_inside_blocks_impulse(&membership, &region_query, &ship_query) =>
            {
                desired = Some(crate::impulse::ImpulsePhase::Charging);
            }
            SystemControlPayload::CancelImpulse => {
                desired = Some(crate::impulse::ImpulsePhase::Idle);
            }
            _ => {}
        }
    }

    if let Some(phase) = desired {
        if let Some(mut cmd_comp) = impulse_cmd_q.iter_mut().next() {
            cmd_comp.0 = phase;
        }
    }
}

fn tick_impulse(
    time: Res<Time>,
    mut ships_q: Query<(&mut ShipImpulse, Option<&ImpulseConfigResource>), With<Ship>>,
) {
    let dt = time.delta_secs();
    for (mut impulse, entity_cfg) in ships_q.iter_mut() {
        let charge_duration = entity_cfg.cloned().unwrap_or_default().charge_duration;
        impulse.0.tick(dt, charge_duration);
    }
}

/// Admission-only (issue #695): turns admitted `ToggleBoost`/`SetBoost`
/// messages into a `BoostCommand` intent (the desired active state),
/// rather than mutating `ShipBoost` directly. The shared
/// `integrate_ship_physics` system applies the actual `activate`/
/// `deactivate` transition (which itself enforces the battery-empty guard,
/// same as the old direct `toggle()`/`activate()` calls did). No-op when
/// the feature is disabled for this ship, matching the old behavior.
pub fn handle_boost_messages(
    ship_query: Query<
        (&AdmittedCommands, Option<&BoostConfigResource>, &ShipBoost),
        With<LocalShip>,
    >,
    mut boost_cmd_q: Query<&mut BoostCommand, With<LocalShip>>,
) {
    let Some((admitted, entity_cfg, entity_boost)) = ship_query.iter().next() else {
        return;
    };
    let enabled = entity_cfg.map(|c| c.enabled).unwrap_or(false);
    if !enabled {
        return;
    }
    let mut desired_active = entity_boost.0.is_active();
    let mut changed = false;
    for cmd in admitted.for_target(crate::system_registry::HELM_SYSTEM_ID) {
        match &cmd.payload {
            SystemControlPayload::ToggleBoost => {
                desired_active = !desired_active;
                changed = true;
            }
            SystemControlPayload::SetBoost { active } => {
                desired_active = *active;
                changed = true;
            }
            _ => {}
        }
    }
    if changed {
        if let Some(mut cmd_comp) = boost_cmd_q.iter_mut().next() {
            cmd_comp.0 = desired_active;
        }
    }
}

fn normalized_boost_drain_factor(thrust: f32, steering: f32) -> f32 {
    thrust.clamp(-1.0, 1.0).abs() + steering.clamp(-1.0, 1.0).abs()
}

fn tick_boost(
    time: Res<Time>,
    mut boost_entity_q: Query<(Option<&BoostConfigResource>, &mut ShipBoost), With<LocalShip>>,
    last_input_q: Query<&LastHelmInput, With<LocalShip>>,
    sessions: Res<Sessions>,
    impulse_q: Query<&ShipImpulse, With<LocalShip>>,
    ship_components: Query<(&ShipConfigComponent, &ShipSystemControlSources), With<LocalShip>>,
) {
    let Some((_ship_config, control_sources)) = ship_components.iter().next() else {
        return;
    };
    let Some((entity_cfg, mut entity_boost)) = boost_entity_q.iter_mut().next() else {
        return;
    };
    let config = entity_cfg.cloned().unwrap_or_default();
    if !config.enabled {
        return;
    }
    let last_input = last_input_q.single().copied().unwrap_or_default();
    let policy = helm_control_policy(control_sources);
    let has_helm = sessions
        .0
        .holder_for_station(&crate::messages::StationId("helm".into()))
        .is_some()
        || policy.operate_ai;
    let impulse_active = impulse_q
        .iter()
        .next()
        .map(|i| i.0.is_active())
        .unwrap_or(false);
    let drain_factor = if !has_helm {
        0.0
    } else if impulse_active {
        normalized_boost_drain_factor(1.0, 0.0)
    } else {
        normalized_boost_drain_factor(last_input.thrust, last_input.steering)
    };
    entity_boost.0.tick_with_drain_factor(
        time.delta_secs(),
        config.active_duration,
        config.recharge_duration,
        drain_factor,
    );
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

/// Applies commanded impulse/boost phase transitions (issue #695), split
/// out from `integrate_ship_physics` so it can run *before*
/// `process_helm_inputs` — whose stale-input edge-detection needs to
/// observe this tick's freshly-transitioned `ShipImpulse.phase`, not last
/// tick's (the old fused `process_helm_inputs` mutated `ShipImpulse` via
/// `handle_impulse_messages` before its own edge-detect read it in the
/// same tick; splitting admission from integration would otherwise delay
/// that transition by one tick).
///
/// Uses change detection (`Ref::is_changed`) rather than unconditionally
/// re-applying the persisted intent every tick: the intent components
/// default to `Idle`/`false`, and blindly re-applying that default every
/// tick would fight any *other* code path (including test harnesses) that
/// sets `ShipImpulse`/`ShipBoost` directly without going through the
/// intent-command pipeline. Only a tick where the intent was actually
/// written (by `operate_helm_ai`'s AI decision, `handle_impulse_messages`,
/// or `handle_boost_messages`) triggers a transition; `start_charge`/
/// `cancel_charge` and `activate`/`deactivate` are themselves idempotent,
/// so re-applying an intent that happens to already match current state is
/// harmless.
fn apply_helm_commands(
    mut ships: Query<
        (
            Option<&mut ShipImpulse>,
            Option<Ref<ImpulseCommand>>,
            Option<&mut ShipBoost>,
            Option<Ref<BoostCommand>>,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    for (impulse, impulse_cmd, boost, boost_cmd) in ships.iter_mut() {
        if let (Some(mut impulse), Some(cmd)) = (impulse, impulse_cmd) {
            // Exclude the insertion tick (issue #695 follow-up): LOD
            // promotion inserts a fresh default `ImpulseCommand`, which
            // Bevy also reports as "changed" on that same tick. Without
            // `!cmd.is_added()`, a promoted NPC's legitimate in-progress
            // impulse would be silently force-reset to `Idle` purely as a
            // side effect of gaining `AiHighFidelity`, not from any actual
            // AI decision or player command. The default should persist
            // untouched until something explicitly writes a new value on a
            // later tick.
            if cmd.is_changed() && !cmd.is_added() {
                match cmd.0 {
                    crate::impulse::ImpulsePhase::Charging => impulse.0.start_charge(),
                    crate::impulse::ImpulsePhase::Idle => impulse.0.cancel_charge(),
                    crate::impulse::ImpulsePhase::Active => {}
                }
            }
        }
        if let (Some(mut boost), Some(cmd)) = (boost, boost_cmd) {
            // Same insertion-tick exclusion as above; currently latent for
            // boost since no AI path admits `ShipBoost` for NPCs, but kept
            // consistent for when that changes.
            if cmd.is_changed() && !cmd.is_added() {
                if cmd.0 {
                    boost.0.activate();
                } else {
                    boost.0.deactivate();
                }
            }
        }
    }
}

/// Sole writer of the helm path into `ShipPhysics` (issue #699; extracted
/// from the old fused `process_helm_inputs` monolith by issue #695).
///
/// Reads the `ThrustInput`/`SteeringInput`/`LateralThrustInput` intent
/// components — written this tick by whichever of `process_helm_inputs`
/// (human admission) or `operate_helm_ai` (AI decision) is authoritative for
/// a given ship's helm, per the existing `ControlTickPolicy` mutual-exclusion
/// gate — plus the post-transition `ShipImpulse`/`ShipBoost` state applied
/// by `apply_helm_commands`, and performs the actual physics integration.
/// Runs for both the player ship and any AI-promoted NPC (anything
/// carrying `AiHighFidelity`, which is exactly the set of ships carrying
/// these intent components). Human and AI helm therefore share one
/// integrator and produce identical trajectories from identical intent —
/// nothing below this point branches on human-vs-AI.
///
/// Concerns handled here, in order:
///  - impulse autopilot override (forces thrust=1, steering=0, lateral=0),
///  - engine-damage thrust scaling (issue #511),
///  - impulse acceleration multiplier,
///  - boost-drive speed/acceleration/steering multiplier,
///  - exactly one `compute_physics` call per ship per frame,
///  - visual banking/roll lerp.
///
/// Visual banking/roll is preserved as LocalShip-only, exactly as before —
/// `operate_helm_ai` never applied roll to NPCs, and this system doesn't
/// start doing so either.
///
/// This system is the only *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`, enforced in debug
/// builds by `HelmPhysicsWriteGuard`. It is not the only writer of those
/// fields overall — see the sanctioned-exception table on `ShipPhysics`
/// (`src/ship/state.rs`) for the four out-of-band writers.
#[allow(clippy::too_many_arguments)]
fn integrate_ship_physics(
    time: Res<Time>,
    physics_cfg_res: Option<Res<ShipPhysicsConfigResource>>,
    bank_cfg_res: Option<Res<BankConfigResource>>,
    mut ships: Query<
        (
            Entity,
            Has<LocalShip>,
            &ShipSystemControlSources,
            &mut ShipPhysics,
            Option<&ShipModifiers>,
            Option<&ShipPhysicsConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&BoostConfigResource>,
            Option<&BankConfigResource>,
            &ThrustInput,
            &SteeringInput,
            &LateralThrustInput,
            (Option<&ShipImpulse>, Option<&ShipBoost>),
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
    #[cfg(debug_assertions)] frame: Res<crate::ship::helm::HelmPhysicsFrame>,
    #[cfg(debug_assertions)] mut guard_q: Query<&mut crate::ship::helm::HelmPhysicsWriteGuard>,
    #[cfg(debug_assertions)] mut commands: Commands,
) {
    let dt = time.delta_secs().min(HELM_AI_MAX_DT_SECS);

    for (
        // Only read by the debug-only write tracker below; underscored so
        // release builds need no blanket `allow(unused_variables)`.
        _entity,
        is_local,
        sources,
        mut physics,
        modifiers,
        physics_cfg_comp,
        impulse_cfg,
        boost_cfg_comp,
        bank_cfg_comp,
        thrust_in,
        steering_in,
        lateral_in,
        (impulse, boost),
    ) in ships.iter_mut()
    {
        // Debug-only single-writer tripwire (issue #699). Self-healing: ships
        // that lack a guard get one, so promotion/demotion needs no bookkeeping.
        #[cfg(debug_assertions)]
        {
            let entity = _entity;
            match guard_q.get_mut(entity) {
                Ok(mut guard) => guard.record_write(entity, "integrate_ship_physics", frame.0),
                Err(_) => {
                    let mut guard = crate::ship::helm::HelmPhysicsWriteGuard::default();
                    guard.record_write(entity, "integrate_ship_physics", frame.0);
                    commands.entity(entity).insert(guard);
                }
            }
        }

        let default_modifiers;
        let modifiers: &ShipModifiers = match modifiers {
            Some(m) => m,
            None => {
                default_modifiers = ShipModifiers::new();
                &default_modifiers
            }
        };

        let state = ShipPhysicsState {
            x: physics.x,
            z: physics.z,
            yaw: physics.yaw,
            forward_speed: physics.forward_speed,
            lateral_speed: physics.lateral_speed,
        };

        let impulse_active = impulse.map(|i| i.0.is_active()).unwrap_or(false);

        let input = if impulse_active {
            // Autopilot: full forward thrust, zero steering. Helm input is ignored.
            ShipPhysicsInput {
                thrust: 1.0,
                steering: 0.0,
                lateral: 0.0,
            }
        } else {
            ShipPhysicsInput {
                thrust: thrust_in.0,
                steering: steering_in.0,
                lateral: lateral_in.0,
            }
        };

        // ── Engine-damage thrust scaling (issue #511) ──────────────────────
        // Count how many fine engine systems are online. Each offline engine
        // removes 50% of the computed thrust. If both engines are offline,
        // thrust is zeroed.
        let port_offline = sources
            .0
            .offline_systems
            .contains(&crate::system_registry::helm_engine_port_system_id());
        let stbd_offline = sources
            .0
            .offline_systems
            .contains(&crate::system_registry::helm_engine_starboard_system_id());
        let engine_thrust_scale: f32 = match (port_offline, stbd_offline) {
            (true, true) => 0.0,
            (true, false) | (false, true) => 0.5,
            (false, false) => 1.0,
        };
        let scaled_input = ShipPhysicsInput {
            thrust: input.thrust * engine_thrust_scale,
            steering: input.steering,
            lateral: input.lateral,
        };

        let mut config = physics_cfg_comp
            .map(|c| c.0)
            .or_else(|| physics_cfg_res.as_deref().map(|c| c.0))
            .unwrap_or_else(ShipPhysicsConfig::new);
        config.max_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
        config.max_reverse_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
        config.max_yaw_rate *= modifiers.get(&ModifierSlot::MaxYawRate);

        if impulse_active {
            // Mirror `ship/impulse.rs::apply_to_physics`: a non-positive
            // multiplier (e.g. an unset TOML field defaulting to 0) falls
            // back to the const instead of nuking acceleration entirely.
            let impulse_cfg = impulse_cfg.cloned().unwrap_or_default();
            let mult = if impulse_cfg.acceleration_multiplier > 0.0 {
                impulse_cfg.acceleration_multiplier
            } else {
                crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER
            };
            config.acceleration *= mult;
        }

        // Boost drive: while engaged, multiply max speed and acceleration.
        // Only applies when the ship's TOML enabled the feature.
        let boost_cfg = boost_cfg_comp.cloned().unwrap_or_default();
        let boost_active = boost.map(|b| b.0.is_active()).unwrap_or(false);
        if boost_cfg.enabled && boost_active {
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
        physics.lateral_speed = result.lateral_speed;

        // Visual banking: LocalShip only, exactly as before — the old
        // `operate_helm_ai` never applied roll to NPCs, and this shared
        // step doesn't start doing so either. Uses the unscaled
        // `input.steering` so roll reflects intent, not engine count.
        if is_local {
            let bank_cfg = bank_cfg_comp
                .cloned()
                .or_else(|| bank_cfg_res.as_deref().cloned())
                .unwrap_or_default();
            let max_bank_rad = bank_cfg.max_bank_deg.to_radians();
            let target_roll = if impulse_active {
                0.0
            } else {
                -input.steering * max_bank_rad
            };
            let lerp_factor = (bank_cfg.bank_lerp_rate * dt).min(1.0);
            physics.roll = physics.roll + (target_roll - physics.roll) * lerp_factor;
        }
    }
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

/// Format a `CoordinationPayload` into a short text string for viewscreen chatter.
fn format_coordination_chatter(payload: &CoordinationPayload) -> String {
    match payload {
        CoordinationPayload::Advisory { message } => message.clone(),
        CoordinationPayload::Alert { title, body } => {
            if body.is_empty() {
                title.clone()
            } else {
                format!("{title}: {body}")
            }
        }
        CoordinationPayload::FrequencyHint { frequency } => {
            format!("Frequency hint: {frequency:.1}")
        }
        CoordinationPayload::ShieldFacingDown {
            label,
            offline_remaining,
        } => {
            format!("{label} offline ({offline_remaining:.0}s)")
        }
        CoordinationPayload::ShieldFacingRestored { label } => {
            format!("{label} restored")
        }
        CoordinationPayload::TargetDesignation { label, .. } => {
            format!("Designating target: {label}")
        }
        CoordinationPayload::ArcBearingRequest { label, .. } => {
            format!("Come about, bring phasers to bear on {label}")
        }
        CoordinationPayload::PowerBrownout {
            label,
            allocated_level,
            ..
        } => {
            format!("{label} brownout (level {allocated_level})")
        }
        CoordinationPayload::NavigateTo { label, .. } => {
            format!("Navigation: steer toward {label}")
        }
        CoordinationPayload::RepairRequest {
            station_label,
            tier,
            ..
        } => {
            format!("Repair requested for {station_label} ({tier:?})")
        }
        CoordinationPayload::ThreatBearing {
            bearing_rad, label, ..
        } => {
            let bearing_deg = (bearing_rad.to_degrees() + 360.0) % 360.0;
            format!("Sensors: threat bearing {bearing_deg:.0}° - {label}")
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
            Option<&mut PendingArcBearingRequest>,
            Option<&mut crate::ai_plugin::ShipAiMemory>,
            Option<&mut RepairHumanAlerted>,
            Option<&mut crate::console::repair::server::RepairRequestQueue>,
            Option<&mut crate::ship::shields::PendingShieldsThreatBearing>,
            Has<LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    sessions: Res<Sessions>,
    mut outbox: ResMut<crate::lobby::LobbyOutbox>,
    mut chatter_writer: MessageWriter<AiChatterEvent>,
) {
    let repair_id = crate::ship::system_registry::repair_system_id();
    let shields_id = crate::system_registry::shields_system_id();
    let now = time.elapsed_secs();
    for (
        ship_config,
        control_sources,
        mut queue,
        mut pending_bearing,
        mut ai_memory,
        mut alerted,
        mut repair_queue,
        mut pending_shields_threat,
        is_local,
    ) in ship_components.iter_mut()
    {
        let due = queue.0.due_messages(now);
        for msg in due {
            let target_policy = control_sources.0.policy_for(&msg.target);
            let action = if !target_policy.operate_ai && !target_policy.accept_human_input {
                coordination::DeliverAction::Consume
            } else {
                let target_control = control_sources.0.source_for(&msg.target);
                coordination::route_coordination(msg.sender_origin, target_control)
            };

            match action {
                coordination::DeliverAction::Consume => {
                    // RepairRequest for AI repair: push into the priority queue.
                    if target_policy.operate_ai && msg.target == repair_id {
                        if let CoordinationPayload::RepairRequest {
                            station_id,
                            station_label,
                            tier,
                            deficit,
                        } = &msg.payload
                        {
                            if let Some(ref mut rq) = repair_queue {
                                rq.push_or_merge(
                                    crate::console::repair::server::RepairQueueEntry {
                                        station_id: station_id.clone(),
                                        station_label: station_label.clone(),
                                        tier: *tier,
                                        deficit: *deficit,
                                    },
                                );
                            }
                        }
                    }
                    // AI Helm folds a consumed arc-bearing request into its
                    // steering (issue #677) rather than only chattering about it.
                    if target_policy.operate_ai
                        && msg.target == crate::system_registry::helm_system_id()
                    {
                        if let CoordinationPayload::ArcBearingRequest { uuid, .. } = &msg.payload {
                            if let Some(pending) = pending_bearing.as_deref_mut() {
                                pending.0 = uuid::Uuid::parse_str(uuid).ok();
                            }
                        }
                        // Channel-3 Navigation-to-Helm handoff (issue #681):
                        // stash the long-range steer target for AI Helm's
                        // fallthrough in operate_helm.
                        if let CoordinationPayload::NavigateTo { x, z, .. } = &msg.payload {
                            if let Some(ai_mem) = ai_memory.as_deref_mut() {
                                ai_mem.0.nav_goal = Some([*x, *z]);
                            }
                        }
                    }
                    // AI Shields consumes a Sensors threat bearing to rotate
                    // the closest facing toward the incoming threat (issue #683).
                    if target_policy.operate_ai && msg.target == shields_id {
                        if let CoordinationPayload::ThreatBearing { bearing_rad, .. } = &msg.payload
                        {
                            if let Some(pending) = pending_shields_threat.as_deref_mut() {
                                pending.0 = Some(*bearing_rad);
                            }
                        }
                    }
                    // AI→AI: emit viewscreen chatter for the LocalShip only.
                    if is_local {
                        let from_label = if msg.sender_label.is_empty() {
                            "AI".to_string()
                        } else {
                            msg.sender_label.clone()
                        };
                        let to_label = msg.target.0.clone();
                        let text = format_coordination_chatter(&msg.payload);
                        chatter_writer.write(AiChatterEvent {
                            from_label,
                            to_label,
                            text,
                        });
                    }
                }
                coordination::DeliverAction::Suppress => {}
                coordination::DeliverAction::Popup => {
                    // Popups require a browser-connected console holder.
                    // Only the LocalShip has one — NPCs drain silently.
                    if !is_local {
                        continue;
                    }

                    // Escalation-only filter for repair popups (issue #682):
                    // human repair sees popups only on first-damage and
                    // Disabled/Destroyed tier crossings.
                    if msg.target == repair_id {
                        if let CoordinationPayload::RepairRequest {
                            station_id, tier, ..
                        } = &msg.payload
                        {
                            let already = alerted
                                .as_deref()
                                .and_then(|a| a.0.get(station_id).copied())
                                .unwrap_or(DamageTier::Operational);
                            if *tier < DamageTier::Disabled && already != DamageTier::Operational {
                                continue;
                            }
                            if let Some(a) = alerted.as_deref_mut() {
                                a.0.insert(station_id.clone(), *tier);
                            }
                        }
                    }

                    let label = if msg.sender_label.is_empty() {
                        "AI".to_string()
                    } else {
                        msg.sender_label
                    };

                    let system = ship_config.0.system(&msg.target);
                    let station_opt = system.and_then(|s| s.station.as_ref());

                    if let Some(station_id) = station_opt {
                        if ship_config.0.station(station_id).is_some() {
                            let token: Option<String> = sessions
                                .0
                                .holder_for_station(station_id)
                                .map(|t| t.to_string());

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
/// the current damage tiers of each system in the ship hull.
///
/// Runs in `SimSet::Damage` (after hull damage is applied). For every ship that
/// carries both an [`EntitySystemHull`](crate::entity_spawner::EntitySystemHull)
/// (wrapping [`SystemHull`]) and `ShipSystemControlSources`:
///
/// - Systems in `Disabled` or `Destroyed` tier: their corresponding `SystemId`
///   is added to `offline_systems`.
/// - Systems in `Operational` or `Damaged` tier: their corresponding
///   `SystemId` is removed from `offline_systems` (restoring normal gating).
///
/// The `SystemId` for each entry is the key of the [`SystemHull`] map
/// directly — no `Console` → `SystemId` translation is needed.
///
/// Post-#514: also iterates the ship's `ShipArcHull` (when present) and flips
/// each arc's fine `SystemId("shield-arc-<id>")` in/out of `offline_systems`
/// using the same tier-derivation policy. Ships without a `ShipArcHull` (NPCs,
/// legacy fixtures) are unaffected.
///
/// Fix to issue #617: earlier this system iterated BOTH `EntityConsoleHull`
/// AND `EntitySystemHull` in parallel. In production only one of the two was
/// mutated by damage code, so the second (unmodified) iteration silently
/// cleared `offline_systems` entries that the first iteration correctly
/// inserted. The reviewer caught this and the fix drops the duplicate
/// iteration and picks `EntitySystemHull` as the single source of truth.
pub fn sync_console_damage_tiers(
    mut ships: Query<(
        &crate::entity_spawner::EntitySystemHull,
        Option<&crate::entity_spawner::EntityShipArcHull>,
        &mut ShipSystemControlSources,
    )>,
) {
    for (system_hull_component, arc_hull_opt, mut control_sources) in ships.iter_mut() {
        let hull = &system_hull_component.0;
        for (sid, _cur, _max) in hull.entries() {
            let tier = hull.tier_for(sid);
            match tier {
                DamageTier::Disabled | DamageTier::Destroyed => {
                    control_sources.0.offline_systems.insert(sid.clone());
                }
                DamageTier::Operational | DamageTier::Damaged => {
                    control_sources.0.offline_systems.remove(sid);
                }
            }
        }
        // Per-arc hull tier sync (issue #514).
        if let Some(arc_hull_component) = arc_hull_opt {
            let arc_hull = &arc_hull_component.0;
            for (arc_id, _entry) in arc_hull.iter() {
                let Some(sid) = crate::system_registry::shield_arc_system_id(arc_id) else {
                    continue;
                };
                let tier = arc_hull.tier_for(arc_id);
                match tier {
                    DamageTier::Disabled | DamageTier::Destroyed => {
                        control_sources.0.offline_systems.insert(sid);
                    }
                    DamageTier::Operational | DamageTier::Damaged => {
                        control_sources.0.offline_systems.remove(&sid);
                    }
                }
            }
        }
    }
}

/// Detect damage-tier crossings and emit `CoordinationEnqueue::RepairRequest`
/// when a system drops to a worse tier (issue #682).
///
/// Runs in `SimSet::Damage` (after hull damage is applied). For each ship
/// with both `EntitySystemHull` and `LastSystemTiers`, compares the current
/// tier (via `tier_for`) against the last-seen tier.  On a crossing to a
/// *worse* tier, enqueues a `RepairRequest` for the system's owning station
/// (or `"core"` for ownerless systems).  Destroyed systems are skipped —
/// they are unrepairable.
pub fn detect_damage_tier_crossings(
    mut ships: Query<(
        Entity,
        &crate::entity_spawner::EntitySystemHull,
        &mut LastSystemTiers,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        Option<&mut RepairHumanAlerted>,
    )>,
    mut coord_writer: MessageWriter<CoordinationEnqueue>,
) {
    for (entity, hull_comp, mut last_tiers, config, sources, mut alerted) in &mut ships {
        let hull = &hull_comp.0;
        for (system_id, _cur, _max) in hull.entries() {
            let current_tier = hull.tier_for(system_id);
            let prev_tier = last_tiers
                .0
                .get(system_id)
                .copied()
                .unwrap_or(DamageTier::Operational);

            if current_tier > prev_tier {
                if current_tier == DamageTier::Destroyed {
                    let sender_origin = sources.0.source_for(system_id);
                    coord_writer.write(CoordinationEnqueue {
                        source_entity: entity,
                        sender_origin,
                        target: crate::ship::system_registry::captain_system_id(),
                        payload: CoordinationPayload::Alert {
                            title: format!("System Destroyed: {}", system_id.0),
                            body: format!("{} destroyed.", system_id.0),
                        },
                        sender_label: system_id.0.clone(),
                    });
                    continue;
                }

                let system_config = config.0.system(system_id);
                let station_id = system_config
                    .and_then(|s| s.station.as_ref())
                    .map(|s| s.0.clone())
                    .unwrap_or_else(|| "core".to_string());
                let station_label = station_id.clone();
                let entry = hull.get(system_id).expect("just iterated entry");
                let deficit = entry.max - entry.current;
                let sender_origin = sources.0.source_for(system_id);

                coord_writer.write(CoordinationEnqueue {
                    source_entity: entity,
                    sender_origin,
                    target: crate::ship::system_registry::repair_system_id(),
                    payload: CoordinationPayload::RepairRequest {
                        station_id,
                        station_label,
                        tier: current_tier,
                        deficit,
                    },
                    sender_label: system_id.0.clone(),
                });
            } else if current_tier == DamageTier::Operational && prev_tier > DamageTier::Operational
            {
                let system_config = config.0.system(system_id);
                let station_id = system_config
                    .and_then(|s| s.station.as_ref())
                    .map(|s| s.0.clone())
                    .unwrap_or_else(|| "core".to_string());
                if let Some(ref mut a) = alerted {
                    if crate::console::repair::server::all_systems_in_station_are_operational(
                        &station_id,
                        hull,
                        &config.0,
                    ) {
                        a.0.remove(&station_id);
                    }
                }
            }
        }
        for (system_id, _cur, _max) in hull.entries() {
            last_tiers
                .0
                .insert(system_id.clone(), hull.tier_for(system_id));
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
            .add_plugins(ShipPlugin);
        let hull_config = &[
            (crate::messages::SystemId("helm".into()), 25.0_f32),
            (crate::messages::SystemId("tactical".into()), 25.0),
            (crate::messages::SystemId("power".into()), 25.0),
            (crate::messages::SystemId("shields".into()), 25.0),
        ];
        let ship = app
            .world_mut()
            .spawn((
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
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(
                    hull_config,
                )),
                LastHelmInput::default(),
                crate::simulation::ShipShields(crate::shield::ShieldSystem::default(), 0.5),
                ShipImpulse(crate::impulse::ImpulseState::new()),
            ))
            .id();
        app.world_mut().entity_mut(ship).insert((
            ShipModifiers::new(),
            ShipBoost::default(),
            crate::ai_plugin::AiHighFidelity,
            crate::ship::shields::ShieldArcIntents::default(),
            crate::console_ai_plugin::ShipFrequencyHintState::default(),
            crate::ship::power::PowerReactorIntents::default(),
            crate::ship::power::ShipPowerAiState::default(),
            crate::weapons_plugin::TorpedoIntents::default(),
        ));
        app.world_mut().entity_mut(ship).insert((
            crate::ship::helm::ThrustInput::default(),
            crate::ship::helm::SteeringInput::default(),
            crate::ship::helm::LateralThrustInput::default(),
            crate::ship::helm::ImpulseCommand::default(),
            crate::ship::helm::BoostCommand::default(),
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
            .get_mut::<crate::entity_spawner::EntitySystemHull>()
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

    fn find_ship_entity(app: &mut App) -> Entity {
        app.world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .expect("LocalShip entity must exist")
    }

    fn toggle_boost(app: &mut App) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ShipBoost>()
            .unwrap()
            .0
            .toggle();
    }

    fn boost_is_active(app: &mut App) -> bool {
        let ship = find_ship_entity(app);
        app.world()
            .entity(ship)
            .get::<ShipBoost>()
            .map(|b| b.0.is_active())
            .unwrap_or(false)
    }

    fn boost_battery(app: &mut App) -> f32 {
        let ship = find_ship_entity(app);
        app.world()
            .entity(ship)
            .get::<ShipBoost>()
            .map(|b| b.0.battery)
            .unwrap_or(0.0)
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
                station: "Captain".into(),
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
        let mut q = app.world_mut().query_filtered::<&ShipPhysics, With<Ship>>();
        *q.single(app.world())
            .expect("expected Ship entity with ShipPhysics")
    }

    // Test helper for directly seeding ship physics state — the avoidance
    // tests use it to give the ship a forward speed, which the projection and
    // the `AVOIDANCE_MIN_SPEED` gate both depend on.
    fn set_ship_physics(app: &mut App, physics: ShipPhysics) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipPhysics, With<Ship>>();
        let mut p = q
            .single_mut(app.world_mut())
            .expect("expected Ship with ShipPhysics");
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

    // Regression test for issue #695 follow-up: LOD promotion re-inserting
    // a fresh default `ImpulseCommand` must not silently cancel an
    // in-progress impulse charge on the tick it's (re-)added. Bevy marks a
    // freshly-inserted component as "changed" on its insertion tick, so
    // without the `!cmd.is_added()` guard in `apply_helm_commands`, a
    // ship's legitimate `Charging` state would get force-reset to `Idle`
    // purely as a side effect of gaining `AiHighFidelity`/`ImpulseCommand`
    // again, not from any explicit AI decision or player command.
    #[test]
    fn impulse_command_reinsertion_does_not_cancel_in_progress_charge() {
        let mut app = test_app();
        // Let the app settle past the initial-spawn insertion tick.
        tick(&mut app);

        // Simulate the ship having been mid-charge (e.g. promoted while a
        // human/AI decision had already started charging impulse).
        set_ship_impulse(
            &mut app,
            crate::impulse::ImpulseState {
                phase: ImpulsePhase::Charging,
                charge_progress: 0.4,
            },
        );

        // Simulate LOD demotion: remove the intent component but leave
        // `ShipImpulse` untouched, exactly as `lod_ai_ships`'s demote
        // branch does.
        let ship = find_ship_entity(&mut app);
        app.world_mut().entity_mut(ship).remove::<ImpulseCommand>();
        tick(&mut app);

        // The impulse charge must have persisted across demotion (no
        // `ImpulseCommand` present means `apply_helm_commands` skips this
        // ship entirely).
        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Charging);

        // Simulate LOD re-promotion: `lod_ai_ships` inserts a fresh
        // default `ImpulseCommand` (phase = Idle) on the ship.
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseCommand::default());
        tick(&mut app);

        // On the insertion tick, the default `Idle` value must NOT be
        // force-applied: the in-progress charge should persist untouched
        // because no explicit AI/human decision wrote a new value yet.
        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
            "re-inserting ImpulseCommand on LOD promotion must not cancel an in-progress impulse charge"
        );

        // A subsequent tick where something explicitly writes a changed
        // value (not merely re-inserts) should still apply normally.
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ImpulseCommand>()
            .unwrap()
            .0 = ImpulsePhase::Idle;
        tick(&mut app);
        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Idle);
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
                steering: 0.25,
                lateral: 0.0,
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
                steering: 0.0,
                lateral: 0.0,
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
        assert_eq!(get_last_helm_input(&mut app), LastHelmInput::default());
    }

    #[test]
    fn human_helm_suppresses_ai_operate() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        tick(&mut app);

        assert_eq!(get_last_helm_input(&mut app), LastHelmInput::default());
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

        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Charging,);
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

        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Charging,);
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

        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Idle,);
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

        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Idle,);
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
            shield_arcs: Vec::new(),
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
            audio: None,
            comms: None,
            sensors_console: None,
            navigation_console: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            star: None,
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            target: None,
            cinematic_camera: None,
            ai_profile: None,
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
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseConfigResource {
                charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
                speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
                acceleration_multiplier: 5.0,
                engage_distance: 200.0,
                cancel_distance: 40.0,
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
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseConfigResource {
                charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
                speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
                acceleration_multiplier: 5.0,
                engage_distance: 200.0,
                cancel_distance: 40.0,
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
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseConfigResource {
                charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
                speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
                acceleration_multiplier: 0.0,
                engage_distance: 200.0,
                cancel_distance: 40.0,
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
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);
        toggle_boost(&mut app); // engage
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
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);
        toggle_boost(&mut app);
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
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        toggle_boost(&mut app);
        {
            set_last_helm_input(
                &mut app,
                LastHelmInput {
                    thrust: 1.0,
                    steering: 1.0,
                    lateral: 0.0,
                },
            );
        }

        tick(&mut app);

        let battery = boost_battery(&mut app);
        assert!(
            (battery - 0.9).abs() < 0.001,
            "full thrust + full steering should drain twice the base rate; got {battery}"
        );
    }

    #[test]
    fn active_boost_battery_does_not_drain_with_idle_helm() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        toggle_boost(&mut app);

        tick(&mut app);

        let battery = boost_battery(&mut app);
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
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
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
        assert!(boost_is_active(&mut app));

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(!boost_is_active(&mut app));
    }

    #[test]
    fn control_system_toggle_boost_engages_when_enabled() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
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
        assert!(boost_is_active(&mut app));
    }

    #[test]
    fn control_system_set_boost_sets_active_state() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
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
        assert!(boost_is_active(&mut app));

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::SetBoost { active: false },
            },
        );
        tick(&mut app);
        assert!(!boost_is_active(&mut app));
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
            !boost_is_active(&mut app),
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
    /// need automation to be configured independently of the ship TOML.
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

    fn retreat_scored_objective(anchor: &str, score: f32) -> crate::messages::ScoredObjective {
        crate::messages::ScoredObjective {
            id: format!("retreat-{anchor}"),
            score,
            directive: crate::messages::AiDirective::Retreat {
                anchor: anchor.into(),
            },
            source: crate::messages::ObjectiveSource::Mission,
            relevance: vec![crate::messages::SystemAffinity::Helm],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: format!("retreat-{anchor}"),
                text: format!("Retreat to {anchor}"),
                mandatory: false,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::messages::ObjectiveSource::Mission,
            },
        }
    }

    // ── Per-axis helm AI (issue #701) ──────────────────────────────────────

    /// Point a fine system's control source at `source` on every ship.
    fn set_fine_control_source(
        app: &mut App,
        system_id: crate::messages::SystemId,
        source: ControlSource,
    ) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(system_id.clone(), source);
        }
    }

    /// The "partial automation" wiring the per-axis systems exist for: the
    /// coarse helm stays human-held while both per-axis systems are AI.
    fn set_per_axis_helm_ai(app: &mut App) {
        set_helm_control_source(app, ControlSource::Human);
        set_fine_control_source(
            app,
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );
        set_fine_control_source(
            app,
            crate::system_registry::helm_steering_system_id(),
            ControlSource::Ai,
        );
    }

    fn get_thrust_input(app: &mut App) -> f32 {
        app.world_mut()
            .query_filtered::<&ThrustInput, With<Ship>>()
            .single(app.world())
            .expect("expected Ship with ThrustInput")
            .0
    }

    fn get_steering_input(app: &mut App) -> f32 {
        app.world_mut()
            .query_filtered::<&SteeringInput, With<Ship>>()
            .single(app.world())
            .expect("expected Ship with SteeringInput")
            .0
    }

    fn set_ship_home_position(app: &mut App, pos: [f32; 3]) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut mem = entity
            .get_mut::<crate::ai_plugin::ShipAiMemory>()
            .expect("expected ShipAiMemory");
        mem.0.home_position = pos;
    }

    fn set_ship_nav_goal(app: &mut App, goal: Option<[f32; 2]>) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut mem = entity
            .get_mut::<crate::ai_plugin::ShipAiMemory>()
            .expect("expected ShipAiMemory");
        mem.0.nav_goal = goal;
    }

    fn get_ship_nav_goal(app: &mut App) -> Option<[f32; 2]> {
        let ship = find_ship_entity(app);
        app.world()
            .entity(ship)
            .get::<crate::ai_plugin::ShipAiMemory>()
            .expect("expected ShipAiMemory")
            .0
            .nav_goal
    }

    /// Build a `ControlSourceResolver` from a shipped hull's TOML the way the
    /// NPC spawn path does (`crate::entities::spawner`): parse the file, then
    /// set every *declared* system to `ControlSource::Ai`. Nothing is hand-set,
    /// so the resolver reflects exactly what the hull declares — which is the
    /// point of the tests that use it.
    fn resolver_from_shipped_npc_hull(toml_str: &str) -> ControlSourceResolver {
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("shipped hull TOML must parse");
        let ship_config = config
            .ship_config
            .expect("shipped hull must declare [[system]] blocks");
        let mut resolver = ControlSourceResolver::new();
        for system in &ship_config.systems {
            resolver.set(system.id.clone(), ControlSource::Ai);
        }
        resolver
    }

    /// Install a resolver verbatim on every ship, replacing whatever the test
    /// harness set up.
    fn install_control_sources(app: &mut App, resolver: &ControlSourceResolver) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0 = resolver.clone();
        }
    }

    /// AC5 (issue #800), and the coverage gap that let the dormancy ship.
    ///
    /// Every other per-axis test hand-builds its control sources, so all of
    /// them passed while `helm-thrust` / `helm-steering` were declared in
    /// **zero** shipped TOMLs — their policy defaulted to Human, the per-axis
    /// systems never fired in shipped content, and `operate_helm_ai` quietly did
    /// all the work. This test refuses to hand-build: the sources come from a
    /// real shipped hull.
    ///
    /// The proof that the *per-axis* systems (not the monolith) produced the
    /// intent is the stand-down: this hull declares all three systems and the
    /// NPC spawn path backfills every one to AI, so `operate_helm_ai` skips both
    /// the thrust and the steering write. A non-zero intent has nowhere else to
    /// come from.
    #[test]
    fn shipped_hull_config_drives_the_per_axis_helm_systems() {
        let resolver =
            resolver_from_shipped_npc_hull(include_str!("../assets/entities/pirate_raider.toml"));

        // The declaration itself — what #800 adds, and what was missing.
        assert!(
            resolver
                .policy_for(&crate::system_registry::helm_thrust_system_id())
                .operate_ai,
            "the shipped hull must declare helm-thrust, or ai_helm_thrust is dormant \
             in shipped content"
        );
        assert!(
            resolver
                .policy_for(&crate::system_registry::helm_steering_system_id())
                .operate_ai,
            "the shipped hull must declare helm-steering, or ai_helm_steering is dormant \
             in shipped content"
        );
        // Precondition for the proof below: with both axes AI, the monolith
        // stands down from both, so it cannot be the writer.
        assert!(
            resolver
                .policy_for(&crate::system_registry::helm_system_id())
                .operate_ai,
            "sanity: the shipped hull also declares the coarse helm, so this is the \
             all-three-AI case the monolith used to own outright"
        );

        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        install_control_sources(&mut app, &resolver);

        tick(&mut app);

        assert!(
            get_thrust_input(&mut app) > 0.0,
            "ai_helm_thrust must drive a shipped hull's throttle toward a Reach anchor \
             (operate_helm_ai stands down from the thrust axis here)"
        );
        assert!(
            get_steering_input(&mut app).abs() > 0.0,
            "ai_helm_steering must drive a shipped hull's yaw toward a Reach anchor \
             (operate_helm_ai stands down from the steering axis here)"
        );
    }

    /// AC3, on a shipped hull: declaring the axes must not change what the ship
    /// actually does. Pins the per-axis path's intent against the coarse path's
    /// for the same hull and the same objective — the monolith is still alive
    /// until #704, so the comparison is available to make.
    #[test]
    fn shipped_hull_per_axis_intent_matches_the_coarse_path() {
        let anchor = "station-alpha";

        let run = |resolver: &ControlSourceResolver| {
            let mut app = test_app();
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            install_control_sources(&mut app, resolver);
            tick(&mut app);
            (get_thrust_input(&mut app), get_steering_input(&mut app))
        };

        let shipped =
            resolver_from_shipped_npc_hull(include_str!("../assets/entities/pirate_raider.toml"));

        // The same hull as it behaved before #800: coarse helm on AI, the two
        // axes undeclared and therefore Human by default.
        let mut pre_800 = shipped.clone();
        pre_800.set(
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Human,
        );
        pre_800.set(
            crate::system_registry::helm_steering_system_id(),
            ControlSource::Human,
        );

        assert_eq!(
            run(&shipped),
            run(&pre_800),
            "declaring helm-thrust/helm-steering must not change AI helm behaviour on a \
             shipped hull: the per-axis path has to reproduce the monolith's intent"
        );
    }

    /// AC3/AC4 on the shipped-hull shape. All three helm systems run for this
    /// ship, so `operate_helm_ai` must yield the `AiMemory` commit to
    /// `ai_helm_steering` (the last to run). If it commits as well, the patrol
    /// waypoint advances twice in one tick and the ship skips a leg — the
    /// double-commit the old coarse gate used to make impossible.
    #[test]
    fn shipped_hull_helm_ai_memory_commits_exactly_once() {
        let mut app = test_app();
        stacked_patrol_world(&mut app);
        let resolver =
            resolver_from_shipped_npc_hull(include_str!("../assets/entities/pirate_raider.toml"));
        install_control_sources(&mut app, &resolver);

        tick(&mut app);

        assert_eq!(
            get_waypoint_index(&mut app),
            1,
            "on a shipped hull all three helm-AI systems run; exactly one must commit \
             AiMemory (2 = operate_helm_ai double-committed against ai_helm_steering, \
             0 = nobody committed)"
        );
    }

    /// AC3 on the shipped-hull shape: the Weapons->Helm arc-bearing bias (#677)
    /// must survive the move to the per-axis path. Before #800 the bias reached
    /// shipped hulls via `operate_helm_ai`; now `ai_helm_steering` owns steering
    /// there and has to fold it in instead. Nothing else pins that on a real
    /// hull's control sources.
    ///
    /// Note this does *not* pin the monolith's arc-bearing stand-down: both
    /// systems compute the same bias from the same inputs, so calling it twice
    /// is currently unobservable. See the comment at that call site.
    #[test]
    fn shipped_hull_helm_ai_folds_pending_arc_bearing_request_into_steering() {
        let mut app = test_app();
        // Destroy target directly ahead and far away, so the baseline pursuit
        // steering (before any arc-bearing bias) is ~0.
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -1000.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);

        // A separate hostile well off to starboard is the arc-bearing request
        // target — distinct from the Destroy pursuit target, so any steering
        // bias can only be attributed to the pending request.
        let bearing_uuid = uuid::Uuid::new_v4();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(bearing_uuid.to_string()),
            crate::entities::spawner::EntityName("Bearing Contact".into()),
            Transform::from_xyz(200.0, 0.0, -1.0),
        ));
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest(Some(bearing_uuid)));

        // Shipped-hull sources: coarse + both axes on AI.
        let resolver =
            resolver_from_shipped_npc_hull(include_str!("../assets/entities/pirate_raider.toml"));
        install_control_sources(&mut app, &resolver);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.steering.abs() > 0.01,
            "ai_helm_steering owns steering on a shipped hull, so it must be the one to \
             fold in the pending arc-bearing request; operate_helm_ai must not consume \
             the request out from under it. got {last:?}"
        );
    }

    /// AC1: `ai_helm_thrust` writes `ThrustInput` when its own system is
    /// AI-operated and the coarse helm is not.
    #[test]
    fn ai_helm_thrust_writes_thrust_intent() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert!(
            get_thrust_input(&mut app) > 0.0,
            "ai_helm_thrust must throttle up toward a Reach anchor"
        );
    }

    /// AC1: the fine gate is real — helm-thrust left Human means no AI write,
    /// even with a live Helm objective on the blackboard.
    #[test]
    fn ai_helm_thrust_does_not_write_when_its_system_is_human() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        // Coarse helm human, helm-thrust left at its Human default.
        set_helm_control_source(&mut app, ControlSource::Human);

        tick(&mut app);

        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "helm-thrust under human control must not be written by ai_helm_thrust"
        );
    }

    /// AC2: `ai_helm_steering` writes `SteeringInput`, steering toward the
    /// selected waypoint. The anchor sits to the right of a ship at the origin
    /// facing yaw 0, so steering must be positive.
    #[test]
    fn ai_helm_steering_writes_steering_intent_toward_waypoint() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert!(
            get_steering_input(&mut app) > 0.0,
            "ai_helm_steering must steer toward an anchor off the starboard bow"
        );
    }

    /// AC2: the fine gate is real for the steering axis too.
    #[test]
    fn ai_helm_steering_does_not_write_when_its_system_is_human() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Human);

        tick(&mut app);

        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "helm-steering under human control must not be written by ai_helm_steering"
        );
    }

    /// The axes are genuinely independent: automating only the throttle must
    /// leave steering alone, which is the whole point of the per-axis split.
    #[test]
    fn per_axis_gates_are_independent() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Human);
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );
        // A stale channel-3 Navigation handoff. `operate_helm` clears this the
        // moment a local Helm objective resolves (which `Reach` does), so it is
        // a direct probe for "did the AiMemory mutation get committed?".
        set_ship_nav_goal(&mut app, Some([-500.0, -500.0]));

        tick(&mut app);

        assert!(
            get_thrust_input(&mut app) > 0.0,
            "throttle axis is AI-operated → must be written"
        );
        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "steering axis is still human → must be untouched"
        );
        // Independent axes must not mean a half-dead AI: the one system that
        // does run still has to commit AiMemory, or waypoint advance and target
        // selection freeze while the throttle happily keeps writing. See
        // `ai_helm_thrust_commits_memory_when_steering_is_human` for the
        // waypoint half of the same rule.
        assert_eq!(
            get_ship_nav_goal(&mut app),
            None,
            "the sole running per-axis system must commit AiMemory, not discard \
             it into a scratch clone; a surviving nav_goal means frozen memory"
        );
    }

    /// Read this ship's committed patrol waypoint index out of `ShipAiMemory`.
    fn get_waypoint_index(app: &mut App) -> usize {
        let ship = find_ship_entity(app);
        app.world()
            .entity(ship)
            .get::<crate::ai_plugin::ShipAiMemory>()
            .expect("expected ShipAiMemory")
            .0
            .waypoint_index
    }

    /// A patrol whose every waypoint sits on top of the ship, so `helm_patrol`
    /// takes its "arrived → advance" branch on each call it is given. That
    /// makes `waypoint_index` a direct counter of `AiMemory` commits: 0 means
    /// nothing committed, 1 means exactly one commit, 2 means it committed
    /// twice. Three waypoints (not two) so a double-advance reads as `2` rather
    /// than wrapping back around to `0` and aliasing with the frozen case.
    fn stacked_patrol_world(app: &mut App) {
        let mut cfg = crate::world::config::WorldConfig::default();
        for wp in ["wp-a", "wp-b", "wp-c"] {
            cfg.anchors.insert(wp.into(), [0.0, 0.0, 0.0]);
        }
        app.insert_resource(cfg);
        set_ship_blackboard_objectives(
            app,
            vec![patrol_scored_objective(vec!["wp-a", "wp-b", "wp-c"], 20.0)],
        );
    }

    /// Regression (issue #701 review, finding 2): in the thrust-AI /
    /// steering-human config `ai_helm_steering` never runs, so if it were still
    /// the *sole* committer of `AiMemory` nothing would commit — `waypoint_index`
    /// would freeze at 0 forever and the throttle would target waypoint 0 for
    /// the rest of the mission. `ai_helm_thrust` must therefore commit when it
    /// is the only per-axis system running.
    #[test]
    fn ai_helm_thrust_commits_memory_when_steering_is_human() {
        let mut app = test_app();
        stacked_patrol_world(&mut app);
        set_helm_control_source(&mut app, ControlSource::Human);
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );

        tick(&mut app);

        assert_eq!(
            get_waypoint_index(&mut app),
            1,
            "ai_helm_thrust is the only per-axis system running, so it must \
             commit AiMemory exactly once; 0 = frozen memory (nothing committed)"
        );
    }

    /// The mirror of the above: steering-AI / thrust-human commits too. Pins
    /// that the two mixed configs behave the same way, rather than one
    /// advancing and one freezing.
    #[test]
    fn ai_helm_steering_commits_memory_when_thrust_is_human() {
        let mut app = test_app();
        stacked_patrol_world(&mut app);
        set_helm_control_source(&mut app, ControlSource::Human);
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_steering_system_id(),
            ControlSource::Ai,
        );

        tick(&mut app);

        assert_eq!(
            get_waypoint_index(&mut app),
            1,
            "ai_helm_steering is the only per-axis system running, so it must \
             commit AiMemory exactly once"
        );
    }

    /// The both-axes-AI case: `ai_helm_thrust` and `ai_helm_steering` both call
    /// `operate_helm`, but exactly one of them may commit the resulting
    /// `AiMemory`. Two commits would double-advance the patrol waypoint,
    /// silently skipping one every tick.
    #[test]
    fn helm_ai_memory_commits_exactly_once() {
        let mut app = test_app();
        stacked_patrol_world(&mut app);
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert_eq!(
            get_waypoint_index(&mut app),
            1,
            "both per-axis systems run, but AiMemory must commit exactly once; \
             2 = double-advance (both committed), 0 = frozen (neither did)"
        );
    }

    /// The coarse path is untouched by the commit rule: `operate_helm_ai` runs
    /// alone and commits once, exactly as before the per-axis split.
    #[test]
    fn coarse_helm_ai_commits_memory_exactly_once() {
        let mut app = test_app();
        stacked_patrol_world(&mut app);
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        assert_eq!(
            get_waypoint_index(&mut app),
            1,
            "coarse operate_helm_ai must still commit AiMemory exactly once"
        );
    }

    /// Regression (issue #701 review, finding 1): `ai_helm_thrust` and
    /// `ai_helm_steering` write one `LastHelmInput` field each, and
    /// `publish_joystick_to_engines` reads both as a pair. Unless it is ordered
    /// after *both* writers it can interleave between them and publish this
    /// tick's AI throttle next to the stale human steering still sitting in
    /// `LastHelmInput` — a torn pair that lands in `HelmEngineBlackboard`, i.e.
    /// on the player's engine gauge. Which half tears is decided by Bevy's
    /// arbitrary intra-set order, so this pins the published pair against the
    /// stale value rather than against a lucky schedule.
    #[test]
    fn helm_ai_last_input_pair_is_not_torn() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        // Off the starboard bow → the AI wants positive thrust AND positive
        // steering, so both differ in sign from the stale human stick below.
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);
        // Stale human stick, hard astern and hard to port, left over from
        // before the axes were handed to the AI.
        set_last_helm_input(
            &mut app,
            LastHelmInput {
                thrust: -0.9,
                steering: -0.9,
                lateral: 0.0,
            },
        );

        tick(&mut app);

        let ai_thrust = get_thrust_input(&mut app);
        let ai_steering = get_steering_input(&mut app);
        assert!(
            ai_thrust > 0.0 && ai_steering > 0.0,
            "precondition: the AI must actually want to move, else there is no \
             stale value to tear against; got thrust={ai_thrust} steering={ai_steering}"
        );

        let queue = app.world().resource::<InterSystemQueue>();
        let port_id = crate::system_registry::helm_engine_port_system_id();
        let msgs: Vec<_> = queue.for_target(port_id.0.as_str()).collect();
        assert!(
            !msgs.is_empty(),
            "expected a JoystickState message for helm-engine-port"
        );

        for msg in &msgs {
            let InterSystemPayload::JoystickState { thrust, steering } = &msg.payload else {
                panic!("expected JoystickState payload");
            };
            assert_eq!(
                (*thrust, *steering),
                (ai_thrust, ai_steering),
                "published joystick pair must be the AI's whole decision. A \
                 mismatch on one axis only means the pair tore: \
                 publish_joystick_to_engines interleaved between ai_helm_thrust \
                 and ai_helm_steering and picked up the stale human -0.9"
            );
        }
    }

    /// AC3 (Retreat consumer): with a Retreat directive top-scored, steering
    /// must resolve the named anchor and steer toward it. `operate_helm`'s
    /// Retreat arm is what does the work — this pins that `ai_helm_steering`
    /// actually routes it through to `SteeringInput`.
    #[test]
    fn ai_helm_steering_retreats_toward_anchor() {
        let mut app = test_app();
        let anchor = "rally-point";
        set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective(anchor, 90.0)]);
        // Rally point off the starboard bow → positive steering.
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert!(
            get_steering_input(&mut app) > 0.0,
            "Retreat must steer toward the named rally anchor"
        );
        assert!(
            get_thrust_input(&mut app) > 0.0,
            "Retreat must also throttle up to actually leave"
        );
    }

    /// AC3 (Retreat consumer, synthetic hull-triggered case): the Retreat that
    /// `aggregate_doctrine_blackboards` injects below the entity's TOML
    /// `[behaviour] retreat_hull_threshold` carries an *empty* anchor, so the
    /// consumer must fall back to the ship's home position rather than idling.
    #[test]
    fn ai_helm_steering_retreat_falls_back_to_home_position() {
        let mut app = test_app();
        // Empty anchor + no anchors in the world config → home_position is the
        // only thing that can resolve. Home is off the starboard bow.
        set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective("", 90.0)]);
        app.insert_resource(crate::world::config::WorldConfig::default());
        set_ship_home_position(&mut app, [100.0, 0.0, 0.0]);
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert!(
            get_steering_input(&mut app) > 0.0,
            "synthetic empty-anchor Retreat must steer toward home_position"
        );
    }

    /// A top-scored Retreat wins over a lower-scored Helm objective pointing
    /// the other way, so the ship actually breaks off rather than pressing on.
    ///
    /// The pool is listed descending by score because that is the contract
    /// every producer honours (`score_doctrine_pool` and
    /// `ObjectiveManager::scored_pool_with_boost` both sort before publishing)
    /// and what `operate_helm` consumes — it takes the first Helm-relevant
    /// entry rather than scanning for the maximum.
    #[test]
    fn ai_helm_steering_retreat_outranks_lower_priority_objective() {
        let mut app = test_app();
        let mut cfg = crate::world::config::WorldConfig::default();
        // Rally to starboard, patrol waypoint to port.
        cfg.anchors.insert("rally".into(), [100.0, 0.0, 0.0]);
        cfg.anchors.insert("wp".into(), [-100.0, 0.0, 0.0]);
        app.insert_resource(cfg);
        set_ship_blackboard_objectives(
            &mut app,
            vec![
                retreat_scored_objective("rally", 90.0),
                patrol_scored_objective(vec!["wp"], 10.0),
            ],
        );
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert!(
            get_steering_input(&mut app) > 0.0,
            "top-scored Retreat must win over the lower-scored patrol waypoint"
        );
    }

    /// AC4: both per-axis systems are `AiHighFidelity`-scoped. A demoted ship
    /// (marker removed) must not be driven by them.
    #[test]
    fn per_axis_helm_ai_is_scoped_to_high_fidelity() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        // Demote: drop the marker, keep the intent components so a write would
        // still be observable if the scoping were missing.
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ai_plugin::AiHighFidelity>();

        tick(&mut app);

        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "ai_helm_thrust must not touch a ship without AiHighFidelity"
        );
        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "ai_helm_steering must not touch a ship without AiHighFidelity"
        );
    }

    /// AC4 / the double-write guard, re-expressed for #800.
    ///
    /// Before #800 the invariant was *per ship*: the coarse half of the
    /// per-axis dual gate meant either the monolith ran or the per-axis systems
    /// did, never both. #800 declares `helm-thrust`/`helm-steering` on every
    /// shipped hull and drops that half, so all three systems now run for the
    /// same ship in the same tick and per-ship exclusion is simply false.
    ///
    /// The invariant it becomes is *per component*: each intent component has
    /// exactly one writer per tick, because `operate_helm_ai` stands down from
    /// any axis whose own system owns it. That is what actually rules out the
    /// #697 failure mode (arbitrary intra-set ordering deciding the outcome),
    /// so that is what this test pins — plus the `AiMemory` commit, which is
    /// the other thing the dropped gate used to keep exactly-once.
    ///
    /// Checked directly against the policy resolver, because these are
    /// properties of the gates themselves and hold for every possible
    /// control-source combination — not just the ones a test happens to tick.
    #[test]
    fn helm_ai_writers_are_mutually_exclusive() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let coarse = crate::system_registry::helm_system_id();
        let thrust = crate::system_registry::helm_thrust_system_id();
        let steering = crate::system_registry::helm_steering_system_id();

        // Exhaust every (coarse, thrust, steering) control-source combination.
        let all = [
            ControlSource::Human,
            ControlSource::Ai,
            ControlSource::Offline,
        ];
        let mut saw_all_three_running = false;

        for c in all {
            for t in all {
                for s in all {
                    let mut r = ControlSourceResolver::new();
                    r.set(coarse.clone(), c);
                    r.set(thrust.clone(), t);
                    r.set(steering.clone(), s);

                    // The gate each system actually applies. The per-axis
                    // systems gate on their own axis alone (#800).
                    let cc = r.policy_for(&coarse).operate_ai;
                    let tt = r.policy_for(&thrust).operate_ai;
                    let ss = r.policy_for(&steering).operate_ai;

                    let monolith_runs = cc;
                    let thrust_runs = tt;
                    let steering_runs = ss;

                    // ── Per-component exclusion ──────────────────────────────
                    // `operate_helm_ai` stands down from an axis owned by that
                    // axis's own system, so it writes each only when the fine
                    // system does not.
                    let monolith_writes_thrust = monolith_runs && !tt;
                    let monolith_writes_steering = monolith_runs && !ss;

                    assert!(
                        !(monolith_writes_thrust && thrust_runs),
                        "two writers of ThrustInput for coarse={c:?} thrust={t:?} steering={s:?}"
                    );
                    assert!(
                        !(monolith_writes_steering && steering_runs),
                        "two writers of SteeringInput for coarse={c:?} thrust={t:?} steering={s:?}"
                    );

                    // Coverage is preserved: whenever the ship's helm is under
                    // AI at all, both axes still get written by someone.
                    if monolith_runs {
                        assert!(
                            monolith_writes_thrust || thrust_runs,
                            "ThrustInput lost its writer for coarse={c:?} thrust={t:?}"
                        );
                        assert!(
                            monolith_writes_steering || steering_runs,
                            "SteeringInput lost its writer for coarse={c:?} steering={s:?}"
                        );
                    }

                    // ── AiMemory commits exactly once ────────────────────────
                    // "The last helm-AI system that runs owns the commit", with
                    // registered order monolith → thrust → steering.
                    let monolith_commits = monolith_runs && !tt && !ss;
                    let thrust_commits = thrust_runs && !ss;
                    let steering_commits = steering_runs;

                    let commits = [monolith_commits, thrust_commits, steering_commits]
                        .iter()
                        .filter(|x| **x)
                        .count();
                    let any_runs = monolith_runs || thrust_runs || steering_runs;
                    assert_eq!(
                        commits,
                        usize::from(any_runs),
                        "AiMemory must commit exactly once when any helm AI runs (and never \
                         otherwise): coarse={c:?} thrust={t:?} steering={s:?}"
                    );

                    if monolith_runs && thrust_runs && steering_runs {
                        saw_all_three_running = true;
                    }
                }
            }
        }

        // The shipped-hull shape (all three declared, station backfilled to AI)
        // must be inside the space this test covers — that combination was
        // unreachable under the old per-ship gate and is the whole point of #800.
        assert!(
            saw_all_three_running,
            "the shipped-hull all-AI combination must be covered"
        );
    }

    /// AC5, observed end-to-end: with the coarse helm on AI, the monolith is
    /// the writer and the per-axis systems stand down — so turning their fine
    /// systems on as well changes nothing about the resulting intent.
    #[test]
    fn coarse_helm_ai_result_is_unchanged_by_per_axis_systems() {
        let anchor = "station-alpha";

        let coarse_only = {
            let mut app = test_app();
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            set_helm_control_source(&mut app, ControlSource::Ai);
            tick(&mut app);
            (get_thrust_input(&mut app), get_steering_input(&mut app))
        };

        let coarse_plus_fine = {
            let mut app = test_app();
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            set_helm_control_source(&mut app, ControlSource::Ai);
            set_fine_control_source(
                &mut app,
                crate::system_registry::helm_thrust_system_id(),
                ControlSource::Ai,
            );
            set_fine_control_source(
                &mut app,
                crate::system_registry::helm_steering_system_id(),
                ControlSource::Ai,
            );
            tick(&mut app);
            (get_thrust_input(&mut app), get_steering_input(&mut app))
        };

        assert_eq!(
            coarse_only, coarse_plus_fine,
            "with the coarse helm on AI the monolith owns the write; the per-axis \
             systems must stand down and leave the result bit-identical"
        );
        assert!(
            coarse_only.0 > 0.0,
            "sanity: the coarse path must actually have driven the ship"
        );
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
        let vb = ViewscreenBlackboard {
            scored_objectives: objectives,
            ..Default::default()
        };
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
    fn helm_ai_navigates_toward_reach_objective() {
        let mut app = test_app();
        // Place anchor 100 units ahead (positive X) — ship starts at origin.
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
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
    fn helm_ai_navigates_toward_retreat_objective() {
        let mut app = test_app();
        // Place anchor 100 units ahead (positive X) — ship starts at origin.
        let anchor = "rally-point";
        set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "AI helm must apply positive thrust toward Retreat anchor; got {last:?}"
        );
    }

    #[test]
    fn helm_ai_patrols_from_viewscreen_objective() {
        let mut app = test_app();
        let anchor = "starbase_patrol_east";
        set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec![anchor], 20.0)]);
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
    fn helm_ai_pursues_named_destroy_objective() {
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
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "AI helm must pursue named Destroy objective target; got {last:?}"
        );
    }

    // ── #674: helm radar gating ─────────────────────────────────────────────

    #[test]
    fn helm_ai_ignores_hostile_beyond_radar_range() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        // Hostile is 100 units away.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
        app.insert_resource(runtime);
        // Radar range (10.0) is far shorter than the hostile's distance (100.0).
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 10.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert_eq!(
            last,
            LastHelmInput::default(),
            "hostile beyond helm radar range must not be perceived; pursuit should fall through to idle, got {last:?}"
        );
    }

    #[test]
    fn helm_ai_pursues_hostile_within_radar_range() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        // Hostile is 100 units away.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
        app.insert_resource(runtime);
        // Radar range (500.0) comfortably covers the hostile's distance (100.0).
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 500.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "hostile within helm radar range must still be pursued as before; got {last:?}"
        );
    }

    // ── #677: Weapons->Helm arc-bearing request ──────────────────────────────

    #[test]
    fn helm_ai_folds_pending_arc_bearing_request_into_steering() {
        let mut app = test_app();
        // Destroy target directly ahead and far away, so the baseline
        // pursuit steering (before any arc-bearing bias) is ~0.
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -1000.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // A separate hostile well off to starboard is the Weapons arc-bearing
        // request target — distinct from the Destroy pursuit target, so any
        // steering bias can only be attributed to the pending request.
        let bearing_uuid = uuid::Uuid::new_v4();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(bearing_uuid.to_string()),
            crate::entities::spawner::EntityName("Bearing Contact".into()),
            Transform::from_xyz(200.0, 0.0, -1.0),
        ));
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest(Some(bearing_uuid)));

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "pending arc-bearing request must not disturb thrust/range-holding; got {last:?}"
        );
        assert!(
            last.steering.abs() > 0.01,
            "pending arc-bearing request must bias steering toward the requested bearing; got {last:?}"
        );
    }

    #[test]
    fn helm_ai_clears_arc_bearing_request_once_facing_already_satisfies_the_arc() {
        let mut app = test_app();
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -1000.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Bearing contact is directly ahead of the ship's starting facing
        // (yaw=0, forward=-Z) — i.e. the ship is already oriented such that a
        // wide-arc fore bank already bears on it.
        let bearing_uuid = uuid::Uuid::new_v4();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(bearing_uuid.to_string()),
            crate::entities::spawner::EntityName("Bearing Contact".into()),
            Transform::from_xyz(0.0, 0.0, -200.0),
        ));
        let ship = find_ship_entity(&mut app);
        app.world_mut().entity_mut(ship).insert((
            PendingArcBearingRequest(Some(bearing_uuid)),
            crate::weapons_plugin::PhaserCombatConfigResource(
                crate::entity_config::PhaserCombatConfig {
                    banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 30.0,
                        auto_arc_deg: 30.0,
                        beam_range: 50.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 3.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    }],
                },
            ),
        ));

        tick(&mut app);

        let pending = app
            .world()
            .get::<PendingArcBearingRequest>(ship)
            .expect("ship must carry PendingArcBearingRequest");
        assert_eq!(
            pending.0, None,
            "a request must clear once the ship's own facing already brings a bank's arc onto the target, \
             not persist indefinitely after being satisfied"
        );
    }

    #[test]
    fn helm_ai_clears_arc_bearing_request_when_target_not_visible() {
        let mut app = test_app();
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -1000.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Pending bearing references an entity that was never spawned — it
        // cannot be visible in the world view.
        let stale_uuid = uuid::Uuid::new_v4();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest(Some(stale_uuid)));

        tick(&mut app);

        let pending = app
            .world()
            .get::<PendingArcBearingRequest>(ship)
            .expect("ship must carry PendingArcBearingRequest");
        assert_eq!(
            pending.0, None,
            "a pending request for a no-longer-visible target must be cleared, not stuck forever"
        );
    }

    #[test]
    fn helm_ai_does_nothing_when_helm_human() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
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
    fn helm_ai_stays_zero_when_destroy_target_missing() {
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

    /// Regression (issue #696 review, finding 2): `[behaviour]
    /// waypoint_arrival_radius` is authored per entity template in TOML and
    /// read by the cursor evaluator at every LOD. The high-LOD helm's own
    /// turn-at-waypoint decision must agree with it rather than hardcoding
    /// `WAYPOINT_ARRIVAL_RADIUS` — otherwise a designer's widened radius is
    /// honoured for triggers but ignored for steering.
    #[test]
    fn high_lod_helm_honours_toml_authored_waypoint_arrival_radius() {
        fn patrol_app(arrival_radius: Option<f32>) -> App {
            let mut app = test_app();
            // wp0 sits 100 units out — inside a 150 radius, outside the
            // default 20.
            let mut cfg = crate::world::config::WorldConfig::default();
            cfg.anchors.insert("wp0".into(), [100.0, 0.0, 0.0]);
            cfg.anchors.insert("wp1".into(), [900.0, 0.0, 0.0]);
            set_ship_blackboard_objectives(
                &mut app,
                vec![patrol_scored_objective(vec!["wp0", "wp1"], 20.0)],
            );
            app.insert_resource(cfg);
            set_helm_control_source(&mut app, ControlSource::Ai);
            if let Some(radius) = arrival_radius {
                let ship = find_ship_entity(&mut app);
                app.world_mut().entity_mut(ship).insert(
                    crate::entities::spawner::BehaviourSection(
                        crate::entity_config::BehaviourConfig {
                            waypoint_arrival_radius: radius,
                            ..Default::default()
                        },
                    ),
                );
            }
            tick(&mut app);
            app
        }

        fn helm_waypoint_index(app: &mut App) -> usize {
            app.world_mut()
                .query::<&crate::ai_plugin::ShipAiMemory>()
                .single(app.world())
                .expect("ship must carry ShipAiMemory")
                .0
                .waypoint_index
        }

        assert_eq!(
            helm_waypoint_index(&mut patrol_app(None)),
            0,
            "with the default arrival radius the helm is still 100 units short of wp0"
        );
        assert_eq!(
            helm_waypoint_index(&mut patrol_app(Some(150.0))),
            1,
            "a TOML-widened arrival radius must move the high-LOD helm on to wp1, \
             the same call the cursor evaluator makes"
        );
    }

    // ── TOML-authored avoidance tuning (AGENTS.md rule 11) ────────────────
    //
    // `[behaviour] avoidance_buffer` / `avoidance_look_ahead_secs` are
    // declared with serde defaults, so a designer can author them per entity
    // template. Three sites feed them to the pure AI: `helm_ai_decision`
    // (steering/thrust), `operate_helm_ai` (lateral dodge) and the standalone
    // `operate_lateral_thrust_ai`. Each test below pins one of the tuning
    // fields by choosing a geometry that the constant and the authored value
    // disagree about, so reverting a site to `crate::ai::AVOIDANCE_*` turns
    // the assertion red.

    /// Seeds a `WorldSnapshot` holding a single stationary obstacle, so the
    /// avoidance maths has exactly one threat to reason about and the
    /// assertions below can attribute any lateral dodge to it alone.
    fn snapshot_with_obstacle(app: &mut App, position: [f32; 3], radius: f32) {
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::new_v4(),
                name: Some("rock".into()),
                position,
                faction: None,
                shields: None,
                hull_fraction: None,
                // `None` yaw keeps the obstacle un-projected, so
                // `avoidance_look_ahead_secs` only moves *our* projected
                // position — one variable, not two.
                yaw: None,
                radius,
                forward_speed: 0.0,
            }],
        });
    }

    fn set_behaviour_section(app: &mut App, behaviour: crate::entity_config::BehaviourConfig) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::entities::spawner::BehaviourSection(behaviour));
    }

    fn lateral_intent(app: &mut App) -> f32 {
        app.world_mut()
            .query::<&LateralThrustInput>()
            .single(app.world())
            .expect("ship must carry LateralThrustInput")
            .0
    }

    /// `operate_lateral_thrust_ai` runs when lateral thrust is AI-operated but
    /// the helm proper is human — the "Simplified" partial-automation rating.
    fn lateral_thrust_ai_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Human);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::system_registry::lateral_thrust_system_id(),
                    ControlSource::Ai,
                );
            }
        }
        set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
        if let Some(behaviour) = behaviour {
            set_behaviour_section(&mut app, behaviour);
        }
        app
    }

    /// A wider TOML `avoidance_buffer` must widen the dodge radius of the
    /// standalone lateral-thrust AI. The obstacle sits 40 units off the bow:
    /// outside the default 5-unit buffer (radius 0+1+5 = 6), inside an
    /// authored 60 (radius 0+1+60 = 61).
    #[test]
    fn lateral_thrust_ai_honours_toml_authored_avoidance_buffer() {
        // Stationary ship, so `avoidance_look_ahead_secs` scales a zero
        // velocity and cannot influence the result — isolating the buffer.
        let obstacle = [4.0, 0.0, -40.0];

        let mut default_app = lateral_thrust_ai_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        // Two ticks: Bevy's first `update()` reports a zero delta, which does
        // not advance the 30 Hz `AiLateralThrustTimer` gate.
        tick_twice(&mut default_app);
        assert_eq!(
            lateral_intent(&mut default_app),
            0.0,
            "with the default 5-unit buffer a 40-unit-distant obstacle is not a threat"
        );

        let mut authored_app = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick_twice(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "a TOML-authored 60-unit avoidance_buffer must bring the same obstacle \
             inside the dodge radius; got no lateral thrust, so the system is still \
             reading crate::ai::AVOIDANCE_BUFFER"
        );
    }

    /// A longer TOML `avoidance_look_ahead_secs` must project the ship further
    /// forward before testing for a threat. At 10 u/s the default 3 s horizon
    /// stops 70 units short of the obstacle (well outside the 6-unit dodge
    /// radius); an authored 10 s lands the projection right on top of it.
    #[test]
    fn lateral_thrust_ai_honours_toml_authored_avoidance_look_ahead() {
        // Forward at yaw 0 is -Z, so the obstacle sits 100 units down -Z with
        // a 2-unit lateral offset to give the dodge a defined sign.
        let obstacle = [2.0, 0.0, -100.0];

        fn moving_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
            let mut app = lateral_thrust_ai_app(behaviour);
            let mut physics = get_ship_physics(&mut app);
            physics.forward_speed = 10.0;
            physics.yaw = 0.0;
            set_ship_physics(&mut app, physics);
            app
        }

        let mut default_app = moving_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        // See the buffer test: the first `update()` has a zero delta.
        tick_twice(&mut default_app);
        assert_eq!(
            lateral_intent(&mut default_app),
            0.0,
            "the default 3 s horizon projects only 30 units ahead — the obstacle at \
             100 is not yet a threat"
        );

        let mut authored_app = moving_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_look_ahead_secs: 10.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick_twice(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "a TOML-authored 10 s look-ahead projects 100 units ahead, onto the \
             obstacle; got no lateral thrust, so the system is still reading \
             crate::ai::AVOIDANCE_LOOK_AHEAD_SECS"
        );
    }

    /// Drives the `helm_ai_decision` → `operate_helm` → `avoidance_steering`
    /// path: a Reach anchor dead ahead down -Z, so the base steer sits in the
    /// deadband at zero and any nonzero `SteeringInput` is avoidance and
    /// nothing else. `avoidance_steering` ignores ships slower than
    /// `AVOIDANCE_MIN_SPEED`, hence the explicit forward speed.
    fn helm_ai_steering_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        app.insert_resource(world_config_with_anchor("far-ahead", [0.0, 0.0, -900.0]));
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective("far-ahead", 8.0)]);
        if let Some(behaviour) = behaviour {
            set_behaviour_section(&mut app, behaviour);
        }
        let mut physics = get_ship_physics(&mut app);
        physics.forward_speed = 10.0;
        physics.yaw = 0.0;
        set_ship_physics(&mut app, physics);
        app
    }

    fn steering_intent(app: &mut App) -> f32 {
        app.world_mut()
            .query::<&SteeringInput>()
            .single(app.world())
            .expect("ship must carry SteeringInput")
            .0
    }

    /// `helm_ai_decision` feeds `avoidance_buffer` to `operate_helm`, where it
    /// widens the radius `avoidance_steering` treats as a threat.
    #[test]
    fn helm_ai_decision_honours_toml_authored_avoidance_buffer() {
        // Projected 30 units ahead (10 u/s × the default 3 s), the obstacle is
        // ~10.8 units away: outside the default 6-unit dodge radius
        // (0 + 1 + 5), inside an authored 61 (0 + 1 + 60).
        let obstacle = [4.0, 0.0, -40.0];

        let mut default_app = helm_ai_steering_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick(&mut default_app);
        assert_eq!(
            steering_intent(&mut default_app),
            0.0,
            "with the default 5-unit buffer the obstacle is no threat and the anchor \
             is dead ahead, so steering stays in the deadband"
        );

        let mut authored_app = helm_ai_steering_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick(&mut authored_app);
        assert!(
            steering_intent(&mut authored_app).abs() > 0.0,
            "a TOML-authored 60-unit avoidance_buffer must make the helm steer around \
             the obstacle; got no steering, so helm_ai_decision is still passing \
             crate::ai::AVOIDANCE_BUFFER"
        );
    }

    /// `helm_ai_decision` feeds `avoidance_look_ahead_secs` to `operate_helm`,
    /// where it sets how far forward `avoidance_steering` projects the ship
    /// before testing for a threat.
    #[test]
    fn helm_ai_decision_honours_toml_authored_avoidance_look_ahead() {
        // At 10 u/s the default 3 s horizon projects 30 units ahead, leaving
        // the obstacle ~70 units off; a 10 s horizon projects 100 units, right
        // onto it.
        let obstacle = [2.0, 0.0, -100.0];

        let mut default_app = helm_ai_steering_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick(&mut default_app);
        assert_eq!(
            steering_intent(&mut default_app),
            0.0,
            "the default 3 s horizon does not reach the obstacle at 100 units"
        );

        let mut authored_app = helm_ai_steering_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_look_ahead_secs: 10.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick(&mut authored_app);
        assert!(
            steering_intent(&mut authored_app).abs() > 0.0,
            "a TOML-authored 10 s look-ahead must bring the obstacle into the helm's \
             projected path; got no steering, so helm_ai_decision is still passing \
             crate::ai::AVOIDANCE_LOOK_AHEAD_SECS"
        );
    }

    /// Drives the full-AI helm's own dodge: `operate_helm_ai` calls
    /// `operate_lateral_thrust` directly. Forward speed is not optional
    /// scaffolding — `operate_lateral_thrust` projects the ship by
    /// `forward_speed * avoidance_look_ahead_secs`, so a stationary ship
    /// collapses that projection onto its own position and makes the look-ahead
    /// term unobservable no matter what value is passed.
    fn helm_ai_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        let mut cfg = crate::world::config::WorldConfig::default();
        // Waypoint far down -Z keeps the helm driving straight ahead, so
        // the lateral axis reflects avoidance alone.
        cfg.anchors.insert("wp0".into(), [0.0, 0.0, -900.0]);
        app.insert_resource(cfg);
        set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
        if let Some(behaviour) = behaviour {
            set_behaviour_section(&mut app, behaviour);
        }
        let mut physics = get_ship_physics(&mut app);
        physics.forward_speed = 10.0;
        physics.yaw = 0.0;
        set_ship_physics(&mut app, physics);
        app
    }

    /// The same two tunables reach the pure AI a second way: `operate_helm_ai`
    /// calls `operate_lateral_thrust` directly for the full-AI helm. The dodge
    /// and the steering must agree about clearance, so this site must read the
    /// same TOML the steering does.
    #[test]
    fn operate_helm_ai_honours_toml_authored_avoidance_buffer() {
        // Projected 30 units ahead (10 u/s × the default 3 s), the obstacle is
        // ~10.8 units away: outside the default 6-unit dodge radius
        // (0 + 1 + 5), inside an authored 61 (0 + 1 + 60).
        let obstacle = [4.0, 0.0, -40.0];

        let mut default_app = helm_ai_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick(&mut default_app);
        assert_eq!(
            lateral_intent(&mut default_app),
            0.0,
            "with the default 5-unit buffer the obstacle sits ~10.8 units off the \
             projected path and is not a threat"
        );

        let mut authored_app = helm_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "operate_helm_ai must pass the TOML-authored avoidance_buffer to \
             operate_lateral_thrust, not crate::ai::AVOIDANCE_BUFFER"
        );
    }

    /// The sixth wired argument: `operate_helm_ai` must pass the TOML-authored
    /// `avoidance_look_ahead_secs` to `operate_lateral_thrust`, which uses it to
    /// project the ship forward before testing for a threat. Mirrors
    /// `lateral_thrust_ai_honours_toml_authored_avoidance_look_ahead`, but for
    /// the full-AI helm path rather than the standalone lateral-thrust system.
    #[test]
    fn operate_helm_ai_honours_toml_authored_avoidance_look_ahead() {
        // Forward at yaw 0 is -Z. At 10 u/s the default 3 s horizon projects
        // only 30 units ahead, leaving the obstacle ~70 units off; an authored
        // 10 s projects 100 units, landing 2 units from it — inside the default
        // 6-unit dodge radius (0 + 1 + 5), so the buffer is held constant and
        // the look-ahead is the only variable.
        let obstacle = [2.0, 0.0, -100.0];

        let mut default_app = helm_ai_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick(&mut default_app);
        assert_eq!(
            lateral_intent(&mut default_app),
            0.0,
            "the default 3 s horizon projects only 30 units ahead — the obstacle at \
             100 is not yet a threat"
        );

        let mut authored_app = helm_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_look_ahead_secs: 10.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "operate_helm_ai must pass the TOML-authored avoidance_look_ahead_secs to \
             operate_lateral_thrust, not crate::ai::AVOIDANCE_LOOK_AHEAD_SECS"
        );
    }

    /// `nav_handoff_speed` is the throttle the helm adopts for a Channel-3
    /// Navigation→Helm handoff. It is authored in `[behaviour]`, and the
    /// `crate::ai::NAV_HANDOFF_SPEED` fallback exists only for an entity with
    /// no `[behaviour]` section at all.
    #[test]
    fn helm_ai_honours_toml_authored_nav_handoff_speed() {
        fn nav_goal_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
            let mut app = test_app();
            set_helm_control_source(&mut app, ControlSource::Ai);
            // A Helm-relevant objective must exist (an empty pool makes
            // `operate_helm_ai` zero the intent and skip the decision
            // entirely), but it must not *resolve* — a Reach whose anchor is
            // absent from the WorldConfig yields `None`, so `operate_helm`
            // falls through to the nav_goal handoff, the only path that reads
            // `nav_handoff_speed`.
            set_ship_blackboard_objectives(
                &mut app,
                vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
            );
            if let Some(behaviour) = behaviour {
                set_behaviour_section(&mut app, behaviour);
            }
            let ship = find_ship_entity(&mut app);
            app.world_mut()
                .entity_mut(ship)
                .get_mut::<crate::ai_plugin::ShipAiMemory>()
                .expect("ship must carry ShipAiMemory")
                .0
                .nav_goal = Some([0.0, -900.0]);
            tick(&mut app);
            app
        }

        fn thrust(app: &mut App) -> f32 {
            app.world_mut()
                .query::<&ThrustInput>()
                .single(app.world())
                .expect("ship must carry ThrustInput")
                .0
        }

        assert!(
            (thrust(&mut nav_goal_app(None)) - crate::ai::NAV_HANDOFF_SPEED).abs() < 1e-6,
            "a ship with no [behaviour] section must fall back to NAV_HANDOFF_SPEED"
        );
        assert!(
            (thrust(&mut nav_goal_app(Some(
                crate::entity_config::BehaviourConfig {
                    nav_handoff_speed: 0.25,
                    ..Default::default()
                }
            ))) - 0.25)
                .abs()
                < 1e-6,
            "a TOML-authored nav_handoff_speed must be the throttle the helm adopts, \
             not crate::ai::NAV_HANDOFF_SPEED"
        );
    }

    /// Regression (issue #696 review, finding 2): Reach completion is the
    /// other site that judged arrival against the hardcoded constant.
    #[test]
    fn detect_reach_completion_honours_toml_authored_arrival_radius() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig};
        use crate::world::server::ObjectiveManagerRes;

        fn reach_app(arrival_radius: Option<f32>) -> App {
            let mut app = test_app();
            let anchor = "dock-mid";
            // 100 units out: inside a 150 radius, outside the default 20.
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            set_helm_control_source(&mut app, ControlSource::Ai);
            if let Some(radius) = arrival_radius {
                let ship = find_ship_entity(&mut app);
                app.world_mut().entity_mut(ship).insert(
                    crate::entities::spawner::BehaviourSection(
                        crate::entity_config::BehaviourConfig {
                            waypoint_arrival_radius: radius,
                            ..Default::default()
                        },
                    ),
                );
            }
            let mut mgr = ObjectiveManager::new();
            mgr.add_full(
                "reach-dock-mid",
                "Dock at Mid",
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
            app
        }

        fn status(app: &App) -> Option<crate::messages::ObjectiveStatus> {
            app.world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .into_iter()
                .find(|o| o.id == "reach-dock-mid")
                .map(|o| o.status)
        }

        assert_eq!(
            status(&reach_app(None)),
            Some(crate::messages::ObjectiveStatus::Active),
            "the default arrival radius must not count 100 units away as reached"
        );
        assert_eq!(
            status(&reach_app(Some(150.0))),
            Some(crate::messages::ObjectiveStatus::Completed),
            "a TOML-widened arrival radius must complete the Reach objective"
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
    fn all_backfill_helm_policy_gates_operate_ai() {
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
    fn backfill_runs_full_operate_helm_with_objectives() {
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

    #[test]
    fn backfill_helm_ai_caps_long_frame_yaw_step() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid),
            crate::entities::spawner::EntityName("enemy_fighter".into()),
            Transform::from_xyz(80.0, 0.0, 0.0),
        ));
        set_ship_blackboard_objectives(
            &mut app,
            vec![destroy_scored_objective("enemy_fighter", 60.0)],
        );
        set_helm_control_source(&mut app, ControlSource::Ai);

        let before = get_ship_physics(&mut app);
        tick(&mut app);
        let after = get_ship_physics(&mut app);

        let max_step = ShipPhysicsConfig::new().max_yaw_rate * HELM_AI_MAX_DT_SECS;
        let yaw_delta = (after.yaw - before.yaw).abs();
        assert!(
            yaw_delta <= max_step + 0.0001,
            "AI helm must not consume a long frame as one oversized yaw step; \
             yaw_delta={yaw_delta}, max_step={max_step}"
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
            AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
            SystemAffinity,
        };

        let fed_uuid = uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap();
        let harrow_uuid = uuid::Uuid::parse_str("cccccccc-3333-4333-8333-cccccccccccc").unwrap();
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
            directive: AiDirective::Destroy {
                target: String::new(),
            },
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
            0.6,
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
            0.6,
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
        let sources = q
            .single(app.world())
            .expect("Ship with ShipSystemControlSources");
        sources
            .0
            .policy_for(&crate::messages::SystemId(system_id.into()))
    }

    fn set_hp(app: &mut App, system_id: crate::messages::SystemId, hp: f32) {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut binding = app.world_mut().entity_mut(ship);
        let mut hull_component = binding
            .get_mut::<crate::entity_spawner::EntitySystemHull>()
            .unwrap();
        // Wipe then restore to exact HP.
        hull_component.0.apply_damage(1_000_000.0, &mut rand::rng());
        hull_component.0.restore(&system_id, hp);
    }

    #[test]
    fn disabled_console_gates_human_and_ai_input() {
        let mut app = test_app();
        // Helm console max_hp = 25. Disabled threshold = 25 % = 6.25 HP.
        // Set Helm to 5 HP (below disabled threshold) → Disabled tier.
        set_hp(&mut app, crate::messages::SystemId("helm".into()), 5.0);
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            !policy.accept_human_input,
            "Disabled console must not accept human input"
        );
        assert!(!policy.operate_ai, "Disabled console must not operate AI");
    }

    #[test]
    fn destroyed_console_gates_human_and_ai_input() {
        let mut app = test_app();
        // Wipe helm to 0 HP → Destroyed tier.
        set_hp(&mut app, crate::messages::SystemId("helm".into()), 0.0);
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            !policy.accept_human_input,
            "Destroyed console must not accept human input"
        );
        assert!(!policy.operate_ai, "Destroyed console must not operate AI");
    }

    #[test]
    fn restored_console_re_enables_input() {
        let mut app = test_app();
        // First disable helm.
        set_hp(&mut app, crate::messages::SystemId("helm".into()), 5.0);
        tick(&mut app);
        // Verify it is gated.
        assert!(!get_policy(&mut app, "helm").accept_human_input);

        // Now restore to operational HP.
        set_hp(&mut app, crate::messages::SystemId("helm".into()), 25.0);
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
        set_hp(&mut app, crate::messages::SystemId("helm".into()), 12.5);
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            policy.accept_human_input,
            "Damaged (but not Disabled) console must still accept human input"
        );
    }

    /// When helm is under AI control (`operate_ai = true`), `process_helm_inputs`
    /// must NOT admit stale human input over the AI's decision.
    ///
    /// Post-#695 `process_helm_inputs` no longer integrates physics at all —
    /// `integrate_ship_physics` is the sole helm-path writer. What this test
    /// pins is the *admission* skip: with helm AI-controlled, a stale non-zero
    /// `LastHelmInput` must not reach the intent components and therefore must
    /// not move the ship. (Before #695 this same setup guarded against a second
    /// `compute_physics` call at a different dt, which made the player ship
    /// move ~3× faster than AI-driven NPCs.)
    #[test]
    fn ai_controlled_helm_does_not_admit_stale_human_input() {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Set a non-zero last input so that if process_helm_inputs incorrectly
        // runs compute_physics it will produce a non-trivial displacement.
        set_last_helm_input(
            &mut app,
            LastHelmInput {
                thrust: 1.0,
                steering: 0.0,
                lateral: 0.0,
            },
        );

        // Snapshot physics before the tick.
        let before = get_ship_physics(&mut app);

        tick(&mut app);

        let after = get_ship_physics(&mut app);

        // operate_helm_ai has no objectives in this test (blackboard empty), so
        // it zeros the intent components. If process_helm_inputs admitted the
        // stale thrust=1.0 anyway, integrate_ship_physics would have moved the
        // ship.
        assert_eq!(
            after.x, before.x,
            "ShipPhysics.x must not advance when helm is AI-controlled: \
             process_helm_inputs must skip admission"
        );
        assert_eq!(
            after.forward_speed, before.forward_speed,
            "forward_speed must not change when process_helm_inputs skips admission"
        );
    }

    // ── integrate_ship_physics single-writer tests (issue #699) ───────────────

    /// Minimal app exercising `integrate_ship_physics` in isolation, with the
    /// debug helm write-tracker wired up exactly as `ShipPlugin` wires it.
    ///
    /// Deliberately excludes `process_helm_inputs`/`operate_helm_ai` so a test
    /// can seed the intent components directly and observe what the integrator
    /// alone does with them — which is the whole point of the #695/#699 split.
    fn integrator_only_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin).insert_resource(
            bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_millis(200)),
        );
        #[cfg(debug_assertions)]
        app.init_resource::<crate::ship::helm::HelmPhysicsFrame>()
            .add_systems(First, crate::ship::helm::tick_helm_physics_frame);
        app.add_systems(Update, integrate_ship_physics);
        app
    }

    /// Spawn a ship into `integrator_only_app` with the given helm control
    /// source and intent, optionally as the `LocalShip`.
    fn spawn_integrator_ship(
        app: &mut App,
        source: ControlSource,
        is_local: bool,
        thrust: f32,
        steering: f32,
        lateral: f32,
    ) -> Entity {
        let mut sources = ShipSystemControlSources::default();
        sources
            .0
            .set(crate::system_registry::helm_system_id(), source);
        let entity = app
            .world_mut()
            .spawn((
                Ship,
                sources,
                ShipPhysics::default(),
                crate::ai_plugin::AiHighFidelity,
                ThrustInput(thrust),
                SteeringInput(steering),
                LateralThrustInput(lateral),
            ))
            .id();
        if is_local {
            app.world_mut().entity_mut(entity).insert(LocalShip);
        }
        entity
    }

    fn physics_of(app: &mut App, entity: Entity) -> ShipPhysics {
        *app.world().entity(entity).get::<ShipPhysics>().unwrap()
    }

    /// AC: `integrate_ship_physics` is the sole helm-path writer of
    /// `ShipPhysics`, observed through the debug write-tracker.
    ///
    /// After a tick, every high-fidelity ship must be stamped, and stamped by
    /// `integrate_ship_physics` — no other system claimed the helm write.
    #[cfg(debug_assertions)]
    #[test]
    fn integrate_ship_physics_is_sole_helm_writer() {
        let mut app = integrator_only_app();
        let ship = spawn_integrator_ship(&mut app, ControlSource::Human, true, 1.0, 0.0, 0.0);

        // Several ticks: the tracker must not trip, and the stamp must track
        // the frame counter rather than going stale.
        for _ in 0..5 {
            tick(&mut app);
            let frame = app
                .world()
                .resource::<crate::ship::helm::HelmPhysicsFrame>()
                .0;
            let guard = app
                .world()
                .entity(ship)
                .get::<crate::ship::helm::HelmPhysicsWriteGuard>()
                .expect("integrate_ship_physics must self-heal a write guard onto every ship");
            assert_eq!(
                guard.last_write(),
                Some((frame, "integrate_ship_physics")),
                "integrate_ship_physics must be the sole helm-path writer of ShipPhysics"
            );
        }

        // Sanity: the ship actually moved, so the tracker was tracking a real
        // integration rather than a no-op.
        assert!(
            physics_of(&mut app, ship).forward_speed > 0.0,
            "ship must have actually been integrated"
        );
    }

    /// The write-tracker must actually bite: if some other writer stamps the
    /// ship for the frame `integrate_ship_physics` is about to run, the
    /// integrator panics rather than silently double-integrating.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "single-writer violation")]
    fn write_tracker_panics_when_a_second_helm_writer_claims_the_same_frame() {
        let mut app = integrator_only_app();
        let ship = spawn_integrator_ship(&mut app, ControlSource::Human, true, 1.0, 0.0, 0.0);
        tick(&mut app);

        // Impersonate a second helm-path writer claiming the *next* frame
        // (the frame counter is bumped in `First`, before the integrator runs).
        let next_frame = app
            .world()
            .resource::<crate::ship::helm::HelmPhysicsFrame>()
            .0
            + 1;
        {
            let mut ship_mut = app.world_mut().entity_mut(ship);
            let mut guard = ship_mut
                .get_mut::<crate::ship::helm::HelmPhysicsWriteGuard>()
                .unwrap();
            guard.record_write(ship, "some_future_helm_system", next_frame);
        }

        tick(&mut app);
    }

    /// AC: AI helm and human helm produce identical trajectories given
    /// equivalent inputs.
    ///
    /// Post-#695 both paths converge on the same intent components, and
    /// `integrate_ship_physics` is the only thing downstream of them. This pins
    /// that the integrator does not branch on human-vs-AI: two ships whose helm
    /// `ControlSource` differs, but whose intent is identical, must trace the
    /// same path. (Set up via `integrator_only_app` so the two *deciders* are
    /// out of the picture — this is specifically about the shared integrator.)
    #[test]
    fn ai_and_human_helm_produce_identical_trajectories() {
        let mut app = integrator_only_app();
        // Non-trivial intent: thrust + steering + strafe, so any human-vs-AI
        // branch anywhere in the integrator would show up as divergence.
        const THRUST: f32 = 0.8;
        // Sized so `acceleration * dt` (≈6.7/tick at the `HELM_AI_MAX_DT_SECS`
        // cap) carries the ship past `THRUST * TEST_MAX_SPEED` = 16.0 well
        // inside the loop below.
        const TEST_MAX_SPEED: f32 = 20.0;
        const TEST_ACCELERATION: f32 = 200.0;
        let human = spawn_integrator_ship(&mut app, ControlSource::Human, false, THRUST, 0.6, -0.4);
        let ai = spawn_integrator_ship(&mut app, ControlSource::Ai, false, THRUST, 0.6, -0.4);

        // `compute_physics` is acceleration-rate-limited: while
        // `|target - forward_speed| > acceleration * dt` the per-tick delta is
        // exactly `acceleration * dt` *regardless of thrust magnitude*, so any
        // thrust divergence between the two ships is invisible. With the stock
        // config and `dt` capped at `HELM_AI_MAX_DT_SECS`, neither ship escapes
        // that regime within a short test — which made this test blind to
        // thrust. Give both ships an identical high-acceleration config so
        // `forward_speed` reaches its thrust-proportional target within a few
        // ticks and thrust becomes observable. Test setup, not gameplay tuning.
        let test_cfg = ShipPhysicsConfigResource(ShipPhysicsConfig {
            max_speed: TEST_MAX_SPEED,
            acceleration: TEST_ACCELERATION,
            ..ShipPhysicsConfig::new()
        });
        for e in [human, ai] {
            app.world_mut().entity_mut(e).insert(test_cfg.clone());
        }

        // Precondition: the two ships genuinely differ in control source,
        // otherwise this test is vacuous.
        let policy_of = |app: &App, e: Entity| {
            helm_control_policy(
                app.world()
                    .entity(e)
                    .get::<ShipSystemControlSources>()
                    .unwrap(),
            )
        };
        assert!(
            policy_of(&app, ai).operate_ai && !policy_of(&app, human).operate_ai,
            "test must compare an AI-controlled helm against a human-controlled one"
        );

        // Compare the whole trajectory, not just the endpoint.
        for step in 0..10 {
            tick(&mut app);
            assert_eq!(
                physics_of(&mut app, human),
                physics_of(&mut app, ai),
                "human- and AI-controlled helm must integrate identically from \
                 identical intent (diverged at step {step})"
            );
        }

        // Sanity: the ships actually moved and turned, so equality is not the
        // trivial equality of two untouched defaults.
        let p = physics_of(&mut app, human);
        assert!(
            p.yaw != 0.0 && p.lateral_speed != 0.0,
            "ships must have actually manoeuvred, got {p:?}"
        );

        // Anti-vacuity guard for thrust specifically. `forward_speed` must have
        // settled at its thrust-proportional target, proving the trajectory left
        // `compute_physics`'s acceleration-rate-limited regime — the regime in
        // which the per-tick delta is independent of thrust and the equality
        // above therefore says nothing about it. Without this, retuning
        // `acceleration`/`max_speed` could silently re-blind the test to thrust.
        assert_eq!(
            p.forward_speed,
            THRUST * TEST_MAX_SPEED,
            "forward_speed must reach its thrust-proportional target, else this \
             test cannot observe thrust at all"
        );
    }

    /// AC: impulse override zeroes steering and lateral.
    ///
    /// While impulse is active the autopilot forces thrust=1/steering=0/
    /// lateral=0 regardless of helm intent. A control ship with identical
    /// intent but no impulse must yaw and strafe, proving the override is what
    /// suppresses them rather than the inputs being ignored generally.
    #[test]
    fn impulse_override_zeroes_steering_and_lateral() {
        let mut app = integrator_only_app();
        let impulsing = spawn_integrator_ship(&mut app, ControlSource::Human, true, 0.0, 1.0, 1.0);
        let control = spawn_integrator_ship(&mut app, ControlSource::Human, true, 0.0, 1.0, 1.0);

        let mut active = crate::impulse::ImpulseState::new();
        active.start_charge();
        active.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        assert_eq!(active.phase, ImpulsePhase::Active, "impulse must be active");
        app.world_mut()
            .entity_mut(impulsing)
            .insert(ShipImpulse(active));

        for _ in 0..5 {
            tick(&mut app);
        }

        let p = physics_of(&mut app, impulsing);
        assert_eq!(p.yaw, 0.0, "impulse override must zero steering, got {p:?}");
        assert_eq!(
            p.lateral_speed, 0.0,
            "impulse override must zero lateral thrust, got {p:?}"
        );
        assert_eq!(
            p.roll, 0.0,
            "impulse override must level the ship, got {p:?}"
        );
        assert!(
            p.forward_speed > 0.0,
            "impulse autopilot must force full forward thrust despite thrust=0.0 intent, got {p:?}"
        );

        // The control ship shares the same intent but has no impulse: it must
        // turn and strafe, so the assertions above are about the override.
        let c = physics_of(&mut app, control);
        assert!(
            c.yaw != 0.0 && c.lateral_speed != 0.0,
            "control ship (no impulse) must steer and strafe from the same intent, got {c:?}"
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
            .add_plugins(ShipPlugin);
        let hull_config = &[
            (crate::messages::SystemId("helm".into()), 25.0_f32),
            (crate::messages::SystemId("tactical".into()), 25.0),
            (crate::messages::SystemId("power".into()), 25.0),
            (crate::messages::SystemId("shields".into()), 25.0),
            (crate::messages::SystemId("helm-engine-port".into()), 15.0),
            (
                crate::messages::SystemId("helm-engine-starboard".into()),
                15.0,
            ),
        ];
        let ship = app
            .world_mut()
            .spawn((
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
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(
                    hull_config,
                )),
                LastHelmInput::default(),
                crate::simulation::ShipShields(crate::shield::ShieldSystem::default(), 0.5),
                ShipImpulse(crate::impulse::ImpulseState::new()),
            ))
            .id();
        app.world_mut()
            .entity_mut(ship)
            .insert((ShipModifiers::new(), ShipBoost::default()));
        // This ship carries no AiHighFidelity bundle by default (unlike
        // `test_app()`), but `integrate_ship_physics` (issue #695) is
        // scoped to `AiHighFidelity`, and these engine-thrust tests drive
        // `ShipPhysics` purely through `LastHelmInput` + the human
        // admission/physics pipeline. Add the marker + helm intent
        // components so physics keeps integrating for this ship, matching
        // pre-#695 behavior where `process_helm_inputs` computed physics
        // for any `LocalShip` unconditionally.
        app.world_mut().entity_mut(ship).insert((
            crate::ai_plugin::AiHighFidelity,
            crate::ship::helm::ThrustInput::default(),
            crate::ship::helm::SteeringInput::default(),
            crate::ship::helm::LateralThrustInput::default(),
            crate::ship::helm::ImpulseCommand::default(),
            crate::ship::helm::BoostCommand::default(),
        ));
        app
    }

    /// Set the HP of a specific system on the LocalShip hull to `new_hp`.
    /// Delegates to `SystemHull::set_hp` which directly sets the value.
    fn set_console_hp_direct(app: &mut App, system_id: crate::messages::SystemId, new_hp: f32) {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut entity_mut = app.world_mut().entity_mut(ship);
        let mut hull = entity_mut
            .get_mut::<crate::entity_spawner::EntitySystemHull>()
            .unwrap();
        hull.0.set_hp(&system_id, new_hp);
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
                lateral: 0.0,
            },
        );

        tick(&mut app);

        let queue = app.world().resource::<InterSystemQueue>();
        let port_id = crate::system_registry::helm_engine_port_system_id();
        let stbd_id = crate::system_registry::helm_engine_starboard_system_id();

        let port_msgs: Vec<_> = queue.for_target(port_id.0.as_str()).collect();
        let stbd_msgs: Vec<_> = queue.for_target(stbd_id.0.as_str()).collect();

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
        let InterSystemPayload::JoystickState { thrust, steering } = &port_msgs[0].payload else {
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
        set_console_hp_direct(
            &mut app,
            crate::messages::SystemId("helm-engine-port".into()),
            0.0,
        );
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

    /// Regression test for the reviewer's finding on issue #617.
    ///
    /// Before the fix, `sync_console_damage_tiers` iterated BOTH
    /// `EntityConsoleHull` AND `EntitySystemHull`. In production only the
    /// former was mutated by damage code, so the second (unmodified)
    /// iteration silently cleared every `offline_systems` entry that the
    /// first correctly inserted — meaning a hull-destroyed system would be
    /// re-marked online on the very next tick.
    ///
    /// This test spawns a ship carrying only `EntitySystemHull`, damages the
    /// helm system to 0 HP, runs the sync system TWICE, and asserts the
    /// SystemId stays in `offline_systems` across both ticks. Under the old
    /// buggy behaviour the second tick would have cleared the entry.
    #[test]
    fn sync_damage_tiers_keeps_disabled_system_offline_across_ticks() {
        let mut app = test_app();
        let helm_sid = crate::messages::SystemId("helm".into());

        // Damage the helm system to 0 HP (Destroyed tier).
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut entity_mut = app.world_mut().entity_mut(ship);
            let mut hull = entity_mut
                .get_mut::<crate::entity_spawner::EntitySystemHull>()
                .unwrap();
            hull.0.set_hp(&helm_sid, 0.0);
        }

        // Tick 1: sync_console_damage_tiers runs, must insert helm into
        // offline_systems.
        tick(&mut app);
        {
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
            assert!(
                control_sources.0.offline_systems.contains(&helm_sid),
                "after tick 1, helm should be in offline_systems (HP = 0)"
            );
        }

        // Tick 2: no damage change. Under the pre-fix bug the second loop
        // (over the unmutated sibling component) would have re-marked helm
        // as Operational and cleared it from offline_systems. After the fix
        // there is only one iteration, so the entry must persist.
        tick(&mut app);
        {
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
            assert!(
                control_sources.0.offline_systems.contains(&helm_sid),
                "after tick 2, helm MUST still be in offline_systems (regression \
                 for issue #617 dual-iteration clobber bug)"
            );
        }
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
        set_last_helm_input(
            &mut app_both,
            LastHelmInput {
                thrust: 1.0,
                steering: 0.0,
                lateral: 0.0,
            },
        );
        for _ in 0..TICKS {
            tick(&mut app_both);
        }
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
        set_console_hp_direct(
            &mut app_one,
            crate::messages::SystemId("helm-engine-port".into()),
            0.0,
        );
        tick(&mut app_one); // let Damage tier propagate
        set_last_helm_input(
            &mut app_one,
            LastHelmInput {
                thrust: 1.0,
                steering: 0.0,
                lateral: 0.0,
            },
        );
        for _ in 0..TICKS {
            tick(&mut app_one);
        }
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

    // ── Fine Power system → offline_systems tests (issue #513) ────────────────

    /// Build an app whose ship carries PowerReactor + PowerBattery hull
    /// entries. Used to exercise the hull → offline_systems chain for the
    /// fine power kinds.
    fn test_app_with_power_hull() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .add_plugins(ShipPlugin);
        let hull_config = &[
            (crate::messages::SystemId("helm".into()), 25.0_f32),
            (crate::messages::SystemId("tactical".into()), 25.0),
            (crate::messages::SystemId("power-reactor".into()), 15.0),
            (crate::messages::SystemId("power-battery".into()), 10.0),
            (crate::messages::SystemId("shields".into()), 25.0),
        ];
        let ship = app
            .world_mut()
            .spawn((
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
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(
                    hull_config,
                )),
                LastHelmInput::default(),
                crate::simulation::ShipShields(crate::shield::ShieldSystem::default(), 0.5),
                ShipImpulse(crate::impulse::ImpulseState::new()),
            ))
            .id();
        app.world_mut()
            .entity_mut(ship)
            .insert((ShipModifiers::new(), ShipBoost::default()));
        app
    }

    #[test]
    fn damaging_power_reactor_hull_to_disabled_puts_power_reactor_in_offline_systems() {
        let mut app = test_app_with_power_hull();
        set_console_hp_direct(
            &mut app,
            crate::messages::SystemId("power-reactor".into()),
            0.0,
        );
        tick(&mut app);

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
        let reactor_id = crate::system_registry::power_reactor_system_id();
        assert!(
            control_sources.0.offline_systems.contains(&reactor_id),
            "power-reactor should be in offline_systems when its hull HP is 0 (Disabled/Destroyed)"
        );
    }

    #[test]
    fn damaging_power_battery_hull_to_disabled_puts_power_battery_in_offline_systems() {
        let mut app = test_app_with_power_hull();
        set_console_hp_direct(
            &mut app,
            crate::messages::SystemId("power-battery".into()),
            0.0,
        );
        tick(&mut app);

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
        let battery_id = crate::system_registry::power_battery_system_id();
        assert!(
            control_sources.0.offline_systems.contains(&battery_id),
            "power-battery should be in offline_systems when its hull HP is 0 (Disabled/Destroyed)"
        );
    }

    // ── Issue #514 shield-arc hull tier sync tests ────────────────────────────

    /// Build a test app with a shield-arc-hull equipped ship. Uses a
    /// small hull budget so `set_arc_hp` is trivial for tests.
    fn test_app_with_shield_arc_hull() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .add_plugins(ShipPlugin);

        let tc = crate::damage::ConsoleTierConfig::default();
        let arc_hull = crate::damage::ShipArcHull::from_entries(vec![
            (
                "fore".into(),
                crate::damage::ArcHullEntry {
                    current: 10.0,
                    max: 10.0,
                    tier_config: tc,
                },
            ),
            (
                "aft".into(),
                crate::damage::ArcHullEntry {
                    current: 10.0,
                    max: 10.0,
                    tier_config: tc,
                },
            ),
        ]);
        let hull_config = &[(crate::messages::SystemId("helm".into()), 25.0_f32)];
        let ship = app
            .world_mut()
            .spawn((
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
                crate::ai_plugin::AiHighFidelity,
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(
                    hull_config,
                )),
                LastHelmInput::default(),
                crate::simulation::ShipShields(crate::shield::ShieldSystem::default(), 0.5),
            ))
            .id();
        app.world_mut().entity_mut(ship).insert((
            ShipModifiers::new(),
            ShipBoost::default(),
            ShipImpulse(crate::impulse::ImpulseState::new()),
            crate::ship::shields::ShieldArcIntents::default(),
            crate::console_ai_plugin::ShipFrequencyHintState::default(),
            crate::ship::power::PowerReactorIntents::default(),
            crate::ship::power::ShipPowerAiState::default(),
            crate::weapons_plugin::TorpedoIntents::default(),
            crate::entity_spawner::EntityShipArcHull(arc_hull),
        ));
        app.world_mut().entity_mut(ship).insert((
            crate::ship::helm::ThrustInput::default(),
            crate::ship::helm::SteeringInput::default(),
            crate::ship::helm::LateralThrustInput::default(),
            crate::ship::helm::ImpulseCommand::default(),
            crate::ship::helm::BoostCommand::default(),
        ));
        app
    }

    #[test]
    fn sync_console_damage_tiers_flips_shield_arc_offline_on_disabled_hp() {
        let mut app = test_app_with_shield_arc_hull();
        // Zero the fore arc hull HP.
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut entity_mut = app.world_mut().entity_mut(ship);
            let mut arc_hull = entity_mut
                .get_mut::<crate::entity_spawner::EntityShipArcHull>()
                .unwrap();
            arc_hull.0.set_hp("fore", 0.0);
        }
        tick(&mut app);
        // After sync, offline_systems must contain shield-arc-fore.
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let cs = app
            .world()
            .entity(ship)
            .get::<ShipSystemControlSources>()
            .unwrap();
        let fore_sid = crate::system_registry::shield_arc_system_id("fore").expect("fore");
        assert!(
            cs.0.offline_systems.contains(&fore_sid),
            "shield-arc-fore must be in offline_systems when its arc HP is 0"
        );
        let aft_sid = crate::system_registry::shield_arc_system_id("aft").expect("aft");
        assert!(
            !cs.0.offline_systems.contains(&aft_sid),
            "shield-arc-aft must NOT be in offline_systems (still at full HP)"
        );
    }

    #[test]
    fn sync_console_damage_tiers_removes_shield_arc_from_offline_on_repair() {
        let mut app = test_app_with_shield_arc_hull();
        // Zero fore, tick to insert into offline_systems.
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut entity_mut = app.world_mut().entity_mut(ship);
            let mut arc_hull = entity_mut
                .get_mut::<crate::entity_spawner::EntityShipArcHull>()
                .unwrap();
            arc_hull.0.set_hp("fore", 0.0);
        }
        tick(&mut app);
        // Restore fore to full.
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut entity_mut = app.world_mut().entity_mut(ship);
            let mut arc_hull = entity_mut
                .get_mut::<crate::entity_spawner::EntityShipArcHull>()
                .unwrap();
            arc_hull.0.set_hp("fore", 10.0);
        }
        tick(&mut app);
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let cs = app
            .world()
            .entity(ship)
            .get::<ShipSystemControlSources>()
            .unwrap();
        let fore_sid = crate::system_registry::shield_arc_system_id("fore").expect("fore");
        assert!(
            !cs.0.offline_systems.contains(&fore_sid),
            "shield-arc-fore must be removed from offline_systems after repair"
        );
    }

    // ── Issue #684: Destroyed-tier alerts to Captain ─────────────────────────

    #[derive(Resource, Default)]
    struct CoordEnqueueBox(Vec<CoordinationEnqueue>);

    fn collect_coord(
        mut reader: MessageReader<CoordinationEnqueue>,
        mut box_: ResMut<CoordEnqueueBox>,
    ) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn drain_coord(app: &mut App) -> Vec<CoordinationEnqueue> {
        let msgs = app.world().resource::<CoordEnqueueBox>().0.clone();
        app.world_mut().resource_mut::<CoordEnqueueBox>().0.clear();
        msgs
    }

    fn coord_test_app() -> App {
        let mut app = test_app();
        app.init_resource::<CoordEnqueueBox>()
            .add_systems(PostUpdate, collect_coord);
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(LastSystemTiers::default());
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipConfigComponent, With<Ship>>();
        for mut cfg in q.iter_mut(app.world_mut()) {
            cfg.0.coordination_lag_secs = 0.0;
        }
        app
    }

    fn set_captain_control_source(app: &mut App, source: ControlSource) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(crate::system_registry::captain_system_id(), source);
        }
    }

    #[test]
    fn destroyed_crossing_emits_alert_to_captain() {
        let mut app = coord_test_app();
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        tick(&mut app);
        let emitted = drain_coord(&mut app);
        let alerts: Vec<_> = emitted
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
            .collect();
        assert_eq!(alerts.len(), 1, "expected exactly one Alert");
        assert_eq!(
            alerts[0].target,
            crate::ship::system_registry::captain_system_id(),
            "Alert must target Captain system"
        );
        assert_eq!(alerts[0].sender_label, "tactical");
        assert!(
            matches!(&alerts[0].payload, CoordinationPayload::Alert { .. }),
            "payload must be Alert"
        );
    }

    #[test]
    fn non_destroyed_crossing_does_not_emit_alert() {
        let mut app = coord_test_app();
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 5.0);
        tick(&mut app);
        let emitted = drain_coord(&mut app);
        let alerts: Vec<_> = emitted
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
            .collect();
        assert_eq!(alerts.len(), 0, "no Alert for non-Destroyed crossing");
        assert!(
            emitted
                .iter()
                .any(|e| matches!(&e.payload, CoordinationPayload::RepairRequest { .. })),
            "expected a RepairRequest for Disabled crossing"
        );
    }

    #[test]
    fn destroyed_alert_fires_once() {
        let mut app = coord_test_app();
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        tick(&mut app);
        let emitted_t1 = drain_coord(&mut app);
        assert_eq!(
            emitted_t1
                .iter()
                .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
                .count(),
            1,
            "first tick must emit Alert"
        );
        tick(&mut app);
        let emitted_t2 = drain_coord(&mut app);
        assert_eq!(
            emitted_t2
                .iter()
                .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
                .count(),
            0,
            "second tick must not re-emit Alert (fire-once)"
        );
    }

    #[test]
    fn destroyed_alert_refires_after_restore_and_re_destroy() {
        let mut app = coord_test_app();
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid.clone(), 0.0);
        tick(&mut app);
        assert_eq!(
            drain_coord(&mut app)
                .iter()
                .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
                .count(),
            1,
            "first destroy must emit Alert"
        );
        set_console_hp_direct(&mut app, tact_sid.clone(), 25.0);
        tick(&mut app);
        drain_coord(&mut app);
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        tick(&mut app);
        assert_eq!(
            drain_coord(&mut app)
                .iter()
                .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
                .count(),
            1,
            "re-destroy after restore must emit Alert again"
        );
    }

    /// Routing test helper: creates a test app without `collect_coord` (to avoid
    /// interfering with the coordination event readers) and sets lag to 0.
    fn routing_test_app() -> App {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(LastSystemTiers::default());
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipConfigComponent, With<Ship>>();
        for mut cfg in q.iter_mut(app.world_mut()) {
            cfg.0.coordination_lag_secs = 0.0;
        }
        app
    }

    #[test]
    fn destroyed_alert_consumed_by_ai_captain() {
        let mut app = routing_test_app();
        start_game_with_helm_and_science(&mut app);
        set_captain_control_source(&mut app, ControlSource::Ai);
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        tick(&mut app);
        tick(&mut app);
        let outbox = app.world().resource::<crate::lobby::LobbyOutbox>();
        let popups: Vec<_> = outbox
            .0
            .iter()
            .filter(|(_, msg)| {
                matches!(
                    msg,
                    crate::messages::ServerMessage::CoordinationPopup { .. }
                )
            })
            .collect();
        assert!(
            popups.is_empty(),
            "AI Captain must not produce CoordinationPopup; got {} popup(s)",
            popups.len()
        );
    }

    #[test]
    fn destroyed_alert_shows_popup_for_human_captain() {
        let mut app = routing_test_app();
        start_game_with_helm_and_science(&mut app);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(SystemId("tactical".into()), ControlSource::Ai);
            }
        }
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        // Tick 1: detect_damage_tier_crossings writes CoordinationEnqueue
        //         into the message send buffer.
        // Tick 2: buffer-swap → handle_coordination_enqueue reads and enqueues
        //         to CoordinationQueue with due_time = now + 0.
        //         process_coordination_lag reads due messages and dispatches
        //         a CoordinationPopup to the LobbyOutbox.
        // Tick 3: consumes the popup and/or allows the broadcast to flush.
        tick(&mut app);
        tick(&mut app);
        tick(&mut app);
        let outbox = app.world().resource::<crate::lobby::LobbyOutbox>();
        let has_popup = outbox.0.iter().any(|(_, msg)| {
            matches!(
                msg,
                crate::messages::ServerMessage::CoordinationPopup { .. }
            )
        });
        assert!(
            has_popup,
            "Human Captain must receive a CoordinationPopup for destroyed system"
        );
    }
}
