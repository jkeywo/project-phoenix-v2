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
    probe.probe = Some(phoenix_model::craft::ProbeProfile { space: 1 });
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
