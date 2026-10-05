use super::*;

/// A ledger sampling on `interval` with the given `(tick, digest)` samples.
fn ledger(interval: u64, samples: &[(u64, u64)]) -> DigestLedger {
    let mut l = DigestLedger::new(interval);
    for &(tick, digest) in samples {
        l.record(tick, digest);
    }
    l
}

const I: u64 = 10;

/// Build the `(local_ledger, peers)` view a given host would hold, from the
/// whole fleet's per-tick folds. This is the shared state the exchange
/// delivers to everyone; the point of splitting it per host is to prove every
/// host derives the SAME plan from its own copy of it.
fn view(
    fleet: &[(HostSlot, Vec<(u64, u64)>)],
    local: HostSlot,
) -> (DigestLedger, BTreeMap<HostSlot, DigestLedger>) {
    let mut local_ledger = DigestLedger::new(I);
    let mut peers = BTreeMap::new();
    for (slot, samples) in fleet {
        if *slot == local {
            local_ledger = ledger(I, samples);
        } else {
            peers.insert(*slot, ledger(I, samples));
        }
    }
    (local_ledger, peers)
}

/// **AC2.** Three hosts, one diverged: the two that agree are the majority,
/// and the leader is the lowest of them.
#[test]
fn a_three_host_fleet_elects_the_lowest_majority_slot() {
    let mut d = BTreeMap::new();
    d.insert(HostSlot(1), 0xAA);
    d.insert(HostSlot(2), 0xAA);
    d.insert(HostSlot(3), 0xBB);
    let e = elect(&d).expect("a strict majority exists");
    assert_eq!(e.canonical_digest, 0xAA);
    assert_eq!(e.leader, HostSlot(1));
    assert_eq!(e.recovering, vec![HostSlot(3)]);
}

/// The leader is the lowest of the MAJORITY, not the lowest slot overall — a
/// divergent slot 1 does not lead its own recovery.
#[test]
fn the_leader_is_the_lowest_of_the_majority_not_of_the_fleet() {
    let mut d = BTreeMap::new();
    d.insert(HostSlot(1), 0xBB); // the diverged one
    d.insert(HostSlot(2), 0xAA);
    d.insert(HostSlot(3), 0xAA);
    let e = elect(&d).expect("a strict majority exists");
    assert_eq!(e.leader, HostSlot(2), "slot 1 diverged, so it cannot lead");
    assert_eq!(e.recovering, vec![HostSlot(1)]);
}

/// **AC6.** Two hosts that disagree have no majority and therefore no safe
/// leader: recovery refuses rather than overwrite one from the other.
#[test]
fn a_two_host_split_has_no_safe_leader() {
    let mut d = BTreeMap::new();
    d.insert(HostSlot(1), 0xAA);
    d.insert(HostSlot(2), 0xBB);
    assert_eq!(
        elect(&d),
        Err(NoRecovery::NoSafeLeader {
            largest_group: 1,
            fleet: 2
        })
    );
}

/// **AC6.** An even four-host partition is a coin-flip, so it is refused too —
/// a bare half is not a strict majority.
#[test]
fn an_even_partition_has_no_safe_leader() {
    let mut d = BTreeMap::new();
    d.insert(HostSlot(1), 0xAA);
    d.insert(HostSlot(2), 0xAA);
    d.insert(HostSlot(3), 0xBB);
    d.insert(HostSlot(4), 0xBB);
    assert!(matches!(
        elect(&d),
        Err(NoRecovery::NoSafeLeader {
            largest_group: 2,
            fleet: 4
        })
    ));
}

/// A five-host three-two split recovers the minority two behind the lowest of
/// the three.
#[test]
fn a_five_host_three_two_split_recovers_the_minority() {
    let mut d = BTreeMap::new();
    d.insert(HostSlot(1), 0xAA);
    d.insert(HostSlot(2), 0xBB);
    d.insert(HostSlot(3), 0xAA);
    d.insert(HostSlot(4), 0xBB);
    d.insert(HostSlot(5), 0xAA);
    let e = elect(&d).expect("three of five is a strict majority");
    assert_eq!(e.canonical_digest, 0xAA);
    assert_eq!(e.leader, HostSlot(1));
    assert_eq!(e.recovering, vec![HostSlot(2), HostSlot(4)]);
}

/// **AC2, the determinism crown.** Every host in the fleet computes the
/// byte-identical plan from its own copy of the shared exchange — the leader,
/// the recovering set, the canonical fold, the divergence tick and the
/// boundary are all host-independent.
///
/// Revert-verify: make [`elect`] pick the leader by anything other than the
/// shared fold (say, the local slot when it is canonical, i.e. "who noticed")
/// and the three plans below stop being equal.
#[test]
fn every_host_computes_the_identical_plan() {
    // Agree at tick 10, diverge at tick 20 with slot 3 the odd one out.
    let fleet = vec![
        (HostSlot(1), vec![(10, 0xAA), (20, 0x11)]),
        (HostSlot(2), vec![(10, 0xAA), (20, 0x11)]),
        (HostSlot(3), vec![(10, 0xAA), (20, 0x99)]),
    ];
    let slots: Vec<HostSlot> = fleet.iter().map(|(s, _)| *s).collect();
    const DELAY: u64 = 2;

    let plans: Vec<RecoveryPlan> = slots
        .iter()
        .map(|&local| {
            let (l, peers) = view(&fleet, local);
            match decide(&slots, local, &l, &peers, I, DELAY) {
                RecoveryDecision::Recover(plan) => plan,
                other => panic!("host {local:?} did not decide to recover: {other:?}"),
            }
        })
        .collect();

    assert_eq!(plans[0], plans[1], "slots 1 and 2 disagree about the plan");
    assert_eq!(plans[1], plans[2], "slots 2 and 3 disagree about the plan");
    assert_eq!(plans[0].divergence_tick, 20);
    assert_eq!(plans[0].last_agreed_tick, Some(10));
    assert_eq!(plans[0].leader, HostSlot(1));
    assert_eq!(plans[0].recovering, vec![HostSlot(3)]);
    assert_eq!(plans[0].canonical_digest, 0x11);

    // …and each host reads its OWN role off that one shared plan.
    assert_eq!(plans[0].role_of(HostSlot(1)), RecoveryRole::Leader);
    assert_eq!(plans[0].role_of(HostSlot(2)), RecoveryRole::Bystander);
    assert_eq!(plans[0].role_of(HostSlot(3)), RecoveryRole::Recovering);
}

/// The divergence tick is the EARLIEST jointly-sampled disagreement, and the
/// last agreement before it is reported for the command window.
#[test]
fn the_divergence_tick_is_the_earliest_jointly_sampled_disagreement() {
    let fleet = vec![
        (HostSlot(1), vec![(10, 0xAA), (20, 0xAA), (30, 0xAA)]),
        (HostSlot(2), vec![(10, 0xAA), (20, 0xAA), (30, 0xAA)]),
        (HostSlot(3), vec![(10, 0xAA), (20, 0xBB), (30, 0xCC)]),
    ];
    let slots: Vec<HostSlot> = fleet.iter().map(|(s, _)| *s).collect();
    let (l, peers) = view(&fleet, HostSlot(1));
    let window = earliest_divergence(&slots, HostSlot(1), &l, &peers)
        .expect("the fleet diverged at tick 20");
    assert_eq!(window.tick, 20, "tick 30 also diverged, but 20 came first");
    assert_eq!(window.last_agreed, Some(10));
}

/// A divergence tick a peer has not sampled yet defers the decision — the
/// plan is never built on a partial fleet.
#[test]
fn a_missing_peer_sample_defers_the_decision() {
    // Slot 3 has not reported tick 20 yet.
    let fleet = vec![
        (HostSlot(1), vec![(10, 0xAA), (20, 0x11)]),
        (HostSlot(2), vec![(10, 0xAA), (20, 0x11)]),
        (HostSlot(3), vec![(10, 0xAA)]),
    ];
    let slots: Vec<HostSlot> = fleet.iter().map(|(s, _)| *s).collect();
    let (l, peers) = view(&fleet, HostSlot(1));
    assert_eq!(
        decide(&slots, HostSlot(1), &l, &peers, I, 2),
        RecoveryDecision::Pending,
        "a decision on a fleet one host has not been heard from is a decision \
             that could differ across hosts — it must wait"
    );
}

/// The boundary is past the detection horizon, at least an interval ahead,
/// and interval-aligned — the three properties every host relies on to pause
/// at the same tick.
#[test]
fn the_boundary_is_past_detection_interval_aligned_and_shared() {
    let interval = 10;
    let delay = 2;
    let b = recovery_boundary(20, interval, delay);
    assert!(
        b >= 20 + delay + 2,
        "the boundary must clear detection: {b}"
    );
    assert!(
        b >= 20 + interval,
        "…and be at least an interval ahead: {b}"
    );
    assert_eq!(b % interval, 0, "…and land on a sampling checkpoint: {b}");
    // A larger delay pushes the boundary out deterministically.
    assert!(recovery_boundary(20, interval, 25) > b);
}

/// A tick only one host sampled is not evidence of anything and is skipped —
/// the fleet is judged only where every slot has folded.
#[test]
fn a_tick_only_one_host_sampled_is_not_a_divergence() {
    let fleet = vec![
        (HostSlot(1), vec![(10, 0xAA), (20, 0x11)]),
        (HostSlot(2), vec![(10, 0xAA)]),
    ];
    let slots: Vec<HostSlot> = fleet.iter().map(|(s, _)| *s).collect();
    let (l, peers) = view(&fleet, HostSlot(1));
    // Slot 1 sampled tick 20; slot 2 has not. That is not a disagreement.
    assert!(earliest_divergence(&slots, HostSlot(1), &l, &peers).is_none());
}
