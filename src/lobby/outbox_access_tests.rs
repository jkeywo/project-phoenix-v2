// Included in server::tests to reuse its ordinary LobbyPlugin harness.

#[test]
fn outbox_access_empty_single_and_multiple_drains_keep_fifo_and_delivery() {
    let mut app = App::new();
    app.init_resource::<LobbyOutbox>()
        .add_message::<OutboundMessage>()
        .add_systems(Update, drain_lobby_outbox);
    let mut cursor = bevy::ecs::message::MessageCursor::<OutboundMessage>::default();
    for count in [0, 1, 3, 0] {
        let expected: Vec<_> = (0..count)
            .map(|index| {
                (
                    Target::Token(format!("recipient-{index}")),
                    ServerMessage::GameStartCountdown {
                        remaining_secs: index + 1,
                    },
                )
            })
            .collect();
        app.world_mut().resource_mut::<LobbyOutbox>().0 = expected.clone();
        app.update();
        let actual: Vec<_> = cursor
            .read(app.world().resource::<Messages<OutboundMessage>>())
            .cloned()
            .collect();
        assert_eq!(actual.len(), expected.len());
        for (actual, (target, message)) in actual.iter().zip(expected) {
            assert_eq!(actual.target, target);
            assert_eq!(actual.delivery, DeliveryClass::Reliable);
            assert_eq!(
                serde_json::to_value(&actual.msg).unwrap(),
                serde_json::to_value(message).unwrap()
            );
        }
        assert!(app.world().resource::<LobbyOutbox>().0.is_empty());
    }
}

#[derive(Resource, Default)]
struct OutboxReconnectWitness(Vec<String>);

fn outbox_reconnect_projection(world: &mut World, token: &str) -> Vec<ServerMessage> {
    assert!(
        world
            .resource::<LobbyOutbox>()
            .0
            .iter()
            .any(|(target, msg)| {
                target == &Target::Token(token.into())
                    && matches!(msg, ServerMessage::Welcome { .. })
            }),
        "the real reconnect callback must still see the pending Welcome before the drain"
    );
    world
        .resource_mut::<OutboxReconnectWitness>()
        .0
        .push(token.into());
    vec![ServerMessage::BlackboardUpdate { updates: vec![] }]
}

#[test]
fn outbox_access_start_and_reconnect_keep_the_existing_boundary_order() {
    use crate::core::broadcast::{RegisterReplicationLifecycle, ReplicationLifecycleAdapter};
    let mut app = test_app();
    app.init_resource::<OutboxReconnectWitness>()
        .init_resource::<crate::server_app::SimOutbox>()
        .register_replication_lifecycle(
            ReplicationLifecycleAdapter::new("outbox-regression")
                .with_reconnect(outbox_reconnect_projection),
        )
        // The production registration's existing edges, without adding any.
        .add_systems(
            FixedUpdate,
            crate::server_app::refresh_caches_on_midgame_reconnect
                .after(crate::lobby::LobbySystemSet)
                .before(drain_lobby_outbox)
                .before(crate::sim_sets::SimSet::Broadcast),
        );
    let identify = |token: &str| ClientMessage::Identify {
        token: token.into(),
        name: token.into(),
    };
    push(&mut app, "captain", identify("captain"));
    tick(&mut app);
    push(&mut app, "captain", ClientMessage::SetReady { ready: true });
    let countdown = tick(&mut app);
    assert!(countdown
        .iter()
        .any(|m| matches!(m.msg, ServerMessage::GameStartCountdown { .. })));
    app.world_mut()
        .resource_mut::<CountdownTimer>()
        .remaining_secs = 0.001;
    let start = tick(&mut app);
    assert_eq!(
        start
            .iter()
            .filter(|m| matches!(m.msg, ServerMessage::GameStarted))
            .count(),
        1
    );
    assert!(app.world().resource::<LobbyOutbox>().0.is_empty());
    assert!(!tick(&mut app)
        .iter()
        .any(|m| matches!(m.msg, ServerMessage::GameStarted)));
    assert_eq!(
        *app.world().resource::<State<GamePhase>>().get(),
        GamePhase::InProgress
    );

    push(&mut app, "captain", identify("captain"));
    push(&mut app, "second", identify("second"));
    let reconnect = tick(&mut app);
    let welcomes: Vec<_> = reconnect
        .iter()
        .filter(|m| matches!(m.msg, ServerMessage::Welcome { .. }))
        .map(|m| {
            assert_eq!(m.delivery, DeliveryClass::Reliable);
            m.target.clone()
        })
        .collect();
    assert_eq!(
        welcomes,
        vec![
            Target::Token("captain".into()),
            Target::Token("second".into())
        ]
    );
    assert_eq!(
        app.world().resource::<OutboxReconnectWitness>().0,
        ["captain", "second"]
    );
    let snapshots: Vec<_> = app
        .world_mut()
        .resource_mut::<crate::server_app::SimOutbox>()
        .drain();
    assert_eq!(snapshots.len(), 2);
    for (snapshot, target) in snapshots.iter().zip(welcomes) {
        assert_eq!(snapshot.target, target);
        assert_eq!(snapshot.delivery, DeliveryClass::Snapshot);
        assert!(matches!(
            snapshot.message,
            ServerMessage::BlackboardUpdate { .. }
        ));
    }
    assert!(app.world().resource::<LobbyOutbox>().0.is_empty());
    assert!(!tick(&mut app)
        .iter()
        .any(|m| matches!(m.msg, ServerMessage::Welcome { .. })));
    assert_eq!(app.world().resource::<OutboxReconnectWitness>().0.len(), 2);
}
