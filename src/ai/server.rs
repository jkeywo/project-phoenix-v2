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
use crate::entity_spawner::{BehaviourSection, EntityUuid};
use crate::server_app::{LocalShip, Ship};
use crate::ship_state::ShipPhysics;

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

    /// Look up the Bevy `Entity` for an entity UUID string.
    pub fn bevy_entity_for_uuid(&self, entity_uuid: &str) -> Option<Entity> {
        self.uuid_to_bevy.get(entity_uuid).copied()
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
    crate::console_ai_plugin::ShipFrequencyHintState,
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
        crate::console_ai_plugin::ShipFrequencyHintState::default(),
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

/// AI personality and capability profile for NPC entities.
#[derive(Component, Clone, Debug)]
pub struct AiProfile {
    pub aggression: f32,
    pub sensor_range: f32,
    /// See [`crate::entity_config::AiProfileConfig::low_lod_cruise_fraction`].
    pub low_lod_cruise_fraction: f32,
    /// See [`crate::entity_config::AiProfileConfig::low_lod_speed_decay_per_sec`].
    pub low_lod_speed_decay_per_sec: f32,
    /// See [`crate::entity_config::AiProfileConfig::low_lod_turn_rate_fraction`].
    pub low_lod_turn_rate_fraction: f32,
}

impl Default for AiProfile {
    fn default() -> Self {
        Self {
            aggression: 0.5,
            sensor_range: 100.0,
            low_lod_cruise_fraction: crate::entity_config::default_low_lod_cruise_fraction(),
            low_lod_speed_decay_per_sec: crate::entity_config::default_low_lod_speed_decay_per_sec(
            ),
            low_lod_turn_rate_fraction: crate::entity_config::default_low_lod_turn_rate_fraction(),
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

/// Marker component added to an AI entity when its owning scenario is unloaded.
///
/// `tick_ai_controllers` reads this component to set `scenario_unloaded: true`
/// in the `WorldView`. The component persists until `tick_ai_controllers`
/// removes it (or until the entity despawns alongside its scenario cleanup).
#[derive(Component)]
pub struct ScenarioUnloadedMarker;

/// Resource kept for backward-compatibility; no longer used for signalling.
#[derive(Resource, Default)]
pub struct ScenariosBeingUnloaded(pub std::collections::HashSet<String>);

/// Emitted by the AI plugin when a ship's [`LastShipAttacker`] changes to name
/// a new attacker.
///
/// The world plugin observes this event to evaluate `on_entity_attacked`
/// trigger conditions without a direct dependency on the AI module.
///
/// [`LastShipAttacker`]: crate::weapons_plugin::LastShipAttacker
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
        &crate::entity_spawner::EntityUuid,
        &Transform,
        Option<&crate::entity_spawner::EntityName>,
        Option<&crate::entity_spawner::FactionComponent>,
        Option<&crate::entity_spawner::EntitySystemHull>,
        Option<&crate::entity_spawner::ColliderSection>,
        Option<&crate::ship_state::ShipPhysics>,
        // Direct-fire reach (issue #788): the longest range this entity can put
        // unguided fire at, published as a threat fact so another ship's helm
        // can derive a safe standoff ring from it. Needs the control sources (to
        // know which banks are offline) and nothing else — reach is the AUTHORED
        // range of each bank, and issue #955 took `ModifierSlot::RadarRange` out
        // of it, so this query no longer reads `ShipModifiers` at all.
        Option<&crate::ship_plugin::ShipSystemControlSources>,
        Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
        Option<&crate::weapons_plugin::BlasterSystemResource>,
    )>,
    asteroids: Query<
        (
            &crate::simulation::AsteroidUuid,
            &Transform,
            &crate::entity_spawner::ColliderSection,
        ),
        With<crate::simulation::Asteroid>,
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
}

/// This entity's longest usable direct-fire reach (issue #788), or `0.0` when it
/// carries no direct-fire armament.
///
/// The Bevy adapter for the pure
/// [`longest_usable_direct_fire_range`](crate::weapons_plugin::longest_usable_direct_fire_range):
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
    phasers: Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
    blasters: Option<&crate::weapons_plugin::BlasterSystemResource>,
) -> f32 {
    use crate::weapons_plugin::{longest_usable_direct_fire_range, DirectFireEmitter};

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
    phasers: Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
    blasters: Option<&crate::weapons_plugin::BlasterSystemResource>,
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
    phasers: Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
    blasters: Option<&crate::weapons_plugin::BlasterSystemResource>,
) -> Vec<(bool, crate::weapons::arc_geometry::WeaponArcBank)> {
    use crate::weapons::arc_geometry::WeaponArcBank;

    // No control sources (a bare test spawn) means nothing is known to be
    // offline, which is the same reading the arc-bearing path takes.
    let is_offline = |sid: Option<crate::messages::SystemId>| -> bool {
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
                crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
            };
            banks.push((
                !is_offline(crate::system_registry::phaser_bank_system_id(&b.id)),
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
                !is_offline(crate::system_registry::blaster_bank_system_id(
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
    mut query: Query<
        (
            // Optional so a static point-defence platform (the station), which
            // authors no `[behaviour]`, still gets a Viewscreen blackboard. Its
            // phaser AI (`ai_phaser_auto_fire`) aims at the `combat_lock` this
            // publishes, so without an entry here the station's Tactical lock was
            // consumed by nothing and it never fired. The `Or<>` filter keeps
            // scenery (stars, asteroids) out — only doctrine ships and turrets
            // qualify.
            Option<&BehaviourSection>,
            &crate::entity_spawner::EntitySystemHull,
            &mut crate::server_app::ShipSystemBlackboards,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::ship::combat_activity::RecentCombatActivity>,
            Option<&crate::weapons_plugin::LastShipAttacker>,
        ),
        Or<(
            With<BehaviourSection>,
            With<crate::entity_spawner::StaticPointDefence>,
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
        .unwrap_or_else(|| crate::entity_config::GlobalConfig::default().attacked_memory_secs);
    for (behaviour, hull, mut blackboards, red_alert_opt, activity_opt, last_attacker_opt) in
        &mut query
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
            Some(crate::messages::SystemBlackboard::TacticalRadar(bb)) => {
                bb.selected_target.clone()
            }
            _ => None,
        };
        let science_target = match blackboards
            .0
            .get(&crate::ship::system_registry::sensor_radar_system_id())
        {
            Some(crate::messages::SystemBlackboard::SensorRadar(bb)) => bb.selected_target.clone(),
            _ => None,
        };
        let viewscreen_bb = crate::messages::ViewscreenBlackboard {
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
            crate::messages::SystemId(
                crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID.to_string(),
            ),
            crate::messages::SystemBlackboard::Viewscreen(viewscreen_bb),
        );
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiTokenRegistry>();
        app.init_resource::<ScenariosBeingUnloaded>();
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
/// [`LastShipAttacker`]: crate::weapons_plugin::LastShipAttacker
fn emit_attacked_on_new_attacker(
    query: Query<
        (&EntityUuid, &crate::weapons_plugin::LastShipAttacker),
        Changed<crate::weapons_plugin::LastShipAttacker>,
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
        .get(&crate::system_registry::viewscreen_system_id())
    {
        Some(crate::messages::SystemBlackboard::Viewscreen(v)) => v,
        _ => return None,
    };
    bb.scored_objectives
        .iter()
        .filter(|o| {
            o.score > 0.0
                && o.relevance.contains(&crate::messages::SystemAffinity::Helm)
                && matches!(
                    o.directive,
                    crate::messages::AiDirective::Patrol { .. }
                        | crate::messages::AiDirective::Reach { .. }
                )
        })
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .and_then(|o| match &o.directive {
            crate::messages::AiDirective::Patrol { anchors, loop_path } => {
                Some((o.id.clone(), anchors.clone(), *loop_path))
            }
            crate::messages::AiDirective::Reach { anchor } => {
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
        .get(&crate::system_registry::viewscreen_system_id())
    {
        Some(crate::messages::SystemBlackboard::Viewscreen(v)) => v,
        _ => return None,
    };
    bb.scored_objectives
        .iter()
        .filter(|o| {
            o.score > 0.0
                && o.relevance.contains(&crate::messages::SystemAffinity::Helm)
                && matches!(o.directive, crate::messages::AiDirective::Destroy { .. })
        })
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .and_then(|o| match &o.directive {
            crate::messages::AiDirective::Destroy { target } if !target.is_empty() => {
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
            Option<&crate::entity_spawner::EntityUuid>,
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
    let default_behaviour = crate::entity_config::BehaviourConfig::default();

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
/// [`BehaviourConfig::low_lod_avoidance_deviation_rad`](crate::entity_config::BehaviourConfig::low_lod_avoidance_deviation_rad).
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
    behaviour: &crate::entity_config::BehaviourConfig,
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
mod tests {
    use super::*;

    // ── anchors_from_world_config (PRD #337/#338 slice 1) ──────────────────

    #[test]
    fn anchors_from_world_config_clones_anchor_table() {
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("alpha".to_string(), [10.0, 0.0, 20.0]);
        world.anchors.insert("beta".to_string(), [-5.0, 1.5, 30.0]);

        let anchors = anchors_from_world_config(&world);
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors.get("alpha"), Some(&[10.0, 0.0, 20.0]));
        assert_eq!(anchors.get("beta"), Some(&[-5.0, 1.5, 30.0]));
    }

    #[test]
    fn anchors_from_world_config_returns_empty_when_no_anchors() {
        let world = crate::world::config::WorldConfig::default();
        assert!(anchors_from_world_config(&world).is_empty());
    }

    // ── build_world_snapshot: asteroids as obstacles ───────────────────────

    fn snapshot_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<WorldSnapshot>()
            .add_systems(Update, build_world_snapshot);
        app
    }

    /// Field asteroids carry `AsteroidUuid`, not `EntityUuid`, because they are
    /// streamed rather than spawned through `spawn_entity`. They used to fall
    /// out of the snapshot entirely, which left `avoidance_steering` blind to
    /// every rock in the field.
    #[test]
    fn world_snapshot_includes_field_asteroids_with_their_radius() {
        let mut app = snapshot_test_app();
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(30.0, 0.0, -12.0),
            crate::entity_spawner::ColliderSection(crate::entity_config::ColliderConfig {
                shape: crate::entity_config::ColliderShape::Ball,
                radius: 4.0,
                length: 0.0,
                movable: false,
            }),
        ));

        app.update();

        let snapshot = app.world().resource::<WorldSnapshot>();
        assert_eq!(
            snapshot.entities.len(),
            1,
            "asteroid must reach the snapshot"
        );
        let rock = &snapshot.entities[0];
        assert_eq!(rock.radius, 4.0, "avoidance sizes the obstacle off radius");
        assert_eq!(rock.position, [30.0, 0.0, -12.0]);
        assert_eq!(rock.faction, None, "a rock is hostile to nobody");
        assert_eq!(rock.forward_speed, 0.0);
    }

    // ── build_world_snapshot: direct-fire reach (issue #788) ───────────────
    //
    // This is genuinely new CROSS-ENTITY plumbing: before #788 nothing published
    // one ship's weapon reach where another ship's AI could read it (the
    // long-dead `WorldView.entity_weapons_range` was zeroed by every producer).
    // The tests below drive the real system, so a regression that stops
    // publishing the field fails here rather than showing up as a destroyer that
    // quietly orbits at its authored margin.

    /// A hull with one 200-unit phaser bank and one 320-unit blaster bank, plus
    /// the components the spawner would attach for them.
    fn armed_hull_components() -> (
        crate::weapons_plugin::PhaserCombatConfigResource,
        crate::weapons_plugin::BlasterSystemResource,
    ) {
        let cfg = crate::entity_config::EntityConfig::from_toml(
            // Each bank AUTHORS its open-fire policy: since #885b stage 5d
            // strict AI-declaration mode rejects a bank that declares neither a
            // policy nor an explicit idle, because nothing would be synthesised
            // for it and it would simply never fire.
            //
            // The ship-level `[weapons_console.ai]` doctrine below is owed for
            // the same reason since issue #956, and this fixture is exactly the
            // hull that makes the point: it is ARMED and carries no
            // `[behaviour]`, so gating the doctrine on `[behaviour]` would have
            // let it ship with no arc-bearing preference at all.
            r#"
name = "Armed"
[weapons_console]

[weapons_console.ai]

[[weapons_console.ai.rule]]
priority = 0
channel = "arc_bearing_first"
when = "true"
verb = "bring_phasers_to_bear"

[[weapons_console.ai.rule]]
priority = 0
channel = "arc_bearing_second"
when = "true"
verb = "bring_blasters_to_bear"

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0
fire_arc_deg = 90
auto_arc_deg = 90
beam_range = 200
beam_damage_per_sec = 3
beam_duration_secs = 4
cooldown_secs = 4

[[weapons_console.phaser_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "fire_phaser"

[[weapons_console.blaster_banks]]
id = "lance"
facing_deg = 0
range = 320

[[weapons_console.blaster_banks.ai.rule]]
priority = 0
channel = "blaster_fire"
when = "true"
verb = "fire_blaster"
"#,
        )
        .expect("fixture hull must parse");
        let wc = cfg
            .weapons_console
            .expect("hull declares [weapons_console]");
        (
            crate::weapons_plugin::PhaserCombatConfigResource(
                crate::entity_config::PhaserCombatConfig::from_weapons_console(&wc),
            ),
            crate::weapons_plugin::BlasterSystemResource(
                wc.blaster_banks
                    .iter()
                    .map(|bc| crate::blaster::BlasterSystem::new(bc.to_runtime()))
                    .collect(),
            ),
        )
    }

    /// The snapshot publishes the LONGEST reach across the entity's direct-fire
    /// banks — here the blaster, which outranges the phaser.
    #[test]
    fn world_snapshot_publishes_the_longest_direct_fire_reach() {
        let mut app = snapshot_test_app();
        let (phasers, blasters) = armed_hull_components();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            phasers,
            blasters,
        ));

        app.update();

        let snapshot = app.world().resource::<WorldSnapshot>();
        assert_eq!(
            snapshot.entities[0].direct_fire_range, 320.0,
            "the reach is the longest bank, not the first or the sum"
        );
    }

    /// **The reach fact is the authored range, not the radar-scaled one
    /// (issue #955).**
    ///
    /// `entity_direct_fire_range` used to multiply each phaser bank's authored
    /// `beam_range` by `ModifierSlot::RadarRange`, so the standoff ring another
    /// helm derived from this fact shrank and grew with the TARGET's sensor
    /// power — a hull resting `sensors` at 1 published two thirds of the reach
    /// its guns were credited with. (That power group is gone since #952; the
    /// slot survives, driven by radar hull damage and region dampening, and the
    /// invariant this test pins survives with it.) Blasters were never scaled, which is what
    /// made the two families disagree about the same question.
    ///
    /// The entity here carries a live `ShipModifiers` with the slot crushed to
    /// ×0.667 and, in the second half, doubled. The published reach must not
    /// move in either direction: the phaser bank reaches 200 and the blaster 320
    /// because that is what they author.
    ///
    /// **Both numbers are asserted, per bank, and that is load-bearing.** The
    /// scalar `direct_fire_range` is a MAX, and the bug this targets only ever
    /// scaled the phaser arm — so on the ×0.667 leg the old coupled code returned
    /// 320 too (`200 × 0.667 = 133` loses to the unscaled blaster) and a
    /// max-only assertion passed against the very bug it names. The per-bank
    /// sectors on `AiWorldEntity::weapon_arcs` are the same
    /// `entity_direct_fire_banks` list unreduced, so checking them makes the
    /// crushed leg discriminate as sharply as the doubled one.
    #[test]
    fn direct_fire_reach_ignores_the_radar_range_slot() {
        use crate::messages::ModifierSlot;
        use crate::modifiers::cache::ModifierSource;
        use crate::modifiers::{Modifier, ShipModifiers};

        // Written under a radar-DAMAGE source since issue #952 retired the
        // `sensors` power group: the slot's producers are now hull damage and
        // region dampening, and this fixture has to move it the way something
        // real still does. The reach question is unchanged — nothing about how
        // far a gun shoots may depend on this slot, whoever wrote it.
        let radar_slot_at = |bonus: f32| -> ShipModifiers {
            let mut mods = ShipModifiers::new();
            mods.add_or_update(Modifier {
                source: ModifierSource::SystemDamage(
                    crate::system_registry::tactical_radar_system_id(),
                ),
                slot: ModifierSlot::RadarRange,
                bonus,
            });
            mods
        };

        for (bonus, label) in [(-0.5f32, "radar crushed"), (1.0, "radar doubled")] {
            let mut app = snapshot_test_app();
            let (phasers, blasters) = armed_hull_components();
            let mods = radar_slot_at(bonus);
            let mult = mods.get(&ModifierSlot::RadarRange);
            assert!(
                (mult - 1.0).abs() > 0.1,
                "{label}: the fixture must actually move the slot or this proves \
                 nothing (got x{mult})"
            );
            app.world_mut().spawn((
                crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                phasers,
                blasters,
                mods,
            ));

            app.update();

            let published = &app.world().resource::<WorldSnapshot>().entities[0];
            assert_eq!(
                published.direct_fire_range, 320.0,
                "{label} (RadarRange x{mult:.3}): the published reach must be the \
                 authored longest bank. A ring sized off this fact has to describe \
                 where the target's guns actually stop, and since #955 that is the \
                 number its file authors — nothing scales it"
            );

            // The phaser's own 200, which the max above can never see: it is the
            // arm the retired coupling scaled, and the only reading that fails on
            // the crushed leg if the multiplication comes back.
            let mut per_bank: Vec<f32> = published.weapon_arcs.iter().map(|s| s.range).collect();
            per_bank.sort_by(|a, b| a.partial_cmp(b).expect("no NaN range"));
            assert_eq!(
                per_bank,
                vec![200.0f32, 320.0],
                "{label} (RadarRange x{mult:.3}): the PER-BANK reaches must be the two \
                 authored numbers. 200 scaled by this slot is {:.1}, which the scalar \
                 `direct_fire_range` above hides behind the unscaled blaster's 320 — so \
                 this is where a phaser arm quietly scaled again shows up",
                200.0 * mult
            );
        }
    }

    /// An offline bank is not a threat, so it must not inflate the ring another
    /// ship keeps. With the blaster shot out, the reach falls back to the
    /// phaser's; with both gone it is zero.
    #[test]
    fn an_offline_bank_stops_counting_toward_direct_fire_reach() {
        let mut app = snapshot_test_app();
        let (phasers, blasters) = armed_hull_components();
        let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
        sources.0.set_offline(
            crate::system_registry::blaster_bank_system_id("lance").unwrap(),
            true,
        );
        let entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                phasers,
                blasters,
                sources,
            ))
            .id();

        app.update();
        assert_eq!(
            app.world().resource::<WorldSnapshot>().entities[0].direct_fire_range,
            200.0,
            "a disabled blaster bank must drop out of the reach"
        );

        app.world_mut()
            .entity_mut(entity)
            .get_mut::<crate::ship_plugin::ShipSystemControlSources>()
            .unwrap()
            .0
            .set_offline(
                crate::system_registry::phaser_bank_system_id("fore").unwrap(),
                true,
            );
        app.update();
        assert_eq!(
            app.world().resource::<WorldSnapshot>().entities[0].direct_fire_range,
            0.0,
            "a fully disarmed ship has no reach at all — the ring collapses to the \
             standing-off hull's own authored margin"
        );
    }

    // ── build_world_snapshot: hostile weapon-arc sectors (issue #874) ──────

    /// A ship heading `yaw` radians, stated the way the SIMULATION states it
    /// (issue #937).
    ///
    /// `ShipPhysics.yaw` is the authority for anything that moves, and its
    /// convention is clockwise (0 = facing −Z, so a heading θ points along
    /// `(sin θ, −cos θ)` — the convention `arc_geometry::world_bearing_deg`
    /// resolves bearings in). The `Transform` a ship carries is the RENDER pose
    /// and holds the negation, because `sync_ship_position` writes
    /// `Quat::from_euler(YXZ, -physics.yaw, …)` and Bevy's Y euler turns the
    /// other way.
    ///
    /// The fixtures below used to hand-roll the Transform alone and assert on
    /// what came out, which pinned the render convention onto a field every
    /// consumer reads in the simulation one. Building both here, from one yaw
    /// and through the same negation the real sync applies, is what makes these
    /// tests a statement about a ship rather than about a quaternion.
    fn hull_pose(yaw: f32) -> (Transform, crate::ship_state::ShipPhysics) {
        (
            Transform::from_xyz(0.0, 0.0, 0.0).with_rotation(Quat::from_rotation_y(-yaw)),
            crate::ship_state::ShipPhysics {
                yaw,
                ..Default::default()
            },
        )
    }

    /// **The snapshot's `yaw` is the SIMULATION's heading, not the render
    /// pose's (issue #937).**
    ///
    /// Every consumer of `AiWorldEntity::yaw` reconstructs a forward vector as
    /// `(sin θ, −cos θ)` — `ai::core::target_relative_motion` for both the helm's
    /// `closing_rate` and the captain's hostile range, both avoidance
    /// projections in `ai::core`, and `arc_geometry::weapon_arc_sectors`, whose
    /// output is compared against `world_bearing_deg`'s `atan2(dx, −dz)`. That
    /// is `ShipPhysics.yaw`'s convention and nothing else's.
    ///
    /// The render `Transform` holds the NEGATION of it, because
    /// `sync_ship_position` writes `Quat::from_euler(YXZ, −physics.yaw, …)` and
    /// Bevy's Y euler turns the other way. Reading the euler straight back — as
    /// this producer did — published every ship's heading mirrored, which is a
    /// silent failure: the field is still present, still finite, still moves
    /// when the ship turns, and every test that built its fixture from a
    /// hand-rolled quaternion still agreed with it.
    ///
    /// So this pin is deliberately NOT "the number equals the number". It runs
    /// the ship through the REAL `sync_ship_position`, then asserts on the
    /// FORWARD VECTOR the snapshot implies — against the direction the physics
    /// integrator would actually carry the hull. A sign flip cannot survive
    /// that, and neither can a future change of either convention that forgets
    /// the other.
    ///
    /// What it cost in play is
    /// `headless_runner::the_composed_destroyer_passes_breaks_off_and_passes_again`:
    /// a mirrored target velocity made `closing_rate` read "still closing" for a
    /// destroyer that had already flown past its target, so the attack pass's
    /// closest-approach detector never fired and the hull ground along at
    /// contact range instead of breaking off.
    #[test]
    fn the_snapshot_publishes_headings_in_the_simulations_own_convention() {
        // Four headings rather than one: a sign flip is invisible at 0 and at
        // pi, which are exactly the two a single-case fixture reaches for.
        for yaw in [0.7_f32, -1.9, 2.6, std::f32::consts::FRAC_PI_2] {
            let mut app = snapshot_test_app();
            app.add_systems(
                Update,
                crate::ship::physics_systems::sync_ship_position.before(build_world_snapshot),
            );
            app.world_mut().spawn((
                crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
                Transform::default(),
                crate::ship_state::ShipPhysics {
                    yaw,
                    forward_speed: 10.0,
                    ..Default::default()
                },
            ));
            app.update();

            let e = &app.world().resource::<WorldSnapshot>().entities[0];
            let published = e.yaw.expect("a ship publishes a heading");
            // The direction the integrator carries the hull, straight off
            // `ShipPhysics` (see `ship_physics::compute_physics`).
            let (truth_x, truth_z) = (crate::simmath::sin(yaw), -crate::simmath::cos(yaw));
            // The direction every consumer reconstructs from the published
            // heading.
            let (read_x, read_z) = (
                crate::simmath::sin(published),
                -crate::simmath::cos(published),
            );
            assert!(
                (read_x - truth_x).abs() < 1e-4 && (read_z - truth_z).abs() < 1e-4,
                "yaw {yaw}: the snapshot published {published}, which reads as \
                 forward ({read_x:.3}, {read_z:.3}) — the hull actually travels \
                 ({truth_x:.3}, {truth_z:.3}). A mirrored heading makes every \
                 relative-velocity and weapon-arc reading in the AI wrong \
                 without making any of them absent."
            );
        }
    }

    /// AC2: the arcs are published for every armed entity, with no scan gate
    /// and no target involved — a hull's arcs are a fact about the hull.
    #[test]
    fn world_snapshot_publishes_world_bearing_weapon_arc_sectors() {
        let mut app = snapshot_test_app();
        let (phasers, blasters) = armed_hull_components();
        // Yawed 90 degrees to starboard, so a forward bank bears on +X.
        let (transform, physics) = hull_pose(std::f32::consts::FRAC_PI_2);
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            transform,
            physics,
            phasers,
            blasters,
        ));

        app.update();

        let arcs = &app.world().resource::<WorldSnapshot>().entities[0].weapon_arcs;
        assert_eq!(arcs.len(), 2, "one sector per direct-fire bank: {arcs:?}");
        for a in arcs {
            assert!(
                (a.bearing_deg - 90.0).abs() < 1e-3,
                "yaw 90 + facing 0 must bear 90: {a:?}"
            );
        }
        assert!((arcs[0].half_angle_deg - 45.0).abs() < 1e-3, "{arcs:?}");
        assert!((arcs[0].range - 200.0).abs() < 1e-3, "phaser reach");
        assert!((arcs[1].range - 320.0).abs() < 1e-3, "blaster reach");
    }

    /// An offline bank is not a threat: it drops out of the sectors exactly as
    /// it drops out of the reach, because both are projections of one list.
    #[test]
    fn an_offline_bank_stops_publishing_its_arc_sector() {
        let mut app = snapshot_test_app();
        let (phasers, blasters) = armed_hull_components();
        let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
        sources.0.set_offline(
            crate::system_registry::blaster_bank_system_id("lance").unwrap(),
            true,
        );
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            phasers,
            blasters,
            sources,
        ));

        app.update();

        let arcs = &app.world().resource::<WorldSnapshot>().entities[0].weapon_arcs;
        assert_eq!(arcs.len(), 1, "the disabled blaster arc must go: {arcs:?}");
        assert!((arcs[0].range - 200.0).abs() < 1e-3, "the phaser remains");
    }

    #[test]
    fn an_unarmed_entity_and_an_asteroid_publish_no_arc_sectors() {
        let mut app = snapshot_test_app();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(30.0, 0.0, -12.0),
            crate::entity_spawner::ColliderSection(crate::entity_config::ColliderConfig {
                shape: crate::entity_config::ColliderShape::Ball,
                radius: 4.0,
                length: 0.0,
                movable: false,
            }),
        ));
        app.update();
        for e in &app.world().resource::<WorldSnapshot>().entities {
            assert!(e.weapon_arcs.is_empty(), "{e:?}");
        }
    }

    /// AC4, Rust half: the AI fact reduction and the wire payload derive from
    /// the SAME producer call.
    ///
    /// Both consumers are exercised against one `build_world_snapshot` run:
    /// the wire conversion the helm blackboard performs, and
    /// `crate::ai::hostile_arc_exposure`, the reduction the helm facts are
    /// seeded from. The assertion is elementwise identity — not "both look
    /// plausible" — so a future change that gave either side its own geometry
    /// would fail here rather than drift silently.
    #[test]
    fn the_wire_payload_and_the_ai_fact_reduction_read_the_same_sectors() {
        let mut app = snapshot_test_app();
        let (phasers, blasters) = armed_hull_components();
        let hostile_faction = uuid::Uuid::new_v4();
        let own_faction = uuid::Uuid::new_v4();
        let (transform, physics) = hull_pose(std::f32::consts::FRAC_PI_2);
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            transform,
            physics,
            crate::entity_spawner::FactionComponent(hostile_faction),
            phasers,
            blasters,
        ));
        app.update();

        let snapshot_entity = app.world().resource::<WorldSnapshot>().entities[0].clone();
        assert!(!snapshot_entity.weapon_arcs.is_empty());

        // (a) The wire payload the helm blackboard builds.
        let wire: Vec<crate::messages::HostileWeaponArc> =
            snapshot_entity.weapon_arcs.iter().map(Into::into).collect();

        // (b) The reduction the helm facts are seeded from, over the same
        //     snapshot entry. Observer 100 units to +X — inside the yawed
        //     hull's forward arcs.
        let mut registry = crate::faction::FactionRegistry::new();
        registry.insert(crate::faction::FactionConfig {
            display_name: None,
            uuid: own_faction,
            name: "Own".into(),
            enemies: vec![hostile_faction],
            compliance: None,
        });
        let view = crate::ai::WorldView {
            entity_pos: [100.0, 0.0, 0.0],
            self_faction: Some(own_faction),
            entities: vec![snapshot_entity.clone()],
            ..Default::default()
        };
        let exposure = crate::ai::hostile_arc_exposure(&view, &registry);

        // Same sectors, elementwise: the wire is a verbatim copy.
        assert_eq!(wire.len(), snapshot_entity.weapon_arcs.len());
        for (w, s) in wire.iter().zip(snapshot_entity.weapon_arcs.iter()) {
            assert_eq!(w.bearing_deg, s.bearing_deg);
            assert_eq!(w.half_angle_deg, s.half_angle_deg);
            assert_eq!(w.range, s.range);
        }
        // And the reduction is a reduction of exactly those sectors: rebuilding
        // it from the WIRE arcs reproduces the fact the policy reads.
        let from_wire = crate::weapons::arc_geometry::arc_exposure(
            &wire
                .iter()
                .map(|w| crate::weapons::arc_geometry::WeaponArcSector {
                    bearing_deg: w.bearing_deg,
                    half_angle_deg: w.half_angle_deg,
                    range: w.range,
                })
                .collect::<Vec<_>>(),
            snapshot_entity.position[0],
            snapshot_entity.position[2],
            100.0,
            0.0,
        );
        assert_eq!(from_wire, exposure);
        assert_eq!(exposure.covering_count, 2, "both banks bear: {exposure:?}");
    }

    /// A friendly ship's arcs are published on the snapshot (they are hull
    /// facts) but must not read as exposure — the reduction is hostility-gated.
    #[test]
    fn a_friendly_ships_arcs_are_not_exposure() {
        let same_faction = uuid::Uuid::new_v4();
        let mut app = snapshot_test_app();
        let (phasers, blasters) = armed_hull_components();
        let (transform, physics) = hull_pose(std::f32::consts::FRAC_PI_2);
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            transform,
            physics,
            crate::entity_spawner::FactionComponent(same_faction),
            phasers,
            blasters,
        ));
        app.update();
        let entity = app.world().resource::<WorldSnapshot>().entities[0].clone();
        assert!(!entity.weapon_arcs.is_empty(), "arcs are still published");

        let mut registry = crate::faction::FactionRegistry::new();
        registry.insert(crate::faction::FactionConfig {
            display_name: None,
            uuid: same_faction,
            name: "Own".into(),
            enemies: vec![],
            compliance: None,
        });
        let view = crate::ai::WorldView {
            entity_pos: [100.0, 0.0, 0.0],
            self_faction: Some(same_faction),
            entities: vec![entity],
            ..Default::default()
        };
        assert_eq!(
            crate::ai::hostile_arc_exposure(&view, &registry).covering_count,
            0
        );
    }

    /// An unarmed entity — and an asteroid — publish no reach, rather than a
    /// default one.
    #[test]
    fn an_unarmed_entity_publishes_no_direct_fire_reach() {
        let mut app = snapshot_test_app();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));
        app.update();
        assert_eq!(
            app.world().resource::<WorldSnapshot>().entities[0].direct_fire_range,
            0.0
        );
    }

    #[test]
    fn world_snapshot_carries_both_entities_and_asteroids() {
        let mut app = snapshot_test_app();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            crate::entity_spawner::ColliderSection(crate::entity_config::ColliderConfig {
                shape: crate::entity_config::ColliderShape::Ball,
                radius: 2.5,
                length: 0.0,
                movable: false,
            }),
        ));

        app.update();

        let snapshot = app.world().resource::<WorldSnapshot>();
        assert_eq!(snapshot.entities.len(), 2);
        assert!(
            snapshot.entities.iter().any(|e| e.radius == 2.5),
            "the asteroid pass must not replace the entity pass"
        );
    }

    /// Issue #958: the `movable` fact the hazard rule keys on is the entity's
    /// AUTHORED `[collider] movable`, not "everything spawned through
    /// `spawn_entity` is a ship". A station and a planet come through the same
    /// `EntityUuid` query a hull does; before this, all three published
    /// `movable: true`, which put static terrain on the ignorable side of the
    /// ignore-smaller rule.
    #[test]
    fn world_snapshot_publishes_authored_mobility_not_the_query_arm() {
        let mut app = snapshot_test_app();
        let ball = |radius: f32, movable: bool| {
            crate::entity_spawner::ColliderSection(crate::entity_config::ColliderConfig {
                shape: crate::entity_config::ColliderShape::Ball,
                radius,
                length: 0.0,
                movable,
            })
        };
        // A hull: authored mobile.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            crate::entity_spawner::EntityName("hull".into()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            ball(5.0, true),
        ));
        // A station: same query arm, authored static.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            crate::entity_spawner::EntityName("station".into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            ball(12.0, false),
        ));
        // An entity with no collider at all falls back to the same static
        // default the TOML parser uses.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            crate::entity_spawner::EntityName("bare".into()),
            Transform::from_xyz(200.0, 0.0, 0.0),
        ));

        app.update();

        let snapshot = app.world().resource::<WorldSnapshot>();
        let movable_of = |name: &str| {
            snapshot
                .entities
                .iter()
                .find(|e| e.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name} must be in the snapshot"))
                .movable
        };
        assert!(movable_of("hull"), "an authored hull is a mobile contact");
        assert!(
            !movable_of("station"),
            "a station must publish as static terrain, not as a ship"
        );
        assert!(
            !movable_of("bare"),
            "an entity with no [collider] must not claim mobility"
        );
    }

    // ── AiTokenRegistry unit tests ─────────────────────────────────────────

    #[test]
    fn register_produces_ai_prefixed_token() {
        let mut reg = AiTokenRegistry::new();
        reg.register("abc-123");
        assert_eq!(reg.token_for_entity("abc-123"), Some("ai:abc-123"));
    }

    #[test]
    fn register_is_idempotent() {
        let mut reg = AiTokenRegistry::new();
        reg.register("abc-123");
        reg.register("abc-123");
        assert_eq!(reg.token_for_entity("abc-123"), Some("ai:abc-123"));
        // Exactly one reverse entry
        assert_eq!(reg.entity_uuid_for_token("ai:abc-123"), Some("abc-123"));
    }

    #[test]
    fn entity_uuid_for_token_returns_none_for_player_token() {
        let reg = AiTokenRegistry::new();
        assert!(reg.entity_uuid_for_token("some-player-uuid").is_none());
    }

    #[test]
    fn entity_uuid_for_token_round_trips() {
        let mut reg = AiTokenRegistry::new();
        reg.register("ent-999");
        assert_eq!(reg.entity_uuid_for_token("ai:ent-999"), Some("ent-999"));
    }

    #[test]
    fn unregister_removes_both_directions() {
        let mut reg = AiTokenRegistry::new();
        reg.register("ent-1");
        reg.unregister("ent-1");
        assert!(reg.token_for_entity("ent-1").is_none());
        assert!(reg.entity_uuid_for_token("ai:ent-1").is_none());
    }

    #[test]
    fn unregister_unknown_entity_is_silent() {
        let mut reg = AiTokenRegistry::new();
        reg.unregister("ghost-uuid"); // must not panic
    }

    #[test]
    fn contains_entity_returns_true_after_register() {
        let mut reg = AiTokenRegistry::new();
        reg.register("ent-x");
        assert!(reg.contains_entity("ent-x"));
    }

    #[test]
    fn contains_entity_returns_false_after_unregister() {
        let mut reg = AiTokenRegistry::new();
        reg.register("ent-x");
        reg.unregister("ent-x");
        assert!(!reg.contains_entity("ent-x"));
    }

    #[test]
    fn multiple_entities_registered_independently() {
        let mut reg = AiTokenRegistry::new();
        reg.register("alpha");
        reg.register("beta");
        reg.register("gamma");
        assert_eq!(reg.token_for_entity("alpha"), Some("ai:alpha"));
        assert_eq!(reg.token_for_entity("beta"), Some("ai:beta"));
        assert_eq!(reg.token_for_entity("gamma"), Some("ai:gamma"));
    }

    #[test]
    fn unregistering_one_does_not_affect_others() {
        let mut reg = AiTokenRegistry::new();
        reg.register("alpha");
        reg.register("beta");
        reg.unregister("alpha");
        assert!(reg.token_for_entity("alpha").is_none());
        assert_eq!(reg.token_for_entity("beta"), Some("ai:beta"));
    }

    // ── Bevy integration tests ─────────────────────────────────────────────

    use crate::config_cache::FactionRegistryResource;
    use crate::entity_config::BehaviourConfig;
    use crate::entity_spawner::EntityUuid;
    use crate::lobby::LobbyPlugin;
    use crate::messages::GamePhase;

    #[derive(Resource, Default)]
    struct AttackedBox(Vec<AiEntityAttacked>);
    #[derive(Resource, Default)]
    struct DestroyedBox(Vec<AiEntityDestroyed>);

    fn collect_attacked(mut r: MessageReader<AiEntityAttacked>, mut b: ResMut<AttackedBox>) {
        for e in r.read() {
            b.0.push(e.clone());
        }
    }
    fn collect_destroyed(mut r: MessageReader<AiEntityDestroyed>, mut b: ResMut<DestroyedBox>) {
        for e in r.read() {
            b.0.push(e.clone());
        }
    }

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(AiPlugin)
            .insert_resource(FactionRegistryResource(
                crate::config_cache::get_faction_registry(),
            ))
            .init_resource::<AttackedBox>()
            .init_resource::<DestroyedBox>()
            .add_systems(PostUpdate, (collect_attacked, collect_destroyed));
        // One fixed step per update (issue #895): AiPlugin's systems run on
        // the logical tick, and each harness tick advances it once.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(200),
        );
        app
    }

    fn spawn_behaviour_entity(app: &mut App, uuid: &str) -> Entity {
        app.world_mut()
            .spawn((
                Transform::from_xyz(1.0, 0.0, 2.0),
                EntityUuid(uuid.to_string()),
                BehaviourSection(BehaviourConfig::default()),
            ))
            .id()
    }

    #[test]
    fn token_registered_after_spawn() {
        let mut app = build_test_app();
        spawn_behaviour_entity(&mut app, "ent-003");
        app.update();
        let reg = app.world().resource::<AiTokenRegistry>();
        assert!(reg.contains_entity("ent-003"), "entity must be registered");
        assert_eq!(reg.token_for_entity("ent-003"), Some("ai:ent-003"));
    }

    #[test]
    fn token_unregistered_after_entity_despawn() {
        let mut app = build_test_app();
        let entity = spawn_behaviour_entity(&mut app, "ent-007");
        app.update();
        // Verify registered
        assert!(app
            .world()
            .resource::<AiTokenRegistry>()
            .contains_entity("ent-007"));
        // Despawn
        app.world_mut().despawn(entity);
        app.update();
        assert!(
            !app.world()
                .resource::<AiTokenRegistry>()
                .contains_entity("ent-007"),
            "token must be unregistered after despawn"
        );
    }

    // ── AiEntityAttacked event ─────────────────────────────────────────────
    //
    // Post-#702 the rising edge lives on `LastShipAttacker`'s change detection
    // rather than on a private `AiMemory.last_attacker` mirror, so these drive
    // that component. They pin the *reader's* half of the exactly-once
    // contract: given a writer that compares before writing, the emitter fires
    // once per new attacker. The writer's half — that `tick_beams` really does
    // compare rather than blind-write under a live beam — is pinned by
    // `sustained_beam_marks_last_attacker_changed_exactly_once` in
    // `console::weapons`. Both halves are required; neither alone
    // establishes the AC.

    /// Write `LastShipAttacker` the way `tick_beams` does — via `set_if_neq`,
    /// not `insert`. The distinction is the whole point: an `insert` marks the
    /// component changed even when the value is identical, so a fixture that
    /// inserted would fake an edge production never produces and this test
    /// would pass on a blind-writing `tick_beams`.
    fn beam_hit(app: &mut App, entity: Entity, attacker: &str) {
        let mut e = app.world_mut().entity_mut(entity);
        let mut last = e
            .get_mut::<crate::weapons_plugin::LastShipAttacker>()
            .expect("ship must carry LastShipAttacker");
        last.set_if_neq(crate::weapons_plugin::LastShipAttacker(Some(
            attacker.to_string(),
        )));
    }

    fn attacked_count(app: &App, entity_uuid: &str) -> usize {
        app.world()
            .resource::<AttackedBox>()
            .0
            .iter()
            .filter(|e| e.entity_uuid == entity_uuid)
            .count()
    }

    #[test]
    fn ai_entity_attacked_event_emitted_when_new_attacker_arrives() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let attacker_id = "aaaaaaaa-0000-0000-0000-000000000099";
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-attacked-001".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::weapons_plugin::LastShipAttacker::default(),
            ))
            .id();

        app.update(); // attach controller; no attacker yet
        beam_hit(&mut app, entity, attacker_id);
        app.update(); // the change is seen — emits AiEntityAttacked

        let events = app.world().resource::<AttackedBox>().0.clone();
        let event = events
            .iter()
            .find(|e| e.entity_uuid == "ent-attacked-001")
            .expect("AiEntityAttacked must be emitted when a new attacker arrives");
        assert_eq!(
            event.attacker_uuid,
            uuid::Uuid::parse_str(attacker_id).unwrap(),
            "the event must name the attacker LastShipAttacker records"
        );
    }

    /// Sustained fire: the beam keeps naming the same shooter every tick, and
    /// the trigger must fire exactly once.
    #[test]
    fn ai_entity_attacked_not_re_emitted_for_same_attacker() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let attacker_id = "aaaaaaaa-0000-0000-0000-000000000088";
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-attacked-002".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::weapons_plugin::LastShipAttacker::default(),
            ))
            .id();

        app.update(); // attach

        // Five ticks of a live beam from one shooter.
        for _ in 0..5 {
            beam_hit(&mut app, entity, attacker_id);
            app.update();
        }

        assert_eq!(
            attacked_count(&app, "ent-attacked-002"),
            1,
            "sustained fire from one shooter must emit AiEntityAttacked exactly once"
        );
    }

    /// The other edge: a *different* shooter is a new attacker and must re-fire,
    /// even though `LastShipAttacker` was already `Some`. Guards against a fix
    /// for the test above that latches on "was ever attacked" instead of "who".
    #[test]
    fn ai_entity_attacked_re_emitted_for_a_different_attacker() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let first = "aaaaaaaa-0000-0000-0000-000000000077";
        let second = "bbbbbbbb-0000-0000-0000-000000000077";
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-attacked-003".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::weapons_plugin::LastShipAttacker::default(),
            ))
            .id();

        app.update();
        beam_hit(&mut app, entity, first);
        app.update();
        beam_hit(&mut app, entity, second);
        app.update();

        assert_eq!(
            attacked_count(&app, "ent-attacked-003"),
            2,
            "a second, different attacker is a new edge and must re-emit"
        );
    }

    /// Clearing the attacker (`clear_last_attacker_on_death` /
    /// `clear_last_attacker_on_red_alert_off` both write `None`) marks the
    /// component changed, but `None` names nobody and must not be reported as
    /// an attack.
    #[test]
    fn clearing_the_attacker_emits_no_attacked_event() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-attacked-004".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::weapons_plugin::LastShipAttacker::default(),
            ))
            .id();

        app.update();
        beam_hit(&mut app, entity, "aaaaaaaa-0000-0000-0000-000000000066");
        app.update();

        // The threat passes — the attacker record is cleared.
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::weapons_plugin::LastShipAttacker(None));
        app.update();

        assert_eq!(
            attacked_count(&app, "ent-attacked-004"),
            1,
            "clearing LastShipAttacker to None must not count as an attack"
        );
    }

    // ── Issue #314: WorldView population from components ───────────────────

    fn make_weapons_console_config(beam_range: f32) -> crate::entity_config::WeaponsConsoleConfig {
        crate::entity_config::WeaponsConsoleConfig {
            torpedo_arc_color: vec![],
            power_multipliers: None,
            phaser_banks: vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 360.0,
                auto_arc_deg: 360.0,
                beam_range,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 3.0,
                cooldown_secs: 3.0,
                beam_color: vec![],
                shield_pierce: Some(0.0),
                marker: None,
                ai: None,
            }],
            blaster_banks: vec![],
            radar: None,
            selector: None,
            selector_idle: false,
            ai: None,
        }
    }

    #[test]
    fn self_hull_fraction_reflects_entity_console_hull() {
        use crate::damage::SystemHull;
        use crate::entity_spawner::EntitySystemHull;
        use crate::messages::SystemId;

        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        // 50 HP out of 100 HP = 0.5 fraction
        let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);
        let mut rng = crate::sim_rng::unseeded_test_rng();
        hull.apply_damage(50.0, &mut rng);

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-hull-frac-001".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                EntitySystemHull(hull),
            ))
            .id();

        app.update(); // attach controller
        app.update(); // tick

        // The hull fraction should be ~0.5; we verify via the world_view that was
        // used internally by confirming the EntitySystemHull component is readable.
        let hull_comp = app.world().get::<EntitySystemHull>(entity).unwrap();
        let frac = hull_comp.0.total_current() / hull_comp.0.total_max();
        assert!(
            (frac - 0.5).abs() < 0.01,
            "hull fraction should be ~0.5, got {frac}"
        );
    }

    #[test]
    fn npc_beam_ready_true_when_active_beam_inactive_and_no_cooldown() {
        use crate::entity_spawner::WeaponsConsoleSection;
        use crate::weapons_plugin::{ActiveBeam, PhaserCooldown};

        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-phaser-002".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                WeaponsConsoleSection(make_weapons_console_config(40.0)),
                ActiveBeam::default(),
                PhaserCooldown::default(),
            ))
            .id();

        app.update(); // attach controller + first tick
        app.update(); // second tick runs the world_view logic

        let beam = app.world().get::<ActiveBeam>(entity).unwrap();
        let cd = app.world().get::<PhaserCooldown>(entity).unwrap();
        assert!(!beam.is_firing(), "beam must not be active");
        assert!(!cd.is_bank_active("fore"), "cooldown must be 0");
    }

    #[test]
    fn npc_beam_ready_false_when_cooldown_active() {
        use crate::entity_spawner::WeaponsConsoleSection;
        use crate::weapons_plugin::{ActiveBeam, PhaserCooldown};

        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let mut cooldown = PhaserCooldown::default();
        cooldown.start_bank("fore", 5.0);

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-phaser-003".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                WeaponsConsoleSection(make_weapons_console_config(40.0)),
                ActiveBeam::default(),
                cooldown,
            ))
            .id();

        app.update();

        let cd = app.world().get::<PhaserCooldown>(entity).unwrap();
        assert!(
            cd.is_bank_active("fore"),
            "phaser must not be ready when bank cooldown is active"
        );
    }

    #[test]
    fn weapons_console_section_attached_when_config_has_weapons_console() {
        use crate::entity_config::EntityConfig;
        use crate::entity_spawner::WeaponsConsoleSection;

        let mut app = build_test_app();

        // Build a minimal EntityConfig with a weapons_console section.
        let config = EntityConfig {
            name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            infrastructure: None,
            operations: None,
            scan: None,
            civilian: None,
            faction: None,
            hull: None,
            weapons_console: Some(make_weapons_console_config(80.0)),
            behaviour: None,
            helm_console: None,
            helm_capability: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            collider: None,
            appearance: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            tags: vec![],
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            radar_appearance: None,
            mesh: None,
            target: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
        };

        let mut commands = app.world_mut().commands();
        let entity = crate::entity_spawner::spawn_entity(
            &mut commands,
            &config,
            bevy::math::Vec3::ZERO,
            "ent-spawner-weapons-001".to_string(),
            None,
        );
        app.world_mut().flush();

        let wc = app.world().get::<WeaponsConsoleSection>(entity);
        assert!(
            wc.is_some(),
            "WeaponsConsoleSection must be attached when config has weapons_console"
        );
        assert!(
            wc.unwrap()
                .0
                .phaser_banks
                .first()
                .map(|b| (b.beam_range - 80.0).abs() < 0.01)
                .unwrap_or(false),
            "beam_range must match config"
        );
    }

    // ── PRD #307: FactionRegistryResource must be accessible as Res (not Option) ──

    /// A minimal system that takes `Res<FactionRegistryResource>` (non-Option).
    /// If the resource is not present, Bevy panics with a missing-resource error.
    /// This test verifies that `build_test_app` — which calls `insert_faction_registry`
    /// via the unconditional path — makes the resource available on native.
    fn read_faction_registry_system(reg: Res<FactionRegistryResource>) {
        // Just accessing it is enough — the test verifies the resource exists.
        let _ = &reg.0;
    }

    // ── aggregate_doctrine_blackboards ────────────────────────────────────────

    /// `aggregate_doctrine_blackboards` must write a `ViewscreenBlackboard` with
    /// at least one `ScoredObjective` carrying `SystemAffinity::Helm` for an
    /// entity whose `BehaviourSection` contains a `Patrol` doctrine entry.
    /// This is the gate the per-axis helm AI checks (`has_helm_objective`);
    /// without it the ship stays still even when Backfill AI is active.
    #[test]
    fn aggregate_doctrine_blackboards_writes_scored_helm_objective() {
        use crate::damage::SystemHull;
        use crate::entity_config::{BehaviourConfig, DoctrineObjective};
        use crate::entity_spawner::EntitySystemHull;
        use crate::messages::{SystemAffinity, SystemId};
        use crate::server_app::ShipSystemBlackboards;
        use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

        let mut app = build_test_app();

        let behaviour = BehaviourConfig {
            doctrine: vec![DoctrineObjective {
                id: "patrol-test".into(),
                text: "Patrol test route".into(),
                directive_kind: Some("Patrol".into()),
                base_priority: 30.0,
                directive_loop: true,
                ..Default::default()
            }],
            ..Default::default()
        };

        let hull = EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            100.0,
        )]));

        app.world_mut().spawn((
            BehaviourSection(behaviour),
            hull,
            ShipSystemBlackboards::default(),
        ));

        app.update();

        let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
        let bb = q
            .iter(app.world())
            .next()
            .expect("entity must have ShipSystemBlackboards");

        let viewscreen =
            bb.0.get(&crate::messages::SystemId(VIEWSCREEN_SYSTEM_ID.to_string()))
                .expect("viewscreen entry must be present after aggregate_doctrine_blackboards");

        let scored = match viewscreen {
            crate::messages::SystemBlackboard::Viewscreen(v) => &v.scored_objectives,
            _ => panic!("expected Viewscreen blackboard"),
        };

        assert!(
            !scored.is_empty(),
            "scored_objectives must not be empty for a Patrol doctrine entity"
        );
        assert!(
            scored
                .iter()
                .any(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm)),
            "at least one scored objective must carry SystemAffinity::Helm"
        );
    }

    /// A `StaticPointDefence` platform (the station) carries NO `[behaviour]`, so
    /// it used to fall outside `aggregate_doctrine_blackboards`' query and never
    /// got a Viewscreen blackboard — and `ai_phaser_auto_fire` aims at the
    /// Viewscreen `combat_lock`, so the station's Tactical lock was consumed by
    /// nothing and it never fired. It must now get a Viewscreen entry whose
    /// `combat_lock` mirrors its tactical radar's `selected_target`.
    #[test]
    fn aggregate_publishes_combat_lock_for_a_behaviourless_point_defence() {
        use crate::damage::SystemHull;
        use crate::entity_spawner::{EntitySystemHull, StaticPointDefence};
        use crate::messages::{SystemBlackboard, SystemId, TacticalRadarBlackboard};
        use crate::server_app::ShipSystemBlackboards;
        use crate::ship::system_registry::{tactical_radar_system_id, VIEWSCREEN_SYSTEM_ID};

        let mut app = build_test_app();

        let locked = uuid::Uuid::new_v4().to_string();
        let mut blackboards = ShipSystemBlackboards::default();
        blackboards.0.insert(
            tactical_radar_system_id(),
            SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
                selected_target: Some(locked.clone()),
                ..Default::default()
            }),
        );
        let hull = EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            100.0,
        )]));

        // No `BehaviourSection`: a static point-defence platform with no doctrine.
        app.world_mut()
            .spawn((StaticPointDefence, hull, blackboards));

        app.update();

        let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
        let bb = q
            .iter(app.world())
            .next()
            .expect("entity must have ShipSystemBlackboards");
        let viewscreen =
            bb.0.get(&SystemId(VIEWSCREEN_SYSTEM_ID.to_string()))
                .expect("a StaticPointDefence must get a Viewscreen entry with no behaviour");
        let combat_lock = match viewscreen {
            SystemBlackboard::Viewscreen(v) => v.combat_lock.clone(),
            _ => panic!("expected Viewscreen blackboard"),
        };
        assert_eq!(
            combat_lock.as_deref(),
            Some(locked.as_str()),
            "the station's Viewscreen combat_lock must mirror its tactical radar selected_target"
        );
    }

    /// Publish a viewscreen pool for one entity and hand back its
    /// `scored_objectives`.
    fn scored_pool_for(
        behaviour: crate::entity_config::BehaviourConfig,
        hull_current: f32,
        hull_max: f32,
    ) -> Vec<crate::messages::ScoredObjective> {
        use crate::damage::SystemHull;
        use crate::entity_spawner::EntitySystemHull;
        use crate::messages::SystemId;
        use crate::server_app::ShipSystemBlackboards;
        use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

        let mut app = build_test_app();

        let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), hull_max)]);
        hull.set_hp(&SystemId("captain".into()), hull_current);

        app.world_mut().spawn((
            BehaviourSection(behaviour),
            EntitySystemHull(hull),
            ShipSystemBlackboards::default(),
        ));
        app.update();

        let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
        let bb = q.iter(app.world()).next().expect("blackboards").clone();
        match bb
            .0
            .get(&crate::messages::SystemId(VIEWSCREEN_SYSTEM_ID.to_string()))
            .expect("viewscreen entry")
        {
            crate::messages::SystemBlackboard::Viewscreen(v) => v.scored_objectives.clone(),
            _ => panic!("expected Viewscreen blackboard"),
        }
    }

    /// A `Retreat` doctrine entry gated on `hull_below` scores only once the
    /// ship is actually hurt (issue #702).
    ///
    /// This is the replacement for the engine's synthetic hull-triggered
    /// Retreat, which `aggregate_doctrine_blackboards` used to inject below a
    /// `[behaviour] retreat_hull_threshold`. That mechanism was inert in
    /// production and could not have worked: it scored 0..1 against doctrine
    /// priorities in the tens, so it lost every contest even at zero hull. An
    /// authored entry scores on the same scale as everything else, which is the
    /// bug fix hiding inside the deletion.
    #[test]
    fn authored_retreat_outranks_doctrine_only_once_hull_is_low() {
        let healthy = scored_pool_for(retreat_behaviour(0.3), 100.0, 100.0);
        let hurt = scored_pool_for(retreat_behaviour(0.3), 10.0, 100.0);

        let score_of = |pool: &[crate::messages::ScoredObjective], id: &str| {
            pool.iter()
                .find(|o| o.id == id)
                .unwrap_or_else(|| panic!("{id} must be in the pool"))
                .score
        };

        assert_eq!(
            score_of(&healthy, "retreat-when-hurt"),
            0.0,
            "at full hull the `hull_below` zero-gate must veto the Retreat \
             outright, so its high base_priority costs nothing"
        );
        assert!(
            score_of(&healthy, "loiter") > 0.0,
            "precondition: the rival objective must be live at full hull"
        );

        assert!(
            score_of(&hurt, "retreat-when-hurt") > score_of(&hurt, "loiter"),
            "below the gate's threshold the Retreat must outrank ordinary \
             doctrine — the score-scale bug the synthetic Retreat could never \
             clear (0..1 against a base_priority in the tens)"
        );
        assert_eq!(
            hurt[0].id, "retreat-when-hurt",
            "and it must lead the pool, since every consumer takes the FIRST \
             Helm-relevant entry rather than scanning for the maximum"
        );
    }

    /// The retreat threshold is designer-tunable per entity template — two
    /// ships at identical hull must disagree about retreating purely on their
    /// TOML, with no recompile.
    ///
    /// Was `retreat_threshold_comes_from_behaviour_config`, which tuned the
    /// engine's `[behaviour] retreat_hull_threshold`. The authored form is
    /// strictly more expressive: the threshold, the destination anchor, the
    /// urgency and the gate condition are all per-hull now, rather than one
    /// hardwired hull ramp to a fixed place.
    #[test]
    fn retreat_threshold_is_authored_per_entity_template() {
        // Both ships sit at 40% hull; only their authored gate differs.
        let brave = scored_pool_for(retreat_behaviour(0.1), 40.0, 100.0);
        let cautious = scored_pool_for(retreat_behaviour(0.9), 40.0, 100.0);

        let retreat_score = |pool: &[crate::messages::ScoredObjective]| {
            pool.iter()
                .find(|o| o.id == "retreat-when-hurt")
                .expect("retreat must be in the pool")
                .score
        };

        assert_eq!(
            retreat_score(&brave),
            0.0,
            "hull 0.4 is above a 0.1 threshold — a brave ship must not retreat"
        );
        assert!(
            retreat_score(&cautious) > 0.0,
            "hull 0.4 is below a 0.9 threshold — a cautious ship must retreat"
        );
    }

    /// The published pool is sorted descending by score.
    ///
    /// `operate_helm` and `resolve_helm_target_position` both take the FIRST
    /// Helm-relevant entry as the top-scored directive rather than scanning for
    /// the maximum, so a pool that is merely "mostly sorted" silently
    /// mis-selects.
    #[test]
    fn published_pool_is_sorted_by_score_descending() {
        let scored = scored_pool_for(retreat_behaviour(0.5), 10.0, 100.0);

        assert!(scored.len() > 1, "precondition: need a pool to sort");
        for pair in scored.windows(2) {
            assert!(
                pair[0].score >= pair[1].score,
                "pool must stay sorted descending: {:?} ({}) preceded {:?} ({})",
                pair[0].id,
                pair[0].score,
                pair[1].id,
                pair[1].score
            );
        }
    }

    /// A hull carrying an authored `Retreat` gated at `threshold`, plus an
    /// ordinary always-on objective to outrank. Mirrors the shape shipped in
    /// `assets/worlds/patrol.toml`, which authors one on `raider_alpha` (#892 —
    /// it used to ship on the retired `pirate_raider.toml`).
    fn retreat_behaviour(threshold: f32) -> crate::entity_config::BehaviourConfig {
        use crate::entity_config::{BehaviourConfig, DoctrineObjective};
        BehaviourConfig {
            doctrine: vec![
                DoctrineObjective {
                    id: "loiter".into(),
                    text: "Loiter".into(),
                    directive_kind: Some("Patrol".into()),
                    base_priority: 20.0,
                    directive_loop: true,
                    ..Default::default()
                },
                DoctrineObjective {
                    id: "retreat-when-hurt".into(),
                    text: "Hull critical - run for the haven".into(),
                    directive_kind: Some("Retreat".into()),
                    directive_anchor: Some("pirate_haven".into()),
                    base_priority: 100.0,
                    zero_gates: vec![crate::objectives::ZeroGateCondition {
                        condition: "hull_below".into(),
                        threshold: Some(threshold),
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// Issue #936, retained through #1010: `WorldConditions.attacked` never
    /// reports whether the ship merely CARRIES the component that would record
    /// an attacker.
    ///
    /// `entity_spawner` inserts `LastShipAttacker::default()` on every ship it
    /// spawns, so the pre-#936 `last_attacker_opt.is_some()` was a constant
    /// `true` from an NPC's first tick and every `attacked` / `not_attacked`
    /// condition in shipped content was decided before the fight began. That
    /// survived because nothing ever looked: the one test over
    /// `aggregate_doctrine_blackboards` spawned an entity with NO
    /// `LastShipAttacker` at all, which reads `false` on both sides of the fix.
    ///
    /// #1010 then moved the signal off `LastShipAttacker` entirely. That field
    /// is set on the first beam that connects and cleared only on death or on
    /// the red-alert on→off edge, and while the shooting continues that edge
    /// never comes: a Harrow's captain stand-down (`combat_window_secs = 10`)
    /// folds the hull's OWN return fire into `secs_since_combat`, so a Harrow
    /// fighting back keeps its own alert up and the latch stayed set for as
    /// long as anything loitered nearby — the playtest's frozen raid. The
    /// signal is now recency of the last LANDED HIT, which is why the middle
    /// case below — a named attacker on a ship nothing has hit lately — leaves
    /// the gate open where it used to veto.
    ///
    /// The doctrine shape is `combat_test.toml`'s: an assault entry zero-gated
    /// on `not_attacked`, which is what "commit to the raid unless something is
    /// shooting at you" is authored as, and which scored 0 on every wave for
    /// the whole life of the #936 bug.
    #[test]
    fn a_default_last_attacker_component_does_not_read_as_attacked() {
        let score_of = |pool: &[crate::messages::ScoredObjective], id: &str| {
            pool.iter()
                .find(|o| o.id == id)
                .unwrap_or_else(|| panic!("{id} must be in the pool"))
                .score
        };

        let untouched = scored_pool_for_attacker(assault_behaviour(), None, Activity::default());
        assert!(
            score_of(&untouched, "assault-starbase") > 0.0,
            "a ship carrying `LastShipAttacker::default()` has NOT been \
             attacked, so a `not_attacked` zero-gate must stay open. Reading \
             the component's presence instead of its contents vetoed this \
             entry on every wave of every run: {untouched:?}"
        );

        let stale_attacker = scored_pool_for_attacker(
            assault_behaviour(),
            Some("attacker-uuid"),
            Activity::default(),
        );
        assert!(
            score_of(&stale_attacker, "assault-starbase") > 0.0,
            "the attacker uuid is a LATCH — set on the first hit, cleared only \
             on death or on the red-alert on→off edge, which a ship still \
             returning fire never reaches. A ship carrying one but taking no \
             hits is not under attack, so the raid must stay live: \
             {stale_attacker:?}"
        );

        let under_fire = scored_pool_for_attacker(
            assault_behaviour(),
            Some("attacker-uuid"),
            Activity {
                last_damage_taken: Some(0.0),
                ..Activity::default()
            },
        );
        assert_eq!(
            score_of(&under_fire, "assault-starbase"),
            0.0,
            "with a hit inside the memory window the `not_attacked` gate must \
             veto the assault so the hull turns and fights: {under_fire:?}"
        );
        assert!(
            score_of(&under_fire, "destroy-hostiles") > 0.0,
            "precondition: the rival entry the veto hands the fight to must be \
             live: {under_fire:?}"
        );
    }

    /// Fire an arc ABSORBS is still being attacked (issue #1010 review).
    ///
    /// `last_damage_taken` moves only when the hull total drops, so shield-eaten
    /// fire lands in `last_hostile_fire_taken` instead. This is not a corner
    /// case: `station_axiom.toml` shoots 5 dps in 4 s bursts with no
    /// `shield_pierce` at a Harrow's single 90 hp arc regenerating 2/s — the
    /// burst (20 dmg) barely outpaces the regen over the same cycle (16 dmg
    /// over 8 s), netting only ~4 hp/cycle, so shield-absorbed fire dominates
    /// the engagement and hull damage does not land until sustained pressure
    /// collapses the arc (roughly three minutes of continuous fire). A gate
    /// reading hull damage alone would leave a Harrow under sustained station
    /// fire flying its raid into the guns for most of a short engagement —
    /// exactly the AC ("a Harrow attacked by the station switches to
    /// self-defence") the old per-beam `LastShipAttacker` signal did satisfy,
    /// since `tick_beams` marks the target on every hit regardless of damage.
    #[test]
    fn shield_absorbed_hostile_fire_closes_the_assault_gate() {
        let score_of = |pool: &[crate::messages::ScoredObjective], id: &str| {
            pool.iter()
                .find(|o| o.id == id)
                .unwrap_or_else(|| panic!("{id} must be in the pool"))
                .score
        };

        let shields_holding = scored_pool_for_attacker(
            assault_behaviour(),
            Some("attacker-uuid"),
            Activity {
                last_hostile_fire_taken: Some(0.0),
                ..Activity::default()
            },
        );
        assert_eq!(
            score_of(&shields_holding, "assault-starbase"),
            0.0,
            "a hit the shields ate is still a hit — the raid must break off \
             even though the hull total never moved: {shields_holding:?}"
        );
    }

    /// Firing your own guns is NOT being attacked (issue #1010 review).
    ///
    /// `last_weapon_fired` is the third reading on `RecentCombatActivity` and
    /// the captain's `secs_since_combat` red-alert fact folds it in — which is
    /// precisely why the alert never stood down mid-fight. Folding it here too
    /// would make a `not_attacked` gate veto itself the instant the hull opened
    /// fire and hold the veto for as long as it kept firing, reinstating the
    /// permanent break-off #1010 exists to remove.
    #[test]
    fn a_ships_own_weapon_fire_does_not_read_as_being_attacked() {
        let score_of = |pool: &[crate::messages::ScoredObjective], id: &str| {
            pool.iter()
                .find(|o| o.id == id)
                .unwrap_or_else(|| panic!("{id} must be in the pool"))
                .score
        };

        let shooting = scored_pool_for_attacker(
            assault_behaviour(),
            Some("attacker-uuid"),
            Activity {
                last_weapon_fired: Some(0.0),
                ..Activity::default()
            },
        );
        assert!(
            score_of(&shooting, "assault-starbase") > 0.0,
            "a raider pressing its own attack has not been attacked — the \
             `not_attacked` gate must stay open: {shooting:?}"
        );
    }

    /// Issue #1010: the `attacked` window DECAYS, so the raid a hit interrupts
    /// comes back once the shooting stops — and it decays over the window the
    /// WORLD AUTHORS, not just over the serde default.
    ///
    /// This is the behaviour `combat_test.toml`'s `assault-starbase` is
    /// authored for and never got: a hit closes the `not_attacked` gate (the
    /// base `destroy_hostiles` arm takes the fight), and a reprieve of
    /// `[global] attacked_memory_secs` with no further hits reopens it. Under
    /// the old `LastShipAttacker` latch the reopen needed a red-alert
    /// stand-down that a ship still returning fire never reached, so the raid
    /// stayed retired for as long as anything loitered nearby.
    ///
    /// Driven over the real fixed clock rather than by poking a boolean, so the
    /// decay is measured in the sim seconds the window is authored in
    /// (AGENTS.md #7) — nothing here reads a wall clock.
    ///
    /// The world here AUTHORS a short window (#889's lesson: a fixture with no
    /// `WorldConfig` exercises only the serde-default fallback arm, so a
    /// typo'd field path in the resolution block would pass unnoticed). Two
    /// seconds is well clear of the 8 s default, so the gate reopening on
    /// schedule can only be the authored value being read.
    #[test]
    fn the_assault_resumes_once_the_authored_attacked_window_elapses() {
        use crate::console::weapons::beam::LastShipAttacker;
        use crate::damage::SystemHull;
        use crate::entity_spawner::EntitySystemHull;
        use crate::messages::SystemId;
        use crate::server_app::ShipSystemBlackboards;

        let window = 2.0_f32;
        let default_window = crate::entity_config::GlobalConfig::default().attacked_memory_secs;
        assert!(
            window < default_window,
            "the authored window must differ from the {default_window}s serde \
             default, or this test cannot tell the two arms apart"
        );

        let mut app = build_test_app();
        // The three clock keys are authored 1:1 so the AI cadence stays one
        // snapshot per fixed step, which is what `build_test_app` is paced for
        // — a `WorldConfig` arms `ai::cadence`'s latches from the authored
        // ratios instead of every step, and the shipped 60/30/10 defaults would
        // put the aggregator on a six-step cadence this fixture was not written
        // against. `sim_tick_hz` sits on `MIN_SIM_TICK_HZ`, the floor
        // `parse_world` enforces.
        app.insert_resource(
            crate::world::config::parse_world(&format!(
                "[global]\n\
                 sim_tick_hz = 30\n\
                 ai_tick_hz = 30\n\
                 ai_snapshot_hz = 30\n\
                 attacked_memory_secs = {window}\n"
            ))
            .expect("world TOML should parse"),
        );
        let ship = app
            .world_mut()
            .spawn((
                BehaviourSection(assault_behaviour()),
                EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    100.0,
                )])),
                ShipSystemBlackboards::default(),
                LastShipAttacker(Some("attacker-uuid".into())),
                Activity::default(),
            ))
            .id();

        app.update();
        assert!(
            published_score(&app, ship, "assault-starbase") > 0.0,
            "precondition: an undamaged raider flies the assault"
        );

        // A hit lands on this tick.
        let hit_at = sim_secs(&app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<Activity>()
            .expect("combat activity")
            .last_damage_taken = Some(hit_at);
        app.update();
        assert_eq!(
            published_score(&app, ship, "assault-starbase"),
            0.0,
            "a fresh hit must close the `not_attacked` gate"
        );
        assert!(
            published_score(&app, ship, "destroy-hostiles") > 0.0,
            "self-defence is what the veto hands the fight to"
        );

        // Halfway through the reprieve, with no further hits: still broken off.
        // A window that expired early would let the raider turn its back on a
        // ship that is still shooting it.
        drive_to_sim_secs(&mut app, hit_at + window * 0.5);
        assert_eq!(
            published_score(&app, ship, "assault-starbase"),
            0.0,
            "the gate must stay shut for the whole authored {window}s window"
        );

        // Past the AUTHORED window — and still well inside the serde default,
        // so a resolution block that silently fell back to the default would
        // leave this at 0 and fail here.
        drive_to_sim_secs(&mut app, hit_at + window + 0.5);
        assert!(
            sim_secs(&app) < hit_at + default_window,
            "precondition: still inside the {default_window}s default, so \
             reopening here can only be the authored {window}s being honoured"
        );
        assert!(
            published_score(&app, ship, "assault-starbase") > 0.0,
            "after the authored {window}s with no further hits the `attacked` \
             memory must decay and the raid resume — under the old latch this \
             stayed 0 for as long as anything loitered nearby"
        );
    }

    /// Simulation seconds elapsed on the fixed clock — the same value
    /// `aggregate_doctrine_blackboards` reads from `Res<Time>` inside
    /// `FixedUpdate`.
    fn sim_secs(app: &App) -> f32 {
        app.world().resource::<Time<Fixed>>().elapsed_secs()
    }

    /// Drive whole fixed steps until the sim clock reaches `target` seconds.
    fn drive_to_sim_secs(app: &mut App, target: f32) {
        let mut guard = 0;
        while sim_secs(app) < target {
            app.update();
            guard += 1;
            assert!(
                guard < 10_000,
                "the fixed clock is not advancing — {} s after {guard} updates",
                sim_secs(app)
            );
        }
    }

    /// The score `aggregate_doctrine_blackboards` last published for `id` in
    /// this entity's viewscreen pool.
    fn published_score(app: &App, ship: Entity, id: &str) -> f32 {
        let bb = app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(ship)
            .expect("blackboards");
        match bb
            .0
            .get(&crate::messages::SystemId(
                crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID.to_string(),
            ))
            .expect("viewscreen entry")
        {
            crate::messages::SystemBlackboard::Viewscreen(v) => {
                v.scored_objectives
                    .iter()
                    .find(|o| o.id == id)
                    .unwrap_or_else(|| panic!("{id} must be in the pool: {v:?}"))
                    .score
            }
            _ => panic!("expected Viewscreen blackboard"),
        }
    }

    /// The combat-activity component the `attacked` gate reads, aliased so the
    /// three-reading case tables above stay legible.
    use crate::ship::combat_activity::RecentCombatActivity as Activity;

    /// Publish a viewscreen pool for one entity that carries a
    /// `LastShipAttacker` and a `RecentCombatActivity`, and hand back its
    /// `scored_objectives`. The activity's timestamps are in sim seconds;
    /// `Some(0.0)` is "at the start of the run", which the single tick this
    /// drives leaves well inside the memory window.
    ///
    /// It takes the whole component rather than one timestamp because WHICH
    /// readings feed the gate is the thing under test: hull damage and
    /// shield-absorbed fire both close it, own weapon fire must not.
    ///
    /// Deliberately separate from [`scored_pool_for`]: that helper spawns
    /// neither component, which is a further state (`None` vs `Some(default)`
    /// vs `Some(attacker)`) and the one the pre-#936 bug hid behind.
    fn scored_pool_for_attacker(
        behaviour: crate::entity_config::BehaviourConfig,
        attacker: Option<&str>,
        activity: Activity,
    ) -> Vec<crate::messages::ScoredObjective> {
        use crate::console::weapons::beam::LastShipAttacker;
        use crate::damage::SystemHull;
        use crate::entity_spawner::EntitySystemHull;
        use crate::messages::SystemId;
        use crate::server_app::ShipSystemBlackboards;
        use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

        let mut app = build_test_app();

        app.world_mut().spawn((
            BehaviourSection(behaviour),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                100.0,
            )])),
            ShipSystemBlackboards::default(),
            LastShipAttacker(attacker.map(str::to_string)),
            activity,
        ));
        app.update();

        let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
        let bb = q.iter(app.world()).next().expect("blackboards").clone();
        match bb
            .0
            .get(&crate::messages::SystemId(VIEWSCREEN_SYSTEM_ID.to_string()))
            .expect("viewscreen entry")
        {
            crate::messages::SystemBlackboard::Viewscreen(v) => v.scored_objectives.clone(),
            _ => panic!("expected Viewscreen blackboard"),
        }
    }

    /// A raid hull shaped like the ones `combat_test.toml` spawns: the
    /// template's untargeted Destroy, plus the world's `not_attacked`-gated
    /// assault on the station.
    fn assault_behaviour() -> crate::entity_config::BehaviourConfig {
        use crate::entity_config::{BehaviourConfig, DoctrineObjective};
        BehaviourConfig {
            doctrine: vec![
                DoctrineObjective {
                    id: "destroy-hostiles".into(),
                    text: "Engage whatever is in front of you".into(),
                    directive_kind: Some("Destroy".into()),
                    base_priority: 38.0,
                    ..Default::default()
                },
                DoctrineObjective {
                    id: "assault-starbase".into(),
                    text: "Press the assault on the station".into(),
                    directive_kind: Some("Destroy".into()),
                    directive_target: Some("world.entity.starbase_alpha.name".into()),
                    base_priority: 50.0,
                    zero_gates: vec![crate::objectives::ZeroGateCondition {
                        condition: "not_attacked".into(),
                        threshold: None,
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn faction_registry_resource_is_present_on_native() {
        let app = build_test_app();
        assert!(
            app.world()
                .get_resource::<FactionRegistryResource>()
                .is_some(),
            "FactionRegistryResource must be present on native without WASM preload"
        );
    }

    #[test]
    fn faction_registry_resource_accessible_as_res_not_option() {
        let mut app = build_test_app();
        app.add_systems(bevy::app::Update, read_faction_registry_system);
        app.update(); // Must not panic
    }

    // ── LOD system tests ─────────────────────────────────────────────────────

    use crate::server_app::{LocalShip, Ship};
    use crate::ship_state::ShipPhysics;

    /// Mirrors the production schedule: `simulate_low_lod_ships` (Physics)
    /// steers from the cursor, then `advance_objective_cursors` (Modifiers)
    /// advances it against the ship's post-movement position. The `SimSet`s
    /// themselves aren't configured here, so the order is stated explicitly.
    fn build_lod_test_app() -> App {
        let mut app = App::new();
        app.add_message::<AiWaypointReached>();
        app.insert_resource(Time::<()>::default()).add_systems(
            Update,
            (
                simulate_low_lod_ships.before(lod_ai_ships),
                lod_ai_ships,
                advance_objective_cursors.after(simulate_low_lod_ships),
            ),
        );
        app
    }

    fn tick_with_dt(app: &mut App, dt_secs: f32) {
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(dt_secs));
        app.update();
    }

    fn spawn_player(app: &mut App, x: f32, z: f32) -> Entity {
        app.world_mut()
            .spawn((
                Ship,
                LocalShip,
                Transform::from_xyz(x, 0.0, z),
                ShipPhysics::default(),
                // Same shared set the production player-ship spawn uses.
                ai_high_fidelity_components(),
                // LOD keys on the ANCHOR's bubble radius, so these mechanic tests
                // give the player a 100-unit bubble — the threshold they were
                // written against back when it was `spawn_npc`'s `sensor_range`
                // arg — so their promote/demote distances (50 in, 200 out,
                // 110/120 hysteresis) still mean what they say.
                LodBubble { radius: 100.0 },
            ))
            .id()
    }

    fn spawn_npc(app: &mut App, x: f32, z: f32, sensor_range: f32) -> Entity {
        app.world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(x, 0.0, z),
                ShipPhysics {
                    x,
                    z,
                    forward_speed: 10.0,
                    yaw: 0.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range,
                    ..Default::default()
                },
            ))
            .id()
    }

    /// The guard against the recurring spawn-path gap.
    ///
    /// Every per-ship AI component that must accompany `AiHighFidelity` is
    /// named ONCE, in `AiHighFidelityComponents`, and every spawn path inserts
    /// that set — so a component added there reaches the player ship
    /// (`server_app::spawn_game_start_entities`), every promoted NPC
    /// (`lod_ai_ships`) and the test twin (`ship::test_support`) at the same
    /// time. This test pins what the set contains, so the components three
    /// separate issues have now silently lost on one path or the other cannot
    /// quietly leave it.
    #[test]
    fn the_high_fidelity_component_set_carries_every_per_ship_ai_component() {
        let mut world = World::new();
        let e = world.spawn(ai_high_fidelity_components()).id();
        assert!(world.get::<AiHighFidelity>(e).is_some(), "the marker");
        assert!(
            world
                .get::<crate::console_ai_plugin::ShipFrequencyHintState>(e)
                .is_some(),
            "frequency-hint state (issue #692)"
        );
        assert!(world.get::<crate::ship::helm::ThrustInput>(e).is_some());
        assert!(world.get::<crate::ship::helm::SteeringInput>(e).is_some());
        assert!(world
            .get::<crate::ship::helm::LateralThrustInput>(e)
            .is_some());
        assert!(world
            .get::<crate::ship::helm::VerticalThrustInput>(e)
            .is_some());
        assert!(world.get::<crate::ship::helm::ImpulseCommand>(e).is_some());
        assert!(world.get::<crate::ship::helm::BoostCommand>(e).is_some());
        assert!(
            world
                .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(e)
                .is_some(),
            "the #882 policy runtime state — the component the player ship was \
             missing, which made `ai_policy_state_tick` skip it silently"
        );

        // Insert and remove are the same unit, so a demoted ship cannot keep
        // half of it.
        world.entity_mut(e).remove::<AiHighFidelityComponents>();
        assert!(world.get::<AiHighFidelity>(e).is_none());
        assert!(world
            .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(e)
            .is_none());
        assert!(world.get::<crate::ship::helm::BoostCommand>(e).is_none());
    }

    #[test]
    fn local_ship_permanently_has_ai_high_fidelity() {
        let mut app = build_lod_test_app();
        let player = spawn_player(&mut app, 0.0, 0.0);
        assert!(
            app.world().get::<AiHighFidelity>(player).is_some(),
            "LocalShip must start with AiHighFidelity"
        );
        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(player).is_some(),
            "LocalShip must retain AiHighFidelity after update"
        );
    }

    #[test]
    fn npc_out_of_range_gets_cheap_movement() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 0.0, 0.0);
        let npc = spawn_npc(&mut app, 500.0, 0.0, 100.0);

        let initial = *app.world().get::<ShipPhysics>(npc).unwrap();
        tick_with_dt(&mut app, 0.1);
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();

        // With yaw=0, forward_speed=10, dt=0.1:
        //   x' = 500 + 10 * sin(0) * 0.1 = 500
        //   z' = 0 - 10 * cos(0) * 0.1 = -1
        assert!(
            (physics.z - (initial.z - 1.0)).abs() < 0.001,
            "NPC z should advance by forward_speed * dt: expected {}, got {}",
            initial.z - 1.0,
            physics.z,
        );
        assert!(
            (physics.x - initial.x).abs() < 0.001,
            "NPC x should not change when yaw=0: expected {}, got {}",
            initial.x,
            physics.x,
        );
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "NPC out of range must not have AiHighFidelity"
        );
    }

    #[test]
    fn npc_in_range_promoted_to_high_fidelity() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 0.0, 0.0);
        let npc = spawn_npc(&mut app, 50.0, 0.0, 100.0);

        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_some(),
            "NPC within sensor_range must be promoted to AiHighFidelity"
        );
    }

    #[test]
    fn dwell_timer_prevents_lod_thrashing() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 0.0, 0.0);
        let npc = spawn_npc(&mut app, 50.0, 0.0, 100.0);

        // First update: promote to High (within range)
        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_some(),
            "NPC must start in High after first update"
        );

        // Move far outside range + hysteresis
        app.world_mut()
            .entity_mut(npc)
            .insert(Transform::from_xyz(200.0, 0.0, 0.0));
        app.world_mut().entity_mut(npc).insert(ShipPhysics {
            x: 200.0,
            z: 0.0,
            forward_speed: 10.0,
            yaw: 0.0,
            ..Default::default()
        });

        // One more update: still within 2s dwell window (only 0.2s elapsed total)
        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_some(),
            "NPC must stay High during dwell window"
        );

        // Advance well past 2-second dwell (35 * 0.1 = 3.5s more elapsed)
        for _ in 0..35 {
            tick_with_dt(&mut app, 0.1);
        }

        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "NPC must demote after dwell window elapses"
        );
    }

    /// A `LodBubble` carrier anchors its own zone, so it is held high-fidelity
    /// unconditionally — the station must run its own guns even when the player
    /// (the only other bubble) is on the far side of the map.
    #[test]
    fn lod_bubble_carrier_is_always_high_fidelity() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 5000.0, 0.0);
        let station = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(0.0, 0.0, 0.0),
                ShipPhysics::default(),
                AiProfile::default(),
                LodBubble { radius: 250.0 },
            ))
            .id();
        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(station).is_some(),
            "a LodBubble carrier must stay high-fidelity even with the player far away"
        );
    }

    /// An NPC inside a NON-player bubble (the station's) is promoted even though
    /// the player is nowhere near — the raid sieging the station runs in full
    /// fidelity so its guns, and the station's, actually fire.
    #[test]
    fn npc_inside_a_non_player_bubble_is_promoted() {
        let mut app = build_lod_test_app();
        // Player's 100-unit bubble parked far away.
        spawn_player(&mut app, 5000.0, 0.0);
        // Station anchor at the origin with a 250-unit bubble.
        app.world_mut().spawn((
            Ship,
            Transform::from_xyz(0.0, 0.0, 0.0),
            ShipPhysics::default(),
            AiProfile::default(),
            LodBubble { radius: 250.0 },
        ));
        // NPC 200 units from the station (inside its bubble), far from the player.
        let npc = spawn_npc(&mut app, 200.0, 0.0, 100.0);
        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_some(),
            "an NPC inside the station's bubble must be promoted with the player far away"
        );
    }

    /// An NPC outside EVERY bubble stays low — the station's bubble does not
    /// blanket the map, it covers its own airspace.
    #[test]
    fn npc_outside_every_bubble_stays_low() {
        let mut app = build_lod_test_app();
        // Player's 100-unit bubble at the origin.
        spawn_player(&mut app, 0.0, 0.0);
        // Station's 250-unit bubble a kilometre away.
        app.world_mut().spawn((
            Ship,
            Transform::from_xyz(1000.0, 0.0, 0.0),
            ShipPhysics::default(),
            AiProfile::default(),
            LodBubble { radius: 250.0 },
        ));
        // NPC at 500: 500 from the player (outside 100) and 500 from the station
        // (outside 250).
        let npc = spawn_npc(&mut app, 500.0, 0.0, 100.0);
        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "an NPC outside every bubble must not be promoted"
        );
    }

    /// Promotion on re-entering the ring restores full doctrine cleanly
    /// (issue #933 AC2 — existing behaviour, pinned here). This is the same
    /// `ai_high_fidelity_components()` unit `the_high_fidelity_component_set_
    /// carries_every_per_ship_ai_component` already pins in isolation; this
    /// test pins it end-to-end through the actual demote → re-enter cycle
    /// that `lod_ai_ships` drives, so a future change to either the demote or
    /// promote arm can't quietly stop restoring the full set on re-entry.
    #[test]
    fn demoted_npc_repromoted_on_re_entry_restores_full_high_fidelity_components() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 0.0, 0.0);
        let npc = spawn_npc(&mut app, 50.0, 0.0, 100.0);

        // Promote (within range).
        tick_with_dt(&mut app, 0.1);
        assert!(app.world().get::<AiHighFidelity>(npc).is_some());
        assert!(app
            .world()
            .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(npc)
            .is_some());

        // Demote: move far outside range + hysteresis, wait out the dwell.
        app.world_mut()
            .entity_mut(npc)
            .insert(Transform::from_xyz(500.0, 0.0, 0.0));
        app.world_mut().entity_mut(npc).insert(ShipPhysics {
            x: 500.0,
            z: 0.0,
            forward_speed: 10.0,
            yaw: 0.0,
            ..Default::default()
        });
        for _ in 0..30 {
            tick_with_dt(&mut app, 0.1);
        }
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "must have demoted before re-entry can be tested"
        );
        assert!(
            app.world()
                .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(npc)
                .is_none(),
            "demotion must strip the whole high-fidelity component set, not just the marker"
        );

        // Re-enter: move back within sensor range.
        app.world_mut()
            .entity_mut(npc)
            .insert(Transform::from_xyz(50.0, 0.0, 0.0));
        app.world_mut().entity_mut(npc).insert(ShipPhysics {
            x: 50.0,
            z: 0.0,
            forward_speed: 10.0,
            yaw: 0.0,
            ..Default::default()
        });
        tick_with_dt(&mut app, 0.1);

        assert!(
            app.world().get::<AiHighFidelity>(npc).is_some(),
            "NPC re-entering sensor range must be promoted back to AiHighFidelity"
        );
        assert!(
            app.world()
                .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(npc)
                .is_some(),
            "re-promotion must restore the FULL high-fidelity component set, \
             not just the AiHighFidelity marker"
        );
    }

    // ── Dead-reckoning fallback: decay + return-to-target (issue #933) ─────────

    /// The named AC3 test: demote a ship mid-escape (moving fast, away from
    /// its standing `Destroy` target) and assert it re-enters the engagement
    /// envelope (comes back within `sensor_range` of its target) within a
    /// bounded simulated time, rather than dead-reckoning its exit velocity
    /// off into the void forever.
    #[test]
    fn demoted_ship_mid_escape_returns_to_engagement_envelope_within_bounded_time() {
        let mut app = build_lod_test_app();

        // Standing target the ship's Destroy directive names, resolvable via
        // WorldSnapshot exactly as the production build_world_snapshot pass
        // would publish it — parked at the origin.
        app.insert_resource(WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::nil(),
                name: Some("target-ship".to_string()),
                position: [0.0, 0.0, 0.0],
                faction: None,
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 5.0,
                forward_speed: 0.0,
                movable: true,
                dangerous: true,
                size_rating: 5.0,
                direct_fire_range: 0.0,
                weapon_arcs: vec![],
            }],
        });

        let sensor_range = 100.0_f32;
        // Demoted mid-escape: parked well outside the ring, at a boosted
        // speed, yaw pointed directly AWAY from the target (forward = (0, 1)
        // at yaw = PI in this sim's (sin(yaw), -cos(yaw)) convention).
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(0.0, 0.0, 300.0),
                ShipPhysics {
                    x: 0.0,
                    z: 300.0,
                    forward_speed: 80.0,
                    yaw: std::f32::consts::PI,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range,
                    ..Default::default()
                },
                // Marks this ship as having been through at least one High↔Low
                // transition already — it really was demoted, not just still
                // approaching for the first time (see the gating note on
                // `simulate_low_lod_ships`).
                LodTransitionTimer {
                    last_state_change_secs: 0.0,
                },
                BehaviourSection(crate::entity_config::BehaviourConfig {
                    doctrine: vec![crate::entity_config::DoctrineObjective {
                        id: "assault".into(),
                        text: "Destroy target-ship".into(),
                        directive_kind: Some("Destroy".into()),
                        directive_target: Some("target-ship".into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                // What `aggregate_doctrine_blackboards` would have published
                // this tick for the doctrine above once scored: a single
                // Destroy entry, ungated, so it scores above 0 and qualifies
                // as the standing target `active_destroy_target` resolves.
                blackboards_with_destroy_pool(&[("assault", 1.0, "target-ship")]),
                crate::entities::spawner::HelmConsoleSection(
                    crate::entity_config::EntityConfig::from_toml(
                        "[helm_console]\nmax_speed = 100.0\nmax_yaw_rate = 1.0\n",
                    )
                    .unwrap()
                    .helm_console
                    .unwrap(),
                ),
            ))
            .id();

        let distance_to_target = |app: &App| -> f32 {
            let physics = app.world().get::<ShipPhysics>(npc).unwrap();
            (physics.x * physics.x + physics.z * physics.z).sqrt()
        };

        assert!(
            distance_to_target(&app) > sensor_range,
            "test setup: must start outside the engagement envelope"
        );

        // Bounded time: 60 simulated seconds at 10 Hz. If the old frozen dead-
        // reckoning fallback were still in place this ship would only ever
        // move farther away (300 + 80*t) and this loop would time out.
        let bound_ticks = 600;
        let mut re_entered = false;
        for _ in 0..bound_ticks {
            tick_with_dt(&mut app, 0.1);
            if distance_to_target(&app) <= sensor_range {
                re_entered = true;
                break;
            }
        }

        assert!(
            re_entered,
            "demoted ship must re-enter the engagement envelope (distance <= {sensor_range}) \
             within {bound_ticks} ticks; final distance was {}",
            distance_to_target(&app)
        );
    }

    /// Companion negative-ish check: without a standing named `Destroy`
    /// target (untargeted / no doctrine at all), the fallback still decays
    /// the frozen speed toward cruise rather than holding the boosted exit
    /// speed forever — it just has nothing to turn toward, so it does not
    /// necessarily return. Pins the decay half of #933 independently of the
    /// steering half.
    #[test]
    fn demoted_ship_with_no_destroy_target_still_decays_frozen_speed_toward_cruise() {
        let mut app = build_lod_test_app();

        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(0.0, 0.0, 300.0),
                ShipPhysics {
                    x: 0.0,
                    z: 300.0,
                    forward_speed: 80.0,
                    yaw: std::f32::consts::PI,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 100.0,
                    ..Default::default()
                },
                LodTransitionTimer {
                    last_state_change_secs: 0.0,
                },
                crate::entities::spawner::HelmConsoleSection(
                    crate::entity_config::EntityConfig::from_toml(
                        "[helm_console]\nmax_speed = 100.0\nmax_yaw_rate = 1.0\n",
                    )
                    .unwrap()
                    .helm_console
                    .unwrap(),
                ),
            ))
            .id();

        for _ in 0..100 {
            tick_with_dt(&mut app, 0.1);
        }

        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        // cruise_fraction defaults to 0.5 -> cruise speed = 100.0 * 0.5 = 50.0
        assert!(
            (physics.forward_speed - 50.0).abs() < 0.5,
            "frozen speed must have decayed to the authored cruise fraction of max_speed, got {}",
            physics.forward_speed
        );
    }

    /// Review follow-up on issue #933: the low-LOD return-steer must resolve
    /// its Destroy target through the *scored* pool, honoring `zero_gates`,
    /// not just grab the first `Destroy` entry in authoring order.
    ///
    /// Shipped counter-example this pins: `combat_test.toml`'s wave ships
    /// author `assault-starbase` (Destroy "Starbase Alpha") gated on
    /// `zero_gates = [{condition = "not_attacked"}]`. Once the ship has been
    /// attacked, `score_doctrine_pool` scores that directive at 0 — exactly
    /// like the high-LOD `plan_helm_travel`, which filters `score > 0.0` and
    /// so stops steering at the starbase. A demoted, attacked ship must agree:
    /// with its only Destroy directive scored at 0 (the gate having fired),
    /// `active_destroy_target` must find nothing to steer toward and the
    /// dead-reckoning fallback must fall back to decay-only, never turning
    /// the frozen exit heading back toward that target.
    #[test]
    fn demoted_attacked_ship_does_not_steer_toward_a_zero_gated_destroy_target() {
        let mut app = build_lod_test_app();

        app.insert_resource(WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::nil(),
                name: Some("target-ship".to_string()),
                position: [0.0, 0.0, 0.0],
                faction: None,
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 5.0,
                forward_speed: 0.0,
                movable: true,
                dangerous: true,
                size_rating: 5.0,
                direct_fire_range: 0.0,
                weapon_arcs: vec![],
            }],
        });

        // Same mid-escape setup as the positive case: parked outside sensor
        // range, boosted speed, yaw pointed directly away from the target.
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(0.0, 0.0, 300.0),
                ShipPhysics {
                    x: 0.0,
                    z: 300.0,
                    forward_speed: 80.0,
                    yaw: std::f32::consts::PI,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 100.0,
                    ..Default::default()
                },
                LodTransitionTimer {
                    last_state_change_secs: 0.0,
                },
                BehaviourSection(crate::entity_config::BehaviourConfig {
                    doctrine: vec![crate::entity_config::DoctrineObjective {
                        id: "assault-starbase".into(),
                        text: "Destroy target-ship".into(),
                        directive_kind: Some("Destroy".into()),
                        directive_target: Some("target-ship".into()),
                        base_priority: 100.0,
                        zero_gates: vec![crate::objectives::ZeroGateCondition {
                            condition: "not_attacked".into(),
                            threshold: None,
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                // The gate has already fired: this ship has been attacked, so
                // `score_doctrine_pool` would score `assault-starbase` at 0 —
                // reflected here exactly as `aggregate_doctrine_blackboards`
                // would publish it for an attacked ship.
                blackboards_with_destroy_pool(&[("assault-starbase", 0.0, "target-ship")]),
                crate::entities::spawner::HelmConsoleSection(
                    crate::entity_config::EntityConfig::from_toml(
                        "[helm_console]\nmax_speed = 100.0\nmax_yaw_rate = 1.0\n",
                    )
                    .unwrap()
                    .helm_console
                    .unwrap(),
                ),
            ))
            .id();

        let distance_to_target = |app: &App| -> f32 {
            let physics = app.world().get::<ShipPhysics>(npc).unwrap();
            (physics.x * physics.x + physics.z * physics.z).sqrt()
        };

        let start_distance = distance_to_target(&app);

        for _ in 0..600 {
            tick_with_dt(&mut app, 0.1);
        }

        assert!(
            distance_to_target(&app) >= start_distance,
            "a zero-gated (score == 0) Destroy directive must not steer the ship back — \
             distance to target must not have decreased, started at {start_distance}, \
             ended at {}",
            distance_to_target(&app)
        );
    }

    /// Companion to the zero-gate test above: resolution must pick the
    /// TOP-SCORING Destroy directive, not the first one in authoring order.
    /// A low-scoring (here, zero-scored/gated) decoy entry authored first
    /// must be skipped in favor of a higher-scoring entry authored after it.
    #[test]
    fn demoted_ship_resolves_destroy_target_by_score_not_authoring_order() {
        let mut app = build_lod_test_app();

        app.insert_resource(WorldSnapshot {
            entities: vec![
                crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::nil(),
                    name: Some("decoy-ship".to_string()),
                    position: [0.0, 0.0, 5_000.0],
                    faction: None,
                    shields: None,
                    hull_fraction: None,
                    yaw: None,
                    radius: 5.0,
                    forward_speed: 0.0,
                    movable: true,
                    dangerous: true,
                    size_rating: 5.0,
                    direct_fire_range: 0.0,
                    weapon_arcs: vec![],
                },
                crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::nil(),
                    name: Some("target-ship".to_string()),
                    position: [0.0, 0.0, 0.0],
                    faction: None,
                    shields: None,
                    hull_fraction: None,
                    yaw: None,
                    radius: 5.0,
                    forward_speed: 0.0,
                    movable: true,
                    dangerous: true,
                    size_rating: 5.0,
                    direct_fire_range: 0.0,
                    weapon_arcs: vec![],
                },
            ],
        });

        let sensor_range = 100.0_f32;
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(0.0, 0.0, 300.0),
                ShipPhysics {
                    x: 0.0,
                    z: 300.0,
                    forward_speed: 80.0,
                    yaw: std::f32::consts::PI,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range,
                    ..Default::default()
                },
                LodTransitionTimer {
                    last_state_change_secs: 0.0,
                },
                // First in authoring order is the zero-scored decoy; the
                // real, higher-scoring target is authored second. Resolution
                // must still pick "target-ship".
                blackboards_with_destroy_pool(&[
                    ("decoy", 0.0, "decoy-ship"),
                    ("assault", 1.0, "target-ship"),
                ]),
                crate::entities::spawner::HelmConsoleSection(
                    crate::entity_config::EntityConfig::from_toml(
                        "[helm_console]\nmax_speed = 100.0\nmax_yaw_rate = 1.0\n",
                    )
                    .unwrap()
                    .helm_console
                    .unwrap(),
                ),
            ))
            .id();

        let distance_to_target = |app: &App| -> f32 {
            let physics = app.world().get::<ShipPhysics>(npc).unwrap();
            (physics.x * physics.x + physics.z * physics.z).sqrt()
        };

        assert!(
            distance_to_target(&app) > sensor_range,
            "test setup: must start outside the engagement envelope"
        );

        let bound_ticks = 600;
        let mut re_entered = false;
        for _ in 0..bound_ticks {
            tick_with_dt(&mut app, 0.1);
            if distance_to_target(&app) <= sensor_range {
                re_entered = true;
                break;
            }
        }

        assert!(
            re_entered,
            "demoted ship must steer toward the top-scoring Destroy target \
             (target-ship at the origin), not the zero-scored decoy at z=5000; \
             final distance to origin was {}",
            distance_to_target(&app)
        );
    }

    // ── Low-LOD ships advance on their destroy objectives (issue #1012) ───────
    //
    // Two gates used to strand a Harrow assault wave short of the station:
    //
    //   1. a ship still in its FIRST Low stretch since spawn (no
    //      `LodTransitionTimer`) was exempt from both dead-reckoning
    //      corrections, on the premise that its heading was authored content.
    //      `spawner.rs` seeds every ship at `yaw: 0.0`, `forward_speed: 0.0`,
    //      so the premise was false and the exemption froze it in place;
    //   2. a wave that flew its `close-on-starbase` Reach to the run-in anchor
    //      hit `route_completed`, which is true forever after on a non-looping
    //      route, and parked — `continue`ing before the destroy-steer could be
    //      reached at all, timer or no timer.
    //
    // Both now yield to a *scored* Destroy. Neither yields to anything else.

    /// The engagement geometry these tests share, mirroring `combat_test.toml`:
    /// a starbase the wave was told to kill, at a bearing the spawn heading
    /// (yaw 0, facing -Z) does not point at.
    ///
    /// Two fields are set explicitly because their defaults are traps:
    ///
    /// * **`uuid`** — a fixture ship whose `EntityUuid` is unparseable (or
    ///   absent) resolves its `self_uuid` to `Uuid::nil()` via `unwrap_or_default`,
    ///   and `avoidance_steering` skips `entity.uuid == excluded_uuid`. A nil
    ///   uuid here would therefore delete the objective from every fixture
    ///   ship's hazard picture — a silent exemption these tests never asked for.
    /// * **`dangerous`** — `AiWorldEntity`'s hand-written `Default` sets it
    ///   `true`, which is right for an obstacle and wrong for a destination.
    ///   The low-LOD scan does not read the flag today (see "What this path
    ///   does NOT share with `assess_hazards`" on `low_lod_avoid_yaw`), so what
    ///   actually keeps avoidance out of these tests is distance — the ship
    ///   never closes to within a buffer of the target. Stating it anyway
    ///   records the intent for the day the filters do reach this path.
    fn snapshot_with_named_entity(name: &str, position: [f32; 3]) -> WorldSnapshot {
        WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::from_u128(0x51a7),
                name: Some(name.to_string()),
                position,
                dangerous: false,
                ..Default::default()
            }],
        }
    }

    /// A `HelmConsoleSection` with the authored limits the low-LOD corrections
    /// scale off: cruise is `max_speed * low_lod_cruise_fraction` (0.5 by
    /// parse default → 50) and the turn ceiling is `max_yaw_rate *
    /// low_lod_turn_rate_fraction` (→ 0.5 rad/s).
    fn test_helm_section(
        max_speed: f32,
        max_yaw_rate: f32,
    ) -> crate::entities::spawner::HelmConsoleSection {
        crate::entities::spawner::HelmConsoleSection(
            crate::entity_config::EntityConfig::from_toml(&format!(
                "[helm_console]\nmax_speed = {max_speed}\nmax_yaw_rate = {max_yaw_rate}\n"
            ))
            .unwrap()
            .helm_console
            .unwrap(),
        )
    }

    /// AC1/AC2: a Harrow spawned straight into low LOD, carrying a scored
    /// `Destroy`, steers at the objective rather than sitting on the canned
    /// spawn heading.
    ///
    /// The pre-fix baseline is exact and this test is not vacuous against it:
    /// the ship spawns exactly as `spawner.rs` seeds one (`yaw: 0.0`,
    /// `forward_speed: 0.0`) and has no `LodTransitionTimer`, so under the old
    /// `(ai_profile, lod_timer.is_some())` gate NEITHER correction applied —
    /// yaw stayed 0.0, speed stayed 0.0, and the distance to the objective
    /// stayed 1500.0 for every tick of the run. That is the loiter.
    #[test]
    fn first_low_ship_with_a_scored_destroy_steers_at_it_instead_of_its_spawn_heading() {
        const TARGET: [f32; 3] = [1500.0, 0.0, 0.0];
        let mut app = build_lod_test_app();
        app.insert_resource(snapshot_with_named_entity("starbase-alpha", TARGET));
        // Far enough that `lod_ai_ships` runs and still never promotes: the ship
        // under test spawns at the origin, so a player there would put it inside
        // its own sensor range on tick 1 and take it off this path entirely.
        spawn_player(&mut app, 0.0, 50_000.0);

        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(0.0, 0.0, 0.0),
                // Exactly what `spawner.rs` seeds: facing -Z, stationary.
                ShipPhysics::default(),
                AiProfile {
                    aggression: 0.5,
                    // Small enough that the ship never promotes over the run.
                    sensor_range: 10.0,
                    ..Default::default()
                },
                BehaviourSection(BehaviourConfig::default()),
                blackboards_with_destroy_pool(&[("assault-starbase", 50.0, "starbase-alpha")]),
                test_helm_section(100.0, 1.0),
            ))
            .id();

        assert!(
            app.world().get::<LodTransitionTimer>(npc).is_none(),
            "test setup: this ship must never have been promoted — the whole \
             point is the FIRST Low stretch since spawn"
        );

        let distance_to_target = |app: &App| -> f32 {
            let p = app.world().get::<ShipPhysics>(npc).unwrap();
            ((TARGET[0] - p.x).powi(2) + (TARGET[2] - p.z).powi(2)).sqrt()
        };
        let start_distance = distance_to_target(&app);
        assert!((start_distance - 1500.0).abs() < 0.001);

        // 10 simulated seconds: ~3.1 s to swing 90° at 0.5 rad/s, then cruise.
        for _ in 0..100 {
            tick_with_dt(&mut app, 0.1);
        }

        let physics = *app.world().get::<ShipPhysics>(npc).unwrap();
        // Target bearing from ~the origin to (1500, 0, 0) is π/2.
        assert!(
            (physics.yaw - std::f32::consts::FRAC_PI_2).abs() < 0.05,
            "yaw must converge on the objective's bearing (π/2), not hold the \
             spawn heading of 0.0; got {}",
            physics.yaw
        );
        assert!(
            physics.forward_speed > 1.0,
            "a ship with an order to carry out must get under way — the \
             cruise ramp is the other half of the correction; got {}",
            physics.forward_speed
        );
        assert!(
            distance_to_target(&app) < start_distance - 300.0,
            "the ship must have closed meaningfully on its objective; started \
             at {start_distance}, ended at {} (pre-fix it never moved at all)",
            distance_to_target(&app)
        );
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "this must be the cheap low-LOD path throughout, never a promotion"
        );
    }

    /// AC1 for the shape `combat_test.toml` actually ships: a wave authoring
    /// BOTH `assault-starbase` (Destroy @50) and `close-on-starbase` (Reach
    /// @35) flies the run-in, and then *diverts to the Destroy* instead of
    /// parking on the anchor.
    ///
    /// This is the gate a relaxed `LodTransitionTimer` check alone would not
    /// have opened: `route_completed` is true forever once a non-looping route
    /// runs past its end, and the coast-to-stop branch `continue`d before any
    /// destroy-steering code could run.
    #[test]
    fn low_lod_ship_diverts_to_its_scored_destroy_when_the_run_in_route_completes() {
        const RUN_IN: [f32; 3] = [600.0, 0.0, 100.0];
        const TARGET: [f32; 3] = [2600.0, 0.0, 100.0];
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world
            .anchors
            .insert("harrow_assault_point".to_string(), RUN_IN);
        app.insert_resource(world);
        app.insert_resource(snapshot_with_named_entity("starbase-alpha", TARGET));
        spawn_player(&mut app, 0.0, 0.0);

        // Spawned 300 units +Z of the run-in point, so the approach itself is
        // flown on the spawn heading (yaw 0 = -Z) and the divert is a clean 90°
        // turn onto the objective's bearing.
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(600.0, 0.0, 400.0),
                ShipPhysics {
                    x: 600.0,
                    z: 400.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 10.0,
                    ..Default::default()
                },
                EntityUuid("wave_1".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                ObjectiveCursors::default(),
                with_reach_objective(
                    blackboards_with_destroy_pool(&[("assault-starbase", 50.0, "starbase-alpha")]),
                    "close-on-starbase",
                    35.0,
                    "harrow_assault_point",
                ),
                test_helm_section(100.0, 1.0),
            ))
            .id();

        let distance_to_target = |app: &App| -> f32 {
            let p = app.world().get::<ShipPhysics>(npc).unwrap();
            ((TARGET[0] - p.x).powi(2) + (TARGET[2] - p.z).powi(2)).sqrt()
        };

        // Fly the run-in until the cursor runs past the end of the one-waypoint
        // Reach — from here on `route_completed` is permanently true.
        let mut completed_after = None;
        for tick in 0..300 {
            tick_with_dt(&mut app, 0.1);
            if cursor_state(&app, npc) == vec![("close-on-starbase".to_string(), 1)] {
                completed_after = Some(tick);
                break;
            }
        }
        assert!(
            completed_after.is_some(),
            "test setup: the ship must actually reach the run-in anchor first"
        );
        let distance_on_arrival = distance_to_target(&app);

        // 15 more seconds, well past the ~4 s the old coast-to-stop needed to
        // bring a 40 u/s run-in to a dead halt.
        for _ in 0..150 {
            tick_with_dt(&mut app, 0.1);
        }

        let physics = *app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            physics.forward_speed > 1.0,
            "a completed run-in must NOT park a ship that still has a scored \
             Destroy to serve; forward_speed was {}",
            physics.forward_speed
        );
        // 0.1 rad, not the 0.05 the geometry happens to land inside: the ship
        // overshoots the target's z during its ~3 s turn and settles a hair
        // PAST π/2 aiming back, ~0.04 rad off. That margin is deterministic but
        // coincidental — the claim is "turned from 0.0 onto the objective's
        // bearing", not "landed within 0.04 of it".
        assert!(
            (physics.yaw - std::f32::consts::FRAC_PI_2).abs() < 0.1,
            "yaw must turn from the run-in heading (0.0) onto the objective's \
             bearing (π/2); got {}",
            physics.yaw
        );
        assert!(
            distance_to_target(&app) < distance_on_arrival - 300.0,
            "the ship must close on the objective after the run-in completes; \
             was {distance_on_arrival} on arrival, now {}",
            distance_to_target(&app)
        );
    }

    /// The divert is not a hole in collision avoidance. Issue #1012 added a
    /// SECOND `low_lod_objective_steer` call site, and the `low_lod_avoid_yaw`
    /// bend that follows it has to still apply there — otherwise a wave that
    /// diverts onto its Destroy flies through whatever lies between it and the
    /// station, which is the failure issue #968 fixed on the other branches.
    ///
    /// Run twice on identical geometry, once with a rock parked on the divert
    /// bearing, and compare. An absolute yaw assertion could not carry this
    /// claim: the ~3 s slew onto the objective's bearing puts yaw far from π/2
    /// all by itself. The two runs must also be bit-identical over the run-in,
    /// which pins the divergence to the divert rather than to the route branch.
    #[test]
    fn low_lod_divert_still_avoids_a_hazard_on_the_objective_bearing() {
        const RUN_IN: [f32; 3] = [600.0, 0.0, 100.0];
        const TARGET: [f32; 3] = [2600.0, 0.0, 100.0];
        // The run-in takes ~9 s; sample well inside it for the "same route,
        // either way" half of the comparison.
        const RUN_IN_TICKS: usize = 80;

        // Yaw after every tick, so the comparison sees the whole manoeuvre
        // rather than one instant the ship may already have flown past.
        let run = |rock: Option<crate::ai::AiWorldEntity>| -> Vec<f32> {
            let mut app = build_lod_test_app();
            let mut world = crate::world::config::WorldConfig::default();
            world
                .anchors
                .insert("harrow_assault_point".to_string(), RUN_IN);
            app.insert_resource(world);
            let mut snapshot = snapshot_with_named_entity("starbase-alpha", TARGET);
            snapshot.entities.extend(rock);
            app.insert_resource(snapshot);
            spawn_player(&mut app, 0.0, 0.0);

            let npc = app
                .world_mut()
                .spawn((
                    Ship,
                    Transform::from_xyz(600.0, 0.0, 400.0),
                    ShipPhysics {
                        x: 600.0,
                        z: 400.0,
                        ..Default::default()
                    },
                    AiProfile {
                        aggression: 0.5,
                        sensor_range: 10.0,
                        ..Default::default()
                    },
                    EntityUuid("wave_1".to_string()),
                    BehaviourSection(BehaviourConfig::default()),
                    ObjectiveCursors::default(),
                    with_reach_objective(
                        blackboards_with_destroy_pool(&[(
                            "assault-starbase",
                            50.0,
                            "starbase-alpha",
                        )]),
                        "close-on-starbase",
                        35.0,
                        "harrow_assault_point",
                    ),
                    test_helm_section(100.0, 1.0),
                ))
                .id();

            (0..200)
                .map(|_| {
                    tick_with_dt(&mut app, 0.1);
                    app.world().get::<ShipPhysics>(npc).unwrap().yaw
                })
                .collect()
        };

        let clear = run(None);
        // Downrange of the divert and 300 units clear of the run-in leg, so it
        // is out of reach until the ship turns onto the objective's bearing.
        // Its own uuid must differ from the fixture ship's (which resolves to
        // nil — `EntityUuid("wave_1")` does not parse) or `avoidance_steering`
        // would skip it as the ship's own snapshot entry.
        let obstructed = run(Some(crate::ai::AiWorldEntity {
            uuid: uuid::Uuid::from_u128(0x0c1a),
            position: [900.0, 0.0, 20.0],
            radius: 40.0,
            ..Default::default()
        }));

        assert_eq!(
            clear[..RUN_IN_TICKS],
            obstructed[..RUN_IN_TICKS],
            "test setup: the rock must be irrelevant until the divert — a \
             difference during the run-in would mean this test pins the route \
             branch's avoidance, which is already covered"
        );
        let max_divergence = clear
            .iter()
            .zip(&obstructed)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_divergence > 0.1,
            "a hazard on the divert bearing must bend the heading off it — the \
             avoidance pass has to run on the #1012 branch too; the largest \
             difference from the clear run was {max_divergence} rad"
        );
    }

    /// The other half of the divert: with nothing *scored* in the Destroy pool,
    /// a completed route still parks the ship exactly as it did before #1012.
    /// This is the Requiem Courier case — a hull whose whole behaviour is one
    /// `Reach` — and the regression #1012's change could most easily cause.
    ///
    /// The pool holds a zero-scored `Destroy` naming an entity the snapshot
    /// really does carry, rather than being empty: an empty pool cannot tell
    /// "`active_destroy_target`'s `score > 0.0` filter held" from "there was
    /// nothing to filter", and the divert's brand-new call site would never see
    /// a zero-scored Destroy at all. Post-#1010 that is the live case — an
    /// assault wave under fire has its `assault-starbase` gated to 0 — so a
    /// wave that has flown its run-in and then taken a hit must park, not
    /// charge on at a target its own doctrine has given up on.
    #[test]
    fn low_lod_ship_still_parks_on_a_completed_route_with_a_zero_gated_destroy() {
        const RUN_IN: [f32; 3] = [600.0, 0.0, 100.0];
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world
            .anchors
            .insert("harrow_assault_point".to_string(), RUN_IN);
        app.insert_resource(world);
        // The Destroy's target IS in the snapshot and 2000 units away: the park
        // must come from the zero gate alone, not from a target that fails to
        // resolve or a snapshot with nothing in it.
        app.insert_resource(snapshot_with_named_entity(
            "starbase-alpha",
            [2600.0, 0.0, 100.0],
        ));
        spawn_player(&mut app, 0.0, 0.0);

        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(600.0, 0.0, 400.0),
                ShipPhysics {
                    x: 600.0,
                    z: 400.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 10.0,
                    ..Default::default()
                },
                EntityUuid("courier".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                ObjectiveCursors::default(),
                with_reach_objective(
                    blackboards_with_destroy_pool(&[("assault-starbase", 0.0, "starbase-alpha")]),
                    "close-on-starbase",
                    35.0,
                    "harrow_assault_point",
                ),
                test_helm_section(100.0, 1.0),
            ))
            .id();

        for _ in 0..450 {
            tick_with_dt(&mut app, 0.1);
        }

        let arrived = *app.world().get::<ShipPhysics>(npc).unwrap();
        assert_eq!(
            arrived.forward_speed, 0.0,
            "with nothing scored to fly at, a finished route must still coast \
             to a stop"
        );
        for _ in 0..50 {
            tick_with_dt(&mut app, 0.1);
        }
        let later = *app.world().get::<ShipPhysics>(npc).unwrap();
        assert_eq!(
            (later.x, later.z),
            (arrived.x, arrived.z),
            "a parked ship must hold station, not resume drifting"
        );
        // The 20-unit arrival radius plus the coast: 40 u/s (max_speed * the
        // 0.4 route fraction) shed at LOW_LOD_ACCEL_PER_SEC = 10 u/s² is 80
        // units. Unchanged by #1012 — the bound is "parked at the anchor",
        // not "sailed across the map at cruise".
        let drift = ((later.x - RUN_IN[0]).powi(2) + (later.z - RUN_IN[2]).powi(2)).sqrt();
        assert!(
            drift < 120.0,
            "the ship must park near the anchor it arrived at, {drift} units off"
        );
    }

    /// AC: the zero-gate interaction survives the relaxed first-Low gate.
    ///
    /// The no-timer twin of
    /// `demoted_attacked_ship_does_not_steer_toward_a_zero_gated_destroy_target`:
    /// post-#1010 an attacked wave's `assault-starbase` scores 0, and
    /// `active_destroy_target` filters on `score > 0.0`. A first-Low ship under
    /// fire must therefore resolve nothing, keep the pre-#933 exemption, and
    /// drift on untouched — neither the turn nor the cruise ramp may fire.
    #[test]
    fn first_low_ship_does_not_steer_toward_a_zero_gated_destroy_target() {
        let mut app = build_lod_test_app();
        app.insert_resource(snapshot_with_named_entity(
            "starbase-alpha",
            [1500.0, 0.0, 0.0],
        ));
        // Far away, so the ship under test is never promoted off this path —
        // a promoted ship would hold yaw and speed for want of being simulated
        // at all, and pass this test vacuously.
        spawn_player(&mut app, 0.0, 50_000.0);

        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(0.0, 0.0, 0.0),
                ShipPhysics {
                    forward_speed: 10.0,
                    yaw: 0.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 10.0,
                    ..Default::default()
                },
                BehaviourSection(BehaviourConfig::default()),
                // The gate has already fired: this ship has taken a hit.
                blackboards_with_destroy_pool(&[("assault-starbase", 0.0, "starbase-alpha")]),
                test_helm_section(100.0, 1.0),
            ))
            .id();

        for _ in 0..100 {
            tick_with_dt(&mut app, 0.1);
        }

        let physics = *app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            physics.yaw.abs() < 1e-6,
            "a zero-scored Destroy is not authored intent and must not turn a \
             first-Low ship; yaw was {}",
            physics.yaw
        );
        assert_eq!(
            physics.forward_speed, 10.0,
            "with nothing resolving, the first-Low exemption stands and the \
             cruise ramp must not fire either"
        );
        assert!(
            physics.x.abs() < 1e-3,
            "the ship must still be drifting straight down -Z, x was {}",
            physics.x
        );
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "guard against a vacuous pass: a promoted ship is not simulated by \
             this path at all and would hold yaw and speed for the wrong reason"
        );
    }

    /// A ship with no `AiProfile` at all is outside low-LOD authoring entirely
    /// and keeps its unmodified drift, scored `Destroy` or not. Pins the one
    /// arm of the gate #1012 deliberately did not widen.
    #[test]
    fn first_low_ship_without_an_ai_profile_keeps_its_unmodified_drift() {
        let mut app = build_lod_test_app();
        app.insert_resource(snapshot_with_named_entity(
            "starbase-alpha",
            [1500.0, 0.0, 0.0],
        ));

        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(0.0, 0.0, 0.0),
                ShipPhysics {
                    forward_speed: 10.0,
                    yaw: 0.0,
                    ..Default::default()
                },
                BehaviourSection(BehaviourConfig::default()),
                blackboards_with_destroy_pool(&[("assault-starbase", 50.0, "starbase-alpha")]),
                test_helm_section(100.0, 1.0),
            ))
            .id();

        for _ in 0..100 {
            tick_with_dt(&mut app, 0.1);
        }

        let physics = *app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            physics.yaw.abs() < 1e-6,
            "no AiProfile means no authored low-LOD tuning to steer by; yaw \
             was {}",
            physics.yaw
        );
        assert_eq!(
            physics.forward_speed, 10.0,
            "no AiProfile means no cruise fraction to ramp toward either"
        );
        assert!(
            (physics.z + 100.0).abs() < 0.1,
            "10 u/s down -Z for 10 s is z = -100; got {}",
            physics.z
        );
    }

    // ── Low-LOD patrol wiring (ObjectiveCursors / advance_cursor) ───────────────

    /// Build a `ShipSystemBlackboards` carrying a single Helm-relevant Patrol
    /// objective under the viewscreen entry (mirrors what
    /// `aggregate_doctrine_blackboards` publishes for a patrolling ship).
    fn blackboards_with_patrol(
        id: &str,
        waypoints: &[&str],
        loop_path: bool,
    ) -> crate::server_app::ShipSystemBlackboards {
        let mut bb = crate::server_app::ShipSystemBlackboards::default();
        bb.0.insert(
            crate::system_registry::viewscreen_system_id(),
            crate::messages::SystemBlackboard::Viewscreen(crate::messages::ViewscreenBlackboard {
                scored_objectives: vec![crate::messages::ScoredObjective {
                    id: id.to_string(),
                    score: 1.0,
                    directive: crate::messages::AiDirective::Patrol {
                        anchors: waypoints.iter().map(|w| w.to_string()).collect(),
                        loop_path,
                    },
                    source: crate::messages::ObjectiveSource::Doctrine,
                    relevance: vec![crate::messages::SystemAffinity::Helm],
                    snapshot: crate::messages::ObjectiveSnapshot {
                        id: id.to_string(),
                        text: "Patrol".to_string(),
                        text_params: Default::default(),
                        mandatory: false,
                        status: crate::messages::ObjectiveStatus::Active,
                        targets: vec![],
                        source: crate::messages::ObjectiveSource::Doctrine,
                    },
                }],
                ..Default::default()
            }),
        );
        bb
    }

    /// Build a `ShipSystemBlackboards` carrying an already-scored Destroy pool
    /// (mirrors what `aggregate_doctrine_blackboards` + `score_doctrine_pool`
    /// publish for a ship's standing Destroy doctrine, `zero_gates` already
    /// applied). Entries are given in authoring order so a test can put a
    /// low-/zero-scoring entry first and a higher-scoring one after it —
    /// exactly the shape the #933 review follow-up caught: the low-LOD Destroy
    /// steer must resolve by score, not by position in this slice.
    fn blackboards_with_destroy_pool(
        entries: &[(&str, f32, &str)],
    ) -> crate::server_app::ShipSystemBlackboards {
        let mut bb = crate::server_app::ShipSystemBlackboards::default();
        bb.0.insert(
            crate::system_registry::viewscreen_system_id(),
            crate::messages::SystemBlackboard::Viewscreen(crate::messages::ViewscreenBlackboard {
                scored_objectives: entries
                    .iter()
                    .map(|(id, score, target)| crate::messages::ScoredObjective {
                        id: id.to_string(),
                        score: *score,
                        directive: crate::messages::AiDirective::Destroy {
                            target: target.to_string(),
                        },
                        source: crate::messages::ObjectiveSource::Doctrine,
                        relevance: vec![crate::messages::SystemAffinity::Helm],
                        snapshot: crate::messages::ObjectiveSnapshot {
                            id: id.to_string(),
                            text: "Destroy".to_string(),
                            text_params: Default::default(),
                            mandatory: false,
                            status: crate::messages::ObjectiveStatus::Active,
                            targets: vec![],
                            source: crate::messages::ObjectiveSource::Doctrine,
                        },
                    })
                    .collect(),
                ..Default::default()
            }),
        );
        bb
    }

    /// Push a Helm-relevant `Reach` entry onto an existing fixture blackboard.
    ///
    /// Composes with [`blackboards_with_destroy_pool`] to author the MIXED
    /// doctrine `combat_test.toml`'s Harrow assault waves carry — a
    /// high-scoring `Destroy` on the starbase plus a lower-scoring `Reach` on
    /// the run-in anchor. That mix is the whole of issue #1012:
    /// `active_waypoint_route` filters to Patrol/Reach and so only ever sees
    /// the Reach, which used to let a completed run-in park the wave with its
    /// top-scoring directive unserved.
    fn with_reach_objective(
        mut bb: crate::server_app::ShipSystemBlackboards,
        id: &str,
        score: f32,
        anchor: &str,
    ) -> crate::server_app::ShipSystemBlackboards {
        if let Some(crate::messages::SystemBlackboard::Viewscreen(v)) =
            bb.0.get_mut(&crate::system_registry::viewscreen_system_id())
        {
            v.scored_objectives.push(crate::messages::ScoredObjective {
                id: id.to_string(),
                score,
                directive: crate::messages::AiDirective::Reach {
                    anchor: anchor.to_string(),
                },
                source: crate::messages::ObjectiveSource::Doctrine,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: id.to_string(),
                    text: "Reach".to_string(),
                    text_params: Default::default(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Doctrine,
                },
            });
        }
        bb
    }

    /// Spawn a low-LOD patrolling NPC at `(x, z)` carrying the TOML-authored
    /// `BehaviourSection` the cursor evaluator reads its arrival radius from.
    fn spawn_patrolling_npc(
        app: &mut App,
        x: f32,
        z: f32,
        uuid: &str,
        objective_id: &str,
        waypoints: &[&str],
        loop_path: bool,
    ) -> Entity {
        app.world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(x, 0.0, z),
                ShipPhysics {
                    x,
                    z,
                    forward_speed: 10.0,
                    yaw: 0.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 100.0,
                    ..Default::default()
                },
                EntityUuid(uuid.to_string()),
                BehaviourSection(BehaviourConfig::default()),
                ObjectiveCursors::default(),
                blackboards_with_patrol(objective_id, waypoints, loop_path),
            ))
            .id()
    }

    /// Every cursor on `entity` as `(objective_id, waypoint_index)`.
    fn cursor_state(app: &App, entity: Entity) -> Vec<(String, usize)> {
        app.world()
            .get::<ObjectiveCursors>(entity)
            .unwrap()
            .0
            .iter()
            .map(|c| (c.objective_id.clone(), c.index()))
            .collect()
    }

    #[test]
    fn cursor_advances_when_ship_arrives_at_its_waypoint() {
        let mut app = build_lod_test_app();
        // Ship starts AT wp0, so it arrives immediately; wp1 is 200 units away.
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 1)],
            "cursor must advance to waypoint 1 after arriving at waypoint 0"
        );
    }

    #[test]
    fn cursor_does_not_advance_while_ship_is_short_of_its_waypoint() {
        let mut app = build_lod_test_app();
        // wp0 sits 200 units away — far outside the default arrival radius.
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [700.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [900.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 0)],
            "cursor must stay on waypoint 0 until the ship reaches it"
        );
    }

    /// The arrival radius is designer-tunable via `[behaviour]
    /// waypoint_arrival_radius` in entity TOML — a ship with a wide radius
    /// counts as arrived from a distance that a default-radius ship does not.
    #[test]
    fn arrival_radius_comes_from_the_entity_behaviour_config() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [600.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [900.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        // Both ships sit 100 units from wp0, but only the wide-radius ship
        // is close enough to count as arrived.
        let narrow = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-narrow",
            "patrol",
            &["wp0", "wp1"],
            true,
        );
        let wide = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-wide",
            "patrol",
            &["wp0", "wp1"],
            true,
        );
        app.world_mut()
            .entity_mut(wide)
            .insert(BehaviourSection(BehaviourConfig {
                waypoint_arrival_radius: 150.0,
                ..Default::default()
            }));

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            cursor_state(&app, narrow),
            vec![("patrol".to_string(), 0)],
            "default arrival radius must not count 100 units away as arrived"
        );
        assert_eq!(
            cursor_state(&app, wide),
            vec![("patrol".to_string(), 1)],
            "a TOML-widened arrival radius must count 100 units away as arrived"
        );
    }

    #[test]
    fn reach_objective_cursor_advances_to_terminal_on_arrival() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world
            .anchors
            .insert("dock".to_string(), [500.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let mut bb = crate::server_app::ShipSystemBlackboards::default();
        bb.0.insert(
            crate::system_registry::viewscreen_system_id(),
            crate::messages::SystemBlackboard::Viewscreen(crate::messages::ViewscreenBlackboard {
                scored_objectives: vec![crate::messages::ScoredObjective {
                    id: "reach-dock".to_string(),
                    score: 1.0,
                    directive: crate::messages::AiDirective::Reach {
                        anchor: "dock".to_string(),
                    },
                    source: crate::messages::ObjectiveSource::Mission,
                    relevance: vec![crate::messages::SystemAffinity::Helm],
                    snapshot: crate::messages::ObjectiveSnapshot {
                        id: "reach-dock".to_string(),
                        text: "Reach the dock".to_string(),
                        text_params: Default::default(),
                        mandatory: false,
                        status: crate::messages::ObjectiveStatus::Active,
                        targets: vec![],
                        source: crate::messages::ObjectiveSource::Mission,
                    },
                }],
                ..Default::default()
            }),
        );

        // Ship sits on the dock anchor → arrived.
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(500.0, 0.0, 500.0),
                ShipPhysics {
                    x: 500.0,
                    z: 500.0,
                    forward_speed: 10.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 100.0,
                    ..Default::default()
                },
                EntityUuid("npc-reach".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                ObjectiveCursors::default(),
                bb,
            ))
            .id();

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            cursor_state(&app, npc),
            vec![("reach-dock".to_string(), 1)],
            "a Reach cursor is a one-waypoint route: arriving moves it to the terminal index"
        );

        // Having arrived, the low-LOD ship coasts to a stop and stays put. It
        // used to fall through to the dumb forward-drift the moment its route
        // went terminal, which sailed the Requiem Courier — a hull whose whole
        // behaviour is one Reach — clean through its destination and out of the
        // scenario at cruise speed.
        for _ in 0..20 {
            tick_with_dt(&mut app, 0.1);
        }
        let arrived = *app.world().get::<ShipPhysics>(npc).unwrap();
        assert_eq!(
            arrived.forward_speed, 0.0,
            "a ship that has flown its route to the end must come to rest"
        );

        for _ in 0..20 {
            tick_with_dt(&mut app, 0.1);
        }
        let later = *app.world().get::<ShipPhysics>(npc).unwrap();
        assert_eq!(
            (later.x, later.z),
            (arrived.x, arrived.z),
            "a stopped ship must hold station, not resume drifting"
        );
        // And it stopped near where it arrived rather than crossing the map.
        let drift = ((later.x - 500.0).powi(2) + (later.z - 500.0).powi(2)).sqrt();
        assert!(
            drift < 10.0,
            "the ship coasted {drift} units past the anchor it arrived at"
        );
    }

    #[test]
    fn low_lod_npc_follows_patrol_route_between_waypoints() {
        let mut app = build_lod_test_app();
        // Ship starts AT wp0; wp1 is offset in +x so the steer is observable
        // in both yaw and position.
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        // Tick 1 arrives at wp0 and advances the cursor to wp1; tick 2 is the
        // first tick that steers toward wp1.
        tick_with_dt(&mut app, 0.1);
        tick_with_dt(&mut app, 0.1);

        // Steering toward wp1 (700,0,500): dx=+200, dz=0 → bearing = π/2.
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            (physics.yaw - std::f32::consts::FRAC_PI_2).abs() < 0.01,
            "yaw should steer toward wp1 bearing (π/2), got {}",
            physics.yaw,
        );
        assert!(
            physics.x > 500.0,
            "ship should advance toward wp1 (+x), got x={}",
            physics.x,
        );

        // Must remain low-LOD (never promoted), proving this is the cheap path.
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "patrolling NPC out of range must stay low-LOD"
        );
    }

    /// End-to-end route following: a low-LOD NPC placed on a two-waypoint
    /// looping route drives itself to the far waypoint, wraps back to the
    /// first, and returns — without ever being promoted to high LOD.
    #[test]
    fn low_lod_npc_patrol_route_wraps_around_and_returns_to_first_waypoint() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        // forward_speed 10 → ~1 unit/tick at dt=0.1, so the 200-unit leg out
        // to wp1 takes ~180 ticks to close to within the 20-unit arrival
        // radius. Run until the cursor wraps back to wp0 (bounded well above
        // that), sampling the cursor each tick to prove the whole cycle.
        let mut seen_indices = Vec::new();
        let mut reached_max_x: f32 = 500.0;
        for _ in 0..600 {
            tick_with_dt(&mut app, 0.1);
            let idx = cursor_state(&app, npc)[0].1;
            if seen_indices.last() != Some(&idx) {
                seen_indices.push(idx);
            }
            reached_max_x = reached_max_x.max(app.world().get::<ShipPhysics>(npc).unwrap().x);
            // Stop on the first wraparound: 0 → 1 → back to 0.
            if seen_indices.len() == 2 {
                break;
            }
        }

        assert_eq!(
            seen_indices,
            vec![1, 0],
            "cursor must advance to wp1, then wrap back to wp0 on a looping route"
        );
        assert!(
            reached_max_x > 680.0,
            "ship must actually travel the leg out to wp1 (x≈700), got max x={}",
            reached_max_x,
        );

        // Steering reads the cursor before the evaluator advances it, so the
        // turn toward wp0 happens on the tick *after* the wrap.
        tick_with_dt(&mut app, 0.1);

        // Having wrapped, it is heading back toward wp0 (-x) → bearing = -π/2.
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            (physics.yaw + std::f32::consts::FRAC_PI_2).abs() < 0.01,
            "after wraparound the ship should steer back toward wp0 (-π/2), got {}",
            physics.yaw,
        );
        assert!(
            physics.x < reached_max_x,
            "ship must be travelling back toward wp0 after the wrap"
        );
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "patrolling NPC out of range must stay low-LOD for the whole route"
        );
    }

    /// The arrival that advances the cursor is announced as an
    /// `AiWaypointReached` message — the bridge the world plugin turns into a
    /// `WorldEvent::WaypointReached` for `on_waypoint_reached` triggers.
    #[test]
    fn reaching_a_waypoint_emits_ai_waypoint_reached() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        tick_with_dt(&mut app, 0.1);

        let messages = app
            .world()
            .resource::<bevy::ecs::message::Messages<AiWaypointReached>>();
        let mut cursor = messages.get_cursor();
        let emitted: Vec<_> = cursor.read(messages).collect();

        assert_eq!(
            emitted.len(),
            1,
            "arriving at wp0 must announce exactly once"
        );
        assert_eq!(emitted[0].entity_uuid, "npc-1");
        assert_eq!(emitted[0].objective_id, "patrol");
        assert_eq!(
            emitted[0].waypoint, "wp0",
            "the announced waypoint must be the one arrived at, not the next one"
        );
    }

    /// Read every `AiWaypointReached` emitted so far, as `(uuid, waypoint)`.
    fn reached_waypoints(app: &App) -> Vec<(String, String)> {
        let messages = app
            .world()
            .resource::<bevy::ecs::message::Messages<AiWaypointReached>>();
        let mut cursor = messages.get_cursor();
        cursor
            .read(messages)
            .map(|m| (m.entity_uuid.clone(), m.waypoint.clone()))
            .collect()
    }

    /// Regression: a tick that carries the cursor past several waypoints at
    /// once must announce every one of them. With `wp0` and `wp1` spaced
    /// closer than the arrival radius, the cursor jumps 0 → 2 in a single
    /// tick; announcing only `wp0` would leave an `on_waypoint_reached`
    /// trigger keyed to `wp1` silently dead.
    #[test]
    fn one_message_per_waypoint_consumed_when_a_tick_skips_several() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        // wp0 and wp1 are 5 units apart — well inside the 20-unit default
        // arrival radius — while wp2 is a long leg away.
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [505.0, 0.0, 500.0]);
        world.anchors.insert("wp2".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1", "wp2"],
            true,
        );

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            reached_waypoints(&app),
            vec![
                ("npc-1".to_string(), "wp0".to_string()),
                ("npc-1".to_string(), "wp1".to_string()),
            ],
            "both waypoints consumed this tick must be announced, in route order"
        );
        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 2)],
            "cursor must land on the far wp2 after skipping wp0 and wp1"
        );
    }

    /// Regression: a looping route whose every waypoint sits inside the
    /// arrival radius closes its lap immediately. Any route with legs shorter
    /// than the authored `waypoint_arrival_radius` does this — a designer
    /// widening the radius for a station-keeping patrol, not just a
    /// pathological case.
    ///
    /// The contract has three parts, and the second and third are what the
    /// original permanent-retirement design broke: the ship announced its lap
    /// and then lost its cursor entirely, fell through to the dumb
    /// forward-move, and flew out of the cluster in a straight line forever
    /// with no way back.
    #[test]
    fn looping_route_entirely_inside_arrival_radius_announces_once_then_holds_station() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        // All three anchors are within the 20-unit default arrival radius of
        // the ship's spawn point.
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [505.0, 0.0, 500.0]);
        world.anchors.insert("wp2".to_string(), [500.0, 0.0, 505.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1", "wp2"],
            true,
        );

        // First tick: the lap closes — each waypoint announced exactly once.
        tick_with_dt(&mut app, 0.1);
        assert_eq!(
            reached_waypoints(&app),
            vec![
                ("npc-1".to_string(), "wp0".to_string()),
                ("npc-1".to_string(), "wp1".to_string()),
                ("npc-1".to_string(), "wp2".to_string()),
            ],
            "the closing lap must announce each waypoint exactly once"
        );

        // ── 2. No per-tick spam, and the ship holds station ────────────────
        // Drain, then tick long enough that a ship flying off at
        // forward_speed (1 unit/tick here) would be 200 units clear of the
        // cluster.
        app.world_mut()
            .resource_mut::<bevy::ecs::message::Messages<AiWaypointReached>>()
            .clear();
        for _ in 0..200 {
            tick_with_dt(&mut app, 0.1);
        }

        assert!(
            reached_waypoints(&app).is_empty(),
            "a settled degenerate route must not re-announce its waypoints every tick"
        );
        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 0)],
            "the cursor must stay on a real waypoint index, not a sentinel"
        );
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        let drift = ((physics.x - 500.0).powi(2) + (physics.z - 500.0).powi(2)).sqrt();
        assert!(
            drift < 20.0,
            "the ship must keep station on its route, not fly out of the cluster: \
             drifted {} units to ({}, {})",
            drift,
            physics.x,
            physics.z,
        );

        // ── 3. Moved out of the radius, the route resumes ──────────────────
        // Shove the ship 2000 units clear (a knockback, tow or scenario
        // teleport does the same thing).
        {
            let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
            physics.x = 2500.0;
            physics.z = 500.0;
        }
        tick_with_dt(&mut app, 0.1);

        assert!(
            reached_waypoints(&app).is_empty(),
            "nothing was arrived at 2000 units out"
        );
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            (physics.yaw + std::f32::consts::FRAC_PI_2).abs() < 0.01,
            "the resumed route must steer back toward wp0 (-π/2), got {}",
            physics.yaw,
        );
        assert!(
            physics.x < 2500.0,
            "the ship must fly back toward its route, got x={}",
            physics.x,
        );

        // Back in the cluster, the lap is flown and announced afresh — the
        // route is alive, not permanently dead.
        {
            let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
            physics.x = 500.0;
            physics.z = 500.0;
        }
        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            reached_waypoints(&app),
            vec![
                ("npc-1".to_string(), "wp0".to_string()),
                ("npc-1".to_string(), "wp1".to_string()),
                ("npc-1".to_string(), "wp2".to_string()),
            ],
            "a route resumed after leaving the arrival radius must announce again"
        );
    }

    /// Regression (issue #696 review): the shipped-content shape of the bug.
    /// `waypoint_arrival_radius` is designer-tunable per entity, so a route
    /// whose legs are shorter than the authored radius is ordinary content —
    /// a station-keeping patrol. It must not silently become "fly off the map
    /// in a straight line, forever".
    #[test]
    fn route_with_legs_shorter_than_the_authored_radius_does_not_die() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        // 100-unit legs against a 150-unit authored radius.
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [600.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );
        app.world_mut()
            .entity_mut(npc)
            .insert(BehaviourSection(BehaviourConfig {
                waypoint_arrival_radius: 150.0,
                ..Default::default()
            }));

        // The lap closes on tick 1: both waypoints are inside the radius.
        tick_with_dt(&mut app, 0.1);
        assert_eq!(
            reached_waypoints(&app),
            vec![
                ("npc-1".to_string(), "wp0".to_string()),
                ("npc-1".to_string(), "wp1".to_string()),
            ],
            "the closing lap announces each waypoint once"
        );

        // 400 ticks at 1 unit/tick: a ship that lost its cursor would be 400
        // units clear by now. This one is still on its route.
        for _ in 0..400 {
            tick_with_dt(&mut app, 0.1);
        }
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        let dist_to_wp0 = ((physics.x - 500.0).powi(2) + (physics.z - 500.0).powi(2)).sqrt();
        assert!(
            dist_to_wp0 < 150.0,
            "the ship must hold its route, not fly off at forward_speed: {} units out",
            dist_to_wp0,
        );
        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 0)],
            "the cursor must still name a real waypoint"
        );

        // And the route is resumable: shoved clear, it steers back.
        app.world_mut()
            .resource_mut::<bevy::ecs::message::Messages<AiWaypointReached>>()
            .clear();
        {
            let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
            physics.x = 3000.0;
            physics.z = 500.0;
        }
        tick_with_dt(&mut app, 0.1);
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            physics.x < 3000.0,
            "a resumed route must fly the ship back toward wp0, got x={}",
            physics.x,
        );
        assert!(
            !app.world().get::<ObjectiveCursors>(npc).unwrap().0[0].settled(),
            "leaving the arrival radius must un-settle the cursor"
        );
    }

    #[test]
    fn no_waypoint_reached_message_while_ship_is_short_of_its_waypoint() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        spawn_patrolling_npc(&mut app, 500.0, 500.0, "npc-1", "patrol", &["wp0"], false);

        tick_with_dt(&mut app, 0.1);

        let messages = app
            .world()
            .resource::<bevy::ecs::message::Messages<AiWaypointReached>>();
        let mut cursor = messages.get_cursor();
        assert_eq!(
            cursor.read(messages).count(),
            0,
            "no arrival must be announced while the ship is still 200 units out"
        );
    }

    /// Issue #968: the low-LOD hazard assessment reasons about the ship's OWN
    /// hull and its OWN authored standoff, not about a point with the module
    /// default.
    ///
    /// `low_lod_avoid_yaw` used to build its `WorldView` from
    /// `WorldView::default()`, which zeroes `self_radius`, and to pass the
    /// parse-default `AVOIDANCE_BUFFER` whatever the hull authored. Both push a
    /// demoted ship's reaction late by exactly the margin it needed: the two
    /// Harrow pickets in `combat_test` ground 3.2 units into radius-4 rocks,
    /// were kicked back to the surface once a second by the collision response,
    /// and drove straight back in.
    ///
    /// The obstacle here sits 10 units off the ship's 3-second projection, so it
    /// is outside a point-model's `0 + 4 + 5` avoidance radius and inside the
    /// radius a 3-unit hull (or a hull authoring a wider buffer) actually needs.
    #[test]
    fn low_lod_avoidance_uses_the_hulls_own_radius_and_authored_buffer() {
        const ROCK_RADIUS: f32 = 4.0;
        const DEFAULT_BUFFER: f32 = 5.0;

        let snapshot = WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::from_u128(1),
                // 10 units from the projected position (0, 0, -9): dx = 6, dz = -8.
                position: [6.0, 0.0, -17.0],
                radius: ROCK_RADIUS,
                ..Default::default()
            }],
        };
        // Facing -Z at 3 u/s, so the 3-second look-ahead projects to z = -9.
        let avoid = |self_radius: f32, buffer: f32| {
            let behaviour = crate::entity_config::BehaviourConfig {
                avoidance_buffer: buffer,
                ..Default::default()
            };
            low_lod_avoid_yaw(
                0.0,
                [0.0, 0.0, 0.0],
                3.0,
                self_radius,
                uuid::Uuid::from_u128(99),
                &behaviour,
                Some(&snapshot),
            )
        };

        assert_eq!(
            avoid(0.0, DEFAULT_BUFFER),
            0.0,
            "control: as a point with the default buffer, this obstacle is out of \
             range and the ship holds its heading"
        );
        assert_ne!(
            avoid(3.0, DEFAULT_BUFFER),
            0.0,
            "a 3-unit hull needs 3 more units of clearance than a point, and must \
             bend its heading for an obstacle a point would ignore"
        );
        assert_ne!(
            avoid(0.0, 20.0),
            0.0,
            "a hull authoring a wider avoidance buffer must react earlier at low \
             fidelity too — the authored value has to reach this path"
        );
    }

    /// Issue #968: the dead-reckoned deviation ceiling is AUTHORED, and the
    /// deviation actually applied scales with threat up to it.
    ///
    /// The 90° default is geometry (the tangent at contact), but the ramp to it
    /// is not — `threat × ceiling` is a proportional bend, not the tangent angle
    /// for the distance in hand — so a hull may author its own. This also pins
    /// what the constant's note warns about: the magnitude no longer passes
    /// through the hull's `max_yaw_rate` at all.
    #[test]
    fn the_low_lod_deviation_ceiling_is_authored_per_hull() {
        // Directly ahead and close enough to saturate: the ship's projected
        // point (0, 0, -9) is well inside a radius-4 rock's skin at z = -10,
        // so the threat fraction is 1.0 and the deviation is the whole ceiling.
        let snapshot = WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::from_u128(1),
                position: [0.5, 0.0, -10.0],
                radius: 4.0,
                ..Default::default()
            }],
        };
        let avoid = |ceiling: f32| {
            let behaviour = crate::entity_config::BehaviourConfig {
                low_lod_avoidance_deviation_rad: ceiling,
                ..Default::default()
            };
            low_lod_avoid_yaw(
                0.0,
                [0.0, 0.0, 0.0],
                3.0,
                1.0,
                uuid::Uuid::from_u128(99),
                &behaviour,
                Some(&snapshot),
            )
        };

        let default_turn = avoid(crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD);
        assert!(
            (default_turn.abs() - crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD).abs() < 1e-4,
            "a saturated hazard must bend the heading by the whole authored \
             ceiling, got {default_turn}"
        );

        // Half the ceiling, same geometry: the authored value is what scales it.
        let half = avoid(crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD / 2.0);
        assert!(
            (half.abs() - crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD / 2.0).abs() < 1e-4,
            "a hull authoring half the ceiling must deviate half as far, got {half}"
        );
        assert_eq!(
            half.signum(),
            default_turn.signum(),
            "the authored ceiling must scale the turn, not reverse it"
        );

        // A hull that authors no deviation at all holds its route bearing.
        assert_eq!(
            avoid(0.0),
            0.0,
            "a zero ceiling means a hull that never leaves its route"
        );
    }

    #[test]
    fn low_lod_without_patrol_objective_keeps_dumb_forward_move() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 0.0, 0.0);

        // NPC carries ObjectiveCursors + an (empty) blackboard but NO patrol
        // objective — it must fall back to the pre-existing dumb forward-move.
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(500.0, 0.0, 0.0),
                ShipPhysics {
                    x: 500.0,
                    z: 0.0,
                    forward_speed: 10.0,
                    yaw: 0.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 100.0,
                    ..Default::default()
                },
                ObjectiveCursors::default(),
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();

        let initial = *app.world().get::<ShipPhysics>(npc).unwrap();
        tick_with_dt(&mut app, 0.1);
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();

        // yaw=0, forward_speed=10, dt=0.1 → z advances by -1, x unchanged.
        assert!(
            (physics.z - (initial.z - 1.0)).abs() < 0.001,
            "no-patrol NPC z should advance by forward_speed * dt: expected {}, got {}",
            initial.z - 1.0,
            physics.z,
        );
        assert!(
            (physics.x - initial.x).abs() < 0.001,
            "no-patrol NPC x should not change when yaw=0: expected {}, got {}",
            initial.x,
            physics.x,
        );
        // Cursor stays empty — nothing was advanced.
        assert!(
            app.world()
                .get::<ObjectiveCursors>(npc)
                .unwrap()
                .0
                .is_empty(),
            "cursor must stay empty when there is no patrol objective"
        );
    }
}
