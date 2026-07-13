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

    // Cinematic camera section
    if let Some(cam) = &config.cinematic_camera {
        entity_commands.insert(CinematicCameraSection(cam.clone()));
    }

    // Behaviour section — signals ai_plugin to attach an AiControllerComponent.
    if let Some(behaviour) = &config.behaviour {
        entity_commands.insert((
            BehaviourSection(behaviour.clone()),
            crate::server_app::ShipSystemBlackboards::default(),
        ));

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
        let mut resolver = crate::ship::control_source::ControlSourceResolver::new();
        for system in &ship_config.0.systems {
            resolver.set(
                system.id.clone(),
                crate::ship::control_source::ControlSource::Ai,
            );
        }
        // Seed ShipPhysics from the spawn position so the per-entity helm loop
        // starts with the correct initial state rather than (0, 0).
        let ship_physics = crate::ship_state::ShipPhysics {
            x: position.x,
            z: position.z,
            yaw: {
                let rot = bevy::math::Quat::from_euler(bevy::math::EulerRot::YXZ, 0.0, 0.0, 0.0);
                let _ = rot;
                0.0 // initial yaw; updated each tick by operate_helm_ai
            },
            ..Default::default()
        };
        entity_commands.insert((
            ship_config,
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ShipSystemControlSources(resolver),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            ship_physics,
            crate::ai_plugin::ShipAiMemory::default(),
            crate::weapons_plugin::WeaponsTarget::default(),
            crate::weapons_plugin::ActiveBeam::default(),
            crate::weapons_plugin::PhaserCooldown::default(),
            crate::sensors_plugin::SensorsTarget::default(),
            crate::ship_state::ShipRedAlert::default(),
            crate::ship_state::ShipViewMode::default(),
            crate::ship_state::ShipPhaserFrequency::default(),
            crate::navigation_plugin::NavigationWaypoint::default(),
        ));
        // Per-entity helm intent (audit follow-up). Every ship carries
        // its own `LastHelmInput` so systems that iterate `With<Ship>`
        // and read `Option<&LastHelmInput>` see a real value on NPCs
        // instead of the `unwrap_or_default()` fallback. Notably
        // `operate_power_ai` reads it to pin `power.helm` to idle when
        // throttle is zero. Inserted separately because Bevy's tuple
        // Bundle max is 15 elements.
        entity_commands.insert(crate::ship_plugin::LastHelmInput::default());
        // Per-ship coordination bus state (audit follow-up). Every ship
        // tracks its own shields down/restore notification cycle and its
        // own sensors→tactical frequency-hint dedupe state so the two
        // coordination emitters (`emit_shields_coordination`,
        // `tick_sensors_frequency_hint`) can iterate `With<Ship>` and
        // route into each ship's own `CoordinationQueue` via
        // `CoordinationEnqueue.source_entity`.
        entity_commands.insert(crate::ship::shields::ShieldsCoordinationState::default());
        entity_commands.insert(crate::ship::sensors::SensorsFrequencyState::default());
        entity_commands.insert(crate::power_plugin::ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::default(),
        ));
        // Per-entity power config (PRD #597 gap-4 closure). NPCs without a
        // `[power]` TOML block get `PowerConfigResource::default()` /
        // `PowerAiConfigResource::default()` so `translate_power_modifiers`,
        // `operate_power_ai`, and `tick_power_system` can iterate every ship
        // uniformly (`With<Ship>`) without an `is_npc` fork. When the TOML
        // supplies `[power]` / `[power.ai]`, those values seed the components.
        let power_config = match &config.power {
            Some(pc) => crate::power_plugin::PowerConfigResource(
                crate::modifiers::power_system::PowerConfig {
                    capacity: pc.capacity,
                    rates: pc.rates,
                    emergency_threshold: pc.emergency_threshold,
                },
            ),
            None => crate::power_plugin::PowerConfigResource::default(),
        };
        entity_commands.insert(power_config);
        let power_ai_config = match config.power.as_ref().and_then(|pc| pc.ai.as_ref()) {
            Some(ai) => crate::power_plugin::PowerAiConfigResource {
                weapons_battery_floor: ai.weapons_battery_floor,
                shields_battery_floor: ai.shields_battery_floor,
                helm_battery_floor: ai.helm_battery_floor,
                helm_throttle_threshold: ai.helm_throttle_threshold,
            },
            None => crate::power_plugin::PowerAiConfigResource::default(),
        };
        entity_commands.insert(power_ai_config);
        // Per-entity power multipliers. Seeded from any per-console TOML
        // `power_multipliers` blocks (helm_console/weapons_console/sensors_console)
        // and otherwise defaulted so NPC ships still get MaxSpeed / PhaserDamage
        // / RadarRange bonuses translated by `translate_power_modifiers`.
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
                        crate::modifiers::power_system::SENSORS_POWER_GROUP.into(),
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
        if let Some(sc) = &config.sensors_console {
            if let Some(pm) = sc.power_multipliers {
                multipliers.insert(
                    crate::messages::PowerGroupId(
                        crate::modifiers::power_system::SENSORS_POWER_GROUP.into(),
                    ),
                    pm,
                );
            }
        }
        entity_commands.insert(crate::power_plugin::PowerMultiplierResource { multipliers });
        // ShipModifiers as per-entity component (PR 6/9 — PRD #597). Every ship
        // gets an empty modifier cache. Region-entry observers and
        // translate_power_modifiers write to the subject entity's cache;
        // translate_impulse_modifiers remains LocalShip-only (ShipImpulse is a
        // player-only mechanic).
        entity_commands.insert(crate::modifiers::ShipModifiers::new());
        // Per-entity ShipRepairTeams — only insert when the entity TOML declares
        // a [repair] block. Absent block means the ship has no repair teams
        // (the default behaviour for NPCs today).
        if let Some(repair_cfg) = &config.repair {
            let team_count = if repair_cfg.repair_team_count > 0 {
                repair_cfg.repair_team_count as usize
            } else {
                2
            };
            let timings = repair_cfg.to_runtime();
            entity_commands.insert(crate::console::repair::server::ShipRepairTeams(
                crate::repair_teams::RepairTeams::new_with_timings(team_count, timings),
            ));
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
            },
        ));
        // Impulse config
        entity_commands.insert(crate::ship_plugin::ImpulseConfigResource {
            charge_duration: hc.impulse_charge_duration,
            speed_multiplier: hc.impulse_speed_multiplier,
            acceleration_multiplier: hc.impulse_acceleration_multiplier,
            engage_distance: hc.impulse_engage_distance,
            cancel_distance: hc.impulse_cancel_distance,
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

    // Comms range - attach CommsRange component when [comms] is present.
    if let Some(comms) = &config.comms {
        entity_commands.insert(crate::comms::CommsRange(comms.range));
    }

    // Shields — translate the [shields_console] config block + designer
    // `[[shield_arc]]` blocks into a `ShipShields` component with focus
    // tuning. Uses the same code path the player ship uses in
    // spawn_game_start_entities so both player and NPC ships read from one
    // TOML section. Placed BEFORE the hull block: the hull block has an
    // early-return for the empty-hull case, so anything after it could be
    // skipped.
    if let Some(sc) = &config.shields_console {
        use crate::weapons::shield::{ShieldFocusConfig, ShieldSystem};
        let ship_wide = sc.base.as_ref().map(|b| b.to_runtime()).unwrap_or_default();
        let shield_system = if !config.shield_arcs.is_empty() {
            let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
            ShieldSystem::from_arcs(&arcs, &ship_wide)
        } else {
            ShieldSystem::new(&ship_wide)
        };
        let mut shields = crate::ship::shields::ShipShields(shield_system);
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
        entity_commands.insert(crate::ship::shields::ShipShields(shield_system));
    }

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

    #[test]
    fn spawn_entity_with_comms_inserts_comms_range_component() {
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
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
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
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
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
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
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
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
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
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
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
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
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
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
            ship_config: None,
            shield_arcs: Vec::new(),
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
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
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
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
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
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
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
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
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
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
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            target: None,
            mesh: None,
            cinematic_camera: None,
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
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
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
            cinematic_camera: None,
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
            name: None,
            star: None,
            class: None,
            hull_id: None,
            power_rating: None,
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
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
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
            cinematic_camera: None,
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

    #[test]
    fn hull_integrity_maps_to_captain_chair_slot() {
        // Stations and asteroids still use hull_integrity in TOML â€” must keep working.
        let mut app = test_app();
        let config = EntityConfig {
            name: None,
            star: None,
            class: None,
            hull_id: None,
            power_rating: None,
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
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
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
            cinematic_camera: None,
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
        use crate::entity_config::EntityConfig;
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
id = "helm"
kind = "helm"
ai_only = true

[[system]]
id = "tactical"
kind = "tactical"
ai_only = true
"#;
        let config = EntityConfig::from_toml(toml).expect("toml must parse");
        assert!(
            config.ship_config.is_some(),
            "EntityConfig.ship_config must be populated from [[system]] blocks"
        );
        let sc = config.ship_config.as_ref().unwrap();
        assert_eq!(sc.systems.len(), 2, "expected two systems (helm, tactical)");
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
            2,
            "spawned NPC entity carries its two declared systems"
        );
        let system_ids: Vec<&str> = comp.0.systems.iter().map(|s| s.id.0.as_str()).collect();
        assert!(system_ids.contains(&"helm"), "helm system must be present");
        assert!(
            system_ids.contains(&"tactical"),
            "tactical system must be present"
        );
    }
}
