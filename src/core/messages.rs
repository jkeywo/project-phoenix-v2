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

/// Stable, designer-authored identifier for a claimable ship station.
///
/// Station ids are ship-local authoring keys, not player tokens and not world
/// entity UUIDs. They are intended to replace console bundles as the wire
/// addressing unit for station ownership in the station/system architecture.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct StationId(pub String);

/// Stable, designer-authored identifier for one capability instance on a ship.
///
/// System ids are ship-wide unique authoring keys such as `phaser-fore` or
/// `torpedo-tube-aft`. They are distinct from world entity UUIDs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SystemId(pub String);

/// Stable, designer-authored identifier for an operator-facing power group.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PowerGroupId(pub String);

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
    /// Seconds remaining on the current load/unload timer (0.0 when done).
    pub reload_secs: f32,
    /// Load state label: "loaded" | "unloaded" | "loading" | "unloading".
    #[serde(default)]
    pub state: String,
    /// Completion fraction `[0.0, 1.0]` for the current load/unload operation.
    #[serde(default)]
    pub progress: f32,
    /// Tube-specific load/unload duration in seconds.
    #[serde(default)]
    pub load_time: f32,
}

/// Static, per-bank configuration sent to clients in `Welcome` so the
/// Tactical UI can render the bank's fire arc on the radar and the
/// per-bank cooldown bar. Only `fire_arc_deg` is exposed —
/// `auto_arc_deg` is a server-side concern.
///
/// `cooldown_secs` is the bank's post-beam cooldown duration in seconds,
/// used by the client as the denominator when rendering the per-bank
/// cooldown bar from `PhaserBankState.cooldown_remaining`. `0.0` means
/// the server is using its default cooldown (the client should render
/// the bar from the live remaining value alone, capped at its own
/// historic peak).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PhaserBankClientConfig {
    pub id: PhaserBank,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    #[serde(default)]
    pub cooldown_secs: f32,
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum TeamSlot {
    #[default]
    Idle,
    /// Team is en route to the target console. `elapsed` counts up toward 5s.
    Travelling { console: Console, elapsed: f32 },
    /// Team is at the console performing repairs.
    Repairing { console: Console },
    /// Team has finished and is returning to engineering.
    /// `remaining` counts down from 5s. `queued` holds the next console to
    /// dispatch to automatically on arrival (if any).
    Returning {
        remaining: f32,
        queued: Option<Console>,
    },
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

#[cfg(test)]
mod console_id_tests {
    use super::*;

    #[test]
    fn station_console_id_round_trips() {
        let consoles = [
            Console::CaptainChair,
            Console::Helm,
            Console::Tactical,
            Console::Repair,
            Console::Sensors,
            Console::Shields,
            Console::Navigation,
            Console::Power,
            Console::Comms,
            Console::Core,
        ];
        for console in &consoles {
            assert_eq!(
                Console::from_console_id(console.station_console_id()),
                Some(console.clone()),
                "round-trip failed for {:?}",
                console
            );
        }
    }

    #[test]
    fn from_console_id_rejects_unknown() {
        assert_eq!(Console::from_console_id("unknown"), None);
        assert_eq!(Console::from_console_id(""), None);
        assert_eq!(Console::from_console_id("HELM"), None);
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
    /// Ownerless AI-only systems (viewscreen etc.). Not player-selectable;
    /// used as a repair target for ship-wide core systems.
    Core,
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
            Console::Core => "Core",
        }
    }

    /// Short abbreviation for use when the tab bar is crowded (5+ consoles).
    /// Console identifier string used in `StationConfig.console` TOML field
    /// to associate a station config with a console variant.
    pub fn station_console_id(&self) -> &'static str {
        match self {
            Console::CaptainChair => "captain",
            Console::Helm => "helm",
            Console::Tactical => "tactical",
            Console::Repair => "repair",
            Console::Sensors => "sensors",
            Console::Shields => "shields",
            Console::Navigation => "navigation",
            Console::Power => "power",
            Console::Comms => "comms",
            Console::Core => "core",
        }
    }

    /// Resolves a station console id string back to the matching [`Console`] variant.
    /// Returns `None` if `id` does not match any known console.
    /// Symmetric with [`Console::station_console_id`].
    pub fn from_console_id(id: &str) -> Option<Console> {
        match id {
            "captain" => Some(Console::CaptainChair),
            "helm" => Some(Console::Helm),
            "tactical" => Some(Console::Tactical),
            "repair" => Some(Console::Repair),
            "sensors" => Some(Console::Sensors),
            "shields" => Some(Console::Shields),
            "navigation" => Some(Console::Navigation),
            "power" => Some(Console::Power),
            "comms" => Some(Console::Comms),
            "core" => Some(Console::Core),
            _ => None,
        }
    }

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
            Console::Core => "CO",
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
    /// Transient phase: asset pre-cache is running after captain pressed Engage.
    /// Auto-transitions to `InProgress` when all rendering assets are ready.
    Loading,
    InProgress,
    GameOver,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Player {
    pub token: String,
    pub name: String,
    pub connected: bool,
    /// True when this player has signalled they are ready to start.
    /// Used in the per-player Ready flow replacing captain Engage.
    #[serde(default)]
    pub ready: bool,
    /// C1: stable station ID — primary addressing unit, replaces consoles over time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<StationId>,
    /// Last rating active for this player's station (persists across disconnect for backfill).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rating: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GameState {
    pub phase: GamePhase,
    pub players: Vec<Player>,
    /// Current per-console complexity preset selection.
    #[serde(default)]
    pub complexity: HashMap<Console, String>,
    /// Static world data — `Some` only after game start has populated the
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
    /// Targetability filter for the Tactical radar. Sourced from
    /// `[weapons_console.radar] selects`.
    #[serde(default)]
    pub tactical_radar_selects: Vec<String>,
    /// Targetability filter for the Sensors long-range radar. Sourced from
    /// `[sensors_console.long_range_radar] selects`.
    #[serde(default)]
    pub sensors_radar_selects: Vec<String>,
    /// Targetability filter for the Navigation system chart. Sourced from
    /// `[navigation_console.system_chart] selects`.
    #[serde(default)]
    pub nav_chart_selects: Vec<String>,
    /// Detection range for the Navigation system chart, in world units.
    /// Sourced from `[navigation_console.system_chart] range` in the ship
    /// TOML.
    #[serde(default = "default_nav_chart_range")]
    pub nav_chart_range: f32,
}

fn default_tactical_radar_range() -> f32 {
    300.0
}

fn default_nav_chart_range() -> f32 {
    500.0
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
            sensors_radar_selects: Vec::new(),
            nav_chart_shows: Vec::new(),
            nav_chart_selects: Vec::new(),
            nav_chart_range: default_nav_chart_range(),
            tactical_radar_shows: Vec::new(),
            tactical_radar_range: default_tactical_radar_range(),
            tactical_radar_selects: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
/// Minimal per-tick world-entity registry broadcast. All per-system ship state
/// has migrated to `SystemBlackboard` (issue #570); only world entity snapshots
/// remain here so the client can track NPC/asteroid positions and hull.
pub struct SimSnapshot {
    /// Per-tick entity state snapshots (position, yaw, hull, flags).
    #[serde(default)]
    pub entity_states: Vec<EntityStateSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WaypointSnapshot {
    pub x: f32,
    pub z: f32,
    /// When `Some`, the waypoint is anchored to the named entity's UUID and
    /// the server rewrites `x`/`z` from the entity's live transform every
    /// tick. When the parent entity despawns, the navigation waypoint is
    /// auto-cleared. When `None`, the waypoint is a free position placed by
    /// tap-to-place and never moves on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
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
    /// Single-facing NPC shield fraction (#471). `Some(current/max)` for
    /// entities with an `EntityShield` component, `None` otherwise. A
    /// broken shield reads as `Some(0.0)` (the `EntityShield::fraction`
    /// helper clamps broken to zero so the bar visibly empties without
    /// a separate "broken" wire field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield_fraction: Option<f32>,
    /// Inner radius for ring-shaped entities (e.g. asteroid fields).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_radius: Option<f32>,
    /// Seconds remaining until the entity completes warp-out (set while in `warping_out` state).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp_out_remaining_secs: Option<f32>,
    /// Optional per-entity world-space size override for the radar icon
    /// blip. When `None`, clients fall back to `radius`. Authors set this in
    /// the entity TOML's `[radar_appearance]` table to fudge radar icon
    /// size independently of the entity's actual physical size. Does not
    /// affect region rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar_size: Option<f32>,
    /// Half-extents for Box-shaped region entities. `[x, y, z]` in world units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_extents: Option<[f32; 3]>,
    /// Point-blip icon name, taken verbatim from the entity TOML's
    /// `[radar_appearance].icon`. Free-form string resolved by naming
    /// convention on the client. `None` means this entity has no point
    /// icon (it may still be a region via `region_colour`, or invisible to
    /// radar entirely if both are absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar_icon: Option<String>,
    /// Area-fill colour for region/field entities, taken verbatim from
    /// `[radar_appearance].region_colour`. `None` means this entity has no
    /// region representation on radar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_colour: Option<[f32; 3]>,
    /// Set to `true` when this entity is referenced by an active mission
    /// objective. The client radar renders a visual indicator for these entities.
    #[serde(default)]
    pub objective_target: bool,
    /// Targetability tags from the entity's `[target]` section.
    /// Empty when the entity has no `[target]` section (not targetable).
    #[serde(default)]
    pub target_tags: Vec<String>,
    /// Cosmetic threat level string: `"none"`, `"low"`, `"medium"`, or `"high"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat_level: Option<String>,
    /// Short description from the entity's `[target]` section.
    /// Falls back to the entity `name` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_description: Option<String>,
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
            shield_fraction: None,
            inner_radius: None,
            warp_out_remaining_secs: None,
            radar_size: None,
            half_extents: None,
            radar_icon: Some("asteroid".into()),
            region_colour: None,
            objective_target: false,
            target_tags: Vec::new(),
            threat_level: None,
            target_description: None,
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
    /// Single-facing NPC shield fraction (#471). Present for entities with
    /// an `EntityShield` component; mirrors `EntitySnapshot.shield_fraction`
    /// for live-tick updates so the Sensors panel can re-render the shield
    /// bar each frame without re-receiving the full snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield_fraction: Option<f32>,
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

/// Static world data sent once per game (on phase transition to InProgress) and replayed
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

/// Destination for repair dispatch in the station/system architecture.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data")]
pub enum RepairTarget {
    Station(StationId),
    Core,
}

/// Typed payload sent to a specific ship system through
/// `ClientMessage::ControlSystem`.
///
/// This is additive scaffolding for ADR-0002. Existing runtime handlers still
/// consume the legacy console-addressed variants until the station/system
/// migration lands.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum SystemControlPayload {
    ToggleRedAlert,
    HelmInput {
        thrust: f32,
        steering: f32,
    },
    StartImpulseCharge,
    CancelImpulse,
    ToggleBoost,
    SetBoost {
        active: bool,
    },
    SetView {
        mode: ViewMode,
    },
    SetTarget {
        uuid: String,
    },
    FirePhaser,
    SetPhaserMode {
        mode: PhaserMode,
    },
    SetPhaserFrequency {
        frequency: f32,
    },
    FireTorpedo {
        target_uuid: Option<String>,
    },
    LoadTube,
    UnloadTube,
    DispatchRepairTeam {
        team_idx: u8,
        target: RepairTarget,
    },
    SetPowerGroupAllocation {
        group: PowerGroupId,
        level: u8,
    },
    SetPower {
        target: Console,
        level: u8,
    },
    Hail {
        target_uuid: String,
    },
    SelectCommsMessage {
        message_id: String,
    },
    RespondToMessage {
        message_id: String,
        response_index: usize,
    },
    ClearComms,
    ShowOnScreen {
        message_id: String,
    },
    SetShieldFocus {
        facing: Option<ViewDirection>,
    },
    SetNavigationWaypoint {
        x: f32,
        z: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_uuid: Option<String>,
    },
    ClearNavigationWaypoint,
    SetScienceTarget {
        uuid: String,
    },
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
    /// Per-player ready toggle — the sole game start mechanism.
    /// When all joined players are ready the game auto-starts.
    SetReady {
        ready: bool,
    },
    FirePhaser {
        bank: PhaserBank,
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
    /// Manually start loading a torpedo into the specified tube.
    LoadTube {
        tube: TorpedoTube,
    },
    /// Manually unload (or cancel loading of) the specified tube.
    UnloadTube {
        tube: TorpedoTube,
    },
    /// Set the phaser emitter frequency (0.0–1.0). Sent by the Tactical holder.
    SetPhaserFrequency {
        frequency: f32,
    },
    /// Future station/system architecture control envelope. Targets one
    /// ship-local system instance by stable `SystemId` and carries a typed
    /// payload for that system kind. Existing runtime handlers ignore this
    /// additive variant until the migration lands.
    ControlSystem {
        target: SystemId,
        payload: SystemControlPayload,
    },
    /// Change the active rating for the sender's station. The rating name
    /// must match one of the station's defined ratings, or be "Backfill"
    /// (which automates every system owned by the station). When the rating
    /// is not found the message is silently ignored.
    /// Validated server-side: sender must hold a station with that rating.
    SetStationRating {
        rating_name: String,
    },
    /// Channel-3 coordination envelope. Carries a typed coordination payload
    /// to be queued with lag and routed at delivery time (issue #494).
    SendCoordination {
        target: SystemId,
        payload: CoordinationPayload,
    },
}

/// Typed payload for a channel-3 coordination message (issue #494).
///
/// These are always lagged and routed through the coordination bus — they
/// never produce immediate authoritative effects.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum CoordinationPayload {
    /// Advisory text message shown to the operator.
    Advisory { message: String },
    /// Alert-level coordination (e.g. AI warns human of a threat).
    Alert { title: String, body: String },
    /// Sensors advises Tactical of the target's shield frequency.
    FrequencyHint { frequency: f32 },
    /// Sent to Helm when a shield facing goes offline; fires once per offline cycle.
    ShieldFacingDown {
        label: String,
        offline_remaining: f32,
    },
    /// Sent to Helm when a shield facing recovers to `restored_notify_pct` of max HP;
    /// only fires on red alert, only after the facing has been down this cycle.
    ShieldFacingRestored { label: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
#[allow(clippy::large_enum_variant)]
pub enum ServerMessage {
    Welcome {
        state: GameState,
        ship_stations: ShipStations,
        ship_config: ShipClientConfig,
        /// Per-station active ratings so clients can render AUTO/read-only
        /// badges immediately on (re)connect without waiting for the first
        /// `RatingChanged` or `SimState`.
        #[serde(default)]
        station_ratings: HashMap<StationId, String>,
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
        /// C1: stable station ID carried alongside the legacy name string.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        station_id: Option<StationId>,
    },
    ReadyChanged {
        token: String,
        ready: bool,
    },
    NameChanged {
        token: String,
        name: String,
    },
    GameStarted,
    /// Broadcast during the `Loading` phase at ~10 Hz. Clients show a progress
    /// bar until `GameStarted` arrives, which transitions the phase to `InProgress`.
    ///
    /// `fraction` is `0.0` (nothing loaded) to `1.0` (all assets ready).
    LoadingProgress {
        fraction: f32,
    },
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
    /// Sent to the Tactical console holder when the Sensors operator designates
    /// a suggested target. Advisory only — does not lock the Tactical console.
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
        /// From the rock's own TOML `[radar_appearance].icon`. `None` (e.g.
        /// cosmetic asteroid variants with no `[radar_appearance]`) means
        /// this rock never appears on radar.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radar_icon: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radar_colour: Option<[f32; 3]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radar_size: Option<f32>,
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
    /// Broadcast to all when a station's active rating changes.
    /// Clients use this to update AUTO/read-only badges for system fragments
    /// belonging to the affected station.
    RatingChanged {
        station_id: StationId,
        rating_name: String,
    },
    /// Sent once at game start and whenever per-console hull HP changes.
    /// Replaces the `console_hull` field that was previously embedded in every `SimState`.
    ConsoleHullUpdate {
        entries: Vec<ConsoleHullStatus>,
    },
    /// Broadcast when the ship takes damage (from collision or damage zone).
    /// `shield` = HP absorbed by shields, `hull` = HP that reached the hull.
    /// Either field may be zero (e.g. shield-only hit has `hull: 0.0`).
    DamageTaken {
        hull: f32,
        shield: f32,
    },
    /// Channel-3 coordination popup delivered to a specific player (issue #494).
    /// Sent to the holder of the target system's console. Carries the typed
    /// coordination payload and the originating sender info.
    CoordinationPopup {
        target: SystemId,
        payload: CoordinationPayload,
        /// Human-readable label for the origin (e.g. "AI Tactical", "Captain").
        #[serde(default)]
        sender_label: String,
    },
    /// Dirty-tracked per-system blackboard sync (issue #557, Channel 1).
    ///
    /// Emitted only for systems whose blackboard changed since the last send.
    /// `updates` is a list of `(SystemId, SystemBlackboard)` pairs.
    BlackboardUpdate {
        updates: Vec<(SystemId, SystemBlackboard)>,
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
/// Produced server-side by `publish_weapons_blackboard` from live ECS
/// transforms joined with the static world entity registry for tags/radius.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RadarBlip {
    /// Stable entity UUID — matches `EntitySnapshot::uuid`. Used to correlate
    /// with `WeaponsBlackboard::target_uuid` for lock highlight.
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
    /// Whether this blip can be selected/targeted on the radar.
    /// Set by the server based on the radar's `selects` filter vs the entity's
    /// `[target].tags`.
    #[serde(default)]
    pub selectable: bool,
    /// Cosmetic threat level string: `"none"`, `"low"`, `"medium"`, or `"high"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat_level: Option<String>,
    /// Short description from the entity's `[target]` section.
    /// Falls back to the entity `name` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Targetability tags from the entity's `[target]` section.
    #[serde(default)]
    pub target_tags: Vec<String>,
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

/// Raw sim truth for the Captain system, published each tick into the ship
/// blackboard (issue #563).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CaptainBlackboard {
    /// Whether the ship is at red alert.
    pub red_alert: bool,
    /// Stable system id for the Red Alert coarse system fragment.
    #[serde(default = "default_red_alert_system_id")]
    pub red_alert_system_id: SystemId,
    /// True when Red Alert is AI-controlled.
    #[serde(default)]
    pub red_alert_auto: bool,
    /// Stable system id for the Viewscreen coarse system.
    #[serde(default = "default_viewscreen_system_id")]
    pub viewscreen_system_id: SystemId,
    /// True when the Viewscreen system is AI-controlled.
    #[serde(default)]
    pub viewscreen_auto: bool,
    /// Current camera direction: `"Fore"`, `"Port"`, `"Starboard"`, `"Aft"`,
    /// or `""` for non-camera views.
    pub view_direction: String,
    /// Full current view mode (tagged enum). Supersedes the removed
    /// `SimSnapshot.view_mode` field (issue #570) so clients can derive
    /// `state.currentView` from the blackboard alone.
    #[serde(default)]
    pub view_mode: ViewMode,
    /// Mission objectives. Updated when `ObjectiveManager` is dirty.
    #[serde(default)]
    pub objectives: Vec<ObjectiveSnapshot>,
    /// Overall ship hull integrity as a percentage (0–100).
    pub hull_integrity_pct: f32,
    /// Computed game status string shown in the captain panel.
    #[serde(default)]
    pub game_status: String,
}

fn default_red_alert_system_id() -> SystemId {
    crate::system_registry::red_alert_system_id()
}

fn default_viewscreen_system_id() -> SystemId {
    crate::system_registry::viewscreen_system_id()
}

impl Default for CaptainBlackboard {
    fn default() -> Self {
        Self {
            red_alert: false,
            red_alert_system_id: default_red_alert_system_id(),
            red_alert_auto: false,
            viewscreen_system_id: default_viewscreen_system_id(),
            viewscreen_auto: false,
            view_direction: "Fore".into(),
            view_mode: ViewMode::Camera(ViewDirection::Fore),
            objectives: Vec::new(),
            hull_integrity_pct: 100.0,
            game_status: String::new(),
        }
    }
}

/// Raw sim truth for the Helm system, published each tick into the ship
/// blackboard (issue #557). GUI derivation (heading strings, radar blips)
/// happens client-side in `gui/console-state.js`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HelmBlackboard {
    pub yaw: f32,
    pub forward_speed: f32,
    pub x: f32,
    pub z: f32,
    /// Impulse drive charge progress (0.0 = idle, 1.0 = fully engaged).
    pub impulse_charge: f32,
    /// Boost battery charge fraction (0.0 empty → 1.0 full).
    pub boost_battery: f32,
    pub boost_active: bool,
    /// True when this ship's TOML includes a boost drive config.
    pub boost_enabled: bool,
}

/// Raw sim truth for the Weapons (Tactical) system, published each tick into
/// the ship blackboard (issue #560).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WeaponsBlackboard {
    pub target_uuid: Option<String>,
    pub target_name: Option<String>,
    pub banks: Vec<PhaserBankState>,
    pub tubes: Vec<TorpedoTubeState>,
    pub torpedo_count: u32,
    pub phaser_mode: PhaserMode,
    /// Static phaser bank arc geometry (from ship config). Included so JS can
    /// draw arc overlays without a separate config request.
    #[serde(default)]
    pub phaser_arcs: Vec<PhaserBankClientConfig>,
    /// Static torpedo tube arc geometry (from ship config).
    #[serde(default)]
    pub torpedo_arcs: Vec<TorpedoTubeClientConfig>,
    /// Radar blips projected into normalised ship-relative coordinates.
    #[serde(default)]
    pub blips: Vec<RadarBlip>,
    /// World region overlays (static shapes drawn on the radar canvas).
    #[serde(default)]
    pub regions: Vec<RadarRegion>,
}

/// Per-system blackboard published each tick. One typed variant per system
/// kind, mirroring the `SystemControlPayload` design. Wire-serialised as a
/// tagged enum so the JS mirror can switch on `kind`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data")]
pub enum SystemBlackboard {
    Helm(HelmBlackboard),
    Weapons(WeaponsBlackboard),
    Power(PowerBlackboard),
    Shields(ShieldsBlackboard),
    Captain(CaptainBlackboard),
    Repair(RepairBlackboard),
    Comms(CommsBlackboard),
    Sensors(SensorsBlackboard),
    Navigation(NavigationBlackboard),
    Viewscreen(ViewscreenBlackboard),
}

/// Raw sim truth for the Power system, published each tick into the ship
/// blackboard (issue #561).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PowerBlackboard {
    /// Per-console power allocation entries (data-driven from ship config).
    pub consoles: Vec<PowerConsoleEntry>,
    /// Sum of current allocations across all powered consoles.
    pub total: u8,
    /// Maximum total allocation (pool cap).
    pub total_max: u8,
    /// Current battery charge (0 – `battery_max`).
    pub battery_charge: f32,
    /// Maximum battery capacity.
    pub battery_max: f32,
    /// Whether the power system is locked (battery exhausted).
    pub locked: bool,
}

/// An authority-checked intra-system command produced by `admit_system_commands`.
///
/// The source identity is stripped at admission; `response_token` carries the
/// originating client's token purely for routing replies (not for behavioral
/// branching).
#[derive(Clone, Debug)]
pub struct AdmittedCommand {
    pub target: SystemId,
    pub payload: SystemControlPayload,
    /// Token used to address a reply back to the originating client.
    /// Handlers must not branch on this for any behavioral decision.
    pub response_token: Option<String>,
}

/// Cleared and refilled each tick by `admit_system_commands` (runs before
/// `SimSet::Input`). Handlers read from this instead of `InboundMessage`.
#[derive(bevy::prelude::Resource, Default)]
pub struct AdmittedCommands(pub Vec<AdmittedCommand>);

impl AdmittedCommands {
    /// Iterate admitted commands targeting the given system ID string.
    pub fn for_target<'a>(&'a self, target: &'a str) -> impl Iterator<Item = &'a AdmittedCommand> {
        self.0.iter().filter(move |c| c.target.0.as_str() == target)
    }
}

// ── Inter-system command channel (issue #559) ─────────────────────────────────

/// Payloads that one system may send to another within the same Simulate tick.
///
/// Inter-system commands originate inside Simulate and are applied immediately
/// (same-tick) by the target system. They are invariant-gated: valid by
/// construction, not by control-state check. The sender mutates only its own
/// state; the target mutates only its own.
#[derive(Clone, Debug)]
pub enum InterSystemPayload {
    /// The Weapons system is drawing energy from the Power battery while a
    /// phaser beam is active. Applied once per tick during `SimSet::Physics`;
    /// consumed by the Power system during `SimSet::Modifiers`.
    DrainWeaponsBattery { amount: f32 },
}

/// An inter-system command: one system commanding another to mutate its own
/// state this tick. See [`InterSystemPayload`] for invariants.
#[derive(Clone, Debug)]
pub struct InterSystemMsg {
    pub target: SystemId,
    pub payload: InterSystemPayload,
}

/// Cleared at the start of each Simulate phase (before `SimSet::Input`) and
/// filled during Simulate by systems that need to mutate a peer system's state.
/// Handlers read from this without authority checks — valid by construction.
#[derive(bevy::prelude::Resource, Default)]
pub struct InterSystemQueue(pub Vec<InterSystemMsg>);

impl InterSystemQueue {
    /// Iterate messages targeting the given system ID string.
    pub fn for_target<'a>(&'a self, target: &'a str) -> impl Iterator<Item = &'a InterSystemMsg> {
        self.0.iter().filter(move |m| m.target.0.as_str() == target)
    }
}

/// Raw sim truth for the Shields system, published each tick into the ship
/// blackboard (issue #562).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ShieldsBlackboard {
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

/// A single entry in [`PowerBlackboard::consoles`].
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

/// Raw sim truth for the Repair system, published each tick into the ship
/// blackboard (issue #564).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RepairBlackboard {
    /// Current team slot states (one entry per repair team).
    pub teams: Vec<TeamSlot>,
    /// Per-console hull status. Drives the hull bar and team-destination labels.
    pub console_hull: Vec<ConsoleHullStatus>,
    /// Travel duration in seconds (from ship TOML `[repair]` block).
    pub travel_duration_secs: f32,
    /// Consoles that can be targeted for repair dispatch (in display order).
    pub damageable_consoles: Vec<Console>,
}

/// Raw sim truth for the Comms system, published each tick into the ship
/// blackboard (issue #565).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CommsBlackboard {
    /// Current inbox messages for the Comms holder, in insertion order.
    pub messages: Vec<CommsMessage>,
    /// Mission objectives visible to Comms.
    #[serde(default)]
    pub objectives: Vec<ObjectiveSnapshot>,
    /// Hailable contacts derived from the active world content.
    pub contacts: Vec<CommsContact>,
}

/// Ship-wide aggregate blackboard written by the Viewscreen phase-1b aggregator
/// (issue #568). Reads all per-system phase-1a blackboards + world registry.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ViewscreenBlackboard {
    /// Whether the ship is currently in red alert.
    pub red_alert: bool,
    /// Overall ship hull integrity as a percentage (0–100).
    pub hull_integrity_pct: f32,
    /// Elapsed-seconds timestamp when the ship last took hull damage, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_damage_taken_secs: Option<f32>,
    /// Elapsed-seconds timestamp when a weapon was last fired, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_weapon_fired_secs: Option<f32>,
}

/// Raw sim truth for the Sensors system, published each tick into the ship
/// blackboard (issue #566).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SensorsBlackboard {
    /// Detection range for the long-range radar widget, in world units.
    #[serde(default = "default_sensors_radar_range")]
    pub radar_range: f32,
    /// Tag filter: only entities whose tags overlap this list are displayed.
    #[serde(default)]
    pub radar_shows: Vec<String>,
    /// Targetability filter: only these entities are selectable on the radar.
    #[serde(default)]
    pub radar_selects: Vec<String>,
}

impl Default for SensorsBlackboard {
    fn default() -> Self {
        Self {
            radar_range: default_sensors_radar_range(),
            radar_shows: Vec::new(),
            radar_selects: Vec::new(),
        }
    }
}

/// Raw sim truth for the Navigation system, published each tick into the ship
/// blackboard (issue #567).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NavigationBlackboard {
    /// Detection range for the navigation system chart, in world units.
    #[serde(default = "default_nav_chart_range")]
    pub nav_chart_range: f32,
    /// Entity-type filter for the navigation chart.
    #[serde(default)]
    pub nav_chart_shows: Vec<String>,
    /// Targetability filter for the navigation chart.
    #[serde(default)]
    pub nav_chart_selects: Vec<String>,
    /// Current shared navigation waypoint (supersedes SimSnapshot.navigation_waypoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_waypoint: Option<WaypointSnapshot>,
}

impl Default for NavigationBlackboard {
    fn default() -> Self {
        Self {
            nav_chart_range: default_nav_chart_range(),
            nav_chart_shows: Vec::new(),
            nav_chart_selects: Vec::new(),
            navigation_waypoint: None,
        }
    }
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
    LoadTube {
        tube: String,
    },
    UnloadTube {
        tube: String,
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
    /// Toggle the boost drive on/off (helm BOOST button).
    ToggleBoost,
    /// Explicitly set boost on or off — used for hold-to-boost (pointerdown/pointerup).
    SetBoost {
        active: bool,
    },
    /// Switch the viewscreen to radar mode (helm ON SCREEN button).
    SetRadarView,
    /// Set power to an explicit level for the named powered console.
    ///
    /// The HTML power panel sends
    /// `{ action: "set_power", console: "Power", target: "helm", level: 3 }`.
    SetPower {
        target: Console,
        level: u8,
    },
    /// Dispatch a repair team to a console (repair console).
    ///
    /// The HTML repair panel sends
    /// `{ action: "dispatch_repair_team", console: "Repair", team_idx: 0, target: "Helm" }`.
    DispatchRepairTeam {
        team_idx: u8,
        target: Console,
    },
    /// Hail a contact (comms console).
    ///
    /// The HTML comms panel sends `{ action: "hail", console: "Comms", target_uuid: "..." }`.
    Hail {
        target_uuid: String,
    },
    /// Select (open) a message in the Comms inbox.
    ///
    /// The HTML comms panel sends `{ action: "select_comms_message", console: "Comms", message_id: "..." }`.
    SelectCommsMessage {
        message_id: String,
    },
    /// Choose a response to a received comms message.
    ///
    /// The HTML comms panel sends
    /// `{ action: "respond_to_message", console: "Comms", message_id: "...", response_index: 0 }`.
    RespondToMessage {
        message_id: String,
        response_index: usize,
    },
    /// Clear all read/orphaned messages from the Comms inbox.
    ///
    /// The HTML comms panel sends `{ action: "clear_comms", console: "Comms" }`.
    ClearComms,
    /// Push the selected comms message to the viewscreen.
    ///
    /// The HTML comms panel sends `{ action: "show_on_screen", console: "Comms", message_id: "..." }`.
    ShowOnScreen {
        message_id: String,
    },
    /// Switch the viewscreen to navigation chart mode (navigation console).
    ///
    /// The HTML navigation panel sends `{ action: "set_navigation_chart", console: "Navigation" }`.
    SetNavigationChart,
    /// Set the shared Navigation waypoint.
    ///
    /// The HTML navigation panel sends
    /// `{ action: "set_navigation_waypoint", console: "Navigation", x: 120.0, z: -45.0 }`
    /// for tap-to-place, or
    /// `{ action: "set_navigation_waypoint", console: "Navigation", x: 120.0, z: -45.0, source_uuid: "..." }`
    /// when anchoring to the currently-selected entity.
    SetNavigationWaypoint {
        x: f32,
        z: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_uuid: Option<String>,
    },
    /// Clear the shared Navigation waypoint.
    ///
    /// The HTML navigation panel sends `{ action: "clear_navigation_waypoint", console: "Navigation" }`.
    ClearNavigationWaypoint,
}

/// Maps a decoded [`UiAction`] to the existing [`ClientMessage`] the server
/// handlers already process. Pure — covered by a native unit test.
pub fn ui_action_to_client_message(a: &UiAction) -> ClientMessage {
    match a {
        UiAction::FireTorpedo { tube, target_uuid } => ClientMessage::FireTorpedo {
            tube: tube.clone(),
            target_uuid: target_uuid.clone(),
        },
        UiAction::LoadTube { tube } => ClientMessage::LoadTube { tube: tube.clone() },
        UiAction::UnloadTube { tube } => ClientMessage::UnloadTube { tube: tube.clone() },
        UiAction::FirePhaser { bank } => ClientMessage::FirePhaser { bank: bank.clone() },
        UiAction::ToggleRedAlert => ClientMessage::ControlSystem {
            target: crate::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::ToggleRedAlert,
        },
        UiAction::SetView { direction } => ClientMessage::ControlSystem {
            target: crate::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Camera(direction.clone()),
            },
        },
        UiAction::HelmInput { thrust, steering } => ClientMessage::ControlSystem {
            target: crate::system_registry::helm_system_id(),
            payload: SystemControlPayload::HelmInput {
                thrust: *thrust,
                steering: *steering,
            },
        },
        UiAction::StartImpulseCharge => ClientMessage::ControlSystem {
            target: crate::system_registry::helm_system_id(),
            payload: SystemControlPayload::StartImpulseCharge,
        },
        UiAction::CancelImpulse => ClientMessage::ControlSystem {
            target: crate::system_registry::helm_system_id(),
            payload: SystemControlPayload::CancelImpulse,
        },
        UiAction::ToggleBoost => ClientMessage::ControlSystem {
            target: crate::system_registry::helm_system_id(),
            payload: SystemControlPayload::ToggleBoost,
        },
        UiAction::SetBoost { active } => ClientMessage::ControlSystem {
            target: crate::system_registry::helm_system_id(),
            payload: SystemControlPayload::SetBoost { active: *active },
        },
        UiAction::SetRadarView => ClientMessage::ControlSystem {
            target: crate::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Radar,
            },
        },
        UiAction::SetPower { target, level } => ClientMessage::ControlSystem {
            target: crate::system_registry::power_system_id(),
            payload: SystemControlPayload::SetPowerGroupAllocation {
                group: crate::power_system::power_group_for_console(target)
                    .unwrap_or_else(|| PowerGroupId(target.station_console_id().into())),
                level: *level,
            },
        },
        UiAction::DispatchRepairTeam { team_idx, target } => ClientMessage::DispatchRepairTeam {
            team_idx: *team_idx,
            console: target.clone(),
        },
        UiAction::Hail { target_uuid } => ClientMessage::ControlSystem {
            target: crate::system_registry::comms_system_id(),
            payload: SystemControlPayload::Hail {
                target_uuid: target_uuid.clone(),
            },
        },
        UiAction::SelectCommsMessage { message_id } => ClientMessage::ControlSystem {
            target: crate::system_registry::comms_system_id(),
            payload: SystemControlPayload::SelectCommsMessage {
                message_id: message_id.clone(),
            },
        },
        UiAction::RespondToMessage {
            message_id,
            response_index,
        } => ClientMessage::ControlSystem {
            target: crate::system_registry::comms_system_id(),
            payload: SystemControlPayload::RespondToMessage {
                message_id: message_id.clone(),
                response_index: *response_index,
            },
        },
        UiAction::ClearComms => ClientMessage::ControlSystem {
            target: crate::system_registry::comms_system_id(),
            payload: SystemControlPayload::ClearComms,
        },
        UiAction::ShowOnScreen { message_id } => ClientMessage::ControlSystem {
            target: crate::system_registry::comms_system_id(),
            payload: SystemControlPayload::ShowOnScreen {
                message_id: message_id.clone(),
            },
        },
        UiAction::SetNavigationChart => ClientMessage::ControlSystem {
            target: crate::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::NavigationChart,
            },
        },
        UiAction::SetNavigationWaypoint { x, z, source_uuid } => ClientMessage::ControlSystem {
            target: crate::system_registry::navigation_system_id(),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: *x,
                z: *z,
                source_uuid: source_uuid.clone(),
            },
        },
        UiAction::ClearNavigationWaypoint => ClientMessage::ControlSystem {
            target: crate::system_registry::navigation_system_id(),
            payload: SystemControlPayload::ClearNavigationWaypoint,
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
        let action = UiAction::FirePhaser {
            bank: "port".into(),
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::FirePhaser {
                bank: "port".into()
            }
        );
    }

    #[test]
    fn toggle_red_alert_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::ToggleRedAlert),
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            }
        );
    }

    #[test]
    fn set_view_direction_maps_to_client_message() {
        let action = UiAction::SetView {
            direction: ViewDirection::Aft,
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(ViewDirection::Aft)
                }
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::HelmInput {
                    thrust: 0.75,
                    steering: -0.5
                }
            }
        );
    }

    #[test]
    fn start_impulse_charge_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::StartImpulseCharge),
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            }
        );
    }

    #[test]
    fn cancel_impulse_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::CancelImpulse),
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::CancelImpulse,
            }
        );
    }

    #[test]
    fn toggle_boost_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::ToggleBoost),
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            }
        );
    }

    #[test]
    fn set_boost_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::SetBoost { active: true }),
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::SetBoost { active: true },
            }
        );
        assert_eq!(
            ui_action_to_client_message(&UiAction::SetBoost { active: false }),
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::SetBoost { active: false },
            }
        );
    }

    #[test]
    fn set_radar_view_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::SetRadarView),
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar
                }
            }
        );
    }

    #[test]
    fn set_power_maps_to_control_system_message() {
        let action = UiAction::SetPower {
            target: Console::Helm,
            level: 3,
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_system_id(),
                payload: SystemControlPayload::SetPowerGroupAllocation {
                    group: PowerGroupId("helm".into()),
                    level: 3,
                }
            }
        );
    }

    #[test]
    fn dispatch_repair_team_maps_to_client_message() {
        let action = UiAction::DispatchRepairTeam {
            team_idx: 1,
            target: Console::Power,
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::DispatchRepairTeam {
                team_idx: 1,
                console: Console::Power
            }
        );
    }

    #[test]
    fn set_navigation_waypoint_maps_to_client_message() {
        let action = UiAction::SetNavigationWaypoint {
            x: 12.5,
            z: -8.0,
            source_uuid: None,
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::ControlSystem {
                target: SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 12.5,
                    z: -8.0,
                    source_uuid: None,
                },
            }
        );
    }

    #[test]
    fn set_navigation_waypoint_with_source_uuid_maps_to_client_message() {
        let action = UiAction::SetNavigationWaypoint {
            x: 12.5,
            z: -8.0,
            source_uuid: Some("abc-123".into()),
        };
        assert_eq!(
            ui_action_to_client_message(&action),
            ClientMessage::ControlSystem {
                target: SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 12.5,
                    z: -8.0,
                    source_uuid: Some("abc-123".into()),
                },
            }
        );
    }

    #[test]
    fn clear_navigation_waypoint_maps_to_client_message() {
        assert_eq!(
            ui_action_to_client_message(&UiAction::ClearNavigationWaypoint),
            ClientMessage::ControlSystem {
                target: SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            }
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
    /// True when every connected player is ready (replaces all_stations_filled
    /// as the launch gate in the per-player Ready flow).
    #[serde(default)]
    pub all_ready: bool,
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
