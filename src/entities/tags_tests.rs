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
        ("moon", EntityTag::Moon),
        ("region", EntityTag::Region),
        ("structure", EntityTag::Structure),
        ("station", EntityTag::Station),
        ("player", EntityTag::Player),
        ("missile", EntityTag::Missile),
        ("trigger_volume", EntityTag::TriggerVolume),
        ("objective_marker", EntityTag::ObjectiveMarker),
        ("gm_removable", EntityTag::GmRemovable),
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

// ── trigger volumes are excluded from navigational radars ──────────────

#[test]
fn trigger_volume_not_matched_by_navigation_chart_filter() {
    // The navigation chart shows Star, Planet, AsteroidField, Region.
    // A pure trigger volume carries only `TriggerVolume`, so it must not
    // match (otherwise it would appear on radars).
    let nav_filter = vec![
        EntityTag::Star,
        EntityTag::Planet,
        EntityTag::AsteroidField,
        EntityTag::Region,
    ];
    let trigger_volume = vec![EntityTag::TriggerVolume];
    assert!(!matches_any(&trigger_volume, &nav_filter));
}
