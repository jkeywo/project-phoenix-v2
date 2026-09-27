use project_phoenix::core::messages::StationId;
use project_phoenix::entities::include_resolve::load_entity_config;
use project_phoenix::world::config::parse_world;
use std::collections::{BTreeMap, BTreeSet};

const PLAYER_HULL: &str = "assets/entities/dynasty_player_cruiser.toml";
const NPC_HULL: &str = "assets/entities/ship_harrow_cruiser.toml";

#[test]
fn strike_reserve_rejects_missing_invalid_and_unknown_weapon_costs() {
    use project_phoenix::entities::{config::EntityConfig, include_resolve::resolve_from_disk};
    let resolved = resolve_from_disk(PLAYER_HULL).unwrap().value;
    for invalid in ["missing", "zero", "negative_multiplier", "unknown"] {
        let mut value = resolved.clone();
        let weapons = value["power"]["strike_reserve"]["weapons"]
            .as_table_mut()
            .unwrap();
        match invalid {
            "missing" => {
                weapons.remove("phaser-fore");
            }
            "zero" => weapons.get_mut("phaser-fore").unwrap()["cost"] = toml::Value::Float(0.0),
            "negative_multiplier" => {
                weapons.get_mut("phaser-fore").unwrap()["damage_multiplier"] =
                    toml::Value::Float(-1.0)
            }
            "unknown" => {
                weapons.insert("nonexistent".into(), weapons["phaser-fore"].clone());
            }
            _ => unreachable!(),
        }
        assert!(
            EntityConfig::from_toml(&toml::to_string(&value).unwrap()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn dynasty_player_cruiser_has_the_agreed_six_role_authority_map() {
    let hull = load_entity_config(PLAYER_HULL).expect("the composed Dynasty player hull parses");
    let ship = hull
        .ship_config
        .expect("the player hull carries a station/system graph");

    let roster = ship
        .stations
        .iter()
        .map(|station| station.id.0.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        roster,
        BTreeSet::from([
            "command",
            "damage-control",
            "gunnery",
            "helm",
            "power",
            "sensors"
        ]),
    );

    let owned = ship
        .stations
        .iter()
        .map(|station| {
            let systems = ship
                .systems_for_station(&station.id)
                .map(|system| system.kind.as_str())
                .collect::<BTreeSet<_>>();
            (station.id.0.as_str(), systems)
        })
        .collect::<BTreeMap<_, _>>();

    assert!(owned["command"].contains("captain") && owned["command"].contains("comms"));
    assert!(owned["helm"].contains("helm_thrust") && owned["helm"].contains("navigation"));
    assert!(
        owned["gunnery"].contains("tactical_radar") && owned["gunnery"].contains("phaser_bank")
    );
    assert_eq!(
        owned["sensors"],
        BTreeSet::from(["sensor_radar", "sensors"])
    );
    assert_eq!(
        owned["power"],
        BTreeSet::from(["power_battery", "power_reactor"])
    );
    assert!(
        owned["damage-control"].contains("shields") && owned["damage-control"].contains("repair")
    );

    for station in &ship.stations {
        assert!(
            !ship
                .systems_for_station(&station.id)
                .collect::<Vec<_>>()
                .is_empty(),
            "{} must remain a working Backfill-capable station",
            station.id.0
        );
        assert!(station
            .console
            .as_deref()
            .is_some_and(|path| path.starts_with("gui/dynasty-cruiser/")));
        assert!(
            !station.tutorials.is_empty(),
            "{} needs role onboarding",
            station.id.0
        );
    }

    let automated = |station: &str| {
        ship.station(&StationId(station.into()))
            .unwrap()
            .ratings
            .iter()
            .find(|rating| rating.name == "Std")
            .unwrap()
            .automated_systems
            .iter()
            .map(|system| system.0.as_str())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(automated("command"), BTreeSet::from(["security"]));
    assert_eq!(automated("helm"), BTreeSet::from(["dock"]));
    assert_eq!(
        automated("damage-control"),
        BTreeSet::from(["tractor", "umbilical"]),
        "inherited coupling gear stays on ordinary AI instead of becoming hidden human controls"
    );
}

#[test]
fn the_new_hull_is_selectable_without_rewriting_the_npc_cruiser() {
    let source = std::fs::read_to_string("assets/worlds/combat_test.toml").unwrap();
    let world = parse_world(&source).expect("combat_test remains valid TOML");
    assert!(world
        .available_ships
        .iter()
        .any(|ship| ship.template_path == PLAYER_HULL));

    let npc = load_entity_config(NPC_HULL).expect("the existing NPC cruiser still parses");
    assert_eq!(
        npc.ship_config
            .expect("NPC cruiser station graph")
            .stations
            .iter()
            .map(|station| station.id.clone())
            .collect::<Vec<_>>(),
        ["captain", "helm", "tactical", "engineering"]
            .into_iter()
            .map(|id| StationId(id.into()))
            .collect::<Vec<_>>(),
    );
    assert!(npc.tags.iter().any(|tag| tag == "npc"));
}

#[test]
fn dynasty_reserve_uses_explicit_ordinary_allocation() {
    use project_phoenix::core::messages::PowerGroupId;
    use project_phoenix::modifiers::power_system::{PowerConfig, PowerSystem};
    let hull = load_entity_config(PLAYER_HULL).unwrap();
    let reactor = hull.power.unwrap();
    assert!(reactor.strike_reserve.is_some());
    let config = PowerConfig {
        strike_reserve: reactor.strike_reserve.clone(),
        capacity: reactor.capacity,
        rates: reactor.rates,
        sustainable_total: reactor.sustainable_total,
        max_commanded_total: reactor.max_commanded_total,
        emergency_threshold: reactor.emergency_threshold,
    };
    let ship = hull.ship_config.unwrap();
    let seed = project_phoenix::ship::power::authored_power_group_seed(&ship.power_groups);
    let mut power = PowerSystem::from_authored_groups(&config, &seed);
    assert_eq!(power.battery_charge, 0.0);
    power.tick(1.0, &config);
    assert_eq!(power.battery_charge, 0.0, "no passive capture");
    power
        .set_group_allocation(&PowerGroupId("strike-reserve".into()), 2)
        .unwrap();
    power.tick(1.0, &config);
    let nominal_charge = power.battery_charge;
    power
        .set_group_allocation(&PowerGroupId("helm".into()), 1)
        .unwrap();
    power
        .set_group_allocation(&PowerGroupId("strike-reserve".into()), 3)
        .unwrap();
    power.tick(1.0, &config);
    assert!(power.battery_charge - nominal_charge > nominal_charge);
    let serialized = serde_json::to_string(&power.capture_continuation()).unwrap();
    let mut restored = PowerSystem::from_authored_groups(&config, &seed);
    restored.restore_continuation(&serde_json::from_str(&serialized).unwrap());
    for _ in 0..30 {
        power.tick(1.0 / 60.0, &config);
        restored.tick(1.0 / 60.0, &config);
        assert_eq!(
            power.capture_continuation(),
            restored.capture_continuation()
        );
    }
    assert!(
        reactor.ai_policy.is_some(),
        "ordinary Power Backfill remains authored"
    );
    assert!(load_entity_config(NPC_HULL)
        .unwrap()
        .power
        .unwrap()
        .strike_reserve
        .is_none());
}
