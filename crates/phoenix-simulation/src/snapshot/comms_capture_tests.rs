use super::*;
use crate::world_id::InstallWorldIdMint;
use bevy::ecs::system::RunSystemOnce;

/// Isolated owner proof uses ordinary accepted radios and their real paid
/// emitter/measurement pass; it is not a full player-admission witness.
fn fixture() -> (App, Entity, Entity, PhoenixSnapshot) {
    let mut app = App::new();
    app.insert_resource(SimTick(10))
        .insert_resource(Time::<Fixed>::from_hz(30.0))
        .init_resource::<crate::comms::server::CommsInboxRes>()
        .init_resource::<crate::comms::server::CommsRuntime>()
        .init_resource::<crate::server_app::SimOutbox>();
    app.world_mut()
        .insert_world_id_mint(crate::world_id::WorldIdMint::default());
    let source = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let receiver = crate::entities::include_resolve::load_entity_config(
        "assets/entities/capture_cruiser.toml",
    )
    .unwrap();
    let source = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &source,
        Vec3::ZERO,
        "capture-source".into(),
        None,
    );
    let receiver = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &receiver,
        Vec3::new(10.0, 0.0, 0.0),
        "capture-listener".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut()
        .get_mut::<crate::core::messages::AdmittedCommands>(source)
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: SystemId("comms".into()),
            payload: crate::core::messages::SystemControlPayload::SendCommsText {
                channel: "fleet".into(),
                audience: phoenix_model::comms_channel::CommsAudience::All,
                body: "Paid foreign evidence".into(),
            },
            response_token: None,
            feedback_correlation: None,
        });
    app.world_mut()
        .run_system_once(crate::comms::radio::tick_radio)
        .unwrap();
    let captured = app
        .world()
        .get::<crate::comms::capture::ShipCommsCapture>(receiver)
        .unwrap();
    let radio = app
        .world()
        .get::<crate::comms::radio::RadioProfile>(receiver)
        .unwrap();
    let view = crate::comms::capture::views(captured, &radio.0).unwrap();
    assert_eq!(view.rows.len(), 1);
    assert!(view.rows[0].evidence.is_none());
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(receiver)
        .unwrap()
        .0
        .insert(
            SystemId("comms".into()),
            crate::core::messages::SystemBlackboard::Comms(
                crate::core::messages::CommsBlackboard {
                    captured: Some(view),
                    ..Default::default()
                },
            ),
        );
    // Match the established sparse capture schema without creating machinery.
    app.world_mut().register_component::<EntitySystemHull>();
    app.world_mut().register_component::<EntityShipArcHull>();
    app.world_mut()
        .register_component::<crate::console::weapons::beam::ActiveBeam>();
    app.world_mut()
        .register_component::<crate::console::weapons::beam::PhaserCooldown>();
    app.world_mut()
        .register_component::<crate::console::weapons::TorpedoSystemResource>();
    app.world_mut()
        .register_component::<crate::console::repair::server::RepairRequestQueue>();
    app.world_mut()
        .register_component::<crate::ship_plugin::RepairHumanAlerted>();
    app.world_mut()
        .register_component::<crate::ai::server::ObjectiveCursors>();
    app.world_mut()
        .register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    let saved = capture(app.world());
    validate_roster(app.world(), &saved).unwrap();
    (app, source, receiver, saved)
}

#[test]
fn actual_paid_capture_required_owner_source_copy_and_view_guards_refuse_without_writes() {
    let (mut app, _, receiver, saved) = fixture();
    let before = capture(app.world());
    let digest = crate::sim_digest::world_digest(app.world());
    for fault in 0..7 {
        let mut bad = serde_json::to_value(&saved).unwrap();
        let row = bad["entities"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["uuid"] == "capture-listener")
            .unwrap();
        match fault {
            0 => row["comms_capture"] = serde_json::Value::Null,
            1 => row["comms_capture"]["owner"] = "foreign".into(),
            2 => row["comms_capture"]["rows"][0]["payload"]["body"] = "contradictory text".into(),
            3 => {
                row["comms_capture"]["rows"][0]["payload"]["transmission"]["channel"] =
                    "public".into()
            }
            4 => row["comms_capture"]["rows"][0]["payload"]["source"] = "capture-listener".into(),
            5 => row["comms_capture"]["rows"][0]["captured_tick"] = 11.into(),
            _ => {
                // Saved derived views cannot bypass the canonical decoder.
                let board = row["blackboards"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|entry| entry[0] == "comms")
                    .unwrap();
                board[1]["data"]["captured"]["rows"][0]["progress_per_mille"] = 999.into();
            }
        }
        let bad: PhoenixSnapshot = serde_json::from_value(bad).unwrap();
        assert!(validate_roster(app.world(), &bad).is_err(), "fault {fault}");
        let row = bad
            .entities
            .iter()
            .find(|row| row.uuid == "capture-listener")
            .unwrap();
        super::super::application::apply_entity_state(app.world_mut(), receiver, row, saved.tick);
        assert_eq!(capture(app.world()), before, "single-row fault {fault}");
        assert_eq!(crate::sim_digest::world_digest(app.world()), digest);
        assert!(
            !restore(app.world_mut(), &bad).is_complete(),
            "whole fault {fault}"
        );
        assert_eq!(capture(app.world()), before);
        assert_eq!(crate::sim_digest::world_digest(app.world()), digest);
    }
}

#[test]
fn absent_historical_source_and_absent_materializable_disabled_receiver_use_saved_candidate() {
    let (mut app, source, receiver, mut saved) = fixture();
    // A fresh host can still contain the bootstrap source that this candidate
    // has legitimately removed. It must not become a required historical body.
    saved.entities.retain(|row| row.uuid != "capture-source");
    validate_roster(app.world(), &saved).unwrap();
    app.world_mut().despawn(source);
    validate_roster(app.world(), &saved).unwrap();
    app.world_mut()
        .entity_mut(receiver)
        .insert(bevy::ecs::entity_disabling::Disabled);
    validate_roster(app.world(), &saved).unwrap();
    let row = saved
        .entities
        .iter_mut()
        .find(|row| row.uuid == "capture-listener")
        .unwrap();
    row.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
        template_path: "assets/entities/capture_cruiser.toml".into(),
        name: "capture-listener".into(),
        ..Default::default()
    });
    app.world_mut().despawn(receiver);
    validate_roster(app.world(), &saved).unwrap();
    saved
        .entities
        .iter_mut()
        .find(|row| row.uuid == "capture-listener")
        .unwrap()
        .comms_capture = None;
    assert!(validate_roster(app.world(), &saved).is_err());
}

#[test]
fn retained_source_security_is_checked_even_after_all_original_copies_are_cleared() {
    let (app, _, receiver, mut saved) = fixture();
    saved.comms.as_mut().unwrap().inbox.clear();
    validate_roster(app.world(), &saved).unwrap();
    let mut bad = serde_json::to_value(&saved).unwrap();
    let row = bad["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["uuid"] == "capture-listener")
        .unwrap();
    // Internally valid protected/unsupported header, with no forged paid work.
    // Withholding its saved display isolates the retained equipment guard.
    row["comms_capture"]["rows"][0]["payload"]["level"] = 3.into();
    let board = row["blackboards"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry[0] == "comms")
        .unwrap();
    board[1]["data"]["captured"] = serde_json::Value::Null;
    let bad: PhoenixSnapshot = serde_json::from_value(bad).unwrap();
    let row = bad
        .entities
        .iter()
        .find(|row| row.uuid == "capture-listener")
        .unwrap();
    validate_row(app.world(), Some(receiver), row, saved.tick).unwrap();
    let (_, reason) = validate_roster(app.world(), &bad).unwrap_err();
    assert!(
        reason.contains("security contradicts retained source equipment"),
        "{reason}"
    );
}
