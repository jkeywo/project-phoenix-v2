/// A duplex text-frame pipe to the rendezvous service.
///
/// The seam between the protocol (everything in this module) and the socket
/// (`tungstenite`, or a test's in-process fake). Deliberately narrow and
/// non-blocking: `Transport::poll` is called from Bevy's `PreUpdate`
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
    /// transport that is not (see `Transport::dispatch`). A socket that
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
