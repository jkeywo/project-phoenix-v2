use serde::{Deserialize, Serialize};
use crate::stations::ShipStations;

/// Which ship attribute a modifier affects. Defined here so it can be used in
/// wire messages without creating a circular dependency with `modifiers.rs`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModifierSlot {
    MaxSpeed,
    MaxYawRate,
    RadarRange,
    PhaserDamage,
    HullDamageTaken,
    RepairRate,
}

/// Who or what applied a modifier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModifierSource {
    Console(Console),
    ImpulseDrive,
    RegionEffect { region_id: String },
}

impl Eq for ModifierSource {}

impl std::hash::Hash for ModifierSource {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            ModifierSource::Console(c) => {
                0u8.hash(state);
                std::mem::discriminant(c).hash(state);
            }
            ModifierSource::ImpulseDrive => {
                1u8.hash(state);
            }
            ModifierSource::RegionEffect { region_id } => {
                2u8.hash(state);
                region_id.hash(state);
            }
        }
    }
}

/// A serialisable snapshot of a single shield facing for broadcasting.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ShieldFacingStatus {
    pub label: String,
    pub hp: i32,
    pub max_hp: i32,
    pub online: bool,
    /// Remaining offline seconds (0.0 when online).
    pub offline_remaining: f32,
}

/// Which phaser bank to address. Used in `SetPhaserMode` and `PhaserFired`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PhaserBank {
    Port,
    Starboard,
}

/// Which torpedo tube to fire from.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TorpedoTube {
    ForePort,
    ForeStarboard,
    Aft,
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
    ScienceRadar,
    SystemChart,
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
    Science,
}

impl Console {
    /// Human-readable name suitable for display in UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            Console::CaptainChair => "Captain's Chair",
            Console::Helm => "Helm",
            Console::Tactical => "Tactical",
            Console::Engineering => "Engineering",
            Console::Science => "Science",
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

/// An asteroid field defined as a donut-shaped ring in world space.
///
/// The field has a centre (`x`, `z`), an `inner_radius` (the clear inner
/// boundary) and an `outer_radius` (the dense outer boundary).  On radar
/// these appear as concentric rings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AsteroidField {
    pub uuid: String,
    pub x: f32,
    pub z: f32,
    pub inner_radius: f32,
    pub outer_radius: f32,
    /// Semantic tags for this entity (e.g. `["asteroid_field"]`).
    #[serde(default)]
    pub tags: Vec<String>,
}

/// One asteroid in a `WorldData` snapshot — position on the play plane,
/// collider radius, and stable UUID for client-side targeting.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AsteroidInfo {
    pub uuid: String,
    pub x: f32,
    pub z: f32,
    pub radius: f32,
    /// Semantic tags for this entity (e.g. `["asteroid"]`).
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Static world data sent once per game (after `StartGame`) and replayed
/// on `Welcome` to clients reconnecting mid-game.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldData {
    pub asteroids: Vec<AsteroidInfo>,
    /// Asteroid field rings, for science radar and system chart rendering.
    #[serde(default)]
    pub asteroid_fields: Vec<AsteroidField>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum ClientMessage {
    Identify { token: String, name: String },
    SetName { name: String },
    SelectStation { station: String },
    ReleaseStation,
    StartGame,
    ToggleRedAlert,
    HelmInput { thrust: f32, steering: f32 },
    /// Sent by the Helm officer to begin charging the impulse drive.
    StartImpulseCharge,
    /// Sent by the Science officer to cancel an active or charging impulse drive.
    CancelImpulse,
    SetView { mode: ViewMode },
    SetTarget { uuid: String },
    /// Science officer taps an entity on their radar to suggest it as a target
    /// to the Weapons console. Advisory only — does not affect lock state.
    SetScienceTarget { uuid: String },
    FirePhaser,
    SetPhaserMode { mode: PhaserMode },
    Repair { console: Console },
    /// Fire a torpedo from the specified tube. `target_uuid` is optional homing target.
    FireTorpedo { tube: TorpedoTube, target_uuid: Option<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum ServerMessage {
    Welcome { state: GameState, ship_stations: ShipStations },
    PlayerJoined { player: Player },
    PlayerLeft { token: String },
    StationAssigned { token: String, station: Option<String>, consoles: Vec<Console> },
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
    WeaponsUpdate {
        target_uuid: Option<String>,
        fire_ready: bool,
        on_cooldown: bool,
        /// Remaining torpedoes in the magazine.
        torpedo_count: u32,
        /// Whether the fore-port tube is loaded and ready.
        fore_port_loaded: bool,
        /// Reload remaining for the fore-port tube (0.0 when loaded).
        fore_port_reload_secs: f32,
        /// Whether the fore-starboard tube is loaded and ready.
        fore_starboard_loaded: bool,
        /// Reload remaining for the fore-starboard tube (0.0 when loaded).
        fore_starboard_reload_secs: f32,
        /// Whether the aft tube is loaded and ready.
        aft_loaded: bool,
        /// Reload remaining for the aft tube (0.0 when loaded).
        aft_reload_secs: f32,
    },
    /// Broadcast when a phaser beam starts. Sent to all players so the renderer
    /// can draw the beam on the viewscreen.
    BeamStarted { target_uuid: String },
    /// Broadcast when a phaser beam ends (natural expiry, sever, or cancel).
    BeamEnded { target_uuid: String },
    /// Broadcast when an asteroid's HP reaches 0 and it is despawned.
    AsteroidDestroyed { uuid: String },
    /// Broadcast to all players when the Science officer taps a radar entity
    /// to designate it as a suggested target. Advisory only — does not lock
    /// the Tactical console.
    ScienceTargetSuggestion { uuid: String },
    /// Sent when a phaser bank fires a shot at a target.
    PhaserFired { bank: PhaserBank, target_uuid: String },
    /// Sent at 10 Hz to each console player.  Carries the remaining cooldown
    /// (penalty or repair) in seconds, whether a repair action is currently
    /// in progress, and whether the last cooldown was a penalty.
    RepairState { remaining_cooldown_secs: f32, in_progress: bool, penalty: bool },
    /// Sent at 10 Hz (or on change) to all players. Contains HP and online
    /// status for every shield facing.
    ShieldStatus { facings: Vec<ShieldFacingStatus> },
    /// Broadcast to all when a torpedo is launched from a tube.
    TorpedoLaunched { uuid: String, tube: TorpedoTube, x: f32, z: f32, heading: f32 },
    /// Broadcast to all when a torpedo is destroyed (expired or hit something).
    TorpedoDestroyed { uuid: String },
    /// Broadcast when a modifier is added or updated on the ship.
    ModifierAdded { source: ModifierSource, slot: ModifierSlot, bonus: f32 },
    /// Broadcast when a modifier is removed from the ship.
    ModifierRemoved { source: ModifierSource, slot: ModifierSlot },
    /// Broadcast when an asteroid is spawned by the window lifecycle system.
    /// Sent to all players so the client can track the new entity.
    AsteroidSpawned {
        uuid: String,
        x: f32,
        y: f32,
        z: f32,
        config_path: String,
        max_hp: i32,
        current_hp: i32,
    },
}
