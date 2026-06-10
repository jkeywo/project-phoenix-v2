pub use crate::entity_tags::EntityTag;
use crate::flag_kind::FlagKind;
use crate::stations_config::ShipStations;
use bevy::prelude::States;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

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
    RegionEffect {
        uuid: Uuid,
    },
    /// A modifier applied by a world trigger (formerly "scenario"). The
    /// `(id, tag)` pair is the identity key: two applications with the same
    /// pair replace each other via add-or-update semantics.
    World {
        id: String,
        tag: String,
    },
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
            ModifierSource::World { id, tag } => {
                3u8.hash(state);
                id.hash(state);
                tag.hash(state);
            }
        }
    }
}

/// Per-console hull integrity snapshot broadcast in `SimSnapshot`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ConsoleHullStatus {
    pub console: Console,
    pub current: f32,
    pub max_hp: f32,
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
    /// Whether this facing is the currently focused arc.
    #[serde(default)]
    pub is_focused: bool,
}

/// String identifier for a phaser bank, matching the `id` field of the
/// `[[weapons_console.phaser_banks]]` array in `player_ship.toml` (e.g.
/// `"port"`, `"starboard"`). Used in `FirePhaser`, `PhaserFired`,
/// `PhaserBankState`, and `PhaserBankClientConfig`.
pub type PhaserBank = String;

/// String identifier for a torpedo tube, matching the `id` field of the
/// `[[torpedoes.tubes]]` array in `player_ship.toml` (e.g. `"fore_port"`,
/// `"aft"`). Used in `FireTorpedo`, `TorpedoLaunched`, `TorpedoTubeState`,
/// and `TorpedoTubeClientConfig`.
pub type TorpedoTube = String;

/// Per-bank state broadcast to the Tactical operator as part of `WeaponsUpdate`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PhaserBankState {
    pub id: PhaserBank,
    /// True if this bank's locked target is within `beam_range` and inside
    /// the bank's `fire_arc_deg` (manual-fire arc).
    pub fire_ready: bool,
    /// True if the bank is in its post-shot cooldown.
    pub on_cooldown: bool,
    /// Seconds remaining on the cooldown timer (0.0 when ready).
    pub cooldown_remaining: f32,
}

/// Per-tube state broadcast to the Tactical operator as part of `WeaponsUpdate`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TorpedoTubeState {
    pub id: TorpedoTube,
    /// True when the tube is loaded and ready to fire.
    pub loaded: bool,
    /// Seconds remaining on the reload timer (0.0 when loaded).
    pub reload_secs: f32,
}

/// Static, per-bank configuration sent to clients in `Welcome` so the
/// Tactical UI can render the bank's fire arc on the radar. Only
/// `fire_arc_deg` is exposed — `auto_arc_deg` is a server-side concern.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PhaserBankClientConfig {
    pub id: PhaserBank,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
}

/// Static, per-tube configuration sent to clients in `Welcome` so the
/// Tactical UI can render torpedo fire arcs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TorpedoTubeClientConfig {
    pub id: TorpedoTube,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
}

/// Firing mode for phaser banks. Matches `phaser::PhaserMode`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PhaserMode {
    #[default]
    Auto,
    Manual,
}

/// The state of a single repair team, broadcast as part of `RepairState`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum TeamSlot {
    Idle,
    /// Team is en route to the target console. `elapsed` counts up toward 5s.
    Travelling {
        console: Console,
        elapsed: f32,
    },
    /// Team is at the console performing repairs.
    Repairing {
        console: Console,
    },
    /// Team has finished and is returning to engineering.
    /// `remaining` counts down from 5s. `queued` holds the next console to
    /// dispatch to automatically on arrival (if any).
    Returning {
        remaining: f32,
        queued: Option<Console>,
    },
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
    /// The Sensors operator has pushed their long-range radar to the viewscreen.
    SensorsRadar,
    SystemChart,
    /// The Navigation officer has pushed the navigation system chart to the
    /// viewscreen. Shows star, planets, asteroid fields, and ship position.
    NavigationChart,
    /// The Comms officer has pushed a message to the viewscreen.
    Comms,
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
    /// Long-range radar and advisory target suggestion (was part of Science).
    Sensors,
    /// Four-quadrant shield status and focus mechanic (was part of Science).
    Shields,
    /// System chart and impulse-cancel control (was part of Science).
    Navigation,
    Power,
    Comms,
}

impl Console {
    /// Human-readable name suitable for display in UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            Console::CaptainChair => "Captain's Chair",
            Console::Helm => "Helm",
            Console::Tactical => "Tactical",
            Console::Repair => "Repair",
            Console::Sensors => "Sensors",
            Console::Shields => "Shields",
            Console::Navigation => "Navigation",
            Console::Power => "Power",
            Console::Comms => "Comms",
        }
    }

    /// Short abbreviation for use when the tab bar is crowded (5+ consoles).
    pub fn initial(&self) -> &'static str {
        match self {
            Console::CaptainChair => "CC",
            Console::Helm => "H",
            Console::Tactical => "T",
            Console::Repair => "R",
            Console::Sensors => "S",
            Console::Shields => "SH",
            Console::Navigation => "N",
            Console::Power => "P",
            Console::Comms => "C",
        }
    }
}

// ── Comms wire types ──────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

/// A single message in the Comms inbox.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CommsMessage {
    /// Stable identifier for this message (server-assigned UUID).
    pub id: String,
    /// UUID of the entity that sent the message (e.g. a station).
    pub sender_uuid: String,
    /// Display name of the sender.
    pub sender_name: String,
    /// Short subject line shown in the message list.
    pub subject: String,
    /// Full message body shown in the expanded chat view.
    pub body: String,
    /// Available response options. Empty while awaiting a reply (loading).
    pub responses: Vec<String>,
    /// Index into `responses` for the reply the player chose, if any.
    pub selected_response: Option<usize>,
    /// True once the player has opened the message.
    pub is_read: bool,
    /// True when the owning scenario has unloaded; responses are disabled and
    /// a "transmission ended" marker should be shown.
    #[serde(default)]
    pub is_orphaned: bool,
    /// True when the sender is currently within comms range of the player
    /// ship. When false, responses should be disabled and an out-of-range
    /// marker shown. Defaults to true for backward compatibility.
    #[serde(default = "default_true")]
    pub sender_in_range: bool,
    /// Conversation thread identifier. All messages belonging to the same
    /// hail/dialogue tree (initial message + all follow-ups) share this UUID.
    /// Defaults to empty string for backward compatibility with old wire
    /// payloads; the client treats an empty value as "own thread" (= message id).
    #[serde(default)]
    pub thread_id: String,
    /// True when this message was flagged as urgent by the world template.
    /// Urgent messages are shown with a `!` marker and an amber tint in the
    /// inbox; the sender's Hail button also receives the `!` marker while any
    /// unread urgent message from that sender exists.
    #[serde(default)]
    pub is_urgent: bool,
}

/// A contact the Comms operator can hail.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CommsContact {
    /// World-entity UUID (matches `EntitySnapshot::uuid`).
    pub uuid: String,
    /// Display name shown in the contact list.
    pub name: String,
    /// True when the contact is currently within comms range of the player
    /// ship. Out-of-range contacts should be hidden or visually muted.
    /// Defaults to true for backward compatibility.
    #[serde(default = "default_true")]
    pub in_range: bool,
    /// True when this contact has at least one unread urgent message in the
    /// inbox. Derived server-side on each `CommsState` broadcast; not stored
    /// persistently. Defaults to false for backward compatibility.
    #[serde(default)]
    pub is_urgent: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Default, States)]
pub enum GamePhase {
    #[default]
    Lobby,
    InProgress,
    GameOver,
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
    /// Current per-console complexity preset selection.
    #[serde(default)]
    pub complexity: HashMap<Console, String>,
    /// Static world data — `Some` only after `StartGame` has populated the
    /// world; `None` while in Lobby or before world initialisation.
    #[serde(default)]
    pub world: Option<WorldData>,
}

/// Static, per-ship configuration sent to clients in `Welcome`.
///
/// Carries the bits of `assets/entities/player_ship.toml` that the client UI
/// needs to render correctly (e.g. helm radar range). Falls back to sensible
/// defaults via `Default` so test code that builds a `Welcome` doesn't have to
/// know about every field.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ShipClientConfig {
    /// Detection range for the helm radar widget, in world units. Sourced
    /// from `[helm_console.radar] range` in the ship TOML.
    #[serde(default = "default_helm_radar_range")]
    pub helm_radar_range: f32,
    /// Number of repair teams on this ship. Sourced from `[hull]
    /// repair_team_count` in the ship TOML. Used by the client Repair panel to
    /// pre-seed team rows on `Welcome` before the first `RepairState` broadcast
    /// arrives.
    #[serde(default = "default_repair_team_count")]
    pub repair_team_count: u8,
    /// Seconds a repair team spends travelling to a console (and returning).
    /// Sourced from `[repair] travel_duration_secs` in the ship TOML. Used by
    /// the client Repair panel to render travel/return progress bars.
    #[serde(default = "default_repair_travel_secs")]
    pub repair_travel_secs: f32,
    /// HP restored per second while a repair team is at a console. Sourced
    /// from `[repair] repair_rate_hp_per_sec`. Used by the client Repair
    /// panel to derive the in-progress repair bar fill duration from the
    /// target console's `max_hp`.
    #[serde(default = "default_repair_rate_hp_per_sec")]
    pub repair_rate_hp_per_sec: f32,
    /// Seconds the impulse drive takes to fully charge. Sourced from
    /// `[helm_console] impulse_charge_duration` in the ship TOML. Used by
    /// the client helm panel to render the charging progress bar at the
    /// correct rate.
    #[serde(default = "default_impulse_charge_duration")]
    pub impulse_charge_duration: f32,
    /// Phaser banks defined on the ship, in TOML order. Used by the Tactical
    /// UI to render fire-arc overlays on radar and label fire buttons.
    #[serde(default)]
    pub phaser_banks: Vec<PhaserBankClientConfig>,
    /// Torpedo tubes defined on the ship, in TOML order.
    #[serde(default)]
    pub torpedo_tubes: Vec<TorpedoTubeClientConfig>,
    /// RGBA colour the renderer uses for phaser beams (from `[weapons_console]
    /// beam_color`). Defaults to a generic orange when missing from TOML.
    #[serde(default = "default_phaser_beam_color")]
    pub phaser_beam_color: [f32; 4],
    /// RGBA colour the Tactical UI uses for torpedo fire-arc overlays.
    #[serde(default = "default_torpedo_arc_color")]
    pub torpedo_arc_color: [f32; 4],
    /// Tag filter for the Helm radar widget. Sourced from
    /// `[helm_console.radar] shows` in the ship TOML.
    #[serde(default)]
    pub helm_radar_shows: Vec<String>,
    /// Detection range for the Sensors long-range radar widget, in world
    /// units. Sourced from `[sensors_console.long_range_radar] range` in the
    /// ship TOML.
    #[serde(default = "default_sensors_radar_range")]
    pub sensors_radar_range: f32,
    /// Tag filter for the Sensors/Science long-range radar. Sourced from
    /// `[sensors_console.long_range_radar] shows`.
    #[serde(default)]
    pub sensors_radar_shows: Vec<String>,
    /// Tag filter for the Navigation system chart. Sourced from
    /// `[navigation_console.system_chart] shows`.
    #[serde(default)]
    pub nav_chart_shows: Vec<String>,
    /// Tag filter for the Tactical radar widget. Sourced from
    /// `[weapons_console.radar] shows`.
    #[serde(default)]
    pub tactical_radar_shows: Vec<String>,
    /// Detection range for the Tactical radar widget, in world units. Sourced
    /// from `[weapons_console.radar] range` in the ship TOML.
    #[serde(default = "default_tactical_radar_range")]
    pub tactical_radar_range: f32,
}

fn default_tactical_radar_range() -> f32 {
    300.0
}

fn default_helm_radar_range() -> f32 {
    500.0
}

fn default_sensors_radar_range() -> f32 {
    500.0
}

fn default_repair_team_count() -> u8 {
    2
}

fn default_repair_travel_secs() -> f32 {
    5.0
}

fn default_repair_rate_hp_per_sec() -> f32 {
    0.5
}

fn default_impulse_charge_duration() -> f32 {
    3.0
}

fn default_phaser_beam_color() -> [f32; 4] {
    [1.0, 0.6, 0.1, 1.0]
}

fn default_torpedo_arc_color() -> [f32; 4] {
    [0.2, 0.7, 1.0, 1.0]
}

impl Default for ShipClientConfig {
    fn default() -> Self {
        Self {
            helm_radar_range: default_helm_radar_range(),
            repair_team_count: default_repair_team_count(),
            repair_travel_secs: default_repair_travel_secs(),
            repair_rate_hp_per_sec: default_repair_rate_hp_per_sec(),
            impulse_charge_duration: default_impulse_charge_duration(),
            phaser_banks: Vec::new(),
            torpedo_tubes: Vec::new(),
            phaser_beam_color: default_phaser_beam_color(),
            torpedo_arc_color: default_torpedo_arc_color(),
            helm_radar_shows: Vec::new(),
            sensors_radar_range: default_sensors_radar_range(),
            sensors_radar_shows: Vec::new(),
            nav_chart_shows: Vec::new(),
            tactical_radar_shows: Vec::new(),
            tactical_radar_range: default_tactical_radar_range(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SimSnapshot {
    pub red_alert: bool,
    pub view_mode: ViewMode,
    pub ship_x: f32,
    pub ship_z: f32,
    pub ship_yaw: f32,
    /// Current forward speed in world units per second. Negative = reversing.
    #[serde(default)]
    pub forward_speed: f32,
    /// Current power allocation levels (Helm, Weapons, Science).
    /// Added to SimSnapshot so all clients see the current configuration.
    #[serde(default = "default_power_levels")]
    pub power_levels: (u8, u8, u8),
    /// Impulse drive charge progress (0.0 = idle, 0.1–1.0 = charging, 1.0 = active).
    /// Broadcast so console panels can show the current impulse drive status.
    #[serde(default)]
    pub impulse_charge_progress: f32,
    /// Engine thrust level for audio volume mapping.
    /// 1.0 when the impulse drive is fully active; otherwise `|helm_thrust|` (0.0–1.0).
    #[serde(default)]
    pub engine_thrust: f32,
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
    /// Per-console hull integrity. Empty when the ship has no per-console hull config.
    #[serde(default)]
    pub console_hull: Vec<ConsoleHullStatus>,
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
    /// Display name from the entity TOML `name` scalar (e.g. "Pirate Raider").
    /// `None` for entities that have no name (e.g. asteroids).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// Seconds remaining until the entity completes warp-out (set while in `warping_out` state).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp_out_remaining_secs: Option<f32>,
    /// Optional per-entity world-space size override for radar blip
    /// rendering. When `None`, clients fall back to `radius`. Authors
    /// set this in the entity TOML's `[radar_appearance]` table to fudge
    /// radar visibility independently of the entity's actual physical size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar_world_size: Option<f32>,
    /// Half-extents for Box-shaped region entities. `[x, y, z]` in world units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_extents: Option<[f32; 3]>,
    /// Radar icon name derived from entity tags by the server at snapshot-encode
    /// time. One of `"ship"`, `"asteroid"`, `"station"`, `"planet"`, `"star"`,
    /// `"torpedo"`. `None` defaults to `"ship"` on the client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar_icon: Option<String>,
    /// Set to `true` when this entity is referenced by an active mission
    /// objective. The client radar renders a visual indicator for these entities.
    #[serde(default)]
    pub objective_target: bool,
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

    /// Half-extents for Box-shaped region entities, or zero array when missing.
    pub fn half_extents_or_zero(&self) -> [f32; 3] {
        self.half_extents.unwrap_or([0.0, 0.0, 0.0])
    }

    /// Convenience constructor for an asteroid entity (the most common case).
    pub fn asteroid(uuid: impl Into<String>, x: f32, z: f32, radius: f32) -> Self {
        Self {
            uuid: uuid.into(),
            id: None,
            name: None,
            position: Some([x, 0.0, z]),
            tags: vec!["asteroid".into()],
            shape: None,
            radius: Some(radius),
            colour: None,
            yaw: None,
            hull_fraction: None,
            inner_radius: None,
            warp_out_remaining_secs: None,
            radar_world_size: None,
            half_extents: None,
            radar_icon: Some("asteroid".into()),
            objective_target: false,
        }
    }

    /// Convenience constructor for an asteroid field entity.
    pub fn asteroid_field(
        uuid: impl Into<String>,
        x: f32,
        z: f32,
        inner_radius: f32,
        outer_radius: f32,
    ) -> Self {
        Self {
            uuid: uuid.into(),
            id: None,
            name: None,
            position: Some([x, 0.0, z]),
            tags: vec!["asteroid_field".into()],
            shape: Some("torus".into()),
            radius: Some(outer_radius),
            colour: None,
            yaw: None,
            hull_fraction: None,
            inner_radius: Some(inner_radius),
            warp_out_remaining_secs: None,
            radar_world_size: None,
            half_extents: None,
            radar_icon: Some("asteroid".into()),
            objective_target: false,
        }
    }

    /// Convenience constructor for a basic entity with position and tags (no extra aspects).
    pub fn simple(uuid: impl Into<String>, x: f32, z: f32, tags: Vec<String>) -> Self {
        Self {
            uuid: uuid.into(),
            id: None,
            name: None,
            position: Some([x, 0.0, z]),
            tags,
            shape: None,
            radius: None,
            colour: None,
            yaw: None,
            hull_fraction: None,
            inner_radius: None,
            warp_out_remaining_secs: None,
            radar_world_size: None,
            half_extents: None,
            radar_icon: None,
            objective_target: false,
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
    /// Four-quadrant shield state, present only for ship-like entities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shields: Option<Vec<ShieldFacingStatus>>,
    /// Seconds remaining until the entity warps out (present only while in `WarpingOut` AI state).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp_out_remaining_secs: Option<f32>,
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
    /// Scenario title for display in the lobby.
    #[serde(default)]
    pub scenario_title: String,
    /// Scenario description / body for display in the lobby.
    #[serde(default)]
    pub scenario_description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum ClientMessage {
    Identify {
        token: String,
        name: String,
    },
    SetName {
        name: String,
    },
    SelectStation {
        station: String,
    },
    ReleaseStation,
    StartGame,
    ToggleRedAlert,
    HelmInput {
        thrust: f32,
        steering: f32,
    },
    /// Sent by the Helm officer to begin charging the impulse drive.
    StartImpulseCharge,
    /// Sent by the Science officer to cancel an active or charging impulse drive.
    CancelImpulse,
    SetView {
        mode: ViewMode,
    },
    SetTarget {
        uuid: String,
    },
    /// Science officer taps an entity on their radar to suggest it as a target
    /// to the Weapons console. Advisory only — does not affect lock state.
    SetScienceTarget {
        uuid: String,
    },
    /// Sensors operator taps an entity on their long-range radar to suggest it
    /// as a target to Tactical. Advisory only — does not affect lock state.
    SetSensorsTarget {
        uuid: String,
    },
    FirePhaser {
        bank: PhaserBank,
    },
    SetPhaserMode {
        mode: PhaserMode,
    },
    /// Dispatch a specific repair team (by index) to the named console.
    /// Supports redirect and recall (see PRD #305).
    DispatchRepairTeam {
        team_idx: u8,
        console: Console,
    },
    /// Fire a torpedo from the specified tube. `target_uuid` is optional homing target.
    FireTorpedo {
        tube: TorpedoTube,
        target_uuid: Option<String>,
    },
    /// Increase power allocation for a console. Validated server-side:
    /// sender must hold `Console::Power`.
    IncreasePower {
        console: Console,
    },
    /// Decrease power allocation for a console. Validated server-side:
    /// sender must hold `Console::Power`.
    DecreasePower {
        console: Console,
    },
    /// Change the complexity preset for a console the sender holds.
    /// Validated server-side: sender must hold the console and the preset
    /// name must exist in the ship's complexity config.
    SetComplexity {
        console: Console,
        preset_name: String,
    },
    /// Set the phaser emitter frequency (0.0–1.0).
    ///
    /// Normally sent by the Tactical holder. When Tactical is at Low
    /// complexity, the Science holder may also send this message
    /// (delegation allowlist in `delegation.rs`).
    SetPhaserFrequency {
        frequency: f32,
    },
    /// Hail a target entity (e.g. a station). Server responds with a
    /// `CommsState` update that adds the contact's message to the inbox.
    /// Sender must hold `Console::Comms`.
    Hail {
        target_uuid: String,
    },
    /// Select a message in the Comms inbox (opens the chat view).
    SelectCommsMessage {
        message_id: String,
    },
    /// Choose a response to a received message.
    RespondToMessage {
        message_id: String,
        response_index: usize,
    },
    /// Clear all read or orphaned messages from the inbox.
    ClearComms,
    /// Display the selected comms message on the viewscreen for the whole crew.
    /// Pushes `ViewMode::Comms` and stores the message in `OnScreenMessage`.
    /// Sender must hold `Console::Comms`.
    ShowOnScreen {
        message_id: String,
    },
    /// Focus one shield arc (Fore/Port/Aft/Starboard), or `None` to clear focus.
    /// Sender must hold `Console::Shields`.
    SetShieldFocus {
        facing: Option<ViewDirection>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum ServerMessage {
    Welcome {
        state: GameState,
        ship_stations: ShipStations,
        ship_config: ShipClientConfig,
    },
    PlayerJoined {
        player: Player,
    },
    PlayerLeft {
        token: String,
    },
    StationAssigned {
        token: String,
        station: Option<String>,
        consoles: Vec<Console>,
    },
    NameChanged {
        token: String,
        name: String,
    },
    GameStarted,
    SimState {
        snapshot: SimSnapshot,
    },
    WorldSetup {
        world: WorldData,
    },
    TargetLock {
        uuid: String,
        locked: bool,
    },
    /// Sent at 10 Hz to the Weapons console player only. `target_uuid` is the
    /// currently locked target (`None` if no lock). `banks` carries per-bank
    /// fire-ready / cooldown state in TOML order, and `tubes` carries per-tube
    /// load state in TOML order. `torpedo_count` is the shared magazine.
    WeaponsUpdate {
        target_uuid: Option<String>,
        /// Display name of the locked target entity, if known.
        #[serde(default)]
        target_name: Option<String>,
        banks: Vec<PhaserBankState>,
        tubes: Vec<TorpedoTubeState>,
        /// Remaining torpedoes in the shared magazine.
        torpedo_count: u32,
        /// Current phaser firing mode (Auto or Manual).
        phaser_mode: PhaserMode,
    },
    /// Broadcast when a phaser beam starts. Sent to all players so the renderer
    /// can draw the beam on the viewscreen.
    ///
    /// `source_uuid` is the firing entity's UUID — the player ship for player
    /// phasers, an NPC's `EntityUuid` for NPC phasers. The renderer resolves
    /// it to a Transform to anchor the beam's origin point.
    BeamStarted {
        bank: PhaserBank,
        source_uuid: String,
        target_uuid: String,
    },
    /// Broadcast when a phaser beam ends (natural expiry, sever, or cancel).
    BeamEnded {
        bank: PhaserBank,
        source_uuid: String,
        target_uuid: String,
    },
    /// Broadcast when an asteroid's HP reaches 0 and it is despawned.
    AsteroidDestroyed {
        uuid: String,
    },
    /// Broadcast to all players when the Science officer taps a radar entity
    /// to designate it as a suggested target. Advisory only — does not lock
    /// the Tactical console.
    ScienceTargetSuggestion {
        uuid: String,
    },
    /// Broadcast to all players when the Sensors operator taps an entity on
    /// their long-range radar to designate it as a suggested target for
    /// Tactical. Advisory only — does not lock the Tactical console.
    SensorsTargetSuggestion {
        uuid: String,
    },
    /// Sent when a phaser bank fires a shot at a target.
    PhaserFired {
        bank: PhaserBank,
        target_uuid: String,
    },
    /// Sent at 10 Hz to the Repair console holder. Contains the current
    /// state of all repair teams, each with a `target_console` field.
    RepairState {
        teams: Vec<TeamSlot>,
    },
    /// Sent at 10 Hz (or on change) to all players. Contains HP and online
    /// status for every shield facing.
    ShieldStatus {
        facings: Vec<ShieldFacingStatus>,
    },
    /// Broadcast to all when a torpedo is launched from a tube.
    TorpedoLaunched {
        uuid: String,
        tube: TorpedoTube,
        x: f32,
        z: f32,
        heading: f32,
    },
    /// Broadcast to all when a torpedo is destroyed (expired or hit something).
    TorpedoDestroyed {
        uuid: String,
    },
    /// Broadcast when a modifier is added or updated on the ship.
    ModifierAdded {
        source: ModifierSource,
        slot: ModifierSlot,
        bonus: f32,
    },
    /// Broadcast when a modifier is removed from the ship.
    ModifierRemoved {
        source: ModifierSource,
        slot: ModifierSlot,
    },
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
        radius: f32,
    },
    /// Sent at 10 Hz to the Power console holder only. Carries the current
    /// power allocation levels, battery charge fraction, and whether the
    /// system is locked (exhaustion state).
    PowerState {
        helm: u8,
        weapons: u8,
        sensors: u8,
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
    /// Broadcast when a console's complexity preset changes.
    ComplexityChanged {
        console: Console,
        preset_name: String,
    },
    /// Sent to the Tactical console holder to hint the correct phaser-frequency
    /// button to press. Emitted by the Science Low AI after `auto_hint_delay_secs`
    /// of a shared locked target.
    FrequencyHint {
        /// The recommended phaser frequency (0.0–1.0).
        frequency: f32,
    },
    /// Broadcast when a station entity is spawned. Clients use this to add
    /// the station to their local world, show it on radar, and make it
    /// targetable by Tactical.
    StationSpawned {
        uuid: String,
        name: String,
        position: [f32; 3],
        /// Render shape: "sphere", "cylinder", or "torus".
        shape: String,
        radius: f32,
        hull_integrity: f32,
    },
    /// Broadcast when a station entity is destroyed (hull reaches 0).
    /// Clients remove it from their local world idempotently.
    StationDestroyed {
        uuid: String,
    },
    /// Pushed to the captain only when the objective list changes (event-driven,
    /// not polled). The list is pre-sorted: mandatory objectives first, then
    /// optional, in insertion order within each group.
    ObjectiveSummary {
        objectives: Vec<ObjectiveSnapshot>,
    },
    /// Sent to the Comms console holder. Contains the current inbox, active
    /// objectives visible to Comms, and the list of hailable contacts.
    /// Broadcast on change (not polled), and replayed on reconnect.
    CommsState {
        messages: Vec<CommsMessage>,
        objectives: Vec<ObjectiveSnapshot>,
        contacts: Vec<CommsContact>,
    },
    /// Broadcast to all players when every console's HP reaches 0.
    /// Clients should show a game-over screen.
    ShipDestroyed,
    /// Broadcast when the game transitions to the GameOver phase.
    /// Carries a human-readable reason string displayed on the game-over screen.
    GameOver {
        reason: String,
    },
    /// Broadcast when the ship takes damage (from collision or damage zone).
    /// `shield` = HP absorbed by shields, `hull` = HP that reached the hull.
    /// Either field may be zero (e.g. shield-only hit has `hull: 0.0`).
    DamageTaken {
        hull: f32,
        shield: f32,
    },
}

// ── HTML console bridge wire types (ADR-0001 / PRD #419) ───────────────────

/// Serialised HUD state pushed to the viewscreen HTML overlay (issue #422).
///
/// Produced by the viewscreen border plugin on change and encoded via
/// `codec::encode_hud_state`. The JS `window.__updateHud` parses this to
/// drive the bottom status strip and the red-alert vignette.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewscreenHudState {
    /// Compass bearing 0–359 (from `yaw_to_compass_bearing`).
    pub heading: u32,
    /// Hull integrity percentage, clamped 0–100.
    pub hull_pct: i32,
    /// Condition string — `"ALERT"` or `"NOMINAL"`.
    pub condition: String,
    /// Whether the ship is at red alert (drives the CSS vignette).
    pub red_alert: bool,
}

/// A single radar blip on the Tactical console radar.
///
/// Positions are normalised to `[-1.0, 1.0]` where ±1.0 = the effective
/// tactical radar range (base `tactical_radar_range` × `RadarRange` modifier).
/// Produced server-side by `recompute_weapons_console_state` from live ECS
/// transforms joined with the static world entity registry for tags/radius.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RadarBlip {
    /// Stable entity UUID — matches `EntitySnapshot::uuid`. Used to correlate
    /// with `WeaponsConsoleState::target_uuid` for lock highlight.
    pub uuid: String,
    /// Radar-space X normalised to `[-1.0, 1.0]` at effective tactical range.
    /// Positive = starboard (right on the radar display).
    pub radar_x: f32,
    /// Radar-space Y normalised to `[-1.0, 1.0]` at effective tactical range.
    /// Positive = forward (up on the radar display).
    pub radar_y: f32,
    /// Scaled radius: `world_radius / effective_range`. Zero for entities
    /// that carry no radius in the world registry.
    pub scaled_radius: f32,
    /// Display kind derived from entity tags.  One of `"asteroid"`, `"ship"`,
    /// `"station"`, or `"unknown"`. Drives blip colour / icon in the HTML
    /// radar renderer.
    pub kind: String,
    /// Icon name for radar display (matches CSS class in `radar-widget.js`).
    /// Derived from entity tags or explicit `radar_icon` from snapshot.
    #[serde(default)]
    pub icon: String,
    /// RGB colour tint for the blip, normalised 0.0–1.0.  Defaults to a
    /// per-kind palette when the snapshot carries no explicit colour.
    #[serde(default)]
    pub color: [f32; 3],
    /// `true` when this entity is referenced by an active mission objective.
    /// The HTML radar widget uses this to draw an objective ring.
    #[serde(default)]
    pub objective_target: bool,
    /// Display name from the entity snapshot, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A radar overlay region drawn as a coloured shape on the Tactical radar.
/// Produced server-side from world entities that carry a `shape` field.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RadarRegion {
    pub uuid: String,
    /// World-space centre X.
    pub x: f32,
    /// World-space centre Z.
    pub z: f32,
    /// Shape type: `"sphere"`, `"box"`, or `"torus"`.
    pub shape: String,
    /// Radius in world units (sphere radius, box circumradius, torus outer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    /// Inner radius for torus shapes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_radius: Option<f32>,
    /// Outer radius for torus shapes (same as `radius` for box/sphere).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer_radius: Option<f32>,
    /// Half-extents `[half_x, half_z]` for box shapes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_extents: Option<[f32; 2]>,
    /// Yaw in radians.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yaw: Option<f32>,
    /// RGB colour tint, normalised 0.0–1.0.
    #[serde(default)]
    pub color: [f32; 3],
    /// Display name from the entity snapshot, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Serialised Tactical (Weapons) console state pushed to the HTML console
/// (issue #422). Mirrors the data assembled by `weapons_update_broadcaster`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WeaponsConsoleState {
    pub target_uuid: Option<String>,
    #[serde(default)]
    pub target_name: Option<String>,
    pub banks: Vec<PhaserBankState>,
    pub tubes: Vec<TorpedoTubeState>,
    pub torpedo_count: u32,
    pub phaser_mode: PhaserMode,
    /// Phaser bank fire-arc geometry (static, sourced from `ShipClientConfigResource`).
    /// Included in every push so the HTML console can draw arc overlays without
    /// ever receiving a `Welcome` message (which only goes to Bevy clients).
    #[serde(default)]
    pub phaser_arcs: Vec<PhaserBankClientConfig>,
    /// Torpedo tube fire-arc geometry (static, from `ShipClientConfigResource`).
    #[serde(default)]
    pub torpedo_arcs: Vec<TorpedoTubeClientConfig>,
    /// Live radar blips: entities within effective tactical radar range, filtered
    /// by `tactical_radar_shows`. Normalised to `[-1.0, 1.0]` radar space where
    /// ±1.0 = effective range boundary.
    #[serde(default)]
    pub blips: Vec<RadarBlip>,
    /// Region shapes for zone overlays on the HTML radar widget.
    /// Sourced from world entities that carry a `shape` field.
    #[serde(default)]
    pub regions: Vec<RadarRegion>,
}

/// Serialised Captain console state pushed to the HTML captain panel.
///
/// Mirrors `WeaponsConsoleState` — written into a single
/// `CaptainConsoleStateComp` component and pushed on change via
/// `ConsoleStateChanged { name: "CaptainChair", json }`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CaptainConsoleState {
    /// Whether the ship is at red alert.
    pub red_alert: bool,
    /// Current camera direction as a plain string: `"Fore"`, `"Port"`,
    /// `"Starboard"`, or `"Aft"`. Non-camera view modes (Radar, etc.) fall
    /// back to `"Fore"`. Serialised directly so the HTML console JS can read
    /// `s.view_direction` without parsing the tagged `ViewMode` enum shape.
    pub view_direction: String,
    /// Mission objectives. Updated when `ObjectiveManager` is dirty.
    #[serde(default)]
    pub objectives: Vec<ObjectiveSnapshot>,
    /// Overall ship hull integrity as a percentage (0–100).
    pub hull_integrity_pct: f32,
    /// Computed game status string shown in the captain panel.
    #[serde(default)]
    pub game_status: String,
}

impl Default for CaptainConsoleState {
    fn default() -> Self {
        Self {
            red_alert: false,
            view_direction: "Fore".into(),
            objectives: Vec::new(),
            hull_integrity_pct: 100.0,
            game_status: String::new(),
        }
    }
}

/// Serialised Helm console state pushed to the HTML helm panel (issue #423).
///
/// Mirrors `WeaponsConsoleState` — written into a single `HelmConsoleStateComp`
/// component and pushed on change via `ConsoleStateChanged { name: "Helm", json }`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HelmConsoleState {
    pub heading: f32,
    pub speed: f32,
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    #[serde(default)]
    pub impulse_charge_progress: f32,
    #[serde(default)]
    pub on_screen: bool,
    #[serde(default)]
    pub lock_id: Option<String>,
}

/// Serialised Shields console state pushed to the HTML shields panel (issue #423).
///
/// Broadcast at 10 Hz only to the player holding `Console::Shields`.
/// Focus and target-bearing are computed server-side from live resources.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ShieldsConsoleState {
    /// Current shield quadrant snapshots (Fore, Port, Aft, Starboard).
    pub facings: Vec<ShieldFacingStatus>,
    /// Overall ship hull integrity as a percentage (0–100).
    pub hull_integrity_pct: f32,
    /// Label of the currently focused facing (None = balanced/omni).
    pub focused_facing: Option<String>,
    /// Grid status string (e.g. "GRID NOMINAL", "EMITTER OFFLINE").
    pub grid_status: String,
    /// Bearing of the current Tactical target in degrees, or None if no target.
    #[serde(default)]
    pub target_bearing: Option<f32>,
}

/// Serialised Power console state pushed to the HTML power panel (issue #425).
///
/// Data-driven: `consoles` is derived from `PowerMultiplierResource` so adding
/// new powered consoles in the ship TOML automatically extends the panel without
/// any HTML changes.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PowerConsoleState {
    /// Per-console power allocation entries. One entry per powered console.
    pub consoles: Vec<PowerConsoleEntry>,
    /// Sum of current allocations across all powered consoles.
    pub total: u8,
    /// Maximum total allocation (pool cap).
    pub total_max: u8,
    /// Current battery charge (0 – `battery_max`).
    pub battery_charge: f32,
    /// Maximum battery capacity (from `PowerConfig::capacity`).
    pub battery_max: f32,
    /// Whether the power system is locked (battery exhausted).
    pub locked: bool,
}

/// A single entry in [`PowerConsoleState::consoles`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PowerConsoleEntry {
    /// Console identifier — the `Console` enum variant name (e.g. `"Helm"`).
    pub id: String,
    /// Display label shown in the HTML panel (e.g. `"HELM"`, `"WEAPONS"`).
    pub label: String,
    /// Current power level (1 – `max_level`).
    pub level: u8,
    /// Maximum power level for this console.
    pub max_level: u8,
}

/// Serialised Repair console state pushed to the HTML repair panel (issue #425).
///
/// Carries everything the HTML panel needs: team state machine slots, per-console
/// hull status, travel timing, and the ordered list of dispatchable consoles.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RepairConsoleState {
    /// Current team slot states (one entry per repair team).
    pub teams: Vec<TeamSlot>,
    /// Per-console hull status. Drives the hull bar and team-destination labels.
    pub console_hull: Vec<ConsoleHullStatus>,
    /// Travel duration in seconds (from ship TOML `[repair]` block).
    pub travel_duration_secs: f32,
    /// Consoles that can be targeted for repair dispatch (in display order).
    pub damageable_consoles: Vec<Console>,
}

/// A console action decoded from the `window.__sendAction` envelope
/// (ADR-0001 §1). The envelope's extra `console` field is ignored by serde.
///
/// Modelled after Tactical actions (issue #422) and Captain actions (issue #428).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UiAction {
    FireTorpedo {
        tube: String,
        #[serde(default)]
        target_uuid: Option<String>,
    },
    FirePhaser {
        bank: String,
    },
    /// Toggle the ship's red-alert state (captain only).
    ToggleRedAlert,
    /// Set the camera view direction (captain console).
    /// The HTML captain panel sends `{ action: "set_view", direction: "Fore" }`.
    SetView {
        direction: ViewDirection,
    },
    /// Helm joystick input (helm console).
    HelmInput {
        thrust: f32,
        steering: f32,
    },
    /// Start charging the impulse drive (helm console).
    StartImpulseCharge,
    /// Cancel the impulse drive charge (helm console).
    CancelImpulse,
    /// Switch the viewscreen to radar mode (helm ON SCREEN button).
    SetRadarView,
    /// Increase power for the named powered console (power console).
    ///
    /// The HTML power panel sends
    /// `{ action: "increase_power", console: "Power", target: "Helm" }`.
    IncreasePower {
        target: Console,
    },
    /// Decrease power for the named powered console (power console).
    ///
    /// The HTML power panel sends
    /// `{ action: "decrease_power", console: "Power", target: "Tactical" }`.
    DecreasePower {
        target: Console,
    },
    /// Dispatch a repair team to a console (repair console).
    ///
    /// The HTML repair panel sends
    /// `{ action: "dispatch_repair_team", console: "Repair", team_idx: 0, target: "Helm" }`.
    DispatchRepairTeam {
        team_idx: u8,
        target: Console,
    },
}

/// Maps a decoded [`UiAction`] to the existing [`ClientMessage`] the server
/// handlers already process. Pure — covered by a native unit test.
pub fn ui_action_to_client_message(a: &UiAction) -> ClientMessage {
    match a {
        UiAction::FireTorpedo { tube, target_uuid } => ClientMessage::FireTorpedo {
            tube: tube.clone(),
            target_uuid: target_uuid.clone(),
        },
        UiAction::FirePhaser { bank } => ClientMessage::FirePhaser { bank: bank.clone() },
        UiAction::ToggleRedAlert => ClientMessage::ToggleRedAlert,
        UiAction::SetView { direction } => ClientMessage::SetView {
            mode: ViewMode::Camera(direction.clone()),
        },
        UiAction::HelmInput { thrust, steering } => ClientMessage::HelmInput {
            thrust: *thrust,
            steering: *steering,
        },
        UiAction::StartImpulseCharge => ClientMessage::StartImpulseCharge,
        UiAction::CancelImpulse => ClientMessage::CancelImpulse,
        UiAction::SetRadarView => ClientMessage::SetView {
            mode: ViewMode::Radar,
        },
        UiAction::IncreasePower { target } => ClientMessage::IncreasePower {
            console: target.clone(),
        },
        UiAction::DecreasePower { target } => ClientMessage::DecreasePower {
            console: target.clone(),
        },
        UiAction::DispatchRepairTeam { team_idx, target } => ClientMessage::DispatchRepairTeam {
            team_idx: *team_idx,
            console: target.clone(),
        },
    }
}

#[cfg(test)]
mod ui_action_tests {
    use super::*;

    #[test]
    fn fire_torpedo_maps_to_client_message() {
        let action = UiAction::FireTorpedo {
            tube: "fore".into(),
            target_uuid: None,
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::FireTorpedo {
                tube: "fore".into(),
                target_uuid: None
            }
        );
    }

    #[test]
    fn fire_torpedo_with_target_maps_to_client_message() {
        let action = UiAction::FireTorpedo {
            tube: "fore".into(),
            target_uuid: Some("abc".into()),
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::FireTorpedo {
                tube: "fore".into(),
                target_uuid: Some("abc".into())
            }
        );
    }

    #[test]
    fn fire_phaser_maps_to_client_message() {
        let action = UiAction::FirePhaser { bank: "port".into() };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::FirePhaser { bank: "port".into() }
        );
    }

    #[test]
    fn toggle_red_alert_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::ToggleRedAlert),
            ClientMessage::ToggleRedAlert
        );
    }

    #[test]
    fn set_view_direction_maps_to_client_message() {
        let action = UiAction::SetView {
            direction: ViewDirection::Aft,
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::SetView {
                mode: ViewMode::Camera(ViewDirection::Aft)
            }
        );
    }

    #[test]
    fn helm_input_maps_to_client_message() {
        let action = UiAction::HelmInput {
            thrust: 0.75,
            steering: -0.5,
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::HelmInput {
                thrust: 0.75,
                steering: -0.5
            }
        );
    }

    #[test]
    fn start_impulse_charge_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::StartImpulseCharge),
            ClientMessage::StartImpulseCharge
        );
    }

    #[test]
    fn cancel_impulse_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::CancelImpulse),
            ClientMessage::CancelImpulse
        );
    }

    #[test]
    fn set_radar_view_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::SetRadarView),
            ClientMessage::SetView {
                mode: ViewMode::Radar
            }
        );
    }

    #[test]
    fn increase_power_maps_to_client_message() {
        let action = UiAction::IncreasePower { target: Console::Helm };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::IncreasePower { console: Console::Helm }
        );
    }

    #[test]
    fn decrease_power_maps_to_client_message() {
        let action = UiAction::DecreasePower { target: Console::Tactical };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::DecreasePower { console: Console::Tactical }
        );
    }

    #[test]
    fn dispatch_repair_team_maps_to_client_message() {
        let action = UiAction::DispatchRepairTeam { team_idx: 1, target: Console::Power };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::DispatchRepairTeam { team_idx: 1, console: Console::Power }
        );
    }
}

// ── Objective wire types ───────────────────────────────────────────────────

/// Status of a mission objective.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ObjectiveStatus {
    Active,
    Completed,
    Failed,
}

/// A single objective as sent to the captain panel.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObjectiveSnapshot {
    /// Stable identifier for this objective (scoped to the scenario that created it).
    pub id: String,
    /// Human-readable description shown on the captain panel.
    pub text: String,
    /// Mandatory objectives must be completed; optional are bonus.
    pub mandatory: bool,
    pub status: ObjectiveStatus,
    /// Entity names this objective is associated with. Each named entity is
    /// marked on the nav radar with an objective ring. May reference real
    /// entities (stations, ships) or invisible `objective_marker` beacons
    /// placed at anchor coordinates. Empty when the objective has no spatial
    /// target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
}

/// JSON payload pushed to the HTML lobby via `LobbyStateChanged`.
/// Mirrors the `LobbyView` derived state for server-side HTML rendering.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LobbyStatePayload {
    pub phase: String,
    pub scenario_title: String,
    pub scenario_body: String,
    pub crew_count: u32,
    pub max_players: u32,
    pub all_stations_filled: bool,
    pub stations: Vec<StationPayload>,
    pub spectators: Vec<String>,
}

/// One station slot in the lobby grid payload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StationPayload {
    pub name: String,
    pub short_code: String,
    pub rank: String,
    pub consoles: Vec<Console>,
    pub holder_name: Option<String>,
    pub is_mine: bool,
    pub preset_names: Vec<String>,
}
