use super::*;

#[test]
fn own_seeker_rejects_far_rear_vertical_and_occluded_subjects() {
    let profile = SeekerProfile {
        range: 40.0,
        fire_arc_deg: 90.0,
        minimum_transmission: 0.25,
    };
    let sample = |position, transmission| {
        seeker_observation(
            &profile,
            [0.0; 3],
            0.0,
            0.0,
            "subject",
            position,
            transmission,
            37,
        )
    };
    let acquired = sample([0.0, 4.0, -20.0], 0.5).unwrap();
    assert_eq!(acquired.sampled_tick, 37);
    assert_eq!(acquired.position, [0.0, 4.0, -20.0]);
    assert!(sample([0.0, 0.0, -41.0], 1.0).is_none());
    assert!(sample([0.0, 0.0, 20.0], 1.0).is_none());
    assert!(sample([0.0, 20.0, 0.0], 1.0).is_none());
    assert!(sample([0.0, 0.0, -20.0], 0.24).is_none());
    assert!(sample([f32::NAN, 0.0, 0.0], 1.0).is_none());
    // A real upward-looking seeker observes the same overhead subject.
    assert!(seeker_observation(
        &profile,
        [0.0; 3],
        0.0,
        std::f32::consts::FRAC_PI_2,
        "subject",
        [0.0, 20.0, 0.0],
        1.0,
        38
    )
    .is_some());
}

#[test]
fn link_geometry_and_original_sample_age_bound_actual_received_updates() {
    let profile = GuidanceReceiverProfile {
        range: 50.0,
        minimum_transmission: 0.2,
        maximum_sample_age_secs: 2.0,
    };
    let original = GuidanceObservation {
        target_uuid: "subject".into(),
        position: [2.0, 3.0, -8.0],
        sampled_tick: 10,
        source: GuidanceSource::Seeker,
    };
    let accepts = |position, transmission, now| {
        receiver_accepts(
            &profile,
            [0.0; 3],
            position,
            transmission,
            &original,
            now,
            0.1,
        )
    };
    assert!(accepts([0.0, 30.0, -30.0], 0.2, 30));
    assert!(!accepts([0.0, 30.0, -30.0], 0.2, 31));
    assert!(!accepts([0.0, 51.0, 0.0], 1.0, 11));
    assert!(!accepts([0.0, 0.0, -10.0], 0.19, 11));
    assert!(!accepts([0.0; 3], 1.0, 9));
    assert_eq!(
        original.sampled_tick, 10,
        "receiving never refreshes original evidence"
    );
}
