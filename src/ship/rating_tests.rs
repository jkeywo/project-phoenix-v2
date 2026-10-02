use super::*;
use crate::ship::config::parse_and_validate;

#[test]
fn floor_raises_individual_systems_without_selecting_a_different_rung() {
    let mut config = parse_and_validate(valid_toml(), KINDS).unwrap();
    let station = StationId("captain".into());
    let red = SystemId("red-alert".into());
    let view = SystemId("viewscreen".into());
    let rung = &mut config
        .stations
        .iter_mut()
        .find(|s| s.id == station)
        .unwrap()
        .ratings[0];
    rung.automated_systems = vec![red.clone()];
    rung.detailed_systems = Some(vec![]);
    let before = config.clone();
    let floor = HashMap::from([(red.clone(), SystemDepth::Detailed)]);
    let result = resolve_system_depths(&config, &station, "Assisted", &floor).unwrap();
    assert_eq!(result[&red], SystemDepth::Detailed);
    assert_eq!(result[&view], SystemDepth::Simplified);
    assert_eq!(
        config, before,
        "a between-rung result must not rewrite the chosen rung"
    );
    let lower = resolve_system_depths(&config, &station, "Manual", &floor).unwrap();
    assert!(lower.values().all(|d| *d == SystemDepth::Detailed));
    let backfill =
        resolve_system_depths(&config, &station, BACKFILL_RATING, &HashMap::new()).unwrap();
    assert!(backfill.values().all(|d| *d == SystemDepth::Ai));
    assert!(resolve_system_depths(&config, &station, "unknown", &floor).is_none());
}

#[test]
fn explicit_depth_lists_reject_dangling_unowned_and_conflicting_systems() {
    for (id, expected) in [
        ("missing", "DanglingRatingReference"),
        ("phaser-fore", "RatingReferencesUnownedSystem"),
        ("red-alert", "ConflictingRatingDepth"),
    ] {
        let mut config = parse_and_validate(valid_toml(), KINDS).unwrap();
        config.stations[0].ratings[0].detailed_systems = Some(vec![SystemId(id.into())]);
        let error = crate::ship::config::validate(&config, KINDS).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn legacy_ratings_preserve_their_sources_and_default_order() {
    let config = parse_and_validate(valid_toml(), KINDS).unwrap();
    for station in &config.stations {
        for rung in &station.ratings {
            let depths =
                resolve_system_depths(&config, &station.id, &rung.name, &HashMap::new()).unwrap();
            for (id, depth) in depths {
                assert_eq!(
                    depth,
                    if rung.automated_systems.contains(&id) {
                        SystemDepth::Ai
                    } else {
                        SystemDepth::Detailed
                    }
                );
            }
        }
    }
    assert_eq!(config.stations[0].ratings[0].name, "Assisted");
}

const KINDS: &[&str] = &[
    "red_alert",
    "helm",
    "phaser_bank",
    "torpedo_magazine",
    "torpedo_tube",
    "viewscreen",
];

fn valid_toml() -> &'static str {
    r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."
short_code = "CPT"

[[station.rating]]
name = "Assisted"
automated_systems = ["red-alert", "viewscreen"]

[[station.rating]]
name = "Manual"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons and threat response."
rank = "Ltn."
short_code = "TAC"

[[station.rating]]
name = "Assisted"
automated_systems = ["torpedo-magazine", "torpedo-tube-fore-port"]

[[station.rating]]
name = "FullAuto"
automated_systems = ["phaser-fore", "torpedo-magazine", "torpedo-tube-fore-port"]

[power_groups.ops]
label = "Operations"
default_level = 2
min_level = 1
max_level = 4

[power_groups.weapons]
label = "Weapons"
default_level = 2
min_level = 1
max_level = 4

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
power_group = "ops"

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
power_group = "weapons"

[system.config]
facing_deg = 0
fire_arc_deg = 270

[[system]]
id = "torpedo-magazine"
kind = "torpedo_magazine"
station = "tactical"
power_group = "weapons"

[[system]]
id = "torpedo-tube-fore-port"
kind = "torpedo_tube"
station = "tactical"
power_group = "weapons"

[[system]]
id = "viewscreen"
kind = "viewscreen"
station = "captain"
power_group = "ops"
"#
}

fn parse() -> ShipConfig {
    parse_and_validate(valid_toml(), KINDS).expect("ship config should parse")
}

// ── resolve_automated_systems ───────────────────────────────────────

#[test]
fn resolves_explicit_rating_systems() {
    let config = parse();
    let result = resolve_automated_systems(&config, &StationId("tactical".into()), "Assisted");
    assert_eq!(
        result,
        Some(vec![
            SystemId("torpedo-magazine".into()),
            SystemId("torpedo-tube-fore-port".into()),
        ])
    );
}

#[test]
fn resolves_empty_rating_systems() {
    let config = parse();
    let result = resolve_automated_systems(&config, &StationId("captain".into()), "Manual");
    assert_eq!(result, Some(vec![]));
}

#[test]
fn backfill_returns_all_station_systems() {
    let config = parse();
    let result = resolve_automated_systems(&config, &StationId("tactical".into()), BACKFILL_RATING);
    let expected: Vec<SystemId> = vec![
        SystemId("phaser-fore".into()),
        SystemId("torpedo-magazine".into()),
        SystemId("torpedo-tube-fore-port".into()),
    ];
    assert_eq!(result, Some(expected));
}

#[test]
fn unknown_rating_returns_none() {
    let config = parse();
    let result = resolve_automated_systems(&config, &StationId("captain".into()), "Bogus");
    assert_eq!(result, None);
}

#[test]
fn unknown_station_returns_none() {
    let config = parse();
    let result = resolve_automated_systems(&config, &StationId("ghost".into()), "Assisted");
    assert_eq!(result, None);
}

#[test]
fn varying_rating_counts_per_station() {
    let config = parse();
    // captain has 2 ratings
    let captain = config.station(&StationId("captain".into())).unwrap();
    assert_eq!(captain.ratings.len(), 2);
    // tactical has 2 ratings
    let tactical = config.station(&StationId("tactical".into())).unwrap();
    assert_eq!(tactical.ratings.len(), 2);
}

// ── available_ratings_for_station ───────────────────────────────────

#[test]
fn available_ratings_includes_backfill() {
    let config = parse();
    let ratings = available_ratings_for_station(&config, &StationId("captain".into()));
    assert!(ratings.contains(&"Assisted"));
    assert!(ratings.contains(&"Manual"));
    assert!(ratings.contains(&BACKFILL_RATING));
}

// ── apply_rating ────────────────────────────────────────────────────

#[test]
fn apply_rating_sets_ai_for_automated_systems() {
    let config = parse();
    let mut resolver = ControlSourceResolver::new();
    let station = StationId("tactical".into());

    // Before: all human
    assert_eq!(
        resolver.source_for(&SystemId("phaser-fore".into())),
        ControlSource::Human
    );

    apply_rating(&config, &station, "FullAuto", &mut resolver);

    // Automated systems → Ai
    assert_eq!(
        resolver.source_for(&SystemId("phaser-fore".into())),
        ControlSource::Ai
    );
    assert_eq!(
        resolver.source_for(&SystemId("torpedo-magazine".into())),
        ControlSource::Ai
    );
}

#[test]
fn apply_rating_restores_human_for_unlisted_systems() {
    let config = parse();
    let mut resolver = ControlSourceResolver::new();
    let station = StationId("tactical".into());

    // Pre-set everything to Ai
    for system in config.systems_for_station(&station) {
        resolver.set(system.id.clone(), ControlSource::Ai);
    }

    // Apply Assisted rating — only torpedo-magazine and torpedo-tube-fore-port stay Ai
    apply_rating(&config, &station, "Assisted", &mut resolver);

    assert_eq!(
        resolver.source_for(&SystemId("torpedo-magazine".into())),
        ControlSource::Ai
    );
    assert_eq!(
        resolver.source_for(&SystemId("phaser-fore".into())),
        ControlSource::Human
    );
}

#[test]
fn apply_rating_does_not_affect_other_station_systems() {
    let config = parse();
    let mut resolver = ControlSourceResolver::new();

    // Pre-set tactical systems to Ai
    for system in config.systems_for_station(&StationId("tactical".into())) {
        resolver.set(system.id.clone(), ControlSource::Ai);
    }

    // Apply Manual rating on captain (empty automated list)
    apply_rating(
        &config,
        &StationId("captain".into()),
        "Manual",
        &mut resolver,
    );

    // Tactical systems should be unaffected
    assert_eq!(
        resolver.source_for(&SystemId("phaser-fore".into())),
        ControlSource::Ai
    );
}

#[test]
fn apply_rating_unknown_station_is_noop() {
    let config = parse();
    let mut resolver = ControlSourceResolver::new();
    let before = resolver.clone();

    apply_rating(
        &config,
        &StationId("ghost".into()),
        "Assisted",
        &mut resolver,
    );
    assert_eq!(resolver, before);
}

#[test]
fn backfill_rating_sets_all_station_systems_to_ai() {
    let config = parse();
    let mut resolver = ControlSourceResolver::new();
    let station = StationId("tactical".into());

    apply_rating(&config, &station, BACKFILL_RATING, &mut resolver);

    for system in config.systems_for_station(&station) {
        assert_eq!(
            resolver.source_for(&system.id),
            ControlSource::Ai,
            "system {} should be Ai under backfill",
            system.id.0
        );
    }
}

// ── ai_only_systems ─────────────────────────────────────────────────

#[test]
fn returns_no_ai_only_systems_after_viewscreen_moved_to_captain() {
    let config = parse();
    let systems = ai_only_systems(&config);
    assert!(
        systems.is_empty(),
        "viewscreen is now owned by captain — no ai_only systems remain"
    );
}

// ── captain rating includes viewscreen ───────────────────────────────

#[test]
fn captain_assisted_rating_includes_viewscreen() {
    let config = parse();
    let result = resolve_automated_systems(&config, &StationId("captain".into()), "Assisted");
    assert_eq!(
        result,
        Some(vec![
            SystemId("red-alert".into()),
            SystemId("viewscreen".into()),
        ])
    );
}

#[test]
fn captain_backfill_automates_red_alert_and_viewscreen() {
    let config = parse();
    let mut resolver = ControlSourceResolver::new();
    let station = StationId("captain".into());

    apply_rating(&config, &station, BACKFILL_RATING, &mut resolver);

    assert_eq!(
        resolver.source_for(&SystemId("red-alert".into())),
        ControlSource::Ai,
    );
    assert_eq!(
        resolver.source_for(&SystemId("viewscreen".into())),
        ControlSource::Ai,
    );
}

#[test]
fn captain_manual_rating_leaves_viewscreen_human() {
    let config = parse();
    let mut resolver = ControlSourceResolver::new();

    // Pre-set viewscreen to Ai
    resolver.set(SystemId("viewscreen".into()), ControlSource::Ai);

    apply_rating(
        &config,
        &StationId("captain".into()),
        "Manual",
        &mut resolver,
    );

    assert_eq!(
        resolver.source_for(&SystemId("viewscreen".into())),
        ControlSource::Human,
        "Manual rating should restore viewscreen to Human"
    );
}

// ── seed_boot_ratings (issue #871) ───────────────────────────────────

/// The behaviour-PRESERVATION proof for #871.
///
/// Before #871, `entities::spawner::spawn_entity` seeded a `[behaviour]`
/// hull by setting **every declared system** to `ControlSource::Ai` in a
/// blanket loop, and NPC hulls declared no stations at all. This test
/// asserts the shared boot path reproduces that result exactly, for every
/// shipped hull: with nobody connected, every system in the hull's config —
/// station-owned or auto-generated — resolves to `Ai`.
///
/// It runs over EVERY hull in `assets/entities/`, not just the NPC ones,
/// because the `alliance_*` hulls are spawned through this same path
/// whenever a world (or the duel harness) spawns one as an opponent. If a
/// station stops owning a system, or an `ai_only` system stops being
/// covered by the second pass, the system silently falls to the resolver's
/// `Human` default and its AI host stops running — which is exactly the
/// failure mode this issue had to avoid.
#[test]
fn every_shipped_hull_boots_fully_ai_when_nobody_is_connected() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/entities");
    let mut checked_hulls = 0usize;
    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .expect("assets/entities must be readable")
        .map(|e| e.expect("readable dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    entries.sort();

    for path in entries {
        let stem = path
            .file_stem()
            .expect("toml file has a stem")
            .to_string_lossy()
            .to_string();
        // Through the include resolver (issue #906) so this keeps covering
        // a hull once it is composed.
        let key = path.to_string_lossy().replace('\\', "/");
        let config = crate::entities::include_resolve::load_entity_config(&key)
            .unwrap_or_else(|e| panic!("{stem} must parse: {e}"));
        let Some(ship_config) = config.ship_config.as_ref() else {
            continue; // scenery: no stations, no systems, nothing to seed
        };
        checked_hulls += 1;

        let (resolver, active_ratings) =
            seed_boot_ratings(ship_config, |_| BACKFILL_RATING.to_string());

        for system in &ship_config.systems {
            let policy = resolver.policy_for(&system.id);
            assert!(
                policy.operate_ai,
                "{stem}: system {:?} must be AI-operated on an unmanned hull. \
                     Owner: {:?}, ai_only: {}. An unowned, non-`ai_only` system \
                     falls to the resolver's Human default and its AI host stops \
                     running.",
                system.id, system.station, system.ai_only
            );
            assert!(
                !policy.accept_human_input,
                "{stem}: system {:?} must not accept human input while unmanned",
                system.id
            );
        }

        for station in &ship_config.stations {
            assert_eq!(
                active_ratings.get(&station.id).map(String::as_str),
                Some(BACKFILL_RATING),
                "{stem}: station {:?} must report Backfill when nobody is connected",
                station.id
            );
        }
    }

    // Nine since issue #954 moved the three-weapon RNG-coverage escort out of
    // `assets/entities/` into the test-fixture directory this walk (top level
    // only, like every other fleet walk) no longer reads. Ten before that,
    // since issue #892 retired `pirate_raider.toml` and
    // `pirate_raider_reinforcement.toml` as display-name duplicates of
    // `ship_harrow_destroyer.toml`. The floor is a "did the scan actually
    // find the hulls?" guard, so it tracks the shipped count down — but only
    // for a hull that genuinely LEFT the fleet. It must never be lowered to
    // accommodate a hull that stopped parsing.
    assert!(
        checked_hulls >= 9,
        "expected every shipped hull to be checked, got {checked_hulls}"
    );
}

/// The other half of the seat symmetry: a manned station's systems come up
/// on that station's authored rating, NOT on Backfill — which is what makes
/// a human at an NPC seat admissible at all.
#[test]
fn a_manned_station_boots_on_its_own_rating_and_leaves_its_systems_human() {
    let config = parse();
    let captain = StationId("captain".into());

    let (resolver, active_ratings) = seed_boot_ratings(&config, |station| {
        if station.id == captain {
            "Manual".to_string()
        } else {
            BACKFILL_RATING.to_string()
        }
    });

    assert_eq!(
        active_ratings.get(&captain).map(String::as_str),
        Some("Manual")
    );
    assert_eq!(
        resolver.source_for(&SystemId("red-alert".into())),
        ControlSource::Human,
        "the Manual rating automates nothing, so the seated human drives red-alert"
    );
    assert_eq!(
        resolver.source_for(&SystemId("phaser-fore".into())),
        ControlSource::Ai,
        "the unmanned Tactical station stays backfilled"
    );
}

/// `ai_only` systems are ownerless by construction, so no station rating can
/// reach them. The second pass in `seed_boot_ratings` is what keeps them
/// AI-operated; without it they fall to the resolver's `Human` default.
#[test]
fn ownerless_ai_only_systems_are_seeded_ai_even_with_no_stations() {
    let config = crate::ship::config::parse_and_validate(
        r#"
[[system]]
id = "shield-arc-all"
kind = "shield_arc"
ai_only = true

[[system]]
id = "red-alert"
kind = "red_alert"
ai_only = true
"#,
        &["shield_arc", "red_alert"],
    )
    .expect("fixture must validate");

    let (resolver, active_ratings) = seed_boot_ratings(&config, |_| BACKFILL_RATING.to_string());

    assert!(active_ratings.is_empty(), "no stations, so no ratings");
    for id in ["shield-arc-all", "red-alert"] {
        assert_eq!(
            resolver.source_for(&SystemId(id.into())),
            ControlSource::Ai,
            "{id} is ownerless + ai_only, so only the second pass can reach it"
        );
    }
}
