//! The native viewscreen actually draws (issue #1121, acceptance criterion 2).
//!
//! # Why this test is `#[ignore]`d
//!
//! It needs a real GPU adapter, and this repository has nowhere to run one.
//! Every job in `.github/workflows/ci.yml` is `ubuntu-latest`; the only
//! `windows-latest` runner in the repo is `deploy-demo.yml`'s
//! `package-native-demo`, which builds a release and packages it without
//! running anything. The browser's equivalent proof
//! (`tests/smoke/viewscreen.render.spec.js`) gets around this with SwiftShader
//! under Playwright; there is no such lane for a native binary here.
//!
//! So it is written to be run **deliberately, on a machine with a GPU**:
//!
//! ```text
//! cargo test --features capture --test native_viewscreen_render -- --ignored --nocapture
//! ```
//!
//! When issue #1121's sibling packaging work adds a Windows CI runner, drop the
//! `#[ignore]` and it runs there unchanged.
//!
//! # What it asserts, and why that is the right assertion
//!
//! The same claim `tests/smoke/viewscreen.render.spec.js` makes about the
//! browser: the viewscreen is **not one flat colour**. That spec exists because
//! a render-graph break need not log anything — the PRD #1023 HDR regression
//! turned the canvas black with a completely clean console, because Bevy's
//! view-target cache is keyed by `(target, usages, hdr, msaa)` and a mismatched
//! `Hdr` between the composite camera pair silently replaces the finished 3-D
//! image with an empty one. A native host is worse off, not better: it has no
//! console for a human to notice is clean.
//!
//! It renders **offscreen** rather than into a window
//! ([`NativeRenderSurface::Offscreen`]) for the reason `capture-billboard` and
//! `tune-lods` already do: it is the same wgpu device, the same render graph and
//! the same `RendererPlugin`, minus the one piece — a winit surface — that
//! cannot be automated. A window adds a compositor, not a renderer.

#![cfg(all(feature = "capture", not(target_arch = "wasm32")))]

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;

use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig,
};
use project_phoenix::render_capture::{
    create_render_target, unpad_rows, ImageCopyPlugin, MainWorldReceiver,
};

const WORLD: &str = "assets/worlds/combat_test.toml";
const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
/// How many frames to give the host before giving up. Generous: a native host
/// streams its skybox, hulls and LOD levels off disk through the ordinary
/// `AssetServer`, and the first frames are legitimately empty.
const MAX_FRAMES: usize = 900;

/// Marks that the offscreen target has been attached, so it happens once.
#[derive(Resource)]
struct OffscreenAttached;

/// How many distinct RGBA values a frame carries, capped at `cap` so a busy
/// frame stops counting early.
fn distinct_colours(pixels: &[u8], cap: usize) -> usize {
    let mut seen: std::collections::HashSet<[u8; 4]> = std::collections::HashSet::new();
    for chunk in pixels.chunks_exact(4) {
        seen.insert([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if seen.len() >= cap {
            break;
        }
    }
    seen.len()
}

#[test]
#[ignore = "needs a real GPU adapter; CI has no Windows runner and no display. \
            Run: cargo test --features capture --test native_viewscreen_render -- --ignored --nocapture"]
fn the_native_viewscreen_draws_a_frame_that_is_not_one_flat_colour() {
    let preload = preload_content_templates(".").expect("the repository's own content preloads");
    let mut cfg = NativeHostConfig::new(WORLD);
    cfg.solo = true;
    cfg.surface = NativeRenderSurface::Offscreen;
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");

    // The readback plumbing `capture-billboard` and `tune-lods` share.
    app.add_plugins(ImageCopyPlugin)
        .add_systems(PostStartup, attach_offscreen_target);

    app.finish();
    app.cleanup();

    let mut best = 0usize;
    let mut frames_with_pixels = 0usize;
    for frame in 0..MAX_FRAMES {
        app.update();
        // Only judge frames from the MISSION. Before `GamePhase::InProgress`
        // the 3-D `GameCamera` is `is_active: false` and only the lobby camera
        // draws, so an early frame could pass this test on UI chrome alone —
        // which is not what "the shared viewscreen renders through native
        // Bevy/wgpu" claims.
        if app
            .world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get()
            != &project_phoenix::core::messages::GamePhase::InProgress
        {
            continue;
        }
        let Some(receiver) = app.world().get_resource::<MainWorldReceiver>() else {
            continue;
        };
        let Some(padded) = receiver.try_iter().last() else {
            continue;
        };
        frames_with_pixels += 1;
        let pixels = unpad_rows(&padded, WIDTH, HEIGHT);
        let colours = distinct_colours(&pixels, 64);
        best = best.max(colours);
        // Two distinct colours is already a drawn image rather than a cleared
        // buffer; wait for a few more so the assertion is about a scene rather
        // than about one stray pixel.
        if colours >= 8 {
            println!(
                "native viewscreen: {colours} distinct colours at frame {frame} \
                 ({frames_with_pixels} frames read back)"
            );
            return;
        }
    }

    panic!(
        "the native viewscreen never drew anything but a flat colour: best was \
         {best} distinct colour(s) over {MAX_FRAMES} frames ({frames_with_pixels} \
         of which read pixels back). A completely flat frame with a clean log is \
         exactly the shape of the PRD #1023 HDR regression — check that the \
         Camera3d/Camera2d pair share one `Hdr` answer (render_setup::apply_target_hdr)."
    );
}

/// Point the viewscreen's composite camera pair at an offscreen image.
///
/// `PostStartup`, so `RendererPlugin::setup`'s `Startup` spawn has already
/// applied. **Both** cameras are retargeted, not just the 3-D one: they are a
/// composite pair (`Camera3d` at `order: -1` over `Core3d`, `Camera2d` at
/// `order: 0` with `ClearColorConfig::None` over `Core2d`), and pointing them
/// at different targets is the same class of mistake as giving them different
/// `Hdr` — the UI camera's upscaling blit would replace the finished 3-D image
/// with an empty one.
fn attach_offscreen_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
    cameras: Query<Entity, With<Camera>>,
    attached: Option<Res<OffscreenAttached>>,
) {
    if attached.is_some() {
        return;
    }
    let (target, copier) = create_render_target(&mut images, &render_device, WIDTH, HEIGHT);
    commands.spawn(copier);
    for camera in cameras.iter() {
        commands.entity(camera).insert(RenderTarget::from(target.clone()));
    }
    commands.insert_resource(OffscreenAttached);
}
