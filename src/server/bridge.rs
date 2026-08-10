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
        AiChatterEvent, AudioConfigChanged, AudioCueEvent, HudStateChanged, LobbyStateChanged,
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
    bevy::{log::LogPlugin, prelude::*, DefaultPlugins},
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
    /// Region wireframes (`DebugRegionsEnabled`).
    Regions,
    /// Modifier debug overlay (`DebugOverlayEnabled`).
    Overlay,
    /// Simulation pause (`SimulationPaused`); also (un)pauses `Time<Virtual>`.
    ///
    /// Reached from the host settings menu's Gameplay tab, which is not
    /// build-gated — so this variant, unlike its neighbours, has a caller that
    /// survives into a demo build.
    Pause,
    /// Damage debug log (`DebugDamageEnabled`).
    Damage,
    /// Entity behavior overlay (`DebugEntitiesEnabled`).
    Entities,
    /// Entity inspector overlay (`DebugEntityInspectorEnabled`).
    EntityInspector,
}

impl From<crate::messages::DebugFlag> for DebugToggleKind {
    /// The wire form of the *diagnostic* half of this enum (issue #940) maps
    /// one-for-one onto it, so an overlay flipped from a phone and one flipped
    /// from the host page converge on the same pending-toggle vocabulary and
    /// the same [`apply_pending_toggles`] call. Exhaustive on purpose: adding a
    /// `DebugFlag` without a kind to carry it is a compile error rather than a
    /// silently ignored toggle.
    ///
    /// [`DebugToggleKind::Pause`] is deliberately outside this conversion's
    /// range. `DebugFlag` has no `Pause` member — a phone reaches the clock
    /// through `ClientMessage::TogglePause`, gated separately — so there is no
    /// wire flag a client could send that stops the simulation through the
    /// overlay drain. `debug_overlay::tests::no_debug_flag_maps_to_the_pause_toggle`
    /// pins that.
    fn from(flag: crate::messages::DebugFlag) -> Self {
        use crate::messages::DebugFlag;
        match flag {
            DebugFlag::Regions => DebugToggleKind::Regions,
            DebugFlag::Modifiers => DebugToggleKind::Overlay,
            DebugFlag::Damage => DebugToggleKind::Damage,
            DebugFlag::Entities => DebugToggleKind::Entities,
            DebugFlag::Inspector => DebugToggleKind::EntityInspector,
        }
    }
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

// ── Host teleport-to-waypoint override (issue #770) ─────────────────────────
//
// A deliberate host-only simulation override: snap the LocalShip's
// authoritative position onto the shared Navigation waypoint. Unlike a client
// helm command it does NOT go through command admission — it directly sets
// `ShipPhysics.{x,z}`, a discontinuous jump contrasted with the helm's velocity
// integration. Kept free of `target_arch = "wasm32"` (though it does touch Bevy
// component types, which compile natively) so the logic is unit-testable under
// plain `cargo test` without a wasm target — the wasm glue (thread-local,
// `wasm_bindgen` export, the Bevy drain system) is gated below.

/// Apply a pending teleport-to-waypoint to one ship's physics.
///
/// Reads the ship's `NavigationWaypoint` snapshot (which resolves BOTH Free and
/// Anchored modes to a live x/z) and, when a waypoint exists, sets the ship's
/// planar position to it. Returns `true` when a teleport happened, `false` when
/// there was no waypoint (a no-op).
///
/// `physics.y` (altitude) is left UNCHANGED on purpose: `WaypointMode` is
/// X/Z-only and carries no altitude, so keeping the ship's current height is the
/// least-surprising behaviour — the ship slides across to the waypoint without
/// changing altitude (issue #768 allows ships to sit at nonzero Y).
pub fn apply_teleport_to_waypoint(
    physics: &mut crate::ship_state::ShipPhysics,
    waypoint: &crate::console::navigation::NavigationWaypoint,
) -> bool {
    match waypoint.snapshot() {
        Some(snapshot) => {
            physics.x = snapshot.x;
            physics.z = snapshot.z;
            // physics.y deliberately unchanged — see the doc comment.
            true
        }
        None => false,
    }
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

    /// Mirror of the `SimulationPaused` resource, written by
    /// `drain_debug_toggles` so `wasm_is_paused()` can answer without a Bevy
    /// world handle. The host settings menu's Gameplay tab reads it each frame
    /// to render its pause/resume affordance (issue #939).
    static SIM_PAUSED: RefCell<bool> = const { RefCell::new(false) };

    /// `?log=` — a category/level spec such as `info,ai=debug,admit=trace`.
    /// Set by JS via `wasm_set_log_spec()` before `wasm_init()`. Parsed by
    /// `crate::logging::parse_log_spec`, the same parser the headless runner's
    /// `--log` flag uses, so the two front ends cannot drift.
    static LOG_SPEC: RefCell<Option<String>> = const { RefCell::new(None) };

    /// `?log_entity=` — comma-separated entity display names to restrict
    /// logging to. Set by JS via `wasm_set_log_entity()` before `wasm_init()`.
    static LOG_ENTITY: RefCell<Option<String>> = const { RefCell::new(None) };

    /// Pending debug-toggle requests queued by the six `wasm_toggle_*`
    /// exports. Drained by `drain_debug_toggles` each `PreUpdate` frame via
    /// `apply_pending_toggles`. Consolidated from six separate
    /// `RefCell<bool>` thread-locals into one enum-keyed set (issue #609).
    static PENDING_TOGGLES: RefCell<HashSet<DebugToggleKind>> = RefCell::new(HashSet::new());

    /// The world this session loaded: `(path, TOML text)`, recorded by
    /// `wasm_load_world`. The snapshot boundary (issue #862) needs both — the
    /// path is `Run::scenario`, and the text is what `snapshot::content_digest`
    /// hashes to produce the content version. Kept here rather than reached for
    /// through `config_cache` so the save path has one obvious source.
    static SNAPSHOT_WORLD: RefCell<Option<(String, String)>> = const { RefCell::new(None) };

    /// Slot name queued by `wasm_save_snapshot`, taken by `drain_snapshot_save`
    /// on the next `PostUpdate`.
    ///
    /// `PostUpdate` rather than the export itself, because a save has to be
    /// taken between fixed steps: `SimRng::state`'s own docs say why — mid-tick,
    /// some systems for the step have drawn and others have not, so "all six
    /// streams right now" is not a point any system agrees on. A JS click can
    /// land anywhere; a Bevy system in `PostUpdate` cannot.
    static PENDING_SAVE: RefCell<Option<String>> = const { RefCell::new(None) };

    /// Host-visible outcome of the last save or resume, `(succeeded, message)`,
    /// **drained** by `wasm_snapshot_status()`.
    ///
    /// Drained rather than latched because the host page polls it: a status
    /// that stayed set would be re-shown every poll, and one that was cleared
    /// on a timer could be missed entirely. Taking it means each outcome is
    /// reported exactly once, whoever asks first.
    static SNAPSHOT_STATUS: RefCell<Option<(bool, String, String)>> =
        const { RefCell::new(None) };

    /// A save that passed the version gate and is waiting for the world to
    /// finish bootstrapping. Applied by `drain_snapshot_restore`.
    static PENDING_RESTORE: RefCell<Option<crate::snapshot::StoredRun>> =
        const { RefCell::new(None) };

    /// Frames `drain_snapshot_restore` has been waiting for `ready_to_restore`.
    /// Reset when a save is staged; compared against
    /// [`RESTORE_DEADLINE_FRAMES`].
    static RESTORE_WAITED: RefCell<u32> = const { RefCell::new(0) };

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
    /// `drain_force_start_input` each `PreUpdate` frame into the
    /// `PendingForceStart` resource; `apply_force_start` (in `FixedUpdate`,
    /// issue #907) is what actually transitions to `InProgress` without any
    /// connected players (fully AI-crewed ship).
    static PENDING_FORCE_START: RefCell<bool> = const { RefCell::new(false) };

    /// Pending host teleport-to-waypoint request from
    /// `wasm_teleport_to_waypoint()` (issue #770). Drained by
    /// `drain_teleport_to_waypoint` each `PreUpdate` frame: a deliberate
    /// host-only simulation override that snaps the LocalShip's authoritative
    /// position to the shared Navigation waypoint. NOT routed through command
    /// admission — this is a direct sim mutation, the point of the control.
    static PENDING_TELEPORT_TO_WAYPOINT: RefCell<bool> = const { RefCell::new(false) };

    /// Whether the LocalShip currently has a shared Navigation waypoint set.
    /// Written each tick by `publish_waypoint_existence`, read back by
    /// `wasm_has_navigation_waypoint()` so the host Debug panel can disable the
    /// teleport control when there is nowhere to teleport to (issue #770, AC2).
    static HAS_NAVIGATION_WAYPOINT: RefCell<bool> = const { RefCell::new(false) };

    /// The logical simulation tick count (issue #895), mirrored each frame by
    /// `publish_sim_tick` and read back by `wasm_sim_tick()` so the smoke
    /// tests can observe the fixed tick advancing independently of the frame
    /// rate — the Rust suite cannot see the browser's frame loop.
    static SIM_TICK_COUNT: RefCell<u64> = const { RefCell::new(0) };

    /// The single Host Channel callback registered by the host page (issue
    /// #818). Signature: `callback(name: string, payload: any)` where `name`
    /// is one of [`host_channels::ALL`] and `payload` is a JSON string for the
    /// message-drained channels, a bare number for `audio_level`, and a
    /// two-element `[x, y]` array for `shake`. Replaces the eight per-channel
    /// callback slots + `set_*_callback` exports.
    static HOST_CHANNEL_CB: RefCell<Option<Function>> = const { RefCell::new(None) };

    /// Latest screen shake offset (x, y) in CSS pixels, written by
    /// [`viewscreen_border::apply_camera_shake`] each frame and read by
    /// [`flush_host_channels`] for the JS callback.
    static SHAKE_OFFSET: RefCell<(f32, f32)> = const { RefCell::new((0.0, 0.0)) };

    /// Latest forcefield SFX volume, written by
    /// [`server::audio::drive_forcefield_level`] each frame and read by
    /// [`flush_host_channels`].
    static FORCEFIELD_LEVEL: RefCell<f32> = const { RefCell::new(0.0) };

    /// Last forcefield level handed to the `audio_level` host channel. Unlike
    /// the shake offset (which fires unconditionally so JS can reset its
    /// transform), a `.volume` write that changes nothing is pure overhead at
    /// 60 Hz — so [`flush_host_channels`] emits `audio_level` only when the
    /// level actually moves. Starts at a sentinel no real level can equal, so
    /// the first flush always fires.
    static LAST_SENT_FORCEFIELD: RefCell<f32> = const { RefCell::new(-1.0) };

    /// Template path of the player ship selected by the host. Set by
    /// `wasm_select_ship()` before `wasm_init()`. When absent, defaults
    /// to `"assets/entities/alliance_cruiser.toml"`.
    static SELECTED_SHIP_TEMPLATE_PATH: RefCell<Option<String>> =
        const { RefCell::new(None) };

    /// Instagib: local ship deals 100× damage.
    static INSTAGIB: RefCell<bool> = const { RefCell::new(false) };

    /// Pending God Mode toggle requests from `wasm_toggle_god_mode()` (issue
    /// #900). Drained by `drain_god_mode_toggle` each `PreUpdate` frame, which
    /// turns each one into a `ToggleGodMode` `InboundMessage` under
    /// `LOCAL_CONSOLE_TOKEN` — the same command-admission boundary every other
    /// host command crosses — rather than writing a bool directly. A count
    /// (not a single bool) so two clicks in one frame toggle twice, matching
    /// what two separate admitted commands on two different ticks would do.
    static PENDING_GOD_MODE_TOGGLES: RefCell<u32> = const { RefCell::new(0) };

    /// Mirrors the authoritative `GodMode` resource each frame so
    /// `wasm_get_god_mode()` can read it back without touching the Bevy World
    /// from outside a system (issue #900). Written by `publish_god_mode`. Same
    /// pattern as `SIM_TICK_COUNT`/`HAS_NAVIGATION_WAYPOINT`.
    static GOD_MODE_MIRROR: RefCell<bool> = const { RefCell::new(false) };
}

// ── Host Channels (issue #818) ─────────────────────────────────────────────
//
// Named host-page-local outbound channels (CONTEXT.md "Host Channel"). These
// feed `server.html` chrome only — they never reach peers and must NOT be
// folded into `ServerMessage`. One flush system (`flush_host_channels`,
// wasm-gated below) drains every channel and hands `(name, payload)` to the
// single JS callback registered via `set_host_channel_callback`.
//
// Adding a host channel = add a name const here (and to `ALL`), drain it in
// `flush_host_channels`, and add one handler entry to the `__hostChannel`
// dispatcher table in `server.html`.
//
// The names are ungated so native `cargo test` can pin the table's shape.
pub mod host_channels {
    /// Viewscreen HUD state — JSON string (`codec::encode_hud_state`).
    pub const HUD: &str = "hud";
    /// Lobby overlay state — JSON string (`codec::encode_lobby_state`).
    pub const LOBBY: &str = "lobby";
    /// AI→AI chatter events — JSON string (`codec::encode_chatter`).
    pub const CHATTER: &str = "chatter";
    /// Merged ship + world audio config — JSON string
    /// (`codec::encode_audio_config`), sent once on game start.
    pub const AUDIO_CONFIG: &str = "audio_config";
    /// One-shot positional audio cues — JSON string
    /// (`codec::encode_audio_cue`).
    pub const AUDIO_CUE: &str = "audio_cue";
    /// Screen-shake offset — two-element `[x, y]` array of CSS pixels,
    /// emitted every frame (`[0, 0]` when idle so JS resets its transform).
    pub const SHAKE: &str = "shake";
    /// Forcefield SFX volume — bare number in 0.0–1.0, emitted only when the
    /// level moves by at least the audible epsilon.
    pub const AUDIO_LEVEL: &str = "audio_level";

    /// Every registered host channel name. The JS dispatcher table in
    /// `server.html` must have a handler per entry.
    pub const ALL: [&str; 7] = [
        HUD,
        LOBBY,
        CHATTER,
        AUDIO_CONFIG,
        AUDIO_CUE,
        SHAKE,
        AUDIO_LEVEL,
    ];
}

// ── Instagib helper ─────────────────────────────────────────────────────────
//
// Unlike God Mode (issue #900), Instagib is not yet routed through command
// admission — it stays a direct thread-local toggle, out of scope for #900.

pub fn is_instagib() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        INSTAGIB.with(|v| *v.borrow())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// Called by JS (host Debug panel God Mode button) to request a God Mode
/// flip (issue #900). Unlike the old thread-local this does NOT flip
/// anything itself: it queues a request that `drain_god_mode_toggle` turns
/// into a `ToggleGodMode` command crossing the normal admission boundary on
/// the next `PreUpdate` frame, so the flip carries a tick, lands in the
/// command log, and replays. The JS binding's signature is unchanged.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_god_mode() {
    PENDING_GOD_MODE_TOGGLES.with(|v| *v.borrow_mut() += 1);
}

/// Called by JS to read the LocalShip's current God Mode state (issue #900),
/// e.g. to reflect it on the Debug panel button. Reads the mirror maintained
/// by `publish_god_mode`, since the authoritative value now lives in the
/// `GodMode` Bevy resource rather than a thread-local this function can touch
/// directly.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_god_mode() -> bool {
    GOD_MODE_MIRROR.with(|v| *v.borrow())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_instagib() {
    INSTAGIB.with(|v| {
        let current = *v.borrow();
        *v.borrow_mut() = !current;
    });
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_instagib() -> bool {
    INSTAGIB.with(|v| *v.borrow())
}

// ── Public WASM API ────────────────────────────────────────────────────────

/// The host page's pre-start ship gate, without the wasm plumbing.
///
/// Given the template path the host is about to fly and the raw bytes it
/// fetched from that path, produce the validated
/// [`ShipConfig`](crate::ship::config::ShipConfig) — or the reason the hull
/// cannot be flown.
///
/// # Why this resolves rather than parsing the delivered text
///
/// The text JS fetches is the hull's **authored** document, which since issue
/// #875 may declare `includes`. `EntityConfig` is `deny_unknown_fields`, so
/// parsing that text directly rejects every composed hull — and the document
/// the game actually runs is the RESOLVED one, so parsing the authored text
/// would validate a document that is not the one being validated for.
/// Resolution goes through [`crate::entity_includes::HostFragmentSource`], the
/// one source that compiles on both targets: on WASM it reads the raw templates
/// the host has already delivered, on native it falls through to the filesystem.
///
/// # Why the fragments are guaranteed to be there
///
/// This runs from `finishInit()`, and `finishInit()` is only ever reached when
/// `wasm_load_config` reports preload complete — which requires the preload
/// queue AND the in-flight set to be empty. A composed hull's fragments are
/// queued through that same pair (`config_cache::wasm_load_config` feeds
/// `preload_step`'s `AwaitingIncludes` back into `queue_and_fire`), and every
/// selectable hull is in the preload set because `world::config`'s
/// `entity_template_paths` walks `available_ships[*].template_path`. So by the
/// time the gate runs, the hull's whole include closure is in
/// `RAW_TEMPLATE_TOML`. An unresolved include here is therefore a real fault,
/// not a race, and is reported as one — the gate is not allowed to shrug.
///
/// The delivered text is recorded only when the host has NOT already delivered
/// that path, so a hull the preload never queued (a world with no
/// `[[available_ships]]`, which falls back to a hard-coded hull) can still be
/// validated, while a mod pack's overridden bytes — recorded by
/// `wasm_load_config`, which applies the overlay — are never clobbered by the
/// plain HTTP text fetched here.
///
/// Parses through `EntityConfig` (not the raw `ShipConfig` parser) so that
/// `[[shield_arc]]` blocks are synthesised into their matching `[[system]]`
/// entries before validation. Ships whose ratings reference a synthesised arc
/// system (e.g. the Courier's single "Std" rating automating
/// `shield-arc-fore`/`shield-arc-aft`) would otherwise fail validation here even
/// though the real in-game config is valid.
///
/// Ungated, and free of Bevy and wasm_bindgen types, so `cargo test` can drive
/// the browser's gate over every shipped hull — see
/// `every_shipped_hull_passes_the_browser_station_gate`.
pub fn validate_ship_stations(
    template_path: &str,
    toml_str: &str,
) -> Result<crate::ship::config::ShipConfig, String> {
    if !crate::config_cache::is_raw_template_delivered(template_path) {
        crate::config_cache::record_raw_template(template_path, toml_str.to_string());
    }
    let resolved = crate::entity_includes::resolve_template(
        template_path,
        &crate::entity_includes::HostFragmentSource,
    )
    .map_err(|e| format!("Station config validation failed: {e}"))?;
    let entity_config = resolved
        .parse()
        .map_err(|e| format!("Station config validation failed: {e}"))?;
    entity_config.ship_config.ok_or_else(|| {
        "Station config validation failed: ship has no [[station]] blocks".to_string()
    })
}

/// Called by JS with the chosen ship template path and the raw TOML content it
/// fetched from that path, to validate the `[[station]]`/`[[system]]` schema
/// before starting the server.
///
/// The path is load-bearing: it is what the include closure is resolved
/// against. See [`validate_ship_stations`] for why the delivered text alone is
/// not enough.
///
/// On success, stores the parsed `ShipStations` internally and returns
/// `Ok(JsValue::UNDEFINED)`. On failure, returns `Err(JsValue)` with a
/// human-readable error string. PeerJS should not start when this returns
/// an error.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_validate_stations(template_path: &str, toml_str: &str) -> Result<JsValue, JsValue> {
    let ship_config =
        validate_ship_stations(template_path, toml_str).map_err(|e| JsValue::from_str(&e))?;
    let stations = crate::stations_config::stations_from_ship_config(&ship_config);
    SHIP_STATIONS.with(|slot| {
        *slot.borrow_mut() = Some(stations);
    });
    SHIP_CONFIG.with(|slot| {
        *slot.borrow_mut() = Some(ship_config);
    });
    Ok(JsValue::UNDEFINED)
}

/// Called by JS on page load. Builds and runs the Bevy app.
///
/// In WASM, `App::run()` hands control to requestAnimationFrame and returns
/// immediately, so this function does not block.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_init() {
    // Fix Rhai's global hashing seed FIRST, before anything can build a script
    // engine (issue #979, M0 spike `rhai-anonymous-function-naming`):
    // `set_hashing_seed` silently no-ops once a hash has been taken, so it must
    // be genuinely first on every peer. Idempotent — see `world::script`.
    crate::world::script::init_hashing_seed();

    // Issue #935: the preload sequence (`wasm_load_world` -> N x
    // `wasm_load_config`) is finished by the time JS calls this — that is the
    // whole point of the "preload complete" handshake `config_cache` runs.
    // Freezing the content ledger here, before anything spawns, is what keeps
    // the content digest independent of how far the world streams afterward:
    // see `content_ledger`'s module docs.
    crate::content_ledger::freeze();

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

    // Boot clock starts here, before any plugin is added, and stops when the
    // app is handed to the frame loop (issue #868). `is_automation` is passed
    // through because it decides what the capture means: under WebDriver the
    // render stack below is skipped, so the frame metric measures the ECS
    // schedule and not rendering.
    crate::perf::browser::boot_begin(is_automation);

    // Read `?log=` / `?log_entity=` before any plugin is added: `LogPlugin`
    // takes its filter at construction, and the resource must be in place
    // before the first system that logs runs.
    let (log_config, log_spec) = log_config_from_url();
    let log_filter = if log_spec.is_empty() {
        LogPlugin::default().filter
    } else {
        format!("{},{}", LogPlugin::default().filter, log_spec)
    };

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
            LogPlugin {
                filter: log_filter.clone(),
                ..default()
            },
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
            AssetPlugin {
                // No .meta sidecars ship with this project. Cloudflare Pages
                // (the demo host) answers a missing file with its SPA
                // index.html at HTTP 200 rather than a 404, so the default
                // `AssetMetaCheck::Always` reads that HTML as a .meta and
                // fails to deserialize it — killing the asset load. Never
                // requesting the sidecar sidesteps the whole class.
                meta_check: bevy::asset::AssetMetaCheck::Never,
                ..default()
            },
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
            .add_message::<crate::console_bridge::LobbyStateChanged>()
            .add_message::<crate::console_bridge::AiChatterEvent>();
    } else {
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: Some(bevy::window::Window {
                        canvas: Some("#canvas".into()),
                        fit_canvas_to_parent: true,
                        ..default()
                    }),
                    ..default()
                })
                .set(LogPlugin {
                    filter: log_filter.clone(),
                    ..default()
                })
                // No .meta sidecars ship with this project. Cloudflare Pages
                // (the demo host) answers a missing file with its SPA
                // index.html at HTTP 200 rather than a 404, so the default
                // `AssetMetaCheck::Always` reads that HTML as a .meta and
                // fails to deserialize it — the asset load dies and the
                // preload gate stalls (issue: demo hangs ~77%). Never
                // requesting the sidecar sidesteps the whole class.
                .set(bevy::asset::AssetPlugin {
                    meta_check: bevy::asset::AssetMetaCheck::Never,
                    ..default()
                }),
        );
    }
    app.insert_resource(log_config)
        .add_plugins(crate::logging::LoggingPlugin);
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
            .unwrap_or_else(|| "assets/entities/alliance_cruiser.toml".to_string());
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

    // Audio is plain data + JS callbacks with no wgpu dependency, so unlike
    // ViewscreenBorderPlugin it is safe to register in automation mode — and
    // registering it in both branches means the smoke tests actually exercise
    // it. The plugin registers its own bridge messages, which the PostUpdate
    // flushes need in either branch.
    app.add_plugins(crate::server::audio::ServerAudioPlugin);

    // Always add the debug overlay plugin; ?debug_regions=1 sets initial state.
    // Runtime toggling via the settings cog's Debug/Cheat tab is handled by
    // drain_debug_toggles.
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
    .init_resource::<PendingForceStart>()
    .add_systems(
        PreUpdate,
        (
            drain_inbound,
            drain_disconnects,
            drain_debug_toggles,
            drain_force_start_input,
            drain_teleport_to_waypoint,
            drain_god_mode_toggle,
            publish_waypoint_existence,
        ),
    )
    // `apply_force_start` writes `NextState<GamePhase>`, so it lives in
    // `FixedUpdate` rather than alongside its own input drain above — see the
    // #907 review note on `apply_force_start` for why.
    .add_systems(
        FixedUpdate,
        apply_force_start.before(crate::sim_sets::SimSet::Input),
    )
    // The JS ingress/egress seams stay frame-driven (issue #895): `PreUpdate`
    // runs before the fixed loop and `PostUpdate` after it, so a frame drains
    // inbound messages before any of its sim ticks and flushes everything
    // those ticks broadcast. Bevy defers message cleanup until the fixed
    // schedules have observed a frame's messages, so a frame that runs zero
    // fixed steps loses nothing.
    .add_systems(
        PostUpdate,
        (
            flush_outbound,
            flush_host_channels,
            publish_sim_tick,
            publish_god_mode,
            publish_debug_mirrors,
            // The snapshot seam (issue #862). `PostUpdate` for the same reason
            // the rest of this list is there — it runs *after* the frame's
            // fixed steps, which is the tick boundary a capture and a restore
            // both have to stand on.
            drain_snapshot_save,
            drain_snapshot_restore,
        ),
    );

    // Insert the validated ShipStations resource if it was pre-validated.
    SHIP_STATIONS.with(|slot| {
        if let Some(stations) = slot.borrow().clone() {
            app.insert_resource(stations);
        }
    });

    // Frame sampling brackets each animation frame's schedule. These two
    // systems read and write nothing in the world — they move a thread-local
    // clock — so they observe the frame without participating in it.
    fn sample_frame_begin() {
        crate::perf::browser::frame_begin();
    }
    fn sample_frame_end() {
        crate::perf::browser::frame_end();
    }
    app.add_systems(bevy::app::First, sample_frame_begin);
    app.add_systems(bevy::app::Last, sample_frame_end);

    crate::perf::browser::boot_end();
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

/// Called by JS once to register the single Host Channel callback (issue
/// #818). Bevy calls `callback(name: string, payload: any)` from
/// [`flush_host_channels`] for every host-page channel:
///
/// - `"hud"`, `"lobby"`, `"chatter"`, `"audio_config"`, `"audio_cue"` —
///   `payload` is a JSON string.
/// - `"shake"` — `payload` is a two-element `[x, y]` array (CSS pixels),
///   emitted every frame.
/// - `"audio_level"` — `payload` is a bare number in 0.0–1.0, emitted on
///   change only.
///
/// Must be registered before `wasm_init()` so the first push is never missed.
/// JS must not assume any cross-channel ordering.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_host_channel_callback(callback: Function) {
    HOST_CHANNEL_CB.with(|slot| {
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

/// Called by [`crate::server::audio::drive_forcefield_level`] (WASM builds
/// only) to store the current frame's forcefield SFX volume for JS.
#[cfg(target_arch = "wasm32")]
pub fn set_forcefield_level(level: f32) {
    FORCEFIELD_LEVEL.with(|slot| {
        *slot.borrow_mut() = level;
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

/// Called by JS to set the log category/level spec from `?log=` in the URL.
/// Must be called before `wasm_init()` to take effect.
///
/// Same syntax as the headless runner's `--log`, e.g.
/// `?log=info,ai=debug,admit=trace`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_set_log_spec(spec: &str) {
    LOG_SPEC.with(|v| *v.borrow_mut() = Some(spec.to_string()));
}

/// Called by JS to restrict logging to named entities, from `?log_entity=`.
/// Must be called before `wasm_init()` to take effect.
///
/// Comma-separated display names, matched exactly then case-insensitively as a
/// substring — e.g. `?log_entity=Ironveil,Ashrender`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_set_log_entity(names: &str) {
    LOG_ENTITY.with(|v| *v.borrow_mut() = Some(names.to_string()));
}

/// Build the [`LogFilterConfig`] for this page from the `?log=` / `?log_entity=`
/// thread-locals. Also returns the raw spec so `wasm_init` can hand it to
/// `LogPlugin`'s own `EnvFilter`.
///
/// A malformed spec warns and falls back to the default rather than aborting
/// startup — a typo in a debug URL parameter should not stop the game booting.
#[cfg(target_arch = "wasm32")]
fn log_config_from_url() -> (crate::logging::LogFilterConfig, String) {
    let spec = LOG_SPEC.with(|v| v.borrow().clone()).unwrap_or_default();
    let mut config = match crate::logging::parse_log_spec(&spec) {
        Ok(cfg) => cfg,
        Err(e) => {
            bevy::log::warn!("?log= is malformed ({e}); ignoring it");
            crate::logging::LogFilterConfig::default()
        }
    };
    if let Some(names) = LOG_ENTITY.with(|v| v.borrow().clone()) {
        config.entity_filter = crate::logging::parse_log_entities(&names);
    }
    (config, spec)
}

// ── The snapshot seam (issue #862) ─────────────────────────────────────────
//
// Four exports and two systems, and the shape of them is dictated by what a
// browser resume actually is.
//
// **Saving** is easy: queue a slot, and let a `PostUpdate` system take the
// capture at a tick boundary and hand the RON to `vellum_save::Store`.
//
// **Resuming is a page load.** "Restore into a fresh app" has exactly one
// honest meaning in a browser: a fresh `App`, and the only way this page gets
// one is to reload. So the host page's resume button does not restore anything
// — it sets `?resume=<slot>` and reloads. On the way back up, JS calls
// `wasm_prepare_resume` BEFORE `wasm_init`, which reads the slot and puts it
// through the version gate; if the gate refuses, the page is told so and boots
// normally, having activated nothing. If it passes, the save waits in a
// thread-local until the scenario has bootstrapped its roster, and
// `drain_snapshot_restore` writes it over the top.
//
// The gate running before `wasm_init` rather than after is the whole point: a
// host must never be half-way into a world it is about to be told it cannot
// have.

/// Queue a save of the running session into `slot`.
///
/// Returns immediately; the capture happens on the next `PostUpdate`, and the
/// outcome is read back through [`wasm_snapshot_status`].
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_save_snapshot(slot: String) {
    PENDING_SAVE.with(|s| *s.borrow_mut() = Some(slot));
}

/// Which button an outcome answers. Carried so the host page can put the
/// answer back on the control that was pressed rather than guessing from the
/// wording of a sentence it is not allowed to paraphrase.
#[cfg(target_arch = "wasm32")]
const SNAPSHOT_SAVE: &str = "save";
#[cfg(target_arch = "wasm32")]
const SNAPSHOT_RESUME: &str = "resume";

/// Record a host-visible outcome for the next [`wasm_snapshot_status`] poll.
#[cfg(target_arch = "wasm32")]
fn set_snapshot_status(ok: bool, source: &str, message: impl Into<String>) {
    SNAPSHOT_STATUS.with(|s| *s.borrow_mut() = Some((ok, source.to_string(), message.into())));
}

/// The host-visible outcome of the last save or resume, **taken** — each
/// outcome is reported exactly once.
///
/// Returns `""` when there is nothing to report, else
/// `"<ok|error>\t<save|resume>\t<message>"`. Tab-separated rather than a status
/// *object* because this crosses a `wasm_bindgen` boundary into a
/// classic-script host page, and one string is the cheapest thing that crosses
/// it; the host page splits on the first two tabs. No field but the message can
/// contain one.
///
/// For a refused resume the message is `vellum_save::Moved`'s own sentence,
/// verbatim — phoenix has no status vocabulary of its own to render it in.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_snapshot_status() -> String {
    SNAPSHOT_STATUS.with(|s| {
        s.borrow_mut()
            .take()
            .map_or_else(String::new, |(ok, source, message)| {
                format!("{}\t{source}\t{message}", if ok { "ok" } else { "error" })
            })
    })
}

/// Read `slot`, put it through the version gate, and hold it for the boot that
/// is about to happen. Call BEFORE `wasm_init`.
///
/// Returns `""` when the save was accepted and is now pending, or the refusal
/// to show the host. A refusal leaves nothing staged, so the page boots into a
/// normal new session.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_prepare_resume(slot: String) -> String {
    let Some((_, _toml)) = SNAPSHOT_WORLD.with(|w| w.borrow().clone()) else {
        // No world means no content digest, so there is nothing to check the
        // save against. Refusing beats guessing.
        return "the scenario has not been loaded yet".to_string();
    };
    let store = vellum_save::LocalStorage::new(crate::snapshot::STORAGE_NAMESPACE);
    let versions = crate::snapshot::versions(&crate::content_ledger::frozen_or_live());
    match crate::snapshot::load_from(&store, &slot, &versions) {
        Ok(run) => {
            PENDING_RESTORE.with(|p| *p.borrow_mut() = Some(run));
            RESTORE_WAITED.with(|w| *w.borrow_mut() = 0);
            SNAPSHOT_STATUS.with(|s| *s.borrow_mut() = None);
            String::new()
        }
        Err(refusal) => {
            // Returned rather than queued: this call is synchronous and the
            // host page has the string in hand, so queuing it too would show
            // the same refusal twice.
            refusal.to_string()
        }
    }
}

/// Whether a save is staged and waiting for the world to finish bootstrapping.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_resume_pending() -> bool {
    PENDING_RESTORE.with(|p| p.borrow().is_some())
}

/// Take a queued save, if there is one.
#[cfg(target_arch = "wasm32")]
fn drain_snapshot_save(world: &mut World) {
    let Some(slot) = PENDING_SAVE.with(|s| s.borrow_mut().take()) else {
        return;
    };
    let Some((path, _toml)) = SNAPSHOT_WORLD.with(|w| w.borrow().clone()) else {
        set_snapshot_status(
            false,
            SNAPSHOT_SAVE,
            "no scenario is loaded, so there is nothing to save",
        );
        return;
    };
    // A save of `Lobby` or `Loading` is meaningless — there is no run in
    // progress to resume, and `Loading` in particular is a one-shot asset
    // preload wait a restore would land back inside of. Refusing here (issue
    // #934) rather than in `snapshot::capture` keeps `capture` a pure "walk
    // whatever the world holds" reader — this is a policy about *when* a save
    // button should work, and it belongs beside the button, not the walk.
    let phase = world
        .get_resource::<bevy::prelude::State<messages::GamePhase>>()
        .map(|s| s.get().clone());
    if !matches!(
        phase,
        Some(messages::GamePhase::InProgress) | Some(messages::GamePhase::GameOver)
    ) {
        set_snapshot_status(false, SNAPSHOT_SAVE, "there is no run in progress to save");
        return;
    }
    let payload = crate::snapshot::capture(world);
    let digest = crate::sim_digest::world_digest(world);
    let seed = world
        .get_resource::<crate::sim_rng::SimRng>()
        .map_or(0, |rng| rng.seed());
    let run = crate::snapshot::run_for(
        payload,
        digest,
        seed,
        path,
        crate::snapshot::versions(&crate::content_ledger::frozen_or_live()),
    );
    let store = vellum_save::LocalStorage::new(crate::snapshot::STORAGE_NAMESPACE);
    // The failure worth naming is `QuotaExceededError`: a save is one RON
    // string in `localStorage`, and a long bounded run is a big one. The store
    // hands the browser's own exception text back, and it is reported as-is —
    // "the save could not be written: QuotaExceededError" says more to whoever
    // has to clear space than any phoenix paraphrase of it would.
    match crate::snapshot::save_to(&store, &slot, &run) {
        Ok(()) => set_snapshot_status(
            true,
            SNAPSHOT_SAVE,
            format!("saved at tick {}", run.ledger.final_tick),
        ),
        Err(why) => set_snapshot_status(
            false,
            SNAPSHOT_SAVE,
            format!("the save could not be written: {why}"),
        ),
    }
}

/// How long a staged save waits for the world to bootstrap before the restore
/// is abandoned and the host is told.
///
/// Frames rather than ticks, because this is a *wall-clock* patience budget for
/// something that has not started ticking yet, and a world that never
/// bootstraps never advances the tick this would otherwise be counted in. At
/// 60fps this is thirty seconds — an order of magnitude past the second or two
/// a normal auto-start takes, and short enough that a host does not sit
/// wondering.
#[cfg(target_arch = "wasm32")]
const RESTORE_DEADLINE_FRAMES: u32 = 1_800;

/// Apply a staged save once the scenario's roster exists.
///
/// Runs every frame while something is staged and does nothing until
/// `ready_to_restore` says the world is far enough along — a fresh app has no
/// ships at tick 0, and restoring into that window writes a ship's state onto
/// components it has not been given yet.
///
/// # The wait is bounded, and the expiry is loud
///
/// `ready_to_restore` can be false forever, and the way it happens is ordinary:
/// a stale `?resume=` outlives its session, the host picks a different roster
/// at boot, and the save then names ships this world will never spawn. Waiting
/// silently for that is the worst available outcome — the page plays a
/// perfectly good *fresh* session while the host believes they resumed, and
/// nothing ever says otherwise. So the wait has a deadline, and reaching it
/// clears the staged save and reports a failure through the same status the
/// save button uses.
#[cfg(target_arch = "wasm32")]
fn drain_snapshot_restore(world: &mut World) {
    let staged = PENDING_RESTORE.with(|p| p.borrow().clone());
    let Some(run) = staged else {
        return;
    };
    let Some(snapshot) = run.snapshot.as_ref() else {
        PENDING_RESTORE.with(|p| *p.borrow_mut() = None);
        set_snapshot_status(
            false,
            SNAPSHOT_RESUME,
            "that save carries no captured state to resume from",
        );
        return;
    };
    if !crate::snapshot::ready_to_restore(world, &snapshot.state) {
        let waited = RESTORE_WAITED.with(|w| {
            let mut w = w.borrow_mut();
            *w += 1;
            *w
        });
        if waited >= RESTORE_DEADLINE_FRAMES {
            PENDING_RESTORE.with(|p| *p.borrow_mut() = None);
            set_snapshot_status(
                false,
                SNAPSHOT_RESUME,
                format!(
                    "this session never built the world that save was taken in, \
                     so the resume was abandoned and you are playing a fresh \
                     session from tick 0 (the save wanted {} ship(s) at tick {})",
                    snapshot.state.entities.len(),
                    snapshot.tick
                ),
            );
        }
        return;
    }
    let report = crate::snapshot::restore(world, &snapshot.state);
    PENDING_RESTORE.with(|p| *p.borrow_mut() = None);

    let restored = crate::sim_digest::world_digest(world);
    if restored != snapshot.digest {
        // The corruption check, and it is vellum's rather than a hash of the
        // text: a snapshot's digest is recomputed BY the restored simulation,
        // so tampered or truncated state cannot restore to the recorded number.
        set_snapshot_status(
            false,
            SNAPSHOT_RESUME,
            format!(
                "the save did not restore cleanly (recorded {:016x}, restored {restored:016x})",
                snapshot.digest
            ),
        );
    } else if report.is_complete() {
        set_snapshot_status(
            true,
            SNAPSHOT_RESUME,
            format!("resumed at tick {}", snapshot.tick),
        );
    } else {
        set_snapshot_status(
            false,
            SNAPSHOT_RESUME,
            format!(
                "resumed at tick {} with {} missing entities",
                snapshot.tick,
                report.gaps.len()
            ),
        );
    }
}

/// Called by JS (the settings cog's Debug/Cheat tab) to toggle region
/// wireframes at runtime.
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

/// Called by JS (the settings cog's Debug/Cheat tab) to toggle the modifier
/// debug overlay at runtime.
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

/// Called by JS to pause/unpause the simulation clock.
///
/// Sets a pending flag that is consumed by `drain_debug_toggles` in the next
/// `PreUpdate` frame, which pauses or unpauses `Time<Virtual>`.
///
/// Named without `debug` (issue #939) because its only caller is the host
/// settings menu's **Gameplay** tab, which ships in the demo build where the
/// Debug/Cheat tab is gone. Nothing on this path is gated by
/// `PHOENIX_DEMO_BUILD`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_pause() {
    PENDING_TOGGLES.with(|set| {
        set.borrow_mut().insert(DebugToggleKind::Pause);
    });
}

/// Called by JS each frame to read back whether the simulation clock is
/// paused, so the settings menu can render pause vs. resume.
///
/// Reads the `SIM_PAUSED` mirror rather than the resource: the toggle applies
/// a frame later, in `PreUpdate`, so a synchronous read-back right after the
/// click would report the stale value (same reasoning as `wasm_get_god_mode`).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_is_paused() -> bool {
    SIM_PAUSED.with(|v| *v.borrow())
}

/// Called by JS to ask whether this page was built by the public demo deploy
/// (`PHOENIX_DEMO_BUILD=true`).
///
/// The host settings menu hides its Debug/Cheat tab when this is true (issue
/// #939). See `crate::build_flags` for why this is its own flag rather than
/// `TRUNK_BUILD_RELEASE` (which the dev host also sets) or `debug_assertions`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_is_demo_build() -> bool {
    crate::build_flags::is_demo_build()
}

/// Called by JS (the settings cog's Debug/Cheat tab) to toggle the damage
/// debug overlay at runtime.
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

/// Called by JS (the settings cog's Debug/Cheat tab) to toggle the entity
/// behavior overlay at runtime.
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

/// Called by JS (the settings cog's Debug/Cheat tab) to toggle the entity
/// inspector overlay at runtime.
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
/// flag is drained into a Bevy resource by `drain_force_start_input` on the
/// next `PreUpdate` frame; the actual phase transition is applied by
/// `apply_force_start` on the next `FixedUpdate` step (issue #907 — see that
/// function's doc for why the transition itself needs to be tick-scoped).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_force_start() {
    PENDING_FORCE_START.with(|v| *v.borrow_mut() = true);
}

/// Called by JS (host Debug panel) to teleport the local ship onto the shared
/// Navigation waypoint (issue #770). A host-only simulation override, not a
/// client command: it sets a pending flag consumed by
/// `drain_teleport_to_waypoint` on the next `PreUpdate`, which directly writes
/// the LocalShip's authoritative `ShipPhysics.{x,z}`. Deliberately bypasses
/// command admission — this is a debug override, never replicated to clients.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_teleport_to_waypoint() {
    PENDING_TELEPORT_TO_WAYPOINT.with(|v| *v.borrow_mut() = true);
}

/// Called by JS each animation frame to check whether the LocalShip currently
/// has a shared Navigation waypoint (issue #770, AC2). The host Debug panel
/// disables the teleport control while this returns `false`. Reads back the
/// value maintained by `publish_waypoint_existence`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_has_navigation_waypoint() -> bool {
    HAS_NAVIGATION_WAYPOINT.with(|v| *v.borrow())
}

/// The logical simulation tick count (issue #895) — the number of completed
/// `FixedUpdate` steps. Read back from the mirror maintained by
/// `publish_sim_tick` each frame.
///
/// Returned as `f64` so JS receives a plain number rather than a `BigInt`;
/// at 60 Hz the count stays exactly representable for ~4.7 million years.
/// The smoke tests sample this twice to assert the sim advances on the
/// authored `[global] sim_tick_hz` clock rather than the rendered frame rate.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_sim_tick() -> f64 {
    SIM_TICK_COUNT.with(|v| *v.borrow()) as f64
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
    // The preload has no single entry point — JS drives it one config at a
    // time — so the clock starts on the first one and stops when the page
    // first observes it complete (issue #868).
    crate::perf::browser::preload_begin_once();
    crate::config_cache::wasm_load_config(path, toml_str)
}

/// Check if preload is complete.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_is_preload_complete() -> bool {
    let complete = crate::config_cache::wasm_is_preload_complete();
    if complete {
        crate::perf::browser::preload_end_once();
    }
    complete
}

/// Unified world loader: a single TOML file containing anchors, immediate
/// entity instances (asteroid fields, stations, NPCs, etc.), named [[entity]]
/// instances for trigger / comms anchors, [[trigger]] blocks, and [[comms]]
/// templates.
///
/// Delegates to `config_cache::wasm_load_world`, which performs the unified
/// `parse_world` pass into the `WORLD_CONFIG` thread-local. After PRD #341
/// this is the only world loader — the legacy two-loader split is gone.
///
/// `curated_ships` (issue #917) is the locked scenario's playable-hull
/// allowlist — the same `template_path` values as the catalog entry's
/// `ships` (`wasm_get_scenario_catalog`) — restricting which
/// `[[available_ships]]` hulls get preloaded. `server.html` passes `[]` when
/// no scenario was resolved through the catalog (e.g. the `?scenario=<path>`
/// dev bypass), which preloads every hull the world offers, unchanged from
/// pre-#917 behaviour.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_load_world(
    path: String,
    toml_str: String,
    curated_ships: Vec<String>,
) -> Result<JsValue, JsValue> {
    // The snapshot boundary's two version inputs, taken on the way past: the
    // path names the scenario a save is *of*, and the text is folded into the
    // content ledger (issue #935) so a designer editing this file invalidates
    // saves recorded against it without anyone remembering to bump a number.
    //
    // `reset` here, not at `wasm_init`: this is the one call JS makes exactly
    // once per world selection, so it is the natural "a new load is starting"
    // boundary — see `content_ledger`'s reset-semantics docs.
    crate::content_ledger::reset();
    crate::content_ledger::record(&path, &toml_str);
    SNAPSHOT_WORLD.with(|slot| {
        *slot.borrow_mut() = Some((path.clone(), toml_str.clone()));
    });
    crate::config_cache::wasm_load_world(path, toml_str, curated_ships)
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
/// Returns a JS array of `{ template_path, label, class, hull_id, power_rating,
/// name }` objects. The label comes from the world's `[available_ships]` entry;
/// the remaining metadata is read from the cached entity config for each ship.
/// When the world has no `available_ships` list, returns an empty array — the
/// host should fall back to the hardcoded `assets/entities/alliance_cruiser.toml`.
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
        arr.push(&ship_entry_to_js(ship));
    }
    arr
}

/// Enrich one `AvailableShipEntry` into a JS `{ template_path, label, class,
/// hull_id, power_rating, name }` object, reading the extra metadata from the
/// cached entity config when it is available.
///
/// Shared by `wasm_get_available_ships` (post-load, reads the loaded
/// `WorldConfig`) and `wasm_get_scenario_catalog` (pre-load, reads the base
/// scenario manifest) so both surfaces present ships identically.
#[cfg(target_arch = "wasm32")]
fn ship_entry_to_js(ship: &crate::world::config::AvailableShipEntry) -> Object {
    let obj = Object::new();
    let label = ship.label.as_deref().unwrap_or(&ship.template_path);
    Reflect::set(
        &obj,
        &JsValue::from_str("template_path"),
        &JsValue::from_str(&ship.template_path),
    )
    .ok();
    Reflect::set(&obj, &JsValue::from_str("label"), &JsValue::from_str(label)).ok();
    if let Some(cfg) = crate::config_cache::get_cached_entity_config(&ship.template_path) {
        if let Some(ref class) = cfg.class {
            Reflect::set(&obj, &JsValue::from_str("class"), &JsValue::from_str(class)).ok();
        }
        if let Some(ref hull_id) = cfg.hull_id {
            Reflect::set(
                &obj,
                &JsValue::from_str("hull_id"),
                &JsValue::from_str(hull_id),
            )
            .ok();
        }
        if let Some(rating) = cfg.power_rating {
            Reflect::set(
                &obj,
                &JsValue::from_str("power_rating"),
                &JsValue::from_f64(rating as f64),
            )
            .ok();
        }
        if let Some(ref name) = cfg.name {
            Reflect::set(&obj, &JsValue::from_str("name"), &JsValue::from_str(name)).ok();
        }
    }
    obj
}

/// Deliver the base scenario manifest (`assets/scenarios.toml`) to Rust.
///
/// Called by JS during preload, before any world is loaded. Stored so
/// `wasm_get_scenario_catalog` can build the pre-load catalog (issue #754).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_push_scenario_manifest(toml_str: String) {
    crate::config_cache::set_scenario_manifest_toml(toml_str);
}

/// Validate an uploaded host mod-pack ZIP and, when accepted, PUSH it onto the
/// session-scoped overlay STACK (issues #760, #987).
///
/// Called by the pre-scenario upload control on the host page with the raw
/// archive bytes. Validation is atomic (`world::mod_pack::validate_mod_pack`):
/// on ANY failure nothing is applied and the returned array carries error
/// findings; on success the pack is appended to the overlay stack (installing
/// pack B after pack A does NOT evict A) and an empty (or warning-only) array is
/// returned, after which JS re-reads `wasm_get_scenario_catalog` and
/// `wasm_active_pack_manifest`.
///
/// The pack is validated against the ALREADY-ACTIVE stack (issue #987): a
/// duplicate pack id is rejected (`duplicate-pack-id`), an authored path shared
/// with an active pack warns (`overlapping-pack-path`), and the candidate's
/// composition may resolve a fragment supplied by an earlier active pack.
///
/// Each finding is a JS object `{ severity, category, message, file, line }`.
/// Manifest root worlds — and the include fragments the pack's entity templates
/// pull in — resolve against the pack first, then the active stack, then base
/// content the host has already fetched (`peek_pending_world_toml` for worlds,
/// `raw_template_text` for entity/fragment TOML).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_add_mod_pack(bytes: &[u8]) -> Array {
    // The host side of the mod-pack compatibility contract (issue #986): read
    // the base manifest's `[content]` identity and INJECT it, rather than let
    // the pure validator reach for a host default — the same seam discipline as
    // `resolve_base` below. A host whose manifest declares no `[content]` block
    // yields an identity no real pack can match (empty id, epoch 0), so an
    // upload is rejected rather than silently accepted against unknown content.
    let base_content = crate::config_cache::get_scenario_manifest_toml()
        .and_then(|toml| crate::world::manifest::parse_content_identity(&toml))
        .unwrap_or_default();
    // The already-active overlay stack the candidate is judged against (#987).
    let active = crate::config_cache::active_packs();
    let result = crate::world::mod_pack::validate_mod_pack(
        bytes,
        &base_content,
        |path| {
            // Base content the host has already fetched, by authored path:
            // world TOML for the manifest, and raw entity/fragment TOML so a
            // pack hull may include a SHIPPED fragment. The active overlay stack
            // is consulted by `validate_mod_pack` itself (via the `active` slice
            // below), BENEATH the candidate and ABOVE this base resolver.
            crate::config_cache::peek_pending_world_toml(path)
                .or_else(|| crate::config_cache::raw_template_text(path))
        },
        &crate::entity_loader::WasmTemplateLoader,
        &active,
    );

    let arr = Array::new();
    for finding in &result.findings {
        let obj = Object::new();
        let severity = match finding.severity {
            crate::world::validate::Severity::Error => "error",
            crate::world::validate::Severity::Warning => "warning",
        };
        Reflect::set(
            &obj,
            &JsValue::from_str("severity"),
            &JsValue::from_str(severity),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("category"),
            &JsValue::from_str(finding.category),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("message"),
            &JsValue::from_str(&finding.message),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("file"),
            &JsValue::from_str(&finding.source.file),
        )
        .ok();
        if let Some(line) = finding.source.line {
            Reflect::set(
                &obj,
                &JsValue::from_str("line"),
                &JsValue::from_f64(line as f64),
            )
            .ok();
        }
        arr.push(&obj);
    }

    // Atomic: PUSH the pack onto the overlay stack only when no finding is an
    // error (AC1). The stack is NOT cleared first — installing B after A keeps A
    // (issue #987); the candidate simply shadows earlier packs for shared paths.
    if result.is_accepted() {
        let (id, name, version) =
            crate::world::manifest::parse_pack_manifest(&result.manifest_toml)
                .ok()
                .and_then(|pm| pm.pack)
                .map(|p| (p.id, p.name, p.version))
                .unwrap_or_default();
        crate::config_cache::push_mod_pack(crate::config_cache::ActivePack {
            id,
            name,
            version,
            files: result.files.into_iter().collect(),
            manifest_toml: result.manifest_toml,
        });
    }
    arr
}

/// Discard the WHOLE host mod-pack overlay stack (issues #760 AC4, #987).
///
/// Called on return-to-lobby (before the next scenario stage), so uploaded state
/// never leaks into a fresh selection or a same-page next round. A page reload
/// clears the thread-local anyway; this covers the same-page seams.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_clear_mod_pack() {
    crate::config_cache::clear_mod_pack_overlay();
}

/// Remove the pack with `id` from the overlay stack (issue #987). Precedence for
/// every path it owned re-resolves automatically — the next pack down that
/// carries the path becomes the winner. Returns whether a pack was removed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_remove_mod_pack(id: String) -> bool {
    crate::config_cache::remove_mod_pack(&id)
}

/// Reorder the overlay stack to match `ids` (oldest → newest / lowest → highest
/// precedence), from the host reorder controls (issue #987). Ids not named keep
/// their relative order after the named ones; unknown ids are ignored.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_reorder_mod_packs(ids: Vec<String>) {
    crate::config_cache::reorder_mod_packs(&ids);
}

/// The active overlay stack + its path conflicts, for the host UI (issue #987).
///
/// Returns `{ packs: [{ id, name, version, file_count, scenarios }], conflicts:
/// [{ path, winner, losers }] }`. `packs` is in load order (oldest → newest);
/// `scenarios` is the pack manifest's `[[scenario]]` id list. `conflicts` names,
/// for each authored path carried by two or more packs, the winning pack id and
/// the shadowed loser ids (load order). `server.html` renders the applied-pack
/// list with remove/reorder controls and the conflict summary from this.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_active_pack_manifest() -> JsValue {
    let packs = crate::config_cache::active_packs();
    let out = Object::new();

    let packs_arr = Array::new();
    for pack in &packs {
        let obj = Object::new();
        Reflect::set(&obj, &JsValue::from_str("id"), &JsValue::from_str(&pack.id)).ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("name"),
            &JsValue::from_str(&pack.name),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("version"),
            &JsValue::from_str(&pack.version),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("file_count"),
            &JsValue::from_f64(pack.files.len() as f64),
        )
        .ok();
        let scenarios = Array::new();
        if let Ok(manifest) = crate::world::manifest::parse_manifest(&pack.manifest_toml) {
            for s in &manifest.scenarios {
                scenarios.push(&JsValue::from_str(&s.id));
            }
        }
        Reflect::set(&obj, &JsValue::from_str("scenarios"), &scenarios).ok();
        packs_arr.push(&obj);
    }
    Reflect::set(&out, &JsValue::from_str("packs"), &packs_arr).ok();

    let conflicts_arr = Array::new();
    for conflict in crate::config_cache::overlay_conflicts(&packs) {
        let obj = Object::new();
        Reflect::set(
            &obj,
            &JsValue::from_str("path"),
            &JsValue::from_str(&conflict.path),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("winner"),
            &JsValue::from_str(&conflict.winner),
        )
        .ok();
        let losers = Array::new();
        for loser in &conflict.losers {
            losers.push(&JsValue::from_str(loser));
        }
        Reflect::set(&obj, &JsValue::from_str("losers"), &losers).ok();
        conflicts_arr.push(&obj);
    }
    Reflect::set(&out, &JsValue::from_str("conflicts"), &conflicts_arr).ok();

    out.into()
}

/// Return the authoritative pre-load scenario/ship catalog.
///
/// Unlike `wasm_get_available_ships` (which needs a loaded `WorldConfig`), this
/// reads the base scenario manifest pushed via `wasm_push_scenario_manifest`
/// and each referenced world TOML delivered via `wasm_push_world_toml`, so the
/// catalog is available *before* a root world is activated (issue #754).
///
/// Returns a JS array of `{ id, world, label, description, source, ships: [...] }`
/// objects where each `ships` entry matches `wasm_get_available_ships`'s shape.
/// `source` (issue #990) is the pack id the scenario came from, or `"base"` for
/// a base-manifest scenario, so the phone picker can badge mod-supplied worlds.
/// Only scenarios whose world TOML has been delivered are catalogued; a
/// scenario whose world is still in flight is omitted until its TOML arrives.
/// Returns an empty array when no manifest has been pushed.
///
/// Also runs `validate_manifest` over the base/demo manifest (issue #917) and
/// logs any findings as browser-console warnings under `LogCat::Config` — a
/// typo'd `ships` curation entry or similar is otherwise silently invisible,
/// since (unlike the mod-pack upload flow, which validates atomically at
/// `wasm_add_mod_pack`) nothing else ever calls `validate_manifest` on
/// this manifest. Findings are never fatal here, matching the
/// `missing-scenario-world` precedent below, where `build_merged_catalog`
/// simply skips an unresolvable entry rather than failing the whole catalog.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_scenario_catalog() -> Array {
    use crate::world::manifest::{build_merged_catalog, parse_manifest, validate_manifest};
    let arr = Array::new();
    let Some(manifest_toml) = crate::config_cache::get_scenario_manifest_toml() else {
        return arr;
    };
    let Ok(manifest) = parse_manifest(&manifest_toml) else {
        return arr;
    };
    // Merge the base manifest with EVERY active mod-pack manifest, in load order
    // (issue #760 AC3, #987), resolving every root world through the overlay-aware
    // resolver (the winning pack's content first, then base). Only
    // manifest-listed scenarios appear.
    let active = crate::config_cache::active_packs();
    let parsed_mods: Vec<(String, crate::world::manifest::Manifest)> = active
        .iter()
        .filter_map(|p| {
            parse_manifest(&p.manifest_toml)
                .ok()
                .map(|m| (p.id.clone(), m))
        })
        .collect();
    let mods: Vec<(&str, &crate::world::manifest::Manifest)> =
        parsed_mods.iter().map(|(id, m)| (id.as_str(), m)).collect();
    let resolve_world = |path: &str| {
        crate::config_cache::mod_pack_overlay_get(path)
            .or_else(|| crate::config_cache::peek_pending_world_toml(path))
    };
    for f in validate_manifest(&manifest, &manifest_toml, &resolve_world) {
        bevy::log::warn!(
            target: crate::logging::LogCat::Config.target(),
            "scenario manifest [{}] {}: {}",
            f.category,
            f.source.reference,
            f.message
        );
    }
    let merged = build_merged_catalog(&manifest, &mods, &resolve_world);
    // Cross-pack duplicate-scenario-id collisions are non-blocking warnings
    // resolved by load order (issue #987) — surface them the same way.
    for f in &merged.findings {
        bevy::log::warn!(
            target: crate::logging::LogCat::Config.target(),
            "scenario catalog [{}] {}: {}",
            f.category,
            f.source.reference,
            f.message
        );
    }
    let catalog = merged.catalog;
    for scenario in &catalog.scenarios {
        let obj = Object::new();
        Reflect::set(
            &obj,
            &JsValue::from_str("id"),
            &JsValue::from_str(&scenario.id),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("world"),
            &JsValue::from_str(&scenario.world),
        )
        .ok();
        if let Some(ref label) = scenario.label {
            Reflect::set(&obj, &JsValue::from_str("label"), &JsValue::from_str(label)).ok();
        }
        if let Some(ref desc) = scenario.description {
            Reflect::set(
                &obj,
                &JsValue::from_str("description"),
                &JsValue::from_str(desc),
            )
            .ok();
        }
        // Provenance for the client (issue #990): the pack id this scenario came
        // from, or the literal `"base"` for a base-manifest scenario. Always
        // present so the phone can badge a mod-supplied scenario and leave a base
        // one unmarked without a second lookup — `ScenarioCatalogEntry::origin`
        // (issue #987) is `None` for base, which this flattens to `"base"`.
        Reflect::set(
            &obj,
            &JsValue::from_str("source"),
            &JsValue::from_str(scenario.origin.as_deref().unwrap_or("base")),
        )
        .ok();
        let ships = Array::new();
        for ship in &scenario.ships {
            ships.push(&ship_entry_to_js(ship));
        }
        Reflect::set(&obj, &JsValue::from_str("ships"), &ships).ok();
        arr.push(&obj);
    }
    arr
}

/// Return the Rhai host-fn signature registry for the scenario script editor
/// (issue #983, Rhai M5).
///
/// The vocabulary a scenario author can call — the trigger builders and `on(..)`
/// the loading engine registers, plus the `ctx.effects` / `ctx.flags` /
/// `ctx.schedule` methods (and the delay-builder verbs) the runtime engine
/// registers — enumerated once in `world::script::authoring` so the editor's
/// autocomplete stays in step with what actually resolves at load and runtime.
///
/// Returns a JS array of `{ name, receiver, category, summary, signature,
/// params: [...] }`. `receiver` is the `ctx` sub-object a method hangs off
/// (`"effects"` / `"flags"` / `"schedule"`), `"delay"` for the
/// `in_seconds(n).<verb>` builder verbs, or `""` for a top-level call.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_script_host_fns() -> Array {
    use crate::world::script::authoring::host_fns;
    let arr = Array::new();
    for hf in host_fns() {
        let obj = Object::new();
        Reflect::set(
            &obj,
            &JsValue::from_str("name"),
            &JsValue::from_str(hf.name),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("receiver"),
            &JsValue::from_str(hf.receiver),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("category"),
            &JsValue::from_str(hf.category),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("summary"),
            &JsValue::from_str(hf.summary),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("signature"),
            &JsValue::from_str(&hf.signature()),
        )
        .ok();
        let params = Array::new();
        for p in hf.params {
            params.push(&JsValue::from_str(p));
        }
        Reflect::set(&obj, &JsValue::from_str("params"), &params).ok();
        arr.push(&obj);
    }
    arr
}

/// Compile a `.rhai` source (a sibling file's whole text, or a lifted inline
/// `[script.*]` block) under the sandbox and return editor diagnostics (issue
/// #983, Rhai M5).
///
/// `line_offset` is added to every reported line so an inline block edited
/// inside its host TOML lands on the correct *document* line — the editor passes
/// the block's start line; a standalone `.rhai` file passes `0`. Returns a JS
/// array of `{ message, line, column, severity }` (empty when the source loads
/// clean). Uses the same loading-engine compile + top-level run as the
/// activation gate, so a source that is clean here is clean there.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_script_diagnostics(source: String, line_offset: u32) -> Array {
    use crate::world::script::authoring::script_diagnostics;
    let arr = Array::new();
    for d in script_diagnostics(&source, line_offset as usize) {
        let obj = Object::new();
        Reflect::set(
            &obj,
            &JsValue::from_str("message"),
            &JsValue::from_str(&d.message),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("line"),
            &JsValue::from_f64(d.line as f64),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("column"),
            &JsValue::from_f64(d.column as f64),
        )
        .ok();
        Reflect::set(
            &obj,
            &JsValue::from_str("severity"),
            &JsValue::from_str(d.severity),
        )
        .ok();
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
/// corresponding Bevy resources: `DebugRegionsEnabled`, `DebugOverlayEnabled`,
/// `SimulationPaused`, `DebugDamageEnabled`, `DebugEntitiesEnabled`, and
/// `DebugEntityInspectorEnabled` — all driven from the settings cog added in
/// #939.
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
    mut paused: ResMut<crate::debug_overlay::SimulationPaused>,
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
    // Same for the pause mirror `wasm_is_paused()` reads (issue #939).
    SIM_PAUSED.with(|v| *v.borrow_mut() = paused.0);

    if pause_changed {
        if paused.0 {
            // Pausing `Time<Virtual>` starves the fixed accumulator, so
            // `FixedUpdate` (and with it `SimTick`) stops advancing entirely —
            // deliberately, this is what `sim-tick.spec.js` DECOUPLING asserts
            // on. Since issue #895 this freezes more than the `SimSet` chain:
            // lobby (countdown, ready-check, `drain_lobby_outbox`) and command
            // admission both moved into `FixedUpdate` too, so pausing now also
            // freezes the lobby and stops admitting commands, which it did not
            // pre-#895 when those ran frame-driven in `Update`. See
            // `wiki/concepts/game-loop.md` for the fuller writeup.
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

/// Bevy-side latch for a pending `wasm_force_start()` request, bridging
/// `drain_force_start_input` (the `PreUpdate` JS-input drain) to
/// `apply_force_start` (the `FixedUpdate` state writer) — see the #907 review
/// note on the latter for why the one function that used to do both is now
/// two, in two different schedules.
#[cfg(target_arch = "wasm32")]
#[derive(Resource, Default)]
struct PendingForceStart(bool);

/// Drains the force-start thread-local each frame into [`PendingForceStart`].
/// The actual phase transition is [`apply_force_start`]'s job — this system
/// only moves the JS-set flag into a Bevy resource so `apply_force_start` can
/// run in `FixedUpdate` without touching a thread-local from inside the fixed
/// schedule.
#[cfg(target_arch = "wasm32")]
fn drain_force_start_input(mut pending: ResMut<PendingForceStart>) {
    let was = PENDING_FORCE_START.with(|v| {
        let was = *v.borrow();
        *v.borrow_mut() = false;
        was
    });
    if was {
        pending.0 = true;
    }
}

/// Applies a pending force-start request. When set, transitions the game
/// directly to `InProgress` (or `Loading` if the asset preload isn't done)
/// without requiring any connected players — used for fully AI-crewed runs.
///
/// **`FixedUpdate`, not `PreUpdate` (issue #907 review).** This used to drain
/// the JS thread-local and write `NextState` in one `PreUpdate` system, same
/// as `headless_auto_start`'s pre-fix shape. A `NextState<GamePhase>` write
/// from `PreUpdate` applies at the FRAME-level `StateTransition` — before
/// that frame's fixed steps run — so `OnEnter(GamePhase::InProgress)` (and
/// the player-ship mint inside it, `spawn_game_start_entities`) landed at a
/// point in the schedule whose relationship to `SimTick` was a function of
/// frame pacing, not of a tick. Moving the write here puts it on the same
/// tick-scoped `StateTransition` site every other phase writer already uses
/// (`register_fixed_state_transition` in `sim_tick.rs`, `tick_countdown` in
/// `lobby/server.rs`), so the mint now stamps a deterministic tick regardless
/// of frame rate. The JS-facing drain stays in `PreUpdate` —
/// [`drain_force_start_input`] above — because reading a thread-local from
/// inside the fixed schedule would run it zero or several times per frame
/// instead of once.
#[cfg(target_arch = "wasm32")]
fn apply_force_start(
    state: Res<State<messages::GamePhase>>,
    mut next_state: ResMut<NextState<messages::GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    preload: Option<Res<crate::server::asset_preload::AssetPreloadResource>>,
    mut pending: ResMut<PendingForceStart>,
) {
    let pending_flag = std::mem::take(&mut pending.0);
    if !pending_flag || state.get() != &messages::GamePhase::Lobby {
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

/// Drains the pending host teleport-to-waypoint flag each frame (issue #770).
/// When set, snaps the LocalShip's authoritative `ShipPhysics.{x,z}` onto the
/// shared Navigation waypoint via [`apply_teleport_to_waypoint`]. A no-op when
/// no waypoint is set. Writing `ShipPhysics` is sufficient for propagation:
/// `sync_ship_position` copies it into `Transform` and the sim-state broadcaster
/// sends the new position next tick — no bespoke broadcast path.
#[cfg(target_arch = "wasm32")]
fn drain_teleport_to_waypoint(
    mut ship_q: Query<
        (
            &mut crate::ship_state::ShipPhysics,
            &crate::console::navigation::NavigationWaypoint,
        ),
        With<crate::server_app::LocalShip>,
    >,
) {
    let requested = PENDING_TELEPORT_TO_WAYPOINT.with(|v| {
        let was = *v.borrow();
        *v.borrow_mut() = false;
        was
    });
    if !requested {
        return;
    }
    for (mut physics, waypoint) in ship_q.iter_mut() {
        apply_teleport_to_waypoint(&mut physics, waypoint);
    }
}

/// Mirrors [`crate::sim_tick::SimTick`] into a thread-local each frame so
/// `wasm_sim_tick()` can read it back (issue #895). Pure read; no gameplay
/// effect.
#[cfg(target_arch = "wasm32")]
fn publish_sim_tick(tick: Res<crate::sim_tick::SimTick>) {
    SIM_TICK_COUNT.with(|v| *v.borrow_mut() = tick.0);
}

/// Drains pending God Mode toggle requests each frame (issue #900), turning
/// each into a `ToggleGodMode` `InboundMessage` under `LOCAL_CONSOLE_TOKEN` —
/// the same host-console authority every other host-only command uses (see
/// [`crate::console_bridge::LOCAL_CONSOLE_TOKEN`]).
///
/// Unlike [`drain_teleport_to_waypoint`] this does NOT mutate simulation
/// state directly: it crosses the normal `InboundMessage` boundary so
/// `command_admission::admit_system_commands` validates, stamps, and logs it
/// exactly like a networked command, and its applier
/// (`server_app::apply_god_mode_toggle`) flips the `GodMode` resource on the
/// tick it was admitted for. That is the whole point of #900: God Mode used
/// to be a thread-local this function would have flipped directly.
#[cfg(target_arch = "wasm32")]
fn drain_god_mode_toggle(mut writer: MessageWriter<InboundMessage>) {
    let pending = PENDING_GOD_MODE_TOGGLES.with(|v| {
        let n = *v.borrow();
        *v.borrow_mut() = 0;
        n
    });
    for _ in 0..pending {
        writer.write(InboundMessage {
            token: crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
            msg: messages::ClientMessage::ControlSystem {
                target: messages::SystemId(crate::system_registry::GOD_MODE_SYSTEM_ID.to_string()),
                payload: messages::SystemControlPayload::ToggleGodMode,
            },
        });
    }
}

/// Mirrors the authoritative `GodMode` resource into a thread-local each frame
/// so `wasm_get_god_mode()` can read it back (issue #900). Pure read; no
/// gameplay effect. `Option<Res<_>>` because the resource is inserted by
/// `add_simulation_plugins_with` and this system is registered unconditionally
/// in `wasm_init` — same defensive shape as `publish_waypoint_existence`.
#[cfg(target_arch = "wasm32")]
fn publish_god_mode(god_mode: Option<Res<crate::server_app::GodMode>>) {
    let active = god_mode.map(|g| g.0).unwrap_or(false);
    GOD_MODE_MIRROR.with(|v| *v.borrow_mut() = active);
}

/// Mirrors the two debug resources that have `wasm_*` read-backs into their
/// thread-locals each frame (issue #940).
///
/// `drain_debug_toggles` already writes both, but only for flips *it* applied.
/// Since a phone can now flip the same resources
/// (`debug_overlay::drain_client_debug_flags`), the host page's own settings cog
/// would otherwise paint a stale Region Wireframes or Pause button whenever a
/// client moved one. Mirroring unconditionally each frame removes the whole
/// staleness class rather than adding a second writer at the new flip site.
#[cfg(target_arch = "wasm32")]
fn publish_debug_mirrors(
    regions: Res<crate::debug_overlay::DebugRegionsEnabled>,
    paused: Res<crate::debug_overlay::SimulationPaused>,
) {
    DEBUG_REGIONS_ENABLED.with(|v| *v.borrow_mut() = regions.0);
    SIM_PAUSED.with(|v| *v.borrow_mut() = paused.0);
}

/// Mirrors the LocalShip's Navigation-waypoint existence into a thread-local
/// each frame so `wasm_has_navigation_waypoint()` can read it back (issue #770,
/// AC2). Pure read; no gameplay effect.
#[cfg(target_arch = "wasm32")]
fn publish_waypoint_existence(
    ship_q: Query<
        &crate::console::navigation::NavigationWaypoint,
        With<crate::server_app::LocalShip>,
    >,
) {
    let has = ship_q.iter().next().is_some_and(|w| w.mode().is_some());
    HAS_NAVIGATION_WAYPOINT.with(|v| *v.borrow_mut() = has);
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

/// The Host Channel flush (issue #818): drains every message-drained host
/// channel and samples the two per-frame value taps, forwarding each as
/// `cb(name, payload)` to the single callback registered via
/// [`set_host_channel_callback`].
///
/// Per-channel behaviour (unchanged from the pre-#818 per-channel flushes):
/// - message channels forward every drained event's JSON, in event order;
/// - `shake` fires every frame (even `[0, 0]`) so the JS handler resets the
///   CSS transform when shake ends;
/// - `audio_level` fires only when the level moved by at least 0.001 — an
///   unchanged `.volume` write 60 times a second buys nothing, and the
///   epsilon is well below audible resolution.
///
/// The message channels are drained even when no callback is registered, so
/// registering late never replays a backlog.
#[cfg(target_arch = "wasm32")]
fn flush_host_channels(
    mut hud: MessageReader<HudStateChanged>,
    mut lobby: MessageReader<LobbyStateChanged>,
    mut chatter: MessageReader<AiChatterEvent>,
    mut audio_config: MessageReader<AudioConfigChanged>,
    mut audio_cue: MessageReader<AudioCueEvent>,
) {
    // Declarative channel table: name → drained JSON payloads. Adding a
    // message channel = one row here (see `host_channels`).
    let message_batches: [(&str, Vec<String>); 5] = [
        (
            host_channels::HUD,
            hud.read().map(|m| m.json.clone()).collect(),
        ),
        (
            host_channels::LOBBY,
            lobby.read().map(|m| m.json.clone()).collect(),
        ),
        (
            host_channels::CHATTER,
            chatter
                .read()
                .filter_map(|ev| codec::encode_chatter(ev).ok())
                .collect(),
        ),
        (
            host_channels::AUDIO_CONFIG,
            audio_config.read().map(|m| m.json.clone()).collect(),
        ),
        (
            host_channels::AUDIO_CUE,
            audio_cue.read().map(|m| m.json.clone()).collect(),
        ),
    ];

    HOST_CHANNEL_CB.with(|slot| {
        let borrowed = slot.borrow();
        let Some(cb) = borrowed.as_ref() else { return };

        for (name, payloads) in &message_batches {
            for json in payloads {
                let _ = cb.call2(
                    &JsValue::NULL,
                    &JsValue::from_str(name),
                    &JsValue::from_str(json),
                );
            }
        }

        // Per-frame tap: shake, unconditional.
        let (x, y) = SHAKE_OFFSET.with(|slot| *slot.borrow());
        let offset = Array::of2(&JsValue::from_f64(x as f64), &JsValue::from_f64(y as f64));
        let _ = cb.call2(
            &JsValue::NULL,
            &JsValue::from_str(host_channels::SHAKE),
            &offset,
        );

        // Per-frame tap: forcefield level, epsilon-deduped.
        let current = FORCEFIELD_LEVEL.with(|slot| *slot.borrow());
        let last = LAST_SENT_FORCEFIELD.with(|slot| *slot.borrow());
        if (current - last).abs() >= 0.001 {
            LAST_SENT_FORCEFIELD.with(|slot| {
                *slot.borrow_mut() = current;
            });
            let _ = cb.call2(
                &JsValue::NULL,
                &JsValue::from_str(host_channels::AUDIO_LEVEL),
                &JsValue::from_f64(current as f64),
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
    use super::{
        apply_pending_toggles, apply_teleport_to_waypoint, host_channels, DebugToggleKind,
    };
    use crate::console::navigation::{NavigationWaypoint, WaypointMode};
    use crate::ship_state::ShipPhysics;

    /// Teleport onto a Free waypoint sets `x`/`z` and leaves `y` unchanged.
    #[test]
    fn teleport_sets_xz_and_preserves_y() {
        let mut physics = ShipPhysics {
            x: 1.0,
            y: 42.0,
            z: 2.0,
            ..Default::default()
        };
        let waypoint = NavigationWaypoint::new(WaypointMode::Free { x: 120.0, z: -45.0 });

        let teleported = apply_teleport_to_waypoint(&mut physics, &waypoint);

        assert!(teleported, "a waypoint exists, so a teleport should happen");
        assert_eq!(physics.x, 120.0);
        assert_eq!(physics.z, -45.0);
        assert_eq!(physics.y, 42.0, "altitude must be left unchanged");
    }

    /// An Anchored waypoint teleports to its live-cached x/z.
    #[test]
    fn teleport_uses_anchored_snapshot_position() {
        let mut physics = ShipPhysics::default();
        let waypoint = NavigationWaypoint::new(WaypointMode::Anchored {
            source_uuid: "target-1".into(),
            last_x: 75.0,
            last_z: -150.0,
        });

        let teleported = apply_teleport_to_waypoint(&mut physics, &waypoint);

        assert!(teleported);
        assert_eq!(physics.x, 75.0);
        assert_eq!(physics.z, -150.0);
    }

    /// With no waypoint set the teleport is a no-op and reports `false`.
    #[test]
    fn teleport_without_waypoint_is_a_noop() {
        let mut physics = ShipPhysics {
            x: 7.0,
            y: 3.0,
            z: 9.0,
            ..Default::default()
        };
        let waypoint = NavigationWaypoint::default();

        let teleported = apply_teleport_to_waypoint(&mut physics, &waypoint);

        assert!(!teleported, "no waypoint means nothing to teleport to");
        assert_eq!(physics.x, 7.0);
        assert_eq!(physics.y, 3.0);
        assert_eq!(physics.z, 9.0);
    }

    /// The Host Channel name table (issue #818) must have no duplicates —
    /// the JS dispatcher in `server.html` keys its handlers by these names.
    #[test]
    fn host_channel_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for name in host_channels::ALL {
            assert!(
                seen.insert(name),
                "duplicate host channel name: {name:?} — each name must map to \
                 exactly one JS handler"
            );
        }
    }

    /// Every named channel const is present in `ALL` (and nothing else is) —
    /// `flush_host_channels` and the JS dispatcher both key off these.
    #[test]
    fn host_channel_all_covers_every_const() {
        assert_eq!(
            host_channels::ALL,
            [
                host_channels::HUD,
                host_channels::LOBBY,
                host_channels::CHATTER,
                host_channels::AUDIO_CONFIG,
                host_channels::AUDIO_CUE,
                host_channels::SHAKE,
                host_channels::AUDIO_LEVEL,
            ]
        );
    }

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
