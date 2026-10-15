use super::*;
use phoenix_model::{
    activity::*,
    identity::SystemId,
    job::{JobId, JobOutcome},
};
use phoenix_sim_world::world::activities::{
    ActivityRequest, ScenarioCallResult, SituationInstance,
};

#[test]
fn author_inspection_preserves_exact_job_and_pending_vs_acknowledged_frontiers() {
    let mut runtime = WorldContentRuntime::default();
    let job = ActivityJob {
        actor: "actual-ship".into(),
        owner: SystemId("handling".into()),
        job: JobId(7),
    };
    let fact = ActivityFact {
        instance: "supply".into(),
        scope: job.clone(),
        tick: 14,
        outcome: ActivityOutcome::Delivery {
            source: "main".into(),
            destination: "magazine".into(),
            commodity: "rounds".into(),
            amount: 2,
            returned: false,
            outcome: JobOutcome::Completed,
        },
    };
    runtime.situations.instances.insert(
        "supply".into(),
        SituationInstance {
            definition: "receipt".into(),
            request: ActivityRequest::Delivery {
                actor: "authored-ship".into(),
                owner: job.owner.clone(),
                source: "main".into(),
                destination: "magazine".into(),
                commodity: "rounds".into(),
                amount: 2,
            },
            armed_tick: 2,
            accepted: Some(job),
            accepted_tick: Some(6),
            achieved: None,
            settled: None,
            state: Some("received".into()),
            activations: Default::default(),
        },
    );
    runtime
        .pending_world_events
        .push(crate::world::content::WorldEvent::Activity(fact.clone()));
    let before = runtime.situations.clone();
    let pending = runtime.pending_world_events.clone();
    let projected = collect_scenario_state(&runtime, &ObjectiveManager::default());
    assert_eq!(
        projected.situations[0].lifecycle,
        before.instances["supply"]
    );
    assert_eq!(projected.situations[0].pending, vec![fact.clone()]);
    assert!(projected.situations[0].lifecycle.settled.is_none());
    assert_eq!(runtime.situations, before);
    assert_eq!(runtime.pending_world_events, pending);
    // Inspection reports what the real World consumer committed; it does not
    // infer success from an accepted request or promote a pending fact.
    let state = runtime.situations.instances.get_mut("supply").unwrap();
    state.settled = Some(fact);
    state
        .activations
        .insert("received".into(), ScenarioCallResult::Refused);
    state.state = None;
    runtime.pending_world_events.clear();
    let projected = collect_scenario_state(&runtime, &ObjectiveManager::default());
    assert_eq!(
        projected.situations[0]
            .lifecycle
            .settled
            .as_ref()
            .unwrap()
            .tick,
        14
    );
    assert_eq!(
        projected.situations[0].lifecycle.activations["received"],
        ScenarioCallResult::Refused
    );
    assert!(projected.situations[0].pending.is_empty());
}

#[test]
fn original_plan_choice_and_derived_graph_inspection_do_not_release_or_acknowledge_work() {
    let source =
        crate::content_fs::read_to_string("tests/fixtures/maintenance-htn-world.toml").unwrap();
    let mut base = crate::world::config::parse_world(&source).unwrap();
    let (compiled, _) = phoenix_sim_world::world::planning::CompiledPlanning::load(
        base.planning.clone().unwrap(),
        |path| crate::content_fs::read_to_string(path).ok(),
    )
    .unwrap();
    let selected = compiled
        .choose("actual-ship", 10, |request| match request {
            ActivityRequest::Repair { team: 1, .. } => Err("requires unavailable team1".into()),
            _ => Ok(()),
        })
        .unwrap();
    let mut runtime = WorldContentRuntime::default();
    let mut choice = selected.choice;
    compiled
        .release(&mut choice, &mut runtime.situations, "actual-ship", 11)
        .unwrap();
    runtime.plan_choice = Some(choice.clone());
    base.compiled_planning = Some(compiled);
    let before = runtime.situations.clone();
    let payload = collect_scenario_state_with_plan(
        &runtime,
        &ObjectiveManager::default(),
        &TriggerFireRecorder::default(),
        Some(&base),
    );
    let plan = payload.planning.unwrap();
    assert_eq!(plan.choice, Some(choice.clone()));
    assert_eq!(plan.leaves.len(), 2);
    assert_eq!(plan.dependencies.len(), 1);
    assert_eq!(
        plan.leaves
            .iter()
            .filter(|leaf| leaf.released_tick.is_some())
            .count(),
        1
    );
    assert!(plan.choice.as_ref().unwrap().refused_methods[0].contains("unavailable team1"));
    assert!(plan.derivation_error.is_none());
    assert_eq!(runtime.plan_choice, Some(choice));
    assert_eq!(runtime.situations, before);
    assert!(runtime.pending_world_events.is_empty());
    runtime.plan_choice = None;
    let waiting = collect_scenario_state_with_plan(
        &runtime,
        &ObjectiveManager::default(),
        &TriggerFireRecorder::default(),
        Some(&base),
    )
    .planning
    .unwrap();
    assert!(waiting.choice.is_none() && waiting.leaves.is_empty());
}
