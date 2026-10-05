// Phaser fire-readiness check, in ship-local radar space.
//
// Historical note: this module used to hold the full pure-Rust radar
// projection pipeline for the Bevy/WASM client consoles. Those consoles are
// now pure HTML/JS (`gui/radar-math.js` owns client-side projection) and the
// server viewscreen radar projects via `gui::radar::project_radar_entity`,
// so only the weapons-server fire check remains here.

use crate::simmath;

/// Returns `true` if a world-space target is within phaser firing parameters:
/// - distance from ship ≤ `phaser_range`, and
/// - inside the ship's 180° forward arc (forward hemisphere in ship-local
///   space).
///
/// The forward arc is defined by `radar_y >= 0` in ship-aligned space, where
/// `radar_y = dot((dx, dz), forward)` and `forward = (sin(yaw), -cos(yaw))`.
/// A target exactly on the beam (at 90° to the side) **is** fire-ready
/// (`radar_y == 0`).
pub fn is_fire_ready_with_range(
    target_x: f32,
    target_z: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    phaser_range: f32,
) -> bool {
    let dx = target_x - ship_x;
    let dz = target_z - ship_z;

    // Range gate: must be within phaser_range.
    if dx * dx + dz * dz > phaser_range * phaser_range {
        return false;
    }

    // Arc gate: must be in the forward 180° hemisphere (radar_y >= 0).
    let sin_y = simmath::sin(ship_yaw);
    let cos_y = simmath::cos(ship_yaw);
    let radar_y = dx * sin_y - dz * cos_y;
    radar_y >= 0.0
}

#[cfg(test)]
#[path = "radar_tests.rs"]
mod tests;
