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

#[test]
fn physical_publication_keeps_complete_status_without_using_legacy_hp_rows() {
    let recipe = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let config = recipe.ship_config.as_ref().unwrap().clone();
    let mut physical = crate::ship::compartment_damage::ShipDamage::from_config(&recipe)
        .unwrap()
        .unwrap();
    let damaged_room = physical
        .definition
        .components
        .keys()
        .find_map(|system| physical.definition.interior.placements.get(system))
        .unwrap()
        .clone();
    let room = physical.state.compartments.get_mut(&damaged_room).unwrap();
    room.hp *= 0.5;
    let expected: Vec<_> = config
        .systems
        .iter()
        .filter_map(|system| {
            super::super::visibility::physical_system_status(&physical, &system.id, &config)
        })
        .collect();
    assert!(!expected.is_empty());
    assert!(expected.iter().all(|status| {
        status.current == 0.0 && status.max_hp == 0.0 && status.physical.is_some()
    }));
    assert!(expected.iter().any(|status| {
        let physical = status.physical.as_ref().unwrap();
        physical.structural_hp < physical.structural_max_hp
    }));

    let mut app = App::new();
    app.add_systems(Update, publish_repair_blackboard);
    // This obsolete row deliberately disagrees with the physical machinery.
    // It must never become an extra HP pool or a repair-group member.
    let entity = app
        .world_mut()
        .spawn((
            ShipConfigComponent(config),
            physical,
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("obsolete-legacy-row".into()),
                9_999.0,
            )])),
            ShipSystemBlackboards::default(),
        ))
        .id();
    app.update();
    let initial = board(&app, entity);
    assert_eq!(initial.system_hull, expected);
    assert_eq!(
        initial.damageable_systems,
        expected
            .iter()
            .map(|status| status.system_id.clone())
            .collect::<Vec<_>>()
    );

    app.world_mut()
        .entity_mut(entity)
        .remove::<EntitySystemHull>();
    app.update();
    assert_eq!(
        serde_json::to_value(board(&app, entity)).unwrap(),
        serde_json::to_value(initial).unwrap(),
        "removing the unused HP projection must preserve the entire physical board"
    );

    // Physical rows still require their config. A sparse legacy owner without
    // it keeps the previous HP fallback even when a physical component exists.
    let fallback = SystemHull::from_config(&[(SystemId("legacy-fallback".into()), 75.0)]);
    app.world_mut()
        .entity_mut(entity)
        .remove::<ShipConfigComponent>()
        .insert(EntitySystemHull(fallback));
    app.update();
    let fallback = board(&app, entity);
    assert_eq!(fallback.system_hull.len(), 1);
    assert_eq!(
        fallback.system_hull[0].system_id,
        SystemId("legacy-fallback".into())
    );
    assert_eq!(fallback.system_hull[0].current, 75.0);
    assert_eq!(fallback.system_hull[0].max_hp, 75.0);
    assert!(fallback.system_hull[0].physical.is_none());
}

#[test]
fn legacy_publication_and_stable_queue_previews_keep_all_fields_without_mutating_requests() {
    use phoenix_model::observation::{ObservationMetadata, ObservationSource};
    let mut hull = SystemHull::from_config(&[
        (SystemId("legacy-first".into()), 100.0),
        (SystemId("legacy-second".into()), 50.0),
    ]);
    hull.set_hp(&SystemId("legacy-first".into()), 20.0);
    let expected_hull: Vec<_> = hull
        .iter()
        .map(|(id, entry)| SystemHullStatus {
            physical: None,
            system_id: id.clone(),
            display_name: entry.display_name.clone(),
            current: entry.current,
            max_hp: entry.max,
            tier: hull.tier_for(id),
            debuff_magnitude: hull.debuff_magnitude_for(id),
        })
        .collect();
    // Equal-severity rows deliberately reverse label order: publication uses
    // the existing stable severity sort, not the AI queue's tie-breaking rule.
    let entries: Vec<_> = [
        ("z-first-tie", DamageTier::Disabled, 30.0),
        ("a-second-tie", DamageTier::Disabled, 30.0),
        ("destroyed", DamageTier::Destroyed, 1.0),
        ("larger-deficit", DamageTier::Disabled, 60.0),
        ("operational", DamageTier::Operational, 100.0),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (label, tier, deficit))| {
        let mut observation = ObservationMetadata::default();
        observation.provenance.kind = ObservationSource::RepairTeam;
        observation.provenance.label = label.into();
        observation.observed_tick = Some(index as u64);
        observation.received_tick = Some(index as u64 + 10);
        RepairQueueEntry {
            observations: vec![observation.clone()],
            readings: vec![RepairSourceReading {
                source: observation,
                tier,
                deficit,
            }],
            group: RepairGroup::Console(phoenix_model::identity::ConsoleId(label.into())),
            station_label: label.into(),
            tier,
            deficit,
        }
    })
    .collect();
    let expected_queue: Vec<_> = [2, 3, 0, 1, 4]
        .into_iter()
        .map(|index| {
            let entry = &entries[index];
            QueueEntryPreview {
                observations: entry.observations.clone(),
                group: entry.group.clone(),
                station_label: entry.station_label.clone(),
                tier: entry.tier,
                deficit: entry.deficit,
            }
        })
        .collect();
    let mut app = App::new();
    app.add_systems(Update, publish_repair_blackboard);
    let entity = app
        .world_mut()
        .spawn((
            EntitySystemHull(hull),
            RepairRequestQueue {
                entries: entries.clone(),
                ..Default::default()
            },
            ShipSystemBlackboards::default(),
        ))
        .id();
    app.update();
    let initial = board(&app, entity);
    assert_eq!(initial.system_hull, expected_hull);
    assert_eq!(initial.queue_depth, expected_queue);
    assert_eq!(
        app.world()
            .get::<RepairRequestQueue>(entity)
            .unwrap()
            .entries,
        entries,
        "sorting borrowed rows must not reorder or consume the actual request queue"
    );
    app.update();
    assert_eq!(
        serde_json::to_value(board(&app, entity)).unwrap(),
        serde_json::to_value(initial).unwrap()
    );
}
