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
