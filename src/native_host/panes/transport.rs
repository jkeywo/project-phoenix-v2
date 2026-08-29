//! Panes as a [`NativeTransport`] (issue #1122).
//!
//! [`PaneBus`] is the shared handle: the pane registry behind an `Arc<Mutex<_>>`
//! so the Bevy transport (which must be `Send + Sync`) and the Ultralight pane
//! host (which is emphatically neither — `Renderer` and `View` are `!Send`) can
//! both reach it. [`PaneTransport`] is the `NativeTransport` implementation over
//! it, and it is the entirety of how a pane talks to the simulation.
//!
//! # In-process is a shortcut through the codec, not through admission
//!
//! PRD #1093 sanctions exactly one saving for a local participant: *"In-process
//! delivery may avoid network serialisation but cannot bypass command admission
//! or projection boundaries."* Concretely, in this codebase:
//!
//! * **Taken:** a pane's message need not survive a WebRTC DataChannel. It is
//!   still JSON on the way out of the page (the page is a real browser document
//!   and `gui/action-map.js` builds real `ClientMessage` JSON), decoded here by
//!   `core::codec` — the one module allowed to know what JSON is — and handed
//!   over as a typed [`ClientMessage`].
//! * **Not taken:** everything else. The message enters through
//!   `Messages<InboundMessage>`, so `lobby::handler` and
//!   `command_admission::policy` see it exactly as they see a phone's, and the
//!   command log records it exactly as it records a phone's (which is correct: a
//!   human's input is not re-derivable by a replay, and unlike
//!   `command_admission::ai_emit`'s in-process AI emissions it genuinely crossed
//!   a client boundary). Outbound, a pane is named by a `Target` the broadcaster
//!   resolved through `SessionManager::holder_for_station`, and by nothing else
//!   — see [`super::routing`].
//!
//! # Identity is pinned at the bus, not trusted from the page
//!
//! `ClientMessage::Identify` carries a token in its *body*, and
//! `lobby::handler::handle_identify` uses that one — the envelope's is ignored.
//! That is right for a phone, where the token is the participant's own secret.
//! It would be wrong here: a pane's page is handed its token by the host, so a
//! page that presented a different one would either be impersonating another
//! pane or reaching for host authority. [`PaneBus::submit`] refuses both, before
//! the message reaches the seam. The seam's own reserved-token refusal and
//! `handle_identify`'s stay where they are; this is a third gate on a hole the
//! other two do not cover, which is another participant's ordinary token.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::core::codec::{self, JsonCodec, MessageCodec};
use crate::core::messages::ClientMessage;
use crate::delivery::serve::HostedDocuments;
use crate::native_host::transport::{NativeTransport, TransportDispatch, TransportEvent};

use super::document::{mint_document_nonce, pane_document_path, pane_url};
use super::identity::PaneIdentity;
use super::recovery::PaneFault;
use super::registry::{OutboundVerdict, PaneDispatch, PaneId, PaneRegistry};
use super::routing::pane_receives;

/// Why something a page said was not passed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneInputRefusal {
    /// The pane is closed, or was never opened.
    UnknownPane(PaneId),
    /// The page produced something that is not a `ClientMessage`. Carries a
    /// truncated snippet, on the same footing as the browser bridge's own
    /// decode failures.
    Undecodable { snippet: String },
    /// The page presented an `Identify` for a token that is not this pane's.
    Impersonation { presented: String },
}

impl std::fmt::Display for PaneInputRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaneInputRefusal::UnknownPane(id) => write!(f, "{id} is not open"),
            PaneInputRefusal::Undecodable { snippet } => {
                write!(f, "not a client message: {snippet:?}")
            }
            PaneInputRefusal::Impersonation { presented } => write!(
                f,
                "a pane may only identify as itself; it presented {presented:?}"
            ),
        }
    }
}

impl std::error::Error for PaneInputRefusal {}

#[derive(Default)]
struct BusState {
    registry: PaneRegistry,
    /// Events a closed pane left behind, awaiting the next poll: whatever its
    /// page had already said, then the `PlayerDisconnected` the lobby is owed —
    /// which is what keeps the station held and flips it to `Backfill`, the same
    /// treatment a phone that walked out of range gets.
    departing: Vec<TransportEvent>,
    /// Panes that failed since the last poll, each with why (issue #1125). A
    /// Bevy system reports and closes them — and, for a view crash, recreates
    /// them; see [`PaneBus::take_faulted`] and [`super::recovery::service_faults`].
    faulted: Vec<(PaneId, PaneFault)>,
    /// The host's in-memory HTTP publications, so a closed pane's document can
    /// be withdrawn. `None` in a test that never published one.
    documents: Option<HostedDocuments>,
    /// Where each pane's document is published. Keyed by pane, because that is
    /// what closing knows about.
    document_paths: BTreeMap<PaneId, String>,
    /// What a recreated pane's document is rebuilt from (issue #1125): the
    /// connectable host address and the pane document body, armed once by
    /// [`LocalPanes::publish`](super::LocalPanes::publish). `None` in a test that
    /// never published a document — a recreated pane then opens with no URL and
    /// no served page, which is fine, because the session token a reconnect needs
    /// is minted regardless.
    recovery_template: Option<(String, String)>,
    /// Panes opened by [`PaneBus::recreate`] whose Ultralight view has not been
    /// built yet, each with the URL its view should navigate to. Drained once per
    /// frame by the pane host; empty on a host with no `ultralight` feature,
    /// where a recreated pane's station simply stays on Backfill until the
    /// operator repairs it.
    pending_views: Vec<(PaneId, String)>,
}

/// The shared pane registry: cheap to clone, every clone the same panes.
#[derive(Clone, Default)]
pub struct PaneBus {
    state: Arc<Mutex<BusState>>,
}

impl PaneBus {
    /// A bus whose panes each hold at most `outbound_cap` queued messages.
    pub fn with_capacity(outbound_cap: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(BusState {
                registry: PaneRegistry::new(outbound_cap),
                ..Default::default()
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BusState> {
        // A poisoned bus means a pane system panicked mid-frame. Recovering the
        // guard is right rather than cascading: the registry's invariants are
        // per-pane queues, and a half-written queue costs a message rather than
        // corrupting the simulation, which owns none of this state.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open a pane. Creates no session — see [`PaneRegistry::open`].
    pub fn open(&self, identity: PaneIdentity) -> PaneId {
        self.lock().registry.open(identity)
    }

    /// Hand the bus the host's in-memory HTTP publications, so it can withdraw
    /// a closed pane's document.
    ///
    /// `HostedDocuments` is cheap to clone and shared with the serving thread,
    /// so the withdrawal below takes effect on the next request.
    pub fn attach_documents(&self, documents: HostedDocuments) {
        self.lock().documents = Some(documents);
    }

    /// Publish one pane's document and remember where, so [`close`](Self::close)
    /// can take it down again.
    ///
    /// A no-op without a prior [`attach_documents`](Self::attach_documents) —
    /// which is the state of every test that drives panes with no HTTP server.
    pub fn publish_document(&self, id: PaneId, path: String, html: String) {
        let mut state = self.lock();
        if let Some(documents) = &state.documents {
            documents.publish(path.clone(), html);
            state.document_paths.insert(id, path);
        }
    }

    /// Close a pane, owe the lobby a disconnect for it, and stop serving its
    /// document.
    ///
    /// Anything the page had already said but not yet been polled for is kept
    /// and delivered ahead of the disconnect: it was said while the pane was
    /// still connected, and the lobby's answer to a `ReleaseStation` followed by
    /// a disconnect is not the answer to a disconnect alone.
    ///
    /// **The document goes with it.** A pane's path is unguessable and served
    /// only to loopback, but neither of those is a reason to keep publishing a
    /// page for a participant that has gone: its id is never reissued, so the
    /// path can never become live again, and a host that ran for a week would
    /// otherwise accumulate one dead document per closed pane.
    pub fn close(&self, id: PaneId) {
        let mut state = self.lock();
        if let Some((token, pending)) = state.registry.close(id) {
            for msg in pending {
                state.departing.push(TransportEvent::Received {
                    token: token.clone(),
                    msg,
                });
            }
            state.departing.push(TransportEvent::Disconnected { token });
        }
        Self::withdraw(&mut state, id);
    }

    /// Stop serving every pane document this bus published.
    ///
    /// Host shutdown's half of the same rule: the process may outlive the
    /// simulation by as long as it takes the delivery thread to notice, and
    /// nothing should be serving a bridge console in that window.
    pub fn withdraw_all(&self) {
        let mut state = self.lock();
        let ids: Vec<PaneId> = state.document_paths.keys().copied().collect();
        for id in ids {
            Self::withdraw(&mut state, id);
        }
    }

    /// Where a pane's document is published, if it still is. Diagnostic, and
    /// what a test asserts a closed pane no longer has.
    pub fn document_path(&self, id: PaneId) -> Option<String> {
        self.lock().document_paths.get(&id).cloned()
    }

    fn withdraw(state: &mut BusState, id: PaneId) {
        if let Some(path) = state.document_paths.remove(&id) {
            if let Some(documents) = &state.documents {
                documents.withdraw(&path);
            }
        }
    }

    /// Report that a pane's document has finished loading, so pushes may begin.
    pub fn mark_live(&self, id: PaneId) {
        let mut state = self.lock();
        if let Some(pane) = state.registry.get_mut(id) {
            pane.mark_live();
        }
    }

    /// The session token a pane presents, if it is open.
    pub fn token_of(&self, id: PaneId) -> Option<String> {
        self.lock().registry.get(id).map(|p| p.token().to_string())
    }

    /// Every open pane's handle, in the order they were opened.
    pub fn open_pane_ids(&self) -> Vec<PaneId> {
        self.lock().registry.open_panes().map(|p| p.id()).collect()
    }

    /// How many panes are open.
    pub fn open_count(&self) -> usize {
        self.lock().registry.open_count()
    }

    /// Hand the simulation something a page asked for, already decoded.
    pub fn submit(&self, id: PaneId, msg: ClientMessage) -> Result<(), PaneInputRefusal> {
        let mut state = self.lock();
        let pane = state
            .registry
            .get_mut(id)
            .ok_or(PaneInputRefusal::UnknownPane(id))?;
        if let ClientMessage::Identify { token, .. } = &msg {
            if token != pane.token() {
                return Err(PaneInputRefusal::Impersonation {
                    presented: token.clone(),
                });
            }
        }
        pane.push_inbound(msg);
        Ok(())
    }

    /// Hand the simulation one raw JSON record a page produced.
    ///
    /// The decode goes through `core::codec`, the same function the browser
    /// bridge's `drain_inbound` uses, so a pane and a phone cannot disagree
    /// about what a `ClientMessage` is.
    pub fn submit_json(&self, id: PaneId, json: &str) -> Result<(), PaneInputRefusal> {
        let token = self.token_of(id).ok_or(PaneInputRefusal::UnknownPane(id))?;
        let (mut decoded, failures) =
            codec::decode_bridge_client_messages(vec![(token, json.to_string())]);
        if let Some(failure) = failures.into_iter().next() {
            return Err(PaneInputRefusal::Undecodable {
                snippet: failure.payload_snippet,
            });
        }
        let Some((_, msg)) = decoded.pop() else {
            return Ok(());
        };
        self.submit(id, msg)
    }

    /// Take everything queued for a pane's page, in order. Empty while the
    /// pane's document is still loading.
    pub fn take_outbound(&self, id: PaneId) -> Vec<PaneDispatch> {
        let mut state = self.lock();
        state
            .registry
            .get_mut(id)
            .map(|p| p.drain_outbound())
            .unwrap_or_default()
    }

    /// Put an un-delivered batch back at the front of a pane's queue.
    /// See [`super::registry::Pane::requeue_front`].
    pub fn requeue_front(&self, id: PaneId, batch: Vec<PaneDispatch>) {
        let mut state = self.lock();
        if let Some(pane) = state.registry.get_mut(id) {
            pane.requeue_front(batch);
        }
    }

    /// Panes that overflowed their reliable budget since the last call.
    ///
    /// A pane in this list has lost state it cannot recover by waiting: its page
    /// is not draining and the queue is entirely made of one-shot transitions.
    /// Closing it is the honest response — the lobby then treats it exactly as a
    /// phone that dropped, and the station flips to `Backfill` rather than
    /// sitting in front of a page that has quietly stopped agreeing with the
    /// simulation.
    ///
    /// # How a LIVE pane's queue gets there
    ///
    /// This queue only holds what the host has **not handed over yet**, so a
    /// wedged page that kept accepting pushes would never fill it — the messages
    /// would pile up on the far side of the bridge instead, in an unbounded JS
    /// array, and this path would be unreachable on any pane past `Loading`.
    ///
    /// So the page has a cap of its own (`pane_boot.js`), and when it is over
    /// that cap `__phoenixPaneApply` **throws**. `pump_pane` reads a throw as a
    /// `PaneSurfaceError::Script`, stops the batch and requeues it here — which
    /// is what makes this queue grow for a `Live` pane at all. Two frames of
    /// that and a pane carrying nothing but reliable traffic is over budget and
    /// in this list.
    ///
    /// The two caps divide the same problem the way the two sides can each see
    /// it: the page knows it is not draining, and only the host knows which
    /// messages may be dropped to make room.
    ///
    /// Since issue #1125 the list carries a [`PaneFault`] per pane, because a
    /// reliable overflow and a crashed view are both faults but recover
    /// differently — see [`super::recovery::service_faults`].
    pub fn take_faulted(&self) -> Vec<(PaneId, PaneFault)> {
        std::mem::take(&mut self.lock().faulted)
    }

    /// Report that an open pane has failed for `reason` (issue #1125).
    ///
    /// The other end of [`take_faulted`](Self::take_faulted): the pane host calls
    /// this when it *notices* a fault the queue cannot — an Ultralight view that
    /// stopped answering, a lost surface — so that fault rides the same
    /// close → Backfill (→ recreate) path a reliable overflow does. Idempotent
    /// per pane: a view that fails every frame is reported once until serviced.
    /// A closed or unknown pane is ignored — there is nothing left to fail.
    pub fn fault(&self, id: PaneId, reason: PaneFault) {
        let mut state = self.lock();
        if state.registry.get_mut(id).is_none() {
            return;
        }
        if !state.faulted.iter().any(|(existing, _)| *existing == id) {
            state.faulted.push((id, reason));
        }
    }

    /// Arm the bus to rebuild a recreated pane's document (issue #1125).
    ///
    /// Called once by [`LocalPanes::publish`](super::LocalPanes::publish) with
    /// the connectable host address and the pane document body — the two things
    /// [`recreate`](Self::recreate) needs to publish a fresh document and build a
    /// URL for a pane brought back on the same identity.
    pub fn arm_recreation(&self, host_addr: String, document_body: String) {
        self.lock().recovery_template = Some((host_addr, document_body));
    }

    /// Reopen a (closed) pane on the **same identity**, so the human reconnects
    /// as the same participant (issue #1125).
    ///
    /// The in-process analogue of a browser client's automatic redial: a fresh
    /// pane with a new [`PaneId`] but the *same session token*, so its page's
    /// `Identify` is a reconnect the lobby answers by restoring the held station
    /// (reconnect-yield) and pushing the current projection. The old pane's
    /// record — kept `Closed`, never reissued — is where the identity is read
    /// from, so this is safe to call after [`close`](Self::close).
    ///
    /// When the bus was armed ([`arm_recreation`](Self::arm_recreation)) the new
    /// pane's document is published at a fresh nonce'd path and a URL is returned
    /// for its view to navigate to; otherwise the pane opens with an empty URL
    /// (all a seam-level reconnect needs is the token). The new pane is also
    /// queued in [`take_pending_views`](Self::take_pending_views) for the pane
    /// host to build a view for.
    ///
    /// Returns `None` only if `closed_id` names no pane at all.
    pub fn recreate(&self, closed_id: PaneId) -> Option<(PaneId, String)> {
        let mut state = self.lock();
        let identity = state.registry.get(closed_id)?.identity().clone();
        let new_id = state.registry.open(identity.clone());
        let url = match state.recovery_template.clone() {
            Some((host_addr, body)) => {
                let nonce = mint_document_nonce();
                let path = pane_document_path(new_id, &nonce);
                if let Some(documents) = &state.documents {
                    documents.publish(path.clone(), body);
                    state.document_paths.insert(new_id, path);
                }
                pane_url(&host_addr, new_id, &nonce, &identity)
            }
            None => String::new(),
        };
        state.pending_views.push((new_id, url.clone()));
        Some((new_id, url))
    }

    /// Panes recreated since the last call, each with the URL its view should
    /// navigate to (issue #1125). Drained by the pane host, which builds one
    /// Ultralight view per entry.
    pub fn take_pending_views(&self) -> Vec<(PaneId, String)> {
        std::mem::take(&mut self.lock().pending_views)
    }

    /// The participant name of an open pane, if any. The pane host looks a
    /// recreated pane's stored geometry up by name (issue #1125), and the runtime
    /// display watcher maps a lost monitor's pane labels to the panes to fail.
    pub fn name_of(&self, id: PaneId) -> Option<String> {
        self.lock()
            .registry
            .get(id)
            .map(|p| p.identity().name().to_string())
    }

    /// The first open pane joining under `name`, if any (issue #1125).
    ///
    /// The runtime display watcher uses this to turn a lost Station monitor's
    /// pane labels (participant names, from the bridge profile) into the pane
    /// handles whose tokens must disconnect.
    pub fn open_pane_for_name(&self, name: &str) -> Option<PaneId> {
        self.lock()
            .registry
            .open_panes()
            .find(|p| p.identity().name() == name)
            .map(|p| p.id())
    }

    /// A [`NativeTransport`] over these panes.
    pub fn transport(&self) -> PaneTransport {
        PaneTransport { bus: self.clone() }
    }
}

/// The transport half: pane input in, audience-projected output out.
pub struct PaneTransport {
    bus: PaneBus,
}

impl NativeTransport for PaneTransport {
    fn poll(&mut self) -> Vec<TransportEvent> {
        let mut state = self.bus.lock();
        let mut events: Vec<TransportEvent> = Vec::new();
        for pane in state.registry.open_panes_mut() {
            let token = pane.token().to_string();
            for msg in pane.drain_inbound() {
                events.push(TransportEvent::Received {
                    token: token.clone(),
                    msg,
                });
            }
        }
        // Departures last, each with whatever its page said before it went: a
        // pane that spoke and then closed in the same frame spoke while it was
        // still connected, and the lobby's answer depends on that order.
        events.append(&mut state.departing);
        events
    }

    fn dispatch(&mut self, dispatch: TransportDispatch<'_>) {
        let mut state = self.bus.lock();
        // Encode once, not once per pane: the payload is identical and a
        // console snapshot is not small.
        let mut encoded: Option<String> = None;
        let mut faulted: Vec<PaneId> = Vec::new();
        for pane in state.registry.open_panes_mut() {
            if !pane_receives(dispatch.target, pane.token()) {
                continue;
            }
            let json = match &encoded {
                Some(json) => json.clone(),
                None => {
                    // A `ServerMessage` that will not encode is a bug in this
                    // crate, not in the pane; there is nothing useful to hand
                    // the page, and the browser transport drops it too.
                    let Ok(json) = JsonCodec.encode_server(dispatch.msg) else {
                        return;
                    };
                    encoded = Some(json.clone());
                    json
                }
            };
            let verdict = pane.push_outbound(PaneDispatch {
                json,
                delivery: dispatch.delivery,
            });
            if verdict == OutboundVerdict::Overflowed {
                faulted.push(pane.id());
            }
        }
        // Once per pane, not once per overflowed message: a wedged page
        // overflows on every dispatch, and the caller's answer is to close it
        // once.
        for id in faulted {
            if !state.faulted.iter().any(|(existing, _)| *existing == id) {
                state.faulted.push((id, PaneFault::ReliableOverflow));
            }
        }
    }

    fn name(&self) -> &'static str {
        "panes"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{DeliveryClass, ServerMessage};
    use crate::lobby::handler::Target;

    fn identity(n: u8) -> PaneIdentity {
        PaneIdentity::adopt(
            format!("3f1a6c2e-0a11-4b3c-9d55-00000000000{n}"),
            format!("crew-{n}"),
        )
        .unwrap()
    }

    #[test]
    fn what_a_page_says_reaches_the_seam_as_a_typed_message_on_the_panes_own_token() {
        let bus = PaneBus::default();
        let id = bus.open(identity(1));
        let token = bus.token_of(id).unwrap();
        bus.submit_json(
            id,
            &format!(r#"{{"type":"Identify","data":{{"token":"{token}","name":"Ada"}}}}"#),
        )
        .unwrap();
        let events = bus.transport().poll();
        assert_eq!(
            events,
            vec![TransportEvent::Received {
                token,
                msg: ClientMessage::Identify {
                    token: bus.token_of(id).unwrap(),
                    name: "Ada".to_string()
                }
            }]
        );
    }

    #[test]
    fn a_pane_cannot_identify_as_the_host_operator() {
        // The third gate. The seam refuses `__local_console__` in the ENVELOPE
        // and `handle_identify` refuses it in the BODY — but a pane that
        // presented it would still have got as far as the seam, and the seam
        // sees the pane's own token there. Refusing at the bus means the
        // message never leaves the pane at all.
        let bus = PaneBus::default();
        let id = bus.open(identity(1));
        let err = bus
            .submit(
                id,
                ClientMessage::Identify {
                    token: crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
                    name: "impostor".to_string(),
                },
            )
            .expect_err("a pane may only identify as itself");
        assert!(matches!(err, PaneInputRefusal::Impersonation { .. }));
        assert!(bus.transport().poll().is_empty());
    }

    #[test]
    fn a_pane_cannot_identify_as_another_pane() {
        // The hole the other two gates do not cover: another participant's
        // ordinary, entirely un-reserved token.
        let bus = PaneBus::default();
        let mine = bus.open(identity(1));
        let theirs = bus.open(identity(2));
        let their_token = bus.token_of(theirs).unwrap();
        assert_eq!(
            bus.submit(
                mine,
                ClientMessage::Identify {
                    token: their_token.clone(),
                    name: "impostor".to_string()
                }
            ),
            Err(PaneInputRefusal::Impersonation {
                presented: their_token
            })
        );
    }

    #[test]
    fn a_targeted_projection_reaches_only_the_pane_it_names() {
        // The acceptance criterion, at the transport: a pane cannot read
        // another pane's audience projection. `Audience::Holding(station)` has
        // already resolved to this `Target::Token` by the time it arrives.
        let bus = PaneBus::default();
        let helm = bus.open(identity(1));
        let comms = bus.open(identity(2));
        bus.mark_live(helm);
        bus.mark_live(comms);
        let helm_token = bus.token_of(helm).unwrap();

        bus.transport().dispatch(TransportDispatch {
            target: &Target::Token(helm_token),
            msg: &ServerMessage::GameStarted,
            delivery: DeliveryClass::Reliable,
        });

        assert_eq!(bus.take_outbound(helm).len(), 1);
        assert!(
            bus.take_outbound(comms).is_empty(),
            "a pane receives its own audience's projections and no others"
        );
    }

    #[test]
    fn a_broadcast_reaches_every_open_pane_and_no_closed_one() {
        let bus = PaneBus::default();
        let a = bus.open(identity(1));
        let b = bus.open(identity(2));
        bus.mark_live(a);
        bus.mark_live(b);
        bus.close(b);
        bus.transport().dispatch(TransportDispatch {
            target: &Target::All,
            msg: &ServerMessage::GameStarted,
            delivery: DeliveryClass::Reliable,
        });
        assert_eq!(bus.take_outbound(a).len(), 1);
        assert!(bus.take_outbound(b).is_empty());
    }

    #[test]
    fn closing_a_pane_owes_the_lobby_exactly_one_disconnect() {
        let bus = PaneBus::default();
        let id = bus.open(identity(1));
        let token = bus.token_of(id).unwrap();
        bus.close(id);
        bus.close(id);
        assert_eq!(
            bus.transport().poll(),
            vec![TransportEvent::Disconnected { token }]
        );
        assert!(bus.transport().poll().is_empty(), "and only once");
    }

    #[test]
    fn a_pane_that_speaks_and_then_closes_in_one_frame_is_heard_before_it_disconnects() {
        // Order matters to the lobby: a `ReleaseStation` followed by a
        // disconnect vacates the seat; the reverse order re-seats a participant
        // who has gone.
        let bus = PaneBus::default();
        let id = bus.open(identity(1));
        bus.submit(id, ClientMessage::ReleaseStation).unwrap();
        bus.close(id);
        let events = bus.transport().poll();
        assert!(matches!(events[0], TransportEvent::Received { .. }));
        assert!(matches!(events[1], TransportEvent::Disconnected { .. }));
    }

    #[test]
    fn a_page_that_produces_nonsense_is_refused_with_a_snippet_rather_than_panicking() {
        let bus = PaneBus::default();
        let id = bus.open(identity(1));
        let err = bus.submit_json(id, "not json at all").unwrap_err();
        assert!(matches!(err, PaneInputRefusal::Undecodable { .. }));
        assert!(bus.transport().poll().is_empty());
    }

    #[test]
    fn closing_a_pane_stops_serving_its_document() {
        // `HostedDocuments::withdraw` had no production caller at all: a pane
        // closed by a fault left its page published for the rest of the
        // process's life, and a host that ran a long session accumulated one
        // dead console per closed pane.
        let bus = PaneBus::default();
        let documents = HostedDocuments::default();
        let a = bus.open(identity(1));
        let b = bus.open(identity(2));
        bus.attach_documents(documents.clone());
        bus.publish_document(a, "/client/pane-0-aaaa.html".to_string(), "a".to_string());
        bus.publish_document(b, "/client/pane-1-bbbb.html".to_string(), "b".to_string());
        assert_eq!(documents.len(), 2);

        bus.close(a);
        assert_eq!(documents.get("/client/pane-0-aaaa.html"), None);
        assert_eq!(bus.document_path(a), None);
        assert_eq!(
            documents.get("/client/pane-1-bbbb.html"),
            Some("b".to_string()),
            "and only the closed pane's"
        );

        // Host shutdown takes the rest, before the delivery thread is joined.
        bus.withdraw_all();
        assert!(documents.is_empty());
    }

    #[test]
    fn a_bus_with_no_documents_attached_closes_panes_exactly_as_before() {
        // Every test that drives panes without an HTTP server, which is most of
        // them: publishing is a no-op and closing must not care.
        let bus = PaneBus::default();
        let id = bus.open(identity(1));
        bus.publish_document(id, "/client/pane-0-aaaa.html".to_string(), "a".to_string());
        assert_eq!(bus.document_path(id), None);
        bus.close(id);
        assert_eq!(bus.open_count(), 0);
    }

    #[test]
    fn a_pane_whose_page_stopped_draining_is_reported_as_faulted() {
        // Every queued message is reliable, so nothing may be dropped. The
        // caller closes the pane rather than letting the page drift out of
        // agreement with the simulation behind a clean log.
        let bus = PaneBus::with_capacity(1);
        let id = bus.open(identity(1));
        bus.mark_live(id);
        let mut transport = bus.transport();
        for _ in 0..3 {
            transport.dispatch(TransportDispatch {
                target: &Target::All,
                msg: &ServerMessage::GameStarted,
                delivery: DeliveryClass::Reliable,
            });
        }
        assert_eq!(bus.take_faulted(), vec![(id, PaneFault::ReliableOverflow)]);
        assert!(bus.take_faulted().is_empty(), "reported once per overflow");
    }
}
