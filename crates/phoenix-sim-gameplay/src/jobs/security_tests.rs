use super::*;

fn muster() -> SecurityJobs {
    SecurityJobs::new(2, SystemId("security".into()))
}

#[test]
fn return_retains_the_same_commitment_and_recall_does_not_release_a_team_early() {
    let mut teams = muster();
    assert!(teams.deploy(0, "target".into(), SecurityAction::Board, 0.2, 2.0));
    let id = teams.jobs.teams[0].as_ref().unwrap().id;
    teams.withdraw(0, 3.0, JobOutcome::Cancelled);
    assert!(!teams.deploy(0, "other".into(), SecurityAction::Board, 0.0, 1.0));
    assert_eq!(teams.jobs.teams[0].as_ref().unwrap().id, id);
    assert_eq!(teams.jobs.teams[0].as_ref().unwrap().outcome, None);
    let json = serde_json::to_string(&teams.saved()).unwrap();
    let mut restored: SecurityJobs = serde_json::from_str(&json).unwrap();
    restored
        .validate(2, &SystemId("security".into()), 0)
        .unwrap();
    assert_eq!(restored, teams);
    restored.home(0);
    assert_eq!(
        restored.jobs.teams[0].as_ref().unwrap().outcome,
        Some(JobOutcome::Cancelled)
    );
    assert!(restored.deploy(0, "other".into(), SecurityAction::Board, 0.0, 1.0));
    assert!(restored.jobs.teams[0].as_ref().unwrap().id > id);
}

#[test]
fn malformed_owner_mint_phase_and_result_are_rejected_without_replacing_live_jobs() {
    let mut teams = muster();
    assert!(teams.deploy(0, "target".into(), SecurityAction::Board, 0.0, 1.0));
    let owner = teams.jobs.owner.clone();
    let mut cases = Vec::new();
    let mut bad = teams.clone();
    bad.jobs.next_id = 1;
    cases.push(bad);
    let mut bad = teams.clone();
    bad.jobs.owner = SystemId("repair".into());
    cases.push(bad);
    let mut bad = teams.clone();
    bad.jobs.teams[0]
        .as_mut()
        .unwrap()
        .activity
        .returning_outcome = Some(JobOutcome::Completed);
    cases.push(bad);
    let mut bad = teams.clone();
    bad.jobs.teams[0].as_mut().unwrap().activity.team.elapsed = f32::INFINITY;
    cases.push(bad);
    let mut bad = teams.clone();
    bad.jobs.teams[0].as_mut().unwrap().execution_tick = 1;
    cases.push(bad);
    let mut bad = teams.clone();
    bad.jobs.teams[1] = bad.jobs.teams[0].clone();
    cases.push(bad);
    for bad in cases {
        assert!(bad.validate(2, &owner, 0).is_err());
    }
    assert_eq!(teams.jobs.next_id, 2);
    assert!(teams[0].is_committed());
}

#[test]
fn old_assignment_vectors_and_unknown_state_are_not_silently_converted() {
    assert!(serde_json::from_str::<SecurityJobs>(r#"{"teams":[]}"#).is_err());
    let mut encoded = serde_json::to_value(muster()).unwrap();
    encoded["unavailable"] = serde_json::json!(true);
    assert!(serde_json::from_value::<SecurityJobs>(encoded).is_err());
}

#[test]
fn shared_work_list_distinguishes_completed_work_from_released_team() {
    let mut teams = muster();
    assert!(teams.deploy(0, "target".into(), SecurityAction::Board, 0.2, 2.0));
    teams.withdraw(0, 3.0, JobOutcome::Completed);
    let returning = teams.work_list();
    assert_eq!(returning[0].phase, phoenix_model::job::JobPhase::Returning);
    assert_eq!(returning[0].outcome, None);
    teams.home(0);
    let terminal = teams.work_list();
    assert_eq!(terminal[0].id, returning[0].id);
    assert_eq!(terminal[0].phase, phoenix_model::job::JobPhase::Finished);
    assert_eq!(terminal[0].outcome, Some(JobOutcome::Completed));
}

#[test]
fn operator_crossing_terms_are_checked_against_actual_configuration() {
    let config = crate::security::teams::SecurityConfig {
        local: None,
        team_count: 2,
        deploy_duration_secs: 2.0,
        withdraw_duration_secs: 3.0,
        range: 10.0,
    };
    let mut teams = muster();
    assert!(teams.deploy(0, "target".into(), SecurityAction::Board, 0.2, 2.0));
    teams.validate_config(&config, 0).unwrap();
    teams.get_mut(0).unwrap().phase_duration = 0.001;
    assert!(teams.validate_config(&config, 0).is_err());
    teams.get_mut(0).unwrap().phase_duration = 2.0;
    teams.withdraw(0, 3.0, JobOutcome::Cancelled);
    teams.validate_config(&config, 0).unwrap();
    teams.get_mut(0).unwrap().phase_duration = 0.001;
    assert!(teams.validate_config(&config, 0).is_err());
}
