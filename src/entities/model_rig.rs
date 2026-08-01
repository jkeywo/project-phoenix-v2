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
//! [markers.fore_emitter]  # free-form name -> single point (post-base-rig space)
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
//! pointing at this file — see [`crate::entity_config::EntityConfig::from_toml`].
//!
//! # Regenerating a ladder (issue #919)
//! A level whose `.glb` was decimated out of another one carries the parameters
//! that produced it in a `[lod.generate]` sub-table
//! ([`crate::entity_config::LodGeneration`]), so the sidecar declares not just
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
//! post-base-rig (model-local, base-applied) space, so resolving a marker to
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

/// A single named mount point in post-base-rig (model-local, base-applied)
/// space. `direction` is a unit vector with forward basis `(0,0,-1)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub position: [f32; 3],
    pub direction: [f32; 3],
}

/// A single anonymous target point in post-base-rig space.
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
    /// [`crate::entity_config::select_lod`]) and builds that level instead.
    /// Fields a level omits fall back to the *entity's* flat `[mesh]` fields
    /// (`colour`/`radius`/`emissive`/`size`/`minor_radius`/`variant`), so one
    /// shared ladder still renders differently-tinted rocks correctly.
    /// Empty (the default) means "no ladder" — the flat `[mesh]` renders as-is.
    #[serde(default)]
    pub lod: Vec<crate::entity_config::LodLevel>,
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

/// ECS component carrying a model's resolved rig points so downstream systems
/// can look up mount points and target points without re-reading the sidecar.
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
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    fn approx(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < EPS)
    }

    #[test]
    fn parses_full_sidecar() {
        let toml = r##"
[base]
offset = [1.0, 2.0, 3.0]
rotation = [0.1, 0.2, 0.3]
scale = [2.0, 1.0, 0.5]

[extents]
min = [-4.0, -1.2, -6.0]
max = [4.0, 1.2, 6.0]
size = [8.0, 2.4, 12.0]

[markers.fore_emitter]
position = [0.0, 0.0, -6.0]
direction = [0.0, 0.0, -1.0]

[markers.aft_exhaust]
position = [0.0, 0.0, 6.0]
direction = [0.0, 0.0, 1.0]

[[target_points]]
position = [0.5, -0.1, 0.0]

[[target_points]]
position = [-0.25, -0.1, 0.25]
"##;
        let rig = ModelRig::from_toml(toml).expect("full sidecar must parse");
        assert!(approx(rig.base.offset, [1.0, 2.0, 3.0]));
        assert!(approx(rig.base.rotation, [0.1, 0.2, 0.3]));
        assert!(approx(rig.base.scale, [2.0, 1.0, 0.5]));

        let ext = rig.extents.as_ref().expect("extents present");
        assert!(approx(ext.min, [-4.0, -1.2, -6.0]));
        assert!(approx(ext.max, [4.0, 1.2, 6.0]));
        assert!(approx(ext.size, [8.0, 2.4, 12.0]));

        assert_eq!(rig.markers.len(), 2);
        let fore = rig.marker("fore_emitter").expect("fore marker present");
        assert!(approx(fore.position, [0.0, 0.0, -6.0]));
        assert!(approx(fore.direction, [0.0, 0.0, -1.0]));
        assert_eq!(rig.target_points.len(), 2);
        assert!(approx(rig.target_points[0].position, [0.5, -0.1, 0.0]));
        assert!(approx(rig.target_points[1].position, [-0.25, -0.1, 0.25]));
    }

    #[test]
    fn sparse_sidecar_uses_base_defaults() {
        // Only a partial [base] — missing fields default.
        let toml = r##"
[base]
offset = [5.0, 0.0, 0.0]
"##;
        let rig = ModelRig::from_toml(toml).expect("sparse sidecar must parse");
        assert!(approx(rig.base.offset, [5.0, 0.0, 0.0]));
        // rotation defaults to zeros, scale defaults to ones.
        assert!(approx(rig.base.rotation, [0.0, 0.0, 0.0]));
        assert!(approx(rig.base.scale, [1.0, 1.0, 1.0]));
        assert!(rig.extents.is_none());
        assert!(rig.markers.is_empty());
        assert!(rig.target_points.is_empty());
    }

    #[test]
    fn empty_sidecar_is_identity_rig() {
        let rig = ModelRig::from_toml("").expect("empty sidecar must parse");
        assert_eq!(rig.base, BaseTransform::default());
        assert!(approx(rig.base.offset, [0.0, 0.0, 0.0]));
        assert!(approx(rig.base.scale, [1.0, 1.0, 1.0]));
        assert!(rig.extents.is_none());
        assert!(rig.markers.is_empty());
        assert!(rig.target_points.is_empty());
    }

    #[test]
    fn markers_subtable_form_parses() {
        // The `[markers.<name>]` subtable form (what smol-toml emits) parses
        // into the flat map.
        let toml = r##"
[markers.port_bank]
position = [-4.0, 0.0, -2.0]
direction = [-1.0, 0.0, 0.0]
"##;
        let rig = ModelRig::from_toml(toml).expect("markers subtable must parse");
        assert_eq!(rig.markers.len(), 1);
        let m = rig.marker("port_bank").expect("port_bank present");
        assert!(approx(m.position, [-4.0, 0.0, -2.0]));
        assert!(approx(m.direction, [-1.0, 0.0, 0.0]));
    }

    #[test]
    fn sidecar_path_default_variant() {
        assert_eq!(
            sidecar_path("assets/models/dynasty_destroyer.glb", None),
            "assets/models/dynasty_destroyer.model.toml"
        );
        // `Some("model")` is treated as the reserved default name.
        assert_eq!(
            sidecar_path("assets/models/dynasty_destroyer.glb", Some("model")),
            "assets/models/dynasty_destroyer.model.toml"
        );
    }

    #[test]
    fn sidecar_path_named_variant() {
        assert_eq!(
            sidecar_path("assets/models/dynasty_destroyer.glb", Some("weathered")),
            "assets/models/dynasty_destroyer.weathered.toml"
        );
    }

    #[test]
    fn sidecar_path_handles_uppercase_extension() {
        assert_eq!(
            sidecar_path("assets/models/Ship.GLB", None),
            "assets/models/Ship.model.toml"
        );
    }

    #[test]
    fn marker_resolve_hit_and_miss() {
        let toml = r##"
[markers.fore_emitter]
position = [0.0, 0.0, -6.0]
direction = [0.0, 0.0, -1.0]
"##;
        let rig = ModelRig::from_toml(toml).unwrap();
        assert!(rig.marker("fore_emitter").is_some());
        assert!(rig.marker("nope").is_none());

        // ModelMarkers component resolves identically.
        let mm = ModelMarkers::from_rig(&rig);
        assert!(mm.get("fore_emitter").is_some());
        assert!(mm.get("missing").is_none());
    }

    #[test]
    fn target_points_array_form_parses() {
        let toml = r##"
[[target_points]]
position = [0.5, -0.1, 0.0]

[[target_points]]
position = [-0.25, -0.1, -0.25]
"##;
        let rig = ModelRig::from_toml(toml).expect("target points must parse");
        assert_eq!(rig.target_points.len(), 2);
        assert!(approx(rig.target_points[0].position, [0.5, -0.1, 0.0]));
        assert!(approx(rig.target_points[1].position, [-0.25, -0.1, -0.25]));

        let mm = ModelMarkers::from_rig(&rig);
        assert_eq!(mm.target_point_count(), 2);
        assert!(approx(
            mm.target_point(1).unwrap().position,
            [-0.25, -0.1, -0.25]
        ));
    }

    #[test]
    fn base_bevy_transform_identity_for_default() {
        let rig = ModelRig::default();
        let t = rig.base_bevy_transform();
        assert!((t.translation - Vec3::ZERO).length() < EPS);
        assert!((t.scale - Vec3::ONE).length() < EPS);
        assert!((t.rotation.angle_between(Quat::IDENTITY)).abs() < EPS);
    }

    // ── Real-asset linkage (alliance_destroyer → its own sidecar) ─────────────

    /// End-to-end (pure, native) proof of marker linkage on a realistic test
    /// target: the Alliance Destroyer's `omni` phaser bank carries `marker =
    /// "phasers_omni"`, its `[mesh] model` resolves to a sidecar that defines a
    /// `phasers_omni` marker, and the resolver returns it.
    ///
    /// (#892) This ran against `pirate_raider.toml` → `dynasty_destroyer.model.toml`
    /// until that hull was retired as a duplicate. It was the ONLY entity
    /// referencing either `dynasty_destroyer.glb` or its `fore_emitter` marker,
    /// so there is no like-for-like replacement: the sidecar is now orphaned
    /// content. The linkage claim itself is hull-agnostic, so it moves to a
    /// shipped (hull, sidecar, marker) triple that still exists.
    #[test]
    fn alliance_destroyer_omni_bank_marker_resolves_in_sidecar() {
        // Through the include resolver (issue #906) so the linkage claim keeps
        // holding once the hull is composed.
        let cfg =
            crate::entity_includes::load_entity_config("assets/entities/alliance_destroyer.toml")
                .expect("alliance_destroyer must compose and parse");

        // The omni bank links to a marker.
        let weapons = cfg
            .weapons_console
            .as_ref()
            .expect("weapons_console present");
        let omni_bank = weapons
            .phaser_banks
            .iter()
            .find(|b| b.id == "omni")
            .expect("omni bank present");
        let marker_name = omni_bank
            .marker
            .as_deref()
            .expect("omni bank carries a marker name");
        assert_eq!(marker_name, "phasers_omni");

        // The mesh model resolves to a sidecar (default variant).
        let mesh = cfg.mesh.as_ref().expect("mesh present");
        let model_path = mesh.model.as_deref().expect("model path present");
        let path = sidecar_path(model_path, mesh.variant.as_deref());
        assert_eq!(path, "assets/models/alliance_destroyer.model.toml");

        // Parse the sidecar and resolve the linked marker.
        let rig_toml =
            std::fs::read_to_string(&path).expect("alliance_destroyer sidecar must exist");
        let rig = ModelRig::from_toml(&rig_toml).expect("sidecar must parse");
        rig.marker(marker_name)
            .expect("phasers_omni marker must resolve in the sidecar");

        // Missing marker → None (caller falls back to origin).
        assert!(rig.marker("does_not_exist").is_none());
    }

    // ── Sidecar-owned LOD chains (issue #914) ────────────────────────────

    #[test]
    fn lod_chain_parses_from_sidecar_toml() {
        let toml = r##"
[base]
scale = [2.0, 2.0, 2.0]

[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"
variant = "small"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod2.glb"

[[lod]]
shape = "sphere"
"##;
        let rig = ModelRig::from_toml(toml).expect("a sidecar ladder must parse");
        assert_eq!(rig.lod.len(), 3);
        assert_eq!(rig.lod[0].max_distance, Some(50.0));
        assert_eq!(rig.lod[0].model.as_deref(), Some("assets/models/rock.glb"));
        assert_eq!(rig.lod[0].variant.as_deref(), Some("small"));
        // A level may omit `variant`: it inherits the entity's `[mesh] variant`.
        assert_eq!(rig.lod[1].variant, None);
        // The last level is the procedural fallback and has no upper bound.
        assert_eq!(rig.lod[2].max_distance, None);
        assert_eq!(
            rig.lod[2].shape,
            Some(crate::entity_config::MeshShape::Sphere)
        );
    }

    #[test]
    fn a_sidecar_without_a_ladder_has_an_empty_chain() {
        let rig = ModelRig::from_toml("[base]\nscale = [1.0, 1.0, 1.0]\n").expect("parses");
        assert!(
            rig.lod.is_empty(),
            "no `[[lod]]` means no ladder — the entity renders its flat [mesh]"
        );
    }

    /// A generated level carries the parameters that produced it (issue #919).
    /// The renderer ignores them; the point is that they parse, so the sidecar
    /// can own its whole ladder without the engine rejecting the sidecar.
    #[test]
    fn a_level_carries_its_generation_parameters() {
        let toml = r##"
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod2.glb"
[lod.generate]
source = "assets/models/rock.glb"
ratio = 0.05
error = 0.1
texture_size = 256

[[lod]]
shape = "sphere"
"##;
        let rig = ModelRig::from_toml(toml).expect("generation params must parse");
        // The near level is authored, not generated.
        assert!(rig.lod[0].generate.is_none());
        let gen = rig.lod[1].generate.as_ref().expect("level 1 is generated");
        assert_eq!(gen.source.as_deref(), Some("assets/models/rock.glb"));
        assert_eq!(gen.ratio, Some(0.05));
        assert_eq!(gen.error, Some(0.1));
        assert_eq!(gen.texture_size, Some(256));
        // Absent means "no Blender pre-pass", not "voxel size zero".
        assert_eq!(gen.remesh_voxel_size, None);
        // Selection is untouched by the added block: still four bands.
        assert_eq!(rig.lod.len(), 3);
    }

    /// The schema is strict, so a mistyped key fails loudly instead of
    /// resolving to a rig that quietly lost a marker or a whole ladder.
    #[test]
    fn an_unknown_sidecar_field_is_rejected() {
        assert!(ModelRig::from_toml("[base]\noffest = [1.0, 0.0, 0.0]\n").is_err());
        assert!(ModelRig::from_toml("lods = []\n").is_err());
        assert!(ModelRig::from_toml("[[lod]]\nmax_distance = 50.0\nmodell = \"a.glb\"\n").is_err());
        // …including inside the build-time block, where a typo would otherwise
        // detach the level from the generator that maintains it (issue #919).
        assert!(ModelRig::from_toml(
            "[[lod]]\nmodel = \"a.glb\"\n[lod.generate]\nratio = 0.5\nerorr = 0.01\n"
        )
        .is_err());
        assert!(ModelRig::from_toml(
            "[markers.fore]\nposition = [0.0, 0.0, 0.0]\ndirection = [0.0, 0.0, -1.0]\nfacing = 1.0\n"
        )
        .is_err());
    }

    #[test]
    fn sidecar_variant_reads_the_variant_back_out_of_a_path() {
        assert_eq!(
            sidecar_variant("assets/models/asteroid_common_1.large.toml"),
            Some("large")
        );
        assert_eq!(
            sidecar_variant("assets/models/dynasty_destroyer.model.toml"),
            Some(DEFAULT_VARIANT)
        );
        // Round-trips with `sidecar_path` for both the default and a named variant.
        for variant in [None, Some("weathered")] {
            let path = sidecar_path("assets/models/ship.glb", variant);
            assert_eq!(
                sidecar_variant(&path),
                Some(variant.unwrap_or(DEFAULT_VARIANT))
            );
        }
        assert_eq!(sidecar_variant("assets/models/ship.glb"), None);
        assert_eq!(sidecar_variant("noextension"), None);
    }

    // ── Shipped-asset conformance ────────────────────────────────────────

    /// Every sidecar in `assets/models` parses under the strict schema.
    ///
    /// The engine degrades a malformed sidecar to an identity rig so a model
    /// always renders, which is exactly why nothing else would fail: a typo
    /// costs the ship its weapon markers, or the rock its whole LOD ladder,
    /// and the only symptom is that the game looks slightly wrong. This is the
    /// test that turns that into a red build. (Mirrors
    /// `all_shipped_entity_templates_parse_strictly` in `entity_config`.)
    #[test]
    fn every_shipped_sidecar_parses_strictly() {
        let mut checked = 0usize;
        let mut problems: Vec<String> = Vec::new();
        for entry in std::fs::read_dir("assets/models")
            .expect("assets/models must exist")
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let file = path.to_string_lossy().replace('\\', "/");
            let toml = std::fs::read_to_string(&path).expect("sidecar readable");
            checked += 1;
            if let Err(e) = ModelRig::from_toml(&toml) {
                problems.push(format!("{file}: {e}"));
            }
        }
        assert!(checked > 0, "assets/models should ship sidecars");
        assert!(
            problems.is_empty(),
            "every shipped model sidecar must parse strictly:\n{}",
            problems.join("\n")
        );
    }

    /// The migration itself (issue #914), asserted on a real shipped pair: the
    /// asteroid entity no longer carries a ladder, and the sidecar its `[mesh]`
    /// resolves to does — with every level's GLB present on disk.
    #[test]
    fn a_shipped_asteroid_reads_its_ladder_from_its_sidecar() {
        let cfg = crate::entity_includes::load_entity_config(
            "assets/entities/asteroid_common_1_large.toml",
        )
        .expect("asteroid template must parse");
        let mesh = cfg.mesh.as_ref().expect("mesh present");
        let path = sidecar_path(
            mesh.model.as_deref().expect("model path"),
            mesh.variant.as_deref(),
        );
        assert_eq!(path, "assets/models/asteroid_common_1.large.toml");

        let rig = ModelRig::from_toml(&std::fs::read_to_string(&path).expect("sidecar exists"))
            .expect("sidecar parses");
        assert_eq!(
            rig.lod.len(),
            4,
            "three GLB steps plus a procedural fallback"
        );

        // Ascending, exclusive bounds; only the final level is unbounded.
        let bounds: Vec<Option<f32>> = rig.lod.iter().map(|l| l.max_distance).collect();
        assert_eq!(
            bounds,
            vec![Some(50.0), Some(100.0), Some(150.0), None],
            "switch distances must survive the move verbatim"
        );

        // Switching behaviour is unchanged by the move: the same pure selector
        // the renderer calls, over the migrated chain, at the authored
        // distances. These are the numbers the pre-#914 `[[mesh.lod]]` blocks
        // produced, asserted rather than eyeballed.
        use crate::entity_config::select_lod;
        for (distance, expected) in [
            (0.0, 0),
            (49.9, 0),
            (50.0, 1),
            (99.9, 1),
            (100.0, 2),
            (149.9, 2),
            (150.0, 3),
            (10_000.0, 3),
        ] {
            assert_eq!(
                select_lod(&rig.lod, distance, None),
                expected,
                "distance {distance} must select level {expected}"
            );
        }
        // …and hysteresis still holds each band across its boundary.
        assert_eq!(select_lod(&rig.lod, 52.0, Some(0)), 0);
        assert_eq!(select_lod(&rig.lod, 48.0, Some(1)), 1);

        // Every GLB level names a file that exists, at the entity's variant.
        for level in rig.lod.iter().filter(|l| l.model.is_some()) {
            let model = level.model.as_deref().unwrap();
            assert!(
                std::path::Path::new(model).exists(),
                "LOD level model {model} must exist"
            );
            let level_sidecar =
                sidecar_path(model, level.variant.as_deref().or(mesh.variant.as_deref()));
            assert!(
                std::path::Path::new(&level_sidecar).exists(),
                "LOD level sidecar {level_sidecar} must exist"
            );
        }
        // Both decimated levels declare how they are regenerated (issue #919),
        // so deleting a `_lod*.glb` is recoverable from the sidecar alone. The
        // near level is the source and declares nothing.
        assert!(rig.lod[0].generate.is_none(), "the source is not generated");
        for level in &rig.lod[1..3] {
            let gen = level
                .generate
                .as_ref()
                .expect("every decimated level declares its parameters");
            assert_eq!(
                gen.source.as_deref(),
                Some("assets/models/asteroid_common_1.glb"),
                "both steps decimate the full model, not each other"
            );
            assert!(matches!(gen.ratio, Some(r) if r > 0.0 && r < 1.0));
            assert!(gen.error.is_some());
            assert!(gen.texture_size.is_some());
        }

        // The last level is the shared procedural sphere, inheriting the
        // entity's radius/colour rather than restating them.
        let last = rig.lod.last().unwrap();
        assert_eq!(last.shape, Some(crate::entity_config::MeshShape::Sphere));
        assert_eq!(last.radius, None);
        assert_eq!(last.colour, None);
    }

    /// Recursively collect every `.toml` file under `dir`, INCLUDING
    /// `assets/entities/fragments/`. Unlike a spawnable-template inventory,
    /// this check has no reason to stop at the top level: a fragment
    /// authoring `[[mesh.lod]]` would reintroduce the banned location into
    /// every hull that includes it, just as silently as a shipped hull
    /// authoring it directly.
    fn collect_toml_files_recursive(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_toml_files_recursive(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                out.push(path);
            }
        }
    }

    /// No entity template — nor any fragment it might compose in — may
    /// reintroduce the old location.
    #[test]
    fn no_shipped_entity_template_still_authors_mesh_lod() {
        let mut paths = Vec::new();
        collect_toml_files_recursive(std::path::Path::new("assets/entities"), &mut paths);
        assert!(
            !paths.is_empty(),
            "assets/entities must exist and contain templates"
        );

        let mut offenders: Vec<String> = Vec::new();
        for path in paths {
            let text = std::fs::read_to_string(&path).expect("template readable");
            let has_lod = toml::from_str::<toml::Value>(&text)
                .ok()
                .as_ref()
                .and_then(|v| v.get("mesh").and_then(|m| m.get("lod")))
                .is_some();
            if has_lod {
                offenders.push(path.to_string_lossy().replace('\\', "/"));
            }
        }
        assert!(
            offenders.is_empty(),
            "[[mesh.lod]] moved to the model sidecar (#914); still authored in:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn base_bevy_transform_xyz_euler_correct() {
        let toml = r##"
[base]
offset = [1.0, 2.0, 3.0]
rotation = [0.5, 0.0, 0.0]
scale = [2.0, 3.0, 4.0]
"##;
        let rig = ModelRig::from_toml(toml).unwrap();
        let t = rig.base_bevy_transform();
        assert!((t.translation - Vec3::new(1.0, 2.0, 3.0)).length() < EPS);
        assert!((t.scale - Vec3::new(2.0, 3.0, 4.0)).length() < EPS);

        let expected = Quat::from_euler(EulerRot::XYZ, 0.5, 0.0, 0.0);
        assert!(t.rotation.angle_between(expected).abs() < EPS);

        // A +0.5 rad rotation about X maps +Z toward -Y / +... sanity: rotate
        // forward (0,0,-1) and confirm it tilts in Y.
        let fwd = t.rotation * Vec3::new(0.0, 0.0, -1.0);
        assert!(fwd.y.abs() > 0.1, "X-rotation should tilt forward in Y");
    }
}
