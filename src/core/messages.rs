use crate::damage::DamageTier;
pub use crate::entity_tags::EntityTag;
pub use crate::ship::manual::ShipManualWire;
use crate::stations_config::ShipStations;
use bevy::prelude::States;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Typed OR-aggregated boolean ship flags (formerly `core/flag_kind.rs`,
/// inlined here — it is a wire type like everything else in this module).
/// Set by modifiers (e.g. region effects) keyed by source; a flag reads true
/// while any source holds it. Serde round-trip pinned in `core/codec.rs`.
/// `Ord` so a flag can key a `BTreeMap` — `ShipModifiers` stores its flag
/// source-sets in one, for the same reason `SystemId` below carries `Ord`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FlagKind {
    CommsJammed,
    SensorBlind,
}

/// Which ship attribute a modifier affects. Defined here so it can be used in
/// wire messages without creating a circular dependency with `modifiers.rs`.
///
/// `Ord` because `(ModifierSource, ModifierSlot)` keys `ShipModifiers`' table,
/// and that table is a `BTreeMap` — its walk is a float accumulation, so the
/// key order has to be a property of the keys and not of a hash seed. The
/// derived order follows declaration order; nothing depends on which order it
/// is, only that every process agrees on it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    /// Per-second shield regeneration on every arc, driven by the `shields`
    /// power group (issue #952). Scales each `ShieldFacing::regen_per_sec` in
    /// `ship::shields::tick_shields`; the arc's authored rate is the ×1.0 rung,
    /// so a hull that never moves the group regenerates exactly what its
    /// `[[shield_arc]]` blocks say.
    ShieldRegen,
}

/// Who or what applied a modifier.
///
/// `Ord` for the same reason [`ModifierSlot`] carries it: the two together key
/// `ShipModifiers`' `BTreeMap` of bonuses, whose iteration order decides the
/// order f32 bonuses are summed in. Every field the derive compares is
/// authored or minted content (a uuid, a world id/tag, a group or system id),
/// so the resulting order is identical in every process.
#[derive(Clone, Debug, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
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
/// `Ord` so system ids can key a `BTreeMap`. The per-ship blackboard map is one,
/// because its iteration order reaches the wire: a `HashMap` ordered those
/// updates by `RandomState`'s per-process seed, so no two runs of the same
/// seeded binary emitted the same byte stream.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SystemId(pub String);

/// Stable, designer-authored identifier for an operator-facing power group.
/// `Ord` so it can sit inside a `ModifierSource` that keys a `BTreeMap`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PowerGroupId(pub String);

/// The single reason a weapon instance (phaser bank, torpedo tube, blaster
/// bank) cannot fire this tick — the shared blocking-reason vocabulary all
/// three families publish (issue #764).
///
/// `Ready` is the not-blocked state. The remaining variants are the union of
/// every blocking case across the three families; a family that has no concept
/// of a given reason (e.g. phasers have no magazine, so never `NoAmmo`) simply
/// never emits it. The JS panels switch on the serialised variant name to pick
/// the shared display label, so the names are wire-stable.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum WeaponBlockReason {
    /// Not blocked — the weapon can fire this tick.
    #[default]
    Ready,
    /// No target is locked.
    NoTarget,
    /// A target is locked but beyond this weapon's effective range.
    OutOfRange,
    /// A target is locked and in range but outside this weapon's fire arc.
    OutOfArc,
    /// The weapon is in its post-shot / post-volley cooldown.
    Cooldown,
    /// The weapon is loading / charging and cannot fire yet (torpedo tube
    /// mid-load, blaster mid-charge or mid-volley).
    Loading,
    /// The weapon has no ammunition available (empty torpedo magazine).
    NoAmmo,
    /// The weapon is offline — disabled or destroyed by hull damage.
    Offline,
}

/// Shared readiness + blocking contract published by every weapon family so
/// Tactical renders equivalent ready / blocked / unavailable feedback for
/// phasers, blasters, and torpedoes (issue #764).
///
/// `ready == (blocking_reason == WeaponBlockReason::Ready)` is an invariant
/// upheld by [`WeaponReadiness::evaluate`]. `target_range` / `target_arc` carry
/// the authoritative geometry whenever a target is locked, regardless of the
/// blocking reason, so the client can show range/arc telemetry even while
/// blocked (e.g. out of arc but in range).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct WeaponReadiness {
    /// True when the weapon can fire this tick (target in range + arc, off
    /// cooldown, loaded/charged, online).
    pub ready: bool,
    /// Why the weapon cannot fire (`Ready` when it can).
    pub blocking_reason: WeaponBlockReason,
    /// Distance to the locked target in world units, when a target is locked.
    pub target_range: Option<f32>,
    /// Absolute angular offset (degrees) between the target bearing and this
    /// weapon's fire-arc centre, when a target is locked.
    pub target_arc: Option<f32>,
}

/// Authoritative target geometry for one weapon instance, used by
/// [`WeaponReadiness::evaluate`]. Computed by each producer from the frozen
/// combat-lock target position + the ship physics + the weapon's own
/// range/arc config, so the three families share one evaluation path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponTargetGeometry {
    /// Distance to the target in world units.
    pub range: f32,
    /// Absolute angular offset (degrees) between the target bearing and the
    /// weapon's fire-arc centre.
    pub arc_offset_deg: f32,
    /// True when `range` is within the weapon's effective range.
    pub in_range: bool,
    /// True when the target is inside the weapon's fire arc.
    pub in_arc: bool,
}

impl WeaponReadiness {
    /// Resolve the shared readiness contract from a weapon's current state.
    ///
    /// Blocking-reason priority (system state dominates target state): `Offline`
    /// → `Cooldown` → `Loading` → `NoAmmo` → `NoTarget` → `OutOfRange` →
    /// `OutOfArc` → `Ready`. `target_range` / `target_arc` are populated from
    /// `target` whenever one is supplied, even when a higher-priority reason
    /// blocks the shot.
    ///
    /// Family-specific inputs a family does not have are passed as `false`
    /// (phasers/blasters never set `no_ammo`; phasers/torpedoes never set a
    /// charge as `loading` unless mid-load).
    pub fn evaluate(
        online: bool,
        on_cooldown: bool,
        loading: bool,
        no_ammo: bool,
        target: Option<WeaponTargetGeometry>,
    ) -> Self {
        let target_range = target.map(|g| g.range);
        let target_arc = target.map(|g| g.arc_offset_deg);
        let reason = if !online {
            WeaponBlockReason::Offline
        } else if on_cooldown {
            WeaponBlockReason::Cooldown
        } else if loading {
            WeaponBlockReason::Loading
        } else if no_ammo {
            WeaponBlockReason::NoAmmo
        } else {
            match target {
                None => WeaponBlockReason::NoTarget,
                Some(g) if !g.in_range => WeaponBlockReason::OutOfRange,
                Some(g) if !g.in_arc => WeaponBlockReason::OutOfArc,
                Some(_) => WeaponBlockReason::Ready,
            }
        };
        Self {
            ready: reason == WeaponBlockReason::Ready,
            blocking_reason: reason,
            target_range,
            target_arc,
        }
    }
}

/// The weapon family an [`CoordinationPayload::ArcBearingRequest`] is emitted
/// for (issue #767). A structural identity — not player-facing text — used to
/// key the emitter debounce, pick the localised chatter/popup label, and
/// document which family's arcs the request carries.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum WeaponFamily {
    /// Beam phaser banks.
    #[default]
    Phasers,
    /// Projectile blaster banks.
    Blasters,
    /// Homing torpedo tubes.
    Torpedoes,
}

/// One usable weapon emitter's fire-arc + range constraint, carried in an
/// [`CoordinationPayload::ArcBearingRequest`] (issue #767).
///
/// The emitter (Tactical) fills the union of a family's ONLINE emitter arcs so
/// Helm can turn toward the emitting family's *actual* nearest arc edge — and
/// self-clear against the same geometry — rather than a hard-coded phaser arc.
/// `arc_deg` is the family's applicable arc (phaser auto-fire arc, blaster /
/// torpedo fire arc); `range` is that emitter's already-modifier-scaled
/// effective range, so Helm's range test matches the emitter's exactly.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct WeaponEmitterArc {
    /// Centre of the emitter's fire arc, degrees clockwise from ship-forward.
    pub facing_deg: f32,
    /// Total fire-arc width in degrees for this family.
    pub arc_deg: f32,
    /// Effective range of this emitter in world units.
    pub range: f32,
}

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
    /// Shared readiness + blocking-reason contract (issue #764).
    #[serde(default)]
    pub readiness: WeaponReadiness,
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
    /// Shared readiness + blocking-reason contract (issue #764).
    #[serde(default)]
    pub readiness: WeaponReadiness,
    /// Barrel indices the most recently launched round left from (issue #766).
    /// One entry per shot (torpedoes fire one-per-burst). Empty when the tube
    /// is idle or has no multi-barrel pattern. Drives the Tactical barrel/step
    /// indicator.
    #[serde(default)]
    pub active_barrels: Vec<u32>,
    /// 1-based index of the current pattern step (0 when idle). With
    /// `pattern_len` this renders as "step N/M" for a patterned attack.
    #[serde(default)]
    pub pattern_step: u32,
    /// Total number of steps in this tube's authored firing pattern (issue
    /// #766). 0 when the tube has no multi-barrel pattern (single-barrel tube).
    #[serde(default)]
    pub pattern_len: u32,
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
    /// Shared readiness + blocking-reason contract (issue #764).
    #[serde(default)]
    pub readiness: WeaponReadiness,
    /// Barrel indices firing on the current pattern step (issue #765). Empty
    /// when the bank is idle or between steps of an alternating pattern with no
    /// barrel currently active. Drives the Tactical barrel/step indicator.
    #[serde(default)]
    pub active_barrels: Vec<u32>,
    /// 1-based index of the current pattern step (0 when idle). Together with
    /// `pattern_len` this renders as "step N/M" for a patterned attack.
    #[serde(default)]
    pub pattern_step: u32,
    /// Total number of steps in this bank's authored firing pattern (issue
    /// #765). 0 when the bank has no multi-barrel pattern (single-barrel volley).
    #[serde(default)]
    pub pattern_len: u32,
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

/// A single authored response option as projected onto the wire (issue #761).
///
/// Carries the authored response `text`, the authored `important` flag (a
/// response the author marked as consequential enough to warrant a client-side
/// confirmation before submission), and the authoritative `available` flag
/// (false when the message's sender is currently out of comms range, mirroring
/// [`CommsMessage::sender_in_range`]). The client greys unavailable responses
/// and confirms important ones; neither flag relaxes any server-side gate.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CommsResponseView {
    /// Authored response text (authored TOML, not a strings.csv id — mirrors
    /// the `display_name` precedent).
    pub text: String,
    /// True when the author flagged this response as important; the client
    /// requires an explicit confirm before submitting it. Defaults to false
    /// for backward-compatible wire payloads.
    #[serde(default)]
    pub important: bool,
    /// True when this response is currently submittable (its message's sender
    /// is in comms range). Defaults to true for backward compatibility.
    #[serde(default = "default_true")]
    pub available: bool,
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
    ///
    /// Promoted from `Vec<String>` (issue #761) to a per-response view so the
    /// client learns each response's authored `important` flag and its
    /// authoritative `available` (in-range) state, not just its text.
    pub responses: Vec<CommsResponseView>,
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

/// One contextual tutorial overlay's trigger condition (issue #916).
///
/// Pure DATA carriage: the trigger vocabulary (`kind` and its parameters) is
/// authored in the ship TOML and evaluated by the client's tutorial
/// state-builder (`gui/tutorial-state.js`). Rust never interprets any of these
/// fields — `kind` is a plain string, not an enum, precisely so a new trigger
/// kind is a TOML + JS change with no Rust branch (AGENTS.md #11 / the
/// pure-JS-client direction). Shipped kinds: `first_visit` (shows until
/// dismissed), `control_unused` (completes when the named console action is
/// first used), `state` (gates on a field of the console's own payload).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TutorialTriggerWire {
    /// Trigger kind code (e.g. `"first_visit"`). Vocabulary owned by
    /// `gui/tutorial-state.js`; unknown kinds fail closed there.
    pub kind: String,
    /// Console action name that completes this overlay when first used
    /// (e.g. `"set_helm"`). Applies to every kind, not only `control_unused`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
    /// `state` kind: dot-path into the console payload (e.g. `"boost_enabled"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `state` kind: comparison operator (`truthy`/`falsy`/`eq`/`ne`/`gt`/
    /// `gte`/`lt`/`lte`); the JS evaluator defaults to `truthy` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// `state` kind: numeric comparison operand for the binary operators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

/// One contextual tutorial overlay authored on a station (issue #916), from a
/// `[[station.tutorial]]` block in the ship TOML.
///
/// `title` and `text` are `strings.csv` ids (never composed English — the
/// same structured-codes-over-the-wire policy as `crate::ship::manual`), and
/// the strings gate enforces both keys because `title`/`text` are in
/// `scripts/strings-rules.mjs` LOCALISABLE. The client component resolves them
/// through `t()` at render time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TutorialOverlayWire {
    /// Stable overlay id, unique within the station only — the client scopes
    /// its persisted dismissal key per station (`<station>/<id>`, see
    /// `gui/tutorial-state.js`), so two stations may author the same id
    /// without sharing dismissal state.
    pub id: String,
    /// When this overlay is eligible to show (see [`TutorialTriggerWire`]).
    pub trigger: TutorialTriggerWire,
    /// `strings.csv` id for the overlay heading.
    pub title: String,
    /// `strings.csv` id for the overlay body text.
    pub text: String,
    /// Optional DOM element id in the console HTML the overlay points at; the
    /// client highlights that control while the overlay is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// Display priority: higher shows first; equal priorities keep authored
    /// order. Lets contextual event tips (red alert, boost ready) preempt the
    /// intro queue without a hardcoded ordering rule.
    #[serde(default)]
    pub priority: i32,
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
    /// RGBA colour the Helm UI uses for the red-alert hostile weapon-arc
    /// overlay (issue #874). Sourced from `[helm_console] hostile_arc_color`.
    /// A gameplay-adjacent presentation value, so it is authored in TOML rather
    /// than inlined in JS (AGENTS.md #11); the default below is the TOML-parse
    /// fallback only.
    #[serde(default = "default_hostile_arc_color")]
    pub hostile_arc_color: [f32; 4],
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
    /// List of system IDs for helm-related systems (thrust, steering, impulse,
    /// boost, lateral-thrust). Sourced from the ship's `[[system]]` entries that
    /// belong to the helm station. Used by the client to know which systems
    /// are helm-axis systems for UI grouping.
    #[serde(default)]
    pub helm_systems: Vec<String>,
    /// Vertical movement mode for this ship. Sourced from
    /// `[helm_capability] vertical_movement_mode` in the ship TOML.
    /// "planar" = no vertical movement, "bounded" = AI-only collision avoidance,
    /// "full_3d" = full six-degree-of-freedom flight.
    #[serde(default = "default_vertical_movement_mode")]
    pub vertical_movement_mode: String,
    /// Steering multiplier applied while impulse drive is active.
    /// 0.0 = no steering, 0.1 = harsh but possible, 1.0 = full steering.
    /// Sourced from `[helm_capability.impulse] steering_multiplier` in the ship TOML.
    #[serde(default = "default_impulse_steering_multiplier")]
    pub impulse_steering_multiplier: f32,
    /// Contextual tutorial overlay definitions per station (issue #916),
    /// keyed by station id. Authored as `[[station.tutorial]]` blocks in the
    /// ship TOML and delivered on `Welcome`; the client's tutorial
    /// state-builder (`gui/tutorial-state.js`) evaluates the trigger
    /// vocabulary — the server carries the data and never interprets it, so
    /// there are no station-specific Rust branches.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub station_tutorials: HashMap<String, Vec<TutorialOverlayWire>>,
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

fn default_vertical_movement_mode() -> String {
    "planar".to_string()
}

fn default_impulse_steering_multiplier() -> f32 {
    0.1
}

fn default_phaser_beam_color() -> [f32; 4] {
    [1.0, 0.6, 0.1, 1.0]
}

fn default_torpedo_arc_color() -> [f32; 4] {
    [0.2, 0.7, 1.0, 1.0]
}

/// TOML-parse fallback for the red-alert hostile weapon-arc overlay colour
/// (issue #874): a hostile red at 7% fill.
///
/// Deliberately far below the Tactical radar's own 0.30 / 0.25 arc opacities —
/// the overlay paints OVER the blips a helm is actually flying by, so this alpha
/// is what keeps them legible through it, and the overlay must read as a hint
/// rather than as a second radar. Authored per hull in
/// `[helm_console] hostile_arc_color`; this constant only covers a hull whose
/// TOML omits it.
///
/// Kept byte-identical to the value every shipped hull authors and to the
/// client-side placeholder in `gui/sim-state.js`. AGENTS.md #11 sanctions a
/// parse default and a client placeholder as separate categories, but a hull
/// that omits the key must not render a visibly different overlay from one that
/// authors the house value — three fallbacks that disagree is a drift bug
/// waiting for the first hull to leave the key out.
fn default_hostile_arc_color() -> [f32; 4] {
    [1.0, 0.3, 0.3, 0.07]
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
            hostile_arc_color: default_hostile_arc_color(),
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
            helm_systems: Vec::new(),
            vertical_movement_mode: default_vertical_movement_mode(),
            impulse_steering_multiplier: default_impulse_steering_multiplier(),
            station_tutorials: HashMap::new(),
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

    /// World-space Y coordinate (vertical / altitude). Returns 0.0 when
    /// `position` is `None`. Used by 3D torpedo guidance and collision
    /// (issue #768); `0.0` for Planar entities on the cruise plane.
    pub fn y(&self) -> f32 {
        self.position.map(|p| p[1]).unwrap_or(0.0)
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
    /// Populated from the same `ShipShields::snapshot()` this ship's own
    /// `ShieldsBlackboard.facings` uses (issue #927 —
    /// `ship::shields::shield_facing_statuses`, called from
    /// `server_app::sim_state_broadcaster`); previously always `None` on
    /// this wire type regardless of the entity's shields, which is why the
    /// Sensors panel's `target_shields` was always empty for every target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shields: Option<Vec<ShieldFacingStatus>>,
    /// Shield generator frequency (0.0-1.0) for this entity, present only
    /// for ship-like entities. The SAME authoritative `ShipShields::frequency()`
    /// `console_ai::server::tick_frequency_hint_high_fidelity` reads to build
    /// `FrequencyHint` (issue #927) — one producer, no parallel derivation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield_freq: Option<f32>,
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
    /// Set the ship's Red Alert state to an explicit desired value (issue
    /// #748). Targets `red-alert`. Replaces the former `ToggleRedAlert`: the
    /// captain UI and the Captain AI both send the desired end state, so a
    /// retried, duplicated, or stale-UI command is idempotent (the handler
    /// assigns, it does not invert).
    SetRedAlert {
        active: bool,
    },
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
    /// Set the vertical (up/down) thrust axis. Targets `helm-vertical-thrust`
    /// (issue #744). AI-only today — emitted by `ai_helm_vertical_thrust`.
    VerticalThrustInput {
        vertical: f32,
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
    /// Flip the God Mode debug cheat (local ship takes no damage), issue #900.
    /// Targets `god-mode` (`system_registry::GOD_MODE_SYSTEM_ID`), an ownerless
    /// capability no ship TOML declares. Admission's `LOCAL_CONSOLE_TOKEN`
    /// branch is the only path that reaches it — see that constant's doc and
    /// `command_admission::policy::is_command_authorized`. Routed through the
    /// normal command log (like every other command) rather than a
    /// `bridge`-local thread-local, so a replay that toggled it reproduces the
    /// same damage outcomes and two instances that disagree on it diverge in
    /// their digest instead of silently forking. A toggle rather than an
    /// explicit `active` flag: the host page has exactly one button and no
    /// retry path, so there is no idempotency hazard to design against.
    ToggleGodMode,
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
    /// Pre-scenario selection request: the sender proposes a scenario by its
    /// stable catalog id (issue #755). Any participant — the host page's own
    /// UI *or* a connected phone — may send it; the host-runtime arbiter
    /// applies first-valid-wins against the pre-load catalog and ignores it
    /// once a scenario is already locked. Deliberately NOT gated to any single
    /// token: both server and phone participants can make the first valid
    /// selection (no voting, no captain authority). Also drives the second
    /// round after Game Over — the return re-enters this same arbiter flow
    /// (issue #756).
    SelectScenario {
        scenario_id: String,
    },
    /// Pre-scenario selection request: the sender proposes a player ship by
    /// its template path (issue #755). Validated by the host-runtime arbiter
    /// against the *locked scenario's* offered ships, first-valid-wins. Like
    /// `SelectScenario`, accepted from any participant token.
    SelectPlayerShip {
        template_path: String,
    },
    /// Flip one host-page debug overlay from a connected phone's Debug/Cheat
    /// tab (issue #940).
    ///
    /// A **session** control, not a ship-system command, which is why it is a
    /// top-level variant rather than a `SystemControlPayload` on the
    /// `ControlSystem` envelope: it carries no simulation state — an overlay
    /// being drawn changes no outcome — so keeping it out of the command log
    /// keeps replays clean. The one client-reachable toggle that *does* change
    /// outcomes, God Mode, stays on the `ControlSystem` admission path (issue
    /// #900) for exactly the opposite reason.
    ///
    /// **Absent from a demo build.** The `phoenix_demo_build` cfg `build.rs`
    /// derives from `PHOENIX_DEMO_BUILD` removes this variant, its drain
    /// (`debug_overlay::drain_client_debug_flags`) and its registration, so a
    /// demo binary's `ClientMessage` has no such shape to decode: the wire
    /// route is gone, not merely refused. That is what the hidden Debug/Cheat
    /// tab claims, and a hidden tab is a forgeable UI fact on its own.
    #[cfg(not(phoenix_demo_build))]
    ToggleDebugFlag {
        flag: DebugFlag,
    },
    /// Pause or resume the simulation clock from a connected phone's Gameplay
    /// tab (issue #940).
    ///
    /// Its own top-level variant rather than a [`DebugFlag`], for two unrelated
    /// reasons that happen to point the same way:
    ///
    ///  1. Pause is not a debug overlay. It stops the clock every system
    ///     advances on, which is why `SimulationPaused` is authoritative state
    ///     while its `Debug*Enabled` neighbours are presentation.
    ///  2. It has to be build-gated **separately**, and it is gated the same
    ///     way for a different reason. A demo is played by N strangers on N
    ///     phones; any one of them could otherwise freeze the mission for
    ///     everyone, repeatedly, and nothing in the drain checks station,
    ///     captaincy or `GamePhase`. So the phone's pause is dev-only, and the
    ///     host's own cog (issue #939) keeps it in every build — the host is a
    ///     single trusted operator standing at the viewscreen.
    ///
    /// Like `ToggleDebugFlag` it never crosses command admission, and here that
    /// is not a preference but a necessity: pausing starves `FixedUpdate`,
    /// which is where admission has run since issue #895, so an admitted pause
    /// could never be undone. It is drained frame-driven in `PreUpdate`
    /// instead — the same schedule the host page's own pause toggle uses.
    #[cfg(not(phoenix_demo_build))]
    TogglePause,
}

/// One host-page debug overlay a settings menu can flip (issue #940).
///
/// Mirrors the diagnostic half of `crate::bridge::DebugToggleKind` — that enum
/// is the host page's local pending-toggle vocabulary and this is its wire
/// form, with a `From<DebugFlag>` conversion between them so the two cannot
/// drift into different flag sets.
///
/// Every member is diagnostic-only by construction, which is what lets
/// `ClientMessage::ToggleDebugFlag` be removed wholesale from a demo build
/// rather than narrowed flag-by-flag. Pause used to be a member and is not one
/// any more: it is authoritative state, it needs a different build story, and a
/// predicate saying "all of these except that one" was the seam through which a
/// demo phone could still freeze the mission. It is
/// `ClientMessage::TogglePause` now.
///
/// Not itself build-gated: `ServerMessage::DebugState` reports these in every
/// build, and a read-back grants no authority.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DebugFlag {
    /// Region wireframes (`DebugRegionsEnabled`).
    Regions,
    /// Modifier debug overlay (`DebugOverlayEnabled`).
    Modifiers,
    /// Damage debug log (`DebugDamageEnabled`).
    Damage,
    /// Entity behaviour overlay (`DebugEntitiesEnabled`).
    Entities,
    /// Entity inspector overlay (`DebugEntityInspectorEnabled`).
    Inspector,
}

impl DebugFlag {
    /// Every flag, in the order `ServerMessage::DebugState` reports them.
    ///
    /// A fixed slice rather than map iteration, so identical state always
    /// produces an identical message — the client's fold diffs it and the
    /// codec test pins its shape.
    pub const ALL: [DebugFlag; 5] = [
        DebugFlag::Regions,
        DebugFlag::Modifiers,
        DebugFlag::Damage,
        DebugFlag::Entities,
        DebugFlag::Inspector,
    ];
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
    /// Weapons asks Helm to yaw so the selected weapon family's firing arc
    /// bears on `uuid`. AI Helm folds this into its steering; human Helm gets a
    /// family-aware popup via `route_coordination` (issues #677, #767).
    ///
    /// `family` names the emitting weapon family (phasers/blasters/torpedoes);
    /// `arcs` carries that family's usable ONLINE emitter arcs so Helm turns
    /// toward — and self-clears against — the actual family's geometry rather
    /// than a hard-coded phaser arc (issue #767).
    ArcBearingRequest {
        uuid: String,
        label: String,
        family: WeaponFamily,
        arcs: Vec<WeaponEmitterArc>,
    },
    /// Weapons withdraws a standing `ArcBearingRequest` because the emitting
    /// family it raised the request for has stopped being usable — a torpedo
    /// tube drained its last round, a bank was knocked offline — while the
    /// request was still standing (issue #932).
    ///
    /// `tick_weapons_arc_request` re-derives family usability every tick, so
    /// it is the one system that can notice this happen; before #932 it only
    /// stopped RE-EMITTING, and the last debounced request simply stood,
    /// unconsumed, until a yielding leg honoured a family with nothing left to
    /// bring to bear. Consumed unconditionally by AI Helm — clearing
    /// `PendingArcBearingRequest` is expiry, not a steering decision, so
    /// `AiPolicy::leg_yields_to_arc_requests` (issue #918) plays no part in
    /// it; only the steering WRITE that a live request can bias is gated by
    /// the leg's consent.
    ArcBearingWithdraw { family: WeaponFamily },
    /// Power system reports a brownout (demand exceeds supply) for a group
    /// that is actively drawing power it cannot get (issue #678).
    /// Fire-once-debounced; only fires when the affected system has level > 1
    /// (not idle at minimum draw) while total allocation > 6 (battery draining).
    PowerBrownout {
        /// Which power group (e.g. "weapons", "helm", "sensors").
        group: String,
        /// `strings.csv` id for the affected system's display label (e.g.
        /// `"power.group.weapons"`); `localiseTree` resolves it client-side
        /// (issue #977).
        label: String,
        /// Current allocated level (what the system is actually getting).
        allocated_level: u8,
    },
    /// Navigation clears Helm to follow the ship's current `NavigationWaypoint`
    /// (issues #681, #702).
    ///
    /// Navigation control is the `generation`: the waypoint is the goal and
    /// lives on the ship as one per-entity component that both consoles read;
    /// `process_coordination_lag` latches only the generation, never a wire
    /// position, so the `AiMemory.nav_goal` split brain this replaced cannot
    /// return. `x` / `z` ride alongside for DISPLAY only (issue #977): the
    /// chatter popup renders "steer toward waypoint (x, z)" from them, replacing
    /// the English `label` Rust used to compose. No navigation logic reads them.
    /// See [`NavigationWaypoint::generation`].
    ///
    /// [`NavigationWaypoint::generation`]: crate::navigation_plugin::NavigationWaypoint::generation
    NavigateTo { generation: u64, x: f32, z: f32 },
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
    /// A seat's coarsened intent / state-change advisory (issue #879).
    ///
    /// Unlike every payload above it, this one is not addressed to a single
    /// console: it is broadcast to every human seat on the SOURCE ship, so the
    /// remaining crew of a partly-backfilled bridge shares one picture of what
    /// the automation is doing. Delivery is transient — the existing popup
    /// surface and nothing else. There is deliberately no durable log.
    ///
    /// Produced only by [`crate::ship::intent_narration::coalesce_intent`], on
    /// a decision *change*, never per shot or per thrust tick. It carries the
    /// coarse fact and the one label naming it and no figures at all — the same
    /// information boundary #737 drew for [`Self::RepairRequest`].
    IntentAdvisory {
        /// Which coarse decision changed.
        kind: IntentKind,
        /// The label naming it: a target, a shield facing, a power group, or
        /// an authored manoeuvre state. `None` for the kinds that name nothing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        /// Per-ship monotonic ordering handle.
        ///
        /// A COUNTER, never a timestamp: two lockstep peers advancing the same
        /// simulation must stamp the same advisory with the same value, and a
        /// wall-clock reading would differ on every host. The same rule
        /// [`Self::NavigateTo`]'s generation follows.
        generation: u64,
    },
}

/// Which coarse decision a seat has just changed (issue #879).
///
/// Deliberately a closed typed set rather than prose: the host emits the fact,
/// the client renders the sentence. A `String` here would put player-visible
/// English in Rust for a payload that has a client-side renderer already.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntentKind {
    /// Took a target where it had none.
    TargetAcquired,
    /// Moved from one target to another.
    TargetSwitched,
    /// The ship's alert state now licenses the aggressive half of its class
    /// doctrine.
    CombatPostureEntered,
    /// …and stood back down from it.
    CombatPostureLeft,
    /// Hull damage crossed the authored break-off threshold.
    BreakingOff,
    /// Concentrated the shield grid on one facing.
    ShieldArcFocused,
    /// A power group is drawing more than the reactor can supply.
    PowerBrownout,
    /// Began a new authored manoeuvre leg.
    ManoeuvreBegun,
}

/// One selectable scenario in the pre-load catalog as delivered to phones
/// over the wire (issue #755). Mirrors `world::manifest::ScenarioCatalogEntry`
/// but lives here as a serde wire type so it can ride inside
/// [`ServerMessage::ScenarioCatalog`]. `ships` reuses the world's
/// [`AvailableShipEntry`] shape so the phone picker presents ships identically
/// to the host page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioCatalogWire {
    pub id: String,
    pub world: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub ships: Vec<crate::world::config::AvailableShipEntry>,
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
        /// Vertical launch position (issue #768). `#[serde(default)]` so a
        /// pre-#768 planar message with no `y` decodes to `0.0`, keeping the
        /// wire backward compatible.
        #[serde(default)]
        y: f32,
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
    /// per-group allocation levels, the battery charge, whether the reserve is
    /// currently emptying, and whether the reactor is locked out after a full
    /// brownout.
    ///
    /// The third group is `shields`, not `sensors` (issue #952). `locked` marks
    /// the exhaustion lock: when the battery bottoms out every group is forced
    /// to 1 and the allocation controls freeze until the reserve recovers past
    /// `emergency_threshold`. `draining` says which way the reserve is moving.
    PowerState {
        helm: u8,
        weapons: u8,
        shields: u8,
        battery_charge: f32,
        draining: bool,
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
    /// Sent to the submitting Comms console holder when a `RespondToMessage`
    /// command is refused by the host (issue #761): the message has no active
    /// dialogue (stale), its sender is out of comms range, or the response
    /// index is out of bounds (forced/stale). The client flashes the attempted
    /// response control red. Accepted responses remain immediate and
    /// irreversible — this is a rejection-only feedback channel.
    CommsResponseRejected {
        message_id: String,
        response_index: usize,
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
    /// Clients should switch back to the lobby panel. Station claims and ready
    /// state are cleared for every player (issue #756); the accompanying
    /// per-player `StationAssigned { station: None }` broadcasts carry the
    /// cleared seats.
    ReturnedToLobby,
    /// Pre-scenario catalog + current lock state, delivered to connected
    /// phones before any world is loaded (issue #755). Phones have no WASM
    /// accessor, so the host-runtime arbiter synthesizes this message: it is
    /// sent when a phone connects and re-broadcast on every lock change so
    /// every phone can render the scenario/ship picker and reflect the
    /// first-valid-wins outcome. `locked_scenario` / `locked_ship` are `None`
    /// until a participant's selection is accepted.
    ScenarioCatalog {
        scenarios: Vec<ScenarioCatalogWire>,
        locked_scenario: Option<String>,
        locked_ship: Option<String>,
    },
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
    /// Read-only ship manual replicated to the phone client (issue #772).
    ///
    /// Fully determined by the selected ship's config, so it is published once
    /// per client alongside `Welcome`. Carries one entry per authored station:
    /// the station's authored overview prose plus generated, structured system
    /// sections (numeric values + machine codes the client renders via `t()`).
    /// The client treats it as presentation state only — it never mutates or
    /// authors from it. See `crate::ship::manual`.
    ShipManual {
        manual: ShipManualWire,
    },
    /// Authoritative read-back of the host's debug/session flags (issue #940).
    ///
    /// Broadcast to every client whenever any of them changes, so a phone's
    /// settings menu paints from what the simulation actually holds rather than
    /// from what it just asked for. That matters more here than for most
    /// controls: a client's toggle can be refused outright (the route does not
    /// exist in a demo build), and God Mode crosses command admission, so it
    /// lands a tick later than the click.
    ///
    /// `flags` carries one entry per `DebugFlag::ALL`, in that order. `paused`
    /// and `god_mode` are separate fields because they are a different kind of
    /// thing: both are authoritative simulation state (a stopped clock, an
    /// invulnerable ship) rather than host-page overlays, and neither has a
    /// `DebugFlag` any more.
    ///
    /// Reported in **every** build, unlike the two client messages that can
    /// change it. A read-back grants no authority: a demo phone learns that the
    /// host paused the mission and still has no way to pause it itself.
    DebugState {
        flags: Vec<(DebugFlag, bool)>,
        paused: bool,
        god_mode: bool,
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
    /// Set when the game has ended, as a `strings.csv` id (issue #977):
    /// `server.game_over.ship_destroyed` for hull death, or the scenario
    /// `game_over` message id otherwise. The HUD channel resolves it through
    /// `localiseTree` client-side. `None` while in progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_over_message: Option<String>,
}

/// A single radar blip on the Tactical console radar.
///
/// Positions are normalised to `[-1.0, 1.0]` where ±1.0 = the effective
/// tactical radar range (base `tactical_radar_range` × `RadarRange` modifier).
/// Produced server-side by `publish_tactical_radar_blackboard` from live ECS
/// transforms joined with the static world entity registry for tags/radius.
/// (It was `publish_weapons_core_blackboard` until issue #829 moved the blips
/// and regions onto the `tactical-radar` blackboard.)
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
    /// `true` when this contact is a HOSTILE whose hull carries at least one
    /// torpedo tube (issue #957). The tactical console badges these so the crew
    /// can tell a torpedo boat from a phaser-only escort *before* the first
    /// torpedo is in flight.
    ///
    /// This is a CAPABILITY, not a readiness reading: it is `true` for a hull
    /// with tubes even when every tube is unloaded and the magazine is empty,
    /// and it does not promise a launch — the world-scoped torpedo conservation
    /// gate (issue #943) can still refuse one from a fully-armed hull. Readiness
    /// flickers tick to tick; what a helm needs to plan around is whether that
    /// bird has tubes at all.
    ///
    /// Hostility is resolved server-side against the observing ship's faction
    /// (`crate::faction::is_enemy`), the same predicate the helm's hostile-arc
    /// overlay uses, so a friendly torpedo boat — and the player's own ship —
    /// never badges itself.
    ///
    /// Not scan-gated. The precedent is local to this struct: [`Self::threat_level`],
    /// [`Self::description`] and [`Self::target_tags`] are already authored
    /// hostile intel shipped through exactly the two gates this field passes —
    /// the tactical radar's `shows` tag filter and its effective-range cull —
    /// and there is no third gate for any of them.
    ///
    /// **And deliberately not red-alert gated**, unlike its nearest wire
    /// sibling [`HelmBlackboard::hostile_weapon_arcs`]. The difference is what
    /// the two carry. Arcs are live per-contact firing geometry that swings as
    /// the hostile manoeuvres, bolted onto the helm blackboard as an extra
    /// channel of its own, so gating them costs the crew nothing until they are
    /// already fighting. `torpedo_armed` is one static bit about a contact the
    /// tactical radar has *already* drawn, next to three other ungated intel
    /// fields on the same blip — and it is precisely the fact a crew needs
    /// *before* the shooting starts, which is the wrong side of a red-alert
    /// gate. Gating it would withhold nothing `threat_level` does not already
    /// give away.
    #[serde(default)]
    pub torpedo_armed: bool,
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
    /// Authoritative Red Alert state of the selected target, replicated onto the
    /// Sensors scan surface only (issue #749). `Some(true)`/`Some(false)` when the
    /// selected target is a Red-Alert-capable ship; `None` when there is no
    /// selection, the target is a non-ship contact (asteroid/star/planet/region),
    /// or the target is otherwise incapable of Red Alert. `None` renders as no
    /// alert field at all on the scan card — the visibility boundary that keeps
    /// this intelligence to the Sensors operator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_target_alert: Option<bool>,
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
    /// Hostile weapon-arc sectors for the helm-radar overlay (issue #874).
    ///
    /// **Populated for the LOCAL ship only, and only at red alert.** The same
    /// posture [`TacticalRadarBlackboard::blips`] takes, for the same reasons:
    /// no NPC pays the bandwidth, and this is not put on `EntitySnapshot` — that
    /// broadcasts every entity to every client, which would leak arc intel to
    /// consoles that have no business with it and to clients not at red alert.
    ///
    /// The sectors are copied VERBATIM from
    /// [`crate::ai::AiWorldEntity::weapon_arcs`] — the same list the helm AI's
    /// exposure fact is reduced from. The client does no arc math on them
    /// beyond world-bearing → screen-angle projection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hostile_weapon_arcs: Vec<HostileWeaponArcContact>,
}

/// One hostile contact's weapon arcs, anchored at that contact's world position
/// (issue #874).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HostileWeaponArcContact {
    /// The hostile's entity uuid, so the client can associate the overlay with
    /// the blip it already renders.
    pub uuid: String,
    /// The hostile's world X — the anchor the wedges radiate from.
    pub x: f32,
    /// The hostile's world Z.
    pub z: f32,
    /// That hostile's arc sectors, in producer order.
    pub arcs: Vec<HostileWeaponArc>,
}

/// One weapon arc as a WORLD-bearing sector (issue #874).
///
/// World-relative rather than ship-relative on purpose: it means the client
/// needs no trigonometry to un-rotate the hostile's hull, so it cannot
/// accidentally compute a different arc from the one the AI reasons about.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HostileWeaponArc {
    /// World bearing of the sector's centre-line, degrees, `(−180, 180]`.
    /// Same convention as ship yaw: `0` points along `−Z`.
    pub bearing_deg: f32,
    /// HALF the arc width, degrees.
    pub half_angle_deg: f32,
    /// The bank's effective reach, world units.
    pub range: f32,
}

impl From<&crate::weapons::arc_geometry::WeaponArcSector> for HostileWeaponArc {
    fn from(s: &crate::weapons::arc_geometry::WeaponArcSector) -> Self {
        Self {
            bearing_deg: s.bearing_deg,
            half_angle_deg: s.half_angle_deg,
            range: s.range,
        }
    }
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
    /// Count of this ship's torpedoes currently in flight (issue #782, AC5).
    /// Published each tick in `SimSet::Publish` so other policies read it on the
    /// NEXT AI tick — the same one-tick-lag discipline as the combat lock. This
    /// is the public authoritative in-flight fact a torpedo tube or magazine
    /// policy (or another ship's policy) can gate on.
    #[serde(default)]
    pub torpedoes_in_flight: u32,
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
    /// True when the reserve is emptying at the current draw. Mirrors
    /// `PowerBlackboard::draining` for reactor-scoped readers, and replaces the
    /// `locked` flag issue #952 retired along with the brownout lock.
    pub draining: bool,
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
    /// Whether the reserve is emptying at the current draw — which way the
    /// charge is going, so the panel can paint a draining reserve.
    /// `#[serde(default)]` for round-tripping older payloads.
    #[serde(default)]
    pub draining: bool,
    /// Whether the reserve is actually FILLING at the current draw.
    ///
    /// Deliberately not the negation of `draining`: a hull may author a rate of
    /// exactly `0.0` for some total, and at that total the reserve is frozen —
    /// neither emptying nor filling. `ph-battery-bar`'s pulsing CHARGING
    /// indicator is driven from this, so that a parked reserve says nothing
    /// rather than promising a recovery that will never arrive.
    /// `#[serde(default)]` for pre-#952 payloads.
    #[serde(default)]
    pub charging: bool,
    /// Whether the reactor is locked out after a full brownout: the battery
    /// bottomed out, every group was forced to 1, and the allocation controls
    /// are frozen until the charge recovers past `emergency_threshold`. Lets the
    /// Power panel grey out its +/- and show the lockout. `#[serde(default)]`
    /// so payloads predating the lock's restoration still decode.
    #[serde(default)]
    pub locked: bool,
}

/// An authority-checked intra-system command produced by `admit_system_commands`.
///
/// The source identity is stripped at admission; `response_token` carries the
/// originating client's token purely for routing replies (not for behavioral
/// branching).
///
/// Deliberately **not** `Serialize` (issue #898). This type is in-process only.
/// The command log's entry type is
/// [`crate::command_admission::log::LoggedCommand`], which carries this
/// command's target and payload but replaces `response_token` with the routed
/// ship's `EntityUuid` — because the log's destinations are save files and
/// peers, and a session token is a bearer credential (AGENTS.md constraint 2).
/// Adding a `Serialize` derive here is how that would quietly come undone.
#[derive(Clone, Debug, PartialEq)]
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
    /// Bearing of this ship's own frozen Combat Lock target, in degrees, or
    /// None if no lock. Renamed from `target_bearing` (issue #926) — it was
    /// easily confused with `threat_bearing` below, a different quantity.
    #[serde(default)]
    pub combat_lock_bearing: Option<f32>,
    /// Relative bearing (degrees) of the nearest hostile in sensor range —
    /// the SAME authoritative fact the backfilled Shields focus AI reads via
    /// `PendingShieldsThreatBearing` (issue #926), sourced from
    /// `ship::sensors::SensorsThreatState` on this ship. `None` when Sensors
    /// reports no hostile in range. One producer for both the AI fact and
    /// this console field — no parallel client-side derivation.
    #[serde(default)]
    pub threat_bearing: Option<f32>,
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
    /// `strings.csv` id for the display label shown in the HTML panel (e.g.
    /// `"power.group.helm"`); `localiseTree` resolves it to "HELM" client-side
    /// (issue #977).
    pub label: String,
    /// Current EFFECTIVE power level (1 – `max_level`) — what the group is
    /// actually running at, which is `commanded_level` unless the reactor's
    /// battery floor is holding it down. This is the number the pips light.
    pub level: u8,
    /// The level this group has been COMMANDED to run at, ignoring any battery
    /// floor (issue #952).
    ///
    /// The panel's `+`/`−` buttons send an ABSOLUTE level, and
    /// `PowerSystem::set_group_allocation` measures the delta against the
    /// commanded level — so a client that steps from `level` while a floor is
    /// in force sends a level BELOW the standing order and silently lowers it.
    /// Helm commanded 4 and floored to 2: `+` would send 3, which is a
    /// *decrease*. The control has to step from this field; `level` is for
    /// display. `#[serde(default)]` for pre-#952 payloads, where a `0` reads as
    /// "unknown" and the client falls back to `level`.
    #[serde(default)]
    pub commanded_level: u8,
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
    /// Comms cares about `Hail` directives (issue #753): the Backfill Comms
    /// AI consumes them from its local scored-objective pool and issues the
    /// same `Hail` action a human Comms officer sends.
    Comms,
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
