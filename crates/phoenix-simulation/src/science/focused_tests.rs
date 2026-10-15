use crate::core::messages::{AdmittedCommand, AdmittedCommands, SystemControlPayload, SystemId};
use crate::science::server::{publish_scan_blackboard, tick_scans};
use crate::science::server::{ScanSaveState, ShipScanRecord};
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;

fn rig() -> (App, Entity) {
    rig_with(false)
}
fn rig_with(dual: bool) -> (App, Entity) {
    rig_with_subject(dual, None)
}
fn rig_with_subject(
    dual: bool,
    subject: Option<&crate::entities::config::EntityConfig>,
) -> (App, Entity) {
    rig_from("assets/entities/alliance_cruiser.toml", dual, subject)
}
fn rig_from(
    path: &str,
    dual: bool,
    subject: Option<&crate::entities::config::EntityConfig>,
) -> (App, Entity) {
    let mut config = crate::entities::include_resolve::load_entity_config(path).unwrap();
    if dual {
        crate::science::test_support::clone_instrument(&mut config, "sensors", "focused-two");
    }
    let mut app = App::new();
    app.insert_resource(Time::<()>::default())
        .insert_resource(Time::<Fixed>::from_seconds(0.1))
        .insert_resource(crate::sim_tick::SimTick(0))
        .add_message::<crate::lobby::OutboundMessage>()
        .add_systems(Update, (tick_scans, publish_scan_blackboard).chain());
    let observer = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "00000000-0000-0000-0000-000000000141".into(),
        None,
    );
    app.world_mut().flush();
    let infrastructure = crate::infrastructure::InfrastructureState::from_config(
        &crate::infrastructure::InfrastructureConfig {
            condition_max: 100.0,
            condition: Some(75.0),
            publish: true,
            ..Default::default()
        },
    );
    if let Some(subject) = subject {
        crate::entities::spawner::spawn_entity(
            &mut app.world_mut().commands(),
            subject,
            Vec3::new(4.0, 0.0, 0.0),
            "00000000-0000-0000-0000-000000000142".into(),
            None,
        );
        app.world_mut().flush();
    } else {
        app.world_mut().spawn((
            crate::entities::spawner::EntityUuid("00000000-0000-0000-0000-000000000142".into()),
            Transform::from_xyz(4.0, 0.0, 0.0),
            crate::entities::spawner::EntityTagsSection(vec!["station".into()]),
            crate::entities::spawner::EntityTarget(crate::entities::target::TargetSection {
                tags: vec!["station".into()],
                threat_level: Default::default(),
                description: None,
            }),
            crate::infrastructure::InfrastructureCondition(infrastructure),
        ));
    }
    // Establish one real paid observation before isolating focused work. No
    // subsequent focused step may spend or replenish the independent radar.
    app.world_mut()
        .get_mut::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .work_continuous(&SystemId("sensor-radar".into()), 0.1, 1.0)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::science::broad::tick)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
        .unwrap();
    (app, observer)
}

fn enquiry_rig() -> (App, Entity) {
    let subject = crate::entities::include_resolve::load_entity_config(
        "tests/fixtures/sensing-enquiry-subject.toml",
    )
    .unwrap();
    rig_from(
        "tests/fixtures/sensing-enquiry-cruiser.toml",
        false,
        Some(&subject),
    )
}
fn question(
    app: &mut App,
    observer: Entity,
    aim: phoenix_model::focused_enquiry::FocusedAim,
    intrusive: bool,
) {
    request(app, observer);
    let mut admitted = app
        .world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap();
    let SystemControlPayload::ScanTarget {
        aim: asked,
        intrusive: mode,
        ..
    } = &mut admitted.0.last_mut().unwrap().payload
    else {
        unreachable!()
    };
    *asked = aim;
    *mode = intrusive;
}

#[test]
fn different_questions_share_one_paid_slot_and_resume_only_their_own_retained_progress() {
    use phoenix_model::focused_enquiry::FocusedAim;
    let (mut app, observer) = enquiry_rig();
    question(&mut app, observer, FocusedAim::Environment, false);
    step(&mut app);
    let owner = SystemId("sensors".into());
    let first = app.world().get::<ShipScanRecord>(observer).unwrap().0[&owner]
        .focused
        .clone()
        .unwrap();
    assert!(first.progress > 0.0 && first.paid_started_tick.is_some());
    question(&mut app, observer, FocusedAim::Resources, true);
    step(&mut app);
    let record = &app.world().get::<ShipScanRecord>(observer).unwrap().0[&owner];
    assert_eq!(record.focused.as_ref().unwrap().aim, FocusedAim::Resources);
    assert_eq!(record.unfinished.len(), 1);
    assert_eq!(record.unfinished[&first.key()].progress, first.progress);
    assert_eq!(
        record.focused.as_ref().unwrap().progress,
        first.progress,
        "another question starts its own first paid slice"
    );
    question(&mut app, observer, FocusedAim::Environment, false);
    step(&mut app);
    let record = &app.world().get::<ShipScanRecord>(observer).unwrap().0[&owner];
    assert_eq!(
        record.focused.as_ref().unwrap().progress,
        first.progress + 0.1
    );
    assert_eq!(record.unfinished.len(), 1);
    assert!(record
        .unfinished
        .values()
        .all(|work| work.aim == FocusedAim::Resources && work.intrusive));
}

#[test]
fn private_and_absent_answers_share_refusal_and_cannot_spend_or_generate_a_reading() {
    use phoenix_model::focused_enquiry::FocusedAim;
    let (mut app, observer) = enquiry_rig();
    let initial = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    for aim in [FocusedAim::General, FocusedAim::Resources] {
        question(&mut app, observer, aim, false);
        step(&mut app);
        let record =
            &app.world().get::<ShipScanRecord>(observer).unwrap().0[&SystemId("sensors".into())];
        assert_eq!(
            record.refusal,
            Some(crate::science::ScanRefusal::NoReadableCondition)
        );
        assert!(record.history.is_empty() && record.focused.is_none());
    }
    assert_eq!(
        app.world()
            .get::<crate::ship::power::ShipPowerSystem>(observer)
            .unwrap()
            .0
            .capture_continuation(),
        initial
    );
}

// Restore's run owner replaces these resources even for empty state. The bare
// scanner fixture supplies the same production run frontier before both boots;
// absence must not be mistaken for the authoritative default in digest checks.
fn prepare_capture_fixture(app: &mut App) {
    app.init_resource::<crate::gm::actions::action::SimulationPaused>()
        .init_resource::<crate::gm::actions::action::GmActionJournal>()
        .init_resource::<crate::gm::operations::puppet::StationPuppets>()
        .init_resource::<crate::gm::operations::puppet::PendingGmStationCommands>()
        .init_resource::<crate::core::report::MissionReport>()
        .add_systems(
            Update,
            crate::regions::server::rebuild_region_read_cache.before(tick_scans),
        )
        .add_systems(
            Update,
            (
                crate::science::passive::publish,
                crate::ship::sensors::publish_sensor_radar_blackboard,
            )
                .after(publish_scan_blackboard),
        );
    let world = app.world_mut();
    // Production installs these optional legacy row types. Registering a type
    // creates no actor state: the existing capture gate can now see the actual
    // physical/repair/scanner owners already spawned by the accepted hull.
    world.register_component::<crate::ship::state::ShipPhysics>();
    world.register_component::<crate::entities::spawner::EntitySystemHull>();
    world.register_component::<crate::ship::state::ShipRedAlert>();
    world.register_component::<crate::console::command::server::ShipStationStances>();
    world.register_component::<crate::ship_plugin::ShipSystemControlSources>();
    world.register_component::<crate::server_app::AsteroidUuid>();
    world.register_component::<crate::console::weapons::beam::ActiveBeam>();
    world.register_component::<crate::console::weapons::beam::PhaserCooldown>();
    world.register_component::<crate::console::weapons::torpedo::TorpedoSystemResource>();
    world.register_component::<crate::entities::spawner::EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::ShipRepairTeams>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::server_app::ShipSystemBlackboards>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.register_component::<crate::ship::shields::ShipShields>();
}

#[test]
fn active_intrusive_resource_question_survives_real_json_restore_and_exact_continuation() {
    use phoenix_model::focused_enquiry::{FocusedAim, ScanQuantityValue};
    let (mut app, observer) = enquiry_rig();
    prepare_capture_fixture(&mut app);
    app.insert_resource(crate::world::server::WorldContentRuntime::default());
    let before = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    question(&mut app, observer, FocusedAim::Resources, true);
    step(&mut app);
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    for _ in 0..4 {
        step(&mut app);
    }
    let snapshot = crate::snapshot::capture(app.world());
    let captured_work = snapshot
        .entities
        .iter()
        .find(|row| row.uuid == "00000000-0000-0000-0000-000000000141")
        .and_then(|row| row.scan.as_ref())
        .and_then(|scan| scan.instruments.get(&SystemId("sensors".into())))
        .and_then(|scan| scan.focused.as_ref())
        .expect("the actual observer and active work must be present in the capture");
    assert!(captured_work.progress > 0.0 && captured_work.progress < captured_work.work_secs);
    let snapshot = serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let (mut resumed, resumed_observer) = enquiry_rig();
    prepare_capture_fixture(&mut resumed);
    resumed.insert_resource(crate::world::server::WorldContentRuntime::default());
    let report = crate::snapshot::restore(resumed.world_mut(), &snapshot);
    assert!(report.gaps.is_empty(), "{report:?}");
    assert_eq!(crate::snapshot::capture(resumed.world()), snapshot);
    for continuation_step in 0..360 {
        step(&mut app);
        step(&mut resumed);
        assert_eq!(
            crate::snapshot::capture(resumed.world()),
            crate::snapshot::capture(app.world())
        );
        assert_eq!(
            crate::sim_digest::world_digest(resumed.world()),
            crate::sim_digest::world_digest(app.world()),
            "step {continuation_step}; first different digest stage: {:?}",
            crate::sim_digest::digest_stages(resumed.world())
                .into_iter()
                .zip(crate::sim_digest::digest_stages(app.world()))
                .find(|(left, right)| left != right)
        );
    }
    let record =
        &app.world().get::<ShipScanRecord>(observer).unwrap().0[&SystemId("sensors".into())];
    assert_eq!(record.history.len(), 1);
    assert!(record.focused.is_none());
    let reading = record.last.as_ref().unwrap();
    assert!(reading.intrusive && reading.aim == FocusedAim::Resources);
    assert_eq!(reading.quantities[0].value, ScanQuantityValue::Count(42));
    assert!(reading.condition_fraction.is_none() && reading.mass.is_none());
    let after = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    let owner = SystemId("sensors".into());
    let paid = before.network.consumers[&owner].energy - after.network.consumers[&owner].energy;
    assert!(
        (paid - 4.0).abs() < 0.0001,
        "two seconds of actual work at cost multiplier two: {paid}"
    );
    assert_eq!(
        resumed
            .world()
            .get::<ShipScanRecord>(resumed_observer)
            .unwrap()
            .save_state(),
        app.world()
            .get::<ShipScanRecord>(observer)
            .unwrap()
            .save_state()
    );
}

#[test]
fn world_enquiry_boundaries_identify_the_actual_paid_observer_and_repeat_for_later_work() {
    use crate::world::config::EnquiryPhase;
    use crate::world::content::WorldEvent;
    use phoenix_model::focused_enquiry::FocusedAim;
    let (mut app, observer) = enquiry_rig();
    app.insert_resource(crate::world::server::WorldContentRuntime::default());
    question(&mut app, observer, FocusedAim::Resources, false);
    step(&mut app);
    assert!(app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_world_events
        .is_empty());
    question(&mut app, observer, FocusedAim::Resources, true);
    question(&mut app, observer, FocusedAim::Resources, true);
    step(&mut app);
    let pending = &app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_world_events;
    assert!(
        matches!(pending.as_slice(),[WorldEvent::Enquiry{target_uuid,observer_uuid,system,aim,intrusive:true,phase:EnquiryPhase::Started}]
        if target_uuid=="00000000-0000-0000-0000-000000000142" && observer_uuid=="00000000-0000-0000-0000-000000000141" && system.0=="sensors" && aim=="resources")
    );
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    for _ in 0..30 {
        step(&mut app);
    }
    let pending = &app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_world_events;
    assert_eq!(pending.len(), 2);
    assert!(matches!(
        pending[1],
        WorldEvent::Enquiry {
            phase: EnquiryPhase::Completed,
            ..
        }
    ));
    question(&mut app, observer, FocusedAim::Resources, true);
    step(&mut app);
    let pending = &app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_world_events;
    assert_eq!(pending.len(), 3);
    assert!(matches!(
        pending[2],
        WorldEvent::Enquiry {
            phase: EnquiryPhase::Started,
            ..
        }
    ));
}
fn step(app: &mut App) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(100));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.update();
}
fn request(app: &mut App, entity: Entity) {
    request_for(app, entity, "sensors");
}
fn request_for(app: &mut App, entity: Entity, owner: &str) {
    let contact = app
        .world()
        .get::<crate::science::broad::BroadTracks>(entity)
        .unwrap()
        .0[&SystemId("sensor-radar".into())]
        .contact_for_source("00000000-0000-0000-0000-000000000142")
        .unwrap()
        .to_owned();
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: SystemId(owner.into()),
            payload: SystemControlPayload::ScanTarget {
                uuid: contact,
                aim: Default::default(),
                intrusive: false,
            },
            response_token: None,
            feedback_correlation: None,
        });
}

#[test]
fn enquiry_requires_timed_paid_work_and_history_does_not_follow_live_condition() {
    let (mut app, observer) = rig();
    let initial = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    request(&mut app, observer);
    step(&mut app);
    let first =
        &app.world().get::<ShipScanRecord>(observer).unwrap().0[&SystemId("sensors".into())];
    assert!(first.last.is_none());
    assert!(first
        .focused
        .as_ref()
        .is_some_and(|work| work.progress > 0.0 && work.progress < work.work_secs));
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    for _ in 0..40 {
        step(&mut app);
    }
    let before = app.world().get::<ShipScanRecord>(observer).unwrap().0
        [&SystemId("sensors".into())]
        .save_state();
    assert!(before.focused.is_none());
    assert_eq!(before.history.len(), 1);
    assert_eq!(before.last, before.history.last().cloned());
    let target = app
        .world_mut()
        .query_filtered::<Entity, With<crate::infrastructure::InfrastructureCondition>>()
        .iter(app.world())
        .find(|entity| *entity != observer)
        .unwrap();
    app.world_mut()
        .get_mut::<crate::infrastructure::InfrastructureCondition>(target)
        .unwrap()
        .0
        .set_condition(20.0);
    for _ in 0..10 {
        step(&mut app);
    }
    assert_eq!(
        app.world().get::<ShipScanRecord>(observer).unwrap().0[&SystemId("sensors".into())]
            .save_state(),
        before
    );
    let charge = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    assert!(
        charge.network.consumers[&SystemId("sensors".into())].energy
            < initial.network.consumers[&SystemId("sensors".into())].energy
    );
    assert_eq!(
        charge.network.consumers[&SystemId("sensor-radar".into())].energy,
        initial.network.consumers[&SystemId("sensor-radar".into())].energy
    );
}

#[test]
fn active_work_round_trips_and_invalid_progress_or_history_frontiers_refuse() {
    let (mut app, observer) = rig();
    request(&mut app, observer);
    step(&mut app);
    let record =
        &app.world().get::<ShipScanRecord>(observer).unwrap().0[&SystemId("sensors".into())];
    let saved: crate::science::server::FocusedSaveState =
        serde_json::from_slice(&serde_json::to_vec(&record.save_state()).unwrap()).unwrap();
    record.validate_state(&saved, 1).unwrap();
    for mode in 0..4 {
        let mut bad = saved.clone();
        let work = bad.focused.as_mut().unwrap();
        match mode {
            0 => work.progress = f32::NAN,
            1 => work.work_secs += 1.0,
            2 => work.last_tick = 2,
            _ => work.subject.clear(),
        }
        assert!(record.validate_state(&bad, 1).is_err(), "mode {mode}");
    }
    let mut resumed = record.clone();
    resumed.restore(&saved);
    assert_eq!(resumed.save_state(), saved);
}

#[test]
fn opaque_terrain_between_clear_endpoints_prevents_paid_enquiry() {
    let (mut app, observer) = rig();
    let initial = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("terrain".into()),
        Transform::from_xyz(2.0, 0.0, 0.0),
        crate::entities::spawner::RegionShapeSection(crate::regions::shape::RegionShape::Sphere {
            radius: 0.5,
        }),
        phoenix_model::field::SignalMaterialConfig {
            sensor_opaque: true,
            comms_opaque: false,
            sensor_absorption_per_unit: 0.0,
            comms_absorption_per_unit: 0.0,
        },
    ));
    request(&mut app, observer);
    for _ in 0..40 {
        step(&mut app);
    }
    let record =
        &app.world().get::<ShipScanRecord>(observer).unwrap().0[&SystemId("sensors".into())];
    assert!(record.history.is_empty());
    assert_eq!(record.focused.as_ref().unwrap().progress, 0.0);
    assert_eq!(
        app.world()
            .get::<crate::ship::power::ShipPowerSystem>(observer)
            .unwrap()
            .0
            .capture_continuation()
            .network
            .consumers,
        initial.network.consumers
    );
}

#[test]
fn independently_fitted_focused_owners_pay_and_continue_only_their_own_enquiries() {
    let (mut app, observer) = rig_with(true);
    let owner = SystemId("focused-two".into());
    let first = SystemId("sensors".into());
    let before = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    request_for(&mut app, observer, &owner.0);
    step(&mut app);
    let records = app.world().get::<ShipScanRecord>(observer).unwrap();
    assert!(records.0[&owner]
        .focused
        .as_ref()
        .is_some_and(|work| work.progress > 0.0));
    assert!(records.0[&first].focused.is_none());
    let saved = records.save_state();
    records.validate_state(&saved, 1).unwrap();
    let json: ScanSaveState = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    assert_eq!(json, saved);
    let mut missing = saved.clone();
    missing.instruments.remove(&owner);
    assert!(records.validate_state(&missing, 1).is_err());
    let after = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .capture_continuation();
    assert!(after.network.consumers[&owner].energy < before.network.consumers[&owner].energy);
    assert_eq!(
        after.network.consumers[&first].energy,
        before.network.consumers[&first].energy
    );
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0;
    assert!(boards.contains_key(&crate::science::scan_blackboard_key_for(&owner)));
    assert!(boards.contains_key(&crate::science::scan_blackboard_key_for(&first)));
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    for _ in 0..40 {
        step(&mut app);
    }
    let records = app.world().get::<ShipScanRecord>(observer).unwrap();
    assert_eq!(records.0[&owner].history.len(), 1);
    assert!(records.0[&first].history.is_empty());
    assert_eq!(
        records.0[&owner].history[0].contact_id.as_deref(),
        app.world()
            .get::<crate::science::broad::BroadTracks>(observer)
            .unwrap()
            .0[&SystemId("sensor-radar".into())]
            .contact_for_source("00000000-0000-0000-0000-000000000142")
    );
}

#[test]
fn focused_only_scope_cannot_admit_a_guessed_physical_actor() {
    let (mut app, observer) = rig();
    app.world_mut()
        .entity_mut(observer)
        .remove::<crate::science::broad::BroadTracks>();
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: SystemId("sensors".into()),
            payload: SystemControlPayload::ScanTarget {
                uuid: "00000000-0000-0000-0000-000000000142".into(),
                aim: Default::default(),
                intrusive: false,
            },
            response_token: None,
            feedback_correlation: None,
        });
    step(&mut app);
    let record =
        &app.world().get::<ShipScanRecord>(observer).unwrap().0[&SystemId("sensors".into())];
    assert!(record.focused.is_none() && record.history.is_empty());
    assert_eq!(
        record.refusal,
        Some(crate::science::ScanRefusal::NoSuchTarget)
    );
}
