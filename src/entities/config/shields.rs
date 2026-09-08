//! Entity schema: shields. Public paths remain in the parent module.
use super::*;

/// AI tuning parameters for the shields focus controller.
///
/// Loaded from `[shields_console.ai]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsAiConfigToml {
    /// Maximum time window (seconds) for tracking incoming damage per arc.
    /// Damage older than this is pruned.
    #[serde(default = "default_shields_ai_damage_window_secs")]
    pub damage_window_secs: f32,
    /// Minimum time window (seconds) before the AI acts on damage concentration.
    /// Damage newer than this is not yet considered.
    #[serde(default = "default_shields_ai_min_damage_window_secs")]
    pub min_damage_window_secs: f32,
    /// Percentage of total damage in the window that must hit the same arc
    /// before the AI focuses it (0–100).
    #[serde(default = "default_shields_ai_damage_pct_threshold")]
    pub damage_pct_threshold: f32,
    /// Percentage threshold: if the lowest-arc normalized health is below this
    /// fraction of the next-lowest arc, focus the weakest arc (0–100).
    #[serde(default = "default_shields_ai_health_ratio_threshold")]
    pub health_ratio_threshold: f32,
}

fn default_shields_ai_damage_window_secs() -> f32 {
    4.0
}
fn default_shields_ai_min_damage_window_secs() -> f32 {
    1.0
}
fn default_shields_ai_damage_pct_threshold() -> f32 {
    50.0
}
fn default_shields_ai_health_ratio_threshold() -> f32 {
    50.0
}

/// Config block for the Shields console focus bonuses/penalties.
///
/// Loaded from `[shields_console]` in the ship entity TOML. The nested
/// `[shields_console.base]` sub-block (modelled by [`ShieldsBaseConfig`])
/// supplies the underlying shield-system base values (number of facings,
/// max HP, regen, offline duration) that were previously hardcoded by
/// `ShieldConfig::default()` at `src/weapons/shield.rs:50-58`.
///
/// Damage multiplier fields `focus_focused_damage_multiplier` and
/// `focus_unfocused_damage_multiplier` default to 1.0 (no change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsConsoleConfig {
    /// Per-level bonus table for the `shields` power group (issue #952),
    /// indexed by level-1. Feeds `ModifierSlot::ShieldRegen`, so it decides
    /// what a reactor point spent here buys in regeneration. Absent ⇒ the
    /// fleet-wide `[-0.5, 0.0, 0.25, 0.5]` default.
    ///
    /// This field moved here from `[sensors_console]` when `shields` replaced
    /// `sensors` as a power group: nothing reads a `sensors` curve any more,
    /// because `ModifierSlot::RadarRange` no longer has a power producer.
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
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
    /// Damage multiplier applied to incoming damage on the focused arc.
    /// 1.0 = no change, 0.7 = 30% reduction.
    #[serde(default = "default_focus_focused_damage_multiplier")]
    pub focus_focused_damage_multiplier: f32,
    /// Damage multiplier applied to incoming damage on non-focused arcs
    /// (when another arc is focused). 1.0 = no change, 1.25 = 25% increase.
    #[serde(default = "default_focus_unfocused_damage_multiplier")]
    pub focus_unfocused_damage_multiplier: f32,
    /// Base shield-system values (number of facings, max HP, regen,
    /// offline duration). When absent the historical hardcoded defaults
    /// from `ShieldConfig::default()` are used.
    #[serde(default)]
    pub base: Option<ShieldsBaseConfig>,
    /// Shield generator frequency (0.0–1.0). Default 0.5. When
    /// `[[shield_arc]]` blocks are present, the first arc's frequency
    /// takes precedence.
    #[serde(default = "default_shield_frequency")]
    pub frequency: f32,
    /// AI tuning parameters for the shields focus controller.
    #[serde(default)]
    pub ai: Option<ShieldsAiConfigToml>,
    /// Inline stateless AI policy for the Shields focus fine system (issue
    /// #783), loaded from `[shields_console.ai_policy]`. Sibling to `ai`: the
    /// typed `ai` knobs remain the fallback default source, while this block —
    /// when authored — carries the same four numbers as `param(...)` entries
    /// (`damage_window_secs`, `min_damage_window_secs`, `damage_pct_threshold`,
    /// `health_ratio_threshold`) plus the prioritised rules that gate whether the
    /// retained arc-ranking kernel acts. Absent, the canonical
    /// [`default_shields_focus_ai_config`] is synthesised at spawn (baseline
    /// preservation). Validated in [`crate::entities::config::EntityConfig::from_toml`]
    /// against [`SHIELD_FOCUS_CHANNELS`] / [`SHIELD_FOCUS_VERBS`].
    #[serde(default)]
    pub ai_policy: Option<FineSystemAiConfigToml>,
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
    1.0
}
fn default_focus_decay_rate() -> f32 {
    10.0
}
fn default_focus_focused_damage_multiplier() -> f32 {
    1.0
}
fn default_focus_unfocused_damage_multiplier() -> f32 {
    1.0
}

impl Default for ShieldsConsoleConfig {
    fn default() -> Self {
        Self {
            power_multipliers: None,
            focus_bonus_max_hp: default_focus_bonus_max_hp(),
            focus_bonus_regen: default_focus_bonus_regen(),
            focus_penalty_max_hp: default_focus_penalty_max_hp(),
            focus_penalty_regen: default_focus_penalty_regen(),
            focus_decay_rate: default_focus_decay_rate(),
            focus_focused_damage_multiplier: default_focus_focused_damage_multiplier(),
            focus_unfocused_damage_multiplier: default_focus_unfocused_damage_multiplier(),
            base: None,
            frequency: default_shield_frequency(),
            ai: None,
            ai_policy: None,
        }
    }
}

/// Base shield-system values loaded from `[shields_console.base]`.
///
/// These map 1:1 onto `crate::weapons::shield::ShieldConfig` (the runtime struct
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
    2.0
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
    pub fn to_runtime(&self) -> crate::weapons::shield::ShieldConfig {
        crate::weapons::shield::ShieldConfig {
            num_facings: self.num_facings,
            max_hp: self.max_hp,
            regen_per_sec: self.regen_per_sec,
            offline_duration: self.offline_duration,
        }
    }
}

/// Designer-authored per-arc shield config (issue #514).
///
/// Loaded from top-level `[[shield_arc]]` blocks in the ship TOML. Every
/// arc auto-generates a matching `[[system]]` entry with
/// `kind = "shield_arc"` and `SystemId("shield-arc-<id>")` during
/// `EntityConfig::from_toml`. See [`ShieldArcConfig::to_runtime`] for the
/// runtime conversion consumed by [`crate::weapons::shield::ShieldSystem::from_arcs`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldArcConfig {
    /// Stable arc id (kebab-case fragment used to build the fine SystemId).
    pub id: String,
    /// Display label shown in the JS panel (e.g. `"Fore"`, `"All"`).
    pub label: String,
    /// Arc centre bearing in degrees (0 = fore, 90 = starboard).
    pub center_deg: f32,
    /// Arc angular width in degrees.
    pub width_deg: f32,
    /// Per-arc override for max HP; falls back to `[shields_console.base] max_hp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hp: Option<i32>,
    /// Per-arc override for regen/sec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regen_per_sec: Option<f32>,
    /// Per-arc override for offline duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline_duration: Option<f32>,
    /// Per-arc hull HP for damage-tier tracking (fed into `ShipArcHull`).
    #[serde(default)]
    pub hull_max_hp: f32,
    /// HP fraction below which the arc enters Damaged tier. Default 0.75.
    #[serde(default = "default_arc_hull_damaged_threshold")]
    pub hull_damaged_threshold_pct: f32,
    /// HP fraction below which the arc enters Disabled tier. Default 0.25.
    #[serde(default = "default_arc_hull_disabled_threshold")]
    pub hull_disabled_threshold_pct: f32,
    /// Debuff magnitude applied on Damaged/Disabled tier. Default 0.15.
    #[serde(default = "default_arc_hull_debuff_magnitude")]
    pub hull_debuff_magnitude: f32,
    /// Hit-routing priority. When multiple arcs cover the same bearing, the
    /// arc with the highest `priority` value absorbs the hit first. Falls
    /// through to the next tier only when all arcs at the higher priority
    /// covering that bearing are offline. Default 1.
    #[serde(default = "default_arc_priority")]
    pub priority: u32,
    /// Shield generator frequency (0.0–1.0) for this arc's shield system.
    /// When multiple arcs are declared, the first arc's frequency seeds the
    /// ship-wide shield frequency. Default 0.5.
    #[serde(default = "default_shield_frequency")]
    pub frequency: f32,
}

fn default_shield_frequency() -> f32 {
    0.5
}

fn default_arc_priority() -> u32 {
    1
}

fn default_arc_hull_damaged_threshold() -> f32 {
    0.75
}

fn default_arc_hull_disabled_threshold() -> f32 {
    0.25
}

fn default_arc_hull_debuff_magnitude() -> f32 {
    0.15
}

impl ShieldArcConfig {
    /// Convert this TOML block into the runtime shape consumed by
    /// [`crate::weapons::shield::ShieldSystem::from_arcs`].
    pub fn to_runtime(&self) -> crate::weapons::shield::ArcRuntimeConfig {
        crate::weapons::shield::ArcRuntimeConfig {
            id: self.id.clone(),
            label: self.label.clone(),
            center_deg: self.center_deg,
            width_deg: self.width_deg,
            max_hp: self.max_hp,
            regen_per_sec: self.regen_per_sec,
            offline_duration: self.offline_duration,
            priority: self.priority,
        }
    }
}
