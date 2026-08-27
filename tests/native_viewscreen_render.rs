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
//! browser, sampled the same way: over the **middle 40% of the frame**, the
//! scene area is **not one flat colour** and **something in it is lit**. That
//! spec exists because a render-graph break need not log anything — the PRD
//! #1023 HDR regression turned the canvas black with a completely clean console,
//! because Bevy's view-target cache is keyed by `(target, usages, hdr, msaa)`
//! and a mismatched `Hdr` between the composite camera pair silently replaces
//! the finished 3-D image with an empty one. A native host is worse off, not
//! better: it has no console for a human to notice is clean.
//!
//! **The crop is what makes this a claim about the 3-D scene.** Both cameras are
//! retargeted at the offscreen image (see [`attach_offscreen_target`] for why
//! they must be), and the 2-D UI camera is deliberately kept active throughout
//! `InProgress` so the FPS counter and radar widgets keep drawing
//! (`server::renderer`). Counted over the whole buffer, antialiased HUD text and
//! the viewscreen border alone produce far more distinct colours than any
//! threshold worth setting — so a live HUD over a completely dead 3-D scene
//! would sail through. Sampling only the middle 40% puts the measurement inside
//! the viewscreen border and away from the chrome, exactly as the browser spec's
//! screenshot `clip` does.
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

/// The fraction of the frame sampled, from the centre out — the middle 40%,
/// matching `tests/smoke/viewscreen.render.spec.js`'s screenshot `clip`.
const CROP: f32 = 0.4;

/// What the browser spec measures, over the same region: how many distinct RGBA
/// values the SCENE AREA carries (capped, so a busy frame stops counting early)
/// and the brightest channel anywhere in it.
///
/// The pair is the assertion, not either half. `distinct > 1` catches the wiped
/// buffer; `max_channel > 16` catches the case that would otherwise pass it —
/// two shades of black, which is a broken composite rather than a drawn scene.
#[derive(Debug, Default, Clone, Copy)]
struct SceneStats {
    distinct_colours: usize,
    max_channel: u8,
}

/// Sample the middle [`CROP`] of a `width`×`height` RGBA buffer.
fn scene_stats(pixels: &[u8], width: u32, height: u32, cap: usize) -> SceneStats {
    let margin = (1.0 - CROP) / 2.0;
    let x0 = (width as f32 * margin) as u32;
    let x1 = (width as f32 * (margin + CROP)) as u32;
    let y0 = (height as f32 * margin) as u32;
    let y1 = (height as f32 * (margin + CROP)) as u32;

    let mut seen: std::collections::HashSet<[u8; 4]> = std::collections::HashSet::new();
    let mut max_channel = 0u8;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * width + x) * 4) as usize;
            let Some(px) = pixels.get(i..i + 4) else {
                continue;
            };
            max_channel = max_channel.max(px[0]).max(px[1]).max(px[2]);
            if seen.len() < cap {
                seen.insert([px[0], px[1], px[2], px[3]]);
            }
        }
    }
    SceneStats {
        distinct_colours: seen.len(),
        max_channel,
    }
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

    let mut best = SceneStats::default();
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
        let stats = scene_stats(&pixels, WIDTH, HEIGHT, 512);
        best.distinct_colours = best.distinct_colours.max(stats.distinct_colours);
        best.max_channel = best.max_channel.max(stats.max_channel);
        // The browser spec's pair, verbatim: more than one colour in the scene
        // area, and something in it actually lit. A native host streams its
        // skybox, hulls and LOD levels off disk through the ordinary
        // `AssetServer`, so early mission frames legitimately fail both.
        if stats.distinct_colours > 1 && stats.max_channel > 16 {
            println!(
                "native viewscreen: {} distinct colours, max channel {} in the \
                 middle {}% at frame {frame} ({frames_with_pixels} frames read back)",
                stats.distinct_colours,
                stats.max_channel,
                (CROP * 100.0) as u32,
            );
            return;
        }
    }

    panic!(
        "the native viewscreen never drew a scene: the best the middle {}% of any \
         frame managed was {} distinct colour(s) with a brightest channel of {} \
         over {MAX_FRAMES} frames ({frames_with_pixels} of which read pixels back). \
         A flat — or uniformly black — scene area under a live HUD, with a clean \
         log, is exactly the shape of the PRD #1023 HDR regression: check that the \
         Camera3d/Camera2d pair share one `Hdr` answer \
         (render_setup::apply_target_hdr).",
        (CROP * 100.0) as u32,
        best.distinct_colours,
        best.max_channel,
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
        commands
            .entity(camera)
            .insert(RenderTarget::from(target.clone()));
    }
    commands.insert_resource(OffscreenAttached);
}
