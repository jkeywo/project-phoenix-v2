//! End-to-end combat balance regressions use the ordinary headless host.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::core::balance::RunOutcome;
use project_phoenix::core::messages::GamePhase;
use project_phoenix::headless::{build_headless_app, build_report, run, HeadlessArgs};

/// CLI duel rosters use the same recoverable-defeat condition as the authored
/// arena. One disabled opponent is insufficient; the last neutralization ends
/// the fight without inventing deaths or removing recoverable ships.
#[test]
fn generated_duel_victory_counts_disabled_opponents_without_destruction() {
    use project_phoenix::ai::server::AiEntityDisabled;
    use project_phoenix::entities::spawner::EntityUuid;
    use project_phoenix::world::server::WorldContentRuntime;

    let args = HeadlessArgs {
        world_path: "assets/worlds/duel.toml".into(),
        side_a: vec!["cruiser".into()],
        side_b: vec!["ship_harrow_destroyer".into(); 2],
        seed: Some(1),
        deterministic: true,
        max_ticks: 30,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).expect("CLI duel loads");
    run(&mut app, 10);
    let opponents: Vec<String> = {
        let world = app.world().resource::<WorldContentRuntime>();
        ["side_b_1", "side_b_2"]
            .iter()
            .map(|name| world.name_to_uuid[*name].clone())
            .collect()
    };
    for (index, uuid) in opponents.iter().enumerate() {
        app.world_mut()
            .resource_mut::<Messages<AiEntityDisabled>>()
            .write(AiEntityDisabled {
                entity_uuid: uuid.clone(),
                disabled: true,
            });
        run(&mut app, 5);
        if index == 0 {
            assert_eq!(
                app.world().resource::<State<GamePhase>>().get(),
                &GamePhase::InProgress,
                "a remaining opponent prevents victory"
            );
        }
    }
    let report = build_report(&mut app, &args, 0.0);
    assert_eq!(report.outcome_report.outcome, RunOutcome::Victory);
    assert!(report
        .damage_by_ship
        .values()
        .all(|ship| ship.death.is_none()));
    let mut entities = app.world_mut().query::<&EntityUuid>();
    for uuid in opponents {
        assert!(entities.iter(app.world()).any(|entity| entity.0 == uuid));
    }
}

fn shield_state(world: &mut World) -> std::collections::BTreeMap<String, serde_json::Value> {
    use project_phoenix::ai::state_tree_runtime::ShipAiTreeRuntime;
    use project_phoenix::core::messages::SystemBlackboard;
    use project_phoenix::entities::spawner::EntityUuid;
    use project_phoenix::server_app::{ShipShields, ShipSystemBlackboards};
    let mut query = world.query::<(
        &EntityUuid,
        &ShipShields,
        &ShipSystemBlackboards,
        &ShipAiTreeRuntime,
    )>();
    query
        .iter(world)
        .map(|(uuid, shields, boards, trees)| {
            let arcs: Vec<_> = shields
                .0
                .facings
                .iter()
                .map(|arc| {
                    (
                        arc.id.clone(),
                        arc.focus_available,
                        arc.hp,
                        arc.hp_frac(),
                        arc.offline_remaining,
                        arc.is_focused,
                    )
                })
                .collect();
            let available: std::collections::BTreeMap<_, _> = boards
                .0
                .iter()
                .filter_map(|(id, board)| {
                    let SystemBlackboard::ShieldArc(arc) = board else {
                        return None;
                    };
                    Some((
                        id,
                        arc.operation
                            .as_ref()
                            .map(|operation| operation.focus_available),
                    ))
                })
                .collect();
            let memories: std::collections::BTreeMap<_, _> = trees
                .0
                .iter()
                .filter(|(id, _)| available.contains_key(id))
                .collect();
            (
                uuid.0.clone(),
                serde_json::json!({"arcs": arcs, "available": available, "memories": memories}),
            )
        })
        .collect()
}

#[test]
fn shield_capabilities_charge_and_tree_memory_survive_a_fresh_host_resume() {
    use project_phoenix::server_app::ShipShields;
    use project_phoenix::snapshot::{
        capture, ready_to_restore, reconcile_world_layers, restore, LayerReconcileStatus,
        PhoenixSnapshot,
    };
    let args = HeadlessArgs {
        world_path: "assets/worlds/duel.toml".into(),
        side_a: vec!["cruiser".into()],
        side_b: vec!["ship_harrow_destroyer".into()],
        seed: Some(1),
        deterministic: true,
        ..Default::default()
    };
    let mut live = build_headless_app(&args).unwrap();
    run(&mut live, 60);
    // Prepare real, non-full charge and supported focus. Capability is read
    // from authored equipment, never inferred from the number of arcs.
    let mut shields = live.world_mut().query::<&mut ShipShields>();
    let mut changed = 0;
    for mut ship in shields.iter_mut(live.world_mut()) {
        let focus = ship.0.facings.iter().position(|arc| arc.focus_available);
        ship.0.set_focused_facing(focus);
        if let Some(arc) = ship.0.facings.first_mut() {
            arc.hp = (arc.max_hp / 3).max(1);
            changed += 1;
        }
    }
    assert!(changed >= 2, "both combatants carry charge to resume");
    run(&mut live, 3);
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let expected = shield_state(live.world_mut());
    assert!(expected.len() >= 2);
    let mut resumed = build_headless_app(&args).unwrap();
    resumed.finish();
    resumed.cleanup();
    let mut ready = false;
    for _ in 0..1000 {
        resumed.update();
        match reconcile_world_layers(resumed.world_mut(), &saved) {
            LayerReconcileStatus::Ready if ready_to_restore(resumed.world(), &saved) => {
                ready = true;
                break;
            }
            LayerReconcileStatus::Failed(path) => panic!("world reconciliation failed: {path}"),
            _ => {}
        }
    }
    assert!(ready, "fresh host reaches the restore boundary");
    let restored = restore(resumed.world_mut(), &saved);
    assert!(restored.is_complete(), "restore gaps: {:?}", restored.gaps);
    assert_eq!(shield_state(resumed.world_mut()), expected);
    for step in 1..=30 {
        live.update();
        resumed.update();
        assert_eq!(
            shield_state(resumed.world_mut()),
            shield_state(live.world_mut()),
            "shield continuation at step {step}"
        );
    }
}
