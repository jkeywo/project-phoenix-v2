//! The Comms concept, consolidated (issue #816): pure evaluators and runtime
//! state in `content`, the thin Bevy applier + `CommsWorldPlugin` in `server`,
//! plus the pure `in_range` distance check (`range`) and the Bevy
//! `CommsRange` marker component (`component`) attached to entities that
//! declare a `[comms]` block in their TOML, plus the pure hail-roster
//! derivation (`roster`) that unions entity-derived contacts into the
//! declarative `[[comms]]` roster (issue #985).
//!
//! `scripted` is the Rhai front-end's half of the applier (issue #984): it
//! materialises `ctx.effects.open_comms(#{…})` requests into live threads. It
//! is kept apart from `server` because the M7 collapse deletes the declarative
//! front-end and this module is what survives it.
//!
//! `ai_choice` is the pure half of the delayed weighted Backfill picker (issue
//! #1343): what an unmanned console may reach for on a live dialogue node, and
//! how one uniform draw becomes the response index it submits. The Bevy half is
//! `console::comms::server::operate_comms_response_ai`.

pub mod ai_choice;
pub mod capture;
pub(crate) mod capture_ai;
pub mod component;
pub mod content;
pub mod formation;
pub mod jamming;
pub mod media_control;
pub mod media_edge;
pub mod media_mesh;
pub mod media_session;
pub(crate) mod planned;
pub mod radio;
pub mod range;
pub(crate) mod reception;
mod request_returns;
pub(crate) mod requests;
pub mod roster;
pub mod scripted;
pub mod server;

pub use component::{CommsHailable, CommsRange};
pub use range::in_range;
pub use roster::{entity_contact_label, merge_entity_contacts, EntityContact};
pub use server::CommsWorldPlugin;

#[cfg(test)]
mod fleet_hail_tests;
