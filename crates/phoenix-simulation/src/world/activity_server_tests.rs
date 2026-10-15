//! Existing live trigger/effect pipeline consumes retained, scoped owner facts.
use super::*;
use phoenix_model::{
    activity::*,
    job::{JobId, JobOutcome},
};
use phoenix_sim_world::world::activities::*;

const SCRIPT: &str = r#"[script]
setup = '''
on_world_loaded("legacy");
fn legacy(ctx) { ctx.flags.increment("legacy", 1); }
fn useful(ctx) { ctx.flags.increment("credited", 1); }
fn settled(ctx) { ctx.flags.increment("settled", 1); }
fn refuse(ctx) { ctx.flags.increment("must_not_commit", 1); throw "refused"; }
'''
"#;

fn prepared(function: &str) -> App {
    let mut app = ai_trigger_test_app();
    let mut scripts = compile_fixture_scripts(SCRIPT);
    let script = scripts.asts.keys().next().unwrap().clone();
    let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
    merge_script_triggers(&mut runtime, &mut scripts, None);
    let definition = SituationDefinition {
        id: "delivery".into(),
        initial: "received".into(),
        state: vec![SituationState {
            id: "received".into(),
            when: SituationCondition::Received,
            call: Some(ScenarioCall {
                script,
                function: function.into(),
            }),
            next: None,
        }],
    };
    let config = SituationConfig {
        id: "supply".into(),
        definition: definition.id.clone(),
        request: ActivityRequest::Delivery {
            actor: "ship".into(),
            owner: SystemId("logistics".into()),
            source: "main".into(),
            destination: "local".into(),
            commodity: "parts".into(),
            amount: 2,
        },
    };
    runtime.situation_definitions = vec![definition.clone()];
    runtime.situation_configs = vec![config.clone()];
    runtime.situations.arm(&config, &definition, 1).unwrap();
    runtime
        .situations
        .accept_delivery(
            "ship",
            &SystemId("logistics".into()),
            "main",
            "local",
            "parts",
            2,
            JobId(1),
            2,
        )
        .unwrap();
    drop(runtime);
    app.insert_resource(scripts);
    app
}
fn receive(app: &mut App, tick: u64, returned: bool) {
    let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
    crate::world::activities::record(
        &mut runtime,
        "ship",
        &SystemId("logistics".into()),
        tick,
        [(
            JobId(1),
            ActivityOutcome::Delivery {
                source: "main".into(),
                destination: if returned { "main" } else { "local" }.into(),
                commodity: "parts".into(),
                amount: 2,
                returned,
                outcome: if returned {
                    JobOutcome::Cancelled
                } else {
                    JobOutcome::Completed
                },
            },
        )],
    );
}
fn value(app: &App, name: &str) -> i64 {
    app.world()
        .resource::<WorldContentRuntime>()
        .flags
        .counter(name)
}

#[test]
fn retained_terminal_fact_survives_delayed_consumer_without_replaying_legacy_events() {
    let mut app = prepared("useful");
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .pending_world_events
        .push(WorldEvent::WorldLoaded);
    receive(&mut app, 3, false);
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(value(&app, "legacy"), 1);
    assert_eq!(value(&app, "credited"), 0);
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(
        runtime.pending_world_events.len(),
        1,
        "bound fact remains in existing saved queue without a logical consumer clock"
    );
    assert!(app
        .world()
        .resource::<WorldEventBuffer>()
        .0
        .iter()
        .all(|event| !matches!(event, WorldEvent::Activity(_))));
    let pending = runtime.pending_world_events.clone();
    receive(&mut app, 7, false);
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .pending_world_events,
        pending,
        "retained original fact is not refreshed"
    );
    app.insert_resource(crate::sim_tick::SimTick(8));
    app.update();
    assert_eq!(value(&app, "credited"), 1);
    assert_eq!(value(&app, "legacy"), 1);
    let saved =
        serde_json::to_vec(&app.world().resource::<WorldContentRuntime>().situations).unwrap();
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .situations = serde_json::from_slice(&saved).unwrap();
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        value(&app, "credited"),
        1,
        "persisted applied activation cannot re-enter"
    );
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .pending_world_events
        .is_empty());
}

#[test]
fn returned_goods_do_not_satisfy_receive_and_sandbox_refusal_discards_buffered_effects_once() {
    let mut returned = prepared("useful");
    returned.insert_resource(crate::sim_tick::SimTick(3));
    receive(&mut returned, 3, true);
    returned.update();
    assert_eq!(value(&returned, "credited"), 0);
    assert!(returned
        .world()
        .resource::<WorldContentRuntime>()
        .situations
        .instances["supply"]
        .settled
        .is_some());
    let mut refused = prepared("refuse");
    refused.insert_resource(crate::sim_tick::SimTick(3));
    receive(&mut refused, 3, false);
    refused.update();
    assert_eq!(value(&refused, "must_not_commit"), 0);
    let frontier = refused
        .world()
        .resource::<WorldContentRuntime>()
        .situations
        .clone();
    assert_eq!(
        frontier.instances["supply"].activations["received"],
        ScenarioCallResult::Refused
    );
    refused.update();
    assert_eq!(
        refused.world().resource::<WorldContentRuntime>().situations,
        frontier
    );
    assert_eq!(value(&refused, "must_not_commit"), 0);
}

#[test]
fn shared_budget_postpones_ready_scenario_without_marking_it_applied_or_refused() {
    let mut app = prepared("useful");
    app.insert_resource(crate::sim_tick::SimTick(3));
    receive(&mut app, 3, false);
    let mut budget = crate::world::script::schedule::TickBudget::new();
    budget.charge_ops(crate::world::script::MAX_OPS_PER_TICK);
    app.world_mut()
        .resource_mut::<WorldScriptRuntime>()
        .seed_invocation_budget(3, budget);
    app.update();
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert!(runtime.situations.instances["supply"]
        .activations
        .is_empty());
    assert!(runtime.situations.instances["supply"].settled.is_some());
    assert_eq!(value(&app, "credited"), 0);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 4;
    app.update();
    assert_eq!(value(&app, "credited"), 1);
}

#[test]
fn authored_permission_uses_actual_effect_boundary_and_refused_effects_cannot_grant_it() {
    for (function, applied) in [("useful", true), ("refuse", false)] {
        let mut app = prepared(function);
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        let definition = &mut runtime.situation_definitions[0];
        definition.state[0].next = Some("permission".into());
        definition.state.push(SituationState {
            id: "permission".into(),
            when: SituationCondition::ScenarioApplied {
                state: "received".into(),
            },
            call: Some(ScenarioCall {
                script: definition.state[0].call.as_ref().unwrap().script.clone(),
                function: "settled".into(),
            }),
            next: None,
        });
        drop(runtime);
        receive(&mut app, 3, false);
        app.insert_resource(crate::sim_tick::SimTick(8));
        app.update();
        let runtime = app.world().resource::<WorldContentRuntime>();
        let instance = &runtime.situations.instances["supply"];
        assert_eq!(instance.settled.as_ref().unwrap().tick, 3);
        assert_eq!(instance.activation_ticks["received"], 8);
        assert_eq!(
            instance.activations["received"],
            if applied {
                ScenarioCallResult::Applied
            } else {
                ScenarioCallResult::Refused
            }
        );
        assert_eq!(
            instance.activation_ticks.get("permission").copied(),
            applied.then_some(8)
        );
        assert_eq!(runtime.flags.counter("settled"), i64::from(applied));
        runtime
            .situations
            .validate(&runtime.situation_definitions, 8)
            .unwrap();
        let before = runtime.situations.clone();
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<WorldContentRuntime>().situations,
            before
        );
    }
}
