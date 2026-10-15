use super::*;
use crate::core::messages::{AdmittedCommand, AdmittedCommands, SystemControlPayload, SystemId};
use crate::science::server::{publish_scan_blackboard, tick_scans};
use bevy::prelude::*;

fn rig() -> (App, Entity) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut app = App::new();
    app.insert_resource(Time::<()>::default())
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
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("00000000-0000-0000-0000-000000000142".into()),
        Transform::from_xyz(4.0, 0.0, 0.0),
        crate::infrastructure::InfrastructureCondition(infrastructure),
    ));
    (app, observer)
}
fn step(app: &mut App) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(100));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.update();
}
fn request(app: &mut App, entity: Entity) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: SystemId("sensors".into()),
            payload: SystemControlPayload::ScanTarget {
                uuid: "00000000-0000-0000-0000-000000000142".into(),
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
    let first = app.world().get::<ShipScanRecord>(observer).unwrap();
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
    let before = app
        .world()
        .get::<ShipScanRecord>(observer)
        .unwrap()
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
        app.world()
            .get::<ShipScanRecord>(observer)
            .unwrap()
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
    let record = app.world().get::<ShipScanRecord>(observer).unwrap();
    let saved: ScanSaveState =
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
    let record = app.world().get::<ShipScanRecord>(observer).unwrap();
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
