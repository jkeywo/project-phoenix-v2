use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub enum SimSet {
    Input,
    Physics,
    Damage,
    Modifiers,
    /// Phase 1a: every system writes its own blackboard from current ECS state.
    /// Runs after Modifiers so blackboards reflect the fully-updated sim state.
    /// Cross-system reads during Physics/Damage/Modifiers use `FrozenBlackboards`
    /// (last tick's snapshot) for determinism.
    Publish,
    /// Phase 1b: ship-wide aggregators read all phase-1a blackboards and write
    /// cross-system views (e.g. the Viewscreen blackboard). Strictly after Publish.
    PublishAggregate,
    Broadcast,
}

/// Ordering label within `SimSet::Physics` marking the AI decision phase:
/// `build_world_snapshot` runs just before it, `operate_helm_ai` /
/// `process_attacker_this_tick` run in/after it. `sync_ship_position` is
/// ordered `.after(process_helm_inputs)`/`.after(operate_helm_ai)` (not
/// relative to this label) so `Transform` reflects this tick's freshly
/// computed `ShipPhysics` rather than a stale pre-movement value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct AiTickLabel;
