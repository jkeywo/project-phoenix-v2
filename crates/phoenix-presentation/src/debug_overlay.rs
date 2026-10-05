use crate::entities::spawner::RegionShapeSection;
use crate::regions::shape::RegionShape;
use bevy::prelude::*;
pub use phoenix_simulation::debug_overlay::*;

/// Server-only plugin that draws region shape wireframes when enabled.
///
/// The `enabled` field supports direct native/test setup. The WASM host starts
/// false and applies `?debug_regions=1` through the catalogue after startup.
pub struct DebugOverlayPlugin {
    pub enabled: bool,
}

impl Plugin for DebugOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DebugRegionsEnabled(self.enabled));
        app.init_resource::<DebugOverlayEnabled>();
        app.init_resource::<SimulationPaused>();
        app.init_resource::<DebugDamageEnabled>();
        app.init_resource::<DamageLog>();
        app.init_resource::<DebugEntitiesEnabled>();
        app.init_resource::<DebugEntityInspectorEnabled>();
        if should_install_region_wireframes() {
            app.add_systems(
                Update,
                draw_region_wireframes.run_if(|r: Res<DebugRegionsEnabled>| r.0),
            );
        }
        // The modifier / damage / entity-behavior / entity-inspector OUTPUTS no
        // longer emit here. As of issue #1150 (PRD #1144) each is a structured
        // serde-JSON payload published on the debug pipeline by `crate::debug`'s
        // per-surface `publish_*` system (added by `DebugPlugin`), retiring the
        // pre-formatted-text path that used to live in this plugin — "one debug
        // system at the end, not two". This plugin keeps only what stayed text or
        // gizmo: the region wireframes above and the enabled-flag resources it
        // owns; the phone settings route (`report_debug_state` and the drains)
        // is registered in `server_app`.
    }
}

fn should_install_region_wireframes() -> bool {
    !is_playwright_automation()
}

/// Draws wireframe outlines for every region entity with a shape component.
fn draw_region_wireframes(regions: Query<(&Transform, &RegionShapeSection)>, mut gizmos: Gizmos) {
    for (transform, shape) in regions.iter() {
        let origin = transform.translation - Vec3::Y * 10.0;
        match &shape.0 {
            RegionShape::Sphere { radius } => {
                draw_sphere_wireframe(&mut gizmos, origin, *radius);
            }
            RegionShape::Box { half_extents, .. } => {
                draw_box_wireframe(&mut gizmos, origin, *half_extents);
            }
            RegionShape::Torus {
                inner_radius,
                outer_radius,
            } => {
                draw_torus_wireframe(&mut gizmos, origin, *inner_radius, *outer_radius);
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
    gizmos.circle(Isometry3d::new(origin, Quat::IDENTITY), radius, color);
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
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (i, j) in edges {
        gizmos.line(corners[i], corners[j], color);
    }
}

fn draw_torus_wireframe(gizmos: &mut Gizmos, origin: Vec3, inner_radius: f32, outer_radius: f32) {
    let color = Color::srgba(0.0, 1.0, 0.3, 0.6);
    // Draw two horizontal circles representing the inner and outer edges of the torus
    gizmos.circle(Isometry3d::new(origin, Quat::IDENTITY), inner_radius, color);
    gizmos.circle(Isometry3d::new(origin, Quat::IDENTITY), outer_radius, color);
}

#[cfg(test)]
#[path = "debug_overlay_tests.rs"]
mod tests;
