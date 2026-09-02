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
//! # `[ai]` — no Origin allow-list on this leg, and what gates a join instead
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
//! client bundle to anyone on the LAN.
//!
//! What that decision leaves behind is a gate that has to be somewhere else,
//! and the minimal honest one is **attempt-limiting, not an origin list**. A
//! hostile page open in a crew member's browser dials this port with whatever
//! `Origin` its own site has, so a header list would not have stopped it; and
//! the join stamp is PUBLIC (this host serves it), which leaves the code as the
//! only secret between a stranger on the LAN and an acting participant. So the
//! defence is at the transport plane, in three layers that hold whether or not
//! the caller is a browser:
//!
//! 1. a per-source failed-guess budget that SURVIVES reconnection, plus a
//!    per-source cap on sockets that never join ([`AdmissionBudgets`]);
//! 2. a global circuit-breaker that slows every lookup ANSWER while the service
//!    is being guessed at, which is the layer that covers a guesser spread
//!    across many addresses;
//! 3. enough authored letters in the code that the keyspace is out of reach at
//!    any rate those two allow (`assets/join/join-codes.toml`, and the
//!    arithmetic is in [`AdmissionBudgets`]'s note).
//!
//! The compatibility handshake and the reserved-token gate still apply on top,
//! as they always did — but they gate what a joiner may BE, not whether it may
//! guess, and only these three bound the guessing.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::net::{IpAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

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
const PUMP_READ_TIMEOUT: Duration = Duration::from_millis(20);

/// How long one `send` may block this joiner's own thread.
///
/// Without it a joiner that stops READING parks this thread inside
/// `socket.send()` until the OS gives up on the send buffer — minutes on a
/// default Windows or Linux stack — while the peer still holds its slot and its
/// place in the host's audience. Five seconds is far longer than any healthy
/// phone needs to accept a frame off a LAN and far shorter than the OS's own
/// patience; past it the peer is detached down the same path a reliable-queue
/// overflow takes, because both mean the same thing: this link can no longer
/// keep the promise its class makes. Not a gameplay value (AGENTS.md rule 11) —
/// it is one socket thread's own scheduling.
const PUMP_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long an accepted socket may hold a thread without ever joining.
///
/// The upgrade itself is bounded by `delivery::serve`'s head-read timeout, and
/// the number of live UN-JOINED sockets is bounded by
/// [`AdmissionBudgets::unjoined_total`] — deliberately its own budget rather
/// than the authored `max_peers_per_record`, which is the record's crew bound
/// (see that field's note). This is the third leg: without it a silent socket
/// would hold its slot for the length of the mission. A phone that has scanned
/// the QR sends `join` in the same breath as the upgrade, so the window is
/// generous. Not a gameplay value (AGENTS.md rule 11).
const JOIN_DEADLINE: Duration = Duration::from_secs(30);

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

// ── Transport-plane admission budgets ───────────────────────────────────────

/// The bounds that make guessing the join code pointless, held by the SERVICE
/// so that they survive a caller hanging up and dialling again.
///
/// # Why the per-connection cap was not a limit
///
/// The authored `max_lookups_per_connection` (60) is charged against a
/// [`Joiner`], and a `Joiner` is one socket. Against anybody willing to
/// reconnect it is not a rate limit at all, it is a batch size: guess sixty
/// times, close, dial again. Measured against this service before these
/// budgets existed, a single machine on loopback sustained ~2,340 wrong-code
/// guesses per second across churned connections — at which rate the five
/// letters this scheme used to author (25^5 ≈ 9.77e6, 23.2 bits) fall in about
/// **35 minutes** of expected search, single-threaded. The join stamp is
/// public, so a hit is a join, a `relay-open` and an acting participant.
///
/// # The arithmetic these numbers are sized against
///
/// The authored suffix is now **eight** letters over the same 25-letter
/// alphabet: 25^8 ≈ 1.526e11 codes, 37.15 bits. Even at the OLD unlimited
/// 2,340 guesses/s the expected search is 1.526e11 / 2 / 2340 ≈ 3.26e7 s ≈
/// **377 days**. The three layers below then take the achievable rate down by
/// another three orders of magnitude, so the code is out of reach by a margin
/// no session, LAN party or weekend has room for. The layering is deliberate:
/// entropy alone would be a single point of failure, and limits alone would
/// leave a 35-minute secret behind a lock somebody only has to be patient with.
///
/// # AGENTS.md rule 11
///
/// These are **transport-plane** numbers, not gameplay ones: they describe one
/// TCP listener defending itself, they never enter the simulation, a snapshot
/// or a digest, and no designer tunes them from
/// `assets/join/join-codes.toml` (which authors what a CODE is, and is read by
/// three languages). They live in Rust beside [`PUMP_READ_TIMEOUT`] and
/// [`JOIN_DEADLINE`], which are here for the same reason. They are a struct
/// rather than bare constants only so a test can prove the SHAPE — a bucket
/// emptying, a breaker ramping — in milliseconds instead of waiting out a
/// production window, exactly as this module already injects a table path.
#[derive(Clone, Debug)]
pub struct AdmissionBudgets {
    /// Wrong guesses one source may make back to back before it is refused.
    ///
    /// Sized for a NAT'd crew, which is the case that must not break: the whole
    /// room can share one address (a phone hotspot, a venue router, a
    /// port-forward from outside), and a CORRECT code costs nothing — only a
    /// FAILED lookup is charged — so this is a budget for TYPOS, not for joins.
    /// Twenty covers a dozen people fumbling eight letters once or twice each
    /// in the same minute, and it refills underneath them while they do.
    pub guess_burst: u32,
    /// How long one wrong guess takes to refund.
    ///
    /// Five seconds: a sustained 0.2 guesses/s per source. A human retyping a
    /// code cannot notice it; a churner drops from 2,340/s to 17,280/DAY, which
    /// against 25^8 is expected search measured in millions of years.
    pub guess_refill: Duration,
    /// Accepted-but-not-yet-joined sockets one source may hold at once.
    ///
    /// An un-joined socket is a live thread, so this is the DoS bound as well
    /// as an attack bound: without it one device holds every slot and the crew
    /// standing in the room is refused. A real phone is un-joined for the
    /// milliseconds between the `ready` frame and its own `join`, so eight
    /// concurrent from one address is already far past a whole crew scanning
    /// the QR at the same moment — and the refusal is soft, because a slot
    /// frees the instant its socket closes.
    pub unjoined_per_source: usize,
    /// Un-joined sockets across ALL sources, the second half of the same bound.
    ///
    /// Deliberately NOT the authored `max_peers_per_record`: that number is
    /// "how much CREW one record may hold", and letting silence spend it is how
    /// a hostile device refuses the room. Sixteen bounds the thread cost of
    /// callers that never join, on top of (not out of) the record's own peer
    /// cap — so the thread ceiling this module can reach is
    /// `max_peers_per_record + unjoined_total`, and it is stated here rather
    /// than left to be discovered.
    pub unjoined_total: usize,
    /// Distinct source addresses tracked at once.
    ///
    /// A per-source table is itself memory a stranger can spend, so it is
    /// bounded and swept of settled sources. Past the bound a NEW source is
    /// simply not tracked per-source — the global breaker below is the layer
    /// that answers a guesser spread across many addresses, and pretending
    /// otherwise (refusing every unknown source) would be a self-inflicted
    /// outage the moment a venue NAT presents a thousand addresses.
    pub tracked_sources: usize,
    /// The window the global failed-guess count is kept over.
    pub breaker_window: Duration,
    /// Failed guesses in that window before any answer is slowed at all.
    ///
    /// Thirty a minute is more fumbling than a whole crew produces on its worst
    /// arrival; below it nobody waits for anything.
    pub breaker_free: u32,
    /// Further failures per added step of delay.
    pub breaker_ramp: u32,
    /// One step of the ramp.
    pub breaker_step: Duration,
    /// How long one `send` may block a joiner's own thread — see
    /// [`PUMP_WRITE_TIMEOUT`], which is this field's production value.
    ///
    /// It rides here, with the admission budgets, rather than staying a bare
    /// constant because it is the one socket bound a test has to be able to
    /// shorten: proving that a joiner which stops READING is detached rather
    /// than parking a thread otherwise means waiting out the production number
    /// on every run.
    pub write_timeout: Duration,
    /// The most any single lookup answer is held.
    ///
    /// Two seconds, and the ceiling is chosen against the CLIENT: a joiner's
    /// first connect attempt allows 8 s (`gui/rendezvous-transport.js`'s
    /// `connectTimeoutMs`), so a legitimate guest caught in the middle of an
    /// attack waits once and gets in, while a guesser that must eat it on every
    /// answer is capped at one guess per two seconds per socket — and, since
    /// [`JOIN_DEADLINE`] is 30 s, at about fifteen guesses per connection.
    /// Combined with `unjoined_total`, the whole service cannot answer more
    /// than `unjoined_total / breaker_max` ≈ 8 wrong guesses a second however
    /// many addresses the guesser has.
    pub breaker_max: Duration,
}

impl Default for AdmissionBudgets {
    fn default() -> Self {
        Self {
            guess_burst: 20,
            guess_refill: Duration::from_secs(5),
            unjoined_per_source: 8,
            unjoined_total: 16,
            tracked_sources: 1024,
            breaker_window: Duration::from_secs(60),
            breaker_free: 30,
            breaker_ramp: 10,
            breaker_step: Duration::from_millis(100),
            write_timeout: PUMP_WRITE_TIMEOUT,
            breaker_max: Duration::from_secs(2),
        }
    }
}

/// What one source address has spent and is holding.
#[derive(Debug, Default)]
struct SourceBudget {
    /// Failed lookups charged and not yet refunded.
    spent: u32,
    /// When the last whole token was refunded — `None` until the first charge.
    refilled_at: Option<Instant>,
    /// Accepted sockets from this source that have not joined.
    unjoined: usize,
}

impl SourceBudget {
    /// Refund whole tokens for the time since the last refund.
    ///
    /// Integer, and the clock is advanced by exactly what was refunded rather
    /// than to `now`: rounding the remainder away on every charge is how a
    /// token bucket quietly becomes a hard cap for anyone charged faster than
    /// the refill period.
    fn refill(&mut self, now: Instant, step: Duration) {
        let Some(at) = self.refilled_at else {
            self.refilled_at = Some(now);
            return;
        };
        if self.spent == 0 {
            self.refilled_at = Some(now);
            return;
        }
        let step_ms = step.as_millis().max(1);
        let elapsed = now.saturating_duration_since(at).as_millis();
        let gained = (elapsed / step_ms).min(u128::from(self.spent)) as u32;
        if gained == 0 {
            return;
        }
        self.spent -= gained;
        self.refilled_at = Some(at + step * gained);
    }

    /// Nothing outstanding and nothing held: safe to forget.
    fn settled(&self) -> bool {
        self.spent == 0 && self.unjoined == 0
    }
}

/// The global failed-guess count, over one window.
struct Breaker {
    window_start: Instant,
    failures: u32,
}

/// The service's cross-connection budgets. Lives on the [`Record`], because a
/// [`Joiner`] is exactly the thing an attacker throws away.
struct Admissions {
    budgets: AdmissionBudgets,
    sources: Mutex<HashMap<IpAddr, SourceBudget>>,
    unjoined: AtomicUsize,
    breaker: Mutex<Breaker>,
}

/// Which budget refused an upgrade, as the reason the operator's log carries.
type SocketRefusal = &'static str;

impl Admissions {
    fn new(budgets: AdmissionBudgets) -> Self {
        Self {
            budgets,
            sources: Mutex::new(HashMap::new()),
            unjoined: AtomicUsize::new(0),
            breaker: Mutex::new(Breaker {
                window_start: Instant::now(),
                failures: 0,
            }),
        }
    }

    /// Take one un-joined socket slot for `source`, or name the budget that
    /// refused it.
    fn take_socket(&self, source: Option<IpAddr>) -> Result<(), SocketRefusal> {
        loop {
            let live = self.unjoined.load(Ordering::Relaxed);
            if live >= self.budgets.unjoined_total {
                return Err("join-sockets-full");
            }
            if self
                .unjoined
                .compare_exchange_weak(live, live + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
        let Some(ip) = source else {
            return Ok(());
        };
        let now = Instant::now();
        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        self.sweep(&mut sources, now, &ip);
        if !sources.contains_key(&ip) && sources.len() >= self.budgets.tracked_sources {
            // Untracked because the table is full: the global cap above and the
            // breaker below are what answer this caller.
            return Ok(());
        }
        let entry = sources.entry(ip).or_default();
        if entry.unjoined >= self.budgets.unjoined_per_source {
            drop(sources);
            self.unjoined.fetch_sub(1, Ordering::Relaxed);
            return Err("join-sockets-busy");
        }
        entry.unjoined += 1;
        Ok(())
    }

    /// Give one un-joined slot back — the socket closed, or it joined.
    fn release_socket(&self, source: Option<IpAddr>) {
        let live = self.unjoined.load(Ordering::Relaxed);
        self.unjoined.fetch_sub(1.min(live), Ordering::Relaxed);
        let Some(ip) = source else {
            return;
        };
        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = sources.get_mut(&ip) {
            entry.unjoined = entry.unjoined.saturating_sub(1);
            if entry.settled() {
                sources.remove(&ip);
            }
        }
    }

    /// Charge one wrong guess. `true` when this source is out of budget.
    fn charge_failure(&self, source: Option<IpAddr>) -> bool {
        self.note_failure();
        let Some(ip) = source else {
            return false;
        };
        let now = Instant::now();
        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        self.sweep(&mut sources, now, &ip);
        if !sources.contains_key(&ip) && sources.len() >= self.budgets.tracked_sources {
            return false;
        }
        let entry = sources.entry(ip).or_default();
        entry.refill(now, self.budgets.guess_refill);
        entry.spent = entry.spent.saturating_add(1);
        if entry.refilled_at.is_none() {
            entry.refilled_at = Some(now);
        }
        entry.spent > self.budgets.guess_burst
    }

    /// Count one failure against the global window.
    fn note_failure(&self) {
        let now = Instant::now();
        let mut breaker = self.breaker.lock().unwrap_or_else(|e| e.into_inner());
        if now.saturating_duration_since(breaker.window_start) >= self.budgets.breaker_window {
            breaker.window_start = now;
            breaker.failures = 0;
        }
        breaker.failures = breaker.failures.saturating_add(1);
    }

    /// How long the NEXT lookup answer is held, right now.
    ///
    /// Applied to every lookup answer, a correct one included. That is not
    /// generosity to the attacker: answering a right code faster than a wrong
    /// one during an attack would be a timing oracle over the same keyspace the
    /// delay exists to protect.
    fn answer_delay(&self) -> Duration {
        let now = Instant::now();
        let mut breaker = self.breaker.lock().unwrap_or_else(|e| e.into_inner());
        if now.saturating_duration_since(breaker.window_start) >= self.budgets.breaker_window {
            breaker.window_start = now;
            breaker.failures = 0;
        }
        let over = breaker.failures.saturating_sub(self.budgets.breaker_free);
        if over == 0 {
            return Duration::ZERO;
        }
        let steps = over.div_ceil(self.budgets.breaker_ramp.max(1));
        (self.budgets.breaker_step * steps).min(self.budgets.breaker_max)
    }

    /// Keep the source table bounded, without ever forgetting a source that
    /// still owes something (`keep` is the one being charged right now).
    fn sweep(&self, sources: &mut HashMap<IpAddr, SourceBudget>, now: Instant, keep: &IpAddr) {
        if sources.len() < self.budgets.tracked_sources {
            return;
        }
        let step = self.budgets.guess_refill;
        for (ip, entry) in sources.iter_mut() {
            if ip != keep {
                entry.refill(now, step);
            }
        }
        sources.retain(|ip, entry| ip == keep || !entry.settled());
    }
}

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
    /// Joiner sockets that have RESOLVED THE CODE — the count
    /// `max_peers_per_record` bounds.
    ///
    /// Un-joined sockets are counted separately, by [`Admissions`], and the
    /// split is the point: `max_peers_per_record` is the authored answer to
    /// "how much crew may one record hold", and a hostile device holding
    /// thirty-two silent sockets used to spend all of it, refusing the room
    /// with `join-sockets-full`. Silence now has a budget of its own, so the
    /// thread cost is still bounded (`max_peers_per_record + unjoined_total`)
    /// without a stranger being able to spend the crew's half of it.
    joined: AtomicUsize,
    /// The cross-connection budgets, held here so that they outlive any one
    /// connection — see [`AdmissionBudgets`].
    admissions: Admissions,
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
        Self::open_with_budgets(table, AdmissionBudgets::default())
    }

    /// [`Self::open`] with the transport-plane budgets named rather than
    /// defaulted — the seam a test uses to watch a bucket empty in
    /// milliseconds. Production calls [`Self::open`]; nothing ships
    /// test-scaled numbers.
    pub fn open_with_budgets(
        table: JoinCodeTable,
        budgets: AdmissionBudgets,
    ) -> Result<(Self, JoinCode), String> {
        let code = table
            .mint_client_code(crate::native_host::join_codes::os_draw)
            .ok_or_else(|| "the authored join-code table minted no usable code".to_string())?;
        let (tx, rx) = mpsc::channel::<String>();
        let record = Arc::new(Record {
            table,
            code: code.clone(),
            to_host: Mutex::new(tx),
            joined: AtomicUsize::new(0),
            admissions: Admissions::new(budgets),
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
            return Err(refuse(&mut stream, 503, "join-closed", Duration::ZERO));
        }
        // The record's own crew bound, on JOINED peers — silence has its own
        // budget below, so a device holding silent sockets can no longer spend
        // the room's places (see `Record::joined`).
        let crew = self.record.joined.load(Ordering::Relaxed);
        if crew >= self.record.table.limits.max_peers_per_record {
            return Err(refuse(&mut stream, 503, "join-record-full", Duration::ZERO));
        }
        // The source address is read HERE, at accept, because it is the only
        // identity a caller cannot discard by reconnecting — which is exactly
        // what the per-connection lookup cap could not survive. `None` (a
        // socket whose peer is already gone) still spends the global un-joined
        // budget; it simply cannot be told apart from another such socket.
        let source = stream.peer_addr().ok().map(|addr| addr.ip());
        if let Err(reason) = self.record.admissions.take_socket(source) {
            // Soft, and said so: a `Retry-After` of one refill period is the
            // truth about a budget that is filling back up, not a ban.
            return Err(refuse(
                &mut stream,
                503,
                reason,
                self.record.admissions.budgets.guess_refill,
            ));
        }

        let accept_key = tungstenite::handshake::derive_accept_key(key.as_bytes());
        let head = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept_key}\r\n\r\n"
        );
        if stream.write_all(head.as_bytes()).is_err() || stream.flush().is_err() {
            self.record.admissions.release_socket(source);
            return Err(UpgradeRefusal {
                status: 500,
                reason: "join-handshake-write-failed",
            });
        }
        // Both directions are bounded. The read timeout is the pump's cadence;
        // the WRITE timeout is what stops a joiner that stalls its own reads
        // from parking this thread inside `send` for as long as the OS will
        // hold a full send buffer (see [`PUMP_WRITE_TIMEOUT`]).
        if stream.set_read_timeout(Some(PUMP_READ_TIMEOUT)).is_err()
            || stream
                .set_write_timeout(Some(self.record.admissions.budgets.write_timeout))
                .is_err()
        {
            self.record.admissions.release_socket(source);
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
        let slot = SocketSlot {
            record: Arc::clone(&record),
            source,
            joined: false,
        };
        let spawned = std::thread::Builder::new()
            .name("phoenix-join".to_string())
            .spawn(move || serve_joiner(&record, socket, slot));
        if spawned.is_err() {
            // The slot went into the closure that was never spawned, so it was
            // dropped with it and has already given its budget back.
            return Err(UpgradeRefusal {
                status: 503,
                reason: "join-thread-unavailable",
            });
        }
        Ok(())
    }
}

/// One accepted socket's place in the budgets, given back however it ends.
///
/// A guard rather than a pair of `fetch_sub` calls because the counters are the
/// difference between a crew being refused and not: a thread that panicked
/// between them would leak a slot for the rest of the mission, and the leak
/// would look exactly like a busy service.
struct SocketSlot {
    record: Arc<Record>,
    source: Option<IpAddr>,
    /// Which budget this socket is currently spending.
    joined: bool,
}

impl SocketSlot {
    /// This socket resolved the code: it stops spending the un-joined budget
    /// and starts counting as crew.
    fn promote(&mut self) {
        if self.joined {
            return;
        }
        self.joined = true;
        self.record.admissions.release_socket(self.source);
        self.record.joined.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for SocketSlot {
    fn drop(&mut self) {
        if self.joined {
            let live = self.record.joined.load(Ordering::Relaxed);
            self.record.joined.fetch_sub(1.min(live), Ordering::Relaxed);
        } else {
            self.record.admissions.release_socket(self.source);
        }
    }
}

/// Answer a refused upgrade as ordinary HTTP and report it.
///
/// Written before the 101, so the caller reads a status rather than watching a
/// socket close for no stated reason — the same courtesy the worker's 403 and
/// 426 extend. A non-zero `retry_after` says the refusal is a budget filling
/// back up rather than a door that has closed: no refusal this door gives is
/// ever permanent, and a NAT'd crew sharing one address has to be able to read
/// that difference.
fn refuse(
    stream: &mut TcpStream,
    status: u16,
    reason: &'static str,
    retry_after: Duration,
) -> UpgradeRefusal {
    let body = reason;
    let extra: Vec<(&str, String)> = if retry_after.is_zero() {
        Vec::new()
    } else {
        vec![("Retry-After", retry_after.as_secs().max(1).to_string())]
    };
    let head = crate::delivery::http::response_head(
        status,
        "Service Unavailable",
        "text/plain; charset=utf-8",
        crate::delivery::http::CachePolicy::Revalidate,
        body.len(),
        &extra,
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
    /// Where this connection came from, for the budgets a reconnect cannot
    /// shed. `None` when the peer address could not be read at accept, or in
    /// the unit tests, which drive the protocol with no socket under it.
    source: Option<IpAddr>,
    /// How long this connection's next answer is held, set by the global
    /// circuit-breaker. Carried on the joiner rather than slept inside the
    /// protocol so that the frame handler stays a pure decision and only the
    /// pump waits — which is also what lets a unit test assert the ramp
    /// without waiting it out.
    throttle: Duration,
    /// Frames to write before the next read, for a connection that has not
    /// attached yet (and so has no outbox).
    pending: Vec<String>,
    /// Set by a refusal the service should also CLOSE the socket after sending
    /// — the registry's `cut()`, carried out here rather than by an adapter.
    cut: bool,
}

type JoinerSocket = tungstenite::WebSocket<TcpStream>;

/// Pump one joiner until its socket dies or the service closes it.
///
/// `slot` is this socket's place in the transport-plane budgets: it moves from
/// the un-joined budget to the record's crew count the moment the code
/// resolves, and gives whichever it holds back when this function returns.
fn serve_joiner(record: &Arc<Record>, mut socket: JoinerSocket, mut slot: SocketSlot) {
    let mut joiner = Joiner {
        id: record.mint_peer_id(),
        joined: false,
        outbox: None,
        lookups: 0,
        source: slot.source,
        throttle: Duration::ZERO,
        pending: Vec::new(),
        cut: false,
    };
    // The registry greets EVERY socket it accepts with `ready`, host and joiner
    // alike, and a joiner sends nothing until it has seen one
    // (`gui/rendezvous-transport.js`'s `case 'ready'` is where `join` goes out).
    // Without this the phone would sit on "connecting" against a host that was
    // waiting for it to speak first.
    reply(&mut joiner, &RendezvousFrame::new("ready"));
    let opened = Instant::now();
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
            if let Err(e) = socket.send(tungstenite::Message::Text(text.into())) {
                return finish(record, &joiner, write_reason(&e));
            }
        }
        if joiner.cut {
            let _ = socket.close(None);
            let _ = socket.flush();
            return finish(record, &joiner, "refused");
        }
        if let Some(outbox) = &joiner.outbox {
            for text in outbox.drain() {
                if let Err(e) = socket.send(tungstenite::Message::Text(text.into())) {
                    return finish(record, &joiner, write_reason(&e));
                }
            }
        }

        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                on_client_frame(record, &mut joiner, &text);
                if joiner.joined {
                    // Idempotent, and this is the one place the socket stops
                    // being silence and starts being crew.
                    slot.promote();
                }
                // Whatever the circuit-breaker demanded is waited out HERE,
                // before the answer is written: a delay applied after the
                // reply would slow nothing down but this host.
                let owed = std::mem::take(&mut joiner.throttle);
                if !owed.is_zero() {
                    std::thread::sleep(owed);
                }
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

/// Why a write failed, as a reason the host half can render.
///
/// A peer that stopped reading is told apart from one that hung up, because
/// they are different operator problems: the second is somebody walking out of
/// range, the first is a link that has stopped draining and would otherwise
/// have held this thread inside `send` for as long as the OS allowed. Both end
/// the same way — detached — which is the [`Enqueued::Overflowed`] path's
/// answer to the same question.
fn write_reason(e: &tungstenite::Error) -> &'static str {
    match e {
        tungstenite::Error::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            "write-timeout"
        }
        _ => "closed",
    }
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
            let verdict =
                record
                    .table
                    .resolve(code_of(&frame), asked_namespace(&frame), &record.code);
            if !settle_lookup(record, joiner, "resolve", verdict) {
                return;
            }
            reply(
                joiner,
                &RendezvousFrame {
                    namespace: Some(record.code.namespace.clone()),
                    admission: Some("open".to_string()),
                    ..RendezvousFrame::new("resolved")
                },
            );
        }
        "join" => {
            if charge_lookup(record, joiner, "join") {
                return;
            }
            let verdict =
                record
                    .table
                    .resolve(code_of(&frame), asked_namespace(&frame), &record.code);
            if !settle_lookup(record, joiner, "join", verdict) {
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
/// the connection is done.
///
/// This is the FIRST of three bounds and the weakest, because it is charged to
/// a socket and a socket is the thing a guesser throws away: sixty attempts,
/// close, dial again. What actually bounds guessing is [`settle_lookup`] below,
/// against budgets a reconnect cannot shed. This one stays because it is still
/// the cheapest way to end a single abusive connection, and because it is the
/// bound the worker applies to the same verb (`registry.js`'s `chargeLookup`)
/// — a phone must not be able to tell the two services apart.
fn charge_lookup(record: &Arc<Record>, joiner: &mut Joiner, request: &str) -> bool {
    joiner.lookups += 1;
    if joiner.lookups > record.table.limits.max_lookups_per_connection {
        fail(joiner, request, "too-many-attempts");
        joiner.cut = true;
        return true;
    }
    false
}

/// Answer one lookup under the cross-connection budgets. `true` when the code
/// was this host's and the caller may go on.
///
/// Three things happen here and the order matters. A WRONG answer is charged to
/// the source's failed-guess bucket and to the global window; the delay this
/// answer owes is then read from the global breaker — after the charge, so a
/// flood pays for itself immediately; and a source that has run its bucket dry
/// gets `too-many-attempts`, which is a SOFT refusal: the bucket refills on a
/// clock, the client renders it as a sentence in front of the entry field
/// rather than a dead end, and the crew member who fumbled twice more than the
/// rest of the room is joining again a few seconds later. Nothing here is ever
/// a permanent ban, because a whole crew can share one address.
fn settle_lookup(
    record: &Arc<Record>,
    joiner: &mut Joiner,
    request: &str,
    verdict: Result<(), crate::native_host::join_codes::CodeRefusal>,
) -> bool {
    let refusal = verdict.err();
    let starved = refusal.is_some() && record.admissions.charge_failure(joiner.source);
    joiner.throttle = record.admissions.answer_delay();
    if starved {
        fail(joiner, request, "too-many-attempts");
        joiner.cut = true;
        return false;
    }
    match refusal {
        Some(reason) => {
            fail(joiner, request, reason);
            false
        }
        None => true,
    }
}

#[cfg(test)]
#[path = "direct_join_tests.rs"]
mod tests;
