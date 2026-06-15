use crate::region_effects::RegionEffectsConfig;
use crate::region_shape::RegionShape;
use serde::de::Error as SerdeError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Configuration for a single named AI state.
///
/// Each entry in a `[[behaviour.state]]` array defines the parameters
/// for one state. The `name` field is used as a stable identifier for
/// per-spawn `[spawn.overrides]` by-name replacement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StateConfig {
    /// Stable name for this state (used in `initial_state` and overrides).
    pub name: String,
    /// State kind: `"idle"`, `"patrolling"`, `"pursuing"`, or `"attacking"`.
    #[serde(default)]
    pub kind: String,
    /// Ordered waypoint anchor names (used by `patrolling`).
    #[serde(default)]
    pub waypoints: Vec<String>,
    /// Whether to loop back to the first waypoint after the last (patrolling).
    #[serde(default)]
    pub loop_path: bool,
    /// Desired forward speed fraction [0, 1], clamped at load time.
    #[serde(default)]
    pub target_speed: f32,
    /// Distance to maintain from target (world units) for the `attacking` state.
    /// The AI thrusts at `target_speed` when further than this, and holds station
    /// (thrust = 0) when closer.
    #[serde(default)]
    pub maintain_range: f32,
    /// Duration in seconds for the `warping_out` state before the entity self-despawns.
    #[serde(default)]
    pub duration_secs: f32,
}

impl StateConfig {
    /// Clamp mutable fields into valid ranges after deserialisation.
    fn clamp(&mut self) {
        self.target_speed = self.target_speed.clamp(0.0, 1.0);
    }
}

/// Configuration for an AI behaviour controller attached to an entity.
/// Re-exports the AI module's config type so callers only need `entity_config`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BehaviourConfig {
    /// Name of the initial AI state (e.g. `"idle"`).
    pub initial_state: String,
    /// Typed state parameter blocks.  An empty vec is valid (only `initial_state`
    /// is required; states with no extra params — like `idle` — need no entry).
    #[serde(default)]
    pub state: Vec<StateConfig>,
    /// Transition rules evaluated in declaration order.
    #[serde(default)]
    pub transition: Vec<crate::ai::TransitionConfig>,
}

/// Shape variant for the `[mesh]` section of an entity TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshShape {
    Sphere,
    Cuboid,
    Torus,
}

/// Visual mesh definition for an entity.
///
/// Present in entity TOMLs as a `[mesh]` section. The renderer creates the
/// appropriate Bevy primitive and material from this data; entities without
/// a `[mesh]` section are not given a 3-D visual on the viewscreen.
///
/// When `model` is set, the renderer loads a GLB scene instead of creating a
/// procedural shape (the `shape`/`colour`/`radius`/etc. fields are ignored for
/// rendering but kept as fallback). `scale` and `rotation` are applied to both
/// paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshConfig {
    /// Path to a .glb file, e.g. "assets/models/dynasty_destroyer.glb".
    /// When set, overrides the procedural shape rendering.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional rig-sidecar variant name. The model's rig sidecar is looked
    /// up alongside the `.glb` as `<stem>.<variant>.toml`. When absent the
    /// reserved default name `"model"` is used (i.e. `<stem>.model.toml`).
    /// `Some("weathered")` selects `<stem>.weathered.toml`.
    #[serde(default)]
    pub variant: Option<String>,
    pub shape: MeshShape,
    /// RGB colour `[r, g, b]` in linear 0–1 range.
    pub colour: Vec<f32>,
    /// Sphere radius, or torus major radius. Ignored for `cuboid`.
    #[serde(default)]
    pub radius: f32,
    /// Full XYZ dimensions of a `cuboid` mesh.
    #[serde(default)]
    pub size: Option<[f32; 3]>,
    /// Tube radius for a `torus` mesh.
    #[serde(default)]
    pub minor_radius: f32,
    /// Emissive multiplier (the renderer multiplies `colour` by this and feeds
    /// the result into `StandardMaterial::emissive`). When `None`, the renderer
    /// applies its own default (typically `0.4` for general-purpose entities).
    #[serde(default)]
    pub emissive: Option<f32>,
    /// Uniform scale multiplier applied to the entity's transform.
    /// Affects both GLB models and procedural shapes.
    #[serde(default = "default_mesh_scale")]
    pub scale: f32,
    /// Euler rotation [x, y, z] in radians applied to the entity's transform.
    /// Affects both GLB models and procedural shapes.
    #[serde(default)]
    pub rotation: [f32; 3],
}

fn default_mesh_scale() -> f32 {
    1.0
}

/// Kind of a `[[light]]` entry: a point light or a directional light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Point,
    Directional,
}

/// One `[[light]]` entry from an entity template. Renderer-only data.
///
/// Replaces the per-section light fields that used to live on `[star]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightConfig {
    pub kind: LightKind,
    /// RGB colour `[r, g, b]` in linear 0–1 range.
    pub colour: [f32; 3],
    /// Light intensity (candela for point lights, illuminance for directional).
    pub intensity: f32,
    /// Range in world units. Required for point lights; ignored for directional.
    #[serde(default)]
    pub range: Option<f32>,
}

/// One entry in the `[[hull.console_hull]]` TOML array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleHullEntry {
    /// Console name matching the `Console` enum variant (e.g. `"Helm"`).
    pub console: crate::messages::Console,
    /// Maximum (and starting) HP for this console.
    pub max_hp: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HullConfig {
    /// Legacy single-value hull integrity (kept for backward compat with station/asteroid configs).
    #[serde(default)]
    pub hull_integrity: f32,
    /// HP for the single CaptainChair console slot (used by NPC ships).
    /// Takes precedence over `hull_integrity` when present.
    #[serde(default)]
    pub captain_chair: Option<f32>,
    /// Per-console hull entries. When present, replaces the single-value fields.
    #[serde(default)]
    pub console_hull: Vec<ConsoleHullEntry>,
    /// Number of repair teams available to this ship (default 0 = legacy).
    #[serde(default)]
    pub repair_team_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Ball,
    Capsule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColliderConfig {
    pub shape: ColliderShape,
    pub radius: f32,
    pub length: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppearanceConfig {
    pub colour: String,
    pub size_min: f32,
    pub size_max: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadarAppearanceConfig {
    pub colour: Vec<f32>,
    #[serde(default)]
    pub radius: Option<f32>,
    /// Authored world-space size override for radar rendering. When
    /// `None`, the entity's physical `radius` is used. Lets authors fudge
    /// radar visibility for tiny or oversized objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_size: Option<f32>,
    /// Radar icon to use for this entity's blip. One of `"ship"`,
    /// `"asteroid"`, `"station"`, `"planet"`, `"star"`, `"torpedo"`.
    /// When absent the server falls back to deriving the icon from the
    /// entity's tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmConsoleConfig {
    #[serde(default)]
    pub max_speed: f32,
    #[serde(default)]
    pub max_reverse_speed: f32,
    #[serde(default)]
    pub acceleration: f32,
    #[serde(default)]
    pub deceleration: f32,
    #[serde(default)]
    pub max_yaw_rate: f32,
    /// Radar configuration for the Helm radar widget, from
    /// `[helm_console.radar]`.
    #[serde(default)]
    pub radar: Option<crate::radar_config::RadarConfig>,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
    /// Total time in seconds to fully charge the impulse drive.
    /// Defaults to `IMPULSE_CHARGE_DURATION` (3.0 s) when absent.
    #[serde(default = "default_impulse_charge_duration")]
    pub impulse_charge_duration: f32,
    /// Speed multiplier applied when impulse drive is active.
    /// Defaults to `IMPULSE_SPEED_MULTIPLIER` (10.0) when absent.
    #[serde(default = "default_impulse_speed_multiplier")]
    pub impulse_speed_multiplier: f32,
    /// Acceleration multiplier applied while impulse drive is active.
    /// Defaults to `IMPULSE_ACCELERATION_MULTIPLIER` (5.0) when absent.
    #[serde(default = "default_impulse_acceleration_multiplier")]
    pub impulse_acceleration_multiplier: f32,
    /// Maximum visual banking (roll) angle in degrees when steering at full
    /// deflection. The ship leans into turns, lerped from 0 toward ±max_bank_deg
    /// based on steering input percentage. 0 = no banking.
    #[serde(default)]
    pub max_bank_deg: f32,
}

impl HelmConsoleConfig {
    /// Radar range from `[helm_console.radar] range`. Returns `0.0` when the
    /// `[helm_console.radar]` table is absent.
    pub fn effective_radar_range(&self) -> f32 {
        self.radar.as_ref().map_or(0.0, |r| r.range)
    }
}

fn default_impulse_charge_duration() -> f32 {
    crate::impulse::IMPULSE_CHARGE_DURATION
}

fn default_impulse_speed_multiplier() -> f32 {
    crate::impulse::IMPULSE_SPEED_MULTIPLIER
}

fn default_impulse_acceleration_multiplier() -> f32 {
    crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER
}

/// Stable identifier for a phaser bank, parsed verbatim from the TOML
/// `id` field on `[[weapons_console.phaser_banks]]`. Used on the wire
/// to address a specific bank (e.g. `FirePhaser { bank: "port" }`).
pub type PhaserBankId = String;

/// One `[[weapons_console.phaser_banks]]` entry. Defines a single phaser
/// bank's orientation on the ship, its full fire arc (used for manual
/// fire validity and for the radar arc overlay), its narrower auto-fire
/// arc (used by `console_ai` for autonomous firing decisions), and its
/// effective beam range.
///
/// `facing_deg` is the bank's centre bearing in ship-local degrees:
/// `0` = forward (−Z), `90` = starboard (+X), `180` = aft (+Z),
/// `-90` / `270` = port. Wraps freely; only the wrapped direction
/// matters.
///
/// `fire_arc_deg` is the full arc width centred on `facing_deg`. A bank
/// with `facing_deg = -90`, `fire_arc_deg = 180` covers the port
/// hemisphere from forward to aft. Values must be in `(0, 360]`.
///
/// `auto_arc_deg` is the (narrower) auto-fire window, also centred on
/// `facing_deg`. Must satisfy `0 < auto_arc_deg <= fire_arc_deg`.
///
/// `beam_range` is in world units. When `0.0`, the renderer/server
/// falls back to the parent `[weapons_console].beam_range`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaserBankConfig {
    pub id: PhaserBankId,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    pub auto_arc_deg: f32,
    #[serde(default)]
    pub beam_range: f32,
    /// Per-bank override for the fraction of beam damage that bypasses
    /// shields. When `None`, the parent `[weapons_console].shield_pierce`
    /// (or its default of `0.0`) is used. Clamped to `[0.0, 1.0]` at
    /// apply time.
    #[serde(default)]
    pub shield_pierce: Option<f32>,
    /// Optional rig-marker name linking this bank to a mount point in the
    /// model's rig sidecar (`[markers.<name>]`). When resolvable, downstream
    /// systems may use the marker's position/direction as the beam origin;
    /// when absent or unresolved they fall back to the hull-offset default.
    #[serde(default)]
    pub marker: Option<String>,
}

/// Stable identifier for a torpedo tube, parsed verbatim from the TOML
/// `id` field on `[[torpedoes.tubes]]`. Used on the wire to address a
/// specific tube (e.g. `FireTorpedo { tube: "fore_port" }`).
pub type TorpedoTubeId = String;

/// One `[[torpedoes.tubes]]` entry. Defines a single torpedo tube's
/// orientation and launch arc. Ammo is **not** per-tube; the entire
/// ship draws from the shared `[torpedoes].count` pool.
///
/// `facing_deg` and `fire_arc_deg` use the same convention as
/// [`PhaserBankConfig`] (ship-local degrees, 0 = forward).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorpedoTubeConfig {
    pub id: TorpedoTubeId,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    /// Per-tube load/unload time override in seconds. Falls back to the
    /// global `[torpedoes] load_time` when absent.
    #[serde(default)]
    pub load_time: Option<f32>,
    /// Optional rig-marker name linking this tube to a mount point in the
    /// model's rig sidecar (`[markers.<name>]`). When absent or unresolved,
    /// callers fall back to the ship-centre launch origin.
    #[serde(default)]
    pub marker: Option<String>,
}

/// Validate a `[[weapons_console.phaser_banks]]` list parsed from TOML.
///
/// Rejects:
///   - empty list (caller may decide to fall back to a single hardcoded
///     bank — this validator returns `Err` so callers see the empty list)
///   - duplicate `id` values
///   - `fire_arc_deg` outside `(0, 360]`
///   - `auto_arc_deg` outside `(0, fire_arc_deg]`
pub fn validate_phaser_banks(banks: &[PhaserBankConfig]) -> Result<(), String> {
    if banks.is_empty() {
        return Err("phaser_banks list is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for b in banks {
        if !seen.insert(b.id.as_str()) {
            return Err(format!("duplicate phaser bank id '{}'", b.id));
        }
        if !(b.fire_arc_deg > 0.0 && b.fire_arc_deg <= 360.0) {
            return Err(format!(
                "phaser bank '{}' has fire_arc_deg={} outside (0, 360]",
                b.id, b.fire_arc_deg
            ));
        }
        if !(b.auto_arc_deg > 0.0 && b.auto_arc_deg <= b.fire_arc_deg) {
            return Err(format!(
                "phaser bank '{}' has auto_arc_deg={} outside (0, fire_arc_deg={}]",
                b.id, b.auto_arc_deg, b.fire_arc_deg
            ));
        }
    }
    Ok(())
}

/// Validate a `[[torpedoes.tubes]]` list parsed from TOML.
///
/// Rejects: empty list, duplicate `id`, `fire_arc_deg` outside `(0, 360]`.
pub fn validate_torpedo_tubes(tubes: &[TorpedoTubeConfig]) -> Result<(), String> {
    if tubes.is_empty() {
        return Err("torpedo tubes list is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for t in tubes {
        if !seen.insert(t.id.as_str()) {
            return Err(format!("duplicate torpedo tube id '{}'", t.id));
        }
        if !(t.fire_arc_deg > 0.0 && t.fire_arc_deg <= 360.0) {
            return Err(format!(
                "torpedo tube '{}' has fire_arc_deg={} outside (0, 360]",
                t.id, t.fire_arc_deg
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponsConsoleConfig {
    #[serde(default)]
    pub beam_range: f32,
    #[serde(default)]
    pub beam_damage_per_sec: f32,
    #[serde(default)]
    pub beam_duration_secs: f32,
    #[serde(default)]
    pub cooldown_secs: f32,
    /// RGBA beam colour as a 4-element float array `[r, g, b, a]` in 0.0–1.0.
    /// When absent (empty vec), the renderer falls back to `beam_render::DEFAULT_BEAM_COLOR`.
    #[serde(default)]
    pub beam_color: Vec<f32>,
    /// RGBA colour used by the client Tactical UI for torpedo fire-arc
    /// overlays. When absent, the `ShipClientConfig` default is used.
    #[serde(default)]
    pub torpedo_arc_color: Vec<f32>,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
    /// Per-bank phaser definitions parsed from
    /// `[[weapons_console.phaser_banks]]`. Each bank has its own facing,
    /// fire arc, auto-fire arc, and range. Empty when the ship has no
    /// explicit per-bank loadout.
    #[serde(default)]
    pub phaser_banks: Vec<PhaserBankConfig>,
    /// Fraction of phaser-beam damage that bypasses shields and applies
    /// directly to the hull. Default `0.0` — all damage is mitigated by
    /// the facing shield quadrant. Used as the global value for NPC
    /// phasers (which do not have per-bank config) and as the fallback
    /// for player banks whose `[[weapons_console.phaser_banks]].shield_pierce`
    /// is unset.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Radar configuration for the Tactical console radar widget, from
    /// `[weapons_console.radar]`.
    #[serde(default)]
    pub radar: Option<crate::radar_config::RadarConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringConsoleConfig {
    /// Path to a complexity TOML file for this console.
    ///
    /// NOTE: repair pacing is configured by the top-level `[repair]` block
    /// (`RepairConfig`), not here.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptainConsoleConfig {
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerConfigSection {
    pub capacity: f32,
    pub rates: [f32; 6],
    pub emergency_threshold: f32,
    /// Path to a complexity TOML file for the Power console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

/// Config block for the Shields console focus bonuses/penalties.
///
/// Loaded from `[shields_console]` in `player_ship.toml`. The nested
/// `[shields_console.base]` sub-block (modelled by [`ShieldsBaseConfig`])
/// supplies the underlying shield-system base values (number of facings,
/// max HP, regen, offline duration) that were previously hardcoded by
/// `ShieldConfig::default()` at `src/weapons/shield.rs:50-58`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsConsoleConfig {
    /// Extra max HP applied to the focused facing.
    #[serde(default = "default_focus_bonus_max_hp")]
    pub focus_bonus_max_hp: i32,
    /// Extra regen per second applied to the focused facing.
    #[serde(default = "default_focus_bonus_regen")]
    pub focus_bonus_regen: f32,
    /// Max HP subtracted from each non-focused facing.
    #[serde(default = "default_focus_penalty_max_hp")]
    pub focus_penalty_max_hp: i32,
    /// Regen per second subtracted from each non-focused facing.
    #[serde(default = "default_focus_penalty_regen")]
    pub focus_penalty_regen: f32,
    /// HP per second decay applied to non-focused facings when above reduced max.
    #[serde(default = "default_focus_decay_rate")]
    pub focus_decay_rate: f32,
    /// Base shield-system values (number of facings, max HP, regen,
    /// offline duration). When absent the historical hardcoded defaults
    /// from `ShieldConfig::default()` are used.
    #[serde(default)]
    pub base: Option<ShieldsBaseConfig>,
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

fn default_focus_bonus_max_hp() -> i32 {
    50
}
fn default_focus_bonus_regen() -> f32 {
    5.0
}
fn default_focus_penalty_max_hp() -> i32 {
    25
}
fn default_focus_penalty_regen() -> f32 {
    2.5
}
fn default_focus_decay_rate() -> f32 {
    10.0
}

impl Default for ShieldsConsoleConfig {
    fn default() -> Self {
        Self {
            focus_bonus_max_hp: default_focus_bonus_max_hp(),
            focus_bonus_regen: default_focus_bonus_regen(),
            focus_penalty_max_hp: default_focus_penalty_max_hp(),
            focus_penalty_regen: default_focus_penalty_regen(),
            focus_decay_rate: default_focus_decay_rate(),
            base: None,
            complexity_toml: None,
        }
    }
}

/// Base shield-system values loaded from `[shields_console.base]`.
///
/// These map 1:1 onto `crate::shield::ShieldConfig` (the runtime struct
/// consumed by `ShieldSystem::new`). All fields default to the historical
/// hardcoded values from `ShieldConfig::default()` so omitting the block
/// changes nothing.
///
/// `num_facings` is exposed for symmetry but the client panel UI assumes
/// 4 quadrants — values other than 4 will break the Shields panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsBaseConfig {
    /// Number of equally-spaced shield facings. The client panel UI
    /// assumes 4 (fore/port/aft/starboard); other values will not render
    /// correctly.
    #[serde(default = "default_shields_num_facings")]
    pub num_facings: usize,
    /// Maximum HP per facing.
    #[serde(default = "default_shields_max_hp")]
    pub max_hp: i32,
    /// HP regenerated per second per online facing.
    #[serde(default = "default_shields_regen_per_sec")]
    pub regen_per_sec: f32,
    /// Seconds a facing stays offline after its HP is depleted.
    #[serde(default = "default_shields_offline_duration")]
    pub offline_duration: f32,
}

fn default_shields_num_facings() -> usize {
    4
}
fn default_shields_max_hp() -> i32 {
    100
}
fn default_shields_regen_per_sec() -> f32 {
    5.0
}
fn default_shields_offline_duration() -> f32 {
    10.0
}

impl Default for ShieldsBaseConfig {
    fn default() -> Self {
        Self {
            num_facings: default_shields_num_facings(),
            max_hp: default_shields_max_hp(),
            regen_per_sec: default_shields_regen_per_sec(),
            offline_duration: default_shields_offline_duration(),
        }
    }
}

impl ShieldsBaseConfig {
    /// Convert this TOML config into a runtime `ShieldConfig`.
    pub fn to_runtime(&self) -> crate::shield::ShieldConfig {
        crate::shield::ShieldConfig {
            num_facings: self.num_facings,
            max_hp: self.max_hp,
            regen_per_sec: self.regen_per_sec,
            offline_duration: self.offline_duration,
        }
    }
}

/// Player-ship phaser combat tuning, derived from the existing flat fields
/// on `[weapons_console]` (`beam_range`, `beam_damage_per_sec`,
/// `beam_duration_secs`, `cooldown_secs`).
///
/// `PhaserCombatConfig` is the player-path source of truth, installed
/// as a Bevy resource by `WeaponsPlugin` (defaults match the constants)
/// and overridden in `spawn_game_start_entities` from the player ship's
/// `[weapons_console]` block.
///
/// NOTE: Per-bank arc/facing fields are defined on
/// [`PhaserBankConfig`] (parsed from `[[weapons_console.phaser_banks]]`).
/// This flat config is being phased out in favour of the per-bank list;
/// it currently remains the seed for shared timings until the weapons
/// server is migrated to consume the per-bank list directly.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaserCombatConfig {
    /// Effective player phaser range in world units. Used as the fallback
    /// when an individual bank's `beam_range` is 0.0.
    pub phaser_range: f32,
    /// Active beam duration in seconds (how long a beam stays on a target).
    pub beam_duration_secs: f32,
    /// Post-beam cooldown in seconds. Shared by every bank.
    pub beam_cooldown_secs: f32,
    /// Damage applied to the target per second of active beam.
    pub beam_damage_per_sec: f32,
    /// Per-bank facing/arc/range list, parsed from
    /// `[[weapons_console.phaser_banks]]` in TOML order. Empty if the ship
    /// has no banks configured (e.g. NPC defaults). The Tactical UI also
    /// receives a stripped subset of these via `PhaserBankClientConfig`.
    pub banks: Vec<PhaserBankConfig>,
}

impl PhaserCombatConfig {
    /// Canonical baseline phaser values, used when the ship TOML omits (or
    /// zeroes) the corresponding `[weapons_console]` field. Other modules
    /// needing the baseline (e.g. `weapons_plugin::BEAM_DAMAGE_PER_SEC`)
    /// alias these rather than restating the numbers.
    pub const DEFAULT_PHASER_RANGE: f32 = 40.0;
    pub const DEFAULT_BEAM_DURATION_SECS: f32 = 6.0;
    pub const DEFAULT_BEAM_COOLDOWN_SECS: f32 = 6.0;
    pub const DEFAULT_BEAM_DAMAGE_PER_SEC: f32 = 5.0;
}

impl Default for PhaserCombatConfig {
    fn default() -> Self {
        Self {
            phaser_range: Self::DEFAULT_PHASER_RANGE,
            beam_duration_secs: Self::DEFAULT_BEAM_DURATION_SECS,
            beam_cooldown_secs: Self::DEFAULT_BEAM_COOLDOWN_SECS,
            beam_damage_per_sec: Self::DEFAULT_BEAM_DAMAGE_PER_SEC,
            banks: Vec::new(),
        }
    }
}

impl PhaserCombatConfig {
    /// Build a `PhaserCombatConfig` from a parsed `[weapons_console]`
    /// block, falling back to `PhaserCombatConfig::default()` for any
    /// field whose TOML value is `<= 0.0` (the same "zero means absent"
    /// convention used by the NPC phaser path at
    /// `src/console/weapons/server.rs:330-337`).
    pub fn from_weapons_console(wc: &WeaponsConsoleConfig) -> Self {
        let default = Self::default();
        Self {
            phaser_range: if wc.beam_range > 0.0 {
                wc.beam_range
            } else {
                default.phaser_range
            },
            beam_duration_secs: if wc.beam_duration_secs > 0.0 {
                wc.beam_duration_secs
            } else {
                default.beam_duration_secs
            },
            beam_cooldown_secs: if wc.cooldown_secs > 0.0 {
                wc.cooldown_secs
            } else {
                default.beam_cooldown_secs
            },
            beam_damage_per_sec: if wc.beam_damage_per_sec > 0.0 {
                wc.beam_damage_per_sec
            } else {
                default.beam_damage_per_sec
            },
            banks: wc.phaser_banks.clone(),
        }
    }
}

/// Config block for the repair-team state machine in a ship TOML.
///
/// Loaded from `[repair]` in `player_ship.toml` (and any NPC ship TOML
/// that wishes to override repair pacing). All fields are optional; missing
/// fields fall back to the same defaults as `RepairTimings::default()` and
/// to the historical hardcoded constants (`TRAVEL_DURATION = 5.0`,
/// `REPAIR_RATE_HP_PER_SEC = 0.5`).
///
/// The same values are forwarded to the client via
/// `ShipClientConfig.repair_travel_secs` and
/// `ShipClientConfig.repair_rate_hp_per_sec` so that the Repair panel UI
/// can derive its progress-bar timings without redefining the constants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairConfig {
    /// Seconds a team spends travelling to a console (and the same again returning).
    #[serde(default = "default_repair_travel_duration_secs")]
    pub travel_duration_secs: f32,
    /// HP restored per second while a team is at a console.
    #[serde(default = "default_repair_rate_hp_per_sec")]
    pub repair_rate_hp_per_sec: f32,
}

fn default_repair_travel_duration_secs() -> f32 {
    5.0
}
fn default_repair_rate_hp_per_sec() -> f32 {
    0.5
}

impl Default for RepairConfig {
    fn default() -> Self {
        Self {
            travel_duration_secs: default_repair_travel_duration_secs(),
            repair_rate_hp_per_sec: default_repair_rate_hp_per_sec(),
        }
    }
}

impl RepairConfig {
    /// Convert this TOML config into a runtime `RepairTimings`.
    pub fn to_runtime(&self) -> crate::repair_teams::RepairTimings {
        crate::repair_teams::RepairTimings {
            travel_duration: self.travel_duration_secs,
            repair_rate_hp_per_sec: self.repair_rate_hp_per_sec,
        }
    }
}

/// Config block for an entity's comms range.
///
/// Loaded from `[comms]` in entity TOMLs. When present, the entity is
/// reachable by the player's Comms console while inside `range` units of the
/// player ship. The player ship's own `[comms].range` defines how far it can
/// listen. Effective range between two entities is `min(a.range, b.range)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsConfig {
    /// Comms range in world units.
    pub range: f32,
}

/// Config block for the torpedo system in a ship TOML.
///
/// Loaded from `[torpedoes]` in `player_ship.toml` (and any NPC ship TOML
/// that wishes to override the torpedo loadout). All fields are optional;
/// missing fields fall back to the same defaults as `TorpedoConfig::default()`.
///
/// `turn_rate_deg_per_sec` is in **degrees per second** for designer
/// readability; it is converted to radians by `to_runtime()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorpedoesConfig {
    #[serde(default = "default_torpedo_count")]
    pub count: u32,
    #[serde(default = "default_torpedo_damage_hull")]
    pub damage_hull: i32,
    #[serde(default = "default_torpedo_damage_shields")]
    pub damage_shields: i32,
    #[serde(default = "default_torpedo_speed")]
    pub speed: f32,
    /// Maximum turn rate in **degrees per second** (homing).
    /// Converted to radians by `to_runtime()`.
    #[serde(default = "default_torpedo_turn_rate_deg_per_sec")]
    pub turn_rate_deg_per_sec: f32,
    #[serde(default = "default_torpedo_lifespan")]
    pub lifespan: f32,
    #[serde(default = "default_torpedo_load_time")]
    pub load_time: f32,
    /// Proximity-detonation radius in world units.
    #[serde(default = "default_torpedo_detonation_radius")]
    pub detonation_radius: f32,
    /// Fraction of `damage_shields` that bypasses shields and adds to
    /// hull damage at detonation. Default `0.0` — `damage_shields` is
    /// fully absorbed by the facing shield quadrant. `damage_hull` is
    /// unaffected (always hits hull). Clamped to `[0.0, 1.0]` at apply
    /// time.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Per-tube torpedo definitions parsed from `[[torpedoes.tubes]]`.
    /// Each tube has its own facing and fire arc. Ammo is shared via
    /// the top-level `count` field. Empty when the ship has no explicit
    /// per-tube loadout.
    #[serde(default)]
    pub tubes: Vec<TorpedoTubeConfig>,
}

fn default_torpedo_count() -> u32 {
    10
}
fn default_torpedo_damage_hull() -> i32 {
    50
}
fn default_torpedo_damage_shields() -> i32 {
    5
}
fn default_torpedo_speed() -> f32 {
    30.0
}
fn default_torpedo_turn_rate_deg_per_sec() -> f32 {
    45.0
}
fn default_torpedo_lifespan() -> f32 {
    20.0
}
fn default_torpedo_load_time() -> f32 {
    10.0
}
fn default_torpedo_detonation_radius() -> f32 {
    5.0
}

impl Default for TorpedoesConfig {
    fn default() -> Self {
        Self {
            count: default_torpedo_count(),
            damage_hull: default_torpedo_damage_hull(),
            damage_shields: default_torpedo_damage_shields(),
            speed: default_torpedo_speed(),
            turn_rate_deg_per_sec: default_torpedo_turn_rate_deg_per_sec(),
            lifespan: default_torpedo_lifespan(),
            load_time: default_torpedo_load_time(),
            detonation_radius: default_torpedo_detonation_radius(),
            shield_pierce: 0.0,
            tubes: Vec::new(),
        }
    }
}

impl TorpedoesConfig {
    /// Convert this TOML config into a runtime `TorpedoConfig`.
    /// Performs the degrees → radians conversion on `turn_rate_deg_per_sec`.
    pub fn to_runtime(&self) -> crate::torpedo::TorpedoConfig {
        crate::torpedo::TorpedoConfig {
            count: self.count,
            damage_hull: self.damage_hull,
            damage_shields: self.damage_shields,
            speed: self.speed,
            turn_rate: self.turn_rate_deg_per_sec.to_radians(),
            lifespan: self.lifespan,
            load_time: self.load_time,
            detonation_radius: self.detonation_radius,
            shield_pierce: self.shield_pierce,
        }
    }
}

/// Config block for the Navigation console in a ship TOML.
///
/// Loaded from `[navigation_console]` in `player_ship.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationConsoleConfig {
    /// System chart radar config for the Navigation console.
    #[serde(default)]
    pub system_chart: crate::radar_config::RadarConfig,
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

/// Config block for the Sensors console in a ship TOML.
///
/// Loaded from `[sensors_console]` in `player_ship.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorsConsoleConfig {
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Long-range radar config for the Sensors console.
    #[serde(default)]
    pub long_range_radar: crate::radar_config::RadarConfig,
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EntityConfig {
    /// Display name (top-level scalar). Informational for most entities; used
    /// by triggers/comms to identify named instances.
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub hull: Option<HullConfig>,
    pub collider: Option<ColliderConfig>,
    pub appearance: Option<AppearanceConfig>,
    pub helm_console: Option<HelmConsoleConfig>,
    pub weapons_console: Option<WeaponsConsoleConfig>,
    pub engineering_console: Option<EngineeringConsoleConfig>,
    pub captain_console: Option<CaptainConsoleConfig>,
    pub power: Option<PowerConfigSection>,
    pub sensors_console: Option<SensorsConsoleConfig>,
    pub navigation_console: Option<NavigationConsoleConfig>,
    /// Shields console focus config.
    pub shields_console: Option<ShieldsConsoleConfig>,
    /// Torpedo system config (player ship and any NPC ship with torpedoes).
    pub torpedoes: Option<TorpedoesConfig>,
    /// Repair team timings (travel duration, repair rate).
    pub repair: Option<RepairConfig>,
    /// Comms range — when present, the entity can send/receive comms within
    /// this radius of the player ship.
    pub comms: Option<CommsConfig>,
    /// Asteroid field section from entity template (donut params, grid, etc.)
    pub asteroid_field: Option<AsteroidFieldConfig>,
    /// Region shape section — present for region entities.
    pub shape: Option<RegionShape>,
    /// Region effects section — present for region entities with effects.
    pub effects: Option<RegionEffectsConfig>,
    /// Optional faction UUID this entity belongs to.
    #[serde(default)]
    pub faction: Option<Uuid>,
    /// Optional AI behaviour controller config.
    #[serde(default)]
    pub behaviour: Option<BehaviourConfig>,
    /// Radar appearance (colour, optional radius) for the helm radar blip.
    #[serde(default)]
    pub radar_appearance: Option<RadarAppearanceConfig>,
    /// Targetability section. When absent the entity is not targetable.
    #[serde(default)]
    pub target: Option<crate::entity_target::TargetSection>,
    /// 3-D mesh definition. When present the entity receives a visual on the viewscreen.
    #[serde(default)]
    pub mesh: Option<MeshConfig>,
    /// Renderer light sources attached to this entity.
    #[serde(default)]
    pub light: Vec<LightConfig>,
}

impl EntityConfig {
    /// `(console, complexity_toml path)` for every console config that
    /// references a complexity TOML.
    pub fn complexity_toml_by_console(&self) -> Vec<(crate::messages::Console, &str)> {
        use crate::messages::Console;
        let refs = [
            (
                Console::Helm,
                self.helm_console
                    .as_ref()
                    .and_then(|c| c.complexity_toml.as_deref()),
            ),
            (
                Console::Tactical,
                self.weapons_console
                    .as_ref()
                    .and_then(|c| c.complexity_toml.as_deref()),
            ),
            (
                Console::Repair,
                self.engineering_console
                    .as_ref()
                    .and_then(|c| c.complexity_toml.as_deref()),
            ),
            (
                Console::CaptainChair,
                self.captain_console
                    .as_ref()
                    .and_then(|c| c.complexity_toml.as_deref()),
            ),
            (
                Console::Sensors,
                self.sensors_console
                    .as_ref()
                    .and_then(|c| c.complexity_toml.as_deref()),
            ),
            (
                Console::Shields,
                self.shields_console
                    .as_ref()
                    .and_then(|c| c.complexity_toml.as_deref()),
            ),
            (
                Console::Navigation,
                self.navigation_console
                    .as_ref()
                    .and_then(|c| c.complexity_toml.as_deref()),
            ),
            (
                Console::Power,
                self.power
                    .as_ref()
                    .and_then(|c| c.complexity_toml.as_deref()),
            ),
        ];
        refs.into_iter()
            .filter_map(|(console, path)| path.map(|p| (console, p)))
            .collect()
    }

    /// Collect all `complexity_toml` paths referenced by any console config.
    pub fn complexity_toml_paths(&self) -> Vec<String> {
        self.complexity_toml_by_console()
            .into_iter()
            .map(|(_, p)| p.to_string())
            .collect()
    }

    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        let mut value: toml::Value = toml::from_str(s)?;
        if let Some(table) = value.as_table_mut() {
            // `[stations]` is parsed from the same ship TOML by
            // `lobby::stations_config` with its own schema; drop it so
            // `deny_unknown_fields` doesn't reject ship templates.
            table.remove("stations");
        }
        let mut config: EntityConfig = value.try_into()?;

        // Validation: region entity with effects but no shape is an error.
        if let Some(ref effects) = config.effects {
            if !effects.is_empty() && config.shape.is_none() {
                return Err(SerdeError::custom(
                    "region entity has effects but no [shape] section",
                ));
            }
        }

        // Clamp target_speed in every StateConfig entry.
        if let Some(ref mut b) = config.behaviour {
            for s in &mut b.state {
                s.clamp();
            }
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]

    use super::*;
    use crate::entity_tags::EntityTag;

    #[test]
    fn all_sections_present_deserializes_to_some() {
        let toml_str = r##"
tags = ["gameplay", "combat", "primary"]

[hull]
hull_integrity = 100

[collider]
shape = "Ball"
radius = 2.0
length = 0.0

[appearance]
colour = "#ff0000"
size_min = 1.0
size_max = 3.0

[helm_console]
max_speed = 50.0
max_reverse_speed = 25.0
acceleration = 16.7
deceleration = 50.0
max_yaw_rate = 0.785

[helm_console.radar]
range = 50.0
shows = ["asteroid"]

[weapons_console]
beam_range = 40.0
beam_damage_per_sec = 5.0
beam_duration_secs = 6.0
cooldown_secs = 6.0

[engineering_console]

[captain_console]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");

        assert_eq!(
            config.tags,
            vec![
                "gameplay".to_string(),
                "combat".to_string(),
                "primary".to_string()
            ]
        );

        assert!(config.hull.is_some());
        assert!((config.hull.as_ref().unwrap().hull_integrity - 100.0).abs() < 1e-6);

        assert!(config.collider.is_some());
        let c = config.collider.as_ref().unwrap();
        assert_eq!(c.shape, ColliderShape::Ball);
        assert_eq!(c.radius, 2.0);

        assert!(config.appearance.is_some());
        assert_eq!(config.appearance.as_ref().unwrap().colour, "#ff0000");

        assert!(config.helm_console.is_some());
        let h = config.helm_console.as_ref().unwrap();
        assert_eq!(h.max_speed, 50.0);
        assert_eq!(h.effective_radar_range(), 50.0);

        assert!(config.weapons_console.is_some());
        let w = config.weapons_console.as_ref().unwrap();
        assert_eq!(w.beam_range, 40.0);
        assert_eq!(w.cooldown_secs, 6.0);

        assert!(config.engineering_console.is_some());

        assert!(config.captain_console.is_some());
    }

    #[test]
    fn only_hull_and_tags_produces_none_for_console_fields() {
        let toml_str = r##"
tags = ["gameplay", "asteroid"]

[hull]
hull_integrity = 80
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");

        assert_eq!(
            config.tags,
            vec!["gameplay".to_string(), "asteroid".to_string()]
        );
        assert!(config.hull.is_some());
        assert!((config.hull.as_ref().unwrap().hull_integrity - 80.0).abs() < 1e-6);
        assert!(config.collider.is_none());
        assert!(config.appearance.is_none());
        assert!(config.helm_console.is_none());
        assert!(config.weapons_console.is_none());
        assert!(config.engineering_console.is_none());
        assert!(config.captain_console.is_none());
        assert!(
            config.radar_appearance.is_none(),
            "radar_appearance should default to None when not in TOML"
        );
    }

    #[test]
    fn malformed_field_returns_error() {
        let toml_str = r##"
[hull]
hull_integrity = "not_an_integer"
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn tags_field_deserializes_to_vec_string() {
        let toml_str = r##"
tags = ["foo", "bar", "baz", "quux"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(
            config.tags,
            vec![
                "foo".to_string(),
                "bar".to_string(),
                "baz".to_string(),
                "quux".to_string()
            ]
        );
    }

    #[test]
    fn collider_capsule_shape_round_trips() {
        let toml_str = r##"
[collider]
shape = "Capsule"
radius = 1.5
length = 6.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(
            config.collider.as_ref().unwrap().shape,
            ColliderShape::Capsule
        );
    }

    #[test]
    fn empty_toml_string_produces_all_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.tags.is_empty());
        assert!(config.hull.is_none());
        assert!(config.collider.is_none());
        assert!(config.appearance.is_none());
        assert!(config.helm_console.is_none());
        assert!(config.weapons_console.is_none());
        assert!(config.engineering_console.is_none());
        assert!(config.captain_console.is_none());
        assert!(
            config.radar_appearance.is_none(),
            "radar_appearance should default to None"
        );
    }

    #[test]
    fn helm_console_partial_fields_work() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(h.max_speed, 30.0);
        assert_eq!(h.max_reverse_speed, 0.0);
    }

    #[test]
    fn helm_console_radar_table_parses_into_nested_field() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.radar]
range = 750.0
shows = ["asteroid", "ship"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        let radar = h.radar.as_ref().expect("helm_console.radar must parse");
        assert_eq!(radar.range, 750.0);
        assert_eq!(h.effective_radar_range(), 750.0);
    }

    #[test]
    fn helm_console_effective_radar_range_zero_when_no_radar_table() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert!(h.radar.is_none());
        assert_eq!(h.effective_radar_range(), 0.0);
    }

    #[test]
    fn weapons_console_beam_color_parses_rgba() {
        let toml_str = r##"
[weapons_console]
beam_color = [1.0, 0.5, 0.2, 0.9]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert_eq!(w.beam_color, vec![1.0, 0.5, 0.2, 0.9]);
    }

    #[test]
    fn weapons_console_beam_color_defaults_to_empty_when_omitted() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert!(
            w.beam_color.is_empty(),
            "beam_color should default to empty vec when omitted"
        );
    }

    // ── Power section tests ────────────────────────────────────────────────

    #[test]
    fn power_section_parses_capacity_rates_emergency_threshold() {
        let toml_str = r##"
[power]
capacity = 150.0
rates = [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]
emergency_threshold = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let p = config.power.expect("power must be Some");
        assert!((p.capacity - 150.0).abs() < 0.001);
        assert_eq!(p.rates, [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]);
        assert!((p.emergency_threshold - 30.0).abs() < 0.001);
    }

    #[test]
    fn power_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(
            config.power.is_none(),
            "power should be None when not specified"
        );
    }

    #[test]
    fn sensors_console_parses_with_long_range_radar() {
        let toml_str = r##"
tags = ["player", "ship"]

[sensors_console]
power_multipliers = [-0.5, 0.0, 0.25, 0.5]

[sensors_console.long_range_radar]
range = 200.0
shows = ["region", "asteroid_field", "asteroid", "ship"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sensors = config
            .sensors_console
            .expect("sensors_console must be Some");
        assert_eq!(sensors.power_multipliers, Some([-0.5, 0.0, 0.25, 0.5]));
        assert_eq!(sensors.long_range_radar.range, 200.0);
        assert!(sensors.long_range_radar.shows.contains(&EntityTag::Region));
        assert!(sensors
            .long_range_radar
            .shows
            .contains(&EntityTag::AsteroidField));
        assert!(sensors
            .long_range_radar
            .shows
            .contains(&EntityTag::Asteroid));
    }

    #[test]
    fn sensors_console_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.sensors_console.is_none());
    }

    #[test]
    fn helm_console_power_multipliers_parses() {
        let toml_str = r##"
[helm_console]
power_multipliers = [-0.8, 0.0, 0.4, 0.8]
max_speed = 50.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(h.power_multipliers, Some([-0.8, 0.0, 0.4, 0.8]));
    }

    #[test]
    fn weapons_console_power_multipliers_parses() {
        let toml_str = r##"
[weapons_console]
power_multipliers = [-0.3, 0.0, 0.15, 0.3]
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert_eq!(w.power_multipliers, Some([-0.3, 0.0, 0.15, 0.3]));
    }

    #[test]
    fn power_multipliers_defaults_to_none_when_omitted() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert!(h.power_multipliers.is_none());
    }

    // ── Complexity TOML reference tests ────────────────────────────────────

    #[test]
    fn weapons_console_complexity_toml_parses() {
        let toml_str = r##"
[weapons_console]
complexity_toml = "assets/complexity/tactical.toml"
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert_eq!(
            w.complexity_toml.as_deref(),
            Some("assets/complexity/tactical.toml")
        );
    }

    #[test]
    fn helm_console_complexity_toml_parses() {
        let toml_str = r##"
[helm_console]
complexity_toml = "assets/complexity/helm.toml"
max_speed = 50.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(
            h.complexity_toml.as_deref(),
            Some("assets/complexity/helm.toml")
        );
    }

    #[test]
    fn engineering_console_complexity_toml_parses() {
        let toml_str = r##"
[engineering_console]
complexity_toml = "assets/complexity/repair.toml"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let e = config
            .engineering_console
            .expect("engineering_console must be Some");
        assert_eq!(
            e.complexity_toml.as_deref(),
            Some("assets/complexity/repair.toml")
        );
    }

    #[test]
    fn captain_console_complexity_toml_parses() {
        let toml_str = r##"
[captain_console]
complexity_toml = "assets/complexity/captain.toml"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let c = config
            .captain_console
            .expect("captain_console must be Some");
        assert_eq!(
            c.complexity_toml.as_deref(),
            Some("assets/complexity/captain.toml")
        );
    }

    #[test]
    fn complexity_toml_defaults_to_none_when_omitted() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.weapons_console.is_none());
    }

    /// Every shipped entity template must parse under the strict
    /// (`deny_unknown_fields`) schema. Catches both schema drift in the code
    /// and typo'd keys in the TOMLs.
    #[test]
    fn all_shipped_entity_templates_parse_strictly() {
        let dir = std::path::Path::new("assets/entities");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).expect("assets/entities must exist") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("template must be readable");
            EntityConfig::from_toml(&text).unwrap_or_else(|e| {
                panic!(
                    "entity template {} failed strict parse: {e}",
                    path.display()
                )
            });
            checked += 1;
        }
        assert!(
            checked > 0,
            "no entity templates found in {}",
            dir.display()
        );
    }

    /// Unknown keys in an entity TOML must be rejected, not silently ignored.
    #[test]
    fn unknown_section_and_field_are_rejected() {
        assert!(EntityConfig::from_toml("[helm_consol]\nmax_speed = 1.0").is_err());
        assert!(EntityConfig::from_toml("[helm_console]\nmax_sped = 1.0").is_err());
    }

    #[test]
    fn weapons_console_without_complexity_toml_defaults_to_none() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert!(w.complexity_toml.is_none());
    }

    #[test]
    fn complexity_toml_paths_returns_multiple_when_several_consoles_referenced() {
        let toml_str = r##"
[helm_console]
complexity_toml = "assets/complexity/helm.toml"
[weapons_console]
complexity_toml = "assets/complexity/tactical.toml"
[engineering_console]
complexity_toml = "assets/complexity/repair.toml"
[captain_console]
complexity_toml = "assets/complexity/captain.toml"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let paths = config.complexity_toml_paths();
        assert_eq!(paths.len(), 4);
        assert!(paths.contains(&"assets/complexity/helm.toml".to_string()));
        assert!(paths.contains(&"assets/complexity/tactical.toml".to_string()));
        assert!(paths.contains(&"assets/complexity/repair.toml".to_string()));
        assert!(paths.contains(&"assets/complexity/captain.toml".to_string()));
    }

    #[test]
    fn complexity_toml_paths_returns_empty_when_no_complexity_refs() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.complexity_toml_paths().is_empty());
    }

    // ── AsteroidField section tests ────────────────────────────────────────

    #[test]
    fn asteroid_field_section_parses_from_template() {
        let toml_str = r##"
tags = ["field", "main"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 150.0
despawn_distance = 250.0
asteroid_type_paths = ["assets/entities/asteroid_small.toml", "assets/entities/asteroid_large.toml"]
cosmetic_type_paths = ["assets/entities/asteroid_cosmetic.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!((field.inner_radius - 100.0).abs() < 1e-6);
        assert_eq!(field.asteroid_type_paths.len(), 2);
        assert_eq!(field.cosmetic_type_paths.len(), 1);
    }

    #[test]
    fn asteroid_field_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.asteroid_field.is_none());
    }

    #[test]
    fn asteroid_field_shape_defaults_to_none_when_omitted() {
        // Back-compat: TOMLs that pre-date the `shape` field must continue
        // to deserialise unchanged, with `shape = None`.
        let toml_str = r##"
[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!(field.shape.is_none());
    }

    #[test]
    fn asteroid_field_anchor_parses_as_optional_string() {
        // PRD #397 fix 5: `[asteroid_field] anchor = "name"` carries the
        // reference verbatim. The serde-skipped `anchor_offset` defaults
        // to `[0,0,0]` and is filled in at spawn time against the world's
        // anchor table.
        let toml_str = r##"
[asteroid_field]
shape = "torus"
anchor = "belt_origin"
inner_radius = 300.0
outer_radius = 350.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert_eq!(field.anchor.as_deref(), Some("belt_origin"));
        assert_eq!(
            field.anchor_offset,
            [0.0, 0.0, 0.0],
            "anchor_offset is serde-skipped and defaults to origin until spawn-time resolution"
        );
    }

    #[test]
    fn asteroid_field_anchor_omitted_defaults_to_none() {
        // Regression guard: existing TOML without an `anchor` key must keep
        // `anchor = None` and `anchor_offset = [0,0,0]` (legacy behaviour).
        let toml_str = r##"
[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!(field.anchor.is_none(), "missing anchor key → None");
        assert_eq!(field.anchor_offset, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn asteroid_field_shape_torus_parses() {
        // Schema: `shape = "torus"` as a sibling of `inner_radius`/`outer_radius`.
        let toml_str = r##"
[asteroid_field]
shape = "torus"
inner_radius = 300.0
outer_radius = 350.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert_eq!(
            field.shape,
            Some(crate::entity_config::AsteroidFieldShape::Torus)
        );
        assert!((field.inner_radius - 300.0).abs() < 1e-6);
        assert!((field.outer_radius - 350.0).abs() < 1e-6);
    }

    #[test]
    fn asteroid_field_shape_unknown_value_errors() {
        let toml_str = r##"
[asteroid_field]
shape = "donut"
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(
            result.is_err(),
            "unknown shape variant must be a parse error"
        );
    }

    // ── name / mesh.emissive / [[light]] tests (PRD: schema refactor slice 3) ──

    #[test]
    fn name_field_parses() {
        let toml_str = r#"name = "Sun""#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.name.as_deref(), Some("Sun"));
    }

    #[test]
    fn name_field_defaults_to_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.name.is_none());
    }

    #[test]
    fn mesh_emissive_field_parses() {
        let toml_str = r##"
[mesh]
shape = "sphere"
colour = [1.0, 0.8, 0.0]
radius = 50.0
emissive = 2.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let mesh = config.mesh.expect("mesh must be Some");
        assert_eq!(mesh.emissive, Some(2.0));
    }

    #[test]
    fn mesh_emissive_defaults_to_none() {
        let toml_str = r##"
[mesh]
shape = "sphere"
colour = [1.0, 1.0, 1.0]
radius = 1.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let mesh = config.mesh.expect("mesh must be Some");
        assert!(mesh.emissive.is_none());
    }

    #[test]
    fn light_array_parses_multiple_entries() {
        let toml_str = r##"
[[light]]
kind = "point"
colour = [1.0, 0.95, 0.85]
intensity = 150000.0
range = 5000.0

[[light]]
kind = "point"
colour = [0.5, 0.5, 1.0]
intensity = 1000.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.light.len(), 2);
        assert_eq!(config.light[0].kind, LightKind::Point);
        assert_eq!(config.light[0].colour, [1.0, 0.95, 0.85]);
        assert!((config.light[0].intensity - 150000.0).abs() < 1e-3);
        assert_eq!(config.light[0].range, Some(5000.0));
        assert_eq!(config.light[1].range, None);
    }

    #[test]
    fn light_defaults_to_empty_vec() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.light.is_empty());
    }

    #[test]
    fn light_directional_kind_parses() {
        let toml_str = r##"
[[light]]
kind = "directional"
colour = [1.0, 1.0, 1.0]
intensity = 10000.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.light.len(), 1);
        assert_eq!(config.light[0].kind, LightKind::Directional);
    }

    // ── Region shape tests ───────────────────────────────────────────────

    #[test]
    fn region_shape_sphere_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "sphere"
radius = 100.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(
            shape,
            crate::region_shape::RegionShape::Sphere { radius: 100.0 }
        );
    }

    #[test]
    fn region_shape_box_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "box"
half_extents = [50.0, 30.0, 40.0]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(
            shape,
            crate::region_shape::RegionShape::Box {
                half_extents: [50.0, 30.0, 40.0],
                yaw: 0.0
            }
        );
    }

    #[test]
    fn region_shape_torus_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "torus"
inner_radius = 50.0
outer_radius = 80.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(
            shape,
            crate::region_shape::RegionShape::Torus {
                inner_radius: 50.0,
                outer_radius: 80.0
            }
        );
    }

    #[test]
    fn region_shape_parses_with_effects() {
        let toml_str = r##"
tags = ["region", "nebula"]

[shape]
type = "sphere"
radius = 150.0

[effects]
[effects.comms_jammed]
[effects.sensor_blind]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.shape.is_some());
        let effects = config.effects.expect("effects must be Some");
        assert!(effects.comms_jammed.is_some());
        assert!(effects.sensor_blind.is_some());
    }

    #[test]
    fn region_effects_without_shape_returns_error() {
        let toml_str = r##"
tags = ["region"]

[effects]
[effects.comms_jammed]
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(
            result.is_err(),
            "region entity with effects but no shape should error"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("shape"),
            "error should mention missing shape: {err}"
        );
    }

    #[test]
    fn shape_alone_without_effects_is_valid() {
        let toml_str = r##"
tags = ["region"]

[shape]
type = "sphere"
radius = 100.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.shape.is_some());
        assert!(config.effects.is_none());
    }

    #[test]
    fn empty_toml_produces_no_shape_or_effects() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.shape.is_none());
        assert!(config.effects.is_none());
    }

    // ── Station hull tests (post-[station] removal; PRD slice 2) ──────────

    #[test]
    fn station_axiom_template_parses_hull_integrity() {
        let toml_str = include_str!("../../assets/entities/station_axiom.toml");
        let config = EntityConfig::from_toml(toml_str).expect("station_axiom.toml must parse");
        let hull = config.hull.as_ref().expect("must have [hull]");
        assert!((hull.hull_integrity - 200.0).abs() < 1e-6);
    }

    #[test]
    fn station_outpost_template_parses_hull_integrity() {
        let toml_str = include_str!("../../assets/entities/station_outpost.toml");
        let config = EntityConfig::from_toml(toml_str).expect("station_outpost.toml must parse");
        let hull = config.hull.as_ref().expect("must have [hull]");
        assert!((hull.hull_integrity - 200.0).abs() < 1e-6);
    }

    #[test]
    fn station_research_outpost_template_parses_hull_integrity() {
        let toml_str = include_str!("../../assets/entities/station_research_outpost.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("station_research_outpost.toml must parse");
        let hull = config.hull.as_ref().expect("must have [hull]");
        assert!((hull.hull_integrity - 60.0).abs() < 1e-6);
    }

    #[test]
    fn all_sections_parsed_in_full_template() {
        let toml_str = r##"
tags = ["full"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005

[hull]
hull_integrity = 100
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(
            config.asteroid_field.is_some(),
            "asteroid_field should be Some"
        );
        assert!(config.hull.is_some(), "hull should be Some");
        assert_eq!(config.tags, vec!["full"]);
    }

    // ── Shipped template TOML files referenced by assets/worlds/default.toml ──
    //
    // These tests embed each template at compile time via include_str! so
    // the build fails if a referenced template is missing or malformed.

    #[test]
    fn star_sun_template_parses_with_mesh_and_lights() {
        let toml_str = include_str!("../../assets/entities/star_sun.toml");
        let config = EntityConfig::from_toml(toml_str).expect("star_sun.toml must parse");
        assert_eq!(config.name.as_deref(), Some("Sun"));
        let mesh = config
            .mesh
            .as_ref()
            .expect("star_sun.toml must have [mesh]");
        assert!(
            mesh.emissive.is_some(),
            "star_sun.toml must set [mesh].emissive"
        );
        assert!(
            !config.light.is_empty(),
            "star_sun.toml must have at least one [[light]]"
        );
        assert_eq!(config.light[0].kind, LightKind::Point);
        let collider = config
            .collider
            .as_ref()
            .expect("star_sun.toml must have [collider]");
        assert_eq!(collider.shape, ColliderShape::Ball);
        assert!((collider.radius - 50.0).abs() < 1e-6);
    }

    #[test]
    fn planet_earth_template_parses_with_mesh_and_collider() {
        let toml_str = include_str!("../../assets/entities/planet_earth.toml");
        let config = EntityConfig::from_toml(toml_str).expect("planet_earth.toml must parse");
        assert_eq!(config.name.as_deref(), Some("Earth"));
        assert!(config.mesh.is_some(), "planet_earth.toml must have [mesh]");
        let collider = config
            .collider
            .as_ref()
            .expect("planet_earth.toml must have [collider]");
        assert_eq!(collider.shape, ColliderShape::Ball);
        assert!((collider.radius - 20.0).abs() < 1e-6);
    }

    #[test]
    fn asteroid_field_main_template_parses_with_field_and_grid() {
        let toml_str = include_str!("../../assets/entities/asteroid_field_main.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("asteroid_field_main.toml must parse");
        let field = config
            .asteroid_field
            .as_ref()
            .expect("must have [asteroid_field]");
        field
            .grid
            .as_ref()
            .expect("must have [asteroid_field.grid]");
        assert_eq!(field.asteroid_type_paths.len(), 2);
        assert_eq!(field.cosmetic_type_paths.len(), 1);
    }

    // ── Faction field tests ────────────────────────────────────────────────

    #[test]
    fn faction_field_parses_from_entity_toml() {
        let toml_str = r#"
tags = ["ship"]
faction = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let faction = config.faction.expect("faction must be Some");
        assert_eq!(faction.to_string(), "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa");
    }

    #[test]
    fn faction_field_defaults_to_none_when_absent() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.faction.is_none());
    }

    #[test]
    fn player_ship_toml_parses_with_federation_faction() {
        let toml_str = include_str!("../../assets/entities/player_ship.toml");
        let config = EntityConfig::from_toml(toml_str).expect("player_ship.toml must parse");
        let faction = config.faction.expect("player_ship must declare a faction");
        // Must match the Federation UUID in assets/factions/federation.toml
        let fed_toml = include_str!("../../assets/factions/federation.toml");
        let fed = crate::faction::parse_faction_config(fed_toml).unwrap();
        assert_eq!(faction, fed.uuid, "player ship faction must be Federation");
    }

    // ── Behaviour block tests ─────────────────────────────────────────────

    #[test]
    fn behaviour_block_parses_initial_state() {
        let toml_str = r##"
tags = ["npc", "patrol"]

[behaviour]
initial_state = "idle"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.initial_state, "idle");
    }

    #[test]
    fn behaviour_block_absent_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.behaviour.is_none());
    }

    #[test]
    fn entity_with_hull_and_behaviour_has_both_sections() {
        let toml_str = r##"
tags = ["npc"]

[hull]
hull_integrity = 50.0

[behaviour]
initial_state = "idle"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.hull.is_some());
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.initial_state, "idle");
    }

    // ── StateConfig tests ──────────────────────────────────────────────────

    #[test]
    fn behaviour_with_patrolling_state_parses() {
        let toml_str = r##"
[behaviour]
initial_state = "patrol_route"

[[behaviour.state]]
name = "patrol_route"
kind = "patrolling"
waypoints = ["alpha", "beta"]
loop_path = true
target_speed = 0.6
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.initial_state, "patrol_route");
        assert_eq!(behaviour.state.len(), 1);
        let state = &behaviour.state[0];
        assert_eq!(state.name, "patrol_route");
        assert_eq!(state.kind, "patrolling");
        assert_eq!(state.waypoints, vec!["alpha", "beta"]);
        assert!(state.loop_path);
        assert!((state.target_speed - 0.6).abs() < 1e-5);
    }

    #[test]
    fn target_speed_clamped_to_zero_when_negative() {
        let toml_str = r##"
[behaviour]
initial_state = "p"

[[behaviour.state]]
name = "p"
kind = "patrolling"
waypoints = []
target_speed = -0.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let state = &config.behaviour.unwrap().state[0];
        assert_eq!(
            state.target_speed, 0.0,
            "negative target_speed must clamp to 0"
        );
    }

    #[test]
    fn target_speed_clamped_to_one_when_above_one() {
        let toml_str = r##"
[behaviour]
initial_state = "p"

[[behaviour.state]]
name = "p"
kind = "patrolling"
waypoints = []
target_speed = 1.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let state = &config.behaviour.unwrap().state[0];
        assert_eq!(state.target_speed, 1.0, "target_speed > 1 must clamp to 1");
    }

    #[test]
    fn behaviour_state_empty_by_default() {
        let toml_str = r##"
[behaviour]
initial_state = "idle"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert!(
            behaviour.state.is_empty(),
            "state array must default to empty"
        );
    }

    #[test]
    fn behaviour_multiple_states_parse() {
        let toml_str = r##"
[behaviour]
initial_state = "idle"

[[behaviour.state]]
name = "idle"
kind = "idle"
target_speed = 0.0

[[behaviour.state]]
name = "patrol"
kind = "patrolling"
waypoints = ["wp1", "wp2"]
loop_path = false
target_speed = 0.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.state.len(), 2);
        assert_eq!(behaviour.state[0].name, "idle");
        assert_eq!(behaviour.state[1].name, "patrol");
    }

    // ── pirate_raider.toml compile-time template tests ─────────────────────

    #[test]
    fn pirate_raider_template_parses_with_pirate_faction() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        // Must have pirate faction UUID
        let faction = config
            .faction
            .expect("pirate_raider must declare a faction");
        assert_eq!(
            faction.to_string(),
            "bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb",
            "pirate_raider faction must be Pirate"
        );
    }

    #[test]
    fn pirate_raider_template_has_hull() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        assert!(
            config.hull.is_some(),
            "pirate_raider must have a [hull] section"
        );
        let hull = config.hull.as_ref().unwrap();
        assert!(
            hull.captain_chair.is_some_and(|hp| hp > 0.0),
            "pirate_raider [hull] must have a positive captain_chair value"
        );
    }

    #[test]
    fn pirate_raider_template_has_helm_and_weapons_console() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        assert!(
            config.helm_console.is_some(),
            "pirate_raider must have a [helm_console]"
        );
        assert!(
            config.weapons_console.is_some(),
            "pirate_raider must have a [weapons_console]"
        );
    }

    #[test]
    fn pirate_raider_template_has_behaviour_with_all_six_states() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        let behaviour = config
            .behaviour
            .expect("pirate_raider must have a [behaviour] block");
        let state_kinds: Vec<&str> = behaviour.state.iter().map(|s| s.kind.as_str()).collect();
        assert!(
            state_kinds.contains(&"patrolling"),
            "must have patrolling state"
        );
        assert!(
            state_kinds.contains(&"pursuing"),
            "must have pursuing state"
        );
        assert!(
            state_kinds.contains(&"attacking"),
            "must have attacking state"
        );
        assert!(state_kinds.contains(&"fleeing"), "must have fleeing state");
        assert!(
            state_kinds.contains(&"warping_out"),
            "must have warping_out state"
        );
    }

    #[test]
    fn pirate_raider_template_transitions_include_enemy_in_range_and_on_attacked() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        let conditions: Vec<&str> = behaviour
            .transition
            .iter()
            .map(|t| t.condition.as_str())
            .collect();
        assert!(
            conditions.contains(&"enemy_in_range"),
            "must have enemy_in_range transition"
        );
        assert!(
            conditions.contains(&"on_attacked"),
            "must have on_attacked transition"
        );
        assert!(
            conditions.contains(&"in_weapons_range"),
            "must have in_weapons_range transition"
        );
        assert!(
            conditions.contains(&"hull_below"),
            "must have hull_below transition"
        );
        assert!(
            conditions.contains(&"on_timer"),
            "must have on_timer transition"
        );
        assert!(
            conditions.contains(&"on_scenario_unloaded"),
            "must have on_scenario_unloaded transition"
        );
    }

    // ── [torpedoes] block tests ────────────────────────────────────────────

    #[test]
    fn torpedoes_block_full_round_trips() {
        let toml_str = r##"
[torpedoes]
count = 12
damage_hull = 60
damage_shields = 7
speed = 35.0
turn_rate_deg_per_sec = 90.0
lifespan = 25.0
load_time = 8.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes must be Some");
        assert_eq!(t.count, 12);
        assert_eq!(t.damage_hull, 60);
        assert_eq!(t.damage_shields, 7);
        assert_eq!(t.speed, 35.0);
        assert_eq!(t.turn_rate_deg_per_sec, 90.0);
        assert_eq!(t.lifespan, 25.0);
        assert_eq!(t.load_time, 8.0);
    }

    #[test]
    fn torpedoes_block_absent_yields_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.torpedoes.is_none());
    }

    #[test]
    fn torpedoes_block_partial_keeps_defaults_for_missing_fields() {
        let toml_str = r##"
[torpedoes]
count = 99
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes must be Some");
        assert_eq!(t.count, 99, "override applied");
        assert_eq!(t.damage_hull, 50, "default preserved");
        assert_eq!(t.damage_shields, 5, "default preserved");
        assert_eq!(t.speed, 30.0, "default preserved");
        assert_eq!(t.turn_rate_deg_per_sec, 45.0, "default preserved");
        assert_eq!(t.lifespan, 20.0, "default preserved");
        assert_eq!(t.load_time, 10.0, "default preserved");
    }

    #[test]
    fn torpedoes_to_runtime_converts_degrees_to_radians() {
        let mut t = TorpedoesConfig::default();
        t.turn_rate_deg_per_sec = 45.0;
        let rt = t.to_runtime();
        assert!(
            (rt.turn_rate - std::f32::consts::FRAC_PI_4).abs() < 1e-5,
            "45 deg/s should convert to PI/4 rad/s, got {}",
            rt.turn_rate
        );
        assert_eq!(rt.count, 10);
        assert_eq!(rt.damage_hull, 50);
        assert_eq!(rt.load_time, 10.0);
    }

    #[test]
    fn torpedoes_defaults_match_runtime_torpedo_config_default() {
        let toml_default = TorpedoesConfig::default().to_runtime();
        let runtime_default = crate::torpedo::TorpedoConfig::default();
        assert_eq!(toml_default.count, runtime_default.count);
        assert_eq!(toml_default.damage_hull, runtime_default.damage_hull);
        assert_eq!(toml_default.damage_shields, runtime_default.damage_shields);
        assert_eq!(toml_default.speed, runtime_default.speed);
        assert!((toml_default.turn_rate - runtime_default.turn_rate).abs() < 1e-5);
        assert_eq!(toml_default.lifespan, runtime_default.lifespan);
        assert_eq!(toml_default.load_time, runtime_default.load_time);
    }

    #[test]
    fn player_ship_toml_torpedoes_block_matches_runtime_default_values() {
        // Drift guard: if the [torpedoes] block in player_ship.toml ever
        // diverges from TorpedoConfig::default(), this test fails so the
        // owner can confirm the change is intentional.
        let toml_str = include_str!("../../assets/entities/player_ship.toml");
        let config = EntityConfig::from_toml(toml_str).expect("player_ship.toml must parse");
        let t = config.torpedoes.expect("player_ship must have [torpedoes]");
        let rt = t.to_runtime();
        let baseline = crate::torpedo::TorpedoConfig::default();
        assert_eq!(rt.count, baseline.count, "magazine size drift");
        assert_eq!(rt.damage_hull, baseline.damage_hull);
        assert_eq!(rt.damage_shields, baseline.damage_shields);
        assert_eq!(rt.speed, baseline.speed);
        assert!((rt.turn_rate - baseline.turn_rate).abs() < 1e-5);
        assert_eq!(rt.lifespan, baseline.lifespan);
        assert_eq!(rt.load_time, baseline.load_time);
    }

    // ── [repair] block tests ───────────────────────────────────────────────

    #[test]
    fn repair_block_full_round_trips() {
        let toml_str = r##"
[repair]
travel_duration_secs = 7.5
repair_rate_hp_per_sec = 1.25
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let r = config.repair.expect("repair must be Some");
        assert_eq!(r.travel_duration_secs, 7.5);
        assert_eq!(r.repair_rate_hp_per_sec, 1.25);
    }

    #[test]
    fn repair_block_absent_yields_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.repair.is_none());
    }

    #[test]
    fn repair_block_partial_keeps_defaults_for_missing_fields() {
        let toml_str = r##"
[repair]
travel_duration_secs = 9.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let r = config.repair.expect("repair must be Some");
        assert_eq!(r.travel_duration_secs, 9.0, "override applied");
        assert_eq!(r.repair_rate_hp_per_sec, 0.5, "default preserved");
    }

    #[test]
    fn repair_to_runtime_preserves_values() {
        let r = RepairConfig {
            travel_duration_secs: 3.0,
            repair_rate_hp_per_sec: 2.0,
        };
        let rt = r.to_runtime();
        assert_eq!(rt.travel_duration, 3.0);
        assert_eq!(rt.repair_rate_hp_per_sec, 2.0);
    }

    #[test]
    fn repair_defaults_match_runtime_repair_timings_default() {
        let toml_default = RepairConfig::default().to_runtime();
        let runtime_default = crate::repair_teams::RepairTimings::default();
        assert_eq!(
            toml_default.travel_duration,
            runtime_default.travel_duration
        );
        assert_eq!(
            toml_default.repair_rate_hp_per_sec,
            runtime_default.repair_rate_hp_per_sec
        );
    }

    #[test]
    fn player_ship_toml_repair_block_matches_runtime_default_values() {
        // Drift guard: if the [repair] block in player_ship.toml ever diverges
        // from RepairTimings::default(), this test fails so the owner can
        // confirm the change is intentional. (The defaults themselves match
        // the historical hardcoded constants in `repair_teams.rs`.)
        let toml_str = include_str!("../../assets/entities/player_ship.toml");
        let config = EntityConfig::from_toml(toml_str).expect("player_ship.toml must parse");
        let r = config.repair.expect("player_ship must have [repair]");
        let rt = r.to_runtime();
        let baseline = crate::repair_teams::RepairTimings::default();
        assert_eq!(
            rt.travel_duration, baseline.travel_duration,
            "travel duration drift"
        );
        assert_eq!(
            rt.repair_rate_hp_per_sec, baseline.repair_rate_hp_per_sec,
            "repair rate drift"
        );
    }

    // ── [shields_console.base] block tests ────────────────────────────────

    #[test]
    fn shields_console_base_block_full_round_trips() {
        let toml_str = r##"
[shields_console]

[shields_console.base]
num_facings = 6
max_hp = 200
regen_per_sec = 7.5
offline_duration = 12.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sc = config
            .shields_console
            .expect("shields_console must be Some");
        let base = sc.base.expect("base sub-block must be Some");
        assert_eq!(base.num_facings, 6);
        assert_eq!(base.max_hp, 200);
        assert_eq!(base.regen_per_sec, 7.5);
        assert_eq!(base.offline_duration, 12.0);
    }

    #[test]
    fn shields_console_without_base_subblock_yields_none() {
        // The flat focus fields parse fine; absent `[shields_console.base]`
        // must produce `base: None` so the runtime falls back to
        // `ShieldConfig::default()`.
        let toml_str = r##"
[shields_console]
focus_bonus_max_hp = 99
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sc = config
            .shields_console
            .expect("shields_console must be Some");
        assert!(
            sc.base.is_none(),
            "base sub-block must default to None when absent"
        );
        assert_eq!(sc.focus_bonus_max_hp, 99, "flat focus field still parses");
    }

    #[test]
    fn shields_base_block_partial_keeps_defaults_for_missing_fields() {
        let toml_str = r##"
[shields_console.base]
max_hp = 250
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let base = config
            .shields_console
            .expect("shields_console")
            .base
            .expect("base");
        assert_eq!(base.max_hp, 250, "override applied");
        assert_eq!(base.num_facings, 4, "default preserved");
        assert_eq!(base.regen_per_sec, 5.0, "default preserved");
        assert_eq!(base.offline_duration, 10.0, "default preserved");
    }

    #[test]
    fn shields_base_to_runtime_preserves_values() {
        let base = ShieldsBaseConfig {
            num_facings: 3,
            max_hp: 75,
            regen_per_sec: 2.5,
            offline_duration: 8.0,
        };
        let rt = base.to_runtime();
        assert_eq!(rt.num_facings, 3);
        assert_eq!(rt.max_hp, 75);
        assert_eq!(rt.regen_per_sec, 2.5);
        assert_eq!(rt.offline_duration, 8.0);
    }

    #[test]
    fn shields_base_defaults_match_runtime_shield_config_default() {
        let toml_default = ShieldsBaseConfig::default().to_runtime();
        let runtime_default = crate::shield::ShieldConfig::default();
        assert_eq!(toml_default.num_facings, runtime_default.num_facings);
        assert_eq!(toml_default.max_hp, runtime_default.max_hp);
        assert_eq!(toml_default.regen_per_sec, runtime_default.regen_per_sec);
        assert_eq!(
            toml_default.offline_duration,
            runtime_default.offline_duration
        );
    }

    #[test]
    fn player_ship_toml_shields_base_block_matches_runtime_default_values() {
        // Drift guard: if [shields_console.base] in player_ship.toml ever
        // diverges from ShieldConfig::default(), this test fails so the
        // owner can confirm the change is intentional.
        let toml_str = include_str!("../../assets/entities/player_ship.toml");
        let config = EntityConfig::from_toml(toml_str).expect("player_ship.toml must parse");
        let base = config
            .shields_console
            .expect("player_ship must have [shields_console]")
            .base
            .expect("player_ship must have [shields_console.base]");
        let rt = base.to_runtime();
        let baseline = crate::shield::ShieldConfig::default();
        assert_eq!(rt.num_facings, baseline.num_facings, "num_facings drift");
        assert_eq!(rt.max_hp, baseline.max_hp, "max_hp drift");
        assert_eq!(rt.regen_per_sec, baseline.regen_per_sec, "regen drift");
        assert_eq!(
            rt.offline_duration, baseline.offline_duration,
            "offline duration drift"
        );
    }

    // ── PhaserCombatConfig (player phaser tuning) tests ───────────────────
    //
    // PhaserCombatConfig is built from the existing flat fields on
    // [weapons_console] (beam_range, beam_damage_per_sec, beam_duration_secs,
    // cooldown_secs). No new TOML keys were introduced for this slice; the
    // change is "the player path now honours them too".

    #[test]
    fn phaser_combat_config_from_weapons_console_uses_supplied_values() {
        let toml_str = r##"
[weapons_console]
beam_range = 99.0
beam_damage_per_sec = 12.0
beam_duration_secs = 4.0
cooldown_secs = 7.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        let combat = PhaserCombatConfig::from_weapons_console(&wc);
        assert_eq!(combat.phaser_range, 99.0);
        assert_eq!(combat.beam_damage_per_sec, 12.0);
        assert_eq!(combat.beam_duration_secs, 4.0);
        assert_eq!(combat.beam_cooldown_secs, 7.5);
    }

    #[test]
    fn phaser_combat_config_falls_back_to_defaults_for_zero_or_missing_fields() {
        // Mirrors the "zero means absent" convention used by the NPC phaser
        // path at console/weapons/server.rs:336-337.
        let toml_str = r##"
[weapons_console]
beam_range = 50.0
# beam_damage_per_sec omitted → 0.0 → fall back to default 5.0
# beam_duration_secs omitted → 0.0 → fall back to default 6.0
# cooldown_secs omitted → 0.0 → fall back to default 6.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        let combat = PhaserCombatConfig::from_weapons_console(&wc);
        assert_eq!(combat.phaser_range, 50.0, "supplied override applied");
        assert_eq!(combat.beam_damage_per_sec, 5.0, "default for omitted field");
        assert_eq!(combat.beam_duration_secs, 6.0, "default for omitted field");
        assert_eq!(combat.beam_cooldown_secs, 6.0, "default for omitted field");
    }

    // ── PhaserBankConfig / TorpedoTubeConfig schema tests (Phase A) ───────

    #[test]
    fn phaser_banks_array_parses_full_entries() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
beam_range = 35.0

[[weapons_console.phaser_banks]]
id = "starboard"
facing_deg = 90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.phaser_banks.len(), 2);
        assert_eq!(wc.phaser_banks[0].id, "port");
        assert_eq!(wc.phaser_banks[0].facing_deg, -90.0);
        assert_eq!(wc.phaser_banks[0].fire_arc_deg, 180.0);
        assert_eq!(wc.phaser_banks[0].auto_arc_deg, 120.0);
        assert_eq!(wc.phaser_banks[0].beam_range, 35.0);
        assert_eq!(wc.phaser_banks[1].id, "starboard");
        assert_eq!(
            wc.phaser_banks[1].beam_range, 0.0,
            "missing beam_range defaults to 0 (caller falls back to parent)"
        );
    }

    #[test]
    fn weapons_console_shield_pierce_defaults_to_zero() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.shield_pierce, 0.0);
    }

    #[test]
    fn weapons_console_shield_pierce_parses_when_present() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0
shield_pierce = 0.25
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert!((wc.shield_pierce - 0.25).abs() < 1e-6);
    }

    #[test]
    fn phaser_bank_shield_pierce_defaults_to_none_when_absent() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.phaser_banks[0].shield_pierce, None);
    }

    #[test]
    fn phaser_bank_shield_pierce_parses_when_present() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
shield_pierce = 0.6
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.phaser_banks[0].shield_pierce, Some(0.6));
    }

    #[test]
    fn torpedoes_shield_pierce_defaults_to_zero() {
        let toml_str = r##"
[torpedoes]
count = 5

[[torpedoes.tubes]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert_eq!(t.shield_pierce, 0.0);
        // Propagates into the runtime config that the in-flight torpedo
        // snapshots at launch.
        assert_eq!(t.to_runtime().shield_pierce, 0.0);
    }

    #[test]
    fn torpedoes_shield_pierce_parses_when_present() {
        let toml_str = r##"
[torpedoes]
count = 5
shield_pierce = 0.5

[[torpedoes.tubes]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert!((t.shield_pierce - 0.5).abs() < 1e-6);
        assert!((t.to_runtime().shield_pierce - 0.5).abs() < 1e-6);
    }

    #[test]
    fn torpedo_in_flight_snapshots_shield_pierce_at_launch() {
        // Wiring proof: changing the in-flight torpedo's snapshot mid-flight
        // doesn't affect future launches (it's a per-torpedo copy).
        use crate::torpedo::{TorpedoConfig, TorpedoSystem};
        use std::collections::HashMap;
        let mut cfg = TorpedoConfig::default();
        cfg.shield_pierce = 0.75;
        let tubes = vec![TorpedoTubeConfig {
            id: "fore".into(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
        }];
        let mut sys = TorpedoSystem::from_configs(&tubes, cfg);
        sys.tube_mut("fore").unwrap().start_load();
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(sys.config.load_time, &targets);
        sys.launch("fore", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert!((sys.in_flight[0].shield_pierce - 0.75).abs() < 1e-6);

        let det = sys.handle_collision_full("t1").unwrap();
        assert!((det.shield_pierce - 0.75).abs() < 1e-6);
    }

    #[test]
    fn phaser_banks_defaults_to_empty_vec_when_absent() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert!(
            wc.phaser_banks.is_empty(),
            "phaser_banks defaults to empty when [[phaser_banks]] absent"
        );
    }

    #[test]
    fn validate_phaser_banks_accepts_valid_list() {
        let banks = vec![
            PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 120.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
            },
            PhaserBankConfig {
                id: "starboard".into(),
                facing_deg: 90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 120.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
            },
        ];
        assert!(validate_phaser_banks(&banks).is_ok());
    }

    #[test]
    fn validate_phaser_banks_rejects_empty_list() {
        let err = validate_phaser_banks(&[]).unwrap_err();
        assert!(err.contains("empty"), "error mentions empty: {err}");
    }

    #[test]
    fn validate_phaser_banks_rejects_duplicate_ids() {
        let banks = vec![
            PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 90.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
            },
            PhaserBankConfig {
                id: "port".into(),
                facing_deg: 90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 90.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
            },
        ];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(err.contains("duplicate"), "error mentions duplicate: {err}");
        assert!(err.contains("port"));
    }

    #[test]
    fn validate_phaser_banks_rejects_auto_arc_greater_than_fire_arc() {
        let banks = vec![PhaserBankConfig {
            id: "port".into(),
            facing_deg: -90.0,
            fire_arc_deg: 90.0,
            auto_arc_deg: 180.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
        }];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(
            err.contains("auto_arc_deg"),
            "error mentions auto arc: {err}"
        );
    }

    #[test]
    fn validate_phaser_banks_rejects_fire_arc_out_of_range() {
        let banks = vec![PhaserBankConfig {
            id: "port".into(),
            facing_deg: 0.0,
            fire_arc_deg: 400.0,
            auto_arc_deg: 90.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
        }];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(
            err.contains("fire_arc_deg"),
            "error mentions fire arc: {err}"
        );

        let banks = vec![PhaserBankConfig {
            id: "port".into(),
            facing_deg: 0.0,
            fire_arc_deg: 0.0,
            auto_arc_deg: 0.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
        }];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(err.contains("fire_arc_deg"), "zero arc rejected: {err}");
    }

    #[test]
    fn torpedo_tubes_array_parses_full_entries() {
        let toml_str = r##"
[torpedoes]
count = 10

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = -30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "fore_starboard"
facing_deg = 30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "aft"
facing_deg = 180.0
fire_arc_deg = 90.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert_eq!(t.tubes.len(), 3);
        assert_eq!(t.tubes[0].id, "fore_port");
        assert_eq!(t.tubes[0].facing_deg, -30.0);
        assert_eq!(t.tubes[0].fire_arc_deg, 90.0);
        assert_eq!(t.tubes[2].id, "aft");
        assert_eq!(t.tubes[2].facing_deg, 180.0);
    }

    #[test]
    fn torpedo_tubes_defaults_to_empty_vec_when_absent() {
        let toml_str = r##"
[torpedoes]
count = 10
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert!(
            t.tubes.is_empty(),
            "tubes defaults to empty when [[torpedoes.tubes]] absent"
        );
    }

    #[test]
    fn validate_torpedo_tubes_accepts_valid_list() {
        let tubes = vec![
            TorpedoTubeConfig {
                id: "fore_port".into(),
                facing_deg: -30.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
            },
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 180.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
            },
        ];
        assert!(validate_torpedo_tubes(&tubes).is_ok());
    }

    #[test]
    fn validate_torpedo_tubes_rejects_empty_list() {
        let err = validate_torpedo_tubes(&[]).unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_torpedo_tubes_rejects_duplicate_ids() {
        let tubes = vec![
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 180.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
            },
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 0.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
            },
        ];
        let err = validate_torpedo_tubes(&tubes).unwrap_err();
        assert!(err.contains("duplicate"));
        assert!(err.contains("aft"));
    }

    #[test]
    fn validate_torpedo_tubes_rejects_fire_arc_out_of_range() {
        let tubes = vec![TorpedoTubeConfig {
            id: "aft".into(),
            facing_deg: 180.0,
            fire_arc_deg: 0.0,
            load_time: None,
            marker: None,
        }];
        let err = validate_torpedo_tubes(&tubes).unwrap_err();
        assert!(err.contains("fire_arc_deg"));
    }

    #[test]
    fn comms_config_parses_range_from_toml() {
        let toml_str = r##"
[comms]
range = 8000.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let comms = config.comms.expect("comms section present");
        assert_eq!(comms.range, 8000.0);
    }

    #[test]
    fn comms_config_is_none_when_section_absent() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.comms.is_none(), "no [comms] → field is None");
    }
}

// ── Leaf scene-shape types moved from former entities/map_config.rs (PRD #341) ──
// These describe entity-template physical/visual properties consumed by
// EntityConfig (one-per-template) and by steroids::spawner. They are not
// world-tree concerns and so live alongside the entity-template schema rather
// than in world::config.

/// Global configuration block (currently used only for the deterministic seed
/// surfaced through WorldConfig).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    /// Global seed for deterministic generation.
    #[serde(default = "default_global_seed")]
    pub seed: u64,
    /// Display name shown in the lobby title bar.
    #[serde(default)]
    pub title: Option<String>,
    /// Short description shown below the title in the lobby.
    #[serde(default)]
    pub description: Option<String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            title: None,
            description: None,
        }
    }
}

fn default_global_seed() -> u64 {
    42
}

/// Configuration for the grid-based asteroid spawner.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GridConfig {
    pub resolution: f32,
    #[serde(default = "default_fill_gameplay")]
    pub fill_gameplay: f32,
    #[serde(default = "default_fill_cosmetic")]
    pub fill_cosmetic: f32,
    #[serde(default)]
    pub uniformity: f32,
    #[serde(default = "default_noise_freq")]
    pub noise_freq: f32,
    #[serde(default = "default_noise_octaves")]
    pub noise_octaves: u32,
    #[serde(default = "default_density_noise_freq")]
    pub density_noise_freq: f32,
    #[serde(default = "default_density_noise_octaves")]
    pub density_noise_octaves: u32,
    #[serde(default)]
    pub jitter: f32,
    #[serde(default)]
    pub cosmetic_y_offset: f32,
    #[serde(default = "default_gameplay_y_variance")]
    pub gameplay_y_variance: f32,
    #[serde(default = "default_spawn_cells")]
    pub spawn_cells: u32,
    #[serde(default = "default_despawn_cells")]
    pub despawn_cells: u32,
}

fn default_fill_gameplay() -> f32 {
    0.4
}
fn default_fill_cosmetic() -> f32 {
    0.15
}
fn default_noise_freq() -> f32 {
    0.02
}
fn default_noise_octaves() -> u32 {
    3
}
fn default_density_noise_freq() -> f32 {
    0.01
}
fn default_density_noise_octaves() -> u32 {
    2
}
fn default_gameplay_y_variance() -> f32 {
    0.5
}
fn default_spawn_cells() -> u32 {
    10
}
fn default_despawn_cells() -> u32 {
    12
}

/// Shape variant for an asteroid field.
///
/// When the TOML schema omits `shape`, the field defaults to the historical
/// behaviour (cell-centre distance check against `inner_radius`/`outer_radius`,
/// which produces a disc/annulus depending on whether `inner_radius` is zero).
///
/// `Torus` selects the explicit annulus eligibility test: a cell is admitted
/// if its XZ bounding box overlaps the annulus `[inner_radius, outer_radius]`
/// around the world origin. Cells whose bounding box lies fully inside
/// `inner_radius` or whose nearest corner is beyond `outer_radius` are
/// rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsteroidFieldShape {
    /// Annulus / ring-belt eligibility based on `inner_radius` and
    /// `outer_radius`. Cells whose bounding box overlaps the annulus
    /// are admitted.
    Torus,
}

/// Configuration for an asteroid field.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AsteroidFieldConfig {
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub density: f32,
    #[serde(default = "default_spawn_distance")]
    pub spawn_distance: f32,
    #[serde(default = "default_despawn_distance")]
    pub despawn_distance: f32,
    #[serde(default)]
    pub asteroid_type_paths: Vec<String>,
    #[serde(default)]
    pub cosmetic_type_paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub grid: Option<GridConfig>,
    /// Fraction of asteroid-collision damage that bypasses shields and
    /// applies directly to the hull. Default `0.0` — asteroid impacts are
    /// fully absorbed by the facing shield quadrant (matching pre-#414
    /// behaviour). Clamped to `[0.0, 1.0]` at apply time.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Optional shape variant. When `None`, the historical cell-centre
    /// distance eligibility test is used. When `Some(Torus)`, cells are
    /// admitted iff their XZ bounding box overlaps the annulus
    /// `[inner_radius, outer_radius]`. See [`AsteroidFieldShape`].
    #[serde(default)]
    pub shape: Option<AsteroidFieldShape>,
    /// Optional world anchor name. When present, the field's eligibility
    /// region and per-asteroid spawn positions are translated so the
    /// `[inner_radius, outer_radius]` annulus is centred on the named
    /// anchor's world position instead of the world origin. The anchor
    /// is resolved against `WorldConfig.anchors` at spawn time; the
    /// resolved offset is written into `anchor_offset`. If the anchor
    /// name is not present in the world's anchor table, the field falls
    /// back to the world origin (`anchor_offset = [0, 0, 0]`) and a
    /// warning is logged.
    #[serde(default)]
    pub anchor: Option<String>,
    /// Resolved world-space offset for the anchor referenced by `anchor`.
    /// Defaults to `[0, 0, 0]` (world origin) when no anchor is set or
    /// the named anchor could not be resolved. Not serialised — derived
    /// at spawn time.
    #[serde(skip)]
    pub anchor_offset: [f32; 3],
}

fn default_spawn_distance() -> f32 {
    150.0
}
fn default_despawn_distance() -> f32 {
    250.0
}
