//! The native host's crew path: a [`NativeTransport`] backed by the rendezvous
//! service's WebSocket game relay (issue #1113; completes issue #1121's
//! deferred "browser clients join the native host" acceptance criterion).
//!
//! # Why this shape, and why it is not WebRTC
//!
//! [`crate::native_host::transport`] left a seam with nothing plugged into it,
//! and said so plainly: the browser host reaches its crew over PeerJS, later
//! WebRTC, and neither runs in a Rust process. Issue #1113 built the piece that
//! changes the answer — the rendezvous service now carries the game's own
//! frames when a direct link cannot be built — and a native host is simply the
//! case where a direct link can *never* be built. It holds one outbound `wss:`
//! connection, registers as a host, and every crew member reaches it over the
//! relay.
//!
//! That makes the native path a USER of the browser's fallback rather than a
//! second transport, which is the whole point: same service, same frame
//! vocabulary (`crate::core::rendezvous`), same delivery classes, same in-band
//! compatibility handshake, same `Identify` gate, same reserved-token refusal.
//! A phone cannot tell it is talking to a native host, and nothing downstream
//! of `InboundMessage` can either.
//!
//! It also means the native host declares `transports: ["ws-relay"]` when it
//! registers. Without that a joiner would spend the entire 8/16/30 s ladder,
//! four times over, discovering that a host with no WebRTC has no WebRTC.
//!
//! # `[ai]` — the dependency choice
//!
//! The socket implementation uses **`tungstenite`**, the synchronous client,
//! rather than `tokio-tungstenite` or `async-tungstenite`. Reason: this crate
//! has no async runtime and every other native I/O surface it owns is blocking
//! std threads — `delivery::serve` is a `TcpListener` and a thread per
//! connection — so an async runtime would be a new architectural dependency
//! bought for one socket. `tungstenite` is the same crate those two wrap.
//! Native-only, behind the `host` feature; no new wasm dependency, and the
//! browser build cannot reach any of it.
//!
//! # What is tested where, honestly
//!
//! * **This module's tests** drive [`RelayTransport`] over an in-process
//!   [`RelaySocket`] fake. They prove the protocol logic: registration, the
//!   stamp handshake, the `Identify`-to-token mapping, audience resolution,
//!   the delivery classes, and the shedding rule. No socket, no service.
//! * **`tests/native_relay_protocol.rs`** pins the Rust frame vocabulary
//!   against the JavaScript one by reading the JS files, because nothing else
//!   can catch the two drifting apart.
//! * **`tests/native_relay_live.rs`** is `#[ignore]`d and needs a real service.
//!   It is the only thing here that proves a byte crosses a real WebSocket;
//!   the command to run it is in its own doc comment.

use std::collections::HashMap;

use crate::core::codec::{
    decode_handshake_frame, decode_rendezvous_frame, encode_handshake_frame,
    encode_rendezvous_frame, JsonCodec, MessageCodec,
};
use crate::core::messages::{ClientMessage, DeliveryClass};
use crate::core::rendezvous::{
    HandshakeFrame, JoinCode, RelayLimits, RendezvousFrame, CLASS_RELIABLE, CLASS_SNAPSHOT,
    JOIN_HANDSHAKE, RENDEZVOUS_PROTOCOL,
};
use crate::delivery::stamp::DeliveryStamp;
use crate::lobby::handler::Target;
use crate::native_host::transport::{NativeTransport, TransportDispatch, TransportEvent};

/// A duplex text-frame pipe to the rendezvous service.
///
/// The seam between the protocol (everything in this module) and the socket
/// (`tungstenite`, or a test's in-process fake). Deliberately narrow and
/// non-blocking: [`RelayTransport::poll`] is called from Bevy's `PreUpdate`
/// once per frame and may not block the simulation for a network round trip.
pub trait RelaySocket: Send + Sync + 'static {
    /// Every text frame that has arrived since the last poll, in order.
    /// Returns empty rather than blocking when nothing has.
    fn poll(&mut self) -> Vec<String>;

    /// Queue one text frame. Failures are the socket's to report through
    /// [`RelaySocket::is_open`] — a send that cannot happen is a dead link, not
    /// an error the simulation can act on mid-frame.
    fn send(&mut self, text: String);

    /// Bytes queued and not yet on the wire. The real backpressure signal, and
    /// the one thing that makes the snapshot class genuinely lossy over a
    /// transport that is not (see [`RelayTransport::dispatch`]). A socket that
    /// cannot report one answers `0`, which disables shedding — honest for a
    /// transport with no such signal, rather than a fabricated number.
    fn buffered_bytes(&self) -> usize {
        0
    }

    /// False once the link is gone. Checked every poll.
    fn is_open(&self) -> bool;

    /// Close the link. Idempotent.
    fn close(&mut self);
}

/// What a native host tells the service about itself when it registers.
pub struct RelayHostConfig {
    /// Which typed code namespace to be issued in — `client` for crew.
    pub namespace: String,
    /// The release GUID the code is registered under. `None` lets the service
    /// use its own bundled table's version, which is what a host built from
    /// the same checkout wants.
    pub version: Option<String>,
    /// This host's own delivery stamp, the authority every joiner's
    /// compatibility handshake is answered from.
    pub stamp: DeliveryStamp,
}

/// The `JoinRefused` code a peer gets for claiming a token only the host
/// runtime may use. Not a [`crate::delivery::stamp::StampMismatch`] code — this
/// is not a verdict about the joiner's BUILD — so it is spelled here and mapped
/// to its own sentence in `gui/join-code.js`'s `REASON_STRING_IDS`.
pub const RESERVED_TOKEN_CODE: &str = "reserved-token";

/// One crew member the service is carrying for us.
struct RelayPeer {
    /// True once the compatibility handshake admitted this build.
    admitted: bool,
    /// True once refused: nothing it sends afterwards may reach the simulation.
    refused: bool,
    /// The session token it presented on `Identify`. `None` until then, which
    /// is why a message before `Identify` has nowhere to go.
    token: Option<String>,
}

/// A [`NativeTransport`] whose wire is the rendezvous service's game relay.
pub struct RelayTransport {
    socket: Box<dyn RelaySocket>,
    config: RelayHostConfig,
    codec: JsonCodec,
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
    pending_drops: Vec<String>,
    /// Anything the operator should see, drained by the host each frame.
    notices: RelayNotices,
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
#[derive(Clone, Default, bevy::prelude::Resource)]
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
    /// The service issued a join code. The five letters go on the viewscreen.
    Coded(JoinCode),
    /// A joiner was refused by the compatibility handshake.
    Refused { peer: String, code: String },
    /// The service, or the link to it, said something went wrong.
    Fault { reason: String },
    /// Snapshot frames were shed to a peer because the socket is behind.
    Shedding { peer: String, total: u64 },
}

impl RelayTransport {
    /// Build a transport over `socket` and register as a host at once.
    pub fn new(socket: impl RelaySocket, config: RelayHostConfig) -> Self {
        Self {
            socket: Box::new(socket),
            config,
            codec: JsonCodec,
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
        let verdict = crate::delivery::check_join_stamp(&self.config.stamp, stamp.as_deref());
        let (frame, admitted, refusal) = match verdict {
            Ok(()) => (HandshakeFrame::accepted(), true, None),
            Err(mismatch) => (
                HandshakeFrame::refused(mismatch.code(), mismatch.detail()),
                false,
                Some(mismatch.code().to_string()),
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
    fn on_game_payload(&mut self, peer: &str, payload: &str, out: &mut Vec<TransportEvent>) {
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

        let Ok(msg) = self.codec.decode_client(payload) else {
            self.notices.push(RelayNotice::Fault {
                reason: format!("undecodable client message from {peer}"),
            });
            return;
        };

        // `Identify` is what names a session token, exactly as it is in
        // server.html. Until one arrives this peer has no identity, so a
        // message before it has nowhere to be delivered and is dropped.
        if let ClientMessage::Identify { token, .. } = &msg {
            // A peer's token is SELF-DECLARED, and two shapes are reserved for
            // the host runtime (`__local_console__` and the `ai:` prefix). The
            // seam refuses commands under them, but a peer left attached under
            // a reserved token still sits in `audience()` and receives that
            // token's projection and every broadcast — so refuse the CONNECTION
            // here, the way server.html does, rather than silently dropping the
            // half of the traffic that happens to travel upwards.
            if crate::lobby::handler::is_reserved_token(token) {
                self.refuse_peer(
                    peer,
                    RESERVED_TOKEN_CODE,
                    format!(
                    "{peer} claimed the reserved token {token}, which only the host runtime may use"
                ),
                );
                return;
            }
            // Duplicate-token sever, and the reason it has to happen HERE: on a
            // native host every crew member is relayed, so re-`Identify` on a
            // fresh rendezvous peer id is the ordinary reconnect, and a phone
            // that lost radio without a TCP FIN leaves the service holding its
            // old socket for minutes. Two peers under one token would both pass
            // `audience()` and double-deliver every `Target::Token` and
            // `Target::All`. The prior peer is dropped WITHOUT a disconnect —
            // the player did not leave, they moved — exactly as server.html
            // replaces `tokenConns` before closing the earlier connection.
            let stale: Vec<String> = self
                .peers
                .iter()
                .filter(|(id, p)| id.as_str() != peer && p.token.as_deref() == Some(token.as_str()))
                .map(|(id, _)| id.clone())
                .collect();
            for id in stale {
                self.peers.remove(&id);
            }
            if let Some(entry) = self.peers.get_mut(peer) {
                entry.token = Some(token.clone());
            }
        }
        let Some(token) = self.peers.get(peer).and_then(|p| p.token.clone()) else {
            self.notices.push(RelayNotice::Fault {
                reason: format!("{peer} sent a command before identifying"),
            });
            return;
        };
        out.push(TransportEvent::Received { token, msg });
    }

    /// Refuse this peer at the transport: tell it why in the same in-band
    /// `JoinRefused` the compatibility handshake uses, mark it refused so
    /// nothing it sends afterwards reaches the simulation, and stop carrying
    /// it. It keeps no token, so it is out of `audience()` at once and its
    /// eventual departure reports no disconnect for a player that never was.
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
            entry.token = None;
        }
    }

    /// A peer's link ended. Reports a disconnect only for one that got as far
    /// as identifying — the lobby has nothing to restore for anybody else.
    ///
    /// The identity guard is the same one server.html's `close` handler applies,
    /// and for the same reason: on a same-token reconnect the NEW peer has
    /// already claimed the token by the time the stale peer's departure lands,
    /// and reporting that departure would flip an actively-driven station to
    /// Backfill. So the question asked here is "is anybody ELSE still holding
    /// this token" — which is what makes a late departure silent for a player
    /// who has already come back on a fresh rendezvous peer id.
    fn drop_peer(&mut self, peer: &str, out: &mut Vec<TransportEvent>) {
        let Some(entry) = self.peers.remove(peer) else {
            return;
        };
        let Some(token) = entry.token else {
            return;
        };
        if self
            .peers
            .values()
            .any(|p| p.token.as_deref() == Some(token.as_str()))
        {
            return;
        }
        out.push(TransportEvent::Disconnected { token });
    }

    fn on_frame(&mut self, frame: RendezvousFrame, out: &mut Vec<TransportEvent>) {
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
                // somebody else's five letters on its viewscreen.
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
                    self.peers.insert(
                        peer,
                        RelayPeer {
                            admitted: false,
                            refused: false,
                            token: None,
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
    fn lose_relay(&mut self, reason: String, out: &mut Vec<TransportEvent>) {
        for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
            self.drop_peer(&peer, out);
        }
        self.code = None;
        self.registered = false;
        self.notices.push(RelayNotice::Fault { reason });
    }

    /// Every peer an audience target resolves to, as rendezvous peer ids.
    fn audience(&self, target: &Target) -> Vec<String> {
        self.peers
            .iter()
            .filter(|(_, p)| p.admitted && p.token.is_some())
            .filter(|(_, p)| match target {
                Target::All => true,
                Target::Token(t) => p.token.as_deref() == Some(t.as_str()),
                Target::AllExcept(t) => p.token.as_deref() != Some(t.as_str()),
            })
            .map(|(id, _)| id.clone())
            .collect()
    }
}

impl NativeTransport for RelayTransport {
    fn poll(&mut self) -> Vec<TransportEvent> {
        let mut out = Vec::new();
        // Links this transport gave up on while dispatching, where it had no
        // way to say so (see `dispatch`).
        for peer in std::mem::take(&mut self.pending_drops) {
            self.drop_peer(&peer, &mut out);
        }
        if !self.socket.is_open() {
            // The link died. Report every identified crew member gone, once —
            // `lose_relay` empties the map, so a second poll produces nothing.
            // It also clears `registered`, which is what lets a socket that
            // redials (relay_socket.rs's supervisor) re-send `host-open` on the
            // service's next `ready` and put a fresh code on the viewscreen.
            if self.link_up {
                self.link_up = false;
                self.lose_relay("unreachable".to_string(), &mut out);
            }
            return out;
        }
        self.link_up = true;
        for text in self.socket.poll() {
            match decode_rendezvous_frame(&text) {
                Ok(frame) => self.on_frame(frame, &mut out),
                Err(_) => self.notices.push(RelayNotice::Fault {
                    reason: "undecodable frame from the rendezvous service".to_string(),
                }),
            }
        }
        out
    }

    fn dispatch(&mut self, dispatch: TransportDispatch<'_>) {
        let targets = self.audience(dispatch.target);
        if targets.is_empty() {
            return;
        }
        let Ok(payload) = self.codec.encode_server(dispatch.msg) else {
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
                    self.pending_drops.extend(targets);
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

#[cfg(test)]
#[path = "relay_transport_tests.rs"]
mod tests;
