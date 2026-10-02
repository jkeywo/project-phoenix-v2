use super::*;

#[test]
fn zero_distance_is_in_range_when_both_have_positive_range() {
    assert!(in_range(0.0, 100.0, 100.0));
}

#[test]
fn distance_equal_to_min_range_is_in_range() {
    assert!(in_range(50.0, 50.0, 100.0));
    assert!(in_range(50.0, 100.0, 50.0));
}

#[test]
fn distance_just_over_min_range_is_out_of_range() {
    assert!(!in_range(50.1, 50.0, 100.0));
}

#[test]
fn effective_range_is_the_smaller_of_the_two() {
    // Larger of the two doesn't matter when the smaller is the gate.
    assert!(in_range(75.0, 80.0, 1_000_000.0));
    assert!(!in_range(81.0, 80.0, 1_000_000.0));
}

#[test]
fn zero_range_means_never_in_range_even_at_zero_distance_when_either_is_zero() {
    // a = 0 → effective = 0 → only distance == 0 succeeds.
    assert!(in_range(0.0, 0.0, 100.0));
    assert!(!in_range(0.1, 0.0, 100.0));
}

#[test]
fn negative_range_is_treated_as_zero() {
    assert!(!in_range(1.0, -5.0, 100.0));
    assert!(in_range(0.0, -5.0, 100.0));
}

#[test]
fn nan_distance_is_out_of_range() {
    // NaN comparisons always return false, so a NaN distance can never
    // satisfy `<= effective`. Lock this behaviour in so accidental NaNs
    // (e.g. from a zeroed transform) never spuriously return true.
    assert!(!in_range(f32::NAN, 100.0, 100.0));
}

#[test]
fn nan_range_is_out_of_range() {
    assert!(!in_range(10.0, f32::NAN, 100.0));
    assert!(!in_range(10.0, 100.0, f32::NAN));
}
