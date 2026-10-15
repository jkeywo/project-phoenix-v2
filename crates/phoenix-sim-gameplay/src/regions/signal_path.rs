//! Deterministic effect-specific segment attenuation, independent of the
//! endpoint-membership modifiers and of player/peer networking.
use super::{
    field::{normalized_weight, FieldConfig, FieldFalloff, FieldPose},
    shape::RegionShape,
};
use glam::Vec3;

pub use phoenix_model::field::SignalMaterialConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalClass {
    Sensors,
    Comms,
}

pub struct SignalVolume<'a> {
    pub id: &'a str,
    pub shape: &'a RegionShape,
    pub pose: FieldPose,
    pub field: Option<&'a FieldConfig>,
    pub material: Option<&'a SignalMaterialConfig>,
}

/// Overlap adds optical depth in stable source-ID order. A positive-length
/// opaque intersection blocks; a tangent or zero-length link is clear. Step
/// integration is exact; other falloffs use eight midpoints on each clipped
/// interval, so a thin cloud cannot be missed by samples across a long link.
pub fn signal_transmission(
    mut volumes: Vec<SignalVolume<'_>>,
    start: Vec3,
    end: Vec3,
    class: SignalClass,
) -> f32 {
    if !start.is_finite() || !end.is_finite() {
        return 0.0;
    }
    let length = (end.as_dvec3() - start.as_dvec3()).length();
    if length == 0.0 {
        return 1.0;
    }
    volumes.sort_by(|a, b| a.id.cmp(b.id));
    let mut depth = 0.0_f64;
    for volume in volumes {
        if !volume.pose.scale.is_finite() || volume.pose.scale <= 0.0 {
            continue;
        }
        let a = (start.as_dvec3() - volume.pose.origin.as_dvec3()) / volume.pose.scale as f64;
        let b = (end.as_dvec3() - volume.pose.origin.as_dvec3()) / volume.pose.scale as f64;
        for (lo, hi) in clipped_intervals_f64(volume.shape, a, b) {
            if hi <= lo {
                continue;
            }
            let inside_length = length * (hi - lo);
            if let Some(material) = volume.material {
                if match class {
                    SignalClass::Sensors => material.sensor_opaque,
                    SignalClass::Comms => material.comms_opaque,
                } {
                    return 0.0;
                }
                depth += inside_length
                    * match class {
                        SignalClass::Sensors => material.sensor_absorption_per_unit,
                        SignalClass::Comms => material.comms_absorption_per_unit,
                    } as f64;
            }
            if let Some(field) = volume.field {
                let coefficient = match class {
                    SignalClass::Sensors => field.sensor_absorption_per_unit,
                    SignalClass::Comms => field.comms_absorption_per_unit,
                };
                let exposure = if field.falloff == FieldFalloff::Step {
                    volume.pose.strength as f64
                } else {
                    (0..8)
                        .map(|sample| {
                            let t = lo + (hi - lo) * (sample as f64 + 0.5) / 8.0;
                            normalized_weight(volume.shape, field.falloff, a.lerp(b, t))
                                * volume.pose.strength as f64
                        })
                        .sum::<f64>()
                        / 8.0
                };
                depth += inside_length * exposure * coefficient as f64;
            }
        }
    }
    (1.0 / (1.0 + depth)) as f32
}

fn circle_interval(
    a: glam::DVec3,
    d: glam::DVec3,
    radius: f64,
    planar: bool,
) -> Option<(f64, f64)> {
    let dot = |u: glam::DVec3, v: glam::DVec3| {
        if planar {
            u.x * v.x + u.z * v.z
        } else {
            u.dot(v)
        }
    };
    let length = dot(d, d).sqrt();
    if length == 0.0 {
        return (dot(a, a) < radius * radius).then_some((0.0, 1.0));
    }
    let direction = d / length;
    let projection = dot(a, direction);
    let closest = a - direction * projection;
    let half_chord_squared = radius * radius - dot(closest, closest);
    if half_chord_squared <= 0.0 {
        return None;
    }
    let center = -projection / length;
    let half = half_chord_squared.sqrt() / length;
    let lo = (center - half).max(0.0);
    let hi = (center + half).min(1.0);
    (hi > lo).then_some((lo, hi))
}

pub fn clipped_intervals(shape: &RegionShape, start: Vec3, end: Vec3) -> Vec<(f64, f64)> {
    clipped_intervals_f64(shape, start.as_dvec3(), end.as_dvec3())
}

fn clipped_intervals_f64(shape: &RegionShape, a: glam::DVec3, end: glam::DVec3) -> Vec<(f64, f64)> {
    let d = end - a;
    match shape {
        RegionShape::Sphere { radius } => circle_interval(a, d, *radius as f64, false)
            .into_iter()
            .collect(),
        RegionShape::Torus {
            inner_radius,
            outer_radius,
        } => {
            let Some((lo, hi)) = circle_interval(a, d, *outer_radius as f64, true) else {
                return vec![];
            };
            if *inner_radius == 0.0 {
                return vec![(lo, hi)];
            }
            let Some((inner_lo, inner_hi)) = circle_interval(a, d, *inner_radius as f64, true)
            else {
                return vec![(lo, hi)];
            };
            [(lo, inner_lo.min(hi)), (inner_hi.max(lo), hi)]
                .into_iter()
                .filter(|(a, b)| b > a)
                .collect()
        }
        RegionShape::Box { half_extents, yaw } => {
            let (sin, cos) = crate::simmath::sin_cos(*yaw);
            let rotate = |v: glam::DVec3| {
                glam::DVec3::new(
                    v.x * cos as f64 + v.z * sin as f64,
                    v.y,
                    -v.x * sin as f64 + v.z * cos as f64,
                )
            };
            let a = rotate(a);
            let d = rotate(d);
            let mut lo = 0.0_f64;
            let mut hi = 1.0_f64;
            for axis in 0..3 {
                let extent = half_extents[axis] as f64;
                if d[axis] == 0.0 {
                    if a[axis].abs() >= extent {
                        return vec![];
                    }
                } else {
                    let t1 = (-extent - a[axis]) / d[axis];
                    let t2 = (extent - a[axis]) / d[axis];
                    lo = lo.max(t1.min(t2));
                    hi = hi.min(t1.max(t2));
                    if hi <= lo {
                        return vec![];
                    }
                }
            }
            vec![(lo, hi)]
        }
    }
}

#[cfg(test)]
#[path = "signal_path_tests.rs"]
mod tests;
