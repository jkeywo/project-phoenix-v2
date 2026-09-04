use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub enum SimSet {
    Input,
    Physics,
    Damage,
    Modifiers,
    /// Phase 1a: every system writes its own blackboard from current ECS state.
    /// Runs after Modifiers so blackboards reflect the fully-updated sim state.
    /// Blackboards are written exactly once per tick, here. Any cross-system
    /// consumer ordered before Publish (Input/Physics/Damage/Modifiers)
    /// therefore reads the values written on the *previous* tick — the
    /// frozen-snapshot guarantee comes from this set ordering, not from a
    /// separate snapshot type.
    Publish,
    /// Phase 1b: ship-wide aggregators read all phase-1a blackboards and write
    /// cross-system views (e.g. the Viewscreen blackboard). Strictly after Publish.
    PublishAggregate,
    Broadcast,
}

/// Ordering label within `SimSet::Input` marking the point at which this tick's
/// `SetRedAlert` orders have LANDED on every ship's
/// [`ShipRedAlert`](crate::ship::state::ShipRedAlert)
/// (`console::captain::server::handle_set_red_alert` carries it).
///
/// Everything in `SimSet::Input` that reads a ship's alert level to decide
/// authoritative state — the Command station's stance host and its alert-change
/// neutral switch (issue #1107 criterion 5: "the same tick the captain raises
/// it") — must be `.after(RedAlertApplied)`. Without the label those systems sit
/// UNORDERED against the applier, and Bevy resolves an unordered pair by the
/// schedule's topological sort: a pair that happens to fall the right way today
/// falls the other way the moment any unrelated system joins the set, which
/// silently costs the stance host a whole `ai_snapshot_hz` cadence period of
/// staleness and moves the authoritative digest. That is exactly the hazard
/// `tests/archetype_order_determinism.rs` exists to catch, and it caught this
/// one when issue #1346 added the Security System's two `Input` systems.
///
/// A label rather than a `.after(handle_set_red_alert)` on each reader, for
/// [`AiTickLabel`]'s reason: the applier is private to the Captain console, and
/// the contract the readers depend on is "the alert level is current", not the
/// identity of the system that made it so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct RedAlertApplied;

/// Ordering label within `SimSet::Physics` marking the AI decision phase:
/// `build_world_snapshot` runs just before it, the per-axis helm AI
/// (`ai_helm_thrust` / `ai_helm_steering` / `ai_helm_lateral_thrust` /
/// `ai_helm_impulse`) and `process_attacker_this_tick` run in/after it.
/// `sync_ship_position` is ordered `.after(process_helm_inputs)` /
/// `.after(integrate_ship_physics)` (not relative to this label) so `Transform`
/// reflects this tick's freshly computed `ShipPhysics` rather than a stale
/// pre-movement value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct AiTickLabel;
