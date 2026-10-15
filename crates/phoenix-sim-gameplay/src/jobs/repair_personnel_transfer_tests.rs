use super::*;
use crate::ship::{
    config::ShipConfig,
    interior::{InteriorConfig, InteriorGraph},
};
use serde_json::json;
fn fixture() -> (RepairTeams, CompiledPersonnel, PersonnelState) {
    let ship: ShipConfig =
        serde_json::from_value(json!({"system":[{"id":"repair","kind":"repair"}]})).unwrap();
    let interior:InteriorConfig=serde_json::from_value(json!({
        "deck":[{"id":"main","label":"Main","damage_weight":1}],
        "compartment":[
            {"id":"base","deck":"main","label":"Base","x":0,"z":0,"width":2,"depth":2,"material":"steel","max_hp":100,"recovery_fraction":0.25,"structural_remedy":"structure"},
            {"id":"home","deck":"main","label":"Home","x":2,"z":0,"width":2,"depth":2,"material":"steel","max_hp":100,"recovery_fraction":0.25,"structural_remedy":"structure"},
            {"id":"bay","deck":"main","label":"Bay","x":4,"z":0,"width":2,"depth":2,"material":"steel","max_hp":100,"recovery_fraction":0.25,"structural_remedy":"structure"}],
        "connection":[{"id":"to-home","from":"base","to":"home","travel_secs":2},{"id":"to-bay","from":"home","to":"bay","travel_secs":2}],
        "placements":{"repair":"home"},"team_homes":["base","home"]})).unwrap();
    let config=serde_json::from_value(json!({
        "repair_team":[{"id":"escort","label":"Escort","team_idx":0,"competencies":["mechanical_repair","personnel_transport"]},{"id":"repair","label":"Repair","team_idx":1,"competencies":["mechanical_repair"]}],
        "officer":[{"id":"chief","label":"Chief","home_compartment":"home","competencies":["mechanical_repair"],"repair_work_multiplier":1.25}]})).unwrap();
    let profile = CompiledPersonnel::compile(&config, &interior, 2).unwrap();
    let state = PersonnelState::new(&profile, "origin");
    let mut teams = RepairTeams::new(2);
    teams.install_layout(InteriorGraph::compile(&interior, &ship, 2).unwrap());
    (teams, profile, state)
}
fn order() -> PersonnelDestinationRequest {
    PersonnelDestinationRequest {
        destination: PersonnelDestination::Craft {
            craft: "pod".into(),
        },
        requested_tick: 1,
    }
}
fn plan() -> PersonnelTransferPlan {
    PersonnelTransferPlan {
        craft: "pod".into(),
        berth: "one".into(),
        bay: SystemId("bay-system".into()),
        pickup: OfficerLocation::AtCompartment {
            actor: "origin".into(),
            compartment: "home".into(),
        },
        pickup_compartment: "home".into(),
        delivery_compartment: "bay".into(),
    }
}
fn tick(
    teams: &mut RepairTeams,
    profile: &CompiledPersonnel,
    state: &mut PersonnelState,
    tick: u64,
    blocked: &BTreeSet<String>,
) {
    let mut roster = crate::personnel::PersonnelRoster::default();
    roster.accept("origin", profile.clone(), None, 0).unwrap();
    roster.origins.get_mut("origin").unwrap().state = state.clone();
    teams.tick_personnel_transfers(
        "origin",
        profile,
        &mut roster,
        tick,
        0.5,
        blocked,
        |_, _| TransferAvailability::Ready,
    );
    *state = roster.state("origin").unwrap().clone();
    state
        .validate(profile, teams.job_state(), "origin", tick)
        .unwrap();
}
#[test]
fn actual_collection_escort_handoff_and_return_keep_one_team_job_and_person() {
    let (mut teams, profile, mut state) = fixture();
    assert_eq!(
        teams.personnel_transfer_teams(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("origin", "chief")
        ),
        vec![0]
    );
    let id = teams
        .claim_personnel_transfer(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("origin", "chief"),
            0,
            order(),
            plan(),
        )
        .unwrap();
    assert!(teams
        .claim_personnel_transfer(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("origin", "chief"),
            1,
            order(),
            plan()
        )
        .is_none());
    assert!(state
        .post(
            &profile,
            teams.job_state(),
            teams.layout().unwrap(),
            &crate::personnel::PersonRef::new("origin", "chief"),
            Some(1),
            2
        )
        .is_err());
    teams.dispatch(0, SystemId("repair".into()), "Repair".into());
    assert_eq!(teams.work_list()[0].id, id);
    for step in 1..=4 {
        tick(&mut teams, &profile, &mut state, step, &BTreeSet::new());
    }
    assert_eq!(
        state.officers["chief"].location,
        OfficerLocation::WithTransfer {
            actor: "origin".into(),
            team_idx: 0,
            job: id
        }
    );
    let blocked = BTreeSet::from(["to-bay".into()]);
    for step in 5..=10 {
        tick(&mut teams, &profile, &mut state, step, &blocked);
    }
    assert!(teams.navigation().unwrap()[0].blocked);
    assert!(teams.claims_personnel_craft("pod"));
    assert!(matches!(
        state.officers["chief"].location,
        OfficerLocation::WithTransfer { .. }
    ));
    for step in 11..=40 {
        tick(&mut teams, &profile, &mut state, step, &BTreeSet::new());
    }
    assert_eq!(
        state.officers["chief"].location,
        OfficerLocation::AboardCraft {
            craft: "pod".into(),
            berth: "one".into()
        }
    );
    assert_eq!(teams.work_list()[0].id, id);
    assert_eq!(teams.work_list()[0].outcome, Some(JobOutcome::Completed));
    assert!(!teams.claims_personnel_craft("pod"));
    assert_eq!(
        state.repair_work_multipliers(&profile, teams.job_state(), "origin"),
        vec![1.0, 1.0]
    );
}
#[test]
fn actual_escort_json_continuation_retains_exact_job_cursor_and_custody() {
    let (mut teams, profile, mut state) = fixture();
    teams
        .claim_personnel_transfer(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("origin", "chief"),
            0,
            order(),
            plan(),
        )
        .unwrap();
    for step in 1..=5 {
        tick(&mut teams, &profile, &mut state, step, &BTreeSet::new());
    }
    assert!(matches!(
        state.officers["chief"].location,
        OfficerLocation::WithTransfer { .. }
    ));
    let saved = serde_json::from_str(&serde_json::to_string(&teams.save_state()).unwrap()).unwrap();
    let (mut resumed, _, _) = fixture();
    resumed.restore_state(&saved).unwrap();
    let mut resumed_state: PersonnelState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    for step in 6..366 {
        assert_eq!(teams.save_state(), resumed.save_state());
        assert_eq!(state, resumed_state);
        tick(&mut teams, &profile, &mut state, step, &BTreeSet::new());
        tick(
            &mut resumed,
            &profile,
            &mut resumed_state,
            step,
            &BTreeSet::new(),
        );
    }
    assert_eq!(teams.save_state(), resumed.save_state());
    assert_eq!(state, resumed_state);
}
#[test]
fn recall_after_collection_returns_real_officer_home_before_team_release() {
    let (mut teams, profile, mut state) = fixture();
    let id = teams
        .claim_personnel_transfer(
            &profile,
            &state,
            &crate::personnel::PersonRef::new("origin", "chief"),
            0,
            order(),
            plan(),
        )
        .unwrap();
    for step in 1..=5 {
        tick(&mut teams, &profile, &mut state, step, &BTreeSet::new());
    }
    let before = state.clone();
    assert!(teams.recall(0));
    assert_eq!(state, before);
    assert!(teams.lowest_free_team().is_some_and(|team| team != 0));
    assert!(matches!(
        state.officers["chief"].location,
        OfficerLocation::WithTransfer { .. }
    ));
    for step in 6..=30 {
        tick(&mut teams, &profile, &mut state, step, &BTreeSet::new());
    }
    assert_eq!(
        state.officers["chief"].location,
        OfficerLocation::AtCompartment {
            actor: "origin".into(),
            compartment: "home".into()
        }
    );
    assert_eq!(teams.work_list()[0].id, id);
    assert_eq!(teams.work_list()[0].outcome, Some(JobOutcome::Cancelled));
}
#[test]
fn origin_loss_preserves_actual_offship_custody_until_actual_craft_loss() {
    let (_, profile, _) = fixture();
    let mut roster = crate::personnel::PersonnelRoster::default();
    roster.accept("origin", profile, None, 0).unwrap();
    roster
        .state_mut("origin")
        .unwrap()
        .officers
        .get_mut("chief")
        .unwrap()
        .location = OfficerLocation::AboardCraft {
        craft: "pod".into(),
        berth: "one".into(),
    };
    roster.remove_actor("origin", 10);
    assert_eq!(roster.origins["origin"].removed_tick, Some(10));
    assert_eq!(
        roster.origins["origin"].state.officers["chief"].location,
        OfficerLocation::AboardCraft {
            craft: "pod".into(),
            berth: "one".into()
        }
    );
    roster.remove_actor("pod", 12);
    assert_eq!(
        roster.origins["origin"].state.officers["chief"].location,
        OfficerLocation::Lost {
            actor: "pod".into()
        }
    );
    assert_eq!(
        roster.origins["origin"].state.officers["chief"].changed_tick,
        12
    );
    let last = roster.clone();
    roster.remove_actor("pod", 13);
    assert_eq!(roster, last);
}
