//! The Weapons console (issue #1186): a pure module whose Bevy plugin, system
//! registration, and server-side adapter code live in the `server` sibling,
//! matching the pure-module + `server.rs` shape the other consoles follow. The
//! weapon-family submodules (`beam`, `blaster`, `torpedo`), the shared utilities
//! (`shared`), and the blackboard publishers (`blackboard`) stay declared here;
//! everything the `server` adapter exposes is re-exported so
//! `crate::console::weapons::X` paths keep resolving.

pub mod beam;
pub mod blackboard;
pub mod blaster;
pub mod server;
pub mod shared;
pub mod torpedo;

pub use server::*;
