use super::*;

fn key(world: u64, ship: u64, system: u64, transition: u64, occurrence: u64) -> CompositeKey {
    CompositeKey {
        world,
        ship,
        system,
        transition,
        occurrence,
    }
}

/// THE fixture. These literals are the reproducibility contract: every
/// recovery orbit direction anyone ever recorded is a function of them.
///
/// If you are here because this failed: changing the fold or its constants
/// is allowed, but it re-rolls every decision derived from a recorded seed.
/// Update the literals below only when that is what you mean to do. Pinning
/// concrete outputs (rather than only properties) is what stops the mapping
/// drifting silently under a refactor that still satisfies every
/// property test in this module.
#[test]
fn composite_seed_is_pinned_to_its_recorded_values() {
    assert_eq!(
        composite_seed(&key(0, 0, 0, 0, 0)),
        13_569_046_481_838_298_424
    );
    assert_eq!(
        composite_seed(&key(1, 2, 3, 4, 5)),
        6_360_597_862_457_118_559
    );
    assert_eq!(
        composite_seed(&key(u64::MAX, 0, 1, 0, 1)),
        1_002_478_115_543_807_285
    );
    assert_eq!(key_from_name("helm-steering"), 3_836_562_346_148_525_846);
    assert_eq!(key_from_name(""), 14_087_677_454_934_409_008);
}

#[test]
fn the_same_key_always_derives_the_same_value() {
    let k = key(7, 11, 13, 17, 19);
    assert_eq!(composite_seed(&k), composite_seed(&k));
    assert_eq!(signed_choice(&k), signed_choice(&k));
    assert_eq!(unit_interval(&k), unit_interval(&k));
}

/// Every field is load-bearing: change one and the value moves.
#[test]
fn every_field_participates_in_the_seed() {
    let base = key(1, 1, 1, 1, 1);
    let seed = composite_seed(&base);
    for mutated in [
        key(2, 1, 1, 1, 1),
        key(1, 2, 1, 1, 1),
        key(1, 1, 2, 1, 1),
        key(1, 1, 1, 2, 1),
        key(1, 1, 1, 1, 2),
    ] {
        assert_ne!(
            composite_seed(&mutated),
            seed,
            "{mutated:?} must not collide with the base key"
        );
    }
}

/// The naive `a ^ b ^ c` composite this replaces is commutative, so
/// reordering the fields (or swapping two of them) collides. This fold must
/// not: the *position* of a key is part of its contribution.
#[test]
fn the_fold_is_order_sensitive_where_xor_would_collide() {
    // Full reversal.
    assert_ne!(
        composite_seed(&key(1, 2, 3, 4, 5)),
        composite_seed(&key(5, 4, 3, 2, 1))
    );
    // A single adjacent swap — the case an XOR fold cannot see at all.
    assert_ne!(
        composite_seed(&key(1, 2, 3, 4, 5)),
        composite_seed(&key(2, 1, 3, 4, 5))
    );
    // Two equal keys in different fields: XOR would cancel them to the
    // same value regardless of where they sat.
    assert_ne!(
        composite_seed(&key(9, 9, 0, 0, 0)),
        composite_seed(&key(0, 0, 9, 9, 0))
    );
    // ...and a pair that XOR would cancel to zero entirely.
    assert_ne!(
        composite_seed(&key(9, 9, 0, 0, 0)),
        composite_seed(&key(0, 0, 0, 0, 0))
    );
}

/// Neighbouring occurrences must land far apart, or a counter that ticks
/// 0, 1, 2, 3 would produce a visibly periodic sequence of choices.
#[test]
fn consecutive_occurrences_do_not_alternate_predictably() {
    let choices: Vec<f64> = (0..16)
        .map(|n| signed_choice(&key(42, 7, 3, 5, n)))
        .collect();
    let alternating: Vec<f64> = (0..16)
        .map(|n| if n % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    assert_ne!(
        choices, alternating,
        "the choice sequence must not be a period-2 alternation"
    );
    // Both outcomes must actually occur across a short run.
    assert!(choices.contains(&1.0) && choices.contains(&-1.0));
}

#[test]
fn unit_interval_stays_in_range_and_spreads() {
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    for n in 0..256 {
        let v = unit_interval(&key(3, 4, 5, 6, n));
        assert!((0.0..1.0).contains(&v), "{v} out of range");
        min = min.min(v);
        max = max.max(v);
    }
    assert!(
        min < 0.1 && max > 0.9,
        "expected spread, got [{min}, {max}]"
    );
}

#[test]
fn signed_choice_is_only_ever_plus_or_minus_one() {
    for n in 0..64 {
        let v = signed_choice(&key(n, n * 3, n * 7, n * 11, n * 13));
        assert!(v == 1.0 || v == -1.0, "got {v}");
    }
}

#[test]
fn bounded_index_stays_inside_its_range_and_tolerates_zero() {
    for n in 0..64 {
        assert!(bounded_index(&key(1, 2, 3, 4, n), 5) < 5);
    }
    assert_eq!(bounded_index(&key(1, 2, 3, 4, 5), 0), 0);
}

#[test]
fn distinct_names_derive_distinct_keys() {
    assert_ne!(key_from_name("helm-steering"), key_from_name("helm-thrust"));
    // Names differing in one byte must not land adjacent.
    let a = key_from_name("recover");
    let b = key_from_name("recoves");
    assert!(a.abs_diff(b) > 1_000_000, "{a} and {b} are too close");
}
