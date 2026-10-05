use super::*;
use crate::command_admission::log::HostSlot;
use crate::core::messages::StationId;
use crate::gm_checkpoint::SeatedShip;
use crate::lockstep::{FleetRoster, FleetShip};

fn seating() -> LiveSeating {
    LiveSeating {
        scenario: "assets/worlds/duel.toml".to_string(),
        ships: vec![SeatedShip {
            slot: 0,
            hull: Some("assets/entities/alliance_cruiser.toml".to_string()),
            stations: vec!["helm".to_string()],
        }],
    }
}

const LOCAL: HostSlot = HostSlot(1);
const PEER: HostSlot = HostSlot(2);
const ELSEWHERE: HostSlot = HostSlot(3);

/// A peer holding `participants`, sitting at `LOCAL`.
fn with_peers(participants: &[HostSlot]) -> GmLiveRestore {
    let mut restore = GmLiveRestore::default();
    restore.context.simulation_peers = participants.len().max(1);
    restore.context.live = Some(seating());
    restore.context.local = LOCAL;
    restore.context.participants = participants.to_vec();
    restore
}

fn request(initiator: HostSlot) -> AcceptedRestore {
    AcceptedRestore {
        operator_id: "gm-1".to_string(),
        correlation: "one".to_string(),
        candidate_slot: "slot-a".to_string(),
        requested_tick: 10,
        initiator,
        order: GmActionOrder::new(initiator, 1),
    }
}

/// Phone consoles are legs on a peer, not participants, so a peer holding a
/// whole crew of them is one simulation peer and is never waited on twice.
#[test]
fn many_phone_consoles_on_one_peer_are_still_one_simulation_peer() {
    let roster = FleetRoster::new(
        vec![FleetShip {
            host: HostSlot::SOLO,
            ship_path: Some("assets/entities/alliance_cruiser.toml".to_string()),
            authored_slot_id: None,
            crew: vec![
                (StationId("helm".to_string()), "commander".to_string()),
                (StationId("tactical".to_string()), "commander".to_string()),
                (
                    StationId("engineering".to_string()),
                    "commander".to_string(),
                ),
            ],
        }],
        HostSlot::SOLO,
    );
    assert_eq!(roster.participants().len(), 1);
    let live = LiveSeating::from_roster("assets/worlds/duel.toml", &roster, None);
    assert_eq!(live.ships.len(), 1);
    assert_eq!(live.ships[0].stations.len(), 3);
}

/// A restore in flight is exactly what the owner's sequencing gate refuses
/// on, and a settled one is exactly what it does not: the two states the
/// sequencer reads have to be distinguishable from the phase alone.
#[test]
fn every_working_phase_is_in_flight_and_every_reported_one_is_not() {
    for phase in [
        GmRestorePhase::Accepted,
        GmRestorePhase::CapturingRecovery,
        GmRestorePhase::AwaitingReadiness,
        GmRestorePhase::Loading,
        GmRestorePhase::AwaitingAgreement,
    ] {
        assert!(phase.in_flight(), "{phase:?} is work in progress");
        assert!(phase.holds_world());
    }
    for phase in [
        GmRestorePhase::Restored,
        GmRestorePhase::RolledBack,
        GmRestorePhase::Failed,
    ] {
        assert!(!phase.in_flight(), "{phase:?} is a reported outcome");
        assert!(
            phase.holds_world(),
            "{phase:?} still leaves the world stopped",
        );
    }
    assert!(!GmRestorePhase::Idle.in_flight());
    assert!(!GmRestorePhase::Idle.holds_world());
}

/// Which peers the room is waiting for, and which it has stopped waiting
/// for, are the two facts the desk draws. Only the coordinating peer waits:
/// a peer that merely answered is not waiting on anybody.
#[test]
fn the_coordinating_peer_names_the_peers_it_is_waiting_for() {
    let mut restore = with_peers(&[LOCAL, PEER, ELSEWHERE]);
    restore.accept(request(LOCAL));
    assert!(restore.coordinating());
    restore.settle(GmRestorePhase::AwaitingReadiness, None, 10);
    assert!(restore.waiting_on(PEER));
    assert!(restore.waiting_on(ELSEWHERE));
    assert!(!restore.waiting_on(LOCAL), "a peer never waits for itself");
    assert_eq!(restore.waiting_peers(), 2);

    restore.ready.insert(PEER);
    assert!(!restore.waiting_on(PEER));
    assert_eq!(restore.waiting_peers(), 1);

    restore.excluded.insert(ELSEWHERE);
    assert!(!restore.waiting_on(ELSEWHERE));
    assert!(restore.excluded(ELSEWHERE));
    assert_eq!(restore.waiting_peers(), 0);
    assert_eq!(restore.excluded_peers(), 1);
    assert_eq!(restore.expected_peers(), vec![PEER]);
}

/// A peer that is not coordinating answers and gets on with it; it must
/// never draw a countdown of its own over the room.
#[test]
fn a_peer_that_is_not_coordinating_waits_on_nobody() {
    let mut restore = with_peers(&[LOCAL, PEER]);
    restore.accept(request(PEER));
    assert!(!restore.coordinating());
    restore.settle(GmRestorePhase::Loading, None, 10);
    assert!(!restore.waiting_on(PEER));
    assert_eq!(restore.waiting_peers(), 0);
}

/// The countdown is REAL seconds and it runs out. A held world spends no
/// ticks, so this is the only clock that can end a wait at all.
#[test]
fn the_readiness_window_is_ten_real_seconds_and_expires() {
    let mut restore = with_peers(&[LOCAL, PEER]);
    restore.accept(request(LOCAL));
    restore.settle(GmRestorePhase::AwaitingReadiness, None, 10);
    restore.open_window();
    assert_eq!(restore.remaining_seconds(), Some(10));
    assert!(!restore.window_expired());

    restore.context.now += 4.5;
    assert_eq!(
        restore.remaining_seconds(),
        Some(6),
        "a countdown rounds UP, so the last whole second is shown as one",
    );
    assert!(!restore.window_expired());

    restore.context.now += PEER_WINDOW_SECONDS;
    assert!(restore.window_expired());
    assert_eq!(restore.remaining_seconds(), Some(0));

    // A reported outcome is not a wait, whatever the clock says.
    restore.settle(GmRestorePhase::Restored, None, 10);
    assert_eq!(restore.remaining_seconds(), None);
}

/// A frame naming a restore this peer is not running is dropped rather than
/// folded into the current one - the whole point of carrying the canonical
/// order on every frame.
#[test]
fn a_frame_names_the_restore_it_belongs_to() {
    let order = GmActionOrder::new(LOCAL, 7);
    let frame = GmRestoreFrame::Loaded {
        from: PEER,
        restore: order,
        tick: 40,
        digest: 0xfeed,
    };
    assert_eq!(frame.from(), PEER);
    assert_eq!(frame.restore(), order);
    assert_ne!(frame.restore(), GmActionOrder::new(LOCAL, 8));
}

#[test]
fn revalidation_reports_the_blocks_the_picker_would_have_shown() {
    let entry = SaveSlotEntry {
        slot_id: "slot-a".to_string(),
        display_name: "Before the ambush".to_string(),
        kind: crate::save_slots::SaveSlotKind::Manual,
        record: None,
        start: crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Empty),
        metadata: crate::save_slots::MetadataStatus::Present,
    };
    let failure = revalidate(&seating(), Some(&entry)).expect_err("an empty row is no candidate");
    assert_eq!(
        failure,
        GmRestoreFailure::CandidateIneligible {
            blocks: vec![CandidateBlock::Unreadable],
        },
    );
    assert_eq!(failure.label_id(), "server.gm.restore.failed.ineligible");
}

#[test]
fn an_absent_candidate_is_refused_rather_than_treated_as_eligible() {
    assert!(revalidate(&seating(), None).is_err());
}

/// Every phase except Idle holds the world, and only the three working ones
/// refuse a concurrent request. A reported outcome must not hold the GM's
/// own resume hostage.
#[test]
fn phase_wire_spellings_are_stable_and_distinct() {
    let phases = [
        GmRestorePhase::Idle,
        GmRestorePhase::Accepted,
        GmRestorePhase::CapturingRecovery,
        GmRestorePhase::Loading,
        GmRestorePhase::Restored,
        GmRestorePhase::RolledBack,
        GmRestorePhase::Failed,
    ];
    let wires: std::collections::BTreeSet<_> = phases.iter().map(|phase| phase.as_wire()).collect();
    assert_eq!(wires.len(), phases.len());
    assert!(GmRestorePhase::Loading.holds_session());
    assert!(!GmRestorePhase::Idle.holds_session());
    // A reported restore no longer refuses a resume — that IS the resume —
    // but the world it reported on is still stopped, and the sequencer has
    // to schedule that resume at the boundary the hold stopped.
    assert!(!GmRestorePhase::Restored.holds_session());
    for phase in phases {
        assert_eq!(phase.holds_world(), phase != GmRestorePhase::Idle);
    }
}
