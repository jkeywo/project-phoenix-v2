use super::*;
use crate::lockstep::{FleetRoster, FleetShip};

fn roster(slots: &[HostSlot], local: HostSlot) -> FleetRoster {
    FleetRoster::new(
        slots.iter().map(|&host| FleetShip::new(host)).collect(),
        local,
    )
}

/// **AC2, the determinism crown for the claim race.** Two machines claim the
/// same vacant slot; the winner is the LOWEST owner-minted `claim_seq`, and it
/// is the same whatever order the two claims are observed in — so every host
/// that hears both agrees the one winner from the shared value, never from
/// who-processed-first locally.
#[test]
fn the_lowest_claim_seq_wins_regardless_of_arrival_order() {
    // Host A hears claim seq 5 then seq 9.
    let mut a = PendingSlotClaims::default();
    a.observe(HostSlot(3), 5, 100);
    a.observe(HostSlot(3), 9, 101);

    // Host B hears them in the OPPOSITE order.
    let mut b = PendingSlotClaims::default();
    b.observe(HostSlot(3), 9, 101);
    b.observe(HostSlot(3), 5, 100);

    assert_eq!(a.winning_seq(HostSlot(3)), Some(5));
    assert_eq!(
        a.winning_seq(HostSlot(3)),
        b.winning_seq(HostSlot(3)),
        "two hosts that heard the same two claims in opposite orders must elect \
             the SAME winner — the resolution is a function of the seq VALUE, not \
             of arrival"
    );
    assert!(a.wins(HostSlot(3), 5) && !a.wins(HostSlot(3), 9));
    // The winning claim's tick travels with it, so the boundary is shared.
    assert_eq!(a.winners.get(&HostSlot(3)).map(|(_, t)| *t), Some(100));
}

/// The leader is the lowest roster slot that is not the one being recovered —
/// in practice the owner, unless the owner itself is recovered.
#[test]
fn the_leader_is_the_lowest_slot_that_is_not_recovering() {
    let roster = roster(&[HostSlot(1), HostSlot(2), HostSlot(3)], HostSlot(1));
    assert_eq!(leader_for(&roster, HostSlot(3)), HostSlot(1));
    assert_eq!(leader_for(&roster, HostSlot(2)), HostSlot(1));
    assert_eq!(
        leader_for(&roster, HostSlot(1)),
        HostSlot(2),
        "recovering the owner falls to the next slot (host migration is out of \
             this issue's scope, but the leader math is still total)"
    );
}

/// A recovery is opened once per slot: a duplicate winning claim frame is inert.
#[test]
fn a_slot_recovery_opens_once() {
    let mut claims = PendingSlotClaims::default();
    claims.observe(HostSlot(3), 5, 100);
    assert_eq!(claims.next_unopened(), Some((HostSlot(3), 5, 100)));
    claims.mark_opened(HostSlot(3));
    assert!(claims.is_opened(HostSlot(3)));
    // A later duplicate does not re-open it.
    claims.observe(HostSlot(3), 5, 100);
    assert_eq!(claims.next_unopened(), None);
}
