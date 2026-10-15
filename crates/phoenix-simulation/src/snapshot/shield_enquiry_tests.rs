use super::*;
use bevy::ecs::system::RunSystemOnce;
#[test]
fn actual_rotating_equipment_json_metadata_refuses_forged_frontiers_without_mutation() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/ship_harrow_resonant_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.insert_resource(SimTick(71));
    world.insert_resource(Time::<Fixed>::from_seconds(1.0));
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "rotating".into(),
        None,
    );
    world.flush();
    let saved = capture(world.entity(entity)).unwrap();
    assert_eq!(saved.cycle_anchor_tick, Some(71));
    assert_eq!(saved.frequency, 0.2);
    let row = EntityState {
        uuid: "rotating".into(),
        shield_frequency: Some(saved.clone()),
        ..Default::default()
    };
    let snapshot = PhoenixSnapshot {
        tick: 71,
        entities: vec![row],
        ..Default::default()
    };
    let decoded = serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    validate_roster(&world, &decoded).unwrap();
    for mode in 0..4 {
        let mut forged = snapshot.clone();
        let state = forged.entities[0].shield_frequency.as_mut().unwrap();
        match mode {
            0 => state.cycle_anchor_tick = Some(72),
            1 => state.cycle_anchor_tick = None,
            2 => state.frequency = 0.8,
            _ => forged.entities[0].shield_frequency = None,
        }
        assert!(validate_roster(&world, &forged).is_err());
        assert_eq!(capture(world.entity(entity)), Some(saved.clone()));
    }
    world.resource_mut::<SimTick>().0 = 101;
    world
        .run_system_once(crate::ship::shield_enquiry::tick)
        .unwrap();
    let current = capture(world.entity(entity)).unwrap();
    assert_eq!(current.frequency, 0.8);
    apply(&mut world.entity_mut(entity), &saved);
    assert_eq!(capture(world.entity(entity)), Some(saved));
}
