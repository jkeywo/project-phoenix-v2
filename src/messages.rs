use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Console {
    CaptainChair,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum GamePhase {
    Lobby,
    InProgress,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub token: String,
    pub name: String,
    pub console: Option<Console>,
    pub connected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameState {
    pub phase: GamePhase,
    pub players: Vec<Player>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimSnapshot {
    pub red_alert: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ClientMessage {
    Identify { token: String, name: String },
    SetName { name: String },
    SelectConsole { console: Console },
    ClearConsole,
    StartGame,
    ToggleRedAlert,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerMessage {
    Welcome { state: GameState },
    PlayerJoined { player: Player },
    PlayerLeft { token: String },
    ConsoleSelected { token: String, console: Console },
    ConsoleCleared { token: String },
    NameChanged { token: String, name: String },
    GameStarted,
    SimState { snapshot: SimSnapshot },
}
