// Pure-Rust radar rendering primitives.
//
// This module is platform-agnostic: it takes the ship pose, asteroid
// positions, and a radar range, and produces unit-square coordinates
// (`-1.0..=1.0` on both axes) for plotting on whatever surface the caller
// wants (Bevy 2D camera in the server renderer, Bevy UI overlay in the
// client). Keeping the math separate makes both renderers identical and
// independently unit-testable.

use crate::entity_tags::{matches_any, parse_tags, EntityTag};
use crate::messages::EntitySnapshot;
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
pub fn project_to_radar(
    world_x: f32,
    world_z: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> Option<(f32, f32)> {
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
    is_fire_ready_with_range(target_x, target_z, ship_x, ship_z, ship_yaw, PHASER_RANGE)
}

/// Like `is_fire_ready` but accepts a caller-supplied `phaser_range`, allowing
/// modifier-scaled range checks without changing the constant.
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

    // Arc gate: must be in the forward 180° hemisphere (radar_y > 0).
    // radar_y = dot((dx,dz), forward) = dx*sin(yaw) - dz*cos(yaw)
    let sin_y = ship_yaw.sin();
    let cos_y = ship_yaw.cos();
    let radar_y = dx * sin_y - dz * cos_y;
    radar_y > 0.0
}

/// Iterator of `(radar_x, radar_y, scaled_radius)` tuples for all entities
/// with tag "asteroid" within `RADAR_RANGE` of the ship. Out-of-range entities
/// are silently skipped.
pub fn radar_dots<'a>(
    entities: &'a [EntitySnapshot],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> impl Iterator<Item = (f32, f32, f32)> + 'a {
    entities.iter().filter_map(move |e| {
        let tags = parse_tags(&e.tags);
        if !tags.contains(&EntityTag::Asteroid) {
            return None;
        }
        project_entity_as_asteroid(e, ship_x, ship_z, ship_yaw)
    })
}

/// Like `radar_dots` but only includes entities whose tags match **at least
/// one** tag in `filter_tags` (OR logic).  If `filter_tags` is empty, no
/// entities are returned.
pub fn radar_dots_filtered<'a>(
    entities: &'a [EntitySnapshot],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    filter_tags: &'a [EntityTag],
) -> impl Iterator<Item = (f32, f32, f32)> + 'a {
    entities.iter().filter_map(move |e| {
        let entity_tags = parse_tags(&e.tags);
        if matches_any(&entity_tags, filter_tags) {
            project_entity_as_asteroid(e, ship_x, ship_z, ship_yaw)
        } else {
            None
        }
    })
}

/// Project an entity to radar coordinates plus its scaled radius.
///
/// Returns `None` if the entity centre is outside `RADAR_RANGE`. The
/// scaled radius is in the same `[-1, 1]` units as the position (i.e. the
/// entity's world radius divided by `RADAR_RANGE`).
pub fn project_entity_as_asteroid(
    entity: &EntitySnapshot,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> Option<(f32, f32, f32)> {
    let (rx, ry) = project_to_radar(entity.x(), entity.z(), ship_x, ship_z, ship_yaw)?;
    Some((rx, ry, entity.radius_or_zero() / RADAR_RANGE))
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
    entities: &'a [EntitySnapshot],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    config: &'a RadarConfig,
) -> impl Iterator<Item = (f32, f32, f32)> + 'a {
    entities.iter().filter_map(move |e| {
        let entity_tags = parse_tags(&e.tags);
        if !matches_any(&entity_tags, &config.shows) {
            return None;
        }
        let (rx, ry) =
            project_to_radar_with_config(e.x(), e.z(), ship_x, ship_z, ship_yaw, config)?;
        Some((rx, ry, e.radius_or_zero() / config.range))
    })
}

/// Project an asteroid field entity to radar ring coordinates.
///
/// Returns `Some((centre_x, centre_y, inner_r, outer_r))` where all values are
/// normalised to `[-1, 1]` units (i.e. divided by `RADAR_RANGE`).
///
/// Returns `None` only if the *entire* ring is completely outside radar range
/// — i.e., the distance from the ship to the entity centre minus the outer
/// radius is greater than `RADAR_RANGE`.  Rings that partially overlap the
/// radar view are returned (the renderer clips them).
pub fn project_entity_as_field(
    entity: &EntitySnapshot,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> Option<(f32, f32, f32, f32)> {
    let dx = entity.x() - ship_x;
    let dz = entity.z() - ship_z;
    let dist = (dx * dx + dz * dz).sqrt();
    let outer_r = entity.radius_or_zero();
    let inner_r = entity.inner_radius_or_zero();

    // Cull only when even the near edge of the ring is beyond radar range.
    if dist - outer_r > RADAR_RANGE {
        return None;
    }

    let cos_y = ship_yaw.cos();
    let sin_y = ship_yaw.sin();
    let centre_x = (dx * cos_y + dz * sin_y) / RADAR_RANGE;
    let centre_y = (dx * sin_y - dz * cos_y) / RADAR_RANGE;

    Some((
        centre_x,
        centre_y,
        inner_r / RADAR_RANGE,
        outer_r / RADAR_RANGE,
    ))
}

/// Iterator of `(centre_x, centre_y, inner_r, outer_r)` tuples (all in radar
/// unit-square coordinates) for all `EntitySnapshot` values tagged
/// `asteroid_field` that are at least partially within `RADAR_RANGE` of the ship.
pub fn radar_rings<'a>(
    entities: &'a [EntitySnapshot],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> impl Iterator<Item = (f32, f32, f32, f32)> + 'a {
    entities.iter().filter_map(move |e| {
        let tags = parse_tags(&e.tags);
        if !tags.contains(&EntityTag::AsteroidField) {
            return None;
        }
        project_entity_as_field(e, ship_x, ship_z, ship_yaw)
    })
}

// ── Sensors / Science radar tab view model ───────────────────────────────────

/// A single entity dot on the Science long-range radar.
#[derive(Clone, Debug, PartialEq)]
pub struct ScienceRadarDot {
    /// Stable entity UUID — used when the player taps the dot to suggest a target.
    pub uuid: String,
    /// Radar-space X coordinate normalised to `[-1.0, 1.0]` at `config.range`.
    pub radar_x: f32,
    /// Radar-space Y coordinate normalised to `[-1.0, 1.0]` at `config.range`.
    pub radar_y: f32,
    /// Scaled radius: `world_radius / config.range`.
    pub scaled_radius: f32,
}

/// An asteroid field ring on the Science long-range radar.
#[derive(Clone, Debug, PartialEq)]
pub struct ScienceRadarRing {
    /// Stable entity UUID — used when the player taps the ring to suggest a target.
    pub uuid: String,
    /// Radar-space X of the ring centre, normalised to `[-1.0, 1.0]`.
    pub centre_x: f32,
    /// Radar-space Y of the ring centre, normalised to `[-1.0, 1.0]`.
    pub centre_y: f32,
    /// Inner radius normalised to the same scale as `centre_x`/`centre_y`.
    pub inner_r: f32,
    /// Outer radius normalised to the same scale as `centre_x`/`centre_y`.
    pub outer_r: f32,
}

/// The complete render data for a Science console long-range radar frame.
///
/// Produced by `compute_science_radar_view` — a pure function that takes the
/// current world snapshot and ship pose and returns everything the Science
/// panel renderer needs to draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScienceRadarView {
    /// Individual entity dots (asteroids, ships, etc.) within config range
    /// whose tags overlap `config.shows`.
    pub dots: Vec<ScienceRadarDot>,
    /// Asteroid field rings within config range (filtered by `config.shows`).
    pub rings: Vec<ScienceRadarRing>,
}

/// Compute the Science console long-range radar view.
///
/// - `entities`: all entities from `WorldData`.
/// - `ship_x`, `ship_z`, `ship_yaw`: current ship pose.
/// - `config`: Science radar configuration (range + tag filter).
///
/// Entities are filtered by both range (`config.range`) and tag (`config.shows`
/// OR logic).  Fields are projected using `RADAR_RANGE`-normalised units
/// re-scaled to `config.range`.
pub fn compute_science_radar_view(
    entities: &[EntitySnapshot],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    config: &RadarConfig,
) -> ScienceRadarView {
    let dots = entities
        .iter()
        .filter_map(|e| {
            let entity_tags = crate::entity_tags::parse_tags(&e.tags);
            if !crate::entity_tags::matches_any(&entity_tags, &config.shows) {
                return None;
            }
            // Skip field-type entities (they become rings, not dots).
            if entity_tags.contains(&EntityTag::AsteroidField) {
                return None;
            }
            let (rx, ry) =
                project_to_radar_with_config(e.x(), e.z(), ship_x, ship_z, ship_yaw, config)?;
            Some(ScienceRadarDot {
                uuid: e.uuid.clone(),
                radar_x: rx,
                radar_y: ry,
                scaled_radius: e.radius_or_zero() / config.range,
            })
        })
        .collect();

    let rings = entities
        .iter()
        .filter_map(|e| {
            let entity_tags = crate::entity_tags::parse_tags(&e.tags);
            if !crate::entity_tags::matches_any(&entity_tags, &config.shows) {
                return None;
            }
            // Only field-type entities become rings.
            if !entity_tags.contains(&EntityTag::AsteroidField) {
                return None;
            }
            // Cull: if even the near edge of the ring is beyond config.range, skip.
            let dx = e.x() - ship_x;
            let dz = e.z() - ship_z;
            let dist = (dx * dx + dz * dz).sqrt();
            let outer_r = e.radius_or_zero();
            if dist - outer_r > config.range {
                return None;
            }
            let cos_y = ship_yaw.cos();
            let sin_y = ship_yaw.sin();
            let centre_x = (dx * cos_y + dz * sin_y) / config.range;
            let centre_y = (dx * sin_y - dz * cos_y) / config.range;
            Some(ScienceRadarRing {
                uuid: e.uuid.clone(),
                centre_x,
                centre_y,
                inner_r: e.inner_radius_or_zero() / config.range,
                outer_r: outer_r / config.range,
            })
        })
        .collect();

    ScienceRadarView { dots, rings }
}

/// Range used by the Navigation console system chart — large enough to show the
/// full solar system layout. Matches the old Science system map range.
/// Range used by the Science console system chart — large enough to show the
/// full solar system layout.
pub const SYSTEM_CHART_RANGE: f32 = 500.0;

/// Maximum range for the Science console long-range radar at full power.
pub const SCIENCE_RADAR_RANGE: f32 = 200.0;

pub const NAVIGATION_CHART_RANGE: f32 = 500.0;

/// Returns the `RadarConfig` for the Navigation console system chart tab.
///
/// Shows navigational entities: stars, planets, asteroid field rings, and
/// regions. Individual asteroids are excluded (they are not navigational
/// features).
pub fn navigation_chart_config() -> RadarConfig {
    RadarConfig {
        range: NAVIGATION_CHART_RANGE,
        shows: vec![
            EntityTag::Star,
            EntityTag::Planet,
            EntityTag::AsteroidField,
            EntityTag::Region,
        ],
    }
}

/// Compute the Navigation console system chart view.
///
/// - `entities`: all entities from `WorldData`.
/// - `ship_x`, `ship_z`, `ship_yaw`: current ship pose.
/// - `config`: Navigation chart configuration (range + tag filter).
///
/// Returns dots and rings for navigational entities (stars, planets, asteroid
/// fields, regions) within `config.range` of the ship. Individual asteroids
/// are excluded.
pub fn compute_navigation_system_chart(
    entities: &[EntitySnapshot],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    config: &RadarConfig,
) -> ScienceRadarView {
    compute_science_radar_view(entities, ship_x, ship_z, ship_yaw, config)
}

/// UUID sentinel used to identify the ship dot inside a `ScienceRadarView`
/// returned by `compute_star_centred_nav_chart`.
pub const SHIP_DOT_UUID: &str = "__ship__";

/// Compute the Navigation console system chart centred on the star.
///
/// Unlike `compute_navigation_system_chart` (which is ship-centred and
/// rotates everything by ship yaw), this function:
///
/// - Centers the projected view on the first entity tagged `"star"` found in
///   `entities`.  If no star is present the origin (0, 0) is used.
/// - Uses a **fixed, north-up** orientation — no yaw rotation is applied.
///   Positive radar-Y is world +X and negative radar-Y is world -X; world Z
///   maps to radar-X.  Concretely: `radar_x = (world_x - star_x) / range`,
///   `radar_y = -(world_z - star_z) / range` so that "up" on screen is the
///   negative-Z world direction (conventional "north").
/// - Includes the ship as a dot with uuid [`SHIP_DOT_UUID`] at its world
///   position relative to the star.  The caller can detect this sentinel and
///   render it differently (e.g. as a heading triangle).
///
/// Returns a [`ScienceRadarView`] so existing rendering code can be reused.
pub fn compute_star_centred_nav_chart(
    entities: &[EntitySnapshot],
    ship_x: f32,
    ship_z: f32,
    config: &RadarConfig,
) -> ScienceRadarView {
    // Find the star to use as the chart origin.
    let (star_x, star_z) = entities
        .iter()
        .find(|e| crate::entity_tags::parse_tags(&e.tags).contains(&EntityTag::Star))
        .map(|e| (e.x(), e.z()))
        .unwrap_or((0.0, 0.0));

    let range = config.range;

    // Project a world position to star-centred, north-up radar coordinates.
    // radar_x = (world_x - star_x) / range
    // radar_y = -(world_z - star_z) / range   (negate Z so north = screen-up)
    let project = |wx: f32, wz: f32| -> Option<(f32, f32)> {
        let dx = wx - star_x;
        let dz = wz - star_z;
        if dx * dx + dz * dz > range * range {
            return None;
        }
        Some((dx / range, -dz / range))
    };

    let dots = entities
        .iter()
        .filter_map(|e| {
            let entity_tags = crate::entity_tags::parse_tags(&e.tags);
            if !crate::entity_tags::matches_any(&entity_tags, &config.shows) {
                return None;
            }
            if entity_tags.contains(&EntityTag::AsteroidField) {
                return None;
            }
            let (rx, ry) = project(e.x(), e.z())?;
            Some(ScienceRadarDot {
                uuid: e.uuid.clone(),
                radar_x: rx,
                radar_y: ry,
                scaled_radius: e.radius_or_zero() / range,
            })
        })
        .chain(
            std::iter::once_with(|| {
                // Always include the ship as a sentinel dot.
                let (rx, ry) = project(ship_x, ship_z)?;
                Some(ScienceRadarDot {
                    uuid: SHIP_DOT_UUID.to_string(),
                    radar_x: rx,
                    radar_y: ry,
                    scaled_radius: 0.0,
                })
            })
            .flatten(),
        )
        .collect();

    let rings = entities
        .iter()
        .filter_map(|e| {
            let entity_tags = crate::entity_tags::parse_tags(&e.tags);
            if !crate::entity_tags::matches_any(&entity_tags, &config.shows) {
                return None;
            }
            if !entity_tags.contains(&EntityTag::AsteroidField) {
                return None;
            }
            let dx = e.x() - star_x;
            let dz = e.z() - star_z;
            let dist = (dx * dx + dz * dz).sqrt();
            let outer_r = e.radius_or_zero();
            if dist - outer_r > range {
                return None;
            }
            // North-up: no yaw rotation.
            let centre_x = dx / range;
            let centre_y = -dz / range;
            Some(ScienceRadarRing {
                uuid: e.uuid.clone(),
                centre_x,
                centre_y,
                inner_r: e.inner_radius_or_zero() / range,
                outer_r: outer_r / range,
            })
        })
        .collect();

    ScienceRadarView { dots, rings }
}

/// Compute the Sensors console long-range radar view.
///
/// Identical semantics to `compute_science_radar_view` — delegates to it
/// directly.  A dedicated entry point is provided so the call site reads as
/// "Sensors" rather than "Science", matching the renamed console.
///
/// - `entities`: all entities from `WorldData`.
/// - `ship_x`, `ship_z`, `ship_yaw`: current ship pose.
/// - `config`: Sensors radar configuration (range + tag filter).
pub fn compute_sensors_radar_view(
    entities: &[EntitySnapshot],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    config: &RadarConfig,
) -> ScienceRadarView {
    compute_science_radar_view(entities, ship_x, ship_z, ship_yaw, config)
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
    fn project_entity_as_asteroid_scales_radius_by_range() {
        let a = EntitySnapshot::asteroid("", 0.0, -10.0, 2.0);
        let (_rx, _ry, r) = project_entity_as_asteroid(&a, 0.0, 0.0, 0.0).unwrap();
        close(r, 2.0 / RADAR_RANGE);
    }

    #[test]
    fn project_entity_as_asteroid_outside_range_returns_none() {
        let a = EntitySnapshot::asteroid("", 100.0, 100.0, 2.0);
        assert!(project_entity_as_asteroid(&a, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn radar_constants_have_expected_values() {
        // PRD pinned values; locking these in protects the ring renderer.
        assert_eq!(RADAR_RANGE, 50.0);
        assert_eq!(RADAR_MID_RING, 25.0);
    }

    fn entity_at(x: f32, z: f32, tags: &[&str]) -> EntitySnapshot {
        EntitySnapshot {
            uuid: "".into(),
            id: None,
            position: Some([x, 0.0, z]),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            shape: None,
            radius: Some(1.0),
            colour: None,
            yaw: None,
            hull_fraction: None,
            inner_radius: None,
            warp_out_remaining_secs: None,
            radar_world_size: None,
        }
    }

    #[test]
    fn radar_dots_single_in_range_matches_project_entity() {
        let e = entity_at(0.0, -10.0, &["asteroid"]);
        let dots: Vec<_> = radar_dots(&[e.clone()], 0.0, 0.0, 0.0).collect();
        assert_eq!(dots.len(), 1);
        let expected = project_entity_as_asteroid(&e, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(dots[0], expected);
    }

    #[test]
    fn radar_dots_skips_out_of_range_asteroids() {
        let far = entity_at(100.0, 100.0, &["asteroid"]);
        let near = entity_at(0.0, -10.0, &["asteroid"]);
        let dots: Vec<_> = radar_dots(&[far, near.clone()], 0.0, 0.0, 0.0).collect();
        assert_eq!(dots.len(), 1);
        assert_eq!(
            dots[0],
            project_entity_as_asteroid(&near, 0.0, 0.0, 0.0).unwrap()
        );
    }

    #[test]
    fn radar_dots_empty_slice_returns_empty_iterator() {
        let dots: Vec<_> = radar_dots(&[], 0.0, 0.0, 0.0).collect();
        assert!(dots.is_empty());
    }

    #[test]
    fn radar_dots_skips_non_asteroid_entities() {
        let ship = entity_at(0.0, -10.0, &["ship"]);
        let dots: Vec<_> = radar_dots(&[ship], 0.0, 0.0, 0.0).collect();
        assert!(
            dots.is_empty(),
            "radar_dots should only show entities tagged 'asteroid'"
        );
    }

    // ── radar_dots_filtered ────────────────────────────────────────────────

    #[test]
    fn radar_dots_filtered_returns_only_matching_tag_entities() {
        let asteroid = entity_at(0.0, -10.0, &["asteroid"]);
        let ship = entity_at(0.0, -15.0, &["ship"]);
        let filter = vec![EntityTag::Asteroid];
        let dots: Vec<_> =
            radar_dots_filtered(&[asteroid.clone(), ship], 0.0, 0.0, 0.0, &filter).collect();
        assert_eq!(dots.len(), 1);
        assert_eq!(
            dots[0],
            project_entity_as_asteroid(&asteroid, 0.0, 0.0, 0.0).unwrap()
        );
    }

    #[test]
    fn radar_dots_filtered_or_logic_includes_either_tag() {
        let a = entity_at(0.0, -10.0, &["asteroid"]);
        let s = entity_at(5.0, -10.0, &["ship"]);
        let filter = vec![EntityTag::Asteroid, EntityTag::Ship];
        let dots: Vec<_> = radar_dots_filtered(&[a, s], 0.0, 0.0, 0.0, &filter).collect();
        assert_eq!(dots.len(), 2);
    }

    #[test]
    fn radar_dots_filtered_empty_filter_returns_nothing() {
        let a = entity_at(0.0, -10.0, &["asteroid"]);
        let dots: Vec<_> = radar_dots_filtered(&[a], 0.0, 0.0, 0.0, &[]).collect();
        assert!(dots.is_empty());
    }

    #[test]
    fn radar_dots_filtered_skips_out_of_range_even_if_tag_matches() {
        let far = entity_at(100.0, 100.0, &["asteroid"]);
        let filter = vec![EntityTag::Asteroid];
        let dots: Vec<_> = radar_dots_filtered(&[far], 0.0, 0.0, 0.0, &filter).collect();
        assert!(dots.is_empty());
    }

    #[test]
    fn radar_dots_filtered_skips_no_tag_entity() {
        let a = entity_at(0.0, -10.0, &[]);
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
        let cfg = RadarConfig {
            range: 30.0,
            shows: vec![EntityTag::Asteroid],
        };
        assert!(project_to_radar_with_config(0.0, -35.0, 0.0, 0.0, 0.0, &cfg).is_none());
        // Same point is within the global RADAR_RANGE (50) — ensure we use config range.
        assert!(project_to_radar(0.0, -35.0, 0.0, 0.0, 0.0).is_some());
    }

    #[test]
    fn config_project_normalises_to_config_range() {
        // Point at config.range ahead should map to radar_y = 1.0.
        let cfg = RadarConfig {
            range: 80.0,
            shows: vec![],
        };
        let p = project_to_radar_with_config(0.0, -80.0, 0.0, 0.0, 0.0, &cfg).unwrap();
        close(p.1, 1.0);
    }

    // ── radar_dots_with_config ────────────────────────────────────────────

    #[test]
    fn dots_with_config_filters_by_shows() {
        let asteroid = entity_at(0.0, -10.0, &["asteroid"]);
        let ship_entity = entity_at(5.0, -10.0, &["ship"]);
        let cfg = RadarConfig {
            range: 50.0,
            shows: vec![EntityTag::Asteroid],
        };
        let dots: Vec<_> =
            radar_dots_with_config(&[asteroid.clone(), ship_entity], 0.0, 0.0, 0.0, &cfg).collect();
        assert_eq!(dots.len(), 1);
        // Scaled radius should use config.range.
        close(dots[0].2, 1.0 / cfg.range);
    }

    #[test]
    fn dots_with_config_empty_shows_returns_nothing() {
        let asteroid = entity_at(0.0, -10.0, &["asteroid"]);
        let cfg = RadarConfig {
            range: 50.0,
            shows: vec![],
        };
        let dots: Vec<_> = radar_dots_with_config(&[asteroid], 0.0, 0.0, 0.0, &cfg).collect();
        assert!(dots.is_empty());
    }

    #[test]
    fn dots_with_config_clips_at_config_range_not_global() {
        // Asteroid at 40 units (inside RADAR_RANGE=50 but outside config range=30).
        let asteroid = entity_at(0.0, -40.0, &["asteroid"]);
        let cfg = RadarConfig {
            range: 30.0,
            shows: vec![EntityTag::Asteroid],
        };
        let dots: Vec<_> = radar_dots_with_config(&[asteroid], 0.0, 0.0, 0.0, &cfg).collect();
        assert!(dots.is_empty());
    }

    #[test]
    fn dots_with_config_or_logic_for_multiple_shows() {
        let a = entity_at(0.0, -10.0, &["asteroid"]);
        let s = entity_at(5.0, -10.0, &["ship"]);
        let cfg = RadarConfig {
            range: 50.0,
            shows: vec![EntityTag::Asteroid, EntityTag::Ship],
        };
        let dots: Vec<_> = radar_dots_with_config(&[a, s], 0.0, 0.0, 0.0, &cfg).collect();
        assert_eq!(dots.len(), 2);
    }

    // ── project_entity_as_field / radar_rings ─────────────────────────────

    fn field_entity(x: f32, z: f32, inner: f32, outer: f32) -> EntitySnapshot {
        EntitySnapshot {
            uuid: "field-1".into(),
            id: None,
            position: Some([x, 0.0, z]),
            tags: vec!["asteroid_field".into()],
            shape: None,
            radius: Some(outer),
            colour: None,
            yaw: None,
            hull_fraction: None,
            inner_radius: Some(inner),
            warp_out_remaining_secs: None,
            radar_world_size: None,
        }
    }

    #[test]
    fn project_entity_as_field_centred_on_ship_maps_to_origin() {
        let field = field_entity(0.0, 0.0, 5.0, 15.0);
        let (cx, cy, ir, or_) = project_entity_as_field(&field, 0.0, 0.0, 0.0).unwrap();
        close(cx, 0.0);
        close(cy, 0.0);
        close(ir, 5.0 / RADAR_RANGE);
        close(or_, 15.0 / RADAR_RANGE);
    }

    #[test]
    fn project_entity_as_field_ahead_maps_to_positive_y() {
        let field = field_entity(0.0, -25.0, 5.0, 10.0);
        let (cx, cy, _ir, _or) = project_entity_as_field(&field, 0.0, 0.0, 0.0).unwrap();
        close(cx, 0.0);
        close(cy, 0.5);
    }

    #[test]
    fn project_entity_as_field_fully_out_of_range_returns_none() {
        // Field centre at 100 units, outer radius 5 → nearest edge at 95 > 50.
        let field = field_entity(100.0, 0.0, 3.0, 5.0);
        assert!(project_entity_as_field(&field, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn project_entity_as_field_partially_in_range_returns_some() {
        // Field centre at 55 units, outer radius 10 → near edge at 45 < 50.
        let field = field_entity(55.0, 0.0, 5.0, 10.0);
        assert!(project_entity_as_field(&field, 0.0, 0.0, 0.0).is_some());
    }

    #[test]
    fn radar_rings_skips_fully_out_of_range_fields() {
        let far = field_entity(200.0, 0.0, 5.0, 10.0);
        let near = field_entity(0.0, -25.0, 5.0, 10.0);
        let rings: Vec<_> = radar_rings(&[far, near.clone()], 0.0, 0.0, 0.0).collect();
        assert_eq!(rings.len(), 1);
        let expected = project_entity_as_field(&near, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(rings[0], expected);
    }

    #[test]
    fn radar_rings_empty_slice_returns_empty() {
        let rings: Vec<_> = radar_rings(&[], 0.0, 0.0, 0.0).collect();
        assert!(rings.is_empty());
    }

    #[test]
    fn radar_rings_skips_non_field_entities() {
        let ast = entity_at(0.0, -10.0, &["asteroid"]);
        let rings: Vec<_> = radar_rings(&[ast], 0.0, 0.0, 0.0).collect();
        assert!(
            rings.is_empty(),
            "radar_rings should only show entities tagged 'asteroid_field'"
        );
    }

    // ── ScienceRadarView ──────────────────────────────────────────────────

    fn science_config() -> RadarConfig {
        RadarConfig {
            range: 100.0,
            shows: vec![EntityTag::Asteroid, EntityTag::AsteroidField],
        }
    }

    fn ast_entity(uuid: &str, x: f32, z: f32) -> EntitySnapshot {
        EntitySnapshot::asteroid(uuid, x, z, 1.0)
    }

    fn fld_entity(uuid: &str, x: f32, z: f32) -> EntitySnapshot {
        EntitySnapshot::asteroid_field(uuid, x, z, 5.0, 15.0)
    }

    #[test]
    fn science_radar_view_empty_world_produces_empty_view() {
        let view = compute_science_radar_view(&[], 0.0, 0.0, 0.0, &science_config());
        assert!(view.dots.is_empty());
        assert!(view.rings.is_empty());
    }

    #[test]
    fn science_radar_view_includes_asteroid_dot_within_range() {
        let a = ast_entity("a1", 0.0, -50.0);
        let view = compute_science_radar_view(&[a], 0.0, 0.0, 0.0, &science_config());
        assert_eq!(view.dots.len(), 1);
        assert_eq!(view.dots[0].uuid, "a1");
    }

    #[test]
    fn science_radar_view_excludes_asteroid_outside_range() {
        let a = ast_entity("far", 0.0, -200.0);
        let view = compute_science_radar_view(&[a], 0.0, 0.0, 0.0, &science_config());
        assert!(view.dots.is_empty());
    }

    #[test]
    fn science_radar_view_includes_field_ring_within_range() {
        let f = fld_entity("f1", 0.0, -50.0);
        let view = compute_science_radar_view(&[f], 0.0, 0.0, 0.0, &science_config());
        assert_eq!(view.rings.len(), 1);
        assert_eq!(view.rings[0].uuid, "f1");
    }

    #[test]
    fn science_radar_view_excludes_entity_not_in_shows() {
        // Config only shows asteroids; a "ship" tagged entity is excluded.
        let cfg = RadarConfig {
            range: 100.0,
            shows: vec![EntityTag::Asteroid],
        };
        let ship = EntitySnapshot::simple("s1", 0.0, -20.0, vec!["ship".into()]);
        let view = compute_science_radar_view(&[ship], 0.0, 0.0, 0.0, &cfg);
        assert!(view.dots.is_empty());
    }

    #[test]
    fn science_radar_dot_position_is_normalised_to_config_range() {
        // Asteroid at exactly config.range ahead → radar_y = 1.0.
        let cfg = RadarConfig {
            range: 80.0,
            shows: vec![EntityTag::Asteroid],
        };
        let a = ast_entity("a2", 0.0, -80.0);
        let view = compute_science_radar_view(&[a], 0.0, 0.0, 0.0, &cfg);
        assert_eq!(view.dots.len(), 1);
        close(view.dots[0].radar_y, 1.0);
    }

    #[test]
    fn science_radar_view_dot_carries_original_uuid() {
        let a = ast_entity("uuid-xyz", 0.0, -30.0);
        let view = compute_science_radar_view(&[a], 0.0, 0.0, 0.0, &science_config());
        assert_eq!(view.dots[0].uuid, "uuid-xyz");
    }

    #[test]
    fn science_radar_view_separates_dots_and_rings() {
        let ast = ast_entity("ast-1", 0.0, -30.0);
        let fld = fld_entity("fld-1", 0.0, -30.0);
        let view = compute_science_radar_view(&[ast, fld], 0.0, 0.0, 0.0, &science_config());
        assert_eq!(view.dots.len(), 1, "asteroid should be a dot");
        assert_eq!(view.dots[0].uuid, "ast-1");
        assert_eq!(view.rings.len(), 1, "field should be a ring");
        assert_eq!(view.rings[0].uuid, "fld-1");
    }

    // ── compute_sensors_radar_view ────────────────────────────────────────

    #[test]
    fn sensors_radar_view_produces_same_output_as_science_radar_view() {
        // compute_sensors_radar_view is a thin wrapper; verify parity.
        let entities = vec![
            ast_entity("ast-sensors-1", 0.0, -30.0),
            fld_entity("fld-sensors-1", 0.0, -30.0),
        ];
        let cfg = science_config();
        let science_view = compute_science_radar_view(&entities, 0.0, 0.0, 0.0, &cfg);
        let sensors_view = compute_sensors_radar_view(&entities, 0.0, 0.0, 0.0, &cfg);
        assert_eq!(science_view, sensors_view);
    }

    #[test]
    fn sensors_radar_view_empty_world_produces_empty_view() {
        let view = compute_sensors_radar_view(&[], 0.0, 0.0, 0.0, &science_config());
        assert!(view.dots.is_empty());
        assert!(view.rings.is_empty());
    }

    #[test]
    fn sensors_radar_view_includes_asteroid_within_config_range() {
        let a = ast_entity("sensors-ast", 0.0, -50.0);
        let view = compute_sensors_radar_view(&[a], 0.0, 0.0, 0.0, &science_config());
        assert_eq!(view.dots.len(), 1);
        assert_eq!(view.dots[0].uuid, "sensors-ast");
    }

    #[test]
    fn sensors_radar_view_excludes_entity_beyond_config_range() {
        let a = ast_entity("sensors-far", 0.0, -200.0);
        let view = compute_sensors_radar_view(&[a], 0.0, 0.0, 0.0, &science_config());
        assert!(view.dots.is_empty());
    }

    // ── compute_navigation_system_chart ─────────────────────────────────

    fn nav_config() -> RadarConfig {
        RadarConfig {
            range: NAVIGATION_CHART_RANGE,
            shows: vec![
                EntityTag::Star,
                EntityTag::Planet,
                EntityTag::AsteroidField,
                EntityTag::Region,
            ],
        }
    }

    fn star_entity(uuid: &str, x: f32, z: f32) -> EntitySnapshot {
        EntitySnapshot::simple(uuid, x, z, vec!["star".into()])
    }

    fn planet_entity(uuid: &str, x: f32, z: f32) -> EntitySnapshot {
        EntitySnapshot::simple(uuid, x, z, vec!["planet".into()])
    }

    #[test]
    fn navigation_chart_empty_world_produces_empty_view() {
        let view = compute_navigation_system_chart(&[], 0.0, 0.0, 0.0, &nav_config());
        assert!(view.dots.is_empty());
        assert!(view.rings.is_empty());
    }

    #[test]
    fn navigation_chart_includes_star_within_range() {
        let s = star_entity("sun", 0.0, -100.0);
        let view = compute_navigation_system_chart(&[s], 0.0, 0.0, 0.0, &nav_config());
        assert_eq!(view.dots.len(), 1);
        assert_eq!(view.dots[0].uuid, "sun");
    }

    #[test]
    fn navigation_chart_includes_planet_within_range() {
        let p = planet_entity("mars", 0.0, -250.0);
        let view = compute_navigation_system_chart(&[p], 0.0, 0.0, 0.0, &nav_config());
        assert_eq!(view.dots.len(), 1);
        assert_eq!(view.dots[0].uuid, "mars");
    }

    #[test]
    fn navigation_chart_includes_field_ring_within_range() {
        let f = fld_entity("belt", 0.0, -100.0);
        let view = compute_navigation_system_chart(&[f], 0.0, 0.0, 0.0, &nav_config());
        assert_eq!(view.rings.len(), 1);
        assert_eq!(view.rings[0].uuid, "belt");
    }

    #[test]
    fn navigation_chart_excludes_individual_asteroid() {
        let a = ast_entity("ast-1", 0.0, -50.0);
        let view = compute_navigation_system_chart(&[a], 0.0, 0.0, 0.0, &nav_config());
        assert!(
            view.dots.is_empty(),
            "individual asteroids must not appear on navigation chart"
        );
    }

    #[test]
    fn navigation_chart_excludes_entity_beyond_range() {
        let s = star_entity("far-star", 0.0, -(NAVIGATION_CHART_RANGE + 100.0));
        let view = compute_navigation_system_chart(&[s], 0.0, 0.0, 0.0, &nav_config());
        assert!(view.dots.is_empty());
    }

    #[test]
    fn navigation_chart_config_range_is_five_hundred() {
        assert_eq!(NAVIGATION_CHART_RANGE, 500.0);
    }

    #[test]
    fn navigation_chart_config_shows_star_planet_field_region() {
        let cfg = nav_config();
        assert!(cfg.shows.contains(&EntityTag::Star));
        assert!(cfg.shows.contains(&EntityTag::Planet));
        assert!(cfg.shows.contains(&EntityTag::AsteroidField));
        assert!(cfg.shows.contains(&EntityTag::Region));
    }

    // ── compute_star_centred_nav_chart ────────────────────────────────────

    fn star_centred_config() -> RadarConfig {
        RadarConfig {
            range: 500.0,
            shows: vec![
                EntityTag::Star,
                EntityTag::Planet,
                EntityTag::AsteroidField,
                EntityTag::Region,
            ],
        }
    }

    /// Star at (0,0), planet at (100,50), ship at (30,-20).
    /// The chart should be centred on the star with no yaw rotation.
    #[test]
    fn star_centred_nav_chart_star_at_origin() {
        let star = EntitySnapshot::simple("sun", 0.0, 0.0, vec!["star".into()]);
        let planet = EntitySnapshot::simple("mars", 100.0, 50.0, vec!["planet".into()]);
        let cfg = star_centred_config();
        let view = compute_star_centred_nav_chart(&[star, planet], 30.0, -20.0, &cfg);

        let star_dot = view
            .dots
            .iter()
            .find(|d| d.uuid == "sun")
            .expect("star must appear as a dot");
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
        close(star_dot.radar_x, 0.0);
        close(star_dot.radar_y, 0.0);
    }

    #[test]
    fn star_centred_nav_chart_planet_at_correct_position() {
        let star = EntitySnapshot::simple("sun", 0.0, 0.0, vec!["star".into()]);
        let planet = EntitySnapshot::simple("mars", 100.0, 50.0, vec!["planet".into()]);
        let cfg = star_centred_config();
        let view = compute_star_centred_nav_chart(&[star, planet], 30.0, -20.0, &cfg);

        let planet_dot = view
            .dots
            .iter()
            .find(|d| d.uuid == "mars")
            .expect("planet must appear as a dot");
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
        // radar_x = (100 - 0) / 500, radar_y = -(50 - 0) / 500
        close(planet_dot.radar_x, 100.0 / 500.0);
        close(planet_dot.radar_y, -50.0 / 500.0);
    }

    #[test]
    fn star_centred_nav_chart_ship_dot_at_correct_position() {
        let star = EntitySnapshot::simple("sun", 0.0, 0.0, vec!["star".into()]);
        let cfg = star_centred_config();
        // Ship at (30, -20) world.
        let view = compute_star_centred_nav_chart(&[star], 30.0, -20.0, &cfg);

        let ship_dot = view
            .dots
            .iter()
            .find(|d| d.uuid == SHIP_DOT_UUID)
            .expect("ship sentinel dot must be present");
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
        // radar_x = (30 - 0) / 500, radar_y = -(-20 - 0) / 500
        close(ship_dot.radar_x, 30.0 / 500.0);
        close(ship_dot.radar_y, 20.0 / 500.0);
    }

    /// Verify that yaw has NO effect on the star-centred chart (north always up).
    #[test]
    fn star_centred_nav_chart_ignores_ship_yaw() {
        let star = EntitySnapshot::simple("sun", 0.0, 0.0, vec!["star".into()]);
        let planet = EntitySnapshot::simple("mars", 100.0, 0.0, vec!["planet".into()]);
        let cfg = star_centred_config();

        // Call with ship yaw = 0 and ship yaw = π/2; planet position must be identical.
        let view_yaw0 =
            compute_star_centred_nav_chart(&[star.clone(), planet.clone()], 0.0, 0.0, &cfg);
        let view_yaw90 =
            compute_star_centred_nav_chart(&[star.clone(), planet.clone()], 0.0, 0.0, &cfg);

        let dot0 = view_yaw0.dots.iter().find(|d| d.uuid == "mars").unwrap();
        let dot90 = view_yaw90.dots.iter().find(|d| d.uuid == "mars").unwrap();
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
        close(dot0.radar_x, dot90.radar_x);
        close(dot0.radar_y, dot90.radar_y);
    }
}
