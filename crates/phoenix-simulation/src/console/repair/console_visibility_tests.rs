use super::*;
use crate::core::messages::ConsoleId;
use crate::ship::components::ConsoleHosts;
use phoenix_model::console::ConsoleHostSnapshot;

fn config() -> ShipConfig {
    toml::from_str(
        r#"
[[console]]
id="repair-screen"
home_station="engineering"
[[console]]
id="visitor"
[[system]]
id="repair"
kind="repair"
console="repair-screen"
[[system]]
id="detached"
kind="sensors"
console="visitor"
"#,
    )
    .unwrap()
}
fn hosts(seat: &str) -> ConsoleHosts {
    ConsoleHosts(vec![
        ConsoleHostSnapshot {
            console: ConsoleId("repair-screen".into()),
            host: Some(StationId("engineering".into())),
            rating: "Std".into(),
        },
        ConsoleHostSnapshot {
            console: ConsoleId("visitor".into()),
            host: Some(StationId(seat.into())),
            rating: "Std".into(),
        },
    ])
}
fn ids(message: &ServerMessage) -> Vec<String> {
    let ServerMessage::SystemHullUpdate { entries, .. } = message else {
        panic!("hull update")
    };
    entries.iter().map(|row| row.system_id.0.clone()).collect()
}
fn hull() -> SystemHull {
    SystemHull::from_config(&[
        (SystemId("repair".into()), 100.0),
        (SystemId("detached".into()), 100.0),
        (SystemId("core".into()), 100.0),
    ])
}
#[test]
fn detached_console_equipment_is_not_core_and_cohosting_does_not_pool_console_views() {
    let config = config();
    let visibility = HullVisibility::from_parts(&hull(), &config, None);
    let repair = ConsoleId("repair-screen".into());
    let visitor = ConsoleId("visitor".into());
    assert!(visibility.console_can_see(&config, &repair, &SystemId("core".into())));
    assert!(!visibility.console_can_see(&config, &repair, &SystemId("detached".into())));
    assert!(visibility.console_can_see(&config, &visitor, &SystemId("detached".into())));
    assert!(!visibility.console_can_see(&config, &visitor, &SystemId("core".into())));
    let union = visibility.projection_for_host(
        &config,
        &hosts("engineering"),
        Some(&StationId("engineering".into())),
    );
    assert_eq!(union.entries.len(), 3);
    assert!(!visibility.console_can_see(&config, &repair, &SystemId("detached".into())));
}
#[test]
fn live_and_registered_reconnect_revoke_previous_host_and_follow_current_console_host() {
    let mut app = App::new();
    let mut sessions = Sessions(crate::lobby::session::SessionManager::new());
    for (token, seat) in [("old", "helm"), ("new", "tactical")] {
        sessions.0.register(token.into(), token.into()).unwrap();
        sessions.0.set_station(token, Some(StationId(seat.into())));
    }
    app.insert_resource(sessions);
    app.init_resource::<crate::server_app::SimOutbox>();
    register_hull_replication_lifecycle(&mut app);
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::LocalShip,
            crate::entities::spawner::EntitySystemHull(hull()),
            crate::ship_plugin::ShipConfigComponent(config()),
            hosts("helm"),
        ))
        .id();
    push_hull_updates(app.world_mut());
    assert_eq!(
        app.world().resource::<LastBroadcastHull>().0["old"]
            .entries
            .len(),
        1
    );
    app.world_mut()
        .resource_mut::<crate::server_app::SimOutbox>()
        .drain();
    app.world_mut().entity_mut(ship).insert(hosts("tactical"));
    push_hull_updates(app.world_mut());
    let sent = app
        .world_mut()
        .resource_mut::<crate::server_app::SimOutbox>()
        .drain();
    for (token, expected) in [("old", vec![]), ("new", vec!["detached".to_string()])] {
        let live = sent
            .iter()
            .find(|entry| entry.target == Target::Token(token.into()))
            .unwrap();
        assert_eq!(ids(&live.message), expected);
        let cache = app.world().resource::<LastBroadcastHull>().0.clone();
        let reconnect =
            crate::core::broadcast::reconnect_registered_replication(app.world_mut(), token);
        let reconnected = reconnect
            .iter()
            .find(|message| matches!(message, ServerMessage::SystemHullUpdate { .. }))
            .unwrap();
        assert_eq!(
            serde_json::to_value(&live.message).unwrap(),
            serde_json::to_value(reconnected).unwrap()
        );
        assert_eq!(app.world().resource::<LastBroadcastHull>().0, cache);
    }
}

#[test]
fn detached_repair_queue_detail_is_withheld_until_a_team_arrives() {
    let config = config();
    let mut teams = RepairTeams::new(1);
    let raw = RepairBlackboard {
        queue_depth: vec![
            QueueEntryPreview {
                group: RepairGroup::Core,
                station_label: "Core".into(),
                tier: DamageTier::Damaged,
                deficit: 10.0,
            },
            QueueEntryPreview {
                group: RepairGroup::Console(ConsoleId("visitor".into())),
                station_label: "Visitor".into(),
                tier: DamageTier::Damaged,
                deficit: 20.0,
            },
        ],
        ..Default::default()
    };
    let repair = ConsoleId("repair-screen".into());
    let before = HullVisibility::from_parts(&hull(), &config, Some(&teams))
        .project_repair_for_console(&config, &repair, &raw);
    assert_eq!(before.queue_depth.len(), 1);
    assert_eq!(before.queue_depth[0].group, RepairGroup::Core);
    teams.restore_slots(&[crate::core::messages::TeamSlot::Repairing {
        system_id: Some(SystemId("detached".into())),
        display_name: Some("Detached".into()),
        priority: None,
        priority_system_id: None,
    }]);
    let after = HullVisibility::from_parts(&hull(), &config, Some(&teams))
        .project_repair_for_console(&config, &repair, &raw);
    assert_eq!(after.queue_depth, raw.queue_depth);
}
