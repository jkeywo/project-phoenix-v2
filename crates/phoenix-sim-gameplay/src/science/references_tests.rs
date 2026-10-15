use super::*;
use crate::science::tracks::{BroadMeasurement, BroadTrackState, TrackIdentity};
use phoenix_model::{
    identity::SystemId,
    messages::{DossierEvidenceSnapshot, DossierSnapshot},
    sensing::BroadConfig,
};

fn track() -> BroadTrack {
    let profile = BroadConfig {
        system: SystemId("radar".into()),
        reading_ticks: 2,
        sample_interval_secs: 1.0,
        identity_secs: 0.0,
        classification_secs: 1.0,
        assessment_secs: 2.0,
        position_error: 1.0,
        drift_error_per_sec: 1.0,
        track_retention_secs: 30.0,
    };
    let mut state = BroadTrackState::new(&profile);
    state
        .advance(
            &profile,
            10,
            1.0,
            1.0,
            &[BroadMeasurement {
                source: "private".into(),
                observation: None,
                selectable: true,
                position: [5.0, 0.0],
                support: 1.0,
                identity: Some(TrackIdentity {
                    name: "observed-name".into(),
                    faction: None,
                    references: Vec::new(),
                }),
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    state.tracks.into_values().next().unwrap()
}
fn dossier(unusual: bool) -> DossierBlackboard {
    DossierBlackboard {
        subjects: vec![DossierSnapshot {
            uuid: "held-subject".into(),
            name: "observed-name".into(),
            evidence: vec![DossierEvidenceSnapshot {
                reference: Some(KnownReferenceMetadata {
                    id: "known-record".into(),
                    unusual,
                    reading_ticks: 2,
                }),
                text: "reference.text".into(),
                provenance: "records".into(),
                gathered_at_tick: 2,
                observation: None,
            }],
            ..Default::default()
        }],
    }
}
#[test]
fn unusual_record_requires_deliberate_attachment_and_never_changes_measurement() {
    let mut held = track();
    let dossiers = dossier(true);
    assert!(project(&held, &dossiers, true).is_empty());
    let before = held.clone();
    let reference = &dossiers.subjects[0].evidence[0].reference.as_ref().unwrap();
    attach(&mut held, reference, 12).unwrap();
    attach(&mut held, reference, 20).unwrap();
    let mut expected = before;
    expected
        .attached_references
        .insert("known-record".into(), 12);
    assert_eq!(held, expected);
    let view = project(&held, &dossiers, true);
    assert_eq!(view[0].gathered_at_tick, 2);
    assert_eq!(view[0].attached_at_tick, 12);
    assert_eq!(view[0].provenance, "records");
    let resumed: BroadTrack = serde_json::from_str(&serde_json::to_string(&held).unwrap()).unwrap();
    assert_eq!(project(&resumed, &dossiers, true), view);
}
#[test]
fn routine_matching_uses_only_observed_name_and_exact_permitted_findings() {
    let mut held = track();
    assert_eq!(project(&held, &dossier(false), true).len(), 1);
    held.identity = None;
    assert!(project(&held, &dossier(false), true).is_empty());
    held.attached_references.insert("known-record".into(), 12);
    assert!(project(&held, &DossierBlackboard::default(), true).is_empty());
    let mut ambiguous = dossier(true);
    let mut conflicting = ambiguous.subjects[0].clone();
    conflicting.evidence[0].text = "different-record".into();
    ambiguous.subjects.push(conflicting);
    assert!(project(&held, &ambiguous, true).is_empty());
}

#[test]
fn attachment_cannot_precede_its_held_evidence_or_exceed_the_saved_frontier() {
    let held = track();
    let profile = BroadConfig {
        system: SystemId("radar".into()),
        reading_ticks: 0,
        sample_interval_secs: 1.0,
        identity_secs: 0.0,
        classification_secs: 1.0,
        assessment_secs: 2.0,
        position_error: 1.0,
        drift_error_per_sec: 1.0,
        track_retention_secs: 30.0,
    };
    let mut state = BroadTrackState::new(&profile);
    state.tracks.insert(held.contact_id.clone(), held);
    let known = BTreeMap::from([("known-record".into(), 8)]);
    let id = state.tracks.keys().next().unwrap().clone();
    state
        .tracks
        .get_mut(&id)
        .unwrap()
        .attached_references
        .insert("known-record".into(), 7);
    assert!(validate_attachments(&state, &known, 20).is_err());
    state
        .tracks
        .get_mut(&id)
        .unwrap()
        .attached_references
        .insert("known-record".into(), 8);
    assert!(validate_attachments(&state, &known, 20).is_ok());
    state
        .tracks
        .get_mut(&id)
        .unwrap()
        .attached_references
        .insert("known-record".into(), 21);
    assert!(validate_attachments(&state, &known, 20).is_err());
}

#[test]
fn automatic_reference_join_never_predates_the_later_held_finding() {
    let held = track();
    let mut dossiers = dossier(false);
    dossiers.subjects[0].evidence[0].gathered_at_tick = 18;
    let view = project(&held, &dossiers, true);
    assert_eq!(view[0].gathered_at_tick, 18);
    assert_eq!(view[0].attached_at_tick, 18);
    assert!(
        held.attached_references.is_empty(),
        "routine projection adds no duplicate evidence owner"
    );
}

#[test]
fn equal_public_names_withhold_routine_matches_but_preserve_deliberate_hypotheses() {
    use phoenix_model::sensing_tracks::SensorTrackSnapshot;
    let mut held = track();
    let first: SensorTrackSnapshot = serde_json::from_value(serde_json::json!({
        "contact_id":"contact-a", "selectable":true, "observed_tick":10,
        "position":[5.0,0.0], "velocity":null, "position_error":1.0,
        "stage":"identified", "identity":held.identity.clone(), "identity_tick":10,
        "classification":null, "classification_tick":null, "assessment":null,
        "assessment_tick":null, "predicted":false
    }))
    .unwrap();
    let mut second = first.clone();
    second.contact_id = "contact-b".into();
    let names = routine_names(&[first.clone(), second]);
    assert!(!names.contains("observed-name"));
    assert!(project(&held, &dossier(false), names.contains("observed-name")).is_empty());
    let unique = routine_names(&[first]);
    assert_eq!(
        project(&held, &dossier(false), unique.contains("observed-name")).len(),
        1
    );
    attach(
        &mut held,
        dossier(true).subjects[0].evidence[0]
            .reference
            .as_ref()
            .unwrap(),
        12,
    )
    .unwrap();
    assert_eq!(
        project(&held, &dossier(true), false).len(),
        1,
        "a deliberate hypothesis is not automatic identity proof"
    );
}
