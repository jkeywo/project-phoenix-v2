use bevy::prelude::*;
use std::collections::VecDeque;

use crate::core::messages::{
    ClientMessage, DeliveryClass, GamePhase, GameState, ServerMessage, ShipClientConfig, WorldData,
};
use crate::lobby::handler;
use crate::lobby::handler::CountdownAction;
pub use crate::lobby::handler::Target;
use crate::lobby::session::SessionManager;
use crate::lobby::stations_config::{stations_from_ship_config, ShipStations};
use crate::ship::rating;
use crate::ship_plugin::{
    load_ship_config_from_disk, ActiveStationRatings, PendingShipConfig, ShipConfigComponent,
    ShipSystemControlSources,
};

/// Server-authoritative pre-game countdown. When `remaining_secs > 0.0` the
/// lobby is counting down and `pending_phase` is the target after the timer
/// expires. Anyone unreadying, disconnecting, or a new player joining resets
/// this timer (via `CountdownAction::Cancel`).
#[derive(Resource)]
pub struct CountdownTimer {
    pub remaining_secs: f32,
    pub pending_phase: Option<GamePhase>,
    /// False while the browser host mesh owns collective start policy. Local
    /// `SetReady` handlers may still update crew state, but cannot arm this
    /// ship's independent countdown.
    local_start_allowed: bool,
}

impl Default for CountdownTimer {
    fn default() -> Self {
        CountdownTimer {
            remaining_secs: 0.0,
            pending_phase: None,
            local_start_allowed: true,
        }
    }
}

/// Coordinated lobby state set by the privileged browser host mesh.
///
/// Enabling fails closed (`validation_passed = false`) until the page publishes
/// its current content/peer validation result. Disabling restores the ordinary
/// single-host lobby behavior.
#[derive(Resource, Clone, Debug)]
pub struct FleetManagedLobby {
    pub enabled: bool,
    pub validation_passed: bool,
}

impl Default for FleetManagedLobby {
    fn default() -> Self {
        Self {
            enabled: false,
            validation_passed: true,
        }
    }
}

impl FleetManagedLobby {
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled && !self.enabled {
            self.validation_passed = false;
        } else if !enabled {
            self.validation_passed = true;
        }
        self.enabled = enabled;
    }
}

pub const MAX_PENDING_START_GRANTS: usize = 32;
pub const MAX_START_GRANT_RESULTS: usize = 32;

#[derive(Resource, Default)]
pub struct PendingStartGrants(VecDeque<crate::lobby::start_policy::StartGrant>);

impl PendingStartGrants {
    pub fn try_push(&mut self, grant: crate::lobby::start_policy::StartGrant) -> bool {
        if self.0.len() >= MAX_PENDING_START_GRANTS {
            return false;
        }
        self.0.push_back(grant);
        true
    }

    fn pop_front(&mut self) -> Option<crate::lobby::start_policy::StartGrant> {
        self.0.pop_front()
    }

    fn requeue(&mut self, grant: crate::lobby::start_policy::StartGrant) {
        debug_assert!(self.0.len() < MAX_PENDING_START_GRANTS);
        self.0.push_back(grant);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.0.len() >= MAX_PENDING_START_GRANTS
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Ensure the technical owner's authenticated decision is in the queue.
    /// An identical early browser proposal already occupies the same logical
    /// slot; otherwise an untrusted tail proposal is displaced if necessary so
    /// queue pressure cannot make one peer drop the authoritative boundary.
    pub fn adopt_canonical(&mut self, grant: crate::lobby::start_policy::StartGrant) {
        if self.0.iter().any(|queued| queued == &grant) {
            return;
        }
        if self.0.len() == MAX_PENDING_START_GRANTS {
            self.0.pop_back();
        }
        self.0.push_front(grant);
    }
}

/// A second, different canonical start decision was proven for the same fleet
/// generation — the one condition [`StartGrantTracker::adopt_canonical`] can
/// refuse, and the whole of what its `Err` means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConflictingCanonicalGrant;

#[derive(Resource, Default)]
pub struct StartGrantTracker {
    last_sequence: u64,
    canonical: Option<crate::lobby::start_policy::StartGrant>,
    embedded: bool,
    failed_closed: bool,
}

impl StartGrantTracker {
    pub fn reset(&mut self) {
        self.last_sequence = 0;
        self.canonical = None;
        self.embedded = false;
        self.failed_closed = false;
    }

    /// Adopt the decision proven by the technical owner's authenticated tick
    /// frame (or the local technical owner immediately before sealing it).
    /// There is exactly one immutable decision per fleet generation.
    pub fn adopt_canonical(
        &mut self,
        grant: &crate::lobby::start_policy::StartGrant,
    ) -> Result<bool, ConflictingCanonicalGrant> {
        match self.canonical.as_ref() {
            None => {
                self.canonical = Some(grant.clone());
                Ok(true)
            }
            Some(canonical) if canonical == grant => Ok(false),
            Some(_) => Err(ConflictingCanonicalGrant),
        }
    }

    pub fn is_canonical(&self, grant: &crate::lobby::start_policy::StartGrant) -> bool {
        self.canonical.as_ref() == Some(grant)
    }

    pub fn mark_embedded(&mut self) {
        self.embedded = true;
    }

    pub fn is_embedded(&self) -> bool {
        self.embedded
    }

    pub fn fail_closed(&mut self) {
        self.failed_closed = true;
    }

    pub fn is_failed_closed(&self) -> bool {
        self.failed_closed
    }
}

#[derive(Resource, Default)]
pub struct StartGrantResults(VecDeque<crate::lobby::start_policy::StartGrantResult>);

impl StartGrantResults {
    pub fn push(&mut self, result: crate::lobby::start_policy::StartGrantResult) {
        if self.0.len() == MAX_START_GRANT_RESULTS {
            self.0.pop_front();
        }
        self.0.push_back(result);
    }

    pub fn drain(
        &mut self,
    ) -> impl Iterator<Item = crate::lobby::start_policy::StartGrantResult> + '_ {
        self.0.drain(..)
    }

    /// Observe typed grant outcomes before the browser bridge destructively
    /// drains them. The GM activity publisher uses this fan-out seam so an
    /// operational Force Start result cannot race its existing host callback.
    pub fn iter(&self) -> impl Iterator<Item = &crate::lobby::start_policy::StartGrantResult> {
        self.0.iter()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[derive(Clone, Debug)]
pub enum FleetLobbyInput {
    Managed(bool),
    Validation(bool),
    Grant(crate::lobby::start_policy::StartGrant),
}

/// Apply ordered browser edge input without collapsing a fleet generation.
/// Returns true when at least one managed-mode transition reset the generation.
/// If the fixed-tick grant queue is full, the blocked grant and every later
/// input remain at the front of `inputs` for a later frame.
pub fn apply_fleet_lobby_inputs(
    inputs: &mut VecDeque<FleetLobbyInput>,
    managed: &mut FleetManagedLobby,
    grants: &mut PendingStartGrants,
    tracker: &mut StartGrantTracker,
    results: &mut StartGrantResults,
) -> bool {
    let mut generation_changed = false;
    while let Some(input) = inputs.pop_front() {
        match input {
            FleetLobbyInput::Managed(enabled) => {
                if managed.enabled != enabled {
                    grants.clear();
                    tracker.reset();
                    results.clear();
                    generation_changed = true;
                }
                managed.set_enabled(enabled);
            }
            FleetLobbyInput::Validation(valid) => {
                managed.validation_passed = valid;
            }
            FleetLobbyInput::Grant(grant) => {
                if grants.is_full() {
                    inputs.push_front(FleetLobbyInput::Grant(grant));
                    break;
                }
                let queued = grants.try_push(grant);
                debug_assert!(queued);
            }
        }
    }
    generation_changed
}

/// Cached `GameState` snapshot derived from `Sessions` + `GamePhase` each frame.
/// Renderer systems read this instead of accessing `Sessions` directly.
#[derive(Resource, Clone)]
pub struct GameStateCache(pub GameState);

/// Pending outbound messages produced by lobby systems.
/// Drained each frame by `drain_lobby_outbox`, which runs unconditionally so
/// messages queued on the Lobby→InProgress transition frame (e.g. GameStarted)
/// are not lost.
#[derive(Resource, Default)]
pub struct LobbyOutbox(pub Vec<(Target, ServerMessage)>);

// ── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct Sessions(pub SessionManager);

/// Bevy resource wrapping the per-ship client config sent in `Welcome`.
/// Populated from the loaded ship TOML by `update_session_with_config`.
///
/// **Legitimately player-only.** This is the subset of ship config that
/// the browser client needs (radar range, chart range, target radii, etc.).
/// Only the LocalShip has a browser client, so a single Resource is
/// sufficient — NPCs do not have consoles that need this data. The full
/// per-ship config lives on the `ShipConfigComponent` per entity.
#[derive(Resource, Default)]
pub struct ShipClientConfigResource(pub ShipClientConfig);

/// Bevy resource wrapping the read-only ship manual replicated to the client
/// (issue #772). Built from the selected ship's config by
/// `update_session_with_config` — the same seam as `ShipClientConfigResource` —
/// and published per-client as a dedicated `ServerMessage::ShipManual` right
/// after `Welcome`. Client-side it is presentation state only.
#[derive(Resource, Default)]
pub struct ShipManualResource(pub crate::ship::manual::ShipManualWire);

/// Server's authoritative copy of the world layout — populated once during
/// world setup and broadcast to clients via `WorldSetup` after `StartGame`,
/// and replayed inside `Welcome` for mid-game reconnects.
#[derive(Resource, Clone, Default)]
pub struct WorldResource(pub WorldData);

/// Template path of the player ship selected during the host first screen.
/// Set by JS via `wasm_select_ship` before `wasm_init`. Defaults to
/// `"assets/entities/alliance_cruiser.toml"` for legacy worlds that don't
/// expose an `available_ships` list.
#[derive(Resource, Clone)]
pub struct SelectedShipResource(pub String);

// ── Messages (Bevy 0.18 pull-based message system) ─────────────────────────

/// A decoded ClientMessage received from one peer, tagged with the sender's
/// session token.
#[derive(Message, Clone)]
pub struct InboundMessage {
    pub token: String,
    pub msg: ClientMessage,
}

/// A lifecycle event signalled by the transport layer when a peer disconnects.
#[derive(Message, Clone)]
pub struct PlayerDisconnected {
    pub token: String,
}

/// A ServerMessage to be forwarded to one or all peers by the JS bridge.
#[derive(Message, Clone)]
pub struct OutboundMessage {
    pub target: Target,
    pub msg: ServerMessage,
    pub delivery: DeliveryClass,
}

// ── System set ─────────────────────────────────────────────────────────────

/// Ordering anchor for every lobby system (in `FixedUpdate` since issue #895):
/// `handle_disconnect` runs first, then the per-variant message systems
/// (Identify / SetName / ReturnToLobby plus the four station-management
/// systems), then `tick_countdown → update_game_state_cache`. Downstream
/// systems that must observe the post-lobby world state order themselves with
/// `.after(LobbySystemSet)` — which is why they share its schedule.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LobbySystemSet;

/// Project the selected ship's resolved authored System topology onto the
/// public instance-id -> kind map used by client command surfaces.
///
/// The include resolver has already composed the selected entity before this
/// seam sees it, so walking `systems` covers every live `[[system]]` entry
/// without teaching the browser any instance-id naming convention.
fn project_system_kinds(
    systems: &[crate::ship::config::SystemInstanceConfig],
) -> std::collections::HashMap<String, String> {
    systems
        .iter()
        .map(|system| (system.id.0.clone(), system.kind.clone()))
        .collect()
}

// ── Plugin ─────────────────────────────────────────────────────────────────

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        {
            use crate::authoritative::{DeclareState, StateClass};
            // Host/session membership, not simulation state. Like FleetRoster,
            // this says who is connected where; any later typed GM command is
            // the value that crosses into the deterministic world.
            app.declare_state::<crate::gm_roster::GmRoster>(
                StateClass::Timer,
                "gm-operator-admission-and-presence",
            )
            .declare_state::<FleetManagedLobby>(
                StateClass::Timer,
                "gm-ready-and-force-start-policy",
            )
            .declare_state::<PendingStartGrants>(
                StateClass::Timer,
                "gm-ready-and-force-start-policy",
            )
            .declare_state::<StartGrantTracker>(
                StateClass::Timer,
                "gm-ready-and-force-start-policy",
            )
            .declare_state::<StartGrantResults>(
                StateClass::Timer,
                "gm-ready-and-force-start-policy",
            );
        }
        if !app.is_plugin_added::<bevy::state::app::StatesPlugin>() {
            app.add_plugins(bevy::state::app::StatesPlugin);
        }
        let initial_cache = GameStateCache(GameState {
            phase: GamePhase::Lobby,
            players: vec![],
            world: None,
        });
        app.insert_resource(Sessions(SessionManager::new()))
            .insert_resource(initial_cache)
            .insert_resource(LobbyOutbox::default())
            .init_resource::<crate::gm_roster::GmRoster>()
            .init_resource::<FleetManagedLobby>()
            .init_resource::<PendingStartGrants>()
            .init_resource::<StartGrantTracker>()
            .init_resource::<StartGrantResults>()
            .insert_resource(ShipClientConfigResource::default())
            .insert_resource(ShipManualResource::default())
            .init_resource::<ShipStations>()
            .init_resource::<CountdownTimer>()
            .init_state::<GamePhase>()
            .add_message::<InboundMessage>()
            .add_message::<OutboundMessage>()
            .add_message::<PlayerDisconnected>()
            .add_systems(Startup, update_session_with_config)
            // Ordering scaffold (replaces the former monolithic `process_lobby`
            // chain). `handle_disconnect` must run before the per-variant message
            // systems so that when a stale disconnect and the reconnect `Identify`
            // land in the same frame (a browser refresh), the seat is vacated+saved
            // first and then restored — not the reverse, which would leave the
            // player marked disconnected with their seat cleared. `tick_countdown`
            // runs after the message systems (but before the outbox drain) so
            // countdown broadcasts reach the outbound bus.
            //
            // `FixedUpdate` since issue #895: the `SimSet` chain lives in the
            // fixed schedule and orders itself `.after(LobbySystemSet)`, an
            // edge that is only real when both sides share a schedule. Lobby
            // handling therefore advances on the logical tick too — inbound
            // messages drained in `PreUpdate` are buffered by Bevy until a
            // fixed step has observed them, and `tick_countdown`'s `Res<Time>`
            // reads the fixed clock, which is what drives the
            // `GamePhase::InProgress` transition on tick time.
            .add_systems(
                FixedUpdate,
                (
                    enforce_fleet_managed_countdown,
                    handle_disconnect,
                    tick_countdown,
                    apply_pending_start_grants,
                    update_game_state_cache,
                )
                    .chain()
                    .in_set(LobbySystemSet),
            )
            // Per-variant message systems (issues #733 + #734). Each owns exactly
            // one ClientMessage variant, reading it via its own `MessageReader`
            // cursor. They replace the monolithic `process_lobby`/`process_message`
            // dispatch. All run after `handle_disconnect` and before
            // `tick_countdown`. They carry different phase gates, so they are
            // registered per matching gate:
            //
            // Identify + the four station systems gate on
            // Lobby/Loading/InProgress (claim/release/toggle + mid-game reconnect).
            .add_systems(
                FixedUpdate,
                (
                    handle_identify_system,
                    handle_select_station_system,
                    handle_release_station_system,
                    handle_set_ready_system,
                    handle_set_spectator_system,
                    handle_set_afk_system,
                    handle_set_station_rating_system,
                    handle_report_station_eligibility_system,
                )
                    .in_set(LobbySystemSet)
                    .after(handle_disconnect)
                    .before(tick_countdown)
                    .run_if(
                        in_state(GamePhase::Lobby)
                            .or(in_state(GamePhase::Loading))
                            .or(in_state(GamePhase::InProgress)),
                    ),
            )
            // SetName gates on Lobby/Loading (rename before the game starts).
            .add_systems(
                FixedUpdate,
                handle_set_name_system
                    .in_set(LobbySystemSet)
                    .after(handle_disconnect)
                    .before(tick_countdown)
                    .run_if(in_state(GamePhase::Lobby).or(in_state(GamePhase::Loading))),
            )
            // ReturnToLobby runs in GameOver (the game-over screen's button)
            // AND in InProgress (the host settings cog's "exit to lobby",
            // issue #939, which aborts a running mission). Both phases have to
            // be registered here or the second is dead on arrival: the
            // sender-authority gate inside
            // `handler::handle_return_to_lobby` never gets a chance to
            // run in a phase this `run_if` filters out. That gate is what keeps
            // the mid-mission abort host-only — this registration is only about
            // which phases the system is allowed to look at the message in.
            .add_systems(
                PreUpdate,
                handle_return_to_lobby_system
                    .before(crate::lockstep::MeshSet)
                    .run_if(in_state(GamePhase::GameOver).or(in_state(GamePhase::InProgress))),
            );
    }
}

/// Project one resolved entity template into the exact static config sent by
/// ordinary `Welcome`. GM Station iframes call this same pure seam, so
/// authored console ranges, filters, arcs, tutorials, hull identity and assist
/// gaps cannot drift into a smaller GM-only schema.
pub(crate) fn project_ship_client_config(
    ship_config: &crate::entities::config::EntityConfig,
) -> ShipClientConfig {
    // Build the client-facing ship config from the same source-of-truth.
    // `HelmConsoleConfig::effective_radar_range()` prefers the structured
    // [helm_console.radar] range when present, falling back to the legacy
    // flat radar_range field, then to the Default.
    let mut next = ShipClientConfig::default();
    if let Some(hc) = &ship_config.helm_console {
        let range = hc.effective_radar_range();
        if range > 0.0 {
            next.helm_radar_range = range;
        }
        // Push the configured impulse charge duration to the client so
        // the helm progress bar advances at the same rate the server
        // is ticking.
        next.impulse_charge_duration = hc.impulse_charge_duration;
        // Red-alert hostile weapon-arc overlay colour (issue #874). Same
        // "exactly four entries or keep the default" shape as
        // `torpedo_arc_color` below.
        if hc.hostile_arc_color.len() == 4 {
            next.hostile_arc_color = [
                hc.hostile_arc_color[0],
                hc.hostile_arc_color[1],
                hc.hostile_arc_color[2],
                hc.hostile_arc_color[3],
            ];
        }
    }
    // [repair] block — pushes repair-team timings to the client so the
    // Repair panel can derive its progress-bar durations without knowing
    // server-side constants. Absent block keeps defaults that match the
    // historical hardcoded constants.
    if let Some(rc) = &ship_config.repair {
        if rc.repair_team_count > 0 {
            next.repair_team_count = rc.repair_team_count as u8;
        }
        next.repair_travel_secs = rc.travel_duration_secs;
        next.repair_rate_hp_per_sec = rc.repair_rate_hp_per_sec;
    }
    // [weapons_console] — push phaser banks (id/facing/fire_arc/cooldown
    // only; auto_arc_deg stays server-side) and the beam/arc colours so
    // the Tactical UI can render fire arcs, colour fire buttons, and
    // size the per-bank cooldown bar.
    if let Some(wc) = &ship_config.weapons_console {
        next.phaser_banks = wc
            .phaser_banks
            .iter()
            .map(|b| crate::core::messages::PhaserBankClientConfig {
                id: b.id.clone(),
                facing_deg: b.facing_deg,
                fire_arc_deg: b.fire_arc_deg,
                // Mirror the server's "zero means absent" fallback so
                // the client always sees the real cooldown duration.
                cooldown_secs: if b.cooldown_secs > 0.0 {
                    b.cooldown_secs
                } else {
                    crate::entities::config::PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS
                },
            })
            .collect();
        let empty_color: Vec<f32> = vec![];
        let beam_color_src = wc
            .phaser_banks
            .first()
            .map(|b| &b.beam_color)
            .unwrap_or(&empty_color);
        if beam_color_src.len() == 4 {
            next.phaser_beam_color = [
                beam_color_src[0],
                beam_color_src[1],
                beam_color_src[2],
                beam_color_src[3],
            ];
        }
        if wc.torpedo_arc_color.len() == 4 {
            next.torpedo_arc_color = [
                wc.torpedo_arc_color[0],
                wc.torpedo_arc_color[1],
                wc.torpedo_arc_color[2],
                wc.torpedo_arc_color[3],
            ];
        }
    }
    // [torpedoes] — per-tube layout (id/facing/fire_arc).
    if let Some(tc) = &ship_config.torpedoes {
        next.torpedo_tubes = tc
            .tubes
            .iter()
            .map(|t| crate::core::messages::TorpedoTubeClientConfig {
                id: t.id.clone(),
                facing_deg: t.facing_deg,
                fire_arc_deg: t.fire_arc_deg,
            })
            .collect();
    }
    // [weapons_console.blaster_banks] — per-bank layout (id/facing/fire_arc/cooldown).
    // Mirrors the phaser "zero means absent" fallback so clients always see the real
    // cooldown duration. Default cooldown is 3.0 s (matches BlasterBankConfig default).
    if let Some(wc) = &ship_config.weapons_console {
        next.blaster_banks = wc
            .blaster_banks
            .iter()
            .map(|b| crate::core::messages::BlasterBankClientConfig {
                id: b.id.clone(),
                facing_deg: b.facing_deg,
                fire_arc_deg: b.fire_arc_deg,
                cooldown_secs: if b.cooldown_secs > 0.0 {
                    b.cooldown_secs
                } else {
                    3.0
                },
            })
            .collect();
    }
    // Radar shows lists — push the TOML-configured tag filters to the
    // client so each console widget can build its RadarFilter without
    // hardcoding tag names.
    if let Some(hc) = &ship_config.helm_console {
        if let Some(r) = &hc.radar {
            next.helm_radar_shows = r.shows.iter().map(|t| t.as_str().to_string()).collect();
        }
    }
    if let Some(sc) = &ship_config.sensors_console {
        next.sensors_radar_range = sc.long_range_radar.range;
        next.sensors_radar_shows = sc
            .long_range_radar
            .shows
            .iter()
            .map(|t| t.as_str().to_string())
            .collect();
        next.sensors_radar_selects = sc
            .long_range_radar
            .selects
            .iter()
            .map(|t| t.as_str().to_string())
            .collect();
    }
    if let Some(nc) = &ship_config.navigation_console {
        next.nav_chart_shows = nc
            .system_chart
            .shows
            .iter()
            .map(|t| t.as_str().to_string())
            .collect();
        next.nav_chart_selects = nc
            .system_chart
            .selects
            .iter()
            .map(|t| t.as_str().to_string())
            .collect();
        if nc.system_chart.range > 0.0 {
            next.nav_chart_range = nc.system_chart.range;
        }
    }
    if let Some(wc) = &ship_config.weapons_console {
        if let Some(r) = &wc.radar {
            next.tactical_radar_shows = r.shows.iter().map(|t| t.as_str().to_string()).collect();
            next.tactical_radar_selects =
                r.selects.iter().map(|t| t.as_str().to_string()).collect();
            next.tactical_radar_range = r.range;
        }
    }
    // Ship identity metadata — class, hull_id, power_rating, css.
    next.class = ship_config.class.clone();
    next.hull_id = ship_config.hull_id.clone();
    next.power_rating = ship_config.power_rating;
    next.ship_css = ship_config.css.clone();
    // Station→system membership map: lets the client aggregate per-station
    // hull without knowing the ship layout. Iterate the stations block of
    // the TOML and collect system ids per station.
    if let Some(sc) = ship_config.ship_config.as_ref() {
        next.station_systems = sc
            .stations
            .iter()
            .map(|station| {
                let system_ids = sc
                    .systems_for_station(&station.id)
                    .map(|sys| sys.id.0.clone())
                    .collect();
                (station.id.0.clone(), system_ids)
            })
            .collect();
        // Authoritative System instance -> Console Family projection plus
        // the separate reserved/aggregate blackboard-key presentation map.
        // The second map is intentionally not folded into the first: those
        // keys share a wire wrapper but have no System command authority.
        let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
            .expect("the built-in System descriptor registry must be valid");
        next.system_console_families = registry.project_console_families(&sc.systems);
        next.system_kinds = project_system_kinds(&sc.systems);
        next.blackboard_console_families = registry.project_blackboard_console_families();
        // Anonymous accessibility eligibility projection (issue #1103):
        // per station → per rating → the T1 assist-functions the station
        // would force its holder to operate manually at that rating. Derived
        // purely from hull topology + rating automation
        // (`eligibility::projected_assist_gaps`) so the client runs the SAME
        // rule locally without any private profile leaving the device. Only
        // stations with a non-empty gap map are carried.
        next.station_assist_gaps = sc
            .stations
            .iter()
            .map(|station| {
                (
                    station.id.0.clone(),
                    crate::ship::eligibility::projected_assist_gaps(station, sc),
                )
            })
            .filter(|(_, gaps)| !gaps.is_empty())
            .collect();
        // Contextual tutorial overlays (issue #916): carry every station's
        // authored `[[station.tutorial]]` blocks to the client verbatim.
        // Generic iteration — no station-specific branches; the client's
        // tutorial state-builder owns the trigger vocabulary.
        next.station_tutorials = sc
            .stations
            .iter()
            .filter(|station| !station.tutorials.is_empty())
            .map(|station| (station.id.0.clone(), station.tutorials.clone()))
            .collect();
    }
    // Helm capability fields — sourced from [helm_capability] if present.
    // helm_systems: all system ids owned by the helm station.
    if let Some(sc) = ship_config.ship_config.as_ref() {
        let helm_station_id = crate::core::messages::StationId("helm".into());
        next.helm_systems = sc
            .systems_for_station(&helm_station_id)
            .map(|sys| sys.id.0.clone())
            .collect();
    }
    if let Some(cap) = &ship_config.helm_capability {
        next.vertical_movement_mode = match cap.vertical_movement_mode {
            crate::entities::config::VerticalMovementMode::Planar => "planar".to_string(),
            crate::entities::config::VerticalMovementMode::Bounded => "bounded".to_string(),
            crate::entities::config::VerticalMovementMode::Full3D => "full_3d".to_string(),
        };
        next.impulse_steering_multiplier = cap.impulse.steering_multiplier;
    }
    next
}

/// Update the Sessions resource with available consoles from the ship's EntityConfig.
pub(crate) fn update_session_with_config(
    mut ship_stations: ResMut<ShipStations>,
    mut ship_client_config: ResMut<ShipClientConfigResource>,
    mut ship_manual: ResMut<ShipManualResource>,
    pending_ship_config: Option<Res<PendingShipConfig>>,
    selected_ship: Option<Res<SelectedShipResource>>,
    browser_gm: Option<Res<crate::gm_projection::BrowserGameMaster>>,
) {
    // An explicit rendererless GM peer owns no local ship and therefore no
    // station/manual config. In particular, do not take the native filesystem
    // fallback below: browser GM boot deliberately skips ship selection.
    if browser_gm.is_some() {
        return;
    }
    let ship_config_resource = if let Some(pending) = pending_ship_config {
        ShipConfigComponent(pending.0.clone())
    } else {
        load_ship_config_from_disk()
    };
    if ship_stations.stations.is_empty() {
        *ship_stations = stations_from_ship_config(&ship_config_resource.0);
    }

    // Use the selected ship path (from available_ships) or fall back to the
    // legacy default for worlds without an `available_ships` list.
    let config_path = selected_ship
        .as_ref()
        .map(|s| s.0.as_str())
        .unwrap_or("assets/entities/alliance_cruiser.toml");

    if let Some(ship_config) = crate::entities::config_cache::get_config_cache().get(config_path) {
        ship_client_config.0 = project_ship_client_config(ship_config);

        // Ship manual (issue #772): build the read-only per-station manual from
        // the same selected-ship config that feeds the client above. Generated
        // system sections need a few values that live outside the station/system
        // topology — for shields, `[shields_console.base] max_hp/regen` — so
        // extract those into `system_extras` keyed by system kind for the
        // kind-keyed providers to read. The aggregator itself is pure.
        if let Some(topology) = ship_config.ship_config.as_ref() {
            let system_extras = build_manual_system_extras(ship_config);
            let registry = crate::ship::manual::ManualProviderRegistry::with_shipped_providers();
            ship_manual.0 =
                crate::ship::manual::build_ship_manual(topology, &registry, &system_extras);
        }
    }
}

/// Extract the kind-keyed `system_extras` the manual providers need from the
/// selected ship's `EntityConfig` (issue #773). The pure `ship::manual` module
/// sees only the station/system topology, so every vessel-specific gameplay
/// value a provider emits — weapon ranges, torpedo capacity, sensor range,
/// power capacity, repair timings, comms range, helm capabilities — is plumbed
/// through here, keyed by system kind, exactly as the shields base block was in
/// issue #772. Machine value-codes only; no player-visible English.
fn build_manual_system_extras(
    ship_config: &crate::entities::config::EntityConfig,
) -> std::collections::HashMap<String, toml::Value> {
    use crate::ship::system_registry as kinds;
    let mut extras: std::collections::HashMap<String, toml::Value> =
        std::collections::HashMap::new();
    let f = toml::Value::Float;
    let i = |n: i64| toml::Value::Integer(n);

    // Shields base (issue #772): `[shields_console.base]` HP + regen. Optional
    // block falls back to the historical shield defaults the runtime also uses.
    if let Some(sc) = &ship_config.shields_console {
        let base_cfg = sc.base.clone().unwrap_or_default();
        let mut base = toml::value::Table::new();
        base.insert("max_hp".into(), i(base_cfg.max_hp as i64));
        base.insert("regen_per_sec".into(), f(base_cfg.regen_per_sec as f64));
        extras.insert(kinds::SHIELDS_KIND.to_string(), toml::Value::Table(base));
    }

    if let Some(wc) = &ship_config.weapons_console {
        // Phaser banks — one per-bank table tagged with its system id so the
        // per-instance provider can find its own values. `0.0` authored fields
        // resolve to the same runtime beam defaults the combat code applies.
        use crate::entities::config::PhaserCombatConfig as P;
        let banks: Vec<toml::Value> = wc
            .phaser_banks
            .iter()
            .filter_map(|b| {
                let sid = crate::ship::system_registry::phaser_bank_system_id(&b.id)?;
                let mut t = toml::value::Table::new();
                t.insert("system_id".into(), toml::Value::String(sid.0));
                let beam_range = if b.beam_range > 0.0 {
                    b.beam_range
                } else {
                    P::DEFAULT_PHASER_RANGE
                };
                let beam_damage = if b.beam_damage_per_sec > 0.0 {
                    b.beam_damage_per_sec
                } else {
                    P::DEFAULT_BEAM_DAMAGE_PER_SEC
                };
                let cooldown = if b.cooldown_secs > 0.0 {
                    b.cooldown_secs
                } else {
                    P::DEFAULT_BEAM_COOLDOWN_SECS
                };
                t.insert("beam_range".into(), f(beam_range as f64));
                t.insert("beam_damage_per_sec".into(), f(beam_damage as f64));
                t.insert("cooldown_secs".into(), f(cooldown as f64));
                t.insert("fire_arc_deg".into(), f(b.fire_arc_deg as f64));
                Some(toml::Value::Table(t))
            })
            .collect();
        if !banks.is_empty() {
            let mut t = toml::value::Table::new();
            t.insert("banks".into(), toml::Value::Array(banks));
            extras.insert(kinds::PHASER_BANK_KIND.to_string(), toml::Value::Table(t));
        }

        // Blaster banks — range / volley / cooldown / fire arc + barrel count.
        let bbanks: Vec<toml::Value> = wc
            .blaster_banks
            .iter()
            .filter_map(|b| {
                let sid = crate::ship::system_registry::blaster_bank_system_id(&b.id)?;
                let barrel_count = if b.barrels.is_empty() {
                    1
                } else {
                    b.barrels.len()
                };
                let mut t = toml::value::Table::new();
                t.insert("system_id".into(), toml::Value::String(sid.0));
                t.insert("range".into(), f(b.range as f64));
                t.insert("volley_count".into(), i(b.volley_count as i64));
                t.insert("cooldown_secs".into(), f(b.cooldown_secs as f64));
                t.insert("fire_arc_deg".into(), f(b.fire_arc_deg as f64));
                t.insert("barrel_count".into(), i(barrel_count as i64));
                Some(toml::Value::Table(t))
            })
            .collect();
        if !bbanks.is_empty() {
            let mut t = toml::value::Table::new();
            t.insert("banks".into(), toml::Value::Array(bbanks));
            extras.insert(kinds::BLASTER_BANK_KIND.to_string(), toml::Value::Table(t));
        }

        // Tactical radar range from `[weapons_console.radar]`.
        if let Some(r) = &wc.radar {
            let mut t = toml::value::Table::new();
            t.insert("range".into(), f(r.range as f64));
            extras.insert(
                kinds::TACTICAL_RADAR_KIND.to_string(),
                toml::Value::Table(t),
            );
        }
    }

    // Torpedoes — shared magazine/warhead figures + per-tube layout.
    if let Some(tc) = &ship_config.torpedoes {
        let mut mag = toml::value::Table::new();
        mag.insert("count".into(), i(tc.count as i64));
        mag.insert("damage_hull".into(), i(tc.damage_hull as i64));
        mag.insert("damage_shields".into(), i(tc.damage_shields as i64));
        mag.insert("load_time".into(), f(tc.load_time as f64));
        extras.insert(
            kinds::TORPEDO_MAGAZINE_KIND.to_string(),
            toml::Value::Table(mag),
        );

        let tubes: Vec<toml::Value> = tc
            .tubes
            .iter()
            .filter_map(|tube| {
                let sid = crate::ship::system_registry::torpedo_tube_system_id(&tube.id)?;
                let load_time = tube.load_time.unwrap_or(tc.load_time);
                let mut t = toml::value::Table::new();
                t.insert("system_id".into(), toml::Value::String(sid.0));
                t.insert("fire_arc_deg".into(), f(tube.fire_arc_deg as f64));
                t.insert("load_time".into(), f(load_time as f64));
                t.insert("volley_max".into(), i(tube.volley_max as i64));
                Some(toml::Value::Table(t))
            })
            .collect();
        if !tubes.is_empty() {
            let mut t = toml::value::Table::new();
            t.insert("tubes".into(), toml::Value::Array(tubes));
            extras.insert(kinds::TORPEDO_TUBE_KIND.to_string(), toml::Value::Table(t));
        }
    }

    // Sensors long-range radar range feeds both the coarse `sensors` section
    // and the fine `sensor_radar` section (same authored range).
    if let Some(sc) = &ship_config.sensors_console {
        let mut t = toml::value::Table::new();
        t.insert("range".into(), f(sc.long_range_radar.range as f64));
        extras.insert(
            kinds::SENSORS_KIND.to_string(),
            toml::Value::Table(t.clone()),
        );
        extras.insert(kinds::SENSOR_RADAR_KIND.to_string(), toml::Value::Table(t));
    }

    // Power — reactor capacity + battery emergency reserve threshold.
    if let Some(pc) = &ship_config.power {
        let mut reactor = toml::value::Table::new();
        reactor.insert("capacity".into(), f(pc.capacity as f64));
        extras.insert(
            kinds::POWER_REACTOR_KIND.to_string(),
            toml::Value::Table(reactor),
        );
        let mut battery = toml::value::Table::new();
        battery.insert(
            "emergency_threshold".into(),
            f(pc.emergency_threshold as f64),
        );
        extras.insert(
            kinds::POWER_BATTERY_KIND.to_string(),
            toml::Value::Table(battery),
        );
    }

    // Repair team timings from `[repair]`.
    if let Some(rc) = &ship_config.repair {
        let mut t = toml::value::Table::new();
        t.insert("repair_team_count".into(), i(rc.repair_team_count as i64));
        t.insert(
            "repair_rate_hp_per_sec".into(),
            f(rc.repair_rate_hp_per_sec as f64),
        );
        t.insert(
            "travel_duration_secs".into(),
            f(rc.travel_duration_secs as f64),
        );
        extras.insert(kinds::REPAIR_KIND.to_string(), toml::Value::Table(t));
    }

    // Comms range from `[comms]`.
    if let Some(cc) = &ship_config.comms {
        let mut t = toml::value::Table::new();
        t.insert("range".into(), f(cc.range as f64));
        extras.insert(kinds::COMMS_KIND.to_string(), toml::Value::Table(t));
    }

    // Helm — speeds from `[helm_console]` and the EFFECTIVE `[helm_capability]`
    // (movement mode + impulse steering), defaulting to planar / full config
    // defaults when the block is absent. Reuses the same movement-mode mapping
    // the client config path uses above.
    let mut helm = toml::value::Table::new();
    if let Some(hc) = &ship_config.helm_console {
        helm.insert("max_speed".into(), f(hc.max_speed as f64));
        helm.insert("max_reverse_speed".into(), f(hc.max_reverse_speed as f64));
        helm.insert("max_yaw_rate".into(), f(hc.max_yaw_rate as f64));
    }
    let cap = ship_config.helm_capability.clone().unwrap_or_default();
    let movement_mode = match cap.vertical_movement_mode {
        crate::entities::config::VerticalMovementMode::Planar => "planar",
        crate::entities::config::VerticalMovementMode::Bounded => "bounded",
        crate::entities::config::VerticalMovementMode::Full3D => "full_3d",
    };
    helm.insert(
        "movement_mode".into(),
        toml::Value::String(movement_mode.to_string()),
    );
    helm.insert(
        "impulse_steering_multiplier".into(),
        f(cap.impulse.steering_multiplier as f64),
    );
    extras.insert(
        kinds::HELM_THRUST_KIND.to_string(),
        toml::Value::Table(helm),
    );

    extras
}

pub fn update_game_state_cache(
    sessions: Res<Sessions>,
    state: Res<State<GamePhase>>,
    world: Option<Res<WorldResource>>,
    mut cache: ResMut<GameStateCache>,
) {
    if !sessions.is_changed() && !state.is_changed() {
        return;
    }
    let world_data = world.as_ref().map(|w| &w.0);
    cache.0 = handler::derive_game_state(&sessions.0, state.get(), world_data);
}

// ── Systems ────────────────────────────────────────────────────────────────

/// Per-variant system for `ClientMessage::Identify` (issue #734) — the reconnect
/// handshake. Gated on Lobby/Loading/InProgress so a browser refresh mid-game
/// still receives its `Welcome` and has its seat restored. Every parameter is
/// sourced exactly as the former `process_lobby` Identify path did:
/// `world` from `WorldResource`, `ship_stations` with a `default()` fallback,
/// `ship_config` from `ShipClientConfigResource`, and the ratings SNAPSHOT from
/// either `active_ratings` (ship present) or `pending_ratings()` (pre-spawn).
/// `handle_identify` takes no `preload_complete` (only `SetReady` needed it).
pub fn handle_identify_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    world: Option<Res<WorldResource>>,
    ship_stations: Option<Res<ShipStations>>,
    ship_client_config: Res<ShipClientConfigResource>,
    ship_manual: Res<ShipManualResource>,
    gm_roster: Res<crate::gm_roster::GmRoster>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let world_data = world.as_ref().map(|w| &w.0);
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::Identify { token, name } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let ratings_snapshot = active_ratings.0.clone();
            let result = handler::handle_identify(
                token,
                name,
                &mut sessions.0,
                phase.clone(),
                world_data,
                stations,
                &ship_client_config.0,
                &ratings_snapshot,
                gm_roster.operators(),
            );
            // Publish the read-only ship manual (issue #772) to this client
            // right after its Welcome — same trigger, same recipient. Only when
            // the identify was accepted (a Welcome is going out).
            let sent_welcome = result
                .outbound
                .iter()
                .any(|(_, m)| matches!(m, ServerMessage::Welcome { .. }));
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
            if sent_welcome {
                outbox.0.push((
                    Target::Token(token.clone()),
                    ServerMessage::ShipManual {
                        manual: ship_manual.0.clone(),
                    },
                ));
            }
        } else {
            // No Ship entity yet (Lobby/Loading) — fall back to whatever
            // ratings players have picked in the lobby so far, so (re)joining
            // clients' Welcome reflects current toggle state.
            let pending_ratings = sessions.0.pending_ratings().clone();
            let result = handler::handle_identify(
                token,
                name,
                &mut sessions.0,
                phase.clone(),
                world_data,
                stations,
                &ship_client_config.0,
                &pending_ratings,
                gm_roster.operators(),
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            let sent_welcome = result
                .outbound
                .iter()
                .any(|(_, m)| matches!(m, ServerMessage::Welcome { .. }));
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
            if sent_welcome {
                outbox.0.push((
                    Target::Token(token.clone()),
                    ServerMessage::ShipManual {
                        manual: ship_manual.0.clone(),
                    },
                ));
            }
        }
    }
}

/// Per-variant system for `ClientMessage::SetName` (issue #734). Gated on
/// Lobby/Loading. The result only carries outbound (a `NameChanged` broadcast),
/// but the dual-path `apply_result` call mirrors the other systems for
/// consistency.
pub fn handle_set_name_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SetName { name } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = handler::handle_set_name(&ev.token, name, &mut sessions.0);
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = handler::handle_set_name(&ev.token, name, &mut sessions.0);
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::ReturnToLobby` (issue #734). Gated on
/// GameOver for a connected participant — the game-over screen's "return to
/// lobby" button — and additionally on `InProgress` for the host page's own
/// settings menu, whose "exit to lobby" aborts a running mission (issue #939).
/// The phase gate itself lives in `handler::handle_return_to_lobby`; this
/// system only classifies the sender's token. `apply_result` routes the phase
/// transition back to `Lobby` plus the cleared-ready / returned broadcasts.
pub fn handle_return_to_lobby_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
    mut gm_journal: Option<ResMut<crate::gm_action::GmActionJournal>>,
    mut gm_log: Option<ResMut<crate::gm_action::GmActionLog>>,
    mut gm_results: Option<ResMut<crate::gm_action::LocalGmActionRefusals>>,
    mut gm_projection: Option<ResMut<crate::gm_action::LastGmSessionProjection>>,
    mut paused: Option<ResMut<crate::gm_action::SimulationPaused>>,
    mut virtual_time: Option<ResMut<Time<bevy::time::Virtual>>>,
) {
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    let mut returned = false;
    for ev in events {
        let ClientMessage::ReturnToLobby = &ev.msg else {
            continue;
        };
        let authority = handler::return_to_lobby_authority(&ev.token);
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = handler::handle_return_to_lobby(&mut sessions.0, phase.clone(), authority);
            returned |= result.new_phase == Some(GamePhase::Lobby);
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = handler::handle_return_to_lobby(&mut sessions.0, phase.clone(), authority);
            returned |= result.new_phase == Some(GamePhase::Lobby);
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
    if returned {
        // Clear the complete per-run lane synchronously before MeshSet. An old
        // due Pause must not reassert itself in `apply_due_actions`, and a GM
        // request queued later in this frame is either cleared here or refused
        // as WrongPhase on the next frame. The technical fleet remains intact.
        if let Some(journal) = gm_journal.as_deref_mut() {
            *journal = Default::default();
        }
        if let Some(log) = gm_log.as_deref_mut() {
            *log = Default::default();
        }
        if let Some(results) = gm_results.as_deref_mut() {
            *results = Default::default();
        }
        if let Some(projection) = gm_projection.as_deref_mut() {
            *projection = Default::default();
        }
        if let Some(paused) = paused.as_deref_mut() {
            paused.0 = false;
        }
        // Return-to-lobby is frame-driven specifically so a paused fixed clock
        // cannot deadlock the host's escape route. This system is ordered before
        // MeshSet: it releases the product hold, then the mesh/model gate gets
        // the final say and may immediately re-pause the shared clock.
        if let Some(virtual_time) = virtual_time.as_deref_mut() {
            virtual_time.unpause();
        }
    }
}

/// Per-variant system for `ClientMessage::SelectStation` (issue #733).
/// Reads its variant off the inbound bus with its own cursor, calls the pure
/// `handler::handle_select_station`, then applies the result to Bevy
/// resources via `apply_result` — using the same dual-path
/// (real ship entity vs. pre-spawn fallback) handling as the other lobby
/// message systems.
pub fn handle_select_station_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SelectStation { station } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = handler::handle_select_station(
                &ev.token,
                station,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = handler::handle_select_station(
                &ev.token,
                station,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::ReleaseStation` (issue #733).
pub fn handle_release_station_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::ReleaseStation = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = handler::handle_release_station(
                &ev.token,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = handler::handle_release_station(
                &ev.token,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::SetReady` (issue #733).
pub fn handle_set_ready_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
    #[cfg(feature = "server")] preload: Option<
        Res<crate::server::asset_preload::AssetPreloadResource>,
    >,
    model_rigs: Option<Res<crate::entities::model_markers::ModelRigReadiness>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let phase = state.get().clone();
    // Preload gate: same logic as handle_disconnect. `AssetPreloadResource` is
    // presentation (`crate::server::asset_preload`), present only in server+render
    // builds, so the read is `server`-gated (issue #1194) to keep this always-
    // compiled lobby system from naming the presentation module with the feature
    // off. Feature-off reads "ready" — matching the None / `!started` default —
    // and no simulation runs in a feature-off build regardless.
    #[cfg(feature = "server")]
    let preload_ready = preload
        .as_ref()
        .map(|p| !p.started || p.complete)
        .unwrap_or(true);
    #[cfg(not(feature = "server"))]
    let preload_ready = true;
    let preload_complete = (crate::debug_overlay::is_playwright_automation() || preload_ready)
        && model_rigs.is_none_or(|rigs| rigs.is_ready());
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SetReady { ready } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = handler::handle_set_ready(
                &ev.token,
                *ready,
                &mut sessions.0,
                phase.clone(),
                preload_complete,
                stations,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = handler::handle_set_ready(
                &ev.token,
                *ready,
                &mut sessions.0,
                phase.clone(),
                preload_complete,
                stations,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::SetSpectator` (issue #1105). Reads
/// its variant off the inbound bus with its own cursor, calls the pure
/// `handler::handle_set_spectator`, and applies the seat-vacate / unready
/// / rating-reset / `SpectatorChanged` broadcasts through the same dual-path
/// `apply_result` the other lobby message systems use. Threads the phase and
/// `ShipStations` through like `handle_release_station_system` does: when a
/// spectator gives up a seat, that seat's rating is reset (Backfill mid-game,
/// base rating pre-game) exactly as a ReleaseStation would.
pub fn handle_set_spectator_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SetSpectator { spectator } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = handler::handle_set_spectator(
                &ev.token,
                *spectator,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = handler::handle_set_spectator(
                &ev.token,
                *spectator,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::SetAfk` (issue #1104). Reads its
/// variant off the inbound bus with its own cursor, calls the pure
/// `handler::handle_set_afk`, and applies the delegate/restore
/// `RatingChanged` + `AfkChanged` broadcasts through the same dual-path
/// `apply_result` the other lobby message systems use. Threads
/// `ActiveStationRatings` through so the pure handler can SNAPSHOT the player's
/// current directly-held Station rating before Backfill overwrites it (AFK-exit
/// restores from that snapshot). Runs on the same Lobby/Loading/InProgress gate
/// as the other station systems.
pub fn handle_set_afk_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SetAfk { afk } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result =
                handler::handle_set_afk(&ev.token, *afk, &mut sessions.0, &active_ratings.0);
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let fallback_ratings = ActiveStationRatings::default();
            let result =
                handler::handle_set_afk(&ev.token, *afk, &mut sessions.0, &fallback_ratings.0);
            let mut apply_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut apply_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::SetStationRating` (issue #733).
/// The pure handler only acts in Lobby/Loading (InProgress rating changes are
/// applied against the live Ship entity by
/// `ship_plugin::handle_station_rating_change`), so the InProgress run is a
/// no-op here — but the system is still gated on InProgress per the user's
/// decision to keep the four station systems on a uniform phase gate.
pub fn handle_set_station_rating_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SetStationRating { rating_name } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = handler::handle_set_station_rating(
                &ev.token,
                rating_name,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = handler::handle_set_station_rating(
                &ev.token,
                rating_name,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::ReportStationEligibility` (issue #1103).
///
/// Stores the sender's ANONYMOUS ineligible-Station set in the SessionManager
/// side-map (off `Player`, never broadcast). There is no `apply_result` /
/// `LobbyOutbox` path on purpose: the report produces no outbound message — it is
/// private host state that only the human-seeking resolver
/// (`resolve_human_seeking_hosts`) and the direct-claim guard
/// (`handle_select_station`) consult, and only as a boolean. The profile and the
/// functional reasons never reach the host. Registered alongside the other
/// station-management systems on the Lobby/Loading/InProgress gate so a client
/// can (re)report as it tweaks its profile before or during a mission.
pub fn handle_report_station_eligibility_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
) {
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::ReportStationEligibility { ineligible } = &ev.msg else {
            continue;
        };
        let set: std::collections::HashSet<crate::core::messages::StationId> =
            ineligible.iter().cloned().collect();
        sessions.0.set_eligibility(&ev.token, set);
    }
}

fn handle_disconnect(
    mut events: MessageReader<PlayerDisconnected>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    stations: Option<Res<ShipStations>>,
    #[cfg(feature = "server")] preload: Option<
        Res<crate::server::asset_preload::AssetPreloadResource>,
    >,
    model_rigs: Option<Res<crate::entities::model_markers::ModelRigReadiness>>,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let empty_stations = ShipStations::default();
    let ship_stations = stations.as_deref().unwrap_or(&empty_stations);

    // Preload gate: same logic as handle_set_ready_system — `server`-gated because
    // `AssetPreloadResource` is presentation (issue #1194); see that system for the
    // full rationale. Feature-off reads "ready" and no simulation runs there.
    #[cfg(feature = "server")]
    let preload_ready = preload
        .as_ref()
        .map(|p| !p.started || p.complete)
        .unwrap_or(true);
    #[cfg(not(feature = "server"))]
    let preload_ready = true;
    let preload_complete = (crate::debug_overlay::is_playwright_automation() || preload_ready)
        && model_rigs.is_none_or(|rigs| rigs.is_ready());

    for ev in events.read() {
        // Apply Backfill rating to the disconnecting player's station so the
        // ship keeps operating without a human at the console.
        // ship_query may return Err if the Ship entity hasn't spawned yet.
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let ratings_snapshot = active_ratings.0.clone();
            let result = handler::process_disconnect_with_stations(
                &ev.token,
                &mut sessions.0,
                ship_stations,
                &cfg.0,
                &mut cs.0,
                &ratings_snapshot,
                state.get().clone(),
                preload_complete,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = handler::process_disconnect(
                &ev.token,
                &mut sessions.0,
                state.get().clone(),
                preload_complete,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

fn apply_result(
    result: handler::LobbyHandlerResult,
    outbox: &mut ResMut<LobbyOutbox>,
    next_state: &mut ResMut<NextState<GamePhase>>,
    ship_config: Option<&ShipConfigComponent>,
    control_sources: Option<&mut ShipSystemControlSources>,
    active_ratings: &mut ActiveStationRatings,
    mut countdown: Option<&mut CountdownTimer>,
) {
    // Handle countdown actions before the phase transition so the cancel
    // broadcast goes out on the same frame as the unready message.
    if let Some(ref action) = result.countdown_action {
        if let Some(ref mut timer) = countdown {
            match action {
                CountdownAction::Start {
                    secs,
                    pending_phase,
                } if timer.local_start_allowed && timer.remaining_secs <= 0.0 => {
                    timer.remaining_secs = *secs as f32;
                    timer.pending_phase = Some(pending_phase.clone());
                    outbox.0.push((
                        Target::All,
                        ServerMessage::GameStartCountdown {
                            remaining_secs: *secs,
                        },
                    ));
                }
                CountdownAction::Cancel if timer.remaining_secs > 0.0 => {
                    timer.remaining_secs = 0.0;
                    timer.pending_phase = None;
                    outbox.0.push((
                        Target::All,
                        ServerMessage::GameStartCountdown { remaining_secs: 0 },
                    ));
                }
                _ => {}
            }
        }
    }

    if let Some(new_phase) = result.new_phase {
        next_state.set(new_phase);
    }
    if let Some((station_id, rating_name)) = result.station_rating_update {
        if let (Some(cfg), Some(cs)) = (ship_config, control_sources) {
            rating::apply_rating(&cfg.0, &station_id, &rating_name, &mut cs.0);
        }
        active_ratings.0.insert(station_id, rating_name);
    }
    outbox.0.extend(result.outbound);
}

/// Keep the legacy per-ship countdown dormant while the fleet mesh owns the
/// collective policy. This runs before every lobby handler, so a `SetReady`
/// processed later in the same fixed tick sees `local_start_allowed == false`.
fn enforce_fleet_managed_countdown(
    managed: Res<FleetManagedLobby>,
    mut timer: ResMut<CountdownTimer>,
    mut outbox: ResMut<LobbyOutbox>,
) {
    timer.local_start_allowed = !managed.enabled;
    if managed.enabled && timer.remaining_secs > 0.0 {
        timer.remaining_secs = 0.0;
        timer.pending_phase = None;
        outbox.0.push((
            Target::All,
            ServerMessage::GameStartCountdown { remaining_secs: 0 },
        ));
    }
}

pub(crate) fn start_result(
    grant: &crate::lobby::start_policy::StartGrant,
    status: crate::lobby::start_policy::StartGrantStatus,
    reason: Option<crate::lobby::start_policy::StartGrantReason>,
    tick: u64,
) -> crate::lobby::start_policy::StartGrantResult {
    crate::lobby::start_policy::StartGrantResult {
        tick,
        status,
        operator_id: grant.operator_id.clone(),
        reason,
        grant_id: Some(grant.id.clone()),
    }
}

/// Apply the host mesh's single start decision on a logical tick.
///
/// The grant is the readiness decision boundary. In particular, an automatic
/// grant is **not** rechecked against this host's local crew after receipt: a
/// late ready withdrawal could otherwise make one peer refuse while its fleet
/// mates start. The mesh owner orders readiness changes before the grant and
/// broadcasts that same idempotency key to every simulation peer. Rust still
/// independently revalidates the conditions that can safely be identical or
/// local-hard gates at application: managed mode, grant shape/sequence,
/// authoritative phase, and local content/peer validation. Forced attribution
/// is likewise frozen by the owner-authenticated grant; re-reading a GM
/// disconnect here could split peers exactly like re-reading local readiness.
/// Once that shared validation admits a grant, every peer enters
/// [`GamePhase::InProgress`] directly. Host-local asset preload state is
/// projected to the mesh before the grant and must not be re-read afterward,
/// or render and rendererless peers could choose different phases.
///
/// Receipt is not the application boundary. A grant waits in the bounded
/// queue until its exact `apply_tick`; applying it on "the next tick" would
/// make async delivery order authoritative. A grant first seen after that tick
/// is refused rather than shifted, surfacing a broken delivery barrier instead
/// of quietly forking the fleet. The host mesh/lockstep layer is responsible
/// for withholding the scheduled tick until every peer has received the grant.
fn apply_pending_start_grants(
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    managed: Res<FleetManagedLobby>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    mut pending: ResMut<PendingStartGrants>,
    mut tracker: ResMut<StartGrantTracker>,
    mut results: ResMut<StartGrantResults>,
    mut outbox: ResMut<LobbyOutbox>,
    roster: Option<Res<crate::lockstep::FleetRoster>>,
    fleet: Option<Res<crate::lockstep::FleetLockstep>>,
    mut mesh_outbox: Option<ResMut<crate::lockstep::MeshOutbox>>,
) {
    let mut started_this_tick = false;
    let source_tick = sim_tick.as_deref().map_or(0, |tick| tick.0);
    let start_result =
        |grant: &crate::lobby::start_policy::StartGrant,
         status: crate::lobby::start_policy::StartGrantStatus,
         reason: Option<crate::lobby::start_policy::StartGrantReason>| {
            start_result(grant, status, reason, source_tick)
        };
    // Process only the grants present at entry. Future grants are requeued;
    // walking until empty would immediately pop the same one forever.
    let queued_at_entry = pending.len();
    for _ in 0..queued_at_entry {
        let Some(mut grant) = pending.pop_front() else {
            break;
        };
        let Ok(sequence) = grant.validate() else {
            results.push(start_result(
                &grant,
                crate::lobby::start_policy::StartGrantStatus::Refused,
                Some(crate::lobby::start_policy::StartGrantReason::InvalidGrant),
            ));
            continue;
        };

        if sequence <= tracker.last_sequence {
            results.push(start_result(
                &grant,
                crate::lobby::start_policy::StartGrantStatus::NoOp,
                Some(crate::lobby::start_policy::StartGrantReason::AlreadyStarted),
            ));
            continue;
        }
        let Some(now) = sim_tick.as_ref().map(|tick| tick.0) else {
            tracker.last_sequence = sequence;
            results.push(start_result(
                &grant,
                crate::lobby::start_policy::StartGrantStatus::Refused,
                Some(crate::lobby::start_policy::StartGrantReason::InvalidGrant),
            ));
            continue;
        };
        let local_is_owner = roster
            .as_deref()
            .is_none_or(|roster| roster.local() == roster.owner());
        if !tracker.is_canonical(&grant) && local_is_owner {
            let refusal = if !managed.enabled {
                Some((
                    crate::lobby::start_policy::StartGrantStatus::Refused,
                    crate::lobby::start_policy::StartGrantReason::FleetNotManaged,
                ))
            } else if state.get() != &GamePhase::Lobby {
                Some((
                    crate::lobby::start_policy::StartGrantStatus::NoOp,
                    crate::lobby::start_policy::StartGrantReason::AlreadyStarted,
                ))
            } else if !managed.validation_passed {
                Some((
                    crate::lobby::start_policy::StartGrantStatus::Refused,
                    crate::lobby::start_policy::StartGrantReason::ValidationFailed,
                ))
            } else {
                None
            };
            if let Some((status, reason)) = refusal {
                tracker.last_sequence = sequence;
                results.push(start_result(&grant, status, Some(reason)));
                continue;
            }
        }
        if !tracker.is_canonical(&grant) && local_is_owner && grant.apply_tick != 0 {
            tracker.last_sequence = sequence;
            results.push(start_result(
                &grant,
                crate::lobby::start_policy::StartGrantStatus::Refused,
                Some(crate::lobby::start_policy::StartGrantReason::UnsafeApplyTick),
            ));
            continue;
        }
        if grant.apply_tick == 0 && local_is_owner {
            let assigned = if roster.as_deref().is_some_and(|roster| !roster.is_solo()) {
                fleet
                    .as_deref()
                    .and_then(|fleet| fleet.ready_through(now).checked_add(1))
            } else {
                Some(now)
            };
            let Some(assigned) = assigned
                .filter(|tick| *tick <= crate::lobby::start_policy::MAX_SAFE_START_APPLY_TICK)
            else {
                tracker.last_sequence = sequence;
                results.push(start_result(
                    &grant,
                    crate::lobby::start_policy::StartGrantStatus::Refused,
                    Some(crate::lobby::start_policy::StartGrantReason::UnsafeApplyTick),
                ));
                continue;
            };
            grant.apply_tick = assigned;
        }

        // A browser proposal is not the decision boundary on a member. The
        // technical owner first seals it into its authenticated TickFrame;
        // `apply_mesh_inbox` then adopts that exact value as canonical. This
        // also permits an early control-plane copy to wait harmlessly on a
        // member until the ordered owner frame arrives.
        if tracker.canonical.is_some() && !tracker.is_canonical(&grant) {
            tracker.last_sequence = tracker.last_sequence.max(sequence);
            results.push(start_result(
                &grant,
                crate::lobby::start_policy::StartGrantStatus::Refused,
                Some(crate::lobby::start_policy::StartGrantReason::ConflictingGrant),
            ));
            continue;
        }
        if !tracker.is_canonical(&grant) {
            let multi_participant = roster.as_deref().is_some_and(|roster| !roster.is_solo());
            if !local_is_owner {
                if now < grant.apply_tick {
                    pending.requeue(grant);
                    continue;
                }
                tracker.last_sequence = sequence;
                results.push(start_result(
                    &grant,
                    crate::lobby::start_policy::StartGrantStatus::Refused,
                    Some(crate::lobby::start_policy::StartGrantReason::UnauthorizedGrant),
                ));
                continue;
            }

            if multi_participant {
                let Some(fleet) = fleet.as_deref() else {
                    tracker.last_sequence = sequence;
                    results.push(start_result(
                        &grant,
                        crate::lobby::start_policy::StartGrantStatus::Refused,
                        Some(crate::lobby::start_policy::StartGrantReason::UnauthorizedGrant),
                    ));
                    continue;
                };
                let Some(mesh_outbox) = mesh_outbox.as_deref_mut() else {
                    tracker.last_sequence = sequence;
                    results.push(start_result(
                        &grant,
                        crate::lobby::start_policy::StartGrantStatus::Refused,
                        Some(crate::lobby::start_policy::StartGrantReason::UnauthorizedGrant),
                    ));
                    continue;
                };
                // The frame sealed after this system declares readiness through
                // this watermark. The decision tick must be strictly beyond it,
                // so every participant has to receive the bearing frame before
                // the barrier can possibly open the decision boundary.
                if fleet.is_alone() || grant.apply_tick <= fleet.ready_through(now) {
                    tracker.last_sequence = sequence;
                    results.push(start_result(
                        &grant,
                        crate::lobby::start_policy::StartGrantStatus::Refused,
                        Some(crate::lobby::start_policy::StartGrantReason::UnsafeApplyTick),
                    ));
                    continue;
                }
                if tracker.adopt_canonical(&grant).is_err()
                    || !mesh_outbox.stage_start_grant(grant.clone())
                {
                    tracker.last_sequence = sequence;
                    results.push(start_result(
                        &grant,
                        crate::lobby::start_policy::StartGrantStatus::Refused,
                        Some(crate::lobby::start_policy::StartGrantReason::ConflictingGrant),
                    ));
                    continue;
                }
                tracker.mark_embedded();
            } else if tracker.adopt_canonical(&grant).is_err() {
                tracker.last_sequence = sequence;
                results.push(start_result(
                    &grant,
                    crate::lobby::start_policy::StartGrantStatus::Refused,
                    Some(crate::lobby::start_policy::StartGrantReason::ConflictingGrant),
                ));
                continue;
            }
        }
        if now < grant.apply_tick {
            // Do not consume the sequence while waiting. Otherwise this same
            // queued grant would look like a duplicate on the next tick and
            // no peer would ever reach its scheduled boundary.
            pending.requeue(grant);
            continue;
        }
        // Consume every well-shaped id once, including refusals. A caller must
        // mint a new grant after any condition changes; replaying the same
        // request later cannot turn a refusal into a mutation.
        tracker.last_sequence = sequence;

        if now > grant.apply_tick {
            results.push(start_result(
                &grant,
                crate::lobby::start_policy::StartGrantStatus::Refused,
                Some(crate::lobby::start_policy::StartGrantReason::MissedApplyTick),
            ));
            continue;
        }

        if started_this_tick || state.get() != &GamePhase::Lobby {
            results.push(start_result(
                &grant,
                crate::lobby::start_policy::StartGrantStatus::NoOp,
                Some(crate::lobby::start_policy::StartGrantReason::AlreadyStarted),
            ));
            continue;
        }
        next_state.set(GamePhase::InProgress);
        outbox.0.push((Target::All, ServerMessage::GameStarted));
        started_this_tick = true;
        results.push(start_result(
            &grant,
            crate::lobby::start_policy::StartGrantStatus::Applied,
            None,
        ));
    }
}

/// Ticks the pre-game countdown each frame. When the countdown reaches 0,
/// transitions to the pending phase and broadcasts `GameStarted`. Also
/// checks `all_ready()` each frame and cancels the countdown if a player
/// unreadied, disconnected, or a new player joined without readying.
fn tick_countdown(
    time: Res<Time>,
    mut timer: ResMut<CountdownTimer>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    sessions: Option<Res<Sessions>>,
) {
    if timer.remaining_secs <= 0.0 {
        return;
    }

    if !timer.local_start_allowed {
        timer.remaining_secs = 0.0;
        timer.pending_phase = None;
        return;
    }

    // Cancel if not all connected players are ready anymore.
    if let Some(ref sessions) = sessions {
        if !sessions.0.all_ready() {
            timer.remaining_secs = 0.0;
            timer.pending_phase = None;
            outbox.0.push((
                Target::All,
                ServerMessage::GameStartCountdown { remaining_secs: 0 },
            ));
            return;
        }
    }

    let prev = timer.remaining_secs;
    timer.remaining_secs -= time.delta_secs();
    if timer.remaining_secs <= 0.0 {
        // Countdown complete — transition.
        timer.remaining_secs = 0.0;
        if let Some(ref phase) = timer.pending_phase {
            next_state.set(phase.clone());
            outbox.0.push((Target::All, ServerMessage::GameStarted));
        }
        timer.pending_phase = None;
    } else {
        // Broadcast when the whole-second display changes.
        let prev_secs = prev.ceil() as u32;
        let now_secs = timer.remaining_secs.ceil() as u32;
        if now_secs != prev_secs {
            outbox.0.push((
                Target::All,
                ServerMessage::GameStartCountdown {
                    remaining_secs: now_secs,
                },
            ));
        }
    }
}

// ── Outbox drain ───────────────────────────────────────────────────────────

/// Plugin that drains [`LobbyOutbox`] into the `OutboundMessage` bus every
/// frame, regardless of the current game phase.
///
/// This phase-agnostic drain is intentional: `tick_countdown` both transitions
/// the phase to `InProgress` *and* queues `GameStarted` in the same frame.
/// A phase-gated drain (such as routing through `LobbyBroadcaster`) would skip
/// the outbox on that transition frame, causing `GameStarted` to be lost.
pub struct LobbyOutboxPlugin;

impl Plugin for LobbyOutboxPlugin {
    fn build(&self, app: &mut App) {
        // `FixedUpdate` with the rest of the lobby (issue #895): the
        // `.after(tick_countdown)` edge — which is what keeps `GameStarted`
        // from being lost on the transition tick — is only real inside the
        // schedule `tick_countdown` runs in.
        app.add_systems(
            FixedUpdate,
            drain_lobby_outbox
                .after(tick_countdown)
                .after(apply_pending_start_grants),
        );
    }
}

pub(crate) fn drain_lobby_outbox(world: &mut World) {
    let entries = std::mem::take(&mut world.resource_mut::<LobbyOutbox>().0);
    for (target, msg) in entries {
        world.write_message(OutboundMessage {
            target,
            msg,
            delivery: DeliveryClass::Reliable,
        });
    }
}

/// Returns a [`LobbyOutboxPlugin`] that drains [`LobbyOutbox`] into the
/// `OutboundMessage` bus each frame.
///
/// This must be registered once (typically in `bridge.rs`) alongside
/// `LobbyPlugin`.
pub fn lobby_outbox_broadcaster() -> LobbyOutboxPlugin {
    LobbyOutboxPlugin
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut outbox: ResMut<Outbox>) {
        for ev in reader.read() {
            outbox.0.push(ev.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(lobby_outbox_broadcaster())
            .add_plugins(bevy::time::TimePlugin)
            .init_resource::<Outbox>()
            .add_systems(PostUpdate, collect);
        crate::sim_tick::register_sim_tick(&mut app);
        // One fixed step per update (issue #895): the lobby runs on the
        // logical tick, and each 1 s harness tick advances it once — the
        // countdown tests count whole seconds per tick.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_secs_f32(1.0),
        );
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let msgs = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        msgs
    }

    #[test]
    fn client_system_kind_projection_preserves_arbitrary_authored_instance_ids() {
        use crate::core::messages::SystemId;
        use crate::ship::config::SystemInstanceConfig;

        let system = |id: &str, kind: &str| SystemInstanceConfig {
            id: SystemId(id.into()),
            kind: kind.into(),
            station: None,
            ai_only: false,
            human_seeking: false,
            seek_order: vec![],
            power_group: None,
            marker: None,
            config: None,
        };
        let projected = project_system_kinds(&[
            system("port-flight-vector", "helm_steering"),
            system("berthing-clamps", "dock"),
            system("pulse-reservoir-seven", "helm_boost"),
        ]);

        assert_eq!(
            projected,
            std::collections::HashMap::from([
                (
                    "port-flight-vector".to_string(),
                    "helm_steering".to_string()
                ),
                ("berthing-clamps".to_string(), "dock".to_string()),
                (
                    "pulse-reservoir-seven".to_string(),
                    "helm_boost".to_string()
                ),
            ])
        );
    }

    #[test]
    fn identify_arrives_via_inbound_message_and_welcome_is_sent_via_outbound() {
        let mut app = test_app();
        push(
            &mut app,
            "peer-id",
            ClientMessage::Identify {
                token: "t1".into(),
                name: "Alice".into(),
            },
        );
        let out = tick(&mut app);
        assert!(out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::Welcome { .. })));
    }

    #[test]
    fn identify_welcome_projects_the_separate_gm_resource() {
        let mut app = test_app();
        app.insert_resource(
            crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
                id: "gm-1".into(),
                name: "Morgan".into(),
                connected: true,
                ready: false,
            }])
            .unwrap(),
        );
        push(
            &mut app,
            "peer-id",
            ClientMessage::Identify {
                token: "t1".into(),
                name: "Alice".into(),
            },
        );

        let out = tick(&mut app);
        let gms = out.iter().find_map(|outbound| match &outbound.msg {
            ServerMessage::Welcome { state, gms, .. } => {
                assert_eq!(state.players.len(), 1);
                Some(gms)
            }
            _ => None,
        });
        assert_eq!(gms.unwrap()[0].id, "gm-1");
        assert_eq!(app.world().resource::<Sessions>().0.players().len(), 1);
    }

    #[test]
    fn select_station_works_during_in_progress_phase() {
        use crate::core::messages::StationId;
        use crate::lobby::stations_config::stations_from_ship_config;
        use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
        use std::collections::HashMap;

        let mut app = test_app();

        // Phase starts at Lobby by default.
        // Add a ship with station config before startup so
        // update_session_with_config sees non-empty stations.
        let ship_config = ShipConfig {
            stations: vec![
                StationConfig {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "Helm station".into(),
                    rank: "Crew".into(),
                    short_code: "H".into(),
                    ratings: vec![StationRatingConfig {
                        name: "Std".into(),
                        automated_systems: vec![],
                        ai_tuning: None,
                    }],
                    console: None,
                    manual_overview: None,
                    tutorials: vec![],
                    human_seeking: false,
                    host_order: vec![],
                    visiting_rating: None,
                    auxiliary: false,
                    command_target: None,
                    stances: vec![],
                },
                StationConfig {
                    id: StationId("tactical".into()),
                    name: "Tactical".into(),
                    description: "Tactical station".into(),
                    rank: "Crew".into(),
                    short_code: "T".into(),
                    ratings: vec![StationRatingConfig {
                        name: "Std".into(),
                        automated_systems: vec![],
                        ai_tuning: None,
                    }],
                    console: None,
                    manual_overview: None,
                    tutorials: vec![],
                    human_seeking: false,
                    host_order: vec![],
                    visiting_rating: None,
                    auxiliary: false,
                    command_target: None,
                    stances: vec![],
                },
            ],
            systems: vec![],
            power_groups: HashMap::new(),
            coordination_lag_secs: 2.0,
        };
        app.world_mut()
            .insert_resource(stations_from_ship_config(&ship_config));
        app.world_mut()
            .insert_resource(ShipClientConfigResource::default());

        // Verify stations are populated
        {
            let stations = app.world().resource::<ShipStations>();
            assert!(
                !stations.stations.is_empty(),
                "ShipStations must be non-empty"
            );
            assert_eq!(stations.stations.len(), 2, "expected 2 stations");
        }

        // Register two players in lobby first.
        // The peer ID (first arg to push) is the session token sent by the bridge,
        // and the Identify message body carries the same token for registration.
        push(
            &mut app,
            "t1",
            ClientMessage::Identify {
                token: "t1".into(),
                name: "Player1".into(),
            },
        );
        push(
            &mut app,
            "t2",
            ClientMessage::Identify {
                token: "t2".into(),
                name: "Player2".into(),
            },
        );
        tick(&mut app);

        // Player1 claims Helm in lobby
        push(
            &mut app,
            "t1",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        let out = tick(&mut app);
        assert!(out.iter().any(|m| {
            matches!(&m.msg, ServerMessage::StationAssigned { token, station, .. }
                if token == "t1" && station == &Some("Helm".into()))
        }));

        // Start the game — both ready triggers countdown
        push(&mut app, "t1", ClientMessage::SetReady { ready: true });
        push(&mut app, "t2", ClientMessage::SetReady { ready: true });
        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::GameStartCountdown { .. })),
            "ready should start countdown"
        );

        // Fast-forward the countdown by advancing the timer directly.
        use crate::lobby::CountdownTimer;
        app.world_mut()
            .resource_mut::<CountdownTimer>()
            .remaining_secs = 0.001;
        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::GameStarted)),
            "countdown expiry must emit GameStarted"
        );

        // Now in InProgress: Player2 claims Tactical (was unclaimed)
        push(
            &mut app,
            "t2",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        let out = tick(&mut app);
        assert!(
            out.iter().any(|m| {
                matches!(&m.msg, ServerMessage::StationAssigned { token, station, .. }
                    if token == "t2" && station == &Some("Tactical".into()))
            }),
            "SelectStation should work during InProgress phase"
        );
    }

    #[test]
    fn release_station_works_during_in_progress_phase() {
        use crate::core::messages::StationId;
        use crate::lobby::stations_config::stations_from_ship_config;
        use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
        use std::collections::HashMap;

        let mut app = test_app();

        // Phase starts at Lobby by default.
        let ship_config = ShipConfig {
            stations: vec![StationConfig {
                id: StationId("helm".into()),
                name: "Helm".into(),
                description: "Helm station".into(),
                rank: "Crew".into(),
                short_code: "H".into(),
                ratings: vec![StationRatingConfig {
                    name: "Std".into(),
                    automated_systems: vec![],
                    ai_tuning: None,
                }],
                console: None,
                manual_overview: None,
                tutorials: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
                command_target: None,
                stances: vec![],
            }],
            systems: vec![],
            power_groups: HashMap::new(),
            coordination_lag_secs: 2.0,
        };
        app.world_mut()
            .insert_resource(stations_from_ship_config(&ship_config));
        app.world_mut()
            .insert_resource(ShipClientConfigResource::default());

        // Register player and claim station in lobby.
        // The peer ID (first arg to push) is the session token sent by the bridge.
        push(
            &mut app,
            "t1",
            ClientMessage::Identify {
                token: "t1".into(),
                name: "Player1".into(),
            },
        );
        tick(&mut app);

        push(
            &mut app,
            "t1",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        tick(&mut app);

        // Start the game — single player ready triggers countdown
        push(&mut app, "t1", ClientMessage::SetReady { ready: true });
        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::GameStartCountdown { .. })),
            "ready should start countdown"
        );

        // Fast-forward the countdown by advancing the timer directly.
        use crate::lobby::CountdownTimer;
        app.world_mut()
            .resource_mut::<CountdownTimer>()
            .remaining_secs = 0.001;
        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::GameStarted)),
            "countdown expiry must emit GameStarted"
        );

        // Now in InProgress: Player1 releases Helm
        push(&mut app, "t1", ClientMessage::ReleaseStation);
        let out = tick(&mut app);
        assert!(
            out.iter().any(|m| {
                matches!(&m.msg, ServerMessage::StationAssigned { token, station, .. }
                    if token == "t1" && station.is_none())
            }),
            "ReleaseStation should work during InProgress phase"
        );
    }

    #[test]
    fn selected_ship_resource_populates_ship_stations_via_update_session() {
        use crate::core::messages::StationId;
        use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
        use std::collections::HashMap;

        let mut app = test_app();

        // Insert PendingShipConfig so update_session_with_config uses it.
        // ShipStations starts empty (init_resource in LobbyPlugin).
        let ship_config = ShipConfig {
            stations: vec![
                StationConfig {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "Helm station".into(),
                    rank: "Crew".into(),
                    short_code: "H".into(),
                    ratings: vec![StationRatingConfig {
                        name: "Std".into(),
                        automated_systems: vec![],
                        ai_tuning: None,
                    }],
                    console: None,
                    manual_overview: None,
                    tutorials: vec![],
                    human_seeking: false,
                    host_order: vec![],
                    visiting_rating: None,
                    auxiliary: false,
                    command_target: None,
                    stances: vec![],
                },
                StationConfig {
                    id: StationId("tactical".into()),
                    name: "Tactical".into(),
                    description: "Tactical station".into(),
                    rank: "Crew".into(),
                    short_code: "T".into(),
                    ratings: vec![StationRatingConfig {
                        name: "Std".into(),
                        automated_systems: vec![],
                        ai_tuning: None,
                    }],
                    console: None,
                    manual_overview: None,
                    tutorials: vec![],
                    human_seeking: false,
                    host_order: vec![],
                    visiting_rating: None,
                    auxiliary: false,
                    command_target: None,
                    stances: vec![],
                },
            ],
            systems: vec![],
            power_groups: HashMap::new(),
            coordination_lag_secs: 2.0,
        };
        app.world_mut()
            .insert_resource(crate::ship_plugin::PendingShipConfig(ship_config.clone()));

        // First update runs Startup systems including update_session_with_config
        app.update();

        // Assert stations were populated from PendingShipConfig
        let stations = app.world().resource::<ShipStations>();
        assert_eq!(stations.stations.len(), 2);
        assert_eq!(stations.stations[0].id.0, "helm");
        assert_eq!(stations.stations[0].name, "Helm");
        assert_eq!(stations.stations[1].id.0, "tactical");
        assert_eq!(stations.stations[1].name, "Tactical");
    }

    // ── #773: system_extras extraction from real hull assets ──────────────────

    /// Parse a real hull TOML into an `EntityConfig`, run the full manual
    /// pipeline (`build_manual_system_extras` + `build_ship_manual`), and return
    /// the resulting manual alongside the config it was built from. This is the
    /// integration path the pure `ship::manual` module can't cover on its own
    /// (it never sees `EntityConfig`).
    fn manual_from_hull(
        path: &str,
    ) -> (
        crate::entities::config::EntityConfig,
        crate::ship::manual::ShipManualWire,
    ) {
        // Through the include resolver (issue #906) — the same document the
        // runtime hull load produces, composed or not.
        let config = crate::entities::include_resolve::load_entity_config(path)
            .unwrap_or_else(|e| panic!("parse {path}: {e}"));
        let topology = config
            .ship_config
            .clone()
            .expect("hull declares a ship_config");
        let extras = build_manual_system_extras(&config);
        let registry = crate::ship::manual::ManualProviderRegistry::with_shipped_providers();
        let manual = crate::ship::manual::build_ship_manual(&topology, &registry, &extras);
        (config, manual)
    }

    #[test]
    fn shared_client_config_projector_preserves_complete_non_default_authored_values() {
        let (config, _) = manual_from_hull("assets/entities/alliance_cruiser.toml");
        let projected = project_ship_client_config(&config);

        assert_eq!(projected.helm_radar_range, 93.75);
        assert_eq!(projected.helm_radar_shows[0], "player");
        assert_eq!(projected.sensors_radar_range, 300.0);
        assert_eq!(
            projected.sensors_radar_selects,
            ["ship", "station", "planet"]
        );
        assert_eq!(projected.nav_chart_range, 800.0);
        assert_eq!(
            projected.nav_chart_selects,
            ["station", "planet", "star", "region"]
        );
        assert_eq!(projected.hostile_arc_color, [1.0, 0.3, 0.3, 0.07]);
        assert_eq!(
            projected
                .phaser_banks
                .iter()
                .map(|bank| (bank.id.as_str(), bank.facing_deg, bank.fire_arc_deg))
                .collect::<Vec<_>>(),
            [("fore", 0.0, 270.0), ("aft", 180.0, 270.0)]
        );
        assert_eq!(
            projected
                .torpedo_tubes
                .iter()
                .map(|tube| (tube.id.as_str(), tube.facing_deg, tube.fire_arc_deg))
                .collect::<Vec<_>>(),
            [
                ("fore_port", 0.0, 90.0),
                ("fore_starboard", 0.0, 90.0),
                ("aft", 180.0, 90.0),
            ]
        );
        assert_eq!(projected.class.as_deref(), Some("cruiser"));
        assert_eq!(projected.hull_id.as_deref(), Some("NCC-1864"));
        assert_eq!(projected.power_rating, Some(90));
        assert_eq!(
            projected.ship_css.as_deref(),
            Some("gui/themes/cruiser.css")
        );
        assert!(
            projected
                .station_tutorials
                .get("helm")
                .is_some_and(|tutorials| tutorials.iter().any(|entry| entry.id == "helm-welcome")),
            "the complete ordinary Welcome config carries authored tutorials"
        );

        let topology = config.ship_config.as_ref().expect("cruiser topology");
        let expected_gaps = topology
            .stations
            .iter()
            .map(|station| {
                (
                    station.id.0.clone(),
                    crate::ship::eligibility::projected_assist_gaps(station, topology),
                )
            })
            .filter(|(_, gaps)| !gaps.is_empty())
            .collect::<std::collections::HashMap<_, _>>();
        assert!(
            !expected_gaps.is_empty(),
            "fixture must exercise assist gaps"
        );
        assert_eq!(projected.station_assist_gaps, expected_gaps);
        assert_eq!(projected.station_systems["helm"], projected.helm_systems);
        assert_eq!(projected.system_kinds["helm-thrust"], "helm_thrust");
    }

    fn find_metric(
        manual: &crate::ship::manual::ShipManualWire,
        kind: &str,
        code: &str,
    ) -> Option<f64> {
        manual
            .stations
            .iter()
            .flat_map(|s| &s.sections)
            .find(|sec| sec.kind == kind)
            .and_then(|sec| sec.metrics.iter().find(|m| m.code == code))
            .map(|m| m.value)
    }

    #[test]
    fn real_hulls_produce_different_manual_values() {
        let (cruiser_cfg, cruiser) = manual_from_hull("assets/entities/alliance_cruiser.toml");
        let (courier_cfg, courier) = manual_from_hull("assets/entities/alliance_courier.toml");

        // Reactor capacity reflects each hull's own authored [power] capacity.
        let cruiser_cap = find_metric(
            &cruiser,
            crate::ship::system_registry::POWER_REACTOR_KIND,
            "capacity",
        );
        let courier_cap = find_metric(
            &courier,
            crate::ship::system_registry::POWER_REACTOR_KIND,
            "capacity",
        );
        assert_eq!(
            cruiser_cap,
            cruiser_cfg.power.as_ref().map(|p| p.capacity as f64)
        );
        assert_eq!(
            courier_cap,
            courier_cfg.power.as_ref().map(|p| p.capacity as f64)
        );
        assert_ne!(
            cruiser_cap, courier_cap,
            "manual content must change with ship configuration (AC2)"
        );

        // Comms range likewise reflects each hull's own authored [comms] range.
        let cruiser_comms =
            find_metric(&cruiser, crate::ship::system_registry::COMMS_KIND, "range");
        let courier_comms =
            find_metric(&courier, crate::ship::system_registry::COMMS_KIND, "range");
        assert_eq!(
            cruiser_comms,
            cruiser_cfg.comms.as_ref().map(|c| c.range as f64)
        );
        assert_eq!(
            courier_comms,
            courier_cfg.comms.as_ref().map(|c| c.range as f64)
        );
        assert_ne!(
            cruiser_comms, courier_comms,
            "manual content must change with ship configuration (AC2)"
        );
    }

    #[test]
    fn cruiser_manual_covers_weapons_helm_and_sensors_from_authored_config() {
        let (cfg, cruiser) = manual_from_hull("assets/entities/alliance_cruiser.toml");

        // Phaser bank beam range reflects the authored config, not a pinned number.
        let authored_beam_range = cfg
            .weapons_console
            .as_ref()
            .and_then(|w| w.phaser_banks.first())
            .map(|b| b.beam_range as f64);
        assert!(authored_beam_range.is_some(), "hull authors a phaser bank");
        assert_eq!(
            find_metric(
                &cruiser,
                crate::ship::system_registry::PHASER_BANK_KIND,
                "beam_range"
            ),
            authored_beam_range
        );
        // Torpedo magazine capacity and tube count reflect the authored [torpedoes] block.
        let torpedoes = cfg.torpedoes.as_ref().expect("hull authors torpedoes");
        assert_eq!(
            find_metric(
                &cruiser,
                crate::ship::system_registry::TORPEDO_MAGAZINE_KIND,
                "capacity"
            ),
            Some(torpedoes.count as f64)
        );
        assert_eq!(
            find_metric(
                &cruiser,
                crate::ship::system_registry::TORPEDO_MAGAZINE_KIND,
                "tubes"
            ),
            Some(torpedoes.tubes.len() as f64)
        );
        // Sensors long-range radar range reflects the authored [sensors_console].
        assert_eq!(
            find_metric(
                &cruiser,
                crate::ship::system_registry::SENSORS_KIND,
                "range"
            ),
            cfg.sensors_console
                .as_ref()
                .map(|s| s.long_range_radar.range as f64)
        );

        // Helm movement mode: no `[helm_capability]` authored ⇒ effective planar.
        let helm = cruiser
            .stations
            .iter()
            .flat_map(|s| &s.sections)
            .find(|sec| sec.kind == crate::ship::system_registry::HELM_THRUST_KIND)
            .expect("helm section present");
        assert_eq!(
            helm.capabilities
                .iter()
                .find(|c| c.code == "movement_mode")
                .map(|c| c.value_code.as_str()),
            Some("planar")
        );
        // And the authored [helm_console] max speed is reflected, not a pinned number.
        assert_eq!(
            helm.metrics
                .iter()
                .find(|m| m.code == "max_speed")
                .map(|m| m.value),
            cfg.helm_console.as_ref().map(|h| h.max_speed as f64)
        );
    }

    #[test]
    fn courier_manual_covers_its_blaster_bank() {
        // The courier carries a blaster, not torpedoes — proving the blaster
        // provider is fed from real authored config.
        let (cfg, courier) = manual_from_hull("assets/entities/alliance_courier.toml");
        let authored_range = cfg
            .weapons_console
            .as_ref()
            .and_then(|w| w.blaster_banks.first())
            .map(|b| b.range as f64);
        assert!(authored_range.is_some(), "hull authors a blaster bank");
        assert_eq!(
            find_metric(
                &courier,
                crate::ship::system_registry::BLASTER_BANK_KIND,
                "range"
            ),
            authored_range
        );
    }

    fn automatic_grant(sequence: u64) -> crate::lobby::start_policy::StartGrant {
        crate::lobby::start_policy::StartGrant {
            id: format!("start-{sequence}"),
            mode: crate::lobby::start_policy::StartGrantMode::Automatic,
            operator_id: None,
            apply_tick: 0,
        }
    }

    fn forced_grant(sequence: u64, operator_id: &str) -> crate::lobby::start_policy::StartGrant {
        crate::lobby::start_policy::StartGrant {
            id: format!("start-{sequence}"),
            mode: crate::lobby::start_policy::StartGrantMode::Forced,
            operator_id: Some(operator_id.into()),
            apply_tick: 0,
        }
    }

    fn enable_managed_lobby(app: &mut App, validation_passed: bool) {
        let mut managed = app.world_mut().resource_mut::<FleetManagedLobby>();
        managed.set_enabled(true);
        managed.validation_passed = validation_passed;
    }

    fn install_two_participant_fleet(app: &mut App, local: u32, owner: u32, delay: u64) {
        use crate::command_admission::HostSlot;

        app.init_resource::<crate::command_admission::log::PendingCommands>();
        crate::lockstep::register_lockstep(app);
        let participants = vec![HostSlot(1), HostSlot(2)];
        let roster = crate::lockstep::FleetRoster::with_participants(
            vec![crate::lockstep::FleetShip::new(HostSlot(1))],
            participants.clone(),
            HostSlot(local),
            HostSlot(owner),
        )
        .unwrap();
        app.insert_resource(roster);
        app.insert_resource(crate::lockstep::FleetLockstep(
            crate::lockstep::LockstepSession::new_at(HostSlot(local), participants, delay, 0)
                .unwrap(),
        ));
        for peer in [HostSlot(1), HostSlot(2)] {
            if peer != HostSlot(local) {
                app.world_mut()
                    .resource_mut::<crate::lockstep::FleetLockstep>()
                    .observe(peer, u64::MAX);
            }
        }
    }

    fn install_gm_owner_fleet(app: &mut App, local: u32, delay: u64) {
        use crate::command_admission::HostSlot;

        app.init_resource::<crate::command_admission::log::PendingCommands>();
        crate::lockstep::register_lockstep(app);
        let participants = vec![HostSlot(1), HostSlot(2), HostSlot(3)];
        let roster = crate::lockstep::FleetRoster::with_participants(
            vec![
                crate::lockstep::FleetShip::new(HostSlot(2)),
                crate::lockstep::FleetShip::new(HostSlot(3)),
            ],
            participants.clone(),
            HostSlot(local),
            HostSlot(1),
        )
        .unwrap();
        app.insert_resource(roster);
        let mut session =
            crate::lockstep::LockstepSession::new_at(HostSlot(local), participants, delay, 0)
                .unwrap();
        // Keep every surviving ship peer ahead of this narrow start-boundary
        // fixture. The GM owner's opening watermark remains exactly `delay`, so
        // its loss is agreed at the same tick the embedded grant names.
        for survivor in [HostSlot(2), HostSlot(3)] {
            if survivor != HostSlot(local) {
                session.observe(survivor, u64::MAX);
            }
        }
        app.insert_resource(crate::lockstep::FleetLockstep(session));
    }

    fn take_start_results(app: &mut App) -> Vec<crate::lobby::start_policy::StartGrantResult> {
        app.world_mut()
            .resource_mut::<StartGrantResults>()
            .drain()
            .collect()
    }

    #[test]
    fn managed_automatic_grant_is_the_readiness_boundary_for_an_empty_ship_host() {
        let mut app = test_app();
        enable_managed_lobby(&mut app, true);
        assert_eq!(
            app.world().resource::<Sessions>().0.readiness_tally(),
            crate::lobby::start_policy::ReadinessTally::default(),
            "this peer must not invent a local participant"
        );
        assert!(app
            .world_mut()
            .resource_mut::<PendingStartGrants>()
            .try_push(automatic_grant(1)));

        let source_tick = app.world().resource::<crate::sim_tick::SimTick>().0;
        let out = tick(&mut app);
        assert!(out
            .iter()
            .any(|message| matches!(message.msg, ServerMessage::GameStarted)));
        let result = take_start_results(&mut app).pop().unwrap();
        assert_eq!(
            result.status,
            crate::lobby::start_policy::StartGrantStatus::Applied
        );
        assert_eq!(result.tick, source_tick);
        assert_eq!(
            app.world().resource::<crate::sim_tick::SimTick>().0,
            source_tick + 1,
            "the result must not inherit PostUpdate's continuation tick"
        );
    }

    #[test]
    fn managed_gm_only_automatic_grant_applies_without_local_crew() {
        let mut app = test_app();
        enable_managed_lobby(&mut app, true);
        app.insert_resource(
            crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
                id: "gm-1".into(),
                name: "Morgan".into(),
                connected: true,
                ready: true,
            }])
            .unwrap(),
        );
        assert!(app
            .world_mut()
            .resource_mut::<PendingStartGrants>()
            .try_push(automatic_grant(1)));

        let out = tick(&mut app);
        assert!(out
            .iter()
            .any(|message| matches!(message.msg, ServerMessage::GameStarted)));
        assert_eq!(
            take_start_results(&mut app)[0].status,
            crate::lobby::start_policy::StartGrantStatus::Applied
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn managed_grant_enters_the_same_phase_for_different_local_preload_states() {
        fn peer_with_preload(complete: bool) -> App {
            let mut app = test_app();
            enable_managed_lobby(&mut app, true);
            let mut preload = crate::server::asset_preload::AssetPreloadResource::default();
            preload.started = true;
            preload.complete = complete;
            app.insert_resource(preload);
            assert!(app
                .world_mut()
                .resource_mut::<PendingStartGrants>()
                .try_push(automatic_grant(1)));
            app
        }

        let mut still_loading_assets = peer_with_preload(false);
        let mut completed_assets = peer_with_preload(true);

        for app in [&mut still_loading_assets, &mut completed_assets] {
            tick(app);
            assert_eq!(
                take_start_results(app)[0].status,
                crate::lobby::start_policy::StartGrantStatus::Applied
            );
            // Apply the NextState scheduled on the fixed tick above.
            tick(app);
            assert_eq!(
                app.world().resource::<State<GamePhase>>().get(),
                &GamePhase::InProgress
            );
        }
    }

    #[test]
    fn managed_force_grant_is_immutable_across_a_late_gm_disconnect() {
        let mut app = test_app();
        enable_managed_lobby(&mut app, true);
        app.insert_resource(
            crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
                id: "gm-1".into(),
                name: "Morgan".into(),
                connected: true,
                ready: false,
            }])
            .unwrap(),
        );
        app.world_mut()
            .resource_mut::<Sessions>()
            .0
            .register("crew-1".into(), "Alice".into())
            .unwrap();
        assert!(app
            .world_mut()
            .resource_mut::<PendingStartGrants>()
            .try_push(forced_grant(1, "gm-1")));

        // The owner authenticated and attributed the force before emitting the
        // grant. A disconnect observed by only this peer after that decision
        // must not make it diverge from peers that already applied the grant.
        app.insert_resource(
            crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
                id: "gm-1".into(),
                name: "Morgan".into(),
                connected: false,
                ready: false,
            }])
            .unwrap(),
        );
        tick(&mut app);
        assert_eq!(
            take_start_results(&mut app)[0].status,
            crate::lobby::start_policy::StartGrantStatus::Applied
        );
    }

    #[test]
    fn managed_validation_refuses_auto_and_force() {
        let mut app = test_app();
        enable_managed_lobby(&mut app, false);
        app.insert_resource(
            crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
                id: "gm-1".into(),
                name: "Morgan".into(),
                connected: true,
                ready: true,
            }])
            .unwrap(),
        );
        {
            let mut pending = app.world_mut().resource_mut::<PendingStartGrants>();
            assert!(pending.try_push(automatic_grant(1)));
            assert!(pending.try_push(forced_grant(2, "gm-1")));
        }
        tick(&mut app);
        let results = take_start_results(&mut app);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| {
            result.status == crate::lobby::start_policy::StartGrantStatus::Refused
                && result.reason
                    == Some(crate::lobby::start_policy::StartGrantReason::ValidationFailed)
        }));
    }

    #[test]
    fn canonical_grant_does_not_reread_validation_at_apply_tick() {
        let mut app = test_app();
        enable_managed_lobby(&mut app, true);
        install_two_participant_fleet(&mut app, 1, 1, 2);
        assert!(app
            .world_mut()
            .resource_mut::<PendingStartGrants>()
            .try_push(automatic_grant(1)));

        tick(&mut app);
        assert!(app
            .world()
            .resource::<StartGrantTracker>()
            .canonical
            .is_some());
        app.world_mut()
            .resource_mut::<FleetManagedLobby>()
            .validation_passed = false;

        for _ in 0..3 {
            tick(&mut app);
        }
        let results = take_start_results(&mut app);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].status,
            crate::lobby::start_policy::StartGrantStatus::Applied,
            "validation was frozen when the owner sealed the canonical grant"
        );
    }

    #[test]
    fn member_with_failed_validation_refuses_owner_grant_before_barrier_opens() {
        use crate::command_admission::HostSlot;

        let mut app = test_app();
        enable_managed_lobby(&mut app, false);
        install_two_participant_fleet(&mut app, 2, 1, 2);
        let mut grant = automatic_grant(1);
        grant.apply_tick = 3;
        app.world_mut()
            .resource_mut::<crate::lockstep::MeshInbox>()
            .push_from(
                crate::lockstep::MeshFrame::Tick(crate::lockstep::TickFrame {
                    from: HostSlot(1),
                    tick: 0,
                    ready_through: 2,
                    commands: Vec::new(),
                    start_grant: Some(grant),
                }),
                crate::lockstep::MeshOrigin::Peer(HostSlot(1)),
            );

        tick(&mut app);
        assert!(app
            .world()
            .resource::<StartGrantTracker>()
            .is_failed_closed());
        assert!(app.world().resource::<PendingStartGrants>().0.is_empty());
        let results = take_start_results(&mut app);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].reason,
            Some(crate::lobby::start_policy::StartGrantReason::ValidationFailed)
        );
        assert_eq!(
            app.world().resource::<State<GamePhase>>().get(),
            &GamePhase::Lobby
        );
    }

    #[test]
    fn duplicate_grant_applies_once_and_reports_a_no_op() {
        let mut app = test_app();
        enable_managed_lobby(&mut app, true);
        {
            let mut pending = app.world_mut().resource_mut::<PendingStartGrants>();
            assert!(pending.try_push(automatic_grant(1)));
            assert!(pending.try_push(automatic_grant(1)));
        }
        let out = tick(&mut app);
        assert_eq!(
            out.iter()
                .filter(|message| matches!(message.msg, ServerMessage::GameStarted))
                .count(),
            1
        );
        let results = take_start_results(&mut app);
        assert_eq!(
            results
                .iter()
                .map(|result| result.status)
                .collect::<Vec<_>>(),
            vec![
                crate::lobby::start_policy::StartGrantStatus::Applied,
                crate::lobby::start_policy::StartGrantStatus::NoOp
            ]
        );
    }

    #[test]
    fn local_owner_cannot_propose_a_preselected_nonzero_apply_tick() {
        let mut app = test_app();
        enable_managed_lobby(&mut app, true);
        install_two_participant_fleet(&mut app, 1, 1, 2);

        let mut forged = automatic_grant(1);
        forged.apply_tick = 99;
        assert!(app
            .world_mut()
            .resource_mut::<PendingStartGrants>()
            .try_push(forged));

        tick(&mut app);
        let results = take_start_results(&mut app);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].status,
            crate::lobby::start_policy::StartGrantStatus::Refused
        );
        assert_eq!(
            results[0].reason,
            Some(crate::lobby::start_policy::StartGrantReason::UnsafeApplyTick)
        );
        assert!(app
            .world()
            .resource::<crate::lockstep::MeshOutbox>()
            .pending_frames()
            .iter()
            .all(|frame| !matches!(
                frame,
                crate::lockstep::MeshFrame::Tick(tick) if tick.start_grant.is_some()
            )));
    }

    #[test]
    fn technical_gm_departure_does_not_strand_an_embedded_start_boundary() {
        use crate::command_admission::HostSlot;

        let mut owner = test_app();
        enable_managed_lobby(&mut owner, true);
        install_gm_owner_fleet(&mut owner, 1, 2);
        assert!(owner
            .world_mut()
            .resource_mut::<PendingStartGrants>()
            .try_push(automatic_grant(1)));
        tick(&mut owner);
        let bearing = owner
            .world_mut()
            .resource_mut::<crate::lockstep::MeshOutbox>()
            .drain()
            .into_iter()
            .find(|frame| {
                matches!(
                    frame,
                    crate::lockstep::MeshFrame::Tick(tick) if tick.start_grant.is_some()
                )
            })
            .expect("the technical owner embeds its assigned boundary in a TickFrame");
        let apply_tick = match &bearing {
            crate::lockstep::MeshFrame::Tick(tick) => {
                assert_eq!(tick.from, HostSlot(1));
                tick.start_grant.as_ref().unwrap().apply_tick
            }
            _ => unreachable!(),
        };
        assert_eq!(apply_tick, 3);

        let mut survivors = [test_app(), test_app()];
        for (app, local) in survivors.iter_mut().zip([2, 3]) {
            enable_managed_lobby(app, true);
            install_gm_owner_fleet(app, local, 2);
            let mut inbox = app.world_mut().resource_mut::<crate::lockstep::MeshInbox>();
            // Stronger than the reliable transport's ordinary ordering: even if
            // the socket-close observation reaches Rust before the final owner
            // frame, the roster still authenticates the frozen owner and the
            // departed wait-set cannot strand its immutable decision.
            inbox.push_from(
                crate::lockstep::MeshFrame::HostLoss(crate::lockstep::HostLossFrame {
                    from: HostSlot(1),
                    lost: HostSlot(1),
                    tick: 0,
                }),
                crate::lockstep::MeshOrigin::LocalObservation,
            );
            inbox.push_from(
                bearing.clone(),
                crate::lockstep::MeshOrigin::Peer(HostSlot(1)),
            );
        }

        for app in &mut survivors {
            tick(app);
            let fleet = app.world().resource::<crate::lockstep::FleetLockstep>();
            assert!(fleet.has_departed(HostSlot(1)));
            assert_eq!(fleet.watermark_of(HostSlot(1)), None);
            assert!(fleet.peers().all(|peer| peer != HostSlot(1)));
            assert_eq!(
                app.world()
                    .resource::<StartGrantTracker>()
                    .canonical
                    .as_ref()
                    .unwrap()
                    .apply_tick,
                apply_tick
            );
            assert_eq!(
                app.world().resource::<State<GamePhase>>().get(),
                &GamePhase::Lobby
            );
            for _ in 0..3 {
                tick(app);
            }
            let results = take_start_results(app);
            assert_eq!(results.len(), 1);
            assert_eq!(
                results[0].status,
                crate::lobby::start_policy::StartGrantStatus::Applied
            );
            tick(app);
            assert_eq!(
                app.world().resource::<State<GamePhase>>().get(),
                &GamePhase::InProgress
            );
        }
        assert_eq!(
            survivors[0]
                .world()
                .resource::<crate::sim_tick::SimTick>()
                .0,
            survivors[1]
                .world()
                .resource::<crate::sim_tick::SimTick>()
                .0
        );
    }

    #[test]
    fn managed_lobby_never_arms_the_legacy_local_countdown() {
        let mut app = test_app();
        enable_managed_lobby(&mut app, true);
        app.world_mut()
            .resource_mut::<Sessions>()
            .0
            .register("t1".into(), "Alice".into())
            .unwrap();
        push(&mut app, "t1", ClientMessage::SetReady { ready: true });

        let out = tick(&mut app);
        assert_eq!(app.world().resource::<CountdownTimer>().remaining_secs, 0.0);
        assert!(!out.iter().any(|message| matches!(
            message.msg,
            ServerMessage::GameStartCountdown { remaining_secs } if remaining_secs > 0
        )));
    }

    #[test]
    fn same_frame_teardown_and_reopen_resets_start_id_generation() {
        let mut inputs = VecDeque::from([
            FleetLobbyInput::Managed(false),
            FleetLobbyInput::Managed(true),
            FleetLobbyInput::Validation(true),
            FleetLobbyInput::Grant(automatic_grant(1)),
        ]);
        let mut managed = FleetManagedLobby {
            enabled: true,
            validation_passed: true,
        };
        let mut grants = PendingStartGrants::default();
        let mut tracker = StartGrantTracker {
            last_sequence: 1,
            ..Default::default()
        };
        let mut results = StartGrantResults::default();

        assert!(apply_fleet_lobby_inputs(
            &mut inputs,
            &mut managed,
            &mut grants,
            &mut tracker,
            &mut results,
        ));
        assert!(inputs.is_empty());
        assert!(managed.enabled);
        assert!(managed.validation_passed);
        assert_eq!(tracker.last_sequence, 0);
        assert_eq!(grants.pop_front().unwrap().id, "start-1");
    }
}
