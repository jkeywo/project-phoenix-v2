/// The promises a captain makes, and whether they end up kept (issue #1029) —
/// a pure record whose resolution writes campaign flags, carrying no queue and
/// no evaluator of its own.
pub mod commitments;
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
