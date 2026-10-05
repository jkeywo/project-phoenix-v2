use super::*;

/// Baseline range used by these tests; matches
/// `PhaserCombatConfig::DEFAULT_PHASER_RANGE`.
const RANGE: f32 = 40.0;

fn fire_ready(target_x: f32, target_z: f32, yaw: f32) -> bool {
    is_fire_ready_with_range(target_x, target_z, 0.0, 0.0, yaw, RANGE)
}

/// Directly ahead, well within range → fire-ready.
#[test]
fn fire_ready_target_ahead_in_range() {
    // yaw=0: forward is -Z. Target at (0, -20) is 20 units ahead.
    assert!(fire_ready(0.0, -20.0, 0.0));
}

/// Directly behind → not fire-ready (aft hemisphere).
#[test]
fn fire_ready_target_behind_is_not_ready() {
    // yaw=0: target at (0, +20) is directly aft.
    assert!(!fire_ready(0.0, 20.0, 0.0));
}

/// Exactly at range, ahead → fire-ready (boundary inclusive).
#[test]
fn fire_ready_at_exact_range_boundary() {
    assert!(fire_ready(0.0, -RANGE, 0.0));
}

/// One unit beyond range → not fire-ready.
#[test]
fn fire_ready_just_outside_range_is_not_ready() {
    assert!(!fire_ready(0.0, -(RANGE + 1.0), 0.0));
}

/// Exactly 90° to the side (beam direction) → fire-ready (arc boundary inclusive).
#[test]
fn fire_ready_at_90_degree_arc_boundary_is_fire_ready() {
    // yaw=0: target at (+20, 0) is exactly 90° to starboard (radar_y = 0).
    assert!(fire_ready(20.0, 0.0, 0.0));
}

/// With ship yaw rotated: target must still be evaluated in ship-local space.
#[test]
fn fire_ready_respects_ship_yaw() {
    // yaw = π/2: ship faces +X. Target at (+20, 0) is directly ahead.
    let yaw = std::f32::consts::FRAC_PI_2;
    assert!(fire_ready(20.0, 0.0, yaw));
    // Same target but ship now faces -X (yaw = -π/2): target is aft.
    assert!(!fire_ready(20.0, 0.0, -yaw));
}

/// A larger caller-supplied range extends the gate.
#[test]
fn fire_ready_with_custom_range_extends_gate() {
    assert!(!fire_ready(0.0, -50.0, 0.0));
    assert!(is_fire_ready_with_range(0.0, -50.0, 0.0, 0.0, 0.0, 60.0));
}
