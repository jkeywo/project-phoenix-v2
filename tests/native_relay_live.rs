//! Native host ↔ a REAL rendezvous service ↔ a real WebSocket client
//! (issue #1113). `#[ignore]`d, because it needs a service to talk to.
//!
//! # Why this is the only proof of some things
//!
//! Everything else about the native crew path is proved against a fake socket
//! (`src/native_host/relay_transport_tests.rs`) or against the JavaScript
//! source as text (`tests/native_relay_protocol.rs`). Neither can catch:
//!
//!   * `tungstenite` and a Cloudflare-shaped WebSocket disagreeing about
//!     framing, masking or close;
//!   * the Rust frame vocabulary and the JavaScript one failing to interoperate
//!     for a reason that is not a renamed identifier;
//!   * the registry refusing something a native host does that a browser does
//!     not — the relay-only `transports` claim, most of all.
//!
//! This test does all three, over a real socket, against the real
//! `worker-rendezvous/src/registry.js`.
//!
//! # Running it
//!
//! Two terminals. The first runs the service locally — the real registry over
//! real sockets, no Cloudflare account and no wrangler:
//!
//! ```text
//! node scripts/rendezvous-dev-server.mjs --port 8788
//! ```
//!
//! The second runs this:
//!
//! ```text
//! cargo test --features host --test native_relay_live -- --ignored --nocapture
//! ```
//!
//! `PHOENIX_RENDEZVOUS` overrides the base URL, so the same test can be pointed
//! at the DEPLOYED service once §3a of docs/delivery-checklist.md is ticked —
//! which is the version worth running before a field session, because it is the
//! only one that exercises TLS and the origin gate:
//!
//! ```text
//! PHOENIX_RENDEZVOUS=https://phoenix-rendezvous.project-phoenix.workers.dev \
//! PHOENIX_ORIGIN=https://pp-dev.kiwigamedesign.co.uk \
//!   cargo test --features host --test native_relay_live -- --ignored --nocapture
//! ```
//!
//! # Why it is `#[ignore]`d rather than skipped-if-unreachable
//!
//! A test that passes when it cannot reach anything is a test that reports
//! green for a broken build. `#[ignore]` says "this did not run"; a silent skip
//! says "this ran and was fine", which is the one thing it must never say.
//!
//! # What is still NOT proved here
//!
//! A BROWSER joining a native host. That needs a browser, a native host with a
//! GPU window, and a service, all at once — and the browser half is the one
//! piece already covered elsewhere (tests/smoke/transport-paths.spec.js drives
//! the shipped client page over the real relay against a browser host). The
//! remaining gap is one manual step, and it is written down in
//! docs/acceptance/1113-networks.md rather than pretended away here.

#![cfg(feature = "host")]

use std::time::{Duration, Instant};

use project_phoenix::core::codec::{
    decode_handshake_frame, decode_rendezvous_frame, encode_handshake_frame,
    encode_rendezvous_frame, JsonCodec, MessageCodec,
};
use project_phoenix::core::messages::{ClientMessage, DeliveryClass, ServerMessage};
use project_phoenix::core::rendezvous::{
    CodeField, HandshakeData, HandshakeFrame, JoinCode, RendezvousFrame, CLASS_RELIABLE,
    JOIN_ACCEPTED, JOIN_HANDSHAKE, TRANSPORT_WS_RELAY,
};
use project_phoenix::delivery::stamp::DeliveryStamp;
use project_phoenix::lobby::handler::Target;
use project_phoenix::native_host::relay_socket::WsRelaySocket;
use project_phoenix::native_host::relay_transport::{RelayHostConfig, RelayNotice, RelayTransport};
use project_phoenix::native_host::transport::{NativeTransport, TransportDispatch, TransportEvent};

fn base() -> String {
    std::env::var("PHOENIX_RENDEZVOUS").unwrap_or_else(|_| "http://127.0.0.1:8788".to_string())
}

fn origin() -> String {
    std::env::var("PHOENIX_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

fn host_stamp() -> DeliveryStamp {
    DeliveryStamp {
        protocol: project_phoenix::core::messages::PROTOCOL_VERSION,
        content_id: "phoenix-base".to_string(),
        content_epoch: 1,
    }
}

/// Poll the transport until `f` answers, or give up. Real sockets take real
/// milliseconds, and a native host polls once per frame, so this is what a
/// frame loop looks like with nothing else in it.
fn pump_until<T>(
    transport: &mut RelayTransport,
    mut f: impl FnMut(&mut RelayTransport, Vec<TransportEvent>) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let events = transport.poll();
        if let Some(found) = f(transport, events) {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "the rendezvous service at {} never answered — is it running? \
             See this file's header for the command.",
            base()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A raw client socket on `/v1/join`, standing in for a phone.
struct Joiner {
    socket: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
}

impl Joiner {
    fn connect() -> Self {
        use tungstenite::client::IntoClientRequest;
        let url = base()
            .trim_end_matches('/')
            .replace("https://", "wss://")
            .replace("http://", "ws://")
            + "/v1/join";
        let mut request = url.as_str().into_client_request().expect("a usable URL");
        request
            .headers_mut()
            .insert("Origin", origin().parse().expect("a usable origin"));
        let (socket, _) = tungstenite::connect(request).unwrap_or_else(|e| {
            panic!("cannot reach the rendezvous service at {url}: {e} — see this file's header")
        });
        Self { socket }
    }

    fn send(&mut self, frame: &RendezvousFrame) {
        let text = encode_rendezvous_frame(frame).expect("encodable");
        self.socket
            .send(tungstenite::Message::Text(text.into()))
            .expect("the socket takes a frame");
    }

    /// Read frames until one of `kind` arrives.
    fn wait_for(&mut self, kind: &str) -> RendezvousFrame {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            assert!(Instant::now() < deadline, "no {kind} frame arrived");
            match self.socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    let frame = decode_rendezvous_frame(&text)
                        .unwrap_or_else(|e| panic!("undecodable frame {text}: {e}"));
                    if frame.kind == "error" {
                        panic!("the service refused a request: {:?}", frame.reason);
                    }
                    if frame.kind == kind {
                        return frame;
                    }
                }
                Ok(_) => {}
                Err(e) => panic!("the join socket died waiting for {kind}: {e}"),
            }
        }
    }
}

#[test]
#[ignore = "needs a running rendezvous service — see this file's header"]
fn a_browser_shaped_client_joins_a_native_host_over_a_real_relay() {
    let socket = WsRelaySocket::connect(&base(), &origin())
        .unwrap_or_else(|e| panic!("{e} — see this file's header for the command"));
    let mut host = RelayTransport::new(
        socket,
        RelayHostConfig {
            namespace: "client".to_string(),
            version: None,
            stamp: host_stamp(),
        },
    );

    // 1. The service issues a code. This alone proves the `host-open` a native
    //    host sends is one a real registry accepts over a real socket.
    let code: JoinCode = pump_until(&mut host, |t, _| {
        t.drain_notices().into_iter().find_map(|n| match n {
            RelayNotice::Coded(code) => Some(code),
            RelayNotice::Fault { reason } => panic!("the service refused this host: {reason}"),
            _ => None,
        })
    });
    println!("issued code {} ({})", code.suffix, code.full);
    assert_eq!(code.suffix.len(), 5);

    // 2. A joiner resolves it — and is told this host has NO WebRTC, which is
    //    the field that stops a phone spending ninety seconds discovering it.
    let mut joiner = Joiner::connect();
    joiner.wait_for("ready");
    joiner.send(&RendezvousFrame {
        // A TYPED code, exactly as a phone sends it — the same field name the
        // service answered `hosted` with, carrying the other of its two shapes.
        code: Some(CodeField::Typed(code.full.clone())),
        ..RendezvousFrame::new("join")
    });
    let joined = joiner.wait_for("joined");
    assert_eq!(
        joined.transports,
        vec![TRANSPORT_WS_RELAY.to_string()],
        "a native host must advertise that the relay is the only way in"
    );

    // 3. It attaches to the relay and the host is told, with the bounds.
    joiner.send(&RendezvousFrame::new("relay-open"));
    let ready = joiner.wait_for("relay-ready");
    assert!(ready.limits.is_some(), "the service advertises its bounds");

    // 4. The compatibility handshake, in band, exactly as a phone sends it.
    let stamp = host_stamp();
    let handshake = encode_handshake_frame(&HandshakeFrame {
        kind: JOIN_HANDSHAKE.to_string(),
        data: HandshakeData {
            stamp: Some(format!(
                "{}/{}/{}",
                stamp.protocol, stamp.content_id, stamp.content_epoch
            )),
            ..Default::default()
        },
    })
    .expect("encodable");
    joiner.send(&RendezvousFrame {
        class: Some(CLASS_RELIABLE.to_string()),
        payload: Some(handshake),
        ..RendezvousFrame::new("relay")
    });

    // The host answers from delivery::check_join_stamp, over the real relay.
    pump_until(&mut host, |_, _| Some(()));
    let verdict_frame = joiner.wait_for("relay");
    let verdict = decode_handshake_frame(verdict_frame.payload.as_deref().unwrap_or(""))
        .expect("a decodable verdict");
    assert_eq!(
        verdict.kind, JOIN_ACCEPTED,
        "a matching build must be admitted; got {verdict:?}"
    );

    // 5. …and the ordinary crew protocol starts. This is the whole claim: the
    //    same `Identify` a phone sends reaches the simulation's inbound bus
    //    under its own session token, with no second protocol anywhere.
    let identify = JsonCodec
        .encode_client(&ClientMessage::Identify {
            token: "live-token".to_string(),
            name: "Ada".to_string(),
        })
        .expect("encodable");
    joiner.send(&RendezvousFrame {
        class: Some(CLASS_RELIABLE.to_string()),
        payload: Some(identify),
        ..RendezvousFrame::new("relay")
    });
    let event = pump_until(&mut host, |_, events| events.into_iter().next());
    assert_eq!(
        event,
        TransportEvent::Received {
            token: "live-token".to_string(),
            msg: ClientMessage::Identify {
                token: "live-token".to_string(),
                name: "Ada".to_string(),
            },
        }
    );

    // 6. And the other direction: an outbound message resolved by audience
    //    reaches the client as an ordinary ServerMessage.
    host.dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });
    let outbound = joiner.wait_for("relay");
    let msg = JsonCodec
        .decode_server(outbound.payload.as_deref().unwrap_or(""))
        .expect("a decodable ServerMessage");
    assert_eq!(msg, ServerMessage::GameStarted);
    println!("native host ↔ real relay ↔ client: round trip complete");
}
