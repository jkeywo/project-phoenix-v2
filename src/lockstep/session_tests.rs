use super::*;

const DELAY: u64 = 6;

fn session() -> LockstepSession {
    LockstepSession::new(HostSlot(1), [HostSlot(1), HostSlot(2)], DELAY)
}

/// The opening window: a fleet that has never spoken can still run the
/// first `delay + 1` ticks, because a host that has run no ticks has by
/// definition already issued everything it will ever issue for them.
///
/// Without this the fleet deadlocks on tick zero — every host waiting for a
/// frame no host can send until it has stepped.
#[test]
fn the_opening_window_needs_no_frames() {
    let session = session();
    for tick in 0..=DELAY {
        assert!(session.may_simulate(tick), "tick {tick} must run unblocked");
    }
    assert!(
        !session.may_simulate(DELAY + 1),
        "and the window has to END, or the barrier never barriers"
    );
}

/// A peer's watermark opens exactly as many further ticks as it declares.
#[test]
fn a_peers_watermark_opens_the_ticks_it_names() {
    let mut session = session();
    session.observe(HostSlot(2), 9);
    assert!(session.may_simulate(9));
    assert!(!session.may_simulate(10));
}

/// Watermarks only move forward, so a duplicated or reordered frame is
/// inert rather than a step backwards.
#[test]
fn a_late_or_repeated_frame_never_walks_a_watermark_back() {
    let mut session = session();
    session.observe(HostSlot(2), 20);
    session.observe(HostSlot(2), 11);
    session.observe(HostSlot(2), 20);
    assert!(
        session.may_simulate(20),
        "an out-of-order frame must not un-declare input the fleet already \
             heard about — reordered and duplicate observations have to converge"
    );
}

/// A host never waits for itself, whatever it is told about its own slot.
#[test]
fn a_host_does_not_wait_for_itself() {
    let mut session = LockstepSession::new(HostSlot(1), [HostSlot(1)], DELAY);
    assert!(session.is_alone());
    session.observe(HostSlot(1), 0);
    assert!(
        session.may_simulate(u64::MAX),
        "a fleet of one must run exactly as an unfleeted host does"
    );
    assert!(session.stall_at(u64::MAX).is_none());
}

/// The stall names the tick and every peer holding it up, with how far each
/// has actually got — which is the difference between "we are stuck" and a
/// diagnostic somebody can act on.
#[test]
fn a_stall_names_the_tick_and_the_peers_holding_it() {
    let mut session = LockstepSession::new(HostSlot(1), [HostSlot(2), HostSlot(3)], DELAY);
    session.observe(HostSlot(2), 30);
    session.observe(HostSlot(3), 12);

    assert!(session.stall_at(12).is_none());
    let stall = session.stall_at(13).expect("slot 3 is behind");
    assert_eq!(stall.waiting_on, vec![(HostSlot(3), 12)]);
    assert!(
        stall.to_string().contains("slot-3"),
        "the diagnostic must name the peer: {stall}"
    );

    let stall = session.stall_at(31).expect("now both are behind");
    assert_eq!(
        stall.waiting_on,
        vec![(HostSlot(2), 30), (HostSlot(3), 12)],
        "sorted by slot, so two hosts report the same stall identically"
    );
}

/// A departed peer stops being one this host waits for, so the barrier that
/// was withholding the tick past its watermark runs again — and a repeated
/// or reordered departure report is inert rather than a resurrection.
#[test]
fn a_departed_peer_is_no_longer_waited_for() {
    let mut session = LockstepSession::new(HostSlot(1), [HostSlot(2), HostSlot(3)], DELAY);
    session.observe(HostSlot(2), 30);
    session.observe(HostSlot(3), 12);

    // The lost host's last watermark is the observation the disconnect tick
    // is derived from — read the same on every survivor.
    assert_eq!(session.watermark_of(HostSlot(3)), Some(12));
    // Stalled at tick 13 waiting on slot 3.
    assert!(session.stall_at(13).is_some());

    session.depart(HostSlot(3));
    assert_eq!(session.watermark_of(HostSlot(3)), None);
    assert!(
        session.stall_at(13).is_none(),
        "once slot 3 has departed, the fleet no longer waits for the ticks \
             it never covered"
    );
    // Slot 2 is still a peer, so the fleet is not alone and still stalls for
    // it beyond its own watermark.
    assert!(!session.is_alone());
    assert!(session.stall_at(31).is_some());

    // A duplicate departure, and a `TickFrame` from the lost host that was
    // in flight when it closed, both change nothing: a departed peer stays
    // departed rather than being resurrected as something to wait for.
    session.depart(HostSlot(3));
    session.observe(HostSlot(3), 99);
    assert!(session.has_departed(HostSlot(3)));
    assert_eq!(
        session.watermark_of(HostSlot(3)),
        None,
        "a late frame from a departed host must not re-insert it — that \
             would re-stall the fleet on a peer that will never speak again"
    );
    assert!(
        session.stall_at(31).is_some(),
        "…but slot 2 still holds tick 31"
    );
}

/// A departed slot a replacement reclaims (issue #1120) is re-admitted to the
/// wait-set at the recovery watermark, so the barrier waits for it again and
/// its watermark advances once more — the inverse of `depart`.
#[test]
fn a_reclaimed_slot_is_waited_for_again_from_the_recovery_watermark() {
    let mut session = LockstepSession::new(HostSlot(1), [HostSlot(2), HostSlot(3)], DELAY);
    // Slot 2 is kept well ahead throughout, so the barrier below is a test of
    // slot 3's re-admission alone rather than of the other peer.
    session.observe(HostSlot(2), u64::MAX);
    session.observe(HostSlot(3), 12);
    session.depart(HostSlot(3));
    assert!(session.has_departed(HostSlot(3)));
    // A departed slot's frames are ignored — the barrier does not wait for it.
    session.observe(HostSlot(3), 50);
    assert_eq!(session.watermark_of(HostSlot(3)), None);

    // Reclaimed at the boundary: waited for again from there.
    session.rejoin(HostSlot(3), 100);
    assert!(!session.has_departed(HostSlot(3)));
    assert_eq!(session.watermark_of(HostSlot(3)), Some(100));
    assert!(
        session.may_simulate(100) && !session.may_simulate(101),
        "the fleet runs up to the recovery boundary but no further until the \
             replacement's genuine post-restore frames advance it"
    );
    // …and now its observations advance the watermark once more.
    session.observe(HostSlot(3), 106);
    assert_eq!(session.watermark_of(HostSlot(3)), Some(106));
}

/// The watermark a host declares is its own clock plus the agreed delay —
/// the one arithmetic the whole scheme rests on.
#[test]
fn the_declared_watermark_is_the_clock_plus_the_delay() {
    let session = session();
    assert_eq!(session.ready_through(0), DELAY);
    assert_eq!(session.ready_through(100), 100 + DELAY);
    assert_eq!(
        session.ready_through(u64::MAX),
        u64::MAX,
        "saturating, because a wrapped watermark would open every tick at once"
    );
}
