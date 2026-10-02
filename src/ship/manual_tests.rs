use super::*;
use crate::ship::config::parse_and_validate;
use crate::ship::rating::BACKFILL_RATING;

const KINDS: &[&str] = &[
    "captain",
    "red_alert",
    "shields",
    "shield_arc",
    "sensors",
    "helm_thrust",
];

/// A cruiser-shaped config: a `science` station owning a `shields` system
/// plus four synthesised `shield_arc` systems (as `entities::config` would
/// synthesise them from `[[shield_arc]]` blocks), and a bare `captain`
/// station with no provider-backed system.
fn ship_toml() -> &'static str {
    r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command."
rank = "Cpt."
manual_overview = "Captain overview prose."

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "science"
name = "Science"
description = "Sensors and shields."
rank = "Ltn."
manual_overview = "Science overview prose."

[[station.rating]]
name = "Std"
automated_systems = ["sensors"]

[[station.rating]]
name = "Simplified"
automated_systems = ["sensors", "shield-arc-fore", "shield-arc-aft"]

[[system]]
id = "captain"
kind = "captain"
station = "captain"

[[system]]
id = "sensors"
kind = "sensors"
station = "science"

[[system]]
id = "shields-system"
kind = "shields"
station = "science"

[[system]]
id = "shield-arc-fore"
kind = "shield_arc"
station = "science"

[[system]]
id = "shield-arc-port"
kind = "shield_arc"
station = "science"

[[system]]
id = "shield-arc-aft"
kind = "shield_arc"
station = "science"

[[system]]
id = "shield-arc-starboard"
kind = "shield_arc"
station = "science"
"#
}

fn parse() -> ShipConfig {
    parse_and_validate(ship_toml(), KINDS).expect("ship config should parse")
}

fn shields_base_extras() -> HashMap<String, toml::Value> {
    let mut base = toml::value::Table::new();
    base.insert("max_hp".into(), toml::Value::Integer(100));
    base.insert("regen_per_sec".into(), toml::Value::Integer(2));
    HashMap::from([(
        crate::ship::system_registry::SHIELDS_KIND.to_string(),
        toml::Value::Table(base),
    )])
}

fn shields_section(manual: &ShipManualWire) -> &SystemManualSection {
    manual
        .stations
        .iter()
        .find(|s| s.station_id == StationId("science".into()))
        .expect("science station present")
        .sections
        .iter()
        .find(|sec| sec.kind == crate::ship::system_registry::SHIELDS_KIND)
        .expect("shields section present")
}

fn metric<'a>(section: &'a SystemManualSection, code: &str) -> &'a SystemManualMetric {
    section
        .metrics
        .iter()
        .find(|m| m.code == code)
        .unwrap_or_else(|| panic!("metric {code} present"))
}

#[test]
fn every_authored_station_is_represented() {
    let manual = build_ship_manual(
        &parse(),
        &ManualProviderRegistry::with_shipped_providers(),
        &shields_base_extras(),
    );
    let ids: Vec<&str> = manual
        .stations
        .iter()
        .map(|s| s.station_id.0.as_str())
        .collect();
    assert_eq!(ids, vec!["captain", "science"]);
}

#[test]
fn station_without_a_provider_appears_overview_only() {
    let manual = build_ship_manual(
        &parse(),
        &ManualProviderRegistry::with_shipped_providers(),
        &shields_base_extras(),
    );
    let captain = manual
        .stations
        .iter()
        .find(|s| s.station_id == StationId("captain".into()))
        .unwrap();
    assert_eq!(captain.overview.as_deref(), Some("Captain overview prose."));
    assert!(
        captain.sections.is_empty(),
        "captain owns no provider-backed system, so it must be overview-only"
    );
}

#[test]
fn station_combines_overview_with_generated_section() {
    let manual = build_ship_manual(
        &parse(),
        &ManualProviderRegistry::with_shipped_providers(),
        &shields_base_extras(),
    );
    let science = manual
        .stations
        .iter()
        .find(|s| s.station_id == StationId("science".into()))
        .unwrap();
    assert_eq!(science.overview.as_deref(), Some("Science overview prose."));
    assert!(
        science
            .sections
            .iter()
            .any(|sec| sec.kind == crate::ship::system_registry::SHIELDS_KIND),
        "science must carry the generated shields section alongside its overview"
    );
}

#[test]
fn shields_section_reflects_configured_values() {
    let manual = build_ship_manual(
        &parse(),
        &ManualProviderRegistry::with_shipped_providers(),
        &shields_base_extras(),
    );
    let section = shields_section(&manual);
    assert_eq!(metric(section, "max_hp").value, 100.0);
    assert_eq!(metric(section, "regen").value, 2.0);
    // Four `[[shield_arc]]` blocks → four synthesised shield_arc systems.
    assert_eq!(metric(section, "arcs").value, 4.0);
}

#[test]
fn shields_section_rating_mapping_matches_resolver() {
    let config = parse();
    let manual = build_ship_manual(
        &config,
        &ManualProviderRegistry::with_shipped_providers(),
        &shields_base_extras(),
    );
    let section = shields_section(&manual);
    let station = StationId("science".into());

    // Every authored rating plus the implicit Backfill is present.
    let ratings: Vec<&str> = section
        .automation
        .iter()
        .map(|a| a.rating.as_str())
        .collect();
    assert_eq!(ratings, vec!["Std", "Simplified", BACKFILL_RATING]);

    // Each row matches resolve_automated_systems exactly.
    for row in &section.automation {
        let expected =
            resolve_automated_systems(&config, &station, &row.rating).expect("rating resolves");
        assert_eq!(
            row.automated_systems, expected,
            "rating {} automation must mirror resolve_automated_systems",
            row.rating
        );
    }

    // Backfill automates every system the station owns.
    let backfill = section
        .automation
        .iter()
        .find(|a| a.rating == BACKFILL_RATING)
        .unwrap();
    assert_eq!(
        backfill.automated_systems,
        vec![
            SystemId("sensors".into()),
            SystemId("shields-system".into()),
            SystemId("shield-arc-fore".into()),
            SystemId("shield-arc-port".into()),
            SystemId("shield-arc-aft".into()),
            SystemId("shield-arc-starboard".into()),
        ]
    );
}

#[test]
fn unregistered_kind_yields_no_section() {
    // An empty registry means even the shields system gets no section —
    // providers are never fabricated.
    let manual = build_ship_manual(
        &parse(),
        &ManualProviderRegistry::new(),
        &shields_base_extras(),
    );
    for station in &manual.stations {
        assert!(
            station.sections.is_empty(),
            "no providers registered, so no sections anywhere"
        );
    }
}

#[test]
fn manual_wire_round_trips_through_json() {
    let manual = build_ship_manual(
        &parse(),
        &ManualProviderRegistry::with_shipped_providers(),
        &shields_base_extras(),
    );
    // serde_json lives only in codec.rs; use toml here for a pure-module
    // round-trip smoke of the wire structs.
    let encoded = toml::to_string(&manual).expect("serialize");
    let decoded: ShipManualWire = toml::from_str(&encoded).expect("deserialize");
    assert_eq!(decoded, manual);
}

// ── #773: per-kind providers, station aggregation, helm capability ─────────

use crate::ship::system_registry as kinds;
use std::collections::HashSet;

const FULL_KINDS: &[&str] = &[
    "captain",
    "red_alert",
    "viewscreen",
    "phaser_bank",
    "blaster_bank",
    "torpedo_magazine",
    "torpedo_tube",
    "tactical_radar",
    "sensors",
    "sensor_radar",
    "power_reactor",
    "power_battery",
    "repair",
    "comms",
    "helm_thrust",
];

/// A multi-station hull exercising every #773 provider: tactical (phaser +
/// torpedo magazine + two tubes + radar), engineering (reactor + battery +
/// repair), science (sensors + sensor radar), comms (comms), helm (throttle).
fn full_ship_toml() -> &'static str {
    r#"
[power_groups.ops]
label = "Ops"
[power_groups.weapons]
label = "Weapons"

[[station]]
id = "helm"
name = "Helm"
description = "Fly."
rank = "Ltn."
[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Fight."
rank = "Ltn."
[[station.rating]]
name = "Std"
automated_systems = []
[[station.rating]]
name = "Simplified"
automated_systems = ["phaser-fore"]

[[station]]
id = "engineering"
name = "Engineering"
description = "Power."
rank = "Ltn."
[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "science"
name = "Science"
description = "Sense."
rank = "Ltn."
[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "comms"
name = "Comms"
description = "Talk."
rank = "Ens."
[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
station = "helm"
[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
[[system]]
id = "torpedo-magazine"
kind = "torpedo_magazine"
station = "tactical"
[[system]]
id = "torpedo-tube-fore"
kind = "torpedo_tube"
station = "tactical"
[[system]]
id = "torpedo-tube-aft"
kind = "torpedo_tube"
station = "tactical"
[[system]]
id = "tactical-radar"
kind = "tactical_radar"
station = "tactical"
[[system]]
id = "power-reactor"
kind = "power_reactor"
station = "engineering"
[[system]]
id = "power-battery"
kind = "power_battery"
station = "engineering"
[[system]]
id = "repair"
kind = "repair"
station = "engineering"
[[system]]
id = "sensors"
kind = "sensors"
station = "science"
[[system]]
id = "sensor-radar"
kind = "sensor_radar"
station = "science"
[[system]]
id = "comms"
kind = "comms"
station = "comms"
"#
}

fn full_config() -> ShipConfig {
    parse_and_validate(full_ship_toml(), FULL_KINDS).expect("full ship config parses")
}

fn table(pairs: &[(&str, toml::Value)]) -> toml::Value {
    let mut t = toml::value::Table::new();
    for (k, v) in pairs {
        t.insert((*k).to_string(), v.clone());
    }
    toml::Value::Table(t)
}

fn sys<'a>(config: &'a ShipConfig, id: &str) -> &'a SystemInstanceConfig {
    config.system(&SystemId(id.into())).expect("system present")
}

#[test]
fn phaser_provider_reflects_the_addressed_bank() {
    let config = full_config();
    let extra = table(&[(
        "banks",
        toml::Value::Array(vec![table(&[
            ("system_id", toml::Value::String("phaser-fore".into())),
            ("beam_range", toml::Value::Float(40.0)),
            ("beam_damage_per_sec", toml::Value::Float(4.0)),
            ("cooldown_secs", toml::Value::Float(6.0)),
            ("fire_arc_deg", toml::Value::Float(270.0)),
        ])]),
    )]);
    let section = PhaserBankManualProvider.build(
        &config,
        sys(&config, "phaser-fore"),
        &StationId("tactical".into()),
        Some(&extra),
    );
    assert_eq!(section.kind, kinds::PHASER_BANK_KIND);
    assert_eq!(metric(&section, "beam_range").value, 40.0);
    assert_eq!(metric(&section, "beam_damage").value, 4.0);
    assert_eq!(metric(&section, "cooldown").value, 6.0);
    assert_eq!(metric(&section, "fire_arc").value, 270.0);
}

#[test]
fn torpedo_magazine_provider_derives_tube_count_from_topology() {
    let config = full_config();
    let extra = table(&[
        ("count", toml::Value::Integer(6)),
        ("damage_hull", toml::Value::Integer(40)),
        ("damage_shields", toml::Value::Integer(4)),
        ("load_time", toml::Value::Float(10.0)),
    ]);
    let section = TorpedoMagazineManualProvider.build(
        &config,
        sys(&config, "torpedo-magazine"),
        &StationId("tactical".into()),
        Some(&extra),
    );
    assert_eq!(metric(&section, "capacity").value, 6.0);
    assert_eq!(metric(&section, "damage_hull").value, 40.0);
    // Two `torpedo_tube` systems declared on this hull.
    assert_eq!(metric(&section, "tubes").value, 2.0);
}

#[test]
fn power_reactor_provider_counts_power_groups() {
    let config = full_config();
    let extra = table(&[("capacity", toml::Value::Float(90.0))]);
    let section = PowerReactorManualProvider.build(
        &config,
        sys(&config, "power-reactor"),
        &StationId("engineering".into()),
        Some(&extra),
    );
    assert_eq!(metric(&section, "capacity").value, 90.0);
    // Two `[power_groups.*]` declared → power_groups metric == 2.
    assert_eq!(metric(&section, "power_groups").value, 2.0);
}

#[test]
fn repair_sensors_comms_providers_reflect_configured_values() {
    let config = full_config();

    let repair = RepairManualProvider.build(
        &config,
        sys(&config, "repair"),
        &StationId("engineering".into()),
        Some(&table(&[
            ("repair_team_count", toml::Value::Integer(2)),
            ("repair_rate_hp_per_sec", toml::Value::Float(0.5)),
            ("travel_duration_secs", toml::Value::Float(5.0)),
        ])),
    );
    assert_eq!(metric(&repair, "teams").value, 2.0);
    assert_eq!(metric(&repair, "rate").value, 0.5);
    assert_eq!(metric(&repair, "travel").value, 5.0);

    let sensors = SensorsManualProvider.build(
        &config,
        sys(&config, "sensors"),
        &StationId("science".into()),
        Some(&table(&[("range", toml::Value::Float(300.0))])),
    );
    assert_eq!(metric(&sensors, "range").value, 300.0);

    let comms = CommsManualProvider.build(
        &config,
        sys(&config, "comms"),
        &StationId("comms".into()),
        Some(&table(&[("range", toml::Value::Float(1200.0))])),
    );
    assert_eq!(metric(&comms, "range").value, 1200.0);
}

#[test]
fn tactical_station_collects_multiple_sections_and_mirrors_ratings() {
    let config = full_config();
    // Minimal extras — values don't matter here, structure does.
    let extras: HashMap<String, toml::Value> = HashMap::from([
        (
            kinds::PHASER_BANK_KIND.to_string(),
            table(&[(
                "banks",
                toml::Value::Array(vec![table(&[(
                    "system_id",
                    toml::Value::String("phaser-fore".into()),
                )])]),
            )]),
        ),
        (
            kinds::TORPEDO_MAGAZINE_KIND.to_string(),
            table(&[("count", toml::Value::Integer(6))]),
        ),
        (
            kinds::TORPEDO_TUBE_KIND.to_string(),
            table(&[("tubes", toml::Value::Array(vec![]))]),
        ),
        (
            kinds::TACTICAL_RADAR_KIND.to_string(),
            table(&[("range", toml::Value::Float(75.0))]),
        ),
    ]);
    let manual = build_ship_manual(
        &config,
        &ManualProviderRegistry::with_shipped_providers(),
        &extras,
    );
    let tactical = manual
        .stations
        .iter()
        .find(|s| s.station_id == StationId("tactical".into()))
        .expect("tactical station present");
    let section_kinds: HashSet<&str> = tactical.sections.iter().map(|s| s.kind.as_str()).collect();
    // Phaser bank, torpedo magazine, two torpedo tubes, and the radar all
    // contribute their own section to the one station.
    assert!(section_kinds.contains(kinds::PHASER_BANK_KIND));
    assert!(section_kinds.contains(kinds::TORPEDO_MAGAZINE_KIND));
    assert!(section_kinds.contains(kinds::TORPEDO_TUBE_KIND));
    assert!(section_kinds.contains(kinds::TACTICAL_RADAR_KIND));
    assert_eq!(
        tactical.sections.len(),
        5,
        "phaser + magazine + 2 tubes + radar = 5 sections"
    );

    // Every generated section mirrors resolve_automated_systems for the
    // owning station (rating mappings identical across sections).
    let station = StationId("tactical".into());
    for section in &tactical.sections {
        for row in &section.automation {
            let expected =
                resolve_automated_systems(&config, &station, &row.rating).expect("rating resolves");
            assert_eq!(row.automated_systems, expected);
        }
    }
}

fn helm_section_for(movement: Option<&str>) -> SystemManualSection {
    let config = full_config();
    let mut pairs = vec![
        ("max_speed", toml::Value::Float(10.0)),
        ("max_reverse_speed", toml::Value::Float(4.0)),
        ("max_yaw_rate", toml::Value::Float(0.4)),
        ("impulse_steering_multiplier", toml::Value::Float(0.1)),
    ];
    if let Some(m) = movement {
        pairs.push(("movement_mode", toml::Value::String(m.into())));
    }
    HelmManualProvider.build(
        &config,
        sys(&config, "helm-thrust"),
        &StationId("helm".into()),
        Some(&table(&pairs)),
    )
}

fn capability<'a>(section: &'a SystemManualSection, code: &str) -> &'a SystemManualCapability {
    section
        .capabilities
        .iter()
        .find(|c| c.code == code)
        .unwrap_or_else(|| panic!("capability {code} present"))
}

#[test]
fn helm_provider_reflects_speeds_and_impulse_steering() {
    let section = helm_section_for(Some("bounded"));
    assert_eq!(section.kind, kinds::HELM_THRUST_KIND);
    assert_eq!(metric(&section, "max_speed").value, 10.0);
    assert_eq!(metric(&section, "max_reverse_speed").value, 4.0);
    assert_eq!(metric(&section, "max_yaw_rate").value, 0.4);
    assert_eq!(metric(&section, "impulse_steering").value, 0.1);
}

#[test]
fn helm_provider_carries_movement_mode_as_a_capability() {
    // AC3: Bounded and Full3D are reflected, and an absent movement mode
    // resolves to the effective Planar default.
    assert_eq!(
        capability(&helm_section_for(Some("bounded")), "movement_mode").value_code,
        "bounded"
    );
    assert_eq!(
        capability(&helm_section_for(Some("full_3d")), "movement_mode").value_code,
        "full_3d"
    );
    assert_eq!(
        capability(&helm_section_for(None), "movement_mode").value_code,
        "planar"
    );
}
