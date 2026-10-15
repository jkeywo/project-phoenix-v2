use super::*;
#[test]
fn independent_instruments_cannot_alias_contact_ids_or_refresh_each_others_work() {
    let mut first = profile();
    first.system = SystemId("a:b".into());
    let mut second = first.clone();
    second.system = SystemId("a".into());
    let mut a = BroadTrackState::new(&first);
    let mut b = BroadTrackState::new(&second);
    let evidence = [sample("same-private-source", 10.0)];
    a.advance(&first, 0, 0.5, 1.0, &evidence).unwrap();
    b.advance(&second, 0, 0.5, 0.0, &evidence).unwrap();
    assert!(b.tracks.is_empty());
    b.advance(&second, 1, 0.5, 1.0, &evidence).unwrap();
    let a_id = a.contact_for_source("same-private-source").unwrap();
    let b_id = b.contact_for_source("same-private-source").unwrap();
    assert_eq!(a_id, "3:a:b:contact-1");
    assert_eq!(b_id, "1:a:contact-1");
    assert!(a.resolve_current(b_id).is_none() && b.resolve_current(a_id).is_none());
    let mut wrong = a.clone();
    let mut record = wrong.tracks.remove(a_id).unwrap();
    record.contact_id = b_id.into();
    wrong.tracks.insert(b_id.into(), record);
    for value in wrong.current.values_mut() {
        *value = b_id.into();
    }
    assert!(wrong.validate(&first, 1, 0.5).is_err());
    let mut wrong = a.clone();
    let mut record = wrong.tracks.remove(a_id).unwrap();
    record.contact_id = "3:a:b:contact-01".into();
    wrong.tracks.insert(record.contact_id.clone(), record);
    for value in wrong.current.values_mut() {
        *value = "3:a:b:contact-01".into();
    }
    assert!(wrong.validate(&first, 1, 0.5).is_err());
}
fn profile() -> BroadConfig {
    BroadConfig {
        system: SystemId("survey-array".into()),
        sample_interval_secs: 0.5,
        identity_secs: 1.0,
        classification_secs: 2.0,
        assessment_secs: 3.0,
        position_error: 1.0,
        drift_error_per_sec: 0.5,
        track_retention_secs: 5.0,
    }
}
fn sample(source: &str, x: f32) -> BroadMeasurement {
    BroadMeasurement {
        source: source.into(),
        position: [x, 0.0],
        support: 1.0,
        identity: Some(TrackIdentity {
            name: "source-name".into(),
            faction: Some("source-faction".into()),
        }),
        classification: Some("ship".into()),
        assessment: Some(TrackAssessment {
            alert: Some(true),
            weapons_restrained: Some(false),
            disabled: Some(false),
        }),
    }
}
#[test]
fn actual_paid_stages_gate_each_finding_without_disclosing_source_identity() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    state
        .advance(&p, 0, 0.5, 1.0, &[sample("SECRET-SOURCE-UUID", 10.0)])
        .unwrap();
    let first = state.projection(&p, 0, 0.5).unwrap();
    assert_eq!(first[0].stage, TrackStage::Return);
    assert!(
        first[0].identity.is_none()
            && first[0].classification.is_none()
            && first[0].assessment.is_none()
    );
    assert!(!serde_json::to_string(&first).unwrap().contains("SECRET"));
    state
        .advance(&p, 1, 0.5, 1.0, &[sample("SECRET-SOURCE-UUID", 10.0)])
        .unwrap();
    let identified = state.projection(&p, 1, 0.5).unwrap();
    assert_eq!(identified[0].stage, TrackStage::Identified);
    assert_eq!(
        identified[0].identity.as_ref().unwrap().faction.as_deref(),
        Some("source-faction")
    );
    assert!(identified[0].classification.is_none());
    for tick in 2..6 {
        state
            .advance(&p, tick, 0.5, 1.0, &[sample("SECRET-SOURCE-UUID", 10.0)])
            .unwrap();
    }
    let last = state.projection(&p, 5, 0.5).unwrap();
    assert_eq!(last[0].stage, TrackStage::Assessed);
    assert!(last[0].assessment.is_some());
}
#[test]
fn partial_paid_work_is_independent_of_contact_count_and_duplicate_ticks() {
    let p = profile();
    let mut one = BroadTrackState::new(&p);
    let mut many = one.clone();
    for tick in 0..4 {
        one.advance(&p, tick, 0.5, 0.5, &[sample("a", 10.0)])
            .unwrap();
        many.advance(&p, tick, 0.5, 0.5, &[sample("a", 10.0), sample("b", 100.0)])
            .unwrap();
    }
    assert_eq!(
        one.tracks["12:survey-array:contact-1"].supported_secs,
        many.tracks["12:survey-array:contact-1"].supported_secs
    );
    assert!(many
        .tracks
        .values()
        .all(|track| track.supported_secs == 1.0));
    let before = many.clone();
    many.advance(&p, 3, 0.5, 1.0, &[sample("a", 900.0)])
        .unwrap();
    assert_eq!(many, before);
}
#[test]
fn lost_prediction_uses_only_measured_motion_and_grows_uncertainty_without_live_input() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    state
        .advance(&p, 0, 0.5, 1.0, &[sample("a", 10.0)])
        .unwrap();
    state
        .advance(&p, 1, 0.5, 1.0, &[sample("a", 12.0)])
        .unwrap();
    let measured = state.tracks["12:survey-array:contact-1"].clone();
    state
        .advance(&p, 2, 0.5, 0.0, &[sample("a", 1000.0)])
        .unwrap();
    assert!(state.resolve_current("12:survey-array:contact-1").is_none());
    assert_eq!(state.tracks["12:survey-array:contact-1"], measured);
    let later = state.projection(&p, 3, 0.5).unwrap();
    let v = measured.velocity.unwrap();
    assert_eq!(
        later[0].position[0],
        bounded_float(f64::from(measured.position[0]) + f64::from(v[0]))
    );
    assert!(later[0].predicted && later[0].position_error > measured.position_error);
    assert_eq!(later[0].identity, measured.identity);
}
#[test]
fn reacquisition_uses_unique_geometry_not_hidden_source_equality() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    state
        .advance(&p, 0, 0.5, 1.0, &[sample("a", 10.0)])
        .unwrap();
    state.advance(&p, 1, 0.5, 1.0, &[]).unwrap();
    state
        .advance(&p, 2, 0.5, 1.0, &[sample("different-source", 10.0)])
        .unwrap();
    assert_eq!(
        state.resolve_current("12:survey-array:contact-1"),
        Some("different-source")
    );
    state.advance(&p, 3, 0.5, 1.0, &[]).unwrap();
    state
        .advance(&p, 4, 0.5, 1.0, &[sample("different-source", 1000.0)])
        .unwrap();
    assert!(state.resolve_current("12:survey-array:contact-1").is_none());
    assert_eq!(
        state.resolve_current("12:survey-array:contact-2"),
        Some("different-source")
    );
}
#[test]
fn confused_crossing_returns_do_not_preserve_private_ground_truth_associations() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    state
        .advance(&p, 0, 0.5, 1.0, &[sample("a", 0.0), sample("b", 10.0)])
        .unwrap();
    state
        .advance(&p, 1, 0.5, 1.0, &[sample("a", 5.0), sample("b", 5.0)])
        .unwrap();
    assert!(
        state.resolve_current("12:survey-array:contact-1").is_none()
            && state.resolve_current("12:survey-array:contact-2").is_none()
    );
    let published = state.projection(&p, 1, 0.5).unwrap();
    assert_eq!(published.iter().filter(|track| !track.predicted).count(), 2);
    assert!(published
        .iter()
        .filter(|track| !track.predicted)
        .all(|track| track.identity.is_none()));
}
#[test]
fn zero_supply_retains_findings_then_bounded_retention_removes_only_stale_tracks() {
    let mut p = profile();
    p.track_retention_secs = 1.0;
    let mut state = BroadTrackState::new(&p);
    state
        .advance(&p, 0, 0.5, 0.0, &[sample("a", 10.0)])
        .unwrap();
    assert!(state.tracks.is_empty());
    state
        .advance(&p, 1, 0.5, 1.0, &[sample("a", 10.0)])
        .unwrap();
    for tick in 2..=3 {
        state.advance(&p, tick, 0.5, 1.0, &[]).unwrap();
        assert!(!state.tracks.is_empty());
    }
    state.advance(&p, 4, 0.5, 1.0, &[]).unwrap();
    assert!(state.tracks.is_empty() && state.current.is_empty());
}
#[test]
fn measurement_error_is_bounded_and_is_not_automatically_truth_centred() {
    let p = profile();
    let actual = [10.0, 3.0];
    let measured = measured_position(actual, p.position_error);
    assert_ne!(actual, measured);
    assert!(distance(actual, measured) <= f64::from(p.position_error));
    assert_eq!(measured_position(actual, 0.0), actual);
}
#[test]
fn continuation_matches_quality_motion_and_identity_after_json_and_refuses_tampering() {
    let p = profile();
    let mut live = BroadTrackState::new(&p);
    for tick in 0..3 {
        live.advance(&p, tick, 0.5, 0.5, &[sample("a", tick as f32)])
            .unwrap();
    }
    let mut resumed: BroadTrackState =
        serde_json::from_str(&serde_json::to_string(&live).unwrap()).unwrap();
    for tick in 3..10 {
        let samples = if tick < 6 {
            vec![sample("a", tick as f32)]
        } else {
            vec![]
        };
        live.advance(&p, tick, 0.5, 0.5, &samples).unwrap();
        resumed.advance(&p, tick, 0.5, 0.5, &samples).unwrap();
        assert_eq!(live, resumed);
        assert_eq!(
            live.projection(&p, tick, 0.5),
            resumed.projection(&p, tick, 0.5)
        );
    }
    let mut bad = live.clone();
    bad.last_sample_tick = Some(100);
    assert!(bad.validate(&p, 10, 0.5).is_err());
    let mut bad = live.clone();
    bad.current.insert("a".into(), "missing-contact".into());
    assert!(bad.validate(&p, 10, 0.5).is_err());
    let before = live.clone();
    assert!(live
        .advance(&p, 10, 0.5, 1.0, &[sample("a", 0.0), sample("a", 1.0)])
        .is_err());
    assert_eq!(live, before);
    assert!(live.advance(&p, 10, 0.5, f32::NAN, &[]).is_err());
    assert_eq!(live, before);
}

#[test]
fn exhausted_frontier_is_rejected_and_near_exhaustion_does_not_panic_the_producer() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    state.last_tick = Some(1);
    state.last_sample_tick = Some(1);
    state.next_contact = u64::MAX;
    assert!(state.validate(&p, 1, 0.5).is_err());
    state.next_contact = u64::MAX - 1;
    state.validate(&p, 1, 0.5).unwrap();
    state
        .advance(&p, 2, 0.5, 1.0, &[sample("new-source", 10.0)])
        .unwrap();
    assert!(state.tracks.is_empty() && state.current.is_empty());
    state.validate(&p, 2, 0.5).unwrap();
}

#[test]
fn reacquired_geometry_retains_original_fact_age_until_new_supported_refresh() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    for tick in 0..6 {
        state
            .advance(&p, tick, 0.5, 1.0, &[sample("a", 10.0)])
            .unwrap();
    }
    let known = state.tracks["12:survey-array:contact-1"].clone();
    state.advance(&p, 6, 0.5, 1.0, &[]).unwrap();
    let mut changed = sample("different-source", 10.0);
    changed.identity.as_mut().unwrap().name = "not-yet-acquired".into();
    state.advance(&p, 7, 0.5, 1.0, &[changed.clone()]).unwrap();
    assert_eq!(
        state.tracks["12:survey-array:contact-1"].identity,
        known.identity
    );
    assert_eq!(
        state.tracks["12:survey-array:contact-1"].identity_tick,
        known.identity_tick
    );
    state.advance(&p, 8, 0.5, 1.0, &[changed]).unwrap();
    assert_eq!(
        state.tracks["12:survey-array:contact-1"]
            .identity
            .as_ref()
            .unwrap()
            .name,
        "not-yet-acquired"
    );
    assert_eq!(
        state.tracks["12:survey-array:contact-1"].identity_tick,
        Some(8)
    );
    assert_eq!(
        state.tracks["12:survey-array:contact-1"].assessment_tick,
        known.assessment_tick
    );
}

#[test]
fn swapping_hidden_source_identifiers_does_not_change_anonymous_public_returns() {
    let p = profile();
    let mut first = BroadTrackState::new(&p);
    let mut second = first.clone();
    first
        .advance(
            &p,
            0,
            0.5,
            1.0,
            &[sample("private-a", 10.0), sample("private-b", 100.0)],
        )
        .unwrap();
    second
        .advance(
            &p,
            0,
            0.5,
            1.0,
            &[sample("private-b", 10.0), sample("private-a", 100.0)],
        )
        .unwrap();
    assert_eq!(
        first.projection(&p, 0, 0.5).unwrap(),
        second.projection(&p, 0, 0.5).unwrap()
    );
    assert_ne!(
        first.current, second.current,
        "only private mechanical routing differs"
    );
}
