//! The Comms concept, consolidated (issue #816): pure evaluators and runtime
//! state in `content`, the thin Bevy applier + `CommsWorldPlugin` in `server`,
//! plus the pure `in_range` distance check (`range`) and the Bevy
//! `CommsRange` marker component (`component`) attached to entities that
//! declare a `[comms]` block in their TOML.

pub mod component;
pub mod content;
pub mod range;
pub mod server;

pub use component::CommsRange;
pub use range::in_range;
pub use server::CommsWorldPlugin;
