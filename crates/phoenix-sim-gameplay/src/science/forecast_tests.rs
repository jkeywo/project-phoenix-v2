use super::*;

fn reading(tick: u64, value: f32, step: f32) -> ScanReadingSnapshot {
    ScanReadingSnapshot {
        subject_uuid: "observed-contact".into(),
        taken_at_tick: tick,
        condition_fraction: value,
        condition_step: step,
        ..Default::default()
    }
}
fn policy() -> ScanForecastPolicy {
    ScanForecastPolicy { horizon_secs: 10.0 }
}

#[test]
fn quantised_readings_propagate_uncertainty_and_freeze_original_times() {
    let old = reading(10, 0.8, 0.1);
    let current = reading(20, 0.6, 0.1);
    let answer = derive(&[old], &current, &policy(), 1.0).unwrap();
    assert_eq!(answer.sample_ticks, [10, 20]);
    assert_eq!(answer.sample_interval_secs, 10.0);
    assert!((answer.ranges[0].lower - 0.25).abs() < 0.00001);
    assert!((answer.ranges[0].upper - 0.55).abs() < 0.00001);
    assert_eq!(answer.assumptions.len(), 2);
}

#[test]
fn copies_and_other_contacts_cannot_supply_a_second_observation() {
    let current = reading(20, 0.6, 0.1);
    let mut other = reading(10, 0.9, 0.1);
    other.subject_uuid = "other-contact".into();
    assert!(derive(
        &[current.clone(), current.clone(), other],
        &current,
        &policy(),
        1.0
    )
    .is_none());
    let mut unknown = current.clone();
    unknown.subject_uuid.clear();
    assert!(derive(&[reading(10, 0.9, 0.1)], &unknown, &policy(), 1.0).is_none());
}

#[test]
fn duplicate_delivery_is_idempotent_and_conflicting_same_time_refuses() {
    let old = reading(10, 0.8, 0.1);
    let current = reading(20, 0.6, 0.1);
    assert_eq!(
        derive(std::slice::from_ref(&old), &current, &policy(), 1.0),
        derive(&[old.clone(), old.clone()], &current, &policy(), 1.0)
    );
    assert!(derive(&[old, reading(10, 0.3, 0.1)], &current, &policy(), 1.0).is_none());
}

#[test]
fn only_common_unambiguous_observed_resource_quantities_are_projected() {
    let mut old = reading(10, 0.8, 0.1);
    old.capacities = vec![
        ("resource.fuel".into(), 100),
        ("resource.ambiguous".into(), 20),
    ];
    let mut current = reading(20, 0.7, 0.1);
    current.capacities = vec![
        ("resource.fuel".into(), 80),
        ("resource.unknown".into(), 5),
        ("resource.ambiguous".into(), 15),
        ("resource.ambiguous".into(), 10),
    ];
    let answer = derive(&[old], &current, &policy(), 1.0).unwrap();
    assert_eq!(answer.ranges.len(), 2);
    assert_eq!(
        answer.ranges[1],
        ForecastRange {
            quantity: ForecastQuantity::Capacity {
                label: "resource.fuel".into()
            },
            lower: 60.0,
            upper: 60.0
        }
    );
}

#[test]
fn history_order_does_not_change_latest_prior_sample_or_reuse_future_data() {
    let current = reading(30, 0.5, 0.1);
    let a = reading(10, 0.8, 0.1);
    let b = reading(20, 0.6, 0.1);
    let future = reading(40, 0.0, 0.01);
    assert_eq!(
        derive(&[a.clone(), b.clone()], &current, &policy(), 1.0),
        derive(&[future, b, a], &current, &policy(), 1.0)
    );
}

#[test]
fn invalid_horizon_clock_or_reading_never_creates_a_projection() {
    let old = reading(10, 0.8, 0.1);
    let current = reading(20, 0.6, 0.1);
    for horizon in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(derive(
            std::slice::from_ref(&old),
            &current,
            &ScanForecastPolicy {
                horizon_secs: horizon
            },
            1.0
        )
        .is_none());
    }
    for tick_secs in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(derive(std::slice::from_ref(&old), &current, &policy(), tick_secs).is_none());
    }
    assert!(derive(&[old], &reading(20, f32::NAN, 0.1), &policy(), 1.0).is_none());
}

#[test]
fn adjacent_large_integer_observations_never_become_false_exact_constant_flow() {
    let mut old = reading(10, 0.8, 0.1);
    old.capacities = vec![("resource.huge".into(), 1_i64 << 53)];
    let mut current = reading(20, 0.7, 0.1);
    current.capacities = vec![("resource.huge".into(), (1_i64 << 53) + 1)];
    let answer = derive(&[old], &current, &policy(), 1.0).unwrap();
    assert_eq!(
        answer.ranges.len(),
        1,
        "condition remains supported; unrepresentable capacity is withheld"
    );
}
