//! Offscreen native counterpart to scripts/capture-planet.mjs.
//! cargo run --features capture --example capture_planet -- target/planet-native
#![allow(clippy::disallowed_methods)]
use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    camera::RenderTarget,
    prelude::*,
    render::renderer::RenderDevice,
    window::ExitCondition,
    winit::WinitPlugin,
};
use project_phoenix::{
    entities::{
        config::EntityConfig,
        planet::{
            PlanetCloudMaterial, PlanetLightingOverride, PlanetRenderPlugin, PlanetSurfaceMaterial,
        },
    },
    render_capture::{create_render_target, unpad_rows, ImageCopyPlugin, MainWorldReceiver},
};

#[derive(Resource)]
struct Capture {
    output: std::path::PathBuf,
    radius_scale: f32,
    textures: Vec<Handle<Image>>,
    frame: usize,
    shot: usize,
    started: std::time::Instant,
}
const SHOTS: [(&str, f32, f32, f32, f32, f32); 5] = [
    ("day", 0.6, 0.2, 105.0, 0.9, 0.35),
    ("terminator", 0.6, 0.2, 105.0, 2.1, 0.2),
    ("night", 0.6, 0.2, 105.0, 3.7, 0.2),
    ("close", 0.6, 0.2, 66.0, 1.5, 0.3),
    ("pole", 0.6, 1.5, 105.0, 1.8, 0.4),
];
fn main() {
    std::env::set_var("BEVY_ASSET_ROOT", std::env::current_dir().unwrap());
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/planet-native".into())
        .into();
    App::new()
        .insert_resource(Capture {
            output,
            radius_scale: 1.0,
            textures: Vec::new(),
            frame: 0,
            shot: 0,
            started: std::time::Instant::now(),
        })
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(PlanetLightingOverride::default())
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins((
            ImageCopyPlugin,
            PlanetRenderPlugin,
            ScheduleRunnerPlugin::run_loop(std::time::Duration::from_millis(1)),
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, drive)
        .run();
}
// Each argument is a distinct Bevy system parameter.
#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    mut capture: ResMut<Capture>,
    server: Res<AssetServer>,
    device: Res<RenderDevice>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut surfaces: ResMut<Assets<PlanetSurfaceMaterial>>,
    mut clouds: ResMut<Assets<PlanetCloudMaterial>>,
) {
    let entity_path = std::env::var("PLANET_ENTITY")
        .unwrap_or_else(|_| "assets/entities/planet_ecumenopolis.toml".into());
    let text = std::fs::read_to_string(entity_path).unwrap();
    let config = EntityConfig::from_toml(&text).unwrap().planet.unwrap();
    capture.radius_scale = config.radius / 33.0;
    capture.textures = project_phoenix::entities::planet::planet_texture_paths(&config)
        .iter()
        .map(|(path, srgb)| {
            project_phoenix::entities::planet::load_planet_image(&server, path, *srgb)
        })
        .collect();
    let entity = commands
        .spawn((Transform::default(), Visibility::default()))
        .id();
    project_phoenix::entities::celestial_visual::insert_planet_visual(
        &mut commands,
        &mut meshes,
        &mut surfaces,
        &mut clouds,
        &server,
        entity,
        &config,
    );
    let (image, copier) = create_render_target(&mut images, &device, 1280, 900);
    commands.spawn(copier);
    let camera = commands
        .spawn((
            Camera3d::default(),
            RenderTarget::from(image),
            project_phoenix::render_setup::game_camera_projection(),
            Transform::default(),
        ))
        .id();
    project_phoenix::render_setup::apply_render_config(
        &mut commands,
        camera,
        &project_phoenix::world::config::RenderConfig::default(),
    );
}
fn drive(
    mut capture: ResMut<Capture>,
    server: Res<AssetServer>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut light: ResMut<PlanetLightingOverride>,
    receiver: Res<MainWorldReceiver>,
    mut exit: MessageWriter<AppExit>,
) {
    if capture.shot >= SHOTS.len() {
        return;
    }
    if capture
        .textures
        .iter()
        .any(|image| !server.is_loaded_with_dependencies(image.id()))
    {
        while receiver.try_recv().is_ok() {}
        assert!(
            capture.started.elapsed().as_secs() < 60,
            "planet textures did not load"
        );
        return;
    }
    let (name, yaw, pitch, radius, sun_yaw, sun_pitch) = SHOTS[capture.shot];
    if capture.frame == 0 {
        *camera.single_mut().unwrap() = Transform::from_translation(
            Vec3::new(
                yaw.sin() * pitch.cos(),
                pitch.sin(),
                yaw.cos() * pitch.cos(),
            ) * radius
                * capture.radius_scale,
        )
        .looking_at(Vec3::ZERO, Vec3::Y);
        light.light_dir = Quat::from_euler(EulerRot::YXZ, sun_yaw, sun_pitch, 0.0) * Vec3::Z;
        capture.started = std::time::Instant::now();
    }
    capture.frame += 1;
    let mut bytes = Vec::new();
    while let Ok(frame) = receiver.try_recv() {
        bytes = frame;
    }
    if capture.frame < 90 || bytes.is_empty() {
        return;
    }
    let pixels = unpad_rows(&bytes, 1280, 900);
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| p[0].max(p[1]).max(p[2]) > 12)
            .count()
            > 1000,
        "planet did not render"
    );
    std::fs::create_dir_all(&capture.output).unwrap();
    image::RgbaImage::from_raw(1280, 900, pixels)
        .unwrap()
        .save(capture.output.join(format!("{name}.png")))
        .unwrap();
    eprintln!(
        "{name}: {:.1} ms/frame including readback and startup",
        capture.started.elapsed().as_secs_f64() * 1000.0 / capture.frame as f64
    );
    capture.frame = 0;
    capture.shot += 1;
    if capture.shot == SHOTS.len() {
        exit.write(AppExit::Success);
    }
}
