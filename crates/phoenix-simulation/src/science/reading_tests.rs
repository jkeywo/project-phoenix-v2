use super::*;
use crate::core::messages::*;
use bevy::ecs::system::RunSystemOnce;

fn rig() -> (App, Entity, SystemId) {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    for profile in &mut config.sensing.as_mut().unwrap().focused {
        profile.reading_ticks = 2;
    }
    let owner = SystemId("sensors".into());
    let scans = super::super::server::ShipScanRecord::from_config(&config).unwrap();
    let (_, runtimes) = crate::ai::state_tree_runtime::compile_ship_trees_with_energy(
        config.ship_config.as_ref().unwrap(),
        config.energy.as_ref(),
    )
    .unwrap();
    let mut sources = crate::ship::control_source::ControlSourceResolver::default();
    sources.set(
        owner.clone(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let mut boards = ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            scored_objectives: vec![ScoredObjective {
                id: "held-request".into(),
                score: 1.0,
                directive: AiDirective::Scan {
                    target: "public-request".into(),
                },
                source: ObjectiveSource::Mission,
                relevance: vec![SystemAffinity::Sensors],
                snapshot: ObjectiveSnapshot {
                    id: "held-request".into(),
                    text: "request.text".into(),
                    text_params: Default::default(),
                    mandatory: false,
                    status: ObjectiveStatus::Active,
                    unassigned: false,
                    progress: None,
                    targets: vec!["public-request".into()],
                    source: ObjectiveSource::Mission,
                },
            }],
            ..Default::default()
        }),
    );
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(0));
    let entity = app
        .world_mut()
        .spawn((
            ShipConfigComponent(config.ship_config.unwrap()),
            ShipSystemControlSources(sources),
            boards,
            scans,
            runtimes,
        ))
        .id();
    (app, entity, owner)
}
fn advance_at(app: &mut App, tick: u64) {
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
    app.world_mut().run_system_once(advance).unwrap();
}
fn ready(app: &App, entity: Entity, owner: &SystemId, tick: u64) -> bool {
    let runtime = &app
        .world()
        .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(entity)
        .unwrap()
        .0[owner];
    request_ready(
        runtime,
        &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
        app.world().get::<ShipSystemBlackboards>(entity).unwrap(),
        owner,
        "public-request",
        tick,
        2,
    )
}

#[test]
fn actual_system_reading_pauses_handoff_and_semantic_revision_restarts_without_tree_execution() {
    let (mut app, entity, owner) = rig();
    advance_at(&mut app, 0);
    assert!(!ready(&app, entity, &owner, 0));
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Human,
        );
    advance_at(&mut app, 1);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    advance_at(&mut app, 2);
    assert!(!ready(&app, entity, &owner, 2));
    advance_at(&mut app, 3);
    assert!(ready(&app, entity, &owner, 3));
    let runtime = &app
        .world()
        .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(entity)
        .unwrap()
        .0[&owner];
    assert!(
        runtime.root.is_none(),
        "reading updater is not another tree/controller"
    );
    let json = serde_json::to_string(runtime).unwrap();
    let restored: phoenix_sim_gameplay::ai::state_tree::TreeRuntime =
        serde_json::from_str(&json).unwrap();
    assert_eq!(&restored, runtime);
    let source = crate::ship::system_registry::viewscreen_system_id();
    if let SystemBlackboard::Viewscreen(board) = app
        .world_mut()
        .get_mut::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .get_mut(&source)
        .unwrap()
    {
        board.scored_objectives[0].score = 9.0;
    }
    advance_at(&mut app, 4);
    assert!(
        ready(&app, entity, &owner, 4),
        "score/duplicate delivery is not new text"
    );
    if let SystemBlackboard::Viewscreen(board) = app
        .world_mut()
        .get_mut::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .get_mut(&source)
        .unwrap()
    {
        board.scored_objectives[0].snapshot.text = "changed.request.text".into();
    }
    advance_at(&mut app, 5);
    assert!(!ready(&app, entity, &owner, 5));
}

#[test]
fn revoked_typed_feed_does_not_earn_or_use_relevant_text() {
    let (mut app, entity, owner) = rig();
    advance_at(&mut app, 0);
    let console = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .system(&owner)
        .unwrap()
        .console
        .clone()
        .unwrap();
    app.world_mut()
        .get_mut::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .retain(|feed| feed.kind != phoenix_model::console::ConsoleObservationKind::Viewscreen);
    for tick in 1..=5 {
        advance_at(&mut app, tick);
        assert!(!ready(&app, entity, &owner, tick));
    }
    assert_eq!(
        app.world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(entity)
            .unwrap()
            .0[&owner]
            .reading
            .as_ref()
            .unwrap()
            .completed_ticks,
        0
    );
}
