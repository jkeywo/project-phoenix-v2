//! Per-model "rig" sidecar: runtime types + pure parser.
//!
//! The editor writes one TOML sidecar next to each `.glb` model under
//! `assets/models/`. The sidecar corrects a raw GLB into game space (the
//! `[base]` rig) and names mount points on the model (the `[markers.<name>]`
//! map) so gameplay systems can attach beams, torpedoes, exhaust, etc. to
//! authored positions instead of hardcoded hull offsets.
//!
//! # Schema (the editor emits exactly this)
//! ```toml
//! [base]                  # corrects raw GLB into game space; applied INNER
//! offset = [0.0,0.0,0.0]    # non-uniform vec3
//! rotation = [0.0,0.0,0.0]  # XYZ-order euler radians
//! scale = [1.0,1.0,1.0]     # non-uniform vec3
//! [extents]               # cached bounds in post-base-rig space (advisory)
//! min = [-4.0,-1.2,-6.0]
//! max = [4.0,1.2,6.0]
//! size = [8.0,2.4,12.0]
//! [base_build]            # how the base .glb is cut from raw art (build-time)
//! budget_mb = 16.0
//! [markers.fore_emitter]  # free-form name -> single point (raw-GLB space)
//! position = [0.0,0.0,-6.0]
//! direction = [0.0,0.0,-1.0]  # unit vector, forward basis (0,0,-1)
//!
//! [[target_points]]       # anonymous phaser hit points enemies can aim at
//! position = [0.5,-0.1,0.0]
//!
//! [[lod]]                 # distance-based LOD chain, ordered near→far
//! max_distance = 50.0       # exclusive upper bound; omit on the last level
//! model = "assets/models/rock.glb"
//! variant = "large"         # omitted → this sidecar's own variant
//!
//! [[lod]]
//! max_distance = 150.0
//! model = "assets/models/rock_lod2.glb"
//! [lod.generate]          # how that .glb is regenerated (build-time only)
//! source = "assets/models/rock.glb"
//! ratio = 0.05
//! error = 0.1
//! texture_size = 256
//!
//! [[lod]]                 # procedural fallback level (no `model`)
//! shape = "sphere"
//! ```
//!
//! # Level of detail (issue #914)
//! The LOD ladder belongs to the **model**, not to the entity: a rock is a rock
//! whichever template spawns it. `[[lod]]` therefore lives here, beside the
//! `.glb` it decimates, and the entity's `[mesh]` only names the model. Entity
//! TOML that still authors `[[mesh.lod]]` is rejected at parse with a message
//! pointing at this file — see [`crate::entities::config::EntityConfig::from_toml`].
//!
//! # Regenerating a ladder (issue #919)
//! A level whose `.glb` was decimated out of another one carries the parameters
//! that produced it in a `[lod.generate]` sub-table
//! ([`crate::entities::config::LodGeneration`]), so the sidecar declares not just
//! *which* files the ladder uses but *how they come back*:
//! `node scripts/generate-lods.mjs <model>` reads exactly these blocks. The
//! engine ignores every one of those keys — they are build-time provenance —
//! but the strict schema still applies, so a misspelling fails the build rather
//! than quietly detaching a level from its generator.
//!
//! # Composition
//! The base rig is applied *inner* to the per-entity transform. The renderer
//! spawns the GLB `SceneRoot` as a CHILD carrying `base_bevy_transform()`,
//! while the per-entity `Transform` (spawn position + per-entity scale /
//! rotation) stays on the parent. Net world transform of the model is
//! `entityTransform ∘ baseRig ∘ model`. Marker positions are authored in
//! raw-GLB space, so resolving a marker to
//! world space means applying `entityTransform ∘ baseRig` to the marker point.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use bevy::math::{EulerRot, Quat, Vec3};
use bevy::prelude::{Component, Transform};

fn zeros() -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

fn ones() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

/// The `[base]` rig: corrects a raw GLB into game space. All fields default so
/// a sparse or empty sidecar parses to an identity rig.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaseTransform {
    /// Non-uniform translation applied to the model, in model-local units.
    #[serde(default = "zeros")]
    pub offset: [f32; 3],
    /// XYZ-order euler rotation in radians.
    #[serde(default = "zeros")]
    pub rotation: [f32; 3],
    /// Non-uniform scale applied to the model.
    #[serde(default = "ones")]
    pub scale: [f32; 3],
}

impl Default for BaseTransform {
    fn default() -> Self {
        BaseTransform {
            offset: zeros(),
            rotation: zeros(),
            scale: ones(),
        }
    }
}

/// Cached bounds of the model in post-base-rig space. Advisory: the engine may
/// store these but need not act on them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extents {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub size: [f32; 3],
}

/// How the base (level-0) `.glb` is cut from the raw art export: the byte
/// budget `scripts/optimise-base.mjs` searches against.
///
/// Build-time provenance, like `[lod.generate]` beside it — the engine parses
/// it and acts on none of it. It exists because the base level is built from
/// art that is NOT in this repository, by a script whose budget was otherwise
/// only ever a command-line argument. A model rebuilt without the number that
/// built it silently reverts to the 8 MB default, and for
/// `alliance_starbase` that is the difference between a 2048px base-colour map
/// and a 512px one — the detail loss John reported once the LOD-scale fix
/// (ed31e485) let the model render at its true size.
///
/// Absent means "the script's own default", so only a model that needs a
/// different budget has to say so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaseBuild {
    /// Byte budget in MB for the base `.glb`, as `optimise-base --budget-mb`.
    pub budget_mb: f32,
}

/// A single named mount point in raw-GLB space. `direction` is a unit vector
/// with forward basis `(0,0,-1)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub position: [f32; 3],
    pub direction: [f32; 3],
}

/// A single anonymous target point in raw-GLB space.
///
/// Phaser PFX can resolve one of these points on the target model so beams hit
/// plausible hull positions instead of always converging on the entity centre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetPoint {
    pub position: [f32; 3],
}

/// A parsed model-rig sidecar.
///
/// `deny_unknown_fields` throughout (issue #914): a sidecar is authored by hand
/// and by the editor, and a mistyped key must fail loudly rather than resolve to
/// an identity rig with a silently missing marker or LOD ladder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ModelRig {
    /// The base rig. Defaults to identity when the `[base]` section is absent.
    #[serde(default)]
    pub base: BaseTransform,
    /// Cached bounds, when present.
    #[serde(default)]
    pub extents: Option<Extents>,
    /// How the base `.glb` is cut from the raw art, when the model needs
    /// anything other than `optimise-base`'s default budget.
    #[serde(default)]
    pub base_build: Option<BaseBuild>,
    /// Free-form marker name -> mount point. smol-toml (editor) and the `toml`
    /// crate (engine) both expand `[markers.<name>]` subtables into this map.
    #[serde(default)]
    pub markers: HashMap<String, Marker>,
    /// Anonymous target points that incoming phaser beams can choose from.
    #[serde(default)]
    pub target_points: Vec<TargetPoint>,
    /// Distance-based level-of-detail bands for this model, ordered near→far
    /// (issue #914). Authored as `[[lod]]` blocks.
    ///
    /// When non-empty, an entity whose `[mesh]` names this model is NOT
    /// rendered from its flat `[mesh]` fields; the renderer picks a level each
    /// frame from the camera distance (see
    /// [`crate::entities::config::select_lod`]) and builds that level instead.
    /// Fields a level omits fall back to the *entity's* flat `[mesh]` fields
    /// (`colour`/`radius`/`emissive`/`size`/`minor_radius`/`variant`), so one
    /// shared ladder still renders differently-tinted rocks correctly.
    /// Empty (the default) means "no ladder" — the flat `[mesh]` renders as-is.
    #[serde(default)]
    pub lod: Vec<crate::entities::config::LodLevel>,
}

impl ModelRig {
    /// Parse a rig sidecar from a TOML string. A sparse or empty document
    /// yields an identity base rig and no markers.
    pub fn from_toml(toml_str: &str) -> Result<ModelRig, toml::de::Error> {
        toml::from_str(toml_str)
    }

    /// Build the Bevy `Transform` for the base rig: `offset` → translation,
    /// `rotation` → XYZ-order euler quat, `scale` → non-uniform scale.
    pub fn base_bevy_transform(&self) -> bevy::prelude::Transform {
        self.base.bevy_transform()
    }

    /// Resolve a marker by name. Returns `None` when the marker is missing, so
    /// callers fall back to their default origin.
    pub fn marker(&self, name: &str) -> Option<&Marker> {
        self.markers.get(name)
    }
}

impl BaseTransform {
    /// Build the Bevy `Transform` for this base rig.
    pub fn bevy_transform(&self) -> bevy::prelude::Transform {
        bevy::prelude::Transform {
            translation: Vec3::from_array(self.offset),
            rotation: Quat::from_euler(
                EulerRot::XYZ,
                self.rotation[0],
                self.rotation[1],
                self.rotation[2],
            ),
            scale: Vec3::from_array(self.scale),
        }
    }
}

/// Authoritative ECS component carrying the marker and target geometry from an
/// entity's **primary authored rig**.
///
/// [`crate::entities::model_markers::sync_authoritative_model_markers`]
/// attaches it independently of rendering. Active GLB/shape/billboard LODs are
/// presentation and must never replace or remove it, so weapons and every
/// deterministic peer resolve the same geometry at every visual distance.
#[derive(Component, Debug, Clone, Default)]
pub struct ModelMarkers {
    markers: HashMap<String, Marker>,
    target_points: Vec<TargetPoint>,
    /// The base rig (`entityTransform ∘ baseRig ∘ model`). Marker positions
    /// are authored in the raw-GLB frame, so resolving one to ship-local space
    /// means applying `baseRig` first, then the entity transform. Defaults to
    /// identity for `from_markers` (test fixtures already in ship space).
    base: Transform,
}

impl ModelMarkers {
    pub fn from_rig(rig: &ModelRig) -> Self {
        Self {
            markers: rig.markers.clone(),
            target_points: rig.target_points.clone(),
            base: rig.base_bevy_transform(),
        }
    }

    pub fn from_markers(markers: HashMap<String, Marker>) -> Self {
        Self {
            markers,
            target_points: Vec::new(),
            base: Transform::IDENTITY,
        }
    }

    /// The base rig transform for this model (`baseRig`). Callers that resolve
    /// marker positions or directions manually (e.g. the camera rig) apply this
    /// inner to the entity transform.
    pub fn base(&self) -> Transform {
        self.base
    }

    /// Resolve a marker by name (None when missing -> caller falls back).
    pub fn get(&self, name: &str) -> Option<&Marker> {
        self.markers.get(name)
    }

    /// Resolve a marker by name to a world-space position, composing the
    /// entity's live `Transform` with the base rig and the marker's raw-GLB
    /// position (`entityTransform ∘ baseRig ∘ marker`). Returns `None` when the
    /// marker is missing so callers fall back to their default origin.
    pub fn resolve_world_position(&self, transform: &Transform, name: &str) -> Option<Vec3> {
        let marker = self.get(name)?;
        let local = self.base.transform_point(Vec3::from_array(marker.position));
        Some(transform.transform_point(local))
    }

    /// Resolve a marker direction to world space through the same composed
    /// transform as its position. Scale is included because it changes the
    /// direction of non-axis-aligned vectors under a non-uniform base rig.
    pub fn resolve_world_direction(&self, transform: &Transform, name: &str) -> Option<Vec3> {
        let marker = self.get(name)?;
        let local = self.base.rotation * (self.base.scale * Vec3::from_array(marker.direction));
        let direction = transform.rotation * (transform.scale * local);
        (direction.length_squared() > 1e-6).then_some(direction.normalize())
    }

    /// Resolve an anonymous phaser target point to world space.
    pub fn resolve_target_point_world_position(
        &self,
        transform: &Transform,
        index: usize,
    ) -> Option<Vec3> {
        let point = self.target_point(index)?;
        let local = self.base.transform_point(Vec3::from_array(point.position));
        Some(transform.transform_point(local))
    }

    /// Iterate over all marker names in this model rig.
    pub fn marker_names(&self) -> impl Iterator<Item = &str> {
        self.markers.keys().map(|s| s.as_str())
    }

    pub fn target_point(&self, index: usize) -> Option<&TargetPoint> {
        self.target_points.get(index)
    }

    pub fn target_point_count(&self) -> usize {
        self.target_points.len()
    }
}

/// The reserved default variant name used when an entity's `[mesh]` does not
/// specify a `variant`.
pub const DEFAULT_VARIANT: &str = "model";

/// Pure path helper: produce the sidecar path for a model.
///
/// `assets/models/<stem>.<variant-or-"model">.toml`. The model path's
/// directory and `assets/` prefix are preserved; only the final `.glb`
/// extension is replaced with `.<variant>.toml`. A `variant` of `Some("model")`
/// is treated the same as the default.
///
/// # Examples
/// * `("assets/models/dynasty_destroyer.glb", None)`
///   → `assets/models/dynasty_destroyer.model.toml`
/// * `("assets/models/dynasty_destroyer.glb", Some("weathered"))`
///   → `assets/models/dynasty_destroyer.weathered.toml`
pub fn sidecar_path(model_path: &str, variant: Option<&str>) -> String {
    let variant = variant.unwrap_or(DEFAULT_VARIANT);
    // Strip a trailing ".glb" (case-insensitive) so the stem is clean; if the
    // path has some other / no extension, just append.
    let stem = match model_path
        .to_ascii_lowercase()
        .strip_suffix(".glb")
        .map(|_| &model_path[..model_path.len() - 4])
    {
        Some(s) => s,
        None => model_path,
    };
    format!("{stem}.{variant}.toml")
}

/// Pure path helper: the inverse of [`sidecar_path`] — which variant a sidecar
/// path names.
///
/// `assets/models/asteroid_common_1.large.toml` → `Some("large")`;
/// `assets/models/dynasty_destroyer.model.toml` → `Some("model")`. `None` when
/// the path is not a `<stem>.<variant>.toml` sidecar at all.
///
/// Used when a sidecar's own `[[lod]]` level omits `variant`: the level then
/// inherits the variant of the sidecar it was declared in, which is exactly the
/// variant the entity's `[mesh]` used to reach that sidecar — so the preload
/// walk and the renderer's `MeshConfig::variant` fallback agree by construction.
pub fn sidecar_variant(sidecar: &str) -> Option<&str> {
    let file = sidecar.rsplit(['/', '\\']).next()?;
    let stem = file.strip_suffix(".toml")?;
    let (base, variant) = stem.rsplit_once('.')?;
    if base.is_empty() || variant.is_empty() {
        return None;
    }
    Some(variant)
}

#[cfg(test)]
#[path = "model_rig_tests.rs"]
mod tests;
