use super::*;
use bevy::ecs::system::RunSystemOnce;
#[test]
fn missing_gpu_reply_fails_the_run_instead_of_remaining_healthy_with_no_frame() {
    let (encode, _receive) = mpsc::sync_channel(1);
    let mut world = World::new();
    world.insert_resource(test_frames::PresentationFailure::default());
    world.insert_resource(TestRender {
        image: None,
        pending: Some(Instant::now() - Duration::from_secs(16)),
        next: Instant::now(),
        encode,
    });
    world.run_system_once(capture).unwrap();
    assert!(world
        .resource::<test_frames::PresentationFailure>()
        .error()
        .unwrap()
        .contains("15 seconds"));
}
