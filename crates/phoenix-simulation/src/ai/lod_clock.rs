//! LOD uses the shared logical clock; historical timestamps remain canonical.
pub(crate) use phoenix_sim_contracts::logical_clock::sample;
#[cfg(test)]
use {bevy::prelude::*, phoenix_sim_contracts::logical_clock::from_world};

#[cfg(test)]
#[path = "lod_clock_tests.rs"]
mod tests;
