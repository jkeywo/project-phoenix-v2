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
//! ```
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
pub struct Extents {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub size: [f32; 3],
}

/// A single named mount point in post-base-rig (model-local, base-applied)
/// space. `direction` is a unit vector with forward basis `(0,0,-1)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub position: [f32; 3],
    pub direction: [f32; 3],
}

/// A single anonymous target point in post-base-rig space.
///
/// Phaser PFX can resolve one of these points on the target model so beams hit
/// plausible hull positions instead of always converging on the entity centre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetPoint {
    pub position: [f32; 3],
}

/// A parsed model-rig sidecar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
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
