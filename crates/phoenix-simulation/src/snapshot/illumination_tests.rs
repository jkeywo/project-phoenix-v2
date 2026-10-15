use super::*;
use phoenix_sim_gameplay::ship::composition::CommandChannelValue;
fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/illumination_cruiser.toml",
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
        "illumination-source".into(),
        None,
    );
    world.flush();
    {
        let mut state = world.get_mut::<ShipIllumination>(entity).unwrap();
        let channels = state
            .0
            .get_mut(&SystemId("science-illumination".into()))
            .unwrap();
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
    assert!(saved.entities[0].illumination.is_some());
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
    let initial = ShipIllumination::from_ship(&config.0).unwrap();
    world.entity_mut(entity).insert(initial);
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &decoded.entities[0],
        decoded.tick,
    );
    assert_eq!(
        world.get::<ShipIllumination>(entity),
        decoded.entities[0].illumination.as_ref()
    );
    assert!(
        world
            .get::<crate::console::weapons::illumination::IlluminationEmissions>(entity)
            .unwrap()
            .0
            .is_empty(),
        "restore does not invent an emission or receiver acknowledgement"
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
            row.illumination = None;
        } else {
            let state = row.illumination.as_mut().unwrap();
            match fault {
                1 => {
                    state
                        .0
                        .get_mut(&SystemId("science-illumination".into()))
                        .unwrap()
                        .0
                        .insert("enabled".into(), CommandChannelValue::EntityTarget(None));
                }
                2 => {
                    state
                        .0
                        .get_mut(&SystemId("science-illumination".into()))
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
