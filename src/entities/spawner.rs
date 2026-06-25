use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::entity_config::EntityConfig;
use crate::entity_config::{AsteroidFieldConfig, LightConfig, StarConfig};
use crate::region_effects::RegionEffectKind;
use crate::region_shape::RegionShape;

// â”€â”€ Marker Components â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Every entity spawned by the generic spawner carries a UUID.
#[derive(Component, Clone, Debug)]
pub struct EntityUuid(pub String);

/// Optional human-readable identifier for the entity instance.
#[derive(Component, Clone, Debug)]
pub struct EntityId(pub String);

/// Display name from the top-level `name = "..."` scalar in the entity TOML.
/// Used by the renderer for HUD labels and by triggers/comms for named instances.
#[derive(Component, Clone, Debug)]
pub struct EntityName(pub String);

/// Present when the EntityConfig had one or more `[[light]]` entries.
/// The renderer reads this component to spawn `PointLight` / `DirectionalLight`
/// components (either on the entity itself or as children for multi-light setups).
#[derive(Component, Clone, Debug)]
pub struct Lights(pub Vec<LightConfig>);

/// Present when the EntityConfig had a [asteroid_field] section.
#[derive(Component, Clone, Debug)]
pub struct AsteroidFieldSection(pub AsteroidFieldConfig);

/// Present when the EntityConfig had a [collider] section.
#[derive(Component, Clone, Debug)]
pub struct ColliderSection(pub crate::entity_config::ColliderConfig);

/// Present when the EntityConfig had an [appearance] section.
#[derive(Component, Clone, Debug)]
pub struct AppearanceSection(pub crate::entity_config::AppearanceConfig);

/// Present when the EntityConfig has a [mesh] section.
/// Drives all 3-D viewscreen rendering â€” the renderer creates a Bevy mesh and
/// material from this data.
#[derive(Component, Clone, Debug)]
pub struct MeshSection(pub crate::entity_config::MeshConfig);

/// Present when the EntityConfig has a [star] section.
#[derive(Component, Clone, Debug)]
pub struct StarSection(pub StarConfig);

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

/// Present when the EntityConfig has a `[helm_console]` section.
/// The AI tick reads this to build a `ShipPhysicsConfig` instead of using hardcoded defaults.
#[derive(Component, Clone, Debug)]
pub struct HelmConsoleSection(pub crate::entity_config::HelmConsoleConfig);

/// Present when the EntityConfig had a [radar_appearance] section.
#[derive(Component, Clone, Debug)]
pub struct RadarAppearanceSection(pub crate::entity_config::RadarAppearanceConfig);

/// Present when the EntityConfig has a `[target]` section.
/// Carries targetability tags, threat level, and description.
#[derive(Component, Clone, Debug)]
pub struct EntityTarget(pub crate::entity_target::TargetSection);

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

/// Single-facing NPC shield component (#471).
///
/// Attached when the entity TOML has a top-level `[shields]` block (see
/// `EntityShieldConfig`). The damage pipeline routes incoming shots
/// through `split_damage_for_pierce`: the absorbed portion lands here,
/// pierced portion bypasses straight to hull. Regen ticks each frame
/// while `!broken && current_hp < max_hp`.
///
/// **Permanent break semantics:** once `current_hp` reaches `0.0`, the
/// shield is latched broken (`broken = true`) and never recovers - all
/// subsequent damage skips the shield routing and goes straight to hull
/// regardless of the attacker's `shield_pierce`. Regen is suppressed
/// while broken.
///
/// Distinct from the player ship's four-quadrant `ShipShields` resource,
/// which carries a multi-facing `ShieldSystem` with offline timers.
#[derive(Component, Clone, Debug)]
pub struct EntityShield {
    pub current_hp: f32,
    pub max_hp: f32,
    pub regen_per_sec: f32,
    pub broken: bool,
}

impl EntityShield {
    /// Build a fresh shield from its config: full HP, not broken.
    pub fn from_config(cfg: &crate::entity_config::EntityShieldConfig) -> Self {
        Self {
            current_hp: cfg.max_hp,
            max_hp: cfg.max_hp,
            regen_per_sec: cfg.regen_per_sec,
            broken: false,
        }
    }

    /// Apply `absorbed` damage to the shield, returning the leak that
    /// overflows to hull. If the shield depletes during this hit, the
    /// `broken` latch is set; subsequent calls return the full `absorbed`
    /// amount as leak (the shield will not regen back).
    pub fn apply_damage(&mut self, absorbed: f32) -> f32 {
        if self.broken {
            return absorbed;
        }
        if absorbed <= 0.0 {
            return 0.0;
        }
        if absorbed >= self.current_hp {
            let leak = absorbed - self.current_hp;
            self.current_hp = 0.0;
            self.broken = true;
            leak
        } else {
            self.current_hp -= absorbed;
            0.0
        }
    }

    /// Advance regen for `dt` seconds. No-op while broken or already at max.
    pub fn tick_regen(&mut self, dt: f32) {
        if self.broken || self.current_hp >= self.max_hp {
            return;
        }
        self.current_hp = (self.current_hp + self.regen_per_sec * dt).min(self.max_hp);
    }

    /// Fraction (0.0..=1.0) of current vs max HP. Returns 0.0 when broken
    /// regardless of `current_hp` (broken shields read as zero on the wire).
    pub fn fraction(&self) -> f32 {
        if self.broken || self.max_hp <= 0.0 {
            0.0
        } else {
            (self.current_hp / self.max_hp).clamp(0.0, 1.0)
        }
    }
}

// â”€â”€ Spawner â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Spawn an entity from a resolved EntityConfig.
///
/// Walks each optional section and inserts a component if present.
/// No type dispatch â€” just checks Option::is_some for each field.
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

    // Collider section â†’ Rapier collider + rigid body
    if let Some(collider) = &config.collider {
        let rapier_collider = match collider.shape {
            crate::entity_config::ColliderShape::Ball => Collider::ball(collider.radius),
            crate::entity_config::ColliderShape::Capsule => {
                Collider::capsule_y(collider.length / 2.0, collider.radius)
            }
        };
        entity_commands.insert((
            rapier_collider,
            RigidBody::KinematicPositionBased,
            ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
            ColliderSection(collider.clone()),
        ));
    }

    // Appearance section
    if let Some(appearance) = &config.appearance {
        entity_commands.insert(AppearanceSection(appearance.clone()));
    }

    // Mesh section
    if let Some(mesh) = &config.mesh {
        entity_commands.insert(MeshSection(mesh.clone()));
    }

    // Star section
    if let Some(star) = &config.star {
        entity_commands.insert(StarSection(star.clone()));
    }

    // Top-level name scalar
    if let Some(name) = &config.name {
        entity_commands.insert(EntityName(name.clone()));
    }

    // Lights array â€” present when one or more [[light]] entries were declared.
    if !config.light.is_empty() {
        entity_commands.insert(Lights(config.light.clone()));
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

    // Behaviour section â€” signals to ai_plugin to attach an AiController
    if let Some(behaviour) = &config.behaviour {
        entity_commands.insert(BehaviourSection(behaviour.clone()));

        let ship_config = crate::ship_plugin::ShipConfigComponent::default();
        let mut resolver = crate::ship::control_source::ControlSourceResolver::new();
        for system in &ship_config.0.systems {
            resolver.set(system.id.clone(), crate::ship::control_source::ControlSource::Ai);
        }
        entity_commands.insert((
            crate::server_app::Ship,
            ship_config,
            crate::ship_plugin::ShipSystemControlSources(resolver),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
        ));
    }

    // Tags â€” mirror TOML tags onto the entity for snapshot builders.
    if !config.tags.is_empty() {
        entity_commands.insert(EntityTagsSection(config.tags.clone()));
    }

    // Radar appearance section
    if let Some(radar_appearance) = &config.radar_appearance {
        entity_commands.insert(RadarAppearanceSection(radar_appearance.clone()));
    }

    // Target section
    if let Some(target) = &config.target {
        entity_commands.insert(EntityTarget(target.clone()));
    }

    // Faction â€” attach a FactionComponent so the AI can read faction from ECS.
    if let Some(faction_uuid) = config.faction {
        entity_commands.insert(FactionComponent(faction_uuid));
    }

    // WeaponsConsole â€” attach a WeaponsConsoleSection so the AI can read weapons config from ECS.
    if let Some(wc) = &config.weapons_console {
        entity_commands.insert(WeaponsConsoleSection(wc.clone()));
    }

    // HelmConsole - attach a HelmConsoleSection so the AI tick can read movement params.
    if let Some(hc) = &config.helm_console {
        entity_commands.insert(HelmConsoleSection(hc.clone()));
    }

    // Comms range - attach CommsRange component when [comms] is present.
    if let Some(comms) = &config.comms {
        entity_commands.insert(crate::comms::CommsRange(comms.range));
    }

    // Shields (#471) - single-facing NPC shield. Attach BEFORE the hull
    // block: the hull block has an early-return for the empty-hull case,
    // so anything after it could be skipped. Placing the shield insert
    // here ensures `[shields]` is always honoured.
    if let Some(shields_cfg) = &config.shields {
        entity_commands.insert(EntityShield::from_config(shields_cfg));
    }

    // Hull â€” attach an EntityConsoleHull component if the config has hull data.
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
        } else if hull.hull_integrity > 0.0 {
            crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
                hull.hull_integrity,
            )])
        } else {
            // Empty hull section â€” skip.
            entity_commands.insert(EntityConsoleHull(crate::damage::ConsoleHull::from_config(
                &[],
            )));
            return entity_commands.id();
        };
        entity_commands.insert(EntityConsoleHull(console_hull));
    }

    entity_commands.id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::*;

    // ── EntityShield unit tests (#471) ──────────────────────────────────────

    #[test]
    fn entity_shield_from_config_full_hp_not_broken() {
        let cfg = EntityShieldConfig {
            max_hp: 60.0,
            regen_per_sec: 1.0,
        };
        let shield = EntityShield::from_config(&cfg);
        assert_eq!(shield.current_hp, 60.0);
        assert_eq!(shield.max_hp, 60.0);
        assert_eq!(shield.regen_per_sec, 1.0);
        assert!(!shield.broken);
    }

    #[test]
    fn entity_shield_apply_damage_partial_returns_zero_leak() {
        let mut shield = EntityShield {
            current_hp: 60.0,
            max_hp: 60.0,
            regen_per_sec: 1.0,
            broken: false,
        };
        let leak = shield.apply_damage(10.0);
        assert_eq!(leak, 0.0);
        assert_eq!(shield.current_hp, 50.0);
        assert!(!shield.broken);
    }

    #[test]
    fn entity_shield_apply_damage_overflow_breaks_and_returns_leak() {
        let mut shield = EntityShield {
            current_hp: 10.0,
            max_hp: 60.0,
            regen_per_sec: 1.0,
            broken: false,
        };
        let leak = shield.apply_damage(15.0);
        assert_eq!(leak, 5.0);
        assert_eq!(shield.current_hp, 0.0);
        assert!(shield.broken, "shield must latch broken once depleted");
    }

    #[test]
    fn entity_shield_apply_damage_when_broken_returns_full_amount_as_leak() {
        let mut shield = EntityShield {
            current_hp: 0.0,
            max_hp: 60.0,
            regen_per_sec: 1.0,
            broken: true,
        };
        let leak = shield.apply_damage(20.0);
        assert_eq!(leak, 20.0, "broken shields pass damage through unchanged");
        assert!(shield.broken);
    }

    #[test]
    fn entity_shield_tick_regen_advances_below_max() {
        let mut shield = EntityShield {
            current_hp: 30.0,
            max_hp: 60.0,
            regen_per_sec: 5.0,
            broken: false,
        };
        shield.tick_regen(1.0);
        assert_eq!(shield.current_hp, 35.0);
    }

    #[test]
    fn entity_shield_tick_regen_clamps_to_max() {
        let mut shield = EntityShield {
            current_hp: 58.0,
            max_hp: 60.0,
            regen_per_sec: 10.0,
            broken: false,
        };
        shield.tick_regen(1.0);
        assert_eq!(shield.current_hp, 60.0);
    }

    #[test]
    fn entity_shield_tick_regen_noop_when_broken() {
        let mut shield = EntityShield {
            current_hp: 0.0,
            max_hp: 60.0,
            regen_per_sec: 5.0,
            broken: true,
        };
        shield.tick_regen(10.0);
        assert_eq!(
            shield.current_hp, 0.0,
            "broken shields must never regen back"
        );
        assert!(shield.broken);
    }

    #[test]
    fn entity_shield_fraction_returns_zero_when_broken() {
        let shield = EntityShield {
            current_hp: 0.0,
            max_hp: 60.0,
            regen_per_sec: 0.0,
            broken: true,
        };
        assert_eq!(shield.fraction(), 0.0);
    }

    #[test]
    fn entity_shield_fraction_returns_ratio_when_intact() {
        let shield = EntityShield {
            current_hp: 30.0,
            max_hp: 60.0,
            regen_per_sec: 0.0,
            broken: false,
        };
        assert_eq!(shield.fraction(), 0.5);
    }

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
    fn spawn_entity_with_comms_inserts_comms_range_component() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            light: Vec::new(),
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: Some(crate::entity_config::CommsConfig { range: 8000.0 }),
            asteroid_field: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let range = world
            .get::<crate::comms::CommsRange>(spawned)
            .expect("CommsRange component should be inserted when [comms] is present");
        assert_eq!(range.0, 8000.0);
    }

    #[test]
    fn spawn_entity_without_comms_omits_comms_range_component() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            light: Vec::new(),
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        assert!(world.get::<crate::comms::CommsRange>(spawned).is_none());
    }

    #[test]
    fn spawn_entity_with_name_inserts_entity_name_component() {
        let mut app = test_app();
        let config = EntityConfig {
            name: Some("Sun".to_string()),
            light: Vec::new(),
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        let name_comp = world
            .get::<EntityName>(spawned)
            .expect("should have EntityName");
        assert_eq!(name_comp.0, "Sun");
    }

    #[test]
    fn spawn_entity_without_name_omits_entity_name_component() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            light: Vec::new(),
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        assert!(world.get::<EntityName>(spawned).is_none());
    }

    #[test]
    fn spawn_entity_with_collider_has_rapier_components() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            star: None,
            light: Vec::new(),
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
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        assert!(
            world.get::<ColliderSection>(spawned).is_some(),
            "should have ColliderSection"
        );
        assert!(
            world.get::<Collider>(spawned).is_some(),
            "should have Rapier Collider"
        );
        assert!(
            world.get::<RigidBody>(spawned).is_some(),
            "should have RigidBody"
        );
    }

    #[test]
    fn spawn_entity_with_lights_inserts_lights_component() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            light: vec![LightConfig {
                kind: LightKind::Point,
                colour: [1.0, 0.95, 0.85],
                intensity: 150000.0,
                range: Some(5000.0),
                face_player: false,
            }],
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        let lights = world.get::<Lights>(spawned).expect("should have Lights");
        assert_eq!(lights.0.len(), 1);
        assert_eq!(lights.0[0].kind, LightKind::Point);
        assert_eq!(lights.0[0].range, Some(5000.0));
    }

    #[test]
    fn spawn_entity_with_asteroid_field_section() {
        use crate::entity_config::AsteroidFieldConfig;
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            star: None,
            light: Vec::new(),
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
                shield_pierce: 0.0,
                shape: None,
                anchor: None,
                anchor_offset: [0.0, 0.0, 0.0],
                random_rotation: None,
            }),
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        let field = world
            .get::<AsteroidFieldSection>(spawned)
            .expect("should have AsteroidFieldSection");
        assert!((field.0.inner_radius - 100.0).abs() < 1e-6);
    }

    #[test]
    fn spawn_entity_with_appearance_section() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            star: None,
            light: Vec::new(),
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
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        let appearance = world
            .get::<AppearanceSection>(spawned)
            .expect("should have AppearanceSection");
        assert_eq!(appearance.0.colour, "#ff0000");
    }

    #[test]
    fn spawn_entity_with_id_carries_id_component() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(
            &mut app,
            &config,
            Vec3::ZERO,
            uuid,
            Some("player-ship".to_string()),
        );

        let world = app.world_mut();
        let id_comp = world
            .get::<EntityId>(spawned)
            .expect("should have EntityId");
        assert_eq!(id_comp.0, "player-ship");
    }

    #[test]
    fn spawn_entity_without_id_has_no_id_component() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        assert!(
            world.get::<EntityId>(spawned).is_none(),
            "should NOT have EntityId"
        );
    }

    #[test]
    fn spawn_entity_with_region_shape_and_effects() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            star: None,
            light: Vec::new(),
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
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::new(100.0, 0.0, 50.0), uuid, None);

        let world = app.world_mut();
        let shape_comp = world
            .get::<RegionShapeSection>(spawned)
            .expect("should have RegionShapeSection");
        assert_eq!(shape_comp.0, RegionShape::Sphere { radius: 150.0 });

        let effects_comp = world
            .get::<RegionEffectsSection>(spawned)
            .expect("should have RegionEffectsSection");
        assert_eq!(effects_comp.0.len(), 2);
        assert!(effects_comp
            .0
            .contains(&crate::region_effects::RegionEffectKind::CommsJam));
        assert!(effects_comp
            .0
            .contains(&crate::region_effects::RegionEffectKind::SensorBlind));
    }

    #[test]
    fn spawn_entity_with_shape_alone_has_no_effects_comp() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            star: None,
            light: Vec::new(),
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
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        assert!(
            world.get::<RegionShapeSection>(spawned).is_some(),
            "should have RegionShapeSection"
        );
        assert!(
            world.get::<RegionEffectsSection>(spawned).is_none(),
            "should NOT have RegionEffectsSection"
        );
    }

    #[test]
    fn spawn_entity_with_faction_uuid_has_faction_component() {
        let mut app = test_app();
        let faction_id = uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap();
        let config = EntityConfig {
            name: None,
            star: None,
            light: Vec::new(),
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
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let comp = world
            .get::<FactionComponent>(spawned)
            .expect("should have FactionComponent");
        assert_eq!(comp.0, faction_id);
    }

    #[test]
    fn spawn_entity_without_faction_has_no_faction_component() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        assert!(
            world.get::<FactionComponent>(spawned).is_none(),
            "should NOT have FactionComponent"
        );
    }

    #[test]
    fn spawn_entity_position_matches_input() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::new(42.0, 0.0, -7.0), uuid, None);

        let world = app.world_mut();
        let transform = world
            .get::<Transform>(spawned)
            .expect("should have Transform");
        assert_eq!(transform.translation.x, 42.0);
        assert_eq!(transform.translation.z, -7.0);
    }

    // â”€â”€ EntityConsoleHull component tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn spawn_entity_with_hull_integrity_attaches_captain_chair_slot() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            star: None,
            light: Vec::new(),
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
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let hull_comp = world
            .get::<EntityConsoleHull>(spawned)
            .expect("should have EntityConsoleHull when hull_integrity > 0");
        assert!(
            (hull_comp.0.total_max() - 60.0).abs() < 1e-6,
            "max HP should be 60"
        );
        assert!(
            (hull_comp.0.total_current() - 60.0).abs() < 1e-6,
            "current HP should start at 60"
        );
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

    // ── EntityShield spawner attachment tests (#471) ────────────────────────

    #[test]
    fn spawn_entity_with_shields_block_attaches_entity_shield() {
        let mut app = test_app();
        let toml = r#"
[hull]
hull_integrity = 60.0

[shields]
max_hp = 30.0
regen_per_sec = 1.5
"#;
        let config = EntityConfig::from_toml(toml).expect("toml must parse");
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let shield = app
            .world()
            .get::<EntityShield>(spawned)
            .expect("entity with [shields] block must have EntityShield component");
        assert_eq!(shield.max_hp, 30.0);
        assert_eq!(shield.current_hp, 30.0);
        assert_eq!(shield.regen_per_sec, 1.5);
        assert!(!shield.broken);
    }

    #[test]
    fn spawn_entity_without_shields_block_omits_entity_shield() {
        let mut app = test_app();
        let toml = r#"
[hull]
hull_integrity = 60.0
"#;
        let config = EntityConfig::from_toml(toml).expect("toml must parse");
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        assert!(
            app.world().get::<EntityShield>(spawned).is_none(),
            "entity without [shields] block must not have EntityShield"
        );
    }

    #[test]
    fn entity_shield_config_parses_with_default_regen() {
        let toml = r#"
[shields]
max_hp = 50.0
"#;
        let config = EntityConfig::from_toml(toml).expect("toml must parse");
        let shields = config.shields.expect("shields block must be present");
        assert_eq!(shields.max_hp, 50.0);
        assert_eq!(shields.regen_per_sec, 0.0, "regen defaults to 0");
    }

    #[test]
    fn hull_integrity_maps_to_captain_chair_slot() {
        // Stations and asteroids still use hull_integrity in TOML â€” must keep working.
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            star: None,
            light: Vec::new(),
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
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            shields: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let hull_comp = world
            .get::<EntityConsoleHull>(spawned)
            .expect("entity with hull_integrity should still get EntityConsoleHull");
        assert!((hull_comp.0.total_max() - 200.0).abs() < 1e-6);
        let entries = hull_comp.0.entries();
        assert_eq!(entries[0].0, crate::messages::Console::CaptainChair);
    }

    // -- Channel-3 NPC routing smoke test (#552) --------------------------------

    #[test]
    fn npc_channel3_coordination_is_consumed() {
        // Pure routing logic: when both sender and target are Ai-controlled,
        // route_coordination must return Consume (not Popup).
        use crate::ship::control_source::ControlSource;
        use crate::ship::coordination::{route_coordination, DeliverAction};
        assert_eq!(
            route_coordination(ControlSource::Ai, ControlSource::Ai),
            DeliverAction::Consume,
        );
    }
}