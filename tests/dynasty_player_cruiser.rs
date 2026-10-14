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
        assert!(ship
            .consoles
            .iter()
            .any(|console| console.home_station.as_ref() == Some(&station.id)
                && console
                    .screen
                    .as_deref()
                    .is_some_and(|path| path.starts_with("gui/dynasty-cruiser/"))));
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
fn dynasty_reserve_uses_the_finite_reactor_capacitor_and_authored_power_tree() {
    use project_phoenix::modifiers::power_system::PowerSystem;
    use project_phoenix::resources::energy_network::EnergyCoolantBindings;
    let hull = load_entity_config(PLAYER_HULL).unwrap();
    let strike = hull.power.as_ref().unwrap().strike_reserve.clone();
    assert!(strike.is_some());
    assert!(hull.power.as_ref().unwrap().ai_policy.is_none());
    let hardware = hull.energy.unwrap();
    let ship = hull.ship_config.unwrap();
    let reactor = ship
        .systems
        .iter()
        .find(|system| system.kind == "power_reactor")
        .unwrap();
    let (programs, frames) =
        project_phoenix::ai::state_tree_runtime::compile_ship_trees_with_energy(
            &ship,
            Some(&hardware),
        )
        .unwrap();
    assert!(programs.0.contains_key(&reactor.id) && frames.0.contains_key(&reactor.id));
    let mut power = PowerSystem::from_energy(hardware.clone(), ship)
        .unwrap()
        .with_strike_reserve(strike.clone())
        .unwrap();
    assert_eq!(power.battery_charge(), power.network().reactor.energy);
    let bindings = EnergyCoolantBindings {
        reactor: "coolant-hold".into(),
        consumers: hardware
            .consumers
            .iter()
            .map(|consumer| (consumer.system.clone(), "coolant-hold".into()))
            .collect(),
    };
    let mut stock = BTreeMap::from([("coolant-hold".into(), 1000_i64)]);
    for group in &hardware.groups {
        power.set_flow(&group.id, 0.0).unwrap();
    }
    power.network_mut().reactor.energy = 0.0;
    power.network_mut().output_actual_per_sec = 0.0;
    power.set_output(0.0).unwrap();
    let tick = |power: &mut PowerSystem, stock: &mut BTreeMap<String, i64>, dt| {
        let result = power.tick_energy(dt, &bindings, stock, &mut |_| 0).unwrap();
        for (id, used) in result.coolant_withdrawals {
            *stock.get_mut(&id).unwrap() -= used;
        }
    };
    tick(&mut power, &mut stock, 1.0);
    assert_eq!(
        power.battery_charge(),
        0.0,
        "zero output cannot capture free energy"
    );
    power
        .set_output(hardware.reactor.sustainable_output_per_sec)
        .unwrap();
    for _ in 0..10 {
        tick(&mut power, &mut stock, 1.0 / 30.0);
    }
    assert!(
        power.battery_charge() > 0.0,
        "ordinary output charges the same capacitor"
    );
    let mut boosted = power.clone();
    let mut boosted_stock = stock.clone();
    boosted
        .set_output(hardware.reactor.max_output_per_sec)
        .unwrap();
    let before = power.battery_charge();
    tick(&mut power, &mut stock, 1.0);
    tick(&mut boosted, &mut boosted_stock, 1.0);
    assert!(boosted.battery_charge() > power.battery_charge());
    assert!(power.battery_charge() > before);
    let weapon = strike.as_ref().unwrap().weapons.iter().next().unwrap();
    assert!(boosted.battery_charge() >= weapon.1.cost);
    boosted.set_strike_boost(true);
    let before = boosted.battery_charge();
    assert_eq!(
        boosted.fire_strike_weapon(weapon.0),
        weapon.1.damage_multiplier
    );
    assert_eq!(boosted.battery_charge(), before - weapon.1.cost);
    let saved =
        serde_json::from_str(&serde_json::to_string(&boosted.capture_continuation()).unwrap())
            .unwrap();
    let mut restored = power.clone();
    restored.restore_continuation(&saved).unwrap();
    let mut restored_stock = boosted_stock.clone();
    for _ in 0..30 {
        tick(&mut boosted, &mut boosted_stock, 1.0 / 60.0);
        tick(&mut restored, &mut restored_stock, 1.0 / 60.0);
        assert_eq!(
            boosted.capture_continuation(),
            restored.capture_continuation()
        );
        assert_eq!(boosted_stock, restored_stock);
    }
    assert!(load_entity_config(NPC_HULL)
        .unwrap()
        .power
        .unwrap()
        .strike_reserve
        .is_none());
}
