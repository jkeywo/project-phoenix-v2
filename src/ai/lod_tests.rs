use super::*;

// ── Promotion (Low → High) ──────────────────────────────────────────────

#[test]
fn promote_when_distance_within_range() {
    let result = evaluate_lod(LodState::Low, 50.0, 100.0, 0.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::High);
}

#[test]
fn no_promotion_when_distance_exceeds_range() {
    let result = evaluate_lod(LodState::Low, 150.0, 100.0, 0.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::Low);
}

#[test]
fn no_promotion_when_distance_equals_sensor_range_times_12() {
    let result = evaluate_lod(LodState::Low, 120.0, 100.0, 100.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::Low);
}

// ── Demotion (High → Low) ───────────────────────────────────────────────

#[test]
fn demote_when_distance_exceeds_hysteresis_and_dwell_met() {
    let result = evaluate_lod(LodState::High, 200.0, 100.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::Low);
}

#[test]
fn no_demotion_when_distance_exceeds_threshold_but_dwell_not_met() {
    let result = evaluate_lod(LodState::High, 200.0, 100.0, 1.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::High);
}

#[test]
fn no_demotion_when_distance_within_sensor_range_even_if_dwell_met() {
    let result = evaluate_lod(LodState::High, 50.0, 100.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::High);
}

#[test]
fn no_demotion_when_distance_in_hysteresis_band() {
    let result = evaluate_lod(LodState::High, 110.0, 100.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::High);
}

#[test]
fn stay_high_when_distance_exactly_at_sensor_range() {
    let result = evaluate_lod(LodState::High, 100.0, 100.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::High);
}

// ── Edge cases ──────────────────────────────────────────────────────────

#[test]
fn zero_distance_promotes_to_high() {
    let result = evaluate_lod(LodState::Low, 0.0, 100.0, 0.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::High);
}

#[test]
fn very_large_distance_stays_low_or_demotes() {
    let low_result = evaluate_lod(LodState::Low, 1e8, 100.0, 0.0, 0.0, 2.0, 0.2);
    assert_eq!(low_result, LodState::Low);

    let high_result = evaluate_lod(LodState::High, 1e8, 100.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(high_result, LodState::Low);
}

// ── Dwell timer semantics ───────────────────────────────────────────────

#[test]
fn promotion_resets_dwell_timer_for_subsequent_demotion() {
    // Promote to High — this implies a state change at now_secs = 10.0
    let promoted = evaluate_lod(LodState::Low, 50.0, 100.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(promoted, LodState::High);

    // At the same timestamp, demotion should not happen because
    // dwell_elapsed = (10.0 - 10.0) = 0.0 which is < 2.0
    // Actually — the *caller* is responsible for storing
    // last_state_change_secs as the time the promotion happened.
    // The function itself doesn't track it. This test demonstrates
    // that after an immediate promotion, if we call again with
    // last_state_change_secs = 10.0 (the promotion time), demotion
    // is correctly blocked.
    let still_high = evaluate_lod(LodState::High, 200.0, 100.0, 10.0, 10.0, 2.0, 0.2);
    assert_eq!(still_high, LodState::High);

    // After dwell_secs have passed, demotion is allowed.
    let demoted = evaluate_lod(LodState::High, 200.0, 100.0, 12.0, 10.0, 2.0, 0.2);
    assert_eq!(demoted, LodState::Low);
}

#[test]
fn dwell_secs_zero_allows_immediate_demotion() {
    let result = evaluate_lod(LodState::High, 200.0, 100.0, 5.0, 5.0, 0.0, 0.2);
    assert_eq!(result, LodState::Low);
}

// ── Boundary: exactly at thresholds ─────────────────────────────────────

#[test]
fn exactly_at_sensor_range_from_low_promotes() {
    let result = evaluate_lod(LodState::Low, 100.0, 100.0, 0.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::High);
}

#[test]
fn exactly_at_demote_threshold_from_high_with_dwell_stays_high() {
    let result = evaluate_lod(LodState::High, 120.0, 100.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::High);
}

#[test]
fn just_above_demote_threshold_with_dwell_demotes() {
    let result = evaluate_lod(LodState::High, 120.001, 100.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::Low);
}

// ── Malformed sensor_range: safe defaults (zero/negative/non-finite) ────
//
// A malformed `AiProfile.sensor_range` must fail toward the cheap
// Low-fidelity path — never permanently stuck High (the way a naive NaN
// comparison would produce), and never permanently promoted (the way a
// naive +infinity comparison would produce).

#[test]
fn zero_sensor_range_never_promotes() {
    let result = evaluate_lod(LodState::Low, 1.0, 0.0, 0.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::Low);
}

#[test]
fn zero_sensor_range_demotes_a_high_ship_once_dwell_elapses() {
    let result = evaluate_lod(LodState::High, 1.0, 0.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::Low);
}

#[test]
fn negative_sensor_range_never_promotes() {
    let result = evaluate_lod(LodState::Low, 1.0, -50.0, 0.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::Low);
}

#[test]
fn negative_sensor_range_demotes_a_high_ship_once_dwell_elapses() {
    let result = evaluate_lod(LodState::High, 1.0, -50.0, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::Low);
}

#[test]
fn nan_sensor_range_never_promotes() {
    let result = evaluate_lod(LodState::Low, 1.0, f32::NAN, 0.0, 0.0, 2.0, 0.2);
    assert_eq!(
        result,
        LodState::Low,
        "a NaN sensor range must not promote (fails closed to Low)"
    );
}

#[test]
fn nan_sensor_range_does_not_leave_a_high_ship_stuck_forever() {
    // A naive `distance > (NaN * 1.2)` demote check is always false
    // (NaN comparisons never succeed), which would pin the ship High
    // forever. The sanitizing guard must prevent that.
    let result = evaluate_lod(LodState::High, 1.0, f32::NAN, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(
        result,
        LodState::Low,
        "a NaN sensor range must still allow demotion once dwell elapses"
    );
}

#[test]
fn positive_infinity_sensor_range_does_not_pin_every_ship_high_forever() {
    // A naive `distance <= infinity` promote check and
    // `distance > infinity` demote check would promote everything and
    // demote nothing, permanently pinning every ship High. The
    // sanitizing guard must prevent that too.
    let promoted = evaluate_lod(LodState::Low, 1.0, f32::INFINITY, 0.0, 0.0, 2.0, 0.2);
    assert_eq!(
        promoted,
        LodState::Low,
        "+infinity sensor range must not promote (fails closed to Low)"
    );

    let demoted = evaluate_lod(LodState::High, 1.0, f32::INFINITY, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(
        demoted,
        LodState::Low,
        "+infinity sensor range must still allow demotion once dwell elapses"
    );
}

#[test]
fn negative_infinity_sensor_range_behaves_like_zero() {
    let result = evaluate_lod(LodState::High, 1.0, f32::NEG_INFINITY, 10.0, 0.0, 2.0, 0.2);
    assert_eq!(result, LodState::Low);
}

// ── decay_speed_toward (issue #933) ─────────────────────────────────────

#[test]
fn decay_speed_toward_steps_down_toward_a_lower_target() {
    let result = decay_speed_toward(100.0, 20.0, 8.0, 1.0);
    assert_eq!(result, 92.0);
}

#[test]
fn decay_speed_toward_does_not_overshoot_past_the_target() {
    let result = decay_speed_toward(22.0, 20.0, 8.0, 1.0);
    assert_eq!(
        result, 20.0,
        "one big step must clamp at the target, not undershoot below it"
    );
}

#[test]
fn decay_speed_toward_ramps_up_toward_a_higher_target() {
    let result = decay_speed_toward(0.0, 20.0, 8.0, 1.0);
    assert_eq!(result, 8.0);
}

#[test]
fn decay_speed_toward_holds_steady_once_at_target() {
    let result = decay_speed_toward(20.0, 20.0, 8.0, 1.0);
    assert_eq!(result, 20.0);
}

#[test]
fn decay_speed_toward_many_ticks_converges_on_target() {
    let mut speed = 100.0f32;
    for _ in 0..700 {
        speed = decay_speed_toward(speed, 20.0, 8.0, 1.0 / 60.0);
    }
    assert!(
        (speed - 20.0).abs() < 1.0,
        "expected convergence toward 20.0 within ~700 ticks (~11.7s at 8 units/s^2), got {speed}"
    );
}

#[test]
fn decay_speed_toward_zero_rate_is_a_no_op() {
    let result = decay_speed_toward(100.0, 20.0, 0.0, 1.0);
    assert_eq!(
        result, 100.0,
        "a malformed zero rate must not snap the speed"
    );
}

#[test]
fn decay_speed_toward_negative_rate_is_a_no_op() {
    let result = decay_speed_toward(100.0, 20.0, -5.0, 1.0);
    assert_eq!(result, 100.0);
}

#[test]
fn decay_speed_toward_zero_dt_is_a_no_op() {
    let result = decay_speed_toward(100.0, 20.0, 8.0, 0.0);
    assert_eq!(result, 100.0);
}

// ── step_yaw_toward (issue #933) ─────────────────────────────────────────

#[test]
fn step_yaw_toward_turns_toward_a_nearby_target_without_overshoot() {
    let result = step_yaw_toward(0.0, 0.1, 1.0);
    assert!(
        (result - 0.1).abs() < 1e-6,
        "small delta within max_step must land exactly on target"
    );
}

#[test]
fn step_yaw_toward_clamps_to_max_step_for_a_large_delta() {
    let result = step_yaw_toward(0.0, 3.0, 0.5);
    assert!((result - 0.5).abs() < 1e-6);
}

#[test]
fn step_yaw_toward_takes_the_shorter_way_around_the_wrap() {
    // From just past +PI to just past -PI is a tiny step the "short way"
    // across the wrap, not a near-2*PI step the "long way".
    let current = std::f32::consts::PI - 0.05;
    let desired = -std::f32::consts::PI + 0.05;
    let result = step_yaw_toward(current, desired, 1.0);
    // Expect it to have advanced past PI (wrapping) rather than swinging
    // all the way back down toward 0.
    let two_pi = std::f32::consts::TAU;
    let mut delta = (desired - result) % two_pi;
    if delta > std::f32::consts::PI {
        delta -= two_pi;
    } else if delta < -std::f32::consts::PI {
        delta += two_pi;
    }
    assert!(
        delta.abs() < 0.06,
        "expected to have nearly reached desired via the short way, remaining delta {delta}"
    );
}

#[test]
fn step_yaw_toward_many_ticks_converges_on_desired() {
    let mut yaw = 0.0f32;
    let desired = std::f32::consts::FRAC_PI_2;
    for _ in 0..200 {
        yaw = step_yaw_toward(yaw, desired, 0.02);
    }
    assert!(
        (yaw - desired).abs() < 1e-3,
        "expected convergence, got {yaw}"
    );
}

#[test]
fn step_yaw_toward_zero_max_step_is_a_no_op() {
    let result = step_yaw_toward(0.5, 2.0, 0.0);
    assert_eq!(
        result, 0.5,
        "a malformed zero turn rate must not snap the heading"
    );
}

#[test]
fn step_yaw_toward_negative_max_step_is_a_no_op() {
    let result = step_yaw_toward(0.5, 2.0, -1.0);
    assert_eq!(result, 0.5);
}
