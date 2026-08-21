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

/// Present when the EntityConfig has a [planet] section.
#[derive(Component, Clone, Debug)]
pub struct PlanetSection(pub crate::entity_config::PlanetConfig);

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
pub struct WeaponsConsoleSection(pub crate::entity_config::WeaponsConsoleConfig);

/// Present when the EntityConfig has a `[helm_console]` section.
/// The AI tick reads this to build a `ShipPhysicsConfig` instead of using hardcoded defaults.
#[derive(Component, Clone, Debug)]
pub struct HelmConsoleSection(pub crate::entity_config::HelmConsoleConfig);

/// Present when the EntityConfig has a `[helm_capability]` section.
/// Describes vertical movement mode and impulse steering policy.
#[derive(Component, Clone, Debug)]
pub struct HelmCapabilitySection(pub crate::entity_config::HelmCapabilityConfig);

/// Present when the EntityConfig had a [radar_appearance] section.
#[derive(Component, Clone, Debug)]
pub struct RadarAppearanceSection(pub crate::entity_config::RadarAppearanceConfig);

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
pub struct EntityTarget(pub crate::entity_target::TargetSection);

/// Present when the EntityConfig has a `[cinematic_camera]` section.
/// The viewscreen reads this for cinematic camera positioning and tracking.
#[derive(Component, Clone, Debug)]
pub struct CinematicCameraSection(pub crate::entity_config::CinematicCameraConfig);

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
pub struct EntitySystemHull(pub crate::damage::SystemHull);

/// Bevy ECS component wrapping the pure [`crate::damage::ShipArcHull`]
/// struct (issue #514). Attached to ship entities that declare
/// `[[shield_arc]]` blocks with `hull_max_hp` fields. `ship/damage.rs` is
/// Bevy-free per AGENTS.md rule 9, so the pure per-arc HP logic lives
/// there and this component wraps it for ECS storage.
///
/// The rest of the codebase uses the type alias
/// [`crate::damage::ShipArcHull`] for readability at call sites — this
/// wrapper is a thin newtype that lets the pure struct participate in
/// Bevy queries.
#[derive(Component, Clone, Debug, Default)]
pub struct EntityShipArcHull(pub crate::damage::ShipArcHull);

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

    // Collider section â†’ Rapier collider + rigid body
    if let Some(collider) = &config.collider {
        let rapier_collider = match collider.shape {
            crate::entity_config::ColliderShape::Ball => Collider::ball(collider.radius),
            crate::entity_config::ColliderShape::Capsule => {
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
            crate::entity_config::ColliderShape::Cylinder => Collider::cylinder(
                collider.half_height.unwrap_or(collider.radius),
                collider.radius,
            ),
        };
        entity_commands.insert((
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

    // Planet section
    if let Some(planet) = &config.planet {
        entity_commands.insert(PlanetSection(planet.clone()));
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

    // Cinematic camera section
    if let Some(cam) = &config.cinematic_camera {
        entity_commands.insert(CinematicCameraSection(cam.clone()));
    }

    // Behaviour section — the "this entity is AI-driven" predicate; ai_plugin
    // registers an AI token for any entity carrying it.
    let static_point_defence = config.is_static_point_defence();
    if config.behaviour.is_some() || static_point_defence {
        if let Some(behaviour) = &config.behaviour {
            entity_commands.insert(BehaviourSection(behaviour.clone()));
        }
        if static_point_defence {
            entity_commands.insert(StaticPointDefence);
        }
        entity_commands.insert(crate::server_app::ShipSystemBlackboards::default());

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
        let (resolver, active_ratings) =
            crate::ship::rating::seed_boot_ratings(&ship_config.0, |_| {
                crate::ship::rating::BACKFILL_RATING.to_string()
            });
        // Seed the reactor from the ship's authored power groups (issue #762)
        // BEFORE `ship_config` is moved into the entity, so authored groups
        // beyond the canonical three (e.g. `ops`) are allocatable rather than
        // returning `UnknownGroup`. Empty for ships with no `[power_groups.*]`.
        let power_group_seed =
            crate::power_plugin::authored_power_group_seed(&ship_config.0.power_groups);
        // Seed ShipPhysics from the spawn position so the per-entity helm loop
        // starts with the correct initial state rather than (0, 0).
        let ship_physics = crate::ship_state::ShipPhysics {
            x: position.x,
            z: position.z,
            yaw: {
                let rot = bevy::math::Quat::from_euler(bevy::math::EulerRot::YXZ, 0.0, 0.0, 0.0);
                let _ = rot;
                0.0 // initial yaw; updated each tick by integrate_ship_physics
            },
            ..Default::default()
        };
        entity_commands.insert((
            ship_config,
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ShipSystemControlSources(resolver),
            crate::ship_plugin::ActiveStationRatings(active_ratings),
            crate::ship_plugin::CoordinationQueue::default(),
            ship_physics,
            crate::ship_plugin::HelmWaypointClearance::default(),
            crate::weapons_plugin::TacticalRadarSelection::default(),
            crate::weapons_plugin::ActiveBeam::default(),
            crate::weapons_plugin::PhaserCooldown::default(),
            crate::sensors_plugin::SensorRadarSelection::default(),
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
                crate::ship_state::ShipRedAlert::default(),
                crate::ship_state::ShipWeaponsHold::default(),
            ),
            crate::ship_state::ShipViewMode::default(),
            crate::ship_state::ShipPhaserFrequency::default(),
            crate::navigation_plugin::NavigationWaypoint::default(),
        ));
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
        entity_commands.insert((
            crate::ship_plugin::LastHelmInput::default(),
            crate::command_plugin::ShipStationStances::default(),
            // Edge-detection scratch for the persist-behind-human trigger
            // (issue #1108). Transient and NOT folded into the sim digest —
            // see the type's own docs; a fresh/reloaded hull records its first
            // observation and fires no edge.
            crate::command_plugin::LastDirectedControl::default(),
        ));
        // Per-objective route cursors: where this ship is on each objective's
        // route. Read by the low-LOD `simulate_low_lod_ships` path, the high-LOD
        // `helm_patrol`, and `operate_navigation_ai`; written only by
        // `advance_objective_cursors`. A ship without one cannot patrol.
        // Inserted separately to stay under Bevy's tuple-Bundle element cap.
        entity_commands.insert(crate::ai_plugin::ObjectiveCursors::default());
        // Per-ship coordination bus state (audit follow-up). Every ship
        // tracks its own shields down/restore notification cycle and its
        // own sensors→tactical frequency-hint dedupe state so the two
        // coordination emitters (`emit_shields_coordination`,
        // `tick_sensors_frequency_hint`) can iterate `With<Ship>` and
        // route into each ship's own `CoordinationQueue` via
        // `CoordinationEnqueue.source_entity`.
        entity_commands.insert(crate::ship::shields::ShieldsCoordinationState::default());
        entity_commands.insert(crate::ship::sensors::SensorsFrequencyState::default());
        entity_commands.insert(crate::ship::sensors::SensorsThreatState::default());
        // Power brownout advisory debounce state (issue #678): per-ship
        // so each ship tracks its own brownout notification cycle.
        entity_commands.insert(crate::ship::power::PowerBrownoutState::default());
        // Weapons->Helm arc-bearing request state (issue #677): per-ship
        // debounce for the channel-3 request, and the pending bearing Helm
        // AI folds into its steering once the request is consumed.
        entity_commands.insert(crate::weapons_plugin::WeaponsArcRequestState::default());
        entity_commands.insert(crate::ship_plugin::PendingArcBearingRequest::default());
        // Distinct docking intent (issue #742): the sanctioned home for
        // controlled reverse / lateral close manoeuvres, kept separate from the
        // facing-only arc-bearing request above.
        entity_commands.insert(crate::ship_plugin::DockingMotionIntent::default());
        entity_commands.insert(crate::ship::shields::PendingShieldsThreatBearing::default());
        // Sensors→Tactical frequency advisory a backfilled Tactical consumes
        // off the channel-3 bus (issue #873).
        entity_commands.insert(crate::ship_plugin::PendingTacticalFrequencyHint::default());
        // Per-ship intent-narration memory (issue #879): the previous decision
        // snapshot of each narrating seat plus this ship's advisory counter.
        // Belongs to the ship rather than to its LOD tier — a demoted hull
        // still has seats to narrate for if a human ever takes one.
        entity_commands.insert(crate::ship_plugin::ShipIntentNarration::default());
        entity_commands.insert(crate::ship_plugin::LastSystemTiers::default());
        entity_commands.insert(crate::ship_plugin::RepairHumanAlerted::default());
        entity_commands.insert(crate::console::repair::server::RepairRequestQueue::default());
        entity_commands.insert(crate::power_plugin::ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::from_authored_groups(
                &crate::modifiers::power_system::PowerConfig::default(),
                &power_group_seed,
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
            Some(pc) => crate::power_plugin::PowerConfigResource(
                crate::modifiers::power_system::PowerConfig {
                    capacity: pc.capacity,
                    rates: pc.rates,
                    sustainable_total: pc.sustainable_total,
                    max_commanded_total: pc.max_commanded_total,
                    emergency_threshold: pc.emergency_threshold,
                },
            ),
            None => crate::power_plugin::PowerConfigResource::default(),
        };
        entity_commands.insert(crate::power_plugin::ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::from_authored_groups(
                &power_config.0,
                &power_group_seed,
            ),
        ));
        entity_commands.insert(power_config);
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
            entity_commands.insert((
                crate::power_plugin::PowerAiPolicy(ai.to_policy().unwrap_or_default()),
                // Carried from the SAME authored block (issue #889's
                // evaluate_every_ticks, wired at runtime): a resolved
                // `AiPolicy` alone forgets this field, so it rides alongside
                // as a sibling component.
                crate::power_plugin::PowerAiCadence(ai.evaluate_every_ticks),
            ));
        }
        // Per-entity power multipliers. Seeded from any per-console TOML
        // `power_multipliers` blocks (helm_console/weapons_console/shields_console)
        // and otherwise defaulted so NPC ships still get MaxSpeed / PhaserDamage
        // / ShieldRegen bonuses translated by `translate_power_modifiers`.
        //
        // After issue #617 the map is keyed by `PowerGroupId`.
        let defaults = [-0.5f32, 0.0, 0.25, 0.5];
        let mut multipliers: std::collections::HashMap<crate::messages::PowerGroupId, [f32; 4]> =
            std::collections::HashMap::from([
                (
                    crate::messages::PowerGroupId(
                        crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                    ),
                    defaults,
                ),
                (
                    crate::messages::PowerGroupId(
                        crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                    ),
                    defaults,
                ),
                (
                    crate::messages::PowerGroupId(
                        crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
                    ),
                    defaults,
                ),
            ]);
        if let Some(hc) = &config.helm_console {
            if let Some(pm) = hc.power_multipliers {
                multipliers.insert(
                    crate::messages::PowerGroupId(
                        crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                    ),
                    pm,
                );
            }
        }
        if let Some(wc) = &config.weapons_console {
            if let Some(pm) = wc.power_multipliers {
                multipliers.insert(
                    crate::messages::PowerGroupId(
                        crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                    ),
                    pm,
                );
            }
        }
        if let Some(sc) = &config.shields_console {
            if let Some(pm) = sc.power_multipliers {
                multipliers.insert(
                    crate::messages::PowerGroupId(
                        crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
                    ),
                    pm,
                );
            }
        }
        // Sensors AI config — loaded from [sensors_console.ai] if present,
        // otherwise the parse-time default. Inserted for EVERY entity that
        // carries a `[behaviour]` block (i.e. every ship spawned through this
        // path); it used to be inserted only when the TOML also carried the
        // `.ai` sub-section, and `tick_frequency_hint_high_fidelity` then fell back to the
        // global Resource. The player ship does not come through here at all —
        // `server_app::spawn_game_start_entities` attaches its copy.
        entity_commands.insert(
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
            entity_commands.insert(crate::ship::sensors::SensorsTargetSelector {
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
            entity_commands.insert(crate::weapons_plugin::TacticalTargetSelector {
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
            entity_commands.insert(crate::console::navigation::NavigationTargetSelector {
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
            entity_commands.insert(crate::console::repair::server::RepairTargetSelector {
                selector: s.to_selector().unwrap_or_default(),
                power_rating: config.power_rating.map(|r| r as f32),
            });
        }
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
            entity_commands.insert(sel);
        }
        if let Some(policy) = comms_response_policy {
            entity_commands.insert(policy);
        }
        if let Some(cadence) = comms_response_cadence {
            entity_commands.insert(cadence);
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
        entity_commands.insert(
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
            entity_commands.insert(crate::ship::shields::ShieldsFocusAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        // Captain AI policy (issue #775) — the inline stateless Red Alert
        // policy from the authored `[captain_console.ai]` block, so
        // `operate_captain_ai` reads a data-authored policy rather than a
        // hardcoded controller.
        if let Some(ai) = config.captain_console.as_ref().and_then(|c| c.ai.as_ref()) {
            entity_commands.insert(crate::captain_plugin::CaptainAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        // Helm Engines/Steering AI policies (issue #779) — the inline
        // longitudinal/yaw policies from the authored `[helm_console.engines_ai]`
        // / `[helm_console.steering_ai]` blocks, so `ai_helm_thrust` /
        // `ai_helm_steering` resolve a data-authored mode verb rather than
        // actuating unconditionally.
        if let Some(ai) = config
            .helm_console
            .as_ref()
            .and_then(|h| h.engines_ai.as_ref())
        {
            entity_commands.insert(crate::ship::helm_ai::HelmEnginesAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        if let Some(ai) = config
            .helm_console
            .as_ref()
            .and_then(|h| h.steering_ai.as_ref())
        {
            entity_commands.insert(crate::ship::helm_ai::HelmSteeringAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        // Helm secondary-actuator AI policies (issue #780): lateral / vertical /
        // impulse / boost, from their authored `[helm_console.*_ai]` blocks, so
        // the secondary hosts resolve a data-authored mode verb rather than a
        // hardcoded branch.
        if let Some(ai) = config
            .helm_console
            .as_ref()
            .and_then(|h| h.lateral_ai.as_ref())
        {
            entity_commands.insert(crate::ship::helm_ai::HelmLateralAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        if let Some(ai) = config
            .helm_console
            .as_ref()
            .and_then(|h| h.vertical_ai.as_ref())
        {
            entity_commands.insert(crate::ship::helm_ai::HelmVerticalAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        if let Some(ai) = config
            .helm_console
            .as_ref()
            .and_then(|h| h.impulse_ai.as_ref())
        {
            entity_commands.insert(crate::ship::helm_ai::HelmImpulseAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        if let Some(ai) = config
            .helm_console
            .as_ref()
            .and_then(|h| h.boost_ai.as_ref())
        {
            entity_commands.insert(crate::ship::helm_ai::HelmBoostAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        entity_commands.insert(crate::power_plugin::PowerMultiplierResource { multipliers });
        // ShipModifiers as per-entity component (PR 6/9 — PRD #597). Every ship
        // gets an empty modifier cache. Region-entry observers and
        // translate_power_modifiers write to the subject entity's cache;
        // translate_impulse_modifiers remains LocalShip-only (ShipImpulse is a
        // player-only mechanic).
        entity_commands.insert(crate::modifiers::ShipModifiers::new());
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
                entity_commands.insert(crate::console::repair::server::ShipRepairTeams(
                    crate::repair_teams::RepairTeams::new_with_timings(
                        repair_cfg.repair_team_count as usize,
                        timings,
                    ),
                ));
            }
        }
        // All ship entities carry the Ship marker — player and NPC alike.
        // The LocalShip marker (not set here) is the viewscreen selector only.
        entity_commands.insert(crate::server_app::Ship);
        // Per-entity CollisionCooldown so NPC ships have their own collision
        // cooldown timer (PRD #597 PR-8). Player ship gets one in
        // `spawn_game_start_entities`.
        entity_commands.insert(crate::server_app::CollisionCooldown::default());
        // Per-entity combat activity trackers (PRD #597 PR-10). Every ship
        // (player + NPC) records its own recent damage/hostile-fire/weapon
        // fire and last attacker.
        entity_commands.insert(crate::ship::combat_activity::RecentCombatActivity::default());
        entity_commands.insert(crate::server_app::WeaponFiredThisTick::default());
        entity_commands.insert(crate::server_app::ShipAttackedThisTick::default());
        entity_commands.insert(crate::weapons_plugin::LastShipAttacker::default());
        // Per-ship impulse drive state (audit follow-up). NPCs carry an
        // idle `ShipImpulse` so `handle_blocks_impulse_region_enter` can
        // route per-subject and future NPC helm AI can toggle impulse
        // through the same per-ship pathway the player uses.
        entity_commands.insert(crate::server_app::ShipImpulse::default());
        // Per-ship boost drive battery (audit follow-up). NPCs carry an
        // empty `ShipBoost` so future NPC helm AI can engage boost through
        // the same per-ship pathway the player uses.
        entity_commands.insert(crate::server_app::ShipBoost::default());
    }

    // AiProfile section — injects AI personality component.
    if let Some(profile) = &config.ai_profile {
        entity_commands.insert(crate::ai_plugin::AiProfile {
            aggression: profile.aggression,
            sensor_range: profile.sensor_range,
            low_lod_cruise_fraction: profile.low_lod_cruise_fraction,
            low_lod_speed_decay_per_sec: profile.low_lod_speed_decay_per_sec,
            low_lod_turn_rate_fraction: profile.low_lod_turn_rate_fraction,
        });
    } else {
        // Ships without an [ai_profile] section get a sensible default.
        entity_commands.insert(crate::ai_plugin::AiProfile::default());
    }

    // LodBubble section — a high-fidelity zone this entity projects (issue: the
    // station being ground down in low-LOD). Authored `[lod_bubble] radius = N`;
    // a player hull that omits it still anchors an implicit default-radius bubble
    // in `lod_ai_ships`, so only a NON-default zone (the station's smaller one)
    // needs the block.
    if let Some(bubble) = &config.lod_bubble {
        entity_commands.insert(crate::ai_plugin::LodBubble {
            radius: bubble.radius,
        });
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

    // Audio section — the local ship's copy drives the host page's sounds.
    if let Some(audio) = &config.audio {
        entity_commands.insert(ShipAudioSection(audio.clone()));
    }

    // Faction â€” attach a FactionComponent so the AI can read faction from ECS.
    if let Some(faction_uuid) = config.faction {
        entity_commands.insert(FactionComponent(faction_uuid));
    }

    // WeaponsConsole — attach a WeaponsConsoleSection so the AI can read weapons config from ECS.
    // Also insert PhaserCombatConfigResource and PhaserRenderConfig as per-entity Components
    // (PR 5/gap-review — PRD #597) so NPC ships share the same per-bank arc/range/damage
    // model as the player ship. tick_beams reads these components uniformly.
    if let Some(wc) = &config.weapons_console {
        entity_commands.insert(WeaponsConsoleSection(wc.clone()));
        // PhaserCombatConfig is built directly from the [[weapons_console.phaser_banks]] list.
        let combat_config = crate::entity_config::PhaserCombatConfig::from_weapons_console(wc);
        entity_commands.insert(crate::weapons_plugin::PhaserCombatConfigResource(
            combat_config,
        ));
        // Per-bank phaser open-fire AI policies (issue #781): each bank's inline
        // authored `ai` block. A bank that authors none contributes no entry —
        // since #885b stage 5d there is no synthesised fallback, and strict
        // AI-declaration mode rejects an unauthored bank at load. `to_policy`
        // cannot fail here — every authored bank block was validated in
        // `EntityConfig::from_toml`.
        let phaser_bank_policies: std::collections::HashMap<String, crate::ai::policy::AiPolicy> =
            wc.phaser_banks
                .iter()
                .filter_map(|b| {
                    let ai = b.ai.as_ref()?;
                    Some((b.id.clone(), ai.to_policy().unwrap_or_default()))
                })
                .collect();
        entity_commands.insert(crate::weapons_plugin::PhaserBankAiPolicies(
            phaser_bank_policies,
        ));
        // The ship-level WEAPONS DOCTRINE (issue #956): which family this hull
        // turns to bring to bear. Validated in `EntityConfig::from_toml`, so
        // `to_policy` cannot fail here; a hull that authors none attaches no
        // component and asks Helm to turn for nothing, which strict
        // AI-declaration mode makes unreachable for an AI-bearing hull.
        if let Some(ai) = wc.ai.as_ref() {
            entity_commands.insert(crate::weapons_plugin::WeaponsDoctrineAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
        // PhaserRenderConfig: take the first bank's beam_color if any, else default.
        let render_config = if let Some(first_bank) = wc.phaser_banks.first() {
            crate::weapons_plugin::PhaserRenderConfig {
                beam_color: if first_bank.beam_color.len() == 4 {
                    [
                        first_bank.beam_color[0],
                        first_bank.beam_color[1],
                        first_bank.beam_color[2],
                        first_bank.beam_color[3],
                    ]
                } else {
                    crate::beam_render::DEFAULT_BEAM_COLOR
                },
                beam_range: if first_bank.beam_range > 0.0 {
                    first_bank.beam_range
                } else {
                    40.0
                },
            }
        } else {
            crate::weapons_plugin::PhaserRenderConfig::default()
        };
        entity_commands.insert(render_config);
    }

    // Torpedoes — attach a `TorpedoSystemResource` component when the entity
    // TOML has a `[torpedoes]` block. Mirrors the player-ship insertion in
    // `server_app.rs::spawn_game_start_entities` — NPCs and the player ship
    // now use the same per-entity component (PRD #597 gap-3 closure).
    if let Some(tc) = &config.torpedoes {
        let runtime_config = tc.to_runtime();
        let torpedo_system = if !tc.tubes.is_empty() {
            crate::torpedo::TorpedoSystem::from_configs(&tc.tubes, runtime_config)
        } else {
            crate::torpedo::TorpedoSystem::new(runtime_config)
        };
        entity_commands.insert(crate::weapons_plugin::TorpedoSystemResource(torpedo_system));

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
        entity_commands.insert(crate::weapons_plugin::TorpedoTubeAiPolicies(tube_policies));

        // The shared magazine's grant AI policy (issue #782, AC1): the authored
        // `[torpedoes].ai` block.
        if let Some(ai) = tc.ai.as_ref() {
            entity_commands.insert(crate::weapons_plugin::TorpedoMagazineAiPolicy(
                ai.to_policy().unwrap_or_default(),
            ));
        }
    }

    // Blasters — attach a `BlasterSystemResource` component when the entity
    // TOML has a non-empty `[[weapons_console.blaster_banks]]` list. Mirrors
    // the torpedo insertion above so NPCs and the player ship both participate
    // in the per-entity component model (issue #631 Finding 1).
    if let Some(wc) = &config.weapons_console {
        if !wc.blaster_banks.is_empty() {
            let blaster_systems: Vec<crate::blaster::BlasterSystem> = wc
                .blaster_banks
                .iter()
                .map(|bc| crate::blaster::BlasterSystem::new(bc.to_runtime()))
                .collect();
            entity_commands.insert(crate::weapons_plugin::BlasterSystemResource(
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
            entity_commands.insert(crate::weapons_plugin::BlasterBankAiPolicies(
                blaster_bank_policies,
            ));
        }
    }

    // HelmConsole - attach a HelmConsoleSection so the AI tick can read movement params.
    // Also insert the four drive-config Components (PR 4 — PRD #597) so NPC ships
    // participate in the per-entity config model alongside the player ship.
    if let Some(hc) = &config.helm_console {
        entity_commands.insert(HelmConsoleSection(hc.clone()));

        // Physics config
        entity_commands.insert(crate::ship_plugin::ShipPhysicsConfigResource(
            crate::ship_physics::ShipPhysicsConfig {
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
                ..crate::ship_physics::ShipPhysicsConfig::new()
            },
        ));
        // Impulse config
        // Impulse config — steering_multiplier from [helm_capability] when present,
        // falling back to the const default (0.1) when absent.
        let impulse_steering = config
            .helm_capability
            .as_ref()
            .map(|cap| cap.impulse.steering_multiplier)
            .unwrap_or(crate::impulse::IMPULSE_STEERING_MULTIPLIER_DEFAULT);
        entity_commands.insert(crate::ship_plugin::ImpulseConfigResource {
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
        entity_commands.insert(boost_cfg);
        // Bank config
        entity_commands.insert(crate::ship_plugin::BankConfigResource {
            max_bank_deg: hc.max_bank_deg,
            bank_lerp_rate: hc.bank_lerp_rate,
        });
    }

    // HelmCapability — attach when [helm_capability] is present.
    if let Some(cap) = &config.helm_capability {
        entity_commands.insert(HelmCapabilitySection(cap.clone()));
    }

    // Comms range - attach CommsRange component when [comms] is present, and
    // the CommsHailable opt-in marker when that block asks for the hail roster
    // (issue #985). Two components, not one: `range` gates reachability for
    // EVERY comms endpoint, while `hailable` is what puts the entity on the
    // roster the Comms officer can call up.
    if let Some(comms) = &config.comms {
        entity_commands.insert(crate::comms::CommsRange(comms.range));
        if comms.hailable {
            entity_commands.insert(crate::comms::CommsHailable {
                display_name: comms.display_name.clone(),
            });
        }
    }

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
        entity_commands.insert(shields);
    } else if !config.shield_arcs.is_empty() {
        // Ships that declare `[[shield_arc]]` blocks without a
        // `[shields_console]` block (some legacy paths). Still build the
        // shield system from arcs, using default focus config.
        use crate::weapons::shield::ShieldSystem;
        let ship_wide = crate::shield::ShieldConfig::default();
        let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
        let shield_system = ShieldSystem::from_arcs(&arcs, &ship_wide);
        let freq = config
            .shield_arcs
            .first()
            .map(|a| a.frequency)
            .unwrap_or(0.5);
        entity_commands.insert(crate::ship::shields::ShipShields(shield_system, freq));
    }

    // Shields damage history — per-ship Component tracking HP deltas for the
    // AI damage-concentration algorithm. Initialised empty; resized lazily.
    entity_commands.insert(crate::ship::shields::ShieldsDamageHistory::default());

    // Per-arc hull HP (issue #514) — populated from `[[shield_arc]].hull_max_hp`
    // and companion threshold/debuff fields. Attaches `EntityShipArcHull`
    // alongside the shield system so `sync_console_damage_tiers` can route arc
    // damage → offline_systems per-arc. Skipped when no arc declares hull HP.
    if !config.shield_arcs.is_empty() {
        let arc_entries: Vec<(String, crate::damage::ArcHullEntry)> = config
            .shield_arcs
            .iter()
            .filter(|a| a.hull_max_hp > 0.0)
            .map(|a| {
                (
                    a.id.clone(),
                    crate::damage::ArcHullEntry {
                        current: a.hull_max_hp,
                        max: a.hull_max_hp,
                        tier_config: crate::damage::ConsoleTierConfig {
                            damaged_threshold_pct: a.hull_damaged_threshold_pct,
                            disabled_threshold_pct: a.hull_disabled_threshold_pct,
                            debuff_magnitude: a.hull_debuff_magnitude,
                        },
                    },
                )
            })
            .collect();
        if !arc_entries.is_empty() {
            entity_commands.insert(EntityShipArcHull(crate::damage::ShipArcHull::from_entries(
                arc_entries,
            )));
        }
    }

    // Infrastructure condition + capacity (issue #1025) — attach the track when
    // `[infrastructure]` is present. Placed BEFORE the hull block for the same
    // reason the shields block is: the hull block has an early return for the
    // empty-hull case, and anything after it could be skipped.
    if let Some(infrastructure) = &config.infrastructure {
        entity_commands.insert(crate::infrastructure::InfrastructureCondition(
            crate::infrastructure::InfrastructureState::from_config(infrastructure),
        ));
    }

    // External operations (issue #1026) — attach the record when `[operations]`
    // is present, placed here for the same reason. The record carries the
    // authored capability table AND the live hold: a hull that can stabilise
    // starts able to and running nothing.
    if let Some(operations) = &config.operations {
        entity_commands.insert(crate::operations::ShipOperations {
            capabilities: operations.clone(),
            ..Default::default()
        });
    }

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
                .find(|s| s.kind == crate::system_registry::TRACTOR_KIND)
                .and_then(|s| s.power_group.clone())
        }) {
            entity_commands.insert(crate::tractor::TractorBeam::new(
                tractor.clone(),
                power_group,
            ));
        }
    }

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
        entity_commands.insert(
            crate::console::repair::external_server::ExternalRepairDispatch::new(external.clone()),
        );
    }

    // The held-response (issue #1158) — attach when `[held_response]` is
    // present, on a TARGET entity. It says what being held DOES to this thing;
    // the tractor server reads it off whatever it is holding. An entity that
    // authors nothing carries no component and is merely held in place.
    if let Some(held_response) = &config.held_response {
        entity_commands.insert(crate::tractor::HeldResponseSection(held_response.clone()));
    }

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
                if let Some(rig) =
                    crate::entities::glb_visual::resolve_sidecar_rig(model, mesh.variant.as_deref())
                {
                    let markers = crate::dock::resolve_dock_markers(&rig);
                    if !markers.is_empty() {
                        entity_commands.insert(markers);
                    }
                }
            }
        }
        if let Some(power_group) = config.ship_config.as_ref().and_then(|sc| {
            sc.systems
                .iter()
                .find(|s| s.kind == crate::system_registry::DOCK_KIND)
                .and_then(|s| s.power_group.clone())
        }) {
            entity_commands.insert(crate::dock::DockControl::new(dock.clone(), power_group));
        }
    }

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
                .find(|s| s.kind == crate::system_registry::UMBILICAL_KIND)
                .and_then(|s| s.power_group.clone())
        }) {
            entity_commands.insert(crate::umbilical::TransferUmbilical::new(
                umbilical.clone(),
                power_group,
            ));
        }
    }

    // The science scan (issue #1032) — attach the record when `[scan]` is
    // present, on the same argument again. The record carries the authored
    // fidelity ladder AND the last reading: a hull that can scan starts able to
    // and having read nothing.
    if let Some(scan) = &config.scan {
        entity_commands.insert(crate::science::ShipScanRecord {
            config: scan.clone(),
            ..Default::default()
        });
    }

    // Civilian traffic (issue #1028) — attach the authored assignment and the
    // live route/order/compliance state when `[civilian]` is present. Same
    // placement argument as the infrastructure block above. The pair is
    // deliberate: the section never changes after spawn, the traffic state is
    // the only half a save has to carry.
    if let Some(civilian) = &config.civilian {
        entity_commands.insert((
            crate::civilian::CivilianSection(civilian.clone()),
            crate::civilian::CivilianTraffic(crate::civilian::CivilianState::from_config(civilian)),
        ));
    }

    // Hull -- attach an EntitySystemHull component if the config has hull data.
    // Per-system entries take precedence; if absent we fall back to the
    // legacy scalar `hull_integrity` value mapped to a single `SystemId("captain")`
    // slot (used by simple entities like asteroids and station spawns).
    if let Some(hull) = &config.hull {
        let system_hull: crate::damage::SystemHull = if !hull.system_hull.is_empty() {
            // Explicit `[[hull.system_hull]]` entries — new authoring path.
            let entries: Vec<(
                crate::messages::SystemId,
                String,
                f32,
                crate::damage::ConsoleTierConfig,
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
                        crate::damage::ConsoleTierConfig {
                            damaged_threshold_pct: e.damaged_threshold_pct,
                            disabled_threshold_pct: e.disabled_threshold_pct,
                            debuff_magnitude: e.debuff_magnitude,
                        },
                    )
                })
                .collect();
            crate::damage::SystemHull::from_config_with_display_names(entries)
        } else if hull.hull_integrity > 0.0 {
            crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".to_string()),
                hull.hull_integrity,
            )])
        } else {
            // Empty hull section — skip.
            entity_commands.insert(EntitySystemHull(crate::damage::SystemHull::default()));
            return entity_commands.id();
        };
        entity_commands.insert(EntitySystemHull(system_hull));
    }

    entity_commands.id()
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::entity_config::*;

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

    // ── Issue #1025: `[infrastructure]`, on both authoring paths ──

    /// The exemplar depot template, trimmed to the two blocks under test.
    const DEPOT_TOML: &str = r#"
[hull]
hull_integrity = 400.0

[infrastructure]
condition_max = 100.0
hull_damage_share = 0.5

[[infrastructure.capacity]]
id = "depot_transfer_throughput"
amount = 40

[[infrastructure.threshold]]
flag = "depot_transfer_capable"
fails_below = 0.4
"#;

    fn lenient(source: &str) -> EntityConfig {
        EntityConfig::from_toml_in_mode(
            source,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("the fixture parses")
    }

    /// **AC1.** A template that authors `[infrastructure]` spawns with a live
    /// condition track; one that does not spawns exactly as it did before the
    /// section existed.
    ///
    /// Both directions, because a gate that only ever reads true would pass the
    /// first half alone — and the second half is the whole "omitting it changes
    /// nothing" claim.
    #[test]
    fn an_authored_infrastructure_table_attaches_a_condition_track_and_omitting_it_attaches_none() {
        let mut app = test_app();
        let e = spawn_and_flush(
            &mut app,
            &lenient(DEPOT_TOML),
            Vec3::ZERO,
            "depot".into(),
            None,
        );
        let track = app
            .world()
            .get::<crate::infrastructure::InfrastructureCondition>(e)
            .expect("an authored [infrastructure] table must attach the track");
        assert_eq!(
            track.0.condition(),
            100.0,
            "intact unless authored otherwise"
        );
        assert_eq!(
            track.0.capacity("depot_transfer_throughput"),
            Some(40),
            "the authored capacity travels onto the entity"
        );
        assert_eq!(
            track.0.flag("depot_transfer_capable"),
            Some(true),
            "and its operational flag starts level-evaluated against the condition"
        );

        let mut app = test_app();
        let e = spawn_and_flush(
            &mut app,
            &lenient("[hull]\nhull_integrity = 400.0\n"),
            Vec3::ZERO,
            "plain".into(),
            None,
        );
        assert!(
            app.world()
                .get::<crate::infrastructure::InfrastructureCondition>(e)
                .is_none(),
            "an entity that authors no [infrastructure] must carry no track — every station, \
             asteroid and hull in the repository is in this arm"
        );
        assert!(
            app.world().get::<EntitySystemHull>(e).is_some(),
            "…and is otherwise spawned exactly as before, hull and all"
        );
    }

    /// **AC1.** The world's `[[entity]].overrides` path reaches the same table.
    ///
    /// This is the half a template test cannot cover: a scenario placing a
    /// shared depot has to be able to say "this one arrives already battered"
    /// without forking the template.
    #[test]
    fn a_world_entity_override_retunes_the_authored_infrastructure_table() {
        let overrides: toml::Value = toml::from_str("[infrastructure]\ncondition = 80.0\n")
            .expect("the override document parses");
        let merged = crate::entity_loader::apply_overrides(&lenient(DEPOT_TOML), &overrides)
            .expect("the override merges onto the template");
        let infrastructure = merged
            .infrastructure
            .as_ref()
            .expect("the merged config still has the table");
        assert_eq!(
            infrastructure.condition,
            Some(80.0),
            "the world's starting condition wins"
        );
        assert_eq!(
            infrastructure.condition_max, 100.0,
            "…while everything the override was silent about survives from the template"
        );
        assert_eq!(
            infrastructure.capacities.len(),
            1,
            "including the authored capacity, which a plain table-replacing merge would have \
             dropped"
        );
        assert_eq!(infrastructure.thresholds.len(), 1, "…and the threshold");

        let mut app = test_app();
        let e = spawn_and_flush(&mut app, &merged, Vec3::ZERO, "depot".into(), None);
        let track = app
            .world()
            .get::<crate::infrastructure::InfrastructureCondition>(e)
            .expect("the merged config still attaches a track");
        assert_eq!(track.0.condition(), 80.0);
    }

    // ── Issue #1158: `[held_response]`, on both authoring paths ──

    /// **AC1.** A target that authors `[held_response]` spawns carrying it; one
    /// that authors nothing carries no component and is merely held in place.
    ///
    /// Both directions: the second arm is the whole "a target that authors
    /// nothing is merely held in place" claim — every derelict and structure
    /// written before this existed is in it.
    #[test]
    fn an_authored_held_response_attaches_a_section_and_omitting_it_attaches_none() {
        let mut app = test_app();
        let e = spawn_and_flush(
            &mut app,
            &lenient(
                "[hull]\nhull_integrity = 400.0\n\n[held_response]\nkind = \"arrest-decline\"\n\
                 recover_per_sec = 20.0\n",
            ),
            Vec3::ZERO,
            "structure".into(),
            None,
        );
        let held = app
            .world()
            .get::<crate::tractor::HeldResponseSection>(e)
            .expect("an authored [held_response] table must attach the section");
        assert_eq!(held.0.kind, crate::tractor::HeldResponseKind::ArrestDecline);
        assert_eq!(held.0.recover_per_sec, Some(20.0));

        let mut app = test_app();
        let e = spawn_and_flush(
            &mut app,
            &lenient("[hull]\nhull_integrity = 400.0\n"),
            Vec3::ZERO,
            "plain".into(),
            None,
        );
        assert!(
            app.world()
                .get::<crate::tractor::HeldResponseSection>(e)
                .is_none(),
            "a target that authors no [held_response] carries no component and is merely held in \
             place — every derelict and structure before this slice is in this arm"
        );
    }

    /// **AC1.** The world's `[[entity]].overrides` path reaches the table too —
    /// the authoring path `probe_held_response.toml` uses to make a shipped depot
    /// arrest-decline without forking the template.
    #[test]
    fn a_world_entity_override_authors_a_held_response_onto_a_target() {
        let overrides: toml::Value =
            toml::from_str("[held_response]\nkind = \"arrest-decline\"\nrecover_per_sec = 20.0\n")
                .expect("the override document parses");
        let merged = crate::entity_loader::apply_overrides(&lenient(DEPOT_TOML), &overrides)
            .expect("the override merges onto a target that authored none");
        let held = merged
            .held_response
            .as_ref()
            .expect("the merged config carries the authored held-response");
        assert_eq!(held.kind, crate::tractor::HeldResponseKind::ArrestDecline);

        let mut app = test_app();
        let e = spawn_and_flush(&mut app, &merged, Vec3::ZERO, "depot".into(), None);
        assert!(
            app.world()
                .get::<crate::tractor::HeldResponseSection>(e)
                .is_some(),
            "…and the merged target spawns carrying it, over its condition track"
        );
    }

    /// **AC7.** A `[held_response]` table whose fields do not match its kind is a
    /// load error naming the field, not a hold that silently arrests nothing.
    #[test]
    fn an_unauthorable_held_response_fails_the_entity_load() {
        let err = EntityConfig::from_toml_in_mode(
            "[hull]\nhull_integrity = 400.0\n\n[held_response]\nkind = \"arrest-decline\"\n",
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect_err("arrest-decline with no recover_per_sec must be refused");
        assert!(
            err.to_string().contains("recover_per_sec"),
            "the refusal must name the missing field, got {err}"
        );
    }

    // ── Issue #1026: `[operations]`, on both authoring paths ──

    /// A hull that authors a stabilise capability, trimmed to the block under
    /// test.
    const TENDER_TOML: &str = r#"
[hull]
hull_integrity = 300.0

[[operations.capability]]
verb = "stabilise"
range = 500.0
duration_secs = 12
power_group = "helm"
min_power_level = 2
condition_on_complete = 20.0
"#;

    /// **AC1.** A hull that authors `[operations]` spawns able to perform them;
    /// one that does not spawns exactly as it did before the section existed.
    ///
    /// This is the TEMPLATE authoring path — the half `probe_stabilise_resume.toml`
    /// cannot cover, because a world entity's `overrides` block is the other
    /// one. Both directions, because a gate that only ever reads true would pass
    /// the first half alone, and the second half is the whole "omitting it
    /// changes nothing" claim: every hull in the repository is in that arm.
    #[test]
    fn an_authored_operations_table_attaches_a_capability_record_and_omitting_it_attaches_none() {
        let mut app = test_app();
        let e = spawn_and_flush(
            &mut app,
            &lenient(TENDER_TOML),
            Vec3::ZERO,
            "tender".into(),
            None,
        );
        let ops = app
            .world()
            .get::<crate::operations::ShipOperations>(e)
            .expect("an authored [operations] table must attach the record");
        let capability = ops
            .capabilities
            .capability(crate::operations::OperationVerb::Stabilise)
            .expect("the authored verb travels onto the entity");
        assert_eq!(capability.range, 500.0);
        assert_eq!(capability.duration_secs, 12);
        assert_eq!(capability.condition_on_complete, 20.0);
        assert!(
            ops.active.is_none() && ops.last_refusal.is_none() && ops.next_id == 0,
            "a hull that can stabilise starts able to and running nothing"
        );

        let mut app = test_app();
        let e = spawn_and_flush(
            &mut app,
            &lenient("[hull]\nhull_integrity = 300.0\n"),
            Vec3::ZERO,
            "plain".into(),
            None,
        );
        assert!(
            app.world()
                .get::<crate::operations::ShipOperations>(e)
                .is_none(),
            "a hull that authors no [operations] must carry no record — every hull in the \
             repository is in this arm"
        );
        assert!(
            app.world().get::<EntitySystemHull>(e).is_some(),
            "…and is otherwise spawned exactly as before, hull and all"
        );
    }

    /// **AC1.** The world's `[[entity]].overrides` path reaches the same table
    /// — the path `probe_stabilise_resume.toml` actually authors through.
    #[test]
    fn a_world_entity_override_can_grant_a_hull_an_operation_capability() {
        let overrides: toml::Value = toml::from_str(
            "[operations]\ncapability = [{ verb = \"stabilise\", range = 2000.0 }]\n",
        )
        .expect("the override document parses");
        let merged = crate::entity_loader::apply_overrides(
            &lenient("[hull]\nhull_integrity = 300.0\n"),
            &overrides,
        )
        .expect("the override merges onto the template");
        let mut app = test_app();
        let e = spawn_and_flush(&mut app, &merged, Vec3::ZERO, "tender".into(), None);
        let ops = app
            .world()
            .get::<crate::operations::ShipOperations>(e)
            .expect("a world can grant a capability the template never authored");
        let capability = ops
            .capabilities
            .capability(crate::operations::OperationVerb::Stabilise)
            .expect("the overridden verb resolves");
        assert_eq!(capability.range, 2000.0, "the world's range wins");
        assert_eq!(
            capability.duration_secs, 20,
            "…while everything the override was silent about takes its authored default rather \
             than zero"
        );
    }

    /// **AC1.** An `[operations]` table that cannot mean anything is a load
    /// error naming the field, not a capability the crew can start and never
    /// finish.
    #[test]
    fn an_unauthorable_operations_table_fails_the_entity_load() {
        let source = "[hull]\nhull_integrity = 300.0\n\n[[operations.capability]]\n\
                      verb = \"stabilise\"\nduration_secs = 0\n";
        let err = EntityConfig::from_toml_in_mode(
            source,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect_err("an operation that takes no time must be refused");
        assert!(
            err.to_string().contains("duration_secs"),
            "the refusal must name the field the author got wrong, got {err}"
        );
    }

    /// **AC1.** An `[infrastructure]` table that cannot mean anything is a load
    /// error naming the field, not a structure that silently never degrades.
    #[test]
    fn an_unauthorable_infrastructure_table_fails_the_entity_load() {
        let source = format!(
            "{DEPOT_TOML}\n[[infrastructure.threshold]]\nflag = \"other\"\nfails_below = 40.0\n"
        );
        let err = EntityConfig::from_toml_in_mode(
            &source,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect_err("a threshold authored in points rather than fractions must be refused");
        assert!(
            err.to_string().contains("FRACTION"),
            "the refusal must say what the field wanted, got {err}"
        );
    }

    /// A `[repair]` table that exists only to carry `[repair.selector]` gives
    /// the ship NO repair teams.
    ///
    /// This is the gate #885b stage 5b had to move. Every hull now authors
    /// `[repair.selector]`, and TOML cannot write that sub-table without
    /// bringing `[repair]` into existence — so the old "the block is present"
    /// gate would have crewed two repair teams onto six NPC hulls that never had
    /// any, purely as a side effect of a table header. The gate is the COUNT:
    /// a ship has repair teams when its TOML says how many.
    ///
    /// Both directions are asserted, because a gate that only ever reads false
    /// would pass the first half alone.
    #[test]
    fn repair_teams_are_gated_on_the_authored_count_not_on_the_repair_table() {
        let selector_only = r#"
[hull]
hull_integrity = 100.0

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
directive_kind = "Destroy"
base_priority = 40.0

[repair.selector]
sources = ["damaged-stations"]
horizon = 1e9
switch_margin = 0.0
eligibility = "candidate_fact(source_repair_request) > 0"
"#;
        let config = EntityConfig::from_toml_in_mode(
            selector_only,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("selector-only [repair] parses");
        assert!(
            config
                .repair
                .as_ref()
                .is_some_and(|r| r.selector.is_some() && !r.declares_teams()),
            "precondition: `[repair.selector]` alone brings `[repair]` into existence \
             but declares no teams"
        );
        let mut app = test_app();
        let e = spawn_and_flush(&mut app, &config, Vec3::ZERO, "selector-only".into(), None);
        assert!(
            app.world()
                .get::<crate::console::repair::server::ShipRepairTeams>(e)
                .is_none(),
            "a `[repair]` table carrying only a selector must NOT crew repair teams — \
             authoring a ranking policy is not the same as declaring the capability it \
             ranks for."
        );
        assert!(
            app.world()
                .get::<crate::console::repair::server::RepairTargetSelector>(e)
                .is_some(),
            "…while the selector itself is still attached: the teams gate dispatch, so \
             a ship that gains them later already has its ranking."
        );

        let with_teams = format!("{selector_only}\n[repair]\nrepair_team_count = 3\n");
        let config = EntityConfig::from_toml_in_mode(
            &with_teams,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("[repair] with a count parses");
        let mut app = test_app();
        let e = spawn_and_flush(&mut app, &config, Vec3::ZERO, "with-teams".into(), None);
        let teams = app
            .world()
            .get::<crate::console::repair::server::ShipRepairTeams>(e)
            .expect("an authored `repair_team_count` must crew that many teams");
        assert_eq!(
            teams.0.slots().len(),
            3,
            "the authored count is the team count, verbatim"
        );
    }

    #[test]
    fn spawn_entity_with_comms_inserts_comms_range_component() {
        let mut app = test_app();
        let config = EntityConfig {
            reference_grid: None,
            name: None,
            display_name: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: Some(crate::entity_config::CommsConfig {
                range: 8000.0,
                hailable: false,
                display_name: None,
            }),
            asteroid_field: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
            planet: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let range = world
            .get::<crate::comms::CommsRange>(spawned)
            .expect("CommsRange component should be inserted when [comms] is present");
        assert_eq!(range.0, 8000.0);
        assert!(
            world.get::<crate::comms::CommsHailable>(spawned).is_none(),
            "a range-only [comms] block must NOT put the entity on the hail roster (#985)"
        );
    }

    /// Issue #985: `[comms] hailable = true` is the opt-in that puts an entity
    /// on the hail roster, and `display_name` rides along as the contact label.
    #[test]
    fn spawn_entity_with_hailable_comms_inserts_the_opt_in_marker() {
        let mut app = test_app();
        let config = EntityConfig {
            name: Some("world.entity.outpost.name".into()),
            comms: Some(crate::entity_config::CommsConfig {
                range: 800.0,
                hailable: true,
                display_name: Some("Relay Outpost".into()),
            }),
            ..EntityConfig::default()
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let hailable = world
            .get::<crate::comms::CommsHailable>(spawned)
            .expect("hailable = true must insert CommsHailable");
        assert_eq!(hailable.display_name.as_deref(), Some("Relay Outpost"));
    }

    #[test]
    fn spawn_entity_without_comms_omits_comms_range_component() {
        let mut app = test_app();
        let config = EntityConfig {
            reference_grid: None,
            name: None,
            display_name: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
            planet: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
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
            reference_grid: None,
            name: Some("Sun".to_string()),
            display_name: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
            planet: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
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
            reference_grid: None,
            name: None,
            display_name: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
            planet: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
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
            reference_grid: None,
            name: None,
            display_name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec![],
            collider: Some(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 3.0,
                length: 0.0,
                half_height: None,
                movable: true,
            }),
            hull: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
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

    /// The `Cylinder` variant's whole meaning is the rapier body it builds, so
    /// this asserts on that body rather than on the config that produced it.
    /// The half-height goes in FIRST and the radius second: getting the two the
    /// wrong way round would build a 17-tall, 7-wide pillar out of a station
    /// that is 34 across and 14 tall, and no other assertion in the tree would
    /// notice.
    ///
    /// The numbers are the shipped starbase's, so this pins the axis too — a
    /// `Collider::cylinder` is Y-axis by construction, and the disc it makes is
    /// the one a station is.
    #[test]
    fn a_cylinder_collider_spawns_a_y_axis_disc_of_the_authored_size() {
        let mut app = test_app();
        let config = EntityConfig {
            collider: Some(ColliderConfig {
                shape: ColliderShape::Cylinder,
                radius: 17.04,
                length: 0.0,
                half_height: Some(7.16),
                movable: false,
            }),
            ..Default::default()
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let collider = app
            .world()
            .get::<Collider>(spawned)
            .expect("a [collider] section must build a rapier Collider");
        let cylinder = collider
            .as_cylinder()
            .expect("shape = \"Cylinder\" must build a rapier cylinder, not a ball or a capsule");
        assert!(
            (cylinder.half_height() - 7.16).abs() < 1e-5,
            "expected the authored half-height, got {}",
            cylinder.half_height()
        );
        assert!(
            (cylinder.radius() - 17.04).abs() < 1e-5,
            "expected the authored radius, got {}",
            cylinder.radius()
        );
    }

    /// A degenerate `Cylinder` cannot arrive from a TOML template — the load
    /// path rejects one — but `spawn_entity` takes an `EntityConfig`, and a
    /// caller that builds one in code bypasses that. The fallback errs UPWARDS
    /// to the enclosing sphere rather than collapsing to nothing: a zero-height
    /// disc is a structure ships fly straight through, an over-tall one merely
    /// stops them early, and only one of those is a bug worth shipping.
    #[test]
    fn a_cylinder_built_in_code_without_a_half_height_falls_back_to_its_radius() {
        let mut app = test_app();
        let config = EntityConfig {
            collider: Some(ColliderConfig {
                shape: ColliderShape::Cylinder,
                radius: 3.8,
                length: 0.0,
                half_height: None,
                movable: false,
            }),
            ..Default::default()
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let collider = app.world().get::<Collider>(spawned).unwrap();
        let cylinder = collider.as_cylinder().expect("still a cylinder");
        assert!(
            (cylinder.half_height() - 3.8).abs() < 1e-5,
            "an unauthored half-height must fall back to the radius, not to zero; got {}",
            cylinder.half_height()
        );
    }

    #[test]
    fn spawn_entity_with_lights_inserts_lights_component() {
        let mut app = test_app();
        let config = EntityConfig {
            reference_grid: None,
            name: None,
            display_name: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
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
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            star: None,
            planet: None,
            ship_config: None,
            shield_arcs: Vec::new(),
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
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
            reference_grid: None,
            name: None,
            display_name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec!["field".to_string()],
            asteroid_field: Some(AsteroidFieldConfig {
                inner_radius: 100.0,
                outer_radius: 200.0,
                density: 0.005,
                weight: 1.0,
                spawn_distance: 150.0,
                despawn_distance: 250.0,
                asteroid_type_paths: vec!["small.toml".into()],
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
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
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
            reference_grid: None,
            name: None,
            display_name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec![],
            appearance: Some(AppearanceConfig {
                colour: "#ff0000".to_string(),
                size_min: 1.0,
                size_max: 3.0,
            }),
            hull: None,
            collider: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
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

    /// Issue #1154: the authored `mass` — already defaulted at parse time —
    /// carries straight onto the spawned entity, unconditionally, exactly as
    /// [`EntityUuid`] does.
    #[test]
    fn spawn_entity_carries_authored_mass_onto_entity_mass_component() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("mass = 45000.0\n").unwrap();

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        let mass = world
            .get::<EntityMass>(spawned)
            .expect("every spawned entity must carry EntityMass");
        assert_eq!(mass.0, 45_000.0);
    }

    /// An entity that authors no `mass` still gets a real, non-zero weight on
    /// the spawned entity — the documented default rides through the spawner
    /// exactly as an authored value does, never falling back to a bare zero.
    #[test]
    fn spawn_entity_without_authored_mass_carries_the_documented_default() {
        let mut app = test_app();
        let config = EntityConfig::from_toml("").unwrap();

        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let world = app.world_mut();
        let mass = world
            .get::<EntityMass>(spawned)
            .expect("every spawned entity must carry EntityMass");
        assert_eq!(mass.0, crate::entity_config::DEFAULT_ENTITY_MASS);
        assert!(
            mass.0 > 0.0,
            "an unauthored entity must never spawn at zero mass"
        );
    }

    #[test]
    fn spawn_entity_with_region_shape_and_effects() {
        let mut app = test_app();
        let config = EntityConfig {
            reference_grid: None,
            name: None,
            display_name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
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
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
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
            reference_grid: None,
            name: None,
            display_name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius: 100.0 }),
            effects: None,
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
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
            reference_grid: None,
            name: None,
            display_name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec![],
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: Some(faction_id),
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
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

    // -- EntitySystemHull component tests --

    #[test]
    fn spawn_entity_with_hull_integrity_attaches_captain_chair_slot() {
        let mut app = test_app();
        let config = EntityConfig {
            reference_grid: None,
            name: None,
            display_name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec![],
            hull: Some(crate::entity_config::HullConfig {
                hull_integrity: 60.0,
                ..Default::default()
            }),
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let hull_comp = world
            .get::<EntitySystemHull>(spawned)
            .expect("should have EntitySystemHull when hull_integrity > 0");
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
            world.get::<EntitySystemHull>(spawned).is_none(),
            "entity with no hull config must not have EntitySystemHull"
        );
    }

    // ── ShipShields spawner attachment tests ────────────────────────────────

    #[test]
    fn spawn_entity_with_shields_console_block_attaches_ship_shields() {
        let mut app = test_app();
        let toml = r#"
[hull]
hull_integrity = 60.0

[shields_console.base]
num_facings = 1
max_hp = 30
regen_per_sec = 1.5
"#;
        let config = EntityConfig::from_toml(toml).expect("toml must parse");
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(spawned)
            .expect("entity with [shields_console] block must have ShipShields component");
        assert_eq!(shields.0.facings.len(), 1);
        assert_eq!(shields.0.facings[0].max_hp, 30);
        assert_eq!(shields.0.facings[0].hp, 30);
        assert_eq!(shields.0.facings[0].regen_per_sec, 1.5);
        assert!(shields.0.facings[0].is_online());
    }

    #[test]
    fn spawn_entity_without_shields_console_block_omits_ship_shields() {
        let mut app = test_app();
        let toml = r#"
[hull]
hull_integrity = 60.0
"#;
        let config = EntityConfig::from_toml(toml).expect("toml must parse");
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        assert!(
            app.world()
                .get::<crate::ship::shields::ShipShields>(spawned)
                .is_none(),
            "entity without [shields_console] block must not have ShipShields"
        );
    }

    /// A `[shields_console]` that carries only an AI policy is not a shield
    /// system.
    ///
    /// This is the shape #885b stage 5c created: `[shields_console.ai_policy]`
    /// is a required declaration on every AI-bearing hull, and writing it brings
    /// `[shields_console]` into existence whether or not the hull has shields.
    /// `ship_requiem_courier.toml` is exactly this — no `[[shield_arc]]`, no
    /// `[shields_console.base]` — and it must keep having no shields at all, so
    /// the gate reads the shield CONTENT rather than the table header. The rule
    /// change rides along: a block authoring neither used to mean a default
    /// four-facing system and now means none.
    #[test]
    fn a_shields_console_holding_only_an_ai_policy_attaches_no_ship_shields() {
        let mut app = test_app();
        let toml = r#"
[hull]
hull_integrity = 60.0

[shields_console.ai_policy]

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "true"
verb = "focus_shield_arc"
"#;
        let config = EntityConfig::from_toml(toml).expect("toml must parse");
        assert!(
            config.shields_console.is_some() && config.shield_arcs.is_empty(),
            "precondition: the table exists but declares no shields"
        );
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        assert!(
            app.world()
                .get::<crate::ship::shields::ShipShields>(spawned)
                .is_none(),
            "declaring a shields-focus POLICY must not conjure a shield system onto a \
             hull that carries no arcs and no base config"
        );
    }

    /// …and a hull with arcs is unaffected by which of the two branches it takes.
    ///
    /// The five Harrow hulls each declare one `[[shield_arc]]` and gained a
    /// `[shields_console]` in stage 5c, moving them from the arcs-only fallback
    /// branch onto the console branch. Both paths must build the same system, or
    /// authoring a policy would have silently retuned five ships' shields.
    #[test]
    fn arcs_build_the_same_shield_system_with_or_without_a_shields_console() {
        const ARC: &str = r#"
[hull]
hull_integrity = 60.0

[[shield_arc]]
id = "all"
label = "test.arc"
center_deg = 0.0
width_deg = 360.0
max_hp = 5
regen_per_sec = 0.0
"#;
        let read = |toml: &str| {
            let mut app = test_app();
            let config = EntityConfig::from_toml(toml).expect("toml must parse");
            let uuid = uuid::Uuid::new_v4().to_string();
            let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
            let s = app
                .world()
                .get::<crate::ship::shields::ShipShields>(spawned)
                .expect("a hull with an arc has shields");
            (
                format!("{:?}", s.0.facings),
                format!("{:?}", s.0.focus_config),
                s.1,
            )
        };
        let (arcs_only, focus_only, freq_only) = read(ARC);
        let (arcs_console, focus_console, freq_console) =
            read(&format!("{ARC}\n[shields_console]\n"));
        assert_eq!(arcs_only, arcs_console, "the facings must be identical");
        assert_eq!(
            focus_only, focus_console,
            "an unauthored `[shields_console]` must supply exactly \
             ShieldFocusConfig::default(), or stage 5c retuned five hulls' focus \
             bonuses by writing a policy next door"
        );
        assert_eq!(freq_only, freq_console, "the first arc owns the frequency");
    }

    #[test]
    fn hull_integrity_maps_to_captain_chair_slot() {
        // Stations and asteroids still use hull_integrity in TOML â€” must keep working.
        let mut app = test_app();
        let config = EntityConfig {
            reference_grid: None,
            name: None,
            display_name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec![],
            hull: Some(crate::entity_config::HullConfig {
                hull_integrity: 200.0,
                ..Default::default()
            }),
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let world = app.world_mut();
        let hull_comp = world
            .get::<EntitySystemHull>(spawned)
            .expect("entity with hull_integrity should still get EntitySystemHull");
        assert!((hull_comp.0.total_max() - 200.0).abs() < 1e-6);
        let entries: Vec<_> = hull_comp.0.entries().collect();
        assert_eq!(
            entries[0].0,
            &crate::messages::SystemId("captain".to_string())
        );
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

    // ── #573: NPC all-AI roster ───────────────────────────────────────────────

    /// NPC ships spawned with a [behaviour] block must have every registered
    /// NPC ships now carry the `Ship` marker (same as player ships).
    /// The `LocalShip` marker is the selector for the viewscreen entity.
    /// All registered systems must be set to `ControlSource::Ai`.
    #[test]
    fn npc_ship_spawn_gives_all_ai_roster_and_no_ship_marker() {
        use crate::entity_config::{BehaviourConfig, DoctrineObjective, EntityConfig};
        use crate::server_app::Ship;
        use crate::ship_plugin::ShipSystemControlSources;
        use bevy::prelude::*;

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);

        let config = EntityConfig {
            behaviour: Some(BehaviourConfig {
                doctrine: vec![DoctrineObjective {
                    id: "destroy-hostiles".into(),
                    text: "Destroy hostiles".into(),
                    directive_kind: Some("Destroy".into()),
                    base_priority: 35.0,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let mut cmds = app.world_mut().commands();
        let entity = spawn_entity(
            &mut cmds,
            &config,
            bevy::math::Vec3::ZERO,
            "npc-001".into(),
            None,
        );
        app.world_mut().flush();

        // NPC ship MUST carry Ship marker (same as player ship after #581 unification)
        assert!(
            app.world().get::<Ship>(entity).is_some(),
            "NPC ship must carry Ship marker (same as player ship after PRD #581)"
        );

        // All registered systems must be AI-controlled
        let sources = app
            .world()
            .get::<ShipSystemControlSources>(entity)
            .expect("NPC ship must have ShipSystemControlSources");
        let config_comp = app
            .world()
            .get::<crate::ship_plugin::ShipConfigComponent>(entity)
            .expect("NPC ship must have ShipConfigComponent");
        for sys in &config_comp.0.systems {
            let policy = sources.0.policy_for(&sys.id);
            assert!(
                policy.operate_ai,
                "system '{}' must be AI-controlled on NPC ship",
                sys.id.0
            );
            assert!(
                !policy.accept_human_input,
                "system '{}' must not accept human input on NPC ship",
                sys.id.0
            );
        }
    }

    #[test]
    fn npc_ship_gets_shipconfig_from_its_own_toml_stations_and_systems() {
        // Regression test for PRD #597 PR-3 (correct redo): NPC ship TOMLs with
        // [[system]] blocks must produce a ShipConfigComponent containing those
        // systems — not the player ship's config, and not an empty config.
        use bevy::prelude::*;

        let toml = r#"
tags = ["ship", "npc"]

[collider]
shape = "Capsule"
radius = 2.0
length = 4.0

[behaviour]

[[behaviour.doctrine]]
id = "test-doctrine"
text = "Test"
directive_kind = "Destroy"
base_priority = 1.0

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
ai_only = true

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
ai_only = true
"#;
        let config = EntityConfig::from_toml_in_mode(
            toml,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("toml must parse");
        assert!(
            config.ship_config.is_some(),
            "EntityConfig.ship_config must be populated from [[system]] blocks"
        );
        let sc = config.ship_config.as_ref().unwrap();
        // Two authored systems plus the AI-only red-alert capability that issue
        // #749 provisions for every behaviour-driven NPC that omits one.
        assert_eq!(
            sc.systems.len(),
            3,
            "expected two authored systems + provisioned red-alert"
        );
        assert!(
            sc.systems
                .iter()
                .any(|s| s.kind == crate::system_registry::RED_ALERT_KIND && s.ai_only),
            "behaviour NPC must be provisioned an ai_only red-alert system (#749)"
        );
        assert_eq!(sc.stations.len(), 0, "NPCs have no stations");

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        let mut cmds = app.world_mut().commands();
        let entity = spawn_entity(
            &mut cmds,
            &config,
            bevy::math::Vec3::ZERO,
            "npc-shipconfig-test".into(),
            None,
        );
        app.world_mut().flush();

        let comp = app
            .world()
            .get::<crate::ship_plugin::ShipConfigComponent>(entity)
            .expect("NPC ship must have ShipConfigComponent");
        assert_eq!(
            comp.0.systems.len(),
            3,
            "spawned NPC entity carries its two declared systems + provisioned red-alert"
        );
        let system_ids: Vec<&str> = comp.0.systems.iter().map(|s| s.id.0.as_str()).collect();
        assert!(
            system_ids.contains(&"helm-thrust"),
            "helm-thrust system must be present"
        );
        assert!(
            system_ids.contains(&"tactical-radar"),
            "tactical-radar system must be present"
        );
    }

    /// Issue #839: a world-spawned Alliance hull (i.e. spawned as an NPC, not
    /// selected by the player) must present as a plain ship — no `player` tag,
    /// ordinary `ship` radar icon. Player identity is injected only at the
    /// player game-start spawn (see `player_ship_identity` in `server_app.rs`),
    /// so the checked-in template must not author it. Parses the real template
    /// so it regresses if the `player` tag / `playerShip` icon creep back in.
    #[test]
    fn world_spawned_alliance_hull_has_no_player_identity() {
        use bevy::prelude::*;

        // Through the resolver (issue #876): this hull is COMPOSED, so its baked
        // bytes are no longer the document the game spawns.
        let config =
            crate::entity_includes::load_entity_config("assets/entities/alliance_cruiser.toml")
                .expect("cruiser template must compose and parse");

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        let entity = {
            let mut cmds = app.world_mut().commands();
            spawn_entity(&mut cmds, &config, Vec3::ZERO, "world-cruiser".into(), None)
        };
        app.world_mut().flush();

        let tags = app
            .world()
            .get::<EntityTagsSection>(entity)
            .expect("hull must carry EntityTagsSection");
        assert!(
            tags.0.iter().any(|t| t == "ship"),
            "world-spawned hull keeps the ship tag; got {:?}",
            tags.0
        );
        assert!(
            !tags.0.iter().any(|t| t == "player"),
            "world-spawned hull must NOT carry the player tag; got {:?}",
            tags.0
        );

        let radar = app
            .world()
            .get::<RadarAppearanceSection>(entity)
            .expect("hull must carry RadarAppearanceSection");
        assert_eq!(
            radar.0.icon.as_deref(),
            Some("ship"),
            "world-spawned hull shows the ordinary ship icon, not playerShip"
        );
    }

    /// Issue #749: a behaviour NPC that omits an explicit red_alert system must
    /// still spawn with the provisioned red_alert control source resolving to
    /// `Ai` — the causal link that lets `operate_captain_ai` raise its Red
    /// Alert. The RNG-coverage escort authors [behaviour] but no red_alert block.
    ///
    /// That template moved out of the shipped fleet in issue #954 — it is a test
    /// fixture under `assets/entities/test/`, kept only so `rng_coverage.toml`
    /// has a hull that fires all three weapon kinds. It still loads through the
    /// real composed path, which is all this test needs of it, and pointing at a
    /// fixture is honest about what it is: the #749 provision is a property of
    /// the SPAWNER, not of any one shipped hull.
    #[test]
    fn spawned_behaviour_npc_red_alert_is_ai_operated() {
        use bevy::prelude::*;

        // Through the REAL load path: the hull is composed since issue #878, so
        // its ship-level AI declarations arrive from the fragment library and
        // `include_str!` would spawn an unresolved document.
        let config = crate::entity_includes::load_entity_config(
            "assets/entities/test/rng_coverage_lancer.toml",
        )
        .expect("the rng-coverage escort must resolve and parse");

        let mut app = test_app();
        let uuid = uuid::Uuid::new_v4().to_string();
        let entity = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

        let sources = app
            .world()
            .get::<crate::ship_plugin::ShipSystemControlSources>(entity)
            .expect("behaviour NPC must carry ShipSystemControlSources");
        assert!(
            sources
                .0
                .policy_for(&crate::system_registry::red_alert_system_id())
                .operate_ai,
            "provisioned red_alert must resolve to Ai so operate_captain_ai can raise it"
        );
        // And the ship carries the Red Alert capability component.
        assert!(
            app.world()
                .get::<crate::ship_state::ShipRedAlert>(entity)
                .is_some(),
            "behaviour NPC must carry the ShipRedAlert capability"
        );
    }
}
