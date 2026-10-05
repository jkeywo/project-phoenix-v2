//! JSON codec for transport envelopes; game payloads remain opaque.
/// Encode one rendezvous frame for the socket.
pub fn encode_rendezvous_frame(
    frame: &crate::rendezvous::RendezvousFrame,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(frame)
}

/// Decode one rendezvous frame off the socket.
pub fn decode_rendezvous_frame(
    s: &str,
) -> Result<crate::rendezvous::RendezvousFrame, serde_json::Error> {
    serde_json::from_str(s)
}

/// Encode one in-band compatibility-handshake frame — the payload INSIDE a
/// relayed game frame, never a `ServerMessage`.
pub fn encode_handshake_frame(
    frame: &crate::rendezvous::HandshakeFrame,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(frame)
}

/// Decode one in-band compatibility-handshake frame.
pub fn decode_handshake_frame(
    s: &str,
) -> Result<crate::rendezvous::HandshakeFrame, serde_json::Error> {
    serde_json::from_str(s)
}
