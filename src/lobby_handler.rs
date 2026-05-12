use crate::messages::{
    ClientMessage, Console, GamePhase, GameState, ServerMessage, WorldData,
};
use crate::session::SessionManager;
use crate::stations::{all_stations_filled, get_station, ShipStations};

#[derive(Clone, Debug)]
pub enum Target {
    All,
    Token(String),
    AllExcept(String),
}

pub struct LobbyHandlerResult {
    pub new_phase: Option<GamePhase>,
    pub outbound: Vec<(Target, ServerMessage)>,
}

/// Derive the canonical `GameState` snapshot from live session + phase state.
/// Pure function — no Bevy, fully testable.
pub fn derive_game_state(sessions: &SessionManager, phase: &GamePhase, world: Option<&WorldData>) -> GameState {
    let world = match (phase, world) {
        (GamePhase::InProgress, Some(w)) => Some(w.clone()),
        _ => None,
    };
    GameState { phase: phase.clone(), players: sessions.players().to_vec(), world }
}

/// Handle one decoded `ClientMessage` from a peer. `token` is the session token
/// for non-Identify messages; for Identify it is ignored (the session token is
/// taken from the message body).
pub fn process_message(
    token: &str,
    msg: &ClientMessage,
    sessions: &mut SessionManager,
    phase: GamePhase,
    world: Option<&WorldData>,
    ship_stations: &ShipStations,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut new_phase = None;

    match msg {
        ClientMessage::Identify { token: id_token, name } => {
            if let Some(player) = sessions.reconnect(id_token) {
                let player = player.clone();
                let state = derive_game_state(sessions, &phase, world);
                outbound.push((Target::Token(id_token.clone()), ServerMessage::Welcome { state, ship_stations: ship_stations.clone() }));
                outbound.push((Target::AllExcept(id_token.clone()), ServerMessage::PlayerJoined { player }));
            } else if let Ok(player) = sessions.register(id_token.clone(), name.clone()) {
                let player = player.clone();
                let state = derive_game_state(sessions, &phase, world);
                outbound.push((Target::Token(id_token.clone()), ServerMessage::Welcome { state, ship_stations: ship_stations.clone() }));
                outbound.push((Target::AllExcept(id_token.clone()), ServerMessage::PlayerJoined { player }));
            }
        }
        ClientMessage::SetName { name } => {
            sessions.set_name(token, name.clone());
            outbound.push((Target::All, ServerMessage::NameChanged {
                token: token.to_string(),
                name: name.clone(),
            }));
        }
        ClientMessage::SelectStation { station } => {
            let player_count = sessions.players().iter().filter(|p| p.connected).count() as u32;

            if ship_stations.configs.is_empty() {
                // No station config loaded (e.g. in integration tests): fall back to
                // display-name-based console toggle for backward compatibility.
                let console = [Console::CaptainChair, Console::Helm, Console::Tactical, Console::Engineering, Console::Science]
                    .into_iter()
                    .find(|c| c.display_name() == station.as_str());
                if let Some(c) = console {
                    let _ = sessions.toggle_console(token, c);
                }
                let consoles = sessions.players()
                    .iter()
                    .find(|p| p.token == token)
                    .map(|p| p.consoles.clone())
                    .unwrap_or_default();
                let station_name = if consoles.is_empty() { None } else { Some(station.clone()) };
                outbound.push((Target::All, ServerMessage::StationAssigned {
                    token: token.to_string(),
                    station: station_name,
                    consoles,
                }));
                return LobbyHandlerResult { new_phase, outbound };
            }

            // Look up the station at the current player count
            let Some(station_def) = get_station(ship_stations, player_count, station) else {
                // Unknown station — silently drop
                return LobbyHandlerResult { new_phase, outbound };
            };

            // Check if sender already holds this exact station (own station → no-op)
            let sender_consoles: Vec<Console> = sessions.players()
                .iter()
                .find(|p| p.token == token)
                .map(|p| p.consoles.clone())
                .unwrap_or_default();
            let sender_is_on_station = sender_consoles == station_def.consoles;
            if sender_is_on_station {
                return LobbyHandlerResult { new_phase, outbound };
            }

            // Check if occupied by another connected player
            let occupied = station_def.consoles.iter().any(|c| {
                sessions.players().iter().any(|p| p.connected && p.token != token && p.consoles.contains(c))
            });
            if occupied {
                return LobbyHandlerResult { new_phase, outbound };
            }

            // Perform the assignment. If sender already has a station, release it first.
            let had_station = !sender_consoles.is_empty();
            if had_station {
                sessions.clear_consoles(token);
                outbound.push((Target::All, ServerMessage::StationAssigned {
                    token: token.to_string(),
                    station: None,
                    consoles: vec![],
                }));
            }

            // Claim new station — assign the station's consoles
            for console in &station_def.consoles {
                let _ = sessions.toggle_console(token, console.clone());
            }
            outbound.push((Target::All, ServerMessage::StationAssigned {
                token: token.to_string(),
                station: Some(station.clone()),
                consoles: station_def.consoles.clone(),
            }));
        }
        ClientMessage::ReleaseStation => {
            // Stub: release all consoles for this player.
            sessions.clear_consoles(token);
            outbound.push((Target::All, ServerMessage::StationAssigned {
                token: token.to_string(),
                station: None,
                consoles: vec![],
            }));
        }
        ClientMessage::StartGame => {
            if sessions.console_holder(Console::CaptainChair) == Some(token)
                && phase == GamePhase::Lobby
            {
                // Gate on all stations filled at the current connected player count,
                // but only when a station config is loaded. When configs is empty
                // (e.g. in integration tests without a ship TOML), allow start unconditionally.
                let can_start = if ship_stations.configs.is_empty() {
                    true
                } else {
                    let player_count = sessions.players().iter().filter(|p| p.connected).count() as u32;
                    let current_consoles: Vec<Console> = sessions.players()
                        .iter()
                        .filter(|p| p.connected)
                        .flat_map(|p| p.consoles.clone())
                        .collect();
                    all_stations_filled(ship_stations, player_count, &current_consoles)
                };
                if can_start {
                    new_phase = Some(GamePhase::InProgress);
                    outbound.push((Target::All, ServerMessage::GameStarted));
                }
            }
        }
        ClientMessage::ToggleRedAlert | ClientMessage::HelmInput { .. } | ClientMessage::SetView { .. } | ClientMessage::SetTarget { .. } | ClientMessage::FirePhaser | ClientMessage::SetPhaserMode { .. } | ClientMessage::Repair { .. } | ClientMessage::SetScienceTarget { .. } | ClientMessage::StartImpulseCharge | ClientMessage::CancelImpulse | ClientMessage::FireTorpedo { .. } => {}
    }

    LobbyHandlerResult { new_phase, outbound }
}

/// Mark a peer as disconnected and return the outbound broadcast.
pub fn process_disconnect(token: &str, sessions: &mut SessionManager) -> LobbyHandlerResult {
    sessions.disconnect(token);
    LobbyHandlerResult {
        new_phase: None,
        outbound: vec![(Target::All, ServerMessage::PlayerLeft { token: token.to_string() })],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{AsteroidInfo, WorldData};
    use crate::stations::ShipStations;

    fn sessions_with(token: &str, name: &str) -> SessionManager {
        let mut s = SessionManager::new();
        s.register(token.to_string(), name.to_string()).unwrap();
        s
    }

    fn default_stations() -> ShipStations { ShipStations::default() }

    fn pm(token: &str, msg: &ClientMessage, sessions: &mut SessionManager, phase: GamePhase, world: Option<&WorldData>) -> LobbyHandlerResult {
        process_message(token, msg, sessions, phase, world, &default_stations())
    }

    // ── process_disconnect ────────────────────────────────────────────────

    #[test]
    fn disconnect_broadcasts_player_left() {
        let mut sessions = sessions_with("t1", "Alice");
        let result = process_disconnect("t1", &mut sessions);
        assert!(result.outbound.iter().any(|(_, m)| {
            matches!(m, ServerMessage::PlayerLeft { token } if token == "t1")
        }));
    }

    #[test]
    fn disconnect_marks_player_as_disconnected() {
        let mut sessions = sessions_with("t1", "Alice");
        process_disconnect("t1", &mut sessions);
        assert!(!sessions.players()[0].connected);
    }

    #[test]
    fn disconnect_returns_no_phase_change() {
        let mut sessions = sessions_with("t1", "Alice");
        let result = process_disconnect("t1", &mut sessions);
        assert!(result.new_phase.is_none());
    }

    // ── process_message: Identify ─────────────────────────────────────────

    #[test]
    fn identify_new_player_sends_welcome_to_sender() {
        let mut sessions = SessionManager::new();
        let msg = ClientMessage::Identify { token: "t1".into(), name: "Alice".into() };
        let result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.iter().any(|(target, m)| {
            matches!(target, Target::Token(t) if t == "t1")
                && matches!(m, ServerMessage::Welcome { .. })
        }));
    }

    #[test]
    fn identify_new_player_broadcasts_player_joined_to_others() {
        let mut sessions = sessions_with("t2", "Bob");
        let msg = ClientMessage::Identify { token: "t1".into(), name: "Alice".into() };
        let result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.iter().any(|(target, m)| {
            matches!(target, Target::AllExcept(t) if t == "t1")
                && matches!(m, ServerMessage::PlayerJoined { .. })
        }));
    }

    #[test]
    fn identify_reconnect_sends_welcome_and_player_joined() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::Identify { token: "t1".into(), name: "Alice".into() };
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
        let msg = ClientMessage::Identify { token: "t1".into(), name: "Alice".into() };
        let result = pm("peer", &msg, &mut sessions, GamePhase::Lobby, None);
        let state = result.outbound.iter().find_map(|(_, m)| match m {
            ServerMessage::Welcome { state, .. } => Some(state.clone()),
            _ => None,
        }).unwrap();
        assert!(state.world.is_none());
    }

    #[test]
    fn welcome_during_in_progress_carries_world_some() {
        let mut sessions = SessionManager::new();
        let world = WorldData { asteroids: vec![AsteroidInfo { uuid: "test-uuid".into(), x: 1.0, z: 2.0, radius: 2.0, tags: vec![] }], asteroid_fields: vec![] };
        let msg = ClientMessage::Identify { token: "t1".into(), name: "Alice".into() };
        let result = pm("peer", &msg, &mut sessions, GamePhase::InProgress, Some(&world));
        let state = result.outbound.iter().find_map(|(_, m)| match m {
            ServerMessage::Welcome { state, .. } => Some(state.clone()),
            _ => None,
        }).unwrap();
        assert!(state.world.is_some());
    }

    // ── process_message: SetName ──────────────────────────────────────────

    #[test]
    fn set_name_broadcasts_name_changed() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::SetName { name: "Alicia".into() };
        let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.iter().any(|(_, m)| {
            matches!(m, ServerMessage::NameChanged { name, .. } if name == "Alicia")
        }));
    }

    // ── Helpers for station-aware tests ──────────────────────────────────

    fn ship_stations() -> ShipStations {
        let toml_str = include_str!("../assets/entities/player_ship.toml");
        crate::stations::parse_and_validate(toml_str).unwrap()
    }

    fn pm_stations(token: &str, msg: &ClientMessage, sessions: &mut SessionManager, phase: GamePhase, world: Option<&WorldData>) -> LobbyHandlerResult {
        process_message(token, msg, sessions, phase, world, &ship_stations())
    }

    // ── process_message: SelectStation / ReleaseStation ───────────────────

    #[test]
    fn select_station_broadcasts_station_assigned() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::SelectStation { station: "Captain".into() };
        let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.iter().any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. })));
    }

    #[test]
    fn release_station_broadcasts_station_assigned() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::ReleaseStation;
        let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.iter().any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. })));
    }

    // ── SelectStation: empty station → StationAssigned with consoles ──────

    #[test]
    fn select_empty_station_assigns_consoles_and_broadcasts() {
        let mut sessions = sessions_with("t1", "Alice");
        // 1 player → station "Captain" with CaptainChair,Helm,Tactical,Engineering
        let msg = ClientMessage::SelectStation { station: "Captain".into() };
        let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        let assigned = result.outbound.iter().find_map(|(_, m)| match m {
            ServerMessage::StationAssigned { token, station, consoles } if token == "t1" => {
                Some((station.clone(), consoles.clone()))
            }
            _ => None,
        });
        let (station_name, consoles) = assigned.expect("StationAssigned not found");
        assert_eq!(station_name, Some("Captain".to_string()));
        assert!(consoles.contains(&crate::messages::Console::CaptainChair));
        assert!(consoles.contains(&crate::messages::Console::Helm));
    }

    #[test]
    fn select_station_broadcast_target_is_all() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::SelectStation { station: "Captain".into() };
        let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.iter().any(|(t, m)| {
            matches!(t, Target::All) && matches!(m, ServerMessage::StationAssigned { .. })
        }));
    }

    // ── SelectStation: own station → no-op ────────────────────────────────

    #[test]
    fn select_own_station_is_noop() {
        let mut sessions = sessions_with("t1", "Alice");
        // First claim Captain
        pm_stations("t1", &ClientMessage::SelectStation { station: "Captain".into() }, &mut sessions, GamePhase::Lobby, None);
        // Try to select Captain again
        let result = pm_stations("t1", &ClientMessage::SelectStation { station: "Captain".into() }, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.is_empty(), "own station select should produce no output");
    }

    // ── SelectStation: occupied station → no-op ───────────────────────────

    #[test]
    fn select_occupied_station_is_noop() {
        let mut sessions = sessions_with("t1", "Alice");
        sessions.register("t2".into(), "Bob".into()).unwrap();
        // t1 takes "Helm" at 2P
        pm_stations("t1", &ClientMessage::SelectStation { station: "Helm".into() }, &mut sessions, GamePhase::Lobby, None);
        // t2 tries to take "Helm" too
        let result = pm_stations("t2", &ClientMessage::SelectStation { station: "Helm".into() }, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.is_empty(), "occupied station select should produce no output");
    }

    // ── SelectStation: unknown station name → no-op ───────────────────────

    #[test]
    fn select_unknown_station_is_noop() {
        let mut sessions = sessions_with("t1", "Alice");
        let result = pm_stations("t1", &ClientMessage::SelectStation { station: "Nonexistent".into() }, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.is_empty(), "unknown station select should produce no output");
    }

    // ── SelectStation: swap → two StationAssigned broadcasts ─────────────

    #[test]
    fn select_new_station_while_in_another_emits_two_station_assigned() {
        let mut sessions = sessions_with("t1", "Alice");
        sessions.register("t2".into(), "Bob".into()).unwrap();
        // t1 takes Helm at 2P
        pm_stations("t1", &ClientMessage::SelectStation { station: "Helm".into() }, &mut sessions, GamePhase::Lobby, None);
        // t2 takes Tactical at 2P
        pm_stations("t2", &ClientMessage::SelectStation { station: "Tactical".into() }, &mut sessions, GamePhase::Lobby, None);
        // Now t1 tries to take Tactical — but it's occupied. Still a no-op.
        // Let's instead test t1 moves from Helm to an empty station by registering a 3rd player.
        // Actually at 2P there are only 2 stations (Helm, Tactical). Both taken. 
        // Register 3rd player so 3P layout has Helm, Tactical, Engineering. 
        // Release t2's Tactical first, then t1 (on Helm) can swap to Tactical.
        pm_stations("t2", &ClientMessage::ReleaseStation, &mut sessions, GamePhase::Lobby, None);
        // t1 is on Helm, Tactical is now free → t1 swaps from Helm to Tactical (at 2P)
        let result = pm_stations("t1", &ClientMessage::SelectStation { station: "Tactical".into() }, &mut sessions, GamePhase::Lobby, None);
        let assigned: Vec<_> = result.outbound.iter().filter_map(|(_, m)| match m {
            ServerMessage::StationAssigned { token, station, consoles } if token == "t1" => {
                Some((station.clone(), consoles.clone()))
            }
            _ => None,
        }).collect();
        assert_eq!(assigned.len(), 2, "swap should produce exactly 2 StationAssigned messages");
        // One release (station=None) and one claim
        let has_release = assigned.iter().any(|(s, c)| s.is_none() && c.is_empty());
        let has_claim = assigned.iter().any(|(s, _)| s.as_deref() == Some("Tactical"));
        assert!(has_release, "swap must include a release StationAssigned");
        assert!(has_claim, "swap must include a claim StationAssigned");
    }

    // ── ReleaseStation: broadcasts empty station ──────────────────────────

    #[test]
    fn release_station_sends_station_none_and_empty_consoles() {
        let mut sessions = sessions_with("t1", "Alice");
        // Claim first
        pm_stations("t1", &ClientMessage::SelectStation { station: "Captain".into() }, &mut sessions, GamePhase::Lobby, None);
        // Release
        let result = pm_stations("t1", &ClientMessage::ReleaseStation, &mut sessions, GamePhase::Lobby, None);
        let found = result.outbound.iter().any(|(_, m)| matches!(m,
            ServerMessage::StationAssigned { token, station: None, consoles } if token == "t1" && consoles.is_empty()
        ));
        assert!(found, "ReleaseStation must broadcast StationAssigned with station=None and empty consoles");
    }

    // ── process_message: StartGame ────────────────────────────────────────

    #[test]
    fn non_captain_cannot_start_game() {
        let mut sessions = sessions_with("t1", "Alice");
        sessions.register("t2".into(), "Bob".into()).unwrap();
        let msg = ClientMessage::StartGame;
        let result = pm("t2", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.new_phase.is_none());
        assert!(!result.outbound.iter().any(|(_, m)| matches!(m, ServerMessage::GameStarted)));
    }

    #[test]
    fn start_game_ignored_when_stations_not_all_filled() {
        // 1 player, captain station requires CaptainChair. Player has CaptainChair but
        // we use ship_stations which at 1P has "Captain" covering all consoles.
        // Since only 1 player and 1 station: if they hold CaptainChair it IS filled.
        // Use 2 players so there's a second station (Tactical) that won't be filled.
        let mut sessions = sessions_with("t1", "Alice");
        sessions.register("t2".into(), "Bob".into()).unwrap();
        // t1 takes Helm (CaptainChair station at 2P), Tactical station is empty
        pm_stations("t1", &ClientMessage::SelectStation { station: "Helm".into() }, &mut sessions, GamePhase::Lobby, None);
        // t1 has CaptainChair but Tactical is unfilled → StartGame should be rejected
        let result = pm_stations("t1", &ClientMessage::StartGame, &mut sessions, GamePhase::Lobby, None);
        assert!(result.new_phase.is_none(), "StartGame should be rejected when not all stations filled");
        assert!(!result.outbound.iter().any(|(_, m)| matches!(m, ServerMessage::GameStarted)));
    }

    #[test]
    fn start_game_succeeds_when_captain_and_all_stations_filled() {
        // 1 player at 1P: "Captain" station covers all consoles. Player takes Captain.
        let mut sessions = sessions_with("t1", "Alice");
        pm_stations("t1", &ClientMessage::SelectStation { station: "Captain".into() }, &mut sessions, GamePhase::Lobby, None);
        // Now t1 holds CaptainChair (via Captain station) and all 1P stations are filled
        let result = pm_stations("t1", &ClientMessage::StartGame, &mut sessions, GamePhase::Lobby, None);
        assert!(result.new_phase.is_some(), "StartGame should succeed");
        assert!(result.outbound.iter().any(|(_, m)| matches!(m, ServerMessage::GameStarted)));
    }

    #[test]
    fn helm_input_in_lobby_produces_no_output() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::HelmInput { thrust: 0.5, steering: 0.0 };
        let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.is_empty());
        assert!(result.new_phase.is_none());
    }
}
