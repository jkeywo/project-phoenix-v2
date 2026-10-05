//! The Navigation console (issue #1186): a pure module whose Bevy plugin,
//! system registration, and server-side adapter code live in the `server`
//! sibling, matching the pure-module + `server.rs` shape the other consoles
//! follow. Everything is re-exported so `crate::console::navigation::X` paths
//! keep resolving.

pub mod server;

pub use server::*;
