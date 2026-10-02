use super::*;

#[test]
fn default_level_applies_to_unlisted_categories() {
    let cfg = LogFilterConfig {
        default_level: LevelFilter::Info,
        ..Default::default()
    };
    assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Warn));
    assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Info));
    assert!(!cfg.cat_enabled(LogCat::Ai, LevelFilter::Debug));
}

#[test]
fn per_category_override_beats_default() {
    let mut per_cat = empty_per_cat();
    per_cat.insert(LogCat::Ai, LevelFilter::Trace);
    per_cat.insert(LogCat::Physics, LevelFilter::Off);
    let cfg = LogFilterConfig {
        default_level: LevelFilter::Warn,
        per_cat,
        entity_filter: None,
    };
    assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Trace));
    assert!(!cfg.cat_enabled(LogCat::Physics, LevelFilter::Error));
    // Unlisted category still gets the default.
    assert!(cfg.cat_enabled(LogCat::Helm, LevelFilter::Warn));
    assert!(!cfg.cat_enabled(LogCat::Helm, LevelFilter::Info));
}

#[test]
fn off_suppresses_every_level() {
    assert!(!LevelFilter::Off.allows(LevelFilter::Error));
    assert!(!LevelFilter::Trace.allows(LevelFilter::Off));
}

#[test]
fn no_entity_filter_allows_everything() {
    let cfg = LogFilterConfig::default();
    assert!(cfg.entity_allowed(Entity::from_raw_u32(7).unwrap()));
}

#[test]
fn entity_filter_matches_exactly_then_case_insensitive_substring() {
    let f = EntityFilter::new(vec!["Ironveil".into()]);
    assert!(f.matches_name("Ironveil"));
    assert!(f.matches_name("ironveil"));
    assert!(f.matches_name("USS Ironveil Mk II"));
    assert!(!f.matches_name("Ashrender"));
}

#[test]
fn entity_filter_denies_unresolved_entities() {
    let cfg = LogFilterConfig {
        entity_filter: Some(EntityFilter::new(vec!["Ironveil".into()])),
        ..Default::default()
    };
    // Nothing resolved yet, so nothing passes.
    assert!(!cfg.entity_allowed(Entity::from_raw_u32(7).unwrap()));
}

#[test]
fn refresh_adds_matching_and_drops_despawned() {
    let mut app = App::new();
    app.insert_resource(LogFilterConfig {
        entity_filter: Some(EntityFilter::new(vec!["Ironveil".into()])),
        ..Default::default()
    });
    app.add_systems(Update, refresh_log_entity_filter);

    let hit = app.world_mut().spawn(EntityName("Ironveil".into())).id();
    let miss = app.world_mut().spawn(EntityName("Ashrender".into())).id();
    app.update();

    let cfg = app.world().resource::<LogFilterConfig>();
    assert!(cfg.entity_allowed(hit));
    assert!(!cfg.entity_allowed(miss));

    app.world_mut().despawn(hit);
    app.update();
    let cfg = app.world().resource::<LogFilterConfig>();
    assert!(!cfg.entity_allowed(hit));
}
