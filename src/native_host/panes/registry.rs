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

/// A pane's handle within one host. Stable for the pane's lifetime; not reused
/// after a close, so a stale reference names a gone pane rather than a
/// different one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u32);

impl std::fmt::Display for PaneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pane-{}", self.0)
    }
}

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
mod tests {
    use super::*;

    fn identity(n: u8) -> PaneIdentity {
        PaneIdentity::adopt(
            format!("3f1a6c2e-0a11-4b3c-9d55-00000000000{n}"),
            format!("crew-{n}"),
        )
        .unwrap()
    }

    fn snapshot(tag: &str) -> PaneDispatch {
        snapshot_of(ServerMessageDiscriminants::GameStarted, tag)
    }

    fn snapshot_of(kind: ServerMessageDiscriminants, tag: &str) -> PaneDispatch {
        PaneDispatch {
            json: tag.to_string(),
            delivery: DeliveryClass::Snapshot,
            kind,
            pending: PendingSnapshot::Replace,
        }
    }

    fn reliable(tag: &str) -> PaneDispatch {
        PaneDispatch {
            json: tag.to_string(),
            delivery: DeliveryClass::Reliable,
            kind: ServerMessageDiscriminants::Welcome,
            pending: PendingSnapshot::Preserve,
        }
    }

    #[test]
    fn opening_a_pane_creates_no_session_and_claims_no_station() {
        // A pane is a logical client, not a seat. It becomes a participant by
        // sending `Identify` through the seam like a phone, and this is the
        // "before" half of that claim.
        let mut registry = PaneRegistry::default();
        let id = registry.open(identity(1));
        let pane = registry.get(id).unwrap();
        assert_eq!(pane.lifecycle(), PaneLifecycle::Loading);
        assert_eq!(pane.queued_outbound(), 0);
        assert_eq!(registry.open_count(), 1);
    }

    #[test]
    fn two_panes_have_different_identities_and_different_handles() {
        let mut registry = PaneRegistry::default();
        let a = registry.open(identity(1));
        let b = registry.open(identity(2));
        assert_ne!(a, b);
        assert_ne!(
            registry.get(a).unwrap().token(),
            registry.get(b).unwrap().token()
        );
    }

    #[test]
    fn a_loading_pane_queues_its_backlog_instead_of_losing_it() {
        // `load_url` returns before the document's own scripts have run, so a
        // push aimed at a page in that window throws and the message is gone.
        // The one that matters is `Welcome`: a pane that missed it sits in the
        // lobby forever with a clean log.
        let mut registry = PaneRegistry::default();
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.push_outbound(reliable("welcome"));
        pane.push_outbound(snapshot("state"));
        assert!(
            pane.drain_outbound().is_empty(),
            "a loading document has no bridge to push into"
        );
        assert_eq!(pane.queued_outbound(), 2);

        pane.mark_live();
        let delivered: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
        assert_eq!(
            delivered,
            vec!["welcome".to_string(), "state".to_string()],
            "the backlog arrives in order, Welcome first"
        );
    }

    #[test]
    fn an_overfull_pane_preserves_every_unsuperseded_message_and_reports_the_fault() {
        let mut registry = PaneRegistry::new(3);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.mark_live();
        assert_eq!(
            pane.push_outbound(reliable("welcome")),
            OutboundVerdict::Queued
        );
        // Three DIFFERENT kinds, so the cap is what decides here and not the
        // same-kind coalescing rule (tested on its own below).
        assert_eq!(
            pane.push_outbound(snapshot_of(ServerMessageDiscriminants::ShieldStatus, "s1")),
            OutboundVerdict::Queued
        );
        assert_eq!(
            pane.push_outbound(snapshot_of(
                ServerMessageDiscriminants::SystemHullUpdate,
                "s2"
            )),
            OutboundVerdict::Queued
        );
        assert_eq!(
            pane.push_outbound(snapshot_of(ServerMessageDiscriminants::RepairState, "s3")),
            OutboundVerdict::Overflowed
        );
        let queued: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
        assert_eq!(
            queued,
            vec!["welcome".to_string(), "s1".to_string(), "s2".to_string()],
            "neither change-only projection was discarded without a replacement"
        );
    }

    #[test]
    fn replacement_stops_at_reliable_messages_and_keeps_the_newest_within_each_segment() {
        // The page only ever applies the newest snapshot of a kind, and every
        // stale one it is handed is a synchronous script evaluation the frame
        // pays for. The survivor sits where the newest arrived — at the tail —
        // and no reliable message moves.
        let mut registry = PaneRegistry::new(8);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.mark_live();
        assert_eq!(
            pane.push_outbound(reliable("welcome")),
            OutboundVerdict::Queued
        );
        assert_eq!(pane.push_outbound(snapshot("s1")), OutboundVerdict::Queued);
        assert_eq!(
            pane.push_outbound(reliable("assigned")),
            OutboundVerdict::Queued
        );
        assert_eq!(pane.push_outbound(snapshot("s2")), OutboundVerdict::Queued);
        assert_eq!(
            pane.push_outbound(snapshot("s3")),
            OutboundVerdict::QueuedSuperseding
        );
        assert_eq!(pane.queued_outbound(), 4);
        let queued: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
        assert_eq!(
            queued,
            vec![
                "welcome".to_string(),
                "s1".to_string(),
                "assigned".to_string(),
                "s3".to_string()
            ],
            "one snapshot survives per ordered segment; no snapshot crosses a reliable barrier"
        );
    }

    #[test]
    fn snapshots_of_different_kinds_do_not_supersede_each_other() {
        // Two kinds carry two different pieces of state; the newest of each is
        // what the page needs, not the newest overall.
        let mut registry = PaneRegistry::new(8);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.mark_live();
        let a = ServerMessageDiscriminants::GameStarted;
        let b = ServerMessageDiscriminants::ShipDestroyed;
        assert_eq!(
            pane.push_outbound(snapshot_of(a, "a1")),
            OutboundVerdict::Queued
        );
        assert_eq!(
            pane.push_outbound(snapshot_of(b, "b1")),
            OutboundVerdict::Queued
        );
        assert_eq!(
            pane.push_outbound(snapshot_of(a, "a2")),
            OutboundVerdict::QueuedSuperseding
        );
        let queued: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
        assert_eq!(queued, vec!["b1".to_string(), "a2".to_string()]);
    }

    #[test]
    fn a_burst_of_one_kind_never_reaches_the_cap_or_touches_reliable_state() {
        // The feedback loop this rule breaks: a slow frame's worth of ticks
        // publishing the same snapshot kind over and over. However long the
        // burst, the queue holds one of them, and the cap never has to choose.
        let mut registry = PaneRegistry::new(3);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.mark_live();
        assert_eq!(
            pane.push_outbound(reliable("welcome")),
            OutboundVerdict::Queued
        );
        assert_eq!(pane.push_outbound(snapshot("s0")), OutboundVerdict::Queued);
        for n in 1..50 {
            assert_eq!(
                pane.push_outbound(snapshot(&format!("s{n}"))),
                OutboundVerdict::QueuedSuperseding
            );
            assert_eq!(pane.queued_outbound(), 2);
        }
        let queued: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
        assert_eq!(queued, vec!["welcome".to_string(), "s49".to_string()]);
    }

    #[test]
    fn requeue_reconciles_newer_snapshots_and_reliable_order_within_the_cap() {
        let mut registry = PaneRegistry::new(2);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.mark_live();
        pane.push_outbound(reliable("welcome"));
        pane.push_outbound(snapshot("old"));
        let batch = pane.drain_outbound();

        // These arrive while evaluate_script is working on the drained batch.
        pane.push_outbound(snapshot("new"));
        assert!(!pane.requeue_front(batch));
        assert_eq!(pane.queued_outbound(), 2);
        assert_eq!(
            pane.drain_outbound(),
            vec![reliable("welcome"), snapshot("new")]
        );
    }

    #[test]
    fn requeue_never_merges_a_snapshot_across_a_reliable_barrier_to_avoid_overflow() {
        let mut registry = PaneRegistry::new(2);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.mark_live();
        pane.push_outbound(snapshot("old"));
        pane.push_outbound(reliable("assigned"));
        let batch = pane.drain_outbound();
        pane.push_outbound(snapshot("new"));
        assert!(pane.requeue_front(batch));
        assert_eq!(
            pane.drain_outbound(),
            vec![snapshot("old"), reliable("assigned")]
        );
    }

    #[test]
    fn requeue_reports_overflow_instead_of_shedding_unsuperseded_snapshots() {
        let mut registry = PaneRegistry::new(2);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.mark_live();
        pane.push_outbound(reliable("welcome"));
        pane.push_outbound(snapshot("old"));
        let batch = pane.drain_outbound();

        pane.push_outbound(reliable("assigned"));
        assert!(pane.requeue_front(batch));
        assert_eq!(pane.queued_outbound(), 2);
        assert_eq!(
            pane.drain_outbound(),
            vec![reliable("welcome"), snapshot("old")]
        );
    }

    #[test]
    fn requeue_reports_reliable_overflow_and_keeps_the_oldest_messages_in_order() {
        let mut registry = PaneRegistry::new(2);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.mark_live();
        pane.push_outbound(reliable("welcome"));
        pane.push_outbound(reliable("assigned"));
        let batch = pane.drain_outbound();

        pane.push_outbound(reliable("started"));
        assert!(pane.requeue_front(batch));
        assert_eq!(pane.queued_outbound(), 2);
        assert_eq!(
            pane.drain_outbound(),
            vec![reliable("welcome"), reliable("assigned")]
        );
    }

    #[test]
    fn a_pane_that_is_all_reliable_and_full_reports_rather_than_dropping_state() {
        // Nothing here may be dropped without breaking the page, so the honest
        // answer is a refusal the caller can act on — not a silently lost
        // `StationAssigned`.
        let mut registry = PaneRegistry::new(2);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.push_outbound(reliable("a"));
        pane.push_outbound(reliable("b"));
        assert_eq!(
            pane.push_outbound(snapshot("s")),
            OutboundVerdict::Overflowed
        );
        assert_eq!(
            pane.push_outbound(reliable("c")),
            OutboundVerdict::Overflowed
        );
        assert_eq!(pane.queued_outbound(), 2);
    }

    #[test]
    fn closing_a_pane_reports_the_token_the_lobby_is_owed_a_disconnect_for() {
        let mut registry = PaneRegistry::default();
        let id = registry.open(identity(1));
        let token = registry.get(id).unwrap().token().to_string();
        assert_eq!(registry.close(id), Some((token, Vec::new())));
        assert_eq!(registry.open_count(), 0);
        assert_eq!(
            registry.close(id),
            None,
            "closing twice owes the lobby one disconnect, not two"
        );
    }

    #[test]
    fn a_closed_panes_handle_is_never_reissued() {
        // A message naming a closed pane must be a message for a pane that has
        // gone — never one quietly delivered to whoever inherited the number.
        let mut registry = PaneRegistry::default();
        let first = registry.open(identity(1));
        registry.close(first);
        let second = registry.open(identity(2));
        assert_ne!(first, second);
        assert!(registry.get_mut(first).is_none());
        assert_eq!(
            registry.get(first).map(|p| p.lifecycle()),
            Some(PaneLifecycle::Closed),
            "the record stays, so the id resolves to 'gone' rather than to nothing"
        );
    }

    #[test]
    fn a_document_that_finishes_loading_after_its_pane_closed_does_not_reopen_it() {
        let mut registry = PaneRegistry::default();
        let id = registry.open(identity(1));
        registry.close(id);
        // Reach past `get_mut`'s own guard to prove `mark_live` refuses too:
        // the two are separate defences and only one of them is on the path a
        // late Ultralight load callback takes.
        let pane = registry.panes.iter_mut().find(|p| p.id == id).unwrap();
        pane.mark_live();
        assert_eq!(pane.lifecycle(), PaneLifecycle::Closed);
    }

    #[test]
    fn a_pane_is_found_by_the_token_it_presents_and_a_closed_one_is_not() {
        let mut registry = PaneRegistry::default();
        let id = registry.open(identity(1));
        let token = registry.get(id).unwrap().token().to_string();
        assert_eq!(registry.find_by_token(&token).map(|p| p.id()), Some(id));
        registry.close(id);
        assert!(registry.find_by_token(&token).is_none());
    }
}
