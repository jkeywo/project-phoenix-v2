use crate::damage::DamageTier;
pub use crate::entity_tags::EntityTag;
use crate::stations_config::ShipStations;
use bevy::prelude::States;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Typed OR-aggregated boolean ship flags (formerly `core/flag_kind.rs`,
/// inlined here — it is a wire type like everything else in this module).
/// Set by modifiers (e.g. region effects) keyed by source; a flag reads true
/// while any source holds it. Serde round-trip pinned in `core/codec.rs`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlagKind {
    CommsJammed,
    SensorBlind,
}

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
    /// Helm console's short-range radar detection range (dedicated slot —
    /// distinct from the tactical/weapons `RadarRange` slot so damaging one
    /// radar system doesn't bleed into another).
    HelmRadarRange,
    /// Sensors console's long-range radar detection range (dedicated slot —
    /// see `HelmRadarRange`).
    SensorRadarRange,
}

/// Who or what applied a modifier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModifierSource {
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
    /// A modifier attributed to a power group (issue #617). The
    /// SystemId-keyed successor of the retired `Console` variant.
    PowerGroup(PowerGroupId),
    /// A modifier derived from a damaged/disabled/destroyed system's
    /// `debuff_magnitude` (e.g. a radar system's detection range shrinking
    /// as it takes damage). Keyed by the system's own `SystemId` so each
    /// damaged system's contribution can be independently added/removed.
    SystemDamage(SystemId),
}

impl Eq for ModifierSource {}

impl std::hash::Hash for ModifierSource {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
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
            ModifierSource::PowerGroup(g) => {
                4u8.hash(state);
                g.hash(state);
            }
            ModifierSource::SystemDamage(sid) => {
                5u8.hash(state);
                sid.hash(state);
            }
        }
    }
}

/// Per-system hull integrity snapshot broadcast in `SimSnapshot` — the
/// SystemId-keyed hull status type (parent issue #516, sub-issue #616).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SystemHullStatus {
    /// Stable, ship-wide system identifier (e.g. `"helm"`, `"phaser-fore"`).
    pub system_id: SystemId,
    /// Human-readable name for UI display (e.g. `"Helm"`, `"Phaser Bank (Fore)"`).
    pub display_name: String,
    pub current: f32,
    pub max_hp: f32,
    /// Derived damage tier for this system.
    pub tier: crate::damage::DamageTier,
    /// Active debuff magnitude for this system (0.0 when Operational or
    /// Destroyed, tier_config.debuff_magnitude when Damaged or Disabled).
    #[serde(default)]
    pub debuff_magnitude: f32,
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
    /// Arc centre bearing in degrees (0 = fore, 90 = starboard, 180 = aft, 270 = port).
    /// Present so the JS panel can draw arbitrary-width / arbitrary-count arcs
    /// without needing separate config. Defaults to 0 for wire compatibility
    /// with pre-#514 payloads.
    #[serde(default)]
    pub center_deg: f32,
    /// Angular width of the arc in degrees. Defaults to 90 for wire
    /// compatibility with pre-#514 payloads (four evenly-spaced facings).
    #[serde(default = "default_arc_width_deg")]
    pub width_deg: f32,
    /// Stable arc id from the ship TOML `[[shield_arc]]` block (e.g. `"fore"`,
    /// `"all"`). Used to correlate the aggregate facings list with the
    /// per-arc fine blackboards under `SystemId("shield-arc-<id>")`.
    /// Defaults to empty for wire compatibility with pre-#514 payloads.
    #[serde(default)]
    pub arc_id: String,
    /// Hit-routing priority. Higher value wins when multiple arcs cover the same bearing.
    /// Defaults to 1 for wire compatibility with older payloads.
    #[serde(default = "default_priority")]
    pub priority: u32,
}

fn default_priority() -> u32 {
    1
}

fn default_arc_width_deg() -> f32 {
    90.0
}

fn default_visual_scale() -> f32 {
    1.0
}

/// String identifier for a phaser bank, matching the `id` field of the
/// `[[weapons_console.phaser_banks]]` array in the ship entity TOML (e.g.
/// `"port"`, `"starboard"`). Used in `FirePhaser`, `PhaserFired`,
/// `PhaserBankState`, and `PhaserBankClientConfig`.
pub type PhaserBank = String;

/// String identifier for a torpedo tube, matching the `id` field of the
/// `[[torpedoes.tubes]]` array in the ship entity TOML (e.g. `"fore_port"`,
/// `"aft"`). Used in `FireTorpedo`, `TorpedoLaunched`, `TorpedoTubeState`,
/// and `TorpedoTubeClientConfig`.
pub type TorpedoTube = String;

/// String identifier for a blaster bank, matching the `id` field of the
/// `[[weapons_console.blaster_banks]]` array in the ship TOML (e.g. `"fore"`,
/// `"aft"`). Used in `BlasterBankState` and `BlasterBankClientConfig`.
pub type BlasterBank = String;

/// How an outbound `ServerMessage` should be delivered over the wire.
///
/// `Reliable` rides the ordered/retransmit DataChannel (PeerJS default).
/// `Snapshot` rides the unordered/no-retransmit DataChannel when available,
/// falling back to the reliable channel when the snapshot channel has not
/// opened yet or has failed. The server decides the delivery class; clients
/// just obey — this value is a routing hint passed through the JS bridge,
/// not serialised as part of the wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryClass {
    Reliable,
    Snapshot,
}

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
    /// True when the tube has at least one torpedo loaded and ready to fire.
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
    /// Maximum number of torpedoes this tube can hold (from TOML `volley_max`).
    #[serde(default = "default_tube_volley_max_wire")]
    pub volley_max: u32,
    /// Number of torpedoes currently loaded and ready to fire.
    #[serde(default)]
    pub loaded_count: u32,
    /// Desired number of loaded torpedoes (0..=volley_max).
    #[serde(default)]
    pub target_count: u32,
    /// Fraction `[0.0, 1.0]` of the in-progress load/unload operation for the
    /// next torpedo. 0.0 when idle.
    #[serde(default)]
    pub load_progress: f32,
}

fn default_tube_volley_max_wire() -> u32 {
    1
}

/// Per-bank blaster state broadcast to the Tactical operator as part of
/// `WeaponsUpdate` (issue #631).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BlasterBankState {
    pub id: String,
    /// True when the bank is ready to accept a new fire/charge command.
    pub fire_ready: bool,
    /// True while the bank is in its post-volley cooldown.
    pub on_cooldown: bool,
    /// Seconds remaining on the cooldown timer (0.0 when ready).
    pub cooldown_remaining: f32,
    /// Projectiles remaining in the current volley (0 when idle).
    pub pending_volley: u32,
    /// Charge phase completion fraction `[0.0, 1.0]` (issue #636).
    /// Always `0.0` for instant-fire banks (`charge_time_secs == 0`).
    #[serde(default)]
    pub charge_progress: f32,
    /// True when this bank requires a charge phase before firing
    /// (`charge_time_secs > 0` in TOML). The client uses this to switch
    /// the fire button to hold-to-fire mode.
    #[serde(default)]
    pub has_charge: bool,
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

/// Static, per-bank configuration sent to clients in `Welcome` so the
/// Tactical UI can render blaster fire arcs on radar.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BlasterBankClientConfig {
    pub id: BlasterBank,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    #[serde(default)]
    pub cooldown_secs: f32,
}

/// Firing mode for phaser banks. Matches `phaser::PhaserMode`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PhaserMode {
    #[default]
    Auto,
    Manual,
}

/// The state of a single repair team, broadcast as part of `RepairState`.
///
/// SystemId-keyed after issue #619 — the legacy `console` / `queued` fields
/// were retired along with the `Console` enum. Every non-Idle variant carries
/// `system_id` + `display_name`; `Returning` additionally carries
/// `queued_system_id` + `queued_display_name` for the auto-dispatch target.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum TeamSlot {
    #[default]
    Idle,
    /// Team is en route to the target system. `elapsed` counts up toward the
    /// configured travel duration.
    Travelling {
        #[serde(default)]
        system_id: Option<SystemId>,
        #[serde(default)]
        display_name: Option<String>,
        elapsed: f32,
        /// On-site repair priority (0 = default, higher = preferred). Only
        /// meaningful when the team is `Repairing`; set via
        /// `SetRepairPriority` command.
        #[serde(default)]
        priority: Option<u8>,
    },
    /// Team is at the system performing repairs.
    Repairing {
        #[serde(default)]
        system_id: Option<SystemId>,
        #[serde(default)]
        display_name: Option<String>,
        /// On-site repair priority (0 = default, higher = preferred). Set
        /// via `SetRepairPriority` command while the team is on site.
        #[serde(default)]
        priority: Option<u8>,
    },
    /// Team has finished and is returning to engineering.
    /// `remaining` counts down from the travel duration.
    /// `queued_system_id` holds the next system to dispatch to
    /// automatically on arrival (if any).
    Returning {
        remaining: f32,
        /// System id we are returning FROM (populated when known).
        #[serde(default)]
        system_id: Option<SystemId>,
        /// Display name for the system we are returning FROM.
        #[serde(default)]
        display_name: Option<String>,
        /// System id of the queued next target.
        #[serde(default)]
        queued_system_id: Option<SystemId>,
        /// Display name of the queued next target.
        #[serde(default)]
        queued_display_name: Option<String>,
    },
}

/// A named camera viewpoint defined by a marker in the ship's model rig.
///
/// Marker names should start with `camera_` to be shown in the captain UI
/// (e.g. `camera_fore`, `camera_port`, `camera_aft`, `camera_starboard`).
///
/// Serialises as a plain string (`#[serde(transparent)]`) — wire-compatible
/// with the old `ViewDirection` string serialization.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct CameraView {
    pub marker_name: String,
}

impl Default for CameraView {
    fn default() -> Self {
        Self {
            marker_name: "camera_fore".into(),
        }
    }
}

impl CameraView {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            marker_name: name.into(),
        }
    }
}

/// What is currently shown on the viewscreen.
///
/// `Camera(view)` is the default exterior view positioned at the named
/// model-rig marker; `Radar` is the top-down tactical view requested by the
/// helm.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data")]
pub enum ViewMode {
    Camera(CameraView),
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
    /// Cinematic camera: dynamic above-and-behind view that tracks nearby
    /// entities, with configurable offset, pitch, and target hysteresis.
    /// Selected via the synthetic "cinematic" camera button.
    Cinematic,
}

impl Default for ViewMode {
    fn default() -> Self {
        ViewMode::Camera(CameraView::default())
    }
}

#[cfg(test)]
mod view_mode_tests {
    use super::*;

    #[test]
    fn default_view_mode_is_camera() {
        assert_eq!(ViewMode::default(), ViewMode::Camera(CameraView::default()));
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
    /// Static world data — `Some` only after game start has populated the
    /// world; `None` while in Lobby or before world initialisation.
    #[serde(default)]
    pub world: Option<WorldData>,
}

/// Static, per-ship configuration sent to clients in `Welcome`.
///
/// Carries the bits of the ship entity TOML that the client UI
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
    /// Blaster banks defined on the ship, in TOML order. Used by the Tactical
    /// UI to render fire-arc overlays on radar and per-bank cooldown bars.
    #[serde(default)]
    pub blaster_banks: Vec<BlasterBankClientConfig>,
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
    /// Ship class identifier (e.g. "battleship"). Sourced from
    /// top-level `class` in the ship TOML.
    #[serde(default)]
    pub class: Option<String>,
    /// Unique hull identifier/registry number. Sourced from
    /// top-level `hull_id` in the ship TOML.
    #[serde(default)]
    pub hull_id: Option<String>,
    /// Authored power rating. Sourced from top-level `power_rating`
    /// in the ship TOML.
    #[serde(default)]
    pub power_rating: Option<i32>,
    /// Per-ship CSS theme URL. Sourced from top-level `css` in the
    /// ship TOML.
    #[serde(default)]
    pub ship_css: Option<String>,
    /// Map from station id string to the list of system id strings that
    /// belong to that station. Populated from `ShipConfig::systems_for_station`
    /// and sent on `Welcome` so the client can aggregate per-station hull
    /// without knowing the ship layout. Uses `#[serde(default)]` for backward
    /// compatibility with older server builds that don't send this field.
    #[serde(default)]
    pub station_systems: HashMap<String, Vec<String>>,
    /// Minimum relative bearing change (radians) for Sensors to re-emit a
    /// `ThreatBearing` coordination message to Shields. Sourced from
    /// `[sensors_console] threat_bearing_epsilon_rad` in the ship TOML.
    /// A change smaller than this is considered unchanged and won't re-trigger.
    #[serde(default = "default_threat_bearing_epsilon_rad")]
    pub threat_bearing_epsilon_rad: f32,
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

pub fn default_sensors_radar_range() -> f32 {
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

fn default_threat_bearing_epsilon_rad() -> f32 {
    0.175
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
            blaster_banks: Vec::new(),
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
            class: None,
            hull_id: None,
            power_rating: None,
            ship_css: None,
            station_systems: HashMap::new(),
            threat_bearing_epsilon_rad: default_threat_bearing_epsilon_rad(),
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
    /// Shield fraction for this entity. `Some(current/max)` for entities with
    /// a `ShipShields` component, `None` otherwise. An offline shield reads as
    /// `Some(0.0)` (all facings offline; the bar visually empties without
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
    /// Shield fraction for this entity. Present for entities with
    /// a `ShipShields` component; mirrors `EntitySnapshot.shield_fraction`
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
/// This is the primary control envelope of the station/system architecture
/// (ADR-0002). A handful of weapons messages (`FirePhaser`, `FireTorpedo`,
/// `LoadTube`, `UnloadTube`) also survive as legacy top-level
/// `ClientMessage` variants that runtime handlers still consume.
/// (`SetPhaserFrequency`'s legacy top-level variant was deleted by #804 —
/// the envelope form targeting `phaser-control` is the only wire path.)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum SystemControlPayload {
    ToggleRedAlert,
    /// Set the throttle axis. Targets `helm-thrust` (issue #801).
    SetThrust {
        value: f32,
    },
    /// Set the yaw axis. Targets `helm-steering` (issue #801).
    SetSteering {
        value: f32,
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
    /// Fire the blaster bank addressed by the `ControlSystem` target SystemId
    /// (issue #631). No fields — the target encodes the bank identity.
    FireBlaster,
    /// Begin the charge phase for a hold-to-fire blaster bank (issue #636).
    ///
    /// When `charge_time_secs == 0` on the target bank this behaves
    /// identically to `FireBlaster` (instant-fire — no delay). When
    /// `charge_time_secs > 0` the bank enters a charge phase and the volley
    /// fires automatically when the charge completes.
    ChargeBlasterStart,
    /// Cancel an in-progress charge phase (issue #636).
    ///
    /// Resets charge progress to 0 with no cooldown and no ammo consumed.
    /// Safe to send even when the bank is not currently charging (no-op).
    ChargeBlasterCancel,
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
    SetShieldArcFocus {
        /// True when this arc becomes the focused facing (bonus + penalty
        /// to the other arcs); false to clear focus. Each button press
        /// targets a specific `shield-arc-<id>` SystemId and sends the
        /// desired new focus state for that arc.
        focused: bool,
    },
    SetNavigationWaypoint {
        x: f32,
        z: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_uuid: Option<String>,
    },
    ClearNavigationWaypoint,
    LateralThrustInput {
        lateral: f32,
    },
    SetScienceTarget {
        uuid: String,
    },
    /// Deselect the Sensors science target (issue #828). Today only the
    /// Sensors AI emits this (its decide loop clears the selection when no
    /// in-range contact remains); the console UI has no deselect control yet,
    /// but the payload is origin-agnostic like every admitted command.
    ClearScienceTarget,
    /// Captain boosts (or toggles off) the priority of a doctrine objective.
    /// Sending the same `id` twice toggles the boost off.
    SetObjectivePriority {
        id: String,
    },
    /// Set the volley target count for the torpedo tube addressed by the
    /// `ControlSystem` target SystemId (issue #632). `count` is clamped to
    /// `[0, tube.volley_max]` server-side.
    SetTorpedoVolleyTarget {
        count: u32,
    },
    /// Set the on-site repair priority for a specific repair team (issue #739).
    /// Only takes effect when the team is in `Repairing` state. `priority`
    /// is a `u8` interpreted as higher = more urgent; the host validates
    /// through normal admission and the repair AI ignores it.
    SetRepairPriority {
        team_idx: u8,
        priority: u8,
    },
}

/// `ClientMessageDiscriminants` (from `strum::EnumDiscriminants`) is a
/// fieldless companion enum that automatically stays in sync with the
/// variant list below — used by the codec's table-driven round-trip harness
/// (issue #610) to enforce that every variant has a sample row.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, strum::EnumDiscriminants)]
#[strum_discriminants(name(ClientMessageDiscriminants), derive(Hash, strum::EnumIter))]
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
    /// Primary station/system architecture control envelope. Targets one
    /// ship-local system instance by stable `SystemId` and carries a typed
    /// payload for that system kind. Runtime handlers across every console
    /// consume this variant (issue #846: all weapons fire/load commands
    /// are now `ControlSystem` messages).
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
    /// Sent from the GameOver screen to return everyone to the Lobby for
    /// another round. Only honoured while `GamePhase::GameOver` is active;
    /// ignored otherwise. Any connected player may trigger it — the phone
    /// client's game-over overlay (`client.html`) sends it as well as the
    /// host page, so it is deliberately NOT gated to the host token.
    ReturnToLobby,
    /// Sent from the server UI when the host selects a scenario after
    /// returning to scenario selection from GameOver. Tells the server
    /// to finalize the selection and broadcast lobby state to clients.
    /// Only the host's scenario panel sends this, so the handler accepts it
    /// solely from `console_bridge::LOCAL_CONSOLE_TOKEN` (issue #822).
    ConfirmScenario,
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
    /// Sensors designates a suggested target for Tactical to lock onto.
    /// Routed via `route_coordination` like any other channel-3 payload:
    /// AI Tactical consumes it silently, human Tactical gets a popup
    /// (issue #676 — replaces the old direct `SensorsTargetSuggestion`).
    TargetDesignation { uuid: String, label: String },
    /// Weapons asks Helm to yaw so the phaser firing arc bears on `uuid`.
    /// AI Helm folds this into its steering; human Helm gets a popup
    /// ("Tactical: come about, bring phasers to bear") via `route_coordination`
    /// (issue #677).
    ArcBearingRequest { uuid: String, label: String },
    /// Power system reports a brownout (demand exceeds supply) for a group
    /// that is actively drawing power it cannot get (issue #678).
    /// Fire-once-debounced; only fires when the affected system has level > 1
    /// (not idle at minimum draw) while total allocation > 6 (battery draining).
    PowerBrownout {
        /// Which power group (e.g. "weapons", "helm", "sensors").
        group: String,
        /// Human-readable label for the affected system (e.g. "WEAPONS").
        label: String,
        /// Current allocated level (what the system is actually getting).
        allocated_level: u8,
    },
    /// Navigation clears Helm to follow the ship's current `NavigationWaypoint`
    /// (issues #681, #702).
    ///
    /// Carries the waypoint's `generation`, not a position. The waypoint is the
    /// goal and lives on the ship as one per-entity component that both
    /// consoles read; duplicating its coordinates onto the wire is what created
    /// the `AiMemory.nav_goal` split brain this replaced. All this message does
    /// is say *which* waypoint the Helm is now cleared to fly to, which — once
    /// it has survived the Channel-3 delivery lag — is the whole of the lag's
    /// job. See [`NavigationWaypoint::generation`].
    ///
    /// [`NavigationWaypoint::generation`]: crate::navigation_plugin::NavigationWaypoint::generation
    NavigateTo { generation: u64, label: String },
    /// A system has crossed to a worse damage tier and needs repair (issue #682).
    ///
    /// `deficit` is the exact HP shortfall and is therefore gated by the #737
    /// visibility boundary: it is `Some` on the host-internal enqueue (the AI
    /// repair queue sorts by it) but `None` on the `CoordinationPopup` copy
    /// whenever the recipient is not entitled to exact detail for `system_id`
    /// — i.e. a non-Core system with no repair team on site. A `None` deficit
    /// is the coarse "needs attention" signal: the tier still crosses, the
    /// number does not.
    ///
    /// `system_id` is the system that crossed the tier. `station_id` is the
    /// bucket that owns it (`"core"` when ownerless). Both are carried because
    /// the visibility gate is per-system while the repair queue dedupes per
    /// station; deriving one from `sender_label` would let the two drift.
    RepairRequest {
        system_id: SystemId,
        station_id: String,
        station_label: String,
        tier: DamageTier,
        deficit: Option<f32>,
    },
    /// Sensors warns Shields of an incoming threat (hostile closing or torpedo).
    ThreatBearing { bearing_rad: f32, label: String },
}

/// `ServerMessageDiscriminants` (from `strum::EnumDiscriminants`) is a
/// fieldless companion enum that automatically stays in sync with the
/// variant list below — used by the codec's table-driven round-trip harness
/// (issue #610) to enforce that every variant has a sample row.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, strum::EnumDiscriminants)]
#[strum_discriminants(name(ServerMessageDiscriminants), derive(Hash, strum::EnumIter))]
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
    /// Broadcast when all players are ready: starts a 5-second server-authoritative
    /// countdown before `GameStarted` is emitted. `remaining_secs` counts down from
    /// 5 to 1, then 0 signals cancellation (someone unreadied or a new player joined).
    GameStartCountdown {
        remaining_secs: u32,
    },
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
        /// Per-bank blaster state (issue #631). Empty when no blaster banks declared.
        #[serde(default)]
        blasters: Vec<BlasterBankState>,
        /// Current phaser frequency (0.0–1.0) from ShipPhaserFrequency.
        #[serde(default = "default_shield_frequency")]
        phaser_frequency: f32,
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
        /// Current shield generator frequency (0.0–1.0).
        #[serde(default)]
        frequency: f32,
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
    /// Broadcast to all when a blaster projectile is launched (issue #631).
    ///
    /// `bank` is the TOML bank id (e.g. `"fore"`); `source_uuid` is the firing
    /// entity's UUID; `x`/`z` is the launch position; `heading` is the initial
    /// travel direction in radians.
    BlasterFired {
        bank: String,
        source_uuid: String,
        projectile_id: String,
        x: f32,
        z: f32,
        heading: f32,
        /// Visual scale hint for the client renderer (issue #638).
        /// Small values (≤ 1.0) render a short bolt; large values render a sphere.
        /// Defaults to 1.0 when absent (old wire format compatibility).
        #[serde(default = "default_visual_scale")]
        visual_scale: f32,
    },
    /// Broadcast to all when a blaster projectile hits a target (issue #631).
    BlasterHit {
        bank: String,
        projectile_id: String,
        target_uuid: String,
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
    /// Broadcast when all players return to the lobby from the GameOver screen.
    /// Clients should switch back to the lobby panel.
    ReturnedToLobby,
    /// Broadcast when the host selects a scenario from the scenario selection
    /// screen. Clients should transition from waiting to the lobby view.
    ScenarioLoaded,
    /// Broadcast to all when a station's active rating changes.
    /// Clients use this to update AUTO/read-only badges for system fragments
    /// belonging to the affected station.
    RatingChanged {
        station_id: StationId,
        rating_name: String,
    },
    /// Sent once at game start and whenever the recipient's *visible* per-system
    /// hull detail changes.
    ///
    /// `entries` is a **per-recipient projection** (issue #737), not the whole
    /// ship: a station holder sees exact detail only for the systems its own
    /// station owns, and the Engineering holder additionally sees ownerless
    /// "core" systems plus any system a repair team is currently on site at.
    /// `aggregate_fraction` is the authoritative ship-wide hull fraction
    /// (0.0–1.0) across *every* damageable system — it is the only whole-ship
    /// figure a recipient may show, because `entries` can no longer be summed
    /// to derive one. `None` only on legacy/unprojected payloads.
    SystemHullUpdate {
        entries: Vec<SystemHullStatus>,
        #[serde(default)]
        aggregate_fraction: Option<f32>,
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
    /// AI-to-AI coordination chatter displayed on the viewscreen.
    /// Emitted when an AI-controlled system sends a level-3 coordination
    /// message to another AI-controlled system. Broadcast to the viewscreen
    /// only (not forwarded to phone clients).
    AiChatter {
        /// Human-readable label of the sending system (e.g. "Shields", "Sensors").
        from_label: String,
        /// Human-readable label of the target system (e.g. "Helm", "Weapons").
        to_label: String,
        /// Concise message body derived from the original CoordinationPayload.
        text: String,
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
    /// Current engine thrust fraction (0.0 = idle, 1.0 = full).
    /// Drives the engine hum volume on the host page.
    #[serde(default)]
    pub engine_thrust: f32,
    /// True while the local ship has an active phaser beam. Drives the looping
    /// phaser SFX on the host page. A bool rather than a level because this
    /// struct is change-detected — see `recompute_hud_state`.
    #[serde(default)]
    pub phaser_firing: bool,
    /// Set when the game has ended. "Ship Destroyed" for hull death; the
    /// scenario `game_over` message otherwise. `None` while in progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_over_message: Option<String>,
}

/// A single radar blip on the Tactical console radar.
///
/// Positions are normalised to `[-1.0, 1.0]` where ±1.0 = the effective
/// tactical radar range (base `tactical_radar_range` × `RadarRange` modifier).
/// Produced server-side by `publish_weapons_core_blackboard` from live ECS
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

/// Raw sim truth for the Tactical Radar system, published each tick into the
/// ship blackboard (issue #829).
///
/// The tactical radar owns the ship's **Combat Lock** — its `selected_target`
/// is the authoritative target selection that used to live on the retired
/// `TacticalRadarSelection` component. Blips and region overlays moved here out of
/// `WeaponsBlackboard`. The viewscreen aggregator lifts `selected_target` into
/// `ViewscreenBlackboard::combat_lock`, and every cross-system consumer reads
/// that frozen viewscreen fact rather than this live selection (spec §1/§3).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TacticalRadarBlackboard {
    /// The Combat Lock: the tactical radar's currently selected target UUID,
    /// or `None`. Mirrors this ship's `TacticalRadarSelection` component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_target: Option<String>,
    /// Radar blips projected into normalised ship-relative coordinates.
    /// Populated for the local ship only (NPCs render no radar).
    #[serde(default)]
    pub blips: Vec<RadarBlip>,
    /// World region overlays (static shapes drawn on the radar canvas).
    #[serde(default)]
    pub regions: Vec<RadarRegion>,
}

/// Raw sim truth for the Sensor Radar system, published each tick into the ship
/// blackboard (issue #829).
///
/// The sensor radar owns the ship's **Science Target** — its `selected_target`
/// mirrors the retired `SensorRadarSelection` component. The viewscreen aggregator
/// lifts it into `ViewscreenBlackboard::science_target`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SensorRadarBlackboard {
    /// The Science Target: the sensor radar's currently selected target UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_target: Option<String>,
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
    /// Current camera marker name (e.g. `"camera_fore"`), or `""` for
    /// non-camera views.
    pub view_direction: String,
    /// Full current view mode (tagged enum). Supersedes the removed
    /// `SimSnapshot.view_mode` field (issue #570) so clients can derive
    /// `state.currentView` from the blackboard alone.
    #[serde(default)]
    pub view_mode: ViewMode,
    /// Available camera marker names for the captain to choose from.
    /// Populated from the local ship's `ModelMarkers` component.
    #[serde(default)]
    pub camera_views: Vec<String>,
    /// Mission objectives. Updated when `ObjectiveManager` is dirty.
    #[serde(default)]
    pub objectives: Vec<ObjectiveSnapshot>,
    /// Overall ship hull integrity as a percentage (0–100).
    pub hull_integrity_pct: f32,
    /// Computed game status string shown in the captain panel.
    #[serde(default)]
    pub game_status: String,
    /// The objective id the captain has chosen to prioritize, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boosted_objective_id: Option<String>,
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
            view_direction: String::new(),
            view_mode: ViewMode::Camera(CameraView::default()),
            camera_views: Vec::new(),
            objectives: Vec::new(),
            hull_integrity_pct: 100.0,
            game_status: String::new(),
            boosted_objective_id: None,
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
    /// Live detection range for the helm radar widget, in world units —
    /// the configured `helm_radar_range` scaled by the `helm-radar` system's
    /// current damage tier (shrinks when Damaged/Disabled, near-zero when
    /// Destroyed). `0.0` (the derived `Default`) means "no live value yet";
    /// callers should fall back to the static `ShipClientConfig` range.
    #[serde(default)]
    pub radar_range: f32,
    /// Current lateral (sideways) speed. Positive = starboard (+X), negative = port (-X).
    pub lateral_speed: f32,
}

/// Raw sim truth for the Helm Lateral Thrust fine system,
/// published each tick into the ship blackboard.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HelmLateralThrustBlackboard {
    /// Current lateral thrust input fraction (-1.0 .. 1.0).
    pub lateral_input: f32,
    /// Whether the lateral thrust system is operational (not disabled or destroyed).
    pub is_online: bool,
    /// Whether the lateral thrust system is under AI control.
    #[serde(default)]
    pub auto: bool,
}

/// Raw sim truth for a single Helm Engine fine system (port or starboard),
/// published each tick into the ship blackboard (issue #511).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HelmEngineBlackboard {
    /// Current thrust fraction applied by this engine (0.0..=1.0).
    /// Zero when the engine is offline (damaged/destroyed).
    pub thrust_fraction: f32,
    /// True when the engine is operational (not disabled or destroyed).
    pub is_online: bool,
}

/// Raw sim truth for a single Phaser Bank fine system, published each tick
/// into the ship blackboard (issue #512).
///
/// This is the per-instance state that the coarse `WeaponsBlackboard` also
/// aggregates in its `banks` field; the per-bank blackboard is emitted so
/// individual system consumers (e.g. bank-level AI) can gate on their own
/// bank without unpacking the whole weapons blackboard.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PhaserBankBlackboard {
    /// True when the bank is operational (not disabled or destroyed by hull damage).
    pub is_online: bool,
    /// True while the bank is in its post-shot cooldown.
    pub on_cooldown: bool,
    /// Seconds remaining on the cooldown timer (0.0 when ready).
    pub cooldown_remaining: f32,
    /// True when the bank can fire this tick (target in arc, off cooldown, online).
    pub fire_ready: bool,
}

/// Raw sim truth for a single Torpedo Tube fine system, published each tick
/// into the ship blackboard (issue #512).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TorpedoTubeBlackboard {
    /// True when the tube is operational (not disabled or destroyed by hull damage).
    pub is_online: bool,
    /// True when the tube has at least one torpedo loaded and ready to fire.
    pub loaded: bool,
    /// Load state label: "loaded" | "unloaded" | "loading" | "unloading".
    pub state: String,
    /// Completion fraction `[0.0, 1.0]` for the current load/unload operation.
    pub progress: f32,
    /// Tube-specific load/unload duration in seconds.
    pub load_time: f32,
    /// Maximum number of torpedoes this tube can hold (from TOML `volley_max`).
    #[serde(default = "default_tube_volley_max_wire")]
    pub volley_max: u32,
    /// Number of torpedoes currently loaded and ready to fire.
    #[serde(default)]
    pub loaded_count: u32,
    /// Desired number of loaded torpedoes (0..=volley_max).
    #[serde(default)]
    pub target_count: u32,
    /// Fraction `[0.0, 1.0]` of the in-progress load operation for the next torpedo.
    #[serde(default)]
    pub load_progress: f32,
}

/// Raw sim truth for the shared Torpedo Magazine fine system, published each
/// tick into the ship blackboard (issue #512).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TorpedoMagazineBlackboard {
    /// True when the magazine is operational. When `false`, tube-load claims
    /// are refused (see [`InterSystemPayload::ClaimTorpedoRound`]) and the
    /// fire path is also blocked so loaded tubes cannot launch.
    pub is_online: bool,
    /// Remaining torpedoes in the shared magazine.
    pub torpedoes_remaining: u32,
    /// Maximum magazine capacity (from ship TOML `[torpedoes] count`).
    pub capacity: u32,
}

/// Raw sim truth for the Power Reactor fine system, published each tick into
/// the ship blackboard (issue #513).
///
/// The reactor owns the allocation surface — the current pool total and cap
/// live here. `is_online: false` reflects a Disabled/Destroyed reactor whose
/// allocation input is refused via the standard `accept_human_input` gate.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PowerReactorBlackboard {
    /// Sum of current per-group allocations (mirrors `PowerBlackboard::total`).
    pub total_allocation: u8,
    /// Maximum total allocation the pool can carry.
    pub max_allocation: u8,
    /// True when the reactor is operational (not disabled or destroyed).
    /// When `false`, `SetPowerGroupAllocation` messages are
    /// refused at admission.
    pub is_online: bool,
    /// True when the power system is in the locked (battery-exhausted) state.
    /// Mirrors `PowerBlackboard::locked` for reactor-scoped readers.
    pub locked: bool,
}

/// Raw sim truth for the Power Battery fine system, published each tick into
/// the ship blackboard (issue #513).
///
/// The battery is the target for channel-2 drain messages (e.g. active
/// phaser beams via `InterSystemPayload::DrainWeaponsBattery`). When
/// `is_online: false` the battery refuses drains — the emergency reserve
/// pool is effectively 0 and downstream consumers cannot pull from it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PowerBatteryBlackboard {
    /// Current battery charge (0.0 – `capacity`).
    pub charge: f32,
    /// Maximum battery capacity (from ship TOML `[power] capacity`).
    pub capacity: f32,
    /// True when the battery is operational (not disabled or destroyed).
    /// When `false`, channel-2 drain messages are refused.
    pub is_online: bool,
    /// Emergency-reserve threshold expressed as a fraction of `capacity`
    /// (0.0 – 1.0). Sourced from ship TOML `[power] emergency_threshold`
    /// divided by capacity; the panel can highlight the bar when charge
    /// drops below this line.
    pub emergency_threshold: f32,
}

/// Raw sim truth for a single Shield Arc fine system, published each tick
/// into the ship blackboard (issue #514).
///
/// One entry per arc under `SystemId("shield-arc-<arc_id>")`. The aggregate
/// `ShieldsBlackboard` continues to be published under `SystemId("shields")`
/// for legacy JS readers; per-arc AI or per-arc UI consumers use these
/// fine blackboards instead of unpacking the aggregate facings vec.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ShieldArcBlackboard {
    /// Human-readable arc label (e.g. `"Fore"`, `"All"`).
    pub label: String,
    /// Current HP for this arc.
    pub hp: i32,
    /// Effective max HP (after focus bonus/penalty).
    pub max_hp: i32,
    /// True when this arc is operational — derived from
    /// `ShipSystemControlSources.offline_systems` on this ship (i.e. hull
    /// damage on the arc's console entry has not pushed it into the
    /// Disabled/Destroyed tier) AND the arc's HP-timer is not currently
    /// offline. Matches the derivation pattern used by
    /// `PowerReactorBlackboard.is_online` / `PhaserBankBlackboard.is_online`.
    pub is_online: bool,
    /// True when this arc is the currently focused facing.
    pub is_focused: bool,
    /// Seconds remaining on the shield-HP offline timer (0.0 when online).
    /// Distinct from `is_online == false` due to hull damage: an arc can
    /// be shield-online (HP > 0, this field is 0) yet hull-offline (its
    /// `SystemId` is in `offline_systems`).
    pub offline_remaining: f32,
    /// Arc centre bearing in degrees.
    pub center_deg: f32,
    /// Arc angular width in degrees.
    pub width_deg: f32,
}

/// Raw sim truth for the Weapons (Tactical) system, published each tick into
/// the ship blackboard (issue #560).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WeaponsBlackboard {
    /// The ship's **Combat Lock**, read from its own frozen
    /// `ViewscreenBlackboard::combat_lock` and filtered for liveness.
    ///
    /// Not a live read of the `TacticalRadarSelection` component: Weapons is a
    /// cross-system consumer of the tactical radar's selection, so it goes
    /// through the viewscreen aggregate like every other consumer (spec §3,
    /// issue #829). Published in `SimSet::Publish` while the aggregator runs in
    /// `SimSet::PublishAggregate`, so this is last tick's lock — the one-tick
    /// lag at 30Hz that spec §1 accepts.
    pub target_uuid: Option<String>,
    /// The Tactical AI's *selected* target — the output of
    /// `ai_target_selection` (issues #697, #700).
    ///
    /// Distinct from `target_uuid`, the ship's applied Combat Lock (set by
    /// whoever last wrote it — human `SetTarget`, the Tactical AI, or the beam
    /// / torpedo paths). `locked_target` is *intent*, `target_uuid` is *truth*.
    ///
    /// **The two are deliberately not collapsed into one field** even though
    /// they agree on an AI-operated ship: on a human-operated Tactical
    /// `locked_target` is `None` while `target_uuid` carries the human's lock,
    /// and telling those two cases apart on the wire is this field's entire
    /// job. Pinned by
    /// `human_tactical_leaves_locked_target_empty_and_keeps_the_human_lock`.
    ///
    /// - Tactical AI-operated: `ai_target_selection` publishes `locked_target`
    ///   and applies the same choice to `TacticalRadarSelection`, so once that
    ///   selection has been through the viewscreen aggregator (one tick) the
    ///   two agree.
    /// - Tactical human-operated: the AI selects nothing, so `locked_target`
    ///   is `None` while `target_uuid` may be set by the human's lock.
    ///
    /// Only `ai_target_selection` writes this field, and nothing on the server
    /// reads it back — it is reported, not consumed. Its job is to make the
    /// AI's reasoning observable and to tell an AI-driven lock apart from a
    /// human's on the wire. `publish_weapons_core_blackboard`
    /// carries the value forward when it rebuilds the blackboard (it runs in
    /// `SimSet::Publish`, after the AI wrote its intent in `SimSet::Input`),
    /// dropping it if the selected entity is no longer live — the beam and
    /// torpedo paths can kill the target after `SimSet::Input`, and publishing
    /// a dead selection would break the "the two agree" guarantee above.
    #[serde(default)]
    pub locked_target: Option<String>,
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
    /// Blaster bank state (issue #631). Empty when the ship has no blaster banks.
    #[serde(default)]
    pub blasters: Vec<BlasterBankState>,
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
    /// Per-engine fine-system blackboard (issue #511). One entry per engine instance.
    HelmEngine(HelmEngineBlackboard),
    /// Per-bank fine-system blackboard (issue #512). One entry per phaser bank instance.
    PhaserBank(PhaserBankBlackboard),
    /// Per-tube fine-system blackboard (issue #512). One entry per torpedo tube instance.
    TorpedoTube(TorpedoTubeBlackboard),
    /// Shared torpedo magazine blackboard (issue #512). One entry per ship.
    TorpedoMagazine(TorpedoMagazineBlackboard),
    /// Power Reactor fine-system blackboard (issue #513). One entry per ship.
    PowerReactor(PowerReactorBlackboard),
    /// Power Battery fine-system blackboard (issue #513). One entry per ship.
    PowerBattery(PowerBatteryBlackboard),
    /// Per-arc fine-system blackboard (issue #514). One entry per shield arc
    /// instance under `SystemId("shield-arc-<arc_id>")`. Coexists with the
    /// aggregate `Shields` blackboard under `SystemId("shields")`.
    ShieldArc(ShieldArcBlackboard),
    /// Helm Lateral Thrust fine-system blackboard.
    HelmLateralThrust(HelmLateralThrustBlackboard),
    /// Tactical radar blackboard (issue #829). One per ship carrying the
    /// Combat Lock + tactical blips/regions, keyed by `tactical_radar_system_id`.
    TacticalRadar(TacticalRadarBlackboard),
    /// Sensor radar blackboard (issue #829). Carries the Science Target,
    /// keyed by `sensor_radar_system_id`.
    SensorRadar(SensorRadarBlackboard),
}

/// Raw sim truth for the Power system, published each tick into the ship
/// blackboard (issue #561).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PowerBlackboard {
    /// Per-power-group allocation entries, keyed on `PowerGroupId` (data-driven
    /// from ship config). `#[serde(default)]` lets pre-#616 payloads (which
    /// carried a `consoles` field instead) round-trip cleanly — the missing
    /// `groups` field decodes to an empty vec.
    #[serde(default)]
    pub groups: Vec<PowerGroupEntry>,
    /// Sum of current allocations across all power groups.
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
///
/// Pure per-ship Component post ship-parity audit; the legacy `Resource`
/// derive has been dropped since no production code reads a global
/// `Res<AdmittedCommands>`.
#[derive(bevy::prelude::Component, Default)]
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
    /// Joystick input published by the Helm Joystick fine system (issue #511)
    /// for consumption by each Helm Engine fine system. Channels thrust and
    /// steering so each engine can independently gate on its own online state.
    JoystickState { thrust: f32, steering: f32 },
    /// A torpedo tube is requesting a round from the shared magazine (issue #512).
    ///
    /// Sent by the tube's `handle_load_tube` handler during `SimSet::Input`
    /// and consumed by the magazine handler `handle_torpedo_magazine_inter_system`
    /// during `SimSet::Physics` on the same tick. The magazine consumer:
    ///
    /// 1. Refuses the claim (no-op) if the magazine is offline (Disabled /
    ///    Destroyed hull tier), leaving the tube unloaded.
    /// 2. Refuses the claim if the magazine's `torpedoes_remaining == 0`.
    /// 3. Otherwise decrements the magazine counter and begins loading the
    ///    named tube (via `TorpedoSystem::start_load_reserved`).
    ///
    /// The `tube` field carries the tube's TOML `id` (e.g. `"fore_port"`).
    ClaimTorpedoRound { tube: TorpedoTube },
}

/// An inter-system command: one system commanding another to mutate its own
/// state this tick. See [`InterSystemPayload`] for invariants.
///
/// `source_entity` identifies which ship the message applies to so
/// per-entity handlers (e.g. `handle_power_inter_system`) can route the
/// mutation to the correct ship's per-entity state. `None` means "target
/// the LocalShip" — used by legacy paths and tests that never spawned a
/// specific ship.
#[derive(Clone, Debug)]
pub struct InterSystemMsg {
    pub target: SystemId,
    pub payload: InterSystemPayload,
    pub source_entity: Option<bevy::prelude::Entity>,
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
    /// Current shield generator frequency (0.0–1.0).
    #[serde(default = "default_shield_frequency")]
    pub frequency: f32,
}

fn default_shield_frequency() -> f32 {
    0.5
}

/// A single entry in [`PowerBlackboard::groups`], one per `PowerGroupId`
/// registered on the ship.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PowerGroupEntry {
    /// Power group identifier — the `PowerGroupId` string (e.g. `"helm"`).
    pub id: String,
    /// Display label shown in the HTML panel (e.g. `"HELM"`, `"WEAPONS"`).
    pub label: String,
    /// Current power level (1 – `max_level`).
    pub level: u8,
    /// Maximum power level for this power group.
    pub max_level: u8,
}

/// Preview of a queued repair request for blackboard publication (issue #682).
///
/// Carries exact damage numbers, so it is subject to the same #737 visibility
/// boundary as [`RepairBlackboard::system_hull`]: `station_id` is the bucket
/// the repair queue is keyed by (a station id, or `"core"` for the ownerless
/// bucket) and exists so the projection can decide entitlement from the same
/// rule rather than parsing the display label.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct QueueEntryPreview {
    pub station_id: String,
    pub station_label: String,
    pub tier: DamageTier,
    pub deficit: f32,
}

/// Raw sim truth for the Repair system, published each tick into the ship
/// blackboard (issue #564).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RepairBlackboard {
    /// Current team slot states (one entry per repair team).
    pub teams: Vec<TeamSlot>,
    /// Travel duration in seconds (from ship TOML `[repair]` block).
    pub travel_duration_secs: f32,
    /// Per-system hull status. Drives the hull bar and team-destination labels.
    ///
    /// On the wire this is the Engineering **projection** (issue #737): core
    /// systems, the Engineering station's own systems, and any system a repair
    /// team is currently on site at. The host-internal copy (read by the repair
    /// AI controller) still carries every system.
    #[serde(default)]
    pub system_hull: Vec<SystemHullStatus>,
    /// Systems that can be targeted for repair dispatch (in display order).
    #[serde(default)]
    pub damageable_systems: Vec<SystemId>,
    /// Priority-queue preview entries (worst-first) for human repair UI (issue #682).
    #[serde(default)]
    pub queue_depth: Vec<QueueEntryPreview>,
    /// Authoritative ship-wide hull fraction (0.0–1.0) across every damageable
    /// system (issue #737). Engineering's hero hull bar reads this, because
    /// `system_hull` is a projection and can no longer be summed to a whole-ship
    /// figure. `None` on the host-internal copy before projection.
    #[serde(default)]
    pub aggregate_hull_fraction: Option<f32>,
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
    /// UUID of the last entity that damaged this ship, if any.
    /// Written by the damage-application path when any ship takes damage.
    /// Captain AI reads this to trigger red-alert when under attack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attacker_uuid: Option<String>,
    /// Utility-scored objective pool, computed by the phase-1b aggregator from
    /// the active `ObjectiveManager` + current world conditions (issue #571).
    /// Per-system AI reads this to select the top directive it can serve.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scored_objectives: Vec<ScoredObjective>,
    /// **Combat Lock** — the tactical radar's selected target, lifted from this
    /// ship's `TacticalRadarBlackboard::selected_target` (issue #829). This is
    /// the ship-wide targeting fact every cross-system consumer reads (weapons
    /// firing, helm pursuit, shields bearing, sensors mirror). Frozen: written
    /// in `SimSet::PublishAggregate`, read by consumers next tick's Input/Physics
    /// (one-tick lag at 30Hz accepted, including firing — spec §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combat_lock: Option<String>,
    /// **Science Target** — the sensor radar's selected target, lifted from this
    /// ship's `SensorRadarBlackboard::selected_target` (issue #829).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub science_target: Option<String>,
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
    /// The UUID of the current science target (set by Sensors console). Broadcast
    /// so all radar views can render a blue target marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub science_target_uuid: Option<String>,
}

impl Default for SensorsBlackboard {
    fn default() -> Self {
        Self {
            radar_range: default_sensors_radar_range(),
            radar_shows: Vec::new(),
            radar_selects: Vec::new(),
            science_target_uuid: None,
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
    /// Whether this objective originates from the active mission or from standing doctrine.
    #[serde(default)]
    pub source: ObjectiveSource,
}

/// Mission-altitude directive attached to an objective. Drives per-system AI
/// operate logic to select which directive to act on (issue #571).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(tag = "kind")]
pub enum AiDirective {
    /// No AI directive — objective is human-facing only.
    #[default]
    None,
    /// Destroy the named target entity.
    Destroy { target: String },
    /// Patrol between the listed anchors in order.
    Patrol {
        anchors: Vec<String>,
        loop_path: bool,
    },
    /// Reach the named anchor position.
    Reach { anchor: String },
    /// Hail the named target entity.
    Hail { target: String },
    /// Retreat to the named anchor position.
    Retreat { anchor: String },
}

/// Whether an objective originates from the active mission or from standing doctrine.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ObjectiveSource {
    #[default]
    Mission,
    Doctrine,
}

/// Which player-ship system cares about a given directive kind.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SystemAffinity {
    Helm,
    Weapons,
    Captain,
}

/// An objective with its computed utility score, published on the Viewscreen
/// blackboard each tick so per-system AI can select the best directive to serve.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ScoredObjective {
    /// Stable identifier, matches `ObjectiveSnapshot::id`.
    pub id: String,
    /// Computed utility score (0.0 = gated-out / inactive).
    pub score: f32,
    /// Machine-readable directive for AI systems.
    pub directive: AiDirective,
    /// Whether this came from the mission or from standing doctrine.
    pub source: ObjectiveSource,
    /// Which ship systems consider this directive relevant.
    pub relevance: Vec<SystemAffinity>,
    /// Human-readable snapshot (prose text, status, targets).
    pub snapshot: ObjectiveSnapshot,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading_progress: Option<f32>,
    /// Remaining seconds in the pre-game countdown, or 0 when no countdown is active.
    #[serde(default)]
    pub countdown_secs: u32,
}

/// One station slot in the lobby grid payload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StationPayload {
    pub name: String,
    pub short_code: String,
    pub rank: String,
    pub holder_name: Option<String>,
    pub is_mine: bool,
    pub preset_names: Vec<String>,
}
