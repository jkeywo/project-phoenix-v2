//! capture-billboard — bake a model's far-LOD billboard atlas, headless.
//!
//!   cargo run --features capture --bin capture-billboard -- \
//!       assets/models/alliance_cruiser.glb assets/models/alliance_cruiser_lod3.png \
//!       [--views 8] [--resolution 256] [--pitch 20]
//!
//! Renders the model to a transparent RGBA target from `views` yaw angles around
//! a horizontal ring, packs the tiles left→right into one PNG, and prints the
//! hull's world size as JSON (`{"world_w":..,"world_h":..}`) for the sidecar's
//! billboard `scale`. It is the native, batchable companion to the model
//! viewer's WASM capture: no window, no visible pane — a local GPU step, like the
//! Blender voxel remesh, that the ladder authoring (scripts/) drives per model.
//!
//! Why native + headless rather than the browser viewer: a WASM/canvas app only
//! renders while its pane composites, so it can neither run headless nor batch.
//! This follows Bevy's `headless_renderer` example — render to an image target,
//! copy the texture to a CPU buffer in a render-graph node, map it, and save —
//! which renders reliably with no display.
//!
//! The offscreen-render plumbing (the render-graph readback, the transparent
//! target, the framing maths) lives in [`project_phoenix::render_capture`], which
//! this tool and `tune-lods` share.

use std::path::PathBuf;
use std::time::Duration;

use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    camera::{ClearColorConfig, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::renderer::RenderDevice,
    window::ExitCondition,
    winit::WinitPlugin,
};

use project_phoenix::entities::model_rig::ModelRig;
use project_phoenix::render_capture::{
    create_render_target, frame_distance, measure_world_bounds, orbit_transform, unpad_rows,
    ImageCopyPlugin, MainWorldReceiver,
};

// ── Config from argv ────────────────────────────────────────────────────────

#[derive(Resource, Clone)]
struct CaptureConfig {
    model: String,
    output: PathBuf,
    views: u32,
    resolution: u32,
    pitch_deg: f32,
    /// The rig's base transform, so the yaw views align with the game's forward.
    base: Transform,
}

fn parse_config() -> CaptureConfig {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if positional.len() < 2 {
        eprintln!(
            "usage: capture-billboard <model.glb> <out.png> [--views 8] [--resolution 256] [--pitch 20]"
        );
        std::process::exit(2);
    }
    let flag = |name: &str, dflt: f32| -> f32 {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(dflt)
    };
    let model = positional[0].clone();
    let output = PathBuf::from(positional[1]);

    // Orientation comes from the rig sidecar's `[base]` (`<stem>.model.toml`), so
    // the yaw views align with the game's forward. Framing is measured from the
    // live geometry, not the sidecar. Falls back to identity if there is no rig.
    let sidecar = model.replace(".glb", ".model.toml");
    let base = std::fs::read_to_string(&sidecar)
        .ok()
        .and_then(|t| ModelRig::from_toml(&t).ok())
        .map(|rig| rig.base_bevy_transform())
        .unwrap_or_default();

    CaptureConfig {
        model,
        output,
        views: flag("--views", 8.0) as u32,
        resolution: flag("--resolution", 256.0) as u32,
        pitch_deg: flag("--pitch", 20.0),
        base,
    }
}

fn main() {
    // Root Bevy's asset server at the working directory (the project root), not
    // the exe's folder — this tool is run from the repo root like the other
    // scripts, and its `assets/` is there.
    if std::env::var_os("BEVY_ASSET_ROOT").is_none() {
        if let Ok(cwd) = std::env::current_dir() {
            std::env::set_var("BEVY_ASSET_ROOT", cwd);
        }
    }

    let config = parse_config();
    let res = config.resolution.max(1);

    App::new()
        .insert_resource(config)
        .insert_resource(ClearColor(Color::NONE))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins(ImageCopyPlugin)
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0)))
        .insert_resource(CaptureProgress::default())
        .insert_resource(TargetSize(res))
        .add_systems(Startup, setup)
        .add_systems(PostUpdate, drive)
        .run();
}

// ── Scene setup ──────────────────────────────────────────────────────────────

#[derive(Resource)]
struct TargetSize(u32);

/// The offscreen camera, repositioned per yaw view.
#[derive(Component)]
struct CaptureCamera;

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
    config: Res<CaptureConfig>,
    size: Res<TargetSize>,
) {
    let res = size.0;

    // Transparent RGBA target + a CPU-mappable copy buffer (shared core).
    let (target, copier) = create_render_target(&mut images, &render_device, res, res);
    commands.spawn(copier);

    // The model, under a parent carrying the rig's base transform, so the yaw
    // ring lines up with the game's forward (its local -Z).
    let rel = config.model.strip_prefix("assets/").unwrap_or(&config.model);
    let scene = asset_server.load(format!("{rel}#Scene0"));
    commands
        .spawn((config.base, Visibility::default()))
        .with_children(|p| {
            p.spawn((SceneRoot(scene), Transform::default()));
        });

    // Lighting in the neighbourhood of the game's default look: a key light. A
    // billboard is drawn unlit, so this only has to read as "a lit hull" at
    // distance, not match a scene exactly.
    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            ..default()
        },
        Transform::from_xyz(1.0, 2.0, 1.5).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Offscreen camera: transparent clear, no skybox, framing from the extent.
    // `AmbientLight` is a per-camera component in Bevy 0.18 — soft fill so the
    // shadowed side of the hull is not pure black.
    commands.spawn((
        CaptureCamera,
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        AmbientLight {
            color: Color::WHITE,
            brightness: 400.0,
            ..default()
        },
        RenderTarget::from(target),
        project_phoenix::render_setup::game_camera_projection(),
        Tonemapping::None,
        Transform::default(),
    ));
}

// ── The per-view state machine (main world) ──────────────────────────────────

#[derive(Resource, Default)]
struct CaptureProgress {
    /// Frames to idle before the first view, so the GLB streams + uploads.
    warmup: u32,
    view: u32,
    /// Frames waited since positioning the camera for `view`.
    settle: u32,
    tiles: Vec<Vec<u8>>,
    started: bool,
    /// World-space bounds of the actual rendered geometry, measured at start.
    center: Vec3,
    /// Half the largest world dimension — the framing radius.
    radius: f32,
    /// Full world width/height of the hull, for the billboard quad `scale`.
    world_size: [f32; 2],
}

/// Frames to let the asset stream and the pipeline fill before trusting a read.
const WARMUP_FRAMES: u32 = 90;
/// Minimum frames after moving the camera before the readback matches this view.
const SETTLE_FRAMES: u32 = 4;
/// A view with fewer opaque pixels than this is treated as "not rendered yet".
const MIN_OPAQUE_PX: usize = 40;
/// Cap on per-view waiting, so a legitimately empty view still advances.
const MAX_SETTLE_FRAMES: u32 = 120;

fn drive(
    mut progress: ResMut<CaptureProgress>,
    config: Res<CaptureConfig>,
    receiver: Res<MainWorldReceiver>,
    mut cameras: Query<&mut Transform, With<CaptureCamera>>,
    bounds_q: Query<(&GlobalTransform, &bevy::camera::primitives::Aabb)>,
    mut exit: MessageWriter<AppExit>,
) {
    if !progress.started {
        progress.warmup += 1;
        // Drain warmup frames' bytes so the first captured tile is fresh.
        while receiver.try_recv().is_ok() {}
        if progress.warmup < WARMUP_FRAMES {
            return;
        }
        // Measure the ACTUAL rendered geometry (the sidecar's `[extents]` do not
        // reliably match it — the viewer measures the live Aabb too).
        let Some((center, radius, size)) = measure_world_bounds(bounds_q.iter()) else {
            return; // meshes not measurable yet — wait
        };
        progress.center = center;
        progress.radius = radius;
        progress.world_size = [size.x.max(size.z), size.y];
        progress.started = true;
        eprintln!(
            "[capture-billboard] {}: world {:.2}×{:.2}, {} views @ {}px",
            config.model, progress.world_size[0], progress.world_size[1], config.views, config.resolution
        );
        let (center, dist) = frame(&progress);
        position_camera(&config, &mut cameras, 0, center, dist);
        return;
    }

    progress.settle += 1;
    // Keep only this frame's freshest readback.
    let mut bytes = Vec::new();
    while let Ok(data) = receiver.try_recv() {
        bytes = data;
    }
    if progress.settle < SETTLE_FRAMES || bytes.is_empty() {
        return; // minimum settle not reached, or nothing delivered yet
    }
    // The GPU mesh can finish uploading a few frames after its CPU Aabb exists,
    // so an early view renders blank. Wait (up to MAX_SETTLE) until the tile
    // actually has a silhouette before accepting it; a genuinely empty view
    // still advances at the cap rather than hanging.
    let tile = unpad_rows(&bytes, config.resolution, config.resolution);
    let opaque = tile.chunks(4).filter(|p| p.get(3).is_some_and(|&a| a > 8)).count();
    if opaque < MIN_OPAQUE_PX && progress.settle < MAX_SETTLE_FRAMES {
        return;
    }
    progress.tiles.push(tile);

    let next = progress.view + 1;
    if next < config.views {
        progress.view = next;
        progress.settle = 0;
        let (center, dist) = frame(&progress);
        position_camera(&config, &mut cameras, next, center, dist);
        return;
    }

    // All views captured — pack and save.
    save_atlas(&config, &progress.tiles);
    let [w, h] = progress.world_size;
    println!(
        "{{\"world_w\":{w},\"world_h\":{h},\"views\":{},\"resolution\":{},\"pitch\":{}}}",
        config.views, config.resolution, config.pitch_deg
    );
    exit.write(AppExit::Success);
}

/// Framing target: the measured centre and the camera distance that fits the
/// hull's radius in the game FOV, with a margin so it never touches the edge.
fn frame(progress: &CaptureProgress) -> (Vec3, f32) {
    let fov = std::f32::consts::FRAC_PI_4;
    (progress.center, frame_distance(progress.radius, fov, 1.25))
}

/// Orbit the camera to yaw `view` at the configured pitch, framed on `center`.
fn position_camera(
    config: &CaptureConfig,
    cameras: &mut Query<&mut Transform, With<CaptureCamera>>,
    view: u32,
    center: Vec3,
    distance: f32,
) {
    let yaw = view as f32 * std::f32::consts::TAU / config.views as f32;
    let pitch = config.pitch_deg.to_radians();
    if let Ok(mut tf) = cameras.single_mut() {
        *tf = orbit_transform(center, distance, yaw, pitch);
    }
}

/// Pack the yaw tiles left→right into one strip and write the PNG.
fn save_atlas(config: &CaptureConfig, tiles: &[Vec<u8>]) {
    let res = config.resolution;
    let views = config.views;
    let row = (res * 4) as usize;
    let atlas_w = res * views;
    let atlas_row = (atlas_w * 4) as usize;
    let mut atlas = vec![0u8; (atlas_w * res * 4) as usize];
    for (t, tile) in tiles.iter().enumerate() {
        if tile.len() < row * res as usize {
            continue;
        }
        let x_off = t * row;
        for y in 0..res as usize {
            let src = y * row;
            let dst = y * atlas_row + x_off;
            atlas[dst..dst + row].copy_from_slice(&tile[src..src + row]);
        }
    }
    let img = image::RgbaImage::from_raw(atlas_w, res, atlas)
        .expect("atlas buffer matches its dimensions");
    if let Some(parent) = config.output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    img.save(&config.output).expect("save atlas PNG");
    eprintln!(
        "[capture-billboard] wrote {} ({}x{}, {} views)",
        config.output.display(),
        atlas_w,
        res,
        views
    );
}
