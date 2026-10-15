use super::server::*;
use bevy::{ecs::system::RunSystemOnce, prelude::*};

fn power() -> crate::ship::power::ShipPowerSystem {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    crate::ship::power::ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(
            config.energy.unwrap(),
            config.ship_config.unwrap(),
        )
        .unwrap(),
    )
}

#[test]
fn actual_hails_use_the_issuing_fleet_slot_on_both_local_ship_projections() {
    use crate::{
        command_admission::HostSlot,
        core::messages::{AdmittedCommand, AdmittedCommands, SystemControlPayload},
        entities::spawner::EntityUuid,
        lockstep::FleetSlotOf,
        server_app::{LocalShip, Ship},
        ship::power::ShipPowerSystem,
    };
    let mut results = Vec::new();
    for local in [1, 2] {
        let mut app = App::new();
        let mut runtime = CommsRuntime::default();
        runtime.range_active = true;
        runtime.fleet_range_active = true;
        // The local projection deliberately contradicts one actual fleet slot.
        runtime.range_flags.insert("station".into(), local == 1);
        runtime
            .fleet_range_flags
            .insert(HostSlot(1), [("station".into(), false)].into());
        runtime
            .fleet_range_flags
            .insert(HostSlot(2), [("station".into(), true)].into());
        app.insert_resource(runtime)
            .init_resource::<crate::world::server::WorldContentRuntime>();
        let owner = crate::ship::system_registry::comms_system_id();
        let mut ships = Vec::new();
        for slot in [1, 2] {
            let entity = app
                .world_mut()
                .spawn((
                    Ship,
                    EntityUuid(format!("ship-{slot}")),
                    FleetSlotOf(HostSlot(slot)),
                    power(),
                    AdmittedCommands(vec![AdmittedCommand {
                        target: owner.clone(),
                        payload: SystemControlPayload::Hail {
                            target_uuid: "station".into(),
                        },
                        response_token: None,
                        feedback_correlation: None,
                    }]),
                ))
                .id();
            if slot == local {
                app.world_mut().entity_mut(entity).insert(LocalShip);
            }
            ships.push(entity);
        }
        let before: Vec<_> = ships
            .iter()
            .map(|entity| {
                app.world()
                    .get::<ShipPowerSystem>(*entity)
                    .unwrap()
                    .0
                    .network()
                    .consumers[&owner]
                    .energy
            })
            .collect();
        app.world_mut()
            .run_system_once(crate::console::comms::server::handle_hail)
            .unwrap();
        let history = app.world().resource::<CommsRuntime>().open_hails.clone();
        assert!(!history.contains("ship-1", "station"));
        assert!(history.contains("ship-2", "station"));
        let after: Vec<_> = ships
            .iter()
            .map(|entity| {
                app.world()
                    .get::<ShipPowerSystem>(*entity)
                    .unwrap()
                    .0
                    .network()
                    .consumers[&owner]
                    .energy
            })
            .collect();
        assert_eq!(before[0], after[0], "an unreachable hail spends no work");
        assert!(
            after[1] < before[1],
            "the reached hail pays actual fitted Comms work"
        );
        let events = &app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_world_events;
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], crate::world::content::WorldEvent::Hailed { target_uuid } if target_uuid == "station")
        );
        results.push((history, after));
    }
    assert_eq!(
        results[0], results[1],
        "LocalShip cannot change authoritative hail outcomes"
    );
}

#[test]
fn actual_clear_retires_only_its_source_hail_history() {
    use crate::{
        core::messages::{AdmittedCommand, AdmittedCommands, SystemControlPayload},
        entities::spawner::EntityUuid,
        server_app::Ship,
    };
    let mut app = App::new();
    let mut runtime = CommsRuntime::default();
    runtime.open_hails.record("a", "station");
    runtime.open_hails.record("b", "station");
    app.insert_resource(runtime)
        .init_resource::<CommsInboxRes>();
    app.world_mut().spawn((
        Ship,
        EntityUuid("a".into()),
        AdmittedCommands(vec![AdmittedCommand {
            target: crate::ship::system_registry::comms_system_id(),
            payload: SystemControlPayload::ClearComms,
            response_token: None,
            feedback_correlation: None,
        }]),
    ));
    app.world_mut()
        .run_system_once(crate::console::comms::server::handle_clear_comms)
        .unwrap();
    let history = &app.world().resource::<CommsRuntime>().open_hails;
    assert!(!history.contains("a", "station"));
    assert!(history.contains("b", "station"));
}
