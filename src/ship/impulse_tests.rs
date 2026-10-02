use super::*;

// --- struct + basic API ---

#[test]
fn start_charge_transitions_idle_to_charging() {
    let mut s = ImpulseState::new();
    s.start_charge();
    assert_eq!(s.phase, ImpulsePhase::Charging);
}

#[test]
fn start_charge_is_noop_when_already_charging() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(1.0, IMPULSE_CHARGE_DURATION); // partial progress
    let progress_before = s.charge_progress;
    s.start_charge(); // should not reset progress
    assert_eq!(s.phase, ImpulsePhase::Charging);
    assert!((s.charge_progress - progress_before).abs() < f32::EPSILON);
}

#[test]
fn start_charge_is_noop_when_active() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION); // fully charged → Active
    assert!(s.is_active());
    s.start_charge(); // should stay Active
    assert_eq!(s.phase, ImpulsePhase::Active);
}

#[test]
fn cancel_charge_returns_to_idle_and_resets_progress() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(1.0, IMPULSE_CHARGE_DURATION);
    s.cancel_charge();
    assert_eq!(s.phase, ImpulsePhase::Idle);
    assert!((s.charge_progress).abs() < f32::EPSILON);
}

#[test]
fn cancel_from_active_returns_to_idle() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
    assert!(s.is_active());
    s.cancel_charge();
    assert_eq!(s.phase, ImpulsePhase::Idle);
}

// --- tick / charge progress ---

#[test]
fn tick_while_idle_does_nothing() {
    let mut s = ImpulseState::new();
    s.tick(10.0, IMPULSE_CHARGE_DURATION);
    assert_eq!(s.phase, ImpulsePhase::Idle);
    assert!((s.charge_progress).abs() < f32::EPSILON);
}

#[test]
fn tick_advances_charge_progress() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(IMPULSE_CHARGE_DURATION / 2.0, IMPULSE_CHARGE_DURATION);
    assert!((s.charge_progress - 0.5).abs() < 0.001);
    assert_eq!(s.phase, ImpulsePhase::Charging);
}

#[test]
fn tick_to_full_charge_transitions_to_active() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
    assert_eq!(s.phase, ImpulsePhase::Active);
    assert!((s.charge_progress - 1.0).abs() < f32::EPSILON);
    assert!(s.is_active());
}

#[test]
fn charge_progress_capped_at_one() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(IMPULSE_CHARGE_DURATION * 5.0, IMPULSE_CHARGE_DURATION);
    assert!((s.charge_progress - 1.0).abs() < f32::EPSILON);
}

#[test]
fn custom_charge_duration_charges_at_configured_rate() {
    let custom_duration = 6.0; // twice the default
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(3.0, custom_duration); // half of custom duration → 50%
    assert!((s.charge_progress - 0.5).abs() < 0.001);
    assert_eq!(s.phase, ImpulsePhase::Charging);
    s.tick(3.0, custom_duration); // full custom duration → Active
    assert_eq!(s.phase, ImpulsePhase::Active);
}

// --- physics modifiers ---

#[test]
fn apply_to_physics_returns_base_values_when_idle() {
    let s = ImpulseState::new();
    let (max_speed, steering) = s.apply_to_physics(25.0, 0.8, IMPULSE_SPEED_MULTIPLIER, 1.0);
    assert!((max_speed - 25.0).abs() < f32::EPSILON);
    assert!((steering - 0.8).abs() < f32::EPSILON);
}

#[test]
fn apply_to_physics_returns_base_values_when_charging() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(1.0, IMPULSE_CHARGE_DURATION);
    let (max_speed, steering) = s.apply_to_physics(25.0, 0.8, IMPULSE_SPEED_MULTIPLIER, 1.0);
    assert!((max_speed - 25.0).abs() < f32::EPSILON);
    assert!((steering - 0.8).abs() < f32::EPSILON);
}

#[test]
fn active_impulse_applies_10x_speed_multiplier() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
    let (max_speed, _) = s.apply_to_physics(25.0, 0.0, IMPULSE_SPEED_MULTIPLIER, 1.0);
    assert!((max_speed - 25.0 * IMPULSE_SPEED_MULTIPLIER).abs() < f32::EPSILON);
}

#[test]
fn active_impulse_zeroes_steering_input() {
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
    let (_, steering) = s.apply_to_physics(25.0, 0.9, IMPULSE_SPEED_MULTIPLIER, 0.0);
    assert!((steering).abs() < f32::EPSILON);
}

#[test]
fn custom_speed_multiplier_applied_when_active() {
    let custom_mult = 5.0;
    let mut s = ImpulseState::new();
    s.start_charge();
    s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
    let (max_speed, _) = s.apply_to_physics(25.0, 0.0, custom_mult, 1.0);
    assert!((max_speed - 25.0 * custom_mult).abs() < f32::EPSILON);
}
