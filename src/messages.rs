use serde::{Deserialize, Serialize};

/// Which phaser bank to address. Used in `SetPhaserMode` and `PhaserFired`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PhaserBank {
    Port,
    Starboard,
}

/// Firing mode for phaser banks. Matches `phaser::PhaserMode`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PhaserMode {
    #[default]
    Auto,
    Manual,
}

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
    Tactical,
    Engineering,
}

impl Console {
    /// Human-readable name suitable for display in UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            Console::CaptainChair => "Captain's Chair",
            Console::Helm => "Helm",
            Console::Tactical => "Tactical",
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
    pub hull_integrity: i32,
    /// The console currently authorized to perform a repair action.
    /// `None` means there are no pending breakdowns.
    #[serde(default)]
    pub authorized_repair_console: Option<Console>,
}

/// One asteroid in a `WorldData` snapshot — position on the play plane,
/// collider radius, and stable UUID for client-side targeting.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AsteroidInfo {
    pub uuid: String,
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
    SetTarget { uuid: String },
    FirePhaser,
    SetPhaserMode { mode: PhaserMode },
    Repair { console: Console },
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
    TargetLock { uuid: String, locked: bool },
    /// Sent at 10 Hz to the Weapons console player only.  `target_uuid` is the
    /// currently locked target (`None` if no lock), `fire_ready` indicates
    /// whether that target is within phaser range and in the forward 180° arc,
    /// and `on_cooldown` indicates whether the phaser is in its post-beam
    /// cooldown period (Fire is blocked).
    WeaponsUpdate { target_uuid: Option<String>, fire_ready: bool, on_cooldown: bool },
    /// Broadcast when a phaser beam starts. Sent to all players so the renderer
    /// can draw the beam on the viewscreen.
    BeamStarted { target_uuid: String },
    /// Broadcast when a phaser beam ends (natural expiry, sever, or cancel).
    BeamEnded { target_uuid: String },
    /// Broadcast when an asteroid's HP reaches 0 and it is despawned.
    AsteroidDestroyed { uuid: String },
    /// Sent when a phaser bank fires a shot at a target.
    PhaserFired { bank: PhaserBank, target_uuid: String },
    /// Sent at 10 Hz to each console player.  Carries the remaining cooldown
    /// (penalty or repair) in seconds, whether a repair action is currently
    /// in progress, and whether the last cooldown was a penalty.
    RepairState { remaining_cooldown_secs: f32, in_progress: bool, penalty: bool },
}
