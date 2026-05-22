//! Bevy `CommsRange` component — attached to entities that declare a
//! `[comms]` section in their TOML.
//!
//! Kept in a sibling file from `range.rs` so the pure math module remains
//! Bevy-free.

use bevy::prelude::Component;

/// Comms range in world units. Attached by [`crate::entities::spawner`] when
/// the source [`crate::entities::config::CommsConfig`] is present.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct CommsRange(pub f32);
