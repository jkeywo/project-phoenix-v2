//! Entity schema: sensors navigation. Public paths remain in the parent module.
use super::*;

/// Config block for the Navigation console in a ship TOML.
///
/// Loaded from `[navigation_console]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationConsoleConfig {
    /// System chart radar config for the Navigation console.
    #[serde(default)]
    pub system_chart: crate::radar_config::RadarConfig,
    /// Inline per-system target selector (issue #778). Loaded from
    /// `[navigation_console.selector]`; absent ⇒ the canonical
    /// [`default_navigation_target_selector_config`] is synthesised at spawn.
    /// `operate_navigation_ai` runs it to rank objective destinations and
    /// eligible chart contacts into the shared Waypoint.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
}

/// Config block for the Sensors console in a ship TOML.
///
/// Loaded from `[sensors_console]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorsConsoleConfig {
    /// Long-range radar config for the Sensors console.
    #[serde(default)]
    pub long_range_radar: crate::radar_config::RadarConfig,
    /// AI tuning parameters for the Sensors frequency-hint controller.
    /// Loaded from `[sensors_console.ai]`.
    #[serde(default)]
    pub ai: Option<SensorsAiConfigToml>,
    /// Inline per-system target selector (issue #776). Loaded from
    /// `[sensors_console.selector]`; absent ⇒ the canonical
    /// [`default_sensors_target_selector_config`] is synthesised at spawn.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
    /// Selected-contact trajectory projection tuning (issue #1339). Loaded
    /// from `[sensors_console.projection]`; absent ⇒ the 60s/10s defaults in
    /// [`SensorsProjectionConfig::default`] apply, so every hull that omits
    /// this table still gets a working projection.
    #[serde(default)]
    pub projection: Option<SensorsProjectionConfig>,
}

/// AI tuning parameters for the Sensors frequency-hint controller
/// (`console_ai::tick_frequency_hint`, issue #692).
///
/// Loaded from `[sensors_console.ai]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorsAiConfigToml {
    /// Delay (seconds) between a target lock and the AI-driven Sensors
    /// operator emitting a `FrequencyHint` coordination message to Tactical.
    #[serde(default = "default_sensors_ai_frequency_hint_delay_secs")]
    pub frequency_hint_delay_secs: f32,
}

fn default_sensors_ai_frequency_hint_delay_secs() -> f32 {
    3.0
}

/// Selected-contact trajectory projection tuning for the Sensors radar
/// (issue #1339).
///
/// Loaded from `[sensors_console.projection]` in the ship entity TOML. Purely
/// a presentation tunable — it controls how far ahead and how densely the
/// client draws the Science Target's projected relative path when its
/// velocity is known; it does not touch Science Target or Combat Lock
/// authority.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorsProjectionConfig {
    /// How far into the future the projection extends, in seconds.
    #[serde(default = "default_sensors_projection_horizon_secs")]
    pub horizon_secs: f32,
    /// Spacing between successive projection markers, in seconds.
    #[serde(default = "default_sensors_projection_marker_interval_secs")]
    pub marker_interval_secs: f32,
}

impl Default for SensorsProjectionConfig {
    fn default() -> Self {
        Self {
            horizon_secs: default_sensors_projection_horizon_secs(),
            marker_interval_secs: default_sensors_projection_marker_interval_secs(),
        }
    }
}

pub fn default_sensors_projection_horizon_secs() -> f32 {
    60.0
}

pub fn default_sensors_projection_marker_interval_secs() -> f32 {
    10.0
}
