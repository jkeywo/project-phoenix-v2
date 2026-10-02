//! The pure lockstep barrier: who is in the fleet, how far ahead input is
//! stamped, and whether this host may simulate the next tick yet (issue #1116).
//!
//! Bevy-free, so the whole of "when may I step?" is decided in one tested place
//! rather than inside a system that also has to own a clock.
//!
//! # The scheme, in one paragraph
//!
//! Every host stamps its own crew's commands for `SimTick + delay` and tells the
//! fleet, every tick, that it is *ready through* that tick — meaning it will
//! never issue anything for a tick at or below it again. A host may simulate
//! tick *T* once every peer is ready through *T*. Because every host starts
//! ready through `delay`, the first `delay + 1` ticks run without waiting for
//! anybody, and thereafter a peer may be up to `delay` ticks behind before
//! anyone stalls. Nothing is speculated: a host that is not ready withholds the
//! tick and says so.
//!
//! # Why the order is peer-independent rather than owner-sequenced
//!
//! The fleet owner (#1114) is the obvious sequencer, and it was rejected. An
//! owner-assigned order costs every non-owner a second network hop before its
//! own command is even ordered, which doubles the delay the fleet has to agree
//! — and the delay is the whole latency budget. It also makes the owner a
//! single point of failure for *ordering*, which #1119's host-loss and #1120's
//! slot recovery would then have to arbitrate before they could do anything
//! else.
//!
//! [`CommandOrder`](crate::command_admission::log::CommandOrder) needs neither.
//! It is `(origin slot, that origin's own sequence)`, which is a **total** order
//! computable from the key alone, so every host sorts the same merged set the
//! same way without asking anybody. The owner is still what makes it work — it
//! minted the slot ids, so "slot 2" means the same host everywhere
//! (`p2p-delta-identity-is-minted`) — it simply does not have to be on the
//! critical path of every command.
//!
//! And the whole key travels with the command, so #1118's recovery can replay a
//! merged history: an entry says which host issued it and where it sat, rather
//! than depending on a receiver's memory of what arrived when.

use std::collections::{BTreeMap, BTreeSet};

use crate::command_admission::log::HostSlot;

/// Why this host is not allowed to run the next tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stall {
    /// The tick that is being withheld.
    pub tick: u64,
    /// Every peer whose input for that tick has not arrived, and how far each
    /// one has actually declared. Sorted by slot, so two hosts reporting the
    /// same stall report it identically.
    pub waiting_on: Vec<(HostSlot, u64)>,
}

impl std::fmt::Display for Stall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tick {} withheld, waiting on", self.tick)?;
        for (slot, ready) in &self.waiting_on {
            write!(f, " {}(ready through {ready})", slot.slot_id())?;
        }
        Ok(())
    }
}

/// The frozen fleet, from the point of view of one host, plus what it knows
/// about how far every other host has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockstepSession {
    local: HostSlot,
    delay: u64,
    /// Every peer's watermark. The local slot is deliberately absent: a host
    /// never waits for itself, and an entry for it would be a second copy of
    /// `SimTick` that could disagree with the real one.
    ready_through: BTreeMap<HostSlot, u64>,
    /// Peers whose host has left (issue #1119). A departed slot is removed from
    /// `ready_through` and remembered here so a `TickFrame` from it that was in
    /// flight when the socket closed — delivered late or out of order — cannot
    /// re-insert it as something to wait for again. This is the barrier half of
    /// "reordered, delayed and duplicate disconnect observations converge on the
    /// same single transition" (AC5).
    departed: BTreeSet<HostSlot>,
}

impl LockstepSession {
    /// Open a session for `local` in a fleet that also holds `peers`, with an
    /// agreed input `delay`.
    ///
    /// Every peer starts ready through `delay`, which is not an optimistic
    /// assumption: a host stamps for `SimTick + delay`, so before it has run a
    /// single tick it has by definition already issued everything it ever will
    /// for ticks `0..=delay`. Seeding anything lower would deadlock the fleet
    /// on tick zero, since no host can send a frame before it has stepped.
    pub fn new(local: HostSlot, peers: impl IntoIterator<Item = HostSlot>, delay: u64) -> Self {
        Self::new_at(local, peers, delay, 0).expect("tick zero plus a u64 delay cannot overflow")
    }

    /// Open a session at an already-agreed activation tick.
    ///
    /// Browser participants rebase to a common non-zero lobby epoch before the
    /// wait-set is installed. Seeding peers through only bare `delay` there
    /// would deadlock immediately because the next local tick is already far
    /// beyond that watermark; the seed is therefore `activation_tick + delay`.
    pub fn new_at(
        local: HostSlot,
        peers: impl IntoIterator<Item = HostSlot>,
        delay: u64,
        activation_tick: u64,
    ) -> Option<Self> {
        let initial_ready = activation_tick.checked_add(delay)?;
        let ready_through = peers
            .into_iter()
            .filter(|slot| *slot != local)
            .map(|slot| (slot, initial_ready))
            .collect();
        Some(Self {
            local,
            delay,
            ready_through,
            departed: BTreeSet::new(),
        })
    }

    /// This host's own slot — the origin every command it admits is ordered
    /// under.
    pub fn local(&self) -> HostSlot {
        self.local
    }

    /// The agreed input delay in logical ticks.
    pub fn delay(&self) -> u64 {
        self.delay
    }

    /// Every peer this host waits for, in slot order.
    pub fn peers(&self) -> impl Iterator<Item = HostSlot> + '_ {
        self.ready_through.keys().copied()
    }

    /// Whether there is anybody to wait for.
    ///
    /// A one-host "fleet" is a real state — the operator opened a fleet and
    /// nobody joined — and it must behave exactly like no fleet at all, or the
    /// solo path acquires a stall it can never clear.
    pub fn is_alone(&self) -> bool {
        self.ready_through.is_empty()
    }

    /// Add a digest-proven mid-session participant at a paused boundary.
    ///
    /// The candidate restored this exact `activation_tick`; seeding its
    /// watermark through `tick + delay` is the same bootstrap guarantee
    /// [`Self::new_at`] gives every original peer.  Exact retries are inert.
    pub fn admit_peer(&mut self, peer: HostSlot, activation_tick: u64) {
        if peer == self.local || self.departed.contains(&peer) {
            return;
        }
        self.ready_through
            .entry(peer)
            .or_insert_with(|| activation_tick.saturating_add(self.delay));
    }

    /// Record a peer's watermark.
    ///
    /// Monotonic by construction: a frame that arrives out of order, or a
    /// duplicate, can only ever be ignored. That is what makes reordered and
    /// repeated delivery converge instead of walking a host's view backwards —
    /// the property #1119 needs of every mesh observation and is cheaper to
    /// build in here than to bolt on there.
    pub fn observe(&mut self, from: HostSlot, ready_through: u64) {
        if from == self.local || self.departed.contains(&from) {
            return;
        }
        let entry = self.ready_through.entry(from).or_insert(0);
        if ready_through > *entry {
            *entry = ready_through;
        }
    }

    /// The last watermark this host heard from `slot`, or `None` for a slot it
    /// does not wait for (its own, or one that has already departed).
    ///
    /// This is the observation issue #1119's disconnect tick is a function of:
    /// the lost host's own last declared `ready_through`, which reliable
    /// delivery gave every survivor identically, so every survivor derives the
    /// same [`agreed_loss_tick`](crate::lockstep::host_loss::agreed_loss_tick)
    /// from it without asking anybody when they noticed.
    pub fn watermark_of(&self, slot: HostSlot) -> Option<u64> {
        self.ready_through.get(&slot).copied()
    }

    /// Stop waiting for a departed host (issue #1119).
    ///
    /// Removing the slot from the wait-set is what lets the barrier resume: a
    /// fleet stalled at the first tick a vanished host never covered
    /// (`watermark + 1`) runs again the instant that host stops being one it
    /// waits for. It is deliberately only the barrier half of the transition —
    /// the ship's flip to Backfill is a tick-stamped event applied at
    /// `agreed_loss_tick`, so this may run frame-driven the moment the loss is
    /// observed without moving any folded state. Idempotent: departing a slot
    /// already gone, or one that was never a peer, changes nothing.
    ///
    /// A departed slot's watermark is forgotten and the slot is remembered as
    /// gone, so a duplicate or reordered report — or a `TickFrame` from the lost
    /// host still in flight — cannot resurrect it as something to wait for again.
    pub fn depart(&mut self, slot: HostSlot) {
        if slot == self.local {
            return;
        }
        self.ready_through.remove(&slot);
        self.departed.insert(slot);
    }

    /// Whether `slot`'s host has left this fleet.
    pub fn has_departed(&self, slot: HostSlot) -> bool {
        self.departed.contains(&slot)
    }

    /// Re-admit a departed slot a replacement machine has claimed (issue #1120).
    ///
    /// The inverse of [`Self::depart`]: it clears the departed mark and re-inserts
    /// the slot into the wait-set at `watermark`, so the barrier waits for it again
    /// and [`Self::observe`] will once more advance its watermark (a departed slot's
    /// observations are otherwise ignored). Seeding it at the recovery `watermark`
    /// — the boundary the whole fleet holds at — lets every survivor run UP TO the
    /// boundary but no further under the barrier alone, which is the same tick the
    /// recovery leader captures its canonical record at; the replacement's genuine
    /// post-restore frames then advance the watermark and the fleet resumes in true
    /// lockstep. Idempotent for the local slot (a host never waits for itself).
    pub fn rejoin(&mut self, slot: HostSlot, watermark: u64) {
        if slot == self.local {
            return;
        }
        self.departed.remove(&slot);
        self.ready_through.insert(slot, watermark);
    }

    /// The watermark this host declares having reached `tick`.
    pub fn ready_through(&self, tick: u64) -> u64 {
        tick.saturating_add(self.delay)
    }

    /// The most recent watermark this host has observed from `slot`, or `None`
    /// for the local slot (which is never tracked — a host does not wait for
    /// itself) or an unknown one.
    ///
    /// Divergence recovery (#1118) reads this to tell a recovering peer has
    /// RESUMED past the recovery boundary: a paused host's last watermark sits at
    /// `boundary + delay`, and the first tick it runs after restoring pushes it to
    /// `boundary + delay + 1`. It is a monotone liveness signal, never a
    /// simulation decision — the barrier still governs which ticks actually run.
    pub fn observed(&self, slot: HostSlot) -> Option<u64> {
        self.ready_through.get(&slot).copied()
    }

    /// Whether every peer's input for `tick` is in hand.
    pub fn may_simulate(&self, tick: u64) -> bool {
        self.ready_through.values().all(|ready| *ready >= tick)
    }

    /// [`None`] when the tick may run; the diagnostic otherwise.
    pub fn stall_at(&self, tick: u64) -> Option<Stall> {
        let waiting_on: Vec<(HostSlot, u64)> = self
            .ready_through
            .iter()
            .filter(|(_, ready)| **ready < tick)
            .map(|(slot, ready)| (*slot, *ready))
            .collect();
        if waiting_on.is_empty() {
            None
        } else {
            Some(Stall { tick, waiting_on })
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
