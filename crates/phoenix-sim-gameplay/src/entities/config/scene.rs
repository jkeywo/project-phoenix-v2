//! Entity schema: scene. Public paths remain in the parent module.
use super::*;

// ── Leaf scene-shape types moved from former entities/map_config.rs (PRD #341) ──
// These describe entity-template physical/visual properties consumed by
// EntityConfig (one-per-template) and by steroids::spawner. They are not
// world-tree concerns and so live alongside the entity-template schema rather
// than in world::config.

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

/// One authored asteroid type in a field's type list, with its relative
/// rarity weight (issue #946).
///
/// Two spellings, both valid TOML in the same array:
///
/// ```toml
/// asteroid_type_paths = [
///     "assets/entities/asteroid_common_1_small.toml",
///     { path = "assets/entities/asteroid_rare_1_small.toml", weight = 0.01 },
/// ]
/// ```
///
/// The bare-string form is the pre-#946 schema and still parses — it means
/// exactly `weight = 1.0` — so every field TOML written before rarity
/// existed keeps working untouched.
///
/// Weights are **relative within one list**, not probabilities: an entry at
/// `0.1` is drawn a tenth as often as an entry at `1.0` beside it. Nothing
/// in Rust knows what "common", "uncommon" or "rare" mean; the rarity tiers
/// are entirely a property of the numbers a designer authors here, so a new
/// tier needs no code change. Non-positive weights clamp to zero, and a list
/// whose weights are *all* zero falls back to a uniform draw rather than
/// erasing the field (the same degenerate-authoring guard the field-level
/// `weight` uses).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AsteroidTypeRef {
    /// `"assets/entities/foo.toml"` — an unweighted entry, i.e. weight 1.0.
    Path(String),
    /// `{ path = "assets/entities/foo.toml", weight = 0.1 }`.
    Weighted {
        path: String,
        #[serde(default = "default_asteroid_type_weight")]
        weight: f32,
    },
}

impl AsteroidTypeRef {
    /// The entity template this entry points at.
    pub fn path(&self) -> &str {
        match self {
            Self::Path(path) => path,
            Self::Weighted { path, .. } => path,
        }
    }

    /// The authored rarity weight; `1.0` for the bare-string form.
    pub fn weight(&self) -> f32 {
        match self {
            Self::Path(_) => default_asteroid_type_weight(),
            Self::Weighted { weight, .. } => *weight,
        }
    }
}

impl From<&str> for AsteroidTypeRef {
    fn from(path: &str) -> Self {
        Self::Path(path.to_string())
    }
}

impl From<String> for AsteroidTypeRef {
    fn from(path: String) -> Self {
        Self::Path(path)
    }
}

fn default_asteroid_type_weight() -> f32 {
    1.0
}

/// Configuration for an asteroid field.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AsteroidFieldConfig {
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub density: f32,
    /// Relative weight of this field's contribution to the world's composed
    /// density evaluator (#913). Every authored asteroid-field entity feeds
    /// one shared evaluator; where several fields cover the same lattice
    /// cell their densities and fill thresholds blend proportionally to
    /// weight, and the spawned rock's tuning (type lists, jitter, rotation,
    /// shield pierce) comes from one contributing field picked by the same
    /// weights. `1.0` (the default) makes all fields equal partners. `0.0`
    /// removes a field's influence wherever a positively-weighted field
    /// also covers the cell; if every covering field is zero-weighted the
    /// blend falls back to uniform so a field can never author itself into
    /// a divide-by-zero.
    #[serde(default = "default_field_weight")]
    pub weight: f32,
    #[serde(default = "default_spawn_distance")]
    pub spawn_distance: f32,
    #[serde(default = "default_despawn_distance")]
    pub despawn_distance: f32,
    /// Gameplay (targetable, hulled) asteroid types this field may spawn,
    /// each with its relative rarity weight. See [`AsteroidTypeRef`].
    #[serde(default)]
    pub asteroid_type_paths: Vec<AsteroidTypeRef>,
    /// Backdrop asteroid types for the two cosmetic layers, same shape.
    #[serde(default)]
    pub cosmetic_type_paths: Vec<AsteroidTypeRef>,
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

fn default_field_weight() -> f32 {
    1.0
}

fn default_spawn_distance() -> f32 {
    150.0
}
fn default_despawn_distance() -> f32 {
    250.0
}

pub use phoenix_sim_contracts::scene::*;
