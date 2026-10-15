use super::*;
use crate::core::messages::{ConsoleId, ScanBlackboard, ScanReadingSnapshot};
use phoenix_model::console::{
    ConsoleConfig, ConsoleFeed, ConsoleHostSnapshot, ConsoleObservationKind,
};
use phoenix_model::observation::ObservationMetadata;

#[test]
fn held_metadata_is_cache_stable_and_handoff_preserves_sample_time() {
    let mut app = App::new();
    let mut sessions = crate::lobby::Sessions(crate::lobby::session::SessionManager::new());
    for (token, seat) in [("first", "a"), ("second", "b")] {
        sessions.0.register(token.into(), token.into()).unwrap();
        sessions.0.set_station(token, Some(StationId(seat.into())));
    }
    app.insert_resource(sessions);
    app.insert_resource(crate::sim_tick::SimTick(50));
    app.init_resource::<crate::server_app::SimOutbox>();
    let mut config: crate::ship::config::ShipConfig = toml::from_str("").unwrap();
    config.consoles.push(ConsoleConfig {
        id: ConsoleId("science".into()),
        feeds: vec![ConsoleFeed {
            source: SystemId("scan/7:scanner".into()),
            kind: ConsoleObservationKind::Scan,
        }],
        ..Default::default()
    });
    let sample = ObservationMetadata {
        observed_tick: Some(17),
        ..Default::default()
    };
    let board = SystemBlackboard::Scan(ScanBlackboard {
        owner: Some(SystemId("scanner".into())),
        reading: Some(ScanReadingSnapshot {
            observation: Some(sample.clone()),
            ..Default::default()
        }),
        ..Default::default()
    });
    let entity = app
        .world_mut()
        .spawn((
            crate::server_app::LocalShip,
            ShipConfigComponent(config),
            crate::server_app::ShipSystemBlackboards(HashMap::from([(
                SystemId("scan/7:scanner".into()),
                board,
            )])),
            ConsoleHosts(vec![ConsoleHostSnapshot {
                console: ConsoleId("science".into()),
                host: Some(StationId("a".into())),
                rating: "Std".into(),
            }]),
        ))
        .id();
    publish_console_blackboards(app.world_mut(), Some(1));
    app.world_mut()
        .resource_mut::<crate::server_app::SimOutbox>()
        .drain()
        .into_iter()
        .for_each(drop);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 80;
    publish_console_blackboards(app.world_mut(), Some(1));
    assert_eq!(
        app.world_mut()
            .resource_mut::<crate::server_app::SimOutbox>()
            .drain()
            .len(),
        0
    );
    app.world_mut().get_mut::<ConsoleHosts>(entity).unwrap().0[0].host =
        Some(StationId("b".into()));
    publish_console_blackboards(app.world_mut(), Some(1));
    let messages: Vec<_> = app
        .world_mut()
        .resource_mut::<crate::server_app::SimOutbox>()
        .drain()
        .into_iter()
        .collect();
    let granted = messages
        .iter()
        .find(|entry| entry.target == crate::lobby::Target::Token("second".into()))
        .unwrap();
    let ServerMessage::BlackboardUpdate {
        updates,
        console_scope_tick,
        ..
    } = &granted.message
    else {
        panic!("expected scoped publication")
    };
    assert_eq!(*console_scope_tick, Some(80));
    assert_eq!(updates[0].1.recorded_observations(), vec![&sample]);
    let revoked = messages
        .iter()
        .find(|entry| entry.target == crate::lobby::Target::Token("first".into()))
        .unwrap();
    let ServerMessage::BlackboardUpdate {
        updates,
        console_scope,
        ..
    } = &revoked.message
    else {
        panic!("expected revocation")
    };
    assert!(updates.is_empty());
    assert_eq!(console_scope.as_ref().unwrap().len(), 0);
}
