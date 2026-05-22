//! Comms range: a pure `in_range` distance check (`range`) and the Bevy
//! `CommsRange` marker component (`component`) attached to entities that
//! declare a `[comms]` block in their TOML.

pub mod range;
pub mod component;

pub use component::CommsRange;
pub use range::in_range;
