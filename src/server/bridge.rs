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
    crate::server_app::add_simulation_plugins,
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

    /// Pending toggle request from `wasm_toggle_debug_regions()`. Drained by
    /// `drain_debug_toggles` each `PreUpdate` frame.
    static PENDING_DEBUG_TOGGLE: RefCell<bool> = const { RefCell::new(false) };

    /// Pending toggle request from `wasm_toggle_debug_overlay()`. Drained by
    /// `drain_debug_toggles` each `PreUpdate` frame.
    static PENDING_TOGGLE_OVERLAY: RefCell<bool> = const { RefCell::new(false) };

    /// Pending toggle request from `wasm_toggle_debug_pause()`. Drained by
    /// `drain_debug_toggles` each `PreUpdate` frame.
    static PENDING_PAUSE_TOGGLE: RefCell<bool> = const { RefCell::new(false) };

    /// Pre-formatted modifier debug text written by `write_debug_state` each
    /// `PostUpdate` frame when the overlay is enabled. Read by
    /// `wasm_get_debug_state()` from JS.
    static DEBUG_STATE_STRING: RefCell<String> = const { RefCell::new(String::new()) };
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
    .add_plugins(crate::lobby::lobby_outbox_broadcaster());
    add_simulation_plugins(&mut app);
    app.add_plugins(WorldPlugin)
    .add_plugins(RendererPlugin)
    .add_plugins(ViewscreenBorderPlugin);

    // Always add the debug overlay plugin; ?debug_regions=1 sets initial state.
    // Runtime toggling via F4 is handled by drain_debug_toggles.
    let debug_regions_initial = DEBUG_REGIONS_ENABLED.with(|v| *v.borrow());
    app.add_plugins(crate::debug_overlay::DebugOverlayPlugin { enabled: debug_regions_initial });

    app.insert_resource(bevy::winit::WinitSettings::game())
    .add_systems(PreUpdate, (drain_inbound, drain_disconnects, drain_debug_toggles))
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

/// Called by JS (e.g. F4 keydown) to toggle region wireframes at runtime.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which flips the `DebugRegionsEnabled` Bevy resource.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_debug_regions() {
    PENDING_DEBUG_TOGGLE.with(|v| *v.borrow_mut() = true);
}

/// Called by JS (e.g. F3 keydown) to toggle the modifier debug overlay at runtime.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which flips the `DebugOverlayEnabled` Bevy resource.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_debug_overlay() {
    PENDING_TOGGLE_OVERLAY.with(|v| *v.borrow_mut() = true);
}

/// Called by JS (e.g. F9 keydown) to toggle the debug simulation pause at runtime.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which pauses or unpauses `Time<Virtual>`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_debug_pause() {
    PENDING_PAUSE_TOGGLE.with(|v| *v.borrow_mut() = true);
}

/// Called by JS each animation frame to read the latest formatted modifier
/// debug state when the overlay is visible.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_debug_state() -> String {
    DEBUG_STATE_STRING.with(|v| v.borrow().clone())
}

/// Called by the Bevy `write_debug_state` system to update the debug state
/// string that JS reads via `wasm_get_debug_state()`.
#[cfg(target_arch = "wasm32")]
pub fn set_debug_state_string(text: String) {
    DEBUG_STATE_STRING.with(|v| *v.borrow_mut() = text);
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
pub fn wasm_load_config(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    crate::config_cache::wasm_load_config(path, toml_str)
}

/// Check if preload is complete.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_is_preload_complete() -> bool {
    crate::config_cache::wasm_is_preload_complete()
}

/// Unified world loader: a single TOML file containing anchors, immediate
/// entity instances (asteroid fields, stations, NPCs, etc.), named [[entity]]
/// instances for trigger / comms anchors, [[trigger]] blocks, and [[comms]]
/// templates.
///
/// Delegates to `config_cache::wasm_load_world`, which performs the unified
/// `parse_world` pass into the `WORLD_CONFIG` thread-local. After PRD #341
/// this is the only world loader — the legacy two-loader split is gone.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_load_world(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    crate::config_cache::wasm_load_world(path, toml_str)
}

/// Register the JS callback used by Rust to request a runtime world TOML fetch.
///
/// The callback signature is: `callback(path: string)`. When called, JS must
/// fetch the TOML at `path` and deliver it via `wasm_push_world_toml`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_world_fetch_callback(callback: js_sys::Function) {
    crate::config_cache::set_world_fetch_callback(callback);
}

/// Deliver a runtime-fetched world TOML to the Rust side.
///
/// Called by JS after fetching a world path that Rust requested via the
/// `set_world_fetch_callback` callback.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_push_world_toml(path: String, toml_str: String) {
    crate::config_cache::wasm_push_world_toml(path, toml_str);
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

/// Drains the pending debug-toggle flags each frame and updates the corresponding
/// Bevy resources: `DebugRegionsEnabled` (F4), `DebugOverlayEnabled` (F3), and
/// `DebugPaused` (F9).
#[cfg(target_arch = "wasm32")]
fn drain_debug_toggles(
    mut regions_enabled: ResMut<crate::debug_overlay::DebugRegionsEnabled>,
    mut overlay_enabled: ResMut<crate::debug_overlay::DebugOverlayEnabled>,
    mut paused: ResMut<crate::debug_overlay::DebugPaused>,
    mut virtual_time: ResMut<Time<bevy::time::Virtual>>,
) {
    // ── Region wireframes toggle (F4) ──────────────────────────────────────
    let pending_regions = PENDING_DEBUG_TOGGLE.with(|v| {
        let was = *v.borrow();
        *v.borrow_mut() = false;
        was
    });
    if pending_regions {
        let new_val = !regions_enabled.0;
        regions_enabled.0 = new_val;
        DEBUG_REGIONS_ENABLED.with(|v| *v.borrow_mut() = new_val);
    }

    // ── Modifier overlay toggle (F3) ───────────────────────────────────────
    let pending_overlay = PENDING_TOGGLE_OVERLAY.with(|v| {
        let was = *v.borrow();
        *v.borrow_mut() = false;
        was
    });
    if pending_overlay {
        overlay_enabled.0 = !overlay_enabled.0;
    }

    // ── Simulation pause toggle (F9) ───────────────────────────────────────
    let pending_pause = PENDING_PAUSE_TOGGLE.with(|v| {
        let was = *v.borrow();
        *v.borrow_mut() = false;
        was
    });
    if pending_pause {
        paused.0 = !paused.0;
        if paused.0 {
            virtual_time.pause();
        } else {
            virtual_time.unpause();
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
