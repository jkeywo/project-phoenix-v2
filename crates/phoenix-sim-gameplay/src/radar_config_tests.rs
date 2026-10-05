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
    assert!(cfg.selects.is_empty());
}

#[test]
fn parse_selects() {
    let toml = r#"
range = 60.0
shows = ["asteroid", "ship"]
selects = ["ship"]
"#;
    let cfg = RadarConfig::from_toml(toml).expect("parse must succeed");
    assert_eq!(cfg.range, 60.0);
    assert_eq!(cfg.shows, vec![EntityTag::Asteroid, EntityTag::Ship]);
    assert_eq!(cfg.selects, vec![EntityTag::Ship]);
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
    assert!(cfg.selects.is_empty());
}

#[test]
fn empty_toml_uses_defaults() {
    let cfg = RadarConfig::from_toml("").expect("empty TOML should use defaults");
    assert_eq!(cfg.range, 50.0);
    assert!(cfg.shows.is_empty());
    assert!(cfg.selects.is_empty());
}

#[test]
fn unknown_tag_strings_are_dropped() {
    let toml = r#"
range = 40.0
shows = ["asteroid", "wormhole", "ship"]
"#;
    let cfg = RadarConfig::from_toml(toml).expect("parse must succeed");
    assert_eq!(cfg.shows, vec![EntityTag::Asteroid, EntityTag::Ship]);
    assert!(cfg.selects.is_empty());
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

#[test]
fn moon_survives_the_shows_filter() {
    // The hulls' `[navigation_console.system_chart] shows` list carries
    // "moon". An unparseable string is dropped in silence here, so a
    // missing `EntityTag` variant would empty the chart of moons with no
    // load error to show for it.
    let toml = r#"
range = 800.0
shows = ["planet", "moon"]
"#;
    let cfg = RadarConfig::from_toml(toml).expect("parse must succeed");
    assert_eq!(cfg.shows, vec![EntityTag::Planet, EntityTag::Moon]);
}

// ── RadarConfig::radar_dampening_multiplier ───────────────────────────

#[test]
fn radar_dampening_multiplier_returns_range_modifier() {
    let cfg = RadarConfig::default();
    assert!((cfg.radar_dampening_multiplier(0.7) - 0.7).abs() < 1e-6);
    assert!((cfg.radar_dampening_multiplier(1.0) - 1.0).abs() < 1e-6);
    assert!((cfg.radar_dampening_multiplier(0.3) - 0.3).abs() < 1e-6);
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
