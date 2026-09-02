//! A real WebSocket client joins a real native host, with no service anywhere
//! (issue #1353).
//!
//! # Why this one is not `#[ignore]`d
//!
//! `tests/native_relay_live.rs` is the same proof over the cloud rendezvous,
//! and it can never run in CI: it needs a service to be running, and a test
//! that quietly skips when it cannot reach one reports green for a broken
//! build. The direct-accept leg has no such dependency — the host IS the
//! service — so the whole path is bindable on loopback and everything that
//! test could only assert against a live worker is asserted here on every run:
//!
//!   * `tungstenite`'s server-side framing against its own client, over the
//!     handshake `delivery::serve` performs by hand (it has already read the
//!     request head, so there is none left for `tungstenite` to read);
//!   * the frame vocabulary interoperating end to end rather than matching as
//!     text (`tests/native_relay_protocol.rs` is the text check);
//!   * the join validation a phone actually meets — code, protocol version,
//!     join stamp, the reserved-token refusal;
//!   * a socket dying producing exactly one `Disconnected` for the station its
//!     holder had.
//!
//! What is still not proved here is a BROWSER doing it, which needs a browser,
//! a GPU window and a phone-shaped page: that is the manual step in
//! `docs/acceptance/1335-native-lobby.md`.

#![cfg(feature = "host")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use project_phoenix::core::codec::{
    decode_handshake_frame, decode_rendezvous_frame, encode_handshake_frame,
    encode_rendezvous_frame, JsonCodec, MessageCodec,
};
use project_phoenix::core::messages::{ClientMessage, DeliveryClass, ServerMessage};
use project_phoenix::core::rendezvous::{
    CodeField, HandshakeData, HandshakeFrame, JoinCode, RendezvousFrame, CLASS_RELIABLE,
    JOIN_ACCEPTED, JOIN_HANDSHAKE, JOIN_REFUSED, RENDEZVOUS_PROTOCOL, TRANSPORT_WS_RELAY,
};
use project_phoenix::delivery::args::{ClientSource, HostArgs};
use project_phoenix::delivery::serve::{load_content, HostServer, ShutdownSignal};
use project_phoenix::delivery::stamp::DeliveryStamp;
use project_phoenix::lobby::handler::Target;
use project_phoenix::native_host::direct_join::DirectJoinService;
use project_phoenix::native_host::join_codes::JoinCodeTable;
use project_phoenix::native_host::relay_transport::{RelayHostConfig, RelayNotice, RelayTransport};
use project_phoenix::native_host::transport::{NativeTransport, TransportDispatch, TransportEvent};

const MANIFEST: &str = "assets/scenarios.toml";
const JOIN_TABLE: &str = "assets/join/join-codes.toml";

fn args() -> HostArgs {
    HostArgs {
        // Port 0: the OS picks a free one and `local_addr()` reports it, so
        // parallel test binaries never collide on a fixed port.
        addr: "127.0.0.1:0".to_string(),
        client: ClientSource::Hosted,
        manifest: MANIFEST.to_string(),
        content_dir: ".".to_string(),
        skip_bundle_check: false,
        sim: None,
        setup: false,
        profile: None,
    }
}

/// A bound host with the direct-join door open, its half of the protocol polled
/// on a thread for the length of the test.
struct Host {
    addr: String,
    code: JoinCode,
    stamp: DeliveryStamp,
    transport: Arc<Mutex<RelayTransport>>,
    events: Arc<Mutex<Vec<TransportEvent>>>,
    notices: Arc<Mutex<Vec<RelayNotice>>>,
    stop: Arc<AtomicBool>,
    shutdown: ShutdownSignal,
    pump: Option<std::thread::JoinHandle<()>>,
    delivery: Option<std::thread::JoinHandle<()>>,
}

impl Host {
    /// Everything `phoenix-host` does for the direct leg, in the order it does
    /// it: bind, open the service, install the door, then wrap the service in
    /// the ordinary host-half transport.
    fn start() -> Self {
        let content = load_content(".", MANIFEST).expect("the repo's own content loads");
        let server = HostServer::bind(&args()).expect("host binds");
        let addr = server.local_addr();

        let table = JoinCodeTable::read(std::path::Path::new(JOIN_TABLE)).expect("the table reads");
        let (service, code) = DirectJoinService::open(table).expect("the service opens");
        server.on_upgrade(Arc::new(service.gate()));

        let shutdown = ShutdownSignal::new();
        let serving = shutdown.clone();
        let delivery = std::thread::spawn(move || {
            let _ = server.serve_until(serving, |_| {});
        });

        let transport = Arc::new(Mutex::new(RelayTransport::new(
            service,
            RelayHostConfig {
                namespace: "client".to_string(),
                version: None,
                stamp: content.manifest.stamp.clone(),
            },
        )));
        let events: Arc<Mutex<Vec<TransportEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let notices: Arc<Mutex<Vec<RelayNotice>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        // A native host polls once per frame; this is that loop with nothing
        // else in it. On a thread rather than interleaved by hand because the
        // joiner's socket reads block, and a test that alternated the two by
        // hand would be asserting on its own scheduling.
        let t = Arc::clone(&transport);
        let e = Arc::clone(&events);
        let n = Arc::clone(&notices);
        let s = Arc::clone(&stop);
        let pump = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                {
                    let mut guard = t.lock().expect("transport poisoned");
                    e.lock().unwrap().extend(guard.poll());
                    n.lock().unwrap().extend(guard.drain_notices());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        Self {
            addr,
            code,
            stamp: content.manifest.stamp,
            transport,
            events,
            notices,
            stop,
            shutdown,
            pump: Some(pump),
            delivery: Some(delivery),
        }
    }

    fn join_url(&self) -> String {
        format!("ws://{}/v1/join", self.addr)
    }

    /// Wait until the host half has produced `want` events, then take them.
    fn events_until(&self, want: usize) -> Vec<TransportEvent> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            {
                let events = self.events.lock().unwrap();
                if events.len() >= want {
                    return events.clone();
                }
            }
            assert!(
                Instant::now() < deadline,
                "the host never saw {want} event(s); it saw {:?}",
                self.events.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn notices_until<T>(&self, mut f: impl FnMut(&RelayNotice) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            {
                for notice in self.notices.lock().unwrap().iter() {
                    if let Some(found) = f(notice) {
                        return found;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "the host reported no such notice"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn dispatch(&self, target: &Target, msg: &ServerMessage, delivery: DeliveryClass) {
        self.transport
            .lock()
            .expect("transport poisoned")
            .dispatch(TransportDispatch {
                target,
                msg,
                delivery,
            });
    }

    fn handshake_payload(&self) -> String {
        encode_handshake_frame(&HandshakeFrame {
            kind: JOIN_HANDSHAKE.to_string(),
            data: HandshakeData {
                stamp: Some(format!(
                    "{}/{}/{}",
                    self.stamp.protocol, self.stamp.content_id, self.stamp.content_epoch
                )),
                ..Default::default()
            },
        })
        .expect("encodable")
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.shutdown.stop();
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
        if let Some(delivery) = self.delivery.take() {
            let _ = delivery.join();
        }
    }
}

/// A raw client socket on `/v1/join`, standing in for a phone.
struct Joiner {
    socket: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
}

impl Joiner {
    fn connect(host: &Host) -> Self {
        // No `Origin` header, deliberately: this leg has no allow-list, and it
        // must not grow one — the phone and the socket are the same origin by
        // construction, so a list of every name a machine answers to would
        // refuse crews for no defence. See `native_host::direct_join`'s header.
        let (socket, response) = tungstenite::connect(host.join_url())
            .unwrap_or_else(|e| panic!("cannot dial {}: {e}", host.join_url()));
        assert_eq!(response.status().as_u16(), 101, "the host upgraded");
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
            // So `wait_for` fails with a stated reason instead of blocking for
            // ever on a frame that is never coming.
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        }
        Self { socket }
    }

    fn send(&mut self, frame: &RendezvousFrame) {
        let text = encode_rendezvous_frame(frame).expect("encodable");
        self.socket
            .send(tungstenite::Message::Text(text.into()))
            .expect("the socket takes a frame");
    }

    fn relay(&mut self, payload: String) {
        self.send(&RendezvousFrame {
            class: Some(CLASS_RELIABLE.to_string()),
            payload: Some(payload),
            ..RendezvousFrame::new("relay")
        });
    }

    /// Read frames until one of `kind` arrives. An `error` is fatal unless it
    /// IS the frame being waited for.
    fn wait_for(&mut self, kind: &str) -> RendezvousFrame {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "no {kind} frame arrived");
            match self.socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    let frame = decode_rendezvous_frame(&text)
                        .unwrap_or_else(|e| panic!("undecodable frame {text}: {e}"));
                    if frame.kind == kind {
                        return frame;
                    }
                    if frame.kind == "error" {
                        panic!("the host refused a request: {:?}", frame.reason);
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => panic!("the join socket died waiting for {kind}: {e}"),
            }
        }
    }

    /// Get as far as a phone gets before it has said anything about itself:
    /// resolved, attached, and admitted by the compatibility handshake.
    fn admitted(host: &Host) -> Self {
        let mut joiner = Joiner::connect(host);
        joiner.wait_for("ready");
        joiner.send(&RendezvousFrame {
            code: Some(CodeField::Typed(host.code.full.clone())),
            ..RendezvousFrame::new("join")
        });
        let joined = joiner.wait_for("joined");
        assert_eq!(
            joined.transports,
            vec![TRANSPORT_WS_RELAY.to_string()],
            "a native host must advertise that the relay is the only way in"
        );
        joiner.send(&RendezvousFrame::new("relay-open"));
        assert!(
            joiner.wait_for("relay-ready").limits.is_some(),
            "both ends are told one ceiling"
        );
        joiner.relay(host.handshake_payload());
        let verdict = decode_handshake_frame(
            joiner
                .wait_for("relay")
                .payload
                .as_deref()
                .unwrap_or_default(),
        )
        .expect("a decodable verdict");
        assert_eq!(verdict.kind, JOIN_ACCEPTED, "got {verdict:?}");
        joiner
    }
}

/// One HTTP request/response against the host, on a fresh socket.
fn http(addr: &str, head: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connects to the host");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("a bounded read");
    stream.write_all(head.as_bytes()).expect("writes a request");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    String::from_utf8_lossy(&response).into_owned()
}

#[test]
fn a_phone_joins_a_native_host_with_no_external_service_anywhere() {
    // Issue #1353's whole acceptance criterion, end to end: a client dials the
    // same origin it would have loaded the bundle from, is validated, is
    // attached, plays, and its unplugging is one clean Disconnected.
    let host = Host::start();
    assert_eq!(
        host.code.suffix.chars().count(),
        5,
        "the code is on the viewscreen before anybody has scanned anything"
    );

    let mut joiner = Joiner::admitted(&host);

    // The ordinary crew protocol, with no second protocol anywhere: the same
    // `Identify` a phone sends over the cloud relay reaches the simulation's
    // inbound bus under its own session token.
    joiner.relay(
        JsonCodec
            .encode_client(&ClientMessage::Identify {
                token: "lan-token".to_string(),
                name: "Ada".to_string(),
            })
            .expect("encodable"),
    );
    assert_eq!(
        host.events_until(1),
        vec![TransportEvent::Received {
            token: "lan-token".to_string(),
            msg: ClientMessage::Identify {
                token: "lan-token".to_string(),
                name: "Ada".to_string(),
            },
        }]
    );

    // …and the other direction, through the audience resolution the host half
    // already owned.
    host.dispatch(
        &Target::All,
        &ServerMessage::GameStarted,
        DeliveryClass::Reliable,
    );
    let outbound = joiner.wait_for("relay");
    assert_eq!(
        JsonCodec
            .decode_server(outbound.payload.as_deref().unwrap_or_default())
            .expect("a decodable ServerMessage"),
        ServerMessage::GameStarted
    );

    // A snapshot is carried too, and on its own class — the lossy one, which is
    // what keeps a late tick from queuing behind a backlog.
    host.dispatch(
        &Target::Token("lan-token".to_string()),
        &ServerMessage::GameStarted,
        DeliveryClass::Snapshot,
    );
    assert_eq!(
        joiner.wait_for("relay").class.as_deref(),
        Some("snapshot"),
        "the delivery class survives the trip"
    );

    // Unplugged. The socket dying is the whole signal — there is no in-band
    // goodbye from a phone that lost radio — and it must produce exactly one
    // disconnect for the token that held the station.
    drop(joiner);
    let events = host.events_until(2);
    assert_eq!(
        events[1],
        TransportEvent::Disconnected {
            token: "lan-token".to_string()
        }
    );
}

#[test]
fn the_reserved_tokens_are_refused_at_this_ingress_too() {
    // The seam's own documented obligation (`native_host::transport`), and the
    // one authorisation decision a transport makes for itself. A peer left
    // attached under `__local_console__` would sit in `audience()` receiving
    // that token's projection and carry host mission-abort authority.
    let host = Host::start();
    let mut joiner = Joiner::admitted(&host);
    joiner.relay(
        JsonCodec
            .encode_client(&ClientMessage::Identify {
                token: "__local_console__".to_string(),
                name: "impostor".to_string(),
            })
            .expect("encodable"),
    );
    let refusal = decode_handshake_frame(
        joiner
            .wait_for("relay")
            .payload
            .as_deref()
            .unwrap_or_default(),
    )
    .expect("a decodable refusal");
    assert_eq!(refusal.kind, JOIN_REFUSED);
    assert_eq!(refusal.data.code.as_deref(), Some("reserved-token"));
    assert!(
        host.events.lock().unwrap().is_empty(),
        "nothing claiming a reserved token reaches the simulation"
    );
    // …and the peer stays refused rather than merely having that one message
    // dropped: it holds no token, so it is out of `audience()` at once and
    // anything else it sends is ignored. Exactly what the cloud leg does with
    // the same claim, because it is the same code doing it.
    joiner.relay(
        JsonCodec
            .encode_client(&ClientMessage::Identify {
                token: "second-try".to_string(),
                name: "impostor".to_string(),
            })
            .expect("encodable"),
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        host.events.lock().unwrap().is_empty(),
        "a refused peer stays refused"
    );
}

#[test]
fn a_build_the_host_would_refuse_is_refused_here_for_the_same_reason() {
    // The join stamp, answered from `delivery::check_join_stamp` — the same
    // authority the browser host asks through `wasm_check_client_stamp`, so a
    // build refused in a browser is refused here with the same machine code.
    let host = Host::start();
    let mut joiner = Joiner::connect(&host);
    joiner.wait_for("ready");
    joiner.send(&RendezvousFrame {
        code: Some(CodeField::Typed(host.code.full.clone())),
        ..RendezvousFrame::new("join")
    });
    joiner.wait_for("joined");
    joiner.send(&RendezvousFrame::new("relay-open"));
    joiner.wait_for("relay-ready");
    joiner.relay(
        encode_handshake_frame(&HandshakeFrame {
            kind: JOIN_HANDSHAKE.to_string(),
            data: HandshakeData {
                stamp: Some("9999/another-content-set/7".to_string()),
                ..Default::default()
            },
        })
        .expect("encodable"),
    );
    let verdict = decode_handshake_frame(
        joiner
            .wait_for("relay")
            .payload
            .as_deref()
            .unwrap_or_default(),
    )
    .expect("a decodable verdict");
    assert_eq!(verdict.kind, JOIN_REFUSED);
    assert!(
        verdict.data.code.is_some(),
        "the refusal names a machine code the phone maps to a sentence"
    );
    host.notices_until(|n| match n {
        RelayNotice::Refused { .. } => Some(()),
        _ => None,
    });
}

#[test]
fn the_code_and_the_protocol_version_are_checked_as_the_worker_checks_them() {
    let host = Host::start();

    // A code that names no record.
    let mut joiner = Joiner::connect(&host);
    joiner.wait_for("ready");
    joiner.send(&RendezvousFrame {
        code: Some(CodeField::Typed("XYZAB".to_string())),
        ..RendezvousFrame::new("join")
    });
    let refusal = joiner.wait_for("error");
    assert_eq!(refusal.request.as_deref(), Some("join"));
    assert_eq!(refusal.reason.as_deref(), Some("unknown"));

    // A frame from a vocabulary this build does not speak.
    let mut joiner = Joiner::connect(&host);
    joiner.wait_for("ready");
    let mut ahead = RendezvousFrame::new("join");
    ahead.v = RENDEZVOUS_PROTOCOL + 1;
    ahead.code = Some(CodeField::Typed(host.code.full.clone()));
    joiner.send(&ahead);
    assert_eq!(
        joiner.wait_for("error").reason.as_deref(),
        Some("unsupported-protocol")
    );
}

#[test]
fn a_malformed_upgrade_cannot_stall_the_delivery_of_the_bundle() {
    // The accept-loop robustness criterion. Each of these would be a socket
    // waiting for bytes that are not coming if it reached a handshake; the
    // door refuses them in HTTP and the connection closes, and the very next
    // request for the manifest is served as if nothing had happened.
    let host = Host::start();
    let addr = host.addr.clone();

    let plain = http(
        &addr,
        &format!("GET /v1/join HTTP/1.1\r\nHost: {addr}\r\n\r\n"),
    );
    assert!(
        plain.starts_with("HTTP/1.1 426"),
        "a plain GET of the join endpoint is told what it is for: {plain}"
    );

    let no_key = http(
        &addr,
        &format!(
            "GET /v1/join HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\r\n"
        ),
    );
    assert!(
        no_key.starts_with("HTTP/1.1 400"),
        "an upgrade with no key is a clean 400: {no_key}"
    );

    let old_version = http(
        &addr,
        &format!(
            "GET /v1/join HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Version: 8\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        ),
    );
    assert!(old_version.starts_with("HTTP/1.1 400"), "{old_version}");

    // The bundle path is untouched by any of it…
    let stamp = http(
        &addr,
        &format!("GET /host/stamp.json HTTP/1.1\r\nHost: {addr}\r\n\r\n"),
    );
    assert!(
        stamp.starts_with("HTTP/1.1 200"),
        "delivery still serves: {stamp}"
    );
    // …and a real joiner still gets in afterwards, which is the claim that
    // matters: nothing about the refusals poisoned the door.
    let _ = Joiner::admitted(&host);
}

#[test]
fn a_socket_that_upgrades_and_says_nothing_holds_nothing_open() {
    // Slow-loris, past the upgrade: the head-read timeout bounds the HTTP half
    // and the socket cap bounds this one, so a silent joiner costs one slot and
    // no crew member is refused because of it.
    let host = Host::start();
    let _silent = Joiner::connect(&host);
    let _ = Joiner::admitted(&host);
}
