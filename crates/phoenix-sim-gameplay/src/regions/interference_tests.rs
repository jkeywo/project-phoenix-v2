use super::*;

#[test]
fn legacy_environmental_denial_is_exact_and_finite_exposure_scales_once() {
    assert_eq!(endpoint_factor(false, 0.0), 1.0);
    for exposure in [0.0, 0.5, 1.0, f64::from(f32::MAX)] {
        assert_eq!(endpoint_factor(true, exposure), 0.0);
    }
    assert_eq!(endpoint_factor(false, 1.0), 0.5);
    assert_eq!(endpoint_factor(false, 3.0), 0.25);
    assert!(endpoint_factor(false, f64::from(f32::MAX)).is_finite());
    for invalid in [-1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(endpoint_factor(false, invalid), 0.0);
    }
}
