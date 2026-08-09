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

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    camera::{ClearColorConfig, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    asset::RenderAssetUsages,
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_graph::{self, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel},
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode,
            PollType, TexelCopyBufferInfo, TexelCopyBufferLayout, TextureDimension, TextureFormat,
            TextureUsages,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        Extract, Render, RenderApp, RenderSystems,
    },
    window::ExitCondition,
    winit::WinitPlugin,
};
use crossbeam_channel::{Receiver, Sender};

use project_phoenix::entities::model_rig::ModelRig;

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

/// The image the offscreen camera renders into, kept so `drive` can read it.
#[derive(Resource)]
struct RenderTargetImage(Handle<Image>);

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
    let extent = Extent3d {
        width: res,
        height: res,
        depth_or_array_layers: 1,
    };

    // Transparent RGBA target + a CPU-mappable copy buffer.
    let mut target = Image::new_fill(
        extent,
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    target.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let target = images.add(target);

    commands.spawn(ImageCopier::new(target.clone(), extent, &render_device));
    commands.insert_resource(RenderTargetImage(target.clone()));

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
        // reliably match it — the viewer measures the live Aabb too). Union every
        // mesh's Aabb in world space.
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let mut any = false;
        for (gt, aabb) in bounds_q.iter() {
            any = true;
            let c: Vec3 = aabb.center.into();
            let h: Vec3 = aabb.half_extents.into();
            for sx in [-1.0, 1.0] {
                for sy in [-1.0, 1.0] {
                    for sz in [-1.0, 1.0] {
                        let corner = gt.transform_point(c + h * Vec3::new(sx, sy, sz));
                        lo = lo.min(corner);
                        hi = hi.max(corner);
                    }
                }
            }
        }
        if !any {
            return; // meshes not measurable yet — wait
        }
        let size = hi - lo;
        progress.center = (lo + hi) * 0.5;
        progress.radius = size.max_element().max(0.05) * 0.5;
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
    let tile = unpad_rows(&bytes, config.resolution);
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
    let distance = progress.radius / (fov * 0.5).tan() * 1.25;
    (progress.center, distance)
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
    let rot = Quat::from_euler(EulerRot::YXZ, yaw, -pitch, 0.0);
    let pos = center + rot * Vec3::new(0.0, 0.0, distance);
    if let Ok(mut tf) = cameras.single_mut() {
        *tf = Transform::from_translation(pos).looking_at(center, Vec3::Y);
    }
}

/// Strip the GPU row padding (buffers align each row to 256 bytes) back to
/// `resolution * 4` bytes per row.
fn unpad_rows(padded: &[u8], resolution: u32) -> Vec<u8> {
    let row = (resolution * 4) as usize;
    let aligned = RenderDevice::align_copy_bytes_per_row(row);
    if aligned == row {
        return padded.to_vec();
    }
    padded
        .chunks(aligned)
        .take(resolution as usize)
        .flat_map(|r| &r[..row.min(r.len())])
        .copied()
        .collect()
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

// ── Render-graph image copy (adapted from bevy's headless_renderer example) ──

#[derive(Resource, Deref)]
struct MainWorldReceiver(Receiver<Vec<u8>>);
#[derive(Resource, Deref)]
struct RenderWorldSender(Sender<Vec<u8>>);

struct ImageCopyPlugin;
impl Plugin for ImageCopyPlugin {
    fn build(&self, app: &mut App) {
        let (s, r) = crossbeam_channel::unbounded();
        let render_app = app.insert_resource(MainWorldReceiver(r)).sub_app_mut(RenderApp);
        let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();
        graph.add_node(ImageCopyLabel, ImageCopyDriver);
        graph.add_node_edge(bevy::render::graph::CameraDriverLabel, ImageCopyLabel);
        render_app
            .insert_resource(RenderWorldSender(s))
            .add_systems(ExtractSchedule, image_copy_extract)
            .add_systems(Render, receive_image_from_buffer.after(RenderSystems::Render));
    }
}

#[derive(Clone, Default, Resource, Deref, DerefMut)]
struct ImageCopiers(Vec<ImageCopier>);

#[derive(Clone, Component)]
struct ImageCopier {
    buffer: Buffer,
    enabled: Arc<AtomicBool>,
    src_image: Handle<Image>,
}

impl ImageCopier {
    fn new(src_image: Handle<Image>, size: Extent3d, render_device: &RenderDevice) -> ImageCopier {
        let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(size.width as usize) * 4;
        let buffer = render_device.create_buffer(&BufferDescriptor {
            label: None,
            size: padded_bytes_per_row as u64 * size.height as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ImageCopier {
            buffer,
            src_image,
            enabled: Arc::new(AtomicBool::new(true)),
        }
    }
    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

fn image_copy_extract(mut commands: Commands, image_copiers: Extract<Query<&ImageCopier>>) {
    commands.insert_resource(ImageCopiers(image_copiers.iter().cloned().collect()));
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, RenderLabel)]
struct ImageCopyLabel;

#[derive(Default)]
struct ImageCopyDriver;

impl render_graph::Node for ImageCopyDriver {
    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let image_copiers = world.get_resource::<ImageCopiers>().unwrap();
        let gpu_images = world
            .get_resource::<RenderAssets<bevy::render::texture::GpuImage>>()
            .unwrap();
        for image_copier in image_copiers.iter() {
            if !image_copier.enabled() {
                continue;
            }
            let Some(src_image) = gpu_images.get(&image_copier.src_image) else {
                continue;
            };
            let mut encoder = render_context
                .render_device()
                .create_command_encoder(&CommandEncoderDescriptor::default());
            let block_dimensions = src_image.texture_format.block_dimensions();
            let block_size = src_image.texture_format.block_copy_size(None).unwrap();
            let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(
                (src_image.size.width as usize / block_dimensions.0 as usize) * block_size as usize,
            );
            encoder.copy_texture_to_buffer(
                src_image.texture.as_image_copy(),
                TexelCopyBufferInfo {
                    buffer: &image_copier.buffer,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(
                            std::num::NonZero::<u32>::new(padded_bytes_per_row as u32)
                                .unwrap()
                                .into(),
                        ),
                        rows_per_image: None,
                    },
                },
                src_image.size,
            );
            let render_queue = world.get_resource::<RenderQueue>().unwrap();
            render_queue.submit(std::iter::once(encoder.finish()));
        }
        Ok(())
    }
}

fn receive_image_from_buffer(
    image_copiers: Res<ImageCopiers>,
    render_device: Res<RenderDevice>,
    sender: Res<RenderWorldSender>,
) {
    for image_copier in image_copiers.0.iter() {
        if !image_copier.enabled() {
            continue;
        }
        let buffer_slice = image_copier.buffer.slice(..);
        let (s, r) = crossbeam_channel::bounded(1);
        buffer_slice.map_async(MapMode::Read, move |r| match r {
            Ok(r) => s.send(r).expect("send map update"),
            Err(err) => panic!("failed to map buffer {err}"),
        });
        render_device
            .poll(PollType::wait_indefinitely())
            .expect("poll device for map async");
        r.recv().expect("receive the map_async message");
        let _ = sender.send(buffer_slice.get_mapped_range().to_vec());
        image_copier.buffer.unmap();
    }
}
