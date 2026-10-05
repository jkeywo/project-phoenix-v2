//! The pure divergence-recovery decision (issue #1118).
//!
//! Bevy-free, so the whole of "who recovers, from whom, and when" is decided in
//! one tested place rather than inside a system that also has to own a clock and
//! a snapshot walk. The Bevy adapter that drives it is `crate::lockstep::recovery`
//! (AGENTS.md rule 10: the decision module stays Bevy-free and its adapter is a
//! sibling), exactly as `crate::session` is the pure barrier and
//! `crate::lockstep::mod` is its adapter.
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
//! The barrier (`crate::session`) is what makes the delivery claims
//! above true: a host may not simulate tick *S* until every peer is ready through
//! *S*, and a peer's readiness for *S* rides the same reliable-ordered channel,
//! after that peer's earlier digest frames. So "the fold for tick *T* has
//! arrived" is not hoped for — it is implied by the fleet having advanced.

use std::collections::{BTreeMap, BTreeSet};

use crate::digest::DigestLedger;
use crate::HostSlot;

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
        for &slot in fleet {
            let fold = if slot == local {
                Some(checkpoint.digest)
            } else {
                peers.get(&slot).and_then(|ledger| ledger.digest_at(tick))
            };
            // Stop at the first incomplete checkpoint, before judging any
            // later checkpoint on a partial fleet.
            digests.insert(slot, fold?);
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
#[path = "recovery_plan_tests.rs"]
mod tests;
