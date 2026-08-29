//! Backfilling a disconnected ship host at an agreed tick (issue #1119).
//!
//! When a ship host vanishes, the fleet must keep running: its ship is not
//! removed and not replaced by a simplified sim — it keeps its complete
//! authoritative state, and only its control SOURCE flips to ordinary Backfill,
//! through the same Station Rating machinery a single-host disconnect already
//! uses (`lobby::handler::process_disconnect_with_stations`,
//! `ship::rating::apply_rating`). The one thing a fleet adds is *when*: the flip
//! must land on the same tick on every surviving host, or the survivors run the
//! lost ship's AI from different ticks and their authoritative folds diverge.
//!
//! # The agreement, and why it is peer-independent
//!
//! The disconnect tick is [`agreed_loss_tick`] of the lost host's own last
//! declared watermark — the first tick past everything that host promised it
//! would ever say. That watermark is the lost slot's
//! [`TickFrame::ready_through`](crate::lockstep::frame::TickFrame::ready_through).
//!
//! But it is derived by exactly ONE authority per loss, not independently on
//! every survivor, because a slot's watermark is shared identically only once
//! that slot has genuinely departed and its final frame has preceded the loss
//! report in the one reliable ordered stream. While the slot is still live — or
//! its loss is still propagating — honest survivors legitimately hold DIFFERENT
//! watermarks for it (the relay lead sees a frame before it forwards it), so a
//! tick each survivor derived from its own watermark could disagree. The star
//! relay (`p2p-delta-transport-is-a-star-today`) resolves this: exactly one host
//! — the connection holder — sees the socket close, derives the tick from the
//! lost slot's last watermark, and stamps that concrete tick into the report it
//! re-broadcasts. Every other survivor adopts that carried tick VERBATIM rather
//! than re-deriving from its own watermark, so all survivors converge on the
//! identical tick under any frame-arrival interleaving. The
//! [`apply_mesh_inbox`](crate::lockstep::apply_mesh_inbox) `HostLoss` arm is where
//! that self-observer-derives / member-honours split lives.
//!
//! The fleet has already run the lost ship on real input right up to the
//! authority's watermark (every command it stamped for a tick at or below it is
//! in hand), so flipping to Backfill at `watermark + 1` is seamless: the last
//! real tick and the first AI tick are adjacent, with no gap to interpolate and
//! nothing to roll back.
//!
//! Rejecting a forged or unauthorised loss report is NOT part of this agreement:
//! honouring a report convergently keeps the fold identical on every host that
//! hears it, but an unauthenticated report for a live slot is still acted on.
//! Full reporter authentication is deferred to #1118/#1120 (see the `HostLoss`
//! arm's `TODO`), because it cannot be done deterministically without a
//! ground-truth liveness signal only the connection holder has.
//!
//! # Convergence (AC5)
//!
//! [`PendingHostLoss`] takes the maximum of every tick anyone derives for a lost
//! slot and drops a report for a slot already applied, so two survivors
//! reporting the loss, one survivor reporting it twice, and a report delivered
//! late or out of order all converge on one transition at one tick. The barrier
//! half of the same property lives in [`LockstepSession::depart`]: a departed
//! slot is remembered, so a `TickFrame` from the lost host still in flight
//! cannot resurrect it.
//!
//! # What is host-local and what is authoritative (trap T4)
//!
//! The CAUSE of the loss — a transport socket closing — is host-local and never
//! replicated; the survivors carry no session for the lost crew, which lived on
//! the vanished machine. The EFFECT — every station on the lost ship at Backfill,
//! its human-seeking systems re-resolved to AI, its slot's frozen crewing
//! emptied so [`resolve_human_seeking_hosts`](crate::ship::coordination_systems::resolve_human_seeking_hosts)
//! keeps them there — is authoritative and applied at the agreed tick, in the
//! fixed schedule, identically on every survivor. This module owns only the
//! authoritative half.
//!
//! # Why the pure math and the Bevy adapter share this file (AGENTS.md rule 10)
//!
//! [`session`](crate::lockstep::session) and [`frame`](crate::lockstep::frame)
//! keep their pure decision Bevy-free with the adapter as a sibling. This file
//! deliberately co-locates the two: the agreement math ([`agreed_loss_tick`] and
//! all of [`PendingHostLoss`]) is pure and total and is unit-tested below with no
//! `World` at all, so rule 10's actual objective — the decision is testable
//! without booting Bevy — is already met. The only Bevy in the file is the thin
//! [`apply_host_loss_backfill`] adapter and the `Resource`/`Query` types it
//! needs; the queue it drives is a plain map. Splitting it into a `_systems.rs`
//! sibling is a valid tidy but buys no extra testability, so it is left as one
//! module on purpose rather than by oversight.

use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::command_admission::log::HostSlot;
use crate::logging::LogCat;
use crate::ship::rating;

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

/// Flip a departed host's ship to Backfill on the agreed tick (issue #1119).
///
/// Runs in `SimSet::Input`, `.before(resolve_human_seeking_hosts)`: at the
/// agreed tick it applies ordinary Backfill to every station the lost ship owns
/// — the same [`rating::apply_rating`] a single-host disconnect calls, reused
/// rather than forked — and empties that slot's frozen crewing in the
/// [`FleetRoster`](crate::lockstep::FleetRoster), so the human-seeking resolver
/// that runs right after sees an uncrewed ship and keeps its Comms/Nav on AI.
/// The ship itself is untouched: it keeps its complete authoritative state and
/// is driven, from this tick on, by the AI every survivor derives from the same
/// ticks (AC2, AC4).
///
/// `SimTick` is `Option<Res<_>>` for the same reason admission takes it as one:
/// a bare-`App` fixture that never registered the tick would otherwise fail
/// parameter validation and skip the flip entirely.
pub fn apply_host_loss_backfill(
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    mut pending: ResMut<PendingHostLoss>,
    mut roster: ResMut<super::FleetRoster>,
    mut ships: Query<(
        &crate::ship_plugin::ShipConfigComponent,
        &mut crate::ship_plugin::ShipSystemControlSources,
        &mut crate::ship_plugin::ActiveStationRatings,
        &super::FleetSlotOf,
    )>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    if pending.pending_len() == 0 {
        return;
    }
    let now = sim_tick.map_or(0, |t| t.0);
    for record in pending.drain_due(now) {
        // Empty the lost slot's frozen crewing so `resolve_human_seeking_hosts`
        // re-resolves its human-seeking systems to AI. Without this the next
        // tick's resolver would read the still-crewed roster and put Comms/Nav
        // back under a human on a ship whose crew is gone.
        roster.depart_slot(record.slot);
        // Flip every station the lost ship owns to Backfill — the SAME rating
        // transition a single-host disconnect applies, on the SAME machinery,
        // just to every station at once because the whole ship lost its crew.
        let mut flipped = false;
        for (config, mut sources, mut ratings, slot_of) in ships.iter_mut() {
            if slot_of.0 != record.slot {
                continue;
            }
            for station in &config.0.stations {
                rating::apply_rating(
                    &config.0,
                    &station.id,
                    rating::BACKFILL_RATING,
                    &mut sources.0,
                );
                ratings
                    .0
                    .insert(station.id.clone(), rating::BACKFILL_RATING.to_string());
            }
            flipped = true;
        }
        if flipped {
            crate::pinfo!(
                log,
                LogCat::Admit,
                "host-mesh backfill: {} left; its ship flips to Backfill at tick {}",
                record.slot.slot_id(),
                record.tick,
            );
        } else {
            crate::pwarn!(
                log,
                LogCat::Admit,
                "host-mesh backfill: {} left at tick {}, but its ship is not in \
                 this world — the transition is recorded but flips nothing",
                record.slot.slot_id(),
                record.tick,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The agreed tick is the first tick past the lost host's watermark, and it
    /// is the same on every host because it is a function of that watermark
    /// alone — never of who noticed the loss first.
    #[test]
    fn the_agreed_tick_is_the_first_uncovered_tick() {
        assert_eq!(agreed_loss_tick(0), 1);
        assert_eq!(agreed_loss_tick(417), 418);
        assert_eq!(
            agreed_loss_tick(u64::MAX),
            u64::MAX,
            "saturating: a watermark at the end of the line names itself rather \
             than wrapping to re-apply the flip on tick 1"
        );
    }

    /// Two reports of one loss, and one report twice, converge on a single
    /// transition at a single tick — the max of everything anyone derived.
    #[test]
    fn reports_converge_on_one_transition_at_one_tick() {
        let mut pending = PendingHostLoss::default();
        // First report from one survivor.
        assert!(pending.observe(HostSlot(3), 100), "a first report changes the picture");
        // A duplicate at the same tick changes nothing and asks for no re-broadcast.
        assert!(!pending.observe(HostSlot(3), 100));
        // A second survivor's report derived a higher tick (it had heard one more
        // of the lost host's frames): the agreement RAISES to it, and that is
        // worth re-broadcasting so the first survivor catches up.
        assert!(pending.observe(HostSlot(3), 102));
        // A lower, later-arriving report cannot walk it back.
        assert!(!pending.observe(HostSlot(3), 99));
        assert_eq!(pending.agreed_tick(HostSlot(3)), Some(102));

        // Nothing applies before its tick…
        assert!(pending.drain_due(101).is_empty());
        // …and exactly one record applies at it, whatever the report history.
        let due = pending.drain_due(102);
        assert_eq!(
            due,
            vec![HostLossRecord {
                slot: HostSlot(3),
                tick: 102
            }]
        );
        assert_eq!(pending.records(), due.as_slice());

        // Once applied, a late duplicate is inert — the flip cannot be re-agreed.
        assert!(!pending.observe(HostSlot(3), 200));
        assert!(pending.is_applied(HostSlot(3)));
        assert!(pending.drain_due(300).is_empty());
        assert_eq!(pending.records().len(), 1, "applying once records once");
    }

    /// A due drain is ordered by `(tick, slot)`, so two survivors that lose the
    /// same pair of hosts record the transitions in the same order.
    #[test]
    fn a_multi_loss_drain_is_ordered_by_tick_then_slot() {
        let mut pending = PendingHostLoss::default();
        pending.observe(HostSlot(4), 50);
        pending.observe(HostSlot(2), 50);
        pending.observe(HostSlot(3), 40);

        let due = pending.drain_due(100);
        assert_eq!(
            due,
            vec![
                HostLossRecord {
                    slot: HostSlot(3),
                    tick: 40
                },
                HostLossRecord {
                    slot: HostSlot(2),
                    tick: 50
                },
                HostLossRecord {
                    slot: HostSlot(4),
                    tick: 50
                },
            ],
            "earlier tick first, then lower slot — a total order every survivor \
             computes the same way"
        );
    }
}
