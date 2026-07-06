// WASM/JS bridge — all public functions are #[wasm_bindgen] exports.
//
// On native targets this module is empty except for the debug-toggle
// pending-set (`DebugToggleKind` + `apply_pending_toggles`), which is kept
// free of `target_arch = "wasm32"` and Bevy types specifically so it can be
// unit-tested natively (see the `tests` module at the bottom of this file).
// The WASM-specific glue (thread-locals, wasm_bindgen exports, the Bevy
// drain system) is gated behind #[cfg(target_arch = "wasm32")].

// Used by the debug-toggle pending-set below, which is compiled on every
// target (see comment on `DebugToggleKind`), so this import is not gated.
use std::collections::HashSet;

#[cfg(target_arch = "wasm32")]
use {
    crate::asteroid_lifecycle::AsteroidLifecyclePlugin,
    crate::codec::{self, JsonCodec, MessageCodec},
    crate::config_cache::ConfigCachePlugin,
    crate::console_bridge::{
        ConsoleStateChanged, HudStateChanged, LobbyStateChanged, LOCAL_CONSOLE_TOKEN,
    },
    crate::lobby::{
        InboundMessage, LobbyOutbox, LobbyPlugin, OutboundMessage, PlayerDisconnected,
        SelectedShipResource, Target,
    },
    crate::messages::{self, DeliveryClass},
    crate::modifier_coordination::ModifierCoordinationPlugin,
    crate::renderer::RendererPlugin,
    crate::server_app::add_simulation_plugins,
    crate::ship::config::ShipConfig,
    crate::ship_plugin::PendingShipConfig,
    crate::stations_config::ShipStations,
    crate::viewscreen_border::ViewscreenBorderPlugin,
    crate::world::WorldPlugin,
    bevy::{prelude::*, DefaultPlugins},
    js_sys::{Array, Function, Object, Reflect},
    std::cell::RefCell,
    wasm_bindgen::prelude::*,
};

// ── Debug toggle pending-set ────────────────────────────────────────────────
//
// One enum-keyed pending set replaces what used to be six near-identical
// `RefCell<bool>` thread-locals (one per debug overlay) plus six matching
// blocks in the drain system (issue #609). Adding a new debug overlay now
// means: add a variant here, add its resource field to `apply_pending_toggles`,
// and add one `wasm_bindgen` export that inserts the variant — no new
// thread-local, no new drain block.
//
// Deliberately kept free of `target_arch = "wasm32"` and Bevy types so it can
// be unit-tested on native (`cargo test`) without a running Bevy App.

/// Identifies which debug overlay/toggle a pending request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebugToggleKind {
    /// F4 — region wireframes (`DebugRegionsEnabled`).
    Regions,
    /// F3 — modifier debug overlay (`DebugOverlayEnabled`).
    Overlay,
    /// F9 — simulation pause (`DebugPaused`); also (un)pauses `Time<Virtual>`.
    Pause,
    /// F8 — damage debug log (`DebugDamageEnabled`).
    Damage,
    /// F7 — entity behavior overlay (`DebugEntitiesEnabled`).
    Entities,
    /// F2 — entity inspector overlay (`DebugEntityInspectorEnabled`).
    EntityInspector,
}

/// Applies a batch of pending debug-toggle requests to plain `bool` flags.
///
/// Pure function, no Bevy/wasm dependency, so it can be exercised directly by
/// unit tests. Each distinct `DebugToggleKind` present in `pending` flips its
/// corresponding flag exactly once, regardless of how many times that variant
/// appears in the iterator (the pending set only ever holds each kind once,
/// but the function doesn't rely on that to stay correct).
///
/// Returns `true` if `*paused` was flipped, so the caller (the Bevy drain
/// system) knows to (un)pause `Time<Virtual>` as a side effect.
pub fn apply_pending_toggles(
    pending: impl IntoIterator<Item = DebugToggleKind>,
    regions: &mut bool,
    overlay: &mut bool,
    paused: &mut bool,
    damage: &mut bool,
    entities: &mut bool,
    entity_inspector: &mut bool,
) -> bool {
    let mut pause_changed = false;
    // Dedupe in case the same kind was queued multiple times between drains.
    let unique: HashSet<DebugToggleKind> = pending.into_iter().collect();
    for kind in unique {
        match kind {
            DebugToggleKind::Regions => *regions = !*regions,
            DebugToggleKind::Overlay => *overlay = !*overlay,
            DebugToggleKind::Pause => {
                *paused = !*paused;
                pause_changed = true;
            }
            DebugToggleKind::Damage => *damage = !*damage,
            DebugToggleKind::Entities => *entities = !*entities,
            DebugToggleKind::EntityInspector => *entity_inspector = !*entity_inspector,
        }
    }
    pause_changed
}

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

    /// Pending debug-toggle requests queued by the six `wasm_toggle_*`
    /// exports. Drained by `drain_debug_toggles` each `PreUpdate` frame via
    /// `apply_pending_toggles`. Consolidated from six separate
    /// `RefCell<bool>` thread-locals into one enum-keyed set (issue #609).
    static PENDING_TOGGLES: RefCell<HashSet<DebugToggleKind>> = RefCell::new(HashSet::new());

    /// Pre-formatted modifier debug text written by `write_debug_state` each
    /// `PostUpdate` frame when the overlay is enabled. Read by
    /// `wasm_get_debug_state()` from JS.
    static DEBUG_STATE_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Pre-formatted damage-log text written by `write_damage_log` each
    /// `PostUpdate` frame when the damage overlay is enabled. Read by
    /// `wasm_get_damage_log()` from JS.
    static DAMAGE_LOG_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Pre-formatted entity behavior text written by `write_entity_debug_state`
    /// each `PostUpdate` frame when the overlay is enabled. Read by
    /// `wasm_get_entity_debug_state()` from JS.
    static ENTITY_DEBUG_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Pre-formatted entity inspector text written by `update_entity_inspector`
    /// each `PostUpdate` frame when the overlay is enabled. Read by
    /// `wasm_get_entity_inspector()` from JS.
    static ENTITY_INSPECTOR_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Pending force-start request from `wasm_force_start()`. Drained by
    /// `drain_force_start` each `PreUpdate` frame to transition directly to
    /// `InProgress` without any connected players (fully AI-crewed ship).
    static PENDING_FORCE_START: RefCell<bool> = const { RefCell::new(false) };

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

    /// JS callback registered by the HTML viewscreen overlay to receive screen
    /// shake offsets. Signature: `callback(x: number, y: number)`.
    /// Called every frame with the current pixel offset; (0, 0) when no shake.
    static SHAKE_CB: RefCell<Option<Function>> = const { RefCell::new(None) };

    /// Latest screen shake offset (x, y) in CSS pixels, written by
    /// [`viewscreen_border::apply_camera_shake`] each frame and read by
    /// [`flush_shake_state`] for the JS callback.
    static SHAKE_OFFSET: RefCell<(f32, f32)> = const { RefCell::new((0.0, 0.0)) };

    /// Template path of the player ship selected by the host. Set by
    /// `wasm_select_ship()` before `wasm_init()`. When absent, defaults
    /// to `"assets/entities/player_ship.toml"`.
    static SELECTED_SHIP_TEMPLATE_PATH: RefCell<Option<String>> =
        const { RefCell::new(None) };
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
        use bevy::{asset::AssetApp, image::Image, mesh::Mesh, pbr::StandardMaterial};
        app.init_asset::<bevy::shader::Shader>()
            .init_asset_loader::<bevy::shader::ShaderLoader>()
            .init_asset::<Image>()
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>();
        // Register messages that non-rendering systems need. These are
        // normally registered by ViewscreenBorderPlugin / RendererPlugin
        // which we skip in automation mode.
        app.add_message::<crate::console_bridge::HudStateChanged>()
            .add_message::<crate::console_bridge::LobbyStateChanged>();
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
            app.insert_resource(PendingShipConfig(config));
        }
    });
    app.add_plugins(LobbyPlugin)
        .add_plugins(crate::lobby::lobby_outbox_broadcaster());
    add_simulation_plugins(&mut app);
    app.add_plugins(WorldPlugin);
    // Insert the selected ship resource (set by wasm_select_ship before
    // wasm_init was called). Falls back to the legacy default path.
    {
        let ship_path = SELECTED_SHIP_TEMPLATE_PATH
            .with(|slot| slot.borrow().clone())
            .unwrap_or_else(|| "assets/entities/player_ship.toml".to_string());
        app.insert_resource(SelectedShipResource(ship_path));
    }
    if is_automation {
        // push_lobby_state is normally registered by ViewscreenBorderPlugin,
        // which we skip in automation mode. Register it directly so the HTML
        // lobby panel stays updated during smoke tests.
        app.add_systems(Update, crate::server::viewscreen_border::push_lobby_state);
    } else {
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
        // Keep the host simulation ticking even after Playwright opens a client
        // page in front of it; otherwise Identify stays queued and Welcome never
        // leaves the server.
        focused_mode: bevy::winit::UpdateMode::Continuous,
        unfocused_mode: bevy::winit::UpdateMode::Continuous,
    })
    .add_systems(
        PreUpdate,
        (
            drain_inbound,
            drain_disconnects,
            drain_debug_toggles,
            drain_ui_actions,
            drain_force_start,
        ),
    )
    .add_systems(
        PostUpdate,
        (
            flush_outbound,
            flush_hud_state,
            flush_console_state,
            flush_lobby_state,
            flush_shake_state,
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

/// Called by JS once to register the screen-shake callback.
/// Bevy calls `callback(x: number, y: number)` every frame with the current
/// CSS pixel offset. `(0, 0)` means no shake.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_shake_callback(callback: Function) {
    SHAKE_CB.with(|slot| {
        *slot.borrow_mut() = Some(callback);
    });
}

/// Called by [`viewscreen_border::apply_camera_shake`] (WASM builds only) to
/// store the current frame's screen-shake offset for JS.
#[cfg(target_arch = "wasm32")]
pub fn set_shake_offset(x: f32, y: f32) {
    SHAKE_OFFSET.with(|slot| {
        *slot.borrow_mut() = (x, y);
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
    PENDING_TOGGLES.with(|set| {
        set.borrow_mut().insert(DebugToggleKind::Regions);
    });
}

/// Called by JS (e.g. F3 keydown) to toggle the modifier debug overlay at runtime.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which flips the `DebugOverlayEnabled` Bevy resource.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_debug_overlay() {
    PENDING_TOGGLES.with(|set| {
        set.borrow_mut().insert(DebugToggleKind::Overlay);
    });
}

/// Called by JS (e.g. F9 keydown) to toggle the debug simulation pause at runtime.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which pauses or unpauses `Time<Virtual>`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_debug_pause() {
    PENDING_TOGGLES.with(|set| {
        set.borrow_mut().insert(DebugToggleKind::Pause);
    });
}

/// Called by JS (e.g. F8 keydown) to toggle the damage debug overlay at runtime.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which flips the `DebugDamageEnabled` Bevy resource.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_debug_damage() {
    PENDING_TOGGLES.with(|set| {
        set.borrow_mut().insert(DebugToggleKind::Damage);
    });
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
    PENDING_TOGGLES.with(|set| {
        set.borrow_mut().insert(DebugToggleKind::Entities);
    });
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
    PENDING_TOGGLES.with(|set| {
        set.borrow_mut().insert(DebugToggleKind::EntityInspector);
    });
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

/// Called by JS (lobby "Launch AI Ship" button) to start the game with no
/// human players — all stations run under AI/backfill control.
///
/// Only takes effect when the game is currently in the `Lobby` phase. The
/// actual phase transition is applied by `drain_force_start` on the next
/// `PreUpdate` frame so it runs safely inside the Bevy schedule.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_force_start() {
    PENDING_FORCE_START.with(|v| *v.borrow_mut() = true);
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

/// Return the list of available player ships for the currently loaded world.
///
/// Returns a JS array of `{ template_path: string, label: string }` objects.
/// The list comes from the world's `[available_ships]` entries (issue #623).
/// When the world has no `available_ships` list, returns an empty array — the
/// host should fall back to the hardcoded `assets/entities/player_ship.toml`.
///
/// Uses `js_sys::Array` / `JsValue` to avoid manual JSON construction (which
/// would need escaping for `"` and `\` in template_path or label values).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_available_ships() -> Array {
    let world_config = crate::config_cache::get_world_config();
    let ships = match world_config {
        Some(ref wc) => &wc.available_ships,
        None => return Array::new(),
    };
    let arr = Array::new();
    for ship in ships {
        let obj = Object::new();
        let label = ship.label.as_deref().unwrap_or(&ship.template_path);
        Reflect::set(
            &obj,
            &JsValue::from_str("template_path"),
            &JsValue::from_str(&ship.template_path),
        )
        .ok();
        Reflect::set(&obj, &JsValue::from_str("label"), &JsValue::from_str(label)).ok();
        arr.push(&obj);
    }
    arr
}

/// Store the host's chosen player ship template path.
///
/// Must be called before `wasm_init()`. The path is used by
/// `update_session_with_config` and `spawn_game_start_entities` to
/// load the correct ship config and entity template.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_select_ship(template_path: &str) {
    SELECTED_SHIP_TEMPLATE_PATH.with(|slot| {
        *slot.borrow_mut() = Some(template_path.to_string());
    });
}

// ── Bevy bridge systems ────────────────────────────────────────────────────

/// Drains the inbound queue each frame and injects messages into Bevy.
/// Decode failures are logged as warnings with truncated token/payload.
#[cfg(target_arch = "wasm32")]
fn drain_inbound(mut writer: MessageWriter<InboundMessage>) {
    let pending: Vec<(String, String)> = INBOUND_QUEUE.with(|q| q.borrow_mut().drain(..).collect());
    let (successes, failures) = codec::decode_bridge_client_messages(pending);
    for err in &failures {
        bevy::log::warn!(
            "decode failure from token={}: payload={}",
            err.token,
            err.payload_snippet
        );
    }
    for (token, msg) in successes {
        writer.write(InboundMessage { token, msg });
    }
}

/// Drains the pending debug-toggle set each frame and updates the
/// corresponding Bevy resources: `DebugRegionsEnabled` (F4),
/// `DebugOverlayEnabled` (F3), `DebugPaused` (F9), `DebugDamageEnabled` (F8),
/// `DebugEntitiesEnabled` (F7), and `DebugEntityInspectorEnabled` (F2).
///
/// The actual flag-flipping logic lives in [`apply_pending_toggles`], a pure
/// function with no Bevy/wasm dependency so it's unit-testable on native.
/// This system just drains the thread-local set, calls that function against
/// the live resources, and handles the one Bevy-specific side effect
/// (pausing/unpausing `Time<Virtual>`) based on the returned flag.
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
    let pending: Vec<DebugToggleKind> =
        PENDING_TOGGLES.with(|set| set.borrow_mut().drain().collect());
    if pending.is_empty() {
        return;
    }

    let pause_changed = apply_pending_toggles(
        pending,
        &mut regions_enabled.0,
        &mut overlay_enabled.0,
        &mut paused.0,
        &mut damage_enabled.0,
        &mut entities_enabled.0,
        &mut entity_inspector_enabled.0,
    );

    // Region-wireframe state also lives in a thread-local (read back by
    // `wasm_is_debug_regions_enabled()`), so mirror the resource into it.
    DEBUG_REGIONS_ENABLED.with(|v| *v.borrow_mut() = regions_enabled.0);

    if pause_changed {
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
    let pending: Vec<String> = DISCONNECT_QUEUE.with(|q| q.borrow_mut().drain(..).collect());
    for token in pending {
        writer.write(PlayerDisconnected { token });
    }
}

/// Drains the force-start flag each frame. When set, transitions the game
/// directly to `InProgress` (or `Loading` if the asset preload isn't done)
/// without requiring any connected players — used for fully AI-crewed runs.
#[cfg(target_arch = "wasm32")]
fn drain_force_start(
    state: Res<State<messages::GamePhase>>,
    mut next_state: ResMut<NextState<messages::GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    preload: Option<Res<crate::server::asset_preload::AssetPreloadResource>>,
) {
    let pending = PENDING_FORCE_START.with(|v| {
        let was = *v.borrow();
        *v.borrow_mut() = false;
        was
    });
    if !pending || state.get() != &messages::GamePhase::Lobby {
        return;
    }
    let preload_complete = if crate::debug_overlay::is_playwright_automation() {
        true
    } else {
        preload
            .as_ref()
            .map(|p| !p.started || p.complete)
            .unwrap_or(true)
    };
    if preload_complete {
        next_state.set(messages::GamePhase::InProgress);
        outbox
            .0
            .push((Target::All, messages::ServerMessage::GameStarted));
    } else {
        next_state.set(messages::GamePhase::Loading);
    }
}

/// Reads outbound messages each frame and forwards them to the JS callback.
#[cfg(target_arch = "wasm32")]
fn flush_outbound(mut reader: MessageReader<OutboundMessage>) {
    let dispatches: Vec<(String, String, String)> = reader
        .read()
        .filter_map(|out| {
            let payload = JsonCodec.encode_server(&out.msg).ok()?;
            let target = match &out.target {
                Target::All => "all".to_string(),
                Target::Token(t) => format!("token:{t}"),
                Target::AllExcept(t) => format!("except:{t}"),
            };
            let class_str = match out.delivery {
                DeliveryClass::Reliable => "reliable",
                DeliveryClass::Snapshot => "snapshot",
            };
            Some((target, payload, class_str.to_string()))
        })
        .collect();

    if dispatches.is_empty() {
        return;
    }

    OUTBOUND_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            for (target, payload, class_str) in &dispatches {
                let _ = cb.call3(
                    &JsValue::NULL,
                    &JsValue::from_str(target),
                    &JsValue::from_str(payload),
                    &JsValue::from_str(class_str),
                );
            }
        }
    });
}

/// Drains the UI-action queue each frame: decodes each `__sendAction` envelope
/// into a `UiAction`, maps it to the corresponding `ClientMessage`, and injects
/// it as an `InboundMessage` from the local console token so the existing
/// weapons handlers process it. Decode failures are logged as warnings.
#[cfg(target_arch = "wasm32")]
fn drain_ui_actions(mut writer: MessageWriter<InboundMessage>) {
    let pending: Vec<String> = UI_ACTION_QUEUE.with(|q| q.borrow_mut().drain(..).collect());
    for json in pending {
        match codec::decode_ui_action(&json) {
            Ok(action) => {
                let msg = messages::ui_action_to_client_message(&action);
                writer.write(InboundMessage {
                    token: LOCAL_CONSOLE_TOKEN.to_string(),
                    msg,
                });
            }
            Err(_) => {
                let snippet: String = json.chars().take(80).collect();
                bevy::log::warn!("decode failure from local console: payload={}", snippet);
            }
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
fn flush_console_state(mut reader: Option<MessageReader<ConsoleStateChanged>>) {
    let Some(mut reader) = reader else { return };
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

/// Reads the current screen-shake offset each frame and forwards it to the
/// registered shake callback via `cb.call2(NULL, x, y)`.
///
/// Always fires on every frame (even with (0, 0)) so the JS handler resets
/// the CSS transform when shake ends.
#[cfg(target_arch = "wasm32")]
fn flush_shake_state() {
    let current = SHAKE_OFFSET.with(|slot| *slot.borrow());
    SHAKE_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            let _ = cb.call2(
                &JsValue::NULL,
                &JsValue::from_f64(current.0 as f64),
                &JsValue::from_f64(current.1 as f64),
            );
        }
    });
}

// ── Tests ───────────────────────────────────────────────────────────────────
//
// `apply_pending_toggles` and `DebugToggleKind` are defined outside the
// `target_arch = "wasm32"` gate specifically so this module runs under plain
// `cargo test` on native, with no Bevy App and no wasm_bindgen involved.
#[cfg(test)]
mod tests {
    use super::{apply_pending_toggles, DebugToggleKind};

    /// Queuing a single toggle flips exactly the corresponding flag, exactly
    /// once, and leaves every other flag untouched.
    #[test]
    fn queueing_regions_toggle_flips_only_regions_once() {
        let (mut regions, mut overlay, mut paused, mut damage, mut entities, mut inspector) =
            (false, false, false, false, false, false);

        let pause_changed = apply_pending_toggles(
            [DebugToggleKind::Regions],
            &mut regions,
            &mut overlay,
            &mut paused,
            &mut damage,
            &mut entities,
            &mut inspector,
        );

        assert!(regions, "Regions flag should have flipped to true");
        assert!(!overlay);
        assert!(!paused);
        assert!(!damage);
        assert!(!entities);
        assert!(!inspector);
        assert!(!pause_changed, "pause was not in this batch");
    }

    /// Draining an empty pending set (e.g. the following frame, after the
    /// queue was already drained) must not flip anything again.
    #[test]
    fn draining_empty_set_does_not_flip_again() {
        let (mut regions, mut overlay, mut paused, mut damage, mut entities, mut inspector) =
            (true, false, false, false, false, false);

        let pause_changed = apply_pending_toggles(
            std::iter::empty(),
            &mut regions,
            &mut overlay,
            &mut paused,
            &mut damage,
            &mut entities,
            &mut inspector,
        );

        assert!(regions, "previous state must be preserved, not re-toggled");
        assert!(!pause_changed);
    }

    /// The pause toggle both flips the flag and reports that it changed, so
    /// the caller knows to (un)pause `Time<Virtual>`.
    #[test]
    fn queueing_pause_toggle_flips_paused_and_reports_change() {
        let (mut regions, mut overlay, mut paused, mut damage, mut entities, mut inspector) =
            (false, false, false, false, false, false);

        let pause_changed = apply_pending_toggles(
            [DebugToggleKind::Pause],
            &mut regions,
            &mut overlay,
            &mut paused,
            &mut damage,
            &mut entities,
            &mut inspector,
        );

        assert!(paused);
        assert!(pause_changed);
    }

    /// Multiple distinct toggles queued in the same batch each flip their own
    /// flag independently.
    #[test]
    fn queueing_multiple_distinct_toggles_flips_each_independently() {
        let (mut regions, mut overlay, mut paused, mut damage, mut entities, mut inspector) =
            (false, false, false, false, false, false);

        apply_pending_toggles(
            [
                DebugToggleKind::Overlay,
                DebugToggleKind::Damage,
                DebugToggleKind::EntityInspector,
            ],
            &mut regions,
            &mut overlay,
            &mut paused,
            &mut damage,
            &mut entities,
            &mut inspector,
        );

        assert!(!regions);
        assert!(overlay);
        assert!(!paused);
        assert!(damage);
        assert!(!entities);
        assert!(inspector);
    }

    /// A duplicate variant appearing twice in the same batch (e.g. queued
    /// from a `HashSet` that happened to be built with a duplicate insert)
    /// still only flips the flag once — the function itself dedupes.
    #[test]
    fn duplicate_variant_in_same_batch_flips_only_once() {
        let (mut regions, mut overlay, mut paused, mut damage, mut entities, mut inspector) =
            (false, false, false, false, false, false);

        apply_pending_toggles(
            [DebugToggleKind::Entities, DebugToggleKind::Entities],
            &mut regions,
            &mut overlay,
            &mut paused,
            &mut damage,
            &mut entities,
            &mut inspector,
        );

        assert!(entities, "should have flipped once (false -> true)");
    }
}
