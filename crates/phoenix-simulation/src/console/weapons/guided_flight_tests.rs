use super::*;
use crate::entities::spawner::EntityUuid;
use std::collections::HashMap;

fn host() -> App {
    let mut app = App::new();
    register(&mut app);
    let world = app.world_mut();
    world.insert_resource(crate::sim_tick::SimTick(42));
    world.insert_resource(Time::<Fixed>::default());
    crate::world_id::install(world, crate::world_id::WorldIdMint::default());
    // Actual capture joins use optional registered owners even for bare hosts.
    world.register_component::<crate::ship::state::ShipPhysics>();
    world.register_component::<crate::entities::spawner::EntitySystemHull>();
    world.register_component::<crate::ship::state::ShipRedAlert>();
    world.register_component::<crate::console::command::server::ShipStationStances>();
    world.register_component::<crate::ship_plugin::ShipSystemControlSources>();
    world.register_component::<crate::console::weapons::beam::ActiveBeam>();
    world.register_component::<crate::console::weapons::beam::PhaserCooldown>();
    world.register_component::<crate::entities::spawner::EntityShipArcHull>();
    world.register_component::<crate::console::repair::ShipRepairTeams>();
    world.register_component::<crate::console::repair::RepairRequestQueue>();
    world.register_component::<crate::ship::components::RepairHumanAlerted>();
    world.register_component::<crate::server_app::ShipSystemBlackboards>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.register_component::<crate::ship::shields::ShipShields>();
    app
}

fn launched() -> TorpedoSystemResource {
    let mut owner = TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig {
        load_time: 0.1,
        ..Default::default()
    }));
    let tube = owner.0.tubes[0].id.clone();
    assert!(owner.0.start_load(&tube));
    owner.0.tick(0.1, &HashMap::new(), &mut || unreachable!());
    assert!(matches!(
        owner.0.launch(
            &tube,
            "physical-round".into(),
            1.0,
            2.0,
            3.0,
            0.0,
            Some("target".into()),
            Some("source".into())
        ),
        crate::weapons::torpedo::LaunchResult::Launched { .. }
    ));
    owner
}

fn carriers(world: &mut World) -> Vec<Entity> {
    let mut query = world.query_filtered::<Entity, With<GuidedFlightCarrier>>();
    query.iter(world).collect()
}

#[test]
fn actual_source_despawn_moves_flights_without_machinery_or_payload_loss() {
    let mut app = host();
    let owner = launched();
    let before = owner.0.in_flight.clone();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), crate::server_app::Ship, owner))
        .id();
    app.world_mut().despawn(source);
    app.world_mut().flush();
    let rows = carriers(app.world_mut());
    assert_eq!(rows.len(), 1);
    let carrier = rows[0];
    let world = app.world();
    let actual = world.get::<TorpedoSystemResource>(carrier).unwrap();
    assert_eq!(actual.0.in_flight, before);
    assert!(actual.0.tubes.is_empty());
    assert!(actual.0.burst_states.is_empty());
    assert_eq!(actual.0.magazine.level, 0);
    assert_eq!(actual.0.magazine.ceiling, 0);
    assert!(world.get::<crate::server_app::Ship>(carrier).is_none());
    assert!(world.get::<Transform>(carrier).is_none());
    assert!(world
        .get::<crate::ship_plugin::ShipConfigComponent>(carrier)
        .is_none());
    assert!(world
        .get::<crate::ship::power::ShipPowerSystem>(carrier)
        .is_none());
    // The old source attribution and launch facts live on the real round.
    assert_eq!(actual.0.in_flight[0].source_uuid.as_deref(), Some("source"));
}

#[test]
fn replacing_actual_launcher_keeps_existing_flights_once_and_cannot_upgrade_their_profile() {
    let mut app = host();
    let original = launched();
    let before = original.0.in_flight.clone();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), original.clone()))
        .id();
    let mut replacement = original;
    replacement.0.config.damage_hull = 100000;
    replacement.0.in_flight[0].profile.damage_hull = 100000;
    app.world_mut().entity_mut(source).insert(replacement);
    app.world_mut().flush();
    assert!(app
        .world()
        .get::<TorpedoSystemResource>(source)
        .unwrap()
        .0
        .in_flight
        .is_empty());
    let rows = carriers(app.world_mut());
    assert_eq!(rows.len(), 1);
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(rows[0])
            .unwrap()
            .0
            .in_flight,
        before
    );
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(source)
            .unwrap()
            .0
            .config
            .damage_hull,
        100000
    );
}

#[test]
fn carrier_json_restore_discards_bootstrap_flights_and_continues_same_vector_exactly() {
    let mut original = host();
    let source = original
        .world_mut()
        .spawn((EntityUuid("source".into()), launched()))
        .id();
    original.world_mut().despawn(source);
    original.world_mut().flush();
    let snapshot = crate::snapshot::capture(original.world());
    assert_eq!(snapshot.entities.len(), 1);
    assert!(snapshot.entities[0].guided_flight_carrier);
    assert_eq!(
        snapshot.entities[0]
            .weapons
            .as_ref()
            .unwrap()
            .torpedoes_in_flight
            .len(),
        1
    );
    let snapshot = serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let mut resumed = host();
    resumed
        .world_mut()
        .spawn((EntityUuid("bootstrap-not-saved".into()), launched()));
    let report = crate::snapshot::restore(resumed.world_mut(), &snapshot);
    assert!(report.is_complete(), "{report:?}");
    resumed.world_mut().flush();
    assert_eq!(
        carriers(resumed.world_mut()).len(),
        1,
        "bootstrap removal must not rehome another flight"
    );
    let left = carriers(original.world_mut())[0];
    let right = carriers(resumed.world_mut())[0];
    let targets = HashMap::from([("target".into(), (10.0, 8.0, -30.0))]);
    for step in 0..360 {
        assert_eq!(
            crate::snapshot::capture(original.world()),
            crate::snapshot::capture(resumed.world()),
            "step{step}"
        );
        assert_eq!(
            crate::sim_digest::world_digest(original.world()),
            crate::sim_digest::world_digest(resumed.world()),
            "step{step}"
        );
        for (app, entity) in [(&mut original, left), (&mut resumed, right)] {
            app.world_mut()
                .get_mut::<TorpedoSystemResource>(entity)
                .unwrap()
                .0
                .tick_with_work(
                    1.0 / 60.0,
                    &targets,
                    &mut |_| panic!("carrier must never request work"),
                    &mut || panic!("carrier must never mint a shot"),
                );
        }
    }
}

#[test]
fn forged_carrier_machinery_and_single_actor_duplicate_inverses_do_not_write() {
    let mut app = host();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), launched()))
        .id();
    let saved_source = crate::snapshot::capture(app.world())
        .entities
        .into_iter()
        .find(|row| row.uuid == "source")
        .unwrap();
    assert_eq!(
        saved_source
            .weapons
            .as_ref()
            .unwrap()
            .torpedoes_in_flight
            .len(),
        1
    );
    app.world_mut().despawn(source);
    app.world_mut().flush();
    let before = crate::snapshot::capture(app.world());
    for case in 0..4 {
        let mut forged = before.clone();
        match case {
            0 => forged.entities[0].physics = Some([0.0; 8]),
            1 => forged.entities[0]
                .weapons
                .as_mut()
                .unwrap()
                .bursts
                .push(crate::snapshot::BurstState::default()),
            2 => {
                forged.entities[0]
                    .weapons
                    .as_mut()
                    .unwrap()
                    .magazine
                    .as_mut()
                    .unwrap()
                    .level = 1
            }
            3 => forged.entities[0].guided_flight_carrier = false,
            _ => unreachable!(),
        }
        let report = crate::snapshot::restore(app.world_mut(), &forged);
        assert!(!report.is_complete(), "case{case}: {report:?}");
        assert_eq!(crate::snapshot::capture(app.world()), before);
    }
    let new_body = app
        .world_mut()
        .spawn((EntityUuid("source".into()), empty()))
        .id();
    crate::snapshot::apply_entity_state(app.world_mut(), new_body, &saved_source, 42);
    assert!(app
        .world()
        .get::<TorpedoSystemResource>(new_body)
        .unwrap()
        .0
        .in_flight
        .is_empty());
    assert_eq!(carriers(app.world_mut()).len(), 1);
}

#[test]
fn nonmission_cleanup_does_not_rehome_actual_flights() {
    let mut app = host();
    app.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), launched()))
        .id();
    app.world_mut().despawn(source);
    app.world_mut().flush();
    assert!(carriers(app.world_mut()).is_empty());
}
