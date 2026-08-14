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

/// Opt-in marker: this entity belongs on the hail roster (issue #985).
///
/// Attached by [`crate::entities::spawner`] when the entity's `[comms]` block
/// sets `hailable = true`. Carrying [`CommsRange`] alone is NOT enough — every
/// shipped warship and station declares a range (that is what makes them
/// range-gated senders), and treating all of them as hailable would rewrite the
/// contact roster of every shipped world. The flag is the deliberate opt-in;
/// see [`crate::comms::roster`] for the dual-source rule it feeds.
///
/// `display_name` carries the optional `[comms] display_name` — authored
/// player-facing text for the contact row, independent of the entity's `name`
/// reference id (mirrors `CommsTemplate::display_name`, issue #751).
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct CommsHailable {
    /// Authored player-facing label; `None` falls back to `EntityName`, then
    /// the raw UUID. See [`crate::comms::roster::entity_contact_label`].
    pub display_name: Option<String>,
}
