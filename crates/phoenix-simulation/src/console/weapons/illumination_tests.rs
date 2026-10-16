use super::*;
use crate::core::messages::{AdmittedCommand, SensorRadarBlackboard, SystemBlackboard};
use crate::science::broad::{BroadMeasurement, BroadTrackState, BroadTracks};
use bevy::ecs::system::RunSystemOnce;
use std::collections::HashMap;
const OWNER: &str = "science-illumination";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn fixture() -> (World, Entity, String) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/illumination_cruiser.toml",
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
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(100));
    world.insert_resource(Time::<Fixed>::from_hz(60.0));
    world.insert_resource(crate::world::server::WorldContentRuntime::default());
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "science-source".into(),
        None,
    );
    world.flush();
    let mut tracks = BroadTrackState::new(&profile);
    tracks
        .advance(
            &profile,
            100,
            1.0 / 60.0,
            1.0,
            &[BroadMeasurement {
                source: "actual-target".into(),
                selectable: true,
                observation: None,
                position: [0.0, -40.0],
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
        "science-source",
        &tracks,
        &profile,
        100,
        1.0 / 60.0,
    )
    .unwrap();
    let contact = records[0].contact_id.clone();
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            profile.system.clone(),
            SystemBlackboard::SensorRadar(SensorRadarBlackboard {
                tracks: Some(records),
                ..Default::default()
            }),
        );
    world
        .entity_mut(entity)
        .insert(BroadTracks(BTreeMap::from([(profile.system, tracks)])));
    {
        let mut provider = world
            .get_mut::<super::super::TorpedoSystemResource>(entity)
            .unwrap();
        let tube = provider.0.tubes[0].id.clone();
        provider.0.set_volley_target(&tube, 1);
        assert!(provider.0.start_load(&tube));
        let load = provider.0.tube(&tube).unwrap().load_time;
        provider
            .0
            .tick(load, &HashMap::new(), &mut || unreachable!());
        assert!(matches!(
            provider.0.launch(
                &tube,
                "illuminated-round".into(),
                0.0,
                0.0,
                -5.0,
                0.0,
                Some("actual-target".into()),
                Some("science-source".into())
            ),
            crate::weapons::torpedo::LaunchResult::Launched { .. }
        ));
        assert!(
            provider.0.in_flight[0]
                .profile
                .receiver
                .unwrap()
                .accept_external_illumination
        );
        assert!(provider.0.in_flight[0].profile.seeker.unwrap().range < 35.0);
    }
    (world, entity, contact)
}
fn command(world: &mut World, entity: Entity, payload: Control) {
    world.get_mut::<AdmittedCommands>(entity).unwrap().0 = vec![AdmittedCommand {
        target: id(OWNER),
        payload,
        response_token: None,
        feedback_correlation: None,
    }];
    world.run_system_once(handle_commands).unwrap();
    world.get_mut::<AdmittedCommands>(entity).unwrap().0.clear();
}
fn prepare(world: &mut World, entity: Entity, contact: &str) {
    command(
        world,
        entity,
        Control::SetIlluminationTarget {
            target_uuid: Some(contact.into()),
        },
    );
    command(
        world,
        entity,
        Control::SetIlluminationEnabled { enabled: true },
    );
}
fn round(world: &World, entity: Entity) -> &crate::weapons::torpedo::Torpedo {
    &world
        .get::<super::super::TorpedoSystemResource>(entity)
        .unwrap()
        .0
        .in_flight[0]
}
fn energy(world: &World, entity: Entity) -> f32 {
    world
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&id(OWNER)]
        .energy
}
#[test]
fn actual_paid_science_emission_assists_matching_opted_in_round_beyond_seeker_without_retarget() {
    let (mut world, entity, contact) = fixture();
    prepare(&mut world, entity, &contact);
    let before = round(&world, entity).clone();
    let reserve = energy(&world, entity);
    world.run_system_once(tick).unwrap();
    assert!(energy(&world, entity) < reserve);
    assert!(world
        .get::<IlluminationEmissions>(entity)
        .unwrap()
        .0
        .contains(&id(OWNER)));
    let actual = round(&world, entity);
    assert_eq!(actual.target_uuid, before.target_uuid);
    assert_eq!(actual.profile, before.profile);
    assert_eq!(
        [
            actual.x,
            actual.y,
            actual.z,
            actual.heading,
            actual.pitch,
            actual.lifespan_remaining,
            actual.durability_remaining
        ],
        [
            before.x,
            before.y,
            before.z,
            before.heading,
            before.pitch,
            before.lifespan_remaining,
            before.durability_remaining
        ]
    );
    let reading = actual.last_guidance.as_ref().unwrap();
    assert_eq!(reading.sampled_tick, 100);
    assert_eq!(reading.target_uuid, "actual-target");
    let phoenix_model::guided_guidance::GuidanceSource::Console { samples, .. } = &reading.source
    else {
        panic!("Science assistance must retain Console provenance")
    };
    assert!(!samples.is_empty());
    let held = reading.clone();
    command(
        &mut world,
        entity,
        Control::SetIlluminationEnabled { enabled: false },
    );
    let stopped = energy(&world, entity);
    world.run_system_once(tick).unwrap();
    assert_eq!(energy(&world, entity), stopped);
    assert_eq!(round(&world, entity).last_guidance.as_ref(), Some(&held));
    assert!(world
        .get::<IlluminationEmissions>(entity)
        .unwrap()
        .0
        .is_empty());
}
#[test]
fn emitter_does_not_retarget_or_reveal_receiver_presence_and_own_payment_is_ordinary() {
    for mode in 0..3 {
        let (mut world, entity, contact) = fixture();
        prepare(&mut world, entity, &contact);
        {
            let mut provider = world
                .get_mut::<super::super::TorpedoSystemResource>(entity)
                .unwrap();
            match mode {
                0 => provider.0.in_flight[0].target_uuid = Some("other-intent".into()),
                1 => {
                    provider.0.in_flight[0]
                        .profile
                        .receiver
                        .as_mut()
                        .unwrap()
                        .accept_external_illumination = false
                }
                _ => provider.0.in_flight.clear(),
            }
        }
        let before = energy(&world, entity);
        world.run_system_once(tick).unwrap();
        assert!(
            energy(&world, entity) < before,
            "source pays independently of secret remote eligibility"
        );
        assert!(world
            .get::<IlluminationEmissions>(entity)
            .unwrap()
            .0
            .contains(&id(OWNER)));
        assert!(world
            .get::<super::super::TorpedoSystemResource>(entity)
            .unwrap()
            .0
            .in_flight
            .iter()
            .all(|round| round.last_guidance.is_none()));
    }
}
#[test]
fn absent_height_unknown_handle_unready_mount_and_stale_original_evidence_cannot_emit() {
    for fault in 0..4 {
        let (mut world, entity, contact) = fixture();
        prepare(&mut world, entity, &contact);
        match fault {
            0 => {
                let mut boards = world
                    .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
                    .unwrap();
                let SystemBlackboard::SensorRadar(board) =
                    boards.0.get_mut(&id("sensor-radar")).unwrap()
                else {
                    unreachable!()
                };
                board.tracks.as_mut().unwrap()[0].elevation = None;
            }
            1 => command(
                &mut world,
                entity,
                Control::SetIlluminationTarget {
                    target_uuid: Some("guessed-private-id".into()),
                },
            ),
            2 => {
                let mut mounts = world
                    .get_mut::<crate::weapons::mounts::ShipMounts>(entity)
                    .unwrap();
                mounts.0.get_mut(&id(OWNER)).unwrap().offset_deg = 90.0;
            }
            _ => world.resource_mut::<crate::sim_tick::SimTick>().0 = 1000,
        }
        let before = energy(&world, entity);
        world.run_system_once(tick).unwrap();
        if fault == 1 {
            assert!(
                round(&world, entity).last_guidance.is_some(),
                "refused guess preserves previous permitted target"
            );
        } else if fault == 3 {
            assert!(
                round(&world, entity).last_guidance.is_none(),
                "receiver refuses original stale date"
            );
        } else {
            assert_eq!(energy(&world, entity), before);
            assert!(round(&world, entity).last_guidance.is_none());
        }
    }
}

#[test]
fn effective_offline_source_cannot_spend_or_emit_retained_enabled_intent() {
    let (mut world, entity, contact) = fixture();
    prepare(&mut world, entity, &contact);
    world
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            id(OWNER),
            crate::ship::control_source::ControlSource::Offline,
        );
    let before = energy(&world, entity);
    world.run_system_once(tick).unwrap();
    assert_eq!(energy(&world, entity), before);
    assert!(world
        .get::<IlluminationEmissions>(entity)
        .unwrap()
        .0
        .is_empty());
    assert!(round(&world, entity).last_guidance.is_none());
}
#[test]
fn receiver_original_source_concealment_withholds_new_assistance_without_cancelling_flight() {
    let (mut world, entity, contact) = fixture();
    prepare(&mut world, entity, &contact);
    world
        .get_mut::<super::super::TorpedoSystemResource>(entity)
        .unwrap()
        .0
        .in_flight[0]
        .source_uuid = Some("round-owner".into());
    crate::gm::operations::contact::set(
        &mut world
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "round-owner",
        "actual-target",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    let before = round(&world, entity).clone();
    world.run_system_once(tick).unwrap();
    assert_eq!(round(&world, entity), &before);
    assert!(
        world
            .get::<IlluminationEmissions>(entity)
            .unwrap()
            .0
            .contains(&id(OWNER)),
        "source feedback remains only its own emission"
    );
}

#[test]
fn ordinary_compiled_science_tree_reads_explicit_mission_then_uses_shared_admission_and_channels() {
    use crate::core::messages::*;
    let (mut world, entity, contact) = fixture();
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            crate::ship::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(ViewscreenBlackboard {
                scored_objectives: vec![ScoredObjective {
                    id: "support".into(),
                    score: 50.0,
                    directive: AiDirective::Illuminate {
                        target: contact.clone(),
                    },
                    source: ObjectiveSource::Mission,
                    relevance: vec![SystemAffinity::Sensors],
                    snapshot: ObjectiveSnapshot {
                        id: "support".into(),
                        text: "illumination.mission.support".into(),
                        text_params: Default::default(),
                        mandatory: false,
                        status: ObjectiveStatus::Active,
                        unassigned: false,
                        progress: None,
                        targets: vec![contact.clone()],
                        source: ObjectiveSource::Mission,
                    },
                }],
                ..Default::default()
            }),
        );
    let mut app = App::new();
    std::mem::swap(app.world_mut(), &mut world);
    // The entity fixture supplies physical owners, not App schedule resources.
    app.world_mut()
        .init_resource::<bevy::ecs::schedule::Schedules>();
    app.configure_sets(
        FixedUpdate,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
        )
            .chain(),
    );
    crate::ai::host::register_ai_host_env(&mut app);
    crate::science::reading::register(&mut app);
    register(&mut app);
    let initial = energy(app.world(), entity);
    for now in 100..=131 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = now;
        app.world_mut().run_schedule(FixedUpdate);
        if now < 130 {
            assert!(app
                .world()
                .get::<ShipIllumination>(entity)
                .unwrap()
                .target(
                    &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
                    &id(OWNER)
                )
                .is_none());
            assert_eq!(
                energy(app.world(), entity),
                initial,
                "reading alone cannot emit paid support"
            );
        }
    }
    let commands = &app.world().get::<AdmittedCommands>(entity).unwrap().0;
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command.payload, Control::SetIlluminationTarget { .. }))
            .count(),
        1
    );
    assert!(commands.iter().all(|command| command.target == id(OWNER)
        && command.response_token.as_deref() == Some("ai:science-source")));
    assert!(energy(app.world(), entity) < initial);
    assert_eq!(
        round(app.world(), entity)
            .last_guidance
            .as_ref()
            .unwrap()
            .sampled_tick,
        100
    );
    assert!(app
        .world()
        .get::<IlluminationEmissions>(entity)
        .unwrap()
        .0
        .contains(&id(OWNER)));
}

#[test]
fn consumer_payment_matches_clone_emitter_with_partial_and_receiver_refusal() {
    for (case, already_changed) in (0..6).flat_map(|case| [false, true].map(|dirty| (case, dirty)))
    {
        let (mut actual, a, target_a) = fixture();
        let (mut expected, b, target_b) = fixture();
        prepare(&mut actual, a, &target_a);
        prepare(&mut expected, b, &target_b);
        for (world, entity) in [(&mut actual, a), (&mut expected, b)] {
            let dt = world.resource::<Time<Fixed>>().timestep().as_secs_f32();
            let mut power = world.get_mut::<ShipPowerSystem>(entity).unwrap();
            let rate = power
                .0
                .config()
                .consumers
                .iter()
                .find(|consumer| consumer.system == id(OWNER))
                .unwrap()
                .continuous_cost_per_sec;
            match case {
                1 => {
                    power
                        .0
                        .network_mut()
                        .consumers
                        .get_mut(&id(OWNER))
                        .unwrap()
                        .energy = rate * dt * 0.5
                }
                2 => {
                    power
                        .0
                        .network_mut()
                        .consumers
                        .get_mut(&id(OWNER))
                        .unwrap()
                        .energy = 0.0
                }
                3 => {
                    power
                        .0
                        .network_mut()
                        .consumers
                        .get_mut(&id(OWNER))
                        .unwrap()
                        .thermal_stopped = true
                }
                4 => {
                    power.0.network_mut().consumers.remove(&id(OWNER));
                }
                _ => {}
            }
            drop(power);
            if case == 5 {
                world
                    .get_mut::<super::super::TorpedoSystemResource>(entity)
                    .unwrap()
                    .0
                    .in_flight[0]
                    .profile
                    .receiver
                    .as_mut()
                    .unwrap()
                    .accept_external_illumination = false;
            }
        }
        // Fixture setup must not hide accidental change detection on refusal.
        actual.clear_trackers();
        expected.clear_trackers();
        if already_changed {
            actual.get_mut::<ShipPowerSystem>(a).unwrap().set_changed();
            expected
                .get_mut::<ShipPowerSystem>(b)
                .unwrap()
                .set_changed();
        }
        let before = actual.get::<ShipPowerSystem>(a).unwrap().0.clone();
        actual.run_system_once(tick).unwrap();
        expected
            .run_system_once(payment_reference::tick_reference)
            .unwrap();
        let changed = actual
            .entity(a)
            .get_ref::<ShipPowerSystem>()
            .unwrap()
            .is_changed();
        let oracle_changed = expected
            .entity(b)
            .get_ref::<ShipPowerSystem>()
            .unwrap()
            .is_changed();
        assert_eq!(
            changed, oracle_changed,
            "case {case}, already changed {already_changed}"
        );
        assert_eq!(changed, already_changed || case == 0 || case == 5);
        let power = &actual.get::<ShipPowerSystem>(a).unwrap().0;
        let oracle = &expected.get::<ShipPowerSystem>(b).unwrap().0;
        assert_eq!(power, oracle, "case {case}");
        assert_eq!(
            serde_json::to_string(&power.capture_continuation()).unwrap(),
            serde_json::to_string(&oracle.capture_continuation()).unwrap()
        );
        assert_eq!(
            power.emitted_work(&id(OWNER)).to_bits(),
            oracle.emitted_work(&id(OWNER)).to_bits()
        );
        assert_eq!(
            actual.get::<IlluminationEmissions>(a).unwrap().0,
            expected.get::<IlluminationEmissions>(b).unwrap().0
        );
        assert_eq!(
            round(&actual, a).last_guidance,
            round(&expected, b).last_guidance
        );
        if (1..=4).contains(&case) {
            assert_eq!(power, &before);
        }
        if case == 5 {
            assert!(power.emitted_work(&id(OWNER)) > before.emitted_work(&id(OWNER)));
            assert!(round(&actual, a).last_guidance.is_none());
        }
    }
}
