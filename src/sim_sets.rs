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
    Broadcast,
}

/// Ordering label within `SimSet::Physics`: ensure player ship position
/// is synced to Transform before the AI tick reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct AiTickLabel;
