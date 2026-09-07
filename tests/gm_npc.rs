//! Authored NPC intent through canonical actions, ordinary AI and real host restoration.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::{
    command_admission::HostSlot,
    entities::spawner::{BehaviourSection, EntityUuid},
    gm_action::*,
    gm_npc::NpcDoctrineState,
    sim_tick::SimTick,
    world::server::WorldContentRuntime,
};
use project_phoenix as phoenix;

const WORLD: &str = "tests/fixtures/worlds/gm_npc_doctrine.toml";
const LAYER: &str = "tests/fixtures/gm_npc_layer.toml";
fn args() -> phoenix::headless::HeadlessArgs {
    phoenix::headless::HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1308),
        deterministic: true,
        ..Default::default()
    }
}
fn boot() -> App {
    let mut app = phoenix::headless::build_headless_app(&args()).unwrap();
    app.add_plugins((
        phoenix::gm_projection::GmProjectionPlugin,
        phoenix::gm_activity::GmActivityPlugin,
    ));
    app.insert_resource(phoenix::gm_projection::BrowserGameMaster);
    app.finish();
    app.cleanup();
    for _ in 0..400 {
        app.update();
    }
    assert_eq!(
        *app.world()
            .resource::<State<phoenix::core::messages::GamePhase>>()
            .get(),
        phoenix::core::messages::GamePhase::InProgress
    );
    app
}
fn named(app: &mut App, name: &str) -> (Entity, String) {
    let uuid = app.world().resource::<WorldContentRuntime>().name_to_uuid[name].clone();
    let mut query = app.world_mut().query::<(Entity, &EntityUuid)>();
    (
        query
            .iter(app.world())
            .find(|(_, id)| id.0 == uuid)
            .unwrap()
            .0,
        uuid,
    )
}
fn grant(sequence: u64, tick: u64, target: &str, doctrine: &str) -> GmActionGrant {
    let from = HostSlot(if sequence % 2 == 0 { 2 } else { 3 });
    GmActionGrant {
        from,
        sequenced_by: HostSlot(1),
        operator_id: format!("gm-{}", from.0),
        correlation: GmActionId::new(format!("npc-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(from, sequence),
        action: GmAction::SetNpcDoctrine {
            target: target.into(),
            doctrine: doctrine.into(),
        },
    }
}
fn apply(app: &mut App, sequence: u64, target: &str, doctrine: &str) {
    let tick = app.world().resource::<SimTick>().0;
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(sequence, tick, target, doctrine))
        .unwrap();
    app.world_mut().run_system_once(apply_due_actions).unwrap();
}
fn doctrine(app: &App, entity: Entity) -> &str {
    &app.world()
        .get::<NpcDoctrineState>(entity)
        .unwrap()
        .0
        .as_ref()
        .unwrap()
        .id
}

#[test]
fn canonical_npc_choices_operate_real_ai_and_publish_attributed_results() {
    use phoenix::console_bridge::GmEntityProjectionChanged;
    let mut app = boot();
    let (npc, uuid) = named(&mut app, "Directive courier");
    let (_, station) = named(&mut app, "Incompatible station");
    let before = *app
        .world()
        .get::<phoenix::ship::state::ShipPhysics>(npc)
        .unwrap();
    apply(&mut app, 1, &uuid, "north");
    let retry = app.world().resource::<GmActionJournal>().grants()[0].clone();
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(retry)
        .unwrap();
    app.world_mut().run_system_once(apply_due_actions).unwrap();
    assert_eq!(
        app.world().resource::<GmActionLog>().entries().len(),
        1,
        "same correlation is one fact"
    );
    apply(&mut app, 2, &uuid, "north");
    apply(&mut app, 3, &station, "north");
    let mut activity = None;
    for _ in 0..180 {
        app.update();
        if let Some(changed) = app
            .world_mut()
            .resource_mut::<Messages<phoenix::console_bridge::GmActivityFeedChanged>>()
            .drain()
            .last()
        {
            activity = Some(changed.payload);
        }
    }
    let after = app
        .world()
        .get::<phoenix::ship::state::ShipPhysics>(npc)
        .unwrap();
    let distance = |x: f32, z: f32| x * x + (z + 1500.0) * (z + 1500.0);
    assert!(
        distance(after.x, after.z) < distance(before.x, before.z) - 1.0,
        "ordinary AI must fly toward the authored anchor: before={before:?}, after={after:?}"
    );
    let projection = app
        .world_mut()
        .resource_mut::<Messages<GmEntityProjectionChanged>>()
        .drain()
        .last()
        .unwrap()
        .payload;
    assert_eq!(
        projection.npc_doctrines[&uuid].current.as_deref(),
        Some("north")
    );
    assert_eq!(
        projection.npc_doctrines[&uuid].intent.as_deref(),
        Some("Fly north")
    );
    assert!(!projection.npc_doctrines.contains_key(&station));
    assert!(projection
        .entities
        .iter()
        .filter(|entity| entity.kind == phoenix::gm_projection::GmEntityKind::PlayerShip)
        .all(|entity| !projection.npc_doctrines.contains_key(&entity.entity_id)));
    let facts = app.world().resource::<GmActionLog>().entries();
    assert_eq!(
        facts.iter().map(|fact| fact.outcome).collect::<Vec<_>>(),
        [
            GmActionOutcome::Applied,
            GmActionOutcome::NoOp,
            GmActionOutcome::Refused
        ]
    );
    assert!(facts
        .iter()
        .all(|fact| fact.npc_doctrine.as_deref() == Some("north")));
    let activity = activity.expect("ordinary activity producer published the actions");
    let row = activity.entries.iter().find(|row| matches!(&row.detail, phoenix::gm_activity::GmActivityDetail::GmAction(detail) if detail.correlation == "npc-1")).unwrap();
    assert_eq!(row.ships.len(), 1);
    assert_eq!(row.ships[0].entity_id, uuid);
    let phoenix::gm_activity::GmActivityDetail::GmAction(detail) = &row.detail else {
        unreachable!()
    };
    assert_eq!(detail.operator.id, "gm-3");
    assert!(
        matches!(&detail.action, phoenix::gm_activity::GmActivityAction::SetNpcDoctrine { target, doctrine } if target == &uuid && doctrine == "north")
    );
}

#[test]
fn scripted_choice_and_apply_tick_refusals_share_the_same_doctrine_path() {
    let mut app = boot();
    let (npc, uuid) = named(&mut app, "Directive courier");
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .pending_world_events
        .push(phoenix::world::content::WorldEvent::FlagSet {
            name: "scripted-doctrine".into(),
            origin_layer: None,
        });
    app.update();
    assert_eq!(
        doctrine(&app, npc),
        "north",
        "real Rhai effect reaches the shared canonical doctrine applier"
    );
    apply(&mut app, 1, &uuid, "north");
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[0].outcome,
        GmActionOutcome::NoOp
    );
    let tick = app.world().resource::<SimTick>().0;
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(2, tick, &uuid, "east"))
        .unwrap();
    let original = app.world().get::<BehaviourSection>(npc).unwrap().clone();
    app.world_mut()
        .get_mut::<phoenix::ship::components::ShipConfigComponent>(npc)
        .unwrap()
        .0
        .systems
        .retain(|system| system.kind != "helm_steering");
    app.world_mut().run_system_once(apply_due_actions).unwrap();
    assert_eq!(
        app.world().get::<BehaviourSection>(npc).unwrap().0,
        original.0
    );
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[1].reason,
        Some(GmActionRefusalReason::NpcDoctrineIncompatible)
    );
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .gm_npc_doctrine_palette
        .clear();
    apply(&mut app, 3, &uuid, "east");
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[2].reason,
        Some(GmActionRefusalReason::UnknownNpcDoctrine)
    );
    // Entity removal after sequencing is validated at application, with no other NPC mutation.
    let config =
        phoenix::world::config::parse_world(include_str!("fixtures/worlds/gm_npc_doctrine.toml"))
            .unwrap();
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .gm_npc_doctrine_palette = config.gm_npc_doctrine_palette;
    let tick = app.world().resource::<SimTick>().0;
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(4, tick, &uuid, "east"))
        .unwrap();
    app.world_mut().despawn(npc);
    app.world_mut().run_system_once(apply_due_actions).unwrap();
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[3].reason,
        Some(GmActionRefusalReason::UnknownEntity)
    );
}

fn load_layer(app: &mut App) {
    use phoenix::world::server::*;
    app.world_mut()
        .resource_mut::<PendingWorldLayerChanges>()
        .0
        .push(WorldLayerChange::Load {
            path: LAYER.into(),
            loader_path: None,
        });
    for _ in 0..300 {
        app.update();
        if app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .get(LAYER)
            .is_some_and(|layer| layer.is_active)
        {
            return;
        }
    }
    panic!("layer did not load");
}

#[test]
fn local_ship_only_directives_are_not_offered_or_applied_to_an_equipped_npc() {
    use phoenix::entities::config::DoctrineObjective;
    let mut app = boot();
    let (npc, uuid) = named(&mut app, "Directive courier");
    let config = &app
        .world()
        .get::<phoenix::ship::components::ShipConfigComponent>(npc)
        .unwrap()
        .0;
    for kind in ["comms", "navigation"] {
        assert!(config.systems.iter().any(|system| system.kind == kind));
    }
    let original = app
        .world()
        .get::<BehaviourSection>(npc)
        .unwrap()
        .0
        .doctrine
        .clone();
    for (index, kind) in ["Hail", "Order"].iter().enumerate() {
        let target_fields = if *kind == "Hail" {
            "directive_hail_target = \"Incompatible station\""
        } else {
            "directive_order_target = \"traffic\"\ndirective_order_route = \"route\""
        };
        let objective: DoctrineObjective = toml::from_str(&format!(
            "id = \"unsupported-npc-intent\"\ndirective_kind = \"{kind}\"\n{target_fields}"
        ))
        .unwrap();
        phoenix::entities::config::validate_doctrine_directives(std::slice::from_ref(&objective))
            .unwrap();
        let profile = phoenix::gm_npc::NpcDoctrinePaletteEntry {
            id: (*kind).into(),
            label: (*kind).into(),
            targets: vec![uuid.clone()],
            doctrine: vec![objective],
            origin_layer: None,
        };
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .gm_npc_doctrine_palette
            .push(profile);
        apply(&mut app, index as u64 + 1, &uuid, kind);
    }
    assert!(app
        .world()
        .resource::<GmActionLog>()
        .entries()
        .iter()
        .all(|row| row.outcome == GmActionOutcome::Refused
            && row.reason == Some(GmActionRefusalReason::NpcDoctrineIncompatible)));
    assert_eq!(
        app.world().get::<BehaviourSection>(npc).unwrap().0.doctrine,
        original
    );
    assert!(app
        .world()
        .get::<NpcDoctrineState>(npc)
        .unwrap()
        .0
        .is_none());
    app.update();
    let projection = app
        .world_mut()
        .resource_mut::<Messages<phoenix::console_bridge::GmEntityProjectionChanged>>()
        .drain()
        .last()
        .unwrap()
        .payload;
    assert_eq!(
        projection.npc_doctrines[&uuid]
            .choices
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["north", "east"]
    );
}

#[test]
fn snapshot_restores_applied_doctrine_after_layer_withdrawal_and_clears_bootstrap_selection() {
    use phoenix::{
        sim_digest::{digest_stages, first_divergent_scope, world_digest},
        snapshot::{
            capture, ready_to_restore, reconcile_world_layers, restore, LayerReconcileStatus,
            PhoenixSnapshot,
        },
        world::server::*,
    };
    // Match the ordinary resume boundary: derived layer content must exist
    // before the authoritative overwrite can restore its state by identity.
    fn restore_ordinary(app: &mut App, saved: &PhoenixSnapshot) {
        for _ in 0..1_000 {
            match reconcile_world_layers(app.world_mut(), saved) {
                LayerReconcileStatus::Ready if ready_to_restore(app.world(), saved) => {
                    let report = restore(app.world_mut(), saved);
                    assert!(report.is_complete(), "{:?}", report.gaps);
                    return;
                }
                LayerReconcileStatus::Failed(path) => {
                    panic!("world-layer reconciliation failed at {path}");
                }
                LayerReconcileStatus::Ready | LayerReconcileStatus::Waiting => {}
            }
            app.update();
        }
        panic!("ordinary NPC restore never became ready");
    }
    let mut live = boot();
    let (npc, uuid) = named(&mut live, "Directive courier");
    let baseline = capture(live.world());
    let original = live
        .world()
        .get::<BehaviourSection>(npc)
        .unwrap()
        .0
        .doctrine
        .clone();
    load_layer(&mut live);
    apply(&mut live, 1, &uuid, "layer-north");
    live.update();
    let saved = capture(live.world());
    let encoded = ron::ser::to_string(&saved).unwrap();
    let saved = ron::from_str(&encoded).unwrap();
    let mut resumed = boot();
    let (other, _) = named(&mut resumed, "Directive courier");
    restore_ordinary(&mut resumed, &saved);
    assert_eq!(
        world_digest(live.world()),
        world_digest(resumed.world()),
        "{:?}",
        first_divergent_scope(resumed.world(), &digest_stages(live.world()))
    );
    for app in [&mut live, &mut resumed] {
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(LAYER.into()));
        app.update();
    }
    assert_eq!(doctrine(&live, npc), "layer-north");
    assert_eq!(doctrine(&resumed, other), "layer-north");
    assert!(resumed
        .world()
        .resource::<WorldContentRuntime>()
        .gm_npc_doctrine_palette
        .iter()
        .all(|row| row.id != "layer-north"));
    assert_eq!(world_digest(live.world()), world_digest(resumed.world()));
    for _ in 0..30 {
        live.update();
        resumed.update();
    }
    assert_eq!(world_digest(live.world()), world_digest(resumed.world()));
    restore_ordinary(&mut resumed, &baseline);
    assert!(resumed
        .world()
        .get::<NpcDoctrineState>(other)
        .unwrap()
        .0
        .is_none());
    assert_eq!(
        resumed
            .world()
            .get::<BehaviourSection>(other)
            .unwrap()
            .0
            .doctrine,
        original
    );
}

#[test]
fn seeded_recorded_replay_preserves_simultaneous_choices_duplicate_and_real_ai_result() {
    use phoenix::headless::{
        replay::{drive_run, drive_run_with_gm_actions},
        verify_artifact, ReplayArtifact,
    };
    let mut settings = args();
    settings.max_ticks = 300;
    let mut discovery = drive_run(&settings, &[], 25).unwrap();
    let (_, uuid) = named(discovery.app_mut(), "Directive courier");
    let mut planned = GmActionJournal::default();
    for (i, choice) in ["north", "north", "east"].iter().enumerate() {
        planned
            .insert(grant(i as u64 + 1, 120, &uuid, choice))
            .unwrap();
    }
    let mut recorded = drive_run_with_gm_actions(&settings, &[], &planned, 25).unwrap();
    let (npc, _) = named(recorded.app_mut(), "Directive courier");
    assert_eq!(doctrine(recorded.app_mut(), npc), "east");
    assert_eq!(
        recorded
            .app_mut()
            .world()
            .resource::<GmActionLog>()
            .entries()
            .iter()
            .map(|fact| fact.outcome)
            .collect::<Vec<_>>(),
        [
            GmActionOutcome::Applied,
            GmActionOutcome::NoOp,
            GmActionOutcome::Applied
        ]
    );
    let recorded_physics = *recorded
        .app_mut()
        .world()
        .get::<phoenix::ship::state::ShipPhysics>(npc)
        .unwrap();
    let artifact = ReplayArtifact::capture(
        &settings,
        recorded.recorded_log(),
        recorded.recorded_gm_actions(),
        recorded.tick(),
        recorded.seal(),
    )
    .unwrap();
    let artifact = ReplayArtifact::from_ron(&artifact.to_ron().unwrap()).unwrap();
    assert_eq!(verify_artifact(&artifact).unwrap(), None);
    let mut replayed = drive_run_with_gm_actions(
        &artifact.replay_args(),
        artifact.log.entries(),
        &artifact.gm_actions,
        25,
    )
    .unwrap();
    let (other, _) = named(replayed.app_mut(), "Directive courier");
    assert_eq!(doctrine(replayed.app_mut(), other), "east");
    assert_eq!(
        *replayed
            .app_mut()
            .world()
            .get::<phoenix::ship::state::ShipPhysics>(other)
            .unwrap(),
        recorded_physics
    );
    assert_eq!(replayed.seal().final_digest, artifact.ledger.final_digest);
    assert_ne!(discovery.seal().final_digest, artifact.ledger.final_digest);
}
