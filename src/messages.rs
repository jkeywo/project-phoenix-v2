use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub use crate::entity_tags::EntityTag;
use crate::flag_kind::FlagKind;
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
    RegionEffect { uuid: Uuid },
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
            ModifierSource::RegionEffect { uuid } => {
                2u8.hash(state);
                uuid.hash(state);
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

/// A shape used for the repair mini-game. Assigned randomly to each
/// breakdown entry and fixed for its lifetime.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Shape {
    Square,
    Triangle,
    Circle,
}

/// The state of a single repair team, broadcast as part of `RepairState`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum TeamSlot {
    Idle,
    Repairing { progress: f32 },
    Cooldown { progress: f32 },
}

impl Default for TeamSlot {
    fn default() -> Self {
        Self::Idle
    }
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Console {
    CaptainChair,
    Helm,
    Tactical,
    Repair,
    Science,
    Power,
}

impl Console {
    /// Human-readable name suitable for display in UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            Console::CaptainChair => "Captain's Chair",
            Console::Helm => "Helm",
            Console::Tactical => "Tactical",
            Console::Repair => "Repair",
            Console::Science => "Science",
            Console::Power => "Power",
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
    pub hull_integrity: f32,
    /// Current power allocation levels (Helm, Weapons, Science).
    /// Added to SimSnapshot so all clients see the current configuration.
    #[serde(default = "default_power_levels")]
    pub power_levels: (u8, u8, u8),
    /// Boolean flags that are currently active on the ship.
    /// Populated from `ShipModifiers::flags()` each tick.
    #[serde(default)]
    pub flags: Vec<FlagKind>,
    /// Per-tick entity state snapshots (position, yaw, hull, flags).
    #[serde(default)]
    pub entity_states: Vec<EntityStateSnapshot>,
    /// Current radar configuration ranges.
    #[serde(default)]
    pub radar_state: RadarStateSnapshot,
}

fn default_power_levels() -> (u8, u8, u8) {
    (2, 2, 2)
}

/// A single entity in the unified wire format.
///
/// Carries the minimum identifying fields plus optional aspect fields for
/// visualisation.  Every entity has a `uuid` and `tags`; all other fields
/// are `Option` and only present when relevant to the entity type.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EntitySnapshot {
    pub uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yaw: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull_fraction: Option<f32>,
    /// Inner radius for ring-shaped entities (e.g. asteroid fields).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_radius: Option<f32>,
}

impl EntitySnapshot {
    /// World-space X coordinate (play-plane horizontal). Returns 0.0 when `position` is `None`.
    pub fn x(&self) -> f32 {
        self.position.map(|p| p[0]).unwrap_or(0.0)
    }

    /// World-space Z coordinate (play-plane depth). Returns 0.0 when `position` is `None`.
    pub fn z(&self) -> f32 {
        self.position.map(|p| p[2]).unwrap_or(0.0)
    }

    /// Entity radius or 0.0 when missing.
    pub fn radius_or_zero(&self) -> f32 {
        self.radius.unwrap_or(0.0)
    }

    /// Entity inner radius or 0.0 when missing.
    pub fn inner_radius_or_zero(&self) -> f32 {
        self.inner_radius.unwrap_or(0.0)
    }

    /// Convenience constructor for an asteroid entity (the most common case).
    pub fn asteroid(uuid: impl Into<String>, x: f32, z: f32, radius: f32) -> Self {
        Self {
            uuid: uuid.into(),
            id: None,
            position: Some([x, 0.0, z]),
            tags: vec!["asteroid".into()],
            shape: None,
            radius: Some(radius),
            colour: None,
            yaw: None,
            hull_fraction: None,
            inner_radius: None,
        }
    }

    /// Convenience constructor for an asteroid field entity.
    pub fn asteroid_field(uuid: impl Into<String>, x: f32, z: f32, inner_radius: f32, outer_radius: f32) -> Self {
        Self {
            uuid: uuid.into(),
            id: None,
            position: Some([x, 0.0, z]),
            tags: vec!["asteroid_field".into()],
            shape: None,
            radius: Some(outer_radius),
            colour: None,
            yaw: None,
            hull_fraction: None,
            inner_radius: Some(inner_radius),
        }
    }

    /// Convenience constructor for a basic entity with position and tags (no extra aspects).
    pub fn simple(uuid: impl Into<String>, x: f32, z: f32, tags: Vec<String>) -> Self {
        Self {
            uuid: uuid.into(),
            id: None,
            position: Some([x, 0.0, z]),
            tags,
            shape: None,
            radius: None,
            colour: None,
            yaw: None,
            hull_fraction: None,
            inner_radius: None,
        }
    }
}

/// Per-tick state for a single entity.  Lighter than `EntitySnapshot` —
/// only the fields that change every frame.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EntityStateSnapshot {
    pub uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yaw: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull_fraction: Option<f32>,
    #[serde(default)]
    pub flags: Vec<FlagKind>,
}

/// Per-tick radar configuration snapshot.  Mirrors the effective ranges
/// after modifier application so the client can display the correct scale.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RadarStateSnapshot {
    /// Effective range of the helm radar (world units).
    pub helm_range: f32,
    /// Effective range of the tactical/weapons radar.
    pub tactical_range: f32,
    /// Effective range of the science long-range radar.
    pub science_long_range: f32,
    /// Range of the system chart (typically large / fixed).
    pub science_system_map: f32,
}

impl Default for RadarStateSnapshot {
    fn default() -> Self {
        Self {
            helm_range: 50.0,
            tactical_range: 60.0,
            science_long_range: 200.0,
            science_system_map: 500.0,
        }
    }
}

/// Static world data sent once per game (after `StartGame`) and replayed
/// on `Welcome` to clients reconnecting mid-game.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldData {
    /// All static entities in the world (asteroids, fields, stations, …).
    #[serde(default)]
    pub entities: Vec<EntitySnapshot>,
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
    Repair { shape: Shape },
    /// Fire a torpedo from the specified tube. `target_uuid` is optional homing target.
    FireTorpedo { tube: TorpedoTube, target_uuid: Option<String> },
    /// Increase power allocation for a console. Validated server-side:
    /// sender must hold `Console::Power`.
    IncreasePower { console: Console },
    /// Decrease power allocation for a console. Validated server-side:
    /// sender must hold `Console::Power`.
    DecreasePower { console: Console },
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
    /// in progress, whether the last cooldown was a penalty, the current
    /// team slot statuses, and the current breakdown (console + shape) or
    /// `None` when the queue is empty.
    RepairState {
        remaining_cooldown_secs: f32,
        in_progress: bool,
        penalty: bool,
        teams: [TeamSlot; 3],
        current_breakdown: Option<(Console, Shape)>,
    },
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
    /// Sent to a specific console holder to show a repair icon with the
    /// given shape. Indistinguishable from a decoy icon on the wire.
    ShowRepairIcon { shape: Shape },
    /// Sent to a specific console holder to clear their repair icon.
    ClearRepairIcon,
    /// Sent at 10 Hz to the Power console holder only. Carries the current
    /// power allocation levels, battery charge fraction, and whether the
    /// system is locked (exhaustion state).
    PowerState {
        helm: u8,
        weapons: u8,
        science: u8,
        battery_charge: f32,
        locked: bool,
    },
    /// Broadcast when a non-asteroid entity is spawned at runtime (e.g. by a
    /// scenario trigger). Carries a full `EntitySnapshot` so the client can
    /// incorporate it immediately.
    EntitySpawned {
        snapshot: EntitySnapshot,
    },
    /// Broadcast when a non-asteroid entity is despawned at runtime.
    /// The client removes it from its local world data idempotently.
    EntityDespawned {
        uuid: String,
    },
}
