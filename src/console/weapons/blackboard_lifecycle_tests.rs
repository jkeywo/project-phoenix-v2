use super::*;

#[test]
fn tactical_radar_projection_is_independent_of_snapshot_and_ecs_storage_order() {
    use crate::entities::spawner::EntityUuid;
    use bevy::ecs::system::RunSystemOnce;

    fn observe(reverse: bool) -> SystemBlackboard {
        let mut app = App::new();
        let mut entities: Vec<_> = ["contact-a", "contact-b"]
            .into_iter()
            .map(|id| crate::core::messages::EntitySnapshot {
                uuid: id.into(),
                tags: vec!["ship".into()],
                shape: Some("sphere".into()),
                radius: Some(if id == "contact-a" { 10.0 } else { 20.0 }),
                ..Default::default()
            })
            .collect();
        if reverse {
            entities.reverse();
        }
        for entity in &entities {
            app.world_mut().spawn((
                EntityUuid(entity.uuid.clone()),
                Transform::from_xyz(20.0, 0.0, 0.0),
            ));
        }
        app.insert_resource(WorldResource(crate::core::messages::WorldData {
            entities,
            ..Default::default()
        }))
        .insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::core::messages::ShipClientConfig {
                tactical_radar_range: 100.0,
                tactical_radar_shows: vec!["ship".into()],
                ..Default::default()
            },
        ));
        let viewer = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::server_app::LocalShip,
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();
        app.world_mut()
            .run_system_once(publish_tactical_radar_blackboard)
            .unwrap();
        let board = app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(viewer)
            .unwrap()
            .0[&crate::ship::system_registry::tactical_radar_system_id()]
            .clone();
        let SystemBlackboard::TacticalRadar(radar) = &board else {
            panic!("actual Tactical radar publication")
        };
        assert_eq!(radar.blips.len(), 2);
        assert_eq!(radar.regions.len(), 2);
        board
    }

    assert_eq!(observe(false), observe(true));
}

#[test]
fn tactical_objective_annotations_and_regions_use_the_observing_ship_scope() {
    use crate::entities::spawner::EntityUuid;
    use bevy::ecs::system::RunSystemOnce;
    let mut app = App::new();
    let entities = vec![
        crate::core::messages::EntitySnapshot {
            uuid: "marker".into(),
            id: Some("marker".into()),
            tags: vec!["objective_marker".into()],
            shape: Some("sphere".into()),
            radius: Some(10.0),
            radar_icon: Some("waypoint".into()),
            objective_target: true,
            ..Default::default()
        },
        crate::core::messages::EntitySnapshot {
            uuid: "contact".into(),
            id: Some("contact".into()),
            tags: vec!["ship".into()],
            radar_icon: Some("ship".into()),
            objective_target: true,
            ..Default::default()
        },
    ];
    app.insert_resource(WorldResource(crate::core::messages::WorldData {
        entities,
        ..Default::default()
    }))
    .insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            tactical_radar_range: 100.0,
            tactical_radar_shows: vec!["ship".into(), "objective_marker".into()],
            ..Default::default()
        },
    ))
    .init_resource::<crate::world::server::ObjectiveManagerRes>();
    for id in ["marker", "contact"] {
        app.world_mut()
            .spawn((EntityUuid(id.into()), Transform::from_xyz(20.0, 0.0, 0.0)));
    }
    let viewer = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            EntityUuid("ship-b".into()),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();
    {
        let manager = &mut app
            .world_mut()
            .resource_mut::<crate::world::server::ObjectiveManagerRes>()
            .0;
        manager.add(
            "private",
            "objective.test",
            true,
            vec!["marker".into(), "contact".into()],
        );
        manager.set_recipients("private", vec!["ship-a".into()]);
    }
    for (ship, expected_blips, expected_regions) in [("ship-b", 1, 0), ("ship-a", 2, 1)] {
        app.world_mut().get_mut::<EntityUuid>(viewer).unwrap().0 = ship.into();
        app.world_mut()
            .run_system_once(publish_tactical_radar_blackboard)
            .unwrap();
        let boards = app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(viewer)
            .unwrap();
        let SystemBlackboard::TacticalRadar(radar) =
            &boards.0[&crate::ship::system_registry::tactical_radar_system_id()]
        else {
            panic!("radar publication")
        };
        assert_eq!(radar.blips.len(), expected_blips);
        assert_eq!(radar.regions.len(), expected_regions);
        assert!(radar
            .blips
            .iter()
            .all(|blip| blip.objective_target == (ship == "ship-a")));
    }
    assert!(app
        .world()
        .resource::<WorldResource>()
        .0
        .entities
        .iter()
        .all(|entity| entity.objective_target));
}

#[test]
fn registered_reset_rearms_initial_default_projection_for_a_second_run() {
    let mut app = App::new();
    register_weapons_replication_lifecycle(&mut app);
    let default_projection = LastWeaponsUpdate::default();

    *app.world_mut().resource_mut::<WeaponsUpdateFirstTick>() = WeaponsUpdateFirstTick(false);
    assert!(
        publish_weapons_update_if_changed(app.world_mut(), default_projection.clone()).is_empty()
    );

    crate::core::broadcast::reset_registered_replication(app.world_mut());

    assert!(app.world().resource::<WeaponsUpdateFirstTick>().0);
    assert_eq!(
        publish_weapons_update_if_changed(app.world_mut(), default_projection.clone()),
        vec![weapons_update_message(default_projection)]
    );
    assert!(!app.world().resource::<WeaponsUpdateFirstTick>().0);
}
