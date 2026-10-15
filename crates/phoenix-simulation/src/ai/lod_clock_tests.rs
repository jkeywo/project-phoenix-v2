use super::*;

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
}

#[test]
fn actual_world_without_logical_clock_uses_the_same_generic_time_as_lod() {
    let mut world = World::new();
    let mut clock = Time::<()>::default();
    clock.advance_by(std::time::Duration::from_secs(10));
    world.insert_resource(clock);
    assert_eq!(from_world(&world), 10.0);

    let mut fixed = Time::<Fixed>::default();
    fixed.advance_by(std::time::Duration::from_secs(3));
    world.insert_resource(fixed);
    assert_eq!(
        from_world(&world),
        10.0,
        "a different Fixed epoch cannot replace the consumer's actual generic clock"
    );
    world.remove_resource::<Time>();
    assert_eq!(
        from_world(&world),
        3.0,
        "Fixed-only capture fixtures retain their supported fallback"
    );
}
