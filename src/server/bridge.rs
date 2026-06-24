// WASM/JS bridge — all public functions are #[wasm_bindgen] exports.
//
// On native targets this module is empty; the WASM-specific code is
// gated behind #[cfg(target_arch = "wasm32")].

#[cfg(target_arch = "wasm32")]
use {
    crate::asteroid_lifecycle::AsteroidLifecyclePlugin,
    crate::codec::{self, JsonCodec, MessageCodec},
    crate::config_cache::ConfigCachePlugin,
    crate::console_bridge::{
        ConsoleStateChanged, HudStateChanged, LobbyStateChanged, LOCAL_CONSOLE_TOKEN,
    },
    crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, PlayerDisconnected, Target},
    crate::messages,
    crate::modifier_coordination::ModifierCoordinationPlugin,
    crate::renderer::RendererPlugin,
    crate::server_app::add_simulation_plugins,
    crate::ship::config::ShipConfig,
    crate::ship_plugin::ShipConfigResource,
    crate::stations_config::ShipStations,
    crate::viewscreen_border::ViewscreenBorderPlugin,
    crate::world::WorldPlugin,
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

    /// Validated ShipConfig, stored by wasm_validate_stations() so
    /// wasm_init() can insert it as a ShipConfigResource before LobbyPlugin
    /// tries to init_resource it (panicking in WASM via std::fs::read_to_string).
    static SHIP_CONFIG: RefCell<Option<ShipConfig>> = const { RefCell::new(None) };

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

    /// Pending toggle request from `wasm_toggle_debug_damage()`. Drained by
    /// `drain_debug_toggles` each `PreUpdate` frame.
    static PENDING_TOGGLE_DAMAGE: RefCell<bool> = const { RefCell::new(false) };

    /// Pre-formatted modifier debug text written by `write_debug_state` each
    /// `PostUpdate` frame when the overlay is enabled. Read by
    /// `wasm_get_debug_state()` from JS.
    static DEBUG_STATE_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Pre-formatted damage-log text written by `write_damage_log` each
    /// `PostUpdate` frame when the damage overlay is enabled. Read by
    /// `wasm_get_damage_log()` from JS.
    static DAMAGE_LOG_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Pending toggle request from `wasm_toggle_debug_entities()`. Drained by
    /// `drain_debug_toggles` each `PreUpdate` frame.
    static PENDING_TOGGLE_ENTITIES: RefCell<bool> = const { RefCell::new(false) };

    /// Pre-formatted entity behavior text written by `write_entity_debug_state`
    /// each `PostUpdate` frame when the overlay is enabled. Read by
    /// `wasm_get_entity_debug_state()` from JS.
    static ENTITY_DEBUG_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Pending toggle request from `wasm_toggle_entity_inspector()`. Drained by
    /// `drain_debug_toggles` each `PreUpdate` frame.
    static PENDING_TOGGLE_ENTITY_INSPECTOR: RefCell<bool> = const { RefCell::new(false) };

    /// Pre-formatted entity inspector text written by `update_entity_inspector`
    /// each `PostUpdate` frame when the overlay is enabled. Read by
    /// `wasm_get_entity_inspector()` from JS.
    static ENTITY_INSPECTOR_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Raw `__sendAction` JSON envelopes pushed by `wasm_ui_action`, waiting to
    /// be decoded and injected into Bevy by `drain_ui_actions`.
    static UI_ACTION_QUEUE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

    /// JS callback registered by the HTML console to receive per-console state
    /// pushes. Signature: `callback(name: string, stateJson: string)`.
    static CONSOLE_STATE_CB: RefCell<Option<Function>> = const { RefCell::new(None) };

    /// JS callback registered by the HTML viewscreen overlay to receive HUD
    /// state pushes. Signature: `callback(stateJson: string)`.
    static HUD_STATE_CB: RefCell<Option<Function>> = const { RefCell::new(None) };

    /// JS callback registered by the HTML lobby overlay to receive lobby state
    /// pushes. Signature: `callback(stateJson: string)`.
    static LOBBY_STATE_CB: RefCell<Option<Function>> = const { RefCell::new(None) };
}

// ── Public WASM API ────────────────────────────────────────────────────────

/// Called by JS with the raw player_ship.toml content to validate the
/// `[[station]]` schema before starting the server.
///
/// On success, stores the parsed `ShipStations` internally and returns
/// `Ok(JsValue::UNDEFINED)`. On failure, returns `Err(JsValue)` with a
/// human-readable error string. PeerJS should not start when this returns
/// an error.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_validate_stations(toml_str: &str) -> Result<JsValue, JsValue> {
    use crate::ship::system_registry::SystemKindRegistry;
    let registry = SystemKindRegistry::with_core_systems()
        .map_err(|e| JsValue::from_str(&format!("System registry init failed: {}", e)))?;
    let kinds: Vec<&str> = registry.kinds().collect();
    match crate::ship::config::parse_and_validate(toml_str, &kinds) {
        Ok(ship_config) => {
            let stations = crate::stations_config::stations_from_ship_config(&ship_config);
            SHIP_STATIONS.with(|slot| {
                *slot.borrow_mut() = Some(stations);
            });
            SHIP_CONFIG.with(|slot| {
                *slot.borrow_mut() = Some(ship_config);
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
    // Route Rust panics through console.error with a useful message + location.
    // Without this, a panic in any Bevy system traps the wasm instance and
    // every subsequent JS→WASM call surfaces as a bare "RuntimeError: memory
    // access out of bounds" pointing at whatever entry point fired next
    // (typically `wasm_receive_message` since the host page receives PeerJS
    // messages continuously). `set_once` is idempotent.
    console_error_panic_hook::set_once();

    // Detect WebDriver/Playwright automation (navigator.webdriver). In
    // headless CI the Bevy RenderPlugin panics trying to initialise wgpu
    // (no GPU available), so we skip render/audio/gltf/gizmo plugins.
    let is_automation = web_sys::window()
        .and_then(|w| {
            let nav = w.navigator();
            js_sys::Reflect::get(&nav, &"webdriver".into())
                .ok()
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false);

    let mut app = App::new();
    if is_automation {
        use bevy::{
            a11y::AccessibilityPlugin,
            app::{PanicHandlerPlugin, TaskPoolPlugin},
            asset::AssetPlugin,
            diagnostic::{DiagnosticsPlugin, FrameCountPlugin},
            input::InputPlugin,
            log::LogPlugin,
            scene::ScenePlugin,
            state::app::StatesPlugin,
            time::TimePlugin,
            transform::TransformPlugin,
            winit::WinitPlugin,
        };
        app.add_plugins((
            PanicHandlerPlugin,
            LogPlugin::default(),
            TaskPoolPlugin::default(),
            FrameCountPlugin,
            TimePlugin::default(),
            TransformPlugin::default(),
            DiagnosticsPlugin,
            InputPlugin::default(),
            bevy::window::WindowPlugin {
                primary_window: Some(bevy::window::Window {
                    canvas: Some("#canvas".into()),
                    fit_canvas_to_parent: true,
                    ..default()
                }),
                ..default()
            },
            AccessibilityPlugin,
            AssetPlugin::default(),
            ScenePlugin::default(),
            WinitPlugin::default(),
            StatesPlugin,
        ));
        // Register asset types that simulation plugins (StarRenderPlugin,
        // render_spawned_entities, asset_preload etc.) depend on. Without
        // RenderPlugin these aren't auto-registered.
        use bevy::{
            asset::AssetApp,
            image::Image,
            mesh::Mesh,
            pbr::StandardMaterial,
        };
        app.init_asset::<bevy::shader::Shader>()
            .init_asset_loader::<bevy::shader::ShaderLoader>()
            .init_asset::<Image>()
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>();
    } else {
        app.add_plugins(DefaultPlugins.set(bevy::window::WindowPlugin {
            primary_window: Some(bevy::window::Window {
                canvas: Some("#canvas".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }));
    }
    app.add_plugins(ConfigCachePlugin)
    .add_plugins(AsteroidLifecyclePlugin)
    .add_plugins(ModifierCoordinationPlugin);
    // Insert ShipConfigResource before LobbyPlugin so its
    // .init_resource::<ShipConfigResource>() is a no-op (the default
    // calls load_ship_config_from_disk which uses std::fs — panics in WASM).
    SHIP_CONFIG.with(|slot| {
        if let Some(config) = slot.borrow_mut().take() {
            app.insert_resource(ShipConfigResource(config));
        }
    });
    app.add_plugins(LobbyPlugin)
    .add_plugins(crate::lobby::lobby_outbox_broadcaster());
    add_simulation_plugins(&mut app);
    app.add_plugins(WorldPlugin);
    if !is_automation {
        app.add_plugins(RendererPlugin)
            .add_plugins(ViewscreenBorderPlugin);
    }

    // Always add the debug overlay plugin; ?debug_regions=1 sets initial state.
    // Runtime toggling via F4 is handled by drain_debug_toggles.
    let debug_regions_initial = DEBUG_REGIONS_ENABLED.with(|v| *v.borrow());
    app.add_plugins(crate::debug_overlay::DebugOverlayPlugin {
        enabled: debug_regions_initial,
    });

    app.insert_resource(bevy::winit::WinitSettings {
        // Continuous mode ensures the event loop drives requestAnimationFrame
        // updates on every frame. Reactive mode (used previously) can stall in
        // headless Chromium when the page has no UI events, causing the inbound
        // message pipeline (wasm_receive_message → drain_inbound → process_lobby)
        // to never run — all smoke tests that use createTestClient timeout waiting
        // for Welcome.
        focused_mode: bevy::winit::UpdateMode::Continuous,
        unfocused_mode: bevy::winit::UpdateMode::Reactive {
            wait: std::time::Duration::from_secs_f64(1.0 / 20.0),
            react_to_device_events: false,
            react_to_user_events: false,
            react_to_window_events: true,
        },
    })
    .add_systems(
        PreUpdate,
        (
            drain_inbound,
            drain_disconnects,
            drain_debug_toggles,
            drain_ui_actions,
        ),
    )
    .add_systems(
        PostUpdate,
        (
            flush_outbound,
            flush_hud_state,
            flush_console_state,
            flush_lobby_state,
        ),
    );

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
        q.borrow_mut()
            .push((sender_token.to_string(), json.to_string()));
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

/// Called by the HTML transport shim (ADR-0001 §3) when a local HTML console
/// triggers an action. `json` is the raw `__sendAction` envelope; it is queued
/// and decoded by `drain_ui_actions` on the next `PreUpdate` frame.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_ui_action(json: &str) {
    UI_ACTION_QUEUE.with(|q| {
        q.borrow_mut().push(json.to_string());
    });
}

/// Called by JS once to register the per-console state-push callback.
/// Bevy calls `callback(name: string, stateJson: string)` on console change.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_console_state_callback(callback: Function) {
    CONSOLE_STATE_CB.with(|slot| {
        *slot.borrow_mut() = Some(callback);
    });
}

/// Called by JS once to register the viewscreen HUD-state push callback.
/// Bevy calls `callback(stateJson: string)` on HUD change.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_hud_state_callback(callback: Function) {
    HUD_STATE_CB.with(|slot| {
        *slot.borrow_mut() = Some(callback);
    });
}

/// Called by JS once to register the lobby-state push callback.
/// Bevy calls `callback(stateJson: string)` on lobby state change.
/// Must be registered before `wasm_init()` so the first push is never missed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_lobby_state_callback(callback: Function) {
    LOBBY_STATE_CB.with(|slot| {
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

/// Called by JS (e.g. F8 keydown) to toggle the damage debug overlay at runtime.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which flips the `DebugDamageEnabled` Bevy resource.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_debug_damage() {
    PENDING_TOGGLE_DAMAGE.with(|v| *v.borrow_mut() = true);
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

/// Called by JS each animation frame to read the latest damage log text when
/// the damage overlay is visible.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_damage_log() -> String {
    DAMAGE_LOG_STRING.with(|v| v.borrow().clone())
}

/// Called by the Bevy `write_damage_log` system to update the damage log
/// string that JS reads via `wasm_get_damage_log()`.
#[cfg(target_arch = "wasm32")]
pub fn set_damage_log_string(text: String) {
    DAMAGE_LOG_STRING.with(|v| *v.borrow_mut() = text);
}

/// Called by JS (e.g. F5 keydown) to toggle the entity behavior overlay at runtime.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which flips the `DebugEntitiesEnabled` Bevy resource.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_debug_entities() {
    PENDING_TOGGLE_ENTITIES.with(|v| *v.borrow_mut() = true);
}

/// Called by JS each animation frame to read the latest entity behavior debug
/// text when the overlay is visible.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_entity_debug_state() -> String {
    ENTITY_DEBUG_STRING.with(|v| v.borrow().clone())
}

/// Called by the Bevy `write_entity_debug_state` system to update the entity
/// debug string that JS reads via `wasm_get_entity_debug_state()`.
#[cfg(target_arch = "wasm32")]
pub fn set_entity_debug_string(text: String) {
    ENTITY_DEBUG_STRING.with(|v| *v.borrow_mut() = text);
}

/// Called by JS (e.g. F6 keydown) to toggle the entity inspector overlay at runtime.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which flips the `DebugEntityInspectorEnabled` Bevy resource.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_entity_inspector() {
    PENDING_TOGGLE_ENTITY_INSPECTOR.with(|v| *v.borrow_mut() = true);
}

/// Called by JS each animation frame to read the latest entity inspector text
/// when the overlay is visible.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_entity_inspector() -> String {
    ENTITY_INSPECTOR_STRING.with(|v| v.borrow().clone())
}

/// Called by the Bevy `update_entity_inspector` system to update the entity
/// inspector string that JS reads via `wasm_get_entity_inspector()`.
#[cfg(target_arch = "wasm32")]
pub fn set_entity_inspector_string(text: String) {
    ENTITY_INSPECTOR_STRING.with(|v| *v.borrow_mut() = text);
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

/// Deliver a runtime-fetched model-rig sidecar TOML to the Rust side.
///
/// Called by JS after fetching a sidecar path that Rust requested via the
/// `set_world_fetch_callback` callback (the same callback serves both world
/// TOMLs and rig sidecars). Pass an empty string when the sidecar is absent
/// (404) so the renderer proceeds with an identity base rig.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_push_sidecar_toml(path: String, toml_str: String) {
    crate::config_cache::wasm_push_sidecar_toml(path, toml_str);
}

// ── Bevy bridge systems ────────────────────────────────────────────────────

/// Drains the inbound queue each frame and injects messages into Bevy.
#[cfg(target_arch = "wasm32")]
fn drain_inbound(mut writer: MessageWriter<InboundMessage>) {
    let pending: Vec<(String, String)> = INBOUND_QUEUE.with(|q| q.borrow_mut().drain(..).collect());

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
    mut damage_enabled: ResMut<crate::debug_overlay::DebugDamageEnabled>,
    mut entities_enabled: ResMut<crate::debug_overlay::DebugEntitiesEnabled>,
    mut entity_inspector_enabled: ResMut<crate::debug_overlay::DebugEntityInspectorEnabled>,
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

    // ── Damage overlay toggle (F8) ─────────────────────────────────────────
    let pending_damage = PENDING_TOGGLE_DAMAGE.with(|v| {
        let was = *v.borrow();
        *v.borrow_mut() = false;
        was
    });
    if pending_damage {
        damage_enabled.0 = !damage_enabled.0;
    }

    // ── Entity behavior overlay toggle (F5) ────────────────────────────────
    let pending_entities = PENDING_TOGGLE_ENTITIES.with(|v| {
        let was = *v.borrow();
        *v.borrow_mut() = false;
        was
    });
    if pending_entities {
        entities_enabled.0 = !entities_enabled.0;
    }

    // ── Entity inspector overlay toggle (F6) ───────────────────────────────
    let pending_inspector = PENDING_TOGGLE_ENTITY_INSPECTOR.with(|v| {
        let was = *v.borrow();
        *v.borrow_mut() = false;
        was
    });
    if pending_inspector {
        entity_inspector_enabled.0 = !entity_inspector_enabled.0;
    }
}

/// Drains the disconnect queue each frame and injects lifecycle events into Bevy.
#[cfg(target_arch = "wasm32")]
fn drain_disconnects(mut writer: MessageWriter<PlayerDisconnected>) {
    let pending: Vec<String> = DISCONNECT_QUEUE.with(|q| q.borrow_mut().drain(..).collect());
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

/// Drains the UI-action queue each frame: decodes each `__sendAction` envelope
/// into a `UiAction`, maps it to the corresponding `ClientMessage`, and injects
/// it as an `InboundMessage` from the local console token so the existing
/// weapons handlers process it. Decode failures are ignored (matching
/// `drain_inbound`).
#[cfg(target_arch = "wasm32")]
fn drain_ui_actions(mut writer: MessageWriter<InboundMessage>) {
    let pending: Vec<String> = UI_ACTION_QUEUE.with(|q| q.borrow_mut().drain(..).collect());
    for json in pending {
        if let Ok(action) = codec::decode_ui_action(&json) {
            let msg = messages::ui_action_to_client_message(&action);
            writer.write(InboundMessage {
                token: LOCAL_CONSOLE_TOKEN.to_string(),
                msg,
            });
        }
    }
}

/// Reads `HudStateChanged` messages each frame and forwards the JSON to the
/// registered HUD-state callback via `cb.call1(NULL, json)`.
#[cfg(target_arch = "wasm32")]
fn flush_hud_state(mut reader: MessageReader<HudStateChanged>) {
    let payloads: Vec<String> = reader.read().map(|m| m.json.clone()).collect();
    if payloads.is_empty() {
        return;
    }
    HUD_STATE_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            for json in &payloads {
                let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(json));
            }
        }
    });
}

/// Reads `ConsoleStateChanged` messages each frame and forwards `(name, json)`
/// to the registered console-state callback via `cb.call2(NULL, name, json)`.
#[cfg(target_arch = "wasm32")]
fn flush_console_state(mut reader: MessageReader<ConsoleStateChanged>) {
    let payloads: Vec<(String, String)> = reader
        .read()
        .map(|m| (m.name.clone(), m.json.clone()))
        .collect();
    if payloads.is_empty() {
        return;
    }
    CONSOLE_STATE_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            for (name, json) in &payloads {
                let _ = cb.call2(
                    &JsValue::NULL,
                    &JsValue::from_str(name),
                    &JsValue::from_str(json),
                );
            }
        }
    });
}

/// Reads `LobbyStateChanged` messages each frame and forwards the JSON to the
/// registered lobby-state callback via `cb.call1(NULL, json)`.
#[cfg(target_arch = "wasm32")]
fn flush_lobby_state(mut reader: MessageReader<LobbyStateChanged>) {
    let payloads: Vec<String> = reader.read().map(|m| m.json.clone()).collect();
    if payloads.is_empty() {
        return;
    }
    LOBBY_STATE_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            for json in &payloads {
                let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(json));
            }
        }
    });
}
