use super::*;

fn id(value: &str) -> SystemId {
    SystemId(value.into())
}

fn rig() -> (World, Entity) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "00000000-0000-0000-0000-000000000013".into(),
        None,
    );
    world.flush();
    assert!(world
        .get::<ShipRepairTeams>(entity)
        .unwrap()
        .0
        .layout()
        .is_some());
    (world, entity)
}

fn replacement(world: &World, entity: Entity, slot: &str) -> String {
    let state = world.get::<ShipFitting>(entity).unwrap();
    let current = &state.equipment.installed(slot).unwrap().definition;
    let profile = &world.get::<CompiledShipFitting>(entity).unwrap().0;
    profile
        .config()
        .equipment
        .iter()
        .find(|equipment| {
            &equipment.id != current && profile.compatible(slot, &equipment.id).is_ok()
        })
        .unwrap()
        .id
        .clone()
}

fn parts(world: &World, entity: Entity) -> i64 {
    world
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock("machinery_parts")
        .unwrap()
        .level
}

#[test]
fn field_refit_uses_existing_team_actual_spare_and_paid_parts_without_recharging() {
    let (mut world, entity) = rig();
    let choice = replacement(&world, entity, "fore-beam");
    let before = world
        .get::<ShipFitting>(entity)
        .unwrap()
        .equipment
        .assemblies
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let initial_parts = parts(&world, entity);
    let initial_energy = world
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .capture_continuation();
    request(&mut world, entity, &id("phaser-fore"), "fore-beam", &choice).unwrap();
    dispatch(&mut world, entity, &id("repair"), 0, "fore-beam").unwrap();
    let mut saw_removal = false;
    for tick in 1..1600 {
        advance(&mut world, entity, tick, 0.2);
        let state = world.get::<ShipFitting>(entity).unwrap();
        saw_removal |= state.equipment.installed("fore-beam").is_none();
        if state.work.is_empty() {
            break;
        }
    }
    let state = world.get::<ShipFitting>(entity).unwrap();
    assert!(saw_removal, "real timed vacancy must precede installation");
    assert!(state.work.is_empty());
    assert_eq!(
        state.equipment.installed("fore-beam").unwrap().definition,
        choice
    );
    assert_eq!(before, state.equipment.assemblies.keys().cloned().collect());
    assert!(parts(&world, entity) < initial_parts);
    let energy = world
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .capture_continuation();
    assert_eq!(energy.network.reactor, initial_energy.network.reactor);
    for (system, before) in &initial_energy.network.consumers {
        if system != &id("repair") {
            assert_eq!(&energy.network.consumers[system], before);
        }
    }
    let definition = world
        .get::<crate::console::weapons::PhaserCombatConfigResource>(entity)
        .unwrap();
    assert_eq!(
        definition
            .0
            .banks
            .iter()
            .find(|bank| bank.id == "fore")
            .unwrap()
            .beam_damage_per_sec,
        40.0
    );
}

#[test]
fn foreign_system_and_unrequested_repair_work_refuse_without_any_equipment_or_team_writes() {
    let (mut world, entity) = rig();
    let state = world.get::<ShipFitting>(entity).unwrap().clone();
    let teams = world.get::<ShipRepairTeams>(entity).unwrap().0.save_state();
    let choice = replacement(&world, entity, "fore-beam");
    assert!(request(
        &mut world,
        entity,
        &id("power-reactor"),
        "fore-beam",
        &choice
    )
    .is_err());
    assert!(dispatch(&mut world, entity, &id("repair"), 0, "fore-beam").is_err());
    assert_eq!(world.get::<ShipFitting>(entity).unwrap(), &state);
    assert_eq!(
        world.get::<ShipRepairTeams>(entity).unwrap().0.save_state(),
        teams
    );
}

#[test]
fn reactor_spare_requires_real_facility_coupling() {
    let (mut world, entity) = rig();
    let choice = replacement(&world, entity, "reactor");
    request(&mut world, entity, &id("power-reactor"), "reactor", &choice).unwrap();
    let before = world.get::<ShipFitting>(entity).unwrap().clone();
    assert!(dispatch(&mut world, entity, &id("repair"), 0, "reactor")
        .unwrap_err()
        .contains("facility"));
    assert_eq!(world.get::<ShipFitting>(entity).unwrap(), &before);
}

#[test]
fn request_changes_intent_and_never_mints_a_missing_assembly() {
    let (mut world, entity) = rig();
    let choice = replacement(&world, entity, "fore-beam");
    world
        .get_mut::<ShipFitting>(entity)
        .unwrap()
        .equipment
        .assemblies
        .retain(|_, item| !matches!(item.location, EquipmentLocation::Stored { .. }));
    let count = world
        .get::<ShipFitting>(entity)
        .unwrap()
        .equipment
        .assemblies
        .len();
    request(&mut world, entity, &id("phaser-fore"), "fore-beam", &choice).unwrap();
    assert!(dispatch(&mut world, entity, &id("repair"), 0, "fore-beam").is_err());
    assert_eq!(
        world
            .get::<ShipFitting>(entity)
            .unwrap()
            .equipment
            .assemblies
            .len(),
        count
    );
}

fn fetched_rig() -> (World, Entity, String, String) {
    let (mut world, entity) = rig();
    let choice = replacement(&world, entity, "fore-beam");
    let mut state = world.get_mut::<ShipFitting>(entity).unwrap();
    let item = state
        .equipment
        .assemblies
        .values_mut()
        .find(|item| {
            item.definition == choice && matches!(item.location, EquipmentLocation::Stored { .. })
        })
        .unwrap();
    let item_id = item.id.clone();
    item.location = EquipmentLocation::Stored {
        locker: "support-equipment".into(),
    };
    drop(state);
    request(&mut world, entity, &id("phaser-fore"), "fore-beam", &choice).unwrap();
    dispatch(&mut world, entity, &id("repair"), 0, "fore-beam").unwrap();
    (world, entity, item_id, choice)
}

#[test]
fn stored_salvage_is_fetched_by_actual_team_then_delivered_before_any_hardware_work() {
    let (mut world, entity, item, choice) = fetched_rig();
    let initial_parts = parts(&world, entity);
    let initial = world.get::<ShipFitting>(entity).unwrap().clone();
    assert_eq!(initial.work["fore-beam"].stage, RefitStage::Fetching);
    assert!(
        matches!(&initial.equipment.assemblies[&item].location, EquipmentLocation::Stored { locker } if locker == "support-equipment")
    );
    let job = initial.work["fore-beam"].job;
    let mut saw_fetch_crossing = false;
    let mut saw_delivery_crossing = false;
    let mut saw_vacancy = false;
    for tick in 1..1600 {
        advance(&mut world, entity, tick, 0.125);
        let state = world.get::<ShipFitting>(entity).unwrap();
        let teams = world.get::<ShipRepairTeams>(entity).unwrap();
        if let Some(work) = state.work.get("fore-beam") {
            assert_eq!(work.job, job);
            let crossing = matches!(
                teams.0.refit_location(job),
                Some(phoenix_model::interior::TeamLocation::Crossing { .. })
            );
            saw_fetch_crossing |= work.stage == RefitStage::Fetching && crossing;
            saw_delivery_crossing |= work.stage == RefitStage::Delivering && crossing;
            if matches!(work.stage, RefitStage::Fetching | RefitStage::Delivering) {
                assert!(!work.reserved);
                assert_eq!(parts(&world, entity), initial_parts);
                assert_eq!(
                    state.equipment.installed("fore-beam"),
                    initial.equipment.installed("fore-beam")
                );
            }
        }
        saw_vacancy |= state.equipment.installed("fore-beam").is_none();
        if state.work.is_empty() {
            break;
        }
    }
    let state = world.get::<ShipFitting>(entity).unwrap();
    assert!(saw_fetch_crossing && saw_delivery_crossing && saw_vacancy);
    assert!(state.work.is_empty());
    assert_eq!(state.equipment.installed("fore-beam").unwrap().id, item);
    assert_eq!(
        state.equipment.installed("fore-beam").unwrap().definition,
        choice
    );
    assert_eq!(
        state.equipment.assemblies.len(),
        initial.equipment.assemblies.len()
    );
    assert_eq!(parts(&world, entity), initial_parts - 1);
}

#[test]
fn cancellation_during_delivery_keeps_item_with_same_team_until_actual_storage_return() {
    let (mut world, entity, item, _) = fetched_rig();
    let original = world
        .get::<ShipFitting>(entity)
        .unwrap()
        .equipment
        .installed("fore-beam")
        .unwrap()
        .clone();
    let initial_parts = parts(&world, entity);
    let mut tick = 1;
    loop {
        advance(&mut world, entity, tick, 0.125);
        tick += 1;
        let state = world.get::<ShipFitting>(entity).unwrap();
        let work = &state.work["fore-beam"];
        if work.stage == RefitStage::Delivering
            && matches!(
                world
                    .get::<ShipRepairTeams>(entity)
                    .unwrap()
                    .0
                    .refit_location(work.job),
                Some(phoenix_model::interior::TeamLocation::Crossing { .. })
            )
        {
            break;
        }
        assert!(tick < 400);
    }
    let job = world.get::<ShipFitting>(entity).unwrap().work["fore-beam"].job;
    cancel(&mut world, entity, &id("repair"), "fore-beam").unwrap();
    advance(&mut world, entity, tick, 0.125);
    tick += 1;
    assert_eq!(
        world.get::<ShipFitting>(entity).unwrap().work["fore-beam"].stage,
        RefitStage::Returning
    );
    assert!(matches!(
        world
            .get::<ShipFitting>(entity)
            .unwrap()
            .equipment
            .assemblies[&item]
            .location,
        EquipmentLocation::InWork { .. }
    ));
    assert!(world
        .get::<ShipRepairTeams>(entity)
        .unwrap()
        .0
        .is_refit_team(0));
    let mut next_tick = tick;
    for now in tick..tick + 800 {
        advance(&mut world, entity, now, 0.125);
        next_tick = now + 1;
        if world.get::<ShipFitting>(entity).unwrap().work.is_empty() {
            break;
        }
    }
    let state = world.get::<ShipFitting>(entity).unwrap();
    assert!(state.work.is_empty());
    assert_eq!(state.equipment.installed("fore-beam"), Some(&original));
    assert!(
        matches!(&state.equipment.assemblies[&item].location, EquipmentLocation::Stored { locker } if locker == "support-equipment")
    );
    assert_eq!(parts(&world, entity), initial_parts);
    assert!(world
        .get::<ShipRepairTeams>(entity)
        .unwrap()
        .0
        .is_refit_team(0)); // Still physically returning home.
    assert_eq!(
        job,
        world
            .get::<ShipRepairTeams>(entity)
            .unwrap()
            .0
            .refit_job(1)
            .unwrap()
    );
    // The fitting work row is gone, but the same actual Job still has to reach
    // home. The idle fast path must not strand this cancelled carrier.
    assert!(world
        .get::<ShipRepairTeams>(entity)
        .unwrap()
        .0
        .has_unsettled_refits());
    for now in next_tick..next_tick + 800 {
        advance(&mut world, entity, now, 0.125);
        if !world
            .get::<ShipRepairTeams>(entity)
            .unwrap()
            .0
            .has_unsettled_refits()
        {
            break;
        }
    }
    let teams = &world.get::<ShipRepairTeams>(entity).unwrap().0;
    assert!(!teams.has_unsettled_refits());
    assert_eq!(teams.job_state().teams[0].as_ref().unwrap().id, job);
    assert_eq!(
        teams.job_state().teams[0].as_ref().unwrap().outcome,
        Some(phoenix_model::job::JobOutcome::Cancelled)
    );
    assert!(teams.free_team_indices(None).contains(&0));
    assert_eq!(parts(&world, entity), initial_parts);
}

#[test]
fn idle_fitting_keeps_selected_equipment_and_clock_without_touching_machinery() {
    let (mut world, entity) = rig();
    let choice = replacement(&world, entity, "fore-beam");
    request(&mut world, entity, &id("phaser-fore"), "fore-beam", &choice).unwrap();
    let mut expected = world.get::<ShipFitting>(entity).unwrap().clone();
    assert!(expected.work.is_empty());
    assert!(!expected.requested.is_empty());
    assert!(!world
        .get::<ShipRepairTeams>(entity)
        .unwrap()
        .0
        .has_unsettled_refits());
    world.clear_trackers();
    advance(&mut world, entity, 17, 0.125);
    expected.last_tick = Some(17);
    assert_eq!(world.get::<ShipFitting>(entity).unwrap(), &expected);
    macro_rules! unchanged {
        ($owner:ty) => {
            assert!(
                !world
                    .entity(entity)
                    .get_ref::<$owner>()
                    .unwrap()
                    .is_changed(),
                "idle fitting must not reinsert {}",
                stringify!($owner)
            );
        };
    }
    unchanged!(ShipRepairTeams);
    unchanged!(InfrastructureCondition);
    unchanged!(ResourceServices);
    unchanged!(ShipDamage);
    unchanged!(ShipPowerSystem);
}

#[test]
fn idle_fitting_missing_required_owner_keeps_original_clock() {
    let (mut world, entity) = rig();
    let before = world.get::<ShipFitting>(entity).unwrap().clone();
    world.entity_mut(entity).remove::<ResourceServices>();
    advance(&mut world, entity, 17, 0.125);
    assert_eq!(world.get::<ShipFitting>(entity).unwrap(), &before);
}
