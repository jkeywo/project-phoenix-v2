use super::*;
use phoenix_model::{
    identity::SystemId,
    sensing_sharing::{RadarSampleOrigin, RadarSampleProvenance},
    sensing_tracks::TrackStage,
};

fn sample() -> SensorTrackSnapshot {
    SensorTrackSnapshot {
        elevation: None,
        contact_id: "observed-ordnance".into(),
        references: Vec::new(),
        samples: vec![RadarSampleProvenance {
            origin: RadarSampleOrigin {
                source: "public-origin".into(),
                system: SystemId("radar".into()),
                contact: "observed-ordnance".into(),
            },
            sample_id: "sample-2".into(),
            observed_tick: 2,
            received_tick: None,
            observer_position: Some([0.0; 2]),
            velocity_error: 0.5,
            drift_error_per_sec: 0.1,
        }],
        observation: None,
        selectable: false,
        observed_tick: 2,
        position: [100.0, 20.0],
        velocity: Some([-10.0, 0.0]),
        position_error: 1.0,
        stage: TrackStage::Classified,
        identity: None,
        identity_tick: None,
        classification: Some("torpedo".into()),
        classification_tick: Some(2),
        assessment: None,
        assessment_tick: None,
        predicted: false,
    }
}

#[test]
fn conditional_approach_uses_measured_motion_and_widens_its_corridor() {
    let track = sample();
    let result = derive(&[track.clone()], [0.0; 2], [0.0; 2], 60.0);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].seconds, 10.0);
    assert_eq!(result[0].contact_position, [0.0, 20.0]);
    assert!((result[0].position_error - 7.0).abs() < 0.001);
    assert!((result[0].separation[0] - 13.0).abs() < 0.001);
    let shorter = derive(&[track], [0.0; 2], [0.0; 2], 5.0);
    assert_eq!(shorter[0].seconds, 5.0);
    assert_eq!(shorter[0].contact_position, [50.0, 20.0]);
}

#[test]
fn unknown_motion_precision_or_classification_never_becomes_safe_or_precise_prediction() {
    for case in 0..5 {
        let mut track = sample();
        match case {
            0 => track.velocity = None,
            1 => track.samples.clear(),
            2 => track.samples[0].velocity_error = f32::NAN,
            3 => track.classification = None,
            _ => track.velocity = Some([10.0, 0.0]),
        }
        assert!(derive(&[track], [0.0; 2], [0.0; 2], 60.0).is_empty());
    }
}

#[test]
fn own_motion_changes_the_conditional_approach_without_changing_the_observation() {
    let track = sample();
    assert!(derive(&[track.clone()], [0.0; 2], [-10.0, 0.0], 60.0).is_empty());
    let result = derive(&[track.clone()], [0.0; 2], [10.0, 0.0], 60.0);
    assert_eq!(result[0].seconds, 5.0);
    assert_eq!(result[0].own_position, [50.0, 0.0]);
    assert_eq!(track, sample());
}

#[test]
fn large_coordinate_rounding_cannot_shrink_a_small_stated_corridor() {
    let mut track = sample();
    track.position = [100000008.0, 1.0];
    track.position_error = 0.01;
    track.velocity = Some([-0.5, 0.0]);
    track.samples[0].velocity_error = 0.0;
    track.samples[0].drift_error_per_sec = 0.0;
    let result = derive(&[track], [100000000.0, 0.0], [0.0; 2], 3.0);
    assert_eq!(result.len(), 1);
    let row = &result[0];
    assert!(f64::from(row.position_error) >= 1.5 + f64::from(0.01_f32));
    assert!(f64::from(row.separation[0]) <= 6.5_f64.hypot(1.0) - f64::from(0.01_f32));
    assert!(f64::from(row.separation[1]) >= 6.5_f64.hypot(1.0) + f64::from(0.01_f32));
}

#[test]
fn duplicate_feeds_never_choose_one_of_two_conflicting_claims() {
    let first = sample();
    assert_eq!(
        derive(&[first.clone(), first.clone()], [0.0; 2], [0.0; 2], 60.0).len(),
        1
    );
    let mut conflicting = first.clone();
    conflicting.velocity = Some([-20.0, 0.0]);
    assert!(derive(&[first, conflicting], [0.0; 2], [0.0; 2], 60.0).is_empty());
}
