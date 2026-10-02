use super::*;
use crate::gm_roster::{GmOperator, GmRoster};

fn gm(id: &str, connected: bool, ready: bool) -> GmOperator {
    GmOperator {
        id: id.into(),
        name: id.into(),
        connected,
        ready,
    }
}

fn roster(rows: Vec<GmOperator>) -> GmRoster {
    GmRoster::try_new(rows).unwrap()
}

#[test]
fn zero_participants_waits_and_stationless_crew_counts_like_any_crew() {
    assert_eq!(
        evaluate_start_policy([], &GmRoster::default(), true, StartTrigger::Automatic),
        StartPolicyDecision::Wait {
            reason: StartPolicyReason::NoParticipants
        }
    );
    assert_eq!(
        evaluate_start_policy(
            [ReadinessTally::try_new(1, 1).unwrap()],
            &GmRoster::default(),
            true,
            StartTrigger::Automatic,
        ),
        StartPolicyDecision::Start { forced_by: None }
    );
}

#[test]
fn every_connected_crew_cohort_and_gm_must_be_ready() {
    let gms = roster(vec![gm("gm-1", true, true), gm("gm-2", false, false)]);
    assert_eq!(
        evaluate_start_policy(
            [
                ReadinessTally::try_new(2, 2).unwrap(),
                ReadinessTally::try_new(1, 0).unwrap(),
            ],
            &gms,
            true,
            StartTrigger::Automatic,
        ),
        StartPolicyDecision::Wait {
            reason: StartPolicyReason::NotReady
        }
    );
    assert_eq!(
        evaluate_start_policy(
            [
                ReadinessTally::try_new(2, 2).unwrap(),
                ReadinessTally::try_new(1, 1).unwrap(),
            ],
            &gms,
            true,
            StartTrigger::Automatic,
        ),
        StartPolicyDecision::Start { forced_by: None }
    );
}

#[test]
fn gm_only_auto_start_excludes_disconnected_gms() {
    let gms = roster(vec![gm("gm-1", true, true), gm("gm-2", false, true)]);
    assert_eq!(
        evaluate_start_policy([], &gms, true, StartTrigger::Automatic),
        StartPolicyDecision::Start { forced_by: None }
    );
}

#[test]
fn every_connected_gm_is_an_equal_force_authority() {
    let gms = roster(vec![gm("gm-1", true, false), gm("gm-2", true, false)]);
    for id in ["gm-1", "gm-2"] {
        assert_eq!(
            evaluate_start_policy(
                [ReadinessTally::try_new(2, 0).unwrap()],
                &gms,
                true,
                StartTrigger::Forced { operator_id: id },
            ),
            StartPolicyDecision::Start {
                forced_by: Some(id.to_string())
            }
        );
    }
}

#[test]
fn unknown_disconnected_and_ship_identities_cannot_force() {
    let gms = roster(vec![gm("gm-1", false, false)]);
    for id in ["gm-1", "ship-1", "unknown"] {
        assert_eq!(
            evaluate_start_policy(
                [ReadinessTally::try_new(1, 0).unwrap()],
                &gms,
                true,
                StartTrigger::Forced { operator_id: id },
            ),
            StartPolicyDecision::Refused {
                reason: StartPolicyReason::GmNotConnected
            }
        );
    }
}

#[test]
fn validation_failure_blocks_auto_and_force() {
    let gms = roster(vec![gm("gm-1", true, true)]);
    assert_eq!(
        evaluate_start_policy([], &gms, false, StartTrigger::Automatic),
        StartPolicyDecision::Refused {
            reason: StartPolicyReason::ValidationFailed
        }
    );
    assert_eq!(
        evaluate_start_policy(
            [],
            &gms,
            false,
            StartTrigger::Forced {
                operator_id: "gm-1"
            },
        ),
        StartPolicyDecision::Refused {
            reason: StartPolicyReason::ValidationFailed
        }
    );
}

#[test]
fn tally_and_grant_shapes_enforce_their_invariants() {
    assert_eq!(
        ReadinessTally::try_new(1, 2),
        Err(ReadinessTallyError::ReadyExceedsConnected)
    );
    let automatic = StartGrant {
        id: "start-7".into(),
        mode: StartGrantMode::Automatic,
        operator_id: None,
        apply_tick: 412,
    };
    assert_eq!(automatic.validate(), Ok(7));
    assert_eq!(
        StartGrant {
            operator_id: Some("gm-1".into()),
            ..automatic.clone()
        }
        .validate(),
        Err(StartGrantError::InvalidAttribution)
    );
    assert_eq!(
        StartGrant {
            id: "start-8".into(),
            mode: StartGrantMode::Forced,
            operator_id: Some("gm-1".into()),
            apply_tick: 412,
        }
        .validate(),
        Ok(8)
    );
    assert_eq!(
        StartGrant {
            apply_tick: MAX_SAFE_START_APPLY_TICK + 1,
            ..automatic
        }
        .validate(),
        Err(StartGrantError::InvalidApplyTick)
    );
}
