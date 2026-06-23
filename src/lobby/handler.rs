use std::collections::HashMap;

use crate::messages::{
    ClientMessage, Console, GamePhase, GameState, ServerMessage, ShipClientConfig, StationId,
    WorldData,
};
use crate::session::SessionManager;
use crate::stations_config::{get_station, ShipStations};

#[derive(Clone, Debug, PartialEq)]
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
pub fn derive_game_state(
    sessions: &SessionManager,
    phase: &GamePhase,
    world: Option<&WorldData>,
) -> GameState {
    let world = match (phase, world) {
        (GamePhase::InProgress, Some(w)) => Some(w.clone()),
        _ => None,
    };
    GameState {
        phase: phase.clone(),
        players: sessions.players().to_vec(),
        complexity: HashMap::new(),
        world,
    }
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
    ship_config: &ShipClientConfig,
    // When `false`, `StartGame` transitions to `Loading` instead of `InProgress`.
    preload_complete: bool,
    // Per-station active ratings to embed in Welcome for (re)connecting clients.
    station_ratings: &HashMap<StationId, String>,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut new_phase = None;

    match msg {
        ClientMessage::Identify {
            token: id_token,
            name,
        } => {
            let is_reconnect = sessions.reconnect(id_token).is_some();
            let joined = if is_reconnect {
                true
            } else {
                sessions.register(id_token.clone(), name.clone()).is_ok()
            };

            if joined {
                // Send Welcome and PlayerJoined
                let player = sessions
                    .players()
                    .iter()
                    .find(|p| p.token == *id_token)
                    .cloned()
                    .unwrap();
                let state = derive_game_state(sessions, &phase, world);
                outbound.push((
                    Target::Token(id_token.clone()),
                    ServerMessage::Welcome {
                        state,
                        ship_stations: ship_stations.clone(),
                        ship_config: ship_config.clone(),
                        station_ratings: station_ratings.clone(),
                    },
                ));
                outbound.push((
                    Target::AllExcept(id_token.clone()),
                    ServerMessage::PlayerJoined { player },
                ));

                // Fixed roster per #495: no advance_on_join cascade.
                // Existing players keep their stations when new players join.
                // The new joiner selects a station manually via SelectStation.

                // Spectator queue: if station config is loaded, check if stations are full.
                // If the joining player has no consoles (they haven't selected a station,
                // or reconnect cleared their consoles), and max_players slots are all taken,
                // push them to the spectator queue.
                if !ship_stations.stations.is_empty() {
                    let connected_with_consoles = sessions
                        .players()
                        .iter()
                        .filter(|p| p.connected && !p.consoles.is_empty())
                        .count() as u32;
                    let player_has_consoles = sessions
                        .players()
                        .iter()
                        .find(|p| p.token == *id_token)
                        .map(|p| !p.consoles.is_empty())
                        .unwrap_or(false);
                    let is_already_spectator = sessions.spectator_queue().contains(id_token);
                    if !player_has_consoles && !is_already_spectator {
                        let capacity = ship_stations.stations.len() as u32;
                        let at_capacity = connected_with_consoles >= capacity;
                        if at_capacity {
                            sessions.push_spectator(id_token.clone());
                            outbound.push((
                                Target::Token(id_token.clone()),
                                ServerMessage::StationAssigned {
                                    token: id_token.clone(),
                                    station: None,
                                    consoles: vec![],
                                    station_id: None,
                                },
                            ));
                        }
                    }
                }
            }
        }
        ClientMessage::SetName { name } => {
            sessions.set_name(token, name.clone());
            outbound.push((
                Target::All,
                ServerMessage::NameChanged {
                    token: token.to_string(),
                    name: name.clone(),
                },
            ));
        }
        ClientMessage::SelectStation { station } => {
            let _player_count = sessions.players().iter().filter(|p| p.connected).count() as u32;

            if ship_stations.stations.is_empty() {
                // No station config loaded (e.g. in integration tests): fall back to
                // display-name-based console toggle for backward compatibility.
                let console = [
                    Console::CaptainChair,
                    Console::Helm,
                    Console::Tactical,
                    Console::Repair,
                    Console::Sensors,
                    Console::Shields,
                    Console::Navigation,
                    Console::Power,
                    Console::Comms,
                ]
                .into_iter()
                .find(|c| c.display_name() == station.as_str());
                if let Some(c) = console {
                    let _ = sessions.toggle_console(token, c);
                }
                let consoles = sessions
                    .players()
                    .iter()
                    .find(|p| p.token == token)
                    .map(|p| p.consoles.clone())
                    .unwrap_or_default();
                let station_name = if consoles.is_empty() {
                    None
                } else {
                    Some(station.clone())
                };
                outbound.push((
                    Target::All,
                    ServerMessage::StationAssigned {
                        token: token.to_string(),
                        station: station_name,
                        consoles,
                        station_id: None,
                    },
                ));
                return LobbyHandlerResult {
                    new_phase,
                    outbound,
                };
            }

            // Fixed roster per #495 / B3: flat station list, no player-count dimension.
            let station_def = get_station(ship_stations, station);
            let Some(station_def) = station_def else {
                // Unknown station — silently drop
                return LobbyHandlerResult {
                    new_phase,
                    outbound,
                };
            };

            // Check if sender already holds this exact station (own station → no-op)
            let sender_consoles: Vec<Console> = sessions
                .players()
                .iter()
                .find(|p| p.token == token)
                .map(|p| p.consoles.clone())
                .unwrap_or_default();
            let sender_is_on_station = sender_consoles == station_def.consoles;
            if sender_is_on_station {
                return LobbyHandlerResult {
                    new_phase,
                    outbound,
                };
            }

            // Check if occupied by another connected player
            let occupied = station_def.consoles.iter().any(|c| {
                sessions
                    .players()
                    .iter()
                    .any(|p| p.connected && p.token != token && p.consoles.contains(c))
            });
            if occupied {
                return LobbyHandlerResult {
                    new_phase,
                    outbound,
                };
            }

            // Perform the assignment. If sender already has a station, release it first.
            let had_station = !sender_consoles.is_empty();
            if had_station {
                sessions.clear_consoles(token);
                sessions.set_station(token, None);
                outbound.push((
                    Target::All,
                    ServerMessage::StationAssigned {
                        token: token.to_string(),
                        station: None,
                        consoles: vec![],
                        station_id: None,
                    },
                ));
            }

            // Claim new station — assign the station's consoles.
            // Track toggle results so the broadcast reflects real session state
            // (defends against silent divergence when a console is unavailable
            // because the ship config does not declare it).
            let mut toggle_failed = false;
            for console in &station_def.consoles {
                if sessions.toggle_console(token, console.clone()).is_err() {
                    toggle_failed = true;
                }
            }
            let actual_consoles: Vec<Console> = sessions
                .players()
                .iter()
                .find(|p| p.token == token)
                .map(|p| p.consoles.clone())
                .unwrap_or_default();
            if toggle_failed || actual_consoles != station_def.consoles {
                // Partial assignment — roll back so wire == truth.
                sessions.clear_consoles(token);
                sessions.set_station(token, None);
                outbound.push((
                    Target::All,
                    ServerMessage::StationAssigned {
                        token: token.to_string(),
                        station: None,
                        consoles: vec![],
                        station_id: None,
                    },
                ));
                return LobbyHandlerResult {
                    new_phase,
                    outbound,
                };
            }
            sessions.set_station(token, Some(station_def.id.clone()));
            outbound.push((
                Target::All,
                ServerMessage::StationAssigned {
                    token: token.to_string(),
                    station: Some(station.clone()),
                    consoles: actual_consoles,
                    station_id: Some(station_def.id.clone()),
                },
            ));
        }
        ClientMessage::ReleaseStation => {
            // Stub: release all consoles for this player.
            sessions.clear_consoles(token);
            sessions.set_station(token, None);
            // Push the releaser to the back of the spectator queue.
            if !ship_stations.stations.is_empty() {
                sessions.push_spectator(token.to_string());
            }
            outbound.push((
                Target::All,
                ServerMessage::StationAssigned {
                    token: token.to_string(),
                    station: None,
                    consoles: vec![],
                    station_id: None,
                },
            ));
        }
        ClientMessage::SetReady { ready } => {
            sessions.set_ready(token, *ready);
            outbound.push((
                Target::All,
                ServerMessage::ReadyChanged {
                    token: token.to_string(),
                    ready: *ready,
                },
            ));
            // Auto-start when all connected players are ready.
            if phase == GamePhase::Lobby && sessions.all_ready() {
                if preload_complete || ship_stations.stations.is_empty() {
                    new_phase = Some(GamePhase::InProgress);
                    outbound.push((Target::All, ServerMessage::GameStarted));
                } else {
                    new_phase = Some(GamePhase::Loading);
                }
            }
        }
        ClientMessage::StartGame => {
            // Legacy compat: any player can force-start during Lobby.
            // CaptainChair and all-stations-filled checks removed per #495.
            // The primary start path is now SetReady + auto-start.
            if phase == GamePhase::Lobby {
                if preload_complete || ship_stations.stations.is_empty() {
                    new_phase = Some(GamePhase::InProgress);
                    outbound.push((Target::All, ServerMessage::GameStarted));
                } else {
                    new_phase = Some(GamePhase::Loading);
                }
            }
        }
        // SetReady IS handled above (not a no-op in lobby).
        ClientMessage::FirePhaser { .. }
        | ClientMessage::SetPhaserFrequency { .. }
        | ClientMessage::DispatchRepairTeam { .. }
        | ClientMessage::FireTorpedo { .. }
        | ClientMessage::ControlSystem { .. }
        | ClientMessage::SetStationRating { .. }
        | ClientMessage::SendCoordination { .. }
        | ClientMessage::LoadTube { .. }
        | ClientMessage::UnloadTube { .. } => {}
    }

    LobbyHandlerResult {
        new_phase,
        outbound,
    }
}

/// Mark a peer as disconnected. No station cascade — the leaver's station
/// becomes free for others to claim manually via SelectStation (fixed roster
/// per #495).
pub fn process_disconnect_with_stations(
    token: &str,
    sessions: &mut SessionManager,
    _ship_stations: &ShipStations,
) -> LobbyHandlerResult {
    sessions.disconnect(token);
    sessions.remove_spectator(token);

    LobbyHandlerResult {
        new_phase: None,
        outbound: vec![(
            Target::All,
            ServerMessage::PlayerLeft {
                token: token.to_string(),
            },
        )],
    }
}

/// Mark a peer as disconnected and return the outbound broadcast.
pub fn process_disconnect(token: &str, sessions: &mut SessionManager) -> LobbyHandlerResult {
    sessions.disconnect(token);
    LobbyHandlerResult {
        new_phase: None,
        outbound: vec![(
            Target::All,
            ServerMessage::PlayerLeft {
                token: token.to_string(),
            },
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{EntitySnapshot, StationId, WorldData};
    use crate::stations_config::{ShipStations, StationDef};

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

    fn pm(
        token: &str,
        msg: &ClientMessage,
        sessions: &mut SessionManager,
        phase: GamePhase,
        world: Option<&WorldData>,
    ) -> LobbyHandlerResult {
        process_message(
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
        let result = process_disconnect("t1", &mut sessions);
        assert!(result
            .outbound
            .iter()
            .any(|(_, m)| { matches!(m, ServerMessage::PlayerLeft { token } if token == "t1") }));
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
        // Flat roster matching player_ship.toml after B3.
        // 9 stations, one console each.
        ShipStations {
            stations: vec![
                StationDef {
                    id: StationId("captain".into()),
                    name: "Captain".into(),
                    description: "Command the bridge.".into(),
                    consoles: vec![Console::CaptainChair],
                    rank: "Cpt.".into(),
                    short_code: "CPT".into(),
                },
                StationDef {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "Pilot the ship.".into(),
                    consoles: vec![Console::Helm],
                    rank: "Ltn.".into(),
                    short_code: "HLM".into(),
                },
                StationDef {
                    id: StationId("tactical".into()),
                    name: "Tactical".into(),
                    description: "Manage weapons.".into(),
                    consoles: vec![Console::Tactical],
                    rank: "Ltn.".into(),
                    short_code: "TAC".into(),
                },
                StationDef {
                    id: StationId("repair".into()),
                    name: "Repair".into(),
                    description: "Repair systems.".into(),
                    consoles: vec![Console::Repair],
                    rank: "Ltn.".into(),
                    short_code: "ENG".into(),
                },
                StationDef {
                    id: StationId("sensors".into()),
                    name: "Sensors".into(),
                    description: "Monitor sensors.".into(),
                    consoles: vec![Console::Sensors],
                    rank: "Ens.".into(),
                    short_code: "SCI".into(),
                },
                StationDef {
                    id: StationId("shields".into()),
                    name: "Shields".into(),
                    description: "Manage shields.".into(),
                    consoles: vec![Console::Shields],
                    rank: "Ens.".into(),
                    short_code: "SHD".into(),
                },
                StationDef {
                    id: StationId("navigation".into()),
                    name: "Navigation".into(),
                    description: "Plot course.".into(),
                    consoles: vec![Console::Navigation],
                    rank: "Ens.".into(),
                    short_code: "NAV".into(),
                },
                StationDef {
                    id: StationId("power".into()),
                    name: "Power".into(),
                    description: "Manage power.".into(),
                    consoles: vec![Console::Power],
                    rank: "Ltn.".into(),
                    short_code: "PWR".into(),
                },
                StationDef {
                    id: StationId("comms".into()),
                    name: "Comms".into(),
                    description: "Hail contacts.".into(),
                    consoles: vec![Console::Comms],
                    rank: "Ens.".into(),
                    short_code: "COM".into(),
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
        process_message(
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
                consoles,
                ..
            } if token == "t1" => Some((station.clone(), consoles.clone())),
            _ => None,
        });
        let (station_name, consoles) = assigned.expect("StationAssigned not found");
        assert_eq!(station_name, Some("Captain".to_string()));
        assert!(consoles.contains(&crate::messages::Console::CaptainChair));
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
                token,
                station_id,
                ..
            } if token == "t1" => station_id.as_ref(),
            _ => None,
        });
        assert_eq!(
            station_id,
            Some(&StationId("helm".into())),
            "StationAssigned must carry station_id after SelectStation"
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
                ServerMessage::StationAssigned {
                    token,
                    station,
                    consoles,
                    ..
                } if token == "t1" => Some((station.clone(), consoles.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            assigned.len(),
            2,
            "swap should produce exactly 2 StationAssigned messages"
        );
        // One release (station=None) and one claim
        let has_release = assigned.iter().any(|(s, c)| s.is_none() && c.is_empty());
        let has_claim = assigned
            .iter()
            .any(|(s, _)| s.as_deref() == Some("Tactical"));
        assert!(has_release, "swap must include a release StationAssigned");
        assert!(has_claim, "swap must include a claim StationAssigned");
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
        let found = result.outbound.iter().any(|(_, m)| matches!(m,
            ServerMessage::StationAssigned { token, station: None, consoles, .. } if token == "t1" && consoles.is_empty()
        ));
        assert!(
            found,
            "ReleaseStation must broadcast StationAssigned with station=None and empty consoles"
        );
    }

    // ── process_message: StartGame ────────────────────────────────────────

    #[test]
    fn any_player_can_start_game() {
        // Legacy StartGame: any player (not just captain) can force-start.
        let mut sessions = sessions_with("t1", "Alice");
        sessions.register("t2".into(), "Bob".into()).unwrap();
        // t2 (non-captain) can StartGame — captain check removed per #495
        let result = pm(
            "t2",
            &ClientMessage::StartGame,
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        assert!(result.new_phase.is_some(), "any player can StartGame");
        assert!(result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::GameStarted)));
    }

    #[test]
    fn start_game_succeeds_even_with_empty_stations() {
        // Legacy StartGame: succeeds regardless of stations-filled state.
        // Empty seats are automated via AI backfill per #495.
        let mut sessions = sessions_with("t1", "Alice");
        let result = pm(
            "t1",
            &ClientMessage::StartGame,
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        assert!(result.new_phase.is_some(), "StartGame should succeed");
        assert!(result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::GameStarted)));
    }

    #[test]
    fn control_system_in_lobby_produces_no_output() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::ControlSystem {
            target: crate::system_registry::helm_system_id(),
            payload: crate::messages::SystemControlPayload::HelmInput {
                thrust: 0.5,
                steering: 0.0,
            },
        };
        let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.is_empty());
        assert!(result.new_phase.is_none());
    }

    // ── Spectator queue: push on join when max_players reached ───────────

    fn sessions_at_max(stations: &ShipStations) -> SessionManager {
        // Fill every station slot (9 stations after B3).
        let mut sessions = SessionManager::new();
        for (tok, name) in [
            ("t1", "Alice"),
            ("t2", "Bob"),
            ("t3", "Carol"),
            ("t4", "Dave"),
            ("t5", "Eve"),
            ("t6", "Frank"),
            ("t7", "Grace"),
            ("t8", "Heidi"),
            ("t9", "Ivan"),
        ] {
            sessions.register(tok.into(), name.into()).unwrap();
        }
        for (tok, station_name) in [
            ("t1", "Captain"),
            ("t2", "Helm"),
            ("t3", "Tactical"),
            ("t4", "Repair"),
            ("t5", "Sensors"),
            ("t6", "Shields"),
            ("t7", "Navigation"),
            ("t8", "Power"),
            ("t9", "Comms"),
        ] {
            let station_def = get_station(stations, station_name).unwrap();
            for console in &station_def.consoles {
                let _ = sessions.toggle_console(tok, console.clone());
            }
        }
        sessions
    }

    #[test]
    fn joining_when_max_players_filled_goes_to_spectator_queue() {
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        // 10th player identifies — all 9 stations already have holders
        let msg = ClientMessage::Identify {
            token: "t10".into(),
            name: "Judy".into(),
        };
        let result = process_message(
            "t10",
            &msg,
            &mut sessions,
            GamePhase::Lobby,
            None,
            &stations,
            &default_ship_config(),
            true,
            &HashMap::new(),
        );
        assert!(
            sessions.spectator_queue().contains(&"t10".to_string()),
            "t10 should be in the spectator queue"
        );
        // Should receive StationAssigned with station=None
        let got_spectator_assigned = result.outbound.iter().any(|(target, m)| {
            matches!(target, Target::Token(t) if t == "t10")
                && matches!(m, ServerMessage::StationAssigned { token, station: None, consoles, .. } if token == "t10" && consoles.is_empty())
        });
        assert!(
            got_spectator_assigned,
            "spectator should receive StationAssigned {{ station: None }}"
        );
    }

    #[test]
    fn release_station_pushes_token_to_spectator_queue() {
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        process_message(
            "t1",
            &ClientMessage::ReleaseStation,
            &mut sessions,
            GamePhase::Lobby,
            None,
            &stations,
            &default_ship_config(),
            true,
            &HashMap::new(),
        );
        assert!(
            sessions.spectator_queue().contains(&"t1".to_string()),
            "releaser should be pushed to spectator queue"
        );
    }

    #[test]
    fn reconnect_restores_previous_station_when_free() {
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        // t1 held the Captain station. They disconnect (seat reserved, no
        // cascade) then reconnect via Identify — the seat is still free.
        let captain_consoles = sessions
            .players()
            .iter()
            .find(|p| p.token == "t1")
            .map(|p| p.consoles.clone())
            .unwrap();
        sessions.disconnect("t1");
        let msg = ClientMessage::Identify {
            token: "t1".into(),
            name: "Alice".into(),
        };
        let _result = process_message(
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
        // Reconnect restores the previously-held seat.
        let restored = sessions
            .players()
            .iter()
            .find(|p| p.token == "t1")
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        assert_eq!(
            restored, captain_consoles,
            "reconnecting player should have their previous seat restored when free"
        );
    }

    #[test]
    fn disconnect_with_spectator_in_queue_cascade_fills_all_slots_spectator_stays() {
        // All 9 stations filled, t10 is spectator. t1 disconnects.
        // Fixed roster per #495: no cascade on disconnect. t1's station becomes
        // free. t10 stays queued (must manually claim via SelectStation).
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        sessions.register("t10".into(), "Judy".into()).unwrap();
        sessions.push_spectator("t10".into());
        let _result = process_disconnect_with_stations("t1", &mut sessions, &stations);
        // t10 stays in spectator queue (fixed roster: no auto-promotion)
        assert!(
            sessions.spectator_queue().contains(&"t10".to_string()),
            "t10 should remain in queue (fixed roster)"
        );
    }

    #[test]
    fn disconnect_with_spectator_promotes_when_cascade_leaves_empty_slot() {
        // Fixed roster per #495: spectators are NOT auto-promoted on disconnect.
        // The spectator queue is preserved. This test verifies the queue is
        // correctly threaded through process_disconnect_with_stations — no
        // spectator promotion occurs, and the queue remains intact.
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        sessions.register("t10".into(), "Judy".into()).unwrap();
        sessions.push_spectator("t10".into());
        sessions.push_spectator("nonexistent-extra".into()); // second spectator
        let _result = process_disconnect_with_stations("t1", &mut sessions, &stations);
        // Spectator queue is preserved (both spectators still queued)
        assert!(sessions.spectator_queue().contains(&"t10".to_string()));
    }

    #[test]
    fn spectator_can_claim_station_vacated_by_disconnect_at_max_players() {
        // All 9 stations filled, t10 joins as spectator, one station-holder
        // disconnects, then the spectator selects the vacated station.
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        // Add t10 as spectator
        sessions.register("t10".into(), "Judy".into()).unwrap();
        sessions.push_spectator("t10".into());

        // t6 (Shields) disconnects
        let _ = process_disconnect_with_stations("t6", &mut sessions, &stations);

        // t10 selects "Shields"
        let result = pm_stations(
            "t10",
            &ClientMessage::SelectStation {
                station: "Shields".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        // t10 must receive a StationAssigned with station = Some("Shields")
        let assigned = result.outbound.iter().find_map(|(_, m)| match m {
            ServerMessage::StationAssigned {
                token,
                station: Some(name),
                consoles,
                ..
            } if token == "t10" => Some((name.clone(), consoles.clone())),
            _ => None,
        });
        assert!(
            assigned.is_some(),
            "t10 should receive StationAssigned with station=Some(Shields); got outbound: {:?}",
            result.outbound
        );
        let (name, consoles) = assigned.unwrap();
        assert_eq!(name, "Shields");
        assert!(!consoles.is_empty(), "consoles should not be empty");
    }

    #[test]
    fn mid_game_disconnect_emits_player_left_and_station_assigned_changes() {
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        // t1 disconnects mid-game
        let result = process_disconnect_with_stations("t1", &mut sessions, &stations);
        // PlayerLeft must be in output
        assert!(result
            .outbound
            .iter()
            .any(|(_, m)| { matches!(m, ServerMessage::PlayerLeft { token } if token == "t1") }));
        // Fixed roster: no StationAssigned cascade on disconnect.
        let any_station_assigned = result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. }));
        assert!(
            !any_station_assigned,
            "fixed roster: disconnect should NOT emit StationAssigned cascade"
        );
    }

    #[test]
    fn spectator_queue_persists_across_lobby_to_in_progress_phase_transition() {
        // The spectator queue on SessionManager is phase-agnostic.
        // StartGame does not clear the queue — spectators remain spectators mid-game.
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        sessions.register("t10".into(), "Judy".into()).unwrap();
        sessions.push_spectator("t10".into());
        assert_eq!(
            sessions.spectator_queue().len(),
            1,
            "queue must have one spectator before transition"
        );
        // Simulate a phase transition by doing StartGame (which only changes new_phase).
        // The sessions object is unaffected — queue should still be there.
        let result = pm_stations(
            "t1",
            &ClientMessage::StartGame,
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        // t1 holds CaptainChair (Captain station at 6P has CaptainChair console)
        // Only the captain can start, and all stations must be filled.
        // Regardless of whether StartGame succeeds, the spectator queue must survive.
        let _ = result;
        assert_eq!(
            sessions.spectator_queue().len(),
            1,
            "spectator queue must persist regardless of phase change"
        );
    }

    // ── Lobby join/leave station rules ────────────────────────────────────

    #[test]
    fn joining_player_is_never_auto_assigned_a_station_in_lobby() {
        let stations = ship_stations();
        // t1 identifies fresh (no prior players).
        let mut sessions = SessionManager::new();
        let msg = ClientMessage::Identify {
            token: "t1".into(),
            name: "Alice".into(),
        };
        let result = process_message(
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
        // t1 must have no consoles — no auto-assignment.
        let consoles = sessions
            .players()
            .iter()
            .find(|p| p.token == "t1")
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        assert!(
            consoles.is_empty(),
            "new joiner should not be auto-assigned a station"
        );
        // No StationAssigned broadcast targeted at t1 with a real station.
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
        // t1 is already in the lobby and has selected Captain (1P station).
        let mut sessions = SessionManager::new();
        sessions.register("t1".into(), "Alice".into()).unwrap();
        // Manually assign t1 to Captain station.
        let captain_def = get_station(&stations, "Captain").unwrap();
        for c in &captain_def.consoles {
            let _ = sessions.toggle_console("t1", c.clone());
        }

        // t2 joins.
        let msg = ClientMessage::Identify {
            token: "t2".into(),
            name: "Bob".into(),
        };
        let result = process_message(
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

        // Fixed roster per #495: t1 keeps their station when a new player joins.
        let t1_consoles = sessions
            .players()
            .iter()
            .find(|p| p.token == "t1")
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        assert_eq!(
            t1_consoles, captain_def.consoles,
            "t1 should keep their Captain station (fixed roster)"
        );

        // No StationAssigned should be emitted for t1 (they kept their station).
        let t1_station_assigned = result.outbound.iter().any(|(_, m)| {
            matches!(m,
                ServerMessage::StationAssigned { token, .. } if token == "t1"
            )
        });
        assert!(
            !t1_station_assigned,
            "t1 should not receive StationAssigned (no station change)"
        );

        // t2 must NOT be auto-assigned.
        let t2_consoles = sessions
            .players()
            .iter()
            .find(|p| p.token == "t2")
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        assert!(
            t2_consoles.is_empty(),
            "t2 should not be auto-assigned a station on join"
        );
    }

    #[test]
    fn disconnect_does_not_promote_spectator_in_lobby() {
        // All 9 stations filled, t10 is spectator. t1 disconnects.
        // In lobby, spectators should NOT be auto-promoted.
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        sessions.register("t10".into(), "Judy".into()).unwrap();
        sessions.push_spectator("t10".into());

        let _result = process_disconnect_with_stations("t1", &mut sessions, &stations);

        // t10 must still be in the spectator queue (not promoted).
        assert!(
            sessions.spectator_queue().contains(&"t10".to_string()),
            "spectator must NOT be auto-promoted on disconnect in lobby"
        );
        // t10 must still have no consoles.
        let t10_consoles = sessions
            .players()
            .iter()
            .find(|p| p.token == "t10")
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        assert!(
            t10_consoles.is_empty(),
            "spectator t10 must not receive consoles automatically"
        );
    }

    // ── SelectStation broadcast hardening (Option C) ─────────────────────

    #[test]
    fn select_station_rolls_back_when_console_unavailable() {
        // If the ship config does not declare one of the station's consoles as
        // available, toggle_console will fail. The handler must roll back the
        // partial assignment and broadcast station=None — never lie on the wire
        // by claiming consoles the session does not actually hold.
        use crate::entity_config::EntityConfig;
        // Build an EntityConfig that omits CaptainChair (no [captain_console]).
        let toml_str = r#"
tags = ["player"]
[helm_console]
[weapons_console]
[engineering_console]
"#;
        let cfg = EntityConfig::from_toml(toml_str).unwrap();
        let mut sessions = SessionManager::new_with_config(&cfg);
        sessions.register("t1".into(), "Alice".into()).unwrap();

        let stations = ship_stations();
        // 1P "Captain" station requires CaptainChair, which is not available.
        let result = process_message(
            "t1",
            &ClientMessage::SelectStation {
                station: "Captain".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
            &stations,
            &default_ship_config(),
            true,
            &HashMap::new(),
        );

        // Session state: t1 must have no consoles (rollback).
        let consoles = sessions
            .players()
            .iter()
            .find(|p| p.token == "t1")
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        assert!(
            consoles.is_empty(),
            "session must roll back partial assignment, got {:?}",
            consoles
        );

        // Wire: must broadcast StationAssigned { station: None, consoles: [] }.
        let lied = result.outbound.iter().any(|(_, m)| {
            matches!(m,
                ServerMessage::StationAssigned { token, station: Some(_), .. } if token == "t1"
            )
        });
        assert!(
            !lied,
            "handler must not broadcast a station the session does not hold"
        );
        let rolled_back = result.outbound.iter().any(|(_, m)| matches!(m,
            ServerMessage::StationAssigned { token, station: None, consoles, .. } if token == "t1" && consoles.is_empty()
        ));
        assert!(
            rolled_back,
            "handler must broadcast rollback StationAssigned with station=None"
        );
    }

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

        // t2 ready → auto-start
        let msg = ClientMessage::SetReady { ready: true };
        let result = pm("t2", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.new_phase.is_some(), "must auto-start when all ready");
        assert!(result
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
}
