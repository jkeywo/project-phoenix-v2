/// Bevy plugin: NPC AI lifecycle — registers synthetic `ai:<uuid>` tokens,
/// drives per-entity helm/weapons/doctrine AI, and manages NPC hull tracking.
///
/// Compiled only for the `server` feature (same gate as `simulation.rs`).
use crate::simmath;
use bevy::prelude::*;
use std::collections::HashMap;

/// Build the AI anchor lookup table from the unified `WorldConfig` (PRD #337).
///
/// `WorldConfig.anchors()` already returns `HashMap<String, [f32; 3]>` with
/// 2-element anchors widened to 3 components at parse time, so the AI tick
/// just needs a clone.
///
/// This is the sole anchor source after PRD #341.
pub fn anchors_from_world_config(
    world: &crate::world::config::WorldConfig,
) -> HashMap<String, [f32; 3]> {
    world.anchors().clone()
}

use crate::ai::lod::{evaluate_lod, LodState};
use crate::entities::spawner::{BehaviourSection, EntityUuid};
use crate::server_app::{LocalShip, Ship};
use crate::ship::state::ShipPhysics;

// The slower snapshot cadence that gates `build_world_snapshot` and
// `aggregate_doctrine_blackboards` used to be a private, HARDCODED 10 Hz
// `AiSnapshotTimer` living right here — a second AI clock, free to drift out of
// phase with the helm one and unreachable from any world TOML. Issue #889
// retired it: the rate is now authored as `[global] ai_snapshot_hz` and the
// latch is DERIVED from the single `[global] ai_tick_hz` base tick as an
// integer multiple. Both systems keep the identical gate under its new home.
pub use crate::ai::cadence::{ai_snapshot_ready, AiSnapshotReady};

// ── AiTokenRegistry ───────────────────────────────────────────────────────────

/// Maps entity UUID → synthetic token string (`"ai:<uuid>"`).
/// The simulation's token-to-entity lookup falls back to this registry when
/// a player session lookup misses.
#[derive(Resource, Default)]
pub struct AiTokenRegistry {
    /// entity_uuid → token string
    by_entity: HashMap<String, String>,
    /// token string → entity_uuid (reverse lookup)
    by_token: HashMap<String, String>,
    /// Bevy Entity id → entity_uuid (for despawn handler)
    by_bevy_entity: HashMap<Entity, String>,
    /// entity_uuid → Bevy Entity (for admission-gate entity lookup)
    uuid_to_bevy: HashMap<String, Entity>,
}

impl AiTokenRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a synthetic token for an entity UUID, also storing the Bevy
    /// entity for despawn-time lookup. Idempotent.
    pub fn register(&mut self, entity_uuid: &str) -> &str {
        let token = format!("ai:{}", entity_uuid);
        self.by_entity
            .entry(entity_uuid.to_string())
            .or_insert_with(|| {
                self.by_token.insert(token.clone(), entity_uuid.to_string());
                token.clone()
            });
        self.by_entity
            .get(entity_uuid)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Register a synthetic token AND record the Bevy `Entity` for
    /// despawn-time unregistration. Idempotent.
    pub fn register_with_entity(&mut self, entity_uuid: &str, entity: Entity) {
        self.register(entity_uuid);
        self.by_bevy_entity.insert(entity, entity_uuid.to_string());
        self.uuid_to_bevy.insert(entity_uuid.to_string(), entity);
    }

    /// Unregister by entity UUID; silently does nothing if not present.
    pub fn unregister(&mut self, entity_uuid: &str) {
        if let Some(token) = self.by_entity.remove(entity_uuid) {
            self.by_token.remove(&token);
        }
        self.uuid_to_bevy.remove(entity_uuid);
    }

    /// Unregister by Bevy `Entity`; used by the despawn handler when the
    /// UUID component is no longer accessible.
    pub fn unregister_by_bevy_entity(&mut self, entity: Entity) {
        if let Some(uuid) = self.by_bevy_entity.remove(&entity) {
            self.unregister(&uuid);
        }
    }

    /// Look up the Bevy `Entity` for an AI token string. Used by the
    /// admission gate to verify the token belongs to the player ship.
    pub fn bevy_entity_for_token(&self, token: &str) -> Option<Entity> {
        let uuid = self.by_token.get(token)?;
        self.uuid_to_bevy.get(uuid).copied()
    }

    /// Look up an entity UUID by its synthetic token. Returns `None` when
    /// the token is not an AI token (so callers can fall back gracefully).
    pub fn entity_uuid_for_token(&self, token: &str) -> Option<&str> {
        self.by_token.get(token).map(|s| s.as_str())
    }

    /// Look up the synthetic token for an entity UUID.
    pub fn token_for_entity(&self, entity_uuid: &str) -> Option<&str> {
        self.by_entity.get(entity_uuid).map(|s| s.as_str())
    }

    /// Returns `true` when the registry contains a record for this entity UUID.
    pub fn contains_entity(&self, entity_uuid: &str) -> bool {
        self.by_entity.contains_key(entity_uuid)
    }
}

// ── AI entity markers ─────────────────────────────────────────────────────────

// `ShipAiMemory(AiMemory)` lived here until issue #702 deleted it. There is no
// private per-entity AI memory any more: every goal the AI serves is read from
// a surface some console owns and a human could equally drive —
// `TacticalRadarSelection` (Tactical), `NavigationWaypoint` (Navigation),
// `ObjectiveCursors` (the objective), `LastShipAttacker` (the world). Adding a
// private mirror of any of them back re-creates the split brain — and the
// helm/weapons targeting divergence — that removing it fixed.
//
// (Issue #882's `world::flags::AiPolicyMemory` is a DIFFERENT thing and is
// deliberately named differently: it is owned by one fine system's policy
// runtime, is only readable through that system's own `memory(...)` atoms, and
// is not a ship-wide reasoning blob.)

/// Marker component: entity is eligible for high-fidelity AI simulation.
/// Entities without this marker run at reduced simulation fidelity.
///
/// # Why it REQUIRES `HelmPhysicsWriteGuard` in debug builds (issue #1051)
///
/// Same argument as `server_app::LocalShip`'s `#[require]` of
/// `HumanSeekingHosts` (issue #984's S7 fix, 66c3c1bd), and found the same way.
/// `integrate_ship_physics` is the only writer of the debug-only write-tracker,
/// it runs on exactly this marker, and it used to `Commands::insert` the guard
/// the first time it saw a ship. That is an ARCHETYPE MOVE on a mid-run tick:
/// Bevy allocates archetype ids in creation order and every query iterates its
/// matched archetypes in that order, so the extra archetype re-orders the ones
/// the ship hulls land in, the per-victim RNG draws in the damage sites
/// interleave differently, and the authoritative digest moves.
///
/// Because the guard is `#[cfg(debug_assertions)]`, that mid-run move happened
/// in dev builds and *not* in release builds — which is exactly the
/// cross-environment digest instability issue #1051 was opened for. Measured on
/// c2c38984: a dev build differing from the standard one in nothing but
/// `debug-assertions = false` reproduced the release-profile `duel` and
/// `rng_coverage` digests byte for byte, and `duel` diverged at the gameplay
/// level with it (different knockouts, different shots fired). Requiring the
/// guard makes it arrive in the SAME transition as the marker on both promotion
/// routes, so debug and release builds create the same archetypes in the same
/// order and the integrator needs no `Commands` at all.
#[derive(Component)]
#[cfg_attr(debug_assertions, require(crate::ship::helm::HelmPhysicsWriteGuard))]
pub struct AiHighFidelity;

/// The per-ship AI components that MUST travel with [`AiHighFidelity`] — the
/// single source of truth for both spawn paths.
///
/// ## Why this exists
///
/// A ship can acquire the marker by two entirely separate routes: NPCs through
/// [`lod_ai_ships`] promotion, and the local player ship through
/// `server_app::spawn_game_start_entities` (which does NOT go through
/// `entities::spawner`). Three times now a new per-ship AI component has been
/// added to one route and silently missed on the other — #785's
/// `RepairTargetSelector`, #786's `CommsTargetSelector`, and #882's
/// `HelmBoostAiPolicyState` — and each time the failure was SILENT: the system
/// that queries the component non-optionally just skips the ship, or the host
/// falls back to a stateless/default arm, with no warning and no error.
///
/// Naming the set ONCE, as a type alias plus a constructor, removes the class
/// of bug rather than patching the instance: every site inserts
/// [`ai_high_fidelity_components`] and the demote path removes
/// `AiHighFidelityComponents`, so adding a component here reaches the player
/// ship, every promoted NPC, and the test twin in `ship::test_support`
/// together, and insert/remove can no longer drift apart.
///
/// The marker itself is a member so that "high fidelity" is one indivisible
/// unit: there is no way to insert the marker without the components it implies.
pub type AiHighFidelityComponents = (
    AiHighFidelity,
    crate::console_ai::server::ShipFrequencyHintState,
    crate::ship::helm::ThrustInput,
    crate::ship::helm::SteeringInput,
    crate::ship::helm::LateralThrustInput,
    crate::ship::helm::VerticalThrustInput,
    crate::ship::helm::ImpulseCommand,
    crate::ship::helm::BoostCommand,
    crate::ship::helm_ai::HelmBoostAiPolicyState,
    // Issue #883: the two travel axes gained their own policy runtime state, and
    // the derived fly-through pass surface the motion planner reads. All three go
    // through this set for the reason above — the destroyer doctrine would
    // silently degrade to its stateless shadow on any spawn path that missed one.
    crate::ship::helm_ai::HelmEnginesAiPolicyState,
    crate::ship::helm_ai::HelmSteeringAiPolicyState,
    crate::ship::helm_ai::HelmPassSurface,
    // Issue #788: the bounded range-history window behind the "has this ship
    // HELD its safe distance" half of the recovery re-entry gate. Same reason
    // again — a spawn path that missed it would leave the destroyer unable to
    // ever re-enter, silently.
    crate::ship::helm_ai::HelmRecoveryHistory,
);

/// Build the [`AiHighFidelityComponents`] set at its defaults.
///
/// Every field is `default()`, which is also the AC5 reset for the #882 policy
/// runtime state: a ship that (re-)enters high fidelity starts from the
/// policy's authored initial state rather than resuming a stale one.
pub fn ai_high_fidelity_components() -> AiHighFidelityComponents {
    (
        AiHighFidelity,
        crate::console_ai::server::ShipFrequencyHintState::default(),
        crate::ship::helm::ThrustInput::default(),
        crate::ship::helm::SteeringInput::default(),
        crate::ship::helm::LateralThrustInput::default(),
        crate::ship::helm::VerticalThrustInput::default(),
        crate::ship::helm::ImpulseCommand::default(),
        crate::ship::helm::BoostCommand::default(),
        crate::ship::helm_ai::HelmBoostAiPolicyState::default(),
        crate::ship::helm_ai::HelmEnginesAiPolicyState::default(),
        crate::ship::helm_ai::HelmSteeringAiPolicyState::default(),
        crate::ship::helm_ai::HelmPassSurface::default(),
        crate::ship::helm_ai::HelmRecoveryHistory::default(),
    )
}

/// Whether an entity carries every member of [`AiHighFidelityComponents`].
///
/// Kept beside the canonical tuple and constructor so lifecycle consumers do
/// not retype bundle membership independently. Snapshot restore uses this to
/// distinguish a complete High bootstrap (safe to overwrite in place) from a
/// partial one that must be reset through [`ai_high_fidelity_components`].
pub fn has_ai_high_fidelity_components(entity: &EntityWorldMut<'_>) -> bool {
    entity.contains::<AiHighFidelity>()
        && entity.contains::<crate::console_ai::server::ShipFrequencyHintState>()
        && entity.contains::<crate::ship::helm::ThrustInput>()
        && entity.contains::<crate::ship::helm::SteeringInput>()
        && entity.contains::<crate::ship::helm::LateralThrustInput>()
        && entity.contains::<crate::ship::helm::VerticalThrustInput>()
        && entity.contains::<crate::ship::helm::ImpulseCommand>()
        && entity.contains::<crate::ship::helm::BoostCommand>()
        && entity.contains::<crate::ship::helm_ai::HelmBoostAiPolicyState>()
        && entity.contains::<crate::ship::helm_ai::HelmEnginesAiPolicyState>()
        && entity.contains::<crate::ship::helm_ai::HelmSteeringAiPolicyState>()
        && entity.contains::<crate::ship::helm_ai::HelmPassSurface>()
        && entity.contains::<crate::ship::helm_ai::HelmRecoveryHistory>()
}

/// AI personality and capability profile for NPC entities.
#[derive(Component, Clone, Debug)]
pub struct AiProfile {
    pub aggression: f32,
    pub sensor_range: f32,
    /// See [`crate::entities::config::AiProfileConfig::low_lod_cruise_fraction`].
    pub low_lod_cruise_fraction: f32,
    /// See [`crate::entities::config::AiProfileConfig::low_lod_speed_decay_per_sec`].
    pub low_lod_speed_decay_per_sec: f32,
    /// See [`crate::entities::config::AiProfileConfig::low_lod_turn_rate_fraction`].
    pub low_lod_turn_rate_fraction: f32,
}

impl Default for AiProfile {
    fn default() -> Self {
        Self {
            aggression: 0.5,
            sensor_range: 100.0,
            low_lod_cruise_fraction: crate::entities::config::default_low_lod_cruise_fraction(),
            low_lod_speed_decay_per_sec:
                crate::entities::config::default_low_lod_speed_decay_per_sec(),
            low_lod_turn_rate_fraction: crate::entities::config::default_low_lod_turn_rate_fraction(
            ),
        }
    }
}

/// Tracks time since last LOD state transition for dwell-based demotion.
#[derive(Component, Clone, Debug)]
pub struct LodTransitionTimer {
    pub last_state_change_secs: f64,
}

/// A high-fidelity **bubble**: this entity projects a zone of `radius` world
/// units inside which every NPC is kept promoted to `AiHighFidelity`, and the
/// carrier itself is always high-fidelity.
///
/// LOD used to be a single implicit bubble around the player's `LocalShip`
/// ([`lod_ai_ships`]) sized by each NPC's own `sensor_range`: a ship ran the
/// full weapons / target-selection AI only while it was near the player, so any
/// combat the player was not standing next to happened in the cheap low-LOD path
/// where movement is dead-reckoned. That is wrong for a defended object —
/// Starbase Alpha in `combat_test` sat in low-LOD being ground down while the
/// player hunted elsewhere, its own point defence never running and the raiders
/// sieging it dead-reckoned rather than fighting. A bubble makes "is this near
/// enough to the action to simulate in full" a property of *anchors*: a player
/// ship always projects one (the `LocalShip` is an implicit anchor at
/// [`DEFAULT_PLAYER_LOD_BUBBLE_RADIUS`] unless it authors its own), and a
/// stationary defended object like the station projects a smaller one, so the
/// raid sieging it — and the station's own guns — run in full whether or not the
/// player is looking. Authored as `[lod_bubble] radius = N`.
#[derive(Component, Clone, Copy, Debug)]
pub struct LodBubble {
    pub radius: f32,
}

/// The bubble radius a player `LocalShip` projects when it authors no
/// `[lod_bubble]` of its own — every player hull is an anchor without having to
/// repeat the block. Generous enough to cover a normal engagement so an NPC
/// closing on the player is in full fidelity before it opens fire; deliberately
/// WIDER than the old per-NPC `sensor_range` promotion, which is what re-timed
/// far-from-player combats (`probe_despawn`'s duel gains its natural second kill
/// once both hulls run in full).
pub const DEFAULT_PLAYER_LOD_BUBBLE_RADIUS: f32 = 600.0;

/// Per-objective route cursors: where this ship is on each objective's route.
///
/// Each entry is a [`PatrolCursor`] tracking the current waypoint for one
/// objective. Entries are independent — advancing one does not affect others.
/// Cursor state is interpreted (and its out-of-range terminal stop owned) by
/// the pure `ai::patrol_cursor` module.
///
/// # Sole writer
///
/// `advance_objective_cursors` (`SimSet::Modifiers`) is the only writer: it
/// owns arrival detection and cursor advancement for every ship, every
/// objective, at every LOD. Everyone else reads —
/// `simulate_low_lod_ships` (`SimSet::Physics`) to cheaply steer NPCs outside
/// sensor range, `helm_patrol` to steer high-LOD ships, `operate_navigation_ai`
/// to place the waypoint. One writer in one set is what stops a cursor from
/// being advanced twice in a tick.
///
/// # Why a side-table (issue #702)
///
/// Keyed by `objective_id` per ship, rather than living on the objective. It
/// cannot live there: mission objectives are a single shared world-level
/// record (every ship pursuing one would share a cursor), and doctrine
/// objectives are rebuilt from TOML every tick (a cursor on one would be reset
/// every tick).
///
/// Named `PatrolCursors` until #702 generalised it: the name always lied,
/// since it handled `Reach` too. It is now *the* cursor surface for every
/// directive. Before #702 the high-LOD helm path kept a rival cursor of its own
/// in `AiMemory.waypoint_index`, so patrol position was tracked in two places
/// that could disagree; there is now one.
///
/// Present on every ship (player + NPC). The player ship was missing it —
/// `entities/spawner.rs` inserted it, `server_app.rs` did not — which silently
/// disabled AI patrol on the player ship under `Backfill`.
#[derive(Component, Clone, Debug, Default)]
pub struct ObjectiveCursors(pub Vec<crate::ai::patrol_cursor::PatrolCursor>);

/// Marker component set on NPC entities currently in a warp-out sequence.
/// Carries the data needed to draw the warp-exit visual and to populate
/// `EntitySnapshot::warp_out_remaining_secs` in the broadcast.
/// Kept for interface compatibility; not set by the doctrine-based AI system.
#[derive(Component)]
pub struct WarpOutMarker {
    pub remaining_secs: f32,
    pub target_speed: f32,
}

// ── Events ────────────────────────────────────────────────────────────────────

/// Emitted by the AI plugin when a ship's [`LastShipAttacker`] changes to name
/// a new attacker.
///
/// The world plugin observes this event to evaluate `on_entity_attacked`
/// trigger conditions without a direct dependency on the AI module.
///
/// [`LastShipAttacker`]: crate::console::weapons::LastShipAttacker
#[derive(Message, Clone, Debug)]
pub struct AiEntityAttacked {
    pub entity_uuid: String,
    pub attacker_uuid: uuid::Uuid,
}

/// Emitted by the AI plugin when an NPC entity's hull reaches ≤ 0.0.
///
/// The world plugin observes this event to evaluate `on_entity_destroyed`
/// trigger conditions without a direct dependency on the AI module.
#[derive(Message, Clone, Debug)]
pub struct AiEntityDestroyed {
    pub entity_uuid: String,
}

/// Emitted by `advance_objective_cursors` when a ship reaches the waypoint its
/// cursor is currently pointing at, immediately before the cursor advances.
///
/// The world plugin reads this in `tick_trigger_pipeline` and turns it into a
/// `WorldEvent::WaypointReached`, which drives `on_waypoint_reached` scenario
/// triggers — the same event-bridge shape `AiEntityAttacked` /
/// `AiEntityDestroyed` already use, so the AI module stays free of any
/// dependency on world content.
#[derive(Message, Clone, Debug)]
pub struct AiWaypointReached {
    /// UUID of the ship that arrived.
    pub entity_uuid: String,
    /// Id of the objective whose cursor advanced.
    pub objective_id: String,
    /// Anchor name of the waypoint that was reached.
    pub waypoint: String,
}

// ── WorldSnapshot resource ────────────────────────────────────────────────────

/// Snapshot of all ship/world-entity positions built once per tick.
/// All `operate_*_ai` handlers read from this resource rather than building
/// their own queries.
#[derive(Resource, Default)]
pub struct WorldSnapshot {
    pub entities: Vec<crate::ai::AiWorldEntity>,
}

/// Build the [`WorldSnapshot`] from every entity with an `EntityUuid`, plus
/// every field asteroid. Runs in `SimSet::Physics` before `AiTickLabel` so
/// per-system AI handlers see a consistent frame.
///
/// Asteroids need the separate pass because they are streamed by
/// `src/asteroids/lifecycle.rs` rather than `spawn_entity`, so they carry an
/// `AsteroidUuid` instead of an `EntityUuid` and never matched the query below.
/// That is why AI collision avoidance flew straight through asteroid fields:
/// not a tuning problem, the rocks simply were not in the world it steers from.
/// Keying off `AsteroidUuid` rather than back-filling `EntityUuid` keeps them
/// out of the radar, networking, and targeting paths that key on the latter.
pub(crate) fn build_world_snapshot(
    mut snapshot: ResMut<WorldSnapshot>,
    query: Query<(
        &crate::entities::spawner::EntityUuid,
        &Transform,
        Option<&crate::entities::spawner::EntityName>,
        Option<&crate::entities::spawner::FactionComponent>,
        Option<&crate::entities::spawner::EntitySystemHull>,
        Option<&crate::entities::spawner::ColliderSection>,
        Option<&crate::ship::state::ShipPhysics>,
        // Direct-fire reach (issue #788): the longest range this entity can put
        // unguided fire at, published as a threat fact so another ship's helm
        // can derive a safe standoff ring from it. Needs the control sources (to
        // know which banks are offline) and nothing else — reach is the AUTHORED
        // range of each bank, and issue #955 took `ModifierSlot::RadarRange` out
        // of it, so this query no longer reads `ShipModifiers` at all.
        Option<&crate::ship_plugin::ShipSystemControlSources>,
        Option<&crate::console::weapons::PhaserCombatConfigResource>,
        Option<&crate::console::weapons::BlasterSystemResource>,
    )>,
    asteroids: Query<
        (
            &crate::server_app::AsteroidUuid,
            &Transform,
            &crate::entities::spawner::ColliderSection,
        ),
        With<crate::server_app::Asteroid>,
    >,
) {
    snapshot.entities = query
        .iter()
        .map(
            |(
                uuid,
                transform,
                name,
                faction,
                hull,
                collider,
                physics,
                control_sources,
                phasers,
                blasters,
            )| {
                let hull_fraction = hull.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let radius = collider.map(|c| c.0.radius).unwrap_or(0.0);
                // Prefer ShipPhysics.forward_speed (authoritative for all ships after #587);
                // Use ShipPhysics.forward_speed (authoritative after #581).
                let forward_speed = physics.map(|p| p.forward_speed).unwrap_or(0.0);
                let direct_fire_range =
                    entity_direct_fire_range(control_sources, phasers, blasters);
                // The SIMULATION's heading, in `ShipPhysics.yaw`'s convention —
                // NOT the render transform's euler (issue #937).
                //
                // `ship::physics_systems::sync_ship_position` writes
                // `Quat::from_euler(YXZ, -physics.yaw, 0, roll)`: Bevy's Y euler
                // turns counter-clockwise and `ShipPhysics.yaw` turns clockwise,
                // so the transform deliberately carries the NEGATED heading.
                // Reading that euler straight back published every ship's heading
                // mirrored, while every consumer of this field — the two
                // `target_relative_motion` callers, both avoidance projections in
                // `ai::core`, and `entity_weapon_arc_sectors` below —
                // reconstructs a forward vector as `(sin(yaw), -cos(yaw))`, which
                // is `ShipPhysics.yaw`'s convention and nothing else's.
                //
                // The visible cost was the destroyer's attack pass. Its
                // closest-approach detector is `fact(closing_rate) <
                // param(closing_rate_epsilon)`, and `closing_rate` is built from
                // BOTH ships' reconstructed velocities; a mirrored target
                // velocity made it read "still closing" for a hull that had
                // already flown past, so `inbound` never handed off to `escape`.
                // The destroyer merged, jammed against its target and ground
                // there at contact range instead of breaking off and re-passing.
                //
                // Preferred off `ShipPhysics` for the same reason `forward_speed`
                // above is: that component is the authority for anything that
                // moves. An entity without one (a station, a planet) keeps its
                // authored transform, converted by the same negation.
                let yaw = physics
                    .map(|p| p.yaw)
                    .unwrap_or_else(|| -transform.rotation.to_euler(bevy::math::EulerRot::YXZ).0);
                // Issue #874: the one producer call. Everything downstream —
                // the helm AI exposure fact and the helm-radar overlay — reads
                // these sectors rather than deriving arcs of its own.
                let weapon_arcs =
                    entity_weapon_arc_sectors(yaw, control_sources, phasers, blasters);
                crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::parse_str(&uuid.0).unwrap_or_default(),
                    name: name.as_ref().map(|n| n.0.clone()),
                    position: [
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ],
                    faction: faction.map(|f| f.0),
                    hull_fraction,
                    yaw: Some(yaw),
                    radius,
                    forward_speed,
                    shields: None,
                    // Mobility is an AUTHORED fact off `[collider] movable`
                    // (issue #958), not "everything spawned through
                    // `spawn_entity` is a ship". Publishing `true` for the whole
                    // query said a station, a planet and a moon all manoeuvre,
                    // which put them on the wrong side of the ignore-smaller
                    // rule in `assess_hazards` — the exact hole this closes —
                    // and let them push vertical avoidance (#780) and moving
                    // urgency (#744) they have no business pushing. An entity
                    // with no `[collider]` at all rates 0 radius and falls back
                    // to the same static default the parser uses.
                    movable: collider.map(|c| c.0.movable).unwrap_or(false),
                    // Every physical body in the snapshot is a collision danger;
                    // size rating tracks the collision radius (issue #743).
                    dangerous: true,
                    size_rating: radius,
                    direct_fire_range,
                    weapon_arcs,
                }
            },
        )
        .collect();

    // Asteroids are obstacles and nothing else: no faction to be hostile to, no
    // hull fraction to retreat over, and they do not move. Only position and
    // radius matter, which is exactly what `avoidance_steering` reads.
    snapshot
        .entities
        .extend(
            asteroids
                .iter()
                .map(|(uuid, transform, collider)| crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::parse_str(&uuid.0).unwrap_or_default(),
                    name: None,
                    position: [
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ],
                    faction: None,
                    hull_fraction: None,
                    yaw: Some(0.0),
                    radius: collider.0.radius,
                    forward_speed: 0.0,
                    shields: None,
                    // Same authored fact as the query arm above (issue #958):
                    // a rock's TOML omits `movable`, so the parse default makes
                    // it terrain — never size-ignored, never a vertical or
                    // moving-urgency contributor.
                    movable: collider.0.movable,
                    // Still a dangerous collision hazard; size rating tracks the
                    // collision radius (issue #743).
                    dangerous: true,
                    size_rating: collider.0.radius,
                    // An asteroid shoots at nobody.
                    direct_fire_range: 0.0,
                    weapon_arcs: Vec::new(),
                }),
        );

    // Bevy query iteration follows archetype-creation order, which can change
    // after an unrelated component insert. Hazard assessment accumulates
    // floating-point contributions in this order, so publish one canonical
    // world order for every consumer rather than leaking ECS layout into the
    // authoritative simulation.
    snapshot.entities.sort_by_key(|entity| entity.uuid);
}

/// This entity's longest usable direct-fire reach (issue #788), or `0.0` when it
/// carries no direct-fire armament.
///
/// The Bevy adapter for the pure
/// [`longest_usable_direct_fire_range`](crate::console::weapons::longest_usable_direct_fire_range):
/// it reads the per-bank configuration off the entity, applies the same offline
/// gate the arc-bearing evaluation applies, and hands a flat list to the pure
/// function. Torpedo tubes are deliberately absent — a homing round has no
/// standoff radius.
///
/// Reach is the AUTHORED per-bank range and nothing else (issue #955). It used
/// to be scaled by `ModifierSlot::RadarRange`, which made a standoff ring shrink
/// and grow with the target's sensor power; that coupling is gone, so the ring a
/// helm derives from this fact is the ring the target's guns actually hold.
fn entity_direct_fire_range(
    control_sources: Option<&crate::ship_plugin::ShipSystemControlSources>,
    phasers: Option<&crate::console::weapons::PhaserCombatConfigResource>,
    blasters: Option<&crate::console::weapons::BlasterSystemResource>,
) -> f32 {
    use crate::console::weapons::{longest_usable_direct_fire_range, DirectFireEmitter};

    let emitters: Vec<DirectFireEmitter> =
        entity_direct_fire_banks(control_sources, phasers, blasters)
            .into_iter()
            .map(|(online, bank)| DirectFireEmitter {
                online,
                usable: true,
                range: bank.range,
            })
            .collect();
    longest_usable_direct_fire_range(&emitters)
}

/// This entity's ONLINE direct-fire arcs as world-bearing sectors (issue #874).
///
/// **The single producer call.** Its output is published on
/// [`crate::ai::AiWorldEntity::weapon_arcs`], and BOTH consumers read it from
/// there: the helm AI's exposure fact reduction
/// ([`crate::weapons::arc_geometry::arc_exposure`]) and the local ship's
/// helm-radar overlay payload (`publish_helm_blackboard`). Neither recomputes
/// the geometry, so what a human helm sees and what a backfilled helm policy
/// reasons about cannot diverge.
///
/// Torpedo tubes are deliberately absent, for the same reason
/// [`entity_direct_fire_range`] excludes them: a homing round's threat has no
/// bounded radius, so a wedge drawn at "the tube's range" would be a lie about
/// where it is safe to stand.
fn entity_weapon_arc_sectors(
    ship_yaw: f32,
    control_sources: Option<&crate::ship_plugin::ShipSystemControlSources>,
    phasers: Option<&crate::console::weapons::PhaserCombatConfigResource>,
    blasters: Option<&crate::console::weapons::BlasterSystemResource>,
) -> Vec<crate::weapons::arc_geometry::WeaponArcSector> {
    let banks: Vec<crate::weapons::arc_geometry::WeaponArcBank> =
        entity_direct_fire_banks(control_sources, phasers, blasters)
            .into_iter()
            .filter(|(online, _)| *online)
            .map(|(_, bank)| bank)
            .collect();
    crate::weapons::arc_geometry::weapon_arc_sectors(ship_yaw, &banks)
}

/// Read this entity's per-bank direct-fire configuration off its components,
/// paired with the online flag, in one place (issue #874).
///
/// Extracted so the reach fact (#788) and the arc sectors (#874) cannot disagree
/// about which banks exist, what they reach, or which of them are offline: both
/// are projections of this one list.
fn entity_direct_fire_banks(
    control_sources: Option<&crate::ship_plugin::ShipSystemControlSources>,
    phasers: Option<&crate::console::weapons::PhaserCombatConfigResource>,
    blasters: Option<&crate::console::weapons::BlasterSystemResource>,
) -> Vec<(bool, crate::weapons::arc_geometry::WeaponArcBank)> {
    use crate::weapons::arc_geometry::WeaponArcBank;

    // No control sources (a bare test spawn) means nothing is known to be
    // offline, which is the same reading the arc-bearing path takes.
    let is_offline = |sid: Option<crate::core::messages::SystemId>| -> bool {
        match (control_sources, sid) {
            (Some(cs), Some(id)) => cs.0.is_offline(&id),
            _ => false,
        }
    };

    let mut banks: Vec<(bool, WeaponArcBank)> = Vec::new();
    if let Some(cfg) = phasers {
        for b in &cfg.0.banks {
            // The authored `beam_range`, unscaled (issue #955) — exactly what
            // the blaster arm below already does with its own authored range.
            let range = if b.beam_range > 0.0 {
                b.beam_range
            } else {
                crate::entities::config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
            };
            banks.push((
                !is_offline(crate::ship::system_registry::phaser_bank_system_id(&b.id)),
                WeaponArcBank {
                    facing_deg: b.facing_deg,
                    fire_arc_deg: b.fire_arc_deg,
                    range,
                },
            ));
        }
    }
    if let Some(res) = blasters {
        for bs in &res.0 {
            banks.push((
                !is_offline(crate::ship::system_registry::blaster_bank_system_id(
                    &bs.config.id,
                )),
                WeaponArcBank {
                    facing_deg: bs.config.facing_deg,
                    fire_arc_deg: bs.config.fire_arc_deg,
                    range: bs.config.range,
                },
            ));
        }
    }
    banks
}

/// Score each entity's doctrine and write `scored_objectives` into its
/// `ShipSystemBlackboards` viewscreen entry. Covers all ships that carry a
/// `BehaviourSection` — both NPC ships and any future player-ship variant that
/// opts into doctrine-based AI.
///
/// After PRD #597 PR 10: reads red-alert / combat-activity / last-attacker
/// from each ship's own per-entity components, so NPC ship viewscreen
/// blackboards mirror the same fields the player ship exposes.
///
/// # Coexists with the LocalShip writer (issue #842)
///
/// This writes the template doctrine pool for EVERY `BehaviourSection` ship,
/// including the game-start player (which after #842 carries `BehaviourSection`
/// as well as `LocalShip`). For that one ship the entry written here is then
/// *merged*, not overwritten, by `publish_viewscreen_blackboard`, which runs
/// `.after` this system and unions the scenario `ObjectiveManager` pool over the
/// top. Do NOT "fix" the double-write by having this system skip `LocalShip` or
/// by dropping that ordering: the merge is deliberate, and clobbering either way
/// drops one of the two objective pools (the regression #842's guard exists to
/// catch). Pure NPCs (no `LocalShip`) are unaffected — only this writer touches
/// their entry.
pub(crate) fn aggregate_doctrine_blackboards(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    // `ai`-category decision-trace instrumentation (issue #1146). `Option<Res>`
    // for the usual bare-`App` reason (a fixture need not insert either); with
    // `ai` logging off both read as "not enabled" and the trace block is skipped
    // whole, so a default run pays nothing and its digest is unmoved.
    log: Option<Res<crate::logging::LogFilterConfig>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    mut query: Query<
        (
            // The Bevy entity + its display name, for the `ai` decision trace's
            // per-entity filter and its `ship` field (issue #1146). Read-only.
            Entity,
            Option<&crate::entities::spawner::EntityName>,
            // Optional so a static point-defence platform (the station), which
            // authors no `[behaviour]`, still gets a Viewscreen blackboard. Its
            // phaser AI (`ai_phaser_auto_fire`) aims at the `combat_lock` this
            // publishes, so without an entry here the station's Tactical lock was
            // consumed by nothing and it never fired. The `Or<>` filter keeps
            // scenery (stars, asteroids) out — only doctrine ships and turrets
            // qualify.
            Option<&BehaviourSection>,
            &crate::entities::spawner::EntitySystemHull,
            &mut crate::server_app::ShipSystemBlackboards,
            Option<&crate::ship::state::ShipRedAlert>,
            Option<&crate::ship::combat_activity::RecentCombatActivity>,
            Option<&crate::console::weapons::LastShipAttacker>,
        ),
        Or<(
            With<BehaviourSection>,
            With<crate::entities::spawner::StaticPointDefence>,
        )>,
    >,
) {
    // Sim seconds off the fixed clock — Bevy context-switches `Res<Time>` to
    // `Time<Fixed>` inside `FixedUpdate`, so this is the tick's own elapsed
    // time and never a wall clock (AGENTS.md #7).
    let now = time.elapsed_secs();
    let attacked_memory_secs = world_config
        .as_deref()
        .map(|wc| wc.global.attacked_memory_secs)
        .unwrap_or_else(|| crate::entities::config::GlobalConfig::default().attacked_memory_secs);
    for (
        ship_entity,
        name_opt,
        behaviour,
        hull,
        mut blackboards,
        red_alert_opt,
        activity_opt,
        last_attacker_opt,
    ) in &mut query
    {
        let hull_fraction = {
            let max = hull.0.total_max();
            if max > 0.0 {
                (hull.0.total_current() / max).clamp(0.0, 1.0)
            } else {
                1.0
            }
        };
        let red_alert = red_alert_opt.map(|ra| ra.0).unwrap_or(false);
        // Whether something LANDED A HIT on this ship recently, on a decaying
        // window (issue #1010) — not whether it carries the component that
        // would record an attacker (issue #936), and not whether one was ever
        // recorded at all.
        //
        // #936's fix read `LastShipAttacker`'s contents rather than the
        // component's presence, which unpinned the constant `true` every ship
        // was born with. But the contents are a LATCH: set on the first beam
        // that connects, cleared only on death or on the red-alert on→off edge
        // (`server_app::clear_last_attacker_on_red_alert_off`). Every Harrow
        // DOES author a captain stand-down (`combat_window_secs = 10.0`), so
        // the latch releases about ten seconds after a fight ends — but it
        // cannot release DURING one, and "during" is broader than it looks:
        // the captain's `secs_since_combat` fact folds the hull's OWN weapon
        // fire in alongside damage taken, so a Harrow returning fire keeps
        // resetting its own stand-down clock and red alert never drops. (Hulls
        // authoring an alert-on-hostile rule — `alliance_courier.toml`'s
        // priority-5 one; a Harrow authors none — hold the alert up on mere
        // contact too.) With a player loitering in the raid's vicinity the
        // latch therefore never released, which is what the playtest saw:
        //
        //   * `combat_test.toml`'s `assault-starbase` Destroy override is
        //     zero-gated on `not_attacked`. Under the latch the first shot any
        //     Harrow took retired the raid this scenario is named for, and it
        //     stayed retired for as long as anything hung around — the ship
        //     kills what shot it and then has no assault to go back to.
        //   * `ship_harrow_patrol.toml` documents a picket that HOLDS station
        //     undisturbed (Patrol 30+15 vs Destroy 38) and commits when shot at
        //     (Destroy 38+25). It could commit, but it could not stand back
        //     down and resume the picket while the intruder stayed put.
        //
        // Recency of the last landed hit decays instead, over the TOML-authored
        // `[global] attacked_memory_secs`: a hit closes a `not_attacked` gate
        // (self-defence outranks the raid via the base `destroy_hostiles` arm),
        // and a window with none reopens it. That gives the doctrine gate its
        // OWN window, decoupled from the red-alert/`LastShipAttacker` chain —
        // the per-hull captain `combat_window_secs` still governs alert posture,
        // this governs whether the raid resumes. `LastShipAttacker` still
        // identifies WHO is shooting for the viewscreen below; it no longer
        // decides WHETHER.
        //
        // `last_landed_hit_secs` folds hull damage together with
        // shield-absorbed hostile fire, because `last_damage_taken` alone
        // misses everything an arc eats — see that function for why the shipped
        // station never gets past a Harrow's arc.
        //
        // The LocalShip half of the same publish (`publish_viewscreen_blackboard`,
        // `src/server_app.rs`) says in a comment that it uses "the same
        // `attacked` signal the NPC path uses" — the symmetry rule (AGENTS.md
        // #6). Both sites run the same fold into the same predicate so that
        // stays true by construction rather than by two copies agreeing.
        let attacked = crate::objectives::attacked_recently(
            activity_opt.and_then(|a| {
                crate::objectives::last_landed_hit_secs(
                    a.last_damage_taken,
                    a.last_hostile_fire_taken,
                )
            }),
            now,
            attacked_memory_secs,
        );
        let conditions = crate::objectives::WorldConditions {
            red_alert,
            hull_fraction,
            attacked,
        };
        // Retreat is ordinary authored doctrine (issue #702). A synthetic
        // hull-triggered Retreat used to be injected here; it was deleted
        // because it could not work and never had:
        //
        //  1. Its anchor was always empty (the anchors map is not in scope
        //     here), so it leaned on an `AiMemory.home_position` fallback that
        //     production never seeded — `register_ai_tokens_on_spawn` only
        //     seeded it when no memory existed, but the spawner had already
        //     inserted a default in the same batch. Every shipped ship therefore
        //     "retreated" to world origin.
        //  2. Its score ran 0..1 while doctrine `base_priority` runs in the
        //     tens, so it could never outrank anything even after re-sorting.
        //
        // A designer authors the same intent with vocabulary that already
        // exists, on the right score scale and with a real anchor:
        //
        //     [[behaviour.doctrine]]
        //     id               = "retreat-when-hurt"
        //     directive_kind   = "Retreat"
        //     directive_anchor = "pirate_haven"
        //     base_priority    = 100.0
        //     zero_gates       = [{ condition = "hull_below", threshold = 0.3 }]
        //
        // `hull_fraction` below is what `hull_below` gates on, so the trigger
        // is unchanged — only now it is tunable per hull without a recompile.
        // A point-defence turret has no doctrine pool; it scores nothing and
        // relies entirely on the `combat_lock` lift below.
        let scored = behaviour
            .map(|b| crate::ai::score_doctrine_pool(&b.0.doctrine, &conditions))
            .unwrap_or_default();
        // Lift Combat Lock + Science Target from this ship's own radar
        // blackboards (issue #829). They were published this tick in
        // `SimSet::Publish`, which runs before this `PublishAggregate` system.
        let combat_lock = match blackboards
            .0
            .get(&crate::ship::system_registry::tactical_radar_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::TacticalRadar(bb)) => {
                bb.selected_target.clone()
            }
            _ => None,
        };
        let science_target = match blackboards
            .0
            .get(&crate::ship::system_registry::sensor_radar_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::SensorRadar(bb)) => {
                bb.selected_target.clone()
            }
            _ => None,
        };
        // ── `ai`-category decision trace (issue #1146) ──────────────────────
        // A read-only projection of the pool just scored. Gated on the `ai`
        // category being enabled for THIS ship, so a default-level run formats
        // no label, clones nothing, and touches neither the world nor the RNG —
        // the seeded digest is byte-identical whether `ai=debug` is on or off
        // (`tests/ai_decision_log.rs`). The directive-change event's
        // previous directive is read from last tick's pool, which is still on
        // the blackboard until the `insert` below overwrites it — no cross-tick
        // tracking resource, and nothing here that a fixed-tick reader consults.
        {
            let cfg = crate::logging::AsLogFilter::log_filter(&log);
            let ai_debug = cfg.cat_enabled(
                crate::logging::LogCat::Ai,
                crate::logging::LevelFilter::Debug,
            );
            let ai_info = cfg.cat_enabled(
                crate::logging::LogCat::Ai,
                crate::logging::LevelFilter::Info,
            );
            if (ai_debug || ai_info) && cfg.entity_allowed(ship_entity) {
                let tick = sim_tick.as_deref().map(|t| t.0).unwrap_or(0);
                let ship = name_opt.map(|n| n.0.as_str()).unwrap_or("<unnamed>");
                // Per-tick scoring trace: why the top directive won this tick.
                crate::pdebug!(
                    log,
                    crate::logging::LogCat::Ai,
                    entity = ship_entity,
                    tick = tick,
                    ship = ship,
                    "doctrine {}",
                    crate::ai::decision_trace::format_pool(&scored)
                );
                // Structured directive-change event: the per-ship timeline entry.
                let change = match blackboards
                    .0
                    .get(&crate::ship::system_registry::viewscreen_system_id())
                {
                    Some(crate::core::messages::SystemBlackboard::Viewscreen(v)) => {
                        crate::ai::decision_trace::directive_change(&v.scored_objectives, &scored)
                    }
                    _ => crate::ai::decision_trace::directive_change(&[], &scored),
                };
                if let Some(change) = change {
                    crate::pinfo!(
                        log,
                        crate::logging::LogCat::Ai,
                        entity = ship_entity,
                        ai_event = "directive_change",
                        tick = tick,
                        ship = ship,
                        prev = change.prev.as_str(),
                        new = change.new.as_str(),
                        target = change.target.as_str(),
                        score = change.score,
                        "directive {} -> {}",
                        change.prev,
                        change.new
                    );
                }
            }
        }

        let viewscreen_bb = crate::core::messages::ViewscreenBlackboard {
            red_alert,
            hull_integrity_pct: hull_fraction * 100.0,
            last_damage_taken_secs: activity_opt.and_then(|a| a.last_damage_taken),
            last_weapon_fired_secs: activity_opt.and_then(|a| a.last_weapon_fired),
            last_attacker_uuid: last_attacker_opt.and_then(|la| la.0.clone()),
            scored_objectives: scored,
            combat_lock,
            science_target,
        };
        blackboards.0.insert(
            crate::core::messages::SystemId(
                crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID.to_string(),
            ),
            crate::core::messages::SystemBlackboard::Viewscreen(viewscreen_bb),
        );
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiTokenRegistry>();
        app.add_message::<AiEntityAttacked>();
        app.add_message::<AiEntityDestroyed>();
        app.add_message::<AiWaypointReached>();
        app.init_resource::<WorldSnapshot>();
        // The ONE shared AI decision cadence (issues #889, #895), which also
        // derives the slower snapshot latch these two systems gate on. Its
        // derivation system lives in `FixedLast`, so the flag is consumed by
        // every `FixedUpdate` system it gates before it is re-armed for the
        // next step — no per-system `.after()` edge needed.
        crate::ai::cadence::register_ai_cadence(app);
        app.add_systems(
            FixedUpdate,
            build_world_snapshot
                .in_set(crate::sim_sets::SimSet::Physics)
                .before(crate::sim_sets::AiTickLabel)
                .run_if(ai_snapshot_ready),
        );
        app.add_systems(
            FixedUpdate,
            aggregate_doctrine_blackboards
                .in_set(crate::sim_sets::SimSet::PublishAggregate)
                .run_if(ai_snapshot_ready),
        );
        app.add_systems(
            FixedUpdate,
            (
                simulate_low_lod_ships
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .before(lod_ai_ships),
                lod_ai_ships
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .before(build_world_snapshot)
                    .before(crate::sim_sets::AiTickLabel),
            ),
        );
        app.add_systems(
            FixedUpdate,
            (
                register_ai_tokens_on_spawn,
                emit_attacked_on_new_attacker
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .in_set(crate::sim_sets::AiTickLabel),
                unregister_on_despawn,
            ),
        );
    }
}

// -- Systems -------------------------------------------------------------------------------

/// Register a synthetic `ai:<uuid>` token the tick an entity's `BehaviourSection`
/// first appears. `Added<BehaviourSection>` fires exactly once — the spawn tick —
/// and `register_with_entity` is idempotent, so this both registers once and
/// records the Bevy `Entity` for despawn-time unregistration. `BehaviourSection`
/// *is* the "this entity is AI-driven" predicate (issue #832); it is inserted at
/// spawn and only ever removed on despawn, so nothing here needs a separate marker.
fn register_ai_tokens_on_spawn(
    mut registry: ResMut<AiTokenRegistry>,
    query: Query<(Entity, &EntityUuid), Added<BehaviourSection>>,
) {
    for (entity, uuid) in &query {
        registry.register_with_entity(&uuid.0, entity);
    }
}

/// Emit `AiEntityAttacked` whenever a ship's [`LastShipAttacker`] changes to
/// name a new attacker.
///
/// [`LastShipAttacker`] *is* the "who last attacked me" surface — the same one
/// `ai_target_selection` reads — so the rising-edge latch that fires this event
/// is that component's own change detection (issue #702). It replaces the
/// private `AiMemory.last_attacker` mirror and the `AttackerThisTick` one-tick
/// component that used to carry the edge across.
///
/// Exactly one event per new attacker under sustained fire: `tick_beams`
/// compares before writing, so a beam that keeps naming the same shooter never
/// marks the component changed and this system never re-fires. The two clear
/// paths (`clear_last_attacker_on_death` /
/// `clear_last_attacker_on_red_alert_off`) do mark it changed, but only ever
/// write `None` — which the `Some` guard below skips. Re-attack by the same
/// shooter after a clear is a genuine new edge and correctly re-fires.
///
/// Runs in `SimSet::Physics`, i.e. *before* the `SimSet::Damage` `tick_beams`
/// that writes the component, so the event lands on the tick after the hit —
/// the same one-tick bridge `AttackerThisTick` gave it.
///
/// [`LastShipAttacker`]: crate::console::weapons::LastShipAttacker
fn emit_attacked_on_new_attacker(
    query: Query<
        (&EntityUuid, &crate::console::weapons::LastShipAttacker),
        Changed<crate::console::weapons::LastShipAttacker>,
    >,
    mut attacked_events: MessageWriter<AiEntityAttacked>,
) {
    for (uuid, last_attacker) in query.iter() {
        // `None` is a clear, not an attack; an unparseable UUID names nobody.
        let Some(attacker_uuid) = last_attacker
            .0
            .as_deref()
            .and_then(|a| uuid::Uuid::parse_str(a).ok())
        else {
            continue;
        };
        attacked_events.write(AiEntityAttacked {
            entity_uuid: uuid.0.clone(),
            attacker_uuid,
        });
    }
}

/// Unregister synthetic tokens when AI-controlled entities are despawned.
///
/// Keys off `RemovedComponents<BehaviourSection>`: `BehaviourSection` is present
/// on every AI entity from spawn and is only ever removed on despawn (issue #832),
/// so its removal is a faithful despawn edge.
fn unregister_on_despawn(
    mut registry: ResMut<AiTokenRegistry>,
    mut removed: RemovedComponents<BehaviourSection>,
) {
    for entity in removed.read() {
        registry.unregister_by_bevy_entity(entity);
    }
}

// ── LOD Management ──────────────────────────────────────────────────────────────

/// Fractional hysteresis band applied on top of `AiProfile.sensor_range`
/// to prevent rapid LOD oscillation near the range boundary.
const LOD_HYSTERESIS: f32 = 0.2;

/// Minimum time (seconds) that must elapse before a demotion is allowed.
const LOD_DWELL_SECS: f64 = 2.0;

/// Evaluate LOD for every NPC ship against the high-fidelity **bubbles** in the
/// world (see [`LodBubble`]).
///
/// An NPC is promoted to `AiHighFidelity` while it is inside any bubble and
/// demoted once it has left every bubble by the hysteresis margin for the dwell
/// window. The anchors are the player `LocalShip` (an implicit bubble at
/// [`DEFAULT_PLAYER_LOD_BUBBLE_RADIUS`], or its authored `[lod_bubble]` radius)
/// plus every entity carrying a [`LodBubble`] — the station's smaller one. An
/// NPC that is itself a bubble carrier (the station) is held high-fidelity
/// unconditionally: it anchors a zone, so it is never demoted out of one.
/// `LocalShip` is never evaluated and keeps its `AiHighFidelity` marker.
///
/// Promotion keys on the ANCHOR's radius, not the NPC's own `sensor_range` as it
/// used to: "is this near enough to the action to run in full" is a fact about
/// how close a bubble is, not about how far the NPC can see. The most-inside
/// anchor (the one maximising `radius - distance`) decides, so an NPC counts as
/// inside if ANY bubble contains it, and the hysteresis is measured against that
/// same bubble.
fn lod_ai_ships(
    time: Res<Time>,
    player: Query<(&Transform, Option<&LodBubble>), (With<LocalShip>, With<Ship>)>,
    anchor_bubbles: Query<(&Transform, &LodBubble), Without<LocalShip>>,
    npcs: Query<
        (
            Entity,
            &Transform,
            Has<LodBubble>,
            Has<AiHighFidelity>,
            Option<&LodTransitionTimer>,
        ),
        (With<Ship>, Without<LocalShip>),
    >,
    mut commands: Commands,
) {
    let Ok((player_transform, player_bubble)) = player.single() else {
        return;
    };
    let now_secs = time.elapsed_secs() as f64;

    // Anchors: the player (implicit or authored radius) plus every non-player
    // bubble carrier (the station). `(x, z, radius)`.
    let player_radius = player_bubble
        .map(|b| b.radius)
        .unwrap_or(DEFAULT_PLAYER_LOD_BUBBLE_RADIUS);
    let mut anchors: Vec<(f32, f32, f32)> = vec![(
        player_transform.translation.x,
        player_transform.translation.z,
        player_radius,
    )];
    for (transform, bubble) in &anchor_bubbles {
        anchors.push((
            transform.translation.x,
            transform.translation.z,
            bubble.radius,
        ));
    }

    for (entity, transform, has_bubble, is_high, timer) in &npcs {
        let current_state = if is_high {
            LodState::High
        } else {
            LodState::Low
        };

        // A bubble carrier (the station) anchors its own zone — hold it high
        // unconditionally so a defended object's own guns always run.
        let new_state = if has_bubble {
            LodState::High
        } else {
            // The most-inside anchor: the bubble with the largest signed
            // penetration (`radius - distance`). Feeding that pair to
            // `evaluate_lod` makes "inside if ANY bubble contains it" fall out
            // of the same distance-vs-threshold comparison the single-bubble
            // form used, with the hysteresis judged against that same bubble.
            let (distance, radius) = anchors
                .iter()
                .map(|&(ax, az, r)| {
                    let dx = transform.translation.x - ax;
                    let dz = transform.translation.z - az;
                    ((dx * dx + dz * dz).sqrt(), r)
                })
                .max_by(|(d1, r1), (d2, r2)| {
                    (r1 - d1)
                        .partial_cmp(&(r2 - d2))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("anchors always contains at least the player");
            let last_change = timer.map(|t| t.last_state_change_secs).unwrap_or(0.0);
            evaluate_lod(
                current_state,
                distance,
                radius,
                now_secs,
                last_change,
                LOD_DWELL_SECS,
                LOD_HYSTERESIS,
            )
        };

        if new_state != current_state {
            let timer_comp = LodTransitionTimer {
                last_state_change_secs: now_secs,
            };
            match new_state {
                LodState::High => {
                    // The marker AND every AI intent/state component it implies
                    // (issue #692, extended by #693 for power, #695 for helm,
                    // #882 for the policy runtime state), from the ONE shared
                    // definition the player-ship spawn also uses. Re-inserting
                    // defaults on promotion IS the #882 AC5 "AI gains control"
                    // reset — `ai_policy_state_tick` then puts the machine in
                    // the policy's authored initial state on its first run.
                    //
                    // Power AI is stateless since issue #784 (the
                    // `PowerAiPolicy` is attached at spawn, not LOD-scoped), so
                    // no per-fidelity power state is in the set.
                    commands
                        .entity(entity)
                        .insert(ai_high_fidelity_components());
                    commands.entity(entity).insert(timer_comp);
                }
                LodState::Low => {
                    // Removed as the same unit that was inserted, so the two
                    // halves cannot drift: a re-promoted ship cannot resume a
                    // stale mid-manoeuvre policy state (issue #882 AC5).
                    commands.entity(entity).remove::<AiHighFidelityComponents>();
                    commands.entity(entity).insert(timer_comp);
                }
            }
        }
    }
}

/// The active Helm-relevant waypoint route a ship should be following, read
/// from its viewscreen blackboard.
///
/// `aggregate_doctrine_blackboards` publishes `scored_objectives` there for
/// every `BehaviourSection` ship regardless of LOD — the same entry
/// the per-axis helm AI reads on the high-LOD path (see `ship_plugin.rs`, the
/// "Shared helm-AI decision inputs" note).
/// Both the low-LOD steering path and the cursor evaluator resolve their route
/// through this one function so they can never disagree about which objective
/// owns the cursor.
///
/// Returns `(objective_id, waypoint_anchor_names, loop_path)` for the
/// highest-scored `Patrol` or `Reach` directive. `Reach` is modelled as a
/// one-waypoint, non-looping route.
fn active_waypoint_route(
    blackboards: &crate::server_app::ShipSystemBlackboards,
) -> Option<(String, Vec<String>, bool)> {
    let bb = match blackboards
        .0
        .get(&crate::ship::system_registry::viewscreen_system_id())
    {
        Some(crate::core::messages::SystemBlackboard::Viewscreen(v)) => v,
        _ => return None,
    };
    bb.scored_objectives
        .iter()
        .filter(|o| {
            o.score > 0.0
                && o.relevance
                    .contains(&crate::core::messages::SystemAffinity::Helm)
                && matches!(
                    o.directive,
                    crate::core::messages::AiDirective::Patrol { .. }
                        | crate::core::messages::AiDirective::Reach { .. }
                )
        })
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .and_then(|o| match &o.directive {
            crate::core::messages::AiDirective::Patrol { anchors, loop_path } => {
                Some((o.id.clone(), anchors.clone(), *loop_path))
            }
            crate::core::messages::AiDirective::Reach { anchor } => {
                Some((o.id.clone(), vec![anchor.clone()], false))
            }
            _ => None,
        })
}

/// The named target of this ship's top-scoring standing `Destroy` directive,
/// read from the same pre-scored `scored_objectives` pool `active_waypoint_route`
/// reads (issue #933 review follow-up).
///
/// Resolving through the scored pool — rather than the first `directive_kind
/// == "Destroy"` entry in authoring order — is what makes this agree with the
/// high-LOD Helm (`plan_helm_travel`, `ai/core.rs`) and with
/// `score_doctrine_pool`'s own `zero_gates`: a Destroy directive gated on
/// `not_attacked` scores 0 once the ship has been hit and is filtered out
/// here exactly as it is there, so a demoted, attacked ship stops steering at
/// a target the high-LOD Helm has already given up on (the shipped
/// `assault-starbase` doctrine in `combat_test.toml` is the concrete case).
///
/// `scored_objectives` is populated by `aggregate_doctrine_blackboards`, which
/// runs over every `BehaviourSection` ship regardless of LOD and scores against
/// that ship's own `hull_fraction`/`red_alert`/`attacked` facts — the same
/// facts `WorldConditions` needs — so a demoted ship's pool is exactly as
/// current as a high-LOD ship's; low LOD does not narrow what this can see.
/// An empty `target` (auto-acquire, no single position) is treated the same
/// as "no qualifying directive": there is nothing deterministic to turn
/// toward, only the cruise-speed decay applies.
fn active_destroy_target(blackboards: &crate::server_app::ShipSystemBlackboards) -> Option<String> {
    let bb = match blackboards
        .0
        .get(&crate::ship::system_registry::viewscreen_system_id())
    {
        Some(crate::core::messages::SystemBlackboard::Viewscreen(v)) => v,
        _ => return None,
    };
    bb.scored_objectives
        .iter()
        .filter(|o| {
            o.score > 0.0
                && o.relevance
                    .contains(&crate::core::messages::SystemAffinity::Helm)
                && matches!(
                    o.directive,
                    crate::core::messages::AiDirective::Destroy { .. }
                )
        })
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .and_then(|o| match &o.directive {
            crate::core::messages::AiDirective::Destroy { target } if !target.is_empty() => {
                Some(target.clone())
            }
            _ => None,
        })
}

/// Advance every ship's objective cursors as it reaches its waypoints.
///
/// Runs in `SimSet::Modifiers` — after `Physics` has moved the ships this
/// tick, so arrival is judged against fresh positions, and before `Publish`.
/// This is the single owner of `ObjectiveCursors` state: every steering path —
/// low-LOD (`simulate_low_lod_ships`) and high-LOD (`helm_patrol`) alike — only
/// *reads* the cursor, so a cursor can never be advanced twice in one tick.
///
/// Per ship, per active Helm-relevant `Patrol`/`Reach` objective: advance the
/// cursor via the pure `advance_cursor` (which judges arrival against the
/// radius and handles wraparound, terminal stops, settling degenerate looping
/// routes, and skipping waypoints whose anchors are unknown), then emit one
/// `AiWaypointReached` per waypoint it reports as consumed.
///
/// Covers all ships carrying a `BehaviourSection` regardless of LOD. Since
/// #702 that is the whole story: the high-LOD helm path (`helm_patrol`) reads
/// these same cursors rather than advancing a rival `AiMemory.waypoint_index`
/// of its own.
pub(crate) fn advance_objective_cursors(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<(
        Option<&BehaviourSection>,
        &ShipPhysics,
        &crate::server_app::ShipSystemBlackboards,
        &mut ObjectiveCursors,
        Option<&EntityUuid>,
    )>,
    mut reached: MessageWriter<AiWaypointReached>,
) {
    let Some(world_config) = world_config else {
        return;
    };
    let anchors = anchors_from_world_config(&world_config);

    for (behaviour, physics, blackboards, mut cursors, entity_uuid) in &mut ships {
        let Some((obj_id, waypoints, loop_path)) = active_waypoint_route(blackboards) else {
            continue;
        };

        // Arrival radius is authored per entity template in TOML
        // (`[behaviour] waypoint_arrival_radius`), so designers can tune how
        // close a ship must get before its cursor advances.
        let arrival_radius = behaviour
            .map(|b| b.0.waypoint_arrival_radius)
            .unwrap_or(crate::ai::WAYPOINT_ARRIVAL_RADIUS);
        let entity_pos = [physics.x, 0.0, physics.z];

        // Look up (or lazily insert) this objective's cursor by objective id.
        if !cursors.0.iter().any(|c| c.objective_id == obj_id) {
            cursors
                .0
                .push(crate::ai::patrol_cursor::PatrolCursor::new(obj_id.clone()));
        }
        let cursor = cursors
            .0
            .iter_mut()
            .find(|c| c.objective_id == obj_id)
            .expect("cursor entry just ensured to exist");

        // One tick can carry a ship past several waypoints at once (waypoints
        // spaced closer together than the arrival radius, or a slow tick), so
        // `advance_cursor` reports *every* waypoint it consumed rather than
        // just the first — one message each, or triggers keyed to the
        // intermediate waypoints would silently never fire.
        let reached_waypoints = crate::ai::patrol_cursor::advance_cursor(
            cursor,
            &waypoints,
            loop_path,
            entity_pos,
            &anchors,
            arrival_radius,
        );

        // Only ships with a UUID can be named by a scenario trigger; bare
        // test entities without one simply advance their cursor silently.
        let Some(uuid) = entity_uuid else {
            continue;
        };
        for waypoint in reached_waypoints {
            reached.write(AiWaypointReached {
                entity_uuid: uuid.0.clone(),
                objective_id: obj_id.clone(),
                waypoint,
            });
        }
    }
}

/// Cheap steering / forward-movement simulation for ships running at low
/// fidelity (those without the `AiHighFidelity` marker).
///
/// A low-LOD ship with an active Helm-relevant `Patrol`/`Reach` objective
/// cheaply follows its route: it snaps its heading toward the waypoint its
/// [`ObjectiveCursors`] entry currently points at and advances forward at
/// `forward_speed`. Ships with no such objective keep the pre-existing dumb
/// forward-drift so they don't regress to standing still.
///
/// # Arriving is not the same as having nowhere to go
///
/// A ship that has flown a non-looping route to its end has *arrived*, and coasts
/// to a stop where it is (`route_completed`). Lumping that in with "no route"
/// and drifting on is what sent the Requiem Courier — whose whole behaviour is
/// one `Reach` — sailing through its destination at cruise speed and out of the
/// scenario, ~100 u past the anchor twenty seconds later and still going. The
/// high-LOD path has always held station on arrival (`helm_navigate_to` returns
/// a zero decision inside the arrival radius); this is the low-LOD path agreeing
/// with it.
///
/// The dumb drift still covers the genuinely routeless cases: no objective, no
/// cursor component, an empty route, or an anchor the world does not define.
/// How long that last case drifts depends on the route:
///
/// * **Non-looping**: one tick. `advance_objective_cursors` steps past the
///   unknown anchor on the same tick, and the cursor either lands on a waypoint
///   that resolves or runs off the end, where `route_completed` takes over.
/// * **Looping**: indefinitely. A lap that finds nowhere to steer settles the
///   cursor on a *valid* index rather than running past the end (see "Settling"
///   on `advance_cursor`), so `route_completed` is `false` by construction and
///   this drift runs every tick for as long as the anchors stay undefined.
///
/// `ship_harrow_warhawk.toml` patrols `warhawk_patrol_a`/`_b`, which only
/// `combat_test.toml` declares — `before_the_fire.toml` spawns the same hull and
/// declares neither. It is inert there today because the speed ramp lives in the
/// `Some(target_pos)` branch, so a hull that never had a flyable route sits at
/// `forward_speed == 0` and drifts nowhere. The defect in the warhawk case is
/// content — a world that spawns a hull without declaring its patrol anchors —
/// which holding station would hide rather than fix. Arrival is different in
/// kind: there the route *did* resolve and the ship is where it was sent.
///
/// ## …unless the ship still has an order to carry out (issue #1012)
///
/// A completed route parks the ship *only* when there is nothing else scored
/// for it to fly at. `active_waypoint_route` sees a hull's `Patrol`/`Reach`
/// entries and nothing else, so a doctrine that authors BOTH kinds gets its
/// route flown and its `Destroy` silently dropped on arrival. That is exactly
/// what `combat_test.toml`'s assault waves author — `assault-starbase`
/// (Destroy @50) *and* `close-on-starbase` (Reach @35, the run-in to
/// `harrow_assault_point`) — and parking on the run-in point, 100 units off a
/// station the wave was sent to kill, with its top-scoring directive unserved,
/// is the loiter the playtest reported. So a finished route yields to a scored
/// `Destroy` whose target resolves in the `WorldSnapshot`: the ship commits to
/// the target instead of coasting to a stop, on the same authored turn rate and
/// cruise fraction the dead-reckoning fallback below uses. One thing the
/// divert does not do: honour `maintain_range` — it steers straight at the
/// target's position with no stand-off, so a low-LOD hull that catches up
/// overflies or circles the target at cruise speed instead of holding at the
/// authored range, harmless at production numbers since the high-fidelity
/// path takes over on promotion. With no such Destroy — the Requiem Courier,
/// whose whole behaviour is one `Reach` — the coast-to-stop is untouched.
///
/// This reaches the unknown-anchor case too, and deliberately: a route whose
/// anchors the world never defines reads as *completed* from the tick after the
/// cursor skips past them (see "What "finished" does and does not mean" on
/// `route_completed`), so post-#1012 such a ship commits to its scored Destroy
/// rather than parking where it happened to be standing. That is the better
/// failure mode for the same reason the warhawk paragraph gives: the content
/// defect is an undeclared anchor, and a wave that still flies at the thing it
/// was sent to kill is both closer to the authored intent and louder about the
/// missing anchor than one parked in empty space.
///
/// # A demoted ship's frozen exit speed does not dead-reckon forever (issue #933)
///
/// A ship *demoted* from high LOD mid-manoeuvre used to carry whatever
/// `forward_speed`/`yaw` it had at that instant into the dumb drift above and
/// keep it forever — any hull demoted while moving fast (boosted or otherwise)
/// left the scenario permanently. Two authored corrections now apply in the
/// dumb-drift branch. Both read off the ship's own `AiProfile` and are pure
/// functions of tick + state (no RNG): the frozen speed moves toward
/// `AiProfile::low_lod_cruise_fraction * max_speed` at
/// `AiProfile::low_lod_speed_decay_per_sec` (`ai::lod::decay_speed_toward`,
/// which ramps a stopped ship UP to the cruise fraction as readily as it decays
/// a boosted one down to it), and a ship whose doctrine carries a standing
/// `Destroy` directive naming a target that resolves in the (possibly stale)
/// `WorldSnapshot` turns its heading toward it at
/// `AiProfile::low_lod_turn_rate_fraction * max_yaw_rate`
/// (`ai::lod::step_yaw_toward`). An untargeted Destroy (empty `target`, the
/// same "no hostile in range" case the warhawk's routeless drift already
/// covers), and any Destroy its own `zero_gates` have scored to 0, get the
/// cruise decay only — there is no single position to turn toward. A bare test
/// entity with no `AiProfile` component at all keeps the pre-#933 unmodified
/// drift regardless of transition history.
///
/// ## Which ships the corrections reach (#933's gate, corrected by #1012)
///
/// Issue #933 gated both corrections on `LodTransitionTimer` being present —
/// i.e. on the ship having been through at least one High↔Low transition —
/// reasoning that a ship still in its first Low stretch since spawn was
/// "closing on whatever heading/speed its template or a `spawn_entity` override
/// set", deliberate authored content rather than a frozen exit velocity.
/// **That premise was false.** `entity_spawner::spawn_entity` — the function
/// that actually seeds `ShipPhysics` — takes no rotation parameter at all;
/// the authored value never enters it, and the `Quat` it builds there is made
/// from hardcoded zero literals, unrelated to any authored data. Every
/// spawner-path ship's `ShipPhysics` comes up at `yaw: 0.0`,
/// `forward_speed: 0.0` regardless. A `spawn_entity` trigger action's
/// `rotation` is applied separately: the effect in `world/server.rs`
/// overwrites the render `Transform` *after* `spawn_entity` returns, so it
/// turns the rendered hull and nothing else. A static `[[entity]]` row's
/// `rotation` is parsed but never applied anywhere —
/// `spawn_world_entities` resolves position only. (The one authored rotation
/// that *does* reach `ShipPhysics.yaw` is the player ship's
/// `[mission.player_spawn].rotation` in `server_app.rs`, not a spawner-path
/// hull.) A first-Low ship was never flying an authored approach; it was
/// sitting still, facing -Z.
///
/// The exemption therefore survives only where it says something true: a
/// first-Low ship with nothing scored to fly at keeps the pre-#933 drift — bare
/// fixtures, and hulls whose authored drift is all the content there is. A
/// *scored* `Destroy` that resolves in the snapshot is explicit authored intent
/// and overrides the exemption, so such a ship gets the turn and the cruise ramp
/// from its very first tick, promoted or not (issue #1012). A zero-gated Destroy
/// is not scored intent and changes nothing: `active_destroy_target` filters on
/// `score > 0.0`, so a first-Low ship already under fire still will not chase a
/// target its own doctrine has given up on.
///
/// Read-only with respect to the cursor: arrival detection and advancement
/// belong to `advance_objective_cursors` in `SimSet::Modifiers`.
///
/// This is the *low-fidelity* path. It reads the same `ObjectiveCursors` the
/// high-LOD path (`helm_patrol` in `ai/core.rs`) reads — since #702 there is
/// one cursor surface rather than two rival ones, so a ship promoted or demoted
/// between LODs resumes its route exactly where it left off.
///
/// # Sanctioned out-of-band `ShipPhysics` writer (issue #699)
///
/// `integrate_ship_physics` is the sole *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`. This system writes
/// `x`/`z`/`yaw` directly and is an intentional exception: it is filtered to
/// `Without<AiHighFidelity>`, i.e. exactly the ships that carry no helm intent
/// components at all, so the helm path cannot serve them and the two writers
/// can never touch the same entity. It deliberately does not opt into the
/// debug `HelmPhysicsWriteGuard`. See the writer-policy table on `ShipPhysics`
/// (`src/ship/state.rs`).
///
/// The "can never touch the same entity" claim above is not left to this
/// comment: `low_lod_and_helm_ship_physics_writers_can_never_share_an_entity`
/// (`tests/headless_runner.rs`) reads the access sets Bevy derived from both
/// systems as the production plugins registered them, and fails if either
/// filter is widened enough for the two to overlap.
fn simulate_low_lod_ships(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    world_snapshot: Option<Res<WorldSnapshot>>,
    mut ships: Query<
        (
            &mut ShipPhysics,
            Option<&crate::server_app::ShipSystemBlackboards>,
            Option<&ObjectiveCursors>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Option<&AiProfile>,
            Option<&LodTransitionTimer>,
            // Read-only, and both `Option`, so neither filters the iteration
            // set. The hull's own collider radius and its authored avoidance
            // tuning reach the low-LOD hazard assessment (issue #968) — see
            // `low_lod_avoid_yaw`.
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::BehaviourSection>,
            // Needed only to keep this ship's own snapshot entry out of its own
            // hazard picture — see `low_lod_avoid_yaw`.
            Option<&crate::entities::spawner::EntityUuid>,
        ),
        (With<Ship>, Without<AiHighFidelity>),
    >,
) {
    let dt = time.delta_secs();
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();

    // Default speed fraction when the low-LOD path has a valid route but the
    // helm config is absent (unusual — all shipped hulls carry one).
    const LOW_LOD_SPEED_FRACTION: f32 = 0.4;
    // Simple ramp rate so forward_speed doesn't snap from 0 to max in one tick.
    const LOW_LOD_ACCEL_PER_SEC: f32 = 10.0;
    // Fallback used only when a ship has no `HelmConsoleSection` at all — its
    // authored `max_yaw_rate` is unavailable, so the dead-reckoning fallback's
    // return-steering (issue #933) needs *some* turn-rate ceiling to clamp to.
    const LOW_LOD_DEFAULT_MAX_YAW_RATE: f32 = 0.5;

    // Every parse-time default in one place, for hulls that author no
    // `[behaviour]` block at all. Built once rather than per ship: it owns a
    // (here always empty) doctrine `Vec`.
    let default_behaviour = crate::entities::config::BehaviourConfig::default();

    for (
        mut physics,
        blackboards,
        cursors,
        helm_section,
        ai_profile,
        lod_timer,
        collider_section,
        behaviour_section,
        entity_uuid,
    ) in &mut ships
    {
        // No Helm console means no engine: this hull cannot propel itself, so
        // its top speed is 0, not a fabricated default. That is the difference
        // between "a mover whose authored max_speed we couldn't read" and "a
        // thing with no propulsion at all" — combat_test's Starbase Alpha (a
        // `StaticPointDefence`) carries the full `Ship` substrate for its
        // targeting and beams, so it rides this path once the player drifts far
        // enough to demote it, but with max_speed 0 every speed ramp below
        // multiplies to 0 and it stays put. It still gets every OTHER low-LOD
        // update (yaw, hazard avoidance) — the platform is not special-cased out
        // of the path, it simply has nothing to accelerate. Any future
        // propulsion-less Ship entity (mine, comms buoy) inherits the same.
        //
        // A helm section that IS present but authors a non-positive max_speed is
        // malformed authoring, not "no engine": keep the fixture fallback so the
        // bare-`App` movers that rely on it still move.
        let max_speed = match helm_section {
            None => 0.0,
            Some(h) if h.0.max_speed > 0.0 => h.0.max_speed,
            Some(_) => 20.0,
        };
        // The same facts the high-fidelity planner feeds `assess_hazards`, read
        // off this hull rather than defaulted (issue #968): its collider radius,
        // and the whole authored `[behaviour]` avoidance block (buffer,
        // look-ahead, severity exponent, deviation ceiling).
        let self_radius = collider_section.map(|c| c.0.radius.max(0.0)).unwrap_or(0.0);
        let behaviour = behaviour_section
            .map(|b| &b.0)
            .unwrap_or(&default_behaviour);
        let self_uuid = entity_uuid
            .and_then(|u| uuid::Uuid::parse_str(&u.0).ok())
            .unwrap_or_default();
        let max_yaw_rate = helm_section
            .map(|h| h.0.max_yaw_rate)
            .filter(|&r| r > 0.0)
            .unwrap_or(LOW_LOD_DEFAULT_MAX_YAW_RATE);
        let route = blackboards.and_then(active_waypoint_route);

        // Where this ship's *top-scoring, still-qualifying* standing `Destroy`
        // directive points, if it names a target that resolves in the (possibly
        // stale) `WorldSnapshot`. Resolved through the scored pool
        // (`active_destroy_target`) rather than the first `directive_kind ==
        // "Destroy"` entry in authoring order, so this agrees with
        // `plan_helm_travel`/`score_doctrine_pool` instead of steering at a
        // target the high-LOD Helm has already given up on (issue #933 review
        // follow-up). Untargeted Destroy directives (auto-acquire, empty
        // target) and zero-gated ones resolve to `None` — there is nothing
        // deterministic to turn toward.
        //
        // A closure rather than a value: a ship following a live route never
        // asks, so the snapshot scan is paid only where it can change the
        // outcome.
        let resolve_destroy_target = || -> Option<[f32; 3]> {
            blackboards
                .and_then(active_destroy_target)
                .and_then(|target_name| {
                    world_snapshot.as_ref().and_then(|snap| {
                        snap.entities
                            .iter()
                            .find(|e| e.name.as_deref() == Some(target_name.as_str()))
                            .map(|e| e.position)
                    })
                })
        };

        // Steer along the route only when we have a Patrol/Reach objective AND
        // a `ObjectiveCursors` component tracking the waypoint index. A low-LOD
        // ship lacking either just falls through to the dumb forward-move
        // below. A ship whose cursor entry has not been created yet (the
        // evaluator inserts it at the end of this tick) steers toward the
        // first waypoint in the meantime.
        if let (Some((obj_id, waypoints, loop_path)), Some(cursors)) = (route, cursors) {
            let index = cursors
                .0
                .iter()
                .find(|c| c.objective_id == obj_id)
                .map(|c| c.index())
                .unwrap_or(0);

            let target =
                crate::ai::patrol_cursor::cursor_target(index, &waypoints, loop_path, &anchors);

            if let Some(target_pos) = target {
                // Cheap steering: snap yaw toward the target XZ bearing, then
                // advance forward. The forward vector below is
                // (sin(yaw), -cos(yaw)), so the bearing that makes the ship
                // face `target` is `dx.atan2(-dz)` (the same world-bearing
                // convention used across the sim's yaw math).
                let dx = target_pos[0] - physics.x;
                let dz = target_pos[2] - physics.z;
                if dx * dx + dz * dz > f32::EPSILON {
                    physics.yaw = simmath::atan2(dx, -dz);
                }
                // Accelerate toward the target speed so a ship that spawned
                // far from the player (and never had `integrate_ship_physics`
                // running for it) actually moves. Without this a low-LOD ship
                // stays stuck at forward_speed = 0 forever.
                let target_speed = max_speed * LOW_LOD_SPEED_FRACTION;
                if physics.forward_speed < target_speed {
                    physics.forward_speed =
                        (physics.forward_speed + LOW_LOD_ACCEL_PER_SEC * dt).min(target_speed);
                }
                physics.yaw = low_lod_avoid_yaw(
                    physics.yaw,
                    [physics.x, 0.0, physics.z],
                    physics.forward_speed,
                    self_radius,
                    self_uuid,
                    behaviour,
                    world_snapshot.as_deref(),
                );
                physics.x += physics.forward_speed * simmath::sin(physics.yaw) * dt;
                physics.z -= physics.forward_speed * simmath::cos(physics.yaw) * dt;
                continue;
            }
            // Route flown to its end: the ship is where its objective sent it.
            if crate::ai::patrol_cursor::route_completed(index, &waypoints, loop_path) {
                // The `ai_profile` test is the OUTER one, and `and_then` is
                // lazy, so a hull with no `AiProfile` never pays for
                // `resolve_destroy_target`'s snapshot scan: it has no authored
                // cruise/turn tuning to commit with, so the answer could not
                // change what it does. Matching on the `(ai_profile, ...)` pair
                // would evaluate the scan for every such ship every tick.
                if let Some((profile, target_pos)) =
                    ai_profile.and_then(|p| resolve_destroy_target().map(|pos| (p, pos)))
                {
                    // …but arriving is not the same as being done. A scored
                    // `Destroy` outranks a finished run-in: the route was the
                    // approach, the Destroy is the point of it, and parking on
                    // the anchor is the loiter issue #1012 is about. Commit —
                    // same authored turn rate and cruise fraction the
                    // dead-reckoning fallback below uses.
                    low_lod_objective_steer(
                        &mut physics,
                        Some(target_pos),
                        profile,
                        max_speed,
                        max_yaw_rate,
                        dt,
                    );
                } else {
                    // Nothing left to fly at: coast to a stop on the same ramp
                    // it accelerated on rather than drifting on forever — see
                    // "Arriving is not the same as having nowhere to go" above.
                    physics.forward_speed =
                        (physics.forward_speed - LOW_LOD_ACCEL_PER_SEC * dt).max(0.0);
                }
                physics.yaw = low_lod_avoid_yaw(
                    physics.yaw,
                    [physics.x, 0.0, physics.z],
                    physics.forward_speed,
                    self_radius,
                    self_uuid,
                    behaviour,
                    world_snapshot.as_deref(),
                );
                physics.x += physics.forward_speed * simmath::sin(physics.yaw) * dt;
                physics.z -= physics.forward_speed * simmath::cos(physics.yaw) * dt;
                continue;
            }

            // `target == None` for a route that is not finished (empty route /
            // unknown anchor) → fall through to the dumb forward-move fallback
            // below. On a non-looping route the evaluator skips past an unknown
            // anchor on this same tick, so that drift lasts one tick; a looping
            // route with no resolvable anchor settles on a valid index instead
            // and keeps drifting every tick — see "Arriving is not the same as
            // having nowhere to go" above for why that is left as-is.
        }

        // Dumb forward-move fallback: no patrol objective, no cursor component,
        // or a stalled/terminal patrol. This used to be a pure frozen-velocity
        // extrapolation — whatever forward_speed and yaw the ship had at the
        // moment of demotion, it kept forever, so a hull demoted mid-manoeuvre
        // at boosted speed left the scenario permanently (issue #933). Two
        // authored, deterministic corrections apply now, both driven off this
        // ship's own `AiProfile`, and they reach a ship for either of two
        // reasons:
        //
        //   * `LodTransitionTimer` is present — the ship has been through at
        //     least one LOD transition, so its speed and heading really are a
        //     frozen exit velocity. That is issue #933's case.
        //   * a scored `Destroy` resolves — the ship has an order to carry out.
        //     A ship in its very first Low stretch since spawn used to be
        //     exempt outright, on the since-disproved premise that its heading
        //     was authored content worth protecting. `entity_spawner::spawn_entity`
        //     takes no rotation parameter — the authored value never reaches
        //     it, and `ShipPhysics` is seeded at `yaw: 0.0` regardless. A
        //     `spawn_entity` trigger's `rotation` and a static `[[entity]]`
        //     row's `rotation` both land on the render `Transform` only,
        //     applied after the sim state is already set — so every
        //     spawner-path ship starts facing -Z at zero speed and there was
        //     never an authored approach there to preserve. A wave told to
        //     kill something must be allowed to turn and go (issue #1012).
        //
        // A first-Low ship with NOTHING scored to fly at still keeps the
        // pre-#933 drift — that is the half of the exemption that was true —
        // and a bare test entity with no `AiProfile` at all is left alone
        // either way, preserving existing coverage that never opted into
        // low-LOD authoring:
        if let Some(profile) = ai_profile {
            let destroy_target_pos = resolve_destroy_target();
            if lod_timer.is_some() || destroy_target_pos.is_some() {
                low_lod_objective_steer(
                    &mut physics,
                    destroy_target_pos,
                    profile,
                    max_speed,
                    max_yaw_rate,
                    dt,
                );
            }
        }

        physics.yaw = low_lod_avoid_yaw(
            physics.yaw,
            [physics.x, 0.0, physics.z],
            physics.forward_speed,
            self_radius,
            self_uuid,
            behaviour,
            world_snapshot.as_deref(),
        );
        physics.x += physics.forward_speed * simmath::sin(physics.yaw) * dt;
        physics.z -= physics.forward_speed * simmath::cos(physics.yaw) * dt;
    }
}

/// The low-LOD "carry out the standing order" correction: bring `forward_speed`
/// to this hull's authored cruise fraction and, when a scored `Destroy` target
/// resolved, turn the heading toward it at the authored fraction of the hull's
/// `max_yaw_rate`.
///
/// One function so the two callers in [`simulate_low_lod_ships`] — a ship that
/// has flown its run-in to the end (issue #1012) and a ship dead-reckoning with
/// no route at all (issue #933) — cannot drift apart on how hard a low-LOD ship
/// commits. `destroy_target_pos` is `None` for an untargeted or zero-gated
/// Destroy, in which case the speed correction applies alone: there is no single
/// position to turn toward.
///
/// `decay_speed_toward` is bidirectional, so this is a *ramp* for a ship parked
/// on its arrival anchor at zero speed just as much as it is a decay for one
/// demoted mid-boost. Pure function of tick + state: no RNG, no hidden clock.
fn low_lod_objective_steer(
    physics: &mut ShipPhysics,
    destroy_target_pos: Option<[f32; 3]>,
    profile: &AiProfile,
    max_speed: f32,
    max_yaw_rate: f32,
    dt: f32,
) {
    let cruise_speed = max_speed * profile.low_lod_cruise_fraction;
    physics.forward_speed = crate::ai::lod::decay_speed_toward(
        physics.forward_speed,
        cruise_speed,
        profile.low_lod_speed_decay_per_sec,
        dt,
    );

    let Some(target_pos) = destroy_target_pos else {
        return;
    };
    let dx = target_pos[0] - physics.x;
    let dz = target_pos[2] - physics.z;
    if dx * dx + dz * dz > f32::EPSILON {
        // The forward vector is (sin(yaw), -cos(yaw)), so the bearing that
        // faces `target_pos` is `dx.atan2(-dz)` — the same world-bearing
        // convention the route-steering branch above uses.
        let desired_yaw = simmath::atan2(dx, -dz);
        let max_step = max_yaw_rate * profile.low_lod_turn_rate_fraction * dt;
        physics.yaw = crate::ai::lod::step_yaw_toward(physics.yaw, desired_yaw, max_step);
    }
}

/// Bend a low-LOD ship's heading around imminent collisions, using the same
/// projected-collision model the high-fidelity Helm AI steers on
/// (`crate::ai::avoidance_steering`).
///
/// Dead-reckoned ships (demoted out of `AiHighFidelity`) have no helm intent
/// components, so `helm_motion_planner` never runs for them and they used to
/// fly straight through everything in `WorldSnapshot` — including field
/// asteroids, which the snapshot has carried since the fix for "AI collision
/// avoidance flew straight through asteroid fields" (see `build_world_snapshot`).
/// That fix only reached ships still on the full Helm AI path; a ship a
/// player is not currently looking at is exactly the case most likely to be
/// demoted, so most of the asteroid-field traffic was never covered. This is
/// a direct yaw bend rather than a steering command because there is no
/// per-axis actuator to route one through at this fidelity.
///
/// # Why a TURN and not a push (issue #968)
///
/// This used to take `assess_hazards`' repulsion vector as a desired heading and
/// step toward it at `max_yaw_rate * dt * urgency`. Two things were wrong with
/// that, and only the first was visible while the response was too weak to stop
/// anything:
///
/// * the repulsion points radially AWAY from the obstacle, so the ship's answer
///   to "there is a rock on my route" was to face back the way it came. The
///   caller re-snaps yaw onto the route bearing every tick, so the two fought:
///   turn out, clear the buffer, snap back, drive in. Once severity was fixed
///   the ship stopped penetrating and simply pinned itself against the rock's
///   skin instead, its patrol over — the same trapped mission this issue is
///   about, reached from the other side.
/// * the step was rate-limited against a bearing the caller had already snapped,
///   so at 60 Hz the heading could never deviate by more than a single tick's
///   turn from the route no matter how urgent the hazard.
///
/// Both are answered by steering rather than pushing: the deviation is an ANGLE
/// off the route bearing, applied after the caller's snap and proportional to
/// the threat, up to the hull's authored
/// [`BehaviourConfig::low_lod_avoidance_deviation_rad`](crate::entities::config::BehaviourConfig::low_lod_avoidance_deviation_rad).
/// A ship at full threat flies the tangent — around the obstacle — and eases
/// back onto its route as it clears. Stateless, so nothing has to be unwound
/// — true for the route branch above, where `physics.yaw` is re-snapped to
/// the route bearing by `atan2` every tick before this runs, so the bend
/// never accumulates. On the divert and dead-reckoning branches, though,
/// there is no snap: this function's output IS next tick's `physics.yaw`, so
/// a bent heading only relaxes at `low_lod_objective_steer`'s rate-limited
/// `step_yaw_toward` — the doctrine turn rate, e.g. ~5.7s to unwind a
/// full-magnitude bend on a Harrow destroyer diverting at full threat. That
/// is bounded and self-correcting, not divergent — the same pre-#933
/// property this module already relies on elsewhere, now shared by the
/// divert as well.
///
/// One consequence worth naming: the hull's `max_yaw_rate` no longer bounds this
/// manoeuvre. The old form stepped toward a heading at `max_yaw_rate * dt`; this
/// one sets the heading outright, so a battleship swings as sharply as a courier
/// unless the two author different deviation ceilings. That is the trade for
/// being able to deviate at all — see the constant's own note.
///
/// # What this path does NOT share with `assess_hazards` (issue #968)
///
/// [`crate::ai::avoidance_steering`] scans every entity in the snapshot. It does
/// not apply `assess_hazards`' two authored filters: the `dangerous` flag, and
/// the mobile-only ignore-smaller rule (`hazard_ignore_size_ratio`) that issue
/// #958 records as a doctrine invariant — static terrain is avoided at any
/// relative size, a small SHIP may be swept past. Both are inert in shipped
/// content today (`WorldSnapshot` carries only real obstacles, and no hull
/// authors a non-zero ratio), so the two fidelities agree. The first hull to
/// author the ratio will behave differently demoted than promoted, and that is
/// the point at which this needs the filters rather than a note.
///
/// `self_uuid` excludes the ship's own `WorldSnapshot` entry, which projects to
/// exactly its own position and would otherwise register as an unavoidable
/// full-threat collision with itself.
fn low_lod_avoid_yaw(
    yaw: f32,
    pos: [f32; 3],
    forward_speed: f32,
    self_radius: f32,
    self_uuid: uuid::Uuid,
    behaviour: &crate::entities::config::BehaviourConfig,
    snapshot: Option<&WorldSnapshot>,
) -> f32 {
    let Some(snapshot) = snapshot else {
        return yaw;
    };
    // `self_radius` and the hull's OWN authored avoidance tuning are used here
    // rather than the module defaults (issue #968). A demoted hull was assessing
    // hazards as if it were a point: the required clearance was short by its own
    // hull radius, and the severity ramp used the parse default even on a hull
    // that had authored a wider standoff. Both pushed a low-LOD ship's reaction
    // late — measured on `combat_test`, the two Harrow pickets ground through
    // radius-4 rocks to 3.2 units of penetration, being kicked back to the
    // surface once a second and driving straight back in.
    let steering = crate::ai::avoidance_steering(
        pos,
        yaw,
        forward_speed,
        self_radius,
        self_uuid,
        &snapshot.entities,
        behaviour.avoidance_buffer,
        behaviour.avoidance_look_ahead_secs,
        behaviour.hazard_threat_exponent,
    );
    if steering == 0.0 {
        return yaw;
    }
    yaw + steering * behaviour.low_lod_avoidance_deviation_rad
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "server_tests.rs"]
mod tests;
