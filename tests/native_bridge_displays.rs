//! Real borderless-fullscreen bridge surfaces open on this machine's monitors,
//! with the geometry the profile asked for (issue #1123, acceptance criterion 6,
//! Windows integration half).
//!
//! # Why this test is `#[ignore]`d
//!
//! It opens **real winit windows** and needs real monitors and a GPU adapter —
//! and, unlike the offscreen render proof (`tests/native_viewscreen_render.rs`),
//! it cannot fall back to an offscreen surface, because borderless-fullscreen is
//! a claim about a physical display. Every CI job here is `ubuntu-latest` and
//! headless; the one `windows-latest` runner (`deploy-demo.yml`) runs nothing. So
//! this is written to be run **deliberately, on a machine with displays**:
//!
//! ```text
//! cargo test --features host --test native_bridge_displays -- --ignored --nocapture
//! ```
//!
//! It briefly takes over the screen(s) it covers, which is the nature of the
//! proof; it exits the instant it has verified the geometry.
//!
//! # What a real run still has to show for the 2-up split (issue #1332)
//!
//! What this test proves is the part a machine can judge: that a surface covers
//! the display it was assigned, at that display's geometry. Everything issue
//! #1332 decides *about* a screen holding two consoles is decided in code that
//! runs headlessly — the law's occupancy, capacity and greying in
//! `src/native_host/bridge_layout.rs`, the single tiling and the re-tile rebuild
//! in `follow_layout_stations`, the row in
//! `tests/client/host-lobby-view.test.js` — because none of it needs a pixel.
//!
//! Three claims are left that only real monitors can settle, and they belong in
//! issue #1335's guided acceptance kit rather than here, because each one ends
//! in a person saying whether what they are looking at is right:
//!
//! * **Both halves are operable.** Two consoles side by side on one physical
//!   screen, each taking touch and keyboard on its own half — the routing is
//!   #1124's `route_touch_input`, and only a finger proves the boundary is where
//!   the rectangle says it is. A screen shared by a hand-authored `--pane`
//!   console and a lobby-opened one is the same claim with both kinds on it.
//! * **The split is legible at bridge distance.** That is what
//!   `MAX_PANES_PER_STATION = 2` exists to bound, and it is a judgement about a
//!   room rather than about a number.
//! * **The re-tile blink is tolerable.** Seating a second console rebuilds the
//!   first (`LayoutAdoption::ConsoleRetiling`); the seat comes back on the same
//!   token, but how long the page takes to load — and whether the operator's
//!   notice arrives before the crew member asks what happened — is a stopwatch
//!   question on real hardware.
//!
//! Running this file's own test with a `--profile` that seats two panes on one
//! monitor is the nearest automated approximation: it confirms that the Station
//! window the two halves are composited onto does cover its display.
//!
//! # Why it is one app, not two
//!
//! winit's event loop can be created **once per process**, so the test cannot
//! enumerate monitors in one app and open windows in a second. Instead it does
//! both inside one running app: a driver system reads the monitors the moment
//! winit reports them, builds a profile that names the primary monitor the
//! viewscreen (and a second monitor, if present, a Station), inserts it, and lets
//! [`BridgeDisplayPlugin`] apply it — then, once the windows exist and winit has
//! sized them, checks each window's physical geometry against the monitor it was
//! placed on and exits. The verdict is carried out of the app through a shared
//! handle, because `App::run()` consumes the world.

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::window::{Monitor, PrimaryMonitor, PrimaryWindow, Window, WindowMode};

use project_phoenix::native_host::bridge_display::{
    BridgeDisplayApplied, BridgeDisplayConfig, BridgeDisplayPlugin, BridgeStationSurfaces,
    BridgeSurface,
};
use project_phoenix::native_host::bridge_profile::{
    identify, BridgeProfile, DisplayEntry, PaneSlot, RawMonitor, ROLE_STATION, ROLE_VIEWSCREEN,
};

/// Frames to give winit to report the monitors and, once the profile is applied,
/// to resize the borderless-fullscreen windows to their monitors. Generous; the
/// test exits as soon as the geometry matches, so the ceiling only bites on a
/// genuine failure.
const FRAME_BUDGET: u32 = 900;

/// The verdict, carried out of the app (which consumes its world on `run`).
type Verdict = Arc<Mutex<Option<Result<String, String>>>>;

#[derive(Resource, Clone)]
struct Outcome(Verdict);

/// What the driver expects each surface to become, recorded when it builds the
/// profile so the check can compare against it.
#[derive(Clone)]
struct Expected {
    viewscreen_id: String,
    viewscreen_size: (u32, u32),
    station: Option<(String, (u32, u32))>,
}

#[derive(Resource, Default)]
struct Driver {
    built: bool,
    expected: Option<Expected>,
    frames: u32,
}

#[test]
#[ignore = "opens real borderless-fullscreen windows; needs real monitors and a GPU. \
            Run: cargo test --features host --test native_bridge_displays -- --ignored --nocapture"]
fn a_bridge_profile_covers_each_monitor_with_a_borderless_fullscreen_surface() {
    let verdict: Verdict = Arc::new(Mutex::new(None));

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "phoenix-host — bridge display test".to_string(),
                    ..default()
                }),
                ..default()
            })
            .set(bevy::log::LogPlugin {
                filter: "warn".to_string(),
                ..default()
            })
            // The Rust test harness runs each `#[test]` on a spawned thread, and
            // winit refuses to create its event loop off the main thread. The
            // shipped `phoenix-host` binary has no such problem — its `main`
            // owns the main thread — so this is a test-only accommodation.
            .set(bevy::winit::WinitPlugin {
                run_on_any_thread: true,
            }),
    );
    app.add_plugins(BridgeDisplayPlugin);
    app.insert_resource(Outcome(verdict.clone()));
    app.init_resource::<Driver>();
    app.add_systems(Update, drive);

    app.run();

    let result = verdict.lock().unwrap().take();
    match result {
        Some(Ok(summary)) => println!("native bridge displays: {summary}"),
        Some(Err(reason)) => panic!("native bridge displays: {reason}"),
        None => panic!(
            "native bridge displays: the app exited without reaching a verdict — no monitors \
             were ever reported, or the window never applied the profile"
        ),
    }
}

/// The whole test loop, in one system: build the profile from the real monitors,
/// let the plugin apply it, then verify the surface geometry and exit.
#[allow(clippy::too_many_arguments)]
fn drive(
    monitors: Query<(&Monitor, Has<PrimaryMonitor>)>,
    applied: Option<Res<BridgeDisplayApplied>>,
    primary_win: Query<(&Window, Option<&BridgeSurface>), With<PrimaryWindow>>,
    windows: Query<&Window>,
    stations: Option<Res<BridgeStationSurfaces>>,
    mut driver: ResMut<Driver>,
    outcome: Res<Outcome>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    driver.frames += 1;
    if driver.frames > FRAME_BUDGET {
        finish(
            &outcome,
            &mut exit,
            Err(format!(
                "timed out after {FRAME_BUDGET} frames (built profile: {}, applied: {})",
                driver.built,
                applied.is_some()
            )),
        );
        return;
    }

    // Phase 1 — build a profile from the monitors winit reports.
    if !driver.built {
        let raws: Vec<(RawMonitor, bool)> = monitors
            .iter()
            .map(|(m, primary)| {
                (
                    RawMonitor {
                        name: m.name.clone(),
                        physical_width: m.physical_width,
                        physical_height: m.physical_height,
                        position_x: m.physical_position.x,
                        position_y: m.physical_position.y,
                        scale_factor: m.scale_factor,
                        primary,
                    },
                    primary,
                )
            })
            .collect();
        if raws.is_empty() {
            return; // winit has not reported the monitors yet.
        }
        let discovered = identify(&raws.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>());

        // The viewscreen is the primary monitor (or the first, if none is
        // flagged); a Station is any other monitor, when there is one.
        let vs_index = raws.iter().position(|(_, p)| *p).unwrap_or(0);
        let vs = &discovered[vs_index];
        let station = discovered.iter().find(|d| d.identity != vs.identity);

        let mut entries = vec![DisplayEntry {
            id: vs.identity.as_str().to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        }];
        if let Some(st) = station {
            entries.push(DisplayEntry {
                id: st.identity.as_str().to_string(),
                role: ROLE_STATION.to_string(),
                split: None,
                panes: vec![PaneSlot::for_participant("TestOperator")],
            });
        }
        let profile = BridgeProfile {
            version: project_phoenix::native_host::bridge_profile::PROFILE_VERSION,
            displays: entries,
            touch: Vec::new(),
            media: Vec::new(),
        };
        let validated = profile
            .validate()
            .expect("a profile built from real monitors validates");

        driver.expected = Some(Expected {
            viewscreen_id: vs.identity.as_str().to_string(),
            viewscreen_size: (vs.geometry.physical_width, vs.geometry.physical_height),
            station: station.map(|st| {
                (
                    st.identity.as_str().to_string(),
                    (st.geometry.physical_width, st.geometry.physical_height),
                )
            }),
        });
        commands.insert_resource(BridgeDisplayConfig {
            profile: validated,
            // An operator wrote this one, so the adapter places the windows it
            // names (issue #1330 made that distinction explicit).
            authored: true,
        });
        driver.built = true;
        return;
    }

    // Phase 2 — wait for the plugin to open the surfaces.
    if applied.is_none() {
        return;
    }
    let expected = driver
        .expected
        .clone()
        .expect("built implies an expectation");

    // The viewscreen is the primary window: check it is tagged, borderless
    // fullscreen, and sized to its monitor. winit resizes a frame or two after
    // the mode change, so a mismatch here is "not yet" until the budget runs out.
    let Ok((window, surface)) = primary_win.single() else {
        return;
    };
    let Some(surface) = surface else {
        return; // tag not applied yet
    };
    if surface.identity != expected.viewscreen_id {
        finish(
            &outcome,
            &mut exit,
            Err(format!(
                "the primary window was tagged for monitor {:?}, expected the viewscreen {:?}",
                surface.identity, expected.viewscreen_id
            )),
        );
        return;
    }
    if !matches!(window.mode, WindowMode::BorderlessFullscreen(_)) {
        finish(
            &outcome,
            &mut exit,
            Err(format!(
                "the viewscreen window is {:?}, not borderless fullscreen",
                window.mode
            )),
        );
        return;
    }
    let vs_size = (window.physical_width(), window.physical_height());
    if vs_size != expected.viewscreen_size {
        return; // winit still resizing; keep waiting within the budget
    }

    // The Station window (if the machine has a second monitor): sized to its
    // monitor and listed in the published surfaces.
    let mut station_report = "no second monitor — viewscreen only".to_string();
    if let Some((station_id, station_size)) = &expected.station {
        let Some(surfaces) = stations.as_ref() else {
            return;
        };
        let Some(station) = surfaces.0.iter().find(|s| &s.identity == station_id) else {
            finish(
                &outcome,
                &mut exit,
                Err(format!(
                    "the Station surface for {station_id:?} was not opened"
                )),
            );
            return;
        };
        let Ok(station_window) = windows.get(station.window) else {
            return;
        };
        let got = (
            station_window.physical_width(),
            station_window.physical_height(),
        );
        if got != *station_size {
            return; // still resizing
        }
        if !matches!(station_window.mode, WindowMode::BorderlessFullscreen(_)) {
            finish(
                &outcome,
                &mut exit,
                Err(format!(
                    "the Station window is {:?}, not borderless fullscreen",
                    station_window.mode
                )),
            );
            return;
        }
        // The single pane covers the whole Station monitor.
        let pane = &station.panes[0];
        if pane.rect.width != station_size.0 || pane.rect.height != station_size.1 {
            finish(
                &outcome,
                &mut exit,
                Err(format!(
                    "the single Station pane rect {:?} does not cover its {station_size:?} monitor",
                    pane.rect
                )),
            );
            return;
        }
        station_report = format!(
            "station {station_id} borderless fullscreen at {}x{}, one pane covering it",
            station_size.0, station_size.1
        );
    }

    finish(
        &outcome,
        &mut exit,
        Ok(format!(
            "viewscreen {} borderless fullscreen at {}x{}; {station_report}",
            expected.viewscreen_id, vs_size.0, vs_size.1
        )),
    );
}

/// Record the verdict and ask the app to exit.
fn finish(outcome: &Outcome, exit: &mut MessageWriter<AppExit>, verdict: Result<String, String>) {
    let mut slot = outcome.0.lock().unwrap();
    if slot.is_none() {
        *slot = Some(verdict);
    }
    exit.write(AppExit::Success);
}
