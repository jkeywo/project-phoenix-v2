pub mod config;
pub mod content;
/// Named, inspectable, mutable mission deadlines (issue #1024) — a *record*
/// layered over the existing `pending_callbacks` deferred-work queue, never a
/// second scheduler.
pub mod deadlines;
pub mod delayed;
pub mod dispatch;
pub mod flags;
pub mod layers;
pub mod manifest;
pub mod mod_pack;
pub mod scenario;
pub mod script;
pub mod server;
pub mod validate;
pub use server::WorldPlugin;
