//! Shared native headless-render core for the offscreen capture tools.
//!
//! Both `capture-billboard` (bake a far-LOD atlas) and `tune-lods` (measure how
//! different two LOD levels look on screen) need the same thing: render a model
//! into a transparent RGBA target with no window, copy that texture back to a
//! CPU buffer, and read the pixels. That plumbing — lifted from Bevy's
//! `headless_renderer` example — lives here so the two tools share one copy of
//! it rather than diverging.
//!
//! What this module owns:
//!   * [`ImageCopyPlugin`] and its render-graph node, which copy the render
//!     target's texture into a mappable buffer every frame and ship the bytes to
//!     the main world over a channel ([`MainWorldReceiver`]).
//!   * [`create_render_target`] — a transparent RGBA render target of any
//!     width/height, plus the matching [`ImageCopier`] to read it back.
//!   * [`unpad_rows`] — strip the GPU's 256-byte row alignment back to tight
//!     `width * 4` rows (generalised to non-square targets).
//!   * [`measure_world_bounds`] / [`frame_distance`] / [`orbit_transform`] —
//!     the framing maths: union the live mesh AABBs, fit them in the game FOV,
//!     and orbit a camera around them.
//!
//! What it deliberately does NOT own: the per-tool state machine (how many
//! views, how long to settle, what to do with the pixels). That stays in each
//! `bin`, because the two tools capture for entirely different reasons.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use bevy::{
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
};
use crossbeam_channel::{Receiver, Sender};

// ── Render target ─────────────────────────────────────────────────────────

/// Create a transparent RGBA8 render target of `width`×`height`, and the
/// [`ImageCopier`] that reads it back each frame. Spawn the copier as an entity
/// and attach the returned handle to a camera's `RenderTarget`.
pub fn create_render_target(
    images: &mut Assets<Image>,
    render_device: &RenderDevice,
    width: u32,
    height: u32,
) -> (Handle<Image>, ImageCopier) {
    let extent = Extent3d {
        width: width.max(1),
        height: height.max(1),
        depth_or_array_layers: 1,
    };
    let mut target = Image::new_fill(
        extent,
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    target.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let handle = images.add(target);
    let copier = ImageCopier::new(handle.clone(), extent, render_device);
    (handle, copier)
}

/// Strip the GPU row padding (buffers align each row to 256 bytes) back to a
/// tight `width * 4` bytes per row, keeping exactly `height` rows.
///
/// Generalised over both dimensions (the billboard tool's square tiles were a
/// special case): the readback buffer pads *rows*, so the height only bounds how
/// many rows to take, while the width sets both the aligned stride and the tight
/// row length.
pub fn unpad_rows(padded: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row = (width * 4) as usize;
    let aligned = RenderDevice::align_copy_bytes_per_row(row);
    if aligned == row {
        return padded.to_vec();
    }
    padded
        .chunks(aligned)
        .take(height as usize)
        .flat_map(|r| &r[..row.min(r.len())])
        .copied()
        .collect()
}

// ── Framing ─────────────────────────────────────────────────────────────────

/// The world-space centre, framing radius (half the largest dimension) and full
/// size of the union of every measurable mesh AABB, or `None` when no geometry
/// is measurable yet (the caller waits and retries).
///
/// The live AABBs are measured rather than the sidecar's cached `[extents]`,
/// which do not reliably match the rendered geometry — the model viewer measures
/// the live bounds too.
pub fn measure_world_bounds<'a>(
    bounds: impl Iterator<Item = (&'a GlobalTransform, &'a bevy::camera::primitives::Aabb)>,
) -> Option<(Vec3, f32, Vec3)> {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut any = false;
    for (gt, aabb) in bounds {
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
        return None;
    }
    let size = hi - lo;
    let center = (lo + hi) * 0.5;
    let radius = size.max_element().max(0.05) * 0.5;
    Some((center, radius, size))
}

/// Camera distance that fits a sphere of `radius` in a vertical field of view of
/// `fov` radians, with `margin` slack so the subject never touches the edge.
pub fn frame_distance(radius: f32, fov: f32, margin: f32) -> f32 {
    radius / (fov * 0.5).tan() * margin
}

/// A camera transform orbited to `yaw`/`pitch` (radians) around `center` at
/// `distance`, looking back at the centre.
pub fn orbit_transform(center: Vec3, distance: f32, yaw: f32, pitch: f32) -> Transform {
    let rot = Quat::from_euler(EulerRot::YXZ, yaw, -pitch, 0.0);
    let pos = center + rot * Vec3::new(0.0, 0.0, distance);
    Transform::from_translation(pos).looking_at(center, Vec3::Y)
}

// ── Render-graph image copy (adapted from Bevy's headless_renderer example) ──

/// Main-world end of the readback channel: the freshest render target bytes,
/// still row-padded — call [`unpad_rows`] before use.
#[derive(Resource, Deref)]
pub struct MainWorldReceiver(pub Receiver<Vec<u8>>);

#[derive(Resource, Deref)]
struct RenderWorldSender(Sender<Vec<u8>>);

/// Copies every enabled [`ImageCopier`]'s render target into its mappable buffer
/// each frame and ships the mapped bytes to [`MainWorldReceiver`].
pub struct ImageCopyPlugin;

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

/// One offscreen render target and the CPU-mappable buffer that reads it back.
#[derive(Clone, Component)]
pub struct ImageCopier {
    buffer: Buffer,
    enabled: Arc<AtomicBool>,
    src_image: Handle<Image>,
}

impl ImageCopier {
    fn new(src_image: Handle<Image>, size: Extent3d, render_device: &RenderDevice) -> ImageCopier {
        // The row stride MUST match what `ImageCopyDriver` writes:
        // `align_copy_bytes_per_row(width * 4)`. Aligning `width` and then
        // multiplying by 4 (as an earlier version did) only coincides with that
        // for widths where `width * 4` is already aligned — e.g. the billboard
        // tool's 256px square — and over-allocates a mismatched stride for a
        // 1920-wide game-resolution target, leaving `unpad_rows` a garbage tail.
        let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(size.width as usize * 4);
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
