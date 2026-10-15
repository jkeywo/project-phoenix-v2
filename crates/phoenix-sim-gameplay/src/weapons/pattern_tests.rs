use super::*;

fn step(barrels: &[u32], offset: f32) -> BarrelPatternStep {
    BarrelPatternStep {
        barrels: barrels.to_vec(),
        offset_secs: offset,
    }
}

#[test]
fn empty_pattern_single_barrel_ok() {
    assert!(validate_barrel_pattern("w", 1, &[]).is_ok());
    assert!(validate_barrel_pattern("w", 0, &[]).is_ok());
}

#[test]
fn empty_pattern_multi_barrel_rejected() {
    assert!(validate_barrel_pattern("w", 2, &[]).is_err());
}

#[test]
fn alternating_pattern_ok() {
    let p = vec![step(&[0], 0.0), step(&[1], 0.2)];
    assert!(validate_barrel_pattern("w", 2, &p).is_ok());
}

#[test]
fn simultaneous_pattern_ok() {
    let p = vec![step(&[0, 1], 0.0)];
    assert!(validate_barrel_pattern("w", 2, &p).is_ok());
}

#[test]
fn empty_barrels_step_rejected() {
    let p = vec![step(&[], 0.0)];
    assert!(validate_barrel_pattern("w", 2, &p).is_err());
}

#[test]
fn barrel_index_out_of_range_rejected() {
    let p = vec![step(&[2], 0.0)];
    let err = validate_barrel_pattern("w", 2, &p).unwrap_err();
    assert!(err.contains("barrel index 2"), "{err}");
}

#[test]
fn negative_offset_rejected() {
    let p = vec![step(&[0], -0.1)];
    assert!(validate_barrel_pattern("w", 1, &p).is_err());
}

#[test]
fn timed_and_ammunition_counted_patterns_share_stable_order_but_keep_different_cadence() {
    let p = vec![step(&[2], 0.5), step(&[1, 0], 0.0), step(&[2], 0.0)];
    assert!(validate_barrel_pattern("w", 3, &p).is_ok());
    assert_eq!(
        firing_schedule(&p, 99, 10.0),
        vec![(vec![1, 0], 0.0), (vec![2], 0.0), (vec![2], 0.5)]
    );
    // This origin list is cycled by loaded-round count; it never schedules an
    // extra shot or replaces the tube's burst interval with authored offsets.
    assert_eq!(barrel_sequence(&p), vec![(1, 1), (0, 1), (2, 2), (2, 3)]);
}

#[test]
fn implicit_barrel_keeps_uniform_volley_and_origin_only_fallback() {
    assert_eq!(
        firing_schedule(&[], 3, 0.25),
        vec![(vec![0], 0.0), (vec![0], 0.25), (vec![0], 0.5)]
    );
    assert!(firing_schedule(&[], 0, 0.25).is_empty());
    assert_eq!(barrel_sequence(&[]), vec![(0, 0)]);
}

#[test]
fn a_step_cannot_claim_the_same_physical_barrel_twice() {
    let duplicate = [BarrelPatternStep {
        barrels: vec![0, 0],
        offset_secs: 0.0,
    }];
    assert!(validate_barrel_pattern("battery", 1, &duplicate).is_err());
    let repeated_over_time = [
        BarrelPatternStep {
            barrels: vec![0],
            offset_secs: 0.0,
        },
        BarrelPatternStep {
            barrels: vec![0],
            offset_secs: 0.2,
        },
    ];
    validate_barrel_pattern("battery", 1, &repeated_over_time).unwrap();
}
