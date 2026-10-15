use super::*;
use phoenix_model::messages::SystemId;
fn policy(batch: i64, fraction: f32) -> HandlingConfig {
    HandlingConfig {
        system: SystemId("logistics".into()),
        max_active: 1,
        max_batch: batch,
        units_per_sec: 1.0,
        reorder_fraction: fraction,
    }
}
fn observed(id: &str, occupied: i64, target: i64, priority: u16) -> ReplenishmentObservation {
    ReplenishmentObservation {
        store: id.into(),
        occupied,
        target,
        priority,
        replenishing: false,
    }
}
fn ids(rows: &[ReplenishmentObservation], policy: &HandlingConfig) -> Vec<String> {
    ordered_replenishment_candidates(rows, policy)
        .unwrap()
        .into_iter()
        .map(|v| v.store)
        .collect()
}

#[test]
fn batch_and_fraction_thresholds_are_inclusive_without_overordering_full_stores() {
    let rows = [
        observed("batch", 6, 10, 0),
        observed("under-batch", 7, 10, 0),
        observed("fraction", 5, 10, 0),
        observed("empty", 0, 1, 0),
        observed("full", 10, 10, 0),
        observed("zero", 0, 0, 0),
    ];
    assert_eq!(
        ids(&rows, &policy(4, 0.5)),
        vec!["empty", "fraction", "batch"]
    );
    assert_eq!(
        ids(&rows, &policy(4, 1.0)),
        vec!["empty", "fraction", "batch", "under-batch"]
    );
    assert_eq!(
        ids(
            &[observed("boundary", 1, 2, 0), observed("above", 2, 3, 0)],
            &policy(4, 0.5)
        ),
        vec!["boundary"]
    );
    assert_eq!(
        ids(
            &[observed("empty", 0, 1, 0), observed("small", 1, 2, 0)],
            &policy(4, 0.0)
        ),
        vec!["empty"]
    );
}
#[test]
fn priorities_then_exact_fullness_then_stable_identity_determine_order() {
    let rows = [
        observed("late", 0, 10, 2),
        observed("high", 9, 10, 0),
        observed("b", 2, 4, 1),
        observed("a", 1, 2, 1),
        observed("emptier", 1, 3, 1),
    ];
    assert_eq!(
        ids(&rows, &policy(1, 1.0)),
        vec!["high", "emptier", "a", "b", "late"]
    );
    let mut reversed = rows.to_vec();
    reversed.reverse();
    assert_eq!(ids(&rows, &policy(1, 1.0)), ids(&reversed, &policy(1, 1.0)));
    // Both ratios round to 1.0 in floating-point, but differ by real stock.
    let huge = [
        observed("a-fuller", i64::MAX - 1, i64::MAX, 0),
        observed("z-emptier", i64::MAX - 2, i64::MAX, 0),
    ];
    assert_eq!(ids(&huge, &policy(1, 1.0)), vec!["z-emptier", "a-fuller"]);
}
#[test]
fn exact_reorder_threshold_handles_large_odd_targets_and_subnormal_authored_values() {
    let target = i64::MAX;
    let threshold = target / 2;
    let rows = [
        observed("at", threshold, target, 0),
        observed("over", threshold + 1, target, 0),
    ];
    assert_eq!(ids(&rows, &policy(i64::MAX, 0.5)), vec!["at"]);
    assert_eq!(reorder_threshold(target, 1.0), target);
    assert_eq!(reorder_threshold(target, f32::from_bits(1)), 0);
    assert_eq!(
        ids(
            &[
                observed("empty", 0, target, 0),
                observed("one", 1, target, 0)
            ],
            &policy(i64::MAX, f32::from_bits(1))
        ),
        vec!["empty"]
    );
}
#[test]
fn invalid_observations_and_policy_refuse_instead_of_ranking_hidden_or_malformed_values() {
    for rows in [
        vec![observed("same", 1, 2, 0), observed("same", 0, 2, 1)],
        vec![observed("bad", -1, 2, 0)],
        vec![observed("bad", 0, -1, 0)],
    ] {
        assert!(ordered_replenishment_candidates(&rows, &policy(1, 1.0)).is_err());
    }
    for fraction in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(ordered_replenishment_candidates(&[], &policy(1, fraction)).is_err());
    }
    assert!(ordered_replenishment_candidates(&[], &policy(0, 1.0)).is_err());
}
