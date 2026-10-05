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

use std::collections::HashMap;

use bevy::math::{EulerRot, Quat, Vec3};
use bevy::prelude::{Component, Transform};

pub use phoenix_model::rig::*;
pub trait ModelRigTransform {
    fn base_bevy_transform(&self) -> Transform;
}
impl ModelRigTransform for ModelRig {
    fn base_bevy_transform(&self) -> Transform {
        self.base.bevy_transform()
    }
}
pub trait BaseTransformExt {
    fn bevy_transform(&self) -> Transform;
}
impl BaseTransformExt for BaseTransform {
    /// Build the Bevy `Transform` for this base rig.
    fn bevy_transform(&self) -> bevy::prelude::Transform {
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

pub use phoenix_content::rig::{parse_model_rig, sidecar_path, sidecar_variant, DEFAULT_VARIANT};

#[cfg(test)]
#[path = "model_rig_tests.rs"]
mod tests;
