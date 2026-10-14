use super::*;
use crate::ai::state_tree::*;

#[test]
fn repair_tools_pay_only_on_site_work_and_pause_without_losing_the_job() {
    let mut teams = RepairTeams::new_with_timings(1, timings());
    let mut hull = hull();
    teams.dispatch(0, sid("a"), "A".into());
    let mut calls = 0;
    teams.tick_with_work(1.0, &mut hull, None, |_, _| {
        calls += 1;
        0.0
    });
    assert_eq!(calls, 0); // Walking to the system uses no repair machinery.
    let job = teams.work_list()[0].id;
    teams.tick_with_work(1.0, &mut hull, None, |_, _| 0.25);
    assert_eq!(hull.current_for(&sid("a")), Some(50.5));
    teams.tick_with_work(1.0, &mut hull, None, |_, _| 0.0);
    assert_eq!(hull.current_for(&sid("a")), Some(50.5));
    assert_eq!(teams.work_list()[0].id, job);
    teams.tick_with_work(1.0, &mut hull, None, |_, _| 1.0);
    assert_eq!(hull.current_for(&sid("a")), Some(52.5));
    teams.recall(0);
    teams.tick_with_work(1.0, &mut hull, None, |_, _| {
        panic!("return must not spend tool energy")
    });
    assert_eq!(teams.lowest_free_team(), Some(0));
}
fn sid(value: &str) -> SystemId {
    SystemId(value.into())
}
fn hull() -> SystemHull {
    let mut hull = SystemHull::from_config(&[(sid("a"), 100.0)]);
    hull.set_hp(&sid("a"), 50.0);
    hull
}
fn timings() -> RepairTimings {
    RepairTimings {
        travel_duration: 1.0,
        repair_rate_hp_per_sec: 2.0,
    }
}
#[test]
fn default_job_keeps_phase_timing_identity_and_bounded_outcomes() {
    let mut teams = RepairTeams::new_with_timings(1, timings());
    let mut hull = hull();
    teams.dispatch(0, sid("a"), "A".into());
    let id = teams.work_list()[0].id;
    teams.tick(1.0, &mut hull, None);
    assert_eq!(hull.current_for(&sid("a")), Some(50.0));
    assert!(matches!(teams.slots()[0], TeamSlot::Repairing { .. }));
    teams.tick(1.0, &mut hull, None);
    assert_eq!(hull.current_for(&sid("a")), Some(52.0));
    teams.recall(0);
    assert_eq!(teams.work_list()[0].id, id);
    assert!(teams.lowest_free_team().is_none());
    teams.tick(1.0, &mut hull, None);
    assert_eq!(teams.work_list()[0].outcome, Some(JobOutcome::Cancelled));
    assert_eq!(teams.lowest_free_team(), Some(0));
    teams.dispatch(0, sid("a"), "A".into());
    assert_ne!(teams.work_list()[0].id, id);
    assert_eq!(teams.work_list().len(), 1);
}
#[test]
fn external_claim_is_canonical_idle_projection_and_atomic_retarget() {
    let mut teams = RepairTeams::new(2);
    assert!(teams.claim_external(1, "ally".into()));
    let state = teams.save_state();
    assert!(matches!(teams.slots()[1], TeamSlot::Idle));
    assert_eq!(teams.free_team_indices(None), vec![0]);
    teams.dispatch(1, sid("a"), "A".into());
    assert_eq!(teams.save_state(), state);
    assert!(teams.claim_external(1, "ally".into()));
    assert_eq!(teams.save_state(), state);
    assert!(!teams.claim_external(0, "other".into()));
    assert_eq!(teams.external_target(), Some("ally"));
    assert!(teams.step_external());
    assert!(teams.external_effect_enabled());
    assert!(teams.claim_external(1, "other".into()));
    assert_ne!(teams.work_list()[0].id, state.teams[1].as_ref().unwrap().id);
    assert!(teams.release_external());
    assert_eq!(teams.free_team_indices(None), vec![0, 1]);
}
fn referenced(final_value: bool) -> AiTreeConfig {
    let mut config = repair_workflow::default_config();
    let mut child = config.tree[0].clone();
    child.id = "child".into();
    child.memory.insert("visited".into(), false);
    child.state[0].task = vec![
        TaskDef {
            id: "remember".into(),
            when: vec![],
            action: TaskAction::Remember {
                key: "visited".into(),
                value: BoolSource::Literal { value: true },
            },
        },
        TaskDef {
            id: "write".into(),
            when: vec![],
            action: TaskAction::Write {
                channel: "apply_repair".into(),
                value: BoolSource::Literal { value: true },
            },
        },
    ];
    config.tree[0].state[0].task.push(TaskDef {
        id: "child".into(),
        when: vec![],
        action: TaskAction::Call {
            tree: "child".into(),
            parameters: BTreeMap::new(),
        },
    });
    config.tree[0].state[0].task.push(TaskDef {
        id: "final".into(),
        when: vec![],
        action: TaskAction::Write {
            channel: "apply_repair".into(),
            value: BoolSource::Literal { value: final_value },
        },
    });
    config.tree.push(child);
    config
}
#[test]
fn references_and_ordered_writes_gate_one_effect_and_private_memory_survives_restore() {
    for enabled in [false, true] {
        let config = referenced(enabled);
        let mut teams =
            RepairTeams::new_with_workflow(1, timings(), sid("repair"), Some(&config)).unwrap();
        let mut original = hull();
        teams.dispatch(0, sid("a"), "A".into());
        teams.tick(1.0, &mut original, None);
        teams.tick(1.0, &mut original, None);
        assert_eq!(
            original.current_for(&sid("a")),
            Some(if enabled { 52.0 } else { 50.0 })
        );
        let saved = teams.save_state();
        let job = saved.teams[0].as_ref().unwrap();
        assert!(
            job.runtime
                .root
                .as_ref()
                .unwrap()
                .calls
                .values()
                .next()
                .unwrap()
                .memory["visited"]
        );
        let mut resumed =
            RepairTeams::new_with_workflow(1, timings(), sid("repair"), Some(&config)).unwrap();
        resumed.restore_state(&saved).unwrap();
        assert_eq!(resumed.save_state(), saved);
        let mut restored_hull = original.clone();
        teams.tick(1.0, &mut original, None);
        resumed.tick(1.0, &mut restored_hull, None);
        assert_eq!(teams.save_state(), resumed.save_state());
        assert_eq!(
            original.current_for(&sid("a")),
            restored_hull.current_for(&sid("a"))
        );
        assert_eq!(teams.work_list()[0].paused, !enabled);
    }
}
#[test]
fn restore_rejects_duplicate_claims_growth_owner_and_invalid_private_continuation_atomically() {
    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("a"), "A".into());
    teams.dispatch(1, sid("b"), "B".into());
    let valid = teams.save_state();
    let mut bad = valid.clone();
    bad.teams.push(None);
    assert!(teams.restore_state(&bad).is_err());
    bad = valid.clone();
    bad.teams[1].as_mut().unwrap().id = bad.teams[0].as_ref().unwrap().id;
    assert!(teams.restore_state(&bad).is_err());
    bad = valid.clone();
    bad.owner = sid("other");
    assert!(teams.restore_state(&bad).is_err());
    bad = valid.clone();
    bad.teams[0].as_mut().unwrap().runtime.last_tick = Some(0);
    assert!(teams.restore_state(&bad).is_err());
    assert_eq!(teams.save_state(), valid);
}
#[test]
fn workflow_failure_keeps_commitment_visible_and_recall_can_clean_up() {
    let mut teams = RepairTeams::new_with_timings(1, timings());
    let mut hull = hull();
    teams.dispatch(0, sid("a"), "A".into());
    teams.tick(0.5, &mut hull, None);
    teams.state.teams[0]
        .as_mut()
        .unwrap()
        .runtime
        .root
        .as_mut()
        .unwrap()
        .active_leaf = "missing".into();
    teams.tick(0.1, &mut hull, None);
    assert!(teams.work_list()[0].workflow_error);
    assert!(teams.work_list()[0].paused);
    assert_eq!(teams.lowest_free_team(), None);
    assert!(teams.recall(0));
    teams.tick(0.5, &mut hull, None);
    assert_eq!(teams.lowest_free_team(), Some(0));
    assert_eq!(teams.work_list()[0].outcome, Some(JobOutcome::Failed));
}
#[test]
fn workflow_context_refuses_unavailable_facts_and_channels() {
    let mut config = repair_workflow::default_config();
    config.tree[0].state[0].task[0].action = TaskAction::Write {
        channel: "weapons_free".into(),
        value: BoolSource::Literal { value: true },
    };
    assert!(repair_workflow::compile_workflow(Some(&config)).is_err());
}

#[test]
fn terminal_workflow_cannot_release_team_and_returning_failure_can_clean_up() {
    let mut config = repair_workflow::default_config();
    config.tree[0].state[0].task.push(TaskDef {
        id: "finish".into(),
        when: vec![BoolCondition {
            value: BoolSource::Fact {
                name: "returning".into(),
            },
            equals: true,
        }],
        action: TaskAction::Complete {
            result: Completion::Failure,
        },
    });
    let mut teams =
        RepairTeams::new_with_workflow(1, timings(), sid("repair"), Some(&config)).unwrap();
    let mut hull = hull();
    teams.dispatch(0, sid("a"), "A".into());
    teams.tick(0.5, &mut hull, None);
    teams.recall(0);
    teams.tick(0.2, &mut hull, None);
    assert!(teams.work_list()[0].workflow_error);
    assert_eq!(teams.lowest_free_team(), None);
    let saved = teams.save_state();
    let mut resumed =
        RepairTeams::new_with_workflow(1, timings(), sid("repair"), Some(&config)).unwrap();
    resumed.restore_state(&saved).unwrap();
    assert!(resumed.recall(0));
    resumed.tick(0.5, &mut hull, None);
    assert_eq!(resumed.lowest_free_team(), Some(0));
    assert_eq!(resumed.work_list()[0].outcome, Some(JobOutcome::Failed));
}
