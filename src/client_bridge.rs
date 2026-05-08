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
//!   - The player's display name arrives via `wasm_client_set_name`.
//!   - The player's session token arrives via `wasm_client_set_token`
//!     (used by the lobby UI to highlight the local player's selection).
//!
//! On native targets this module exists but the wasm-bindgen exports are
//! gated behind `cfg(target_arch = "wasm32")` so `cargo check --features
//! client` still passes off-WASM.

#[cfg(target_arch = "wasm32")]
use {
    crate::client_app::{ClientAppPlugin, InboundServerMessage, OutboundClientMessage},
    crate::client_lobby::{ActiveConsole, LocalPlayerToken},
    crate::messages::Console,
    crate::codec::{JsonCodec, MessageCodec},
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
    /// decoded and forwarded into Bevy as `InboundServerMessage` events.
    static INBOUND_QUEUE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

    /// The player's chosen display name, updated each time JS calls
    /// `wasm_client_set_name`. Defaults to an empty string until set.
    static PLAYER_NAME: RefCell<String> = const { RefCell::new(String::new()) };

    /// The player's session token, set once by JS via `wasm_client_set_token`.
    /// Empty until JS supplies it; the lobby UI treats an empty token as
    /// "I am not yet known to the server".
    static PLAYER_TOKEN: RefCell<String> = const { RefCell::new(String::new()) };

    /// The console the player is currently viewing, set by the JS tab bar via
    /// `wasm_client_set_active_console`. Empty string means "auto" (no tab override).
    static ACTIVE_CONSOLE: RefCell<String> = const { RefCell::new(String::new()) };

    /// JS callback registered by the host page to forward outbound
    /// `ClientMessage` JSON back to the server peer over PeerJS.
    /// Signature: `callback(payload: string)`.
    static OUTBOUND_CB: RefCell<Option<Function>> = const { RefCell::new(None) };
}

// ── Placeholder plugin ─────────────────────────────────────────────────────

/// Marker plugin retained for backwards compatibility with the #46 wiring;
/// new behaviour belongs in `client_app::ClientAppPlugin`. Kept as a
/// public name so other modules (or future tests) can refer to a stable
/// extension point.
pub struct ClientRendererPlugin;

#[cfg(target_arch = "wasm32")]
impl Plugin for ClientRendererPlugin {
    fn build(&self, _app: &mut App) {
        // Intentionally empty — see #46 / #47.
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ClientRendererPlugin {
    /// Native stub — the client app only ever runs in WASM. Exists so that
    /// `cargo check --features client` succeeds on the host triple too.
    pub fn new() -> Self { Self }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for ClientRendererPlugin {
    fn default() -> Self { Self }
}

// ── Public WASM API ────────────────────────────────────────────────────────

/// Called by JS on page load. Builds and runs the client Bevy app with
/// `DefaultPlugins` plus `ClientAppPlugin` (lobby UI) and `ClientRendererPlugin`.
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
        .add_plugins(ClientAppPlugin)
        .add_plugins(ClientRendererPlugin)
        .add_systems(Update, (
            forward_local_token,
            forward_active_console,
            forward_inbound_messages,
            flush_outbound_messages,
        ))
        .run();
}

/// Called by JS to deliver a `ServerMessage` JSON payload from the host
/// peer. The payload is queued and decoded next frame.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_client_receive(json: &str) {
    INBOUND_QUEUE.with(|q| {
        q.borrow_mut().push(json.to_string());
    });
}

/// Called by JS when the player edits the name `<input>`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_client_set_name(name: &str) {
    PLAYER_NAME.with(|n| {
        *n.borrow_mut() = name.to_string();
    });
}

/// Called by JS once the local player's session token is known. The token
/// is forwarded into Bevy as the `LocalPlayerToken` resource so the lobby
/// UI can highlight the player's own selection.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_client_set_token(token: &str) {
    PLAYER_TOKEN.with(|t| {
        *t.borrow_mut() = token.to_string();
    });
}

/// Called by the JS tab bar when the player switches active console.
/// `console` is the console name string (e.g. `"CaptainChair"`, `"Helm"`)
/// or `""` to clear the override (auto mode).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_client_set_active_console(console: &str) {
    ACTIVE_CONSOLE.with(|c| {
        *c.borrow_mut() = console.to_string();
    });
}

/// Called by JS to register the outbound message callback.
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

/// Pulls the latest token from the thread-local slot into Bevy's
/// `LocalPlayerToken` resource each frame. Cheap (single string compare)
/// and avoids needing a wakeup signal from JS.
#[cfg(target_arch = "wasm32")]
fn forward_local_token(mut token: ResMut<LocalPlayerToken>) {
    PLAYER_TOKEN.with(|t| {
        let latest = t.borrow();
        if latest.as_str() != token.0 {
            token.0 = latest.clone();
        }
    });
}

/// Pulls the active-console override from the thread-local slot (set by the
/// JS tab bar) into Bevy's `ActiveConsole` resource each frame.
#[cfg(target_arch = "wasm32")]
fn forward_active_console(mut active: ResMut<ActiveConsole>) {
    ACTIVE_CONSOLE.with(|c| {
        let latest = c.borrow();
        let parsed: Option<Console> = match latest.as_str() {
            "CaptainChair" => Some(Console::CaptainChair),
            "Helm"         => Some(Console::Helm),
            "Tactical"     => Some(Console::Tactical),
            "Engineering"  => Some(Console::Engineering),
            _              => None,
        };
        if active.0 != parsed {
            active.0 = parsed;
        }
    });
}

/// Drains the inbound queue, decodes each JSON payload as a
/// `ServerMessage`, and writes one `InboundServerMessage` event per
/// successful decode. Malformed payloads are silently dropped — the JS
/// side already logs decode errors at its own layer.
#[cfg(target_arch = "wasm32")]
fn forward_inbound_messages(mut writer: MessageWriter<InboundServerMessage>) {
    let pending: Vec<String> =
        INBOUND_QUEUE.with(|q| q.borrow_mut().drain(..).collect());
    for json in pending {
        if let Ok(msg) = JsonCodec.decode_server(&json) {
            writer.write(InboundServerMessage(msg));
        }
    }
}

/// Reads outbound `ClientMessage` events emitted by the lobby UI (and
/// later by other client systems), encodes each as JSON, and forwards
/// them through the JS callback registered by `set_client_send_callback`.
#[cfg(target_arch = "wasm32")]
fn flush_outbound_messages(mut reader: MessageReader<OutboundClientMessage>) {
    let payloads: Vec<String> = reader
        .read()
        .filter_map(|out| JsonCodec.encode_client(&out.0).ok())
        .collect();
    if payloads.is_empty() {
        return;
    }
    OUTBOUND_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            for payload in &payloads {
                let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(payload));
            }
        }
    });
}
