//! A real console page, in a real Ultralight pane, answering a real host
//! (issue #1122, acceptance criterion 3).
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
//! # What it asserts, and why that is the right assertion
//!
//! Everything about a pane that only a browser engine can settle, and nothing
//! that does not need one — the contracts are `tests/native_host_panes.rs`'s,
//! run by the ordinary `cargo test`. Here:
//!
//! 1. **The real page loads over HTTP from this process's own delivery server.**
//!    Not an `include_str!`'d document: the pane navigates to a URL the
//!    `HostServer` publishes, at the client directory's own depth, so every
//!    relative `gui/` module and console iframe resolves exactly as it does for
//!    a phone.
//! 2. **The injected link replaces PeerJS and the page joins by itself.** The
//!    first outbound `ClientMessage` a pane produces is an `Identify` carrying
//!    the token the *host* minted — which is the whole of "in-process delivery
//!    uses the same logical-client boundaries".
//! 3. **One real input reaches the page and comes back out as a command.** A
//!    click and a keystroke, translated into Ultralight events, land in the
//!    lobby's own name field, and the page's own debounced handler answers with
//!    a `SetName` on the pane's own token. No test hook: the DOM, the page's
//!    listener and the bridge.
//! 4. **The pane rasterises.** A frame is copied out of the surface with a
//!    non-empty dirty region, which is the difference between a page that ran
//!    and a page that would draw nothing.

#![cfg(all(feature = "ultralight", not(target_arch = "wasm32")))]

use std::time::{Duration, Instant};

use project_phoenix::core::messages::ClientMessage;
use project_phoenix::delivery::args::{ClientSource, HostArgs};
use project_phoenix::delivery::serve::{HostServer, ShutdownSignal};
use project_phoenix::native_host::panes::document::pane_url;
use project_phoenix::native_host::panes::surface::{pump_pane, PaneSurface};
use project_phoenix::native_host::panes::ultralight::{stage_sdk, UltralightPaneSurface};
use project_phoenix::native_host::panes::LocalPanes;
use project_phoenix::native_host::transport::{NativeTransport, TransportEvent};
use vellum_ultralight::runtime::{MouseButton, PaneSpec, RuntimeOptions, UltralightRuntime};

/// The built client bundle. `scripts/build-client.mjs` writes it.
const CLIENT_DIR: &str = "dist";
/// How long a page gets to load, join, and answer before the test gives up.
const PATIENCE: Duration = Duration::from_secs(30);
/// Pane size. Wide enough that the lobby's name field is not laid out off
/// screen, which would make the click below land on nothing.
const PANE_SIZE: (u32, u32) = (1280, 900);

#[test]
#[ignore = "needs the Ultralight SDK and a built client bundle; CI has neither. \
            Run: node scripts/build-client.mjs && cargo test --features ultralight \
            --test native_host_pane_ultralight -- --ignored --nocapture"]
fn a_pane_loads_the_real_console_page_joins_and_answers_one_input() {
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

    // A delivery host on a loopback port of the OS's choosing, serving the real
    // bundle. `skip_bundle_check` forgives a bundle with no `[content]` identity
    // (an un-built checkout's, or one built from a different manifest); it does
    // not forgive a real mismatch, and a pane is not what that pin protects.
    let args = HostArgs {
        addr: "127.0.0.1:0".to_string(),
        client: ClientSource::Bundled {
            dir: CLIENT_DIR.to_string(),
        },
        manifest: "assets/scenarios.toml".to_string(),
        content_dir: ".".to_string(),
        skip_bundle_check: true,
        sim: None,
    };
    let server = HostServer::bind(&args).expect("the delivery host binds");
    let addr = server.local_addr();
    let documents = server.hosted_documents();
    server
        .enable_shutdown_polling()
        .expect("the listener can be polled for shutdown");
    let shutdown = ShutdownSignal::new();
    let serving = shutdown.clone();
    let delivery = std::thread::spawn(move || {
        let _ = server.serve_until(serving, |_| {});
    });

    let panes = LocalPanes::open(&["Ada".to_string()], &addr);
    let bus = panes.bus.clone();
    let pane = panes.opened[0].id;
    let token = panes.opened[0].identity.token().to_string();
    panes
        .publish(&html, &documents)
        .expect("the client page becomes a pane document");

    // The SDK is linked but not staged; on Windows a missing shared library is
    // a process that dies with an OS code and no message at all.
    println!("[pane] {}", stage_sdk().expect("the Ultralight SDK stages"));

    let runtime = UltralightRuntime::start(&RuntimeOptions::default())
        .expect("the Ultralight runtime starts");
    let view = runtime
        .create_pane(&PaneSpec {
            width: PANE_SIZE.0,
            height: PANE_SIZE.1,
            device_scale: 1.0,
            transparent: false,
        })
        .expect("the pane's view is created");
    let mut surface = UltralightPaneSurface::new(view);
    let url = pane_url(&addr, pane);
    println!("[pane] loading {url}");
    surface.load(&url).expect("the pane navigates");
    surface.view_mut().focus();

    // ── 1 + 2: the page loads and joins by itself ────────────────────────────
    let mut identify: Option<ClientMessage> = None;
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && identify.is_none() {
        runtime.update();
        surface.refresh_loaded();
        pump_pane(&bus, pane, &mut surface);
        runtime.render();
        for event in bus.transport().poll() {
            if let TransportEvent::Received { msg, token: seen } = event {
                assert_eq!(seen, token, "a pane speaks on its own token and no other");
                if matches!(msg, ClientMessage::Identify { .. }) {
                    identify = Some(msg);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    let identify = identify.unwrap_or_else(|| {
        panic!(
            "the pane's page did not identify within {PATIENCE:?}. It loaded {url}; check the \
             host is serving the bundle and that `ultralight.log` has no page errors"
        )
    });
    match &identify {
        ClientMessage::Identify {
            token: presented,
            name,
        } => {
            assert_eq!(
                presented, &token,
                "the page identifies with the token the HOST minted, not one of its own"
            );
            assert_eq!(name, "Ada", "and under the name the operator gave");
        }
        other => panic!("expected an Identify, got {other:?}"),
    }
    println!("[pane] joined as Ada on {}…", &token[..8]);

    // ── 4: it rasterised ────────────────────────────────────────────────────
    let mut pixels = vec![0u8; (PANE_SIZE.0 * PANE_SIZE.1 * 4) as usize];
    let mut painted = None;
    for _ in 0..120 {
        runtime.update();
        runtime.render();
        if let Ok(Some(dirty)) = surface.view_mut().copy_frame(&mut pixels, false) {
            painted = Some(dirty);
            break;
        }
        std::thread::sleep(Duration::from_millis(8));
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
    let rect = surface
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
    let (x, y) = rect
        .split_once(',')
        .map(|(a, b)| (a.parse::<i32>().unwrap(), b.parse::<i32>().unwrap()))
        .expect("a coordinate pair");
    println!("[pane] clicking #name-input at {x},{y}");
    surface.view_mut().mouse_move(x, y);
    surface.view_mut().mouse_down(x, y, MouseButton::Left);
    surface.view_mut().mouse_up(x, y, MouseButton::Left);
    for _ in 0..10 {
        runtime.update();
        runtime.render();
        std::thread::sleep(Duration::from_millis(8));
    }
    surface.view_mut().key_char("Z");

    // The page debounces its name field by half a second before sending, so this
    // is a wait on the PAGE's own behaviour, not on ours.
    let mut renamed = None;
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && renamed.is_none() {
        runtime.update();
        pump_pane(&bus, pane, &mut surface);
        runtime.render();
        for event in bus.transport().poll() {
            if let TransportEvent::Received {
                msg: ClientMessage::SetName { name },
                token: seen,
            } = event
            {
                assert_eq!(seen, token);
                renamed = Some(name);
            }
        }
        std::thread::sleep(Duration::from_millis(8));
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

    shutdown.stop();
    let _ = delivery.join();
}
