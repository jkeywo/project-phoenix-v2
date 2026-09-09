//! Native-only visual experiment. See ../README.md for controls and limitations.
mod flare;

#[cfg(target_arch = "wasm32")]
compile_error!("The lighting lab requires a native GPU backend.");

use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::{prepass::DepthPrepass, tonemapping::Tonemapping},
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    light::{
        CascadeShadowConfigBuilder, DirectionalLightShadowMap, FogVolume, NotShadowCaster,
        VolumetricFog, VolumetricLight,
    },
    post_process::bloom::Bloom,
    prelude::*,
    reflect::TypePath,
    render::{
        render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat},
        view::{
            screenshot::{save_to_disk, Screenshot},
            Hdr,
        },
    },
    shader::ShaderRef,
};
use serde::Deserialize;
use std::path::PathBuf;

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
    fog_extent: f32,
    density: f32,
    scattering: f32,
    asymmetry: f32,
    fog_steps: u32,
    flare: f32,
    bloom: f32,
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
    mode: usize,
    rays: bool,
    shadows: bool,
    flare: bool,
    flare_intensity: f32,
    cruise: bool,
    density: f32,
    yaw: f32,
    pitch: f32,
    velocity: Vec3,
    paused: bool,
    elapsed: f32,
    capture: Option<PathBuf>,
    capture_requested: bool,
    capture_age: f32,
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
        "assets/shaders/dust_mote.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

fn main() {
    let root = std::env::current_dir()
        .ok()
        .filter(|path| path.join("prototypes/native-lighting/scene.toml").is_file())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .canonicalize()
        .expect("repository root");
    let config: Config = toml::from_str(
        &std::fs::read_to_string(root.join("prototypes/native-lighting/scene.toml"))
            .expect("scene.toml"),
    )
    .expect("valid scene.toml");
    assert!(config.fog_extent > 0.0 && config.mote_radius > 0.0 && config.mote_depth > 0.0);
    let labels =
        std::fs::read_to_string(root.join("assets/strings/strings.csv")).expect("string table");
    // Lab rows contain no CSV quoting or commas. The game's localisation pipeline is untouched.
    let labels = Labels(
        labels
            .lines()
            .filter(|line| line.starts_with("lab.lighting."))
            .map(|line| {
                let mut fields = line.splitn(3, ',');
                let id = fields.next().unwrap().to_owned();
                fields.next();
                (id, fields.next().unwrap_or("").to_owned())
            })
            .collect(),
    );
    let args: Vec<String> = std::env::args().collect();
    let capture = args
        .iter()
        .position(|a| a == "--capture")
        .map(|i| PathBuf::from(args.get(i + 1).expect("--capture needs a PNG path")));
    let mode = args
        .iter()
        .position(|a| a == "--mode")
        .map(|i| {
            args.get(i + 1)
                .expect("--mode needs 1..3")
                .parse::<usize>()
                .expect("numeric mode")
                - 1
        })
        .unwrap_or(0);
    assert!(mode < 3);
    let flare_intensity = args
        .iter()
        .position(|a| a == "--flare")
        .map(|i| {
            args.get(i + 1)
                .expect("--flare needs an intensity")
                .parse::<f32>()
                .expect("numeric flare intensity")
        })
        .unwrap_or(config.flare);
    assert!(
        flare_intensity.is_finite(),
        "flare intensity must be finite"
    );
    let title = labels.get("lab.lighting.title").to_owned();
    App::new()
        .insert_resource(Lab {
            mode,
            rays: false,
            shadows: !args.iter().any(|a| a == "--no-shadows"),
            flare: !args.iter().any(|a| a == "--no-flare"),
            flare_intensity: flare_intensity.clamp(0.0, 3.0),
            cruise: args.iter().any(|a| a == "--cruise"),
            density: config.density,
            yaw: 0.0,
            pitch: 0.0,
            velocity: Vec3::ZERO,
            paused: capture.is_some(),
            elapsed: if capture.is_some() { 3.0 } else { 0.0 },
            capture,
            capture_requested: false,
            capture_age: 0.0,
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
                    file_path: root.to_string_lossy().into_owned(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title,
                        resolution: (1440, 900).into(),
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
        .add_systems(
            Update,
            (
                controls,
                animate,
                update_dust,
                update_flare,
                hud,
                capture_frame,
            )
                .chain(),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    cfg: Res<Config>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut dust: ResMut<Assets<DustMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    commands.spawn((
        LabCamera,
        Camera3d::default(),
        Hdr,
        Msaa::Off,
        DepthPrepass,
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
        Bloom {
            intensity: cfg.bloom,
            ..default()
        },
        VolumetricFog {
            ambient_intensity: 0.0,
            step_count: cfg.fog_steps,
            ..default()
        },
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
        VolumetricLight,
        CascadeShadowConfigBuilder {
            num_cascades: 3,
            maximum_distance: cfg.shadow_distance,
            first_cascade_far_bound: 25.0,
            ..default()
        }
        .build(),
    ));
    commands.spawn((
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
        SceneRoot(assets.load(format!("assets/{}", cfg.model))),
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
    commands.spawn((
        FogVolume {
            density_texture: Some(images.add(density_image())),
            density_factor: cfg.density,
            scattering: cfg.scattering,
            absorption: 0.08,
            scattering_asymmetry: cfg.asymmetry,
            fog_color: Color::srgb(0.8, 0.87, 1.0),
            ..default()
        },
        Transform::from_translation(cfg.camera.into()).with_scale(Vec3::splat(cfg.fog_extent)),
    ));
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
                texture: assets.load(format!("assets/pfx/{name}")),
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

fn density_image() -> Image {
    const N: u32 = 64;
    let mut data = Vec::with_capacity((N * N * N) as usize);
    for z in 0..N {
        for y in 0..N {
            for x in 0..N {
                let p = Vec3::new(x as f32, y as f32, z as f32) * std::f32::consts::TAU / N as f32;
                let broad = (p.x * 3.0 + (p.z * 2.0).sin()).sin() * (p.y * 2.0 + p.z).cos();
                let fine = (p.x * 11.0 + p.y * 7.0).sin() * (p.z * 9.0 - p.y * 4.0).cos();
                let density = (0.3 + broad * 0.45 + fine * 0.14).clamp(0.0, 1.0);
                data.push((density * 255.0) as u8);
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: N,
            height: N,
            depth_or_array_layers: N,
        },
        TextureDimension::D3,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    cfg: Res<Config>,
    mut lab: ResMut<Lab>,
    mut camera: Query<&mut Transform, With<LabCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    let dt = time.delta_secs().min(0.05);
    for (key, mode) in [
        (KeyCode::Digit1, 0),
        (KeyCode::Digit2, 1),
        (KeyCode::Digit3, 2),
    ] {
        if keys.just_pressed(key) {
            lab.mode = mode;
        }
    }
    if keys.just_pressed(KeyCode::KeyL) {
        lab.rays = !lab.rays;
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
    lab.flare_intensity = (lab.flare_intensity + flare_delta * flare_rate * dt).clamp(0.0, 3.0);
    if keys.just_pressed(KeyCode::Space) {
        lab.cruise = !lab.cruise;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        lab.paused = !lab.paused;
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        lab.density = (lab.density / 1.4).max(0.0001);
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        lab.density = (lab.density * 1.4).min(0.2);
    }
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
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
    mut commands: Commands,
    cfg: Res<Config>,
    lab: Res<Lab>,
    camera: Query<
        (Entity, &Transform, Option<&VolumetricFog>),
        (With<LabCamera>, Without<Mote>, Without<FogVolume>),
    >,
    mut fog: Query<(&mut Transform, &mut FogVolume), Without<Mote>>,
    mut motes: Query<(&Mote, &mut Transform, &mut Visibility), Without<FogVolume>>,
) {
    let Ok((entity, camera, volumetric)) = camera.single() else {
        return;
    };
    let enabled = lab.mode != 0 && lab.rays;
    if enabled && volumetric.is_none() {
        commands.entity(entity).insert(VolumetricFog {
            ambient_intensity: 0.0,
            step_count: cfg.fog_steps,
            ..default()
        });
    } else if !enabled && volumetric.is_some() {
        commands.entity(entity).remove::<VolumetricFog>();
    }
    for (mut transform, mut fog) in &mut fog {
        transform.translation = camera.translation;
        // Compensate volume movement: the density remains in world space and never rotates with the camera.
        fog.density_texture_offset = camera.translation / cfg.fog_extent;
        fog.density_factor = if lab.mode != 0 && lab.rays {
            lab.density
        } else {
            0.0
        };
    }
    let half = Vec3::new(cfg.mote_radius, cfg.mote_radius, cfg.mote_depth * 0.5);
    for (mote, mut transform, mut visibility) in &mut motes {
        let visible = lab.mode == 0 || (lab.mode == 2 && mote.index % 8 == 0);
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

fn update_flare(
    cfg: Res<Config>,
    lab: Res<Lab>,
    mut camera: Query<
        (&Camera, &Transform, &Projection, &mut flare::FlareSettings),
        With<LabCamera>,
    >,
) {
    let Ok((camera, transform, projection, mut flare)) = camera.single_mut() else {
        return;
    };
    let global = GlobalTransform::from(*transform);
    let star = Vec3::from(cfg.star);
    let front = star + (transform.translation - star).normalize_or_zero() * cfg.star_radius;
    let Some(ndc) = camera.world_to_ndc(&global, front) else {
        flare.source.w = 0.0;
        return;
    };
    let local = transform.rotation.inverse() * (star - transform.translation);
    if local.z >= -cfg.star_radius {
        flare.source.w = 0.0;
        return;
    }
    let Projection::Perspective(p) = projection else {
        return;
    };
    let radius_y = cfg.star_radius / (-local.z * (p.fov * 0.5).tan()) * 0.5;
    let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let fade = ((0.6 - (uv - Vec2::splat(0.5)).abs().max_element()) / 0.1).clamp(0.0, 1.0);
    flare.source = Vec4::new(
        uv.x,
        uv.y,
        ndc.z,
        if lab.flare {
            lab.flare_intensity * fade
        } else {
            0.0
        },
    );
    flare.shape = Vec4::new(radius_y / p.aspect_ratio, radius_y, p.aspect_ratio, 0.0);
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
    let mode = labels.get(
        [
            "lab.lighting.motes",
            "lab.lighting.volume",
            "lab.lighting.hybrid",
        ][lab.mode],
    );
    for mut text in &mut text {
        **text = format!(
            "{}\n{}\n{}\n{}\n{} | L {} | H {} | F {} {:.2} | {:.1} u/s | {:.0} FPS | {:.4}",
            labels.get("lab.lighting.title"),
            labels.get("lab.lighting.controls"),
            labels.get("lab.lighting.toggles"),
            labels.get("lab.lighting.motion"),
            mode,
            yes(lab.rays),
            yes(lab.shadows),
            yes(lab.flare),
            lab.flare_intensity,
            lab.velocity.length(),
            fps,
            lab.density
        );
    }
}

fn capture_frame(
    mut commands: Commands,
    time: Res<Time>,
    mut lab: ResMut<Lab>,
    mut exit: MessageWriter<AppExit>,
) {
    if lab.capture.is_none() {
        return;
    }
    lab.capture_age += time.delta_secs();
    if !lab.capture_requested && lab.capture_age > 8.0 {
        let path = lab.capture.clone().unwrap();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("capture directory");
        }
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        lab.capture_requested = true;
    }
    if lab.capture_requested && lab.capture_age > 11.0 {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::projected_motion;
    use bevy::prelude::*;

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
