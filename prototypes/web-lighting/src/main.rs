//! WebGL2 experiment with CPU visibility probes and one shadow cascade.
mod flare;

use bevy::camera::primitives::Aabb;
use bevy::{
    core_pipeline::tonemapping::Tonemapping,
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, NotShadowCaster},
    prelude::*,
    reflect::TypePath,
    render::{render_resource::AsBindGroup, view::Hdr},
    shader::ShaderRef,
    transform::TransformSystems,
};
use serde::Deserialize;

#[derive(Resource, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    camera: [f32; 3],
    star: [f32; 3],
    star_radius: f32,
    star_illuminance: f32,
    ambient: f32,
    shadow_map: usize,
    shadow_distance: f32,
    flare: f32,
    flight_speed: f32,
    model: String,
    model_scale: f32,
    motes: usize,
    mote_radius: f32,
    mote_depth: f32,
    mote_width: f32,
}
#[derive(Resource)]
struct Lab {
    motes: bool,
    occlusion: bool,
    visibility: f32,
    shadows: bool,
    flare: bool,
    flare_intensity: f32,
    cruise: bool,
    yaw: f32,
    pitch: f32,
    velocity: Vec3,
    paused: bool,
    elapsed: f32,
}
#[derive(Resource)]
struct Labels(std::collections::HashMap<String, String>);
impl Labels {
    fn get<'a>(&'a self, id: &'a str) -> &'a str {
        self.0.get(id).map(String::as_str).unwrap_or(id)
    }
}
#[derive(Component)]
struct LabCamera;
#[derive(Component)]
struct Star;
#[derive(Component)]
struct Help;
#[derive(Component)]
struct Rock {
    phase: f32,
    home: Vec3,
}
#[derive(Component)]
struct Mote {
    index: usize,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct DustMaterial {
    #[uniform(0)]
    tint_brightness: Vec4,
    #[uniform(0)]
    opacity_masks: Vec4,
    #[texture(1)]
    #[sampler(2)]
    texture: Handle<Image>,
}
impl Material for DustMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/dust_mote.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

fn main() {
    let config: Config = toml::from_str(include_str!("../scene.toml")).expect("valid scene config");
    assert!(config.mote_radius > 0.0 && config.mote_depth > 0.0);
    let labels = Labels(
        include_str!("../../../assets/strings/strings.csv")
            .lines()
            .filter(|line| line.starts_with("lab."))
            .map(|line| {
                let mut parts = line.splitn(3, ',');
                let id = parts.next().unwrap().to_owned();
                parts.next();
                (id, parts.next().unwrap_or("").to_owned())
            })
            .collect(),
    );
    let title = labels.get("lab.web.title").to_owned();
    App::new()
        .insert_resource(Lab {
            motes: true,
            occlusion: true,
            visibility: 1.0,
            shadows: true,
            flare: true,
            flare_intensity: config.flare,
            cruise: false,
            yaw: 0.0,
            pitch: 0.0,
            velocity: Vec3::ZERO,
            paused: false,
            elapsed: 0.0,
        })
        .insert_resource(DirectionalLightShadowMap {
            size: config.shadow_map,
        })
        .insert_resource(config)
        .insert_resource(labels)
        .insert_resource(ClearColor(Color::srgb(0.001, 0.002, 0.006)))
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: "assets".into(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title,
                        resolution: (1280, 800).into(),
                        canvas: Some("#lab".into()),
                        fit_canvas_to_parent: true,
                        prevent_default_event_handling: true,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins((
            flare::FlarePlugin,
            MaterialPlugin::<DustMaterial>::default(),
            FrameTimeDiagnosticsPlugin::default(),
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, (controls, animate, update_dust, hud).chain())
        .add_systems(PostUpdate, update_flare.after(TransformSystems::Propagate))
        .run();
}

fn setup(
    mut commands: Commands,
    cfg: Res<Config>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut dust: ResMut<Assets<DustMaterial>>,
) {
    commands.spawn((
        LabCamera,
        Camera3d::default(),
        Hdr,
        Msaa::Sample4,
        Projection::Perspective(PerspectiveProjection {
            far: 1000.0,
            ..default()
        }),
        Transform::from_translation(cfg.camera.into()),
        AmbientLight {
            brightness: cfg.ambient,
            ..default()
        },
        Tonemapping::TonyMcMapface,
        flare::FlareSettings::default(),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: cfg.star_illuminance,
            color: Color::srgb(1.0, 0.91, 0.75),
            shadows_enabled: true,
            ..default()
        },
        Transform::from_translation(cfg.star.into()).looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder {
            num_cascades: 1,
            maximum_distance: cfg.shadow_distance,
            first_cascade_far_bound: 25.0,
            ..default()
        }
        .build(),
    ));
    commands.spawn((
        Star,
        Mesh3d(meshes.add(Sphere::new(cfg.star_radius).mesh().ico(4).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.78, 0.37),
            emissive: LinearRgba::new(12.0, 8.0, 3.0, 1.0),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(cfg.star.into()),
        NotShadowCaster,
    ));
    commands.spawn((
        SceneRoot(assets.load(cfg.model.clone())),
        Transform::from_xyz(-3.0, -1.0, 0.0).with_scale(Vec3::splat(cfg.model_scale)),
    ));
    let rock_mesh = meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap());
    let rock_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.22, 0.20, 0.18),
        perceptual_roughness: 0.94,
        ..default()
    });
    for i in 0..14 {
        let phase = i as f32 * 2.39996;
        let home = if i == 0 {
            Vec3::new(6.0, 5.5, -20.0)
        } else {
            Vec3::new(phase.sin() * 15.0, phase.cos() * 7.0, -8.0 - i as f32 * 3.5)
        };
        commands.spawn((
            Rock { phase, home },
            Mesh3d(rock_mesh.clone()),
            MeshMaterial3d(rock_mat.clone()),
            Transform::from_translation(home).with_scale(
                Vec3::new(1.6, 2.0, 1.3) * if i == 0 { 2.2 } else { 0.6 + i as f32 * 0.045 },
            ),
        ));
    }
    // A receiver with ribs makes cast shadows easy to judge without a planet shader.
    let deck = materials.add(StandardMaterial {
        base_color: Color::srgb(0.32, 0.38, 0.43),
        metallic: 0.55,
        perceptual_roughness: 0.5,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(28.0, 0.4, 24.0))),
        MeshMaterial3d(deck.clone()),
        Transform::from_xyz(0.0, -5.0, -6.0),
    ));
    for x in [-10.0, -5.0, 0.0, 5.0, 10.0] {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.35, 4.0, 0.35))),
            MeshMaterial3d(deck.clone()),
            Transform::from_xyz(x, -3.0, -12.0),
        ));
    }
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let textures = [
        "space_mote_streak_head.png",
        "space_mote_streak_soft.png",
        "space_mote_compact_core.png",
    ];
    let layers: Vec<_> = textures
        .iter()
        .enumerate()
        .map(|(i, name)| {
            dust.add(DustMaterial {
                tint_brightness: Vec4::new(0.7, 0.8, 1.0, 0.7 - i as f32 * 0.15),
                opacity_masks: Vec4::new(0.45, 0.12, 0.38, 0.15),
                texture: assets.load(format!("pfx/{name}")),
            })
        })
        .collect();
    for index in 0..cfg.motes {
        commands.spawn((
            Mote { index },
            Mesh3d(quad.clone()),
            MeshMaterial3d(layers[index % 3].clone()),
            Transform::default(),
            NotShadowCaster,
        ));
    }
    commands.spawn((
        Help,
        Text::new(""),
        TextFont {
            font_size: 17.0,
            ..default()
        },
        TextColor(Color::srgb(0.75, 0.86, 0.95)),
        Node {
            position_type: PositionType::Absolute,
            top: px(16),
            left: px(20),
            ..default()
        },
    ));
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    cfg: Res<Config>,
    mut lab: ResMut<Lab>,
    mut camera: Query<&mut Transform, With<LabCamera>>,
) {
    let dt = time.delta_secs().min(0.05);
    if keys.just_pressed(KeyCode::KeyM) {
        lab.motes = !lab.motes;
    }
    if keys.just_pressed(KeyCode::KeyO) {
        lab.occlusion = !lab.occlusion;
    }
    if keys.just_pressed(KeyCode::KeyH) {
        lab.shadows = !lab.shadows;
    }
    if keys.just_pressed(KeyCode::KeyF) {
        lab.flare = !lab.flare;
    }
    let flare_delta = u8::from(keys.pressed(KeyCode::Equal)) as f32
        - u8::from(keys.pressed(KeyCode::Minus)) as f32;
    let flare_rate = if keys.pressed(KeyCode::ShiftLeft) {
        0.1
    } else {
        0.5
    };
    let flare_tap = u8::from(keys.just_pressed(KeyCode::Equal)) as f32
        - u8::from(keys.just_pressed(KeyCode::Minus)) as f32;
    let adjustment = if flare_tap != 0.0 {
        flare_tap * flare_rate * 0.2
    } else {
        flare_delta * flare_rate * dt
    };
    lab.flare_intensity = (lab.flare_intensity + adjustment).clamp(0.0, 3.0);
    if keys.just_pressed(KeyCode::Space) {
        lab.cruise = !lab.cruise;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        lab.paused = !lab.paused;
    }
    let Ok(mut camera) = camera.single_mut() else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyR) {
        *camera = Transform::from_translation(cfg.camera.into());
        lab.yaw = 0.0;
        lab.pitch = 0.0;
        lab.cruise = false;
        lab.elapsed = 0.0;
    }
    let axis = |positive, negative| {
        u8::from(keys.pressed(positive)) as f32 - u8::from(keys.pressed(negative)) as f32
    };
    lab.yaw += axis(KeyCode::ArrowLeft, KeyCode::ArrowRight) * dt;
    lab.pitch = (lab.pitch + axis(KeyCode::ArrowUp, KeyCode::ArrowDown) * dt).clamp(-1.4, 1.4);
    camera.rotation = Quat::from_euler(EulerRot::YXZ, lab.yaw, lab.pitch, 0.0);
    let forward = axis(KeyCode::KeyW, KeyCode::KeyS) + if lab.cruise { 1.0 } else { 0.0 };
    let speed = cfg.flight_speed
        * if keys.pressed(KeyCode::ShiftLeft) {
            4.0
        } else {
            1.0
        };
    lab.velocity = camera.rotation
        * Vec3::new(
            axis(KeyCode::KeyD, KeyCode::KeyA),
            axis(KeyCode::KeyE, KeyCode::KeyQ),
            -forward,
        )
        * speed;
    if !lab.paused {
        camera.translation += lab.velocity * dt;
        lab.elapsed += dt;
    }
}

fn animate(
    cfg: Res<Config>,
    lab: Res<Lab>,
    mut rocks: Query<(&Rock, &mut Transform)>,
    mut lights: Query<&mut DirectionalLight>,
) {
    for (rock, mut transform) in &mut rocks {
        transform.translation = rock.home
            + Vec3::new(
                (lab.elapsed * 0.45 + rock.phase).sin() * 4.0,
                (lab.elapsed * 0.3 + rock.phase).sin(),
                0.0,
            );
        transform.rotation = Quat::from_euler(
            EulerRot::XYZ,
            lab.elapsed * 0.1,
            rock.phase + lab.elapsed * 0.17,
            0.2,
        );
    }
    for mut light in &mut lights {
        light.shadows_enabled = lab.shadows;
        light.illuminance = cfg.star_illuminance;
    }
}

fn update_dust(
    cfg: Res<Config>,
    lab: Res<Lab>,
    camera: Query<&Transform, (With<LabCamera>, Without<Mote>)>,
    mut motes: Query<(&Mote, &mut Transform, &mut Visibility)>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let half = Vec3::new(cfg.mote_radius, cfg.mote_radius, cfg.mote_depth * 0.5);
    for (mote, mut transform, mut visibility) in &mut motes {
        let visible = lab.motes;
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if !visible {
            continue;
        }
        let n = mote.index as f32;
        let seed = Vec3::new(
            (n * 0.7548777).fract(),
            (n * 0.5698403).fract(),
            (n * 0.438289).fract(),
        );
        let anchor = seed * half * 2.0;
        let relative = anchor - camera.translation;
        transform.translation = camera.translation
            + Vec3::new(
                (relative.x + half.x).rem_euclid(half.x * 2.0) - half.x,
                (relative.y + half.y).rem_euclid(half.y * 2.0) - half.y,
                (relative.z + half.z).rem_euclid(half.z * 2.0) - half.z,
            );
        let local_velocity = camera.rotation.inverse() * -lab.velocity;
        let local_position =
            camera.rotation.inverse() * (transform.translation - camera.translation);
        // Differentiate perspective x/-z and y/-z. Forward flight fans out
        // radially; using velocity.xy alone makes every forward streak horizontal.
        let projected_velocity = projected_motion(local_position, local_velocity);
        let angle = projected_velocity.y.atan2(projected_velocity.x);
        transform.rotation = camera.rotation * Quat::from_rotation_z(angle);
        let width = cfg.mote_width * (1.0 + (mote.index % 3) as f32 * 0.5);
        transform.scale = Vec3::new(width * (1.0 + lab.velocity.length() * 0.25), width, 1.0);
    }
}

fn projected_motion(position: Vec3, velocity: Vec3) -> Vec2 {
    velocity.truncate() * -position.z + position.truncate() * velocity.z
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
fn update_flare(
    cfg: Res<Config>,
    time: Res<Time>,
    mut lab: ResMut<Lab>,
    mut cameras: Query<
        (
            &Camera,
            &GlobalTransform,
            &Projection,
            &mut flare::FlareSettings,
        ),
        With<LabCamera>,
    >,
    boxes: Query<
        (&GlobalTransform, &Aabb, &InheritedVisibility),
        (With<Mesh3d>, Without<Mote>, Without<Star>, Without<Rock>),
    >,
    rocks: Query<&GlobalTransform, With<Rock>>,
) {
    let Ok((camera, global, projection, mut flare)) = cameras.single_mut() else {
        return;
    };
    flare.source.w = 0.0;
    let Projection::Perspective(p) = projection else {
        return;
    };
    let star = Vec3::from(cfg.star);
    let local = global.affine().inverse().transform_point3(star);
    if local.z >= -cfg.star_radius {
        lab.visibility = 0.0;
        return;
    }
    let Some(ndc) = camera.world_to_ndc(global, star) else {
        return;
    };
    let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let fade = ((0.6 - (uv - Vec2::splat(0.5)).abs().max_element()) / 0.1).clamp(0.0, 1.0);
    if fade <= 0.0 {
        lab.visibility = 0.0;
        return;
    }
    let eye = global.translation();
    let toward_eye = (eye - star).normalize_or_zero();
    let right = Vec3::Y.cross(toward_eye).normalize_or_zero();
    let up = toward_eye.cross(right);
    // Cache inverses once per object rather than once per visibility ray.
    let boxes: Vec<_> = boxes
        .iter()
        .filter(|(_, _, visible)| visible.get())
        .map(|(t, a, _)| {
            (
                t.affine().inverse(),
                Vec3::from(a.center),
                Vec3::from(a.half_extents),
            )
        })
        .collect();
    let rocks: Vec<_> = rocks.iter().map(|t| t.affine().inverse()).collect();
    let mut visible = 0.0;
    const SAMPLES: u32 = 24;
    if lab.occlusion {
        for i in 0..SAMPLES {
            let theta = i as f32 * 2.399963;
            let r = ((i as f32 + 0.5) / SAMPLES as f32).sqrt() * 0.88;
            let target = star
                + cfg.star_radius
                    * (right * theta.cos() * r
                        + up * theta.sin() * r
                        + toward_eye * (1.0 - r * r).sqrt());
            let blocked = rocks
                .iter()
                .any(|inv| segment_sphere(inv.transform_point3(eye), inv.transform_point3(target)))
                || boxes.iter().any(|(inv, center, half)| {
                    segment_box(
                        inv.transform_point3(eye),
                        inv.transform_point3(target),
                        *center,
                        *half,
                    )
                });
            if !blocked {
                visible += 1.0 / SAMPLES as f32;
            }
        }
    } else {
        visible = 1.0;
    }
    // Smooth only the visibility fraction; the camera position remains immediate.
    let blend = 1.0 - (-time.delta_secs() * 12.0).exp();
    lab.visibility += (visible - lab.visibility) * blend;
    let radius_y = cfg.star_radius / (-local.z * (p.fov * 0.5).tan()) * 0.5;
    flare.source = Vec4::new(
        uv.x,
        uv.y,
        0.0,
        if lab.flare {
            lab.flare_intensity * fade
        } else {
            0.0
        },
    );
    flare.shape = Vec4::new(
        radius_y / p.aspect_ratio,
        radius_y,
        p.aspect_ratio,
        lab.visibility,
    );
}

fn hud(
    labels: Res<Labels>,
    diagnostics: Res<DiagnosticsStore>,
    lab: Res<Lab>,
    mut text: Query<&mut Text, With<Help>>,
) {
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let yes = |on| {
        labels.get(if on {
            "lab.lighting.on"
        } else {
            "lab.lighting.off"
        })
    };
    for mut text in &mut text {
        **text = format!(
            "{}\n{}\n{}\nM {} | H {} | F {} {:.2} | O {} {:.0}% | {:.1} u/s | {:.0} FPS",
            labels.get("lab.web.title"),
            labels.get("lab.web.controls"),
            labels.get("lab.web.motion"),
            yes(lab.motes),
            yes(lab.shadows),
            yes(lab.flare),
            lab.flare_intensity,
            yes(lab.occlusion),
            lab.visibility * 100.0,
            lab.velocity.length(),
            fps
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{projected_motion, segment_box, segment_sphere};
    use bevy::prelude::*;

    #[test]
    fn visibility_segments_stop_at_the_star_and_respect_misses() {
        let eye = Vec3::new(0.0, 0.0, 5.0);
        let beyond = Vec3::new(0.0, 0.0, -5.0);
        assert!(segment_box(eye, beyond, Vec3::ZERO, Vec3::ONE));
        assert!(segment_sphere(eye, beyond));
        let near_star = Vec3::new(0.0, 0.0, 2.0);
        assert!(!segment_box(eye, near_star, Vec3::ZERO, Vec3::ONE));
        assert!(!segment_sphere(eye, near_star));
        let offset = Vec3::X * 3.0;
        assert!(!segment_box(
            eye + offset,
            beyond + offset,
            Vec3::ZERO,
            Vec3::ONE
        ));
        assert!(!segment_sphere(eye + offset, beyond + offset));
    }

    #[test]
    fn streak_direction_matches_observed_perspective_motion() {
        // Check against an actual projected displacement, including reverse
        // and strafe. Forward velocity alone has no screen-space XY direction.
        for position in [Vec3::new(3.0, 2.0, -20.0), Vec3::new(-5.0, 4.0, -12.0)] {
            for velocity in [
                Vec3::Z * 8.0,
                Vec3::NEG_Z * 8.0,
                Vec3::X * 5.0,
                Vec3::new(3.0, -2.0, 5.0),
            ] {
                let project = |p: Vec3| p.truncate() / -p.z;
                let observed =
                    (project(position + velocity * 0.01) - project(position)).normalize();
                let streak = projected_motion(position, velocity).normalize();
                assert!(streak.dot(observed) > 0.9999);
            }
        }
    }
}
