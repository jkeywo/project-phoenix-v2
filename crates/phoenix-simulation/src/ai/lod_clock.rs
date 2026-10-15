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
        fixed.map_or(0.0, |time| f64::from(time.elapsed_secs())),
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
    fallback - age
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_transition_age_continues_exactly_across_different_host_epochs() {
        let mut world = World::new();
        let period = crate::sim_tick::sim_tick_period(60.0);
        world.insert_resource(Time::<Fixed>::from_duration(period));
        world.insert_resource(crate::sim_tick::SimTick(103));
        let last = sample(Some(1), Some(period.as_secs_f64()), 0.0);
        let age = from_world(&world) - last;
        let restored = restore_timestamp(&world, 103, age, 700.0);
        assert_eq!(restored.to_bits(), last.to_bits());
        for tick in 104..464 {
            world.resource_mut::<crate::sim_tick::SimTick>().0 = tick;
            assert_eq!(
                (from_world(&world) - last).to_bits(),
                (from_world(&world) - restored).to_bits()
            );
        }
    }

    #[test]
    fn bare_fixtures_keep_elapsed_clock_and_invalid_periods_cannot_drive_logical_time() {
        for period in [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(sample(Some(100), period, 1.25), 1.25);
        }
        assert_eq!(sample(None, Some(0.1), 1.25), 1.25);
        assert_eq!(restore_timestamp(&World::new(), 100, 0.75, 1.25), 0.5);
    }
}
