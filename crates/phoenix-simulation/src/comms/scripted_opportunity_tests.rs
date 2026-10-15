//! Typed Offered state is a precondition for entry-owner failure tests. Actual
//! source discovery and physical work are covered by the full-host consumer.
use super::*;
use crate::world::opportunities::{OpportunityRecord, OpportunitySourceFailure, OpportunityState};
use phoenix_model::comms_request::OpportunityRef;
use phoenix_sim_world::world::planning::opportunities::{
    MaintenanceOpportunityInput, MaintenanceSearchResult, OperatorInvocation,
};

fn fixture(body: &str) -> (App, OpenCommsRequest) {
    let mut app = recipient_comms_app();
    app.insert_resource(crate::sim_tick::SimTick(100));
    app.insert_resource(compile_fixture(body));
    let reference = OpportunityRef {
        source: "maintenance".into(),
        ordinal: 1,
        actor: "lead".into(),
    };
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .opportunities
        .push(OpportunityRecord {
            reference: reference.clone(),
            discovered_tick: 90,
            input: MaintenanceOpportunityInput {
                compartment: "engineering".into(),
                hp: 80.0,
                max_hp: 100.0,
                remedy_hp: 20.0,
                required_parts: 2,
                local_available: 2,
                local_headroom: 8,
                source_available_above_floor: 4,
                known: true,
                report: None,
                repair_idle: true,
                handling_idle: true,
                route_available: true,
                outstanding_supply: false,
            },
            proposal: MaintenanceSearchResult {
                steps: vec![OperatorInvocation {
                    task: "maintenance.restore-room".into(),
                    arguments: Default::default(),
                }],
                expansions: 1,
            },
            state: OpportunityState::Offered,
        });
    let open = OpenCommsRequest {
        opportunity: Some(reference.clone()),
        recipient_ship: Some(crate::command_admission::log::ShipKey("lead".into())),
        thread_id: Some(reference.instance_id()),
        ..request("offer", "_self")
    };
    (app, open)
}
#[test]
fn actual_final_root_failures_keep_typed_source_outcome_and_commit_no_buffered_effects() {
    for (index,body,cause) in [
        (0,"fn offer(ctx) {ctx.flags.forbidden=1;}",OpportunitySourceFailure::NoNode),
        (1,"fn offer(ctx) {ctx.flags.forbidden=1;42}",OpportunitySourceFailure::Shape),
        (2,"fn offer(ctx) {ctx.flags.forbidden=1;throw \"refused\";}",OpportunitySourceFailure::Refused),
        (3,"fn offer(ctx) {#{message:\"Available\",responses:[]}}",OpportunitySourceFailure::Unresolved),
        (4,"fn offer(ctx) {#{message:\"Available\",responses:[]}}",OpportunitySourceFailure::MissingUnit),
        (5,"fn offer(ctx) {ctx.flags.forbidden=1;#{message:\"Unavailable\",opportunity_decision:\"accept\",responses:[]}}",OpportunitySourceFailure::Refused),
        (6,"fn offer(ctx) {ctx.flags.forbidden=1;#{message:\"No choices\",responses:[]}}",OpportunitySourceFailure::Shape),
        (7,"fn offer(ctx) {ctx.flags.forbidden=1;#{message:\"No callable choice\",responses:[#{text:\"Choose\",on_pick:\"absent\"}]}}",OpportunitySourceFailure::Unresolved),
    ] {
        let (mut app,mut open)=fixture(body);
        if index==3 {open.root_fn="missing".into();}
        if index==4 {open.script_path="missing.rhai".into();}
        app.world_mut().resource_mut::<WorldScriptRuntime>().pending_comms_opens.push(open);
        app.update();
        let runtime=app.world().resource::<WorldContentRuntime>();
        assert!(matches!(runtime.opportunities[0].state,OpportunityState::SourceUnavailable{tick:100,cause:actual} if actual==cause),"failure{index}");
        assert_eq!(runtime.flags.counter("forbidden"),0,"failure{index} discards buffers");
        assert!(runtime.commitments.is_empty());assert!(runtime.situations.is_empty());
        assert!(app.world().resource::<WorldScriptRuntime>().pending_comms_opens.is_empty());
        assert!(app.world().resource::<CommsRuntime>().active_dialogues.is_empty());
    }
}
#[test]
fn budget_deferral_keeps_same_original_offer_and_legacy_tail_policy_then_enters_once() {
    let (mut app, open) = fixture(
        "fn offer(ctx) {ctx.flags.increment(\"entered\",1);#{message:\"Available\",responses:[#{text:\"Choose\",on_pick:\"choose\"}]}} fn choose(ctx){#{message:\"Decision requested\",opportunity_decision:\"accept\",responses:[]}}",
    );
    let original = serde_json::to_value(&open).unwrap();
    {
        let mut script = app.world_mut().resource_mut::<WorldScriptRuntime>();
        script.prepare_invocation_tick(100);
        script
            .budget
            .charge_ops(crate::world::script::MAX_OPS_PER_TICK);
        // A legacy refusal first must not drop a later actual offer.
        script.pending_comms_opens.push(request("offer", "legacy"));
        script.pending_comms_opens.push(open);
        script
            .pending_comms_opens
            .push(request("offer", "legacy-tail"));
    }
    app.update();
    let script = app.world().resource::<WorldScriptRuntime>();
    assert_eq!(script.pending_comms_opens.len(), 1);
    assert_eq!(
        serde_json::to_value(&script.pending_comms_opens[0]).unwrap(),
        original
    );
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert!(matches!(
        runtime.opportunities[0].state,
        OpportunityState::Offered
    ));
    assert_eq!(runtime.flags.counter("entered"), 0);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 101;
    app.update();
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("entered"),
        1
    );
    assert!(app
        .world()
        .resource::<WorldScriptRuntime>()
        .pending_comms_opens
        .is_empty());
    assert_eq!(
        app.world()
            .resource::<CommsRuntime>()
            .active_dialogues
            .len(),
        1
    );
}
