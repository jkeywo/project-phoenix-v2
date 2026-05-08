use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ViewDirection {
    #[default]
    Fore,
    Aft,
    Port,
    Starboard,
}

/// What is currently shown on the viewscreen.
///
/// `Camera(direction)` is the default exterior view; `Radar` is the
/// top-down tactical view requested by the helm.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data")]
pub enum ViewMode {
    Camera(ViewDirection),
    Radar,
}

impl Default for ViewMode {
    fn default() -> Self {
        ViewMode::Camera(ViewDirection::Fore)
    }
}

#[cfg(test)]
mod view_mode_tests {
    use super::*;

    #[test]
    fn default_view_mode_is_camera_fore() {
        assert_eq!(ViewMode::default(), ViewMode::Camera(ViewDirection::Fore));
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Console {
    CaptainChair,
    Helm,
    Weapons,
    Engineering,
}

impl Console {
    /// Human-readable name suitable for display in UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            Console::CaptainChair => "Captain's Chair",
            Console::Helm => "Helm",
            Console::Weapons => "Weapons",
            Console::Engineering => "Engineering",
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
    /// Static world data — `Some` only after `StartGame` has populated the
    /// world; `None` while in Lobby or before world initialisation.
    #[serde(default)]
    pub world: Option<WorldData>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SimSnapshot {
    pub red_alert: bool,
    pub view_mode: ViewMode,
    pub ship_x: f32,
    pub ship_z: f32,
    pub ship_yaw: f32,
    /// The console currently authorized to perform a repair action.
    /// `None` means there are no pending breakdowns.
    #[serde(default)]
    pub authorized_repair_console: Option<Console>,
}

/// One asteroid in a `WorldData` snapshot — position on the play plane
/// and collider radius.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AsteroidInfo {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
}

/// Static world data sent once per game (after `StartGame`) and replayed
/// on `Welcome` to clients reconnecting mid-game.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldData {
    pub asteroids: Vec<AsteroidInfo>,
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
    SetView { mode: ViewMode },
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
    WorldSetup { world: WorldData },
}
