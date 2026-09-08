//! Entity schema: visual. Public paths remain in the parent module.
use super::*;

/// Shape variant for the `[mesh]` section of an entity TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshShape {
    Sphere,
    Cuboid,
    Torus,
}

impl MeshShape {
    /// Parse the lowercase name TOML uses, or `None` for anything else —
    /// including the empty string, which is how the model viewer's panel says
    /// "this level is a GLB, not a shape".
    pub fn parse(name: &str) -> Option<MeshShape> {
        match name {
            "sphere" => Some(MeshShape::Sphere),
            "cuboid" => Some(MeshShape::Cuboid),
            "torus" => Some(MeshShape::Torus),
            _ => None,
        }
    }
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
///
/// **Level of detail is not authored here** (issue #914). The LOD ladder
/// belongs to the model, so it lives in the model's rig sidecar as
/// [`crate::entities::model_rig::ModelRig::lod`]; this section only names the model. The
/// flat fields above remain the fallback every level falls back to. A leftover
/// `[[mesh.lod]]` block is rejected by [`EntityConfig::from_toml`] with a
/// message naming the sidecar it belongs in.
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
    /// RGB colour `[r, g, b]`, sRGB 0–1 — the renderer feeds it straight to
    /// `Color::srgb` (`procedural_mesh_material`).
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
}

fn default_mesh_scale() -> f32 {
    1.0
}

/// Hysteresis margin (world units) applied by [`select_lod`]. Once an entity is
/// showing a given level, the camera distance must move past the band boundary
/// by more than this margin before the level switches. This prevents rapid
/// flip-flopping when the camera hovers exactly on a boundary.
pub const LOD_HYSTERESIS_MARGIN: f32 = 5.0;

/// One distance band in a model rig sidecar's `[[lod]]` chain
/// ([`crate::entities::model_rig::ModelRig::lod`]).
///
/// Levels are declared near→far in ascending `max_distance` order. Each level
/// self-describes as either a GLB level (`model` set) or a procedural level
/// (`shape` set); a level with neither is invalid and is skipped by the
/// renderer. Every visual field is optional — when omitted, the renderer falls
/// back to the corresponding flat [`MeshConfig`] field of the *entity* that
/// named the model, so a level only needs to declare what differs from the
/// shared defaults.
///
/// The type stays here, next to [`select_lod`], because selection is entity
/// rendering logic; only the *authoring location* moved to the sidecar (#914).
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
    /// Path to a billboard atlas `.png` for this band. When set, this is a
    /// **billboard level**: the renderer draws a single camera-facing quad
    /// textured from the atlas (a yaw ring of pre-rendered views of the model),
    /// picking the tile nearest the camera's heading relative to the entity.
    ///
    /// It is the far replacement for a procedural `shape` stand-in — a captured
    /// silhouette of the actual hull reads far better at 400+ than a coloured
    /// sphere — and, because the PNG loads long before a multi-MB GLB, it is
    /// also what shows while the near levels are still streaming in. Mutually
    /// exclusive with `model` and `shape`; the atlas is baked by the model
    /// viewer's capture tool (see `[lod.capture]`).
    #[serde(default)]
    pub billboard: Option<String>,
    /// Procedural shape for this band. Used only when `model` is `None`.
    #[serde(default)]
    pub shape: Option<MeshShape>,
    /// RGB colour `[r, g, b]`, sRGB 0–1 — the renderer feeds it straight to
    /// `Color::srgb` (`procedural_mesh_material`). Falls back to
    /// [`MeshConfig::colour`].
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
    /// Euler `[x, y, z]` rotation in radians for a **procedural** level.
    ///
    /// Applied to the level's own visual, never to the entity: an entity's
    /// rotation is simulation state — physics owns it on anything that moves —
    /// so writing it here would fight the sim every time the level changed. A
    /// GLB level takes its orientation from its rig sidecar's `[base] rotation`
    /// instead, which is why this is ignored for one.
    ///
    /// It exists for the same reason a level has a `scale`: a sphere standing
    /// in for a hull wants to point the way the hull points.
    #[serde(default)]
    pub rotation: Option<[f32; 3]>,
    /// Non-uniform `[x, y, z]` scale for this level, multiplied onto the
    /// entity's own uniform [`MeshConfig::scale`].
    ///
    /// The flat `[mesh]` scale is a single number, which is all a model needs —
    /// but a procedural level is a *stand-in* for a model, and a sphere is the
    /// wrong shape for almost everything it stands in for. Three numbers turn
    /// that sphere into an ellipsoid roughly the proportions of the thing it
    /// replaces, which is the difference between a distant hull reading as a
    /// hull and reading as a ball.
    ///
    /// Applies to GLB levels too, for the same reason a level may override any
    /// other visual field. Omitted means `[1, 1, 1]` — the entity's own scale,
    /// unchanged — and every level recomputes it on switch, so moving between
    /// levels that do and do not declare one is symmetric.
    #[serde(default)]
    pub scale: Option<[f32; 3]>,
    /// How this level's `model` was decimated out of a source GLB (issue #919).
    /// Authored as a `[lod.generate]` sub-table. Build-time provenance only —
    /// see [`LodGeneration`]; the renderer never reads it.
    #[serde(default)]
    pub generate: Option<LodGeneration>,
    /// Whether this level's `model` ships a rig sidecar of its own — the
    /// ladder's *convention*, recorded at build time by whichever pipeline
    /// script wrote the ladder (issue: sidecar-probe-404).
    ///
    /// Only a GENERATED tier carries it: level 0's model IS the primary GLB, so
    /// its sidecar is the one being read right now, and a billboard level has no
    /// GLB to ask about.
    ///
    /// `None` means the sidecar predates the field — a mod pack's hand-authored
    /// ladder, chiefly. The renderer then falls back to probing the tier sidecar
    /// as it always did; see [`crate::entities::glb_visual::resolve_tier_parent_scale`].
    #[serde(default)]
    pub tier_rig: Option<TierRig>,
    /// How this level's `billboard` atlas was captured. Authored as a
    /// `[lod.capture]` sub-table. Build-time provenance only — see
    /// [`LodCapture`]; the renderer never reads it.
    #[serde(default)]
    pub capture: Option<LodCapture>,
}

/// Whether a generated LOD tier's `.glb` ships a rig sidecar beside it.
///
/// Two pipelines write ladders and they differ on this, which is the whole
/// reason [`crate::entities::glb_visual::tier_parent_scale`] exists. The fact
/// was previously *inferred at runtime* by reading the tier's sidecar and
/// seeing what came back — and on wasm "seeing what came back" is an HTTP
/// fetch, so every hull ladder in a browser session begged a 404 for a file
/// deliberately not shipped. It is knowable when the ladder is authored, so it
/// is authored.
///
/// Recorded per level rather than once per ladder because TOML root keys must
/// precede every table header, so a ladder-wide key could not live beside the
/// `[[lod]]` blocks that `scripts/viewer-lods.mjs` rewrites — and a field the
/// ladder writer does not rewrite is a field that goes stale. On the level it
/// rides the same `LEVEL_KEYS` round trip as `model` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierRig {
    /// No sidecar beside this tier's `.glb` — every ship hull, the starbase, the
    /// research outpost. The tier resolves an identity rig, so the parent
    /// transform owes it the whole `[base].scale`. Nothing may fetch its
    /// sidecar: there has never been one to fetch.
    Identity,
    /// A sidecar sits beside this tier's `.glb` carrying the primary's `[base]`
    /// rig verbatim — every asteroid class, written by
    /// `scripts/import-asteroids.mjs`. The tier applies the base scale itself,
    /// so the parent owes it none.
    Baked,
}

/// Decimation parameters that produced a generated LOD level's `.glb`
/// (issue #919), authored as a `[lod.generate]` sub-table on the level.
///
/// **Ignored at runtime.** Every field here is meaningless to the renderer: by
/// the time the game loads the ladder, the decimation has already happened and
/// the only thing that matters is the file on disk. It lives in the sidecar
/// anyway so the sidecar *fully* declares its ladder — the distances, the files,
/// and how those files come back if someone deletes them. The alternative was a
/// second list of ratios inside a build script, which is exactly the hardcoded
/// table this repo does not keep (Key Constraint 11).
///
/// `scripts/generate-lods.mjs` is the one reader: it plans a
/// simplify → resize run per declared level and records the result in
/// `scripts/lod-manifest.toml`. A level with no `[lod.generate]` is authored by
/// hand and the generator leaves it alone.
///
/// ```toml
/// [[lod]]
/// max_distance = 100.0
/// model = "assets/models/asteroid_common_1_lod1.glb"
///
/// [lod.generate]
/// source = "assets/models/asteroid_common_1.glb"
/// ratio = 0.25
/// error = 0.01
/// texture_size = 512
/// ```
///
/// Optional throughout, and deliberately so: the engine must never reject a
/// sidecar over a build-time key it does not use. Validation of the *values*
/// (a missing `source`, a ratio outside 0–1, two sidecars claiming the same
/// output with different parameters) belongs to the generator, which is where
/// the parameters mean something. `deny_unknown_fields` still applies, so a
/// misspelled key fails loudly rather than being silently dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LodGeneration {
    /// The `.glb` this level is decimated from — normally the ladder's own
    /// near level. Omitted means "the first GLB level of this chain".
    #[serde(default)]
    pub source: Option<String>,
    /// meshoptimizer target ratio (0–1) of vertices to keep.
    #[serde(default)]
    pub ratio: Option<f32>,
    /// meshoptimizer error limit, as a fraction of mesh radius.
    #[serde(default)]
    pub error: Option<f32>,
    /// Maximum texture dimension (px) after decimation. Omitted means the
    /// source's textures are carried over untouched.
    #[serde(default)]
    pub texture_size: Option<u32>,
    /// Voxel size for the optional Blender voxel-remesh pre-pass
    /// (`scripts/blender-voxel-remesh.py`), for meshes that decimate badly.
    /// Omitted means no pre-pass, which is the case for every shipped ladder.
    ///
    /// **In the model's own units — the raw GLB geometry, before `[base] scale`.**
    /// Every other number in this file (`max_distance`, `[extents]`) is
    /// post-scale world units, so on a rock scaled 4.2x the two are nothing
    /// alike: `1.0` against an extent of 8 looks small and in fact spans half
    /// the mesh, which remeshes the asteroid into a cube. Divide the extent by
    /// the base scale to see what the model measures, then take a small
    /// fraction of that (a sixty-fourth is a reasonable start).
    #[serde(default)]
    pub remesh_voxel_size: Option<f32>,
}

/// Capture parameters that produced a billboard level's atlas `.png`, authored
/// as a `[lod.capture]` sub-table on the level.
///
/// **Ignored at runtime**, exactly like [`LodGeneration`]: once the atlas is on
/// disk the renderer only needs the file. It lives in the sidecar so the ladder
/// *fully* declares how the atlas comes back — the model viewer's capture tool
/// (`src/viewer/capture.rs`, reached from the LOD panel) is the one reader, and
/// re-baking needs a GPU + the browser viewer, so like the Blender voxel pre-pass
/// this is a local step. CI only re-hashes it against the separate capture
/// provenance contract in `scripts/lod-capture-manifest.toml` (#1245); generated
/// GLB provenance remains independently owned by `scripts/lod-manifest.toml`.
///
/// ```toml
/// [[lod]]
/// billboard = "assets/models/alliance_battleship_lod3.png"
/// scale = [11.3, 12.0, 1.0]   # world width/height of the quad
///
/// [lod.capture]
/// source = "assets/models/alliance_battleship.glb"
/// yaw_views = 8       # tiles around a horizontal ring, packed left→right
/// resolution = 256    # per-tile pixels (square)
/// pitch = 20.0        # camera pitch in degrees above the ring plane
/// ```
///
/// Optional throughout for the same reason [`LodGeneration`] is: the engine must
/// never reject a sidecar over a build-time key it does not use. `deny_unknown_fields`
/// still applies, so a misspelled key fails the build rather than being dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LodCapture {
    /// The `.glb` the atlas was rendered from — the ladder's near level.
    #[serde(default)]
    pub source: Option<String>,
    /// Number of yaw views packed into the atlas, left→right (a horizontal ring).
    #[serde(default)]
    pub yaw_views: Option<u32>,
    /// Per-tile resolution in pixels (square tiles).
    #[serde(default)]
    pub resolution: Option<u32>,
    /// Camera pitch in degrees above the ring plane the views were rendered at.
    #[serde(default)]
    pub pitch: Option<f32>,
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

/// Reject an entity TOML that still authors `[[mesh.lod]]` (issue #914).
///
/// The ladder now lives in the model's rig sidecar. Silently ignoring the old
/// location would leave an author convinced they had a ladder while the entity
/// rendered a single level forever, and letting `deny_unknown_fields` handle it
/// yields "unknown field `lod`" — true, but it does not say where the field
/// went. So the check runs first and names the exact sidecar file whenever the
/// `[mesh]` section identifies a model, because "move it to the sidecar" is
/// only actionable if you know *which* sidecar.
pub(super) fn reject_relocated_mesh_lod(value: &toml::Value) -> Result<(), toml::de::Error> {
    let Some(mesh) = value.get("mesh") else {
        return Ok(());
    };
    if mesh.get("lod").is_none() {
        return Ok(());
    }
    let sidecar = mesh
        .get("model")
        .and_then(|m| m.as_str())
        .map(|model| {
            crate::entities::model_rig::sidecar_path(
                model,
                mesh.get("variant").and_then(|v| v.as_str()),
            )
        })
        .unwrap_or_else(|| "assets/models/<model>.<variant>.toml".to_string());
    Err(SerdeError::custom(format!(
        "[[mesh.lod]] has moved to the model rig sidecar (issue #914): author the \
         chain as [[lod]] blocks in {sidecar} and delete it from the entity TOML. \
         The entity's [mesh] keeps the model reference and the flat fallback fields \
         that sidecar levels fall back to."
    )))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppearanceConfig {
    pub colour: String,
    pub size_min: f32,
    pub size_max: f32,
}

/// Radar icon name injected onto the player's own ship at game-start spawn
/// (see `player_ship_identity` in `src/server_app.rs`, which writes it into
/// [`RadarAppearanceConfig::icon`]). Because it is injected at spawn rather than
/// authored in any hull template, the preload scan never discovers it — so the
/// presentation preloader (`crate::server::asset_preload`) reads this same
/// constant to load the icon unconditionally.
///
/// Lives here — sim-side, beside the [`RadarAppearanceConfig`] it fills — rather
/// than in the presentation `asset_preload` module (issue #1194): the player
/// ship's radar identity is authoritative spawn data set by always-compiled
/// code, so the `--server` feature gate must not be able to compile it out.
/// Keep the injection site and the preloader in sync through this one constant.
pub const PLAYER_SHIP_RADAR_ICON: &str = "playerShip";

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
