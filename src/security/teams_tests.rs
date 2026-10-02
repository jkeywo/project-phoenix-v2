use super::*;

fn team(state: SecurityTeamState) -> SecurityTeam {
    SecurityTeam {
        state,
        ..Default::default()
    }
}

// ── The action and priority vocabularies ─────────────────────────────────

#[test]
fn every_action_id_round_trips_and_is_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for action in SecurityAction::ALL {
        assert!(seen.insert(action.as_str()), "duplicate action id");
        assert_eq!(SecurityAction::parse(action.as_str()), Some(action));
    }
    assert_eq!(SecurityAction::parse("evacuate_everyone"), None);
}

/// The four verbs issue #1346 names as the required vocabulary are all here,
/// spelled the way a scenario authors them.
#[test]
fn the_required_action_vocabulary_is_present() {
    for id in [
        "secure_contain",
        "assist_evacuation",
        "board",
        "place_charges",
    ] {
        assert!(
            SecurityAction::parse(id).is_some(),
            "the required vocabulary must include '{id}'"
        );
    }
}

#[test]
fn priority_orders_most_urgent_first() {
    assert!(SecurityPriority::LifeSafety < SecurityPriority::EvacuationUnderway);
    assert!(SecurityPriority::EvacuationUnderway < SecurityPriority::UrgentObjective);
    assert!(SecurityPriority::UrgentObjective < SecurityPriority::ThreatContainment);
    assert!(SecurityPriority::ThreatContainment < SecurityPriority::Optional);
}

#[test]
fn an_objective_promotes_lesser_work_but_never_demotes_life_safety() {
    assert_eq!(
        SecurityPriority::Optional.promoted_by_objective(),
        SecurityPriority::UrgentObjective
    );
    assert_eq!(
        SecurityPriority::ThreatContainment.promoted_by_objective(),
        SecurityPriority::UrgentObjective
    );
    assert_eq!(
        SecurityPriority::LifeSafety.promoted_by_objective(),
        SecurityPriority::LifeSafety,
        "a mission naming a fire cannot make it less urgent"
    );
    assert_eq!(
        SecurityPriority::EvacuationUnderway.promoted_by_objective(),
        SecurityPriority::EvacuationUnderway
    );
}

#[test]
fn every_refusal_has_its_own_string_id() {
    let mut seen = std::collections::BTreeSet::new();
    for refusal in SecurityRefusal::ALL {
        assert!(
            seen.insert(refusal.string_id()),
            "two refusals share one strings.csv id"
        );
        assert!(refusal
            .string_id()
            .starts_with("security.dispatch.refused."));
    }
}

// ── The dispatch verdict ─────────────────────────────────────────────────

#[test]
fn an_available_team_a_real_target_an_offered_action_in_range_dispatches() {
    let available = team(SecurityTeamState::Available);
    assert_eq!(
        dispatch_status(Some(&available), true, true, Some(120.0), 400.0, false),
        Ok(())
    );
    // Exactly at the range boundary still dispatches.
    assert_eq!(
        dispatch_status(Some(&available), true, true, Some(400.0), 400.0, false),
        Ok(())
    );
}

#[test]
fn a_disabled_security_system_refuses_before_anything_else() {
    assert_eq!(
        dispatch_status(None, false, false, None, 400.0, true),
        Err(SecurityRefusal::Disabled)
    );
    let available = team(SecurityTeamState::Available);
    assert_eq!(
        dispatch_status(Some(&available), true, true, Some(1.0), 400.0, true),
        Err(SecurityRefusal::Disabled)
    );
}

#[test]
fn an_unknown_team_index_and_a_busy_team_are_distinct_refusals() {
    assert_eq!(
        dispatch_status(None, true, true, Some(1.0), 400.0, false),
        Err(SecurityRefusal::NoSuchTeam)
    );
    for busy in [
        SecurityTeamState::Deploying,
        SecurityTeamState::Working,
        SecurityTeamState::Withdrawing,
        SecurityTeamState::Unavailable,
    ] {
        assert_eq!(
            dispatch_status(Some(&team(busy)), true, true, Some(1.0), 400.0, false),
            Err(SecurityRefusal::TeamBusy),
            "a {busy:?} team cannot take a second assignment"
        );
    }
}

/// The invalid-target half of issue #1346's AC3: a target nothing answers to
/// and a target that does not offer the asked-for verb are different answers,
/// and neither sends anybody.
#[test]
fn an_invalid_target_and_an_unoffered_action_refuse_distinctly() {
    let available = team(SecurityTeamState::Available);
    assert_eq!(
        dispatch_status(Some(&available), false, false, None, 400.0, false),
        Err(SecurityRefusal::NoSuchTarget)
    );
    assert_eq!(
        dispatch_status(Some(&available), true, false, Some(10.0), 400.0, false),
        Err(SecurityRefusal::ActionUnavailable)
    );
}

#[test]
fn a_target_past_the_authored_range_or_one_that_cannot_be_located_is_out_of_range() {
    let available = team(SecurityTeamState::Available);
    assert_eq!(
        dispatch_status(Some(&available), true, true, Some(400.1), 400.0, false),
        Err(SecurityRefusal::OutOfRange)
    );
    assert_eq!(
        dispatch_status(Some(&available), true, true, None, 400.0, false),
        Err(SecurityRefusal::OutOfRange)
    );
}

// ── The team state machine ───────────────────────────────────────────────

#[test]
fn a_team_walks_available_deploying_working_withdrawing_and_home() {
    let mut t = SecurityTeam::default();
    assert!(t.is_available());
    assert!(!t.is_committed());

    t.deploy("target-1".into(), SecurityAction::SecureContain, 0.4, 6.0);
    assert_eq!(t.state, SecurityTeamState::Deploying);
    assert!(t.is_committed() && t.is_interruptible() && !t.is_available());
    assert_eq!(t.target.as_deref(), Some("target-1"));

    t.begin_work(20.0);
    assert_eq!(t.state, SecurityTeamState::Working);
    assert_eq!(t.phase_duration, 20.0);
    assert!(t.is_interruptible());

    t.withdraw(4.0);
    assert_eq!(t.state, SecurityTeamState::Withdrawing);
    assert!(t.is_committed(), "a team on its way home is not back yet");
    assert!(
        !t.is_interruptible(),
        "nothing about the target can interrupt a team already withdrawing"
    );

    t.arrive_home();
    assert!(t.is_available());
    assert_eq!(t.target, None);
    assert_eq!(t.action, None);
}

#[test]
fn progress_is_the_current_phase_and_a_zero_length_phase_reads_complete() {
    let mut t = SecurityTeam::default();
    assert_eq!(t.progress(), 1.0, "an idle team is not mid-anything");
    t.deploy("t".into(), SecurityAction::Board, 0.0, 10.0);
    assert_eq!(t.progress(), 0.0);
    t.elapsed = 5.0;
    assert_eq!(t.progress(), 0.5);
    t.elapsed = 99.0;
    assert_eq!(t.progress(), 1.0, "progress is clamped");
    t.begin_work(0.0);
    assert_eq!(t.progress(), 1.0);
}

// ── The backfill selection ───────────────────────────────────────────────

fn candidate(
    target: &str,
    action: SecurityAction,
    priority: SecurityPriority,
    eligible: bool,
) -> SecurityCandidate {
    SecurityCandidate {
        target: target.into(),
        action,
        priority,
        eligible,
    }
}

#[test]
fn nothing_is_selected_with_no_free_teams_or_no_candidates() {
    let pool = vec![candidate(
        "a",
        SecurityAction::SecureContain,
        SecurityPriority::LifeSafety,
        true,
    )];
    assert!(select_assignments(&pool, 0).is_empty());
    assert!(select_assignments(&[], 2).is_empty());
}

/// The whole ordering issue #1346 names, in one pass: life safety first,
/// then an evacuation already underway, then an urgent Objective, then
/// containment, then optional work.
#[test]
fn selection_follows_the_authored_priority_order() {
    let pool = vec![
        candidate(
            "opt",
            SecurityAction::Board,
            SecurityPriority::Optional,
            true,
        ),
        candidate(
            "contain",
            SecurityAction::SecureContain,
            SecurityPriority::ThreatContainment,
            true,
        ),
        candidate(
            "obj",
            SecurityAction::PlaceCharges,
            SecurityPriority::UrgentObjective,
            true,
        ),
        candidate(
            "evac",
            SecurityAction::AssistEvacuation,
            SecurityPriority::EvacuationUnderway,
            true,
        ),
        candidate(
            "life",
            SecurityAction::SecureContain,
            SecurityPriority::LifeSafety,
            true,
        ),
    ];
    let chosen = select_assignments(&pool, 5);
    let names: Vec<&str> = chosen.iter().map(|i| pool[*i].target.as_str()).collect();
    assert_eq!(names, vec!["life", "evac", "obj", "contain", "opt"]);
}

#[test]
fn an_ineligible_candidate_is_never_selected() {
    let pool = vec![
        candidate(
            "unreachable",
            SecurityAction::SecureContain,
            SecurityPriority::LifeSafety,
            false,
        ),
        candidate(
            "reachable",
            SecurityAction::Board,
            SecurityPriority::Optional,
            true,
        ),
    ];
    // Two teams: the first is spent on the reachable optional job, and the
    // second is HELD for the unreachable life-safety one.
    let chosen = select_assignments(&pool, 2);
    assert_eq!(chosen, vec![1]);
}

/// The reservation, stated exactly as issue #1346 does: spending both teams
/// must not make a known higher-priority task impossible.
#[test]
fn the_last_team_is_reserved_when_a_higher_priority_job_is_known_but_blocked() {
    let pool = vec![
        candidate(
            "fire",
            SecurityAction::SecureContain,
            SecurityPriority::LifeSafety,
            false,
        ),
        candidate(
            "salvage-a",
            SecurityAction::Board,
            SecurityPriority::Optional,
            true,
        ),
        candidate(
            "salvage-b",
            SecurityAction::Board,
            SecurityPriority::Optional,
            true,
        ),
    ];
    let chosen = select_assignments(&pool, 2);
    assert_eq!(
        chosen.len(),
        1,
        "one team goes; the other is kept for the fire"
    );
    assert_eq!(pool[chosen[0]].target, "salvage-a");
}

#[test]
fn the_last_team_is_spent_when_nothing_higher_is_pending() {
    let pool = vec![
        candidate(
            "salvage-a",
            SecurityAction::Board,
            SecurityPriority::Optional,
            true,
        ),
        candidate(
            "salvage-b",
            SecurityAction::Board,
            SecurityPriority::Optional,
            true,
        ),
    ];
    assert_eq!(select_assignments(&pool, 2).len(), 2);
}

/// A blocked job of EQUAL or LOWER priority reserves nothing — the rule is
/// about protecting something more urgent, not about hoarding.
#[test]
fn an_equal_or_lower_priority_blocked_job_reserves_nothing() {
    let pool = vec![
        candidate(
            "blocked",
            SecurityAction::Board,
            SecurityPriority::Optional,
            false,
        ),
        candidate(
            "go-a",
            SecurityAction::SecureContain,
            SecurityPriority::Optional,
            true,
        ),
        candidate(
            "go-b",
            SecurityAction::SecureContain,
            SecurityPriority::ThreatContainment,
            true,
        ),
    ];
    assert_eq!(select_assignments(&pool, 2).len(), 2);
}

/// The most urgent job known is never held back for itself.
#[test]
fn the_top_priority_job_is_never_reserved_against() {
    let pool = vec![
        candidate(
            "blocked-optional",
            SecurityAction::Board,
            SecurityPriority::Optional,
            false,
        ),
        candidate(
            "fire",
            SecurityAction::SecureContain,
            SecurityPriority::LifeSafety,
            true,
        ),
    ];
    let chosen = select_assignments(&pool, 1);
    assert_eq!(chosen.len(), 1);
    assert_eq!(pool[chosen[0]].target, "fire");
}

#[test]
fn ties_break_deterministically_by_target_then_action() {
    let pool = vec![
        candidate(
            "beta",
            SecurityAction::SecureContain,
            SecurityPriority::LifeSafety,
            true,
        ),
        candidate(
            "alpha",
            SecurityAction::PlaceCharges,
            SecurityPriority::LifeSafety,
            true,
        ),
        candidate(
            "alpha",
            SecurityAction::Board,
            SecurityPriority::LifeSafety,
            true,
        ),
    ];
    let chosen = select_assignments(&pool, 3);
    let labels: Vec<String> = chosen
        .iter()
        .map(|i| format!("{}/{}", pool[*i].target, pool[*i].action.as_str()))
        .collect();
    assert_eq!(
        labels,
        vec!["alpha/board", "alpha/place_charges", "beta/secure_contain"]
    );
}

// ── The pool the selection ranks ─────────────────────────────────────────

fn working_on(target: &str, action: SecurityAction) -> SecurityTeam {
    let mut team = SecurityTeam::default();
    team.deploy(target.to_string(), action, 0.5, 2.0);
    team.begin_work(4.0);
    team
}

/// The job a committed team already holds leaves the pool, so the free team
/// beside it is ranked against what is genuinely left rather than against a
/// duplicate it can only drop.
#[test]
fn work_a_committed_team_already_holds_leaves_the_pool() {
    let pool = vec![
        candidate(
            "skyhook",
            SecurityAction::AssistEvacuation,
            SecurityPriority::LifeSafety,
            true,
        ),
        candidate(
            "gallery",
            SecurityAction::SecureContain,
            SecurityPriority::ThreatContainment,
            true,
        ),
    ];
    let teams = vec![
        working_on("skyhook", SecurityAction::AssistEvacuation),
        SecurityTeam::default(),
    ];

    let left = unassigned_candidates(&pool, &teams);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].target, "gallery");

    // The regression the filter exists for: one free team, and the top of the
    // unfiltered pool is the job the busy team is already on. Selecting from
    // the raw pool spends the slot on that duplicate and strands the team;
    // selecting from the filtered pool sends it to the fire.
    let chosen = select_assignments(&left, 1);
    assert_eq!(chosen.len(), 1, "the free team must be spent, not stranded");
    assert_eq!(left[chosen[0]].target, "gallery");
}

/// Only the exact target-and-action pair is held: the same target's OTHER
/// authored actions, and the same action elsewhere, both stay assignable.
#[test]
fn only_the_exact_job_a_team_holds_leaves_the_pool() {
    let pool = vec![
        candidate(
            "gallery",
            SecurityAction::SecureContain,
            SecurityPriority::ThreatContainment,
            true,
        ),
        candidate(
            "gallery",
            SecurityAction::AssistEvacuation,
            SecurityPriority::LifeSafety,
            true,
        ),
        candidate(
            "ladder",
            SecurityAction::SecureContain,
            SecurityPriority::ThreatContainment,
            true,
        ),
    ];
    let teams = vec![working_on("gallery", SecurityAction::SecureContain)];
    let left = unassigned_candidates(&pool, &teams);
    assert_eq!(left.len(), 2);
    assert!(left
        .iter()
        .all(|c| !(c.target == "gallery" && c.action == SecurityAction::SecureContain)));
}

/// A team at home holds nothing, so an idle muster filters nothing out.
#[test]
fn an_idle_muster_holds_nothing_back() {
    let pool = vec![candidate(
        "gallery",
        SecurityAction::SecureContain,
        SecurityPriority::ThreatContainment,
        true,
    )];
    let teams = vec![SecurityTeam::default(), SecurityTeam::default()];
    assert_eq!(unassigned_candidates(&pool, &teams), pool);
}

/// Work under way needs no team reserved for it: removing it from the pool
/// also removes it from the reservation's reckoning, so the last free team
/// goes to the lesser job instead of being held for a fire already being
/// fought.
#[test]
fn the_reservation_does_not_hold_a_team_for_work_already_under_way() {
    let pool = vec![
        candidate(
            "fire",
            SecurityAction::SecureContain,
            SecurityPriority::LifeSafety,
            false,
        ),
        candidate(
            "salvage",
            SecurityAction::Board,
            SecurityPriority::Optional,
            true,
        ),
    ];
    assert!(
        select_assignments(&pool, 1).is_empty(),
        "with the fire unassigned and out of reach the last team is held"
    );

    let teams = vec![working_on("fire", SecurityAction::SecureContain)];
    let left = unassigned_candidates(&pool, &teams);
    let chosen = select_assignments(&left, 1);
    assert_eq!(chosen.len(), 1);
    assert_eq!(left[chosen[0]].target, "salvage");
}

// ── Config validation ────────────────────────────────────────────────────

fn config() -> SecurityConfig {
    SecurityConfig {
        team_count: 2,
        deploy_duration_secs: 6.0,
        withdraw_duration_secs: 4.0,
        range: 400.0,
    }
}

#[test]
fn a_well_formed_security_config_validates() {
    assert!(config().validate().is_ok());
}

#[test]
fn a_teamless_hull_a_negative_crossing_or_a_zero_reach_is_rejected() {
    assert!(SecurityConfig {
        team_count: 0,
        ..config()
    }
    .validate()
    .is_err());
    assert!(SecurityConfig {
        deploy_duration_secs: -1.0,
        ..config()
    }
    .validate()
    .is_err());
    assert!(SecurityConfig {
        withdraw_duration_secs: f32::NAN,
        ..config()
    }
    .validate()
    .is_err());
    assert!(SecurityConfig {
        range: 0.0,
        ..config()
    }
    .validate()
    .is_err());
}

#[test]
fn the_security_config_round_trips_through_toml() {
    let authored = r#"
team_count = 2
deploy_duration_secs = 6.0
withdraw_duration_secs = 4.0
range = 400.0
"#;
    let parsed: SecurityConfig = toml::from_str(authored).expect("security config parses");
    assert_eq!(parsed, config());
    parsed.validate().expect("valid");
}

#[test]
fn a_misspelt_security_field_is_a_parse_error_rather_than_a_silent_default() {
    let err = toml::from_str::<SecurityConfig>(
        "team_count = 2\ndeploy_duration_secs = 6.0\nwithdraw_duration_secs = 4.0\nrng = 400.0",
    )
    .expect_err("a misspelt field must not be swallowed");
    assert!(err.to_string().contains("rng"), "got {err}");
}

#[test]
fn a_target_table_round_trips_and_defaults_its_outcome_value() {
    let authored = r#"
[[action]]
action = "assist_evacuation"
duration_secs = 20.0
risk = 0.4
priority = "life_safety"
outcome_flag = "compartment_evacuated"

[[action]]
action = "secure_contain"
duration_secs = 30.0
risk = 0.6
priority = "threat_containment"
outcome_flag = "fire_contained"
outcome_value = 2
warning = "security.warning.fire"
"#;
    let parsed: SecurityTargetConfig =
        toml::from_str(authored).expect("security target config parses");
    parsed.validate().expect("valid");
    assert_eq!(parsed.actions.len(), 2);
    let evac = parsed
        .action(SecurityAction::AssistEvacuation)
        .expect("the evacuation action is offered");
    assert_eq!(evac.priority, SecurityPriority::LifeSafety);
    assert_eq!(
        evac.outcome_value, 1,
        "the plain boolean flag is the default"
    );
    assert_eq!(evac.warning, None);
    let contain = parsed
        .action(SecurityAction::SecureContain)
        .expect("the containment action is offered");
    assert_eq!(contain.outcome_value, 2);
    assert_eq!(contain.warning.as_deref(), Some("security.warning.fire"));
    assert_eq!(parsed.action(SecurityAction::Board), None);
}

#[test]
fn an_empty_or_duplicated_target_table_is_rejected() {
    assert!(SecurityTargetConfig { actions: vec![] }.validate().is_err());
    let duplicated = SecurityTargetConfig {
        actions: vec![
            action_config(SecurityAction::Board),
            action_config(SecurityAction::Board),
        ],
    };
    let err = duplicated
        .validate()
        .expect_err("a repeated verb is unreachable");
    assert!(err.contains("board"), "the error must name the verb: {err}");
}

fn action_config(action: SecurityAction) -> SecurityActionConfig {
    SecurityActionConfig {
        action,
        duration_secs: 10.0,
        risk: 0.5,
        priority: SecurityPriority::Optional,
        outcome_flag: None,
        outcome_value: 1,
        warning: None,
    }
}

#[test]
fn an_instant_action_an_out_of_band_risk_or_a_blank_flag_is_rejected() {
    assert!(SecurityActionConfig {
        duration_secs: 0.0,
        ..action_config(SecurityAction::Board)
    }
    .validate()
    .is_err());
    assert!(SecurityActionConfig {
        risk: 1.5,
        ..action_config(SecurityAction::Board)
    }
    .validate()
    .is_err());
    assert!(SecurityActionConfig {
        risk: -0.1,
        ..action_config(SecurityAction::Board)
    }
    .validate()
    .is_err());
    assert!(SecurityActionConfig {
        outcome_flag: Some("   ".into()),
        ..action_config(SecurityAction::Board)
    }
    .validate()
    .is_err());
    assert!(SecurityActionConfig {
        warning: Some("".into()),
        ..action_config(SecurityAction::Board)
    }
    .validate()
    .is_err());
}

#[test]
fn an_unknown_authored_action_id_is_a_parse_error() {
    let err = toml::from_str::<SecurityTargetConfig>(
        "[[action]]\naction = \"vent_the_deck\"\nduration_secs = 5.0\nrisk = 0.1\npriority = \
             \"optional\"\n",
    )
    .expect_err("an unimplemented verb must be refused at load");
    assert!(err.to_string().contains("vent_the_deck"), "got {err}");
}
