//! Tests for the native host's relay-backed crew path (issue #1113).
//!
//! Everything here drives [`RelayTransport`] over an in-process
//! [`RelaySocket`] fake, so what it proves is the PROTOCOL: registration, the
//! compatibility handshake, the `Identify`-to-token mapping, audience
//! resolution, the delivery classes and the shedding rule. It proves nothing at
//! all about a real WebSocket — that is `tests/native_relay_live.rs`, which is
//! `#[ignore]`d because it needs a service to talk to.
//!
//! The fake is deliberately a fake SOCKET rather than a fake service: every
//! frame these tests feed in is one the shipped `worker-rendezvous` registry
//! really sends, and the assertions read the frames this transport really puts
//! on the wire. A fake at any higher level would be asserting against a second
//! implementation of the thing under test.

use super::*;
use crate::core::messages::ServerMessage;
use std::sync::{Arc, Mutex};

/// A [`RelaySocket`] with two queues and a settable backlog.
#[derive(Clone, Default)]
struct FakeSocket {
    inbound: Arc<Mutex<Vec<String>>>,
    outbound: Arc<Mutex<Vec<String>>>,
    buffered: Arc<Mutex<usize>>,
    open: Arc<Mutex<bool>>,
}

impl FakeSocket {
    fn new() -> Self {
        Self {
            open: Arc::new(Mutex::new(true)),
            ..Default::default()
        }
    }

    /// Queue one frame as if the service had sent it.
    fn arrive(&self, frame: &RendezvousFrame) {
        self.inbound
            .lock()
            .unwrap()
            .push(encode_rendezvous_frame(frame).expect("encodable frame"));
    }

    /// Every frame the transport has sent, decoded, oldest first.
    fn sent(&self) -> Vec<RendezvousFrame> {
        self.outbound
            .lock()
            .unwrap()
            .iter()
            .map(|t| decode_rendezvous_frame(t).expect("the transport sent decodable JSON"))
            .collect()
    }

    fn sent_of(&self, kind: &str) -> Vec<RendezvousFrame> {
        self.sent().into_iter().filter(|f| f.kind == kind).collect()
    }

    fn set_buffered(&self, bytes: usize) {
        *self.buffered.lock().unwrap() = bytes;
    }

    fn die(&self) {
        *self.open.lock().unwrap() = false;
    }
}

impl RelaySocket for FakeSocket {
    fn poll(&mut self) -> Vec<String> {
        self.inbound.lock().unwrap().drain(..).collect()
    }

    fn send(&mut self, text: String) {
        self.outbound.lock().unwrap().push(text);
    }

    fn buffered_bytes(&self) -> usize {
        *self.buffered.lock().unwrap()
    }

    fn is_open(&self) -> bool {
        *self.open.lock().unwrap()
    }

    fn close(&mut self) {
        self.die();
    }
}

/// A host stamp a matching client satisfies.
fn host_stamp() -> DeliveryStamp {
    DeliveryStamp {
        protocol: crate::core::messages::PROTOCOL_VERSION,
        content_id: "phoenix-base".to_string(),
        content_epoch: 1,
    }
}

/// The stamp field a matching client presents, in its wire spelling.
fn matching_stamp_field() -> String {
    let s = host_stamp();
    format!("{}/{}/{}", s.protocol, s.content_id, s.content_epoch)
}

fn transport() -> (RelayTransport, FakeSocket) {
    let socket = FakeSocket::new();
    let transport = RelayTransport::new(
        socket.clone(),
        RelayHostConfig {
            namespace: "client".to_string(),
            version: None,
            stamp: host_stamp(),
        },
    );
    (transport, socket)
}

fn frame(kind: &str) -> RendezvousFrame {
    RendezvousFrame::new(kind)
}

/// One relayed game payload from `peer`.
fn relayed(peer: &str, payload: &str) -> RendezvousFrame {
    RendezvousFrame {
        from: Some(peer.to_string()),
        class: Some(CLASS_RELIABLE.to_string()),
        payload: Some(payload.to_string()),
        ..frame("relay")
    }
}

fn peer_joined(peer: &str) -> RendezvousFrame {
    RendezvousFrame {
        peer: Some(peer.to_string()),
        ..frame("relay-peer")
    }
}

/// Drive a peer all the way to an identified crew member, as the real sequence
/// does: the service announces it, it presents a stamp, it identifies.
fn admit(t: &mut RelayTransport, socket: &FakeSocket, peer: &str, token: &str) -> Vec<TransportEvent> {
    socket.arrive(&peer_joined(peer));
    let handshake = encode_handshake_frame(&HandshakeFrame {
        kind: JOIN_HANDSHAKE.to_string(),
        data: crate::core::rendezvous::HandshakeData {
            stamp: Some(matching_stamp_field()),
            ..Default::default()
        },
    })
    .unwrap();
    socket.arrive(&relayed(peer, &handshake));
    let identify = JsonCodec
        .encode_client(&ClientMessage::Identify {
            token: token.to_string(),
            name: "Ada".to_string(),
        })
        .unwrap();
    socket.arrive(&relayed(peer, &identify));
    t.poll()
}

// ── Registration ────────────────────────────────────────────────────────────

#[test]
fn it_registers_as_a_host_that_can_only_be_reached_over_the_relay() {
    // The claim is the point. A native host has no WebRTC at all, and a joiner
    // that did not know would spend the whole 8/16/30 s ladder four times over
    // discovering it before falling back to the only path there ever was.
    let (mut t, socket) = transport();
    socket.arrive(&frame("ready"));
    t.poll();

    let opens = socket.sent_of("host-open");
    assert_eq!(opens.len(), 1, "one registration");
    assert_eq!(opens[0].transports, vec!["ws-relay".to_string()]);
    assert_eq!(opens[0].namespace.as_deref(), Some("client"));
    assert_eq!(opens[0].v, RENDEZVOUS_PROTOCOL);
}

#[test]
fn a_second_ready_does_not_mint_a_second_code() {
    // The browser host guards the same way: a re-entered registration while
    // one is live would leave two records and two sets of five letters, one of
    // which nobody is reading.
    let (mut t, socket) = transport();
    socket.arrive(&frame("ready"));
    socket.arrive(&frame("ready"));
    t.poll();
    assert_eq!(socket.sent_of("host-open").len(), 1);
}

#[test]
fn the_issued_code_reaches_the_operator() {
    let (mut t, socket) = transport();
    socket.arrive(&RendezvousFrame {
        code: Some(crate::core::rendezvous::CodeField::Issued(Box::new(JoinCode {
            full: "proj_ver_QUARK".to_string(),
            suffix: "QUARK".to_string(),
            ..Default::default()
        }))),
        ..frame("hosted")
    });
    t.poll();
    assert_eq!(t.code().map(|c| c.suffix.as_str()), Some("QUARK"));
    assert!(matches!(
        t.drain_notices().as_slice(),
        [RelayNotice::Coded(code)] if code.suffix == "QUARK"
    ));
}

#[test]
fn a_service_speaking_another_protocol_revision_is_refused_rather_than_guessed_at() {
    // Both ends hard-refuse a foreign `v`, which is what lets the vocabulary
    // grow without a flag day — and what makes a stale deployed worker a
    // stated fault instead of a mystery.
    let (mut t, socket) = transport();
    socket.arrive(&RendezvousFrame {
        v: RENDEZVOUS_PROTOCOL + 1,
        ..frame("ready")
    });
    t.poll();
    assert!(socket.sent_of("host-open").is_empty());
    assert!(matches!(
        t.drain_notices().as_slice(),
        [RelayNotice::Fault { reason }] if reason.contains("rendezvous v")
    ));
}

// ── The compatibility handshake ─────────────────────────────────────────────

#[test]
fn a_matching_build_is_accepted_and_then_speaks_the_ordinary_crew_protocol() {
    let (mut t, socket) = transport();
    let events = admit(&mut t, &socket, "peer-1", "tok-1");

    // The acceptance went out on the reliable class, and nothing else did.
    let payloads = socket.sent_of("relay");
    assert_eq!(payloads.len(), 1, "just the verdict");
    assert_eq!(payloads[0].class.as_deref(), Some(CLASS_RELIABLE));
    assert_eq!(payloads[0].to.as_deref(), Some("peer-1"));
    let verdict = decode_handshake_frame(payloads[0].payload.as_ref().unwrap()).unwrap();
    assert_eq!(verdict.kind, "JoinAccepted");

    // …and the Identify that followed reached the simulation under its token.
    assert_eq!(
        events,
        vec![TransportEvent::Received {
            token: "tok-1".to_string(),
            msg: ClientMessage::Identify {
                token: "tok-1".to_string(),
                name: "Ada".to_string(),
            },
        }]
    );
}

#[test]
fn a_mismatched_build_is_refused_with_the_same_code_the_browser_host_uses() {
    // Same authority — delivery::check_join_stamp — as the browser asks through
    // wasm_check_client_stamp, so a phone gets the same sentence whichever kind
    // of host refused it.
    let (mut t, socket) = transport();
    socket.arrive(&peer_joined("peer-1"));
    let handshake = encode_handshake_frame(&HandshakeFrame {
        kind: JOIN_HANDSHAKE.to_string(),
        data: crate::core::rendezvous::HandshakeData {
            stamp: Some("999/phoenix-base/1".to_string()),
            ..Default::default()
        },
    })
    .unwrap();
    socket.arrive(&relayed("peer-1", &handshake));
    assert!(t.poll().is_empty(), "a refused build reaches nothing");

    let verdict =
        decode_handshake_frame(socket.sent_of("relay")[0].payload.as_ref().unwrap()).unwrap();
    assert_eq!(verdict.kind, "JoinRefused");
    assert_eq!(verdict.data.code.as_deref(), Some("protocol-mismatch"));
    assert!(matches!(
        t.drain_notices().as_slice(),
        [RelayNotice::Refused { code, .. }] if code == "protocol-mismatch"
    ));
}

#[test]
fn a_joiner_presenting_no_stamp_at_all_is_refused() {
    // #1112 made a stamp REQUIRED: every client that can reach a host is a
    // built Phoenix bundle carrying one, so a blank is a hand-copied page.
    let (mut t, socket) = transport();
    socket.arrive(&peer_joined("peer-1"));
    let handshake = encode_handshake_frame(&HandshakeFrame {
        kind: JOIN_HANDSHAKE.to_string(),
        data: crate::core::rendezvous::HandshakeData::default(),
    })
    .unwrap();
    socket.arrive(&relayed("peer-1", &handshake));
    t.poll();
    let verdict =
        decode_handshake_frame(socket.sent_of("relay")[0].payload.as_ref().unwrap()).unwrap();
    assert_eq!(verdict.data.code.as_deref(), Some("client-stamp-missing"));
}

#[test]
fn nothing_a_peer_sends_before_the_handshake_reaches_the_simulation() {
    // An open link is not admission. A build the host is about to refuse has no
    // business queuing simulation traffic, so early frames are DROPPED rather
    // than buffered — the same rule the browser host applies.
    let (mut t, socket) = transport();
    socket.arrive(&peer_joined("peer-1"));
    let identify = JsonCodec
        .encode_client(&ClientMessage::Identify {
            token: "tok-1".to_string(),
            name: "impostor".to_string(),
        })
        .unwrap();
    socket.arrive(&relayed("peer-1", &identify));
    assert!(t.poll().is_empty());
}

#[test]
fn nothing_a_refused_peer_sends_afterwards_reaches_the_simulation_either() {
    let (mut t, socket) = transport();
    socket.arrive(&peer_joined("peer-1"));
    let handshake = encode_handshake_frame(&HandshakeFrame {
        kind: JOIN_HANDSHAKE.to_string(),
        data: crate::core::rendezvous::HandshakeData {
            stamp: Some("999/phoenix-base/1".to_string()),
            ..Default::default()
        },
    })
    .unwrap();
    socket.arrive(&relayed("peer-1", &handshake));
    t.poll();

    let identify = JsonCodec
        .encode_client(&ClientMessage::Identify {
            token: "tok-1".to_string(),
            name: "Ada".to_string(),
        })
        .unwrap();
    socket.arrive(&relayed("peer-1", &identify));
    assert!(t.poll().is_empty());
}

#[test]
fn a_command_from_a_peer_that_never_identified_has_nowhere_to_go() {
    // Session tokens are the identity system; a peer id is ephemeral. Without
    // an Identify there is no token to deliver under, and inventing one would
    // be inventing a player.
    let (mut t, socket) = transport();
    socket.arrive(&peer_joined("peer-1"));
    let handshake = encode_handshake_frame(&HandshakeFrame {
        kind: JOIN_HANDSHAKE.to_string(),
        data: crate::core::rendezvous::HandshakeData {
            stamp: Some(matching_stamp_field()),
            ..Default::default()
        },
    })
    .unwrap();
    socket.arrive(&relayed("peer-1", &handshake));
    t.poll();

    let ready = JsonCodec
        .encode_client(&ClientMessage::SetReady { ready: true })
        .unwrap();
    socket.arrive(&relayed("peer-1", &ready));
    assert!(t.poll().is_empty());
}

// ── Outbound: audience and delivery class ───────────────────────────────────

#[test]
fn a_broadcast_reaches_every_identified_crew_member() {
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    admit(&mut t, &socket, "peer-2", "tok-2");
    let before = socket.sent_of("relay").len();

    t.dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });

    let sent: Vec<_> = socket.sent_of("relay").into_iter().skip(before).collect();
    let mut to: Vec<_> = sent.iter().filter_map(|f| f.to.clone()).collect();
    to.sort();
    assert_eq!(to, vec!["peer-1".to_string(), "peer-2".to_string()]);
}

#[test]
fn a_token_target_reaches_exactly_the_peer_holding_it() {
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    admit(&mut t, &socket, "peer-2", "tok-2");
    let before = socket.sent_of("relay").len();

    t.dispatch(TransportDispatch {
        target: &Target::Token("tok-2".to_string()),
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });

    let sent: Vec<_> = socket.sent_of("relay").into_iter().skip(before).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to.as_deref(), Some("peer-2"));
}

#[test]
fn an_except_target_reaches_everyone_else() {
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    admit(&mut t, &socket, "peer-2", "tok-2");
    let before = socket.sent_of("relay").len();

    t.dispatch(TransportDispatch {
        target: &Target::AllExcept("tok-2".to_string()),
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });

    let sent: Vec<_> = socket.sent_of("relay").into_iter().skip(before).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to.as_deref(), Some("peer-1"));
}

#[test]
fn the_delivery_class_survives_the_crossing() {
    // The relay's whole risk: a WebSocket is reliable and ordered, so a naive
    // implementation silently upgrades the lossy class and loses the property
    // the snapshot class exists for.
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    let before = socket.sent_of("relay").len();

    for delivery in [DeliveryClass::Snapshot, DeliveryClass::Reliable] {
        t.dispatch(TransportDispatch {
            target: &Target::All,
            msg: &ServerMessage::GameStarted,
            delivery,
        });
    }

    let classes: Vec<_> = socket
        .sent_of("relay")
        .into_iter()
        .skip(before)
        .filter_map(|f| f.class)
        .collect();
    assert_eq!(classes, vec!["snapshot".to_string(), "reliable".to_string()]);
}

#[test]
fn a_backed_up_socket_sheds_snapshots_and_never_sheds_commands() {
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    let before = socket.sent_of("relay").len();
    socket.set_buffered(RelayLimits::default().max_send_buffer_bytes + 1);
    t.drain_notices();

    t.dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Snapshot,
    });
    t.dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });

    // Exactly one frame went out, and it is the reliable one: queuing the
    // snapshot behind the backlog is the head-of-line blocking its class exists
    // to avoid, while a dropped command would break a guarantee the simulation
    // is written against.
    let sent: Vec<_> = socket.sent_of("relay").into_iter().skip(before).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].class.as_deref(), Some(CLASS_RELIABLE));
    assert!(t
        .drain_notices()
        .iter()
        .any(|n| matches!(n, RelayNotice::Shedding { .. })));
}

#[test]
fn nothing_is_dispatched_to_a_peer_that_has_not_identified() {
    // Audience resolution runs on tokens, so a peer with no token is in no
    // audience — including `Target::All`.
    let (mut t, socket) = transport();
    socket.arrive(&peer_joined("peer-1"));
    t.poll();
    t.dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });
    assert!(socket.sent_of("relay").is_empty());
}

#[test]
fn the_service_s_authored_limits_beat_this_builds_defaults() {
    // The numbers are a designer's (assets/join/join-codes.toml), advertised on
    // `relay-peer`. A host carrying its own copy would need a native release
    // every time one was retuned.
    let (mut t, socket) = transport();
    socket.arrive(&RendezvousFrame {
        peer: Some("peer-1".to_string()),
        limits: Some(RelayLimits {
            max_frame_bytes: 8,
            max_send_buffer_bytes: 16,
        }),
        ..frame("relay-peer")
    });
    t.poll();
    let handshake = encode_handshake_frame(&HandshakeFrame {
        kind: JOIN_HANDSHAKE.to_string(),
        data: crate::core::rendezvous::HandshakeData {
            stamp: Some(matching_stamp_field()),
            ..Default::default()
        },
    })
    .unwrap();
    socket.arrive(&relayed("peer-1", &handshake));
    let identify = JsonCodec
        .encode_client(&ClientMessage::Identify {
            token: "tok-1".to_string(),
            name: "Ada".to_string(),
        })
        .unwrap();
    socket.arrive(&relayed("peer-1", &identify));
    t.poll();
    t.drain_notices();

    let before = socket.sent_of("relay").len();
    t.dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });
    // Refused locally rather than by being cut off at the service's ceiling.
    assert_eq!(socket.sent_of("relay").len(), before);
    assert!(t
        .drain_notices()
        .iter()
        .any(|n| matches!(n, RelayNotice::Fault { reason } if reason.contains("at most 8"))));
}

// ── Endings ─────────────────────────────────────────────────────────────────

#[test]
fn a_departed_peer_reaches_the_lobby_as_a_disconnect() {
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    socket.arrive(&RendezvousFrame {
        peer: Some("peer-1".to_string()),
        ..frame("relay-peer-left")
    });
    assert_eq!(
        t.poll(),
        vec![TransportEvent::Disconnected {
            token: "tok-1".to_string()
        }]
    );
    assert_eq!(t.relayed_peers(), 0);
}

#[test]
fn a_peer_that_never_identified_produces_no_disconnect() {
    // There is no session for the lobby to restore or release, and a
    // fabricated token would be a fabricated player leaving.
    let (mut t, socket) = transport();
    socket.arrive(&peer_joined("peer-1"));
    t.poll();
    socket.arrive(&RendezvousFrame {
        peer: Some("peer-1".to_string()),
        ..frame("relay-peer-left")
    });
    assert!(t.poll().is_empty());
}

#[test]
fn the_service_closing_our_relay_disconnects_everyone_it_was_carrying() {
    // Unlike a DataChannel, a relayed link does not outlive the service that
    // introduced it: when the relay goes, every crew member on it is
    // unreachable, and the lobby has to be told or their stations never flip
    // to Backfill.
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    admit(&mut t, &socket, "peer-2", "tok-2");
    socket.arrive(&RendezvousFrame {
        reason: Some("relay-overflow".to_string()),
        ..frame("relay-closed")
    });
    let mut tokens: Vec<_> = t
        .poll()
        .into_iter()
        .filter_map(|e| match e {
            TransportEvent::Disconnected { token } => Some(token),
            _ => None,
        })
        .collect();
    tokens.sort();
    assert_eq!(tokens, vec!["tok-1".to_string(), "tok-2".to_string()]);
    assert!(t.code().is_none(), "the code is gone with the record");
}

#[test]
fn a_dead_socket_disconnects_everyone_once_and_not_twice() {
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    socket.die();
    assert_eq!(
        t.poll(),
        vec![TransportEvent::Disconnected {
            token: "tok-1".to_string()
        }]
    );
    assert!(
        t.poll().is_empty(),
        "a second poll must not report the same player gone again"
    );
}

#[test]
fn an_undecodable_frame_is_a_notice_rather_than_a_panic() {
    // This is an unauthenticated public service on the other end of the socket.
    let (mut t, socket) = transport();
    socket.outbound.lock().unwrap().clear();
    socket.inbound.lock().unwrap().push("not json".to_string());
    assert!(t.poll().is_empty());
    assert!(matches!(
        t.drain_notices().as_slice(),
        [RelayNotice::Fault { .. }]
    ));
}

#[test]
fn it_names_itself_in_the_operator_log() {
    let (t, _socket) = transport();
    assert_eq!(t.name(), "ws-relay");
}
