use super::*;

#[test]
fn every_shipped_task_verb_has_a_recorded_decision() {
    // The ratchet: a new TASK_VERB_* constant must be given an entry (and
    // therefore a reason) here rather than silently defaulting to "not a
    // demand" through the unknown-verb rule.
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/core/task_lifecycle.rs"
    ))
    .expect("task_lifecycle.rs is readable");
    let declared: Vec<String> = source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub const TASK_VERB_"))
        .filter_map(|rest| rest.split('"').nth(1).map(str::to_string))
        .collect();
    assert!(
        declared.len() >= 6,
        "expected the shipped verb vocabulary, found {declared:?}"
    );
    for verb in declared {
        assert!(
            TASK_DEMAND_INVENTORY.iter().any(|rule| rule.verb == verb),
            "task verb '{verb}' has no entry in TASK_DEMAND_INVENTORY: decide whether it can \
                 require a human and say why"
        );
    }
}

#[test]
fn no_shipped_task_verb_is_a_demand_and_an_unknown_verb_is_excluded() {
    for rule in TASK_DEMAND_INVENTORY {
        assert!(!rule.counts, "{} unexpectedly counts", rule.verb);
        assert!(!rule.why.is_empty(), "{} has no reason", rule.verb);
        assert!(!task_verb_counts(rule.verb));
    }
    // Per-team Security slots are `security_team_0`, `security_team_1`, …
    assert!(!task_verb_counts("security_team_1"));
    // And an activation this build has never heard of is unattributed
    // source state, which PRD #1419 excludes rather than guesses at.
    assert!(!task_verb_counts("teleport_the_admiral"));
}

#[test]
fn levels_say_whether_they_are_about_a_person() {
    assert!(!GmWorkloadLevel::Backfill.counts_people());
    assert!(!GmWorkloadLevel::Offline.counts_people());
    assert!(GmWorkloadLevel::Underused.counts_people());
    assert!(GmWorkloadLevel::Engaged.counts_people());
    assert!(GmWorkloadLevel::Overloaded.counts_people());
    assert_eq!(GmWorkloadLevel::Overloaded.as_str(), "overloaded");
}

#[test]
fn one_demand_reported_twice_occupies_one_slot() {
    let mut demands: StationDemands = BTreeMap::new();
    let station = StationId("engineering".into());
    let reason = GmAttentionReason {
        id: REPAIR_REASON.into(),
        params: BTreeMap::new(),
    };
    record(
        &mut demands,
        &station,
        "repair:ship/helm".into(),
        REPAIR_SOURCE,
        reason.clone(),
    );
    record(
        &mut demands,
        &station,
        "repair:ship/helm".into(),
        REPAIR_SOURCE,
        reason.clone(),
    );
    record(
        &mut demands,
        &station,
        "repair:ship/weapons".into(),
        REPAIR_SOURCE,
        reason,
    );
    assert_eq!(demands[&station.0].len(), 2);
}

#[test]
fn the_stopwatch_keys_and_reads_one_station_at_a_time() {
    let watch = GmWorkloadWatch {
        spells: BTreeMap::from([(GmWorkloadWatch::key("ship-a", "helm"), 12)]),
        nav_arrivals: BTreeMap::from([("ship-a".to_string(), 7)]),
    };
    assert_eq!(watch.ticks("ship-a", "helm"), 12);
    assert_eq!(watch.ticks("ship-a", "comms"), 0);
    assert_eq!(watch.len(), 1);
    assert!(!watch.is_empty());
    // The arrival latch is per HULL, not per Station: one course, one hull,
    // one answer to "has it been flown".
    assert_eq!(watch.nav_arrival("ship-a"), Some(7));
    assert_eq!(watch.nav_arrival("ship-b"), None);
}

#[test]
fn whole_simulation_seconds_are_floored_and_survive_a_broken_rate() {
    assert_eq!(sim_seconds(90, 30.0), 3);
    assert_eq!(sim_seconds(89, 30.0), 2);
    assert_eq!(sim_seconds(90, 0.0), 0);
}
