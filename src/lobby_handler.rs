use crate::messages::{
    ClientMessage, Console, GamePhase, GameState, ServerMessage, WorldData,
};
use crate::session::SessionManager;
use crate::stations::ShipStations;

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
            // Stub: look up console by display_name to maintain backward compat
            // with existing session logic until slice 5 replaces this.
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
                new_phase = Some(GamePhase::InProgress);
                outbound.push((Target::All, ServerMessage::GameStarted));
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

    // ── process_message: SelectStation / ReleaseStation ───────────────────

    #[test]
    fn select_station_broadcasts_station_assigned() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::SelectStation { station: "Captain".into() };
        let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.iter().any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. })));
    }

    #[test]
    fn release_station_broadcasts_station_assigned() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::ReleaseStation;
        let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.iter().any(|(_, m)| matches!(m, ServerMessage::StationAssigned { .. })));
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
    fn helm_input_in_lobby_produces_no_output() {
        let mut sessions = sessions_with("t1", "Alice");
        let msg = ClientMessage::HelmInput { thrust: 0.5, steering: 0.0 };
        let result = pm("t1", &msg, &mut sessions, GamePhase::Lobby, None);
        assert!(result.outbound.is_empty());
        assert!(result.new_phase.is_none());
    }
}
