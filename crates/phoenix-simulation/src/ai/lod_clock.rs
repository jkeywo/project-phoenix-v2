//! LOD transitions belong to the logical run, not a fresh host's Bevy epoch.
use bevy::prelude::*;

fn valid_period(period: Option<f64>) -> Option<f64> {
    period.filter(|period| period.is_finite() && *period > 0.0)
}

pub(crate) fn sample(tick: Option<u64>, period: Option<f64>, fallback: f64) -> f64 {
    tick.zip(valid_period(period))
        .map_or(fallback, |(tick, period)| tick as f64 * period)
}

pub(crate) fn from_world(world: &World) -> f64 {
    let fixed = world.get_resource::<Time<Fixed>>();
    sample(
        world
            .get_resource::<crate::sim_tick::SimTick>()
            .map(|tick| tick.0),
        fixed.map(|time| time.timestep().as_secs_f64()),
        fallback_clock(
            world,
            fixed.map_or(0.0, |time| f64::from(time.elapsed_secs())),
        ),
    )
}

pub(crate) fn restore_timestamp(world: &World, tick: u64, age: f64, fallback: f64) -> f64 {
    let period = world
        .get_resource::<Time<Fixed>>()
        .map(|time| time.timestep().as_secs_f64());
    if world.contains_resource::<crate::sim_tick::SimTick>() {
        if let Some(period) = valid_period(period) {
            // Actual transitions occur on fixed ticks. Recover their historical
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
