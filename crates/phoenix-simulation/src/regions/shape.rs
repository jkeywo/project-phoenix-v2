use crate::simmath;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RegionShape {
    Sphere {
        radius: f32,
    },
    Box {
        half_extents: [f32; 3],
        #[serde(default)]
        yaw: f32,
    },
    Torus {
        inner_radius: f32,
        outer_radius: f32,
    },
}

impl RegionShape {
    pub fn contains(&self, point: glam::Vec3, origin: glam::Vec3) -> bool {
        let delta = point - origin;
        match self {
            RegionShape::Sphere { radius } => delta.length_squared() <= radius * radius,
            RegionShape::Box { half_extents, yaw } => {
                // Rotate delta by -yaw around Y axis to get into the box's local frame
                let (sin_y, cos_y) = simmath::sin_cos(*yaw);
                let local_x = delta.x * cos_y + delta.z * sin_y;
                let local_z = -delta.x * sin_y + delta.z * cos_y;
                local_x.abs() <= half_extents[0]
                    && delta.y.abs() <= half_extents[1]
                    && local_z.abs() <= half_extents[2]
            }
            RegionShape::Torus {
                inner_radius,
                outer_radius,
            } => {
                let xz_dist = (delta.x * delta.x + delta.z * delta.z).sqrt();
                xz_dist >= *inner_radius && xz_dist <= *outer_radius
            }
        }
    }
}

#[cfg(test)]
#[path = "shape_tests.rs"]
mod tests;
