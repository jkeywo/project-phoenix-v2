//! The real crew lobby, in a real Ultralight view, over the real bridge
//! (issue #1325).
//!
//! # Why this test is `#[ignore]`d
//!
//! The same two reasons `tests/native_host_pane_ultralight.rs` is, and they are
//! worth repeating rather than cross-referencing. It links the **Ultralight
//! SDK**, which `ul-next-sys`'s build script downloads at build time and whose
//! shared libraries have to be staged beside the test binary with its
//! `resources/` in the working directory; and it loads the document over HTTP
//! from a real delivery host. Nothing in CI has any of that: every job in
//! `.github/workflows/ci.yml` is `ubuntu-latest`, and the `ultralight` feature
//! exists precisely so none of them pays a hundred-megabyte proprietary
//! download to run four thousand unit tests.
//!
//! So it is written to be run **deliberately, on a Windows machine, from this
//! checkout**:
//!
//! ```text
//! TRUNK_BUILD_RELEASE=true trunk build --release
//! cargo test --features ultralight --test native_host_lobby_ultralight -- --ignored --nocapture
//! ```
//!
//! `trunk build` is not optional and is a different prerequisite from the pane
//! test's `build-client.mjs`: the lobby document is built from the **host**
//! page's own `#lobby-panel` markup and renders with the `gui/` modules beside
//! it, so what it needs on disk is `dist/index.html` and `dist/gui/`, which is
//! trunk's output rather than the phone bundle's.
//!
//! # ONE test, because Ultralight is one renderer per process
//!
//! `UltralightRuntime` owns the `Renderer`, which owns the JavaScript VM and the
//! resource cache; a second one in the same process simply never finishes
//! loading a document. Cargo gives this file one binary and runs its tests in
//! threads of it, so two `#[test]`s that each started a runtime would be a
//! second renderer whichever order they ran in.
//!
//! # What it asserts, and why those are the right assertions
//!
//! Everything about the surface that only a browser engine can settle. The
//! contracts — the document assembly, the bridge's latest-wins and deferral
//! rules, the reveal state machine — are unit-tested by the ordinary
//! `cargo test`, in `native_host::host_lobby`.
//!
//! 1. **The document loads over HTTP from this process's own delivery server**,
//!    at the host page's own depth, so `gui/host-lobby.css`,
//!    `gui/host-lobby-view.js`, `gui/host-lobby-render.js` and
//!    `assets/strings/strings.csv` all resolve as they do for the host page.
//! 2. **A real `LobbyStatePayload` — the same bytes the web host's `lobby`
//!    channel carries — renders the real lobby.** The station grid fills, the
//!    claimed seat carries its holder's initials, the crew counter counts. That
//!    is the whole "one rendering path" claim, exercised through the shared
//!    modules rather than asserted about them.
//! 3. **The chrome yields on mission start and comes back on the host key.**
//!    The page's half of the reveal: the phase hides the panel, and the reveal
//!    flag the host pushes brings it back without a reload.
//! 4. **The surface rasterises.** A frame is copied out with a non-empty dirty
//!    region, which is the difference between a page that ran and a page that
//!    would draw nothing.
//!
//! The half this cannot reach is the compositing itself — the Bevy node, its
//! `display`, and the router placement. Those need a window and a GPU adapter;
//! `native_host::host_lobby::reveal` is where their decision is made and tested.

#![cfg(all(feature = "ultralight", not(target_arch = "wasm32")))]

use std::time::{Duration, Instant};

use project_phoenix::core::codec;
use project_phoenix::core::messages::{LobbyStatePayload, StationPayload};
use project_phoenix::delivery::args::{ClientSource, HostArgs};
use project_phoenix::delivery::serve::{HostServer, ShutdownSignal};
use project_phoenix::native_host::host_lobby::{pump_host_lobby, LocalHostLobby};
use project_phoenix::native_host::panes::surface::PaneSurface;
use project_phoenix::native_host::panes::ultralight::{stage_sdk, UltralightPaneSurface};
use vellum_ultralight::runtime::{PaneSession, PaneSpec, RuntimeOptions, UltralightRuntime};

/// The built host bundle. `trunk build` writes it.
const CLIENT_DIR: &str = "dist";
/// How long the page gets to load and render before the test gives up.
const PATIENCE: Duration = Duration::from_secs(30);
/// Surface size. A whole viewscreen, because that is what the surface takes.
const SURFACE_SIZE: (u32, u32) = (1280, 900);

/// A running delivery host and the way to stop it.
struct Delivery {
    addr: String,
    documents: project_phoenix::delivery::serve::HostedDocuments,
    shutdown: ShutdownSignal,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Delivery {
    /// A delivery host on a loopback port of the OS's choosing, serving the real
    /// bundle. `skip_bundle_check` forgives a bundle with no `[content]`
    /// identity (an un-built checkout's, or one built from another manifest); it
    /// does not forgive a real mismatch, and the lobby surface is not what that
    /// pin protects.
    fn start() -> Self {
        let args = HostArgs {
            addr: "127.0.0.1:0".to_string(),
            client: ClientSource::Bundled {
                dir: CLIENT_DIR.to_string(),
            },
            manifest: "assets/scenarios.toml".to_string(),
            content_dir: ".".to_string(),
            skip_bundle_check: true,
            sim: None,
            setup: false,
            profile: None,
        };
        let server = HostServer::bind(&args).expect("the delivery host binds");
        let addr = server.local_addr();
        let documents = server.hosted_documents();
        server
            .enable_shutdown_polling()
            .expect("the listener can be polled for shutdown");
        let shutdown = ShutdownSignal::new();
        let serving = shutdown.clone();
        let thread = std::thread::spawn(move || {
            let _ = server.serve_until(serving, |_| {});
        });
        Self {
            addr,
            documents,
            shutdown,
            thread: Some(thread),
        }
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        self.shutdown.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One lobby snapshot, in the shape `viewscreen_border::push_lobby_state`
/// builds and `codec::encode_lobby_state` encodes.
fn lobby_payload(phase: &str, holder: Option<&str>) -> String {
    let station = |name: &str, code: &str, rank: &str, holder: Option<&str>| StationPayload {
        name: name.to_string(),
        short_code: code.to_string(),
        rank: rank.to_string(),
        holder_name: holder.map(str::to_string),
        is_mine: false,
        preset_names: vec![],
    };
    let payload = LobbyStatePayload {
        phase: phase.to_string(),
        scenario_title: "Combat Test".to_string(),
        scenario_body: "A shakedown run.".to_string(),
        crew_count: holder.is_some() as u32,
        max_players: 2,
        all_stations_filled: false,
        all_ready: false,
        stations: vec![
            station("Helm", "HLM", "Lieutenant", holder),
            station("Tactical", "TAC", "Ensign", None),
        ],
        spectators: vec![],
        loading_progress: None,
        countdown_secs: 0,
    };
    codec::encode_lobby_state(&payload).expect("the lobby payload encodes")
}

#[test]
#[ignore = "needs the Ultralight SDK, a trunk-built dist/, and a machine that can run both"]
fn the_native_lobby_renders_the_web_hosts_own_lobby_over_the_bridge() {
    match stage_sdk() {
        Ok(summary) => eprintln!("[lobby] {summary}"),
        Err(e) => panic!("the Ultralight SDK is not staged: {e}"),
    }

    let delivery = Delivery::start();
    let index = std::path::Path::new(CLIENT_DIR).join("index.html");
    let host_page = std::fs::read_to_string(&index).unwrap_or_else(|e| {
        panic!(
            "{} is missing ({e}) — run `TRUNK_BUILD_RELEASE=true trunk build --release` first",
            index.display()
        )
    });

    let lobby = LocalHostLobby::open(&delivery.addr);
    lobby
        .publish(&host_page, &delivery.documents)
        .expect("the host page becomes a lobby document");

    let runtime = UltralightRuntime::start(&RuntimeOptions::default()).expect("the runtime starts");
    let view = runtime
        .create_pane(&PaneSpec {
            width: SURFACE_SIZE.0,
            height: SURFACE_SIZE.1,
            device_scale: 1.0,
            transparent: false,
            session: Some(PaneSession::ephemeral("host-lobby".to_string())),
        })
        .expect("the view is created");
    let mut surface = UltralightPaneSurface::new(view);
    surface.load(&lobby.url()).expect("the document loads");

    let bridge = lobby.bridge.clone();

    // One frame of the host's own loop: service the library, move what is
    // pending across the bridge, rasterise. The SAME `pump_host_lobby`
    // `drive_panes` calls.
    let frame = |surface: &mut UltralightPaneSurface| {
        runtime.update();
        surface.refresh_loaded();
        pump_host_lobby(&bridge, surface);
        runtime.render();
    };

    /// Evaluate a page expression, or `None` while the page cannot answer.
    fn probe(surface: &mut UltralightPaneSurface, script: &str) -> Option<String> {
        surface.view_mut().evaluate(script).ok()
    }

    // ── 2. A real payload renders the real lobby ────────────────────────────
    bridge.push_lobby_state(lobby_payload("Lobby", Some("ada")));

    let deadline = Instant::now() + PATIENCE;
    let mut cards = String::new();
    while Instant::now() < deadline {
        frame(&mut surface);
        if let Some(count) = probe(
            &mut surface,
            "String(document.querySelectorAll('#station-grid .station-card').length)",
        ) {
            if count != "0" && !count.is_empty() {
                cards = count;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    assert_eq!(
        cards, "2",
        "the shared renderer builds one card per station from the payload"
    );
    assert_eq!(
        probe(
            &mut surface,
            "document.querySelector('#station-grid .station-card.claimed .card-avatar').textContent"
        )
        .as_deref(),
        Some("AD"),
        "the claimed seat carries its holder's initials"
    );
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('lobby-crew-count').textContent"
        )
        .as_deref(),
        Some("1/2"),
    );
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('lobby-title').textContent"
        )
        .as_deref(),
        Some("Combat Test"),
        "the world's authored title, resolved through the same localisation boundary",
    );
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('lobby-panel').style.display"
        )
        .as_deref(),
        Some(""),
        "the lobby phase shows the chrome"
    );

    // ── 4. The surface rasterises ───────────────────────────────────────────
    let mut pixels = vec![0u8; (SURFACE_SIZE.0 * SURFACE_SIZE.1 * 4) as usize];
    let dirty = surface
        .view_mut()
        .copy_frame(&mut pixels, true)
        .expect("the surface has pixels to copy");
    assert!(
        dirty.is_some(),
        "a page that rendered has a non-empty dirty region"
    );

    // ── 3. Mission start yields the chrome, and the host key brings it back ──
    bridge.push_lobby_state(lobby_payload("InProgress", Some("ada")));
    bridge.push_reveal(false);
    for _ in 0..8 {
        frame(&mut surface);
    }
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('lobby-panel').style.display"
        )
        .as_deref(),
        Some("none"),
        "on mission start the lobby chrome yields to the viewscreen"
    );

    bridge.push_reveal(true);
    for _ in 0..8 {
        frame(&mut surface);
    }
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('lobby-panel').style.display"
        )
        .as_deref(),
        Some(""),
        "the host key brings the chrome back in play, with no reload"
    );
    assert_eq!(
        probe(
            &mut surface,
            "String(document.querySelectorAll('#station-grid .station-card').length)"
        )
        .as_deref(),
        Some("2"),
        "…still showing the state it was last given — the surface was never rebuilt"
    );

    // The read-only half: nothing on this surface can launch anything.
    assert_eq!(
        probe(
            &mut surface,
            "String(document.querySelectorAll('button').length)"
        )
        .as_deref(),
        Some("0"),
        "the lobby document carries no controls in this slice"
    );
}
