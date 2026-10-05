pub use phoenix_sim_session::lobby::session::*;

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

#[cfg(test)]
use crate::{core::messages::StationId, ship::config::ShipConfig};
