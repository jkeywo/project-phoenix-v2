//! Native gameplay presentation selected in the lighting lab.
// Render-only trigonometry never feeds authoritative simulation state.
#![allow(clippy::disallowed_methods)]
mod flare;
use crate::{
    core::messages::{GamePhase, ViewMode},
    entities::{billboard::BillboardPose, spawner::StarSection, star::StarHalo},
    render_setup::GameCamera,
    server_app::LocalShip,
    ship::state::{ShipPhysics, ShipViewMode},
    world::{config::WorldConfig, native_render_config::NativeRenderConfig},
};
use bevy::{
    core_pipeline::prepass::DepthPrepass,
    light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, NotShadowCaster},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    transform::TransformSystems,
};

pub struct NativeVisualPlugin;
impl Plugin for NativeVisualPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            flare::FlarePlugin,
            MaterialPlugin::<MoteMaterial>::default(),
        ))
        .init_resource::<MotePool>()
        .add_systems(Update, prepare_cameras)
        .add_systems(PostUpdate, update_motes.before(TransformSystems::Propagate))
        .add_systems(PostUpdate, update_stars.after(TransformSystems::Propagate));
    }
}
fn prepare_cameras(
    mut commands: Commands,
    cameras: Query<Entity, (With<GameCamera>, Without<flare::FlareSettings>)>,
) {
    for entity in &cameras {
        commands
            .entity(entity)
            .insert((DepthPrepass, flare::FlareSettings::default()));
    }
}
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct MoteMaterial {
    #[uniform(0)]
    tint_brightness: Vec4,
    #[uniform(0)]
    opacity_masks: Vec4,
    #[texture(1)]
    #[sampler(2)]
    texture: Handle<Image>,
}
impl Material for MoteMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/dust_mote.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}
#[derive(Component)]
struct NativeMote(usize);
#[derive(Resource, Default)]
struct MotePool {
    config: Option<NativeRenderConfig>,
}
fn visible_view(mode: &ShipViewMode) -> bool {
    matches!(mode.view_mode, ViewMode::Camera(_) | ViewMode::Cinematic)
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_motes(
    mut commands: Commands,
    world: Option<Res<WorldConfig>>,
    defaults: Local<NativeRenderConfig>,
    phase: Res<State<GamePhase>>,
    mut pool: ResMut<MotePool>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MoteMaterial>>,
    camera: Query<&Transform, (With<GameCamera>, Without<NativeMote>)>,
    ship: Query<(&ShipPhysics, &ShipViewMode), With<LocalShip>>,
    mut motes: Query<(Entity, &NativeMote, &mut Transform, &mut Visibility)>,
) {
    let cfg = world
        .as_ref()
        .and_then(|w| w.render.as_ref())
        .map(|r| &r.native)
        .unwrap_or(&defaults);
    let enabled = *phase.get() == GamePhase::InProgress
        && cfg.motes
        && world
            .as_ref()
            .and_then(|w| w.dust.as_ref())
            .and_then(|d| d.enabled)
            .unwrap_or(true);
    if !enabled || pool.config.as_ref() != Some(cfg) {
        for (entity, ..) in &motes {
            commands.entity(entity).despawn();
        }
        pool.config = None;
        if !enabled {
            return;
        }
    }
    let Ok(camera) = camera.single() else {
        return;
    };
    let ship = ship.single().ok();
    let visible = ship.is_some_and(|(_, mode)| visible_view(mode));
    if pool.config.is_none() {
        let mesh = meshes.add(Rectangle::default());
        let mats: Vec<_> = (0..3)
            .map(|i| {
                materials.add(MoteMaterial {
                    tint_brightness: Vec3::from(cfg.mote_tint).extend(cfg.mote_brightness[i]),
                    opacity_masks: Vec4::new(cfg.mote_opacity, 0.12, 0.38, 0.15),
                    texture: assets.load(cfg.mote_textures[i].clone()),
                })
            })
            .collect();
        for i in 0..cfg.mote_count.min(10000) {
            commands.spawn((
                NativeMote(i),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mats[i % 3].clone()),
                Transform::default(),
                Visibility::Hidden,
                NotShadowCaster,
            ));
        }
        pool.config = Some(cfg.clone());
        return;
    }
    let velocity = ship
        .map(|(p, _)| {
            let (s, c) = p.yaw.sin_cos();
            Vec3::new(
                s * p.forward_speed + c * p.lateral_speed,
                p.vertical_speed,
                -c * p.forward_speed + s * p.lateral_speed,
            )
        })
        .unwrap_or_default();
    let half = Vec3::new(
        cfg.mote_half_width.max(0.1),
        cfg.mote_half_width.max(0.1),
        cfg.mote_depth.max(0.2) * 0.5,
    );
    for (_, mote, mut transform, mut visibility) in &mut motes {
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if !visible {
            continue;
        }
        let n = mote.0 as f32;
        let anchor = Vec3::new(
            (n * 0.7548777).fract(),
            (n * 0.5698403).fract(),
            (n * 0.438289).fract(),
        ) * half
            * 2.0;
        let rel = anchor - camera.translation;
        transform.translation = camera.translation
            + Vec3::new(
                (rel.x + half.x).rem_euclid(half.x * 2.0) - half.x,
                (rel.y + half.y).rem_euclid(half.y * 2.0) - half.y,
                (rel.z + half.z).rem_euclid(half.z * 2.0) - half.z,
            );
        let local = camera.rotation.inverse() * (transform.translation - camera.translation);
        let travel = projected_motion(local, camera.rotation.inverse() * -velocity);
        transform.rotation = camera.rotation * Quat::from_rotation_z(travel.y.atan2(travel.x));
        let width = cfg.mote_width.max(0.0) * (1.0 + (mote.0 % 3) as f32 * 0.5);
        transform.scale = Vec3::new(
            width * (1.0 + velocity.length() * cfg.mote_streak_per_speed.max(0.0)),
            width,
            1.0,
        );
    }
}
fn projected_motion(position: Vec3, velocity: Vec3) -> Vec2 {
    velocity.truncate() * -position.z + position.truncate() * velocity.z
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_stars(
    mut commands: Commands,
    world: Option<Res<WorldConfig>>,
    defaults: Local<NativeRenderConfig>,
    phase: Res<State<GamePhase>>,
    mut cameras: Query<
        (
            &Camera,
            &GlobalTransform,
            &Projection,
            &mut flare::FlareSettings,
        ),
        With<GameCamera>,
    >,
    stars: Query<(Entity, &GlobalTransform, &StarSection)>,
    ship: Query<&ShipViewMode, With<LocalShip>>,
    mut lights: Query<(Entity, Option<&ChildOf>, &mut DirectionalLight)>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    noncasters: Query<
        Entity,
        (
            Or<(With<StarSection>, With<StarHalo>, With<BillboardPose>)>,
            Without<NotShadowCaster>,
        ),
    >,
) {
    for entity in &noncasters {
        commands.entity(entity).insert(NotShadowCaster);
    }
    let cfg = world
        .as_ref()
        .and_then(|w| w.render.as_ref())
        .map(|r| &r.native)
        .unwrap_or(&defaults);
    let Ok((camera, global, projection, mut flare)) = cameras.single_mut() else {
        return;
    };
    flare.source.w = 0.0;
    let active = *phase.get() == GamePhase::InProgress && ship.single().is_ok_and(visible_view);
    let dominant = stars.iter().max_by(|a, b| {
        let score = |s: &(Entity, &GlobalTransform, &StarSection)| {
            s.2 .0.radius / s.1.translation().distance(global.translation()).max(0.001)
        };
        score(a).total_cmp(&score(b)).then_with(|| a.0.cmp(&b.0))
    });
    shadow_map.size = cfg.shadow_resolution.clamp(256, 4096).next_power_of_two();
    for (entity, parent, mut light) in &mut lights {
        let root = parent.map(ChildOf::parent).unwrap_or(entity);
        if stars.get(root).is_err() {
            continue;
        }
        let enabled = active && cfg.star_shadows && dominant.is_some_and(|s| s.0 == root);
        if light.shadows_enabled != enabled {
            light.shadows_enabled = enabled;
        }
        if enabled {
            commands.entity(entity).insert(
                CascadeShadowConfigBuilder {
                    num_cascades: 3,
                    maximum_distance: cfg.shadow_distance.max(1.0),
                    ..default()
                }
                .build(),
            );
        }
    }
    if !active {
        return;
    }
    let Some((_, star_global, star)) = dominant else {
        return;
    };
    let Projection::Perspective(p) = projection else {
        return;
    };
    let radius = star.0.radius
        * star_global
            .to_scale_rotation_translation()
            .0
            .abs()
            .max_element();
    let center = star_global.translation();
    let local = global.affine().inverse().transform_point3(center);
    if local.z >= -radius {
        return;
    }
    let front = center + (global.translation() - center).normalize_or_zero() * radius;
    let Some(ndc) = camera.world_to_ndc(global, front) else {
        return;
    };
    let radius_y = radius / (-local.z * (p.fov * 0.5).tan()) * 0.5;
    let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let fade = ((0.6 - (uv - Vec2::splat(0.5)).abs().max_element()) / 0.1).clamp(0.0, 1.0);
    flare.source = Vec4::new(
        uv.x,
        uv.y,
        ndc.z,
        cfg.flare_intensity.clamp(0.0, 3.0) * fade,
    );
    flare.shape = Vec4::new(radius_y / p.aspect_ratio, radius_y, p.aspect_ratio, 0.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streak_tracks_perspective_motion_including_reverse_and_strafe() {
        for position in [Vec3::new(3.0, 2.0, -10.0), Vec3::new(-2.0, -1.0, -15.0)] {
            for velocity in [
                Vec3::Z,
                Vec3::NEG_Z,
                Vec3::X,
                Vec3::Y,
                Vec3::new(2.0, 1.0, 3.0),
            ] {
                let project = |p: Vec3| p.truncate() / -p.z;
                let observed =
                    (project(position + velocity * 0.001) - project(position)).normalize();
                assert!(
                    projected_motion(position, velocity)
                        .normalize()
                        .dot(observed)
                        > 0.999
                );
            }
        }
    }
}
