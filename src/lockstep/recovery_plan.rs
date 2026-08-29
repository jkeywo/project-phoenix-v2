//! The pure divergence-recovery decision (issue #1118).
//!
//! Bevy-free, so the whole of "who recovers, from whom, and when" is decided in
//! one tested place rather than inside a system that also has to own a clock and
//! a snapshot walk. The Bevy adapter that drives it is [`crate::lockstep::recovery`]
//! (AGENTS.md rule 10: the decision module stays Bevy-free and its adapter is a
//! sibling), exactly as [`crate::lockstep::session`] is the pure barrier and
//! [`crate::lockstep::mod`] is its adapter.
//!
//! # Determinism is the whole point (and the rule a sibling issue failed twice)
//!
//! Every value this module produces — the divergence tick, the recovery leader,
//! the recovery-boundary tick — is a **deterministic function of the periodic
//! digest exchange** and of the two numbers the fleet agreed when it joined
//! (`command_delay_ticks` and the digest interval). Nothing here reads a
//! local-only, arrival-ordered, skew-prone value. A raw receive-watermark
//! differs across honest hosts at the same instant, so a decision gated on
//! "who noticed the divergence first" is exactly the bug that must not appear:
//!
//! * **The divergence tick** ([`earliest_divergence`]) is the earliest checkpoint
//!   tick the WHOLE fleet has sampled and NOT agreed on. Each fleet slot's fold
//!   at a checkpoint tick is a fixed value it sampled and published; the exchange
//!   delivers every slot's fold for tick *T* to every host (the star relay
//!   forwards a sibling's frame verbatim, and the channel is reliable-ordered).
//!   So the set `{slot -> fold at T}` is globally well-defined, and every host
//!   that has received all of it computes the identical tick. The *instant* each
//!   host has it may differ; the tick it derives does not.
//!
//! * **The leader** ([`elect`]) is the lowest slot in the group whose fold
//!   commands a **strict majority** of the fleet at the divergence tick. Same
//!   input `{slot -> fold at T}` on every host, same election. A divergent peer
//!   cannot make itself leader by shouting first: leadership is a function of the
//!   fold VALUES, not of arrival order, and a peer in the minority is never
//!   eligible. When no fold commands a strict majority (a two-host split, an even
//!   partition) there is no safe leader and the decision is a clean failure — the
//!   fleet does not let a coin-flip half overwrite the other.
//!
//! * **The boundary tick** ([`recovery_boundary`]) is pure arithmetic on the
//!   divergence tick, the digest interval, and the agreed delay — all shared. It
//!   is placed far enough past the divergence tick that every host is guaranteed
//!   to have detected the split and set its boundary before it would run that
//!   tick (a peer's fold for tick *T* is in hand before any host runs tick
//!   *T + delay + 1* on a reliable-ordered channel), so every host pauses at the
//!   same tick.
//!
//! The barrier ([`crate::lockstep::session`]) is what makes the delivery claims
//! above true: a host may not simulate tick *S* until every peer is ready through
//! *S*, and a peer's readiness for *S* rides the same reliable-ordered channel,
//! after that peer's earlier digest frames. So "the fold for tick *T* has
//! arrived" is not hoped for — it is implied by the fleet having advanced.

use std::collections::{BTreeMap, BTreeSet};

use crate::command_admission::log::HostSlot;
use crate::sim_digest::DigestLedger;

/// This host's part in a recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryRole {
    /// Holds a canonical fold and transfers its record to the divergent host(s).
    Leader,
    /// Diverged from the majority; restores the leader's canonical record.
    Recovering,
    /// Canonical, but not the leader — waits at the boundary while the leader
    /// heals the fleet, then resumes.
    Bystander,
}

/// The fleet-wide fold disagreement a recovery answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivergenceWindow {
    /// The earliest checkpoint tick the whole fleet has sampled and NOT agreed on.
    pub tick: u64,
    /// The last checkpoint tick before it the whole fleet agreed on, if any — the
    /// near edge of the command window a diagnostic reads over.
    pub last_agreed: Option<u64>,
    /// Every fleet slot's fold at `tick`, in slot order.
    pub digests: BTreeMap<HostSlot, u64>,
}

/// Why a divergence cannot be safely recovered from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoRecovery {
    /// No fold commands a strict majority of the fleet — a two-host split, or an
    /// even partition. Overwriting either half from the other would be a guess,
    /// so the fleet refuses rather than pick one blindly (AC2/AC6).
    NoSafeLeader {
        /// The size of the largest agreeing group.
        largest_group: usize,
        /// How many slots the fleet has.
        fleet: usize,
    },
}

impl std::fmt::Display for NoRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoRecovery::NoSafeLeader {
                largest_group,
                fleet,
            } => write!(
                f,
                "no fold commands a strict majority: the largest agreeing group is \
                 {largest_group} of {fleet} hosts, so there is no canonical record \
                 to recover from without a coin-flip"
            ),
        }
    }
}

/// A deterministic recovery plan every host computes identically from the shared
/// digest exchange.
///
/// Host-independent by construction: `leader`, `recovering`, `canonical_digest`,
/// `divergence_tick` and `boundary_tick` are the same on every host. Which of the
/// three roles a given host plays is derived from the plan by [`Self::role_of`],
/// not baked into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryPlan {
    /// The earliest checkpoint tick the fleet disagreed on.
    pub divergence_tick: u64,
    /// The last checkpoint before it the fleet agreed on.
    pub last_agreed_tick: Option<u64>,
    /// The tick every host pauses at, the leader captures at, and the recovering
    /// host restores to.
    pub boundary_tick: u64,
    /// The fold the majority took at the divergence tick — the record every
    /// recovering host must restore to.
    pub canonical_digest: u64,
    /// The lowest slot in the majority: the one that transfers its record.
    pub leader: HostSlot,
    /// Every slot whose fold at the divergence tick was not canonical, in slot
    /// order — the hosts that restore.
    pub recovering: Vec<HostSlot>,
    /// Every fleet slot's fold at the divergence tick, in slot order.
    pub digests: BTreeMap<HostSlot, u64>,
}

impl RecoveryPlan {
    /// The part `slot` plays in this recovery.
    pub fn role_of(&self, slot: HostSlot) -> RecoveryRole {
        if slot == self.leader {
            RecoveryRole::Leader
        } else if self.recovering.contains(&slot) {
            RecoveryRole::Recovering
        } else {
            RecoveryRole::Bystander
        }
    }
}

/// The outcome of asking the digest exchange whether a recovery is due.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryDecision {
    /// The fleet still agrees on every jointly-sampled checkpoint, or a peer's
    /// fold for the divergent tick has not arrived yet. Ask again next frame.
    Pending,
    /// A recoverable divergence: this is the plan every host derives.
    Recover(RecoveryPlan),
    /// A divergence with no safe leader — the clean-failure path (AC6).
    Unrecoverable {
        window: DivergenceWindow,
        reason: NoRecovery,
    },
}

/// The election result: the canonical fold, the leader, and who must restore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Election {
    pub canonical_digest: u64,
    pub leader: HostSlot,
    pub recovering: Vec<HostSlot>,
}

/// The earliest checkpoint tick the whole fleet has sampled and disagrees on.
///
/// Walks this host's own checkpoints in tick order. For each, it gathers every
/// fleet slot's fold at that tick — its own from `local_ledger`, each peer's from
/// `peers`. The moment a slot's fold for a candidate tick is missing it stops:
/// on a reliable-ordered channel a peer's earlier sample always precedes its
/// later one, so a tick this host cannot yet complete for one peer is a tick no
/// later checkpoint can be complete for either. That is what makes the returned
/// tick a function of *arrived* shared state rather than of arrival timing —
/// [`RecoveryDecision::Pending`] until the shared view is complete, then the same
/// answer on every host.
///
/// `None` while every jointly-sampled checkpoint agrees, or while the fleet's
/// samples are still incomplete.
pub fn earliest_divergence(
    fleet: &[HostSlot],
    local: HostSlot,
    local_ledger: &DigestLedger,
    peers: &BTreeMap<HostSlot, DigestLedger>,
) -> Option<DivergenceWindow> {
    let mut last_agreed = None;
    for checkpoint in &local_ledger.checkpoints {
        let tick = checkpoint.tick;
        let mut digests = BTreeMap::new();
        let mut complete = true;
        for &slot in fleet {
            let fold = if slot == local {
                Some(checkpoint.digest)
            } else {
                peers.get(&slot).and_then(|ledger| ledger.digest_at(tick))
            };
            match fold {
                Some(value) => {
                    digests.insert(slot, value);
                }
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if !complete {
            // The shared view for this tick is incomplete, and every later
            // checkpoint's is too. Defer rather than judge on a partial fleet.
            break;
        }
        let distinct: BTreeSet<u64> = digests.values().copied().collect();
        if distinct.len() <= 1 {
            last_agreed = Some(tick);
        } else {
            return Some(DivergenceWindow {
                tick,
                last_agreed,
                digests,
            });
        }
    }
    None
}

/// Elect the recovery leader from each fleet slot's fold at the divergence tick,
/// or refuse when no fold commands a strict majority.
///
/// The canonical fold is the one held by the largest group; the leader is the
/// lowest slot in that group. A strict majority (more than half the fleet) is
/// required: when the largest group is a tie or a bare half, there is no
/// canonical record and the fleet must not overwrite one half from the other.
/// This is the same rule stated in the module docs, and it is deterministic — the
/// groups are keyed by fold value, so a size tie resolves to the lowest fold
/// value, and the winning group's leader is its lowest slot.
pub fn elect(digests: &BTreeMap<HostSlot, u64>) -> Result<Election, NoRecovery> {
    let fleet = digests.len();
    let mut groups: BTreeMap<u64, Vec<HostSlot>> = BTreeMap::new();
    for (&slot, &fold) in digests {
        groups.entry(fold).or_default().push(slot);
    }
    // Groups are visited in ascending fold order (a `BTreeMap`), and a group only
    // displaces the incumbent when it is STRICTLY larger — so a size tie keeps the
    // lower fold value, and the choice is deterministic on every host.
    let mut best: Option<(u64, Vec<HostSlot>)> = None;
    for (fold, members) in groups {
        let larger = best.as_ref().is_none_or(|(_, m)| members.len() > m.len());
        if larger {
            best = Some((fold, members));
        }
    }
    let (canonical_digest, members) = best.expect("the fold set is never empty");
    let largest = members.len();
    // Strict majority: `largest > fleet / 2`, written to avoid integer division
    // rounding a bare half up to a win.
    if largest * 2 <= fleet {
        return Err(NoRecovery::NoSafeLeader {
            largest_group: largest,
            fleet,
        });
    }
    let leader = members
        .iter()
        .copied()
        .min()
        .expect("a majority group is non-empty");
    let recovering: Vec<HostSlot> = digests
        .iter()
        .filter(|(_, &fold)| fold != canonical_digest)
        .map(|(&slot, _)| slot)
        .collect();
    Ok(Election {
        canonical_digest,
        leader,
        recovering,
    })
}

/// The tick every host pauses at, the leader captures at, and the recovering
/// host restores to.
///
/// Pure arithmetic on the divergence tick and the two numbers the fleet agreed at
/// join, so every host computes it identically. It is placed:
///
/// * **past detection** — a peer's fold for tick *T* is in hand before any host
///   runs tick *T + delay + 1*, so `+ delay + 2` guarantees every host has set
///   its boundary before it would run this tick;
/// * **at least one whole interval ahead** — so the divergent host does not race
///   its own detection; and
/// * **on a sampling checkpoint** (rounded up to a multiple of `interval`) — a
///   tidiness that keeps the number a clean function of the shared interval.
///
/// `interval` is clamped to at least `1`: a fleet always samples on a non-zero
/// interval (a solo host never reaches recovery), and the clamp only keeps the
/// arithmetic total.
pub fn recovery_boundary(divergence_tick: u64, interval: u64, delay: u64) -> u64 {
    let interval = interval.max(1);
    let floor = divergence_tick
        .saturating_add(delay)
        .saturating_add(2)
        .saturating_add(interval);
    floor.div_ceil(interval).saturating_mul(interval)
}

/// Ask the digest exchange whether a recovery is due, and if so, with what plan.
///
/// The one call the Bevy adapter makes each frame while no recovery is active.
/// It composes [`earliest_divergence`], [`elect`] and [`recovery_boundary`] into
/// the single deterministic decision every host reaches from the same shared
/// state.
pub fn decide(
    fleet: &[HostSlot],
    local: HostSlot,
    local_ledger: &DigestLedger,
    peers: &BTreeMap<HostSlot, DigestLedger>,
    interval: u64,
    delay: u64,
) -> RecoveryDecision {
    let Some(window) = earliest_divergence(fleet, local, local_ledger, peers) else {
        return RecoveryDecision::Pending;
    };
    match elect(&window.digests) {
        Ok(election) => RecoveryDecision::Recover(RecoveryPlan {
            divergence_tick: window.tick,
            last_agreed_tick: window.last_agreed,
            boundary_tick: recovery_boundary(window.tick, interval, delay),
            canonical_digest: election.canonical_digest,
            leader: election.leader,
            recovering: election.recovering,
            digests: window.digests,
        }),
        Err(reason) => RecoveryDecision::Unrecoverable { window, reason },
    }
}

#[cfg(test)]
mod tests {
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
        assert!(b >= 20 + delay + 2, "the boundary must clear detection: {b}");
        assert!(b >= 20 + interval, "…and be at least an interval ahead: {b}");
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
}
