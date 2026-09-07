//! Objective layer ownership follows the ordinary headless boot and snapshot
//! continuation contract. Like snapshot_resume.rs, this has its own binary so
//! another lib test cannot choose Bevy's process-global task-pool configuration.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::command_admission::HostSlot;
use project_phoenix::gm_action::{
    GmAction, GmActionGrant, GmActionId, GmActionJournal, GmActionOrder,
};
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::sim_digest::{digest_stages, first_divergent_scope, world_digest};
use project_phoenix::sim_tick::SimTick;
use project_phoenix::snapshot::{capture, restore};
use project_phoenix::world::server::{
    ObjectiveManagerRes, PendingWorldLayerChanges, WorldContentRuntime, WorldLayerChange,
    WorldLayerMap,
};

const LAYER: &str = "tests/fixtures/gm_objective_layer.toml";

fn boot() -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/worlds/layer_dynamic_resume.toml".into(),
        seed: Some(1307),
        deterministic: true,
        ..Default::default()
    })
    .expect("ordinary headless world builds");
    app.finish();
    app.cleanup();
    // The same stable running-mission boundary used by snapshot_resume.rs.
    for _ in 0..400 {
        app.update();
    }
    assert_eq!(
        *app.world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get(),
        project_phoenix::core::messages::GamePhase::InProgress
    );
    app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
        "base-obj",
        "objective.base",
        false,
        vec![],
    );
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
            assert_eq!(
                app.world()
                    .resource::<WorldContentRuntime>()
                    .gm_objective_palette[0]
                    .origin_layer
                    .as_deref(),
                Some(LAYER)
            );
            return app;
        }
    }
    panic!("Objective layer did not activate");
}

#[test]
fn restored_gm_objective_layer_ownership_matches_uninterrupted_unload() {
    let mut uninterrupted = boot();
    let host = HostSlot(1);
    let apply_tick = uninterrupted.world().resource::<SimTick>().0;
    uninterrupted
        .world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(GmActionGrant {
            from: host,
            sequenced_by: host,
            operator_id: "gm-one".into(),
            correlation: GmActionId::new("layer-activate").unwrap(),
            recovery_generation: 0,
            apply_tick,
            order: GmActionOrder::new(host, 1),
            action: GmAction::ObjectiveAction {
                objective: "layer-escort".into(),
                verb: project_phoenix::gm_objective::ObjectiveVerb::Activate,
                recipients: vec![],
            },
        })
        .unwrap();
    uninterrupted.update();
    assert_eq!(
        uninterrupted.world().resource::<WorldLayerMap>().0[LAYER].owned_objective_ids,
        ["layer-escort"]
    );
    let saved = capture(uninterrupted.world());
    assert_eq!(
        saved
            .layer_flags
            .iter()
            .find(|layer| layer.path == LAYER)
            .unwrap()
            .owned_objective_ids,
        ["layer-escort"]
    );
    let saved_digest = world_digest(uninterrupted.world());
    // Ownership alone is authoritative before unload creates any other change.
    uninterrupted
        .world_mut()
        .resource_mut::<WorldLayerMap>()
        .0
        .get_mut(LAYER)
        .unwrap()
        .owned_objective_ids
        .clear();
    assert_ne!(world_digest(uninterrupted.world()), saved_digest);
    uninterrupted
        .world_mut()
        .resource_mut::<WorldLayerMap>()
        .0
        .get_mut(LAYER)
        .unwrap()
        .owned_objective_ids = vec!["layer-escort".into()];

    let mut resumed = boot();
    assert!(resumed.world().resource::<WorldLayerMap>().0[LAYER]
        .owned_objective_ids
        .is_empty());
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "restore gaps: {:?}", report.gaps);
    assert_eq!(
        world_digest(resumed.world()),
        saved_digest,
        "first divergent scope: {:?}",
        first_divergent_scope(resumed.world(), &digest_stages(uninterrupted.world()))
    );
    for app in [&mut uninterrupted, &mut resumed] {
        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert_eq!(objectives.active_station_stances().len(), 1);
        assert!(objectives
            .scored_pool_for(&Default::default(), "ship-a")
            .iter()
            .any(|entry| entry.snapshot.id == "layer-escort"));
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(LAYER.into()));
    }
    // Both run the ordinary continuation schedule, including real layer unload.
    uninterrupted.update();
    resumed.update();
    for app in [&uninterrupted, &resumed] {
        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(objectives.status("layer-escort").is_none());
        assert!(objectives.status("base-obj").is_some());
        assert!(objectives.active_station_stances().is_empty());
        assert!(app
            .world()
            .resource::<WorldContentRuntime>()
            .gm_objective_palette
            .is_empty());
        assert!(!app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .contains_key(LAYER));
    }
    assert_eq!(
        world_digest(uninterrupted.world()),
        world_digest(resumed.world()),
        "first divergent scope after unload: {:?}",
        first_divergent_scope(resumed.world(), &digest_stages(uninterrupted.world()))
    );
}
