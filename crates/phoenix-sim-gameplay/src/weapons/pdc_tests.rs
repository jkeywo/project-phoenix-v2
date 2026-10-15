use super::*;
#[test]
fn measured_elevated_crossing_lead_meets_the_same_xyz_at_flight_time() {
    let origin = [2.0, 3.0, 4.0];
    let position = [20.0, 13.0, -30.0];
    let velocity = [-2.0, 1.0, 3.0];
    let speed = 40.0;
    let [heading, pitch] = lead_angles(origin, position, velocity, speed).unwrap();
    let direction = [
        crate::simmath::sin(heading) * crate::simmath::cos(pitch),
        crate::simmath::sin(pitch),
        -crate::simmath::cos(heading) * crate::simmath::cos(pitch),
    ];
    let relative = [
        position[0] - origin[0],
        position[1] - origin[1],
        position[2] - origin[2],
    ];
    let closing = [
        direction[0] * speed - velocity[0],
        direction[1] * speed - velocity[1],
        direction[2] * speed - velocity[2],
    ];
    let time = relative
        .into_iter()
        .zip(closing)
        .map(|(r, v)| r * v)
        .sum::<f32>()
        / closing.into_iter().map(|v| v * v).sum::<f32>();
    assert!(time > 0.0 && pitch > 0.0);
    for i in 0..3 {
        assert!(
            (origin[i] + direction[i] * speed * time - position[i] - velocity[i] * time).abs()
                < 0.00001
        );
    }
    assert!(lead_angles([0.0; 3], [0.0, 10.0, 0.0], [0.0, 100.0, 0.0], 10.0).is_none());
    assert!(lead_angles([0.0; 3], [0.0, 1.0, -1.0], [0.0; 3], f32::NAN).is_none());
}
#[test]
fn actual_swept_contact_respects_height_and_tiny_long_link_bodies() {
    assert!(
        swept_sphere_fraction([-10.0, 5.0, 0.0], [10.0, 5.0, 0.0], [0.0, 0.0, 0.0], 1.0).is_none()
    );
    let hit =
        swept_sphere_fraction([-1e8, 5.0, 0.0], [1e8, 5.0, 0.0], [0.0, 5.0, 0.0], 0.1).unwrap();
    assert!((hit - 0.5).abs() < f32::EPSILON);
    assert_eq!(
        swept_sphere_fraction([0.0; 3], [0.0; 3], [0.0; 3], 1.0),
        Some(0.0)
    );
    assert_eq!(
        swept_sphere_fraction([0.0; 3], [0.0; 3], [2.0, 0.0, 0.0], 1.0),
        None
    );
    assert!(swept_sphere_fraction([0.0; 3], [1.0; 3], [0.0; 3], f32::INFINITY).is_none());
}

#[test]
fn moving_relative_sweep_uses_common_time_instead_of_the_missile_endpoint() {
    // Both centers reach the crossing at t=.5; neither endpoint is near the
    // other's segment endpoint. Endpoint-only sphere contact misses this.
    assert!(moving_sphere_fraction(
        [-10.0, 5.0, 0.0],
        [10.0, 5.0, 0.0],
        0.1,
        [0.0, 5.0, -10.0],
        [0.0, 5.0, 10.0],
        0.1
    )
    .is_some());
    // Missile ends on the bolt's old path but arrives after the bolt passed.
    assert!(
        swept_sphere_fraction([-10.0, 5.0, 0.0], [10.0, 5.0, 0.0], [-5.0, 5.0, 0.0], 0.2).is_some()
    );
    assert!(moving_sphere_fraction(
        [-10.0, 5.0, 0.0],
        [10.0, 5.0, 0.0],
        0.1,
        [-5.0, 5.0, 10.0],
        [-5.0, 5.0, 0.0],
        0.1
    )
    .is_none());
    // Each radius is valid f32, although their sum exceeds f32::MAX.
    assert_eq!(
        moving_sphere_fraction(
            [f32::MAX, 0.0, 0.0],
            [f32::MAX, 0.0, 0.0],
            f32::MAX,
            [-f32::MAX, 0.0, 0.0],
            [-f32::MAX, 0.0, 0.0],
            f32::MAX
        ),
        Some(0.0)
    );
}
