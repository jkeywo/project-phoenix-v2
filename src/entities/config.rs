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
    ///
    /// Defaulted (issue #838): a doctrine directive authored *inline* — as a
    /// world `spawn_entity` override rather than in a ship template — routinely
    /// omits display prose (every wave in `combat_test.toml` does). Before this
    /// was `#[serde(default)]`, such an override reparsed with a "missing field
    /// `text`" error inside `dispatch_spawn_entity`'s round-trip (step 2b), and
    /// the failure was silently swallowed — discarding the *entire* override,
    /// faction and behaviour alike, and leaving the raw template. A hull with no
    /// template `[behaviour]` (e.g. `alliance_destroyer`) was left inert and,
    /// worse, kept its template faction, so a world-spawned "hostile" was
    /// neither hostile nor armed. An empty `text` is already a tested value in
    /// `score_doctrine_pool`; the captain panel simply shows no prose for it.
    #[serde(default)]
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
    /// Named target for `Destroy` directives. Resolved by `ai_target_selection`
    /// (tier 1) onto the ship's `TacticalRadarSelection`, which is what the Helm and the
    /// firing systems then read.
    #[serde(default)]
    pub directive_target: Option<String>,
    /// Named anchor for `Reach` and `Retreat` directives.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// fallthrough, used when no local Helm-relevant objective resolves but
    /// Navigation has set a waypoint the Helm is cleared to follow.
    /// Defaults to [`crate::ai::NAV_HANDOFF_SPEED`] when absent.
    #[serde(default = "default_nav_handoff_speed")]
    pub nav_handoff_speed: f32,
    /// Distance (world units) within which a docking intent switches from
    /// normal objective approach to the close-quarters docking manoeuvre
    /// (controlled reverse / lateral translation). Issue #742.
    /// Defaults to [`crate::ai::DOCKING_ENGAGE_DISTANCE`] when absent.
    #[serde(default = "default_docking_engage_distance")]
    pub docking_engage_distance: f32,
    /// Speed fraction `[0, 1]` capping the low-speed reverse / lateral
    /// translation of a docking close manoeuvre. Issue #742.
    /// Defaults to [`crate::ai::DOCKING_APPROACH_SPEED`] when absent.
    #[serde(default = "default_docking_approach_speed")]
    pub docking_approach_speed: f32,
    /// Authored ignore-smaller rule (issue #743): the shared hazard assessment
    /// skips a hazard whose `size_rating` is below this ship's own scaled by
    /// this ratio. `0.0` (the default) disables the rule so every dangerous
    /// hazard is assessed; `1.0` ignores any hazard strictly smaller than self.
    /// Defaults to [`crate::ai::HAZARD_IGNORE_SIZE_RATIO`] when absent.
    #[serde(default = "default_hazard_ignore_size_ratio")]
    pub hazard_ignore_size_ratio: f32,
    /// Authored lateral-thrust sensitivity to the shared hazard surface (issue
    /// #743): the multiplier the fine lateral-thrust actuator applies to the
    /// hazard assessment's starboard repulsion before clamping to `[-1, 1]`.
    /// Defaults to [`crate::ai::LATERAL_HAZARD_SENSITIVITY`] when absent.
    #[serde(default = "default_lateral_hazard_sensitivity")]
    pub lateral_hazard_sensitivity: f32,
    /// Authored vertical-thrust sensitivity to the shared hazard surface (issue
    /// #744): the multiplier the vertical-thrust actuator applies to the shared
    /// assessment's moving-hazard threat before clamping to `[0, 1]`.
    /// Defaults to [`crate::ai::VERTICAL_HAZARD_SENSITIVITY`] when absent.
    #[serde(default = "default_vertical_hazard_sensitivity")]
    pub vertical_hazard_sensitivity: f32,
    /// Authored hazard-urgency threshold at or above which an imminent collision
    /// may TEMPORARILY override the ship's desired facing toward the escape
    /// direction (issue #780, AC4). Ordinary avoidance below this only bends
    /// travel and never touches facing. Defaults to
    /// [`crate::ai::IMMINENT_COLLISION_FACING_THRESHOLD`] (`1.0` — effectively
    /// off) when absent; the override is stateless and self-clears the tick
    /// urgency drops back under the threshold.
    #[serde(default = "default_imminent_collision_facing_threshold")]
    pub imminent_collision_facing_threshold: f32,
    // `retreat_hull_threshold` lived here until issue #702. It fed a synthetic
    // hull-triggered Retreat that could never win (0..1 score against doctrine's
    // tens) and always steered to world origin (its anchor was empty and the
    // `home_position` it fell back on was never seeded in production). Retreat
    // is now authored as ordinary doctrine, which is strictly more expressive —
    // a real anchor, a real priority, and any `zero_gates` combination rather
    // than one hardwired hull ramp:
    //
    //     [[behaviour.doctrine]]
    //     id               = "retreat-when-hurt"
    //     directive_kind   = "Retreat"
    //     directive_anchor = "pirate_haven"
    //     base_priority    = 100.0
    //     zero_gates       = [{ condition = "hull_below", threshold = 0.3 }]
}

/// Hand-written so `BehaviourConfig::default()` agrees with what serde
/// produces for a `[behaviour]` block that omits every optional field.
/// A derived `Default` would silently zero the tuning fields — a
/// `waypoint_arrival_radius` of `0.0` means "arrival requires landing on the
/// anchor exactly", which no NPC ever does.
impl Default for BehaviourConfig {
    fn default() -> Self {
        Self {
            doctrine: Vec::new(),
            waypoint_arrival_radius: default_waypoint_arrival_radius(),
            avoidance_buffer: default_avoidance_buffer(),
            avoidance_look_ahead_secs: default_avoidance_look_ahead_secs(),
            nav_handoff_speed: default_nav_handoff_speed(),
            docking_engage_distance: default_docking_engage_distance(),
            docking_approach_speed: default_docking_approach_speed(),
            hazard_ignore_size_ratio: default_hazard_ignore_size_ratio(),
            lateral_hazard_sensitivity: default_lateral_hazard_sensitivity(),
            vertical_hazard_sensitivity: default_vertical_hazard_sensitivity(),
            imminent_collision_facing_threshold: default_imminent_collision_facing_threshold(),
        }
    }
}

fn default_imminent_collision_facing_threshold() -> f32 {
    crate::ai::IMMINENT_COLLISION_FACING_THRESHOLD
}

fn default_waypoint_arrival_radius() -> f32 {
    crate::ai::WAYPOINT_ARRIVAL_RADIUS
}

fn default_avoidance_buffer() -> f32 {
    crate::ai::AVOIDANCE_BUFFER
}

fn default_nav_handoff_speed() -> f32 {
    crate::ai::NAV_HANDOFF_SPEED
}

fn default_avoidance_look_ahead_secs() -> f32 {
    crate::ai::AVOIDANCE_LOOK_AHEAD_SECS
}

fn default_docking_engage_distance() -> f32 {
    crate::ai::DOCKING_ENGAGE_DISTANCE
}

fn default_docking_approach_speed() -> f32 {
    crate::ai::DOCKING_APPROACH_SPEED
}

fn default_hazard_ignore_size_ratio() -> f32 {
    crate::ai::HAZARD_IGNORE_SIZE_RATIO
}

fn default_lateral_hazard_sensitivity() -> f32 {
    crate::ai::LATERAL_HAZARD_SENSITIVITY
}

fn default_vertical_hazard_sensitivity() -> f32 {
    crate::ai::VERTICAL_HAZARD_SENSITIVITY
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

fn default_planet_radius() -> f32 {
    20.0
}

fn default_planet_emissive_strength() -> f32 {
    1.0
}

fn default_planet_emissive_night_only() -> bool {
    true
}

fn default_planet_cloud_scale() -> f32 {
    1.03
}

fn default_planet_atmosphere_strength() -> f32 {
    1.0
}

fn default_planet_longitude_segments() -> u32 {
    128
}

fn default_planet_latitude_segments() -> u32 {
    64
}

/// Textured planet visual definition (`[planet]` section).
///
/// Renders as a UV sphere with a custom shader sampling equirectangular
/// texture maps: day/night lighting relative to the star, optional
/// nightside-gated emissive (city lights / nightglow), an optional
/// alpha-blended cloud/smog/ash shell on a slightly larger sphere, and an
/// optional fresnel atmosphere rim glow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetConfig {
    #[serde(default = "default_planet_radius")]
    pub radius: f32,
    #[serde(default = "default_planet_longitude_segments")]
    pub longitude_segments: u32,
    #[serde(default = "default_planet_latitude_segments")]
    pub latitude_segments: u32,
    /// Core surface texture set (`[planet.surface]`). Required.
    pub surface: PlanetSurfaceConfig,
    /// Optional cloud/smog/ash shell (`[planet.clouds]`).
    #[serde(default)]
    pub clouds: Option<PlanetCloudsConfig>,
    /// Optional atmosphere rim glow (`[planet.atmosphere]`).
    #[serde(default)]
    pub atmosphere: Option<PlanetAtmosphereConfig>,
}

/// Core surface texture maps for a `[planet]`. Paths are TOML-style
/// (`assets/...`-prefixed) like `MeshConfig.model`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetSurfaceConfig {
    /// Base colour map (sRGB). Required.
    pub albedo: String,
    /// Tangent-space normal map (linear).
    #[serde(default)]
    pub normal: Option<String>,
    /// Grayscale roughness map (linear).
    #[serde(default)]
    pub roughness: Option<String>,
    /// Emissive colour map (sRGB): city lights, nightglow, lava glow.
    #[serde(default)]
    pub emissive_colour: Option<String>,
    /// Grayscale emissive mask (linear). When absent the emissive colour map
    /// is used unmasked (maps that are black where unlit need no mask).
    #[serde(default)]
    pub emissive_mask: Option<String>,
    /// Gate emission to the night side (city lights). `false` for emission
    /// that is visible on the day side too (lava).
    #[serde(default = "default_planet_emissive_night_only")]
    pub emissive_night_only: bool,
    #[serde(default = "default_planet_emissive_strength")]
    pub emissive_strength: f32,
}

/// Cloud/smog/ash shell rendered on a second, slightly larger sphere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetCloudsConfig {
    /// Cloud colour map (sRGB). Required.
    pub albedo: String,
    /// Grayscale opacity map (linear). When absent the albedo luminance is
    /// used as opacity.
    #[serde(default)]
    pub opacity: Option<String>,
    /// Cloud normal map — accepted for authoring completeness but unused by
    /// the core shader.
    #[serde(default)]
    pub normal: Option<String>,
    /// Shell radius as a multiple of the planet radius.
    #[serde(default = "default_planet_cloud_scale")]
    pub scale: f32,
    /// Longitudinal drift in UV wraps per second. 0 = static.
    #[serde(default)]
    pub drift_speed: f32,
}

/// Fresnel rim atmosphere glow parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetAtmosphereConfig {
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub colour: [f32; 3],
    #[serde(default = "default_planet_atmosphere_strength")]
    pub strength: f32,
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
    /// Inline stateless AI policy for the **Engines** (longitudinal thrust)
    /// fine system, from `[helm_console.engines_ai]` (issue #779). Absent ⇒ the
    /// canonical [`default_engines_ai_config`] (unconditional actuate) is
    /// synthesised at spawn. Drives the `longitudinal` channel with the
    /// `actuate_desired_travel` mode verb.
    #[serde(default)]
    pub engines_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Steering** (yaw) fine system, from
    /// `[helm_console.steering_ai]` (issue #779). Absent ⇒ the canonical
    /// [`default_steering_ai_config`] is synthesised at spawn. Drives the `yaw`
    /// channel with the `actuate_desired_facing` mode verb.
    #[serde(default)]
    pub steering_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Lateral Thrust** fine system, from
    /// `[helm_console.lateral_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_lateral_ai_config`] (unconditional actuate) is synthesised at
    /// spawn. Drives the `lateral` channel with the `actuate_lateral_thrust`
    /// mode verb.
    #[serde(default)]
    pub lateral_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Vertical Thrust** fine system, from
    /// `[helm_console.vertical_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_vertical_ai_config`] (unconditional actuate) is synthesised at
    /// spawn. Drives the `vertical` channel with the `actuate_vertical_thrust`
    /// mode verb.
    #[serde(default)]
    pub vertical_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Impulse** fine system, from
    /// `[helm_console.impulse_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_impulse_ai_config`] (unconditional permit) is synthesised at
    /// spawn. Drives the `impulse` channel with the `engage_impulse` mode verb.
    #[serde(default)]
    pub impulse_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Boost** fine system, from
    /// `[helm_console.boost_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_boost_ai_config`] (explicit idle — no AI boost) is synthesised
    /// at spawn. Drives the `boost` channel with the `engage_boost` mode verb.
    #[serde(default)]
    pub boost_ai: Option<FineSystemAiConfigToml>,
}

/// What vertical movement capability the ship has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VerticalMovementMode {
    /// No vertical movement — planar-only flight (current default).
    #[default]
    Planar,
    /// AI-only bounded vertical motion for collision avoidance.
    Bounded,
    /// Full 3D six-degree-of-freedom flight.
    Full3D,
}

/// Impulse capability tuning loaded from `[helm_capability.impulse]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpulseCapabilityConfig {
    /// Steering multiplier applied while impulse is active.
    /// 0.0 = no steering, 0.1 = harsh but possible, 1.0 = full steering.
    #[serde(default = "default_impulse_steering_multiplier")]
    pub steering_multiplier: f32,
}

fn default_impulse_steering_multiplier() -> f32 {
    0.1
}

impl Default for ImpulseCapabilityConfig {
    fn default() -> Self {
        Self {
            steering_multiplier: default_impulse_steering_multiplier(),
        }
    }
}

/// Optional helm capability declaration for an entity (`[helm_capability]`).
///
/// When absent, the ship has no special helm capability and operates at the
/// default planar mode with full steering during impulse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmCapabilityConfig {
    /// Vertical movement mode. Defaults to `Planar`.
    #[serde(default)]
    pub vertical_movement_mode: VerticalMovementMode,
    /// Maximum vertical offset (world units) a `Bounded` craft may climb away
    /// from its cruise plane while dodging moving hazards (issue #744). Ignored
    /// for `Planar` (no vertical motion) and `Full3D` (unbounded).
    /// Defaults to [`crate::ai::MAX_VERTICAL_OFFSET`] when absent.
    #[serde(default = "default_max_vertical_offset")]
    pub max_vertical_offset: f32,
    /// Gradual return-to-cruise gain for a `Bounded` craft once avoidance
    /// urgency falls (issue #744): the vertical actuator eases the ship back to
    /// its cruise plane at `-y * vertical_return_rate` rather than snapping.
    /// Defaults to [`crate::ai::VERTICAL_RETURN_RATE`] when absent.
    #[serde(default = "default_vertical_return_rate")]
    pub vertical_return_rate: f32,
    /// Impulse capability tuning.
    #[serde(default)]
    pub impulse: ImpulseCapabilityConfig,
}

fn default_max_vertical_offset() -> f32 {
    crate::ai::MAX_VERTICAL_OFFSET
}

fn default_vertical_return_rate() -> f32 {
    crate::ai::VERTICAL_RETURN_RATE
}

/// Hand-written so `HelmCapabilityConfig::default()` matches what serde produces
/// for a `[helm_capability]` block that omits every optional field — a derived
/// `Default` would zero the vertical tunables instead of reading their authored
/// constant defaults.
impl Default for HelmCapabilityConfig {
    fn default() -> Self {
        Self {
            vertical_movement_mode: VerticalMovementMode::default(),
            max_vertical_offset: default_max_vertical_offset(),
            vertical_return_rate: default_vertical_return_rate(),
            impulse: ImpulseCapabilityConfig::default(),
        }
    }
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
    /// Inline stateless AI policy for this bank's open-fire decision
    /// (issue #781). When authored it is validated at content load and drives
    /// `ai_phaser_auto_fire`'s per-bank fire gate; when absent the canonical
    /// [`default_phaser_bank_ai_config`] (unconditional fire) is synthesised at
    /// spawn so baseline auto-fire is preserved. An explicit `idle = true` is the
    /// per-bank opt-out (AC1).
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
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
    /// Optional rig-marker name linking this bank to a mount point. In the
    /// single-barrel (backward-compat) case this is the sole projectile origin.
    #[serde(default)]
    pub marker: Option<String>,
    /// Authored barrel-marker names (issue #765). Each entry is a rig-marker
    /// name; a barrel-index pattern step addresses these by position. When
    /// empty the bank has one implicit barrel = `marker` (unchanged behaviour).
    #[serde(default)]
    pub barrels: Vec<String>,
    /// Timed multi-barrel firing pattern (issue #765). A step fires its listed
    /// barrel indices simultaneously at `offset_secs`; successive steps at
    /// increasing offsets alternate. When empty the bank fires the uniform
    /// `volley_count` volley from the single implicit barrel (unchanged).
    #[serde(default)]
    pub pattern: crate::weapons::pattern::BarrelPattern,
    /// Maximum range in world units. Projectile lifespan is computed per-bank
    /// as `range / projectile_speed`. Use `default_blaster_range` (35.0) when
    /// absent from TOML.
    #[serde(default = "default_blaster_range")]
    pub range: f32,
    /// Inline stateless AI policy for this bank's open-fire decision
    /// (issue #781). When authored it is validated at content load and drives
    /// `tick_blaster_auto_fire`'s per-bank fire gate; when absent the canonical
    /// [`default_blaster_bank_ai_config`] (unconditional fire) is synthesised at
    /// spawn so baseline auto-fire is preserved. An explicit `idle = true` is the
    /// per-bank opt-out (AC1).
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
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
            barrels: self.barrels.clone(),
            pattern: self.pattern.clone(),
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
    /// callers fall back to the ship-centre launch origin. In the single-barrel
    /// (backward-compat) case this is the sole launch origin.
    #[serde(default)]
    pub marker: Option<String>,
    /// Authored barrel-marker names (issue #766). Each entry is a rig-marker
    /// name; a barrel-index pattern step addresses these by position. When
    /// empty the tube has one implicit barrel = `marker` (unchanged behaviour).
    /// Reuses the exact schema blasters wired in issue #765.
    #[serde(default)]
    pub barrels: Vec<String>,
    /// Timed multi-barrel firing pattern (issue #766). A step lists barrel
    /// indices; successive steps at increasing offsets order the barrels a
    /// volley's rounds leave from. The pattern governs only WHICH barrel each
    /// launched round leaves from and in what order — never how many rounds
    /// exist: the magazine, `loaded_count`, and the burst cadence remain the
    /// sole authority over the torpedo count. When empty the tube launches
    /// from the single implicit barrel exactly as before.
    #[serde(default)]
    pub pattern: crate::weapons::pattern::BarrelPattern,
    /// Maximum number of torpedoes that can be loaded into this tube at once
    /// (volley capacity). Default `1` preserves existing single-shot
    /// behaviour. Values greater than 1 allow the tube to queue multiple
    /// torpedoes and fire them as a rapid burst.
    #[serde(default = "default_tube_volley_max")]
    pub volley_max: u32,
    /// How many rounds an AI-operated crew keeps loaded in this tube.
    ///
    /// The AI has no console to poke, so it issues the same
    /// `SetTorpedoVolleyTarget` command a human operator's console sends
    /// (see `console_ai::server::ai_torpedo_load`) and this is the count it
    /// asks for. Falls back to `[torpedoes] ai_volley_target`, then to
    /// [`Self::volley_max`] — a designer who says nothing gets "the AI keeps
    /// the tube as full as it can", which is the sane default for a hull that
    /// authored tubes at all. Clamped to `volley_max` at runtime.
    /// `Some(0)` disables AI loading for this tube.
    #[serde(default)]
    pub ai_target_count: Option<u32>,
    /// Inline stateless AI policy for this tube's load + launch decisions
    /// (issue #782). When authored it is validated at content load and drives
    /// `ai_torpedo_load`'s per-tube load gate and `ai_torpedo_auto_fire`'s
    /// per-tube launch gate; when absent the canonical
    /// [`default_torpedo_tube_ai_config`] (unconditional load + launch) is
    /// synthesised at spawn so baseline behaviour is preserved. An explicit
    /// `idle = true` is the per-tube opt-out (AC1).
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
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

/// Validate a `[[weapons_console.blaster_banks]]` list parsed from TOML
/// (issue #765).
///
/// An empty list is accepted — most hulls carry no blasters. Rejects:
///   - duplicate `id` values,
///   - `fire_arc_deg` outside `(0, 360]`,
///   - a barrel pattern that fires no barrels in a step, references a barrel
///     index beyond the declared barrel count, uses a negative offset, or is
///     omitted while more than one barrel is declared (see
///     [`crate::weapons::pattern::validate_barrel_pattern`]).
///
/// The barrel count is the authored `barrels.len()`, or `1` for the implicit
/// single-barrel (backward-compat) bank.
pub fn validate_blaster_banks(banks: &[BlasterBankConfig]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for b in banks {
        if !seen.insert(b.id.as_str()) {
            return Err(format!("duplicate blaster bank id '{}'", b.id));
        }
        if !(b.fire_arc_deg > 0.0 && b.fire_arc_deg <= 360.0) {
            return Err(format!(
                "blaster bank '{}' has fire_arc_deg={} outside (0, 360]",
                b.id, b.fire_arc_deg
            ));
        }
        let barrel_count = if b.barrels.is_empty() {
            1
        } else {
            b.barrels.len()
        };
        crate::weapons::pattern::validate_barrel_pattern(
            &format!("blaster bank '{}'", b.id),
            barrel_count,
            &b.pattern,
        )?;
    }
    Ok(())
}

/// Validate a `[[torpedoes.tubes]]` list parsed from TOML.
///
/// Rejects: empty list, duplicate `id`, `fire_arc_deg` outside `(0, 360]`, and
/// (issue #766) a barrel pattern that fires no barrels in a step, references a
/// barrel index beyond the declared barrel count, uses a negative offset, or is
/// omitted while more than one barrel is declared (see
/// [`crate::weapons::pattern::validate_barrel_pattern`]).
///
/// The barrel count is the authored `barrels.len()`, or `1` for the implicit
/// single-barrel (backward-compat) tube.
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
        let barrel_count = if t.barrels.is_empty() {
            1
        } else {
            t.barrels.len()
        };
        crate::weapons::pattern::validate_barrel_pattern(
            &format!("torpedo tube '{}'", t.id),
            barrel_count,
            &t.pattern,
        )?;
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
    /// Inline per-system target selector (issue #777). Loaded from
    /// `[weapons_console.selector]`; absent ⇒ the canonical
    /// [`default_tactical_target_selector_config`] is synthesised at spawn.
    /// Mirrors [`SensorsConsoleConfig::selector`] — the Tactical host ranks its
    /// own candidates independently and remains the sole writer of the
    /// authoritative `TacticalRadarSelection`.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
    /// Explicit Tactical-radar idle declaration (issue #781, AC6). When `true`
    /// the radar takes NO AI target selection — `ai_target_selection` clears any
    /// stale lock and skips the ship — even when a tactical fine system is
    /// AI-operated. This is the explicit AI-or-idle opt-out that distinguishes
    /// "the radar deliberately makes no AI selection" from "no selector authored
    /// → default selector". Defaults to `false` (radar runs its selector as
    /// before), so baseline behaviour is preserved.
    #[serde(default)]
    pub selector_idle: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringConsoleConfig {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CaptainConsoleConfig {
    /// Inline stateless AI policy for the Captain's Red Alert fine system
    /// (`[captain_console.ai]`, issue #775). When present it is validated at
    /// content load and drives `operate_captain_ai`; when absent the canonical
    /// [`default_captain_ai_config`] policy is synthesised at spawn.
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

/// Config block for the Comms CONSOLE's AI (issue #786), loaded from
/// `[comms_console]`.
///
/// Deliberately separate from the top-level `[comms]` section: that one is the
/// per-ENTITY comms RANGE (`CommsConfig`), present on stations and NPCs that are
/// merely hailable, and has nothing to do with who operates the console. The AI
/// policy belongs to the console, next to `[captain_console.ai]` and
/// `[sensors_console.selector]`.
///
/// Comms is the FIRST system to author BOTH fine-system AI machines: a #776
/// `selector` (WHO to hail — a variable candidate set keyed by real contact
/// UUIDs) and a #775 channel/verb `ai` policy (HOW to answer an open dialogue —
/// a fixed, index-addressed response list). See [`COMMS_RESPOND_CHANNEL`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CommsConsoleConfig {
    /// Inline per-system target selector for hail target ranking (issue #786).
    /// Loaded from `[comms_console.selector]`; absent ⇒ the canonical
    /// [`default_comms_target_selector_config`] is synthesised at spawn.
    /// Validated in [`EntityConfig::from_toml`] against
    /// [`COMMS_SELECTOR_SOURCES`].
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
    /// Inline stateless AI policy for the Comms dialogue-response fine system
    /// (issue #786). Loaded from `[comms_console.ai]`; absent ⇒ the canonical
    /// [`default_comms_response_ai_config`] is synthesised at spawn (baseline
    /// preservation). Validated against [`COMMS_RESPOND_CHANNELS`] /
    /// [`COMMS_RESPOND_VERBS`].
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

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

// ── Inline stateless fine-system AI policy (issue #775) ───────────────────────

/// Parse-time fallback for the Captain Red Alert combat window, in seconds.
///
/// Sanctioned by AGENTS.md rule #11 as a parse-default only: it seeds the
/// synthesised default Captain policy for ships that do not author
/// `[captain_console.ai]`. Authors override it via the named
/// `combat_window_secs` parameter, so no gameplay value is pinned in Rust.
pub const DEFAULT_CAPTAIN_COMBAT_WINDOW_SECS: f32 = 10.0;

// ── Parse-time defaults for the synthesised Power allocation policy (#784) ────
//
// Sanctioned by AGENTS.md rule #11 as parse-defaults + vocabulary only: they
// seed the synthesised [`default_power_ai_config`] for ships that do not author
// `[power.ai_policy]`, reproducing the retired stateful engine's baseline. A
// ship authoring the block overrides all of these with its own `param` reserves
// and per-rule `level` payloads, so no gameplay value is pinned in Rust.

/// Forward-thrust level (0.0–1.0) above which the default helm-elevation rule
/// fires. Matches the retired `movement_thrust_threshold` default.
pub const DEFAULT_POWER_THRUST_THRESHOLD: f32 = 0.7;
/// Minimum battery reserve (0–100) for the default helm-elevation rule. Matches
/// the retired `movement_battery_engage_min_pct` default.
pub const DEFAULT_POWER_HELM_RESERVE: f32 = 50.0;
/// Minimum battery reserve (0–100) for the default weapons-elevation rule.
/// Matches the retired `red_alert_battery_engage_min_pct` default.
pub const DEFAULT_POWER_WEAPONS_RESERVE: f32 = 10.0;
/// Absolute allocation level the default elevate rules raise their group to.
/// The canonical groups seed at level 2, so this reproduces the retired `+1`
/// nudge (2 → 3) as an absolute target.
pub const DEFAULT_POWER_ELEVATED_LEVEL: u8 = 3;
/// Absolute allocation level the default baseline (fallback) rules hold their
/// group at — the canonical seeded level, so a calm ship's baseline emit is a
/// no-op the host skips.
pub const DEFAULT_POWER_BASELINE_LEVEL: u8 = 2;

/// The `red_alert` output channel: the one channel the Captain policy drives.
pub const CAPTAIN_RED_ALERT_CHANNEL: &str = "red_alert";
/// The `set_red_alert` verb: the one typed verb the Captain policy emits.
pub const CAPTAIN_SET_RED_ALERT_VERB: &str = "set_red_alert";

// ── Helm fine-system AI policy channels/verbs (issue #779) ────────────────────
//
// Engines and Steering are the first *continuous* fine actuators to move onto
// the data-authored #775 policy spine. Each drives a single output channel with
// a single value-less **mode** verb: the verb decides *whether* to actuate this
// tick, while the continuous thrust/yaw magnitude stays sourced from the shared
// `DesiredMotion` planner fact (AGENTS.md rule #11 — no geometry pinned here).

/// The `longitudinal` output channel: the Engines fine system's thrust axis.
pub const HELM_LONGITUDINAL_CHANNEL: &str = "longitudinal";
/// The `actuate_desired_travel` verb: the Engines mode verb. Its presence tells
/// the host to emit `SetThrust` with the scalar decoded from the planner's
/// `desired_velocity_local`; its absence ("hold") emits nothing.
pub const HELM_ACTUATE_DESIRED_TRAVEL_VERB: &str = "actuate_desired_travel";

/// The `yaw` output channel: the Steering fine system's turn axis.
pub const HELM_YAW_CHANNEL: &str = "yaw";
/// The `actuate_desired_facing` verb: the Steering mode verb. Its presence tells
/// the host to emit `SetSteering` with the scalar decoded from the planner's
/// `desired_facing_local`; its absence ("hold") emits nothing.
pub const HELM_ACTUATE_DESIRED_FACING_VERB: &str = "actuate_desired_facing";
/// The `hold_committed_heading` verb: the Steering fine system's SECOND mode
/// verb (issue #883). Its presence tells the host to fly the heading frozen in
/// this system's own `memory(escape_heading_rad)` at the last committed
/// transition, rather than re-solving the facing against a moving target.
/// Distinct from a "hold" (no rule fires), which holds the last steering
/// COMMAND and would keep a non-zero yaw turning for ever.
pub const HELM_HOLD_COMMITTED_HEADING_VERB: &str = "hold_committed_heading";

/// The verbs a Steering (`yaw`) policy may emit (issues #779, #883).
pub const HELM_STEERING_VERBS: &[&str] = &[
    HELM_ACTUATE_DESIRED_FACING_VERB,
    HELM_HOLD_COMMITTED_HEADING_VERB,
];

// ── Helm secondary fine-actuator AI policy channels/verbs (issue #780) ────────
//
// Lateral thrust, bounded vertical thrust, impulse, and boost move onto the same
// #775 policy spine. Each drives a single output channel with a single value-less
// MODE verb: the verb decides *whether* to actuate this tick, while the
// continuous magnitude / engage-vs-cancel decision stays sourced host-side from
// geometry, capability, and the shared hazard assessment (AGENTS.md rule #11 —
// no gameplay scalar pinned in the verb).

/// The `lateral` output channel: the Lateral Thrust fine system's dodge axis.
pub const HELM_LATERAL_CHANNEL: &str = "lateral";
/// The `actuate_lateral_thrust` verb: the Lateral Thrust mode verb. Its presence
/// lets the host emit `LateralThrustInput` with the magnitude from the shared
/// hazard surface (or docking translation); its absence ("hold") emits nothing.
pub const HELM_ACTUATE_LATERAL_THRUST_VERB: &str = "actuate_lateral_thrust";

/// The `vertical` output channel: the Vertical Thrust fine system's climb axis.
pub const HELM_VERTICAL_CHANNEL: &str = "vertical";
/// The `actuate_vertical_thrust` verb: the Vertical Thrust mode verb. Its
/// presence lets the host emit `VerticalThrustInput` with the climb/return
/// magnitude gated on the authored `VerticalMovementMode`; its absence emits
/// nothing.
pub const HELM_ACTUATE_VERTICAL_THRUST_VERB: &str = "actuate_vertical_thrust";

/// The `impulse` output channel: the Impulse fine system's engage/cancel axis.
pub const HELM_IMPULSE_CHANNEL: &str = "impulse";
/// The `engage_impulse` verb: the Impulse mode verb. Its presence permits the
/// impulse manoeuvre this tick; the host still applies the authored doctrine
/// `use_impulse` and the `decide_impulse` geometry. Its absence ("hold") emits
/// nothing.
pub const HELM_ENGAGE_IMPULSE_VERB: &str = "engage_impulse";

/// The `boost` output channel: the Boost fine system's engage axis.
pub const HELM_BOOST_CHANNEL: &str = "boost";
/// The `engage_boost` verb: the Boost mode verb. Its presence drives the ship's
/// boost active via the same admitted `SetBoost` a human uses; its absence
/// ("hold"/idle) leaves boost as it is.
pub const HELM_ENGAGE_BOOST_VERB: &str = "engage_boost";

// ── Weapon-bank fine-system AI policy channels/verbs (issue #781) ─────────────
//
// Each AI-capable phaser and blaster bank drives a single output channel with a
// single value-less ACTION verb: the verb decides *whether* to open fire this
// tick, while the target (the ship's authoritative combat lock), the firing
// bank, and the beam frequency all come from the host context — never the verb
// (AGENTS.md rule #11 — no fire thresholds/ranges/arcs/cooldowns pinned here;
// those stay TOML on the bank configs). Each bank enforces availability,
// cooldown, range, arc, and target validity host-side before resolving its
// policy, so the runtime only reports *whether the authored behaviour permits*
// firing an already-ready bank.

/// The `phaser_fire` output channel: a phaser bank's open-fire axis.
pub const PHASER_FIRE_CHANNEL: &str = "phaser_fire";
/// The `fire_phaser` verb: the phaser-bank fire verb. Its presence tells the
/// host to emit the same admitted `FirePhaser` a human does; its absence
/// ("hold"/idle) holds this bank's fire.
pub const PHASER_FIRE_VERB: &str = "fire_phaser";

/// The registered output channels a phaser bank policy may drive (issue #781).
pub const PHASER_BANK_CHANNELS: &[&str] = &[PHASER_FIRE_CHANNEL];
/// The registered verbs a phaser bank policy may emit (issue #781).
pub const PHASER_BANK_VERBS: &[&str] = &[PHASER_FIRE_VERB];

/// The `blaster_fire` output channel: a blaster bank's open-fire axis.
pub const BLASTER_FIRE_CHANNEL: &str = "blaster_fire";
/// The `fire_blaster` verb: the blaster-bank fire verb. Its presence tells the
/// host to emit the same admitted `ChargeBlasterStart` a human does; its absence
/// ("hold"/idle) holds this bank's volley.
pub const BLASTER_FIRE_VERB: &str = "fire_blaster";

/// The registered output channels a blaster bank policy may drive (issue #781).
pub const BLASTER_BANK_CHANNELS: &[&str] = &[BLASTER_FIRE_CHANNEL];
/// The registered verbs a blaster bank policy may emit (issue #781).
pub const BLASTER_BANK_VERBS: &[&str] = &[BLASTER_FIRE_VERB];

// ── Torpedo tube + magazine fine-system AI policy channels/verbs (issue #782) ─
//
// A torpedo tube is a two-stage pipeline owned by two fine systems: the TUBE
// decides whether to LOAD (reserve a round from the shared magazine) and whether
// to LAUNCH (fire an already-loaded round), while the shared MAGAZINE arbitrates
// whether to GRANT a pending reservation. Every verb is value-less: the tube, its
// authored volley target, the ship's authoritative combat lock, and all
// thresholds stay TOML/host-side, never in the verb (AGENTS.md rule #11). The
// host enforces loaded state, magazine availability, target validity, range, and
// arc before resolving these policies, so the runtime only reports *whether the
// authored behaviour permits* the load/launch/grant of an already-ready stage.

/// The `torpedo_load` output channel: a tube's load-a-round axis.
pub const TORPEDO_LOAD_CHANNEL: &str = "torpedo_load";
/// The `load_torpedo` verb. Its presence tells the host to emit the same admitted
/// `SetTorpedoVolleyTarget` a Tactical player does; its absence ("hold"/idle)
/// leaves the tube's volley target where it is.
pub const TORPEDO_LOAD_VERB: &str = "load_torpedo";

/// The `torpedo_launch` output channel: a tube's launch-a-loaded-round axis.
pub const TORPEDO_LAUNCH_CHANNEL: &str = "torpedo_launch";
/// The `launch_torpedo` verb. Its presence tells the host to emit the same
/// admitted `FireTorpedo` a human does; its absence ("hold"/idle) holds fire.
pub const TORPEDO_LAUNCH_VERB: &str = "launch_torpedo";

/// The registered output channels a torpedo tube policy may drive (issue #782).
pub const TORPEDO_TUBE_CHANNELS: &[&str] = &[TORPEDO_LOAD_CHANNEL, TORPEDO_LAUNCH_CHANNEL];
/// The registered verbs a torpedo tube policy may emit (issue #782).
pub const TORPEDO_TUBE_VERBS: &[&str] = &[TORPEDO_LOAD_VERB, TORPEDO_LAUNCH_VERB];

/// The `torpedo_magazine_grant` output channel: the shared magazine's
/// grant-a-claim axis, resolved inside the single magazine consumer.
pub const TORPEDO_MAGAZINE_CHANNEL: &str = "torpedo_magazine_grant";
/// The `grant_torpedo_round` verb. Its presence permits a pending
/// `ClaimTorpedoRound` reservation to proceed; its absence ("hold"/idle) refuses
/// the claim without touching the magazine counter.
pub const TORPEDO_MAGAZINE_GRANT_VERB: &str = "grant_torpedo_round";

/// The registered output channels a torpedo magazine policy may drive (#782).
pub const TORPEDO_MAGAZINE_CHANNELS: &[&str] = &[TORPEDO_MAGAZINE_CHANNEL];
/// The registered verbs a torpedo magazine policy may emit (issue #782).
pub const TORPEDO_MAGAZINE_VERBS: &[&str] = &[TORPEDO_MAGAZINE_GRANT_VERB];

// ── Shields focus fine-system AI policy channel/verb (issue #783) ─────────────
//
// The Shields fine system focuses ONE of the ship's four arcs at a time. The
// #783 conversion keeps the retained arc-ranking kernel (`tick_shield_focus_ai`:
// damage-concentration primary, health-imbalance fallback) as the 4-way argmax
// and lifts only the AUTHORED windows/thresholds and the gate (whether to act)
// into an inline stateless policy. The `focus_shield_arc` verb is value-less —
// which arc wins is the kernel's call from the host context, never the verb
// (AGENTS.md rule #11: the concentration %, windows, and health ratio are policy
// `param`s, not literals). This is the channel/verb model of #775/#779–#782, not
// the #776 selector: shield arcs are a fixed 4-set of in-ship indices, not UUID
// entities, so there is nothing for a candidate-source selector to union.

/// The `shield_focus` output channel: the Shields fine system's focus-an-arc axis.
pub const SHIELD_FOCUS_CHANNEL: &str = "shield_focus";
/// The `focus_shield_arc` verb. Its presence tells the host to run the retained
/// arc-ranking kernel and emit the same admitted `SetShieldArcFocus` a human
/// Shields operator does; its absence ("hold"/idle) leaves the focus where it is.
pub const SHIELD_FOCUS_VERB: &str = "focus_shield_arc";

/// The registered output channels a Shields focus policy may drive (issue #783).
pub const SHIELD_FOCUS_CHANNELS: &[&str] = &[SHIELD_FOCUS_CHANNEL];
/// The registered verbs a Shields focus policy may emit (issue #783).
pub const SHIELD_FOCUS_VERBS: &[&str] = &[SHIELD_FOCUS_VERB];

/// Authored policy-parameter name: the maximum recent-damage window (seconds)
/// the kernel measures concentration over. Read host-side from the Shields focus
/// policy `param` map (issue #783); the kernel's arg equivalent of the retained
/// typed `ShieldsAiConfigToml::damage_window_secs` knob.
pub const SHIELD_FOCUS_DAMAGE_WINDOW_PARAM: &str = "damage_window_secs";
/// Authored policy-parameter name: the minimum window (seconds) floor.
pub const SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM: &str = "min_damage_window_secs";
/// Authored policy-parameter name: the concentration threshold (0–100).
pub const SHIELD_FOCUS_DAMAGE_PCT_PARAM: &str = "damage_pct_threshold";
/// Authored policy-parameter name: the health-imbalance fallback ratio (0–100).
pub const SHIELD_FOCUS_HEALTH_RATIO_PARAM: &str = "health_ratio_threshold";

// ── Power group allocation fine-system AI policy verb (issue #784) ────────────
//
// The Power reactor fine system allocates the ship's battery budget across the
// ship's AUTHORED power groups. The #784 conversion moves Power onto the same
// inline stateless `FineSystemAiConfigToml` spine as #779–#783, with two
// novelties: (1) the output CHANNELS are the ship's `[power_groups.*]` keys —
// dynamic per-ship data, not a fixed const slice — so the valid-channel set is
// built at load from ship data (AC1 "no fixed catalogue"); (2) the
// `set_power_group_allocation` verb is the FIRST verb to carry a MAGNITUDE — an
// absolute target level — in its payload (every prior verb was value-less or the
// boolean `set_red_alert`). The applier re-clamps to the per-group `[1, max]`
// range and the ship-wide `total <= 8` cap, so an absolute level is safe and
// idempotent; the host skips the emit when `level == current`.

/// The `set_power_group_allocation` verb: set the rule's power group to an
/// absolute target level. Its magnitude is the authored per-rule `level`
/// payload — never an inline Rust number (AGENTS.md rule #11).
pub const POWER_SET_ALLOCATION_VERB: &str = "set_power_group_allocation";

/// Authored policy-parameter name: the forward-thrust level (0.0–1.0) above
/// which the default helm-elevation rule considers the ship actively driving.
pub const POWER_THRUST_THRESHOLD_PARAM: &str = "thrust_threshold";
/// Authored policy-parameter name: the minimum battery reserve (0–100) the
/// default helm-elevation rule requires before raising helm power (AC2).
pub const POWER_HELM_RESERVE_PARAM: &str = "min_reserve_helm";
/// Authored policy-parameter name: the minimum battery reserve (0–100) the
/// default weapons-elevation rule requires before raising weapons power (AC2).
pub const POWER_WEAPONS_RESERVE_PARAM: &str = "min_reserve_weapons";
/// Authored policy-parameter name: the (zero) reserve the default LOWERING
/// baseline rules reference so every rule declares a reserve (AC2) without ever
/// gating a de-allocation, which can never cause a brownout.
pub const POWER_BASELINE_RESERVE_PARAM: &str = "min_reserve_baseline";

/// Host-seeded fact name: current battery charge as a percentage (0–100). The
/// reserve guard `fact(battery_pct) >= param(min_reserve_*)` reads this; it is
/// the stateless brownout-avoidance predicate (AC5).
pub const POWER_BATTERY_PCT_FACT: &str = "battery_pct";
/// Host-seeded fact name: latest forward thrust (0.0–1.0) from `LastHelmInput`.
pub const POWER_THRUST_FACT: &str = "thrust";
/// Host-seeded fact name: red-alert state as `1.0`/`0.0`.
pub const POWER_RED_ALERT_FACT: &str = "red_alert";

// ── Comms dialogue-response fine-system AI policy channel/verb (issue #786) ───
//
// Comms is the FIRST fine system to author BOTH machines at once, because it
// owns two different decisions:
//
//   * WHO to hail — a variable, per-tick candidate set of real contacts keyed by
//     genuine entity UUID. That is #776 selector vocabulary (see
//     [`COMMS_SELECTOR_SOURCES`] / [`default_comms_target_selector_config`]).
//   * HOW to answer an open dialogue — a fixed, small, INDEX-addressed set
//     (`ActiveDialogue.current_node.responses`, addressed by `usize`). That is
//     the #775 channel/verb model, for the same reason Shields (#783) stayed on
//     it: there is no entity set for a candidate-source selector to union.
//
// The `respond_to_message` verb is the SECOND value-carrying verb (after #784's
// `set_power_group_allocation`): only the response INDEX rides the verb — WHICH
// message is being answered comes from the host context, never the policy.

/// The `comms_respond` output channel: the Comms fine system's
/// answer-an-open-dialogue axis, resolved once per message awaiting a response.
pub const COMMS_RESPOND_CHANNEL: &str = "comms_respond";
/// The `respond_to_message` verb: answer the message being resolved with the
/// authored `response_index` payload. Its presence tells the host to emit the
/// same admitted `RespondToMessage` a human Comms officer sends — through the
/// SAME `handle_respond_to_message` router, so trigger actions and follow-ups
/// fire identically for AI and human (AGENTS.md rule #6). Its absence
/// ("hold"/idle) leaves the dialogue open this tick.
pub const COMMS_RESPOND_VERB: &str = "respond_to_message";

/// The registered output channels a Comms response policy may drive (#786).
pub const COMMS_RESPOND_CHANNELS: &[&str] = &[COMMS_RESPOND_CHANNEL];
/// The registered verbs a Comms response policy may emit (issue #786).
pub const COMMS_RESPOND_VERBS: &[&str] = &[COMMS_RESPOND_VERB];

// ── Per-system target selector sources (issue #776) ───────────────────────────

/// Candidate source: the ship's frozen combat lock (Tactical's designated
/// firing target), surfaced to the Sensors selector as the highest-priority
/// tier so Sensors mirrors what Tactical is engaging.
pub const SELECTOR_SOURCE_COMBAT_LOCK: &str = "combat-lock";
/// Candidate source: named `Destroy` objective targets resolved from the
/// scored objective pool.
pub const SELECTOR_SOURCE_OBJECTIVE_DESTROY: &str = "objective-destroy";
/// Candidate source: faction-hostile radar contacts inside the ship's horizon.
pub const SELECTOR_SOURCE_RADAR_CONTACTS: &str = "radar-contacts";

/// The registered candidate sources the Sensors target selector may union.
pub const SENSORS_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_COMBAT_LOCK,
    SELECTOR_SOURCE_OBJECTIVE_DESTROY,
    SELECTOR_SOURCE_RADAR_CONTACTS,
];

/// Candidate source: the ship's advisory **Science Target** — the Sensors
/// radar's selected target, surfaced from the frozen viewscreen blackboard
/// (issue #777). Tactical may strongly favour this pick through an authored
/// score bonus, but independently revalidates it before copying (AC2/AC3). It
/// is deliberately NOT the same as `combat-lock`: that is Tactical's OWN output
/// and is excluded to avoid circularity.
pub const SELECTOR_SOURCE_SENSORS_DESIGNATION: &str = "sensors-designation";
/// Candidate source: whoever last attacked this ship (`LastShipAttacker`).
pub const SELECTOR_SOURCE_LAST_ATTACKER: &str = "last-attacker";

/// The registered candidate sources the Tactical target selector may union
/// (issue #777). `combat-lock` is intentionally absent: it is Tactical's own
/// authoritative output, so unioning it would be circular. The ship's current
/// lock is instead surfaced by the host as an internal `source_retained`
/// retention candidate (not a cross-system source), so it too is absent here.
pub const TACTICAL_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_SENSORS_DESIGNATION,
    SELECTOR_SOURCE_OBJECTIVE_DESTROY,
    SELECTOR_SOURCE_LAST_ATTACKER,
    SELECTOR_SOURCE_RADAR_CONTACTS,
];

/// Candidate source: positive, Navigation-relevant (Helm-affinity) objective
/// destinations (issue #778). The Navigation host ranks the scored objective
/// pool, resolves the winner's directive to a destination — a fixed world
/// anchor (Reach / Retreat / Patrol) or a live entity anchor (Destroy) — and
/// surfaces it as the sole `reachable` candidate of this source.
pub const SELECTOR_SOURCE_NAV_OBJECTIVE: &str = "navigation-objectives";
/// Candidate source: live entities the Navigation chart shows, surfaced as
/// authorable entity-anchored destinations (issue #778). They do NOT carry the
/// `reachable` marker under the canonical policy, so by default they enrich a
/// coincident objective destination rather than independently steering the
/// ship; an author may re-tune the selector's eligibility to admit them.
pub const SELECTOR_SOURCE_CHART_CONTACTS: &str = "chart-contacts";

/// The registered candidate sources the Navigation target selector may union
/// (issue #778).
pub const NAVIGATION_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_NAV_OBJECTIVE,
    SELECTOR_SOURCE_CHART_CONTACTS,
];

/// Candidate source: stations the ship's coordination-delivered
/// `RepairRequestQueue` reports as damaged (issue #785). This is the AC1
/// "authoritative observable damage" surface: the Repair AI ranks only stations
/// a `RepairRequest` actually delivered — issue #830 deliberately removed the
/// raw hull poll, so a station nobody reported is not a candidate.
pub const SELECTOR_SOURCE_DAMAGED_STATIONS: &str = "damaged-stations";
/// Candidate source: the ownerless ship-wide `core` repair bucket (issue #785),
/// the second [`crate::messages::RepairTarget`] variant. Surfaced as a candidate
/// so an author can weight core repairs into the ranking; under the canonical
/// policy it only becomes eligible once a `RepairRequest` names it, mirroring
/// how `chart-contacts` enriches rather than independently steers Navigation.
pub const SELECTOR_SOURCE_CORE_BUCKET: &str = "core-bucket";

/// The registered candidate sources the Repair target selector may union
/// (issue #785).
pub const REPAIR_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_DAMAGED_STATIONS,
    SELECTOR_SOURCE_CORE_BUCKET,
];

/// Candidate source: positive, Comms-relevant `Hail` directives resolved from
/// the scored objective pool (issue #786). This is the AC1 surface: the Comms
/// AI ranks the hail orders it has actually been given, resolving each
/// directive's authored entity NAME to a real contact UUID before it can become
/// a candidate.
pub const SELECTOR_SOURCE_HAIL_OBJECTIVES: &str = "hail-objectives";
/// Candidate source: the authoritative comms contact list (issue #786) —
/// `CommsRuntime.contacts`, the same hailable roster a human Comms officer sees.
/// Under the canonical policy a contact is NOT independently eligible (the
/// default eligibility keys on `source_hail_objective`): it ENRICHES a
/// coincident hail directive with its live readings, exactly as
/// `chart-contacts` enriches a Navigation destination (#778). An author may
/// widen the eligibility to let the Comms AI hail on its own initiative.
pub const SELECTOR_SOURCE_COMMS_CONTACTS: &str = "comms-contacts";

/// The registered candidate sources the Comms hail selector may union
/// (issue #786).
pub const COMMS_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_HAIL_OBJECTIVES,
    SELECTOR_SOURCE_COMMS_CONTACTS,
];

/// Parse-time fallbacks for the default Tactical selector (AGENTS.md rule #11
/// parse-defaults only). The retired tier order was
/// `objective > retained > last-attacker > nearest`; each tier becomes an
/// additive source weight, highest-first, with the Sensors-favour bonus (AC2)
/// slotted between objective and retained. `switch_margin` is the anti-thrash
/// hysteresis (AC5). Every value is overridable via `[weapons_console.selector]`
/// fields or its `param` table, so no gameplay value is pinned into a live tick.
///
/// PRECEDENCE INVARIANT — because the selector sums weights and a single
/// candidate can carry several source markers at once (the ship's current lock
/// is commonly ALSO its Sensors designation, and may also be the last attacker
/// and the nearest hostile), the weights are chosen so `objective_weight`
/// strictly dominates the MAXIMUM achievable non-objective stack by more than
/// `switch_margin`:
///
/// ```text
///   sensors_designation + retained + last_attacker + radar
///     = 500 + 200 + 100 + 1 = 801  <  1000 − 50 = 950  =  objective − margin
/// ```
///
/// So an in-range named Destroy objective ALWAYS wins the ranking AND survives
/// hysteresis retention — even against the ship's own retained Sensors
/// designation. `retained` still exceeds `last_attacker`, so an established
/// engagement is not broken off by a fresh attacker (the retired tier-2 > tier-3
/// ordering). Retention thus has a bounded additive contribution AND
/// switch-margin hysteresis; the invariant, not the mechanism name, is what
/// guarantees objective primacy. This invariant is asserted in
/// `default_tactical_selector_objective_dominates_max_non_objective_stack`.
const DEFAULT_TACTICAL_OBJECTIVE_WEIGHT: f32 = 1000.0;
const DEFAULT_TACTICAL_SENSORS_DESIGNATION_WEIGHT: f32 = 500.0;
const DEFAULT_TACTICAL_RETAINED_WEIGHT: f32 = 200.0;
const DEFAULT_TACTICAL_LAST_ATTACKER_WEIGHT: f32 = 100.0;
const DEFAULT_TACTICAL_RADAR_WEIGHT: f32 = 1.0;
const DEFAULT_TACTICAL_SWITCH_MARGIN: f32 = 50.0;

/// Parse-time fallbacks for the default Sensors selector (AGENTS.md rule #11
/// parse-defaults only). Authors override every one via `[sensors_console.selector]`
/// fields or its `param` table, so no gameplay value is pinned into a live tick.
///
/// `HORIZON` is deliberately large: the Sensors host owns the live,
/// damage-scaled horizon and pre-filters candidates to it (`effective_sensor_range`),
/// so the selector's own horizon is a static outer bound, not the live gate.
const DEFAULT_SELECTOR_HORIZON: f32 = 1.0e9;
const DEFAULT_SELECTOR_SWITCH_MARGIN: f32 = 0.0;
const DEFAULT_SELECTOR_COMBAT_LOCK_WEIGHT: f32 = 1000.0;
const DEFAULT_SELECTOR_OBJECTIVE_WEIGHT: f32 = 100.0;
const DEFAULT_SELECTOR_RADAR_WEIGHT: f32 = 1.0;

/// Parse-time fallbacks for the default Navigation selector (issue #778,
/// AGENTS.md rule #11 parse-defaults only). The retired Navigation AI ranked
/// the scored objective pool and steered to the best positive Helm-relevant
/// objective's destination; those objective destinations are the `reachable`
/// tier here (`objective_weight`), dominating the enriching chart-contacts tier
/// (`chart_contact_weight`). `switch_margin` is the anti-thrash hysteresis
/// (AC3). Authors override every value via `[navigation_console.selector]`, so
/// no gameplay value is pinned into a live tick.
const DEFAULT_NAV_OBJECTIVE_WEIGHT: f32 = 100.0;
const DEFAULT_NAV_CHART_CONTACT_WEIGHT: f32 = 1.0;
const DEFAULT_NAV_SWITCH_MARGIN: f32 = 0.0;

/// Parse-time fallbacks for the default Repair selector (issue #785,
/// AGENTS.md rule #11 parse-defaults only). The retired hardcoded Repair
/// comparator sorted the repair-request queue by `(tier desc, deficit desc)`;
/// here that becomes an additive utility over two ladders.
///
/// PRECEDENCE INVARIANT — tier STRICTLY dominates deficit. The tier ladder
/// contributes `tier_weight` once per damage-tier ordinal step (Damaged = 1×,
/// Disabled = 2×), while the deficit ladder contributes `deficit_weight` once
/// per crossed damage-fraction band, so the maximum achievable deficit stack is
///
/// ```text
///   3 × deficit_weight = 3 × 100 = 300  <  1000 = tier_weight
/// ```
///
/// i.e. a single tier step always outranks the entire deficit ladder, exactly
/// reproducing the retired comparator's "tier first, deficit only as the
/// tie-break" ordering. Asserted in
/// `default_repair_selector_tier_dominates_max_deficit_stack`.
///
/// The deficit ladder is a BANDED approximation of the retired continuous
/// `deficit desc` tie-break, because an authored score term contributes a fixed
/// weight when its boolean guard fires — the selector has no multiplicative
/// term. Within one band the ranking falls through to the selector's documented
/// smallest-key tie-break (station id), which is deterministic (AC4). Authors
/// widen or narrow the bands via the `deficit_band_*` params without touching
/// Rust.
///
/// WHY THE BANDS SIT INSIDE THE URGENT RANGE — do NOT "helpfully" realign them
/// to the `DamageTier` thresholds (0.75 / 0.25 HP, `src/ship/damage.rs`). The
/// deficit ladder only ever discriminates WITHIN one tier, because tier
/// strictly dominates it. Bands placed AT the tier thresholds
/// (0.25 / 0.5 / 0.75 damage fraction) are therefore all-or-nothing dead
/// weight: every Disabled station is by definition above 0.75 damage, so all
/// three bands fire for all of them and a station at 1/100 HP scores exactly
/// what a station at 24/100 HP scores. Sitting inside the urgent range
/// (0.80 / 0.90 / 0.95) instead, the ladder splits the Disabled tier into four
/// buckets and the nearly-dead station actually outranks the barely-disabled
/// one — which is the whole point of a tie-break. The Damaged tier's remaining
/// span (0.25–0.75 damage) resolves on the deterministic station-id tie-break;
/// an author who wants discrimination there re-points `deficit_band_low`.
///
/// `switch_margin` is 0: Repair's retained pick is the authoritative
/// `TeamSlot`, and only Idle teams are dispatched, so there is no AI-side
/// hysteresis to tune (see `operate_repair_ai`'s AC5 note).
const DEFAULT_REPAIR_TIER_WEIGHT: f32 = 1000.0;
const DEFAULT_REPAIR_DEFICIT_WEIGHT: f32 = 100.0;
const DEFAULT_REPAIR_DEFICIT_BAND_LOW: f32 = 0.80;
const DEFAULT_REPAIR_DEFICIT_BAND_MID: f32 = 0.90;
const DEFAULT_REPAIR_DEFICIT_BAND_HIGH: f32 = 0.95;
const DEFAULT_REPAIR_SWITCH_MARGIN: f32 = 0.0;

/// Parse-time fallbacks for the default Comms hail selector (issue #786,
/// AGENTS.md rule #11 parse-defaults only). The retired hardcoded Comms AI
/// filtered the scored pool to positive, Comms-relevant `Hail` directives and
/// took the FIRST one that resolved and was in range (`scored_pool` is sorted
/// descending, so that was an implicit "highest score wins" argmax). Here that
/// ordering becomes an authored BANDED score ladder.
///
/// WHY A BANDED LADDER AND NOT A SINGLE `objective_score` TERM — the #785
/// lesson. A `ScoreTerm` contributes a FIXED weight when its boolean guard
/// fires; the selector has no multiplicative term, so a continuous reading can
/// only enter the ranking as a ladder of thresholds. One term guarded on
/// `objective_score > 0` would give every eligible hail an identical score and
/// collapse the whole ranking onto the selector's smallest-UUID tie-break —
/// i.e. the POLICY would not rank at all (AC1). The ladder makes the ranking
/// genuinely authored.
///
/// WHERE THE BANDS SIT AND WHY — the other #785 lesson: bands must be placed
/// where the population actually is, or they discriminate nothing. Objective
/// utility is `base_priority (+10 if mandatory) + condition modifiers`, and the
/// shipped content authors `base_priority` at 20 / 30 / 35 / 40 / 45 / 50 / 80 /
/// 100. Bands at 25 / 45 / 75 therefore split that population four ways:
///
/// ```text
///   20            → 0 bands   (background chatter)
///   30 / 35 / 40  → 1 band    (routine orders)
///   45 / 50       → 2 bands   (priority orders)
///   80 / 100      → 3 bands   (mission-critical)
/// ```
///
/// Bands at, say, 100/200/300 would fire for nothing and every hail would tie;
/// bands at 1/2/3 would fire for everything and every hail would tie. Within one
/// band the ranking falls through to the selector's documented smallest-UUID
/// tie-break, which is deterministic. Authors re-point `score_band_*` without
/// touching Rust.
///
/// `switch_margin` is 0 and the host passes `current: None`: a hail is a
/// ONE-SHOT event, not a retained target, so there is nothing to apply
/// hysteresis to (see `operate_comms_ai`'s AC5 note).
const DEFAULT_COMMS_SCORE_BAND_WEIGHT: f32 = 100.0;
const DEFAULT_COMMS_SCORE_BAND_LOW: f32 = 25.0;
const DEFAULT_COMMS_SCORE_BAND_MID: f32 = 45.0;
const DEFAULT_COMMS_SCORE_BAND_HIGH: f32 = 75.0;
const DEFAULT_COMMS_SWITCH_MARGIN: f32 = 0.0;

/// Parse-time fallback: the response index the canonical Comms response policy
/// picks (issue #786). Reproduces the retired `handle_comms_channel2` stub's
/// hardcoded `record_response(&id, 0)` — the FIRST available response — with the
/// difference that it now travels through admission and the real router.
const DEFAULT_COMMS_RESPONSE_INDEX: u8 = 0;

/// One authored additive utility term (`[[sensors_console.selector.score]]`,
/// issue #776): a guard expression plus the weight it contributes to a
/// candidate's score when it fires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreTermToml {
    /// Guard expression over self/candidate/target fact contexts; the term
    /// contributes `weight` when it evaluates `true`.
    pub when: String,
    /// Weight added to the candidate's score when `when` fires. An authored
    /// field (AGENTS.md rule #11 permits authored gameplay values in TOML).
    pub weight: f32,
}

/// Inline per-system target selector for an AI-capable fine system that owns a
/// target (`[sensors_console.selector]`, issue #776).
///
/// Sibling to [`FineSystemAiConfigToml`]: where the #775 policy resolves a
/// verb per output channel, the selector answers "which entity is my target?".
/// It unions authored candidate `sources`, filters inside `horizon`, keeps
/// candidates whose `eligibility` guard fires, sums the additive `score`,
/// retains the current target within `switch_margin`, and returns the winning
/// contact — all as a pure function of the immutable per-tick snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiSelectorToml {
    /// Named numeric parameters referenced by the eligibility/score guards.
    #[serde(default)]
    pub param: std::collections::HashMap<String, f32>,
    /// Registered candidate-source ids this selector unions.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Effective horizon (planar distance) beyond which candidates are dropped.
    pub horizon: f32,
    /// Hysteresis margin: the current target is retained while its score is
    /// within this margin of the best candidate's score.
    pub switch_margin: f32,
    /// Candidate eligibility guard over self/candidate/target fact contexts.
    pub eligibility: String,
    /// Additive utility terms summed per eligible candidate.
    #[serde(default)]
    pub score: Vec<ScoreTermToml>,
}

impl FineSystemAiSelectorToml {
    /// Resolve this authored block into the pure typed
    /// [`crate::ai::selector::TargetSelector`] the runtime consumes.
    ///
    /// Returns a diagnostic `Err` on an unparseable eligibility/score guard;
    /// call [`validate_fine_system_ai_selector`] first at content-load time so
    /// this never fails after world activation.
    pub fn to_selector(&self) -> Result<crate::ai::selector::TargetSelector, String> {
        let mut params = crate::world::flags::AiParams::new();
        for (k, v) in &self.param {
            params.set(k, *v as f64);
        }
        let eligibility = crate::world::flags::parse_predicate(&self.eligibility)?;
        let mut score = Vec::with_capacity(self.score.len());
        for term in &self.score {
            let when = crate::world::flags::parse_predicate(&term.when)?;
            score.push(crate::ai::selector::ScoreTerm {
                when,
                weight: term.weight as f64,
            });
        }
        Ok(crate::ai::selector::TargetSelector {
            params,
            sources: self.sources.clone(),
            horizon: self.horizon,
            switch_margin: self.switch_margin,
            eligibility,
            score,
        })
    }
}

/// The canonical default Sensors target selector synthesised for ships that do
/// not author `[sensors_console.selector]` (issue #776).
///
/// Reproduces the retired hardcoded Sensors tiers as data: prefer the combat
/// lock, then a named objective target, then a radar hostile, all restricted
/// to detectable, hostile contacts. All weights/horizon/margin are named
/// parameters or parse-time defaults, so a designer can retune without Rust.
pub fn default_sensors_target_selector_config() -> FineSystemAiSelectorToml {
    let mut param = std::collections::HashMap::new();
    param.insert(
        "combat_lock_weight".to_string(),
        DEFAULT_SELECTOR_COMBAT_LOCK_WEIGHT,
    );
    param.insert(
        "objective_weight".to_string(),
        DEFAULT_SELECTOR_OBJECTIVE_WEIGHT,
    );
    param.insert("radar_weight".to_string(), DEFAULT_SELECTOR_RADAR_WEIGHT);
    FineSystemAiSelectorToml {
        param,
        sources: SENSORS_SELECTOR_SOURCES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        horizon: DEFAULT_SELECTOR_HORIZON,
        switch_margin: DEFAULT_SELECTOR_SWITCH_MARGIN,
        // Only detectable, hostile contacts are eligible — this is the
        // hidden/friendly drop (AC4), expressed over the candidate context.
        eligibility: "candidate_fact(detectable) > 0 and candidate_fact(hostile) > 0".to_string(),
        score: vec![
            ScoreTermToml {
                when: "candidate_fact(source_combat_lock) > 0".to_string(),
                weight: DEFAULT_SELECTOR_COMBAT_LOCK_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(source_objective) > 0".to_string(),
                weight: DEFAULT_SELECTOR_OBJECTIVE_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(source_radar) > 0".to_string(),
                weight: DEFAULT_SELECTOR_RADAR_WEIGHT,
            },
        ],
    }
}

/// The canonical default Tactical target selector synthesised for ships that
/// do not author `[weapons_console.selector]` (issue #777).
///
/// Encodes the retired hardcoded Tactical tier chain as data. The old tier
/// order was `objective ≫ retained ≫ last-attacker ≫ nearest-hostile`; each
/// tier becomes an additive source weight, highest-first, and the advisory
/// Sensors designation gets its own favour bonus (AC2) — below explicit mission
/// orders (`objective`) but above the retained lock and whoever last shot us.
///
/// Because the selector SUMS weights and one entity can carry several source
/// markers (the current lock is commonly also the Sensors designation, and may
/// also be the last attacker and nearest hostile), a naive high `retained`
/// weight would let `sensors_designation + retained` overtake a distinct
/// in-range `objective` — the ship would refuse to retarget onto its explicit
/// mission objective. The weights are therefore sized so `objective` strictly
/// dominates the maximum achievable non-objective stack by more than
/// `switch_margin` (see the PRECEDENCE INVARIANT on the constant block above:
/// 500 + 200 + 100 + 1 = 801 < 1000 − 50). Retention keeps a bounded additive
/// contribution (still > `last_attacker`, so an established engagement is not
/// broken off by a fresh attacker) AND the selector's switch-margin hysteresis;
/// the invariant, not the mechanism, is what guarantees objective primacy.
///
/// Independent revalidation (AC3) is the eligibility guard: a candidate is
/// engageable when it is detectable AND either an explicit target the host
/// already vetted (`source_objective` / `source_last_attacker` /
/// `source_retained`) OR independently hostile — so a friendly Sensors
/// designation, carrying only `source_sensors_designation`, is dropped rather
/// than copied.
///
/// All weights, the Sensors-favour bonus, the switch margin, and the horizon
/// are named parameters or parse-time defaults, so a designer retunes Tactical
/// target ranking without touching Rust (AGENTS.md rule #11).
pub fn default_tactical_target_selector_config() -> FineSystemAiSelectorToml {
    let mut param = std::collections::HashMap::new();
    param.insert(
        "objective_weight".to_string(),
        DEFAULT_TACTICAL_OBJECTIVE_WEIGHT,
    );
    param.insert(
        "sensors_designation_weight".to_string(),
        DEFAULT_TACTICAL_SENSORS_DESIGNATION_WEIGHT,
    );
    param.insert(
        "retained_weight".to_string(),
        DEFAULT_TACTICAL_RETAINED_WEIGHT,
    );
    param.insert(
        "last_attacker_weight".to_string(),
        DEFAULT_TACTICAL_LAST_ATTACKER_WEIGHT,
    );
    param.insert("radar_weight".to_string(), DEFAULT_TACTICAL_RADAR_WEIGHT);
    FineSystemAiSelectorToml {
        param,
        sources: TACTICAL_SELECTOR_SOURCES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        horizon: DEFAULT_SELECTOR_HORIZON,
        switch_margin: DEFAULT_TACTICAL_SWITCH_MARGIN,
        // AC3 independent revalidation. Detectable is a precondition for every
        // candidate; beyond that, an explicit target the host already vetted
        // (objective order / last attacker / retained lock) is engageable
        // regardless of faction, while any other candidate — crucially the
        // advisory Sensors designation and auto-acquired radar contacts — must
        // be independently `hostile`. This is what makes Tactical refuse a
        // friendly Sensors pick (AC3) while still honouring a mission that
        // names a factionless assault target.
        eligibility: "candidate_fact(detectable) > 0 and (candidate_fact(source_objective) > 0 \
                      or candidate_fact(source_last_attacker) > 0 \
                      or candidate_fact(source_retained) > 0 \
                      or candidate_fact(hostile) > 0)"
            .to_string(),
        // Additive source weights, highest-first. The `source_retained` term is
        // bounded (see the PRECEDENCE INVARIANT): it exceeds `last_attacker` so
        // an established lock is not stolen by a fresh attacker, but the whole
        // non-objective stack stays below `objective − switch_margin`.
        score: vec![
            ScoreTermToml {
                when: "candidate_fact(source_objective) > 0".to_string(),
                weight: DEFAULT_TACTICAL_OBJECTIVE_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(source_sensors_designation) > 0".to_string(),
                weight: DEFAULT_TACTICAL_SENSORS_DESIGNATION_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(source_retained) > 0".to_string(),
                weight: DEFAULT_TACTICAL_RETAINED_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(source_last_attacker) > 0".to_string(),
                weight: DEFAULT_TACTICAL_LAST_ATTACKER_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(source_radar) > 0".to_string(),
                weight: DEFAULT_TACTICAL_RADAR_WEIGHT,
            },
        ],
    }
}

/// The canonical default Navigation target selector synthesised for ships that
/// do not author `[navigation_console.selector]` (issue #778).
///
/// Encodes the retired hardcoded Navigation AI ranking as data. The old path
/// picked the top positive Helm-relevant objective and resolved its directive
/// to a destination; here that destination is the sole `reachable` candidate of
/// the `navigation-objectives` source and always outweighs the `chart-contacts`
/// tier. Because the default eligibility admits only `reachable` candidates —
/// and only the objective source marks its resolved destination reachable — the
/// canonical policy drives the waypoint from objectives alone (the retired
/// contract). Chart contacts are surfaced so an author can weight them into
/// eligible destinations without touching Rust; by default they merely enrich a
/// coincident objective destination. All weights, the switch margin, and the
/// horizon are named parameters or parse-time defaults (AGENTS.md rule #11).
pub fn default_navigation_target_selector_config() -> FineSystemAiSelectorToml {
    let mut param = std::collections::HashMap::new();
    param.insert("objective_weight".to_string(), DEFAULT_NAV_OBJECTIVE_WEIGHT);
    param.insert(
        "chart_contact_weight".to_string(),
        DEFAULT_NAV_CHART_CONTACT_WEIGHT,
    );
    FineSystemAiSelectorToml {
        param,
        sources: NAVIGATION_SELECTOR_SOURCES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        // The Navigation chart is the whole-system view, not a radar: the host
        // owns no live horizon gate (a Destroy hand-off deliberately steers the
        // Helm toward something it cannot yet see), so the selector's own
        // horizon is a static outer bound, matching Sensors/Tactical.
        horizon: DEFAULT_SELECTOR_HORIZON,
        switch_margin: DEFAULT_NAV_SWITCH_MARGIN,
        // Only reachable destinations are eligible. The objective source marks
        // its resolved destination reachable; chart contacts do not, so the
        // canonical policy reproduces the retired "objectives drive the AI
        // waypoint" contract. An author may widen this to admit chart contacts.
        eligibility: "candidate_fact(reachable) > 0".to_string(),
        score: vec![
            ScoreTermToml {
                when: "candidate_fact(source_nav_objective) > 0".to_string(),
                weight: DEFAULT_NAV_OBJECTIVE_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(source_chart_contact) > 0".to_string(),
                weight: DEFAULT_NAV_CHART_CONTACT_WEIGHT,
            },
        ],
    }
}

/// The canonical default Repair target selector synthesised for ships that do
/// not author `[repair.selector]` (issue #785).
///
/// Encodes the retired hardcoded Repair comparator as data. `operate_repair_ai`
/// used to sort the repair-request queue by `(tier desc, deficit desc)` and hand
/// the top unassigned station to the lowest free team; here the same ordering is
/// an additive utility whose tier ladder strictly dominates its deficit ladder
/// (see the PRECEDENCE INVARIANT on the constant block above).
///
/// Eligibility does the AC1 + AC2 work:
///   - `source_repair_request > 0` — only stations coordination actually
///     reported are ranked. This is what preserves the baseline: the
///     `core-bucket` source is surfaced for authors but carries no repair
///     request of its own, so by default it never independently selects (the
///     same shape as Navigation's `chart-contacts`).
///   - `assigned < 1` — a station a team is already Travelling to, Repairing,
///     or that an earlier team in this same tick was just dispatched to, is
///     excluded, so N free teams pick N DISTINCT stations (AC2/AC4).
///
/// All weights, bands and the switch margin are named parameters or parse-time
/// defaults, so a designer retunes repair priority without Rust (rule #11).
pub fn default_repair_target_selector_config() -> FineSystemAiSelectorToml {
    let mut param = std::collections::HashMap::new();
    param.insert("tier_weight".to_string(), DEFAULT_REPAIR_TIER_WEIGHT);
    param.insert("deficit_weight".to_string(), DEFAULT_REPAIR_DEFICIT_WEIGHT);
    param.insert(
        "deficit_band_low".to_string(),
        DEFAULT_REPAIR_DEFICIT_BAND_LOW,
    );
    param.insert(
        "deficit_band_mid".to_string(),
        DEFAULT_REPAIR_DEFICIT_BAND_MID,
    );
    param.insert(
        "deficit_band_high".to_string(),
        DEFAULT_REPAIR_DEFICIT_BAND_HIGH,
    );
    FineSystemAiSelectorToml {
        param,
        sources: REPAIR_SELECTOR_SOURCES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        // Repair candidates are the operating ship's OWN stations, so every
        // candidate sits at the ship's position and the horizon never gates.
        // Kept at the shared static outer bound for consistency with the other
        // selector hosts.
        horizon: DEFAULT_SELECTOR_HORIZON,
        switch_margin: DEFAULT_REPAIR_SWITCH_MARGIN,
        // AC2 eligibility: an unassigned, coordination-reported station whose
        // damage is actually repairable. `tier_ordinal` is the DamageTier
        // discriminant (Operational 0, Damaged 1, Disabled 2, Destroyed 3) — a
        // structural enum ordinal, not a tunable gameplay value. Destroyed is
        // excluded because a repair team alone cannot lift the latch.
        eligibility: "candidate_fact(source_repair_request) > 0 \
                      and candidate_fact(assigned) < 1 \
                      and candidate_fact(tier_ordinal) > 0 \
                      and candidate_fact(tier_ordinal) < 3"
            .to_string(),
        score: vec![
            // Tier ladder — one `tier_weight` step per ordinal reached.
            ScoreTermToml {
                when: "candidate_fact(tier_ordinal) >= 1".to_string(),
                weight: DEFAULT_REPAIR_TIER_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(tier_ordinal) >= 2".to_string(),
                weight: DEFAULT_REPAIR_TIER_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(tier_ordinal) >= 3".to_string(),
                weight: DEFAULT_REPAIR_TIER_WEIGHT,
            },
            // Deficit ladder — the banded stand-in for the retired continuous
            // `deficit desc` tie-break. Bounded to 3 × deficit_weight so it can
            // never overturn a tier step.
            ScoreTermToml {
                when: "candidate_fact(damage_fraction) >= param(deficit_band_low)".to_string(),
                weight: DEFAULT_REPAIR_DEFICIT_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(damage_fraction) >= param(deficit_band_mid)".to_string(),
                weight: DEFAULT_REPAIR_DEFICIT_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(damage_fraction) >= param(deficit_band_high)".to_string(),
                weight: DEFAULT_REPAIR_DEFICIT_WEIGHT,
            },
        ],
    }
}

/// The canonical default Comms hail selector synthesised for ships that do not
/// author `[comms_console.selector]` (issue #786).
///
/// Encodes the retired hardcoded Comms filter+argmax as data. `operate_comms_ai`
/// used to `filter(score > 0 && relevance contains Comms && directive is Hail)`
/// and then `find_map` the first entry that resolved to a UUID and was in range;
/// here the filter becomes the `eligibility` guard and the implicit
/// highest-score-first ordering becomes the authored banded `score` ladder (see
/// the PRECEDENCE / BAND-PLACEMENT notes on the constant block above).
///
/// Eligibility does the AC1 + AC2 work:
///   - `source_hail_objective > 0` — only targets an active `Hail` directive
///     actually names are ranked. This is what preserves the baseline: the
///     `comms-contacts` source is surfaced for authors but carries no directive
///     of its own, so by default a contact never independently hails (the same
///     shape as Navigation's `chart-contacts`, #778).
///   - `in_range > 0` — the AC2 comms-range gate, seeded host-side from
///     `CommsRuntime.range_flags` honouring `range_active`. Defence in depth:
///     `handle_hail` keeps its own hard server-side range check.
///   - `objective_score > 0` — the zero-gate drop, exactly the retired
///     `s.score > 0.0` filter.
///   - `has_open_hail_thread < 1` — the anti-respam gate, read from the
///     AUTHORITATIVE `CommsRuntime.open_hails` record that `handle_hail` writes
///     for every hail (human officer or AI) rather than from the retired
///     `CommsAiHailState.last_hailed` AI memory. It is TERMINATING: a hail that
///     fires no `on_hailed` template still arms it, so a standing directive
///     cannot re-emit every tick. It re-arms on two externally-driven events —
///     a human officer's `ClearComms`, and the target ceasing to be a live hail
///     candidate (its directive gone, or out of range), which `operate_comms_ai`
///     retires per tick. The second is what an UNMANNED ship relies on: there is
///     no scripted `ClearComms`, so without it a later `Hail` directive naming
///     an already-hailed contact would be dropped forever. Termination survives
///     because a standing directive's target stays a candidate every tick.
///     Deliberately NOT `has_unread_from_sender` — that fact is true of any
///     inbound message whatever its provenance, so gating on it would let a
///     scenario-pushed greeting permanently suppress a legitimate hail.
///   - `self_fact(comms_available) > 0` — the AC2 system-availability gate, read
///     off `EntitySystemHull`: a Disabled or Destroyed Comms system stops the
///     ship hailing at all.
///
/// All weights and bands are named parameters or parse-time defaults, so a
/// designer retunes hail priority without Rust (rule #11). Comms RANGE is
/// already authored via `[comms].range` and is deliberately NOT duplicated as a
/// second constant here — the selector reads it as a seeded fact.
pub fn default_comms_target_selector_config() -> FineSystemAiSelectorToml {
    let mut param = std::collections::HashMap::new();
    param.insert("score_band_low".to_string(), DEFAULT_COMMS_SCORE_BAND_LOW);
    param.insert("score_band_mid".to_string(), DEFAULT_COMMS_SCORE_BAND_MID);
    param.insert("score_band_high".to_string(), DEFAULT_COMMS_SCORE_BAND_HIGH);
    param.insert(
        "score_band_weight".to_string(),
        DEFAULT_COMMS_SCORE_BAND_WEIGHT,
    );
    FineSystemAiSelectorToml {
        param,
        sources: COMMS_SELECTOR_SOURCES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        // Comms candidates are hail TARGETS, not spatial destinations: comms
        // reach is the authored `[comms].range` radius, already resolved into
        // the `in_range` fact by `update_comms_range_flags`. The host therefore
        // places every candidate at the ship's own origin so the planar horizon
        // never double-gates what range already decided; the value is kept at
        // the shared static outer bound for consistency with the other hosts.
        horizon: DEFAULT_SELECTOR_HORIZON,
        switch_margin: DEFAULT_COMMS_SWITCH_MARGIN,
        eligibility: "candidate_fact(source_hail_objective) > 0 \
                      and candidate_fact(in_range) > 0 \
                      and candidate_fact(objective_score) > 0 \
                      and candidate_fact(has_open_hail_thread) < 1 \
                      and self_fact(comms_available) > 0"
            .to_string(),
        score: vec![
            ScoreTermToml {
                when: "candidate_fact(objective_score) >= param(score_band_low)".to_string(),
                weight: DEFAULT_COMMS_SCORE_BAND_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(objective_score) >= param(score_band_mid)".to_string(),
                weight: DEFAULT_COMMS_SCORE_BAND_WEIGHT,
            },
            ScoreTermToml {
                when: "candidate_fact(objective_score) >= param(score_band_high)".to_string(),
                weight: DEFAULT_COMMS_SCORE_BAND_WEIGHT,
            },
        ],
    }
}

/// Validate an inline per-system target selector before world activation
/// (issue #776), mirroring [`validate_fine_system_ai_policy`].
///
/// Rejects:
///   - an unknown candidate source id,
///   - an unparseable `eligibility` or score `when` expression,
///   - a `param(...)` reference to a parameter the author never declared.
pub fn validate_fine_system_ai_selector(
    cfg: &FineSystemAiSelectorToml,
    valid_sources: &[&str],
) -> Result<(), String> {
    for src in &cfg.sources {
        if !valid_sources.contains(&src.as_str()) {
            return Err(format!(
                "target selector references unknown source '{src}' (valid: {valid_sources:?})"
            ));
        }
    }
    let check_params = |pred: &crate::world::flags::Predicate, what: &str| -> Result<(), String> {
        let mut refs = Vec::new();
        pred.referenced_params(&mut refs);
        for name in refs {
            if !cfg.param.contains_key(&name) {
                return Err(format!(
                    "target selector {what} references undeclared parameter '{name}'"
                ));
            }
        }
        Ok(())
    };
    let eligibility = crate::world::flags::parse_predicate(&cfg.eligibility)
        .map_err(|e| format!("target selector has invalid `eligibility` expression: {e}"))?;
    check_params(&eligibility, "eligibility")?;
    for (idx, term) in cfg.score.iter().enumerate() {
        let when = crate::world::flags::parse_predicate(&term.when)
            .map_err(|e| format!("target selector score term {idx} has invalid `when`: {e}"))?;
        check_params(&when, &format!("score term {idx}"))?;
    }
    Ok(())
}

/// One authored inline policy rule (`[[captain_console.ai.rule]]`, issue #775).
///
/// A rule binds a `priority` and an output `channel` to a `when` predicate
/// (the shared `world::flags` grammar, extended with typed `fact(...)` atoms
/// and `param(...)` references) and a typed `verb`. `value` is the boolean the
/// verb applies for boolean-channel verbs such as `set_red_alert`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiRuleToml {
    /// Higher wins within a channel; ties resolve to the earliest-authored rule.
    pub priority: i32,
    /// Output channel this rule contributes to (e.g. `"red_alert"`).
    pub channel: String,
    /// Guard expression; the rule fires when it evaluates `true`.
    pub when: String,
    /// Typed verb applied when this rule wins its channel (e.g. `"set_red_alert"`).
    pub verb: String,
    /// Boolean output value for boolean-channel verbs. Defaults to `false`.
    #[serde(default)]
    pub value: bool,
    /// Magnitude payload for value-carrying verbs (issue #784). Currently the
    /// absolute target level for `set_power_group_allocation`; ignored by
    /// value-less and boolean verbs. Defaults to `0`.
    #[serde(default)]
    pub level: u8,
    /// Index payload for the `respond_to_message` verb (issue #786): WHICH of
    /// the open dialogue node's responses this rule answers with. Ignored by
    /// every other verb. Deliberately a separate field from `level` — the two
    /// address different things (a power magnitude vs. a position in a fixed
    /// response list), and fusing them would make an authored rule's meaning
    /// depend on its verb. Defaults to `0` (the first response), reproducing the
    /// retired channel-2 auto-response stub. A rule that should NOT answer this
    /// tick simply does not fire; there is no "don't respond" index.
    #[serde(default)]
    pub response_index: u8,
}

/// One authored state of an inline STATEFUL policy
/// (`[[<system>.ai.state]]`, issue #882).
///
/// A state carries its own continuous `rule` list — the very same
/// [`FineSystemAiRuleToml`] the stateless path uses, so a rule's meaning does
/// not change with where it is authored — and its own outgoing `transition`
/// list. Note [`FineSystemAiRuleToml`] deliberately gained NO `state` field:
/// it is `deny_unknown_fields`, and nesting rules under the state that owns
/// them keeps a rule's owning state unambiguous.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiStateToml {
    /// Unique state id within this policy; referenced by `initial_state` and
    /// by every transition's `to`.
    pub id: String,
    /// Continuous per-channel rules that apply while this state is current.
    #[serde(default)]
    pub rule: Vec<FineSystemAiRuleToml>,
    /// Outgoing transitions, at most one of which fires per eligible tick.
    #[serde(default)]
    pub transition: Vec<FineSystemAiTransitionToml>,
}

/// One authored transition out of the enclosing state
/// (`[[<system>.ai.state.transition]]`, issue #882).
///
/// There is no `from`: the source is the state this table is nested in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiTransitionToml {
    /// Higher wins; ties resolve to the earliest-authored transition.
    pub priority: i32,
    /// The state id entered when this transition fires.
    pub to: String,
    /// Guard expression; the transition becomes eligible when it evaluates
    /// `true`. May read `memory(...)` and `state_time` as well as the usual
    /// facts/flags/params.
    pub when: String,
}

/// Inline AI policy for an AI-capable fine system
/// (`[captain_console.ai]`, issues #775, #882).
///
/// A system declares EITHER a policy (`param` + `rule`, and optionally the
/// #882 state machine) OR an explicit `idle = true`. An empty declaration
/// (`ai = {}`) is neither and is rejected by
/// [`validate_fine_system_ai_policy`] — silence is not a valid declaration.
///
/// ## Back-compat guarantee (issue #882)
///
/// Every field added by the stateful path is `#[serde(default)]`, so all
/// twelve shipped stateless blocks parse byte-identically and decode to a
/// policy whose `machine` is `None`. A block that authors no `state` never
/// enters the transition code path at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiConfigToml {
    /// Explicit idle marker. Mutually exclusive with `rule` and with `state`.
    #[serde(default)]
    pub idle: bool,
    /// Named numeric parameters referenced by rule guards.
    #[serde(default)]
    pub param: std::collections::HashMap<String, f32>,
    /// Prioritised per-channel reactive rules (the stateless path).
    #[serde(default)]
    pub rule: Vec<FineSystemAiRuleToml>,
    /// The state entered on reset. Required when — and rejected unless —
    /// `state` is non-empty (issue #882).
    #[serde(default)]
    pub initial_state: Option<String>,
    /// The declared states of the OPTIONAL state machine (issue #882).
    /// Absent/empty ⇒ this is a stateless policy.
    #[serde(default)]
    pub state: Vec<FineSystemAiStateToml>,
    /// Typed private memory declarations: name → initial value (issue #882).
    /// Readable through the `memory(name)` atom by THIS fine system only.
    #[serde(default)]
    pub memory: std::collections::HashMap<String, f32>,
}

impl FineSystemAiConfigToml {
    /// Resolve this authored block into the pure typed [`crate::ai::policy::AiPolicy`]
    /// the runtime evaluator consumes.
    ///
    /// Returns a diagnostic `Err` on an unparseable guard or unknown verb; call
    /// [`validate_fine_system_ai_policy`] first at content-load time so this
    /// never fails after world activation.
    pub fn to_policy(&self) -> Result<crate::ai::policy::AiPolicy, String> {
        let mut params = crate::world::flags::AiParams::new();
        for (k, v) in &self.param {
            params.set(k, *v as f64);
        }
        let rules = decode_rules(&self.rule)?;
        // The OPTIONAL #882 state machine. No authored `state` tables ⇒ `None`,
        // which is what every shipped stateless block decodes to.
        let machine = if self.state.is_empty() {
            None
        } else {
            let mut states = Vec::with_capacity(self.state.len());
            for s in &self.state {
                let mut transitions = Vec::with_capacity(s.transition.len());
                for t in &s.transition {
                    transitions.push(crate::ai::policy::AiPolicyTransition {
                        priority: t.priority,
                        to: t.to.clone(),
                        when: crate::world::flags::parse_predicate(&t.when)?,
                    });
                }
                states.push(crate::ai::policy::AiPolicyState {
                    id: s.id.clone(),
                    rules: decode_rules(&s.rule)?,
                    transitions,
                });
            }
            Some(crate::ai::policy::AiPolicyMachine {
                initial: self.initial_state.clone().ok_or_else(|| {
                    "ai policy declares states but no `initial_state`".to_string()
                })?,
                initial_memory: self.initial_memory(),
                states,
            })
        };
        Ok(crate::ai::policy::AiPolicy {
            params,
            rules,
            idle: self.idle,
            machine,
        })
    }

    /// The authored initial values of this policy's typed private memory
    /// (issue #882), as the runtime bag a fresh state component starts from.
    pub fn initial_memory(&self) -> crate::world::flags::AiPolicyMemory {
        let mut m = crate::world::flags::AiPolicyMemory::new();
        for (k, v) in &self.memory {
            m.set(k, *v as f64);
        }
        m
    }
}

/// Decode one authored rule list into typed policy rules (issue #882).
///
/// Shared by the top-level stateless `rule` list and by each state's own
/// `rule` list, so a rule decodes identically wherever it is authored.
fn decode_rules(
    src: &[FineSystemAiRuleToml],
) -> Result<Vec<crate::ai::policy::AiPolicyRule>, String> {
    let mut rules = Vec::with_capacity(src.len());
    for r in src {
        let when = crate::world::flags::parse_predicate(&r.when)?;
        rules.push(crate::ai::policy::AiPolicyRule {
            priority: r.priority,
            channel: r.channel.clone(),
            when,
            verb: decode_verb(r)?,
        });
    }
    Ok(rules)
}

/// Decode one authored rule's `verb` (plus its payload fields) into the typed
/// [`crate::ai::policy::AiPolicyVerb`] (issue #882 extraction; the match body
/// is unchanged from #775–#786).
fn decode_verb(r: &FineSystemAiRuleToml) -> Result<crate::ai::policy::AiPolicyVerb, String> {
    Ok(match r.verb.as_str() {
        CAPTAIN_SET_RED_ALERT_VERB => crate::ai::policy::AiPolicyVerb::SetRedAlert(r.value),
        // Helm continuous-actuator mode verbs (issue #779): value-less;
        // the `value` field is ignored — the magnitude lives in the
        // planner fact, not the policy.
        HELM_ACTUATE_DESIRED_TRAVEL_VERB => crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel,
        HELM_ACTUATE_DESIRED_FACING_VERB => crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing,
        // The frozen-heading Steering mode verb (issue #883): also value-less —
        // the heading is host-written private memory, not an authored constant.
        HELM_HOLD_COMMITTED_HEADING_VERB => crate::ai::policy::AiPolicyVerb::HoldCommittedHeading,
        // Helm secondary-actuator mode verbs (issue #780): value-less,
        // like the travel-axis verbs above.
        HELM_ACTUATE_LATERAL_THRUST_VERB => crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
        HELM_ACTUATE_VERTICAL_THRUST_VERB => crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
        HELM_ENGAGE_IMPULSE_VERB => crate::ai::policy::AiPolicyVerb::EngageImpulse,
        HELM_ENGAGE_BOOST_VERB => crate::ai::policy::AiPolicyVerb::EngageBoost,
        // Weapon-bank action verbs (issue #781): value-less, like the
        // helm mode verbs. The `value` field is ignored — the target and
        // firing bank come from the host context, not the policy.
        PHASER_FIRE_VERB => crate::ai::policy::AiPolicyVerb::FirePhaser,
        BLASTER_FIRE_VERB => crate::ai::policy::AiPolicyVerb::FireBlaster,
        // Torpedo tube + magazine action verbs (issue #782): value-less,
        // like the weapon-bank verbs. The `value` field is ignored — the
        // tube, volley target, combat lock, and magazine come from the
        // host context, not the policy.
        TORPEDO_LOAD_VERB => crate::ai::policy::AiPolicyVerb::LoadTorpedo,
        TORPEDO_LAUNCH_VERB => crate::ai::policy::AiPolicyVerb::LaunchTorpedo,
        TORPEDO_MAGAZINE_GRANT_VERB => crate::ai::policy::AiPolicyVerb::GrantTorpedoRound,
        // Shields focus action verb (issue #783): value-less, like the
        // weapon-bank verbs. The `value` field is ignored — which of the
        // four arcs is focused comes from the retained ranking kernel in
        // the host context, not the policy.
        SHIELD_FOCUS_VERB => crate::ai::policy::AiPolicyVerb::FocusShieldArc,
        // Power group allocation verb (issue #784): the FIRST verb to
        // carry a magnitude. The absolute target level is the authored
        // per-rule `level` payload, never an inline Rust number.
        POWER_SET_ALLOCATION_VERB => {
            crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(r.level)
        }
        // Comms dialogue-response verb (issue #786): the SECOND
        // value-carrying verb. Only the response INDEX rides the verb —
        // WHICH message is being answered comes from the host context.
        COMMS_RESPOND_VERB => crate::ai::policy::AiPolicyVerb::RespondToMessage(r.response_index),
        other => return Err(format!("unknown ai policy verb '{other}'")),
    })
}

/// The canonical default Captain Red Alert policy synthesised for ships that
/// do not author `[captain_console.ai]` (issue #775).
///
/// Two rules on the single `red_alert` channel: raise Red Alert while the ship
/// has been in combat within the authored `combat_window_secs`, otherwise
/// stand down. Equivalent behaviour to the retired hardcoded `CaptainAi`, but
/// now data-shaped and with the window as a named parameter.
pub fn default_captain_ai_config() -> FineSystemAiConfigToml {
    let mut param = std::collections::HashMap::new();
    param.insert(
        "combat_window_secs".to_string(),
        DEFAULT_CAPTAIN_COMBAT_WINDOW_SECS,
    );
    FineSystemAiConfigToml {
        idle: false,
        param,
        rule: vec![
            FineSystemAiRuleToml {
                priority: 10,
                channel: CAPTAIN_RED_ALERT_CHANNEL.to_string(),
                when: "fact(secs_since_combat) < param(combat_window_secs)".to_string(),
                verb: CAPTAIN_SET_RED_ALERT_VERB.to_string(),
                value: true,
                level: 0,
                response_index: 0,
            },
            FineSystemAiRuleToml {
                priority: 0,
                channel: CAPTAIN_RED_ALERT_CHANNEL.to_string(),
                when: "true".to_string(),
                verb: CAPTAIN_SET_RED_ALERT_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            },
        ],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default Comms dialogue-response policy synthesised for ships
/// that do not author `[comms_console.ai]` (issue #786).
///
/// BASELINE PRESERVATION. The retired behaviour was a three-line stub inside
/// `handle_comms_channel2`: `if policy.operate_ai && !responses.is_empty() {
/// inbox.record_response(&id, 0) }` — an unconditional "always take the first
/// response" that wrote the inbox DIRECTLY, bypassing admission and bypassing
/// `handle_respond_to_message` entirely (so no trigger action ever fired and no
/// follow-up ever advanced). This single rule reproduces exactly that DECISION
/// — priority 0, `when "true"`, `response_index = 0` — while the emission now
/// travels through `emit_ai_command` → `AdmittedCommands` → the real router, so
/// an AI answer fires the response's actions and advances its follow-up just
/// like a human answer (AGENTS.md rule #6).
///
/// # Why the rule is GUARDED rather than `when = "true"`
///
/// The retired stub ran only on channel-2 ARRIVAL, so it could not repeat and
/// its sender was present by construction. This policy is re-resolved every
/// tick against every open dialogue, so an unconditional rule would re-emit a
/// response the router rejects, forever:
///
///   - `fact(sender_in_range) > 0` — `handle_respond_to_message` refuses a
///     response whose sender has left comms range. Without this the AI re-emits
///     the doomed `RespondToMessage` (re-flashing the officer's rejection) every
///     tick until the sender returns. Baseline-preserving: the retired stub's
///     sender was always in range.
///   - `fact(comms_available) > 0` — the AC2 system-availability gate, read off
///     `EntitySystemHull`: a Disabled or Destroyed Comms system stops the ship
///     ANSWERING as well as hailing.
///
/// An author gates or re-points it without Rust: raise a higher-priority rule
/// guarded on `fact(is_urgent)`, `fact(response_count)`, a scenario flag, …, or
/// declare `idle = true` to make the Comms AI answer nothing at all.
pub fn default_comms_response_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: COMMS_RESPOND_CHANNEL.to_string(),
            when: "fact(comms_available) > 0 and fact(sender_in_range) > 0".to_string(),
            verb: COMMS_RESPOND_VERB.to_string(),
            value: false,
            level: 0,
            response_index: DEFAULT_COMMS_RESPONSE_INDEX,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default Engines (longitudinal thrust) policy synthesised for
/// ships that do not author `[helm_console.engines_ai]` (issue #779).
///
/// One unconditional rule on the `longitudinal` channel: always actuate the
/// planner's desired travel. This reproduces the pre-#779 behaviour, where
/// `ai_helm_thrust` emitted `SetThrust` every tick — but now the *decision* to
/// actuate flows through a data-authored policy verb a designer can gate, rather
/// than a hardcoded unconditional branch. The continuous magnitude still comes
/// from `DesiredMotion`, so no thrust value is pinned in Rust.
pub fn default_engines_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_LONGITUDINAL_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: HELM_ACTUATE_DESIRED_TRAVEL_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default Steering (yaw) policy synthesised for ships that do
/// not author `[helm_console.steering_ai]` (issue #779). Mirror of
/// [`default_engines_ai_config`] on the `yaw` channel: always actuate the
/// planner's desired facing.
pub fn default_steering_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_YAW_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: HELM_ACTUATE_DESIRED_FACING_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default Lateral Thrust policy synthesised for ships that do not
/// author `[helm_console.lateral_ai]` (issue #780).
///
/// One unconditional rule on the `lateral` channel: always actuate. This
/// reproduces the pre-#780 baseline, where `ai_helm_lateral_thrust` ran the dodge
/// (and docking translation) every tick — the DECISION to actuate now flows
/// through a data-authored policy verb a designer can gate, while the continuous
/// dodge magnitude still comes from the shared hazard surface.
pub fn default_lateral_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_LATERAL_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: HELM_ACTUATE_LATERAL_THRUST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default Vertical Thrust policy synthesised for ships that do not
/// author `[helm_console.vertical_ai]` (issue #780).
///
/// One unconditional rule on the `vertical` channel: always actuate. Baseline
/// preserving — the pre-#780 `ai_helm_vertical_thrust` ran the bounded/full-3D
/// climb-and-return every tick, gated only on the authored `VerticalMovementMode`
/// (which stays a host-side capability gate, not a policy scalar). A `Planar`
/// hull still takes no vertical component because the host zeroes it regardless
/// of the policy verb.
pub fn default_vertical_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_VERTICAL_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: HELM_ACTUATE_VERTICAL_THRUST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default Impulse policy synthesised for ships that do not author
/// `[helm_console.impulse_ai]` (issue #780).
///
/// One unconditional rule on the `impulse` channel: the policy always PERMITS the
/// impulse manoeuvre. This preserves the pre-#780 baseline exactly, because the
/// engage-vs-cancel decision itself is still made host-side from the authored
/// doctrine `use_impulse` fact and the `decide_impulse` geometry — the policy
/// verb is an additional authored gate layered on top, defaulting to "permit".
pub fn default_impulse_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_IMPULSE_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: HELM_ENGAGE_IMPULSE_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default Boost policy synthesised for ships that do not author
/// `[helm_console.boost_ai]` (issue #780).
///
/// An explicit **idle** declaration: today no AI ever engages boost, so the
/// baseline-preserving default is "takes no AI boost action". A hull opts in by
/// authoring `[helm_console.boost_ai]` with a rule on the `boost` channel; until
/// then `ai_helm_boost` resolves `None` every tick and emits nothing. Idle is a
/// legal, distinct-from-silence declaration accepted by validation.
pub fn default_boost_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: true,
        param: std::collections::HashMap::new(),
        rule: Vec::new(),
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default phaser-bank open-fire policy synthesised for AI-capable
/// phaser banks that do not author an inline `ai` block (issue #781).
///
/// One unconditional rule on the `phaser_fire` channel: always fire. This
/// reproduces the pre-#781 baseline exactly, where a bank auto-fired whenever the
/// host found it off-cooldown with the target in range and arc — the host still
/// enforces all of those readiness gates, and the DECISION to open fire now flows
/// through a data-authored policy verb a designer can gate (mirrors
/// [`default_engines_ai_config`]). No fire threshold/range/arc/cooldown is pinned
/// in the verb; those stay TOML on the bank config. An explicit `idle` is the
/// opt-out (AC1).
pub fn default_phaser_bank_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: PHASER_FIRE_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: PHASER_FIRE_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default Shields focus policy synthesised for ships that do not
/// author `[shields_console.ai_policy]` (issue #783).
///
/// Two rules on the `shield_focus` channel, both emitting the value-less
/// `focus_shield_arc` verb: a priority-10 DAMAGE rule that fires when the most
/// concentrated arc's recent incoming-damage share reaches the authored
/// `damage_pct_threshold`, and a priority-0 IMBALANCE FALLBACK rule guarded
/// `true`. Because the fallback is unconditional, the retained arc-ranking kernel
/// (`tick_shield_focus_ai`) still runs every tick exactly as it did before #783 —
/// so OMITTING the block reproduces today's decisions bit-for-bit (baseline
/// preservation). The kernel owns the 4-way argmax (damage-concentration primary,
/// health-imbalance fallback); this policy owns only the authored numbers and the
/// gate. All four params are seeded from the retained typed `default_shields_ai_*`
/// values so the kernel reads the same windows/thresholds it always did. An
/// explicit `idle` is the opt-out.
pub fn default_shields_focus_ai_config() -> FineSystemAiConfigToml {
    let mut param = std::collections::HashMap::new();
    param.insert(
        SHIELD_FOCUS_DAMAGE_WINDOW_PARAM.to_string(),
        default_shields_ai_damage_window_secs(),
    );
    param.insert(
        SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM.to_string(),
        default_shields_ai_min_damage_window_secs(),
    );
    param.insert(
        SHIELD_FOCUS_DAMAGE_PCT_PARAM.to_string(),
        default_shields_ai_damage_pct_threshold(),
    );
    param.insert(
        SHIELD_FOCUS_HEALTH_RATIO_PARAM.to_string(),
        default_shields_ai_health_ratio_threshold(),
    );
    FineSystemAiConfigToml {
        idle: false,
        param,
        rule: vec![
            FineSystemAiRuleToml {
                priority: 10,
                channel: SHIELD_FOCUS_CHANNEL.to_string(),
                // The concentration fact is a percentage (0–100) so it compares
                // directly against the authored `damage_pct_threshold` — the
                // predicate grammar has no arithmetic, so the host seeds the fact
                // already scaled.
                when: format!(
                    "fact(recent_damage_pct_max) >= param({SHIELD_FOCUS_DAMAGE_PCT_PARAM})"
                ),
                verb: SHIELD_FOCUS_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            },
            FineSystemAiRuleToml {
                priority: 0,
                channel: SHIELD_FOCUS_CHANNEL.to_string(),
                when: "true".to_string(),
                verb: SHIELD_FOCUS_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            },
        ],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default Power allocation policy synthesised for ships that do
/// not author `[power.ai_policy]` (issue #784).
///
/// Reproduces the retired stateful engine's two behaviours as inline stateless
/// per-group rules with reserve guards, so OMITTING the block preserves baseline
/// behaviour:
///   - `helm`: a priority-10 rule that elevates helm power to level 3 while
///     forward thrust is sustained AND the battery is at or above the authored
///     `min_reserve_helm` floor, with a priority-0 `true` fallback that holds
///     helm at its baseline level 2.
///   - `weapons`: the same shape keyed on red alert with a `min_reserve_weapons`
///     floor.
///
/// Every rule declares a MINIMUM PERMITTED BATTERY RESERVE (AC2): the elevate
/// rules gate on `fact(battery_pct) >= param(min_reserve_*)`, and the baseline
/// rules — which only ever LOWER allocation — carry a `0` reserve param so they
/// too declare one while never being able to cause a brownout. Because an
/// elevate guard cannot fire below its reserve, allocation never rises when the
/// battery can't sustain it (AC5 brownout avoidance) — replacing the retired
/// global emergency exception with per-rule reserve guards.
///
/// The magnitudes (3 elevated, 2 baseline) and thresholds are parse-time
/// defaults + vocabulary in Rust (sanctioned by AGENTS.md rule #11); a ship that
/// authors `[power.ai_policy]` overrides them entirely with its own `level`
/// payloads and `param` reserves. Groups the ship authors but this default does
/// not name (e.g. `sensors`, `ops`) resolve to `None` and hold their seeded
/// level.
pub fn default_power_ai_config() -> FineSystemAiConfigToml {
    let mut param = std::collections::HashMap::new();
    param.insert(
        POWER_THRUST_THRESHOLD_PARAM.to_string(),
        DEFAULT_POWER_THRUST_THRESHOLD,
    );
    param.insert(
        POWER_HELM_RESERVE_PARAM.to_string(),
        DEFAULT_POWER_HELM_RESERVE,
    );
    param.insert(
        POWER_WEAPONS_RESERVE_PARAM.to_string(),
        DEFAULT_POWER_WEAPONS_RESERVE,
    );
    // A shared zero-reserve param the lowering baseline rules reference so every
    // rule declares a reserve (AC2) without ever gating a de-allocation.
    param.insert(POWER_BASELINE_RESERVE_PARAM.to_string(), 0.0);
    let helm = crate::modifiers::power_system::HELM_POWER_GROUP;
    let weapons = crate::modifiers::power_system::WEAPONS_POWER_GROUP;
    FineSystemAiConfigToml {
        idle: false,
        param,
        rule: vec![
            FineSystemAiRuleToml {
                priority: 10,
                channel: helm.to_string(),
                when: format!(
                    "fact({POWER_THRUST_FACT}) >= param({POWER_THRUST_THRESHOLD_PARAM}) \
                     and fact({POWER_BATTERY_PCT_FACT}) >= param({POWER_HELM_RESERVE_PARAM})"
                ),
                verb: POWER_SET_ALLOCATION_VERB.to_string(),
                value: false,
                level: DEFAULT_POWER_ELEVATED_LEVEL,
                response_index: 0,
            },
            FineSystemAiRuleToml {
                priority: 0,
                channel: helm.to_string(),
                when: format!(
                    "fact({POWER_BATTERY_PCT_FACT}) >= param({POWER_BASELINE_RESERVE_PARAM})"
                ),
                verb: POWER_SET_ALLOCATION_VERB.to_string(),
                value: false,
                level: DEFAULT_POWER_BASELINE_LEVEL,
                response_index: 0,
            },
            FineSystemAiRuleToml {
                priority: 10,
                channel: weapons.to_string(),
                when: format!(
                    "fact({POWER_RED_ALERT_FACT}) > 0 \
                     and fact({POWER_BATTERY_PCT_FACT}) >= param({POWER_WEAPONS_RESERVE_PARAM})"
                ),
                verb: POWER_SET_ALLOCATION_VERB.to_string(),
                value: false,
                level: DEFAULT_POWER_ELEVATED_LEVEL,
                response_index: 0,
            },
            FineSystemAiRuleToml {
                priority: 0,
                channel: weapons.to_string(),
                when: format!(
                    "fact({POWER_BATTERY_PCT_FACT}) >= param({POWER_BASELINE_RESERVE_PARAM})"
                ),
                verb: POWER_SET_ALLOCATION_VERB.to_string(),
                value: false,
                level: DEFAULT_POWER_BASELINE_LEVEL,
                response_index: 0,
            },
        ],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default blaster-bank open-fire policy synthesised for AI-capable
/// blaster banks that do not author an inline `ai` block (issue #781). Mirror of
/// [`default_phaser_bank_ai_config`] on the `blaster_fire` channel: always fire,
/// with the host still enforcing availability, cooldown, range, arc, and target
/// validity before the volley starts.
pub fn default_blaster_bank_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: BLASTER_FIRE_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: BLASTER_FIRE_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default torpedo-tube policy synthesised for AI-capable tubes
/// that do not author an inline `ai` block (issue #782).
///
/// Two unconditional rules — one on the `torpedo_load` channel, one on the
/// `torpedo_launch` channel — so a tube with no authored policy keeps loading and
/// launching exactly as before (AC1 baseline). The host still enforces loaded
/// state, magazine availability, target validity, range, and arc; the DECISION to
/// load or launch now flows through data-authored policy verbs a designer can
/// gate. No count/range/arc is pinned in the verbs; those stay TOML on the tube.
/// An explicit `idle` is the opt-out.
pub fn default_torpedo_tube_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![
            FineSystemAiRuleToml {
                priority: 0,
                channel: TORPEDO_LOAD_CHANNEL.to_string(),
                when: "true".to_string(),
                verb: TORPEDO_LOAD_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            },
            FineSystemAiRuleToml {
                priority: 0,
                channel: TORPEDO_LAUNCH_CHANNEL.to_string(),
                when: "true".to_string(),
                verb: TORPEDO_LAUNCH_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            },
        ],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// The canonical default torpedo-magazine policy synthesised for a shared
/// magazine that does not author an inline `ai` block (issue #782). One
/// unconditional rule on the `torpedo_magazine_grant` channel: always grant. This
/// reproduces the pre-#782 baseline where every claim that passed the offline +
/// non-empty gates was granted; the offline gate remains the hard authority and
/// this policy is a data-authored arbiter layered on top.
pub fn default_torpedo_magazine_ai_config() -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: TORPEDO_MAGAZINE_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: TORPEDO_MAGAZINE_GRANT_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
}

/// Validate an inline fine-system AI policy before world activation
/// (issues #775, #882), mirroring [`validate_phaser_banks`] et al.
///
/// Rejects (stateless path, issue #775):
///   - a silent declaration (neither `idle` nor any `rule` nor any `state`),
///   - a contradictory declaration (`idle = true` alongside rules/states),
///   - a MIXED declaration: top-level `rule`s alongside `state`s (issue #883) —
///     a machine resolves only in-state, so those rules are silently dead,
///   - an unparseable `when` guard (reusing `parse_predicate`'s diagnostic),
///   - an unknown output `channel` or `verb`,
///   - a `param(...)` reference to a parameter the author never declared.
///
/// Additionally rejects (stateful path, issue #882 AC6):
///   - an `initial_state` naming a state that was never declared (and a
///     `state` list with no `initial_state` at all),
///   - a transition whose `to` names a state that was never declared,
///   - duplicate state ids,
///   - an unreachable state (no inbound transition and not the initial state),
///   - a `memory(...)` or `state_time` reference in a STATELESS policy, and a
///     `memory(...)` reference to a slot the author never declared.
pub fn validate_fine_system_ai_policy(
    cfg: &FineSystemAiConfigToml,
    valid_channels: &[&str],
    valid_verbs: &[&str],
) -> Result<(), String> {
    if cfg.idle {
        if !cfg.rule.is_empty() {
            return Err("ai policy declares idle = true but also carries rules".into());
        }
        if !cfg.state.is_empty() {
            return Err("ai policy declares idle = true but also carries states".into());
        }
        return Ok(());
    }
    if cfg.rule.is_empty() && cfg.state.is_empty() {
        return Err(
            "ai policy is empty: declare at least one rule or state, or set idle = true".into(),
        );
    }
    // A policy is EITHER stateless (top-level `rule`) or a machine (`state`),
    // never both (issue #883, carried forward from the #882 review). A machine
    // resolves EXCLUSIVELY through `resolve_channel_in_state`, so top-level
    // rules on a stateful policy are silently dead code — and worse, the
    // `stateful` flag below makes a `memory(...)` reference inside one VALIDATE
    // while always evaluating false at runtime, because the stateless scan hands
    // `best_in` an empty memory bag. Both failures are silent, which is exactly
    // the class of defect #882's blocking bug belonged to, so the shape is
    // rejected at load rather than merely discouraged.
    if !cfg.rule.is_empty() && !cfg.state.is_empty() {
        return Err(format!(
            "ai policy declares both top-level rules ({}) and states ({}): a stateful \
             policy resolves only inside its current state, so the top-level rules \
             would never fire. Move them into the state(s) that should own them, or \
             delete the state machine",
            cfg.rule.len(),
            cfg.state.len()
        ));
    }
    let stateful = !cfg.state.is_empty();

    // ── Per-rule checks, run unchanged over the top-level rules and over each
    // state's own rules (issue #882 extends the loop's reach, not its body).
    let check_rule = |what: &str, r: &FineSystemAiRuleToml| -> Result<(), String> {
        if !valid_channels.contains(&r.channel.as_str()) {
            return Err(format!(
                "ai policy {what} has unknown channel '{}' (valid: {valid_channels:?})",
                r.channel
            ));
        }
        if !valid_verbs.contains(&r.verb.as_str()) {
            return Err(format!(
                "ai policy {what} has unknown verb '{}' (valid: {valid_verbs:?})",
                r.verb
            ));
        }
        let pred = crate::world::flags::parse_predicate(&r.when)
            .map_err(|e| format!("ai policy {what} has invalid `when` expression: {e}"))?;
        check_policy_predicate(cfg, stateful, &pred, what)
    };
    for (idx, r) in cfg.rule.iter().enumerate() {
        check_rule(&format!("rule {idx}"), r)?;
    }

    if !stateful {
        return Ok(());
    }

    // ── State-graph checks (issue #882 AC6) ─────────────────────────────────
    let mut seen: Vec<&str> = Vec::with_capacity(cfg.state.len());
    for s in &cfg.state {
        if seen.contains(&s.id.as_str()) {
            return Err(format!("ai policy declares duplicate state id '{}'", s.id));
        }
        seen.push(&s.id);
    }
    let Some(initial) = cfg.initial_state.as_deref() else {
        return Err("ai policy declares states but no `initial_state`".into());
    };
    if !seen.contains(&initial) {
        return Err(format!(
            "ai policy `initial_state` names undeclared state '{initial}' (declared: {seen:?})"
        ));
    }
    for s in &cfg.state {
        for (tidx, t) in s.transition.iter().enumerate() {
            let what = format!("state '{}' transition {tidx}", s.id);
            if !seen.contains(&t.to.as_str()) {
                return Err(format!(
                    "ai policy {what} targets undeclared state '{}' (declared: {seen:?})",
                    t.to
                ));
            }
            let pred = crate::world::flags::parse_predicate(&t.when)
                .map_err(|e| format!("ai policy {what} has invalid `when` expression: {e}"))?;
            check_policy_predicate(cfg, stateful, &pred, &what)?;
        }
        for (idx, r) in s.rule.iter().enumerate() {
            check_rule(&format!("state '{}' rule {idx}", s.id), r)?;
        }
    }
    // Reachability is a FIXPOINT walk from `initial`, following transitions only
    // out of states already known reachable. A single pass over every state's
    // transitions would only catch zero-inbound orphans: a disconnected cluster
    // (`initial = a`; `b -> c`; `c -> b`) is targeted by transitions and would
    // pass, yet nothing can ever enter it. Every transition target is already
    // known to name a declared state by the loop above, so this walk cannot
    // wander off the graph.
    let mut reachable: Vec<&str> = vec![initial];
    let mut frontier: Vec<&str> = vec![initial];
    while let Some(current) = frontier.pop() {
        let Some(s) = cfg.state.iter().find(|s| s.id == current) else {
            continue;
        };
        for t in &s.transition {
            if !reachable.contains(&t.to.as_str()) {
                reachable.push(&t.to);
                frontier.push(&t.to);
            }
        }
    }
    for s in &cfg.state {
        if !reachable.contains(&s.id.as_str()) {
            return Err(format!(
                "ai policy declares unreachable state '{}': it is neither the \
                 initial state nor the target of any transition",
                s.id
            ));
        }
    }
    Ok(())
}

/// Shared guard-expression checks for a policy predicate (issues #775, #882).
///
/// `param(...)` must be declared; `memory(...)` must be declared AND the policy
/// must be stateful; `state_time` requires a stateful policy. The stateless
/// rejections are AC6's "a memory or state-time reference in a stateless
/// policy" — private state is meaningless without a state machine to own it,
/// and silently reading `false` would be a trap.
fn check_policy_predicate(
    cfg: &FineSystemAiConfigToml,
    stateful: bool,
    pred: &crate::world::flags::Predicate,
    what: &str,
) -> Result<(), String> {
    let mut refs = Vec::new();
    pred.referenced_params(&mut refs);
    for name in refs {
        if !cfg.param.contains_key(&name) {
            return Err(format!(
                "ai policy {what} references undeclared parameter '{name}'"
            ));
        }
    }
    let mut mem_refs = Vec::new();
    pred.referenced_memory(&mut mem_refs);
    if !stateful && !mem_refs.is_empty() {
        return Err(format!(
            "ai policy {what} references memory('{}') but the policy declares no states: \
             private memory requires a stateful policy",
            mem_refs[0]
        ));
    }
    for name in mem_refs {
        if !cfg.memory.contains_key(&name) {
            return Err(format!(
                "ai policy {what} references undeclared memory '{name}'"
            ));
        }
    }
    if !stateful && pred.references_state_time() {
        return Err(format!(
            "ai policy {what} references state_time but the policy declares no states: \
             state time requires a stateful policy"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerConfigSection {
    pub capacity: f32,
    pub rates: [f32; 6],
    pub emergency_threshold: f32,
    /// Inline stateless AI policy for the Power reactor fine system (issue
    /// #784), loaded from `[power.ai_policy]`. Replaces the retired stateful
    /// `[power.ai]` engine (`PowerAiConfigToml` + `EngageState` hysteresis).
    /// Each authored `[[power.ai_policy.rule]]` binds a `priority` and a power
    /// GROUP `channel` to a `when` guard and the value-carrying
    /// `set_power_group_allocation` verb (its `level` payload the absolute
    /// target). Every allocation rule declares a minimum battery reserve as a
    /// `param(...)` referenced by its guard (AC2). Absent, the canonical
    /// [`default_power_ai_config`] is synthesised at spawn (baseline
    /// preservation). Validated in [`EntityConfig::from_toml`] against
    /// [`POWER_SET_ALLOCATION_VERB`] and a valid-channel set built dynamically
    /// from the ship's `[power_groups.*]` keys (AC1 — no fixed catalogue).
    #[serde(default)]
    pub ai_policy: Option<FineSystemAiConfigToml>,
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
    /// Inline stateless AI policy for the Shields focus fine system (issue
    /// #783), loaded from `[shields_console.ai_policy]`. Sibling to `ai`: the
    /// typed `ai` knobs remain the fallback default source, while this block —
    /// when authored — carries the same four numbers as `param(...)` entries
    /// (`damage_window_secs`, `min_damage_window_secs`, `damage_pct_threshold`,
    /// `health_ratio_threshold`) plus the prioritised rules that gate whether the
    /// retained arc-ranking kernel acts. Absent, the canonical
    /// [`default_shields_focus_ai_config`] is synthesised at spawn (baseline
    /// preservation). Validated in [`crate::entities::config::EntityConfig::from_toml`]
    /// against [`SHIELD_FOCUS_CHANNELS`] / [`SHIELD_FOCUS_VERBS`].
    #[serde(default)]
    pub ai_policy: Option<FineSystemAiConfigToml>,
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
            ai_policy: None,
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
    /// Inline per-system target selector (issue #785). Loaded from
    /// `[repair.selector]`; absent ⇒ the canonical
    /// [`default_repair_target_selector_config`] is synthesised at spawn.
    /// `operate_repair_ai` runs it once per free team to rank the ship's
    /// damaged stations into ordinary admitted `DispatchRepairTeam` inputs.
    ///
    /// This is the first selector block that is NOT inside a `*_console`
    /// section: repair teams are a ship-wide engineering capability whose
    /// tunables already live under `[repair]`, so the selector joins them there
    /// rather than inventing a `[repair_console]` table the wire never uses.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
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
            selector: None,
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
    /// Ship-wide default for `[[torpedoes.tubes]] ai_target_count` — how many
    /// rounds an AI-operated crew keeps loaded in each tube. A per-tube
    /// `ai_target_count` overrides it; when both are absent each tube falls
    /// back to its own `volley_max`.
    #[serde(default)]
    pub ai_volley_target: Option<u32>,
    /// Inline stateless AI policy for the shared magazine's grant decision
    /// (issue #782, AC1). When authored it is validated at content load and
    /// resolved inside `handle_torpedo_magazine_inter_system` right before the
    /// authoritative `claim_magazine_round`; when absent the canonical
    /// [`default_torpedo_magazine_ai_config`] (unconditional grant) is
    /// synthesised at spawn so baseline claim behaviour is preserved. The offline
    /// gate stays the hard authority; this is a data-authored arbiter on top.
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
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
            ai_volley_target: None,
            ai: None,
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
            ai_volley_target: self.ai_volley_target,
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
    /// Inline per-system target selector (issue #778). Loaded from
    /// `[navigation_console.selector]`; absent ⇒ the canonical
    /// [`default_navigation_target_selector_config`] is synthesised at spawn.
    /// `operate_navigation_ai` runs it to rank objective destinations and
    /// eligible chart contacts into the shared Waypoint.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
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
    /// AI tuning parameters for the Sensors frequency-hint controller.
    /// Loaded from `[sensors_console.ai]`.
    #[serde(default)]
    pub ai: Option<SensorsAiConfigToml>,
    /// Inline per-system target selector (issue #776). Loaded from
    /// `[sensors_console.selector]`; absent ⇒ the canonical
    /// [`default_sensors_target_selector_config`] is synthesised at spawn.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
}

/// AI tuning parameters for the Sensors frequency-hint controller
/// (`console_ai::tick_frequency_hint`, issue #692).
///
/// Loaded from `[sensors_console.ai]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorsAiConfigToml {
    /// Delay (seconds) between a target lock and the AI-driven Sensors
    /// operator emitting a `FrequencyHint` coordination message to Tactical.
    #[serde(default = "default_sensors_ai_frequency_hint_delay_secs")]
    pub frequency_hint_delay_secs: f32,
}

fn default_sensors_ai_frequency_hint_delay_secs() -> f32 {
    3.0
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
    /// Optional helm capability declaration (`[helm_capability]`).
    /// Describes vertical movement mode and impulse steering policy.
    #[serde(default)]
    pub helm_capability: Option<HelmCapabilityConfig>,
    pub weapons_console: Option<WeaponsConsoleConfig>,
    pub engineering_console: Option<EngineeringConsoleConfig>,
    pub captain_console: Option<CaptainConsoleConfig>,
    /// Comms CONSOLE config (issue #786): the hail `selector` and the
    /// dialogue-response `ai` policy. Distinct from the `comms` field below,
    /// which is the per-entity comms RANGE.
    #[serde(default)]
    pub comms_console: Option<CommsConsoleConfig>,
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
    /// Ship audio: filenames and tuning for the ambient bed, engine, blaster,
    /// phaser loop, and forcefield. Server-only playback — the host page's JS
    /// builds its audio graph from this. `None` ⇒ the ship is silent.
    #[serde(default)]
    pub audio: Option<crate::audio_config::ShipAudioConfig>,
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
    /// Textured planet visual. Takes precedence over `[mesh]` when both are
    /// present (`[mesh]` stays as a fallback for headless/editor contexts).
    #[serde(default)]
    pub planet: Option<PlanetConfig>,
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
        if !config.shield_arcs.is_empty()
            || ship_config_toml.is_some()
            || config.behaviour.is_some()
        {
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

            // Provision a mandatory AI-only Red Alert capability for every
            // behaviour-driven NPC ship (issue #749). Behaviour-driven ships run
            // an `operate_captain_ai` loop that can raise its own Red Alert, but
            // only when the Red Alert system's control source resolves to `Ai` —
            // which requires the system to be *listed* on the ship. Authors who
            // omit an explicit `[[system]] kind = "red_alert"` block (every Harrow
            // and pirate NPC) would otherwise leave it defaulting to `Human`,
            // silently blocking the AI captain from ever going to Red Alert.
            //
            // Ownerless (`station = None`) + `ai_only = true`, mirroring how NPC
            // shield-arc / phaser / reactor systems are synthesised above and
            // satisfying `OwnerlessSystemWithoutAiOnly`. Idempotent: an explicit
            // authored red_alert system (the Alliance ships) is left untouched.
            if config.behaviour.is_some()
                && !ship_config
                    .systems
                    .iter()
                    .any(|s| s.kind == crate::system_registry::RED_ALERT_KIND)
            {
                ship_config
                    .systems
                    .push(crate::ship::config::SystemInstanceConfig {
                        id: crate::messages::SystemId(
                            crate::system_registry::RED_ALERT_SYSTEM_ID.into(),
                        ),
                        kind: crate::system_registry::RED_ALERT_KIND.into(),
                        station: None,
                        ai_only: true,
                        power_group: None,
                        marker: None,
                        config: None,
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

        // Validate an authored inline Captain AI policy before world
        // activation (issue #775). The Captain's single AI-capable fine system
        // (Red Alert) drives one output channel with one verb. Structural,
        // expression, channel, verb, and parameter errors are deterministic
        // content errors surfaced through serde so the entity fails to load.
        if let Some(ai) = config.captain_console.as_ref().and_then(|c| c.ai.as_ref()) {
            validate_fine_system_ai_policy(
                ai,
                &[CAPTAIN_RED_ALERT_CHANNEL],
                &[CAPTAIN_SET_RED_ALERT_VERB],
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate authored inline Engines/Steering AI policies before world
        // activation (issue #779). These are the first continuous fine
        // actuators on the #775 spine: Engines drives the `longitudinal`
        // channel, Steering the `yaw` channel, each with its own single mode
        // verb. Unknown channels/verbs, unparseable guards, and undeclared
        // parameter references fail the entity load here, before any live tick.
        if let Some(hc) = config.helm_console.as_ref() {
            if let Some(ai) = hc.engines_ai.as_ref() {
                validate_fine_system_ai_policy(
                    ai,
                    &[HELM_LONGITUDINAL_CHANNEL],
                    &[HELM_ACTUATE_DESIRED_TRAVEL_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.steering_ai.as_ref() {
                validate_fine_system_ai_policy(ai, &[HELM_YAW_CHANNEL], HELM_STEERING_VERBS)
                    .map_err(SerdeError::custom)?;
            }
            // Secondary helm fine-actuator policies (issue #780): each drives its
            // own single channel with its own single mode verb. Wrong-axis verbs,
            // unknown channels, unparseable guards, and undeclared parameter
            // references fail the entity load here, before any live tick.
            if let Some(ai) = hc.lateral_ai.as_ref() {
                validate_fine_system_ai_policy(
                    ai,
                    &[HELM_LATERAL_CHANNEL],
                    &[HELM_ACTUATE_LATERAL_THRUST_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.vertical_ai.as_ref() {
                validate_fine_system_ai_policy(
                    ai,
                    &[HELM_VERTICAL_CHANNEL],
                    &[HELM_ACTUATE_VERTICAL_THRUST_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.impulse_ai.as_ref() {
                validate_fine_system_ai_policy(
                    ai,
                    &[HELM_IMPULSE_CHANNEL],
                    &[HELM_ENGAGE_IMPULSE_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.boost_ai.as_ref() {
                validate_fine_system_ai_policy(
                    ai,
                    &[HELM_BOOST_CHANNEL],
                    &[HELM_ENGAGE_BOOST_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
        }

        // Validate authored inline per-bank weapon AI policies before world
        // activation (issue #781). Each AI-capable phaser and blaster bank may
        // declare an inline `ai` block driving its single `phaser_fire` /
        // `blaster_fire` channel with its single fire verb. Unknown
        // channels/verbs, unparseable guards, and undeclared parameter
        // references fail the entity load here, before any live tick — mirroring
        // the helm validation block above.
        if let Some(wc) = config.weapons_console.as_ref() {
            for bank in &wc.phaser_banks {
                if let Some(ai) = bank.ai.as_ref() {
                    validate_fine_system_ai_policy(ai, PHASER_BANK_CHANNELS, PHASER_BANK_VERBS)
                        .map_err(SerdeError::custom)?;
                }
            }
            for bank in &wc.blaster_banks {
                if let Some(ai) = bank.ai.as_ref() {
                    validate_fine_system_ai_policy(ai, BLASTER_BANK_CHANNELS, BLASTER_BANK_VERBS)
                        .map_err(SerdeError::custom)?;
                }
            }
        }

        // Validate authored inline torpedo tube + magazine AI policies before
        // world activation (issue #782). Each AI-capable tube may declare an
        // inline `ai` block driving its `torpedo_load` / `torpedo_launch`
        // channels; the shared magazine may declare its own `ai` block driving
        // the `torpedo_magazine_grant` channel. Unknown channels/verbs (a launch
        // verb on the magazine channel, a grant verb on a tube), unparseable
        // guards, and undeclared parameter references fail the entity load here,
        // before any live tick — mirroring the weapon-bank validation above.
        if let Some(tc) = config.torpedoes.as_ref() {
            for tube in &tc.tubes {
                if let Some(ai) = tube.ai.as_ref() {
                    validate_fine_system_ai_policy(ai, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS)
                        .map_err(SerdeError::custom)?;
                }
            }
            if let Some(ai) = tc.ai.as_ref() {
                validate_fine_system_ai_policy(
                    ai,
                    TORPEDO_MAGAZINE_CHANNELS,
                    TORPEDO_MAGAZINE_VERBS,
                )
                .map_err(SerdeError::custom)?;
            }
        }

        // Validate an authored inline Shields focus AI policy before world
        // activation (issue #783). The Shields fine system drives its single
        // `shield_focus` channel with its single value-less `focus_shield_arc`
        // verb. A wrong-axis verb (e.g. a fire verb), an unknown channel, an
        // unparseable guard, and undeclared `param(...)` references fail the
        // entity load here, before any live tick — mirroring the helm/weapon
        // validation blocks above.
        if let Some(ai) = config
            .shields_console
            .as_ref()
            .and_then(|sc| sc.ai_policy.as_ref())
        {
            validate_fine_system_ai_policy(ai, SHIELD_FOCUS_CHANNELS, SHIELD_FOCUS_VERBS)
                .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Power allocation policy before world
        // activation (issue #784). Unlike every fine system above, the Power
        // reactor has NO fixed channel catalogue: its output channels are the
        // ship's AUTHORED `[power_groups.*]` keys, so the valid-channel set is
        // built dynamically from ship data here (AC1 "no fixed system
        // catalogue"). The single verb is the value-carrying
        // `set_power_group_allocation`. A rule targeting a non-authored group,
        // a wrong verb, an unparseable guard, or an undeclared `param(...)`
        // reserve fails the entity load here, before any live tick.
        if let Some(ai) = config.power.as_ref().and_then(|p| p.ai_policy.as_ref()) {
            let valid_channels: Vec<&str> = config
                .ship_config
                .as_ref()
                .map(|sc| sc.power_groups.keys().map(|g| g.0.as_str()).collect())
                .unwrap_or_default();
            validate_fine_system_ai_policy(ai, &valid_channels, &[POWER_SET_ALLOCATION_VERB])
                .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Sensors target selector before world
        // activation (issue #776). Unknown sources, unparseable
        // eligibility/score expressions, and undeclared `param(...)` references
        // are deterministic content errors surfaced through serde so the entity
        // fails to load before any live tick evaluates it.
        if let Some(sel) = config
            .sensors_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector(sel, SENSORS_SELECTOR_SOURCES)
                .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Tactical target selector before world
        // activation (issue #777). Same deterministic content-error surface as
        // the Sensors selector above — the Tactical host is the sole writer of
        // the authoritative weapons target, so a malformed ranking must fail
        // the entity load rather than reach a live tick.
        if let Some(sel) = config
            .weapons_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector(sel, TACTICAL_SELECTOR_SOURCES)
                .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Navigation target selector before world
        // activation (issue #778). Same deterministic content-error surface as
        // the Sensors/Tactical selectors above — the Navigation host emits the
        // authoritative waypoint through admission from this ranking, so a
        // malformed selector must fail the entity load rather than reach a live
        // tick.
        if let Some(sel) = config
            .navigation_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector(sel, NAVIGATION_SELECTOR_SOURCES)
                .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Repair target selector before world
        // activation (issue #785). Same deterministic content-error surface as
        // the Sensors/Tactical/Navigation selectors above — `operate_repair_ai`
        // emits admitted `DispatchRepairTeam` inputs from this ranking, so a
        // malformed selector must fail the entity load rather than reach a live
        // tick. `[repair.selector]` is the first selector block outside a
        // `*_console` section.
        if let Some(sel) = config.repair.as_ref().and_then(|c| c.selector.as_ref()) {
            validate_fine_system_ai_selector(sel, REPAIR_SELECTOR_SOURCES)
                .map_err(SerdeError::custom)?;
        }

        // Validate the authored Comms console AI blocks before world activation
        // (issue #786). Comms is the first system to author BOTH machines, so
        // both validators run: the hail SELECTOR against its registered
        // candidate sources, and the dialogue-response POLICY against the single
        // `comms_respond` channel and its `respond_to_message` verb. Both emit
        // admitted commands into the shared comms router, so a malformed block
        // must fail the entity load rather than reach a live tick.
        if let Some(sel) = config
            .comms_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector(sel, COMMS_SELECTOR_SOURCES)
                .map_err(SerdeError::custom)?;
        }
        if let Some(ai) = config.comms_console.as_ref().and_then(|c| c.ai.as_ref()) {
            validate_fine_system_ai_policy(ai, COMMS_RESPOND_CHANNELS, COMMS_RESPOND_VERBS)
                .map_err(SerdeError::custom)?;
        }

        // Clamp target_speed in every doctrine entry.
        if let Some(ref mut b) = config.behaviour {
            for d in &mut b.doctrine {
                d.target_speed = d.target_speed.clamp(0.0, 1.0);
            }
        }

        Ok(config)
    }

    /// Serialize this config to a `toml::Value` **losslessly** — re-emitting the
    /// `[[station]]` / `[[system]]` / `[power_groups]` and `[[shield_arc]]`
    /// blocks that [`from_toml`](Self::from_toml) assembles into the
    /// `#[serde(skip)]` [`ship_config`](Self::ship_config) /
    /// [`shield_arcs`](Self::shield_arcs) fields.
    ///
    /// # Why this exists (issue #838)
    ///
    /// The override-merge path (`entity_loader::resolve_entity` and
    /// `world::dispatch::dispatch_spawn_entity`) resolves a `spawn_entity` /
    /// `[[entity]]` override by round-tripping the template through TOML:
    /// `template → toml::Value → merge(overrides) → EntityConfig::from_toml`.
    /// A plain `toml::to_string(&config)` drops `ship_config` and `shield_arcs`
    /// because both are `#[serde(skip)]` (they have no serialized representation
    /// — `from_toml` reconstructs them from the raw blocks at parse time). The
    /// merged string therefore carried **no ship systems at all**, and the
    /// re-parsed config spawned a hull with zero stations, zero weapons, and
    /// nothing under AI control: a world-spawned "hostile" that could never lock
    /// a target or fire. Re-emitting the blocks here makes the round-trip
    /// faithful, so an override preserves the template's whole system suite.
    ///
    /// Synthesized `shield_arc` systems are filtered out of the emitted `system`
    /// array on purpose: `from_toml` re-synthesizes exactly one per
    /// `[[shield_arc]]` block, and emitting both would trip `DuplicateSystemId`.
    pub fn to_toml_value(&self) -> Result<toml::Value, toml::ser::Error> {
        let mut value = toml::Value::try_from(self)?;
        let table = value
            .as_table_mut()
            .expect("EntityConfig always serializes to a TOML table");

        if let Some(ship_config) = &self.ship_config {
            if !ship_config.stations.is_empty() {
                table.insert(
                    "station".to_string(),
                    toml::Value::try_from(&ship_config.stations)?,
                );
            }
            let declared_systems: Vec<&crate::ship::config::SystemInstanceConfig> = ship_config
                .systems
                .iter()
                .filter(|s| s.kind != crate::system_registry::SHIELD_ARC_KIND)
                .collect();
            if !declared_systems.is_empty() {
                table.insert(
                    "system".to_string(),
                    toml::Value::try_from(&declared_systems)?,
                );
            }
            if !ship_config.power_groups.is_empty() {
                table.insert(
                    "power_groups".to_string(),
                    toml::Value::try_from(&ship_config.power_groups)?,
                );
            }
        }

        if !self.shield_arcs.is_empty() {
            table.insert(
                "shield_arc".to_string(),
                toml::Value::try_from(&self.shield_arcs)?,
            );
        }

        Ok(value)
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

    // ── Ship audio tests ─────────────────────────────────────────────────

    /// `EntityConfig` is `deny_unknown_fields`, so an `[audio]` block in a
    /// shipped template would break *every* load of that template — not just
    /// audio — if the field were ever removed. These parse the real files.
    #[test]
    fn player_ship_templates_parse_audio_block() {
        for (name, toml_str) in [
            (
                "alliance_cruiser",
                include_str!("../../assets/entities/alliance_cruiser.toml"),
            ),
            (
                "alliance_destroyer",
                include_str!("../../assets/entities/alliance_destroyer.toml"),
            ),
            (
                "alliance_battleship",
                include_str!("../../assets/entities/alliance_battleship.toml"),
            ),
        ] {
            let config =
                EntityConfig::from_toml(toml_str).unwrap_or_else(|e| panic!("{name}.toml: {e}"));
            let audio = config
                .audio
                .as_ref()
                .unwrap_or_else(|| panic!("{name}.toml must have [audio]"));

            assert_eq!(
                audio.ambient.as_ref().expect("[audio.ambient]").file,
                "assets/sounds/Ambient.mp3",
                "{name}"
            );
            assert_eq!(
                audio.engine.as_ref().expect("[audio.engine]").file,
                "assets/sounds/Engine.mp3",
                "{name}"
            );
            assert_eq!(
                audio.blaster.as_ref().expect("[audio.blaster]").file,
                "assets/sounds/Blaster.mp3",
                "{name}"
            );
            assert_eq!(
                audio
                    .phaser_loop
                    .as_ref()
                    .expect("[audio.phaser_loop]")
                    .file,
                "assets/sounds/PhaserLoop.mp3",
                "{name}"
            );
            assert_eq!(
                audio.forcefield.as_ref().expect("[audio.forcefield]").file,
                "assets/sounds/ForcefieldHit.mp3",
                "{name}"
            );
        }
    }

    /// Preserves the volumes the JS previously hardcoded (`hum.volume = 0.25`,
    /// `engine.volume = thrust * 0.15`), so making them data-driven did not
    /// silently change how the game sounds.
    #[test]
    fn cruiser_audio_preserves_legacy_volumes() {
        let toml_str = include_str!("../../assets/entities/alliance_cruiser.toml");
        let config = EntityConfig::from_toml(toml_str).expect("must parse");
        let audio = config.audio.as_ref().expect("[audio]");
        assert_eq!(audio.ambient.as_ref().unwrap().volume, 0.25);
        assert_eq!(audio.engine.as_ref().unwrap().volume_at_full_thrust, 0.15);
        assert_eq!(audio.engine.as_ref().unwrap().idle_volume, 0.0);
    }

    #[test]
    fn entity_without_audio_block_parses_to_none() {
        let config =
            EntityConfig::from_toml(include_str!("../../assets/entities/station_axiom.toml"))
                .expect("must parse");
        assert!(config.audio.is_none());
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
        // Display text lives in assets/strings/strings.csv; the TOML holds the
        // string id, which is what Rust passes through to the client.
        assert_eq!(config.name.as_deref(), Some("entity.star_sun.name"));
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
        assert_eq!(config.name.as_deref(), Some("entity.planet_earth.name"));
        assert!(config.mesh.is_some(), "planet_earth.toml must have [mesh]");
        let collider = config
            .collider
            .as_ref()
            .expect("planet_earth.toml must have [collider]");
        assert_eq!(collider.shape, ColliderShape::Ball);
        assert!((collider.radius - 20.0).abs() < 1e-6);

        // Textured-planet section: earth has clouds, atmosphere, and
        // night-gated city-light emission without a separate mask.
        let planet = config
            .planet
            .as_ref()
            .expect("planet_earth.toml must have [planet]");
        assert!((planet.radius - 20.0).abs() < 1e-6);
        assert!(planet.surface.normal.is_some());
        assert!(planet.surface.emissive_colour.is_some());
        assert!(planet.surface.emissive_mask.is_none());
        assert!(planet.surface.emissive_night_only);
        assert!(planet.clouds.is_some());
        assert!(planet.atmosphere.is_some());
    }

    #[test]
    fn planet_lava_template_parses_with_dayside_emission() {
        let toml_str = include_str!("../../assets/entities/planet_lava.toml");
        let config = EntityConfig::from_toml(toml_str).expect("planet_lava.toml must parse");
        let planet = config
            .planet
            .as_ref()
            .expect("planet_lava.toml must have [planet]");
        // Lava glows on the day side too — the night gate must be off.
        assert!(!planet.surface.emissive_night_only);
        assert!(planet.surface.emissive_mask.is_some());
        let clouds = planet.clouds.as_ref().expect("ash shell expected");
        assert!((clouds.scale - 1.03).abs() < 1e-6);
        assert!(
            (clouds.drift_speed - 0.0).abs() < 1e-6,
            "no motion by default"
        );
    }

    #[test]
    fn moon_luna_template_parses_surface_only() {
        let toml_str = include_str!("../../assets/entities/moon_luna.toml");
        let config = EntityConfig::from_toml(toml_str).expect("moon_luna.toml must parse");
        let planet = config
            .planet
            .as_ref()
            .expect("moon_luna.toml must have [planet]");
        assert!(planet.surface.emissive_colour.is_none());
        assert!(planet.clouds.is_none());
        assert!(planet.atmosphere.is_none());
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

    /// The Courier is the two-station player hull. Its TOML-authored system
    /// ownership and support loops are individually easy to break, so pin them
    /// against the real asset.
    #[test]
    fn courier_toml_is_a_valid_two_station_hull() {
        use crate::messages::{StationId, SystemId};

        let toml_str = include_str!("../../assets/entities/alliance_courier.toml");
        let config = EntityConfig::from_toml(toml_str).expect("alliance_courier must parse");
        let ship_config = config.ship_config.clone().expect("ship_config present");

        // Two stations, one rating each.
        assert_eq!(ship_config.stations.len(), 2);
        let captain = StationId("captain".into());
        let tactical = StationId("tactical".into());
        assert_eq!(ship_config.stations[0].id, captain);
        assert_eq!(ship_config.stations[1].id, tactical);
        assert_eq!(ship_config.stations[0].ratings.len(), 1);
        assert_eq!(ship_config.stations[1].ratings.len(), 1);
        assert_eq!(
            ship_config.stations[0].console.as_deref(),
            Some("gui/courier/captain.html")
        );
        assert_eq!(
            ship_config.stations[1].console.as_deref(),
            Some("gui/courier/tactical.html")
        );

        // The guns live on Tactical, so every ship-level Tactical gate and the
        // WeaponsUpdate broadcast must resolve there.
        assert_eq!(ship_config.weapons_station(), Some(tactical.clone()));

        // Exactly one weapon: one blaster, no phasers, no torpedoes.
        let weapons = config.weapons_console.as_ref().expect("weapons_console");
        assert_eq!(weapons.blaster_banks.len(), 1);
        assert_eq!(weapons.blaster_banks[0].id, "fore");
        assert!(
            weapons.phaser_banks.is_empty(),
            "courier has no phasers — an absent list must not synthesise a default bank"
        );
        assert!(config.torpedoes.is_none(), "courier has no torpedoes");

        // Power is fully authored, including every canonical group.
        assert!(
            config.power.is_some(),
            "courier has an authored [power] block"
        );
        assert_eq!(ship_config.power_groups.len(), 4);

        // Every system is owned by Captain or Tactical. Ownerless + ai_only
        // would be inert on the player spawn path.
        for sys in &ship_config.systems {
            assert!(
                matches!(sys.station.as_ref(), Some(station) if station == &captain || station == &tactical),
                "system {:?} must be station-owned",
                sys.id
            );
            assert!(!sys.ai_only, "system {:?} must not rely on ai_only", sys.id);
        }

        // Two arcs, fore and aft, hang off Captain's shields system.
        assert_eq!(config.shield_arcs.len(), 2);
        let arc_ids: Vec<&str> = config.shield_arcs.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(arc_ids, vec!["fore", "aft"]);

        // Red Alert is human-controlled at the Captain station.
        let automated = &ship_config.stations[0].ratings[0].automated_systems;
        assert!(!automated.contains(&SystemId("red-alert".into())));
        assert!(automated.is_empty());

        // Cinematic button only resolves when this block exists.
        assert!(config.cinematic_camera.is_some());

        // One team serves both stations.
        let repair = config.repair.as_ref().expect("[repair] present");
        assert_eq!(repair.repair_team_count, 1);
        assert!(repair.repair_rate_hp_per_sec < 0.5);
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
        // Destroy directive kind so `ai_target_selection` picks it up.
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
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        }];
        let mut sys = TorpedoSystem::from_configs(&tubes, cfg);
        assert!(sys.start_load("fore"));
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        sys.tick(sys.config.load_time, &targets, &mut || "test".into());
        sys.launch("fore", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
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
                barrels: Vec::new(),
                pattern: Vec::new(),
                volley_max: 1,
                ai_target_count: None,
                ai: None,
            },
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 180.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                barrels: Vec::new(),
                pattern: Vec::new(),
                volley_max: 1,
                ai_target_count: None,
                ai: None,
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
                barrels: Vec::new(),
                pattern: Vec::new(),
                volley_max: 1,
                ai_target_count: None,
                ai: None,
            },
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 0.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                barrels: Vec::new(),
                pattern: Vec::new(),
                volley_max: 1,
                ai_target_count: None,
                ai: None,
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
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        }];
        let err = validate_torpedo_tubes(&tubes).unwrap_err();
        assert!(err.contains("fire_arc_deg"));
    }

    // ── Torpedo tube barrel-pattern validation (issue #766) ──────────────────

    fn torpedo_tube(id: &str) -> TorpedoTubeConfig {
        TorpedoTubeConfig {
            id: id.into(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        }
    }

    #[test]
    fn validate_torpedo_tubes_accepts_legacy_single_barrel() {
        // No barrels + no pattern is the backward-compat single-barrel tube.
        assert!(validate_torpedo_tubes(&[torpedo_tube("fore")]).is_ok());
    }

    #[test]
    fn validate_torpedo_tubes_accepts_valid_pattern() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into(), "b1".into()];
        t.pattern = vec![
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![0],
                offset_secs: 0.0,
            },
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![0, 1],
                offset_secs: 0.3,
            },
        ];
        assert!(validate_torpedo_tubes(&[t]).is_ok());
    }

    #[test]
    fn validate_torpedo_tubes_rejects_barrel_index_out_of_range() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into(), "b1".into()];
        t.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![2], // only 0,1 exist
            offset_secs: 0.0,
        }];
        let err = validate_torpedo_tubes(&[t]).unwrap_err();
        assert!(err.contains("barrel index 2"), "{err}");
    }

    #[test]
    fn validate_torpedo_tubes_rejects_negative_offset() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into()];
        t.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![0],
            offset_secs: -0.5,
        }];
        let err = validate_torpedo_tubes(&[t]).unwrap_err();
        assert!(err.contains("offset_secs"), "{err}");
    }

    #[test]
    fn validate_torpedo_tubes_rejects_empty_step() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into()];
        t.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![],
            offset_secs: 0.0,
        }];
        assert!(validate_torpedo_tubes(&[t]).is_err());
    }

    #[test]
    fn validate_torpedo_tubes_rejects_multi_barrel_without_pattern() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into(), "b1".into()];
        // No pattern: under-specified for >1 barrel.
        let err = validate_torpedo_tubes(&[t]).unwrap_err();
        assert!(err.contains("pattern"), "{err}");
    }

    #[test]
    fn every_shipped_torpedo_tube_config_validates() {
        // The authoring gate the editor enforces must hold for shipped hulls.
        let mut problems: Vec<String> = Vec::new();
        let entries = std::fs::read_dir("assets/entities").expect("assets/entities exists");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let cfg: EntityConfig = match toml::from_str(&text) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let Some(torpedoes) = cfg.torpedoes.as_ref() else {
                continue;
            };
            if torpedoes.tubes.is_empty() {
                continue;
            }
            if let Err(e) = validate_torpedo_tubes(&torpedoes.tubes) {
                problems.push(format!("{}: {e}", path.display()));
            }
        }
        assert!(
            problems.is_empty(),
            "shipped torpedo tubes invalid: {problems:?}"
        );
    }

    // ── Blaster bank validation (issue #765) ─────────────────────────────────

    fn blaster_bank(id: &str) -> BlasterBankConfig {
        BlasterBankConfig {
            id: id.into(),
            fire_arc_deg: 90.0,
            ..BlasterBankConfig::default()
        }
    }

    #[test]
    fn validate_blaster_banks_accepts_empty_list() {
        // Most hulls carry no blasters; an empty list is fine.
        assert!(validate_blaster_banks(&[]).is_ok());
    }

    #[test]
    fn validate_blaster_banks_accepts_legacy_single_barrel() {
        // No barrels + no pattern is the backward-compat single-barrel bank.
        let banks = vec![blaster_bank("fore")];
        assert!(validate_blaster_banks(&banks).is_ok());
    }

    #[test]
    fn validate_blaster_banks_accepts_valid_pattern() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into(), "b1".into()];
        b.pattern = vec![
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![0],
                offset_secs: 0.0,
            },
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![0, 1],
                offset_secs: 0.3,
            },
        ];
        assert!(validate_blaster_banks(&[b]).is_ok());
    }

    #[test]
    fn validate_blaster_banks_rejects_barrel_index_out_of_range() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into(), "b1".into()];
        b.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![2], // only 0,1 exist
            offset_secs: 0.0,
        }];
        let err = validate_blaster_banks(&[b]).unwrap_err();
        assert!(err.contains("barrel index 2"), "{err}");
    }

    #[test]
    fn validate_blaster_banks_rejects_negative_offset() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into()];
        b.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![0],
            offset_secs: -0.5,
        }];
        let err = validate_blaster_banks(&[b]).unwrap_err();
        assert!(err.contains("offset_secs"), "{err}");
    }

    #[test]
    fn validate_blaster_banks_rejects_empty_step() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into()];
        b.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![],
            offset_secs: 0.0,
        }];
        assert!(validate_blaster_banks(&[b]).is_err());
    }

    #[test]
    fn validate_blaster_banks_rejects_multi_barrel_without_pattern() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into(), "b1".into()];
        // No pattern: under-specified for >1 barrel.
        let err = validate_blaster_banks(&[b]).unwrap_err();
        assert!(err.contains("pattern"), "{err}");
    }

    #[test]
    fn validate_blaster_banks_rejects_duplicate_ids() {
        let banks = vec![blaster_bank("fore"), blaster_bank("fore")];
        let err = validate_blaster_banks(&banks).unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn every_shipped_blaster_bank_config_validates() {
        // The authoring gate the editor enforces must hold for shipped hulls.
        let mut problems: Vec<String> = Vec::new();
        let entries = std::fs::read_dir("assets/entities").expect("assets/entities exists");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let file = path.to_string_lossy().replace('\\', "/");
            let toml = std::fs::read_to_string(&path).expect("entity readable");
            let cfg = match EntityConfig::from_toml(&toml) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(weapons) = cfg.weapons_console.as_ref() {
                if let Err(e) = validate_blaster_banks(&weapons.blaster_banks) {
                    problems.push(format!("{file}: {e}"));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "shipped blaster banks must validate:\n{}",
            problems.join("\n")
        );
    }

    // ── Inline fine-system AI policy (issue #775) ────────────────────────────

    const CHANNELS: &[&str] = &[CAPTAIN_RED_ALERT_CHANNEL];
    const VERBS: &[&str] = &[CAPTAIN_SET_RED_ALERT_VERB];

    fn captain_ai_toml() -> &'static str {
        r#"
name = "Test Cruiser"

[captain_console.ai]
param = { combat_window_secs = 8.0 }

[[captain_console.ai.rule]]
priority = 10
channel = "red_alert"
when = "fact(secs_since_combat) < param(combat_window_secs)"
verb = "set_red_alert"
value = true

[[captain_console.ai.rule]]
priority = 0
channel = "red_alert"
when = "true"
verb = "set_red_alert"
value = false
"#
    }

    #[test]
    fn captain_ai_policy_parses_and_resolves_to_typed_policy() {
        let cfg = EntityConfig::from_toml(captain_ai_toml()).expect("parse must succeed");
        let ai = cfg
            .captain_console
            .as_ref()
            .and_then(|c| c.ai.as_ref())
            .expect("captain_console.ai present");
        assert_eq!(ai.param.get("combat_window_secs"), Some(&8.0));
        assert_eq!(ai.rule.len(), 2);
        let policy = ai.to_policy().expect("policy resolves");
        assert_eq!(policy.rules.len(), 2);
        assert!(!policy.idle);
    }

    #[test]
    fn default_captain_policy_validates_and_resolves() {
        let cfg = default_captain_ai_config();
        assert!(validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).is_ok());
        assert!(cfg.to_policy().is_ok());
    }

    // ── Optional stateful policy schema (issue #882) ─────────────────────────

    /// AC7 — THE back-compat guard. Every shipped stateless block still parses
    /// AND decodes to a policy with NO machine, no states and no memory: the
    /// #882 schema fields are all `#[serde(default)]`, so nothing an author
    /// wrote before this issue changed meaning. Enumerates all fourteen
    /// canonical defaults behind the twelve Group A hosts.
    #[test]
    fn every_shipped_stateless_default_still_parses_as_stateless() {
        let shipped: Vec<(&str, FineSystemAiConfigToml)> = vec![
            ("captain", default_captain_ai_config()),
            ("comms_response", default_comms_response_ai_config()),
            ("engines", default_engines_ai_config()),
            ("steering", default_steering_ai_config()),
            ("lateral", default_lateral_ai_config()),
            ("vertical", default_vertical_ai_config()),
            ("impulse", default_impulse_ai_config()),
            ("boost", default_boost_ai_config()),
            ("phaser_bank", default_phaser_bank_ai_config()),
            ("blaster_bank", default_blaster_bank_ai_config()),
            ("torpedo_tube", default_torpedo_tube_ai_config()),
            ("torpedo_magazine", default_torpedo_magazine_ai_config()),
            ("shields_focus", default_shields_focus_ai_config()),
            ("power", default_power_ai_config()),
        ];
        for (name, cfg) in shipped {
            assert!(
                cfg.initial_state.is_none() && cfg.state.is_empty() && cfg.memory.is_empty(),
                "{name}: a shipped default must declare no #882 state machine"
            );
            let policy = cfg
                .to_policy()
                .unwrap_or_else(|e| panic!("{name}: must decode: {e}"));
            assert!(
                policy.machine().is_none() && policy.initial_state().is_none(),
                "{name}: a stateless block must decode to `machine: None`"
            );
            assert_eq!(
                policy.rules.len(),
                cfg.rule.len(),
                "{name}: every authored rule still decodes to a top-level rule"
            );
        }
    }

    /// A minimal authored stateful block: `initial_state`, two states with
    /// their own continuous rules, explicitly prioritised transitions, and a
    /// typed private memory declaration (AC1).
    fn stateful_boost_toml() -> &'static str {
        r#"
name = "Stateful"
[helm_console.boost_ai]
initial_state = "cruise"

[helm_console.boost_ai.param]
surge_urgency = 0.5
surge_dwell_secs = 3.0
max_engagements = 3.0

[helm_console.boost_ai.memory]
engagements = 0.0

[[helm_console.boost_ai.state]]
id = "cruise"

[[helm_console.boost_ai.state.transition]]
priority = 10
to = "surge"
when = "fact(hazard_urgency) > param(surge_urgency) and memory(engagements) < param(max_engagements)"

[[helm_console.boost_ai.state]]
id = "surge"

[[helm_console.boost_ai.state.rule]]
priority = 0
channel = "boost"
when = "true"
verb = "engage_boost"

[[helm_console.boost_ai.state.transition]]
priority = 0
to = "cruise"
when = "state_time >= param(surge_dwell_secs)"
"#
    }

    /// AC1: an authored stateful block round-trips through the TOML schema into
    /// the typed machine, with per-state rules and prioritised transitions.
    #[test]
    fn stateful_policy_round_trips_from_toml_to_typed_machine() {
        let cfg = EntityConfig::from_toml(stateful_boost_toml()).expect("parse must succeed");
        let ai = cfg
            .helm_console
            .as_ref()
            .and_then(|h| h.boost_ai.as_ref())
            .expect("helm_console.boost_ai present");
        assert_eq!(ai.initial_state.as_deref(), Some("cruise"));
        assert_eq!(ai.state.len(), 2);
        assert_eq!(ai.memory.get("engagements"), Some(&0.0));

        let policy = ai.to_policy().expect("policy resolves");
        let machine = policy.machine().expect("machine decoded");
        assert_eq!(machine.initial, "cruise");
        assert_eq!(machine.states.len(), 2);
        assert!(
            policy.rules.is_empty(),
            "a purely stateful policy carries no top-level rules"
        );
        let cruise = machine.state("cruise").expect("cruise declared");
        assert!(cruise.rules.is_empty());
        assert_eq!(cruise.transitions.len(), 1);
        assert_eq!(cruise.transitions[0].to, "surge");
        assert_eq!(cruise.transitions[0].priority, 10);
        let surge = machine.state("surge").expect("surge declared");
        assert_eq!(surge.rules.len(), 1);
        assert_eq!(surge.rules[0].channel, HELM_BOOST_CHANNEL);
        assert_eq!(
            surge.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::EngageBoost
        );
        assert_eq!(machine.initial_memory.get("engagements"), Some(0.0));
    }

    /// Build a stateful policy config for the AC6 rejection cases directly, so
    /// each rejection is isolated from TOML surface noise.
    fn stateful_cfg(
        initial: Option<&str>,
        states: Vec<FineSystemAiStateToml>,
    ) -> FineSystemAiConfigToml {
        FineSystemAiConfigToml {
            idle: false,
            param: std::collections::HashMap::new(),
            rule: Vec::new(),
            initial_state: initial.map(str::to_string),
            state: states,
            memory: std::collections::HashMap::new(),
        }
    }

    fn boost_state(id: &str, to: &[&str]) -> FineSystemAiStateToml {
        FineSystemAiStateToml {
            id: id.to_string(),
            rule: Vec::new(),
            transition: to
                .iter()
                .map(|t| FineSystemAiTransitionToml {
                    priority: 0,
                    to: t.to_string(),
                    when: "true".to_string(),
                })
                .collect(),
        }
    }

    /// AC6: an `initial_state` naming a state that was never declared.
    #[test]
    fn undeclared_initial_state_is_rejected() {
        let cfg = stateful_cfg(Some("nowhere"), vec![boost_state("cruise", &[])]);
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("initial_state") && err.contains("nowhere"),
            "got: {err}"
        );
    }

    /// AC6: states declared with no `initial_state` at all is the same defect —
    /// there is no entry point.
    #[test]
    fn states_without_an_initial_state_are_rejected() {
        let cfg = stateful_cfg(None, vec![boost_state("cruise", &[])]);
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("initial_state"), "got: {err}");
        // The decoder refuses it too, so a caller skipping validation cannot
        // build a half-machine.
        assert!(cfg.to_policy().is_err());
    }

    /// AC6: a transition targeting a state that was never declared.
    #[test]
    fn transition_to_undeclared_state_is_rejected() {
        let cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &["surge"])]);
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("undeclared state") && err.contains("surge"),
            "got: {err}"
        );
    }

    /// AC6: duplicate state ids — "which `cruise` did you mean?" has no answer.
    #[test]
    fn duplicate_state_ids_are_rejected() {
        let cfg = stateful_cfg(
            Some("cruise"),
            vec![boost_state("cruise", &[]), boost_state("cruise", &[])],
        );
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("duplicate state id"), "got: {err}");
    }

    /// AC6: an unreachable state — neither the initial state nor any
    /// transition's target. A self-loop does NOT make a state reachable.
    #[test]
    fn unreachable_state_is_rejected() {
        let cfg = stateful_cfg(
            Some("cruise"),
            vec![
                boost_state("cruise", &[]),
                boost_state("orphan", &["orphan"]),
            ],
        );
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("unreachable state") && err.contains("orphan"),
            "got: {err}"
        );
        // ...but wiring it up from the initial state makes it legal.
        let ok = stateful_cfg(
            Some("cruise"),
            vec![
                boost_state("cruise", &["orphan"]),
                boost_state("orphan", &[]),
            ],
        );
        assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    /// AC6, the transitive case: a DISCONNECTED CLUSTER. `cruise` is the
    /// initial state; `drift` and `wander` transition to each other but nothing
    /// reaches either of them. Both are "the target of a transition", so a
    /// single pass that credits every transition target regardless of whether
    /// its source is itself reachable accepts this graph — which is exactly the
    /// dead branch AC6 exists to reject. Reachability has to be a fixpoint walk
    /// from `initial`.
    #[test]
    fn disconnected_state_cluster_is_rejected() {
        let cfg = stateful_cfg(
            Some("cruise"),
            vec![
                boost_state("cruise", &[]),
                boost_state("drift", &["wander"]),
                boost_state("wander", &["drift"]),
            ],
        );
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("unreachable state") && (err.contains("drift") || err.contains("wander")),
            "got: {err}"
        );
        // Wiring ONE edge from the initial state into the cluster makes the
        // whole cluster reachable — the walk is transitive, not one-hop.
        let ok = stateful_cfg(
            Some("cruise"),
            vec![
                boost_state("cruise", &["drift"]),
                boost_state("drift", &["wander"]),
                boost_state("wander", &["drift"]),
            ],
        );
        assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    /// AC6: a `memory(...)` reference in a STATELESS policy. Private memory has
    /// no owner without a state machine, and reading a silent `false` would be
    /// a trap rather than a diagnostic.
    #[test]
    fn memory_reference_in_a_stateless_policy_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: std::collections::HashMap::new(),
            rule: vec![FineSystemAiRuleToml {
                priority: 0,
                channel: HELM_BOOST_CHANNEL.to_string(),
                when: "memory(engagements) > 0".to_string(),
                verb: HELM_ENGAGE_BOOST_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("memory") && err.contains("no states"),
            "got: {err}"
        );
    }

    /// AC6: a `state_time` reference in a STATELESS policy — the same defect on
    /// the other private atom.
    #[test]
    fn state_time_reference_in_a_stateless_policy_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: std::collections::HashMap::new(),
            rule: vec![FineSystemAiRuleToml {
                priority: 0,
                channel: HELM_BOOST_CHANNEL.to_string(),
                when: "state_time > 5".to_string(),
                verb: HELM_ENGAGE_BOOST_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("state_time") && err.contains("no states"),
            "got: {err}"
        );
    }

    /// An undeclared `memory(...)` slot is rejected in a STATEFUL policy too —
    /// the same contract `param(...)` has carried since #775.
    #[test]
    fn undeclared_memory_slot_is_rejected_in_a_stateful_policy() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.state[0].transition = vec![FineSystemAiTransitionToml {
            priority: 0,
            to: "cruise".to_string(),
            when: "memory(never_declared) > 0".to_string(),
        }];
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("undeclared memory"), "got: {err}");
    }

    /// The existing per-rule channel/verb/param checks run unchanged over each
    /// STATE's rules, not just the top-level list.
    #[test]
    fn per_state_rules_get_the_same_channel_and_verb_checks() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.state[0].rule = vec![FineSystemAiRuleToml {
            priority: 0,
            channel: "not_a_channel".to_string(),
            when: "true".to_string(),
            verb: HELM_ENGAGE_BOOST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }];
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("unknown channel") && err.contains("state 'cruise' rule 0"),
            "got: {err}"
        );
    }

    /// `idle = true` alongside states is as contradictory as `idle` alongside
    /// rules.
    #[test]
    fn idle_alongside_states_is_rejected() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.idle = true;
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("idle") && err.contains("states"), "got: {err}");
    }

    /// Issue #883, carried forward from the #882 review: a policy declaring BOTH
    /// top-level rules and states is rejected outright.
    ///
    /// Before this, the shape validated and then quietly did nothing useful: a
    /// machine resolves exclusively through `resolve_channel_in_state`, so the
    /// top-level rules were dead code that looked live. Worse, `stateful` is
    /// computed from the presence of states, so a `memory(...)` reference in one
    /// of those dead top-level rules PASSED validation and then evaluated false
    /// for ever (the stateless scan hands `best_in` an empty memory bag). Two
    /// silent failures in one shape — the same class as #882's blocking bug.
    #[test]
    fn a_policy_with_both_top_level_rules_and_states_is_rejected() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.rule = vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_BOOST_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: HELM_ENGAGE_BOOST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }];
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("both top-level rules") && err.contains("states"),
            "got: {err}"
        );
    }

    /// The rejection above must not catch either honest shape: a purely
    /// stateless policy and a purely stateful one both still validate. (Every
    /// shipped default is the former; the destroyer's three policies are the
    /// latter.)
    #[test]
    fn rule_xor_state_leaves_both_honest_shapes_valid() {
        let stateless = default_boost_ai_config();
        assert!(validate_fine_system_ai_policy(&stateless, BOOST_CHANNELS, BOOST_VERBS).is_ok());
        let stateful = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        assert!(stateful.rule.is_empty(), "the fixture must be state-only");
        assert!(validate_fine_system_ai_policy(&stateful, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    // ── The Harrow Destroyer hull (issue #883) ───────────────────────────────

    const HARROW_DESTROYER_TOML: &str =
        include_str!("../../assets/entities/ship_harrow_destroyer.toml");

    /// AC4, both halves. Forward blasters are PRESENT and correctly forward
    /// (narrow arc dead ahead); torpedoes are ABSENT — no magazine, no tubes,
    /// and no torpedo system entry.
    ///
    /// The absence is asserted explicitly because it is content that is very
    /// easy to "helpfully" restore: every other armed NPC hull in the set has a
    /// `[torpedoes]` block, so copying one as a starting point re-adds it
    /// silently, and nothing else in the suite would notice.
    #[test]
    fn harrow_destroyer_carries_forward_blasters_and_no_torpedoes() {
        let cfg = EntityConfig::from_toml(HARROW_DESTROYER_TOML)
            .expect("the destroyer hull must pass content validation");

        let wc = cfg
            .weapons_console
            .as_ref()
            .expect("the hull declares [weapons_console]");
        assert!(
            !wc.blaster_banks.is_empty(),
            "the destroyer's whole armament is its forward blasters"
        );
        for bank in &wc.blaster_banks {
            assert_eq!(
                bank.facing_deg, 0.0,
                "bank '{}' must face dead ahead: a fly-through fires off the bow",
                bank.id
            );
            assert!(
                bank.fire_arc_deg > 0.0 && bank.fire_arc_deg <= 90.0,
                "bank '{}' must be a NARROW forward arc, got {}",
                bank.id,
                bank.fire_arc_deg
            );
        }
        assert!(
            wc.phaser_banks.is_empty(),
            "the destroyer is blaster-armed only"
        );

        // The absence assertion (AC4).
        assert!(
            cfg.torpedoes.is_none(),
            "the destroyer must carry NO torpedo magazine"
        );
        let ship_config = cfg
            .ship_config
            .as_ref()
            .expect("the hull declares [[system]] blocks");
        for system in &ship_config.systems {
            assert!(
                !system.kind.contains("torpedo"),
                "the destroyer must declare no torpedo system, found '{:?}' ({})",
                system.id,
                system.kind
            );
        }
    }

    /// The doctrine itself: all three travel axes author a STATEFUL policy, the
    /// two yaw mode verbs are both used, and boost is authored — the block
    /// without which `ai_helm_boost` returns before it does anything and the
    /// escape leg silently loses its back half.
    #[test]
    fn harrow_destroyer_authors_the_fly_through_machine_on_all_three_axes() {
        let cfg = EntityConfig::from_toml(HARROW_DESTROYER_TOML).expect("hull must parse");
        let hc = cfg
            .helm_console
            .as_ref()
            .expect("the hull declares [helm_console]");
        assert!(
            hc.boost.is_some(),
            "[helm_console.boost] is mandatory: without it the spawner inserts a \
             DISABLED BoostConfigResource and ai_helm_boost stands down"
        );

        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref()),
            ("steering_ai", hc.steering_ai.as_ref()),
            ("boost_ai", hc.boost_ai.as_ref()),
        ] {
            let ai = ai.unwrap_or_else(|| panic!("{name} must be authored"));
            assert!(
                ai.rule.is_empty(),
                "{name} must be state-only (rule XOR state)"
            );
            assert_eq!(
                ai.state.len(),
                3,
                "{name} authors the three-state pass machine"
            );
            assert_eq!(ai.initial_state.as_deref(), Some("acquire"));
            let policy = ai.to_policy().expect("must decode");
            assert!(
                policy.machine().is_some(),
                "{name} must decode to a machine"
            );
        }

        // The yaw channel carries BOTH mode verbs, and which one wins is the
        // whole doctrine: tracking while inbound, frozen heading on the escape.
        let steering = hc.steering_ai.as_ref().unwrap();
        let verbs: Vec<&str> = steering
            .state
            .iter()
            .flat_map(|s| s.rule.iter())
            .map(|r| r.verb.as_str())
            .collect();
        assert!(verbs.contains(&HELM_ACTUATE_DESIRED_FACING_VERB));
        assert!(verbs.contains(&HELM_HOLD_COMMITTED_HEADING_VERB));
    }

    /// AC6: every manoeuvre threshold the doctrine flies by is an authored
    /// `param`, and the host-side pass surface can find the four it reads by
    /// name. A rename in either direction lights this up — which matters,
    /// because the host's response to a missing param is to decline the pass
    /// entirely and quietly fall back to ordinary doctrine travel.
    #[test]
    fn harrow_destroyer_authors_every_manoeuvre_threshold_as_a_param() {
        let cfg = EntityConfig::from_toml(HARROW_DESTROYER_TOML).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        let engines = hc.engines_ai.as_ref().unwrap();
        let steering = hc.steering_ai.as_ref().unwrap();
        let boost = hc.boost_ai.as_ref().unwrap();

        for required in [
            crate::ship::helm_ai::APPROACH_SPEED_PARAM,
            crate::ship::helm_ai::ESCAPE_SPEED_PARAM,
        ] {
            assert!(
                engines.param.contains_key(required),
                "engines_ai must author `{required}`"
            );
        }
        for required in [
            crate::ship::helm_ai::TRACKING_DEADBAND_PARAM,
            crate::ship::helm_ai::TRACKING_FULL_STEER_PARAM,
        ] {
            assert!(
                steering.param.contains_key(required),
                "steering_ai must author `{required}`"
            );
        }
        // Shared manoeuvre thresholds — every axis's guards reference them, so
        // every axis declares them (validation rejects an undeclared reference).
        for ai in [engines, steering, boost] {
            for required in [
                "commit_range",
                "closing_rate_epsilon",
                "closest_approach_hysteresis",
                "escape_duration_secs",
            ] {
                assert!(ai.param.contains_key(required), "must author `{required}`");
            }
            assert!(
                ai.memory
                    .contains_key(crate::ship::helm_ai::MIN_RANGE_SEEN_MEMORY),
                "the closest-approach detector's running minimum must be declared"
            );
        }
        assert!(boost.param.contains_key("escape_boost_secs"));
    }

    /// The authored stateful block validates end to end through the real
    /// content-load path (`EntityConfig::from_toml` runs the validator).
    #[test]
    fn authored_stateful_block_passes_content_validation() {
        let cfg = EntityConfig::from_toml(stateful_boost_toml()).expect("parse must succeed");
        let ai = cfg
            .helm_console
            .as_ref()
            .and_then(|h| h.boost_ai.as_ref())
            .expect("boost_ai present");
        assert!(validate_fine_system_ai_policy(ai, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    #[test]
    fn empty_ai_declaration_is_rejected_as_silence() {
        // `[captain_console.ai]` with neither `idle` nor a rule is silence.
        let toml = r#"
name = "Silent"
[captain_console.ai]
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("empty") || err.contains("idle"), "got: {err}");
    }

    #[test]
    fn explicit_idle_declaration_is_accepted() {
        let toml = r#"
name = "Idle"
[captain_console.ai]
idle = true
"#;
        let cfg = EntityConfig::from_toml(toml).expect("idle is a valid declaration");
        let ai = cfg.captain_console.unwrap().ai.unwrap();
        assert!(ai.idle);
        assert!(ai.to_policy().unwrap().idle);
    }

    // ── Per-bank weapon AI policy (issue #781) ───────────────────────────────

    #[test]
    fn default_phaser_and_blaster_bank_policies_validate_and_resolve() {
        let p = default_phaser_bank_ai_config();
        assert!(
            validate_fine_system_ai_policy(&p, PHASER_BANK_CHANNELS, PHASER_BANK_VERBS).is_ok()
        );
        let pp = p.to_policy().expect("phaser default resolves");
        // Baseline: unconditional fire (not idle, one rule).
        assert!(!pp.idle);
        assert_eq!(pp.rules.len(), 1);

        let b = default_blaster_bank_ai_config();
        assert!(
            validate_fine_system_ai_policy(&b, BLASTER_BANK_CHANNELS, BLASTER_BANK_VERBS).is_ok()
        );
        assert!(!b.to_policy().expect("blaster default resolves").idle);
    }

    #[test]
    fn phaser_bank_inline_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Gunboat"

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
auto_arc_deg = 60.0

[[weapons_console.phaser_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "fact(in_range) > 0 and fact(in_arc) > 0"
verb = "fire_phaser"
value = false
"#;
        let cfg = EntityConfig::from_toml(toml).expect("phaser bank ai must parse + validate");
        let bank = &cfg.weapons_console.unwrap().phaser_banks[0];
        let policy = bank.ai.as_ref().unwrap().to_policy().unwrap();
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::FirePhaser
        );
    }

    #[test]
    fn blaster_bank_inline_idle_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Escort"

[[weapons_console.blaster_banks]]
id = "fore"

[weapons_console.blaster_banks.ai]
idle = true
"#;
        let cfg = EntityConfig::from_toml(toml).expect("blaster bank idle ai must parse");
        let bank = &cfg.weapons_console.unwrap().blaster_banks[0];
        assert!(bank.ai.as_ref().unwrap().to_policy().unwrap().idle);
    }

    #[test]
    fn phaser_bank_ai_rejects_unknown_verb_at_load() {
        // The blaster verb on a phaser bank channel is an authoring error caught
        // by the from_toml validation loop, before any live tick.
        let toml = r#"
name = "Bad"

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
auto_arc_deg = 60.0

[[weapons_console.phaser_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "fire_blaster"
value = false
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn blaster_bank_ai_rejects_unknown_channel_at_load() {
        let toml = r#"
name = "Bad2"

[[weapons_console.blaster_banks]]
id = "fore"

[[weapons_console.blaster_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "fire_blaster"
value = false
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    // ── Shields focus AI policy (issue #783) ─────────────────────────────────

    #[test]
    fn default_shields_focus_policy_validates_and_resolves() {
        let s = default_shields_focus_ai_config();
        assert!(
            validate_fine_system_ai_policy(&s, SHIELD_FOCUS_CHANNELS, SHIELD_FOCUS_VERBS).is_ok()
        );
        let sp = s.to_policy().expect("shields focus default resolves");
        // Baseline: a priority-10 damage rule + a priority-0 imbalance fallback.
        assert!(!sp.idle);
        assert_eq!(sp.rules.len(), 2);
        // All four authored numbers seeded as params.
        assert!(sp.params.get(SHIELD_FOCUS_DAMAGE_WINDOW_PARAM).is_some());
        assert!(sp
            .params
            .get(SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM)
            .is_some());
        assert!(sp.params.get(SHIELD_FOCUS_DAMAGE_PCT_PARAM).is_some());
        assert!(sp.params.get(SHIELD_FOCUS_HEALTH_RATIO_PARAM).is_some());
    }

    #[test]
    fn shields_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Warden"

[shields_console.ai_policy]
param = { damage_window_secs = 6.0, min_damage_window_secs = 2.0, damage_pct_threshold = 60.0, health_ratio_threshold = 40.0 }

[[shields_console.ai_policy.rule]]
priority = 10
channel = "shield_focus"
when = "fact(recent_damage_pct_max) >= param(damage_pct_threshold)"
verb = "focus_shield_arc"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "true"
verb = "focus_shield_arc"
"#;
        let cfg = EntityConfig::from_toml(toml).expect("shields ai_policy must parse + validate");
        let ai = cfg
            .shields_console
            .as_ref()
            .and_then(|sc| sc.ai_policy.as_ref())
            .expect("shields_console.ai_policy present");
        assert_eq!(ai.param.get("damage_window_secs"), Some(&6.0));
        assert_eq!(ai.param.get("health_ratio_threshold"), Some(&40.0));
        let policy = ai.to_policy().expect("policy resolves");
        assert_eq!(policy.rules.len(), 2);
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::FocusShieldArc
        );
    }

    // ── Power allocation AI policy (issue #784) ──────────────────────────────

    /// Minimal ship TOML carrying authored `[power_groups.*]` plus a
    /// `[power.ai_policy]` block, used by the power-policy load tests.
    fn power_policy_toml(rules: &str) -> String {
        format!(
            r#"
name = "Reactorer"

[power]
capacity = 90
rates = [ 5, 4, 3, 2, -2, -5 ]
emergency_threshold = 22

[power.ai_policy.param]
thrust_threshold = 0.7
min_reserve_helm = 50.0

{rules}

[power_groups.helm]
label = "HELM"
default_level = 2

[power_groups.weapons]
label = "WEAPONS"
default_level = 2

[power_groups.sensors]
label = "SENSORS"
default_level = 1

[power_groups.ops]
label = "OPS"
default_level = 1
"#
        )
    }

    #[test]
    fn default_power_policy_validates_and_resolves() {
        // The synthesised default reproduces the retired engine as four rules
        // (helm elevate + baseline, weapons elevate + baseline), all emitting the
        // value-carrying allocation verb, and every rule declares a reserve param.
        let cfg = default_power_ai_config();
        // Validated against the canonical group channels.
        assert!(validate_fine_system_ai_policy(
            &cfg,
            &["helm", "weapons", "sensors"],
            &[POWER_SET_ALLOCATION_VERB]
        )
        .is_ok());
        let p = cfg.to_policy().expect("default power policy resolves");
        assert!(!p.idle);
        assert_eq!(p.rules.len(), 4);
        // The elevate rules carry the absolute magnitude in the verb payload.
        assert!(p.rules.iter().any(|r| r.verb
            == crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(
                DEFAULT_POWER_ELEVATED_LEVEL
            )));
        assert!(cfg.param.contains_key(POWER_HELM_RESERVE_PARAM));
        assert!(cfg.param.contains_key(POWER_WEAPONS_RESERVE_PARAM));
    }

    #[test]
    fn power_ai_policy_parses_and_decodes_magnitude_verb_from_toml() {
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 10
channel = "helm"
when = "fact(thrust) >= param(thrust_threshold) and fact(battery_pct) >= param(min_reserve_helm)"
verb = "set_power_group_allocation"
level = 3

[[power.ai_policy.rule]]
priority = 0
channel = "ops"
when = "true"
verb = "set_power_group_allocation"
level = 1"#,
        );
        let cfg = EntityConfig::from_toml(&toml).expect("power ai_policy must parse + validate");
        let ai = cfg
            .power
            .as_ref()
            .and_then(|p| p.ai_policy.as_ref())
            .expect("power.ai_policy present");
        let policy = ai.to_policy().expect("policy resolves");
        assert_eq!(policy.rules.len(), 2);
        // The magnitude decodes into the verb payload (AC: absolute level).
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(3)
        );
    }

    #[test]
    fn power_ai_policy_rejects_non_authored_group_channel() {
        // AC1: channels are validated against the ship's AUTHORED power groups.
        // A rule targeting a group the ship does not author fails the load.
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 0
channel = "shields"
when = "true"
verb = "set_power_group_allocation"
level = 2"#,
        );
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn power_ai_policy_rejects_wrong_verb_at_load() {
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 0
channel = "helm"
when = "true"
verb = "focus_shield_arc"
level = 2"#,
        );
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn power_ai_policy_rejects_undeclared_reserve_param() {
        // AC2 / AC6: a guard referencing an undeclared min-reserve param fails.
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 10
channel = "weapons"
when = "fact(battery_pct) >= param(min_reserve_weapons)"
verb = "set_power_group_allocation"
level = 3"#,
        );
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("undeclared parameter"), "got: {err}");
    }

    #[test]
    fn power_ai_policy_rejects_unparseable_guard() {
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 0
channel = "helm"
when = "fact(thrust) >>> broken"
verb = "set_power_group_allocation"
level = 2"#,
        );
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("invalid `when`"), "got: {err}");
    }

    #[test]
    fn power_ai_policy_rejects_empty_declaration() {
        let toml = power_policy_toml("");
        // `[power.ai_policy.param]` present but no rule and no idle → silence.
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("ai policy is empty"), "got: {err}");
    }

    #[test]
    fn shields_idle_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Passive"

[shields_console.ai_policy]
idle = true
"#;
        let cfg = EntityConfig::from_toml(toml).expect("shields idle ai_policy must parse");
        let ai = cfg.shields_console.unwrap().ai_policy.unwrap();
        assert!(ai.to_policy().unwrap().idle);
    }

    #[test]
    fn shields_ai_policy_rejects_wrong_verb_at_load() {
        // A fire verb on the shield_focus channel is an authoring error caught by
        // the from_toml validation loop, before any live tick.
        let toml = r#"
name = "Bad"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "true"
verb = "fire_phaser"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn shields_ai_policy_rejects_unknown_channel_at_load() {
        let toml = r#"
name = "Bad2"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "focus_shield_arc"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn shields_ai_policy_rejects_undeclared_param_at_load() {
        let toml = r#"
name = "Bad3"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "fact(recent_damage_pct_max) >= param(nonexistent)"
verb = "focus_shield_arc"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("undeclared parameter"), "got: {err}");
    }

    // ── Torpedo tube + magazine AI policy (issue #782) ───────────────────────

    #[test]
    fn default_torpedo_tube_and_magazine_policies_validate_and_resolve() {
        let t = default_torpedo_tube_ai_config();
        assert!(
            validate_fine_system_ai_policy(&t, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS).is_ok()
        );
        let tp = t.to_policy().expect("tube default resolves");
        // Baseline: unconditional load + launch (not idle, two rules).
        assert!(!tp.idle);
        assert_eq!(tp.rules.len(), 2);

        let m = default_torpedo_magazine_ai_config();
        assert!(validate_fine_system_ai_policy(
            &m,
            TORPEDO_MAGAZINE_CHANNELS,
            TORPEDO_MAGAZINE_VERBS
        )
        .is_ok());
        assert!(!m.to_policy().expect("magazine default resolves").idle);
    }

    #[test]
    fn torpedo_tube_inline_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Bomber"

[torpedoes]
count = 8

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = 0.0
fire_arc_deg = 90.0

[[torpedoes.tubes.ai.rule]]
priority = 0
channel = "torpedo_load"
when = "fact(magazine) > 0"
verb = "load_torpedo"
value = false

[[torpedoes.tubes.ai.rule]]
priority = 0
channel = "torpedo_launch"
when = "fact(target_facing_shields) <= 0"
verb = "launch_torpedo"
value = false
"#;
        let cfg = EntityConfig::from_toml(toml).expect("tube ai must parse + validate");
        let tube = &cfg.torpedoes.unwrap().tubes[0];
        let policy = tube.ai.as_ref().unwrap().to_policy().unwrap();
        assert_eq!(policy.rules.len(), 2);
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::LoadTorpedo
        );
        assert_eq!(
            policy.rules[1].verb,
            crate::ai::policy::AiPolicyVerb::LaunchTorpedo
        );
    }

    #[test]
    fn torpedo_magazine_inline_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Bomber"

[torpedoes]
count = 8

[[torpedoes.ai.rule]]
priority = 0
channel = "torpedo_magazine_grant"
when = "fact(in_flight) < 3"
verb = "grant_torpedo_round"
value = false
"#;
        let cfg = EntityConfig::from_toml(toml).expect("magazine ai must parse + validate");
        let policy = cfg
            .torpedoes
            .unwrap()
            .ai
            .as_ref()
            .unwrap()
            .to_policy()
            .unwrap();
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::GrantTorpedoRound
        );
    }

    #[test]
    fn torpedo_tube_inline_idle_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Bomber"

[torpedoes]
count = 8

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = 0.0
fire_arc_deg = 90.0

[torpedoes.tubes.ai]
idle = true
"#;
        let cfg = EntityConfig::from_toml(toml).expect("tube idle ai must parse");
        let tube = &cfg.torpedoes.unwrap().tubes[0];
        assert!(tube.ai.as_ref().unwrap().to_policy().unwrap().idle);
    }

    #[test]
    fn torpedo_tube_ai_rejects_unknown_verb_at_load() {
        // The magazine grant verb on a tube channel is an authoring error caught
        // by the from_toml validation loop, before any live tick.
        let toml = r#"
name = "Bad"

[torpedoes]
count = 8

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = 0.0
fire_arc_deg = 90.0

[[torpedoes.tubes.ai.rule]]
priority = 0
channel = "torpedo_load"
when = "true"
verb = "grant_torpedo_round"
value = false
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn torpedo_magazine_ai_rejects_unknown_channel_at_load() {
        // A tube channel on the magazine block is rejected.
        let toml = r#"
name = "Bad"

[torpedoes]
count = 8

[[torpedoes.ai.rule]]
priority = 0
channel = "torpedo_launch"
when = "true"
verb = "grant_torpedo_round"
value = false
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn idle_with_rules_is_contradictory_and_rejected() {
        let cfg = FineSystemAiConfigToml {
            idle: true,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                when: "true".into(),
                verb: CAPTAIN_SET_RED_ALERT_VERB.into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
        assert!(err.contains("idle"), "got: {err}");
    }

    #[test]
    fn invalid_when_expression_is_rejected() {
        let toml = r#"
name = "BadExpr"
[captain_console.ai]
[[captain_console.ai.rule]]
priority = 1
channel = "red_alert"
when = "fact(x) &"
verb = "set_red_alert"
value = true
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(
            err.contains("invalid `when`") || err.contains("position"),
            "got: {err}"
        );
    }

    #[test]
    fn unknown_channel_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: "shields".into(),
                when: "true".into(),
                verb: CAPTAIN_SET_RED_ALERT_VERB.into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn unknown_verb_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                when: "true".into(),
                verb: "launch_torpedoes".into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn undeclared_parameter_reference_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: Default::default(), // no params declared
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                when: "fact(secs_since_combat) < param(combat_window_secs)".into(),
                verb: CAPTAIN_SET_RED_ALERT_VERB.into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
        assert!(err.contains("undeclared parameter"), "got: {err}");
    }

    #[test]
    fn unknown_verb_surfaces_through_to_policy() {
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                when: "true".into(),
                verb: "nope".into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        assert!(cfg.to_policy().is_err());
    }

    // ── Helm Engines/Steering AI policy (issue #779) ─────────────────────────

    const ENGINES_CHANNELS: &[&str] = &[HELM_LONGITUDINAL_CHANNEL];
    const ENGINES_VERBS: &[&str] = &[HELM_ACTUATE_DESIRED_TRAVEL_VERB];
    const STEERING_CHANNELS: &[&str] = &[HELM_YAW_CHANNEL];
    const STEERING_VERBS: &[&str] = &[HELM_ACTUATE_DESIRED_FACING_VERB];

    #[test]
    fn default_helm_policies_validate_and_resolve() {
        let eng = default_engines_ai_config();
        assert!(validate_fine_system_ai_policy(&eng, ENGINES_CHANNELS, ENGINES_VERBS).is_ok());
        let eng_policy = eng.to_policy().expect("engines policy resolves");
        assert_eq!(
            eng_policy.resolve_channel(
                HELM_LONGITUDINAL_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel),
            "the default Engines policy actuates desired travel unconditionally"
        );

        let steer = default_steering_ai_config();
        assert!(validate_fine_system_ai_policy(&steer, STEERING_CHANNELS, STEERING_VERBS).is_ok());
        let steer_policy = steer.to_policy().expect("steering policy resolves");
        assert_eq!(
            steer_policy.resolve_channel(
                HELM_YAW_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing),
        );
    }

    #[test]
    fn authored_helm_policies_parse_and_resolve_to_typed_policy() {
        let toml = r#"
name = "Test Cruiser"

[helm_console]
max_speed = 30.0

[helm_console.engines_ai]
param = { arrival_radius = 5.0 }

[[helm_console.engines_ai.rule]]
priority = 10
channel = "longitudinal"
when = "fact(distance_to_dest) > param(arrival_radius)"
verb = "actuate_desired_travel"

[helm_console.steering_ai]
idle = true
"#;
        let cfg = EntityConfig::from_toml(toml).expect("parse must succeed");
        let hc = cfg.helm_console.as_ref().expect("helm_console present");
        let engines = hc.engines_ai.as_ref().expect("engines_ai present");
        assert_eq!(engines.param.get("arrival_radius"), Some(&5.0));
        let engines_policy = engines.to_policy().expect("engines policy resolves");
        assert_eq!(engines_policy.rules.len(), 1);
        // An explicit idle Steering policy is a legal declaration (a ship whose
        // Steering never AI-actuates), distinct from silence.
        let steering = hc.steering_ai.as_ref().expect("steering_ai present");
        assert!(steering.to_policy().expect("steering resolves").idle);
    }

    #[test]
    fn unknown_helm_engines_verb_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: HELM_LONGITUDINAL_CHANNEL.into(),
                // The Steering verb on the Engines channel is unknown here.
                verb: HELM_ACTUATE_DESIRED_FACING_VERB.into(),
                when: "true".into(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err =
            validate_fine_system_ai_policy(&cfg, ENGINES_CHANNELS, ENGINES_VERBS).unwrap_err();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn helm_wrong_channel_is_rejected() {
        // The Captain's `red_alert` channel is not a valid Steering channel.
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                verb: HELM_ACTUATE_DESIRED_FACING_VERB.into(),
                when: "true".into(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err =
            validate_fine_system_ai_policy(&cfg, STEERING_CHANNELS, STEERING_VERBS).unwrap_err();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn unknown_helm_verb_rejected_at_entity_load() {
        let toml = r#"
name = "BadHelm"
[helm_console]
max_speed = 30.0
[helm_console.engines_ai]
[[helm_console.engines_ai.rule]]
priority = 1
channel = "longitudinal"
when = "true"
verb = "warp_speed"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn empty_helm_engines_declaration_is_rejected_as_silence() {
        let toml = r#"
name = "SilentHelm"
[helm_console]
max_speed = 30.0
[helm_console.engines_ai]
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("empty") || err.contains("idle"), "got: {err}");
    }

    // ── Helm secondary-actuator AI policy (issue #780) ───────────────────────

    const LATERAL_CHANNELS: &[&str] = &[HELM_LATERAL_CHANNEL];
    const LATERAL_VERBS: &[&str] = &[HELM_ACTUATE_LATERAL_THRUST_VERB];
    const VERTICAL_CHANNELS: &[&str] = &[HELM_VERTICAL_CHANNEL];
    const VERTICAL_VERBS: &[&str] = &[HELM_ACTUATE_VERTICAL_THRUST_VERB];
    const IMPULSE_CHANNELS: &[&str] = &[HELM_IMPULSE_CHANNEL];
    const IMPULSE_VERBS: &[&str] = &[HELM_ENGAGE_IMPULSE_VERB];
    const BOOST_CHANNELS: &[&str] = &[HELM_BOOST_CHANNEL];
    const BOOST_VERBS: &[&str] = &[HELM_ENGAGE_BOOST_VERB];

    #[test]
    fn default_secondary_helm_policies_validate_and_resolve() {
        // Lateral / vertical / impulse default to unconditional actuate/permit;
        // boost defaults to explicit idle (no AI boost).
        let lat = default_lateral_ai_config();
        assert!(validate_fine_system_ai_policy(&lat, LATERAL_CHANNELS, LATERAL_VERBS).is_ok());
        assert_eq!(
            lat.to_policy().unwrap().resolve_channel(
                HELM_LATERAL_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateLateralThrust),
        );

        let vert = default_vertical_ai_config();
        assert!(validate_fine_system_ai_policy(&vert, VERTICAL_CHANNELS, VERTICAL_VERBS).is_ok());
        assert_eq!(
            vert.to_policy().unwrap().resolve_channel(
                HELM_VERTICAL_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust),
        );

        let imp = default_impulse_ai_config();
        assert!(validate_fine_system_ai_policy(&imp, IMPULSE_CHANNELS, IMPULSE_VERBS).is_ok());
        assert_eq!(
            imp.to_policy().unwrap().resolve_channel(
                HELM_IMPULSE_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::EngageImpulse),
        );

        let boost = default_boost_ai_config();
        assert!(validate_fine_system_ai_policy(&boost, BOOST_CHANNELS, BOOST_VERBS).is_ok());
        let boost_policy = boost.to_policy().unwrap();
        assert!(
            boost_policy.idle,
            "default boost policy is an explicit idle"
        );
        assert_eq!(
            boost_policy.resolve_channel(
                HELM_BOOST_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            None,
            "an idle boost policy never engages"
        );
    }

    #[test]
    fn authored_secondary_helm_policies_parse_at_entity_load() {
        let toml = r#"
name = "Test Cruiser"

[helm_console]
max_speed = 30.0

[helm_console.boost_ai.param]
boost_urgency = 0.5

[[helm_console.boost_ai.rule]]
priority = 10
channel = "boost"
when = "fact(hazard_urgency) > param(boost_urgency) and fact(boost_available) > 0"
verb = "engage_boost"

[helm_console.impulse_ai]
[[helm_console.impulse_ai.rule]]
priority = 10
channel = "impulse"
when = "fact(impulse_available) > 0"
verb = "engage_impulse"
"#;
        let cfg = EntityConfig::from_toml(toml).expect("parse must succeed");
        let hc = cfg.helm_console.as_ref().expect("helm_console present");
        let boost = hc.boost_ai.as_ref().expect("boost_ai present");
        assert_eq!(boost.to_policy().unwrap().rules.len(), 1);
        let impulse = hc.impulse_ai.as_ref().expect("impulse_ai present");
        assert_eq!(impulse.to_policy().unwrap().rules.len(), 1);
    }

    #[test]
    fn wrong_verb_on_secondary_helm_channel_is_rejected() {
        // The impulse verb on the boost channel is unknown to the boost host.
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: HELM_BOOST_CHANNEL.into(),
                verb: HELM_ENGAGE_IMPULSE_VERB.into(),
                when: "true".into(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn wrong_secondary_helm_verb_rejected_at_entity_load() {
        // Authoring the lateral verb on the vertical channel fails the load.
        let toml = r#"
name = "BadHelm"
[helm_console]
max_speed = 30.0
[helm_console.vertical_ai]
[[helm_console.vertical_ai.rule]]
priority = 1
channel = "vertical"
when = "true"
verb = "actuate_lateral_thrust"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    // ── Sensors target selector schema + validation (issue #776) ─────────────

    fn sensors_selector_toml() -> &'static str {
        r##"
[sensors_console.selector]
horizon = 4000.0
switch_margin = 25.0
sources = ["combat-lock", "objective-destroy", "radar-contacts"]
eligibility = "candidate_fact(detectable) > 0 and candidate_fact(hostile) > 0"

[sensors_console.selector.param]
lock_weight = 900.0

[[sensors_console.selector.score]]
when = "candidate_fact(source_combat_lock) > 0"
weight = 900.0

[[sensors_console.selector.score]]
when = "candidate_fact(source_radar) > 0"
weight = 1.0
"##
    }

    #[test]
    fn sensors_selector_parses_and_resolves_to_typed_selector() {
        let config = EntityConfig::from_toml(sensors_selector_toml()).expect("parse must succeed");
        let sel = config
            .sensors_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
            .expect("selector section present");
        let resolved = sel.to_selector().expect("selector resolves");
        assert_eq!(resolved.horizon, 4000.0);
        assert_eq!(resolved.switch_margin, 25.0);
        assert_eq!(resolved.score.len(), 2);
        assert!(validate_fine_system_ai_selector(sel, SENSORS_SELECTOR_SOURCES).is_ok());
    }

    #[test]
    fn default_sensors_selector_is_valid_and_resolves() {
        let cfg = default_sensors_target_selector_config();
        assert!(validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).is_ok());
        let resolved = cfg.to_selector().expect("default selector resolves");
        assert_eq!(resolved.score.len(), 3);
    }

    #[test]
    fn selector_unknown_source_is_rejected() {
        let mut cfg = default_sensors_target_selector_config();
        cfg.sources.push("mystery-source".into());
        let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("mystery-source"), "got: {err}");
    }

    #[test]
    fn selector_unparseable_eligibility_is_rejected() {
        let mut cfg = default_sensors_target_selector_config();
        cfg.eligibility = "candidate_fact(hostile) >".into();
        let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("eligibility"), "got: {err}");
    }

    #[test]
    fn selector_undeclared_param_reference_is_rejected() {
        let mut cfg = default_sensors_target_selector_config();
        cfg.eligibility = "self_fact(power_rating) >= param(never_declared)".into();
        let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("never_declared"), "got: {err}");
    }

    #[test]
    fn selector_bad_content_fails_entity_load() {
        let bad = r##"
[sensors_console.selector]
horizon = 100.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(detectable) > 0"
"##;
        assert!(
            EntityConfig::from_toml(bad).is_err(),
            "unknown selector source must fail from_toml before world activation"
        );
    }

    // ── Navigation target selector schema + validation (issue #778) ──────────

    fn navigation_selector_toml() -> &'static str {
        r##"
[navigation_console.selector]
horizon = 5000.0
switch_margin = 30.0
sources = ["navigation-objectives", "chart-contacts"]
eligibility = "candidate_fact(reachable) > 0"

[navigation_console.selector.param]
objective_weight = 200.0

[[navigation_console.selector.score]]
when = "candidate_fact(source_nav_objective) > 0"
weight = 200.0

[[navigation_console.selector.score]]
when = "candidate_fact(source_chart_contact) > 0"
weight = 1.0
"##
    }

    #[test]
    fn navigation_selector_parses_and_resolves_to_typed_selector() {
        let config =
            EntityConfig::from_toml(navigation_selector_toml()).expect("parse must succeed");
        let sel = config
            .navigation_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
            .expect("selector section present");
        let resolved = sel.to_selector().expect("selector resolves");
        assert_eq!(resolved.horizon, 5000.0);
        assert_eq!(resolved.switch_margin, 30.0);
        assert_eq!(resolved.score.len(), 2);
        assert!(validate_fine_system_ai_selector(sel, NAVIGATION_SELECTOR_SOURCES).is_ok());
    }

    #[test]
    fn default_navigation_selector_is_valid_and_resolves() {
        let cfg = default_navigation_target_selector_config();
        assert!(validate_fine_system_ai_selector(&cfg, NAVIGATION_SELECTOR_SOURCES).is_ok());
        let resolved = cfg.to_selector().expect("default selector resolves");
        // objective + chart-contact tiers.
        assert_eq!(resolved.score.len(), 2);
    }

    #[test]
    fn navigation_selector_unknown_source_is_rejected() {
        let mut cfg = default_navigation_target_selector_config();
        cfg.sources.push("radar-contacts".into());
        let err = validate_fine_system_ai_selector(&cfg, NAVIGATION_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("radar-contacts"), "got: {err}");
    }

    #[test]
    fn navigation_selector_bad_content_fails_entity_load() {
        let bad = r##"
[navigation_console.selector]
horizon = 100.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(reachable) > 0"
"##;
        assert!(
            EntityConfig::from_toml(bad).is_err(),
            "unknown selector source must fail from_toml before world activation"
        );
    }

    // ── Tactical target selector schema + validation (issue #777) ────────────

    fn tactical_selector_toml() -> &'static str {
        r##"
[weapons_console.selector]
horizon = 3000.0
switch_margin = 40.0
sources = ["sensors-designation", "objective-destroy", "last-attacker", "radar-contacts"]
eligibility = "candidate_fact(detectable) > 0 and (candidate_fact(source_objective) > 0 or candidate_fact(hostile) > 0)"

[weapons_console.selector.param]
sensors_designation_weight = 800.0

[[weapons_console.selector.score]]
when = "candidate_fact(source_sensors_designation) > 0"
weight = 800.0

[[weapons_console.selector.score]]
when = "candidate_fact(source_radar) > 0"
weight = 1.0
"##
    }

    #[test]
    fn tactical_selector_parses_and_resolves_to_typed_selector() {
        let config = EntityConfig::from_toml(tactical_selector_toml()).expect("parse must succeed");
        let sel = config
            .weapons_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
            .expect("selector section present");
        let resolved = sel.to_selector().expect("selector resolves");
        assert_eq!(resolved.horizon, 3000.0);
        assert_eq!(resolved.switch_margin, 40.0);
        assert_eq!(resolved.score.len(), 2);
        assert!(validate_fine_system_ai_selector(sel, TACTICAL_SELECTOR_SOURCES).is_ok());
    }

    #[test]
    fn default_tactical_selector_is_valid_and_resolves() {
        let cfg = default_tactical_target_selector_config();
        assert!(validate_fine_system_ai_selector(&cfg, TACTICAL_SELECTOR_SOURCES).is_ok());
        let resolved = cfg.to_selector().expect("default selector resolves");
        // objective, sensors-designation, retained, last-attacker, radar.
        assert_eq!(resolved.score.len(), 5);
    }

    /// The precedence invariant that prevents the #777 additive-stacking bug:
    /// the objective weight must strictly dominate the maximum non-objective
    /// stack (`sensors_designation + retained + last_attacker + radar`) by more
    /// than `switch_margin`, so an in-range named Destroy objective always wins
    /// the ranking AND survives hysteresis retention — even against the ship's
    /// own current lock coinciding with its Sensors designation.
    #[test]
    fn default_tactical_selector_objective_dominates_max_non_objective_stack() {
        // Const-block asserts: the invariant is over compile-time constants, so
        // this is a static guard, not a runtime check (clippy).
        const {
            let max_non_objective = DEFAULT_TACTICAL_SENSORS_DESIGNATION_WEIGHT
                + DEFAULT_TACTICAL_RETAINED_WEIGHT
                + DEFAULT_TACTICAL_LAST_ATTACKER_WEIGHT
                + DEFAULT_TACTICAL_RADAR_WEIGHT;
            // objective must dominate the max non-objective stack by more than
            // the switch margin — otherwise a stacked non-objective candidate can
            // beat, or be retained over, an explicit Destroy objective (#777).
            assert!(
                max_non_objective
                    < DEFAULT_TACTICAL_OBJECTIVE_WEIGHT - DEFAULT_TACTICAL_SWITCH_MARGIN
            );
            // Retention must still outrank a fresh last attacker so an
            // established engagement is not broken off (retired tier-2 > tier-3).
            assert!(DEFAULT_TACTICAL_RETAINED_WEIGHT > DEFAULT_TACTICAL_LAST_ATTACKER_WEIGHT);
        }
    }

    #[test]
    fn tactical_selector_rejects_combat_lock_source() {
        // `combat-lock` is Tactical's OWN output — unioning it would be
        // circular, so it is not a registered Tactical source.
        let mut cfg = default_tactical_target_selector_config();
        cfg.sources.push(SELECTOR_SOURCE_COMBAT_LOCK.into());
        let err = validate_fine_system_ai_selector(&cfg, TACTICAL_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains(SELECTOR_SOURCE_COMBAT_LOCK), "got: {err}");
    }

    #[test]
    fn tactical_selector_bad_content_fails_entity_load() {
        let bad = r##"
[weapons_console.selector]
horizon = 100.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(detectable) > 0"
"##;
        assert!(
            EntityConfig::from_toml(bad).is_err(),
            "unknown Tactical selector source must fail from_toml before world activation"
        );
    }

    // ── Repair selector (issue #785) ────────────────────────────────────────

    /// BASELINE PRESERVATION: the default Repair selector reproduces the retired
    /// `(tier desc, deficit desc)` comparator, so a single damage-tier step must
    /// strictly dominate the entire deficit ladder.
    #[test]
    fn default_repair_selector_tier_dominates_max_deficit_stack() {
        const {
            // Three bands, each worth one `deficit_weight`.
            let max_deficit_stack = 3.0 * DEFAULT_REPAIR_DEFICIT_WEIGHT;
            assert!(max_deficit_stack < DEFAULT_REPAIR_TIER_WEIGHT);
            // ...and it must survive hysteresis retention too.
            assert!(max_deficit_stack < DEFAULT_REPAIR_TIER_WEIGHT - DEFAULT_REPAIR_SWITCH_MARGIN);
            // The bands are a monotone ladder over the [0,1] damage fraction.
            assert!(DEFAULT_REPAIR_DEFICIT_BAND_LOW < DEFAULT_REPAIR_DEFICIT_BAND_MID);
            assert!(DEFAULT_REPAIR_DEFICIT_BAND_MID < DEFAULT_REPAIR_DEFICIT_BAND_HIGH);
            assert!(DEFAULT_REPAIR_DEFICIT_BAND_HIGH < 1.0);
            // ...and they sit INSIDE the urgent range, strictly above the
            // Damaged→Disabled damage-fraction boundary (1 − 0.25 HP). Bands
            // placed AT the tier thresholds all fire together for every
            // Disabled station and discriminate nothing — see the const doc.
            assert!(DEFAULT_REPAIR_DEFICIT_BAND_LOW > 1.0 - 0.25);
        }
    }

    #[test]
    fn default_repair_selector_config_validates() {
        let cfg = default_repair_target_selector_config();
        assert!(
            validate_fine_system_ai_selector(&cfg, REPAIR_SELECTOR_SOURCES).is_ok(),
            "the canonical Repair selector must validate against its own sources"
        );
        assert!(
            cfg.to_selector().is_ok(),
            "the canonical Repair selector must resolve to a typed selector"
        );
    }

    #[test]
    fn repair_selector_rejects_unregistered_source() {
        let mut cfg = default_repair_target_selector_config();
        cfg.sources.push(SELECTOR_SOURCE_RADAR_CONTACTS.into());
        let err = validate_fine_system_ai_selector(&cfg, REPAIR_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains(SELECTOR_SOURCE_RADAR_CONTACTS), "got: {err}");
    }

    #[test]
    fn repair_selector_undeclared_param_is_rejected() {
        let mut cfg = default_repair_target_selector_config();
        cfg.eligibility = "candidate_fact(damage_fraction) >= param(nope)".to_string();
        let err = validate_fine_system_ai_selector(&cfg, REPAIR_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("nope"), "got: {err}");
    }

    /// `[repair.selector]` is the first selector block outside a `*_console`
    /// section; it parses, and bad content fails the entity load before any
    /// live tick.
    #[test]
    fn repair_selector_parses_from_toml_and_bad_content_fails_entity_load() {
        let good = r##"
[repair]
repair_team_count = 2

[repair.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["damaged-stations", "core-bucket"]
eligibility = "candidate_fact(source_repair_request) > 0"

[[repair.selector.score]]
when = "candidate_fact(tier_ordinal) >= 2"
weight = 100.0
"##;
        let cfg = EntityConfig::from_toml(good).expect("valid [repair.selector] must parse");
        let sel = cfg
            .repair
            .expect("repair section present")
            .selector
            .expect("selector present");
        assert_eq!(sel.sources.len(), 2);
        assert_eq!(sel.score.len(), 1);

        let bad = r##"
[repair]
repair_team_count = 2

[repair.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(source_repair_request) > 0"
"##;
        assert!(
            EntityConfig::from_toml(bad).is_err(),
            "unknown Repair selector source must fail from_toml before world activation"
        );
    }

    #[test]
    fn repair_config_without_selector_defaults_to_none() {
        let cfg = EntityConfig::from_toml("[repair]\nrepair_team_count = 2\n")
            .expect("parse must succeed");
        assert!(cfg.repair.expect("repair present").selector.is_none());
    }

    // ── Comms console AI (issue #786) ───────────────────────────────────────

    /// BAND PLACEMENT (the #785 lesson): the objective-score ladder must be a
    /// strictly increasing set of thresholds that actually straddles the
    /// population of authored `base_priority` values (20 … 100), or every hail
    /// scores identically and the "ranking" collapses onto the selector's
    /// smallest-UUID tie-break.
    #[test]
    fn default_comms_selector_bands_are_a_monotone_ladder_over_real_scores() {
        const {
            assert!(DEFAULT_COMMS_SCORE_BAND_LOW < DEFAULT_COMMS_SCORE_BAND_MID);
            assert!(DEFAULT_COMMS_SCORE_BAND_MID < DEFAULT_COMMS_SCORE_BAND_HIGH);
            // Straddles the shipped authoring range: the lowest band sits above
            // the cheapest authored priority (20) and the highest below the
            // dearest (100), so all four buckets are reachable.
            assert!(DEFAULT_COMMS_SCORE_BAND_LOW > 20.0);
            assert!(DEFAULT_COMMS_SCORE_BAND_HIGH < 100.0);
            // A hail is a one-shot event: nothing to retain, so no hysteresis.
            assert!(DEFAULT_COMMS_SWITCH_MARGIN == 0.0);
        }
    }

    #[test]
    fn default_comms_selector_config_validates() {
        let cfg = default_comms_target_selector_config();
        assert!(
            validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).is_ok(),
            "the canonical Comms selector must validate against its own sources"
        );
        assert!(
            cfg.to_selector().is_ok(),
            "the canonical Comms selector must resolve to a typed selector"
        );
    }

    /// The two eligibility terms the #786 review added, pinned by name so a
    /// future edit cannot quietly drop them:
    ///   - `has_open_hail_thread` (NOT `has_unread_from_sender`) is the
    ///     anti-respam gate — it must key on hails WE issued, or a
    ///     scenario-pushed greeting permanently suppresses a legitimate hail;
    ///   - `self_fact(comms_available)` is the AC2 system-availability gate,
    ///     which the AC names explicitly and which nothing else in the hail path
    ///     enforces.
    #[test]
    fn default_comms_selector_eligibility_names_the_anti_respam_and_availability_gates() {
        let cfg = default_comms_target_selector_config();
        assert!(
            cfg.eligibility
                .contains("candidate_fact(has_open_hail_thread) < 1"),
            "got: {}",
            cfg.eligibility
        );
        assert!(
            !cfg.eligibility.contains("has_unread_from_sender"),
            "inbound traffic of unknown provenance must NOT gate hailing; got: {}",
            cfg.eligibility
        );
        assert!(
            cfg.eligibility.contains("self_fact(comms_available) > 0"),
            "got: {}",
            cfg.eligibility
        );
    }

    #[test]
    fn comms_selector_rejects_unregistered_source() {
        let mut cfg = default_comms_target_selector_config();
        cfg.sources.push(SELECTOR_SOURCE_RADAR_CONTACTS.into());
        let err = validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains(SELECTOR_SOURCE_RADAR_CONTACTS), "got: {err}");
    }

    #[test]
    fn comms_selector_undeclared_param_is_rejected() {
        let mut cfg = default_comms_target_selector_config();
        cfg.eligibility = "candidate_fact(objective_score) >= param(nope)".to_string();
        let err = validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("nope"), "got: {err}");
    }

    #[test]
    fn comms_selector_bad_guard_is_rejected() {
        let mut cfg = default_comms_target_selector_config();
        cfg.eligibility = "candidate_fact(in_range) >>> 0".to_string();
        assert!(validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).is_err());
    }

    /// BASELINE PRESERVATION: the canonical response policy reproduces the
    /// retired `handle_comms_channel2` stub's decision — a single rule answering
    /// with index 0 — while routing it through admission.
    ///
    /// The rule is not `when = "true"`. The stub ran ONLY on channel-2 arrival,
    /// so it could not repeat and its sender was in range by construction; this
    /// policy is re-resolved every tick against every open dialogue. The two
    /// guard terms restore exactly those two implicit preconditions:
    /// `sender_in_range` (or the router rejects the response, forever) and
    /// `comms_available` (AC2 — a Destroyed Comms system answers nothing).
    #[test]
    fn default_comms_response_ai_config_reproduces_the_retired_stub_decision() {
        let cfg = default_comms_response_ai_config();
        assert!(
            validate_fine_system_ai_policy(&cfg, COMMS_RESPOND_CHANNELS, COMMS_RESPOND_VERBS)
                .is_ok(),
            "the canonical Comms response policy must validate"
        );
        assert_eq!(cfg.rule.len(), 1);
        assert_eq!(cfg.rule[0].channel, COMMS_RESPOND_CHANNEL);
        assert_eq!(
            cfg.rule[0].when, "fact(comms_available) > 0 and fact(sender_in_range) > 0",
            "AC2 system availability and the router's range precondition are both \
             named — an unguarded rule re-emits rejected responses every tick"
        );
        assert_eq!(cfg.rule[0].response_index, DEFAULT_COMMS_RESPONSE_INDEX);
        let policy = cfg.to_policy().expect("must resolve to a typed policy");
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::RespondToMessage(0),
            "the authored response_index must ride the verb"
        );
    }

    /// The `response_index` payload decodes onto the verb (the SECOND
    /// value-carrying verb, after `set_power_group_allocation`), and is a
    /// SEPARATE field from `level` so a rule's meaning never depends on its verb.
    #[test]
    fn comms_respond_verb_decodes_its_own_response_index_field() {
        let cfg = FineSystemAiConfigToml {
            idle: false,
            param: std::collections::HashMap::new(),
            rule: vec![FineSystemAiRuleToml {
                priority: 5,
                channel: COMMS_RESPOND_CHANNEL.to_string(),
                when: "true".to_string(),
                verb: COMMS_RESPOND_VERB.to_string(),
                value: true,
                // A non-zero `level` must be ignored by this verb.
                level: 3,
                response_index: 2,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let policy = cfg.to_policy().expect("must resolve");
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::RespondToMessage(2)
        );
    }

    #[test]
    fn comms_response_policy_rejects_wrong_verb_and_unknown_channel() {
        let mut wrong_verb = default_comms_response_ai_config();
        wrong_verb.rule[0].verb = POWER_SET_ALLOCATION_VERB.to_string();
        let err = validate_fine_system_ai_policy(
            &wrong_verb,
            COMMS_RESPOND_CHANNELS,
            COMMS_RESPOND_VERBS,
        )
        .unwrap_err();
        assert!(err.contains(POWER_SET_ALLOCATION_VERB), "got: {err}");

        let mut wrong_channel = default_comms_response_ai_config();
        wrong_channel.rule[0].channel = "shield_focus".to_string();
        let err = validate_fine_system_ai_policy(
            &wrong_channel,
            COMMS_RESPOND_CHANNELS,
            COMMS_RESPOND_VERBS,
        )
        .unwrap_err();
        assert!(err.contains("shield_focus"), "got: {err}");
    }

    #[test]
    fn comms_response_policy_rejects_undeclared_param() {
        let mut cfg = default_comms_response_ai_config();
        cfg.rule[0].when = "fact(response_count) > param(nope)".to_string();
        let err = validate_fine_system_ai_policy(&cfg, COMMS_RESPOND_CHANNELS, COMMS_RESPOND_VERBS)
            .unwrap_err();
        assert!(err.contains("nope"), "got: {err}");
    }

    /// `[comms_console]` carries BOTH machines, parses, and bad content in
    /// either fails the entity load before any live tick.
    #[test]
    fn comms_console_parses_both_blocks_and_bad_content_fails_entity_load() {
        let good = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives", "comms-contacts"]
eligibility = "candidate_fact(source_hail_objective) > 0 and candidate_fact(in_range) > 0"

[[comms_console.selector.score]]
when = "candidate_fact(objective_score) > 0"
weight = 100.0

[[comms_console.ai.rule]]
priority = 10
channel = "comms_respond"
when = "fact(is_urgent) > 0"
verb = "respond_to_message"
response_index = 1
"##;
        let cfg = EntityConfig::from_toml(good).expect("valid [comms_console] must parse");
        let console = cfg.comms_console.expect("comms_console present");
        let sel = console.selector.expect("selector present");
        assert_eq!(sel.sources.len(), 2);
        assert_eq!(sel.score.len(), 1);
        let ai = console.ai.expect("ai present");
        assert_eq!(ai.rule.len(), 1);
        assert_eq!(ai.rule[0].response_index, 1);

        let bad_source = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["radar-contacts"]
eligibility = "candidate_fact(source_hail_objective) > 0"
"##;
        assert!(
            EntityConfig::from_toml(bad_source).is_err(),
            "unknown Comms selector source must fail from_toml"
        );

        let bad_guard = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives"]
eligibility = "candidate_fact(in_range) >>> 0"
"##;
        assert!(
            EntityConfig::from_toml(bad_guard).is_err(),
            "an unparseable Comms selector guard must fail from_toml"
        );

        let undeclared_param = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives"]
eligibility = "candidate_fact(objective_score) > param(nope)"
"##;
        assert!(
            EntityConfig::from_toml(undeclared_param).is_err(),
            "an undeclared selector param must fail from_toml"
        );

        let bad_verb = r##"
[[comms_console.ai.rule]]
priority = 0
channel = "comms_respond"
when = "true"
verb = "set_red_alert"
"##;
        assert!(
            EntityConfig::from_toml(bad_verb).is_err(),
            "a non-Comms verb on the comms_respond channel must fail from_toml"
        );

        let bad_channel = r##"
[[comms_console.ai.rule]]
priority = 0
channel = "not_a_channel"
when = "true"
verb = "respond_to_message"
"##;
        assert!(
            EntityConfig::from_toml(bad_channel).is_err(),
            "an unknown Comms channel must fail from_toml"
        );
    }

    /// `[comms]` (per-entity comms RANGE) and `[comms_console]` (the console's
    /// AI) are deliberately different sections; authoring one must not imply the
    /// other.
    #[test]
    fn comms_range_section_does_not_carry_the_console_ai() {
        let cfg = EntityConfig::from_toml("[comms]\nrange = 8000.0\n").expect("parse must succeed");
        assert!(cfg.comms.is_some());
        assert!(
            cfg.comms_console.is_none(),
            "[comms] is the entity's comms RANGE; the console AI lives in [comms_console]"
        );
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

    // ── NPC red-alert provisioning (issue #749) ─────────────────────────────────

    fn red_alert_systems(config: &EntityConfig) -> Vec<&crate::ship::config::SystemInstanceConfig> {
        config
            .ship_config
            .as_ref()
            .map(|sc| {
                sc.systems
                    .iter()
                    .filter(|s| s.kind == crate::system_registry::RED_ALERT_KIND)
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn behaviour_npc_without_red_alert_gets_ai_only_ownerless_provision() {
        // The Harrow Lancer authors [behaviour] and shield arcs but no
        // red_alert system. Spawn provisioning must add exactly one AI-only,
        // ownerless red_alert capability so the AI captain can raise it.
        let toml_str = include_str!("../../assets/entities/ship_harrow_lancer.toml");
        let config = EntityConfig::from_toml(toml_str).expect("harrow lancer must parse");
        let reds = red_alert_systems(&config);
        assert_eq!(
            reds.len(),
            1,
            "behaviour NPC must be provisioned exactly one red_alert system"
        );
        let sys = reds[0];
        assert_eq!(sys.id.0, crate::system_registry::RED_ALERT_SYSTEM_ID);
        assert!(sys.ai_only, "provisioned red_alert must be ai_only");
        assert!(
            sys.station.is_none(),
            "provisioned red_alert must be ownerless"
        );
    }

    #[test]
    fn pirate_raider_gets_red_alert_provision() {
        // A second behaviour NPC to confirm the provisioning is not lancer-specific.
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate raider must parse");
        let reds = red_alert_systems(&config);
        assert_eq!(reds.len(), 1, "behaviour NPC provisioned one red_alert");
        assert!(reds[0].ai_only && reds[0].station.is_none());
    }

    #[test]
    fn explicit_red_alert_is_left_untouched() {
        // The Alliance Destroyer authors an explicit red_alert system owned by
        // the captain station. Provisioning must be idempotent: no second
        // system, and the authored ownership survives (AC4).
        let toml_str = include_str!("../../assets/entities/alliance_destroyer.toml");
        let config = EntityConfig::from_toml(toml_str).expect("alliance destroyer must parse");
        let reds = red_alert_systems(&config);
        assert_eq!(
            reds.len(),
            1,
            "authored red_alert must not be double-provisioned"
        );
        assert_eq!(
            reds[0].station,
            Some(crate::messages::StationId("captain".into())),
            "authored captain ownership must survive provisioning"
        );
        assert!(
            !reds[0].ai_only,
            "authored player red_alert must remain non-ai_only"
        );
    }

    #[test]
    fn non_behaviour_entity_gets_no_red_alert_provision() {
        // An asteroid has no [behaviour] block → no ship capabilities at all.
        let toml_str = r#"
tags = ["asteroid"]
"#;
        let config = EntityConfig::from_toml(toml_str).expect("asteroid must parse");
        assert!(
            config.ship_config.is_none(),
            "non-behaviour entity must not synthesise a ship_config"
        );
        assert!(
            red_alert_systems(&config).is_empty(),
            "non-behaviour entity must get no red_alert system"
        );
    }
}

// ── Leaf scene-shape types moved from former entities/map_config.rs (PRD #341) ──
// These describe entity-template physical/visual properties consumed by
// EntityConfig (one-per-template) and by steroids::spawner. They are not
// world-tree concerns and so live alongside the entity-template schema rather
// than in world::config.

/// Global configuration block (deterministic seed, lobby metadata, and the
/// shared AI-helm sim-tick rate, all surfaced through WorldConfig).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    /// Master seed for deterministic generation, feeding `SimRng` (issue
    /// #837). `None` — the key omitted — means "draw one from the OS"; it is
    /// not defaulted to a constant, because a constant here would make the
    /// random tier of the seed-precedence chain unreachable.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Display name shown in the lobby title bar.
    #[serde(default)]
    pub title: Option<String>,
    /// Short description shown below the title in the lobby.
    #[serde(default)]
    pub description: Option<String>,
    /// Fixed rate (Hz) of the shared AI-helm sim tick that gates every
    /// per-axis AI helm system (`ai_helm_thrust`, `ai_helm_steering`,
    /// `ai_helm_lateral_thrust`, `ai_helm_impulse`), decoupling AI helm
    /// decision cadence from the host's frame rate (issue #803, PRD #620).
    /// The default matches the old `AiLateralThrustTimer` period.
    #[serde(default = "default_ai_helm_tick_hz")]
    pub ai_helm_tick_hz: f32,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            seed: None,
            title: None,
            description: None,
            ai_helm_tick_hz: default_ai_helm_tick_hz(),
        }
    }
}

fn default_ai_helm_tick_hz() -> f32 {
    30.0
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
