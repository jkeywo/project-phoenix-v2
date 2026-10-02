use super::*;

#[test]
fn new_starts_idle_with_full_battery() {
    let s = BoostState::new();
    assert!(!s.active);
    assert!((s.battery - 1.0).abs() < f32::EPSILON);
}

#[test]
fn toggle_engages_when_battery_available() {
    let mut s = BoostState::new();
    s.toggle();
    assert!(s.is_active());
}

#[test]
fn toggle_disengages_when_active() {
    let mut s = BoostState::new();
    s.toggle();
    s.toggle();
    assert!(!s.is_active());
}

#[test]
fn toggle_does_not_engage_on_empty_battery() {
    let mut s = BoostState::new();
    s.battery = 0.0;
    s.toggle();
    assert!(!s.is_active(), "must not engage with an empty battery");
}

#[test]
fn tick_drains_battery_while_active() {
    let mut s = BoostState::new();
    s.toggle();
    s.tick(
        BOOST_ACTIVE_DURATION / 2.0,
        BOOST_ACTIVE_DURATION,
        BOOST_RECHARGE_DURATION,
    );
    assert!((s.battery - 0.5).abs() < 0.001);
    assert!(s.is_active());
}

#[test]
fn tick_auto_disengages_when_battery_empties() {
    let mut s = BoostState::new();
    s.toggle();
    s.tick(
        BOOST_ACTIVE_DURATION,
        BOOST_ACTIVE_DURATION,
        BOOST_RECHARGE_DURATION,
    );
    assert!((s.battery).abs() < f32::EPSILON);
    assert!(!s.is_active(), "should auto-disengage at empty");
}

#[test]
fn battery_never_goes_negative() {
    let mut s = BoostState::new();
    s.toggle();
    s.tick(
        BOOST_ACTIVE_DURATION * 5.0,
        BOOST_ACTIVE_DURATION,
        BOOST_RECHARGE_DURATION,
    );
    assert!(s.battery >= 0.0);
    assert!((s.battery).abs() < f32::EPSILON);
}

#[test]
fn tick_recharges_battery_while_idle() {
    let mut s = BoostState::new();
    s.battery = 0.0;
    s.tick(
        BOOST_RECHARGE_DURATION / 2.0,
        BOOST_ACTIVE_DURATION,
        BOOST_RECHARGE_DURATION,
    );
    assert!((s.battery - 0.5).abs() < 0.001);
    assert!(!s.is_active());
}

#[test]
fn recharge_clamps_at_full() {
    let mut s = BoostState::new();
    s.battery = 0.0;
    s.tick(
        BOOST_RECHARGE_DURATION * 5.0,
        BOOST_ACTIVE_DURATION,
        BOOST_RECHARGE_DURATION,
    );
    assert!((s.battery - 1.0).abs() < f32::EPSILON);
}

#[test]
fn can_reengage_with_partial_battery() {
    let mut s = BoostState::new();
    s.toggle();
    s.tick(
        BOOST_ACTIVE_DURATION / 2.0,
        BOOST_ACTIVE_DURATION,
        BOOST_RECHARGE_DURATION,
    );
    s.toggle(); // disengage at 50%
    assert!(!s.is_active());
    s.toggle(); // re-engage on partial battery
    assert!(s.is_active());
}

#[test]
fn non_positive_durations_fall_back_to_consts() {
    let mut s = BoostState::new();
    s.toggle();
    // active_duration = 0 → falls back to BOOST_ACTIVE_DURATION
    s.tick(BOOST_ACTIVE_DURATION / 2.0, 0.0, 0.0);
    assert!(
        (s.battery - 0.5).abs() < 0.001,
        "expected fallback drain rate"
    );

    s.toggle(); // idle
    s.tick(BOOST_RECHARGE_DURATION / 2.0, 0.0, 0.0);
    // 0.5 + 0.5 = 1.0
    assert!(
        (s.battery - 1.0).abs() < 0.001,
        "expected fallback recharge rate"
    );
}

#[test]
fn active_boost_does_not_drain_when_demand_is_zero() {
    let mut s = BoostState::new();
    s.toggle();
    s.tick_with_drain_factor(
        BOOST_ACTIVE_DURATION,
        BOOST_ACTIVE_DURATION,
        BOOST_RECHARGE_DURATION,
        0.0,
    );
    assert!((s.battery - 1.0).abs() < f32::EPSILON);
    assert!(s.is_active());
}

#[test]
fn active_boost_drains_twice_as_fast_at_full_thrust_and_steering() {
    let mut s = BoostState::new();
    s.toggle();
    s.tick_with_drain_factor(
        BOOST_ACTIVE_DURATION / 4.0,
        BOOST_ACTIVE_DURATION,
        BOOST_RECHARGE_DURATION,
        2.0,
    );
    assert!((s.battery - 0.5).abs() < 0.001);
    assert!(s.is_active());
}
