use super::*;

fn default_config() -> LateralThrustConfig {
    LateralThrustConfig::new()
}

#[test]
fn zero_input_decelerates_to_zero() {
    let speed = compute_lateral_speed(10.0, 0.0, 1.0, &default_config());
    assert!(
        (speed - 0.0).abs() < f32::EPSILON,
        "expected 0, got {speed}"
    );
}

#[test]
fn full_input_approaches_max_speed() {
    let speed = compute_lateral_speed(0.0, 1.0, 5.0, &default_config());
    assert!(speed >= default_config().max_lateral_speed - 0.1);
}

#[test]
fn negative_input_approaches_negative_max() {
    let speed = compute_lateral_speed(0.0, -1.0, 5.0, &default_config());
    assert!(speed <= -default_config().max_lateral_speed + 0.1);
}

#[test]
fn speed_capped_at_max() {
    let speed = compute_lateral_speed(0.0, 1.0, 10.0, &default_config());
    assert!(speed <= default_config().max_lateral_speed);
}

#[test]
fn speed_clamped_to_negative_max() {
    let speed = compute_lateral_speed(0.0, -1.0, 10.0, &default_config());
    assert!(speed >= -default_config().max_lateral_speed);
}

#[test]
fn lateral_displacement_positive_for_starboard() {
    // Yaw = 0 → facing -Z; right is +X.
    let (dx, dz) = lateral_displacement(0.0, 10.0, 1.0);
    assert!(dx > 0.0, "expected positive X displacement, got {dx}");
    assert!(
        (dz).abs() < 0.001,
        "expected minimal Z displacement, got {dz}"
    );
}

#[test]
fn lateral_displacement_negative_for_port() {
    // Yaw = 0 → facing -Z; left is -X.
    let (dx, dz) = lateral_displacement(0.0, -10.0, 1.0);
    assert!(dx < 0.0, "expected negative X displacement, got {dx}");
    assert!(
        (dz).abs() < 0.001,
        "expected minimal Z displacement, got {dz}"
    );
}

#[test]
fn lateral_displacement_rotates_with_yaw() {
    // Yaw = PI/2 → facing +X; right is +Z.
    let (dx, dz) = lateral_displacement(std::f32::consts::FRAC_PI_2, 10.0, 1.0);
    assert!(
        (dx).abs() < 0.001,
        "expected minimal X displacement, got {dx}"
    );
    assert!(dz > 0.0, "expected positive Z displacement, got {dz}");
}
