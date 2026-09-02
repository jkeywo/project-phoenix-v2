//! The host is its own rendezvous: LAN joins accepted directly (issue #1353).
//!
//! # What this is
//!
//! A native host already accepts inbound HTTP on its delivery port — that is
//! how a phone gets the client bundle in the first place. This module makes it
//! accept the game socket on the SAME port: an upgrade on `/v1/join` is taken
//! off the HTTP path by [`crate::delivery::serve`]'s
//! [`ConnectionUpgrade`](crate::delivery::serve::ConnectionUpgrade) door, and
//! everything the rendezvous worker would have done for that joiner is done
//! here, in process. A LAN game then needs **zero external services**: no
//! worker deployed, no `--rendezvous`, no internet.
//!
//! Both hosts dialled OUT to a meet-in-the-middle relay until now, and that
//! shape was inherited rather than chosen: a BROWSER host cannot accept an
//! inbound connection, so the only thing two browsers can do is meet at a third
//! party. A native process can accept, and issue #1121 deferred exactly this
//! leg (see the module note in [`crate::native_host::transport`]).
//!
//! # The trick: the host half is already written
//!
//! [`RelayTransport`](crate::native_host::relay_transport::RelayTransport) is
//! the whole host end of the rendezvous protocol — registration, the in-band
//! compatibility handshake against `delivery::check_join_stamp`, the `Identify`
//! gate and its reserved-token refusal, the duplicate-token sever, audience
//! resolution, and the delivery-class shedding rule. It reaches its wire
//! through one narrow trait,
//! [`RelaySocket`](crate::native_host::relay_transport::RelaySocket).
//!
//! So the direct leg is **not** a second host implementation. It is a
//! `RelaySocket` whose far end is not a socket at all but this file: the
//! single-game subset of `worker-rendezvous/src/registry.js`, speaking the
//! service half to real joiner sockets on one side and the exact same frame
//! vocabulary to `RelayTransport` on the other. Nothing about how a native host
//! treats a crew member is written twice, and a phone cannot tell which leg it
//! came in on — which is the property the whole design is for.
//!
//! What collapses, compared with the worker: there is no `/v1/host` socket
//! (the host is this process), one record instead of a map, and no code
//! rotation or reclaim grace (a record whose host has gone is a process that
//! has exited). What does NOT collapse is the validation — join code, protocol
//! version, join stamp, the reserved-token gate at ingress — because a phone
//! must get the same answers from both legs or the two are different games.
//!
//! # Threads, and why there is no runtime
//!
//! One thread per joiner connection, exactly as `delivery::serve` runs one
//! thread per HTTP connection and [`crate::native_host::relay_socket`] runs one
//! for the outbound relay. Each owns its `tungstenite` socket, reads with a
//! short timeout, and drains that peer's outbox between reads — the same pump
//! shape, for the same reason: `RelaySocket::poll` runs in Bevy's `PreUpdate`
//! and may not block the simulation for a network round trip. `Cargo.toml`
//! records why this repository has no async runtime to reach for instead.
//!
//! # `[ai]` — no Origin allow-list on this leg
//!
//! The worker demands a present, allow-listed `Origin` on every upgrade
//! (`worker-rendezvous/src/index.js`), and that gate is right THERE: it is a
//! public, multi-tenant service on a different origin from every page it
//! serves, so a request without an `Origin` is precisely the scripted
//! non-browser caller the 403 exists to stop.
//!
//! Here the page and the socket are the SAME origin by construction — the
//! phone loaded the bundle from this host and dials the host it loaded from —
//! so an allow-list would have to be a list of every name and address a phone
//! might reach this machine by (its IPv4, its IPv6, its mDNS name, whatever the
//! operator passed to `--addr`), and getting it wrong refuses the whole crew
//! with nothing on screen saying why. That is the 2026-08 TURN incident's
//! failure mode, bought for no defence: this port already hands the entire
//! client bundle to anyone on the LAN, and what actually gates a JOIN is the
//! five-letter code, the compatibility handshake and the reserved-token gate,
//! all of which are applied below.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{mpsc, Arc, Mutex};

use crate::core::codec::{decode_rendezvous_frame, encode_rendezvous_frame};
use crate::core::rendezvous::{
    CodeField, JoinCode, RelayLimits, RendezvousFrame, CLASS_RELIABLE, CLASS_SNAPSHOT,
    RENDEZVOUS_PROTOCOL, TRANSPORT_WS_RELAY,
};
use crate::delivery::http::Request;
use crate::delivery::serve::{ConnectionUpgrade, UpgradeRefusal};
use crate::native_host::join_codes::{JoinCodeTable, NAMESPACE_CLIENT, NAMESPACE_SERVER};
use crate::native_host::relay_transport::RelaySocket;

/// The endpoint a joiner dials, and the one this module claims off the HTTP
/// path. The same path the worker serves and the same one
/// `gui/rendezvous-transport.js` builds with `socketUrl(base, '/v1/join')` — a
/// phone dials one string whichever service is behind it.
pub const JOIN_PATH: &str = "/v1/join";

/// How long a joiner's thread waits for a frame before draining its outbox.
///
/// The same cadence [`crate::native_host::relay_socket`]'s pump uses, for the
/// same reason: short enough that an outbound burst is never held behind a
/// quiet link for more than a couple of simulation frames, long enough that an
/// idle joiner is not a spin loop. Not a gameplay value — it is one socket
/// thread's own scheduling (AGENTS.md rule 11).
const PUMP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(20);

/// How long an accepted socket may hold a thread without ever joining.
///
/// The upgrade itself is bounded by `delivery::serve`'s head-read timeout, and
/// the number of live join sockets is bounded by the authored
/// `max_peers_per_record`. This is the third leg: without it, that cap could be
/// filled by sockets that upgraded and then said nothing, and the crew standing
/// in the room would be refused by a full table of silence. A phone that has
/// scanned the QR sends `join` in the same breath as the upgrade, so the window
/// is generous. Not a gameplay value (AGENTS.md rule 11).
const JOIN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// The peer id this service uses for the host itself, on the `from` field of a
/// frame it hands a joiner. The worker puts the host's connection id there; a
/// joiner does not read it (`gui/rendezvous-relay.js` routes on `class`), so
/// what matters is only that it is stable and cannot collide with a joiner's.
const HOST_PEER: &str = "host";

/// The prefix every peer id this leg mints carries.
///
/// Peer-namespace separation, and it is load-bearing the moment a host runs
/// BOTH legs (`--rendezvous` plus direct accept): the two
/// `RelayTransport`s keep separate peer maps, and this prefix means a
/// worker-minted UUID and a directly-accepted joiner could not be confused even
/// if they were ever put in one table — by an operator reading a log, if
/// nothing else.
const PEER_PREFIX: &str = "lan-";

// ── The shared record ───────────────────────────────────────────────────────

/// One joiner's outbox, written by the simulation thread and drained by that
/// joiner's own connection thread.
///
/// The two classes are queued separately because they promise different things,
/// and this is where the promise is kept on THIS side of the wire — the mirror
/// of `worker-rendezvous/src/relay.js`'s mailboxes:
///
/// * **reliable** is ordered and never shed. A queue past its authored depth
///   ends the session with a stated reason rather than becoming a quietly lossy
///   reliable channel — a dropped command would fork the guarantee the
///   simulation is written against.
/// * **snapshot** is latest-wins. Past its depth the OLDEST frames go, because
///   a late snapshot is worthless and the next tick supersedes it.
struct PeerOutbox {
    queue: Mutex<OutQueue>,
    /// Bytes queued and not yet written, across both classes. The backpressure
    /// signal `RelayTransport` sheds snapshots against, summed over peers by
    /// [`DirectJoinService::buffered_bytes`].
    queued: AtomicUsize,
    /// Set when this peer is finished: the host closed it, the service is
    /// shutting down, or its reliable queue overflowed. The connection thread
    /// notices within one [`PUMP_READ_TIMEOUT`] and closes the socket.
    closed: AtomicBool,
    /// Snapshot frames this outbox has displaced, for the `relay-degraded`
    /// notice. A running total, not a delta: a queue at its bound sheds one
    /// frame per arrival, so a delta reads "1" forever however many were lost.
    dropped: AtomicU64,
    max_reliable: usize,
    max_snapshot: usize,
}

#[derive(Default)]
struct OutQueue {
    reliable: VecDeque<String>,
    snapshot: VecDeque<String>,
}

/// What enqueuing one frame did.
enum Enqueued {
    /// Queued; this many snapshot frames were displaced by it.
    Ok { dropped: u64 },
    /// The reliable queue is past its authored depth: this session is over.
    Overflowed,
}

impl PeerOutbox {
    fn new(limits: &crate::native_host::join_codes::RawLimits) -> Self {
        Self {
            queue: Mutex::new(OutQueue::default()),
            queued: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            max_reliable: limits.max_relay_queue_reliable,
            max_snapshot: limits.max_relay_queue_snapshot,
        }
    }

    fn push(&self, class: &str, text: String) -> Enqueued {
        let len = text.len();
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if class == CLASS_SNAPSHOT {
            queue.snapshot.push_back(text);
            let mut dropped = 0u64;
            while queue.snapshot.len() > self.max_snapshot {
                if let Some(old) = queue.snapshot.pop_front() {
                    self.queued.fetch_sub(
                        old.len().min(self.queued.load(Ordering::Relaxed)),
                        Ordering::Relaxed,
                    );
                }
                dropped += 1;
            }
            self.queued.fetch_add(len, Ordering::Relaxed);
            self.dropped.fetch_add(dropped, Ordering::Relaxed);
            return Enqueued::Ok { dropped };
        }
        queue.reliable.push_back(text);
        self.queued.fetch_add(len, Ordering::Relaxed);
        if queue.reliable.len() > self.max_reliable {
            return Enqueued::Overflowed;
        }
        Enqueued::Ok { dropped: 0 }
    }

    /// Everything waiting, reliable first.
    ///
    /// The classes drain in a fixed order rather than interleaved, and reliable
    /// goes first deliberately: a command and the snapshot that reflects it must
    /// not swap places, while two snapshots are already free to.
    fn drain(&self) -> Vec<String> {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<String> = queue.reliable.drain(..).collect();
        out.extend(queue.snapshot.drain(..));
        let bytes: usize = out.iter().map(String::len).sum();
        self.queued.fetch_sub(
            bytes.min(self.queued.load(Ordering::Relaxed)),
            Ordering::Relaxed,
        );
        out
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

/// The one record this service holds, plus every attached joiner.
struct Record {
    table: JoinCodeTable,
    /// The code minted once, at bind. There is no rotation and no reclaim: a
    /// record whose host has gone is a process that has exited, so the two
    /// lifecycle features the worker grew for a socket that might come back
    /// (issue #1115) have nothing to be about here.
    code: JoinCode,
    /// Frames for the host half, drained by [`DirectJoinService::poll`].
    to_host: Mutex<Sender<String>>,
    /// Live joiner sockets, whether or not they have joined yet — this is the
    /// count `max_peers_per_record` bounds.
    sockets: AtomicUsize,
    /// Attached relay peers, by minted peer id.
    peers: Mutex<HashMap<String, Arc<PeerOutbox>>>,
    next_peer: AtomicU64,
    open: AtomicBool,
}

impl Record {
    fn limits_frame(&self) -> RelayLimits {
        RelayLimits {
            max_frame_bytes: self.table.limits.max_relay_frame_bytes,
            max_send_buffer_bytes: self.table.limits.max_relay_send_buffer_bytes,
        }
    }

    /// Hand one frame to the host half. Silently dropped once the host end has
    /// gone, which is a process on its way out rather than an error to report.
    fn tell_host(&self, frame: &RendezvousFrame) {
        if let Ok(text) = encode_rendezvous_frame(frame) {
            let _ = self
                .to_host
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(text);
        }
    }

    fn mint_peer_id(&self) -> String {
        let n = self.next_peer.fetch_add(1, Ordering::Relaxed);
        format!("{PEER_PREFIX}{n}")
    }

    /// Detach `peer` and tell the host it left — `detachRelay` in the registry,
    /// which likewise reports to the host whichever end asked for the close.
    fn detach(&self, peer: &str, reason: &str) {
        let removed = self
            .peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(peer);
        if let Some(outbox) = removed {
            outbox.close();
            self.tell_host(&RendezvousFrame {
                peer: Some(peer.to_string()),
                reason: Some(reason.to_string()),
                ..RendezvousFrame::new("relay-peer-left")
            });
        }
    }
}

// ── The service, as a RelaySocket ───────────────────────────────────────────

/// The in-process rendezvous, seen from the host half as an ordinary
/// [`RelaySocket`].
///
/// Wrap it in a
/// [`RelayTransport`](crate::native_host::relay_transport::RelayTransport)
/// exactly as [`crate::native_host::relay_socket::WsRelaySocket`] is wrapped,
/// and install [`Self::gate`] on the delivery server.
pub struct DirectJoinService {
    record: Arc<Record>,
    /// Behind a `Mutex` purely to be `Sync`, like `WsRelaySocket`'s: the only
    /// reader is `poll(&mut self)`, which takes it with `get_mut`.
    from_service: Mutex<Receiver<String>>,
}

impl DirectJoinService {
    /// Mint this host's code and open the service.
    ///
    /// Minting happens HERE, at bind, rather than on the first joiner: the code
    /// goes on the viewscreen before anybody has scanned anything, which is the
    /// whole point of the join panel (issue #1329).
    pub fn open(table: JoinCodeTable) -> Result<(Self, JoinCode), String> {
        let code = table
            .mint_client_code(crate::native_host::join_codes::os_draw)
            .ok_or_else(|| "the authored join-code table minted no usable code".to_string())?;
        let (tx, rx) = mpsc::channel::<String>();
        let record = Arc::new(Record {
            table,
            code: code.clone(),
            to_host: Mutex::new(tx),
            sockets: AtomicUsize::new(0),
            peers: Mutex::new(HashMap::new()),
            next_peer: AtomicU64::new(1),
            open: AtomicBool::new(true),
        });
        // The service's opening frame, queued before anything is polled — the
        // registry's `connect()` sends exactly this, and it is what makes
        // `RelayTransport` send its `host-open` and be answered with the code.
        record.tell_host(&RendezvousFrame {
            ..RendezvousFrame::new("ready")
        });
        Ok((
            Self {
                record,
                from_service: Mutex::new(rx),
            },
            code,
        ))
    }

    /// The door handler to install on the delivery server.
    pub fn gate(&self) -> DirectJoinGate {
        DirectJoinGate {
            record: Arc::clone(&self.record),
        }
    }

    /// The code this host minted.
    pub fn code(&self) -> &JoinCode {
        &self.record.code
    }
}

impl RelaySocket for DirectJoinService {
    fn poll(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let rx = self
            .from_service
            .get_mut()
            .unwrap_or_else(|e| e.into_inner());
        loop {
            match rx.try_recv() {
                Ok(text) => out.push(text),
                Err(TryRecvError::Empty) => break,
                // Every sender is held by the record this object owns, so this
                // cannot happen while the service is open; treat it as closed
                // rather than spinning.
                Err(TryRecvError::Disconnected) => {
                    self.record.open.store(false, Ordering::Relaxed);
                    break;
                }
            }
        }
        out
    }

    fn send(&mut self, text: String) {
        let Ok(frame) = decode_rendezvous_frame(&text) else {
            return;
        };
        let record = &self.record;
        match frame.kind.as_str() {
            // The host registering. The worker would mint here; this service
            // minted at bind, so it only reports.
            "host-open" => record.tell_host(&RendezvousFrame {
                code: Some(CodeField::Issued(Box::new(record.code.clone()))),
                admission: Some("open".to_string()),
                ..RendezvousFrame::new("hosted")
            }),
            "relay" => {
                let (Some(to), Some(payload)) = (frame.to.clone(), frame.payload.clone()) else {
                    return;
                };
                let class = frame.class.clone().unwrap_or_default();
                let outbox = record
                    .peers
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&to)
                    .cloned();
                let Some(outbox) = outbox else {
                    // The peer detached a tick ago and the host is still
                    // broadcasting to it. Exactly the registry's `no-peer`, and
                    // exactly the per-REQUEST refusal `RelayTransport` treats as
                    // a notice rather than as relay loss.
                    record.tell_host(&RendezvousFrame {
                        request: Some("relay".to_string()),
                        reason: Some("no-peer".to_string()),
                        ..RendezvousFrame::new("error")
                    });
                    return;
                };
                // `from`, not `to`: the frame is now travelling the other way,
                // and this is the shape the joiner's own relay reads
                // (`gui/rendezvous-relay.js`).
                let carried = encode_rendezvous_frame(&RendezvousFrame {
                    from: Some(HOST_PEER.to_string()),
                    class: Some(class.clone()),
                    payload: Some(payload),
                    ..RendezvousFrame::new("relay")
                });
                let Ok(carried) = carried else {
                    return;
                };
                match outbox.push(&class, carried) {
                    Enqueued::Ok { dropped } if dropped > 0 => {
                        record.tell_host(&RendezvousFrame {
                            peer: Some(to.clone()),
                            class: Some(class.clone()),
                            dropped: Some(outbox.dropped.load(Ordering::Relaxed) as u32),
                            ..RendezvousFrame::new("relay-degraded")
                        });
                    }
                    Enqueued::Ok { .. } => {}
                    Enqueued::Overflowed => {
                        // A reliable queue that filled is a session that can no
                        // longer keep the promise its class makes. End it with a
                        // reason both ends can render — `flushRelay`'s answer.
                        Self::close_peer(record, &to, "relay-overflow");
                    }
                }
            }
            // The host evicting one crew member: the reserved-token refusal and
            // the duplicate-token dance both come through here.
            "relay-close" => {
                if let Some(to) = frame.to {
                    Self::close_peer(record, &to, "host-closed");
                }
            }
            _ => {}
        }
    }

    fn buffered_bytes(&self) -> usize {
        self.record
            .peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|p| p.queued.load(Ordering::Relaxed))
            .sum()
    }

    fn is_open(&self) -> bool {
        self.record.open.load(Ordering::Relaxed)
    }

    fn close(&mut self) {
        self.record.open.store(false, Ordering::Relaxed);
        for outbox in self
            .record
            .peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            outbox.close();
        }
    }
}

impl DirectJoinService {
    /// Tell `peer` its relay is over, then detach it.
    ///
    /// Both halves matter and the order does: the joiner already treats
    /// `relay-closed` as the end of its game path, so it fails and re-enters its
    /// reconnect loop rather than sitting on a socket that will never carry
    /// another frame. Left one-sided, the phone would show "connected" for a
    /// link this host had already walked away from.
    fn close_peer(record: &Arc<Record>, peer: &str, reason: &str) {
        let outbox = record
            .peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(peer)
            .cloned();
        if let Some(outbox) = outbox {
            if let Ok(text) = encode_rendezvous_frame(&RendezvousFrame {
                reason: Some(reason.to_string()),
                ..RendezvousFrame::new("relay-closed")
            }) {
                let _ = outbox.push(CLASS_RELIABLE, text);
            }
        }
        record.detach(peer, reason);
    }
}

// ── The door ────────────────────────────────────────────────────────────────

/// The [`ConnectionUpgrade`] half: it answers `/v1/join` upgrades.
#[derive(Clone)]
pub struct DirectJoinGate {
    record: Arc<Record>,
}

impl ConnectionUpgrade for DirectJoinGate {
    fn serves(&self, path: &str) -> bool {
        path == JOIN_PATH
    }

    fn accept(
        &self,
        mut stream: TcpStream,
        _req: &Request,
        key: &str,
    ) -> Result<(), UpgradeRefusal> {
        if !self.record.open.load(Ordering::Relaxed) {
            return Err(refuse(&mut stream, 503, "join-closed"));
        }
        // The bound on live join sockets, and the reason it is on SOCKETS
        // rather than on joined peers: a socket that upgraded and never joined
        // still costs a thread. `max_peers_per_record` is the authored number
        // for "how much presence one record may hold", which is what this is.
        let live = self.record.sockets.load(Ordering::Relaxed);
        if live >= self.record.table.limits.max_peers_per_record {
            return Err(refuse(&mut stream, 503, "join-sockets-full"));
        }

        let accept_key = tungstenite::handshake::derive_accept_key(key.as_bytes());
        let head = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept_key}\r\n\r\n"
        );
        if stream.write_all(head.as_bytes()).is_err() || stream.flush().is_err() {
            return Err(UpgradeRefusal {
                status: 500,
                reason: "join-handshake-write-failed",
            });
        }
        if stream.set_read_timeout(Some(PUMP_READ_TIMEOUT)).is_err() {
            return Err(UpgradeRefusal {
                status: 500,
                reason: "join-socket-not-pollable",
            });
        }

        // `from_raw_socket`, not `accept`: `delivery::serve` has already read
        // the request head off this stream (that is how the path and the key
        // were known at all), so there is no handshake left for `tungstenite`
        // to read — only the 101 above to write, which is why this module
        // writes it. The alternative was peeking the socket without consuming
        // it, which std cannot do portably.
        let socket = tungstenite::WebSocket::from_raw_socket(
            stream,
            tungstenite::protocol::Role::Server,
            None,
        );
        let record = Arc::clone(&self.record);
        record.sockets.fetch_add(1, Ordering::Relaxed);
        let spawned = std::thread::Builder::new()
            .name("phoenix-join".to_string())
            .spawn(move || {
                serve_joiner(&record, socket);
                record.sockets.fetch_sub(1, Ordering::Relaxed);
            });
        if spawned.is_err() {
            self.record.sockets.fetch_sub(1, Ordering::Relaxed);
            return Err(UpgradeRefusal {
                status: 503,
                reason: "join-thread-unavailable",
            });
        }
        Ok(())
    }
}

/// Answer a refused upgrade as ordinary HTTP and report it.
///
/// Written before the 101, so the caller reads a status rather than watching a
/// socket close for no stated reason — the same courtesy the worker's 403 and
/// 426 extend.
fn refuse(stream: &mut TcpStream, status: u16, reason: &'static str) -> UpgradeRefusal {
    let body = reason;
    let head = crate::delivery::http::response_head(
        status,
        "Service Unavailable",
        "text/plain; charset=utf-8",
        crate::delivery::http::CachePolicy::Revalidate,
        body.len(),
        &[],
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
    UpgradeRefusal { status, reason }
}

// ── One joiner's connection ─────────────────────────────────────────────────

/// What this service knows about one joiner socket, on its own thread.
struct Joiner {
    /// The peer id this connection was minted, at accept.
    ///
    /// Minted once and for the whole connection, exactly as the worker's is its
    /// connection id: `joined` carries it, `relay-open` attaches under it, and
    /// every relayed frame names it. Minting one per frame would leave the host
    /// holding an id nothing ever detaches.
    id: String,
    /// True once `join` resolved the code.
    joined: bool,
    /// Its outbox, once `relay-open` attached it.
    outbox: Option<Arc<PeerOutbox>>,
    /// Lookups charged, against the authored per-connection cap.
    lookups: usize,
    /// Frames to write before the next read, for a connection that has not
    /// attached yet (and so has no outbox).
    pending: Vec<String>,
    /// Set by a refusal the service should also CLOSE the socket after sending
    /// — the registry's `cut()`, carried out here rather than by an adapter.
    cut: bool,
}

type JoinerSocket = tungstenite::WebSocket<TcpStream>;

/// Pump one joiner until its socket dies or the service closes it.
fn serve_joiner(record: &Arc<Record>, mut socket: JoinerSocket) {
    let mut joiner = Joiner {
        id: record.mint_peer_id(),
        joined: false,
        outbox: None,
        lookups: 0,
        pending: Vec::new(),
        cut: false,
    };
    let opened = std::time::Instant::now();
    loop {
        if !record.open.load(Ordering::Relaxed) {
            break;
        }
        if joiner.outbox.as_ref().is_some_and(|o| o.is_closed()) {
            // Drain whatever the host queued before it closed us — the
            // `relay-closed` frame is in there, and it is the difference
            // between a stated end and an unexplained drop.
            for text in joiner
                .outbox
                .as_ref()
                .map(|o| o.drain())
                .unwrap_or_default()
            {
                let _ = socket.send(tungstenite::Message::Text(text.into()));
            }
            break;
        }
        if !joiner.joined && opened.elapsed() > JOIN_DEADLINE {
            break;
        }

        for text in std::mem::take(&mut joiner.pending) {
            if socket
                .send(tungstenite::Message::Text(text.into()))
                .is_err()
            {
                return finish(record, &joiner, "closed");
            }
        }
        if joiner.cut {
            let _ = socket.close(None);
            let _ = socket.flush();
            return finish(record, &joiner, "refused");
        }
        if let Some(outbox) = &joiner.outbox {
            for text in outbox.drain() {
                if socket
                    .send(tungstenite::Message::Text(text.into()))
                    .is_err()
                {
                    return finish(record, &joiner, "closed");
                }
            }
        }

        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                on_client_frame(record, &mut joiner, &text);
            }
            Ok(tungstenite::Message::Close(_)) => break,
            // Binary, ping and pong are `tungstenite`'s own business or nothing
            // of ours: the protocol is JSON text, both ways, in every service.
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    let _ = socket.close(None);
    let _ = socket.flush();
    finish(record, &joiner, "closed");
}

/// This connection is over: detach it, which is what tells the host the crew
/// member has gone and turns into a `PlayerDisconnected` for whichever station
/// they held.
fn finish(record: &Arc<Record>, joiner: &Joiner, reason: &str) {
    // Safe for a connection that never attached: `detach` reports to the host
    // only for a peer it was actually holding, so a socket that upgraded and
    // said nothing produces no phantom departure.
    record.detach(&joiner.id, reason);
}

/// Queue one frame for a joiner, wherever it currently belongs.
fn reply(joiner: &mut Joiner, frame: &RendezvousFrame) {
    if let Ok(text) = encode_rendezvous_frame(frame) {
        match &joiner.outbox {
            Some(outbox) => {
                let _ = outbox.push(CLASS_RELIABLE, text);
            }
            None => joiner.pending.push(text),
        }
    }
}

/// The registry's `fail()`: one refusal, naming the request and one stable
/// machine reason, which `gui/join-code.js`'s `reasonStringId` maps to a
/// sentence. This service never ships prose.
fn fail(joiner: &mut Joiner, request: &str, reason: &str) {
    reply(
        joiner,
        &RendezvousFrame {
            request: Some(request.to_string()),
            reason: Some(reason.to_string()),
            ..RendezvousFrame::new("error")
        },
    );
}

/// One decoded frame from a joiner. The registry's `receive()`, for one record.
fn on_client_frame(record: &Arc<Record>, joiner: &mut Joiner, text: &str) {
    let Ok(frame) = decode_rendezvous_frame(text) else {
        fail(joiner, "unknown", "malformed");
        return;
    };
    if frame.v != RENDEZVOUS_PROTOCOL {
        // A vocabulary this build does not speak. Terminal for the joiner —
        // every subsequent frame would get the same answer — which is why
        // `unsupported-protocol` is in the client's non-retryable set.
        fail(joiner, frame.kind.as_str(), "unsupported-protocol");
        joiner.cut = true;
        return;
    }
    match frame.kind.as_str() {
        "resolve" => {
            if charge_lookup(record, joiner, "resolve") {
                return;
            }
            match record
                .table
                .resolve(code_of(&frame), asked_namespace(&frame), &record.code)
            {
                Ok(()) => reply(
                    joiner,
                    &RendezvousFrame {
                        namespace: Some(record.code.namespace.clone()),
                        admission: Some("open".to_string()),
                        ..RendezvousFrame::new("resolved")
                    },
                ),
                Err(reason) => fail(joiner, "resolve", reason),
            }
        }
        "join" => {
            if charge_lookup(record, joiner, "join") {
                return;
            }
            if let Err(reason) =
                record
                    .table
                    .resolve(code_of(&frame), asked_namespace(&frame), &record.code)
            {
                fail(joiner, "join", reason);
                return;
            }
            joiner.joined = true;
            reply(
                joiner,
                &RendezvousFrame {
                    peer: Some(joiner.id.clone()),
                    admission: Some("open".to_string()),
                    // What this host can answer on, and the reason the field
                    // exists: a native host has no WebRTC of any kind, so a
                    // joiner that did not know would spend the whole 8/16/30 s
                    // ladder four times over discovering it.
                    transports: vec![TRANSPORT_WS_RELAY.to_string()],
                    ..RendezvousFrame::new("joined")
                },
            );
        }
        "relay-open" => {
            if !joiner.joined {
                fail(joiner, "relay-open", "not-joined");
                return;
            }
            if joiner.outbox.is_some() {
                fail(joiner, "relay-open", "already-relaying");
                return;
            }
            let peer = joiner.id.clone();
            let outbox = Arc::new(PeerOutbox::new(&record.table.limits));
            {
                let mut peers = record.peers.lock().unwrap_or_else(|e| e.into_inner());
                if peers.len() >= record.table.limits.max_relay_peers_per_record {
                    drop(peers);
                    // Its own reason rather than the join path's
                    // `admission-closed`: a crew list that is full and a relay
                    // that is full are different problems with different
                    // remedies, and the phone's diagnostics say which.
                    fail(joiner, "relay-open", "relay-full");
                    return;
                }
                peers.insert(peer.clone(), Arc::clone(&outbox));
            }
            // The HOST is told FIRST, deliberately: the joiner's very next act
            // is to put its compatibility handshake on the relay, and a host
            // that had not yet built its side of the pair would drop it.
            record.tell_host(&RendezvousFrame {
                peer: Some(peer.clone()),
                limits: Some(record.limits_frame()),
                ..RendezvousFrame::new("relay-peer")
            });
            joiner.outbox = Some(outbox);
            reply(
                joiner,
                &RendezvousFrame {
                    peer: Some(peer),
                    limits: Some(record.limits_frame()),
                    ..RendezvousFrame::new("relay-ready")
                },
            );
        }
        "relay" => {
            if joiner.outbox.is_none() {
                // The relay is the fallback for a direct link that could not be
                // built, not a way to reach a host without ever resolving its
                // code: everything `join` and `relay-open` decided applies
                // before a single game frame is carried.
                fail(joiner, "relay", "not-relaying");
                return;
            }
            let class = frame.class.clone().unwrap_or_default();
            if class != CLASS_RELIABLE && class != CLASS_SNAPSHOT {
                fail(joiner, "relay", "malformed");
                return;
            }
            let payload = frame.payload.clone().unwrap_or_default();
            if payload.len() > record.table.limits.max_relay_frame_bytes {
                fail(joiner, "relay", "relay-too-large");
                return;
            }
            record.tell_host(&RendezvousFrame {
                from: Some(joiner.id.clone()),
                class: Some(class),
                payload: Some(payload),
                ..RendezvousFrame::new("relay")
            });
        }
        // A client stepping off the relay, or leaving the record. Both detach
        // this connection's relay attachment; only `leave` gives up the record
        // itself, and it ends the connection with it — there is nothing else a
        // joiner does here.
        "relay-close" | "leave" => {
            record.detach(&joiner.id, "closed");
            joiner.outbox = None;
            if frame.kind == "leave" {
                joiner.joined = false;
                joiner.cut = true;
            }
        }
        // A native host has no WebRTC, and says so in `transports` on every
        // `joined`. A signal frame therefore means a joiner ignored that and
        // offered anyway; there is no peer to carry it to.
        "signal" => fail(joiner, "signal", "no-peer"),
        other => fail(joiner, other, "malformed"),
    }
}

/// The code string off a `resolve`/`join` frame. Only the TYPED shape is a
/// code a joiner may send — an issued one is what a service hands a host — so
/// anything else reads as empty and is refused by the resolver.
fn code_of(frame: &RendezvousFrame) -> &str {
    match &frame.code {
        Some(CodeField::Typed(text)) => text.as_str(),
        _ => "",
    }
}

/// Which typed namespace a frame is asking within. Absent, misspelled or
/// anything but `server` means the crew namespace — `askedNamespace`, and the
/// same additive posture: a phone built before issue #1114 sends nothing and
/// means exactly what it always did.
fn asked_namespace(frame: &RendezvousFrame) -> &'static str {
    match frame.namespace.as_deref() {
        Some(NAMESPACE_SERVER) => NAMESPACE_SERVER,
        _ => NAMESPACE_CLIENT,
    }
}

/// Charge one lookup against the authored per-connection cap. Returns true when
/// the connection is done — the suffix space is small and codes are private, so
/// an unbounded socket could walk the namespace.
fn charge_lookup(record: &Arc<Record>, joiner: &mut Joiner, request: &str) -> bool {
    joiner.lookups += 1;
    if joiner.lookups > record.table.limits.max_lookups_per_connection {
        fail(joiner, request, "too-many-attempts");
        joiner.cut = true;
        return true;
    }
    false
}

#[cfg(test)]
#[path = "direct_join_tests.rs"]
mod tests;
