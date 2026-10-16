use super::*;
fn owner() -> SystemId {
    SystemId("probe-bay".into())
}
fn profile() -> BayConfig {
    BayConfig {
        compartment: "bay".into(),
        capacity_space: 2,
        launch_work_secs: 2.0,
        recovery_work_secs: 3.0,
        recovery_range: 8.0,
        maximum_parent_speed: 0.1,
        release_offset: [0.0, 0.0, -3.0],
    }
}
fn started() -> BayState {
    let mut state = BayState::default();
    state
        .begin(
            &owner(),
            "actual-probe",
            BayOperation::Launch,
            &profile(),
            1.0,
            10,
        )
        .unwrap();
    state
}
#[test]
fn actual_job_paid_fraction_pause_and_duplicate_tick_preserve_one_progress_owner() {
    let mut state = started();
    let mut paid = 0.0f64;
    assert!(!state
        .advance(&owner(), 10, 1.0, 1.0, |_, _| panic!(
            "same tick cannot pay"
        ))
        .unwrap());
    assert!(!state
        .advance(&owner(), 11, 1.0, 0.5, |rate, seconds| {
            paid += f64::from(rate) * f64::from(seconds) * 0.5;
            0.5
        })
        .unwrap());
    assert_eq!(state.job.as_ref().unwrap().activity.worked_secs, 0.25);
    let id = state.job.as_ref().unwrap().id;
    state
        .advance(&owner(), 12, 1.0, 0.0, |_, _| {
            panic!("unavailable cannot pay")
        })
        .unwrap();
    let before = state.clone();
    assert!(state
        .begin(&owner(), "other", BayOperation::Launch, &profile(), 1.0, 12)
        .is_err());
    assert_eq!(state, before);
    state
        .begin(
            &owner(),
            "actual-probe",
            BayOperation::Launch,
            &profile(),
            1.0,
            12,
        )
        .unwrap();
    assert_eq!(state.job.as_ref().unwrap().id, id);
    assert!(!state
        .advance(&owner(), 13, 1.0, 0.0, |_, _| panic!(
            "unavailable cannot pay"
        ))
        .unwrap());
    for tick in 14..21 {
        state
            .advance(&owner(), tick, 0.25, 1.0, |rate, seconds| {
                paid += f64::from(rate) * f64::from(seconds);
                1.0
            })
            .unwrap();
    }
    assert!(state.job.as_ref().unwrap().activity.ready());
    assert_eq!(paid, 2.0);
    assert!(state
        .advance(&owner(), 21, 1.0, 1.0, |_, _| panic!(
            "waiting placement cannot pay again"
        ))
        .unwrap());
    assert!(state.complete());
    assert!(!state.complete());
    state.validate(&owner(), 21).unwrap();
}
#[test]
fn json_partial_job_continues_exactly_and_malformed_frontier_refuses_without_writes() {
    let mut live = started();
    live.advance(&owner(), 11, 0.25, 1.0, |_, _| 1.0).unwrap();
    let mut restored: BayState =
        serde_json::from_slice(&serde_json::to_vec(&live).unwrap()).unwrap();
    for tick in 12..372 {
        assert_eq!(
            live.advance(&owner(), tick, 0.25, 1.0, |_, _| 1.0),
            restored.advance(&owner(), tick, 0.25, 1.0, |_, _| 1.0)
        );
        assert_eq!(live, restored);
    }
    let before = live.clone();
    assert!(live
        .advance(&owner(), 372, f32::NAN, 1.0, |_, _| panic!())
        .is_err());
    assert_eq!(live, before);
    let mut malformed = before.clone();
    malformed.job.as_mut().unwrap().activity.worked_secs = 3.0;
    assert!(malformed.validate(&owner(), 372).is_err());
    malformed = before.clone();
    malformed.job.as_mut().unwrap().execution_tick = 373;
    assert!(malformed.validate(&owner(), 372).is_err());
    let mut invalid_grant = started();
    let before = invalid_grant.clone();
    assert!(invalid_grant
        .advance(&owner(), 11, 1.0, 1.0, |_, _| f32::NAN)
        .is_err());
    assert_eq!(invalid_grant, before);
}
#[test]
fn tiny_fraction_residual_is_paid_before_completion_without_zero_interval_stall() {
    let mut state = started();
    let job = state.job.as_mut().unwrap();
    job.activity.profile.launch_work_secs = f32::from_bits(1);
    job.activity.worked_secs = f64::from(f32::from_bits(1)) * 0.5;
    let mut paid_seconds = 0.0;
    assert!(state
        .advance(&owner(), 11, f32::from_bits(1), 1.0, |_, seconds| {
            paid_seconds = seconds;
            1.0
        })
        .unwrap());
    assert_eq!(paid_seconds, f32::from_bits(1));
    assert_eq!(
        state.job.unwrap().activity.worked_secs,
        f64::from(f32::from_bits(1))
    );
}

#[test]
fn voluntary_cancel_settles_work_without_physical_handoff_refund_or_reused_job_identity() {
    let mut state = started();
    state.advance(&owner(), 11, 0.5, 1.0, |_, _| 1.0).unwrap();
    let old_id = state.job.as_ref().unwrap().id;
    assert!(state.cancel());
    assert!(!state.cancel());
    assert!(!state.complete());
    let settled = state.job.as_ref().unwrap();
    assert_eq!(settled.outcome, Some(JobOutcome::Cancelled));
    assert_eq!(settled.activity.worked_secs, 0.5);
    let saved: BayState = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(saved, state);
    state
        .begin(
            &owner(),
            "other-probe",
            BayOperation::Recover,
            &profile(),
            1.0,
            12,
        )
        .unwrap();
    let next = state.job.as_ref().unwrap();
    assert!(next.id.0 > old_id.0);
    assert_eq!(next.activity.worked_secs, 0.0);
    assert_eq!(next.activity.required_secs(), 3.0);
    assert!(next.outcome.is_none());
}

#[test]
fn physical_removal_is_a_single_terminal_failure_and_completion_cannot_remain_paused() {
    let mut state = started();
    state.advance(&owner(), 11, 2.0, 1.0, |_, _| 1.0).unwrap();
    assert!(state.complete());
    state.validate(&owner(), 11).unwrap();
    let mut removed = started();
    let target = removed.job.as_ref().unwrap().activity.craft_uuid.clone();
    assert!(!removed.fail_removed("another-craft", 11));
    assert!(!removed.fail_removed(&target, 9));
    assert!(removed.fail_removed(&target, 11));
    assert!(!removed.fail_removed(&target, 12));
    removed.validate(&owner(), 11).unwrap();
    assert_eq!(
        removed.job.as_ref().unwrap().outcome,
        Some(JobOutcome::Failed)
    );
    assert!(!removed.complete());
    let mut invalid = state;
    invalid.job.as_mut().unwrap().paused = true;
    assert!(invalid.validate(&owner(), 11).is_err());
}

#[test]
fn receiving_work_retains_exact_order_and_cannot_complete_without_actual_handoff_receipt() {
    use phoenix_model::{
        activity::ActivityJob,
        comms_request::{CommsRequestReference, GenericCommsRequest},
    };
    let order = CraftReceptionOrder {
        request: CommsRequestReference {
            formation_sequence: None,
            origin_id: "actual-request".into(),
            source_ship: "origin".into(),
            recipient_ship: "receiver".into(),
            emitted_tick: 1,
            heard_tick: 2,
            request: GenericCommsRequest::ReceiveCraft {
                craft: "actual-pod".into(),
            },
        },
        decision_tick: 3,
    };
    let mut state = BayState::default();
    state
        .begin_order(
            &owner(),
            "actual-pod",
            BayOperation::Recover,
            &profile(),
            1.0,
            4,
            Some(&order),
        )
        .unwrap();
    let before = state.clone();
    let mut replaced = order.clone();
    replaced.decision_tick = 4;
    assert!(state
        .begin_order(
            &owner(),
            "actual-pod",
            BayOperation::Recover,
            &profile(),
            1.0,
            4,
            Some(&replaced)
        )
        .is_err());
    assert_eq!(state, before);
    for now in 5..8 {
        state.advance(&owner(), now, 1.0, 1.0, |_, _| 1.0).unwrap();
    }
    let ready = state.clone();
    assert!(!state.complete());
    assert_eq!(state, ready);
    let job = state.job.as_ref().unwrap();
    let receipt = CraftReceptionReceipt {
        order,
        scope: ActivityJob {
            actor: "receiver".into(),
            owner: owner(),
            job: job.id,
        },
        craft: "actual-pod".into(),
        handoff_tick: 8,
        people: Vec::new(),
    };
    let mut wrong = receipt.clone();
    wrong.scope.job = JobId(90);
    assert!(!state.complete_receiving(Some(wrong)));
    assert_eq!(state, ready);
    assert!(state.complete_receiving(Some(receipt.clone())));
    state.validate(&owner(), 8).unwrap();
    assert_eq!(state.job.as_ref().unwrap().activity.receipt, Some(receipt));
    let restored: BayState = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(state, restored);
    assert!(!state.complete());
}
