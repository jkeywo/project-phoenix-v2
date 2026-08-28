//! The real WebSocket behind [`crate::native_host::relay_transport`]
//! (issue #1113).
//!
//! One blocking `tungstenite` client on its own thread, with two channels back
//! to the simulation. `RelayTransport::poll()` runs in Bevy's `PreUpdate` once
//! per frame and may not block for a network round trip, so the thread does the
//! blocking and the queues do the crossing.
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
        use tungstenite::client::IntoClientRequest;

        let url = host_socket_url(base)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| RelayConnectError::BadUrl(format!("{url}: {e}")))?;
        request.headers_mut().insert(
            "Origin",
            origin
                .parse()
                .map_err(|_| RelayConnectError::BadUrl(format!("bad origin {origin}")))?,
        );

        let (socket, _response) = tungstenite::connect(request)
            .map_err(|e| RelayConnectError::Handshake(e.to_string()))?;

        let (inbound_tx, inbound_rx) = mpsc::channel::<String>();
        let (outbound_tx, outbound_rx) = mpsc::channel::<String>();
        let open = Arc::new(AtomicBool::new(true));
        let queued = Arc::new(AtomicUsize::new(0));

        // One thread owns the socket, because `tungstenite`'s is not `Sync` and
        // splitting it would need a lock held across a blocking read. A single
        // pump alternates: drain everything queued for sending, then take one
        // inbound frame with a read timeout, then go round again. The timeout is
        // what keeps a quiet link from starving the outbound direction.
        let pump_open = Arc::clone(&open);
        let pump_queued = Arc::clone(&queued);
        std::thread::Builder::new()
            .name("phoenix-relay".to_string())
            .spawn(move || {
                pump(socket, inbound_tx, outbound_rx, pump_open, pump_queued);
            })
            .map_err(|e| {
                RelayConnectError::Handshake(format!("cannot start the relay thread: {e}"))
            })?;

        Ok(Self {
            inbound: std::sync::Mutex::new(inbound_rx),
            outbound: outbound_tx,
            open,
            queued,
        })
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
    mut socket: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    inbound: Sender<String>,
    outbound: Receiver<String>,
    open: Arc<AtomicBool>,
    queued: Arc<AtomicUsize>,
) {
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
        let _ = stream.set_read_timeout(Some(PUMP_READ_TIMEOUT));
    }
    #[cfg(feature = "host")]
    if let tungstenite::stream::MaybeTlsStream::Rustls(tls) = socket.get_ref() {
        let _ = tls.get_ref().set_read_timeout(Some(PUMP_READ_TIMEOUT));
    }

    loop {
        // Everything waiting to go out, before blocking on a read.
        loop {
            match outbound.try_recv() {
                Ok(text) => {
                    let len = text.len();
                    if socket
                        .send(tungstenite::Message::Text(text.into()))
                        .is_err()
                    {
                        open.store(false, Ordering::Relaxed);
                        return;
                    }
                    queued.fetch_sub(len.min(queued.load(Ordering::Relaxed)), Ordering::Relaxed);
                }
                Err(TryRecvError::Empty) => break,
                // The transport was dropped; nothing more will ever be sent.
                Err(TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    open.store(false, Ordering::Relaxed);
                    return;
                }
            }
        }

        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                if inbound.send(text.to_string()).is_err() {
                    open.store(false, Ordering::Relaxed);
                    return;
                }
            }
            // The service speaks JSON text; binary, ping and pong are
            // tungstenite's own business or nothing of ours.
            Ok(tungstenite::Message::Close(_)) => {
                open.store(false, Ordering::Relaxed);
                return;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                open.store(false, Ordering::Relaxed);
                return;
            }
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
