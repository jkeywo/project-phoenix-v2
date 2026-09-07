//! The pane input adapter builds a real router over real window geometry and
//! runs its whole input + draw pipeline on this machine (issue #1124, the
//! Windows integration half).
//!
//! # Why this test is `#[ignore]`d
//!
//! It needs the **Ultralight SDK** (a ~100 MB proprietary download, behind
//! `--features ultralight`), a **real winit window** and a **GPU adapter** for
//! Bevy's default render stack — none of which CI has (every job is
//! `ubuntu-latest` and headless). So it is written to be run deliberately, on the
//! dev machine:
//!
//! ```text
//! node scripts/build-client.mjs   # not needed here; panes load about:blank
//! cargo test --features ultralight --test native_host_input -- --ignored --nocapture
//! ```
//!
//! # What it proves, and what it cannot
//!
//! It proves the feature-gated adapter (`native_host::panes::ultralight`)
//! **constructs and runs on real hardware**: two panes open on the real primary
//! window, the pure [`PaneRouter`] is built from that window's actual geometry
//! and scale, and a synthetic coordinate resolves to the correct pane and
//! pane-local logical position through the real transform. The whole input group
//! (pointer, touch, focus, indicator) and the per-frame drive loop run for many
//! frames against a live Ultralight runtime without panicking.
//!
//! What it **cannot** prove on this machine is the multi-monitor and multi-touch
//! behaviour: the dev box has one monitor and no touch input. That is the
//! acceptance kit's job (`docs/acceptance/1124-input.md`). The routing *logic*
//! for both is covered by the pure tests in
//! `src/native_host/input_routing_tests.rs`, which run in ordinary CI.
//!
//! It also cannot prove anything about the **upload path** (issue #1404): the
//! app below is built around `PaneDisplayPlugin` alone, so nothing ever drains
//! `PanePendingUploads`. Each pane copies exactly its pool's worth of frames and
//! is `starved` from then on. That is harmless for the routing and focus
//! assertions made here — they do not look at pixels — but it does mean the
//! copy-and-upload path is exercised for only the first few frames, and is
//! covered properly by `tests/native_pane_upload_gpu.rs` and the unit tests in
//! `src/native_host/panes/upload.rs`.

#![cfg(all(feature = "ultralight", not(target_arch = "wasm32")))]

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, Window};

use project_phoenix::native_host::input_routing::WindowKey;
use project_phoenix::native_host::panes::ultralight::{
    PaneDisplayConfig, PaneDisplayEntry, PaneDisplayPlugin, PaneHost,
};
use project_phoenix::native_host::panes::{PaneBus, PaneBusResource, PaneIdentity};

/// Frames to give the pane host to open its views and run its pipeline. Generous;
/// the test exits as soon as it has checked the router.
const FRAME_BUDGET: u32 = 600;

type Verdict = Arc<Mutex<Option<Result<String, String>>>>;

#[derive(Resource, Clone)]
struct Outcome(Verdict);

#[derive(Resource, Default)]
struct Frames(u32);

#[test]
#[ignore = "needs the Ultralight SDK, a real window and a GPU. \
            Run: cargo test --features ultralight --test native_host_input -- --ignored --nocapture"]
fn the_pane_input_adapter_builds_a_router_over_real_window_geometry() {
    // The shipped binary stages the SDK's shared libraries beside the executable
    // at startup; a test binary must do the same or the Ultralight runtime cannot
    // find them. `deps/` is the test's own directory.
    match project_phoenix::native_host::panes::ultralight::stage_sdk() {
        Ok(summary) => println!("native input: {summary}"),
        Err(e) => panic!("native input: could not stage the Ultralight SDK: {e}"),
    }

    let verdict: Verdict = Arc::new(Mutex::new(None));

    // Two panes, loading about:blank so the test needs no HTTP server — it is the
    // routing wiring under test, not the console page (that is
    // `native_host_pane_ultralight.rs`).
    let bus = PaneBus::default();
    let ada = bus.open(PaneIdentity::mint("Ada".to_string()));
    let grace = bus.open(PaneIdentity::mint("Grace".to_string()));

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "phoenix-host — input routing test".to_string(),
                    ..default()
                }),
                ..default()
            })
            .set(bevy::log::LogPlugin {
                filter: "warn".to_string(),
                ..default()
            })
            // The Rust test harness runs each `#[test]` off the main thread, and
            // winit refuses its event loop there. The shipped binary's `main`
            // owns the main thread, so this is a test-only accommodation.
            .set(bevy::winit::WinitPlugin {
                run_on_any_thread: true,
            }),
    );
    app.insert_resource(PaneBusResource(bus));
    app.insert_resource(PaneDisplayConfig {
        panes: vec![
            PaneDisplayEntry {
                id: ada,
                url: "about:blank".to_string(),
                label: "Ada".to_string(),
            },
            PaneDisplayEntry {
                id: grace,
                url: "about:blank".to_string(),
                label: "Grace".to_string(),
            },
        ],
    });
    app.add_plugins(PaneDisplayPlugin);
    app.insert_resource(Outcome(verdict.clone()));
    app.init_resource::<Frames>();
    app.add_systems(Update, check);

    app.run();

    // Bind the guard's result out before matching, so the `MutexGuard` temporary
    // is dropped rather than held across the arms (`App::run` consumes the world,
    // so the verdict has to travel out through the shared handle).
    let result = verdict.lock().unwrap().take();
    match result {
        Some(Ok(summary)) => println!("native input: {summary}"),
        Some(Err(reason)) => panic!("native input: {reason}"),
        None => panic!("native input: exited without a verdict — the pane host never built"),
    }
}

/// Wait for the pane host to build, then check the router it built against the
/// real primary window.
fn check(
    host: Option<Res<PaneHost>>,
    primary: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut frames: ResMut<Frames>,
    outcome: Res<Outcome>,
    mut exit: MessageWriter<AppExit>,
) {
    frames.0 += 1;
    if frames.0 > FRAME_BUDGET {
        finish(
            &outcome,
            &mut exit,
            Err(format!(
                "timed out after {FRAME_BUDGET} frames (host built: {})",
                host.is_some()
            )),
        );
        return;
    }
    let Some(host) = host else {
        return; // pane host still initialising
    };
    let Ok((entity, window)) = primary.single() else {
        return;
    };

    assert!(
        host.thread_is_running(),
        "the pane runtime must run on its own thread"
    );
    let router = host.router();
    if router.len() != 2 {
        finish(
            &outcome,
            &mut exit,
            Err(format!(
                "router has {} placements, expected 2",
                router.len()
            )),
        );
        return;
    }

    // Two panes tile the one window left-to-right: a point in the left quarter
    // resolves to the first pane, one in the right quarter to the second — the
    // real transform over the real window scale.
    let key = WindowKey(entity.to_bits());
    let scale = window.scale_factor() as f64;
    let w = window.physical_width() as f64;
    let h = window.physical_height() as f64;
    let left = router.resolve_in_window(key, w * 0.25, h * 0.5);
    let right = router.resolve_in_window(key, w * 0.75, h * 0.5);

    let order = router.focus_order();
    let Some(left) = left else {
        finish(
            &outcome,
            &mut exit,
            Err("left-quarter point hit no pane".into()),
        );
        return;
    };
    let Some(right) = right else {
        finish(
            &outcome,
            &mut exit,
            Err("right-quarter point hit no pane".into()),
        );
        return;
    };
    if left.pane != order[0] || right.pane != order[1] {
        finish(
            &outcome,
            &mut exit,
            Err(format!(
                "left resolved to {:?} and right to {:?}, expected {:?} then {:?}",
                left.pane, right.pane, order[0], order[1]
            )),
        );
        return;
    }
    // The pane-local coordinate is the physical offset divided by the window
    // scale, so it must be a sane page pixel inside the pane.
    if left.local_x < 0 || (left.local_x as f64) > w / scale {
        finish(
            &outcome,
            &mut exit,
            Err(format!("left local x {} is outside the pane", left.local_x)),
        );
        return;
    }

    // Keyboard focus is seeded onto the first pane the moment panes exist, so a
    // pure-keyboard operator has a visible focus indicator and a defined target
    // for the first keystroke (acceptance criterion 2). It must be the first
    // pane in placement order, not left unset.
    //
    // The first *pane*: `FocusRing::focused_on_first_pane` skips the host-lobby
    // surface (issue #1325), which carries no typeable control. This host opens
    // no lobby, so the order is the two panes and `order[0]` is the first of
    // them either way — the exclusion is pinned by the pure tests in
    // `native_host::input_routing_tests`, which need no GPU to run.
    if host.focused_pane() != Some(order[0]) {
        finish(
            &outcome,
            &mut exit,
            Err(format!(
                "expected initial focus seeded onto the first pane {:?}, but focus was {:?}",
                order[0],
                host.focused_pane()
            )),
        );
        return;
    }

    finish(
        &outcome,
        &mut exit,
        Ok(format!(
            "two panes on the {}x{} (scale {scale}) primary window; a left point resolved to \
             {:?} at local ({}, {}) and a right point to {:?} — one pointer, both panes",
            window.physical_width(),
            window.physical_height(),
            left.pane,
            left.local_x,
            left.local_y,
            right.pane,
        )),
    );
}

fn finish(outcome: &Outcome, exit: &mut MessageWriter<AppExit>, verdict: Result<String, String>) {
    let mut slot = outcome.0.lock().unwrap();
    if slot.is_none() {
        *slot = Some(verdict);
    }
    exit.write(AppExit::Success);
}
