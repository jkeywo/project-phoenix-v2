//! Native adaptation of the simulation's frame-driven transport seam.
//!
//! The simulation exchanges `InboundMessage`, `PlayerDisconnected` and
//! `OutboundMessage`. The browser adapts those messages in `server::bridge`;
//! native adapters implement [NativeTransport] and use the same lobby,
//! command Admission and audience projection.
//!
//! LAN and cloud both use [RelayTransport](super::relay_transport::RelayTransport)
//! over different physical sockets. Embedded Station panes use
//! [PaneTransport](super::panes::PaneTransport), retaining their own minted
//! identity gate and queues. [PairedTransport] shares the pure
//! [ConnectionRegistry](crate::session_connections::ConnectionRegistry) across
//! every composed leg; only the current physical owner can supply a Session's
//! input, owe its departure or receive its targeted messages.
//!
//! # Connection ownership and Session authority
//!
//! Build/role admission and physical send/close stay in each adapter. The
//! registry binds an accepted Identify once and handles replacement before
//! teardown. It never changes authoritative Session state, Station Admission
//! or audience projection. This Bevy seam retains reserved-token defence for
//! queue-only adapters too.
//!
//! # Ordering
//!
//! Ingress runs in PreUpdate and egress in PostUpdate: the seam is frame-driven,
//! while the simulation is tick-driven. Bevy defers message cleanup until the
//! fixed schedules have observed a frame's messages, so a frame that runs zero
//! fixed steps loses nothing.

use bevy::prelude::*;

use super::connections::SharedConnections;
use crate::core::messages::{ClientMessage, DeliveryClass, ServerMessage};
use crate::lobby::handler::Target;
use crate::lobby::{InboundMessage, OutboundMessage, PlayerDisconnected};
use crate::logging::{LogCat, LogFilterConfig};

/// Phoenix's vocabulary binding; the transport implementation is reusable.
pub struct PhoenixProtocol;
impl phoenix_transport::Profile for PhoenixProtocol {
    type Inbound = ClientMessage;
    type Outbound = ServerMessage;
    type Connections = SharedConnections;
}
pub type TransportEvent = phoenix_transport::Event<ClientMessage>;
pub type TransportDispatch<'a> = phoenix_transport::Dispatch<'a, ServerMessage>;
pub trait NativeTransport: phoenix_transport::Transport<PhoenixProtocol> {}
impl<T: phoenix_transport::Transport<PhoenixProtocol> + ?Sized> NativeTransport for T {}

/// The transport a native host is currently using, if any.
///
/// `Option`al by construction: a host with no transport resource inserted runs
/// the mission with nobody able to connect (every station on `Backfill`), which
/// is exactly what a solo viewscreen run is, and exactly what the digest
/// equivalence tests want.
#[derive(Resource)]
pub struct NativeTransportLink(pub Box<dyn NativeTransport>);

impl NativeTransportLink {
    /// Wrap `transport` for insertion as a resource.
    pub fn new(transport: impl NativeTransport) -> Self {
        Self(Box::new(transport))
    }
}

/// Registers the two seam systems. Adding it without a
/// [`NativeTransportLink`] resource is a no-op, so a host can install the plugin
/// unconditionally and the transport later.
pub struct NativeTransportPlugin;

impl Plugin for NativeTransportPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, drain_native_inbound)
            .add_systems(PostUpdate, flush_native_outbound);
    }
}

/// Poll the transport and write the simulation's inbound messages.
///
/// The reserved-token gate lives here, and it is the one authorisation decision
/// the seam makes on its own. `lobby::handler::is_reserved_token` refuses
/// `__local_console__` and any `ai:`-prefixed token: the first is the host
/// operator's own bypass (it skips the station-tenure branch of
/// `is_command_authorized` entirely and carries mission-abort authority), the
/// second is how in-process AI emissions are labelled. Neither may be claimed
/// by something arriving through a transport — a participant, in-process or
/// otherwise, is an ordinary session token going through `Identify` like a
/// phone.
///
/// A refusal is dropped and logged, not answered: the browser's ingress does the
/// same, and there is no session to answer to yet.
fn drain_native_inbound(
    transport: Option<ResMut<NativeTransportLink>>,
    mut inbound: MessageWriter<InboundMessage>,
    mut disconnects: MessageWriter<PlayerDisconnected>,
    log: Option<Res<LogFilterConfig>>,
    role: Option<Res<crate::native_host::session_role::NativeSessionRoleState>>,
) {
    let Some(mut transport) = transport else {
        return;
    };
    let gm_only = role.as_ref().is_some_and(|state| {
        matches!(
            state.role(),
            crate::native_host::session_role::NativeSessionRole::StandaloneGameMaster
                | crate::native_host::session_role::NativeSessionRole::FleetGameMaster
        )
    });
    for event in transport.0.poll() {
        if gm_only {
            crate::pdebug!(
                log,
                LogCat::Admit,
                "native transport: crew ingress is disabled for the selected GM role"
            );
            continue;
        }
        match event {
            TransportEvent::Received { token, msg } => {
                if crate::lobby::handler::is_reserved_token(&token) {
                    crate::pwarn!(
                        log,
                        LogCat::Admit,
                        "native transport: refusing reserved token {token:?} — a \
                         participant joins with an ordinary session token"
                    );
                    continue;
                }
                inbound.write(InboundMessage { token, msg });
            }
            TransportEvent::Disconnected { token } => {
                if crate::lobby::handler::is_reserved_token(&token) {
                    continue;
                }
                crate::pdebug!(log, LogCat::Lobby, "native transport: {token} disconnected");
                disconnects.write(PlayerDisconnected { token });
            }
        }
    }
}

/// Hand every outbound message to the transport, in production order.
fn flush_native_outbound(
    transport: Option<ResMut<NativeTransportLink>>,
    mut reader: MessageReader<OutboundMessage>,
) {
    let Some(mut transport) = transport else {
        // Nothing is listening. The messages still drain — Bevy clears them
        // after the fixed schedules have seen a frame — so an unconnected host
        // does not accumulate an unbounded outbox.
        reader.read().for_each(|_| {});
        return;
    };
    for out in reader.read() {
        transport.0.dispatch(TransportDispatch {
            target: &out.target,
            msg: &out.msg,
            delivery: out.delivery,
        });
    }
}

// ── Two transports on one host ──────────────────────────────────────────────

/// Two [`NativeTransport`]s driven as one (issue #1122).
///
/// LAN, cloud and pane adapters share one connection registry here. Poll order
/// remains first then second, preserving order within each leg. Dispatch visits
/// both, but only the current owner's physical connection can receive a Target.
/// Compose before polling crew traffic. It nests, so a third leg is
/// `PairedTransport::new(PairedTransport::new(a, b), c)`.
pub type PairedTransport<A, B> = phoenix_transport::PairedTransport<PhoenixProtocol, A, B>;

// ── Loopback ────────────────────────────────────────────────────────────────

/// An in-process [`NativeTransport`] backed by two queues.
///
/// It is the transport the seam's own tests drive, and the shape issue #1122's
/// Ultralight pane wants: no sockets, no codec, decoded messages in and decoded
/// messages out — while still entering through `Messages<InboundMessage>` and
/// still leaving through an `Audience`-resolved `Target`, which is what "cannot
/// bypass admission or projection" means concretely.
///
/// Clone the [`LoopbackHandle`] before inserting the transport; that is the end
/// a caller pushes into and reads out of.
pub struct LoopbackTransport {
    handle: LoopbackHandle,
}

/// The caller's end of a [`LoopbackTransport`]. Cheap to clone; every clone
/// refers to the same queues.
#[derive(Clone, Default)]
pub struct LoopbackHandle {
    inbox: std::sync::Arc<std::sync::Mutex<Vec<TransportEvent>>>,
    outbox: std::sync::Arc<std::sync::Mutex<Vec<(Target, ServerMessage, DeliveryClass)>>>,
}

impl LoopbackHandle {
    /// Queue a decoded client message from `token` for the next poll.
    pub fn send(&self, token: &str, msg: ClientMessage) {
        self.inbox
            .lock()
            .expect("loopback inbox poisoned")
            .push(TransportEvent::Received {
                token: token.to_string(),
                msg,
            });
    }

    /// Queue a disconnect for `token`.
    pub fn disconnect(&self, token: &str) {
        self.inbox
            .lock()
            .expect("loopback inbox poisoned")
            .push(TransportEvent::Disconnected {
                token: token.to_string(),
            });
    }

    /// Take everything dispatched so far, oldest first.
    pub fn drain_outbound(&self) -> Vec<(Target, ServerMessage, DeliveryClass)> {
        self.outbox
            .lock()
            .expect("loopback outbox poisoned")
            .drain(..)
            .collect()
    }

    /// A transport that shares these queues.
    pub fn transport(&self) -> LoopbackTransport {
        LoopbackTransport {
            handle: self.clone(),
        }
    }
}

impl phoenix_transport::Transport<crate::native_host::transport::PhoenixProtocol>
    for LoopbackTransport
{
    fn poll(&mut self) -> Vec<TransportEvent> {
        self.handle
            .inbox
            .lock()
            .expect("loopback inbox poisoned")
            .drain(..)
            .collect()
    }

    fn dispatch(&mut self, dispatch: TransportDispatch<'_>) {
        self.handle
            .outbox
            .lock()
            .expect("loopback outbox poisoned")
            .push((
                dispatch.target.clone(),
                dispatch.msg.clone(),
                dispatch.delivery,
            ));
    }

    fn name(&self) -> &'static str {
        "loopback"
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
