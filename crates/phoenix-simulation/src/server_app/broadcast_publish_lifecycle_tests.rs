use super::*;
use crate::core::broadcast::{
    reconnect_registered_replication, reset_registered_replication, ReplicationLifecycleRegistry,
};
use crate::core::messages::{HelmBlackboard, RepairBlackboard, SystemBlackboard, SystemId};
use crate::entities::spawner::EntitySystemHull;
use crate::lobby::{Sessions, Target};
use crate::modifiers::repair_teams::RepairTeams;
use crate::ship::config::ShipConfig;
use crate::ship::console_publication::LastConsolePublications;
use crate::ship::damage::SystemHull;
use crate::ship_plugin::ShipConfigComponent;

fn helm_blackboard() -> SystemBlackboard {
    SystemBlackboard::Helm(HelmBlackboard {
        movement: None,
        boost_battery: 1.0,
        boost_enabled: true,
        ..Default::default()
    })
}

fn blackboard_lifecycle_app() -> App {
    let config = ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = "Flying."
rank = "Ltn."

[[station]]
id = "engineering"
name = "Engineering"
description = "Fixing."
rank = "Ltn."

[[console]]
id = "helm-console"
home_station = "helm"
screen = "fixture.html"
[[console.feed]]
source = "helm"
kind = "Helm"

[[console]]
id = "engineering-console"
home_station = "engineering"
screen = "fixture.html"
[[console.feed]]
source = "helm"
kind = "Helm"

[[system]]
id = "helm-radar"
kind = "helm_radar"
console = "helm-console"

[[system]]
id = "repair"
kind = "repair"
console = "engineering-console"
"#,
        &["helm_radar", "repair"],
    )
    .expect("Blackboard lifecycle fixture must parse");

    let mut sessions = crate::lobby::session::SessionManager::new();
    sessions.register("eng".into(), "Engineer".into()).unwrap();
    sessions.register("pilot".into(), "Pilot".into()).unwrap();
    sessions.set_station(
        "eng",
        Some(crate::core::messages::StationId("engineering".into())),
    );
    sessions.set_station(
        "pilot",
        Some(crate::core::messages::StationId("helm".into())),
    );

    let mut blackboards = ShipSystemBlackboards::default();
    // The raw Repair payload deliberately contains detail. The projection
    // must keep only the detail each recipient can see.
    blackboards.0.insert(
        SystemId("repair".into()),
        SystemBlackboard::Repair(RepairBlackboard {
            aggregate_hull_fraction: Some(0.5),
            destroyed_hull_fraction: Some(0.25),
            damageable_systems: vec![
                SystemId("core".into()),
                SystemId("helm-radar".into()),
                SystemId("repair".into()),
            ],
            ..Default::default()
        }),
    );
    blackboards
        .0
        .insert(SystemId("helm".into()), helm_blackboard());

    let mut app = App::new();
    app.insert_resource(Sessions(sessions));
    app.init_resource::<SimOutbox>();
    register_blackboard_replication_lifecycle(&mut app);
    app.world_mut().spawn((
        LocalShip,
        crate::ship::components::ConsoleHosts(
            config
                .consoles
                .iter()
                .map(|console| phoenix_model::console::ConsoleHostSnapshot {
                    console: console.id.clone(),
                    host: console.home_station.clone(),
                    rating: String::new(),
                })
                .collect(),
        ),
        ShipConfigComponent(config),
        EntitySystemHull(SystemHull::from_config(&[
            (SystemId("core".into()), 100.0),
            (SystemId("helm-radar".into()), 100.0),
            (SystemId("repair".into()), 100.0),
        ])),
        crate::console::repair::server::ShipRepairTeams(RepairTeams::new(1)),
        blackboards,
    ));
    app
}

fn live_updates_for_token(app: &mut App, token: &str) -> Vec<(SystemId, SystemBlackboard)> {
    app.world_mut()
        .resource_mut::<LastConsolePublications>()
        .0
        .remove(token);
    crate::ship::console_publication::publish_console_blackboards(app.world_mut(), Some(0));
    app.world_mut()
        .resource_mut::<SimOutbox>()
        .drain()
        .into_iter()
        .filter(|entry| entry.target == Target::Token(token.into()))
        .flat_map(|entry| match entry.message {
            ServerMessage::BlackboardUpdate { updates, .. } => updates
                .into_iter()
                .map(|(id, board)| (id, board.as_ref().clone()))
                .collect(),
            _ => vec![],
        })
        .collect()
}

fn reconnect_updates_for_token(app: &mut App, token: &str) -> Vec<(SystemId, SystemBlackboard)> {
    reconnect_registered_replication(app.world_mut(), token)
        .into_iter()
        .flat_map(|message| match message {
            ServerMessage::BlackboardUpdate { updates, .. } => updates
                .into_iter()
                .map(|(id, board)| (id, board.as_ref().clone()))
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn blackboard_owner_resets_the_actual_recipient_projection_cache() {
    let mut app = blackboard_lifecycle_app();
    assert!(!live_updates_for_token(&mut app, "eng").is_empty());
    assert!(!app
        .world()
        .resource::<LastConsolePublications>()
        .0
        .is_empty());
    assert_eq!(
        app.world()
            .resource::<ReplicationLifecycleRegistry>()
            .keys()
            .collect::<Vec<_>>(),
        vec![BLACKBOARD_REPLICATION_KEY]
    );
    reset_registered_replication(app.world_mut());
    assert!(app
        .world()
        .resource::<LastConsolePublications>()
        .0
        .is_empty());
}

#[test]
fn reconnect_projection_matches_live_visibility_for_each_recipient() {
    let mut app = blackboard_lifecycle_app();

    for token in ["eng", "pilot"] {
        let live = live_updates_for_token(&mut app, token);
        let reconnect = reconnect_updates_for_token(&mut app, token);
        assert_eq!(
            reconnect, live,
            "reconnect must reproduce the live Blackboard visibility for {token}"
        );
        assert_eq!(
            reconnect
                .iter()
                .map(|(id, _)| id.0.as_str())
                .collect::<Vec<_>>(),
            if token == "eng" {
                vec!["helm", "repair"]
            } else {
                vec!["helm"]
            },
            "every current permitted Blackboard must be present in stable order"
        );
    }

    let pilot = reconnect_updates_for_token(&mut app, "pilot");
    assert!(
        !pilot
            .iter()
            .any(|(_, board)| matches!(board, SystemBlackboard::Repair(_))),
        "a Console without a Repair binding must receive no Repair publication"
    );
}

#[test]
fn presentation_rebase_republishes_unchanged_recipient_boards_and_reconnect_stamp() {
    use crate::audio_lifecycle::RoomAudioLifecycle;
    let mut app = blackboard_lifecycle_app();
    app.init_resource::<SimOutbox>()
        .init_resource::<RoomAudioLifecycle>()
        .add_systems(Update, broadcast_blackboard_updates);
    app.world_mut()
        .resource_mut::<RoomAudioLifecycle>()
        .state
        .generation = 7;
    app.update();
    let first = app.world_mut().resource_mut::<SimOutbox>().drain();
    assert_eq!(first.len(), 3);
    app.update();
    assert!(app
        .world_mut()
        .resource_mut::<SimOutbox>()
        .drain()
        .is_empty());
    app.world_mut()
        .resource_mut::<RoomAudioLifecycle>()
        .state
        .generation = 8;
    app.update();
    let rebased = app.world_mut().resource_mut::<SimOutbox>().drain();
    assert_eq!(rebased.len(), first.len());
    for (old, new) in first.iter().zip(&rebased) {
        assert_eq!(old.target, new.target);
        let ServerMessage::BlackboardUpdate {
            updates: old_rows, ..
        } = &old.message
        else {
            panic!("old-board")
        };
        let ServerMessage::BlackboardUpdate {
            updates: new_rows,
            presentation_generation,
            ..
        } = &new.message
        else {
            panic!("new-board")
        };
        assert_eq!(old_rows, new_rows);
        assert_eq!(*presentation_generation, Some(8));
    }
    for message in reconnect_registered_replication(app.world_mut(), "eng") {
        if let ServerMessage::BlackboardUpdate {
            presentation_generation,
            ..
        } = message
        {
            assert_eq!(presentation_generation, Some(8));
        }
    }
}

#[test]
fn reconnect_missing_config_or_hosts_never_falls_back_to_global_boards() {
    let mut app = blackboard_lifecycle_app();
    let world = app.world_mut();
    let entity = world
        .query_filtered::<Entity, With<LocalShip>>()
        .single(world)
        .unwrap();
    world
        .entity_mut(entity)
        .remove::<crate::ship::components::ConsoleHosts>();
    let packets = reconnect_registered_replication(app.world_mut(), "eng");
    assert_eq!(packets.len(), 1);
    let ServerMessage::BlackboardUpdate {
        console_scope,
        updates,
        ..
    } = &packets[0]
    else {
        panic!("scope revoke")
    };
    assert_eq!(console_scope, &Some(vec![]));
    assert!(updates.is_empty());
    app.world_mut()
        .entity_mut(entity)
        .remove::<ShipConfigComponent>();
    assert!(reconnect_registered_replication(app.world_mut(), "eng").is_empty());
}

#[test]
fn privileged_local_reconnect_matches_live_raw_publication() {
    let mut app = blackboard_lifecycle_app();
    let token = crate::console_bridge::LOCAL_CONSOLE_TOKEN;
    let live = live_updates_for_token(&mut app, token);
    assert_eq!(reconnect_updates_for_token(&mut app, token), live);
    assert_eq!(live.len(), 2);
    let repair = live
        .iter()
        .find_map(|(_, board)| {
            if let SystemBlackboard::Repair(repair) = board {
                Some(repair)
            } else {
                None
            }
        })
        .expect("raw Repair");
    assert_eq!(repair.aggregate_hull_fraction, Some(0.5));
}
