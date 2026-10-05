use bevy::prelude::*;
use phoenix_runtime::HostSlot;
use std::collections::{BTreeMap, BTreeSet};

/// The tick a fleet agrees a vanished host's ship flips to Backfill on: the
/// first tick past that host's last declared watermark (issue #1119).
///
/// Pure and total, so every survivor derives the same tick from the same
/// watermark — see the module docs for why that is the whole of peer-
/// independence. Saturating, so a watermark at the end of the number line names
/// itself rather than wrapping to zero and re-applying the transition on tick 1.
pub fn agreed_loss_tick(lost_watermark: u64) -> u64 {
    lost_watermark.saturating_add(1)
}

/// One applied host-loss transition, as the fleet's host-loss log records it.
///
/// The tick-stamped record AC1 asks for: a logged event, applied at the same
/// tick by every surviving host. It carries the slot and the tick and nothing
/// else — the ship it names keeps its full state, so there is nothing about that
/// state to record here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostLossRecord {
    /// The fleet slot whose host left.
    pub slot: HostSlot,
    /// The agreed tick its ship flipped to Backfill on.
    pub tick: u64,
}

/// Host-loss transitions observed but not yet applied, plus the log of those
/// that have (issue #1119).
///
/// The tick-stamped queue for the Backfill flip, the twin of
/// [`PendingCommands`](crate::command_admission::log::PendingCommands) for
/// ordinary input: a loss is OBSERVED frame-driven (a socket closed, or a peer
/// said one did), recorded here stamped for [`agreed_loss_tick`], and APPLIED in
/// the fixed schedule when `SimTick` reaches that tick — so the flip lands
/// deterministically on the agreed tick rather than at whatever wall-time the
/// observation arrived.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingHostLoss {
    /// Lost slot → the agreed apply tick, taking the maximum of every report so
    /// reordered and duplicate observations converge (AC5).
    pending: BTreeMap<HostSlot, u64>,
    /// Slots whose transition has already applied, so a later report of the same
    /// loss is a no-op rather than a second flip.
    applied_slots: BTreeSet<HostSlot>,
    /// The applied transitions, in apply order — the fleet's host-loss log.
    applied: Vec<HostLossRecord>,
}

impl PendingHostLoss {
    /// Record a report that `lost` has left, agreeing tick `at`.
    ///
    /// Returns `true` when this host's picture of the loss CHANGED — a first
    /// report, or a later report that raised the agreed tick — which is the
    /// signal to re-broadcast it so the rest of the fleet converges on the
    /// highest tick anyone derived. A report for a slot whose transition has
    /// already applied is dropped and returns `false`: the flip has happened and
    /// cannot be re-agreed.
    pub fn observe(&mut self, lost: HostSlot, at: u64) -> bool {
        if self.applied_slots.contains(&lost) {
            return false;
        }
        match self.pending.get_mut(&lost) {
            Some(existing) => {
                if at > *existing {
                    *existing = at;
                    true
                } else {
                    false
                }
            }
            None => {
                self.pending.insert(lost, at);
                true
            }
        }
    }

    /// Whether a loss for `slot` is known — pending or already applied.
    pub fn is_known(&self, slot: HostSlot) -> bool {
        self.applied_slots.contains(&slot) || self.pending.contains_key(&slot)
    }

    /// Whether `slot`'s Backfill transition has already applied.
    pub fn is_applied(&self, slot: HostSlot) -> bool {
        self.applied_slots.contains(&slot)
    }

    /// The agreed apply tick for a pending loss, if one is queued.
    pub fn agreed_tick(&self, slot: HostSlot) -> Option<u64> {
        self.pending.get(&slot).copied()
    }

    /// Remove and return every transition now due (stamped at or before `now`),
    /// recording each in the host-loss log as it applies.
    ///
    /// Ordered by `(tick, slot)` so two survivors that apply the same due set on
    /// the same tick record it in the same order — the host-loss log is
    /// byte-identical on hosts that agree, exactly as the command log is. "At or
    /// before" rather than "exactly", for the same reason [`PendingCommands`]
    /// drains that way: a survivor that observed the loss late applies the flip
    /// on the first tick it can rather than stranding it forever.
    ///
    /// [`PendingCommands`]: crate::command_admission::log::PendingCommands
    pub fn drain_due(&mut self, now: u64) -> Vec<HostLossRecord> {
        let mut due: Vec<HostLossRecord> = self
            .pending
            .iter()
            .filter(|(_, tick)| **tick <= now)
            .map(|(slot, tick)| HostLossRecord {
                slot: *slot,
                tick: *tick,
            })
            .collect();
        due.sort_by_key(|r| (r.tick, r.slot));
        for record in &due {
            self.pending.remove(&record.slot);
            self.applied_slots.insert(record.slot);
            self.applied.push(*record);
        }
        due
    }

    /// The host-loss log: every transition applied, in apply order.
    pub fn records(&self) -> &[HostLossRecord] {
        &self.applied
    }

    /// How many losses are waiting for their agreed tick.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}
