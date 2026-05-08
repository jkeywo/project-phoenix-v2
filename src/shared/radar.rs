// Pure-Rust radar rendering primitives.
//
// This module is platform-agnostic: it takes the ship pose, asteroid
// positions, and a radar range, and produces unit-square coordinates
// (`-1.0..=1.0` on both axes) for plotting on whatever surface the caller
// wants (Bevy 2D camera in the server renderer, Bevy UI overlay in the
// client). Keeping the math separate makes both renderers identical and
// independently unit-testable.

use crate::shared::messages::AsteroidInfo;

/// Range, in world units, that the radar covers from its centre to the
/// outer ring. Anything beyond is clipped.
pub const RADAR_RANGE: f32 = 50.0;

/// Inner reference ring drawn at this fraction of the outer ring.
pub const RADAR_MID_RING: f32 = 25.0;

/// Project a world-space point onto the radar's unit square, relative to
/// the ship's position and yaw.
///
/// The radar is *ship-centred and ship-aligned*: the ship sits at the
/// origin and "forward" (the direction the ship faces) points up. Points
/// outside `RADAR_RANGE` return `None`.
///
/// Output is normalised to `[-1.0, 1.0]` on both axes (1.0 = `RADAR_RANGE`).
/// X is right, Y is forward.
pub fn project_to_radar(world_x: f32, world_z: f32, ship_x: f32, ship_z: f32, ship_yaw: f32) -> Option<(f32, f32)> {
    // World-space displacement from ship to point.
    let dx = world_x - ship_x;
    let dz = world_z - ship_z;

    // Range cull before any rotation work.
    if dx * dx + dz * dz > RADAR_RANGE * RADAR_RANGE {
        return None;
    }

    // Rotate the world displacement by -yaw so the ship's forward direction
    // becomes the +Y axis on the radar. With yaw=0 the ship faces -Z, so
    // forward (-Z) must map to +Y; a point directly ahead (dz < 0) yields
    // +radar_y. Hence radar_y = -dz at yaw=0.
    let cos_y = ship_yaw.cos();
    let sin_y = ship_yaw.sin();
    // Rotation by -yaw applied to (dx, dz):
    //   rx =  dx*cos(yaw) - dz*sin(yaw)
    //   rz =  dx*sin(yaw) + dz*cos(yaw)
    let rx =  dx * cos_y - dz * sin_y;
    let rz =  dx * sin_y + dz * cos_y;

    let radar_x = rx / RADAR_RANGE;
    let radar_y = -rz / RADAR_RANGE;
    Some((radar_x, radar_y))
}

/// Iterator of `(radar_x, radar_y, scaled_radius)` tuples for all asteroids
/// within `RADAR_RANGE` of the ship. Out-of-range asteroids are silently skipped.
pub fn radar_dots<'a>(
    asteroids: &'a [AsteroidInfo],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> impl Iterator<Item = (f32, f32, f32)> + 'a {
    asteroids.iter().filter_map(move |a| project_asteroid(a, ship_x, ship_z, ship_yaw))
}

/// Project an asteroid to radar coordinates plus its scaled radius.
///
/// Returns `None` if the asteroid centre is outside `RADAR_RANGE`. The
/// scaled radius is in the same `[-1, 1]` units as the position (i.e. the
/// asteroid's world radius divided by `RADAR_RANGE`).
pub fn project_asteroid(asteroid: &AsteroidInfo, ship_x: f32, ship_z: f32, ship_yaw: f32) -> Option<(f32, f32, f32)> {
    let (rx, ry) = project_to_radar(asteroid.x, asteroid.z, ship_x, ship_z, ship_yaw)?;
    Some((rx, ry, asteroid.radius / RADAR_RANGE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "expected {b}, got {a}");
    }

    #[test]
    fn point_at_ship_position_maps_to_centre() {
        let p = project_to_radar(10.0, -3.0, 10.0, -3.0, 0.0).unwrap();
        close(p.0, 0.0);
        close(p.1, 0.0);
    }

    #[test]
    fn point_directly_ahead_at_yaw_zero_maps_to_positive_y() {
        // Ship at origin, yaw=0 means facing -Z. A point at (0, -25) is
        // directly ahead: half the radar range, so radar_y = +0.5.
        let p = project_to_radar(0.0, -25.0, 0.0, 0.0, 0.0).unwrap();
        close(p.0, 0.0);
        close(p.1, 0.5);
    }

    #[test]
    fn point_to_starboard_at_yaw_zero_maps_to_positive_x() {
        // Yaw=0 (facing -Z), a point at (+25, 0) is on the right wing.
        let p = project_to_radar(25.0, 0.0, 0.0, 0.0, 0.0).unwrap();
        close(p.0, 0.5);
        close(p.1, 0.0);
    }

    #[test]
    fn point_outside_range_returns_none() {
        // 51 > RADAR_RANGE
        assert!(project_to_radar(51.0, 0.0, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn point_exactly_at_range_is_kept() {
        // Just inside the radar boundary
        let p = project_to_radar(RADAR_RANGE, 0.0, 0.0, 0.0, 0.0).unwrap();
        close(p.0, 1.0);
        close(p.1, 0.0);
    }

    #[test]
    fn ship_yaw_rotates_world_to_keep_forward_pointing_up() {
        // Yaw = π/2: ship has rotated 90° anticlockwise around +Y.
        // From the ship's frame, what was at world +X is now behind
        // (or in front, depending on rotation sense). The convention
        // used in ship_physics rotates the heading by +yaw counter-
        // clockwise when viewed from above; the transform here must be
        // its inverse.
        // Place a point ahead of the ship (in its local frame) at world
        // location consistent with yaw=π/2 facing +X (since yaw=0 → -Z,
        // yaw=π/2 → -Z rotated by +90°). Easier: assert symmetry — a
        // point directly to the ship's right always maps to (+rx, 0)
        // regardless of yaw.
        let yaw = std::f32::consts::FRAC_PI_2;
        // World position of "25 units to ship's right" when yaw=π/2:
        // ship faces (sin(yaw), 0, -cos(yaw)) = (1, 0, 0). Right of that
        // is (-cos(yaw), 0, -sin(yaw)) = (0, 0, -1). So 25 to the right
        // is world (0, 0, -25).
        let p = project_to_radar(0.0, -25.0, 0.0, 0.0, yaw).unwrap();
        close(p.0, 0.5);
        close(p.1, 0.0);
    }

    #[test]
    fn project_asteroid_scales_radius_by_range() {
        let a = AsteroidInfo { x: 0.0, z: -10.0, radius: 2.0 };
        let (_rx, _ry, r) = project_asteroid(&a, 0.0, 0.0, 0.0).unwrap();
        close(r, 2.0 / RADAR_RANGE);
    }

    #[test]
    fn project_asteroid_outside_range_returns_none() {
        let a = AsteroidInfo { x: 100.0, z: 100.0, radius: 2.0 };
        assert!(project_asteroid(&a, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn radar_constants_have_expected_values() {
        // PRD pinned values; locking these in protects the ring renderer.
        assert_eq!(RADAR_RANGE, 50.0);
        assert_eq!(RADAR_MID_RING, 25.0);
    }

    #[test]
    fn radar_dots_single_in_range_matches_project_asteroid() {
        let asteroid = AsteroidInfo { x: 0.0, z: -10.0, radius: 2.0 };
        let dots: Vec<_> = radar_dots(&[asteroid.clone()], 0.0, 0.0, 0.0).collect();
        assert_eq!(dots.len(), 1);
        let expected = project_asteroid(&asteroid, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(dots[0], expected);
    }

    #[test]
    fn radar_dots_skips_out_of_range_asteroids() {
        let far = AsteroidInfo { x: 100.0, z: 100.0, radius: 2.0 };
        let near = AsteroidInfo { x: 0.0, z: -10.0, radius: 2.0 };
        let dots: Vec<_> = radar_dots(&[far, near.clone()], 0.0, 0.0, 0.0).collect();
        assert_eq!(dots.len(), 1);
        assert_eq!(dots[0], project_asteroid(&near, 0.0, 0.0, 0.0).unwrap());
    }

    #[test]
    fn radar_dots_empty_slice_returns_empty_iterator() {
        let dots: Vec<_> = radar_dots(&[], 0.0, 0.0, 0.0).collect();
        assert!(dots.is_empty());
    }
}
