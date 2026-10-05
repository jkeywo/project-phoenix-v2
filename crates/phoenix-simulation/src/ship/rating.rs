pub use phoenix_sim_gameplay::ship::rating::*;

#[cfg(test)]
#[path = "rating_tests.rs"]
mod tests;

#[cfg(test)]
use crate::core::messages::{StationId, SystemId};
#[cfg(test)]
use crate::ship::config::ShipConfig;
#[cfg(test)]
use crate::ship::control_source::ControlSource;
#[cfg(test)]
use crate::ship::control_source::ControlSourceResolver;
#[cfg(test)]
/// Authored control depth, ordered so a scenario can only raise it.
pub use phoenix_model::wire::SystemDepth;
#[cfg(test)]
use std::collections::HashMap;
