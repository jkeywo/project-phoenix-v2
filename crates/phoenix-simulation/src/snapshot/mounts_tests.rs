use super::*;
use crate::weapons::mounts::{MountProfiles, ShipMounts};

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.register_component::<EntitySystemHull>();
    world.insert_resource(SimTick(20));
    world.insert_resource(Time::<Fixed>::default());
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "mounted-ship".into(),
        None,
    );
    world.flush();
    world
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntitySpawnOrigin(
            crate::world::spawn_origin::SpawnOrigin {
                template_path: "assets/entities/alliance_cruiser.toml".into(),
                name: "mounted-ship".into(),
                ..Default::default()
            },
        ));
    let profile = world.get::<MountProfiles>(entity).unwrap().clone();
    assert_eq!(profile.0.len(), 2);
    let mut states = world.get_mut::<ShipMounts>(entity).unwrap();
    let owner = SystemId("phaser-fore".into());
    states
        .0
        .get_mut(&owner)
        .unwrap()
        .advance(&profile.0[&owner], 20, 0.25, true, Some(90.0), 0.5)
        .unwrap();
    assert!(states.0[&owner].offset_deg > 0.0);
    let snapshot = capture(&world);
    (world, entity, snapshot)
}

#[test]
fn actual_spawn_json_and_application_preserve_one_bearing_owner_and_future_motion() {
    let (mut world, entity, snapshot) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    validate_roster(&world, &decoded).unwrap();
    let profile = world.get::<MountProfiles>(entity).unwrap().clone();
    let expected = decoded.entities[0].mounts.clone().unwrap();
    world.entity_mut(entity).insert(ShipMounts::new(&profile));
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &decoded.entities[0],
        decoded.tick,
    );
    assert_eq!(world.get::<ShipMounts>(entity), Some(&expected));
    assert_eq!(capture(&world).entities[0].mounts, Some(expected.clone()));
    let mut resumed = world.get::<ShipMounts>(entity).unwrap().clone();
    let mut continuous = expected;
    for tick in 21..61 {
        for (owner, config) in &profile.0 {
            let target = Some(-75.0);
            continuous
                .0
                .get_mut(owner)
                .unwrap()
                .advance(config, tick, 0.1, true, target, 0.5)
                .unwrap();
            resumed
                .0
                .get_mut(owner)
                .unwrap()
                .advance(config, tick, 0.1, true, target, 0.5)
                .unwrap();
        }
        assert_eq!(resumed, continuous);
    }
}

#[test]
fn missing_or_malformed_mounts_refuse_the_whole_candidate_before_any_write() {
    for mutation in 0..7 {
        let (mut world, entity, mut snapshot) = fixture();
        match mutation {
            0 => snapshot.entities[0].mounts = None,
            1 => {
                snapshot.entities[0]
                    .mounts
                    .as_mut()
                    .unwrap()
                    .0
                    .remove(&SystemId("phaser-fore".into()));
            }
            2 => {
                snapshot.entities[0]
                    .mounts
                    .as_mut()
                    .unwrap()
                    .0
                    .insert(SystemId("unknown".into()), Default::default());
            }
            3 => {
                snapshot.entities[0]
                    .mounts
                    .as_mut()
                    .unwrap()
                    .0
                    .get_mut(&SystemId("phaser-fore".into()))
                    .unwrap()
                    .offset_deg = 136.0
            }
            4 => {
                snapshot.entities[0]
                    .mounts
                    .as_mut()
                    .unwrap()
                    .0
                    .get_mut(&SystemId("phaser-fore".into()))
                    .unwrap()
                    .last_tick = Some(21)
            }
            5 => {
                world.entity_mut(entity).remove::<MountProfiles>();
            }
            _ => {
                snapshot.entities[0]
                    .mounts
                    .as_mut()
                    .unwrap()
                    .0
                    .get_mut(&SystemId("phaser-fore".into()))
                    .unwrap()
                    .last_tick = None
            }
        }
        world.resource_mut::<SimTick>().0 = 7;
        world.spawn((EntityUuid("surplus".into()), ShipRedAlert(false)));
        snapshot.entities[0].red_alert = Some(true);
        let before = capture(&world);
        let report = restore(&mut world, &snapshot);
        assert!(!report.is_complete(), "mutation {mutation}");
        assert_eq!(report.entities_spawned, 0);
        assert_eq!(report.despawned, 0);
        assert_eq!(capture(&world), before, "mutation {mutation}");
    }
}

#[test]
fn missing_actor_profile_comes_from_saved_source_and_orphan_states_refuse() {
    let (mut world, entity, mut snapshot) = fixture();
    assert!(snapshot.entities[0].spawn.is_some());
    world.despawn(entity);
    validate_roster(&world, &snapshot).unwrap();
    snapshot.entities[0].spawn = None;
    assert!(validate_roster(&world, &snapshot).is_err());
}
