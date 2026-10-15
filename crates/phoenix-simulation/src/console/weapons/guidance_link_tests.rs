use super::*;
use crate::core::messages::{AdmittedCommand, SystemBlackboard, TacticalRadarBlackboard};
use bevy::ecs::system::{RunSystemOnce, SystemState};
use std::collections::{BTreeMap, HashMap};

struct Fixture {
    app: App,
    source: Entity,
    tube: SystemId,
    contact: String,
    measured: [f32; 3],
}
fn fixture() -> Fixture {
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
        .find(|profile| profile.system.0 == "tactical-radar")
        .unwrap()
        .clone();
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(100));
    app.insert_resource(Time::<Fixed>::from_hz(60.0));
    app.insert_resource(crate::world::server::WorldContentRuntime::default());
    let source = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "source".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut().spawn((
        EntityUuid("private-target".into()),
        Transform::from_xyz(900.0, 900.0, 900.0),
    ));
    let mut state = crate::science::broad::BroadTrackState::new(&profile);
    state
        .advance(
            &profile,
            100,
            1.0 / 60.0,
            1.0,
            &[crate::science::broad::BroadMeasurement {
                source: "private-target".into(),
                selectable: true,
                observation: None,
                position: [20.0, -30.0],
                elevation: Some(phoenix_model::sensing_tracks::ObservedElevation {
                    height: 3.0,
                    error: profile.position_error,
                    velocity: None,
                }),
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let records = phoenix_sim_gameplay::science::sharing::projection(
        "source",
        &state,
        &profile,
        100,
        1.0 / 60.0,
    )
    .unwrap();
    let contact = records[0].contact_id.clone();
    let measured = [
        records[0].position[0],
        records[0].elevation.unwrap().height,
        records[0].position[1],
    ];
    assert!(
        records[0].selectable && records[0].elevation.is_some() && !records[0].samples.is_empty()
    );
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(source)
        .unwrap()
        .0
        .insert(
            SystemId("tactical-radar".into()),
            SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
                tracks: Some(records),
                ..Default::default()
            }),
        );
    app.world_mut()
        .entity_mut(source)
        .insert(BroadTracks(BTreeMap::from([(profile.system, state)])));
    let (tube, round) = {
        let mut owner = app
            .world_mut()
            .get_mut::<TorpedoSystemResource>(source)
            .unwrap();
        let tube = owner.0.tubes[0].id.clone();
        owner.0.set_volley_target(&tube, 1);
        assert!(owner.0.start_load(&tube));
        let load_time = owner.0.tube(&tube).unwrap().load_time;
        owner
            .0
            .tick(load_time, &HashMap::new(), &mut || unreachable!());
        assert!(matches!(
            owner.0.launch(
                &tube,
                "own-emitted-round".into(),
                0.0,
                0.0,
                -5.0,
                0.0,
                Some("original-intent".into()),
                Some("source".into())
            ),
            crate::weapons::torpedo::LaunchResult::Launched { .. }
        ));
        (
            crate::ship::system_registry::torpedo_tube_system_id(&tube).unwrap(),
            owner.0.in_flight[0].clone(),
        )
    };
    let mut knowledge = app
        .world_mut()
        .get_mut::<ShipGuidanceKnowledge>(source)
        .unwrap();
    record_emitted(
        &mut knowledge,
        &round,
        Some("original-public-intent".into()),
        Some(99),
        100,
    );
    drop(knowledge);
    Fixture {
        app,
        source,
        tube,
        contact,
        measured,
    }
}
fn send(f: &mut Fixture, payload: SystemControlPayload) {
    f.app
        .world_mut()
        .get_mut::<AdmittedCommands>(f.source)
        .unwrap()
        .0 = vec![AdmittedCommand {
        target: f.tube.clone(),
        payload,
        response_token: None,
        feedback_correlation: None,
    }];
    f.app
        .world_mut()
        .run_system_once(handle_guidance_commands)
        .unwrap();
}
fn energy(f: &Fixture) -> f32 {
    f.app
        .world()
        .get::<ShipPowerSystem>(f.source)
        .unwrap()
        .0
        .network()
        .consumers[&f.tube]
        .energy
}
fn round(f: &Fixture) -> &crate::weapons::torpedo::Torpedo {
    &f.app
        .world()
        .get::<TorpedoSystemResource>(f.source)
        .unwrap()
        .0
        .in_flight[0]
}
fn known(f: &Fixture) -> &KnownGuidedLaunch {
    &f.app
        .world()
        .get::<ShipGuidanceKnowledge>(f.source)
        .unwrap()
        .0
        .launches["own-emitted-round"]
}
fn retarget(f: &mut Fixture) {
    let target_uuid = f.contact.clone();
    send(
        f,
        SystemControlPayload::RetargetGuidedRound {
            round_uuid: "own-emitted-round".into(),
            target_uuid,
        },
    );
}

#[test]
fn paid_actual_receiver_uses_dated_xyz_and_stop_preserves_payload_endurance_and_pose() {
    let mut f = fixture();
    let before = round(&f).clone();
    let initial = energy(&f);
    retarget(&mut f);
    assert!(energy(&f) < initial, "real finite local tube work was paid");
    assert_eq!(round(&f).target_uuid.as_deref(), Some("private-target"));
    assert_eq!(
        round(&f).last_guidance.as_ref().unwrap().position,
        f.measured
    );
    assert_eq!(round(&f).last_guidance.as_ref().unwrap().sampled_tick, 100);
    assert!(known(&f).acknowledged.is_some());
    assert_eq!(known(&f).issued.target.as_deref(), Some(f.contact.as_str()));
    send(
        &mut f,
        SystemControlPayload::StopGuidedRoundGuidance {
            round_uuid: "own-emitted-round".into(),
        },
    );
    let mut expected = before;
    expected.target_uuid = None;
    expected.last_guidance = None;
    assert_eq!(round(&f), &expected);
    assert!(known(&f).issued.target.is_none());
}

#[test]
fn absent_remote_flight_and_out_of_range_link_have_identical_source_knowledge() {
    let mut gone = fixture();
    let mut distant = fixture();
    gone.app
        .world_mut()
        .get_mut::<TorpedoSystemResource>(gone.source)
        .unwrap()
        .0
        .apply_interception("own-emitted-round", f32::MAX)
        .unwrap();
    let range = round(&distant).profile.receiver.unwrap().range;
    distant
        .app
        .world_mut()
        .get_mut::<TorpedoSystemResource>(distant.source)
        .unwrap()
        .0
        .in_flight[0]
        .x = range * 2.0;
    let initial_gone = energy(&gone);
    let initial_distant = energy(&distant);
    retarget(&mut gone);
    retarget(&mut distant);
    assert_eq!(known(&gone), known(&distant));
    assert!(known(&gone).acknowledged.is_none());
    assert_eq!(
        initial_gone - energy(&gone),
        initial_distant - energy(&distant)
    );
    assert!(energy(&gone) < initial_gone);
    assert_eq!(
        round(&distant).target_uuid.as_deref(),
        Some("original-intent")
    );
}

#[test]
fn missing_height_guessed_handle_and_wrong_tube_do_not_spend_or_issue() {
    for fault in 0..3 {
        let mut f = fixture();
        let initial = energy(&f);
        if fault == 0 {
            let mut boards = f
                .app
                .world_mut()
                .get_mut::<ShipSystemBlackboards>(f.source)
                .unwrap();
            let SystemBlackboard::TacticalRadar(board) = boards
                .0
                .get_mut(&SystemId("tactical-radar".into()))
                .unwrap()
            else {
                unreachable!()
            };
            board.tracks.as_mut().unwrap()[0].elevation = None;
        } else if fault == 1 {
            f.contact = "private-target".into();
        } else {
            f.tube = crate::ship::system_registry::torpedo_tube_system_id("aft").unwrap();
        }
        let before = known(&f).clone();
        retarget(&mut f);
        assert_eq!(known(&f), &before, "fault{fault} issued an order");
        if fault != 2 {
            assert_eq!(energy(&f), initial);
        }
        assert_eq!(round(&f).target_uuid.as_deref(), Some("original-intent"));
    }
}

#[test]
fn stale_sample_or_round_concealment_cannot_refresh_or_retarget_physical_intent() {
    for concealed in [false, true] {
        let mut f = fixture();
        if concealed {
            crate::gm::operations::contact::set(
                &mut f
                    .app
                    .world_mut()
                    .resource_mut::<crate::world::server::WorldContentRuntime>()
                    .contact_overrides,
                "own-emitted-round",
                "private-target",
                crate::gm::operations::contact::ContactMode::Conceal,
            );
        } else {
            f.app
                .world_mut()
                .resource_mut::<crate::sim_tick::SimTick>()
                .0 = 1000;
        }
        retarget(&mut f);
        assert_eq!(round(&f).target_uuid.as_deref(), Some("original-intent"));
        assert!(round(&f).last_guidance.is_none());
        assert_eq!(known(&f).issued.sampled_tick, Some(100));
        assert_eq!(
            known(&f).acknowledged.is_some(),
            concealed,
            "heard receipt does not claim retarget accepted/applied"
        );
    }
}

#[test]
fn measurement_reader_refuses_conflicting_copies_and_unbound_console() {
    let mut f = fixture();
    let sample = |f: &mut Fixture, owner: &SystemId| {
        let mut state = SystemState::<GuidanceMeasurements>::new(f.app.world_mut());
        state.get(f.app.world()).sample(f.source, owner, &f.contact)
    };
    assert!(sample(&mut f, &SystemId("torpedo-tube-fore-port".into())).is_some());
    assert!(sample(&mut f, &SystemId("unbound-system".into())).is_none());
    {
        let mut boards = f
            .app
            .world_mut()
            .get_mut::<ShipSystemBlackboards>(f.source)
            .unwrap();
        let SystemBlackboard::TacticalRadar(board) = boards
            .0
            .get_mut(&SystemId("tactical-radar".into()))
            .unwrap()
        else {
            unreachable!()
        };
        let duplicate = board.tracks.as_ref().unwrap()[0].clone();
        board.tracks.as_mut().unwrap().push(duplicate);
    }
    assert!(sample(&mut f, &SystemId("torpedo-tube-fore-port".into())).is_some());
    {
        let mut boards = f
            .app
            .world_mut()
            .get_mut::<ShipSystemBlackboards>(f.source)
            .unwrap();
        let SystemBlackboard::TacticalRadar(board) = boards
            .0
            .get_mut(&SystemId("tactical-radar".into()))
            .unwrap()
        else {
            unreachable!()
        };
        board.tracks.as_mut().unwrap()[1].position[0] += 1.0;
    }
    assert!(sample(&mut f, &SystemId("torpedo-tube-fore-port".into())).is_none());
}
