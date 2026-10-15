use super::*;

const SHIP: &str = r#"
[[station]]
id = "pilot-seat"
name = "Pilot"
description = "Flight"
rank = ""
[[station.rating]]
name = "Manual"
automated_systems = []

[[console]]
id = "flight-display"
home_station = "pilot-seat"
screen = "gui/helm.html"
[[console.feed]]
source = "phaser-left"
kind = "PhaserBank"

[[console]]
id = "diagnostic-display"
home_station = "pilot-seat"
screen = "gui/helm.html"

[[system]]
id = "phaser-left"
kind = "phaser_bank"
console = "flight-display"
"#;

#[test]
fn system_authority_resolves_through_console_not_an_identity_alias() {
    let config = ShipConfig::from_toml(SHIP, &["phaser_bank"]).unwrap();
    let target = SystemId("phaser-left".into());
    assert_eq!(
        config.console_for_system(&target).unwrap().id,
        ConsoleId("flight-display".into())
    );
    assert_eq!(
        crate::command_admission::policy::station_for_system(&config, None, &target),
        Some(StationId("pilot-seat".into()))
    );
    assert_eq!(
        config
            .systems_for_station(&StationId("pilot-seat".into()))
            .count(),
        1
    );
    assert_eq!(
        config
            .systems_for_station(&StationId("flight-display".into()))
            .count(),
        0
    );
}

#[test]
fn invalid_console_reference_is_refused_instead_of_falling_back() {
    let broken = SHIP.replace(
        "console = \"flight-display\"",
        "console = \"unknown\"\nstation = \"pilot-seat\"",
    );
    assert!(ShipConfig::from_toml(&broken, &["phaser_bank"]).is_err());
}

#[test]
fn duplicate_console_and_unknown_home_are_refused() {
    assert!(ShipConfig::from_toml(
        &SHIP.replace("id = \"diagnostic-display\"", "id = \"flight-display\""),
        &["phaser_bank"]
    )
    .is_err());
    assert!(ShipConfig::from_toml(
        &SHIP.replace("home_station = \"pilot-seat\"", "home_station = \"absent\""),
        &["phaser_bank"]
    )
    .is_err());
}

#[test]
fn ai_only_equipment_can_have_a_screenless_console() {
    let source = "[[console]]\nid = 'autonomy'\n[[system]]\nid = 'engine'\nkind = 'phaser_bank'\nconsole = 'autonomy'\nai_only = true\n";
    let config = ShipConfig::from_toml(source, &["phaser_bank"]).unwrap();
    assert!(config
        .console_for_system(&SystemId("engine".into()))
        .is_some());
    assert_eq!(
        crate::command_admission::policy::station_for_system(
            &config,
            None,
            &SystemId("engine".into())
        ),
        None
    );
}

#[test]
fn detached_console_preserves_own_rating_and_raises_for_scenario_floor() {
    let mut config: ShipConfig = toml::from_str(SHIP).unwrap();
    config.consoles[0].home_station = None;
    config.consoles[0].human_seeking = true;
    config.consoles[0].host_order = vec![StationId("pilot-seat".into())];
    config.consoles[0].visiting_rating = Some("Simplified".into());
    config.consoles[0].ratings = vec![
        super::super::config::StationRatingConfig {
            name: "Std".into(),
            automated_systems: vec![],
            detailed_systems: None,
            ai_tuning: None,
        },
        super::super::config::StationRatingConfig {
            name: "Simplified".into(),
            automated_systems: vec![SystemId("phaser-left".into())],
            detailed_systems: None,
            ai_tuning: None,
        },
    ];
    let console = config.console(&ConsoleId("flight-display".into())).unwrap();
    let active =
        std::collections::HashMap::from([(StationId("pilot-seat".into()), "Manual".into())]);
    let normal = resolve_assignment(&config, console, |_| true, &active, &Default::default());
    assert_eq!(normal.rating, "Simplified");
    assert_eq!(
        config.console_system_depths(console, &normal.rating)[&SystemId("phaser-left".into())],
        super::super::rating::SystemDepth::Ai
    );
    let required = std::collections::BTreeSet::from([SystemId("phaser-left".into())]);
    let raised = resolve_assignment(&config, console, |_| true, &active, &required);
    assert_eq!(raised.rating, "Std");
    assert_eq!(raised.host, Some(StationId("pilot-seat".into())));
}

#[test]
fn feeds_require_real_publication_sources_and_matching_shapes() {
    let valid = ShipConfig::from_toml(SHIP, &["phaser_bank"]).unwrap();
    let mut config = valid.clone();
    config.consoles[0].feeds[0].source = SystemId("flight-display".into());
    assert!(validate_consoles(&config)
        .unwrap_err()
        .to_string()
        .contains("unknown publication"));
    config.consoles[0].feeds[0].source = SystemId("phaser-left".into());
    config.consoles[0].feeds[0].kind = phoenix_model::console::ConsoleObservationKind::Helm;
    assert!(validate_consoles(&config)
        .unwrap_err()
        .to_string()
        .contains("has kind"));
    for (source, kind) in super::super::system_registry::fixed_publication_kinds() {
        config.consoles[0].feeds = vec![phoenix_model::console::ConsoleFeed {
            source: SystemId(source),
            kind,
        }];
        validate_consoles(&config).unwrap();
    }
    config.consoles[0].feeds.clear();
    config.systems[0].kind = "helm_thrust".into();
    config.consoles[0]
        .feeds
        .push(phoenix_model::console::ConsoleFeed {
            source: config.systems[0].id.clone(),
            kind: phoenix_model::console::ConsoleObservationKind::HelmEngine,
        });
    assert!(
        validate_consoles(&config).is_err(),
        "command-only Systems are not publication sources"
    );
}

#[test]
fn operable_equipment_requires_a_screen_even_with_a_home_seat() {
    let mut config: ShipConfig = toml::from_str(SHIP).unwrap();
    config.consoles[0].screen = None;
    assert!(validate_consoles(&config)
        .unwrap_err()
        .to_string()
        .contains("Console screen"));
    config.systems[0].ai_only = true;
    validate_consoles(&config).unwrap();
}

#[test]
fn detached_console_checks_each_candidates_proposed_rating_before_placement() {
    let mut config: ShipConfig = toml::from_str(SHIP).unwrap();
    let mut other = config.stations[0].clone();
    other.id = StationId("second-seat".into());
    config.stations.push(other);
    let console = &mut config.consoles[0];
    console.home_station = None;
    console.human_seeking = true;
    console.host_order = vec![
        StationId("pilot-seat".into()),
        StationId("second-seat".into()),
    ];
    console.visiting_rating = Some("Simplified".into());
    console.ratings = vec![
        super::super::config::StationRatingConfig {
            name: "Std".into(),
            automated_systems: vec![],
            detailed_systems: None,
            ai_tuning: None,
        },
        super::super::config::StationRatingConfig {
            name: "Simplified".into(),
            automated_systems: vec![SystemId("phaser-left".into())],
            detailed_systems: None,
            ai_tuning: None,
        },
    ];
    let console = &config.consoles[0];
    let eligible =
        |seat: &StationId, rating: &str| seat.0 != "pilot-seat" || rating == "Simplified";
    let ordinary = resolve_assignment_with_eligibility(
        &config,
        console,
        eligible,
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(ordinary.host, Some(StationId("pilot-seat".into())));
    assert_eq!(ordinary.rating, "Simplified");
    let floor = std::collections::BTreeSet::from([SystemId("phaser-left".into())]);
    let raised = resolve_assignment_with_eligibility(
        &config,
        console,
        eligible,
        &Default::default(),
        &floor,
    );
    assert_eq!(raised.host, Some(StationId("second-seat".into())));
    assert_eq!(raised.rating, "Std");
    for _ in 0..2 {
        let rejected = resolve_assignment_with_eligibility(
            &config,
            console,
            |_, rating| {
                assert_eq!(
                    rating, "Std",
                    "test the proposed rating, never rejected Backfill"
                );
                false
            },
            &Default::default(),
            &floor,
        );
        assert_eq!(rejected.host, None);
        assert_eq!(rejected.rating, super::super::rating::BACKFILL_RATING);
    }
}

#[test]
fn operable_seeking_console_requires_at_least_one_candidate_seat() {
    let mut config: ShipConfig = toml::from_str(SHIP).unwrap();
    config.consoles[0].home_station = None;
    config.consoles[0].human_seeking = true;
    config.consoles[0].host_order.clear();
    assert!(validate_consoles(&config)
        .unwrap_err()
        .to_string()
        .contains("human hosting route"));
    config.consoles[0]
        .host_order
        .push(StationId("pilot-seat".into()));
    validate_consoles(&config).unwrap();
}

#[test]
fn retired_station_hosting_fields_are_rejected_instead_of_silently_ignored() {
    for field in [
        "console = 'screen.html'",
        "human_seeking = true",
        "host_order = ['pilot-seat']",
        "visiting_rating = 'Manual'",
        "auxiliary = true",
        "command_target = 'pilot-seat'",
    ] {
        let source = SHIP.replace("rank = \"\"", &format!("rank = \"\"\n{field}"));
        assert!(
            ShipConfig::from_toml(&source, &["phaser_bank"]).is_err(),
            "{field} must move to Console"
        );
    }
}

#[test]
fn instance_publication_cannot_replace_an_incompatible_reserved_board() {
    let collision = SHIP.replace("phaser-left", "helm");
    let error = ShipConfig::from_toml(&collision, &["phaser_bank"]).unwrap_err();
    assert!(error.to_string().contains("reserved publication key"));
}

#[test]
fn supported_navigation_and_radar_instance_feeds_round_trip_with_actual_ids() {
    for (kind, publication) in [
        ("navigation", "Navigation"),
        ("sensor_radar", "SensorRadar"),
        ("tactical_radar", "TacticalRadar"),
    ] {
        let authored = SHIP
            .replace("phaser_bank", kind)
            .replace("PhaserBank", publication)
            .replace("phaser-left", "custom-instrument");
        let config = ShipConfig::from_toml(&authored, &[kind]).unwrap();
        let encoded = toml::to_string(&config).unwrap();
        let restored = ShipConfig::from_toml(&encoded, &[kind]).unwrap();
        assert_eq!(restored, config);
        assert_eq!(restored.consoles[0].feeds[0].source.0, "custom-instrument");
        assert_eq!(restored.systems[0].id.0, "custom-instrument");
        let wrong_kind = authored.replace(&format!("kind = \"{publication}\""), "kind = \"Dock\"");
        assert!(ShipConfig::from_toml(&wrong_kind, &[kind]).is_err());
    }
}
