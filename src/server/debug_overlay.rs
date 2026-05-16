use bevy::prelude::*;

use crate::entity_spawner::RegionShapeSection;
use crate::region_shape::RegionShape;

/// Resource indicating whether debug region wireframes are enabled.
#[derive(Resource)]
pub struct DebugRegionsEnabled(pub bool);

/// Server-only plugin that draws region shape wireframes when enabled.
///
/// The `enabled` field is typically set from the `?debug_regions=1` URL parameter
/// on WASM (via `bridge.rs`), or directly in tests.
pub struct DebugOverlayPlugin {
    pub enabled: bool,
}

impl Plugin for DebugOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DebugRegionsEnabled(self.enabled));
        app.add_systems(
            Update,
            draw_region_wireframes.run_if(|r: Res<DebugRegionsEnabled>| r.0),
        );
    }
}

/// Draws wireframe outlines for every region entity with a shape component.
fn draw_region_wireframes(
    regions: Query<(&Transform, &RegionShapeSection)>,
    mut gizmos: Gizmos,
) {
    for (transform, shape) in regions.iter() {
        let origin = transform.translation;
        match &shape.0 {
            RegionShape::Sphere { radius } => {
                draw_sphere_wireframe(&mut gizmos, origin, *radius);
            }
            RegionShape::Box { half_extents } => {
                draw_box_wireframe(&mut gizmos, origin, *half_extents);
            }
            RegionShape::Cylinder { radius, half_height } => {
                draw_cylinder_wireframe(&mut gizmos, origin, *radius, *half_height);
            }
        }
    }
}

fn draw_sphere_wireframe(gizmos: &mut Gizmos, origin: Vec3, radius: f32) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    gizmos.circle(
        Isometry3d::new(origin, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        radius,
        color,
    );
    gizmos.circle(
        Isometry3d::new(origin, Quat::IDENTITY),
        radius,
        color,
    );
    gizmos.circle(
        Isometry3d::new(origin, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        radius,
        color,
    );
}

fn draw_box_wireframe(gizmos: &mut Gizmos, origin: Vec3, half_extents: [f32; 3]) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    let [hx, hy, hz] = half_extents;
    let corners = [
        Vec3::new(-hx, -hy, -hz),
        Vec3::new(hx, -hy, -hz),
        Vec3::new(hx, -hy, hz),
        Vec3::new(-hx, -hy, hz),
        Vec3::new(-hx, hy, -hz),
        Vec3::new(hx, hy, -hz),
        Vec3::new(hx, hy, hz),
        Vec3::new(-hx, hy, hz),
    ]
    .map(|c| origin + c);
    let edges: [(usize, usize); 12] = [
        (0, 1), (1, 2), (2, 3), (3, 0),
        (4, 5), (5, 6), (6, 7), (7, 4),
        (0, 4), (1, 5), (2, 6), (3, 7),
    ];
    for (i, j) in edges {
        gizmos.line(corners[i], corners[j], color);
    }
}

fn draw_cylinder_wireframe(gizmos: &mut Gizmos, origin: Vec3, radius: f32, half_height: f32) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    let top = origin + Vec3::Y * half_height;
    let bottom = origin - Vec3::Y * half_height;
    gizmos.circle(
        Isometry3d::new(top, Quat::IDENTITY),
        radius,
        color,
    );
    gizmos.circle(
        Isometry3d::new(bottom, Quat::IDENTITY),
        radius,
        color,
    );
    for angle in [0.0, std::f32::consts::FRAC_PI_2, std::f32::consts::PI, 3.0 * std::f32::consts::FRAC_PI_2] {
        let offset = Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius);
        gizmos.line(bottom + offset, top + offset, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_regions_disabled_by_default() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(!enabled.0, "default should be disabled");
    }

    #[test]
    fn debug_regions_enabled_when_flag_set() {
        let plugin = DebugOverlayPlugin { enabled: true };
        let mut app = App::new();
        plugin.build(&mut app);
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(enabled.0, "should be enabled when flag is set");
    }

    /// Toggling the resource from false → true should flip DebugRegionsEnabled.
    #[test]
    fn toggle_debug_regions_false_to_true() {
        let plugin = DebugOverlayPlugin { enabled: false };
        let mut app = App::new();
        plugin.build(&mut app);
        // Simulate what drain_debug_toggles does: flip the resource.
        app.world_mut().resource_mut::<DebugRegionsEnabled>().0 = true;
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(enabled.0, "resource should be true after toggle");
    }

    /// Toggling the resource from true → false should flip DebugRegionsEnabled.
    #[test]
    fn toggle_debug_regions_true_to_false() {
        let plugin = DebugOverlayPlugin { enabled: true };
        let mut app = App::new();
        plugin.build(&mut app);
        app.world_mut().resource_mut::<DebugRegionsEnabled>().0 = false;
        let enabled = app.world().resource::<DebugRegionsEnabled>();
        assert!(!enabled.0, "resource should be false after toggle");
    }
}
