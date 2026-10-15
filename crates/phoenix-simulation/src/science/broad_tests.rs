use super::*;

#[test]
fn actual_paid_height_uses_physics_and_distinct_samples_without_live_vertical_velocity() {
    let (mut app, observer, target, profile) = rig();
    app.world_mut().get_mut::<ShipPhysics>(target).unwrap().y = 7.2;
    app.world_mut()
        .get_mut::<Transform>(target)
        .unwrap()
        .translation
        .y = 900.0;
    fund(&mut app, observer, &profile, 1.0);
    let paid = app
        .world()
        .get::<ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let first = state.projection(&profile, 0, 0.5).unwrap()[0].clone();
    let elevation = first.elevation.unwrap();
    assert!((elevation.height - 7.2).abs() <= elevation.error);
    assert_eq!(elevation.error, profile.position_error);
    assert_eq!(elevation.velocity, None);
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(observer)
            .unwrap()
            .0
            .capture_continuation(),
        paid
    );
    app.world_mut().get_mut::<ShipPhysics>(target).unwrap().y = 9.2;
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    assert_eq!(state.projection(&profile, 0, 0.5).unwrap()[0], first);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let next = state.projection(&profile, 1, 0.5).unwrap()[0].clone();
    let next_elevation = next.elevation.unwrap();
    assert_eq!(next.observed_tick, 1);
    assert_eq!(
        next_elevation.velocity,
        Some((next_elevation.height - elevation.height) / 0.5)
    );
    app.world_mut().get_mut::<ShipPhysics>(target).unwrap().y = 100_000.0;
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
    fund(&mut app, observer, &profile, 0.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let held = state.projection(&profile, 2, 0.5).unwrap()[0].clone();
    assert_eq!(held.observed_tick, 1);
    assert_eq!(held.elevation, next.elevation);
    assert!(held.predicted);
    // Even a newly paid sample cannot see a physically out-of-range Y.
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 3;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    assert_eq!(
        state.projection(&profile, 3, 0.5).unwrap()[0].elevation,
        next.elevation
    );
    assert!(state.current.is_empty());
}
#[test]
fn actual_in_flight_ordnance_is_measured_without_guidance_or_launcher_intelligence() {
    let (mut app, observer, target, profile) = rig();
    app.world_mut().despawn(target);
    app.world_mut()
        .get_mut::<BroadProfile>(observer)
        .unwrap()
        .0
        .get_mut(&profile.system)
        .unwrap()
        .radar
        .shows
        .push(phoenix_model::tags::EntityTag::from_str("torpedo").unwrap());
    app.world_mut()
        .get_mut::<crate::server_app::TorpedoSystemResource>(observer)
        .unwrap()
        .0
        .in_flight
        .push(crate::weapons::torpedo::Torpedo {
            previous_position: None,
            collision_interval_secs: None,
            last_guidance: None,
            profile: crate::weapons::torpedo::TorpedoConfig {
                lifespan: 100.0,
                shield_pierce: 0.5,
                ..Default::default()
            }
            .round_profile(Default::default()),
            durability_remaining: crate::weapons::torpedo::TorpedoConfig::default().durability,
            strike_damage_bonus: 0.0,
            uuid: "private-projectile-uuid".into(),
            x: 20.0,
            y: 0.0,
            z: 0.0,
            heading: 2.0,
            pitch: 0.8,
            lifespan_remaining: 100.0,
            target_uuid: Some("secret-guidance-target".into()),
            source_uuid: Some("secret-launcher".into()),
            tube_id: "secret-tube".into(),
        });
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let first = app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system]
        .projection(&profile, 0, 0.5)
        .unwrap();
    assert_eq!(first.len(), 1);
    assert!(first[0].velocity.is_none());
    assert!(!first[0].selectable);
    assert!(first[0].identity.is_none());
    let wire = serde_json::to_string(&first).unwrap();
    for forbidden in [
        "private-projectile-uuid",
        "secret-guidance-target",
        "secret-launcher",
        "secret-tube",
    ] {
        assert!(!wire.contains(forbidden));
    }
    app.world_mut()
        .get_mut::<crate::server_app::TorpedoSystemResource>(observer)
        .unwrap()
        .0
        .in_flight[0]
        .x = 100000.0;
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let held = app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system]
        .projection(&profile, 2, 0.5)
        .unwrap();
    assert_eq!(held[0].observed_tick, 0);
    assert!(held[0].predicted && held[0].position[0] < 100.0);
    // An explicit instrument grant makes actual measured ordnance targetable;
    // classification and velocity still need ordinary paid distinct samples.
    app.world_mut()
        .get_mut::<BroadProfile>(observer)
        .unwrap()
        .0
        .get_mut(&profile.system)
        .unwrap()
        .radar
        .selects
        .push(phoenix_model::tags::EntityTag::Missile);
    app.world_mut()
        .get_mut::<crate::server_app::TorpedoSystemResource>(observer)
        .unwrap()
        .0
        .in_flight[0]
        .x = 20.0;
    for now in 3..=10 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = now;
        fund(&mut app, observer, &profile, 1.0);
        app.update();
        if now == 3 {
            let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
            assert!(
                state.projection(&profile, now, 0.5).unwrap()[0]
                    .classification
                    .is_none(),
                "selection permission cannot grant instant classification"
            );
        }
    }
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let acquired = state.projection(&profile, 10, 0.5).unwrap();
    assert_eq!(acquired.len(), 1);
    assert!(acquired[0].selectable && !acquired[0].predicted);
    assert_eq!(acquired[0].classification.as_deref(), Some("missile"));
    assert!(acquired[0].velocity.is_some() && acquired[0].elevation.unwrap().velocity.is_some());
    assert!(acquired[0].identity.is_none());
    let wire = serde_json::to_string(&acquired).unwrap();
    for forbidden in [
        "private-projectile-uuid",
        "secret-guidance-target",
        "secret-launcher",
        "secret-tube",
    ] {
        assert!(!wire.contains(forbidden));
    }
    app.world_mut()
        .get_mut::<BroadProfile>(observer)
        .unwrap()
        .0
        .get_mut(&profile.system)
        .unwrap()
        .radar
        .selects
        .clear();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 11;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    assert!(!state.projection(&profile, 11, 0.5).unwrap()[0].selectable);
}
#[test]
fn streamed_rock_actor_uses_authored_tags_paid_observation_and_normal_contact_alias() {
    let (mut app, observer, target, profile) = rig();
    app.world_mut().despawn(target);
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/asteroid_common_1_small.toml",
    )
    .unwrap();
    let mut rock =
        crate::asteroids::lifecycle::rock_config("assets/entities/asteroid_common_1_small.toml");
    rock.tags = config.tags.clone();
    let target_tags = config.target.unwrap().tags;
    rock.collider = config.collider.unwrap();
    let private_id = "streamed-rock-private-route";
    let actor = app
        .world_mut()
        .spawn(crate::asteroids::lifecycle::rock_bundle(
            private_id,
            &rock,
            Vec3::new(10.0, 0.0, 0.0),
            Quat::IDENTITY,
            0.0,
            rock.max_hp,
        ))
        .id();
    assert!(app.world().get::<EntityUuid>(actor).is_none());
    let mut world = crate::lobby::server::WorldResource::default();
    world
        .0
        .entities
        .push(crate::core::messages::EntitySnapshot {
            uuid: private_id.into(),
            tags: rock.tags.clone(),
            target_tags,
            // Deliberately contradictory: measurement location comes from the actor.
            position: Some([90000.0, 0.0, 90000.0]),
            ..Default::default()
        });
    app.insert_resource(world);
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    let alias = state.contact_for_source(private_id).unwrap().to_owned();
    assert_ne!(alias, private_id);
    let measured = state.projection(&profile, 0, 0.5).unwrap();
    assert_eq!(measured.len(), 1);
    assert!(measured[0].selectable);
    assert!((measured[0].position[0] - 10.0).abs() <= profile.position_error);
    assert!(!serde_json::to_string(&measured)
        .unwrap()
        .contains(private_id));
    // Display tags cannot grant targetability. Distinct authored targeting
    // tags govern selection even when the object still displays as a rock.
    app.world_mut()
        .get_mut::<BroadProfile>(observer)
        .unwrap()
        .0
        .get_mut(&profile.system)
        .unwrap()
        .radar
        .selects = crate::entities::tags::parse_tags(&["cargo".into()]);
    app.world_mut()
        .resource_mut::<crate::lobby::server::WorldResource>()
        .0
        .entities[0]
        .target_tags = vec!["cargo".into()];
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    assert!(state.projection(&profile, 1, 0.5).unwrap()[0].selectable);
    app.world_mut()
        .resource_mut::<crate::lobby::server::WorldResource>()
        .0
        .entities[0]
        .target_tags
        .clear();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    assert!(!state.projection(&profile, 2, 0.5).unwrap()[0].selectable);
    app.world_mut()
        .get_mut::<Transform>(actor)
        .unwrap()
        .translation
        .x = 1_000_000.0;
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 3;
    fund(&mut app, observer, &profile, 1.0);
    app.update();
    let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
    assert!(state.resolve_current(&alias).is_none());
    let held = state.projection(&profile, 3, 0.5).unwrap();
    assert!(held[0].predicted && held[0].position[0] < 100.0);
}

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
    assert_eq!(ghost.elevation, None);
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
    assert_eq!(public[0].elevation, None);
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

#[test]
fn every_alliance_hull_is_an_actual_selectable_ship_and_missing_target_still_denies() {
    for hull in ["courier", "destroyer", "cruiser", "battleship"] {
        let (mut app, observer, old_target, profile) = rig();
        app.world_mut().despawn(old_target);
        let config = crate::entities::include_resolve::load_entity_config(&format!(
            "assets/entities/alliance_{hull}.toml"
        ))
        .unwrap();
        let source = "authored-ship-target";
        let target = crate::entities::spawner::spawn_entity(
            &mut app.world_mut().commands(),
            &config,
            Vec3::new(10.0, 0.0, 0.0),
            source.into(),
            None,
        );
        app.world_mut().flush();
        fund(&mut app, observer, &profile, 1.0);
        app.update();
        let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
        let alias = state
            .contact_for_source(source)
            .expect("actual paid ship target")
            .to_owned();
        let public = state.projection(&profile, 0, 0.5).unwrap();
        let measured = public
            .iter()
            .find(|record| record.contact_id == alias)
            .unwrap();
        assert!(measured.selectable && !measured.predicted, "{hull}");
        assert_ne!(alias, source);
        assert!(!serde_json::to_string(&public).unwrap().contains(source));
        // Display/physical ship identity alone cannot grant selection. The
        // actual capability is removed, not replaced by a broad fallback.
        app.world_mut()
            .entity_mut(target)
            .remove::<crate::entities::spawner::EntityTarget>();
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
        fund(&mut app, observer, &profile, 1.0);
        app.update();
        let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&profile.system];
        let public = state.projection(&profile, 1, 0.5).unwrap();
        assert!(public.iter().all(|record| !record.selectable), "{hull}");
        use bevy::ecs::system::RunSystemOnce;
        app.world_mut()
            .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
            .unwrap();
        let world = app.world();
        assert!(
            crate::science::contact_routing::resolve(
                &world
                    .get::<crate::ship_plugin::ShipConfigComponent>(observer)
                    .unwrap()
                    .0,
                world
                    .get::<crate::server_app::ShipSystemBlackboards>(observer)
                    .unwrap(),
                world.get::<BroadTracks>(observer),
                &SystemId("sensors".into()),
                &alias,
            )
            .is_none(),
            "{hull}"
        );
    }
}
