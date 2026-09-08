//! Entity schema: power. Public paths remain in the parent module.
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerConfigSection {
    pub capacity: f32,
    pub rates: [f32; 6],
    #[serde(default = "default_sustainable_power_total")]
    pub sustainable_total: u8,
    #[serde(default = "default_max_commanded_power_total")]
    pub max_commanded_total: u8,
    pub emergency_threshold: f32,
    /// Inline stateless AI policy for the Power reactor fine system (issue
    /// #784), loaded from `[power.ai_policy]`. Replaces the retired stateful
    /// `[power.ai]` engine (`PowerAiConfigToml` + `EngageState` hysteresis).
    /// Each authored `[[power.ai_policy.rule]]` binds a `priority` and a power
    /// GROUP `channel` to a `when` guard and the value-carrying
    /// `set_power_group_allocation` verb (its `level` payload the absolute
    /// target). Every allocation rule declares a minimum battery reserve as a
    /// `param(...)` referenced by its guard (AC2). Absent, the canonical
    /// [`default_power_ai_config`] is synthesised at spawn (baseline
    /// preservation). Validated in [`EntityConfig::from_toml`] against
    /// [`POWER_SET_ALLOCATION_VERB`] and a valid-channel set built dynamically
    /// from the ship's `[power_groups.*]` keys (AC1 — no fixed catalogue).
    #[serde(default)]
    pub ai_policy: Option<FineSystemAiConfigToml>,
}

const fn default_sustainable_power_total() -> u8 {
    6
}

const fn default_max_commanded_power_total() -> u8 {
    8
}
