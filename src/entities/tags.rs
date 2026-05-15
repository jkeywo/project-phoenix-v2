// Pure-Rust entity tag system.
//
// `EntityTag` is a typed alternative to the raw `Vec<String>` tags used in
// `EntityConfig`.  The radar filtering API in `radar.rs` accepts slices of
// `EntityTag` so callers do not need to hand-craft string comparisons.

use serde::{Deserialize, Serialize};

/// A semantic label that can be attached to an entity.
///
/// Tags are additive — an entity may carry any number of them.  Radar
/// filtering uses **OR** logic: an entity matches a filter set if it carries
/// *at least one* of the requested tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityTag {
    Asteroid,
    Ship,
    AsteroidField,
    Star,
    Planet,
    Region,
}

impl EntityTag {
    /// Parse a lower-case string tag name into an `EntityTag`.
    ///
    /// Returns `None` for unrecognised strings so callers can gracefully
    /// ignore future extensions without breaking existing configs.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "asteroid" => Some(EntityTag::Asteroid),
            "ship" => Some(EntityTag::Ship),
            "asteroid_field" => Some(EntityTag::AsteroidField),
            "star" => Some(EntityTag::Star),
            "planet" => Some(EntityTag::Planet),
            "region" => Some(EntityTag::Region),
            _ => None,
        }
    }

    /// Convert back to the canonical lower-case string form.
    pub fn as_str(self) -> &'static str {
        match self {
            EntityTag::Asteroid => "asteroid",
            EntityTag::Ship => "ship",
            EntityTag::AsteroidField => "asteroid_field",
            EntityTag::Star => "star",
            EntityTag::Planet => "planet",
            EntityTag::Region => "region",
        }
    }
}

/// Parse a slice of raw string tags into a `Vec<EntityTag>`, silently
/// dropping strings that are not recognised.
pub fn parse_tags(raw: &[String]) -> Vec<EntityTag> {
    raw.iter().filter_map(|s| EntityTag::from_str(s)).collect()
}

/// Returns `true` if `entity_tags` contains **at least one** tag from
/// `filter_tags` (OR logic).  Returns `false` if `filter_tags` is empty.
pub fn matches_any(entity_tags: &[EntityTag], filter_tags: &[EntityTag]) -> bool {
    if filter_tags.is_empty() {
        return false;
    }
    entity_tags.iter().any(|t| filter_tags.contains(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── EntityTag::from_str ────────────────────────────────────────────────

    #[test]
    fn known_tag_strings_round_trip() {
        let cases = [
            ("asteroid", EntityTag::Asteroid),
            ("ship", EntityTag::Ship),
            ("asteroid_field", EntityTag::AsteroidField),
            ("star", EntityTag::Star),
            ("planet", EntityTag::Planet),
            ("region", EntityTag::Region),
        ];
        for (s, expected) in cases {
            assert_eq!(EntityTag::from_str(s), Some(expected), "from_str({s:?})");
            assert_eq!(expected.as_str(), s, "as_str({expected:?})");
        }
    }

    #[test]
    fn unknown_tag_string_returns_none() {
        assert_eq!(EntityTag::from_str("wormhole"), None);
        assert_eq!(EntityTag::from_str(""), None);
        assert_eq!(EntityTag::from_str("Asteroid"), None); // case-sensitive
    }

    // ── parse_tags ─────────────────────────────────────────────────────────

    #[test]
    fn parse_tags_converts_known_strings() {
        let raw = vec!["asteroid".to_string(), "ship".to_string()];
        let tags = parse_tags(&raw);
        assert_eq!(tags, vec![EntityTag::Asteroid, EntityTag::Ship]);
    }

    #[test]
    fn parse_tags_drops_unknown_strings() {
        let raw = vec!["asteroid".to_string(), "wormhole".to_string()];
        let tags = parse_tags(&raw);
        assert_eq!(tags, vec![EntityTag::Asteroid]);
    }

    #[test]
    fn parse_tags_empty_input_returns_empty() {
        assert!(parse_tags(&[]).is_empty());
    }

    // ── matches_any ────────────────────────────────────────────────────────

    #[test]
    fn matches_any_returns_true_when_at_least_one_tag_matches() {
        let entity = vec![EntityTag::Asteroid, EntityTag::Region];
        let filter = vec![EntityTag::Ship, EntityTag::Asteroid];
        assert!(matches_any(&entity, &filter));
    }

    #[test]
    fn matches_any_returns_false_when_no_tags_match() {
        let entity = vec![EntityTag::Asteroid];
        let filter = vec![EntityTag::Ship, EntityTag::Star];
        assert!(!matches_any(&entity, &filter));
    }

    #[test]
    fn matches_any_empty_filter_returns_false() {
        let entity = vec![EntityTag::Asteroid];
        assert!(!matches_any(&entity, &[]));
    }

    #[test]
    fn matches_any_empty_entity_tags_returns_false() {
        let filter = vec![EntityTag::Asteroid];
        assert!(!matches_any(&[], &filter));
    }

    #[test]
    fn matches_any_both_empty_returns_false() {
        assert!(!matches_any(&[], &[]));
    }
}
