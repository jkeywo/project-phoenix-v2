pub use phoenix_sim_gameplay::entities::ai_flag_hosts::*;

#[cfg(test)]
#[path = "ai_flag_hosts_tests.rs"]
mod tests;

#[cfg(test)]
use crate::world::flags::{FactContext, FactId};
