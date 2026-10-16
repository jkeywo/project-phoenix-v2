use super::*;
use crate::entities::spawner::EntitySystemHull;
use crate::server_app::ShipSystemBlackboards;
use crate::ship::damage::SystemHull;

fn board(app: &App, entity: Entity) -> RepairBlackboard {
    let SystemBlackboard::Repair(board) =
        &app.world().get::<ShipSystemBlackboards>(entity).unwrap().0[&repair_system_id()]
    else {
        panic!("published Repair board")
    };
    board.clone()
}

#[test]
fn cached_groups_and_empty_teams_preserve_live_publications_after_config_and_hull_changes() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap()
    .ship_config
    .unwrap();
    let systems: Vec<_> = config
        .systems
        .iter()
        .map(|system| (system.id.clone(), 100.0))
        .collect();
    let mut app = App::new();
    app.add_systems(Update, publish_repair_blackboard);
    let first = app
        .world_mut()
        .spawn((
            ShipConfigComponent(config.clone()),
            EntitySystemHull(SystemHull::from_config(&systems)),
            ShipSystemBlackboards::default(),
        ))
        .id();
    let second = app
        .world_mut()
        .spawn((
            ShipConfigComponent(config.clone()),
            EntitySystemHull(SystemHull::from_config(&systems)),
            ShipSystemBlackboards::default(),
        ))
        .id();
    app.update();
    let initial = board(&app, first);
    assert!(initial.teams.is_empty());
    assert!(initial.jobs.is_empty());
    assert!(!initial.repair_groups.is_empty());
    assert_eq!(
        serde_json::to_value(&initial).unwrap(),
        serde_json::to_value(board(&app, second)).unwrap()
    );
    app.update();
    assert_eq!(
        serde_json::to_value(&initial).unwrap(),
        serde_json::to_value(board(&app, first)).unwrap()
    );

    // Refits/restores replace or mutate this same accepted config component.
    // Group membership and labels must refresh with unchanged System identities.
    {
        let mut live = app
            .world_mut()
            .get_mut::<ShipConfigComponent>(first)
            .unwrap();
        let console = live
            .0
            .consoles
            .iter_mut()
            .find(|console| console.home_station.is_some())
            .unwrap();
        console.home_station = None;
        console.name.push_str(" changed");
    }
    app.update();
    let changed = board(&app, first);
    let live_config = &app.world().get::<ShipConfigComponent>(first).unwrap().0;
    assert_eq!(
        changed.repair_groups,
        repair_group_definitions(live_config, &changed.damageable_systems)
    );
    assert_ne!(changed.repair_groups, initial.repair_groups);
    assert_eq!(
        serde_json::to_value(&initial).unwrap(),
        serde_json::to_value(board(&app, second)).unwrap()
    );

    // Legacy HP membership can change independently of the config component.
    app.world_mut()
        .entity_mut(first)
        .insert(EntitySystemHull(SystemHull::from_config(&systems[1..])));
    app.update();
    let reduced = board(&app, first);
    assert_eq!(reduced.damageable_systems.len(), systems.len() - 1);
    let live_config = &app.world().get::<ShipConfigComponent>(first).unwrap().0;
    assert_eq!(
        reduced.repair_groups,
        repair_group_definitions(live_config, &reduced.damageable_systems)
    );

    // Losing config clears static groups; reinstallation cannot retain stale labels.
    app.world_mut()
        .entity_mut(first)
        .remove::<ShipConfigComponent>();
    app.update();
    assert!(board(&app, first).repair_groups.is_empty());
    app.world_mut()
        .entity_mut(first)
        .insert(ShipConfigComponent(config));
    app.update();
    let restored = board(&app, first);
    let live_config = &app.world().get::<ShipConfigComponent>(first).unwrap().0;
    assert_eq!(
        restored.repair_groups,
        repair_group_definitions(live_config, &restored.damageable_systems)
    );
    assert!(app.world_mut().despawn(first));
    app.update();
    assert_eq!(
        serde_json::to_value(&initial).unwrap(),
        serde_json::to_value(board(&app, second)).unwrap()
    );
}
