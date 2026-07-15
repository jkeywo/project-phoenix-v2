use crate::region_effects::RegionEffectsConfig;
use crate::region_shape::RegionShape;
use serde::de::Error as SerdeError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single standing-doctrine objective declared in an entity's `[behaviour]` block.
///
/// Doctrine objectives replace the old FSM state/transition model: each entry
/// carries a typed `AiDirective`, a utility score (base priority + modifiers +
/// zero-gates), and an optional target speed for the helm to use when executing
/// the directive. The viewscreen aggregator scores these the same way it scores
/// mission objectives; per-system operate functions select the top-scoring
/// directive they can serve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DoctrineObjective {
    /// Stable identifier (e.g. `"patrol-sector"`, `"destroy-hostiles"`).
    pub id: String,
    /// Human-readable prose shown on the captain panel when active.
    pub text: String,
    /// Whether this objective blocks mission completion when active (usually `false` for doctrine).
    #[serde(default)]
    pub mandatory: bool,
    /// Directive kind: `"Patrol"`, `"Destroy"`, `"Reach"`, `"Hail"`, or absent for `None`.
    #[serde(default)]
    pub directive_kind: Option<String>,
    /// Anchor names for `Patrol` directives.
    #[serde(default)]
    pub directive_anchors: Vec<String>,
    /// Whether the patrol loops back to the first anchor after the last.
    #[serde(default)]
    pub directive_loop: bool,
    /// Named target for `Destroy` directives. Runtime-resolved via `AiMemory.target`.
    #[serde(default)]
    pub directive_target: Option<String>,
    /// Named anchor for `Reach` directives.
    #[serde(default)]
    pub directive_anchor: Option<String>,
    /// Named target for `Hail` directives.
    #[serde(default)]
    pub directive_hail_target: Option<String>,
    /// Base utility score before modifiers.
    #[serde(default)]
    pub base_priority: f32,
    /// Veto conditions — force score to 0 when the condition evaluates to false.
    #[serde(default)]
    pub zero_gates: Vec<crate::objectives::ZeroGateCondition>,
    /// Additive score modifiers applied when their condition is true.
    #[serde(default)]
    pub modifiers: Vec<crate::objectives::ConditionModifier>,
    /// Desired helm speed fraction [0, 1] when executing this directive.
    #[serde(default = "default_doctrine_target_speed")]
    pub target_speed: f32,
    /// Distance to maintain from the target (world units) for Destroy directives.
    /// The helm stops thrusting when closer than this.
    #[serde(default = "default_maintain_range")]
    pub maintain_range: f32,
    /// Whether the AI may engage impulse drive while executing this objective.
    /// When absent, defaults to `true` for Reach and Destroy, `false` for Patrol.
    #[serde(default)]
    pub use_impulse: Option<bool>,
}

impl DoctrineObjective {
    /// Resolved effective `use_impulse` value.
    /// Returns `self.use_impulse` if set; otherwise defaults to `true` for
    /// Reach and Destroy directives, `false` for Patrol.
    pub fn effective_use_impulse(&self) -> bool {
        self.use_impulse
            .unwrap_or(!matches!(self.directive_kind.as_deref(), Some("Patrol")))
    }
}

fn default_doctrine_target_speed() -> f32 {
    0.8
}

fn default_maintain_range() -> f32 {
    25.0
}

/// Configuration for an AI behaviour controller attached to an entity.
///
/// The FSM (AiState/TransitionConfig) is dissolved in issue #572. Behaviour is
/// now driven by a list of `DoctrineObjective`s scored by the viewscreen
/// aggregator and interpreted per-system via operate functions.
/// AI profile section: aggression and sensor range for NPC ship AI.
/// Maps to [ai_profile] in entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiProfileConfig {
    pub aggression: f32,
    pub sensor_range: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BehaviourConfig {
    /// Standing-doctrine objectives for this entity template.
    #[serde(default)]
    pub doctrine: Vec<DoctrineObjective>,
    /// Arrival radius in world units — closer than this counts as "reached waypoint".
    /// Defaults to [`crate::ai::WAYPOINT_ARRIVAL_RADIUS`] when absent.
    #[serde(default = "default_waypoint_arrival_radius")]
    pub waypoint_arrival_radius: f32,
    /// Extra clearance (world units) added on top of radii for collision avoidance.
    /// Defaults to [`crate::ai::AVOIDANCE_BUFFER`] when absent.
    #[serde(default = "default_avoidance_buffer")]
    pub avoidance_buffer: f32,
    /// Look-ahead horizon (seconds) for predictive collision avoidance.
    /// Defaults to [`crate::ai::AVOIDANCE_LOOK_AHEAD_SECS`] when absent.
    #[serde(default = "default_avoidance_look_ahead_secs")]
    pub avoidance_look_ahead_secs: f32,
    /// Speed fraction [0, 1] for the Channel-3 Navigation→Helm handoff
    /// fallthrough (nav_goal), used when no local Helm-relevant objective
    /// resolves but Navigation has given a long-range steer target.
    #[serde(default = "default_nav_handoff_speed")]
    pub nav_handoff_speed: f32,
}

fn default_waypoint_arrival_radius() -> f32 {
    crate::ai::WAYPOINT_ARRIVAL_RADIUS
}

fn default_avoidance_buffer() -> f32 {
    crate::ai::AVOIDANCE_BUFFER
}

fn default_nav_handoff_speed() -> f32 {
    0.6
}

fn default_avoidance_look_ahead_secs() -> f32 {
    crate::ai::AVOIDANCE_LOOK_AHEAD_SECS
}

/// Shape variant for the `[mesh]` section of an entity TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshShape {
    Sphere,
    Cuboid,
    Torus,
}

/// Visual mesh definition for an entity.
///
/// Present in entity TOMLs as a `[mesh]` section. The renderer creates the
/// appropriate Bevy primitive and material from this data; entities without
/// a `[mesh]` section are not given a 3-D visual on the viewscreen.
///
/// When `model` is set, the renderer loads a GLB scene instead of creating a
/// procedural shape (the `shape`/`colour`/`radius`/etc. fields are ignored for
/// rendering but kept as fallback). `scale` and `rotation` are applied to both
/// paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshConfig {
    /// Path to a .glb file, e.g. "assets/models/dynasty_destroyer.glb".
    /// When set, overrides the procedural shape rendering.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional rig-sidecar variant name. The model's rig sidecar is looked
    /// up alongside the `.glb` as `<stem>.<variant>.toml`. When absent the
    /// reserved default name `"model"` is used (i.e. `<stem>.model.toml`).
    /// `Some("weathered")` selects `<stem>.weathered.toml`.
    #[serde(default)]
    pub variant: Option<String>,
    pub shape: MeshShape,
    /// RGB colour `[r, g, b]` in linear 0–1 range.
    pub colour: Vec<f32>,
    /// Sphere radius, or torus major radius. Ignored for `cuboid`.
    #[serde(default)]
    pub radius: f32,
    /// Full XYZ dimensions of a `cuboid` mesh.
    #[serde(default)]
    pub size: Option<[f32; 3]>,
    /// Tube radius for a `torus` mesh.
    #[serde(default)]
    pub minor_radius: f32,
    /// Emissive multiplier (the renderer multiplies `colour` by this and feeds
    /// the result into `StandardMaterial::emissive`). When `None`, the renderer
    /// applies its own default (typically `0.4` for general-purpose entities).
    #[serde(default)]
    pub emissive: Option<f32>,
    /// Uniform scale multiplier applied to the entity's transform.
    /// Affects both GLB models and procedural shapes.
    #[serde(default = "default_mesh_scale")]
    pub scale: f32,
    /// Euler rotation [x, y, z] in radians applied to the entity's transform.
    /// Affects both GLB models and procedural shapes.
    #[serde(default)]
    pub rotation: [f32; 3],
    /// Optional distance-based level-of-detail bands, ordered near→far.
    ///
    /// When non-empty, the renderer does **not** render the flat fields above
    /// directly; instead it selects a [`LodLevel`] each frame based on the
    /// entity's distance to the camera (see [`select_lod`]) and renders that
    /// level. Fields the chosen level omits fall back to the flat `MeshConfig`
    /// fields (`colour`/`radius`/`emissive`/`size`/`minor_radius`/`variant`).
    /// The flat fields therefore stay meaningful as shared defaults even when
    /// `lod` is present. Empty (the default) preserves today's single-level
    /// behaviour.
    #[serde(default)]
    pub lod: Vec<LodLevel>,
}

fn default_mesh_scale() -> f32 {
    1.0
}

/// Hysteresis margin (world units) applied by [`select_lod`]. Once an entity is
/// showing a given level, the camera distance must move past the band boundary
/// by more than this margin before the level switches. This prevents rapid
/// flip-flopping when the camera hovers exactly on a boundary.
pub const LOD_HYSTERESIS_MARGIN: f32 = 5.0;

/// One distance band in a [`MeshConfig::lod`] list.
///
/// Levels are declared near→far in ascending `max_distance` order. Each level
/// self-describes as either a GLB level (`model` set) or a procedural level
/// (`shape` set); a level with neither is invalid and is skipped by the
/// renderer. Every visual field is optional — when omitted, the renderer falls
/// back to the corresponding flat [`MeshConfig`] field, so a level only needs
/// to declare what differs from the shared defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LodLevel {
    /// Upper bound (exclusive) of this level's camera-distance band. Level `i`
    /// covers `[bound(i-1), max_distance)`. The final (fallback) level omits
    /// `max_distance`, which is treated as `f32::INFINITY`.
    #[serde(default)]
    pub max_distance: Option<f32>,
    /// Path to a `.glb` file. When set, this is a GLB level and the procedural
    /// fields are ignored for this band.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional rig-sidecar variant name for the GLB (see [`MeshConfig::variant`]).
    #[serde(default)]
    pub variant: Option<String>,
    /// Procedural shape for this band. Used only when `model` is `None`.
    #[serde(default)]
    pub shape: Option<MeshShape>,
    /// RGB colour `[r, g, b]` (linear 0–1). Falls back to `MeshConfig::colour`.
    #[serde(default)]
    pub colour: Option<Vec<f32>>,
    /// Sphere radius / torus major radius. Falls back to `MeshConfig::radius`.
    #[serde(default)]
    pub radius: Option<f32>,
    /// Cuboid full XYZ dimensions. Falls back to `MeshConfig::size`.
    #[serde(default)]
    pub size: Option<[f32; 3]>,
    /// Torus tube radius. Falls back to `MeshConfig::minor_radius`.
    #[serde(default)]
    pub minor_radius: Option<f32>,
    /// Emissive multiplier. Falls back to `MeshConfig::emissive`.
    #[serde(default)]
    pub emissive: Option<f32>,
}

/// Upper bound (exclusive) of level `i`'s distance band. A missing
/// `max_distance` means the band extends to infinity (the fallback level).
fn lod_upper_bound(levels: &[LodLevel], i: usize) -> f32 {
    levels[i].max_distance.unwrap_or(f32::INFINITY)
}

/// The naive (hysteresis-free) level for `distance`: the first level whose
/// upper bound exceeds `distance`, or the last level when `distance` is beyond
/// every bound. `levels` must be non-empty.
fn naive_lod_level(levels: &[LodLevel], distance: f32) -> usize {
    for i in 0..levels.len() {
        if distance < lod_upper_bound(levels, i) {
            return i;
        }
    }
    levels.len() - 1
}

/// Select which LOD level to display for a given camera `distance`, applying
/// hysteresis around band boundaries.
///
/// `levels` are ordered near→far; level `i` nominally covers
/// `[bound(i-1), bound(i))` where `bound(i)` is `levels[i].max_distance`
/// (missing = `f32::INFINITY`). `current` is the level shown last frame, or
/// `None` on the first evaluation.
///
/// Boundary behaviour: when already at `current`, the result only changes once
/// `distance` crosses the relevant boundary by more than
/// [`LOD_HYSTERESIS_MARGIN`]. Crossing outward (to a farther level) requires
/// `distance > upper_bound(current) + margin`; crossing inward (to a nearer
/// level) requires `distance < lower_bound(current) - margin`. Within the
/// margin the level holds. `current == None` uses the naive selection with no
/// hysteresis. Empty `levels` returns `0` (the caller handles the no-LOD case).
pub fn select_lod(levels: &[LodLevel], distance: f32, current: Option<usize>) -> usize {
    if levels.is_empty() {
        return 0;
    }
    let naive = naive_lod_level(levels, distance);
    let Some(cur) = current else {
        return naive;
    };
    // Clamp a possibly-stale index into range, then hold unless the distance has
    // cleared the boundary in the direction of travel by more than the margin.
    let cur = cur.min(levels.len() - 1);
    if naive == cur {
        return cur;
    }
    if naive > cur {
        // Moving outward: only switch once past this level's upper bound + margin.
        if distance > lod_upper_bound(levels, cur) + LOD_HYSTERESIS_MARGIN {
            naive
        } else {
            cur
        }
    } else {
        // Moving inward: only switch once below this level's lower bound - margin.
        let lower_bound = if cur == 0 {
            0.0
        } else {
            lod_upper_bound(levels, cur - 1)
        };
        if distance < lower_bound - LOD_HYSTERESIS_MARGIN {
            naive
        } else {
            cur
        }
    }
}

fn default_star_radius() -> f32 {
    40.0
}

fn default_star_longitude_segments() -> u32 {
    64
}

fn default_star_latitude_segments() -> u32 {
    32
}

fn default_star_surface_colour() -> [f32; 3] {
    [1.0, 0.72, 0.12]
}

fn default_star_hot_colour() -> [f32; 3] {
    [1.0, 0.96, 0.65]
}

fn default_star_cell_colour() -> [f32; 3] {
    [0.95, 0.32, 0.04]
}

fn default_star_halo_colour() -> [f32; 3] {
    [1.0, 0.78, 0.18]
}

fn default_star_halo_radius_multiplier() -> f32 {
    2.4
}

fn default_star_animation_speed() -> f32 {
    1.0
}

/// Animated procedural star/sun visual definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StarConfig {
    pub radius: f32,
    pub longitude_segments: u32,
    pub latitude_segments: u32,
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub surface_colour: [f32; 3],
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub hot_colour: [f32; 3],
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub cell_colour: [f32; 3],
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub halo_colour: [f32; 3],
    pub halo_radius_multiplier: f32,
    pub animation_speed: f32,
}

impl Default for StarConfig {
    fn default() -> Self {
        Self {
            radius: default_star_radius(),
            longitude_segments: default_star_longitude_segments(),
            latitude_segments: default_star_latitude_segments(),
            surface_colour: default_star_surface_colour(),
            hot_colour: default_star_hot_colour(),
            cell_colour: default_star_cell_colour(),
            halo_colour: default_star_halo_colour(),
            halo_radius_multiplier: default_star_halo_radius_multiplier(),
            animation_speed: default_star_animation_speed(),
        }
    }
}

/// Kind of a `[[light]]` entry: a point light or a directional light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Point,
    Directional,
}

/// One `[[light]]` entry from an entity template. Renderer-only data.
///
/// Replaces the per-section light fields that used to live on `[star]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightConfig {
    pub kind: LightKind,
    /// RGB colour `[r, g, b]` in linear 0–1 range.
    pub colour: [f32; 3],
    /// Light intensity (candela for point lights, illuminance for directional).
    pub intensity: f32,
    /// Range in world units. Required for point lights; ignored for directional.
    #[serde(default)]
    pub range: Option<f32>,
    /// If true, the light is spawned as a child entity and continuously
    /// rotated to face the player's ship, regardless of how the parent
    /// entity itself is oriented.
    #[serde(default)]
    pub face_player: bool,
}

/// One entry in the `[[hull.system_hull]]` TOML array — the SystemId-keyed
/// hull config entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemHullEntry {
    /// Stable ship-wide system identifier (e.g. `"helm"`, `"phaser-fore"`).
    /// Deserialises from a bare TOML string via the `SystemId(String)`
    /// newtype.
    pub system_id: crate::messages::SystemId,
    /// Optional human-readable name. When omitted, downstream code falls
    /// back to the raw `system_id` string.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Maximum (and starting) HP for this system.
    pub max_hp: f32,
    /// HP fraction below which the system enters the `Damaged` tier.
    /// Defaults to `0.75` (below 75 % → Damaged).
    #[serde(default = "default_damaged_threshold_pct")]
    pub damaged_threshold_pct: f32,
    /// HP fraction below which the system enters the `Disabled` tier.
    /// Defaults to `0.25` (below 25 % → Disabled).
    #[serde(default = "default_disabled_threshold_pct")]
    pub disabled_threshold_pct: f32,
    /// Performance reduction applied when the system is in the `Damaged` or
    /// `Disabled` tier (fraction, e.g. `0.15` = 15 % reduction).
    /// Defaults to `0.15`.
    #[serde(default = "default_debuff_magnitude")]
    pub debuff_magnitude: f32,
}

fn default_damaged_threshold_pct() -> f32 {
    0.75
}

fn default_disabled_threshold_pct() -> f32 {
    0.25
}

fn default_debuff_magnitude() -> f32 {
    0.15
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HullConfig {
    /// HP for entities with a single hull slot (stations, asteroids, NPC ships).
    #[serde(default)]
    pub hull_integrity: f32,
    /// Per-system hull entries. When present, replaces `hull_integrity`.
    #[serde(default)]
    pub system_hull: Vec<SystemHullEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Ball,
    Capsule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColliderConfig {
    pub shape: ColliderShape,
    pub radius: f32,
    pub length: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppearanceConfig {
    pub colour: String,
    pub size_min: f32,
    pub size_max: f32,
}

/// Declares that an entity should appear on radar and how. There are no
/// defaults derived from tags or entity type anywhere downstream — this
/// table is the single source of truth. At least one of `icon` or
/// `region_colour` must be set; an entity with neither (or with no
/// `[radar_appearance]` table at all) never appears on radar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadarAppearanceConfig {
    /// Point-blip icon name. Free-form — resolved by naming convention to
    /// `assets/radar_icons/Icon-{Capitalized}.png` on both clients. No
    /// whitelist/enum; a missing PNG falls back to a coloured circle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Icon point colour (also the coloured-circle fallback when the icon
    /// PNG is missing). `None` renders the fallback in a single neutral
    /// constant colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<Vec<f32>>,
    /// World-space radius for the icon blip only. When `None`, the entity's
    /// physical collider radius is used. Does not affect region rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// Area-fill colour for region/field entities. Geometry comes from the
    /// entity's existing `[shape]` or `[asteroid_field]` section; this only
    /// controls whether the region is drawn on radar and in what colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_colour: Option<Vec<f32>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmConsoleConfig {
    #[serde(default)]
    pub max_speed: f32,
    #[serde(default)]
    pub max_reverse_speed: f32,
    #[serde(default)]
    pub acceleration: f32,
    #[serde(default)]
    pub deceleration: f32,
    #[serde(default)]
    pub max_yaw_rate: f32,
    /// Radar configuration for the Helm radar widget, from
    /// `[helm_console.radar]`.
    #[serde(default)]
    pub radar: Option<crate::radar_config::RadarConfig>,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Total time in seconds to fully charge the impulse drive.
    /// Defaults to `IMPULSE_CHARGE_DURATION` (3.0 s) when absent.
    #[serde(default = "default_impulse_charge_duration")]
    pub impulse_charge_duration: f32,
    /// Speed multiplier applied when impulse drive is active.
    /// Defaults to `IMPULSE_SPEED_MULTIPLIER` (10.0) when absent.
    #[serde(default = "default_impulse_speed_multiplier")]
    pub impulse_speed_multiplier: f32,
    /// Acceleration multiplier applied while impulse drive is active.
    /// Defaults to `IMPULSE_ACCELERATION_MULTIPLIER` (5.0) when absent.
    #[serde(default = "default_impulse_acceleration_multiplier")]
    pub impulse_acceleration_multiplier: f32,
    /// Minimum distance from target at which AI may engage impulse (world units).
    /// Defaults to 200.0 when absent.
    #[serde(default = "default_impulse_engage_distance")]
    pub impulse_engage_distance: f32,
    /// Distance from target at which AI cancels impulse (world units).
    /// Defaults to 40.0 when absent.
    #[serde(default = "default_impulse_cancel_distance")]
    pub impulse_cancel_distance: f32,
    /// Maximum visual banking (roll) angle in degrees when steering at full
    /// deflection. The ship leans into turns, lerped from 0 toward ±max_bank_deg
    /// based on steering input percentage. 0 = no banking.
    #[serde(default)]
    pub max_bank_deg: f32,
    /// How quickly the ship's visual roll lerps toward the target bank angle
    /// (units: per-second lerp rate). Defaults to
    /// [`crate::ship_plugin::BANK_LERP_RATE`] when absent.
    #[serde(default = "default_bank_lerp_rate")]
    pub bank_lerp_rate: f32,
    /// Optional boost drive config, from `[helm_console.boost]`. When absent the
    /// boost feature is disabled entirely (no button on the helm).
    #[serde(default)]
    pub boost: Option<BoostConfig>,
    /// Optional procedural engine PFX tuning, from `[helm_console.engine_pfx]`.
    /// Rendering code supplies defaults for omitted fields.
    #[serde(default)]
    pub engine_pfx: Option<EnginePfxConfig>,
    /// Optional lateral thrust tuning, from `[helm_console.lateral_thrust]`.
    /// When absent, ShipPhysicsConfig defaults are used.
    #[serde(default)]
    pub lateral_thrust: Option<LateralThrustConfig>,
}

/// Procedural engine trail tuning, from `[helm_console.engine_pfx]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EnginePfxConfig {
    /// RGBA trail colour in 0.0-1.0. When omitted, renderer defaults are used.
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    /// Optional rig-marker names used as exhaust origins.
    #[serde(default)]
    pub markers: Vec<String>,
    /// Seconds each trail segment remains alive. When omitted, renderer defaults are used.
    #[serde(default)]
    pub trail_lifetime_secs: Option<f32>,
    /// Seconds between spawned trail segments. When omitted, renderer defaults are used.
    #[serde(default)]
    pub trail_spawn_interval_secs: Option<f32>,
}

/// Lateral thrust tuning, from `[helm_console.lateral_thrust]`.
/// When absent, the feature uses ShipPhysicsConfig defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateralThrustConfig {
    /// Maximum lateral speed in world units per second.
    #[serde(default = "default_lateral_thrust_max_speed")]
    pub max_lateral_speed: f32,
    /// Lateral acceleration in world units per second squared.
    #[serde(default = "default_lateral_thrust_acceleration")]
    pub lateral_acceleration: f32,
}

fn default_lateral_thrust_max_speed() -> f32 {
    15.0
}

fn default_lateral_thrust_acceleration() -> f32 {
    15.0
}

impl Default for LateralThrustConfig {
    fn default() -> Self {
        Self {
            max_lateral_speed: default_lateral_thrust_max_speed(),
            lateral_acceleration: default_lateral_thrust_acceleration(),
        }
    }
}

/// Boost drive tuning, from `[helm_console.boost]`. Presence of this table is
/// what enables the boost feature on a ship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoostConfig {
    /// Multiplier applied to both max speed and acceleration while engaged.
    pub multiplier: f32,
    /// Multiplier applied to max yaw rate while engaged.
    #[serde(default = "default_boost_steering_multiplier")]
    pub steering_multiplier: f32,
    /// Seconds a full battery lasts while boost is engaged.
    pub active_duration: f32,
    /// Seconds for an empty battery to recharge to full.
    pub recharge_duration: f32,
}

impl HelmConsoleConfig {
    /// Radar range from `[helm_console.radar] range`. Returns `0.0` when the
    /// `[helm_console.radar]` table is absent.
    pub fn effective_radar_range(&self) -> f32 {
        self.radar.as_ref().map_or(0.0, |r| r.range)
    }
}

fn default_bank_lerp_rate() -> f32 {
    crate::ship_plugin::BANK_LERP_RATE
}

fn default_impulse_charge_duration() -> f32 {
    crate::impulse::IMPULSE_CHARGE_DURATION
}

fn default_impulse_speed_multiplier() -> f32 {
    crate::impulse::IMPULSE_SPEED_MULTIPLIER
}

fn default_impulse_engage_distance() -> f32 {
    200.0
}

fn default_impulse_cancel_distance() -> f32 {
    40.0
}

fn default_impulse_acceleration_multiplier() -> f32 {
    crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER
}

fn default_boost_steering_multiplier() -> f32 {
    crate::boost::BOOST_STEERING_MULTIPLIER
}

/// Stable identifier for a phaser bank, parsed verbatim from the TOML
/// `id` field on `[[weapons_console.phaser_banks]]`. Used on the wire
/// to address a specific bank (e.g. `FirePhaser { bank: "port" }`).
pub type PhaserBankId = String;

/// One `[[weapons_console.phaser_banks]]` entry. Defines a single phaser
/// bank's orientation on the ship, its full fire arc (used for manual
/// fire validity and for the radar arc overlay), its narrower auto-fire
/// arc (used by `console_ai` for autonomous firing decisions), and its
/// effective beam range.
///
/// `facing_deg` is the bank's centre bearing in ship-local degrees:
/// `0` = forward (−Z), `90` = starboard (+X), `180` = aft (+Z),
/// `-90` / `270` = port. Wraps freely; only the wrapped direction
/// matters.
///
/// `fire_arc_deg` is the full arc width centred on `facing_deg`. A bank
/// with `facing_deg = -90`, `fire_arc_deg = 180` covers the port
/// hemisphere from forward to aft. Values must be in `(0, 360]`.
///
/// `auto_arc_deg` is the (narrower) auto-fire window, also centred on
/// `facing_deg`. Must satisfy `0 < auto_arc_deg <= fire_arc_deg`.
///
/// `beam_range` is in world units. When `0.0`, falls back to
/// `PhaserCombatConfig::DEFAULT_PHASER_RANGE`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PhaserBankConfig {
    pub id: PhaserBankId,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    pub auto_arc_deg: f32,
    #[serde(default)]
    pub beam_range: f32,
    /// Damage applied to the target per second of active beam.
    /// When `0.0`, falls back to `PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC`.
    #[serde(default)]
    pub beam_damage_per_sec: f32,
    /// Active beam duration in seconds. When `0.0`, falls back to
    /// `PhaserCombatConfig::DEFAULT_BEAM_DURATION_SECS`.
    #[serde(default)]
    pub beam_duration_secs: f32,
    /// Post-beam cooldown in seconds. When `0.0`, falls back to
    /// `PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS`.
    #[serde(default)]
    pub cooldown_secs: f32,
    /// RGBA beam colour as a 4-element float array `[r, g, b, a]` in 0.0–1.0.
    /// When absent (empty vec), the renderer falls back to `beam_render::DEFAULT_BEAM_COLOR`.
    #[serde(default)]
    pub beam_color: Vec<f32>,
    /// Fraction of beam damage that bypasses shields. When `None`, defaults to `0.0`.
    /// Clamped to `[0.0, 1.0]` at apply time.
    #[serde(default)]
    pub shield_pierce: Option<f32>,
    /// Optional rig-marker name linking this bank to a mount point in the
    /// model's rig sidecar (`[markers.<name>]`). When resolvable, downstream
    /// systems may use the marker's position/direction as the beam origin;
    /// when absent or unresolved they fall back to the hull-offset default.
    #[serde(default)]
    pub marker: Option<String>,
}

/// Stable identifier for a blaster bank, parsed verbatim from the TOML
/// `id` field on `[[weapons_console.blaster_banks]]` (issue #631).
pub type BlasterBankId = String;

/// One `[[weapons_console.blaster_banks]]` entry (issue #631).
///
/// A blaster bank fires straight-flying projectiles in data-driven volleys
/// with linear motion prediction at fire time — no homing, no mid-flight
/// correction.
///
/// `facing_deg` and `fire_arc_deg` use the same convention as
/// [`PhaserBankConfig`] (ship-local degrees, 0 = forward). Note: do NOT
/// add `serde(deny_unknown_fields)` here — future issues will add more
/// fields (recoil, screenshake, visual variants).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BlasterBankConfig {
    pub id: BlasterBankId,
    #[serde(default)]
    pub facing_deg: f32,
    #[serde(default = "default_blaster_fire_arc_deg")]
    pub fire_arc_deg: f32,
    #[serde(default = "default_blaster_volley_count")]
    pub volley_count: u32,
    #[serde(default = "default_blaster_volley_interval_secs")]
    pub volley_interval_secs: f32,
    /// Post-volley cooldown in seconds.
    #[serde(default = "default_blaster_cooldown_secs")]
    pub cooldown_secs: f32,
    /// Charge time before firing begins. `0` = instant (click-to-fire);
    /// `>0` = hold-to-fire (reserved for a later issue).
    #[serde(default)]
    pub charge_time_secs: f32,
    #[serde(default = "default_blaster_projectile_speed")]
    pub projectile_speed: f32,
    #[serde(default = "default_blaster_collision_radius")]
    pub collision_radius: f32,
    #[serde(default = "default_blaster_visual_scale")]
    pub visual_scale: f32,
    #[serde(default = "default_blaster_damage")]
    pub damage: i32,
    /// Fraction `[0.0, 1.0]` of damage that bypasses shields entirely.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Recoil impulse magnitude (reserved for a later issue).
    #[serde(default)]
    pub recoil_impulse: f32,
    /// Screenshake magnitude (reserved for a later issue).
    #[serde(default)]
    pub screenshake_magnitude: f32,
    /// Optional rig-marker name linking this bank to a mount point.
    #[serde(default)]
    pub marker: Option<String>,
    /// Maximum range in world units. Projectile lifespan is computed per-bank
    /// as `range / projectile_speed`. Use `default_blaster_range` (35.0) when
    /// absent from TOML.
    #[serde(default = "default_blaster_range")]
    pub range: f32,
}

fn default_blaster_fire_arc_deg() -> f32 {
    90.0
}
fn default_blaster_volley_count() -> u32 {
    3
}
fn default_blaster_volley_interval_secs() -> f32 {
    0.15
}
fn default_blaster_cooldown_secs() -> f32 {
    3.0
}
fn default_blaster_projectile_speed() -> f32 {
    40.0
}
fn default_blaster_collision_radius() -> f32 {
    1.5
}
fn default_blaster_visual_scale() -> f32 {
    1.0
}
fn default_blaster_damage() -> i32 {
    20
}
fn default_blaster_range() -> f32 {
    35.0
}

impl BlasterBankConfig {
    /// Convert this TOML config into a runtime `crate::blaster::BlasterBankConfig`.
    pub fn to_runtime(&self) -> crate::blaster::BlasterBankConfig {
        crate::blaster::BlasterBankConfig {
            id: self.id.clone(),
            facing_deg: self.facing_deg,
            fire_arc_deg: self.fire_arc_deg,
            volley_count: self.volley_count,
            volley_interval_secs: self.volley_interval_secs,
            cooldown_secs: self.cooldown_secs,
            charge_time_secs: self.charge_time_secs,
            projectile_speed: self.projectile_speed,
            collision_radius: self.collision_radius,
            visual_scale: self.visual_scale,
            damage: self.damage,
            shield_pierce: self.shield_pierce,
            recoil_impulse: self.recoil_impulse,
            screenshake_magnitude: self.screenshake_magnitude,
            marker: self.marker.clone(),
            range: self.range,
        }
    }
}

/// Stable identifier for a torpedo tube, parsed verbatim from the TOML
/// `id` field on `[[torpedoes.tubes]]`. Used on the wire to address a
/// specific tube (e.g. `FireTorpedo { tube: "fore_port" }`).
pub type TorpedoTubeId = String;

/// One `[[torpedoes.tubes]]` entry. Defines a single torpedo tube's
/// orientation and launch arc. Ammo is **not** per-tube; the entire
/// ship draws from the shared `[torpedoes].count` pool.
///
/// `facing_deg` and `fire_arc_deg` use the same convention as
/// [`PhaserBankConfig`] (ship-local degrees, 0 = forward).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorpedoTubeConfig {
    pub id: TorpedoTubeId,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    /// Per-tube load/unload time override in seconds. Falls back to the
    /// global `[torpedoes] load_time` when absent.
    #[serde(default)]
    pub load_time: Option<f32>,
    /// Optional rig-marker name linking this tube to a mount point in the
    /// model's rig sidecar (`[markers.<name>]`). When absent or unresolved,
    /// callers fall back to the ship-centre launch origin.
    #[serde(default)]
    pub marker: Option<String>,
    /// Maximum number of torpedoes that can be loaded into this tube at once
    /// (volley capacity). Default `1` preserves existing single-shot
    /// behaviour. Values greater than 1 allow the tube to queue multiple
    /// torpedoes and fire them as a rapid burst.
    #[serde(default = "default_tube_volley_max")]
    pub volley_max: u32,
}

fn default_tube_volley_max() -> u32 {
    1
}

/// Validate a `[[weapons_console.phaser_banks]]` list parsed from TOML.
///
/// Rejects:
///   - empty list (caller may decide to fall back to a single hardcoded
///     bank — this validator returns `Err` so callers see the empty list)
///   - duplicate `id` values
///   - `fire_arc_deg` outside `(0, 360]`
///   - `auto_arc_deg` outside `(0, fire_arc_deg]`
pub fn validate_phaser_banks(banks: &[PhaserBankConfig]) -> Result<(), String> {
    if banks.is_empty() {
        return Err("phaser_banks list is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for b in banks {
        if !seen.insert(b.id.as_str()) {
            return Err(format!("duplicate phaser bank id '{}'", b.id));
        }
        if !(b.fire_arc_deg > 0.0 && b.fire_arc_deg <= 360.0) {
            return Err(format!(
                "phaser bank '{}' has fire_arc_deg={} outside (0, 360]",
                b.id, b.fire_arc_deg
            ));
        }
        if !(b.auto_arc_deg > 0.0 && b.auto_arc_deg <= b.fire_arc_deg) {
            return Err(format!(
                "phaser bank '{}' has auto_arc_deg={} outside (0, fire_arc_deg={}]",
                b.id, b.auto_arc_deg, b.fire_arc_deg
            ));
        }
    }
    Ok(())
}

/// Validate a `[[torpedoes.tubes]]` list parsed from TOML.
///
/// Rejects: empty list, duplicate `id`, `fire_arc_deg` outside `(0, 360]`.
pub fn validate_torpedo_tubes(tubes: &[TorpedoTubeConfig]) -> Result<(), String> {
    if tubes.is_empty() {
        return Err("torpedo tubes list is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for t in tubes {
        if !seen.insert(t.id.as_str()) {
            return Err(format!("duplicate torpedo tube id '{}'", t.id));
        }
        if !(t.fire_arc_deg > 0.0 && t.fire_arc_deg <= 360.0) {
            return Err(format!(
                "torpedo tube '{}' has fire_arc_deg={} outside (0, 360]",
                t.id, t.fire_arc_deg
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponsConsoleConfig {
    /// RGBA colour used by the client Tactical UI for torpedo fire-arc
    /// overlays. When absent, the `ShipClientConfig` default is used.
    #[serde(default)]
    pub torpedo_arc_color: Vec<f32>,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Per-bank phaser definitions parsed from
    /// `[[weapons_console.phaser_banks]]`. Each bank has its own facing,
    /// fire arc, auto-fire arc, range, damage, duration, cooldown, and colour.
    #[serde(default)]
    pub phaser_banks: Vec<PhaserBankConfig>,
    /// Per-bank blaster definitions parsed from
    /// `[[weapons_console.blaster_banks]]` (issue #631). Each bank has its own
    /// facing, fire arc, volley count, damage, shield pierce, and cooldown.
    #[serde(default)]
    pub blaster_banks: Vec<BlasterBankConfig>,
    /// Radar configuration for the Tactical console radar widget, from
    /// `[weapons_console.radar]`.
    #[serde(default)]
    pub radar: Option<crate::radar_config::RadarConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringConsoleConfig {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptainConsoleConfig {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CinematicCameraConfig {
    /// Offset from ship centre in ship-local space (Y-up, Z-behind).
    /// e.g. [0, 8, 15] means 8 units above and 15 units behind.
    pub position: [f32; 3],
    /// Downward pitch from horizontal, in degrees. e.g. 15.0
    #[serde(default = "default_cinematic_pitch")]
    pub default_pitch_deg: f32,
    /// Maximum distance (world units) to consider entities for tracking.
    #[serde(default = "default_cinematic_look_range")]
    pub entity_look_range: f32,
    /// Distance of the default look-ahead point when no entity is tracked.
    #[serde(default = "default_cinematic_look_ahead")]
    pub look_ahead_distance: f32,
    /// Minimum seconds between target re-evaluations (hysteresis).
    #[serde(default = "default_cinematic_hysteresis")]
    pub hysteresis_secs: f32,
    /// How fast (degrees/second) the chase camera's yaw catches up to the
    /// ship's actual heading. A rigid 1:1 lock makes the ship look frozen in
    /// frame during turns (camera and hull rotate identically), so the
    /// camera intentionally lags behind and lets the ship's turn be visible.
    #[serde(default = "default_cinematic_yaw_follow_rate")]
    pub yaw_follow_deg_per_sec: f32,
}

fn default_cinematic_pitch() -> f32 {
    15.0
}
fn default_cinematic_look_range() -> f32 {
    60.0
}
fn default_cinematic_look_ahead() -> f32 {
    100.0
}
fn default_cinematic_hysteresis() -> f32 {
    3.0
}
fn default_cinematic_yaw_follow_rate() -> f32 {
    45.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerAiConfigToml {
    /// Battery fraction below which the AI won't boost weapons power.
    #[serde(default = "default_weapons_battery_floor")]
    pub weapons_battery_floor: f32,
    /// Battery fraction below which the AI won't boost shields power (reserved).
    #[serde(default = "default_shields_battery_floor")]
    pub shields_battery_floor: f32,
    /// Battery fraction below which the AI won't boost helm power.
    #[serde(default = "default_helm_battery_floor")]
    pub helm_battery_floor: f32,
    /// Throttle fraction above which the AI considers helm "active".
    #[serde(default = "default_helm_throttle_threshold")]
    pub helm_throttle_threshold: f32,
}

fn default_weapons_battery_floor() -> f32 {
    0.5
}
fn default_shields_battery_floor() -> f32 {
    0.25
}
fn default_helm_battery_floor() -> f32 {
    0.75
}
fn default_helm_throttle_threshold() -> f32 {
    0.5
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerConfigSection {
    pub capacity: f32,
    pub rates: [f32; 6],
    pub emergency_threshold: f32,
    /// AI tuning parameters loaded from `[power.ai]`.
    #[serde(default)]
    pub ai: Option<PowerAiConfigToml>,
}

/// AI tuning parameters for the shields focus controller.
///
/// Loaded from `[shields_console.ai]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsAiConfigToml {
    /// Maximum time window (seconds) for tracking incoming damage per arc.
    /// Damage older than this is pruned.
    #[serde(default = "default_shields_ai_damage_window_secs")]
    pub damage_window_secs: f32,
    /// Minimum time window (seconds) before the AI acts on damage concentration.
    /// Damage newer than this is not yet considered.
    #[serde(default = "default_shields_ai_min_damage_window_secs")]
    pub min_damage_window_secs: f32,
    /// Percentage of total damage in the window that must hit the same arc
    /// before the AI focuses it (0–100).
    #[serde(default = "default_shields_ai_damage_pct_threshold")]
    pub damage_pct_threshold: f32,
    /// Percentage threshold: if the lowest-arc normalized health is below this
    /// fraction of the next-lowest arc, focus the weakest arc (0–100).
    #[serde(default = "default_shields_ai_health_ratio_threshold")]
    pub health_ratio_threshold: f32,
}

fn default_shields_ai_damage_window_secs() -> f32 {
    4.0
}
fn default_shields_ai_min_damage_window_secs() -> f32 {
    1.0
}
fn default_shields_ai_damage_pct_threshold() -> f32 {
    50.0
}
fn default_shields_ai_health_ratio_threshold() -> f32 {
    50.0
}

/// Config block for the Shields console focus bonuses/penalties.
///
/// Loaded from `[shields_console]` in the ship entity TOML. The nested
/// `[shields_console.base]` sub-block (modelled by [`ShieldsBaseConfig`])
/// supplies the underlying shield-system base values (number of facings,
/// max HP, regen, offline duration) that were previously hardcoded by
/// `ShieldConfig::default()` at `src/weapons/shield.rs:50-58`.
///
/// Damage multiplier fields `focus_focused_damage_multiplier` and
/// `focus_unfocused_damage_multiplier` default to 1.0 (no change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsConsoleConfig {
    /// Extra max HP applied to the focused facing.
    #[serde(default = "default_focus_bonus_max_hp")]
    pub focus_bonus_max_hp: i32,
    /// Extra regen per second applied to the focused facing.
    #[serde(default = "default_focus_bonus_regen")]
    pub focus_bonus_regen: f32,
    /// Max HP subtracted from each non-focused facing.
    #[serde(default = "default_focus_penalty_max_hp")]
    pub focus_penalty_max_hp: i32,
    /// Regen per second subtracted from each non-focused facing.
    #[serde(default = "default_focus_penalty_regen")]
    pub focus_penalty_regen: f32,
    /// HP per second decay applied to non-focused facings when above reduced max.
    #[serde(default = "default_focus_decay_rate")]
    pub focus_decay_rate: f32,
    /// Damage multiplier applied to incoming damage on the focused arc.
    /// 1.0 = no change, 0.7 = 30% reduction.
    #[serde(default = "default_focus_focused_damage_multiplier")]
    pub focus_focused_damage_multiplier: f32,
    /// Damage multiplier applied to incoming damage on non-focused arcs
    /// (when another arc is focused). 1.0 = no change, 1.25 = 25% increase.
    #[serde(default = "default_focus_unfocused_damage_multiplier")]
    pub focus_unfocused_damage_multiplier: f32,
    /// Base shield-system values (number of facings, max HP, regen,
    /// offline duration). When absent the historical hardcoded defaults
    /// from `ShieldConfig::default()` are used.
    #[serde(default)]
    pub base: Option<ShieldsBaseConfig>,
    /// Shield generator frequency (0.0–1.0). Default 0.5. When
    /// `[[shield_arc]]` blocks are present, the first arc's frequency
    /// takes precedence.
    #[serde(default = "default_shield_frequency")]
    pub frequency: f32,
    /// AI tuning parameters for the shields focus controller.
    #[serde(default)]
    pub ai: Option<ShieldsAiConfigToml>,
}

fn default_focus_bonus_max_hp() -> i32 {
    50
}
fn default_focus_bonus_regen() -> f32 {
    5.0
}
fn default_focus_penalty_max_hp() -> i32 {
    25
}
fn default_focus_penalty_regen() -> f32 {
    1.0
}
fn default_focus_decay_rate() -> f32 {
    10.0
}
fn default_focus_focused_damage_multiplier() -> f32 {
    1.0
}
fn default_focus_unfocused_damage_multiplier() -> f32 {
    1.0
}

impl Default for ShieldsConsoleConfig {
    fn default() -> Self {
        Self {
            focus_bonus_max_hp: default_focus_bonus_max_hp(),
            focus_bonus_regen: default_focus_bonus_regen(),
            focus_penalty_max_hp: default_focus_penalty_max_hp(),
            focus_penalty_regen: default_focus_penalty_regen(),
            focus_decay_rate: default_focus_decay_rate(),
            focus_focused_damage_multiplier: default_focus_focused_damage_multiplier(),
            focus_unfocused_damage_multiplier: default_focus_unfocused_damage_multiplier(),
            base: None,
            frequency: default_shield_frequency(),
            ai: None,
        }
    }
}

/// Base shield-system values loaded from `[shields_console.base]`.
///
/// These map 1:1 onto `crate::shield::ShieldConfig` (the runtime struct
/// consumed by `ShieldSystem::new`). All fields default to the historical
/// hardcoded values from `ShieldConfig::default()` so omitting the block
/// changes nothing.
///
/// `num_facings` is exposed for symmetry but the client panel UI assumes
/// 4 quadrants — values other than 4 will break the Shields panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsBaseConfig {
    /// Number of equally-spaced shield facings. The client panel UI
    /// assumes 4 (fore/port/aft/starboard); other values will not render
    /// correctly.
    #[serde(default = "default_shields_num_facings")]
    pub num_facings: usize,
    /// Maximum HP per facing.
    #[serde(default = "default_shields_max_hp")]
    pub max_hp: i32,
    /// HP regenerated per second per online facing.
    #[serde(default = "default_shields_regen_per_sec")]
    pub regen_per_sec: f32,
    /// Seconds a facing stays offline after its HP is depleted.
    #[serde(default = "default_shields_offline_duration")]
    pub offline_duration: f32,
}

fn default_shields_num_facings() -> usize {
    4
}
fn default_shields_max_hp() -> i32 {
    100
}
fn default_shields_regen_per_sec() -> f32 {
    2.0
}
fn default_shields_offline_duration() -> f32 {
    10.0
}

impl Default for ShieldsBaseConfig {
    fn default() -> Self {
        Self {
            num_facings: default_shields_num_facings(),
            max_hp: default_shields_max_hp(),
            regen_per_sec: default_shields_regen_per_sec(),
            offline_duration: default_shields_offline_duration(),
        }
    }
}

impl ShieldsBaseConfig {
    /// Convert this TOML config into a runtime `ShieldConfig`.
    pub fn to_runtime(&self) -> crate::shield::ShieldConfig {
        crate::shield::ShieldConfig {
            num_facings: self.num_facings,
            max_hp: self.max_hp,
            regen_per_sec: self.regen_per_sec,
            offline_duration: self.offline_duration,
        }
    }
}

/// Designer-authored per-arc shield config (issue #514).
///
/// Loaded from top-level `[[shield_arc]]` blocks in the ship TOML. Every
/// arc auto-generates a matching `[[system]]` entry with
/// `kind = "shield_arc"` and `SystemId("shield-arc-<id>")` during
/// `EntityConfig::from_toml`. See [`ShieldArcConfig::to_runtime`] for the
/// runtime conversion consumed by [`crate::shield::ShieldSystem::from_arcs`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldArcConfig {
    /// Stable arc id (kebab-case fragment used to build the fine SystemId).
    pub id: String,
    /// Display label shown in the JS panel (e.g. `"Fore"`, `"All"`).
    pub label: String,
    /// Arc centre bearing in degrees (0 = fore, 90 = starboard).
    pub center_deg: f32,
    /// Arc angular width in degrees.
    pub width_deg: f32,
    /// Per-arc override for max HP; falls back to `[shields_console.base] max_hp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hp: Option<i32>,
    /// Per-arc override for regen/sec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regen_per_sec: Option<f32>,
    /// Per-arc override for offline duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline_duration: Option<f32>,
    /// Per-arc hull HP for damage-tier tracking (fed into `ShipArcHull`).
    #[serde(default)]
    pub hull_max_hp: f32,
    /// HP fraction below which the arc enters Damaged tier. Default 0.75.
    #[serde(default = "default_arc_hull_damaged_threshold")]
    pub hull_damaged_threshold_pct: f32,
    /// HP fraction below which the arc enters Disabled tier. Default 0.25.
    #[serde(default = "default_arc_hull_disabled_threshold")]
    pub hull_disabled_threshold_pct: f32,
    /// Debuff magnitude applied on Damaged/Disabled tier. Default 0.15.
    #[serde(default = "default_arc_hull_debuff_magnitude")]
    pub hull_debuff_magnitude: f32,
    /// Hit-routing priority. When multiple arcs cover the same bearing, the
    /// arc with the highest `priority` value absorbs the hit first. Falls
    /// through to the next tier only when all arcs at the higher priority
    /// covering that bearing are offline. Default 1.
    #[serde(default = "default_arc_priority")]
    pub priority: u32,
    /// Shield generator frequency (0.0–1.0) for this arc's shield system.
    /// When multiple arcs are declared, the first arc's frequency seeds the
    /// ship-wide shield frequency. Default 0.5.
    #[serde(default = "default_shield_frequency")]
    pub frequency: f32,
}

fn default_shield_frequency() -> f32 {
    0.5
}

fn default_arc_priority() -> u32 {
    1
}

fn default_arc_hull_damaged_threshold() -> f32 {
    0.75
}

fn default_arc_hull_disabled_threshold() -> f32 {
    0.25
}

fn default_arc_hull_debuff_magnitude() -> f32 {
    0.15
}

impl ShieldArcConfig {
    /// Convert this TOML block into the runtime shape consumed by
    /// [`crate::shield::ShieldSystem::from_arcs`].
    pub fn to_runtime(&self) -> crate::shield::ArcRuntimeConfig {
        crate::shield::ArcRuntimeConfig {
            id: self.id.clone(),
            label: self.label.clone(),
            center_deg: self.center_deg,
            width_deg: self.width_deg,
            max_hp: self.max_hp,
            regen_per_sec: self.regen_per_sec,
            offline_duration: self.offline_duration,
            priority: self.priority,
        }
    }
}

/// Player-ship phaser combat tuning. All per-bank values (`beam_range`,
/// `beam_damage_per_sec`, `beam_duration_secs`, `cooldown_secs`,
/// `beam_color`, `shield_pierce`) live on each [`PhaserBankConfig`] entry.
///
/// `PhaserCombatConfig` is the player-path source of truth, installed
/// as a Bevy resource by `WeaponsPlugin` and overridden in
/// `spawn_game_start_entities` from the player ship's `[weapons_console]`
/// block.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhaserCombatConfig {
    /// Per-bank facing/arc/range/damage/duration/cooldown/colour list,
    /// parsed from `[[weapons_console.phaser_banks]]` in TOML order. Empty
    /// if the ship has no banks configured. The Tactical UI also receives a
    /// stripped subset of these via `PhaserBankClientConfig`.
    pub banks: Vec<PhaserBankConfig>,
}

impl PhaserCombatConfig {
    /// Canonical baseline phaser values used when a bank's field is `0.0`
    /// (the "zero means absent" convention). Other modules needing the
    /// baseline alias these constants rather than restating the numbers.
    pub const DEFAULT_PHASER_RANGE: f32 = 40.0;
    pub const DEFAULT_BEAM_DURATION_SECS: f32 = 6.0;
    pub const DEFAULT_BEAM_COOLDOWN_SECS: f32 = 6.0;
    pub const DEFAULT_BEAM_DAMAGE_PER_SEC: f32 = 5.0;
}

impl PhaserCombatConfig {
    /// Build a `PhaserCombatConfig` from a parsed `[weapons_console]` block.
    /// All combat tuning is now per-bank; this method just clones the banks list.
    pub fn from_weapons_console(wc: &WeaponsConsoleConfig) -> Self {
        Self {
            banks: wc.phaser_banks.clone(),
        }
    }

    /// Look up a bank by its id. Returns `None` if not found.
    pub fn bank_by_id(&self, id: &str) -> Option<&PhaserBankConfig> {
        self.banks.iter().find(|b| b.id == id)
    }
}

/// Config block for the repair-team state machine in a ship TOML.
///
/// Loaded from `[repair]` in the ship entity TOML (and any NPC ship TOML
/// that wishes to override repair pacing). All fields are optional; missing
/// fields fall back to the same defaults as `RepairTimings::default()` and
/// to the historical hardcoded constants (`TRAVEL_DURATION = 5.0`,
/// `REPAIR_RATE_HP_PER_SEC = 0.5`).
///
/// The same values are forwarded to the client via
/// `ShipClientConfig.repair_travel_secs` and
/// `ShipClientConfig.repair_rate_hp_per_sec` so that the Repair panel UI
/// can derive its progress-bar timings without redefining the constants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairConfig {
    /// Number of repair teams available to this ship (default 0 = legacy, treated as 2).
    #[serde(default)]
    pub repair_team_count: u32,
    /// Seconds a team spends travelling to a console (and the same again returning).
    #[serde(default = "default_repair_travel_duration_secs")]
    pub travel_duration_secs: f32,
    /// HP restored per second while a team is at a console.
    #[serde(default = "default_repair_rate_hp_per_sec")]
    pub repair_rate_hp_per_sec: f32,
}

fn default_repair_travel_duration_secs() -> f32 {
    5.0
}
fn default_repair_rate_hp_per_sec() -> f32 {
    0.5
}

impl Default for RepairConfig {
    fn default() -> Self {
        Self {
            repair_team_count: 0,
            travel_duration_secs: default_repair_travel_duration_secs(),
            repair_rate_hp_per_sec: default_repair_rate_hp_per_sec(),
        }
    }
}

impl RepairConfig {
    /// Convert this TOML config into a runtime `RepairTimings`.
    pub fn to_runtime(&self) -> crate::repair_teams::RepairTimings {
        crate::repair_teams::RepairTimings {
            travel_duration: self.travel_duration_secs,
            repair_rate_hp_per_sec: self.repair_rate_hp_per_sec,
        }
    }
}

/// Config block for an entity's comms range.
///
/// Loaded from `[comms]` in entity TOMLs. When present, the entity is
/// reachable by the player's Comms console while inside `range` units of the
/// player ship. The player ship's own `[comms].range` defines how far it can
/// listen. Effective range between two entities is `min(a.range, b.range)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsConfig {
    /// Comms range in world units.
    pub range: f32,
}

/// Config block for the torpedo system in a ship TOML.
///
/// Loaded from `[torpedoes]` in the ship entity TOML (and any NPC ship TOML
/// that wishes to override the torpedo loadout). All fields are optional;
/// missing fields fall back to the same defaults as `TorpedoConfig::default()`.
///
/// `turn_rate_deg_per_sec` is in **degrees per second** for designer
/// readability; it is converted to radians by `to_runtime()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorpedoesConfig {
    #[serde(default = "default_torpedo_count")]
    pub count: u32,
    #[serde(default = "default_torpedo_damage_hull")]
    pub damage_hull: i32,
    #[serde(default = "default_torpedo_damage_shields")]
    pub damage_shields: i32,
    #[serde(default = "default_torpedo_speed")]
    pub speed: f32,
    /// Maximum turn rate in **degrees per second** (homing).
    /// Converted to radians by `to_runtime()`.
    #[serde(default = "default_torpedo_turn_rate_deg_per_sec")]
    pub turn_rate_deg_per_sec: f32,
    #[serde(default = "default_torpedo_lifespan")]
    pub lifespan: f32,
    #[serde(default = "default_torpedo_load_time")]
    pub load_time: f32,
    /// Proximity-detonation radius in world units.
    #[serde(default = "default_torpedo_detonation_radius")]
    pub detonation_radius: f32,
    /// Fraction of `damage_shields` that bypasses shields and adds to
    /// hull damage at detonation. Default `0.0` — `damage_shields` is
    /// fully absorbed by the facing shield quadrant. `damage_hull` is
    /// unaffected (always hits hull). Clamped to `[0.0, 1.0]` at apply
    /// time.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Per-tube torpedo definitions parsed from `[[torpedoes.tubes]]`.
    /// Each tube has its own facing and fire arc. Ammo is shared via
    /// the top-level `count` field. Empty when the ship has no explicit
    /// per-tube loadout.
    #[serde(default)]
    pub tubes: Vec<TorpedoTubeConfig>,
    /// Interval in seconds between successive torpedo launches in a burst
    /// volley. Applies to all tubes on the ship. Default `0.3s`.
    #[serde(default = "default_burst_interval_secs")]
    pub burst_interval_secs: f32,
}

fn default_burst_interval_secs() -> f32 {
    0.3
}

fn default_torpedo_count() -> u32 {
    10
}
fn default_torpedo_damage_hull() -> i32 {
    50
}
fn default_torpedo_damage_shields() -> i32 {
    5
}
fn default_torpedo_speed() -> f32 {
    15.0
}
fn default_torpedo_turn_rate_deg_per_sec() -> f32 {
    45.0
}
fn default_torpedo_lifespan() -> f32 {
    20.0
}
fn default_torpedo_load_time() -> f32 {
    10.0
}
fn default_torpedo_detonation_radius() -> f32 {
    5.0
}

impl Default for TorpedoesConfig {
    fn default() -> Self {
        Self {
            count: default_torpedo_count(),
            damage_hull: default_torpedo_damage_hull(),
            damage_shields: default_torpedo_damage_shields(),
            speed: default_torpedo_speed(),
            turn_rate_deg_per_sec: default_torpedo_turn_rate_deg_per_sec(),
            lifespan: default_torpedo_lifespan(),
            load_time: default_torpedo_load_time(),
            detonation_radius: default_torpedo_detonation_radius(),
            shield_pierce: 0.0,
            tubes: Vec::new(),
            burst_interval_secs: default_burst_interval_secs(),
        }
    }
}

impl TorpedoesConfig {
    /// Convert this TOML config into a runtime `TorpedoConfig`.
    /// Performs the degrees → radians conversion on `turn_rate_deg_per_sec`.
    pub fn to_runtime(&self) -> crate::torpedo::TorpedoConfig {
        crate::torpedo::TorpedoConfig {
            count: self.count,
            damage_hull: self.damage_hull,
            damage_shields: self.damage_shields,
            speed: self.speed,
            turn_rate: self.turn_rate_deg_per_sec.to_radians(),
            lifespan: self.lifespan,
            load_time: self.load_time,
            detonation_radius: self.detonation_radius,
            shield_pierce: self.shield_pierce,
            burst_interval_secs: self.burst_interval_secs,
        }
    }
}

/// Config block for the Navigation console in a ship TOML.
///
/// Loaded from `[navigation_console]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationConsoleConfig {
    /// System chart radar config for the Navigation console.
    #[serde(default)]
    pub system_chart: crate::radar_config::RadarConfig,
}

/// Config block for the Sensors console in a ship TOML.
///
/// Loaded from `[sensors_console]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorsConsoleConfig {
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Long-range radar config for the Sensors console.
    #[serde(default)]
    pub long_range_radar: crate::radar_config::RadarConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EntityConfig {
    /// Display name (top-level scalar). Informational for most entities; used
    /// by triggers/comms to identify named instances.
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub hull: Option<HullConfig>,
    pub collider: Option<ColliderConfig>,
    pub appearance: Option<AppearanceConfig>,
    pub helm_console: Option<HelmConsoleConfig>,
    pub weapons_console: Option<WeaponsConsoleConfig>,
    pub engineering_console: Option<EngineeringConsoleConfig>,
    pub captain_console: Option<CaptainConsoleConfig>,
    pub power: Option<PowerConfigSection>,
    pub sensors_console: Option<SensorsConsoleConfig>,
    pub navigation_console: Option<NavigationConsoleConfig>,
    /// Unified shields config. Contains focus tuning at the top level and
    /// `num_facings` / `max_hp` / `regen_per_sec` / `offline_duration` in
    /// the nested `.base` sub-block. Every ship (player + NPC) reads this
    /// section — the legacy `[shields]` block was removed as part of the
    /// ship parity audit.
    pub shields_console: Option<ShieldsConsoleConfig>,
    /// Torpedo system config (player ship and any NPC ship with torpedoes).
    pub torpedoes: Option<TorpedoesConfig>,
    /// Repair team timings (travel duration, repair rate).
    pub repair: Option<RepairConfig>,
    /// Comms range — when present, the entity can send/receive comms within
    /// this radius of the player ship.
    pub comms: Option<CommsConfig>,
    /// Asteroid field section from entity template (donut params, grid, etc.)
    pub asteroid_field: Option<AsteroidFieldConfig>,
    /// Region shape section — present for region entities.
    pub shape: Option<RegionShape>,
    /// Region effects section — present for region entities with effects.
    pub effects: Option<RegionEffectsConfig>,
    /// Optional faction UUID this entity belongs to.
    #[serde(default)]
    pub faction: Option<Uuid>,
    /// Optional AI behaviour controller config.
    #[serde(default)]
    pub behaviour: Option<BehaviourConfig>,
    /// Optional AI profile (aggression, sensor range).
    #[serde(default)]
    pub ai_profile: Option<AiProfileConfig>,
    /// Radar appearance (colour, optional radius) for the helm radar blip.
    #[serde(default)]
    pub radar_appearance: Option<RadarAppearanceConfig>,
    /// Targetability section. When absent the entity is not targetable.
    #[serde(default)]
    pub target: Option<crate::entity_target::TargetSection>,
    /// 3-D mesh definition. When present the entity receives a visual on the viewscreen.
    #[serde(default)]
    pub mesh: Option<MeshConfig>,
    /// Procedural animated star/sun visual.
    #[serde(default)]
    pub star: Option<StarConfig>,
    /// Ship class identifier (e.g. "battleship", "cruiser"). Sourced from
    /// top-level TOML `class` field.
    #[serde(default)]
    pub class: Option<String>,
    /// Unique hull identifier/registry number (e.g. "NCC-1701"). Sourced from
    /// top-level TOML `hull_id` field.
    #[serde(default)]
    pub hull_id: Option<String>,
    /// Authored power rating for this ship. Sourced from top-level TOML
    /// `power_rating` field.
    #[serde(default)]
    pub power_rating: Option<i32>,
    /// Per-ship CSS theme URL or inline stylesheet. Sourced from top-level
    /// TOML `css` field.
    #[serde(default)]
    pub css: Option<String>,
    /// Renderer light sources attached to this entity.
    #[serde(default)]
    pub light: Vec<LightConfig>,
    /// Ship stations/systems/power_groups block, populated by parsing the same
    /// `[[station]]` / `[[system]]` / `[power_groups.*]` TOML blocks that
    /// the ship entity TOML uses. Every ship-like entity (player + NPCs) reads
    /// its `ShipConfig` from this field via the same code path — no
    /// entity-type-specific branches.
    #[serde(skip)]
    pub ship_config: Option<crate::ship::config::ShipConfig>,
    /// Cinematic camera config. When present the ship supports the
    /// `ViewMode::Cinematic` viewscreen mode with dynamic entity tracking.
    #[serde(default)]
    pub cinematic_camera: Option<CinematicCameraConfig>,
    /// Designer-authored shield arcs (issue #514). Populated from
    /// top-level `[[shield_arc]]` TOML blocks. When non-empty, the parser
    /// auto-synthesises a matching `[[system]]` entry per arc with
    /// `kind = "shield_arc"` and `SystemId("shield-arc-<id>")`. Consumed
    /// by the runtime path (`ShieldSystem::from_arcs`) that spawns the
    /// ship's `ShipShields` component.
    #[serde(skip)]
    pub shield_arcs: Vec<ShieldArcConfig>,
}

impl EntityConfig {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        let mut value: toml::Value = toml::from_str(s)?;
        // Extract [[shield_arc]] blocks BEFORE stripping so we can populate
        // `EntityConfig.shield_arcs` and synthesise matching `[[system]]`
        // entries during ship-config parsing.
        let shield_arcs_value = if let Some(table) = value.as_table_mut() {
            table.remove("shield_arc")
        } else {
            None
        };
        let shield_arcs: Vec<ShieldArcConfig> = match shield_arcs_value {
            Some(toml::Value::Array(arr)) => arr
                .into_iter()
                .map(|v| v.try_into::<ShieldArcConfig>())
                .collect::<Result<_, _>>()?,
            Some(other) => {
                return Err(SerdeError::custom(format!(
                    "[[shield_arc]] must be an array of tables, got {other:?}"
                )));
            }
            None => Vec::new(),
        };

        // Extract the ship-config sections BEFORE stripping so we can parse
        // them via ShipConfig::from_toml (the same path ship entity TOMLs use).
        let ship_config_toml = if let Some(table) = value.as_table_mut() {
            let has_station = table.contains_key("station");
            let has_system = table.contains_key("system");
            let has_power_groups = table.contains_key("power_groups");
            let out = if has_station || has_system || has_power_groups {
                let mut ship_table = toml::value::Table::new();
                if let Some(v) = table.get("station").cloned() {
                    ship_table.insert("station".to_string(), v);
                }
                if let Some(v) = table.get("system").cloned() {
                    ship_table.insert("system".to_string(), v);
                }
                if let Some(v) = table.get("power_groups").cloned() {
                    ship_table.insert("power_groups".to_string(), v);
                }
                Some(toml::Value::Table(ship_table))
            } else {
                None
            };
            // Now strip so deny_unknown_fields doesn't reject.
            table.remove("stations");
            table.remove("station");
            table.remove("system");
            table.remove("power_groups");
            out
        } else {
            None
        };
        let mut config: EntityConfig = value.try_into()?;
        config.shield_arcs = shield_arcs;

        // Parse the ship_config sub-block via the shared ShipConfig code path.
        // If the ship declares `[[shield_arc]]` blocks, they auto-generate
        // matching `[[system]]` entries with `kind = "shield_arc"` — appended
        // to whatever the ship's TOML already declared before we re-validate.
        if !config.shield_arcs.is_empty() || ship_config_toml.is_some() {
            let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
                .map_err(|e| {
                    serde::de::Error::custom(format!("system registry init failed: {e:?}"))
                })?;
            let kinds: Vec<&str> = registry.kinds().collect();

            let mut ship_config = if let Some(ship_toml_value) = ship_config_toml {
                let ship_toml_str = toml::to_string(&ship_toml_value).map_err(|e| {
                    serde::de::Error::custom(format!("ship_config re-serialise failed: {e}"))
                })?;
                // Parse only (no validation) so ships that declare stations
                // but no `[[system]]` blocks (relying on `[[shield_arc]]`
                // synthesis to populate systems) don't hit `EmptySystems`
                // during the initial pass. Final validation runs after
                // shield_arc synthesis below.
                toml::from_str::<crate::ship::config::ShipConfig>(&ship_toml_str).map_err(|e| {
                    serde::de::Error::custom(format!("ship_config parse failed: {e}"))
                })?
            } else {
                // No station/system/power_groups TOML at all but we do have
                // shield_arcs — synthesise a minimal ShipConfig so the
                // per-arc systems have a home. This path is used by NPC
                // ships that don't otherwise declare `[[system]]` blocks.
                crate::ship::config::ShipConfig {
                    stations: Vec::new(),
                    systems: Vec::new(),
                    power_groups: std::collections::HashMap::new(),
                    coordination_lag_secs: 2.0,
                }
            };

            // Synthesise `[[system]]` entries from `[[shield_arc]]` blocks.
            //
            // For player ships (has a `shields` station or a system with
            // kind="shields" in `systems`), each arc is owned by that
            // station with `ai_only = false`. For NPC ships (no shields
            // station and no shields system), arcs are ownerless AI-only
            // systems, matching how NPC phaser banks / power reactors work.
            // If a kind="shields" system exists, its station assignment
            // is used (allowing shields to live on e.g. Science).
            let shields_station_id = crate::messages::StationId("shields".into());
            let shields_system = ship_config
                .systems
                .iter()
                .find(|s| s.kind == crate::system_registry::SHIELDS_KIND);
            let has_shields_station = shields_system.is_some()
                || ship_config
                    .stations
                    .iter()
                    .any(|s| s.id == shields_station_id);
            let effective_shields_station = shields_system
                .and_then(|s| s.station.clone())
                .unwrap_or(shields_station_id);
            let ops_group = crate::messages::PowerGroupId("ops".into());
            let has_ops_group = ship_config.power_groups.contains_key(&ops_group);

            for arc in &config.shield_arcs {
                let sid =
                    crate::system_registry::shield_arc_system_id(&arc.id).ok_or_else(|| {
                        SerdeError::custom(format!("shield_arc id {:?} is empty", arc.id))
                    })?;
                let mut synthesised_config = toml::value::Table::new();
                synthesised_config.insert(
                    "center_deg".into(),
                    toml::Value::Float(arc.center_deg as f64),
                );
                synthesised_config
                    .insert("width_deg".into(), toml::Value::Float(arc.width_deg as f64));
                if let Some(max_hp) = arc.max_hp {
                    synthesised_config.insert("max_hp".into(), toml::Value::Integer(max_hp as i64));
                }
                if let Some(regen) = arc.regen_per_sec {
                    synthesised_config
                        .insert("regen_per_sec".into(), toml::Value::Float(regen as f64));
                }
                if let Some(offline) = arc.offline_duration {
                    synthesised_config.insert(
                        "offline_duration".into(),
                        toml::Value::Float(offline as f64),
                    );
                }

                ship_config
                    .systems
                    .push(crate::ship::config::SystemInstanceConfig {
                        id: sid,
                        kind: crate::system_registry::SHIELD_ARC_KIND.into(),
                        station: if has_shields_station {
                            Some(effective_shields_station.clone())
                        } else {
                            None
                        },
                        ai_only: !has_shields_station,
                        power_group: if has_ops_group {
                            Some(ops_group.clone())
                        } else {
                            None
                        },
                        marker: None,
                        config: Some(toml::Value::Table(synthesised_config)),
                    });
            }

            // Re-run validation after synthesis (catches duplicate SystemIds,
            // dangling rating refs, etc.).
            if !ship_config.systems.is_empty() {
                crate::ship::config::validate(&ship_config, &kinds).map_err(|e| {
                    serde::de::Error::custom(format!(
                        "ship_config revalidate after shield_arc synthesis failed: {e:?}"
                    ))
                })?;
            }

            config.ship_config = Some(ship_config);
        }

        // Validation: region entity with effects but no shape is an error.
        if let Some(ref effects) = config.effects {
            if !effects.is_empty() && config.shape.is_none() {
                return Err(SerdeError::custom(
                    "region entity has effects but no [shape] section",
                ));
            }
        }

        // Validation: a [radar_appearance] table must declare at least one
        // of icon/region_colour. An empty table is always an author mistake
        // (omit the whole section to mean "don't show on radar").
        if let Some(ref ra) = config.radar_appearance {
            if ra.icon.is_none() && ra.region_colour.is_none() {
                return Err(SerdeError::custom(
                    "[radar_appearance] must set icon and/or region_colour",
                ));
            }
        }

        // Clamp target_speed in every doctrine entry.
        if let Some(ref mut b) = config.behaviour {
            for d in &mut b.doctrine {
                d.target_speed = d.target_speed.clamp(0.0, 1.0);
            }
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]

    use super::*;
    use crate::entity_tags::EntityTag;

    #[test]
    fn all_sections_present_deserializes_to_some() {
        let toml_str = r##"
tags = ["gameplay", "combat", "primary"]

[hull]
hull_integrity = 100

[collider]
shape = "Ball"
radius = 2.0
length = 0.0

[appearance]
colour = "#ff0000"
size_min = 1.0
size_max = 3.0

[helm_console]
max_speed = 50.0
max_reverse_speed = 25.0
acceleration = 16.7
deceleration = 50.0
max_yaw_rate = 0.785

[helm_console.radar]
range = 50.0
shows = ["asteroid"]

[weapons_console]

[engineering_console]

[captain_console]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");

        assert_eq!(
            config.tags,
            vec![
                "gameplay".to_string(),
                "combat".to_string(),
                "primary".to_string()
            ]
        );

        assert!(config.hull.is_some());
        assert!((config.hull.as_ref().unwrap().hull_integrity - 100.0).abs() < 1e-6);

        assert!(config.collider.is_some());
        let c = config.collider.as_ref().unwrap();
        assert_eq!(c.shape, ColliderShape::Ball);
        assert_eq!(c.radius, 2.0);

        assert!(config.appearance.is_some());
        assert_eq!(config.appearance.as_ref().unwrap().colour, "#ff0000");

        assert!(config.helm_console.is_some());
        let h = config.helm_console.as_ref().unwrap();
        assert_eq!(h.max_speed, 50.0);
        assert_eq!(h.effective_radar_range(), 50.0);

        assert!(config.weapons_console.is_some());

        assert!(config.engineering_console.is_some());

        assert!(config.captain_console.is_some());
    }

    #[test]
    fn helm_console_engine_pfx_deserializes_optional_block() {
        let toml_str = r##"
[helm_console]
max_speed = 50.0

[helm_console.engine_pfx]
color = [0.2, 0.7, 1.0, 0.8]
markers = ["engine_port", "engine_starboard"]
trail_lifetime_secs = 0.45
trail_spawn_interval_secs = 0.04
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let pfx = config
            .helm_console
            .as_ref()
            .and_then(|helm| helm.engine_pfx.as_ref())
            .expect("engine_pfx block must parse");

        assert_eq!(pfx.color, Some([0.2, 0.7, 1.0, 0.8]));
        assert_eq!(
            pfx.markers,
            vec!["engine_port".to_string(), "engine_starboard".to_string()]
        );
        assert_eq!(pfx.trail_lifetime_secs, Some(0.45));
        assert_eq!(pfx.trail_spawn_interval_secs, Some(0.04));
    }

    #[test]
    fn helm_console_engine_pfx_fields_default_when_block_is_sparse() {
        let toml_str = r##"
[helm_console]
max_speed = 50.0

[helm_console.engine_pfx]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let pfx = config
            .helm_console
            .as_ref()
            .and_then(|helm| helm.engine_pfx.as_ref())
            .expect("engine_pfx block must parse");

        assert_eq!(pfx.color, None);
        assert!(pfx.markers.is_empty());
        assert_eq!(pfx.trail_lifetime_secs, None);
        assert_eq!(pfx.trail_spawn_interval_secs, None);
    }

    #[test]
    fn only_hull_and_tags_produces_none_for_console_fields() {
        let toml_str = r##"
tags = ["gameplay", "asteroid"]

[hull]
hull_integrity = 80
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");

        assert_eq!(
            config.tags,
            vec!["gameplay".to_string(), "asteroid".to_string()]
        );
        assert!(config.hull.is_some());
        assert!((config.hull.as_ref().unwrap().hull_integrity - 80.0).abs() < 1e-6);
        assert!(config.collider.is_none());
        assert!(config.appearance.is_none());
        assert!(config.helm_console.is_none());
        assert!(config.weapons_console.is_none());
        assert!(config.engineering_console.is_none());
        assert!(config.captain_console.is_none());
        assert!(
            config.radar_appearance.is_none(),
            "radar_appearance should default to None when not in TOML"
        );
    }

    #[test]
    fn malformed_field_returns_error() {
        let toml_str = r##"
[hull]
hull_integrity = "not_an_integer"
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn tags_field_deserializes_to_vec_string() {
        let toml_str = r##"
tags = ["foo", "bar", "baz", "quux"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(
            config.tags,
            vec![
                "foo".to_string(),
                "bar".to_string(),
                "baz".to_string(),
                "quux".to_string()
            ]
        );
    }

    #[test]
    fn collider_capsule_shape_round_trips() {
        let toml_str = r##"
[collider]
shape = "Capsule"
radius = 1.5
length = 6.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(
            config.collider.as_ref().unwrap().shape,
            ColliderShape::Capsule
        );
    }

    #[test]
    fn empty_toml_string_produces_all_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.tags.is_empty());
        assert!(config.hull.is_none());
        assert!(config.collider.is_none());
        assert!(config.appearance.is_none());
        assert!(config.helm_console.is_none());
        assert!(config.weapons_console.is_none());
        assert!(config.engineering_console.is_none());
        assert!(config.captain_console.is_none());
        assert!(
            config.radar_appearance.is_none(),
            "radar_appearance should default to None"
        );
    }

    #[test]
    fn helm_console_partial_fields_work() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(h.max_speed, 30.0);
        assert_eq!(h.max_reverse_speed, 0.0);
    }

    #[test]
    fn helm_console_radar_table_parses_into_nested_field() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.radar]
range = 750.0
shows = ["asteroid", "ship"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        let radar = h.radar.as_ref().expect("helm_console.radar must parse");
        assert_eq!(radar.range, 750.0);
        assert_eq!(h.effective_radar_range(), 750.0);
    }

    #[test]
    fn helm_console_effective_radar_range_zero_when_no_radar_table() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert!(h.radar.is_none());
        assert_eq!(h.effective_radar_range(), 0.0);
    }

    #[test]
    fn helm_console_boost_table_parses_when_present() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.boost]
multiplier = 3.0
steering_multiplier = 2.0
active_duration = 4.0
recharge_duration = 20.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        let boost = h.boost.as_ref().expect("helm_console.boost must parse");
        assert_eq!(boost.multiplier, 3.0);
        assert_eq!(boost.steering_multiplier, 2.0);
        assert_eq!(boost.active_duration, 4.0);
        assert_eq!(boost.recharge_duration, 20.0);
    }

    #[test]
    fn helm_console_boost_steering_multiplier_defaults_to_identity() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.boost]
multiplier = 3.0
active_duration = 4.0
recharge_duration = 20.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        let boost = h.boost.as_ref().expect("helm_console.boost must parse");
        assert_eq!(
            boost.steering_multiplier,
            crate::boost::BOOST_STEERING_MULTIPLIER
        );
    }

    #[test]
    fn helm_console_boost_none_when_table_absent() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert!(
            h.boost.is_none(),
            "missing boost table must disable the feature"
        );
    }

    #[test]
    fn weapons_console_beam_color_parses_rgba() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0
beam_color = [1.0, 0.5, 0.2, 0.9]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert_eq!(w.phaser_banks[0].beam_color, vec![1.0, 0.5, 0.2, 0.9]);
    }

    #[test]
    fn weapons_console_beam_color_defaults_to_empty_when_omitted() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert!(
            w.phaser_banks[0].beam_color.is_empty(),
            "beam_color should default to empty vec when omitted"
        );
    }

    // ── Power section tests ────────────────────────────────────────────────

    #[test]
    fn power_section_parses_capacity_rates_emergency_threshold() {
        let toml_str = r##"
[power]
capacity = 150.0
rates = [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]
emergency_threshold = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let p = config.power.expect("power must be Some");
        assert!((p.capacity - 150.0).abs() < 0.001);
        assert_eq!(p.rates, [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]);
        assert!((p.emergency_threshold - 30.0).abs() < 0.001);
    }

    #[test]
    fn power_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(
            config.power.is_none(),
            "power should be None when not specified"
        );
    }

    #[test]
    fn sensors_console_parses_with_long_range_radar() {
        let toml_str = r##"
tags = ["player", "ship"]

[sensors_console]
power_multipliers = [-0.5, 0.0, 0.25, 0.5]

[sensors_console.long_range_radar]
range = 200.0
shows = ["region", "asteroid_field", "asteroid", "ship"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sensors = config
            .sensors_console
            .expect("sensors_console must be Some");
        assert_eq!(sensors.power_multipliers, Some([-0.5, 0.0, 0.25, 0.5]));
        assert_eq!(sensors.long_range_radar.range, 200.0);
        assert!(sensors.long_range_radar.shows.contains(&EntityTag::Region));
        assert!(sensors
            .long_range_radar
            .shows
            .contains(&EntityTag::AsteroidField));
        assert!(sensors
            .long_range_radar
            .shows
            .contains(&EntityTag::Asteroid));
    }

    #[test]
    fn sensors_console_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.sensors_console.is_none());
    }

    #[test]
    fn helm_console_power_multipliers_parses() {
        let toml_str = r##"
[helm_console]
power_multipliers = [-0.8, 0.0, 0.4, 0.8]
max_speed = 50.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(h.power_multipliers, Some([-0.8, 0.0, 0.4, 0.8]));
    }

    #[test]
    fn weapons_console_power_multipliers_parses() {
        let toml_str = r##"
[weapons_console]
power_multipliers = [-0.3, 0.0, 0.15, 0.3]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert_eq!(w.power_multipliers, Some([-0.3, 0.0, 0.15, 0.3]));
    }

    #[test]
    fn power_multipliers_defaults_to_none_when_omitted() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert!(h.power_multipliers.is_none());
    }

    /// Every shipped entity template must parse under the strict
    /// (`deny_unknown_fields`) schema. Catches both schema drift in the code
    /// and typo'd keys in the TOMLs.
    #[test]
    fn all_shipped_entity_templates_parse_strictly() {
        let dir = std::path::Path::new("assets/entities");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).expect("assets/entities must exist") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("template must be readable");
            EntityConfig::from_toml(&text).unwrap_or_else(|e| {
                panic!(
                    "entity template {} failed strict parse: {e}",
                    path.display()
                )
            });
            checked += 1;
        }
        assert!(
            checked > 0,
            "no entity templates found in {}",
            dir.display()
        );
    }

    /// Unknown keys in an entity TOML must be rejected, not silently ignored.
    #[test]
    fn unknown_section_and_field_are_rejected() {
        assert!(EntityConfig::from_toml("[helm_consol]\nmax_speed = 1.0").is_err());
        assert!(EntityConfig::from_toml("[helm_console]\nmax_sped = 1.0").is_err());
    }

    // ── AsteroidField section tests ────────────────────────────────────────

    #[test]
    fn asteroid_field_section_parses_from_template() {
        let toml_str = r##"
tags = ["field", "main"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 150.0
despawn_distance = 250.0
asteroid_type_paths = ["assets/entities/asteroid_small.toml", "assets/entities/asteroid_large.toml"]
cosmetic_type_paths = ["assets/entities/asteroid_cosmetic.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!((field.inner_radius - 100.0).abs() < 1e-6);
        assert_eq!(field.asteroid_type_paths.len(), 2);
        assert_eq!(field.cosmetic_type_paths.len(), 1);
    }

    #[test]
    fn asteroid_field_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.asteroid_field.is_none());
    }

    #[test]
    fn asteroid_field_shape_defaults_to_none_when_omitted() {
        // Back-compat: TOMLs that pre-date the `shape` field must continue
        // to deserialise unchanged, with `shape = None`.
        let toml_str = r##"
[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!(field.shape.is_none());
    }

    #[test]
    fn asteroid_field_anchor_parses_as_optional_string() {
        // PRD #397 fix 5: `[asteroid_field] anchor = "name"` carries the
        // reference verbatim. The serde-skipped `anchor_offset` defaults
        // to `[0,0,0]` and is filled in at spawn time against the world's
        // anchor table.
        let toml_str = r##"
[asteroid_field]
shape = "torus"
anchor = "belt_origin"
inner_radius = 300.0
outer_radius = 350.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert_eq!(field.anchor.as_deref(), Some("belt_origin"));
        assert_eq!(
            field.anchor_offset,
            [0.0, 0.0, 0.0],
            "anchor_offset is serde-skipped and defaults to origin until spawn-time resolution"
        );
    }

    #[test]
    fn asteroid_field_anchor_omitted_defaults_to_none() {
        // Regression guard: existing TOML without an `anchor` key must keep
        // `anchor = None` and `anchor_offset = [0,0,0]` (legacy behaviour).
        let toml_str = r##"
[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!(field.anchor.is_none(), "missing anchor key → None");
        assert_eq!(field.anchor_offset, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn asteroid_field_shape_torus_parses() {
        // Schema: `shape = "torus"` as a sibling of `inner_radius`/`outer_radius`.
        let toml_str = r##"
[asteroid_field]
shape = "torus"
inner_radius = 300.0
outer_radius = 350.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert_eq!(
            field.shape,
            Some(crate::entity_config::AsteroidFieldShape::Torus)
        );
        assert!((field.inner_radius - 300.0).abs() < 1e-6);
        assert!((field.outer_radius - 350.0).abs() < 1e-6);
    }

    #[test]
    fn asteroid_field_shape_unknown_value_errors() {
        let toml_str = r##"
[asteroid_field]
shape = "donut"
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(
            result.is_err(),
            "unknown shape variant must be a parse error"
        );
    }

    // ── name / mesh.emissive / [[light]] tests (PRD: schema refactor slice 3) ──

    #[test]
    fn name_field_parses() {
        let toml_str = r#"name = "Sun""#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.name.as_deref(), Some("Sun"));
    }

    #[test]
    fn name_field_defaults_to_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.name.is_none());
    }

    #[test]
    fn mesh_emissive_field_parses() {
        let toml_str = r##"
[mesh]
shape = "sphere"
colour = [1.0, 0.8, 0.0]
radius = 50.0
emissive = 2.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let mesh = config.mesh.expect("mesh must be Some");
        assert_eq!(mesh.emissive, Some(2.0));
    }

    #[test]
    fn mesh_emissive_defaults_to_none() {
        let toml_str = r##"
[mesh]
shape = "sphere"
colour = [1.0, 1.0, 1.0]
radius = 1.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let mesh = config.mesh.expect("mesh must be Some");
        assert!(mesh.emissive.is_none());
    }

    #[test]
    fn light_array_parses_multiple_entries() {
        let toml_str = r##"
[[light]]
kind = "point"
colour = [1.0, 0.95, 0.85]
intensity = 150000.0
range = 5000.0

[[light]]
kind = "point"
colour = [0.5, 0.5, 1.0]
intensity = 1000.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.light.len(), 2);
        assert_eq!(config.light[0].kind, LightKind::Point);
        assert_eq!(config.light[0].colour, [1.0, 0.95, 0.85]);
        assert!((config.light[0].intensity - 150000.0).abs() < 1e-3);
        assert_eq!(config.light[0].range, Some(5000.0));
        assert_eq!(config.light[1].range, None);
    }

    #[test]
    fn light_defaults_to_empty_vec() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.light.is_empty());
    }

    #[test]
    fn light_directional_kind_parses() {
        let toml_str = r##"
[[light]]
kind = "directional"
colour = [1.0, 1.0, 1.0]
intensity = 10000.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.light.len(), 1);
        assert_eq!(config.light[0].kind, LightKind::Directional);
    }

    // ── Region shape tests ───────────────────────────────────────────────

    #[test]
    fn region_shape_sphere_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "sphere"
radius = 100.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(
            shape,
            crate::region_shape::RegionShape::Sphere { radius: 100.0 }
        );
    }

    #[test]
    fn region_shape_box_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "box"
half_extents = [50.0, 30.0, 40.0]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(
            shape,
            crate::region_shape::RegionShape::Box {
                half_extents: [50.0, 30.0, 40.0],
                yaw: 0.0
            }
        );
    }

    #[test]
    fn region_shape_torus_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "torus"
inner_radius = 50.0
outer_radius = 80.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(
            shape,
            crate::region_shape::RegionShape::Torus {
                inner_radius: 50.0,
                outer_radius: 80.0
            }
        );
    }

    #[test]
    fn region_shape_parses_with_effects() {
        let toml_str = r##"
tags = ["region", "nebula"]

[shape]
type = "sphere"
radius = 150.0

[effects]
[effects.comms_jammed]
[effects.sensor_blind]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.shape.is_some());
        let effects = config.effects.expect("effects must be Some");
        assert!(effects.comms_jammed.is_some());
        assert!(effects.sensor_blind.is_some());
    }

    #[test]
    fn region_effects_without_shape_returns_error() {
        let toml_str = r##"
tags = ["region"]

[effects]
[effects.comms_jammed]
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(
            result.is_err(),
            "region entity with effects but no shape should error"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("shape"),
            "error should mention missing shape: {err}"
        );
    }

    #[test]
    fn shape_alone_without_effects_is_valid() {
        let toml_str = r##"
tags = ["region"]

[shape]
type = "sphere"
radius = 100.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.shape.is_some());
        assert!(config.effects.is_none());
    }

    #[test]
    fn empty_toml_produces_no_shape_or_effects() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.shape.is_none());
        assert!(config.effects.is_none());
    }

    // ── Station hull tests (post-[station] removal; PRD slice 2) ──────────

    #[test]
    fn station_axiom_template_parses_hull_integrity() {
        let toml_str = include_str!("../../assets/entities/station_axiom.toml");
        let config = EntityConfig::from_toml(toml_str).expect("station_axiom.toml must parse");
        let hull = config.hull.as_ref().expect("must have [hull]");
        // (#474) Buffed from 200 to 800 for the combat-test scenario.
        assert!((hull.hull_integrity - 800.0).abs() < 1e-6);
    }

    #[test]
    fn station_outpost_template_parses_hull_integrity() {
        let toml_str = include_str!("../../assets/entities/station_outpost.toml");
        let config = EntityConfig::from_toml(toml_str).expect("station_outpost.toml must parse");
        let hull = config.hull.as_ref().expect("must have [hull]");
        assert!((hull.hull_integrity - 200.0).abs() < 1e-6);
    }

    #[test]
    fn station_research_outpost_template_parses_hull_integrity() {
        let toml_str = include_str!("../../assets/entities/station_research_outpost.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("station_research_outpost.toml must parse");
        let hull = config.hull.as_ref().expect("must have [hull]");
        assert!((hull.hull_integrity - 60.0).abs() < 1e-6);
    }

    #[test]
    fn all_sections_parsed_in_full_template() {
        let toml_str = r##"
tags = ["full"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005

[hull]
hull_integrity = 100
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(
            config.asteroid_field.is_some(),
            "asteroid_field should be Some"
        );
        assert!(config.hull.is_some(), "hull should be Some");
        assert_eq!(config.tags, vec!["full"]);
    }

    // ── SystemId-keyed hull entries (parent issue #516 sub-issue #616) ────────

    #[test]
    fn hull_system_hull_parses_from_toml() {
        let toml_str = r##"
[hull]
hull_integrity = 100

[[hull.system_hull]]
system_id = "phaser-fore"
display_name = "Phaser Bank (Fore)"
max_hp = 25.0
damaged_threshold_pct = 0.6
disabled_threshold_pct = 0.2
debuff_magnitude = 0.25
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let hull = config.hull.as_ref().expect("hull must parse");
        assert_eq!(hull.system_hull.len(), 1);
        let entry = &hull.system_hull[0];
        assert_eq!(
            entry.system_id,
            crate::messages::SystemId("phaser-fore".into())
        );
        assert_eq!(entry.display_name.as_deref(), Some("Phaser Bank (Fore)"));
        assert!((entry.max_hp - 25.0).abs() < 1e-6);
        assert!((entry.damaged_threshold_pct - 0.6).abs() < 1e-6);
        assert!((entry.disabled_threshold_pct - 0.2).abs() < 1e-6);
        assert!((entry.debuff_magnitude - 0.25).abs() < 1e-6);
    }

    #[test]
    fn hull_system_hull_defaults_when_absent() {
        // Legacy TOML without [[hull.system_hull]] must still parse; the new
        // field defaults to an empty Vec.
        let toml_str = r##"
[hull]
hull_integrity = 100
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let hull = config.hull.as_ref().expect("hull must parse");
        assert!(hull.system_hull.is_empty());
    }

    #[test]
    fn hull_system_hull_entry_optional_fields_default() {
        // Only the required fields (system_id, max_hp) are provided; every
        // other field has a serde default.
        let toml_str = r##"
[hull]
[[hull.system_hull]]
system_id = "helm"
max_hp = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let entry = &config.hull.as_ref().unwrap().system_hull[0];
        assert_eq!(entry.system_id, crate::messages::SystemId("helm".into()));
        assert!(entry.display_name.is_none());
        assert!((entry.damaged_threshold_pct - 0.75).abs() < 1e-6);
        assert!((entry.disabled_threshold_pct - 0.25).abs() < 1e-6);
        assert!((entry.debuff_magnitude - 0.15).abs() < 1e-6);
    }

    // ── Shipped template TOML files referenced by assets/worlds/default.toml ──
    //
    // These tests embed each template at compile time via include_str! so
    // the build fails if a referenced template is missing or malformed.

    #[test]
    fn empty_star_section_uses_defaults() {
        let config = EntityConfig::from_toml("[star]\n").expect("parse must succeed");
        let star = config.star.as_ref().expect("must parse [star]");
        assert!((star.radius - 40.0).abs() < 1e-6);
        assert_eq!(star.longitude_segments, 64);
        assert_eq!(star.latitude_segments, 32);
        assert_eq!(star.surface_colour, [1.0, 0.72, 0.12]);
        assert_eq!(star.hot_colour, [1.0, 0.96, 0.65]);
        assert_eq!(star.cell_colour, [0.95, 0.32, 0.04]);
        assert_eq!(star.halo_colour, [1.0, 0.78, 0.18]);
        assert!((star.halo_radius_multiplier - 2.4).abs() < 1e-6);
        assert!((star.animation_speed - 1.0).abs() < 1e-6);
    }

    #[test]
    fn star_section_overrides_defaults() {
        let toml_str = r#"
[star]
radius = 75.0
longitude_segments = 96
latitude_segments = 48
surface_colour = [0.9, 0.7, 0.2]
hot_colour = [1.0, 1.0, 0.8]
cell_colour = [0.8, 0.2, 0.1]
halo_colour = [1.0, 0.6, 0.1]
halo_radius_multiplier = 3.0
animation_speed = 0.5
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let star = config.star.as_ref().expect("must parse [star]");
        assert!((star.radius - 75.0).abs() < 1e-6);
        assert_eq!(star.longitude_segments, 96);
        assert_eq!(star.latitude_segments, 48);
        assert_eq!(star.surface_colour, [0.9, 0.7, 0.2]);
        assert_eq!(star.hot_colour, [1.0, 1.0, 0.8]);
        assert_eq!(star.cell_colour, [0.8, 0.2, 0.1]);
        assert_eq!(star.halo_colour, [1.0, 0.6, 0.1]);
        assert!((star.halo_radius_multiplier - 3.0).abs() < 1e-6);
        assert!((star.animation_speed - 0.5).abs() < 1e-6);
    }

    #[test]
    fn star_section_rejects_unknown_fields() {
        let result = EntityConfig::from_toml(
            r#"
[star]
radius = 40.0
surfase_colour = [1.0, 0.7, 0.1]
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn star_sun_template_parses_with_star_and_lights() {
        let toml_str = include_str!("../../assets/entities/star_sun.toml");
        let config = EntityConfig::from_toml(toml_str).expect("star_sun.toml must parse");
        assert_eq!(config.name.as_deref(), Some("Sun"));
        let star = config
            .star
            .as_ref()
            .expect("star_sun.toml must have [star]");
        assert!((star.radius - 50.0).abs() < 1e-6);
        assert!(config.mesh.is_none(), "star_sun.toml must not keep [mesh]");
        assert!(
            !config.light.is_empty(),
            "star_sun.toml must have at least one [[light]]"
        );
        assert_eq!(config.light[0].kind, LightKind::Directional);
        let collider = config
            .collider
            .as_ref()
            .expect("star_sun.toml must have [collider]");
        assert_eq!(collider.shape, ColliderShape::Ball);
        assert!((collider.radius - 50.0).abs() < 1e-6);
    }

    #[test]
    fn planet_earth_template_parses_with_mesh_and_collider() {
        let toml_str = include_str!("../../assets/entities/planet_earth.toml");
        let config = EntityConfig::from_toml(toml_str).expect("planet_earth.toml must parse");
        assert_eq!(config.name.as_deref(), Some("Earth"));
        assert!(config.mesh.is_some(), "planet_earth.toml must have [mesh]");
        let collider = config
            .collider
            .as_ref()
            .expect("planet_earth.toml must have [collider]");
        assert_eq!(collider.shape, ColliderShape::Ball);
        assert!((collider.radius - 20.0).abs() < 1e-6);
    }

    #[test]
    fn asteroid_field_main_template_parses_with_field_and_grid() {
        let toml_str = include_str!("../../assets/entities/asteroid_field_main.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("asteroid_field_main.toml must parse");
        let field = config
            .asteroid_field
            .as_ref()
            .expect("must have [asteroid_field]");
        field
            .grid
            .as_ref()
            .expect("must have [asteroid_field.grid]");
        assert_eq!(field.asteroid_type_paths.len(), 8);
        assert_eq!(field.cosmetic_type_paths.len(), 4);
    }

    // ── Faction field tests ────────────────────────────────────────────────

    #[test]
    fn faction_field_parses_from_entity_toml() {
        let toml_str = r#"
tags = ["ship"]
faction = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let faction = config.faction.expect("faction must be Some");
        assert_eq!(faction.to_string(), "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa");
    }

    #[test]
    fn faction_field_defaults_to_none_when_absent() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.faction.is_none());
    }

    #[test]
    fn battleship_toml_parses_with_federation_faction() {
        let toml_str = include_str!("../../assets/entities/alliance_battleship.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("alliance_battleship.toml must parse");
        let faction = config
            .faction
            .expect("alliance_battleship must declare a faction");
        // Must match the Federation UUID in assets/factions/federation.toml
        let fed_toml = include_str!("../../assets/factions/federation.toml");
        let fed = crate::faction::parse_faction_config(fed_toml).unwrap();
        assert_eq!(faction, fed.uuid, "battleship faction must be Federation");
    }

    // ── Behaviour block tests ─────────────────────────────────────────────

    #[test]
    fn behaviour_block_absent_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.behaviour.is_none());
    }

    #[test]
    fn entity_with_hull_and_behaviour_has_both_sections() {
        let toml_str = r##"
tags = ["npc"]

[hull]
hull_integrity = 50.0

[behaviour]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.hull.is_some());
        assert!(config.behaviour.is_some());
    }

    // ── DoctrineObjective tests ────────────────────────────────────────────

    #[test]
    fn behaviour_with_patrol_doctrine_parses() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "patrol-sector"
text = "Patrol the sector"
directive_kind = "Patrol"
directive_anchors = ["alpha", "beta"]
directive_loop = true
base_priority = 20.0
target_speed = 0.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.doctrine.len(), 1);
        let d = &behaviour.doctrine[0];
        assert_eq!(d.id, "patrol-sector");
        assert_eq!(d.directive_kind.as_deref(), Some("Patrol"));
        assert_eq!(d.directive_anchors, vec!["alpha", "beta"]);
        assert!(d.directive_loop);
        assert!((d.base_priority - 20.0).abs() < 1e-5);
        assert!((d.target_speed - 0.5).abs() < 1e-5);
    }

    #[test]
    fn behaviour_with_destroy_doctrine_parses() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
text = "Engage and destroy hostile ships"
directive_kind = "Destroy"
base_priority = 35.0
target_speed = 0.8
maintain_range = 25.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let d = &config.behaviour.unwrap().doctrine[0];
        assert_eq!(d.id, "destroy-hostiles");
        assert_eq!(d.directive_kind.as_deref(), Some("Destroy"));
        assert!((d.base_priority - 35.0).abs() < 1e-5);
        assert!((d.maintain_range - 25.0).abs() < 1e-5);
    }

    #[test]
    fn doctrine_target_speed_clamped_to_zero_when_negative() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "patrol"
text = "Patrol"
base_priority = 10.0
target_speed = -0.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let d = &config.behaviour.unwrap().doctrine[0];
        assert_eq!(d.target_speed, 0.0, "negative target_speed must clamp to 0");
    }

    #[test]
    fn doctrine_target_speed_clamped_to_one_when_above_one() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "pursue"
text = "Pursue"
base_priority = 10.0
target_speed = 1.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let d = &config.behaviour.unwrap().doctrine[0];
        assert_eq!(d.target_speed, 1.0, "target_speed > 1 must clamp to 1");
    }

    #[test]
    fn behaviour_doctrine_empty_by_default() {
        let toml_str = r##"
[behaviour]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert!(
            behaviour.doctrine.is_empty(),
            "doctrine array must default to empty"
        );
    }

    #[test]
    fn behaviour_multiple_doctrine_objectives_parse() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "patrol"
text = "Patrol"
directive_kind = "Patrol"
directive_anchors = ["wp1", "wp2"]
base_priority = 20.0

[[behaviour.doctrine]]
id = "destroy"
text = "Destroy"
directive_kind = "Destroy"
base_priority = 35.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.doctrine.len(), 2);
        assert_eq!(behaviour.doctrine[0].id, "patrol");
        assert_eq!(behaviour.doctrine[1].id, "destroy");
    }

    // ── pirate_raider.toml compile-time template tests ─────────────────────

    #[test]
    fn pirate_raider_template_parses_with_harrow_faction() {
        // (#472) `pirate_raider.toml` was re-factioned from Pirate to Harrow
        // so the player ship's auto-fire (Federation faction) engages it.
        // Filename kept as `pirate_raider.toml` to avoid cascading rename
        // across world TOMLs that reference it.
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        let faction = config
            .faction
            .expect("pirate_raider must declare a faction");
        assert_eq!(
            faction.to_string(),
            "cccccccc-3333-4333-8333-cccccccccccc",
            "pirate_raider faction must be Harrow (#472)"
        );
    }

    #[test]
    fn pirate_raider_template_has_hull() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        assert!(
            config.hull.is_some(),
            "pirate_raider must have a [hull] section"
        );
        let hull = config.hull.as_ref().unwrap();
        assert!(
            hull.hull_integrity > 0.0,
            "pirate_raider [hull] must have a positive hull_integrity value"
        );
    }

    #[test]
    fn pirate_raider_template_has_helm_and_weapons_console() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        assert!(
            config.helm_console.is_some(),
            "pirate_raider must have a [helm_console]"
        );
        assert!(
            config.weapons_console.is_some(),
            "pirate_raider must have a [weapons_console]"
        );
    }

    // ── Shield arc auto-synthesis tests (issue #514) ─────────────────────────

    #[test]
    fn shield_arc_toml_block_synthesises_system_instance() {
        // Minimal ship TOML with a single `[[shield_arc]]` block. The
        // parser must synthesise a matching `[[system]]` entry with
        // `kind = "shield_arc"` and `SystemId("shield-arc-<id>")`.
        let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 90
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.shield_arcs.len(), 1);
        assert_eq!(config.shield_arcs[0].id, "fore");

        let ship_config = config
            .ship_config
            .expect("shield_arc must synthesise a ship_config");
        assert_eq!(ship_config.systems.len(), 1);
        let sys = &ship_config.systems[0];
        assert_eq!(sys.id.0, "shield-arc-fore");
        assert_eq!(sys.kind, "shield_arc");
        // No `[shields]` station on this bare ship → arc is ai_only + ownerless.
        assert!(sys.ai_only, "ownerless arc must be ai_only");
        assert!(sys.station.is_none());
    }

    #[test]
    fn shield_arc_synthesises_with_shields_station_when_present() {
        // A ship that declares a `shields` station gets arcs owned by that
        // station with `ai_only = false`.
        let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 180

[[shield_arc]]
id = "aft"
label = "Aft"
center_deg = 180
width_deg = 180

[[station]]
id = "shields"
name = "Shields"
description = "Manage shield systems."
rank = "Ens."
short_code = "SHD"
console = "shields"

[[station.rating]]
name = "Std"
automated_systems = []
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let ship_config = config.ship_config.expect("ship_config present");
        assert_eq!(ship_config.systems.len(), 2);
        for sys in &ship_config.systems {
            assert!(
                !sys.ai_only,
                "with a shields station, arcs are player-controlled"
            );
            assert_eq!(
                sys.station,
                Some(crate::messages::StationId("shields".into()))
            );
        }
    }

    #[test]
    fn battleship_toml_produces_five_shield_arcs() {
        let toml_str = include_str!("../../assets/entities/alliance_battleship.toml");
        let config = EntityConfig::from_toml(toml_str).expect("alliance_battleship must parse");
        assert_eq!(
            config.shield_arcs.len(),
            5,
            "battleship has 5 arcs (fore, starboard, aft, port, omni)"
        );
        let ids: Vec<&str> = config.shield_arcs.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["fore", "starboard", "aft", "port", "omni"]);

        // Synthesised systems have the expected shape.
        let ship_config = config.ship_config.expect("ship_config present");
        let arc_systems: Vec<_> = ship_config
            .systems
            .iter()
            .filter(|s| s.kind == "shield_arc")
            .collect();
        assert_eq!(arc_systems.len(), 5);
        let sys_ids: Vec<&str> = arc_systems.iter().map(|s| s.id.0.as_str()).collect();
        assert!(sys_ids.contains(&"shield-arc-fore"));
        assert!(sys_ids.contains(&"shield-arc-port"));
        assert!(sys_ids.contains(&"shield-arc-aft"));
        assert!(sys_ids.contains(&"shield-arc-starboard"));
        assert!(sys_ids.contains(&"shield-arc-omni"));
        // Player ship has a shields station → arcs are player-controlled.
        for sys in &arc_systems {
            assert!(!sys.ai_only);
            assert_eq!(
                sys.station,
                Some(crate::messages::StationId("shields".into()))
            );
            assert_eq!(
                sys.power_group,
                Some(crate::messages::PowerGroupId("ops".into()))
            );
        }
    }

    #[test]
    fn npc_ship_with_single_shield_arc_produces_one_arc_system() {
        // Verify each NPC TOML produces exactly one arc system, ai_only,
        // ownerless (no `shields` station declared on NPCs).
        for (path, expected_max_hp) in [
            ("../../assets/entities/pirate_raider.toml", 15),
            ("../../assets/entities/pirate_raider_reinforcement.toml", 15),
            ("../../assets/entities/ship_harrow_patrol.toml", 60),
            ("../../assets/entities/ship_harrow_warhawk.toml", 120),
        ] {
            let toml_str = match path {
                "../../assets/entities/pirate_raider.toml" => {
                    include_str!("../../assets/entities/pirate_raider.toml")
                }
                "../../assets/entities/pirate_raider_reinforcement.toml" => {
                    include_str!("../../assets/entities/pirate_raider_reinforcement.toml")
                }
                "../../assets/entities/ship_harrow_patrol.toml" => {
                    include_str!("../../assets/entities/ship_harrow_patrol.toml")
                }
                "../../assets/entities/ship_harrow_warhawk.toml" => {
                    include_str!("../../assets/entities/ship_harrow_warhawk.toml")
                }
                _ => unreachable!(),
            };
            let config = EntityConfig::from_toml(toml_str)
                .unwrap_or_else(|e| panic!("{path} must parse: {e}"));
            assert_eq!(
                config.shield_arcs.len(),
                1,
                "{path} must declare exactly one shield arc"
            );
            let arc = &config.shield_arcs[0];
            assert_eq!(arc.id, "all", "{path} NPC arc id must be 'all'");
            assert_eq!(arc.max_hp, Some(expected_max_hp), "{path} arc max_hp");

            let ship_config = config
                .ship_config
                .unwrap_or_else(|| panic!("{path} must have ship_config after arc synthesis"));
            let arc_systems: Vec<_> = ship_config
                .systems
                .iter()
                .filter(|s| s.kind == "shield_arc")
                .collect();
            assert_eq!(arc_systems.len(), 1, "{path} exactly one arc system");
            let sys = arc_systems[0];
            assert_eq!(sys.id.0, "shield-arc-all", "{path} SystemId shape");
            assert!(sys.ai_only, "{path} NPC arc must be ai_only");
            assert!(sys.station.is_none(), "{path} NPC arc must be ownerless");
        }
    }

    #[test]
    fn shield_arc_with_hull_max_hp_captures_tier_config() {
        let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 90
hull_max_hp = 7
hull_damaged_threshold_pct = 0.60
hull_disabled_threshold_pct = 0.20
hull_debuff_magnitude = 0.30
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let arc = &config.shield_arcs[0];
        assert_eq!(arc.hull_max_hp, 7.0);
        assert!((arc.hull_damaged_threshold_pct - 0.60).abs() < 1e-6);
        assert!((arc.hull_disabled_threshold_pct - 0.20).abs() < 1e-6);
        assert!((arc.hull_debuff_magnitude - 0.30).abs() < 1e-6);
    }

    #[test]
    fn shield_arc_hull_thresholds_default_when_omitted() {
        let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 90
hull_max_hp = 6
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let arc = &config.shield_arcs[0];
        assert!((arc.hull_damaged_threshold_pct - 0.75).abs() < 1e-6);
        assert!((arc.hull_disabled_threshold_pct - 0.25).abs() < 1e-6);
        assert!((arc.hull_debuff_magnitude - 0.15).abs() < 1e-6);
    }

    #[test]
    fn pirate_raider_template_has_shields_block() {
        // (#474) Harrow Destroyer has a single-facing shield (#471).
        // (#514) Migrated to `[[shield_arc]]` block; `[shields_console]`
        // block was retired for NPCs.
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        assert_eq!(
            config.shield_arcs.len(),
            1,
            "pirate_raider must declare exactly one [[shield_arc]] block"
        );
        let arc = &config.shield_arcs[0];
        assert_eq!(arc.id, "all");
        assert_eq!(arc.max_hp, Some(15));
        assert!((arc.regen_per_sec.expect("regen") - 0.5).abs() < 1e-6);
    }

    #[test]
    fn pirate_raider_template_phaser_has_shield_pierce() {
        // (#474) Harrow weapons all have 0.1 pierce.
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        let wc = config.weapons_console.as_ref().unwrap();
        let bank = wc.phaser_banks.first().expect("must have a phaser bank");
        assert_eq!(bank.shield_pierce, Some(0.1));
    }

    #[test]
    fn ship_harrow_patrol_template_has_two_phaser_banks_and_shields() {
        // (#474) Cruiser gained weapons + shields.
        // (#514) Migrated to `[[shield_arc]]` block.
        let toml_str = include_str!("../../assets/entities/ship_harrow_patrol.toml");
        let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_patrol.toml must parse");
        let wc = config
            .weapons_console
            .as_ref()
            .expect("cruiser must have [weapons_console] (#474)");
        assert_eq!(
            wc.phaser_banks.len(),
            2,
            "cruiser must have port + starboard banks"
        );
        assert_eq!(
            config.shield_arcs.len(),
            1,
            "cruiser must declare one [[shield_arc]] block"
        );
        let arc = &config.shield_arcs[0];
        assert_eq!(arc.id, "all");
        assert_eq!(arc.max_hp, Some(60));
        assert!((arc.regen_per_sec.expect("regen") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ship_harrow_warhawk_template_has_full_behaviour_and_weapons() {
        // (#474) Battleship gained a full behaviour tree + weapons +
        // shields. Previously was a stub.
        let toml_str = include_str!("../../assets/entities/ship_harrow_warhawk.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("ship_harrow_warhawk.toml must parse");
        let wc = config
            .weapons_console
            .as_ref()
            .expect("battleship must have [weapons_console] (#474)");
        assert_eq!(wc.phaser_banks.len(), 2, "battleship must have 2 banks");
        let bank = &wc.phaser_banks[0];
        assert!((bank.beam_damage_per_sec - 12.0).abs() < 1e-6);
        assert!((bank.beam_range - 75.0).abs() < 1e-6);
        // (#514) Battleship migrated to `[[shield_arc]]` block.
        assert_eq!(
            config.shield_arcs.len(),
            1,
            "battleship must declare one [[shield_arc]] block"
        );
        let arc = &config.shield_arcs[0];
        assert_eq!(arc.id, "all");
        assert_eq!(arc.max_hp, Some(120));
        let behaviour = config.behaviour.as_ref().expect("must have [behaviour]");
        let directive_kinds: Vec<Option<&str>> = behaviour
            .doctrine
            .iter()
            .map(|d| d.directive_kind.as_deref())
            .collect();
        assert!(
            directive_kinds.contains(&Some("Patrol")),
            "battleship must have a Patrol doctrine (#572 doctrine-based AI)"
        );
        assert!(
            directive_kinds.contains(&Some("Destroy")),
            "battleship must have a Destroy doctrine (#572 doctrine-based AI)"
        );
    }

    #[test]
    fn station_axiom_template_has_explicit_ball_collider() {
        // (#474) Explicit collider for robust hit detection.
        let toml_str = include_str!("../../assets/entities/station_axiom.toml");
        let config = EntityConfig::from_toml(toml_str).expect("station_axiom.toml must parse");
        let collider = config
            .collider
            .as_ref()
            .expect("station_axiom must have explicit [collider] (#474)");
        assert_eq!(collider.shape, ColliderShape::Ball);
        assert!((collider.radius - 12.0).abs() < 1e-6);
    }

    #[test]
    fn pirate_raider_template_has_doctrine_objectives() {
        // (#572) FSM dissolved — pirate_raider now uses doctrine-based AI.
        // Expects a Patrol objective (sector sweep) and a higher-priority
        // Destroy objective (engage hostiles on sight).
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        let behaviour = config
            .behaviour
            .expect("pirate_raider must have a [behaviour] block");
        let ids: Vec<&str> = behaviour.doctrine.iter().map(|d| d.id.as_str()).collect();
        assert!(
            ids.contains(&"patrol-sector"),
            "must have patrol-sector doctrine"
        );
        assert!(
            ids.contains(&"destroy-hostiles"),
            "must have destroy-hostiles doctrine"
        );
        let destroy = behaviour
            .doctrine
            .iter()
            .find(|d| d.id == "destroy-hostiles")
            .unwrap();
        let patrol = behaviour
            .doctrine
            .iter()
            .find(|d| d.id == "patrol-sector")
            .unwrap();
        assert!(
            destroy.base_priority > patrol.base_priority,
            "destroy-hostiles must outscore patrol-sector"
        );
    }

    #[test]
    fn pirate_raider_doctrine_destroy_has_correct_directive_kind() {
        // (#572) FSM transitions dissolved — engagement logic now lives in the
        // utility scorer. Verify the destroy-hostiles objective carries the
        // Destroy directive kind so operate_weapons picks it up.
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        let destroy = behaviour
            .doctrine
            .iter()
            .find(|d| d.id == "destroy-hostiles")
            .expect("destroy-hostiles doctrine must be present");
        assert_eq!(
            destroy.directive_kind.as_deref(),
            Some("Destroy"),
            "destroy-hostiles must carry directive_kind = 'Destroy'"
        );
    }

    // ── [torpedoes] block tests ────────────────────────────────────────────

    #[test]
    fn torpedoes_block_full_round_trips() {
        let toml_str = r##"
[torpedoes]
count = 12
damage_hull = 60
damage_shields = 7
speed = 35.0
turn_rate_deg_per_sec = 90.0
lifespan = 25.0
load_time = 8.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes must be Some");
        assert_eq!(t.count, 12);
        assert_eq!(t.damage_hull, 60);
        assert_eq!(t.damage_shields, 7);
        assert_eq!(t.speed, 35.0);
        assert_eq!(t.turn_rate_deg_per_sec, 90.0);
        assert_eq!(t.lifespan, 25.0);
        assert_eq!(t.load_time, 8.0);
    }

    #[test]
    fn torpedoes_block_absent_yields_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.torpedoes.is_none());
    }

    #[test]
    fn torpedoes_block_partial_keeps_defaults_for_missing_fields() {
        let toml_str = r##"
[torpedoes]
count = 99
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes must be Some");
        assert_eq!(t.count, 99, "override applied");
        assert_eq!(t.damage_hull, 50, "default preserved");
        assert_eq!(t.damage_shields, 5, "default preserved");
        assert_eq!(t.speed, 15.0, "default preserved");
        assert_eq!(t.turn_rate_deg_per_sec, 45.0, "default preserved");
        assert_eq!(t.lifespan, 20.0, "default preserved");
        assert_eq!(t.load_time, 10.0, "default preserved");
    }

    #[test]
    fn torpedoes_to_runtime_converts_degrees_to_radians() {
        let mut t = TorpedoesConfig::default();
        t.turn_rate_deg_per_sec = 45.0;
        let rt = t.to_runtime();
        assert!(
            (rt.turn_rate - std::f32::consts::FRAC_PI_4).abs() < 1e-5,
            "45 deg/s should convert to PI/4 rad/s, got {}",
            rt.turn_rate
        );
        assert_eq!(rt.count, 10);
        assert_eq!(rt.damage_hull, 50);
        assert_eq!(rt.load_time, 10.0);
    }

    #[test]
    fn torpedoes_defaults_match_runtime_torpedo_config_default() {
        let toml_default = TorpedoesConfig::default().to_runtime();
        let runtime_default = crate::torpedo::TorpedoConfig::default();
        assert_eq!(toml_default.count, runtime_default.count);
        assert_eq!(toml_default.damage_hull, runtime_default.damage_hull);
        assert_eq!(toml_default.damage_shields, runtime_default.damage_shields);
        assert_eq!(toml_default.speed, runtime_default.speed);
        assert!((toml_default.turn_rate - runtime_default.turn_rate).abs() < 1e-5);
        assert_eq!(toml_default.lifespan, runtime_default.lifespan);
        assert_eq!(toml_default.load_time, runtime_default.load_time);
    }

    #[test]
    fn battleship_toml_torpedoes_block_parses_correctly() {
        // Verify the [torpedoes] block in alliance_battleship.toml parses
        // and produces the expected runtime values.
        let toml_str = include_str!("../../assets/entities/alliance_battleship.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("alliance_battleship.toml must parse");
        let t = config
            .torpedoes
            .expect("alliance_battleship must have [torpedoes]");
        let rt = t.to_runtime();
        // Values from alliance_battleship.toml [torpedoes] block
        assert_eq!(rt.count, 30, "battleship magazine size");
        assert_eq!(rt.damage_hull, 40);
        assert_eq!(rt.damage_shields, 4);
        assert_eq!(rt.speed, 15.0);
        assert!((rt.turn_rate - (45f32).to_radians()).abs() < 1e-5);
        assert_eq!(rt.lifespan, 20.0);
        assert_eq!(rt.load_time, 10.0);
    }

    // ── [repair] block tests ───────────────────────────────────────────────

    #[test]
    fn repair_block_full_round_trips() {
        let toml_str = r##"
[repair]
travel_duration_secs = 7.5
repair_rate_hp_per_sec = 1.25
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let r = config.repair.expect("repair must be Some");
        assert_eq!(r.travel_duration_secs, 7.5);
        assert_eq!(r.repair_rate_hp_per_sec, 1.25);
    }

    #[test]
    fn repair_block_absent_yields_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.repair.is_none());
    }

    #[test]
    fn repair_block_partial_keeps_defaults_for_missing_fields() {
        let toml_str = r##"
[repair]
travel_duration_secs = 9.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let r = config.repair.expect("repair must be Some");
        assert_eq!(r.travel_duration_secs, 9.0, "override applied");
        assert_eq!(r.repair_rate_hp_per_sec, 0.5, "default preserved");
    }

    #[test]
    fn repair_to_runtime_preserves_values() {
        let r = RepairConfig {
            travel_duration_secs: 3.0,
            repair_rate_hp_per_sec: 2.0,
            ..Default::default()
        };
        let rt = r.to_runtime();
        assert_eq!(rt.travel_duration, 3.0);
        assert_eq!(rt.repair_rate_hp_per_sec, 2.0);
    }

    #[test]
    fn repair_defaults_match_runtime_repair_timings_default() {
        let toml_default = RepairConfig::default().to_runtime();
        let runtime_default = crate::repair_teams::RepairTimings::default();
        assert_eq!(
            toml_default.travel_duration,
            runtime_default.travel_duration
        );
        assert_eq!(
            toml_default.repair_rate_hp_per_sec,
            runtime_default.repair_rate_hp_per_sec
        );
    }

    #[test]
    fn battleship_toml_repair_block_matches_runtime_default_values() {
        // Drift guard: if the [repair] block in alliance_battleship.toml ever diverges
        // from RepairTimings::default(), this test fails so the owner can
        // confirm the change is intentional. (The defaults themselves match
        // the historical hardcoded constants in `repair_teams.rs`.)
        let toml_str = include_str!("../../assets/entities/alliance_battleship.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("alliance_battleship.toml must parse");
        let r = config
            .repair
            .expect("alliance_battleship must have [repair]");
        let rt = r.to_runtime();
        let baseline = crate::repair_teams::RepairTimings::default();
        assert_eq!(
            rt.travel_duration, baseline.travel_duration,
            "travel duration drift"
        );
        assert_eq!(
            rt.repair_rate_hp_per_sec, baseline.repair_rate_hp_per_sec,
            "repair rate drift"
        );
    }

    // ── [shields_console.base] block tests ────────────────────────────────

    #[test]
    fn shields_console_base_block_full_round_trips() {
        let toml_str = r##"
[shields_console]

[shields_console.base]
num_facings = 6
max_hp = 200
regen_per_sec = 7.5
offline_duration = 12.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sc = config
            .shields_console
            .expect("shields_console must be Some");
        let base = sc.base.expect("base sub-block must be Some");
        assert_eq!(base.num_facings, 6);
        assert_eq!(base.max_hp, 200);
        assert_eq!(base.regen_per_sec, 7.5);
        assert_eq!(base.offline_duration, 12.0);
    }

    #[test]
    fn shields_console_without_base_subblock_yields_none() {
        // The flat focus fields parse fine; absent `[shields_console.base]`
        // must produce `base: None` so the runtime falls back to
        // `ShieldConfig::default()`.
        let toml_str = r##"
[shields_console]
focus_bonus_max_hp = 99
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sc = config
            .shields_console
            .expect("shields_console must be Some");
        assert!(
            sc.base.is_none(),
            "base sub-block must default to None when absent"
        );
        assert_eq!(sc.focus_bonus_max_hp, 99, "flat focus field still parses");
    }

    #[test]
    fn shields_base_block_partial_keeps_defaults_for_missing_fields() {
        let toml_str = r##"
[shields_console.base]
max_hp = 250
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let base = config
            .shields_console
            .expect("shields_console")
            .base
            .expect("base");
        assert_eq!(base.max_hp, 250, "override applied");
        assert_eq!(base.num_facings, 4, "default preserved");
        assert_eq!(base.regen_per_sec, 2.0, "default preserved");
        assert_eq!(base.offline_duration, 10.0, "default preserved");
    }

    #[test]
    fn shields_base_to_runtime_preserves_values() {
        let base = ShieldsBaseConfig {
            num_facings: 3,
            max_hp: 75,
            regen_per_sec: 2.5,
            offline_duration: 8.0,
        };
        let rt = base.to_runtime();
        assert_eq!(rt.num_facings, 3);
        assert_eq!(rt.max_hp, 75);
        assert_eq!(rt.regen_per_sec, 2.5);
        assert_eq!(rt.offline_duration, 8.0);
    }

    #[test]
    fn shields_base_defaults_match_runtime_shield_config_default() {
        let toml_default = ShieldsBaseConfig::default().to_runtime();
        let runtime_default = crate::shield::ShieldConfig::default();
        assert_eq!(toml_default.num_facings, runtime_default.num_facings);
        assert_eq!(toml_default.max_hp, runtime_default.max_hp);
        assert_eq!(toml_default.regen_per_sec, runtime_default.regen_per_sec);
        assert_eq!(
            toml_default.offline_duration,
            runtime_default.offline_duration
        );
    }

    #[test]
    fn battleship_toml_shields_base_block_parses_correctly() {
        // Verify the [shields_console.base] block in alliance_battleship.toml
        // parses and produces the expected runtime values.
        let toml_str = include_str!("../../assets/entities/alliance_battleship.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("alliance_battleship.toml must parse");
        let base = config
            .shields_console
            .expect("alliance_battleship must have [shields_console]")
            .base
            .expect("alliance_battleship must have [shields_console.base]");
        let rt = base.to_runtime();
        // Values from alliance_battleship.toml [shields_console.base] block
        assert_eq!(rt.max_hp, 140, "battleship shield facing max_hp");
        assert_eq!(rt.regen_per_sec, 3.5, "battleship shield regen");
        assert_eq!(rt.offline_duration, 10.0, "offline duration");
    }

    // ── PhaserCombatConfig (player phaser tuning) tests ───────────────────
    //
    // PhaserCombatConfig is built from the per-bank fields on
    // [[weapons_console.phaser_banks]]. All combat tuning is per-bank.

    #[test]
    fn phaser_combat_config_from_weapons_console_clones_banks() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0
beam_range = 99.0
beam_damage_per_sec = 12.0
beam_duration_secs = 4.0
cooldown_secs = 7.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        let combat = PhaserCombatConfig::from_weapons_console(&wc);
        assert_eq!(combat.banks.len(), 1);
        assert_eq!(combat.banks[0].beam_range, 99.0);
        assert_eq!(combat.banks[0].beam_damage_per_sec, 12.0);
        assert_eq!(combat.banks[0].beam_duration_secs, 4.0);
        assert_eq!(combat.banks[0].cooldown_secs, 7.5);
    }

    #[test]
    fn phaser_combat_config_default_has_empty_banks() {
        let combat = PhaserCombatConfig::default();
        assert!(combat.banks.is_empty());
    }

    // ── PhaserBankConfig / TorpedoTubeConfig schema tests (Phase A) ───────

    #[test]
    fn phaser_banks_array_parses_full_entries() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
beam_range = 35.0

[[weapons_console.phaser_banks]]
id = "starboard"
facing_deg = 90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.phaser_banks.len(), 2);
        assert_eq!(wc.phaser_banks[0].id, "port");
        assert_eq!(wc.phaser_banks[0].facing_deg, -90.0);
        assert_eq!(wc.phaser_banks[0].fire_arc_deg, 180.0);
        assert_eq!(wc.phaser_banks[0].auto_arc_deg, 120.0);
        assert_eq!(wc.phaser_banks[0].beam_range, 35.0);
        assert_eq!(wc.phaser_banks[1].id, "starboard");
        assert_eq!(
            wc.phaser_banks[1].beam_range, 0.0,
            "missing beam_range defaults to 0 (caller falls back to parent)"
        );
    }

    #[test]
    fn phaser_bank_shield_pierce_defaults_to_none_when_absent() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.phaser_banks[0].shield_pierce, None);
    }

    #[test]
    fn phaser_bank_shield_pierce_parses_when_present() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
shield_pierce = 0.6
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.phaser_banks[0].shield_pierce, Some(0.6));
    }

    #[test]
    fn torpedoes_shield_pierce_defaults_to_zero() {
        let toml_str = r##"
[torpedoes]
count = 5

[[torpedoes.tubes]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert_eq!(t.shield_pierce, 0.0);
        // Propagates into the runtime config that the in-flight torpedo
        // snapshots at launch.
        assert_eq!(t.to_runtime().shield_pierce, 0.0);
    }

    #[test]
    fn torpedoes_shield_pierce_parses_when_present() {
        let toml_str = r##"
[torpedoes]
count = 5
shield_pierce = 0.5

[[torpedoes.tubes]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert!((t.shield_pierce - 0.5).abs() < 1e-6);
        assert!((t.to_runtime().shield_pierce - 0.5).abs() < 1e-6);
    }

    #[test]
    fn torpedo_in_flight_snapshots_shield_pierce_at_launch() {
        // Wiring proof: changing the in-flight torpedo's snapshot mid-flight
        // doesn't affect future launches (it's a per-torpedo copy).
        use crate::torpedo::{TorpedoConfig, TorpedoSystem};
        use std::collections::HashMap;
        let mut cfg = TorpedoConfig::default();
        cfg.shield_pierce = 0.75;
        let tubes = vec![TorpedoTubeConfig {
            id: "fore".into(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            volley_max: 1,
        }];
        let mut sys = TorpedoSystem::from_configs(&tubes, cfg);
        assert!(sys.start_load("fore"));
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(sys.config.load_time, &targets, &mut || "test".into());
        sys.launch("fore", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert!((sys.in_flight[0].shield_pierce - 0.75).abs() < 1e-6);

        let det = sys.handle_collision_full("t1").unwrap();
        assert!((det.shield_pierce - 0.75).abs() < 1e-6);
    }

    #[test]
    fn phaser_banks_defaults_to_empty_vec_when_absent() {
        let toml_str = r##"
[weapons_console]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert!(
            wc.phaser_banks.is_empty(),
            "phaser_banks defaults to empty when [[phaser_banks]] absent"
        );
    }

    #[test]
    fn validate_phaser_banks_accepts_valid_list() {
        let banks = vec![
            PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 120.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
                ..Default::default()
            },
            PhaserBankConfig {
                id: "starboard".into(),
                facing_deg: 90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 120.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
                ..Default::default()
            },
        ];
        assert!(validate_phaser_banks(&banks).is_ok());
    }

    #[test]
    fn validate_phaser_banks_rejects_empty_list() {
        let err = validate_phaser_banks(&[]).unwrap_err();
        assert!(err.contains("empty"), "error mentions empty: {err}");
    }

    #[test]
    fn validate_phaser_banks_rejects_duplicate_ids() {
        let banks = vec![
            PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 90.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
                ..Default::default()
            },
            PhaserBankConfig {
                id: "port".into(),
                facing_deg: 90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 90.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
                ..Default::default()
            },
        ];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(err.contains("duplicate"), "error mentions duplicate: {err}");
        assert!(err.contains("port"));
    }

    #[test]
    fn validate_phaser_banks_rejects_auto_arc_greater_than_fire_arc() {
        let banks = vec![PhaserBankConfig {
            id: "port".into(),
            facing_deg: -90.0,
            fire_arc_deg: 90.0,
            auto_arc_deg: 180.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        }];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(
            err.contains("auto_arc_deg"),
            "error mentions auto arc: {err}"
        );
    }

    #[test]
    fn validate_phaser_banks_rejects_fire_arc_out_of_range() {
        let banks = vec![PhaserBankConfig {
            id: "port".into(),
            facing_deg: 0.0,
            fire_arc_deg: 400.0,
            auto_arc_deg: 90.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        }];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(
            err.contains("fire_arc_deg"),
            "error mentions fire arc: {err}"
        );

        let banks = vec![PhaserBankConfig {
            id: "port".into(),
            facing_deg: 0.0,
            fire_arc_deg: 0.0,
            auto_arc_deg: 0.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        }];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(err.contains("fire_arc_deg"), "zero arc rejected: {err}");
    }

    #[test]
    fn torpedo_tubes_array_parses_full_entries() {
        let toml_str = r##"
[torpedoes]
count = 10

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = -30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "fore_starboard"
facing_deg = 30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "aft"
facing_deg = 180.0
fire_arc_deg = 90.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert_eq!(t.tubes.len(), 3);
        assert_eq!(t.tubes[0].id, "fore_port");
        assert_eq!(t.tubes[0].facing_deg, -30.0);
        assert_eq!(t.tubes[0].fire_arc_deg, 90.0);
        assert_eq!(t.tubes[2].id, "aft");
        assert_eq!(t.tubes[2].facing_deg, 180.0);
    }

    #[test]
    fn torpedo_tubes_defaults_to_empty_vec_when_absent() {
        let toml_str = r##"
[torpedoes]
count = 10
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert!(
            t.tubes.is_empty(),
            "tubes defaults to empty when [[torpedoes.tubes]] absent"
        );
    }

    #[test]
    fn validate_torpedo_tubes_accepts_valid_list() {
        let tubes = vec![
            TorpedoTubeConfig {
                id: "fore_port".into(),
                facing_deg: -30.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                volley_max: 1,
            },
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 180.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                volley_max: 1,
            },
        ];
        assert!(validate_torpedo_tubes(&tubes).is_ok());
    }

    #[test]
    fn validate_torpedo_tubes_rejects_empty_list() {
        let err = validate_torpedo_tubes(&[]).unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_torpedo_tubes_rejects_duplicate_ids() {
        let tubes = vec![
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 180.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                volley_max: 1,
            },
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 0.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                volley_max: 1,
            },
        ];
        let err = validate_torpedo_tubes(&tubes).unwrap_err();
        assert!(err.contains("duplicate"));
        assert!(err.contains("aft"));
    }

    #[test]
    fn validate_torpedo_tubes_rejects_fire_arc_out_of_range() {
        let tubes = vec![TorpedoTubeConfig {
            id: "aft".into(),
            facing_deg: 180.0,
            fire_arc_deg: 0.0,
            load_time: None,
            marker: None,
            volley_max: 1,
        }];
        let err = validate_torpedo_tubes(&tubes).unwrap_err();
        assert!(err.contains("fire_arc_deg"));
    }

    #[test]
    fn comms_config_parses_range_from_toml() {
        let toml_str = r##"
[comms]
range = 8000.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let comms = config.comms.expect("comms section present");
        assert_eq!(comms.range, 8000.0);
    }

    #[test]
    fn comms_config_is_none_when_section_absent() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.comms.is_none(), "no [comms] → field is None");
    }

    // â"€â"€ LOD selection â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// Two levels: GLB near (< 50), sphere fallback beyond.
    fn two_level_lods() -> Vec<LodLevel> {
        vec![
            LodLevel {
                max_distance: Some(50.0),
                model: Some("assets/models/rock.glb".into()),
                ..Default::default()
            },
            LodLevel {
                max_distance: None,
                shape: Some(MeshShape::Sphere),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn select_lod_empty_returns_zero() {
        assert_eq!(select_lod(&[], 123.0, None), 0);
        assert_eq!(select_lod(&[], 0.0, Some(3)), 0);
    }

    #[test]
    fn select_lod_single_level_always_zero() {
        let levels = vec![LodLevel {
            max_distance: None,
            shape: Some(MeshShape::Sphere),
            ..Default::default()
        }];
        assert_eq!(select_lod(&levels, 0.0, None), 0);
        assert_eq!(select_lod(&levels, 9999.0, None), 0);
        assert_eq!(select_lod(&levels, 9999.0, Some(0)), 0);
    }

    #[test]
    fn select_lod_basic_band_selection() {
        let levels = two_level_lods();
        // Near band → level 0.
        assert_eq!(select_lod(&levels, 0.0, None), 0);
        assert_eq!(select_lod(&levels, 49.0, None), 0);
        // Far band → level 1.
        assert_eq!(select_lod(&levels, 60.0, None), 1);
        assert_eq!(select_lod(&levels, 100_000.0, None), 1);
    }

    #[test]
    fn select_lod_boundary_is_exclusive_upper() {
        let levels = two_level_lods();
        // Exactly at the boundary belongs to the far band (upper bound exclusive).
        assert_eq!(select_lod(&levels, 50.0, None), 1);
        // Just below stays near.
        assert_eq!(select_lod(&levels, 49.999, None), 0);
    }

    #[test]
    fn select_lod_hysteresis_holds_when_moving_outward() {
        let levels = two_level_lods();
        // Currently at near level 0; distance crept just past the boundary but
        // within the margin → hold at 0.
        assert_eq!(select_lod(&levels, 52.0, Some(0)), 0);
        assert_eq!(
            select_lod(&levels, 50.0 + LOD_HYSTERESIS_MARGIN, Some(0)),
            0,
            "exactly boundary + margin still holds (strict >)"
        );
        // Clear of the margin → switch outward to level 1.
        assert_eq!(
            select_lod(&levels, 50.0 + LOD_HYSTERESIS_MARGIN + 0.1, Some(0)),
            1
        );
    }

    #[test]
    fn select_lod_hysteresis_holds_when_moving_inward() {
        let levels = two_level_lods();
        // Currently at far level 1; distance dropped just below the boundary but
        // within the margin → hold at 1.
        assert_eq!(select_lod(&levels, 48.0, Some(1)), 1);
        assert_eq!(
            select_lod(&levels, 50.0 - LOD_HYSTERESIS_MARGIN, Some(1)),
            1,
            "exactly boundary - margin still holds (strict <)"
        );
        // Clear of the margin → switch inward to level 0.
        assert_eq!(
            select_lod(&levels, 50.0 - LOD_HYSTERESIS_MARGIN - 0.1, Some(1)),
            0
        );
    }

    #[test]
    fn select_lod_no_thrash_across_repeated_calls_at_boundary() {
        let levels = two_level_lods();
        // Sit right on the boundary and re-evaluate repeatedly: whatever level we
        // start at, we should stay there (no oscillation).
        let mut level = 0usize;
        for _ in 0..10 {
            level = select_lod(&levels, 50.0, Some(level));
        }
        assert_eq!(level, 0, "started near, stays near at the boundary");

        let mut level = 1usize;
        for _ in 0..10 {
            level = select_lod(&levels, 50.0, Some(level));
        }
        assert_eq!(level, 1, "started far, stays far at the boundary");
    }

    #[test]
    fn select_lod_three_levels_and_out_of_range() {
        let levels = vec![
            LodLevel {
                max_distance: Some(30.0),
                model: Some("a.glb".into()),
                ..Default::default()
            },
            LodLevel {
                max_distance: Some(80.0),
                shape: Some(MeshShape::Sphere),
                ..Default::default()
            },
            LodLevel {
                max_distance: None,
                shape: Some(MeshShape::Sphere),
                ..Default::default()
            },
        ];
        assert_eq!(select_lod(&levels, 10.0, None), 0);
        assert_eq!(select_lod(&levels, 50.0, None), 1);
        assert_eq!(select_lod(&levels, 500.0, None), 2);
        // Negative / zero distances clamp to the nearest band.
        assert_eq!(select_lod(&levels, -5.0, None), 0);
        // A stale current index beyond the list is clamped and re-resolved.
        assert_eq!(select_lod(&levels, 500.0, Some(99)), 2);
    }

    #[test]
    fn lod_level_parses_from_mesh_toml() {
        let toml_str = r##"
[mesh]
shape = "sphere"
colour = [0.5, 0.5, 0.5]
radius = 2.0

[[mesh.lod]]
max_distance = 50.0
model = "assets/models/rock.glb"
variant = "small"

[[mesh.lod]]
shape = "sphere"
radius = 2.0
colour = [0.5, 0.5, 0.5]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let mesh = config.mesh.expect("mesh section present");
        assert_eq!(mesh.lod.len(), 2);
        assert_eq!(mesh.lod[0].max_distance, Some(50.0));
        assert_eq!(mesh.lod[0].model.as_deref(), Some("assets/models/rock.glb"));
        assert_eq!(mesh.lod[0].variant.as_deref(), Some("small"));
        assert_eq!(mesh.lod[1].max_distance, None);
        assert_eq!(mesh.lod[1].shape, Some(MeshShape::Sphere));
    }
}

// ── Leaf scene-shape types moved from former entities/map_config.rs (PRD #341) ──
// These describe entity-template physical/visual properties consumed by
// EntityConfig (one-per-template) and by steroids::spawner. They are not
// world-tree concerns and so live alongside the entity-template schema rather
// than in world::config.

/// Global configuration block (currently used only for the deterministic seed
/// surfaced through WorldConfig).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    /// Global seed for deterministic generation.
    #[serde(default = "default_global_seed")]
    pub seed: u64,
    /// Display name shown in the lobby title bar.
    #[serde(default)]
    pub title: Option<String>,
    /// Short description shown below the title in the lobby.
    #[serde(default)]
    pub description: Option<String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            title: None,
            description: None,
        }
    }
}

fn default_global_seed() -> u64 {
    42
}

/// Configuration for the grid-based asteroid spawner.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GridConfig {
    pub resolution: f32,
    #[serde(default = "default_fill_gameplay")]
    pub fill_gameplay: f32,
    #[serde(default = "default_fill_cosmetic")]
    pub fill_cosmetic: f32,
    #[serde(default)]
    pub uniformity: f32,
    #[serde(default = "default_noise_freq")]
    pub noise_freq: f32,
    #[serde(default = "default_noise_octaves")]
    pub noise_octaves: u32,
    #[serde(default = "default_density_noise_freq")]
    pub density_noise_freq: f32,
    #[serde(default = "default_density_noise_octaves")]
    pub density_noise_octaves: u32,
    #[serde(default)]
    pub jitter: f32,
    #[serde(default)]
    pub cosmetic_y_offset: f32,
    #[serde(default = "default_gameplay_y_variance")]
    pub gameplay_y_variance: f32,
    #[serde(default = "default_spawn_cells")]
    pub spawn_cells: u32,
    #[serde(default = "default_despawn_cells")]
    pub despawn_cells: u32,
}

fn default_fill_gameplay() -> f32 {
    0.4
}
fn default_fill_cosmetic() -> f32 {
    0.15
}
fn default_noise_freq() -> f32 {
    0.02
}
fn default_noise_octaves() -> u32 {
    3
}
fn default_density_noise_freq() -> f32 {
    0.01
}
fn default_density_noise_octaves() -> u32 {
    2
}
fn default_gameplay_y_variance() -> f32 {
    0.5
}
fn default_spawn_cells() -> u32 {
    10
}
fn default_despawn_cells() -> u32 {
    12
}

/// Shape variant for an asteroid field.
///
/// When the TOML schema omits `shape`, the field defaults to the historical
/// behaviour (cell-centre distance check against `inner_radius`/`outer_radius`,
/// which produces a disc/annulus depending on whether `inner_radius` is zero).
///
/// `Torus` selects the explicit annulus eligibility test: a cell is admitted
/// if its XZ bounding box overlaps the annulus `[inner_radius, outer_radius]`
/// around the world origin. Cells whose bounding box lies fully inside
/// `inner_radius` or whose nearest corner is beyond `outer_radius` are
/// rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsteroidFieldShape {
    /// Annulus / ring-belt eligibility based on `inner_radius` and
    /// `outer_radius`. Cells whose bounding box overlaps the annulus
    /// are admitted.
    Torus,
}

/// Configuration for an asteroid field.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AsteroidFieldConfig {
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub density: f32,
    #[serde(default = "default_spawn_distance")]
    pub spawn_distance: f32,
    #[serde(default = "default_despawn_distance")]
    pub despawn_distance: f32,
    #[serde(default)]
    pub asteroid_type_paths: Vec<String>,
    #[serde(default)]
    pub cosmetic_type_paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub grid: Option<GridConfig>,
    /// Fraction of asteroid-collision damage that bypasses shields and
    /// applies directly to the hull. Default `0.0` — asteroid impacts are
    /// fully absorbed by the facing shield quadrant (matching pre-#414
    /// behaviour). Clamped to `[0.0, 1.0]` at apply time.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Optional shape variant. When `None`, the historical cell-centre
    /// distance eligibility test is used. When `Some(Torus)`, cells are
    /// admitted iff their XZ bounding box overlaps the annulus
    /// `[inner_radius, outer_radius]`. See [`AsteroidFieldShape`].
    #[serde(default)]
    pub shape: Option<AsteroidFieldShape>,
    /// Optional world anchor name. When present, the field's eligibility
    /// region and per-asteroid spawn positions are translated so the
    /// `[inner_radius, outer_radius]` annulus is centred on the named
    /// anchor's world position instead of the world origin. The anchor
    /// is resolved against `WorldConfig.anchors` at spawn time; the
    /// resolved offset is written into `anchor_offset`. If the anchor
    /// name is not present in the world's anchor table, the field falls
    /// back to the world origin (`anchor_offset = [0, 0, 0]`) and a
    /// warning is logged.
    #[serde(default)]
    pub anchor: Option<String>,
    /// Resolved world-space offset for the anchor referenced by `anchor`.
    /// Defaults to `[0, 0, 0]` (world origin) when no anchor is set or
    /// the named anchor could not be resolved. Not serialised — derived
    /// at spawn time.
    #[serde(skip)]
    pub anchor_offset: [f32; 3],
    /// Maximum random rotation applied to each spawned asteroid, in degrees.
    /// `[x, y, z]` → ±x° pitch, ±y° roll, ±z° yaw. When `None` (default),
    /// asteroids spawn with no rotation. Set e.g. `[30, 30, 180]` for mild
    /// tilt with full spin freedom.
    #[serde(default)]
    pub random_rotation: Option<[f32; 3]>,
}

fn default_spawn_distance() -> f32 {
    150.0
}
fn default_despawn_distance() -> f32 {
    250.0
}
