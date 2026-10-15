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
