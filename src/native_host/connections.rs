//! Phoenix identity policy for every native transport leg.
pub type SharedConnections = phoenix_transport::shared_connections::SharedConnections<
    crate::session_connections::PhoenixIdentity,
>;
pub(crate) type ConnectionLeg = phoenix_transport::shared_connections::ConnectionLeg<
    crate::session_connections::PhoenixIdentity,
>;
