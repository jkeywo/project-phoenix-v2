use super::*;
use phoenix_model::guided_guidance::GuidanceReceiverProfile;

fn launch(id: &str, tick: u64) -> KnownGuidedLaunch {
    KnownGuidedLaunch {
        round_uuid: id.into(),
        tube: SystemId("torpedo-tube-fore-port".into()),
        launched_tick: tick,
        lifespan_secs: 20.0,
        receiver: Some(GuidanceReceiverProfile {
            range: 200.0,
            minimum_transmission: 0.2,
            maximum_sample_age_secs: 2.0,
        }),
        issued: IssuedGuidanceOrder {
            revision: 0,
            issued_tick: tick,
            target: Some("opaque-contact-7".into()),
            sampled_tick: Some(tick.saturating_sub(1)),
        },
        acknowledged: None,
    }
}

#[test]
fn sent_order_stays_unresolved_without_reverse_receipt_until_known_endurance() {
    let mut state = GuidedSourceKnowledge::default();
    let launch = launch("own-emitted-round", 100);
    let tube = launch.tube.clone();
    assert!(note_launch(&mut state, launch.clone(), 100));
    assert!(!note_launch(&mut state, launch, 100));
    let order = issue(
        &mut state,
        &tube,
        "own-emitted-round",
        Some("new-public-contact".into()),
        Some(108),
        110,
    )
    .unwrap();
    assert_eq!(order.sampled_tick, Some(108));
    let before = state.clone();
    assert!(issue(
        &mut state,
        &SystemId("different-tube".into()),
        "own-emitted-round",
        None,
        None,
        111
    )
    .is_none());
    assert!(issue(&mut state, &tube, "guessed-remote-id", None, None, 111).is_none());
    assert!(issue(&mut state, &tube, "own-emitted-round", None, None, 109).is_none());
    assert_eq!(state, before);
    assert!(!acknowledge(
        &mut state,
        &tube,
        "own-emitted-round",
        &order,
        109
    ));
    assert_eq!(state, before);
    assert!(retire(&mut state, 1100, 1.0 / 60.0));
    assert!(project(&state, &tube, true)[0].unresolved);
    assert_eq!(project(&state, &tube, true)[0].sampled_tick, Some(108));
    assert!(!project(&state, &tube, false)[0].can_retarget);
    assert_eq!(
        state, before,
        "no remote survival/health/pose input exists to rewrite source knowledge"
    );
    assert!(acknowledge(
        &mut state,
        &tube,
        "own-emitted-round",
        &order,
        1110
    ));
    assert!(!project(&state, &tube, true)[0].unresolved);
    let stopped = issue(&mut state, &tube, "own-emitted-round", None, None, 1111).unwrap();
    assert!(project(&state, &tube, true)[0].unresolved);
    assert!(!acknowledge(
        &mut state,
        &tube,
        "own-emitted-round",
        &order,
        1112
    ));
    assert!(acknowledge(
        &mut state,
        &tube,
        "own-emitted-round",
        &stopped,
        1113
    ));
    assert!(retire(&mut state, 1300, 1.0 / 60.0));
    assert!(state.launches.is_empty());
}

#[test]
fn json_continuation_preserves_sent_and_acknowledged_frontiers_without_physical_roster() {
    let mut original = GuidedSourceKnowledge::default();
    let launch = launch("own-emitted-round", 1_000_000_000);
    let tube = launch.tube.clone();
    assert!(note_launch(&mut original, launch, 1_000_000_000));
    let mut resumed: GuidedSourceKnowledge =
        serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    for step in 0..360 {
        let now = 1_000_000_001 + step;
        for state in [&mut original, &mut resumed] {
            if step == 10 || step == 50 {
                issue(
                    state,
                    &tube,
                    "own-emitted-round",
                    Some(format!("contact-{step}")),
                    Some(now - 4),
                    now,
                )
                .unwrap();
            }
            if step == 100 {
                let sent = state.launches["own-emitted-round"].issued.clone();
                assert!(acknowledge(state, &tube, "own-emitted-round", &sent, now));
            }
            assert!(retire(state, now, 1.0 / 60.0));
            validate(state, now).unwrap();
        }
        assert_eq!(resumed, original);
        assert_eq!(
            serde_json::to_value(&resumed).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
    }
    assert_eq!(
        resumed.launches["own-emitted-round"].issued.sampled_tick,
        Some(1_000_000_047)
    );
    assert_eq!(
        resumed.launches["own-emitted-round"]
            .acknowledged
            .as_ref()
            .unwrap()
            .received_tick,
        1_000_000_101
    );
}

#[test]
fn malformed_saved_claims_and_independent_equipment_cannot_issue_orders() {
    let mut valid = GuidedSourceKnowledge::default();
    let launch = launch("own-emitted-round", 100);
    let tube = launch.tube.clone();
    assert!(note_launch(&mut valid, launch, 100));
    for fault in 0..7 {
        let mut bad = valid.clone();
        let entry = bad.launches.get_mut("own-emitted-round").unwrap();
        match fault {
            0 => entry.round_uuid = "different-id".into(),
            1 => entry.launched_tick = 101,
            2 => entry.lifespan_secs = f32::NAN,
            3 => entry.issued.sampled_tick = Some(101),
            4 => entry.tube.0 = "  ".into(),
            5 => entry.receiver.as_mut().unwrap().range = 0.0,
            _ => {
                entry.acknowledged = Some(GuidanceAcknowledgement {
                    order: IssuedGuidanceOrder {
                        revision: 1,
                        ..entry.issued.clone()
                    },
                    received_tick: 100,
                })
            }
        }
        assert!(validate(&bad, 100).is_err(), "forged frontier {fault}");
    }
    let mut independent = valid.clone();
    independent
        .launches
        .get_mut("own-emitted-round")
        .unwrap()
        .receiver = None;
    let before = independent.clone();
    assert!(issue(
        &mut independent,
        &tube,
        "own-emitted-round",
        None,
        None,
        101
    )
    .is_none());
    assert_eq!(independent, before);
    assert!(!project(&independent, &tube, true)[0].can_retarget);
    assert!(!retire(&mut independent, 101, f64::NAN));
    assert_eq!(independent, before);
}

#[test]
fn older_acknowledgement_cannot_claim_an_order_after_the_latest_issue() {
    let mut state = GuidedSourceKnowledge::default();
    let launch = launch("own-emitted-round", 100);
    let tube = launch.tube.clone();
    assert!(note_launch(&mut state, launch, 100));
    let first = issue(
        &mut state,
        &tube,
        "own-emitted-round",
        Some("contact-a".into()),
        Some(101),
        102,
    )
    .unwrap();
    assert!(acknowledge(
        &mut state,
        &tube,
        "own-emitted-round",
        &first,
        103
    ));
    issue(
        &mut state,
        &tube,
        "own-emitted-round",
        Some("contact-b".into()),
        Some(109),
        110,
    )
    .unwrap();
    // A delayed receipt may be later than the newest locally issued order.
    state
        .launches
        .get_mut("own-emitted-round")
        .unwrap()
        .acknowledged
        .as_mut()
        .unwrap()
        .received_tick = 121;
    validate(&state, 130).unwrap();
    let unchanged = state.clone();
    let mut forged = state.clone();
    forged
        .launches
        .get_mut("own-emitted-round")
        .unwrap()
        .acknowledged
        .as_mut()
        .unwrap()
        .order
        .issued_tick = 120;
    assert!(validate(&forged, 130).is_err());
    assert_eq!(state, unchanged);
}

#[test]
fn delayed_emission_uses_its_actual_first_id_not_a_later_order_on_the_same_tube() {
    let mut state = GuidedSourceKnowledge::default();
    let tube = SystemId("torpedo-tube-fore-port".into());
    let first = KnownGuidedLaunchCommand {
        tube: tube.clone(),
        issued_tick: 100,
        target: Some("original-public-contact".into()),
        sampled_tick: Some(99),
    };
    assert!(note_launch_command(
        &mut state,
        "first-actual-emitted-id".into(),
        first.clone(),
        100
    ));
    assert!(note_launch_command(
        &mut state,
        "later-actual-emitted-id".into(),
        KnownGuidedLaunchCommand {
            issued_tick: 120,
            target: Some("different-public-contact".into()),
            sampled_tick: Some(119),
            ..first.clone()
        },
        120
    ));
    assert!(!note_launch_command(
        &mut state,
        "first-actual-emitted-id".into(),
        first.clone(),
        120
    ));
    let restored: GuidedSourceKnowledge =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    validate(&restored, 120).unwrap();
    assert_eq!(restored.launch_commands["first-actual-emitted-id"], first);
    assert_eq!(
        restored.launch_commands["later-actual-emitted-id"]
            .target
            .as_deref(),
        Some("different-public-contact")
    );
}
