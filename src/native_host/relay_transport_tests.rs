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
use crate::core::rendezvous::JOIN_REFUSED;
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
fn admit(
    t: &mut RelayTransport,
    socket: &FakeSocket,
    peer: &str,
    token: &str,
) -> Vec<TransportEvent> {
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
        code: Some(crate::core::rendezvous::CodeField::Issued(Box::new(
            JoinCode {
                full: "proj_ver_QUARK".to_string(),
                suffix: "QUARK".to_string(),
                ..Default::default()
            },
        ))),
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
    assert_eq!(
        classes,
        vec!["snapshot".to_string(), "reliable".to_string()]
    );
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
    // Refused locally rather than by being cut off at the service's ceiling,
    // and the notice names the SERVICE's number rather than this build's
    // default — which is the whole claim of this test.
    assert_eq!(socket.sent_of("relay").len(), before);
    assert!(t
        .drain_notices()
        .iter()
        .any(|n| matches!(n, RelayNotice::Fault { reason } if reason.contains("8 byte frame"))));
    // A reliable frame that will not fit is a broken guarantee, not a dropped
    // frame, so the link it was for ends rather than going quiet.
    assert_eq!(
        t.poll(),
        vec![TransportEvent::Disconnected {
            token: "tok-1".to_string()
        }]
    );
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
fn a_per_request_refusal_leaves_the_crew_and_the_code_alone() {
    // `error` is the registry's generic per-REQUEST refusal, not a link event.
    // A host broadcasting to Target::All addresses a peer the service detached
    // a tick ago and is answered `relay/no-peer` — the NORMAL case every time
    // somebody closes their phone. Folding that into the terminal arm
    // disconnected the whole crew and wiped the join code on every departure.
    let (mut t, socket) = transport();
    socket.arrive(&RendezvousFrame {
        code: Some(crate::core::rendezvous::CodeField::Issued(Box::new(
            JoinCode {
                suffix: "ABCDE".to_string(),
                ..Default::default()
            },
        ))),
        ..frame("hosted")
    });
    t.poll();
    admit(&mut t, &socket, "peer-1", "tok-1");
    admit(&mut t, &socket, "peer-2", "tok-2");
    t.drain_notices();

    for reason in ["no-peer", "not-relaying", "relay-too-large", "malformed"] {
        socket.arrive(&RendezvousFrame {
            reason: Some(reason.to_string()),
            request: Some("relay".to_string()),
            ..frame("error")
        });
        assert!(
            t.poll().is_empty(),
            "a {reason} refusal reported somebody disconnected"
        );
        assert_eq!(t.relayed_peers(), 2, "a {reason} refusal dropped the crew");
        assert!(
            t.code().is_some(),
            "a {reason} refusal wiped the join code off the viewscreen"
        );
        // It is still SAID, so the operator sees the service refusing things.
        assert!(matches!(
            t.drain_notices().as_slice(),
            [RelayNotice::Fault { .. }]
        ));
    }
}

#[test]
fn an_unreachable_error_really_does_end_every_relayed_session() {
    // The other half of the same split: `unreachable` is the registry's
    // record-is-gone answer, and a relayed link cannot outlive its record.
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    admit(&mut t, &socket, "peer-2", "tok-2");
    socket.arrive(&RendezvousFrame {
        reason: Some("unreachable".to_string()),
        request: Some("relay".to_string()),
        ..frame("error")
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
    assert_eq!(t.relayed_peers(), 0);
}

#[test]
fn a_peer_claiming_a_reserved_token_is_refused_rather_than_carried() {
    // The seam drops COMMANDS under a reserved token, but a peer left attached
    // under one still sits in audience() and receives that token's private
    // projection and every broadcast. server.html closes such a connection
    // outright; so does this.
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
    t.drain_notices();
    socket.outbound.lock().unwrap().clear();

    let identify = JsonCodec
        .encode_client(&ClientMessage::Identify {
            token: crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
            name: "Mallory".to_string(),
        })
        .unwrap();
    socket.arrive(&relayed("peer-1", &identify));
    assert!(
        t.poll().is_empty(),
        "a reserved token must not reach the simulation as an Identify"
    );

    // It is TOLD, in the same in-band frame the compatibility handshake uses.
    let refusal = socket
        .sent_of("relay")
        .into_iter()
        .find_map(|f| decode_handshake_frame(&f.payload.unwrap_or_default()).ok())
        .expect("the peer was told why");
    assert_eq!(refusal.kind, JOIN_REFUSED);
    assert_eq!(refusal.data.code.as_deref(), Some(RESERVED_TOKEN_CODE));

    // …and it is no longer addressable: a broadcast reaches nobody.
    socket.outbound.lock().unwrap().clear();
    t.dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });
    assert!(
        socket.sent_of("relay").is_empty(),
        "a refused peer was still handed simulation traffic"
    );
}

#[test]
fn a_stale_peers_departure_cannot_evict_the_player_that_just_reconnected() {
    // Every native crew member is relayed, so re-Identify on a fresh peer id IS
    // the ordinary reconnect — and a phone that lost radio without a TCP FIN
    // leaves the service holding its old socket for minutes. When that stale
    // close finally lands it must not disconnect the player who is at that
    // moment driving a station.
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    let events = admit(&mut t, &socket, "peer-2", "tok-1");
    assert_eq!(
        t.relayed_peers(),
        1,
        "the stale peer is severed, not kept alongside the live one"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, TransportEvent::Disconnected { .. })),
        "the player moved devices; nobody left"
    );

    socket.arrive(&RendezvousFrame {
        peer: Some("peer-1".to_string()),
        ..frame("relay-peer-left")
    });
    assert!(
        t.poll().is_empty(),
        "the stale peer's late departure disconnected the live player"
    );

    // And exactly ONE delivery per broadcast: two peers under one token would
    // have double-delivered every Target::Token and Target::All.
    socket.outbound.lock().unwrap().clear();
    t.dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });
    assert_eq!(socket.sent_of("relay").len(), 1);
}

#[test]
fn the_service_reporting_its_own_shedding_reaches_the_operator() {
    // The host→phone direction of the shed count, which the operator is the one
    // who can act on. Nothing read this frame before.
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    t.drain_notices();
    socket.arrive(&RendezvousFrame {
        peer: Some("peer-1".to_string()),
        dropped: Some(17),
        ..frame("relay-degraded")
    });
    t.poll();
    assert_eq!(
        t.drain_notices(),
        vec![RelayNotice::Shedding {
            peer: "peer-1".to_string(),
            total: 17,
        }]
    );
}

#[test]
fn an_oversized_reliable_message_fails_the_link_rather_than_vanishing() {
    // There is no such thing as a quietly dropped reliable frame: the
    // simulation is written against that guarantee. So the LINK fails and the
    // crew member re-joins, which is what gui/rendezvous-relay.js does too.
    let (mut t, socket) = transport();
    admit(&mut t, &socket, "peer-1", "tok-1");
    // A ceiling the service could authorise, small enough that an ordinary
    // message crosses it.
    socket.arrive(&RendezvousFrame {
        peer: Some("peer-2".to_string()),
        limits: Some(RelayLimits {
            max_frame_bytes: 4,
            max_send_buffer_bytes: 262_144,
        }),
        ..frame("relay-peer")
    });
    t.poll();
    socket.outbound.lock().unwrap().clear();

    t.dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });
    assert!(
        socket.sent_of("relay").is_empty(),
        "an unsendable frame must not be put on the wire"
    );
    // The service must be asked to detach the phone too, before this host
    // drops it locally — a link this host gives up on is otherwise one-sided:
    // the service keeps the phone's mailbox open and the phone sits on a
    // status line still reading "connected" (mirrors `refuse_peer`'s in-band
    // `JoinRefused`, and `gui/rendezvous-transport.js`'s `onFailure`).
    assert_eq!(
        socket.sent_of("relay-close"),
        vec![RendezvousFrame::relay_close("peer-1")],
        "the phone must be told, not just dropped locally"
    );
    assert_eq!(
        t.poll(),
        vec![TransportEvent::Disconnected {
            token: "tok-1".to_string()
        }],
        "the link that could not carry it is ended, visibly"
    );
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
