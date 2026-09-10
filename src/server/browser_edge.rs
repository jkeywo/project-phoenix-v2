//! Browser-only staging and readback. Storage is private; the bridge uses owned
//! drains, typed publication, and synchronous edge operations. No Bevy World is
//! retained here, and callbacks must be cloned before invoking JavaScript.
use super::*;
use std::cell::RefCell;

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// Messages received from JS peers, waiting to be injected into Bevy.
    /// Each entry is (sender_token, json_payload).
    static INBOUND_QUEUE: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };

    /// Disconnect tokens queued by JS, waiting to be injected into Bevy.
    static DISCONNECT_QUEUE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

    /// Host-mesh simulation frames received from another SHIP HOST, waiting to
    /// be handed to `lockstep` (issue #1116). A separate queue from
    /// `INBOUND_QUEUE` because it carries a separate protocol on a separate
    /// socket: a fleet member never identifies, holds no station, and nothing it
    /// says is a `ClientMessage`. Mixing them would make "each crew star belongs
    /// to one host" a filtering rule rather than a fact about the wires.
    /// Each entry is `(authenticated_slot, json_payload)`: the fleet slot the
    /// delivering connection was bound to at join (issue #1120), and the encoded
    /// frame. `0` (never a real fleet slot, which start at `slot-1`) means the page
    /// could not authenticate the connection, so the frame is trusted as before.
    static MESH_INBOUND: RefCell<Vec<(u32, String)>> = const { RefCell::new(Vec::new()) };

    /// Host-mesh frames this host has produced and JS has not sent yet.
    /// Drained by `wasm_take_mesh_frames` rather than pushed through a callback,
    /// because the fleet link is polled by the page's own frame loop and a
    /// callback would deliver a tick frame at whatever moment the simulation
    /// happened to seal it.
    static MESH_OUTBOUND: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

    /// Fleet slots whose ship HOST closed its link, queued by JS for the next
    /// frame (issue #1119). The twin of `DISCONNECT_QUEUE`, but for a peer HOST
    /// rather than a crew member: a crew disconnect flips one station on this
    /// host's own ship, while a host loss flips a whole PEER ship to Backfill at
    /// an agreed tick every survivor derives the same. Kept as bare slot ordinals
    /// — `drain_mesh_inbound` turns each into a `HostLoss` observation whose
    /// agreed tick the simulation derives from the lost host's own watermark, so
    /// the page never has to know a tick.
    static HOST_LOSS_QUEUE: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };

    /// Disconnected fixed slots a replacement machine has validly claimed, queued
    /// by the OWNER page for the next frame (issue #1120). The owner is the only
    /// host that admits claims (the star centre), so it is the only minter of the
    /// monotonic `claim_seq` — `lockstep::SlotClaimSequence` — that makes the race resolution
    /// deterministic. `drain_mesh_inbound` turns each into a granted
    /// `SlotClaimFrame` stamped with the owner's own slot, the next seq and the
    /// current tick, records it in this host's own resolver, and broadcasts it.
    static SLOT_CLAIM_QUEUE: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };


    /// The fleet status mirror `wasm_mesh_status` answers from, written each
    /// frame by `publish_mesh_status`. A mirror rather than a `World` read for
    /// the same reason `SIM_PAUSED` is one: the settings cog asks between
    /// frames, when there is no world handle to ask.
    static MESH_STATUS: RefCell<String> = const { RefCell::new(String::new()) };

    /// A fleet the page has joined but Bevy has not adopted yet: the encoded
    /// roster, applied once on the next frame. Deferred for the same reason
    /// every other JS→Bevy handoff here is — `wasm_join_fleet` is called from a
    /// socket callback, which holds no `World`.
    static PENDING_FLEET_ADOPTIONS: RefCell<VecDeque<PendingFleetAdoption>> =
        const { RefCell::new(VecDeque::new()) };
    static FLEET_JOIN_GENERATION: RefCell<u64> = const { RefCell::new(0) };
    static FLEET_JOIN_STATUS: RefCell<crate::lockstep::FleetJoinStatus> = const {
        RefCell::new(crate::lockstep::FleetJoinStatus {
            generation: 0,
            status: crate::lockstep::FleetJoinStatusKind::Idle,
            reason: None,
        })
    };

    /// A validated crew-public GM roster waiting for its full replacement in
    /// Bevy (issue #1289). Decoded at the WASM boundary so no `serde_json`
    /// escapes `core::codec`; latched here because the JS call has no `World`.
    static PENDING_GM_ROSTER: RefCell<Option<crate::gm_roster::GmRoster>> =
        const { RefCell::new(None) };

    /// Validated privileged GM requests waiting for the next frame-driven
    /// admission pass. This lane remains live while FixedUpdate is paused.
    static PENDING_GM_ACTIONS: RefCell<VecDeque<crate::gm_action::GmActionRequest>> =
        const { RefCell::new(VecDeque::new()) };

    /// Accepted GM join transactions waiting for the deterministic sequencer.
    /// First-time decisions came from a visible peer; reconnects came from the
    /// exact private capability. Kept separate from GM actions in both cases.
    static PENDING_GM_JOINS: RefCell<VecDeque<PendingGmJoin>> =
        const { RefCell::new(VecDeque::new()) };
    /// Read-only progress mirror polled by every server-page surface.
    static GM_JOIN_STATUS: RefCell<crate::gm_join::GmJoinProgress> =
        const { RefCell::new(crate::gm_join::GmJoinProgress::Idle) };

    /// Explicit production GM-page boot request. This is set by the page before
    /// `wasm_init` and takes precedence over the WebDriver probe.
    static GM_HOST_BOOT_REQUESTED: RefCell<bool> = const { RefCell::new(false) };
    /// Read-only browser smoke/diagnostic mirror of the profile actually used.
    static ACTIVE_BOOT_PROFILE: RefCell<&'static str> = const { RefCell::new("not-started") };

    /// Ordered, edge-only coordinated-lobby input waiting for the next
    /// `PreUpdate` drain (issue #1290). One FIFO is essential: a same-frame
    /// leave(false) -> reopen(true) -> start-1 sequence must not collapse its
    /// teardown generation or let a pre-teardown grant cross into the new
    /// fleet.
    static PENDING_FLEET_LOBBY_INPUTS: RefCell<VecDeque<PendingFleetLobbyInput>> =
        const { RefCell::new(VecDeque::new()) };
    /// Latest absolute projections are rebound onto a newly allocated join
    /// generation. The page commonly publishes managed/validation immediately
    /// before calling `wasm_join_fleet`; without these mirrors those samples
    /// would still carry generation zero and be discarded after adoption.
    static LATEST_FLEET_MANAGED: RefCell<Option<bool>> = const { RefCell::new(None) };
    static LATEST_FLEET_VALIDATION: RefCell<Option<bool>> = const { RefCell::new(None) };

    /// Fixed-tick start outcomes mirrored back to the World-less JS poller.
    static START_GRANT_RESULTS: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };

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

    /// Whether the host page's reduced-motion preference
    /// (`prefers-reduced-motion: reduce`) is active, forwarded by
    /// [`wasm_set_reduced_motion`] (issue #1173). Drained each frame into the
    /// `ViewscreenMotion` resource by `viewscreen_border::sync_viewscreen_motion`.
    /// Read continuously, so an OS-level change of the preference takes effect
    /// without a page reload.
    #[cfg(target_arch = "wasm32")]
    static REDUCED_MOTION: RefCell<bool> = const { RefCell::new(false) };

    /// The endpoint's camera/page shake intensity, forwarded by
    /// [`wasm_set_shake_intensity`] (issue #1428). `None` is *nothing has been
    /// published*, which is a different fact from a published `0.0`: the first
    /// takes whatever the reduced-motion preference defaults to, the second is
    /// an operator who asked for no shake and must not be overridden by it.
    #[cfg(target_arch = "wasm32")]
    static SHAKE_INTENSITY: RefCell<Option<f32>> = const { RefCell::new(None) };

    /// The endpoint's shield-flash intensity, forwarded by
    /// [`wasm_set_flash_intensity`] (issue #1428). Same `Option` reasoning as
    /// [`SHAKE_INTENSITY`] above.
    #[cfg(target_arch = "wasm32")]
    static FLASH_INTENSITY: RefCell<Option<f32>> = const { RefCell::new(None) };

    /// Mirror of the `SimulationPaused` resource, written by
    /// `drain_host_controls` so `wasm_is_paused()` can answer without a Bevy
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

    /// Pending absolute states queued by the host's one generic diagnostic
    /// mutation export. The key is the canonical Debug Surface identity; a
    /// second request for the same surface replaces the first before the next
    /// drain. Absent entirely from a public-demo binary.
    #[cfg(not(phoenix_demo_build))]
    static PENDING_DEBUG_SURFACE_STATES: RefCell<HashMap<DebugSurface, bool>> =
        RefCell::new(HashMap::new());

    /// Pending host pause toggle. Separate from diagnostic identity and present
    /// in every build because host pause is a Gameplay control.
    static PENDING_PAUSE: RefCell<bool> = const { RefCell::new(false) };

    /// The world this session loaded: `(path, TOML text)`, recorded by
    /// `wasm_load_world`. The snapshot boundary (issue #862) needs both — the
    /// path is `Run::scenario`, and the text is what `snapshot::content_digest`
    /// hashes to produce the content version. Kept here rather than reached for
    /// through `config_cache` so the save path has one obvious source.
    static SNAPSHOT_WORLD: RefCell<Option<(String, String)>> = const { RefCell::new(None) };

    /// This session's live ship/Station seating (issue #1445), mirrored each
    /// frame from the replicated `FleetRoster` so `wasm_list_save_slots` — a
    /// synchronous JS call with no `World` in scope — can run the shared
    /// candidate preflight against it. `None` before a world has booted, which
    /// is exactly when the landing catalogue has no live session to be a
    /// candidate FOR, and is why a pre-boot row carries no preflight at all.
    static LIVE_SEATING: RefCell<Option<crate::gm_checkpoint::LiveSeating>> =
        const { RefCell::new(None) };

    /// The private LocalStorage namespace chosen for this browser host. The JS
    /// identity installer runs before the first catalogue call, and this cache
    /// then makes every later API/capture use exactly that namespace even if a
    /// script tampers with sessionStorage mid-session.
    static BROWSER_SAVE_NAMESPACE: RefCell<Option<String>> = const { RefCell::new(None) };

    /// Opaque capture tokens plus their peer-local storage intents, queued by
    /// synchronous save exports and drained into
    /// [`crate::save_slots_lifecycle::ManualSaveRequests`] in `PreUpdate`.
    ///
    /// Every token is distinct and FIFO, so two clicks before the next frame
    /// cannot overwrite one another. The destination/name stays in the same
    /// bounded structure until the fixed-tick capture reaches the local outbox.
    /// Nothing here enters the digest or mesh; the fixed schedule sees only the
    /// token.
    static PENDING_BROWSER_SAVES: RefCell<PendingBrowserSaves<BrowserSaveIntent>> =
        const { RefCell::new(PendingBrowserSaves::new()) };

    /// The text of an exported save, waiting for the host page to collect it
    /// (issue #866).
    ///
    /// Parked rather than returned, for [`PENDING_BROWSER_SAVES`]'s reason turned around:
    /// the capture happens on a tick boundary, so the click that
    /// asked for it is long over by the time there is a string to hand back.
    /// Taken exactly once by `wasm_take_exported_snapshot`, which is what turns
    /// it into a download.
    static EXPORTED_ARTIFACT: RefCell<Option<String>> = const { RefCell::new(None) };

    /// FIFO host-visible outcomes of saves and resumes, each
    /// `(succeeded, source, message)` and **drained** by
    /// `wasm_snapshot_status()`.
    ///
    /// Drained rather than latched because the host page polls it: a status
    /// that stayed set would be re-shown every poll, and one that was cleared
    /// on a timer could be missed entirely. Taking it means each outcome is
    /// reported exactly once, whoever asks first. The finite ring drops the
    /// oldest status when an inactive poller has already filled it, keeping the
    /// newest local refusal visible.
    static SNAPSHOT_STATUS: RefCell<BoundedFifo<(bool, String, String), MAX_BROWSER_SAVE_STATUSES>> =
        const { RefCell::new(BoundedFifo::new()) };

    /// PRE-INIT stash for a save that passed the version gate, set by
    /// `wasm_prepare_resume` / `wasm_prepare_import` BEFORE `wasm_init` (a resume
    /// is a page reload, so it runs before there is a `World`). `wasm_init` hands
    /// it off to [`crate::startup_restore`], which owns the complete lifecycle.
    /// Category 4 above.
    static PENDING_RESTORE_STAGED: RefCell<Option<crate::snapshot::StoredRun>> =
        const { RefCell::new(None) };

    /// OUTBOX mirror of whether a restore is still staged, read back by
    /// `wasm_resume_pending()` (issue #1181). Set `true` when a save is staged
    /// pre-init; refreshed each frame by `drain_snapshot_restore` from the
    /// shared driver's pending state. Category 2 above.
    static RESUME_PENDING_MIRROR: RefCell<bool> = const { RefCell::new(false) };

    /// Modifier debug payload as JSON (issue #1150), written by
    /// `debug::modifiers::publish_modifier_debug` each `PostUpdate` frame when
    /// the surface is enabled. Read by `wasm_get_debug_state()` from JS; the dock
    /// parses it and renders the three modifier sections rather than printing it.
    static DEBUG_STATE_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Damage-log debug payload as JSON (issue #1150), written by
    /// `debug::damage::publish_damage_debug` each `PostUpdate` frame when the
    /// surface is enabled. Read by `wasm_get_damage_log()` from JS; the dock
    /// parses and renders it.
    static DAMAGE_LOG_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Entity-behavior debug payload as JSON (issue #1150), written by
    /// `debug::entities::publish_entity_behavior_debug` each `PostUpdate` frame
    /// when the surface is enabled. Read by `wasm_get_entity_debug_state()` from
    /// JS; the dock parses and renders it.
    static ENTITY_DEBUG_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// Entity-inspector debug payload as JSON (issue #1150), written by
    /// `debug::inspector::publish_entity_inspector_debug` each `PostUpdate` frame
    /// when the surface is enabled. Read by `wasm_get_entity_inspector()` from
    /// JS; the dock parses and renders it.
    static ENTITY_INSPECTOR_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// The station-activity debug payload as JSON (issue #1145), written by
    /// `debug::station_activity::publish_station_activity` each tick while the
    /// station-activity flag is on. Read by `wasm_get_station_activity()` from
    /// JS. Unlike its neighbours this is structured JSON, not pre-formatted text
    /// — the dock parses it and draws a chart (`gui/station-activity-chart.js`).
    static STATION_ACTIVITY_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// The AI doctrine-pool debug payload as JSON (issue #1149), written by
    /// `debug::ai_state::publish_ai_doctrine` each tick while the AI-doctrine flag
    /// is on. Read by `wasm_get_ai_doctrine()` from JS. Structured JSON, not
    /// pre-formatted text — the dock parses it and draws a per-ship panel
    /// (`gui/ai-doctrine-panel.js`).
    static AI_DOCTRINE_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// The scenario-state debug payload as JSON (issue #1148), written by
    /// `debug::scenario::publish_scenario_state` each tick while the
    /// scenario-state flag is on. Read by `wasm_get_scenario_state()` from JS.
    /// Like station activity this is structured JSON, not pre-formatted text —
    /// the dock parses it and draws a panel (`gui/scenario-state-panel.js`).
    static SCENARIO_STATE_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// The console input-to-feedback latency payload as JSON (issue #1169),
    /// written by `debug::console_latency::publish_console_latency` each tick
    /// while the console-latency flag is on. Read by `wasm_get_console_latency()`
    /// from JS. Structured JSON like its two neighbours above; the dock parses it
    /// and draws a per-action table (`gui/console-latency-panel.js`).
    static CONSOLE_LATENCY_STRING: RefCell<String> = const { RefCell::new(String::new()) };

    /// The debug-flag read-back as JSON (issue #1169), mirrored by
    /// `debug_overlay::report_debug_state` — the one system that already computes
    /// this set for `ServerMessage::DebugState`. Read by `wasm_get_debug_flags()`
    /// so the host cog paints from the simulation's own answer rather than from
    /// its memory of what it last clicked; a connected phone can flip the same
    /// flags, and for console latency a stale button meant the operator saw a
    /// live surface that was measuring nothing.
    static DEBUG_FLAGS_STRING: RefCell<String> = const { RefCell::new(String::new()) };

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

    /// INBOX: instagib-toggle requests from `wasm_toggle_instagib()`, drained by
    /// `drain_instagib_toggle` each `PreUpdate` into the [`crate::server_app::Instagib`] Resource
    /// (issue #1181). A count (not a bool) so two clicks in one frame flip twice,
    /// matching the God Mode queue; parity is applied by `apply_instagib_toggles`.
    static PENDING_INSTAGIB_TOGGLES: RefCell<u32> = const { RefCell::new(0) };

    /// OUTBOX mirror of the [`crate::server_app::Instagib`] Resource, refreshed each frame by
    /// `publish_instagib` so `wasm_get_instagib()` can read it back without a
    /// `World` handle (issue #1181). Same pattern as `GOD_MODE_MIRROR`.
    static INSTAGIB_MIRROR: RefCell<bool> = const { RefCell::new(false) };

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

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// Candidate-private topology bootstrap. This deliberately does not enter
    /// `PENDING_FLEET_ADOPTIONS`: it may prepare entities for restore, but only
    /// the typed Commit may install the authoritative roster/wait-set.
    static PENDING_GM_JOIN_BOOTSTRAPS: RefCell<VecDeque<PendingGmJoinBootstrap>> =
        const { RefCell::new(VecDeque::new()) };
    /// Owner-sequenced terminal transport losses after visible acceptance.
    static PENDING_GM_JOIN_REFUSALS: RefCell<VecDeque<PendingGmJoinRefusal>> =
        const { RefCell::new(VecDeque::new()) };
}

pub(super) fn publish_last_sent_forcefield(value: f32) {
    LAST_SENT_FORCEFIELD.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_last_sent_forcefield() -> f32 {
    LAST_SENT_FORCEFIELD.with(|value| *value.borrow())
}

pub(super) fn read_forcefield_level() -> f32 {
    FORCEFIELD_LEVEL.with(|value| *value.borrow())
}

pub(super) fn read_shake_offset() -> (f32, f32) {
    SHAKE_OFFSET.with(|value| *value.borrow())
}

pub(super) fn publish_has_navigation_waypoint(value: bool) {
    HAS_NAVIGATION_WAYPOINT.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_sim_paused(value: bool) {
    SIM_PAUSED.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_instagib_mirror(value: bool) {
    INSTAGIB_MIRROR.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn take_pending_instagib_toggles() -> u32 {
    PENDING_INSTAGIB_TOGGLES.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

pub(super) fn publish_god_mode_mirror(value: bool) {
    GOD_MODE_MIRROR.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn take_pending_god_mode_toggles() -> u32 {
    PENDING_GOD_MODE_TOGGLES.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

pub(super) fn publish_sim_tick_count(value: u64) {
    SIM_TICK_COUNT.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn take_pending_teleport_to_waypoint() -> bool {
    PENDING_TELEPORT_TO_WAYPOINT.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

pub(super) fn take_pending_force_start() -> bool {
    PENDING_FORCE_START.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

pub(super) fn drain_disconnect_queue() -> Vec<String> {
    DISCONNECT_QUEUE.with(|value| value.borrow_mut().drain(..).collect())
}

pub(super) fn take_pending_pause() -> bool {
    PENDING_PAUSE.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

pub(super) fn drain_inbound_queue() -> Vec<(String, String)> {
    INBOUND_QUEUE.with(|value| value.borrow_mut().drain(..).collect())
}

pub(super) fn publish_selected_ship_template_path(value: Option<String>) {
    SELECTED_SHIP_TEMPLATE_PATH.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_snapshot_world(value: Option<(String, String)>) {
    SNAPSHOT_WORLD.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_sim_tick_count() -> u64 {
    SIM_TICK_COUNT.with(|value| *value.borrow())
}

pub(super) fn read_has_navigation_waypoint() -> bool {
    HAS_NAVIGATION_WAYPOINT.with(|value| *value.borrow())
}

pub(super) fn publish_pending_teleport_to_waypoint(value: bool) {
    PENDING_TELEPORT_TO_WAYPOINT.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_pending_force_start(value: bool) {
    PENDING_FORCE_START.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_console_latency_string(value: String) {
    CONSOLE_LATENCY_STRING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_console_latency_string() -> String {
    CONSOLE_LATENCY_STRING.with(|value| value.borrow().clone())
}

pub(super) fn publish_debug_flags_string(value: String) {
    DEBUG_FLAGS_STRING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_debug_flags_string() -> String {
    DEBUG_FLAGS_STRING.with(|value| value.borrow().clone())
}

pub(super) fn publish_scenario_state_string(value: String) {
    SCENARIO_STATE_STRING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_scenario_state_string() -> String {
    SCENARIO_STATE_STRING.with(|value| value.borrow().clone())
}

pub(super) fn publish_ai_doctrine_string(value: String) {
    AI_DOCTRINE_STRING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_ai_doctrine_string() -> String {
    AI_DOCTRINE_STRING.with(|value| value.borrow().clone())
}

pub(super) fn publish_station_activity_string(value: String) {
    STATION_ACTIVITY_STRING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_station_activity_string() -> String {
    STATION_ACTIVITY_STRING.with(|value| value.borrow().clone())
}

pub(super) fn publish_entity_inspector_string(value: String) {
    ENTITY_INSPECTOR_STRING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_entity_inspector_string() -> String {
    ENTITY_INSPECTOR_STRING.with(|value| value.borrow().clone())
}

pub(super) fn publish_entity_debug_string(value: String) {
    ENTITY_DEBUG_STRING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_entity_debug_string() -> String {
    ENTITY_DEBUG_STRING.with(|value| value.borrow().clone())
}

pub(super) fn publish_damage_log_string(value: String) {
    DAMAGE_LOG_STRING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_damage_log_string() -> String {
    DAMAGE_LOG_STRING.with(|value| value.borrow().clone())
}

pub(super) fn publish_debug_state_string(value: String) {
    DEBUG_STATE_STRING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_debug_state_string() -> String {
    DEBUG_STATE_STRING.with(|value| value.borrow().clone())
}

pub(super) fn read_sim_paused() -> bool {
    SIM_PAUSED.with(|value| *value.borrow())
}

pub(super) fn publish_pending_pause(value: bool) {
    PENDING_PAUSE.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_resume_pending_mirror(value: bool) {
    RESUME_PENDING_MIRROR.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_live_seating(value: Option<crate::gm_checkpoint::LiveSeating>) {
    LIVE_SEATING.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn live_seating() -> Option<crate::gm_checkpoint::LiveSeating> {
    LIVE_SEATING.with(|slot| slot.borrow().clone())
}

pub(super) fn publish_exported_artifact(value: Option<String>) {
    EXPORTED_ARTIFACT.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn clear_snapshot_status() {
    SNAPSHOT_STATUS.with(|value| value.borrow_mut().clear());
}

pub(super) fn publish_pending_restore_staged(value: Option<crate::snapshot::StoredRun>) {
    PENDING_RESTORE_STAGED.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_selected_ship_template_path() -> Option<String> {
    SELECTED_SHIP_TEMPLATE_PATH.with(|value| value.borrow().clone())
}

pub(super) fn read_snapshot_world() -> Option<(String, String)> {
    SNAPSHOT_WORLD.with(|value| value.borrow().clone())
}

pub(super) fn read_resume_pending_mirror() -> bool {
    RESUME_PENDING_MIRROR.with(|value| *value.borrow())
}

pub(super) fn read_log_entity() -> Option<String> {
    LOG_ENTITY.with(|value| value.borrow().clone())
}

pub(super) fn read_log_spec() -> Option<String> {
    LOG_SPEC.with(|value| value.borrow().clone())
}

pub(super) fn publish_log_entity(value: Option<String>) {
    LOG_ENTITY.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_log_spec(value: Option<String>) {
    LOG_SPEC.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_reduced_motion() -> bool {
    REDUCED_MOTION.with(|value| *value.borrow())
}

pub(super) fn publish_reduced_motion(value: bool) {
    REDUCED_MOTION.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_shake_intensity() -> Option<f32> {
    SHAKE_INTENSITY.with(|value| *value.borrow())
}

pub(super) fn publish_shake_intensity(value: f32) {
    SHAKE_INTENSITY.with(|slot| *slot.borrow_mut() = Some(value));
}

pub(super) fn read_flash_intensity() -> Option<f32> {
    FLASH_INTENSITY.with(|value| *value.borrow())
}

pub(super) fn publish_flash_intensity(value: f32) {
    FLASH_INTENSITY.with(|slot| *slot.borrow_mut() = Some(value));
}

pub(super) fn publish_forcefield_level(value: f32) {
    FORCEFIELD_LEVEL.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_shake_offset(value: (f32, f32)) {
    SHAKE_OFFSET.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_host_channel_cb(value: Option<Function>) {
    HOST_CHANNEL_CB.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_outbound_cb(value: Option<Function>) {
    OUTBOUND_CB.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_mesh_status(value: String) {
    MESH_STATUS.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn clear_start_grant_results() {
    START_GRANT_RESULTS.with(|value| value.borrow_mut().clear());
}

pub(super) fn read_fleet_join_status() -> crate::lockstep::FleetJoinStatus {
    FLEET_JOIN_STATUS.with(|value| value.borrow().clone())
}

pub(super) fn publish_gm_join_status(value: crate::gm_join::GmJoinProgress) {
    GM_JOIN_STATUS.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn drain_pending_gm_joins() -> Vec<PendingGmJoin> {
    PENDING_GM_JOINS.with(|value| value.borrow_mut().drain(..).collect())
}

pub(super) fn drain_pending_gm_join_refusals() -> Vec<PendingGmJoinRefusal> {
    PENDING_GM_JOIN_REFUSALS.with(|value| value.borrow_mut().drain(..).collect())
}

pub(super) fn drain_pending_gm_actions() -> Vec<crate::gm_action::GmActionRequest> {
    PENDING_GM_ACTIONS.with(|value| value.borrow_mut().drain(..).collect())
}

pub(super) fn take_pending_gm_roster() -> Option<crate::gm_roster::GmRoster> {
    PENDING_GM_ROSTER.with(|value| value.borrow_mut().take())
}

pub(super) fn take_host_loss_queue() -> Vec<u32> {
    HOST_LOSS_QUEUE.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

pub(super) fn take_mesh_inbound() -> Vec<(u32, String)> {
    MESH_INBOUND.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

pub(super) fn take_slot_claim_queue() -> Vec<u32> {
    SLOT_CLAIM_QUEUE.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

pub(super) fn take_pending_fleet_adoptions() -> VecDeque<PendingFleetAdoption> {
    PENDING_FLEET_ADOPTIONS.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

pub(super) fn drain_pending_gm_join_bootstraps() -> Vec<PendingGmJoinBootstrap> {
    PENDING_GM_JOIN_BOOTSTRAPS.with(|value| value.borrow_mut().drain(..).collect())
}

pub(super) fn clear_mesh_status() {
    MESH_STATUS.with(|value| value.borrow_mut().clear());
}

pub(super) fn clear_slot_claim_queue() {
    SLOT_CLAIM_QUEUE.with(|value| value.borrow_mut().clear());
}

pub(super) fn clear_host_loss_queue() {
    HOST_LOSS_QUEUE.with(|value| value.borrow_mut().clear());
}

pub(super) fn clear_mesh_outbound() {
    MESH_OUTBOUND.with(|value| value.borrow_mut().clear());
}

pub(super) fn clear_mesh_inbound() {
    MESH_INBOUND.with(|value| value.borrow_mut().clear());
}

pub(super) fn enqueue_host_loss_queue(value: u32) {
    HOST_LOSS_QUEUE.with(|queue| queue.borrow_mut().push(value));
}

pub(super) fn enqueue_disconnect_queue(value: String) {
    DISCONNECT_QUEUE.with(|queue| queue.borrow_mut().push(value));
}

pub(super) fn read_mesh_status() -> String {
    MESH_STATUS.with(|value| value.borrow().clone())
}

pub(super) fn publish_latest_fleet_validation(value: Option<bool>) {
    LATEST_FLEET_VALIDATION.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_latest_fleet_managed(value: Option<bool>) {
    LATEST_FLEET_MANAGED.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_pending_gm_roster(value: Option<crate::gm_roster::GmRoster>) {
    PENDING_GM_ROSTER.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_fleet_join_status(value: crate::lockstep::FleetJoinStatus) {
    FLEET_JOIN_STATUS.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_latest_fleet_validation() -> Option<bool> {
    LATEST_FLEET_VALIDATION.with(|value| *value.borrow())
}

pub(super) fn read_latest_fleet_managed() -> Option<bool> {
    LATEST_FLEET_MANAGED.with(|value| *value.borrow())
}

pub(super) fn enqueue_slot_claim_queue(value: u32) {
    SLOT_CLAIM_QUEUE.with(|queue| queue.borrow_mut().push(value));
}

pub(super) fn enqueue_mesh_inbound(value: (u32, String)) {
    MESH_INBOUND.with(|queue| queue.borrow_mut().push(value));
}

pub(super) fn enqueue_inbound_queue(value: (String, String)) {
    INBOUND_QUEUE.with(|queue| queue.borrow_mut().push(value));
}

pub(super) fn take_pending_restore_staged() -> Option<crate::snapshot::StoredRun> {
    PENDING_RESTORE_STAGED.with(|value| value.borrow_mut().take())
}

pub(super) fn publish_active_boot_profile(value: &'static str) {
    ACTIVE_BOOT_PROFILE.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_gm_host_boot_requested() -> bool {
    GM_HOST_BOOT_REQUESTED.with(|value| *value.borrow())
}

pub(super) fn publish_ship_config(value: Option<ShipConfig>) {
    SHIP_CONFIG.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn publish_ship_stations(value: Option<ShipStations>) {
    SHIP_STATIONS.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_instagib_mirror() -> bool {
    INSTAGIB_MIRROR.with(|value| *value.borrow())
}

pub(super) fn increment_pending_instagib_toggles() {
    PENDING_INSTAGIB_TOGGLES.with(|value| *value.borrow_mut() += 1);
}

pub(super) fn read_god_mode_mirror() -> bool {
    GOD_MODE_MIRROR.with(|value| *value.borrow())
}

pub(super) fn increment_pending_god_mode_toggles() {
    PENDING_GOD_MODE_TOGGLES.with(|value| *value.borrow_mut() += 1);
}

pub(super) fn publish_gm_host_boot_requested(value: bool) {
    GM_HOST_BOOT_REQUESTED.with(|slot| *slot.borrow_mut() = value);
}

pub(super) fn read_fleet_join_generation() -> u64 {
    FLEET_JOIN_GENERATION.with(|value| *value.borrow())
}

pub(super) fn queue_fleet_lobby_input(input: FleetLobbyInput) -> bool {
    let generation = self::read_fleet_join_generation();
    PENDING_FLEET_LOBBY_INPUTS.with(|pending| {
        queue_fleet_lobby_input_bounded(
            &mut pending.borrow_mut(),
            generation,
            input,
            MAX_FLEET_LOBBY_INPUTS,
        )
    })
}

pub(super) fn browser_save_namespace() -> String {
    BROWSER_SAVE_NAMESPACE.with(|cached| {
        if let Some(namespace) = cached.borrow().as_ref() {
            return namespace.clone();
        }

        let window = web_sys::window();
        let from_property = window.as_ref().and_then(|window| {
            Reflect::get(
                window.as_ref(),
                &JsValue::from_str(BROWSER_SAVE_IDENTITY_PROPERTY),
            )
            .ok()
            .and_then(|value| value.as_string())
        });
        let from_session = window.as_ref().and_then(|window| {
            window
                .session_storage()
                .ok()
                .flatten()
                .and_then(|storage| storage.get_item(BROWSER_SAVE_IDENTITY_KEY).ok().flatten())
        });
        let mut identity = from_property
            .or(from_session)
            .filter(|identity| scoped_browser_save_namespace(identity).is_some())
            .unwrap_or_else(mint_fallback_browser_save_identity);

        // A valid identity is the only input accepted by the namespace helper.
        // The fallback minter is defined to produce the same 32-lower-hex shape.
        let namespace = scoped_browser_save_namespace(&identity).unwrap_or_else(|| {
            identity = mint_fallback_browser_save_identity();
            scoped_browser_save_namespace(&identity)
                .expect("the browser save identity minter must produce 32 lowercase hex digits")
        });

        if let Some(window) = window {
            let _ = Reflect::set(
                window.as_ref(),
                &JsValue::from_str(BROWSER_SAVE_IDENTITY_PROPERTY),
                &JsValue::from_str(&identity),
            );
            if let Ok(Some(storage)) = window.session_storage() {
                let _ = storage.set_item(BROWSER_SAVE_IDENTITY_KEY, &identity);
            }
        }

        *cached.borrow_mut() = Some(namespace.clone());
        namespace
    })
}

pub(super) fn take_mesh_frames() -> String {
    MESH_OUTBOUND.with(|q| {
        let frames = std::mem::take(&mut *q.borrow_mut());
        format!("[{}]", frames.join(","))
    })
}

pub(super) fn join_fleet(roster_json: &str) -> String {
    let generation = FLEET_JOIN_GENERATION.with(|counter| {
        let mut counter = counter.borrow_mut();
        *counter = counter.wrapping_add(1).max(1);
        *counter
    });
    if crate::core::codec::decode_fleet_roster(roster_json).is_none() {
        PENDING_FLEET_ADOPTIONS.with(|pending| {
            let mut pending = pending.borrow_mut();
            if matches!(pending.back(), Some(PendingFleetAdoption::Join(_))) {
                pending.pop_back();
            }
        });
        self::publish_fleet_join_status(crate::lockstep::FleetJoinStatus {
            generation,
            status: crate::lockstep::FleetJoinStatusKind::Refused,
            reason: Some("fleet-roster-unreadable".to_string()),
        });
        return "fleet-roster-unreadable".to_string();
    }
    PENDING_FLEET_LOBBY_INPUTS.with(|pending| {
        let mut pending = pending.borrow_mut();
        // This join supersedes any not-yet-drained control projection from the
        // previous generation. Rebind the latest absolute values so the common
        // setters→join ordering cannot leave the freshly adopted World at its
        // unmanaged/fail-closed defaults.
        rebind_fleet_lobby_projections(
            &mut pending,
            generation,
            self::read_latest_fleet_managed(),
            self::read_latest_fleet_validation(),
        );
    });
    PENDING_FLEET_ADOPTIONS.with(|pending| {
        let mut pending = pending.borrow_mut();
        // A newer join cancels an older join that Bevy has not adopted yet.
        // Preserve a preceding Leave: leave→reopen→join in one animation frame
        // must tear down the old generation before installing the new one.
        if matches!(pending.back(), Some(PendingFleetAdoption::Join(_))) {
            pending.pop_back();
        }
        pending.push_back(PendingFleetAdoption::Join(PendingFleetJoin {
            generation,
            roster_json: roster_json.to_string(),
        }));
    });
    self::publish_fleet_join_status(crate::lockstep::FleetJoinStatus {
        generation,
        status: crate::lockstep::FleetJoinStatusKind::Pending,
        reason: None,
    });
    generation.to_string()
}

pub(super) fn leave_fleet() -> String {
    let generation = FLEET_JOIN_GENERATION.with(|counter| {
        let mut counter = counter.borrow_mut();
        *counter = counter.wrapping_add(1).max(1);
        *counter
    });
    PENDING_FLEET_ADOPTIONS.with(|pending| {
        let mut pending = pending.borrow_mut();
        pending.clear();
        pending.push_back(PendingFleetAdoption::Leave { generation });
    });
    self::publish_fleet_join_status(crate::lockstep::FleetJoinStatus {
        generation,
        status: crate::lockstep::FleetJoinStatusKind::Pending,
        reason: None,
    });
    generation.to_string()
}

pub(super) fn fleet_join_status() -> String {
    FLEET_JOIN_STATUS.with(|status| {
        crate::core::codec::encode_fleet_join_status(&status.borrow()).unwrap_or_default()
    })
}

pub(super) fn submit_gm_action(request_json: &str) -> bool {
    const LIMIT: usize = 64;
    let Some(request) = crate::core::codec::decode_gm_action_request(request_json) else {
        return false;
    };
    PENDING_GM_ACTIONS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.len() >= LIMIT {
            return false;
        }
        pending.push_back(request);
        true
    })
}

pub(super) fn begin_gm_join(
    join_id: u64,
    approved_by: u32,
    candidate_host: u32,
    operator_id: &str,
    scenario: &str,
    join_kind: &str,
) -> bool {
    const LIMIT: usize = 4;
    if join_id == 0
        || approved_by == 0
        || candidate_host == 0
        || operator_id.is_empty()
        || operator_id.chars().count() > crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS
        || scenario.is_empty()
        || scenario.len() > 4096
    {
        return false;
    }
    let kind = match join_kind {
        "first-time" => crate::gm_join::GmJoinKind::FirstTime,
        "reconnect" => crate::gm_join::GmJoinKind::Reconnect,
        _ => return false,
    };
    let request = PendingGmJoin {
        id: crate::gm_join::GmJoinId(join_id),
        kind,
        approved_by: crate::command_admission::HostSlot(approved_by),
        candidate: crate::gm_join::GmJoinCandidate {
            host: crate::command_admission::HostSlot(candidate_host),
            operator_id: operator_id.to_string(),
        },
        scenario: scenario.to_string(),
    };
    PENDING_GM_JOINS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if let Some(existing) = pending.iter().find(|existing| existing.id == request.id) {
            return existing == &request;
        }
        if pending.len() >= LIMIT {
            return false;
        }
        pending.push_back(request);
        true
    })
}

pub(super) fn prepare_gm_join_candidate(join_id: u64, roster_json: &str) -> bool {
    const LIMIT: usize = 2;
    if join_id == 0 {
        return false;
    }
    let Some((provisional, _)) = crate::core::codec::decode_fleet_roster(roster_json) else {
        return false;
    };
    if crate::gm_join::GmJoinBootstrap::from_provisional(provisional.clone()).is_err() {
        return false;
    }
    let request = PendingGmJoinBootstrap {
        id: crate::gm_join::GmJoinId(join_id),
        provisional,
    };
    PENDING_GM_JOIN_BOOTSTRAPS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if let Some(existing) = pending.iter().find(|existing| existing.id == request.id) {
            return existing == &request;
        }
        if pending.len() >= LIMIT {
            return false;
        }
        pending.push_back(request);
        true
    })
}

pub(super) fn refuse_gm_join(join_id: u64, reason: &str) -> bool {
    const LIMIT: usize = 4;
    if join_id == 0 || reason != "candidate-disconnected" {
        return false;
    }
    PENDING_GM_JOIN_REFUSALS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.len() >= LIMIT {
            return false;
        }
        pending.push_back(PendingGmJoinRefusal {
            id: crate::gm_join::GmJoinId(join_id),
            reason: crate::gm_join::GmJoinRefusal::CandidateDisconnected,
        });
        true
    })
}

pub(super) fn gm_join_status() -> String {
    GM_JOIN_STATUS.with(|status| {
        crate::core::codec::encode_gm_join_progress(&status.borrow()).unwrap_or_default()
    })
}

pub(super) fn take_start_result() -> String {
    START_GRANT_RESULTS.with(|results| results.borrow_mut().pop_front().unwrap_or_default())
}

pub(super) fn queue_browser_save(intent: BrowserSaveIntent) -> Option<String> {
    let source = intent_source(&intent);
    let token = crate::save_slots::new_manual_slot_id();
    let accepted = PENDING_BROWSER_SAVES
        .with(|pending| pending.borrow_mut().try_push(token.clone(), intent).is_ok());
    if accepted {
        Some(token)
    } else {
        set_snapshot_status(
            false,
            source,
            "too many local save requests are pending; try again after one finishes",
        );
        None
    }
}

pub(super) fn set_snapshot_status(ok: bool, source: &str, message: impl Into<String>) {
    SNAPSHOT_STATUS.with(|statuses| {
        statuses
            .borrow_mut()
            .push_back((ok, source.to_string(), message.into()));
    });
}

pub(super) fn snapshot_status() -> String {
    SNAPSHOT_STATUS.with(|s| {
        s.borrow_mut()
            .pop_front()
            .map_or_else(String::new, |(ok, source, message)| {
                format!("{}\t{source}\t{message}", if ok { "ok" } else { "error" })
            })
    })
}

pub(super) fn take_exported_snapshot() -> String {
    EXPORTED_ARTIFACT.with(|a| a.borrow_mut().take().unwrap_or_default())
}

pub(super) fn boot_profile() -> String {
    ACTIVE_BOOT_PROFILE.with(|active| active.borrow().to_string())
}

pub(super) fn take_ship_config() -> Option<ShipConfig> {
    SHIP_CONFIG.with(|slot| slot.borrow_mut().take())
}

pub(super) fn read_ship_stations() -> Option<ShipStations> {
    SHIP_STATIONS.with(|slot| slot.borrow().clone())
}

pub(super) fn restore_boot_identity() -> Option<crate::snapshot::BootIdentity> {
    PENDING_RESTORE_STAGED.with(|pending| {
        pending
            .borrow()
            .as_ref()
            .and_then(|run| run.snapshot.as_ref())
            .and_then(|snapshot| snapshot.state.boot_identity.clone())
    })
}

pub(super) fn snapshot_scenario() -> Option<String> {
    SNAPSHOT_WORLD.with(|world| {
        world
            .borrow()
            .as_ref()
            .map(|(scenario, _)| scenario.clone())
    })
}

pub(super) fn complete_fleet_adoption(generation: u64, accepted: bool, refusal: &str) {
    FLEET_JOIN_STATUS.with(|status| {
        let mut status = status.borrow_mut();
        // A cancelled older action can still precede the latest generation
        // in this same drain (leave→join). Never let its completion regress
        // the poller to a stale generation.
        if status.generation == generation {
            *status = crate::lockstep::FleetJoinStatus {
                generation,
                status: if accepted {
                    crate::lockstep::FleetJoinStatusKind::Accepted
                } else {
                    crate::lockstep::FleetJoinStatusKind::Refused
                },
                reason: (!accepted).then(|| refusal.to_string()),
            };
        }
    })
}

pub(super) fn publish_mesh_frames(frames: &[crate::lockstep::MeshFrame]) {
    MESH_OUTBOUND.with(|q| {
        let mut q = q.borrow_mut();
        for frame in frames {
            match crate::core::codec::encode_mesh_frame(frame) {
                Ok(json) => q.push(json),
                // A frame that will not encode is dropped with a warning rather
                // than panicking the host: the fleet will stall on the missing
                // watermark and SAY so, which is a better failure than a dead
                // page.
                Err(e) => warn!("dropping an unencodable host-mesh frame: {e}"),
            }
        }
    })
}

pub(super) fn publish_start_result(encoded: String) {
    START_GRANT_RESULTS.with(|outbox| {
        let mut outbox = outbox.borrow_mut();
        if outbox.len() == crate::lobby::server::MAX_START_GRANT_RESULTS {
            outbox.pop_front();
        }
        outbox.push_back(encoded);
    });
}

pub(super) fn take_save_requests() -> VecDeque<String> {
    PENDING_BROWSER_SAVES.with(|pending| pending.borrow_mut().take_requests())
}

pub(super) fn complete_save_intent(token: &str) -> Option<BrowserSaveIntent> {
    PENDING_BROWSER_SAVES.with(|pending| pending.borrow_mut().remove_intent(token))
}

#[cfg(not(phoenix_demo_build))]
pub(super) fn request_debug_surface(surface: DebugSurface, enabled: bool) {
    PENDING_DEBUG_SURFACE_STATES.with(|pending| {
        pending.borrow_mut().insert(surface, enabled);
    })
}

#[cfg(not(phoenix_demo_build))]
pub(super) fn take_debug_surfaces() -> Vec<(DebugSurface, bool)> {
    PENDING_DEBUG_SURFACE_STATES.with(|states| states.borrow_mut().drain().collect())
}

pub(super) fn outbound_callback() -> Option<Function> {
    OUTBOUND_CB.with(|slot| slot.borrow().clone())
}

pub(super) fn host_channel_callback() -> Option<Function> {
    HOST_CHANNEL_CB.with(|slot| slot.borrow().clone())
}

pub(super) fn take_fleet_lobby_inputs(
    adoption: &crate::lockstep::FleetJoinStatus,
) -> Option<VecDeque<FleetLobbyInput>> {
    let wrapped =
        PENDING_FLEET_LOBBY_INPUTS.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    let inputs = wrapped
        .into_iter()
        .filter(|row| row.generation == adoption.generation)
        .map(|row| row.input)
        .collect();
    match adoption.status {
        crate::lockstep::FleetJoinStatusKind::Pending => {
            retry_fleet_lobby_inputs(adoption.generation, inputs);
            None
        }
        crate::lockstep::FleetJoinStatusKind::Refused => None,
        _ => Some(inputs),
    }
}

pub(super) fn retry_fleet_lobby_inputs(generation: u64, inputs: VecDeque<FleetLobbyInput>) {
    PENDING_FLEET_LOBBY_INPUTS.with(|pending| {
        let mut pending = pending.borrow_mut();
        for input in inputs.into_iter().rev() {
            pending.push_front(PendingFleetLobbyInput { generation, input });
        }
    });
}
