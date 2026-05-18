//! `GenericRadar` widget — gizmo-based radar with configurable range, orientation,
//! and per-layer entity filtering.
//!
//! Game-world entities opt in by carrying `OnRadar(RadarLayer)` and
//! `RadarAppearance`.  The reference entity (player ship) carries `RadarCenter`.
//! All drawing is done via Bevy gizmos each frame; the UI node provides only the
//! layout footprint.

use bevy::prelude::*;
use std::collections::HashSet;

// ── Layer / filter ────────────────────────────────────────────────────────────

/// Entity classification for radar layer filtering.
#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
pub enum RadarLayer {
    Ship,
    Asteroid,
    Station,
    Missile,
}

/// Opts a game-world entity into the radar.  The widget draws only entities
/// whose layer passes the widget's `RadarFilter`.
#[derive(Component, Clone, Debug)]
pub struct OnRadar(pub RadarLayer);

/// Component on the radar widget node: which `RadarLayer` values to draw.
#[derive(Component, Clone, Debug)]
pub struct RadarFilter(pub HashSet<RadarLayer>);

// ── Appearance ────────────────────────────────────────────────────────────────

/// How an `OnRadar` entity is rendered on the radar.
#[derive(Clone, Debug)]
pub enum RadarShape {
    Dot,
    Triangle,
    Ring,
    Icon(Handle<Image>),
}

/// Appearance for a radar blip.
#[derive(Component, Clone, Debug)]
pub struct RadarAppearance {
    pub color: Color,
    /// Radius of the drawn shape in pixels (scaled to widget size).
    pub radius: f32,
    pub shape: RadarShape,
}

// ── Orientation mode ──────────────────────────────────────────────────────────

/// Controls how the radar coordinate frame relates to ship heading.
#[derive(Clone, Debug, PartialEq)]
pub enum OrientationMode {
    /// The ship always points "up" — the gizmo frame rotates by ship yaw each frame.
    ShipRelative,
    /// North is always "up" regardless of ship heading.
    WorldFixed,
}

// ── Widget component ──────────────────────────────────────────────────────────

/// Component on the radar UI node.
#[derive(Component)]
pub struct GenericRadarWidget {
    pub range: f32,
    pub orientation: OrientationMode,
    pub filter: RadarFilter,
}

// ── Reference entity ──────────────────────────────────────────────────────────

/// Attach to the player ship (or any reference entity) so the radar widget
/// knows the center position and heading.
#[derive(Component, Default, Clone, Debug)]
pub struct RadarCenter {
    pub world_x: f32,
    pub world_z: f32,
    /// Ship heading in radians.  Ignored for `WorldFixed` orientation.
    pub yaw: f32,
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Returns `true` if `layer` is included in `filter`.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn is_on_radar(filter: &RadarFilter, layer: RadarLayer) -> bool {
    filter.0.contains(&layer)
}

/// Project a world-space entity position onto the radar's normalised coordinate
/// space `[-1.0, 1.0]`, or `None` if the entity is beyond `range`.
///
/// Mirrors the axis conventions of `crate::radar::project_to_radar`:
/// - X right, Y forward (ship-relative); for `WorldFixed`, ship yaw is
///   treated as zero.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn project_radar_entity(
    entity_x: f32,
    entity_z: f32,
    center_x: f32,
    center_z: f32,
    yaw: f32,
    range: f32,
    orientation: &OrientationMode,
) -> Option<(f32, f32)> {
    if range <= 0.0 {
        return None;
    }
    let dx = entity_x - center_x;
    let dz = entity_z - center_z;
    if dx * dx + dz * dz > range * range {
        return None;
    }
    let effective_yaw = match orientation {
        OrientationMode::ShipRelative => yaw,
        OrientationMode::WorldFixed => 0.0,
    };
    let cos_y = effective_yaw.cos();
    let sin_y = effective_yaw.sin();
    // Identical to project_to_radar math with configurable range.
    let radar_x = (dx * cos_y + dz * sin_y) / range;
    let radar_y = (dx * sin_y - dz * cos_y) / range;
    Some((radar_x, radar_y))
}

// ── Spawn helper ──────────────────────────────────────────────────────────────

/// Namespace struct for the `GenericRadar` widget.
pub struct GenericRadar;

impl GenericRadar {
    /// Spawn a `GenericRadar` widget.
    ///
    /// - `size` — width and height of the UI node in pixels.
    /// - `range` — world-unit radius drawn by the radar.
    /// - `orientation` — `ShipRelative` or `WorldFixed`.
    /// - `filter` — which `RadarLayer` values to draw.
    /// - `bg_image` / `overlay_image` — optional background and overlay images.
    ///
    /// Returns the UI node entity.
    pub fn spawn(
        commands: &mut Commands,
        size: f32,
        range: f32,
        orientation: OrientationMode,
        filter: RadarFilter,
        bg_image: Option<Handle<Image>>,
        overlay_image: Option<Handle<Image>>,
    ) -> Entity {
        let mut node = commands.spawn((
            GenericRadarWidget { range, orientation, filter },
            Node {
                width: Val::Px(size),
                height: Val::Px(size),
                position_type: PositionType::Relative,
                ..default()
            },
        ));
        if let Some(bg) = bg_image {
            node.insert(ImageNode::new(bg));
        }
        let entity = node.id();

        if let Some(overlay) = overlay_image {
            commands.entity(entity).with_children(|parent| {
                parent.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(size),
                        height: Val::Px(size),
                        ..default()
                    },
                    ImageNode::new(overlay),
                    ZIndex(1),
                ));
            });
        }

        entity
    }
}

// ── Draw system ───────────────────────────────────────────────────────────────

/// Each frame: for every `GenericRadarWidget` node, project and draw all
/// `OnRadar` + `RadarAppearance` entities that pass the filter.
fn draw_generic_radars(
    mut gizmos: Gizmos,
    radars: Query<(&GenericRadarWidget, &ComputedNode, &GlobalTransform, &ViewVisibility)>,
    blips: Query<(&OnRadar, &RadarAppearance, &GlobalTransform)>,
    centers: Query<&RadarCenter>,
    windows: Query<&Window>,
) {
    let Ok(window) = windows.single() else { return };
    let Ok(center) = centers.single() else { return };
    let viewport_w = window.width();
    let viewport_h = window.height();

    for (widget, computed, gt, vis) in radars.iter() {
        if !vis.get() {
            continue;
        }
        let node_size = computed.size();
        let radius = node_size.x.min(node_size.y) * 0.5;
        if radius <= 0.0 {
            continue;
        }

        // Convert UI node centre from viewport pixels to Camera2d world coordinates.
        let screen = gt.translation().truncate();
        let cx = screen.x - viewport_w / 2.0;
        let cy = viewport_h / 2.0 - screen.y;
        let widget_centre = Vec2::new(cx, cy);

        for (on_radar, appearance, blip_gtf) in blips.iter() {
            if !is_on_radar(&widget.filter, on_radar.0) {
                continue;
            }
            let bpos = blip_gtf.translation();
            let Some((nx, ny)) = project_radar_entity(
                bpos.x,
                bpos.z,
                center.world_x,
                center.world_z,
                center.yaw,
                widget.range,
                &widget.orientation,
            ) else {
                continue;
            };

            let draw_pos = widget_centre + Vec2::new(nx * radius, ny * radius);
            let r = appearance.radius.max(1.0);

            match &appearance.shape {
                RadarShape::Dot => {
                    // Filled disc: concentric rings at 1 px spacing fill the area.
                    let mut rr = 1.0_f32;
                    while rr <= r {
                        gizmos.circle_2d(draw_pos, rr, appearance.color);
                        rr += 1.0;
                    }
                }
                RadarShape::Ring => {
                    gizmos.circle_2d(draw_pos, r, appearance.color);
                }
                RadarShape::Triangle => {
                    let nose  = draw_pos + Vec2::new(0.0, r);
                    let left  = draw_pos + Vec2::new(-r * 0.866, -r * 0.5);
                    let right = draw_pos + Vec2::new( r * 0.866, -r * 0.5);
                    gizmos.line_2d(nose, left, appearance.color);
                    gizmos.line_2d(left, right, appearance.color);
                    gizmos.line_2d(right, nose, appearance.color);
                }
                RadarShape::Icon(_) => {
                    // Images cannot be drawn through gizmos; render a diamond
                    // outline as a distinct fallback marker.
                    let top   = draw_pos + Vec2::new(0.0,  r);
                    let right = draw_pos + Vec2::new( r, 0.0);
                    let bot   = draw_pos + Vec2::new(0.0, -r);
                    let left  = draw_pos + Vec2::new(-r, 0.0);
                    gizmos.line_2d(top, right, appearance.color);
                    gizmos.line_2d(right, bot, appearance.color);
                    gizmos.line_2d(bot, left, appearance.color);
                    gizmos.line_2d(left, top, appearance.color);
                }
            }
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Sub-plugin for the radar widget.  Registered automatically by `GuiPlugin`.
pub struct GuiRadarPlugin;

impl Plugin for GuiRadarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, draw_generic_radars);
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn all_layers_filter() -> RadarFilter {
        let mut s = HashSet::new();
        s.insert(RadarLayer::Ship);
        s.insert(RadarLayer::Asteroid);
        s.insert(RadarLayer::Station);
        s.insert(RadarLayer::Missile);
        RadarFilter(s)
    }

    fn ship_only_filter() -> RadarFilter {
        let mut s = HashSet::new();
        s.insert(RadarLayer::Ship);
        RadarFilter(s)
    }

    fn empty_filter() -> RadarFilter {
        RadarFilter(HashSet::new())
    }

    // ── is_on_radar ──────────────────────────────────────────────────────────

    #[test]
    fn included_layer_passes_filter() {
        let filter = ship_only_filter();
        assert!(is_on_radar(&filter, RadarLayer::Ship));
    }

    #[test]
    fn excluded_layer_fails_filter() {
        let filter = ship_only_filter();
        assert!(!is_on_radar(&filter, RadarLayer::Asteroid));
    }

    #[test]
    fn empty_filter_excludes_all_layers() {
        let filter = empty_filter();
        assert!(!is_on_radar(&filter, RadarLayer::Ship));
        assert!(!is_on_radar(&filter, RadarLayer::Asteroid));
        assert!(!is_on_radar(&filter, RadarLayer::Missile));
    }

    #[test]
    fn all_layers_filter_includes_all() {
        let filter = all_layers_filter();
        assert!(is_on_radar(&filter, RadarLayer::Ship));
        assert!(is_on_radar(&filter, RadarLayer::Asteroid));
        assert!(is_on_radar(&filter, RadarLayer::Station));
        assert!(is_on_radar(&filter, RadarLayer::Missile));
    }

    // ── project_radar_entity ─────────────────────────────────────────────────

    #[test]
    fn center_entity_projects_to_zero() {
        let result = project_radar_entity(0.0, 0.0, 0.0, 0.0, 0.0, 100.0, &OrientationMode::ShipRelative);
        let (x, y) = result.unwrap();
        assert!((x).abs() < 1e-5);
        assert!((y).abs() < 1e-5);
    }

    #[test]
    fn entity_beyond_range_returns_none() {
        let result = project_radar_entity(200.0, 0.0, 0.0, 0.0, 0.0, 100.0, &OrientationMode::ShipRelative);
        assert!(result.is_none());
    }

    #[test]
    fn zero_range_returns_none_safely() {
        let result = project_radar_entity(10.0, 0.0, 0.0, 0.0, 0.0, 0.0, &OrientationMode::ShipRelative);
        assert!(result.is_none());
    }

    #[test]
    fn ship_relative_yaw_zero_ahead_entity_gives_positive_y() {
        // At yaw=0: forward = (sin(0), -cos(0)) = (0,-1) in XZ.
        // Entity at dz=-100 (ahead) → radar_y = dx*sin(0) - dz*cos(0) = 0 - (-100)*1 = +1.0
        let result = project_radar_entity(0.0, -100.0, 0.0, 0.0, 0.0, 100.0, &OrientationMode::ShipRelative);
        let (_, y) = result.unwrap();
        assert!((y - 1.0).abs() < 1e-5, "expected radar_y=1.0, got {y}");
    }

    #[test]
    fn world_fixed_ignores_yaw() {
        let yaw = std::f32::consts::FRAC_PI_2; // 90 degrees
        let ship_relative = project_radar_entity(50.0, 0.0, 0.0, 0.0, yaw, 100.0, &OrientationMode::ShipRelative);
        let world_fixed   = project_radar_entity(50.0, 0.0, 0.0, 0.0, yaw, 100.0, &OrientationMode::WorldFixed);
        // WorldFixed should give the same as ShipRelative at yaw=0
        let world_fixed_0 = project_radar_entity(50.0, 0.0, 0.0, 0.0, 0.0, 100.0, &OrientationMode::ShipRelative);
        assert_ne!(ship_relative, world_fixed, "ship_relative should differ from world_fixed at non-zero yaw");
        assert_eq!(world_fixed, world_fixed_0, "world_fixed should equal ship_relative@yaw=0");
    }

    #[test]
    fn entity_at_range_boundary_is_included() {
        // Exactly at range (dx=100, dz=0, range=100): dx²+dz² = 10000 = range²  → included.
        let result = project_radar_entity(100.0, 0.0, 0.0, 0.0, 0.0, 100.0, &OrientationMode::WorldFixed);
        assert!(result.is_some());
    }
}
