use super::*;
#[test]
fn streak_tracks_perspective_motion_including_reverse_and_strafe() {
    for position in [Vec3::new(3.0, 2.0, -10.0), Vec3::new(-2.0, -1.0, -15.0)] {
        for velocity in [
            Vec3::Z,
            Vec3::NEG_Z,
            Vec3::X,
            Vec3::Y,
            Vec3::new(2.0, 1.0, 3.0),
        ] {
            let project = |p: Vec3| p.truncate() / -p.z;
            let observed = (project(position + velocity * 0.001) - project(position)).normalize();
            assert!(
                projected_motion(position, velocity)
                    .normalize()
                    .dot(observed)
                    > 0.999
            );
        }
    }
}
