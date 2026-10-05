//! Authored primary-rig geometry and visual LOD descriptors.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The `[base]` rig: corrects a raw GLB into game space. All fields default so
/// a sparse or empty sidecar parses to an identity rig.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BaseTransform {
    /// Non-uniform translation applied to the model, in model-local units.
    pub offset: [f32; 3],
    /// XYZ-order euler rotation in radians.
    pub rotation: [f32; 3],
    /// Non-uniform scale applied to the model.
    pub scale: [f32; 3],
}

impl Default for BaseTransform {
    fn default() -> Self {
        BaseTransform {
            offset: [0.0; 3],
            rotation: [0.0; 3],
            scale: [1.0; 3],
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
    pub lod: Vec<crate::entity::LodLevel>,
}

impl ModelRig {
    pub fn marker(&self, name: &str) -> Option<&Marker> {
        self.markers.get(name)
    }
}
