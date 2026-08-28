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

use std::sync::{Arc, Mutex};

use crate::core::codec::{self, JsonCodec, MessageCodec};
use crate::core::messages::ClientMessage;
use crate::native_host::transport::{NativeTransport, TransportDispatch, TransportEvent};

use super::identity::PaneIdentity;
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
    /// Panes that overflowed their reliable budget. A Bevy system reports and
    /// closes them; see [`PaneBus::take_faulted`].
    faulted: Vec<PaneId>,
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

    /// Close a pane and owe the lobby a disconnect for it.
    ///
    /// Anything the page had already said but not yet been polled for is kept
    /// and delivered ahead of the disconnect: it was said while the pane was
    /// still connected, and the lobby's answer to a `ReleaseStation` followed by
    /// a disconnect is not the answer to a disconnect alone.
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
    pub fn take_faulted(&self) -> Vec<PaneId> {
        std::mem::take(&mut self.lock().faulted)
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
            if !state.faulted.contains(&id) {
                state.faulted.push(id);
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
        assert_eq!(bus.take_faulted(), vec![id]);
        assert!(bus.take_faulted().is_empty(), "reported once per overflow");
    }
}
