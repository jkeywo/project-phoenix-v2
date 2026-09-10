//! WebGL2 flare visibility against visible opaque mesh bounds; no depth readback.
#![allow(clippy::disallowed_methods)]
#[cfg(target_arch = "wasm32")]
use crate::entities::planet::PlanetSurfaceMaterial;
use bevy::prelude::*;
#[cfg(target_arch = "wasm32")]
use bevy::{camera::primitives::Aabb, ecs::system::SystemParam};

#[cfg(target_arch = "wasm32")]
#[derive(SystemParam)]
pub(super) struct WebOcclusion<'w, 's> {
    time: Res<'w, Time>,
    materials: Res<'w, Assets<StandardMaterial>>,
    meshes: Query<
        'w,
        's,
        (
            &'static GlobalTransform,
            &'static Aabb,
            &'static InheritedVisibility,
            Option<&'static MeshMaterial3d<StandardMaterial>>,
            Option<&'static MeshMaterial3d<PlanetSurfaceMaterial>>,
        ),
        With<Mesh3d>,
    >,
}
#[cfg(target_arch = "wasm32")]
impl WebOcclusion<'_, '_> {
    pub fn visibility(&self, eye: Vec3, star: Vec3, radius: f32, previous: f32) -> f32 {
        // Material filtering excludes motes, halos, beams and transparent effects.
        // Inherited visibility also excludes the player's hidden first-person hull.
        let proxies: Vec<_> = self
            .meshes
            .iter()
            .filter_map(|(t, a, visible, material, planet)| {
                let opaque = material
                    .and_then(|m| self.materials.get(&m.0))
                    .is_some_and(|m| {
                        matches!(
                            m.alpha_mode,
                            AlphaMode::Opaque | AlphaMode::Mask(_) | AlphaMode::AlphaToCoverage
                        )
                    });
                if !visible.get()
                    || (!opaque && planet.is_none())
                    || t.affine().matrix3.determinant().abs() < 1e-12
                {
                    return None;
                }
                Some((
                    t.affine().inverse(),
                    Vec3::from(a.center),
                    Vec3::from(a.half_extents),
                    planet.is_some(),
                ))
            })
            .collect();
        let toward_eye = (eye - star).normalize_or_zero();
        // Choose a stable non-parallel basis even directly above a star.
        let axis = if toward_eye.y.abs() > 0.99 {
            Vec3::X
        } else {
            Vec3::Y
        };
        let right = axis.cross(toward_eye).normalize_or_zero();
        let up = toward_eye.cross(right);
        const SAMPLES: u32 = 24;
        let mut visible = 0.0;
        for i in 0..SAMPLES {
            let theta = i as f32 * 2.399963;
            let r = ((i as f32 + 0.5) / SAMPLES as f32).sqrt() * 0.88;
            let target = star
                + radius
                    * (right * theta.cos() * r
                        + up * theta.sin() * r
                        + toward_eye * (1.0 - r * r).sqrt());
            let blocked = proxies.iter().any(|(inv, center, half, sphere)| {
                let start = inv.transform_point3(eye);
                let end = inv.transform_point3(target);
                if *sphere {
                    let extent = half.max(Vec3::splat(1e-6));
                    segment_sphere((start - *center) / extent, (end - *center) / extent)
                } else {
                    segment_box(start, end, *center, *half)
                }
            });
            if !blocked {
                visible += 1.0 / SAMPLES as f32;
            }
        }
        previous + (visible - previous) * (1.0 - (-self.time.delta_secs() * 12.0).exp())
    }
}

/// Segment tests use local coordinates, so non-uniformly scaled proxies work too.
fn segment_box(start: Vec3, end: Vec3, center: Vec3, half: Vec3) -> bool {
    let origin = start - center;
    let delta = end - start;
    let mut near: f32 = 0.0;
    let mut far: f32 = 1.0;
    for axis in 0..3 {
        if delta[axis].abs() < 1e-6 {
            if origin[axis].abs() > half[axis] {
                return false;
            }
        } else {
            let a = (-half[axis] - origin[axis]) / delta[axis];
            let b = (half[axis] - origin[axis]) / delta[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return false;
            }
        }
    }
    far > 0.0 && near < 1.0
}
fn segment_sphere(start: Vec3, end: Vec3) -> bool {
    let delta = end - start;
    let t = (-start.dot(delta) / delta.length_squared().max(1e-12)).clamp(0.0, 1.0);
    (start + delta * t).length_squared() < 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segment_does_not_see_occluders_behind_the_source() {
        let eye = Vec3::new(0.0, 0.0, 5.0);
        assert!(segment_box(eye, -eye, Vec3::ZERO, Vec3::ONE));
        assert!(segment_sphere(eye, -eye));
        let near_source = Vec3::new(0.0, 0.0, 2.0);
        assert!(!segment_box(eye, near_source, Vec3::ZERO, Vec3::ONE));
        assert!(!segment_sphere(eye, near_source));
        let offset = Vec3::X * 2.0;
        assert!(!segment_box(
            eye + offset,
            -eye + offset,
            Vec3::ZERO,
            Vec3::ONE
        ));
        assert!(!segment_sphere(eye + offset, -eye + offset));
    }
}
