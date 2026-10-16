use super::*;

#[test]
fn observed_shield_ray_accepts_its_position_cone_but_not_another_approach() {
    let actual = [0.0, 0.0];
    let observer = [0.0, -45.0];
    let sector = shield_observation_sector(observer, actual);
    let measured = measured_position(actual, 2.0);
    assert_ne!(shield_observation_sector(observer, measured), sector);
    assert!(shield_ray_compatible(observer, measured, 2.0, sector));
    assert!(!shield_ray_compatible(observer, measured, 0.0, sector));
    assert!(!shield_ray_compatible([45.0, 0.0], measured, 2.0, sector));
    assert!(!shield_ray_compatible(measured, measured, 2.0, sector));
    for error in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(!shield_ray_compatible(observer, measured, error, sector));
    }
    assert!(!shield_ray_compatible(observer, measured, 2.0, 360));
    assert!(shield_ray_compatible([0.0, -45.0], [0.01, 0.0], 0.1, 359));
    assert!(shield_ray_compatible(
        [f32::MAX, 0.0],
        [-f32::MAX, 0.0],
        2.0,
        90
    ));
}

#[test]
fn distant_elevation_rounding_stays_inside_the_published_error() {
    let mut p = profile();
    p.position_error = 0.8;
    let mut state = BroadTrackState::new(&p);
    let mut value = sample("actor", 20.0);
    let actual_height = 10_000_005.0_f32;
    assert_eq!(
        measured_position([actual_height, 0.0], p.position_error)[0],
        10_000_006.0
    );
    value.elevation = Some(ObservedElevation {
        height: actual_height,
        error: p.position_error,
        velocity: None,
    });
    state.advance(&p, 0, 0.5, 1.0, &[value]).unwrap();
    let observed = state.projection(&p, 0, 0.5).unwrap()[0].elevation.unwrap();
    assert_eq!(observed.height, actual_height);
    assert_eq!(observed.error, p.position_error);
    assert!(
        (f64::from(observed.height) - f64::from(actual_height)).abs() <= f64::from(observed.error)
    );
    assert_eq!(observed.velocity, None);
    let restored: BroadTrackState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(restored, state);
    restored.validate(&p, 0, 0.5).unwrap();
}

#[test]
fn elevation_is_one_dated_paid_measurement_with_sample_derived_vertical_motion() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    let elevated = |height| {
        let mut value = sample("actor", 20.0);
        value.elevation = Some(ObservedElevation {
            height,
            error: p.position_error,
            velocity: None,
        });
        value
    };
    state.advance(&p, 0, 0.5, 1.0, &[elevated(3.2)]).unwrap();
    let id = state.current["actor"].clone();
    let first = state.tracks[&id].elevation.unwrap();
    assert_ne!(first.height, 3.2);
    assert!((first.height - 3.2).abs() <= first.error);
    assert_eq!(first.error, 1.0);
    assert_eq!(first.velocity, None);
    let mut impossible_first = state.clone();
    impossible_first
        .tracks
        .get_mut(&id)
        .unwrap()
        .elevation
        .as_mut()
        .unwrap()
        .velocity = Some(1.0);
    assert!(impossible_first.validate(&p, 0, 0.5).is_err());
    let before = state.clone();
    state.advance(&p, 0, 0.5, 1.0, &[elevated(900.0)]).unwrap();
    assert_eq!(state, before, "same tick cannot add a sample or motion");
    state.advance(&p, 1, 0.5, 0.0, &[elevated(900.0)]).unwrap();
    assert_eq!(state.tracks[&id], before.tracks[&id]);
    let retained = state.projection(&p, 1, 0.5).unwrap();
    assert_eq!(retained[0].elevation, before.tracks[&id].elevation);
    assert_eq!(retained[0].observed_tick, 0);
    assert!(retained[0].predicted);
    let mut restored: BroadTrackState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    for value in [&mut state, &mut restored] {
        value.advance(&p, 2, 0.5, 1.0, &[elevated(5.2)]).unwrap();
    }
    assert_eq!(state, restored);
    let next = state.tracks[&id].elevation.unwrap();
    assert_eq!(next.velocity, Some(next.height - first.height));
    assert_eq!(state.tracks[&id].observed_tick, 2);
    let held = state.tracks[&id].elevation;
    state.advance(&p, 3, 0.5, 0.0, &[elevated(-900.0)]).unwrap();
    assert_eq!(state.projection(&p, 4, 0.5).unwrap()[0].elevation, held);
}

#[test]
fn missing_or_reported_height_cannot_become_zero_or_a_physical_velocity_baseline() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    let mut physical = sample("private-report-actor", 123.0);
    physical.position = [123.0, 456.0];
    physical.elevation = Some(ObservedElevation {
        height: 3.0,
        error: 1.0,
        velocity: None,
    });
    state.advance(&p, 0, 0.5, 1.0, &[physical.clone()]).unwrap();
    state.advance(&p, 4, 0.5, 1.0, &[reported(&p, 2)]).unwrap();
    assert!(state.tracks.values().all(|track| track.elevation.is_none()));
    assert!(state
        .projection(&p, 4, 0.5)
        .unwrap()
        .iter()
        .all(|track| track.elevation.is_none()));
    physical.elevation.as_mut().unwrap().height = 100.0;
    state.advance(&p, 5, 0.5, 1.0, &[physical.clone()]).unwrap();
    let id = state.current[&physical.source].clone();
    assert_eq!(state.tracks[&id].elevation.unwrap().velocity, None);
    physical.elevation = None;
    state.advance(&p, 6, 0.5, 1.0, &[physical]).unwrap();
    assert_eq!(state.tracks[&id].elevation, None);
}

#[test]
fn malformed_elevation_and_report_height_are_refused_before_state_changes() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    for elevation in [
        ObservedElevation {
            height: f32::NAN,
            error: 1.0,
            velocity: None,
        },
        ObservedElevation {
            height: 1.0,
            error: -1.0,
            velocity: None,
        },
        ObservedElevation {
            height: 1.0,
            error: 1.0,
            velocity: Some(1000.0),
        },
        ObservedElevation {
            height: 1.0,
            error: 2.0,
            velocity: None,
        },
    ] {
        let before = state.clone();
        let mut value = sample("actor", 20.0);
        value.elevation = Some(elevation);
        assert!(state.advance(&p, 0, 0.5, 1.0, &[value]).is_err());
        assert_eq!(state, before);
    }
    let mut value = reported(&p, 0);
    value.elevation = Some(ObservedElevation {
        height: 1.0,
        error: 1.0,
        velocity: None,
    });
    assert!(state.advance(&p, 2, 0.5, 1.0, &[value]).is_err());
    state.advance(&p, 2, 0.5, 1.0, &[reported(&p, 0)]).unwrap();
    let mut malformed = state.clone();
    malformed.tracks.values_mut().next().unwrap().elevation = Some(ObservedElevation {
        height: 1.0,
        error: 1.0,
        velocity: None,
    });
    assert!(malformed.validate(&p, 2, 0.5).is_err());
    let mut value = sample("actor", 900.0);
    value.elevation = Some(ObservedElevation {
        height: 1.0,
        error: 1.0,
        velocity: None,
    });
    state.advance(&p, 3, 0.5, 1.0, &[value]).unwrap();
    let id = state.current["actor"].clone();
    state
        .tracks
        .get_mut(&id)
        .unwrap()
        .elevation
        .as_mut()
        .unwrap()
        .velocity = Some(f32::INFINITY);
    assert!(state.validate(&p, 3, 0.5).is_err());
}
#[test]
fn synthetic_evidence_shares_tracks_but_never_becomes_an_actor_route_or_public_decoy_flag() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    let mut ghost = reported(&p, 2);
    ghost.source = synthetic_source(&"é".repeat(64)).unwrap();
    ghost.observation.as_mut().unwrap().subject = ObservationSubject::Unknown;
    ghost.selectable = false;
    state.advance(&p, 4, 0.5, 1.0, &[ghost.clone()]).unwrap();
    let id = state.current[&ghost.source].clone();
    assert!(
        state.resolve_current(&id).is_none() && state.contact_for_source(&ghost.source).is_none()
    );
    let public = state.projection(&p, 4, 0.5).unwrap();
    assert_eq!(
        public[0].observation.as_ref().unwrap().subject,
        ObservationSubject::Entity { uuid: id.clone() }
    );
    assert!(!public[0].selectable);
    let wire = serde_json::to_string(&public).unwrap();
    assert!(!wire.contains("unknown") && !wire.contains("ghost/") && !wire.contains("synthetic"));
    let before = state.clone();
    let mut restored: BroadTrackState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    state.advance(&p, 5, 0.5, 1.0, &[ghost.clone()]).unwrap();
    restored.advance(&p, 5, 0.5, 1.0, &[ghost]).unwrap();
    assert_eq!(state, restored);
    assert_eq!(state.tracks[&id], before.tracks[&id]);
}
#[test]
fn synthetic_sample_cannot_claim_actor_permission_and_private_route_shape_is_validated() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    let before = state.clone();
    let mut ghost = reported(&p, 2);
    ghost.source = synthetic_source("example").unwrap();
    ghost.observation.as_mut().unwrap().subject = ObservationSubject::Unknown;
    assert!(state.advance(&p, 4, 0.5, 1.0, &[ghost.clone()]).is_err());
    assert_eq!(state, before);
    ghost.selectable = false;
    state.advance(&p, 4, 0.5, 1.0, &[ghost.clone()]).unwrap();
    let id = state.current[&ghost.source].clone();
    state
        .tracks
        .get_mut(&id)
        .unwrap()
        .observation
        .as_mut()
        .unwrap()
        .subject = ObservationSubject::Entity { uuid: id.clone() };
    assert!(state.validate(&p, 4, 0.5).is_err());
    assert!(synthetic_source(&"x".repeat(129)).is_err());
    assert!(synthetic_source("line\nbreak").is_err());
}
fn reported(profile: &BroadConfig, observed: u64) -> BroadMeasurement {
    use phoenix_model::observation::*;
    BroadMeasurement {
        elevation: None,
        source: "private-report-actor".into(),
        observation: Some(
            ObservationMetadata::at(
                observed,
                ObservationSubject::Entity {
                    uuid: "private-report-actor".into(),
                },
                ObservationProvenance {
                    kind: ObservationSource::SensorsReport,
                    system: Some(profile.system.clone()),
                    label: "console.sensors.report_source".into(),
                },
                ObservationUncertainty::Quantized {
                    field: "position".into(),
                    step: 10.0,
                },
            )
            .received(observed + 2),
        ),
        selectable: true,
        position: [123.0, 456.0],
        support: 1.0,
        identity: Some(TrackIdentity {
            name: "Reported label".into(),
            faction: None,
            references: Vec::new(),
        }),
        classification: None,
        assessment: None,
    }
}
#[test]
fn held_reports_keep_original_sample_and_permissions_without_new_work_or_private_identity() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    let sample = reported(&p, 2);
    state.advance(&p, 4, 0.5, 1.0, &[sample.clone()]).unwrap();
    let id = state.contact_for_source(&sample.source).unwrap().to_owned();
    let before = state.tracks[&id].clone();
    assert_eq!(before.observed_tick, 2);
    assert_eq!(before.position, sample.position);
    assert_eq!(before.supported_secs, 0.0);
    assert!(before.velocity.is_none());
    for tick in 5..10 {
        state
            .advance(&p, tick, 0.5, 1.0, &[sample.clone()])
            .unwrap();
    }
    assert_eq!(state.tracks[&id], before);
    let picture = state.projection(&p, 9, 0.5).unwrap();
    assert_eq!(picture[0].stage, TrackStage::Identified);
    assert_eq!(
        picture[0].observation.as_ref().unwrap().subject,
        ObservationSubject::Entity { uuid: id.clone() }
    );
    assert_eq!(
        picture[0].observation.as_ref().unwrap().received_tick,
        Some(4)
    );
    let public = serde_json::to_string(&picture).unwrap();
    assert!(!public.contains(&sample.source));
    assert!(picture[0].classification.is_none() && picture[0].assessment.is_none());
    let mut resumed: BroadTrackState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    state.advance(&p, 10, 0.5, 1.0, &[]).unwrap();
    resumed.advance(&p, 10, 0.5, 1.0, &[]).unwrap();
    assert_eq!(state, resumed);
    assert!(state.resolve_current(&id).is_none());
}
#[test]
fn report_future_metadata_and_hidden_facts_refuse_transactionally() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    let before = state.clone();
    for mode in 0..4 {
        let mut sample = reported(&p, 2);
        match mode {
            0 => sample.observation.as_mut().unwrap().observed_tick = Some(6),
            1 => sample.observation.as_mut().unwrap().received_tick = Some(7),
            2 => sample.identity.as_mut().unwrap().faction = Some("private-faction".into()),
            _ => {
                sample.assessment = Some(TrackAssessment {
                    protection: None,
                    alert: Some(true),
                    weapons_restrained: None,
                    disabled: None,
                })
            }
        };
        assert!(state.advance(&p, 4, 0.5, 1.0, &[sample]).is_err());
        assert_eq!(state, before);
    }
}
#[test]
fn newer_report_replaces_only_reported_fields_and_zero_grant_cannot_refresh_it() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    let first = reported(&p, 2);
    state.advance(&p, 4, 0.5, 1.0, &[first.clone()]).unwrap();
    let id = state.contact_for_source(&first.source).unwrap().to_owned();
    let mut second = reported(&p, 3);
    second.position = [200.0, 300.0];
    second.identity = None;
    second.selectable = false;
    state.advance(&p, 5, 0.5, 0.0, &[second.clone()]).unwrap();
    assert_eq!(state.tracks[&id].observed_tick, 2);
    state.advance(&p, 6, 0.5, 1.0, &[second]).unwrap();
    let track = state
        .tracks
        .values()
        .find(|track| track.observed_tick == 3)
        .unwrap();
    assert_eq!(track.position, [200.0, 300.0]);
    assert!(track.identity.is_none() && track.velocity.is_none());
    assert!(!track.selectable);
    assert_eq!(track.supported_secs, 0.0);
}
#[test]
fn return_to_physical_measurement_does_not_infer_motion_from_a_displaced_report() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    let report = reported(&p, 2);
    state.advance(&p, 4, 0.5, 1.0, &[report.clone()]).unwrap();
    let id = state.contact_for_source(&report.source).unwrap().to_owned();
    let physical = sample(&report.source, -1000.0);
    state.advance(&p, 5, 0.5, 1.0, &[physical]).unwrap();
    let track = &state.tracks[&id];
    assert!(track.observation.is_none() && track.velocity.is_none());
    assert!(track.identity.is_none());
    assert_eq!(track.observed_tick, 5);
    assert_eq!(
        track.position,
        measured_position([-1000.0, 0.0], p.position_error)
    );
}
#[test]
fn later_duplicate_observed_identity_retracts_automatic_reference_correlation() {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    let mut first = sample("first-private-source", 10.0);
    first.identity.as_mut().unwrap().references = vec!["mission-ref".into()];
    for tick in 0..2 {
        state.advance(&p, tick, 0.5, 1.0, &[first.clone()]).unwrap();
    }
    let first_id = state
        .contact_for_source("first-private-source")
        .unwrap()
        .to_owned();
    assert_eq!(
        state.tracks[&first_id]
            .identity
            .as_ref()
            .unwrap()
            .references,
        vec!["mission-ref"]
    );
    let duplicate = sample("second-private-source", 100.0);
    state.advance(&p, 2, 0.5, 1.0, &[first, duplicate]).unwrap();
    assert!(state.tracks[&first_id]
        .identity
        .as_ref()
        .unwrap()
        .references
        .is_empty());
}
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
        reading_ticks: 0,
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
        elevation: None,
        observation: None,
        selectable: true,
        source: source.into(),
        position: [x, 0.0],
        support: 1.0,
        identity: Some(TrackIdentity {
            name: "source-name".into(),
            faction: Some("source-faction".into()),
            references: Vec::new(),
        }),
        classification: Some("ship".into()),
        assessment: Some(TrackAssessment {
            protection: None,
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

#[test]
fn live_lifecycle_preserves_paid_support_aging_ambiguity_and_restored_outputs() {
    let mut p = profile();
    p.sample_interval_secs = 1.0;
    let mut checked = BroadTrackState::new(&p);
    let mut live = checked.clone();
    for tick in 0..40 {
        let mut measurements = vec![sample("a", 20.0 + tick as f32 * 0.1)];
        if (6..12).contains(&tick) {
            measurements.push(sample("b", 20.2));
        }
        if (16..28).contains(&tick) {
            measurements.clear();
        }
        let paid = if (10..15).contains(&tick) { 0.0 } else { 0.5 };
        checked
            .advance(&p, tick, 0.25, paid, &measurements)
            .unwrap();
        live.advance_live(&p, tick, 0.25, paid, &measurements)
            .unwrap();
        assert_eq!(live, checked, "canonical state differs at {tick}");
        assert_eq!(
            live.projection_live(&p, tick, 0.25),
            checked.projection(&p, tick, 0.25).unwrap()
        );
        if tick == 18 {
            live = serde_json::from_str(&serde_json::to_string(&live).unwrap()).unwrap();
            live.validate(&p, tick, 0.25).unwrap();
        }
    }
}

#[test]
fn checked_ingress_still_rejects_invalid_findings_transactionally() {
    let mut p = profile();
    p.identity_secs = 0.0;
    let mut state = BroadTrackState::new(&p);
    let mut invalid = sample("a", 20.0);
    invalid.identity.as_mut().unwrap().name.clear();
    let before = state.clone();
    assert!(state.advance(&p, 0, 0.5, 1.0, &[invalid]).is_err());
    assert_eq!(state, before);
    state.owner = SystemId("foreign".into());
    assert!(state.projection(&p, 0, 0.5).is_err());
}
