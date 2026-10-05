use crate::entities::config::{AsteroidFieldConfig, LightConfig, StarConfig};
use crate::regions::{effects::RegionEffectKind, shape::RegionShape};
use bevy::prelude::*;
pub use phoenix_sim_contracts::identity::*;
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
/// `crates/phoenix-simulation/src/console/weapons/mod.rs`, matches `Or<(With<Ship>, With<StaticPointDefence>)>`) —
/// an unfactioned one stays invisible only because the faction gate
/// (`is_hostile` / `faction::is_enemy`) requires a `FactionComponent` on
/// both sides.
#[derive(Component, Clone, Debug)]
pub struct StaticPointDefence;

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
