use super::*;
use crate::core::messages::{AdmittedCommand, EntitySnapshot, WorldData};
fn owner() -> SystemId {
    SystemId("navigation".into())
}
fn rig() -> (App, Entity, Entity) {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    config.navigation_console.as_mut().unwrap().sharing =
        Some(phoenix_model::sensing_sharing::RadarSharingConfig {
            system: owner(),
            share_own_faction: false,
            receive: true,
        });
    validate_config(&config).unwrap();
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(0))
        .insert_resource(Time::<Fixed>::from_seconds(0.5));
    app.insert_resource(crate::lobby::WorldResource(WorldData {
        entities: vec![
            EntitySnapshot {
                uuid: "public-planet".into(),
                name: Some("planet.label".into()),
                tags: vec!["charted".into(), "planet".into()],
                position: Some([40.0, 0.0, 0.0]),
                ..Default::default()
            },
            EntitySnapshot {
                uuid: "private-actor".into(),
                name: Some("SECRET".into()),
                tags: vec!["planet".into()],
                position: Some([41.0, 0.0, 0.0]),
                ..Default::default()
            },
        ],
        ..Default::default()
    }));
    let mut actors = Vec::new();
    for (id, x) in [
        ("00000000-0000-0000-0000-000000000161", 0.0),
        ("00000000-0000-0000-0000-000000000162", 20.0),
    ] {
        let entity = crate::entities::spawner::spawn_entity(
            &mut app.world_mut().commands(),
            &config,
            Vec3::new(x, 0.0, 0.0),
            id.into(),
            None,
        );
        install(&config, &mut app.world_mut().commands().entity(entity));
        actors.push(entity);
    }
    app.world_mut().flush();
    for entity in &actors {
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(*entity)
            .unwrap()
            .0
            .set(owner(), crate::ship::control_source::ControlSource::Human);
        for radio in config
            .ship_config
            .as_ref()
            .unwrap()
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::COMMS_KIND)
        {
            app.world_mut()
                .get_mut::<ShipSystemControlSources>(*entity)
                .unwrap()
                .0
                .set(
                    radio.id.clone(),
                    crate::ship::control_source::ControlSource::Human,
                );
        }
    }
    app.add_systems(
        Update,
        (
            super::super::server::handle_navigation_waypoint,
            apply_commands,
            tick,
        )
            .chain(),
    );
    (app, actors[0], actors[1])
}
fn order(app: &mut App, entity: Entity, payload: SystemControlPayload) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: owner(),
            payload,
            response_token: None,
            feedback_correlation: None,
        });
}
fn step(app: &mut App) {
    app.update();
    let mut commands = app.world_mut().query::<&mut AdmittedCommands>();
    for mut commands in commands.iter_mut(app.world_mut()) {
        commands.0.clear();
    }
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
}
fn received(app: &App, entity: Entity) -> Vec<NavigationSharedObservation> {
    app.world()
        .get::<NavigationSharing>(entity)
        .unwrap()
        .0
        .projection()
}

#[test]
fn actual_radio_offline_and_jamming_pause_delivery_without_renewing_sample_time() {
    let (mut app, sender, receiver) = rig();
    let radio = app
        .world()
        .get::<ShipConfigComponent>(sender)
        .unwrap()
        .0
        .systems
        .iter()
        .find(|system| system.kind == crate::ship::system_registry::COMMS_KIND)
        .unwrap()
        .id
        .clone();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(sender)
        .unwrap()
        .0
        .set_offline(radio.clone(), true);
    order(
        &mut app,
        sender,
        SystemControlPayload::SetRadarSharing {
            share_own_faction: true,
            receive: true,
        },
    );
    order(
        &mut app,
        sender,
        SystemControlPayload::SetNavigationWaypoint {
            x: 5.0,
            z: 6.0,
            source_uuid: None,
        },
    );
    step(&mut app);
    assert!(received(&app, receiver).is_empty());
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(sender)
        .unwrap()
        .0
        .set_offline(radio, false);
    step(&mut app);
    let original = received(&app, receiver);
    assert!(
        original
            .iter()
            .any(|sample| sample.kind == NavigationSharedKind::Waypoint
                && sample.provenance.observed_tick == 0)
    );
    let source = crate::core::messages::ModifierSource::RegionEffect {
        uuid: uuid::Uuid::from_u128(77),
    };
    app.world_mut()
        .get_mut::<crate::modifiers::ShipModifiers>(receiver)
        .unwrap()
        .add_flag(source.clone(), crate::core::messages::FlagKind::CommsJammed);
    order(
        &mut app,
        sender,
        SystemControlPayload::SetNavigationWaypoint {
            x: 15.0,
            z: 16.0,
            source_uuid: None,
        },
    );
    step(&mut app);
    assert_eq!(received(&app, receiver), original);
    app.world_mut()
        .get_mut::<crate::modifiers::ShipModifiers>(receiver)
        .unwrap()
        .remove_flag(source, crate::core::messages::FlagKind::CommsJammed);
    step(&mut app);
    assert!(
        received(&app, receiver)
            .iter()
            .any(|sample| sample.kind == NavigationSharedKind::Waypoint
                && sample.position == [15.0, 16.0]
                && sample.provenance.observed_tick == 2)
    );
}

#[test]
fn normal_paid_waypoint_and_explicit_sharing_never_replace_recipient_course_or_control() {
    for source in [
        crate::ship::control_source::ControlSource::Human,
        crate::ship::control_source::ControlSource::Ai,
    ] {
        let (mut app, sender, receiver) = rig();
        order(
            &mut app,
            sender,
            SystemControlPayload::SetNavigationWaypoint {
                x: 50.0,
                z: 60.0,
                source_uuid: None,
            },
        );
        order(
            &mut app,
            receiver,
            SystemControlPayload::SetNavigationWaypoint {
                x: 1.0,
                z: 2.0,
                source_uuid: None,
            },
        );
        let before = app
            .world()
            .get::<crate::ship::power::ShipPowerSystem>(sender)
            .unwrap()
            .0
            .capture_continuation()
            .network
            .consumers[&owner()]
            .energy;
        step(&mut app);
        assert!(received(&app, receiver).is_empty());
        assert_eq!(
            app.world()
                .get::<crate::ship::power::ShipPowerSystem>(sender)
                .unwrap()
                .0
                .capture_continuation()
                .network
                .consumers[&owner()]
                .energy,
            before - 1.0
        );
        order(
            &mut app,
            sender,
            SystemControlPayload::SetRadarSharing {
                share_own_faction: true,
                receive: true,
            },
        );
        step(&mut app);
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(sender)
            .unwrap()
            .0
            .set(owner(), source);
        let course = app
            .world()
            .get::<NavigationWaypoint>(receiver)
            .unwrap()
            .clone();
        let first = received(&app, receiver);
        assert!(
            first
                .iter()
                .any(|sample| sample.kind == NavigationSharedKind::Waypoint
                    && sample.position == [50.0, 60.0])
        );
        step(&mut app);
        assert_eq!(received(&app, receiver), first);
        assert_eq!(
            app.world().get::<NavigationWaypoint>(receiver).unwrap(),
            &course
        );
        assert_eq!(
            app.world()
                .get::<ShipSystemControlSources>(receiver)
                .unwrap()
                .0
                .source_for(&owner()),
            crate::ship::control_source::ControlSource::Human
        );
    }
}
#[test]
fn only_explicit_public_chart_and_permitted_anchor_geometry_enter_the_shared_view() {
    let (mut app, sender, receiver) = rig();
    order(
        &mut app,
        sender,
        SystemControlPayload::SetRadarSharing {
            share_own_faction: true,
            receive: true,
        },
    );
    order(
        &mut app,
        sender,
        SystemControlPayload::SetNavigationWaypoint {
            x: 999.0,
            z: 999.0,
            source_uuid: Some("private-actor".into()),
        },
    );
    step(&mut app);
    let first = received(&app, receiver);
    assert_eq!(first.len(), 1);
    assert!(matches!(&first[0].kind,NavigationSharedKind::Landmark {id,..} if id=="public-planet"));
    assert!(!serde_json::to_string(&first).unwrap().contains("SECRET"));
    assert!(
        !serde_json::to_string(&first)
            .unwrap()
            .contains("private-actor")
    );
    order(
        &mut app,
        sender,
        SystemControlPayload::SetNavigationWaypoint {
            x: 666.0,
            z: 666.0,
            source_uuid: Some("public-planet".into()),
        },
    );
    step(&mut app);
    assert!(
        received(&app, receiver)
            .iter()
            .any(|sample| sample.kind == NavigationSharedKind::Waypoint
                && sample.position == [40.0, 0.0])
    );
}
#[test]
fn actual_link_loss_retains_fixed_history_without_refresh_and_json_keeps_receipt_frontier() {
    let (mut app, sender, receiver) = rig();
    order(
        &mut app,
        sender,
        SystemControlPayload::SetRadarSharing {
            share_own_faction: true,
            receive: true,
        },
    );
    step(&mut app);
    let before = received(&app, receiver);
    assert!(!before.is_empty());
    let barrier = app
        .world_mut()
        .spawn((
            EntityUuid("comms-obstruction".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            crate::entities::spawner::RegionShapeSection(
                crate::regions::shape::RegionShape::Sphere { radius: 3.0 },
            ),
            phoenix_model::field::SignalMaterialConfig {
                sensor_absorption_per_unit: 0.0,
                comms_absorption_per_unit: 0.0,
                sensor_opaque: false,
                comms_opaque: true,
            },
        ))
        .id();
    step(&mut app);
    assert_eq!(received(&app, receiver), before);
    let state = app
        .world()
        .get::<NavigationSharing>(receiver)
        .unwrap()
        .0
        .clone();
    let restored: NavigationSharingState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(state, restored);
    app.world_mut().despawn(barrier);
    step(&mut app);
    assert_eq!(received(&app, receiver), before);
    app.world_mut()
        .get_mut::<FactionComponent>(receiver)
        .unwrap()
        .0 = uuid::Uuid::from_u128(42);
    step(&mut app);
    assert_eq!(received(&app, receiver), before);
}
#[test]
fn recipient_conceal_and_reception_intent_filter_new_points_without_erasing_held_history() {
    let (mut app, sender, receiver) = rig();
    let receiver_id = app.world().get::<EntityUuid>(receiver).unwrap().0.clone();
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    crate::gm::operations::contact::set(
        &mut runtime.contact_overrides,
        &receiver_id,
        "public-planet",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    app.insert_resource(runtime);
    order(
        &mut app,
        sender,
        SystemControlPayload::SetRadarSharing {
            share_own_faction: true,
            receive: true,
        },
    );
    step(&mut app);
    assert!(received(&app, receiver).is_empty());
    order(
        &mut app,
        receiver,
        SystemControlPayload::SetRadarSharing {
            share_own_faction: false,
            receive: false,
        },
    );
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .contact_overrides
        .clear();
    step(&mut app);
    assert!(received(&app, receiver).is_empty());
    order(
        &mut app,
        receiver,
        SystemControlPayload::SetRadarSharing {
            share_own_faction: false,
            receive: true,
        },
    );
    step(&mut app);
    assert_eq!(received(&app, receiver).len(), 1);
}
