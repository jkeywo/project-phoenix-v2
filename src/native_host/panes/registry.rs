//! Pane lifecycle and the two queues each pane owns (issue #1122).
//!
//! One [`Pane`] is one logical client: an isolated document, a
//! [`PaneIdentity`](super::identity::PaneIdentity), an **input queue** of
//! decoded `ClientMessage`s the page has asked for, and an **outbound queue** of
//! encoded `ServerMessage`s waiting to be pushed into it. [`PaneRegistry`] owns
//! them and nothing else.
//!
//! Pure and Bevy-free, on purpose: everything about which pane gets what is
//! decided here, so it is decided somewhere a unit test can reach without a GPU,
//! an SDK, a window or an `App`. The Ultralight half
//! ([`super::ultralight`](super)) only draws and forwards input; the Bevy half
//! ([`super::transport`]) only moves messages across the seam.
//!
//! # The outbound cap, and why it is not a plain ring buffer
//!
//! A page that stops draining — a pane still loading, a stalled frame, a
//! document that threw during boot — must not grow the host's memory without
//! bound. But dropping the *oldest* message indiscriminately is wrong in a way
//! that takes an hour to diagnose: `Welcome`, `StationAssigned` and
//! `GameStarted` are one-shot state transitions, and a pane that missed its
//! `Welcome` sits in the lobby forever with a completely healthy-looking log.
//!
//! So the cap respects the delivery class and payload semantics: complete
//! snapshots can supersede the same kind, and partial
//! updates compose. None may be discarded without a replacement: even a full
//! projection can publish only on change. An otherwise-full queue faults the
//! pane through the existing overflow/close path instead of losing state.
//!
//! # Coalesce complete states and compose deltas before the cap is reached
//!
//! The cap is the safety net; the everyday rule is stricter. A phone's snapshot
//! class rides a lossy, unordered channel. That classification alone does not
//! make a payload complete: BlackboardUpdate carries changed System keys, and
//! SimState carries sparse fields for changed entities. A pane's queue once kept
//! every message, and every one it kept was a synchronous script evaluation on the
//! frame that finally drained it: a slow frame unpacked into ten simulation
//! ticks, each publishing snapshots to every pane, and the next frame paid for
//! all of them — which made it slower still (measured at 25–50 ms of a 180 ms
//! frame with three consoles open, issue #1403). Complete snapshots replace the
//! same kind; partial snapshots merge by System or entity key. The merge stops
//! at reliable messages and events, preserving reset, spawn, despawn and
//! recipient-visibility boundaries. All retained data belongs to this PaneId;
//! closing or superseding it discards the queue with the incarnation.
//!
//! # This queue is only half the bound, and the other half is in the page
//!
//! What is queued here is what the host has **not handed over yet**. A `Live`
//! pane whose page accepts every push and then does nothing with it therefore
//! never fills this queue — the backlog would sit in an unbounded JavaScript
//! array on the other side of the bridge, and the overflow-close this cap
//! describes would be unreachable for every pane past [`PaneLifecycle::Loading`].
//!
//! So `pane_boot.js` caps the page's own inbox and **throws** past it.
//! [`super::surface::pump_pane`] reads that throw the way it reads any failed
//! push — stop the batch, requeue it in order — which is what puts a wedged
//! `Live` pane's traffic back into this queue, where the cap below can see it.
//! The division is not arbitrary: the page is the only side that knows it has
//! stopped draining, and the host is the only side that knows which messages may
//! safely supersede queued state.

use std::collections::VecDeque;

use crate::core::codec::JsonCodec;
use crate::core::messages::{
    ClientMessage, DeliveryClass, ServerMessage, ServerMessageDiscriminants,
};

mod snapshot;
use snapshot::{merge_delta, PendingSnapshot};

use super::identity::PaneIdentity;

pub use phoenix_platform::input_routing::PaneId;

/// Where a pane is in its life.
///
/// A pane is `Loading` from the moment it is opened until its document reports
/// that it has finished loading. The distinction is not cosmetic: a page that
/// has not run its own scripts yet has no bridge to push into, so every push
/// aimed at a `Loading` pane would throw. Queueing instead means the first frame
/// after the load delivers the whole backlog in order, `Welcome` first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneLifecycle {
    /// Open, document not yet ready to receive pushes.
    Loading,
    /// Open and taking traffic in both directions.
    Live,
    /// The Session moved to another connection. Keep its screen reservation
    /// until the operator changes it, but accept no traffic or auto-recovery.
    Superseded,
    /// Closed. Its session is the lobby's business now — the same disconnect
    /// path a phone that walked out of range takes, which keeps the station
    /// held and flips it to `Backfill`.
    Closed,
}

/// One outbound message waiting for its page.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneDispatch {
    /// The encoded `ServerMessage`, in the same JSON a phone would receive.
    pub json: String,
    /// Which channel it would have ridden on a network transport.
    pub delivery: DeliveryClass,
    /// Which `ServerMessage` variant this is — the coalescing key for a
    /// snapshot (see the module note). Set where the message is encoded, never
    /// parsed back out of the JSON.
    pub kind: ServerMessageDiscriminants,
    pending: PendingSnapshot,
}

impl PaneDispatch {
    pub(super) fn encoded(json: String, delivery: DeliveryClass, message: &ServerMessage) -> Self {
        Self {
            json,
            delivery,
            kind: ServerMessageDiscriminants::from(message),
            pending: PendingSnapshot::for_message(delivery, message),
        }
    }

    fn is_barrier(&self) -> bool {
        matches!(self.pending, PendingSnapshot::Preserve)
    }

    fn absorb(&mut self, older: &Self) -> bool {
        if self.kind != older.kind {
            return false;
        }
        match (&older.pending, &self.pending) {
            (PendingSnapshot::Replace, PendingSnapshot::Replace) => true,
            (PendingSnapshot::Delta(old), PendingSnapshot::Delta(new)) => {
                let Some(merged) = merge_delta(old, new) else {
                    return false;
                };
                let Ok(json) = JsonCodec.encode_server(&merged) else {
                    // Keep both originals if the merged payload cannot encode.
                    return false;
                };
                self.json = json;
                self.pending = PendingSnapshot::Delta(std::sync::Arc::new(merged));
                true
            }
            _ => false,
        }
    }
}

/// What happened to a message handed to a full pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboundVerdict {
    /// Queued with room to spare.
    Queued,
    /// Queued, replacing or composing with the older snapshot of the same kind
    /// that was still waiting. Ordinary whenever the
    /// host outpaces the page — see the module note on coalescing.
    QueuedSuperseding,
    /// Refused a message that could not safely supersede queued state. Even a
    /// complete projection can be the producer's final change-only update.
    /// The caller should close the pane rather than pretend this was fine.
    Overflowed,
}

/// One isolated logical client.
#[derive(Debug)]
pub struct Pane {
    id: PaneId,
    identity: PaneIdentity,
    lifecycle: PaneLifecycle,
    /// What the page has asked for, awaiting the next transport poll.
    inbound: VecDeque<ClientMessage>,
    /// What the simulation has produced for this pane, awaiting the next push.
    outbound: VecDeque<PaneDispatch>,
    outbound_cap: usize,
}

impl Pane {
    /// This pane's handle.
    pub fn id(&self) -> PaneId {
        self.id
    }

    /// Who this pane is to the lobby.
    pub fn identity(&self) -> &PaneIdentity {
        &self.identity
    }

    /// The session token this pane presents. Shorthand for
    /// `pane.identity().token()`, because routing asks for it constantly.
    pub fn token(&self) -> &str {
        self.identity.token()
    }

    /// Where this pane is in its life.
    pub fn lifecycle(&self) -> PaneLifecycle {
        self.lifecycle
    }

    /// Mark the pane's document loaded, so pushes may begin.
    ///
    /// Idempotent, and never resurrects a closed pane: a document that finishes
    /// loading after the operator shut its pane must not reopen it.
    pub fn mark_live(&mut self) {
        if self.lifecycle == PaneLifecycle::Loading {
            self.lifecycle = PaneLifecycle::Live;
        }
    }

    pub(crate) fn supersede(&mut self) {
        self.lifecycle = PaneLifecycle::Superseded;
        self.inbound.clear();
        self.outbound.clear();
    }

    /// Queue something the page asked for.
    pub fn push_inbound(&mut self, msg: ClientMessage) {
        self.inbound.push_back(msg);
    }

    /// Take everything the page has asked for since the last call, in order.
    pub fn drain_inbound(&mut self) -> Vec<ClientMessage> {
        self.inbound.drain(..).collect()
    }

    /// Queue something for the page. See the module note for the cap policy.
    pub fn push_outbound(&mut self, mut dispatch: PaneDispatch) -> OutboundVerdict {
        // Never move a delta across Welcome, a despawn, a Station change or
        // another ordered event. Those can change the meaning/visibility of it.
        if !dispatch.is_barrier() {
            let superseded = self
                .outbound
                .iter()
                .enumerate()
                .rev()
                .take_while(|(_, d)| !d.is_barrier())
                .find_map(|(index, d)| (d.kind == dispatch.kind).then_some(index));
            if let Some(index) = superseded {
                if dispatch.absorb(&self.outbound[index]) {
                    self.outbound.remove(index);
                    self.outbound.push_back(dispatch);
                    return OutboundVerdict::QueuedSuperseding;
                }
            }
        }
        if self.outbound.len() < self.outbound_cap {
            self.outbound.push_back(dispatch);
            return OutboundVerdict::Queued;
        }
        OutboundVerdict::Overflowed
    }

    /// Take everything queued for the page, in order.
    ///
    /// A `Loading` pane hands back nothing and keeps its queue: its document has
    /// no bridge to push into yet, and a push that throws is a message lost.
    pub fn drain_outbound(&mut self) -> Vec<PaneDispatch> {
        if self.lifecycle != PaneLifecycle::Live {
            return Vec::new();
        }
        self.outbound.drain(..).collect()
    }

    /// Put an un-delivered batch back at the front of the queue, in order.
    ///
    /// The push half of a frame can fail for an entirely ordinary reason (a
    /// document that has loaded but whose modules have not run), and the batch
    /// it was carrying has already been drained. Returning it — rather than
    /// dropping it, or appending it behind whatever the same frame produced
    /// after it — is what makes the next frame retry from exactly where this one
    /// stopped, with `Welcome` still first.
    ///
    /// The simulation can enqueue more messages while the pane thread pushes
    /// its drained batch. Replay both batches through the ordinary cap and
    /// coalescing policy, oldest first, so a newer queued snapshot supersedes
    /// a returned one and reliable messages retain their original order.
    /// Returns true if any message exceeded the reliable budget; the caller
    /// must report the same fault as an overflow from `push_outbound`.
    pub fn requeue_front(&mut self, batch: Vec<PaneDispatch>) -> bool {
        let newer = std::mem::take(&mut self.outbound);
        let mut overflowed = false;
        for dispatch in batch.into_iter().chain(newer) {
            overflowed |= self.push_outbound(dispatch) == OutboundVerdict::Overflowed;
        }
        overflowed
    }

    /// How many messages are waiting for this page. Diagnostic.
    pub fn queued_outbound(&self) -> usize {
        self.outbound.len()
    }
}

/// Every pane one host owns.
#[derive(Debug)]
pub struct PaneRegistry {
    panes: Vec<Pane>,
    next_id: u32,
    outbound_cap: usize,
}

/// The default outbound backlog a pane may accumulate before snapshots start
/// being dropped.
///
/// Not a gameplay value and not a tuning knob a designer would reach for — it is
/// a memory bound on a queue whose consumer is a document that has not finished
/// loading. Sized to comfortably cover a document load (a few seconds of
/// snapshot cadence) without letting a wedged page grow unboundedly.
pub const DEFAULT_OUTBOUND_CAP: usize = 512;

impl Default for PaneRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_OUTBOUND_CAP)
    }
}

impl PaneRegistry {
    /// An empty registry whose panes each hold at most `outbound_cap` messages.
    pub fn new(outbound_cap: usize) -> Self {
        Self {
            panes: Vec::new(),
            next_id: 0,
            outbound_cap: outbound_cap.max(1),
        }
    }

    /// Open a pane for `identity`, returning its handle.
    ///
    /// The pane starts `Loading`, holds no station, and has said nothing to the
    /// lobby: opening a pane creates no session. A session appears when the
    /// pane's page sends `Identify` through the transport seam like any other
    /// participant, which is the point.
    pub fn open(&mut self, identity: PaneIdentity) -> PaneId {
        let id = PaneId(self.next_id);
        self.next_id += 1;
        self.panes.push(Pane {
            id,
            identity,
            lifecycle: PaneLifecycle::Loading,
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            outbound_cap: self.outbound_cap,
        });
        id
    }

    /// Close a pane, returning the token the lobby is owed a disconnect for and
    /// anything the page had already said but not yet been polled for.
    ///
    /// The pending input comes back rather than being dropped because order
    /// matters to the lobby: a `ReleaseStation` followed by a disconnect vacates
    /// the seat, and losing the first leaves the station held by a participant
    /// who has gone. What the page never *heard* is dropped, because there is no
    /// longer anywhere to put it.
    ///
    /// The pane's record stays, marked [`PaneLifecycle::Closed`], and its id is
    /// never reissued — a message that arrives naming a closed pane is then a
    /// message for a pane that has gone, rather than one silently delivered to
    /// whoever inherited the number.
    pub fn close(&mut self, id: PaneId) -> Option<(String, Vec<ClientMessage>)> {
        let pane = self.panes.iter_mut().find(|p| p.id == id)?;
        if pane.lifecycle == PaneLifecycle::Closed {
            return None;
        }
        pane.lifecycle = PaneLifecycle::Closed;
        pane.outbound.clear();
        let pending: Vec<ClientMessage> = pane.inbound.drain(..).collect();
        Some((pane.identity.token().to_string(), pending))
    }

    /// One pane by handle, closed ones included.
    pub fn get(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|p| p.id == id)
    }

    /// One pane that may exchange traffic. Closed and superseded panes are
    /// excluded; physical presence alone does not permit further queue writes.
    pub fn get_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|p| {
            p.id == id && matches!(p.lifecycle, PaneLifecycle::Loading | PaneLifecycle::Live)
        })
    }

    /// Every open pane, in the order they were opened.
    pub fn open_panes(&self) -> impl Iterator<Item = &Pane> {
        self.panes
            .iter()
            .filter(|p| p.lifecycle != PaneLifecycle::Closed)
    }

    /// Every open pane, mutably, in the order they were opened.
    pub fn open_panes_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        self.panes
            .iter_mut()
            .filter(|p| p.lifecycle != PaneLifecycle::Closed)
    }

    /// How many panes are open.
    pub fn open_count(&self) -> usize {
        self.open_panes().count()
    }

    /// The open pane presenting `token`, if any.
    pub fn find_by_token(&self, token: &str) -> Option<&Pane> {
        self.open_panes().find(|p| p.token() == token)
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
