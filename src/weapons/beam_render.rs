//! Pure beam-rendering geometry helpers.
//!
//! These functions are Bevy-free and fully unit-testable.  The actual
//! gizmo calls live in `renderer.rs`; this module provides the
//! *positions* to draw from/to and the colour to use.
//!
//! # Coordinate system
//! World-space XZ plane (Y-up).  Ship heading at `yaw = 0` faces −Z.
//! * Forward: `( sin(yaw), 0, −cos(yaw) )`
//! * Right (starboard): `( cos(yaw), 0,  sin(yaw) )`
//! * Left  (port):      `(−cos(yaw), 0, −sin(yaw) )`

/// Lateral hull offset (world units) from the ship centre to the point
/// where each phaser bank's emitter is positioned.
pub const BANK_HULL_OFFSET: f32 = 4.0;

/// Default beam colour as RGBA when none is configured.
pub const DEFAULT_BEAM_COLOR: [f32; 4] = [1.0, 0.4, 0.1, 1.0];

/// World-space XZ origin point for a phaser bank's emitter.
///
/// # Arguments
/// * `ship_x`, `ship_z` – ship centre position.
/// * `ship_yaw` – ship heading in radians.
/// * `bank_side` – `-1.0` for port (offsets left), `+1.0` for starboard (offsets right).
///   Callers compute this from a bank's `facing_deg` (negative → port).
/// * `hull_offset` – lateral distance from centre to the emitter.
///
/// Returns `(x, z)`.
pub fn bank_origin(
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    bank_side: f32,
    hull_offset: f32,
) -> (f32, f32) {
    // Right vector: (cos(yaw), sin(yaw)) in XZ
    let right_x = ship_yaw.cos();
    let right_z = ship_yaw.sin();
    (
        ship_x + bank_side * right_x * hull_offset,
        ship_z + bank_side * right_z * hull_offset,
    )
}

/// World-space XZ endpoint for a phaser beam.
///
/// If the target is within `max_range` from the ship centre, the endpoint
/// is exactly the target position.  Otherwise the endpoint is clamped to
/// `max_range` along the direction from the ship to the target.
///
/// Returns `(x, z)`.
pub fn beam_endpoint(
    ship_x: f32,
    ship_z: f32,
    target_x: f32,
    target_z: f32,
    max_range: f32,
) -> (f32, f32) {
    let dx = target_x - ship_x;
    let dz = target_z - ship_z;
    let dist = (dx * dx + dz * dz).sqrt();
    if dist <= max_range || dist < 1e-6 {
        (target_x, target_z)
    } else {
        let scale = max_range / dist;
        (ship_x + dx * scale, ship_z + dz * scale)
    }
}

/// Resolve the beam colour from an optional RGBA config slice.
///
/// If `configured` has exactly 4 elements they are used as `[r, g, b, a]`.
/// Otherwise falls back to `DEFAULT_BEAM_COLOR`.
///
/// Returns `[r, g, b, a]` in 0.0–1.0.
pub fn resolve_beam_color(configured: &[f32]) -> [f32; 4] {
    if configured.len() == 4 {
        [configured[0], configured[1], configured[2], configured[3]]
    } else {
        DEFAULT_BEAM_COLOR
    }
}

#[cfg(test)]
mod tests {
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
}
