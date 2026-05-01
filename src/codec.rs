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
        Player { token: "tok".into(), name: "Alice".into(), console: None, connected: true }
    }

    fn state() -> GameState {
        GameState { phase: GamePhase::Lobby, players: vec![player()] }
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
        let msg = ServerMessage::ConsoleSelected { token: "tok".into(), console: Console::CaptainChair };
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
        let msg = ServerMessage::SimState { snapshot: SimSnapshot { red_alert: true } };
        let rt = codec().decode_server(&codec().encode_server(&msg).unwrap()).unwrap();
        assert_eq!(msg, rt);
    }
}
