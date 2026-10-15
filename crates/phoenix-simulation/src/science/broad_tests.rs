use super::*;
#[test]
fn paid_sensor_tracks_include_ghosts_without_any_scene_entity_and_keep_unchanged_sample_time() {
    let (mut app, observer, _, profile) = rig();
    let observer_id = app.world().get::<EntityUuid>(observer).unwrap().0.clone();
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime
        .contact_information
        .ghosts
        .entry(observer_id.clone())
        .or_default()
        .insert(
            "explicit-point".into(),
            crate::gm::operations::information::GhostContact {
                id: "explicit-point".into(),
                palette: "SECRET PALETTE".into(),
                label: "Reported point".into(),
                position_mm: [500000, 0, 90000],
            },
        );
    app.insert_resource(runtime);
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let source = synthetic_source("explicit-point").unwrap();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let id = state.current[&source].clone();
    let first = state.tracks[&id].clone();
    let public = state.projection(&profile, 0, 0.5).unwrap();
    let ghost = public.iter().find(|track| track.contact_id == id).unwrap();
    assert_eq!(ghost.position, [500.0, 90.0]);
    assert_eq!(ghost.identity.as_ref().unwrap().name, "Reported point");
    assert!(!ghost.selectable);
    assert!(state.resolve_current(&id).is_none());
    let wire = serde_json::to_string(ghost).unwrap();
    assert!(
        !wire.contains("SECRET") && !wire.contains("explicit-point") && !wire.contains("unknown")
    );
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    fund(&mut app, observer, &profile, 0.0);
    app.update();
    assert!(
        !app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system]
            .current
            .contains_key(&source)
    );
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    assert_eq!(
        app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system].tracks[&id],
        first
    );
    {
        let mut runtime = app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>();
        let ghost = runtime
            .contact_information
            .ghosts
            .get_mut(&observer_id)
            .unwrap()
            .get_mut("explicit-point")
            .unwrap();
        ghost.label = "Changed report".into();
        ghost.position_mm[0] = 700000;
    }
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 3;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let changed = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system].tracks[&id];
    assert_eq!(changed.observed_tick, 3);
    assert_eq!(changed.position, [700.0, 90.0]);
    assert_eq!(changed.identity.as_ref().unwrap().name, "Changed report");
    assert!(changed.velocity.is_none());
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .contact_information
        .ghosts
        .clear();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 4;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    assert!(
        !app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system]
            .current
            .contains_key(&source)
    );
}
#[test]
fn absent_report_never_falls_back_to_scene_and_hide_policy_never_restores_live_identity() {
    use crate::gm::operations::information::reports::{ReportPolicy, ReportSample, ReportState};
    let (mut app, observer, target, profile) = rig();
    let observer_id = app.world().get::<EntityUuid>(observer).unwrap().0.clone();
    let target_id = app.world().get::<EntityUuid>(target).unwrap().0.clone();
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime
        .contact_information
        .reports
        .entry(observer_id.clone())
        .or_default()
        .insert(
            target_id.clone(),
            ReportState::new(ReportPolicy {
                delay_ticks: 0,
                position_step_mm: 0,
                hide_identity: true,
            }),
        );
    app.insert_resource(runtime);
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    assert!(
        app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system]
            .tracks
            .is_empty()
    );
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .contact_information
        .reports
        .get_mut(&observer_id)
        .unwrap()
        .get_mut(&target_id)
        .unwrap()
        .presented = Some(ReportSample {
        observed_tick: 1,
        position_mm: [9000, 0, 2000],
        name: "SECRET LIVE NAME".into(),
    });
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let picture = state.projection(&profile, 1, 0.5).unwrap();
    assert_eq!(picture.len(), 1);
    assert!(picture[0].identity.is_none() && !picture[0].selectable);
    assert_eq!(picture[0].position, [9.0, 2.0]);
    let public = serde_json::to_string(&picture).unwrap();
    assert!(!public.contains("SECRET") && !public.contains("hidden-until-supported"));
}
#[test]
fn classification_substitutes_only_already_detected_identity_without_granting_range() {
    let (mut app, observer, target, profile) = rig();
    let observer_id = app.world().get::<EntityUuid>(observer).unwrap().0.clone();
    let target_id = app.world().get::<EntityUuid>(target).unwrap().0.clone();
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime
        .contact_classifications
        .entry(observer_id)
        .or_default()
        .insert(
            target_id,
            crate::gm::operations::contact::ReportedClassification {
                palette: "private-palette".into(),
                label: "Reported identity".into(),
            },
        );
    app.insert_resource(runtime);
    for tick in 0..12 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        fund(&mut app, observer, &profile, 1.0);
        app.update();
    }
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let picture = state.projection(&profile, 11, 0.5).unwrap();
    assert_eq!(
        picture[0].identity.as_ref().unwrap().name,
        "Reported identity"
    );
    let public = serde_json::to_string(&picture).unwrap();
    assert!(!public.contains("private-palette") && !public.contains("hidden-until-supported"));
    app.world_mut().get_mut::<ShipPhysics>(target).unwrap().x = 10000.0;
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 12;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    assert!(
        app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system]
            .current
            .is_empty()
    );
}
#[test]
fn substituted_sensor_report_gets_own_handle_without_live_geometry_or_facts() {
    use crate::gm::operations::information::reports::{ReportPolicy, ReportSample, ReportState};
    let (mut app, observer, target, profile) = rig();
    let observer_id = app.world().get::<EntityUuid>(observer).unwrap().0.clone();
    let target_id = app.world().get::<EntityUuid>(target).unwrap().0.clone();
    app.world_mut()
        .entity_mut(target)
        .insert(crate::entities::spawner::EntityTarget(
            crate::entities::target::TargetSection {
                tags: vec!["ship".into()],
                threat_level: Default::default(),
                description: None,
            },
        ));
    // The actual actor is now out of reach. Only an already-permitted delayed
    // sample may appear; its reported position/name are deliberately different.
    app.world_mut().get_mut::<ShipPhysics>(target).unwrap().x = 10000.0;
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    let mut report = ReportState::new(ReportPolicy {
        delay_ticks: 2,
        position_step_mm: 10000,
        hide_identity: false,
    });
    report.next_tick = Some(6);
    report.presented = Some(ReportSample {
        observed_tick: 2,
        position_mm: [123000, 0, 456000],
        name: "Reported vessel".into(),
    });
    runtime
        .contact_information
        .reports
        .entry(observer_id.clone())
        .or_default()
        .insert(target_id.clone(), report);
    app.insert_resource(runtime);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 4;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let public = state.projection(&profile, 4, 0.5).unwrap();
    assert_eq!(public.len(), 1);
    assert_eq!(public[0].position, [123.0, 456.0]);
    assert_eq!(public[0].observed_tick, 2);
    assert_eq!(public[0].identity.as_ref().unwrap().name, "Reported vessel");
    assert!(public[0].identity.as_ref().unwrap().faction.is_none());
    assert!(public[0].assessment.is_none() && public[0].classification.is_none());
    assert!(public[0].selectable);
    assert_ne!(public[0].contact_id, target_id);
    assert_eq!(
        state.resolve_current(&public[0].contact_id),
        Some(target_id.as_str())
    );
    assert!(!serde_json::to_string(&public).unwrap().contains(&target_id));
    // A second independent instrument never gains the Sensors report.
    let tactical = app.world().get::<BroadProfile>(observer).unwrap().0
        [&SystemId("tactical-radar".into())]
        .profile
        .clone();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 5;
    fund(&mut app, observer, &profile, 1.0);
    fund(&mut app, observer, &tactical, 1.0);
    app.update();
    assert!(
        app.world().get::<BroadTracks>(observer).unwrap().0[&tactical.system]
            .tracks
            .is_empty()
    );
    let after = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    assert_eq!(after.tracks.values().next().unwrap().observed_tick, 2);
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        &observer_id,
        &target_id,
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 6;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    assert!(
        app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system]
            .current
            .is_empty()
    );
}
#[test]
fn routine_reference_correlation_requires_a_unique_display_label() {
    let record = crate::world::config::WorldEntity {
        name: Some("objective-ref".into()),
        display_name: Some("Courier".into()),
        ..Default::default()
    };
    assert_eq!(
        routine_references("Courier", &[record.clone()], ["objective-ref"].into_iter()),
        vec!["objective-ref"]
    );
    let unrelated = crate::world::config::WorldEntity {
        name: Some("other-ref".into()),
        display_name: Some("Courier".into()),
        ..Default::default()
    };
    assert!(routine_references(
        "Courier",
        &[record, unrelated],
        ["objective-ref"].into_iter()
    )
    .is_empty());
}
#[test]
fn two_actual_radars_keep_paid_work_and_routing_separate_on_one_ship() {
    let (mut app, observer, _, sensor) = rig();
    let tactical = app.world().get::<BroadProfile>(observer).unwrap().0
        [&phoenix_model::identity::SystemId("tactical-radar".into())]
        .profile
        .clone();
    fund(&mut app, observer, &sensor, 1.0);
    fund(&mut app, observer, &tactical, 0.0);
    let before = app
        .world()
        .get::<ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    app.update();
    let states = &app.world().get::<BroadTracks>(observer).unwrap().0;
    assert_eq!(states[&sensor.system].tracks.len(), 1);
    assert!(states[&tactical.system].tracks.is_empty());
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(observer)
            .unwrap()
            .0
            .capture_continuation(),
        before
    );
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    fund(&mut app, observer, &sensor, 0.0);
    fund(&mut app, observer, &tactical, 1.0);
    app.update();
    let states = &app.world().get::<BroadTracks>(observer).unwrap().0;
    let sensor_id = states[&sensor.system].tracks.keys().next().unwrap();
    let tactical_id = states[&tactical.system].tracks.keys().next().unwrap();
    assert_ne!(sensor_id, tactical_id);
    assert!(states[&sensor.system]
        .resolve_current(tactical_id)
        .is_none());
    assert!(states[&sensor.system].current.is_empty());
    assert_eq!(states[&tactical.system].current.len(), 1);
}
fn rig() -> (App, Entity, Entity, BroadConfig) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profile = config
        .sensing
        .as_ref()
        .unwrap()
        .broad
        .iter()
        .find(|p| p.system.0 == "sensor-radar")
        .unwrap()
        .clone();
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
    let tracks = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let first = tracks.projection(&profile, 0, 0.5).unwrap();
    assert_eq!(first.len(), 1);
    assert!(first[0].identity.is_none() && first[0].velocity.is_none());
    app.world_mut().get_mut::<ShipPhysics>(target).unwrap().x = 12.0;
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let measured = app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system]
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
    let tracks = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
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
    assert!(
        app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system]
            .tracks
            .is_empty()
    );
}
