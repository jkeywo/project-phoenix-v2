use super::*;
fn state(owner: &str) -> NavigationSharingState {
    NavigationSharingState::new(&RadarSharingConfig {
        system: SystemId(owner.into()),
        share_own_faction: false,
        receive: true,
    })
}
fn point(x: f32) -> NavigationSamplePoint {
    NavigationSamplePoint {
        kind: NavigationSharedKind::Waypoint,
        position: [x, 7.0],
        observed_tick: None,
        observer_position: None,
    }
}
#[test]
fn repeated_receipt_and_relay_retain_original_point_pose_and_clocks() {
    let mut source = state("navigation-a");
    source.sample("a", 3, [10.0, 20.0], &[point(5.0)]).unwrap();
    let original = source.exports();
    source.sample("a", 9, [99.0, 99.0], &[point(5.0)]).unwrap();
    assert_eq!(source.exports(), original);
    let mut receiver = state("navigation-b");
    receiver.accept("b", 10, &original).unwrap();
    let first = receiver.projection();
    receiver.accept("b", 12, &original).unwrap();
    assert_eq!(receiver.projection(), first);
    assert_eq!(first[0].provenance.received_tick, Some(10));
    let mut third = state("navigation-c");
    third.accept("c", 13, &receiver.exports()).unwrap();
    third.accept("c", 13, &original).unwrap();
    assert_eq!(third.received.len(), 1);
    assert_eq!(
        third.projection()[0].provenance.origin,
        original[0].provenance.origin
    );
    assert_eq!(third.projection()[0].provenance.observed_tick, 3);
    assert_eq!(
        third.projection()[0].provenance.observer_position,
        Some([10.0, 20.0])
    );
    assert!(!serde_json::to_string(&third.projection())
        .unwrap()
        .contains("private-actor"));
}
#[test]
fn later_course_can_choose_an_older_held_measurement_without_renewing_its_age() {
    let mut source = state("navigation-a");
    source.sample("a", 10, [0.0, 0.0], &[point(4.0)]).unwrap();
    let old = source.exports();
    let mut receiver = state("navigation-b");
    receiver.accept("b", 10, &old).unwrap();
    let mut held = point(8.0);
    held.observed_tick = Some(2);
    held.observer_position = Some([-5.0, 0.0]);
    source.sample("a", 12, [0.0, 0.0], &[held]).unwrap();
    receiver.accept("b", 13, &source.exports()).unwrap();
    receiver.accept("b", 14, &old).unwrap();
    let sample = &receiver.projection()[0];
    assert_eq!(sample.position, [8.0, 7.0]);
    assert_eq!(sample.provenance.observed_tick, 2);
    assert_eq!(sample.published_tick, 12);
}
#[test]
fn intent_is_independent_and_invalid_future_receipt_is_transactional_with_json_continuation() {
    let mut source = state("navigation-a");
    source.sample("a", 1, [0.0, 0.0], &[point(4.0)]).unwrap();
    let mut receiver = state("navigation-b");
    receiver.receive = false;
    receiver.accept("b", 2, &source.exports()).unwrap();
    assert!(receiver.received.is_empty());
    receiver.receive = true;
    receiver.accept("b", 2, &source.exports()).unwrap();
    let before = receiver.clone();
    let mut bad = source.exports();
    bad[0].published_tick = 9;
    assert!(receiver.accept("b", 3, &bad).is_err());
    assert_eq!(receiver, before);
    let restored: NavigationSharingState =
        serde_json::from_str(&serde_json::to_string(&receiver).unwrap()).unwrap();
    assert_eq!(restored, receiver);
    restored.validate(2).unwrap();
}
#[test]
fn self_echo_and_duplicate_originals_cannot_create_more_history() {
    let mut source = state("navigation-a");
    source.sample("a", 1, [0.0, 0.0], &[point(4.0)]).unwrap();
    source.accept("a", 2, &source.exports()).unwrap();
    assert!(source.received.is_empty());
    let mut receiver = state("navigation-b");
    let packet = source.exports();
    receiver
        .accept("b", 2, &[packet[0].clone(), packet[0].clone()])
        .unwrap();
    assert_eq!(receiver.received.len(), 1);
    receiver
        .validate_owner("b", &SystemId("navigation-b".into()), 2)
        .unwrap();
    assert!(receiver
        .validate_owner("a", &SystemId("navigation-b".into()), 2)
        .is_err());
}
