use super::*;
use crate::core::messages::{
    ConsoleId, HelmBlackboard, QueueEntryPreview, RepairBlackboard, SystemHullStatus, TeamSlot,
};
use phoenix_model::console::{
    ConsoleConfig, ConsoleFeed, ConsoleHostSnapshot, ConsoleObservationKind,
};

fn fixture() -> (App, Entity) {
    let mut app = App::new();
    let mut sessions = crate::lobby::Sessions(crate::lobby::session::SessionManager::new());
    sessions.0.register("crew".into(), "Crew".into()).unwrap();
    sessions
        .0
        .set_station("crew", Some(StationId("home".into())));
    app.insert_resource(sessions);
    app.insert_resource(crate::sim_tick::SimTick(10));
    app.init_resource::<crate::server_app::SimOutbox>();
    crate::server_app::register_blackboard_replication_lifecycle(&mut app);
    let mut config: crate::ship::config::ShipConfig = toml::from_str("").unwrap();
    config.consoles.push(ConsoleConfig {
        id: ConsoleId("screen".into()),
        feeds: ["a", "b"]
            .into_iter()
            .map(|id| ConsoleFeed {
                source: SystemId(id.into()),
                kind: ConsoleObservationKind::Helm,
            })
            .collect(),
        ..Default::default()
    });
    let boards = crate::server_app::ShipSystemBlackboards(
        ["a", "b"]
            .into_iter()
            .map(|id| {
                (
                    SystemId(id.into()),
                    SystemBlackboard::Helm(HelmBlackboard::default()),
                )
            })
            .collect(),
    );
    let entity = app
        .world_mut()
        .spawn((
            crate::server_app::LocalShip,
            ShipConfigComponent(config),
            boards,
            ConsoleHosts(vec![ConsoleHostSnapshot {
                console: ConsoleId("screen".into()),
                host: Some(StationId("home".into())),
                rating: "Std".into(),
            }]),
        ))
        .id();
    (app, entity)
}

fn sent(app: &mut App, generation: u64) -> Vec<ServerMessage> {
    publish_console_blackboards(app.world_mut(), Some(generation));
    app.world_mut()
        .resource_mut::<crate::server_app::SimOutbox>()
        .drain()
        .into_iter()
        .filter(|entry| entry.target == crate::lobby::Target::Token("crew".into()))
        .map(|entry| entry.message)
        .collect()
}

fn assert_complete_local_packet(app: &mut App, entity: Entity) {
    let boards: BTreeMap<_, _> = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .iter()
        .map(|(id, board)| (id.clone(), board.clone()))
        .collect();
    let expected = ServerMessage::BlackboardUpdate {
        console_scope: Some(boards.keys().cloned().collect()),
        console_scope_tick: Some(app.world().resource::<crate::sim_tick::SimTick>().0),
        updates: boards.into_iter().collect(),
        presentation_generation: Some(0),
    };
    app.update();
    let entries = app
        .world_mut()
        .resource_mut::<crate::server_app::SimOutbox>()
        .drain();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].target,
        crate::lobby::Target::Token(crate::console_bridge::LOCAL_CONSOLE_TOKEN.into())
    );
    assert_eq!(
        entries[0].delivery,
        crate::core::messages::DeliveryClass::Snapshot
    );
    let codec = crate::core::codec::JsonCodec;
    assert_eq!(
        codec.encode_server(&entries[0].message).unwrap(),
        codec.encode_server(&expected).unwrap()
    );
}

#[test]
fn local_only_cached_query_preserves_complete_packets_across_change_and_removal() {
    let (mut app, entity) = fixture();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ))
    .add_systems(Update, crate::server_app::broadcast_blackboard_updates);
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            SystemId("repair".into()),
            SystemBlackboard::Repair(RepairBlackboard::default()),
        );
    // The reserved route retains raw Repair data even with no phone viewers.
    assert_complete_local_packet(&mut app, entity);
    app.update();
    publish_console_blackboards(app.world_mut(), None);
    assert!(app
        .world_mut()
        .resource_mut::<crate::server_app::SimOutbox>()
        .drain()
        .is_empty());

    // A new archetype must remain visible to the retained scheduled QueryState.
    app.world_mut()
        .entity_mut(entity)
        .insert(Name::new("new archetype"));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            SystemId("b".into()),
            SystemBlackboard::Helm(HelmBlackboard {
                x: 42.0,
                ..Default::default()
            }),
        );
    assert_complete_local_packet(&mut app, entity);
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .remove(&SystemId("a".into()));
    assert_complete_local_packet(&mut app, entity);
    assert_eq!(app.world().resource::<LastConsolePublications>().0.len(), 1);
}

#[test]
fn scoped_updates_are_complete_on_change_and_revoke_then_rebase() {
    let (mut app, entity) = fixture();
    assert_eq!(sent(&mut app, 1).len(), 1);
    assert!(sent(&mut app, 1).is_empty());
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 11;
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            SystemId("b".into()),
            SystemBlackboard::Helm(HelmBlackboard {
                movement: None,
                x: 42.0,
                ..Default::default()
            }),
        );
    let messages = sent(&mut app, 1);
    let ServerMessage::BlackboardUpdate {
        updates,
        console_scope,
        console_scope_tick,
        ..
    } = &messages[0]
    else {
        panic!("board projection")
    };
    assert_eq!(
        updates
            .iter()
            .map(|(id, _)| id.0.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(console_scope.as_ref().unwrap().len(), 2);
    assert_eq!(*console_scope_tick, Some(11));
    app.world_mut().get_mut::<ConsoleHosts>(entity).unwrap().0[0].host = None;
    let messages = sent(&mut app, 1);
    let ServerMessage::BlackboardUpdate {
        updates,
        console_scope,
        ..
    } = &messages[0]
    else {
        panic!("empty revoke")
    };
    assert!(updates.is_empty());
    assert_eq!(console_scope, &Some(vec![]));
    app.world_mut().get_mut::<ConsoleHosts>(entity).unwrap().0[0].host =
        Some(StationId("home".into()));
    assert_eq!(sent(&mut app, 1).len(), 1);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 0;
    let messages = sent(&mut app, 2);
    let ServerMessage::BlackboardUpdate {
        updates,
        presentation_generation,
        console_scope_tick,
        ..
    } = &messages[0]
    else {
        panic!("rebase")
    };
    assert_eq!(updates.len(), 2);
    assert_eq!(
        (*presentation_generation, *console_scope_tick),
        (Some(2), Some(0))
    );
}

fn repair_config() -> crate::ship::config::ShipConfig {
    toml::from_str(
        r#"
[[console]]
id="repair-screen"
home_station="engineering"
[[console]]
id="reader"
[[console.feed]]
source="repair"
kind="Repair"
[[console]]
id="unbound"
[[system]]
id="repair"
kind="repair"
console="repair-screen"
[[system]]
id="hidden"
kind="sensors"
console="unbound"
[[system]]
id="onsite"
kind="sensors"
console="unbound"
"#,
    )
    .unwrap()
}

#[test]
fn repair_publication_follows_console_and_explicit_feed_without_revealing_hidden_detail() {
    use crate::console::repair::visibility::HullVisibility;
    use crate::ship::damage::DamageTier;
    let config = repair_config();
    let ids = ["repair", "core", "onsite", "hidden"];
    let entries: Vec<_> = ids
        .iter()
        .map(|id| SystemHullStatus {
            physical: None,
            system_id: SystemId((*id).into()),
            display_name: (*id).into(),
            current: 25.0,
            max_hp: 100.0,
            tier: DamageTier::Operational,
            debuff_magnitude: 0.0,
        })
        .collect();
    let owners = ids
        .iter()
        .map(|id| {
            (
                SystemId((*id).into()),
                (*id != "core").then(|| StationId((*id).into())),
            )
        })
        .collect();
    let visibility = HullVisibility::new(
        entries.clone(),
        owners,
        Some(StationId("engineering".into())),
        vec![SystemId("onsite".into())],
    );
    let raw = RepairBlackboard {
        repair_groups: Vec::new(),
        system_hull: entries,
        priority_targets: ids.iter().map(|id| SystemId((*id).into())).collect(),
        teams: vec![TeamSlot::Idle],
        queue_depth: ids
            .iter()
            .map(|id| QueueEntryPreview {
                observations: vec![],
                group: fixture_repair_group(id),
                station_label: (*id).into(),
                tier: DamageTier::Operational,
                deficit: 75.0,
            })
            .collect(),
        ..Default::default()
    };
    let boards = HashMap::from([(
        SystemId("repair".into()),
        SystemBlackboard::Repair(raw.clone()),
    )]);
    let expected = project_permitted_board(
        &config,
        &SystemId("repair".into()),
        &boards[&SystemId("repair".into())],
        Some(&visibility),
    );
    let SystemBlackboard::Repair(projected) = &expected else {
        unreachable!()
    };
    assert_eq!(
        projected
            .system_hull
            .iter()
            .map(|row| row.system_id.0.as_str())
            .collect::<Vec<_>>(),
        ["repair", "core", "onsite"]
    );
    assert_eq!(
        projected
            .priority_targets
            .iter()
            .map(|id| id.0.as_str())
            .collect::<Vec<_>>(),
        ["repair", "core", "onsite"]
    );
    assert_eq!(
        projected
            .queue_depth
            .iter()
            .map(|row| row.station_label.as_str())
            .collect::<Vec<_>>(),
        ["repair", "core", "onsite"]
    );
    assert_eq!(projected.teams, raw.teams);
    for (console, seat) in [
        ("repair-screen", "engineering"),
        ("repair-screen", "visitor"),
        ("reader", "visitor"),
    ] {
        let hosts = ConsoleHosts(vec![ConsoleHostSnapshot {
            console: ConsoleId(console.into()),
            host: Some(StationId(seat.into())),
            rating: "Std".into(),
        }]);
        let permitted =
            permitted_publications(&config, &hosts, Some(&StationId(seat.into())), &boards);
        let board = permitted.get(&SystemId("repair".into())).unwrap();
        assert_eq!(
            project_permitted_board(
                &config,
                &SystemId("repair".into()),
                board,
                Some(&visibility)
            ),
            expected
        );
    }
    let hosts = ConsoleHosts(vec![ConsoleHostSnapshot {
        console: ConsoleId("unbound".into()),
        host: Some(StationId("visitor".into())),
        rating: "Std".into(),
    }]);
    assert!(
        permitted_publications(&config, &hosts, Some(&StationId("visitor".into())), &boards)
            .is_empty()
    );
}

#[test]
fn registered_reconnect_matches_live_repair_feed_and_does_not_change_publication_cache() {
    let (mut app, entity) = fixture();
    app.world_mut()
        .get_mut::<ShipConfigComponent>(entity)
        .unwrap()
        .0 = repair_config();
    app.world_mut().get_mut::<ConsoleHosts>(entity).unwrap().0[0].console =
        ConsoleId("reader".into());
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0 = HashMap::from([(
        SystemId("repair".into()),
        SystemBlackboard::Repair(RepairBlackboard {
            repair_groups: Vec::new(),
            teams: vec![TeamSlot::Idle],
            ..Default::default()
        }),
    )]);
    let hull = crate::ship::damage::SystemHull::from_config(&[
        (SystemId("repair".into()), 100.0),
        (SystemId("hidden".into()), 100.0),
    ]);
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntitySystemHull(hull));
    let live = sent(&mut app, 0);
    let cache = app.world().resource::<LastConsolePublications>().0.clone();
    let reconnect =
        crate::core::broadcast::reconnect_registered_replication(app.world_mut(), "crew");
    let reconnected = reconnect
        .into_iter()
        .find(|message| matches!(message, ServerMessage::BlackboardUpdate { .. }))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&live[0]).unwrap(),
        serde_json::to_value(reconnected).unwrap()
    );
    assert_eq!(app.world().resource::<LastConsolePublications>().0, cache);
}

fn fixture_repair_group(id: &str) -> crate::core::messages::RepairGroup {
    use crate::core::messages::{RepairGroup, StationId};
    if id == "core" {
        RepairGroup::Core
    } else {
        RepairGroup::Station(StationId(id.into()))
    }
}
