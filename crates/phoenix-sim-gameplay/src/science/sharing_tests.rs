use super::super::tracks::{BroadMeasurement, TrackIdentity};
use super::*;
use phoenix_model::identity::SystemId;

#[test]
fn relayed_elevation_keeps_its_original_sample_and_rejects_malformed_receipts() {
    use phoenix_model::sensing_tracks::ObservedElevation;
    let p = profile();
    let mut source = observed(1, [20.0, 0.0], [-20.0, 0.0]);
    source.tracks.values_mut().next().unwrap().elevation = Some(ObservedElevation {
        height: 30.0,
        error: p.position_error,
        velocity: Some(2.0),
    });
    let packet = export_samples("source", &source, &p, 1, 1.0).unwrap();
    let mut receiver = BroadTrackState::new(&p);
    receive_samples("receiver", &mut receiver, &p, 2, 1.0, &packet).unwrap();
    let held = projection("receiver", &receiver, &p, 2, 1.0).unwrap()[0].clone();
    assert_eq!(held.elevation, packet[0].record.elevation);
    assert_eq!(held.observed_tick, 1);
    let relay = export_samples("receiver", &receiver, &p, 3, 1.0).unwrap();
    let mut third: BroadTrackState =
        serde_json::from_str(&serde_json::to_string(&receiver).unwrap()).unwrap();
    receive_samples("third", &mut third, &p, 4, 1.0, &relay).unwrap();
    let later = projection("third", &third, &p, 4, 1.0).unwrap()[0].clone();
    assert_eq!(later.elevation, held.elevation);
    assert_eq!(later.observed_tick, 1);
    let before = receiver.clone();
    let mut malformed = packet;
    malformed[0].record.elevation.as_mut().unwrap().error = -1.0;
    assert!(receive_samples("receiver", &mut receiver, &p, 4, 1.0, &malformed).is_err());
    assert_eq!(receiver, before);
}

#[test]
fn independent_planar_geometry_cannot_fuse_disjoint_observed_heights() {
    use phoenix_model::sensing_tracks::ObservedElevation;
    let p = profile();
    let mut first = observed(1, [-2.0, 0.0], [-30.0, 0.0]);
    first.tracks.values_mut().next().unwrap().elevation = Some(ObservedElevation {
        height: 30.0,
        error: p.position_error,
        velocity: None,
    });
    let mut receiver = observed(1, [2.0, 0.0], [30.0, 0.0]);
    receiver.tracks.values_mut().next().unwrap().elevation = Some(ObservedElevation {
        height: -30.0,
        error: p.position_error,
        velocity: None,
    });
    let packet = export_samples("first", &first, &p, 1, 1.0).unwrap();
    receive_samples("receiver", &mut receiver, &p, 1, 1.0, &packet).unwrap();
    let original = receiver.clone();
    let rows = projection("receiver", &receiver, &p, 1, 1.0).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.samples.len() == 1));
    assert_eq!(receiver, original);
}

fn profile() -> BroadConfig {
    BroadConfig {
        reading_ticks: 0,
        system: SystemId("sensor-radar".into()),
        sample_interval_secs: 1.0,
        identity_secs: 0.0,
        classification_secs: 0.0,
        assessment_secs: 0.0,
        position_error: 5.0,
        drift_error_per_sec: 1.0,
        track_retention_secs: 20.0,
    }
}
fn observed(tick: u64, position: [f32; 2], pose: [f32; 2]) -> BroadTrackState {
    let p = profile();
    let mut state = BroadTrackState::new(&p);
    state
        .advance(
            &p,
            tick,
            1.0,
            1.0,
            &[BroadMeasurement {
                elevation: None,
                source: "private-target-route".into(),
                observation: None,
                selectable: true,
                position,
                support: 1.0,
                identity: Some(TrackIdentity {
                    name: "measured label".into(),
                    faction: None,
                    references: vec![],
                }),
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    record_observer_pose(&mut state, tick, pose);
    state
}

#[test]
fn actual_receipt_preserves_origin_sample_pose_and_first_time_across_relays() {
    let p = profile();
    let source = observed(1, [20.0, 0.0], [-20.0, 0.0]);
    let original = export_samples("source-ship", &source, &p, 1, 1.0).unwrap();
    let mut receiver = BroadTrackState::new(&p);
    receive_samples("receiving-ship", &mut receiver, &p, 2, 1.0, &original).unwrap();
    let id = receiver.tracks.keys().next().unwrap().clone();
    assert_eq!(receiver.resolve_current(&id), Some("private-target-route"));
    let received = receiver.tracks[&id].received.as_ref().unwrap();
    assert_eq!(received.provenance.observed_tick, 1);
    assert_eq!(received.provenance.received_tick, Some(2));
    assert_eq!(received.provenance.observer_position, Some([-20.0, 0.0]));
    receive_samples("receiving-ship", &mut receiver, &p, 3, 1.0, &original).unwrap();
    assert_eq!(receiver.tracks.len(), 1);
    assert_eq!(
        receiver.tracks[&id]
            .received
            .as_ref()
            .unwrap()
            .provenance
            .received_tick,
        Some(2)
    );
    assert_eq!(receiver.tracks[&id].supported_secs, 0.0);
    let forwarded = export_samples("receiving-ship", &receiver, &p, 3, 1.0).unwrap();
    assert_eq!(
        forwarded[0].provenance.origin,
        original[0].provenance.origin
    );
    assert_eq!(
        forwarded[0].provenance.sample_id,
        original[0].provenance.sample_id
    );
    let public =
        serde_json::to_string(&projection("receiving-ship", &receiver, &p, 3, 1.0).unwrap())
            .unwrap();
    assert!(!public.contains("private-target-route") && !public.contains("source-ship"));
    let mut third = BroadTrackState::new(&p);
    receive_samples("third-ship", &mut third, &p, 3, 1.0, &forwarded).unwrap();
    receive_samples("third-ship", &mut third, &p, 3, 1.0, &original).unwrap();
    assert_eq!(third.tracks.len(), 1);
}

#[test]
fn reception_off_gap_and_old_samples_never_rejuvenate_or_overwrite_findings() {
    let p = profile();
    let source = observed(1, [20.0, 0.0], [0.0, 0.0]);
    let old = export_samples("source", &source, &p, 1, 1.0).unwrap();
    let mut receiver = BroadTrackState::new(&p);
    receiver.receive_shared = false;
    let untouched = receiver.clone();
    receive_samples("receiver", &mut receiver, &p, 2, 1.0, &old).unwrap();
    assert_eq!(receiver, untouched);
    receiver.receive_shared = true;
    let newer =
        export_samples("source", &observed(2, [25.0, 0.0], [1.0, 0.0]), &p, 2, 1.0).unwrap();
    receive_samples("receiver", &mut receiver, &p, 2, 1.0, &newer).unwrap();
    let held = receiver.tracks.values().next().unwrap().clone();
    receive_samples("receiver", &mut receiver, &p, 3, 1.0, &old).unwrap();
    assert_eq!(receiver.tracks.values().next().unwrap(), &held);
    let picture = projection("receiver", &receiver, &p, 3, 1.0).unwrap();
    assert!(picture[0].predicted && !picture[0].selectable);
    assert_eq!(picture[0].observed_tick, 2);
    assert!(receiver.resolve_current(&picture[0].contact_id).is_none());
}

#[test]
fn independent_useful_geometry_refines_position_but_a_relay_copy_does_not() {
    let p = profile();
    let first = observed(1, [-2.0, 0.0], [-30.0, 0.0]);
    let mut receiver = observed(1, [2.0, 0.0], [30.0, 0.0]);
    let packet = export_samples("first", &first, &p, 1, 1.0).unwrap();
    receive_samples("receiver", &mut receiver, &p, 1, 1.0, &packet).unwrap();
    let originals = receiver.clone();
    let fused = projection("receiver", &receiver, &p, 1, 1.0).unwrap();
    assert!(fused
        .iter()
        .any(|row| row.position_error < 5.0 && row.samples.len() == 2));
    assert!(fused
        .iter()
        .all(|row| row.identity.as_ref().unwrap().name == "measured label"));
    assert_eq!(receiver, originals, "fusion has no canonical mutation");
    let mut relay = BroadTrackState::new(&p);
    let receiver_originals = export_samples("receiver", &receiver, &p, 1, 1.0).unwrap();
    receive_samples("relay", &mut relay, &p, 1, 1.0, &receiver_originals).unwrap();
    let copy = export_samples("relay", &relay, &p, 1, 1.0).unwrap();
    receive_samples("receiver", &mut receiver, &p, 1, 1.0, &copy).unwrap();
    assert_eq!(
        projection("receiver", &receiver, &p, 1, 1.0).unwrap(),
        fused
    );
}

#[test]
fn overlapping_three_return_chain_cannot_choose_an_ambiguous_middle_partner() {
    let p = profile();
    let mut records = Vec::new();
    for (index, x) in [0.0, 1.5, 3.0].into_iter().enumerate() {
        let mut row = projection(
            &format!("ship-{index}"),
            &observed(1, [x, 0.0], [index as f32 * 30.0, 0.0]),
            &p,
            1,
            1.0,
        )
        .unwrap()
        .remove(0);
        row.contact_id = format!("public-{index}");
        row.position = [x, 0.0];
        row.position_error = 1.0;
        records.push(row);
    }
    let before = records.clone();
    refine_geometry(&mut records);
    assert_eq!(
        records, before,
        "reciprocal geometric ambiguity preserves all independent returns"
    );
}

#[test]
fn unequal_radius_fusion_never_excludes_a_compatible_lens_endpoint() {
    let p = profile();
    for second_radius in [2.0, 5.0] {
        let mut a = projection("a", &observed(1, [0.0, 0.0], [-30.0, 0.0]), &p, 1, 1.0)
            .unwrap()
            .remove(0);
        let mut b = projection("b", &observed(1, [9.0, 0.0], [30.0, 0.0]), &p, 1, 1.0)
            .unwrap()
            .remove(0);
        a.position = [0.0, 0.0];
        a.position_error = 10.0;
        b.contact_id = "other-public-contact".into();
        b.position = [9.0, 0.0];
        b.position_error = second_radius;
        let compatible = [9.0 - second_radius, 0.0];
        let mut records = vec![a, b];
        refine_geometry(&mut records);
        assert!(records.iter().all(|record| {
            (record.position[0] - compatible[0]).hypot(record.position[1] - compatible[1])
                <= record.position_error
        }));
        if second_radius == 2.0 {
            assert_eq!(
                records.len(),
                2,
                "no narrower safe enclosure for this unequal lens"
            );
        }
    }
}

#[test]
fn invalid_receipt_is_transactional_and_json_continuation_retains_the_same_samples() {
    let p = profile();
    let mut receiver = BroadTrackState::new(&p);
    let packet =
        export_samples("source", &observed(1, [20.0, 0.0], [0.0, 0.0]), &p, 1, 1.0).unwrap();
    receive_samples("receiver", &mut receiver, &p, 2, 1.0, &packet).unwrap();
    let before = receiver.clone();
    let mut bad = packet.clone();
    bad[0].provenance.observed_tick = 99;
    assert!(receive_samples("receiver", &mut receiver, &p, 2, 1.0, &bad).is_err());
    assert_eq!(receiver, before);
    let mut restored: BroadTrackState =
        serde_json::from_str(&serde_json::to_string(&receiver).unwrap()).unwrap();
    receiver.advance(&p, 3, 1.0, 0.0, &[]).unwrap();
    restored.advance(&p, 3, 1.0, 0.0, &[]).unwrap();
    assert_eq!(receiver, restored);
    assert_eq!(
        projection("receiver", &receiver, &p, 3, 1.0).unwrap(),
        projection("receiver", &restored, &p, 3, 1.0).unwrap()
    );
}

#[test]
fn empty_live_delivery_expires_routes_without_refreshing_original_evidence() {
    let p = profile();
    let source = observed(1, [20.0, 0.0], [-20.0, 0.0]);
    let packet = export_samples("source", &source, &p, 1, 1.0).unwrap();
    let mut receiver = BroadTrackState::new(&p);
    receive_samples("receiver", &mut receiver, &p, 2, 1.0, &packet).unwrap();
    let before = receiver.clone();
    let id = receiver.tracks.keys().next().unwrap().clone();
    assert!(receiver.resolve_current(&id).is_some());
    let mut checked = receiver.clone();
    receive_samples("receiver", &mut checked, &p, 3, 1.0, &[]).unwrap();
    receive_samples_live("receiver", &mut receiver, &p, 3, 1.0, &[]).unwrap();
    assert_eq!(receiver, checked);
    assert_eq!(receiver.tracks, before.tracks);
    assert_eq!(receiver.next_contact, before.next_contact);
    assert_eq!(receiver.resolve_current(&id), None);
    assert_eq!(
        projection_live("receiver", &receiver, &p, 3, 1.0).unwrap(),
        projection("receiver", &checked, &p, 3, 1.0).unwrap()
    );
    receiver.receive_shared = false;
    let stopped = receiver.clone();
    receive_samples_live("receiver", &mut receiver, &p, 4, 1.0, &[]).unwrap();
    assert_eq!(receiver, stopped);
}

#[test]
fn same_origin_fusion_keeps_each_contact_and_provenance_exactly() {
    let p = profile();
    let first = observed(1, [-2.0, 0.0], [-30.0, 0.0]);
    let second = observed(1, [2.0, 0.0], [30.0, 0.0]);
    let mut rows = projection("same-observer", &first, &p, 1, 1.0).unwrap();
    let mut other = projection("same-observer", &second, &p, 1, 1.0)
        .unwrap()
        .remove(0);
    other.contact_id.push_str("-other");
    rows.push(other);
    let before = rows.clone();
    refine_geometry(&mut rows);
    assert_eq!(rows, before);
    // Cross-origin geometry remains exercised by the existing fusion tests.
}
