use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::entity_config::EntityConfig;
use crate::map_config::{StarConfig, PlanetConfig, AsteroidFieldConfig};
use crate::region_shape::RegionShape;
use crate::region_effects::RegionEffectKind;

// ── Marker Components ──────────────────────────────────────────────

/// Every entity spawned by the generic spawner carries a UUID.
#[derive(Component, Clone, Debug)]
pub struct EntityUuid(pub String);

/// Optional human-readable identifier for the entity instance.
#[derive(Component, Clone, Debug)]
pub struct EntityId(pub String);

/// Present when the EntityConfig had a [star] section.
#[derive(Component, Clone, Debug)]
pub struct StarSection(pub StarConfig);

/// Present when the EntityConfig had a [planet] section.
#[derive(Component, Clone, Debug)]
pub struct PlanetSection(pub PlanetConfig);

/// Present when the EntityConfig had a [asteroid_field] section.
#[derive(Component, Clone, Debug)]
pub struct AsteroidFieldSection(pub AsteroidFieldConfig);

/// Present when the EntityConfig had a [collider] section.
#[derive(Component, Clone, Debug)]
pub struct ColliderSection(pub crate::entity_config::ColliderConfig);

/// Present when the EntityConfig had an [appearance] section.
#[derive(Component, Clone, Debug)]
pub struct AppearanceSection(pub crate::entity_config::AppearanceConfig);

/// Present when the EntityConfig had a [shape] section (region entity).
#[derive(Component, Clone, Debug)]
pub struct RegionShapeSection(pub RegionShape);

/// Present when the EntityConfig had a [effects] section.
#[derive(Component, Clone, Debug)]
pub struct RegionEffectsSection(pub Vec<RegionEffectKind>);

/// Present when the EntityConfig had a [behaviour] section.
/// Carries the initial AI state name so `ai_plugin` can attach an `AiController`.
#[derive(Component, Clone, Debug)]
pub struct BehaviourSection(pub crate::entity_config::BehaviourConfig);

/// Present when the EntityConfig has a non-empty `tags` list.
/// Mirrors the TOML tags onto the ECS entity so snapshot builders can include them.
#[derive(Component, Clone, Debug)]
pub struct EntityTagsSection(pub Vec<String>);

/// Present when the EntityConfig has a `faction` UUID.
/// The AI tick reads this component to determine `self_faction` and enemy evaluation.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct FactionComponent(pub uuid::Uuid);

/// Present when the EntityConfig has a `[weapons_console]` section.
/// The AI tick reads this component to determine weapons range and phaser readiness.
#[derive(Component, Clone, Debug)]
pub struct WeaponsConsoleSection(pub crate::entity_config::WeaponsConsoleConfig);

/// Present when the EntityConfig had a [radar_appearance] section.
#[derive(Component, Clone, Debug)]
pub struct RadarAppearanceSection(pub crate::entity_config::RadarAppearanceConfig);

/// Hull tracker attached to any entity (NPC ship, asteroid) that carries a
/// `[hull]` section in its TOML config. For NPC ships the HP is placed in a
/// single `CaptainChair` console slot; asteroids use the same single-slot
/// convention. Damage systems query this component to deal damage and detect
/// destruction.
///
/// This is a Bevy ECS component wrapping the pure `ConsoleHull` struct, as
/// distinct from the `ShipHullIntegrity` resource used for the player ship.
#[derive(Component, Clone, Debug)]
pub struct EntityConsoleHull(pub crate::damage::ConsoleHull);

/// Present on entities spawned by a scenario, identifies which scenario owns them.
///
/// Entities spawned from a map (not a scenario) do not carry this component.
/// When the owning scenario is unloaded, this component is removed from the entity
/// and the entity continues to function as self-directed.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ScenarioOwner(pub String);

// ── Spawner ────────────────────────────────────────────────────────

/// Spawn an entity from a resolved EntityConfig.
///
/// Walks each optional section and inserts a component if present.
/// No type dispatch — just checks Option::is_some for each field.
///
/// Returns the spawned Entity. Callers must flush commands (e.g. via app.update())
/// before querying components on the returned entity.
pub fn spawn_entity(
    commands: &mut Commands,
    config: &EntityConfig,
    position: Vec3,
    uuid: String,
    id: Option<String>,
) -> Entity {
    let mut entity_commands = commands.spawn((
        Transform::from_translation(position),
        Visibility::default(),
        EntityUuid(uuid.clone()),
    ));

    // Insert optional human-readable ID
    if let Some(human_id) = id {
        entity_commands.insert(EntityId(human_id));
    }

    // Collider section → Rapier collider + rigid body
    if let Some(collider) = &config.collider {
        let rapier_collider = match collider.shape {
            crate::entity_config::ColliderShape::Ball => {
                Collider::ball(collider.radius)
            }
            crate::entity_config::ColliderShape::Capsule => {
                Collider::capsule_y(collider.length / 2.0, collider.radius)
            }
        };
        entity_commands.insert((
            rapier_collider,
            RigidBody::KinematicPositionBased,
            ActiveCollisionTypes::KINEMATIC_STATIC,
            ColliderSection(collider.clone()),
        ));
    }

    // Appearance section
    if let Some(appearance) = &config.appearance {
        entity_commands.insert(AppearanceSection(appearance.clone()));
    }

    // Star section
    if let Some(star) = &config.star {
        entity_commands.insert(StarSection(star.clone()));
    }

    // Planet section
    if let Some(planet) = &config.planet {
        entity_commands.insert(PlanetSection(planet.clone()));
    }

    // Asteroid field section
    if let Some(field) = &config.asteroid_field {
        entity_commands.insert(AsteroidFieldSection(field.clone()));
    }

    // Region shape section
    if let Some(shape) = &config.shape {
        entity_commands.insert(RegionShapeSection(shape.clone()));
    }

    // Region effects section
    if let Some(effects) = &config.effects {
        if !effects.is_empty() {
            entity_commands.insert(RegionEffectsSection(effects.to_kinds()));
        }
    }

    // Behaviour section — signals to ai_plugin to attach an AiController
    if let Some(behaviour) = &config.behaviour {
        entity_commands.insert(BehaviourSection(behaviour.clone()));
    }

    // Tags — mirror TOML tags onto the entity for snapshot builders.
    if !config.tags.is_empty() {
        entity_commands.insert(EntityTagsSection(config.tags.clone()));
    }

    // Radar appearance section
    if let Some(radar_appearance) = &config.radar_appearance {
        entity_commands.insert(RadarAppearanceSection(radar_appearance.clone()));
    }

    // Faction — attach a FactionComponent so the AI can read faction from ECS.
    if let Some(faction_uuid) = config.faction {
        entity_commands.insert(FactionComponent(faction_uuid));
    }

    // WeaponsConsole — attach a WeaponsConsoleSection so the AI can read weapons config from ECS.
    if let Some(wc) = &config.weapons_console {
        entity_commands.insert(WeaponsConsoleSection(wc.clone()));
    }

    // Hull — attach an EntityConsoleHull component if the config has hull data.
    // Per-console entries (console_hull) take precedence; if absent, the legacy
    // hull_integrity value is mapped to a single CaptainChair slot.
    if let Some(hull) = &config.hull {
        let console_hull = if !hull.console_hull.is_empty() {
            // Explicit per-console entries (player ship path).
            let entries: Vec<(crate::messages::Console, f32)> = hull
                .console_hull
                .iter()
                .map(|e| (e.console.clone(), e.max_hp))
                .collect();
            crate::damage::ConsoleHull::from_config(&entries)
        } else if let Some(hp) = hull.captain_chair {
            // Spec-required NPC ship path: `captain_chair = <n>` in TOML.
            crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
                hp,
            )])
        } else if hull.hull_integrity > 0.0 {
            // Legacy fallback: `hull_integrity = <n>` (stations, asteroids).
            crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
                hull.hull_integrity,
            )])
        } else {
            // Empty hull section — skip.
            entity_commands.insert(EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[])));
            return entity_commands.id();
        };
        entity_commands.insert(EntityConsoleHull(console_hull));
    }

    entity_commands.id()
}

/// Query for all entities with StarSection and ensure they have visual
/// meshes and materials. Runs as a startup system after entity spawning.
pub fn render_star_sections(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    stars: Query<(Entity, &StarSection, &Transform), Without<Mesh3d>>,
) {
    for (entity, star, _transform) in stars.iter() {
        let mesh = meshes.add(Sphere { radius: star.0.radius });
        let color = if star.0.colour.len() >= 3 {
            Color::srgb(star.0.colour[0], star.0.colour[1], star.0.colour[2])
        } else {
            Color::srgb(1.0, 1.0, 1.0)
        };
        let mat = materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * 2.0,
            ..default()
        });
        let light_color = star.0.light_colour.as_ref()
            .filter(|c| c.len() >= 3)
            .map(|c| Color::srgb(c[0], c[1], c[2]))
            .unwrap_or(Color::WHITE);
        let range = star.0.light_range.unwrap_or(star.0.radius * 60.0);
        let intensity = star.0.light_intensity.unwrap_or(star.0.radius * 2000.0);
        commands.entity(entity).insert((
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            PointLight {
                color: light_color,
                intensity,
                range,
                shadows_enabled: false,
                ..default()
            },
        ));
    }
}

/// Query for all entities with PlanetSection and ensure they have visual
/// meshes and materials.
pub fn render_planet_sections(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    planets: Query<(Entity, &PlanetSection, &Transform), Without<Mesh3d>>,
) {
    for (entity, planet, _transform) in planets.iter() {
        let mesh = meshes.add(Sphere { radius: planet.0.radius });
        let color = if planet.0.colour.len() >= 3 {
            Color::srgb(planet.0.colour[0], planet.0.colour[1], planet.0.colour[2])
        } else {
            Color::srgb(0.5, 0.5, 0.5)
        };
        let mat = materials.add(StandardMaterial {
            base_color: color,
            ..default()
        });
        commands.entity(entity).insert((
            Mesh3d(mesh),
            MeshMaterial3d(mat),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::*;
    use crate::map_config::StarConfig;

    /// Helper: build a minimal Bevy app for spawning tests.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app
    }

    /// Call `spawn_entity` then flush commands via app.update() so components
    /// are queryable.
    fn spawn_and_flush(
        app: &mut App,
        config: &EntityConfig,
        position: Vec3,
        uuid: String,
        id: Option<String>,
    ) -> Entity {
        let entity = {
            let mut commands = app.world_mut().commands();
            spawn_entity(&mut commands, config, position, uuid, id)
        };
        app.update();
        entity
    }

    #[test]
    fn spawn_entity_with_hull_and_star_has_both_components() {
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec!["test".to_string()],
            hull: Some(HullConfig { hull_integrity: 100.0, ..Default::default() }),
            star: Some(StarConfig {
                name: "TestStar".to_string(),
                radius: 10.0,
                colour: vec![1.0, 1.0, 1.0],
                position: vec![0.0, 0.0, 0.0],
                tags: vec![],
                light_range: None,
                light_intensity: None,
                light_colour: None,
            }),
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            science_console: None,
            sensors_console: None,
            shields_console: None,
            planet: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        assert!(world.get::<EntityUuid>(spawned).is_some(), "should have EntityUuid");
        assert!(world.get::<StarSection>(spawned).is_some(), "should have StarSection");
        assert_eq!(world.get::<StarSection>(spawned).unwrap().0.name, "TestStar");
    }

    #[test]
    fn spawn_entity_with_collider_has_rapier_components() {
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec![],
            collider: Some(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 3.0,
                length: 0.0,
            }),
            hull: None,
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
            radar_appearance: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        assert!(world.get::<ColliderSection>(spawned).is_some(), "should have ColliderSection");
        assert!(world.get::<Collider>(spawned).is_some(), "should have Rapier Collider");
        assert!(world.get::<RigidBody>(spawned).is_some(), "should have RigidBody");
    }

    #[test]
    fn spawn_entity_with_planet_section() {
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec![],
            planet: Some(PlanetConfig {
                name: "TestPlanet".to_string(),
                radius: 15.0,
                colour: vec![0.0, 0.5, 1.0],
                position: vec![100.0, 0.0, 100.0],
                tags: vec![],
            }),
            hull: None,
            collider: None,
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
            asteroid_field: None,
            shape: None,
            effects: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::new(100.0, 0.0, 100.0), uuid, None);

        let world = app.world_mut();
        let planet_section = world.get::<PlanetSection>(spawned).expect("should have PlanetSection");
        assert!((planet_section.0.radius - 15.0).abs() < 1e-6);
        assert_eq!(planet_section.0.name, "TestPlanet");
    }

    #[test]
    fn spawn_entity_with_asteroid_field_section() {
        use crate::map_config::AsteroidFieldConfig;
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec!["field".to_string()],
            asteroid_field: Some(AsteroidFieldConfig {
                inner_radius: 100.0,
                outer_radius: 200.0,
                density: 0.005,
                spawn_distance: 150.0,
                despawn_distance: 250.0,
                asteroid_type_paths: vec!["small.toml".to_string()],
                cosmetic_type_paths: vec![],
                tags: vec![],
                grid: None,
            }),
            hull: None,
            collider: None,
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
            shape: None,
            effects: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        let field = world.get::<AsteroidFieldSection>(spawned).expect("should have AsteroidFieldSection");
        assert!((field.0.inner_radius - 100.0).abs() < 1e-6);
    }

    #[test]
    fn spawn_entity_with_appearance_section() {
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec![],
            appearance: Some(AppearanceConfig {
                colour: "#ff0000".to_string(),
                size_min: 1.0,
                size_max: 3.0,
            }),
            hull: None,
            collider: None,
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
            radar_appearance: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        let appearance = world.get::<AppearanceSection>(spawned).expect("should have AppearanceSection");
        assert_eq!(appearance.0.colour, "#ff0000");
    }

    #[test]
    fn spawn_entity_with_id_carries_id_component() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, Some("player-ship".to_string()));

        let world = app.world_mut();
        let id_comp = world.get::<EntityId>(spawned).expect("should have EntityId");
        assert_eq!(id_comp.0, "player-ship");
    }

    #[test]
    fn spawn_entity_without_id_has_no_id_component() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        assert!(world.get::<EntityId>(spawned).is_none(), "should NOT have EntityId");
    }

    #[test]
    fn spawn_entity_with_region_shape_and_effects() {
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec!["region".to_string(), "nebula".to_string()],
            shape: Some(RegionShape::Sphere { radius: 150.0 }),
            effects: Some(crate::region_effects::RegionEffectsConfig {
                comms_jammed: Some(crate::region_effects::CommsJamEffect {}),
                sensor_blind: Some(crate::region_effects::SensorBlindEffect {}),
                ..Default::default()
            }),
            hull: None,
            collider: None,
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
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::new(100.0, 0.0, 50.0), uuid, None);

        let world = app.world_mut();
        let shape_comp = world.get::<RegionShapeSection>(spawned)
            .expect("should have RegionShapeSection");
        assert_eq!(shape_comp.0, RegionShape::Sphere { radius: 150.0 });

        let effects_comp = world.get::<RegionEffectsSection>(spawned)
            .expect("should have RegionEffectsSection");
        assert_eq!(effects_comp.0.len(), 2);
        assert!(effects_comp.0.contains(&crate::region_effects::RegionEffectKind::CommsJam));
        assert!(effects_comp.0.contains(&crate::region_effects::RegionEffectKind::SensorBlind));
    }

    #[test]
    fn spawn_entity_with_shape_alone_has_no_effects_comp() {
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius: 100.0 }),
            effects: None,
            hull: None,
            collider: None,
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
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        assert!(world.get::<RegionShapeSection>(spawned).is_some(), "should have RegionShapeSection");
        assert!(world.get::<RegionEffectsSection>(spawned).is_none(), "should NOT have RegionEffectsSection");
    }

    #[test]
    fn spawn_entity_with_faction_uuid_has_faction_component() {
        let mut app = test_app();
        let faction_id = uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap();
        let config = EntityConfig {
            tags: vec![],
            faction: Some(faction_id),
            hull: None,
            collider: None,
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
            behaviour: None,
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let comp = world.get::<FactionComponent>(spawned).expect("should have FactionComponent");
        assert_eq!(comp.0, faction_id);
    }

    #[test]
    fn spawn_entity_without_faction_has_no_faction_component() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        assert!(world.get::<FactionComponent>(spawned).is_none(), "should NOT have FactionComponent");
    }

    #[test]
    fn spawn_entity_position_matches_input() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::new(42.0, 0.0, -7.0), uuid, None);

        let world = app.world_mut();
        let transform = world.get::<Transform>(spawned).expect("should have Transform");
        assert_eq!(transform.translation.x, 42.0);
        assert_eq!(transform.translation.z, -7.0);
    }

    // ── ScenarioOwner component tests ───────────────────────────────────────

    // spawn_entity does not attach ScenarioOwner — caller's responsibility
    #[test]
    fn spawn_entity_does_not_attach_scenario_owner_by_default() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        assert!(
            app.world().get::<ScenarioOwner>(spawned).is_none(),
            "spawn_entity must not attach ScenarioOwner; caller inserts it"
        );
    }

    // Caller can insert ScenarioOwner on the returned entity
    #[test]
    fn caller_can_insert_scenario_owner_after_spawn() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = {
            let mut commands = app.world_mut().commands();
            let e = spawn_entity(&mut commands, &config, Vec3::ZERO, uuid, None);
            commands.entity(e).insert(ScenarioOwner("scenarios/phase1.toml".to_string()));
            e
        };
        app.update();
        let owner = app.world().get::<ScenarioOwner>(spawned).expect("should have ScenarioOwner");
        assert_eq!(owner.0, "scenarios/phase1.toml");
    }

    // ── EntityConsoleHull component tests ───────────────────────────────────

    #[test]
    fn spawn_entity_with_legacy_hull_integrity_attaches_captain_chair_slot() {
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec![],
            hull: Some(crate::entity_config::HullConfig {
                hull_integrity: 60.0,
                ..Default::default()
            }),
            collider: None,
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
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let hull_comp = world.get::<EntityConsoleHull>(spawned)
            .expect("should have EntityConsoleHull when hull_integrity > 0");
        assert!((hull_comp.0.total_max() - 60.0).abs() < 1e-6, "max HP should be 60");
        assert!((hull_comp.0.total_current() - 60.0).abs() < 1e-6, "current HP should start at 60");
    }

    #[test]
    fn spawn_entity_without_hull_has_no_entity_console_hull() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        assert!(
            world.get::<EntityConsoleHull>(spawned).is_none(),
            "entity with no hull config must not have EntityConsoleHull"
        );
    }

    #[test]
    fn npc_captain_chair_field_maps_to_captain_chair_console_slot() {
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec!["npc".to_string()],
            hull: Some(crate::entity_config::HullConfig {
                captain_chair: Some(60.0),
                ..Default::default()
            }),
            collider: None,
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
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let hull_comp = world.get::<EntityConsoleHull>(spawned)
            .expect("NPC with captain_chair should have EntityConsoleHull");
        let entries = hull_comp.0.entries();
        assert_eq!(entries.len(), 1, "NPC should have exactly one hull slot");
        assert_eq!(entries[0].0, crate::messages::Console::CaptainChair, "slot should be CaptainChair");
        assert!((entries[0].2 - 60.0).abs() < 1e-6, "max_hp should be 60");
        assert!((entries[0].1 - 60.0).abs() < 1e-6, "current should start at 60");
    }

    #[test]
    fn legacy_hull_integrity_still_maps_to_captain_chair_slot() {
        // Stations and asteroids still use hull_integrity in TOML — must keep working.
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec![],
            hull: Some(crate::entity_config::HullConfig {
                hull_integrity: 200.0,
                ..Default::default()
            }),
            collider: None,
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
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let hull_comp = world.get::<EntityConsoleHull>(spawned)
            .expect("entity with hull_integrity should still get EntityConsoleHull");
        assert!((hull_comp.0.total_max() - 200.0).abs() < 1e-6);
        let entries = hull_comp.0.entries();
        assert_eq!(entries[0].0, crate::messages::Console::CaptainChair);
    }

    #[test]
    fn captain_chair_takes_precedence_over_hull_integrity() {
        // If both are set, captain_chair wins.
        let mut app = test_app();
        let config = EntityConfig {
            tags: vec![],
            hull: Some(crate::entity_config::HullConfig {
                captain_chair: Some(80.0),
                hull_integrity: 999.0,
                ..Default::default()
            }),
            collider: None,
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
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let hull_comp = world.get::<EntityConsoleHull>(spawned).unwrap();
        assert!((hull_comp.0.total_max() - 80.0).abs() < 1e-6,
            "captain_chair should take precedence over hull_integrity");
    }
}
