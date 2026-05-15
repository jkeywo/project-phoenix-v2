use bevy::prelude::*;
use crate::damage::HullIntegrity;
use crate::lobby::WorldResource;
use crate::simulation::{Ship, ShipHullIntegrity};

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_world_hardcoded);
    }
}

/// Fallback world setup with hardcoded values for development/testing.
/// Runs only when no map config was preloaded; config-based setup takes over otherwise.
fn setup_world_hardcoded(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    _world: ResMut<WorldResource>,
) {
    if crate::config_cache::get_map_config().is_some() {
        return;
    }

    // ── Starfield skybox ───────────────────────────────────────────────────
    // Procedural points: many small unlit white spheres at radius ~2000
    // around the origin. Cheap and works on WebGL2.
    let star_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        unlit: true,
        ..default()
    });
    let star_mesh = meshes.add(Sphere { radius: 1.0 });
    let star_count = 400u32;
    let radius = 2000.0_f32;
    for i in 0..star_count {
        // Deterministic pseudo-random unit vector via golden-spiral on a sphere.
        let frac = (i as f32 + 0.5) / star_count as f32;
        let phi = (1.0 - 2.0 * frac).acos();
        let theta = std::f32::consts::PI * (1.0 + 5_f32.sqrt()) * i as f32;
        let x = phi.sin() * theta.cos() * radius;
        let y = phi.sin() * theta.sin() * radius;
        let z = phi.cos() * radius;
        // Hash for size variation
        let h = ((i.wrapping_mul(2654435761)) ^ 0xDEADBEEF) % 100;
        let scale = 1.5 + (h as f32) / 25.0; // 1.5..5.5
        commands.spawn((
            Mesh3d(star_mesh.clone()),
            MeshMaterial3d(star_mat.clone()),
            Transform::from_xyz(x, y, z).with_scale(Vec3::splat(scale)),
        ));
    }

    // Spawn ship via the generic entity spawner using a hardcoded EntityConfig
    // (mirrors assets/entities/player_ship.toml's collider). This is the
    // no-MapConfig fallback path; the [[entity]]/spawn_game_start path is
    // preferred and runs whenever MapConfig is loaded.
    let ship_config = crate::entity_config::EntityConfig {
        tags: vec!["player".to_string(), "ship".to_string()],
        collider: Some(crate::entity_config::ColliderConfig {
            shape: crate::entity_config::ColliderShape::Capsule,
            radius: 6.0,
            length: 6.0,
        }),
        hull: Some(crate::entity_config::HullConfig { hull_integrity: 100.0 }),
        appearance: None,
        helm_console: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        power: None,
        science_console: None,
            sensors_console: None,
        shields_console: None,
        star: None,
        planet: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        station: None,
        faction: None,
        behaviour: None,
    };
    let ship_uuid = crate::entity_loader::assign_uuid();
    let ship_entity = crate::entity_spawner::spawn_entity(
        &mut commands, &ship_config, Vec3::ZERO, ship_uuid, Some("player-ship".to_string()),
    );
    commands.entity(ship_entity).insert(Ship);
    commands.insert_resource(ShipHullIntegrity(HullIntegrity::with_hp(100.0)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_plugin_is_a_plugin() {
        // Verify WorldPlugin implements Plugin (compile-time check).
        fn assert_plugin<T: Plugin>() {}
        assert_plugin::<WorldPlugin>();
    }
}
