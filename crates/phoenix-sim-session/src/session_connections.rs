//! Phoenix identity policy over the reusable physical connection registry.

pub use phoenix_transport::connections::{BindRefusal, ConnectionId, MAX_TOKEN_CHARS};
pub struct PhoenixIdentity;
impl phoenix_transport::connections::IdentityPolicy for PhoenixIdentity {
    fn is_reserved(token: &str) -> bool {
        crate::lobby::handler::is_reserved_token(token)
    }
}
pub type ConnectionRegistry = phoenix_transport::connections::ConnectionRegistry<PhoenixIdentity>;
pub fn validate_token(token: &str) -> Result<(), BindRefusal> {
    phoenix_transport::connections::validate_token::<PhoenixIdentity>(token)
}
