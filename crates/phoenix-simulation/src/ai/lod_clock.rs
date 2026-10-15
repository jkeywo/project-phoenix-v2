//! Fixed-tick historical events belong to the run, not a fresh host's epoch.
use bevy::prelude::*;
pub(crate) use phoenix_sim_contracts::logical_clock::{from_world, sample};

fn valid_period(period: Option<f64>) -> Option<f64> {
    period.filter(|period| period.is_finite() && *period > 0.0)
}

pub(crate) fn restore_timestamp(world: &World, tick: u64, age: f64, fallback: f64) -> f64 {
    let period = world
        .get_resource::<Time<Fixed>>()
        .map(|time| time.timestep().as_secs_f64());
    if world.contains_resource::<crate::sim_tick::SimTick>() {
        if let Some(period) = valid_period(period) {
            // Actual historical events occur on fixed ticks. Recover their
            // tick rather than lose its low bits through (now - (now - then)).
            // This does not round the authored dwell budget or elapsed work.
            return libm::round(tick as f64 - age / period) * period;
        }
    }
    fallback_clock(world, fallback) - age
}

fn fallback_clock(world: &World, fallback: f64) -> f64 {
    world
        .get_resource::<Time>()
        .map_or(fallback, |time| f64::from(time.elapsed_secs()))
}

#[cfg(test)]
#[path = "lod_clock_tests.rs"]
mod tests;
