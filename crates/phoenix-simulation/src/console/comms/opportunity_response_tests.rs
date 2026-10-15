//! Actual admission/response guard tests. The typed offered-record precondition
//! tests Comms decision authority, not discovery or physical-plan completion.
use super::*;
use crate::world::opportunities::{OpportunityRecord, OpportunityState};
use phoenix_model::comms_request::{OpportunityDecision, OpportunityRef};
use phoenix_sim_world::world::planning::opportunities::{
    MaintenanceOpportunityInput, MaintenanceSearchResult, OperatorInvocation,
};
const ACTOR: &str = "comms-fixture-ship";
fn setup(callback: &str) -> (App, String, OpportunityRef) {
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, "ordinary-external-contact");
    app.insert_resource(crate::sim_tick::SimTick(100));
    app.insert_resource(crate::comms::scripted::tests::compile_fixture(callback));
    let message = seat_scripted_dialogue(
        &mut app,
        "_self",
        "Own maintenance report",
        vec!["choose"],
        false,
    );
    let reference = OpportunityRef {
        source: "maintenance".into(),
        ordinal: 1,
        actor: ACTOR.into(),
    };
    let mut received = app
        .world()
        .resource::<CommsInboxRes>()
        .0
        .messages()
        .into_iter()
        .find(|row| row.id == message)
        .unwrap();
    received.recipient_ship = Some(crate::command_admission::log::ShipKey(ACTOR.into()));
    {
        let mut inbox = app.world_mut().resource_mut::<CommsInboxRes>();
        inbox.0.remove(&message);
        assert!(inbox.0.inject(received));
    }
    {
        let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
        let script = &mut comms.active_dialogues.get_mut(&message).unwrap().script;
        script.opportunity = Some(reference.clone());
        script.recipient_ship = Some(crate::command_admission::log::ShipKey(ACTOR.into()));
        comms.range_active = true;
        comms.range_flags.clear();
        comms.fleet_range_active = true;
        comms.fleet_range_flags.clear();
        assert!(!crate::comms::server::slot_may_answer_sender(
            &comms,
            Some(crate::command_admission::HostSlot::SOLO),
            "_self"
        ));
    }
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
    (app, message, reference)
}
const CLEAN: &str = r#"fn choose(ctx) {#{message:"Decision requested",opportunity_decision:"accept",responses:[]}}"#;
#[test]
fn bound_internal_reply_requests_pending_then_duplicate_is_refused_without_ledger() {
    let (mut app, message, _) = setup(CLEAN);
    let out = respond(&mut app, &message, 0);
    assert!(find_rejection(&out).is_none());
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert!(matches!(
        runtime.opportunities[0].state,
        OpportunityState::Pending {
            decision: OpportunityDecision::Accept,
            tick: 100
        }
    ));
    assert!(runtime.commitments.is_empty());
    assert!(
        runtime.situations.instances.is_empty(),
        "Comms acceptance creates no physical job/situation"
    );
    let record = runtime.opportunities[0].clone();
    let out = respond(&mut app, &message, 0);
    assert!(find_rejection(&out).is_some());
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(runtime.opportunities[0], record);
    assert!(runtime.commitments.is_empty());
}
#[test]
fn commitment_bearing_bound_decision_refuses_before_pending_or_any_buffered_effect() {
    let (mut app, message, _) = setup(
        r#"fn choose(ctx){ctx.flags.forbidden=1;ctx.commitments.record(#{id:"premature",made_to:"crew",terms:"terms",resolves_when:"restored"});#{message:"Decision requested",opportunity_decision:"accept",responses:[]}}"#,
    );
    let before = app
        .world()
        .resource::<WorldContentRuntime>()
        .opportunities
        .clone();
    let ledger = app
        .world()
        .resource::<WorldContentRuntime>()
        .commitments
        .clone();
    let out = respond(&mut app, &message, 0);
    assert!(find_rejection(&out).is_some());
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(runtime.opportunities, before);
    assert_eq!(runtime.commitments, ledger);
    assert_eq!(runtime.flags.counter("forbidden"), 0);
    assert_eq!(
        app.world()
            .resource::<CommsInboxRes>()
            .0
            .messages()
            .iter()
            .find(|row| row.id == message)
            .unwrap()
            .selected_response,
        None
    );
}
#[test]
fn stale_foreign_unbound_duplicate_and_decided_internal_offers_never_pay_or_run() {
    for fault in 0..6 {
        let (mut app, message, reference) = setup(
            r#"fn choose(ctx){ctx.flags.forbidden=1;#{message:"Decision requested",opportunity_decision:"accept",responses:[]}}"#,
        );
        if fault < 3 {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            let script = &mut comms.active_dialogues.get_mut(&message).unwrap().script;
            match fault {
                0 => script.opportunity.as_mut().unwrap().ordinal = 2,
                1 => script.opportunity.as_mut().unwrap().actor = "foreign-ship".into(),
                _ => script.opportunity = None,
            }
        } else {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            match fault {
                3 => {
                    let record = runtime.opportunities[0].clone();
                    runtime.opportunities.push(record);
                }
                4 => {
                    runtime.opportunities[0].state = OpportunityState::Pending {
                        decision: OpportunityDecision::Accept,
                        tick: 99,
                    }
                }
                _ => {
                    runtime.opportunities[0].state = OpportunityState::Refused {
                        decision_tick: 98,
                        tick: 99,
                        reason: "withdrawn".into(),
                    }
                }
            }
        }
        let before = app
            .world()
            .resource::<WorldContentRuntime>()
            .opportunities
            .clone();
        let mut query = app
            .world_mut()
            .query::<&crate::ship::power::ShipPowerSystem>();
        let power = query.single(app.world()).unwrap().0.capture_continuation();
        let out = respond(&mut app, &message, 0);
        assert!(find_rejection(&out).is_some(), "fault{fault}");
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(runtime.opportunities, before);
        assert!(runtime.commitments.is_empty());
        assert_eq!(runtime.flags.counter("forbidden"), 0);
        let mut query = app
            .world_mut()
            .query::<&crate::ship::power::ShipPowerSystem>();
        assert_eq!(
            query.single(app.world()).unwrap().0.capture_continuation(),
            power,
            "fault{fault} refused before payment"
        );
        assert_eq!(reference.ordinal, 1);
    }
}
#[test]
fn ordinary_comms_ai_uses_the_same_bound_internal_reply_and_pending_owner() {
    let (mut app, message, _) = setup(CLEAN);
    app.add_systems(
        FixedUpdate,
        operate_comms_response_ai
            .before(handle_respond_to_message)
            .after(crate::server_app::AdmissionSet),
    );
    let mut query = app
        .world_mut()
        .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<LocalShip>>();
    query.single_mut(app.world_mut()).unwrap().0.set(
        crate::ship::system_registry::comms_system_id(),
        ControlSource::Ai,
    );
    let out = tick(&mut app);
    assert!(find_rejection(&out).is_none());
    assert!(matches!(
        app.world().resource::<WorldContentRuntime>().opportunities[0].state,
        OpportunityState::Pending {
            decision: OpportunityDecision::Accept,
            tick: 100
        }
    ));
    assert_eq!(
        app.world()
            .resource::<CommsInboxRes>()
            .0
            .messages()
            .iter()
            .find(|row| row.id == message)
            .unwrap()
            .selected_response,
        Some(0)
    );
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .commitments
        .is_empty());
}
