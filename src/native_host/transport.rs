//! The native transport seam (issue #1121).
//!
//! The simulation's entire network surface is three Bevy messages declared in
//! [`crate::lobby::server`]: [`InboundMessage`], [`PlayerDisconnected`] and
//! [`OutboundMessage`]. The browser host adapts them to PeerJS with exactly two
//! systems — `drain_inbound` in `PreUpdate` and `flush_outbound` in `PostUpdate`
//! (`server::bridge`) — and everything else about the transport is JavaScript.
//!
//! This module is the native statement of the same two systems, against a
//! [`NativeTransport`] trait a real transport plugs into. It exists **now**,
//! ahead of that transport, for two reasons:
//!
//! * The Phoenix WebRTC transport that replaces PeerJS is issue #1112, in
//!   flight on another track. PeerJS is browser JavaScript and cannot run in a
//!   native process at all, so issue #1121's "browser clients join the native
//!   host" acceptance criterion is **deferred** to it. What #1121 owes it is a
//!   seam that is already wired, already ordered correctly against the fixed
//!   tick, and already applying the reserved-token gate — so connecting it is
//!   an `insert_resource`, not a re-plumb.
//! * Issue #1122's in-process Ultralight pane is the same shape: PRD #1093 says
//!   an in-process participant "may avoid network serialisation but cannot
//!   bypass command admission or projection boundaries". A pane is therefore a
//!   [`NativeTransport`] that skips the JSON codec and hands over a decoded
//!   [`ClientMessage`] — which is what this trait carries, rather than bytes.
//!
//! # What the seam does NOT do
//!
//! It does not mint tokens, map connections, or decide who is who. Admission is
//! still `command_admission`'s and `lobby::handler`'s; audience projection is
//! still `core::broadcast::audience`'s, resolving every variant through
//! `SessionManager::holder_for_station`. A local participant that is a
//! registered session holding a station is projected to identically to a phone,
//! with no code here. The one thing the seam owes on its own account is the
//! **reserved-token refusal**, because the browser applies it at its own
//! ingress (`server.html`'s `isPeerTokenAllowed`) as well as inside
//! `handle_identify`: an in-process path that skipped it would be a hole the
//! network path is not. See [`drain_native_inbound`].
//!
//! # Ordering
//!
//! Ingress runs in `PreUpdate` and egress in `PostUpdate`, i.e. the seam is
//! **frame**-driven while the simulation is **tick**-driven — deliberately, and
//! for the reason `server::bridge` documents: Bevy defers message cleanup until
//! the fixed schedules have observed a frame's messages, so a frame that runs
//! zero fixed steps loses nothing.

use bevy::prelude::*;

use crate::core::messages::{ClientMessage, DeliveryClass, ServerMessage};
use crate::lobby::handler::Target;
use crate::lobby::{InboundMessage, OutboundMessage, PlayerDisconnected};
use crate::logging::{LogCat, LogFilterConfig};

/// Something a transport observed and is handing the simulation.
#[derive(Clone, Debug, PartialEq)]
pub enum TransportEvent {
    /// A decoded client message from `token`. Decoded, not raw JSON: a network
    /// transport runs `core::codec` itself (that is where `serde_json` lives),
    /// and an in-process participant has nothing to decode.
    Received { token: String, msg: ClientMessage },
    /// `token`'s connection went away. The lobby owns what that means for the
    /// station they held — the seam only reports it.
    Disconnected { token: String },
}

/// One dispatch the simulation is handing a transport.
///
/// Borrowed rather than owned so a transport that only needs to look at the
/// message (to route it, or to hand it straight to an in-process pane) pays no
/// clone, and the `Target`/`DeliveryClass` pair arrives exactly as the
/// broadcaster resolved it.
pub struct TransportDispatch<'a> {
    pub target: &'a Target,
    pub msg: &'a ServerMessage,
    pub delivery: DeliveryClass,
}

/// What a native transport must provide to carry the Phoenix protocol.
///
/// Two methods, matching the two systems in this module. Implementations are
/// polled once per frame from `PreUpdate` and dispatched to once per frame from
/// `PostUpdate`; neither is called from a fixed step, and neither may block.
pub trait NativeTransport: Send + Sync + 'static {
    /// Everything that arrived since the last poll, in arrival order.
    fn poll(&mut self) -> Vec<TransportEvent>;

    /// Deliver one outbound message. Called once per `OutboundMessage`, in the
    /// order the simulation produced them.
    fn dispatch(&mut self, dispatch: TransportDispatch<'_>);

    /// A short name for the operator log. Defaults to `"native"`.
    fn name(&self) -> &'static str {
        "native"
    }
}

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
) {
    let Some(mut transport) = transport else {
        return;
    };
    for event in transport.0.poll() {
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
/// A native host with local Station panes *and* remote participants has two
/// transports and one seam. This is that composition, and it is deliberately
/// dumb: poll `first` then `second`, dispatch to both, and let each decide for
/// itself whether the `Target` names anyone it knows. Neither sees the other's
/// traffic, because neither is asked about it — the pane bus routes by token
/// through `panes::routing`, and a network transport routes by its own
/// connection table, exactly as `server.html`'s does.
///
/// Issue #1112 is the intended production user: it brings the network half, and
/// what it needs from #1122 is that installing it alongside the panes is an
/// `insert_resource`, not a re-plumb. It nests, so a third transport is
/// `PairedTransport::new(PairedTransport::new(a, b), c)`.
pub struct PairedTransport<A: NativeTransport, B: NativeTransport> {
    first: A,
    second: B,
}

impl<A: NativeTransport, B: NativeTransport> PairedTransport<A, B> {
    /// Drive `first` and `second` as one transport. Polled in that order, so a
    /// frame's events are ordered by transport and then by arrival within it.
    pub fn new(first: A, second: B) -> Self {
        Self { first, second }
    }
}

impl<A: NativeTransport, B: NativeTransport> NativeTransport for PairedTransport<A, B> {
    fn poll(&mut self) -> Vec<TransportEvent> {
        let mut events = self.first.poll();
        events.extend(self.second.poll());
        events
    }

    fn dispatch(&mut self, dispatch: TransportDispatch<'_>) {
        self.first.dispatch(TransportDispatch {
            target: dispatch.target,
            msg: dispatch.msg,
            delivery: dispatch.delivery,
        });
        self.second.dispatch(dispatch);
    }

    fn name(&self) -> &'static str {
        "paired"
    }
}

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

impl NativeTransport for LoopbackTransport {
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
mod tests {
    use super::*;
    use crate::core::messages::ServerMessage;

    /// A bare app carrying just the three transport messages and the seam.
    /// Deliberately not a whole simulation: what is under test here is the
    /// seam's own contract, and a bare `App` touches no process-global state.
    fn seam_app() -> (App, LoopbackHandle) {
        let mut app = App::new();
        app.add_message::<InboundMessage>()
            .add_message::<OutboundMessage>()
            .add_message::<PlayerDisconnected>()
            .add_plugins(NativeTransportPlugin);
        let handle = LoopbackHandle::default();
        app.insert_resource(NativeTransportLink::new(handle.transport()));
        (app, handle)
    }

    fn inbound_tokens(app: &mut App) -> Vec<String> {
        let messages = app
            .world()
            .resource::<bevy::ecs::message::Messages<InboundMessage>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).map(|m| m.token.clone()).collect()
    }

    #[test]
    fn a_polled_client_message_reaches_the_simulations_inbound_bus() {
        let (mut app, handle) = seam_app();
        handle.send(
            "player-token-1",
            ClientMessage::Identify {
                token: "player-token-1".to_string(),
                name: "Ada".to_string(),
            },
        );
        app.update();
        assert_eq!(inbound_tokens(&mut app), vec!["player-token-1".to_string()]);
    }

    #[test]
    fn the_seam_refuses_the_host_operators_reserved_token_at_ingress() {
        // `__local_console__` skips the station-tenure branch of
        // `is_command_authorized` and carries host mission-abort authority. A
        // transport participant must not be able to claim it, in-process or
        // otherwise — the browser refuses it at its own ingress too.
        let (mut app, handle) = seam_app();
        handle.send(
            crate::console_bridge::LOCAL_CONSOLE_TOKEN,
            ClientMessage::Identify {
                token: crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
                name: "impostor".to_string(),
            },
        );
        handle.send(
            "ai:helm",
            ClientMessage::Identify {
                token: "ai:helm".to_string(),
                name: "impostor".to_string(),
            },
        );
        app.update();
        assert!(
            inbound_tokens(&mut app).is_empty(),
            "no reserved token may cross the native ingress"
        );
    }

    #[test]
    fn a_disconnect_reaches_the_lobbys_disconnect_bus() {
        let (mut app, handle) = seam_app();
        handle.disconnect("player-token-1");
        app.update();
        let messages = app
            .world()
            .resource::<bevy::ecs::message::Messages<PlayerDisconnected>>();
        let mut cursor = messages.get_cursor();
        let tokens: Vec<String> = cursor.read(messages).map(|m| m.token.clone()).collect();
        assert_eq!(tokens, vec!["player-token-1".to_string()]);
    }

    #[test]
    fn outbound_messages_reach_the_transport_with_their_target_and_delivery_class() {
        let (mut app, handle) = seam_app();
        app.world_mut().write_message(OutboundMessage {
            target: Target::Token("player-token-1".to_string()),
            msg: ServerMessage::GameStarted,
            delivery: DeliveryClass::Reliable,
        });
        app.update();
        let dispatched = handle.drain_outbound();
        assert_eq!(dispatched.len(), 1, "one dispatch");
        assert_eq!(
            dispatched[0].0,
            Target::Token("player-token-1".to_string()),
            "the resolved audience target reaches the transport unflattened"
        );
        assert_eq!(dispatched[0].2, DeliveryClass::Reliable);
    }

    #[test]
    fn a_host_with_no_transport_still_drains_its_outbox() {
        let mut app = App::new();
        app.add_message::<InboundMessage>()
            .add_message::<OutboundMessage>()
            .add_message::<PlayerDisconnected>()
            .add_plugins(NativeTransportPlugin);
        app.world_mut().write_message(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::GameStarted,
            delivery: DeliveryClass::Reliable,
        });
        // Two frames: Bevy's double-buffered messages are cleared one frame
        // after they are written, so the second update is where an undrained
        // outbox would show up.
        app.update();
        app.update();
        let messages = app
            .world()
            .resource::<bevy::ecs::message::Messages<OutboundMessage>>();
        assert_eq!(
            messages.len(),
            0,
            "an unconnected host must not accumulate an unbounded outbox"
        );
    }
}
