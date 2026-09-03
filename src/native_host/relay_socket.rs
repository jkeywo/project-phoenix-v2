//! The real WebSocket behind [`crate::native_host::relay_transport`]
//! (issue #1113).
//!
//! One blocking `tungstenite` client on its own thread, with two channels back
//! to the simulation. `RelayTransport::poll()` runs in Bevy's `PreUpdate` once
//! per frame and may not block for a network round trip, so the thread does the
//! blocking and the queues do the crossing.
//!
//! # It redials, and it has to
//!
//! A rendezvous socket dies for entirely routine reasons: a Durable Object
//! eviction, a worker redeploy, an idle timeout, a laptop's Wi-Fi blip. The
//! browser host answers that with `lostService()` — discard the socket,
//! re-register on a backoff, repaint the new code — and states why: a
//! host that simply stopped would be unjoinable until somebody reloaded the
//! viewscreen. A NATIVE host has no viewscreen to reload; it is a process
//! running an authoritative mission, and losing this socket would end every
//! route in for the rest of it.
//!
//! So the thread is a supervisor rather than a single pump: dial, pump until
//! the socket dies, discard whatever was queued for the dead socket, wait a
//! backoff, dial again. `is_open()` goes false in between, which is what
//! `RelayTransport::poll` reads to report the crew gone and clear both the code
//! and its `registered` flag; when the redial lands, the service's `ready`
//! frame re-registers the host and issues a FRESH code, which the operator log
//! prints. The old code really is dead — the record went with it — and keeping
//! the same letters across a host drop needs persistence in the service
//! (issue #1115), exactly as on the browser side.
//!
//! # `[ai]` — why `tungstenite` and not one of its async wrappers
//!
//! Stated fully in `Cargo.toml` beside the dependency, and in short: this crate
//! has no async runtime, every native I/O surface it owns is a blocking std
//! thread (`delivery::serve` is a `TcpListener` with a thread per connection),
//! and `tokio-tungstenite`/`async-tungstenite` are wrappers around this crate
//! that would each bring a runtime along for one socket. The seam is a
//! non-blocking `poll`/`send` pair either way.
//!
//! # What this file is NOT
//!
//! It is not the protocol. Registration, the compatibility handshake, the
//! `Identify` gate, audience resolution and the delivery classes are all in
//! `relay_transport.rs`, driven by a fake socket in its own tests. This file
//! only moves text frames, which is why it has no unit tests of its own worth
//! writing: everything it does is `tungstenite`'s, and the one thing it adds —
//! that a real service accepts these frames — needs a real service. That proof
//! is `tests/native_relay_live.rs`, `#[ignore]`d, with its command written down.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{mpsc, Arc};

use crate::native_host::relay_transport::RelaySocket;

/// A `tungstenite` client socket, pumped by a reader thread and a writer thread.
pub struct WsRelaySocket {
    /// Behind a `Mutex` purely to be `Sync`: `NativeTransportLink` is a Bevy
    /// resource and every resource must be, while `mpsc::Receiver` is `Send`
    /// and not `Sync`. Nothing ever contends for it — the only reader is
    /// `poll(&mut self)`, which takes it with `get_mut` and never locks.
    inbound: std::sync::Mutex<Receiver<String>>,
    outbound: Sender<String>,
    open: Arc<AtomicBool>,
    /// Bytes handed to the writer thread and not yet written. The backpressure
    /// signal `RelayTransport` sheds snapshot frames against.
    ///
    /// It measures the QUEUE, not the kernel's socket buffer — `tungstenite`
    /// exposes no equivalent of a browser's `bufferedAmount`, and a number this
    /// file invented would be worse than an honest approximation. It rises
    /// exactly when the writer cannot keep up, which is the condition the
    /// shedding rule is about.
    queued: Arc<AtomicUsize>,
    /// Set by [`RelaySocket::close`]: stop pumping and stop REDIALING. Without
    /// it a closed transport would leave a thread reconnecting to the service
    /// for the life of the process.
    shutdown: Arc<AtomicBool>,
}

/// Why a native host could not reach the rendezvous service.
#[derive(Debug)]
pub enum RelayConnectError {
    /// The URL was not something a WebSocket client could dial.
    BadUrl(String),
    /// The connection or the upgrade failed. Carries `tungstenite`'s own words,
    /// which name the HTTP status for an origin refusal — the one failure an
    /// operator can act on directly (docs/delivery-checklist.md §3a).
    Handshake(String),
}

impl std::fmt::Display for RelayConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayConnectError::BadUrl(u) => write!(f, "not a usable rendezvous URL: {u}"),
            RelayConnectError::Handshake(e) => write!(f, "rendezvous handshake failed: {e}"),
        }
    }
}

impl std::error::Error for RelayConnectError {}

/// The host endpoint on a rendezvous service base URL.
///
/// `https://…` becomes `wss://…`, `http://…` becomes `ws://…` (a local
/// `wrangler dev`), and anything already a socket scheme is left alone — the
/// same upgrade `socketUrl()` performs in gui/rendezvous-transport.js, kept
/// here rather than asked of the operator.
pub fn host_socket_url(base: &str) -> Result<String, RelayConnectError> {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(RelayConnectError::BadUrl(base.to_string()));
    }
    let (scheme, rest) = match trimmed.split_once("://") {
        Some(("https", rest)) => ("wss", rest),
        Some(("http", rest)) => ("ws", rest),
        Some(("wss", rest)) => ("wss", rest),
        Some(("ws", rest)) => ("ws", rest),
        _ => return Err(RelayConnectError::BadUrl(base.to_string())),
    };
    if rest.is_empty() {
        return Err(RelayConnectError::BadUrl(base.to_string()));
    }
    Ok(format!("{scheme}://{rest}/v1/host"))
}

impl WsRelaySocket {
    /// Dial the rendezvous service's host endpoint and start pumping.
    ///
    /// `origin` is sent as the `Origin` header, because the service's upgrade
    /// gate demands a present, allow-listed one — a browser always sends one,
    /// so exempting requests without it would exempt exactly the scripted
    /// caller that 403 exists to stop (worker-rendezvous/src/index.js). A
    /// native host therefore has to claim an origin the deployment allows;
    /// which one is an operator decision and a checklist item, not a default
    /// this file can invent.
    pub fn connect(base: &str, origin: &str) -> Result<Self, RelayConnectError> {
        let url = host_socket_url(base)?;
        // The FIRST dial is synchronous, so a typo'd URL, an unreachable
        // service or an origin the deployment does not allow is a startup
        // error the operator reads immediately — rather than a process that
        // boots into a silent redial loop against a service that will never
        // accept it.
        let socket = dial(&url, origin)?;

        let (inbound_tx, inbound_rx) = mpsc::channel::<String>();
        let (outbound_tx, outbound_rx) = mpsc::channel::<String>();
        let open = Arc::new(AtomicBool::new(true));
        let queued = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));

        // One thread owns the socket, because `tungstenite`'s is not `Sync` and
        // splitting it would need a lock held across a blocking read. It
        // supervises rather than pumps once: see the module header.
        let supervisor = Supervisor {
            url,
            origin: origin.to_string(),
            inbound: inbound_tx,
            outbound: outbound_rx,
            open: Arc::clone(&open),
            queued: Arc::clone(&queued),
            shutdown: Arc::clone(&shutdown),
        };
        std::thread::Builder::new()
            .name("phoenix-relay".to_string())
            .spawn(move || supervisor.run(socket))
            .map_err(|e| {
                RelayConnectError::Handshake(format!("cannot start the relay thread: {e}"))
            })?;

        Ok(Self {
            inbound: std::sync::Mutex::new(inbound_rx),
            outbound: outbound_tx,
            open,
            queued,
            shutdown,
        })
    }
}

/// Dial the host endpoint once, claiming `origin`.
fn dial(url: &str, origin: &str) -> Result<WsStream, RelayConnectError> {
    use tungstenite::client::IntoClientRequest;

    let mut request = url
        .into_client_request()
        .map_err(|e| RelayConnectError::BadUrl(format!("{url}: {e}")))?;
    request.headers_mut().insert(
        "Origin",
        origin
            .parse()
            .map_err(|_| RelayConnectError::BadUrl(format!("bad origin {origin}")))?,
    );
    let (socket, _response) =
        tungstenite::connect(request).map_err(|e| RelayConnectError::Handshake(e.to_string()))?;
    Ok(socket)
}

/// The redial ladder, in milliseconds, capped so a service that is down for an
/// hour is still polled about once a minute rather than once a day.
///
/// The same shape as `nextBackoffDelay` in gui/connection-manager.js, which the
/// browser host's `lostService()` uses for the identical event — doubling from
/// a second, ceiling at a minute. Not a gameplay value: it is how often one
/// socket asks an unreachable service to try again (AGENTS.md rule 11).
pub(crate) fn redial_delay_ms(attempt: u32) -> u64 {
    const BASE_MS: u64 = 1_000;
    const CEILING_MS: u64 = 60_000;
    BASE_MS
        .saturating_mul(1u64 << attempt.min(6))
        .min(CEILING_MS)
}

type WsStream = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

/// Why one pump stopped: the socket died (redial), or this end is finished.
#[derive(PartialEq)]
enum PumpEnd {
    SocketDied,
    Finished,
}

/// Owns the socket for the process's lifetime, across as many dials as it takes.
struct Supervisor {
    url: String,
    origin: String,
    inbound: Sender<String>,
    outbound: Receiver<String>,
    open: Arc<AtomicBool>,
    queued: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
}

impl Supervisor {
    fn run(self, first: WsStream) {
        let mut socket = Some(first);
        let mut attempt = 0u32;
        loop {
            if let Some(s) = socket.take() {
                self.open.store(true, Ordering::Relaxed);
                attempt = 0;
                let end = pump(
                    s,
                    &self.inbound,
                    &self.outbound,
                    &self.queued,
                    &self.shutdown,
                );
                self.open.store(false, Ordering::Relaxed);
                if end == PumpEnd::Finished {
                    return;
                }
            }
            if self.shutdown.load(Ordering::Relaxed) {
                return;
            }
            // Anything queued for the socket that just died is thrown away
            // rather than replayed onto the next one. Those frames name peers
            // and a record that no longer exist, and `RelayTransport` has
            // already reported that whole crew gone.
            while self.outbound.try_recv().is_ok() {}
            self.queued.store(0, Ordering::Relaxed);

            // Sleep in short slices so `close()` is not held up for a minute.
            let delay = redial_delay_ms(attempt);
            attempt = attempt.saturating_add(1);
            let mut slept = 0;
            while slept < delay {
                if self.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(PUMP_READ_TIMEOUT);
                slept += PUMP_READ_TIMEOUT.as_millis() as u64;
            }
            socket = dial(&self.url, &self.origin).ok();
        }
    }
}

/// How long the reader waits for a frame before checking the outbound queue.
///
/// Short enough that an outbound burst is never held behind a quiet link for
/// longer than a couple of simulation frames, long enough that an idle host is
/// not a spin loop. Not a gameplay value: it is the granularity of one socket
/// thread's own scheduling, invisible to the simulation and to any authored
/// content (AGENTS.md rule 11).
const PUMP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(20);

fn pump(
    mut socket: WsStream,
    inbound: &Sender<String>,
    outbound: &Receiver<String>,
    queued: &Arc<AtomicUsize>,
    shutdown: &Arc<AtomicBool>,
) -> PumpEnd {
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
        let _ = stream.set_read_timeout(Some(PUMP_READ_TIMEOUT));
    }
    #[cfg(feature = "host")]
    if let tungstenite::stream::MaybeTlsStream::Rustls(tls) = socket.get_ref() {
        let _ = tls.get_ref().set_read_timeout(Some(PUMP_READ_TIMEOUT));
    }

    loop {
        if shutdown.load(Ordering::Relaxed) {
            let _ = socket.close(None);
            return PumpEnd::Finished;
        }
        // Everything waiting to go out, before blocking on a read.
        loop {
            match outbound.try_recv() {
                Ok(text) => {
                    let len = text.len();
                    if socket
                        .send(tungstenite::Message::Text(text.into()))
                        .is_err()
                    {
                        return PumpEnd::SocketDied;
                    }
                    queued.fetch_sub(len.min(queued.load(Ordering::Relaxed)), Ordering::Relaxed);
                }
                Err(TryRecvError::Empty) => break,
                // The transport was dropped; nothing more will ever be sent,
                // so there is nothing left to redial FOR.
                Err(TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    return PumpEnd::Finished;
                }
            }
        }

        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                if inbound.send(text.to_string()).is_err() {
                    return PumpEnd::Finished;
                }
            }
            // The service speaks JSON text; binary, ping and pong are
            // tungstenite's own business or nothing of ours.
            Ok(tungstenite::Message::Close(_)) => return PumpEnd::SocketDied,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return PumpEnd::SocketDied,
        }
    }
}

impl RelaySocket for WsRelaySocket {
    fn poll(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let inbound = self.inbound.get_mut().expect("relay inbox poisoned");
        loop {
            match inbound.try_recv() {
                Ok(text) => out.push(text),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.open.store(false, Ordering::Relaxed);
                    break;
                }
            }
        }
        out
    }

    fn send(&mut self, text: String) {
        self.queued.fetch_add(text.len(), Ordering::Relaxed);
        if self.outbound.send(text).is_err() {
            self.open.store(false, Ordering::Relaxed);
        }
    }

    fn buffered_bytes(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    fn close(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.open.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_upgrades_a_service_base_to_the_host_endpoint() {
        // The same upgrade gui/rendezvous-transport.js's socketUrl() performs.
        // Asked of the operator instead, it would be one more thing to get
        // wrong on a checklist that already has the origin allowlist on it.
        assert_eq!(
            host_socket_url("https://phoenix-rendezvous.project-phoenix.workers.dev").unwrap(),
            "wss://phoenix-rendezvous.project-phoenix.workers.dev/v1/host"
        );
        // A local `wrangler dev` is plain HTTP and must stay plain.
        assert_eq!(
            host_socket_url("http://localhost:8787/").unwrap(),
            "ws://localhost:8787/v1/host"
        );
        // Already a socket scheme: left alone rather than upgraded twice.
        assert_eq!(
            host_socket_url("wss://example.test").unwrap(),
            "wss://example.test/v1/host"
        );
    }

    #[test]
    fn the_redial_ladder_doubles_and_then_holds_at_a_minute() {
        // The same shape gui/connection-manager.js's nextBackoffDelay gives the
        // browser host for the identical event. A service that is down for an
        // hour is still asked once a minute; a blip is retried in a second.
        assert_eq!(redial_delay_ms(0), 1_000);
        assert_eq!(redial_delay_ms(1), 2_000);
        assert_eq!(redial_delay_ms(4), 16_000);
        assert_eq!(redial_delay_ms(6), 60_000);
        // And it never runs away: an attempt count that keeps climbing for the
        // rest of the mission must not overflow or stop retrying.
        assert_eq!(redial_delay_ms(40), 60_000);
        assert_eq!(redial_delay_ms(u32::MAX), 60_000);
    }

    #[test]
    fn it_refuses_something_that_is_not_a_service_url() {
        // A typo'd `--rendezvous` must fail at parse with a stated reason, not
        // as a connection attempt to a hostname made of the whole argument.
        for bad in [
            "",
            "   ",
            "phoenix-rendezvous.workers.dev",
            "ftp://x",
            "https://",
        ] {
            assert!(
                host_socket_url(bad).is_err(),
                "{bad:?} is not a rendezvous base"
            );
        }
    }
}
