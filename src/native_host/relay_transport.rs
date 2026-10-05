//! Phoenix codec and compatibility policy for the reusable relay.
use crate::core::codec::JsonCodec;
use crate::core::messages::{ClientMessage, ServerMessage};
use crate::delivery::stamp::DeliveryStamp;
use crate::native_host::transport::PhoenixProtocol;
use phoenix_transport::relay::{CompatibilityRefusal, RelayProtocol};
pub use phoenix_transport::relay::{RelayNotice, INVALID_TOKEN_CODE, RESERVED_TOKEN_CODE};
pub use phoenix_transport::socket::RelaySocket;
pub type RelayHostConfig = phoenix_transport::relay::RelayHostConfig<DeliveryStamp>;
pub type RelayTransport = phoenix_transport::relay::RelayTransport<PhoenixProtocol>;

impl RelayProtocol for PhoenixProtocol {
    type Identity = crate::session_connections::PhoenixIdentity;
    type Stamp = DeliveryStamp;
    fn check_stamp(stamp: &DeliveryStamp, peer: Option<&str>) -> Result<(), CompatibilityRefusal> {
        crate::delivery::check_join_stamp(stamp, peer).map_err(|m| CompatibilityRefusal {
            code: m.code().into(),
            detail: m.detail(),
        })
    }
    fn decode_client(payload: &str) -> Result<ClientMessage, String> {
        JsonCodec.decode_client(payload).map_err(|e| e.to_string())
    }
    fn identity(message: &ClientMessage) -> Option<&str> {
        if let ClientMessage::Identify { token, .. } = message {
            Some(token)
        } else {
            None
        }
    }
    fn encode_server(message: &ServerMessage) -> Result<String, String> {
        JsonCodec.encode_server(message).map_err(|e| e.to_string())
    }
}

/// Bevy owns the operator projection; the queue is transport-owned.
#[derive(Clone, Default, bevy::prelude::Resource)]
pub struct RelayNotices(pub phoenix_transport::relay::RelayNotices);
impl RelayNotices {
    pub fn drain(&self) -> Vec<RelayNotice> {
        self.0.drain()
    }
}

#[cfg(test)]
#[path = "relay_transport_tests.rs"]
mod tests;
