use super::*;
fn rig() -> (App, Entity, Entity, BroadConfig) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profile = config.sensing.as_ref().unwrap().broad.clone().unwrap();
    let mut app = App::new();
    app.insert_resource(Time::<Fixed>::from_seconds(0.5))
        .insert_resource(crate::sim_tick::SimTick(0))
        .add_systems(Update, tick);
    let observer = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "00000000-0000-0000-0000-000000000141".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut().entity_mut(observer).insert((
        BroadProfile(profile.clone()),
        BroadTracks(BroadTrackState::new(&profile)),
    ));
    let target = app
        .world_mut()
        .spawn((
            EntityUuid("00000000-0000-0000-0000-000000000142".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            EntityName("hidden-until-supported".into()),
            EntityTagsSection(vec!["ship".into()]),
            ShipPhysics {
                x: 10.0,
                forward_speed: 1000.0,
                ..Default::default()
            },
        ))
        .id();
    (app, observer, target, profile)
}
fn fund(app: &mut App, observer: Entity, profile: &BroadConfig, activity: f32) {
    app.world_mut()
        .get_mut::<ShipPowerSystem>(observer)
        .unwrap()
        .0
        .work_continuous(&profile.system, 0.5, activity)
        .unwrap();
}
#[test]
fn actual_broad_work_reuses_exact_paid_reserve_grant_and_no_live_velocity() {
    let (mut app, observer, target, profile) = rig();
    fund(&mut app, observer, &profile, 1.0);
    let paid = app
        .world()
        .get::<ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    app.update();
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(observer)
            .unwrap()
            .0
            .capture_continuation(),
        paid,
        "producer cannot charge twice"
    );
    let tracks = &app.world().get::<BroadTracks>(observer).unwrap().0;
    let first = tracks.projection(&profile, 0, 0.5).unwrap();
    assert_eq!(first.len(), 1);
    assert!(first[0].identity.is_none() && first[0].velocity.is_none());
    app.world_mut().get_mut::<ShipPhysics>(target).unwrap().x = 12.0;
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let measured = app
        .world()
        .get::<BroadTracks>(observer)
        .unwrap()
        .0
        .projection(&profile, 1, 0.5)
        .unwrap()[0]
        .clone();
    assert!(
        measured.velocity.unwrap()[0].abs() < 20.0,
        "true forward speed1000 is never a measurement"
    );
    app.world_mut().get_mut::<ShipPhysics>(target).unwrap().x = 1000.0;
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
    fund(&mut app, observer, &profile, 0.0);
    app.update();
    let tracks = &app.world().get::<BroadTracks>(observer).unwrap().0;
    let lost = tracks.projection(&profile, 2, 0.5).unwrap();
    assert!(lost[0].predicted && lost[0].position_error > measured.position_error);
    assert!(tracks.resolve_current(&measured.contact_id).is_none());
}
#[test]
fn actual_opaque_signal_path_prevents_return_despite_healthy_paid_receiver() {
    let (mut app, observer, _, profile) = rig();
    app.world_mut().spawn((
        EntityUuid("opaque-volume".into()),
        Transform::from_xyz(5.0, 0.0, 0.0),
        crate::entities::spawner::RegionShapeSection(crate::regions::shape::RegionShape::Sphere {
            radius: 1.0,
        }),
        phoenix_model::field::SignalMaterialConfig {
            sensor_absorption_per_unit: 0.0,
            comms_absorption_per_unit: 0.0,
            sensor_opaque: true,
            comms_opaque: false,
        },
    ));
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    assert!(app
        .world()
        .get::<BroadTracks>(observer)
        .unwrap()
        .0
        .tracks
        .is_empty());
}
