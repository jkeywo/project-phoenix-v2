use super::*;
use crate::jobs::security::SecurityJobs;
use crate::security::teams::{SecurityPriority, SecurityTeamState};
use crate::ship::interior::*;

fn profile() -> CompiledLocalSecurity {
    let ship = crate::ship::config::ShipConfig {
        consoles: vec![],
        stations: vec![],
        systems: vec![],
        power_groups: Default::default(),
        coordination_lag_secs: 0.0,
    };
    let graph = InteriorGraph::compile(
        &InteriorConfig {
            decks: vec![DeckConfig {
                id: "main".into(),
                label: "test.main".into(),
                damage_weight: 1.0,
            }],
            compartments: ["bridge", "engineering"]
                .into_iter()
                .enumerate()
                .map(|(index, id)| CompartmentConfig {
                    id: id.into(),
                    deck: "main".into(),
                    label: format!("test.{id}"),
                    x: index as f32 * 5.0,
                    z: 0.0,
                    width: 4.0,
                    depth: 4.0,
                    material: "bulkhead".into(),
                    max_hp: 100.0,
                    recovery_fraction: 0.2,
                    structural_remedy: "structure".into(),
                })
                .collect(),
            connections: vec![ConnectionConfig {
                id: "corridor".into(),
                from: "bridge".into(),
                to: "engineering".into(),
                travel_secs: 2.0,
                boundary: None,
            }],
            placements: Default::default(),
            team_homes: vec![],
        },
        &ship,
        0,
    )
    .unwrap();
    CompiledLocalSecurity {
        graph,
        config: LocalSecurityConfig {
            team_homes: vec!["bridge".into()],
            targets: vec![LocalSecurityTarget {
                compartment: "engineering".into(),
                actions: vec![SecurityActionConfig {
                    action: SecurityAction::SecureContain,
                    duration_secs: 2.0,
                    risk: 0.0,
                    priority: SecurityPriority::LifeSafety,
                    outcome_flag: Some("contained".into()),
                    outcome_value: 1,
                    warning: None,
                }],
            }],
        },
    }
}
fn muster(profile: &CompiledLocalSecurity) -> SecurityJobs {
    let mut state = SecurityJobs::new(1, crate::ship::system_registry::security_system_id());
    state.navigation = Some(profile.initial_navigation());
    assert!(state.deploy(
        0,
        address("engineering"),
        SecurityAction::SecureContain,
        0.0,
        0.0
    ));
    state
}

#[test]
fn sealed_route_waits_then_actual_arrival_precedes_paid_work_and_actual_return_releases_job() {
    let profile = profile();
    let mut state = muster(&profile);
    let blocked = BTreeSet::from(["corridor".into()]);
    assert!(tick(&profile, &mut state, 10.0, &[10.0], &blocked, false).is_empty());
    assert_eq!(state[0].state, SecurityTeamState::Deploying);
    assert!(state.navigation.as_ref().unwrap()[0].blocked);
    assert!(state.work_list()[0].paused);
    tick(&profile, &mut state, 2.0, &[2.0], &BTreeSet::new(), false);
    assert_eq!(state[0].state, SecurityTeamState::Working);
    assert_eq!(state[0].elapsed, 0.0, "travel cannot also credit work");
    assert!(tick(&profile, &mut state, 10.0, &[0.0], &BTreeSet::new(), false).is_empty());
    assert_eq!(state[0].elapsed, 0.0, "unfunded work cannot progress");
    let events = tick(&profile, &mut state, 2.0, &[2.0], &BTreeSet::new(), false);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, Some(("contained".into(), 1)));
    assert_eq!(state[0].state, SecurityTeamState::Withdrawing);
    assert!(!state.deploy(
        0,
        address("engineering"),
        SecurityAction::SecureContain,
        0.0,
        0.0
    ));
    tick(&profile, &mut state, 2.0, &[0.0], &BTreeSet::new(), false);
    assert!(state[0].is_available());
    assert_eq!(
        state.work_list()[0].outcome,
        Some(phoenix_model::job::JobOutcome::Completed)
    );
    profile.validate_navigation(&state).unwrap();
}

#[test]
fn recall_mid_crossing_preserves_cursor_across_json_and_does_not_teleport_or_repeat_outcome() {
    let profile = profile();
    let mut state = muster(&profile);
    tick(&profile, &mut state, 1.0, &[0.0], &BTreeSet::new(), false);
    let id = state.work_list()[0].id;
    state.withdraw(0, 0.0, phoenix_model::job::JobOutcome::Cancelled);
    assert!(matches!(
        state.navigation.as_ref().unwrap()[0].location,
        RouteLocation::Crossing { elapsed: 1.0, .. }
    ));
    let mut restored: SecurityJobs =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    for _ in 0..6 {
        let first = tick(&profile, &mut state, 0.5, &[0.0], &BTreeSet::new(), false);
        let second = tick(
            &profile,
            &mut restored,
            0.5,
            &[0.0],
            &BTreeSet::new(),
            false,
        );
        assert!(first.is_empty() && second.is_empty());
        assert_eq!(restored, state);
    }
    assert!(state[0].is_available());
    assert_eq!(state.work_list()[0].id, id);
    assert_eq!(
        state.work_list()[0].outcome,
        Some(phoenix_model::job::JobOutcome::Cancelled)
    );
}

#[test]
fn machinery_loss_returns_the_same_physical_team_instead_of_disappearing_it() {
    let profile = profile();
    let mut state = muster(&profile);
    tick(&profile, &mut state, 1.0, &[0.0], &BTreeSet::new(), false);
    let before = state.navigation.clone();
    let events = tick(&profile, &mut state, 1.0, &[0.0], &BTreeSet::new(), true);
    assert_eq!(events.len(), 1);
    assert!(!events[0].completed);
    assert_eq!(state.navigation, before);
    assert_eq!(state[0].state, SecurityTeamState::Withdrawing);
    for _ in 0..3 {
        tick(&profile, &mut state, 1.0, &[0.0], &BTreeSet::new(), true);
    }
    assert_eq!(state[0].state, SecurityTeamState::Unavailable);
    assert_eq!(
        state.work_list()[0].outcome,
        Some(phoenix_model::job::JobOutcome::Failed)
    );
}
