use super::*;

#[path = "activity_snapshot_tests.rs"]
mod activities;

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/personnel_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.insert_resource(SimTick(20));
    world.insert_resource(Time::<Fixed>::default());
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "personnel-ship".into(),
        None,
    );
    world.flush();
    let profile = world.get::<CompiledPersonnel>(entity).unwrap().clone();
    let jobs = world
        .get::<crate::console::repair::server::ShipRepairTeams>(entity)
        .unwrap()
        .0
        .save_state();
    world
        .get_mut::<PersonnelState>(entity)
        .unwrap()
        .post(
            &profile,
            &jobs,
            config.interior.as_ref().unwrap(),
            "repair-chief",
            Some(0),
            18,
        )
        .unwrap();
    let saved = capture(&world);
    assert!(saved.entities[0].personnel.is_some());
    assert!(saved.entities[0].repair.as_ref().unwrap().jobs.is_some());
    (world, entity, saved)
}

#[test]
fn actual_spawn_and_json_application_preserve_one_custody_owner_and_team_frontier() {
    let (mut world, entity, saved) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    validate_roster(&world, &decoded).unwrap();
    let jobs = world
        .get::<crate::console::repair::server::ShipRepairTeams>(entity)
        .unwrap()
        .0
        .save_state();
    let profile = world.get::<CompiledPersonnel>(entity).unwrap().clone();
    world
        .entity_mut(entity)
        .insert(PersonnelState::new(&profile));
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &decoded.entities[0],
        decoded.tick,
    );
    assert_eq!(
        world.get::<PersonnelState>(entity),
        decoded.entities[0].personnel.as_ref()
    );
    assert_eq!(
        world
            .get::<crate::console::repair::server::ShipRepairTeams>(entity)
            .unwrap()
            .0
            .save_state(),
        jobs
    );
    assert_eq!(
        capture(&world).entities[0].personnel,
        decoded.entities[0].personnel
    );
}

#[test]
fn forged_custody_refuses_before_any_global_or_entity_write() {
    let (mut world, _, saved) = fixture();
    let before = capture(&world);
    let before_digest = crate::sim_digest::world_digest(&world);
    for fault in 0..5 {
        let mut candidate = saved.clone();
        candidate.tick += 10;
        let row = &mut candidate.entities[0];
        match fault {
            0 => row.personnel = None,
            1 => {
                row.personnel
                    .as_mut()
                    .unwrap()
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .changed_tick = candidate.tick + 1
            }
            2 => {
                row.personnel
                    .as_mut()
                    .unwrap()
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .location =
                    phoenix_model::personnel::OfficerLocation::WithRepairTeam { team_idx: 99 }
            }
            3 => row.repair.as_mut().unwrap().jobs = None,
            _ => {
                row.personnel.as_mut().unwrap().officers.clear();
            }
        }
        let report = restore(&mut world, &candidate);
        assert!(!report.is_complete());
        assert!(
            format!("{:?}", report.gaps).contains("personnel")
                || format!("{:?}", report.gaps).contains("officer")
        );
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), before_digest);
    }
}

#[path = "planning_snapshot_tests.rs"]
mod planning;
