// WASM/JS bridge — all public functions are #[wasm_bindgen] exports.
//
// On native targets this module is empty; the WASM-specific code is
// gated behind #[cfg(target_arch = "wasm32")].

#[cfg(target_arch = "wasm32")]
use {
    crate::asteroid_lifecycle::AsteroidLifecyclePlugin,
    crate::codec::{JsonCodec, MessageCodec},
    crate::config_cache::ConfigCachePlugin,
    crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, PlayerDisconnected, Target},
    crate::modifier_coordination::ModifierCoordinationPlugin,
    crate::renderer::RendererPlugin,
    crate::simulation::SimulationPlugin,
    crate::world::WorldPlugin,
    crate::stations_config::ShipStations,
    crate::viewscreen_border::ViewscreenBorderPlugin,
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
    /// Messages received from JS peers, waiting to be injected into Bevy.
    /// Each entry is (sender_token, json_payload).
    static INBOUND_QUEUE: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };

    /// Disconnect tokens queued by JS, waiting to be injected into Bevy.
    static DISCONNECT_QUEUE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

    /// JS callback registered by the host page to receive outbound messages.
    /// Signature: callback(target: string, payload: string)
    static OUTBOUND_CB: RefCell<Option<Function>> = const { RefCell::new(None) };

    /// Validated ShipStations config, stored by wasm_validate_stations() so
    /// wasm_init() can insert it as a Bevy resource.
    static SHIP_STATIONS: RefCell<Option<ShipStations>> = const { RefCell::new(None) };

    /// Whether `?debug_regions=1` was specified in the URL. Set by JS via
    /// `wasm_set_debug_regions()` before `wasm_init()`.
    static DEBUG_REGIONS_ENABLED: RefCell<bool> = const { RefCell::new(false) };
}

// ── Public WASM API ────────────────────────────────────────────────────────

/// Called by JS with the raw player_ship.toml content to validate the
/// `[stations]` section before starting the server.
///
/// On success, stores the parsed `ShipStations` internally and returns
/// `Ok(JsValue::UNDEFINED)`. On failure, returns `Err(JsValue)` with a
/// human-readable error string. PeerJS should not start when this returns
/// an error.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_validate_stations(toml_str: &str) -> Result<JsValue, JsValue> {
    match crate::stations_config::parse_and_validate(toml_str) {
        Ok(stations) => {
            SHIP_STATIONS.with(|slot| {
                *slot.borrow_mut() = Some(stations);
            });
            Ok(JsValue::UNDEFINED)
        }
        Err(e) => Err(JsValue::from_str(&format!(
            "Station config validation failed: {}",
            e
        ))),
    }
}

/// Called by JS on page load. Builds and runs the Bevy app.
///
/// In WASM, `App::run()` hands control to requestAnimationFrame and returns
/// immediately, so this function does not block.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_init() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(bevy::window::WindowPlugin {
        primary_window: Some(bevy::window::Window {
            canvas: Some("#canvas".into()),
            fit_canvas_to_parent: true,
            ..default()
        }),
        ..default()
    }))
    .add_plugins(ConfigCachePlugin)
    .add_plugins(AsteroidLifecyclePlugin)
    .add_plugins(ModifierCoordinationPlugin)
    .add_plugins(LobbyPlugin)
    .add_plugins(crate::lobby::lobby_outbox_broadcaster())
    .add_plugins(SimulationPlugin)
    .add_plugins(WorldPlugin)
    .add_plugins(RendererPlugin)
    .add_plugins(ViewscreenBorderPlugin);

    // Conditionally add the debug overlay when ?debug_regions=1 is present.
    DEBUG_REGIONS_ENABLED.with(|v| {
        if *v.borrow() {
            app.add_plugins(crate::debug_overlay::DebugOverlayPlugin { enabled: true });
        }
    });

    app.insert_resource(bevy::winit::WinitSettings::game())
    .add_systems(PreUpdate, (drain_inbound, drain_disconnects))
    .add_systems(PostUpdate, flush_outbound);

    // Insert the validated ShipStations resource if it was pre-validated.
    SHIP_STATIONS.with(|slot| {
        if let Some(stations) = slot.borrow().clone() {
            app.insert_resource(stations);
        }
    });

    app.run();
}

/// Called by JS to deliver an inbound message from a peer into Bevy.
///
/// `sender_token` — the session token of the sender (resolved by the JS
/// bridge from its peer-id → token map; for Identify it equals the token
/// inside the JSON payload).
/// `json` — a JSON-encoded `ClientMessage`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_receive_message(sender_token: &str, json: &str) {
    INBOUND_QUEUE.with(|q| {
        q.borrow_mut().push((sender_token.to_string(), json.to_string()));
    });
}

/// Called by JS when a peer connection closes.
///
/// Queues a disconnect lifecycle event that Bevy processes next frame,
/// replacing the old workaround of dispatching a fake `ClearConsole` message.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_player_disconnected(token: &str) {
    DISCONNECT_QUEUE.with(|q| {
        q.borrow_mut().push(token.to_string());
    });
}

/// Called by JS to register the outbound message callback.
///
/// Bevy will invoke `callback(target: string, payload: string)` for every
/// outbound `ServerMessage`, where `target` is one of:
/// `"all"` — broadcast to every peer
/// `"token:<token>"` — send to one peer
/// `"except:<token>"` — broadcast excluding one peer
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_message_callback(callback: Function) {
    OUTBOUND_CB.with(|slot| {
        *slot.borrow_mut() = Some(callback);
    });
}

/// Called by JS to set the debug_regions flag from the URL parameter.
/// Must be called before `wasm_init()` to take effect.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_set_debug_regions(enabled: bool) {
    DEBUG_REGIONS_ENABLED.with(|v| *v.borrow_mut() = enabled);
}

/// Called by JS (or smoke tests) to query whether debug regions are enabled.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_is_debug_regions_enabled() -> bool {
    DEBUG_REGIONS_ENABLED.with(|v| *v.borrow())
}

// ── Config Preload Exports ──────────────────────────────────────────────────

/// Re-export config preload functions from config_cache module.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_config_request_callback(callback: js_sys::Function) {
    crate::config_cache::set_config_request_callback(callback);
}

/// Re-export config preload functions from config_cache module.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_load_map(toml_str: String) -> Result<JsValue, JsValue> {
    crate::config_cache::wasm_load_map(toml_str)
}

/// Re-export config preload functions from config_cache module.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_load_config(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    crate::config_cache::wasm_load_config(path, toml_str)
}

/// Check if preload is complete.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_is_preload_complete() -> bool {
    crate::config_cache::wasm_is_preload_complete()
}

/// Called by JS with the path and TOML content of a scenario file.
///
/// On success, parses and stores the scenario; returns `Ok(JsValue::TRUE)`.
/// On parse failure, returns `Err(JsValue)` with a human-readable error.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_load_scenario(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    crate::config_cache::wasm_load_scenario(path, toml_str)
}

/// Return the `default_scenario` path from the loaded map config, or `undefined`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_default_scenario_path() -> Option<String> {
    crate::config_cache::wasm_get_default_scenario_path()
}

// ── Bevy bridge systems ────────────────────────────────────────────────────

/// Drains the inbound queue each frame and injects messages into Bevy.
#[cfg(target_arch = "wasm32")]
fn drain_inbound(mut writer: MessageWriter<InboundMessage>) {
    let pending: Vec<(String, String)> =
        INBOUND_QUEUE.with(|q| q.borrow_mut().drain(..).collect());

    for (token, json) in pending {
        if let Ok(msg) = JsonCodec.decode_client(&json) {
            writer.write(InboundMessage { token, msg });
        }
    }
}

/// Drains the disconnect queue each frame and injects lifecycle events into Bevy.
#[cfg(target_arch = "wasm32")]
fn drain_disconnects(mut writer: MessageWriter<PlayerDisconnected>) {
    let pending: Vec<String> =
        DISCONNECT_QUEUE.with(|q| q.borrow_mut().drain(..).collect());
    for token in pending {
        writer.write(PlayerDisconnected { token });
    }
}

/// Reads outbound messages each frame and forwards them to the JS callback.
#[cfg(target_arch = "wasm32")]
fn flush_outbound(mut reader: MessageReader<OutboundMessage>) {
    let dispatches: Vec<(String, String)> = reader
        .read()
        .filter_map(|out| {
            let payload = JsonCodec.encode_server(&out.msg).ok()?;
            let target = match &out.target {
                Target::All => "all".to_string(),
                Target::Token(t) => format!("token:{t}"),
                Target::AllExcept(t) => format!("except:{t}"),
            };
            Some((target, payload))
        })
        .collect();

    if dispatches.is_empty() {
        return;
    }

    OUTBOUND_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            for (target, payload) in &dispatches {
                let _ = cb.call2(
                    &JsValue::NULL,
                    &JsValue::from_str(target),
                    &JsValue::from_str(payload),
                );
            }
        }
    });
}
