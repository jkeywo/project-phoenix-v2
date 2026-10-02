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
fn three_legs_fold_into_one_link_and_each_reports_its_own_departures() {
    // What `phoenix-host` builds since issue #1353: local panes, the direct
    // LAN accept leg and the cloud relay, folded through `Box<dyn>` into the
    // one `NativeTransportLink` resource. Two claims worth pinning: every
    // leg's events reach the simulation (a fold that dropped one would leave
    // a whole crew silently unable to act), and a `Disconnected` carries the
    // token of whoever's socket actually died — the legs keep separate
    // connection tables, so a departure on one must not be attributed to a
    // peer on another.
    let panes = LoopbackHandle::default();
    let direct = LoopbackHandle::default();
    let relay = LoopbackHandle::default();

    let mut app = App::new();
    app.add_message::<InboundMessage>()
        .add_message::<OutboundMessage>()
        .add_message::<PlayerDisconnected>()
        .add_plugins(NativeTransportPlugin);
    let mut composed: Option<Box<dyn NativeTransport>> = None;
    for leg in [
        Box::new(panes.transport()) as Box<dyn NativeTransport>,
        Box::new(direct.transport()),
        Box::new(relay.transport()),
    ] {
        composed = Some(match composed {
            Some(existing) => Box::new(PairedTransport::new(existing, leg)),
            None => leg,
        });
    }
    app.insert_resource(NativeTransportLink(composed.expect("three legs")));

    for (handle, token) in [
        (&panes, "pane-token"),
        (&direct, "lan-token"),
        (&relay, "cloud-token"),
    ] {
        handle.send(
            token,
            ClientMessage::Identify {
                token: token.to_string(),
                name: "Ada".to_string(),
            },
        );
    }
    direct.disconnect("lan-token");
    app.update();

    assert_eq!(
        inbound_tokens(&mut app),
        vec![
            "pane-token".to_string(),
            "lan-token".to_string(),
            "cloud-token".to_string()
        ],
        "every leg's traffic reaches the simulation, in leg order"
    );
    let messages = app
        .world()
        .resource::<bevy::ecs::message::Messages<PlayerDisconnected>>();
    let mut cursor = messages.get_cursor();
    let gone: Vec<String> = cursor.read(messages).map(|m| m.token.clone()).collect();
    assert_eq!(
        gone,
        vec!["lan-token".to_string()],
        "only the leg whose socket died reports a departure"
    );

    // …and one outbound message is offered to all three, so each decides for
    // itself whether the resolved `Target` names anyone it is carrying.
    app.world_mut().write_message(OutboundMessage {
        target: Target::All,
        msg: ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });
    app.update();
    for handle in [&panes, &direct, &relay] {
        assert_eq!(handle.drain_outbound().len(), 1);
    }
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
