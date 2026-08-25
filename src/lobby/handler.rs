use std::collections::HashMap;

use crate::core::messages::{
    GamePhase, GameState, ServerMessage, ShipClientConfig, StationId, WorldData,
};
use crate::lobby::session::SessionManager;
use crate::lobby::stations_config::{get_station, ShipStations};
use crate::ship::config::ShipConfig;
use crate::ship::control_source::ControlSourceResolver;
use crate::ship::rating;

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
    /// branch of `handle_identify` (restore).
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

/// True when `token` is reserved for the host runtime and so must never be
/// claimable by a network peer.
///
/// Two shapes carry authority no phone is entitled to:
///
///   - [`crate::console_bridge::LOCAL_CONSOLE_TOKEN`], which command admission
///     grants a blanket bypass (`command_admission::policy`, and see
///     `ship::system_registry` on god mode relying on it) and which
///     [`return_to_lobby_authority`] reads as `Host`, and
///   - the `ai:` prefix, which `admit_system_commands` routes to an NPC's own
///     entity rather than the `LocalShip`.
///
/// The peer `Identify` path in `server.html` refuses these before a token is
/// ever recorded for a connection — that is the gate that matters, since every
/// later message is dispatched under the recorded token. This is the same
/// refusal server-side, so a peer that reaches the handler by some other route
/// still cannot register a session under a reserved name.
pub(crate) fn is_reserved_token(token: &str) -> bool {
    token == crate::console_bridge::LOCAL_CONSOLE_TOKEN || token.starts_with("ai:")
}

/// Handle `Identify`: (re)register the session, restore a held station on
/// reconnect-yield, and emit `Welcome` / `PlayerJoined` (and, at capacity, an
/// empty `StationAssigned`). `token` from the envelope is ignored — the session
/// token comes from the message body. A reserved token
/// ([`is_reserved_token`]) registers nothing and answers nothing.
pub(crate) fn handle_identify(
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

    // Reserved host-runtime tokens are refused outright: registering one would
    // put a peer's session behind a name that command admission and
    // `return_to_lobby_authority` both read as host authority. Silent — there
    // is no legitimate sender to explain this to.
    if is_reserved_token(&id_token) {
        return LobbyHandlerResult {
            new_phase: None,
            outbound: Vec::new(),
            station_rating_update: None,
            countdown_action: None,
        };
    }
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
                let capacity = ship_stations
                    .stations
                    .iter()
                    .filter(|station| !station.auxiliary)
                    .count() as u32;
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
pub(crate) fn handle_set_name(
    token: &str,
    name: &str,
    sessions: &mut SessionManager,
) -> LobbyHandlerResult {
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
    if station_def.auxiliary {
        return LobbyHandlerResult {
            new_phase: None,
            outbound,
            station_rating_update: None,
            countdown_action: None,
        };
    }

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

    // Defense-in-depth accessibility guard (issue #1103 AC1). The client already
    // blocks and privately explains an ineligible claim; this silently no-ops a
    // claim the sender has reported itself ineligible for, taking the SAME
    // neutral path as an occupied/invalid seat. No reason is ever computed or put
    // on the wire — the host holds only the anonymous boolean. DEFAULT TRUE keeps
    // a silent/legacy client (never reported) claimable as today.
    if !sessions.is_eligible(token, &station_def.id) {
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

    // A Spectator (issue #1105) has opted out of both a Station and readiness.
    // A stray `SetReady` from one must neither flip a flag it does not own nor
    // trip the start countdown — no-op it before anything mutates. (Readiness
    // already excludes spectators in `all_ready`; this stops a spectator's own
    // flag from ever being set true in the first place.)
    if sessions.is_spectator(token) {
        return LobbyHandlerResult {
            new_phase: None,
            outbound,
            station_rating_update: None,
            countdown_action: None,
        };
    }

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

/// Handle `SetSpectator` (issue #1105): join or leave the explicit Spectator
/// role. Mirrors `handle_release_station`'s seat-vacate + unready, but keyed on
/// an explicit role rather than a seat.
///
/// On `spectator == true`: set the flag (which vacates any held Station via
/// `SessionManager::set_spectator`), broadcast the seat clear if one was held,
/// clear the ready flag and broadcast `ReadyChanged { false }`, then broadcast
/// `SpectatorChanged { true }`.
///
/// On `spectator == false`: clear the flag and broadcast `SpectatorChanged
/// { false }`, leaving the participant a seatless, un-ready lobby member again.
///
/// When a seat is vacated on the way in, the vacated Station's rating is reset
/// exactly as `handle_release_station` does it (Backfill mid-game; pending
/// cleared + base rating pre-game): giving up a seat via Spectate must leave
/// that station in the same state as giving it up via ReleaseStation, rather
/// than stranding a stale `pending_ratings` entry for the next claimant (or a
/// dark, non-Backfill seat in-progress).
pub(crate) fn handle_set_spectator(
    token: &str,
    spectator: bool,
    sessions: &mut SessionManager,
    phase: GamePhase,
    ship_stations: &ShipStations,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut station_rating_update: Option<(StationId, String)> = None;

    if spectator {
        // Capture the seat BEFORE `set_spectator` vacates it, so we only emit a
        // `StationAssigned { None }` (and reset the seat's rating) when a seat
        // was actually released.
        let vacated_station = sessions.station_for_token(token).cloned();
        sessions.set_spectator(token, true); // sets flag + clears station (invariant)
        if vacated_station.is_some() {
            outbound.push((
                Target::All,
                ServerMessage::StationAssigned {
                    token: token.to_string(),
                    station: None,
                    station_id: None,
                },
            ));
        }
        sessions.set_ready(token, false);
        outbound.push((
            Target::All,
            ServerMessage::ReadyChanged {
                token: token.to_string(),
                ready: false,
            },
        ));
        // Reset the vacated seat's rating, mirroring `handle_release_station`.
        if let Some(station_id) = vacated_station {
            if phase == GamePhase::InProgress {
                let backfill = rating::BACKFILL_RATING.to_string();
                outbound.push((
                    Target::All,
                    ServerMessage::RatingChanged {
                        station_id: station_id.clone(),
                        rating_name: backfill.clone(),
                    },
                ));
                station_rating_update = Some((station_id, backfill));
            } else {
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
        }
    } else {
        sessions.set_spectator(token, false);
    }

    outbound.push((
        Target::All,
        ServerMessage::SpectatorChanged {
            token: token.to_string(),
            spectator,
        },
    ));

    LobbyHandlerResult {
        new_phase: None,
        outbound,
        station_rating_update,
        countdown_action: None,
    }
}

/// Handle `SetAfk` (issue #1104): enter or leave the AFK presence state. AFK
/// delegates every System on the player's DIRECTLY-held Station through ordinary
/// AI control while retaining the seat and reconnect identity, and (via the
/// per-tick resolver's AFK gate) makes the player an ineligible host for visiting
/// Stations. Leaving AFK restores the prior coherent control configuration.
///
/// On `afk == true`: snapshot the seat's current rating into the
/// `afk_prev_rating` side-map (AC4's restore target), then move that Station to
/// `BACKFILL_RATING` — the EXACT move `process_disconnect_with_stations` makes,
/// reusing `rating::apply_rating` (through `apply_result`) unchanged, so AFK adds
/// no new control mechanism (AC2). The seat is RETAINED, so no `StationAssigned`
/// is emitted and the client keeps its console focus (AC4). Broadcasts
/// `RatingChanged { station, Backfill }` for the delegated seat, then
/// `AfkChanged { true }`.
///
/// On `afk == false`: re-apply the snapshotted prior rating and consume the
/// snapshot (AC4). Visiting Stations need nothing stored — the pure per-tick
/// resolver re-includes the no-longer-AFK holder on the next tick (AC4).
/// Broadcasts `RatingChanged { station, prev }`, then `AfkChanged { false }`.
///
/// `AfkChanged` carries ONLY the boolean — no accessibility detail (AC5).
///
/// Takes `station_ratings` (the live `ActiveStationRatings` map) purely to READ
/// the pre-AFK rating for the snapshot; the delegation/restore themselves ride
/// on `station_rating_update`, applied by `apply_result` like every other lobby
/// rating change.
pub(crate) fn handle_set_afk(
    token: &str,
    afk: bool,
    sessions: &mut SessionManager,
    station_ratings: &HashMap<StationId, String>,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut station_rating_update: Option<(StationId, String)> = None;

    // AFK never vacates the seat, so the held Station is the same before and
    // after the flag flips — capture it once.
    let held_station = sessions.station_for_token(token).cloned();

    if afk {
        // Guard against a redundant re-entry (already AFK): re-snapshotting would
        // capture the Backfill we ourselves applied and clobber the true prior
        // rating. Only the first entry snapshots + delegates.
        if !sessions.is_afk(token) {
            if let Some(ref sid) = held_station {
                // Snapshot the CURRENT rating before Backfill overwrites it. A
                // seat with no recorded rating restores to Backfill, a harmless
                // already-automated target.
                let prev = station_ratings
                    .get(sid)
                    .cloned()
                    .unwrap_or_else(|| rating::BACKFILL_RATING.to_string());
                sessions.set_afk_prev_rating(token, prev);

                let backfill = rating::BACKFILL_RATING.to_string();
                outbound.push((
                    Target::All,
                    ServerMessage::RatingChanged {
                        station_id: sid.clone(),
                        rating_name: backfill.clone(),
                    },
                ));
                station_rating_update = Some((sid.clone(), backfill));
            }
        }
        sessions.set_afk(token, true);
    } else {
        sessions.set_afk(token, false);
        // Restore the directly-held Station's prior configuration; visiting
        // Stations re-resolve on the next tick (nothing to store). Consume the
        // snapshot so a later disconnect cannot re-apply it.
        if let Some(prev) = sessions.afk_prev_rating_for(token).cloned() {
            sessions.clear_afk_prev_rating(token);
            if let Some(ref sid) = held_station {
                outbound.push((
                    Target::All,
                    ServerMessage::RatingChanged {
                        station_id: sid.clone(),
                        rating_name: prev.clone(),
                    },
                ));
                station_rating_update = Some((sid.clone(), prev));
            }
        }
    }

    outbound.push((
        Target::All,
        ServerMessage::AfkChanged {
            token: token.to_string(),
            afk,
        },
    ));

    LobbyHandlerResult {
        new_phase: None,
        outbound,
        station_rating_update,
        countdown_action: None,
    }
}

/// Who asked to return to the lobby, which decides which phases honour it.
///
/// The `ReturnToLobby` wire variant itself is deliberately un-gated — any
/// connected participant may send it — so the authority distinction is made
/// here, from the sender's token, rather than by adding a second message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReturnToLobbyAuthority {
    /// A connected participant's Game Over overlay (phone or host page alike).
    /// Honoured only during `GameOver` — a phone must not be able to abandon a
    /// mission everyone else is still flying.
    Participant,
    /// The host page itself, under `LOCAL_CONSOLE_TOKEN`. Its settings menu
    /// carries an "exit to lobby" that also aborts a mission still
    /// `InProgress` (issue #939).
    Host,
}

/// Classify a `ReturnToLobby` sender. The host page injects its own actions
/// under `console_bridge::LOCAL_CONSOLE_TOKEN`; every other token is a peer.
pub(crate) fn return_to_lobby_authority(token: &str) -> ReturnToLobbyAuthority {
    if token == crate::console_bridge::LOCAL_CONSOLE_TOKEN {
        ReturnToLobbyAuthority::Host
    } else {
        ReturnToLobbyAuthority::Participant
    }
}

/// True when `authority` may return the session to the lobby from `phase`.
fn may_return_to_lobby(phase: &GamePhase, authority: ReturnToLobbyAuthority) -> bool {
    match authority {
        ReturnToLobbyAuthority::Participant => *phase == GamePhase::GameOver,
        // The host's abort is the only way out of a running mission that does
        // not require losing it, so it covers `InProgress` as well.
        ReturnToLobbyAuthority::Host => {
            *phase == GamePhase::GameOver || *phase == GamePhase::InProgress
        }
    }
}

/// Handle `ReturnToLobby`: return every participant to shared pre-scenario
/// selection for another round (issue #756). Clears each player's station claim
/// and ready flag plus all pending ratings, broadcasts the cleared seats
/// (`StationAssigned { station: None }`) and readies (`ReadyChanged { ready:
/// false }`) per player, emits `ReturnedToLobby`, and transitions to `Lobby`.
/// Player identity — token, name, connection, and `last_rating` — is preserved.
///
/// Honoured from `GameOver` for anyone, and additionally from `InProgress` for
/// the host page's settings menu (issue #939). No-op otherwise.
pub(crate) fn handle_return_to_lobby(
    sessions: &mut SessionManager,
    phase: GamePhase,
    authority: ReturnToLobbyAuthority,
) -> LobbyHandlerResult {
    let mut outbound = Vec::new();
    let mut new_phase: Option<GamePhase> = None;

    if may_return_to_lobby(&phase, authority) {
        // Capture the roster (token + whether it held a seat) before mutating,
        // so we broadcast a station-release only for players who actually held
        // a station this round.
        let roster: Vec<(String, bool)> = sessions
            .players()
            .iter()
            .map(|p| (p.token.clone(), p.station.is_some()))
            .collect();

        sessions.reset_ready();
        sessions.clear_all_stations();
        sessions.clear_all_pending_ratings();
        // Anonymous accessibility eligibility (issue #1103) is per-round lobby
        // state like the pending ratings above: drop every token's report so the
        // next round starts from the DEFAULT-TRUE baseline and each client
        // re-reports against the freshly selected hull.
        sessions.clear_all_eligibility();

        for (token, had_station) in &roster {
            if *had_station {
                // Authoritative seat release (AGENTS.md #5): clients follow the
                // server's cleared roster. Mirrors `handle_release_station`.
                outbound.push((
                    Target::All,
                    ServerMessage::StationAssigned {
                        token: token.clone(),
                        station: None,
                        station_id: None,
                    },
                ));
            }
            outbound.push((
                Target::All,
                ServerMessage::ReadyChanged {
                    token: token.clone(),
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
#[path = "handler_tests.rs"]
mod tests;
