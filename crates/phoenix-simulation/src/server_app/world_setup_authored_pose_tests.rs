use super::*;
use crate::entities::spawner::EntityUuid;
use crate::ship::state::ShipPhysics;
use crate::world::config::{TransformConfig, WorldConfig, WorldEntity, WorldEntitySpawnOn};

fn row(name: Option<&str>, template: &str, rotation: Option<f32>) -> WorldEntity {
    WorldEntity {
        template_path: template.into(),
        name: name.map(str::to_owned),
        transform: Some(TransformConfig {
            position: Some([12.0, 0.0, -45.0]),
            rotation: rotation.map(|yaw| [0.0, yaw, 0.0]),
            scale: Some([1.5, 2.0, 0.75]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn assert_pose(world: &World, entity: Entity, yaw: f32, physical: bool) {
    let transform = world.get::<Transform>(entity).unwrap();
    assert_eq!(transform.translation, Vec3::new(12.0, 0.0, -45.0));
    assert_eq!(transform.scale, Vec3::new(1.5, 2.0, 0.75));
    assert!(transform
        .rotation
        .abs_diff_eq(Quat::from_rotation_y(yaw), 1e-6));
    if physical {
        let physics = world.get::<ShipPhysics>(entity).unwrap();
        assert_eq!(physics.yaw, -yaw);
        // The sole pose synchronizer's next projection retains this heading.
        let canonical = Quat::from_euler(EulerRot::YXZ, -physics.yaw, 0.0, physics.roll);
        assert!(canonical.abs_diff_eq(transform.rotation, 1e-6));
        assert_eq!((physics.x, physics.z), (12.0, -45.0));
    } else {
        assert!(world.get::<ShipPhysics>(entity).is_none());
    }
}

#[test]
fn immediate_named_and_anonymous_world_rows_seed_existing_physical_yaw() {
    let ship = "assets/entities/alliance_cruiser.toml";
    let scenery = "assets/entities/nav_beacon.toml";
    let mut config = WorldConfig {
        entities: vec![
            row(Some("named"), ship, Some(0.7)),
            row(None, ship, Some(-0.9)),
            row(Some("default"), ship, None),
            row(None, scenery, Some(0.4)),
        ],
        ..Default::default()
    };
    config
        .name_to_uuid
        .insert("named".into(), "named-id".into());
    config
        .name_to_uuid
        .insert("default".into(), "default-id".into());
    let cache =
        crate::entities::config_cache::ConfigCache::from(std::collections::HashMap::from([
            (
                ship.into(),
                crate::entities::include_resolve::load_entity_config(ship).unwrap(),
            ),
            (
                scenery.into(),
                crate::entities::include_resolve::load_entity_config(scenery).unwrap(),
            ),
        ]));
    let mut app = App::new();
    let mut public_world = WorldResource::default();
    let named = {
        let mut commands = app.world_mut().commands();
        let named = crate::world::server::spawn_immediate_entities_internal(
            &mut commands,
            &config,
            &cache,
            None,
            None,
        );
        assert_eq!(
            spawn_anonymous_entities_internal(
                &mut commands,
                &mut public_world,
                &config,
                &cache,
                None
            ),
            2
        );
        named
    };
    app.world_mut().flush();
    assert_eq!(named.len(), 2);
    assert_pose(app.world(), named[0], 0.7, true);
    assert_pose(app.world(), named[1], 0.0, true);
    let mut query = app.world_mut().query::<(Entity, &Transform)>();
    let anonymous: Vec<_> = query
        .iter(app.world())
        .filter(|(e, _)| !named.contains(e))
        .map(|(e, _)| e)
        .collect();
    assert_eq!(anonymous.len(), 2);
    for entity in anonymous {
        if app.world().get::<ShipPhysics>(entity).is_some() {
            assert_pose(app.world(), entity, -0.9, true);
        } else {
            assert_pose(app.world(), entity, 0.4, false);
        }
    }
}

#[test]
fn game_start_nonplayer_world_row_keeps_authored_yaw_without_changing_player_seed() {
    let ship = "assets/entities/alliance_cruiser.toml";
    let mut entities = vec![
        row(Some("player"), ship, Some(0.2)),
        row(Some("foe"), ship, Some(0.7)),
        row(Some("default"), ship, None),
        row(
            Some("static"),
            "assets/entities/nav_beacon.toml",
            Some(-0.4),
        ),
    ];
    for entity in &mut entities {
        entity.spawn_on = WorldEntitySpawnOn::GameStart;
    }
    let mut config = WorldConfig {
        entities,
        ..Default::default()
    };
    for name in ["player", "foe", "default", "static"] {
        config.name_to_uuid.insert(name.into(), name.into());
    }
    let mut app = App::new();
    app.insert_resource(config);
    app.add_systems(Update, spawn_game_start_entities);
    app.update();
    let mut query = app.world_mut().query::<(Entity, &EntityUuid)>();
    let rows: std::collections::HashMap<_, _> = query
        .iter(app.world())
        .map(|(entity, id)| (id.0.clone(), entity))
        .collect();
    assert_eq!(rows.len(), 4);
    assert_pose(app.world(), rows["foe"], 0.7, true);
    assert_pose(app.world(), rows["default"], 0.0, true);
    assert_pose(app.world(), rows["static"], -0.4, false);
    let player = rows["player"];
    assert!(app.world().get::<LocalShip>(player).is_some());
    // Existing fleet seed convention remains outside this non-player fix.
    assert!((app.world().get::<ShipPhysics>(player).unwrap().yaw - 0.2).abs() < 1e-6);
}
