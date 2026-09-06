//! A pane frame reaches a **persistent** GPU texture as a sub-rectangle
//! (issue #1404, slice 1).
//!
//! # Why this test is `#[ignore]`d
//!
//! Same reason as `tests/native_viewscreen_render.rs`: it needs a real GPU
//! adapter, and every CI job here is `ubuntu-latest` with no display. Run it
//! deliberately, on a machine with a GPU:
//!
//! ```text
//! cargo test --features capture --test native_pane_upload_gpu -- --ignored --nocapture
//! ```
//!
//! # What it proves, and why the unit tests do not
//!
//! `panes::upload`'s unit tests check the *arithmetic* — the offset, the stride,
//! and every layout wgpu would refuse. They cannot check the claim the slice is
//! actually about, which is that a `write_texture` of a dirty rectangle into a
//! `RenderAssetUsages::RENDER_WORLD` image **lands where it is aimed and leaves
//! the rest of the texture alone**. That is a statement about wgpu's strided
//! copy out of a full-size staging buffer, and only a device can answer it.
//!
//! So: a 64×48 `Bgra8UnormSrgb` image filled black, read back through the same
//! [`ImageCopyPlugin`] the capture tools use. Two uploads at two different
//! rectangles, in two different colours, several frames apart. Both rectangles
//! must be present **at the same time** in the final readback — which is the
//! persistence claim, because a texture that were re-created per frame would
//! carry only the newer one — and every pixel outside them must still be the
//! original fill, which is the "only the dirty rectangle was written" claim.
//!
//! No Ultralight SDK is needed: the producer is synthesised. What is *not*
//! covered here is the producer's own bookkeeping (`drive_panes`, the staging
//! pool, the resize epoch), which is `tests/native_host_pane_ultralight.rs`'s
//! and the rig's.

#![cfg(all(feature = "capture", not(target_arch = "wasm32")))]

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::renderer::RenderDevice;

use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::native_host::panes::upload::{
    PaneFrameBuffer, PanePendingUploads, PaneUpload,
};
use project_phoenix::native_host::panes::PaneId;
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig,
};
use project_phoenix::render_capture::{
    unpad_rows, ImageCopier, ImageCopyPlugin, MainWorldReceiver,
};
use vellum_ultralight::surface::DirtyRect;

const WORLD: &str = "assets/worlds/combat_test.toml";
const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
/// Frames given to each upload to be extracted, prepared, written and read back.
const SETTLE_FRAMES: usize = 10;

/// The pane texture's fill, in the `Bgra8UnormSrgb` byte order it is stored in.
const BLACK: [u8; 4] = [0, 0, 0, 255];
/// B, G, R, A — magenta.
const MAGENTA: [u8; 4] = [255, 0, 255, 255];
/// B, G, R, A — cyan.
const CYAN: [u8; 4] = [255, 255, 0, 255];

/// The pane the synthetic frames claim to come from.
const PANE: PaneId = PaneId(1);

fn rect(left: u32, top: u32, right: u32, bottom: u32) -> DirtyRect {
    DirtyRect {
        left,
        top,
        right,
        bottom,
    }
}

/// A full-size staging buffer, black everywhere except `area`, which is
/// `colour` — exactly the shape a pane's copy produces: only the dirty
/// rectangle is current.
fn staged(area: DirtyRect, colour: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for _ in 0..WIDTH * HEIGHT {
        bytes.extend_from_slice(&BLACK);
    }
    for y in area.top..area.bottom {
        for x in area.left..area.right {
            let i = ((y * WIDTH + x) * 4) as usize;
            bytes[i..i + 4].copy_from_slice(&colour);
        }
    }
    bytes
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * WIDTH + x) * 4) as usize;
    let mut out = [0u8; 4];
    out.copy_from_slice(&pixels[i..i + 4]);
    out
}

fn contains(area: DirtyRect, x: u32, y: u32) -> bool {
    x >= area.left && x < area.right && y >= area.top && y < area.bottom
}

/// Pump `frames` app updates, returning the freshest readback seen.
fn pump(app: &mut App, frames: usize) -> Option<Vec<u8>> {
    let mut latest = None;
    for _ in 0..frames {
        app.update();
        if let Some(receiver) = app.world().get_resource::<MainWorldReceiver>() {
            if let Some(padded) = receiver.try_iter().last() {
                latest = Some(unpad_rows(&padded, WIDTH, HEIGHT));
            }
        }
    }
    latest
}

#[test]
#[ignore = "needs a real GPU adapter; CI has no Windows runner and no display. \
            Run: cargo test --features capture --test native_pane_upload_gpu -- --ignored --nocapture"]
fn two_pane_frames_write_their_own_rectangles_into_one_persistent_texture() {
    let preload = preload_content_templates(".").expect("the repository's own content preloads");
    let mut cfg = NativeHostConfig::new(WORLD);
    cfg.solo = true;
    cfg.surface = NativeRenderSurface::Offscreen;
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");
    app.add_plugins(ImageCopyPlugin);
    app.finish();
    app.cleanup();
    // One frame so the render device and the render sub-app exist.
    app.update();

    let extent = Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    // A pane texture, minted exactly as `panes::ultralight` mints one: opaque
    // BGRA, black, RENDER_WORLD only. `Image::new_fill`'s default usages carry
    // COPY_DST (which is what `write_texture` needs) and COPY_SRC (which is what
    // the readback needs).
    let image = Image::new_fill(
        extent,
        TextureDimension::D2,
        &BLACK,
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let copier = {
        let device = app.world().resource::<RenderDevice>().clone();
        ImageCopier::new(handle.clone(), extent, &device)
    };
    app.world_mut().spawn(copier);

    let first = rect(16, 8, 48, 24);
    let second = rect(4, 32, 20, 44);

    let push = |app: &mut App, area: DirtyRect, colour: [u8; 4]| {
        let bytes = PaneFrameBuffer::new(PANE, staged(area, colour), None);
        app.world_mut()
            .resource_mut::<PanePendingUploads>()
            .uploads
            .push(PaneUpload {
                image: handle.id(),
                epoch: 0,
                rect: area,
                surface: (WIDTH, HEIGHT),
                full: false,
                bytes,
                attempts: 0,
            });
    };

    push(&mut app, first, MAGENTA);
    let after_first = pump(&mut app, SETTLE_FRAMES).expect("the texture reads back");
    assert_eq!(
        pixel(&after_first, 20, 12),
        MAGENTA,
        "the first frame's rectangle should be on the texture"
    );

    push(&mut app, second, CYAN);
    let after_second = pump(&mut app, SETTLE_FRAMES).expect("the texture reads back again");

    // The persistence claim: the FIRST rectangle is still there after the
    // second upload, which a per-frame texture re-creation could not manage.
    for (x, y) in [(16, 8), (20, 12), (47, 23)] {
        assert_eq!(
            pixel(&after_second, x, y),
            MAGENTA,
            "({x},{y}) is in the first rectangle and should have survived the second upload"
        );
    }
    for (x, y) in [(4, 32), (10, 38), (19, 43)] {
        assert_eq!(
            pixel(&after_second, x, y),
            CYAN,
            "({x},{y}) is in the second rectangle"
        );
    }
    // …and nothing else was touched, which is the sub-rectangle claim.
    let mut outside = 0usize;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if contains(first, x, y) || contains(second, x, y) {
                continue;
            }
            assert_eq!(
                pixel(&after_second, x, y),
                BLACK,
                "({x},{y}) is outside both rectangles and should still be the fill"
            );
            outside += 1;
        }
    }
    println!(
        "pane upload: two rectangles present, {outside} pixels outside them still the original fill"
    );
}
