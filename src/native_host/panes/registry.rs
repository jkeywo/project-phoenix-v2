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
//! So the cap is per delivery class, which is the same distinction the browser
//! transport already makes with its two DataChannels: **`Snapshot` messages are
//! droppable** (the next one supersedes them entirely) and **`Reliable` ones are
//! not**. Over the cap, the oldest *snapshot* goes; if there are none, the pane
//! is over its reliable budget and that is a fault worth reporting rather than
//! hiding, so [`Pane::push_outbound`] says so.

use std::collections::VecDeque;

use crate::core::messages::{ClientMessage, DeliveryClass};

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
    /// Closed. Its session is the lobby's business now — the same disconnect
    /// path a phone that walked out of range takes, which keeps the station
    /// held and flips it to `Backfill`.
    Closed,
}

/// One outbound message waiting for its page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneDispatch {
    /// The encoded `ServerMessage`, in the same JSON a phone would receive.
    pub json: String,
    /// Which channel it would have ridden on a network transport.
    pub delivery: DeliveryClass,
}

/// What happened to a message handed to a full pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboundVerdict {
    /// Queued with room to spare.
    Queued,
    /// Queued, and an older snapshot was dropped to make room. Ordinary under
    /// load: the next snapshot supersedes the one that went.
    QueuedDroppingSnapshot,
    /// Refused. The pane is over its budget and every message in the queue is
    /// reliable, so nothing may be dropped without breaking the page's state.
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

    /// Queue something the page asked for.
    pub fn push_inbound(&mut self, msg: ClientMessage) {
        self.inbound.push_back(msg);
    }

    /// Take everything the page has asked for since the last call, in order.
    pub fn drain_inbound(&mut self) -> Vec<ClientMessage> {
        self.inbound.drain(..).collect()
    }

    /// Queue something for the page. See the module note for the cap policy.
    pub fn push_outbound(&mut self, dispatch: PaneDispatch) -> OutboundVerdict {
        if self.outbound.len() < self.outbound_cap {
            self.outbound.push_back(dispatch);
            return OutboundVerdict::Queued;
        }
        let oldest_snapshot = self
            .outbound
            .iter()
            .position(|d| d.delivery == DeliveryClass::Snapshot);
        match oldest_snapshot {
            Some(index) => {
                self.outbound.remove(index);
                self.outbound.push_back(dispatch);
                OutboundVerdict::QueuedDroppingSnapshot
            }
            None => OutboundVerdict::Overflowed,
        }
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
    /// Deliberately not capped: this is a batch that was already inside the cap
    /// a moment ago, and refusing it here would drop precisely the messages the
    /// cap policy protects.
    pub fn requeue_front(&mut self, batch: Vec<PaneDispatch>) {
        for dispatch in batch.into_iter().rev() {
            self.outbound.push_front(dispatch);
        }
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

    /// One pane by handle, mutably. Closed panes are **not** returned: nothing
    /// may queue traffic for a pane that has gone.
    pub fn get_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.panes
            .iter_mut()
            .find(|p| p.id == id && p.lifecycle != PaneLifecycle::Closed)
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
        PaneDispatch {
            json: tag.to_string(),
            delivery: DeliveryClass::Snapshot,
        }
    }

    fn reliable(tag: &str) -> PaneDispatch {
        PaneDispatch {
            json: tag.to_string(),
            delivery: DeliveryClass::Reliable,
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
    fn an_overfull_pane_drops_its_oldest_snapshot_and_keeps_every_reliable_message() {
        // The distinction the browser transport draws with its two
        // DataChannels: a snapshot is superseded by the next one, a state
        // transition is not.
        let mut registry = PaneRegistry::new(3);
        let id = registry.open(identity(1));
        let pane = registry.get_mut(id).unwrap();
        pane.mark_live();
        assert_eq!(
            pane.push_outbound(reliable("welcome")),
            OutboundVerdict::Queued
        );
        assert_eq!(pane.push_outbound(snapshot("s1")), OutboundVerdict::Queued);
        assert_eq!(pane.push_outbound(snapshot("s2")), OutboundVerdict::Queued);
        assert_eq!(
            pane.push_outbound(snapshot("s3")),
            OutboundVerdict::QueuedDroppingSnapshot
        );
        let queued: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
        assert_eq!(
            queued,
            vec!["welcome".to_string(), "s2".to_string(), "s3".to_string()],
            "the oldest snapshot went; the reliable message stayed"
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
