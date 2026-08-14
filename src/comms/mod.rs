//! The Comms concept, consolidated (issue #816): pure evaluators and runtime
//! state in `content`, the thin Bevy applier + `CommsWorldPlugin` in `server`,
//! plus the pure `in_range` distance check (`range`) and the Bevy
//! `CommsRange` marker component (`component`) attached to entities that
//! declare a `[comms]` block in their TOML, plus the pure hail-roster
//! derivation (`roster`) that unions entity-derived contacts into the
//! declarative `[[comms]]` roster (issue #985).

pub mod component;
pub mod content;
pub mod range;
pub mod roster;
pub mod server;

pub use component::{CommsHailable, CommsRange};
pub use range::in_range;
pub use roster::{entity_contact_label, merge_entity_contacts, EntityContact};
pub use server::CommsWorldPlugin;
