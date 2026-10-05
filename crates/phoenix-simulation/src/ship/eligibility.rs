pub use phoenix_sim_gameplay::ship::eligibility::*;

#[cfg(test)]
#[path = "eligibility_tests.rs"]
mod tests;

#[cfg(test)]
use crate::ship::config::ShipConfig;
#[cfg(test)]
use crate::ship::system_registry as kinds;
