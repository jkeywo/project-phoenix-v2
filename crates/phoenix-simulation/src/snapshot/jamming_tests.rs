use super::*;
use bevy::ecs::system::RunSystemOnce;
use phoenix_sim_gameplay::ship::composition::CommandChannelValue;
fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/jamming_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.insert_resource(SimTick(100));
    world.insert_resource(Time::<Fixed>::from_hz(60.0));
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "jamming-source".into(),
        None,
    );
    world.flush();
    {
        let mut state = world.get_mut::<ShipJamming>(entity).unwrap();
        let channels = state.0.get_mut(&SystemId("comms-jamming".into())).unwrap();
        channels
            .set("enabled", CommandChannelValue::Boolean(true))
            .unwrap();
        channels
            .set(
                "target",
                CommandChannelValue::EntityTarget(Some("previous-permitted-contact".into())),
            )
            .unwrap();
    }
    let saved = capture(&world);
    assert!(saved.entities[0].jamming.is_some());
    (world, entity, saved)
}
#[test]
fn actual_installed_channels_json_and_single_row_apply_keep_one_owner_without_receiver_history() {
    let (mut world, entity, saved) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    validate_entity(&world, entity, &decoded.entities[0]).unwrap();
    let config = world
        .get::<crate::ship_plugin::ShipConfigComponent>(entity)
        .unwrap();
    let initial = ShipJamming::from_ship(&config.0).unwrap();
    world.entity_mut(entity).insert(initial);
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &decoded.entities[0],
        decoded.tick,
    );
    assert_eq!(
        world.get::<ShipJamming>(entity),
        decoded.entities[0].jamming.as_ref()
    );
    assert!(
        world
            .get::<crate::comms::jamming::JammingEmissions>(entity)
            .unwrap()
            .is_unprocessed(),
        "restore does not invent an emission or receiver acknowledgement"
    );
    let factor = world
        .run_system_once(|paths: crate::regions::signal_path::RegionSignalQuery| {
            paths.comms_receiver_factor(Vec3::ZERO, None)
        })
        .unwrap();
    assert_eq!(
        factor, 1.0,
        "unprocessed restored intent cannot itself create interference"
    );
    let energy = world
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .capture_continuation();
    world.run_system_once(crate::comms::jamming::tick).unwrap();
    assert_eq!(
        world
            .get::<crate::ship::power::ShipPowerSystem>(entity)
            .unwrap()
            .0
            .capture_continuation(),
        energy,
        "old retained handle cannot buy work without actual current permitted evidence"
    );
}
#[test]
fn malformed_channel_owner_frontiers_refuse_before_single_row_writes() {
    let (mut world, entity, saved) = fixture();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..4 {
        let mut row = saved.entities[0].clone();
        if fault == 0 {
            row.jamming = None;
        } else {
            let state = row.jamming.as_mut().unwrap();
            match fault {
                1 => {
                    state
                        .0
                        .get_mut(&SystemId("comms-jamming".into()))
                        .unwrap()
                        .0
                        .insert("enabled".into(), CommandChannelValue::EntityTarget(None));
                }
                2 => {
                    state
                        .0
                        .get_mut(&SystemId("comms-jamming".into()))
                        .unwrap()
                        .0
                        .insert("other".into(), CommandChannelValue::Boolean(true));
                }
                _ => {
                    state
                        .0
                        .insert(SystemId("foreign-system".into()), Default::default());
                }
            }
        }
        assert!(validate_entity(&world, entity, &row).is_err());
        super::super::application::apply_entity_state(&mut world, entity, &row, saved.tick + 10);
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}

#[test]
fn replacement_recipe_cannot_borrow_live_jamming_and_invalid_recipe_never_writes() {
    let (mut world, entity, saved) = fixture();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for template in [
        "assets/entities/alliance_cruiser.toml",
        "assets/entities/missing-jammer.toml",
    ] {
        let mut row = saved.entities[0].clone();
        row.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
            template_path: template.into(),
            name: "candidate".into(),
            ..Default::default()
        });
        assert!(validate_entity(&world, entity, &row).is_err());
        super::super::application::apply_entity_state(&mut world, entity, &row, saved.tick + 1);
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}

#[test]
fn absent_runtime_source_bad_channels_refuse_before_materialization_or_clock_writes() {
    let (mut world, entity, mut saved) = fixture();
    saved.entities[0].spawn = Some(crate::world::spawn_origin::SpawnOrigin {
        template_path: "assets/entities/jamming_cruiser.toml".into(),
        name: "jamming-source".into(),
        ..Default::default()
    });
    world.despawn(entity);
    assert!(
        validate_roster(&world, &saved).is_ok(),
        "valid absent recipe remains supported"
    );
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..3 {
        let mut bad = saved.clone();
        bad.tick += 1;
        match fault {
            0 => bad.entities[0]
                .jamming
                .as_mut()
                .unwrap()
                .0
                .get_mut(&SystemId("comms-jamming".into()))
                .unwrap()
                .0
                .insert("enabled".into(), CommandChannelValue::EntityTarget(None)),
            1 => {
                bad.entities[0].jamming = None;
                None
            }
            _ => {
                bad.entities[0].spawn.as_mut().unwrap().overrides =
                    Some(toml::from_str("damage = 'malformed'").unwrap());
                None
            }
        };
        assert!(validate_roster(&world, &bad).is_err());
        let report = super::super::restore(&mut world, &bad);
        assert!(report.gaps.iter().any(|gap|matches!(gap,RestoreGap::ResourceStateInvalid{reason,..} if reason.contains("jamming"))),"{report:?}");
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}
