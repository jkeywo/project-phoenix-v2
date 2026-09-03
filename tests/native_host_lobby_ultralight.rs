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
//! 5. **The join QR draws, toggles, and says when there is nothing to draw**
//!    (issue #1329). The vendored encoder loads from this process's own
//!    delivery server — the claim a CDN `<script>` could not make on a bridge
//!    machine with no internet — and rasterises into the page's canvas; a
//!    phone's toggle and the surface's own control both flip the panel; and a
//!    host nobody can join says so instead of framing a dead code. What the QR
//!    *encodes* is settled without a browser, in
//!    `native_host::host_lobby::join` and `tests/client/host-qr.test.js`, which
//!    pin the same join-URL literal this file asserts on screen.
//! 6. **The scenario picker builds, answers a click, and closes** (issue
//!    #1328). The shared renderer's dynamic
//!    `import('./components/ph-ship-picker.js')` resolving from this document's
//!    own depth is a claim only a real module loader can settle; so is a real
//!    click reaching the page→host queue THIS surface drains, which is not the
//!    pane bus's. The AI-launch control is asserted present and driven by the
//!    same `aiLaunchVisible` the host page's is.
//! 7. **The monitor row draws, and a press comes back** (issue #1330). A real
//!    `BridgeLayoutPayload` builds real `<button>` elements through the shared
//!    renderer, and clicking one puts a `set-viewscreen` record on the page's
//!    own queue, which the host drains over the real bridge.
//! 8. **A station's screen row draws inside its card, and both of its presses
//!    come back** (issue #1331). The strip is built by the same shared
//!    renderer, the viewscreen's own display is never among its buttons, and a
//!    screen press and the off button leave the page as `assign-station` and
//!    `unassign-station`. What a press then OPENS — a Station window and a
//!    seated pane — is winit's and the pane host's, and is the #1335 kit's
//!    walkthrough.
//!
//! Together, (6), (7) and (8) are the whole page→host direction, in the engine
//! that actually runs it — and they are where a namespace mistake would
//! surface, because the lobby drains `phoenixHostLobbyOut` and a pane drains
//! `phoenixPaneOut`. All six record kinds ride ONE queue and one drain, so a
//! test that saw only some of them arrive would be the first sign of a second
//! reader.
//!
//! The half this cannot reach is the compositing itself — the Bevy node, its
//! `display`, and the router placement. Those need a window and a GPU adapter;
//! `native_host::host_lobby::reveal` is where their decision is made and tested.
//!
//! Nor can it reach the **window move** a press causes: that is winit's, on real
//! monitors, and `tests/native_bridge_displays.rs` is the ignored test that
//! opens real borderless-fullscreen surfaces. The whole-feature walkthrough —
//! press a button on one screen and watch the viewscreen arrive on another — is
//! the **#1335 acceptance kit**'s step, and belongs in `docs/acceptance/`
//! beside the #1124/#1126/#1128 kits rather than in any `cargo test`.

#![cfg(all(feature = "ultralight", not(target_arch = "wasm32")))]

use std::time::{Duration, Instant};

use project_phoenix::core::codec;
use project_phoenix::core::messages::{LobbyStatePayload, ScenarioCatalogWire, StationPayload};
use project_phoenix::core::rendezvous::JoinCode;
use project_phoenix::delivery::args::{ClientSource, HostArgs};
use project_phoenix::delivery::serve::{HostServer, ShutdownSignal};
use project_phoenix::native_host::host_lobby::layout::{
    BridgeLayoutPayload, LayoutNoticePayload, MonitorButtonPayload, StationRowPayload,
    StationScreenPayload,
};
use project_phoenix::native_host::host_lobby::{
    pump_host_lobby, JoinInvite, LocalHostLobby, ScenarioPanelPayload,
};
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
    let station = |id: &str, name: &str, code: &str, rank: &str, holder: Option<&str>| {
        StationPayload {
            // The key the per-station screen row is joined to its card by
            // (issue #1331).
            id: id.to_string(),
            name: name.to_string(),
            short_code: code.to_string(),
            rank: rank.to_string(),
            holder_name: holder.map(str::to_string),
            is_mine: false,
            preset_names: vec![],
        }
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
            station("helm", "Helm", "HLM", "Lieutenant", holder),
            station("weapons", "Tactical", "TAC", "Ensign", None),
        ],
        spectators: vec![],
        loading_progress: None,
        countdown_secs: 0,
    };
    codec::encode_lobby_state(&payload).expect("the lobby payload encodes")
}

/// A two-monitor bridge, in the shape `host_lobby::layout::bridge_layout_payload`
/// builds from a live [`BridgeLayout`] (issues #1330, #1331).
///
/// `helm_on_benq` seats the first station's console on the second display, so
/// the screen row can be driven through both of its states.
fn monitor_row(
    viewscreen_is_second: bool,
    helm_on_benq: bool,
    notices: Vec<LayoutNoticePayload>,
) -> String {
    let monitor =
        |identity: &str, name: &str, w: u32, h: u32, primary, viewscreen| MonitorButtonPayload {
            identity: identity.to_string(),
            name: Some(name.to_string()),
            width: w,
            height: h,
            primary,
            viewscreen,
            stations: Vec::new(),
            // Nothing a `--profile` opened: this fixture's screens are the
            // lobby's to fill and to free.
            reserved: Vec::new(),
        };
    codec::encode_bridge_layout(&BridgeLayoutPayload {
        monitors: vec![
            monitor(
                "BRAVIA@3840x2160",
                "BRAVIA",
                3840,
                2160,
                true,
                !viewscreen_is_second,
            ),
            monitor(
                "BenQ EX@1920x1080",
                "BenQ EX",
                1920,
                1080,
                false,
                viewscreen_is_second,
            ),
        ],
        // The screen rows the layout law's `eligibility` produces for this
        // arrangement: the viewscreen's own display is excluded for every
        // station, and the other is either free or holding helm's console.
        stations: vec![
            StationRowPayload {
                station: "helm".to_string(),
                assigned_to: helm_on_benq.then(|| "BenQ EX@1920x1080".to_string()),
                monitors: vec![
                    StationScreenPayload {
                        identity: "BRAVIA@3840x2160".to_string(),
                        choice: if viewscreen_is_second {
                            "eligible".to_string()
                        } else {
                            "excluded".to_string()
                        },
                        excluded: (!viewscreen_is_second).then(|| "is-viewscreen".to_string()),
                    },
                    StationScreenPayload {
                        identity: "BenQ EX@1920x1080".to_string(),
                        choice: match (viewscreen_is_second, helm_on_benq) {
                            (true, _) => "excluded".to_string(),
                            (false, true) => "selected".to_string(),
                            (false, false) => "eligible".to_string(),
                        },
                        excluded: viewscreen_is_second.then(|| "is-viewscreen".to_string()),
                    },
                ],
            },
            StationRowPayload {
                station: "weapons".to_string(),
                assigned_to: None,
                monitors: vec![
                    StationScreenPayload {
                        identity: "BRAVIA@3840x2160".to_string(),
                        choice: if viewscreen_is_second {
                            "eligible".to_string()
                        } else {
                            "excluded".to_string()
                        },
                        excluded: (!viewscreen_is_second).then(|| "is-viewscreen".to_string()),
                    },
                    StationScreenPayload {
                        identity: "BenQ EX@1920x1080".to_string(),
                        choice: if viewscreen_is_second {
                            "excluded".to_string()
                        } else {
                            "eligible".to_string()
                        },
                        excluded: viewscreen_is_second.then(|| "is-viewscreen".to_string()),
                    },
                ],
            },
        ],
        notices,
    })
    .expect("the monitor row encodes")
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
    // `for_host_lobby`, not `new`: the lobby document installs the
    // `phoenixHostLobbyOut` queue, and a surface built for a pane would drain
    // `phoenixPaneOut` — a function this page never defines — so every pick and
    // every button press would vanish with a clean log. That is the exact
    // mistake sections 6 and 7 below would otherwise not catch.
    let mut surface = UltralightPaneSurface::for_host_lobby(view);
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

    // The surface's launch control (issue #1328). It exists — #1325 stripped it
    // because nothing answered it, and `apply_force_start` stopping being
    // wasm-only removed that reason — and its visibility is the shared view
    // model's `aiLaunchVisible`, exactly as it is on the host page. The payload
    // above has a connected player, so it is hidden.
    assert_eq!(
        probe(
            &mut surface,
            "String(document.getElementById('ai-launch-btn') !== null)"
        )
        .as_deref(),
        Some("true"),
        "the lobby document carries the launch control the shared renderer drives"
    );
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('ai-launch-btn').style.display"
        )
        .as_deref(),
        Some("none"),
        "…hidden while somebody is connected, from the same view model the page uses"
    );

    // ── 6. The scenario picker, in a real browser engine (issue #1328) ──────
    //
    // The half no unit test can reach: the shared renderer's dynamic
    // `import('./components/ph-ship-picker.js')` actually resolving from this
    // document's own depth, and the picker's buttons actually appearing on a
    // page a viewscreen is showing.
    bridge.push_scenario(
        ScenarioPanelPayload {
            scenarios: vec![ScenarioCatalogWire {
                id: "combat_test".to_string(),
                world: "assets/worlds/combat_test.toml".to_string(),
                label: Some("Combat Test".to_string()),
                description: None,
                ships: Vec::new(),
            }],
            locked_scenario: None,
            locked_ship: None,
            locked: false,
        }
        .to_json(),
    );
    let deadline = Instant::now() + PATIENCE;
    let mut entries = String::new();
    while Instant::now() < deadline {
        frame(&mut surface);
        if let Some(count) = probe(
            &mut surface,
            "String(document.querySelectorAll('#world-list .scenario-entry').length)",
        ) {
            if count != "0" && !count.is_empty() {
                entries = count;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    assert_eq!(
        entries, "1",
        "the shared picker builds one button per catalogue entry from the payload"
    );
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('scenario-panel').style.display"
        )
        .as_deref(),
        Some(""),
        "and the panel it builds them into is on screen while the pick is open"
    );
    // A click is what an operator makes, and it must reach the host's own
    // page→host queue — the one this surface drains, not the pane bus's.
    //
    // Read through `bridge.take_records()`, not `surface.drain()`, because
    // `frame` above already ran `pump_host_lobby`, which is what moves the
    // page's queue into the bridge — the surface's own queue is empty by now.
    // Taking here also leaves the bridge clean for section 7, which asserts on
    // exactly what ITS press queued: one drain, one reader, in the test too.
    probe(
        &mut surface,
        "document.querySelector('#world-list .scenario-entry').click()",
    );
    frame(&mut surface);
    let picked = bridge.take_records();
    assert!(
        picked
            .iter()
            .any(|r| r.contains("select_scenario") && r.contains("combat_test")),
        "a click on the viewscreen's picker queues the record the host arbitrates: {picked:?}"
    );

    // …and a world landing takes the picker off the screen.
    bridge.push_scenario(
        ScenarioPanelPayload {
            locked: true,
            ..Default::default()
        }
        .to_json(),
    );
    for _ in 0..8 {
        frame(&mut surface);
    }
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('scenario-panel').style.display"
        )
        .as_deref(),
        Some("none"),
        "a loaded world closes the picker and uncovers the crew lobby"
    );

    // ── 5. The join QR, in a real browser engine (issue #1329) ──────────────
    //
    // The half no unit test can reach: the VENDORED encoder actually loading
    // from this process's own delivery server, and actually rasterising into
    // the page's canvas. Everything about what it encodes is settled in
    // `native_host::host_lobby::join` and `tests/client/host-qr.test.js`; what
    // is settled here is that the code appears on a screen.
    bridge.push_lobby_state(lobby_payload("Lobby", Some("ada")));
    lobby.publish_join(&JoinInvite::from_code(
        &JoinCode {
            full: "PHX-1-ABCDE".to_string(),
            suffix: "ABCDE".to_string(),
            ..Default::default()
        },
        "http://192.168.1.5:8080/",
        None,
    ));
    for _ in 0..8 {
        frame(&mut surface);
    }

    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('qr-url').textContent"
        )
        .as_deref(),
        Some("http://192.168.1.5:8080/client/index.html#PHX-1-ABCDE"),
        "the URL under the code is the join URL a phone needs — the SAME literal \
         tests/client/host-qr.test.js pins, built by the same gui/join-url.js the \
         browser host uses"
    );
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('join-code').textContent"
        )
        .as_deref(),
        Some("ABCDE"),
        "…and the the code beside it, for a camera that will not focus"
    );
    assert_eq!(
        probe(
            &mut surface,
            "String(document.getElementById('qr').width > 0 \
             && document.getElementById('qr').style.display === 'block')"
        )
        .as_deref(),
        Some("true"),
        "the vendored encoder loaded from this host's own server and rasterised into \
         the canvas — the claim a CDN <script> could not make on a bridge machine"
    );
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('overlay').style.display"
        )
        .as_deref(),
        Some("block"),
        "the lobby phase shows the join panel"
    );

    // A phone's ToggleQrCode, arriving the way `drain_client_qr_toggle` sends it.
    bridge.push_qr_toggle();
    for _ in 0..8 {
        frame(&mut surface);
    }
    assert_eq!(
        probe(
            &mut surface,
            "document.getElementById('overlay').style.display"
        )
        .as_deref(),
        Some("none"),
        "a phone's toggle hides the panel"
    );

    // The surface's own control, which is what an operator has once F9 has
    // revealed the surface in play: a click on it, in the page, flips the same
    // state without asking the host anything.
    assert_eq!(
        probe(
            &mut surface,
            "(function(){ document.getElementById('host-lobby-qr-toggle').click(); \
             return document.getElementById('overlay').style.display; })()"
        )
        .as_deref(),
        Some("block"),
        "the surface's own control flips the panel back"
    );

    // A host nobody can join says so, in the string table's words, instead of
    // framing a QR that cannot work.
    lobby.publish_join(&JoinInvite::Off);
    for _ in 0..8 {
        frame(&mut surface);
    }
    assert_eq!(
        probe(
            &mut surface,
            "String(document.getElementById('qr-panel').classList.contains('joining-off') \
             && document.getElementById('qr').style.display === 'none')"
        )
        .as_deref(),
        Some("true"),
        "`--solo` (or no --rendezvous) states that joining is off rather than \
         showing a dead QR"
    );

    // ── 7. The monitor row draws, and a press comes back (issue #1330) ───────
    bridge.push_layout(monitor_row(false, false, Vec::new()));
    for _ in 0..8 {
        frame(&mut surface);
    }
    assert_eq!(
        probe(
            &mut surface,
            "String(document.querySelectorAll('#monitor-row-buttons button').length)"
        )
        .as_deref(),
        Some("2"),
        "the shared renderer builds one button per monitor the host reported"
    );
    assert_eq!(
        probe(
            &mut surface,
            "document.querySelector('#monitor-row-buttons button').getAttribute('aria-pressed')"
        )
        .as_deref(),
        Some("true"),
        "the display showing the viewscreen says so to a screen reader, not only in colour"
    );
    let label = probe(
        &mut surface,
        "document.querySelector('#monitor-row-buttons button').textContent",
    )
    .unwrap_or_default();
    assert!(
        label.contains("BRAVIA") && label.contains("3840"),
        "a button names its display recognisably: {label:?}"
    );
    assert!(
        !label.contains('\u{27e8}'),
        "every string resolved through the real strings.csv: {label:?}"
    );

    // The press. A real click on a real button, through the delegated listener
    // `host_lobby_link.js` installed, onto the page's own out-queue — which the
    // host drains on the very next frame over the same bridge.
    let _ = probe(
        &mut surface,
        "document.querySelector('[data-monitor=\"BenQ EX@1920x1080\"]').click(); 'clicked'",
    );
    frame(&mut surface);
    assert_eq!(
        bridge.take_records(),
        vec![r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#.to_string()],
        "the press reaches the host as the record the layout law takes"
    );

    // The host's answer — accepted here, so the mark moves — repaints the row
    // without rebuilding the surface. (A refusal is the same push carrying a
    // notice; the sentence it renders is asserted below.)
    bridge.push_layout(monitor_row(true, false, Vec::new()));
    for _ in 0..8 {
        frame(&mut surface);
    }
    assert_eq!(
        probe(
            &mut surface,
            "document.querySelectorAll('#monitor-row-buttons button')[1].getAttribute('aria-pressed')"
        )
        .as_deref(),
        Some("true"),
        "the mark moved to the display that was pressed"
    );

    // A refusal is visible feedback, resolved from its id on the page.
    bridge.push_layout(monitor_row(
        true,
        false,
        vec![LayoutNoticePayload {
            id: "server.bridge_layout.unknown_monitor".to_string(),
            params: [("monitor".to_string(), "Unplugged@1920x1080".to_string())]
                .into_iter()
                .collect(),
        }],
    ));
    for _ in 0..8 {
        frame(&mut surface);
    }
    let notice = probe(
        &mut surface,
        "document.getElementById('monitor-row-notice').textContent",
    )
    .unwrap_or_default();
    assert!(
        notice.contains("Unplugged@1920x1080") && !notice.contains('\u{27e8}'),
        "the refusal is a sentence the operator can read: {notice:?}"
    );

    // ── 8. A station's screen row, and both of its presses (issue #1331) ────
    //
    // The page→host direction for the OTHER row on this surface. What is
    // settled here and nowhere else is that the strip is built inside a real
    // station card by the shared renderer, that its buttons are reachable, and
    // that each of the two verbs leaves the page as the record the layout law
    // takes. What a press then OPENS — a Station window and a seated pane — is
    // winit's and the pane host's, and belongs to the #1335 acceptance kit's
    // walkthrough: press a screen button on the viewscreen and watch a console
    // appear on the other monitor.
    bridge.push_layout(monitor_row(false, false, Vec::new()));
    for _ in 0..8 {
        frame(&mut surface);
    }
    assert_eq!(
        probe(
            &mut surface,
            "String(document.querySelectorAll('#station-grid .station-card')[0]\
             .querySelectorAll('.station-screen-button').length)"
        )
        .as_deref(),
        Some("2"),
        "off, plus the one display that is not showing the viewscreen"
    );
    let strip = probe(
        &mut surface,
        "document.querySelector('#station-grid .station-screens').textContent",
    )
    .unwrap_or_default();
    assert!(
        strip.contains("BenQ EX") && !strip.contains("BRAVIA"),
        "the viewscreen's own display is never offered a console: {strip:?}"
    );
    assert!(
        !strip.contains('\u{27e8}'),
        "every string resolved through the real strings.csv: {strip:?}"
    );

    let _ = probe(
        &mut surface,
        "document.querySelector('[data-station=\"helm\"][data-screen=\"BenQ EX@1920x1080\"]')\
         .click(); 'clicked'",
    );
    frame(&mut surface);
    assert_eq!(
        bridge.take_records(),
        vec![
            r#"{"kind":"assign-station","station":"helm","monitor":"BenQ EX@1920x1080"}"#
                .to_string()
        ],
        "a screen press reaches the host as the assign the layout law takes"
    );

    // The host's answer: the console is open, so the row marks that screen —
    // and the off button, which is what closes it again.
    bridge.push_layout(monitor_row(false, true, Vec::new()));
    for _ in 0..8 {
        frame(&mut surface);
    }
    assert_eq!(
        probe(
            &mut surface,
            "document.querySelector('[data-station=\"helm\"][data-screen=\"BenQ EX@1920x1080\"]')\
             .getAttribute('aria-pressed')"
        )
        .as_deref(),
        Some("true"),
        "the chosen screen says so to a screen reader, not only in colour"
    );
    let _ = probe(
        &mut surface,
        "document.querySelector('[data-station=\"helm\"][data-screen=\"\"]').click(); 'clicked'",
    );
    frame(&mut surface);
    assert_eq!(
        bridge.take_records(),
        vec![r#"{"kind":"unassign-station","station":"helm"}"#.to_string()],
        "and the off button closes it, through the law's own verb"
    );
}
