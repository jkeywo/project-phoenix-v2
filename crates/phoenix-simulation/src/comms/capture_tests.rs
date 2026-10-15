use super::*;
use crate::core::messages::{AdmittedCommand, AdmittedCommands, CommsBlackboard, SystemId};
use crate::world_id::InstallWorldIdMint;
use bevy::ecs::system::RunSystemOnce;

fn fixture(distance: f32, level: u8) -> (App, Entity, Entity) {
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(10))
        .insert_resource(Time::<Fixed>::from_hz(30.0))
        .init_resource::<CommsInboxRes>()
        .init_resource::<CommsRuntime>()
        .init_resource::<crate::server_app::SimOutbox>();
    app.world_mut()
        .insert_world_id_mint(crate::world_id::WorldIdMint::default());
    let mut source = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    source
        .comms
        .as_mut()
        .unwrap()
        .radio
        .as_mut()
        .unwrap()
        .channels
        .iter_mut()
        .find(|row| row.id == "fleet")
        .unwrap()
        .protection_level = level;
    let mut receiver = crate::entities::include_resolve::load_entity_config(
        "assets/entities/capture_cruiser.toml",
    )
    .unwrap();
    receiver
        .comms
        .as_mut()
        .unwrap()
        .radio
        .as_mut()
        .unwrap()
        .capture
        .as_mut()
        .unwrap()
        .ticks_per_level = 3;
    let source = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &source,
        Vec3::ZERO,
        "emitter".into(),
        None,
    );
    let receiver = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &receiver,
        Vec3::new(distance, 0.0, 0.0),
        "listener".into(),
        None,
    );
    app.world_mut().flush();
    if let Some(mut pose) = app
        .world_mut()
        .get_mut::<crate::ship::state::ShipPhysics>(receiver)
    {
        pose.x = distance;
        pose.z = 0.0;
    }
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: SystemId("comms".into()),
            payload: SystemControlPayload::SendCommsText {
                channel: "fleet".into(),
                audience: phoenix_model::comms_channel::CommsAudience::Faction {
                    faction_uuid: uuid::Uuid::nil().to_string(),
                },
                body: "Actual protected foreign traffic".into(),
            },
            response_token: None,
            feedback_correlation: None,
        });
    (app, source, receiver)
}
fn capture_view(
    app: &App,
    receiver: Entity,
) -> phoenix_model::comms_capture::CommsCaptureControlsView {
    views(
        app.world().get::<ShipCommsCapture>(receiver).unwrap(),
        &app.world()
            .get::<super::super::radio::RadioProfile>(receiver)
            .unwrap()
            .0,
    )
    .unwrap()
}
fn energy(app: &App, receiver: Entity) -> f32 {
    app.world()
        .get::<crate::ship::power::ShipPowerSystem>(receiver)
        .unwrap()
        .0
        .network()
        .consumers[&SystemId("comms".into())]
        .energy
}
#[test]
fn actual_paid_foreign_emission_captures_without_audience_key_or_request_participation() {
    let (mut app, source, receiver) = fixture(10.0, 1);
    let before = energy(&app, receiver);
    app.world_mut()
        .run_system_once(super::super::radio::tick_radio)
        .unwrap();
    assert!(energy(&app, receiver) < before);
    let captured = capture_view(&app, receiver);
    assert_eq!(captured.rows.len(), 1);
    assert!(captured.rows[0].evidence.is_none());
    assert!(captured.rows[0].feasible);
    assert_eq!(
        app.world()
            .resource::<CommsInboxRes>()
            .0
            .iter()
            .filter(|row| row.is_for_ship(Some("listener")))
            .count(),
        0,
        "interception does not become an addressed inbox or response"
    );
    let public = serde_json::to_string(&captured).unwrap();
    for private in [
        "Actual protected foreign traffic",
        "emitter",
        "alliance-fleet",
    ] {
        assert!(!public.contains(private));
    }
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0
        .clear();
    let owner = SystemId("comms".into());
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(receiver)
        .unwrap()
        .0
        .insert(
            owner.clone(),
            SystemBlackboard::Comms(CommsBlackboard {
                captured: Some(captured),
                ..Default::default()
            }),
        );
    app.world_mut()
        .get_mut::<AdmittedCommands>(receiver)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: owner,
            payload: SystemControlPayload::SelectCommsCapture { capture: Some(1) },
            response_token: None,
            feedback_correlation: None,
        });
    for tick in 11..=13 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        let before = energy(&app, receiver);
        app.world_mut()
            .run_system_once(super::super::radio::tick_radio)
            .unwrap();
        assert!(energy(&app, receiver) < before);
        if tick < 13 {
            assert!(capture_view(&app, receiver).rows[0].evidence.is_none());
        }
        app.world_mut()
            .get_mut::<AdmittedCommands>(receiver)
            .unwrap()
            .0
            .clear();
    }
    let evidence = capture_view(&app, receiver)
        .rows
        .remove(0)
        .evidence
        .unwrap();
    assert_eq!(evidence.body, "Actual protected foreign traffic");
    assert_eq!(
        app.world()
            .resource::<CommsInboxRes>()
            .0
            .iter()
            .filter(|row| row.is_for_ship(Some("listener")))
            .count(),
        0
    );
}
#[test]
fn never_received_history_cannot_be_recovered_by_later_range_or_elapsed_time() {
    let (mut app, source, receiver) = fixture(10_000.0, 1);
    app.world_mut()
        .run_system_once(super::super::radio::tick_radio)
        .unwrap();
    assert!(capture_view(&app, receiver).rows.is_empty());
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0
        .clear();
    app.world_mut()
        .get_mut::<crate::ship::state::ShipPhysics>(receiver)
        .unwrap()
        .x = 10.0;
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 11;
    app.world_mut()
        .run_system_once(super::super::radio::tick_radio)
        .unwrap();
    assert!(capture_view(&app, receiver).rows.is_empty());
    assert!(app
        .world()
        .resource::<CommsInboxRes>()
        .0
        .iter()
        .any(|row| row.sender_uuid == "emitter"));
}
#[test]
fn actual_higher_protection_is_captured_but_never_feasible_without_supported_key() {
    let (mut app, source, receiver) = fixture(10.0, 3);
    app.world_mut()
        .run_system_once(super::super::radio::tick_radio)
        .unwrap();
    assert_eq!(capture_view(&app, receiver).rows.len(), 1);
    assert!(!capture_view(&app, receiver).rows[0].feasible);
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0
        .clear();
    for tick in 11..=370 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        app.world_mut()
            .run_system_once(super::super::radio::tick_radio)
            .unwrap();
        assert!(capture_view(&app, receiver).rows[0].evidence.is_none());
        assert_eq!(capture_view(&app, receiver).rows[0].progress_per_mille, 0);
    }
}
