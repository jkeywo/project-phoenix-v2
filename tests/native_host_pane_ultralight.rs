//! Real console pages, in real Ultralight panes, answering a real host
//! (issue #1122, acceptance criteria 2 and 3).
//!
//! # Why this test is `#[ignore]`d
//!
//! It links the **Ultralight SDK**, which `ul-next-sys`'s build script downloads
//! at build time, and it needs that SDK's shared libraries staged beside the
//! test binary and its `resources/` in the working directory. Nothing in CI has
//! any of that: every job in `.github/workflows/ci.yml` is `ubuntu-latest`, and
//! the `ultralight` feature exists precisely so none of them pays a
//! hundred-megabyte proprietary download to run four thousand unit tests.
//!
//! So it is written to be run **deliberately, on a Windows machine, from this
//! checkout**:
//!
//! ```text
//! node scripts/build-client.mjs
//! cargo test --features ultralight --test native_host_pane_ultralight -- --ignored --nocapture
//! ```
//!
//! `scripts/build-client.mjs` is not optional: a pane loads the *built bundle's*
//! `client/index.html`, and the whole claim here is that the page it shows is
//! the one a phone loads.
//!
//! # ONE test, because Ultralight is one renderer per process
//!
//! `UltralightRuntime` owns the `Renderer`, which owns the JavaScript VM and the
//! resource cache, and a second one in the same process does not load documents
//! — it simply sits in `is_loading()` forever. Cargo gives this file one binary
//! and runs its tests in threads of that binary, so two `#[test]`s that each
//! started a runtime would be a second renderer whichever order they ran in.
//! Everything below therefore shares one runtime, one delivery host and one
//! simulation, and drives **two** panes across it — which is closer to the real
//! thing anyway.
//!
//! # What it asserts, and why those are the right assertions
//!
//! Everything about a pane that only a browser engine can settle, and nothing
//! that does not need one — the contracts are `tests/native_host_panes.rs`'s,
//! run by the ordinary `cargo test`.
//!
//! 1. **The real page loads over HTTP from this process's own delivery server.**
//!    Not an `include_str!`'d document: the pane navigates to a URL the
//!    `HostServer` publishes, at the client directory's own depth, so every
//!    relative `gui/` module and console iframe resolves exactly as it does for
//!    a phone.
//! 2. **The injected link replaces PeerJS and the page joins by itself**, on the
//!    identity it read out of the URL *fragment* — which is the whole of
//!    "in-process delivery uses the same logical-client boundaries", carried the
//!    one way that puts nothing on the wire. The served document is checked for
//!    the token as well, because that is the claim the fragment exists for.
//! 3. **One real input reaches the page and comes back out as a command.** A
//!    click and a keystroke, translated into Ultralight events, land in the
//!    lobby's own name field, and the page's own debounced handler answers with
//!    a `SetName` on the pane's own token. No test hook: the DOM, the page's
//!    listener and the bridge.
//! 4. **The pane rasterises.** A frame is copied out of the surface with a
//!    non-empty dirty region, which is the difference between a page that ran
//!    and a page that would draw nothing.
//! 5. **The console surface itself works** — criterion 3's second half, and the
//!    half a lobby screen does not prove. The pane claims a Station through the
//!    ordinary contracts, the page mounts that Station's console iframe from
//!    `gui/`, the iframe has `__updateConsole` installed (`gui/iframe-bridge.js`
//!    pushes state through exactly that), and a click on a control *inside the
//!    iframe* produces the expected `ControlSystemCorrelated` on the pane's own
//!    token, retaining the button's action correlation.
//! 6. **Two panes are storage-isolated** — criterion 2's "isolated state". Both
//!    documents come from ONE origin, this host's, so the browser's own
//!    same-origin rules do not separate them; Ultralight keys `localStorage` on
//!    the `Session` a view was created in, and a view created without one lands
//!    in the renderer's single persistent default session. Without a session per
//!    pane the two would share a store — `gui/session-token.js`'s
//!    `session-token` key included, which is the single value deciding which
//!    participant a page is.

#![cfg(all(feature = "ultralight", not(target_arch = "wasm32")))]

use std::time::{Duration, Instant};

use bevy::prelude::*;

use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::core::messages::{ClientMessage, GamePhase};
use project_phoenix::delivery::args::{ClientSource, HostArgs};
use project_phoenix::delivery::serve::{HostServer, ShutdownSignal};
use project_phoenix::native_host::panes::surface::{pump_pane, PaneSurface};
use project_phoenix::native_host::panes::ultralight::{stage_sdk, UltralightPaneSurface};
use project_phoenix::native_host::panes::{service_faults, LocalPanes, PaneFault, PaneId};
use project_phoenix::native_host::transport::{
    NativeTransport, NativeTransportLink, TransportDispatch, TransportEvent,
};
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig,
};
use vellum_ultralight::runtime::{
    MouseButton, PaneSession, PaneSpec, RuntimeOptions, UltralightRuntime,
};

/// The built client bundle. `scripts/build-client.mjs` writes it.
const CLIENT_DIR: &str = "dist";
/// The flagship scenario, and the one the curated public catalogue publishes.
const WORLD: &str = "assets/worlds/combat_test.toml";
/// A fixed seed, so nothing here is a draw from the OS.
const SEED: u64 = 20260894;
/// How long a page gets to load, join, and answer before the test gives up.
const PATIENCE: Duration = Duration::from_secs(30);
/// Pane size. Wide enough that the lobby's name field is not laid out off
/// screen, which would make the click below land on nothing.
const PANE_SIZE: (u32, u32) = (1280, 900);

/// The station this test seats its pane at, and the control it clicks.
///
/// Pinned by name rather than chosen relationally, unlike
/// `tests/native_host_panes.rs`: this test needs a console with a control that
/// is present unconditionally, produces a `ControlSystemCorrelated` with no
/// target lock or prior selection, and is idempotent. The Captain's Red Alert
/// button is that control — `gui/components/ph-red-alert.js` sends `set_red_alert` with an
/// explicit desired state, and `gui/action-map.js` turns it into
/// `ControlSystemCorrelated { target: "red-alert", … }`.
const STATION: &str = "captain";
const RED_ALERT_SYSTEM: &str = "red-alert";

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
    /// identity (an un-built checkout's, or one built from a different
    /// manifest); it does not forgive a real mismatch, and a pane is not what
    /// that pin protects.
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
            // Issue #1123's bridge-display flags. Neither claim in this file is
            // about them; `src/delivery/args.rs`'s own tests and
            // `src/native_host/bridge_profile.rs`'s cover `--setup`/`--profile`.
            setup: false,
            test_output: None,
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

/// Everything the simulation's own seam has been handed, kept for assertions.
#[derive(Clone, Default)]
struct Observed(std::sync::Arc<std::sync::Mutex<Vec<TransportEvent>>>);

impl Observed {
    fn take(&self) -> Vec<TransportEvent> {
        std::mem::take(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// A [`NativeTransport`] that records what it polled on the way past.
///
/// A poll **consumes**: the app's own `drain_native_inbound` empties the pane
/// bus every frame, so a test that also polled it would either see nothing (the
/// app got there first) or starve the simulation of the very messages it is
/// asserting reached it. Sitting between the two is the only way to assert on
/// the wire *and* on what the wire produced — and it changes nothing about the
/// path, because it forwards every event unmodified.
struct Observing<T: NativeTransport> {
    inner: T,
    seen: Observed,
}

impl<T: NativeTransport> NativeTransport for Observing<T> {
    fn poll(&mut self) -> Vec<TransportEvent> {
        let events = self.inner.poll();
        self.seen
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(events.iter().cloned());
        events
    }

    fn dispatch(&mut self, dispatch: TransportDispatch<'_>) {
        self.inner.dispatch(dispatch);
    }

    fn name(&self) -> &'static str {
        "observed-panes"
    }
}

#[test]
#[ignore = "needs the Ultralight SDK and a built client bundle; CI has neither. \
            Run: node scripts/build-client.mjs && cargo test --features ultralight \
            --test native_host_pane_ultralight -- --ignored --nocapture"]
fn two_panes_load_real_console_pages_join_operate_a_station_and_share_no_storage() {
    let index = std::path::Path::new(CLIENT_DIR)
        .join("client")
        .join("index.html");
    let html = std::fs::read_to_string(&index).unwrap_or_else(|e| {
        panic!(
            "{} is missing ({e}) — run `node scripts/build-client.mjs` first; a pane loads the \
             BUILT bundle's page, which is the whole point of this test",
            index.display()
        )
    });
    let delivery = Delivery::start();

    // A distinctive name keeps the privacy assertion independent of bundle
    // identifiers such as OperatorSurfaceAdapter, which happen to contain Ada.
    const ADA_NAME: &str = "Ada-private-pane-fixture";
    let panes = LocalPanes::open(&[ADA_NAME.to_string(), "Grace".to_string()], &delivery.addr);
    let bus = panes.bus.clone();
    let ada = panes.opened[0].id;
    let ada_token = panes.opened[0].identity.token().to_string();
    let views = panes.views();
    panes
        .publish(&html, &delivery.documents)
        .expect("the client page becomes a pane document");

    // ── 2a: the identity is in the URL, and in no byte this host serves ─────
    let ada_path = bus
        .document_path(ada)
        .expect("the pane's document is published");
    let served = delivery
        .documents
        .get(&ada_path)
        .expect("and readable back out");
    assert!(
        !served.contains(&ada_token),
        "a pane's session token must not appear in a body this host serves"
    );
    assert!(
        !served.contains(ADA_NAME),
        "nor must the participant name it joins under"
    );
    assert!(
        views[0].1.contains(&ada_token),
        "it rides in the URL fragment instead"
    );

    // The authoritative half. `--solo` so the mission is already running: a
    // crewed start hands off through `Loading`, whose last step is the asset
    // preloader's, and this profile composes the render *contract* rather than
    // a renderer.
    let preload = preload_content_templates(".").expect("the repository's own content preloads");
    let mut cfg = NativeHostConfig::new(WORLD);
    cfg.seed = Some(SEED);
    cfg.solo = true;
    cfg.surface = NativeRenderSurface::Contract;
    cfg.panes = Some(panes);
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");
    // This test drives its own views; the app's pane display plugin would try
    // to build a SECOND Ultralight runtime, and there is only one per process.
    // It is already inert without a primary window — `NativeRenderSurface::
    // Contract` has none — and removing the resource makes that explicit rather
    // than incidental.
    app.world_mut()
        .remove_resource::<project_phoenix::native_host::panes::ultralight::PaneDisplayConfig>();
    // Replace the link the builder installed with the same transport behind a
    // recorder. Nothing about the path changes; see `Observing`.
    let observed = Observed::default();
    app.insert_resource(NativeTransportLink::new(Observing {
        inner: bus.transport(),
        seen: observed.clone(),
    }));

    // The SDK is linked but not staged; on Windows a missing shared library is
    // a process that dies with an OS code and no message at all.
    println!("[pane] {}", stage_sdk().expect("the Ultralight SDK stages"));

    let runtime = UltralightRuntime::start(&RuntimeOptions::default())
        .expect("the Ultralight runtime starts");
    let mut surfaces: Vec<(PaneId, UltralightPaneSurface)> = Vec::new();
    for (id, url) in &views {
        let view = runtime
            .create_pane(&PaneSpec {
                width: PANE_SIZE.0,
                height: PANE_SIZE.1,
                device_scale: 1.0,
                transparent: false,
                session: Some(PaneSession::ephemeral(id.to_string())),
            })
            .expect("the pane's view is created");
        assert_eq!(
            view.session_name(),
            Some(id.to_string().as_str()),
            "each pane runs in a storage session of its own, named after the pane"
        );
        let mut surface = UltralightPaneSurface::new(view);
        println!("[pane] {id} loading {url}");
        surface.load(url).expect("the pane navigates");
        surfaces.push((*id, surface));
    }
    // Ultralight drops input into an unfocused view, and the clicks below are
    // Ada's.
    surfaces[0].1.view_mut().focus();

    pump(&mut app, 8);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "a solo host is flying the mission before anybody joins"
    );

    // One frame of everything: the library, every pane's traffic in both
    // directions, the simulation, and the rasteriser. Returns what Ada said.
    let frame = |app: &mut App,
                 surfaces: &mut Vec<(PaneId, UltralightPaneSurface)>|
     -> Vec<ClientMessage> {
        runtime.update();
        for (id, surface) in surfaces.iter_mut() {
            surface.refresh_loaded();
            pump_pane(&bus, *id, surface);
        }
        runtime.render();
        app.update();
        let mut seen = Vec::new();
        for event in observed.take() {
            if let TransportEvent::Received { msg, token } = event {
                if token == ada_token {
                    seen.push(msg);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(8));
        seen
    };

    // ── 1 + 2b: the pages load and join by themselves ───────────────────────
    //
    // NOTE: the transport poll above is the app's, so these are observed on
    // their way past rather than intercepted — the app is seeing the same
    // events and seating the sessions from them.
    let mut identify: Option<ClientMessage> = None;
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && identify.is_none() {
        for msg in frame(&mut app, &mut surfaces) {
            if matches!(msg, ClientMessage::Identify { .. }) {
                identify = Some(msg);
            }
        }
    }
    let identify = identify.unwrap_or_else(|| {
        panic!(
            "the pane's page did not identify within {PATIENCE:?}. It loaded {}; check the host \
             is serving the bundle and that `ultralight.log` has no page errors",
            views[0].1
        )
    });
    match &identify {
        ClientMessage::Identify {
            token: presented,
            name,
        } => {
            assert_eq!(
                presented, &ada_token,
                "the page identifies with the token the HOST minted — which it read out of the \
                 URL fragment, not out of the document"
            );
            assert_eq!(name, ADA_NAME, "and under the name the operator gave");
        }
        other => panic!("expected an Identify, got {other:?}"),
    }
    println!("[pane] joined as Ada on {}…", &ada_token[..8]);

    // ── 4: it rasterised ────────────────────────────────────────────────────
    let mut pixels = vec![0u8; (PANE_SIZE.0 * PANE_SIZE.1 * 4) as usize];
    let mut painted = None;
    for _ in 0..120 {
        frame(&mut app, &mut surfaces);
        if let Ok(Some(dirty)) = surfaces[0].1.view_mut().copy_frame(&mut pixels, false) {
            painted = Some(dirty);
            break;
        }
    }
    let painted = painted.expect("the pane repainted at least once");
    assert!(
        painted.pixel_count() > 0,
        "a repaint that covers no pixels is a page that would draw nothing"
    );
    assert!(
        pixels.iter().any(|b| *b != 0),
        "the copied frame is entirely zero — the surface produced no image"
    );
    println!("[pane] painted {} pixels", painted.pixel_count());

    // ── 3: one real input, in and out ───────────────────────────────────────
    //
    // The lobby's name field. Located by asking the page where it is rather than
    // by pinning a coordinate, so a layout change moves the click with it.
    let rect = surfaces[0]
        .1
        .view_mut()
        .evaluate(
            "(function(){var e=document.getElementById('name-input');\
             if(!e) return '';var r=e.getBoundingClientRect();\
             if(r.width<1||r.height<1) return '';\
             return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()",
        )
        .expect("the page answers a script");
    assert!(
        !rect.is_empty(),
        "the lobby's #name-input is not laid out — the page did not reach the lobby view"
    );
    let (x, y) = parse_point(&rect);
    println!("[pane] clicking #name-input at {x},{y}");
    click(&runtime, &mut surfaces[0].1, x, y);
    surfaces[0].1.view_mut().key_char("Z");

    // The page debounces its name field by half a second before sending, so this
    // is a wait on the PAGE's own behaviour, not on ours.
    let mut renamed = None;
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && renamed.is_none() {
        for msg in frame(&mut app, &mut surfaces) {
            if let ClientMessage::SetName { name } = msg {
                renamed = Some(name);
            }
        }
    }
    let renamed = renamed.unwrap_or_else(|| {
        panic!(
            "a click and a keystroke did not reach the page's own name field within {PATIENCE:?}"
        )
    });
    assert!(
        renamed.contains('Z'),
        "the typed character must reach the command the page sends: got {renamed:?}"
    );
    println!("[pane] one input round-tripped: SetName {renamed:?}");

    // ── 5: the console surface, which is the half a lobby screen does not
    //       prove ──────────────────────────────────────────────────────────
    //
    // Claimed through the ordinary contracts, one message per pumped batch:
    // the lobby's per-variant systems share one FixedUpdate set and are not
    // chained, so a `SelectStation` handled before its `Identify` is silently
    // ignored. A real participant never does that either.
    bus.submit(
        ada,
        ClientMessage::SelectStation {
            station: STATION.to_string(),
        },
    )
    .expect("a pane may claim a station");
    for _ in 0..8 {
        frame(&mut app, &mut surfaces);
    }
    bus.submit(ada, ClientMessage::SetReady { ready: true })
        .expect("a pane may ready");
    for _ in 0..8 {
        frame(&mut app, &mut surfaces);
    }
    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    let seated = sessions
        .0
        .players()
        .iter()
        .find(|p| p.token == ada_token)
        .and_then(|p| p.station.as_ref().map(|s| s.0.clone()));
    assert_eq!(
        seated.as_deref(),
        Some(STATION),
        "the pane must hold the Station it claimed before its console can mean anything"
    );

    // The page mounts that Station's console as an iframe of the ordinary
    // `gui/` page — `planMounts` → `resolveConsoleUrl`, resolved relative to
    // the pane's own document, which is why the document sits at the client
    // directory's own depth.
    let mut mounted = String::new();
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && mounted.is_empty() {
        frame(&mut app, &mut surfaces);
        mounted = surfaces[0]
            .1
            .view_mut()
            .evaluate(
                "(function(){var f=document.getElementById('captain-iframe');\
                 if(!f||!f.src) return '';\
                 if(!f.contentWindow||typeof f.contentWindow.__updateConsole!=='function') \
                 return '';\
                 return f.src;})()",
            )
            .unwrap_or_default();
    }
    assert!(
        !mounted.is_empty(),
        "the captain console iframe never appeared with __updateConsole installed — \
         gui/iframe-bridge.js pushes every state snapshot through exactly that function, \
         so a console without it is a console that can never be told anything"
    );
    assert!(
        mounted.contains("gui/") && mounted.ends_with(".html"),
        "the console must be the ordinary gui/ page a phone loads: {mounted}"
    );
    println!("[pane] console mounted: {mounted}");

    // And a click on a control INSIDE that iframe becomes a command. The Red
    // Alert button lives in a shadow root, so this reaches through the iframe
    // and the shadow boundary and converts to the pane's own coordinates.
    let mut rect = String::new();
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && rect.is_empty() {
        frame(&mut app, &mut surfaces);
        rect = surfaces[0]
            .1
            .view_mut()
            .evaluate(
                "(function(){var f=document.getElementById('captain-iframe');\
                 if(!f||!f.contentDocument) return '';\
                 var host=f.contentDocument.getElementById('red-alert');\
                 if(!host||!host.shadowRoot) return '';\
                 var b=host.shadowRoot.getElementById('alert-btn');\
                 if(!b||b.disabled) return '';\
                 b.scrollIntoView();\
                 var fr=f.getBoundingClientRect(), r=b.getBoundingClientRect();\
                 if(r.width<1||r.height<1||fr.width<1||fr.height<1) return '';\
                 var px=Math.round(fr.left+r.left+r.width/2);\
                 var py=Math.round(fr.top+r.top+r.height/2);\
                 var over=document.elementFromPoint(px,py);\
                 var tag=over?(over.tagName+'#'+(over.id||'')):'nothing';\
                 return px+','+py+','+tag;})()",
            )
            .unwrap_or_default();
    }
    if rect.is_empty() {
        // Four different failures land here and they read identically from
        // outside, so the readout separates them. Each of the last three has
        // been a real one:
        //
        //   * the button is DISABLED — the station is still on Backfill, so the
        //     seat never took;
        //   * the page cannot find ITSELF in the roster (`ui.me=null`) — a token
        //     question, not a seating one: the page matches on `myToken`, which
        //     `gui/session-token.js` resolves from storage, while the pane's
        //     transport identifies with the token the host minted;
        //   * the link is down or nothing is being delivered (`link=false`,
        //     `deliver=object`, a growing `inbox`);
        //   * `pendingFrame` is non-null with everything else correct — the page
        //     asked for a render and its `requestAnimationFrame` never fired.
        //     An offscreen Ultralight view services rAF only inside a rendering
        //     update and only runs one when something is dirty, so a page whose
        //     only pending work IS that render deadlocks against itself.
        //     `pane_boot.js` drives rAF off a timer for exactly this reason;
        //     seeing it here again means that override did not take.
        let state = surfaces[0]
            .1
            .view_mut()
            .evaluate(
                "(function(){\
                 var lob=document.getElementById('lobby-ui');\
                 var sec=document.getElementById('captain-ui');\
                 var f=document.getElementById('captain-iframe');\
                 var r=f?f.getBoundingClientRect():null;\
                 var ls=window.lobbyState;\
                 var roster=ls&&ls.players?JSON.stringify(ls.players.map(function(p){\
                 return {n:p.name,t:p.token,s:p.station,r:p.ready,sp:p.spectator};})):\
                 'no lobbyState';\
                 var pane=window.__phoenixPane||{};\
                 var mine='<page scope unreadable>';\
                 try{mine='myToken='+String(myToken)\
                 +' claim='+String(pendingMidGameClaim)\
                 +' active='+String(activeConsole)\
                 +' pendingFrame='+String(_renderFrame)\
                 +' ui='+JSON.stringify({ph:uiState.phase,n:(uiState.players||[]).length,\
                 me:(uiState.players||[]).find(function(p){return p.token===myToken;})||null});}\
                 catch(e){mine='<page scope unreadable: '+e+'>';}\
                 return 'phase='+(ls?ls.phase:'?')+' roster='+roster\
                 +' '+mine\
                 +' pane.token='+String(pane.token)\
                 +' link='+String(window.phoenixLink&&window.phoenixLink.connected)\
                 +' deliver='+(typeof pane.deliver)+' inbox='+((pane.inbox||[]).length)\
                 +' lobby-ui.class='+(lob?(lob.className||'(empty)'):'absent')\
                 +' captain-ui.display='+(sec?getComputedStyle(sec).display:'absent')\
                 +' iframe='+(r?(r.width+'x'+r.height):'absent');})()",
            )
            .unwrap_or_else(|e| format!("<unreadable: {e}>"));
        panic!(
            "the captain console's Red Alert button is not laid out or is disabled. The page \
             says: {state}"
        );
    }
    let mut fields = rect.split(',');
    let x: i32 = fields.next().and_then(|v| v.parse().ok()).unwrap_or(-1);
    let y: i32 = fields.next().and_then(|v| v.parse().ok()).unwrap_or(-1);
    let over = fields.next().unwrap_or("").to_string();
    assert!(
        x >= 0 && y >= 0 && (x as u32) < PANE_SIZE.0 && (y as u32) < PANE_SIZE.1,
        "the button is outside the pane at {x},{y}; the click would land on nothing"
    );
    // Hit-testing from the PAGE's side, because that is what a mouse event
    // does: the point has to land on the console's iframe, or the click reaches
    // whatever is stacked over it instead.
    assert!(
        over.starts_with("IFRAME#captain-iframe"),
        "the point {x},{y} over the Red Alert button hit {over} rather than the console's \
         iframe — a click there would go somewhere else entirely"
    );
    // Record what the console iframe posts up to the page, so a failure below
    // says WHERE the path broke: no message at all is a click that never
    // reached the button, and a `console_action` with no correlated command
    // after it is `gui/action-map.js` or the injected link.
    surfaces[0]
        .1
        .view_mut()
        .evaluate(
            "window.__paneProbe=[];window.addEventListener('message',function(e){\
             try{window.__paneProbe.push(JSON.stringify(e.data));\
             if(e.data.type==='console_action'){var a=JSON.parse(e.data.payload);\
             if(a.action==='set_red_alert')window.__paneRedAlertCorrelation=a.correlation;}\
             }catch(_){}});'ok'",
        )
        .expect("the page takes a probe");

    println!("[pane] clicking the console's Red Alert at {x},{y}");
    click(&runtime, &mut surfaces[0].1, x, y);

    let mut commanded = None;
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && commanded.is_none() {
        for msg in frame(&mut app, &mut surfaces) {
            if let ClientMessage::ControlSystemCorrelated {
                correlation,
                target,
                payload,
            } = msg
            {
                commanded = Some((correlation, target, payload));
            }
        }
    }
    let (correlation, target, payload) = commanded.unwrap_or_else(|| {
        let probe = surfaces[0]
            .1
            .view_mut()
            .evaluate("String((window.__paneProbe||[]).join(' | '))")
            .unwrap_or_else(|e| format!("<probe unreadable: {e}>"));
        let audio = surfaces[0]
            .1
            .view_mut()
            .evaluate(
                "(function(){try{var el=document.getElementById('ui-click');\
                 if(!el)return 'no #ui-click';el.currentTime=0;el.playbackRate=1;\
                 var p=el.play();return 'played '+Object.prototype.toString.call(p);}\
                 catch(e){return 'threw: '+e;}})()",
            )
            .unwrap_or_else(|e| format!("<unreadable: {e}>"));
        panic!(
            "a click inside the console iframe produced no ControlSystemCorrelated — the console's own \
             postMessage, gui/action-map.js and the injected link are the path it takes. \
             The page received: {probe}. The page's UI click sound: {audio}"
        )
    });
    let posted_correlation = surfaces[0]
        .1
        .view_mut()
        .evaluate("String(window.__paneRedAlertCorrelation || '')")
        .expect("the console action's correlation can be inspected");
    assert_eq!(
        correlation.as_str(),
        posted_correlation,
        "the host must receive the correlation minted by the clicked console control"
    );
    assert_eq!(
        target.0, RED_ALERT_SYSTEM,
        "the command must name the system the clicked control drives"
    );
    assert!(
        matches!(
            payload,
            project_phoenix::core::messages::SystemControlPayload::SetRedAlert { active: true }
        ),
        "the Captain's console sends an explicit desired state: got {payload:?}"
    );
    println!(
        "[pane] console click round-tripped: ControlSystemCorrelated {{ target: {target:?} }}"
    );

    // ── 6: the two panes share no storage ───────────────────────────────────
    //
    // Both documents came from ONE origin — this host's — so nothing the
    // browser does separates them. The `PaneSession` per pane is what does.
    assert!(
        surfaces[1].1.is_ready(),
        "the second pane's document must have loaded before storage means anything"
    );
    surfaces[0]
        .1
        .view_mut()
        .evaluate("localStorage.setItem('pane-isolation-probe', 'ada')")
        .expect("the first pane writes to its own storage");
    for _ in 0..10 {
        frame(&mut app, &mut surfaces);
    }
    assert_eq!(
        surfaces[0]
            .1
            .view_mut()
            .evaluate("String(localStorage.getItem('pane-isolation-probe'))")
            .expect("the first pane reads its own storage"),
        "ada",
        "a pane can read what it wrote"
    );
    assert_eq!(
        surfaces[1]
            .1
            .view_mut()
            .evaluate("String(localStorage.getItem('pane-isolation-probe'))")
            .expect("the second pane reads its own storage"),
        "null",
        "one pane's localStorage must be invisible to another; they are separate \
         participants sharing nothing but an HTTP origin"
    );
    println!("[pane] two panes, two stores");

    // ── 7: a crashed pane recovers as an ordinary reconnect (issue #1125) ─────
    //
    // Ada holds the captain's chair (section 5). Injecting a view crash at the
    // seam — the same signal `drive_panes`' frame-copy watchdog raises for a real
    // crashed view — disconnects her through the ordinary session path (the chair
    // falls to Backfill) and recreates her pane on the SAME token. A fresh real
    // view loads the same console, rejoins by itself, and reconnect-yield hands
    // the chair back to the human. This is the real-view half of #1125's
    // recreation criterion; the SDK-free half is `tests/native_host_panes.rs`.
    let captain = project_phoenix::core::messages::StationId(STATION.to_string());
    let holder = |app: &App| {
        app.world()
            .resource::<project_phoenix::lobby::Sessions>()
            .0
            .holder_for_station(&captain)
            .map(str::to_string)
    };
    assert_eq!(
        holder(&app).as_deref(),
        Some(ada_token.as_str()),
        "Ada holds the captain's chair before the crash"
    );

    bus.fault(ada, PaneFault::ViewCrashed);
    let outcomes = service_faults(&bus);
    let (recreated, recreated_url) = outcomes
        .iter()
        .find(|o| o.failed == ada)
        .and_then(|o| o.recreated.clone())
        .expect("a view crash recreates Ada's pane on the same identity");
    assert_eq!(
        bus.token_of(recreated).as_deref(),
        Some(ada_token.as_str()),
        "the recreated pane carries Ada's own token"
    );

    // Drop the crashed view and drive frames so the disconnect reaches the lobby:
    // the chair falls to Backfill and resolves to no connected holder.
    surfaces.retain(|(id, _)| *id != ada);
    for _ in 0..8 {
        frame(&mut app, &mut surfaces);
    }
    assert!(
        holder(&app).is_none(),
        "while Ada's pane is gone, the captain's chair is on Backfill"
    );

    // Build the recreated view — the in-process analogue of a phone reloading —
    // and let it rejoin on its own from the fresh document `recreate` published.
    let recreated_view = runtime
        .create_pane(&PaneSpec {
            width: PANE_SIZE.0,
            height: PANE_SIZE.1,
            device_scale: 1.0,
            transparent: false,
            session: Some(PaneSession::ephemeral(recreated.to_string())),
        })
        .expect("the recreated pane's view is created");
    let mut recreated_surface = UltralightPaneSurface::new(recreated_view);
    println!("[pane] {recreated} reloading {recreated_url} to reconnect as Ada");
    recreated_surface
        .load(&recreated_url)
        .expect("the recreated pane navigates");
    surfaces.push((recreated, recreated_surface));

    let deadline = Instant::now() + PATIENCE;
    let mut reconnected = false;
    while Instant::now() < deadline && !reconnected {
        frame(&mut app, &mut surfaces);
        reconnected = holder(&app).as_deref() == Some(ada_token.as_str());
    }
    assert!(
        reconnected,
        "the recreated pane must rejoin on Ada's token and reclaim the captain's chair — \
         the pane analogue of a phone reconnecting on its saved session"
    );
    println!("[pane] a crashed pane recovered and reconnected on the same identity");
}

/// Pump `app` for `frames` frames of fixed virtual time, as the headless
/// harness does.
fn pump(app: &mut App, frames: u64) {
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    ));
    app.finish();
    app.cleanup();
    for _ in 0..frames {
        app.update();
    }
}

/// `"x,y"` from a page script into a coordinate pair.
fn parse_point(rect: &str) -> (i32, i32) {
    rect.split_once(',')
        .map(|(a, b)| (a.parse::<i32>().unwrap(), b.parse::<i32>().unwrap()))
        .unwrap_or_else(|| panic!("expected a coordinate pair, got {rect:?}"))
}

/// A full click at page coordinates, with the frames Ultralight needs either
/// side of it.
///
/// The move is not optional: Ultralight decides what is under the pointer from
/// the *move*, so a press with no preceding one lands on whatever was hovered
/// last.
fn click(runtime: &UltralightRuntime, surface: &mut UltralightPaneSurface, x: i32, y: i32) {
    surface.view_mut().mouse_move(x, y);
    for _ in 0..10 {
        runtime.update();
        runtime.render();
        std::thread::sleep(Duration::from_millis(8));
    }
    surface.view_mut().mouse_down(x, y, MouseButton::Left);
    surface.view_mut().mouse_up(x, y, MouseButton::Left);
    for _ in 0..10 {
        runtime.update();
        runtime.render();
        std::thread::sleep(Duration::from_millis(8));
    }
}
