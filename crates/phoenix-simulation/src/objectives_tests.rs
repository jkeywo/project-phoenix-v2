use super::*;
use crate::core::messages::AiDirective;
use crate::core::messages::ObjectiveSnapshot;
use crate::core::messages::ObjectiveSource;
use crate::core::messages::ObjectiveStatus;
use crate::core::messages::ScoredObjective;
use crate::core::messages::StationId;
use crate::core::messages::SystemAffinity;
use crate::ship::config::StationStanceConfig;
use std::collections::BTreeMap;

// ── issue #1162: operate-directive relevance + selection ────────────────

#[test]
fn tractor_verbs_route_to_engineering_only() {
    for d in [
        AiDirective::Tow { target: "t".into() },
        AiDirective::Stabilise { target: "t".into() },
        AiDirective::Escort { target: "t".into() },
    ] {
        assert_eq!(
            directive_relevance(&d),
            vec![SystemAffinity::Engineering],
            "tractor verbs route to Engineering, the tractor's owner"
        );
    }
}

#[test]
fn transfer_routes_to_both_helm_and_engineering() {
    assert_eq!(
        directive_relevance(&AiDirective::Transfer { target: "t".into() }),
        vec![SystemAffinity::Helm, SystemAffinity::Engineering],
        "Transfer is a two-seat chain: Helm docks, Engineering runs the umbilical"
    );
}

#[test]
fn field_repair_routes_to_repair_only() {
    assert_eq!(
        directive_relevance(&AiDirective::FieldRepair { target: "t".into() }),
        vec![SystemAffinity::Repair],
    );
}

#[test]
fn order_routes_to_navigation_and_projects_its_payload() {
    let directive = AiDirective::Order {
        target: "Meridian Freight".into(),
        route: "storm_shelter_run".into(),
    };

    assert_eq!(
        directive_relevance(&directive),
        vec![SystemAffinity::Navigation],
        "ordering civilian traffic belongs to Navigation, not player-ship Helm"
    );
    assert_eq!(
        order_directive(&directive),
        Some(("Meridian Freight", "storm_shelter_run"))
    );
    assert_eq!(order_directive(&AiDirective::None), None);
}

#[test]
fn scan_routes_to_sensors_only_and_exposes_its_target() {
    let directive = AiDirective::Scan {
        target: "Ladder Depot B".into(),
    };
    assert_eq!(
        directive_relevance(&directive),
        vec![SystemAffinity::Sensors]
    );
    assert_eq!(scan_directive_target(&directive), Some("Ladder Depot B"));
    assert_eq!(
        scan_directive_target(&AiDirective::Hail {
            target: "Control".into()
        }),
        None
    );
}

fn scored_dir(id: &str, score: f32, directive: AiDirective) -> ScoredObjective {
    let relevance = directive_relevance(&directive);
    ScoredObjective {
        id: id.into(),
        score,
        directive,
        source: ObjectiveSource::Mission,
        relevance,
        snapshot: ObjectiveSnapshot {
            progress: None,
            unassigned: false,
            id: id.into(),
            text: String::new(),
            text_params: BTreeMap::new(),
            mandatory: false,
            status: ObjectiveStatus::Active,
            targets: vec![],
            source: ObjectiveSource::Mission,
        },
    }
}

#[test]
fn top_operate_directive_picks_the_first_relevant_positive_match() {
    // Pool is sorted descending by score (as `scored_pool` leaves it).
    let pool = vec![
        scored_dir(
            "tow",
            10.0,
            AiDirective::Tow {
                target: "hulk".into(),
            },
        ),
        scored_dir(
            "tow2",
            5.0,
            AiDirective::Tow {
                target: "other".into(),
            },
        ),
    ];
    let picked = top_operate_directive(&pool, SystemAffinity::Engineering, |d| {
        tractor_directive_target(d).is_some()
    });
    assert_eq!(picked.and_then(tractor_directive_target), Some("hulk"));
}

#[test]
fn top_operate_directive_skips_zero_score_and_wrong_affinity() {
    let pool = vec![
        // Zero score — skipped even though the kind matches.
        scored_dir("z", 0.0, AiDirective::Tow { target: "a".into() }),
        // A FieldRepair (Repair affinity) is invisible to an Engineering query.
        scored_dir("fr", 9.0, AiDirective::FieldRepair { target: "b".into() }),
    ];
    assert!(
        top_operate_directive(&pool, SystemAffinity::Engineering, |d| {
            tractor_directive_target(d).is_some()
        })
        .is_none()
    );
    // The FieldRepair IS visible to a Repair query.
    assert_eq!(
        top_operate_directive(&pool, SystemAffinity::Repair, |d| {
            field_repair_directive_target(d).is_some()
        })
        .and_then(field_repair_directive_target),
        Some("b")
    );
}

#[test]
fn transfer_reaches_both_seats_but_not_a_tractor_query() {
    let pool = vec![scored_dir(
        "xf",
        7.0,
        AiDirective::Transfer {
            target: "tender".into(),
        },
    )];
    // Both the Helm dock seat and the Engineering umbilical seat see it.
    assert_eq!(
        top_operate_directive(&pool, SystemAffinity::Helm, |d| {
            transfer_directive_target(d).is_some()
        })
        .and_then(transfer_directive_target),
        Some("tender")
    );
    assert_eq!(
        top_operate_directive(&pool, SystemAffinity::Engineering, |d| {
            transfer_directive_target(d).is_some()
        })
        .and_then(transfer_directive_target),
        Some("tender")
    );
    // But a tractor query (Engineering, tractor verbs) does NOT — Transfer
    // is not a tractor verb, so it never pulls a lock.
    assert!(
        top_operate_directive(&pool, SystemAffinity::Engineering, |d| {
            tractor_directive_target(d).is_some()
        })
        .is_none()
    );
}

#[test]
fn engineering_seat_defers_rescue_to_a_higher_scored_tractor_obligation() {
    // Both orders sit on the one Engineering seat. The tractor Stabilise
    // outscores the rescue, so the seat winner is the Stabilise.
    let pool = vec![
        scored_dir(
            "stab",
            9.0,
            AiDirective::Stabilise {
                target: "tender".into(),
            },
        ),
        scored_dir(
            "rescue",
            4.0,
            AiDirective::Rescue {
                target: "lifeboat".into(),
            },
        ),
    ];
    let seat = |d: &AiDirective| engineering_seat_operate_target(d).is_some();
    let winner = top_operate_directive(&pool, SystemAffinity::Engineering, seat);
    // Transporter host keeps only a rescue winner → stands down here.
    assert_eq!(winner.and_then(rescue_directive_target), None);
    // Tractor host keeps the tractor winner → holds the seat.
    assert_eq!(winner.and_then(tractor_directive_target), Some("tender"));
}

#[test]
fn engineering_seat_gives_a_higher_scored_rescue_the_seat() {
    // Reverse the scores: the rescue now outranks the tractor obligation.
    let pool = vec![
        scored_dir(
            "rescue",
            9.0,
            AiDirective::Rescue {
                target: "lifeboat".into(),
            },
        ),
        scored_dir(
            "stab",
            4.0,
            AiDirective::Stabilise {
                target: "tender".into(),
            },
        ),
    ];
    let seat = |d: &AiDirective| engineering_seat_operate_target(d).is_some();
    let winner = top_operate_directive(&pool, SystemAffinity::Engineering, seat);
    // Rescue wins the seat; the tractor host stands down.
    assert_eq!(winner.and_then(rescue_directive_target), Some("lifeboat"));
    assert_eq!(winner.and_then(tractor_directive_target), None);
}

#[test]
fn empty_manager_returns_no_snapshots() {
    let mgr = ObjectiveManager::new();
    assert!(mgr.sorted_snapshots().is_empty());
}

#[test]
fn empty_manager_is_not_dirty() {
    let mgr = ObjectiveManager::new();
    assert!(!mgr.is_dirty());
}

#[test]
fn add_objective_appears_in_snapshots_as_active() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Destroy the convoy", true, vec![]);
    let snapshots = mgr.sorted_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].id, "obj-1");
    assert_eq!(snapshots[0].text, "Destroy the convoy");
    assert!(snapshots[0].mandatory);
    assert_eq!(snapshots[0].status, ObjectiveStatus::Active);
}

#[test]
fn add_objective_marks_dirty() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Text", false, vec![]);
    assert!(mgr.is_dirty());
}

#[test]
fn mark_clean_clears_dirty_flag() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Text", false, vec![]);
    mgr.mark_clean();
    assert!(!mgr.is_dirty());
}

#[test]
fn adding_duplicate_id_is_noop() {
    let mut mgr = ObjectiveManager::new();
    let first = mgr.add("obj-1", "First", true, vec![]);
    let second = mgr.add("obj-1", "Second", false, vec![]);
    assert!(first);
    assert!(!second);
    assert_eq!(mgr.sorted_snapshots().len(), 1);
    assert_eq!(mgr.sorted_snapshots()[0].text, "First");
}

#[test]
fn mandatory_objectives_sort_before_optional() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("opt-1", "Optional A", false, vec![]);
    mgr.add("man-1", "Mandatory A", true, vec![]);
    mgr.add("opt-2", "Optional B", false, vec![]);
    mgr.add("man-2", "Mandatory B", true, vec![]);

    let snaps = mgr.sorted_snapshots();
    assert_eq!(snaps.len(), 4);
    assert!(snaps[0].mandatory);
    assert!(snaps[1].mandatory);
    assert!(!snaps[2].mandatory);
    assert!(!snaps[3].mandatory);
    assert_eq!(snaps[0].id, "man-1");
    assert_eq!(snaps[1].id, "man-2");
    assert_eq!(snaps[2].id, "opt-1");
    assert_eq!(snaps[3].id, "opt-2");
}

#[test]
fn complete_transitions_active_to_completed() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Destroy convoy", true, vec![]);
    mgr.mark_clean();

    let result = mgr.complete("obj-1");
    assert!(result);
    assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Completed);
    assert!(mgr.is_dirty());
}

#[test]
fn complete_returns_false_for_unknown_id() {
    let mut mgr = ObjectiveManager::new();
    let result = mgr.complete("nonexistent");
    assert!(!result);
    assert!(!mgr.is_dirty());
}

#[test]
fn complete_returns_false_if_already_completed() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Text", true, vec![]);
    mgr.complete("obj-1");
    mgr.mark_clean();

    let result = mgr.complete("obj-1");
    assert!(!result);
    assert!(!mgr.is_dirty());
}

#[test]
fn fail_transitions_active_to_failed() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Save the station", true, vec![]);
    mgr.mark_clean();

    let result = mgr.fail("obj-1");
    assert!(result);
    assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Failed);
    assert!(mgr.is_dirty());
}

#[test]
fn fail_returns_false_for_unknown_id() {
    let mut mgr = ObjectiveManager::new();
    assert!(!mgr.fail("ghost"));
    assert!(!mgr.is_dirty());
}

#[test]
fn fail_returns_false_if_already_failed() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Text", true, vec![]);
    mgr.fail("obj-1");
    mgr.mark_clean();
    assert!(!mgr.fail("obj-1"));
    assert!(!mgr.is_dirty());
}

#[test]
fn add_objective_stores_targets() {
    let mut mgr = ObjectiveManager::new();
    mgr.add(
        "obj-1",
        "Destroy Ironveil",
        true,
        vec!["Ironveil".to_string()],
    );
    let snaps = mgr.sorted_snapshots();
    assert_eq!(snaps[0].targets, vec!["Ironveil".to_string()]);
}

#[test]
fn add_objective_stores_multiple_targets() {
    let mut mgr = ObjectiveManager::new();
    mgr.add(
        "obj-1",
        "Hail Axiom Station or Research Outpost",
        false,
        vec!["Axiom Station".to_string(), "Research Outpost".to_string()],
    );
    let snaps = mgr.sorted_snapshots();
    assert_eq!(
        snaps[0].targets,
        vec!["Axiom Station".to_string(), "Research Outpost".to_string()]
    );
}

#[test]
fn add_objective_without_targets_is_empty() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Survive", true, vec![]);
    let snaps = mgr.sorted_snapshots();
    assert!(snaps[0].targets.is_empty());
}

#[test]
fn add_objective_without_targets_is_empty_vec() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Survive", true, vec![]);
    let snaps = mgr.sorted_snapshots();
    assert!(snaps[0].targets.is_empty());
}

// ── scored_pool tests (issue #571) ─────────────────────────────────────

#[test]
fn scored_pool_empty_when_no_objectives() {
    let mgr = ObjectiveManager::new();
    assert!(mgr.scored_pool(&WorldConditions::default()).is_empty());
}

#[test]
fn scored_pool_excludes_completed_and_failed() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("a", "Active", false, vec![]);
    mgr.add("b", "Done", false, vec![]);
    mgr.complete("b");
    mgr.add("c", "Failed", false, vec![]);
    mgr.fail("c");
    let pool = mgr.scored_pool(&WorldConditions::default());
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].id, "a");
}

#[test]
fn scored_pool_base_priority_is_score() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "obj-1",
        "Patrol",
        false,
        vec![],
        AiDirective::Patrol {
            anchors: vec!["alpha".into()],
            loop_path: true,
        },
        UtilityConfig {
            base_priority: 40.0,
            ..Default::default()
        },
        ObjectiveSource::Mission,
    );
    let pool = mgr.scored_pool(&WorldConditions::default());
    assert!((pool[0].score - 40.0).abs() < f32::EPSILON);
}

#[test]
fn mandatory_bonus_added_to_score() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "obj-1",
        "Mandatory patrol",
        true,
        vec![],
        AiDirective::default(),
        UtilityConfig {
            base_priority: 30.0,
            ..Default::default()
        },
        ObjectiveSource::Mission,
    );
    let pool = mgr.scored_pool(&WorldConditions::default());
    assert!((pool[0].score - (30.0 + MANDATORY_BONUS)).abs() < f32::EPSILON);
}

#[test]
fn zero_gate_forces_score_to_zero_when_condition_false() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "obj-1",
        "Flee",
        false,
        vec![],
        AiDirective::default(),
        UtilityConfig {
            base_priority: 80.0,
            zero_gates: vec![ZeroGateCondition {
                condition: "hull_below".into(),
                threshold: Some(0.3),
            }],
            ..Default::default()
        },
        ObjectiveSource::Doctrine,
    );
    // Hull is 1.0 → hull_below(0.3) is false → gate fails → score = 0
    let pool = mgr.scored_pool(&WorldConditions {
        hull_fraction: 1.0,
        ..Default::default()
    });
    assert_eq!(pool[0].score, 0.0);
}

#[test]
fn zero_gate_passes_when_condition_true() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "obj-1",
        "Flee",
        false,
        vec![],
        AiDirective::default(),
        UtilityConfig {
            base_priority: 80.0,
            zero_gates: vec![ZeroGateCondition {
                condition: "hull_below".into(),
                threshold: Some(0.3),
            }],
            ..Default::default()
        },
        ObjectiveSource::Doctrine,
    );
    // Hull is 0.2 → hull_below(0.3) is true → gate passes → full score
    let pool = mgr.scored_pool(&WorldConditions {
        hull_fraction: 0.2,
        ..Default::default()
    });
    assert!((pool[0].score - 80.0).abs() < f32::EPSILON);
}

#[test]
fn modifier_adds_weight_when_condition_true() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "obj-1",
        "Attack",
        false,
        vec![],
        AiDirective::Destroy {
            target: "enemy".into(),
        },
        UtilityConfig {
            base_priority: 50.0,
            modifiers: vec![ConditionModifier {
                condition: "red_alert".into(),
                threshold: None,
                weight: 20.0,
            }],
            ..Default::default()
        },
        ObjectiveSource::Doctrine,
    );
    let pool = mgr.scored_pool(&WorldConditions {
        red_alert: true,
        hull_fraction: 1.0,
        attacked: false,
    });
    assert!((pool[0].score - 70.0).abs() < f32::EPSILON);
}

#[test]
fn modifier_skipped_when_condition_false() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "obj-1",
        "Attack",
        false,
        vec![],
        AiDirective::Destroy {
            target: "enemy".into(),
        },
        UtilityConfig {
            base_priority: 50.0,
            modifiers: vec![ConditionModifier {
                condition: "red_alert".into(),
                threshold: None,
                weight: 20.0,
            }],
            ..Default::default()
        },
        ObjectiveSource::Doctrine,
    );
    let pool = mgr.scored_pool(&WorldConditions {
        red_alert: false,
        hull_fraction: 1.0,
        attacked: false,
    });
    assert!((pool[0].score - 50.0).abs() < f32::EPSILON);
}

#[test]
fn scored_pool_sorted_descending_by_score() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "low",
        "Low priority",
        false,
        vec![],
        AiDirective::default(),
        UtilityConfig {
            base_priority: 10.0,
            ..Default::default()
        },
        ObjectiveSource::Doctrine,
    );
    mgr.add_full(
        "high",
        "High priority",
        false,
        vec![],
        AiDirective::default(),
        UtilityConfig {
            base_priority: 60.0,
            ..Default::default()
        },
        ObjectiveSource::Mission,
    );
    let pool = mgr.scored_pool(&WorldConditions::default());
    assert_eq!(pool[0].id, "high");
    assert_eq!(pool[1].id, "low");
}

#[test]
fn captain_priority_selection_outranks_a_higher_authored_score() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "patrol",
        "Patrol",
        false,
        vec![],
        AiDirective::Patrol {
            anchors: vec!["alpha".into()],
            loop_path: true,
        },
        UtilityConfig {
            base_priority: 90.0,
            ..Default::default()
        },
        ObjectiveSource::Mission,
    );
    mgr.add_full(
        "destroy-priority",
        "Destroy priority target",
        false,
        vec!["priority-target".into()],
        AiDirective::Destroy {
            target: "priority-target".into(),
        },
        UtilityConfig {
            base_priority: 10.0,
            ..Default::default()
        },
        ObjectiveSource::Mission,
    );

    let pool = mgr.scored_pool_with_boost(&WorldConditions::default(), Some("destroy-priority"));

    assert_eq!(pool[0].id, "destroy-priority");
    assert_eq!(pool[0].score, f32::MAX);
}

#[test]
fn patrol_directive_has_helm_relevance() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "p",
        "Patrol",
        false,
        vec![],
        AiDirective::Patrol {
            anchors: vec!["a".into()],
            loop_path: false,
        },
        UtilityConfig {
            base_priority: 1.0,
            ..Default::default()
        },
        ObjectiveSource::Mission,
    );
    let pool = mgr.scored_pool(&WorldConditions::default());
    assert_eq!(pool[0].relevance, vec![SystemAffinity::Helm]);
}

#[test]
fn destroy_directive_has_helm_weapons_captain_relevance() {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "d",
        "Destroy",
        false,
        vec![],
        AiDirective::Destroy {
            target: "target".into(),
        },
        UtilityConfig {
            base_priority: 1.0,
            ..Default::default()
        },
        ObjectiveSource::Mission,
    );
    let pool = mgr.scored_pool(&WorldConditions::default());
    assert_eq!(
        pool[0].relevance,
        vec![
            SystemAffinity::Helm,
            SystemAffinity::Weapons,
            SystemAffinity::Captain
        ]
    );
}

#[test]
fn none_directive_has_no_relevance() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj", "No directive", false, vec![]);
    let pool = mgr.scored_pool(&WorldConditions::default());
    assert!(pool[0].relevance.is_empty());
}

#[test]
fn hail_directive_has_comms_relevance() {
    // Issue #753: Hail is a Comms action, so the Backfill Comms AI can
    // consume it from the scored pool by affinity.
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "h",
        "Hail",
        false,
        vec![],
        AiDirective::Hail {
            target: "Station Alpha".into(),
        },
        UtilityConfig {
            base_priority: 1.0,
            ..Default::default()
        },
        ObjectiveSource::Mission,
    );
    let pool = mgr.scored_pool(&WorldConditions::default());
    assert_eq!(pool[0].relevance, vec![SystemAffinity::Comms]);
}

// ── remove clears runtime record (issue #751/#752) ─────────────────────

#[test]
fn remove_drops_record_and_marks_dirty() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Text", true, vec![]);
    mgr.mark_clean();
    assert!(mgr.remove("obj-1"));
    assert!(mgr.sorted_snapshots().is_empty());
    assert!(mgr.scored_pool(&WorldConditions::default()).is_empty());
    assert!(mgr.is_dirty());
}

// ── The transition log (issue #1338) ───────────────────────────────────

/// The log records every mutation in the order it was made, and the whole
/// reason it exists is that the *status field* cannot: an objective added
/// and completed before anyone reads the manager has one status and two
/// transitions.
#[test]
fn the_transition_log_records_every_mutation_in_order() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "world.probe.objective.one", true, vec![]);
    mgr.complete("obj-1");
    mgr.add("obj-2", "world.probe.objective.two", false, vec![]);
    mgr.fail("obj-2");

    let log = mgr.drain_transitions();
    let shape: Vec<(&str, ObjectiveTransitionKind)> =
        log.iter().map(|t| (t.id.as_str(), t.kind)).collect();
    assert_eq!(
        shape,
        vec![
            ("obj-1", ObjectiveTransitionKind::Posted),
            ("obj-1", ObjectiveTransitionKind::Completed),
            ("obj-2", ObjectiveTransitionKind::Posted),
            ("obj-2", ObjectiveTransitionKind::Failed),
        ]
    );
    // Each entry carries the objective's own fields, so a reader never has
    // to look the record back up.
    assert_eq!(log[0].text, "world.probe.objective.one");
    assert!(log[0].mandatory);
    assert!(!log[3].mandatory);

    // Draining empties it: it is a per-tick buffer, not a second copy of
    // the objective set.
    assert!(mgr.drain_transitions().is_empty());
}

/// A call that changes nothing logs nothing — a duplicate id, a completion
/// of an already-completed objective, a removal of a ghost. Otherwise the
/// timeline would carry beats for events that did not happen.
#[test]
fn no_op_calls_log_no_transition() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Text", true, vec![]);
    let _ = mgr.drain_transitions();

    assert!(!mgr.add("obj-1", "Text again", false, vec![]));
    assert!(mgr.complete("obj-1"));
    assert!(!mgr.complete("obj-1"));
    assert!(!mgr.fail("obj-1"));
    assert!(!mgr.remove("ghost"));

    let log = mgr.drain_transitions();
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0].kind, ObjectiveTransitionKind::Completed);
}

/// A removal is logged off the record BEFORE it is dropped, so the entry
/// still carries the objective's fields — which is what lets the recorder
/// reconcile an objective posted and removed inside one tick.
#[test]
fn a_removal_is_logged_with_the_record_it_dropped() {
    let mut mgr = ObjectiveManager::new();
    mgr.add(
        "obj-1",
        "world.probe.objective.one",
        true,
        vec!["uuid-a".into()],
    );
    assert!(mgr.remove("obj-1"));

    let log = mgr.drain_transitions();
    assert_eq!(log.len(), 2, "{log:?}");
    assert_eq!(log[1].kind, ObjectiveTransitionKind::Removed);
    assert_eq!(log[1].text, "world.probe.objective.one");
    assert_eq!(log[1].targets, vec!["uuid-a".to_string()]);
}

#[test]
fn remove_unknown_id_is_noop() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "Text", true, vec![]);
    mgr.mark_clean();
    assert!(!mgr.remove("ghost"));
    assert!(!mgr.is_dirty());
}

#[test]
fn removed_id_can_be_re_added_fresh() {
    // After removal the id is free again: a re-add is a genuine insert, not
    // a dedup no-op — so an unloaded-then-reloaded layer re-registers its
    // objective cleanly (#752 lifecycle).
    let mut mgr = ObjectiveManager::new();
    mgr.add("obj-1", "First", true, vec![]);
    mgr.remove("obj-1");
    assert!(mgr.add("obj-1", "Second", false, vec![]));
    let snaps = mgr.sorted_snapshots();
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0].text, "Second");
}

// ── objective-contributed Command stances (issue #1110) ────────────────

fn objective_stance() -> (StationId, StationStanceConfig) {
    (
        StationId("tactical".into()),
        StationStanceConfig {
            id: "objective-escort".into(),
            label: String::new(),
            kind: crate::ship::config::StanceKind::Standard,
            high_alert: true,
            persist_behind_human: true,
            ai_engaged: false,
        },
    )
}

fn add_with_stance(
    mgr: &mut ObjectiveManager,
    id: &str,
    stance: Option<(StationId, StationStanceConfig)>,
) {
    mgr.add_full_with_params(
        id,
        "text",
        BTreeMap::new(),
        false,
        vec![],
        AiDirective::None,
        UtilityConfig::default(),
        ObjectiveSource::Mission,
        stance,
    );
}

#[test]
fn a_contributed_stance_is_exposed_only_while_active() {
    let mut mgr = ObjectiveManager::new();
    add_with_stance(&mut mgr, "escort", Some(objective_stance()));
    // Active → the contribution is exposed against its target Station.
    assert_eq!(mgr.active_station_stances(), vec![objective_stance()]);
}

#[test]
fn an_objective_without_a_stance_contributes_nothing() {
    let mut mgr = ObjectiveManager::new();
    add_with_stance(&mut mgr, "plain", None);
    assert!(mgr.active_station_stances().is_empty());
}

#[test]
fn recipient_scope_filters_crew_ai_and_stances_without_changing_subjects() {
    let mut mgr = ObjectiveManager::new();
    mgr.add("legacy", "Shared", false, vec!["subject-contact".into()]);
    add_with_stance(&mut mgr, "escort", Some(objective_stance()));
    assert!(mgr.set_recipients(
        "escort",
        vec!["ship-b".into(), "ship-a".into(), "ship-a".into()]
    ));
    assert_eq!(mgr.recipients("escort").unwrap(), &["ship-a", "ship-b"]);
    assert!(!mgr.set_recipients("escort", vec!["ship-c".into()]));
    assert_eq!(mgr.targets("legacy").unwrap(), &["subject-contact"]);
    let conditions = WorldConditions::default();
    for ship in ["ship-a", "ship-b"] {
        assert_eq!(mgr.snapshots_for(ship).len(), 2);
        assert_eq!(mgr.scored_pool_for(&conditions, ship).len(), 2);
        assert_eq!(
            mgr.active_station_stances_for(ship),
            vec![objective_stance()]
        );
    }
    for ship in ["ship-c", ""] {
        assert_eq!(
            mgr.snapshots_for(ship)
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            vec!["legacy"]
        );
        assert_eq!(
            mgr.scored_pool_with_boost_for(&conditions, Some("escort"), ship)
                .len(),
            1
        );
        assert!(mgr.active_station_stances_for(ship).is_empty());
    }
    assert!(mgr.complete("escort"));
    assert_eq!(mgr.status("escort"), Some(&ObjectiveStatus::Completed));
    assert_eq!(
        mgr.snapshots_for("ship-a").len(),
        2,
        "terminal record stays visible to intended recipients"
    );
    assert_eq!(mgr.scored_pool_for(&conditions, "ship-a").len(), 1);
    assert!(mgr.active_station_stances_for("ship-a").is_empty());
}

#[test]
fn restoring_objective_records_preserves_authored_state_and_no_reopen_or_story_beats() {
    let mut mgr = ObjectiveManager::new();
    add_with_stance(&mut mgr, "escort", Some(objective_stance()));
    mgr.set_recipients("escort", vec!["ship-a".into()]);
    mgr.fail("escort");
    let wire = serde_json::to_vec(mgr.records()).unwrap();
    let mut restored = ObjectiveManager::new();
    restored.add("bootstrap-only", "Bootstrap", false, vec![]);
    restored.restore_records(serde_json::from_slice(&wire).unwrap());
    assert_eq!(restored.records(), mgr.records());
    assert!(restored.drain_transitions().is_empty());
    assert!(!restored.add("escort", "Cannot reopen", false, vec![]));
    assert!(!restored.complete("escort"));
    assert!(!restored.fail("escort"));
    assert!(!restored.set_recipients("escort", vec!["ship-b".into()]));
    assert_eq!(restored.recipients("escort").unwrap(), &["ship-a"]);
    assert!(restored.status("bootstrap-only").is_none());
}

#[test]
fn completing_the_objective_withdraws_its_contribution() {
    let mut mgr = ObjectiveManager::new();
    add_with_stance(&mut mgr, "escort", Some(objective_stance()));
    assert!(mgr.complete("escort"));
    assert!(
        mgr.active_station_stances().is_empty(),
        "a completed objective no longer contributes its stance",
    );
}

#[test]
fn failing_the_objective_withdraws_its_contribution() {
    let mut mgr = ObjectiveManager::new();
    add_with_stance(&mut mgr, "escort", Some(objective_stance()));
    assert!(mgr.fail("escort"));
    assert!(
        mgr.active_station_stances().is_empty(),
        "a failed objective no longer contributes its stance",
    );
}

#[test]
fn removing_the_objective_withdraws_its_contribution() {
    // Invalidation (`remove`) deletes the record outright, so its
    // contribution disappears the same way completion/failure withdraw it.
    let mut mgr = ObjectiveManager::new();
    add_with_stance(&mut mgr, "escort", Some(objective_stance()));
    assert!(mgr.remove("escort"));
    assert!(mgr.active_station_stances().is_empty());
}

// ── is_visible_objective (objective-visibility-policy, #752) ────────────

fn scored(id: &str, source: ObjectiveSource, base: f32) -> ScoredObjective {
    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        id,
        "text",
        false,
        vec![],
        AiDirective::None,
        UtilityConfig {
            base_priority: base,
            ..Default::default()
        },
        source,
    );
    mgr.scored_pool(&WorldConditions::default())
        .into_iter()
        .next()
        .unwrap()
}

#[test]
fn mission_objective_is_visible_even_at_zero_score() {
    let o = scored("m", ObjectiveSource::Mission, 0.0);
    assert_eq!(o.score, 0.0);
    assert!(is_visible_objective(&o));
}

#[test]
fn doctrine_objective_hidden_at_zero_score_visible_when_positive() {
    let zero = scored("d0", ObjectiveSource::Doctrine, 0.0);
    assert!(!is_visible_objective(&zero));
    let positive = scored("d1", ObjectiveSource::Doctrine, 5.0);
    assert!(is_visible_objective(&positive));
}

#[test]
fn scored_pool_is_total_ordered_on_equal_scores_by_insertion() {
    // Equal scores keep insertion order under the stable `total_cmp` sort,
    // giving a deterministic tiebreak the consumers rely on.
    let mut mgr = ObjectiveManager::new();
    mgr.add("first", "A", false, vec![]);
    mgr.add("second", "B", false, vec![]);
    let pool = mgr.scored_pool(&WorldConditions::default());
    assert_eq!(pool[0].id, "first");
    assert_eq!(pool[1].id, "second");
}

// ── attacked_recently (issue #1010) ────────────────────────────────────

/// The boundary both publish sites share. It lives here, on the pure
/// function, so the NPC aggregator and the LocalShip publisher cannot
/// drift apart on where the window ends: they call this, and this is what
/// is pinned.
#[test]
fn attacked_recency_window_opens_and_closes_on_the_boundary() {
    // Never damaged: nothing to decay from, so never under attack.
    assert!(!attacked_recently(None, 100.0, 8.0));
    // The hit itself, and everything strictly inside the window.
    assert!(attacked_recently(Some(100.0), 100.0, 8.0));
    assert!(attacked_recently(Some(100.0), 107.9, 8.0));
    // The far edge: at exactly the window the memory has expired, so the
    // raid resumes rather than hanging on for one more tick.
    assert!(!attacked_recently(Some(100.0), 108.0, 8.0));
    assert!(!attacked_recently(Some(100.0), 200.0, 8.0));
}

/// A designer authoring a zero (or negative) window means "no memory at
/// all" — a hit must not read as an attack even on the tick it lands.
#[test]
fn a_non_positive_attacked_window_never_reads_as_attacked() {
    assert!(!attacked_recently(Some(100.0), 100.0, 0.0));
    assert!(!attacked_recently(Some(100.0), 100.0, -1.0));
}

/// Shield-absorbed fire IS being attacked. `last_damage_taken` only moves
/// when the hull total drops, and shield-absorbed fire dominates early;
/// hull damage lands only after sustained pressure collapses an arc — so a
/// fold over hull damage alone would leave a Harrow under sustained
/// station fire reading "not attacked" for most of a short engagement and
/// flying its raid into the guns.
#[test]
fn shield_absorbed_hostile_fire_counts_as_a_landed_hit() {
    assert_eq!(last_landed_hit_secs(None, Some(100.0)), Some(100.0));
    assert!(attacked_recently(
        last_landed_hit_secs(None, Some(100.0)),
        103.0,
        8.0
    ));
}

/// The fold takes the MORE RECENT of the two readings either way round, so
/// a hull hit long ago and skimmed a moment ago still reads as under
/// attack (and not the other way about).
#[test]
fn the_landed_hit_fold_takes_the_more_recent_reading() {
    assert_eq!(last_landed_hit_secs(None, None), None);
    assert_eq!(last_landed_hit_secs(Some(100.0), None), Some(100.0));
    assert_eq!(last_landed_hit_secs(Some(100.0), Some(140.0)), Some(140.0));
    assert_eq!(last_landed_hit_secs(Some(140.0), Some(100.0)), Some(140.0));

    // Hull damage at 100, shields skimmed at 140, now 145: the stale hull
    // reading must not expire the window the fresh one keeps open.
    assert!(attacked_recently(
        last_landed_hit_secs(Some(100.0), Some(140.0)),
        145.0,
        8.0
    ));
}
