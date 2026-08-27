use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::entities::config::EntityConfig;
use crate::entities::config::{AsteroidFieldConfig, LightConfig, StarConfig};
use crate::regions::effects::RegionEffectKind;
use crate::regions::shape::RegionShape;

// â”€â”€ Marker Components â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Every entity spawned by the generic spawner carries a UUID.
#[derive(Component, Clone, Debug)]
pub struct EntityUuid(pub String);

/// Every entity spawned by the generic spawner carries its authored mass
/// (issue #1154), in the game's own mass unit — [`EntityConfig::mass`]
/// verbatim, already defaulted at parse time, so this is NEVER absent and
/// NEVER zero. Unconditional like [`EntityUuid`] rather than optional like
/// [`EntityName`]: every entity has a weight, whether an author chose one or
/// not, so there is no "no mass" case for an `Option` to represent. Nothing
/// mutates this after spawn — it is content identity, not simulation state,
/// exactly as [`EntityUuid`] is.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct EntityMass(pub f32);

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
pub struct ColliderSection(pub crate::entities::config::ColliderConfig);

/// Present when the EntityConfig had an [appearance] section.
#[derive(Component, Clone, Debug)]
pub struct AppearanceSection(pub crate::entities::config::AppearanceConfig);

/// Present when the EntityConfig has a [mesh] section.
/// Drives all 3-D viewscreen rendering â€” the renderer creates a Bevy mesh and
/// material from this data.
#[derive(Component, Clone, Debug)]
pub struct MeshSection(pub crate::entities::config::MeshConfig);

/// Present when the EntityConfig has a [star] section.
#[derive(Component, Clone, Debug)]
pub struct StarSection(pub StarConfig);

/// Present when the EntityConfig has a [planet] section.
#[derive(Component, Clone, Debug)]
pub struct PlanetSection(pub crate::entities::config::PlanetConfig);

/// Present when the EntityConfig had a [shape] section (region entity).
#[derive(Component, Clone, Debug)]
pub struct RegionShapeSection(pub RegionShape);

/// Present when the EntityConfig had a [effects] section.
#[derive(Component, Clone, Debug)]
pub struct RegionEffectsSection(pub Vec<RegionEffectKind>);

/// Present when the EntityConfig had a [behaviour] section.
/// Carries the initial AI state name so `ai_plugin` can attach an `AiController`.
#[derive(Component, Clone, Debug)]
pub struct BehaviourSection(pub crate::entities::config::BehaviourConfig);

/// Marks an ownerless, stationary weapons platform. It uses the shared ship
/// combat substrate for its own target selection and beams. As of issue
/// #1011, a factioned `StaticPointDefence` entity IS acquirable by the
/// ordinary hostile scan (`ai_target_selection`'s `hostile_scan_q`, in
/// `src/console/weapons/mod.rs`, matches `Or<(With<Ship>, With<StaticPointDefence>)>`) —
/// an unfactioned one stays invisible only because the faction gate
/// (`is_hostile` / `faction::is_enemy`) requires a `FactionComponent` on
/// both sides.
#[derive(Component, Clone, Debug)]
pub struct StaticPointDefence;

/// Present when the EntityConfig has a non-empty `tags` list.
/// Mirrors the TOML tags onto the ECS entity so snapshot builders can include them.
#[derive(Component, Clone, Debug)]
pub struct EntityTagsSection(pub Vec<String>);

/// Present on an entity a **script spawned mid-run**, carrying what the spawn
/// was made from (issue #863) — see [`crate::world::spawn_origin`] for why the
/// record exists and why it rides on the entity.
///
/// Absent on every authored `[[entity]]` block, and the absence is the useful
/// half of the signal: an entity with no origin is one any fresh boot of the
/// same scenario puts back by itself, so a resume waits for the bootstrap to
/// produce it rather than building it. An entity *with* one is a consequence of
/// how this particular run went, and nothing but the save will ever put it back.
///
/// Written at exactly one site — `world::server`'s `ActionCmd::SpawnEntity` arm,
/// the one place a runtime spawn happens — and read at exactly two:
/// `snapshot::capture` and `snapshot::restore`.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct EntitySpawnOrigin(pub crate::world::spawn_origin::SpawnOrigin);

/// Present when the EntityConfig has a `faction` UUID.
/// The AI tick reads this component to determine `self_faction` and enemy evaluation.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct FactionComponent(pub uuid::Uuid);

/// Present when the EntityConfig has a `[weapons_console]` section.
/// The AI tick reads this component to determine weapons range and phaser readiness.
#[derive(Component, Clone, Debug)]
pub struct WeaponsConsoleSection(pub crate::entities::config::WeaponsConsoleConfig);

/// Present when the EntityConfig has a `[helm_console]` section.
/// The AI tick reads this to build a `ShipPhysicsConfig` instead of using hardcoded defaults.
#[derive(Component, Clone, Debug)]
pub struct HelmConsoleSection(pub crate::entities::config::HelmConsoleConfig);

/// Present when the EntityConfig has a `[helm_capability]` section.
/// Describes vertical movement mode and impulse steering policy.
#[derive(Component, Clone, Debug)]
pub struct HelmCapabilitySection(pub crate::entities::config::HelmCapabilityConfig);

/// Present when the EntityConfig had a [radar_appearance] section.
#[derive(Component, Clone, Debug)]
pub struct RadarAppearanceSection(pub crate::entities::config::RadarAppearanceConfig);

/// Present when the EntityConfig has an `[audio]` section.
///
/// Read off the `LocalShip` by `server::audio::push_audio_config` to build the
/// host page's audio graph. It has to be a component rather than a resource
/// because the lobby ship picker chooses the hull at game start — see
/// `spawn_game_start_entities`, which overrides the world's placeholder config
/// with the selected ship.
#[derive(Component, Clone, Debug)]
pub struct ShipAudioSection(pub crate::audio_config::ShipAudioConfig);

/// Present when the EntityConfig has a `[target]` section.
/// Carries targetability tags, threat level, and description.
#[derive(Component, Clone, Debug)]
pub struct EntityTarget(pub crate::entities::target::TargetSection);

/// Present when the EntityConfig has a `[cinematic_camera]` section.
/// The viewscreen reads this for cinematic camera positioning and tracking.
#[derive(Component, Clone, Debug)]
pub struct CinematicCameraSection(pub crate::entities::config::CinematicCameraConfig);

/// Hull tracker attached to any entity (NPC ship, asteroid) that carries a
/// `[hull]` section in its TOML config. For NPC ships the HP is placed in a
/// single `CaptainChair` console slot; asteroids use the same single-slot
/// convention. Damage systems query this component to deal damage and detect
/// destruction.
///
/// This is a Bevy ECS component wrapping the pure `SystemHull` struct
/// (parent issue #516 sub-issue #616). It is the sole per-ship hull store
/// after PRD #597 PR 10 (the retired `ShipHullIntegrity` global resource
/// that used to hold the player-ship copy was deleted along with its
/// dual-write bridge).
#[derive(Component, Clone, Debug)]
pub struct EntitySystemHull(pub crate::ship::damage::SystemHull);

/// Bevy ECS component wrapping the pure [`crate::ship::damage::ShipArcHull`]
/// struct (issue #514). Attached to ship entities that declare
/// `[[shield_arc]]` blocks with `hull_max_hp` fields. `ship/damage.rs` is
/// Bevy-free per AGENTS.md rule 9, so the pure per-arc HP logic lives
/// there and this component wraps it for ECS storage.
///
/// The rest of the codebase uses the type alias
/// [`crate::ship::damage::ShipArcHull`] for readability at call sites — this
/// wrapper is a thin newtype that lets the pure struct participate in
/// Bevy queries.
#[derive(Component, Clone, Debug, Default)]
pub struct EntityShipArcHull(pub crate::ship::damage::ShipArcHull);

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
        EntityMass(config.mass),
    ));

    // Insert optional human-readable ID
    if let Some(human_id) = id {
        entity_commands.insert(EntityId(human_id));
    }

    for section in SPAWN_SECTIONS {
        section.apply(config, position, &mut entity_commands);
    }

    entity_commands.id()
}

// ── SpawnSection ladder ──────────────────────────────────────────
//
// Each optional `[section]` of an `EntityConfig` is one `SpawnSection`: given
// the resolved config and the spawn position, it inserts whatever components
// that section contributes. `spawn_entity` walks `SPAWN_SECTIONS` once, in
// order, so adding a section is a new impl plus one registry line rather than
// an edit threaded through the middle of a 1,000-line function.
//
// The registry ORDER is load-bearing, not cosmetic. Sections insert in the
// exact sequence they always did, because a different insert order changes the
// order archetypes are first created in, which the authoritative-state digest
// is sensitive to. Reordering `SPAWN_SECTIONS` is a determinism change; adding
// a section at the end is not.
trait SpawnSection {
    fn apply(&self, config: &EntityConfig, position: Vec3, cmds: &mut EntityCommands);
}

/// Every section, in insertion order. See [`SpawnSection`] on why the order
/// is load-bearing.
const SPAWN_SECTIONS: &[&dyn SpawnSection] = &[
    &ColliderSpawn,
    &AppearanceSpawn,
    &MeshSpawn,
    &StarSpawn,
    &PlanetSpawn,
    &NameSpawn,
    &LightsSpawn,
    &AsteroidFieldSpawn,
    &RegionShapeSpawn,
    &RegionEffectsSpawn,
    &CinematicCameraSpawn,
    &BehaviourSpawn,
    &AiProfileSpawn,
    &LodBubbleSpawn,
    &TagsSpawn,
    &RadarAppearanceSpawn,
    &TargetSpawn,
    &AudioSpawn,
    &FactionSpawn,
    &WeaponsConsoleSpawn,
    &TorpedoesSpawn,
    &BlastersSpawn,
    &HelmConsoleSpawn,
    &HelmCapabilitySpawn,
    &CommsSpawn,
    &ShieldsSpawn,
    &ShieldsDamageHistorySpawn,
    &ArcHullSpawn,
    &InfrastructureSpawn,
    &TractorSpawn,
    &ExternalRepairDispatchSpawn,
    &HeldResponseSpawn,
    &DockSpawn,
    &UmbilicalSpawn,
    &ScanSpawn,
    &CivilianSpawn,
    &HullSpawn,
];

struct ColliderSpawn;
impl SpawnSection for ColliderSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Collider section â†’ Rapier collider + rigid body
        if let Some(collider) = &config.collider {
            let rapier_collider = match collider.shape {
                crate::entities::config::ColliderShape::Ball => Collider::ball(collider.radius),
                crate::entities::config::ColliderShape::Capsule => {
                    Collider::capsule_y(collider.length / 2.0, collider.radius)
                }
                // `Collider::cylinder` takes the half-height FIRST and the radius
                // second, and takes the half-height rather than the full height —
                // which is why the TOML authors `half_height` instead of reusing
                // the Capsule's `length`. The number in the file is the number
                // handed to rapier; nothing is doubled or halved on the way.
                //
                // A `Cylinder` cannot reach here without a half-height: the load
                // path rejects one (`entity_config::validate_collider_config`),
                // because a zero-thickness disc is a body nothing can ever be
                // inside — the pass-through bug the station-collider correction
                // just fixed. The fallback is the belt to that braces, and it errs
                // UPWARDS to the radius, i.e. to the enclosing sphere this variant
                // replaces: a degenerate authored body keeps ships outside a hull
                // rather than letting them through it.
                crate::entities::config::ColliderShape::Cylinder => Collider::cylinder(
                    collider.half_height.unwrap_or(collider.radius),
                    collider.radius,
                ),
            };
            cmds.insert((
                rapier_collider,
                // Pin the physics shape to its AUTHORED size regardless of the
                // entity's `Transform.scale` (issue: starbase collider oversize).
                //
                // Rapier's `apply_scale` folds `GlobalTransform.scale` into the
                // collider shape by default (`ColliderScale::Relative(ONE)`). That is
                // fine while the transform's scale is 1, which it always is HEADLESS —
                // nothing scales an entity's transform there. But under `render`
                // (`opts.render`, i.e. the browser), `update_mesh_lod` writes the
                // model's `[base].scale` onto this same entity's `Transform` for every
                // non-near LOD tier, because the generated LOD meshes are authored at
                // raw model size and the parent has to supply the base scale (see
                // `tier_parent_scale`). For the starbase that base scale is [15,18,18],
                // so its authored radius-17.04 cylinder was silently inflated to a
                // ~300-unit disc the moment the station dropped to LOD1/2 — a ship
                // dead-stopped and took ram damage hundreds of units out in clear sky,
                // and only in the browser (headless, with no LOD system, never saw it,
                // so no digest ever recorded the inflation). The render comment on
                // `render_spawned_entities` already asserts the invariant this makes
                // true: "an entity's transform is simulation state ... a visual effect
                // has no business animating it." `Absolute(ONE)` REPLACES the transform
                // scale rather than multiplying it, so the shape stays the authored
                // size in both worlds and the physics matches what the renderer draws.
                ColliderScale::Absolute(Vect::ONE),
                RigidBody::KinematicPositionBased,
                ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
                ColliderSection(collider.clone()),
            ));
        }
    }
}

struct AppearanceSpawn;
impl SpawnSection for AppearanceSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Appearance section
        if let Some(appearance) = &config.appearance {
            cmds.insert(AppearanceSection(appearance.clone()));
        }
    }
}

struct MeshSpawn;
impl SpawnSection for MeshSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Mesh section
        if let Some(mesh) = &config.mesh {
            cmds.insert(MeshSection(mesh.clone()));
        }
    }
}

struct StarSpawn;
impl SpawnSection for StarSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Star section
        if let Some(star) = &config.star {
            cmds.insert(StarSection(star.clone()));
        }
    }
}

struct PlanetSpawn;
impl SpawnSection for PlanetSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Planet section
        if let Some(planet) = &config.planet {
            cmds.insert(PlanetSection(planet.clone()));
        }
    }
}

struct NameSpawn;
impl SpawnSection for NameSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Top-level name scalar
        if let Some(name) = &config.name {
            cmds.insert(EntityName(name.clone()));
        }
    }
}

struct LightsSpawn;
impl SpawnSection for LightsSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Lights array â€” present when one or more [[light]] entries were declared.
        if !config.light.is_empty() {
            cmds.insert(Lights(config.light.clone()));
        }
    }
}

struct AsteroidFieldSpawn;
impl SpawnSection for AsteroidFieldSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Asteroid field section
        if let Some(field) = &config.asteroid_field {
            cmds.insert(AsteroidFieldSection(field.clone()));
        }
    }
}

struct RegionShapeSpawn;
impl SpawnSection for RegionShapeSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Region shape section
        if let Some(shape) = &config.shape {
            cmds.insert(RegionShapeSection(shape.clone()));
        }
    }
}

struct RegionEffectsSpawn;
impl SpawnSection for RegionEffectsSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Region effects section
        if let Some(effects) = &config.effects {
            if !effects.is_empty() {
                cmds.insert(RegionEffectsSection(effects.to_kinds()));
            }
        }
    }
}

struct CinematicCameraSpawn;
impl SpawnSection for CinematicCameraSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Cinematic camera section
        if let Some(cam) = &config.cinematic_camera {
            cmds.insert(CinematicCameraSection(cam.clone()));
        }
    }
}

struct BehaviourSpawn;
impl SpawnSection for BehaviourSpawn {
    fn apply(&self, config: &EntityConfig, position: Vec3, cmds: &mut EntityCommands) {
        // Behaviour section — the "this entity is AI-driven" predicate; ai_plugin
        // registers an AI token for any entity carrying it.
        //
        // Decomposed (issue #1198) into per-concern sub-builders. Each inserts a
        // cohesive slice of the ship's components, and they are called here in the
        // EXACT order their inserts previously ran inline. Component-insertion order
        // is archetype-creation order, which the authoritative digest is sensitive
        // to (see [`SpawnSection`] above and `tests/archetype_order_determinism.rs`),
        // so this split is pure code motion: the sequence and set of `.insert()`
        // calls is byte-for-byte the one that shipped before it.
        let static_point_defence = config.is_static_point_defence();
        if !(config.behaviour.is_some() || static_point_defence) {
            return;
        }
        insert_behaviour_markers(config, static_point_defence, cmds);
        let power_group_seed = insert_ship_config_and_core_bundle(config, position, cmds);
        insert_ship_scratch_state(cmds);
        insert_power_state(config, &power_group_seed, cmds);
        // Built here (pure, no inserts) so its authored-block reads sit where they
        // always did; the map is inserted later, in place, by
        // `insert_power_multipliers_and_modifiers`.
        let multipliers = build_power_multipliers(config);
        insert_target_selectors(config, cmds);
        insert_ai_policies(config, cmds);
        insert_power_multipliers_and_modifiers(config, multipliers, cmds);
        insert_ship_markers_and_trackers(cmds);
    }
}

/// The "this entity is AI-driven" markers: the optional [`BehaviourSection`], the
/// [`StaticPointDefence`] marker for a static platform, and the empty per-ship
/// blackboard set every AI hull carries.
fn insert_behaviour_markers(
    config: &EntityConfig,
    static_point_defence: bool,
    cmds: &mut EntityCommands,
) {
    if let Some(behaviour) = &config.behaviour {
        cmds.insert(BehaviourSection(behaviour.clone()));
    }
    if static_point_defence {
        cmds.insert(StaticPointDefence);
    }
    cmds.insert(crate::server_app::ShipSystemBlackboards::default());
}

/// Builds the ship's [`ShipConfigComponent`] from its own TOML blocks, seeds the
/// boot ratings / physics, and inserts the core per-ship state bundle.
///
/// Returns the authored power-group seed, computed BEFORE `ship_config` is moved
/// into the entity so [`insert_power_state`] can build the reactor from groups
/// beyond the canonical three.
fn insert_ship_config_and_core_bundle(
    config: &EntityConfig,
    position: Vec3,
    cmds: &mut EntityCommands,
) -> Vec<(crate::core::messages::PowerGroupId, u8)> {
    // Build the ship's ShipConfigComponent from its own TOML [[station]]/
    // [[system]]/[power_groups] blocks, parsed the same way ship entity TOMLs
    // is parsed. If the entity TOML declared none, this is a truly empty
    // config (no stations, no systems) — the loop below simply sets nothing.
    let ship_config = match &config.ship_config {
        Some(sc) => crate::ship_plugin::ShipConfigComponent(sc.clone()),
        None => crate::ship_plugin::ShipConfigComponent(crate::ship::config::ShipConfig {
            stations: vec![],
            systems: vec![],
            power_groups: std::collections::HashMap::new(),
            coordination_lag_secs: 0.0,
        }),
    };
    // Boot seeding (issue #871). Every hull spawned here comes up with
    // nobody connected, so every station boots on `Backfill` — which is
    // what "NPC" now means: a stationed ship with nobody in the seats.
    //
    // This is the SAME `seed_boot_ratings` the player game-start path in
    // `server_app::spawn_game_start_entities` calls, not a parallel
    // NPC-only rule. The blanket "set every declared system to Ai" loop
    // that used to live here worked only because NPC hulls declared no
    // stations at all; once they do, a system's control source has to come
    // from its station's rating exactly as it does on a player hull, or a
    // human taking an NPC seat could never be admitted.
    //
    // A hull with no stations (a bare `[behaviour]` entity, or one whose
    // only systems are auto-generated) still ends up all-Ai: every one of
    // its systems is `ai_only` and the second pass covers them.
    let (resolver, active_ratings) = crate::ship::rating::seed_boot_ratings(&ship_config.0, |_| {
        crate::ship::rating::BACKFILL_RATING.to_string()
    });
    // Seed the reactor from the ship's authored power groups (issue #762)
    // BEFORE `ship_config` is moved into the entity, so authored groups
    // beyond the canonical three (e.g. `ops`) are allocatable rather than
    // returning `UnknownGroup`. Empty for ships with no `[power_groups.*]`.
    let power_group_seed =
        crate::ship::power::authored_power_group_seed(&ship_config.0.power_groups);
    // Seed ShipPhysics from the spawn position so the per-entity helm loop
    // starts with the correct initial state rather than (0, 0).
    let ship_physics = crate::ship::state::ShipPhysics {
        x: position.x,
        z: position.z,
        yaw: {
            let rot = bevy::math::Quat::from_euler(bevy::math::EulerRot::YXZ, 0.0, 0.0, 0.0);
            let _ = rot;
            0.0 // initial yaw; updated each tick by integrate_ship_physics
        },
        ..Default::default()
    };
    cmds.insert((
        ship_config,
        crate::core::messages::AdmittedCommands::default(),
        crate::ship_plugin::ShipSystemControlSources(resolver),
        crate::ship_plugin::ActiveStationRatings(active_ratings),
        crate::ship_plugin::CoordinationQueue::default(),
        ship_physics,
        crate::ship_plugin::HelmWaypointClearance::default(),
        crate::console::weapons::TacticalRadarSelection::default(),
        crate::console::weapons::ActiveBeam::default(),
        crate::console::weapons::PhaserCooldown::default(),
        crate::ship::sensors::SensorRadarSelection::default(),
        // The restraint lever (issue #1041) rides in the SAME bundle as the
        // alert it layers under, nested rather than added as a second
        // `insert`. Every ship carries it, player and NPC alike, so the
        // captain's order and a scenario's order land on the same state and
        // the fire hosts need no "is this an NPC?" branch; it defaults to
        // released, so a hull nobody orders behaves exactly as it did
        // before this issue.
        //
        // The nesting is load-bearing, not tidiness. A second `insert` is a
        // second queued command and a second archetype move per ship, which
        // shifts what the command queue does afterwards — and a world that
        // never pulls this lever must be byte-identical, which is the
        // acceptance criterion this whole slice is built to. Bundles nest,
        // so pairing it with `ShipRedAlert` keeps the tuple inside Bevy's
        // 15-element ceiling without paying for a second command.
        (
            crate::ship::state::ShipRedAlert::default(),
            crate::ship::state::ShipWeaponsHold::default(),
        ),
        crate::ship::state::ShipViewMode::default(),
        crate::ship::state::ShipPhaserFrequency::default(),
        crate::console::navigation::NavigationWaypoint::default(),
    ));
    power_group_seed
}

/// Per-ship scratch/coordination state every AI hull carries: helm intent,
/// objective cursors, the channel-3 debounce cells, and the repair queue. All
/// default-constructed, so this depends on neither the config nor the position.
fn insert_ship_scratch_state(cmds: &mut EntityCommands) {
    // Per-entity helm intent (audit follow-up). Every ship carries
    // its own `LastHelmInput` so systems that iterate `With<Ship>`
    // and read `Option<&LastHelmInput>` see a real value on NPCs
    // instead of the `unwrap_or_default()` fallback. Notably
    // `console_ai::server::ai_power_allocation` reads `thrust` to drive
    // its hysteresis-based movement rule (`tick_power_movement_rule`),
    // engaging/disengaging helm power by ±1 on sustained high/low
    // thrust rather than pinning to an absolute level. Inserted
    // separately because Bevy's tuple Bundle max is 15 elements.
    // `ShipStationStances` rides the same command as `LastHelmInput` (a
    // tuple is one Bundle, one archetype move) so a hull nobody commands
    // pays no extra insert and stays byte-identical. Empty by default: only
    // a human Command operator's explicit stance pick ever fills it.
    cmds.insert((
        crate::ship_plugin::LastHelmInput::default(),
        crate::console::command::server::ShipStationStances::default(),
        // Edge-detection scratch for the persist-behind-human trigger
        // (issue #1108). Transient and NOT folded into the sim digest —
        // see the type's own docs; a fresh/reloaded hull records its first
        // observation and fires no edge.
        crate::console::command::server::LastDirectedControl::default(),
    ));
    // Per-objective route cursors: where this ship is on each objective's
    // route. Read by the low-LOD `simulate_low_lod_ships` path, the high-LOD
    // `helm_patrol`, and `operate_navigation_ai`; written only by
    // `advance_objective_cursors`. A ship without one cannot patrol.
    // Inserted separately to stay under Bevy's tuple-Bundle element cap.
    cmds.insert(crate::ai::server::ObjectiveCursors::default());
    // Per-ship coordination bus state (audit follow-up). Every ship
    // tracks its own shields down/restore notification cycle and its
    // own sensors→tactical frequency-hint dedupe state so the two
    // coordination emitters (`emit_shields_coordination`,
    // `tick_sensors_frequency_hint`) can iterate `With<Ship>` and
    // route into each ship's own `CoordinationQueue` via
    // `CoordinationEnqueue.source_entity`.
    cmds.insert(crate::ship::shields::ShieldsCoordinationState::default());
    cmds.insert(crate::ship::sensors::SensorsFrequencyState::default());
    cmds.insert(crate::ship::sensors::SensorsThreatState::default());
    // Power brownout advisory debounce state (issue #678): per-ship
    // so each ship tracks its own brownout notification cycle.
    cmds.insert(crate::ship::power::PowerBrownoutState::default());
    // Weapons->Helm arc-bearing request state (issue #677): per-ship
    // debounce for the channel-3 request, and the pending bearing Helm
    // AI folds into its steering once the request is consumed.
    cmds.insert(crate::console::weapons::WeaponsArcRequestState::default());
    cmds.insert(crate::ship_plugin::PendingArcBearingRequest::default());
    // Distinct docking intent (issue #742): the sanctioned home for
    // controlled reverse / lateral close manoeuvres, kept separate from the
    // facing-only arc-bearing request above.
    cmds.insert(crate::ship_plugin::DockingMotionIntent::default());
    cmds.insert(crate::ship::shields::PendingShieldsThreatBearing::default());
    // Sensors→Tactical frequency advisory a backfilled Tactical consumes
    // off the channel-3 bus (issue #873).
    cmds.insert(crate::ship_plugin::PendingTacticalFrequencyHint::default());
    // Per-ship intent-narration memory (issue #879): the previous decision
    // snapshot of each narrating seat plus this ship's advisory counter.
    // Belongs to the ship rather than to its LOD tier — a demoted hull
    // still has seats to narrate for if a human ever takes one.
    cmds.insert(crate::ship_plugin::ShipIntentNarration::default());
    cmds.insert(crate::ship_plugin::LastSystemTiers::default());
    cmds.insert(crate::ship_plugin::RepairHumanAlerted::default());
    cmds.insert(crate::console::repair::server::RepairRequestQueue::default());
}

/// The reactor and its allocation AI: the [`ShipPowerSystem`] (seeded from the
/// authored power groups), the per-entity [`PowerConfigResource`], and the
/// optional inline power-allocation policy from `[power.ai_policy]`.
fn insert_power_state(
    config: &EntityConfig,
    power_group_seed: &[(crate::core::messages::PowerGroupId, u8)],
    cmds: &mut EntityCommands,
) {
    cmds.insert(crate::ship::power::ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_authored_groups(
            &crate::modifiers::power_system::PowerConfig::default(),
            power_group_seed,
        ),
    ));
    // Per-entity power config (PRD #597 gap-4 closure). NPCs without a
    // `[power]` TOML block get `PowerConfigResource::default()` /
    // `PowerAiConfigResource::default()` so `translate_power_modifiers`,
    // `ai_power_allocation` (issue #693), and `tick_power_system` can
    // iterate every ship uniformly (`With<Ship>`) without an `is_npc`
    // fork. When the TOML supplies `[power]` / `[power.ai]`, those
    // values seed the components.
    let power_config = match &config.power {
        Some(pc) => {
            crate::ship::power::PowerConfigResource(crate::modifiers::power_system::PowerConfig {
                capacity: pc.capacity,
                rates: pc.rates,
                sustainable_total: pc.sustainable_total,
                max_commanded_total: pc.max_commanded_total,
                emergency_threshold: pc.emergency_threshold,
            })
        }
        None => crate::ship::power::PowerConfigResource::default(),
    };
    cmds.insert(crate::ship::power::ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_authored_groups(
            &power_config.0,
            power_group_seed,
        ),
    ));
    cmds.insert(power_config);
    // Inline stateless Power allocation AI policy (issue #784) — from the
    // ship's `[power.ai_policy]` block. Since #885b stage 5d there is no
    // Rust-side synthesiser behind it: strict AI-declaration mode
    // (`AiDeclarationMode::DEFAULT`) rejects an AI-capable hull that omits
    // the block at load, so an AI-bearing entity always authors one and the
    // `None` arm is reached only by a config built in code. Nothing is
    // attached in that case — an undeclared system gets no automation, which
    // is PRD #774 US7's requirement. `to_policy` cannot fail here: the block
    // was validated in `EntityConfig::from_toml`.
    if let Some(ai) = config.power.as_ref().and_then(|pc| pc.ai_policy.as_ref()) {
        cmds.insert((
            crate::ship::power::PowerAiPolicy(ai.to_policy().unwrap_or_default()),
            // Carried from the SAME authored block (issue #889's
            // evaluate_every_ticks, wired at runtime): a resolved
            // `AiPolicy` alone forgets this field, so it rides alongside
            // as a sibling component.
            crate::ship::power::PowerAiCadence(ai.evaluate_every_ticks),
        ));
    }
}

/// Per-entity power multipliers, defaulted then overridden by any per-console
/// `power_multipliers` blocks. Pure — no inserts; the caller inserts the result
/// via [`insert_power_multipliers_and_modifiers`] at the original insert site.
fn build_power_multipliers(
    config: &EntityConfig,
) -> std::collections::HashMap<crate::core::messages::PowerGroupId, [f32; 4]> {
    // Per-entity power multipliers. Seeded from any per-console TOML
    // `power_multipliers` blocks (helm_console/weapons_console/shields_console)
    // and otherwise defaulted so NPC ships still get MaxSpeed / PhaserDamage
    // / ShieldRegen bonuses translated by `translate_power_modifiers`.
    //
    // After issue #617 the map is keyed by `PowerGroupId`.
    let defaults = [-0.5f32, 0.0, 0.25, 0.5];
    let mut multipliers: std::collections::HashMap<crate::core::messages::PowerGroupId, [f32; 4]> =
        std::collections::HashMap::from([
            (
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                ),
                defaults,
            ),
            (
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                ),
                defaults,
            ),
            (
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
                ),
                defaults,
            ),
        ]);
    if let Some(hc) = &config.helm_console {
        if let Some(pm) = hc.power_multipliers {
            multipliers.insert(
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                ),
                pm,
            );
        }
    }
    if let Some(wc) = &config.weapons_console {
        if let Some(pm) = wc.power_multipliers {
            multipliers.insert(
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                ),
                pm,
            );
        }
    }
    if let Some(sc) = &config.shields_console {
        if let Some(pm) = sc.power_multipliers {
            multipliers.insert(
                crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
                ),
                pm,
            );
        }
    }
    multipliers
}

/// The AI target/selector policies read straight off the authored `selector`
/// blocks: sensors AI config, then the sensors / tactical / navigation / repair
/// target selectors.
fn insert_target_selectors(config: &EntityConfig, cmds: &mut EntityCommands) {
    // Sensors AI config — loaded from [sensors_console.ai] if present,
    // otherwise the parse-time default. Inserted for EVERY entity that
    // carries a `[behaviour]` block (i.e. every ship spawned through this
    // path); it used to be inserted only when the TOML also carried the
    // `.ai` sub-section, and `tick_frequency_hint_high_fidelity` then fell back to the
    // global Resource. The player ship does not come through here at all —
    // `server_app::spawn_game_start_entities` attaches its copy.
    cmds.insert(
        config
            .sensors_console
            .as_ref()
            .and_then(|sc| sc.ai.as_ref())
            .map(|ai| crate::ship::sensors::SensorsAiConfigResource {
                frequency_hint_delay_secs: ai.frequency_hint_delay_secs,
            })
            .unwrap_or_default(),
    );
    // Sensors target selector (issue #776) — the per-system ranking policy
    // `operate_sensors_ai` runs to pick the science target, from the
    // authored `[sensors_console.selector]` block. Since #885b stage 5d
    // there is no Rust-side synthesiser behind it: strict AI-declaration
    // mode rejects an AI-capable hull that omits the block at load, so an
    // unauthored selector means no component and therefore no ranking rather
    // than an invented one. `to_selector` cannot fail here: the block was
    // validated in `EntityConfig::from_toml`. Power rating is exposed to the
    // selector as `self_fact(power_rating)`.
    if let Some(s) = config
        .sensors_console
        .as_ref()
        .and_then(|sc| sc.selector.as_ref())
    {
        cmds.insert(crate::ship::sensors::SensorsTargetSelector {
            selector: s.to_selector().unwrap_or_default(),
            power_rating: config.power_rating.map(|r| r as f32),
        });
    }
    // Tactical target selector (issue #777) — the per-system ranking policy
    // `ai_target_selection` runs to pick the authoritative weapons target,
    // from the authored `[weapons_console.selector]` block.
    if let Some(s) = config
        .weapons_console
        .as_ref()
        .and_then(|wc| wc.selector.as_ref())
    {
        cmds.insert(crate::console::weapons::TacticalTargetSelector {
            selector: s.to_selector().unwrap_or_default(),
            power_rating: config.power_rating.map(|r| r as f32),
            // AC6 (issue #781): explicit radar idle from `[weapons_console]
            // selector_idle`, else the baseline (radar runs its selector).
            idle: config
                .weapons_console
                .as_ref()
                .map(|wc| wc.selector_idle)
                .unwrap_or(false),
        });
    }
    // Navigation target selector (issue #778) — the per-system ranking
    // policy `operate_navigation_ai` runs to rank objective destinations and
    // eligible chart contacts into the shared Waypoint, from the authored
    // `[navigation_console.selector]` block.
    if let Some(s) = config
        .navigation_console
        .as_ref()
        .and_then(|nc| nc.selector.as_ref())
    {
        cmds.insert(crate::console::navigation::NavigationTargetSelector {
            selector: s.to_selector().unwrap_or_default(),
            power_rating: config.power_rating.map(|r| r as f32),
        });
    }
    // Repair target selector (issue #785) — the per-system ranking policy
    // `operate_repair_ai` runs once per free repair team to rank the ship's
    // damaged stations into ordinary admitted `DispatchRepairTeam` inputs,
    // from the authored `[repair.selector]` block. Attached whenever the
    // selector is authored, not only to ships that declare repair TEAMS —
    // the teams component is what gates dispatch, and a ship that gains
    // teams later still has its ranking.
    if let Some(s) = config.repair.as_ref().and_then(|rc| rc.selector.as_ref()) {
        cmds.insert(crate::console::repair::server::RepairTargetSelector {
            selector: s.to_selector().unwrap_or_default(),
            power_rating: config.power_rating.map(|r| r as f32),
        });
    }
}

/// The remaining per-console AI policies: comms hail selector + response policy,
/// shields AI config + focus policy, the captain red-alert policy, and the helm
/// fine-system policy map.
fn insert_ai_policies(config: &EntityConfig, cmds: &mut EntityCommands) {
    // Comms hail selector + dialogue-response policy (issue #786) — the two
    // halves of the Comms console's AI, from the authored
    // `[comms_console.selector]` / `[comms_console.ai]` blocks. Resolved by
    // the SHARED `comms_console_ai_components` helper, because
    // `server_app::spawn_game_start_entities` must attach the identical pair
    // to the player ship — the only ship either Comms AI host actually runs
    // on, since both are filtered `With<LocalShip>`. Each half is `None` when
    // its block is unauthored, and nothing is attached for it.
    let (comms_selector, comms_response_policy, comms_response_cadence) =
        crate::console::comms::server::comms_console_ai_components(config);
    if let Some(sel) = comms_selector {
        cmds.insert(sel);
    }
    if let Some(policy) = comms_response_policy {
        cmds.insert(policy);
    }
    if let Some(cadence) = comms_response_cadence {
        cmds.insert(cadence);
    }
    // Shields AI config — loaded from [shields_console.ai] if present,
    // otherwise the parse-time default. Inserted for every entity carrying
    // a `[behaviour]` block, alongside the sensors block above and inside
    // the same ship gate: `ai_shield_focus` and `emit_shields_coordination`
    // both query `With<Ship>`, so asteroids/stars/planets have no use for
    // it. It used to be inserted only when the TOML also carried the `.ai`
    // sub-section, and those readers then fell back to the global Resource
    // — which `server_app` writes from the PLAYER ship's TOML. Every NPC
    // now owns its own shields-AI tuning.
    cmds.insert(
        config
            .shields_console
            .as_ref()
            .and_then(|sc| sc.ai.as_ref())
            .map(|ai| crate::ship::shields::ShieldsAiConfigResource {
                damage_window_secs: ai.damage_window_secs,
                min_damage_window_secs: ai.min_damage_window_secs,
                damage_pct_threshold: ai.damage_pct_threshold,
                health_ratio_threshold: ai.health_ratio_threshold,
                ..Default::default()
            })
            .unwrap_or_default(),
    );
    // Shields focus AI policy (issue #783) — the inline stateless
    // `shield_focus` policy from the authored `[shields_console.ai_policy]`
    // block, so `ai_shield_focus` resolves a data-authored gate + reads the
    // authored windows/thresholds from the policy `param` map rather than the
    // retired `ai_cfg.*` reads. `to_policy` cannot fail here: the block was
    // validated in `EntityConfig::from_toml`.
    if let Some(ai) = config
        .shields_console
        .as_ref()
        .and_then(|sc| sc.ai_policy.as_ref())
    {
        cmds.insert(crate::ship::shields::ShieldsFocusAiPolicy(
            ai.to_policy().unwrap_or_default(),
        ));
    }
    // Captain AI policy (issue #775) — the inline stateless Red Alert
    // policy from the authored `[captain_console.ai]` block, so
    // `operate_captain_ai` reads a data-authored policy rather than a
    // hardcoded controller.
    if let Some(ai) = config.captain_console.as_ref().and_then(|c| c.ai.as_ref()) {
        cmds.insert(crate::console::captain::server::CaptainAiPolicy(
            ai.to_policy().unwrap_or_default(),
        ));
    }
    // Helm fine-system AI policies (issues #779/#780, collapsed by #1209):
    // the inline `[helm_console.*_ai]` policies — engines (longitudinal),
    // steering (yaw), lateral, vertical, impulse, boost — resolved into ONE
    // keyed `FineSystemAiPolicies` map so each host reads a data-authored mode
    // verb by its `system_id()` rather than actuating unconditionally. One
    // entry per authored block; an unauthored axis contributes none (strict
    // AI-declaration mode rejects an unauthored AI-capable axis at load).
    // Built the same shape the weapon banks use above — mirror of
    // `PhaserBankAiPolicies`. `to_policy` cannot fail: each block was
    // validated in `EntityConfig::from_toml`.
    if let Some(hc) = config.helm_console.as_ref() {
        use crate::ship::system_registry as sr;
        let mut fine_policies: std::collections::BTreeMap<
            crate::core::messages::SystemId,
            crate::ai::policy::AiPolicy,
        > = std::collections::BTreeMap::new();
        for (block, system_id) in [
            (hc.engines_ai.as_ref(), sr::helm_thrust_system_id()),
            (hc.steering_ai.as_ref(), sr::helm_steering_system_id()),
            (hc.lateral_ai.as_ref(), sr::lateral_thrust_system_id()),
            (hc.vertical_ai.as_ref(), sr::vertical_thrust_system_id()),
            (hc.impulse_ai.as_ref(), sr::helm_impulse_system_id()),
            (hc.boost_ai.as_ref(), sr::helm_boost_system_id()),
        ] {
            if let Some(ai) = block {
                fine_policies.insert(system_id, ai.to_policy().unwrap_or_default());
            }
        }
        cmds.insert(crate::ship::helm_ai::FineSystemAiPolicies(fine_policies));
    }
}

/// Inserts the [`PowerMultiplierResource`] built by [`build_power_multipliers`],
/// the empty per-entity [`ShipModifiers`] cache, and — only when the config
/// declares repair TEAMS — the [`ShipRepairTeams`].
fn insert_power_multipliers_and_modifiers(
    config: &EntityConfig,
    multipliers: std::collections::HashMap<crate::core::messages::PowerGroupId, [f32; 4]>,
    cmds: &mut EntityCommands,
) {
    cmds.insert(crate::ship::power::PowerMultiplierResource { multipliers });
    // ShipModifiers as per-entity component (PR 6/9 — PRD #597). Every ship
    // gets an empty modifier cache. Region-entry observers and
    // translate_power_modifiers write to the subject entity's cache;
    // translate_impulse_modifiers remains LocalShip-only (ShipImpulse is a
    // player-only mechanic).
    cmds.insert(crate::modifiers::ShipModifiers::new());
    // Per-entity ShipRepairTeams — only insert when the entity TOML declares
    // repair TEAMS, i.e. a `[repair] repair_team_count` above zero. No
    // count means the ship has no repair teams (the default behaviour for
    // NPCs today).
    //
    // The gate is the COUNT and not the presence of `[repair]`, because
    // since #885b every hull authors `[repair.selector]` and TOML cannot
    // write that sub-table without bringing `[repair]` into existence — a
    // presence gate would hand two teams to six NPC hulls that never had
    // any. See `RepairConfig::declares_teams`.
    if let Some(repair_cfg) = &config.repair {
        if repair_cfg.declares_teams() {
            let timings = repair_cfg.to_runtime();
            cmds.insert(crate::console::repair::server::ShipRepairTeams(
                crate::modifiers::repair_teams::RepairTeams::new_with_timings(
                    repair_cfg.repair_team_count as usize,
                    timings,
                ),
            ));
        }
    }
}

/// The `Ship` marker and the per-ship trackers every hull (player + NPC) carries:
/// collision cooldown, combat-activity trackers, and the idle impulse/boost drives.
fn insert_ship_markers_and_trackers(cmds: &mut EntityCommands) {
    // All ship entities carry the Ship marker — player and NPC alike.
    // The LocalShip marker (not set here) is the viewscreen selector only.
    cmds.insert(crate::server_app::Ship);
    // Per-entity CollisionCooldown so NPC ships have their own collision
    // cooldown timer (PRD #597 PR-8). Player ship gets one in
    // `spawn_game_start_entities`.
    cmds.insert(crate::server_app::CollisionCooldown::default());
    // Per-entity combat activity trackers (PRD #597 PR-10). Every ship
    // (player + NPC) records its own recent damage/hostile-fire/weapon
    // fire and last attacker.
    cmds.insert(crate::ship::combat_activity::RecentCombatActivity::default());
    cmds.insert(crate::server_app::WeaponFiredThisTick::default());
    cmds.insert(crate::server_app::ShipAttackedThisTick::default());
    cmds.insert(crate::console::weapons::LastShipAttacker::default());
    // Per-ship impulse drive state (audit follow-up). NPCs carry an
    // idle `ShipImpulse` so `handle_blocks_impulse_region_enter` can
    // route per-subject and future NPC helm AI can toggle impulse
    // through the same per-ship pathway the player uses.
    cmds.insert(crate::server_app::ShipImpulse::default());
    // Per-ship boost drive battery (audit follow-up). NPCs carry an
    // empty `ShipBoost` so future NPC helm AI can engage boost through
    // the same per-ship pathway the player uses.
    cmds.insert(crate::server_app::ShipBoost::default());
}

struct AiProfileSpawn;
impl SpawnSection for AiProfileSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // AiProfile section — injects AI personality component.
        if let Some(profile) = &config.ai_profile {
            cmds.insert(crate::ai::server::AiProfile {
                aggression: profile.aggression,
                sensor_range: profile.sensor_range,
                low_lod_cruise_fraction: profile.low_lod_cruise_fraction,
                low_lod_speed_decay_per_sec: profile.low_lod_speed_decay_per_sec,
                low_lod_turn_rate_fraction: profile.low_lod_turn_rate_fraction,
            });
        } else {
            // Ships without an [ai_profile] section get a sensible default.
            cmds.insert(crate::ai::server::AiProfile::default());
        }
    }
}

struct LodBubbleSpawn;
impl SpawnSection for LodBubbleSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // LodBubble section — a high-fidelity zone this entity projects (issue: the
        // station being ground down in low-LOD). Authored `[lod_bubble] radius = N`;
        // a player hull that omits it still anchors an implicit default-radius bubble
        // in `lod_ai_ships`, so only a NON-default zone (the station's smaller one)
        // needs the block.
        if let Some(bubble) = &config.lod_bubble {
            cmds.insert(crate::ai::server::LodBubble {
                radius: bubble.radius,
            });
        }
    }
}

struct TagsSpawn;
impl SpawnSection for TagsSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Tags â€” mirror TOML tags onto the entity for snapshot builders.
        if !config.tags.is_empty() {
            cmds.insert(EntityTagsSection(config.tags.clone()));
        }
    }
}

struct RadarAppearanceSpawn;
impl SpawnSection for RadarAppearanceSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Radar appearance section
        if let Some(radar_appearance) = &config.radar_appearance {
            cmds.insert(RadarAppearanceSection(radar_appearance.clone()));
        }
    }
}

struct TargetSpawn;
impl SpawnSection for TargetSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Target section
        if let Some(target) = &config.target {
            cmds.insert(EntityTarget(target.clone()));
        }
    }
}

struct AudioSpawn;
impl SpawnSection for AudioSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Audio section — the local ship's copy drives the host page's sounds.
        if let Some(audio) = &config.audio {
            cmds.insert(ShipAudioSection(audio.clone()));
        }
    }
}

struct FactionSpawn;
impl SpawnSection for FactionSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Faction â€” attach a FactionComponent so the AI can read faction from ECS.
        if let Some(faction_uuid) = config.faction {
            cmds.insert(FactionComponent(faction_uuid));
        }
    }
}

struct WeaponsConsoleSpawn;
impl SpawnSection for WeaponsConsoleSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // WeaponsConsole — attach a WeaponsConsoleSection so the AI can read weapons config from ECS.
        // Also insert PhaserCombatConfigResource and PhaserRenderConfig as per-entity Components
        // (PR 5/gap-review — PRD #597) so NPC ships share the same per-bank arc/range/damage
        // model as the player ship. tick_beams reads these components uniformly.
        if let Some(wc) = &config.weapons_console {
            cmds.insert(WeaponsConsoleSection(wc.clone()));
            // PhaserCombatConfig is built directly from the [[weapons_console.phaser_banks]] list.
            let combat_config =
                crate::entities::config::PhaserCombatConfig::from_weapons_console(wc);
            cmds.insert(crate::console::weapons::PhaserCombatConfigResource(
                combat_config,
            ));
            // Per-bank phaser open-fire AI policies (issue #781): each bank's inline
            // authored `ai` block. A bank that authors none contributes no entry —
            // since #885b stage 5d there is no synthesised fallback, and strict
            // AI-declaration mode rejects an unauthored bank at load. `to_policy`
            // cannot fail here — every authored bank block was validated in
            // `EntityConfig::from_toml`.
            let phaser_bank_policies: std::collections::HashMap<
                String,
                crate::ai::policy::AiPolicy,
            > = wc
                .phaser_banks
                .iter()
                .filter_map(|b| {
                    let ai = b.ai.as_ref()?;
                    Some((b.id.clone(), ai.to_policy().unwrap_or_default()))
                })
                .collect();
            cmds.insert(crate::console::weapons::PhaserBankAiPolicies(
                phaser_bank_policies,
            ));
            // The ship-level WEAPONS DOCTRINE (issue #956): which family this hull
            // turns to bring to bear. Validated in `EntityConfig::from_toml`, so
            // `to_policy` cannot fail here; a hull that authors none attaches no
            // component and asks Helm to turn for nothing, which strict
            // AI-declaration mode makes unreachable for an AI-bearing hull.
            if let Some(ai) = wc.ai.as_ref() {
                cmds.insert(crate::console::weapons::WeaponsDoctrineAiPolicy(
                    ai.to_policy().unwrap_or_default(),
                ));
            }
            // PhaserRenderConfig: take the first bank's beam_color if any, else default.
            let render_config = if let Some(first_bank) = wc.phaser_banks.first() {
                crate::console::weapons::PhaserRenderConfig {
                    beam_color: if first_bank.beam_color.len() == 4 {
                        [
                            first_bank.beam_color[0],
                            first_bank.beam_color[1],
                            first_bank.beam_color[2],
                            first_bank.beam_color[3],
                        ]
                    } else {
                        crate::weapons::beam_render::DEFAULT_BEAM_COLOR
                    },
                    beam_range: if first_bank.beam_range > 0.0 {
                        first_bank.beam_range
                    } else {
                        40.0
                    },
                }
            } else {
                crate::console::weapons::PhaserRenderConfig::default()
            };
            cmds.insert(render_config);
        }
    }
}

struct TorpedoesSpawn;
impl SpawnSection for TorpedoesSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Torpedoes — attach a `TorpedoSystemResource` component when the entity
        // TOML has a `[torpedoes]` block. Mirrors the player-ship insertion in
        // `server_app.rs::spawn_game_start_entities` — NPCs and the player ship
        // now use the same per-entity component (PRD #597 gap-3 closure).
        if let Some(tc) = &config.torpedoes {
            let runtime_config = tc.to_runtime();
            let torpedo_system = if !tc.tubes.is_empty() {
                crate::weapons::torpedo::TorpedoSystem::from_configs(&tc.tubes, runtime_config)
            } else {
                crate::weapons::torpedo::TorpedoSystem::new(runtime_config)
            };
            cmds.insert(crate::console::weapons::TorpedoSystemResource(
                torpedo_system,
            ));

            // Per-tube torpedo load + launch AI policies (issue #782): each tube's
            // inline authored `ai` block. A tube that authors none contributes no
            // entry — strict AI-declaration mode rejects that at load.
            // Validated at load, so `to_policy` cannot fail here.
            let tube_policies: std::collections::HashMap<String, crate::ai::policy::AiPolicy> = tc
                .tubes
                .iter()
                .filter_map(|t| {
                    let ai = t.ai.as_ref()?;
                    Some((t.id.clone(), ai.to_policy().unwrap_or_default()))
                })
                .collect();
            cmds.insert(crate::console::weapons::TorpedoTubeAiPolicies(
                tube_policies,
            ));

            // The shared magazine's grant AI policy (issue #782, AC1): the authored
            // `[torpedoes].ai` block.
            if let Some(ai) = tc.ai.as_ref() {
                cmds.insert(crate::console::weapons::TorpedoMagazineAiPolicy(
                    ai.to_policy().unwrap_or_default(),
                ));
            }
        }
    }
}

struct BlastersSpawn;
impl SpawnSection for BlastersSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Blasters — attach a `BlasterSystemResource` component when the entity
        // TOML has a non-empty `[[weapons_console.blaster_banks]]` list. Mirrors
        // the torpedo insertion above so NPCs and the player ship both participate
        // in the per-entity component model (issue #631 Finding 1).
        if let Some(wc) = &config.weapons_console {
            if !wc.blaster_banks.is_empty() {
                let blaster_systems: Vec<crate::weapons::blaster::BlasterSystem> = wc
                    .blaster_banks
                    .iter()
                    .map(|bc| crate::weapons::blaster::BlasterSystem::new(bc.to_runtime()))
                    .collect();
                cmds.insert(crate::console::weapons::BlasterSystemResource(
                    blaster_systems,
                ));
                // Per-bank blaster open-fire AI policies (issue #781): each bank's
                // inline authored `ai` block. A bank that authors none contributes no
                // entry. Validated at load, so `to_policy` cannot fail.
                let blaster_bank_policies: std::collections::HashMap<
                    String,
                    crate::ai::policy::AiPolicy,
                > = wc
                    .blaster_banks
                    .iter()
                    .filter_map(|b| {
                        let ai = b.ai.as_ref()?;
                        Some((b.id.clone(), ai.to_policy().unwrap_or_default()))
                    })
                    .collect();
                cmds.insert(crate::console::weapons::BlasterBankAiPolicies(
                    blaster_bank_policies,
                ));
            }
        }
    }
}

struct HelmConsoleSpawn;
impl SpawnSection for HelmConsoleSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // HelmConsole - attach a HelmConsoleSection so the AI tick can read movement params.
        // Also insert the four drive-config Components (PR 4 — PRD #597) so NPC ships
        // participate in the per-entity config model alongside the player ship.
        if let Some(hc) = &config.helm_console {
            cmds.insert(HelmConsoleSection(hc.clone()));

            // Physics config
            cmds.insert(crate::ship_plugin::ShipPhysicsConfigResource(
                crate::ship::physics::ShipPhysicsConfig {
                    max_speed: hc.max_speed,
                    max_reverse_speed: hc.max_reverse_speed,
                    acceleration: hc.acceleration,
                    deceleration: hc.deceleration,
                    max_yaw_rate: hc.max_yaw_rate,
                    low_speed_turn_boost: hc.low_speed_turn_boost,
                    max_lateral_speed: hc
                        .lateral_thrust
                        .as_ref()
                        .map(|lt| lt.max_lateral_speed)
                        .unwrap_or(15.0),
                    lateral_acceleration: hc
                        .lateral_thrust
                        .as_ref()
                        .map(|lt| lt.lateral_acceleration)
                        .unwrap_or(15.0),
                    // Vertical axis (issue #744): no dedicated helm_console TOML yet,
                    // so take the ShipPhysicsConfig defaults.
                    ..crate::ship::physics::ShipPhysicsConfig::new()
                },
            ));
            // Impulse config
            // Impulse config — steering_multiplier from [helm_capability] when present,
            // falling back to the const default (0.1) when absent.
            let impulse_steering = config
                .helm_capability
                .as_ref()
                .map(|cap| cap.impulse.steering_multiplier)
                .unwrap_or(crate::ship::impulse::IMPULSE_STEERING_MULTIPLIER_DEFAULT);
            cmds.insert(crate::ship_plugin::ImpulseConfigResource {
                charge_duration: hc.impulse_charge_duration,
                speed_multiplier: hc.impulse_speed_multiplier,
                acceleration_multiplier: hc.impulse_acceleration_multiplier,
                engage_distance: hc.impulse_engage_distance,
                cancel_distance: hc.impulse_cancel_distance,
                steering_multiplier: impulse_steering,
            });
            // Boost config (disabled when [helm_console.boost] is absent)
            let boost_cfg = hc
                .boost
                .as_ref()
                .map(|b| crate::ship_plugin::BoostConfigResource {
                    enabled: true,
                    multiplier: b.multiplier,
                    steering_multiplier: b.steering_multiplier,
                    active_duration: b.active_duration,
                    recharge_duration: b.recharge_duration,
                })
                .unwrap_or_default();
            cmds.insert(boost_cfg);
            // Bank config
            cmds.insert(crate::ship_plugin::BankConfigResource {
                max_bank_deg: hc.max_bank_deg,
                bank_lerp_rate: hc.bank_lerp_rate,
            });
        }
    }
}

struct HelmCapabilitySpawn;
impl SpawnSection for HelmCapabilitySpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // HelmCapability — attach when [helm_capability] is present.
        if let Some(cap) = &config.helm_capability {
            cmds.insert(HelmCapabilitySection(cap.clone()));
        }
    }
}

struct CommsSpawn;
impl SpawnSection for CommsSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Comms range - attach CommsRange component when [comms] is present, and
        // the CommsHailable opt-in marker when that block asks for the hail roster
        // (issue #985). Two components, not one: `range` gates reachability for
        // EVERY comms endpoint, while `hailable` is what puts the entity on the
        // roster the Comms officer can call up.
        if let Some(comms) = &config.comms {
            cmds.insert(crate::comms::CommsRange(comms.range));
            if comms.hailable {
                cmds.insert(crate::comms::CommsHailable {
                    display_name: comms.display_name.clone(),
                });
            }
        }
    }
}

struct ShieldsSpawn;
impl SpawnSection for ShieldsSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Shields — translate the [shields_console] config block + designer
        // `[[shield_arc]]` blocks into a `ShipShields` component with focus
        // tuning. Uses the same code path the player ship uses in
        // spawn_game_start_entities so both player and NPC ships read from one
        // TOML section. Placed BEFORE the hull block: the hull block has an
        // early-return for the empty-hull case, so anything after it could be
        // skipped.
        //
        // The gate is the shield CONTENT, not the `[shields_console]` header. Since
        // #885b every AI-bearing hull authors `[shields_console.ai_policy]`, and a
        // sub-table cannot be written without bringing its parent into existence —
        // so the header no longer distinguishes "this hull has shields" from "this
        // hull declared its shields-focus policy". `ship_requiem_courier.toml` is
        // exactly that shape: no `[[shield_arc]]`, no `[shields_console.base]`, and
        // it must keep having no shield system at all. A `[shields_console]` block
        // that authors neither therefore now means NO shields where it used to mean
        // a default four-facing system; nothing shipped is in that state.
        let shields_content = config
            .shields_console
            .as_ref()
            .filter(|sc| sc.base.is_some() || !config.shield_arcs.is_empty());
        if let Some(sc) = shields_content {
            use crate::weapons::shield::{ShieldFocusConfig, ShieldSystem};
            let ship_wide = sc.base.as_ref().map(|b| b.to_runtime()).unwrap_or_default();
            let shield_system = if !config.shield_arcs.is_empty() {
                let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
                ShieldSystem::from_arcs(&arcs, &ship_wide)
            } else {
                ShieldSystem::new(&ship_wide)
            };
            let freq = config
                .shield_arcs
                .first()
                .map(|a| a.frequency)
                .unwrap_or(sc.frequency);
            let mut shields = crate::ship::shields::ShipShields(shield_system, freq);
            shields.0.focus_config = ShieldFocusConfig {
                bonus_max_hp: sc.focus_bonus_max_hp,
                bonus_regen: sc.focus_bonus_regen,
                penalty_max_hp: sc.focus_penalty_max_hp,
                penalty_regen: sc.focus_penalty_regen,
                decay_rate: sc.focus_decay_rate,
                focused_damage_multiplier: sc.focus_focused_damage_multiplier,
                unfocused_damage_multiplier: sc.focus_unfocused_damage_multiplier,
            };
            cmds.insert(shields);
        } else if !config.shield_arcs.is_empty() {
            // Ships that declare `[[shield_arc]]` blocks without a
            // `[shields_console]` block (some legacy paths). Still build the
            // shield system from arcs, using default focus config.
            use crate::weapons::shield::ShieldSystem;
            let ship_wide = crate::weapons::shield::ShieldConfig::default();
            let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
            let shield_system = ShieldSystem::from_arcs(&arcs, &ship_wide);
            let freq = config
                .shield_arcs
                .first()
                .map(|a| a.frequency)
                .unwrap_or(0.5);
            cmds.insert(crate::ship::shields::ShipShields(shield_system, freq));
        }
    }
}

struct ShieldsDamageHistorySpawn;
impl SpawnSection for ShieldsDamageHistorySpawn {
    fn apply(&self, _config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Shields damage history — per-ship Component tracking HP deltas for the
        // AI damage-concentration algorithm. Initialised empty; resized lazily.
        cmds.insert(crate::ship::shields::ShieldsDamageHistory::default());
    }
}

struct ArcHullSpawn;
impl SpawnSection for ArcHullSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Per-arc hull HP (issue #514) — populated from `[[shield_arc]].hull_max_hp`
        // and companion threshold/debuff fields. Attaches `EntityShipArcHull`
        // alongside the shield system so `sync_console_damage_tiers` can route arc
        // damage → offline_systems per-arc. Skipped when no arc declares hull HP.
        if !config.shield_arcs.is_empty() {
            let arc_entries: Vec<(String, crate::ship::damage::ArcHullEntry)> = config
                .shield_arcs
                .iter()
                .filter(|a| a.hull_max_hp > 0.0)
                .map(|a| {
                    (
                        a.id.clone(),
                        crate::ship::damage::ArcHullEntry {
                            current: a.hull_max_hp,
                            max: a.hull_max_hp,
                            tier_config: crate::ship::damage::ConsoleTierConfig {
                                damaged_threshold_pct: a.hull_damaged_threshold_pct,
                                disabled_threshold_pct: a.hull_disabled_threshold_pct,
                                debuff_magnitude: a.hull_debuff_magnitude,
                            },
                        },
                    )
                })
                .collect();
            if !arc_entries.is_empty() {
                cmds.insert(EntityShipArcHull(
                    crate::ship::damage::ShipArcHull::from_entries(arc_entries),
                ));
            }
        }
    }
}

struct InfrastructureSpawn;
impl SpawnSection for InfrastructureSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Infrastructure condition + capacity (issue #1025) — attach the track when
        // `[infrastructure]` is present. Placed BEFORE the hull block for the same
        // reason the shields block is: the hull block has an early return for the
        // empty-hull case, and anything after it could be skipped.
        if let Some(infrastructure) = &config.infrastructure {
            cmds.insert(crate::infrastructure::InfrastructureCondition(
                crate::infrastructure::InfrastructureState::from_config(infrastructure),
            ));
        }
    }
}

struct TractorSpawn;
impl SpawnSection for TractorSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // The tractor beam (issue #1156) — attach the beam when `[tractor]` is
        // present, on the same argument again. The power group is read from the
        // tractor `[[system]]` block (its single authored source), so the component
        // is self-contained after spawn and the tick never re-walks the systems
        // list. `EntityConfig` validation already guaranteed the paired system with a
        // power group exists, so the resolve below cannot silently drop the beam on a
        // hull that authored it; the belt-and-braces `if let` only guards a
        // component-less spawn path.
        if let Some(tractor) = &config.tractor {
            if let Some(power_group) = config.ship_config.as_ref().and_then(|sc| {
                sc.systems
                    .iter()
                    .find(|s| s.kind == crate::ship::system_registry::TRACTOR_KIND)
                    .and_then(|s| s.power_group.clone())
            }) {
                cmds.insert(crate::tractor::TractorBeam::new(
                    tractor.clone(),
                    power_group,
                ));
            }
        }
    }
}

struct ExternalRepairDispatchSpawn;
impl SpawnSection for ExternalRepairDispatchSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // External repair-team dispatch (issue #1161) — attach the record when
        // `[repair.external_dispatch]` is authored, so the repair console can send a
        // team to a nearby ally or structure. Placed HERE, outside the `[behaviour]`
        // gate above, on the tractor's argument: the capability belongs to any hull
        // that authors it, player or NPC, and a behaviour-less player hull would miss
        // it inside that gate (the same footgun `server_app` re-spells `ShipRepairTeams`
        // for). A hull that authors no dispatch table carries no component and cannot
        // dispatch abroad — unchanged in every way. `EntityConfig` validation already
        // rejected a non-positive reach or rate, so the clone below is usable.
        if let Some(external) = config
            .repair
            .as_ref()
            .and_then(|rc| rc.external_dispatch.as_ref())
        {
            cmds.insert(
                crate::console::repair::external_server::ExternalRepairDispatch::new(
                    external.clone(),
                ),
            );
        }
    }
}

struct HeldResponseSpawn;
impl SpawnSection for HeldResponseSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // The held-response (issue #1158) — attach when `[held_response]` is
        // present, on a TARGET entity. It says what being held DOES to this thing;
        // the tractor server reads it off whatever it is holding. An entity that
        // authors nothing carries no component and is merely held in place.
        if let Some(held_response) = &config.held_response {
            cmds.insert(crate::tractor::HeldResponseSection(held_response.clone()));
        }
    }
}

struct DockSpawn;
impl SpawnSection for DockSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Docking (issue #1159) — a hull opts into docking with a `[dock]` table.
        // Its presence, and ONLY its presence, triggers the one spawn-time read of
        // the model rig sidecar for `dock`-prefixed markers, so a world whose hulls
        // author no `[dock]` reads no sidecar here and its `content_digest` is
        // unchanged. Two components come out of it:
        //
        //   * `DockMarkers` — the dock markers resolved from the rig sidecar into the
        //     hull's own frame — on any hull with a `[dock]` table AND dock markers.
        //     This is what makes a hull DOCKABLE (a passive berth needs only this).
        //   * `DockControl` — the live dock control — additionally on a hull whose
        //     `[[system]] kind = "dock"` gives the dock a power group and a station,
        //     making it an ACTIVE docker. `EntityConfig` validation already paired
        //     the two, so the resolve below cannot silently drop the control.
        if let Some(dock) = &config.dock {
            if let Some(mesh) = &config.mesh {
                if let Some(model) = mesh.model.as_deref() {
                    if let Some(rig) = crate::entities::glb_visual::resolve_sidecar_rig(
                        model,
                        mesh.variant.as_deref(),
                    ) {
                        let markers = crate::dock::resolve_dock_markers(&rig);
                        if !markers.is_empty() {
                            cmds.insert(markers);
                        }
                    }
                }
            }
            if let Some((system_id, power_group)) = config.ship_config.as_ref().and_then(|sc| {
                sc.systems
                    .iter()
                    .find(|s| s.kind == crate::ship::system_registry::DOCK_KIND)
                    .and_then(|s| {
                        s.power_group
                            .clone()
                            .map(|power_group| (s.id.clone(), power_group))
                    })
            }) {
                cmds.insert(crate::dock::DockControl::new(
                    system_id,
                    dock.clone(),
                    power_group,
                ));
            }
        }
    }
}

struct UmbilicalSpawn;
impl SpawnSection for UmbilicalSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // The transfer umbilical (issue #1160) — attach the umbilical when
        // `[umbilical]` is present, on the same argument as the tractor. The power
        // group is read from the umbilical `[[system]]` block (its single authored
        // source), so the component is self-contained after spawn and the tick never
        // re-walks the systems list. `EntityConfig` validation already guaranteed the
        // paired system with a power group exists, so the resolve below cannot
        // silently drop the umbilical on a hull that authored it; the belt-and-braces
        // `if let` only guards a component-less spawn path.
        if let Some(umbilical) = &config.umbilical {
            if let Some(power_group) = config.ship_config.as_ref().and_then(|sc| {
                sc.systems
                    .iter()
                    .find(|s| s.kind == crate::ship::system_registry::UMBILICAL_KIND)
                    .and_then(|s| s.power_group.clone())
            }) {
                cmds.insert(crate::umbilical::TransferUmbilical::new(
                    umbilical.clone(),
                    power_group,
                ));
            }
        }
    }
}

struct ScanSpawn;
impl SpawnSection for ScanSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // The science scan (issue #1032) — attach the record when `[scan]` is
        // present, on the same argument again. The record carries the authored
        // fidelity ladder AND the last reading: a hull that can scan starts able to
        // and having read nothing.
        if let Some(scan) = &config.scan {
            cmds.insert(crate::science::ShipScanRecord {
                config: scan.clone(),
                ..Default::default()
            });
        }
    }
}

struct CivilianSpawn;
impl SpawnSection for CivilianSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Civilian traffic (issue #1028) — attach the authored assignment and the
        // live route/order/compliance state when `[civilian]` is present. Same
        // placement argument as the infrastructure block above. The pair is
        // deliberate: the section never changes after spawn, the traffic state is
        // the only half a save has to carry.
        if let Some(civilian) = &config.civilian {
            cmds.insert((
                crate::civilian::CivilianSection(civilian.clone()),
                crate::civilian::CivilianTraffic(crate::civilian::CivilianState::from_config(
                    civilian,
                )),
            ));
        }
    }
}

struct HullSpawn;
impl SpawnSection for HullSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Hull -- attach an EntitySystemHull component if the config has hull data.
        // Per-system entries take precedence; if absent we fall back to the
        // legacy scalar `hull_integrity` value mapped to a single `SystemId("captain")`
        // slot (used by simple entities like asteroids and station spawns).
        if let Some(hull) = &config.hull {
            let system_hull: crate::ship::damage::SystemHull = if !hull.system_hull.is_empty() {
                // Explicit `[[hull.system_hull]]` entries — new authoring path.
                let entries: Vec<(
                    crate::core::messages::SystemId,
                    String,
                    f32,
                    crate::ship::damage::ConsoleTierConfig,
                )> = hull
                    .system_hull
                    .iter()
                    .map(|e| {
                        let display = e
                            .display_name
                            .clone()
                            .unwrap_or_else(|| e.system_id.0.clone());
                        (
                            e.system_id.clone(),
                            display,
                            e.max_hp,
                            crate::ship::damage::ConsoleTierConfig {
                                damaged_threshold_pct: e.damaged_threshold_pct,
                                disabled_threshold_pct: e.disabled_threshold_pct,
                                debuff_magnitude: e.debuff_magnitude,
                            },
                        )
                    })
                    .collect();
                crate::ship::damage::SystemHull::from_config_with_display_names(entries)
            } else if hull.hull_integrity > 0.0 {
                crate::ship::damage::SystemHull::from_config(&[(
                    crate::core::messages::SystemId("captain".to_string()),
                    hull.hull_integrity,
                )])
            } else {
                // Empty hull section — skip.
                cmds.insert(EntitySystemHull(crate::ship::damage::SystemHull::default()));
                return;
            };
            cmds.insert(EntitySystemHull(system_hull));
        }
    }
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "spawner_tests.rs"]
mod tests;
