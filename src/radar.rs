// Pure-Rust radar rendering primitives.
//
// This module is platform-agnostic: it takes the ship pose, asteroid
// positions, and a radar range, and produces unit-square coordinates
// (`-1.0..=1.0` on both axes) for plotting on whatever surface the caller
// wants (Bevy 2D camera in the server renderer, Bevy UI overlay in the
// client). Keeping the math separate makes both renderers identical and
// independently unit-testable.

use crate::messages::AsteroidInfo;
use crate::entity_tags::{EntityTag, matches_any, parse_tags};
use crate::radar_config::RadarConfig;

/// Range, in world units, that the radar covers from its centre to the
/// outer ring. Anything beyond is clipped.
pub const RADAR_RANGE: f32 = 50.0;

/// Maximum firing range, in world units, for the phaser weapon.
pub const PHASER_RANGE: f32 = 40.0;

/// Inner reference ring drawn at this fraction of the outer ring.
pub const RADAR_MID_RING: f32 = 25.0;

/// The Weapons console radar range — asteroids within this distance of the
/// ship can be locked as targets. Distinct from `RADAR_RANGE` (the helm's
/// situational-awareness view) so each console can be tuned independently.
pub const WEAPONS_RADAR_RANGE: f32 = 60.0;

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

    // Project world displacement onto ship-local axes.
    //
    // The ship's axes in world XZ space are:
    //   forward = (sin(yaw), -cos(yaw))
    //   right   = (cos(yaw),  sin(yaw))   [forward × world_up, right-hand rule]
    //
    // radar_x = dot((dx,dz), right)   = dx*cos(yaw) + dz*sin(yaw)
    // radar_y = dot((dx,dz), forward) = dx*sin(yaw) - dz*cos(yaw)
    //
    // At yaw=0: right=(1,0), forward=(0,-1).  A point at dz=-25 (ahead) gives
    // radar_y = +0.5.  A point at dx=+25 (starboard) gives radar_x = +0.5.
    let cos_y = ship_yaw.cos();
    let sin_y = ship_yaw.sin();

    let radar_x = (dx * cos_y + dz * sin_y) / RADAR_RANGE;
    let radar_y = (dx * sin_y - dz * cos_y) / RADAR_RANGE;
    Some((radar_x, radar_y))
}

/// Returns `true` if a world-space target is within phaser firing parameters:
/// - distance from ship ≤ `PHASER_RANGE` (40 world units), and
/// - inside the ship's 180° forward arc (forward hemisphere in ship-local space).
///
/// The forward arc is defined by `radar_y > 0` in the same ship-aligned
/// projection used by `project_to_radar`.  A target exactly on the beam (at
/// 90° to the side) is **not** fire-ready (`radar_y == 0`).
pub fn is_fire_ready(
    target_x: f32,
    target_z: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> bool {
    let dx = target_x - ship_x;
    let dz = target_z - ship_z;

    // Range gate: must be within PHASER_RANGE.
    if dx * dx + dz * dz > PHASER_RANGE * PHASER_RANGE {
        return false;
    }

    // Arc gate: must be in the forward 180° hemisphere (radar_y > 0).
    // radar_y = dot((dx,dz), forward) = dx*sin(yaw) - dz*cos(yaw)
    let sin_y = ship_yaw.sin();
    let cos_y = ship_yaw.cos();
    let radar_y = dx * sin_y - dz * cos_y;
    radar_y > 0.0
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

/// Like `radar_dots` but only includes entities whose tags match **at least
/// one** tag in `filter_tags` (OR logic).  If `filter_tags` is empty, no
/// entities are returned.
pub fn radar_dots_filtered<'a>(
    asteroids: &'a [AsteroidInfo],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    filter_tags: &'a [EntityTag],
) -> impl Iterator<Item = (f32, f32, f32)> + 'a {
    asteroids.iter().filter_map(move |a| {
        let entity_tags = parse_tags(&a.tags);
        if matches_any(&entity_tags, filter_tags) {
            project_asteroid(a, ship_x, ship_z, ship_yaw)
        } else {
            None
        }
    })
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

/// Project a world-space point onto the radar's unit square using a `RadarConfig`.
///
/// Behaves identically to `project_to_radar` but uses the range from
/// `config.range` instead of the global `RADAR_RANGE` constant.  Output is
/// normalised to `[-1.0, 1.0]` on both axes where ±1.0 = `config.range`.
pub fn project_to_radar_with_config(
    world_x: f32,
    world_z: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    config: &RadarConfig,
) -> Option<(f32, f32)> {
    let dx = world_x - ship_x;
    let dz = world_z - ship_z;
    let range = config.range;

    if dx * dx + dz * dz > range * range {
        return None;
    }

    let cos_y = ship_yaw.cos();
    let sin_y = ship_yaw.sin();

    let radar_x = (dx * cos_y + dz * sin_y) / range;
    let radar_y = (dx * sin_y - dz * cos_y) / range;
    Some((radar_x, radar_y))
}

/// Iterator of `(radar_x, radar_y, scaled_radius)` tuples for entities within
/// `config.range`, filtered by `config.shows` using OR tag logic.
///
/// - Entities outside `config.range` are skipped.
/// - Entities whose tags do not overlap `config.shows` are skipped.
/// - If `config.shows` is empty, no entities are returned.
pub fn radar_dots_with_config<'a>(
    asteroids: &'a [AsteroidInfo],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    config: &'a RadarConfig,
) -> impl Iterator<Item = (f32, f32, f32)> + 'a {
    asteroids.iter().filter_map(move |a| {
        let entity_tags = parse_tags(&a.tags);
        if !matches_any(&entity_tags, &config.shows) {
            return None;
        }
        let (rx, ry) = project_to_radar_with_config(a.x, a.z, ship_x, ship_z, ship_yaw, config)?;
        Some((rx, ry, a.radius / config.range))
    })
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
        // Yaw = π/2: ship faces +X (forward = (sin(π/2), -cos(π/2)) = (1, 0) in XZ).
        // The ship's starboard (right) vector = forward × world_up = +Z direction.
        // So 25 units to starboard is world (x=0, z=+25).
        // That point should map to radar_x=+0.5, radar_y=0.
        let yaw = std::f32::consts::FRAC_PI_2;
        let p = project_to_radar(0.0, 25.0, 0.0, 0.0, yaw).unwrap();
        close(p.0, 0.5);
        close(p.1, 0.0);

        // Also verify: a point directly ahead at yaw=π/2 is at world (+25, 0).
        // It should map to radar_x=0, radar_y=+0.5.
        let q = project_to_radar(25.0, 0.0, 0.0, 0.0, yaw).unwrap();
        close(q.0, 0.0);
        close(q.1, 0.5);
    }

    #[test]
    fn project_asteroid_scales_radius_by_range() {
        let a = AsteroidInfo { uuid: "".into(), x: 0.0, z: -10.0, radius: 2.0, tags: vec![] };
        let (_rx, _ry, r) = project_asteroid(&a, 0.0, 0.0, 0.0).unwrap();
        close(r, 2.0 / RADAR_RANGE);
    }

    #[test]
    fn project_asteroid_outside_range_returns_none() {
        let a = AsteroidInfo { uuid: "".into(), x: 100.0, z: 100.0, radius: 2.0, tags: vec![] };
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
        let asteroid = AsteroidInfo { uuid: "".into(), x: 0.0, z: -10.0, radius: 2.0, tags: vec![] };
        let dots: Vec<_> = radar_dots(&[asteroid.clone()], 0.0, 0.0, 0.0).collect();
        assert_eq!(dots.len(), 1);
        let expected = project_asteroid(&asteroid, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(dots[0], expected);
    }

    #[test]
    fn radar_dots_skips_out_of_range_asteroids() {
        let far = AsteroidInfo { uuid: "".into(), x: 100.0, z: 100.0, radius: 2.0, tags: vec![] };
        let near = AsteroidInfo { uuid: "".into(), x: 0.0, z: -10.0, radius: 2.0, tags: vec![] };
        let dots: Vec<_> = radar_dots(&[far, near.clone()], 0.0, 0.0, 0.0).collect();
        assert_eq!(dots.len(), 1);
        assert_eq!(dots[0], project_asteroid(&near, 0.0, 0.0, 0.0).unwrap());
    }

    #[test]
    fn radar_dots_empty_slice_returns_empty_iterator() {
        let dots: Vec<_> = radar_dots(&[], 0.0, 0.0, 0.0).collect();
        assert!(dots.is_empty());
    }

    // ── radar_dots_filtered ────────────────────────────────────────────────

    fn asteroid_with_tags(x: f32, z: f32, tags: &[&str]) -> AsteroidInfo {
        AsteroidInfo {
            uuid: "".into(),
            x,
            z,
            radius: 1.0,
            tags: tags.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn radar_dots_filtered_returns_only_matching_tag_entities() {
        let asteroid = asteroid_with_tags(0.0, -10.0, &["asteroid"]);
        let ship = asteroid_with_tags(0.0, -15.0, &["ship"]);
        let filter = vec![EntityTag::Asteroid];
        let dots: Vec<_> = radar_dots_filtered(&[asteroid.clone(), ship], 0.0, 0.0, 0.0, &filter).collect();
        assert_eq!(dots.len(), 1);
        assert_eq!(dots[0], project_asteroid(&asteroid, 0.0, 0.0, 0.0).unwrap());
    }

    #[test]
    fn radar_dots_filtered_or_logic_includes_either_tag() {
        let a = asteroid_with_tags(0.0, -10.0, &["asteroid"]);
        let s = asteroid_with_tags(5.0, -10.0, &["ship"]);
        let filter = vec![EntityTag::Asteroid, EntityTag::Ship];
        let dots: Vec<_> = radar_dots_filtered(&[a, s], 0.0, 0.0, 0.0, &filter).collect();
        assert_eq!(dots.len(), 2);
    }

    #[test]
    fn radar_dots_filtered_empty_filter_returns_nothing() {
        let a = asteroid_with_tags(0.0, -10.0, &["asteroid"]);
        let dots: Vec<_> = radar_dots_filtered(&[a], 0.0, 0.0, 0.0, &[]).collect();
        assert!(dots.is_empty());
    }

    #[test]
    fn radar_dots_filtered_skips_out_of_range_even_if_tag_matches() {
        let far = asteroid_with_tags(100.0, 100.0, &["asteroid"]);
        let filter = vec![EntityTag::Asteroid];
        let dots: Vec<_> = radar_dots_filtered(&[far], 0.0, 0.0, 0.0, &filter).collect();
        assert!(dots.is_empty());
    }

    #[test]
    fn radar_dots_filtered_skips_no_tag_entity() {
        let a = asteroid_with_tags(0.0, -10.0, &[]);
        let filter = vec![EntityTag::Asteroid];
        let dots: Vec<_> = radar_dots_filtered(&[a], 0.0, 0.0, 0.0, &filter).collect();
        assert!(dots.is_empty());
    }

    // ── is_fire_ready ──────────────────────────────────────────────────────────

    /// Directly ahead, well within range → fire-ready.
    #[test]
    fn fire_ready_target_ahead_in_range() {
        // yaw=0: forward is -Z. Target at (0, -20) is 20 units ahead.
        assert!(is_fire_ready(0.0, -20.0, 0.0, 0.0, 0.0));
    }

    /// Directly behind → not fire-ready (aft hemisphere).
    #[test]
    fn fire_ready_target_behind_is_not_ready() {
        // yaw=0: target at (0, +20) is directly aft.
        assert!(!is_fire_ready(0.0, 20.0, 0.0, 0.0, 0.0));
    }

    /// Exactly at 40-unit range, ahead → fire-ready (boundary inclusive).
    #[test]
    fn fire_ready_at_exact_range_boundary() {
        assert!(is_fire_ready(0.0, -PHASER_RANGE, 0.0, 0.0, 0.0));
    }

    /// One unit beyond 40-unit range → not fire-ready.
    #[test]
    fn fire_ready_just_outside_range_is_not_ready() {
        assert!(!is_fire_ready(0.0, -(PHASER_RANGE + 1.0), 0.0, 0.0, 0.0));
    }

    /// Exactly 90° to the side (beam direction) → not fire-ready (arc boundary exclusive).
    #[test]
    fn fire_ready_at_90_degree_arc_boundary_is_not_ready() {
        // yaw=0: target at (+20, 0) is exactly 90° to starboard (radar_y = 0).
        assert!(!is_fire_ready(20.0, 0.0, 0.0, 0.0, 0.0));
    }

    /// Just inside the forward arc (slightly ahead of beam) → fire-ready.
    #[test]
    fn fire_ready_just_inside_forward_arc() {
        // Target at (+20, -1): mostly starboard but slightly ahead.
        assert!(is_fire_ready(20.0, -1.0, 0.0, 0.0, 0.0));
    }

    /// With ship yaw rotated: target must still be evaluated in ship-local space.
    #[test]
    fn fire_ready_respects_ship_yaw() {
        // yaw = π/2: ship faces +X. Target at (+20, 0) is directly ahead.
        let yaw = std::f32::consts::FRAC_PI_2;
        assert!(is_fire_ready(20.0, 0.0, 0.0, 0.0, yaw));
        // Same target but ship now faces -X (yaw = -π/2): target is aft.
        assert!(!is_fire_ready(20.0, 0.0, 0.0, 0.0, -yaw));
    }

    // ── project_to_radar_with_config ──────────────────────────────────────

    fn helm_config() -> RadarConfig {
        RadarConfig {
            range: 50.0,
            shows: vec![EntityTag::Asteroid],
        }
    }

    #[test]
    fn config_project_ahead_matches_fixed_function() {
        let cfg = helm_config();
        let fixed = project_to_radar(0.0, -25.0, 0.0, 0.0, 0.0).unwrap();
        let with_cfg = project_to_radar_with_config(0.0, -25.0, 0.0, 0.0, 0.0, &cfg).unwrap();
        close(with_cfg.0, fixed.0);
        close(with_cfg.1, fixed.1);
    }

    #[test]
    fn config_project_clips_at_config_range() {
        // Config range = 30; point at 35 units ahead is out of range.
        let cfg = RadarConfig { range: 30.0, shows: vec![EntityTag::Asteroid] };
        assert!(project_to_radar_with_config(0.0, -35.0, 0.0, 0.0, 0.0, &cfg).is_none());
        // Same point is within the global RADAR_RANGE (50) — ensure we use config range.
        assert!(project_to_radar(0.0, -35.0, 0.0, 0.0, 0.0).is_some());
    }

    #[test]
    fn config_project_normalises_to_config_range() {
        // Point at config.range ahead should map to radar_y = 1.0.
        let cfg = RadarConfig { range: 80.0, shows: vec![] };
        let p = project_to_radar_with_config(0.0, -80.0, 0.0, 0.0, 0.0, &cfg).unwrap();
        close(p.1, 1.0);
    }

    // ── radar_dots_with_config ────────────────────────────────────────────

    #[test]
    fn dots_with_config_filters_by_shows() {
        let asteroid = asteroid_with_tags(0.0, -10.0, &["asteroid"]);
        let ship_entity = asteroid_with_tags(5.0, -10.0, &["ship"]);
        let cfg = RadarConfig {
            range: 50.0,
            shows: vec![EntityTag::Asteroid],
        };
        let dots: Vec<_> = radar_dots_with_config(&[asteroid.clone(), ship_entity], 0.0, 0.0, 0.0, &cfg).collect();
        assert_eq!(dots.len(), 1);
        // Scaled radius should use config.range.
        close(dots[0].2, asteroid.radius / cfg.range);
    }

    #[test]
    fn dots_with_config_empty_shows_returns_nothing() {
        let asteroid = asteroid_with_tags(0.0, -10.0, &["asteroid"]);
        let cfg = RadarConfig { range: 50.0, shows: vec![] };
        let dots: Vec<_> = radar_dots_with_config(&[asteroid], 0.0, 0.0, 0.0, &cfg).collect();
        assert!(dots.is_empty());
    }

    #[test]
    fn dots_with_config_clips_at_config_range_not_global() {
        // Asteroid at 40 units (inside RADAR_RANGE=50 but outside config range=30).
        let asteroid = asteroid_with_tags(0.0, -40.0, &["asteroid"]);
        let cfg = RadarConfig { range: 30.0, shows: vec![EntityTag::Asteroid] };
        let dots: Vec<_> = radar_dots_with_config(&[asteroid], 0.0, 0.0, 0.0, &cfg).collect();
        assert!(dots.is_empty());
    }

    #[test]
    fn dots_with_config_or_logic_for_multiple_shows() {
        let a = asteroid_with_tags(0.0, -10.0, &["asteroid"]);
        let s = asteroid_with_tags(5.0, -10.0, &["ship"]);
        let cfg = RadarConfig {
            range: 50.0,
            shows: vec![EntityTag::Asteroid, EntityTag::Ship],
        };
        let dots: Vec<_> = radar_dots_with_config(&[a, s], 0.0, 0.0, 0.0, &cfg).collect();
        assert_eq!(dots.len(), 2);
    }
}

