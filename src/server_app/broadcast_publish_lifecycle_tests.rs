use super::*;
use crate::console::repair::visibility::{self, LastVisibleRepairBlackboard};
use crate::core::broadcast::{
    reconnect_registered_replication, reset_registered_replication, ReplicationLifecycleRegistry,
};
use crate::core::messages::{HelmBlackboard, RepairBlackboard, SystemBlackboard, SystemId};
use crate::entities::spawner::EntitySystemHull;
use crate::lobby::{Sessions, Target};
use crate::modifiers::repair_teams::RepairTeams;
use crate::ship::config::ShipConfig;
use crate::ship::damage::SystemHull;
use crate::ship_plugin::ShipConfigComponent;

fn helm_blackboard() -> SystemBlackboard {
    SystemBlackboard::Helm(HelmBlackboard {
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

[[system]]
id = "helm-radar"
kind = "helm_radar"
station = "helm"

[[system]]
id = "repair"
kind = "repair"
station = "engineering"
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
        .insert(SystemId("z-shared".into()), helm_blackboard());

    let mut app = App::new();
    app.insert_resource(Sessions(sessions));
    register_blackboard_replication_lifecycle(&mut app);
    app.world_mut().spawn((
        LocalShip,
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
    let mut query = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let mut current: Vec<_> = query
        .single(app.world())
        .expect("LocalShip Blackboards")
        .0
        .iter()
        .map(|(id, blackboard)| (id.clone(), blackboard.clone()))
        .collect();
    current.sort_by(|a, b| a.0.cmp(&b.0));

    let hull_visibility = visibility::hull_visibility(app.world_mut());
    let viewers = visibility::connected_viewers(app.world());
    let mut last = LastVisibleRepairBlackboard::default();
    let pending = visibility::project_repair_blackboards(
        current,
        hull_visibility.as_ref(),
        &viewers,
        &mut last,
    );

    let mut visible = Vec::new();
    for (target, message) in pending {
        let for_recipient = matches!(target, Target::All)
            || matches!(&target, Target::Token(target_token) if target_token == token);
        if !for_recipient {
            continue;
        }
        if let ServerMessage::BlackboardUpdate { updates, .. } = message {
            visible.extend(updates);
        }
    }
    visible.sort_by(|a, b| a.0.cmp(&b.0));
    visible
}

fn reconnect_updates_for_token(app: &mut App, token: &str) -> Vec<(SystemId, SystemBlackboard)> {
    reconnect_registered_replication(app.world_mut(), token)
        .into_iter()
        .flat_map(|message| match message {
            ServerMessage::BlackboardUpdate { updates, .. } => updates,
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn blackboard_owner_registers_reset_for_both_live_projection_caches() {
    let mut app = blackboard_lifecycle_app();
    app.world_mut()
        .resource_mut::<LastBroadcastBlackboards>()
        .0
        .insert(SystemId("cached".into()), helm_blackboard());
    {
        let mut projected = app
            .world_mut()
            .resource_mut::<LastVisibleRepairBlackboard>();
        projected
            .projections
            .insert("eng".into(), RepairBlackboard::default());
        projected.stations.insert(
            "eng".into(),
            Some(crate::core::messages::StationId("engineering".into())),
        );
    }

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
        .resource::<LastBroadcastBlackboards>()
        .0
        .is_empty());
    let projected = app.world().resource::<LastVisibleRepairBlackboard>();
    assert!(projected.projections.is_empty());
    assert!(projected.stations.is_empty());
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
            vec!["repair", "z-shared"],
            "every current permitted Blackboard must be present in stable order"
        );
    }

    let pilot = reconnect_updates_for_token(&mut app, "pilot");
    let repair = pilot
        .iter()
        .find_map(|(_, blackboard)| match blackboard {
            SystemBlackboard::Repair(repair) => Some(repair),
            _ => None,
        })
        .expect("pilot receives the withholding Repair overwrite");
    assert!(repair.system_hull.is_empty());
    assert!(repair.damageable_systems.is_empty());
    assert!(repair.aggregate_hull_fraction.is_none());
    assert!(repair.destroyed_hull_fraction.is_none());
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
