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
// Presentation-only beam gizmo geometry (drawn by renderer.rs): never feeds
// simulation state, so std transcendentals are fine (issue #908, simmath.rs).
#[allow(clippy::disallowed_methods)]
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
#[path = "beam_render_tests.rs"]
mod tests;
