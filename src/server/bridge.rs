// WASM/JS bridge — all public functions are #[wasm_bindgen] exports.
//
// On native targets this module carries the debug-toggle MARSHALLING system —
// `drain_client_debug_flags` (the phone route that feeds the canonical
// `debug::catalogue` adapters, moved here
// from `debug_overlay` in issue #1193 so the always-compiled sim half of the
// overlay names nothing under `crate::server::bridge`). The canonical identity
// lives in `core::debug_surface`; there is no second pending-toggle enum. The
// WASM-specific glue (thread-locals, wasm_bindgen exports, the
// host-page Bevy drain system) is gated behind #[cfg(target_arch = "wasm32")].

#[cfg(not(phoenix_demo_build))]
use crate::core::debug_surface::DebugSurface;

#[cfg(all(target_arch = "wasm32", not(phoenix_demo_build)))]
use std::collections::HashMap;

#[cfg(test)]
use crate::lobby::FleetLobbyInput;
#[cfg(any(target_arch = "wasm32", test))]
use std::collections::{BTreeMap, VecDeque};

// `drain_client_debug_flags` (moved here from `debug_overlay` in issue #1193) is
// a Bevy system, so it needs these prelude types on the native path — the rest
// of this module's native portion is Bevy-free. On WASM they come from the
// `bevy::prelude::*` glob in the gated `use` block below, and under a demo build
// the drain is compiled out entirely, so the import is scoped to match.
#[cfg(all(not(target_arch = "wasm32"), not(phoenix_demo_build)))]
use bevy::prelude::{Commands, MessageReader};

// The public GM-roster replacement is covered by native unit tests, including
// the demo-build gate. Keep `World` available for that test-only path even
// though the browser debug drain above is intentionally absent from a demo.
#[cfg(all(not(target_arch = "wasm32"), any(not(phoenix_demo_build), test)))]
use bevy::prelude::World;

// The force-start path stopped being wasm-only in issue #1328: a native host's
// lobby is its viewscreen, and a lobby with only phone crew has to be
// launchable from it. `PendingForceStart` and `apply_force_start` below are
// therefore compiled on every target, so these four prelude names are too.
// Named explicitly rather than glob-imported because the rest of this module's
// native portion is deliberately Bevy-free — see the module note. On WASM the
// gated `bevy::prelude::*` below also provides them; an explicit import wins
// over a glob, so the two do not collide.
use bevy::prelude::{NextState, Res, ResMut, Resource, State};
// `apply_force_start` (native since #1328) reads `FleetManagedLobby` by bare name;
// the wasm `use` block below carries it for the browser path, so import it for the
// native path here. Gated off wasm to avoid colliding with that explicit import.
#[cfg(not(target_arch = "wasm32"))]
use crate::lobby::FleetManagedLobby;

#[cfg(target_arch = "wasm32")]
use {
    crate::asteroids::lifecycle::AsteroidLifecyclePlugin,
    crate::boot::{BootPlan, BootProfile, WorldIngest},
    crate::console_bridge::{
        AiChatterEvent, AudioConfigChanged, AudioCueEvent, GmActivityFeedChanged, GmCommsChanged,
        GmEntityProjectionChanged, GmMissionChanged, GmSessionChanged, GmSpawnChanged,
        GmStationProjectionChanged, HudStateChanged, LobbyStateChanged,
    },
    crate::core::codec::{self, JsonCodec},
    crate::core::messages::{self, DeliveryClass},
    crate::entities::config_cache::ConfigCachePlugin,
    crate::gm_activity::GmActivityPlugin,
    crate::gm_projection::{BrowserGameMaster, GmProjectionPlugin},
    crate::lobby::stations_config::ShipStations,
    crate::lobby::{
        FleetLobbyInput, FleetManagedLobby, InboundMessage, LobbyPlugin, OutboundMessage,
        PendingStartGrants, PlayerDisconnected, SelectedShipResource, StartGrantResults, Target,
    },
    crate::modifiers::coordination::ModifierCoordinationPlugin,
    crate::server_app::{add_simulation_plugins_with, SimPluginOptions},
    crate::ship::config::ShipConfig,
    crate::ship_plugin::PendingShipConfig,
    crate::world::load::WasmReader,
    crate::world::WorldPlugin,
    bevy::{log::LogPlugin, prelude::*},
    js_sys::{Array, Function, Object, Reflect},
    wasm_bindgen::prelude::*,
};

/// Drain `ClientMessage::ToggleDebugFlag` from connected phones and apply it.
///
/// Moved here from `debug_overlay` in issue #1193: it is the phone-route
/// MARSHALLING that feeds the Debug Surface catalogue, so it belongs on the
/// bridge/marshalling side of the sim↔presentation seam rather than in the
/// always-compiled overlay. `debug_overlay` keeps the authority filter
/// ([`crate::debug_overlay::admitted_flag_toggles`], still sim-side) and the
/// overlay resources this flips; only the bridge command queue lives here.
///
/// **Not compiled into a demo build**, and neither is the message it reads.
///
/// Reads raw `InboundMessage` rather than `AdmittedCommands` deliberately —
/// see the variant's doc for why these never cross command admission. The
/// authority check is not skipped, it is `admitted_flag_toggles`.
///
/// The flag-flipping itself is `debug::catalogue::apply_pending_toggles`, the
/// same module-owned adapter seam the host page uses. Pause is not a
/// `DebugSurface`, so this drain cannot touch the clock by construction.
#[cfg(not(phoenix_demo_build))]
pub fn drain_client_debug_flags(
    mut reader: MessageReader<crate::lobby::InboundMessage>,
    sessions: Res<crate::lobby::Sessions>,
    mut commands: Commands,
) {
    let mut requests: Vec<(String, DebugSurface)> = Vec::new();
    for ev in reader.read() {
        if let crate::core::messages::ClientMessage::ToggleDebugFlag { flag } = &ev.msg {
            requests.push((ev.token.clone(), *flag));
        }
    }
    if requests.is_empty() {
        return;
    }

    let pending = crate::debug_overlay::admitted_flag_toggles(
        requests.iter().map(|(token, flag)| (token.as_str(), *flag)),
        |token| sessions.0.players().iter().any(|p| p.token == token),
    );
    if pending.is_empty() {
        return;
    }

    commands.queue(move |world: &mut World| {
        crate::debug::catalogue::apply_pending_toggles(world, pending);
    });
}

#[cfg(target_arch = "wasm32")]
#[path = "browser_edge.rs"]
mod browser_edge;
#[cfg(target_arch = "wasm32")]
use browser_edge as edge;

// ── De-globalised bridge state (issue #1181) ────────────────────────────────
//
// These typed Bevy Resources hold the STATE that simulation systems read or
// write, moved out of the thread-locals below so the access is visible to the
// scheduler and the seam logic is unit-testable on native without a JS host.
//
// The wasm edge KEEPS a minimal thread-local inbox/outbox (see the big comment
// in `browser_edge`): a JS call arrives synchronously, outside Bevy's
// schedule and with no `World` handle, so the value it carries has nowhere to
// live but a thread-local until a `PreUpdate` seam system can drain it into one
// of these Resources; symmetrically a value the sim produced has to be mirrored
// back into a thread-local for a JS getter that likewise has no `World`. What
// moved here is the DURABLE, sim-visible state in between; what stayed at the
// edge is only that transient transport.
//
// Two former members of this block moved to sim-side homes in issue #1194, so
// this presentation module no longer DEFINES sim-visible state that
// always-compiled code reads: `Instagib` now lives beside its sibling `GodMode`
// in `crate::server_app`, and `BridgeWorldSource` beside its `RawWorldSource`
// consumer in `crate::world::server`. The wasm edge below only mirrors, drains,
// and inserts them through those new paths.

#[cfg(target_arch = "wasm32")]
struct PendingFleetJoin {
    generation: u64,
    roster_json: String,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingGmJoin {
    id: crate::gm_join::GmJoinId,
    kind: crate::gm_join::GmJoinKind,
    approved_by: crate::command_admission::HostSlot,
    candidate: crate::gm_join::GmJoinCandidate,
    scenario: String,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingGmJoinBootstrap {
    id: crate::gm_join::GmJoinId,
    provisional: crate::lockstep::FleetRoster,
}

#[cfg(target_arch = "wasm32")]
struct PendingGmJoinRefusal {
    id: crate::gm_join::GmJoinId,
    reason: crate::gm_join::GmJoinRefusal,
}

#[cfg(target_arch = "wasm32")]
enum PendingFleetAdoption {
    Join(PendingFleetJoin),
    Leave { generation: u64 },
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug)]
struct PendingFleetLobbyInput {
    generation: u64,
    input: FleetLobbyInput,
}

/// Apply a batch of queued instagib-toggle requests to the flag (issue #1181).
///
/// Pure, so the drain semantics are unit-testable on native. Each queued toggle
/// flips the flag once, so the net effect is a parity of `count` — two clicks in
/// one frame cancel, matching what two flips on two ticks would do. Mirrors the
/// count-based God Mode drain (`PENDING_GOD_MODE_TOGGLES`), minus the command
/// admission route instagib deliberately does not take (see the instagib
/// helper below).
pub fn apply_instagib_toggles(count: u32, current: &mut bool) {
    if count % 2 == 1 {
        *current = !*current;
    }
}

/// Raw host-only mutations are safe only before a participant wait-set exists.
///
/// These controls predate fleet lockstep and carry no tick/order/origin. Once a
/// second simulation participates, applying one locally would fork the world;
/// the narrow #1290 policy is therefore to consume and refuse them while a
/// [`crate::lockstep::FleetLockstep`] resource is installed.
#[cfg(target_arch = "wasm32")]
pub(crate) const fn raw_host_control_allowed(fleet_active: bool) -> bool {
    !fleet_active
}

// ── Host teleport-to-waypoint override (issue #770) ─────────────────────────
//
// A deliberate host-only simulation override: snap the LocalShip's authoritative
// position onto the shared Navigation waypoint. Unlike a client helm command it
// does NOT go through command admission — it directly sets `ShipPhysics.{x,z}`,
// a discontinuous jump contrasted with the helm's velocity integration. The pure
// override logic is `crate::console::navigation::server::apply_teleport_to_waypoint`
// (relocated sim-side in issue #1194 — it mutates only `ShipPhysics` from a
// `NavigationWaypoint`, so it must not sit in this presentation module); the wasm
// glue (thread-local, `wasm_bindgen` export, the Bevy drain system) stays here,
// gated below, and calls into it.

// Browser storage belongs to browser_edge. This module exposes synchronous JS
// exports and scheduled Bevy drains/publications; it holds no ambient cells.
// Browser callbacks and pre-App staging cannot be Bevy Resources. Their typed
// adapter operations return owned data and never retain a World handle.

/// What the browser should do with a fixed-tick manual capture once it reaches
/// the peer-local outbox. This is storage/presentation intent only; the sim sees
/// an opaque token and never synchronises any of these values.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug)]
enum BrowserSaveIntent {
    /// The original debug API: write exactly the Store slot it was handed.
    LegacySlot(String),
    /// Catalogue create: stable internal id plus arbitrary display metadata.
    CreateManual {
        slot_id: String,
        display_name: String,
    },
    /// Capture the current run and expose its portable RON through the existing
    /// one-shot export getter (#866).
    ExportCurrent,
}

/// Maximum number of browser-originated captures that may be outstanding at
/// once. The request FIFO and its intent catalogue share this one limit so a
/// render frame cannot enqueue an unbounded number of full snapshot walks.
#[cfg(any(target_arch = "wasm32", test))]
const MAX_PENDING_BROWSER_SAVES: usize = 64;

/// Session-storage key populated by `gui/browser-save-identity.js` before the
/// first catalogue read. The Rust bridge reads the same value so every Store
/// operation — including fixed-tick autosaves with no JS action attached — is
/// scoped to this simulation peer rather than the origin.
#[cfg(target_arch = "wasm32")]
const BROWSER_SAVE_IDENTITY_KEY: &str = "phoenix-save-peer-id";

/// Window property carrying the identity when Storage access itself is denied.
/// This is an in-memory fallback only; vellum-save will still report the local
/// backend refusal, but a second live peer must not fall back to a shared key.
#[cfg(target_arch = "wasm32")]
const BROWSER_SAVE_IDENTITY_PROPERTY: &str = "__phoenixSavePeerIdentity";

#[cfg(any(target_arch = "wasm32", test))]
fn scoped_browser_save_namespace(identity: &str) -> Option<String> {
    let valid = identity.len() == 32
        && identity
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
    valid.then(|| format!("{}:{identity}", crate::snapshot::STORAGE_NAMESPACE))
}

/// Maximum number of save outcomes retained for the host page's status poll.
/// When the host stops polling, new outcomes replace the oldest one rather
/// than letting this presentation-only outbox grow without bound.
#[cfg(target_arch = "wasm32")]
const MAX_BROWSER_SAVE_STATUSES: usize = 64;

/// The browser edge's atomic request/intent pair.
///
/// Taking the request FIFO does not release capacity: the intent remains until
/// the corresponding fixed-boundary result is drained (or the request is
/// refused). This bounds the whole in-flight lifetime, not just one rendered
/// frame's ingress.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug)]
struct PendingBrowserSaves<I> {
    requests: VecDeque<String>,
    intents: BTreeMap<String, I>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl<I> PendingBrowserSaves<I> {
    const fn new() -> Self {
        Self {
            requests: VecDeque::new(),
            intents: BTreeMap::new(),
        }
    }

    fn try_push(&mut self, token: String, intent: I) -> Result<(), I> {
        if self.requests.len() >= MAX_PENDING_BROWSER_SAVES
            || self.intents.len() >= MAX_PENDING_BROWSER_SAVES
            || self.intents.contains_key(&token)
        {
            return Err(intent);
        }
        self.intents.insert(token.clone(), intent);
        self.requests.push_back(token);
        Ok(())
    }

    fn take_requests(&mut self) -> VecDeque<String> {
        std::mem::take(&mut self.requests)
    }

    fn remove_intent(&mut self, token: &str) -> Option<I> {
        self.intents.remove(token)
    }
}

/// A bounded FIFO that keeps the newest `LIMIT` values. Retained values are
/// still observed oldest-first, while an inactive poller cannot create an
/// unbounded result queue. In particular, an overload refusal is always the
/// newest retained status and therefore remains visible locally.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug)]
struct BoundedFifo<T, const LIMIT: usize> {
    values: VecDeque<T>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl<T, const LIMIT: usize> BoundedFifo<T, LIMIT> {
    const fn new() -> Self {
        Self {
            values: VecDeque::new(),
        }
    }

    fn push_back(&mut self, value: T) {
        if LIMIT == 0 {
            return;
        }
        if self.values.len() == LIMIT {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    fn pop_front(&mut self) -> Option<T> {
        self.values.pop_front()
    }

    #[cfg(target_arch = "wasm32")]
    fn clear(&mut self) {
        self.values.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.values.len()
    }
}

#[cfg(any(target_arch = "wasm32", test))]
const MAX_FLEET_LOBBY_INPUTS: usize = 64;

/// Queue one ordered fleet-lobby edge without silently losing generation state.
///
/// Consecutive validation samples are absolute-state projections, so only the
/// newest one matters and may replace the tail in place. Consecutive identical
/// managed samples are likewise idempotent. A managed transition, a grant, or
/// a validation separated from the previous sample by either of those edges is
/// never displaced: refusal lets the page retry without changing ordering.
#[cfg(any(target_arch = "wasm32", test))]
fn queue_fleet_lobby_input_bounded(
    pending: &mut VecDeque<PendingFleetLobbyInput>,
    generation: u64,
    input: FleetLobbyInput,
    limit: usize,
) -> bool {
    if let Some(back) = pending.back_mut() {
        match (&mut back.input, &input) {
            (FleetLobbyInput::Validation(queued), FleetLobbyInput::Validation(latest))
                if back.generation == generation =>
            {
                *queued = *latest;
                return true;
            }
            (FleetLobbyInput::Managed(queued), FleetLobbyInput::Managed(latest))
                if back.generation == generation && *queued == *latest =>
            {
                return true;
            }
            _ => {}
        }
    }
    if pending.len() >= limit {
        return false;
    }
    pending.push_back(PendingFleetLobbyInput { generation, input });
    true
}

#[cfg(any(target_arch = "wasm32", test))]
fn rebind_fleet_lobby_projections(
    pending: &mut VecDeque<PendingFleetLobbyInput>,
    generation: u64,
    managed: Option<bool>,
    validation: Option<bool>,
) {
    pending.clear();
    if let Some(enabled) = managed {
        let queued = queue_fleet_lobby_input_bounded(
            pending,
            generation,
            FleetLobbyInput::Managed(enabled),
            MAX_FLEET_LOBBY_INPUTS,
        );
        debug_assert!(queued);
    }
    if let Some(valid) = validation {
        let queued = queue_fleet_lobby_input_bounded(
            pending,
            generation,
            FleetLobbyInput::Validation(valid),
            MAX_FLEET_LOBBY_INPUTS,
        );
        debug_assert!(queued);
    }
}

#[cfg(target_arch = "wasm32")]
fn queue_fleet_lobby_input(input: FleetLobbyInput) -> bool {
    edge::queue_fleet_lobby_input(input)
}

/// Resolve and cache the private browser Store namespace.
///
/// `gui/browser-save-identity.js` normally establishes the identity before the
/// catalogue mounts. The fallback is minted inside this WASM instance and
/// mirrored back into sessionStorage/window so an early direct export remains
/// isolated too. It does not enter simulation state or a peer message.
#[cfg(target_arch = "wasm32")]
fn browser_save_namespace() -> String {
    edge::browser_save_namespace()
}

#[cfg(target_arch = "wasm32")]
fn mint_fallback_browser_save_identity() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // `getrandom/wasm_js` uses Web Crypto in supported browsers. If a host
        // denies that API as well as storage, mix the two JS-local clocks so
        // this live instance still does not collapse onto a shared namespace;
        // vellum-save will report the durable backend refusal separately.
        let mixed = js_sys::Date::now().to_bits() ^ js_sys::Math::random().to_bits();
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = mixed.rotate_left(index as u32).to_le_bytes()[index % 8];
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(target_arch = "wasm32")]
fn browser_save_store() -> vellum_save::LocalStorage {
    vellum_save::LocalStorage::new(browser_save_namespace())
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
    /// Rendererless GM peer's absolute omniscient ship-map projection. This
    /// callback is page-local and never enters the peer transport.
    pub const GM_ENTITY: &str = "gm_entity";
    /// Rendererless GM peer's absolute bounded multi-category feed. This
    /// callback is page-local and never enters the peer transport.
    pub const GM_ACTIVITY: &str = "gm_activity";
    /// Rendererless GM peer's authored Station-interface projection.
    pub const GM_STATION: &str = "gm_station";
    /// Authoritative pause state plus attributed typed-action results.
    pub const GM_SESSION: &str = "gm_session";
    /// The GM-operable authored-event registry and its attributed results
    /// (issue #1301) — what the mission panel lists and fires.
    pub const GM_MISSION: &str = "gm_mission";
    /// The scenario-authored GM spawn palette and its attributed placement
    /// results (issue #1305) — what the placement panel lists and places.
    pub const GM_SPAWN: &str = "gm_spawn";
    pub const GM_COMMS: &str = "gm_comms";

    /// Every registered host channel name. The JS dispatcher table in
    /// `server.html` must have a handler per entry.
    pub const ALL: [&str; 14] = [
        HUD,
        LOBBY,
        CHATTER,
        AUDIO_CONFIG,
        AUDIO_CUE,
        SHAKE,
        AUDIO_LEVEL,
        GM_ENTITY,
        GM_ACTIVITY,
        GM_STATION,
        GM_SESSION,
        GM_MISSION,
        GM_SPAWN,
        GM_COMMS,
    ];
}

/// Select the explicit production rendererless browser GM profile. The page
/// calls this before [`wasm_init`]; it deliberately does not depend on
/// `navigator.webdriver`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_prepare_game_master() {
    edge::publish_gm_host_boot_requested(true);
}

/// Read-only identity of the profile actually composed by [`wasm_init`].
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_boot_profile() -> String {
    edge::boot_profile()
}

// ── Instagib helper (issue #900 context, de-globalised in #1181) ────────────
//
// Unlike God Mode (issue #900), Instagib is not routed through command
// admission — it flips the [`crate::server_app::Instagib`] Resource directly. Since issue #1181 the
// authoritative flag lives in that Resource (read by `tick_beams_apply_damage`)
// rather than a thread-local `is_instagib()` reached ambiently; the wasm edge
// keeps only the toggle inbox and the read-back mirror.

/// Called by JS (host Debug panel God Mode button) to request a God Mode
/// flip (issue #900). Unlike the old thread-local this does NOT flip
/// anything itself: it queues a request that `drain_god_mode_toggle` turns
/// into a `ToggleGodMode` command crossing the normal admission boundary on
/// the next `PreUpdate` frame, so the flip carries a tick, lands in the
/// command log, and replays. The JS binding's signature is unchanged.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_god_mode() {
    edge::increment_pending_god_mode_toggles();
}

/// Called by JS to read the LocalShip's current God Mode state (issue #900),
/// e.g. to reflect it on the Debug panel button. Reads the mirror maintained
/// by `publish_god_mode`, since the authoritative value now lives in the
/// `GodMode` Bevy resource rather than a thread-local this function can touch
/// directly.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_god_mode() -> bool {
    edge::read_god_mode_mirror()
}

/// Called by JS (settings cog Debug/Cheat tab) to request an instagib flip.
///
/// Queues a request that `drain_instagib_toggle` applies to the [`crate::server_app::Instagib`]
/// Resource on the next `PreUpdate` frame (issue #1181). The JS binding's
/// signature is unchanged; only the state it targets moved from a thread-local
/// into a Resource `tick_beams_apply_damage` reads through the scheduler.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_instagib() {
    edge::increment_pending_instagib_toggles();
}

/// Called by JS each frame to read back the instagib flag for the cog button
/// (issue #1181). Reads the `INSTAGIB_MIRROR` maintained by `publish_instagib`,
/// since the authoritative value now lives in the [`crate::server_app::Instagib`] Resource this
/// `World`-less function cannot touch directly (same pattern as
/// `wasm_get_god_mode`).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_instagib() -> bool {
    edge::read_instagib_mirror()
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
/// Resolution goes through [`crate::entities::include_resolve::HostFragmentSource`], the
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
    if !crate::entities::config_cache::is_raw_template_delivered(template_path) {
        crate::entities::config_cache::record_raw_template(template_path, toml_str.to_string());
    }
    let resolved = crate::entities::include_resolve::resolve_template(
        template_path,
        &crate::entities::include_resolve::HostFragmentSource,
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
/// human-readable error string. The crew transport should not start when this returns
/// an error.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_validate_stations(template_path: &str, toml_str: &str) -> Result<JsValue, JsValue> {
    let ship_config =
        validate_ship_stations(template_path, toml_str).map_err(|e| JsValue::from_str(&e))?;
    let stations = crate::lobby::stations_config::stations_from_ship_config(&ship_config);
    edge::publish_ship_stations(Some(stations));
    edge::publish_ship_config(Some(ship_config));
    Ok(JsValue::UNDEFINED)
}

/// Called by JS on page load. Builds and runs the Bevy app.
///
/// A [`boot::build`](crate::boot::build) adapter since issue #1219: the shared
/// core, the renderer axis (the real viewscreen stack for the host, the surrogate
/// for automation), and the world-ingestion order (the Rhai hashing-seed pin and
/// the content-ledger freeze — the browser's world itself arrives by the JS
/// preload, so the plan is [`WorldIngest::HostPreloaded`]) all come from
/// [`crate::boot`]. The two branches now differ by exactly one thing — the
/// [`BootProfile`] the WebDriver (`is_automation`) probe picks. What stays here is
/// the genuinely browser-only wiring boot has no reason to know about: the
/// WebDriver probe, the `?log=` URL parse, the JS ingress/egress `PreUpdate`/
/// `PostUpdate` seams, the debug-overlay/winit/audio wiring, and the thread-local
/// resource hand-offs.
///
/// In WASM, `App::run()` hands control to requestAnimationFrame and returns
/// immediately, so this function does not block.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_init() {
    if !crate::entities::config_cache::wasm_is_preload_complete() {
        web_sys::console::error_1(&JsValue::from_str(
            "Content preload is incomplete; refusing to initialise",
        ));
        return;
    }
    // Route Rust panics through console.error with a useful message + location.
    // Without this, a panic in any Bevy system traps the wasm instance and
    // every subsequent JS→WASM call surfaces as a bare "RuntimeError: memory
    // access out of bounds" pointing at whatever entry point fired next
    // (typically `wasm_receive_message` since the host page receives client
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
    let is_browser_gm = edge::read_gm_host_boot_requested();

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

    // The boot seam (issue #1219). Both branches are the SAME `boot::build` call —
    // the shared core, the renderer axis (the real viewscreen stack for the host,
    // the surrogate the automation branch used to spell out by hand), and the
    // world-ingestion order — differing only by the `BootProfile` the WebDriver
    // probe chose. The world itself is NOT read here: the JS preload parsed it into
    // the config cache and `WorldPlugin`'s Startup systems insert it, so the plan is
    // `HostPreloaded`: boot pins the Rhai hashing seed and requests the content
    // freeze after Startup compiles the root scripts, before any spawn. The
    // `world_path`/`reader`/`script_resolver` a `HostPreloaded` plan carries are the
    // browser's genuine ones, kept for shape and future use but consulted by no boot
    // in this mode — see `WorldIngest::HostPreloaded`.
    let profile = if is_browser_gm {
        BootProfile::BrowserGameMaster
    } else if is_automation {
        BootProfile::BrowserAutomation
    } else {
        BootProfile::BrowserHost
    };
    edge::publish_active_boot_profile(match profile {
        BootProfile::BrowserGameMaster => "browser-game-master",
        BootProfile::BrowserAutomation => "browser-automation",
        BootProfile::BrowserHost => "browser-host",
        _ => unreachable!("wasm_init selects only browser profiles"),
    });
    let plan = BootPlan {
        profile,
        world_ingest: WorldIngest::HostPreloaded,
        log_filter,
        // The world path is read straight from the `SNAPSHOT_WORLD` edge stash
        // here — this is `wasm_init` building the app, itself an edge call with
        // no `World` yet. Systems that need it get the `BridgeWorldSource`
        // Resource inserted below instead (issue #1181).
        world_path: edge::read_snapshot_world()
            .map(|(path, _)| path)
            .unwrap_or_default(),
        reader: Box::new(WasmReader),
        script_resolver: Box::new(crate::entities::config_cache::production_script_resolver()),
        single_threaded: false,
        raw_transform: None,
        // Inert for a browser profile: which renderer the browser stands up is
        // decided by the target (`#[cfg(target_arch = "wasm32")]`), not by the
        // plan. The axis exists for the native host, which is both the shipped
        // target and the test target and so has to decide at runtime.
        native_surface: crate::boot::NativeRenderSurface::Contract,
    };
    // `HostPreloaded` neither reads nor validates a world, so `ingest_world` cannot
    // return `Err` for it — this `expect` documents an unreachable, not a runtime
    // failure mode the browser could actually hit.
    let mut app =
        crate::boot::build(plan).expect("browser boot composes a HostPreloaded plan infallibly");

    if is_browser_gm {
        app.insert_resource(BrowserGameMaster);
    }
    app.add_plugins((GmProjectionPlugin, GmActivityPlugin));

    app.insert_resource(log_config)
        .add_plugins(crate::logging::LoggingPlugin);
    app.add_plugins(ConfigCachePlugin)
        .add_plugins(AsteroidLifecyclePlugin)
        .add_plugins(ModifierCoordinationPlugin);
    // Insert ShipConfigResource before LobbyPlugin so its
    // .init_resource::<ShipConfigResource>() is a no-op (the default
    // calls load_ship_config_from_disk which uses std::fs — panics in WASM).
    if let Some(config) = edge::take_ship_config() {
        app.insert_resource(PendingShipConfig(config));
    };
    app.add_plugins(LobbyPlugin)
        .add_plugins(crate::lobby::lobby_outbox_broadcaster());
    // Keep simulation registration on the same renderer axis as boot. The
    // WebDriver profile has no RenderPlugin, so installing render-coupled
    // systems here would create an AssetPreloadResource that can never finish
    // and would leave every fleet start validation permanently false.
    add_simulation_plugins_with(
        &mut app,
        SimPluginOptions {
            render: !(is_automation || is_browser_gm),
            ..default()
        },
    );
    app.add_plugins(WorldPlugin);
    // Insert the selected ship resource (set by wasm_select_ship before
    // wasm_init was called).
    //
    // A hull that was actually SELECTED is inserted whatever the profile,
    // because the two browser GM routes differ on exactly this point: a GM that
    // joined somebody else's fleet calls no `wasm_select_ship` and owns no ship,
    // while a standalone GM — the landing's Host as GM route — picked a World
    // and a hull on the way in, and that hull is its own ship with every station
    // on AI backfill. Reading the selection rather than the profile is what lets
    // one boot answer both without a third flag to keep in step.
    //
    // The legacy fallback stays profile-bound: a world with no `available_ships`
    // still boots a `BrowserHost` on the shipped cruiser, but a GM that selected
    // nothing selected nothing, and inventing a cruiser for it would give a
    // joined GM a local ship it never asked for.
    if let Some(ship_path) = edge::read_selected_ship_template_path() {
        app.insert_resource(SelectedShipResource(ship_path));
    } else if !is_browser_gm {
        app.insert_resource(SelectedShipResource(
            "assets/entities/alliance_cruiser.toml".to_string(),
        ));
    }
    // The same pre-init record that selected the hull also owns the frozen
    // fleet topology. Install it before Startup spawns any GameStart ships, so
    // every slot takes the same authored spawn/component set it did at capture.
    // This is a new independent session, however, so it deliberately does not
    // recreate the old FleetLockstep wait set: this peer's local ship can be
    // claimed afresh and the other saved ships begin on AI backfill.
    // `wasm_prepare_resume` has already rejected a different selected hull.
    if let Some(boot) = edge::restore_boot_identity() {
        crate::server_app::stage_resume_game_start_entity_uuids(app.world_mut(), &boot);
        crate::lockstep::start_saved_fleet_standalone(app.world_mut(), boot.fleet);
    }
    // The renderer axis (the real viewscreen stack for the host, the surrogate's
    // `push_lobby_state` for automation) is now boot's — see `boot::render_stack`
    // and `boot::render_surrogate`.

    // Audio is plain data + JS callbacks with no wgpu dependency, so unlike the
    // viewscreen renderer it is safe to register in automation mode — and
    // registering it in both branches means the smoke tests actually exercise
    // it. The plugin registers its own bridge messages, which the PostUpdate
    // flushes need in either branch.
    app.add_plugins(crate::server::audio::ServerAudioPlugin);

    // Always add the debug overlay plugin. `?debug_regions=1` and settings-cog
    // changes both queue the canonical setter and land through
    // `drain_host_controls`; there is no second pre-init mutation export.
    app.add_plugins(crate::debug_overlay::DebugOverlayPlugin { enabled: false });

    app.insert_resource(bevy::winit::WinitSettings {
        // Keep the host simulation ticking even after Playwright opens a client
        // page in front of it; otherwise Identify stays queued and Welcome never
        // leaves the server.
        focused_mode: bevy::winit::UpdateMode::Continuous,
        unfocused_mode: bevy::winit::UpdateMode::Continuous,
    })
    .init_resource::<PendingForceStart>()
    // De-globalised bridge state (issue #1181): the durable, sim-visible half of
    // the former thread-locals lives in these Resources. `Instagib` starts off;
    // The shared startup-restore driver takes the pre-init save below.
    // `BridgeWorldSource` is
    // inserted just below, only when a world was loaded.
    .init_resource::<crate::server_app::Instagib>()
    .add_systems(
        PreUpdate,
        (
            drain_inbound,
            drain_gm_roster,
            // A leave and reopen can be queued in one animation frame. Apply
            // the world teardown/adoption first, then the new generation's
            // managed/validation edges; otherwise scheduler order could let
            // teardown erase the newly opened lobby state.
            drain_disconnects,
            drain_snapshot_requests,
            drain_host_controls
                .before(crate::debug::catalogue::refresh_readback)
                // The marker/mesh barrier is the final Time<Virtual> decision
                // before the fixed runner. A same-frame host unpause must land
                // first so it cannot reopen a clock held for authoritative rig
                // delivery (issue #1291).
                .before(crate::lockstep::MeshSet),
            drain_force_start_input,
            drain_teleport_to_waypoint,
            drain_god_mode_toggle,
            drain_instagib_toggle,
            publish_waypoint_existence,
        ),
    )
    // The fleet's ingress and generation-scoped lobby projections form one
    // explicit sequence before the barrier: adopt the pending roster first,
    // then apply only that generation's managed/validation/grant inputs.
    .add_systems(
        PreUpdate,
        (drain_mesh_inbound, drain_fleet_lobby_input)
            .chain()
            .before(crate::lockstep::MeshSet),
    )
    .add_systems(
        PreUpdate,
        // Mesh input establishes the owner's latest canonical sequence first;
        // local GM ingress then joins that order before the reducer and before
        // any fixed step. This is what makes an apply-at-now standalone Pause
        // incapable of leaking one forbidden simulation tick.
        (drain_gm_join_input, drain_gm_action_input)
            .chain()
            .after(crate::lockstep::apply_mesh_inbox)
            .before(crate::gm_action::apply_due_actions)
            .in_set(crate::lockstep::MeshSet),
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
            flush_host_channels
                .after(crate::gm_action::publish_session_projection)
                .after(crate::gm_event::publish_mission_projection)
                .after(crate::gm_spawn::publish_spawn_projection)
                .after(crate::gm_comms::publish_comms_projection)
                .after(crate::gm_activity::publish_frame_activity),
            publish_sim_tick,
            publish_god_mode,
            publish_instagib,
            publish_pause_mirror,
            publish_gm_join_status,
            // The snapshot seam (issues #862 and #865). FixedLast already
            // captured every due run at its exact logical tick; PostUpdate
            // performs only peer-local storage/export and fresh-app restore,
            // after all of this frame's fixed steps have completed.
            drain_lifecycle_saves,
            drain_snapshot_restore,
            // The fleet's egress (issue #1116). `PostUpdate` for the same
            // reason as its neighbours: it runs after the frame's fixed steps,
            // so everything those ticks sealed goes out in one batch.
            flush_mesh_outbound,
            publish_mesh_status,
            flush_start_grant_results.after(crate::gm_activity::publish_frame_activity),
        ),
    );

    // Insert the validated ShipStations resource if it was pre-validated.
    if let Some(stations) = edge::read_ship_stations() {
        app.insert_resource(stations);
    };

    // Hand the loaded world's raw `(path, TOML)` source into the World as a
    // Resource (issue #1181), so `world::server::insert_raw_world_source_resource`
    // reads it at `Startup` instead of reaching back through the bridge with a
    // free function. Inserted only when a world was actually loaded — the
    // browser always loads one before `wasm_init`, but the absent case leaves
    // the resource off exactly as the old `get_raw_world_source() == None` did.
    if let Some((path, toml)) = edge::read_snapshot_world() {
        app.insert_resource(crate::world::server::BridgeWorldSource { path, toml });
    }

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

    // A compatible record was staged before this App existed. Install the
    // lifecycle gate before `run` can execute even one fixed step, preventing
    // the fresh bootstrap's automatic saves from overwriting that record.
    if let Some(run) = edge::take_pending_restore_staged() {
        crate::startup_restore::stage(app.world_mut(), run);
    }

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
    edge::enqueue_inbound_queue((sender_token.to_string(), json.to_string()));
}

/// Called by JS with one host-mesh frame from another ship host (issue #1116),
/// tagged with the fleet slot the delivering connection was authenticated to
/// (issue #1120).
///
/// `authenticated_slot` is the `N` in the `slot-N` the page bound this connection
/// to at join — the transport-level proof of who is speaking, which the simulation
/// checks the frame's own declared `from` against at the mesh boundary
/// (`lockstep::apply_mesh_inbox`). `0` (never a real fleet slot, which start at
/// `slot-1`) means the page could not resolve it, so the frame is trusted as it
/// was before this authentication existed.
///
/// `json` is the `{ m, t, tick, d }` envelope `gui/host-mesh.js` decoded and
/// recognised as the simulation's. The page never reads the body; this is where it
/// is understood. A frame that is not one of those, or is of a revision this build
/// does not speak, is dropped here rather than guessed at, exactly as the JS
/// decoder drops one it does not recognise.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_receive_mesh_frame(authenticated_slot: u32, json: &str) {
    edge::enqueue_mesh_inbound((authenticated_slot, json.to_string()));
}

/// The fleet slot a host-mesh frame declares it came from, for the OWNER to
/// authenticate a member's frame before relaying it (issue #1120).
///
/// The frame body is opaque to `gui/host-mesh.js` — it is Rust-minted — so the page
/// cannot read the declared `from` itself. This decodes it and returns the `N` in
/// `slot-N`, or `-1` for a frame this build cannot decode. The owner compares it to
/// the slot it bound the delivering connection to at join: a mismatch is a forged
/// origin, dropped at the star centre before it can reach a sibling, which is what
/// makes the mesh-boundary authentication real for members who cannot themselves
/// re-authenticate a relayed frame.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_mesh_frame_from(json: &str) -> i32 {
    crate::core::codec::decode_mesh_frame(json)
        .map(|frame| i32::try_from(frame.from().0).unwrap_or(-1))
        .unwrap_or(-1)
}

/// Called by the OWNER page when a replacement machine has validly claimed a
/// disconnected fixed slot (issue #1120).
///
/// `slot` is the `N` in the `slot-N` being reclaimed. The page has already checked
/// — in `gui/host-mesh.js`'s `admitHost` claim path — that the slot exists, is
/// frozen (post-mission-start), is currently disconnected, and that this is the
/// FIRST claim to reach the owner for it; this call is what turns that admission
/// into the fleet-wide, deterministic `SlotClaimFrame`. Queued for the next frame,
/// where `drain_mesh_inbound` mints the owner's next `claim_seq`, stamps the
/// current tick, records it in this host's own resolver and broadcasts it.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_claim_slot(slot: u32) {
    edge::enqueue_slot_claim_queue(slot);
}

/// Everything this host wants to say to its fleet, as a JSON array of encoded
/// frames, taken and cleared.
///
/// Polled by the page's own loop rather than pushed through a callback: the
/// fleet link is a socket the page owns, and a callback would hand it a tick
/// frame at whatever instant the simulation sealed it — inside a fixed step,
/// which is the one place JS must not be re-entered from.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_take_mesh_frames() -> String {
    edge::take_mesh_frames()
}

/// Called by JS when the fleet roster freezes and the mission starts.
///
/// `roster_json` is the frozen fleet: which host flies which hull, who is
/// aboard each and at what Station Rating, and which slot is this host's. Every
/// host in the fleet receives the identical roster, which is what lets them
/// spawn identical ships with identical identities and seed identical ratings.
///
/// Returns the queued generation on success or a machine reason on immediate
/// decode refusal. Queueing is not acceptance: the page must poll
/// [`wasm_fleet_join_status`] and withhold grants until that same generation is
/// accepted by the Bevy-world drain.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_join_fleet(roster_json: &str) -> String {
    edge::join_fleet(roster_json)
}

/// Leave the currently installed fleet at the next safe Bevy-world drain.
///
/// The returned decimal generation is polled through
/// [`wasm_fleet_join_status`], exactly like a join. Calling Leave cancels any
/// not-yet-adopted join and clears old-generation edge/frame latches. A fresh
/// Lobby teardown is accepted; a mission that has started is refused with
/// `fleet-leave-not-lobby` and remains installed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_leave_fleet() -> String {
    edge::leave_fleet()
}

/// Poll the latest roster adoption attempt.
///
/// Exact JSON: `{generation,status,reason}`, with status one of
/// `idle|pending|accepted|refused` and a null reason except on refusal.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_fleet_join_status() -> String {
    edge::fleet_join_status()
}

/// Replace the crew-public Game Master roster on the next frame (issue #1289).
///
/// The host page owns the complete rendezvous projection and therefore sends a
/// complete array, never deltas. Each row is exactly
/// `{ id, name, connected, ready }`;
/// the codec rejects duplicate/unbounded rows and any private extra field.
/// Returns `""` on success or a stable machine reason on refusal.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_set_gm_roster(roster_json: &str) -> String {
    match crate::core::codec::decode_gm_roster(roster_json) {
        Some(roster) => {
            edge::publish_pending_gm_roster(Some(roster));
            String::new()
        }
        None => "gm-roster-unreadable".to_string(),
    }
}

/// Queue one typed, attributed GM action for privileged frame-driven
/// admission. The browser never receives a generic mutation route.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_submit_gm_action(request_json: &str) -> bool {
    edge::submit_gm_action(request_json)
}

/// Queue one GM paused-transfer transaction for owner sequencing (#1293/#1294).
/// A first-time request reaches this only after visible acceptance; a reconnect
/// reaches it automatically after the private capability selected an existing
/// disconnected operator.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_begin_gm_join(
    join_id: u64,
    approved_by: u32,
    candidate_host: u32,
    operator_id: &str,
    scenario: &str,
    join_kind: &str,
) -> bool {
    edge::begin_gm_join(
        join_id,
        approved_by,
        candidate_host,
        operator_id,
        scenario,
        join_kind,
    )
}

/// Prepare a candidate's world topology without admitting it to the
/// authoritative roster or lockstep wait-set. Commit is the only code path
/// which installs those resources.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_prepare_gm_join_candidate(join_id: u64, roster_json: &str) -> bool {
    edge::prepare_gm_join_candidate(join_id, roster_json)
}

/// Queue the owner's terminal answer when an accepted candidate disconnects.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_refuse_gm_join(join_id: u64, reason: &str) -> bool {
    edge::refuse_gm_join(join_id, reason)
}

/// Read-only absolute join progress for transport/public roster commit.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_gm_join_status() -> String {
    edge::gm_join_status()
}

/// Enable or disable browser-mesh ownership of collective lobby start.
/// Enabling fails closed until [`wasm_set_fleet_start_validation`] supplies the
/// current local content/peer validation result.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_set_fleet_managed_lobby(enabled: bool) -> String {
    edge::publish_latest_fleet_managed(Some(enabled));
    if queue_fleet_lobby_input(FleetLobbyInput::Managed(enabled)) {
        String::new()
    } else {
        "fleet-lobby-input-queue-full".to_string()
    }
}

/// Publish this host's independent validation gate for the next fixed-tick
/// grant. Readiness is not sent through this seam; it is ordered by the mesh
/// owner before the common grant.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_set_fleet_start_validation(valid: bool) -> String {
    edge::publish_latest_fleet_validation(Some(valid));
    if queue_fleet_lobby_input(FleetLobbyInput::Validation(valid)) {
        String::new()
    } else {
        "fleet-lobby-input-queue-full".to_string()
    }
}

/// Queue one exact host-mesh start grant for fixed-tick application.
/// Returns `""` when queued or a stable machine refusal immediately.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_apply_start_grant(grant_json: &str) -> String {
    let Some(grant) = crate::core::codec::decode_start_grant(grant_json) else {
        return "start-grant-unreadable".to_string();
    };
    if queue_fleet_lobby_input(FleetLobbyInput::Grant(grant)) {
        String::new()
    } else {
        "start-grant-queue-full".to_string()
    }
}

/// Drain one local fixed-tick grant result as JSON, or `""` when none waits.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_take_start_result() -> String {
    edge::take_start_result()
}

/// What this host's fleet link looks like from the simulation's side (issue
/// #1116), as JSON for the operator surface and the smoke tests.
///
/// ```json
/// { "in_fleet": true, "slot": 1, "tick": 412, "delay": 6,
///   "stalled": false, "stalled_frames": 0, "waiting_on": [2],
///   "peers_heard": [2], "agreed": true }
/// ```
///
/// Read-only and derived — it reports the barrier and the digest exchange, and
/// changes neither. `waiting_on` is the diagnostic that turns "the mission
/// froze" into "slot 2 is behind", which is the difference between a bug report
/// and a fix.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_mesh_status() -> String {
    edge::read_mesh_status()
}

/// Called by JS when a peer connection closes.
///
/// Queues a disconnect lifecycle event that Bevy processes next frame,
/// replacing the old workaround of dispatching a fake `ClearConsole` message.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_player_disconnected(token: &str) {
    edge::enqueue_disconnect_queue(token.to_string());
}

/// Called by JS when a peer SHIP HOST's link closes, or when a survivor relays a
/// host-loss report (issue #1119).
///
/// `slot` is the fleet slot ordinal (the `N` in `slot-N`) whose host vanished.
/// Queued for the next frame, where `drain_mesh_inbound` turns it into a
/// `HostLoss` observation: the simulation agrees the disconnect tick from that
/// host's own last watermark — the same on every survivor — and flips its ship
/// to Backfill there. Idempotent from the page's side too: reporting the same
/// slot twice, or a slot already backfilled, converges on the one transition.
///
/// Deliberately separate from [`wasm_player_disconnected`]: a crew member
/// leaving flips one station on THIS host's own ship, while a host leaving flips
/// a whole PEER ship, at an agreed tick, on every surviving host at once.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_host_departed(slot: u32) {
    edge::enqueue_host_loss_queue(slot);
}

/// Adopt a fleet the page joined, and hand the simulation everything its peers
/// have said since the last frame (issue #1116).
///
/// Runs in `PreUpdate` before `lockstep::MeshSet`, which is the same place and
/// for the same reason `drain_inbound` runs before admission: the page delivers
/// per FRAME and the simulation consumes per TICK, so the handoff has to happen
/// once, before any of the frame's fixed steps.
#[cfg(target_arch = "wasm32")]
fn clear_fleet_bridge_latches(world: &mut World) {
    edge::clear_mesh_inbound();
    edge::clear_mesh_outbound();
    edge::clear_host_loss_queue();
    edge::clear_slot_claim_queue();
    world
        .resource_mut::<crate::lockstep::SlotClaimSequence>()
        .reset();
    edge::clear_start_grant_results();
    edge::clear_mesh_status();
}

#[cfg(target_arch = "wasm32")]
fn drain_mesh_inbound(world: &mut World) {
    let bootstraps = edge::drain_pending_gm_join_bootstraps();
    for pending in bootstraps {
        if let Err(reason) =
            crate::gm_join::prepare_candidate_bootstrap(world, pending.provisional.clone())
        {
            world
                .resource_mut::<crate::gm_join::GmJoinRuntime>()
                .refuse(pending.id, reason.clone());
            world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                crate::lockstep::MeshFrame::GmJoin(crate::gm_join::GmJoinFrame::Refused {
                    from: pending.provisional.local(),
                    id: pending.id,
                    reason,
                }),
            );
        }
    }
    let adoptions = edge::take_pending_fleet_adoptions();
    for adoption in adoptions {
        let (generation, accepted, refusal) = match adoption {
            PendingFleetAdoption::Join(pending) => {
                let accepted = if let Some((roster, delay)) =
                    crate::core::codec::decode_fleet_roster(&pending.roster_json)
                {
                    // An authored delay of `None` means "whatever this world says",
                    // which is the ordinary case: the fleet agreed a mission, and the
                    // mission's `[global] command_delay_ticks` is the number.
                    let delay = delay.unwrap_or_else(|| crate::lockstep::authored_delay(world));
                    crate::lockstep::join_fleet(world, roster, delay)
                } else {
                    false
                };
                (pending.generation, accepted, "fleet-adoption-refused")
            }
            PendingFleetAdoption::Leave { generation } => {
                let accepted = crate::lockstep::leave_fleet(world).is_ok();
                if accepted {
                    clear_fleet_bridge_latches(world);
                }
                (generation, accepted, "fleet-leave-not-lobby")
            }
        };
        edge::complete_fleet_adoption(generation, accepted, refusal);
    }
    // A granted slot claim the owner admitted (issue #1120): mint the next
    // deterministic `claim_seq`, stamp the current tick, and build the fleet-wide
    // `SlotClaimFrame`. Done here — not in `wasm_claim_slot` — because it needs the
    // world's `SimTick` and this host's own slot, which a socket callback has no
    // handle to.
    let claimed = edge::take_slot_claim_queue();
    let frames = edge::take_mesh_inbound();
    // Slots whose HOST link closed on this machine. Each becomes a self-reported
    // `HostLoss` with tick 0; `apply_mesh_inbox` derives the real agreed tick
    // from the lost host's own last watermark, so the page hands over only the
    // fact of the loss, never a tick it has no way to know.
    let departed = edge::take_host_loss_queue();
    if frames.is_empty() && departed.is_empty() && claimed.is_empty() {
        return;
    }

    // The owner's own granted claims become `SlotClaimFrame`s: recorded in this
    // host's own resolver (a `LocalObservation`, authentic by construction) and
    // broadcast to the fleet through the ordinary outbox.
    if !claimed.is_empty() {
        let owner = world
            .get_resource::<crate::lockstep::FleetRoster>()
            .map(|r| r.local())
            .unwrap_or(crate::command_admission::HostSlot::SOLO);
        let tick = world
            .get_resource::<crate::sim_tick::SimTick>()
            .map_or(0, |t| t.0);
        for slot in claimed {
            let claim_seq = world
                .resource_mut::<crate::lockstep::SlotClaimSequence>()
                .next_claim();
            let frame = crate::lockstep::MeshFrame::SlotClaim(crate::lockstep::SlotClaimFrame {
                from: owner,
                slot: crate::command_admission::HostSlot(slot),
                claim_seq,
                tick,
            });
            if let Some(mut inbox) = world.get_resource_mut::<crate::lockstep::MeshInbox>() {
                inbox.push_from(frame.clone(), crate::lockstep::MeshOrigin::LocalObservation);
            }
            if let Some(mut outbox) = world.get_resource_mut::<crate::lockstep::MeshOutbox>() {
                outbox.push(frame);
            }
        }
    }

    let decoded: Vec<(crate::lockstep::MeshFrame, u32)> = frames
        .iter()
        .filter_map(|(slot, raw)| {
            crate::core::codec::decode_mesh_frame(raw).map(|frame| (frame, *slot))
        })
        .collect();
    if let Some(mut inbox) = world.get_resource_mut::<crate::lockstep::MeshInbox>() {
        // Decoded tick/digest frames BEFORE the self-reported host-loss frames,
        // so a departing host's final watermark is observed before the loss tick
        // is derived from it — the same order every survivor sees over the
        // reliable relay. `order_mesh_inbound` (issue #1119) owns and documents
        // that ordering; this authenticated path mirrors it — decoded frames
        // tagged with the slot the delivering connection was bound to (issue
        // #1120), then the local self-observations tagged `LocalObservation` —
        // rather than routing through it, because it carries no origin. Reversing
        // the order flips Backfill one tick early, which `tests/lockstep_backfill.
        // rs`'s star-topology case guards.
        for (frame, slot) in decoded {
            // `slot == 0` (never a real fleet slot) is the page saying it could
            // not authenticate the connection: trust the frame as before.
            let origin = if slot == 0 {
                crate::lockstep::MeshOrigin::Unauthenticated
            } else {
                crate::lockstep::MeshOrigin::Peer(crate::command_admission::HostSlot(slot))
            };
            inbox.push_from(frame, origin);
        }
        for slot in departed {
            inbox.push_from(
                crate::lockstep::MeshFrame::HostLoss(crate::lockstep::HostLossFrame {
                    from: crate::command_admission::HostSlot(slot),
                    lost: crate::command_admission::HostSlot(slot),
                    tick: 0,
                }),
                crate::lockstep::MeshOrigin::LocalObservation,
            );
        }
    }
}

/// Apply one full public GM-roster replacement and emit a reliable crew delta
/// only when its canonical contents actually changed.
#[cfg(any(target_arch = "wasm32", test))]
fn apply_gm_roster_replacement(
    world: &mut World,
    mut replacement: crate::gm_roster::GmRoster,
) -> bool {
    if let Some(current) = world.get_resource::<crate::gm_roster::GmRoster>() {
        replacement.clear_reconnected_readiness(current);
    }
    if world
        .get_resource::<crate::gm_roster::GmRoster>()
        .is_some_and(|current| current == &replacement)
    {
        return false;
    }

    let gms = replacement.projection();
    world.insert_resource(replacement);
    world.write_message(crate::lobby::OutboundMessage {
        target: crate::lobby::Target::All,
        msg: crate::core::messages::ServerMessage::GmRosterChanged { gms },
        delivery: crate::core::messages::DeliveryClass::Reliable,
    });
    true
}

/// Drain the validated host-page latch into the authoritative public resource.
#[cfg(target_arch = "wasm32")]
fn drain_gm_roster(world: &mut World) {
    if let Some(replacement) = edge::take_pending_gm_roster() {
        apply_gm_roster_replacement(world, replacement);
    }
}

/// Admit queued browser-GM requests only after this frame's authenticated mesh
/// input has updated the canonical sequence frontier and immediately before
/// `gm_action::apply_due_actions`. A standalone Pause/Resume at the current
/// boundary therefore takes effect before this frame can spend a fixed step.
#[cfg(target_arch = "wasm32")]
fn drain_gm_action_input(world: &mut World) {
    let requests = edge::drain_pending_gm_actions();
    for request in requests {
        // `submit_local` consumes the request, so the refusal is built from a
        // retained copy rather than from hand-picked fields: an ingress refusal
        // must carry the SAME attributed identity — operator, correlation, kind
        // and the action's stable target — that a canonical one does.
        let refused = request.clone();
        if let Err(reason) = crate::gm_action::submit_local(world, request) {
            let tick = world
                .get_resource::<crate::sim_tick::SimTick>()
                .map_or(0, |tick| tick.0);
            world
                .resource_mut::<crate::gm_action::LocalGmActionRefusals>()
                .push(crate::gm_action::LoggedGmAction::refused_request(
                    &refused, tick, reason,
                ));
        }
    }
}

/// Turn an accepted or capability-authenticated request into one deterministic
/// pause agreement.
#[cfg(target_arch = "wasm32")]
fn drain_gm_join_input(world: &mut World) {
    let refusals = edge::drain_pending_gm_join_refusals();
    for refusal in refusals {
        let _ = crate::gm_join::refuse_join(world, refusal.id, refusal.reason);
    }
    let requests = edge::drain_pending_gm_joins();
    for request in requests {
        let id = request.id;
        let result = match request.kind {
            crate::gm_join::GmJoinKind::FirstTime => crate::gm_join::begin_join(
                world,
                id,
                request.approved_by,
                request.candidate,
                request.scenario,
            ),
            crate::gm_join::GmJoinKind::Reconnect => {
                crate::gm_join::begin_reconnect(world, id, request.candidate, request.scenario)
            }
        };
        if let Err(reason) = result {
            world
                .resource_mut::<crate::gm_join::GmJoinRuntime>()
                .refuse(id, reason);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn publish_gm_join_status(runtime: Res<crate::gm_join::GmJoinRuntime>) {
    edge::publish_gm_join_status(runtime.progress().clone());
}

/// Drain the browser mesh's edge-only lobby state into typed Bevy resources.
#[cfg(target_arch = "wasm32")]
fn drain_fleet_lobby_input(
    mut managed: ResMut<FleetManagedLobby>,
    mut grants: ResMut<PendingStartGrants>,
    mut tracker: ResMut<crate::lobby::server::StartGrantTracker>,
    mut results: ResMut<StartGrantResults>,
) {
    let adoption = edge::read_fleet_join_status();
    let Some(mut inputs) = edge::take_fleet_lobby_inputs(&adoption) else {
        return;
    };
    if crate::lobby::apply_fleet_lobby_inputs(
        &mut inputs,
        &mut managed,
        &mut grants,
        &mut tracker,
        &mut results,
    ) {
        edge::clear_start_grant_results();
    }
    edge::retry_fleet_lobby_inputs(adoption.generation, inputs);
}

/// Mirror fixed-tick grant outcomes into the bounded JS-facing FIFO.
#[cfg(target_arch = "wasm32")]
fn flush_start_grant_results(mut results: ResMut<StartGrantResults>) {
    for result in results.drain() {
        if let Ok(encoded) = crate::core::codec::encode_start_grant_result(&result) {
            edge::publish_start_result(encoded);
        }
    }
}

/// Encode everything the simulation wants to say to its fleet, for the page to
/// pick up with [`wasm_take_mesh_frames`].
#[cfg(target_arch = "wasm32")]
fn flush_mesh_outbound(mut outbox: ResMut<crate::lockstep::MeshOutbox>) {
    let frames = outbox.drain();
    if frames.is_empty() {
        return;
    }
    edge::publish_mesh_frames(&frames);
}

/// Keep the fleet-status mirror honest, each frame.
#[cfg(target_arch = "wasm32")]
fn publish_mesh_status(
    session: Option<Res<crate::lockstep::FleetLockstep>>,
    roster: Option<Res<crate::lockstep::FleetRoster>>,
    diagnostics: Res<crate::lockstep::MeshDiagnostics>,
    agreement: Res<crate::lockstep::MeshAgreement>,
    delay: Res<crate::command_admission::CommandDelay>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
) {
    let tick = sim_tick.map_or(0, |t| t.0);
    let status = match session {
        Some(session) => crate::core::codec::encode_mesh_status(
            true,
            roster.map(|r| r.local().0),
            tick,
            delay.0,
            &diagnostics,
            &agreement,
            &session.peers().map(|slot| slot.0).collect::<Vec<_>>(),
        ),
        None => crate::core::codec::encode_mesh_status(
            false,
            None,
            tick,
            delay.0,
            &diagnostics,
            &agreement,
            &[],
        ),
    };
    edge::publish_mesh_status(status);
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
    edge::publish_outbound_cb(Some(callback));
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
    edge::publish_host_channel_cb(Some(callback));
}

/// Called by [`viewscreen_border::apply_camera_shake`] (WASM builds only) to
/// store the current frame's screen-shake offset for JS.
#[cfg(target_arch = "wasm32")]
pub fn set_shake_offset(x: f32, y: f32) {
    edge::publish_shake_offset((x, y));
}

/// Called by [`crate::server::audio::drive_forcefield_level`] (WASM builds
/// only) to store the current frame's forcefield SFX volume for JS.
#[cfg(target_arch = "wasm32")]
pub fn set_forcefield_level(level: f32) {
    edge::publish_forcefield_level(level);
}

/// Called by the host page to forward its reduced-motion preference
/// (`window.matchMedia('(prefers-reduced-motion: reduce)').matches`) to the
/// viewscreen renderer (issue #1173). May be called before `wasm_init()` and at
/// any time after (e.g. from the media-query `change` listener): the value is
/// drained into `ViewscreenMotion` every frame by
/// `viewscreen_border::sync_reduced_motion`, so a runtime change takes effect
/// without a reload.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_set_reduced_motion(enabled: bool) {
    edge::publish_reduced_motion(enabled);
}

/// Called by JS (or the viewscreen reduced-motion smoke) to query the
/// reduced-motion preference the host page last forwarded — the observable
/// proof that the profile value reached the WASM render path (issue #1173).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_is_reduced_motion() -> bool {
    edge::read_reduced_motion()
}

/// Read the current reduced-motion request for `sync_reduced_motion` to drain
/// into the `ViewscreenMotion` resource each frame (issue #1173).
#[cfg(target_arch = "wasm32")]
pub(crate) fn reduced_motion_requested() -> bool {
    edge::read_reduced_motion()
}

/// Native: the host's reduced-motion preference, read from the
/// `PHOENIX_REDUCED_MOTION` environment variable (issue #1173). The desktop
/// viewscreen has no DOM `prefers-reduced-motion`, so this env var is the native
/// analog of the WASM host page forwarding the browser preference — it seeds
/// `ViewscreenMotion` once at startup via
/// `viewscreen_border::init_native_reduced_motion`. Enabled by any of
/// `1`/`true`/`yes`/`on`/`reduce` (case-insensitive); unset or anything else
/// leaves normal motion in place.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn native_reduced_motion() -> bool {
    std::env::var("PHOENIX_REDUCED_MOTION")
        .map(|val| {
            matches!(
                val.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on" | "reduce"
            )
        })
        .unwrap_or(false)
}

/// Called by JS to set the log category/level spec from `?log=` in the URL.
/// Must be called before `wasm_init()` to take effect.
///
/// Same syntax as the headless runner's `--log`, e.g.
/// `?log=info,ai=debug,admit=trace`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_set_log_spec(spec: &str) {
    edge::publish_log_spec(Some(spec.to_string()));
}

/// Called by JS to restrict logging to named entities, from `?log_entity=`.
/// Must be called before `wasm_init()` to take effect.
///
/// Comma-separated display names, matched exactly then case-insensitively as a
/// substring — e.g. `?log_entity=Ironveil,Ashrender`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_set_log_entity(names: &str) {
    edge::publish_log_entity(Some(names.to_string()));
}

/// Build the [`LogFilterConfig`] for this page from the `?log=` / `?log_entity=`
/// thread-locals. Also returns the raw spec so `wasm_init` can hand it to
/// `LogPlugin`'s own `EnvFilter`.
///
/// A malformed spec warns and falls back to the default rather than aborting
/// startup — a typo in a debug URL parameter should not stop the game booting.
#[cfg(target_arch = "wasm32")]
fn log_config_from_url() -> (crate::logging::LogFilterConfig, String) {
    let spec = edge::read_log_spec().unwrap_or_default();
    let mut config = match crate::logging::parse_log_spec(&spec) {
        Ok(cfg) => cfg,
        Err(e) => {
            bevy::log::warn!("?log= is malformed ({e}); ignoring it");
            crate::logging::LogFilterConfig::default()
        }
    };
    if let Some(names) = edge::read_log_entity() {
        config.entity_filter = crate::logging::parse_log_entities(&names);
    }
    (config, spec)
}

// ── The snapshot seam (issue #862) ─────────────────────────────────────────
//
// The exports and systems here are shaped by what browser save and resume
// actually mean.
//
// **Saving** is two-stage: `PreUpdate` admits browser requests to the lifecycle
// resource, `FixedLast` captures every due run at its exact logical tick, and
// `PostUpdate` hands the resulting RON to `vellum_save::Store`. The storage
// outcome is presentation-only and never enters the simulation digest or mesh.
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

/// Queue one peer-local fixed-tick capture and retain its browser-only storage
/// intent until the captured run reaches `PostUpdate`.
#[cfg(target_arch = "wasm32")]
fn queue_browser_save(intent: BrowserSaveIntent) -> Option<String> {
    edge::queue_browser_save(intent)
}

/// Queue a save of the running session into `slot`.
///
/// Returns immediately; the capture happens on the next fixed-tick boundary,
/// storage drains afterward in `PostUpdate`, and the outcome is read back
/// through [`wasm_snapshot_status`].
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_save_snapshot(slot: String) {
    let _ = queue_browser_save(BrowserSaveIntent::LegacySlot(slot));
}

/// Queue an EXPORT of the running session (issue #866).
///
/// The same queue, the same capture and the same tick boundary as
/// [`wasm_save_snapshot`]; only the destination differs. The resulting RON is
/// collected by [`wasm_take_exported_snapshot`] once the capture has been taken.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_export_snapshot() {
    let _ = queue_browser_save(BrowserSaveIntent::ExportCurrent);
}

/// Request a named manual catalogue save at this peer's next fixed boundary.
/// Returns its stable internal slot id immediately; the status poll reports the
/// later Store outcome. Returns `""` and queues a local failure status when the
/// finite browser request queue is full.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_create_save_slot(display_name: String) -> String {
    let slot_id = crate::save_slots::new_manual_slot_id();
    let accepted = queue_browser_save(BrowserSaveIntent::CreateManual {
        slot_id: slot_id.clone(),
        display_name,
    });
    accepted.map_or_else(String::new, |_| slot_id)
}

/// Take the exported save's text, if one is waiting.
///
/// Returns `""` when there is nothing to collect. Taken rather than read, so a
/// host page polling this cannot download the same save twice.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_take_exported_snapshot() -> String {
    edge::take_exported_snapshot()
}

/// The file name a host is offered for an exported save.
///
/// Published rather than spelled in JS so the extension and the Rust constant
/// that explains it (`snapshot::EXPORT_FILE_NAME`) cannot drift apart.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_export_file_name() -> String {
    crate::snapshot::EXPORT_FILE_NAME.to_string()
}

/// Read this browser's private save catalogue. Objects are assembled through
/// `js_sys`; save metadata never takes the crate's JSON codec exception.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_list_save_slots() -> Result<Array, JsValue> {
    let store = browser_save_store();
    let current = crate::snapshot::versions(&crate::content_ledger::frozen_or_live());
    let mut entries = crate::save_slots::list_slots_with_content_check(
        &store,
        &current,
        crate::save_slots::ContentCheck::Full,
    )
    .map_err(|error| JsValue::from_str(&format!("{error:?}")))?;
    let loaded_scenario = if crate::content_ledger::is_frozen() {
        edge::snapshot_scenario()
    } else {
        None
    };
    defer_unloaded_scenario_content(&mut entries, loaded_scenario.as_deref());
    let rows = Array::new();
    for entry in entries {
        rows.push(&save_slot_js(entry));
    }
    Ok(rows)
}

/// Apply content compatibility only to the scenario whose ledger is loaded and
/// frozen. Format/rules failures and damaged Store rows remain hard failures;
/// another scenario's content is unknown until a fresh boot loads that row.
#[cfg(any(target_arch = "wasm32", test))]
fn defer_unloaded_scenario_content(
    entries: &mut [crate::save_slots::SaveSlotEntry],
    loaded_scenario: Option<&str>,
) {
    use crate::save_slots::StartState;

    for entry in entries {
        let content_is_checkable = loaded_scenario.is_some_and(|scenario| {
            entry
                .record
                .as_ref()
                .is_some_and(|record| record.scenario == scenario)
        });
        if content_is_checkable {
            continue;
        }

        if matches!(
            &entry.start,
            StartState::Ready
                | StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                    vellum_save::Moved::Content { .. }
                ))
        ) {
            entry.start = StartState::ContentDeferred;
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn save_slot_js(entry: crate::save_slots::SaveSlotEntry) -> Object {
    use crate::save_slots::{MetadataStatus, SaveSlotKind};

    let row = Object::new();
    save_js_field(&row, "slot_id", &JsValue::from_str(&entry.slot_id));
    save_js_field(
        &row,
        "kind",
        &JsValue::from_str(match entry.kind {
            SaveSlotKind::Autosave => "autosave",
            SaveSlotKind::Manual => "manual",
        }),
    );
    save_js_field(
        &row,
        "display_name",
        &JsValue::from_str(&entry.display_name),
    );
    let (metadata, metadata_error) = match entry.metadata {
        MetadataStatus::NotApplicable => ("none", None),
        MetadataStatus::Present => ("present", None),
        MetadataStatus::Missing => ("missing", None),
        MetadataStatus::Corrupt => ("corrupt", None),
        MetadataStatus::Unreadable(error) => ("unreadable", Some(error)),
    };
    save_js_field(&row, "metadata", &JsValue::from_str(metadata));
    save_js_field(
        &row,
        "metadata_error",
        &metadata_error.map_or(JsValue::NULL, |error| JsValue::from_str(&error)),
    );

    match entry.record {
        Some(record) => {
            save_js_field(&row, "scenario", &JsValue::from_str(&record.scenario));
            save_js_field(
                &row,
                "selected_ship",
                &record
                    .boot_identity
                    .as_ref()
                    .map_or(JsValue::NULL, |boot| JsValue::from_str(&boot.selected_ship)),
            );
            save_js_field(&row, "seed", &JsValue::from_str(&record.seed.to_string()));
            save_js_field(
                &row,
                "capture_tick",
                &JsValue::from_str(&record.capture_tick.to_string()),
            );
            let versions = Object::new();
            save_js_field(
                &versions,
                "format",
                &JsValue::from_f64(f64::from(record.versions.format)),
            );
            save_js_field(
                &versions,
                "rules",
                &JsValue::from_str(&record.versions.rules),
            );
            save_js_field(
                &versions,
                "content",
                &JsValue::from_str(&format!("{:016x}", record.versions.content)),
            );
            save_js_field(&row, "versions", &versions);
        }
        None => {
            for field in [
                "scenario",
                "selected_ship",
                "seed",
                "capture_tick",
                "versions",
            ] {
                save_js_field(&row, field, &JsValue::NULL);
            }
        }
    }

    let start = save_slot_start_projection(&entry.start);
    save_js_field(&row, "compatible", &JsValue::from_bool(start.compatible));
    save_js_field(&row, "startable", &JsValue::from_bool(start.startable));
    save_js_field(
        &row,
        "refusal_kind",
        &start.refusal_kind.map_or(JsValue::NULL, JsValue::from_str),
    );
    save_js_field(
        &row,
        "refusal",
        &start
            .refusal
            .as_deref()
            .map_or(JsValue::NULL, JsValue::from_str),
    );
    row
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct SaveSlotStartProjection {
    compatible: bool,
    startable: bool,
    refusal_kind: Option<&'static str>,
    refusal: Option<String>,
}

/// Pure half of the wasm-bindgen row projection. Kept outside JS object
/// construction so native tests prove deferred content is startable without
/// weakening any real refusal.
#[cfg(any(target_arch = "wasm32", test))]
fn save_slot_start_projection(start: &crate::save_slots::StartState) -> SaveSlotStartProjection {
    use crate::save_slots::StartState;

    match start {
        StartState::Ready => SaveSlotStartProjection {
            compatible: true,
            startable: true,
            refusal_kind: None,
            refusal: None,
        },
        StartState::ContentDeferred => SaveSlotStartProjection {
            compatible: false,
            startable: true,
            refusal_kind: Some("content-pending"),
            refusal: None,
        },
        StartState::Refused(refusal) => {
            let kind = match refusal {
                crate::snapshot::LoadRefusal::Empty => "empty",
                crate::snapshot::LoadRefusal::Unreadable(_) => "unreadable",
                crate::snapshot::LoadRefusal::Unparsable(_) => "unparsable",
                crate::snapshot::LoadRefusal::Moved(vellum_save::Moved::Format { .. }) => "format",
                crate::snapshot::LoadRefusal::Moved(vellum_save::Moved::Rules { .. }) => "rules",
                crate::snapshot::LoadRefusal::Moved(vellum_save::Moved::Content { .. }) => {
                    "content"
                }
            };
            SaveSlotStartProjection {
                compatible: false,
                startable: false,
                refusal_kind: Some(kind),
                refusal: Some(refusal.to_string()),
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn save_js_field(object: &Object, field: &str, value: &JsValue) {
    Reflect::set(object, &JsValue::from_str(field), value).ok();
}

/// Rename only a manual slot's sidecar. Returns empty on success.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_rename_save_slot(slot_id: String, display_name: String) -> String {
    let store = browser_save_store();
    crate::save_slots::rename_slot(&store, &slot_id, display_name)
        .err()
        .map_or_else(String::new, |error| format!("{error:?}"))
}

/// Export an existing selected slot through #866's one-shot artifact getter.
/// Compatibility is deliberately not a copy gate; only starting is gated.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_export_save_slot(slot_id: String) -> String {
    let store = browser_save_store();
    match crate::save_slots::export_slot(&store, &slot_id) {
        Ok(text) => {
            edge::publish_exported_artifact(Some(text));
            String::new()
        }
        Err(refusal) => refusal.to_string(),
    }
}

/// Confirmation-bearing deletion backend. Passing `false` performs no Store
/// mutation and returns a stable code for the later UI adapter to localise.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_delete_save_slot(slot_id: String, confirmed: bool) -> String {
    if !confirmed {
        return "confirmation-required".to_string();
    }
    let store = browser_save_store();
    crate::save_slots::delete_slot(&store, &slot_id)
        .err()
        .map_or_else(String::new, |error| format!("{error:?}"))
}

/// Stage a compatible local slot for the next fresh app boot. This is an alias
/// of the established pre-init resume gate; it never receives a `World`, so a
/// running session cannot be restored through it.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_stage_save_slot(slot_id: String) -> String {
    wasm_prepare_resume(slot_id)
}

/// Which button an outcome answers. Carried so the host page can put the
/// answer back on the control that was pressed rather than guessing from the
/// wording of a sentence it is not allowed to paraphrase.
#[cfg(target_arch = "wasm32")]
const SNAPSHOT_SAVE: &str = "save";
#[cfg(target_arch = "wasm32")]
const SNAPSHOT_RESUME: &str = "resume";
/// The export half of issue #866. Its own label rather than `save`'s, because
/// the two controls sit in different places and an answer belongs on the one
/// that was pressed.
#[cfg(target_arch = "wasm32")]
const SNAPSHOT_EXPORT: &str = "export";

/// Record a host-visible outcome for the next [`wasm_snapshot_status`] poll.
#[cfg(target_arch = "wasm32")]
fn set_snapshot_status(ok: bool, source: &str, message: impl Into<String>) {
    edge::set_snapshot_status(ok, source, message)
}

/// Take the oldest retained host-visible save or resume outcome. Each retained
/// outcome is reported exactly once; if an inactive poller fills the finite
/// outbox, the oldest status is replaced so the newest local refusal stays
/// visible.
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
    edge::snapshot_status()
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
    if !crate::entities::config_cache::wasm_is_preload_complete() {
        return "scenario content preload is incomplete".to_string();
    }
    let Some((path, toml)) = edge::read_snapshot_world() else {
        // No world means no content digest, so there is nothing to check the
        // save against. Refusing beats guessing.
        return "the scenario has not been loaded yet".to_string();
    };
    let Some(world_config) = crate::entities::config_cache::get_world_config() else {
        return "the scenario has not been loaded yet".to_string();
    };
    let store = browser_save_store();
    let versions = match browser_resume_versions(
        &path,
        &toml,
        &crate::entities::config_cache::production_script_resolver(),
    ) {
        Ok(versions) => versions,
        Err(refusal) => return refusal,
    };
    let selected_ship = edge::read_selected_ship_template_path()
        .unwrap_or_else(|| "assets/entities/alliance_cruiser.toml".to_string());
    match load_resume_after_scenario(&store, &slot, &versions, &selected_ship, &world_config) {
        Ok(run) => {
            // Stash pre-init; `wasm_init` hands this off to the shared restore
            // driver with a fresh patience budget, and the mirror
            // makes `wasm_resume_pending()` answer true until the drain clears it
            // (issue #1181).
            edge::publish_pending_restore_staged(Some(run));
            edge::publish_resume_pending_mirror(true);
            edge::clear_snapshot_status();
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

/// Complete the root's declared content before the browser's early full version
/// gate. A running capture includes Startup's script record; pre-init has only
/// loaded the world/templates so far. Lift the same sources and declare their
/// record without compiling or executing them, leaving Startup's validation and
/// freeze intact. The production resolver also records exact sibling/overlay
/// bodies, so take the ledger snapshot only after resolving them.
#[cfg(any(target_arch = "wasm32", test))]
fn browser_resume_versions(
    path: &str,
    world_toml: &str,
    resolver: &dyn crate::world::script::load::ScriptResolver,
) -> Result<vellum_save::Versions, String> {
    if !crate::content_ledger::is_frozen() {
        let raw: toml::Value =
            toml::from_str(world_toml).map_err(|error: toml::de::Error| error.to_string())?;
        let (sources, findings) =
            crate::world::script::load::lift_world_scripts(path, &raw, resolver);
        if crate::world::validate::has_error(&findings) {
            let messages: Vec<_> = findings
                .iter()
                .filter(|finding| finding.is_error())
                .map(|finding| finding.message.as_str())
                .collect();
            return Err(messages.join("; "));
        }
        if let Some(digest) =
            crate::world::script::load::script_source_ledger_digest(path, &sources)
        {
            digest.apply();
        }
    }
    Ok(crate::snapshot::versions(
        &crate::content_ledger::frozen_or_live(),
    ))
}

/// Read, parse and fully version-gate a selected row after that row's scenario
/// has loaded, before the run can enter the staged restore hand-off.
#[cfg(any(target_arch = "wasm32", test))]
fn load_resume_after_scenario<S: vellum_save::Store>(
    store: &S,
    slot: &str,
    current: &vellum_save::Versions,
    selected_ship: &str,
    world_config: &crate::world::config::WorldConfig,
) -> Result<crate::snapshot::StoredRun, BrowserResumeRefusal> {
    let run =
        crate::snapshot::load_from(store, slot, current).map_err(BrowserResumeRefusal::Load)?;
    validate_browser_resume_boot(&run, selected_ship, world_config)?;
    Ok(run)
}

/// Apply the boot-shape gate shared by browser catalogue resume and portable
/// import once the selected scenario and hull are known.
#[cfg(any(target_arch = "wasm32", test))]
fn validate_browser_resume_boot(
    run: &crate::snapshot::StoredRun,
    selected_ship: &str,
    world_config: &crate::world::config::WorldConfig,
) -> Result<(), BrowserResumeRefusal> {
    let required =
        crate::snapshot::required_boot_identity(run).map_err(BrowserResumeRefusal::Load)?;
    if required.selected_ship != selected_ship {
        return Err(BrowserResumeRefusal::WrongSelectedShip {
            saved: required.selected_ship.clone(),
            loaded: selected_ship.to_string(),
        });
    }
    crate::snapshot::validate_boot_identity_for_world(required, world_config)
        .map_err(BrowserResumeRefusal::Load)
}

/// Parse a portable artifact and apply the same post-scenario boot gate as a
/// catalogue resume. Keeping this pure makes it impossible for the import edge
/// to bypass the selected-hull or authored-GameStart identity checks.
#[cfg(any(target_arch = "wasm32", test))]
fn import_resume_after_scenario(
    text: &str,
    current: &vellum_save::Versions,
    selected_ship: &str,
    world_config: &crate::world::config::WorldConfig,
) -> Result<crate::snapshot::StoredRun, BrowserResumeRefusal> {
    let run =
        crate::snapshot::import_artifact(text, current).map_err(BrowserResumeRefusal::Load)?;
    validate_browser_resume_boot(&run, selected_ship, world_config)?;
    Ok(run)
}

/// Enter a portable save's text into a catalogue as a new manual slot
/// (issue #1363).
///
/// Pure over the `Store`, for the reason [`import_resume_after_scenario`] is
/// pure over its gate: the one rule this path must not break — a file that is
/// not a `Run` never becomes a row — is then a test rather than a claim about
/// an edge no native test can call.
///
/// **Compatibility is deliberately not an entry gate**, which is
/// [`wasm_export_save_slot`]'s rule read in the other direction: copying a save
/// in and copying one out are both ungated, and only STARTING is gated. It is
/// also the only rule that could be applied here. The version gate's content
/// dimension is a digest over the world a save names, so it has nothing to
/// check against until that world is loaded — and a host standing at the
/// catalogue has loaded none. Gating here would refuse every save from a world
/// this page has not booted, which is most of them.
///
/// Nothing is lost by that. `wasm_list_save_slots` runs the compatibility check
/// that puts a refusal on the row (#1363's AC3), and a Start reloads into
/// [`wasm_prepare_resume`], where the gate runs before anything is restored
/// (AC4). An imported save is a slot like any other from the moment it is
/// written, which is the whole of "enters an imported save into the same list"
/// (AC2).
#[cfg(any(target_arch = "wasm32", test))]
fn import_artifact_into_catalogue<S: vellum_save::Store>(
    store: &S,
    text: &str,
    display_name: &str,
) -> Result<String, ImportSlotRefusal> {
    let run = crate::snapshot::StoredRun::from_ron(text).map_err(|error| {
        ImportSlotRefusal::Damaged(crate::snapshot::LoadRefusal::Unparsable(error.to_string()))
    })?;
    crate::save_slots::create_manual_save(store, display_name, &run)
        .map_err(ImportSlotRefusal::NotStored)
}

/// Why an imported file did not become a catalogue row.
///
/// Two classes, and they are two for [`wasm_peek_import`]'s reason: they send a
/// host to different places. A damaged file means pick another one; a Store
/// that would not take it means make room, and the file is fine.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum ImportSlotRefusal {
    Damaged(crate::snapshot::LoadRefusal),
    NotStored(crate::save_slots::CatalogueError),
}

#[cfg(any(target_arch = "wasm32", test))]
impl std::fmt::Display for ImportSlotRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // The same wire shape every other transfer answer uses: the class,
            // a tab, and a sentence the page is not allowed to paraphrase.
            Self::Damaged(refusal) => write!(formatter, "damaged\t{refusal}"),
            Self::NotStored(error) => write!(formatter, "not-stored\t{error:?}"),
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum BrowserResumeRefusal {
    Load(crate::snapshot::LoadRefusal),
    WrongSelectedShip { saved: String, loaded: String },
}

#[cfg(any(target_arch = "wasm32", test))]
impl std::fmt::Display for BrowserResumeRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load(refusal) => write!(formatter, "{refusal}"),
            Self::WrongSelectedShip { saved, loaded } => write!(
                formatter,
                "the save requires hull {saved:?}, but this new session loaded {loaded:?}"
            ),
        }
    }
}

/// Whether a save is staged and waiting for the world to finish bootstrapping.
///
/// Reads the `RESUME_PENDING_MIRROR` edge cache (issue #1181): once `wasm_init`
/// has handed the staged save to the shared restore driver, this
/// `World`-less getter can no longer read the Resource directly, so
/// `drain_snapshot_restore` mirrors its presence out each frame — true while the
/// save waits, false the moment it is applied or abandoned.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_resume_pending() -> bool {
    edge::read_resume_pending_mirror()
}

/// Which scenario an imported file belongs to, BEFORE any world is loaded on its
/// behalf (issue #866).
///
/// Returns `"ok\t<scenario path>"`, or `"damaged\t<why>"` for a file that is not
/// a save this build can parse at all. Tab-separated for
/// [`wasm_snapshot_status`]'s reason: one string is the cheapest thing that
/// crosses this boundary, and the host page needs the CLASS as well as the
/// sentence — a damaged file and an incompatible one send a host to two
/// different places.
///
/// Only the damaged class can be answered here, and that is the point of having
/// two calls rather than one. The version gate needs a content digest, a content
/// digest needs a loaded world, and which world to load is written inside the
/// file — so parsing has to come first and the gate second. Splitting them means
/// a damaged file is refused before this page loads a scenario on its behalf,
/// rather than after.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_peek_import(text: String) -> String {
    match crate::snapshot::peek_artifact_scenario(&text) {
        Ok(scenario) => format!("ok\t{scenario}"),
        Err(refusal) => format!("damaged\t{refusal}"),
    }
}

/// Enter an imported file into this browser's own save catalogue, as a new
/// manual slot named `display_name` (issue #1363's AC2).
///
/// Returns `""` when the row was written, or `"<class>\t<message>"` when it was
/// not — `damaged` for a file that is not a `Run` this build can parse, and
/// `not-stored` for a Store that would not take it. See
/// [`import_artifact_into_catalogue`] for why there is no third class, and in
/// particular why compatibility is not one.
///
/// This is the half of #866's import that #1363 adds: the importer moved into
/// the catalogue's header, so importing is now an action ON the catalogue, and
/// an action on a list that does not change the list would be a control sitting
/// somewhere it does not belong. The staged direct boot below is unchanged and
/// still runs after this — the file both joins the list and starts, rather than
/// only starting.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_import_save_slot(text: String, display_name: String) -> String {
    match import_artifact_into_catalogue(&browser_save_store(), &text, &display_name) {
        Ok(_) => String::new(),
        Err(refusal) => refusal.to_string(),
    }
}

/// Put an imported file through the version gate and stage it for the boot that
/// is about to happen (issue #866). Call BEFORE `wasm_init`, exactly where
/// [`wasm_prepare_resume`] is called.
///
/// Returns `""` when the file was accepted and is now pending, or
/// `"<class>\t<message>"` when it was not. The two classes are the two AC5
/// answers and they are deliberately not one:
///
/// * `damaged` — the file is not a `Run` this build can parse. Truncated,
///   hand-edited, or never a save. The host should pick another file.
/// * `incompatible` — the file is intact and this build cannot honour it. The
///   message is `vellum_save::Moved`'s own sentence, verbatim, because it names
///   WHICH dimension moved and to what, and phoenix has no vocabulary that would
///   say more.
///
/// A refusal stages nothing, so the page boots a normal new session — the same
/// promise [`wasm_prepare_resume`] makes, for the same reason.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_prepare_import(text: String) -> String {
    if !crate::entities::config_cache::wasm_is_preload_complete() {
        return "incompatible\tscenario content preload is incomplete".to_string();
    }
    let Some((path, toml)) = edge::read_snapshot_world() else {
        // Same guard as `wasm_prepare_resume`: no world means no content digest,
        // so there is nothing to check the save against.
        return format!("damaged\t{}", "the scenario has not been loaded yet");
    };
    let Some(world_config) = crate::entities::config_cache::get_world_config() else {
        return format!("damaged\t{}", "the scenario has not been loaded yet");
    };
    let versions = match browser_resume_versions(
        &path,
        &toml,
        &crate::entities::config_cache::production_script_resolver(),
    ) {
        Ok(versions) => versions,
        Err(refusal) => return format!("incompatible\t{refusal}"),
    };
    let selected_ship = edge::read_selected_ship_template_path()
        .unwrap_or_else(|| "assets/entities/alliance_cruiser.toml".to_string());
    match import_resume_after_scenario(&text, &versions, &selected_ship, &world_config) {
        Ok(run) => {
            // Same pre-init hand-off as `wasm_prepare_resume` (issue #1181).
            edge::publish_pending_restore_staged(Some(run));
            edge::publish_resume_pending_mirror(true);
            edge::clear_snapshot_status();
            String::new()
        }
        // The classification is `LoadRefusal`'s own, not a re-reading of the
        // message: `Unparsable` IS "the file is damaged" and `Moved` IS "this
        // build cannot honour it", and keeping the match here means the host
        // page never has to infer a class from a sentence it is not allowed to
        // paraphrase.
        Err(BrowserResumeRefusal::Load(crate::snapshot::LoadRefusal::Moved(moved))) => {
            format!("incompatible\t{moved}")
        }
        Err(refusal @ BrowserResumeRefusal::WrongSelectedShip { .. }) => {
            format!("incompatible\t{refusal}")
        }
        Err(BrowserResumeRefusal::Load(other)) => format!("damaged\t{other}"),
    }
}

/// Move every JS save click into the lifecycle resource before the fixed loop.
/// Requests made in one frame remain distinct and all target this peer's next
/// `FixedLast`; an invalid phase refuses them without taking a snapshot.
#[cfg(target_arch = "wasm32")]
fn drain_snapshot_requests(world: &mut World) {
    let requests = edge::take_save_requests();
    if requests.is_empty() {
        return;
    }
    let may_capture = world
        .get_resource::<State<messages::GamePhase>>()
        .is_some_and(|phase| {
            matches!(
                phase.get(),
                messages::GamePhase::InProgress | messages::GamePhase::GameOver
            )
        });
    for token in requests {
        if may_capture {
            crate::save_slots_lifecycle::request_manual_save(world, token);
            continue;
        }
        let intent = edge::complete_save_intent(&token);
        let source = intent.as_ref().map_or(SNAPSHOT_SAVE, intent_source);
        set_snapshot_status(false, source, "there is no run in progress to save");
    }
}

#[cfg(target_arch = "wasm32")]
fn intent_source(intent: &BrowserSaveIntent) -> &'static str {
    match intent {
        BrowserSaveIntent::ExportCurrent => SNAPSHOT_EXPORT,
        BrowserSaveIntent::LegacySlot(_) | BrowserSaveIntent::CreateManual { .. } => SNAPSHOT_SAVE,
    }
}

/// Drain every fixed-tick capture into this browser's private Store. Storage is
/// downstream of the sim: a quota/backend failure records a local status and
/// cannot change the tick, digest, or another peer.
#[cfg(target_arch = "wasm32")]
fn drain_lifecycle_saves(world: &mut World) {
    // A manual request can be consumed without capture when its deterministic
    // boundary crosses out of InProgress or while startup restore is staged.
    // Clear the matching browser-only intent and report it once; an absent
    // intent means an earlier edge refusal already resolved it, so do not emit a
    // duplicate status.
    loop {
        let refusal = world
            .get_resource_mut::<crate::save_slots_lifecycle::RefusedManualSaves>()
            .and_then(|mut refusals| refusals.pop_front());
        let Some(refusal) = refusal else {
            break;
        };
        let intent = edge::complete_save_intent(&refusal.slot_id);
        let Some(intent) = intent else {
            continue;
        };
        let source = intent_source(&intent);
        let message = match refusal.reason {
            crate::save_slots::ManualSaveRefusalReason::PhaseChanged { .. } => {
                "there is no run in progress to save"
            }
            crate::save_slots::ManualSaveRefusalReason::StartupRestorePending => {
                "the save request was cancelled while a local session restore was starting"
            }
        };
        set_snapshot_status(false, source, message);
    }

    loop {
        let pending = world
            .get_resource_mut::<crate::save_slots_lifecycle::PendingStoredRuns>()
            .and_then(|mut runs| runs.pop_front());
        let Some(pending) = pending else {
            return;
        };
        let tick = pending.run.ledger.final_tick;
        let store = browser_save_store();
        match pending.decision.slot {
            crate::save_slots::CaptureSlot::RollingAutosave => {
                if let Err(error) = crate::save_slots::write_autosave(&store, &pending.run) {
                    set_snapshot_status(
                        false,
                        SNAPSHOT_SAVE,
                        format!("the save could not be written: {error:?}"),
                    );
                }
            }
            crate::save_slots::CaptureSlot::Manual(token) => {
                let intent = edge::complete_save_intent(&token);
                let intent = intent.unwrap_or(BrowserSaveIntent::LegacySlot(token));
                let source = intent_source(&intent);
                let written = match intent {
                    BrowserSaveIntent::LegacySlot(slot)
                        if slot == crate::save_slots::AUTOSAVE_SLOT =>
                    {
                        crate::save_slots::write_autosave(&store, &pending.run)
                            .map_err(|error| format!("{error:?}"))
                    }
                    BrowserSaveIntent::LegacySlot(slot) => {
                        crate::snapshot::save_to(&store, &slot, &pending.run)
                    }
                    BrowserSaveIntent::CreateManual {
                        slot_id,
                        display_name,
                    } => crate::save_slots::write_manual_save(
                        &store,
                        &slot_id,
                        display_name,
                        &pending.run,
                    )
                    .map_err(|error| format!("{error:?}")),
                    BrowserSaveIntent::ExportCurrent => {
                        crate::snapshot::export_artifact(&pending.run).map(|text| {
                            edge::publish_exported_artifact(Some(text));
                        })
                    }
                };
                match written {
                    Ok(()) => {
                        set_snapshot_status(true, source, format!("saved at tick {tick}"));
                    }
                    Err(error) => set_snapshot_status(
                        false,
                        source,
                        format!("the save could not be written: {error}"),
                    ),
                }
            }
        }
    }
}

/// Mirror and report the shared driver's result at the browser edge. All
/// reconciliation, patience, verification and capture cleanup live in the
/// cross-target adapter; JS retains only its pending/readback transport.
#[cfg(target_arch = "wasm32")]
fn drain_snapshot_restore(world: &mut World) {
    use crate::startup_restore::{RestoreFailure, RestoreOutcome};

    let outcome = crate::startup_restore::advance(world);
    edge::publish_resume_pending_mirror(crate::startup_restore::is_pending(world));
    let Some(outcome) = outcome else {
        return;
    };
    let (ok, message) = match outcome {
        RestoreOutcome::Applied { tick } => (true, format!("resumed at tick {tick}")),
        RestoreOutcome::Failed(failure) => (false, match failure {
            RestoreFailure::NoSnapshot => {
                "that save carries no captured state to resume from".to_string()
            }
            RestoreFailure::LayerFailed { path } => format!(
                "the save requires world layer '{path}', but that layer could not be reconstructed"
            ),
            RestoreFailure::NotReady { tick, entities } => format!(
                "this session never built the world that save was taken in (the save wanted {entities} ship(s) at tick {tick})"
            ),
            RestoreFailure::DigestMismatch { expected, actual } => format!(
                "the save did not restore cleanly (recorded {expected:016x}, restored {actual:016x})"
            ),
            RestoreFailure::Incomplete { tick, gaps } => {
                format!("resumed at tick {tick} with {gaps} missing entities")
            }
        }),
    };
    set_snapshot_status(ok, SNAPSHOT_RESUME, message);
}

/// Set one host diagnostic surface by its catalogue-owned wire name.
///
/// This is the only host diagnostic mutation export. It carries an absolute
/// state derived from the authoritative readback, so a phone and the host
/// cannot race a relative local toggle into the opposite value. Unknown names
/// are rejected without queuing anything.
///
/// Absent from a public-demo binary. Readback exports remain available there.
#[cfg(all(target_arch = "wasm32", not(phoenix_demo_build)))]
#[wasm_bindgen]
pub fn wasm_set_debug_surface(wire_name: String, enabled: bool) -> bool {
    let Some(surface) = DebugSurface::from_wire_name(&wire_name) else {
        return false;
    };
    edge::request_debug_surface(surface, enabled);
    true
}

/// Called by JS to pause/unpause the simulation clock.
///
/// Sets a pending flag that is consumed by `drain_host_controls` in the next
/// `PreUpdate` frame, which pauses or unpauses `Time<Virtual>`.
///
/// Named without `debug` (issue #939) because its only caller is the host
/// settings menu's **Gameplay** tab, which ships in the demo build where the
/// Debug/Cheat tab is gone. Nothing on this path is gated by
/// `PHOENIX_DEMO_BUILD`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_toggle_pause() {
    edge::publish_pending_pause(true);
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
    edge::read_sim_paused()
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

/// Called by JS each animation frame to read the latest modifier debug payload
/// as JSON while the surface is visible (issue #1150). The dock parses it and
/// renders the modifier sections; empty until the first publish.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_debug_state() -> String {
    edge::read_debug_state_string()
}

/// Called by the Bevy `debug::modifiers::publish_modifier_debug` system to update
/// the modifier debug JSON that JS reads via `wasm_get_debug_state()`.
#[cfg(target_arch = "wasm32")]
pub fn set_debug_state_string(text: String) {
    edge::publish_debug_state_string(text);
}

/// Called by JS each animation frame to read the latest damage-log payload as
/// JSON while the surface is visible (issue #1150). The dock parses and renders
/// it; empty until the first publish.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_damage_log() -> String {
    edge::read_damage_log_string()
}

/// Called by the Bevy `debug::damage::publish_damage_debug` system to update the
/// damage-log JSON that JS reads via `wasm_get_damage_log()`.
#[cfg(target_arch = "wasm32")]
pub fn set_damage_log_string(text: String) {
    edge::publish_damage_log_string(text);
}

/// Called by JS each animation frame to read the latest entity-behavior payload
/// as JSON while the surface is visible (issue #1150). The dock parses and
/// renders it; empty until the first publish.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_entity_debug_state() -> String {
    edge::read_entity_debug_string()
}

/// Called by the Bevy `debug::entities::publish_entity_behavior_debug` system to
/// update the entity-behavior JSON that JS reads via `wasm_get_entity_debug_state()`.
#[cfg(target_arch = "wasm32")]
pub fn set_entity_debug_string(text: String) {
    edge::publish_entity_debug_string(text);
}

/// Called by JS each animation frame to read the latest entity-inspector payload
/// as JSON while the surface is visible (issue #1150). The dock parses and
/// renders it; empty until the first publish.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_entity_inspector() -> String {
    edge::read_entity_inspector_string()
}

/// Called by the Bevy `debug::inspector::publish_entity_inspector_debug` system
/// to update the entity-inspector JSON that JS reads via `wasm_get_entity_inspector()`.
#[cfg(target_arch = "wasm32")]
pub fn set_entity_inspector_string(text: String) {
    edge::publish_entity_inspector_string(text);
}

/// Called by JS each animation frame to read the latest station-activity payload
/// as JSON while the chart is visible (issue #1145).
///
/// Returns the raw JSON string `debug::station_activity::publish_station_activity`
/// wrote; the dock parses it and draws a chart rather than printing it. Empty
/// until the first publish.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_station_activity() -> String {
    edge::read_station_activity_string()
}

/// Called by the Bevy `publish_station_activity` system to update the
/// station-activity JSON that JS reads via `wasm_get_station_activity()`.
#[cfg(target_arch = "wasm32")]
pub fn set_station_activity_string(text: String) {
    edge::publish_station_activity_string(text);
}

/// Called by JS each animation frame to read the latest AI doctrine-pool payload
/// as JSON while the panel is visible (issue #1149).
///
/// Returns the raw JSON string `debug::ai_state::publish_ai_doctrine` wrote; the
/// dock parses it and draws a per-ship panel. Empty until the first publish.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_ai_doctrine() -> String {
    edge::read_ai_doctrine_string()
}

/// Called by the Bevy `publish_ai_doctrine` system to update the AI doctrine-pool
/// JSON that JS reads via `wasm_get_ai_doctrine()`.
#[cfg(target_arch = "wasm32")]
pub fn set_ai_doctrine_string(text: String) {
    edge::publish_ai_doctrine_string(text);
}

/// Called by JS each animation frame to read the latest scenario-state payload
/// as JSON while the panel is visible (issue #1148).
///
/// Returns the raw JSON string `debug::scenario::publish_scenario_state` wrote;
/// the dock parses it and draws a panel. Empty until the first publish.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_scenario_state() -> String {
    edge::read_scenario_state_string()
}

/// Called by the Bevy `publish_scenario_state` system to update the
/// scenario-state JSON that JS reads via `wasm_get_scenario_state()`.
#[cfg(target_arch = "wasm32")]
pub fn set_scenario_state_string(text: String) {
    edge::publish_scenario_state_string(text);
}

/// Called by JS while the settings cog is open, to read the debug flags the
/// simulation actually holds (issue #1169).
///
/// Returns the JSON object `debug_overlay::report_debug_state` mirrors here —
/// `{"Regions":false,"ConsoleLatency":true,…}`, keyed by catalogue wire
/// names — or an empty string before the first report.
///
/// # Why the cog needed a read-back at all
///
/// The debug OUTPUT resources had no read-back export, so the cog painted from
/// its own module-local memory of what it had clicked. A phone flipping the same
/// flag left the two disagreeing, and for console latency that disagreement is
/// not cosmetic. The mirror is written by the
/// one system that already computes this set for the wire, so the host page and
/// a connected phone read the same answer derived from the same place.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_debug_flags() -> String {
    edge::read_debug_flags_string()
}

/// Called by the Bevy `report_debug_state` system to mirror the debug-flag
/// read-back JS reads via [`wasm_get_debug_flags`].
#[cfg(target_arch = "wasm32")]
pub fn set_debug_flags_string(text: String) {
    edge::publish_debug_flags_string(text);
}

/// Called by JS each animation frame to read the latest console-latency payload
/// as JSON while the panel is visible (issue #1169).
///
/// Returns the raw JSON string `debug::console_latency::publish_console_latency`
/// wrote; the dock parses it and draws a per-action table. Empty until the first
/// publish.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_console_latency() -> String {
    edge::read_console_latency_string()
}

/// Called by the Bevy `publish_console_latency` system to update the
/// console-latency JSON that JS reads via `wasm_get_console_latency()`.
#[cfg(target_arch = "wasm32")]
pub fn set_console_latency_string(text: String) {
    edge::publish_console_latency_string(text);
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
    edge::publish_pending_force_start(true);
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
    edge::publish_pending_teleport_to_waypoint(true);
}

/// Called by JS each animation frame to check whether the LocalShip currently
/// has a shared Navigation waypoint (issue #770, AC2). The host Debug panel
/// disables the teleport control while this returns `false`. Reads back the
/// value maintained by `publish_waypoint_existence`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_has_navigation_waypoint() -> bool {
    edge::read_has_navigation_waypoint()
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
    edge::read_sim_tick_count() as f64
}

// ── Config Preload Exports ──────────────────────────────────────────────────

/// Re-export config preload functions from config_cache module.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_config_request_callback(callback: js_sys::Function) {
    crate::entities::config_cache::set_config_request_callback(callback);
}

/// Re-export config preload functions from config_cache module.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_load_config(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    // The preload has no single entry point — JS drives it one config at a
    // time — so the clock starts on the first one and stops when the page
    // first observes it complete (issue #868).
    crate::perf::browser::preload_begin_once();
    crate::entities::config_cache::wasm_load_config(path, toml_str)
}

/// Check if preload is complete.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_is_preload_complete() -> bool {
    let complete = crate::entities::config_cache::wasm_is_preload_complete();
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
    let toml_str = crate::entities::config_cache::mod_pack_overlay_get(&path).unwrap_or(toml_str);
    crate::content_ledger::record(&path, &toml_str);
    edge::publish_snapshot_world(Some((path.clone(), toml_str.clone())));
    crate::entities::config_cache::wasm_load_world(path, toml_str, curated_ships)
}

/// Authoritative resident world source, including pack-only scenarios.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_preload_world_source(path: String) -> Option<String> {
    match crate::entities::config_cache::world_fetch_state(&path) {
        crate::entities::config_cache::WorldFetchState::Ready(source) => Some(source),
        _ => None,
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_preload_error() -> String {
    crate::entities::config_cache::preload_error()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_fail_preload_fetch(path: String, message: String) {
    crate::entities::config_cache::fail_preload_fetch(path, message);
}

/// Register the JS callback used by Rust to request runtime world/script content.
///
/// The callback signature is: `callback(path: string)`. When called, JS must
/// fetch the TOML or Rhai source at `path` and deliver it via
/// `wasm_push_world_toml`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_world_fetch_callback(callback: js_sys::Function) {
    crate::entities::config_cache::set_world_fetch_callback(callback);
}

/// Deliver runtime-fetched world TOML or sibling Rhai source to the Rust side.
///
/// Called by JS after fetching a world/script path that Rust requested via the
/// `set_world_fetch_callback` callback.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_push_world_toml(path: String, toml_str: String) {
    crate::entities::config_cache::wasm_push_world_toml(path, toml_str);
}

/// Report a terminal runtime world/script fetch failure. Separate from
/// `wasm_push_world_toml` because an empty sibling Rhai file is valid.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_fail_world_fetch(path: String, message: String) {
    crate::entities::config_cache::wasm_fail_world_fetch(path, message);
}

/// Deliver an entity template to Rust for PRE-LOAD catalogue enrichment.
///
/// `wasm_get_scenario_catalog` is read before any world is activated, so
/// `delivery::payload::ship_payload` finds no cached template and publishes
/// `template_path` + `label` and nothing else — the reason every hull card in
/// the picker badged `[UNKNOWN]` with no registry, mass or power rating. The
/// host page closes that gap by fetching each hull the first catalogue pass
/// names and delivering it here BEFORE reading the catalogue again.
///
/// Pass `root = true` for a hull the catalogue named and `false` for an include
/// fragment. The return value is the canonical fragment paths still missing;
/// fetch those, deliver them the same way, and repeat until it comes back
/// empty. This store is separate from the preload's own and is read only by
/// `delivery::payload::ship_payload` — see `config_cache::push_catalog_template`.
///
/// Pass an EMPTY `toml_str` when the fetch failed, the way `handleConfigRequest`
/// calls `wasm_load_config(path, '')` on a 404: a mod pack's own hull has no URL
/// at all, and that is what lets its text be taken from the session overlay
/// instead. With no overlay copy either the call is a no-op.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_push_catalog_template(path: String, toml_str: String, root: bool) -> Array {
    let out = Array::new();
    for p in crate::entities::config_cache::push_catalog_template(path, toml_str, root) {
        out.push(&JsValue::from_str(&p));
    }
    out
}

/// Deliver a preloaded or runtime-fetched model-rig sidecar TOML to Rust.
///
/// Before boot, the entity-config preload calls this for every primary authored
/// rig and uses the return value as its completion signal; this records the
/// exact bytes before the content ledger freezes. The runtime world callback
/// reuses it for later/generated paths and ignores the return. Pass an empty
/// string when the sidecar is absent (404) so every target binds the same empty
/// bytes and proceeds with an identity rig.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_push_sidecar_toml(path: String, toml_str: String) -> bool {
    crate::entities::config_cache::wasm_push_sidecar_toml(path, toml_str)
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
    let world_config = crate::entities::config_cache::get_world_config();
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

/// Return the scenario-authored GM role preset list for the currently loaded
/// world (issue #1319), as a JSON array of
/// `{ id, label, panels, quick_actions, contacts }` objects.
///
/// Presentation only: the browser GM page (`gui/gm-role-presets.js`) filters
/// its own panels/quick actions against whichever preset a Game Master picks
/// and live-switches. Nothing here reaches `GmOperator`, the crew-public GM
/// roster, a `GmAction`, a snapshot, or the sim digest — see
/// `pasm/spec/design/gm-console-t2.yaml`'s `gm-t2-performing-surface`.
///
/// Returns `"[]"` when the world declares none, or before a world has
/// loaded — the GM page falls back to the single built-in "All" preset,
/// which is never authored and always available.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_get_gm_role_presets() -> String {
    let world_config = crate::entities::config_cache::get_world_config();
    match world_config {
        Some(ref wc) => crate::core::codec::encode_gm_role_presets(&wc.gm_role_presets),
        None => "[]".to_string(),
    }
}

/// Enrich one `AvailableShipEntry` into a JS `{ template_path, label, class,
/// hull_id, power_rating, name }` object, reading the extra metadata from the
/// cached entity config when it is available.
///
/// Shared by `wasm_get_available_ships` (post-load, reads the loaded
/// `WorldConfig`) and `wasm_get_scenario_catalog` (pre-load, reads the base
/// scenario manifest) so both surfaces present ships identically — and, since
/// PRD #855, with the native host too: the field list is
/// `delivery::payload::ship_payload`'s, not this function's.
#[cfg(target_arch = "wasm32")]
fn ship_entry_to_js(ship: &crate::world::config::AvailableShipEntry) -> Object {
    let payload = crate::delivery::payload::ship_payload(ship);
    crate::core::codec::encode_catalog_ship(&payload)
        .ok()
        .and_then(|json| js_sys::JSON::parse(&json).ok())
        .map(Object::from)
        .unwrap_or_default()
}

/// Deliver the base scenario manifest (`assets/scenarios.toml`) to Rust.
///
/// Called by JS during preload, before any world is loaded. Stored so
/// `wasm_get_scenario_catalog` can build the pre-load catalog (issue #754).
///
/// It is ALSO this host's content identity: [`wasm_delivery_stamp`],
/// [`wasm_delivery_stamp_field`], [`wasm_check_client_stamp`] and
/// [`wasm_check_host_stamp`] all build their stamp from whatever was pushed
/// here, and an empty store stamps an identity that matches nothing (and, for a
/// fleet, is refused outright). So `server.html` pushes it on EVERY boot path —
/// `pushScenarioManifest()` — not only from the catalogue build the
/// `?scenario=` bypass skips.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_push_scenario_manifest(toml_str: String) {
    crate::entities::config_cache::set_scenario_manifest_toml(toml_str);
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
/// content the host has already fetched (`cached_base_world_source` for worlds,
/// `raw_template_text` for entity/fragment TOML).
///
/// **Absent from a demo build** (PRD #855, `build_flags::accepts_mod_pack_
/// uploads`). The public build ships a deliberately restricted catalogue —
/// combat_test with the Alliance Destroyer and Alliance Cruiser, curated by
/// `assets/scenarios.demo.toml` — and this is the one call that
/// widens it at runtime, adding whatever scenarios and hulls an uploaded ZIP
/// carries. Gating it with `#[cfg]` rather than a runtime refusal is the same
/// doctrine `command_admission::debug_route` follows and for the same reason:
/// the host page's upload button is hidden in a demo build
/// (`gui/build-flags.js`'s `offersModPackUpload`), a hidden button is a UI fact,
/// and UI facts are forgeable. With the export compiled out, the hidden control
/// and the closed route cannot come apart.
///
/// The rest of the overlay surface (`wasm_clear_mod_pack`,
/// `wasm_remove_mod_pack`, `wasm_reorder_mod_packs`, `wasm_active_pack_
/// manifest`) is deliberately NOT gated: `server.html` calls those
/// unconditionally, and with nothing able to enter the stack they operate on an
/// empty one and answer emptily. Gating the entrance is the whole restriction;
/// gating the readers would only turn a no-op into a `TypeError`.
#[cfg(all(target_arch = "wasm32", not(phoenix_demo_build)))]
#[wasm_bindgen]
pub fn wasm_add_mod_pack(bytes: &[u8]) -> Array {
    // The host side of the mod-pack compatibility contract (issue #986): read
    // the base manifest's `[content]` identity and INJECT it, rather than let
    // the pure validator reach for a host default — the same seam discipline as
    // `resolve_base` below. A host whose manifest declares no `[content]` block
    // yields an identity no real pack can match (empty id, epoch 0), so an
    // upload is rejected rather than silently accepted against unknown content.
    let base_content = crate::entities::config_cache::get_scenario_manifest_toml()
        .and_then(|toml| crate::world::manifest::parse_content_identity(&toml))
        .unwrap_or_default();
    // The already-active overlay stack the candidate is judged against (#987).
    let active = crate::entities::config_cache::active_packs();
    let result = crate::world::mod_pack::validate_mod_pack(
        bytes,
        &base_content,
        |path| {
            // Base content the host has already fetched, by authored path:
            // world TOML for the manifest, and raw entity/fragment TOML so a
            // pack hull may include a SHIPPED fragment. The active overlay stack
            // is consulted by `validate_mod_pack` itself (via the `active` slice
            // below), BENEATH the candidate and ABOVE this base resolver.
            crate::entities::config_cache::cached_base_world_source(path)
                .or_else(|| crate::entities::config_cache::raw_template_text(path))
        },
        &crate::entities::loader::WasmTemplateLoader,
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
        crate::entities::config_cache::push_mod_pack(crate::entities::config_cache::ActivePack {
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
    crate::entities::config_cache::clear_mod_pack_overlay();
}

/// Remove the pack with `id` from the overlay stack (issue #987). Precedence for
/// every path it owned re-resolves automatically — the next pack down that
/// carries the path becomes the winner. Returns whether a pack was removed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_remove_mod_pack(id: String) -> bool {
    crate::entities::config_cache::remove_mod_pack(&id)
}

/// Reorder the overlay stack to match `ids` (oldest → newest / lowest → highest
/// precedence), from the host reorder controls (issue #987). Ids not named keep
/// their relative order after the named ones; unknown ids are ignored.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_reorder_mod_packs(ids: Vec<String>) {
    crate::entities::config_cache::reorder_mod_packs(&ids);
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
    let packs = crate::entities::config_cache::active_packs();
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
    for conflict in crate::entities::config_cache::overlay_conflicts(&packs) {
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
    let Some(manifest_toml) = crate::entities::config_cache::get_scenario_manifest_toml() else {
        return arr;
    };
    let Ok(manifest) = parse_manifest(&manifest_toml) else {
        return arr;
    };
    // Merge the base manifest with EVERY active mod-pack manifest, in load order
    // (issue #760 AC3, #987), resolving every root world through the overlay-aware
    // resolver (the winning pack's content first, then base). Only
    // manifest-listed scenarios appear.
    let active = crate::entities::config_cache::active_packs();
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
    let resolve_world = |path: &str| crate::entities::config_cache::resolved_world_source(path);
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
    crate::core::codec::encode_scenario_catalog(&crate::delivery::payload::catalog_payload(
        &catalog,
    ))
    .ok()
    .and_then(|json| js_sys::JSON::parse(&json).ok())
    .map(|value| Array::from(&value))
    .unwrap_or_default()
}

/// Publish the browser picker's current enriched catalogue through the same
/// typed message and pack projection as the native host. Taking the current
/// picker snapshot preserves asynchronous template enrichment and curation.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_scenario_catalog_message(
    scenarios_json: &str,
    locked_scenario: Option<String>,
    locked_ship: Option<String>,
) -> Result<String, JsValue> {
    browser_scenario_catalog_message(scenarios_json, locked_scenario, locked_ship)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// Target-free adapter so fixtures exercise the browser's real decode/project/
/// encode path, rather than a second JavaScript message implementation.
pub fn browser_scenario_catalog_message(
    scenarios_json: &str,
    locked_scenario: Option<String>,
    locked_ship: Option<String>,
) -> Result<String, serde_json::Error> {
    use crate::core::codec::JsonCodec;
    let scenarios = crate::core::codec::decode_scenario_catalog(scenarios_json)?;
    let payload = crate::delivery::payload::catalogue_snapshot(
        scenarios,
        &crate::entities::config_cache::active_packs(),
        locked_scenario,
        locked_ship,
    );
    JsonCodec.encode_server(&crate::core::messages::ServerMessage::ScenarioCatalog(
        payload,
    ))
}

/// This host's delivery version stamp, as the JSON `phoenix-host` serves at
/// `/host/stamp.json` (PRD #855).
///
/// The browser host's half of the version pin: `server.html` can hand a peer
/// the same three numbers a native host publishes, encoded by the same
/// `codec::encode_delivery_stamp`, so "native and browser hosts consume the
/// same protocol contract" is checkable rather than asserted.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_delivery_stamp() -> String {
    let manifest_toml =
        crate::entities::config_cache::get_scenario_manifest_toml().unwrap_or_default();
    crate::core::codec::encode_delivery_stamp(&crate::delivery::stamp::DeliveryStamp::for_manifest(
        &manifest_toml,
    ))
}

/// Judge a joining client's version stamp against this host's (issue #1111,
/// made mandatory in #1112).
///
/// The host half of the Phoenix join handshake. `server.html` hands over the
/// `<protocol>/<content_id>/<content_epoch>` field a joiner declared over its
/// DataChannel and gets back `{"ok":true,…}` or `{"ok":false,"code":…,…}`; an
/// empty string means the client declared nothing, which is now REFUSED
/// (`client-stamp-missing`) rather than admitted — every client that can reach
/// a Phoenix host is a built Phoenix bundle carrying the field. See
/// [`crate::delivery::check_join_stamp`] for the full rule.
///
/// This export is the reason the verdict is not re-implemented in JavaScript.
/// The rendezvous service's version advice is discovery help; the authority
/// stays `delivery::stamp::check_client_stamp`, the same pin the native host
/// enforces over HTTP.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_check_client_stamp(client_stamp: String) -> String {
    let manifest_toml =
        crate::entities::config_cache::get_scenario_manifest_toml().unwrap_or_default();
    let host = crate::delivery::stamp::DeliveryStamp::for_manifest(&manifest_toml);
    let verdict = crate::delivery::check_join_stamp(&host, Some(client_stamp.as_str()));
    crate::core::codec::encode_join_verdict(&verdict, &host)
}

/// Judge a joining SHIP HOST's version stamp against this host's (issue #1114).
///
/// The fleet half of the same handshake, and a separate export rather than a
/// flag on the one above because the two answers genuinely differ: a host with
/// no manifest loaded admits a phone on the protocol alone and admits no ship
/// at all. See [`crate::delivery::check_host_stamp`] for why.
///
/// Same `{"ok":…}` shape and the same `StampMismatch::code()` vocabulary, so
/// neither `gui/host-mesh.js` nor `gui/join-code.js`'s reason map needs a
/// fleet-only spelling of "that build does not match".
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_check_host_stamp(peer_stamp: String) -> String {
    let manifest_toml =
        crate::entities::config_cache::get_scenario_manifest_toml().unwrap_or_default();
    let host = crate::delivery::stamp::DeliveryStamp::for_manifest(&manifest_toml);
    let verdict = crate::delivery::check_host_stamp(&host, Some(peer_stamp.as_str()));
    crate::core::codec::encode_join_verdict(&verdict, &host)
}

/// This host's own stamp as the three-part `<protocol>/<content_id>/<epoch>`
/// FIELD (issue #1114).
///
/// [`wasm_delivery_stamp`] answers the same three numbers as a JSON object,
/// because that is what `/host/stamp.json` publishes. A ship host JOINING a
/// fleet has to present them in the compact form the handshake reads, and
/// having the page reassemble that string from the JSON would be a second,
/// quietly divergent spelling of a format `delivery::parse_stamp_field`
/// already owns — the same field a client bundle carries in its
/// `phoenix-client-stamp` meta tag.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn wasm_delivery_stamp_field() -> String {
    let manifest_toml =
        crate::entities::config_cache::get_scenario_manifest_toml().unwrap_or_default();
    let stamp = crate::delivery::stamp::DeliveryStamp::for_manifest(&manifest_toml);
    format!(
        "{}/{}/{}",
        stamp.protocol, stamp.content_id, stamp.content_epoch
    )
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
    edge::publish_selected_ship_template_path(Some(template_path.to_string()));
}

// ── Bevy bridge systems ────────────────────────────────────────────────────

/// Drains the inbound queue each frame and injects messages into Bevy.
/// Decode failures are logged as warnings with truncated token/payload.
#[cfg(target_arch = "wasm32")]
fn drain_inbound(mut writer: MessageWriter<InboundMessage>) {
    let pending: Vec<(String, String)> = edge::drain_inbound_queue();
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

/// Drains host-page diagnostic states and the separate Gameplay pause toggle.
///
/// Diagnostic mutation delegates to module-owned catalogue adapters. The
/// bridge neither names their Resources nor carries a positional boolean list.
/// Pause stays on its own thread-local and resource path because it changes the
/// authoritative clock and is available to the trusted host in demo builds.
#[cfg(target_arch = "wasm32")]
fn drain_host_controls(world: &mut World) {
    #[cfg(not(phoenix_demo_build))]
    {
        let pending: Vec<(DebugSurface, bool)> = edge::take_debug_surfaces();
        crate::debug::catalogue::apply_pending_states(world, pending);
    }

    let pause_changed = edge::take_pending_pause();
    if pause_changed
        && raw_host_control_allowed(world.contains_resource::<crate::lockstep::FleetLockstep>())
    {
        let paused = {
            let mut state = world.resource_mut::<crate::debug_overlay::SimulationPaused>();
            state.0 = !state.0;
            state.0
        };
        edge::publish_sim_paused(paused);
        let mut virtual_time = world.resource_mut::<Time<bevy::time::Virtual>>();
        if paused {
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
    let pending: Vec<String> = edge::drain_disconnect_queue();
    for token in pending {
        writer.write(PlayerDisconnected { token });
    }
}

/// Bevy-side latch for a pending force-start request, bridging whatever asked
/// for it to [`apply_force_start`] (the `FixedUpdate` state writer) — see the
/// #907 review note on the latter for why the one function that used to do both
/// is now two, in two different schedules.
///
/// **Two things ask, and only one of them is JavaScript.** On the browser host
/// it is `wasm_force_start()`, drained out of a thread-local by
/// [`drain_force_start_input`] in `PreUpdate`. On a native host (issue #1328) it
/// is the lobby surface's own AI-launch control, which sets this resource
/// directly from a Bevy system
/// (`native_host::host_lobby::drain_surface_records`) — there is no thread-local
/// and no JS to read one out of.
///
/// Which is why the latch, rather than each caller writing `NextState` itself:
/// the *decision* (is this the Lobby? has the preload finished? is there a world
/// at all?) is one policy, stated once in `apply_force_start`, and the request is
/// just a bool that policy reads.
#[derive(Resource, Default)]
pub struct PendingForceStart(pub bool);

/// The legacy "Launch AI Ship" route is valid only outside coordinated fleet
/// lobby ownership. Compiled on every target since `apply_force_start` became
/// native (#1328): a native host's force-start takes the same managed-mode bypass
/// guard the browser host does.
fn legacy_force_start_allowed(
    managed: &crate::lobby::FleetManagedLobby,
    phase: &crate::core::messages::GamePhase,
) -> bool {
    !managed.enabled && phase == &crate::core::messages::GamePhase::Lobby
}

/// Drains the force-start thread-local each frame into [`PendingForceStart`].
/// The actual phase transition is [`apply_force_start`]'s job — this system
/// only moves the JS-set flag into a Bevy resource so `apply_force_start` can
/// run in `FixedUpdate` without touching a thread-local from inside the fixed
/// schedule.
#[cfg(target_arch = "wasm32")]
fn drain_force_start_input(mut pending: ResMut<PendingForceStart>) {
    let was = edge::take_pending_force_start();
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
///
/// **Not wasm-only since issue #1328.** The rule this applies — Lobby only, wait
/// for the preload, announce `GameStarted` — is host policy rather than browser
/// glue, and a native host needs exactly it: its lobby is the viewscreen, and a
/// crew who are all on phones must be launchable from the surface in front of
/// them. `native_host::app` registers this system on the same
/// `.before(SimSet::Input)` edge `wasm_init` gives it, so the mint inside
/// `OnEnter(InProgress)` stamps the same tick on both hosts. Only
/// [`drain_force_start_input`] stays behind the `wasm32` gate, because a
/// thread-local set by JavaScript is the one part of this that genuinely is
/// browser glue.
///
/// # The world guard
///
/// `world_config` is `None` on a `--lobby` host that has not been given a
/// scenario yet (issue #1326), and starting a mission there would run
/// `spawn_game_start_entities` over no world at all. It is the same guard
/// `native_host::app::solo_auto_start` carries, for the same reason, and it
/// changes nothing in the browser: a host page loads its world before
/// `wasm_init` composes the `App`, so the resource is there before the first
/// fixed step.
///
/// A request that arrives with no world is **dropped**, not held. Remembering it
/// would start the mission the instant somebody else's scenario pick landed,
/// which is not what the person who pressed the button asked for.
pub(crate) fn apply_force_start(
    state: Res<State<crate::core::messages::GamePhase>>,
    mut next_state: ResMut<NextState<crate::core::messages::GamePhase>>,
    mut outbox: ResMut<crate::lobby::LobbyOutbox>,
    preload: Option<Res<crate::server::asset_preload::AssetPreloadResource>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut pending: ResMut<PendingForceStart>,
    managed: Res<FleetManagedLobby>,
) {
    let pending_flag = std::mem::take(&mut pending.0);
    // A fleet-managed lobby owns its own start path, so legacy force-start is
    // gated to a standalone lobby (`legacy_force_start_allowed`) — and, for the
    // `--lobby` host (#1326), only once a world has actually been picked.
    if !pending_flag || !legacy_force_start_allowed(&managed, state.get()) || world_config.is_none()
    {
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
        next_state.set(crate::core::messages::GamePhase::InProgress);
        outbox.0.push((
            crate::lobby::Target::All,
            crate::core::messages::ServerMessage::GameStarted,
        ));
    } else {
        next_state.set(crate::core::messages::GamePhase::Loading);
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
    fleet: Option<Res<crate::lockstep::FleetLockstep>>,
    mut ship_q: Query<
        (
            &mut crate::ship::state::ShipPhysics,
            &crate::console::navigation::NavigationWaypoint,
        ),
        With<crate::server_app::LocalShip>,
    >,
) {
    let requested = edge::take_pending_teleport_to_waypoint();
    if !requested || !raw_host_control_allowed(fleet.is_some()) {
        return;
    }
    for (mut physics, waypoint) in ship_q.iter_mut() {
        crate::console::navigation::server::apply_teleport_to_waypoint(&mut physics, waypoint);
    }
}

/// Mirrors [`crate::sim_tick::SimTick`] into a thread-local each frame so
/// `wasm_sim_tick()` can read it back (issue #895). Pure read; no gameplay
/// effect.
#[cfg(target_arch = "wasm32")]
fn publish_sim_tick(tick: Res<crate::sim_tick::SimTick>) {
    edge::publish_sim_tick_count(tick.0);
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
    let pending = edge::take_pending_god_mode_toggles();
    for _ in 0..pending {
        writer.write(InboundMessage {
            token: crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
            msg: messages::ClientMessage::ControlSystem {
                target: messages::SystemId(
                    crate::ship::system_registry::GOD_MODE_SYSTEM_ID.to_string(),
                ),
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
    edge::publish_god_mode_mirror(active);
}

/// Drains the queued instagib toggles each frame into the [`crate::server_app::Instagib`] Resource
/// (issue #1181). Unlike `drain_god_mode_toggle` it flips the Resource directly
/// rather than crossing command admission — instagib is a raw host cheat, not a
/// replicated command. The parity logic is [`apply_instagib_toggles`], unit-
/// tested on native.
#[cfg(target_arch = "wasm32")]
fn drain_instagib_toggle(
    fleet: Option<Res<crate::lockstep::FleetLockstep>>,
    mut instagib: ResMut<crate::server_app::Instagib>,
) {
    let count = edge::take_pending_instagib_toggles();
    if raw_host_control_allowed(fleet.is_some()) {
        apply_instagib_toggles(count, &mut instagib.0);
    } else {
        // Joining canonicalises the flag off. Keep it neutral even if this
        // drain happens later in the same PreUpdate as fleet adoption.
        instagib.0 = false;
    }
}

/// Mirrors the authoritative [`crate::server_app::Instagib`] Resource into a thread-local each
/// frame so `wasm_get_instagib()` can read it back without a `World` handle
/// (issue #1181). Pure read; the same pattern as `publish_god_mode`.
#[cfg(target_arch = "wasm32")]
fn publish_instagib(instagib: Res<crate::server_app::Instagib>) {
    edge::publish_instagib_mirror(instagib.0);
}

/// Mirror the pause resource for the host Gameplay control's synchronous
/// getter. Diagnostic readback comes from the canonical all-build catalogue
/// resource instead.
#[cfg(target_arch = "wasm32")]
fn publish_pause_mirror(paused: Res<crate::debug_overlay::SimulationPaused>) {
    edge::publish_sim_paused(paused.0);
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
    edge::publish_has_navigation_waypoint(has);
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

    if let Some(cb) = edge::outbound_callback() {
        for (target, payload, class_str) in &dispatches {
            let _ = cb.call3(
                &JsValue::NULL,
                &JsValue::from_str(target),
                &JsValue::from_str(payload),
                &JsValue::from_str(class_str),
            );
        }
    };
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
    mut gm_entity: MessageReader<GmEntityProjectionChanged>,
    mut gm_activity: MessageReader<GmActivityFeedChanged>,
    mut gm_station: MessageReader<GmStationProjectionChanged>,
    mut gm_session: MessageReader<GmSessionChanged>,
    mut gm_mission: MessageReader<GmMissionChanged>,
    mut gm_spawn: MessageReader<GmSpawnChanged>,
    mut gm_comms: MessageReader<GmCommsChanged>,
) {
    // Declarative channel table: name → drained JSON payloads. Adding a
    // message channel = one row here (see `host_channels`).
    let message_batches: [(&str, Vec<String>); 12] = [
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
        (
            host_channels::GM_ENTITY,
            gm_entity
                .read()
                .filter_map(|event| codec::encode_gm_entity_projection(&event.payload).ok())
                .collect(),
        ),
        (
            host_channels::GM_ACTIVITY,
            gm_activity
                .read()
                .filter_map(|event| codec::encode_gm_activity_feed(&event.payload).ok())
                .collect(),
        ),
        (
            host_channels::GM_STATION,
            gm_station
                .read()
                .filter_map(|event| codec::encode_gm_station_projection(&event.payload).ok())
                .collect(),
        ),
        (
            host_channels::GM_SESSION,
            gm_session
                .read()
                .filter_map(|event| codec::encode_gm_session_projection(&event.payload).ok())
                .collect(),
        ),
        (
            host_channels::GM_MISSION,
            gm_mission
                .read()
                .filter_map(|event| codec::encode_gm_mission_projection(&event.payload).ok())
                .collect(),
        ),
        (
            host_channels::GM_COMMS,
            gm_comms
                .read()
                .filter_map(|event| codec::encode_gm_comms_projection(&event.payload).ok())
                .collect(),
        ),
        (
            host_channels::GM_SPAWN,
            gm_spawn
                .read()
                .filter_map(|event| codec::encode_gm_spawn_projection(&event.payload).ok())
                .collect(),
        ),
    ];

    {
        let Some(cb) = edge::host_channel_callback() else {
            return;
        };

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
        let (x, y) = edge::read_shake_offset();
        let offset = Array::of2(&JsValue::from_f64(x as f64), &JsValue::from_f64(y as f64));
        let _ = cb.call2(
            &JsValue::NULL,
            &JsValue::from_str(host_channels::SHAKE),
            &offset,
        );

        // Per-frame tap: forcefield level, epsilon-deduped.
        let current = edge::read_forcefield_level();
        let last = edge::read_last_sent_forcefield();
        if (current - last).abs() >= 0.001 {
            edge::publish_last_sent_forcefield(current);
            let _ = cb.call2(
                &JsValue::NULL,
                &JsValue::from_str(host_channels::AUDIO_LEVEL),
                &JsValue::from_f64(current as f64),
            );
        }
    };
}

// ── Tests ───────────────────────────────────────────────────────────────────
//
// The Debug Surface adapter behavior stays native-testable even though the
// host export is WASM-only; the bridge test below feeds the same canonical
// identities the phone drain collects through the catalogue applier.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "bridge_resume_content_tests.rs"]
mod resume_content_tests;

#[cfg(test)]
mod tests {
    use super::{
        apply_gm_roster_replacement, apply_instagib_toggles, defer_unloaded_scenario_content,
        host_channels, import_artifact_into_catalogue, import_resume_after_scenario,
        legacy_force_start_allowed, load_resume_after_scenario, queue_fleet_lobby_input_bounded,
        rebind_fleet_lobby_projections, save_slot_start_projection, scoped_browser_save_namespace,
        BoundedFifo, BrowserResumeRefusal, ImportSlotRefusal, PendingBrowserSaves,
        MAX_PENDING_BROWSER_SAVES,
    };
    use crate::console::navigation::server::apply_teleport_to_waypoint;
    use crate::console::navigation::{NavigationWaypoint, WaypointMode};
    use crate::server_app::Instagib;
    use crate::ship::state::ShipPhysics;
    use bevy::prelude::{App, Messages};
    use std::fmt;

    #[derive(Debug)]
    struct ReadOnlyStoreError;

    impl fmt::Display for ReadOnlyStoreError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("read-only test store")
        }
    }

    struct ReadOnlyStore {
        slot: String,
        text: String,
    }

    impl vellum_save::Store for ReadOnlyStore {
        type Error = ReadOnlyStoreError;

        fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
            Ok((slot == self.slot).then(|| self.text.clone()))
        }

        fn write(&self, _slot: &str, _contents: &str) -> Result<(), Self::Error> {
            Err(ReadOnlyStoreError)
        }

        fn remove(&self, _slot: &str) -> Result<(), Self::Error> {
            Err(ReadOnlyStoreError)
        }

        fn slots(&self) -> Result<Vec<String>, Self::Error> {
            Ok(vec![self.slot.clone()])
        }
    }

    /// A Store that actually takes writes, for the paths that make a row.
    #[derive(Default)]
    struct MapStore {
        slots: std::cell::RefCell<std::collections::BTreeMap<String, String>>,
    }

    impl vellum_save::Store for MapStore {
        type Error = ReadOnlyStoreError;

        fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
            Ok(self.slots.borrow().get(slot).cloned())
        }

        fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
            self.slots
                .borrow_mut()
                .insert(slot.to_string(), contents.to_string());
            Ok(())
        }

        fn remove(&self, slot: &str) -> Result<(), Self::Error> {
            self.slots.borrow_mut().remove(slot);
            Ok(())
        }

        fn slots(&self) -> Result<Vec<String>, Self::Error> {
            Ok(self.slots.borrow().keys().cloned().collect())
        }
    }

    /// A minimal portable artifact of `scenario`, as a file a host would pick.
    fn portable_artifact(scenario: &str, versions: &vellum_save::Versions) -> String {
        crate::snapshot::run_for(
            crate::snapshot::PhoenixSnapshot {
                tick: 42,
                boot_identity: Some(crate::snapshot::BootIdentity {
                    selected_ship: "assets/entities/alliance_cruiser.toml".into(),
                    fleet: crate::lockstep::FleetRoster::default(),
                    game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                        authored_index: 0,
                        entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                    }],
                }),
                ..Default::default()
            },
            0xfeed,
            17,
            scenario,
            versions.clone(),
        )
        .to_ron()
        .expect("portable artifact must encode")
    }

    fn one_game_start_world() -> crate::world::config::WorldConfig {
        crate::world::config::parse_world(
            r#"
[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
"#,
        )
        .expect("browser resume world fixture must parse")
    }

    fn content_refusal(saved_content: u64, current_content: u64) -> crate::save_slots::StartState {
        let saved = vellum_save::Versions::new(7, "rules", saved_content);
        let current = vellum_save::Versions::new(7, "rules", current_content);
        let moved = saved.check(&current).expect_err("content must move");
        assert!(matches!(moved, vellum_save::Moved::Content { .. }));
        crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(moved))
    }

    fn catalogue_row(
        scenario: &str,
        start: crate::save_slots::StartState,
    ) -> crate::save_slots::SaveSlotEntry {
        crate::save_slots::SaveSlotEntry {
            slot_id: crate::save_slots::AUTOSAVE_SLOT.to_string(),
            kind: crate::save_slots::SaveSlotKind::Autosave,
            display_name: crate::save_slots::AUTOSAVE_SLOT.to_string(),
            metadata: crate::save_slots::MetadataStatus::NotApplicable,
            record: Some(crate::save_slots::SaveRecordSummary {
                scenario: scenario.to_string(),
                seed: 17,
                capture_tick: 42,
                boot_identity: Some(crate::snapshot::BootIdentity {
                    selected_ship: "assets/entities/alliance_cruiser.toml".into(),
                    fleet: crate::lockstep::FleetRoster::default(),
                    game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                        authored_index: 0,
                        entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                    }],
                }),
                versions: vellum_save::Versions::new(7, "rules", 0x1234),
            }),
            start,
        }
    }

    #[test]
    fn gm_roster_replacement_broadcasts_only_when_canonical_contents_change() {
        use crate::core::messages::{DeliveryClass, ServerMessage};
        use crate::gm_roster::{GmOperator, GmRoster};
        use crate::lobby::{OutboundMessage, Target};

        let mut app = App::new();
        app.add_message::<OutboundMessage>()
            .init_resource::<GmRoster>();
        let mut cursor = app
            .world()
            .resource::<Messages<OutboundMessage>>()
            .get_cursor();
        let first = GmRoster::try_new(vec![
            GmOperator {
                id: "gm-2".into(),
                name: String::new(),
                connected: false,
                ready: false,
            },
            GmOperator {
                id: "gm-1".into(),
                name: "Morgan".into(),
                connected: true,
                ready: true,
            },
        ])
        .unwrap();

        assert!(apply_gm_roster_replacement(app.world_mut(), first.clone()));
        assert!(
            !apply_gm_roster_replacement(app.world_mut(), first),
            "the same canonical full replacement is a no-op"
        );

        let messages: Vec<_> = cursor
            .read(app.world().resource::<Messages<OutboundMessage>>())
            .collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].target, Target::All);
        assert_eq!(messages[0].delivery, DeliveryClass::Reliable);
        assert!(matches!(
            &messages[0].msg,
            ServerMessage::GmRosterChanged { gms }
                if gms.iter().map(|gm| gm.id.as_str()).collect::<Vec<_>>()
                    == vec!["gm-1", "gm-2"]
        ));
    }

    #[test]
    fn gm_roster_replacement_clears_ready_on_reconnect() {
        use crate::gm_roster::{GmOperator, GmRoster};
        use crate::lobby::OutboundMessage;

        let mut app = App::new();
        app.add_message::<OutboundMessage>().insert_resource(
            GmRoster::try_new(vec![GmOperator {
                id: "gm-1".into(),
                name: "Morgan".into(),
                connected: false,
                ready: false,
            }])
            .unwrap(),
        );
        let replacement = GmRoster::try_new(vec![GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: true,
            ready: true,
        }])
        .unwrap();

        assert!(apply_gm_roster_replacement(app.world_mut(), replacement));
        let gm = &app.world().resource::<GmRoster>().operators()[0];
        assert!(gm.connected);
        assert!(!gm.ready, "a reconnect always returns unready");
    }

    #[test]
    fn legacy_ai_force_start_is_refused_while_fleet_managed() {
        let mut managed = crate::lobby::FleetManagedLobby::default();
        assert!(legacy_force_start_allowed(
            &managed,
            &crate::core::messages::GamePhase::Lobby
        ));
        managed.set_enabled(true);
        assert!(!legacy_force_start_allowed(
            &managed,
            &crate::core::messages::GamePhase::Lobby
        ));
        managed.set_enabled(false);
        assert!(!legacy_force_start_allowed(
            &managed,
            &crate::core::messages::GamePhase::InProgress
        ));
    }

    #[test]
    fn fleet_lobby_queue_coalesces_absolute_samples_without_losing_edges() {
        use crate::lobby::FleetLobbyInput;
        use std::collections::VecDeque;

        let mut pending = VecDeque::new();
        assert!(queue_fleet_lobby_input_bounded(
            &mut pending,
            8,
            FleetLobbyInput::Managed(false),
            4,
        ));
        assert!(queue_fleet_lobby_input_bounded(
            &mut pending,
            8,
            FleetLobbyInput::Managed(true),
            4,
        ));
        assert!(queue_fleet_lobby_input_bounded(
            &mut pending,
            8,
            FleetLobbyInput::Validation(false),
            4,
        ));
        assert!(queue_fleet_lobby_input_bounded(
            &mut pending,
            8,
            FleetLobbyInput::Validation(true),
            4,
        ));
        assert_eq!(
            pending.len(),
            3,
            "only the consecutive validation coalesces"
        );
        assert!(matches!(
            pending.pop_front().map(|row| row.input),
            Some(FleetLobbyInput::Managed(false))
        ));
        assert!(matches!(
            pending.pop_front().map(|row| row.input),
            Some(FleetLobbyInput::Managed(true))
        ));
        assert!(matches!(
            pending.pop_front().map(|row| row.input),
            Some(FleetLobbyInput::Validation(true))
        ));
    }

    #[test]
    fn fleet_lobby_queue_refuses_rather_than_displacing_a_generation_edge() {
        use crate::lobby::FleetLobbyInput;
        use std::collections::VecDeque;

        let mut pending = VecDeque::new();
        assert!(queue_fleet_lobby_input_bounded(
            &mut pending,
            2,
            FleetLobbyInput::Managed(false),
            2,
        ));
        assert!(queue_fleet_lobby_input_bounded(
            &mut pending,
            2,
            FleetLobbyInput::Managed(true),
            2,
        ));
        assert!(!queue_fleet_lobby_input_bounded(
            &mut pending,
            2,
            FleetLobbyInput::Validation(true),
            2,
        ));
        assert_eq!(pending.len(), 2);
        assert!(matches!(pending[0].input, FleetLobbyInput::Managed(false)));
        assert!(matches!(pending[1].input, FleetLobbyInput::Managed(true)));
    }

    #[test]
    fn prejoin_lobby_projections_rebind_before_the_new_generations_grant() {
        use crate::lobby::start_policy::{StartGrant, StartGrantMode};
        use crate::lobby::FleetLobbyInput;
        use std::collections::VecDeque;

        let mut pending = VecDeque::new();
        assert!(queue_fleet_lobby_input_bounded(
            &mut pending,
            0,
            FleetLobbyInput::Managed(true),
            64,
        ));
        assert!(queue_fleet_lobby_input_bounded(
            &mut pending,
            0,
            FleetLobbyInput::Validation(true),
            64,
        ));

        rebind_fleet_lobby_projections(&mut pending, 1, Some(true), Some(true));
        assert!(queue_fleet_lobby_input_bounded(
            &mut pending,
            1,
            FleetLobbyInput::Grant(StartGrant {
                id: "start-1".into(),
                mode: StartGrantMode::Automatic,
                operator_id: None,
                apply_tick: 0,
            }),
            64,
        ));
        assert_eq!(
            pending.iter().map(|row| row.generation).collect::<Vec<_>>(),
            vec![1, 1, 1]
        );

        let mut inputs = pending.into_iter().map(|row| row.input).collect();
        let mut managed = crate::lobby::FleetManagedLobby::default();
        let mut grants = crate::lobby::PendingStartGrants::default();
        let mut tracker = crate::lobby::server::StartGrantTracker::default();
        let mut results = crate::lobby::StartGrantResults::default();
        crate::lobby::apply_fleet_lobby_inputs(
            &mut inputs,
            &mut managed,
            &mut grants,
            &mut tracker,
            &mut results,
        );
        assert!(inputs.is_empty());
        assert!(managed.enabled);
        assert!(managed.validation_passed);
        assert_eq!(grants.len(), 1);
    }

    #[test]
    fn save_slot_projection_keeps_only_deferred_content_startable() {
        use crate::save_slots::StartState;

        let ready = save_slot_start_projection(&StartState::Ready);
        assert!(ready.compatible);
        assert!(ready.startable);
        assert_eq!(ready.refusal_kind, None);

        let deferred = save_slot_start_projection(&StartState::ContentDeferred);
        assert!(!deferred.compatible);
        assert!(deferred.startable);
        assert_eq!(deferred.refusal_kind, Some("content-pending"));
        assert_eq!(deferred.refusal, None);

        let saved = vellum_save::Versions::new(7, "rules-before", 0x1234);
        let current = vellum_save::Versions::new(7, "rules-now", 0x1234);
        let rules = saved.check(&current).expect_err("rules moved");
        let refused = save_slot_start_projection(&StartState::Refused(
            crate::snapshot::LoadRefusal::Moved(rules),
        ));
        assert!(!refused.compatible);
        assert!(!refused.startable);
        assert_eq!(refused.refusal_kind, Some("rules"));

        let corrupt = save_slot_start_projection(&StartState::Refused(
            crate::snapshot::LoadRefusal::Unparsable("damaged run".into()),
        ));
        assert!(!corrupt.startable);
        assert_eq!(corrupt.refusal_kind, Some("unparsable"));
    }

    #[test]
    fn save_catalogue_defers_content_on_fresh_boot() {
        let mut entries = [catalogue_row(
            "assets/worlds/scenario-a.toml",
            crate::save_slots::StartState::Ready,
        )];

        defer_unloaded_scenario_content(&mut entries, None);

        assert_eq!(
            entries[0].start,
            crate::save_slots::StartState::ContentDeferred,
            "even an accidental digest match is not proof before a scenario is loaded"
        );
    }

    #[test]
    fn save_catalogue_keeps_same_scenario_content_mismatch_refused() {
        let scenario = "assets/worlds/scenario-a.toml";
        let mut entries = [catalogue_row(scenario, content_refusal(0x1111, 0x2222))];

        defer_unloaded_scenario_content(&mut entries, Some(scenario));

        assert!(matches!(
            entries[0].start,
            crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Content { .. }
            ))
        ));
    }

    #[test]
    fn save_catalogue_defers_other_scenario_content_but_not_hard_failures() {
        let saved_rules = vellum_save::Versions::new(7, "rules-before", 0x1111);
        let current_rules = vellum_save::Versions::new(7, "rules-now", 0x1111);
        let rules_moved = saved_rules
            .check(&current_rules)
            .expect_err("rules must move");
        let mut entries = [
            catalogue_row(
                "assets/worlds/scenario-b.toml",
                content_refusal(0x1111, 0x2222),
            ),
            catalogue_row(
                "assets/worlds/scenario-b.toml",
                crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                    rules_moved,
                )),
            ),
            catalogue_row(
                "assets/worlds/scenario-b.toml",
                crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Unreadable(
                    "backend unavailable".into(),
                )),
            ),
        ];

        defer_unloaded_scenario_content(&mut entries, Some("assets/worlds/scenario-a.toml"));

        assert_eq!(
            entries[0].start,
            crate::save_slots::StartState::ContentDeferred
        );
        assert!(matches!(
            entries[1].start,
            crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Rules { .. }
            ))
        ));
        assert!(matches!(
            entries[2].start,
            crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Unreadable(_))
        ));
    }

    #[test]
    fn resume_gate_refuses_content_after_selected_scenario_loads() {
        let scenario = "assets/worlds/scenario-b.toml";
        let saved_versions = vellum_save::Versions::new(7, "rules", 0x1111);
        let loaded_versions = vellum_save::Versions::new(7, "rules", 0x2222);
        let mut catalogue = [catalogue_row(scenario, content_refusal(0x1111, 0xaaaa))];
        defer_unloaded_scenario_content(&mut catalogue, Some("assets/worlds/scenario-a.toml"));
        assert_eq!(
            catalogue[0].start,
            crate::save_slots::StartState::ContentDeferred
        );

        let run = crate::snapshot::run_for(
            crate::snapshot::PhoenixSnapshot::default(),
            0,
            17,
            scenario,
            saved_versions,
        );
        let store = ReadOnlyStore {
            slot: "selected-slot".into(),
            text: run.to_ron().expect("test run must encode"),
        };

        let refusal = load_resume_after_scenario(
            &store,
            "selected-slot",
            &loaded_versions,
            "assets/entities/alliance_cruiser.toml",
            &one_game_start_world(),
        )
        .expect_err("the full post-load content gate must refuse the run");
        assert!(matches!(
            refusal,
            BrowserResumeRefusal::Load(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Content { .. }
            ))
        ));
    }

    #[test]
    fn browser_resume_refuses_a_different_selected_hull_before_staging() {
        let current = vellum_save::Versions::new(7, "rules", 0x1234);
        let run = crate::snapshot::run_for(
            crate::snapshot::PhoenixSnapshot {
                tick: 42,
                boot_identity: Some(crate::snapshot::BootIdentity {
                    selected_ship: "assets/entities/alliance_destroyer.toml".into(),
                    fleet: crate::lockstep::FleetRoster::default(),
                    game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                        authored_index: 0,
                        entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                    }],
                }),
                ..Default::default()
            },
            0xfeed,
            17,
            "assets/worlds/default.toml",
            current.clone(),
        );
        let store = ReadOnlyStore {
            slot: "selected-slot".into(),
            text: run.to_ron().expect("test run must encode"),
        };

        let refusal = load_resume_after_scenario(
            &store,
            "selected-slot",
            &current,
            "assets/entities/alliance_cruiser.toml",
            &one_game_start_world(),
        )
        .expect_err("a different hull must never receive the saved component state");
        assert_eq!(
            refusal,
            BrowserResumeRefusal::WrongSelectedShip {
                saved: "assets/entities/alliance_destroyer.toml".into(),
                loaded: "assets/entities/alliance_cruiser.toml".into(),
            }
        );
    }

    #[test]
    fn an_imported_artifact_becomes_a_row_of_the_ordinary_catalogue() {
        // Issue #1363's AC2. The importer now sits in the save catalogue's
        // header, which makes importing an action ON that list — and an action
        // on a list that leaves the list unchanged is a control in the wrong
        // panel. So the file becomes a manual slot like any other, listed by
        // the same `list_slots` the catalogue reads, under the name the
        // operator's file had.
        let current = vellum_save::Versions::new(7, "rules", 0x1234);
        let artifact = portable_artifact("assets/worlds/default.toml", &current);
        let store = MapStore::default();

        let slot_id = import_artifact_into_catalogue(&store, &artifact, "away-team.ron")
            .expect("an intact artifact must enter the catalogue");

        let listed = crate::save_slots::list_slots(&store, &current)
            .expect("the catalogue must list what was just written");
        let row = listed
            .iter()
            .find(|entry| entry.slot_id == slot_id)
            .expect("the imported save must be a row of the ordinary catalogue");
        assert_eq!(row.kind, crate::save_slots::SaveSlotKind::Manual);
        assert_eq!(row.display_name, "away-team.ron");
        assert_eq!(
            row.record
                .as_ref()
                .expect("an imported row carries the run's own summary")
                .scenario,
            "assets/worlds/default.toml"
        );
        // ...and it is startable the moment it lands, which is what makes it a
        // row of this list rather than a private shelf beside it.
        assert!(matches!(row.start, crate::save_slots::StartState::Ready));
    }

    #[test]
    fn a_file_that_is_not_a_run_never_becomes_a_row() {
        // The one rule this path must not break, and the reason the helper is
        // pure: a catalogue row is a promise that something can be read back.
        let store = MapStore::default();
        let refusal = import_artifact_into_catalogue(&store, "not a save at all", "junk.txt")
            .expect_err("an unparsable file must not become a row");
        assert!(matches!(refusal, ImportSlotRefusal::Damaged(_)));
        assert!(refusal.to_string().starts_with("damaged\t"));
        assert!(
            vellum_save::Store::slots(&store)
                .expect("the fake store lists")
                .is_empty(),
            "a refused import must leave the Store untouched"
        );
    }

    #[test]
    fn a_store_that_will_not_take_it_is_a_different_answer_from_a_damaged_file() {
        // Two classes because they send a host to two different places: pick
        // another file, against make room. The page is not left to infer which
        // from an English sentence it may not paraphrase.
        let current = vellum_save::Versions::new(7, "rules", 0x1234);
        let artifact = portable_artifact("assets/worlds/default.toml", &current);
        let store = ReadOnlyStore {
            slot: "occupied".into(),
            text: String::new(),
        };
        let refusal = import_artifact_into_catalogue(&store, &artifact, "away-team.ron")
            .expect_err("a Store that refuses writes cannot make a row");
        assert!(matches!(refusal, ImportSlotRefusal::NotStored(_)));
        assert!(refusal.to_string().starts_with("not-stored\t"));
    }

    #[test]
    fn an_incompatible_artifact_still_enters_the_catalogue_and_is_refused_on_its_row() {
        // Compatibility is not an ENTRY gate, exactly as it is not a copy gate
        // on the way out (`wasm_export_save_slot`). It could not be one here:
        // the content dimension is a digest over the world the save names, and
        // a host standing at the catalogue has loaded no world. The refusal
        // arrives where #1363's AC3 asks for it — on the row — and the gate
        // that AC4 is about still runs before anything is restored.
        let saved = vellum_save::Versions::new(6, "rules", 0x1234);
        let current = vellum_save::Versions::new(7, "rules", 0x1234);
        let artifact = portable_artifact("assets/worlds/default.toml", &saved);
        let store = MapStore::default();

        let slot_id = import_artifact_into_catalogue(&store, &artifact, "older.ron")
            .expect("an intact artifact from another build still enters the list");
        let listed =
            crate::save_slots::list_slots(&store, &current).expect("the catalogue must list it");
        let row = listed
            .iter()
            .find(|entry| entry.slot_id == slot_id)
            .expect("the imported save is a row");
        assert!(
            matches!(
                row.start,
                crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(_))
            ),
            "the row says it cannot be started, and names the dimension that moved"
        );
        assert!(!row.can_start());
    }

    #[test]
    fn portable_import_applies_the_non_default_hull_gate_before_staging() {
        let current = vellum_save::Versions::new(7, "rules", 0x1234);
        let run = crate::snapshot::run_for(
            crate::snapshot::PhoenixSnapshot {
                tick: 42,
                boot_identity: Some(crate::snapshot::BootIdentity {
                    selected_ship: "assets/entities/alliance_cruiser.toml".into(),
                    fleet: crate::lockstep::FleetRoster::default(),
                    game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                        authored_index: 0,
                        entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                    }],
                }),
                ..Default::default()
            },
            0xfeed,
            17,
            "assets/worlds/default.toml",
            current.clone(),
        );
        let artifact = run.to_ron().expect("portable artifact must encode");
        let world = one_game_start_world();

        let accepted = import_resume_after_scenario(
            &artifact,
            &current,
            "assets/entities/alliance_cruiser.toml",
            &world,
        )
        .expect("the artifact's non-default saved hull must pass unchanged");
        assert_eq!(
            crate::snapshot::required_boot_identity(&accepted)
                .expect("accepted import keeps boot identity")
                .selected_ship,
            "assets/entities/alliance_cruiser.toml"
        );

        let refusal = import_resume_after_scenario(
            &artifact,
            &current,
            "assets/entities/alliance_destroyer.toml",
            &world,
        )
        .expect_err("a portable import must not bypass the selected-hull gate");
        assert_eq!(
            refusal,
            BrowserResumeRefusal::WrongSelectedShip {
                saved: "assets/entities/alliance_cruiser.toml".into(),
                loaded: "assets/entities/alliance_destroyer.toml".into(),
            }
        );

        let mut world_with_unmapped_npc = world.clone();
        let mut npc_row = world_with_unmapped_npc.entities[0].clone();
        npc_row.id = Some("game-start-npc".into());
        world_with_unmapped_npc.entities.push(npc_row);
        assert!(matches!(
            import_resume_after_scenario(
                &artifact,
                &current,
                "assets/entities/alliance_cruiser.toml",
                &world_with_unmapped_npc,
            ),
            Err(BrowserResumeRefusal::Load(
                crate::snapshot::LoadRefusal::Unparsable(_)
            ))
        ));

        let mut out_of_bounds = run;
        out_of_bounds
            .snapshot
            .as_mut()
            .unwrap()
            .state
            .boot_identity
            .as_mut()
            .unwrap()
            .game_start_entity_uuids[0]
            .authored_index = 1;
        let artifact = out_of_bounds.to_ron().unwrap();
        assert!(matches!(
            import_resume_after_scenario(
                &artifact,
                &current,
                "assets/entities/alliance_cruiser.toml",
                &world,
            ),
            Err(BrowserResumeRefusal::Load(
                crate::snapshot::LoadRefusal::Unparsable(_)
            ))
        ));
    }

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
                host_channels::GM_ENTITY,
                host_channels::GM_ACTIVITY,
                host_channels::GM_STATION,
                host_channels::GM_SESSION,
                host_channels::GM_MISSION,
                host_channels::GM_SPAWN,
                host_channels::GM_COMMS,
            ]
        );
    }

    /// A connected phone's bridge batch reaches the same catalogue adapter as
    /// the host route, while Pause stays untouched and duplicate diagnostic
    /// toggles still collapse to one flip.
    #[cfg(not(phoenix_demo_build))]
    #[test]
    fn phone_bridge_batch_applies_the_named_surface_and_not_pause() {
        use crate::core::debug_surface::DebugSurface;
        use crate::core::messages::ClientMessage;
        use crate::lobby::{InboundMessage, Sessions};
        use bevy::prelude::{App, Messages, Update};

        let mut app = App::new();
        app.add_message::<InboundMessage>();
        app.init_resource::<crate::debug_overlay::DebugRegionsEnabled>();
        app.init_resource::<crate::debug_overlay::DebugOverlayEnabled>();
        app.init_resource::<crate::debug_overlay::SimulationPaused>();
        app.init_resource::<crate::debug_overlay::DebugDamageEnabled>();
        app.init_resource::<crate::debug_overlay::DebugEntitiesEnabled>();
        app.init_resource::<crate::debug_overlay::DebugEntityInspectorEnabled>();
        app.init_resource::<crate::debug::DebugStationActivityEnabled>();
        app.init_resource::<crate::debug::DebugAiDoctrineEnabled>();
        app.init_resource::<crate::debug::DebugScenarioStateEnabled>();
        app.init_resource::<crate::debug::DebugConsoleLatencyEnabled>();
        let mut sessions = crate::lobby::session::SessionManager::new();
        sessions
            .register("phone".into(), "Tester".into())
            .expect("register connected phone");
        app.insert_resource(Sessions(sessions));
        app.add_systems(Update, super::drain_client_debug_flags);
        for _ in 0..2 {
            app.world_mut()
                .resource_mut::<Messages<InboundMessage>>()
                .write(InboundMessage {
                    token: "phone".into(),
                    msg: ClientMessage::ToggleDebugFlag {
                        flag: DebugSurface::Damage,
                    },
                });
        }
        app.update();

        assert!(
            app.world()
                .resource::<crate::debug_overlay::DebugDamageEnabled>()
                .0
        );
        assert!(
            !app.world()
                .resource::<crate::debug_overlay::SimulationPaused>()
                .0
        );
    }

    // ── Instagib queue-drain semantics (issue #1181) ────────────────────────

    /// Draining an empty instagib queue leaves the flag untouched — the frame
    /// after a drain, with nothing queued, must not re-flip.
    #[test]
    fn draining_no_instagib_toggles_leaves_the_flag() {
        let mut on = false;
        apply_instagib_toggles(0, &mut on);
        assert!(!on, "no queued toggles must not flip");

        let mut already_on = true;
        apply_instagib_toggles(0, &mut already_on);
        assert!(already_on, "no queued toggles must preserve an on flag");
    }

    /// One queued toggle flips the flag exactly once.
    #[test]
    fn one_instagib_toggle_flips_once() {
        let mut on = false;
        apply_instagib_toggles(1, &mut on);
        assert!(on, "one toggle: false -> true");

        apply_instagib_toggles(1, &mut on);
        assert!(!on, "one toggle again: true -> false");
    }

    /// The queue is a COUNT, so its parity decides the net flip — two clicks in
    /// one frame cancel, three land as one, matching what the same clicks spread
    /// over separate ticks would do (the God Mode drain's contract).
    #[test]
    fn instagib_toggle_count_applies_by_parity() {
        let mut on = false;
        apply_instagib_toggles(2, &mut on);
        assert!(!on, "two toggles in one frame cancel");

        apply_instagib_toggles(3, &mut on);
        assert!(on, "three toggles net to one flip");

        apply_instagib_toggles(4, &mut on);
        assert!(on, "four toggles net to no change");
    }

    /// The drain's shape end to end: the `Instagib` Resource starts off, a queued
    /// batch flips it, and a mirror read reflects the Resource — the same round
    /// trip `drain_instagib_toggle` + `publish_instagib` perform on wasm, minus
    /// the thread-local edge.
    #[test]
    fn instagib_resource_round_trips_a_drained_batch() {
        let mut instagib = Instagib::default();
        assert!(!instagib.0, "starts off");

        // One frame's queue of a single click.
        apply_instagib_toggles(1, &mut instagib.0);
        assert_eq!(instagib, Instagib(true), "a click turns it on");

        // A frame that queued nothing must leave it on.
        apply_instagib_toggles(0, &mut instagib.0);
        assert_eq!(instagib, Instagib(true), "an empty frame preserves it");
    }

    #[test]
    fn browser_save_queue_is_bounded_fifo_and_rejected_work_never_drains() {
        let mut pending = PendingBrowserSaves::new();
        for index in 0..MAX_PENDING_BROWSER_SAVES {
            assert_eq!(
                pending.try_push(format!("token-{index}"), index),
                Ok(()),
                "every request through the finite capacity is accepted"
            );
        }

        assert_eq!(
            pending.try_push("rejected".to_string(), usize::MAX),
            Err(usize::MAX),
            "the first request beyond the bound is refused"
        );
        let requests = pending.take_requests();
        assert_eq!(requests.len(), MAX_PENDING_BROWSER_SAVES);
        assert_eq!(requests.front().map(String::as_str), Some("token-0"));
        let expected_last = format!("token-{}", MAX_PENDING_BROWSER_SAVES - 1);
        assert_eq!(
            requests.back().map(String::as_str),
            Some(expected_last.as_str()),
            "accepted requests retain FIFO order"
        );
        assert!(
            !requests.iter().any(|token| token == "rejected"),
            "a refused call must never reach the fixed-boundary snapshot drain"
        );

        assert_eq!(
            pending.try_push("still-full".to_string(), usize::MAX),
            Err(usize::MAX),
            "taking ingress does not release an in-flight intent"
        );
        assert_eq!(pending.remove_intent("token-0"), Some(0));
        assert_eq!(
            pending.remove_intent("token-0"),
            None,
            "a fixed-boundary refusal can clear and report an intent only once"
        );
        assert_eq!(
            pending.try_push("recovered".to_string(), usize::MAX),
            Ok(())
        );
        assert_eq!(
            pending.take_requests().into_iter().collect::<Vec<_>>(),
            vec!["recovered"],
            "draining one result releases exactly one request slot"
        );
    }

    #[test]
    fn browser_save_namespace_accepts_only_canonical_peer_identity() {
        let identity = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            scoped_browser_save_namespace(identity).as_deref(),
            Some("phoenix:0123456789abcdef0123456789abcdef")
        );

        for invalid in [
            "",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef:123456789abcdef",
        ] {
            assert_eq!(
                scoped_browser_save_namespace(invalid),
                None,
                "an unvalidated local value must not choose a Store namespace"
            );
        }
    }

    #[test]
    fn browser_status_outbox_keeps_newest_bound_and_recovers_after_poll() {
        let mut statuses = BoundedFifo::<_, 3>::new();
        statuses.push_back("oldest");
        statuses.push_back("second");
        statuses.push_back("third");
        statuses.push_back("queue full refusal");

        assert_eq!(statuses.len(), 3);
        assert_eq!(statuses.pop_front(), Some("second"));
        assert_eq!(statuses.pop_front(), Some("third"));
        assert_eq!(
            statuses.pop_front(),
            Some("queue full refusal"),
            "the newest overload status remains visible and retained values stay FIFO"
        );
        assert_eq!(statuses.pop_front(), None);

        statuses.push_back("after recovery");
        assert_eq!(statuses.pop_front(), Some("after recovery"));
    }
}
