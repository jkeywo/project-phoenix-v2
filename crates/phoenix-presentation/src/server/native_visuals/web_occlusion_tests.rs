use super::*;
#[test]
fn segment_does_not_see_occluders_behind_the_source() {
    let eye = Vec3::new(0.0, 0.0, 5.0);
    assert!(segment_box(eye, -eye, Vec3::ZERO, Vec3::ONE));
    assert!(segment_sphere(eye, -eye));
    let near_source = Vec3::new(0.0, 0.0, 2.0);
    assert!(!segment_box(eye, near_source, Vec3::ZERO, Vec3::ONE));
    assert!(!segment_sphere(eye, near_source));
    let offset = Vec3::X * 2.0;
    assert!(!segment_box(
        eye + offset,
        -eye + offset,
        Vec3::ZERO,
        Vec3::ONE
    ));
    assert!(!segment_sphere(eye + offset, -eye + offset));
}
