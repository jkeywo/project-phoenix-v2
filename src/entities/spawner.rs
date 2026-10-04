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

/// Canonical entity-template path that produced this live entity.
///
/// Unlike [`EntitySpawnOrigin`], this identity is present for authored and
/// runtime spawns alike. Live inspection must never recover provenance by
/// comparing resolved configs: two templates may intentionally resolve to the
/// same ship topology while remaining different authored sources.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct EntityTemplatePath(pub String);

impl EntityTemplatePath {
    pub fn new(path: &str) -> Self {
        Self(crate::entities::include_resolve::canonical_template_path(
            path,
        ))
    }
}

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
/// Its primary model/variant and authored parent transform are simulation
/// content used by the renderer-independent marker loader; the remaining shape
/// and material fields drive 3-D viewscreen presentation.
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
    spawn_entity_with_ship_seed(commands, config, position, uuid, id, None)
}

/// Spawn with crew/topology inputs already resolved by the fleet adapter.
pub(crate) fn spawn_entity_with_ship_seed(
    commands: &mut Commands,
    config: &EntityConfig,
    position: Vec3,
    uuid: String,
    id: Option<String>,
    seed: Option<super::ship_spawn::ShipSpawnSeed>,
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

    super::ship_spawn::install(config, position, seed, &mut entity_commands);
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
// Ship capabilities are installed together after these independent entity sections.
// Keep this traversal stable; determinism is verified across archetype layouts.
trait SpawnSection {
    fn apply(&self, config: &EntityConfig, position: Vec3, cmds: &mut EntityCommands);
}

/// Independent entity sections, in stable insertion order.
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
    &SensorsObservationSpawn,
    &FactionSpawn,
    &HelmCapabilitySpawn,
    &CommsSpawn,
    &ShieldsDamageHistorySpawn,
    &InfrastructureSpawn,
    &TractorSpawn,
    &ExternalRepairDispatchSpawn,
    &HeldResponseSpawn,
    &DockSpawn,
    &UmbilicalSpawn,
    &SecuritySpawn,
    &SecurityTargetSpawn,
    &TransporterSpawn,
    &CivilianRescueSpawn,
    &DemolitionTargetSpawn,
    &ScanSpawn,
    &DebrisSpawn,
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
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        if let Some(behaviour) = &config.behaviour {
            cmds.insert((
                BehaviourSection(behaviour.clone()),
                crate::gm_npc::NpcDoctrineState::default(),
            ));
        }
        if config.is_static_point_defence() {
            cmds.insert(StaticPointDefence);
        }
    }
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

struct SensorsObservationSpawn;
impl SpawnSection for SensorsObservationSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        if let Some(sensors) = &config.sensors_console {
            cmds.insert(crate::gm_information::reports::SensorsObservationConfig(
                sensors.long_range_radar.clone(),
            ));
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

struct ShieldsDamageHistorySpawn;
impl SpawnSection for ShieldsDamageHistorySpawn {
    fn apply(&self, _config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Shields damage history — per-ship Component tracking HP deltas for the
        // AI damage-concentration algorithm. Initialised empty; resized lazily.
        cmds.insert(crate::ship::shields::ShieldsDamageHistory::default());
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

struct TransporterSpawn;
impl SpawnSection for TransporterSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // The rescue transporter (issue #1348) — attach when `[transporter]` is
        // present, the tractor's argument exactly. The power group is read from
        // the transporter `[[system]]` block (its single authored source);
        // `EntityConfig` validation already guaranteed the paired system with a
        // power group exists, so the resolve below cannot silently drop it.
        if let Some(transporter) = &config.transporter {
            if let Some(power_group) = config.ship_config.as_ref().and_then(|sc| {
                sc.systems
                    .iter()
                    .find(|s| s.kind == crate::ship::system_registry::TRANSPORTER_KIND)
                    .and_then(|s| s.power_group.clone())
            }) {
                cmds.insert(crate::transporter::Transporter::new(
                    transporter.clone(),
                    power_group,
                ));
            }
        }
    }
}

struct CivilianRescueSpawn;
impl SpawnSection for CivilianRescueSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // The civilians a contact carries (issue #1348) — attach when
        // `[civilian_rescue]` is present, on a TARGET entity. A contact that
        // authors nothing carries no component and offers no rescue.
        if let Some(civilian_rescue) = &config.civilian_rescue {
            cmds.insert(crate::transporter::CivilianRescue::new(
                civilian_rescue.count,
            ));
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
                    if let Some(rig) = crate::entities::model_markers::resolve_sidecar_rig(
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

struct SecuritySpawn;
impl SpawnSection for SecuritySpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // Security teams (issue #1346) — attach the muster when `[security]` is
        // present, on the same argument as the umbilical. Which STATION owns the
        // system rides the `[[system]]` block and is read by admission, not by the
        // component, so nothing about the teams themselves needs it here.
        // `EntityConfig` validation already guaranteed the paired system exists.
        if let Some(security) = &config.security {
            cmds.insert(crate::security::ShipSecurityTeams::new(security.clone()));
            // A hull that musters Security teams is the one that fires the charges
            // they place (issue #1350): `DetonateCharges` goes to the `security`
            // system, so the demolition refusal projection rides the same hull.
            // It holds no authoritative state — charged/detonated are world flags —
            // so a hull with no demolition target in its world simply never has a
            // refusal to show.
            cmds.insert(crate::demolition::DemolitionControl::default());
        }
    }
}

struct SecurityTargetSpawn;
impl SpawnSection for SecurityTargetSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // What a Security team may be sent HERE to do (issue #1346). Independent
        // of `[security]` above: an entity that offers work needs no teams of its
        // own, and a hull with teams need offer none.
        if let Some(target) = &config.security_target {
            cmds.insert(crate::security::SecurityTargetActions(target.clone()));
        }
    }
}

struct DemolitionTargetSpawn;
impl SpawnSection for DemolitionTargetSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // What a controlled demolition may do HERE (issue #1350). Independent of
        // `[security_target]` above, though on the Falling Skyway obstruction the
        // two ride the same entity: the `place_charges` action arms the charges,
        // and this table says what detonating them clears.
        if let Some(target) = &config.demolition_target {
            cmds.insert(crate::demolition::DemolitionTarget(target.clone()));
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

struct DebrisSpawn;
impl SpawnSection for DebrisSpawn {
    fn apply(&self, config: &EntityConfig, _position: Vec3, cmds: &mut EntityCommands) {
        // The moving hazard (issue #1347) — attach the contact when `[debris]` is
        // present, on the same argument as the scan record above. The component
        // carries the authored table AND this run's history of the contact: it
        // starts drifting, unread, and unconfirmed, which is the whole point —
        // a rock is not a threat until somebody has been and looked at it.
        if let Some(debris) = &config.debris {
            cmds.insert(crate::debris::DebrisThreat::new(debris.clone()));
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
