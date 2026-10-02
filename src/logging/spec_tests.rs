use super::*;

#[test]
fn empty_spec_is_the_default() {
    let cfg = parse_log_spec("").unwrap();
    assert_eq!(cfg.default_level, LevelFilter::Warn);
    assert!(cfg.per_cat.is_empty());
}

#[test]
fn bare_level_sets_the_default() {
    let cfg = parse_log_spec("debug").unwrap();
    assert_eq!(cfg.default_level, LevelFilter::Debug);
}

#[test]
fn mixed_spec_sets_default_and_overrides() {
    let cfg = parse_log_spec("info,ai=debug,admit=trace,physics=off").unwrap();
    assert_eq!(cfg.default_level, LevelFilter::Info);
    assert_eq!(cfg.per_cat[&LogCat::Ai], LevelFilter::Debug);
    assert_eq!(cfg.per_cat[&LogCat::Admit], LevelFilter::Trace);
    assert_eq!(cfg.per_cat[&LogCat::Physics], LevelFilter::Off);
    assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Debug));
    assert!(!cfg.cat_enabled(LogCat::Physics, LevelFilter::Error));
    // Unlisted category falls back to the spec's default.
    assert!(cfg.cat_enabled(LogCat::Helm, LevelFilter::Info));
}

#[test]
fn whitespace_and_trailing_commas_are_tolerated() {
    let cfg = parse_log_spec(" info , ai = debug , ").unwrap();
    assert_eq!(cfg.default_level, LevelFilter::Info);
    assert_eq!(cfg.per_cat[&LogCat::Ai], LevelFilter::Debug);
}

#[test]
fn category_and_level_are_case_insensitive() {
    let cfg = parse_log_spec("AI=DEBUG").unwrap();
    assert_eq!(cfg.per_cat[&LogCat::Ai], LevelFilter::Debug);
}

#[test]
fn unknown_category_is_an_error() {
    assert_eq!(
        parse_log_spec("warpcore=debug").unwrap_err(),
        LogSpecError::UnknownCategory("warpcore".into())
    );
}

#[test]
fn unknown_level_is_an_error() {
    assert_eq!(
        parse_log_spec("ai=chatty").unwrap_err(),
        LogSpecError::UnknownLevel("chatty".into())
    );
}

#[test]
fn double_equals_is_malformed() {
    assert!(matches!(
        parse_log_spec("ai=debug=trace").unwrap_err(),
        LogSpecError::Malformed(_)
    ));
}

#[test]
fn entity_list_parses_and_trims() {
    let f = parse_log_entities("Ironveil, Ashrender").unwrap();
    assert_eq!(f.names, vec!["Ironveil", "Ashrender"]);
}

#[test]
fn empty_entity_list_means_no_filtering() {
    assert!(parse_log_entities("").is_none());
    assert!(parse_log_entities("  , ").is_none());
}

#[test]
fn every_category_round_trips_through_its_target_string() {
    use strum::IntoEnumIterator;
    for cat in LogCat::iter() {
        let spec = format!("{}=trace", cat.target());
        let cfg = parse_log_spec(&spec)
            .unwrap_or_else(|e| panic!("category {cat:?} failed to parse: {e}"));
        assert_eq!(cfg.per_cat[&cat], LevelFilter::Trace);
    }
}
