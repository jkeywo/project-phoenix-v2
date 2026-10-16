use super::*;
use crate::core::messages::SystemId;
use crate::ship::composition::SystemComponentConfig;

fn installed() -> EntityConfig {
    let mut config =
        crate::test_fixtures::load_entity_config("assets/entities/local_jump_cruiser.toml")
            .unwrap();
    let bay_id = SystemId("actual-bay".into());
    let ship = config.ship_config.as_mut().unwrap();
    let mut system = ship.system(&SystemId("local-jump".into())).unwrap().clone();
    system.id = bay_id.clone();
    system.kind = CRAFT_BAY_KIND.into();
    system.channels.clear();
    system.components = vec![
        SystemComponentConfig::PowerInput { id: "input".into() },
        SystemComponentConfig::CraftBay {
            id: "bay".into(),
            power_input: "input".into(),
            config: phoenix_model::craft::BayConfig {
                compartment: "dedicated-probe-room".into(),
                capacity_space: 2,
                launch_work_secs: 2.0,
                recovery_work_secs: 3.0,
                recovery_range: 8.0,
                maximum_parent_speed: 0.1,
                release_offset: [0.0, 0.0, -3.0],
            },
        },
    ];
    ship.systems.push(system);
    let interior = config.interior.as_mut().unwrap();
    let mut room = interior
        .compartments
        .iter()
        .find(|room| room.id == "engineering")
        .unwrap()
        .clone();
    room.id = "dedicated-probe-room".into();
    room.x += 30.0;
    interior.compartments.push(room);
    interior
        .placements
        .insert(bay_id.clone(), "dedicated-probe-room".into());
    let energy = config.energy.as_mut().unwrap();
    let mut consumer = energy
        .consumers
        .iter()
        .find(|consumer| consumer.system.0 == "local-jump")
        .unwrap()
        .clone();
    consumer.system = bay_id;
    energy.consumers.push(consumer);
    config
}
#[test]
fn dedicated_actual_bay_cannot_double_allocate_cargo_or_portable_equipment_space() {
    let config = installed();
    validate_entity(&config).unwrap();
    for fault in 0..3 {
        let mut invalid = config.clone();
        match fault {
            0 => {
                invalid.logistics.as_mut().unwrap().storage[0].compartment =
                    "dedicated-probe-room".into();
            }
            1 => {
                invalid.fitting.as_mut().unwrap().lockers[0].compartment =
                    "dedicated-probe-room".into();
            }
            _ => {
                invalid
                    .interior
                    .as_mut()
                    .unwrap()
                    .placements
                    .insert(SystemId("actual-bay".into()), "engineering".into());
            }
        }
        assert!(validate_entity(&invalid).is_err(), "case {fault}");
    }
}
#[test]
fn actual_bay_without_paid_consumer_refuses_instead_of_borrowing_another_reserve() {
    let mut config = installed();
    config
        .energy
        .as_mut()
        .unwrap()
        .consumers
        .retain(|consumer| consumer.system.0 != "actual-bay");
    assert!(validate_entity(&config).is_err());
    let mut config = installed();
    config
        .energy
        .as_mut()
        .unwrap()
        .consumers
        .iter_mut()
        .find(|consumer| consumer.system.0 == "actual-bay")
        .unwrap()
        .continuous_cost_per_sec = 0.0;
    assert!(validate_entity(&config).is_err());
}

#[test]
fn actual_battery_probe_profile_cannot_gain_generator_or_propulsion_from_custody() {
    let mut probe =
        crate::test_fixtures::load_entity_config("assets/entities/battery_probe.toml").unwrap();
    probe.probe = Some(phoenix_model::craft::ProbeProfile {
        space: 1,
        passenger_berths: Vec::new(),
    });
    probe.mesh = Some(toml::from_str("shape='sphere'\ncolour=[0.4,0.8,1.0]\nradius=0.5").unwrap());
    validate_entity(&probe).unwrap();
    let mut generator = probe.clone();
    generator
        .energy
        .as_mut()
        .unwrap()
        .reactor
        .max_output_per_sec = 1.0;
    assert!(validate_entity(&generator).is_err());
    let mut engine = probe.clone();
    engine.ship_config.as_mut().unwrap().systems[0].kind = HELM_THRUST_KIND.into();
    assert!(validate_entity(&engine).is_err());
    let mut movable = probe.clone();
    movable.collider.as_mut().unwrap().movable = true;
    assert!(validate_entity(&movable).is_err());
    let cruiser =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    probe.mesh = cruiser.mesh;
    assert!(validate_entity(&probe).is_err());
}

#[test]
fn actual_bay_cannot_omit_its_physical_damage_or_operation_owner() {
    let mut config = installed();
    config.damage = None;
    assert!(validate_entity(&config).is_err());
    let mut config = installed();
    config
        .ship_config
        .as_mut()
        .unwrap()
        .systems
        .iter_mut()
        .find(|system| system.id.0 == "actual-bay")
        .unwrap()
        .damage_components
        .retain(|component| component.binding != phoenix_model::damage::DamageBinding::Operation);
    assert!(validate_entity(&config).is_err());
}

#[test]
fn fixed_battery_turret_uses_existing_static_ship_substrate_without_changing_sensor_probe() {
    let turret =
        crate::test_fixtures::load_entity_config("assets/entities/battery_turret.toml").unwrap();
    validate_entity(&turret).unwrap();
    assert!(turret.is_static_point_defence());
    assert!(turret.behaviour.is_none());
    assert!(turret.helm_console.is_none());
    assert_eq!(
        turret.weapons_console.as_ref().unwrap().phaser_banks.len(),
        1
    );
    let bank = &turret.weapons_console.as_ref().unwrap().phaser_banks[0];
    assert_eq!(bank.id, "turret");
    assert!(
        turret
            .weapons_console
            .as_ref()
            .unwrap()
            .ai
            .as_ref()
            .unwrap()
            .idle
    );
    assert_eq!(bank.beam_range, 40.0);
    assert_eq!(bank.beam_damage_per_sec, 2.0);
    let sensor =
        crate::test_fixtures::load_entity_config("assets/entities/physical_probe.toml").unwrap();
    validate_entity(&sensor).unwrap();
    assert!(!sensor.is_static_point_defence());
    assert_eq!(
        turret.energy.as_ref().unwrap().reactor,
        sensor.energy.as_ref().unwrap().reactor
    );
    let original_sensor = &sensor.energy.as_ref().unwrap().consumers[0];
    assert_eq!(
        turret
            .energy
            .as_ref()
            .unwrap()
            .consumers
            .iter()
            .find(|consumer| consumer.system == original_sensor.system),
        Some(original_sensor)
    );
    assert_eq!(
        turret.collider.as_ref().unwrap(),
        sensor.collider.as_ref().unwrap()
    );
    assert_eq!(
        turret.interior.as_ref().unwrap().compartments,
        sensor.interior.as_ref().unwrap().compartments
    );
    assert_eq!(
        turret.probe.as_ref().unwrap().space,
        sensor.probe.as_ref().unwrap().space
    );
}

#[test]
fn fixed_turret_refuses_unsupported_hardware_and_missing_real_ai_scope_or_paid_work() {
    use phoenix_model::console::ConsoleObservationKind;
    let valid =
        crate::test_fixtures::load_entity_config("assets/entities/battery_turret.toml").unwrap();
    for fault in 0..13 {
        let mut invalid = valid.clone();
        let bank = SystemId("phaser-turret".into());
        let radar = SystemId("tactical-radar".into());
        match fault {
            0 => {
                invalid
                    .ship_config
                    .as_mut()
                    .unwrap()
                    .systems
                    .iter_mut()
                    .find(|system| system.id == bank)
                    .unwrap()
                    .ai_only = false
            }
            1 => invalid.ship_config.as_mut().unwrap().consoles[0]
                .feeds
                .retain(|feed| feed.kind != ConsoleObservationKind::TacticalRadar),
            2 => invalid.ship_config.as_mut().unwrap().consoles[0]
                .feeds
                .retain(|feed| feed.kind != ConsoleObservationKind::PhaserBank),
            3 => {
                invalid
                    .energy
                    .as_mut()
                    .unwrap()
                    .consumers
                    .iter_mut()
                    .find(|consumer| consumer.system == bank)
                    .unwrap()
                    .continuous_cost_per_sec = 0.0
            }
            4 => {
                invalid
                    .energy
                    .as_mut()
                    .unwrap()
                    .consumers
                    .iter_mut()
                    .find(|consumer| consumer.system == radar)
                    .unwrap()
                    .continuous_cost_per_sec = f32::NAN
            }
            5 => invalid
                .probe
                .as_mut()
                .unwrap()
                .passenger_berths
                .push("occupied-berth".into()),
            6 => invalid
                .weapons_console
                .as_mut()
                .unwrap()
                .blaster_banks
                .push(Default::default()),
            7 => invalid.collider.as_mut().unwrap().movable = true,
            8 => invalid.energy.as_mut().unwrap().reactor.max_output_per_sec = 1.0,
            9 => invalid
                .sensing
                .as_mut()
                .unwrap()
                .broad
                .retain(|scanner| scanner.system != radar),
            10 => invalid.sensing.as_mut().unwrap().control.combat_lock = None,
            11 => invalid.weapons_console.as_mut().unwrap().selector = None,
            _ => invalid.ship_config.as_mut().unwrap().consoles[0]
                .feeds
                .retain(|feed| feed.kind != ConsoleObservationKind::Viewscreen),
        }
        assert!(validate_entity(&invalid).is_err(), "case {fault}");
    }
}

#[test]
fn actual_identity_only_decoy_uses_paid_fixed_equipment_without_fitting_a_sensor_or_weapon() {
    let decoy =
        crate::test_fixtures::load_entity_config("assets/entities/battery_decoy.toml").unwrap();
    validate_entity(&decoy).unwrap();
    assert!(!decoy.is_static_point_defence());
    assert!(
        decoy.weapons_console.is_none()
            && decoy.sensing.is_none()
            && decoy.sensors_console.is_none()
    );
    assert!(decoy.behaviour.is_none() && decoy.helm_console.is_none());
    let ship = decoy.ship_config.as_ref().unwrap();
    assert_eq!(ship.systems.len(), 1);
    assert!(ship.stations.is_empty());
    let emitter = &ship.systems[0];
    assert_eq!(emitter.kind, IDENTITY_EMITTER_KIND);
    let composition = crate::ship::composition::identity_composition(&emitter.components).unwrap();
    assert_eq!(composition.config.callsign, "Harrow frigate");
    assert_ne!(
        composition.config.faction(),
        decoy.faction.map(|faction| faction.to_string())
    );
    let battery = &decoy.energy.as_ref().unwrap().consumers[0];
    assert_eq!(battery.system, emitter.id);
    assert_eq!(battery.initial_energy, 24.0);
    assert_eq!(battery.continuous_cost_per_sec, 2.0);
    assert_eq!(decoy.energy.as_ref().unwrap().reactor.initial_energy, 0.0);
    let mut sensor =
        crate::test_fixtures::load_entity_config("assets/entities/physical_probe.toml").unwrap();
    sensor.sensing = None;
    assert!(
        validate_entity(&sensor).is_err(),
        "sensorless acceptance belongs solely to actual identity-only composition"
    );
}

#[test]
fn identity_decoy_refuses_other_capabilities_unpaid_work_and_missing_actual_owners() {
    let valid =
        crate::test_fixtures::load_entity_config("assets/entities/battery_decoy.toml").unwrap();
    let sensor =
        crate::test_fixtures::load_entity_config("assets/entities/physical_probe.toml").unwrap();
    for fault in 0..13 {
        let mut invalid = valid.clone();
        match fault {
            0 => invalid.energy.as_mut().unwrap().reactor.initial_energy = 1.0,
            1 => invalid.energy.as_mut().unwrap().reactor.max_output_per_sec = 1.0,
            2 => invalid.energy.as_mut().unwrap().consumers[0].initial_energy = 0.0,
            3 => invalid.energy.as_mut().unwrap().consumers[0].continuous_cost_per_sec = f32::NAN,
            4 => invalid.ship_config.as_mut().unwrap().systems[0].ai_only = false,
            5 => invalid.ship_config.as_mut().unwrap().consoles.clear(),
            6 => invalid
                .probe
                .as_mut()
                .unwrap()
                .passenger_berths
                .push("passenger".into()),
            7 => invalid.sensing = sensor.sensing.clone(),
            8 => invalid
                .ship_config
                .as_mut()
                .unwrap()
                .systems
                .push(sensor.ship_config.as_ref().unwrap().systems[0].clone()),
            9 => invalid.ship_config.as_mut().unwrap().systems[0]
                .channels
                .clear(),
            10 => invalid.ship_config.as_mut().unwrap().systems[0].config = None,
            11 => invalid.collider.as_mut().unwrap().movable = true,
            _ => invalid.comms = Some(toml::from_str("range=40\nhailable=true").unwrap()),
        }
        assert!(validate_entity(&invalid).is_err(), "case {fault}");
    }
}

#[test]
fn actual_accepted_unshielded_recipes_keep_only_authored_consoles() {
    for (path, expected) in [
        ("assets/entities/battery_decoy.toml", "decoy"),
        ("assets/entities/physical_probe.toml", "probe"),
    ] {
        let resolved = crate::test_fixtures::resolve_from_disk(path).unwrap();
        let config = EntityConfig::from_toml(&resolved.toml).unwrap();
        assert!(config.shield_arcs.is_empty());
        let ship = config.ship_config.as_ref().unwrap();
        assert_eq!(ship.consoles.len(), 1, "{path}");
        assert_eq!(ship.consoles[0].id.0, expected, "{path}");
        assert!(ship.systems.iter().all(|system| system
            .console
            .as_ref()
            .is_some_and(|console| console.0 == expected)));
    }
}

#[test]
fn actual_accepted_unshielded_recipe_preserves_explicit_empty_console() {
    let resolved =
        crate::test_fixtures::resolve_from_disk("assets/entities/physical_probe.toml").unwrap();
    let mut source: toml::Value = toml::from_str(&resolved.toml).unwrap();
    source
        .get_mut("console")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(toml::from_str::<toml::Value>("id='authored-empty'").unwrap());
    let config = EntityConfig::from_toml(&toml::to_string(&source).unwrap()).unwrap();
    let ship = config.ship_config.as_ref().unwrap();
    assert_eq!(
        ship.consoles
            .iter()
            .map(|console| console.id.0.as_str())
            .collect::<Vec<_>>(),
        ["probe", "authored-empty"]
    );
}

#[test]
fn actual_accepted_shielded_hull_keeps_generated_arc_scope_and_feeds() {
    use phoenix_model::console::ConsoleObservationKind;
    let resolved =
        crate::test_fixtures::resolve_from_disk("assets/entities/ship_harrow_patrol.toml").unwrap();
    let config = EntityConfig::from_toml(&resolved.toml).unwrap();
    assert!(!config.shield_arcs.is_empty());
    let ship = config.ship_config.as_ref().unwrap();
    assert!(ship
        .systems
        .iter()
        .all(|system| system.kind != SHIELDS_KIND));
    let generated = ship
        .consoles
        .iter()
        .filter(|console| console.id.0 == "generated-shields")
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 1);
    for arc in &config.shield_arcs {
        let id = shield_arc_system_id(&arc.id).unwrap();
        let system = ship.system(&id).unwrap();
        assert_eq!(system.console.as_ref(), Some(&generated[0].id));
        assert!(system.ai_only);
        assert!(generated[0]
            .feeds
            .iter()
            .any(|feed| feed.source == id && feed.kind == ConsoleObservationKind::ShieldArc));
    }
}
