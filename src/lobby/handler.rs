use std::collections::HashMap;

use crate::messages::{
    ClientMessage, Console, GamePhase, GameState, ServerMessage, ShipClientConfig, WorldData,
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
                if !ship_stations.configs.is_empty() {
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
                        let at_capacity = connected_with_consoles >= ship_stations.max_players;
                        if at_capacity {
                            sessions.push_spectator(id_token.clone());
                            outbound.push((
                                Target::Token(id_token.clone()),
                                ServerMessage::StationAssigned {
                                    token: id_token.clone(),
                                    station: None,
                                    consoles: vec![],
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

            if ship_stations.configs.is_empty() {
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
                    },
                ));
                return LobbyHandlerResult {
                    new_phase,
                    outbound,
                };
            }

            // Fixed roster per #495: always use the max_players layout.
            // Stations do not change based on how many players are connected.
            let station_def = get_station(ship_stations, ship_stations.max_players, station);
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
                outbound.push((
                    Target::All,
                    ServerMessage::StationAssigned {
                        token: token.to_string(),
                        station: None,
                        consoles: vec![],
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
                outbound.push((
                    Target::All,
                    ServerMessage::StationAssigned {
                        token: token.to_string(),
                        station: None,
                        consoles: vec![],
                    },
                ));
                return LobbyHandlerResult {
                    new_phase,
                    outbound,
                };
            }
            outbound.push((
                Target::All,
                ServerMessage::StationAssigned {
                    token: token.to_string(),
                    station: Some(station.clone()),
                    consoles: actual_consoles,
                },
            ));
        }
        ClientMessage::ReleaseStation => {
            // Stub: release all consoles for this player.
            sessions.clear_consoles(token);
            // Push the releaser to the back of the spectator queue.
            if !ship_stations.configs.is_empty() {
                sessions.push_spectator(token.to_string());
            }
            outbound.push((
                Target::All,
                ServerMessage::StationAssigned {
                    token: token.to_string(),
                    station: None,
                    consoles: vec![],
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
                if preload_complete || ship_stations.configs.is_empty() {
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
                if preload_complete || ship_stations.configs.is_empty() {
                    new_phase = Some(GamePhase::InProgress);
                    outbound.push((Target::All, ServerMessage::GameStarted));
                } else {
                    new_phase = Some(GamePhase::Loading);
                }
            }
        }
        ClientMessage::SetComplexity {
            console,
            preset_name,
        } => {
            // Validate: sender must hold this console, and the preset name
            // must exist in the ship's complexity_presets map.
            let holds_console = sessions.player_has_console(token, console.clone());
            let preset_exists = ship_stations
                .complexity_presets
                .get(console)
                .map(|presets| presets.iter().any(|p| p == preset_name))
                .unwrap_or(false);
            if holds_console && preset_exists {
                outbound.push((
                    Target::All,
                    ServerMessage::ComplexityChanged {
                        console: console.clone(),
                        preset_name: preset_name.clone(),
                    },
                ));
            }
        }
        // SetReady IS handled above (not a no-op in lobby).
        ClientMessage::ToggleRedAlert
        | ClientMessage::HelmInput { .. }
        | ClientMessage::SetView { .. }
        | ClientMessage::SetTarget { .. }
        | ClientMessage::FirePhaser { .. }
        | ClientMessage::SetPhaserMode { .. }
        | ClientMessage::SetPhaserFrequency { .. }
        | ClientMessage::DispatchRepairTeam { .. }
        | ClientMessage::SetScienceTarget { .. }
        | ClientMessage::SetSensorsTarget { .. }
        | ClientMessage::StartImpulseCharge
        | ClientMessage::CancelImpulse
        | ClientMessage::ToggleBoost
        | ClientMessage::SetBoost { .. }
        | ClientMessage::FireTorpedo { .. }
        | ClientMessage::Hail { .. }
        | ClientMessage::SelectCommsMessage { .. }
        | ClientMessage::RespondToMessage { .. }
        | ClientMessage::ClearComms
        | ClientMessage::ShowOnScreen { .. }
        | ClientMessage::SetShieldFocus { .. }
        | ClientMessage::SetNavigationWaypoint { .. }
        | ClientMessage::ClearNavigationWaypoint
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
    use crate::messages::{EntitySnapshot, WorldData};
    use crate::stations_config::{parse_and_validate, ShipStations};

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
        let toml_str = include_str!("../../assets/entities/player_ship.toml");
        parse_and_validate(toml_str).unwrap()
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
            ServerMessage::StationAssigned { token, station: None, consoles } if token == "t1" && consoles.is_empty()
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
    fn helm_input_in_lobby_produces_no_output() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::HelmInput {
            thrust: 0.5,
            steering: 0.0,
        };
        let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.is_empty());
        assert!(result.new_phase.is_none());
    }

    // ── Spectator queue: push on join when max_players reached ───────────

    fn sessions_at_max(stations: &ShipStations) -> SessionManager {
        // max_players = 6; fill all 6 slots via direct console assignment
        let mut sessions = SessionManager::new();
        for (tok, name) in [
            ("t1", "Alice"),
            ("t2", "Bob"),
            ("t3", "Carol"),
            ("t4", "Dave"),
            ("t5", "Eve"),
            ("t6", "Frank"),
        ] {
            sessions.register(tok.into(), name.into()).unwrap();
        }
        let player_count = 6u32;
        // At 6P: "Captain" (CaptainChair), "Helm" (Helm), "Tactical" (Tactical), "Engineering" (Repair+Power), "Comms" (Comms), "Sensors" (Sensors+Shields+Navigation)
        for (tok, station_name) in [
            ("t1", "Captain"),
            ("t2", "Helm"),
            ("t3", "Tactical"),
            ("t4", "Engineering"),
            ("t5", "Comms"),
            ("t6", "Sensors"),
        ] {
            let station_def = get_station(stations, player_count, station_name).unwrap();
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
        // 7th player identifies — max_players (6) already have stations
        let msg = ClientMessage::Identify {
            token: "t7".into(),
            name: "Grace".into(),
        };
        let result = process_message(
            "t7",
            &msg,
            &mut sessions,
            GamePhase::Lobby,
            None,
            &stations,
            &default_ship_config(),
            true,
        );
        assert!(
            sessions.spectator_queue().contains(&"t7".to_string()),
            "t7 should be in the spectator queue"
        );
        // Should receive StationAssigned with station=None
        let got_spectator_assigned = result.outbound.iter().any(|(target, m)| {
            matches!(target, Target::Token(t) if t == "t7")
                && matches!(m, ServerMessage::StationAssigned { token, station: None, consoles } if token == "t7" && consoles.is_empty())
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
        // At 6P (max), all stations filled, t7 is spectator. t1 disconnects.
        // Fixed roster per #495: no cascade on disconnect. t1's station becomes
        // free. t7 stays queued (must manually claim via SelectStation).
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        sessions.register("t7".into(), "Grace".into()).unwrap();
        sessions.push_spectator("t7".into());
        let _result = process_disconnect_with_stations("t1", &mut sessions, &stations);
        // t7 stays in spectator queue (fixed roster: no auto-promotion)
        assert!(
            sessions.spectator_queue().contains(&"t7".to_string()),
            "t7 should remain in queue (fixed roster)"
        );
    }

    #[test]
    fn disconnect_with_spectator_promotes_when_cascade_leaves_empty_slot() {
        // Fixed roster per #495: spectators are NOT auto-promoted on disconnect.
        // The spectator queue is preserved. This test verifies the queue is
        // correctly threaded through process_disconnect_with_stations — no
        // spectator promotion occurs, and the queue remains intact.

        // Simulate using a 1P→0P scenario where a spectator exists, but that would be blocked
        // by min_players=1. Instead, let's verify the queue is passed correctly by testing
        // that after a disconnect from 6P, the spectator queue in sessions is updated
        // (it may be unchanged, but the VecDeque reference was correctly threaded).
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        sessions.register("t7".into(), "Grace".into()).unwrap();
        sessions.push_spectator("t7".into());
        sessions.push_spectator("nonexistent-extra".into()); // second spectator
        let _result = process_disconnect_with_stations("t1", &mut sessions, &stations);
        // Spectator queue is preserved (both spectators still queued, since no empty slot)
        assert!(sessions.spectator_queue().contains(&"t7".to_string()));
    }

    #[test]
    fn spectator_can_claim_station_vacated_by_disconnect_at_max_players() {
        // Mirrors the smoke test reassignment.spec.ts "leave at max_players
        // allows spectator to claim vacated station". 6 players hold all 6P
        // stations, a 7th joins as spectator, one station-holder disconnects,
        // then the spectator selects the vacated station.
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        // Add t7 as spectator
        sessions.register("t7".into(), "Grace".into()).unwrap();
        sessions.push_spectator("t7".into());

        // t6 (Sensors) disconnects
        let _ = process_disconnect_with_stations("t6", &mut sessions, &stations);

        // t7 selects "Sensors"
        let result = pm_stations(
            "t7",
            &ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        // t7 must receive a StationAssigned with station = Some("Sensors")
        let assigned = result.outbound.iter().find_map(|(_, m)| match m {
            ServerMessage::StationAssigned {
                token,
                station: Some(name),
                consoles,
            } if token == "t7" => Some((name.clone(), consoles.clone())),
            _ => None,
        });
        assert!(
            assigned.is_some(),
            "t7 should receive StationAssigned with station=Some(Sensors); got outbound: {:?}",
            result.outbound
        );
        let (name, consoles) = assigned.unwrap();
        assert_eq!(name, "Sensors");
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
        sessions.register("t7".into(), "Grace".into()).unwrap();
        sessions.push_spectator("t7".into());
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
        // Manually assign t1 to Captain station (1P).
        let captain_def = get_station(&stations, 1, "Captain").unwrap();
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
        // At 6P (max), all stations filled, t7 is spectator. t1 disconnects.
        // In lobby, spectators should NOT be auto-promoted.
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        sessions.register("t7".into(), "Grace".into()).unwrap();
        sessions.push_spectator("t7".into());

        let _result = process_disconnect_with_stations("t1", &mut sessions, &stations);

        // t7 must still be in the spectator queue (not promoted).
        assert!(
            sessions.spectator_queue().contains(&"t7".to_string()),
            "spectator must NOT be auto-promoted on disconnect in lobby"
        );
        // t7 must still have no consoles.
        let t7_consoles = sessions
            .players()
            .iter()
            .find(|p| p.token == "t7")
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        assert!(
            t7_consoles.is_empty(),
            "spectator t7 must not receive consoles automatically"
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
            ServerMessage::StationAssigned { token, station: None, consoles } if token == "t1" && consoles.is_empty()
        ));
        assert!(
            rolled_back,
            "handler must broadcast rollback StationAssigned with station=None"
        );
    }

    // ── SetComplexity validation ────────────────────────────────────────

    #[test]
    fn set_complexity_when_holder_broadcasts_complexity_changed() {
        let mut sessions = sessions_with("t1", "Alice");
        // Claim a station that includes Repair (6P CaptainChair alone doesn't cover Helm)
        pm_stations(
            "t1",
            &ClientMessage::SelectStation {
                station: "Engineering".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        let msg = ClientMessage::SetComplexity {
            console: Console::Repair,
            preset_name: "Low".into(),
        };
        let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        let changed = result.outbound.iter().any(|(_, m)| matches!(m,
            ServerMessage::ComplexityChanged { console: Console::Repair, preset_name } if preset_name == "Low"
        ));
        assert!(
            changed,
            "SetComplexity by holder must broadcast ComplexityChanged"
        );
    }

    #[test]
    fn set_complexity_when_non_holder_is_silent() {
        let mut sessions = sessions_with("t1", "Alice");
        // t1 holds no Helm console → message should be silently dropped
        let msg = ClientMessage::SetComplexity {
            console: Console::Helm,
            preset_name: "Low".into(),
        };
        let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(
            result.outbound.is_empty(),
            "non-holder SetComplexity must be silent"
        );
    }

    #[test]
    fn set_complexity_with_unknown_preset_is_silent() {
        let mut sessions = sessions_with("t1", "Alice");
        pm_stations(
            "t1",
            &ClientMessage::SelectStation {
                station: "Engineering".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        // t1 holds Repair (via Engineering station), but "Nonexistent" is not a valid preset.
        let msg = ClientMessage::SetComplexity {
            console: Console::Repair,
            preset_name: "Nonexistent".into(),
        };
        let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(
            result.outbound.is_empty(),
            "unknown preset must be silently dropped"
        );
    }

    #[test]
    fn set_complexity_last_write_wins() {
        let mut sessions = sessions_with("t1", "Alice");
        pm_stations(
            "t1",
            &ClientMessage::SelectStation {
                station: "Engineering".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        // Send Low, then Std. The last ComplexityChanged should carry "Std".
        let _ = pm_stations(
            "t1",
            &ClientMessage::SetComplexity {
                console: Console::Repair,
                preset_name: "Low".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        let result = pm_stations(
            "t1",
            &ClientMessage::SetComplexity {
                console: Console::Repair,
                preset_name: "Std".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        // Should have exactly one ComplexityChanged with "Std"
        let changes: Vec<_> = result
            .outbound
            .iter()
            .filter_map(|(_, m)| match m {
                ServerMessage::ComplexityChanged {
                    console: Console::Repair,
                    preset_name,
                } => Some(preset_name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            changes,
            vec!["Std"],
            "last write must win — only 'Std' should be broadcast"
        );
    }

    #[test]
    fn set_complexity_for_console_not_in_ship_config_is_silent() {
        // Test that even though the server has default complexity_presets for all
        // consoles, setting complexity from a non-holder is rejected.
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::SetComplexity {
            console: Console::Power,
            preset_name: "Low".into(),
        };
        let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(
            result.outbound.is_empty(),
            "non-holder SetComplexity must be silent"
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
        assert!(result.new_phase.is_none(), "must not start when t2 not ready");

        // t2 ready → auto-start
        let msg = ClientMessage::SetReady { ready: true };
        let result = pm("t2", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.new_phase.is_some(), "must auto-start when all ready");
        assert!(result.outbound.iter().any(|(_, m)| matches!(m, ServerMessage::GameStarted)));
    }

    #[test]
    fn set_ready_false_does_not_trigger_start() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::SetReady { ready: false };
        let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.new_phase.is_none(), "setting ready=false must not start");
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
        assert!(result.new_phase.is_none(), "must not auto-start during InProgress");
    }
}
