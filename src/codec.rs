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

    struct PrettyJsonCodec;

    impl MessageCodec for PrettyJsonCodec {
        type Error = serde_json::Error;
        fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error> { serde_json::to_string_pretty(msg) }
        fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error> { serde_json::from_str(s) }
        fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error> { serde_json::to_string_pretty(msg) }
        fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error> { serde_json::from_str(s) }
    }

    fn assert_client_roundtrip<C: MessageCodec>(codec: &C, msg: ClientMessage)
    where C::Error: std::fmt::Debug {
        let encoded = codec.encode_client(&msg).unwrap();
        let decoded = codec.decode_client(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    fn assert_server_roundtrip<C: MessageCodec>(codec: &C, msg: ServerMessage)
    where C::Error: std::fmt::Debug {
        let encoded = codec.encode_server(&msg).unwrap();
        let decoded = codec.decode_server(&encoded).unwrap();
        assert_eq!(msg, decoded);
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
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_name() {
        let msg = ClientMessage::SetName { name: "Carol".into() };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_select_console() {
        let msg = ClientMessage::SelectConsole { console: Console::CaptainChair };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_clear_console() {
        let msg = ClientMessage::ClearConsole;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_start_game() {
        let msg = ClientMessage::StartGame;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_toggle_red_alert() {
        let msg = ClientMessage::ToggleRedAlert;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_fore() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Fore) };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_aft() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_port() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Port) };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_starboard() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Starboard) };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_radar() {
        let msg = ClientMessage::SetView { mode: ViewMode::Radar };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_helm_input() {
        let msg = ClientMessage::HelmInput { thrust: 0.75, steering: -0.5 };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_select_console_helm() {
        let msg = ClientMessage::SelectConsole { console: Console::Helm };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_select_console_weapons() {
        let msg = ClientMessage::SelectConsole { console: Console::Weapons };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_select_console_engineering() {
        let msg = ClientMessage::SelectConsole { console: Console::Engineering };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_console_selected_weapons() {
        let msg = ServerMessage::ConsoleSelected { token: "tok".into(), consoles: vec![Console::Weapons] };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_console_selected_engineering() {
        let msg = ServerMessage::ConsoleSelected { token: "tok".into(), consoles: vec![Console::Engineering] };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    // ServerMessage round-trips

    #[test]
    fn server_welcome() {
        let msg = ServerMessage::Welcome { state: state() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_player_joined() {
        let msg = ServerMessage::PlayerJoined { player: player() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_player_left() {
        let msg = ServerMessage::PlayerLeft { token: "tok".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_console_selected() {
        let msg = ServerMessage::ConsoleSelected { token: "tok".into(), consoles: vec![Console::CaptainChair] };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_console_selected_helm() {
        let msg = ServerMessage::ConsoleSelected { token: "tok".into(), consoles: vec![Console::Helm] };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_console_cleared() {
        let msg = ServerMessage::ConsoleCleared { token: "tok".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_name_changed() {
        let msg = ServerMessage::NameChanged { token: "tok".into(), name: "Dave".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_game_started() {
        let msg = ServerMessage::GameStarted;
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
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
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
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
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
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
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
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
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn asteroid_info_with_uuid_round_trips_in_world_setup() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                asteroids: vec![
                    AsteroidInfo {
                        uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                        x: 12.5,
                        z: -8.0,
                        radius: 2.0,
                    },
                ],
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn world_data_with_asteroids_round_trips_inside_world_setup() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                asteroids: vec![
                    AsteroidInfo { uuid: "a1b2c3d4-e5f6-4789-8abc-def012345678".into(), x: 1.0, z: 2.0, radius: 2.0 },
                    AsteroidInfo { uuid: "b2c3d4e5-f6a7-4890-9bcd-ef0123456789".into(), x: -3.5, z: 4.25, radius: 1.5 },
                ],
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn welcome_with_world_some_round_trips() {
        let msg = ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![player()],
                world: Some(WorldData {
                    asteroids: vec![AsteroidInfo { uuid: "c3d4e5f6-a7b8-4901-acde-f01234567890".into(), x: 0.0, z: 0.0, radius: 2.0 }],
                }),
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn welcome_with_world_none_round_trips() {
        let msg = ServerMessage::Welcome { state: state() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg.clone());
        match msg {
            ServerMessage::Welcome { state } => assert!(state.world.is_none()),
            _ => panic!("expected Welcome"),
        }
    }
}
