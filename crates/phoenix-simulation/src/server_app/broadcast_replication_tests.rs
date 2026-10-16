use super::*;

fn seed(
    positions: &mut LastBroadcastEntityPositions,
    health: &mut LastBroadcastEntityHealth,
    uuid: &str,
) {
    positions.0.insert(uuid.into(), (Vec3::ZERO, 0.0));
    health
        .0
        .insert(uuid.into(), (Some(1.0), Some(1.0), None, None));
}

#[test]
fn uuid_pruning_removes_only_named_entities() {
    let mut positions = LastBroadcastEntityPositions::default();
    let mut health = LastBroadcastEntityHealth::default();
    for uuid in ["keep", "gone-a", "gone-b"] {
        seed(&mut positions, &mut health, uuid);
    }

    prune_entity_replication_caches(
        &mut positions,
        &mut health,
        &["gone-a".into(), "gone-b".into(), "unknown".into()],
    );

    assert_eq!(positions.0.keys().collect::<Vec<_>>(), vec!["keep"]);
    assert_eq!(health.0.keys().collect::<Vec<_>>(), vec!["keep"]);
}

#[test]
fn repeated_uuid_pruning_keeps_long_session_caches_bounded() {
    let mut positions = LastBroadcastEntityPositions::default();
    let mut health = LastBroadcastEntityHealth::default();
    const LIVE_AT_ONCE: usize = 5;

    for cycle in 0..500 {
        seed(&mut positions, &mut health, &format!("asteroid-{cycle}"));
        if cycle >= LIVE_AT_ONCE {
            prune_entity_replication_caches(
                &mut positions,
                &mut health,
                &[format!("asteroid-{}", cycle - LIVE_AT_ONCE)],
            );
        }
    }

    assert_eq!(positions.0.len(), LIVE_AT_ONCE);
    assert_eq!(health.0.len(), LIVE_AT_ONCE);
}

#[test]
fn console_health_keeps_detached_and_same_seat_scopes_distinct() {
    use crate::core::messages::{ConsoleId, SystemId};
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = "Pilot"
rank = "Ens."
[[console]]
id = "flight"
home_station = "helm"
screen = "fixture.html"
[[console]]
id = "drive"
home_station = "helm"
screen = "fixture.html"
[[console]]
id = "detached"
[[console]]
id = "unmodeled"
[[system]]
id = "a"
kind = "fixture"
console = "flight"
[[system]]
id = "b"
kind = "fixture"
console = "drive"
[[system]]
id = "c"
kind = "fixture"
console = "detached"
ai_only = true
[[system]]
id = "d"
kind = "fixture"
console = "detached"
ai_only = true
"#,
        &["fixture"],
    )
    .unwrap();
    let mut hull = crate::ship::damage::SystemHull::from_config(&[
        (SystemId("a".into()), 100.0),
        (SystemId("b".into()), 100.0),
        (SystemId("c".into()), 100.0),
        (SystemId("d".into()), 300.0),
        (SystemId("core".into()), 900.0),
    ]);
    hull.restore(&SystemId("a".into()), -75.0);
    hull.restore(&SystemId("c".into()), -100.0);
    let rows = console_health_from_parts(&config, Some(&hull), None);
    assert_eq!(
        rows.iter().map(|r| r.console.clone()).collect::<Vec<_>>(),
        ["flight", "drive", "detached", "unmodeled"].map(|id| ConsoleId(id.into()))
    );
    assert_eq!(
        rows.iter().map(|r| r.health).collect::<Vec<_>>(),
        vec![Some(0.25), Some(1.0), Some(0.75), None]
    );
    assert!(console_health_from_parts(&config, None, None)
        .iter()
        .all(|r| r.health.is_none()));
}

// Producer seam only: no simulation machinery or transport is registered. The
// connected case exercises the existing private-field construction/projection;
// its reserved local packet is the oracle for the local-only fast path.
fn context_fixture(connected: bool, hosted: bool) -> (App, Entity) {
    use crate::core::messages::{ConsoleId, StationId};
    use crate::ship::components::{ConsoleHosts, ConsoleWorkingViews, ShipConfigComponent};
    let mut app = App::new();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ))
    .insert_resource(crate::sim_tick::SimTick(41))
    .init_resource::<crate::audio_lifecycle::RoomAudioLifecycle>()
    .init_resource::<LastBroadcastEntityPositions>()
    .init_resource::<LastBroadcastEntityHealth>()
    .init_resource::<crate::console::repair::visibility::LastBroadcastHull>()
    .init_resource::<SimOutbox>()
    .init_resource::<StationImportanceRes>()
    .add_plugins(sim_state_broadcaster());
    app.world_mut()
        .resource_mut::<crate::audio_lifecycle::RoomAudioLifecycle>()
        .state
        .generation = 7;
    for (token, station) in [
        ("first", Some("first")),
        ("second", Some("second")),
        ("unseated", None),
    ] {
        let mut sessions = app.world_mut().resource_mut::<crate::lobby::Sessions>();
        sessions.0.register(token.into(), token.into()).unwrap();
        sessions
            .0
            .set_station(token, station.map(|station| StationId(station.into())));
        if !connected {
            sessions.0.disconnect(token);
        }
    }
    let config = toml::from_str(
        "[[console]]\nid='one'\nhome_station='first'\n[[console]]\nid='two'\nhome_station='second'\n[[system]]\nid='a'\nkind='fixture'\nconsole='one'\n[[system]]\nid='b'\nkind='fixture'\nconsole='two'",
    ).unwrap();
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        SystemId("a".into()),
        crate::ship::control_source::ControlSource::Offline,
    );
    sources.set(
        SystemId("b".into()),
        crate::ship::control_source::ControlSource::Human,
    );
    let entity = app
        .world_mut()
        .spawn((
            LocalShip,
            EntityUuid("snapshot-owner".into()),
            Transform::from_xyz(1.0, 2.0, 3.0),
            ShipConfigComponent(config),
            crate::ship::components::ShipSystemControlSources(sources),
            crate::ship::state::ShipRedAlert(true),
            ConsoleHosts(if hosted {
                ["one", "two"]
                    .into_iter()
                    .zip(["first", "second"])
                    .map(
                        |(console, host)| phoenix_model::console::ConsoleHostSnapshot {
                            console: ConsoleId(console.into()),
                            host: Some(StationId(host.into())),
                            rating: "Std".into(),
                        },
                    )
                    .collect()
            } else {
                Vec::new()
            }),
        ))
        .id();
    app.world_mut()
        .get_mut::<ConsoleWorkingViews>(entity)
        .unwrap()
        .0 = ["one", "two"]
        .into_iter()
        .map(|console| {
            (
                ConsoleId(console.into()),
                phoenix_model::console::ConsoleWorkingView {
                    overlay: Some(format!("private-{console}")),
                    ..Default::default()
                },
            )
        })
        .collect();
    (app, entity)
}

fn context_packets(
    app: &mut App,
) -> Vec<(crate::lobby::Target, crate::core::messages::SimSnapshot)> {
    let producer = app.world().resource::<crate::core::broadcast::broadcaster::BroadcastRegistry<crate::core::broadcast::Sim>>().registrations[0].producer.clone();
    let direct = producer(app.world_mut());
    let mut packets = app
        .world_mut()
        .resource_mut::<SimOutbox>()
        .drain()
        .into_iter()
        .map(|entry| {
            assert_eq!(entry.delivery, DeliveryClass::Snapshot);
            (entry.target, entry.message)
        })
        .collect::<Vec<_>>();
    packets.extend(
        direct
            .into_iter()
            .map(|message| (crate::lobby::Target::All, message)),
    );
    packets
        .into_iter()
        .map(|(target, message)| {
            let ServerMessage::SimState { snapshot } = message else {
                panic!("fixture has no physical hull updates")
            };
            (target, snapshot)
        })
        .collect()
}

#[test]
fn local_only_snapshot_matches_connected_local_packet_with_metadata_and_cache_progression() {
    use crate::core::messages::ConsoleId;
    let (mut alone, alone_entity) = context_fixture(false, true);
    let (mut crewed, crewed_entity) = context_fixture(true, true);
    let local = crate::lobby::Target::Token(crate::console_bridge::LOCAL_CONSOLE_TOKEN.into());
    for step in 0..3 {
        let only = context_packets(&mut alone);
        let all = context_packets(&mut crewed);
        assert_eq!(only.len(), 1);
        assert_eq!(all.len(), 4);
        assert_eq!(only[0].0, local);
        let reserved = all.iter().find(|(target, _)| target == &local).unwrap();
        assert_eq!(
            serde_json::to_value(&only[0].1).unwrap(),
            serde_json::to_value(&reserved.1).unwrap()
        );
        assert_eq!(only[0].1.observation_tick, Some(41 + step));
        assert_eq!(only[0].1.observation_generation, Some(7));
        assert_eq!(
            only[0].1.station_importance.iter().any(|row| row.critical),
            step % 2 == 0,
            "the real importance ingest must still follow each alert edge"
        );
        assert!(only[0].1.system_status.is_empty() && only[0].1.console_views.is_empty());
        assert!(
            !only[0].1.entity_states.is_empty(),
            "position deltas must still advance"
        );
        assert_eq!(
            alone.world().resource::<LastBroadcastEntityPositions>().0,
            crewed.world().resource::<LastBroadcastEntityPositions>().0
        );
        assert_eq!(
            alone.world().resource::<LastBroadcastEntityHealth>().0,
            crewed.world().resource::<LastBroadcastEntityHealth>().0
        );
        assert_eq!(
            serde_json::to_value(
                alone
                    .world()
                    .resource::<StationImportanceRes>()
                    .0
                    .station_snapshots(&HashMap::new())
            )
            .unwrap(),
            serde_json::to_value(
                crewed
                    .world()
                    .resource::<StationImportanceRes>()
                    .0
                    .station_snapshots(&HashMap::new())
            )
            .unwrap()
        );
        let first = &all
            .iter()
            .find(|(target, _)| target == &crate::lobby::Target::Token("first".into()))
            .unwrap()
            .1;
        let unseated = &all
            .iter()
            .find(|(target, _)| target == &crate::lobby::Target::Token("unseated".into()))
            .unwrap()
            .1;
        assert!(unseated.system_status.is_empty() && unseated.console_views.is_empty());
        assert_eq!(
            first.console_views.keys().cloned().collect::<Vec<_>>(),
            vec![ConsoleId("one".into())]
        );
        assert_eq!(
            first.system_status.keys().cloned().collect::<Vec<_>>(),
            vec![SystemId("a".into())]
        );
        assert!(first.system_status[&SystemId("a".into())]
            .reasons
            .iter()
            .any(|reason| reason.code == phoenix_model::system_status::StatusReasonCode::Offline));
        assert_eq!(first.observation_tick, only[0].1.observation_tick);
        assert_eq!(
            first.observation_generation,
            only[0].1.observation_generation
        );
        for (app, entity) in [(&mut alone, alone_entity), (&mut crewed, crewed_entity)] {
            app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
            app.world_mut()
                .get_mut::<Transform>(entity)
                .unwrap()
                .translation
                .x += 1.0;
            app.world_mut()
                .get_mut::<crate::ship::state::ShipRedAlert>(entity)
                .unwrap()
                .0 = step % 2 != 0;
        }
    }
}

#[test]
fn unhosted_all_packet_excludes_private_context_even_with_connected_sessions() {
    let (mut alone, _) = context_fixture(false, false);
    let (mut crewed, _) = context_fixture(true, false);
    let only = context_packets(&mut alone);
    let connected = context_packets(&mut crewed);
    assert_eq!(only.len(), 1);
    assert_eq!(connected.len(), 1);
    assert_eq!(connected[0].0, crate::lobby::Target::All);
    assert_eq!(
        serde_json::to_value(&only[0].1).unwrap(),
        serde_json::to_value(&connected[0].1).unwrap()
    );
    assert!(connected[0].1.system_status.is_empty() && connected[0].1.console_views.is_empty());
    assert_eq!(connected[0].1.observation_tick, Some(41));
    assert_eq!(connected[0].1.observation_generation, Some(7));
}
