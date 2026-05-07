//! Client-side WASM/JS bridge.
//!
//! This module is compiled when the `client` Cargo feature is enabled and is
//! the entry point for the Trunk-built `client.html` WASM bundle. It mirrors
//! the structure of `bridge.rs` (the server bridge) but the surface is
//! deliberately smaller and reflects the *player's* view of the world:
//!
//!   - Inbound traffic from JS is `ServerMessage` JSON (the host pushes
//!     state to the player), arriving via `wasm_client_receive`.
//!   - Outbound traffic to JS is `ClientMessage` JSON (the player pushes
//!     intent to the host), pushed via the callback registered with
//!     `set_client_send_callback`.
//!   - The player's display name arrives via `wasm_client_set_name` from
//!     the page's `<input>` and is held in a thread-local resource for any
//!     future Bevy systems that need it.
//!
//! On native targets this module exists but the wasm-bindgen exports are
//! gated behind `cfg(target_arch = "wasm32")` so `cargo check --features
//! client` still passes off-WASM.

#[cfg(target_arch = "wasm32")]
use {
    bevy::{prelude::*, DefaultPlugins},
    js_sys::Function,
    std::cell::RefCell,
    wasm_bindgen::prelude::*,
};

// ── Thread-local state ─────────────────────────────────────────────────────
//
// WASM is single-threaded; RefCell is safe here.

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// Inbound `ServerMessage` JSON payloads queued by JS, waiting to be
    /// drained into Bevy by `drain_inbound`.
    static INBOUND_QUEUE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

    /// The player's chosen display name, updated each time JS calls
    /// `wasm_client_set_name`. Defaults to an empty string until set.
    static PLAYER_NAME: RefCell<String> = const { RefCell::new(String::new()) };

    /// JS callback registered by the host page to forward outbound
    /// `ClientMessage` JSON back to the server peer over PeerJS.
    /// Signature: `callback(payload: string)`.
    static OUTBOUND_CB: RefCell<Option<Function>> = const { RefCell::new(None) };
}

// ── Placeholder plugin ─────────────────────────────────────────────────────
//
// The full client renderer (radar, helm, console UI, etc.) lands in later
// issues. For now this is an empty plugin so the page boots cleanly and
// later issues have a stable insertion point.

/// Placeholder plugin that the client-side Bevy app can extend in later
/// issues. Currently empty — it merely exists so `wasm_client_init` has a
/// stable, single seam to add per-screen renderers under.
pub struct ClientRendererPlugin;

#[cfg(target_arch = "wasm32")]
impl Plugin for ClientRendererPlugin {
    fn build(&self, _app: &mut App) {
        // Intentionally empty — see #46.
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ClientRendererPlugin {
    /// Native stub — the client app only ever runs in WASM. Exists so that
    /// `cargo check --features client` succeeds on the host triple too.
    pub fn new() -> Self { Self }
}

// ── Public WASM API ────────────────────────────────────────────────────────

/// Called by JS on page load. Builds and runs the client Bevy app with
/// `DefaultPlugins` (3D, windowing, input, etc.) plus `ClientRendererPlugin`.
///
/// In WASM `App::run()` hands control to requestAnimationFrame and returns
/// immediately, so this function does not block.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_client_init() {
    App::new()
        .add_plugins(DefaultPlugins.set(bevy::window::WindowPlugin {
            primary_window: Some(bevy::window::Window {
                canvas: Some("#canvas".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(ClientRendererPlugin)
        .add_systems(Update, drain_inbound)
        .run();
}

/// Called by JS to deliver a `ServerMessage` JSON payload from the host
/// peer. The payload is queued and drained into Bevy on the next frame.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_client_receive(json: &str) {
    INBOUND_QUEUE.with(|q| {
        q.borrow_mut().push(json.to_string());
    });
}

/// Called by JS when the player edits the name `<input>`. The new name is
/// stored in the thread-local `PLAYER_NAME` slot so later issues (Identify
/// dispatch, on-screen labels) can read it without re-querying the DOM.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_client_set_name(name: &str) {
    PLAYER_NAME.with(|n| {
        *n.borrow_mut() = name.to_string();
    });
}

/// Called by JS to register the outbound message callback. Bevy systems
/// that want to send a `ClientMessage` to the host peer build the JSON
/// themselves and invoke this callback via the helper in this module.
///
/// Signature expected from JS: `callback(payload: string)`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_client_send_callback(callback: Function) {
    OUTBOUND_CB.with(|slot| {
        *slot.borrow_mut() = Some(callback);
    });
}

// ── Bevy bridge systems ────────────────────────────────────────────────────

/// Drains the inbound queue each frame. Decoding into a typed
/// `ServerMessage` and routing into Bevy events lands in later issues; for
/// now we only clear the queue so it cannot grow unbounded.
#[cfg(target_arch = "wasm32")]
fn drain_inbound() {
    INBOUND_QUEUE.with(|q| {
        q.borrow_mut().clear();
    });
}

// ── Native-side helpers (test surface) ─────────────────────────────────────
//
// These exist so that on native targets we can still write basic unit tests
// against the (very small) state-shape of the client bridge without dragging
// in wasm-bindgen.

#[cfg(not(target_arch = "wasm32"))]
impl Default for ClientRendererPlugin {
    fn default() -> Self { Self }
}
