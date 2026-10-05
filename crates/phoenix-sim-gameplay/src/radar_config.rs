// Pure-Rust radar configuration module.
//
// `RadarConfig` holds the per-console radar parameters: detection range and a
// tag-based entity filter.  Instances are loaded from the ship entity TOML
// (e.g. `[helm_console.radar]`, `[weapons_console.radar]`).
//
// This module has no Bevy dependency — it is fully unit-testable on native.

use crate::entities::tags::{parse_tags, EntityTag};
use serde::{Deserialize, Deserializer, Serialize};

/// Configuration for a single radar instance.
///
/// Loaded from ship entity TOML under a console sub-table, e.g.:
///
/// ```toml
/// [helm_console.radar]
/// range = 50.0
/// shows = ["asteroid"]
///
/// [weapons_console.radar]
/// range = 60.0
/// shows = ["asteroid", "ship"]
/// selects = ["ship", "station"]
/// ```
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RadarConfig {
    /// Maximum detection range in world units.
    pub range: f32,
    /// Tag filter: only entities whose tags overlap this list are displayed.
    /// Uses OR logic — an entity must match **at least one** tag.
    /// An empty list means nothing is shown.
    pub shows: Vec<EntityTag>,
    /// Targetability filter: only entities whose `[target].tags` overlap this
    /// list are selectable on the radar. Uses OR logic.
    /// An empty list means nothing is selectable.
    #[serde(default)]
    pub selects: Vec<EntityTag>,
}

impl Default for RadarConfig {
    fn default() -> Self {
        Self {
            range: 50.0,
            shows: vec![EntityTag::Asteroid],
            selects: Vec::new(),
        }
    }
}

/// Intermediate deserialisation type for TOML parsing.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRadarConfig {
    #[serde(default = "default_range")]
    range: f32,
    #[serde(default)]
    shows: Vec<String>,
    #[serde(default)]
    selects: Vec<String>,
}

fn default_range() -> f32 {
    50.0
}

impl RadarConfig {
    /// Returns the radar dampening multiplier for this configuration.
    ///
    /// Reads the resolved `ShipModifiers` value for the `RadarRange` slot,
    /// which incorporates region-based dampening effects. When no modifiers
    /// are active this returns `1.0`.
    pub fn radar_dampening_multiplier(&self, range_modifier: f32) -> f32 {
        range_modifier
    }

    /// Parse a `RadarConfig` from a TOML string.
    ///
    /// Unknown tag strings are silently dropped so that future extensions do
    /// not break existing configurations.
    ///
    /// # Errors
    /// Returns a `String` error description if the TOML is malformed.
    pub fn from_toml(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| e.to_string())
    }
}

impl<'de> Deserialize<'de> for RadarConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRadarConfig::deserialize(deserializer)?;
        let shows = parse_tags(&raw.shows);
        let selects = parse_tags(&raw.selects);
        Ok(RadarConfig {
            range: raw.range,
            shows,
            selects,
        })
    }
}

#[cfg(test)]
#[path = "radar_config_tests.rs"]
mod tests;
