use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub enum SimSet {
    Input,
    Physics,
    Damage,
    Modifiers,
    Broadcast,
}
