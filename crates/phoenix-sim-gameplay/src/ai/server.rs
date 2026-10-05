use bevy::prelude::*;
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
/// enough to the action to simulate in full" a property of *anchors*: every
/// frozen-roster fleet ship always projects one (at
/// [`DEFAULT_FLEET_LOD_BUBBLE_RADIUS`] unless it authors its own), and a
/// stationary defended object like the station projects a smaller one, so the
/// raid sieging it — and the station's own guns — run in full whether or not the
/// player is looking. Authored as `[lod_bubble] radius = N`.
#[derive(Component, Clone, Copy, Debug)]
pub struct LodBubble {
    pub radius: f32,
}

/// The bubble radius a frozen-roster fleet ship projects when it authors no
/// `[lod_bubble]` of its own — every crewed hull is an anchor on every peer
/// without repeating the block. Generous enough to cover a normal engagement so
/// an NPC closing on any fleet ship is in full fidelity before it opens fire;
/// deliberately wider than the old per-NPC `sensor_range` promotion, which is
/// what re-timed far-from-player combats (`probe_despawn`'s duel gains its
/// natural second kill once both hulls run in full).
pub const DEFAULT_FLEET_LOD_BUBBLE_RADIUS: f32 = 600.0;

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

/// Snapshot of all ship/world-entity positions built once per tick.
/// All `operate_*_ai` handlers read from this resource rather than building
/// their own queries.
#[derive(Resource, Default)]
pub struct WorldSnapshot {
    pub entities: Vec<crate::ai::AiWorldEntity>,
}
