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
use project_phoenix::native_host::direct_join::{AdmissionBudgets, DirectJoinService};
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
        Self::start_with(AdmissionBudgets::default())
    }

    /// The same host with the transport-plane budgets named — the seam the
    /// attack tests below drive, so a bucket empties and a breaker ramps
    /// inside a test's patience rather than over a production minute. Nothing
    /// ships test-sized numbers: [`Self::start`] takes the defaults.
    fn start_with(budgets: AdmissionBudgets) -> Self {
        let content = load_content(".", MANIFEST).expect("the repo's own content loads");
        let server = HostServer::bind(&args()).expect("host binds");
        let addr = server.local_addr();

        let table = JoinCodeTable::read(std::path::Path::new(JOIN_TABLE)).expect("the table reads");
        let (service, code) =
            DirectJoinService::open_with_budgets(table, budgets).expect("the service opens");
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

    /// [`Self::connect`], but riding out a door that is momentarily busy.
    ///
    /// What a real client does with a 503 on the upgrade: the refusal is soft
    /// and carries `Retry-After`, and `gui/connection-manager.js` comes back on
    /// its own backoff. A test that treated one busy answer as a failure would
    /// be asserting the opposite of the property the soft limits are for.
    fn connect_soon(host: &Host) -> Self {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match tungstenite::connect(host.join_url()) {
                Ok((socket, response)) if response.status().as_u16() == 101 => {
                    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    }
                    return Self { socket };
                }
                other => {
                    assert!(
                        Instant::now() < deadline,
                        "the door never opened for a legitimate joiner: {other:?}"
                    );
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
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
        8,
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
        code: Some(CodeField::Typed("XYZABCDE".to_string())),
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
    // and the un-joined budget bounds this one, so a silent joiner costs one
    // slot and no crew member is refused because of it.
    let host = Host::start();
    let _silent = Joiner::connect(&host);
    let _ = Joiner::admitted(&host);
}

// ── Guessing the code, empirically ──────────────────────────────────────────
//
// The security review's finding, and its own probe shape reproduced as a test.
// `max_lookups_per_connection` is charged to a SOCKET, so a caller willing to
// reconnect is not rate-limited at all: the reviewer measured ~2,340 wrong
// guesses a second across churned connections, which walked the old
// typed keyspace (25^5, 23.2 bits) in ~35 minutes. What is asserted here
// is the fix working end to end on a real socket, not the arithmetic — the
// arithmetic is in `AdmissionBudgets`'s note and in the authored table.

/// A wrong code, of the authored length and in the authored alphabet — so it
/// reaches the lookup rather than being turned back by `validateSuffix`'s
/// length or charset rules, which would measure nothing.
fn wrong_code(n: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKMNOPQRSTUVWXYZ";
    let mut out = String::new();
    let mut v = n;
    for _ in 0..8 {
        out.push(ALPHABET[v % ALPHABET.len()] as char);
        v /= ALPHABET.len();
    }
    out
}

/// What one guessing run got out of the service.
#[derive(Clone, Copy, Debug, Default)]
struct Storm {
    /// Wrong codes the service actually EVALUATED — the throughput that
    /// matters, because only these walk the keyspace.
    guesses: usize,
    /// Answers that were a budget refusing rather than a lookup happening.
    refused: usize,
    /// Upgrades the door would not take at all.
    turned_away: usize,
}

/// The budgets as they were BEFORE this round: every cross-connection limit
/// off, leaving only the authored per-connection cap that a reconnect resets.
fn unlimited() -> AdmissionBudgets {
    AdmissionBudgets {
        guess_burst: u32::MAX,
        unjoined_per_source: 4096,
        unjoined_total: 4096,
        breaker_free: u32::MAX,
        breaker_step: Duration::ZERO,
        breaker_max: Duration::ZERO,
        ..AdmissionBudgets::default()
    }
}

/// Read frames until an `error` answers, or `until` passes.
fn wait_for_answer(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    until: Instant,
) -> Option<String> {
    while Instant::now() < until {
        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                let frame = decode_rendezvous_frame(&text).ok()?;
                if frame.kind == "error" {
                    return frame.reason;
                }
                if frame.kind == "joined" {
                    return Some("joined".to_string());
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return None,
        }
    }
    None
}

/// Churn connections firing wrong codes for `window`, on `workers` threads.
///
/// Deliberately the reviewer's shape rather than one long socket: a few guesses
/// per connection and then throw it away, which is what made the per-connection
/// cap a batch size instead of a limit.
fn guess_storm(url: &str, right: &str, window: Duration, workers: usize) -> Storm {
    const GUESSES_PER_CONNECTION: usize = 8;
    let deadline = Instant::now() + window;
    let mut threads = Vec::new();
    for worker in 0..workers {
        let url = url.to_string();
        let right = right.to_string();
        threads.push(std::thread::spawn(move || {
            let mut local = Storm::default();
            let mut n = worker * 1_000_000;
            while Instant::now() < deadline {
                let Ok((mut socket, response)) = tungstenite::connect(&url) else {
                    local.turned_away += 1;
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                if response.status().as_u16() != 101 {
                    local.turned_away += 1;
                    continue;
                }
                if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
                }
                for _ in 0..GUESSES_PER_CONNECTION {
                    if Instant::now() >= deadline {
                        break;
                    }
                    n += 1;
                    let code = wrong_code(n);
                    if code == right {
                        continue;
                    }
                    let frame = RendezvousFrame {
                        code: Some(CodeField::Typed(code)),
                        ..RendezvousFrame::new("join")
                    };
                    let text = encode_rendezvous_frame(&frame).expect("encodable");
                    if socket
                        .send(tungstenite::Message::Text(text.into()))
                        .is_err()
                    {
                        break;
                    }
                    // Generous per-guess patience, so a throttled answer is
                    // counted as the slow answer it is rather than as a
                    // timeout that would flatter the fix.
                    match wait_for_answer(&mut socket, Instant::now() + Duration::from_secs(4)) {
                        Some(reason) if reason == "too-many-attempts" => {
                            local.refused += 1;
                            break;
                        }
                        Some(_) => local.guesses += 1,
                        None => break,
                    }
                }
                let _ = socket.close(None);
                let _ = socket.flush();
            }
            local
        }));
    }
    let mut total = Storm::default();
    for t in threads {
        let local = t.join().expect("a storm worker");
        total.guesses += local.guesses;
        total.refused += local.refused;
        total.turned_away += local.turned_away;
    }
    total
}

#[test]
fn guessing_the_code_by_reconnecting_collapses_under_the_shipped_budgets() {
    // The blocker, measured. The SAME probe runs against two hosts that differ
    // only in their transport-plane budgets, so the number is a before/after of
    // this round's fix rather than of the machine it ran on.
    let window = Duration::from_millis(1500);

    let before = {
        let host = Host::start_with(unlimited());
        guess_storm(&host.join_url(), &host.code.suffix, window, 4)
    };
    let after = {
        let host = Host::start();
        guess_storm(&host.join_url(), &host.code.suffix, window, 4)
    };
    println!("guess storm before={before:?} after={after:?}");

    assert!(
        before.guesses > 200,
        "the probe has to actually work against an unbudgeted door, or this \
         test proves nothing: {before:?}"
    );
    // The per-source bucket is the bound, and it is the bound whatever the
    // connections do: a burst, then a refill clock nobody can hurry.
    let budgets = AdmissionBudgets::default();
    let ceiling = budgets.guess_burst as usize
        + (window.as_millis() / budgets.guess_refill.as_millis()) as usize
        + 2;
    assert!(
        after.guesses <= ceiling,
        "the whole run may evaluate at most the burst plus what refilled \
         ({ceiling}), however many sockets it spread itself over: {after:?}"
    );
    assert!(
        after.guesses * 5 < before.guesses,
        "and that is a collapse, not a trim: {before:?} → {after:?}"
    );
    assert!(
        after.refused > 0,
        "the budget said so in band, rather than dropping sockets silently: \
         {after:?}"
    );
}

#[test]
fn the_crew_still_joins_from_the_address_the_guessing_is_coming_from() {
    // The point of every limit here being SOFT, at the sharpest angle the
    // no-Origin decision leaves standing: on loopback the guesser and the crew
    // member ARE the same address, exactly as a hostile page open in a crew
    // member's own browser would be. A correct code costs no budget, so the
    // guest gets in WHILE the guessing is going on.
    let host = Host::start();
    let url = host.join_url();
    let right = host.code.suffix.clone();
    let storm = std::thread::spawn(move || guess_storm(&url, &right, Duration::from_secs(3), 2));

    // Give the storm long enough to have emptied the bucket and started
    // collecting refusals before the crew member tries at all.
    std::thread::sleep(Duration::from_millis(300));

    let joined_at = Instant::now();
    let mut joiner = Joiner::connect_soon(&host);
    joiner.wait_for("ready");
    joiner.send(&RendezvousFrame {
        code: Some(CodeField::Typed(host.code.full.clone())),
        ..RendezvousFrame::new("join")
    });
    let joined = joiner.wait_for("joined");
    let waited = joined_at.elapsed();
    println!("legit join during the attack took {waited:?}");
    assert_eq!(joined.admission.as_deref(), Some("open"));
    assert!(
        waited < Duration::from_secs(8),
        "and inside the client's own first connect timeout, not merely \
         eventually: {waited:?}"
    );

    // …all the way to an acting participant, not just past the code check.
    joiner.send(&RendezvousFrame::new("relay-open"));
    joiner.wait_for("relay-ready");
    joiner.relay(host.handshake_payload());
    let verdict = decode_handshake_frame(
        joiner
            .wait_for("relay")
            .payload
            .as_deref()
            .unwrap_or_default(),
    )
    .expect("a decodable verdict");
    assert_eq!(verdict.kind, JOIN_ACCEPTED);

    let storm = storm.join().expect("the storm ends");
    println!("storm alongside the join: {storm:?}");
    assert!(
        storm.refused > 0,
        "the guessing really was being refused throughout: {storm:?}"
    );
}

#[test]
fn a_crew_sharing_one_address_survives_its_own_typos() {
    // The sizing argument for `guess_burst`, asserted rather than asserted-to:
    // a whole crew behind one NAT fumbling the code must not lock the room out.
    // Only FAILED lookups are charged, so the budget is a typo allowance.
    let host = Host::start();
    let budgets = AdmissionBudgets::default();
    let typos = budgets.guess_burst as usize - 4;
    for n in 0..typos {
        let mut fumbling = Joiner::connect(&host);
        fumbling.wait_for("ready");
        fumbling.send(&RendezvousFrame {
            code: Some(CodeField::Typed(wrong_code(n))),
            ..RendezvousFrame::new("join")
        });
        assert_eq!(
            fumbling.wait_for("error").reason.as_deref(),
            Some("unknown"),
            "typo {n} is answered as a wrong code, not as an accusation"
        );
    }
    // …and the next person, on that same address, reads the viewscreen
    // correctly and is in.
    let _ = Joiner::admitted(&host);
}

#[test]
fn a_device_hoarding_silent_sockets_cannot_spend_the_rooms_places() {
    // The secondary finding. Un-joined sockets used to be counted against the
    // authored `max_peers_per_record`, so one device holding thirty-two silent
    // sockets refused the crew with `join-sockets-full`. Silence now has a
    // budget of its own, and crossing it is a stated, retryable refusal.
    let host = Host::start_with(AdmissionBudgets {
        unjoined_per_source: 3,
        unjoined_total: 6,
        ..AdmissionBudgets::default()
    });

    // Three real crew members, admitted. They cost the RECORD's peer cap, and
    // pointedly not the un-joined budget — which is what the next step proves.
    let _crew: Vec<Joiner> = (0..3).map(|_| Joiner::admitted(&host)).collect();

    let mut silent = Vec::new();
    for _ in 0..3 {
        silent.push(Joiner::connect(&host));
    }
    let refused = http(&host.addr, &upgrade_head(&host.addr));
    assert!(
        refused.starts_with("HTTP/1.1 503"),
        "the fourth silent socket is refused: {refused}"
    );
    assert!(
        refused.contains("join-sockets-busy"),
        "…by the budget that is about silence, and it says which: {refused}"
    );
    assert!(
        refused.contains("Retry-After"),
        "…softly, with the truth that it is a budget filling back up: {refused}"
    );

    // A slot frees the instant a silent socket closes: nothing here is a ban.
    silent.pop();
    let freed = Instant::now() + Duration::from_secs(5);
    loop {
        let again = http(&host.addr, &upgrade_head(&host.addr));
        if again.starts_with("HTTP/1.1 101") {
            break;
        }
        assert!(Instant::now() < freed, "the freed slot never came back");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A WebSocket upgrade head for `/v1/join`, as `delivery::serve` wants it.
fn upgrade_head(addr: &str) -> String {
    format!(
        "GET /v1/join HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    )
}

#[test]
fn a_joiner_that_stops_reading_is_detached_rather_than_parking_a_thread() {
    // The minor finding: only a READ timeout was set, so a joiner that stalled
    // its own reads parked this host's thread inside `send` for as long as the
    // OS would hold a full send buffer — minutes, with the peer still in the
    // host's audience. The write timeout bounds it, and a timed-out write ends
    // the same way a reliable-queue overflow does: detached, and said so.
    let host = Host::start_with(AdmissionBudgets {
        write_timeout: Duration::from_millis(300),
        ..AdmissionBudgets::default()
    });
    let mut joiner = Joiner::admitted(&host);
    joiner.relay(
        JsonCodec
            .encode_client(&ClientMessage::Identify {
                token: "stalled".to_string(),
                name: "Ada".to_string(),
            })
            .expect("encodable"),
    );
    host.events_until(1);

    // From here the joiner reads NOTHING, and the host writes more than any
    // socket buffer will hold. Reliable, deliberately: the snapshot class is
    // allowed to shed, and shedding is not what is being tested.
    for _ in 0..8 {
        host.dispatch(
            &Target::Token("stalled".to_string()),
            &ServerMessage::NameChanged {
                token: "stalled".to_string(),
                name: "x".repeat(200_000),
            },
            DeliveryClass::Reliable,
        );
    }

    let events = host.events_until(2);
    assert_eq!(
        events[1],
        TransportEvent::Disconnected {
            token: "stalled".to_string()
        },
        "the stalled peer is detached, not left holding a thread"
    );
    drop(joiner);
}
