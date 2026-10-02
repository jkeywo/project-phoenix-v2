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
