//! Billboard capture: render the loaded subject to a transparent yaw-ring atlas.
//!
//! The far level of a model's LOD ladder is a billboard (see
//! [`crate::entities::billboard`]) — a camera-facing quad textured from a strip
//! of pre-rendered views of the hull. This module bakes that strip from the
//! model the viewer is already showing, so the billboard matches how the game
//! actually renders the ship (same materials, same shaders, same lighting) at a
//! fraction of the triangles.
//!
//! # How it runs (a small per-frame state machine)
//!
//! GPU→CPU readback is asynchronous, so one view is captured at a time across a
//! few frames ([`drive_capture`]):
//!   1. **Position** the offscreen camera at the next yaw around the subject.
//!   2. **WaitRender** one frame so that camera renders the image target.
//!   3. **WaitShot** — a [`Screenshot`] of the target; its observer stores the
//!      tile when the readback lands.
//! After the last view the tiles are packed left→right into one RGBA atlas and
//! parked in [`CaptureState::result`], which the JS panel polls, PNG-encodes on
//! a canvas, and POSTs to `dev-viewer.mjs` (`/api/lod/capture`).
//!
//! The offscreen camera carries NO skybox and clears to a fully transparent
//! colour, so the atlas has a clean alpha cutout of the hull. It reuses the
//! game's projection, and the scene's own lights light the subject, so a tile is
//! the ship as the game would show it, seen from that heading.

use std::cell::RefCell;
use std::f32::consts::TAU;

use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::image::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::viewer::subject::SubjectState;

/// A pending capture request, set by the panel and consumed by [`start_capture`].
#[derive(Resource, Default)]
pub struct CaptureRequest(pub Option<CaptureParams>);

#[derive(Clone, Copy, Debug)]
pub struct CaptureParams {
    pub views: u32,
    pub resolution: u32,
    /// Camera pitch above the ring plane, in degrees.
    pub pitch_deg: f32,
}

/// The finished atlas, polled out by the JS bridge.
pub struct CaptureResult {
    /// Row-major RGBA8 of the whole `views`-wide strip.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Model this was captured from (for the sidecar patch).
    pub source: String,
    pub views: u32,
    pub resolution: u32,
    pub pitch_deg: f32,
    /// World width/height of the hull, for the billboard level's `scale`.
    pub world_size: [f32; 2],
}

#[derive(PartialEq, Clone, Copy)]
enum Step {
    Position,
    WaitRender,
    WaitShot,
}

struct CaptureJob {
    params: CaptureParams,
    source: String,
    image: Handle<Image>,
    camera: Entity,
    center: Vec3,
    distance: f32,
    world_size: [f32; 2],
    next: u32,
    step: Step,
    wait: u32,
    captured: bool,
    tiles: Vec<Vec<u8>>,
}

#[derive(Resource, Default)]
pub struct CaptureState {
    job: Option<CaptureJob>,
    pub result: Option<CaptureResult>,
}

/// Marks the throwaway offscreen camera a capture spawns.
#[derive(Component)]
pub(crate) struct CaptureCamera;

/// Begin a capture when one is requested and the subject has known extents.
pub(crate) fn start_capture(
    mut commands: Commands,
    mut request: ResMut<CaptureRequest>,
    mut state: ResMut<CaptureState>,
    mut images: ResMut<Assets<Image>>,
    subject: Res<SubjectState>,
    args: Res<crate::viewer::ViewerArgs>,
) {
    if state.job.is_some() {
        return; // one at a time
    }
    let Some(params) = request.0 else {
        return;
    };
    let Some(extents) = subject.extents else {
        return; // subject not framed yet — try again next frame
    };
    request.0 = None;

    let res = params.resolution.max(1);
    let views = params.views.max(1);

    // Transparent RGBA target the offscreen camera renders into.
    let size = Extent3d {
        width: res,
        height: res,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING
        | TextureUsages::COPY_SRC
        | TextureUsages::RENDER_ATTACHMENT;
    let image = images.add(image);

    // Framing: subject sits at the origin (see `frame_subject_once`). Distance
    // from the largest extent and the game FOV, with the same margin the orbit
    // camera uses, so a tile frames the hull the way the viewer does.
    let largest = extents.max_element().max(0.1);
    let fov = std::f32::consts::FRAC_PI_4;
    let distance = (largest * 0.5) / (fov * 0.5).tan() * 1.8;

    let camera = commands
        .spawn((
            CaptureCamera,
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::NONE),
                // Render before the main window camera; different target, so the
                // order only needs to be deterministic.
                order: -10,
                ..default()
            },
            // In Bevy 0.18 the render target is its own component (Camera
            // `#[require(RenderTarget)]`), not a `Camera` field.
            RenderTarget::from(image.clone()),
            crate::render_setup::game_camera_projection(),
            Transform::default(),
        ))
        .id();

    // Billboard world size: the hull's silhouette box — widest horizontal axis
    // by its height. The quad faces the camera, so this covers it from any yaw.
    let world_size = [extents.x.max(extents.z), extents.y];

    bevy::log::info!(
        "capture: start views={views} res={res} dist={distance:.1} extents={extents:?}"
    );
    state.job = Some(CaptureJob {
        params: CaptureParams {
            views,
            resolution: res,
            pitch_deg: params.pitch_deg,
        },
        source: args.model.clone().unwrap_or_default(),
        image,
        camera,
        center: Vec3::ZERO,
        distance,
        world_size,
        next: 0,
        step: Step::Position,
        wait: 0,
        captured: false,
        tiles: Vec::with_capacity(views as usize),
    });
}

/// Advance the capture state machine one frame.
pub(crate) fn drive_capture(
    mut commands: Commands,
    mut state: ResMut<CaptureState>,
    mut cameras: Query<&mut Transform, With<CaptureCamera>>,
) {
    let Some(job) = state.job.as_mut() else {
        return;
    };

    match job.step {
        Step::Position => {
            let yaw = job.next as f32 * TAU / job.params.views as f32;
            let pitch = job.params.pitch_deg.to_radians();
            // Camera orbits the subject: yaw around +Y, then pitch up.
            let rot = Quat::from_euler(EulerRot::YXZ, yaw, -pitch, 0.0);
            let pos = job.center + rot * Vec3::new(0.0, 0.0, job.distance);
            if let Ok(mut tf) = cameras.get_mut(job.camera) {
                *tf = Transform::from_translation(pos).looking_at(job.center, Vec3::Y);
            }
            job.step = Step::WaitRender;
            job.wait = 1;
        }
        Step::WaitRender => {
            if job.wait > 0 {
                job.wait -= 1;
                return;
            }
            job.captured = false;
            commands.spawn(Screenshot::image(job.image.clone())).observe(
                |trigger: On<ScreenshotCaptured>, mut state: ResMut<CaptureState>| {
                    if let Some(job) = state.job.as_mut() {
                        if let Some(data) = trigger.image.data.clone() {
                            job.tiles.push(data);
                        }
                        job.captured = true;
                    }
                },
            );
            job.step = Step::WaitShot;
        }
        Step::WaitShot => {
            if !job.captured {
                return;
            }
            job.next += 1;
            if job.next < job.params.views {
                job.step = Step::Position;
            } else {
                finish_capture(&mut commands, &mut state);
            }
        }
    }
}

/// Pack the captured tiles into one strip and park the result.
fn finish_capture(commands: &mut Commands, state: &mut CaptureState) {
    let Some(job) = state.job.take() else {
        return;
    };
    commands.entity(job.camera).try_despawn();

    let res = job.params.resolution;
    let views = job.params.views;
    let row = (res * 4) as usize;
    let atlas_w = res * views;
    let mut atlas = vec![0u8; (atlas_w * res * 4) as usize];
    let atlas_row = (atlas_w * 4) as usize;

    for (t, tile) in job.tiles.iter().enumerate() {
        if tile.len() < (row * res as usize) {
            continue; // a tile that didn't read back cleanly — leave it blank
        }
        let x_off = t as usize * row;
        for y in 0..res as usize {
            let src = y * row;
            let dst = y * atlas_row + x_off;
            atlas[dst..dst + row].copy_from_slice(&tile[src..src + row]);
        }
    }

    state.result = Some(CaptureResult {
        rgba: atlas,
        width: atlas_w,
        height: res,
        source: job.source,
        views,
        resolution: res,
        pitch_deg: job.params.pitch_deg,
        world_size: job.world_size,
    });
}

// ── Output bridge: a finished atlas, parked for the JS panel to poll ─────────
//
// The `#[wasm_bindgen]` getters in `mod.rs` cannot touch the ECS, so — exactly
// as `stats` does with its JSON — `publish_capture` copies a finished result
// into a thread-local the getters read. The panel polls `capture_ready`, pulls
// the bytes + meta, PNG-encodes on a canvas and POSTs, then calls `capture_clear`.

thread_local! {
    static CAPTURE_RGBA: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    static CAPTURE_META: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Move a finished result out of the ECS and into the poll-able thread-local.
pub(crate) fn publish_capture(mut state: ResMut<CaptureState>) {
    let Some(result) = state.result.take() else {
        return;
    };
    // Hand-built JSON (serde_json is confined to codec.rs, Key Constraint 1),
    // the same way `stats` publishes. `{:?}` on the path emits a quoted,
    // escaped JSON string.
    let meta = format!(
        "{{\"width\":{},\"height\":{},\"source\":{:?},\"views\":{},\"resolution\":{},\"pitch\":{},\"world_w\":{},\"world_h\":{}}}",
        result.width,
        result.height,
        result.source,
        result.views,
        result.resolution,
        result.pitch_deg,
        result.world_size[0],
        result.world_size[1],
    );
    CAPTURE_RGBA.with(|c| *c.borrow_mut() = Some(result.rgba));
    CAPTURE_META.with(|c| *c.borrow_mut() = meta);
}

/// True once a baked atlas is waiting to be read.
pub fn capture_ready() -> bool {
    CAPTURE_META.with(|c| !c.borrow().is_empty())
}

/// The finished atlas metadata as JSON.
pub fn capture_meta() -> String {
    CAPTURE_META.with(|c| c.borrow().clone())
}

/// Take the finished atlas RGBA bytes (row-major, `width`×`height`×4).
pub fn capture_take_rgba() -> Vec<u8> {
    CAPTURE_RGBA.with(|c| c.borrow_mut().take().unwrap_or_default())
}

/// Clear the parked result after the panel has consumed it.
pub fn capture_clear() {
    CAPTURE_RGBA.with(|c| *c.borrow_mut() = None);
    CAPTURE_META.with(|c| c.borrow_mut().clear());
}
