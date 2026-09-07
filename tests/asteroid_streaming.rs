//! The native template cache is process-global, so cache-mutating streaming
//! regressions live in their own integration-test process rather than libtest.
#![cfg(not(target_arch = "wasm32"))]

use bevy::prelude::*;
use project_phoenix::asteroids::lifecycle::{
    AsteroidEntityMap, AsteroidLifecyclePlugin, AsteroidWindow,
};
use project_phoenix::entities::config::EntityConfig;
use project_phoenix::entities::config_cache::insert_native_config;
use project_phoenix::entities::spawner::{AsteroidFieldSection, EntitySystemHull, MeshSection};
use project_phoenix::lobby::WorldResource;
use project_phoenix::server_app::{
    Asteroid, LastBroadcastEntityHealth, LastBroadcastEntityPositions, SimOutbox,
};
use project_phoenix::ship::state::ShipPhysics;

const ROCK: &str = "streaming-test/gameplay.toml";
const COSMETIC: &str = "streaming-test/cosmetic.toml";

fn template(radius: f32, hp: f32) -> EntityConfig {
    EntityConfig::from_toml(&format!(
        r#"
tags = ["asteroid", "authored-test"]
[collider]
shape = "Ball"
radius = {radius}
length = 0.0
[hull]
hull_integrity = {hp}
[mesh]
shape = "sphere"
radius = {radius}
colour = [0.6, 0.5, 0.4]
[radar_appearance]
icon = "asteroid"
colour = [0.2, 0.3, 0.4]
size = {radius}
"#
    ))
    .unwrap()
}

fn move_and_stream(app: &mut App, ship: Entity, x: f32) {
    app.world_mut().get_mut::<ShipPhysics>(ship).unwrap().x = x;
    app.world_mut().run_schedule(FixedUpdate);
}

fn identities(app: &App) -> Vec<(String, Option<[f32; 3]>)> {
    app.world()
        .resource::<WorldResource>()
        .0
        .entities
        .iter()
        .map(|rock| (rock.uuid.clone(), rock.position))
        .collect()
}

fn assert_templates(app: &mut App, radius: f32, hp: f32, cosmetic_radius: f32) {
    let snapshots = &app.world().resource::<WorldResource>().0.entities;
    assert!(!snapshots.is_empty(), "the real streamer must create rocks");
    for rock in snapshots {
        assert_eq!(rock.radius, Some(radius));
        assert_eq!(rock.tags, ["asteroid", "authored-test"]);
        assert_eq!(rock.radar_icon.as_deref(), Some("asteroid"));
        assert_eq!(rock.colour, Some([0.2, 0.3, 0.4]));
        assert_eq!(rock.radar_size, Some(radius));
    }
    let mut rocks = app
        .world_mut()
        .query_filtered::<(&MeshSection, &EntitySystemHull), With<Asteroid>>();
    let mut gameplay_count = 0;
    for (mesh, hull) in rocks.iter(app.world()) {
        gameplay_count += 1;
        assert_eq!(mesh.0.radius, radius);
        assert_eq!(hull.0.total_current(), hp);
    }
    assert!(gameplay_count > 0);
    let mut cosmetics = app
        .world_mut()
        .query_filtered::<&MeshSection, Without<Asteroid>>();
    let mut cosmetic_count = 0;
    for mesh in cosmetics.iter(app.world()) {
        cosmetic_count += 1;
        assert_eq!(mesh.0.radius, cosmetic_radius);
    }
    assert!(
        cosmetic_count > 0,
        "both cosmetic layers exercise their lookup"
    );
    let window = app.world().resource::<AsteroidWindow>();
    assert!(window
        .cosmetic_upper_slots
        .iter()
        .flatten()
        .any(Option::is_some));
    assert!(window
        .cosmetic_lower_slots
        .iter()
        .flatten()
        .any(Option::is_some));
    assert_eq!(
        app.world().resource::<AsteroidEntityMap>().0.len(),
        gameplay_count
    );
}

#[test]
fn streaming_template_delivery_and_replacement_preserve_cell_identity_and_order() {
    let field = EntityConfig::from_toml(&format!(
        r#"
[asteroid_field]
inner_radius = 0.0
outer_radius = 10000.0
density = 0.0
shape = "torus"
asteroid_type_paths = ["{ROCK}"]
cosmetic_type_paths = ["{COSMETIC}"]
[asteroid_field.grid]
resolution = 10.0
fill_gameplay = 0.0
fill_cosmetic = 0.0
spawn_cells = 1
despawn_cells = 2
cosmetic_y_offset = 20.0
gameplay_y_variance = 0.0
"#
    ))
    .unwrap()
    .asteroid_field
    .unwrap();
    let mut app = App::new();
    app.init_resource::<WorldResource>()
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<LastBroadcastEntityHealth>()
        .add_plugins(AsteroidLifecyclePlugin);
    let ship = app
        .world_mut()
        .spawn((
            ShipPhysics::default(),
            project_phoenix::lockstep::FleetSlotOf(
                project_phoenix::command_admission::HostSlot::SOLO,
            ),
        ))
        .id();
    app.world_mut().spawn(AsteroidFieldSection(field));

    // Missing templates keep the existing fallback. A failed lookup must not
    // become a sticky cache entry that hides later preload delivery.
    move_and_stream(&mut app, ship, 0.0);
    let original = identities(&app);
    assert!(!original.is_empty());
    for rock in &app.world().resource::<WorldResource>().0.entities {
        assert_eq!(rock.radius, Some(2.0));
        assert_eq!(rock.tags, ["asteroid"]);
    }
    let mut meshes = app.world_mut().query::<&MeshSection>();
    assert_eq!(meshes.iter(app.world()).count(), 0);

    insert_native_config(ROCK.into(), template(4.0, 100.0));
    insert_native_config(COSMETIC.into(), template(8.0, 0.0));
    move_and_stream(&mut app, ship, 500.0);
    move_and_stream(&mut app, ship, 0.0);
    assert_eq!(
        identities(&app),
        original,
        "delivery must not change cell IDs, positions or spawn order"
    );
    assert_templates(&mut app, 4.0, 100.0, 8.0);

    // The template cache remains the source of truth on later spawns and on
    // the shared snapshot-restore rock_config path; no per-path memo survives.
    insert_native_config(ROCK.into(), template(6.0, 150.0));
    insert_native_config(COSMETIC.into(), template(12.0, 0.0));
    move_and_stream(&mut app, ship, 500.0);
    move_and_stream(&mut app, ship, 0.0);
    assert_eq!(identities(&app), original);
    assert_templates(&mut app, 6.0, 150.0, 12.0);
    let restored = project_phoenix::asteroids::lifecycle::rock_config(ROCK);
    assert_eq!(restored.collider.radius, 6.0);
    assert_eq!(restored.max_hp, 150.0);
}
