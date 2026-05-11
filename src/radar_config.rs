// Pure-Rust radar configuration module.
//
// `RadarConfig` holds the per-console radar parameters: detection range and a
// tag-based entity filter.  Instances are loaded from the ship entity TOML
// (e.g. `[helm_console.radar]`, `[weapons_console.radar]`).
//
// This module has no Bevy dependency — it is fully unit-testable on native.

use serde::Deserialize;
use crate::entity_tags::EntityTag;

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
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RadarConfig {
    /// Maximum detection range in world units.
    pub range: f32,
    /// Tag filter: only entities whose tags overlap this list are displayed.
    /// Uses OR logic — an entity must match **at least one** tag.
    /// An empty list means nothing is shown.
    pub shows: Vec<EntityTag>,
}

impl Default for RadarConfig {
    fn default() -> Self {
        Self {
            range: 50.0,
            shows: vec![EntityTag::Asteroid],
        }
    }
}

/// Intermediate deserialisation type for TOML parsing.
#[derive(Deserialize)]
struct RawRadarConfig {
    #[serde(default = "default_range")]
    range: f32,
    #[serde(default)]
    shows: Vec<String>,
}

fn default_range() -> f32 {
    50.0
}

impl RadarConfig {
    /// Parse a `RadarConfig` from a TOML string.
    ///
    /// Unknown tag strings are silently dropped so that future extensions do
    /// not break existing configurations.
    ///
    /// # Errors
    /// Returns a `String` error description if the TOML is malformed.
    pub fn from_toml(toml_str: &str) -> Result<Self, String> {
        let raw: RawRadarConfig = toml::from_str(toml_str).map_err(|e| e.to_string())?;
        let shows = raw
            .shows
            .iter()
            .filter_map(|s| EntityTag::from_str(s))
            .collect();
        Ok(RadarConfig {
            range: raw.range,
            shows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── RadarConfig::from_toml ─────────────────────────────────────────────

    #[test]
    fn parse_range_and_shows() {
        let toml = r#"
range = 50.0
shows = ["asteroid"]
"#;
        let cfg = RadarConfig::from_toml(toml).expect("parse must succeed");
        assert_eq!(cfg.range, 50.0);
        assert_eq!(cfg.shows, vec![EntityTag::Asteroid]);
    }

    #[test]
    fn parse_multiple_tags() {
        let toml = r#"
range = 60.0
shows = ["asteroid", "ship"]
"#;
        let cfg = RadarConfig::from_toml(toml).expect("parse must succeed");
        assert_eq!(cfg.range, 60.0);
        assert_eq!(cfg.shows, vec![EntityTag::Asteroid, EntityTag::Ship]);
    }

    #[test]
    fn empty_toml_uses_defaults() {
        let cfg = RadarConfig::from_toml("").expect("empty TOML should use defaults");
        assert_eq!(cfg.range, 50.0);
        assert!(cfg.shows.is_empty());
    }

    #[test]
    fn unknown_tag_strings_are_dropped() {
        let toml = r#"
range = 40.0
shows = ["asteroid", "wormhole", "ship"]
"#;
        let cfg = RadarConfig::from_toml(toml).expect("parse must succeed");
        assert_eq!(cfg.shows, vec![EntityTag::Asteroid, EntityTag::Ship]);
    }

    #[test]
    fn malformed_toml_returns_error() {
        let result = RadarConfig::from_toml("range = [[[");
        assert!(result.is_err());
    }

    #[test]
    fn range_only_toml_has_empty_shows() {
        let toml = "range = 75.0\n";
        let cfg = RadarConfig::from_toml(toml).expect("parse must succeed");
        assert_eq!(cfg.range, 75.0);
        assert!(cfg.shows.is_empty());
    }

    #[test]
    fn all_known_tags_parse_correctly() {
        let toml = r#"
range = 100.0
shows = ["asteroid", "ship", "asteroid_field", "star", "planet", "region"]
"#;
        let cfg = RadarConfig::from_toml(toml).expect("parse must succeed");
        assert_eq!(cfg.shows.len(), 6);
        assert!(cfg.shows.contains(&EntityTag::Asteroid));
        assert!(cfg.shows.contains(&EntityTag::Ship));
        assert!(cfg.shows.contains(&EntityTag::AsteroidField));
        assert!(cfg.shows.contains(&EntityTag::Star));
        assert!(cfg.shows.contains(&EntityTag::Planet));
        assert!(cfg.shows.contains(&EntityTag::Region));
    }

    // ── RadarConfig::default ───────────────────────────────────────────────

    #[test]
    fn default_range_is_fifty() {
        let cfg = RadarConfig::default();
        assert_eq!(cfg.range, 50.0);
    }

    #[test]
    fn default_shows_asteroid() {
        let cfg = RadarConfig::default();
        assert_eq!(cfg.shows, vec![EntityTag::Asteroid]);
    }
}
