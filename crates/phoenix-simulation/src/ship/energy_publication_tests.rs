use super::*;
use crate::modifiers::power_system::{PowerConfig, PowerSystem, StrikeReserveConfig};
use crate::resources::server::ResourceServices;
use crate::resources::service::{ResourceEndpoint, ResourceLink};
use crate::server_app::ShipSystemBlackboards;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};

fn replace_inputs(app: &mut App, entity: Entity, case: usize) {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut hardware = authored.energy.unwrap();
    let mut ship = authored.ship_config.unwrap();
    if case == 1 {
        hardware.groups.reverse();
        hardware.consumers.reverse();
        for group in &mut hardware.groups {
            group.parent = None;
            group.max_flow_per_sec += 1.0;
        }
        for group in ship.power_groups.values_mut() {
            group.label = "changed label".into();
        }
        let old = hardware.consumers[0].system.clone();
        let new = SystemId("replacement-consumer".into());
        hardware.consumers[0].system = new.clone();
        ship.systems
            .iter_mut()
            .find(|system| system.id == old)
            .unwrap()
            .id = new;
        ship.systems
            .iter_mut()
            .for_each(|system| system.power_group = None);
    }
    if case == 2 {
        hardware.consumers.clear();
        hardware.groups.clear();
    }
    for (index, consumer) in hardware.consumers.iter_mut().enumerate() {
        consumer.coolant = ResourceLink {
            provider: Some(ResourceEndpoint::Capacity {
                id: if case == 1 && index == 0 {
                    "second-provider"
                } else {
                    "coolant"
                }
                .into(),
            }),
            service: None,
        };
    }
    hardware.reactor.coolant = if case == 2 {
        ResourceLink {
            service: Some("missing".into()),
            provider: None,
        }
    } else {
        ResourceLink {
            provider: Some(ResourceEndpoint::Capacity {
                id: "coolant".into(),
            }),
            service: None,
        }
    };
    let mut power = PowerSystem::from_energy(hardware, ship.clone()).unwrap();
    let legacy = PowerConfig {
        strike_reserve: Some(StrikeReserveConfig {
            group: "reserve".into(),
            units_per_level: 1.0,
            ai_enable_at: None,
            weapons: [(
                "fixture-weapon".into(),
                crate::modifiers::strike_reserve::StrikeWeaponConfig {
                    cost: 3.0,
                    damage_multiplier: 2.0,
                },
            )]
            .into(),
        }),
        ..Default::default()
    };
    power = power
        .with_strike_reserve(legacy.strike_reserve.clone())
        .unwrap();
    power.set_strike_boost(case == 1);
    if case == 0 {
        let id = power
            .config()
            .consumers
            .iter()
            .find(|consumer| {
                power
                    .ship_config()
                    .systems
                    .iter()
                    .any(|system| system.id == consumer.system && system.power_group.is_some())
            })
            .unwrap()
            .system
            .clone();
        power.set_fine_flow(&id, Some(0.25)).unwrap();
    }
    if case == 1 {
        let ids: Vec<_> = power
            .config()
            .consumers
            .iter()
            .map(|consumer| consumer.system.clone())
            .collect();
        let mut saved = power.capture_continuation();
        saved.last_reactor_energy_delta_per_sec = -1.0;
        saved.coolant_credit.insert("coolant".into(), 0.25);
        for id in ids {
            saved.last_intake_per_sec.insert(id.clone(), 0.5);
            saved.network.fine_flows.insert(id, 0.25);
        }
        // Publication is read-only: use the same actual dynamic reserve fields
        // even when a fixture does not run the allocation/thermal processors.
        let network = power.network_mut();
        network.generation_enabled = false;
        network.mode = EnergyAllocationMode::Minimal;
        network.cooling_usage = CoolingUsage::High;
        network.reactor.energy = 1.0;
        network.reactor.heat = 123.0;
        network.reactor.thermal_stopped = true;
        network.output_actual_per_sec = 0.0;
        for state in network.groups.values_mut() {
            state.cut = true;
            state.effective_flow_per_sec = 0.0;
            state.priority = 7;
        }
        for state in network.consumers.values_mut() {
            state.energy = 0.5;
            state.heat = 42.0;
            state.thermal_stopped = true;
        }
        // Restore actual continuation receipts as well as reserve/thermal state.
        saved.network = power.network().clone();
        saved.network.fine_flows.clear(); // ungrouped consumers cannot have overrides
        power.restore_continuation(&saved).unwrap();
    }
    let mut sources = ShipSystemControlSources::default();
    sources.0.set_offline(
        crate::ship::system_registry::power_reactor_system_id(),
        case == 1,
    );
    sources.0.set_offline(
        crate::ship::system_registry::power_battery_system_id(),
        case == 2,
    );
    app.world_mut().entity_mut(entity).insert((
        ShipPowerSystem(power),
        ShipConfigComponent(ship),
        sources,
        ResourceServices(authored.resource_services),
        PowerConfigResource(legacy),
    ));
    if let Some(config) = authored.infrastructure {
        let mut state = crate::infrastructure::InfrastructureState::from_config(&config);
        if case == 1 {
            state.adjust_capacity("coolant", -10);
        }
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::infrastructure::InfrastructureCondition(state));
    }
    if case == 2 {
        app.world_mut()
            .entity_mut(entity)
            .remove::<PowerConfigResource>();
    }
}

fn boards(app: &App, entity: Entity) -> serde_json::Value {
    serde_json::to_value(&app.world().get::<ShipSystemBlackboards>(entity).unwrap().0).unwrap()
}
#[test]
fn reused_power_publication_matches_original_across_refit_restore_and_sources() {
    let mut actual = App::new();
    actual.add_systems(Update, publish_power_blackboard);
    let a = actual
        .world_mut()
        .spawn(ShipSystemBlackboards::default())
        .id();
    let mut expected = App::new();
    expected.add_systems(Update, reference::publish_power_blackboard_reference);
    let b = expected
        .world_mut()
        .spawn(ShipSystemBlackboards::default())
        .id();
    for case in [0, 0, 1, 2, 0] {
        replace_inputs(&mut actual, a, case);
        replace_inputs(&mut expected, b, case);
        actual.update();
        expected.update();
        assert_eq!(boards(&actual, a), boards(&expected, b), "case {case}");
    }
    let key = SystemId(crate::ship::system_registry::POWER_SYSTEM_ID.into());
    for replacement in [
        SystemBlackboard::Power(PowerBlackboard::default()),
        SystemBlackboard::PowerBattery(PowerBatteryBlackboard {
            charge: 99.0,
            capacity: 99.0,
            is_online: false,
            emergency_threshold: 9.0,
        }),
    ] {
        actual
            .world_mut()
            .get_mut::<ShipSystemBlackboards>(a)
            .unwrap()
            .0
            .insert(key.clone(), replacement);
        actual.update();
        expected.update();
        assert_eq!(boards(&actual, a), boards(&expected, b));
    }
}

#[test]
fn unchanged_power_publication_retains_vector_and_string_allocations() {
    let mut app = App::new();
    app.add_systems(Update, publish_power_blackboard);
    let entity = app.world_mut().spawn(ShipSystemBlackboards::default()).id();
    replace_inputs(&mut app, entity, 0);
    app.update();
    let pointers = |app: &App| {
        let boards = &app.world().get::<ShipSystemBlackboards>(entity).unwrap().0;
        let SystemBlackboard::Power(board) =
            &boards[&SystemId(crate::ship::system_registry::POWER_SYSTEM_ID.into())]
        else {
            panic!()
        };
        let energy = board.energy.as_ref().unwrap();
        (
            energy.groups.as_ptr(),
            energy.consumers.as_ptr(),
            energy.coolant.as_ptr(),
            energy.groups[0].id.as_ptr(),
            energy.groups[0].label.as_ptr(),
            energy.consumers[0].system.0.as_ptr(),
            energy.coolant[0].provider.as_ptr(),
        )
    };
    let before = pointers(&app);
    app.update();
    assert_eq!(before, pointers(&app));
}
