use super::*;
use crate::jobs::repair::RepairTeams;
use crate::ship::{config::ShipConfig, interior::InteriorGraph};
use serde_json::json;

fn fixture() -> (
    CompiledPersonnel,
    PersonnelState,
    RepairTeams,
    InteriorGraph,
) {
    let ship: ShipConfig = serde_json::from_value(json!({"system":[]})).unwrap();
    let interior: InteriorConfig = serde_json::from_value(json!({
        "deck":[{"id":"main","label":"Main","damage_weight":1.0}],
        "compartment":[
            {"id":"home","deck":"main","label":"Home","x":0,"z":0,"width":2,"depth":2,"material":"steel","max_hp":100,"recovery_fraction":0.25,"structural_remedy":"structural"},
            {"id":"work","deck":"main","label":"Work","x":2,"z":0,"width":2,"depth":2,"material":"steel","max_hp":100,"recovery_fraction":0.25,"structural_remedy":"structural"}],
        "connection":[{"id":"door","from":"home","to":"work","travel_secs":2}],
        "placements":{},"team_homes":["home","home"]
    })).unwrap();
    let graph = InteriorGraph::compile(&interior, &ship, 2).unwrap();
    let config: PersonnelConfig = serde_json::from_value(json!({
        "repair_team":[{"id":"alpha","label":"Alpha","team_idx":0,"competencies":["mechanical_repair"]},
            {"id":"beta","label":"Beta","team_idx":1,"competencies":["mechanical_repair"]}],
        "officer":[{"id":"chief","label":"Chief","home_compartment":"home","competencies":["mechanical_repair"],"repair_work_multiplier":1.25},
            {"id":"deputy","label":"Deputy","home_compartment":"home","competencies":["mechanical_repair"],"repair_work_multiplier":1.1}]
    })).unwrap();
    let compiled = CompiledPersonnel::compile(&config, &interior, 2).unwrap();
    let state = PersonnelState::new(&compiled, "ship");
    let mut teams = RepairTeams::new(2);
    teams.install_layout(graph.clone());
    (compiled, state, teams, graph)
}

#[test]
fn exclusive_posting_uses_actual_team_cursor_and_cannot_teleport_officers() {
    let (profile, mut personnel, teams, graph) = fixture();
    let mut jobs = teams.save_state();
    personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "chief"),
            Some(0),
            4,
        )
        .unwrap();
    let posted = personnel.clone();
    // Identical intent has no new receipt or movement even at a later tick.
    personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "chief"),
            Some(0),
            5,
        )
        .unwrap();
    assert_eq!(personnel, posted);
    assert!(personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "deputy"),
            Some(0),
            5
        )
        .is_err());
    assert_eq!(personnel, posted);
    assert!(!graph.advance(
        &mut jobs.navigation.as_mut().unwrap()[0],
        "work",
        0.5,
        &BTreeSet::new()
    ));
    let crossing = jobs.navigation.as_ref().unwrap()[0].clone();
    personnel.validate(&profile, &jobs, "ship", 5).unwrap();
    assert!(personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "chief"),
            None,
            5
        )
        .is_err());
    assert!(personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "chief"),
            Some(1),
            5
        )
        .is_err());
    assert_eq!(personnel, posted);
    assert_eq!(jobs.navigation.as_ref().unwrap()[0], crossing);
    assert_eq!(
        personnel.repair_work_multipliers(&profile, &jobs, "ship"),
        vec![1.0, 1.0]
    );
    assert!(graph.advance(
        &mut jobs.navigation.as_mut().unwrap()[0],
        "work",
        2.0,
        &BTreeSet::new()
    ));
    assert!(personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "chief"),
            None,
            7
        )
        .is_err());
    assert!(graph.advance(
        &mut jobs.navigation.as_mut().unwrap()[0],
        "home",
        2.0,
        &BTreeSet::new()
    ));
    personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "chief"),
            None,
            9,
        )
        .unwrap();
    assert_eq!(
        personnel.officers["chief"].location,
        OfficerLocation::AtCompartment {
            actor: "ship".into(),
            compartment: "home".into()
        }
    );
}

#[test]
fn full_json_frontier_rejects_duplicate_custody_unknown_teams_and_future_postings() {
    let (profile, mut personnel, teams, graph) = fixture();
    let jobs = teams.save_state();
    personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "chief"),
            Some(0),
            8,
        )
        .unwrap();
    let saved = serde_json::to_string(&personnel).unwrap();
    let restored: PersonnelState = serde_json::from_str(&saved).unwrap();
    restored.validate(&profile, &jobs, "ship", 8).unwrap();
    assert_eq!(restored, personnel);
    for mode in 0..4 {
        let mut forged = restored.clone();
        match mode {
            0 => {
                forged.officers.get_mut("deputy").unwrap().location =
                    OfficerLocation::WithRepairTeam {
                        actor: "ship".into(),
                        team_idx: 0,
                    }
            }
            1 => {
                forged.officers.get_mut("chief").unwrap().location =
                    OfficerLocation::WithRepairTeam {
                        actor: "ship".into(),
                        team_idx: 9,
                    }
            }
            2 => forged.officers.get_mut("chief").unwrap().changed_tick = 9,
            _ => {
                forged.officers.remove("deputy");
            }
        }
        assert!(forged.validate(&profile, &jobs, "ship", 8).is_err());
        assert_eq!(serde_json::to_string(&personnel).unwrap(), saved);
    }
}

#[test]
fn posted_continuation_cannot_cross_between_distinct_actual_team_homes() {
    let (profile, _, mut teams, mut graph) = fixture();
    graph.config.team_homes[1] = "work".into();
    teams.install_layout(graph.clone());
    let profile = CompiledPersonnel::compile(&profile.config, &graph.config, 2).unwrap();
    let jobs = teams.save_state();
    let mut personnel = PersonnelState::new(&profile, "ship");
    personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "chief"),
            Some(0),
            4,
        )
        .unwrap();
    personnel.validate(&profile, &jobs, "ship", 4).unwrap();
    let original = personnel.clone();
    assert!(personnel
        .post(
            &profile,
            &jobs,
            &graph.config,
            &crate::personnel::PersonRef::new("ship", "chief"),
            Some(1),
            4
        )
        .is_err());
    assert_eq!(personnel, original);
    let mut forged = original;
    forged.officers.get_mut("chief").unwrap().location = OfficerLocation::WithRepairTeam {
        actor: "ship".into(),
        team_idx: 1,
    };
    assert!(
        forged.validate(&profile, &jobs, "ship", 4).is_err(),
        "restore must retain the same home-only physical handoff rule as admission"
    );
}

#[test]
fn retained_roster_keeps_origin_identity_and_never_recreates_lost_officers() {
    let (profile, _, _, _) = fixture();
    let mut roster = PersonnelRoster::default();
    roster.accept("first", profile.clone(), None, 0).unwrap();
    roster.accept("second", profile.clone(), None, 0).unwrap();
    assert_eq!(roster.origins.len(), 2);
    assert_eq!(roster.state("first").unwrap().officers.len(), 2);
    roster.remove_actor("first", 7);
    assert!(roster.state("first").is_none());
    assert!(roster.state_mut("first").is_none());
    assert!(roster.state("second").is_some());
    for state in roster.origins["first"].state.officers.values() {
        assert_eq!(
            state.location,
            OfficerLocation::Lost {
                actor: "first".into()
            }
        );
        assert_eq!(state.changed_tick, 7);
    }
    let saved = roster.clone();
    roster.remove_actor("first", 8);
    assert_eq!(roster, saved);
    assert!(roster.accept("first", profile, None, 8).is_err());
    assert_eq!(roster, saved);
    let decoded: PersonnelRoster =
        serde_json::from_str(&serde_json::to_string(&roster).unwrap()).unwrap();
    assert_eq!(decoded, roster);
}

#[test]
fn actual_holder_loss_never_uses_the_retained_origin_as_remote_occupancy() {
    let (profile, _, _, _) = fixture();
    let mut roster = PersonnelRoster::default();
    roster.accept("origin", profile.clone(), None, 0).unwrap();
    // Owner-level loss input: explicit actual holder differs from stable origin.
    // No supported foreign admission/transfer is claimed by this structural test.
    roster
        .state_mut("origin")
        .unwrap()
        .officers
        .get_mut("chief")
        .unwrap()
        .location = OfficerLocation::AtCompartment {
        actor: "receiver".into(),
        compartment: "home".into(),
    };
    roster
        .state_mut("origin")
        .unwrap()
        .officers
        .get_mut("deputy")
        .unwrap()
        .location = OfficerLocation::WithRepairTeam {
        actor: "receiver".into(),
        team_idx: 0,
    };
    let before = roster.origins["origin"].state.clone();
    roster.remove_actor("origin", 10);
    assert_eq!(
        roster.origins["origin"].state, before,
        "origin removal cannot kill actual offship people"
    );
    roster.remove_actor("receiver", 12);
    for person in roster.origins["origin"].state.officers.values() {
        assert_eq!(
            person.location,
            OfficerLocation::Lost {
                actor: "receiver".into()
            }
        );
        assert_eq!(person.changed_tick, 12);
    }
    let lost = roster.clone();
    roster.remove_actor("receiver", 13);
    assert_eq!(roster, lost);
}
