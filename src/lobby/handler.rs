use std::collections::HashMap;

use crate::messages::{
    ClientMessage, GamePhase, GameState, ServerMessage, ShipClientConfig, StationId, WorldData,
};
use crate::session::SessionManager;
use crate::ship::config::ShipConfig;
use crate::ship::control_source::ControlSourceResolver;
use crate::ship::rating;
use crate::stations_config::{get_station, ShipStations};

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    All,
    Token(String),
    AllExcept(String),
}

/// Signal from a pure handler to the Bevy runtime about the pre-game countdown.
#[derive(Clone, Debug, PartialEq)]
pub enum CountdownAction {
    /// Start countdown for the given number of seconds, then transition to
    /// the specified phase. Ignored if a countdown is already running.
    Start { secs: u32, pending_phase: GamePhase },
    /// Cancel any active countdown (someone unreadied, a new player joined, etc.).
    Cancel,
}

pub struct LobbyHandlerResult {
    pub new_phase: Option<GamePhase>,
    pub outbound: Vec<(Target, ServerMessage)>,
    /// Station rating change the Bevy runtime must apply to the control-source
    /// resolver and active_ratings map (in addition to broadcasting it).
    /// Set by `process_disconnect_with_stations` (backfill) and the reconnect
    /// branch of `process_message` (restore).
    pub station_rating_update: Option<(StationId, String)>,
    /// Optional countdown action. When set, `new_phase` and the corresponding
    /// `GameStarted` outbound message must NOT be produced for this round —
    /// the caller (Bevy system) handles the countdown lifecycle instead.
    pub countdown_action: Option<CountdownAction>,
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
    // When `false`, `SetReady` transitions to `Loading` instead of `InProgress`.
    preload_complete: bool,
    // Per-station active ratings to embed in Welcome for (re)connecting clients.
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
        ClientMessage::ReturnToLobby => handle_return_to_lobby(sessions, phase),
        ClientMessage::ConfirmScenario => handle_confirm_scenario(phase),
        ClientMessage::SetStationRating { rating_name } => {
            handle_set_station_rating(token, rating_name, sessions, phase, ship_stations)
        }
        // SetReady IS handled above (not a no-op in lobby). The seven runtime
        // variants are no-ops here — they are handled by the console server
        // plugins, not the lobby handler.
        ClientMessage::FirePhaser { .. }
        | ClientMessage::FireTorpedo { .. }
        | ClientMessage::ControlSystem { .. }
        | ClientMessage::SendCoordination { .. }
        | ClientMessage::LoadTube { .. }
        | ClientMessage::UnloadTube { .. } => LobbyHandlerResult {
            new_phase: None,
            outbound: Vec::new(),
            station_rating_update: None,
            countdown_action: None,
        },
    }
}

/// Handle `Identify`: (re)register the session, restore a held station on
/// reconnect-yield, and emit `Welcome` / `PlayerJoined` (and, at capacity, an
/// empty `StationAssigned`). `token` from the envelope is ignored — the session
/// token comes from the message body.
fn handle_identify(
    id_token: &str,
    name: &str,
    sessions: &mut SessionManager,
    phase: GamePhase,
    world: Option<&WorldData>,
    ship_stations: &ShipStations,
    ship_config: &ShipClientConfig,
    station_ratings: &HashMap<StationId, String>,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut station_rating_update: Option<(StationId, String)> = None;

    // Clamp token to 64 chars and name to 32 chars (issue #602).
    let id_token = id_token.chars().take(64).collect::<String>();
    let name = name.chars().take(32).collect::<String>();
    let is_reconnect = sessions.reconnect(&id_token).is_some();
    let joined = if is_reconnect {
        true
    } else {
        sessions.register(id_token.clone(), name.clone()).is_ok()
    };

    if joined {
        // Reconnect-yield: if the player had a station and it is still
        // unclaimed (no connected peer holds that station), restore
        // their seat and broadcast the pre-disconnect rating.
        let mut reconnect_station_update: Option<(StationId, String)> = None;
        if is_reconnect && !ship_stations.stations.is_empty() {
            let station_id = sessions.station_for_token(&id_token).cloned();
            if let Some(ref sid) = station_id {
                let occupied = sessions.players().iter().any(|p| {
                    p.connected && p.token != *id_token && p.station.as_ref() == Some(sid)
                });
                if !occupied {
                    // Capture restore info for post-Welcome broadcasts.
                    let last_rating = sessions
                        .players()
                        .iter()
                        .find(|p| p.token == *id_token)
                        .and_then(|p| p.last_rating.clone());
                    let rating = last_rating.unwrap_or_else(|| "Std".to_string());
                    reconnect_station_update = Some((sid.clone(), rating));
                } else {
                    sessions.set_station(&id_token, None);
                }
            }
        }

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

        // Broadcast restored station + rating (after Welcome so client is ready).
        if let Some((ref restored_sid, ref restored_rating)) = reconnect_station_update {
            if let Some(station_def) = ship_stations
                .stations
                .iter()
                .find(|sd| &sd.id == restored_sid)
            {
                outbound.push((
                    Target::All,
                    ServerMessage::StationAssigned {
                        token: id_token.clone(),
                        station: Some(station_def.name.clone()),
                        station_id: Some(restored_sid.clone()),
                    },
                ));
                outbound.push((
                    Target::All,
                    ServerMessage::RatingChanged {
                        station_id: restored_sid.clone(),
                        rating_name: restored_rating.clone(),
                    },
                ));
                // Clear last_rating now that it has been applied.
                sessions.set_last_rating(&id_token, None);
                station_rating_update = reconnect_station_update;
            }
        }

        // Fixed roster per #495: no advance_on_join cascade.
        // Existing players keep their stations when new players join.
        // The new joiner selects a station manually via SelectStation.

        if !ship_stations.stations.is_empty() {
            let connected_with_stations = sessions
                .players()
                .iter()
                .filter(|p| p.connected && p.station.is_some())
                .count() as u32;
            let player_has_station = sessions
                .players()
                .iter()
                .find(|p| p.token == *id_token)
                .map(|p| p.station.is_some())
                .unwrap_or(false);
            if !player_has_station {
                let capacity = ship_stations.stations.len() as u32;
                let at_capacity = connected_with_stations >= capacity;
                if at_capacity {
                    outbound.push((
                        Target::Token(id_token.clone()),
                        ServerMessage::StationAssigned {
                            token: id_token.clone(),
                            station: None,
                            station_id: None,
                        },
                    ));
                }
            }
        }
    }

    LobbyHandlerResult {
        new_phase: None,
        outbound,
        station_rating_update,
        countdown_action: None,
    }
}

/// Handle `SetName`: update the session's display name and broadcast
/// `NameChanged` to everyone.
fn handle_set_name(token: &str, name: &str, sessions: &mut SessionManager) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    sessions.set_name(token, name.to_string());
    outbound.push((
        Target::All,
        ServerMessage::NameChanged {
            token: token.to_string(),
            name: name.to_string(),
        },
    ));
    LobbyHandlerResult {
        new_phase: None,
        outbound,
        station_rating_update: None,
        countdown_action: None,
    }
}

/// Handle `SelectStation`: claim `station` for `token`, releasing any current
/// station first. Silently ignores unknown stations, own-station re-selects,
/// and stations occupied by another connected player.
pub(crate) fn handle_select_station(
    token: &str,
    station: &str,
    sessions: &mut SessionManager,
    phase: GamePhase,
    ship_stations: &ShipStations,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut station_rating_update: Option<(StationId, String)> = None;

    if ship_stations.stations.is_empty() {
        // No station config loaded — silently ignore (no backward-compat toggle).
        return LobbyHandlerResult {
            new_phase: None,
            outbound,
            station_rating_update: None,
            countdown_action: None,
        };
    }

    let station_def = get_station(ship_stations, station);
    let Some(station_def) = station_def else {
        return LobbyHandlerResult {
            new_phase: None,
            outbound,
            station_rating_update: None,
            countdown_action: None,
        };
    };

    // Check if sender already holds this station (own station → no-op)
    let sender_station = sessions.station_for_token(token).cloned();
    if sender_station.as_ref() == Some(&station_def.id) {
        return LobbyHandlerResult {
            new_phase: None,
            outbound,
            station_rating_update: None,
            countdown_action: None,
        };
    }

    // Check if occupied by another connected player
    let occupied = sessions
        .players()
        .iter()
        .any(|p| p.connected && p.token != token && p.station.as_ref() == Some(&station_def.id));
    if occupied {
        return LobbyHandlerResult {
            new_phase: None,
            outbound,
            station_rating_update: None,
            countdown_action: None,
        };
    }

    let mid_game_claim = phase == GamePhase::InProgress;
    if mid_game_claim {
        sessions.set_ready(token, false);
        outbound.push((
            Target::All,
            ServerMessage::ReadyChanged {
                token: token.to_string(),
                ready: false,
            },
        ));
    }

    // Release current station if held.
    if let Some(previous_station) = sender_station {
        sessions.set_station(token, None);
        outbound.push((
            Target::All,
            ServerMessage::StationAssigned {
                token: token.to_string(),
                station: None,
                station_id: None,
            },
        ));
        if mid_game_claim {
            let backfill = rating::BACKFILL_RATING.to_string();
            outbound.push((
                Target::All,
                ServerMessage::RatingChanged {
                    station_id: previous_station.clone(),
                    rating_name: backfill.clone(),
                },
            ));
            station_rating_update = Some((previous_station, backfill));
        } else {
            // Pre-InProgress: don't let a new claimant inherit a
            // stranger's lobby-chosen complexity toggle.
            sessions.clear_pending_rating(&previous_station);
            let base_rating = get_station(ship_stations, &previous_station.0)
                .and_then(|def| def.ratings.first().cloned())
                .unwrap_or_else(|| "Std".to_string());
            outbound.push((
                Target::All,
                ServerMessage::RatingChanged {
                    station_id: previous_station,
                    rating_name: base_rating,
                },
            ));
        }
    }

    // Assign the station.
    sessions.set_station(token, Some(station_def.id.clone()));
    outbound.push((
        Target::All,
        ServerMessage::StationAssigned {
            token: token.to_string(),
            station: Some(station.to_string()),
            station_id: Some(station_def.id.clone()),
        },
    ));

    // Mid-game claim is a pending join: the player can read the station
    // help and press Ready, but Backfill AI remains in control until
    // SetReady(true) applies the normal human rating.
    LobbyHandlerResult {
        new_phase: None,
        outbound,
        station_rating_update,
        countdown_action: None,
    }
}

/// Handle `ReleaseStation`: give up the caller's station, unready them, and
/// reset the vacated station's rating (Backfill mid-game, base rating pre-game).
pub(crate) fn handle_release_station(
    token: &str,
    sessions: &mut SessionManager,
    phase: GamePhase,
    ship_stations: &ShipStations,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut station_rating_update: Option<(StationId, String)> = None;

    let released_station = sessions.station_for_token(token).cloned();
    sessions.set_station(token, None);
    sessions.set_ready(token, false);
    outbound.push((
        Target::All,
        ServerMessage::StationAssigned {
            token: token.to_string(),
            station: None,
            station_id: None,
        },
    ));
    outbound.push((
        Target::All,
        ServerMessage::ReadyChanged {
            token: token.to_string(),
            ready: false,
        },
    ));
    if phase == GamePhase::InProgress {
        if let Some(station_id) = released_station {
            let backfill = rating::BACKFILL_RATING.to_string();
            outbound.push((
                Target::All,
                ServerMessage::RatingChanged {
                    station_id: station_id.clone(),
                    rating_name: backfill.clone(),
                },
            ));
            station_rating_update = Some((station_id, backfill));
        }
    } else if let Some(station_id) = released_station {
        // Pre-InProgress: don't let a new claimant inherit a
        // stranger's lobby-chosen complexity toggle.
        sessions.clear_pending_rating(&station_id);
        let base_rating = get_station(ship_stations, &station_id.0)
            .and_then(|def| def.ratings.first().cloned())
            .unwrap_or_else(|| "Std".to_string());
        outbound.push((
            Target::All,
            ServerMessage::RatingChanged {
                station_id,
                rating_name: base_rating,
            },
        ));
    }

    LobbyHandlerResult {
        new_phase: None,
        outbound,
        station_rating_update,
        countdown_action: None,
    }
}

/// Handle `SetReady`: record the caller's ready flag and broadcast it. During
/// Lobby, manage the 5-second start countdown; while InProgress, applying a
/// ready flag restores the caller's station to the default human rating.
pub(crate) fn handle_set_ready(
    token: &str,
    ready: bool,
    sessions: &mut SessionManager,
    phase: GamePhase,
    preload_complete: bool,
    ship_stations: &ShipStations,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut station_rating_update: Option<(StationId, String)> = None;
    let mut countdown_action = None;

    sessions.set_ready(token, ready);
    outbound.push((
        Target::All,
        ServerMessage::ReadyChanged {
            token: token.to_string(),
            ready,
        },
    ));
    // Start 5-second countdown when all players ready up during Lobby.
    // If someone unreadies during the countdown, cancel it.
    if phase == GamePhase::Lobby {
        if sessions.all_ready() {
            let pending_phase = if preload_complete || ship_stations.stations.is_empty() {
                GamePhase::InProgress
            } else {
                GamePhase::Loading
            };
            countdown_action = Some(CountdownAction::Start {
                secs: 5,
                pending_phase,
            });
        } else if !ready {
            // Unready while countdown may be active → cancel it.
            countdown_action = Some(CountdownAction::Cancel);
        }
    } else if phase == GamePhase::InProgress && ready {
        if let Some(station_id) = sessions.station_for_token(token).cloned() {
            let default_rating = "Std".to_string();
            outbound.push((
                Target::All,
                ServerMessage::RatingChanged {
                    station_id: station_id.clone(),
                    rating_name: default_rating.clone(),
                },
            ));
            station_rating_update = Some((station_id, default_rating));
        }
    }

    LobbyHandlerResult {
        new_phase: None,
        outbound,
        station_rating_update,
        countdown_action,
    }
}

/// Handle `ReturnToLobby`: from `GameOver`, reset ready flags + pending ratings,
/// broadcast the cleared readies and `ReturnedToLobby`, and transition to
/// `Lobby`. No-op in any other phase.
fn handle_return_to_lobby(sessions: &mut SessionManager, phase: GamePhase) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut new_phase: Option<GamePhase> = None;

    if phase == GamePhase::GameOver {
        sessions.reset_ready();
        sessions.clear_all_pending_ratings();
        for p in sessions.players() {
            outbound.push((
                Target::All,
                ServerMessage::ReadyChanged {
                    token: p.token.clone(),
                    ready: false,
                },
            ));
        }
        outbound.push((Target::All, ServerMessage::ReturnedToLobby));
        new_phase = Some(GamePhase::Lobby);
    }

    LobbyHandlerResult {
        new_phase,
        outbound,
        station_rating_update: None,
        countdown_action: None,
    }
}

/// Handle `ConfirmScenario`: broadcast `ScenarioLoaded` during Lobby.
fn handle_confirm_scenario(phase: GamePhase) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    if phase == GamePhase::Lobby {
        outbound.push((Target::All, ServerMessage::ScenarioLoaded));
    }
    LobbyHandlerResult {
        new_phase: None,
        outbound,
        station_rating_update: None,
        countdown_action: None,
    }
}

/// Handle `SetStationRating` in Lobby/Loading: validate the rating against the
/// station def, record it as pending, and broadcast the live toggle. InProgress
/// is handled by `ship_plugin::handle_station_rating_change` instead.
pub(crate) fn handle_set_station_rating(
    token: &str,
    rating_name: &str,
    sessions: &mut SessionManager,
    phase: GamePhase,
    ship_stations: &ShipStations,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();

    // InProgress is handled by `ship_plugin::handle_station_rating_change`
    // against the live Ship entity. Pre-spawn (Lobby/Loading), there is
    // no ControlSourceResolver to apply to yet, so record the choice as
    // "pending" and apply it once the ship spawns
    // (`spawn_game_start_entities`), while still broadcasting it live so
    // other lobby clients see the toggle update immediately.
    if phase == GamePhase::Lobby || phase == GamePhase::Loading {
        if let Some(station_id) = sessions.station_for_token(token).cloned() {
            let valid = get_station(ship_stations, &station_id.0)
                .map(|def| def.ratings.iter().any(|r| r == rating_name))
                .unwrap_or(false);
            if valid {
                sessions.set_pending_rating(&station_id, rating_name.to_string());
                outbound.push((
                    Target::All,
                    ServerMessage::RatingChanged {
                        station_id,
                        rating_name: rating_name.to_string(),
                    },
                ));
            }
        }
    }

    LobbyHandlerResult {
        new_phase: None,
        outbound,
        station_rating_update: None,
        countdown_action: None,
    }
}

/// Mark a peer as disconnected. Applies `Backfill` AI rating to the leaver's
/// station so the ship continues operating automatically. Broadcasts
/// `PlayerLeft` + `RatingChanged { station_id, "Backfill" }`.
///
/// The `resolver` is mutated in place so the Bevy runtime's
/// `ShipSystemControlSources` stays consistent; `station_ratings` is used to
/// capture the pre-disconnect rating into `Player.last_rating` for the
/// reconnect-yield logic.
pub fn process_disconnect_with_stations(
    token: &str,
    sessions: &mut SessionManager,
    ship_stations: &ShipStations,
    ship_config: &ShipConfig,
    resolver: &mut ControlSourceResolver,
    station_ratings: &HashMap<StationId, String>,
    phase: GamePhase,
    preload_complete: bool,
) -> LobbyHandlerResult {
    // Capture station + rating before disconnect mutates the player.
    let station_id = sessions.station_for_token(token).cloned();
    if let Some(ref sid) = station_id {
        let current_rating = station_ratings.get(sid).cloned();
        sessions.set_last_rating(token, current_rating);
        // Station stays on the Player record so reconnect can restore it
        // if no one else claimed it. SelectStation checks p.connected to
        // determine occupancy, so the disconnected player's station is
        // still available for claim mid-game.
    }

    sessions.disconnect(token);

    // Use countdown action instead of direct phase transition.
    let new_phase = None;
    let countdown_action = if phase == GamePhase::Lobby && sessions.all_ready() {
        let pending_phase = if preload_complete || ship_stations.stations.is_empty() {
            GamePhase::InProgress
        } else {
            GamePhase::Loading
        };
        Some(CountdownAction::Start {
            secs: 5,
            pending_phase,
        })
    } else if phase == GamePhase::Lobby {
        // Not all ready anymore → cancel any active countdown.
        Some(CountdownAction::Cancel)
    } else {
        None
    };

    let mut outbound = vec![(
        Target::All,
        ServerMessage::PlayerLeft {
            token: token.to_string(),
        },
    )];

    let mut station_rating_update = None;

    if let Some(ref sid) = station_id {
        rating::apply_rating(ship_config, sid, rating::BACKFILL_RATING, resolver);
        outbound.push((
            Target::All,
            ServerMessage::RatingChanged {
                station_id: sid.clone(),
                rating_name: rating::BACKFILL_RATING.to_string(),
            },
        ));
        station_rating_update = Some((sid.clone(), rating::BACKFILL_RATING.to_string()));
    }

    LobbyHandlerResult {
        new_phase,
        outbound,
        station_rating_update,
        countdown_action,
    }
}

/// Mark a peer as disconnected (no station info available; Bevy-only path).
pub fn process_disconnect(
    token: &str,
    sessions: &mut SessionManager,
    phase: GamePhase,
    preload_complete: bool,
) -> LobbyHandlerResult {
    sessions.disconnect(token);

    // Use countdown action instead of direct phase transition.
    let new_phase = None;
    let countdown_action = if phase == GamePhase::Lobby && sessions.all_ready() {
        Some(CountdownAction::Start {
            secs: 5,
            pending_phase: if preload_complete {
                GamePhase::InProgress
            } else {
                GamePhase::Loading
            },
        })
    } else if phase == GamePhase::Lobby {
        Some(CountdownAction::Cancel)
    } else {
        None
    };

    let outbound = vec![(
        Target::All,
        ServerMessage::PlayerLeft {
            token: token.to_string(),
        },
    )];

    LobbyHandlerResult {
        new_phase,
        outbound,
        station_rating_update: None,
        countdown_action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{EntitySnapshot, StationId, WorldData};
    use crate::ship::control_source::ControlSourceResolver;
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
        crate::ship::config::parse_and_validate(TOML, KINDS)
            .expect("backfill_ship_config must parse")
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
        let result =
            pd_stations_with_phase("t2", &mut sessions, &stations, GamePhase::Lobby, false);
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
                },
                StationDef {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "Pilot the ship.".into(),
                    rank: "Ltn.".into(),
                    short_code: "HLM".into(),
                    console: None,
                    ratings: vec![],
                },
                StationDef {
                    id: StationId("tactical".into()),
                    name: "Tactical".into(),
                    description: "Manage weapons.".into(),
                    rank: "Ltn.".into(),
                    short_code: "TAC".into(),
                    console: None,
                    ratings: vec![],
                },
                StationDef {
                    id: StationId("repair".into()),
                    name: "Repair".into(),
                    description: "Repair systems.".into(),
                    rank: "Ltn.".into(),
                    short_code: "ENG".into(),
                    console: None,
                    ratings: vec![],
                },
                StationDef {
                    id: StationId("sensors".into()),
                    name: "Sensors".into(),
                    description: "Monitor sensors.".into(),
                    rank: "Ens.".into(),
                    short_code: "SCI".into(),
                    console: None,
                    ratings: vec![],
                },
                StationDef {
                    id: StationId("shields".into()),
                    name: "Shields".into(),
                    description: "Manage shields.".into(),
                    rank: "Ens.".into(),
                    short_code: "SHD".into(),
                    console: None,
                    ratings: vec![],
                },
                StationDef {
                    id: StationId("navigation".into()),
                    name: "Navigation".into(),
                    description: "Plot course.".into(),
                    rank: "Ens.".into(),
                    short_code: "NAV".into(),
                    console: None,
                    ratings: vec![],
                },
                StationDef {
                    id: StationId("power".into()),
                    name: "Power".into(),
                    description: "Manage power.".into(),
                    rank: "Ltn.".into(),
                    short_code: "PWR".into(),
                    console: None,
                    ratings: vec![],
                },
                StationDef {
                    id: StationId("comms".into()),
                    name: "Comms".into(),
                    description: "Hail contacts.".into(),
                    rank: "Ens.".into(),
                    short_code: "COM".into(),
                    console: None,
                    ratings: vec![],
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
                station_id,
                ..
            } if token == "t1" => Some((station.clone(), station_id.clone())),
            _ => None,
        });
        let (station_name, station_id) = assigned.expect("StationAssigned not found");
        assert_eq!(station_name, Some("Captain".to_string()));
        assert_eq!(
            station_id,
            Some(crate::messages::StationId("captain".into()))
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
        let result = process_message(
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
        let result = process_message(
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
        let result = process_message(
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
        for token in ["t1", "t2"] {
            assert!(
                result.outbound.iter().any(|(target, m)| {
                    matches!(target, Target::All)
                        && matches!(m, ServerMessage::ReadyChanged { token: t, ready: false } if t == token)
                }),
                "expected ReadyChanged(false) broadcast for {token}"
            );
        }
        assert!(
            result.outbound.iter().any(|(target, m)| {
                matches!(target, Target::All) && matches!(m, ServerMessage::ReturnedToLobby)
            }),
            "expected ReturnedToLobby broadcast"
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

    #[test]
    fn confirm_scenario_during_lobby_broadcasts_scenario_loaded() {
        let mut sessions = sessions_with("t1", "Alice");
        let result = pm(
            "t1",
            &ClientMessage::ConfirmScenario,
            &mut sessions,
            GamePhase::Lobby,
            None,
        );
        assert!(result.new_phase.is_none());
        assert!(result.outbound.iter().any(|(target, m)| {
            matches!(target, Target::All) && matches!(m, ServerMessage::ScenarioLoaded)
        }));
    }

    #[test]
    fn confirm_scenario_outside_lobby_is_noop() {
        let mut sessions = sessions_with("t1", "Alice");
        for phase in [
            GamePhase::Loading,
            GamePhase::InProgress,
            GamePhase::GameOver,
        ] {
            let phase_copy = phase.clone();
            let result = pm(
                "t1",
                &ClientMessage::ConfirmScenario,
                &mut sessions,
                phase,
                None,
            );
            assert!(
                result.outbound.is_empty(),
                "no outbound for phase {phase_copy:?}"
            );
        }
    }

    #[test]
    fn control_system_in_lobby_produces_no_output() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::ControlSystem {
            target: crate::system_registry::helm_thrust_system_id(),
            payload: crate::messages::SystemControlPayload::SetThrust { value: 0.5 },
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
        let reconnect_result = process_message(
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
        let reconnect_result = process_message(
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
}
