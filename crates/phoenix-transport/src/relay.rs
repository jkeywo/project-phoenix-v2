//! Relay admission, reconnection, delivery classes and connection ownership.
//! The application supplies only its codec, identity extraction and compatibility policy.
use crate::codec::{
    decode_handshake_frame, decode_rendezvous_frame, encode_handshake_frame,
    encode_rendezvous_frame,
};
use crate::connections::{BindRefusal, ConnectionId, IdentityPolicy};
use crate::rendezvous::{
    HandshakeFrame, JoinCode, RelayLimits, RendezvousFrame, CLASS_RELIABLE, CLASS_SNAPSHOT,
    JOIN_HANDSHAKE, RENDEZVOUS_PROTOCOL,
};
use crate::shared_connections::{ConnectionLeg, SharedConnections};
use crate::socket::RelaySocket;
use crate::{DeliveryClass, Dispatch, Event, Profile, Target, Transport};
use std::collections::HashMap;

pub struct CompatibilityRefusal {
    pub code: String,
    pub detail: String,
}
pub trait RelayProtocol: Profile<Connections = SharedConnections<Self::Identity>> {
    type Identity: IdentityPolicy;
    type Stamp: Send + Sync;
    fn check_stamp(stamp: &Self::Stamp, peer: Option<&str>) -> Result<(), CompatibilityRefusal>;
    fn decode_client(payload: &str) -> Result<Self::Inbound, String>;
    fn identity(message: &Self::Inbound) -> Option<&str>;
    fn encode_server(message: &Self::Outbound) -> Result<String, String>;
}

/// What a native host tells the service about itself when it registers.
pub struct RelayHostConfig<S> {
    /// Which typed code namespace to be issued in — `client` for crew.
    pub namespace: String,
    /// The release GUID the code is registered under. `None` lets the service
    /// use its own bundled table's version, which is what a host built from
    /// the same checkout wants.
    pub version: Option<String>,
    /// This host's own delivery stamp, the authority every joiner's
    /// compatibility handshake is answered from.
    pub stamp: S,
}

/// The `JoinRefused` code a peer gets for claiming a token only the host
/// runtime may use. Not a [`crate::delivery::stamp::StampMismatch`] code — this
/// is not a verdict about the joiner's BUILD — so it is spelled here and mapped
/// to its own sentence in `gui/join-code.js`'s `REASON_STRING_IDS`.
pub const RESERVED_TOKEN_CODE: &str = "reserved-token";

/// A malformed or changed connection identity cannot be repaired by retrying
/// the same saved token. The browser treats this refusal as terminal.
pub const INVALID_TOKEN_CODE: &str = "invalid-token";

/// One crew member the service is carrying for us.
struct RelayPeer {
    connection: ConnectionId,
    /// True once the compatibility handshake admitted this build.
    admitted: bool,
    /// True once refused: nothing it sends afterwards may reach the simulation.
    refused: bool,
}

/// A [`NativeTransport`] whose wire is the rendezvous service's game relay.
pub struct RelayTransport<P: RelayProtocol> {
    connections: ConnectionLeg<P::Identity>,
    socket: Box<dyn RelaySocket>,
    config: RelayHostConfig<P::Stamp>,
    limits: RelayLimits,
    peers: HashMap<String, RelayPeer>,
    /// The code the service issued, once it has. What the operator reads out.
    code: Option<JoinCode>,
    /// True once `host-open` has been sent, so a re-`ready` cannot register a
    /// second record and mint a second code while the first is live.
    registered: bool,
    /// Snapshot frames shed for backpressure, over this transport's whole life.
    /// A diagnostics counter, not a control: nothing branches on it.
    shed_snapshots: u64,
    /// False while the socket is down. Held so the teardown runs ONCE on the
    /// way down and a redialed socket (see [`crate::native_host::relay_socket`])
    /// is noticed on the way back up instead of being polled forever as dead.
    link_up: bool,
    /// Peers whose link failed inside [`NativeTransport::dispatch`], which
    /// cannot emit events. Drained by the next [`NativeTransport::poll`].
    pending_drops: Vec<(String, ConnectionId)>,
    /// Anything the operator should see, drained by the host each frame.
    notices: RelayNotices,
    profile: std::marker::PhantomData<fn() -> P>,
}

/// The operator's end of a [`RelayTransport`]'s notice queue.
///
/// A shared handle rather than a method on the transport, and for a concrete
/// reason: once the transport is inside a `NativeTransportLink` it is a
/// `Box<dyn NativeTransport>`, and getting a `RelayTransport` back out of one
/// would mean either a downcast (a new `Any` bound on a trait that does not
/// need one) or a second resource. This is the second resource, and it is the
/// same shape [`crate::native_host::transport::LoopbackHandle`] already uses
/// for the same reason.
#[derive(Clone, Default)]
pub struct RelayNotices(std::sync::Arc<std::sync::Mutex<Vec<RelayNotice>>>);

impl RelayNotices {
    /// Take everything queued since the last drain, oldest first.
    pub fn drain(&self) -> Vec<RelayNotice> {
        self.0
            .lock()
            .expect("relay notices poisoned")
            .drain(..)
            .collect()
    }

    fn push(&self, notice: RelayNotice) {
        self.0.lock().expect("relay notices poisoned").push(notice);
    }
}

/// Something worth telling the operator, drained by whoever owns the transport.
///
/// The transport cannot log for itself: `plog!` takes an
/// `Option<Res<LogFilterConfig>>`, which is a Bevy system parameter, and this
/// object is a resource that Bevy calls rather than a system. So it reports,
/// and whoever owns the transport drains it — today that is
/// `report_relay_notices` in `src/bin/phoenix_host.rs`, which writes them onto
/// the host binary's own operator log beside the join code it prints.
#[derive(Clone, Debug, PartialEq)]
pub enum RelayNotice {
    /// The service issued a join code. The code goes on the viewscreen.
    Coded(JoinCode),
    /// A joiner was refused by the compatibility handshake.
    Refused { peer: String, code: String },
    /// The service, or the link to it, said something went wrong.
    Fault { reason: String },
    /// Snapshot frames were shed to a peer because the socket is behind.
    Shedding { peer: String, total: u64 },
}

impl<P: RelayProtocol> RelayTransport<P> {
    /// Build a transport over `socket` and register as a host at once.
    pub fn new(socket: impl RelaySocket, config: RelayHostConfig<P::Stamp>) -> Self {
        Self {
            connections: ConnectionLeg::default(),
            socket: Box::new(socket),
            config,
            profile: std::marker::PhantomData,
            limits: RelayLimits::default(),
            peers: HashMap::new(),
            code: None,
            registered: false,
            shed_snapshots: 0,
            link_up: true,
            pending_drops: Vec::new(),
            notices: RelayNotices::default(),
        }
    }

    /// The operator's end of this transport's notice queue. Clone it before
    /// handing the transport to `NativeTransportLink`; that is the end a host
    /// reads the issued join code and every fault out of.
    pub fn notices(&self) -> RelayNotices {
        self.notices.clone()
    }

    /// Report into `notices` rather than into this transport's own queue
    /// (issue #1353).
    ///
    /// A host can now hold TWO of these — the cloud relay and the in-process
    /// direct-accept service — while `NativeTransportLink` is one resource and
    /// `RelayNotices` is one resource. That is not an accident of Bevy: the
    /// queue is a channel to the OPERATOR, and there is one operator with one
    /// terminal. Call this on the second leg, right after building it, so both
    /// legs' codes, refusals and faults arrive in the order they happened.
    pub fn share_notices(&mut self, notices: RelayNotices) {
        self.notices = notices;
    }

    /// The join code the service issued, if it has yet.
    pub fn code(&self) -> Option<&JoinCode> {
        self.code.as_ref()
    }

    /// Take everything the operator should be told since the last drain.
    pub fn drain_notices(&mut self) -> Vec<RelayNotice> {
        self.notices.drain()
    }

    /// How many crew members the service is currently carrying.
    pub fn relayed_peers(&self) -> usize {
        self.peers.values().filter(|p| p.admitted).count()
    }

    /// Bytes the underlying service is holding, un-written, across every peer —
    /// the same total the transport sheds snapshots against. Exposed for
    /// operator diagnostics and so a test can watch the reliable-backpressure
    /// ceiling hold under a stalled peer (issue #1354).
    pub fn buffered_bytes(&self) -> usize {
        self.socket.buffered_bytes()
    }

    fn send_frame(&mut self, frame: &RendezvousFrame) {
        match encode_rendezvous_frame(frame) {
            Ok(text) => self.socket.send(text),
            Err(e) => self.notices.push(RelayNotice::Fault {
                reason: format!("could not encode a {} frame: {e}", frame.kind),
            }),
        }
    }

    /// Put one already-encoded game payload on the relay, for `peer`.
    fn send_payload(&mut self, peer: &str, class: &str, payload: String) {
        let frame = RendezvousFrame::relay(peer, class, payload);
        self.send_frame(&frame);
    }

    /// Answer one joiner's compatibility handshake.
    ///
    /// The same authority the browser host asks — `delivery::check_join_stamp`,
    /// through `wasm_check_client_stamp` there and directly here — so a build
    /// this host would refuse in a browser is refused here for the same reason
    /// and with the same machine code on the phone's screen.
    fn answer_handshake(&mut self, peer: &str, payload: &str) {
        let stamp = decode_handshake_frame(payload)
            .ok()
            .and_then(|f| f.data.stamp);
        let verdict = P::check_stamp(&self.config.stamp, stamp.as_deref());
        let (frame, admitted, refusal) = match verdict {
            Ok(()) => (HandshakeFrame::accepted(), true, None),
            Err(mismatch) => (
                HandshakeFrame::refused(mismatch.code.as_str(), mismatch.detail.clone()),
                false,
                Some(mismatch.code.as_str().to_string()),
            ),
        };
        if let Some(entry) = self.peers.get_mut(peer) {
            entry.admitted = admitted;
            entry.refused = !admitted;
        }
        if let Some(code) = refusal {
            self.notices.push(RelayNotice::Refused {
                peer: peer.to_string(),
                code,
            });
        }
        match encode_handshake_frame(&frame) {
            Ok(text) => self.send_payload(peer, CLASS_RELIABLE, text),
            Err(e) => self.notices.push(RelayNotice::Fault {
                reason: format!("could not encode a join verdict: {e}"),
            }),
        }
    }

    /// One relayed game payload from `peer`.
    fn on_game_payload(&mut self, peer: &str, payload: &str, out: &mut Vec<Event<P::Inbound>>) {
        let Some(entry) = self.peers.get(peer) else {
            return;
        };
        if entry.refused {
            return;
        }
        if !entry.admitted {
            // Nothing but the compatibility handshake exists on this link yet.
            // A build the host is about to refuse has no business queuing
            // simulation traffic, so anything else is DROPPED rather than
            // buffered — exactly what the browser host does.
            if decode_handshake_frame(payload)
                .map(|f| f.kind == JOIN_HANDSHAKE)
                .unwrap_or(false)
            {
                self.answer_handshake(peer, payload);
            }
            return;
        }

        let Ok(msg) = P::decode_client(payload) else {
            self.notices.push(RelayNotice::Fault {
                reason: format!("undecodable client message from {peer}"),
            });
            return;
        };

        let connection = entry.connection;
        if let Some(token) = P::identity(&msg) {
            let verdict = self.connections.shared.lock().bind(connection, token);
            if let Err(refusal) = verdict {
                let code = if refusal == BindRefusal::ReservedToken {
                    RESERVED_TOKEN_CODE
                } else {
                    INVALID_TOKEN_CODE
                };
                self.refuse_peer(
                    peer,
                    code,
                    format!("connection identity refused: {refusal:?}"),
                );
                return;
            }
        }
        let Some(token) = self
            .connections
            .shared
            .lock()
            .sender(connection)
            .map(str::to_owned)
        else {
            return;
        };
        out.push(Event::Received { token, msg });
    }

    /// Refuse this peer at the transport: tell it why in the same in-band
    /// `JoinRefused` the compatibility handshake uses, mark it refused so
    /// nothing it sends afterwards reaches the simulation, and stop carrying
    /// it. It is out of `audience()` at once. The next poll disconnects an
    /// already identified owner; a never-identified or stale link owes nothing.
    fn refuse_peer(&mut self, peer: &str, code: &str, detail: String) {
        self.notices.push(RelayNotice::Refused {
            peer: peer.to_string(),
            code: code.to_string(),
        });
        match encode_handshake_frame(&HandshakeFrame::refused(code, detail)) {
            Ok(text) => self.send_payload(peer, CLASS_RELIABLE, text),
            Err(e) => self.notices.push(RelayNotice::Fault {
                reason: format!("could not encode a refusal: {e}"),
            }),
        }
        if let Some(entry) = self.peers.get_mut(peer) {
            entry.refused = true;
            entry.admitted = false;
            self.pending_drops
                .push((peer.to_string(), entry.connection));
        }
    }

    /// Only departure of the host-wide current incarnation disconnects a
    /// Session. A superseded link's close is silent across transport legs too.
    fn drop_peer(&mut self, peer: &str, out: &mut Vec<Event<P::Inbound>>) {
        let Some(entry) = self.peers.remove(peer) else {
            return;
        };
        if let Some(token) = self.connections.shared.lock().close(entry.connection) {
            out.push(Event::Disconnected { token });
        }
    }

    fn on_frame(&mut self, frame: RendezvousFrame, out: &mut Vec<Event<P::Inbound>>) {
        if frame.v != RENDEZVOUS_PROTOCOL {
            self.notices.push(RelayNotice::Fault {
                reason: format!(
                    "the service speaks rendezvous v{} and this build speaks v{RENDEZVOUS_PROTOCOL}",
                    frame.v
                ),
            });
            return;
        }
        match frame.kind.as_str() {
            "ready" => {
                if self.registered {
                    return;
                }
                self.registered = true;
                let open = RendezvousFrame::host_open(
                    &self.config.namespace.clone(),
                    self.config.version.as_deref(),
                );
                self.send_frame(&open);
            }
            "hosted" => {
                // Only an ISSUED code, never a typed one: `code` carries both
                // shapes on the wire (see `CodeField`), and a host that read a
                // client's typed string as its own issued identifier would put
                // somebody else's code on its viewscreen.
                if let Some(code) = frame.code.as_ref().and_then(|c| c.issued()).cloned() {
                    self.code = Some(code.clone());
                    self.notices.push(RelayNotice::Coded(code));
                }
            }
            "relay-peer" => {
                if let Some(peer) = frame.peer {
                    if let Some(limits) = frame.limits {
                        // The service's authored numbers beat this build's
                        // conservative defaults, so a designer retuning them in
                        // assets/join/join-codes.toml does not need a release.
                        self.limits = limits;
                    }
                    // Duplicate announcements are idempotent. Peer ids belong
                    // to this socket; a later incarnation gets a fresh handle.
                    if self.peers.contains_key(&peer) {
                        return;
                    }
                    let connection = self.connections.shared.lock().open(self.connections.id);
                    self.peers.insert(
                        peer,
                        RelayPeer {
                            connection,
                            admitted: false,
                            refused: false,
                        },
                    );
                }
            }
            "relay" => {
                if let (Some(from), Some(payload)) = (frame.from, frame.payload) {
                    self.on_game_payload(&from, &payload, out);
                }
            }
            "relay-peer-left" => {
                if let Some(peer) = frame.peer {
                    self.drop_peer(&peer, out);
                }
            }
            "relay-closed" => {
                // The service has stopped carrying us. Every relayed crew
                // member is now unreachable: unlike a DataChannel, a relayed
                // link does not outlive the service that introduced it.
                let reason = frame.reason.unwrap_or_else(|| "unreachable".to_string());
                self.lose_relay(reason, out);
            }
            "error" => {
                // NOT the same thing, and folding the two was a crew-wide
                // outage on every ordinary departure. `error` is `registry.js`'s
                // generic per-REQUEST refusal (`fail(connId, request, reason)`):
                // a host broadcasting snapshots to `Target::All` addresses a
                // peer the service detached a tick ago and is answered
                // `relay/no-peer`, every single time somebody closes their
                // phone. Treating that as total relay loss disconnected
                // everybody else and wiped the join code.
                //
                // So a refusal is a NOTICE and nothing more — matching
                // `createRendezvousHost`'s `case 'error'`, which only tears
                // down for `unreachable`. Nothing is dropped for it either: the
                // frame names the request, never the peer, so there is no
                // subject to drop even when the reason is `no-peer`. The peer
                // that really went arrives as `relay-peer-left` a moment later,
                // which is the frame that DOES name it.
                let reason = frame.reason.unwrap_or_else(|| "unreachable".to_string());
                if Self::is_terminal_error(&reason) {
                    self.lose_relay(reason, out);
                    return;
                }
                let request = frame.request.unwrap_or_else(|| "unknown".to_string());
                self.notices.push(RelayNotice::Fault {
                    reason: format!("the service refused a {request} frame: {reason}"),
                });
            }
            "relay-degraded" => {
                // The service shedding snapshot frames it could not hand to a
                // peer. Reported as the service's own cumulative total for that
                // mailbox, NOT folded into `shed_snapshots`: that counter is
                // what this host shed against its own send buffer, and adding
                // two different measurements of two different queues together
                // would give the operator a number that means nothing.
                self.notices.push(RelayNotice::Shedding {
                    peer: frame.peer.unwrap_or_default(),
                    total: u64::from(frame.dropped.unwrap_or(0)),
                });
            }
            _ => {}
        }
    }

    /// Reasons on an `error` frame that are about the LINK rather than one
    /// request, and so really do end every relayed session.
    ///
    /// `unreachable` is the registry's record-is-gone answer, `not-connected`
    /// says the service is not holding this socket at all, and a protocol
    /// refusal means every subsequent frame gets the same answer. Everything
    /// else — `no-peer`, `not-relaying`, `relay-too-large`, `malformed`,
    /// `relay-full`, `forbidden-role` — is a refusal of ONE request.
    fn is_terminal_error(reason: &str) -> bool {
        matches!(
            reason,
            "unreachable" | "not-connected" | "unsupported-protocol"
        )
    }

    /// The relay itself is gone: report every identified crew member as
    /// disconnected, forget the code, and allow a future `ready` to register
    /// again (a reconnected socket re-sends `host-open` and is issued a fresh
    /// code, exactly as the browser host's `lostService()` does).
    fn lose_relay(&mut self, reason: String, out: &mut Vec<Event<P::Inbound>>) {
        for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
            self.drop_peer(&peer, out);
        }
        self.code = None;
        self.registered = false;
        self.notices.push(RelayNotice::Fault { reason });
    }

    /// Retire replaced links without publishing a Session disconnect.
    /// Other legs observe replacement on their next poll/dispatch; the registry
    /// has already stopped accepting their traffic at the binding boundary.
    fn retire_superseded(&mut self) {
        let stale: Vec<_> = {
            let registry = self.connections.shared.lock();
            self.peers
                .iter()
                .filter(|(_, peer)| registry.is_superseded(peer.connection))
                .map(|(id, peer)| (id.clone(), peer.connection))
                .collect()
        };
        for (peer, connection) in stale {
            self.send_frame(&RendezvousFrame::relay_close(&peer));
            self.peers.remove(&peer);
            assert!(self.connections.shared.lock().close(connection).is_none());
        }
    }

    /// Resolve audiences through the host-wide owner, then select this leg's links.
    fn audience(&self, target: &Target) -> Vec<String> {
        let recipients = self.connections.shared.lock().recipients(target);
        self.peers
            .iter()
            .filter(|(_, p)| p.admitted && recipients.contains(&p.connection))
            .map(|(id, _)| id.clone())
            .collect()
    }
}

impl<P: RelayProtocol> Transport<P> for RelayTransport<P> {
    fn share_connections(&mut self, connections: SharedConnections<P::Identity>) {
        assert!(
            self.peers.is_empty(),
            "compose transports before accepting crew"
        );
        self.connections = ConnectionLeg::new(connections);
    }

    fn poll(&mut self) -> Vec<Event<P::Inbound>> {
        self.retire_superseded();
        let mut out = Vec::new();
        // Links this transport gave up on while dispatching, where it had no
        // way to say so (see `dispatch`).
        for (peer, connection) in std::mem::take(&mut self.pending_drops) {
            if self
                .peers
                .get(&peer)
                .is_some_and(|entry| entry.connection == connection)
            {
                self.drop_peer(&peer, &mut out);
            }
        }
        // Consume lifecycle edges before consulting the current level: the
        // socket may have redialled completely between two simulation polls.
        for event in self.socket.poll_events() {
            match event {
                crate::socket::RelaySocketEvent::Opened => self.link_up = true,
                crate::socket::RelaySocketEvent::Closed => {
                    if self.link_up {
                        self.link_up = false;
                        self.lose_relay("unreachable".to_string(), &mut out);
                    }
                }
                crate::socket::RelaySocketEvent::Text(text) => {
                    // Non-redialling adapters supply frames and a level only.
                    if !self.link_up && self.socket.is_open() {
                        self.link_up = true;
                    }
                    if self.link_up && self.socket.is_open() {
                        match decode_rendezvous_frame(&text) {
                            Ok(frame) => self.on_frame(frame, &mut out),
                            Err(_) => self.notices.push(RelayNotice::Fault {
                                reason: "undecodable frame from the rendezvous service".to_string(),
                            }),
                        }
                    }
                }
            }
        }
        if !self.socket.is_open() && self.link_up {
            self.link_up = false;
            self.lose_relay("unreachable".to_string(), &mut out);
        }
        self.retire_superseded();
        out
    }

    fn dispatch(&mut self, dispatch: Dispatch<'_, P::Outbound>) {
        self.retire_superseded();
        let targets = self.audience(dispatch.target);
        if targets.is_empty() {
            return;
        }
        let Ok(payload) = P::encode_server(dispatch.msg) else {
            self.notices.push(RelayNotice::Fault {
                reason: "could not encode an outbound message".to_string(),
            });
            return;
        };
        // Over the authored ceiling, measured the same way the service measures
        // it. What happens next depends on the CLASS, because the two classes
        // promise different things:
        //
        //   snapshot  a lost frame is what the class is for. Count it as shed,
        //             say so, and let the next tick supersede it.
        //   reliable  there is no such thing as a quietly dropped reliable
        //             frame — the simulation is written against the guarantee.
        //             So the LINK fails instead, and the affected crew members
        //             re-join (their stations flip to Backfill meanwhile),
        //             which is the same answer gui/rendezvous-relay.js gives.
        if payload.len() > self.limits.max_frame_bytes {
            let over = format!(
                "a {} byte message does not fit the relay's {} byte frame",
                payload.len(),
                self.limits.max_frame_bytes
            );
            match dispatch.delivery {
                DeliveryClass::Snapshot => {
                    self.shed_snapshots += targets.len() as u64;
                    self.notices.push(RelayNotice::Shedding {
                        peer: targets.first().cloned().unwrap_or_default(),
                        total: self.shed_snapshots,
                    });
                    self.notices.push(RelayNotice::Fault { reason: over });
                }
                DeliveryClass::Reliable => {
                    self.notices.push(RelayNotice::Fault {
                        reason: format!("{over}: ending the relayed links it was for"),
                    });
                    // Tell the service to detach each peer before this host
                    // drops it locally. Left one-sided, the service would keep
                    // the peer's mailbox open and the phone would sit on a
                    // status line still reading "connected" while this host
                    // had already walked away — mirrors `refuse_peer`'s
                    // in-band notice above, and `gui/rendezvous-transport.js`'s
                    // `onFailure`.
                    for peer in &targets {
                        self.send_frame(&RendezvousFrame::relay_close(peer));
                    }
                    self.pending_drops
                        .extend(targets.into_iter().filter_map(|peer| {
                            self.peers
                                .get(&peer)
                                .map(|entry| (peer.clone(), entry.connection))
                        }));
                }
            }
            return;
        }

        // The lossy class staying lossy over a transport that is not. A
        // WebSocket is reliable and ordered, so queuing a snapshot behind an
        // existing backlog is exactly the head-of-line blocking the class
        // exists to avoid — and the next tick supersedes it. Reliable frames
        // are never shed here; a dropped command would break the guarantee the
        // simulation is written against.
        let behind = self.socket.buffered_bytes() > self.limits.max_send_buffer_bytes;
        let class = match dispatch.delivery {
            DeliveryClass::Reliable => CLASS_RELIABLE,
            DeliveryClass::Snapshot => {
                if behind {
                    self.shed_snapshots += targets.len() as u64;
                    self.notices.push(RelayNotice::Shedding {
                        peer: targets.first().cloned().unwrap_or_default(),
                        total: self.shed_snapshots,
                    });
                    return;
                }
                CLASS_SNAPSHOT
            }
        };
        for peer in targets {
            self.send_payload(&peer, class, payload.clone());
        }
    }

    fn name(&self) -> &'static str {
        "ws-relay"
    }
}
