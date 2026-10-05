use crate::command_admission::{CommandOrder, HostSlot, ShipKey};
use crate::core::messages::{SystemControlPayload, SystemId};
use crate::lobby::start_policy::{StartGrant, StartGrantMode};
use crate::lockstep::{
    DigestFrame, HostLossFrame, MeshCommand, MeshFrame, TickFrame, HOST_MESH_PROTOCOL,
};

fn tick_frame() -> MeshFrame {
    MeshFrame::Tick(TickFrame {
        from: HostSlot(2),
        tick: 412,
        ready_through: 418,
        commands: vec![
            MeshCommand {
                tick: 418,
                order: CommandOrder::new(HostSlot(2), 7),
                ship: ShipKey("00000000-0000-8000-8000-000000000001".into()),
                target: SystemId("helm-steering".into()),
                payload: SystemControlPayload::SetSteering { value: -0.4 },
            },
            MeshCommand {
                tick: 418,
                order: CommandOrder::new(HostSlot(2), 8),
                ship: ShipKey("00000000-0000-8000-8000-000000000001".into()),
                target: SystemId("red-alert".into()),
                payload: SystemControlPayload::SetRedAlert { active: true },
            },
        ],
        start_grant: Some(StartGrant {
            id: "start-7".into(),
            mode: StartGrantMode::Forced,
            operator_id: Some("gm-1".into()),
            apply_tick: 419,
        }),
    })
}

/// The wire shape is the envelope `gui/host-mesh.js` owns, and a frame
/// survives it unchanged.
#[test]
fn a_tick_frame_round_trips_through_the_shared_envelope() {
    let frame = tick_frame();
    let text = super::encode_mesh_frame(&frame).expect("encodes");
    assert!(
        text.contains(&format!("\"m\":{HOST_MESH_PROTOCOL}")),
        "the revision travels: {text}"
    );
    assert!(text.contains("\"t\":\"tick\""), "{text}");
    assert!(
        text.contains("\"tick\":412"),
        "the envelope's own tick stamp — the field #1114 added for exactly \
             this — must carry the tick the frame applies at: {text}"
    );
    assert!(
        text.contains("\"apply_tick\":419"),
        "the owner-scheduled start boundary travels inside the tick frame: {text}"
    );
    assert_eq!(super::decode_mesh_frame(&text), Some(frame));
}

/// A digest crosses as a hex STRING.
///
/// The load-bearing half of this vocabulary's encoding: a digest is a `u64`
/// and JavaScript's number type loses integers above 2^53, so a JSON number
/// would silently round the very value two hosts compare — reporting a
/// divergence the fleet does not have, or missing one it does.
#[test]
fn a_digest_crosses_as_a_string_because_json_numbers_lose_it() {
    let digest = 0xdead_beef_dead_beef_u64;
    assert!(
        digest > (1_u64 << 53),
        "precondition: the sample must be big enough for a JSON number to \
             round it, or this test proves nothing"
    );
    let frame = MeshFrame::Digest(DigestFrame {
        from: HostSlot(1),
        tick: 300,
        digest,
    });
    let text = super::encode_mesh_frame(&frame).expect("encodes");
    assert!(
        text.contains("\"digest\":\"deadbeefdeadbeef\""),
        "the digest must be a hex string: {text}"
    );
    assert_eq!(super::decode_mesh_frame(&text), Some(frame));
}

/// A snapshot chunk round-trips through the shared envelope (issue #1117),
/// and its two u64 fields — `transfer_id` and `whole_hash` — cross as hex
/// strings for the same reason a digest does: a JSON number rounds above 2^53,
/// and the whole-hash is the value a receiver verifies the record against.
#[test]
fn a_snapshot_chunk_round_trips_and_keeps_its_u64_fields_exact() {
    use crate::lockstep::SnapshotChunk;
    let whole_hash = 0xfeed_face_dead_beef_u64;
    let transfer_id = 0x0123_4567_89ab_cdef_u64;
    assert!(whole_hash > (1_u64 << 53) && transfer_id > (1_u64 << 53));
    let frame = MeshFrame::Snapshot(SnapshotChunk {
        from: HostSlot(2),
        transfer_id,
        tick: 400,
        seq: 3,
        total: 9,
        whole_hash,
        crc: 0xdead_beef,
        text: "a RON slice with \"quotes\" and \n a newline".to_string(),
    });
    let text = super::encode_mesh_frame(&frame).expect("encodes");
    assert!(text.contains("\"t\":\"snapshot\""), "{text}");
    assert!(
        text.contains("\"whole_hash\":\"feedfacedeadbeef\""),
        "{text}"
    );
    assert!(
        text.contains("\"transfer_id\":\"0123456789abcdef\""),
        "{text}"
    );
    assert_eq!(super::decode_mesh_frame(&text), Some(frame));
}

/// Everything that is not a simulation frame of a revision this build
/// speaks answers `None`, together — the same discipline
/// `decodeHostFrame` keeps on the other side of the wire.
#[test]
fn anything_that_is_not_a_frame_of_this_revision_is_refused() {
    for raw in [
        "not json at all",
        r#"{"type":"Identify","token":"abc"}"#,
        // A superseded revision — refused whole rather than half-read.
        r#"{"m":2,"t":"tick","tick":1,"d":{"from":1,"tick":1,"ready_through":1,"commands":[]}}"#,
        // A lobby frame on the simulation decoder.
        r#"{"m":4,"t":"hello","tick":null,"d":{}}"#,
        // A tick frame missing its watermark and commands.
        r#"{"m":4,"t":"tick","tick":1,"d":{"from":1,"tick":1}}"#,
        // A digest as a JSON number, which loses the top bits — refused.
        r#"{"m":4,"t":"digest","tick":1,"d":{"from":1,"tick":1,"digest":12345}}"#,
        // A host-loss frame missing the slot it names.
        r#"{"m":4,"t":"host-loss","tick":1,"d":{"from":1,"tick":1}}"#,
        // A slot-claim missing the slot it reclaims (issue #1120).
        r#"{"m":4,"t":"slot-claim","tick":1,"d":{"from":1,"claim_seq":1}}"#,
    ] {
        assert_eq!(
            super::decode_mesh_frame(raw),
            None,
            "must be refused rather than half understood: {raw}"
        );
    }
}

/// A host-loss report (issue #1119) survives the shared envelope: the slot
/// it names, the reporter, and the agreed tick all come back unchanged.
#[test]
fn a_host_loss_frame_round_trips_through_the_shared_envelope() {
    let frame = MeshFrame::HostLoss(HostLossFrame {
        from: HostSlot(1),
        lost: HostSlot(3),
        tick: 418,
    });
    let text = super::encode_mesh_frame(&frame).expect("encodes");
    assert!(
        text.contains(&format!("\"m\":{HOST_MESH_PROTOCOL}")),
        "the revision travels: {text}"
    );
    assert!(text.contains("\"t\":\"host-loss\""), "{text}");
    assert!(
        text.contains("\"lost\":3"),
        "the slot whose host left must survive the wire: {text}"
    );
    assert!(
        text.contains("\"tick\":418"),
        "the agreed disconnect tick must survive the wire: {text}"
    );
    assert_eq!(super::decode_mesh_frame(&text), Some(frame));
}

/// A slot-claim announcement (issue #1120) survives the shared envelope: the
/// slot it reclaims, the owner that stamped it, the deterministic claim
/// sequence and the tick all come back unchanged.
#[test]
fn a_slot_claim_frame_round_trips_through_the_shared_envelope() {
    let frame = MeshFrame::SlotClaim(crate::lockstep::SlotClaimFrame {
        from: crate::command_admission::HostSlot(1),
        slot: crate::command_admission::HostSlot(3),
        claim_seq: 7,
        tick: 512,
    });
    let text = super::encode_mesh_frame(&frame).expect("encodes");
    assert!(text.contains("\"t\":\"slot-claim\""), "{text}");
    assert!(
        text.contains("\"slot\":3"),
        "the slot being reclaimed must survive the wire: {text}"
    );
    assert!(
        text.contains("\"claim_seq\":7"),
        "the deterministic tiebreak must survive the wire: {text}"
    );
    assert_eq!(super::decode_mesh_frame(&text), Some(frame));
}

#[test]
fn an_attributed_gm_action_round_trips_through_the_shared_envelope() {
    let frame = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Granted(
        crate::gm_action::GmActionGrant {
            from: HostSlot(2),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: crate::gm_action::GmActionId::new("pause-17").unwrap(),
            recovery_generation: 0,
            apply_tick: 419,
            order: crate::gm_action::GmActionOrder::new(HostSlot(2), 17),
            action: crate::gm_action::GmAction::SetSessionPaused { active: true },
        },
    ));
    let text = super::encode_mesh_frame(&frame).expect("encodes");
    assert!(
        text.contains(&format!("\"m\":{HOST_MESH_PROTOCOL}")),
        "the revision travels: {text}"
    );
    assert!(text.contains("\"t\":\"gm-action\""), "{text}");
    assert!(text.contains("\"operator_id\":\"gm-1\""), "{text}");
    assert!(text.contains("\"tick\":419"), "{text}");
    assert_eq!(super::decode_mesh_frame(&text), Some(frame));
}

#[test]
fn gm_proposals_and_canonical_refusals_round_trip_on_the_same_lane() {
    let proposal = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Proposal(
        crate::gm_action::GmActionProposal {
            from: HostSlot(2),
            operator_id: "gm-1".into(),
            correlation: crate::gm_action::GmActionId::new("proposal-1").unwrap(),
            action: crate::gm_action::GmAction::SetSessionPaused { active: true },
        },
    ));
    let refusal = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(
        crate::gm_action::GmActionRefusal {
            sequenced_by: HostSlot(1),
            requester: HostSlot(2),
            operator_id: "gm-1".into(),
            correlation: crate::gm_action::GmActionId::new("proposal-1").unwrap(),
            action_kind: crate::gm_action::GmActionKind::SessionPause,
            requested_active: true,
            tick: 419,
            reason: crate::gm_action::GmActionRefusalReason::WrongPhase,
            target: None,
            verb: None,
            lever: None,
            effect_scope: None,
            objective_verb: None,
            objective_instance_scope: None,
            objective_recipients: None,
            comms_recipients: None,
            observer: None,
            npc_doctrine: None,
        },
    ));
    // A refused Fire crosses the same lane still naming the event it tried
    // to fire (issue #1301); every GM must read the same attributed answer.
    let refused_fire = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(
        crate::gm_action::GmActionRefusal {
            sequenced_by: HostSlot(1),
            requester: HostSlot(2),
            operator_id: "gm-1".into(),
            correlation: crate::gm_action::GmActionId::new("fire-1").unwrap(),
            action_kind: crate::gm_action::GmActionKind::EventControl,
            requested_active: true,
            tick: 419,
            reason: crate::gm_action::GmActionRefusalReason::UnknownGmEvent,
            target: Some("base-world::breach_alarm".into()),
            verb: Some(crate::gm_action::GmEventVerb::Fire),
            lever: None,
            effect_scope: None,
            objective_verb: None,
            objective_instance_scope: None,
            objective_recipients: None,
            comms_recipients: None,
            observer: None,
            npc_doctrine: None,
        },
    ));
    // And a refused SKIP crosses it naming both the event and the lever
    // (issue #1304): without the lever every GM's feed would report it as a
    // refused Fire, the opposite sentence about the same button.
    let refused_skip = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(
        crate::gm_action::GmActionRefusal {
            sequenced_by: HostSlot(1),
            requester: HostSlot(2),
            operator_id: "gm-1".into(),
            correlation: crate::gm_action::GmActionId::new("skip-1").unwrap(),
            action_kind: crate::gm_action::GmActionKind::EventControl,
            requested_active: true,
            tick: 419,
            reason: crate::gm_action::GmActionRefusalReason::UnknownGmEvent,
            target: Some("base-world::breach_alarm".into()),
            verb: None,
            lever: Some(crate::gm_event::GmEventLever::SkipNext),
            effect_scope: None,
            objective_verb: None,
            objective_instance_scope: None,
            objective_recipients: None,
            comms_recipients: None,
            observer: None,
            npc_doctrine: None,
        },
    ));
    for frame in [proposal, refusal, refused_fire, refused_skip] {
        let text = super::encode_mesh_frame(&frame).unwrap();
        assert_eq!(super::decode_mesh_frame(&text), Some(frame));
    }
}

/// The Skip ingress (issue #1304): its own verb on the same exact narrow
/// shape, so a page that means to fire cannot arm a skip by getting one
/// value wrong -- the two levers do opposite things to the same event.
#[test]
fn arming_a_skip_uses_its_own_verb_on_the_same_exact_typed_ingress() {
    let request = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":"base-world::courier_lost"}"#,
        )
        .expect("valid skip request");
    assert_eq!(
        request.action,
        crate::gm_action::GmAction::ArmGmEventSkip {
            event: "base-world::courier_lost".into(),
        }
    );
    assert_eq!(
        request.action.kind(),
        crate::gm_action::GmActionKind::EventControl,
        "one family, three levers"
    );
    assert_eq!(
        request.action.event_lever(),
        Some(crate::gm_event::GmEventLever::SkipNext),
        "and the durable result must be able to say WHICH lever"
    );
    assert_eq!(request.action.target_id(), Some("base-world::courier_lost"));
    assert_eq!(request.action.requested_pause(), None);

    for refused in [
        // An unqualified id cannot name one event across layers.
        r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":"courier_lost"}"#,
        r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":""}"#,
        r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":"base-world::"}"#,
        // A second field is a wider mutation surface, not a Skip.
        r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":"base-world::a","count":2}"#,
        r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip"}"#,
    ] {
        assert!(
            super::decode_gm_action_request(refused).is_none(),
            "must fail closed: {refused}"
        );
    }
}

#[test]
fn first_time_gm_join_frames_round_trip_on_the_shared_envelope() {
    use crate::gm_join::{
        GmJoinApproval, GmJoinCandidate, GmJoinCommit, GmJoinFrame, GmJoinId, GmJoinKind,
        GmJoinRefusal,
    };

    let candidate = GmJoinCandidate {
        host: HostSlot(3),
        operator_id: "gm-2".into(),
    };
    let frames = [
        MeshFrame::GmJoin(GmJoinFrame::Pause(GmJoinApproval {
            id: GmJoinId(7),
            kind: GmJoinKind::FirstTime,
            owner: HostSlot(1),
            approved_by: HostSlot(2),
            candidate: candidate.clone(),
            apply_tick: 419,
            transfer_id: 0x1293_0000_0000_0007,
        })),
        MeshFrame::GmJoin(GmJoinFrame::Restored {
            from: HostSlot(3),
            id: GmJoinId(7),
            digest: 0xfeed_beef,
        }),
        MeshFrame::GmJoin(GmJoinFrame::RestoreBoundary {
            from: HostSlot(3),
            id: GmJoinId(7),
            boundary: 4,
        }),
        MeshFrame::GmJoin(GmJoinFrame::Committed(GmJoinCommit {
            id: GmJoinId(7),
            kind: GmJoinKind::Reconnect,
            owner: HostSlot(1),
            candidate: candidate.clone(),
            tick: 419,
            digest: 0xfeed_beef,
        })),
        MeshFrame::GmJoin(GmJoinFrame::Refused {
            from: HostSlot(1),
            id: GmJoinId(7),
            reason: GmJoinRefusal::DigestMismatch {
                expected: 1,
                restored: 2,
            },
        }),
    ];
    for frame in frames {
        let text = super::encode_mesh_frame(&frame).expect("encodes");
        assert!(
            text.contains(&format!("\"m\":{HOST_MESH_PROTOCOL}")),
            "the revision travels: {text}"
        );
        assert!(text.contains("\"t\":\"gm-join\""), "{text}");
        assert_eq!(super::decode_mesh_frame(&text), Some(frame));
    }
}

/// The multi-peer live-restore lane crosses the JS wire intact (issue
/// #1447), carrying the canonical order that says WHICH restore each frame
/// belongs to - and carrying no candidate slot id, because which private
/// catalogue row a GM picked is their storage key and not fleet traffic.
#[test]
fn live_restore_frames_round_trip_on_the_shared_envelope() {
    use crate::gm_action::GmActionOrder;
    use crate::gm_restore::{GmRestoreFailure, GmRestoreFrame};

    let restore = GmActionOrder::new(HostSlot(3), 11);
    let frames = [
        GmRestoreFrame::Ready {
            from: HostSlot(2),
            restore,
        },
        GmRestoreFrame::Loaded {
            from: HostSlot(2),
            restore,
            tick: 418,
            digest: 0xdead_beef_0bad_c0de,
        },
        GmRestoreFrame::Unable {
            from: HostSlot(1),
            restore,
            failure: GmRestoreFailure::TransferIncomplete {
                detail: "the candidate never finished arriving".into(),
            },
        },
        GmRestoreFrame::Settle {
            from: HostSlot(3),
            restore,
            commit: false,
            failure: Some(GmRestoreFailure::PeerDigestMismatch { peers: 2 }),
        },
        GmRestoreFrame::Settle {
            from: HostSlot(3),
            restore,
            commit: true,
            failure: None,
        },
    ];
    for frame in frames {
        let wire = MeshFrame::GmRestore(frame.clone());
        let text = super::encode_mesh_frame(&wire).expect("encodes");
        assert!(
            text.contains(&format!("\"m\":{HOST_MESH_PROTOCOL}")),
            "the revision travels: {text}"
        );
        assert!(text.contains("\"t\":\"gm-restore\""), "{text}");
        assert!(
            !text.contains("\"candidate\""),
            "a catalogue key is one peer's storage, never fleet traffic: {text}"
        );
        assert_eq!(super::decode_mesh_frame(&text), Some(wire));
    }
}

#[test]
fn the_gm_action_ingress_is_one_exact_bounded_typed_shape() {
    let request = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"pause-17","action":"set_session_paused","active":true}"#,
        )
        .expect("valid request");
    assert_eq!(request.operator_id, "gm-1");
    assert_eq!(request.correlation.as_str(), "pause-17");
    assert_eq!(request.action.requested_pause(), Some(true));

    for refused in [
        r#"{"operator_id":"","correlation":"pause-17","action":"set_session_paused","active":true}"#,
        r#"{"operator_id":"gm-1","correlation":"bad id","action":"set_session_paused","active":true}"#,
        r#"{"operator_id":"gm-1","correlation":"pause-17","action":"toggle_pause","active":true}"#,
        r#"{"operator_id":"gm-1","correlation":"pause-17","action":"set_session_paused","active":true,"component":"Transform"}"#,
    ] {
        assert!(
            super::decode_gm_action_request(refused).is_none(),
            "must fail closed: {refused}"
        );
    }
}

#[test]
fn station_takeover_and_existing_system_commands_share_the_exact_typed_ingress() {
    let takeover = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"take-17","action":"set_station_puppet","ship":"ship-1","station":"captain","active":true}"#,
        )
        .expect("valid takeover request");
    assert_eq!(
        takeover.action,
        crate::gm_action::GmAction::SetStationPuppet {
            ship: crate::command_admission::log::ShipKey("ship-1".into()),
            station: crate::core::messages::StationId("captain".into()),
            active: true,
        }
    );

    let command = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"command-17","action":"issue_station_command","ship":"ship-1","station":"captain","target":"red-alert","payload":{"type":"SetRedAlert","data":{"active":true}}}"#,
        )
        .expect("valid existing System command");
    let crate::gm_action::GmAction::IssueStationCommand { payload, .. } = command.action else {
        panic!("decoded the wrong typed action")
    };
    assert_eq!(
        super::decode_canonical_system_command(payload.as_str()),
        Some(crate::core::messages::SystemControlPayload::SetRedAlert { active: true })
    );
    assert!(
        super::decode_canonical_system_command(
            r#"{ "type":"SetRedAlert", "data":{"active":true} }"#
        )
        .is_none(),
        "only the canonical command bytes replay"
    );

    for refused in [
        r#"{"operator_id":"gm-1","correlation":"take-17","action":"set_station_puppet","ship":"ship-1","station":"captain","active":true,"authority":"human"}"#,
        r#"{"operator_id":"gm-1","correlation":"command-17","action":"issue_station_command","ship":"ship-1","station":"captain","target":"red-alert","payload":{"type":"NotACommand"}}"#,
    ] {
        assert!(
            super::decode_gm_action_request(refused).is_none(),
            "must fail closed: {refused}"
        );
    }
}

/// The Fire ingress (issue #1301) shares the exact same narrow shape: one
/// stable layer-qualified event id, nothing else, and a per-shape field
/// count so a second target cannot ride along.
#[test]
fn firing_an_authored_gm_event_uses_the_same_exact_typed_ingress() {
    let request = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":"base-world::breach_alarm"}"#,
        )
        .expect("valid fire request");
    assert_eq!(
        request.action,
        crate::gm_action::GmAction::FireGmEvent {
            event: "base-world::breach_alarm".into(),
        }
    );
    assert_eq!(
        request.action.kind(),
        crate::gm_action::GmActionKind::EventControl
    );
    assert_eq!(
        request.action.target_id(),
        Some("base-world::breach_alarm"),
        "the durable result must be able to say WHICH event was fired"
    );
    assert_eq!(request.action.requested_pause(), None);

    for refused in [
        // An unqualified id cannot name one event across layers.
        r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":"breach_alarm"}"#,
        r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":""}"#,
        r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":"base-world::"}"#,
        // A second field is a wider mutation surface, not a Fire.
        r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":"base-world::a","handler":"anything"}"#,
        r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event"}"#,
    ] {
        assert!(
            super::decode_gm_action_request(refused).is_none(),
            "must fail closed: {refused}"
        );
    }
}

/// Issue #1303: pausing an authored event crosses the same narrow typed
/// ingress, carrying the ABSOLUTE state rather than a toggle.
#[test]
fn pausing_an_authored_gm_event_uses_the_same_exact_typed_ingress() {
    for (raw, active) in [
        (
            r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::breach_alarm","active":true}"#,
            true,
        ),
        (
            r#"{"operator_id":"gm-1","correlation":"resume-3","action":"set_event_paused","event":"base-world::breach_alarm","active":false}"#,
            false,
        ),
    ] {
        let request = super::decode_gm_action_request(raw).expect("valid pause request");
        assert_eq!(
            request.action,
            crate::gm_action::GmAction::SetEventPaused {
                event: "base-world::breach_alarm".into(),
                active,
            }
        );
        assert_eq!(
            request.action.kind(),
            crate::gm_action::GmActionKind::EventControl,
            "Pause routes to the mission surface beside Fire"
        );
        assert_eq!(
            request.action.verb(),
            Some(crate::gm_action::GmEventVerb::Pause),
            "and the durable result must be able to say WHICH lever it was"
        );
        assert_eq!(request.action.target_id(), Some("base-world::breach_alarm"));
        assert_eq!(
            request.action.requested_pause(),
            None,
            "a paused EVENT is not a paused session"
        );
        assert_eq!(request.action.requested_active(), active);
    }

    for refused in [
        // An unqualified id cannot name one event across layers.
        r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"breach_alarm","active":true}"#,
        r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"","active":true}"#,
        r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::","active":true}"#,
        // Absolute state only: a toggle would depend on arrival order.
        r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::a"}"#,
        r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::a","active":"yes"}"#,
        // A second target is a wider mutation surface, not a Pause.
        r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::a","active":true,"ship":"player-1"}"#,
        r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused"}"#,
    ] {
        assert!(
            super::decode_gm_action_request(refused).is_none(),
            "must fail closed: {refused}"
        );
    }
}

/// A refused Pause replicates as a refused PAUSE (issue #1303): the lever
/// crosses the owner-decision lane beside the event id, because a frame
/// carrying only the id could be republished on every other GM's feed as a
/// refused Fire of that same event.
#[test]
fn a_replicated_event_control_refusal_carries_its_lever() {
    let refusal = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(
        crate::gm_action::GmActionRefusal {
            sequenced_by: HostSlot(1),
            requester: HostSlot(2),
            operator_id: "gm-1".into(),
            correlation: crate::gm_action::GmActionId::new("pause-1").unwrap(),
            action_kind: crate::gm_action::GmActionKind::EventControl,
            requested_active: false,
            tick: 419,
            reason: crate::gm_action::GmActionRefusalReason::UnknownGmEvent,
            target: Some("base-world::breach_alarm".into()),
            verb: Some(crate::gm_action::GmEventVerb::Pause),
            lever: None,
            effect_scope: None,
            objective_verb: None,
            objective_instance_scope: None,
            objective_recipients: None,
            comms_recipients: None,
            observer: None,
            npc_doctrine: None,
        },
    ));
    let text = super::encode_mesh_frame(&refusal).expect("encodes");
    assert!(text.contains(r#""verb":"pause""#), "{text}");
    assert_eq!(super::decode_mesh_frame(&text), Some(refusal));

    // The same frame from a peer that predates the lever: the key is simply
    // absent, and it still decodes rather than being dropped on the floor.
    // `validate_fleet_frame` is what then refuses it, rather than this
    // ingress guessing Fire.
    let legacy = text.replace(r#","verb":"pause""#, "");
    let legacy_value: serde_json::Value = serde_json::from_str(&legacy).unwrap();
    assert!(legacy_value["d"].get("verb").is_none(), "{legacy}");
    let Some(MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(decoded))) =
        super::decode_mesh_frame(&legacy)
    else {
        panic!("a legacy refusal still decodes")
    };
    assert_eq!(decoded.verb, None);
    assert_eq!(decoded.target.as_deref(), Some("base-world::breach_alarm"));
}

/// No session token can reach this wire, because the type it projects from
/// carries none. Asserted at the encoding site because this is the moment
/// the frame becomes bytes on a socket.
#[test]
fn the_wire_carries_no_session_token() {
    let text = super::encode_mesh_frame(&tick_frame()).expect("encodes");
    assert!(!text.contains("response_token"), "{text}");
    assert!(!text.contains("token"), "{text}");
}

/// The direct-effect ingress (issue #1310): one entity identity, one kind,
/// one positive integer amount in milli-HP, and a per-shape field count so
/// nothing wider can ride in behind it.
#[test]
fn a_direct_effect_uses_the_same_exact_typed_ingress() {
    let request = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"hit-4","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":25000,"scope":"entity","scope_id":null}"#,
        )
        .expect("valid direct-effect request");
    assert_eq!(
        request.action,
        crate::gm_action::GmAction::ApplyDirectEffect {
            target: "npc-1".into(),
            scope: crate::gm_effect::GmDirectEffectScope::Entity,
            effect: crate::gm_effect::GmDirectEffectKind::Damage,
            amount_milli_hp: 25_000,
        }
    );
    assert_eq!(
        request.action.kind(),
        crate::gm_action::GmActionKind::DirectEffect
    );
    assert_eq!(
        request.action.target_id(),
        Some("npc-1"),
        "the durable result must be able to say WHAT was hit"
    );

    let heal = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"heal-4","action":"apply_direct_effect","entity":"npc-1","effect":"heal","amount_milli_hp":1,"scope":"entity","scope_id":null}"#,
        )
        .expect("valid heal request");
    assert!(matches!(
        heal.action,
        crate::gm_action::GmAction::ApplyDirectEffect {
            effect: crate::gm_effect::GmDirectEffectKind::Heal,
            ..
        }
    ));

    // The narrowed scopes (issue #1311) arrive through the SAME arm, as a
    // discriminant plus one ship-local authoring key. Nothing else about
    // the request changes shape, which is the point of scope being a field
    // of one mechanic rather than a second action.
    let station = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"hit-5","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":25000,"scope":"station","scope_id":"helm"}"#,
        )
        .expect("valid Station-scoped request");
    assert_eq!(
        station.action,
        crate::gm_action::GmAction::ApplyDirectEffect {
            target: "npc-1".into(),
            scope: crate::gm_effect::GmDirectEffectScope::Station(
                crate::core::messages::StationId("helm".into())
            ),
            effect: crate::gm_effect::GmDirectEffectKind::Damage,
            amount_milli_hp: 25_000,
        }
    );
    let system = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"heal-5","action":"apply_direct_effect","entity":"npc-1","effect":"heal","amount_milli_hp":4000,"scope":"system","scope_id":"impulse-drive"}"#,
        )
        .expect("valid System-scoped request");
    assert_eq!(
        system.action,
        crate::gm_action::GmAction::ApplyDirectEffect {
            target: "npc-1".into(),
            scope: crate::gm_effect::GmDirectEffectScope::System(crate::core::messages::SystemId(
                "impulse-drive".into()
            )),
            effect: crate::gm_effect::GmDirectEffectKind::Heal,
            amount_milli_hp: 4_000,
        }
    );

    for refused in [
        // A signed amount is not the vocabulary: the sign is the kind.
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":-5,"scope":"entity","scope_id":null}"#,
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":0,"scope":"entity","scope_id":null}"#,
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":2.5,"scope":"entity","scope_id":null}"#,
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"","effect":"damage","amount_milli_hp":5,"scope":"entity","scope_id":null}"#,
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"vaporise","amount_milli_hp":5,"scope":"entity","scope_id":null}"#,
        // A scope this build does not implement cannot be named from here.
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"deck","scope_id":"a"}"#,
        // A narrowed scope with no id names nothing, and a whole-entity one
        // carrying an id is a request the ingress would have to reinterpret.
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"station","scope_id":null}"#,
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"system","scope_id":""}"#,
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"entity","scope_id":"helm"}"#,
        // The field-count guard stays exact in both directions.
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5}"#,
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"entity","scope_id":null,"station":"helm"}"#,
        r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage"}"#,
    ] {
        assert!(
            super::decode_gm_action_request(refused).is_none(),
            "must fail closed: {refused}"
        );
    }
}
