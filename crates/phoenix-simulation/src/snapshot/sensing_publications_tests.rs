use super::*;
use crate::snapshot::EntityState;
use bevy::ecs::system::RunSystemOnce;

const OBSERVER: &str = "00000000-0000-0000-0000-000000000141";
const SUBJECT: &str = "00000000-0000-0000-0000-000000000142";
fn owner() -> SystemId {
    SystemId("sensor-radar".into())
}
fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.insert_resource(Time::<Fixed>::from_seconds(0.1));
    world.insert_resource(crate::sim_tick::SimTick(10));
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        OBSERVER.into(),
        None,
    );
    world.flush();
    world.spawn((
        EntityUuid(SUBJECT.into()),
        Transform::from_xyz(10.0, 0.0, 0.0),
        crate::entities::spawner::EntityTagsSection(vec!["ship".into()]),
        crate::entities::spawner::EntityTarget(crate::entities::target::TargetSection {
            tags: vec!["ship".into()],
            threat_level: Default::default(),
            description: None,
        }),
    ));
    world
        .get_mut::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .work_continuous(&owner(), 0.1, 1.0)
        .unwrap();
    world.run_system_once(crate::science::broad::tick).unwrap();
    world
        .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
        .unwrap();
    let saved = world.get::<ShipSystemBlackboards>(entity).unwrap().0[&owner()].clone();
    assert_eq!(
        crate::science::contact_routing::observations(&saved)
            .unwrap()
            .len(),
        1,
        "real finite paid acquisition must produce the held measurement"
    );
    // This is the production order: publications complete before the counter
    // advances. No new acquisition takes place during restore.
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 11;
    world
        .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
        .unwrap();
    let mut snapshot = super::super::capture(&world);
    snapshot.entities = vec![EntityState {
        uuid: OBSERVER.into(),
        blackboards: vec![(owner().0, saved)],
        ..Default::default()
    }];
    (world, entity, snapshot)
}
fn tracks(world: &World, entity: Entity) -> Vec<SensorTrackSnapshot> {
    crate::science::contact_routing::observations(
        &world.get::<ShipSystemBlackboards>(entity).unwrap().0[&owner()],
    )
    .unwrap()
    .to_vec()
}

#[test]
fn real_acquired_view_keeps_its_frontier_without_rewinding_work_or_freezing_next_publication() {
    let (mut world, entity, snapshot) = fixture();
    let saved =
        crate::science::contact_routing::observations(&snapshot.entities[0].blackboards[0].1)
            .unwrap()
            .to_vec();
    let fresh = tracks(&world, entity);
    assert!(fresh[0].position_error > saved[0].position_error);
    let authoritative = world.get::<BroadTracks>(entity).unwrap().0.clone();
    restore_held(&mut world, &snapshot);
    assert_eq!(tracks(&world, entity), saved);
    assert_eq!(world.resource::<crate::sim_tick::SimTick>().0, 11);
    assert_eq!(world.get::<BroadTracks>(entity).unwrap().0, authoritative);
    world
        .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
        .unwrap();
    assert_eq!(tracks(&world, entity), fresh);
}

#[test]
fn missing_forged_wrong_owner_and_stale_views_keep_the_canonical_rebuild_and_cannot_route() {
    for variant in 0..5 {
        let (mut world, entity, mut snapshot) = fixture();
        match variant {
            0 => snapshot.entities[0].blackboards.clear(),
            1 => {
                tracks_mut(&mut snapshot.entities[0].blackboards[0].1).unwrap()[0].position[0] +=
                    50.0
            }
            2 => {
                tracks_mut(&mut snapshot.entities[0].blackboards[0].1).unwrap()[0].contact_id =
                    "guessed-contact".into()
            }
            3 => snapshot.entities[0].blackboards[0].0 = "tactical-radar".into(),
            _ => {
                world
                    .get_mut::<BroadTracks>(entity)
                    .unwrap()
                    .0
                    .get_mut(&owner())
                    .unwrap()
                    .current
                    .clear();
                world
                    .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
                    .unwrap();
            }
        }
        let fresh = tracks(&world, entity);
        restore_held(&mut world, &snapshot);
        assert_eq!(tracks(&world, entity), fresh, "variant {variant}");
        let config = &world.get::<ShipConfigComponent>(entity).unwrap().0;
        assert!(crate::science::contact_routing::resolve(
            config,
            world.get::<ShipSystemBlackboards>(entity).unwrap(),
            world.get::<BroadTracks>(entity),
            &SystemId("sensors".into()),
            "guessed-contact",
        )
        .is_none());
        if variant == 4 {
            assert!(fresh[0].predicted && !fresh[0].selectable);
            assert!(crate::science::contact_routing::resolve(
                config,
                world.get::<ShipSystemBlackboards>(entity).unwrap(),
                world.get::<BroadTracks>(entity),
                &SystemId("sensors".into()),
                &fresh[0].contact_id,
            )
            .is_none());
        }
    }
}
