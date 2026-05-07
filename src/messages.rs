use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ViewDirection {
    #[default]
    Fore,
    Aft,
    Port,
    Starboard,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Console {
    CaptainChair,
    Helm,
}

impl Console {
    /// Human-readable name suitable for display in UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            Console::CaptainChair => "Captain's Chair",
            Console::Helm => "Helm",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum GamePhase {
    Lobby,
    InProgress,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Player {
    pub token: String,
    pub name: String,
    pub consoles: Vec<Console>,
    pub connected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GameState {
    pub phase: GamePhase,
    pub players: Vec<Player>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SimSnapshot {
    pub red_alert: bool,
    pub view_direction: ViewDirection,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum ClientMessage {
    Identify { token: String, name: String },
    SetName { name: String },
    SelectConsole { console: Console },
    ClearConsole,
    StartGame,
    ToggleRedAlert,
    HelmInput { thrust: f32, steering: f32 },
    SetView { direction: ViewDirection },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum ServerMessage {
    Welcome { state: GameState },
    PlayerJoined { player: Player },
    PlayerLeft { token: String },
    ConsoleSelected { token: String, consoles: Vec<Console> },
    ConsoleCleared { token: String },
    NameChanged { token: String, name: String },
    GameStarted,
    SimState { snapshot: SimSnapshot },
}
