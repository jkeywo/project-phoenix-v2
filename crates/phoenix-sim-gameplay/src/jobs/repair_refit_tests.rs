use super::*;
use crate::ship::interior::{InteriorConfig, InteriorGraph};
use serde_json::json;

fn sid(id: &str) -> SystemId {
    SystemId(id.into())
}
fn fixture() -> RepairTeams {
    let ship: ShipConfig = serde_json::from_value(
        json!({"system":[{"id":"a","kind":"sensors"},{"id":"b","kind":"comms"}]}),
    )
    .unwrap();
    let interior: InteriorConfig = serde_json::from_value(json!({
        "deck":[{"id":"main","label":"Main","damage_weight":1.0}],
        "compartment":[
            {"id":"home","deck":"main","label":"Home","x":0,"z":0,"width":4,"depth":4,"material":"steel"},
            {"id":"work","deck":"main","label":"Work","x":4,"z":0,"width":4,"depth":4,"material":"steel"}],
        "connection":[{"id":"hall","from":"home","to":"work","travel_secs":2}],
        "placements":{"a":"work","b":"home"},"team_homes":["home","home"]
    })).unwrap();
    let mut teams = RepairTeams::new(2);
    teams.install_layout(InteriorGraph::compile(&interior, &ship, 2).unwrap());
    teams
}
fn claim(teams: &mut RepairTeams) -> JobId {
    teams.claim_refit(0, 1, sid("a"), "A".into()).unwrap()
}
fn arrive(teams: &mut RepairTeams) {
    teams.tick_refits(2.0, &BTreeSet::new());
    teams.tick_refits(0.5, &BTreeSet::new());
}

#[test]
fn refit_uses_real_route_then_holds_team_until_physical_return() {
    let mut teams = fixture();
    let id = claim(&mut teams);
    assert_eq!(teams.refit_job(1), Some(id));
    assert_eq!(teams.free_team_indices(None), vec![1]);
    assert!(!teams.finish_refit(id));
    teams.tick_refits(1.0, &BTreeSet::new());
    assert!(matches!(
        teams.navigation().unwrap()[0].location,
        TeamLocation::Crossing { elapsed: 1.0, .. }
    ));
    assert!(!teams.refit_effect_enabled(id));
    teams.tick_refits(1.0, &BTreeSet::new());
    assert!(!teams.refit_effect_enabled(id)); // Travel permission is not work permission.
    teams.tick_refits(0.5, &BTreeSet::new());
    assert!(teams.refit_effect_enabled(id));
    assert!(teams.on_site_systems().next().is_none()); // No normal repair execution.
    assert!(teams.finish_refit(id));
    assert!(!teams.refit_effect_enabled(id));
    assert!(teams.is_committed_to_operation(0, None));
    teams.tick_refits(1.0, &BTreeSet::new());
    assert_eq!(teams.free_team_indices(None), vec![1]);
    teams.tick_refits(1.0, &BTreeSet::new());
    assert_eq!(teams.free_team_indices(None), vec![0, 1]);
    assert_eq!(teams.work_list()[0].outcome, Some(JobOutcome::Completed));
    teams.restore_state(&teams.save_state()).unwrap();
}

#[test]
fn normal_repair_tick_does_not_advance_refit_route_workflow_or_damage() {
    let mut teams = fixture();
    claim(&mut teams);
    teams.tick_refits(0.5, &BTreeSet::new());
    let saved = teams.save_state();
    let mut hull = SystemHull::from_config(&[(sid("a"), 100.0)]);
    hull.set_hp(&sid("a"), 20.0);
    teams.tick_with_work(10.0, &mut hull, None, |_, _| {
        panic!("refit must not pay normal repair")
    });
    assert_eq!(teams.save_state(), saved);
    assert_eq!(hull.current_for(&sid("a")), Some(20.0));
}

#[test]
fn repair_external_and_refit_share_one_exclusive_team_pool() {
    let mut teams = fixture();
    assert!(RepairTeams::new(2)
        .claim_refit(0, 1, sid("a"), "A".into())
        .is_none());
    assert!(teams.claim_refit(0, 0, sid("a"), "A".into()).is_none());
    assert!(teams
        .claim_refit(0, 1, sid("missing"), "A".into())
        .is_none());
    let id = claim(&mut teams);
    assert!(!teams.claim_external(0, "other-ship".into()));
    assert!(teams.claim_refit(1, 1, sid("b"), "B".into()).is_none());
    assert!(teams.claim_refit(1, 2, sid("a"), "A".into()).is_none());
    teams.dispatch(0, sid("b"), "B".into());
    teams.dispatch(1, sid("a"), "A".into());
    assert_eq!(teams.refit_job(1), Some(id));
    assert_eq!(teams.free_team_indices(None), vec![1]);
    assert!(!teams.dispatch_physical(0, sid("b"), None, "B".into()));
    assert!(!teams.dispatch_physical(1, sid("a"), None, "A".into()));
    assert!(!teams.dispatch_physical(
        1,
        sid("b"),
        Some(phoenix_model::damage::RepairWorkTarget::Component {
            system: sid("a"),
            component: "operation".into()
        }),
        "A".into()
    ));
    teams.dispatch(1, sid("b"), "B".into());
    assert!(teams.claim_refit(1, 2, sid("b"), "B".into()).is_none());
}

#[test]
fn recall_mid_crossing_returns_from_actual_position_and_preserves_cancelled_outcome() {
    let mut teams = fixture();
    let id = claim(&mut teams);
    teams.tick_refits(0.5, &BTreeSet::new());
    let position = teams.navigation().unwrap()[0].clone();
    assert!(teams.recall_refit(id));
    assert_eq!(teams.navigation().unwrap()[0], position);
    assert!(!teams.recall(0));
    teams.tick_refits(1.5, &BTreeSet::new()); // Finish committed outward crossing.
    assert!(!teams.refit_effect_enabled(id));
    assert!(teams.is_refit_team(0));
    teams.tick_refits(2.0, &BTreeSet::new());
    assert!(!teams.is_refit_team(0));
    assert_eq!(teams.work_list()[0].outcome, Some(JobOutcome::Cancelled));
    teams.restore_state(&teams.save_state()).unwrap();
}

#[test]
fn blocked_refit_routes_resume_and_workflow_pause_cannot_grant_work() {
    let mut teams = fixture();
    let id = claim(&mut teams);
    let blocked = BTreeSet::from(["hall".into()]);
    teams.tick_refits(20.0, &blocked);
    assert!(
        matches!(&teams.navigation().unwrap()[0].location, TeamLocation::Compartment { compartment } if compartment == "home")
    );
    assert!(!teams.refit_effect_enabled(id));
    arrive(&mut teams);
    assert!(teams.refit_effect_enabled(id));
    let mut workflow = repair_workflow::default_config();
    for task in &mut workflow.tree[0].state[0].task {
        if task.id == "repair" {
            task.action = state_tree::TaskAction::Write {
                channel: "apply_repair".into(),
                value: state_tree::BoolSource::Literal { value: false },
            };
        }
    }
    teams.program = repair_workflow::compile_workflow(Some(&workflow)).unwrap();
    // Runtime identity belongs to its compiled workflow; test the new policy
    // from a clean workflow frame while retaining physical commitment.
    let job = teams.state.jobs.teams[0].as_mut().unwrap();
    job.runtime = Default::default();
    job.execution_tick = 0;
    teams.tick_refits(1.0, &BTreeSet::new());
    assert!(!teams.refit_effect_enabled(id));
    assert!(teams.recall(0));
    teams.tick_refits(2.0, &BTreeSet::new());
    assert_eq!(teams.work_list()[0].outcome, Some(JobOutcome::Cancelled));
}

#[test]
fn refit_continuation_preserves_crossing_and_rejects_forged_claims_without_writes() {
    let mut teams = fixture();
    claim(&mut teams);
    teams.tick_refits(0.75, &BTreeSet::new());
    let saved = teams.save_state();
    let mut resumed = fixture();
    resumed.restore_state(&saved).unwrap();
    for _ in 0..16 {
        teams.tick_refits(0.125, &BTreeSet::new());
        resumed.tick_refits(0.125, &BTreeSet::new());
        assert_eq!(teams.save_state(), resumed.save_state());
    }
    let baseline = resumed.save_state();
    let mut invalid = Vec::new();
    let mut duplicate = baseline.clone();
    duplicate.teams[1] = duplicate.teams[0].clone();
    duplicate.teams[1].as_mut().unwrap().id = JobId(duplicate.next_id);
    duplicate.next_id += 1;
    invalid.push(duplicate);
    let mut remote = baseline.clone();
    remote.navigation.as_mut().unwrap()[0] = fixture().navigation().unwrap()[0].clone();
    invalid.push(remote);
    let mut bad_request = baseline.clone();
    if let RepairActivity::Refit { request, .. } =
        &mut bad_request.teams[0].as_mut().unwrap().activity
    {
        *request = 0;
    }
    invalid.push(bad_request);
    let mut bad_timer = saved.clone();
    if let RepairActivity::Refit {
        phase: TeamSlot::Travelling { elapsed, .. },
        ..
    } = &mut bad_timer.teams[0].as_mut().unwrap().activity
    {
        *elapsed = f32::NAN;
    }
    invalid.push(bad_timer);
    let mut bad_phase = baseline.clone();
    if let RepairActivity::Refit {
        phase: TeamSlot::Repairing { system_id, .. },
        ..
    } = &mut bad_phase.teams[0].as_mut().unwrap().activity
    {
        *system_id = Some(sid("b"));
    }
    invalid.push(bad_phase);
    for state in invalid {
        assert!(resumed.restore_state(&state).is_err());
        assert_eq!(resumed.save_state(), baseline);
    }
}
