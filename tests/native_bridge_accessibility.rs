//! The configured pane layout stays operable under the shared Accessibility
//! profile on this machine's REAL monitors (issue #1128, acceptance criterion 5,
//! the Windows multi-monitor smoke half).
//!
//! # Why this test is `#[ignore]`d
//!
//! The pure model in `src/native_host/setup_accessibility.rs` proves the reflow
//! headroom, the focus order across monitors and split panes, the reticle
//! geometry and the setup-action reachability on *synthetic* monitor geometries,
//! in ordinary CI on no hardware. The one thing it cannot supply is a **real**
//! monitor's real resolution and real scale factor — and a bridge is set up on
//! whatever displays are actually plugged in. So this reads the monitors winit
//! reports on the dev box, builds the same one-/two-pane profile the operator
//! would, resolves it against those real monitors, and asserts the layout
//! preserves every console across the supported text-scale extremes and that the
//! keyboard-focus order reaches every pane on every monitor.
//!
//! Every CI job here is `ubuntu-latest` with one headless display; the one
//! `windows-latest` runner (`deploy-demo.yml`) runs nothing. So this is written
//! to be run **deliberately, on the multi-monitor Windows dev box**:
//!
//! ```text
//! cargo test --features host --test native_bridge_accessibility -- --ignored --nocapture
//! ```
//!
//! It needs a display to enumerate monitors (the winit event loop), but it does
//! **not** open Station windows or draw the reticle — that visual half (the
//! bracketed focus frame, the contrast-bolded reticle, reflow with real text) is
//! the human walkthrough in `docs/acceptance/1128-accessibility.md`. This test is
//! the arithmetic half made real: the numbers this machine's monitors actually
//! report, run through the same pure checks CI runs on invented ones.

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::window::Monitor;

use project_phoenix::native_host::bridge_profile::{
    identify, resolve, BridgeProfile, DisplayEntry, PaneSlot, PaneSplit, RawMonitor, ROLE_STATION,
    ROLE_VIEWSCREEN,
};
use project_phoenix::native_host::panes::os_prefs::query_os_accessibility_prefs;
use project_phoenix::native_host::setup_accessibility::{
    bridge_focus_order, bridge_preserves_all_consoles, render_accessibility_setup_report,
    SUPPORTED_TEXT_SCALE_MAX, SUPPORTED_TEXT_SCALE_MIN,
};

/// Frames to give winit to report the monitors. Generous; the test finishes the
/// instant the list is non-empty, so the ceiling only bites on a genuine failure.
const FRAME_BUDGET: u32 = 600;

/// The verdict, carried out of the app (which consumes its world on `run`).
type Verdict = Arc<Mutex<Option<Result<String, String>>>>;

#[derive(Resource, Clone)]
struct Outcome(Verdict);

#[derive(Resource, Default)]
struct Frames(u32);

#[test]
#[ignore = "reads this machine's real monitors; needs a display. \
            Run: cargo test --features host --test native_bridge_accessibility -- --ignored --nocapture"]
fn the_configured_layout_stays_operable_on_real_monitors() {
    let verdict: Verdict = Arc::new(Mutex::new(None));

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "phoenix-host — bridge accessibility test".to_string(),
                    visible: false,
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
            // shipped `phoenix-host` binary has no such problem.
            .set(bevy::winit::WinitPlugin {
                run_on_any_thread: true,
            }),
    );
    app.insert_resource(Outcome(verdict.clone()));
    app.init_resource::<Frames>();
    app.add_systems(Update, drive);
    app.run();

    let result = verdict.lock().unwrap().take();
    match result {
        Some(Ok(summary)) => println!("native bridge accessibility:\n{summary}"),
        Some(Err(reason)) => panic!("native bridge accessibility: {reason}"),
        None => panic!(
            "native bridge accessibility: the app exited without a verdict — no monitors were \
             ever reported"
        ),
    }
}

/// Enumerate the real monitors, build a one-/two-pane profile from them, and
/// assert the layout preserves every console across the supported text-scale
/// extremes and that the focus order reaches every pane.
fn drive(
    monitors: Query<(&Monitor, Has<bevy::window::PrimaryMonitor>)>,
    mut frames: ResMut<Frames>,
    outcome: Res<Outcome>,
    mut exit: MessageWriter<AppExit>,
) {
    frames.0 += 1;
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
        if frames.0 >= FRAME_BUDGET {
            finish(&outcome, &mut exit, Err("no monitors were reported".into()));
        }
        return;
    }

    let discovered = identify(&raws.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>());

    // The viewscreen is the primary monitor (or the first); a Station is any
    // other monitor, split side by side into two panes. With only one monitor the
    // single display is the Station of two panes — the single-monitor fallback
    // the #1124 kit uses — so the check still exercises a two-pane split.
    let vs_index = raws.iter().position(|(_, p)| *p).unwrap_or(0);
    let vs = &discovered[vs_index];
    let station = discovered.iter().find(|d| d.identity != vs.identity);

    let mut entries = Vec::new();
    let station_id = match station {
        Some(st) => {
            entries.push(DisplayEntry {
                id: vs.identity.as_str().to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            });
            st.identity.as_str().to_string()
        }
        // One monitor: it is the Station itself.
        None => vs.identity.as_str().to_string(),
    };
    entries.push(DisplayEntry {
        id: station_id.clone(),
        role: ROLE_STATION.to_string(),
        split: Some(PaneSplit::SideBySide),
        panes: vec![
            PaneSlot {
                label: "Ada".to_string(),
            },
            PaneSlot {
                label: "Grace".to_string(),
            },
        ],
    });

    let profile = BridgeProfile {
        version: project_phoenix::native_host::bridge_profile::PROFILE_VERSION,
        displays: entries,
        touch: Vec::new(),
        media: Vec::new(),
    };
    let validated = match profile.validate() {
        Ok(v) => v,
        Err(e) => {
            finish(
                &outcome,
                &mut exit,
                Err(format!(
                    "a profile built from real monitors did not validate: {e}"
                )),
            );
            return;
        }
    };
    let resolved = resolve(&validated, &discovered);
    let panes = bridge_focus_order(&resolved);

    if panes.len() != 2 {
        finish(
            &outcome,
            &mut exit,
            Err(format!(
                "expected two Station panes on {station_id}, got {}",
                panes.len()
            )),
        );
        return;
    }

    // AC1 on real geometry: every pane preserves its console across the supported
    // extremes. Each pane is checked at the minimum and the maximum text scale.
    for pane in &panes {
        let b = pane.content_box();
        for scale in [SUPPORTED_TEXT_SCALE_MIN, SUPPORTED_TEXT_SCALE_MAX] {
            if !b.preserves_console_at_scale(scale) {
                finish(
                    &outcome,
                    &mut exit,
                    Err(format!(
                        "pane {} on {} ({}x{} logical) does not preserve its console at text \
                         scale {scale}x — this monitor is too small for a two-pane split; use one \
                         pane or a larger display",
                        pane.label, pane.monitor, b.logical_width as u32, b.logical_height as u32,
                    )),
                );
                return;
            }
        }
    }

    if !bridge_preserves_all_consoles(&resolved) {
        finish(
            &outcome,
            &mut exit,
            Err("the bridge does not preserve all consoles".into()),
        );
        return;
    }

    // AC3 on real geometry: the focus order reaches every configured pane.
    let labels: Vec<&str> = panes.iter().map(|p| p.label.as_str()).collect();
    if labels != ["Ada", "Grace"] {
        finish(
            &outcome,
            &mut exit,
            Err(format!("unexpected focus order: {labels:?}")),
        );
        return;
    }

    // The report an operator would read on this machine, for the --nocapture log.
    let report =
        render_accessibility_setup_report(Some(&resolved), &query_os_accessibility_prefs());
    finish(
        &outcome,
        &mut exit,
        Ok(format!(
            "{} monitor(s); two-pane Station on {station_id} preserves both consoles from \
             {SUPPORTED_TEXT_SCALE_MIN}x to {SUPPORTED_TEXT_SCALE_MAX}x; focus order {labels:?}.\n\n{report}",
            discovered.len()
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
