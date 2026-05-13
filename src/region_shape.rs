use glam::Vec3;

#[derive(Clone, Debug, PartialEq)]
pub enum RegionShape {
    Sphere { radius: f32 },
    Box { half_extents: Vec3 },
    Cylinder { radius: f32, half_height: f32 },
}

impl RegionShape {
    pub fn contains(&self, point: Vec3, origin: Vec3) -> bool {
        let delta = point - origin;
        match self {
            RegionShape::Sphere { radius } => {
                delta.length_squared() <= radius * radius
            }
            RegionShape::Box { half_extents } => {
                delta.x.abs() <= half_extents.x
                    && delta.y.abs() <= half_extents.y
                    && delta.z.abs() <= half_extents.z
            }
            RegionShape::Cylinder { radius, half_height } => {
                let horiz_sq = delta.x * delta.x + delta.z * delta.z;
                horiz_sq <= radius * radius && delta.y.abs() <= *half_height
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let b = RegionShape::Box { half_extents: Vec3::new(5.0, 3.0, 4.0) };
        assert!(b.contains(Vec3::ZERO, Vec3::ZERO));
    }

    #[test]
    fn box_contains_point_at_edge() {
        let b = RegionShape::Box { half_extents: Vec3::new(5.0, 3.0, 4.0) };
        assert!(b.contains(Vec3::new(5.0, 0.0, 0.0), Vec3::ZERO));
        assert!(b.contains(Vec3::new(0.0, 3.0, 0.0), Vec3::ZERO));
        assert!(b.contains(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO));
    }

    #[test]
    fn box_rejects_point_just_outside() {
        let b = RegionShape::Box { half_extents: Vec3::new(5.0, 3.0, 4.0) };
        assert!(!b.contains(Vec3::new(5.001, 0.0, 0.0), Vec3::ZERO));
        assert!(!b.contains(Vec3::new(0.0, 3.001, 0.0), Vec3::ZERO));
        assert!(!b.contains(Vec3::new(0.0, 0.0, 4.001), Vec3::ZERO));
    }

    #[test]
    fn box_rejects_point_far_outside() {
        let b = RegionShape::Box { half_extents: Vec3::new(5.0, 3.0, 4.0) };
        assert!(!b.contains(Vec3::new(100.0, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn box_works_with_non_zero_origin() {
        let b = RegionShape::Box { half_extents: Vec3::new(5.0, 5.0, 5.0) };
        let origin = Vec3::new(10.0, 10.0, 10.0);
        assert!(b.contains(Vec3::new(12.0, 10.0, 10.0), origin));
        assert!(!b.contains(Vec3::new(16.0, 10.0, 10.0), origin));
    }

    // ── Cylinder containment ───────────────────────────────────────────────

    #[test]
    fn cylinder_contains_point_at_centre() {
        let c = RegionShape::Cylinder { radius: 5.0, half_height: 3.0 };
        assert!(c.contains(Vec3::ZERO, Vec3::ZERO));
    }

    #[test]
    fn cylinder_contains_point_at_radial_edge() {
        let c = RegionShape::Cylinder { radius: 5.0, half_height: 3.0 };
        assert!(c.contains(Vec3::new(5.0, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn cylinder_contains_point_at_top_edge() {
        let c = RegionShape::Cylinder { radius: 5.0, half_height: 3.0 };
        assert!(c.contains(Vec3::new(0.0, 3.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn cylinder_rejects_point_above_top() {
        let c = RegionShape::Cylinder { radius: 5.0, half_height: 3.0 };
        assert!(!c.contains(Vec3::new(0.0, 3.001, 0.0), Vec3::ZERO));
    }

    #[test]
    fn cylinder_rejects_point_outside_radius() {
        let c = RegionShape::Cylinder { radius: 5.0, half_height: 3.0 };
        assert!(!c.contains(Vec3::new(5.001, 0.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn cylinder_rejects_point_outside_radius_and_within_height() {
        let c = RegionShape::Cylinder { radius: 5.0, half_height: 3.0 };
        assert!(!c.contains(Vec3::new(5.001, 1.0, 0.0), Vec3::ZERO));
    }

    #[test]
    fn cylinder_works_with_non_zero_origin() {
        let c = RegionShape::Cylinder { radius: 5.0, half_height: 3.0 };
        let origin = Vec3::new(10.0, 5.0, 10.0);
        assert!(c.contains(Vec3::new(12.0, 5.0, 10.0), origin));
        assert!(!c.contains(Vec3::new(16.0, 5.0, 10.0), origin));
        assert!(!c.contains(Vec3::new(10.0, 9.0, 10.0), origin));
    }
}
