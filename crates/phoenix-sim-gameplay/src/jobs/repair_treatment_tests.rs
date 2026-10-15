use super::*;
use crate::{
    personnel::*,
    ship::{
        config::ShipConfig,
        interior::{InteriorConfig, InteriorGraph},
    },
};
use serde_json::json;
fn fixture() -> (RepairTeams, CompiledPersonnel, PersonnelState) {
    let ship: ShipConfig =
        serde_json::from_value(json!({"system":[{"id":"repair","kind":"repair"}]})).unwrap();
    let interior:InteriorConfig=serde_json::from_value(json!({
        "deck":[{"id":"main","label":"Main","damage_weight":1}],
        "compartment":[{"id":"home","deck":"main","label":"Home","x":0,"z":0,"width":4,"depth":4,"material":"steel","max_hp":100,"recovery_fraction":0.25,"structural_remedy":"structure"},
        {"id":"clinic","deck":"main","label":"Clinic","x":4,"z":0,"width":4,"depth":4,"material":"steel","max_hp":100,"recovery_fraction":0.25,"structural_remedy":"structure"}],
        "connection":[{"id":"hall","from":"home","to":"clinic","travel_secs":2}],"placements":{"repair":"clinic"},"team_homes":["home","clinic"]})).unwrap();
    let config:PersonnelConfig=serde_json::from_value(json!({
        "repair_team":[{"id":"medics","label":"Medics","team_idx":0,"competencies":["medical_care"]},{"id":"engineers","label":"Engineers","team_idx":1,"competencies":["mechanical_repair"]}],
        "officer":[{"id":"chief","label":"Chief","home_compartment":"clinic","competencies":["mechanical_repair"],"repair_work_multiplier":1.25,"initial_condition":"minor"}],
        "care":{"system":"repair","minor_work_secs":3,"severe_work_secs":6,"requirement":[{"resource":"medication","service":"medication","amount":1}]}})).unwrap();
    let profile = CompiledPersonnel::compile(&config, &interior, 2).unwrap();
    let state = PersonnelState::new(&profile, "ship");
    let mut teams = RepairTeams::new(2);
    teams.install_layout(InteriorGraph::compile(&interior, &ship, 2).unwrap());
    (teams, profile, state)
}
fn tick(
    teams: &mut RepairTeams,
    profile: &CompiledPersonnel,
    state: &mut PersonnelState,
    tick: u64,
    dt: f32,
    blocked: &BTreeSet<String>,
    stock: &mut i32,
    power: f32,
) {
    teams.tick_treatments(
        profile,
        state,
        "ship",
        tick,
        dt,
        blocked,
        |commit, _, _| {
            if *stock <= 0 {
                return false;
            }
            if commit {
                *stock -= 1;
            }
            true
        },
        |_, _| power,
    );
}
#[test]
fn actual_care_reserves_same_team_travels_then_consumes_once_and_returns() {
    let (mut teams, profile, mut state) = fixture();
    assert_eq!(
        state.treatment_teams(
            &profile,
            teams.job_state(),
            &crate::personnel::PersonRef::new("ship", "chief")
        ),
        vec![0]
    );
    assert!(state
        .post(
            &profile,
            teams.job_state(),
            teams.layout().unwrap(),
            &crate::personnel::PersonRef::new("ship", "chief"),
            Some(1),
            0
        )
        .is_err());
    let id = teams
        .claim_treatment(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("ship", "chief"),
            0,
        )
        .unwrap();
    assert!(!teams.claim_external(0, "other-ship".into()));
    assert!(teams
        .claim_refit(1, 2, SystemId("repair".into()), "Repair".into())
        .is_none());
    assert!(teams
        .claim_refit(0, 1, SystemId("repair".into()), "Repair".into())
        .is_none());
    assert!(!teams.dispatch_physical(0, SystemId("repair".into()), None, "Repair".into()));
    assert!(teams
        .claim_treatment(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("ship", "chief"),
            1
        )
        .is_none());
    let mut stock = 1;
    tick(
        &mut teams,
        &profile,
        &mut state,
        1,
        4.0,
        &BTreeSet::from(["hall".into()]),
        &mut stock,
        1.0,
    );
    assert_eq!(stock, 1);
    assert_eq!(teams.work_list()[0].work_progress, Some(0));
    tick(
        &mut teams,
        &profile,
        &mut state,
        2,
        2.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert_eq!(stock, 1);
    assert_eq!(state.officers["chief"].condition, PersonnelCondition::Minor);
    tick(
        &mut teams,
        &profile,
        &mut state,
        3,
        1.0,
        &BTreeSet::new(),
        &mut stock,
        0.0,
    );
    assert_eq!(stock, 1);
    tick(
        &mut teams,
        &profile,
        &mut state,
        4,
        1.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert_eq!(stock, 0);
    tick(
        &mut teams,
        &profile,
        &mut state,
        5,
        2.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert_eq!(state.officers["chief"].condition, PersonnelCondition::Fit);
    assert!(teams.is_committed_to_operation(0, None));
    assert_eq!(teams.work_list()[0].id, id);
    assert_eq!(teams.work_list()[0].outcome, None);
    tick(
        &mut teams,
        &profile,
        &mut state,
        6,
        2.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert_eq!(stock, 0);
    assert_eq!(teams.work_list()[0].outcome, Some(JobOutcome::Completed));
    state
        .validate(&profile, teams.job_state(), "ship", 6)
        .unwrap();
}
#[test]
fn recall_json_retains_custody_and_committed_medication_without_refund() {
    let (mut teams, profile, mut state) = fixture();
    teams
        .claim_treatment(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("ship", "chief"),
            0,
        )
        .unwrap();
    let mut stock = 2;
    tick(
        &mut teams,
        &profile,
        &mut state,
        1,
        2.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    tick(
        &mut teams,
        &profile,
        &mut state,
        2,
        1.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert_eq!(stock, 1);
    assert!(teams.recall(0));
    let saved: RepairJobState =
        serde_json::from_slice(&serde_json::to_vec(teams.job_state()).unwrap()).unwrap();
    let mut restored = fixture().0;
    restored.restore_state(&saved).unwrap();
    state
        .validate(&profile, restored.job_state(), "ship", 2)
        .unwrap();
    tick(
        &mut restored,
        &profile,
        &mut state,
        3,
        2.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert_eq!(stock, 1);
    assert_eq!(state.officers["chief"].condition, PersonnelCondition::Minor);
    assert_eq!(restored.work_list()[0].outcome, Some(JobOutcome::Cancelled));
}
#[test]
fn missing_medication_waits_and_candidate_work_or_custody_forgery_refuses() {
    let (mut teams, profile, mut state) = fixture();
    teams
        .claim_treatment(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("ship", "chief"),
            0,
        )
        .unwrap();
    let mut stock = 0;
    tick(
        &mut teams,
        &profile,
        &mut state,
        1,
        2.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    tick(
        &mut teams,
        &profile,
        &mut state,
        2,
        1.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert!(teams.work_list()[0].waiting_parts);
    assert_eq!(state.officers["chief"].condition, PersonnelCondition::Minor);
    let original = teams.save_state();
    for mode in 0..5 {
        let mut bad = original.clone();
        let RepairActivity::Treatment { work, .. } = &mut bad.teams[0].as_mut().unwrap().activity
        else {
            panic!()
        };
        match mode {
            0 => work.work_secs = 0.001,
            1 => work.patient = crate::personnel::PersonRef::new("ship", "missing"),
            2 => work.condition = PersonnelCondition::Severe,
            3 => work.progress = 2.0,
            _ => {
                bad.navigation.as_mut().unwrap()[0].location = TeamLocation::Compartment {
                    compartment: "home".into(),
                };
            }
        }
        assert!(state.validate(&profile, &bad, "ship", 2).is_err());
        assert_eq!(teams.save_state(), original);
    }
}

#[test]
fn actual_completed_care_survives_a_supported_failing_return_tree_and_still_goes_home() {
    let (mut teams, profile, mut state) = fixture();
    let mut config = repair_workflow::default_config();
    config.tree[0].state[0].task.push(state_tree::TaskDef {
        id: "return-fails".into(),
        when: vec![state_tree::BoolCondition {
            value: state_tree::BoolSource::Fact {
                name: "returning".into(),
            },
            equals: true,
        }],
        action: state_tree::TaskAction::Complete {
            result: state_tree::Completion::Failure,
        },
    });
    teams.program = repair_workflow::compile_workflow(Some(&config)).unwrap();
    teams
        .claim_treatment(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("ship", "chief"),
            0,
        )
        .unwrap();
    let mut stock = 1;
    tick(
        &mut teams,
        &profile,
        &mut state,
        1,
        2.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    tick(
        &mut teams,
        &profile,
        &mut state,
        2,
        3.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert_eq!(state.officers["chief"].condition, PersonnelCondition::Fit);
    tick(
        &mut teams,
        &profile,
        &mut state,
        3,
        0.5,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert!(teams.work_list()[0].workflow_error);
    state
        .validate(&profile, teams.job_state(), "ship", 3)
        .unwrap();
    let saved: RepairJobState =
        serde_json::from_slice(&serde_json::to_vec(teams.job_state()).unwrap()).unwrap();
    let mut restored = fixture().0;
    restored.program = repair_workflow::compile_workflow(Some(&config)).unwrap();
    restored.restore_state(&saved).unwrap();
    tick(
        &mut restored,
        &profile,
        &mut state,
        4,
        2.0,
        &BTreeSet::new(),
        &mut stock,
        1.0,
    );
    assert_eq!(restored.work_list()[0].outcome, Some(JobOutcome::Completed));
    assert_eq!(state.officers["chief"].condition, PersonnelCondition::Fit);
    assert_eq!(stock, 0);
}

#[test]
fn forged_extra_treatment_team_is_rejected_before_navigation_or_home_indexing() {
    let (mut teams, profile, state) = fixture();
    teams
        .claim_treatment(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("ship", "chief"),
            0,
        )
        .unwrap();
    let original = teams.save_state();
    let mut bad = original.clone();
    let mut extra = bad.teams[0].clone().unwrap();
    extra.id.0 += 1;
    extra.outcome = Some(JobOutcome::Cancelled);
    let RepairActivity::Treatment {
        phase, cancelled, ..
    } = &mut extra.activity
    else {
        panic!()
    };
    *cancelled = true;
    *phase = TeamSlot::Returning {
        remaining: 0.0,
        system_id: None,
        display_name: None,
        queued_system_id: None,
        queued_display_name: None,
    };
    bad.next_id = extra.id.0 + 1;
    bad.teams.push(Some(extra));
    bad.navigation
        .as_mut()
        .unwrap()
        .push(original.navigation.as_ref().unwrap()[0].clone());
    assert!(teams.restore_state(&bad).is_err());
    assert_eq!(teams.save_state(), original);
}
