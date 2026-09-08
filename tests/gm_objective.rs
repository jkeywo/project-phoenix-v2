//! Authored Objective controls use the real agreed-tick reducer (#1307).
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::command_admission::HostSlot;
use phoenix::core::messages::ObjectiveStatus;
use phoenix::gm_action::*;
use phoenix::gm_objective::ObjectiveVerb;
use phoenix::world::server::{ObjectiveManagerRes, WorldContentRuntime};
use project_phoenix as phoenix;

fn palette() -> Vec<phoenix::gm_objective::ObjectivePaletteEntry> {
    let config = phoenix::world::config::parse_world(
        r#"
[global]
name = "test"
[[gm_objective_palette]]
id = "escort"
label = "objective.escort"
text = "objective.escort"
targets = ["contact"]
recipients = ["ship-a"]
mandatory = true
base_priority = 7.0
[[gm_objective_palette]]
id = "hold"
label = "objective.hold"
text = "objective.hold"
"#,
    )
    .unwrap();
    config.gm_objective_palette
}
fn grant(
    sequence: u64,
    tick: u64,
    id: &str,
    verb: ObjectiveVerb,
    recipients: Vec<String>,
) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-one".into(),
        correlation: GmActionId::new(format!("objective-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::ObjectiveAction {
            objective: id.into(),
            verb,
            recipients,
        },
    }
}
fn bare() -> App {
    let mut app = App::new();
    app.insert_resource(phoenix::sim_tick::SimTick(42))
        .init_resource::<SimulationPaused>()
        .init_resource::<GmActionJournal>()
        .init_resource::<GmActionLog>()
        .init_resource::<ObjectiveManagerRes>()
        .init_resource::<WorldContentRuntime>()
        .add_message::<phoenix::core::balance::BalanceEvent>();
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .gm_objective_palette = palette();
    for (slot, id) in [(1, "ship-a"), (2, "ship-b")] {
        app.world_mut().spawn((
            phoenix::entities::spawner::EntityUuid(id.into()),
            phoenix::lockstep::FleetSlotOf(HostSlot(slot)),
        ));
    }
    app
}
fn apply(app: &mut App, g: GmActionGrant) {
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(g)
        .unwrap();
    app.world_mut().run_system_once(apply_due_actions).unwrap();
}

#[test]
fn authored_scope_lifecycle_and_same_tick_conflicts_are_atomic_without_local_ship() {
    let mut app = bare();
    let scoped = vec!["ship-a".to_string()];
    let activation = grant(1, 42, "escort", ObjectiveVerb::Activate, scoped.clone());
    apply(&mut app, activation.clone());
    apply(&mut app, activation);
    {
        let manager = &app.world().resource::<ObjectiveManagerRes>().0;
        assert_eq!(manager.status("escort"), Some(&ObjectiveStatus::Active));
        assert_eq!(manager.targets("escort").unwrap(), ["contact"]);
        assert_eq!(manager.snapshots_for("ship-a").len(), 1);
        assert!(manager.snapshots_for("ship-b").is_empty());
        assert_eq!(
            manager.scored_pool_for(&Default::default(), "ship-a")[0].score,
            17.0
        );
    }
    apply(
        &mut app,
        grant(2, 42, "escort", ObjectiveVerb::Complete, vec![]),
    );
    assert_eq!(
        app.world()
            .resource::<ObjectiveManagerRes>()
            .0
            .status("escort"),
        Some(&ObjectiveStatus::Active)
    );
    apply(
        &mut app,
        grant(3, 42, "escort", ObjectiveVerb::Complete, scoped.clone()),
    );
    apply(
        &mut app,
        grant(4, 42, "escort", ObjectiveVerb::Fail, scoped.clone()),
    );
    apply(
        &mut app,
        grant(5, 42, "escort", ObjectiveVerb::Activate, scoped),
    );
    let manager = &mut app.world_mut().resource_mut::<ObjectiveManagerRes>().0;
    assert_eq!(manager.status("escort"), Some(&ObjectiveStatus::Completed));
    assert_eq!(manager.drain_transitions().len(), 2);
    let events: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<phoenix::core::balance::BalanceEvent>>()
        .drain()
        .collect();
    assert_eq!(
        events.len(),
        3,
        "posted + completion score event + completed; refusals add nothing"
    );
    let facts = app.world().resource::<GmActionLog>().entries();
    assert_eq!(facts.len(), 5);
    assert_eq!(
        facts[1].reason,
        Some(GmActionRefusalReason::ObjectiveScopeMismatch)
    );
    assert_eq!(
        facts[3].reason,
        Some(GmActionRefusalReason::ObjectiveNotActive)
    );
    assert_eq!(facts[4].outcome, GmActionOutcome::NoOp);
    assert_eq!(facts[2].objective_verb, Some(ObjectiveVerb::Complete));
    assert_eq!(facts[2].objective_recipients, Some(vec!["ship-a".into()]));
}

#[test]
fn apply_tick_checks_lost_recipients_unknown_ids_and_ingress_authority() {
    let mut app = bare();
    let g = grant(
        1,
        44,
        "escort",
        ObjectiveVerb::Activate,
        vec!["ship-a".into()],
    );
    apply(&mut app, g);
    assert!(app.world().resource::<GmActionLog>().entries().is_empty());
    let entity = app
        .world_mut()
        .query::<(Entity, &phoenix::entities::spawner::EntityUuid)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == "ship-a")
        .unwrap()
        .0;
    app.world_mut().despawn(entity);
    app.world_mut()
        .resource_mut::<phoenix::sim_tick::SimTick>()
        .0 = 44;
    app.world_mut().run_system_once(apply_due_actions).unwrap();
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[0].reason,
        Some(GmActionRefusalReason::ObjectiveScopeMismatch)
    );
    apply(
        &mut app,
        grant(2, 44, "invented", ObjectiveVerb::Activate, vec![]),
    );
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[1].reason,
        Some(GmActionRefusalReason::UnknownObjective)
    );
    let request = GmActionRequest {
        operator_id: "unbound".into(),
        correlation: GmActionId::new("unbound").unwrap(),
        action: GmAction::ObjectiveAction {
            objective: "hold".into(),
            verb: ObjectiveVerb::Activate,
            recipients: vec![],
        },
    };
    assert!(submit_local(app.world_mut(), request).is_err());
    let roster = phoenix::lockstep::FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![phoenix::lockstep::FleetGm {
            host: HostSlot(2),
            operator_id: "real-gm".into(),
        }],
        HostSlot(1),
        HostSlot(1),
    )
    .unwrap();
    let forged = grant(3, 44, "hold", ObjectiveVerb::Activate, vec![]);
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Granted(forged), &roster),
        Err(GmActionRefusalReason::OperatorMismatch)
    );
    assert!(app
        .world()
        .resource::<ObjectiveManagerRes>()
        .0
        .sorted_snapshots()
        .is_empty());
}

#[test]
fn closed_ingress_and_authored_palette_reject_untrusted_vocabulary() {
    let valid = serde_json::json!({"action":"objective_action","operator_id":"gm-one","correlation":"valid","objective":"hold","verb":"activate","recipients":[]});
    assert!(phoenix::core::codec::decode_gm_action_request(&valid.to_string()).is_some());
    for (key, value) in [
        ("text", serde_json::json!("arbitrary")),
        ("base_priority", serde_json::json!(999)),
        ("targets", serde_json::json!(["another"])),
    ] {
        let mut changed = valid.clone();
        changed[key] = value;
        assert!(phoenix::core::codec::decode_gm_action_request(&changed.to_string()).is_none());
    }
    for verb in ["delete", "reopen", "invent"] {
        let mut changed = valid.clone();
        changed["verb"] = serde_json::json!(verb);
        assert!(phoenix::core::codec::decode_gm_action_request(&changed.to_string()).is_none());
    }
    let mut duplicate_scope = valid.clone();
    duplicate_scope["recipients"] = serde_json::json!(["ship-a", "ship-a"]);
    assert!(phoenix::core::codec::decode_gm_action_request(&duplicate_scope.to_string()).is_none());
    let duplicate = r#"[[gm_objective_palette]]
id="duplicate"
label="objective.test"
text="objective.test"
[[gm_objective_palette]]
id="duplicate"
label="objective.test"
text="objective.test"
"#;
    assert!(phoenix::world::config::parse_world(duplicate)
        .unwrap_err()
        .contains("duplicate"));
}

fn seeded() -> App {
    let args = phoenix::headless::HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1307),
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
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .gm_objective_palette = palette();
    app
}
#[test]
fn seeded_canonical_objectives_replay_and_restore_terminal_records() {
    let mut live = seeded();
    let tick = live.world().resource::<phoenix::sim_tick::SimTick>().0 + 2;
    let grants = vec![
        grant(1, tick, "hold", ObjectiveVerb::Activate, vec![]),
        grant(2, tick + 2, "hold", ObjectiveVerb::Fail, vec![]),
    ];
    let encoded = serde_json::to_string(&grants).unwrap();
    for g in grants {
        live.world_mut()
            .resource_mut::<GmActionJournal>()
            .insert(g)
            .unwrap();
    }
    for _ in 0..10 {
        live.update();
    }
    assert_eq!(
        live.world()
            .resource::<ObjectiveManagerRes>()
            .0
            .status("hold"),
        Some(&ObjectiveStatus::Failed)
    );
    let mut replay = seeded();
    for g in serde_json::from_str::<Vec<GmActionGrant>>(&encoded).unwrap() {
        replay
            .world_mut()
            .resource_mut::<GmActionJournal>()
            .insert(g)
            .unwrap();
    }
    for _ in 0..10 {
        replay.update();
    }
    assert_eq!(
        phoenix::sim_digest::world_digest(live.world()),
        phoenix::sim_digest::world_digest(replay.world())
    );
    let saved = phoenix::snapshot::capture(live.world());
    let mut restored = seeded();
    let report = phoenix::snapshot::restore(restored.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(
        restored
            .world()
            .resource::<ObjectiveManagerRes>()
            .0
            .records(),
        live.world().resource::<ObjectiveManagerRes>().0.records()
    );
    assert!(restored
        .world_mut()
        .resource_mut::<ObjectiveManagerRes>()
        .0
        .drain_transitions()
        .is_empty());
}
