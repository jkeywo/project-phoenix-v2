use crate::messages::{ClientMessage, ServerMessage};

pub trait MessageCodec {
    type Error;
    fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error>;
    fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error>;
    fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error>;
    fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error>;
}

pub struct JsonCodec;

impl MessageCodec for JsonCodec {
    type Error = serde_json::Error;

    fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error> {
        serde_json::to_string(msg)
    }

    fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error> {
        serde_json::from_str(s)
    }

    fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error> {
        serde_json::to_string(msg)
    }

    fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error> {
        serde_json::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;

    fn codec() -> JsonCodec {
        JsonCodec
    }

    fn player() -> Player {
        Player { token: "tok".into(), name: "Alice".into(), consoles: vec![], connected: true }
    }

    fn state() -> GameState {
        GameState { phase: GamePhase::Lobby, players: vec![player()], world: None }
    }

    // ClientMessage round-trips

    #[test]
    fn client_identify() {
        let msg = ClientMessage::Identify { token: "t".into(), name: "Bob".into() };
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_set_name() {
        let msg = ClientMessage::SetName { name: "Carol".into() };
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_select_console() {
        let msg = ClientMessage::SelectConsole { console: Console::CaptainChair };
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_clear_console() {
        let msg = ClientMessage::ClearConsole;
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_start_game() {
        let msg = ClientMessage::StartGame;
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_toggle_red_alert() {
        let msg = ClientMessage::ToggleRedAlert;
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_set_view_fore() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Fore) };
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_set_view_aft() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) };
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_set_view_port() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Port) };
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_set_view_starboard() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Starboard) };
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_set_view_radar() {
        let msg = ClientMessage::SetView { mode: ViewMode::Radar };
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_helm_input() {
        let msg = ClientMessage::HelmInput { thrust: 0.75, steering: -0.5 };
        let json = codec().encode_client(&msg).unwrap();
        let rt = codec().decode_client(&json).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn client_select_console_helm() {
        let msg = ClientMessage::SelectConsole { console: Console::Helm };
        let rt = codec().decode_client(&codec().encode_client(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    // ServerMessage round-trips

    #[test]
    fn server_welcome() {
        let msg = ServerMessage::Welcome { state: state() };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn server_player_joined() {
        let msg = ServerMessage::PlayerJoined { player: player() };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn server_player_left() {
        let msg = ServerMessage::PlayerLeft { token: "tok".into() };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn server_console_selected() {
        let msg = ServerMessage::ConsoleSelected { token: "tok".into(), consoles: vec![Console::CaptainChair] };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn server_console_selected_helm() {
        let msg = ServerMessage::ConsoleSelected { token: "tok".into(), consoles: vec![Console::Helm] };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn server_console_cleared() {
        let msg = ServerMessage::ConsoleCleared { token: "tok".into() };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn server_name_changed() {
        let msg = ServerMessage::NameChanged { token: "tok".into(), name: "Dave".into() };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn server_game_started() {
        let msg = ServerMessage::GameStarted;
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn server_sim_state() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: true,
                view_mode: ViewMode::Camera(ViewDirection::Fore),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
            },
        };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn sim_snapshot_view_mode_starboard() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::Camera(ViewDirection::Starboard),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
            },
        };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn sim_snapshot_view_mode_radar() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::Radar,
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
            },
        };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn sim_snapshot_carries_ship_position_and_yaw() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 12.5,
                ship_z: -8.25,
                ship_yaw: 1.5707,
            },
        };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn world_data_with_asteroids_round_trips_inside_world_setup() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                asteroids: vec![
                    AsteroidInfo { x: 1.0, z: 2.0, radius: 2.0 },
                    AsteroidInfo { x: -3.5, z: 4.25, radius: 1.5 },
                ],
            },
        };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn welcome_with_world_some_round_trips() {
        let msg = ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![player()],
                world: Some(WorldData {
                    asteroids: vec![AsteroidInfo { x: 0.0, z: 0.0, radius: 2.0 }],
                }),
            },
        };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }

    #[test]
    fn welcome_with_world_none_round_trips() {
        // Lobby phase has no world yet
        let msg = ServerMessage::Welcome { state: state() };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
        match rt {
            ServerMessage::Welcome { state } => assert!(state.world.is_none()),
            _ => panic!("expected Welcome"),
        }
    }
}
