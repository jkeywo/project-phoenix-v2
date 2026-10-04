use super::*;

fn command(origin: u32, seq: u64) -> MeshCommand {
    MeshCommand {
        tick: 12,
        order: CommandOrder::new(HostSlot(origin), seq),
        ship: ShipKey("uuid-ship".into()),
        target: SystemId("helm".into()),
        payload: SystemControlPayload::SetRedAlert { active: true },
    }
}

/// The one rule a receiver can enforce about a peer's traffic: a host may
/// speak only for its own slot.
#[test]
fn a_command_belongs_to_the_slot_that_ordered_it() {
    let cmd = command(2, 0);
    assert!(cmd.is_from(HostSlot(2)));
    assert!(
        !cmd.is_from(HostSlot(3)),
        "a frame from slot 3 carrying slot 2's order is one host speaking \
             for another, and the receiver must be able to say so"
    );
}

/// The frames round-trip through the serde shape the codec and the RON
/// diagnostics both use. Asserted here rather than in the codec because the
/// property belongs to the vocabulary: a frame that cannot survive being
/// written down cannot be replayed by #1118 either.
#[test]
fn every_frame_round_trips() {
    let frames = vec![
        MeshFrame::Tick(TickFrame {
            from: HostSlot(1),
            tick: 10,
            ready_through: 16,
            commands: vec![command(1, 0), command(1, 1)],
            start_grant: None,
        }),
        MeshFrame::Digest(DigestFrame {
            from: HostSlot(2),
            tick: 400,
            digest: 0xdead_beef,
        }),
        MeshFrame::Snapshot(SnapshotChunk {
            from: HostSlot(2),
            transfer_id: 0x1117,
            tick: 400,
            seq: 1,
            total: 4,
            whole_hash: 0xfeed_face,
            crc: 0xabad_1dea,
            text: "portable-record-slice".to_string(),
        }),
        MeshFrame::HostLoss(HostLossFrame {
            from: HostSlot(1),
            lost: HostSlot(3),
            tick: 418,
        }),
        MeshFrame::SlotClaim(SlotClaimFrame {
            from: HostSlot(1),
            slot: HostSlot(3),
            claim_seq: 7,
            tick: 512,
        }),
        // The multi-peer live-restore lane (issue #1447). Included here for
        // the same reason every other frame is: a frame that cannot survive
        // being written down cannot cross a mesh either.
        MeshFrame::GmRestore(crate::gm_restore::GmRestoreFrame::Settle {
            from: HostSlot(1),
            restore: crate::gm_action::GmActionOrder::new(HostSlot(3), 4),
            commit: false,
            failure: Some(crate::gm_restore::GmRestoreFailure::PeerDigestMismatch { peers: 1 }),
        }),
        MeshFrame::GmRestore(crate::gm_restore::GmRestoreFrame::Loaded {
            from: HostSlot(2),
            restore: crate::gm_action::GmActionOrder::new(HostSlot(3), 4),
            tick: 512,
            digest: 0x0bad_c0de,
        }),
        MeshFrame::GmAction(crate::gm_action::GmActionFrame::Granted(
            crate::gm_action::GmActionGrant {
                from: HostSlot(2),
                sequenced_by: HostSlot(1),
                operator_id: "gm-1".into(),
                correlation: crate::gm_action::GmActionId::new("pause-1").unwrap(),
                recovery_generation: 0,
                apply_tick: 513,
                order: crate::gm_action::GmActionOrder::new(HostSlot(2), 1),
                action: crate::gm_action::GmAction::SetSessionPaused { active: true },
            },
        )),
    ];
    for frame in frames {
        let text = ron::ser::to_string(&frame).expect("a frame serialises");
        let back: MeshFrame = ron::from_str(&text).expect("and comes back");
        assert_eq!(back, frame);
        assert!(
            !text.contains("response_token"),
            "a mesh frame must never carry a session token:\n{text}"
        );
    }
}

/// The two halves of the host mesh agree on the revision they speak.
///
/// Pinned as a value rather than compared to the JS constant, because the
/// JS half is not compiled here — the Vitest suite asserts the same number
/// from its side, and the pair of pins is what catches a one-sided bump.
#[test]
fn the_protocol_revision_is_pinned() {
    assert_eq!(
        HOST_MESH_PROTOCOL, 16,
        "bumping this is a fleet-wide incompatible change: gui/host-mesh.js \
             refuses a frame whose `m` it does not know, so both halves and the \
             Vitest pin move together or a mixed fleet fails to agree a tick"
    );
}
