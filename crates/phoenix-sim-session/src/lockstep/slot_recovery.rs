use super::FleetRoster;
use bevy::prelude::*;
use phoenix_runtime::HostSlot;
use std::collections::{BTreeMap, BTreeSet};

/// Every replacement claim on a disconnected slot this host has heard, and the
/// deterministic winner of each (issue #1120, AC2).
///
/// Pure and total, so the whole of "which claim won" is decided in one tested
/// place. The winner for a slot is the lowest `claim_seq` ever observed for it —
/// the first the owner minted — which is a function of the shared claim VALUES and
/// not of the order this host happened to receive them, so every host that has
/// heard the same claims agrees the same winner.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingSlotClaims {
    /// slot -> (winning claim_seq, the tick the owner stamped the winning claim).
    winners: BTreeMap<HostSlot, (u64, u64)>,
    /// Slots whose recovery [`drive_slot_recovery`] has already opened, so a later
    /// duplicate claim frame is inert rather than a second recovery.
    opened: BTreeSet<HostSlot>,
}

impl PendingSlotClaims {
    /// Record a granted claim on `slot`, sequenced `claim_seq`, stamped at `tick`.
    ///
    /// The winner is the lowest `claim_seq` seen so far, so a lower one arriving
    /// later supersedes a higher one already recorded — the resolution is a
    /// function of the values, deterministic under any arrival order. The winning
    /// claim's `tick` travels with its seq, because the recovery boundary is
    /// derived from it and must be the same everywhere.
    pub fn observe(&mut self, slot: HostSlot, claim_seq: u64, tick: u64) {
        match self.winners.get_mut(&slot) {
            Some(entry) if claim_seq < entry.0 => *entry = (claim_seq, tick),
            Some(_) => {}
            None => {
                self.winners.insert(slot, (claim_seq, tick));
            }
        }
    }

    /// The winning `claim_seq` for `slot`, if any claim has been heard.
    pub fn winning_seq(&self, slot: HostSlot) -> Option<u64> {
        self.winners.get(&slot).map(|(seq, _)| *seq)
    }

    /// Whether `claim_seq` is the winning claim for `slot` — "did this claim win?".
    pub fn wins(&self, slot: HostSlot, claim_seq: u64) -> bool {
        self.winning_seq(slot) == Some(claim_seq)
    }

    /// Whether a recovery for `slot` has already been opened.
    pub fn is_opened(&self, slot: HostSlot) -> bool {
        self.opened.contains(&slot)
    }

    /// The next winning claim whose recovery has not been opened yet, as
    /// `(slot, claim_seq, stamped_tick)`, lowest slot first for a stable order.
    pub fn next_unopened(&self) -> Option<(HostSlot, u64, u64)> {
        self.winners
            .iter()
            .find(|(slot, _)| !self.opened.contains(slot))
            .map(|(slot, (seq, tick))| (*slot, *seq, *tick))
    }

    pub fn mark_opened(&mut self, slot: HostSlot) {
        self.opened.insert(slot);
    }
}

/// The leader that transfers the canonical record for a recovery of `recovering`
/// (issue #1120): the lowest roster slot that is NOT the recovering one.
///
/// Pure and shared — every host computes the same leader from the frozen roster
/// and the recovering slot alone. In practice this is the owner (`slot-1`, the star
/// centre, which a member's socket close never removes), unless the owner itself is
/// the slot being recovered, which host migration — out of this issue's scope —
/// would own.
pub fn leader_for(roster: &FleetRoster, recovering: HostSlot) -> HostSlot {
    roster
        .ships()
        .iter()
        .map(|ship| ship.host)
        .filter(|&host| host != recovering)
        .min()
        .unwrap_or_else(|| roster.lead())
}
