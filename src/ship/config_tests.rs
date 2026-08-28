use super::*;

const KINDS: &[&str] = &[
    "red_alert",
    "helm",
    "phaser_bank",
    "torpedo_magazine",
    "torpedo_tube",
    "viewscreen",
    "sensors",
    // The seek-order fixtures below author a real seeking system, and on
    // every shipped hull that is `comms`.
    "comms",
];

fn valid_toml() -> &'static str {
    r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."
short_code = "CPT"
console = "gui/captain-console.html"
manual_overview = "You command the bridge and set the ship's posture."

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
console = "gui/tactical-console.html"

[[station.rating]]
name = "Assisted"
automated_systems = ["torpedo-magazine", "torpedo-tube-fore-port"]

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
marker = "phasers_fore"

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

fn parse_ok(toml: &str) -> ShipConfig {
    parse_and_validate(toml, KINDS).expect("ship config should parse")
}

#[test]
fn coordination_lag_requires_a_finite_non_negative_duration() {
    for authored in ["-0.1", "nan", "inf"] {
        let toml = format!("coordination_lag_secs = {authored}\n{}", valid_toml());
        assert!(
            matches!(
                parse_and_validate(&toml, KINDS),
                Err(ShipConfigError::InvalidCoordinationLagSecs { .. })
            ),
            "coordination_lag_secs = {authored} must be rejected"
        );
    }

    let zero = format!("coordination_lag_secs = 0.0\n{}", valid_toml());
    assert_eq!(parse_ok(&zero).coordination_lag_secs, 0.0);
}

#[test]
fn new_ship_toml_parses_into_typed_model() {
    let config = parse_ok(valid_toml());

    assert_eq!(config.stations.len(), 2);
    assert_eq!(config.systems.len(), 5);
    assert_eq!(config.stations[1].id, StationId("tactical".into()));
    assert_eq!(config.stations[1].ratings[0].name, "Assisted");
    assert_eq!(
        config.stations[1].ratings[0].automated_systems,
        vec![
            SystemId("torpedo-magazine".into()),
            SystemId("torpedo-tube-fore-port".into())
        ]
    );
    assert_eq!(
        config.power_groups[&PowerGroupId("weapons".into())].label,
        "Weapons"
    );
}

#[test]
fn ship_config_round_trips_through_toml() {
    let config = parse_ok(valid_toml());
    let encoded = toml::to_string(&config).expect("ship config should serialize");
    let decoded = parse_ok(&encoded);

    assert_eq!(decoded, config);
}

#[test]
fn station_config_parses_manual_overview_field() {
    let config = parse_ok(valid_toml());

    let captain = config.station(&StationId("captain".into())).unwrap();
    assert_eq!(
        captain.manual_overview.as_deref(),
        Some("You command the bridge and set the ship's posture."),
    );
    // Absent on stations that authored none (issue #772 default).
    let tactical = config.station(&StationId("tactical".into())).unwrap();
    assert_eq!(tactical.manual_overview, None);
}

#[test]
fn station_config_manual_overview_survives_round_trip() {
    let config = parse_ok(valid_toml());
    let encoded = toml::to_string(&config).expect("ship config should serialize");
    let decoded = parse_ok(&encoded);

    assert_eq!(
        decoded
            .station(&StationId("captain".into()))
            .and_then(|s| s.manual_overview.clone()),
        Some("You command the bridge and set the ship's posture.".to_string()),
    );
}

// ── Contextual tutorial overlays (issue #916) ─────────────────────────

/// A ship config authoring `[[station.tutorial]]` blocks on one station:
/// one per shipped trigger kind, exercising every optional field.
fn tutorial_toml() -> &'static str {
    r#"
[[station]]
id = "helm"
name = "Helm"
description = "Fly the ship."
rank = "Ltn."

[[station.rating]]
name = "Std"
automated_systems = []

[[station.tutorial]]
id = "helm-welcome"
title = "entity.test.station.helm.tutorial.welcome.title"
text = "entity.test.station.helm.tutorial.welcome.text"
anchor = "helm-radar"
trigger = { kind = "first_visit" }

[[station.tutorial]]
id = "helm-joystick"
title = "entity.test.station.helm.tutorial.joystick.title"
text = "entity.test.station.helm.tutorial.joystick.text"
trigger = { kind = "control_unused", control = "set_helm" }

[[station.tutorial]]
id = "helm-boost"
priority = 10
title = "entity.test.station.helm.tutorial.boost.title"
text = "entity.test.station.helm.tutorial.boost.text"
trigger = { kind = "state", path = "boost_enabled", op = "truthy", control = "set_boost" }

[[station]]
id = "captain"
name = "Captain"
description = "Command."
rank = "Cpt."

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "helm"
kind = "helm"
station = "helm"

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
"#
}

#[test]
fn station_config_parses_tutorial_overlay_blocks() {
    let config = parse_ok(tutorial_toml());
    let helm = config.station(&StationId("helm".into())).unwrap();
    assert_eq!(helm.tutorials.len(), 3);

    let welcome = &helm.tutorials[0];
    assert_eq!(welcome.id, "helm-welcome");
    assert_eq!(welcome.trigger.kind, "first_visit");
    assert_eq!(welcome.anchor.as_deref(), Some("helm-radar"));
    assert_eq!(welcome.priority, 0, "priority defaults to 0");
    assert_eq!(
        welcome.title,
        "entity.test.station.helm.tutorial.welcome.title"
    );

    let joystick = &helm.tutorials[1];
    assert_eq!(joystick.trigger.kind, "control_unused");
    assert_eq!(joystick.trigger.control.as_deref(), Some("set_helm"));
    assert_eq!(joystick.anchor, None);

    let boost = &helm.tutorials[2];
    assert_eq!(boost.priority, 10);
    assert_eq!(boost.trigger.kind, "state");
    assert_eq!(boost.trigger.path.as_deref(), Some("boost_enabled"));
    assert_eq!(boost.trigger.op.as_deref(), Some("truthy"));
    assert_eq!(boost.trigger.control.as_deref(), Some("set_boost"));

    // A station that authors none keeps an empty list (serde default).
    let captain = config.station(&StationId("captain".into())).unwrap();
    assert!(captain.tutorials.is_empty());
}

#[test]
fn station_config_tutorials_survive_round_trip() {
    let config = parse_ok(tutorial_toml());
    let encoded = toml::to_string(&config).expect("ship config should serialize");
    let decoded = parse_ok(&encoded);
    assert_eq!(decoded, config);
}

// ── Human-seeking systems (issue #984) ────────────────────────────────

#[test]
fn system_config_parses_human_seeking_flag() {
    let config = parse_ok(valid_toml());

    // Absent on systems that authored none (serde default).
    let red_alert = config.system(&SystemId("red-alert".into())).unwrap();
    assert!(!red_alert.human_seeking);
}

#[test]
fn system_config_human_seeking_survives_round_trip() {
    let toml = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "comms"
kind = "sensors"
station = "captain"
human_seeking = true
"#;
    let config = parse_ok(toml);
    assert!(
        config
            .system(&SystemId("comms".into()))
            .unwrap()
            .human_seeking
    );

    let encoded = toml::to_string(&config).expect("ship config should serialize");
    let decoded = parse_ok(&encoded);
    assert_eq!(decoded, config);
}

// ── Authored seek order (issue #984) ──────────────────────────────────

/// Two stations and one seeking system on the first of them, so a
/// `seek_order` is a two-name permutation and every rule has room to fail.
fn seek_order_toml(system_block: &str) -> String {
    format!(
        r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Fight the ship."
rank = "Lt. Cmdr."

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "engineering"
name = "Engineering"
description = "Keep it running."
rank = "Ltn."

[[station.rating]]
name = "Std"
automated_systems = []

{system_block}
"#
    )
}

#[test]
fn seek_order_is_absent_by_default_and_round_trips_when_authored() {
    // A hull that authors none keeps an empty list AND serialises without
    // the key, so an untouched hull's TOML is byte-for-byte what it was.
    let plain = parse_ok(&seek_order_toml(
        "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\n",
    ));
    let comms = plain.system(&SystemId("comms".into())).unwrap();
    assert!(comms.seek_order.is_empty());
    let encoded = toml::to_string(&plain).expect("ship config should serialize");
    assert!(
        !encoded.contains("seek_order"),
        "an unauthored seek_order must not appear on the way out:\n{encoded}"
    );

    let authored = parse_ok(&seek_order_toml(
        "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"tactical\", \"engineering\"]\n",
    ));
    assert_eq!(
        authored
            .system(&SystemId("comms".into()))
            .unwrap()
            .seek_order,
        vec![
            StationId("tactical".into()),
            StationId("engineering".into())
        ]
    );
    let encoded = toml::to_string(&authored).expect("ship config should serialize");
    assert_eq!(parse_ok(&encoded), authored);
}

#[test]
fn seek_order_rejects_a_station_this_hull_does_not_have() {
    let err = ShipConfig::from_toml(
        &seek_order_toml(
            "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"tactical\", \"engineering\", \"science\"]\n",
        ),
        KINDS,
    );
    assert!(matches!(
        err,
        Err(ShipConfigError::SeekOrderUnknownStation { ref station, .. })
            if station.0 == "science"
    ));
}

#[test]
fn seek_order_rejects_the_same_station_twice() {
    let err = ShipConfig::from_toml(
        &seek_order_toml(
            "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"tactical\", \"tactical\", \"engineering\"]\n",
        ),
        KINDS,
    );
    assert!(matches!(
        err,
        Err(ShipConfigError::SeekOrderDuplicateStation { .. })
    ));
}

/// The list is the WHOLE walk, so a station left off is a console the seek
/// could never reach — refused at load rather than discovered by a crew.
#[test]
fn seek_order_rejects_an_incomplete_walk() {
    let err = ShipConfig::from_toml(
        &seek_order_toml(
            "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"tactical\"]\n",
        ),
        KINDS,
    );
    assert!(matches!(
        err,
        Err(ShipConfigError::SeekOrderMissingStation { ref station, .. })
            if station.0 == "engineering"
    ));
}

/// Owner-first is the rule that keeps a hull's own officer at their own
/// console. A complete permutation that starts anywhere else is still wrong.
#[test]
fn seek_order_rejects_an_order_that_does_not_start_at_the_owner() {
    let err = ShipConfig::from_toml(
        &seek_order_toml(
            "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"engineering\", \"tactical\"]\n",
        ),
        KINDS,
    );
    assert!(matches!(
        err,
        Err(ShipConfigError::SeekOrderOwnerNotFirst { ref owner, ref first, .. })
            if owner.0 == "tactical" && first.as_ref().map(|s| s.0.as_str()) == Some("engineering")
    ));
}

#[test]
fn seek_order_rejects_a_system_that_does_not_seek() {
    let err = ShipConfig::from_toml(
        &seek_order_toml(
            "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nseek_order = [\"tactical\", \"engineering\"]\n",
        ),
        KINDS,
    );
    assert!(matches!(
        err,
        Err(ShipConfigError::SeekOrderWithoutHumanSeeking { .. })
    ));
}

#[test]
fn accessors_find_stations_systems_and_power_group_members() {
    let config = ShipConfig::from_toml(valid_toml(), KINDS).unwrap();

    assert_eq!(
        config
            .station(&StationId("tactical".into()))
            .map(|s| &s.name),
        Some(&"Tactical".to_string())
    );
    assert_eq!(
        config
            .system(&SystemId("phaser-fore".into()))
            .map(|s| &s.kind),
        Some(&"phaser_bank".to_string())
    );
    assert_eq!(
        config
            .systems_for_station(&StationId("tactical".into()))
            .map(|s| s.id.clone())
            .collect::<Vec<_>>(),
        vec![
            SystemId("phaser-fore".into()),
            SystemId("torpedo-magazine".into()),
            SystemId("torpedo-tube-fore-port".into())
        ]
    );
    assert_eq!(
        config
            .systems_in_power_group(&PowerGroupId("ops".into()))
            .map(|s| s.id.clone())
            .collect::<Vec<_>>(),
        vec![SystemId("red-alert".into()), SystemId("viewscreen".into())]
    );
}

#[test]
fn rejects_ownerless_without_ai_only() {
    // Build a config where a system has no station and no ai_only flag.
    // This is done by appending a new orphan system after the valid TOML.
    let toml = format!(
        "{}\n[[system]]\nid = \"orphan\"\nkind = \"viewscreen\"\npower_group = \"ops\"\n",
        valid_toml()
    );

    assert_eq!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::OwnerlessSystemWithoutAiOnly {
            system: SystemId("orphan".into())
        })
    );
}

#[test]
fn rejects_core_as_station_id() {
    let toml = valid_toml().replace("id = \"captain\"", "id = \"core\"");

    assert_eq!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::ReservedCoreStationId {
            station: StationId("core".into())
        })
    );
}

#[test]
fn rejects_missing_required_station_description() {
    let toml = valid_toml().replace("description = \"Command the bridge.\"\n", "");

    assert!(matches!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::ParseError(_))
    ));
}

#[test]
fn rejects_missing_required_station_rank() {
    let toml = valid_toml().replace("rank = \"Cpt.\"\n", "");

    assert!(matches!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::ParseError(_))
    ));
}

#[test]
fn rejects_missing_required_rating_automated_systems() {
    let toml = valid_toml().replace("automated_systems = []\n", "");

    assert!(matches!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::ParseError(_))
    ));
}

#[test]
fn rejects_empty_system_id() {
    let toml = valid_toml().replace("id = \"viewscreen\"", "id = \"\"");

    assert_eq!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::EmptySystemId)
    );
}

#[test]
fn rejects_duplicate_system_id() {
    let toml = valid_toml().replace("id = \"viewscreen\"", "id = \"red-alert\"");

    assert_eq!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::DuplicateSystemId {
            id: SystemId("red-alert".into())
        })
    );
}

#[test]
fn rejects_dangling_rating_reference() {
    let toml = valid_toml().replace(
        "automated_systems = [\"torpedo-magazine\", \"torpedo-tube-fore-port\"]",
        "automated_systems = [\"torpedo-magazine\", \"missing-system\"]",
    );

    assert_eq!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::DanglingRatingReference {
            station: StationId("tactical".into()),
            rating: "Assisted".into(),
            system: SystemId("missing-system".into())
        })
    );
}

#[test]
fn rejects_rating_reference_to_unowned_system() {
    let toml = valid_toml().replace(
        "automated_systems = [\"torpedo-magazine\", \"torpedo-tube-fore-port\"]",
        "automated_systems = [\"torpedo-magazine\", \"red-alert\"]",
    );

    assert_eq!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::RatingReferencesUnownedSystem {
            station: StationId("tactical".into()),
            rating: "Assisted".into(),
            system: SystemId("red-alert".into()),
            owner: Some(StationId("captain".into()))
        })
    );
}

#[test]
fn rejects_unknown_system_kind() {
    let toml = valid_toml().replace("kind = \"viewscreen\"", "kind = \"magic\"");

    assert_eq!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::UnknownSystemKind {
            system: SystemId("viewscreen".into()),
            kind: "magic".into()
        })
    );
}

#[test]
fn rejects_unknown_power_group() {
    let toml = valid_toml().replace("power_group = \"weapons\"", "power_group = \"missing\"");

    assert_eq!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::UnknownPowerGroup {
            system: SystemId("phaser-fore".into()),
            power_group: PowerGroupId("missing".into())
        })
    );
}

#[test]
fn rejects_unknown_station_owner() {
    let toml = valid_toml().replace("station = \"captain\"", "station = \"ghost\"");

    assert_eq!(
        parse_and_validate(&toml, KINDS),
        Err(ShipConfigError::UnknownStation {
            system: SystemId("red-alert".into()),
            station: StationId("ghost".into())
        })
    );
}

#[test]
fn station_config_parses_console_field() {
    let config = parse_ok(valid_toml());

    let captain = config.station(&StationId("captain".into())).unwrap();
    assert_eq!(captain.console.as_deref(), Some("gui/captain-console.html"));

    let tactical = config.station(&StationId("tactical".into())).unwrap();
    assert_eq!(
        tactical.console.as_deref(),
        Some("gui/tactical-console.html")
    );
}

#[test]
fn station_config_console_defaults_to_none_when_absent() {
    let toml = valid_toml().replace("console = \"gui/captain-console.html\"\n", "");
    let config = parse_ok(&toml);

    let captain = config.station(&StationId("captain".into())).unwrap();
    assert_eq!(captain.console, None);
}

#[test]
fn station_system_and_console_resolution_for_battleship_style_config() {
    let toml = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command."
rank = "Cpt."
short_code = "CPT"
console = "gui/captain-console.html"

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "helm"
name = "Helm"
description = "Pilot."
rank = "Ltn."
short_code = "HLM"
console = "gui/helm-console.html"

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."
short_code = "TAC"
console = "gui/tactical-console.html"

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "repair"
name = "Repair"
description = "Repair."
rank = "Ltn."
short_code = "ENG"
console = "gui/repair-console.html"

[[station.rating]]
name = "Std"
automated_systems = []

[power_groups.ops]
label = "Operations"
default_level = 2
min_level = 1
max_level = 4

[power_groups.helm]
label = "Propulsion"
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
id = "helm"
kind = "helm"
station = "helm"
power_group = "helm"

[[system]]
id = "tactical"
kind = "phaser_bank"
station = "tactical"
power_group = "weapons"

[[system]]
id = "viewscreen"
kind = "viewscreen"
station = "captain"
power_group = "ops"
"#;
    let config = parse_ok(toml);

    assert_eq!(config.stations.len(), 4);
    assert_eq!(config.systems.len(), 4);

    for station in &config.stations {
        match station.id.0.as_str() {
            "captain" => {
                assert_eq!(station.console.as_deref(), Some("gui/captain-console.html"));
                let systems: Vec<_> = config.systems_for_station(&station.id).collect();
                assert_eq!(systems.len(), 2);
            }
            "helm" => {
                assert_eq!(station.console.as_deref(), Some("gui/helm-console.html"));
                let systems: Vec<_> = config.systems_for_station(&station.id).collect();
                assert_eq!(systems.len(), 1);
            }
            "tactical" => {
                assert_eq!(
                    station.console.as_deref(),
                    Some("gui/tactical-console.html")
                );
                let systems: Vec<_> = config.systems_for_station(&station.id).collect();
                assert_eq!(systems.len(), 1);
            }
            "repair" => {
                assert_eq!(station.console.as_deref(), Some("gui/repair-console.html"));
                let systems: Vec<_> = config.systems_for_station(&station.id).collect();
                assert_eq!(systems.len(), 0);
            }
            other => panic!("unexpected station id: {other}"),
        }
    }

    let captain_station = config.station(&StationId("captain".into())).unwrap();
    let captain_system_ids: Vec<&str> = config
        .systems_for_station(&captain_station.id)
        .map(|s| s.id.0.as_str())
        .collect();
    assert_eq!(captain_system_ids, vec!["red-alert", "viewscreen"]);
}

// ── weapons_station ──────────────────────────────────────────────────

/// The crewed hulls put their fine weapon systems on a station named
/// "tactical"; resolving from config must not change that.
#[test]
fn weapons_station_resolves_tactical_for_crewed_hull_shape() {
    let toml = r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
"#;
    let config = ShipConfig::from_toml(toml, KINDS).unwrap();
    assert_eq!(config.weapons_station(), Some(StationId("tactical".into())));
}

/// The Courier puts its blaster on the single "pilot" station. This is the
/// case the whole lookup exists for.
#[test]
fn weapons_station_resolves_pilot_when_weapons_live_on_pilot() {
    let toml = r#"
[[station]]
id = "pilot"
name = "Pilot"
description = "Everything."
rank = "Ltn."

[[system]]
id = "blaster-fore"
kind = "blaster_bank"
station = "pilot"
"#;
    let config = ShipConfig::from_toml(toml, &["blaster_bank"]).unwrap();
    assert_eq!(config.weapons_station(), Some(StationId("pilot".into())));
}

/// NPCs declare no `station` on any system — no human owns their guns.
#[test]
fn weapons_station_is_none_for_npc_shape() {
    let toml = r#"
[[system]]
id = "phaser-fore"
kind = "phaser_bank"
ai_only = true
"#;
    let config = ShipConfig::from_toml(toml, KINDS).unwrap();
    assert_eq!(config.weapons_station(), None);
}

/// Legacy/test ships declare a `tactical` station but no fine weapon
/// systems. They must keep resolving to it, or the pre-lookup gates change
/// behaviour.
#[test]
fn weapons_station_falls_back_to_tactical_station_without_fine_systems() {
    let toml = r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."

[[system]]
id = "helm"
kind = "helm"
station = "tactical"
"#;
    let config = ShipConfig::from_toml(toml, KINDS).unwrap();
    assert_eq!(config.weapons_station(), Some(StationId("tactical".into())));
}

// ── sensors_station ──────────────────────────────────────────────────

#[test]
fn sensors_station_resolves_declared_station() {
    let toml = r#"
[[station]]
id = "sensors"
name = "Sensors"
description = "Long-range sensors."
rank = "Ens."

[[system]]
id = "sensors"
kind = "sensors"
station = "sensors"
"#;
    let config = ShipConfig::from_toml(toml, KINDS).unwrap();
    assert_eq!(config.sensors_station(), Some(StationId("sensors".into())));
}

/// NPCs declare no `station` on their sensors system — no human owns it.
#[test]
fn sensors_station_is_none_for_npc_shape() {
    let toml = r#"
[[system]]
id = "sensors"
kind = "sensors"
ai_only = true
"#;
    let config = ShipConfig::from_toml(toml, KINDS).unwrap();
    assert_eq!(config.sensors_station(), None);
}

#[test]
fn sensors_station_is_none_when_no_sensors_system_declared() {
    let toml = r#"
[[system]]
id = "helm"
kind = "helm"
ai_only = true
"#;
    let config = ShipConfig::from_toml(toml, KINDS).unwrap();
    assert_eq!(config.sensors_station(), None);
}

// ── Command stances (issue #1107) ─────────────────────────────────────────

const STANCE_KINDS: &[&str] = &["red_alert", "sensors", "command"];

/// A captain, a proving station (sensors) authoring a full stance catalogue,
/// and an auxiliary Command station directing it, hosted by the captain.
fn command_toml(catalogue: &str, command_extra: &str) -> String {
    format!(
        r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "proving"
name = "Proving"
description = "The AI-controlled proving station."
rank = "Ltn."
{catalogue}

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "command"
name = "Command"
description = "Direct an AI station."
rank = "Cpt."
console = "gui/command-console.html"
auxiliary = true
human_seeking = true
host_order = ["captain"]
visiting_rating = "Std"
command_target = "proving"
{command_extra}

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"

[[system]]
id = "sensors"
kind = "sensors"
station = "proving"

[[system]]
id = "command"
kind = "command"
station = "command"
"#
    )
}

const FULL_CATALOGUE: &str = r#"
[[station.stance]]
id = "proving-standard"
kind = "standard"
high_alert = true
persist_behind_human = true

[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;

fn parse_command(catalogue: &str, extra: &str) -> Result<ShipConfig, ShipConfigError> {
    ShipConfig::from_toml(&command_toml(catalogue, extra), STANCE_KINDS)
}

#[test]
fn command_station_and_stance_catalogue_parse_and_round_trip() {
    let config = parse_command(FULL_CATALOGUE, "").expect("command hull parses");
    let command = config.station(&StationId("command".into())).unwrap();
    assert!(command.auxiliary);
    assert!(command.human_seeking);
    assert_eq!(command.host_order, vec![StationId("captain".into())]);
    assert_eq!(command.command_target, Some(StationId("proving".into())));

    let proving = config.station(&StationId("proving".into())).unwrap();
    assert_eq!(proving.stances.len(), 3);
    assert_eq!(proving.stances[0].id, "proving-standard");
    assert_eq!(proving.stances[0].kind, StanceKind::Standard);
    assert!(proving.stances[0].high_alert);
    // The authored persistence flag (issue #1108 AC1) parses and round-trips;
    // an unauthored stance defaults to non-persistent.
    assert!(proving.stances[0].persist_behind_human);
    assert!(!proving.stances[1].persist_behind_human);
    assert_eq!(proving.stances[1].kind, StanceKind::NormalAlertNeutral);
    assert!(!proving.stances[1].high_alert);
    assert_eq!(proving.stances[2].kind, StanceKind::HighAlertNeutral);

    // A hull that authors no catalogue keeps an empty list and serialises
    // without the key, so untouched hulls round-trip byte-for-byte.
    let encoded = toml::to_string(&config).expect("serialise");
    let decoded = ShipConfig::from_toml(&encoded, STANCE_KINDS).unwrap();
    assert_eq!(decoded, config);
    let captain_encoded =
        toml::to_string(config.station(&StationId("captain".into())).unwrap()).unwrap();
    assert!(!captain_encoded.contains("stance"));
    assert!(!captain_encoded.contains("command_target"));
}

#[test]
fn command_target_must_name_a_real_station() {
    let toml = command_toml(FULL_CATALOGUE, "")
        .replace("command_target = \"proving\"", "command_target = \"ghost\"");
    assert!(matches!(
        ShipConfig::from_toml(&toml, STANCE_KINDS),
        Err(ShipConfigError::CommandTargetUnknownStation { ref target, .. })
            if target.0 == "ghost"
    ));
}

#[test]
fn command_target_must_author_a_catalogue() {
    // Point Command at the captain, which has no stances.
    let toml = command_toml(FULL_CATALOGUE, "").replace(
        "command_target = \"proving\"",
        "command_target = \"captain\"",
    );
    assert!(matches!(
        ShipConfig::from_toml(&toml, STANCE_KINDS),
        Err(ShipConfigError::CommandTargetHasNoStances { ref target, .. })
            if target.0 == "captain"
    ));
}

#[test]
fn catalogue_must_have_exactly_one_of_each_neutral() {
    // Drop the high-alert neutral.
    let missing = r#"
[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"
"#;
    assert!(matches!(
        parse_command(missing, ""),
        Err(ShipConfigError::StanceCatalogueNeutralCount {
            kind: StanceKind::HighAlertNeutral,
            found: 0,
            ..
        })
    ));
}

#[test]
fn neutral_stance_posture_must_agree_with_kind() {
    // normal_alert_neutral authored as high_alert = true.
    let bad = r#"
[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"
high_alert = true

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;
    assert!(matches!(
        parse_command(bad, ""),
        Err(ShipConfigError::NeutralStancePostureMismatch {
            kind: StanceKind::NormalAlertNeutral,
            ..
        })
    ));
}

#[test]
fn catalogue_rejects_duplicate_stance_ids() {
    let dupe = r#"
[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "proving-normal"
kind = "high_alert_neutral"
high_alert = true
"#;
    assert!(matches!(
        parse_command(dupe, ""),
        Err(ShipConfigError::DuplicateStanceId { ref stance, .. })
            if stance == "proving-normal"
    ));
}

#[test]
fn the_ai_engaged_flag_parses_and_round_trips() {
    // Issue #1109: a single standard stance may carry the AI Command
    // high-alert pick, and it survives a serialise/parse round-trip.
    let config = parse_command(FULL_CATALOGUE_AI_ENGAGED, "").expect("hull parses");
    let proving = config.station(&StationId("proving".into())).unwrap();
    assert!(proving.stances[0].ai_engaged);
    assert!(!proving.stances[1].ai_engaged);
    let encoded = toml::to_string(&config).expect("serialise");
    let decoded = ShipConfig::from_toml(&encoded, STANCE_KINDS).unwrap();
    assert_eq!(decoded, config);
}

#[test]
fn catalogue_rejects_more_than_one_ai_engaged_stance() {
    // Issue #1109: the AI's high-alert choice is a single authored posture.
    let two = r#"
[[station.stance]]
id = "proving-a"
kind = "standard"
high_alert = true
ai_engaged = true

[[station.stance]]
id = "proving-b"
kind = "standard"
high_alert = true
ai_engaged = true

[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;
    assert!(matches!(
        parse_command(two, ""),
        Err(ShipConfigError::MultipleAiEngagedStances { .. })
    ));
}

#[test]
fn catalogue_rejects_an_ai_engaged_neutral() {
    // Issue #1109: a neutral is already the tracking default, so it may not
    // be flagged as the engaged posture.
    let bad = r#"
[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"
ai_engaged = true

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;
    assert!(matches!(
        parse_command(bad, ""),
        Err(ShipConfigError::AiEngagedStanceNotStandard { ref stance, .. })
            if stance == "proving-normal"
    ));
}

const FULL_CATALOGUE_AI_ENGAGED: &str = r#"
[[station.stance]]
id = "proving-standard"
kind = "standard"
high_alert = true
ai_engaged = true

[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;
