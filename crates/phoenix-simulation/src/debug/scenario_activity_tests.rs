use super::*;
use phoenix_sim_world::world::activities::{ActivityRequest, ScenarioCallResult, SituationInstance};
use phoenix_model::{
    activity::*,
    identity::SystemId,
    job::{JobId, JobOutcome},
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
