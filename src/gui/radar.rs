//! `GenericRadar` widget — UI-node-based radar with configurable range,
//! orientation, and per-layer entity filtering.
//!
//! Game-world entities opt in by carrying `OnRadar(RadarLayer)` and
//! `RadarAppearance`.  The reference entity (player ship) carries `RadarCenter`.
//! Blips are reconciled each frame into child UI nodes of the radar widget so
//! they render *over* the radar background (gizmos draw under UI and would be
//! occluded by the opaque radar dial image).

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

// ── Layer / filter ────────────────────────────────────────────────────────────

/// Entity classification for radar layer filtering.
#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
pub enum RadarLayer {
    Ship,
    Asteroid,
    Station,
    Missile,
    Planet,
    Star,
}

/// Opts a game-world entity into the radar.  The widget draws only entities
/// whose layer passes the widget's `RadarFilter`.
#[derive(Component, Clone, Debug)]
pub struct OnRadar(pub RadarLayer);

/// Component on the radar widget node: which `RadarLayer` values to draw.
#[derive(Component, Clone, Debug)]
pub struct RadarFilter(pub HashSet<RadarLayer>);

// ── Appearance ────────────────────────────────────────────────────────────────

/// Which icon to render for a radar blip. Each variant maps 1:1 to a
/// PNG in `assets/radar_icons/`. `Missile` uses `Icon-Torpedo.png`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RadarIcon {
    Ship,
    Asteroid,
    Station,
    Planet,
    Star,
    Torpedo,
}

/// How an `OnRadar` entity is rendered on the radar.
///
/// `world_size` is a world-space diameter that the radar widget
/// projects into pixels via its current `range` and pixel radius.
/// `color` tints the icon (use `Color::WHITE` for no tint).
#[derive(Component, Clone, Debug)]
pub struct RadarAppearance {
    pub icon: RadarIcon,
    pub world_size: f32,
    pub color: Color,
}

/// Maps `RadarIcon` to the loaded `Handle<Image>` for the corresponding
/// PNG. Populated once at startup by `client::phone_border::framing`
/// when `PhoneAssets` is ready. Empty until populated; blips with a
/// missing icon render as a coloured square (defensive fallback).
#[derive(Resource, Default)]
pub struct RadarIconLookup(pub HashMap<RadarIcon, Handle<Image>>);

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

// ── Blip node tag ────────────────────────────────────────────────────────────

/// Tags a UI node spawned by `sync_radar_blip_nodes` to represent a
/// blip. Holds the source `Entity` so the diff can reconcile.
#[derive(Component)]
struct RadarBlipNode {
    source: Entity,
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Minimum blip diameter in pixels so very small or distant entities
/// remain visible on the HUD even when truthful projection would
/// produce sub-pixel sizes.
pub const MIN_BLIP_PX: f32 = 8.0;

/// Project a world-space size into a pixel diameter for a radar blip
/// icon, given the radar's pixel radius and world-space range. Clamps
/// to `[MIN_BLIP_PX, radar_pixel_diameter]`.
pub fn world_size_to_px(world_size: f32, range: f32, radar_radius_px: f32) -> f32 {
    if range <= 0.0 || radar_radius_px <= 0.0 {
        return MIN_BLIP_PX;
    }
    let raw = world_size / range * radar_radius_px * 2.0;
    raw.clamp(MIN_BLIP_PX, radar_radius_px * 2.0)
}

/// Given a projection in normalised radar coords (`nx, ny` in [-1, 1]
/// with +y up per gizmo convention), the radar's pixel radius, and a
/// blip's half-size in pixels, returns the top-left corner of a UI
/// `Node` that centres the blip on the projection point inside the
/// radar widget. Y is flipped to UI's y-down convention.
pub fn blip_local_offset(nx: f32, ny: f32, radar_radius_px: f32, half_size_px: f32) -> (f32, f32) {
    let left = radar_radius_px + nx * radar_radius_px - half_size_px;
    let top = radar_radius_px - ny * radar_radius_px - half_size_px;
    (left, top)
}

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
    entity_radius: f32,
    orientation: &OrientationMode,
) -> Option<(f32, f32)> {
    if range <= 0.0 {
        return None;
    }
    let dx = entity_x - center_x;
    let dz = entity_z - center_z;
    let effective_range = range + entity_radius.max(0.0);
    if dx * dx + dz * dz > effective_range * effective_range {
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
    /// Spawn a `GenericRadar` widget that fills its parent's slot.
    ///
    /// - `range` — world-unit radius drawn by the radar.
    /// - `orientation` — `ShipRelative` or `WorldFixed`.
    /// - `filter` — which `RadarLayer` values to draw.
    /// - `bg_image` (back layer) / `overlay_image` (front layer) — optional images.
    ///
    /// Returns the UI node entity.
    pub fn spawn(
        commands: &mut Commands,
        range: f32,
        orientation: OrientationMode,
        filter: RadarFilter,
        bg_image: Option<Handle<Image>>,
        overlay_image: Option<Handle<Image>>,
    ) -> Entity {
        let mut node = commands.spawn((
            GenericRadarWidget {
                range,
                orientation,
                filter,
            },
            Node {
                width: Val::Percent(100.0),
                aspect_ratio: Some(1.0),
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
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        top: Val::Px(0.0),
                        left: Val::Px(0.0),
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

// ── Sync system ───────────────────────────────────────────────────────────────

/// Each frame: for every visible `GenericRadarWidget`, project all
/// `OnRadar` + `RadarAppearance` entities passing the filter into child UI
/// nodes (icons via `RadarIconLookup`). New blips spawn, existing ones
/// update in place, missing ones despawn.
fn sync_radar_blip_nodes(
    mut commands: Commands,
    radars: Query<(
        Entity,
        &GenericRadarWidget,
        &ComputedNode,
        &bevy::camera::visibility::InheritedVisibility,
        Option<&Children>,
    )>,
    blips: Query<(Entity, &OnRadar, &RadarAppearance, &GlobalTransform)>,
    centers: Query<&RadarCenter>,
    mut existing_nodes: Query<(&mut Node, &mut ImageNode, &RadarBlipNode)>,
    icons: Res<RadarIconLookup>,
) {
    let Some(center) = centers.iter().next() else { return };

    for (radar_entity, widget, computed, vis, children) in radars.iter() {
        if !vis.get() {
            continue;
        }
        let size = computed.size();
        let radar_radius_px = size.x.min(size.y) * 0.5;
        if radar_radius_px <= 0.0 {
            continue;
        }

        // Build intended blip set: source -> (left, top, size_px, color, icon_handle)
        let mut intended: HashMap<Entity, (f32, f32, f32, Color, Option<Handle<Image>>)> =
            HashMap::new();
        for (src, on_radar, appearance, blip_gtf) in blips.iter() {
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
                appearance.world_size * 0.5,
                &widget.orientation,
            ) else {
                continue;
            };
            // Circular clip: skip blips whose centre lies outside the radar circle.
            if nx * nx + ny * ny > 1.0 {
                continue;
            }
            let size_px = world_size_to_px(appearance.world_size, widget.range, radar_radius_px);
            let half = size_px * 0.5;
            let (left, top) = blip_local_offset(nx, ny, radar_radius_px, half);
            let icon_handle = icons.0.get(&appearance.icon).cloned();
            intended.insert(src, (left, top, size_px, appearance.color, icon_handle));
        }

        // Reconcile existing children.
        if let Some(children) = children {
            for child in children.iter() {
                if let Ok((mut node, mut image, tag)) = existing_nodes.get_mut(child) {
                    if let Some((left, top, size_px, color, icon_handle)) =
                        intended.remove(&tag.source)
                    {
                        node.left = Val::Px(left);
                        node.top = Val::Px(top);
                        node.width = Val::Px(size_px);
                        node.height = Val::Px(size_px);
                        if let Some(h) = icon_handle {
                            image.image = h;
                        }
                        image.color = color;
                    } else {
                        // Source no longer present — despawn.
                        commands.entity(child).despawn();
                    }
                }
            }
        }

        // Spawn new nodes for any remaining intended blips.
        if !intended.is_empty() {
            commands.entity(radar_entity).with_children(|parent| {
                for (source, (left, top, size_px, color, icon_handle)) in intended.drain() {
                    let node = Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(left),
                        top: Val::Px(top),
                        width: Val::Px(size_px),
                        height: Val::Px(size_px),
                        ..default()
                    };
                    if let Some(h) = icon_handle {
                        parent.spawn((
                            node,
                            ImageNode::new(h).with_color(color),
                            ZIndex(0),
                            RadarBlipNode { source },
                        ));
                    } else {
                        // Defensive fallback: coloured square when icon missing.
                        parent.spawn((
                            node,
                            ImageNode::solid_color(color),
                            ZIndex(0),
                            RadarBlipNode { source },
                        ));
                    }
                }
            });
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Sub-plugin for the radar widget.  Registered automatically by `GuiPlugin`.
pub struct GuiRadarPlugin;

impl Plugin for GuiRadarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RadarIconLookup>()
            .add_systems(Update, sync_radar_blip_nodes);
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
        s.insert(RadarLayer::Planet);
        s.insert(RadarLayer::Star);
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
        let result = project_radar_entity(
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            100.0,
            0.0,
            &OrientationMode::ShipRelative,
        );
        let (x, y) = result.unwrap();
        assert!((x).abs() < 1e-5);
        assert!((y).abs() < 1e-5);
    }

    #[test]
    fn entity_beyond_range_returns_none() {
        let result = project_radar_entity(
            200.0,
            0.0,
            0.0,
            0.0,
            0.0,
            100.0,
            0.0,
            &OrientationMode::ShipRelative,
        );
        assert!(result.is_none());
    }

    #[test]
    fn zero_range_returns_none_safely() {
        let result = project_radar_entity(
            10.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            &OrientationMode::ShipRelative,
        );
        assert!(result.is_none());
    }

    #[test]
    fn ship_relative_yaw_zero_ahead_entity_gives_positive_y() {
        // At yaw=0: forward = (sin(0), -cos(0)) = (0,-1) in XZ.
        // Entity at dz=-100 (ahead) → radar_y = dx*sin(0) - dz*cos(0) = 0 - (-100)*1 = +1.0
        let result = project_radar_entity(
            0.0,
            -100.0,
            0.0,
            0.0,
            0.0,
            100.0,
            0.0,
            &OrientationMode::ShipRelative,
        );
        let (_, y) = result.unwrap();
        assert!((y - 1.0).abs() < 1e-5, "expected radar_y=1.0, got {y}");
    }

    #[test]
    fn world_fixed_ignores_yaw() {
        let yaw = std::f32::consts::FRAC_PI_2; // 90 degrees
        let ship_relative = project_radar_entity(
            50.0,
            0.0,
            0.0,
            0.0,
            yaw,
            100.0,
            0.0,
            &OrientationMode::ShipRelative,
        );
        let world_fixed = project_radar_entity(
            50.0,
            0.0,
            0.0,
            0.0,
            yaw,
            100.0,
            0.0,
            &OrientationMode::WorldFixed,
        );
        // WorldFixed should give the same as ShipRelative at yaw=0
        let world_fixed_0 = project_radar_entity(
            50.0,
            0.0,
            0.0,
            0.0,
            0.0,
            100.0,
            0.0,
            &OrientationMode::ShipRelative,
        );
        assert_ne!(
            ship_relative, world_fixed,
            "ship_relative should differ from world_fixed at non-zero yaw"
        );
        assert_eq!(
            world_fixed, world_fixed_0,
            "world_fixed should equal ship_relative@yaw=0"
        );
    }

    #[test]
    fn entity_at_range_boundary_is_included() {
        // Exactly at range (dx=100, dz=0, range=100): dx²+dz² = 10000 = range²  → included.
        let result = project_radar_entity(
            100.0,
            0.0,
            0.0,
            0.0,
            0.0,
            100.0,
            0.0,
            &OrientationMode::WorldFixed,
        );
        assert!(result.is_some());
    }

    #[test]
    fn entity_radius_extends_detection_range() {
        // Entity center at 120 units, range=100, entity_radius=25 → 120 <= 125 → included.
        let result = project_radar_entity(
            120.0,
            0.0,
            0.0,
            0.0,
            0.0,
            100.0,
            25.0,
            &OrientationMode::WorldFixed,
        );
        assert!(result.is_some());
    }

    #[test]
    fn entity_radius_does_not_include_fully_out_of_range() {
        // Entity center at 200 units, range=100, entity_radius=25 → 200 > 125 → excluded.
        let result = project_radar_entity(
            200.0,
            0.0,
            0.0,
            0.0,
            0.0,
            100.0,
            25.0,
            &OrientationMode::WorldFixed,
        );
        assert!(result.is_none());
    }

    // ── world_size_to_px ─────────────────────────────────────────────────────

    #[test]
    fn world_size_to_px_returns_min_when_range_zero() {
        assert_eq!(world_size_to_px(50.0, 0.0, 140.0), MIN_BLIP_PX);
    }

    #[test]
    fn world_size_to_px_returns_min_when_radius_zero() {
        assert_eq!(world_size_to_px(50.0, 500.0, 0.0), MIN_BLIP_PX);
    }

    #[test]
    fn world_size_to_px_below_min_clamps_up() {
        assert_eq!(world_size_to_px(0.1, 500.0, 140.0), MIN_BLIP_PX);
    }

    #[test]
    fn world_size_to_px_linear_interior() {
        // 50 / 500 * 140 * 2 = 28.0
        assert!((world_size_to_px(50.0, 500.0, 140.0) - 28.0).abs() < 1e-5);
    }

    #[test]
    fn world_size_to_px_above_diameter_clamps() {
        // raw would be 1000/500*140*2 = 560 → clamps to diameter 280.
        assert!((world_size_to_px(1000.0, 500.0, 140.0) - 280.0).abs() < 1e-5);
    }

    // ── blip_local_offset ────────────────────────────────────────────────────

    #[test]
    fn blip_local_offset_centre_projection() {
        let (left, top) = blip_local_offset(0.0, 0.0, 140.0, 8.0);
        assert!((left - 132.0).abs() < 1e-5);
        assert!((top - 132.0).abs() < 1e-5);
    }

    #[test]
    fn blip_local_offset_right_edge() {
        let (left, top) = blip_local_offset(1.0, 0.0, 140.0, 8.0);
        assert!((left - 272.0).abs() < 1e-5);
        assert!((top - 132.0).abs() < 1e-5);
    }

    #[test]
    fn blip_local_offset_top_edge() {
        // ny = 1 → top = 140 - 1*140 - 8 = -8 (Y flipped to UI's y-down).
        let (left, top) = blip_local_offset(0.0, 1.0, 140.0, 8.0);
        assert!((left - 132.0).abs() < 1e-5);
        assert!((top - (-8.0)).abs() < 1e-5);
    }

    #[test]
    fn blip_local_offset_bottom_edge() {
        let (left, top) = blip_local_offset(0.0, -1.0, 140.0, 8.0);
        assert!((left - 132.0).abs() < 1e-5);
        assert!((top - 272.0).abs() < 1e-5);
    }
}
