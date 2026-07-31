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
mod tests {
    use super::*;
    use glam::Vec3;

    // ── Sphere containment ─────────────────────────────────────────────────

    #[test]
    fn sphere_contains_point_at_centre() {
        let sphere = RegionShape::Sphere { radius: 10.0 };
        assert!(sphere.contains(Vec3::ZERO, Vec3::ZERO));
    }

    #[test]
    fn sphere_contains_point_at_edge() {
        let sphere = RegionShape::Sphere { radius: 10.0 };
        assert!(sphere.contains(Vec3::new(10.0, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn sphere_rejects_point_just_outside() {
        let sphere = RegionShape::Sphere { radius: 10.0 };
        assert!(!sphere.contains(Vec3::new(10.001, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn sphere_contains_point_at_diagonal_edge() {
        let sphere = RegionShape::Sphere { radius: 10.0 };
        let d = 10.0_f32 / std::f32::consts::SQRT_2;
        assert!(sphere.contains(Vec3::new(d, d, 0.0), Vec3::ZERO));
    }

    #[test]
    fn sphere_rejects_point_beyond_diagonal() {
        let sphere = RegionShape::Sphere { radius: 10.0 };
        assert!(!sphere.contains(Vec3::new(10.0, 10.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn sphere_works_with_non_zero_origin() {
        let sphere = RegionShape::Sphere { radius: 5.0 };
        let origin = Vec3::new(10.0, 20.0, 30.0);
        assert!(sphere.contains(Vec3::new(12.0, 20.0, 30.0), origin)); // 2 units away
        assert!(!sphere.contains(Vec3::new(16.0, 20.0, 30.0), origin)); // 6 units away
    }

    // ── Box containment ────────────────────────────────────────────────────

    #[test]
    fn box_contains_point_at_centre() {
        let b = RegionShape::Box {
            half_extents: [5.0, 3.0, 4.0],
            yaw: 0.0,
        };
        assert!(b.contains(glam::Vec3::ZERO, glam::Vec3::ZERO));
    }

    #[test]
    fn box_contains_point_at_edge() {
        let b = RegionShape::Box {
            half_extents: [5.0, 3.0, 4.0],
            yaw: 0.0,
        };
        assert!(b.contains(glam::Vec3::new(5.0, 0.0, 0.0), glam::Vec3::ZERO));
        assert!(b.contains(glam::Vec3::new(0.0, 3.0, 0.0), glam::Vec3::ZERO));
        assert!(b.contains(glam::Vec3::new(0.0, 0.0, 4.0), glam::Vec3::ZERO));
    }

    #[test]
    fn box_rejects_point_just_outside() {
        let b = RegionShape::Box {
            half_extents: [5.0, 3.0, 4.0],
            yaw: 0.0,
        };
        assert!(!b.contains(glam::Vec3::new(5.001, 0.0, 0.0), glam::Vec3::ZERO));
        assert!(!b.contains(glam::Vec3::new(0.0, 3.001, 0.0), glam::Vec3::ZERO));
        assert!(!b.contains(glam::Vec3::new(0.0, 0.0, 4.001), glam::Vec3::ZERO));
    }

    #[test]
    fn box_rejects_point_far_outside() {
        let b = RegionShape::Box {
            half_extents: [5.0, 3.0, 4.0],
            yaw: 0.0,
        };
        assert!(!b.contains(glam::Vec3::new(100.0, 0.0, 0.0), glam::Vec3::ZERO));
    }

    #[test]
    fn box_works_with_non_zero_origin() {
        let b = RegionShape::Box {
            half_extents: [5.0, 5.0, 5.0],
            yaw: 0.0,
        };
        let origin = glam::Vec3::new(10.0, 10.0, 10.0);
        assert!(b.contains(glam::Vec3::new(12.0, 10.0, 10.0), origin));
        assert!(!b.contains(glam::Vec3::new(16.0, 10.0, 10.0), origin));
    }

    #[test]
    fn box_with_yaw_rejects_point_outside_rotated_frame() {
        // Box aligned at 45° yaw, half_extents [5, 5, 1].
        // Without rotation a point at (3, 0, 3) would be inside (3 < 5, 3 > 1 is false — wait).
        // Use a thin box: half_extents [10, 5, 1] rotated 45°.
        // In the rotated frame the "thin" axis is z_local.
        // A point at (2, 0, 0) world: local_x = 2*cos45 + 0*sin45 = sqrt(2) ≈ 1.41
        //                             local_z = -2*sin45 + 0*cos45 = -sqrt(2) ≈ -1.41
        // |local_z| = 1.41 > half_extents[2] = 1.0 → outside (correct rejection)
        let yaw = std::f32::consts::FRAC_PI_4; // 45 degrees
        let b = RegionShape::Box {
            half_extents: [10.0, 5.0, 1.0],
            yaw,
        };
        // Point (2, 0, 0): world-delta = (2,0,0)
        // local_x = 2*cos(45) ≈ 1.414, local_z = -2*sin(45) ≈ -1.414
        // |local_z| ≈ 1.414 > 1.0 → outside
        assert!(
            !b.contains(Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO),
            "point should be outside thin rotated box"
        );

        // Point (0.5, 0, 0.5): local_x = 0.5*cos45 + 0.5*sin45 = sqrt(2)*0.5 ≈ 0.707
        //                       local_z = -0.5*sin45 + 0.5*cos45 = 0 → inside
        assert!(
            b.contains(Vec3::new(0.5, 0.0, 0.5), Vec3::ZERO),
            "point on axis of rotated box should be inside"
        );
    }

    // ── Torus containment ──────────────────────────────────────────────────

    #[test]
    fn torus_contains_point_in_donut_ring() {
        let t = RegionShape::Torus {
            inner_radius: 5.0,
            outer_radius: 10.0,
        };
        // XZ distance = 7.5, between 5 and 10 → inside
        assert!(t.contains(Vec3::new(7.5, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn torus_rejects_point_in_hole() {
        let t = RegionShape::Torus {
            inner_radius: 5.0,
            outer_radius: 10.0,
        };
        // XZ distance = 2, less than inner_radius 5 → inside hole → false
        assert!(!t.contains(Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn torus_rejects_point_outside() {
        let t = RegionShape::Torus {
            inner_radius: 5.0,
            outer_radius: 10.0,
        };
        // XZ distance = 15, greater than outer_radius 10 → outside → false
        assert!(!t.contains(Vec3::new(15.0, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn torus_contains_point_at_inner_edge() {
        let t = RegionShape::Torus {
            inner_radius: 5.0,
            outer_radius: 10.0,
        };
        assert!(t.contains(Vec3::new(5.0, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn torus_contains_point_at_outer_edge() {
        let t = RegionShape::Torus {
            inner_radius: 5.0,
            outer_radius: 10.0,
        };
        assert!(t.contains(Vec3::new(10.0, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn torus_works_with_non_zero_origin() {
        let t = RegionShape::Torus {
            inner_radius: 5.0,
            outer_radius: 10.0,
        };
        let origin = Vec3::new(100.0, 0.0, 100.0);
        // Point 7 units away in X from origin → inside donut
        assert!(t.contains(Vec3::new(107.0, 0.0, 100.0), origin));
        // Point 2 units away → in hole
        assert!(!t.contains(Vec3::new(102.0, 0.0, 100.0), origin));
    }
}
