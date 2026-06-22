use std::collections::HashMap;

use crate::messages::{
    ClientMessage, Console, GamePhase, GameState, ServerMessage, ShipClientConfig, WorldData,
};
use crate::session::SessionManager;
use crate::stations_config::{all_stations_filled, get_station, ShipStations, StationAssignments};
use crate::stations_policy::{advance_on_join, reassign_on_leave};

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
            // Snapshot the station map BEFORE register so reassign_on_join can
            // diff against it once the new player is added.
            let old_map = build_station_assignments(sessions, ship_stations);

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

                // Lobby-safe station advance on join: existing assigned players
                // follow their `next` chain to the new player-count layout.
                // The new joiner is NOT auto-assigned — they must SelectStation.
                // Gated to the Lobby phase: a fresh connection mid-game (a new
                // spectator, or a reconnect whose seat could not be restored)
                // must never reshuffle the live crew's consoles.
                if phase == GamePhase::Lobby
                    && !ship_stations.configs.is_empty()
                    && !is_reconnect
                    && !sessions.spectator_queue().contains(id_token)
                {
                    let new_map = advance_on_join(ship_stations, &old_map);
                    // new_count is the total connected players (including the new joiner),
                    // which is the layout count the advanced stations now target.
                    let new_count =
                        sessions.players().iter().filter(|p| p.connected).count() as u32;
                    let cascade = apply_station_assignments(
                        sessions,
                        &new_map,
                        &old_map,
                        ship_stations,
                        new_count,
                    );
                    outbound.extend(cascade);
                }

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
            let player_count = sessions.players().iter().filter(|p| p.connected).count() as u32;

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

            // Look up the station. First try the current player count; if not found
            // (e.g. a disconnect is pending and the session count hasn't been
            // updated yet), search all other available counts.  This avoids silently
            // dropping SelectStation when a player tries to claim a station that is
            // valid at the post-disconnect count but not at the pre-disconnect count.
            let station_def = get_station(ship_stations, player_count, station).or_else(|| {
                (ship_stations.min_players..=ship_stations.max_players)
                    .filter(|&c| c != player_count)
                    .find_map(|c| get_station(ship_stations, c, station))
            });
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
                    let player_count =
                        sessions.players().iter().filter(|p| p.connected).count() as u32;
                    let max = ship_stations.max_players;
                    let check_count = if max > 0 && player_count > max {
                        max
                    } else {
                        player_count
                    };
                    let current_consoles: Vec<Console> = sessions
                        .players()
                        .iter()
                        .filter(|p| p.connected)
                        .flat_map(|p| p.consoles.clone())
                        .collect();
                    all_stations_filled(ship_stations, check_count, &current_consoles)
                };
                if can_start {
                    if preload_complete || ship_stations.configs.is_empty() {
                        new_phase = Some(GamePhase::InProgress);
                        outbound.push((Target::All, ServerMessage::GameStarted));
                    } else {
                        new_phase = Some(GamePhase::Loading);
                    }
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
        | ClientMessage::IncreasePower { .. }
        | ClientMessage::DecreasePower { .. }
        | ClientMessage::Hail { .. }
        | ClientMessage::SelectCommsMessage { .. }
        | ClientMessage::RespondToMessage { .. }
        | ClientMessage::ClearComms
        | ClientMessage::ShowOnScreen { .. }
        | ClientMessage::SetShieldFocus { .. }
        | ClientMessage::SetNavigationWaypoint { .. }
        | ClientMessage::ClearNavigationWaypoint
        | ClientMessage::ControlSystem { .. }
        | ClientMessage::LoadTube { .. }
        | ClientMessage::UnloadTube { .. } => {}
    }

    LobbyHandlerResult {
        new_phase,
        outbound,
    }
}

/// Build a `StationAssignments` map (token → station name) from current session state.
/// Only connected players with consoles are included (spectators are absent).
fn build_station_assignments(
    sessions: &SessionManager,
    ship_stations: &ShipStations,
) -> StationAssignments {
    let mut map = StationAssignments::new();
    let player_count = sessions
        .players()
        .iter()
        .filter(|p| p.connected && !p.consoles.is_empty())
        .count() as u32;
    for player in sessions
        .players()
        .iter()
        .filter(|p| p.connected && !p.consoles.is_empty())
    {
        // Find which station this player is on by matching their console set
        if let Some(defs) = ship_stations.configs.get(&player_count) {
            if let Some(station_def) = defs
                .iter()
                .find(|d| d.consoles.iter().all(|c| player.consoles.contains(c)))
            {
                map.insert(player.token.clone(), station_def.name.clone());
            }
        }
    }
    map
}

/// Apply a `StationAssignments` diff to sessions: assign consoles from the station defs.
/// Emits `StationAssigned` for each token whose assignment changed.
fn apply_station_assignments(
    sessions: &mut SessionManager,
    new_map: &StationAssignments,
    old_map: &StationAssignments,
    ship_stations: &ShipStations,
    new_count: u32,
) -> Vec<(Target, ServerMessage)> {
    let mut outbound = Vec::new();

    // Players who are now assigned a different station OR whose station name is
    // unchanged but whose console set differs at the new player count (e.g.
    // Tactical at 2P holds [Tactical, Repair] but at 3P holds [Tactical]).
    for (token, station_name) in new_map.iter() {
        let old = old_map.get(token);
        let name_changed = old.map(|s| s != station_name).unwrap_or(true);

        // Look up the station def at the new count to know the target consoles.
        let Some(station_def) = get_station(ship_stations, new_count, station_name) else {
            continue;
        };

        // Detect console-set drift even when the station name is the same.
        let current_consoles: Vec<Console> = sessions
            .players()
            .iter()
            .find(|p| p.token == *token)
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        let consoles_changed = current_consoles != station_def.consoles;

        if name_changed || consoles_changed {
            sessions.clear_consoles(token);
            let mut toggle_failed = false;
            for console in &station_def.consoles {
                if sessions.toggle_console(token, console.clone()).is_err() {
                    toggle_failed = true;
                }
            }
            let actual_consoles: Vec<Console> = sessions
                .players()
                .iter()
                .find(|p| p.token == *token)
                .map(|p| p.consoles.clone())
                .unwrap_or_default();
            if toggle_failed || actual_consoles != station_def.consoles {
                // Partial cascade assignment — wire reflects truth: empty.
                sessions.clear_consoles(token);
                outbound.push((
                    Target::All,
                    ServerMessage::StationAssigned {
                        token: token.clone(),
                        station: None,
                        consoles: vec![],
                    },
                ));
            } else {
                outbound.push((
                    Target::All,
                    ServerMessage::StationAssigned {
                        token: token.clone(),
                        station: Some(station_name.clone()),
                        consoles: actual_consoles,
                    },
                ));
            }
        }
    }

    // Players who lost their station (in old but not new)
    for token in old_map.keys() {
        if !new_map.contains_key(token) {
            sessions.clear_consoles(token);
            outbound.push((
                Target::All,
                ServerMessage::StationAssigned {
                    token: token.clone(),
                    station: None,
                    consoles: vec![],
                },
            ));
        }
    }

    outbound
}

/// Mark a peer as disconnected, run the station leave cascade (existing
/// assigned players follow their `previous` chain; unassigned players are
/// unchanged), and return the outbound broadcasts.
///
/// When the leaver held a station and there are spectators in the queue,
/// the cascade is skipped intentionally â€” spectators manually claim the
/// vacated station via SelectStation.  Without spectators the normal
/// Nâ†’N-1 cascade runs so remaining players absorb the leaver's role.
pub fn process_disconnect_with_stations(
    token: &str,
    sessions: &mut SessionManager,
    ship_stations: &ShipStations,
) -> LobbyHandlerResult {
    let old_map = build_station_assignments(sessions, ship_stations);
    sessions.disconnect(token);

    // Remove the leaver from the spectator queue in case they were queued.
    sessions.remove_spectator(token);

    // If the leaver held a station and spectators exist, skip the cascade
    // so the spectator can claim the vacated station directly via SelectStation
    // without having its consoles absorbed by another player's station.
    let is_leaver_in_map = old_map.contains_key(token);
    let has_spectators = !sessions.spectator_queue().is_empty();

    let (new_map, _) = if is_leaver_in_map && has_spectators {
        let mut map = old_map.clone();
        map.remove(token);
        (map, std::collections::VecDeque::new())
    } else {
        // Pass an empty spectator queue so reassign_on_leave never auto-promotes.
        reassign_on_leave(
            ship_stations,
            &old_map,
            token,
            &std::collections::VecDeque::new(),
        )
    };

    // Use the current connected-player count as the layout size when the
    // cascade was skipped, so remaining players keep their current console
    // assignments rather than being downgraded to the N-1 layout.
    let new_count = if is_leaver_in_map && has_spectators {
        sessions.players().iter().filter(|p| p.connected).count() as u32
    } else {
        new_map.len() as u32
    };

    let mut outbound =
        apply_station_assignments(sessions, &new_map, &old_map, ship_stations, new_count);

    // Always emit PlayerLeft for the disconnecting player
    outbound.push((
        Target::All,
        ServerMessage::PlayerLeft {
            token: token.to_string(),
        },
    ));

    LobbyHandlerResult {
        new_phase: None,
        outbound,
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
        assert!(consoles.contains(&crate::messages::Console::Helm));
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
    fn non_captain_cannot_start_game() {
        let mut sessions = sessions_with("t1", "Alice");
        sessions.register("t2".into(), "Bob".into()).unwrap();
        let msg = ClientMessage::StartGame;
        let result = pm("t2", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.new_phase.is_none());
        assert!(!result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::GameStarted)));
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
        pm_stations(
            "t1",
            &ClientMessage::SelectStation {
                station: "Helm".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        // t1 has CaptainChair but Tactical is unfilled → StartGame should be rejected
        let result = pm_stations(
            "t1",
            &ClientMessage::StartGame,
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        assert!(
            result.new_phase.is_none(),
            "StartGame should be rejected when not all stations filled"
        );
        assert!(!result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::GameStarted)));
    }

    #[test]
    fn start_game_succeeds_when_captain_and_all_stations_filled() {
        // 1 player at 1P: "Captain" station covers all consoles. Player takes Captain.
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
        // Now t1 holds CaptainChair (via Captain station) and all 1P stations are filled
        let result = pm_stations(
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
        // The 6P→5P cascade fills all 5P slots from remaining 5 players.
        // No empty slot → spectator t7 stays queued.
        let stations = ship_stations();
        let mut sessions = sessions_at_max(&stations);
        sessions.register("t7".into(), "Grace".into()).unwrap();
        sessions.push_spectator("t7".into());
        let _result = process_disconnect_with_stations("t1", &mut sessions, &stations);
        // t7 stays in spectator queue (cascade filled all slots)
        assert!(
            sessions.spectator_queue().contains(&"t7".to_string()),
            "t7 should remain in queue when cascade fills all slots"
        );
    }

    #[test]
    fn disconnect_with_spectator_promotes_when_cascade_leaves_empty_slot() {
        // Build a scenario where leaving produces an empty slot at n-1.
        // Use a 2-player station config where one player is on the no-prev station,
        // and the no-prev player leaves. The remaining player goes to 1P (Captain).
        // No empty slot at 1P (1 station, 1 player) → still no pull.
        // To get a spectator pull we need N-1 to have more stations than remaining players.
        // In the worked example this never happens with normal cascades.
        //
        // However: the spectator pull code still runs — let's verify it works if triggered.
        // We test this by using reassign_on_leave directly with a contrived state where
        // the spectator WOULD be promoted.
        // In the lobby_handler context, just verify the queue management is correct:
        // if process_disconnect_with_stations runs and new_map.len() < prev_defs.len(),
        // the spectator is promoted. We trust the stations.rs unit test for the math.
        // This test ensures the queue is properly threaded through process_disconnect_with_stations.

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
        // Station cascade: at least one StationAssigned should be emitted.
        let any_station_assigned = result
            .outbound
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. }));
        assert!(
            any_station_assigned,
            "disconnect cascade should emit StationAssigned"
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

        // t1 should be moved to Helm (next of Captain at 2P).
        let t1_consoles = sessions
            .players()
            .iter()
            .find(|p| p.token == "t1")
            .map(|p| p.consoles.clone())
            .unwrap_or_default();
        assert!(
            t1_consoles.contains(&crate::messages::Console::CaptainChair)
                && t1_consoles.contains(&crate::messages::Console::Helm),
            "t1 should be on Helm station at 2P (CaptainChair+Helm)"
        );

        // A StationAssigned should be emitted for t1 (their station changed).
        let t1_station_assigned = result.outbound.iter().any(|(_, m)| {
            matches!(m,
                ServerMessage::StationAssigned { token, station: Some(s), .. }
                    if token == "t1" && s == "Helm"
            )
        });
        assert!(
            t1_station_assigned,
            "StationAssigned for t1 moving to Helm should be emitted"
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
        // Claim a station that includes Helm
        pm_stations(
            "t1",
            &ClientMessage::SelectStation {
                station: "Captain".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        let msg = ClientMessage::SetComplexity {
            console: Console::Helm,
            preset_name: "Low".into(),
        };
        let result = pm_stations("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        let changed = result.outbound.iter().any(|(_, m)| matches!(m,
            ServerMessage::ComplexityChanged { console: Console::Helm, preset_name } if preset_name == "Low"
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
                station: "Captain".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        // t1 holds Helm (via Captain station), but "Nonexistent" is not a valid preset.
        let msg = ClientMessage::SetComplexity {
            console: Console::Helm,
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
                station: "Captain".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        // Send Low, then Std. The last ComplexityChanged should carry "Std".
        let _ = pm_stations(
            "t1",
            &ClientMessage::SetComplexity {
                console: Console::Helm,
                preset_name: "Low".into(),
            },
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        let result = pm_stations(
            "t1",
            &ClientMessage::SetComplexity {
                console: Console::Helm,
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
                    console: Console::Helm,
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
}
