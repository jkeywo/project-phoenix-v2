use super::*;

// ── bank_origin ─────────────────────────────────────────────────────────

/// At yaw=0, ship faces −Z.
/// Right (starboard) vector = (cos 0, sin 0) = (+1, 0) in XZ.
/// Port (-1.0) offset = (−4, 0).  Starboard (+1.0) offset = (+4, 0).
#[test]
fn bank_origin_port_at_yaw_zero() {
    let (x, z) = bank_origin(0.0, 0.0, 0.0, -1.0, 4.0);
    assert!((x - (-4.0)).abs() < 1e-5, "x should be -4, got {x}");
    assert!(z.abs() < 1e-5, "z should be 0, got {z}");
}

#[test]
fn bank_origin_starboard_at_yaw_zero() {
    let (x, z) = bank_origin(0.0, 0.0, 0.0, 1.0, 4.0);
    assert!((x - 4.0).abs() < 1e-5, "x should be +4, got {x}");
    assert!(z.abs() < 1e-5, "z should be 0, got {z}");
}

/// Ship offset by (10, 5) – origin should shift accordingly.
#[test]
fn bank_origin_respects_ship_position() {
    let (x, z) = bank_origin(10.0, 5.0, 0.0, 1.0, 4.0);
    assert!((x - 14.0).abs() < 1e-5);
    assert!((z - 5.0).abs() < 1e-5);
}

/// At yaw = π/2, ship faces +X.
/// Right (starboard) vector = (cos π/2, sin π/2) ≈ (0, +1).
#[test]
fn bank_origin_starboard_at_yaw_pi_over_2() {
    let yaw = std::f32::consts::FRAC_PI_2;
    let (x, z) = bank_origin(0.0, 0.0, yaw, 1.0, 4.0);
    // cos(π/2) ≈ 0, sin(π/2) = 1 → offset = (0, +4)
    assert!(x.abs() < 1e-5, "x should be ~0, got {x}");
    assert!((z - 4.0).abs() < 1e-5, "z should be +4, got {z}");
}

// ── beam_endpoint ────────────────────────────────────────────────────────

/// Target within range → endpoint equals target.
#[test]
fn beam_endpoint_within_range_returns_target() {
    let (x, z) = beam_endpoint(0.0, 0.0, 10.0, 0.0, 40.0);
    assert!((x - 10.0).abs() < 1e-5);
    assert!(z.abs() < 1e-5);
}

/// Target exactly at max range → endpoint equals target.
#[test]
fn beam_endpoint_at_max_range_returns_target() {
    let (x, z) = beam_endpoint(0.0, 0.0, 40.0, 0.0, 40.0);
    assert!((x - 40.0).abs() < 1e-4);
    assert!(z.abs() < 1e-5);
}

/// Target beyond max range → endpoint clamped to max_range along direction.
#[test]
fn beam_endpoint_beyond_range_clamps_to_max() {
    // Target at (80, 0), max_range = 40 → endpoint = (40, 0)
    let (x, z) = beam_endpoint(0.0, 0.0, 80.0, 0.0, 40.0);
    assert!((x - 40.0).abs() < 1e-4, "x should be 40, got {x}");
    assert!(z.abs() < 1e-5);
}

/// Non-axis direction, target beyond range.
#[test]
fn beam_endpoint_diagonal_beyond_range() {
    // Target at (30, 40) = distance 50, max_range = 25 → scale = 0.5
    // endpoint = (15, 20)
    let (x, z) = beam_endpoint(0.0, 0.0, 30.0, 40.0, 25.0);
    assert!((x - 15.0).abs() < 1e-4, "x should be 15, got {x}");
    assert!((z - 20.0).abs() < 1e-4, "z should be 20, got {z}");
}

/// Ship not at origin.
#[test]
fn beam_endpoint_non_zero_ship_position() {
    // Ship at (10, 10), target at (10, 60) = distance 50, max_range = 25
    // endpoint = (10, 35)
    let (x, z) = beam_endpoint(10.0, 10.0, 10.0, 60.0, 25.0);
    assert!((x - 10.0).abs() < 1e-4);
    assert!((z - 35.0).abs() < 1e-4, "z should be 35, got {z}");
}

// ── resolve_beam_color ───────────────────────────────────────────────────

#[test]
fn resolve_beam_color_uses_configured_when_four_elements() {
    let color = resolve_beam_color(&[0.5, 0.3, 0.8, 0.7]);
    assert_eq!(color, [0.5, 0.3, 0.8, 0.7]);
}

#[test]
fn resolve_beam_color_falls_back_to_default_when_empty() {
    let color = resolve_beam_color(&[]);
    assert_eq!(color, DEFAULT_BEAM_COLOR);
}

#[test]
fn resolve_beam_color_falls_back_to_default_when_wrong_length() {
    let color = resolve_beam_color(&[1.0, 0.5, 0.2]);
    assert_eq!(color, DEFAULT_BEAM_COLOR);
}
