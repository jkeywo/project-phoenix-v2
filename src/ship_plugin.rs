use bevy::prelude::*;
use std::collections::HashMap;

use crate::console_bridge::AiChatterEvent;
use crate::control_source::ControlSourceResolver;
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

/// The shared fixed-rate AI-helm sim tick (issue #803). One repeating timer
/// gates **all four** per-axis AI helm systems (`ai_helm_thrust`,
/// `ai_helm_steering`, `ai_helm_lateral_thrust`, `ai_helm_impulse`) so the
/// AI's helm decision cadence is decoupled from the frame rate. Production
/// time is rAF-driven — `bridge.rs` installs `WinitSettings` with
/// `UpdateMode::Continuous` for both focused and unfocused — so `Update` runs
/// at the host's display refresh, ~16.7 ms at 60 Hz and ~6.9 ms at 144 Hz.
/// Without this gate the helm AI would recompute once per rendered frame and
/// a 144 Hz host would steer on ~4x fresher data than a 60 Hz one — precisely
/// the nondeterminism PRD #620 (P2P deterministic lockstep) exists to remove.
/// (`WorldSnapshot` itself is rebuilt every frame — see
/// `ai::server::build_world_snapshot` — so the ticks this gate skips would
/// have seen genuinely fresh data, not a recomputation of an identical
/// result.)
///
/// The rate is TOML-authored: `[global] ai_helm_tick_hz` in the world TOML
/// (`GlobalConfig::ai_helm_tick_hz`, serde default 30 Hz — the old
/// `AiLateralThrustTimer` period). The resource is created at plugin build,
/// before any `WorldConfig` exists, so `tick_ai_helm_timer` reconciles the
/// period against the loaded world config on each frame (a cheap
/// duration-equality check that only writes when the authored rate differs).
#[derive(Resource)]
struct AiHelmTickTimer(Timer);

impl Default for AiHelmTickTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(
            1.0 / crate::entity_config::GlobalConfig::default().ai_helm_tick_hz,
            TimerMode::Repeating,
        ))
    }
}

/// Boolean latch set each frame by `tick_ai_helm_timer` (issue #803).
/// `run_if` conditions must use read-only params, so the timer is advanced by
/// a dedicated system that writes this flag, which the condition then reads —
/// the same shape as `ai::server::AiSnapshotReady`. Initialises to `true` so
/// the very first update always runs the helm AI (before the timer has had a
/// chance to fire).
#[derive(Resource)]
struct AiHelmTickReady(bool);

/// Advance the `AiHelmTickTimer` and set `AiHelmTickReady`. Registered
/// `.after` all four per-axis AI helm systems so the flag is consumed before
/// it is re-armed for the next frame. Only leaves `true` when the timer
/// fires; on frames where it doesn't the flag is explicitly cleared so the
/// gated systems skip their work.
///
/// Also reconciles the timer period against the TOML-authored
/// `[global] ai_helm_tick_hz` once `WorldConfig` exists — the timer resource
/// is created at plugin build, before the world TOML has been parsed.
fn tick_ai_helm_timer(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut timer: ResMut<AiHelmTickTimer>,
    mut ready: ResMut<AiHelmTickReady>,
) {
    if let Some(wc) = world_config.as_deref() {
        let hz = wc.global.ai_helm_tick_hz;
        if hz > 0.0 {
            let configured = std::time::Duration::from_secs_f32(1.0 / hz);
            if timer.0.duration() != configured {
                timer.0.set_duration(configured);
            }
        }
    }
    ready.0 = timer.0.tick(time.delta()).just_finished();
}

/// Read-only run condition for the four per-axis AI helm systems: fires only
/// on shared sim-tick frames (issue #803).
fn ai_helm_tick_ready(ready: Res<AiHelmTickReady>) -> bool {
    ready.0
}

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
/// by `ai_helm_steering` to bias steering toward the requested bearing.
/// (`operate_helm_ai` was the other reader until #704 deleted it; it stood down
/// from the whole arc-bearing step whenever helm-steering was AI, so the fold
/// into steering is now unconditional rather than a fallback.)
#[derive(Component, Clone, Debug, Default)]
pub struct PendingArcBearingRequest(pub Option<uuid::Uuid>);

/// Which generation of this ship's [`NavigationWaypoint`] the AI Helm is
/// cleared to follow (issue #702).
///
/// The Channel-3 Navigation→Helm lag, reduced to one integer. `Navigation` sets
/// the waypoint and enqueues `CoordinationPayload::NavigateTo` carrying its
/// `generation`; the message spends the delivery lag in the queue; when
/// `process_coordination_lag` finally consumes it, it latches the generation
/// here. The AI Helm travels to the waypoint only while
/// `clearance == waypoint.generation()`.
///
/// Because the waypoint bumps its generation whenever it names somewhere new,
/// *every* new waypoint re-incurs the lag. There is only ever one waypoint and
/// no copy of the previous one, so during the lag the Helm does not keep flying
/// the old bearing: [`cleared_nav_waypoint`] yields `None` and the Helm falls
/// through to its own local objectives, or idles if it has none. A bare `bool`
/// ("Navigation has spoken") would only delay the first order and then wave
/// every subsequent waypoint through instantly.
///
/// `None` = never cleared for anything.
///
/// [`NavigationWaypoint`]: crate::navigation_plugin::NavigationWaypoint
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct HelmWaypointClearance(pub Option<u64>);

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
        // Shared AI-helm sim tick (issue #803): the timer/latch pair that
        // gates all four per-axis AI helm systems, plus the dedicated system
        // that advances it. The tick system runs `.after` every gated system
        // so the latch is consumed before it is re-armed — the same
        // consume-then-arm shape as `ai::server::tick_ai_snapshot_timer`.
        .init_resource::<AiHelmTickTimer>()
        .insert_resource(AiHelmTickReady(true))
        .add_systems(
            Update,
            tick_ai_helm_timer
                .after(ai_helm_thrust)
                .after(ai_helm_steering)
                .after(ai_helm_lateral_thrust)
                .after(ai_helm_impulse),
        )
        .add_systems(
            Update,
            (
                // `.after(AiTickLabel)`: this system reads the viewscreen
                // blackboard's scored objectives, which the AI tick writes.
                ai_helm_lateral_thrust
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel)
                    .before(process_helm_inputs)
                    // Shared AI-helm sim tick (issue #803) — one fixed-rate
                    // cadence for all four per-axis systems.
                    .run_if(ai_helm_tick_ready),
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
                    .after(handle_impulse_messages)
                    .after(handle_boost_messages)
                    .before(process_helm_inputs)
                    .before(tick_impulse)
                    .before(tick_boost),
                process_helm_inputs
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(tick_impulse),
                publish_joystick_to_engines
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(process_helm_inputs),
                // A `LastHelmInput` thrust/steering pair reader.
                // `ai_helm_thrust`/`ai_helm_steering` declare
                // `.before(operate_helm_engine_ai)` themselves, and
                // `ai_helm_lateral_thrust` (the `.lateral` field's writer) is
                // transitively before it via `process_helm_inputs`.
                operate_helm_engine_ai.in_set(crate::sim_sets::SimSet::Physics),
                detect_reached_objective_completion.in_set(crate::sim_sets::SimSet::Broadcast),
                tick_impulse.in_set(crate::sim_sets::SimSet::Physics),
                tick_boost.in_set(crate::sim_sets::SimSet::Physics),
                handle_impulse_messages.in_set(crate::sim_sets::SimSet::Input),
                handle_boost_messages.in_set(crate::sim_sets::SimSet::Input),
                // Sole helm-path writer of ShipPhysics (issues #695, #699):
                // reads the intent components written by
                // `process_helm_inputs` (human/admission) and the per-axis
                // helm AI (`ai_helm_thrust`/`ai_helm_steering`/
                // `ai_helm_lateral_thrust`), plus the post-transition
                // `ShipImpulse`/`ShipBoost` state applied by
                // `apply_helm_commands`, then performs the actual physics
                // integration for whichever ship (LocalShip or promoted NPC)
                // has fresh values this tick. Ordered after every writer of
                // those intents and after `tick_impulse` so it reads this
                // tick's freshly-ticked impulse phase, mirroring the old fused
                // `process_helm_inputs` ordering. (`ai_helm_thrust` /
                // `ai_helm_steering` declare `.before(integrate_ship_physics)`
                // themselves; `ai_helm_lateral_thrust` and `ai_helm_impulse`
                // are covered by `.after(process_helm_inputs)` /
                // `.after(apply_helm_commands)` respectively.)
                integrate_ship_physics
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(process_helm_inputs)
                    .after(apply_helm_commands)
                    .after(tick_impulse)
                    .after(tick_boost),
                sync_ship_position
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(process_helm_inputs)
                    .after(integrate_ship_physics),
                handle_station_rating_change.in_set(crate::sim_sets::SimSet::Input),
                handle_coordination_enqueue.in_set(crate::sim_sets::SimSet::Input),
                handle_coordination_messages.in_set(crate::sim_sets::SimSet::Input),
                process_coordination_lag.in_set(crate::sim_sets::SimSet::Modifiers),
                sync_console_damage_tiers.in_set(crate::sim_sets::SimSet::Damage),
                detect_damage_tier_crossings.in_set(crate::sim_sets::SimSet::Damage),
            )
                .after(crate::lobby::LobbySystemSet),
        );

        // Per-axis helm AI (issue #701). Registered separately from the tuple
        // above purely because that tuple is at Bevy's 20-element limit.
        //
        // Ordering contract:
        // - `.after(AiTickLabel)`: both systems read the viewscreen
        //   blackboard's scored objectives, which the AI tick writes.
        // - Each system gates on its own axis alone (#800) and is the sole
        //   writer of the axis it owns, so no edge between them (or against a
        //   second writer) is needed for write exclusion. They are
        //   deliberately *unordered* against each other: each writes only its
        //   own axis and neither reads the other's output.
        // - Ordered AFTER `process_helm_inputs`, unlike
        //   `ai_helm_lateral_thrust` (lateral's fine system being AI-operated
        //   means no lateral command is ever admitted, so there is nothing
        //   for `process_helm_inputs` to clobber it with). Thrust and
        //   steering are per-axis admitted (`SetThrust` → `helm-thrust`,
        //   `SetSteering` → `helm-steering`), and `process_helm_inputs`
        //   additionally skips writing an AI-held axis's intent. Running last
        //   makes the per-axis AI the authoritative writer of its axis
        //   deterministically rather than by set-order luck.
        // - Both systems write `LastHelmInput.{thrust,steering}` for the
        //   player ship, one field each, and are the only writers of those
        //   fields. Every reader of that *pair* in `SimSet::Physics` must be
        //   ordered after BOTH of them, or it can observe a torn pair — this
        //   tick's AI throttle next to last tick's stale human steering. The
        //   pair readers are `publish_joystick_to_engines`,
        //   `operate_helm_engine_ai` and `tick_boost`;
        //   `helm_ai_last_input_pair_is_not_torn` pins the result.
        //   (`ai_helm_lateral_thrust` writes only the disjoint `.lateral`
        //   field and is already `.before(process_helm_inputs)`;
        //   `ai_power_allocation` reads `.thrust` alone, so it cannot see a
        //   torn pair and needs no edge.)
        app.add_systems(
            Update,
            (ai_helm_thrust, ai_helm_steering)
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::sim_sets::AiTickLabel)
                .after(process_helm_inputs)
                .before(publish_joystick_to_engines)
                .before(operate_helm_engine_ai)
                .before(tick_boost)
                .before(integrate_ship_physics)
                // Shared AI-helm sim tick (issue #803) — one fixed-rate
                // cadence for all four per-axis systems.
                .run_if(ai_helm_tick_ready),
        );

        // Per-axis helm AI: impulse (issue #703). Registered on its own because
        // its ordering is the mirror image of the other two per-axis systems.
        //
        // `.before(apply_helm_commands)` is the hard requirement — that is what
        // consumes `ImpulseCommand` into a `ShipImpulse` phase transition, and
        // it already runs before `process_helm_inputs`, so this system cannot
        // join `ai_helm_thrust`/`ai_helm_steering` in running last.
        // `.after(AiTickLabel)` covers the scored objectives this system reads.
        // The `apply_helm_commands` edge also transitively keeps this system
        // before `ai_helm_thrust` (`apply_helm_commands` runs before
        // `process_helm_inputs`, which `ai_helm_thrust` runs after), so no
        // explicit edge between them is needed.
        app.add_systems(
            Update,
            ai_helm_impulse
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::sim_sets::AiTickLabel)
                .before(apply_helm_commands)
                // Shared AI-helm sim tick (issue #803) — one fixed-rate
                // cadence for all four per-axis systems.
                .run_if(ai_helm_tick_ready),
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

    // Per-axis admission (issue #801). Each axis is applied only when its own
    // declared system is NOT AI-operated: admission already refuses human
    // commands for an AI-held axis (`accept_human_input == false`), and the
    // per-axis AI (`ai_helm_thrust` / `ai_helm_steering`) is the authoritative
    // writer of that axis's intent this tick. Skipping per axis — instead of
    // the old ship-wide coarse-`helm` gate — is what fixes the #701 mismatch:
    // with `helm-thrust = Ai` and `helm-steering = Human`, the human's
    // steering is admitted and applied while the thrust axis stays with the AI.
    let thrust_ai = sources
        .0
        .policy_for(&crate::system_registry::helm_thrust_system_id())
        .operate_ai;
    let steering_ai = sources
        .0
        .policy_for(&crate::system_registry::helm_steering_system_id())
        .operate_ai;

    if !thrust_ai {
        for cmd in admitted.for_target(crate::system_registry::HELM_THRUST_SYSTEM_ID) {
            if let SystemControlPayload::SetThrust { value } = &cmd.payload {
                last_input.thrust = *value;
            }
        }
    }

    if !steering_ai {
        for cmd in admitted.for_target(crate::system_registry::HELM_STEERING_SYSTEM_ID) {
            if let SystemControlPayload::SetSteering { value } = &cmd.payload {
                last_input.steering = *value;
            }
        }
    }

    for cmd in admitted.for_target(&crate::system_registry::lateral_thrust_system_id().0) {
        if let SystemControlPayload::LateralThrustInput { lateral } = &cmd.payload {
            last_input.lateral = *lateral;
        }
    }

    if let Some((thrust_in, steering_in, lateral_in)) = intent_q.iter_mut().next() {
        if let Some(mut t) = thrust_in {
            if !thrust_ai {
                t.0 = last_input.thrust;
            }
        }
        if let Some(mut s) = steering_in {
            if !steering_ai {
                s.0 = last_input.steering;
            }
        }
        if let Some(mut l) = lateral_in {
            l.0 = last_input.lateral;
        }
    }
}

/// True when the AI helm is flying this ship: both stick axes
/// (`helm-thrust` AND `helm-steering`) are AI-operated. The coarse `helm`
/// system this used to gate on was deleted by #801; per Rule 6 the answer
/// derives from the per-axis declarations, never a coarse fallback.
pub(crate) fn helm_axes_operate_ai(sources: &ShipSystemControlSources) -> bool {
    sources
        .0
        .policy_for(&crate::system_registry::helm_thrust_system_id())
        .operate_ai
        && sources
            .0
            .policy_for(&crate::system_registry::helm_steering_system_id())
            .operate_ai
}

// ── Shared helm-AI decision inputs (issue #701) ───────────────────────────────
//
// The per-axis `ai_helm_thrust` / `ai_helm_steering` / `ai_helm_lateral_thrust`
// / `ai_helm_impulse` all need the same three inputs: the world entity list,
// the entity's scored objectives, and a `WorldView`. These helpers are the
// single implementation of each, so the per-axis systems cannot silently
// drift from the monolith they replace in #704.

/// The console-owned surfaces the AI Helm derives its goals from (issue #702).
///
/// Every one of these is a shared, authoritative surface that a human operator
/// could equally drive — that symmetry is the point. The Helm reads them; it
/// owns none of them, and keeps no private copy of any of them:
///
/// | Surface | Owner | Answers |
/// |---|---|---|
/// | [`WeaponsTarget`] | Tactical (human `SetTarget` / `ai_target_selection`) | who to pursue |
/// | [`NavigationWaypoint`] + [`HelmWaypointClearance`] | Navigation (+ the Channel-3 lag) | where to travel |
/// | [`ObjectiveCursors`] | `advance_objective_cursors` | where on the route |
///
/// All `Option` because minimal test spawns omit them; a missing surface means
/// "no goal from that console", never a fabricated default.
///
/// Bundled as one `QueryData` because all three per-axis helm systems need the
/// identical set, and because their per-system queries are close to Bevy's
/// tuple cap.
///
/// [`WeaponsTarget`]: crate::weapons_plugin::WeaponsTarget
/// [`NavigationWaypoint`]: crate::navigation_plugin::NavigationWaypoint
/// [`ObjectiveCursors`]: crate::ai_plugin::ObjectiveCursors
#[derive(bevy::ecs::query::QueryData)]
pub struct HelmAiSurfaces {
    weapons_target: Option<&'static crate::weapons_plugin::WeaponsTarget>,
    waypoint: Option<&'static crate::navigation_plugin::NavigationWaypoint>,
    clearance: Option<&'static HelmWaypointClearance>,
    cursors: Option<&'static crate::ai_plugin::ObjectiveCursors>,
}

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
    let from_blackboard = match blackboards
        .0
        .get(&crate::system_registry::helm_station_key())
    {
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

/// The Navigation waypoint this ship's AI Helm is currently *cleared* to follow
/// (issue #702), or `None` if there is none or the clearance has not caught up.
///
/// This is the whole of the Channel-3 Navigation-to-Helm lag on the read side.
/// Navigation — `operate_navigation_ai` or a human's admitted
/// `SetNavigationWaypoint` alike (AGENTS.md rule 6) — sets `NavigationWaypoint`
/// and enqueues a `NavigateTo`
/// carrying its `generation`; that message serves the delivery lag in the queue;
/// `process_coordination_lag` then latches the generation into
/// `HelmWaypointClearance`. Until the latch matches, the Helm has been given the
/// waypoint but not yet the order, so this returns `None` — every *new* waypoint
/// re-incurs the lag, not merely the first.
///
/// `None` during the lag does not mean "carry on as before": the waypoint is
/// overwritten in place and the old position is not kept anywhere, so the Helm
/// cannot resume the previous bearing. It falls back to its own local
/// objectives, or idles if it has none, until the clearance catches up.
///
/// A ship missing either component (bare test spawns) is never cleared, which is
/// the same safe default: it falls back to its own local objectives.
fn cleared_nav_waypoint(
    waypoint: Option<&crate::navigation_plugin::NavigationWaypoint>,
    clearance: Option<&HelmWaypointClearance>,
) -> Option<[f32; 2]> {
    let waypoint = waypoint?;
    let cleared_generation = clearance?.0?;
    if cleared_generation != waypoint.generation() {
        return None;
    }
    let snapshot = waypoint.snapshot()?;
    Some([snapshot.x, snapshot.z])
}

/// This ship's Tactical lock as a UUID, for the Helm to pursue (issue #702).
///
/// `WeaponsTarget` is a `String` because it may name an asteroid as well as an
/// entity; the Helm only pursues things with a canonical UUID, and an
/// unparseable id names nobody.
fn helm_weapons_target(
    weapons_target: Option<&crate::weapons_plugin::WeaponsTarget>,
) -> Option<uuid::Uuid> {
    weapons_target?
        .0
        .as_deref()
        .and_then(|t| uuid::Uuid::parse_str(t).ok())
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
///
/// Takes everything by shared reference: `operate_helm` has been pure since
/// #702, so calling this twice in a tick (once per axis) is safe by
/// construction rather than by scheduling.
#[allow(clippy::too_many_arguments)]
fn helm_ai_decision(
    world_view: &crate::ai::WorldView,
    scored: &[crate::messages::ScoredObjective],
    behaviour_section: Option<&crate::entities::spawner::BehaviourSection>,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    cursors: Option<&crate::ai_plugin::ObjectiveCursors>,
    weapons_target: Option<&crate::weapons_plugin::WeaponsTarget>,
    nav_waypoint: Option<[f32; 2]>,
    forward_speed: f32,
) -> (f32, f32) {
    const NO_CURSORS: &[crate::ai::patrol_cursor::PatrolCursor] = &[];
    crate::ai::operate_helm(
        world_view,
        scored,
        behaviour_section
            .map(|b| b.0.doctrine.as_slice())
            .unwrap_or(&[]),
        anchors,
        cursors.map(|c| c.0.as_slice()).unwrap_or(NO_CURSORS),
        helm_weapons_target(weapons_target),
        nav_waypoint,
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

/// Resolve the target position from the highest-scored Helm objective.
///
/// Owned by `ai_helm_impulse`, its sole caller since #704. It was
/// `operate_helm_ai`'s helper, shared with `ai_helm_impulse` when #703 extracted
/// that system; deleting the monolith left the helper with one caller rather
/// than none, so it stays a free function here (beside the other shared helm-AI
/// input helpers) rather than being inlined — `ai_helm_impulse` is not its only
/// *conceivable* caller, and the top-objective selection it performs has to stay
/// consistent with the `top_obj` filter at that call site — see
/// `ai_helm_impulse_leaves_the_drive_alone_without_a_helm_objective`, which pins
/// the two agreeing.
///
/// Reads exactly the surfaces `operate_helm` reads (issue #702) — the ship's
/// `WeaponsTarget` for `Destroy`, the objective's cursor for `Patrol`, the named
/// anchor for `Reach`/`Retreat`. Two answers to "where is the Helm going?" must
/// not diverge, or the ship charges its impulse drive at a point it is not
/// steering toward.
fn resolve_helm_target_position(
    scored: &[crate::messages::ScoredObjective],
    world_view: &crate::ai::WorldView,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    cursors: Option<&crate::ai_plugin::ObjectiveCursors>,
    weapons_target: Option<uuid::Uuid>,
) -> Option<[f32; 3]> {
    use crate::messages::{AiDirective, SystemAffinity};
    let top = scored
        .iter()
        .find(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm))?;
    match &top.directive {
        // `Reach` and `Retreat` are the same shape: fly to a named anchor.
        // An unknown or empty anchor resolves to nowhere, exactly as the
        // matching arms of `operate_helm` do.
        AiDirective::Reach { anchor } | AiDirective::Retreat { anchor } => {
            anchors.get(anchor.as_str()).copied()
        }
        // The directive's own `target` is Tactical's input, not the Helm's.
        // `helm_destroy` pursues the `WeaponsTarget` that `ai_target_selection`
        // resolved from it, so this must read the same lock or the impulse
        // could aim at the authored target while the helm closes on whoever
        // Tactical actually locked.
        AiDirective::Destroy { .. } => {
            let uuid = weapons_target?;
            world_view
                .entities
                .iter()
                .find(|e| e.uuid == uuid)
                .map(|e| e.position)
        }
        AiDirective::Patrol {
            anchors: waypoints,
            loop_path,
        } => {
            let index = cursors
                .and_then(|c| {
                    c.0.iter()
                        .find(|cursor| cursor.objective_id == top.id)
                        .map(|cursor| cursor.index())
                })
                .unwrap_or(0);
            crate::ai::patrol_cursor::cursor_target(index, waypoints, *loop_path, anchors)
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
        if !helm_axes_operate_ai(sources) {
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

// ── Per-axis helm AI (issues #701, #703) ──────────────────────────────────────
//
// `ai_helm_thrust`, `ai_helm_steering`, `ai_helm_lateral_thrust` and
// `ai_helm_impulse` are the per-axis helm AI: one owns `ThrustInput`, one
// `SteeringInput`, one `LateralThrustInput`, one `ImpulseCommand`. Each gates on
// its own axis alone:
//
//     if !<own axis>.operate_ai { continue; }
//
// They are the successors to the `operate_helm_ai` monolith (deleted in #704,
// after #800/#703 declared every axis on every shipped hull and removed the
// coarse half of each gate).
//
// **Each intent component has exactly one writer, and it is the component's own
// system:**
//
//   ThrustInput        ← `ai_helm_thrust`         iff T
//   SteeringInput      ← `ai_helm_steering`       iff S
//   LateralThrustInput ← `ai_helm_lateral_thrust` iff L
//   ImpulseCommand     ← `ai_helm_impulse`        iff I
//
// (T/S/L/I = the helm-thrust / helm-steering / helm-lateral-thrust /
// helm-impulse `operate_ai` policies.) One writer per component means Bevy's
// arbitrary intra-set ordering cannot decide the outcome (the #697 failure
// mode) because there is nothing to decide between.
//
// **The coarse `helm` policy C is no longer an input to any of this.** It gated
// the monolith and nothing else; with the monolith gone, no helm-AI system reads
// it. That is a load-bearing absence, not an accident: `C` is exactly the
// coarse-fallback channel #800 spent an issue proving dormant, and re-admitting
// it would resurrect the failure mode where an axis is silently driven by
// something other than its own declaration.
// `helm_writers_are_invariant_under_coarse_policy` pins the whole outcome
// invariant under C over every (C, T, S, L, I) combination;
// `coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do` pins it
// end-to-end through a ticking app;
// `shipped_hull_helm_is_driven_by_the_per_axis_declarations_alone` pins it on a
// real hull's control sources.
//
// The corollary is that an axis a hull does not declare is an axis no AI drives.
// `ControlSource::default()` is `Human` (`operate_ai == false`), so an
// undeclared axis resolves to "human-held" and its system stands down; before
// #704 the monolith quietly covered that case. All nine shipped hulls therefore
// declare all four axes — see `shipped_hull_config_drives_the_per_axis_helm_systems`
// and `shipped_hull_config_drives_ai_helm_lateral_thrust`, which pin the
// declarations themselves against the real TOMLs. Adding a hull means declaring
// four axes, not one.
//
// Per the owner's ruling each of these systems calls the pure
// `crate::ai::operate_helm` and keeps only its own output, duplicating the
// `WorldView` build per ship per tick. There is deliberately no shared cached
// `HelmDecision` component; that would re-create the mini-monolith this split
// exists to remove. (`ai_helm_lateral_thrust` does not need `operate_helm` at
// all — `operate_lateral_thrust` is a separate pure function — so it only
// duplicates the `WorldView`.)
//
// **No shared mutable state** (issue #702). `operate_helm` is a pure function:
// it reads the ship's surfaces (`WeaponsTarget`, `NavigationWaypoint` +
// `HelmWaypointClearance`, `ObjectiveCursors`, the scored pool) and returns
// `(thrust, steering)`. Both systems can call it, in either order, and each
// keeps only its own axis. Each goal lives with the console that owns it —
// Tactical selects the target, Navigation sets the waypoint, the objective's
// cursor tracks the route — so "how many times did `operate_helm` run?" is
// not a question anyone has to answer.
//
// The **`LastHelmInput` ordering** remains load-bearing. Both systems write
// `LastHelmInput` for the player ship, one field each (`.thrust` /
// `.steering`), and they are the only writers of those fields. Any reader of
// that *pair* in
// `SimSet::Physics` must therefore be ordered after BOTH, or it can observe a
// torn pair — this tick's AI throttle beside last tick's stale human steering.
// The pair readers are `publish_joystick_to_engines`, `operate_helm_engine_ai`
// and `tick_boost`; `helm_ai_last_input_pair_is_not_torn` pins the result.
//
// (`ai_helm_lateral_thrust` also writes `LastHelmInput`, but only the disjoint
// `.lateral` field, and it is already `.before(process_helm_inputs)` — hence
// already before these two. `ai_power_allocation` reads `.thrust` alone, so it
// cannot see a torn pair and needs no edge.)
//
// Because `operate_helm` is a pure function of (world_view, scored, surfaces,
// tuning), both systems reach an identical decision from identical inputs — so
// the axes never disagree, and the per-axis result stays bit-identical to what
// the monolith produced before #800.

/// Per-axis helm AI: throttle. Writes `ThrustInput` for ships whose
/// helm-thrust system is AI-operated, whatever the coarse helm is doing — since
/// #704 deleted `operate_helm_ai` this is the axis's only AI writer (issues
/// #800, #704).
///
/// `AiHighFidelity`-scoped: `ThrustInput` only exists on ships carrying that
/// marker (`lod_ai_ships` inserts/removes it with the intent bundle), so this
/// system can take `&mut ThrustInput` directly rather than `Option<&mut _>`.
/// (Every per-axis helm system is scoped this way since #703 brought
/// `ai_helm_lateral_thrust` into line.)
///
/// Mutates nothing but its own axis: `operate_helm` is pure since #702, so
/// there is no `AiMemory` commit to own and no ordering against
/// `ai_helm_steering` to arbitrate — see the module note.
///
/// Takes `&ShipPhysics` read-only and never advances physics itself;
/// `integrate_ship_physics` is the sole helm-path writer (issues #695, #699).
#[allow(clippy::too_many_arguments)]
fn ai_helm_thrust(
    mut local_ship_input: Query<&mut LastHelmInput, With<crate::server_app::LocalShip>>,
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    entity_fallback_q: HelmAiFallbackQuery,
    mut ships: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Has<crate::server_app::LocalShip>,
            HelmAiSurfaces,
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
        blackboards,
        entity_uuid,
        faction,
        collider,
        helm_section,
        behaviour_section,
        is_local,
        surfaces,
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

        // No commit rule, and no scratch clone to dodge one: `operate_helm` is
        // pure, so we simply ask it and keep our axis (issue #702).
        let (thrust, _steering) = helm_ai_decision(
            &world_view,
            &scored,
            behaviour_section,
            &anchors,
            surfaces.cursors,
            surfaces.weapons_target,
            cleared_nav_waypoint(surfaces.waypoint, surfaces.clearance),
            physics.forward_speed,
        );

        thrust_in.0 = thrust;

        // Mirror to LastHelmInput for the player ship so broadcast /
        // fine-engine bookkeeping consumers see the AI's throttle. Since #704
        // this system is the sole writer of the `.thrust` field on the AI path
        // (`operate_helm_ai` mirrored it field-wise for the coarse path). The
        // ordering that stops a reader seeing this beside a stale `.steering`
        // survives #702 — see the module note.
        if is_local {
            if let Some(mut li) = local_ship_input.iter_mut().next() {
                li.thrust = thrust;
            }
        }
    }
}

/// Per-axis helm AI: steering. Writes `SteeringInput` for ships whose
/// helm-steering system is AI-operated, whatever the coarse helm is doing —
/// since #704 deleted `operate_helm_ai` this is the axis's only AI writer, and
/// it owns the arc-bearing step outright (issues #800, #704).
///
/// Steers toward the selected waypoint/target chosen by the pure
/// `crate::ai::operate_helm`, which resolves the top-scored Helm-relevant
/// directive. That includes the **Retreat consumer** (issue #688): when
/// `AiDirective::Retreat` is the top-scored directive, `operate_helm`'s Retreat
/// arm resolves its named anchor and steers toward it.
/// `ai_helm_steering_retreats_toward_anchor` pins that behaviour through this
/// system, and `ai_helm_steering_retreat_with_unknown_anchor_falls_through`
/// pins the other side of it.
///
/// Retreat reached this system via a *synthetic* objective injected by
/// `aggregate_doctrine_blackboards` below a `[behaviour] retreat_hull_threshold`
/// until #702; it is now ordinary authored doctrine, and the empty-anchor /
/// `home_position` fallback that the synthetic one depended on is gone.
///
/// Mutates nothing but its own axis — `operate_helm` is pure since #702, so
/// there is no `AiMemory` commit and no ordering against `ai_helm_thrust` (see
/// the module note). Takes `&ShipPhysics` read-only; `integrate_ship_physics`
/// is the sole helm-path physics writer (issues #695, #699).
#[allow(clippy::too_many_arguments)]
fn ai_helm_steering(
    mut local_ship_input: Query<&mut LastHelmInput, With<crate::server_app::LocalShip>>,
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    entity_fallback_q: HelmAiFallbackQuery,
    mut ships: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Has<crate::server_app::LocalShip>,
            Option<&mut PendingArcBearingRequest>,
            Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
            HelmAiSurfaces,
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
        blackboards,
        entity_uuid,
        faction,
        collider,
        helm_section,
        behaviour_section,
        is_local,
        mut pending_bearing,
        combat_config_opt,
        surfaces,
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

        // Pure call, no commit — see the module note (issue #702).
        let (_thrust, mut steering) = helm_ai_decision(
            &world_view,
            &scored,
            behaviour_section,
            &anchors,
            surfaces.cursors,
            surfaces.weapons_target,
            cleared_nav_waypoint(surfaces.waypoint, surfaces.clearance),
            physics.forward_speed,
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

/// Per-axis helm AI: impulse drive. Writes `ImpulseCommand` for ships whose
/// helm-impulse system is AI-operated, whatever the coarse helm is doing —
/// since #704 deleted `operate_helm_ai` this is the impulse decision's only AI
/// writer (issues #703, #704).
///
/// `AiHighFidelity`-scoped (AC3): `ImpulseCommand` only exists on ships
/// carrying that marker (`lod_ai_ships` inserts/removes it with the intent
/// bundle), so the query can take `&mut ImpulseCommand` directly.
///
/// **Reads the shared helm surfaces; mutates none of them.** It resolves where
/// the Helm is going via `resolve_helm_target_position`, over the same
/// `WeaponsTarget` / `ObjectiveCursors` the steering decision uses, so the drive
/// charges toward the point the ship is actually steering at.
///
/// Writes only on an `Engage`/`Cancel` decision, never on `NoChange`:
/// `apply_helm_commands` transitions on `ImpulseCommand` change detection, so
/// an unconditional write would re-issue `start_charge`/`cancel_charge` every
/// tick.
#[allow(clippy::too_many_arguments)]
fn ai_helm_impulse(
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    entity_fallback_q: HelmAiFallbackQuery,
    mut ships: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Has<crate::server_app::LocalShip>,
            Option<&ShipImpulse>,
            Option<&ImpulseConfigResource>,
            HelmAiSurfaces,
            &mut ImpulseCommand,
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
        blackboards,
        entity_uuid,
        faction,
        collider,
        helm_section,
        behaviour_section,
        is_local,
        impulse_comp,
        impulse_cfg,
        surfaces,
        mut impulse_cmd,
    ) in ships.iter_mut()
    {
        // Gate on our own system alone — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_impulse_system_id())
            .operate_ai
        {
            continue;
        }

        // No drive or no per-hull drive config → nothing to command. Matches
        // the monolith, which guards the same pair.
        let (Some(impulse), Some(cfg)) = (impulse_comp, impulse_cfg) else {
            continue;
        };

        let scored = helm_ai_scored_objectives(blackboards);
        // No Helm objective → leave `ImpulseCommand` alone. The monolith's
        // no-objective branch `continue`s before its impulse block for exactly
        // the same reason: an in-progress charge is not something a lull in
        // objectives should cancel.
        //
        // Behaviourally this is a *redundant* early-out rather than a
        // load-bearing gate. Two more `score > 0.0 && Helm`-relevant filters
        // downstream reach the same `continue` on their own:
        // `resolve_helm_target_position`'s top-objective selection, and the
        // `top_obj` lookup that resolves `use_impulse` below. Mutation-testing
        // confirms it: this line and either one of those can be deleted
        // individually with every test still green; only removing all three
        // turns `ai_helm_impulse_leaves_the_drive_alone_without_a_helm_objective`
        // red.
        //
        // It is kept because it short-circuits the `WorldView` build and the
        // `operate_helm` replay below — real work, per ship per tick — and
        // because it keeps the shape legible against the monolith it replaces.
        if !has_helm_objective(&scored) {
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

        // Resolve where the Helm is going, from the same surfaces `operate_helm`
        // reads. There is no `helm_ai_decision` replay here any more: it existed
        // only to advance a scratch `AiMemory` so that this lookup would see the
        // post-advance `waypoint_index`. The cursor is read-only and lives
        // outside the decision now, so the replay computed an answer nobody
        // used (issue #702).
        let Some(target_pos) = resolve_helm_target_position(
            &scored,
            &world_view,
            &anchors,
            surfaces.cursors,
            helm_weapons_target(surfaces.weapons_target),
        ) else {
            continue;
        };

        // Whether the AI may engage impulse at all while pursuing this
        // objective is TOML-authored per doctrine entry
        // (`[[behaviour.doctrine]] use_impulse`); an objective with no matching
        // doctrine entry never engages.
        let top_obj = scored.iter().find(|o| {
            o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm)
        });
        let use_impulse = top_obj
            .and_then(|obj| {
                behaviour_section.and_then(|b| b.0.doctrine.iter().find(|d| d.id == obj.id))
            })
            .map(|d| d.effective_use_impulse())
            .unwrap_or(false);
        if !use_impulse {
            continue;
        }

        let decision = crate::ai::decide_impulse(&crate::ai::ImpulseDecisionInput {
            pos: [physics.x, physics.z],
            yaw: physics.yaw,
            target_pos,
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

// ── Per-axis helm AI: lateral thrust (issues #697, #703) ──────────────────────
//
// Born in #697 as `operate_lateral_thrust_ai`, a *partial-automation* system:
// its gate was `L && !C`, so it ran only when the lateral-thrust system was AI
// and the coarse helm was human. That second half existed because
// `operate_helm_ai` owned `LateralThrustInput` outright whenever the coarse
// helm was AI, and two writers would have raced.
//
// #703 gave lateral thrust the same treatment #800 gave thrust and steering:
// the monolith stood down from `LateralThrustInput` iff helm-lateral-thrust was
// AI, so the coarse half of the gate became redundant and came off. `L && !C`
// and `C && !L` were disjoint by construction; `L` and `C && !L` still were.
// #704 then deleted the monolith outright, so the `C && !L` writer is gone and
// `L` alone decides — there is no longer a second writer to be disjoint from.
//
// **This changes when the system runs.** The issue's acceptance criterion says
// "lateral thrust AI works under partial automation only", which described the
// #697 gate. That wording cannot survive the collapse it asks for in the same
// breath: dropping `!C` is precisely what makes the system run under full
// automation too. Under full automation this system now produces the dodge the
// monolith used to. Partial automation is no longer the only case it serves; it
// is the only case in which it is the sole helm AI running.
//
// **Partial automation is not left untouched, though**, and it would be wrong to
// file the whole of #703 under "behaviour-preserving": the divergence fixes
// below apply to the `L && !C` path too, where #697's system — not the monolith
// — was the writer. Radar gating is the one that bites. #697 ran with unlimited
// range, so a Simplified-rating cruiser whose helm radar is shot up **will now
// dodge less** than it did before this change: it stops reacting to obstacles it
// cannot actually see. That is the intended outcome — it is what the monolith
// has always done, and aligning the two is the point of the exercise — but it is
// a real behaviour change on that path, not a preservation of it.
//
// Behaviour parity with the monolith is not automatic, and #697's version did
// not have it. Three divergences had to be closed before the coarse half could
// come off, or shipped hulls (`alliance_cruiser`/`_destroyer`/`_courier` all
// declare helm-lateral-thrust and flip to this system the moment their helm
// station goes unmanned) would have silently changed behaviour under full
// automation:
//
//   * **Radar gating.** #697 built its own `WorldView` with
//     `visible_entities(pos, 0.0, ..)` — range 0 means *unlimited*. The monolith
//     gates by the ship's damage-scaled helm radar range. A cruiser with a
//     shot-up radar would have started dodging rocks it cannot see. Now shares
//     `helm_ai_world_view`.
//   * **Snapshot fallback.** #697 early-returned when `WorldSnapshot` was
//     absent; the monolith falls back to a direct ECS query. Now shares
//     `helm_ai_snapshot_entities`.
//   * **No-objective zeroing.** #697 `continue`d, leaving a stale dodge latched;
//     the monolith zeroes the axis so `integrate_ship_physics` decelerates it
//     off through the normal physics curve. Now zeroes.
//
// The ~30 Hz throttle on this system predates the split (it was the private
// `AiLateralThrustTimer` until #803) and is load-bearing: production `Update`
// is rAF-driven (`server/bridge.rs` installs `WinitSettings` with
// `UpdateMode::Continuous`), i.e. ~16.7 ms at 60 Hz, so a 33.3 ms period fires
// roughly every *other* frame — a real throttle, and the only one on this
// path, since `build_world_snapshot` runs every frame. Coupling the dodge
// cadence to the host's display refresh rate is precisely the nondeterminism
// PRD #620 (P2P deterministic lockstep) exists to remove.
//
// #803 generalised the gate: the private timer became the shared
// `AiHelmTickTimer` / `AiHelmTickReady` sim tick (see the resource note at the
// top of this file), and **all four** per-axis systems — this one,
// `ai_helm_thrust`, `ai_helm_steering` and `ai_helm_impulse` — now attach the
// same `run_if(ai_helm_tick_ready)` condition, so the whole helm AI decides on
// one fixed-rate cadence instead of the lateral axis alone being throttled
// while its siblings ran per rendered frame. The rate is TOML-authored
// (`[global] ai_helm_tick_hz`, default 30 Hz — the old timer's period, so the
// lateral cadence is unchanged). A skipped frame runs none of the four, so an
// axis simply holds its last intent through the gap and
// `integrate_ship_physics` keeps integrating it.
// `*_runs_on_the_shared_sim_tick_not_per_frame` pins the cadence for each of
// the four systems.

/// Per-axis helm AI: lateral thrust. Writes `LateralThrustInput` for ships
/// whose helm-lateral-thrust system is AI-operated, whatever the coarse helm is
/// doing — since #704 deleted `operate_helm_ai` this is the axis's only AI
/// writer (issues #703, #704).
///
/// `AiHighFidelity`-scoped (AC3). #697 deliberately was not, taking
/// `Option<&mut LateralThrustInput>` so it could match a demoted NPC that had
/// lost the component and skip the write. That rationale only ever bought the
/// right to iterate ships it could do nothing for: the intent component and the
/// marker are inserted and removed together by `lod_ai_ships`, so "demoted" and
/// "no `LateralThrustInput`" are the same set, and the guarded write always
/// skipped. The one thing the loop body could still have done for such a ship —
/// mirror to `LastHelmInput` — is reachable only for `LocalShip`, which never
/// demotes. So the scoping is behaviour-preserving, and the query takes
/// `&mut LateralThrustInput` directly like its two siblings.
///
/// Does not touch `AiMemory`: `crate::ai::operate_lateral_thrust` is a pure
/// function of the world view, the scored objectives and the hull's avoidance
/// tuning. It is therefore outside the commit ordering in the module note above.
#[allow(clippy::too_many_arguments)]
fn ai_helm_lateral_thrust(
    mut local_ship_input: Query<&mut LastHelmInput, With<crate::server_app::LocalShip>>,
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    entity_fallback_q: HelmAiFallbackQuery,
    mut ships: Query<
        (
            &ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            &ShipPhysics,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            // Optional, so it does not filter the iteration set: a ship without
            // a `[behaviour]` section still runs AI lateral thrust, on the
            // `crate::ai::*` fallbacks that match the serde defaults.
            Option<&crate::entities::spawner::BehaviourSection>,
            Has<crate::server_app::LocalShip>,
            &mut LateralThrustInput,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    // The ~30 Hz throttle that used to live here as a private timer is now
    // the shared `run_if(ai_helm_tick_ready)` sim-tick gate on the
    // registration (issue #803), common to all four per-axis systems. A
    // skipped frame runs nothing at all rather than writing a stale value —
    // including the no-objective zeroing below — so the axis simply holds its
    // last value through the gap and `integrate_ship_physics` keeps
    // integrating it.
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
        blackboards,
        physics,
        collider,
        entity_uuid,
        faction,
        helm_section,
        behaviour_section,
        is_local,
        mut lateral_in,
    ) in ships.iter_mut()
    {
        // Gate on our own system alone (issue #703) — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::lateral_thrust_system_id())
            .operate_ai
        {
            continue;
        }

        let scored = helm_ai_scored_objectives(blackboards);
        if !has_helm_objective(&scored) {
            // No objectives → zero the dodge rather than latch the last one,
            // matching what the monolith does for the axis.
            lateral_in.0 = 0.0;
            if is_local {
                if let Some(mut li) = local_ship_input.iter_mut().next() {
                    li.lateral = 0.0;
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

        // TOML-authored avoidance tuning, same as `helm_ai_decision` uses: how
        // much clearance this hull wants is a property of the hull, not of which
        // system happens to be automated. The dodge and the yaw must agree, or
        // the ship sidesteps an obstacle its steering has already dismissed.
        // `full_ai_helm_honours_toml_authored_avoidance_buffer` /
        // `..._look_ahead` pin this site against the constants (commit 7f4e2661).
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

        lateral_in.0 = lateral;

        // Mirror to LastHelmInput for the player ship — see `ai_helm_thrust`.
        if is_local {
            if let Some(mut li) = local_ship_input.iter_mut().next() {
                li.lateral = lateral;
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
/// The per-axis helm AI already drives physics (via the intent components and
/// `integrate_ship_physics`); this system only ensures the fine engine systems
/// reflect AI-controlled thrust in the blackboard so the GUI can show AUTO
/// badges correctly.
///
/// Reads the `LastHelmInput` thrust/steering pair, so it must run after both
/// `ai_helm_thrust` and `ai_helm_steering` — they declare
/// `.before(operate_helm_engine_ai)` themselves. See the registration note.
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

    for cmd in admitted.for_target(crate::system_registry::HELM_IMPULSE_SYSTEM_ID) {
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
    for cmd in admitted.for_target(crate::system_registry::HELM_BOOST_SYSTEM_ID) {
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
    let has_helm = sessions
        .0
        .holder_for_station(&crate::messages::StationId(
            crate::system_registry::HELM_STATION_ID.into(),
        ))
        .is_some()
        || helm_axes_operate_ai(control_sources);
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
/// written (by `ai_helm_impulse`'s AI decision, `handle_impulse_messages`,
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
/// (human admission) or the per-axis helm AI (`ai_helm_thrust` /
/// `ai_helm_steering` / `ai_helm_lateral_thrust`) is authoritative for a given
/// ship's helm, per the existing `ControlTickPolicy` gate — plus the
/// post-transition `ShipImpulse`/`ShipBoost` state applied
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
/// Visual banking/roll is preserved as LocalShip-only, exactly as before — the
/// helm AI has never applied roll to NPCs (the `operate_helm_ai` monolith did
/// not, nor do its per-axis successors), and this system doesn't start doing so
/// either.
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

        // Visual banking: LocalShip only, exactly as before — the helm AI has
        // never applied roll to NPCs (neither the old `operate_helm_ai` nor its
        // per-axis successors), and this shared step doesn't start doing so
        // either. Uses the unscaled
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
            // The generation is an internal handle, not something a bridge
            // officer would say out loud; the label is the human-facing part.
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
            Option<&mut HelmWaypointClearance>,
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
        mut waypoint_clearance,
        mut alerted,
        mut repair_queue,
        mut pending_shields_threat,
        is_local,
    ) in ship_components.iter_mut()
    {
        let due = queue.0.due_messages(now);
        for msg in due {
            // Coordination targets are console-level station-id keys (issue
            // #801), so a helm-directed message cannot gate on a `helm`
            // system that no longer exists. The Helm console's effective
            // control source derives from its stick axes: AI when both
            // `helm-thrust` and `helm-steering` are AI-operated (the shape
            // Backfill and NPC spawn produce), otherwise the steering axis —
            // the axis helm-directed coordination (arc bearings, navigation
            // clearances) actually drives — is the representative. Every
            // other target resolves through `policy_for` as before; station
            // keys with no registered system (e.g. `"tactical"`) get the
            // default Human policy, unchanged from when the coarse tactical
            // id was undeclared.
            let helm_key = crate::system_registry::helm_station_key();
            let (target_policy, target_control) = if msg.target == helm_key {
                if helm_axes_operate_ai(control_sources) {
                    (
                        crate::ship::control_source::control_tick_policy(ControlSource::Ai),
                        ControlSource::Ai,
                    )
                } else {
                    let rep = crate::system_registry::helm_steering_system_id();
                    (
                        control_sources.0.policy_for(&rep),
                        control_sources.0.source_for(&rep),
                    )
                }
            } else {
                (
                    control_sources.0.policy_for(&msg.target),
                    control_sources.0.source_for(&msg.target),
                )
            };
            let action = if !target_policy.operate_ai && !target_policy.accept_human_input {
                coordination::DeliverAction::Consume
            } else {
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
                    if target_policy.operate_ai && msg.target == helm_key {
                        if let CoordinationPayload::ArcBearingRequest { uuid, .. } = &msg.payload {
                            if let Some(pending) = pending_bearing.as_deref_mut() {
                                pending.0 = uuid::Uuid::parse_str(uuid).ok();
                            }
                        }
                        // Channel-3 Navigation-to-Helm handoff (issues #681,
                        // #702): the order has now served its delivery lag, so
                        // clear the AI Helm to follow this generation of the
                        // ship's `NavigationWaypoint`. No position is copied —
                        // the waypoint is the goal, and `operate_helm` reads it
                        // straight off the ship.
                        if let CoordinationPayload::NavigateTo { generation, .. } = &msg.payload {
                            if let Some(clearance) = waypoint_clearance.as_deref_mut() {
                                clearance.0 = Some(*generation);
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
    use crate::control_source::{ControlSource, ControlTickPolicy};
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
            // The console-owned surfaces the AI helm derives its goals from
            // (issue #702). Production spawns all four on every ship; see
            // `HelmAiSurfaces`.
            crate::weapons_plugin::WeaponsTarget::default(),
            crate::navigation_plugin::NavigationWaypoint::default(),
            HelmWaypointClearance::default(),
            crate::ai_plugin::ObjectiveCursors::default(),
        ));
        app
    }

    /// Lock this ship's Tactical surface onto `uuid` (issue #702).
    ///
    /// The helm pursues `WeaponsTarget`; it no longer resolves a `Destroy`
    /// directive's authored name itself. In production `ai_target_selection`
    /// does that resolution (tier 1) and publishes the result here, so a test
    /// that poses a Destroy objective and expects pursuit must supply the lock
    /// that system would have written.
    fn set_ship_weapons_target(app: &mut App, uuid: &str) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut target = entity
            .get_mut::<crate::weapons_plugin::WeaponsTarget>()
            .expect("ship must carry WeaponsTarget");
        target.0 = Some(uuid.to_string());
    }

    /// Give this ship a Navigation waypoint *and* the Channel-3 clearance to
    /// fly it, as `operate_navigation_ai` → `process_coordination_lag` would
    /// once the order came due (issue #702). Returns the waypoint's generation.
    use crate::navigation_plugin::WaypointMode;

    fn set_cleared_nav_waypoint(app: &mut App, x: f32, z: f32) -> u64 {
        let ship = find_ship_entity(app);
        let generation = {
            let mut entity = app.world_mut().entity_mut(ship);
            let mut waypoint = entity
                .get_mut::<crate::navigation_plugin::NavigationWaypoint>()
                .expect("ship must carry NavigationWaypoint");
            waypoint.set(WaypointMode::Free { x, z });
            waypoint.generation()
        };
        let mut entity = app.world_mut().entity_mut(ship);
        let mut clearance = entity
            .get_mut::<HelmWaypointClearance>()
            .expect("ship must carry HelmWaypointClearance");
        clearance.0 = Some(generation);
        generation
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

    /// Put the whole helm — the coarse `helm` system and all four per-axis
    /// systems — on `source`.
    ///
    /// Before #704 this set the coarse system alone, which was enough: the
    /// `operate_helm_ai` monolith gated on the coarse policy and drove every
    /// axis whose own system was not AI, so "coarse = Ai" *was* "the helm is on
    /// AI". #704 deleted the monolith and with it the coarse fallback, so the
    /// coarse system alone now drives nothing at all and a fixture that set only
    /// it would assert against a ship no system is flying — a vacuous pass.
    ///
    /// Setting all five together is the faithful successor because it is what
    /// the shipped hulls actually do: every one of the nine declares all four
    /// axes with the same owner as the coarse `helm` (thrust/steering since
    /// #800, impulse/lateral since #704), so an unmanned station backfills all
    /// five to AI and a manned one leaves all five on the human. They move
    /// together in content; they move together here.
    ///
    /// Tests that need the axes to diverge from the coarse system — the
    /// per-axis gate and stand-down tests — call `set_fine_control_source`
    /// afterwards to override individual axes.
    fn set_helm_control_source(app: &mut App, source: ControlSource) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(crate::system_registry::helm_thrust_system_id(), source);
            cs.0.set(crate::system_registry::helm_steering_system_id(), source);
            cs.0.set(crate::system_registry::helm_impulse_system_id(), source);
            cs.0.set(crate::system_registry::helm_boost_system_id(), source);
            cs.0.set(crate::system_registry::lateral_thrust_system_id(), source);
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
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.25 },
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
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: -1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
            },
        );
        tick_twice(&mut app);

        // Human input must be ignored when policy is AI; no AiControllerComponent
        // on the player ship yet, so LastHelmInput stays at default.
        assert_eq!(get_last_helm_input(&mut app), LastHelmInput::default());
    }

    /// The #701 mismatch, fixed by #801: with `helm-thrust = Ai` and
    /// `helm-steering = Human`, the human's combined joystick input used to be
    /// admitted or refused on the COARSE helm policy, so the whole input got
    /// in and the AI's thrust write had to win by ordering. Per-axis wire
    /// targets make admission itself per-axis: the human's `SetSteering` is
    /// admitted, the human's `SetThrust` is refused at the gate.
    #[test]
    fn per_axis_admission_fixes_the_coarse_vs_per_axis_mismatch() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);
        // AI holds the throttle; the human keeps the stick.
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );

        // The human joystick fans out into the two per-axis messages.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.25 },
            },
        );
        tick_twice(&mut app);

        let last = get_last_helm_input(&mut app);
        assert_eq!(
            last.steering, 0.25,
            "the human-held steering axis must admit the human's SetSteering"
        );
        assert_eq!(
            last.thrust, 0.0,
            "the AI-held thrust axis must refuse the human's SetThrust at admission"
        );
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
                target: crate::system_registry::helm_impulse_system_id(),
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
                target: crate::system_registry::helm_impulse_system_id(),
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
                target: crate::system_registry::helm_impulse_system_id(),
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
                target: crate::system_registry::helm_impulse_system_id(),
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
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
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
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
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
            planet: None,
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
                target: crate::system_registry::helm_impulse_system_id(),
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
                target: crate::system_registry::helm_impulse_system_id(),
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
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
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
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.0 },
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
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.0 },
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
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut base,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.0 },
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
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
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
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.0 },
            },
        );
        push(
            &mut base,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
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
                target: crate::system_registry::helm_boost_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(boost_is_active(&mut app));

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_boost_system_id(),
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
                target: crate::system_registry::helm_boost_system_id(),
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
                target: crate::system_registry::helm_boost_system_id(),
                payload: SystemControlPayload::SetBoost { active: true },
            },
        );
        tick(&mut app);
        assert!(boost_is_active(&mut app));

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_boost_system_id(),
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
                target: crate::system_registry::helm_boost_system_id(),
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
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
            },
        );
        tick(&mut app);

        // Press IMPULSE → starts charging. `LastHelmInput` must be cleared.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
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
                target: crate::system_registry::helm_impulse_system_id(),
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

    /// The pre-#800 shape: the coarse `helm` system on AI with all four per-axis
    /// systems left Human — which is what an *undeclared* axis resolves to
    /// (`ControlSource::default() == Human`, so `operate_ai == false`).
    ///
    /// This was the configuration `operate_helm_ai` was built to serve, and the
    /// one every shipped hull was in before #800/#704 declared the axes. Since
    /// #704 deleted the monolith it drives nothing at all, and several tests
    /// below exist to pin exactly that: the coarse system is inert on its own,
    /// there is no coarse fallback, and re-introducing one would light these up.
    fn set_coarse_helm_only_ai(app: &mut App) {
        set_helm_control_source(app, ControlSource::Human);
        set_fine_control_source(
            app,
            // #801: "helm" is a station id, not a system. Seeding it here is
            // the point of the test — it must drive nothing.
            crate::messages::SystemId(crate::system_registry::HELM_STATION_ID.to_string()),
            ControlSource::Ai,
        );
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

    fn get_impulse_command(app: &mut App) -> crate::impulse::ImpulsePhase {
        app.world_mut()
            .query_filtered::<&ImpulseCommand, With<Ship>>()
            .single(app.world())
            .expect("expected Ship with ImpulseCommand")
            .0
    }

    fn set_impulse_command(app: &mut App, phase: crate::impulse::ImpulsePhase) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ImpulseCommand>()
            .expect("expected ImpulseCommand")
            .0 = phase;
    }

    /// A `[behaviour]` section whose one doctrine entry matches `objective_id`
    /// and permits impulse.
    ///
    /// `use_impulse` is authored explicitly rather than left to
    /// `effective_use_impulse`'s directive-kind default, so these tests pin the
    /// impulse *system* and not that default (which says `false` for Patrol —
    /// the directive some of them use).
    ///
    /// `target_speed`/`maintain_range` are restated because `DoctrineObjective`
    /// derives `Default`, which zeroes them rather than applying their serde
    /// `default =` values; a zero `target_speed` would silently pin the helm's
    /// throttle at 0 alongside whatever the test meant to measure.
    fn impulse_doctrine(objective_id: &str) -> crate::entity_config::BehaviourConfig {
        crate::entity_config::BehaviourConfig {
            doctrine: vec![crate::entity_config::DoctrineObjective {
                id: objective_id.into(),
                use_impulse: Some(true),
                target_speed: 0.8,
                maintain_range: 25.0,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// A ship set up for `ai_helm_impulse`: a per-hull impulse config (the
    /// system no-ops without one) and helm-impulse on AI. The coarse helm is
    /// left Human — which until #704 was what kept `operate_helm_ai` from being
    /// the writer of the `ImpulseCommand` these tests measure, and now simply
    /// isolates the axis. The shipped-hull test below exercises the everything-AI
    /// case.
    fn impulse_ai_app(objective: crate::messages::ScoredObjective) -> App {
        let mut app = test_app();
        let objective_id = objective.id.clone();
        set_ship_blackboard_objectives(&mut app, vec![objective]);
        set_behaviour_section(&mut app, impulse_doctrine(&objective_id));
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseConfigResource::default());
        set_helm_control_source(&mut app, ControlSource::Human);
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_impulse_system_id(),
            ControlSource::Ai,
        );
        app
    }

    /// Build a `ControlSourceResolver` from a shipped hull's TOML the way the
    /// game does when nobody is driving: parse the file, then set every
    /// *declared* system to `ControlSource::Ai`. That is literally what the NPC
    /// spawn path (`crate::entities::spawner`) does, and what the `Backfill`
    /// rating does to a player hull whose station goes unmanned — the two hull
    /// families reach the same end state, so the same helper serves both.
    ///
    /// Nothing is hand-set, so the resolver reflects exactly what the hull
    /// declares — which is the point of the tests that use it.
    fn resolver_from_shipped_hull(toml_str: &str) -> ControlSourceResolver {
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

    /// #704's precondition, pinned against every hull the game ships.
    ///
    /// The delete is only behaviour-preserving if every hull declares every axis.
    /// `ControlSource::default()` is `Human` (`operate_ai == false`), so an
    /// *undeclared* axis resolves to "human-held" and its per-axis system stands
    /// down — and until #704 the `operate_helm_ai` monolith silently covered that
    /// case, because it stood down only from axes that were declared *and* AI.
    /// Undeclare an axis after the delete and nothing writes that component at
    /// all: the ship loses the behaviour, quietly, with every test still green.
    ///
    /// That is not hypothetical. When #704 went to delete the monolith, five NPC
    /// hulls declared neither `helm-impulse` nor `helm-lateral-thrust`, and
    /// `alliance_battleship` declared no `helm-lateral-thrust` — so the monolith
    /// was still driving impulse and the avoidance dodge on their behalf, and
    /// deleting it would have removed both. #704 declares them; this test is what
    /// stops the gap re-opening, and it is deliberately a *table over every hull*
    /// rather than one hull per axis, because the previous shipped-hull tests
    /// (`shipped_hull_config_drives_the_per_axis_helm_systems` on `pirate_raider`,
    /// `shipped_hull_config_drives_ai_helm_lateral_thrust` on `alliance_cruiser`)
    /// each pinned one hull and one axis pair, which is exactly how six hulls
    /// drifted without anything going red.
    ///
    /// Reads the shipped TOMLs through the same resolver the game builds, so it
    /// fails on the declaration a hull is actually missing rather than on a
    /// hand-built fixture's idea of one.
    #[test]
    fn every_shipped_hull_declares_every_helm_axis() {
        let hulls: [(&str, &str); 9] = [
            (
                "alliance_battleship",
                include_str!("../assets/entities/alliance_battleship.toml"),
            ),
            (
                "alliance_courier",
                include_str!("../assets/entities/alliance_courier.toml"),
            ),
            (
                "alliance_cruiser",
                include_str!("../assets/entities/alliance_cruiser.toml"),
            ),
            (
                "alliance_destroyer",
                include_str!("../assets/entities/alliance_destroyer.toml"),
            ),
            (
                "pirate_raider",
                include_str!("../assets/entities/pirate_raider.toml"),
            ),
            (
                "pirate_raider_reinforcement",
                include_str!("../assets/entities/pirate_raider_reinforcement.toml"),
            ),
            (
                "ship_harrow_patrol",
                include_str!("../assets/entities/ship_harrow_patrol.toml"),
            ),
            (
                "ship_harrow_warhawk",
                include_str!("../assets/entities/ship_harrow_warhawk.toml"),
            ),
            (
                "ship_requiem_courier",
                include_str!("../assets/entities/ship_requiem_courier.toml"),
            ),
        ];

        let axes: [(&str, crate::messages::SystemId); 4] = [
            (
                "helm-thrust",
                crate::system_registry::helm_thrust_system_id(),
            ),
            (
                "helm-steering",
                crate::system_registry::helm_steering_system_id(),
            ),
            (
                "helm-impulse",
                crate::system_registry::helm_impulse_system_id(),
            ),
            (
                "helm-lateral-thrust",
                crate::system_registry::lateral_thrust_system_id(),
            ),
        ];

        for (hull, toml_str) in hulls {
            let resolver = resolver_from_shipped_hull(toml_str);

            // Sanity (#801): the coarse `helm` system is deleted from every
            // shipped hull — a TOML that still declared it would fail parse
            // (the kind is unregistered), but pin the resolver view too.
            assert!(
                !resolver
                    .policy_for(&crate::messages::SystemId(
                        crate::system_registry::HELM_STATION_ID.to_string()
                    ))
                    .operate_ai,
                "{hull} must NOT declare a coarse `helm` system (#801)"
            );

            for (axis_name, axis_id) in &axes {
                assert!(
                    resolver.policy_for(axis_id).operate_ai,
                    "{hull} does not declare `{axis_name}`. Since #704 deleted \
                     operate_helm_ai there is no coarse fallback: an undeclared axis \
                     resolves to ControlSource::Human, its per-axis system stands down, \
                     and nothing writes that intent component at all — the hull silently \
                     loses the behaviour. Declare it in the hull TOML with the same owner \
                     as the coarse `helm`"
                );
            }
        }
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
    /// That the *per-axis* systems produced the intent needed proving while the
    /// monolith was alive, and the proof was its stand-down: this hull declares
    /// every axis and the NPC spawn path backfills each to AI, so
    /// `operate_helm_ai` skipped both writes. Since #704 deleted it the point is
    /// simply structural — a non-zero intent has no other possible writer.
    #[test]
    fn shipped_hull_config_drives_the_per_axis_helm_systems() {
        let resolver =
            resolver_from_shipped_hull(include_str!("../assets/entities/pirate_raider.toml"));

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
        // #801: the shipped hull no longer declares a coarse helm at all —
        // the per-axis declarations above are the whole story.

        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        install_control_sources(&mut app, &resolver);

        tick(&mut app);

        assert!(
            get_thrust_input(&mut app) > 0.0,
            "ai_helm_thrust must drive a shipped hull's throttle toward a Reach anchor \
             (since #704 it is the thrust axis's only AI writer)"
        );
        assert!(
            get_steering_input(&mut app).abs() > 0.0,
            "ai_helm_steering must drive a shipped hull's yaw toward a Reach anchor \
             (since #704 it is the steering axis's only AI writer)"
        );
    }

    /// Ported in #704 from `shipped_hull_per_axis_intent_matches_the_coarse_path`,
    /// which pinned the #800 migration on a real hull: the per-axis path had to
    /// reproduce the monolith's intent exactly, so `run(&shipped)` had to equal
    /// `run(&pre_800)`. `pre_800` *is* the monolith path, so the delete removes
    /// the right-hand side of that equality outright.
    ///
    /// Kept, with both terms retained and the question changed from "do these
    /// agree?" to "which of these still drives the ship?". That is the honest
    /// successor: the old test's whole point was that the two paths were
    /// interchangeable on shipped content, and #704's point is that only one of
    /// them exists. Same hull, same resolver, same objective, same measurement.
    ///
    /// The `pre_800` arm is what makes this more than a restatement of
    /// `shipped_hull_config_drives_the_per_axis_helm_systems`: it pins that the
    /// hull's *declarations* are load-bearing. Strip `helm-thrust`/`helm-steering`
    /// back out of `pirate_raider.toml` and the shipped arm keeps passing on a
    /// coarse fallback if one ever returns — this arm would not.
    #[test]
    fn shipped_hull_helm_is_driven_by_the_per_axis_declarations_alone() {
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
            resolver_from_shipped_hull(include_str!("../assets/entities/pirate_raider.toml"));

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

        let shipped_intent = run(&shipped);
        assert!(
            shipped_intent.0 > 0.0 && shipped_intent.1.abs() > 0.0,
            "a shipped hull's declared per-axis systems must drive it toward a Reach \
             anchor (got {shipped_intent:?})"
        );
        assert_eq!(
            run(&pre_800),
            (0.0, 0.0),
            "with helm-thrust/helm-steering undeclared the hull's coarse helm is on AI \
             and nothing else is — the shape operate_helm_ai used to serve. Since #704 \
             deleted it that ship must not move: the axis declarations, not the coarse \
             system, are what fly it"
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
            resolver_from_shipped_hull(include_str!("../assets/entities/pirate_raider.toml"));
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
        // The third assertion here used to be a `nav_goal` probe for "did the
        // AiMemory mutation get committed?" — the #701 commit rule's half of
        // this test. #702 made `operate_helm` pure, so there is no commit to
        // observe and no half-dead-AI failure mode to guard: a system that runs
        // computes its axis from the shared surfaces and writes it, full stop.
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

    /// AC3 (Retreat consumer, unresolvable case): a Retreat naming an anchor
    /// the world does not declare resolves to nowhere and leaves the ship idle.
    ///
    /// This asserted the opposite until #702: an *empty*-anchor Retreat — which
    /// is what `aggregate_doctrine_blackboards` synthesised below a
    /// `[behaviour] retreat_hull_threshold` — used to fall back to the ship's
    /// `AiMemory.home_position`. Both the injector and `home_position` are gone.
    /// The fallback only ever looked like a safety net: `home_position` was
    /// never seeded in production, so "retreat home" meant "fly to world
    /// origin" on every shipped ship. Retreat is authored doctrine with a real
    /// anchor now (see `assets/entities/pirate_raider.toml`), and an anchor that
    /// resolves to nothing steers nowhere — see
    /// `ai_helm_steering_retreats_toward_anchor` for the resolvable case.
    #[test]
    fn ai_helm_steering_retreat_with_unknown_anchor_does_not_steer() {
        let mut app = test_app();
        // No anchors in the world config → the Retreat cannot resolve, and
        // there is no lower-priority objective to fall through to.
        set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective("", 90.0)]);
        app.insert_resource(crate::world::config::WorldConfig::default());
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "a Retreat that names nowhere must not steer; the old home_position \
             fallback made this a flight to world origin"
        );
        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "and must not throttle up either"
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

    // ── Per-axis helm AI: impulse (issue #703) ─────────────────────────────

    /// AC1: `ai_helm_impulse` writes `ImpulseCommand`, gating on helm-impulse
    /// alone. The anchor is dead ahead down -Z at 500 units — past the
    /// 200-unit `engage_distance` and inside the angle tolerance — so the
    /// decision is `Engage`.
    #[test]
    fn ai_helm_impulse_engages_toward_a_distant_target_ahead() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Charging,
            "ai_helm_impulse must command a charge toward a distant anchor dead ahead"
        );
    }

    /// AC1: the gate is real. Identical geometry to the test above, but
    /// helm-impulse is left at its Human default — and the coarse helm is
    /// Human too, so nothing may command the drive.
    #[test]
    fn ai_helm_impulse_does_not_write_when_its_system_is_human() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_impulse_system_id(),
            ControlSource::Human,
        );

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "helm-impulse under human control must not be commanded by ai_helm_impulse"
        );
    }

    /// AC1, the deactivate half: inside `cancel_distance` with a charge already
    /// running, `ai_helm_impulse` must stand the drive down. The command starts
    /// at `Charging`, so `Idle` here is an observed write and not the default.
    #[test]
    fn ai_helm_impulse_cancels_when_the_target_is_close() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        // 20 units out — inside the 40-unit `cancel_distance`.
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -20.0]));
        // `decide_impulse` only cancels from a non-Idle phase.
        let mut state = crate::impulse::ImpulseState::new();
        state.start_charge();
        set_ship_impulse(&mut app, state);
        set_impulse_command(&mut app, crate::impulse::ImpulsePhase::Charging);

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "ai_helm_impulse must cancel the charge once the target is inside \
             cancel_distance; still Charging means it never wrote"
        );
    }

    /// AC3: `ai_helm_impulse` is `AiHighFidelity`-scoped. The demoted ship keeps
    /// its `ImpulseCommand` here only so a stray write would be observable.
    #[test]
    fn ai_helm_impulse_is_scoped_to_high_fidelity() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));

        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ai_plugin::AiHighFidelity>();

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "ai_helm_impulse must not touch a ship without AiHighFidelity"
        );
    }

    /// A live Helm objective is a precondition: `operate_helm_ai`'s
    /// no-objective branch `continue`d before its impulse block rather than
    /// cancelling, and `ai_helm_impulse` inherited that when #703 extracted it —
    /// a behaviour it now carries alone, the monolith having been deleted in
    /// #704. A lull in objectives is not a reason to drop an in-progress
    /// charge.
    ///
    /// Pins the *behaviour*, not any one line. `ai_helm_impulse` enforces it
    /// three times over — the `has_helm_objective` early-out,
    /// `resolve_helm_target_position`'s top-objective filter, and the `top_obj`
    /// lookup behind `use_impulse` — each carrying the same `score > 0.0 &&
    /// Helm`-relevant predicate. Deleting any one or two of them leaves this
    /// green; only losing all three turns it red. That is a statement about the
    /// implementation being belt-and-braces, not about the test being weak: the
    /// behaviour it asserts (a dead objective must not cancel a live charge) is
    /// the thing that matters, and it is unreachable by any single regression.
    #[test]
    fn ai_helm_impulse_leaves_the_drive_alone_without_a_helm_objective() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        // Inside cancel_distance with a charge running: the one geometry where
        // a system that ignored the objective gate would visibly cancel.
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -20.0]));
        let mut state = crate::impulse::ImpulseState::new();
        state.start_charge();
        set_ship_impulse(&mut app, state);
        set_impulse_command(&mut app, crate::impulse::ImpulsePhase::Charging);
        // Same objective, scored dead: `has_helm_objective` requires score > 0.
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 0.0)]);

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Charging,
            "with no live Helm objective ai_helm_impulse must leave ImpulseCommand \
             untouched, as the monolith does"
        );
    }

    /// `use_impulse` is TOML-authored per doctrine entry (AGENTS.md rule 11):
    /// an objective whose doctrine forbids impulse must not engage it, however
    /// inviting the geometry.
    #[test]
    fn ai_helm_impulse_honours_toml_authored_use_impulse() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
        // The same doctrine entry `impulse_ai_app` installs, with the one
        // authored field flipped.
        set_behaviour_section(
            &mut app,
            crate::entity_config::BehaviourConfig {
                doctrine: vec![crate::entity_config::DoctrineObjective {
                    id: "reach-station-alpha".into(),
                    use_impulse: Some(false),
                    target_speed: 0.8,
                    maintain_range: 25.0,
                    ..Default::default()
                }],
                ..Default::default()
            },
        );

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "[[behaviour.doctrine]] use_impulse = false must veto the engage that \
             ai_helm_impulse_engages_toward_a_distant_target_ahead proves is otherwise \
             reachable from this geometry"
        );
    }

    /// `ai_helm_impulse` must resolve its target from the *same* waypoint the
    /// rest of the helm AI is steering at this tick — one leg further on than the
    /// tick started, because `advance_objective_cursors` (`SimSet::Modifiers`)
    /// runs before this system and has already advanced the cursor off the
    /// waypoint underfoot.
    ///
    /// The name is historical, and so is the failure it guards: this system used
    /// to reach that leg by *replaying* the helm decision on a scratch clone of
    /// `AiMemory`, which only matched the committer's view while the memory was
    /// still pre-commit — hence `.before(operate_helm_ai)`. #702 deleted
    /// `AiMemory` and with it the clone, the replay and the commit; the cursor is
    /// now a read-only surface that cannot move underneath this system at all
    /// (see the registration comment on `ai_helm_impulse`). What is left to pin
    /// is the answer, not the mechanism that reached it.
    ///
    /// The patrol makes the leg observable. wp-a and wp-b both sit on the ship,
    /// so the cursor advances off wp-a during `Modifiers`; wp-c is 500 units dead
    /// ahead.
    ///
    ///   correct → cursor 1 → target wp-b underfoot → inside cancel_distance
    ///             with a charge running → **Cancel**
    ///   broken  → a leg out of step → target wp-c at 500 → far → NoChange →
    ///             command stays `Charging`
    ///
    /// So the correct answer is also the one that performs a write, which keeps
    /// a do-nothing regression from passing this too.
    #[test]
    fn ai_helm_impulse_reads_pre_commit_memory() {
        let mut app = impulse_ai_app(patrol_scored_objective(vec!["wp-a", "wp-b", "wp-c"], 20.0));
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.anchors.insert("wp-a".into(), [0.0, 0.0, 0.0]);
        cfg.anchors.insert("wp-b".into(), [0.0, 0.0, 0.0]);
        cfg.anchors.insert("wp-c".into(), [0.0, 0.0, -500.0]);
        app.insert_resource(cfg);
        set_behaviour_section(&mut app, impulse_doctrine("obj-defend"));
        // Coarse helm on AI, as this test has always run it. (This was once
        // load-bearing: it put `operate_helm_ai` in the tick as the committer
        // this system had to run ahead of. There is no committer now.)
        set_helm_control_source(&mut app, ControlSource::Ai);
        let mut state = crate::impulse::ImpulseState::new();
        state.start_charge();
        set_ship_impulse(&mut app, state);
        set_impulse_command(&mut app, crate::impulse::ImpulsePhase::Charging);

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "ai_helm_impulse must resolve its target from this tick's advance (wp-b, \
             underfoot) — still Charging means it replayed the decision on memory \
             operate_helm_ai had already committed and skipped a leg to wp-c"
        );
    }

    /// The coverage gap #800 was caught by, applied to impulse: every test above
    /// hand-builds its control sources, so all of them would pass with
    /// `helm-impulse` declared in zero TOMLs. This one refuses to hand-build.
    ///
    /// `alliance_cruiser` declares the coarse helm *and* helm-impulse, so with
    /// the station unmanned the monolith stands down from the impulse decision
    /// and a `Charging` command has nowhere else to come from.
    #[test]
    fn shipped_hull_config_drives_ai_helm_impulse() {
        let resolver =
            resolver_from_shipped_hull(include_str!("../assets/entities/alliance_cruiser.toml"));
        assert!(
            resolver
                .policy_for(&crate::system_registry::helm_impulse_system_id())
                .operate_ai,
            "the shipped hull must declare helm-impulse, or ai_helm_impulse is dormant \
             in shipped content"
        );
        // #801: the shipped hull no longer declares a coarse helm at all.

        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
        install_control_sources(&mut app, &resolver);

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Charging,
            "ai_helm_impulse must drive a shipped hull's impulse decision \
             (operate_helm_ai stands down from it here)"
        );
    }

    // ── Per-axis helm AI: lateral thrust (issue #703) ──────────────────────

    /// An obstacle the default avoidance tuning ignores and an authored 60-unit
    /// `avoidance_buffer` treats as a threat (radius 0 + 1 + 60 = 61 > 40), on a
    /// stationary ship so the look-ahead cannot also move. Any nonzero lateral
    /// is this obstacle and nothing else.
    fn lateral_dodge_app() -> App {
        let mut app = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut app, [4.0, 0.0, -40.0], 1.0);
        app
    }

    /// AC2, the gate collapse itself. `ai_helm_lateral_thrust`'s old `L && !C`
    /// gate stood the system down whenever the coarse helm was AI, because the
    /// monolith owned `LateralThrustInput` outright in that case. Since #703 the
    /// monolith stands down instead — so if the `!C` half had been left in
    /// place, this configuration would leave the dodge with **no writer at all**
    /// rather than two.
    ///
    /// That asymmetry is what this test exploits: a nonzero lateral proves the
    /// half came off. (It cannot distinguish one writer from two — both compute
    /// the identical dodge from identical inputs — which is what
    /// `helm_writers_are_invariant_under_coarse_policy` is for.)
    #[test]
    fn ai_helm_lateral_thrust_dodges_when_the_coarse_helm_is_also_ai() {
        let mut app = lateral_dodge_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick_twice(&mut app);

        assert!(
            lateral_intent(&mut app).abs() > 0.0,
            "with helm-lateral-thrust on AI the dodge must be written whatever the \
             coarse helm is doing; zero means the collapsed gate stood the system \
             down and the monolith had already stood down too"
        );
    }

    /// AC3, and a behaviour change #697 declined to make:
    /// `ai_helm_lateral_thrust` is now `AiHighFidelity`-scoped like its two
    /// siblings. The coarse helm stays Human, so the monolith (also scoped)
    /// cannot cover for it.
    #[test]
    fn ai_helm_lateral_thrust_is_scoped_to_high_fidelity() {
        let mut app = lateral_dodge_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ai_plugin::AiHighFidelity>();

        tick_twice(&mut app);

        assert_eq!(
            lateral_intent(&mut app),
            0.0,
            "ai_helm_lateral_thrust must not touch a ship without AiHighFidelity"
        );
    }

    /// The monolith zeroes `LateralThrustInput` when no Helm objective is live,
    /// so the shared integrator decelerates the dodge off through the normal
    /// physics curve. #697 `continue`d instead, latching the last dodge forever.
    /// That divergence had to close before the monolith could stand down.
    #[test]
    fn ai_helm_lateral_thrust_zeroes_the_dodge_without_a_helm_objective() {
        let mut app = lateral_dodge_app();
        tick_twice(&mut app);
        assert!(
            lateral_intent(&mut app).abs() > 0.0,
            "precondition: the obstacle produces a dodge while an objective is live"
        );

        // Objectives go quiet; the obstacle does not move.
        set_ship_blackboard_objectives(&mut app, vec![]);
        tick(&mut app);

        assert_eq!(
            lateral_intent(&mut app),
            0.0,
            "no live Helm objective must zero the dodge, not latch the last one"
        );
    }

    fn set_lateral_intent(app: &mut App, value: f32) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<LateralThrustInput>()
            .expect("ship must carry LateralThrustInput")
            .0 = value;
    }

    /// A sentinel the helm-AI maths can never produce — intents are
    /// normalised to [-1, 1] — so a frame that leaves the sentinel standing
    /// is a frame the probed system did not run on.
    const CADENCE_SENTINEL: f32 = 123.456;

    /// Drive `app` at 10 ms per frame — under the 33.3 ms shared sim-tick
    /// period, i.e. what a 60 Hz rAF-driven host actually does — and count
    /// the frames on which the probed system ran. `arm` re-stamps the probe
    /// before each frame; `ran_this_frame` reads it back after.
    ///
    /// The shared AI-helm sim tick (issue #803) is a real fixed-rate
    /// throttle, not a formality. Production `Update` is rAF-driven:
    /// `server/bridge.rs` installs `WinitSettings` with
    /// `UpdateMode::Continuous` for both focused and unfocused, so a 60 Hz
    /// host frames at ~16.7 ms — under the period — and the helm AI must
    /// recompute on only *some* frames. Without the gate the AI's decision
    /// cadence would follow the host's display refresh rate (a 144 Hz host
    /// deciding on ~4x fresher data than a 60 Hz one), which is exactly the
    /// nondeterminism PRD #620's lockstep has to eliminate. Until #803 only
    /// the lateral axis was throttled (by the private `AiLateralThrustTimer`);
    /// all four per-axis systems now share one cadence, and there is one of
    /// these tests per system.
    fn count_sim_tick_runs(
        app: &mut App,
        mut arm: impl FnMut(&mut App),
        mut ran_this_frame: impl FnMut(&mut App) -> bool,
    ) -> (usize, usize) {
        const FRAME_MS: u64 = 10;
        const TICKS: usize = 12;
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(FRAME_MS),
        ));
        let mut ran = 0usize;
        for _ in 0..TICKS {
            arm(app);
            tick(app);
            if ran_this_frame(app) {
                ran += 1;
            }
        }
        (ran, TICKS)
    }

    /// Shared assertions for the four cadence tests. `ran > 0` guards the
    /// probe itself; `ran <= ticks / 2` is the throttle. Over 12 frames x
    /// 10 ms the 33.3 ms timer fires ~3 times, plus the first frame's
    /// `AiHelmTickReady`-initialises-`true` free run (mirroring
    /// `AiSnapshotReady`); `ticks / 2` leaves generous margin while still
    /// failing loudly if the gate goes away.
    fn assert_shared_sim_tick_cadence(system: &str, (ran, ticks): (usize, usize)) {
        assert!(
            ran > 0,
            "precondition: {ticks} frames x 10 ms spans several 33.3 ms periods, so \
             {system} must run at least once — 0 runs means the probe is broken and \
             this test proves nothing about cadence"
        );
        assert!(
            ran <= ticks / 2,
            "the shared AI-helm sim tick must throttle {system}: at 10 ms/frame — \
             under the 33.3 ms period, i.e. what a 60 Hz rAF-driven host actually \
             does — it ran on {ran} of {ticks} frames. Running every frame means the \
             run_if(ai_helm_tick_ready) gate is gone and the decision cadence \
             follows display refresh rate again (PRD #620)"
        );
    }

    fn set_thrust_intent(app: &mut App, value: f32) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ThrustInput>()
            .expect("ship must carry ThrustInput")
            .0 = value;
    }

    fn set_steering_intent(app: &mut App, value: f32) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<SteeringInput>()
            .expect("ship must carry SteeringInput")
            .0 = value;
    }

    /// The coarse helm is set to **AI** purely to isolate the writer, and that
    /// is load-bearing: `process_helm_inputs` also writes `LateralThrustInput`
    /// (from `LastHelmInput`), on its own 30 Hz `HelmInputTimer`, and it only
    /// stands down when the *coarse* helm is AI-operated — the lateral policy
    /// does not stop it. Left Human (as `lateral_thrust_ai_app` leaves it) it
    /// would clear the sentinel on its own cadence and this test would pass
    /// even with `ai_helm_lateral_thrust` disabled outright. With the coarse
    /// helm on AI, `process_helm_inputs` early-returns, leaving this system
    /// the sole writer.
    #[test]
    fn ai_helm_lateral_thrust_runs_on_the_shared_sim_tick_not_per_frame() {
        let mut app = lateral_dodge_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        let counts = count_sim_tick_runs(
            &mut app,
            |app| set_lateral_intent(app, CADENCE_SENTINEL),
            |app| lateral_intent(app) != CADENCE_SENTINEL,
        );
        assert_shared_sim_tick_cadence("ai_helm_lateral_thrust", counts);
    }

    /// AC (issue #803): `ai_helm_thrust` used to run once per rendered frame;
    /// it must now run on the shared sim tick. `set_per_axis_helm_ai` puts the
    /// thrust axis on AI, so `process_helm_inputs` skips the axis and this
    /// system is `ThrustInput`'s sole writer — the sentinel can only be
    /// cleared by it.
    #[test]
    fn ai_helm_thrust_runs_on_the_shared_sim_tick_not_per_frame() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        let counts = count_sim_tick_runs(
            &mut app,
            |app| set_thrust_intent(app, CADENCE_SENTINEL),
            |app| get_thrust_input(app) != CADENCE_SENTINEL,
        );
        assert_shared_sim_tick_cadence("ai_helm_thrust", counts);
    }

    /// AC (issue #803): `ai_helm_steering` on the shared sim tick — same
    /// isolation argument as the thrust test.
    #[test]
    fn ai_helm_steering_runs_on_the_shared_sim_tick_not_per_frame() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        let counts = count_sim_tick_runs(
            &mut app,
            |app| set_steering_intent(app, CADENCE_SENTINEL),
            |app| get_steering_input(app) != CADENCE_SENTINEL,
        );
        assert_shared_sim_tick_cadence("ai_helm_steering", counts);
    }

    /// AC (issue #803): `ai_helm_impulse` on the shared sim tick.
    /// `ImpulseCommand` is an enum, so the probe is a reset-and-observe
    /// rather than a sentinel: each frame re-arms the drive to `Idle` (both
    /// the command and the `ShipImpulse` phase, so `decide_impulse` sees the
    /// same Engage-able geometry every time — the anchor 500 units dead
    /// ahead, past `engage_distance`); a frame that ends `Charging` is a
    /// frame the system ran on.
    #[test]
    fn ai_helm_impulse_runs_on_the_shared_sim_tick_not_per_frame() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));

        let counts = count_sim_tick_runs(
            &mut app,
            |app| {
                set_ship_impulse(app, crate::impulse::ImpulseState::new());
                set_impulse_command(app, crate::impulse::ImpulsePhase::Idle);
            },
            |app| get_impulse_command(app) == crate::impulse::ImpulsePhase::Charging,
        );
        assert_shared_sim_tick_cadence("ai_helm_impulse", counts);
    }

    /// The shared sim-tick rate is TOML-authored (`[global] ai_helm_tick_hz`),
    /// not hardcoded: `tick_ai_helm_timer` must reconcile the timer period
    /// against a loaded `WorldConfig` that authors a different rate. At an
    /// authored 100 Hz the 10 ms frames land exactly on the period, so the
    /// lateral dodge recomputes every frame — where the default 30 Hz gate
    /// (asserted by the cadence tests above) allows at most half.
    #[test]
    fn ai_helm_tick_rate_is_reconfigured_from_world_config() {
        let mut app = lateral_dodge_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.global.ai_helm_tick_hz = 100.0;
        // `lateral_dodge_app` leaves no WorldConfig installed; the dodge only
        // needs the snapshot obstacle, so the empty-anchor config is inert
        // apart from the authored tick rate.
        app.insert_resource(cfg);

        let (ran, ticks) = count_sim_tick_runs(
            &mut app,
            |app| set_lateral_intent(app, CADENCE_SENTINEL),
            |app| lateral_intent(app) != CADENCE_SENTINEL,
        );
        assert!(
            ran > ticks / 2,
            "with [global] ai_helm_tick_hz = 100 the 10 ms period fires every frame, \
             so the dodge must recompute on (nearly) all of them — {ran} of {ticks} \
             means tick_ai_helm_timer never applied the TOML-authored rate"
        );
    }

    /// The #800 coverage gap, applied to lateral thrust. `alliance_cruiser`
    /// declares the coarse helm *and* helm-lateral-thrust, so an unmanned Helm
    /// puts both on AI — the exact combination the old `!C` half made
    /// unreachable, and the one every hand-built test above misses.
    #[test]
    fn shipped_hull_config_drives_ai_helm_lateral_thrust() {
        let resolver =
            resolver_from_shipped_hull(include_str!("../assets/entities/alliance_cruiser.toml"));
        assert!(
            resolver
                .policy_for(&crate::system_registry::lateral_thrust_system_id())
                .operate_ai,
            "the shipped hull must declare helm-lateral-thrust, or ai_helm_lateral_thrust \
             is dormant in shipped content"
        );
        // #801: the shipped hull no longer declares a coarse helm at all.

        let mut app = lateral_dodge_app();
        install_control_sources(&mut app, &resolver);

        tick_twice(&mut app);

        assert!(
            lateral_intent(&mut app).abs() > 0.0,
            "ai_helm_lateral_thrust must drive a shipped hull's dodge (since #704 it is \
             the lateral axis's only AI writer)"
        );
    }

    /// Pins the per-axis gate algebra: **the coarse helm policy `C` is not an
    /// input to any intent writer.** Each writer is a function of its own axis
    /// alone — this test sweeps C across all three control sources for every
    /// fixed (T,S,L,I) and demands the whole outcome (every component's
    /// writer) be invariant under it. It also pins the coverage half: each
    /// component is written exactly when its own axis is AI.
    ///
    /// This is a **model** test: it states the gate algebra against the policy
    /// resolver, it does not run the systems. A coarse fallback re-introduced
    /// into `ai_helm_thrust` leaves this test green; what catches that is
    /// `coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do` and
    /// its siblings, which exercise the real systems. Read this test as the
    /// specification and those as the enforcement.
    #[test]
    fn helm_writers_are_invariant_under_coarse_policy() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        // #801: "helm" is not a system; seeding it (the C dimension) must
        // have no influence on any writer — which is what this test proves.
        let coarse = crate::messages::SystemId(crate::system_registry::HELM_STATION_ID.to_string());
        let thrust = crate::system_registry::helm_thrust_system_id();
        let steering = crate::system_registry::helm_steering_system_id();
        let lateral = crate::system_registry::lateral_thrust_system_id();
        let impulse = crate::system_registry::helm_impulse_system_id();

        let all = [
            ControlSource::Human,
            ControlSource::Ai,
            ControlSource::Offline,
        ];

        // Every writer decision for one ship in one tick: which system writes
        // each intent component.
        #[derive(Debug, PartialEq, Eq)]
        struct HelmWriters {
            thrust: bool,
            steering: bool,
            lateral: bool,
            impulse: bool,
        }

        let mut saw_all_four_running = false;

        for t in all {
            for s in all {
                for l in all {
                    for i in all {
                        // Sweep the coarse source innermost so that, for one
                        // fixed (T,S,L,I), we can compare the outcome across all
                        // three coarse sources and demand they agree.
                        let mut outcome_per_coarse = Vec::new();

                        for c in all {
                            let mut r = ControlSourceResolver::new();
                            r.set(coarse.clone(), c);
                            r.set(thrust.clone(), t);
                            r.set(steering.clone(), s);
                            r.set(lateral.clone(), l);
                            r.set(impulse.clone(), i);

                            // The gate each system actually applies: its own
                            // system alone (#800 for thrust/steering, #703 for
                            // lateral/impulse). No system reads the coarse
                            // policy — the one that did is gone.
                            let tt = r.policy_for(&thrust).operate_ai;
                            let ss = r.policy_for(&steering).operate_ai;
                            let ll = r.policy_for(&lateral).operate_ai;
                            let ii = r.policy_for(&impulse).operate_ai;

                            let writers = HelmWriters {
                                thrust: tt,
                                steering: ss,
                                lateral: ll,
                                impulse: ii,
                            };

                            // Each component is written exactly when its own
                            // axis is AI — never otherwise (no coarse fallback),
                            // never dropped when it is (no lost writer).
                            for (name, own_axis_is_ai, written) in [
                                ("ThrustInput", tt, writers.thrust),
                                ("SteeringInput", ss, writers.steering),
                                ("LateralThrustInput", ll, writers.lateral),
                                ("ImpulseCommand", ii, writers.impulse),
                            ] {
                                assert_eq!(
                                    written, own_axis_is_ai,
                                    "{name} must be written exactly when its own axis is \
                                     AI-operated: coarse={c:?} thrust={t:?} steering={s:?} \
                                     lateral={l:?} impulse={i:?}"
                                );
                            }

                            if tt && ss && ll && ii {
                                saw_all_four_running = true;
                            }

                            outcome_per_coarse.push(writers);
                        }

                        // The #704 invariant: nothing above depended on `c`.
                        for (idx, other) in outcome_per_coarse.iter().enumerate().skip(1) {
                            assert_eq!(
                                &outcome_per_coarse[0], other,
                                "the coarse helm policy must not influence any helm-AI \
                                 writer — #704 deleted the only system that read it. \
                                 Differed between coarse={:?} and coarse={:?} at \
                                 thrust={t:?} steering={s:?} lateral={l:?} impulse={i:?}",
                                all[0], all[idx]
                            );
                        }
                    }
                }
            }
        }

        // The shipped-hull shape (every axis declared, station backfilled to AI)
        // must be inside the space this test covers — that combination was
        // unreachable under the old per-ship gates and is the whole point of
        // #800, #703 and #704.
        assert!(
            saw_all_four_running,
            "the shipped-hull all-AI combination must be covered"
        );
    }

    /// Ported in #704 from `coarse_helm_ai_result_is_unchanged_by_per_axis_systems`,
    /// which pinned #800's stand-down: with the coarse helm on AI the monolith
    /// owned the write and the per-axis systems stood down, so turning the fine
    /// systems on changed nothing and the two runs were bit-identical.
    ///
    /// Both terms of that equality were the monolith's output, so the delete
    /// removes the property rather than moving it. Kept — same fixture, same two
    /// runs, same measurement — with the assertion inverted, because inverting it
    /// is precisely what #704 does: the coarse system no longer writes anything,
    /// so the two runs must now *differ*, and the difference is the whole delete.
    /// Equality here would now mean either a surviving coarse fallback (both
    /// non-zero) or a dead per-axis path (both zero); the old test could not tell
    /// you about either, and this one fails on both.
    ///
    /// This had an end-to-end companion, `coarse_helm_alone_commits_no_memory`,
    /// pinning that the coarse system wrote no `AiMemory` while this one pins
    /// that it writes no intent. #702 deleted `AiMemory`, so the companion had
    /// nothing left to observe and went with it; "writes no intent" is now the
    /// whole of the property.
    #[test]
    fn coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do() {
        let anchor = "station-alpha";

        let coarse_only = {
            let mut app = test_app();
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            set_coarse_helm_only_ai(&mut app);
            tick(&mut app);
            (get_thrust_input(&mut app), get_steering_input(&mut app))
        };

        let coarse_plus_fine = {
            let mut app = test_app();
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            set_helm_control_source(&mut app, ControlSource::Ai);
            tick(&mut app);
            (get_thrust_input(&mut app), get_steering_input(&mut app))
        };

        assert_eq!(
            coarse_only,
            (0.0, 0.0),
            "the coarse helm system has no AI behaviour of its own since #704 deleted \
             operate_helm_ai; on its own it must leave the intent components untouched \
             (non-zero = a coarse fallback has come back)"
        );
        assert!(
            coarse_plus_fine.0 > 0.0 && coarse_plus_fine.1.abs() > 0.0,
            "declaring the axes is what drives the ship now: the per-axis systems must \
             produce the intent the monolith used to (got {coarse_plus_fine:?})"
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
        let target_uuid_str = target_uuid.clone();
        runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
        app.insert_resource(runtime);
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Tactical's lock: what the helm pursues (issue #702).
        set_ship_weapons_target(&mut app, &target_uuid_str);
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
        let target_uuid_str = target_uuid.clone();
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

        // Tactical's lock: what the helm pursues (issue #702).
        set_ship_weapons_target(&mut app, &target_uuid_str);
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
        let destroy_uuid_str = destroy_uuid.clone();
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

        // Tactical's lock: what the helm pursues (issue #702).
        set_ship_weapons_target(&mut app, &destroy_uuid_str);
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

    // ── Channel-3 Navigation→Helm clearance (issue #702) ──────────────────
    //
    // `cleared_nav_waypoint` is where the Channel-3 lag lives on the read side:
    // the Helm follows the ship's `NavigationWaypoint` only while its
    // `HelmWaypointClearance` names that waypoint's `generation`. These pin the
    // gate itself — deleting the comparison must not be a silent no-op.

    /// The happy path: clearance matches the waypoint's generation, so the Helm
    /// is cleared to fly it.
    #[test]
    fn cleared_nav_waypoint_returns_the_waypoint_when_the_clearance_matches() {
        let waypoint = crate::navigation_plugin::NavigationWaypoint::new(WaypointMode::Free {
            x: 5.0,
            z: -7.0,
        });
        let clearance = HelmWaypointClearance(Some(waypoint.generation()));

        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), Some(&clearance)),
            Some([5.0, -7.0])
        );
    }

    /// The lag itself: Navigation has set a *new* waypoint, but the `NavigateTo`
    /// carrying its generation is still in the coordination queue. The Helm must
    /// not fly it yet — it has been given the waypoint but not the order.
    ///
    /// This is why the clearance is a generation rather than a bool: a bool
    /// ("Navigation has spoken") would go true once and wave every subsequent
    /// waypoint straight through, so only the first order would ever be delayed.
    #[test]
    fn cleared_nav_waypoint_withholds_a_waypoint_newer_than_the_clearance() {
        let mut waypoint = crate::navigation_plugin::NavigationWaypoint::new(WaypointMode::Free {
            x: 5.0,
            z: -7.0,
        });
        // The Helm was cleared for this one, and is flying it.
        let clearance = HelmWaypointClearance(Some(waypoint.generation()));
        assert!(cleared_nav_waypoint(Some(&waypoint), Some(&clearance)).is_some());

        // Navigation now re-tasks the ship. The order has not arrived yet.
        waypoint.set(WaypointMode::Free { x: 900.0, z: 900.0 });

        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), Some(&clearance)),
            None,
            "a re-tasked waypoint must re-incur the Channel-3 lag; without this \
             every waypoint after the first would be followed instantly"
        );

        // …and once `process_coordination_lag` latches the new generation, it is.
        let caught_up = HelmWaypointClearance(Some(waypoint.generation()));
        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), Some(&caught_up)),
            Some([900.0, 900.0])
        );
    }

    /// A ship never cleared for anything follows nothing.
    #[test]
    fn cleared_nav_waypoint_is_none_without_a_clearance() {
        let waypoint = crate::navigation_plugin::NavigationWaypoint::new(WaypointMode::Free {
            x: 5.0,
            z: -7.0,
        });

        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), Some(&HelmWaypointClearance(None))),
            None,
            "never cleared = never followed"
        );
        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), None),
            None,
            "a ship with no clearance component at all is never cleared"
        );
        assert_eq!(
            cleared_nav_waypoint(None, Some(&HelmWaypointClearance(Some(1)))),
            None,
            "a clearance with no waypoint names nowhere"
        );
    }

    /// Through the real system: an uncleared waypoint does not move the ship,
    /// and the same waypoint does once the clearance lands.
    ///
    /// The unit tests above pin `cleared_nav_waypoint`; this pins that
    /// `ai_helm_thrust` actually consults it rather than reading the waypoint
    /// directly and skipping the lag.
    #[test]
    fn ai_helm_flies_the_nav_waypoint_only_once_cleared() {
        fn app_with_waypoint(clear_it: bool) -> App {
            let mut app = test_app();
            set_helm_control_source(&mut app, ControlSource::Ai);
            // A Helm-relevant objective that cannot resolve, so the only thing
            // left to fly is the Navigation waypoint.
            set_ship_blackboard_objectives(
                &mut app,
                vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
            );
            if clear_it {
                set_cleared_nav_waypoint(&mut app, 0.0, -900.0);
            } else {
                // Waypoint set, order not yet delivered.
                let ship = find_ship_entity(&mut app);
                let mut entity = app.world_mut().entity_mut(ship);
                let mut waypoint = entity
                    .get_mut::<crate::navigation_plugin::NavigationWaypoint>()
                    .expect("ship must carry NavigationWaypoint");
                waypoint.set(WaypointMode::Free { x: 0.0, z: -900.0 });
            }
            tick(&mut app);
            app
        }

        assert_eq!(
            get_thrust_input(&mut app_with_waypoint(false)),
            0.0,
            "the waypoint is set but the Channel-3 order has not been delivered, \
             so the AI helm must not fly it yet"
        );
        assert!(
            get_thrust_input(&mut app_with_waypoint(true)) > 0.0,
            "once process_coordination_lag latches the clearance, the same \
             waypoint must be flown"
        );
    }

    /// Rule-6 symmetry, end to end over the wire: a *human* navigation
    /// officer's admitted `SetNavigationWaypoint` reaches an AI Helm exactly
    /// as an AI-set waypoint does — the same `NavigateTo` clearance, the same
    /// Channel-3 delivery lag, the same `HelmWaypointClearance` latch — and
    /// the AI Helm then flies it.
    ///
    /// Before the fix only `operate_navigation_ai` enqueued the clearance, so
    /// a human-set waypoint sat on the shared `NavigationWaypoint` forever
    /// unfollowed: `cleared_nav_waypoint` withholds any generation the
    /// clearance has not latched, and nothing ever latched one.
    #[test]
    fn human_set_nav_waypoint_eventually_clears_and_the_ai_helm_flies_it() {
        let mut app = test_app();
        // The waypoint write path lives in NavigationPlugin
        // (`handle_navigation_waypoint`); its blackboard publisher needs the
        // client-config resource.
        app.add_plugins(crate::navigation_plugin::NavigationPlugin)
            .init_resource::<crate::lobby::server::ShipClientConfigResource>();

        // A human captain + navigation officer, game started; the Helm
        // station is unmanned and on AI.
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::Identify {
                token: "navigation".into(),
                name: "Decker".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::SelectStation {
                station: "Navigation".into(),
            },
        );
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SetReady { ready: true });
        push(
            &mut app,
            "navigation",
            ClientMessage::SetReady { ready: true },
        );
        tick(&mut app);

        set_helm_control_source(&mut app, ControlSource::Ai);
        // A Helm-relevant objective that cannot resolve, so the only thing
        // left to fly is the Navigation waypoint (same shape as
        // `ai_helm_flies_the_nav_waypoint_only_once_cleared`).
        set_ship_blackboard_objectives(
            &mut app,
            vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
        );

        // The human sets the waypoint over the wire.
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: crate::messages::SystemControlPayload::SetNavigationWaypoint {
                    x: 0.0,
                    z: -900.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);

        let ship = find_ship_entity(&mut app);
        let generation = app
            .world()
            .entity(ship)
            .get::<crate::navigation_plugin::NavigationWaypoint>()
            .expect("ship must carry NavigationWaypoint")
            .generation();
        assert!(
            app.world()
                .entity(ship)
                .get::<crate::navigation_plugin::NavigationWaypoint>()
                .and_then(|w| w.snapshot())
                .is_some(),
            "the admitted SetNavigationWaypoint must set the shared waypoint"
        );
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            None,
            "the NavigateTo order must still be serving its Channel-3 lag"
        );
        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "the AI Helm must not fly a waypoint before the clearance lands"
        );

        // Serve the Channel-3 delivery lag (authored per hull; each tick
        // advances the manual clock by 200 ms), plus slack for the tick that
        // enqueues and the tick that delivers.
        let lag_secs = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipConfigComponent, With<Ship>>();
            q.single(app.world())
                .expect("ship config")
                .0
                .coordination_lag_secs
        };
        let ticks = (lag_secs / 0.2).ceil() as u32 + 4;
        for _ in 0..ticks {
            tick(&mut app);
        }

        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            Some(generation),
            "the human-set waypoint's NavigateTo must latch its generation \
             into the AI Helm's clearance once the lag is served"
        );
        assert!(
            get_thrust_input(&mut app) > 0.0,
            "once cleared, the AI Helm must fly the human-set waypoint — \
             rule-6 symmetry with the AI-set path"
        );
    }

    /// Waypoint clearance survives a helm control flip: a waypoint set while
    /// the helm is HUMAN-manned delivers as suppressed/popup (no latch); when
    /// the helm later flips to AI (disconnect → Backfill), the shared issuer
    /// re-issues the `NavigateTo` on the Human→AI edge, the order serves the
    /// normal Channel-3 lag, latches, and the AI helm flies the existing
    /// waypoint — no human re-set required, and no instant latch.
    #[test]
    fn waypoint_set_while_helm_human_is_flown_once_helm_flips_to_ai() {
        let mut app = test_app();
        app.add_plugins(crate::navigation_plugin::NavigationPlugin)
            .init_resource::<crate::lobby::server::ShipClientConfigResource>();

        // A human captain + navigation officer, game started. The helm axes
        // stay on their default Human control for now.
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::Identify {
                token: "navigation".into(),
                name: "Decker".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::SelectStation {
                station: "Navigation".into(),
            },
        );
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SetReady { ready: true });
        push(
            &mut app,
            "navigation",
            ClientMessage::SetReady { ready: true },
        );
        tick(&mut app);

        // A Helm-relevant objective that cannot resolve, so once the helm is
        // AI the only thing left to fly is the Navigation waypoint.
        set_ship_blackboard_objectives(
            &mut app,
            vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
        );

        // The human sets the waypoint over the wire while the helm is human.
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: crate::messages::SystemControlPayload::SetNavigationWaypoint {
                    x: 0.0,
                    z: -900.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);

        let ship = find_ship_entity(&mut app);
        let generation = app
            .world()
            .entity(ship)
            .get::<crate::navigation_plugin::NavigationWaypoint>()
            .expect("ship must carry NavigationWaypoint")
            .generation();

        // Serve well past the delivery lag with the helm still human: the
        // order routes to the human helm (suppress — human sender, human
        // target) and must NOT latch a clearance.
        let lag_secs = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipConfigComponent, With<Ship>>();
            q.single(app.world())
                .expect("ship config")
                .0
                .coordination_lag_secs
        };
        let ticks = (lag_secs / 0.2).ceil() as u32 + 4;
        for _ in 0..ticks {
            tick(&mut app);
        }
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            None,
            "an order delivered to a human helm must not latch a clearance"
        );
        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "no AI helm, no flight — nothing should be driving the thrust axis"
        );

        // The helm flips to AI (the disconnect → Backfill shape).
        set_helm_control_source(&mut app, ControlSource::Ai);

        // The clearance must not latch instantly — the re-issued order still
        // serves the authored Channel-3 delivery lag.
        tick(&mut app);
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            None,
            "the re-issued NavigateTo must serve the delivery lag, not latch instantly"
        );

        // Serve the lag (authored per hull), plus slack for the tick that
        // enqueues and the tick that delivers.
        for _ in 0..ticks {
            tick(&mut app);
        }
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            Some(generation),
            "after the helm flips to AI, the re-issued NavigateTo must latch \
             the existing waypoint's generation once the lag is served"
        );
        assert!(
            get_thrust_input(&mut app) > 0.0,
            "the AI helm must fly the waypoint that was set while the helm \
             was human — clearance survives the control flip"
        );
    }

    /// Regression (issue #696 review, finding 2): `[behaviour]
    /// waypoint_arrival_radius` is authored per entity template in TOML and
    /// read by the cursor evaluator at every LOD. The high-LOD helm's own
    /// turn-at-waypoint decision must agree with it rather than hardcoding
    /// `WAYPOINT_ARRIVAL_RADIUS` — otherwise a designer's widened radius is
    /// honoured for triggers but ignored for steering.
    ///
    /// Probed through the helm's *steering* rather than through a waypoint
    /// index (issue #702). The helm no longer keeps an index of its own to
    /// look at: `advance_objective_cursors` owns every cursor, and `helm_patrol`
    /// only reads. What the radius still decides here — and all this test ever
    /// really cared about — is the helm's own arrival branch: short of the
    /// radius it turns toward the waypoint; inside it, it flies straight
    /// through. That is directly observable.
    #[test]
    fn high_lod_helm_honours_toml_authored_waypoint_arrival_radius() {
        fn patrol_app(arrival_radius: Option<f32>) -> App {
            let mut app = test_app();
            // wp0 sits 100 units to starboard — inside a 150 radius, outside
            // the default 20.
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

        assert!(
            get_steering_input(&mut patrol_app(None)) > 0.0,
            "with the default arrival radius the helm is still 100 units short of \
             wp0, so it must turn toward it (wp0 is to starboard)"
        );
        assert_eq!(
            get_steering_input(&mut patrol_app(Some(150.0))),
            0.0,
            "a TOML-widened arrival radius must put the high-LOD helm *inside* \
             wp0, so it flies straight through — the same radius, and the same \
             call, the cursor evaluator makes. A hardcoded WAYPOINT_ARRIVAL_RADIUS \
             would still be turning."
        );
    }

    // ── TOML-authored avoidance tuning (AGENTS.md rule 11) ────────────────
    //
    // `[behaviour] avoidance_buffer` / `avoidance_look_ahead_secs` are
    // declared with serde defaults, so a designer can author them per entity
    // template. Two sites feed them to the pure AI: `helm_ai_decision`
    // (steering/thrust) and the per-axis `ai_helm_lateral_thrust` (lateral
    // dodge). Each test below pins one of the tuning
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

    /// `ai_helm_lateral_thrust` under the "Simplified" partial-automation
    /// rating: lateral thrust AI-operated, the helm proper still human. Since
    /// #703 the coarse helm's state no longer gates the system, but these tests
    /// keep it human so the monolith cannot be the writer of the dodge they
    /// measure.
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
        // Two ticks: belt-and-braces against the shared AI-helm sim tick
        // (#803) — the first update runs on the ready latch's initial `true`,
        // and the second's 200 ms delta fires the timer outright.
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
        // See the buffer test: two ticks, because the timer skips the first.
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

    /// Drives the full-AI helm's dodge — every helm axis on AI, the shape an
    /// unmanned Helm station or an NPC hull comes up in.
    ///
    /// Until #704 the subject here was `operate_helm_ai`, which called
    /// `operate_lateral_thrust` itself; the dodge now comes from
    /// `ai_helm_lateral_thrust` like every other lateral write. The fixture
    /// still earns its place next to `lateral_thrust_ai_app`: that one pins the
    /// same tunables under the *Simplified* rating (coarse helm human, lateral
    /// automated — what the cruiser and destroyer ship), this one under a
    /// fully-AI helm. Same system, the two gate shapes real content deploys.
    ///
    /// Forward speed is not optional scaffolding — `operate_lateral_thrust`
    /// projects the ship by `forward_speed * avoidance_look_ahead_secs`, so a
    /// stationary ship collapses that projection onto its own position and makes
    /// the look-ahead term unobservable no matter what value is passed.
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

    /// The same two tunables reach the pure AI a second way: the full-AI helm's
    /// dodge. The dodge and the steering must agree about clearance, so this
    /// site must read the same TOML the steering does.
    ///
    /// Ported in #704: the subject was `operate_helm_ai`'s own
    /// `operate_lateral_thrust` call and is now `ai_helm_lateral_thrust`, the
    /// only remaining caller. Faithful because the property under test is
    /// unchanged — a TOML-authored `avoidance_buffer` must reach
    /// `operate_lateral_thrust` on a fully-AI helm rather than the
    /// `crate::ai::AVOIDANCE_BUFFER` constant — and it is asserted on the same
    /// hull, obstacle and geometry as before. What the delete changed is only
    /// *which* system performs the write, and hence the tick count: the
    /// monolith's call was unthrottled, whereas `ai_helm_lateral_thrust` is
    /// gated by the deliberate shared AI-helm sim tick (~30 Hz by default,
    /// issue #803). Hence `tick_twice`, matching `lateral_thrust_ai_honours_*`.
    #[test]
    fn full_ai_helm_honours_toml_authored_avoidance_buffer() {
        // Projected 30 units ahead (10 u/s × the default 3 s), the obstacle is
        // ~10.8 units away: outside the default 6-unit dodge radius
        // (0 + 1 + 5), inside an authored 61 (0 + 1 + 60).
        let obstacle = [4.0, 0.0, -40.0];

        let mut default_app = helm_ai_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick_twice(&mut default_app);
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
        tick_twice(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "the full-AI helm must pass the TOML-authored avoidance_buffer to \
             operate_lateral_thrust, not crate::ai::AVOIDANCE_BUFFER"
        );
    }

    /// The sixth wired argument: the full-AI helm must pass the TOML-authored
    /// `avoidance_look_ahead_secs` to `operate_lateral_thrust`, which uses it to
    /// project the ship forward before testing for a threat. Mirrors
    /// `lateral_thrust_ai_honours_toml_authored_avoidance_look_ahead`, but with
    /// every helm axis on AI rather than the Simplified rating's lateral-only.
    ///
    /// Ported in #704 exactly as its `avoidance_buffer` sibling above was — same
    /// property, same geometry, new writer, hence `tick_twice` for the shared
    /// AI-helm sim tick. See that test's note.
    #[test]
    fn full_ai_helm_honours_toml_authored_avoidance_look_ahead() {
        // Forward at yaw 0 is -Z. At 10 u/s the default 3 s horizon projects
        // only 30 units ahead, leaving the obstacle ~70 units off; an authored
        // 10 s projects 100 units, landing 2 units from it — inside the default
        // 6-unit dodge radius (0 + 1 + 5), so the buffer is held constant and
        // the look-ahead is the only variable.
        let obstacle = [2.0, 0.0, -100.0];

        let mut default_app = helm_ai_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick_twice(&mut default_app);
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
        tick_twice(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "the full-AI helm must pass the TOML-authored avoidance_look_ahead_secs to \
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
            // falls through to the Navigation waypoint handoff, the only path
            // that reads `nav_handoff_speed`.
            set_ship_blackboard_objectives(
                &mut app,
                vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
            );
            if let Some(behaviour) = behaviour {
                set_behaviour_section(&mut app, behaviour);
            }
            // Post-#702 the handoff is the ship's own `NavigationWaypoint`,
            // gated by a matching `HelmWaypointClearance`, rather than a
            // private `AiMemory.nav_goal` copy. Dead ahead and far away, so the
            // helm throttles up at exactly `nav_handoff_speed`.
            set_cleared_nav_waypoint(&mut app, 0.0, -900.0);
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

    // (a) Pirate raider — verifies that an NPC ship with both stick axes on
    // Ai control satisfies `helm_axes_operate_ai`, the gate every per-ship
    // "is the AI flying this" consumer reads since #801.
    #[test]
    fn pirate_raider_ai_helm_policy_routes_through_npc_path() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut resolver = ControlSourceResolver::new();
        for system_id in [
            crate::system_registry::helm_thrust_system_id(),
            crate::system_registry::helm_steering_system_id(),
        ] {
            resolver.set(system_id, ControlSource::Ai);
        }
        let sources = ShipSystemControlSources(resolver);
        assert!(
            helm_axes_operate_ai(&sources),
            "NPC raider helm axes must route through the AI helm path"
        );
        assert!(
            !sources
                .0
                .policy_for(&crate::system_registry::helm_thrust_system_id())
                .accept_human_input,
            "NPC raider must not accept human helm input"
        );
    }

    // (b) All-Backfill player ship — verifies that when the player ship has
    // both stick axes on Ai control but no AiControllerComponent (no
    // behaviour tree), `helm_axes_operate_ai` still returns true. A single
    // AI axis is NOT enough — the predicate answers "is the AI flying this
    // ship", which needs both.
    #[test]
    fn all_backfill_helm_policy_gates_operate_ai() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut resolver = ControlSourceResolver::new();
        resolver.set(
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );
        let sources = ShipSystemControlSources(resolver);
        assert!(
            !helm_axes_operate_ai(&sources),
            "one AI axis alone must not satisfy the whole-helm AI predicate"
        );

        let mut resolver = ControlSourceResolver::new();
        resolver.set(
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );
        resolver.set(
            crate::system_registry::helm_steering_system_id(),
            ControlSource::Ai,
        );
        let sources = ShipSystemControlSources(resolver);
        assert!(
            helm_axes_operate_ai(&sources),
            "Backfill player helm (both axes AI) must satisfy the AI-helm gate"
        );
    }

    // (c) Player ship Backfill runs full operate_helm (avoidance + doctrine).
    // Verifies that the player ship on Backfill goes through the same
    // `operate_helm` decision (via the per-axis AI systems) as NPC ships — not
    // a Reach-only stub — satisfying issue #587 AC.
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
        // Tactical's lock: what the helm pursues (issue #702).
        set_ship_weapons_target(&mut app, &target_uuid);

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
    /// Deliberately excludes `process_helm_inputs` and the per-axis AI helm
    /// systems so a test
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
            .set(crate::system_registry::helm_thrust_system_id(), source);
        sources
            .0
            .set(crate::system_registry::helm_steering_system_id(), source);
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
        let ai_of = |app: &App, e: Entity| {
            helm_axes_operate_ai(
                app.world()
                    .entity(e)
                    .get::<ShipSystemControlSources>()
                    .unwrap(),
            )
        };
        assert!(
            ai_of(&app, ai) && !ai_of(&app, human),
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
            // The console-owned surfaces the AI helm derives its goals from
            // (issue #702) — see `HelmAiSurfaces`.
            crate::weapons_plugin::WeaponsTarget::default(),
            crate::navigation_plugin::NavigationWaypoint::default(),
            HelmWaypointClearance::default(),
            crate::ai_plugin::ObjectiveCursors::default(),
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
            // The console-owned surfaces the AI helm derives its goals from
            // (issue #702) — see `HelmAiSurfaces`.
            crate::weapons_plugin::WeaponsTarget::default(),
            crate::navigation_plugin::NavigationWaypoint::default(),
            HelmWaypointClearance::default(),
            crate::ai_plugin::ObjectiveCursors::default(),
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
