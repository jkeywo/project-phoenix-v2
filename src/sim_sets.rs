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
