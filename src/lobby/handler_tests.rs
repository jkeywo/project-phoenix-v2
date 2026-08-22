use super::*;
use crate::core::messages::{ClientMessage, EntitySnapshot, StationId, WorldData};
use crate::lobby::stations_config::{ShipStations, StationDef};
use crate::ship::control_source::ControlSourceResolver;

/// Test-only dispatch matching the former production `process_message`
/// signature. Production now routes each `ClientMessage` lobby variant
/// through its own per-variant Bevy system in `LobbySystemSet` (issue #734),
/// so this helper preserves the exact per-variant pure-fn calls the tests
/// exercise (including dropping `preload_complete` for the `Identify` arm,
/// which `handle_identify` never took).
fn dispatch(
    token: &str,
    msg: &ClientMessage,
    sessions: &mut SessionManager,
    phase: GamePhase,
    world: Option<&WorldData>,
    ship_stations: &ShipStations,
    ship_config: &ShipClientConfig,
    preload_complete: bool,
    station_ratings: &HashMap<StationId, String>,
) -> LobbyHandlerResult {
    match msg {
        ClientMessage::Identify {
            token: id_token,
            name,
        } => handle_identify(
            id_token,
            name,
            sessions,
            phase,
            world,
            ship_stations,
            ship_config,
            station_ratings,
        ),
        ClientMessage::SetName { name } => handle_set_name(token, name, sessions),
        ClientMessage::SelectStation { station } => {
            handle_select_station(token, station, sessions, phase, ship_stations)
        }
        ClientMessage::ReleaseStation => {
            handle_release_station(token, sessions, phase, ship_stations)
        }
        ClientMessage::SetReady { ready } => handle_set_ready(
            token,
            *ready,
            sessions,
            phase,
            preload_complete,
            ship_stations,
        ),
        ClientMessage::SetSpectator { spectator } => {
            handle_set_spectator(token, *spectator, sessions, phase, ship_stations)
        }
        ClientMessage::SetAfk { afk } => handle_set_afk(token, *afk, sessions, station_ratings),
        ClientMessage::ReturnToLobby => {
            handle_return_to_lobby(sessions, phase, return_to_lobby_authority(token))
        }
        ClientMessage::SetStationRating { rating_name } => {
            handle_set_station_rating(token, rating_name, sessions, phase, ship_stations)
        }
        // The runtime variants are no-ops in the lobby — handled by the
        // console server plugins, not the lobby handler.
        //
        // `SelectScenario` / `SelectPlayerShip` (issue #755) are resolved
        // *before* any world is loaded by the host-runtime arbiter (server
        // .html + gui/scenario-arbiter.js), which intercepts them on the
        // datachannel and the local-console path and never forwards them to
        // WASM. Should one ever reach the running Bevy app (e.g. a late
        // phone message after the world is already loading), the selection
        // is already settled — so it is a deliberate no-op here.
        //
        // `StationVisited` (issue #1101) is likewise not a lobby concern: it
        // is drained frame-driven in `server_app::drain_station_visited`,
        // which mutates the host importance state, so the lobby has nothing
        // to add.
        // `ReportStationEligibility` (issue #1103) is stored directly into
        // the SessionManager side-map by `handle_report_station_eligibility_system`,
        // not through a pure result-producing handler — so like the other
        // runtime variants it is a no-op on this dispatch path.
        ClientMessage::ControlSystem { .. }
        | ClientMessage::SendCoordination { .. }
        | ClientMessage::SelectScenario { .. }
        | ClientMessage::SelectPlayerShip { .. }
        | ClientMessage::ReportStationEligibility { .. }
        | ClientMessage::StationVisited { .. } => LobbyHandlerResult {
            new_phase: None,
            outbound: Vec::new(),
            station_rating_update: None,
            countdown_action: None,
        },
        // `ToggleDebugFlag` / `TogglePause` (issue #940) are likewise not a
        // lobby concern: both are drained frame-driven in `debug_overlay`,
        // which carries their authority check, so the lobby has nothing to
        // add. A separate arm rather than another `|` because both variants
        // carry `#[cfg(not(phoenix_demo_build))]` — in a demo build they do
        // not exist and neither does this arm, and a `#[cfg]` cannot be
        // hung on one alternative of a pattern.
        #[cfg(not(phoenix_demo_build))]
        ClientMessage::ToggleDebugFlag { .. } | ClientMessage::TogglePause => LobbyHandlerResult {
            new_phase: None,
            outbound: Vec::new(),
            station_rating_update: None,
            countdown_action: None,
        },
    }
}

fn sessions_with(token: &str, name: &str) -> SessionManager {
    let mut s = SessionManager::new();
    s.register(token.to_string(), name.to_string()).unwrap();
    s
}

fn default_stations() -> ShipStations {
    ShipStations::default()
}
fn default_ship_config() -> ShipClientConfig {
    ShipClientConfig::default()
}

/// Minimal `ShipConfig` for backfill unit tests: a captain station that
/// owns a red-alert system so `apply_rating` has something to flip.
fn backfill_ship_config() -> crate::ship::config::ShipConfig {
    const TOML: &str = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."
short_code = "CPT"

[[station.rating]]
name = "Manual"
automated_systems = []

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
power_group = "ops"

[power_groups.ops]
label = "Ops"
default_level = 2
min_level = 1
max_level = 4
"#;
    const KINDS: &[&str] = &["red_alert", "captain"];
    crate::ship::config::parse_and_validate(TOML, KINDS).expect("backfill_ship_config must parse")
}

/// Call `process_disconnect_with_stations` with a no-op resolver and empty
/// station_ratings (for tests that only care about PlayerLeft output).
fn pd_stations(
    token: &str,
    sessions: &mut SessionManager,
    ship_stations: &ShipStations,
) -> LobbyHandlerResult {
    pd_stations_with_phase(token, sessions, ship_stations, GamePhase::Lobby, true)
}

/// Like `pd_stations` but with explicit phase and preload_complete.
fn pd_stations_with_phase(
    token: &str,
    sessions: &mut SessionManager,
    ship_stations: &ShipStations,
    phase: GamePhase,
    preload_complete: bool,
) -> LobbyHandlerResult {
    let mut resolver = ControlSourceResolver::new();
    process_disconnect_with_stations(
        token,
        sessions,
        ship_stations,
        &backfill_ship_config(),
        &mut resolver,
        &HashMap::new(),
        phase,
        preload_complete,
    )
}

fn pm(
    token: &str,
    msg: &ClientMessage,
    sessions: &mut SessionManager,
    phase: GamePhase,
    world: Option<&WorldData>,
) -> LobbyHandlerResult {
    dispatch(
        token,
        msg,
        sessions,
        phase,
        world,
        &default_stations(),
        &default_ship_config(),
        true, // preload is always complete in tests (no Bevy app)
        &HashMap::new(),
    )
}

// ── process_disconnect ────────────────────────────────────────────────

#[test]
fn disconnect_broadcasts_player_left() {
    let mut sessions = sessions_with("t1", "Alice");
    let result = process_disconnect("t1", &mut sessions, GamePhase::Lobby, true);
    assert!(result
        .outbound
        .iter()
        .any(|(_, m)| { matches!(m, ServerMessage::PlayerLeft { token } if token == "t1") }));
}

#[test]
fn disconnect_marks_player_as_disconnected() {
    let mut sessions = sessions_with("t1", "Alice");
    process_disconnect("t1", &mut sessions, GamePhase::Lobby, true);
    assert!(!sessions.players()[0].connected);
}

#[test]
fn disconnect_returns_no_phase_change() {
    // t1 (ready) disconnects, leaving t2 (not-ready) → no auto-start.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    sessions.set_ready("t1", true);
    let result = process_disconnect("t1", &mut sessions, GamePhase::Lobby, true);
    assert!(result.new_phase.is_none());
}

#[test]
fn disconnect_last_not_ready_auto_starts() {
    // t1 ready, t2 not-ready; t2 disconnects → all remaining connected
    // players are ready → countdown starts.
    let stations = ship_stations();
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    sessions.set_ready("t1", true);
    let result = pd_stations_with_phase("t2", &mut sessions, &stations, GamePhase::Lobby, true);
    assert_eq!(
        result.countdown_action,
        Some(CountdownAction::Start {
            secs: 5,
            pending_phase: GamePhase::InProgress
        }),
        "disconnect of last not-ready player must start countdown"
    );
    assert!(
        result.new_phase.is_none(),
        "new_phase must be None when countdown is used"
    );
    assert!(
        !result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::GameStarted)),
        "GameStarted must not be sent during countdown"
    );
}

#[test]
fn disconnect_auto_start_enters_loading_when_preload_not_complete() {
    let stations = ship_stations();
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    sessions.set_ready("t1", true);
    let result = pd_stations_with_phase("t2", &mut sessions, &stations, GamePhase::Lobby, false);
    assert_eq!(
        result.countdown_action,
        Some(CountdownAction::Start {
            secs: 5,
            pending_phase: GamePhase::Loading
        }),
        "disconnect must start countdown toward Loading when preload not complete"
    );
    assert!(
        result.new_phase.is_none(),
        "new_phase must be None when countdown is used"
    );
    // GameStarted should NOT be sent during countdown.
    assert!(
        !result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::GameStarted)),
        "GameStarted must not be sent during countdown"
    );
}

#[test]
fn disconnect_last_player_does_not_auto_start() {
    // Single player registers but never sets ready, then disconnects.
    // The game must NOT auto-start with zero human players.
    let mut sessions = sessions_with("t1", "Alice");
    let result = process_disconnect("t1", &mut sessions, GamePhase::Lobby, true);
    assert!(
        result.new_phase.is_none(),
        "last player disconnect must not auto-start the game"
    );
    assert!(
        !result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::GameStarted)),
        "GameStarted must not be sent when last player disconnects"
    );
}

#[test]
fn reconnect_comes_back_not_ready() {
    // t1 sets ready, disconnects (clears ready), reconnects via Identify
    // → ready must remain false.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_ready("t1", true);
    assert!(sessions.players()[0].ready);
    let _ = process_disconnect("t1", &mut sessions, GamePhase::Lobby, true);
    assert!(
        !sessions.players()[0].ready,
        "disconnect must clear ready flag"
    );
    // Reconnect via Identify
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Alice".into(),
    };
    let _ = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(
        !sessions.players()[0].ready,
        "reconnected player must come back not-ready"
    );
}

// ── process_message: Identify ─────────────────────────────────────────

#[test]
fn identify_new_player_sends_welcome_to_sender() {
    let mut sessions = SessionManager::new();
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Alice".into(),
    };
    let result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result.outbound.iter().any(|(target, m)| {
        matches!(target, Target::Token(t) if t == "t1")
            && matches!(m, ServerMessage::Welcome { .. })
    }));
}

#[test]
fn identify_new_player_broadcasts_player_joined_to_others() {
    let mut sessions = sessions_with("t2", "Bob");
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Alice".into(),
    };
    let result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result.outbound.iter().any(|(target, m)| {
        matches!(target, Target::AllExcept(t) if t == "t1")
            && matches!(m, ServerMessage::PlayerJoined { .. })
    }));
}

#[test]
fn identify_reconnect_sends_welcome_and_player_joined() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Alice".into(),
    };
    let result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result.outbound.iter().any(|(target, m)| {
        matches!(target, Target::Token(t) if t == "t1")
            && matches!(m, ServerMessage::Welcome { .. })
    }));
    assert!(result.outbound.iter().any(|(target, m)| {
        matches!(target, Target::AllExcept(t) if t == "t1")
            && matches!(m, ServerMessage::PlayerJoined { .. })
    }));
}

#[test]
fn welcome_during_lobby_carries_world_none() {
    let mut sessions = SessionManager::new();
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Alice".into(),
    };
    let result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
    let state = result
        .outbound
        .iter()
        .find_map(|(_, m)| match m {
            ServerMessage::Welcome { state, .. } => Some(state.clone()),
            _ => None,
        })
        .unwrap();
    assert!(state.world.is_none());
}

#[test]
fn welcome_during_in_progress_carries_world_some() {
    let mut sessions = SessionManager::new();
    let world = WorldData {
        entities: vec![EntitySnapshot::asteroid("test-uuid", 1.0, 2.0, 2.0)],
        ..Default::default()
    };
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Alice".into(),
    };
    let result = pm(
        "peer",
        &msg,
        &mut sessions,
        GamePhase::InProgress,
        Some(&world),
    );
    let state = result
        .outbound
        .iter()
        .find_map(|(_, m)| match m {
            ServerMessage::Welcome { state, .. } => Some(state.clone()),
            _ => None,
        })
        .unwrap();
    assert!(state.world.is_some());
}

// ── process_message: SetName ──────────────────────────────────────────

#[test]
fn set_name_broadcasts_name_changed() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::SetName {
        name: "Alicia".into(),
    };
    let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result.outbound.iter().any(|(_, m)| {
        matches!(m, ServerMessage::NameChanged { name, .. } if name == "Alicia")
    }));
}

// ── Helpers for station-aware tests ──────────────────────────────────

fn ship_stations() -> ShipStations {
    // Flat roster used by station-aware tests.
    // 9 stations, one console each.
    ShipStations {
        stations: vec![
            StationDef {
                id: StationId("captain".into()),
                name: "Captain".into(),
                description: "Command the bridge.".into(),
                rank: "Cpt.".into(),
                short_code: "CPT".into(),
                console: None,
                ratings: vec!["Std".into(), "Simplified".into()],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
            },
            StationDef {
                id: StationId("helm".into()),
                name: "Helm".into(),
                description: "Pilot the ship.".into(),
                rank: "Ltn.".into(),
                short_code: "HLM".into(),
                console: None,
                ratings: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
            },
            StationDef {
                id: StationId("tactical".into()),
                name: "Tactical".into(),
                description: "Manage weapons.".into(),
                rank: "Ltn.".into(),
                short_code: "TAC".into(),
                console: None,
                ratings: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
            },
            StationDef {
                id: StationId("repair".into()),
                name: "Repair".into(),
                description: "Repair systems.".into(),
                rank: "Ltn.".into(),
                short_code: "ENG".into(),
                console: None,
                ratings: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
            },
            StationDef {
                id: StationId("sensors".into()),
                name: "Sensors".into(),
                description: "Monitor sensors.".into(),
                rank: "Ens.".into(),
                short_code: "SCI".into(),
                console: None,
                ratings: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
            },
            StationDef {
                id: StationId("shields".into()),
                name: "Shields".into(),
                description: "Manage shields.".into(),
                rank: "Ens.".into(),
                short_code: "SHD".into(),
                console: None,
                ratings: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
            },
            StationDef {
                id: StationId("navigation".into()),
                name: "Navigation".into(),
                description: "Plot course.".into(),
                rank: "Ens.".into(),
                short_code: "NAV".into(),
                console: None,
                ratings: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
            },
            StationDef {
                id: StationId("power".into()),
                name: "Power".into(),
                description: "Manage power.".into(),
                rank: "Ltn.".into(),
                short_code: "PWR".into(),
                console: None,
                ratings: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
            },
            StationDef {
                id: StationId("comms".into()),
                name: "Comms".into(),
                description: "Hail contacts.".into(),
                rank: "Ens.".into(),
                short_code: "COM".into(),
                console: None,
                ratings: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
            },
        ],
    }
}

fn pm_stations(
    token: &str,
    msg: &ClientMessage,
    sessions: &mut SessionManager,
    phase: GamePhase,
    world: Option<&WorldData>,
) -> LobbyHandlerResult {
    dispatch(
        token,
        msg,
        sessions,
        phase,
        world,
        &ship_stations(),
        &default_ship_config(),
        true, // preload is always complete in tests (no Bevy app)
        &HashMap::new(),
    )
}

// ── process_message: SelectStation / ReleaseStation ───────────────────

#[test]
fn select_station_broadcasts_station_assigned() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::SelectStation {
        station: "Captain".into(),
    };
    let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. })));
}

#[test]
fn release_station_broadcasts_station_assigned() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::ReleaseStation;
    let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. })));
}

// ── SelectStation: empty station → StationAssigned with consoles ──────

#[test]
fn select_empty_station_assigns_consoles_and_broadcasts() {
    let mut sessions = sessions_with("t1", "Alice");
    // 1 player → station "Captain" with CaptainChair,Helm,Tactical,Repair,Power
    let msg = ClientMessage::SelectStation {
        station: "Captain".into(),
    };
    let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    let assigned = result.outbound.iter().find_map(|(_, m)| match m {
        ServerMessage::StationAssigned {
            token,
            station,
            station_id,
            ..
        } if token == "t1" => Some((station.clone(), station_id.clone())),
        _ => None,
    });
    let (station_name, station_id) = assigned.expect("StationAssigned not found");
    assert_eq!(station_name, Some("Captain".to_string()));
    assert_eq!(
        station_id,
        Some(crate::core::messages::StationId("captain".into()))
    );
}

#[test]
fn select_station_broadcast_target_is_all() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::SelectStation {
        station: "Captain".into(),
    };
    let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result.outbound.iter().any(|(t, m)| {
        matches!(t, Target::All) && matches!(m, ServerMessage::StationAssigned { .. })
    }));
}

#[test]
fn select_station_sets_player_station_field() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::SelectStation {
        station: "Captain".into(),
    };
    pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    let station_id = sessions.station_for_token("t1");
    assert_eq!(
        station_id,
        Some(&StationId("captain".into())),
        "Player.station must be set to the StationId after SelectStation"
    );
}

// ── Accessibility eligibility on the direct claim (issue #1103) ───────

#[test]
fn select_station_allows_claim_for_eligible_sender() {
    let mut sessions = sessions_with("t1", "Alice");
    // No eligibility reported → DEFAULT TRUE → the claim proceeds as today.
    handle_select_station(
        "t1",
        "captain",
        &mut sessions,
        GamePhase::Lobby,
        &ship_stations(),
    );
    assert_eq!(
        sessions.station_for_token("t1"),
        Some(&StationId("captain".into())),
        "an eligible (unreported) sender claims the seat normally"
    );
}

#[test]
fn select_station_silently_no_ops_an_ineligible_claim() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_eligibility(
        "t1",
        std::collections::HashSet::from([StationId("captain".into())]),
    );
    let result = handle_select_station(
        "t1",
        "captain",
        &mut sessions,
        GamePhase::Lobby,
        &ship_stations(),
    );
    assert_eq!(
        sessions.station_for_token("t1"),
        None,
        "an ineligible claim must not seat the player"
    );
    assert!(
        result.outbound.is_empty(),
        "an ineligible claim takes the neutral no-op path: no broadcast, no reason on the wire"
    );
}

// ── #1106: a Spectator claims an eligible open Station ─────────────────
// The claim runs through the SAME authoritative path as an ordinary lobby
// claim (handle_select_station); a success seats the player AND clears the
// Spectator role server-side (the set_station invariant). There is NO
// spectator-specific host code — these tests assert the existing path
// already admits an ex-Spectator race-safely.

#[test]
fn spectator_claiming_open_station_is_seated_and_role_cleared() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_spectator("t1", true);
    assert!(sessions.is_spectator("t1"));
    // A mid-mission claim of an open, eligible seat.
    handle_select_station(
        "t1",
        "captain",
        &mut sessions,
        GamePhase::InProgress,
        &ship_stations(),
    );
    assert_eq!(
        sessions.station_for_token("t1"),
        Some(&StationId("captain".into())),
        "a Spectator's claim seats them through the normal admission path"
    );
    assert!(
        !sessions.is_spectator("t1"),
        "a successful claim clears the Spectator role (set_station invariant)"
    );
}

#[test]
fn simultaneous_spectator_claims_first_wins_second_stays_spectator() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    sessions.set_spectator("t1", true);
    sessions.set_spectator("t2", true);
    // t1 wins the race for the open seat.
    handle_select_station(
        "t1",
        "helm",
        &mut sessions,
        GamePhase::InProgress,
        &ship_stations(),
    );
    assert_eq!(
        sessions.station_for_token("t1"),
        Some(&StationId("helm".into())),
        "the first claim is seated"
    );
    assert!(
        !sessions.is_spectator("t1"),
        "the winner is no longer a Spectator"
    );
    // t2's simultaneous claim of the now-occupied seat is a silent no-op.
    let result = handle_select_station(
        "t2",
        "helm",
        &mut sessions,
        GamePhase::InProgress,
        &ship_stations(),
    );
    assert!(
        result.outbound.is_empty(),
        "the losing claim takes the neutral no-op path: no self-addressed message"
    );
    assert_eq!(
        sessions.station_for_token("t2"),
        None,
        "the loser is not seated"
    );
    assert!(
        sessions.is_spectator("t2"),
        "the loser stays a Spectator — its role flag is untouched"
    );
}

#[test]
fn ineligible_spectator_claim_is_noop_and_stays_spectator() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_spectator("t1", true);
    sessions.set_eligibility(
        "t1",
        std::collections::HashSet::from([StationId("captain".into())]),
    );
    let result = handle_select_station(
        "t1",
        "captain",
        &mut sessions,
        GamePhase::InProgress,
        &ship_stations(),
    );
    assert!(
        result.outbound.is_empty(),
        "an ineligible claim takes the neutral no-op path"
    );
    assert_eq!(
        sessions.station_for_token("t1"),
        None,
        "an ineligible claim must not seat the player"
    );
    assert!(
        sessions.is_spectator("t1"),
        "an ineligible claim leaves the participant a Spectator"
    );
}

#[test]
fn spectator_claim_of_stale_taken_seat_is_noop() {
    // A crew member already holds the seat; the Spectator's roster was stale
    // (the seat looked open). Claiming an occupied seat no-ops.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Watcher".into()).unwrap();
    sessions.set_station("t1", Some(StationId("helm".into())));
    sessions.set_spectator("t2", true);
    let result = handle_select_station(
        "t2",
        "helm",
        &mut sessions,
        GamePhase::InProgress,
        &ship_stations(),
    );
    assert!(
        result.outbound.is_empty(),
        "a stale claim of an already-taken seat no-ops"
    );
    assert_eq!(
        sessions.station_for_token("t2"),
        None,
        "the Spectator is not seated"
    );
    assert!(
        sessions.is_spectator("t2"),
        "the Spectator stays a Spectator"
    );
}

#[test]
fn reconnect_after_spectator_claim_restores_seat_and_role() {
    let stations = ship_stations();
    let config = backfill_ship_config();
    let mut sessions = SessionManager::new();
    sessions.register("t1".into(), "Alice".into()).unwrap();
    sessions.set_spectator("t1", true);
    // The Spectator claims an open, eligible seat through the normal path.
    handle_select_station("t1", "captain", &mut sessions, GamePhase::Lobby, &stations);
    let captain_id = sessions.station_for_token("t1").cloned();
    assert!(captain_id.is_some(), "the claim seats the ex-Spectator");
    assert!(
        !sessions.is_spectator("t1"),
        "the claim clears the Spectator role"
    );
    // Drop the connection, then reconnect via Identify (mirrors
    // reconnect_restores_station_when_unclaimed).
    let mut resolver = ControlSourceResolver::new();
    let _ = process_disconnect_with_stations(
        "t1",
        &mut sessions,
        &stations,
        &config,
        &mut resolver,
        &HashMap::new(),
        GamePhase::Lobby,
        true,
    );
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Alice".into(),
    };
    let _ = dispatch(
        "t1",
        &msg,
        &mut sessions,
        GamePhase::Lobby,
        None,
        &stations,
        &default_ship_config(),
        true,
        &HashMap::new(),
    );
    assert_eq!(
        sessions.station_for_token("t1"),
        captain_id.as_ref(),
        "reconnect restores the seat claimed from the spectator surface"
    );
    assert!(
        !sessions.is_spectator("t1"),
        "the reconnected participant is crew, not a Spectator"
    );
}

#[test]
fn release_station_clears_player_station_field() {
    let mut sessions = sessions_with("t1", "Alice");
    pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Captain".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    pm_stations(
        "t1",
        &ClientMessage::ReleaseStation,
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert_eq!(
        sessions.station_for_token("t1"),
        None,
        "Player.station must be cleared after ReleaseStation"
    );
}

#[test]
fn select_station_includes_station_id_in_broadcast() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::SelectStation {
        station: "Helm".into(),
    };
    let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    let station_id = result.outbound.iter().find_map(|(_, m)| match m {
        ServerMessage::StationAssigned {
            token, station_id, ..
        } if token == "t1" => station_id.as_ref(),
        _ => None,
    });
    assert_eq!(
        station_id,
        Some(&StationId("helm".into())),
        "StationAssigned must carry station_id after SelectStation"
    );
}

// ── SetSpectator (issue #1105) ────────────────────────────────────────

#[test]
fn set_spectator_true_vacates_station_unreadies_and_broadcasts() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    sessions.set_ready("t1", true);
    let result = pm_stations(
        "t1",
        &ClientMessage::SetSpectator { spectator: true },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert!(sessions.is_spectator("t1"), "flag is set");
    assert_eq!(sessions.station_for_token("t1"), None, "seat vacated");
    assert!(!sessions.players()[0].ready, "ready cleared");
    assert!(
        result.outbound.iter().any(|(t, m)| matches!(m,
            ServerMessage::StationAssigned { token, station_id, .. }
                if token == "t1" && station_id.is_none())
            && *t == Target::All),
        "must broadcast StationAssigned {{ None }} for the vacated seat"
    );
    assert!(
        result.outbound.iter().any(|(_, m)| matches!(m,
            ServerMessage::ReadyChanged { token, ready: false } if token == "t1")),
        "must broadcast ReadyChanged {{ false }}"
    );
    assert!(
        result.outbound.iter().any(|(_, m)| matches!(m,
            ServerMessage::SpectatorChanged { token, spectator: true } if token == "t1")),
        "must broadcast SpectatorChanged {{ true }}"
    );
}

#[test]
fn set_spectator_true_in_lobby_clears_pending_rating_and_resets_broadcast() {
    // Mirror of `release_station_in_lobby_clears_pending_rating_and_resets_broadcast`:
    // a seated player picks a non-base rating, then Spectates. Entering
    // Spectator from a seat must leave that station in exactly the state a
    // `ReleaseStation` would — pending cleared, base `RatingChanged` broadcast
    // — so the next claimant does not inherit a stale complexity toggle.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    sessions.set_pending_rating(&StationId("captain".into()), "Simplified".into());

    let result = pm_stations(
        "t1",
        &ClientMessage::SetSpectator { spectator: true },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );

    assert!(
        sessions
            .pending_rating_for(&StationId("captain".into()))
            .is_none(),
        "vacated seat's pending rating must be cleared"
    );
    assert!(
        result.outbound.iter().any(|(t, m)| {
            matches!(t, Target::All)
                && matches!(
                    m,
                    ServerMessage::RatingChanged { station_id, rating_name }
                        if *station_id == StationId("captain".into()) && rating_name == "Std"
                )
        }),
        "must broadcast a base RatingChanged for the vacated seat"
    );
}

#[test]
fn set_spectator_true_during_inprogress_restores_backfill_rating() {
    // In-progress analog of
    // `release_station_during_inprogress_restores_backfill_rating`: a seated
    // player who Spectates mid-mission must hand the seat back to Backfill,
    // exactly like a `ReleaseStation`, not leave it dark at a stale rating.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    sessions.set_ready("t1", true);
    let result = pm_stations(
        "t1",
        &ClientMessage::SetSpectator { spectator: true },
        &mut sessions,
        GamePhase::InProgress,
        None,
    );
    assert!(result.outbound.iter().any(|(_, m)| matches!(
        m,
        ServerMessage::RatingChanged { station_id, rating_name }
        if station_id.0 == "captain" && rating_name == rating::BACKFILL_RATING
    )));
    assert_eq!(
        result.station_rating_update,
        Some((
            StationId("captain".into()),
            rating::BACKFILL_RATING.to_string()
        ))
    );
    assert_eq!(sessions.station_for_token("t1"), None);
    assert!(!sessions.players()[0].ready);
    assert!(sessions.is_spectator("t1"));
}

#[test]
fn set_spectator_false_clears_flag_without_vacating() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_spectator("t1", true);
    let result = pm_stations(
        "t1",
        &ClientMessage::SetSpectator { spectator: false },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert!(!sessions.is_spectator("t1"), "flag cleared");
    assert!(
        result.outbound.iter().any(|(_, m)| matches!(m,
            ServerMessage::SpectatorChanged { token, spectator: false } if token == "t1")),
        "must broadcast SpectatorChanged {{ false }}"
    );
    assert!(
        !result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. })),
        "no seat to vacate → no StationAssigned"
    );
}

#[test]
fn spectator_set_ready_is_noop_and_does_not_start_countdown() {
    let mut sessions = sessions_with("t1", "Watcher");
    sessions.set_spectator("t1", true);
    let result = pm_stations(
        "t1",
        &ClientMessage::SetReady { ready: true },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert!(
        !sessions.players()[0].ready,
        "a spectator's ready flag must stay false"
    );
    assert!(
        result.countdown_action.is_none(),
        "a spectator's SetReady must not start the countdown"
    );
    assert!(
        result.outbound.is_empty(),
        "a spectator's SetReady is a silent no-op (no ReadyChanged)"
    );
}

#[test]
fn seated_player_readies_while_spectator_present_starts_countdown() {
    // AC2: a sitting spectator neither counts toward readiness nor delays
    // start — the lone crew member readying is enough to start the game.
    let mut sessions = sessions_with("t1", "Crew");
    sessions.register("t2".into(), "Watcher".into()).unwrap();
    sessions.set_spectator("t2", true);
    let result = pm_stations(
        "t1",
        &ClientMessage::SetReady { ready: true },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert!(
        matches!(result.countdown_action, Some(CountdownAction::Start { .. })),
        "the game must start with only crew ready and a spectator sitting"
    );
}

#[test]
fn spectator_stays_spectator_across_reconnect_dispatch() {
    let mut sessions = sessions_with("t1", "Watcher");
    sessions.set_spectator("t1", true);
    sessions.disconnect("t1");
    // Reconnect via Identify with the same token (record never pruned).
    pm_stations(
        "t1",
        &ClientMessage::Identify {
            token: "t1".into(),
            name: "Watcher".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert!(
        sessions.is_spectator("t1"),
        "a reconnected participant stays a spectator"
    );
    assert!(sessions.players()[0].connected, "and is reconnected");
    assert!(
        !sessions.all_ready(),
        "and remains out of readiness after reconnect"
    );
}

// ── AFK presence (issue #1104) ────────────────────────────────────────

/// A station_ratings map with one seat rated, for the AFK snapshot path.
fn ratings_map(station: &str, rating: &str) -> HashMap<StationId, String> {
    HashMap::from([(StationId(station.into()), rating.to_string())])
}

#[test]
fn entering_afk_delegates_the_held_station_to_backfill_and_broadcasts() {
    // AC2: entering AFK moves the player's directly-held Station to Backfill
    // (which delegates every owned System to AI) exactly like a disconnect,
    // WITHOUT vacating the seat, and broadcasts AfkChanged.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    let ratings = ratings_map("captain", "Manual");

    let result = handle_set_afk("t1", true, &mut sessions, &ratings);

    assert!(sessions.is_afk("t1"), "the player is now AFK");
    assert_eq!(
        sessions.station_for_token("t1"),
        Some(&StationId("captain".into())),
        "AFK retains the seat — reconnect identity is preserved"
    );
    assert_eq!(
        result.station_rating_update,
        Some((
            StationId("captain".into()),
            rating::BACKFILL_RATING.to_string()
        )),
        "the held station is delegated to Backfill"
    );
    assert!(
        result.outbound.iter().any(|(_, m)| matches!(
            m,
            ServerMessage::RatingChanged { station_id, rating_name }
                if station_id.0 == "captain" && rating_name == rating::BACKFILL_RATING
        )),
        "broadcasts RatingChanged {{ Backfill }} for the delegated seat"
    );
    assert!(
        result.outbound.iter().any(|(t, m)| matches!(t, Target::All)
            && matches!(m, ServerMessage::AfkChanged { token, afk: true } if token == "t1")),
        "broadcasts AfkChanged {{ true }}"
    );
    // The seat is retained, so no StationAssigned rides along (no focus steal).
    assert!(
        !result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. })),
        "AFK never vacates the seat, so no StationAssigned is emitted"
    );
}

#[test]
fn afk_broadcast_carries_only_the_boolean_no_accessibility_detail() {
    // AC5 / AC7 privacy guard: the AfkChanged delta carries exactly the
    // token and the boolean. If a reason/profile ever leaked onto the wire
    // it would have to be a new field on this variant — this pins that the
    // variant stays a bare presence flag.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    let ratings = ratings_map("captain", "Manual");
    let result = handle_set_afk("t1", true, &mut sessions, &ratings);
    let afk_changed = result
        .outbound
        .iter()
        .find_map(|(_, m)| match m {
            ServerMessage::AfkChanged { token, afk } => Some((token.clone(), *afk)),
            _ => None,
        })
        .expect("an AfkChanged is broadcast");
    assert_eq!(afk_changed, ("t1".to_string(), true));
}

#[test]
fn leaving_afk_restores_the_exact_prior_rating() {
    // AC4: leaving AFK re-applies the SNAPSHOTTED prior rating (not a
    // hardcoded default), returning the seat to its prior coherent config.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));

    // Enter AFK with the seat on "Manual"; the seat is now Backfilled and
    // "Manual" is snapshotted.
    handle_set_afk("t1", true, &mut sessions, &ratings_map("captain", "Manual"));
    assert_eq!(
        sessions.afk_prev_rating_for("t1"),
        Some(&"Manual".to_string())
    );

    // Leave AFK: the live station_ratings now read Backfill (what we set),
    // but the restore must use the SNAPSHOT, not the live value.
    let result = handle_set_afk(
        "t1",
        false,
        &mut sessions,
        &ratings_map("captain", rating::BACKFILL_RATING),
    );

    assert!(!sessions.is_afk("t1"), "the player is no longer AFK");
    assert_eq!(
        result.station_rating_update,
        Some((StationId("captain".into()), "Manual".to_string())),
        "the exact pre-AFK rating is restored, not Backfill"
    );
    assert!(
        result.outbound.iter().any(|(_, m)| matches!(
            m,
            ServerMessage::RatingChanged { station_id, rating_name }
                if station_id.0 == "captain" && rating_name == "Manual"
        )),
        "broadcasts RatingChanged {{ Manual }} on return"
    );
    assert!(
        result.outbound.iter().any(|(_, m)| matches!(
            m,
            ServerMessage::AfkChanged { token, afk: false } if token == "t1"
        )),
        "broadcasts AfkChanged {{ false }}"
    );
    assert!(
        !result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. })),
        "return never re-sends StationAssigned — the console keeps focus"
    );
    assert_eq!(
        sessions.afk_prev_rating_for("t1"),
        None,
        "the snapshot is consumed on return"
    );
}

#[test]
fn afk_then_disconnect_then_reconnect_then_leave_restores_prior_rating() {
    // AC5 composition: a player enters AFK, then drops, then reconnects, then
    // leaves AFK. The disconnect writes Backfill into last_rating, but the
    // INDEPENDENT afk_prev_rating snapshot preserves the true prior rating,
    // so leaving AFK still restores "Manual" rather than the Backfill the
    // disconnect captured.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));

    handle_set_afk("t1", true, &mut sessions, &ratings_map("captain", "Manual"));

    // A disconnect while AFK: last_rating captures the CURRENT (Backfill)
    // rating, and afk must survive the drop.
    sessions.set_last_rating("t1", Some(rating::BACKFILL_RATING.to_string()));
    sessions.disconnect("t1");
    assert!(sessions.is_afk("t1"), "AFK survives the disconnect");
    assert_eq!(
        sessions.players()[0].last_rating,
        Some(rating::BACKFILL_RATING.to_string()),
        "last_rating captured Backfill — it must NOT be the restore source"
    );

    // Reconnect the same token.
    sessions.reconnect("t1");
    assert!(sessions.is_afk("t1"), "still AFK after reconnect");

    // Leave AFK: the snapshot, not last_rating, drives the restore.
    let result = handle_set_afk(
        "t1",
        false,
        &mut sessions,
        &ratings_map("captain", rating::BACKFILL_RATING),
    );
    assert_eq!(
        result.station_rating_update,
        Some((StationId("captain".into()), "Manual".to_string())),
        "the true pre-AFK rating survives the disconnect and is restored"
    );
}

// ── SelectStation: own station → no-op ────────────────────────────────

#[test]
fn select_own_station_is_noop() {
    let mut sessions = sessions_with("t1", "Alice");
    // First claim Captain
    pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Captain".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    // Try to select Captain again
    let result = pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Captain".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert!(
        result.outbound.is_empty(),
        "own station select should produce no output"
    );
}

// ── SelectStation: occupied station → no-op ───────────────────────────

#[test]
fn select_occupied_station_is_noop() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    // t1 takes "Helm" at 2P
    pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Helm".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    // t2 tries to take "Helm" too
    let result = pm_stations(
        "t2",
        &ClientMessage::SelectStation {
            station: "Helm".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert!(
        result.outbound.is_empty(),
        "occupied station select should produce no output"
    );
}

// ── SelectStation: unknown station name → no-op ───────────────────────

#[test]
fn select_unknown_station_is_noop() {
    let mut sessions = sessions_with("t1", "Alice");
    let result = pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Nonexistent".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert!(
        result.outbound.is_empty(),
        "unknown station select should produce no output"
    );
}

// ── SelectStation: swap → two StationAssigned broadcasts ─────────────

#[test]
fn select_new_station_while_in_another_emits_two_station_assigned() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    // t1 takes Helm at 2P
    pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Helm".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    // t2 takes Tactical at 2P
    pm_stations(
        "t2",
        &ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    // Now t1 tries to take Tactical — but it's occupied. Still a no-op.
    // Let's instead test t1 moves from Helm to an empty station by registering a 3rd player.
    // Actually at 2P there are only 2 stations (Helm, Tactical). Both taken.
    // Register 3rd player so 3P layout has Helm, Tactical, Repair.
    // Release t2's Tactical first, then t1 (on Helm) can swap to Tactical.
    pm_stations(
        "t2",
        &ClientMessage::ReleaseStation,
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    // t1 is on Helm, Tactical is now free → t1 swaps from Helm to Tactical (at 2P)
    let result = pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    let assigned: Vec<_> = result
        .outbound
        .iter()
        .filter_map(|(_, m)| match m {
            ServerMessage::StationAssigned { token, station, .. } if token == "t1" => {
                Some(station.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        assigned.len(),
        2,
        "swap should produce exactly 2 StationAssigned messages"
    );
    // One release (station=None) and one claim
    let has_release = assigned.iter().any(|s| s.is_none());
    let has_claim = assigned.iter().any(|s| s.as_deref() == Some("Tactical"));
    assert!(has_release, "swap must include a release StationAssigned");
    assert!(has_claim, "swap must include a claim StationAssigned");
}

// ── SelectStation / SetReady mid-game handoff ────────────────────────

#[test]
fn select_station_during_inprogress_keeps_backfill_until_ready() {
    let mut sessions = sessions_with("t1", "Alice");
    let station_ratings = HashMap::from([(
        StationId("captain".into()),
        rating::BACKFILL_RATING.to_string(),
    )]);
    let result = dispatch(
        "t1",
        &ClientMessage::SelectStation {
            station: "Captain".into(),
        },
        &mut sessions,
        GamePhase::InProgress,
        None,
        &ship_stations(),
        &default_ship_config(),
        true,
        &station_ratings,
    );
    assert!(
        result.outbound.iter().any(|(_, m)| matches!(
            m,
            ServerMessage::StationAssigned { token, station, .. }
            if token == "t1" && station.as_deref() == Some("Captain")
        )),
        "mid-game SelectStation should still assign the station for lobby help"
    );
    assert!(
        result.outbound.iter().any(|(_, m)| matches!(
            m,
            ServerMessage::ReadyChanged { token, ready }
            if token == "t1" && !*ready
        )),
        "mid-game SelectStation should force the claimant back to unready"
    );
    let ready_idx = result
        .outbound
        .iter()
        .position(|(_, m)| {
            matches!(
                m,
                ServerMessage::ReadyChanged { token, ready }
                if token == "t1" && !*ready
            )
        })
        .expect("ReadyChanged(false) not found");
    let assigned_idx = result
        .outbound
        .iter()
        .position(|(_, m)| {
            matches!(
                m,
                ServerMessage::StationAssigned { token, station, .. }
                if token == "t1" && station.as_deref() == Some("Captain")
            )
        })
        .expect("StationAssigned not found");
    assert!(
        ready_idx < assigned_idx,
        "mid-game claim must mark the player unready before assignment reaches clients"
    );
    assert!(
        !result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::RatingChanged { .. })),
        "mid-game SelectStation must not stop Backfill AI"
    );
    assert_eq!(
        result.station_rating_update, None,
        "mid-game SelectStation must not change control sources"
    );
    assert!(
        !sessions.players()[0].ready,
        "mid-game claimant must press Ready before joining"
    );
}

#[test]
fn set_ready_during_inprogress_resets_backfill_rating() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    let station_ratings = HashMap::from([(
        StationId("captain".into()),
        rating::BACKFILL_RATING.to_string(),
    )]);
    let result = dispatch(
        "t1",
        &ClientMessage::SetReady { ready: true },
        &mut sessions,
        GamePhase::InProgress,
        None,
        &ship_stations(),
        &default_ship_config(),
        true,
        &station_ratings,
    );
    assert!(result.outbound.iter().any(|(_, m)| matches!(
        m,
        ServerMessage::RatingChanged { station_id, rating_name }
        if station_id.0 == "captain" && rating_name == "Std"
    )));
    assert_eq!(
        result.station_rating_update,
        Some((StationId("captain".into()), "Std".to_string()))
    );
}

#[test]
fn select_station_during_lobby_does_not_emit_rating_changed() {
    let mut sessions = sessions_with("t1", "Alice");
    let result = pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Captain".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    // In lobby, no rating change should occur
    let has_rating_changed = result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::RatingChanged { .. }));
    assert!(
        !has_rating_changed,
        "lobby SelectStation must not emit RatingChanged"
    );
    assert!(
        result.station_rating_update.is_none(),
        "lobby SelectStation must not set station_rating_update"
    );
}

#[test]
fn release_station_during_inprogress_restores_backfill_rating() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    sessions.set_ready("t1", true);
    let result = dispatch(
        "t1",
        &ClientMessage::ReleaseStation,
        &mut sessions,
        GamePhase::InProgress,
        None,
        &ship_stations(),
        &default_ship_config(),
        true,
        &HashMap::new(),
    );
    assert!(result.outbound.iter().any(|(_, m)| matches!(
        m,
        ServerMessage::RatingChanged { station_id, rating_name }
        if station_id.0 == "captain" && rating_name == rating::BACKFILL_RATING
    )));
    assert_eq!(
        result.station_rating_update,
        Some((
            StationId("captain".into()),
            rating::BACKFILL_RATING.to_string()
        ))
    );
    assert_eq!(sessions.station_for_token("t1"), None);
    assert!(!sessions.players()[0].ready);
}

// ── ReleaseStation: broadcasts empty station ──────────────────────────

#[test]
fn release_station_sends_station_none_and_empty_consoles() {
    let mut sessions = sessions_with("t1", "Alice");
    // Claim first
    pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Captain".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    // Release
    let result = pm_stations(
        "t1",
        &ClientMessage::ReleaseStation,
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    let found = result.outbound.iter().any(|(_, m)| {
        matches!(m,
            ServerMessage::StationAssigned { token, station: None, .. } if token == "t1"
        )
    });
    assert!(
        found,
        "ReleaseStation must broadcast StationAssigned with station=None"
    );
}

// ── process_message: SetReady / auto-start ───────────────────────────

#[test]
fn set_ready_all_players_starts_game() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    // t1 sets ready — not all ready yet (t2 still unready)
    let result_t1 = pm(
        "t1",
        &ClientMessage::SetReady { ready: true },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert!(
        result_t1.new_phase.is_none(),
        "game must not start until all players are ready"
    );
    assert!(
        result_t1.countdown_action.is_none(),
        "no countdown until all players are ready"
    );
    // t2 sets ready — now all ready → start countdown
    let result_t2 = pm(
        "t2",
        &ClientMessage::SetReady { ready: true },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert_eq!(
        result_t2.countdown_action,
        Some(CountdownAction::Start {
            secs: 5,
            pending_phase: GamePhase::InProgress
        }),
        "countdown should start when all players are ready"
    );
    assert!(
        result_t2.new_phase.is_none(),
        "new_phase must be None during countdown"
    );
    assert!(
        !result_t2
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::GameStarted)),
        "GameStarted must not be sent during countdown"
    );
}

#[test]
fn set_ready_single_player_starts_game() {
    let mut sessions = sessions_with("t1", "Alice");
    let result = pm(
        "t1",
        &ClientMessage::SetReady { ready: true },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    assert_eq!(
        result.countdown_action,
        Some(CountdownAction::Start {
            secs: 5,
            pending_phase: GamePhase::InProgress
        }),
        "countdown should start when only player is ready"
    );
    assert!(
        result.new_phase.is_none(),
        "new_phase must be None during countdown"
    );
    assert!(
        !result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::GameStarted)),
        "GameStarted must not be sent during countdown"
    );
}

// ── ReturnToLobby ────────────────────────────────────────────────────

#[test]
fn return_to_lobby_during_game_over_transitions_phase_and_resets_ready() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    sessions.set_ready("t1", true);
    sessions.set_ready("t2", true);
    // Both players held a station this round; the return must release them.
    sessions.set_station("t1", Some(StationId("captain".into())));
    sessions.set_station("t2", Some(StationId("helm".into())));

    let result = pm(
        "t1",
        &ClientMessage::ReturnToLobby,
        &mut sessions,
        GamePhase::GameOver,
        None,
    );

    assert_eq!(result.new_phase, Some(GamePhase::Lobby));
    assert!(
        !sessions.players().iter().any(|p| p.ready),
        "ready flags must be cleared on return to lobby"
    );
    // Station claims must be cleared for every player (issue #756).
    assert_eq!(
        sessions.station_for_token("t1"),
        None,
        "t1's station must be cleared on return to lobby"
    );
    assert_eq!(
        sessions.station_for_token("t2"),
        None,
        "t2's station must be cleared on return to lobby"
    );
    for token in ["t1", "t2"] {
        assert!(
            result.outbound.iter().any(|(target, m)| {
                matches!(target, Target::All)
                    && matches!(m, ServerMessage::ReadyChanged { token: t, ready: false } if t == token)
            }),
            "expected ReadyChanged(false) broadcast for {token}"
        );
        assert!(
            result.outbound.iter().any(|(target, m)| {
                matches!(target, Target::All)
                    && matches!(
                        m,
                        ServerMessage::StationAssigned { token: t, station: None, station_id: None }
                        if t == token
                    )
            }),
            "expected StationAssigned(None) broadcast for {token}"
        );
    }
    assert!(
        result.outbound.iter().any(|(target, m)| {
            matches!(target, Target::All) && matches!(m, ServerMessage::ReturnedToLobby)
        }),
        "expected ReturnedToLobby broadcast"
    );
}

/// Player identity (token / name / connection) survives the return so the
/// second round starts with the same roster — only seats + ready clear
/// (issue #756).
#[test]
fn return_to_lobby_preserves_player_identity() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    sessions.set_station("t1", Some(StationId("captain".into())));

    let result = pm(
        "t1",
        &ClientMessage::ReturnToLobby,
        &mut sessions,
        GamePhase::GameOver,
        None,
    );

    assert_eq!(result.new_phase, Some(GamePhase::Lobby));
    assert_eq!(sessions.players().len(), 2, "roster must be preserved");
    let alice = sessions
        .players()
        .iter()
        .find(|p| p.token == "t1")
        .expect("t1 must still be registered");
    assert_eq!(alice.name, "Alice", "name must be preserved");
    assert!(alice.connected, "connection must be preserved");
    assert_eq!(alice.station, None, "seat must be cleared");
    // A player who held no seat gets no StationAssigned release.
    assert!(
        !result.outbound.iter().any(|(_, m)| matches!(
            m,
            ServerMessage::StationAssigned { token, .. } if token == "t2"
        )),
        "seatless player must not receive a StationAssigned release"
    );
}

#[test]
fn return_to_lobby_outside_game_over_is_noop() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_ready("t1", true);

    let result = pm(
        "t1",
        &ClientMessage::ReturnToLobby,
        &mut sessions,
        GamePhase::InProgress,
        None,
    );

    assert!(result.new_phase.is_none());
    assert!(result.outbound.is_empty());
    assert!(
        sessions.players().iter().any(|p| p.ready),
        "ready flags must be untouched outside GameOver"
    );
}

/// A peer's token is self-declared, so `Identify` must refuse the two
/// host-runtime shapes. Registering one would put a network peer behind a
/// name that command admission bypasses (`LOCAL_CONSOLE_TOKEN`, god mode
/// and every other simulation override) or that routes commands to an
/// NPC's own entity (`ai:`), and — since issue #939 —
/// `return_to_lobby_authority` reads the first as host authority and would
/// let a phone abort a mission in progress.
#[test]
fn identify_refuses_reserved_host_runtime_tokens() {
    for reserved in [
        crate::console_bridge::LOCAL_CONSOLE_TOKEN,
        crate::command_admission::ai_emit::AI_BACKFILL_TOKEN,
        "ai:some-npc-uuid",
    ] {
        assert!(
            is_reserved_token(reserved),
            "{reserved} must be classified reserved"
        );

        let mut sessions = SessionManager::new();
        let result = handle_identify(
            reserved,
            "Mallory",
            &mut sessions,
            GamePhase::Lobby,
            None,
            &default_stations(),
            &default_ship_config(),
            &HashMap::new(),
        );

        assert!(
            sessions.players().is_empty(),
            "{reserved} must not register a session"
        );
        assert!(
            result.outbound.is_empty(),
            "{reserved} must get no Welcome and must not be announced to anyone"
        );
        assert_eq!(
            return_to_lobby_authority(reserved) == ReturnToLobbyAuthority::Host,
            reserved == crate::console_bridge::LOCAL_CONSOLE_TOKEN,
            "only the local console reads as host authority"
        );
    }
}

/// A perfectly ordinary token is still accepted — the refusal above is
/// narrow, not a new class of rejected joins.
#[test]
fn identify_still_accepts_an_ordinary_peer_token() {
    let mut sessions = SessionManager::new();
    let result = handle_identify(
        "t1",
        "Alice",
        &mut sessions,
        GamePhase::Lobby,
        None,
        &default_stations(),
        &default_ship_config(),
        &HashMap::new(),
    );
    assert_eq!(sessions.players().len(), 1);
    assert!(result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::Welcome { .. })));
}

/// The host page's settings menu (issue #939) carries an "exit to lobby"
/// that has to work mid-mission, not only from the Game Over screen. It
/// arrives under `LOCAL_CONSOLE_TOKEN`, which is what opens `InProgress`.
#[test]
fn host_return_to_lobby_aborts_a_mission_in_progress() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_ready("t1", true);
    sessions.set_station("t1", Some(StationId("captain".into())));

    let result = pm(
        crate::console_bridge::LOCAL_CONSOLE_TOKEN,
        &ClientMessage::ReturnToLobby,
        &mut sessions,
        GamePhase::InProgress,
        None,
    );

    assert_eq!(result.new_phase, Some(GamePhase::Lobby));
    assert_eq!(sessions.station_for_token("t1"), None, "seat must clear");
    assert!(
        !sessions.players().iter().any(|p| p.ready),
        "ready flags must be cleared by the host's mid-mission abort"
    );
    assert!(
        result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::ReturnedToLobby)),
        "expected ReturnedToLobby broadcast"
    );
}

/// The host's extra reach stops at `InProgress` — the menu is not a way to
/// bounce the roster out of a lobby they are still filling.
#[test]
fn host_return_to_lobby_from_lobby_is_noop() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_ready("t1", true);

    let result = pm(
        crate::console_bridge::LOCAL_CONSOLE_TOKEN,
        &ClientMessage::ReturnToLobby,
        &mut sessions,
        GamePhase::Lobby,
        None,
    );

    assert!(result.new_phase.is_none());
    assert!(result.outbound.is_empty());
}

/// The mid-mission reach is the HOST's, not every participant's: a phone
/// must not be able to abandon a mission the rest of the crew is flying.
/// (`return_to_lobby_outside_game_over_is_noop` covers the same token from
/// the other direction; this one names the reason.)
#[test]
fn participant_cannot_abort_a_mission_in_progress() {
    let mut sessions = sessions_with("t1", "Alice");
    assert_eq!(
        return_to_lobby_authority("t1"),
        ReturnToLobbyAuthority::Participant
    );
    assert_eq!(
        return_to_lobby_authority(crate::console_bridge::LOCAL_CONSOLE_TOKEN),
        ReturnToLobbyAuthority::Host
    );

    let result = pm(
        "t1",
        &ClientMessage::ReturnToLobby,
        &mut sessions,
        GamePhase::InProgress,
        None,
    );
    assert!(result.new_phase.is_none());
}

/// The phone client's game-over overlay sends `ReturnToLobby`
/// (`client.html` `initReturnToLobby`), so it is deliberately NOT gated to
/// the host token — any connected player may trigger the return.
#[test]
fn return_to_lobby_from_network_token_is_accepted() {
    let mut sessions = sessions_with("t1", "Alice");
    let result = pm(
        "t1",
        &ClientMessage::ReturnToLobby,
        &mut sessions,
        GamePhase::GameOver,
        None,
    );
    assert_eq!(result.new_phase, Some(GamePhase::Lobby));
}

#[test]
fn control_system_in_lobby_produces_no_output() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::ControlSystem {
        target: crate::ship::system_registry::helm_thrust_system_id(),
        payload: crate::core::messages::SystemControlPayload::SetThrust { value: 0.5 },
    };
    let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result.outbound.is_empty());
    assert!(result.new_phase.is_none());
}

#[test]
fn reconnect_restores_station_when_unclaimed() {
    let stations = ship_stations();
    let config = backfill_ship_config();
    let mut sessions = SessionManager::new();
    sessions.register("t1".into(), "Alice".into()).unwrap();
    let captain_def = get_station(&stations, "Captain").unwrap();
    sessions.set_station("t1", Some(captain_def.id.clone()));
    let captain_id = sessions.station_for_token("t1").cloned();
    let mut resolver = ControlSourceResolver::new();
    let _ = process_disconnect_with_stations(
        "t1",
        &mut sessions,
        &stations,
        &config,
        &mut resolver,
        &HashMap::new(),
        GamePhase::Lobby,
        true,
    );
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Alice".into(),
    };
    let _result = dispatch(
        "t1",
        &msg,
        &mut sessions,
        GamePhase::Lobby,
        None,
        &stations,
        &default_ship_config(),
        true,
        &HashMap::new(),
    );
    let restored = sessions.station_for_token("t1");
    assert_eq!(
        restored,
        captain_id.as_ref(),
        "reconnecting player should have their previous station restored when free"
    );
}

#[test]
fn mid_game_disconnect_emits_player_left_and_no_station_cascade() {
    let stations = ship_stations();
    let mut sessions = SessionManager::new();
    sessions.register("t1".into(), "Alice".into()).unwrap();
    let captain_def = get_station(&stations, "Captain").unwrap();
    sessions.set_station("t1", Some(captain_def.id.clone()));
    let result = pd_stations("t1", &mut sessions, &stations);
    assert!(result
        .outbound
        .iter()
        .any(|(_, m)| { matches!(m, ServerMessage::PlayerLeft { token } if token == "t1") }));
    let any_station_assigned = result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. }));
    assert!(
        !any_station_assigned,
        "fixed roster: disconnect should NOT emit StationAssigned cascade"
    );
}

// ── Lobby join/leave station rules ────────────────────────────────────

#[test]
fn joining_player_is_never_auto_assigned_a_station_in_lobby() {
    let stations = ship_stations();
    let mut sessions = SessionManager::new();
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Alice".into(),
    };
    let result = dispatch(
        "t1",
        &msg,
        &mut sessions,
        GamePhase::Lobby,
        None,
        &stations,
        &default_ship_config(),
        true,
        &HashMap::new(),
    );
    let station = sessions.station_for_token("t1");
    assert!(
        station.is_none(),
        "new joiner should not be auto-assigned a station"
    );
    let auto_assigned = result.outbound.iter().any(|(_, m)| {
        matches!(m,
            ServerMessage::StationAssigned { token, station: Some(_), .. } if token == "t1"
        )
    });
    assert!(
        !auto_assigned,
        "no StationAssigned with a station should be emitted for the joiner"
    );
}

#[test]
fn existing_assigned_player_follows_next_when_second_player_joins() {
    let stations = ship_stations();
    let mut sessions = SessionManager::new();
    sessions.register("t1".into(), "Alice".into()).unwrap();
    let captain_def = get_station(&stations, "Captain").unwrap();
    sessions.set_station("t1", Some(captain_def.id.clone()));

    let msg = ClientMessage::Identify {
        token: "t2".into(),
        name: "Bob".into(),
    };
    let result = dispatch(
        "t2",
        &msg,
        &mut sessions,
        GamePhase::Lobby,
        None,
        &stations,
        &default_ship_config(),
        true,
        &HashMap::new(),
    );

    let t1_station = sessions.station_for_token("t1");
    assert_eq!(
        t1_station,
        Some(&captain_def.id),
        "t1 should keep their Captain station (fixed roster)"
    );

    let t1_station_assigned = result.outbound.iter().any(|(_, m)| {
        matches!(m,
            ServerMessage::StationAssigned { token, .. } if token == "t1"
        )
    });
    assert!(
        !t1_station_assigned,
        "t1 should not receive StationAssigned (no station change)"
    );

    let t2_station = sessions.station_for_token("t2");
    assert!(
        t2_station.is_none(),
        "t2 should not be auto-assigned a station on join"
    );
}

// ── SelectStation broadcast hardening (Option C) ─────────────────────

// ── process_message: SetReady ─────────────────────────────────────

#[test]
fn set_ready_broadcasts_ready_changed() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::SetReady { ready: true };
    let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result.outbound.iter().any(|(_, m)| matches!(m,
        ServerMessage::ReadyChanged { token, ready } if token == "t1" && *ready
    )));
    assert!(sessions.players()[0].ready);
}

#[test]
fn set_ready_auto_starts_when_all_ready() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    // t1 ready, t2 not → no auto-start
    let msg = ClientMessage::SetReady { ready: true };
    let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(
        result.new_phase.is_none(),
        "must not start when t2 not ready"
    );
    assert!(
        result.countdown_action.is_none(),
        "no countdown when t2 not ready"
    );

    // t2 ready → countdown starts
    let msg = ClientMessage::SetReady { ready: true };
    let result = pm("t2", &msg, &mut sessions, GamePhase::Lobby, None);
    assert_eq!(
        result.countdown_action,
        Some(CountdownAction::Start {
            secs: 5,
            pending_phase: GamePhase::InProgress
        }),
        "must start countdown when all ready"
    );
    assert!(
        result.new_phase.is_none(),
        "new_phase must be None during countdown"
    );
    assert!(!result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::GameStarted)));
}

#[test]
fn set_ready_false_does_not_trigger_start() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::SetReady { ready: false };
    let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(
        result.new_phase.is_none(),
        "setting ready=false must not start"
    );
    assert!(result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::ReadyChanged { .. })));
}

#[test]
fn set_ready_in_progress_broadcasts_ready_changed_but_does_not_auto_start() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_ready("t1", true);
    // Now try SetReady in InProgress — broadcasts ReadyChanged but does NOT start
    let msg = ClientMessage::SetReady { ready: false };
    let result = pm("t1", &msg, &mut sessions, GamePhase::InProgress, None);
    // ReadyChanged is still broadcast (status update), but no GameStarted
    assert!(result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::ReadyChanged { .. })));
    assert!(
        result.new_phase.is_none(),
        "must not auto-start during InProgress"
    );
}

// ── C3/C4: AI backfill on disconnect + reconnect-yield ───────────────

#[test]
fn disconnect_with_station_emits_rating_changed_backfill() {
    let stations = ship_stations();
    let config = backfill_ship_config();
    let mut sessions = SessionManager::new();
    sessions.register("t1".into(), "Alice".into()).unwrap();
    sessions.set_station("t1", Some(StationId("captain".into())));
    let mut resolver = ControlSourceResolver::new();
    let station_ratings = HashMap::from([(StationId("captain".into()), "Manual".into())]);
    let result = process_disconnect_with_stations(
        "t1",
        &mut sessions,
        &stations,
        &config,
        &mut resolver,
        &station_ratings,
        GamePhase::Lobby,
        true,
    );
    // Must broadcast PlayerLeft
    assert!(result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::PlayerLeft { token } if token == "t1")));
    // Must broadcast RatingChanged { station_id: "captain", "Backfill" }
    assert!(result.outbound.iter().any(|(_, m)| matches!(
        m,
        ServerMessage::RatingChanged { station_id, rating_name }
        if station_id.0 == "captain" && rating_name == "Backfill"
    )));
    // station_rating_update must be set
    assert_eq!(
        result.station_rating_update,
        Some((StationId("captain".into()), "Backfill".to_string()))
    );
}

#[test]
fn disconnect_saves_pre_disconnect_rating_as_last_rating() {
    let stations = ship_stations();
    let config = backfill_ship_config();
    let mut sessions = SessionManager::new();
    sessions.register("t1".into(), "Alice".into()).unwrap();
    sessions.set_station("t1", Some(StationId("captain".into())));
    let mut resolver = ControlSourceResolver::new();
    let station_ratings = HashMap::from([(StationId("captain".into()), "Assisted".into())]);
    process_disconnect_with_stations(
        "t1",
        &mut sessions,
        &stations,
        &config,
        &mut resolver,
        &station_ratings,
        GamePhase::Lobby,
        true,
    );
    let last = sessions
        .players()
        .iter()
        .find(|p| p.token == "t1")
        .and_then(|p| p.last_rating.as_deref());
    assert_eq!(
        last,
        Some("Assisted"),
        "last_rating must capture pre-disconnect rating"
    );
}

#[test]
fn disconnect_without_station_emits_no_rating_changed() {
    let stations = ship_stations();
    let config = backfill_ship_config();
    let mut sessions = sessions_with("t1", "Alice");
    // No station set → spectator disconnect, no RatingChanged
    let mut resolver = ControlSourceResolver::new();
    let result = process_disconnect_with_stations(
        "t1",
        &mut sessions,
        &stations,
        &config,
        &mut resolver,
        &HashMap::new(),
        GamePhase::Lobby,
        true,
    );
    assert!(result.station_rating_update.is_none());
    let has_rating_changed = result
        .outbound
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::RatingChanged { .. }));
    assert!(
        !has_rating_changed,
        "spectator disconnect must not emit RatingChanged"
    );
}

#[test]
fn reconnect_after_disconnect_restores_station_and_restores_rating() {
    // Full cycle: t1 connects, selects Captain, disconnects (station kept,
    // backfill applied), then reconnects — gets Captain back with pre-disconnect rating.
    let stations = ship_stations();
    let config = backfill_ship_config();
    let mut sessions = sessions_with("t1", "Alice");
    // t1 selects Captain via pm_stations
    pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Captain".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    // Simulate t1 having a pre-disconnect rating of "Manual"
    let station_ratings = HashMap::from([(StationId("captain".into()), "Manual".into())]);
    let mut resolver = ControlSourceResolver::new();
    process_disconnect_with_stations(
        "t1",
        &mut sessions,
        &stations,
        &config,
        &mut resolver,
        &station_ratings,
        GamePhase::Lobby,
        true,
    );
    // t1 reconnects via Identify
    let reconnect_result = dispatch(
        "t1",
        &ClientMessage::Identify {
            token: "t1".into(),
            name: "Alice".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
        &stations,
        &default_ship_config(),
        true,
        &HashMap::new(),
    );
    // Station must be restored
    let restored_station = sessions.station_for_token("t1");
    assert!(
        restored_station.is_some(),
        "station must be restored on reconnect after disconnect"
    );
    // StationAssigned must be in outbound
    assert!(reconnect_result.outbound.iter().any(|(_, m)| matches!(
        m,
        ServerMessage::StationAssigned { token, station: Some(_), .. } if token == "t1"
    )));
    // RatingChanged must be in outbound (rating restored from last_rating → "Manual")
    assert!(reconnect_result.outbound.iter().any(|(_, m)| matches!(
        m,
        ServerMessage::RatingChanged { station_id, rating_name }
        if station_id.0 == "captain" && rating_name == "Manual"
    )));
    // station_rating_update must be set
    assert_eq!(
        reconnect_result.station_rating_update,
        Some((StationId("captain".into()), "Manual".to_string()))
    );
}

#[test]
fn reconnect_falls_back_to_spectator_when_station_claimed() {
    // t1 disconnects, t2 claims Captain, t1 reconnects → no restore.
    let stations = ship_stations();
    let config = backfill_ship_config();
    let mut sessions = sessions_with("t1", "Alice");
    sessions.register("t2".into(), "Bob".into()).unwrap();
    pm_stations(
        "t1",
        &ClientMessage::SelectStation {
            station: "Captain".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    let mut resolver = ControlSourceResolver::new();
    process_disconnect_with_stations(
        "t1",
        &mut sessions,
        &stations,
        &config,
        &mut resolver,
        &HashMap::new(),
        GamePhase::Lobby,
        true,
    );
    // t2 claims Captain while t1 is away
    pm_stations(
        "t2",
        &ClientMessage::SelectStation {
            station: "Captain".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
    );
    // t1 reconnects
    let reconnect_result = dispatch(
        "t1",
        &ClientMessage::Identify {
            token: "t1".into(),
            name: "Alice".into(),
        },
        &mut sessions,
        GamePhase::Lobby,
        None,
        &stations,
        &default_ship_config(),
        true,
        &HashMap::new(), // station_ratings shows nothing at Backfill since t2 claimed
    );
    let t1_station = sessions.station_for_token("t1");
    assert!(
        t1_station.is_none(),
        "t1 must NOT get Captain back when t2 has claimed it"
    );
    let _ = reconnect_result;
}

// ── Identify field clamping (issue #602) ─────────────────────────────

#[test]
fn identify_clamps_token_to_64_chars() {
    let mut sessions = SessionManager::new();
    let long_token = "a".repeat(100);
    let msg = ClientMessage::Identify {
        token: long_token.clone(),
        name: "Bob".into(),
    };
    let _result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
    let player = sessions.players().iter().find(|p| p.name == "Bob").unwrap();
    assert_eq!(player.token.len(), 64);
    assert_eq!(player.token, "a".repeat(64));
}

#[test]
fn identify_clamps_name_to_32_chars() {
    let mut sessions = SessionManager::new();
    let long_name = "b".repeat(50);
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: long_name.clone(),
    };
    let _result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
    let player = sessions.players().iter().find(|p| p.token == "t1").unwrap();
    assert_eq!(player.name.len(), 32);
    assert_eq!(player.name, "b".repeat(32));
}

#[test]
fn identify_passes_short_token_and_name_unchanged() {
    let mut sessions = SessionManager::new();
    let msg = ClientMessage::Identify {
        token: "t1".into(),
        name: "Bob".into(),
    };
    let _result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
    let player = sessions.players().iter().find(|p| p.token == "t1").unwrap();
    assert_eq!(player.token, "t1");
    assert_eq!(player.name, "Bob");
}

#[test]
fn identify_clamped_token_still_reconnects() {
    let clamped_token = "a".repeat(64);
    let long_token = "a".repeat(100);
    let mut sessions = SessionManager::new();
    sessions
        .register(clamped_token.clone(), "Alice".into())
        .unwrap();
    sessions.set_ready(&clamped_token, true);
    sessions.disconnect(&clamped_token);
    // Reconnect with the long token → clamped to 64 chars → matches stored session.
    let msg = ClientMessage::Identify {
        token: long_token.clone(),
        name: "Alice".into(),
    };
    let _result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(sessions
        .players()
        .iter()
        .any(|p| p.token == clamped_token && p.connected));
}

// ── SetStationRating: pre-InProgress pending-rating path ───────────────

#[test]
fn set_station_rating_in_lobby_records_pending_and_broadcasts() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    let msg = ClientMessage::SetStationRating {
        rating_name: "Simplified".into(),
    };
    let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);

    assert_eq!(
        sessions.pending_rating_for(&StationId("captain".into())),
        Some(&"Simplified".to_string())
    );
    assert!(result.outbound.iter().any(|(t, m)| {
        matches!(t, Target::All)
            && matches!(
                m,
                ServerMessage::RatingChanged { station_id, rating_name }
                    if *station_id == StationId("captain".into()) && rating_name == "Simplified"
            )
    }));
    // No live resolver yet — nothing for the Bevy runtime to apply.
    assert!(result.station_rating_update.is_none());
}

#[test]
fn set_station_rating_in_lobby_rejects_unknown_rating_name() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    let msg = ClientMessage::SetStationRating {
        rating_name: "NotARealRating".into(),
    };
    let _result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(sessions
        .pending_rating_for(&StationId("captain".into()))
        .is_none());
}

#[test]
fn set_station_rating_in_lobby_ignored_when_sender_holds_no_station() {
    let mut sessions = sessions_with("t1", "Alice");
    let msg = ClientMessage::SetStationRating {
        rating_name: "Simplified".into(),
    };
    let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
    assert!(result
        .outbound
        .iter()
        .all(|(_, m)| !matches!(m, ServerMessage::RatingChanged { .. })));
}

#[test]
fn set_station_rating_ignored_during_in_progress_lobby_path() {
    // InProgress SetStationRating is handled by ship_plugin against the live
    // Ship entity, not this pure lobby handler — no pending state, no broadcast.
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    let msg = ClientMessage::SetStationRating {
        rating_name: "Simplified".into(),
    };
    let result = pm_stations("t1", &msg, &mut sessions, GamePhase::InProgress, None);
    assert!(sessions
        .pending_rating_for(&StationId("captain".into()))
        .is_none());
    assert!(result.outbound.is_empty());
}

#[test]
fn release_station_in_lobby_clears_pending_rating_and_resets_broadcast() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    sessions.set_pending_rating(&StationId("captain".into()), "Simplified".into());

    let result = pm_stations(
        "t1",
        &ClientMessage::ReleaseStation,
        &mut sessions,
        GamePhase::Lobby,
        None,
    );

    assert!(sessions
        .pending_rating_for(&StationId("captain".into()))
        .is_none());
    assert!(result.outbound.iter().any(|(t, m)| {
        matches!(t, Target::All)
            && matches!(
                m,
                ServerMessage::RatingChanged { station_id, rating_name }
                    if *station_id == StationId("captain".into()) && rating_name == "Std"
            )
    }));
}

#[test]
fn select_station_away_from_previous_in_lobby_clears_previous_pending_rating() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_station("t1", Some(StationId("captain".into())));
    sessions.set_pending_rating(&StationId("captain".into()), "Simplified".into());

    let msg = ClientMessage::SelectStation {
        station: "Helm".into(),
    };
    let _result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);

    assert!(sessions
        .pending_rating_for(&StationId("captain".into()))
        .is_none());
}

#[test]
fn return_to_lobby_clears_all_pending_ratings() {
    let mut sessions = sessions_with("t1", "Alice");
    sessions.set_pending_rating(&StationId("captain".into()), "Simplified".into());
    sessions.set_pending_rating(&StationId("tactical".into()), "Simplified".into());
    sessions.set_ready("t1", true);

    let _result = pm_stations(
        "t1",
        &ClientMessage::ReturnToLobby,
        &mut sessions,
        GamePhase::GameOver,
        None,
    );

    assert!(sessions.pending_ratings().is_empty());
}
