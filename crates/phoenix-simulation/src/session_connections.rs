pub use phoenix_sim_session::session_connections::*;

pub mod browser;

#[cfg(test)]
#[path = "session_connections_tests.rs"]
mod tests;

#[cfg(test)]
use crate::lobby::handler::Target;
