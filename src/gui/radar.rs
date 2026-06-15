//! `GenericRadar` widget — UI-node-based radar with configurable range,
//! orientation, and tag-based entity filtering.
//!
//! Game-world entities opt in by carrying `OnRadar` (with their tag list) and
//! `RadarAppearance`.  The reference entity (player ship) carries `RadarCenter`.
//! Blips are reconciled each frame into child UI nodes of the radar widget so
//! they render *over* the radar background (gizmos draw under UI and would be
//! occluded by the opaque radar dial image).

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};
use std::collections::HashMap;

use crate::messages::EntitySnapshot;

// ── Filter ────────────────────────────────────────────────────────────────────

/// Opts a game-world entity into the radar. Carries the entity's tag list so
/// `sync_radar_blip_nodes` can check it against the widget's `RadarFilter`.
#[derive(Component, Clone, Debug)]
pub struct OnRadar(pub Vec<String>);

/// Component on the radar widget node: which tag strings to draw.
/// An entity passes the filter if it carries **at least one** of these tags
/// (OR logic). An empty filter shows nothing.
#[derive(Component, Clone, Debug)]
pub struct RadarFilter(pub std::collections::HashSet<String>);

impl RadarFilter {
    /// Build a `RadarFilter` from a slice of tag strings (e.g. one of the
    /// `ShipClientConfig.*_shows` fields, or a TOML `shows = [...]` list).
    pub fn from_shows<S: AsRef<str>>(shows: &[S]) -> Self {
        RadarFilter(shows.iter().map(|s| s.as_ref().to_string()).collect())
    }
}

// ── Appearance ────────────────────────────────────────────────────────────────

/// Which icon to render for a radar blip. Each variant maps 1:1 to a
/// PNG in `assets/radar_icons/`. `Missile` uses `Icon-Torpedo.png`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RadarIcon {
    Ship,
    PlayerShip,
    Asteroid,
    Station,
    Planet,
    Star,
    Torpedo,
    Battleship,
    Cruiser,
    Destroyer,
}

/// How a region entity's shape is rendered on the 2D radar projection.
/// Each variant maps to a different UI node layout.
#[derive(Clone, Debug, PartialEq)]
pub enum RegionRadarShape {
    /// Filled circle with `radius` world units.
    Sphere { radius: f32 },
    /// Filled rectangle extruded from `half_extents` (XZ plane).
    Box {
        half_extents_x: f32,
        half_extents_z: f32,
        yaw: f32,
    },
    /// Annular ring with inner and outer radius (world units).
    Torus {
        inner_radius: f32,
        outer_radius: f32,
    },
}

/// How an `OnRadar` entity is rendered on the radar.
///
/// `world_size` is a world-space diameter that the radar widget
/// projects into pixels via its current `range` and pixel radius.
/// `color` tints the icon (use `Color::WHITE` for no tint).
///
/// When `region_colour` is `Some`, the entity is a region rendered
/// as its shape (torus, sphere, box) filled with that colour rather
/// than as a point icon.
#[derive(Component, Clone, Debug)]
pub struct RadarAppearance {
    pub icon: RadarIcon,
    pub world_size: f32,
    pub color: Color,
    /// Region fill colour. `Some` → render as region shape;
    /// `None` → render as a point icon.
    pub region_colour: Option<Color>,
    /// The region shape geometry. Meaningful only when
    /// `region_colour` is `Some`. When `Some` and `region_colour`
    /// is `Some`, the shape is rendered on the radar.
    pub region_shape: Option<RegionRadarShape>,
    /// When `true` the entity is referenced by an active mission objective
    /// and should render a gold ring indicator on the radar.
    pub objective_target: bool,
}

impl RadarAppearance {
    /// The player-ship reference blip used at the centre of every radar.
    /// Soft cyan-white, ship icon, 6.0-unit world size.
    pub fn player_ship() -> Self {
        Self {
            icon: RadarIcon::PlayerShip,
            world_size: 6.0,
            color: Color::srgb(0.95, 0.95, 1.0),
            region_colour: None,
            region_shape: None,
            objective_target: false,
        }
    }

    /// A point-entity blip (ship, station, planet, star, missile, asteroid).
    pub fn point(icon: RadarIcon, world_size: f32, color: Color) -> Self {
        Self {
            icon,
            world_size,
            color,
            region_colour: None,
            region_shape: None,
            objective_target: false,
        }
    }

    /// A region-entity blip rendered as a filled/outlined shape.
    pub fn region(
        icon: RadarIcon,
        world_size: f32,
        region_colour: Color,
        shape: Option<RegionRadarShape>,
    ) -> Self {
        Self {
            icon,
            world_size,
            color: region_colour,
            region_colour: Some(region_colour),
            region_shape: shape,
            objective_target: false,
        }
    }
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

// ── Clip mode ─────────────────────────────────────────────────────────────────

/// Controls how the radar widget clips rendered blips at its boundary.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum RadarClipMode {
    /// No clipping — blips may overflow the node's bounds.
    #[default]
    None,
    /// Blips are visually clipped to the inscribed circle of the radar widget
    /// via per-pixel shader discard in `RadarBlipMaterial`. No extra image needed.
    Circle,
    /// Blips are clipped to the rectangular bounds of the radar widget.
    /// Requires `overflow: Overflow::Clip` on the spawned node (set it after
    /// `GenericRadar::spawn` via `commands.entity(radar).insert(Node { … })`).
    Square,
}

// ── Widget component ──────────────────────────────────────────────────────────

/// Component on the radar UI node.
#[derive(Component)]
pub struct GenericRadarWidget {
    pub range: f32,
    pub orientation: OrientationMode,
    pub filter: RadarFilter,
    pub clip_mode: RadarClipMode,
    /// Fraction of the (root or overlay) inscribed-circle radius that corresponds
    /// to the visual radar face.  `1.0` means the radar circle fills the full
    /// face area; values < 1.0 scale blip positions and the per-pixel clip
    /// boundary inward to match a background image whose circular face is
    /// smaller than the image.
    pub face_fraction: f32,
}

/// Captures the overlay-image child entity so `sync_radar_blip_nodes` can read
/// that child's `ComputedNode` size for the radar-radius calculation instead of
/// using the root widget's size.
#[derive(Component)]
pub struct RadarOverlayEntity(pub Entity);

// ── Per-widget behaviour overrides ────────────────────────────────────────────

/// Identifies which logical radar a `GenericRadarWidget` represents.
///
/// One component per widget; unifies what used to be four separate empty
/// marker components (`HelmRadarWidget`, `ScienceRadarWidget`,
/// `NavRadarWidget`, `TacticalRadarWidget`) plus four viewscreen modes.
///
/// The unified `sync_radar_widgets_from_lobby` system reads the variant to
/// pick which `ShipClientConfig.*_shows` (and optionally range) field to copy
/// into the widget's `RadarFilter`. Per-console bridge systems use it as a
/// query filter to find their widget's `RadarBlipMap`.
#[derive(Component, Copy, Clone, Eq, PartialEq, Debug)]
pub enum ConsoleRadar {
    /// Phone helm console — ship-relative, ranged from `helm_radar_range`.
    Helm,
    /// Phone sensors console — world-fixed, long-range.
    Sensors,
    /// Phone navigation console — world-centred, auto-scaled.
    Navigation,
    /// Phone tactical (weapons) console — world-fixed, mid-range.
    Tactical,
    /// Server viewscreen helm radar mode.
    ViewscreenHelm,
    /// Server viewscreen science / sensors radar mode.
    ViewscreenScience,
    /// Server viewscreen system-chart mode.
    ViewscreenSystemChart,
    /// Server viewscreen navigation-chart mode.
    ViewscreenNav,
}

/// If present on a `GenericRadarWidget` entity, the radar is centred on the
/// world origin `(0, 0)` rather than the `RadarCenter` reference entity, and
/// orientation is always north-up (effective yaw = 0).
#[derive(Component)]
pub struct WorldCentredRadar;

/// If present on a `GenericRadarWidget` entity, the widget recomputes its
/// `range` every frame so that all visible blips fit within the display
/// plus a `margin` factor (e.g. `1.1` = 10 % extra space).
/// A `min_range` floor prevents the view from collapsing when no blips exist.
#[derive(Component)]
pub struct AutoScaleRadar {
    /// Multiplicative margin applied to the farthest blip distance.
    /// `1.1` means 10 % padding beyond the outermost entity.
    pub margin: f32,
    /// Minimum display range in world units even when blips are absent.
    pub min_range: f32,
}

/// Render text name labels for named navigational blips; opt-in per widget.
#[derive(Component)]
pub struct RadarBlipLabels;

/// User-controlled zoom/pan overlay for a radar widget. When `user_engaged`
/// is true, the widget ignores AutoScaleRadar's fitted range and WorldCentred
/// origin, instead centring on `(pan_x, pan_z)` (world units, relative to the
/// widget's natural centre) and dividing the fitted range by `zoom`.
#[derive(Component, Clone, Debug)]
pub struct RadarViewControl {
    pub pan_x: f32,
    pub pan_z: f32,
    pub zoom: f32,
    pub user_engaged: bool,
}

impl Default for RadarViewControl {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_z: 0.0,
            zoom: 1.0,
            user_engaged: false,
        }
    }
}

/// Smallest user zoom factor (most zoomed-out relative to the fitted range).
pub const RADAR_MIN_ZOOM: f32 = 0.25;
/// Largest user zoom factor (most zoomed-in relative to the fitted range).
pub const RADAR_MAX_ZOOM: f32 = 8.0;
/// Absolute floor on the world-unit range when zooming in, so the view never
/// collapses to a degenerate (sub-`project_radar_entity`-safe) range.
const RADAR_MIN_RANGE: f32 = 10.0;

/// Last-known per-frame geometry of a radar widget, written by
/// [`sync_radar_blip_nodes`] so that pointer observers (which cannot read the
/// system's locals) can convert pixel deltas into world-unit pan deltas.
#[derive(Component, Default)]
pub struct RadarLastGeom {
    /// Pixel radius that maps to one `range` of world units across the half-width.
    pub radar_radius_px: f32,
    /// Effective world-unit range in use after auto-scale / view-control.
    pub range: f32,
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

// ── Blip world pose ──────────────────────────────────────────────────────────

/// World-space position and heading carried by every radar-blip ECS entity
/// (`RadarCenter` and per-uuid blips both live alongside this).
///
/// Replaces `Transform` / `GlobalTransform` on blip entities. Blips are
/// parented to a `Node`-based radar widget so that `despawn` on the widget
/// cleans up its blip set, but `Node` no longer satisfies the
/// `GlobalTransform` propagation invariant in Bevy 0.18 (UI uses
/// `UiTransform` instead) — putting `GlobalTransform` on a child of a Node
/// triggers B0004. `BlipWorldPose` is a plain data carrier, never
/// propagated, so the parent-child invariant is not violated.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct BlipWorldPose {
    pub x: f32,
    pub z: f32,
    /// Heading in radians.
    pub yaw: f32,
}

/// Carries the optional display name of a radar blip so the per-frame
/// reconciler can render a text label next to it (on opt-in widgets).
#[derive(Component, Clone, Debug, Default)]
pub struct BlipLabel(pub Option<String>);

// ── Blip node tag ────────────────────────────────────────────────────────────

/// Tags a UI node spawned by `sync_radar_blip_nodes` to represent a
/// blip. Holds the source `Entity` so the diff can reconcile.
#[derive(Component)]
struct RadarBlipNode {
    source: Entity,
}

/// Tags a UI node spawned by `sync_radar_blip_nodes` to represent a
/// region shape. Holds the source `Entity` and shape data so the
/// diff can reconcile and render the correct geometry.
#[derive(Component)]
pub struct RadarRegionNode {
    source: Entity,
}

/// Tags a UI text node spawned by `sync_radar_blip_nodes` to render the
/// name label of a navigational blip. Holds the source `Entity` so the
/// diff can reconcile.
#[derive(Component)]
struct RadarLabelNode {
    source: Entity,
}

/// Triggered on a **radar widget entity** when the player clicks one of its
/// blips. The event target is the radar; the payload carries the source ECS
/// entity (`RadarBlipNode::source`) so observers can resolve identity (e.g.
/// `RadarEntityUuid`) without doing their own picking.
///
/// Targeting the radar (rather than the source entity) lets each console
/// attach its own per-radar `.observe(...)` handler — e.g. the tactical radar
/// triggers a phaser lock; a future sensors radar might trigger a science
/// designation — without consoles needing to gate on "is this MY radar?".
#[derive(EntityEvent, Clone, Debug)]
pub struct RadarBlipClicked {
    /// The radar widget entity that owns the tapped blip. Also the
    /// `EntityEvent::event_target`, so `commands.entity(radar).observe(...)`
    /// fires when this event triggers.
    #[event_target]
    pub radar: Entity,
    /// The source ECS blip entity (e.g. a sim-snapshot entity carrying
    /// `RadarEntityUuid`). Observers query components on this entity to
    /// recover the wire-level identity of what was tapped.
    pub source: Entity,
}

// ── Blip shader material ──────────────────────────────────────────────────────

/// Per-blip `UiMaterial` that tints the icon texture and clips pixels to the
/// radar's circular boundary when `clip_circle > 0.5`.
///
/// Binding layout matches `assets/shaders/radar_blip.wgsl`:
///   0 = icon texture, 1 = icon sampler, 2 = uniform struct.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct RadarBlipMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub icon: Handle<Image>,
    #[uniform(2)]
    pub color_r: f32,
    #[uniform(2)]
    pub color_g: f32,
    #[uniform(2)]
    pub color_b: f32,
    #[uniform(2)]
    pub color_a: f32,
    #[uniform(2)]
    pub radar_nx: f32,
    #[uniform(2)]
    pub radar_ny: f32,
    #[uniform(2)]
    pub size_frac: f32,
    /// Non-zero enables per-pixel circular clip in the fragment shader.
    #[uniform(2)]
    pub clip_circle: f32,
    /// Non-zero draws a red targeting ring around the blip (replaces tint-only
    /// approach that was invisible on coloured icons).
    #[uniform(2)]
    pub highlighted: f32,
    /// std140 padding to 48 bytes (3 × 16 = 3 vec4s).
    #[uniform(2)]
    pub _pad0: f32,
    #[uniform(2)]
    pub _pad1: f32,
    #[uniform(2)]
    pub _pad2: f32,
}

impl UiMaterial for RadarBlipMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/radar_blip.wgsl".into()
    }
}

/// 1×1 white pixel image used as the fallback icon for blips whose real
/// icon texture has not loaded yet, ensuring the tint colour still renders.
#[derive(Resource)]
pub struct RadarBlipFallbackIcon(pub Handle<Image>);

fn setup_radar_blip_fallback(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(RadarBlipFallbackIcon(images.add(Image::default())));
}

// ── Icon mapping ──────────────────────────────────────────────────────────────

/// Map a tag string to the `RadarIcon` used for its blip.
///
/// - `"ship"`, `"pirate"`, `"player"` → `Ship`
/// - `"asteroid"`, `"asteroid_field"` → `Asteroid`
/// - `"station"` → `Station`
/// - `"missile"`, `"torpedo"` → `Torpedo`
/// - `"planet"` → `Planet`
/// - `"star"` → `Star`
/// - `"battleship"` → `Battleship`
/// - `"cruiser"` → `Cruiser`
/// - `"destroyer"` → `Destroyer`
/// - anything else → `Ship` (defensive fallback)
pub fn icon_from_radar_icon_str(s: &str) -> RadarIcon {
    match s {
        "ship" | "pirate" => RadarIcon::Ship,
        "player" | "player_ship" => RadarIcon::PlayerShip,
        "asteroid" | "asteroid_field" => RadarIcon::Asteroid,
        "station" => RadarIcon::Station,
        "missile" | "torpedo" => RadarIcon::Torpedo,
        "planet" => RadarIcon::Planet,
        "star" => RadarIcon::Star,
        "battleship" => RadarIcon::Battleship,
        "cruiser" => RadarIcon::Cruiser,
        "destroyer" => RadarIcon::Destroyer,
        _ => RadarIcon::Ship,
    }
}

/// Build a `RegionRadarShape` from an `EntitySnapshot` based on its
/// `shape` string and geometric fields. Returns `None` when the
/// snapshot has no recognised shape (unknown or missing).
pub fn region_shape_from_snapshot(snapshot: &EntitySnapshot) -> Option<RegionRadarShape> {
    match snapshot.shape.as_deref() {
        Some("torus") => Some(RegionRadarShape::Torus {
            inner_radius: snapshot.inner_radius_or_zero(),
            outer_radius: snapshot.radius_or_zero(),
        }),
        Some("sphere") => Some(RegionRadarShape::Sphere {
            radius: snapshot.radius_or_zero(),
        }),
        Some("box") => {
            let he = snapshot.half_extents_or_zero();
            Some(RegionRadarShape::Box {
                half_extents_x: he[0],
                half_extents_z: he[2],
                yaw: 0.0,
            })
        }
        _ => None,
    }
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Returns `true` if any tag in `tags` is present in `filter`.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn is_on_radar(filter: &RadarFilter, tags: &[String]) -> bool {
    tags.iter().any(|t| filter.0.contains(t))
}

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
/// with +y up per gizmo convention), the radar widget's pixel center
/// (`center_x_px`, `center_y_px`), the pixel radius used for scaling
/// (`radar_radius_px`), and a blip's half-size in pixels, returns the
/// top-left corner of a UI `Node` that centres the blip on the projection
/// point inside the radar widget. Y is flipped to UI's y-down convention.
///
/// `center_x_px` and `center_y_px` are computed as `size.x * 0.5` and
/// `size.y * 0.5` from `ComputedNode::size()`. Keeping them separate from
/// `radar_radius_px` (which is `size.x.min(size.y) * 0.5`) corrects blip
/// positions when the widget is not perfectly square.
pub fn blip_local_offset(
    nx: f32,
    ny: f32,
    center_x_px: f32,
    center_y_px: f32,
    radar_radius_px: f32,
    half_size_px: f32,
) -> (f32, f32) {
    let left = center_x_px + nx * radar_radius_px - half_size_px;
    let top = center_y_px - ny * radar_radius_px - half_size_px;
    (left, top)
}

/// Apply a single multiplicative zoom step, clamped to the user zoom range.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn apply_zoom_step(zoom: f32, factor: f32) -> f32 {
    (zoom * factor).clamp(RADAR_MIN_ZOOM, RADAR_MAX_ZOOM)
}

/// Update a zoom factor from a pinch gesture given the previous and new
/// finger-to-finger distances. A non-positive `prev_dist` (the first frame of
/// a pinch) is a no-op so the zoom does not jump.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn pinch_zoom(zoom: f32, prev_dist: f32, new_dist: f32) -> f32 {
    const EPS: f32 = 1e-3;
    if prev_dist > EPS {
        apply_zoom_step(zoom, new_dist / prev_dist)
    } else {
        zoom
    }
}

/// Convert a pixel delta into a world-unit delta for panning, given the
/// current `range` and the radar's pixel radius. `radar_radius_px` maps to one
/// `range` of world units across the half-width (the inverse of
/// [`world_size_to_px`]). A non-positive radius yields `0.0`.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn px_to_world_delta(px_delta: f32, range: f32, radar_radius_px: f32) -> f32 {
    if radar_radius_px > 0.0 {
        px_delta * range / radar_radius_px
    } else {
        0.0
    }
}

/// Project a world-space entity position onto the radar's normalised coordinate
/// space `[-1.0, 1.0]`, or `None` if the entity is beyond `range`.
///
/// Axis conventions (shared with `gui/radar-math.js` on the JS client):
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
    // Ship-aligned projection: forward = (sin yaw, -cos yaw), right = (cos yaw, sin yaw).
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
    /// - `clip_mode` — `Circle`, `Square`, or `None`. Stored on the widget component.
    ///   `Circle` clips blips per-pixel in the fragment shader; no extra image needed.
    ///   `Square` clips via `overflow: Overflow::clip()` on the spawned node.
    /// - `overlay_fraction` — fraction of the widget at which the overlay image is
    ///   rendered (`1.0` = full size; `0.875` centres a 560 px image inside a 640 px
    ///   widget). Pass `1.0` when no overlay image is used.
    /// - `face_fraction` — fraction of the widget's inscribed-circle radius that
    ///   the visible radar face occupies.  Blip positions and the circular clip mask
    ///   are scaled by this value.  Pass `1.0` for image-free radars.
    ///
    /// Returns the UI node entity.
    pub fn spawn(
        commands: &mut Commands,
        range: f32,
        orientation: OrientationMode,
        filter: RadarFilter,
        bg_image: Option<Handle<Image>>,
        overlay_image: Option<Handle<Image>>,
        clip_mode: RadarClipMode,
        overlay_fraction: f32,
        face_fraction: f32,
    ) -> Entity {
        let mut node = commands.spawn((
            GenericRadarWidget {
                range,
                orientation,
                filter,
                clip_mode,
                face_fraction,
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
            let margin_pct = (1.0 - overlay_fraction) * 50.0;
            let child = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(overlay_fraction * 100.0),
                        height: Val::Percent(overlay_fraction * 100.0),
                        top: Val::Percent(margin_pct),
                        left: Val::Percent(margin_pct),
                        ..default()
                    },
                    ImageNode::new(overlay),
                    ZIndex(1),
                ))
                .id();
            commands.entity(entity).add_child(child);
            commands.entity(entity).insert(RadarOverlayEntity(child));
        }

        entity
    }
}

// ── Bridge: entity snapshots → radar ECS entities ────────────────────────────

/// Persistent map of `EntitySnapshot.uuid` → blip ECS entities for a single
/// radar widget. Add one as a sibling component on the `GenericRadarWidget`
/// entity so each radar keeps its own blip set.
///
/// `center` is the player-ship reference entity. `blips` is everything else.
#[derive(Component, Default)]
pub struct RadarBlipMap {
    pub center: Option<Entity>,
    pub blips: HashMap<String, Entity>,
}

/// The world-space position + heading of the player ship, supplied to
/// [`bridge_sim_to_radar`] for the centre blip and `RadarCenter` reference.
///
/// Decouples the bridge from `ShipView` (client) vs `ShipState` (server) —
/// callers compute their own `RadarCenterPose`.
#[derive(Clone, Copy, Debug)]
pub struct RadarCenterPose {
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
}

/// Reconcile a list of `EntitySnapshot`s into ECS radar blip entities under
/// `map`, plus a single `RadarCenter` reference entity for the player ship.
///
/// All four phone-console bridges and the server viewscreen bridge share this
/// implementation. Per-console behaviour is captured by the inputs:
/// - `pose` — where the player ship is (computed by the caller from
///   `ShipView` or `ShipState`).
/// - `entities` — `&[EntitySnapshot]` from `ClientSimState.world.entities`
///   or `WorldResource.0.entities`.
///
/// All blip ECS entities (including the centre) are spawned as children of
/// `widget` so they are despawned automatically when the widget is removed
/// (e.g. on phone-orientation respawn).
///
/// Every blip is tagged with [`RadarEntityUuid`] so tap-to-target and
/// `RadarTargetHighlight` work on any radar.
/// Navigational tags whose named entities receive a text label on opt-in
/// radars (currently only the Navigation console).
const NAV_LABEL_TAGS: [&str; 5] = ["star", "planet", "station", "asteroid_field", "region"];

/// Predicate: should this blip receive a text name label? True when the
/// display name is present and non-empty AND at least one tag is in the
/// navigational label set. Pure (Bevy-free) so it is unit-testable.
fn should_label(name: &Option<String>, tags: &[String]) -> bool {
    let has_name = name
        .as_deref()
        .map(|n| !n.trim().is_empty())
        .unwrap_or(false);
    has_name && tags.iter().any(|t| NAV_LABEL_TAGS.contains(&t.as_str()))
}

pub fn bridge_sim_to_radar(
    commands: &mut Commands,
    widget: Entity,
    map: &mut RadarBlipMap,
    pose: RadarCenterPose,
    entities: &[EntitySnapshot],
) {
    // ── Player-ship RadarCenter blip ──────────────────────────────────────
    let pose_comp = BlipWorldPose {
        x: pose.x,
        z: pose.z,
        yaw: pose.yaw,
    };
    let centre_bundle = (
        RadarCenter {
            world_x: pose.x,
            world_z: pose.z,
            yaw: pose.yaw,
        },
        OnRadar(vec!["player".to_string()]),
        RadarAppearance::player_ship(),
        pose_comp,
    );
    match map.center {
        Some(e) => {
            commands.entity(e).insert(centre_bundle);
        }
        None => {
            let e = commands.spawn(centre_bundle).id();
            commands.entity(widget).add_child(e);
            map.center = Some(e);
        }
    }

    // ── World entity blips ────────────────────────────────────────────────
    let mut seen = std::collections::HashSet::<String>::new();
    for snapshot in entities {
        let uuid = &snapshot.uuid;
        if !seen.insert(uuid.clone()) {
            continue;
        }
        if snapshot.tags.is_empty() {
            continue;
        }
        // The player ship is already rendered as the RadarCenter blip with the
        // correct PlayerShip icon. Skip it from the world entity list to avoid
        // a duplicate red "ship" blip appearing at the same position.
        if snapshot.tags.iter().any(|t| t == "player") {
            continue;
        }

        let entity_yaw = snapshot.yaw.unwrap_or(0.0);
        let tag_colour = snapshot.colour.map(|c| Color::srgb(c[0], c[1], c[2]));
        let icon_str = snapshot.radar_icon.as_deref().unwrap_or("ship");
        let icon = icon_from_radar_icon_str(icon_str);
        let is_region = snapshot.tags.iter().any(|t| t == "region");
        let is_field = snapshot.tags.iter().any(|t| t == "asteroid_field");

        let world_size = snapshot
            .radar_world_size
            .or_else(|| Some(snapshot.radius_or_zero()))
            .filter(|s| *s > 0.0)
            .unwrap_or(4.0);

        let mut appearance = if is_region || is_field {
            let default_col = if is_field {
                Color::srgb(0.52, 0.32, 0.18)
            } else {
                Color::srgb(0.8, 0.4, 0.8)
            };
            let region_colour = tag_colour.unwrap_or(default_col);
            let region_shape = region_shape_from_snapshot(snapshot);
            RadarAppearance::region(icon, world_size, region_colour, region_shape)
        } else {
            let color = tag_colour.unwrap_or(Color::srgb(0.95, 0.95, 1.0));
            RadarAppearance::point(icon, world_size, color)
        };
        appearance.objective_target = snapshot.objective_target;

        let bundle = (
            OnRadar(snapshot.tags.clone()),
            appearance,
            BlipWorldPose {
                x: snapshot.x(),
                z: snapshot.z(),
                yaw: entity_yaw,
            },
            RadarEntityUuid(uuid.clone()),
            BlipLabel(snapshot.name.clone()),
        );

        if let Some(&existing) = map.blips.get(uuid) {
            commands.entity(existing).insert(bundle);
        } else {
            let blip = commands.spawn(bundle).id();
            commands.entity(widget).add_child(blip);
            map.blips.insert(uuid.clone(), blip);
        }
    }

    // Despawn blips for entities that have left the world.
    map.blips.retain(|uuid, &mut entity| {
        if seen.contains(uuid) {
            true
        } else {
            commands.entity(entity).despawn();
            false
        }
    });
}

// ── Sync system ───────────────────────────────────────────────────────────────

/// Layout data and shader uniforms for a single radar blip, collected
/// during one reconciliation frame.
struct BlipIntent {
    left: f32,
    top: f32,
    size_px: f32,
    color: Color,
    icon: Option<Handle<Image>>,
    angle: f32,
    nx: f32,
    ny: f32,
    size_frac: f32,
    clip_circle: f32,
    /// True when this blip's uuid matches the widget's `RadarTargetHighlight`.
    highlighted: bool,
}

/// Each frame: for every visible `GenericRadarWidget`, project all
/// `OnRadar` + `RadarAppearance` entities passing the filter into child UI
/// nodes (icons via `RadarIconLookup`). New blips spawn, existing ones
/// update in place, missing ones despawn.
///
/// Supports two optional per-widget components:
/// - [`WorldCentredRadar`] — centres the projection on world origin `(0, 0)`
///   instead of the `RadarCenter` entity, and forces north-up orientation.
/// - [`AutoScaleRadar`] — recomputes `GenericRadarWidget::range` each frame
///   so every visible blip fits within the display area.
/// - [`RadarViewControl`] — when `user_engaged`, overrides the fitted range
///   (divided by `zoom`) and the projection centre (offset by pan), and the
///   resulting geometry is published to [`RadarLastGeom`] for pointer observers.
fn sync_radar_blip_nodes(
    mut commands: Commands,
    mut radars: Query<(
        Entity,
        &mut GenericRadarWidget,
        &ComputedNode,
        &bevy::camera::visibility::InheritedVisibility,
        Option<&Children>,
        Option<&WorldCentredRadar>,
        Option<&AutoScaleRadar>,
        Option<&RadarOverlayEntity>,
        Option<&RadarTargetHighlight>,
        Option<&mut RadarTargetRing>,
        Option<&mut RadarObjectiveRingEntities>,
        Option<&RadarBlipLabels>,
        Option<&mut RadarViewControl>,
    )>,
    overlay_nodes: Query<&ComputedNode>,
    blips: Query<(
        Entity,
        &OnRadar,
        &RadarAppearance,
        &BlipWorldPose,
        Option<&RadarEntityUuid>,
        Option<&BlipLabel>,
    )>,
    centers: Query<&RadarCenter>,
    mut existing_blip_nodes: Query<
        (
            &mut Node,
            &MaterialNode<RadarBlipMaterial>,
            &mut Transform,
            &RadarBlipNode,
        ),
        (Without<RadarRegionNode>, Without<RadarLabelNode>),
    >,
    mut existing_region_nodes: Query<
        (
            &mut Node,
            &mut BackgroundColor,
            &mut BorderColor,
            &RadarRegionNode,
        ),
        (Without<RadarBlipNode>, Without<RadarLabelNode>),
    >,
    mut existing_label_nodes: Query<
        (&mut Node, &RadarLabelNode),
        (Without<RadarBlipNode>, Without<RadarRegionNode>),
    >,
    icons: Res<RadarIconLookup>,
    mut blip_materials: ResMut<Assets<RadarBlipMaterial>>,
    fallback: Option<Res<RadarBlipFallbackIcon>>,
) {
    // Cache the global RadarCenter once; only used for ship-centred widgets.
    let global_center = centers.iter().next();

    for (
        radar_entity,
        mut widget,
        computed,
        vis,
        children,
        world_centred,
        auto_scale,
        overlay,
        target_highlight,
        mut target_ring,
        mut objective_rings,
        show_labels,
        view_control,
    ) in radars.iter_mut()
    {
        if !vis.get() {
            continue;
        }
        // When an overlay entity exists, use its computed size for the radar
        // radius so blip positioning and clipping respect the overlay's actual
        // rendered area rather than the root widget's area.
        //
        // `ComputedNode::size()` returns **physical** pixels, but child blip
        // nodes are positioned with `Val::Px(..)` which Bevy treats as
        // **logical** pixels (multiplied internally by the scale factor at
        // layout time). On HiDPI / phone displays (scale_factor != 1) these
        // two spaces differ, so we promote the centre / radius into logical
        // pixels here by multiplying by `inverse_scale_factor`. All
        // downstream `BlipIntent.left/top/size_px` values are therefore in
        // logical pixels and can be fed directly to `Val::Px`.
        let inv_sf = computed.inverse_scale_factor();
        let face_size_logical = overlay
            .and_then(|o| overlay_nodes.get(o.0).ok())
            .map_or_else(|| computed.size(), |cn| cn.size())
            * inv_sf;
        let computed_size_logical = computed.size() * inv_sf;
        let center_x_px = computed_size_logical.x * 0.5;
        let center_y_px = computed_size_logical.y * 0.5;
        let radar_radius_px =
            face_size_logical.x.min(face_size_logical.y) * 0.5 * widget.face_fraction;
        if radar_radius_px <= 0.0 {
            continue;
        }

        // ── Determine projection centre and effective yaw ─────────────────────
        let (center_x, center_z, effective_yaw) = if world_centred.is_some() {
            (0.0_f32, 0.0_f32, 0.0_f32)
        } else {
            let Some(center) = global_center else {
                continue;
            };
            let yaw = match widget.orientation {
                OrientationMode::ShipRelative => center.yaw,
                OrientationMode::WorldFixed => 0.0,
            };
            (center.world_x, center.world_z, yaw)
        };

        // ── Auto-scale range ──────────────────────────────────────────────────
        if let Some(auto_scale) = auto_scale {
            let max_dist = blips
                .iter()
                .filter(|(_, on_radar, _, _, _, _)| is_on_radar(&widget.filter, &on_radar.0))
                .filter_map(|(_, _, appearance, blip_pose, _, _)| {
                    let dx = blip_pose.x - center_x;
                    let dz = blip_pose.z - center_z;
                    let dist = (dx * dx + dz * dz).sqrt() + appearance.world_size;
                    if dist > 0.0 {
                        Some(dist)
                    } else {
                        None
                    }
                })
                .fold(0.0_f32, f32::max);
            if max_dist > 0.0 {
                widget.range = (max_dist * auto_scale.margin).max(auto_scale.min_range);
            } else {
                widget.range = auto_scale.min_range;
            }
        }

        // ── User zoom / pan override ──────────────────────────────────────────
        // The auto-scale block above fitted `widget.range` around the BASE
        // (world-centred or ship-centred) centre, independent of pan. When the
        // user has engaged manual control, divide that fitted range by `zoom`
        // and offset the projection centre by the accumulated pan (world units).
        let (center_x, center_z) =
            if let Some(vc) = view_control.as_deref().filter(|v| v.user_engaged) {
                let fitted = widget.range;
                let zoomed = (fitted / vc.zoom).max(RADAR_MIN_RANGE);
                widget.range = zoomed;
                (center_x + vc.pan_x, center_z + vc.pan_z)
            } else {
                (center_x, center_z)
            };

        // Publish geometry so pointer observers can map pixels → world units.
        // Only widgets that opt into manual zoom/pan (`RadarViewControl`) have a
        // pointer observer that reads this, so skip the per-frame write elsewhere.
        if view_control.is_some() {
            commands.entity(radar_entity).insert(RadarLastGeom {
                radar_radius_px,
                range: widget.range,
            });
        }

        let range = widget.range;

        // ── Build intended blip set (point entities) ─────────────────────────
        let mut intended: HashMap<Entity, BlipIntent> = HashMap::new();
        // Track the screen position of the highlighted blip so we can place
        // the targeting ring node (spawned separately at a fixed pixel size).
        let mut highlighted_blip_center: Option<(f32, f32)> = None;
        // Track the screen positions of objective-target blips for gold rings.
        let mut objective_blip_centers: HashMap<Entity, (f32, f32)> = HashMap::new();

        // ── Build intended region set (region shape entities) ────────────────
        // source entity → (nx, ny, colour, shape, outer_size_px)
        let mut intended_regions: HashMap<Entity, (f32, f32, Color, RegionRadarShape, f32)> =
            HashMap::new();

        // ── Build intended label set (named navigational blips) ──────────────
        // source entity → (text, label_left, label_top)
        let mut intended_labels: HashMap<Entity, (String, f32, f32)> = HashMap::new();

        for (src, on_radar, appearance, blip_pose, blip_uuid, blip_label) in blips.iter() {
            if !is_on_radar(&widget.filter, &on_radar.0) {
                continue;
            }
            // Objective-marker beacons are invisible until they become an active
            // objective target. This keeps the nav chart uncluttered — beacons
            // that haven't been referenced by any active objective never render.
            if on_radar.0.iter().any(|t| t == "objective_marker") && !appearance.objective_target {
                continue;
            }
            let ent_radius = appearance.world_size * 0.5;
            let Some((nx, ny)) = project_radar_entity(
                blip_pose.x,
                blip_pose.z,
                center_x,
                center_z,
                effective_yaw,
                range,
                ent_radius,
                &widget.orientation,
            ) else {
                continue;
            };
            if nx * nx + ny * ny > 1.0 {
                continue;
            }

            if let Some(region_colour) = appearance.region_colour {
                // ── Region entity: render as shape ────────────────────────────
                let shape = appearance
                    .region_shape
                    .clone()
                    .unwrap_or(RegionRadarShape::Sphere {
                        radius: appearance.world_size,
                    });
                let outer_size_px = match &shape {
                    RegionRadarShape::Sphere { radius } => {
                        world_size_to_px(*radius, range, radar_radius_px)
                    }
                    RegionRadarShape::Torus { outer_radius, .. } => {
                        world_size_to_px(*outer_radius, range, radar_radius_px)
                    }
                    RegionRadarShape::Box {
                        half_extents_x,
                        half_extents_z,
                        ..
                    } => {
                        let w = world_size_to_px(*half_extents_x, range, radar_radius_px);
                        let h = world_size_to_px(*half_extents_z, range, radar_radius_px);
                        w.max(h)
                    }
                };
                intended_regions.insert(src, (nx, ny, region_colour, shape, outer_size_px));
                if show_labels.is_some() {
                    let name = blip_label.and_then(|l| l.0.clone());
                    if should_label(&name, &on_radar.0) {
                        let label_left = center_x_px + nx * radar_radius_px;
                        let label_top = center_y_px - ny * radar_radius_px;
                        intended_labels
                            .insert(src, (name.unwrap_or_default(), label_left, label_top));
                    }
                }
            } else {
                // ── Point entity: render as icon ─────────────────────────────
                let icon_angle =
                    icon_rotation_angle(blip_pose.yaw, effective_yaw, &widget.orientation);
                let size_px = world_size_to_px(appearance.world_size, range, radar_radius_px);
                let half = size_px * 0.5;
                let (left, top) =
                    blip_local_offset(nx, ny, center_x_px, center_y_px, radar_radius_px, half);
                let icon_handle = icons.0.get(&appearance.icon).cloned();
                let size_frac = half / radar_radius_px;
                let clip_circle = if widget.clip_mode == RadarClipMode::Circle {
                    1.0_f32
                } else {
                    0.0_f32
                };
                let highlight_match = matches!(
                    (target_highlight, blip_uuid),
                    (Some(hl), Some(uuid)) if hl.0.as_deref() == Some(uuid.0.as_str())
                );
                if highlight_match {
                    // Record blip centre so we can position the ring node below.
                    highlighted_blip_center = Some((left + half, top + half));
                }
                if appearance.objective_target {
                    objective_blip_centers.insert(src, (left + half, top + half));
                }
                let blip_color = if highlight_match {
                    Color::srgb(1.0, 0.85, 0.1)
                } else {
                    appearance.color
                };
                intended.insert(
                    src,
                    BlipIntent {
                        left,
                        top,
                        size_px,
                        color: blip_color,
                        icon: icon_handle,
                        angle: icon_angle,
                        nx,
                        ny,
                        size_frac,
                        clip_circle,
                        highlighted: highlight_match,
                    },
                );
                if show_labels.is_some() {
                    let name = blip_label.and_then(|l| l.0.clone());
                    if should_label(&name, &on_radar.0) {
                        let label_left = left + size_px;
                        let label_top = top;
                        intended_labels
                            .insert(src, (name.unwrap_or_default(), label_left, label_top));
                    }
                }
            }
        }

        // ── Reconcile existing children ───────────────────────────────────────
        if let Some(children) = children {
            for child in children.iter() {
                if let Ok((mut node, mat_node, mut transform, tag)) =
                    existing_blip_nodes.get_mut(child)
                {
                    if let Some(intent) = intended.remove(&tag.source) {
                        node.left = Val::Px(intent.left);
                        node.top = Val::Px(intent.top);
                        node.width = Val::Px(intent.size_px);
                        node.height = Val::Px(intent.size_px);
                        transform.rotation = Quat::from_rotation_z(intent.angle);
                        if let Some(mat) = blip_materials.get_mut(&mat_node.0) {
                            let LinearRgba {
                                red,
                                green,
                                blue,
                                alpha,
                            } = intent.color.to_linear();
                            mat.color_r = red;
                            mat.color_g = green;
                            mat.color_b = blue;
                            mat.color_a = alpha;
                            mat.radar_nx = intent.nx;
                            mat.radar_ny = intent.ny;
                            mat.size_frac = intent.size_frac;
                            mat.clip_circle = intent.clip_circle;
                            mat.highlighted = if intent.highlighted { 1.0 } else { 0.0 };
                            if let Some(h) = intent.icon {
                                mat.icon = h;
                            }
                        }
                    } else {
                        commands.entity(child).despawn();
                    }
                } else if let Ok((mut node, mut bg, mut border_color, tag)) =
                    existing_region_nodes.get_mut(child)
                {
                    if let Some((nx, ny, colour, shape, _)) = intended_regions.remove(&tag.source) {
                        update_region_node(
                            &mut node,
                            &mut bg,
                            &mut border_color,
                            nx,
                            ny,
                            colour,
                            &shape,
                            range,
                            center_x_px,
                            center_y_px,
                            radar_radius_px,
                        );
                    } else {
                        commands.entity(child).despawn();
                    }
                } else if let Ok((mut node, tag)) = existing_label_nodes.get_mut(child) {
                    if let Some((_text, label_left, label_top)) =
                        intended_labels.remove(&tag.source)
                    {
                        node.left = Val::Px(label_left);
                        node.top = Val::Px(label_top);
                    } else {
                        commands.entity(child).despawn();
                    }
                }
            }
        }

        // ── Spawn new blip nodes ─────────────────────────────────────────────
        if !intended.is_empty() || !intended_regions.is_empty() || !intended_labels.is_empty() {
            let fallback_icon: Handle<Image> =
                fallback.as_ref().map(|f| f.0.clone()).unwrap_or_default();
            commands.entity(radar_entity).with_children(|parent| {
                for (source, intent) in intended.drain() {
                    let icon_handle = intent.icon.unwrap_or_else(|| fallback_icon.clone());
                    let LinearRgba {
                        red,
                        green,
                        blue,
                        alpha,
                    } = intent.color.to_linear();
                    let mat_handle = blip_materials.add(RadarBlipMaterial {
                        icon: icon_handle,
                        color_r: red,
                        color_g: green,
                        color_b: blue,
                        color_a: alpha,
                        radar_nx: intent.nx,
                        radar_ny: intent.ny,
                        size_frac: intent.size_frac,
                        clip_circle: intent.clip_circle,
                        highlighted: if intent.highlighted { 1.0 } else { 0.0 },
                        _pad0: 0.0,
                        _pad1: 0.0,
                        _pad2: 0.0,
                    });
                    let node = Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(intent.left),
                        top: Val::Px(intent.top),
                        width: Val::Px(intent.size_px),
                        height: Val::Px(intent.size_px),
                        ..default()
                    };
                    let transform = Transform::from_rotation(Quat::from_rotation_z(intent.angle));
                    parent.spawn((
                        node,
                        MaterialNode(mat_handle),
                        transform,
                        ZIndex(10),
                        RadarBlipNode { source },
                        Button,
                        Interaction::default(),
                    ));
                }
                for (source, (nx, ny, colour, shape, _)) in intended_regions.drain() {
                    let (node, bg, border_color) = region_shape_node(
                        source,
                        nx,
                        ny,
                        colour,
                        &shape,
                        range,
                        center_x_px,
                        center_y_px,
                        radar_radius_px,
                    );
                    parent.spawn((
                        node,
                        bg,
                        border_color,
                        ZIndex(3),
                        RadarRegionNode { source },
                    ));
                }
                for (source, (text, label_left, label_top)) in intended_labels.drain() {
                    parent.spawn((
                        Text::new(text),
                        TextFont {
                            font_size: 11.0,
                            ..default()
                        },
                        TextColor(Color::srgba(0.6, 1.0, 0.85, 0.9)),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(label_left),
                            top: Val::Px(label_top),
                            ..default()
                        },
                        ZIndex(11),
                        RadarLabelNode { source },
                    ));
                }
            });
        }

        // ── Targeting ring node ───────────────────────────────────────────────
        // When this widget has a `RadarTargetRing` component, spawn/update a
        // fixed-size circular ring UI node centred on the highlighted blip.
        // The ring is independent of the blip icon size so it is always visible.
        const RING_PX: f32 = 28.0;
        const RING_BORDER_PX: f32 = 2.0;
        if let Some(ref mut ring) = target_ring {
            match highlighted_blip_center {
                Some((cx, cy)) => {
                    // Position the ring centred on the blip icon.
                    let ring_left = cx - RING_PX * 0.5;
                    let ring_top = cy - RING_PX * 0.5;
                    let ring_node = Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(ring_left),
                        top: Val::Px(ring_top),
                        width: Val::Px(RING_PX),
                        height: Val::Px(RING_PX),
                        border: UiRect::all(Val::Px(RING_BORDER_PX)),
                        border_radius: BorderRadius::all(Val::Percent(50.0)),
                        ..default()
                    };
                    match ring.0 {
                        Some(e) => {
                            // Update existing ring node position.
                            commands.entity(e).insert(ring_node);
                        }
                        None => {
                            // Spawn a new ring node as a child of the radar widget.
                            let e = commands
                                .spawn((
                                    ring_node,
                                    BorderColor::all(Color::srgb(1.0, 0.2, 0.2)),
                                    BackgroundColor(Color::NONE),
                                    ZIndex(20),
                                ))
                                .id();
                            commands.entity(radar_entity).add_child(e);
                            ring.0 = Some(e);
                        }
                    }
                }
                None => {
                    // No highlighted blip — despawn the ring node if it exists.
                    if let Some(e) = ring.0.take() {
                        commands.entity(e).despawn();
                    }
                }
            }
        }

        // ── Objective ring nodes ──────────────────────────────────────────────
        // Maintain a set of ring nodes, one per blip referenced by an active
        // mission objective. Each ring is a fixed-size gold circle positioned
        // at the blip center. Rings are tracked in RadarObjectiveRingEntities.
        const OBJ_RING_PX: f32 = 24.0;
        const OBJ_RING_BORDER_PX: f32 = 2.0;
        if let Some(ref mut rings) = objective_rings {
            let mut next_rings: Vec<(Entity, Entity)> = Vec::new();

            // Update existing rings or spawn new ones for current objective blips.
            for (blip_src, &(cx, cy)) in &objective_blip_centers {
                let ring_left = cx - OBJ_RING_PX * 0.5;
                let ring_top = cy - OBJ_RING_PX * 0.5;
                let ring_node = Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(ring_left),
                    top: Val::Px(ring_top),
                    width: Val::Px(OBJ_RING_PX),
                    height: Val::Px(OBJ_RING_PX),
                    border: UiRect::all(Val::Px(OBJ_RING_BORDER_PX)),
                    border_radius: BorderRadius::all(Val::Percent(50.0)),
                    ..default()
                };

                let ring_entity = match rings.0.iter().find(|(blip, _)| blip == blip_src) {
                    Some((_, ring)) => {
                        commands.entity(*ring).insert(ring_node);
                        *ring
                    }
                    None => {
                        let e = commands
                            .spawn((
                                ring_node,
                                BorderColor::all(Color::srgb(1.0, 0.75, 0.1)),
                                BackgroundColor(Color::NONE),
                                ZIndex(15),
                            ))
                            .id();
                        commands.entity(radar_entity).add_child(e);
                        e
                    }
                };
                next_rings.push((*blip_src, ring_entity));
            }

            // Despawn rings for blips that are no longer objective targets.
            for (blip, ring) in &rings.0 {
                if !next_rings.iter().any(|(b, _)| b == blip) {
                    commands.entity(*ring).despawn();
                }
            }

            rings.0 = next_rings;
        }
    }
}

/// Compute the Z-rotation angle for a radar blip icon based on the
/// entity's world yaw and the radar's orientation mode.
fn icon_rotation_angle(entity_yaw: f32, effective_yaw: f32, _orientation: &OrientationMode) -> f32 {
    effective_yaw - entity_yaw
}

/// Update an existing region node's layout to match the current projection.
fn update_region_node(
    node: &mut Node,
    bg: &mut BackgroundColor,
    border_color: &mut BorderColor,
    nx: f32,
    ny: f32,
    colour: Color,
    shape: &RegionRadarShape,
    range: f32,
    center_x_px: f32,
    center_y_px: f32,
    radar_radius_px: f32,
) {
    match *shape {
        RegionRadarShape::Sphere { radius } => {
            let diameter_px = world_size_to_px(radius, range, radar_radius_px).max(2.0);
            let half = diameter_px * 0.5;
            let (left, top) =
                blip_local_offset(nx, ny, center_x_px, center_y_px, radar_radius_px, half);
            node.left = Val::Px(left);
            node.top = Val::Px(top);
            node.width = Val::Px(diameter_px);
            node.height = Val::Px(diameter_px);
            node.border = UiRect::default();
            bg.0 = colour.with_alpha(0.3);
            *border_color = BorderColor::all(colour);
            node.border_radius = BorderRadius::all(Val::Percent(50.0));
        }
        RegionRadarShape::Torus {
            inner_radius,
            outer_radius,
        } => {
            let outer_px = world_size_to_px(outer_radius, range, radar_radius_px).max(2.0);
            let inner_px = (inner_radius / range * radar_radius_px * 2.0).max(0.0);
            let border_px = ((outer_px - inner_px) * 0.5).max(1.0);
            let half = outer_px * 0.5;
            let (left, top) =
                blip_local_offset(nx, ny, center_x_px, center_y_px, radar_radius_px, half);
            node.left = Val::Px(left);
            node.top = Val::Px(top);
            node.width = Val::Px(outer_px);
            node.height = Val::Px(outer_px);
            node.border = UiRect::all(Val::Px(border_px));
            bg.0 = Color::NONE;
            *border_color = BorderColor::all(colour);
            node.border_radius = BorderRadius::all(Val::Percent(50.0));
        }
        RegionRadarShape::Box {
            half_extents_x,
            half_extents_z,
            ..
        } => {
            let width_px = world_size_to_px(half_extents_x, range, radar_radius_px).max(2.0);
            let height_px = world_size_to_px(half_extents_z, range, radar_radius_px).max(2.0);
            let half_w = width_px * 0.5;
            let half_h = height_px * 0.5;
            let (left, top) =
                blip_local_offset(nx, ny, center_x_px, center_y_px, radar_radius_px, half_w);
            let top = top - (half_h - half_w);
            node.left = Val::Px(left);
            node.top = Val::Px(top);
            node.width = Val::Px(width_px);
            node.height = Val::Px(height_px);
            node.border = UiRect::default();
            bg.0 = colour.with_alpha(0.3);
            *border_color = BorderColor::all(colour);
            node.border_radius = BorderRadius::ZERO;
        }
    }
}

/// Build a UI node bundle for a region shape entity.
fn region_shape_node(
    _source: Entity,
    nx: f32,
    ny: f32,
    colour: Color,
    shape: &RegionRadarShape,
    range: f32,
    center_x_px: f32,
    center_y_px: f32,
    radar_radius_px: f32,
) -> (Node, BackgroundColor, BorderColor) {
    let mut node = Node {
        position_type: PositionType::Absolute,
        ..default()
    };

    let mut bg_mut = BackgroundColor(Color::NONE);
    let mut border_color_mut = BorderColor::all(colour);
    update_region_node(
        &mut node,
        &mut bg_mut,
        &mut border_color_mut,
        nx,
        ny,
        colour,
        shape,
        range,
        center_x_px,
        center_y_px,
        radar_radius_px,
    );

    (node, bg_mut, border_color_mut)
}

// ── Radar arcs ────────────────────────────────────────────────────────────────

/// Distinguishes phaser arcs from torpedo arcs so consumers can colour them
/// independently. Carried on `RadarArc` and used as a tie-breaker key when
/// reconciling arc child nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RadarArcKind {
    Phaser,
    Torpedo,
}

/// One arc specification rendered on a radar widget.
///
/// `facing_deg` is the bearing of the arc's centreline in ship-relative
/// coordinates (0° = forward, +90° = starboard, matching `TorpedoTubeConfig`
/// and `PhaserBankConfig` semantics). `fire_arc_deg` is the full arc width.
/// Both are in degrees.
#[derive(Clone, Debug, PartialEq)]
pub struct RadarArc {
    pub id: String,
    pub kind: RadarArcKind,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    pub color: Color,
}

/// Component on a `GenericRadarWidget` listing the arcs to draw.
/// Replace the whole vector to update the on-radar arc set.
#[derive(Component, Clone, Default, Debug)]
pub struct RadarArcs(pub Vec<RadarArc>);

/// Component on a `GenericRadarWidget` selecting at most one blip to highlight
/// in red. The widget matches against blip-source entities carrying
/// [`RadarEntityUuid`].
#[derive(Component, Clone, Default, Debug)]
pub struct RadarTargetHighlight(pub Option<String>);

/// Optional companion to [`RadarTargetHighlight`] on a `GenericRadarWidget`.
/// When present, `sync_radar_blip_nodes` spawns a fixed-size circular ring
/// node centred on the currently targeted blip and despawns it when there is
/// no target. The ring is independent of the blip icon size, so it is always
/// clearly visible regardless of how small the blip is in world-space.
#[derive(Component, Default, Debug)]
pub struct RadarTargetRing(pub Option<Entity>);

/// Stores ring-node entities for blips that are mission-objective targets.
/// Each entry is `(blip_entity, ring_node_entity)`. Reconciled every frame in
/// `sync_radar_blip_nodes` — stale rings are despawned, new ones are spawned.
#[derive(Component, Default, Debug)]
pub struct RadarObjectiveRingEntities(pub Vec<(Entity, Entity)>);

/// Component on a blip-source ECS entity (e.g. an NPC ship snapshot) exposing
/// the entity's wire UUID to the radar widget. Required for
/// `RadarTargetHighlight` lookups; optional otherwise.
#[derive(Component, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RadarEntityUuid(pub String);

/// Per-arc `UiMaterial`. Binding layout matches `assets/shaders/radar_arc.wgsl`:
///   0 = uniform struct.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct RadarArcMaterial {
    #[uniform(0)]
    pub color_r: f32,
    #[uniform(0)]
    pub color_g: f32,
    #[uniform(0)]
    pub color_b: f32,
    #[uniform(0)]
    pub color_a: f32,
    #[uniform(0)]
    pub facing_rad: f32,
    #[uniform(0)]
    pub half_arc_rad: f32,
    #[uniform(0)]
    pub _pad0: f32,
    #[uniform(0)]
    pub _pad1: f32,
}

impl UiMaterial for RadarArcMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/radar_arc.wgsl".into()
    }
}

/// Tags a UI node spawned by `sync_radar_arc_nodes` so the diff can reconcile
/// in place. The key is `(kind, id)`.
#[derive(Component, Clone, Debug)]
struct RadarArcNode {
    kind: RadarArcKind,
    id: String,
}

/// Pure helper: is `bearing_deg` within the arc centred on `facing_deg` of
/// width `fire_arc_deg`? All inputs in degrees; result is a closed interval
/// `[-half, +half]` after wrap-around to `(-180, 180]`.
pub fn arc_contains(facing_deg: f32, fire_arc_deg: f32, bearing_deg: f32) -> bool {
    let half = fire_arc_deg.abs() * 0.5;
    let mut delta = bearing_deg - facing_deg;
    // Wrap to (-180, 180].
    while delta > 180.0 {
        delta -= 360.0;
    }
    while delta <= -180.0 {
        delta += 360.0;
    }
    delta.abs() <= half + 1e-4
}

/// Reconcile arc child nodes for every visible `GenericRadarWidget` that
/// carries [`RadarArcs`]. Arcs render as full-size overlays beneath blips
/// (z-index 5, blips use 10).
#[allow(clippy::type_complexity)]
fn sync_radar_arc_nodes(
    mut commands: Commands,
    radars: Query<(
        Entity,
        &RadarArcs,
        &bevy::camera::visibility::InheritedVisibility,
        Option<&Children>,
    )>,
    mut existing: Query<(
        &mut Node,
        &MaterialNode<RadarArcMaterial>,
        &mut RadarArcNode,
    )>,
    mut materials: ResMut<Assets<RadarArcMaterial>>,
) {
    for (radar_entity, arcs, vis, children) in radars.iter() {
        if !vis.get() {
            continue;
        }
        // Build intended set keyed by (kind, id).
        let mut intended: std::collections::HashMap<(RadarArcKind, String), &RadarArc> =
            std::collections::HashMap::new();
        for arc in arcs.0.iter() {
            intended.insert((arc.kind, arc.id.clone()), arc);
        }

        // Reconcile existing children.
        if let Some(children) = children {
            for child in children.iter() {
                if let Ok((mut node, mat_node, tag)) = existing.get_mut(child) {
                    let key = (tag.kind, tag.id.clone());
                    if let Some(arc) = intended.remove(&key) {
                        node.width = Val::Percent(100.0);
                        node.height = Val::Percent(100.0);
                        if let Some(mat) = materials.get_mut(&mat_node.0) {
                            let LinearRgba {
                                red,
                                green,
                                blue,
                                alpha,
                            } = arc.color.to_linear();
                            mat.color_r = red;
                            mat.color_g = green;
                            mat.color_b = blue;
                            mat.color_a = alpha;
                            mat.facing_rad = arc.facing_deg.to_radians();
                            mat.half_arc_rad = arc.fire_arc_deg.to_radians() * 0.5;
                        }
                    } else {
                        commands.entity(child).despawn();
                    }
                }
            }
        }

        // Spawn new arcs.
        if !intended.is_empty() {
            commands.entity(radar_entity).with_children(|parent| {
                for ((kind, id), arc) in intended.drain() {
                    let LinearRgba {
                        red,
                        green,
                        blue,
                        alpha,
                    } = arc.color.to_linear();
                    let mat = materials.add(RadarArcMaterial {
                        color_r: red,
                        color_g: green,
                        color_b: blue,
                        color_a: alpha,
                        facing_rad: arc.facing_deg.to_radians(),
                        half_arc_rad: arc.fire_arc_deg.to_radians() * 0.5,
                        _pad0: 0.0,
                        _pad1: 0.0,
                    });
                    parent.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            top: Val::Px(0.0),
                            left: Val::Px(0.0),
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        MaterialNode(mat),
                        ZIndex(5),
                        RadarArcNode { kind, id },
                    ));
                }
            });
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Detects press transitions on `RadarBlipNode` UI nodes and triggers
/// `RadarBlipClicked` on the **radar widget entity** (the parent of the
/// blip UI node), carrying the source ECS blip entity in the payload.
///
/// Triggering on the radar (not the source) means each console can attach
/// a per-radar `.observe(...)` handler scoped to its own radar without
/// having to filter by "which radar did this come from?" in the body.
fn detect_radar_blip_press(
    query: Query<(Entity, &Interaction, &RadarBlipNode, &ChildOf), Changed<Interaction>>,
    mut commands: Commands,
) {
    for (_blip_entity, interaction, blip, child_of) in query.iter() {
        if *interaction == Interaction::Pressed {
            commands.trigger(RadarBlipClicked {
                radar: child_of.parent(),
                source: blip.source,
            });
        }
    }
}

/// Plugin for the radar widget. Registered by `ServerViewscreenRadarPlugin`.
pub struct GuiRadarPlugin;

impl Plugin for GuiRadarPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<RadarBlipMaterial>::default())
            .add_plugins(UiMaterialPlugin::<RadarArcMaterial>::default())
            .init_resource::<RadarIconLookup>()
            .add_systems(Startup, setup_radar_blip_fallback)
            .add_systems(
                Update,
                (
                    sync_radar_blip_nodes,
                    sync_radar_arc_nodes,
                    detect_radar_blip_press,
                ),
            );
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── should_label predicate ────────────────────────────────────────────────

    #[test]
    fn should_label_named_nav_entity() {
        assert!(should_label(
            &Some("Sol".to_string()),
            &["star".to_string()]
        ));
        assert!(should_label(
            &Some("Starbase 12".to_string()),
            &["station".to_string(), "friendly".to_string()]
        ));
    }

    #[test]
    fn should_label_rejects_missing_or_empty_name() {
        assert!(!should_label(&None, &["star".to_string()]));
        assert!(!should_label(
            &Some("".to_string()),
            &["planet".to_string()]
        ));
        assert!(!should_label(
            &Some("   ".to_string()),
            &["planet".to_string()]
        ));
    }

    #[test]
    fn should_label_rejects_non_nav_tags() {
        assert!(!should_label(
            &Some("Raider".to_string()),
            &["pirate".to_string()]
        ));
        assert!(!should_label(
            &Some("Torp".to_string()),
            &["torpedo".to_string()]
        ));
        assert!(!should_label(
            &Some("You".to_string()),
            &["player".to_string()]
        ));
    }

    // ── icon_from_radar_icon_str ──────────────────────────────────────────────

    #[test]
    fn icon_from_str_known_tags() {
        assert_eq!(icon_from_radar_icon_str("ship"), RadarIcon::Ship);
        assert_eq!(icon_from_radar_icon_str("pirate"), RadarIcon::Ship);
        assert_eq!(icon_from_radar_icon_str("player"), RadarIcon::PlayerShip);
        assert_eq!(icon_from_radar_icon_str("asteroid"), RadarIcon::Asteroid);
        assert_eq!(
            icon_from_radar_icon_str("asteroid_field"),
            RadarIcon::Asteroid
        );
        assert_eq!(icon_from_radar_icon_str("station"), RadarIcon::Station);
        assert_eq!(icon_from_radar_icon_str("missile"), RadarIcon::Torpedo);
        assert_eq!(icon_from_radar_icon_str("torpedo"), RadarIcon::Torpedo);
        assert_eq!(icon_from_radar_icon_str("planet"), RadarIcon::Planet);
        assert_eq!(icon_from_radar_icon_str("star"), RadarIcon::Star);
    }

    #[test]
    fn icon_from_str_unknown_falls_back_to_ship() {
        assert_eq!(icon_from_radar_icon_str("wormhole"), RadarIcon::Ship);
        assert_eq!(icon_from_radar_icon_str(""), RadarIcon::Ship);
    }

    // ── helper constructors ───────────────────────────────────────────────────

    fn filter(tags: &[&str]) -> RadarFilter {
        RadarFilter(tags.iter().map(|s| s.to_string()).collect())
    }

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    // ── is_on_radar ──────────────────────────────────────────────────────────

    #[test]
    fn matching_tag_passes_filter() {
        assert!(is_on_radar(&filter(&["ship"]), &tags(&["ship"])));
    }

    #[test]
    fn non_matching_tag_fails_filter() {
        assert!(!is_on_radar(&filter(&["ship"]), &tags(&["asteroid"])));
    }

    #[test]
    fn empty_filter_excludes_all() {
        assert!(!is_on_radar(&filter(&[]), &tags(&["ship"])));
        assert!(!is_on_radar(&filter(&[]), &tags(&["asteroid"])));
    }

    #[test]
    fn empty_tags_excluded_by_any_filter() {
        assert!(!is_on_radar(&filter(&["ship"]), &tags(&[])));
    }

    #[test]
    fn multi_tag_entity_passes_if_any_match() {
        // entity has "pirate" and "ship"; filter includes "ship"
        assert!(is_on_radar(&filter(&["ship"]), &tags(&["pirate", "ship"])));
    }

    #[test]
    fn player_tag_passes_player_filter() {
        assert!(is_on_radar(&filter(&["player"]), &tags(&["player"])));
    }

    #[test]
    fn player_does_not_pass_ship_only_filter() {
        // player and ship are distinct tags
        assert!(!is_on_radar(&filter(&["ship"]), &tags(&["player"])));
    }

    #[test]
    fn all_tags_filter_accepts_all_known_tags() {
        let f = filter(&[
            "player",
            "ship",
            "asteroid",
            "asteroid_field",
            "station",
            "missile",
            "planet",
            "star",
            "region",
        ]);
        for tag in &[
            "player",
            "ship",
            "asteroid",
            "asteroid_field",
            "station",
            "missile",
            "planet",
            "star",
            "region",
        ] {
            assert!(is_on_radar(&f, &tags(&[tag])), "tag {tag} should pass");
        }
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
        // Square radar: center == radar_radius == 140.
        let (left, top) = blip_local_offset(0.0, 0.0, 140.0, 140.0, 140.0, 8.0);
        assert!((left - 132.0).abs() < 1e-5);
        assert!((top - 132.0).abs() < 1e-5);
    }

    #[test]
    fn blip_local_offset_right_edge() {
        let (left, top) = blip_local_offset(1.0, 0.0, 140.0, 140.0, 140.0, 8.0);
        assert!((left - 272.0).abs() < 1e-5);
        assert!((top - 132.0).abs() < 1e-5);
    }

    #[test]
    fn blip_local_offset_top_edge() {
        // ny = 1 → top = 140 - 1*140 - 8 = -8 (Y flipped to UI's y-down).
        let (left, top) = blip_local_offset(0.0, 1.0, 140.0, 140.0, 140.0, 8.0);
        assert!((left - 132.0).abs() < 1e-5);
        assert!((top - (-8.0)).abs() < 1e-5);
    }

    #[test]
    fn blip_local_offset_bottom_edge() {
        let (left, top) = blip_local_offset(0.0, -1.0, 140.0, 140.0, 140.0, 8.0);
        assert!((left - 132.0).abs() < 1e-5);
        assert!((top - 272.0).abs() < 1e-5);
    }

    #[test]
    fn blip_local_offset_non_square_widget_centers_correctly() {
        // Widget is 300 wide × 200 tall → center_x=150, center_y=100, radius=100.
        // Player ship at (0,0) should land at (center_x - half, center_y - half).
        let half = 8.0;
        let (left, top) = blip_local_offset(0.0, 0.0, 150.0, 100.0, 100.0, half);
        assert!((left - (150.0 - half)).abs() < 1e-5, "left={left}");
        assert!((top - (100.0 - half)).abs() < 1e-5, "top={top}");
    }

    // ── HiDPI / scale-factor regression ──────────────────────────────────────
    //
    // `ComputedNode::size()` and `GlobalTransform::translation()` are in
    // **physical** pixels, but blip nodes are positioned with `Val::Px(..)`
    // which Bevy treats as **logical** pixels. On HiDPI displays
    // (`scale_factor != 1`) `sync_radar_blip_nodes` must convert physical
    // values to logical (multiply by `inverse_scale_factor`) before feeding
    // them into `blip_local_offset`, otherwise blips render at the wrong
    // distance from radar centre and `on_tactical_radar_tap` projects
    // entities into a different space than the pointer.

    #[test]
    fn blip_local_offset_at_phone_scale_factor_lays_out_in_logical_px() {
        // Phone: scale_factor = 2, so a 240-logical-px radar widget has
        // ComputedNode::size() == 480 physical px.
        let scale_factor: f32 = 2.0;
        let inv_sf = 1.0 / scale_factor;
        let physical_size = 480.0_f32;

        // The renderer must convert to logical before computing layout, so
        // the values flowing into blip_local_offset are 240, 240, 120.
        let logical_size = physical_size * inv_sf;
        let centre_logical = logical_size * 0.5;
        let radius_logical = logical_size * 0.5;

        // Entity projected at right edge of the radar (nx=1, ny=0).
        let (left, top) = blip_local_offset(
            1.0,
            0.0,
            centre_logical,
            centre_logical,
            radius_logical,
            8.0,
        );

        // Expected (in logical px, ready for Val::Px):
        //   left = 120 + 1.0 * 120 - 8 = 232
        //   top  = 120 - 0.0 * 120 - 8 = 112
        assert!((left - 232.0).abs() < 1e-5, "left={left}");
        assert!((top - 112.0).abs() < 1e-5, "top={top}");

        // Cross-check the bug we just fixed: had the renderer fed the raw
        // physical values straight in (centre=240, radius=240), the right-edge
        // blip would have ended up at left = 240 + 240 - 8 = 472, which is
        // far outside the 240-logical-px-wide radar widget → blips drift /
        // clip / tap math diverges from visual layout on phones.
        let (bad_left, _) = blip_local_offset(
            1.0,
            0.0,
            physical_size * 0.5,
            physical_size * 0.5,
            physical_size * 0.5,
            8.0,
        );
        assert!(
            bad_left > logical_size,
            "regression sentinel: bad_left={bad_left} should overflow {logical_size}"
        );
    }

    // ── apply_zoom_step ───────────────────────────────────────────────────────

    #[test]
    fn apply_zoom_step_multiplies_in_range() {
        assert!((apply_zoom_step(1.0, 1.25) - 1.25).abs() < 1e-6);
        assert!((apply_zoom_step(2.0, 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn apply_zoom_step_clamps_at_max() {
        assert_eq!(apply_zoom_step(RADAR_MAX_ZOOM, 4.0), RADAR_MAX_ZOOM);
    }

    #[test]
    fn apply_zoom_step_clamps_at_min() {
        assert_eq!(apply_zoom_step(RADAR_MIN_ZOOM, 0.1), RADAR_MIN_ZOOM);
    }

    // ── pinch_zoom ────────────────────────────────────────────────────────────

    #[test]
    fn pinch_zoom_applies_distance_ratio() {
        // Fingers move apart 2× → zoom in 2×.
        assert!((pinch_zoom(1.0, 50.0, 100.0) - 2.0).abs() < 1e-6);
        // Fingers move together → zoom out.
        assert!((pinch_zoom(2.0, 100.0, 50.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pinch_zoom_zero_prev_dist_is_noop() {
        assert_eq!(pinch_zoom(1.5, 0.0, 100.0), 1.5);
    }

    #[test]
    fn pinch_zoom_respects_clamp() {
        assert_eq!(pinch_zoom(RADAR_MAX_ZOOM, 10.0, 1000.0), RADAR_MAX_ZOOM);
    }

    // ── px_to_world_delta ─────────────────────────────────────────────────────

    #[test]
    fn px_to_world_delta_zero_radius_is_zero() {
        assert_eq!(px_to_world_delta(50.0, 1000.0, 0.0), 0.0);
        assert_eq!(px_to_world_delta(50.0, 1000.0, -1.0), 0.0);
    }

    #[test]
    fn px_to_world_delta_scales_linearly() {
        // Half the radar half-width across a 1000-unit range = 500 world units.
        assert!((px_to_world_delta(100.0, 1000.0, 200.0) - 500.0).abs() < 1e-4);
        // Doubling the pixel delta doubles the world delta.
        let a = px_to_world_delta(10.0, 800.0, 160.0);
        let b = px_to_world_delta(20.0, 800.0, 160.0);
        assert!((b - 2.0 * a).abs() < 1e-4);
    }

    #[test]
    fn radar_blip_press_triggers_event_on_radar_with_source_payload() {
        // detect_radar_blip_press must fire RadarBlipClicked targeting the
        // **radar widget** entity (the parent of the blip UI node), carrying
        // the **source** ECS blip entity in the payload. This is the contract
        // every per-console observer (tactical, sensors, …) relies on.

        use bevy::ecs::observer::On;
        use std::sync::{Arc, Mutex};

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, detect_radar_blip_press);

        // Spawn the source ECS entity (what the wire-level uuid lives on).
        let source = app.world_mut().spawn_empty().id();

        // Spawn the radar widget entity.
        let radar = app.world_mut().spawn_empty().id();

        // Spawn the UI blip node as a child of the radar, with Pressed
        // interaction so the system picks it up next frame.
        let mut blip_cmd = app
            .world_mut()
            .spawn((RadarBlipNode { source }, Interaction::Pressed));
        blip_cmd.insert(ChildOf(radar));

        // Capture observer fires. Register on the **radar** entity — this is
        // the per-radar registration pattern consoles use.
        let captured: Arc<Mutex<Vec<(Entity, Entity)>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_obs = captured.clone();
        app.world_mut()
            .entity_mut(radar)
            .observe(move |trigger: On<RadarBlipClicked>| {
                let ev = trigger.event();
                captured_for_obs.lock().unwrap().push((ev.radar, ev.source));
            });

        // One frame: detect_radar_blip_press queues the trigger; observer runs.
        app.update();

        let hits = captured.lock().unwrap().clone();
        assert_eq!(hits.len(), 1, "observer fired exactly once: {hits:?}");
        assert_eq!(hits[0].0, radar, "event.radar is the radar entity");
        assert_eq!(
            hits[0].1, source,
            "event.source is the source ECS blip entity"
        );
    }

    // ── arc_contains ────────────────────────────────────────────────────────

    #[test]
    fn arc_contains_centre_bearing() {
        assert!(arc_contains(0.0, 90.0, 0.0));
    }

    #[test]
    fn arc_contains_at_arc_edge_inclusive() {
        assert!(arc_contains(0.0, 90.0, 45.0));
        assert!(arc_contains(0.0, 90.0, -45.0));
    }

    #[test]
    fn arc_contains_rejects_outside_arc() {
        assert!(!arc_contains(0.0, 90.0, 46.0));
        assert!(!arc_contains(0.0, 90.0, -46.0));
    }

    #[test]
    fn arc_contains_wraps_around_180() {
        // Facing aft (180°) with 90° arc covers (135° .. -135°).
        assert!(arc_contains(180.0, 90.0, 170.0));
        assert!(arc_contains(180.0, 90.0, -170.0));
        assert!(!arc_contains(180.0, 90.0, 90.0));
    }

    #[test]
    fn arc_contains_negative_arc_is_treated_as_absolute() {
        // A negative arc width is a TOML mistake; absolute keeps behaviour sane.
        assert!(arc_contains(0.0, -90.0, 30.0));
    }

    #[test]
    fn arc_contains_starboard_phaser_at_90_deg() {
        // PRD example: starboard phaser bank facing 90° with 90° arc covers
        // (45° .. 135°) — beam can lock anywhere on the right hemisphere.
        assert!(arc_contains(90.0, 90.0, 90.0));
        assert!(arc_contains(90.0, 90.0, 45.0));
        assert!(arc_contains(90.0, 90.0, 135.0));
        assert!(!arc_contains(90.0, 90.0, 0.0));
        assert!(!arc_contains(90.0, 90.0, 180.0));
    }
}
