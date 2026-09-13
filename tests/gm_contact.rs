//! Observer-specific basic contact overrides through the canonical action boundary.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::{
    command_admission::{log::ShipKey, HostSlot},
    entities::spawner::EntityUuid,
    gm_action::*,
    gm_contact::{mode, ContactMode},
    lockstep::FleetSlotOf,
    sim_tick::SimTick,
    world::server::WorldContentRuntime,
};
use project_phoenix as phoenix;

fn grant(sequence: u64, tick: u64, ship: &str, target: &str, mode: ContactMode) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm".into(),
        correlation: GmActionId::new(format!("contact-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::SetContactOverride {
            ship: ShipKey(ship.into()),
            target: target.into(),
            mode,
        },
    }
}
fn bare() -> App {
    let mut app = App::new();
    app.insert_resource(SimTick(42))
        .init_resource::<SimulationPaused>()
        .init_resource::<GmActionJournal>()
        .init_resource::<GmActionLog>()
        .init_resource::<WorldContentRuntime>();
    app.world_mut().spawn((
        EntityUuid("observer".into()),
        phoenix::server_app::Ship,
        FleetSlotOf(HostSlot(1)),
    ));
    app.world_mut().spawn((
        EntityUuid("other".into()),
        phoenix::server_app::Ship,
        FleetSlotOf(HostSlot(2)),
    ));
    app.world_mut().spawn(EntityUuid("target".into()));
    app
}
fn apply(app: &mut App, grant: GmActionGrant) {
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant)
        .unwrap();
    app.world_mut().run_system_once(apply_due_actions).unwrap();
}
#[test]
fn absolute_modes_are_attributed_idempotent_and_private_to_the_observer() {
    let mut app = bare();
    for (index, requested) in [
        ContactMode::Reveal,
        ContactMode::Reveal,
        ContactMode::Conceal,
        ContactMode::Normal,
    ]
    .into_iter()
    .enumerate()
    {
        apply(
            &mut app,
            grant(index as u64 + 1, 42, "observer", "target", requested),
        );
        let overrides = &app
            .world()
            .resource::<WorldContentRuntime>()
            .contact_overrides;
        assert_eq!(mode(overrides, "observer", "target"), requested);
        assert_eq!(mode(overrides, "other", "target"), ContactMode::Normal);
    }
    let facts = app.world().resource::<GmActionLog>().entries();
    assert_eq!(
        facts.iter().map(|f| f.outcome).collect::<Vec<_>>(),
        [
            GmActionOutcome::Applied,
            GmActionOutcome::NoOp,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied
        ]
    );
    assert!(facts.iter().all(
        |f| f.observer.as_deref() == Some("observer") && f.target.as_deref() == Some("target")
    ));
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .contact_overrides
        .is_empty());
}
#[test]
fn stale_target_and_non_player_observer_refuse_and_either_departure_prunes_pairs() {
    for (observer, target) in [
        ("missing", "target"),
        ("target", "other"),
        ("observer", "missing"),
    ] {
        let mut app = bare();
        apply(
            &mut app,
            grant(1, 42, observer, target, ContactMode::Reveal),
        );
        assert_eq!(
            app.world().resource::<GmActionLog>().entries()[0].outcome,
            GmActionOutcome::Refused
        );
        assert!(app
            .world()
            .resource::<WorldContentRuntime>()
            .contact_overrides
            .is_empty());
    }
    for removed in ["observer", "target"] {
        let mut app = bare();
        apply(
            &mut app,
            grant(1, 42, "observer", "target", ContactMode::Conceal),
        );
        let entity = app
            .world_mut()
            .query::<(Entity, &EntityUuid)>()
            .iter(app.world())
            .find(|(_, id)| id.0 == removed)
            .unwrap()
            .0;
        app.world_mut().despawn(entity);
        app.world_mut()
            .run_system_once(phoenix::gm_contact::prune)
            .unwrap();
        assert!(app
            .world()
            .resource::<WorldContentRuntime>()
            .contact_overrides
            .is_empty());
    }
}
fn seeded() -> (App, String, String) {
    let args = phoenix::headless::HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1309),
        deterministic: true,
        max_ticks: 260,
        ..Default::default()
    };
    let mut app = phoenix::headless::build_headless_app(&args).unwrap();
    app.finish();
    app.cleanup();
    for _ in 0..180 {
        app.update();
    }
    let (entity, observer) = app
        .world_mut()
        .query_filtered::<(Entity, &EntityUuid), With<phoenix::server_app::LocalShip>>()
        .iter(app.world())
        .map(|(entity, id)| (entity, id.0.clone()))
        .next()
        .unwrap();
    app.world_mut()
        .entity_mut(entity)
        .insert(FleetSlotOf(HostSlot(1)));
    let target = app.world().resource::<WorldContentRuntime>().name_to_uuid
        ["world.entity.raider_alpha.name"]
        .clone();
    (app, observer, target)
}
#[test]
fn seeded_contact_state_replays_and_snapshot_restores_the_same_sensor_picture() {
    for requested in [
        ContactMode::Reveal,
        ContactMode::Conceal,
        ContactMode::Normal,
    ] {
        let (mut live, observer, target) = seeded();
        let tick = live.world().resource::<SimTick>().0 + 2;
        let action = grant(1, tick, &observer, &target, requested);
        let bytes = serde_json::to_string(&action).unwrap();
        live.world_mut()
            .resource_mut::<GmActionJournal>()
            .insert(action)
            .unwrap();
        for _ in 0..5 {
            live.update();
        }
        assert_eq!(
            mode(
                &live
                    .world()
                    .resource::<WorldContentRuntime>()
                    .contact_overrides,
                &observer,
                &target
            ),
            requested
        );
        let blackboards = live
            .world_mut()
            .query::<(&EntityUuid, &phoenix::server_app::ShipSystemBlackboards)>()
            .iter(live.world())
            .find(|(id, _)| id.0 == observer)
            .unwrap()
            .1;
        assert!(blackboards.0.values().any(|bb| matches!(bb, phoenix::core::messages::SystemBlackboard::Sensors(sensors) if sensors.contact_overrides.get(&target).copied().unwrap_or_default() == requested)));
        let (mut replay, replay_observer, replay_target) = seeded();
        assert_eq!((&observer, &target), (&replay_observer, &replay_target));
        replay
            .world_mut()
            .resource_mut::<GmActionJournal>()
            .insert(serde_json::from_str(&bytes).unwrap())
            .unwrap();
        for _ in 0..5 {
            replay.update();
        }
        assert_eq!(
            phoenix::sim_digest::world_digest(live.world()),
            phoenix::sim_digest::world_digest(replay.world())
        );
        let live_picture = sensors_picture(&mut live, &observer);
        let saved = phoenix::snapshot::capture(live.world());
        let (mut restored, _, _) = seeded();
        let report = phoenix::snapshot::restore(restored.world_mut(), &saved);
        assert!(report.is_complete(), "{:?}", report.gaps);
        assert_eq!(
            restored
                .world()
                .resource::<WorldContentRuntime>()
                .contact_overrides,
            live.world()
                .resource::<WorldContentRuntime>()
                .contact_overrides
        );
        assert_eq!(
            restored.world().resource::<GmActionLog>().entries(),
            live.world().resource::<GmActionLog>().entries()
        );
        restored
            .world_mut()
            .run_system_once(phoenix::ship::sensors::publish_sensors_blackboard)
            .unwrap();
        assert_eq!(sensors_picture(&mut restored, &observer), live_picture);
        let with_contact = phoenix::sim_digest::world_digest(restored.world());
        restored
            .world_mut()
            .resource_mut::<WorldContentRuntime>()
            .contact_overrides
            .clear();
        assert_eq!(
            with_contact == phoenix::sim_digest::world_digest(restored.world()),
            requested == ContactMode::Normal
        );
    }
}

#[test]
fn browser_modes_are_strict_and_an_unadmitted_operator_cannot_write_state() {
    for mode in ["reveal", "conceal", "normal"] {
        let text = format!(
            r#"{{"operator_id":"gm","correlation":"contact","action":"set_contact_override","ship":"observer","target":"target","mode":"{mode}"}}"#
        );
        let request = phoenix::core::codec::decode_gm_action_request(&text).unwrap();
        let mut app = bare();
        assert!(submit_local(app.world_mut(), request).is_err());
        assert!(app
            .world()
            .resource::<WorldContentRuntime>()
            .contact_overrides
            .is_empty());
        let extended = text.replace(
            &format!(r#""mode":"{mode}""#),
            &format!(r#""mode":"{mode}","delay":1"#),
        );
        assert!(phoenix::core::codec::decode_gm_action_request(&extended).is_none());
    }
    for mode in ["false_contact", "delay", "degradation", "REVEAL"] {
        let text = format!(
            r#"{{"operator_id":"gm","correlation":"contact","action":"set_contact_override","ship":"observer","target":"target","mode":"{mode}"}}"#
        );
        assert!(phoenix::core::codec::decode_gm_action_request(&text).is_none());
    }
}

#[test]
fn refusal_and_reconstructed_frontier_keep_both_identities_and_mode() {
    let grant = grant(1, 42, "observer", "target", ContactMode::Conceal);
    let request = GmActionRequest {
        operator_id: grant.operator_id.clone(),
        correlation: grant.correlation.clone(),
        action: grant.action.clone(),
    };
    let refused =
        LoggedGmAction::refused_request(&request, 42, GmActionRefusalReason::OperatorMismatch);
    assert_eq!(refused.observer.as_deref(), Some("observer"));
    assert_eq!(refused.target.as_deref(), Some("target"));
    assert_eq!(refused.action_kind, GmActionKind::ContactConceal);
    let refusal = GmActionRefusal {
        sequenced_by: HostSlot(1),
        requester: HostSlot(1),
        operator_id: "gm".into(),
        correlation: request.correlation,
        action_kind: GmActionKind::ContactConceal,
        requested_active: true,
        tick: 42,
        reason: GmActionRefusalReason::UnknownEntity,
        target: Some("target".into()),
        observer: Some("observer".into()),
        objective_verb: None,
        objective_recipients: None,
        comms_recipients: None,
        npc_doctrine: None,
        verb: None,
        lever: None,
        effect_scope: None,
    };
    let frame = phoenix::lockstep::MeshFrame::GmAction(GmActionFrame::Refused(refusal));
    let encoded = phoenix::core::codec::encode_mesh_frame(&frame).unwrap();
    assert_eq!(
        phoenix::core::codec::decode_mesh_frame(&encoded),
        Some(frame)
    );
    assert!(phoenix::core::codec::decode_mesh_frame(
        &encoded.replace("contact-conceal", "session-pause")
    )
    .is_none());
    let mut journal = GmActionJournal::default();
    journal.insert(grant).unwrap();
    journal.restore_applied_frontier(1).unwrap();
    assert_eq!(
        journal.applied_log().entries()[0].observer.as_deref(),
        Some("observer")
    );
}

#[test]
fn persisted_result_observer_must_match_grant_and_exact_retransmission_is_once() {
    let mut app = bare();
    let request = grant(1, 42, "observer", "target", ContactMode::Reveal);
    apply(&mut app, request.clone());
    apply(&mut app, request);
    assert_eq!(app.world().resource::<GmActionLog>().entries().len(), 1);
    let stored = serde_json::to_value(app.world().resource::<GmActionJournal>()).unwrap();
    for observer in [serde_json::Value::Null, serde_json::json!("other")] {
        let mut malformed = stored.clone();
        malformed["applied_results"][0]["observer"] = observer;
        assert!(serde_json::from_value::<GmActionJournal>(malformed).is_err());
    }
    assert!(serde_json::from_value::<GmActionJournal>(stored).is_ok());
}

fn sensors_picture(app: &mut App, observer: &str) -> phoenix::core::messages::SensorsBlackboard {
    let bbs = app
        .world_mut()
        .query::<(&EntityUuid, &phoenix::server_app::ShipSystemBlackboards)>()
        .iter(app.world())
        .find(|(id, _)| id.0 == observer)
        .unwrap()
        .1;
    bbs.0
        .values()
        .find_map(|bb| match bb {
            phoenix::core::messages::SystemBlackboard::Sensors(bb) => Some(bb.clone()),
            _ => None,
        })
        .unwrap()
}

fn classification_palette(app: &mut App) {
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .gm_palette
        .push(phoenix::world::config::GmPaletteEntry {
            id: "freighter".into(),
            label: "test.reported.freighter".into(),
            template_path: "assets/entities/alliance_cruiser.toml".into(),
            ..Default::default()
        });
}

fn classification_grant(
    sequence: u64,
    tick: u64,
    observer: &str,
    target: &str,
    palette: Option<&str>,
) -> GmActionGrant {
    let mut result = grant(sequence, tick, observer, target, ContactMode::Normal);
    result.action = GmAction::SetContactClassification {
        ship: ShipKey(observer.into()),
        target: target.into(),
        palette: palette.map(str::to_owned),
    };
    result
}

#[test]
fn classification_is_absolute_attributed_observer_scoped_and_independent_of_detection() {
    let mut app = bare();
    classification_palette(&mut app);
    apply(
        &mut app,
        grant(1, 42, "observer", "target", ContactMode::Conceal),
    );
    let request = classification_grant(2, 42, "observer", "target", Some("freighter"));
    apply(&mut app, request.clone());
    apply(&mut app, request);
    apply(
        &mut app,
        classification_grant(3, 42, "observer", "target", Some("freighter")),
    );
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(
        runtime.contact_classifications["observer"]["target"].label,
        "test.reported.freighter"
    );
    assert!(!runtime.contact_classifications.contains_key("other"));
    assert_eq!(
        mode(&runtime.contact_overrides, "observer", "target"),
        ContactMode::Conceal
    );
    let results = app.world().resource::<GmActionLog>().entries();
    assert_eq!(results.len(), 3);
    assert_eq!(results[1].action_kind, GmActionKind::ContactMisclassify);
    assert_eq!(results[1].observer.as_deref(), Some("observer"));
    assert_eq!(results[1].target.as_deref(), Some("target"));
    assert_eq!(results[2].outcome, GmActionOutcome::NoOp);
    apply(
        &mut app,
        classification_grant(4, 42, "observer", "target", None),
    );
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert!(runtime.contact_classifications.is_empty());
    assert_eq!(
        mode(&runtime.contact_overrides, "observer", "target"),
        ContactMode::Conceal
    );
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[3].action_kind,
        GmActionKind::ContactClassificationNormal
    );
}

#[test]
fn classification_refuses_unknown_palette_stale_entities_and_unadmitted_operators() {
    for (observer, target, palette) in [
        ("observer", "target", "missing"),
        ("missing", "target", "freighter"),
        ("observer", "missing", "freighter"),
    ] {
        let mut app = bare();
        classification_palette(&mut app);
        apply(
            &mut app,
            classification_grant(1, 42, observer, target, Some(palette)),
        );
        assert!(app
            .world()
            .resource::<WorldContentRuntime>()
            .contact_classifications
            .is_empty());
        assert_eq!(
            app.world().resource::<GmActionLog>().entries()[0].outcome,
            GmActionOutcome::Refused
        );
    }
    let request = phoenix::core::codec::decode_gm_action_request(
        r#"{"operator_id":"gm","correlation":"classify","action":"set_contact_classification","ship":"observer","target":"target","palette":"freighter"}"#).unwrap();
    let mut app = bare();
    classification_palette(&mut app);
    assert!(submit_local(app.world_mut(), request).is_err());
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .contact_classifications
        .is_empty());
    for invalid in ["", "line\nbreak", &"x".repeat(129)] {
        assert!(
            classification_grant(1, 42, "observer", "target", Some(invalid))
                .validate()
                .is_err()
        );
    }
}

#[test]
fn classification_replays_snapshots_and_publishes_only_the_observers_reported_label() {
    let (mut live, observer, target) = seeded();
    classification_palette(&mut live);
    let tick = live.world().resource::<SimTick>().0 + 2;
    let action = classification_grant(1, tick, &observer, &target, Some("freighter"));
    let frame = phoenix::lockstep::MeshFrame::GmAction(GmActionFrame::Granted(action.clone()));
    let encoded = phoenix::core::codec::encode_mesh_frame(&frame).unwrap();
    assert_eq!(
        phoenix::core::codec::decode_mesh_frame(&encoded),
        Some(frame)
    );
    live.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(action.clone())
        .unwrap();
    let (mut replay, _, _) = seeded();
    classification_palette(&mut replay);
    replay
        .world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(action)
        .unwrap();
    for _ in 0..5 {
        live.update();
        replay.update();
    }
    assert_eq!(
        phoenix::sim_digest::world_digest(live.world()),
        phoenix::sim_digest::world_digest(replay.world())
    );
    let picture = sensors_picture(&mut live, &observer);
    assert_eq!(
        picture.contact_classifications[&target],
        "test.reported.freighter"
    );
    assert!(picture.contact_overrides.is_empty());
    let saved = phoenix::snapshot::capture(live.world());
    let (mut restored, _, _) = seeded();
    classification_palette(&mut restored);
    let report = phoenix::snapshot::restore(restored.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    restored
        .world_mut()
        .run_system_once(phoenix::ship::sensors::publish_sensors_blackboard)
        .unwrap();
    assert_eq!(sensors_picture(&mut restored, &observer), picture);
    assert_eq!(
        restored.world().resource::<GmActionLog>().entries(),
        live.world().resource::<GmActionLog>().entries()
    );
    let with_classification = phoenix::sim_digest::world_digest(restored.world());
    restored
        .world_mut()
        .resource_mut::<WorldContentRuntime>()
        .contact_classifications
        .clear();
    assert_ne!(
        with_classification,
        phoenix::sim_digest::world_digest(restored.world())
    );
}

#[test]
fn classification_prunes_with_either_identity_and_despawn_captures_its_inverse() {
    for removed in ["observer", "target"] {
        let mut app = bare();
        classification_palette(&mut app);
        apply(
            &mut app,
            classification_grant(1, 42, "observer", "target", Some("freighter")),
        );
        let entity = app
            .world_mut()
            .query::<(Entity, &EntityUuid)>()
            .iter(app.world())
            .find(|(_, id)| id.0 == removed)
            .unwrap()
            .0;
        app.world_mut().despawn(entity);
        app.world_mut()
            .run_system_once(phoenix::gm_contact::prune)
            .unwrap();
        assert!(app
            .world()
            .resource::<WorldContentRuntime>()
            .contact_classifications
            .is_empty());
    }
    let mut app = bare();
    classification_palette(&mut app);
    apply(
        &mut app,
        grant(1, 42, "observer", "target", ContactMode::Reveal),
    );
    apply(
        &mut app,
        classification_grant(2, 42, "observer", "target", Some("freighter")),
    );
    let target = app
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == "target")
        .unwrap()
        .0;
    let cleared = phoenix::gm_despawn::remove_entity(app.world_mut(), target);
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].mode, ContactMode::Reveal);
    assert_eq!(
        cleared[0].classification.as_ref().unwrap().label,
        "test.reported.freighter"
    );
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .contact_classifications
        .is_empty());
}

#[test]
fn viewscreen_classification_keeps_detection_geometry_and_truth_intact() {
    let mut app = bare();
    classification_palette(&mut app);
    apply(
        &mut app,
        classification_grant(1, 42, "observer", "target", Some("freighter")),
    );
    let runtime = app.world().resource::<WorldContentRuntime>();
    let truth = vec![phoenix::core::messages::EntitySnapshot {
        uuid: "target".into(),
        name: Some("true-raider".into()),
        position: Some([20., 0., 0.]),
        radar_icon: Some("ship".into()),
        tags: vec!["ship".into()],
        ..Default::default()
    }];
    let mut projected = truth.clone();
    phoenix::gm_contact::classify_viewscreen_contacts(
        &mut projected,
        &runtime.contact_classifications,
        "observer",
    );
    assert_eq!(
        projected[0].name.as_deref(),
        Some("test.reported.freighter")
    );
    assert_eq!(projected[0].position, truth[0].position);
    assert_eq!(projected[0].radar_icon, truth[0].radar_icon);
    assert_eq!(truth[0].name.as_deref(), Some("true-raider"));
    let mut other = truth.clone();
    phoenix::gm_contact::classify_viewscreen_contacts(
        &mut other,
        &runtime.contact_classifications,
        "other",
    );
    assert_eq!(other, truth);
}
