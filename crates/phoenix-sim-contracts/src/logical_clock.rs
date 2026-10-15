//! Shared clock for fixed-step historical timestamps across host continuations.
use bevy::prelude::*;

pub fn sample(tick: Option<u64>, period: Option<f64>, fallback: f64) -> f64 {
    tick.zip(period.filter(|period| period.is_finite() && *period > 0.0))
        .map_or(fallback, |(tick, period)| tick as f64 * period)
}

pub fn elapsed_secs(
    tick: Option<&crate::sim_tick::SimTick>,
    fixed: Option<&Time<Fixed>>,
    fallback: f32,
) -> f32 {
    sample(
        tick.map(|tick| tick.0),
        fixed.map(|time| time.timestep().as_secs_f64()),
        f64::from(fallback),
    ) as f32
}

pub fn from_world(world: &World) -> f64 {
    let fixed = world.get_resource::<Time<Fixed>>();
    sample(
        world
            .get_resource::<crate::sim_tick::SimTick>()
            .map(|tick| tick.0),
        fixed.map(|time| time.timestep().as_secs_f64()),
        world.get_resource::<Time>().map_or_else(
            || fixed.map_or(0.0, |time| f64::from(time.elapsed_secs())),
            |time| f64::from(time.elapsed_secs()),
        ),
    )
}
