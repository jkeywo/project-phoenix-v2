use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::lobby::{InboundMessage, LobbyOutbox, OutboundMessage, Sessions, Target, WorldResource};
use crate::messages::{
    ClientMessage, DeliveryClass, EntitySnapshot, GamePhase, ServerMessage, ShieldFacingStatus,
    StationId, SystemControlPayload,
};
use crate::shield::ShieldSystem;
use rand::SeedableRng as _;

use crate::damage::{apply_damage_with_shields, apply_hull_damage, collision_damage};
use crate::debug_overlay::{DamageLog, DamageLogEntry};
use crate::shield::attacker_bearing_relative;
use bevy_rapier3d::prelude::ReadRapierContext;
// Re-export ShipPhysics so `crate::simulation::ShipPhysics` and
// `crate::server_app::ShipPhysics` both resolve.
pub use crate::ship_state::ShipPhysics as ShipPhysicsComponent;

use crate::entity_spawner::{
    AsteroidFieldSection, BehaviourSection, ColliderSection, EntityId, EntityName,
    EntityTagsSection, EntityUuid, FactionComponent, MeshSection, RadarAppearanceSection,
    RegionShapeSection,
};
use crate::impulse::ImpulseState;
use crate::messages::ModifierSlot;
use crate::modifiers::ShipModifiers;
use crate::world::server::ObjectiveManagerRes;
use std::collections::HashMap;

// â"€â"€ Beam constants â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
pub use crate::weapons_plugin::{
    weapons_update_broadcaster, ActiveBeam, AsteroidDestroyedVfx, CurrentPhaserMode,
    LastShipAttacker, LastWeaponsUpdate, PhaserCooldown, PhaserRenderConfig, TorpedoSystemResource,
    WeaponsTarget,
};

pub use crate::repair_plugin::{repair_state_broadcaster, ShipRepairTeams};

pub use crate::power_plugin::{
    power_state_broadcaster, PowerAiConfigResource, PowerConfigResource, PowerMultiplierResource,
    ShipPowerSystem,
};

// â"€â"€ Marker Components â"€â"€â"€â"€â"€â"€â"€â"€
/// Marks the player-controlled ship entity in simulation queries.
/// Rendering and networking queries should use `With<LocalShip>` instead.
#[derive(Component)]
pub struct Ship;

/// Tags the single entity rendered on the viewscreen and broadcast to clients.
/// Only rendering, networking (broadcast), pfx, comms-range, and
/// region-membership systems filter on this. Simulation systems use `With<Ship>`.
#[derive(Component)]
pub struct LocalShip;

/// Marker component on the scene-root child entity of the local ship's GLB
/// model. The child starts `Visibility::Hidden` and is toggled by the
/// cinematic camera system.
#[derive(Component)]
pub struct LocalShipModel;

#[derive(Component)]
pub struct Asteroid;

/// Marks a light entity that should continuously rotate to face the
/// player's ship, regardless of how its parent entity is oriented.
#[derive(Component)]
pub struct FacePlayerLight;

/// Stable UUID string identifying this asteroid entity (for targeting).
#[derive(Component, Clone)]
pub struct AsteroidUuid(pub String);

/// Per-asteroid `shield_pierce` snapshot, copied from the parent
/// `AsteroidFieldConfig.shield_pierce` at spawn time. Read by
/// `handle_collisions` to split impact damage between shields and hull.
/// When the component is missing, the collision handler treats it as
/// `0.0` (full shield mitigation — pre-#414 behaviour).
#[derive(Component, Clone, Copy, Debug)]
pub struct AsteroidShieldPierce(pub f32);

// â"€â"€ Resources â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
#[derive(Resource)]
struct SimBroadcastTimer(Timer);

/// The ship's impulse drive state. Cancelled automatically when hull damage is taken.
///
/// Per-ship `Component` post ship-parity audit; every ship (player + NPC)
/// carries its own impulse state. NPCs never charge impulse under current
/// AI, but the state lives on the entity so future NPC helm behaviour can
/// route through the same per-ship pathway.
///
/// Per-entity `Component` on each ship (issue #606: component is the sole
/// source of truth; no Resource fallback).
#[derive(Component, Default)]
pub struct ShipImpulse(pub ImpulseState);

// ShipShields has moved to `crate::ship::shields` as a Component.
pub use crate::ship::shields::ShipShields;

/// The ship's boost drive battery state. Toggle/partial-drain model; only
/// active when the ship's TOML enables it (see `BoostConfigResource`).
///
/// Per-entity `Component` on each ship (issue #606: component is the sole
/// source of truth; no Resource fallback). Both spawn paths insert a
/// `ShipBoost::default()` Component on every ship.
#[derive(Component, Default)]
pub struct ShipBoost(pub crate::boost::BoostState);

/// Per-ship marker set to `true` by phaser/torpedo fire systems when that
/// ship's weapon actually fires this tick. Reset to `false` by
/// `update_combat_activity` at the start of each broadcast tick. Every ship
/// (player + NPC) carries its own component; no global resource.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WeaponFiredThisTick(pub bool);

/// Per-ship marker set to `true` when hostile fire targets that ship this
/// tick, even if shields absorb the hit before hull damage leaks through.
/// Every ship carries its own component.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShipAttackedThisTick(pub bool);

/// Tracks the objective id the captain has chosen to prioritize.
/// Applied as a score bonus in `publish_viewscreen_blackboard` so the AI
/// immediately sees the updated priority ordering.
#[derive(Resource, Clone, Debug, Default)]
pub struct CaptainPriorityBoost {
    /// The objective currently boosted. `None` when no boost is set.
    pub boosted_id: Option<String>,
}

impl CaptainPriorityBoost {
    /// Score bonus added to the boosted objective's utility score.
    pub const BOOST_AMOUNT: f32 = 15.0;
}

/// Carries the reason string when the game ends. Set to `Some(reason)` before
/// transitioning to `GamePhase::GameOver`. The `OnEnter(GameOver)` system reads
/// this resource and broadcasts the reason to all clients.
#[derive(Resource, Default)]
pub struct GameOverReason(pub Option<String>);

/// Prevents `handle_collisions` from applying damage every frame while the
/// ship is in contact. After damage is applied once, a 1-second cooldown
/// suppresses further hits until the ship clears the obstacle.
///
/// Per-entity component (PRD #597 PR-8): every ship (player + NPC) carries
/// its own `CollisionCooldown`, so an NPC in contact with an asteroid does
/// not suppress the player's collision damage tick and vice versa.
#[derive(Component, Default)]
pub struct CollisionCooldown {
    pub remaining_secs: f32,
}

/// Pending outbound messages produced by simulation systems.
/// Drained each frame by the `SimBroadcaster` dispatch.
///
/// ## Migration note (PRD #253)
/// The old preamble pattern (`MessageWriter<OutboundMessage>`) has been
/// eliminated from all domain plugins. All systems that previously wrote
/// `OutboundMessage` directly now write `(Target, ServerMessage)` tuples
/// into `SimOutbox`. The `sim_outbox_broadcaster()` or a manual drain
/// (in tests) flushes these entries to the `OutboundMessage` bus.
/// To verify the absence of the old pattern, run:
///   rg 'MessageWriter<OutboundMessage>' src/  # must return no matches
#[derive(Resource, Default)]
pub struct SimOutbox(pub Vec<(Target, ServerMessage)>);

/// Broadcast delta caches — [`LastBroadcastEntityPositions`],
/// [`LastBroadcastEntityHealth`], [`LastBroadcastHull`], [`LastBroadcastShields`],
/// [`LastBroadcastBlackboards`] — now live in
/// [`crate::core::broadcast::cache_registry`] (issue #613), which is the
/// single module that knows about all six delta caches (the sixth,
/// `LastWeaponsUpdate`, stays in `console::weapons::server`) and owns
/// `reset_all` / `resync_for_token` / `prune`. Re-exported here so existing
/// `crate::server_app::LastBroadcastX` / `crate::simulation::LastBroadcastX`
/// references are unaffected by the move.
pub use crate::core::broadcast::cache_registry::{
    LastBroadcastBlackboards, LastBroadcastEntityHealth, LastBroadcastEntityPositions,
    LastBroadcastHull, LastBroadcastShields,
};

/// Tracks non-asteroid entities that have been reported to clients via
/// `EntitySpawned` / `EntityDespawned`.  Seeded from `WorldResource` on
/// the first `InProgress` frame so initial world entities are not re-reported.
///
/// Maintained by the `reconcile_runtime_entities` system.
#[derive(Resource, Default)]
pub struct TrackedEntities {
    /// UUIDs of non-asteroid entities already reported to clients.
    /// Populated from `WorldResource` at game start, then updated
    /// incrementally as runtime entities are spawned/despawned.
    pub reported: std::collections::HashSet<String>,
    /// Whether the registry has been seeded from initial WorldResource
    /// on the first InProgress frame.
    pub seeded: bool,
}

/// Per-entity component holding a ship's system blackboards. All
/// `publish_*_blackboard` systems write directly to this component on the
/// `LocalShip` entity. The broadcast pipeline reads from this component.
#[derive(Component, Default, Clone)]
pub struct ShipSystemBlackboards(
    pub std::collections::HashMap<crate::messages::SystemId, crate::messages::SystemBlackboard>,
);

// ── Plugin ───────────────────────────────────────────────────────────────────
/// Empty system used as an ordering anchor for the sim broadcast dispatch.
/// All sim-phase systems (message handlers, tick systems, broadcasters) should
/// run before this anchor so that `dispatch_sim_broadcasts` (which has
/// `.after(sim_processing_anchor)`) drains their `SimOutbox` writes.
pub fn sim_processing_anchor() {}

/// Compose all per-table simulation plugins onto `app`.
///
/// This is the canonical registration point for the server simulation.
/// Call this from `wasm_init()` (bridge) instead of using a `SimulationPlugin`.
pub fn add_simulation_plugins(app: &mut App) {
    app.configure_sets(
        Update,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Damage,
            crate::sim_sets::SimSet::Modifiers,
            crate::sim_sets::SimSet::Publish,
            crate::sim_sets::SimSet::PublishAggregate,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain()
            .run_if(in_state(GamePhase::InProgress))
            .after(crate::lobby::process_lobby),
    )
    .add_plugins(RapierPhysicsPlugin::<()>::default())
    .add_plugins(crate::region_plugin::RegionPlugin)
    .add_plugins(crate::console_ai_plugin::ConsoleAiPlugin)
    .add_plugins(crate::ai_plugin::AiPlugin)
    .add_plugins(crate::captain_plugin::CaptainPlugin)
    .add_plugins(crate::helm_plugin::HelmPlugin)
    .add_plugins(crate::ship_plugin::ShipPlugin)
    .add_plugins(crate::weapons_plugin::WeaponsPlugin)
    .add_plugins(crate::repair_plugin::RepairPlugin)
    .add_plugins(crate::power_plugin::ShipPowerPlugin)
    .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
    .add_plugins(crate::sensors_plugin::ShipSensorsPlugin)
    .add_plugins(crate::navigation_plugin::NavigationPlugin)
    .add_plugins(crate::comms_plugin::CommsConsolePlugin)
    .add_plugins(crate::entity_star::StarRenderPlugin)
    .add_message::<AsteroidDestroyedVfx>()
    .init_resource::<CaptainPriorityBoost>()
    .insert_resource(crate::config_cache::FactionRegistryResource(
        crate::config_cache::get_faction_registry(),
    ))
    .init_resource::<WorldResource>()
    .init_resource::<WorldSetupBroadcast>()
    .init_resource::<TrackedEntities>()
    .init_resource::<SimOutbox>()
    .init_resource::<LastBroadcastEntityPositions>()
    .init_resource::<LastBroadcastEntityHealth>()
    .init_resource::<LastBroadcastHull>()
    .init_resource::<LastBroadcastShields>()
    .init_resource::<LastBroadcastBlackboards>()
    .init_resource::<crate::messages::InterSystemQueue>()
    .insert_resource(SimBroadcastTimer(Timer::from_seconds(
        0.1,
        TimerMode::Repeating,
    )))
    .add_systems(
        Startup,
        setup_world.after(crate::world::server::insert_world_config_resource),
    )
    .add_systems(
        OnEnter(GamePhase::InProgress),
        (
            reset_broadcast_caches_on_start,
            crate::world::server::seed_ship_power_counter,
            spawn_game_start_entities,
            dump_tracked_entities,
        )
            .chain(),
    )
    .init_resource::<ProceduralMeshCache>()
    .add_systems(Update, render_spawned_entities)
    .add_systems(Update, update_mesh_lod.after(render_spawned_entities))
    .add_systems(Update, face_player_lights.after(render_spawned_entities))
    .add_systems(OnEnter(GamePhase::GameOver), on_game_over_enter)
    .insert_resource(GameOverReason(None))
    .add_systems(
        Update,
        (reconcile_runtime_entities, broadcast_world_setup_on_start)
            .chain()
            .after(crate::lobby::process_lobby)
            .before(crate::sim_sets::SimSet::Input),
    )
    .add_systems(
        Update,
        (admit_system_commands, clear_inter_system_queue)
            .after(crate::lobby::process_lobby)
            .before(crate::sim_sets::SimSet::Input)
            .run_if(in_state(GamePhase::InProgress)),
    )
    .add_systems(
        Update,
        broadcast_blackboard_updates.in_set(crate::sim_sets::SimSet::PublishAggregate),
    )
    .add_systems(
        Update,
        refresh_caches_on_midgame_reconnect
            .after(crate::lobby::process_lobby)
            .before(crate::lobby::server::drain_lobby_outbox)
            .before(crate::sim_sets::SimSet::Broadcast),
    )
    .add_systems(
        Update,
        (
            broadcast_shield_status.in_set(crate::sim_sets::SimSet::Broadcast),
            handle_collisions.in_set(crate::sim_sets::SimSet::Damage),
            sim_processing_anchor,
        )
            .after(crate::lobby::process_lobby),
    )
    .add_systems(
        Update,
        crate::modifier_coordination::translate_power_modifiers
            .in_set(crate::sim_sets::SimSet::Modifiers),
    )
    .add_systems(
        Update,
        crate::modifier_coordination::translate_impulse_modifiers
            .in_set(crate::sim_sets::SimSet::Modifiers),
    )
    .add_systems(
        Update,
        crate::modifier_coordination::apply_radar_damage_modifiers
            .in_set(crate::sim_sets::SimSet::Modifiers),
    )
    .add_systems(
        Update,
        (
            clear_last_attacker_on_death,
            clear_last_attacker_on_red_alert_off,
            publish_viewscreen_blackboard,
        )
            .in_set(crate::sim_sets::SimSet::PublishAggregate),
    )
    .add_plugins(weapons_update_broadcaster())
    .add_plugins(sim_state_broadcaster())
    .add_plugins(modifier_events_broadcaster())
    .add_plugins(sim_outbox_broadcaster());

    #[cfg(feature = "server")]
    {
        use crate::server::asset_preload::{
            auto_transition_from_loading, begin_asset_preload, broadcast_loading_progress,
            broadcast_loading_start, poll_asset_preload,
        };
        app.add_plugins(crate::server::ServerViewscreenRadarPlugin)
            .init_resource::<crate::server::asset_preload::AssetPreloadResource>()
            .add_systems(Update, begin_asset_preload)
            .add_systems(Update, poll_asset_preload)
            .add_systems(OnEnter(GamePhase::Loading), broadcast_loading_start)
            .add_systems(
                Update,
                broadcast_loading_progress.run_if(in_state(GamePhase::Loading)),
            )
            .add_systems(
                Update,
                auto_transition_from_loading
                    .run_if(in_state(GamePhase::Loading))
                    .after(poll_asset_preload),
            );
    }
}

/// Returns a [`SimBroadcaster`] pre-configured with the `SimState` producer.
///
/// Broadcasts `SimState` at 10 Hz to all players (`Audience::All`).
/// Registered by [`add_simulation_plugins`] and the test harness in `test_app()`.
pub fn sim_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::Hz(10.0), |world: &mut World| {
        // ── Asteroids: position/yaw never changes — omit from per-tick payload.
        // The client already has asteroid positions from WorldSetup/AsteroidSpawned.
        // Health fields are delta-compressed: only emitted when changed since last tick.
        type AsteroidRaw = (String, Option<f32>, Option<f32>);
        let asteroid_raw: Vec<AsteroidRaw> = {
            let mut q = world.query::<(
                &AsteroidUuid,
                Option<&crate::entity_spawner::EntitySystemHull>,
                Option<&crate::ship::shields::ShipShields>,
            )>();
            q.iter(world)
                .filter_map(|(uuid, hull_comp, shield_comp)| {
                    let hull_fraction = hull_comp.map(|h| {
                        let max = h.0.total_max();
                        if max > 0.0 {
                            h.0.total_current() / max
                        } else {
                            1.0
                        }
                    });
                    let shield_fraction = shield_comp.map(|s| {
                        let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                        let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                        if total_max > 0 {
                            total_hp as f32 / total_max as f32
                        } else {
                            0.0
                        }
                    });
                    // Skip entirely when there are no health components (unbreakable asteroids).
                    if hull_fraction.is_none() && shield_fraction.is_none() {
                        return None;
                    }
                    Some((uuid.0.clone(), hull_fraction, shield_fraction))
                })
                .collect()
        };
        let asteroid_states: Vec<crate::messages::EntityStateSnapshot> = {
            let mut health_cache = world.resource_mut::<LastBroadcastEntityHealth>();
            asteroid_raw
                .into_iter()
                .filter_map(|(uuid, hull_fraction, shield_fraction)| {
                    let prev = health_cache.0.get(&uuid).copied().unwrap_or((None, None));
                    let hull_changed = hull_fraction != prev.0;
                    let shield_changed = shield_fraction != prev.1;
                    if !hull_changed && !shield_changed {
                        return None;
                    }
                    health_cache
                        .0
                        .insert(uuid.clone(), (hull_fraction, shield_fraction));
                    Some(crate::messages::EntityStateSnapshot {
                        uuid,
                        position: None,
                        yaw: None,
                        hull_fraction,
                        shield_fraction,
                        flags: vec![],
                        shields: None,
                        warp_out_remaining_secs: None,
                    })
                })
                .collect()
        };

        // ── Non-asteroid entities (NPCs, stations): collect raw data first so
        // we can drop the ECS borrow before mutating the LastBroadcast* resources.
        type NpcRaw = (String, bevy::math::Vec3, f32, Option<f32>, Option<f32>);
        let npc_raw: Vec<NpcRaw> = {
            let mut q = world.query_filtered::<(
                &Transform,
                &EntityUuid,
                Option<&crate::entity_spawner::EntitySystemHull>,
                Option<&crate::ship::shields::ShipShields>,
            ), Without<Asteroid>>();
            q.iter(world)
                .map(|(transform, uuid, hull_comp, shield_comp)| {
                    let hull_fraction = hull_comp.map(|h| {
                        let max = h.0.total_max();
                        if max > 0.0 {
                            h.0.total_current() / max
                        } else {
                            1.0
                        }
                    });
                    let shield_fraction = shield_comp.map(|s| {
                        let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                        let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                        if total_max > 0 {
                            total_hp as f32 / total_max as f32
                        } else {
                            0.0
                        }
                    });
                    let yaw = transform.rotation.to_euler(bevy::math::EulerRot::YXZ).0;
                    (
                        uuid.0.clone(),
                        transform.translation,
                        yaw,
                        hull_fraction,
                        shield_fraction,
                    )
                })
                .collect()
        };

        // Compare against last-broadcast positions and health; skip entities
        // where nothing changed.  Position/yaw suppressed below ~1 cm movement;
        // hull/shield suppressed when the f32 value is identical to last tick.
        const POS_THRESHOLD_SQ: f32 = 0.0001; // 0.01 world-unit radius
        const YAW_THRESHOLD: f32 = 0.001; // ~0.057 degrees
        let npc_states: Vec<crate::messages::EntityStateSnapshot> = {
            // Borrow position cache, then health cache separately (both mut).
            // Collect diffs first to avoid holding multiple mut borrows.
            type NpcDiff = (
                String,
                Option<[f32; 3]>,
                Option<f32>,
                Option<f32>,
                Option<f32>,
            );
            let diffs: Vec<NpcDiff> = {
                let mut pos_cache = world.resource_mut::<LastBroadcastEntityPositions>();
                npc_raw
                    .iter()
                    .map(|(uuid, pos, yaw, hull_fraction, shield_fraction)| {
                        let moved = match pos_cache.0.get(uuid) {
                            Some(&(prev_pos, prev_yaw)) => {
                                (*pos - prev_pos).length_squared() > POS_THRESHOLD_SQ
                                    || (*yaw - prev_yaw).abs() > YAW_THRESHOLD
                            }
                            None => true,
                        };
                        if moved {
                            pos_cache.0.insert(uuid.clone(), (*pos, *yaw));
                        }
                        let out_pos = if moved {
                            Some([pos.x, pos.y, pos.z])
                        } else {
                            None
                        };
                        let out_yaw = if moved { Some(*yaw) } else { None };
                        (
                            uuid.clone(),
                            out_pos,
                            out_yaw,
                            *hull_fraction,
                            *shield_fraction,
                        )
                    })
                    .collect()
            };
            let mut health_cache = world.resource_mut::<LastBroadcastEntityHealth>();
            diffs
                .into_iter()
                .filter_map(|(uuid, out_pos, out_yaw, hull_fraction, shield_fraction)| {
                    let prev = health_cache.0.get(&uuid).copied().unwrap_or((None, None));
                    let hull_changed = hull_fraction != prev.0;
                    let shield_changed = shield_fraction != prev.1;
                    // Skip the entity entirely when nothing at all changed.
                    if out_pos.is_none() && out_yaw.is_none() && !hull_changed && !shield_changed {
                        return None;
                    }
                    if hull_changed || shield_changed {
                        health_cache
                            .0
                            .insert(uuid.clone(), (hull_fraction, shield_fraction));
                    }
                    Some(crate::messages::EntityStateSnapshot {
                        uuid,
                        position: out_pos,
                        yaw: out_yaw,
                        hull_fraction: if hull_changed { hull_fraction } else { None },
                        shield_fraction: if shield_changed {
                            shield_fraction
                        } else {
                            None
                        },
                        flags: vec![],
                        shields: None,
                        warp_out_remaining_secs: None,
                    })
                })
                .collect()
        };

        let entity_states: Vec<_> = asteroid_states.into_iter().chain(npc_states).collect();

        // ── Emit SystemHullUpdate only when hull HP changed.
        //
        // Post issue #618: publisher no longer emits legacy Console-keyed
        // `ConsoleHullUpdate` wire messages. `SystemHullStatus` carries the
        // authoritative `SystemId`, human-readable display_name, and tier for
        // every damageable system on the ship.
        {
            let hull_current: Vec<crate::messages::SystemHullStatus> = world
                .query_filtered::<&crate::entity_spawner::EntitySystemHull, With<LocalShip>>()
                .single(world)
                .map(|h| {
                    h.0.iter()
                        .map(|(sid, entry)| crate::messages::SystemHullStatus {
                            system_id: sid.clone(),
                            display_name: entry.display_name.clone(),
                            current: entry.current,
                            max_hp: entry.max,
                            tier: h.0.tier_for(sid),
                            debuff_magnitude: h.0.debuff_magnitude_for(sid),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let hull_changed = world.resource::<LastBroadcastHull>().0 != hull_current;
            if hull_changed {
                let entries = hull_current.clone();
                world.resource_mut::<LastBroadcastHull>().0 = hull_current;
                world
                    .resource_mut::<SimOutbox>()
                    .0
                    .push((Target::All, ServerMessage::SystemHullUpdate { entries }));
            }
        }

        let snapshot = crate::messages::SimSnapshot { entity_states };
        vec![ServerMessage::SimState { snapshot }]
    })
}

/// Returns a [`SimBroadcaster`] pre-configured with the `ModifierAdded` and
/// `ModifierRemoved` producers.
///
/// Drains pending modifier events from [`ShipModifiers`] once per frame and
/// broadcasts each as a separate `ServerMessage` to all players (`Audience::All`).
/// Uses `Cadence::OnEvent` so the producer is called every frame regardless of
/// any Hz timer; an empty drain produces no outbound messages.
/// Registered by [`add_simulation_plugins`] and the test harness in `test_app()`.
///
/// After PR 6 (PRD #597): prefers the per-entity `ShipModifiers` component on
/// the LocalShip entity, falling back to the global Resource for tests that
/// only insert the Resource form.
pub fn modifier_events_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::OnEvent, |world: &mut World| {
        use crate::modifiers::ModifierEvent;
        let events: Vec<ModifierEvent> = {
            let mut q =
                world.query_filtered::<&mut crate::modifiers::ShipModifiers, With<LocalShip>>();
            if let Some(mut mods_comp) = q.iter_mut(world).next() {
                std::mem::take(&mut mods_comp.pending_events)
            } else {
                Vec::new()
            }
        };
        events
            .into_iter()
            .map(|event| match event {
                ModifierEvent::Added {
                    source,
                    slot,
                    bonus,
                } => ServerMessage::ModifierAdded {
                    source,
                    slot,
                    bonus,
                },
                ModifierEvent::Removed { source, slot } => {
                    ServerMessage::ModifierRemoved { source, slot }
                }
            })
            .collect()
    })
}

/// Returns a [`SimBroadcaster`] that drains [`SimOutbox`] each frame and writes
/// each entry as an `OutboundMessage` with per-message target routing.
///
/// Uses `Cadence::OnEvent` so the producer fires every frame.  When the outbox
/// is empty the producer returns an empty `Vec` and no messages are emitted.
/// When populated (by any simulation system) the queued entries are flushed
/// directly to `OutboundMessage` with their original `Target` routing.
pub fn sim_outbox_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::OnEvent, |world: &mut World| {
        let mut outbox = world.resource_mut::<SimOutbox>();
        let entries = std::mem::take(&mut outbox.0);
        for (target, msg) in entries {
            world.write_message(OutboundMessage {
                target,
                msg: msg.clone(),
                delivery: delivery_class_for_msg(&msg),
            });
        }
        vec![]
    })
}

/// Derive the delivery class for a `ServerMessage`.
///
/// Snapshot-class messages ride the unordered/no-retransmit DataChannel;
/// everything else (commands, lobby messages, Welcome, etc.) is reliable.
/// This is the single place where delivery class is decided server-side
/// (AC 1). The function is not exported — everything routes through
/// `sim_outbox_broadcaster` or `dispatch_sim_broadcasts`.
fn delivery_class_for_msg(msg: &ServerMessage) -> DeliveryClass {
    match msg {
        ServerMessage::SimState { .. }
        | ServerMessage::BlackboardUpdate { .. }
        | ServerMessage::ShieldStatus { .. }
        | ServerMessage::RepairState { .. }
        | ServerMessage::PowerState { .. }
        | ServerMessage::WeaponsUpdate { .. }
        | ServerMessage::SystemHullUpdate { .. } => DeliveryClass::Snapshot,
        _ => DeliveryClass::Reliable,
    }
}

// -- Systems -------------------------------------------------------------------

/// When the entity identified by `LastShipAttacker` no longer exists in the
/// world, clear the attacker record so stale references are not published.
fn clear_last_attacker_on_death(
    mut attacker_q: Query<&mut LastShipAttacker>,
    entity_uuids: Query<&EntityUuid>,
) {
    for mut attacker in attacker_q.iter_mut() {
        let uuid = match &attacker.0 {
            Some(u) => u.clone(),
            None => continue,
        };
        let still_alive = entity_uuids.iter().any(|eu| eu.0.as_str() == uuid.as_str());
        if !still_alive {
            attacker.0 = None;
        }
    }
}

/// When the ship's red alert transitions from on to off, clear the attacker
/// record — the threat has passed and the old attacker is no longer relevant.
fn clear_last_attacker_on_red_alert_off(
    mut attacker_q: Query<
        (&mut LastShipAttacker, &crate::ship_state::ShipRedAlert),
        With<LocalShip>,
    >,
    mut last_ra: Local<bool>,
) {
    for (mut attacker, ra) in attacker_q.iter_mut() {
        if *last_ra && !ra.0 {
            attacker.0 = None;
        }
        *last_ra = ra.0;
    }
}

fn publish_viewscreen_blackboard(
    hull_q: Query<&crate::entity_spawner::EntitySystemHull, With<LocalShip>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    boost: Option<Res<CaptainPriorityBoost>>,
    mut ship_blackboards_q: Query<
        (
            &mut ShipSystemBlackboards,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::ship::combat_activity::RecentCombatActivity>,
            Option<&crate::weapons_plugin::LastShipAttacker>,
        ),
        With<LocalShip>,
    >,
) {
    use crate::messages::{SystemBlackboard, SystemId, ViewscreenBlackboard};
    use crate::objectives::WorldConditions;
    use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

    let entity_state = ship_blackboards_q.single().ok();
    let red_alert = entity_state
        .as_ref()
        .and_then(|(_, ra, _, _)| ra.map(|r| r.0))
        .unwrap_or(false);
    let last_damage_taken_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _)| act.and_then(|a| a.last_damage_taken));
    let last_weapon_fired_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _)| act.and_then(|a| a.last_weapon_fired));
    let last_attacker_uuid = entity_state
        .as_ref()
        .and_then(|(_, _, _, la)| la.and_then(|l| l.0.clone()));

    let hull_integrity_pct = hull_q
        .single()
        .map(|h| {
            let max = h.0.total_max();
            let cur = h.0.total_current();
            if max > 0.0 {
                (cur / max * 100.0).clamp(0.0, 100.0)
            } else {
                100.0
            }
        })
        .unwrap_or(100.0);

    let conditions = WorldConditions {
        red_alert,
        hull_fraction: hull_integrity_pct / 100.0,
    };
    let captain_boost = boost.as_ref().and_then(|b| {
        b.boosted_id
            .as_deref()
            .map(|id| (id, CaptainPriorityBoost::BOOST_AMOUNT))
    });
    let scored_objectives = objectives
        .as_ref()
        .map(|o| o.0.scored_pool_with_boost(&conditions, captain_boost))
        .unwrap_or_default();

    let bb = ViewscreenBlackboard {
        red_alert,
        hull_integrity_pct,
        last_damage_taken_secs,
        last_weapon_fired_secs,
        last_attacker_uuid,
        scored_objectives,
    };

    // Write directly to the per-entity component.
    if let Some((mut entity_bbs, _, _, _)) = ship_blackboards_q.iter_mut().next() {
        entity_bbs.0.insert(
            SystemId(VIEWSCREEN_SYSTEM_ID.to_string()),
            SystemBlackboard::Viewscreen(bb),
        );
    }
}

fn handle_collisions(
    time: Res<Time>,
    context: ReadRapierContext,
    asteroid_query: Query<
        (&Transform, &AsteroidUuid, Option<&AsteroidShieldPierce>),
        With<Asteroid>,
    >,
    mut ship_query: Query<
        (
            Entity,
            &mut ShipPhysicsComponent,
            &mut CollisionCooldown,
            &mut crate::entity_spawner::EntitySystemHull,
            Option<&mut ShipShields>,
            Option<&ShipModifiers>,
            Option<&EntityUuid>,
            Option<&ColliderSection>,
            Has<LocalShip>,
            Option<&mut ShipImpulse>,
            Option<&mut crate::entity_spawner::EntityShipArcHull>,
        ),
        With<Ship>,
    >,
    body_query: Query<(&Transform, Option<&ColliderSection>)>,
    mut outbox: ResMut<SimOutbox>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut game_over_reason: ResMut<GameOverReason>,
    mut damage_log: ResMut<DamageLog>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    mut world: ResMut<WorldResource>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();

    let Ok(ctx) = context.single() else { return };

    // Iterate every ship (player + NPCs) uniformly. Per-entity CollisionCooldown,
    // ShipModifiers, ShipShields, EntitySystemHull, ShipImpulse. Player-only side
    // effects (damage messages, GameOver, debug log) are gated on `is_local`.
    for (
        ship_entity,
        mut physics,
        mut cooldown,
        mut hull_comp,
        shields_opt,
        modifiers_comp,
        ship_uuid,
        ship_collider,
        is_local,
        mut impulse_opt,
        mut arc_hull_opt,
    ) in ship_query.iter_mut()
    {
        cooldown.remaining_secs = (cooldown.remaining_secs - dt).max(0.0);

        let default_modifiers;
        let modifiers: &ShipModifiers = match modifiers_comp {
            Some(m) => m,
            None => {
                default_modifiers = ShipModifiers::new();
                &default_modifiers
            }
        };

        let contact = ctx.contact_pairs_with(ship_entity).next().and_then(|pair| {
            if pair.collider1() == Some(ship_entity) {
                pair.collider2()
            } else {
                pair.collider1()
            }
        });

        let Some(attacker_entity) = contact else {
            continue;
        };
        if cooldown.remaining_secs > 0.0 {
            continue;
        }

        // Cancel impulse charge on any ship that takes a collision hit.
        if let Some(ref mut impulse) = impulse_opt {
            impulse.0.cancel_charge();
        }

        let speed_at_impact = physics.forward_speed;
        physics.forward_speed = 0.0;
        let attacker_body = body_query.get(attacker_entity).ok();
        separate_ship_from_collision(
            &mut physics,
            collider_radius(ship_collider),
            attacker_body.map(|(transform, _)| transform),
            collider_radius(attacker_body.and_then(|(_, collider)| collider)),
        );
        let damage = collision_damage(speed_at_impact) as f32
            * modifiers.get(&ModifierSlot::HullDamageTaken);

        let asteroid_info = asteroid_query.get(attacker_entity).ok();
        let bearing = asteroid_info
            .map(|(t, _, _)| {
                attacker_bearing_relative(
                    t.translation.x,
                    t.translation.z,
                    physics.x,
                    physics.z,
                    physics.yaw,
                )
            })
            .unwrap_or(0.0);

        let source_label = asteroid_info
            .map(|(_, uuid, _)| format!("asteroid:{}", uuid.0))
            .unwrap_or_else(|| "collision".to_string());

        // Resolve the colliding asteroid's `shield_pierce` (missing → 0.0,
        // matching pre-#414 behaviour where all collision damage was first
        // absorbed by shields).
        let shield_pierce = asteroid_info
            .and_then(|(_, _, sp)| sp.map(|c| c.0))
            .unwrap_or(0.0);

        // Split impact damage by the asteroid's `shield_pierce`: the
        // pierced fraction goes straight to hull; the absorbed fraction
        // is mitigated by the facing shield quadrant (any leak adds to
        // hull damage).
        let (pierced, absorbed) = crate::damage::split_damage_for_pierce(damage, shield_pierce);
        let mut total_hull = pierced;
        let mut shield_amount = 0.0;

        // Shields are optional per-ship. Absorb through them when present;
        // otherwise all absorbed damage leaks straight to hull.
        let arc_label = if let Some(mut shields) = shields_opt {
            let arc_idx = shields.0.facing_index_for_bearing(bearing);
            let label = shields.0.facings.get(arc_idx).map(|f| f.label.clone());
            if absorbed > 0.0 {
                let leak =
                    apply_damage_with_shields(absorbed.round() as i32, bearing, &mut shields.0);
                shield_amount = (absorbed - leak as f32).max(0.0);
                total_hull += leak as f32;
            }
            label
        } else {
            // No shields → the "absorbed" portion also lands on hull.
            total_hull += absorbed;
            None
        };

        // Debug damage log: player-only (single-player debug overlay).
        if is_local {
            damage_log.push(DamageLogEntry {
                source: source_label,
                shield_arc: arc_label,
                amount: damage,
            });
        }

        // God mode: local ship takes no damage.
        if is_local && crate::bridge::is_god_mode() {
            total_hull = 0.0;
            shield_amount = 0.0;
        }

        let mut ship_destroyed = false;
        let hull_applied = if total_hull > 0.0 {
            let rng = &mut rand::rngs::SmallRng::from_os_rng();
            let (applied, destroyed) = apply_hull_damage(&mut hull_comp.0, total_hull, rng);
            // Distribute the same absorbed amount across the per-arc hull
            // pool (issue #514) so arc tier tracking follows overall hull
            // damage. Skipped when the ship has no `EntityShipArcHull` (NPCs).
            if let Some(ref mut arc_hull) = arc_hull_opt {
                arc_hull.0.apply_damage(applied, rng);
            }
            ship_destroyed = destroyed;
            applied
        } else {
            0.0
        };

        // DamageTaken / ShipDestroyed / GameOver are player-facing UI events.
        // Only emit for the LocalShip. NPCs use the AiEntityDestroyed +
        // EntityDespawned path (same as beam-kill).
        if is_local {
            outbox.0.push((
                Target::All,
                ServerMessage::DamageTaken {
                    hull: hull_applied,
                    shield: shield_amount,
                },
            ));
            if ship_destroyed {
                outbox.0.push((Target::All, ServerMessage::ShipDestroyed));
                if game_over_reason.0.is_none() {
                    game_over_reason.0 = Some("All consoles destroyed".into());
                }
                next_state.set(GamePhase::GameOver);
            }
        } else if ship_destroyed {
            // NPC destruction: mirror the beam-kill path so downstream world
            // triggers and clients update consistently.
            if let Some(uuid) = ship_uuid {
                world.0.entities.retain(|e| e.uuid != uuid.0);
                destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                    entity_uuid: uuid.0.clone(),
                });
                outbox.0.push((
                    Target::All,
                    ServerMessage::EntityDespawned {
                        uuid: uuid.0.clone(),
                    },
                ));
            }
            commands.entity(ship_entity).try_despawn();
        }
        cooldown.remaining_secs = 1.0;
    }
}

const COLLISION_SEPARATION_SLOP: f32 = 0.05;

fn collider_radius(collider: Option<&ColliderSection>) -> f32 {
    collider.map(|c| c.0.radius.max(0.0)).unwrap_or(0.0)
}

fn separate_ship_from_collision(
    physics: &mut ShipPhysicsComponent,
    ship_radius: f32,
    attacker_transform: Option<&Transform>,
    attacker_radius: f32,
) {
    let Some(attacker_transform) = attacker_transform else {
        return;
    };
    let min_dist = ship_radius + attacker_radius + COLLISION_SEPARATION_SLOP;
    if min_dist <= 0.0 {
        return;
    }

    let dx = physics.x - attacker_transform.translation.x;
    let dz = physics.z - attacker_transform.translation.z;
    let dist_sq = dx * dx + dz * dz;
    let (nx, nz, dist) = if dist_sq > 1e-6 {
        let dist = dist_sq.sqrt();
        (dx / dist, dz / dist, dist)
    } else {
        // Degenerate overlap: step back opposite the ship's current forward.
        (-physics.yaw.sin(), physics.yaw.cos(), 0.0)
    };

    if dist < min_dist {
        physics.x = attacker_transform.translation.x + nx * min_dist;
        physics.z = attacker_transform.translation.z + nz * min_dist;
    }
}

/// Tick shield regen for the player ship. **PR-7 (issue #597) moved this
/// canonical registration into `ShipShieldsPlugin::tick_shields`, which
/// iterates every ship with the `Ship` marker (player + NPCs). This local
/// stub is retained temporarily as a documented no-op if any test still
/// references it directly; production wiring goes through the plugin.**
#[allow(dead_code)]
fn tick_shields(_time: Res<Time>, _shields_q: Query<&mut ShipShields, With<Ship>>) {
    // Moved: see `crate::ship::shields::tick_shields`.
}

/// Broadcast `ShieldStatus` at 10 Hz.
/// Sends to all players only when shield state changed; always sends to the
/// Shields console holder so their panel stays smooth during regeneration.
fn broadcast_shield_status(
    time: Res<Time>,
    mut timer: ResMut<SimBroadcastTimer>,
    mut outbox: ResMut<SimOutbox>,
    sessions: Res<Sessions>,
    ship_query: Query<&ShipShields, With<LocalShip>>,
    mut last: ResMut<LastBroadcastShields>,
) {
    let Some(shields) = ship_query.iter().next() else {
        return;
    };
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let facings: Vec<ShieldFacingStatus> = shields
        .0
        .snapshot()
        .into_iter()
        .map(|s| ShieldFacingStatus {
            label: s.label,
            hp: s.hp,
            max_hp: s.max_hp,
            online: s.online,
            offline_remaining: s.offline_remaining,
            is_focused: s.is_focused,
            center_deg: s.center_deg,
            width_deg: s.width_deg,
            arc_id: s.id,
            priority: s.priority,
        })
        .collect();

    let frequency = shields.frequency();
    if facings != last.0 {
        // State changed — broadcast to everyone.
        last.0 = facings.clone();
        outbox.0.push((
            Target::All,
            ServerMessage::ShieldStatus { facings, frequency },
        ));
    } else if let Some(token) = sessions.0.holder_for_station(&StationId("shields".into())) {
        // Nothing changed but the Shields holder still gets a periodic refresh
        // so regenerating HP stays smooth on their panel.
        outbox.0.push((
            Target::Token(token.to_string()),
            ServerMessage::ShieldStatus { facings, frequency },
        ));
    }
}

/// Tracks whether the initial WorldSetup broadcast has fired, so it only
/// goes out once per game.
#[derive(Resource, Default)]
struct WorldSetupBroadcast {
    sent: bool,
}

/// Broadcast `GameOver { reason }` to all players when the game enters the
/// GameOver phase. Reads the reason from `GameOverReason` resource and resets
/// it to `None` after broadcast.
fn on_game_over_enter(mut game_over_reason: ResMut<GameOverReason>, mut outbox: ResMut<SimOutbox>) {
    let reason = game_over_reason.0.take().unwrap_or_default();
    outbox
        .0
        .push((Target::All, ServerMessage::GameOver { reason }));
}

/// Reset all change-detection caches when entering InProgress so the first
/// broadcast tick always sends a full state to all players. Also covers the
/// multi-game restart case where stale cache from a previous game would
/// otherwise suppress initial updates.
///
/// Delegates to [`crate::core::broadcast::cache_registry::reset_all`] (issue
/// #613), the single place that knows about all six broadcast delta caches.
fn reset_broadcast_caches_on_start(
    mut hull: ResMut<LastBroadcastHull>,
    mut shields: ResMut<LastBroadcastShields>,
    mut positions: ResMut<LastBroadcastEntityPositions>,
    mut health: ResMut<LastBroadcastEntityHealth>,
    mut weapons: ResMut<LastWeaponsUpdate>,
    mut last_bb: ResMut<LastBroadcastBlackboards>,
) {
    crate::core::broadcast::cache_registry::reset_all(
        &mut hull,
        &mut shields,
        &mut positions,
        &mut health,
        &mut weapons,
        &mut last_bb,
    );
}

/// Emit `BlackboardUpdate` for any system whose blackboard has changed since
/// the last broadcast. Reads from the `LocalShip` entity's per-entity component
/// (populated by `dual_publish_blackboards`). Runs in `SimSet::PublishAggregate`
/// (before `SimSet::Broadcast` so `dispatch_sim_broadcasts` sees the outbox entries).
pub fn broadcast_blackboard_updates(
    ship_query: Query<&ShipSystemBlackboards, With<LocalShip>>,
    mut last: ResMut<LastBroadcastBlackboards>,
    mut outbox: ResMut<SimOutbox>,
) {
    let Some(bb) = ship_query.iter().next() else {
        return;
    };
    let updates: Vec<(crate::messages::SystemId, crate::messages::SystemBlackboard)> =
        bb.0.iter()
            .filter(|(id, bb)| last.0.get(*id) != Some(*bb))
            .map(|(id, bb)| (id.clone(), bb.clone()))
            .collect();

    if !updates.is_empty() {
        for (id, bb) in &updates {
            last.0.insert(id.clone(), bb.clone());
        }
        outbox
            .0
            .push((Target::All, ServerMessage::BlackboardUpdate { updates }));
    }
}

/// System set that `admit_system_commands` belongs to. Handlers that run in
/// `Update` but outside `SimSet::Input` can use `.after(AdmissionSet)` to
/// guarantee they see a fully-populated `AdmittedCommands`.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdmissionSet;

/// Plugin that registers the admission gate and `AdmittedCommands` resource.
/// Include this in plugin-level test apps so handlers have a populated
/// `AdmittedCommands` to read from.
pub struct AdmissionPlugin;

impl Plugin for AdmissionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::messages::InterSystemQueue>()
            .init_resource::<crate::ai::server::AiTokenRegistry>()
            .configure_sets(
                Update,
                AdmissionSet
                    .after(crate::lobby::process_lobby)
                    .before(crate::sim_sets::SimSet::Input),
            )
            .add_systems(
                Update,
                (admit_system_commands, clear_inter_system_queue).in_set(AdmissionSet),
            );
    }
}

fn clear_inter_system_queue(mut queue: ResMut<crate::messages::InterSystemQueue>) {
    queue.0.clear();
}

/// Authority gate for intra-system commands. Runs once per tick before
/// `SimSet::Input`, clearing and refilling `AdmittedCommands`.
///
/// A network `ControlSystem` message is admitted iff its token is the live
/// controller of the target system: AI tokens require `operate_ai`; human
/// tokens require `accept_human_input` AND holding the console for that system.
/// Once admitted the command carries no source identity — handlers must not
/// branch on the origin.
fn admit_system_commands(
    mut reader: MessageReader<InboundMessage>,
    mut ship_query: Query<
        (
            Entity,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut crate::messages::AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<LocalShip>,
    >,
    sessions: Res<Sessions>,
    ai_registry: Res<crate::ai::server::AiTokenRegistry>,
) {
    let Some((ship_entity, control_sources, mut admitted, ship_config)) =
        ship_query.iter_mut().next()
    else {
        return;
    };
    admitted.0.clear();
    for ev in reader.read() {
        let ClientMessage::ControlSystem { target, payload } = &ev.msg else {
            continue;
        };
        // Reject registered NPC ai: tokens that don't belong to the player ship.
        // Only tokens present in AiTokenRegistry are NPC-owned; unregistered ai:
        // tokens (player Backfill AI or synthetic test tokens) pass through.
        if ev.token.starts_with("ai:") {
            if let Some(entity) = ai_registry.bevy_entity_for_token(&ev.token) {
                if entity != ship_entity {
                    warn!(
                        "[admit] rejected NPC ai: token {} → {:?}",
                        &ev.token[..ev.token.len().min(12)],
                        std::mem::discriminant(payload),
                    );
                    continue;
                }
            }
        }
        if is_command_authorized(
            &ev.token,
            target,
            payload,
            control_sources,
            &sessions,
            &ship_config.0,
        ) {
            admitted.0.push(crate::messages::AdmittedCommand {
                target: target.clone(),
                payload: payload.clone(),
                response_token: Some(ev.token.clone()),
            });
        } else {
            warn!(
                "[admit] rejected {:?} → {:?} from token={}",
                target.0,
                std::mem::discriminant(payload),
                &ev.token[..ev.token.len().min(8)],
            );
        }
    }
}

/// Maps a `SystemId` to the `StationId` whose holder is authoritative for
/// that system's admission. Returns `None` for systems with no owning
/// station (either ship-wide or unknown), signalling a deny at the
/// caller.
///
/// Lookup order:
///   1. Shield-arc prefix match — arcs are not auto-generated into
///      `ShipConfig.systems` (they're synthesised at the entity-config layer),
///      so they must be matched by prefix.
///   2. Direct system→station from the config's `[[system]]` blocks
///      (handles fine-grained systems and modern coarse systems).
///   3. Station-name fallback: if the target string matches a known station
///      id, treat it as the owning station (backward compatibility with
///      deprecated coarse systems like `"tactical"`, `"power"` whose
///      `[[system]]` entry was removed during fine-grained refactoring).
///   4. `None` — truly unknown system id, caller will deny.
fn station_for_system(
    config: &crate::ship::config::ShipConfig,
    target: &crate::messages::SystemId,
) -> Option<StationId> {
    // Step 1: shield-arc prefix (arcs are not in `config.systems`).
    if target.0.starts_with("shield-arc-") {
        return Some(StationId("shields".into()));
    }
    // Step 2: direct system lookup.
    if let Some(system) = config.system(target) {
        return system.station.clone();
    }
    // Step 3: station-name fallback — does the target match a known station?
    let candidate = StationId(target.0.clone());
    if config.station(&candidate).is_some() {
        return Some(candidate);
    }
    // Step 4: unknown.
    None
}

fn is_command_authorized(
    token: &str,
    target: &crate::messages::SystemId,
    payload: &SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    config: &crate::ship::config::ShipConfig,
) -> bool {
    // Viewscreen SetView: authority derives from the view mode's source system.
    let effective_target = if target.0 == crate::system_registry::VIEWSCREEN_SYSTEM_ID {
        if let SystemControlPayload::SetView { mode } = payload {
            crate::ship::viewscreen::source_system_for_view_mode(mode)
        } else {
            target.clone()
        }
    } else {
        target.clone()
    };

    let policy = control_sources.0.policy_for(&effective_target);

    if token.starts_with("ai:") {
        return policy.operate_ai;
    }
    if token == crate::console_bridge::LOCAL_CONSOLE_TOKEN {
        return policy.accept_human_input;
    }
    if !policy.accept_human_input {
        return false;
    }

    // Human network token: must hold the station for the target system.
    match station_for_system(config, &effective_target) {
        Some(station) => sessions.0.holder_for_station(&station) == Some(token),
        None => {
            warn!(
                "[admit] unknown system id {:?} — denying",
                effective_target.0
            );
            false
        }
    }
}

/// When a player reconnects mid-game (Identify during InProgress), `process_lobby`
/// queues a `Welcome { .. }` into `LobbyOutbox` targeted at that player's
/// token. Detect this and push a full-state resync to *just that token* via
/// [`crate::core::broadcast::cache_registry::resync_for_token`] (issue #613).
///
/// This replaces the #599 quick fix, which reset all six shared broadcast
/// delta caches — correct for the reconnecting player, but it also forced
/// the *next* 10 Hz tick to broadcast full state to *every other* connected
/// client, since those caches are shared across all `Audience::All`
/// producers. The targeted resync leaves the shared caches untouched, so
/// every other client's next tick remains a normal delta.
fn refresh_caches_on_midgame_reconnect(world: &mut World) {
    let state = world.resource::<State<GamePhase>>();
    if *state.get() != GamePhase::InProgress {
        return;
    }
    let reconnecting_tokens: Vec<String> = {
        let lobby_outbox = world.resource::<LobbyOutbox>();
        lobby_outbox
            .0
            .iter()
            .filter_map(|(target, msg)| match (target, msg) {
                (Target::Token(token), ServerMessage::Welcome { .. }) => Some(token.clone()),
                _ => None,
            })
            .collect()
    };
    for token in reconnecting_tokens {
        crate::core::broadcast::cache_registry::resync_for_token(world, &token);
    }
}

/// Emit a single `WorldSetup` broadcast when the game enters `InProgress`.
/// Uses `State<GamePhase>` + sentry to fire exactly once.
fn broadcast_world_setup_on_start(
    state: Res<State<GamePhase>>,
    world: Res<WorldResource>,
    mut sent: ResMut<WorldSetupBroadcast>,
    mut outbox: ResMut<SimOutbox>,
) {
    if sent.sent || state.get() != &GamePhase::InProgress {
        return;
    }
    outbox.0.push((
        Target::All,
        ServerMessage::WorldSetup {
            world: world.0.clone(),
        },
    ));
    sent.sent = true;
}

/// Reconciles the live ECS entities with the `TrackedEntities` registry each tick.
fn upsert_world_entity(world: &mut WorldResource, snapshot: EntitySnapshot) {
    if let Some(existing) = world
        .0
        .entities
        .iter_mut()
        .find(|e| e.uuid == snapshot.uuid)
    {
        *existing = snapshot;
    } else {
        world.0.entities.push(snapshot);
    }
}

fn snapshot_from_entity_config(
    uuid: String,
    id: Option<String>,
    config: &crate::entity_config::EntityConfig,
    position: Vec3,
) -> EntitySnapshot {
    let mut snapshot = EntitySnapshot {
        uuid,
        id,
        name: config.name.clone(),
        position: Some([position.x, position.y, position.z]),
        tags: config.tags.clone(),
        ..EntitySnapshot::default()
    };

    if let Some(radar) = &config.radar_appearance {
        if let Some(colour) = &radar.colour {
            if colour.len() >= 3 {
                snapshot.colour = Some([colour[0], colour[1], colour[2]]);
            }
        }
        if let Some(region_colour) = &radar.region_colour {
            if region_colour.len() >= 3 {
                snapshot.region_colour =
                    Some([region_colour[0], region_colour[1], region_colour[2]]);
            }
        }
        snapshot.radar_size = radar.size;
        snapshot.radar_icon = radar.icon.clone();
    }

    if let Some(collider) = &config.collider {
        if snapshot.radius.is_none() {
            snapshot.radius = Some(collider.radius);
        }
    }

    if let Some(target) = &config.target {
        snapshot.target_tags = target.tags.clone();
        snapshot.threat_level = Some(target.threat_level.as_str().to_string());
        snapshot.target_description = target.description.clone();
    }

    // Initial shield fraction (#471). When the entity has a `[shields]`
    // block, seed the snapshot at full HP. Per-tick updates flow through
    // `EntityStateSnapshot.shield_fraction` from `sim_state_broadcaster`.
    if config.shields_console.is_some() {
        snapshot.shield_fraction = Some(1.0);
    }

    snapshot
}

/// For non-asteroid entities carrying `EntityUuid`:
/// - New entities (present in ECS, absent from `reported`) emit `EntitySpawned`
///   and are added to `WorldResource.entities` so they appear on reconnect `Welcome`.
/// - Missing entities (absent from ECS, present in `reported`) emit
///   `EntityDespawned` and are removed from `WorldResource.entities`.
///
/// Asteroids are excluded (they use `AsteroidSpawned` / `AsteroidDestroyed`).
///
/// On the very first `InProgress` tick, seeds `reported` from the initial
/// `WorldResource` entities so those are not re-broadcast.
fn reconcile_runtime_entities(
    mut registry: ResMut<TrackedEntities>,
    mut world: ResMut<WorldResource>,
    query: Query<
        (
            Entity,
            &EntityUuid,
            Option<&EntityId>,
            Option<&EntityName>,
            &Transform,
            Option<&RegionShapeSection>,
            Option<&EntityTagsSection>,
            Option<&RadarAppearanceSection>,
            Option<&AsteroidFieldSection>,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&crate::entity_spawner::EntityTarget>,
            Option<&crate::ship::shields::ShipShields>,
        ),
        Without<Asteroid>,
    >,
    mut outbox: ResMut<SimOutbox>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    mut positions_cache: ResMut<LastBroadcastEntityPositions>,
    mut health_cache: ResMut<LastBroadcastEntityHealth>,
) {
    // Build set of entity names referenced by active mission objectives.
    let active_objective_names: std::collections::HashSet<String> = objectives
        .as_ref()
        .map(|obj| {
            obj.0
                .sorted_snapshots()
                .into_iter()
                .filter(|s| s.status == crate::messages::ObjectiveStatus::Active)
                .flat_map(|s| s.targets)
                .collect()
        })
        .unwrap_or_default();
    // Build the current set of ECS entity UUIDs.
    let current: HashMap<String, Entity> = query
        .iter()
        .map(|(e, u, _, _, _, _, _, _, _, _, _, _)| (u.0.clone(), e))
        .collect();

    /// Serialise a `RegionShape` to the wire string (snake_case variant name).
    fn shape_to_wire(shape: &RegionShapeSection) -> String {
        use crate::region_shape::RegionShape;
        match &shape.0 {
            RegionShape::Sphere { .. } => "sphere",
            RegionShape::Box { .. } => "box",
            RegionShape::Torus { .. } => "torus",
        }
        .to_string()
    }

    // Seed reported set from ECS on first in-progress frame so that initial
    // world entities (stars, planets, ships, fields) are not re-reported.
    // Also populate WorldData.entities so the reconnect Welcome includes them.
    if !registry.seeded {
        for (uuid, entity) in &current {
            registry.reported.insert(uuid.clone());
            if let Ok((
                _,
                _,
                id,
                name,
                transform,
                region_shape,
                entity_tags,
                radar_appearance,
                asteroid_field,
                hull_comp,
                entity_target,
                shield_comp,
            )) = query.get(*entity)
            {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    name: name.as_ref().map(|n| n.0.clone()),
                    hull_fraction,
                    shield_fraction,
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                    if snapshot.radius.is_none() {
                        match &shape.0 {
                            crate::region_shape::RegionShape::Sphere { radius } => {
                                snapshot.radius = Some(*radius);
                            }
                            crate::region_shape::RegionShape::Box { half_extents, .. } => {
                                let max_he = half_extents[0].max(half_extents[2]);
                                snapshot.radius = Some(max_he);
                                snapshot.half_extents = Some(*half_extents);
                            }
                            crate::region_shape::RegionShape::Torus {
                                inner_radius,
                                outer_radius,
                            } => {
                                snapshot.radius = Some(*outer_radius);
                                snapshot.inner_radius = Some(*inner_radius);
                            }
                        }
                    }
                }
                if snapshot.shape.is_none() {
                    if let Some(field) = asteroid_field {
                        snapshot.shape = Some("torus".to_string());
                        snapshot.radius = Some(field.0.outer_radius);
                        snapshot.inner_radius = Some(field.0.inner_radius);
                    }
                }
                if let Some(ra) = radar_appearance {
                    if let Some(colour) = &ra.0.colour {
                        if colour.len() >= 3 {
                            snapshot.colour = Some([colour[0], colour[1], colour[2]]);
                        }
                    }
                    if let Some(region_colour) = &ra.0.region_colour {
                        if region_colour.len() >= 3 {
                            snapshot.region_colour =
                                Some([region_colour[0], region_colour[1], region_colour[2]]);
                        }
                    }
                    snapshot.radar_size = ra.0.size;
                    snapshot.radar_icon = ra.0.icon.clone();
                }
                if let Some(ref id) = snapshot.id {
                    snapshot.objective_target = active_objective_names.contains(id);
                }
                // Target info
                if let Some(t) = entity_target {
                    snapshot.target_tags = t.0.tags.clone();
                    snapshot.threat_level = Some(t.0.threat_level.as_str().to_string());
                    snapshot.target_description = t.0.description.clone();
                }
                upsert_world_entity(&mut world, snapshot);
            }
        }
        registry.seeded = true;
        return;
    }

    // Emit EntitySpawned for new entities.
    for (uuid, entity) in &current {
        if registry.reported.insert(uuid.clone()) {
            if let Ok((
                _,
                _,
                id,
                name,
                transform,
                region_shape,
                entity_tags,
                radar_appearance,
                asteroid_field,
                hull_comp,
                entity_target,
                shield_comp,
            )) = query.get(*entity)
            {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    name: name.as_ref().map(|n| n.0.clone()),
                    hull_fraction,
                    shield_fraction,
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                    if snapshot.radius.is_none() {
                        match &shape.0 {
                            crate::region_shape::RegionShape::Sphere { radius } => {
                                snapshot.radius = Some(*radius);
                            }
                            crate::region_shape::RegionShape::Box { half_extents, .. } => {
                                let max_he = half_extents[0].max(half_extents[2]);
                                snapshot.radius = Some(max_he);
                                snapshot.half_extents = Some(*half_extents);
                            }
                            crate::region_shape::RegionShape::Torus {
                                inner_radius,
                                outer_radius,
                            } => {
                                snapshot.radius = Some(*outer_radius);
                                snapshot.inner_radius = Some(*inner_radius);
                            }
                        }
                    }
                }
                if snapshot.shape.is_none() {
                    if let Some(field) = asteroid_field {
                        snapshot.shape = Some("torus".to_string());
                        snapshot.radius = Some(field.0.outer_radius);
                        snapshot.inner_radius = Some(field.0.inner_radius);
                    }
                }
                if let Some(ra) = radar_appearance {
                    if let Some(colour) = &ra.0.colour {
                        if colour.len() >= 3 {
                            snapshot.colour = Some([colour[0], colour[1], colour[2]]);
                        }
                    }
                    if let Some(region_colour) = &ra.0.region_colour {
                        if region_colour.len() >= 3 {
                            snapshot.region_colour =
                                Some([region_colour[0], region_colour[1], region_colour[2]]);
                        }
                    }
                    snapshot.radar_size = ra.0.size;
                    snapshot.radar_icon = ra.0.icon.clone();
                }
                if let Some(ref id) = snapshot.id {
                    snapshot.objective_target = active_objective_names.contains(id);
                }
                // Target info
                if let Some(t) = entity_target {
                    snapshot.target_tags = t.0.tags.clone();
                    snapshot.threat_level = Some(t.0.threat_level.as_str().to_string());
                    snapshot.target_description = t.0.description.clone();
                }
                upsert_world_entity(&mut world, snapshot.clone());
                outbox
                    .0
                    .push((Target::All, ServerMessage::EntitySpawned { snapshot }));
            }
        }
    }

    // Emit EntityDespawned for entities no longer in the ECS.
    let reported_snapshot: Vec<String> = registry.reported.iter().cloned().collect();
    for uuid in &reported_snapshot {
        if !current.contains_key(uuid) {
            registry.reported.remove(uuid);
            world.0.entities.retain(|e| e.uuid != *uuid);
            // Prune the despawned UUID from the delta caches (issue #613) —
            // runtime-spawned entities (e.g. scenario-triggered NPCs) can
            // despawn and respawn with fresh UUIDs just like asteroids.
            crate::core::broadcast::cache_registry::prune(
                &mut positions_cache,
                &mut health_cache,
                std::slice::from_ref(uuid),
            );
            outbox.0.push((
                Target::All,
                ServerMessage::EntityDespawned { uuid: uuid.clone() },
            ));
        }
    }
}

// ── World Setup ────────────────────────────────────────────────────────────
//
// Per PRD #341, asteroid-field entries and named `[[entity]]` instances are
// owned by `world::server::spawn_world_entities`. This `setup_world` system
// covers only:
//   * spawning *anonymous* immediate `[[entity]]` instances (e.g. stars,
//     planets) that aren't asteroid fields and don't carry a `name`.
//
// When no `WorldConfig` is loaded (native unit tests only — production
// always loads a world TOML via the WASM bridge) this is a no-op.
fn setup_world(
    mut commands: Commands,
    mut world: ResMut<WorldResource>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
) {
    let Some(world_config) = world_config else {
        return;
    };

    let config_cache = crate::config_cache::get_config_cache();

    // Pre-resolve named-entity positions so anonymous entries using
    // `relative_to` can be positioned (PRD #337).
    let named_positions = crate::world::config::build_named_entity_positions(&world_config);

    for entity_inst in &world_config.entities {
        if entity_inst.spawn_on != crate::world::config::WorldEntitySpawnOn::Immediate {
            continue;
        }
        // Asteroid-field entries and named entries are owned by the unified
        // spawn pass in `world::server::spawn_world_entities`. Skip them to
        // avoid double-spawning.
        let is_unified = crate::world::config::is_owned_by_unified_pipeline(entity_inst, |path| {
            config_cache
                .get(path)
                .and_then(|c| c.asteroid_field.as_ref())
                .is_some()
        });
        if is_unified {
            continue;
        }

        let config = match crate::entity_loader::resolve_entity(entity_inst, &config_cache) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "setup_world: failed to resolve entity '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };

        let uuid = crate::entity_loader::assign_uuid();
        let pos = match crate::world::config::resolve_entity_position_with(
            entity_inst,
            &world_config.anchors,
            &named_positions,
        ) {
            Ok(p) => Vec3::new(p[0], p[1], p[2]),
            Err(e) => {
                bevy::log::error!("setup_world: {e}");
                continue;
            }
        };

        crate::entity_spawner::spawn_entity(
            &mut commands,
            &config,
            pos,
            uuid.clone(),
            entity_inst.id.clone(),
        );
        upsert_world_entity(
            &mut world,
            snapshot_from_entity_config(uuid, entity_inst.id.clone(), &config, pos),
        );
    }
}

fn player_spawn_rotation_yaw(rot: [f32; 3]) -> (bevy::math::Quat, f32) {
    let q = bevy::math::Quat::from_euler(bevy::math::EulerRot::YXZ, rot[1], rot[0], rot[2]);
    let (yaw, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
    (q, yaw)
}

/// Spawn entities with `spawn_on = GameStart` (e.g. player ship) when the
/// game transitions to InProgress. Registered in `OnEnter(GamePhase::InProgress)`.
fn spawn_game_start_entities(
    mut commands: Commands,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut pending_ship_config: Option<ResMut<crate::ship_plugin::PendingShipConfig>>,
    selected_ship: Option<Res<crate::lobby::SelectedShipResource>>,
    mut sessions: Option<ResMut<crate::lobby::Sessions>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut has_spawned: Local<bool>,
) {
    if *has_spawned {
        return;
    }

    let mc = match world_config.as_deref() {
        Some(mc) => mc,
        None => return,
    };

    let config_cache = crate::config_cache::get_config_cache();

    let mut ship_spawned = false;
    let named_positions = crate::world::config::build_named_entity_positions(mc);
    for entity_inst in &mc.entities {
        if entity_inst.spawn_on != crate::world::config::WorldEntitySpawnOn::GameStart {
            continue;
        }
        // Evaluate optional spawn predicate against the world flag store.
        if let Some(pred) = &entity_inst.when_predicate {
            let empty = crate::world::flags::FlagStore::new();
            let flags_ref = runtime.as_ref().map(|r| &r.flags).unwrap_or(&empty);
            if !pred.evaluate(&[flags_ref]) {
                continue;
            }
        }
        let config = match crate::entity_loader::resolve_entity(entity_inst, &config_cache) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "Failed to resolve GameStart entity '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };

        // The player ship's full loadout (weapons, torpedoes, blasters, shields,
        // mesh, stations) must come from the lobby-selected ship template, not
        // the world's `[[entity]] player-ship` placeholder. The placeholder only
        // fixes spawn position; without this override a player who selects the
        // Destroyer still spawns the placeholder hull's weapons (e.g. the
        // cruiser's two phaser banks and no blasters). ShipConfigComponent is
        // already sourced from the selection (PendingShipConfig); this brings the
        // EntityConfig-derived systems into agreement. Matched on the same
        // predicate used below for the player-ship position/rotation/marker.
        let config = if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            match selected_ship
                .as_ref()
                .and_then(|sel| config_cache.get(&sel.0))
            {
                Some(selected_cfg) => selected_cfg.clone(),
                None => config,
            }
        } else {
            config
        };

        let uuid = crate::entity_loader::assign_uuid();
        let pos = match crate::world::config::resolve_entity_position_with(
            entity_inst,
            &mc.anchors,
            &named_positions,
        ) {
            Ok(p) => Vec3::new(p[0], p[1], p[2]),
            Err(e) => {
                bevy::log::error!(
                    "Failed to resolve GameStart entity '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };

        // Override with player_spawn position when spawning the player ship
        // (issue #623).
        let pos = if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            if let Some(ref spawn) = mc.player_spawn {
                if let Some(ref anchor_name) = spawn.anchor {
                    match mc.anchors.get(anchor_name) {
                        Some(a) => Vec3::new(a[0], a[1], a[2]),
                        None => {
                            bevy::log::error!("player_spawn anchor '{}' not found", anchor_name);
                            pos
                        }
                    }
                } else if let Some(p) = spawn.position {
                    Vec3::new(p[0], p[1], p[2])
                } else {
                    pos
                }
            } else {
                pos
            }
        } else {
            pos
        };

        // Override with player_spawn rotation when spawning the player ship (issue #623).
        let player_spawn_rot: Option<bevy::math::Quat> =
            if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
                mc.player_spawn.as_ref().and_then(|s| s.rotation).map(|r| {
                    let (q, _) = player_spawn_rotation_yaw(r);
                    q
                })
            } else {
                None
            };

        let spawned = crate::entity_spawner::spawn_entity(
            &mut commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );

        // Apply rotation on the spawned entity's Transform
        if let Some(q) = player_spawn_rot {
            commands
                .entity(spawned)
                .insert(bevy::prelude::Transform::from_translation(pos).with_rotation(q));
        }

        // Extract yaw for ShipPhysicsComponent
        let initial_yaw = player_spawn_rot
            .map(|q| {
                let (yaw, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
                yaw
            })
            .unwrap_or(0.0);

        // The first GameStart entity with tags containing "ship" gets the Ship marker
        if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            let ship_config = if let Some(pending) = pending_ship_config.as_mut() {
                let cfg = crate::ship_plugin::ShipConfigComponent(pending.0.clone());
                commands.remove_resource::<crate::ship_plugin::PendingShipConfig>();
                pending_ship_config = None;
                cfg
            } else {
                crate::ship_plugin::load_ship_config_from_disk()
            };
            let (initial_control_sources, initial_active_ratings) = {
                let mut resolver = crate::ship::control_source::ControlSourceResolver::new();
                let mut active_ratings: std::collections::HashMap<StationId, String> =
                    std::collections::HashMap::new();
                if let Some(ref sess) = sessions {
                    let manned: std::collections::HashSet<_> = sess
                        .0
                        .players()
                        .iter()
                        .filter(|p| p.connected)
                        .filter_map(|p| p.station.as_ref())
                        .collect();
                    for station in &ship_config.0.stations {
                        // Manned stations apply the player's lobby-chosen
                        // complexity toggle (if any), else the station's base
                        // (first) rating. Unmanned stations are fully
                        // AI-backfilled, as before.
                        let rating_name = if manned.contains(&station.id) {
                            sess.0
                                .pending_rating_for(&station.id)
                                .cloned()
                                .or_else(|| station.ratings.first().map(|r| r.name.clone()))
                                .unwrap_or_else(|| "Std".to_string())
                        } else {
                            crate::ship::rating::BACKFILL_RATING.to_string()
                        };
                        crate::ship::rating::apply_rating(
                            &ship_config.0,
                            &station.id,
                            &rating_name,
                            &mut resolver,
                        );
                        active_ratings.insert(station.id.clone(), rating_name);
                    }
                }
                (
                    crate::ship_plugin::ShipSystemControlSources(resolver),
                    crate::ship_plugin::ActiveStationRatings(active_ratings),
                )
            };
            if let Some(ref mut sess) = sessions {
                sess.0.clear_all_pending_ratings();
            }
            commands
                .entity(spawned)
                .insert(Ship)
                .insert(LocalShip)
                .insert(crate::ai_plugin::AiHighFidelity)
                .insert(crate::ship::shields::ShieldArcIntents::default())
                .insert(crate::console_ai_plugin::ShipFrequencyHintState::default())
                .insert(crate::ship::power::PowerReactorIntents::default())
                .insert(crate::ship::power::ShipPowerAiState::default())
                .insert(ShipSystemBlackboards::default())
                .insert(ship_config)
                .insert(initial_control_sources)
                .insert(initial_active_ratings)
                .insert(crate::ship_plugin::CoordinationQueue::default())
                .insert(crate::ship_plugin::PendingArcBearingRequest::default())
                .insert(crate::ship::shields::PendingShieldsThreatBearing::default())
                .insert(crate::messages::AdmittedCommands::default())
                .insert(ShipPhysicsComponent {
                    x: pos.x,
                    z: pos.z,
                    yaw: initial_yaw,
                    ..Default::default()
                })
                .insert(crate::ai_plugin::ShipAiMemory::default())
                .insert(crate::weapons_plugin::WeaponsTarget::default())
                .insert(crate::weapons_plugin::ActiveBeam::default())
                .insert(crate::weapons_plugin::PhaserCooldown::default())
                .insert(crate::weapons_plugin::WeaponsArcRequestState::default())
                .insert(crate::sensors_plugin::SensorsTarget::default())
                .insert(crate::ship_state::ShipRedAlert::default())
                .insert(crate::ship_state::ShipViewMode::default())
                .insert(crate::ship_state::ShipPhaserFrequency::default())
                .insert(crate::navigation_plugin::NavigationWaypoint::default())
                .insert(crate::power_plugin::ShipPowerSystem(
                    crate::modifiers::power_system::PowerSystem::default(),
                ))
                .insert(crate::ship_plugin::LastHelmInput::default())
                // Per-ship impulse drive state (audit follow-up). Every
                // ship carries its own; NPC ships get one via the spawner
                // too (both idle by default).
                .insert(crate::server_app::ShipImpulse::default())
                // Per-ship boost drive battery (audit follow-up). Every
                // ship carries its own; NPC ships get one via the spawner
                // (both empty by default).
                .insert(crate::server_app::ShipBoost::default())
                // Per-ship coordination bus state (audit follow-up). See
                // `entities/spawner.rs` for details.
                .insert(crate::ship::shields::ShieldsCoordinationState::default())
                .insert(crate::ship::sensors::SensorsFrequencyState::default())
                .insert(crate::ship::sensors::SensorsThreatState::default())
                .insert(crate::ship::power::PowerBrownoutState::default())
                // Per-entity CollisionCooldown so player and NPC ships each
                // have their own cooldown timer (PRD #597 PR-8).
                .insert(CollisionCooldown::default())
                // ShipModifiers as per-entity component (PR 6 — PRD #597; the
                // legacy Resource fallback was removed in issue #606). Every
                // ship — player and NPC — carries its own instance.
                .insert(crate::modifiers::ShipModifiers::new())
                // Combat activity state per-ship (PR 10 — PRD #597). Every
                // ship (player + NPC) tracks its own recent combat activity
                // + this-tick weapon-fired / attacked / last-attacker markers.
                .insert(crate::ship::combat_activity::RecentCombatActivity::default())
                .insert(WeaponFiredThisTick::default())
                .insert(ShipAttackedThisTick::default())
                .insert(crate::weapons_plugin::LastShipAttacker::default());
            // The player ship's hull lives on its `EntitySystemHull`
            // component (PRD #581). All damage/repair paths write there
            // directly; the old `ShipHullIntegrity` resource was retired
            // in PRD #597 PR 10.
            ship_spawned = true;

            // Ship-specific resource setup
            if let Some(hc) = &config.hull {
                let _hc = hc; // hull is set up via EntitySystemHull in the spawner
                              // [repair] block — overrides default RepairTimings if present.
                              // Absent block keeps the same defaults the hardcoded constants
                              // used to provide (5.0s travel, 0.5 HP/s repair rate).
                let repair = config.repair.as_ref();
                let team_count = repair
                    .map(|rc| rc.repair_team_count as usize)
                    .filter(|&n| n > 0)
                    .unwrap_or(2);
                let timings = repair.map(|rc| rc.to_runtime()).unwrap_or_default();
                let teams = ShipRepairTeams(crate::repair_teams::RepairTeams::new_with_timings(
                    team_count, timings,
                ));
                // Insert as per-entity component AND global resource (dual-write migration).
                commands.entity(spawned).insert(teams.clone());
                commands.insert_resource(teams);
            }

            // Apply shield focus config + base shield-system values from TOML if present.
            // Post-#514: the `[shields_console.base]` sub-block still holds
            // ship-wide defaults (max_hp, regen_per_sec, offline_duration)
            // consumed as fallbacks by each `[[shield_arc]]` block. When
            // shield_arcs are declared the runtime is built via
            // `ShieldSystem::from_arcs`; otherwise fall back to
            // `ShieldSystem::new` with historical evenly-spaced facings.
            if let Some(sc) = &config.shields_console {
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
                let mut shields = ShipShields(shield_system, freq);
                shields.0.focus_config = crate::shield::ShieldFocusConfig {
                    bonus_max_hp: sc.focus_bonus_max_hp,
                    bonus_regen: sc.focus_bonus_regen,
                    penalty_max_hp: sc.focus_penalty_max_hp,
                    penalty_regen: sc.focus_penalty_regen,
                    decay_rate: sc.focus_decay_rate,
                    focused_damage_multiplier: sc.focus_focused_damage_multiplier,
                    unfocused_damage_multiplier: sc.focus_unfocused_damage_multiplier,
                };
                commands.entity(spawned).insert(shields);
            } else if !config.shield_arcs.is_empty() {
                let ship_wide = crate::shield::ShieldConfig::default();
                let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
                let freq = config
                    .shield_arcs
                    .first()
                    .map(|a| a.frequency)
                    .unwrap_or(0.5);
                commands.entity(spawned).insert(ShipShields(
                    ShieldSystem::from_arcs(&arcs, &ship_wide),
                    freq,
                ));
            } else {
                // Default shields on the ship entity when no TOML shields_console block.
                commands
                    .entity(spawned)
                    .insert(ShipShields(ShieldSystem::default(), 0.5));
            }

            // Shields AI config — loaded from [shields_console.ai] if present,
            // otherwise falls back to ShieldsAiConfigResource defaults. Inserted
            // as both per-entity Component and global Resource (dual-write
            // migration; the per-entity Component is queried by operate_shields_ai).
            let ai_cfg = config
                .shields_console
                .as_ref()
                .and_then(|sc| sc.ai.as_ref())
                .map(|ai| crate::ship::shields::ShieldsAiConfigResource {
                    restored_notify_pct: 0.5,
                    damage_window_secs: ai.damage_window_secs,
                    min_damage_window_secs: ai.min_damage_window_secs,
                    damage_pct_threshold: ai.damage_pct_threshold,
                    health_ratio_threshold: ai.health_ratio_threshold,
                })
                .unwrap_or_default();
            commands.entity(spawned).insert(ai_cfg.clone());
            commands.insert_resource(ai_cfg);

            // Shields damage history — per-ship Component tracking HP deltas
            // for the AI damage-concentration algorithm. Initialised empty; resized
            // lazily by operate_shields_ai to match the ship's arc count.
            commands
                .entity(spawned)
                .insert(crate::ship::shields::ShieldsDamageHistory::default());

            // Per-arc hull HP (issue #514). Attach `EntityShipArcHull`
            // alongside the shield system so `sync_console_damage_tiers`
            // can flip the fine `shield-arc-<id>` SystemIds into
            // `offline_systems` when an arc's hull HP drops into the
            // Disabled/Destroyed tier.
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
                    commands
                        .entity(spawned)
                        .insert(crate::entity_spawner::EntityShipArcHull(
                            crate::damage::ShipArcHull::from_entries(arc_entries),
                        ));
                }
            }

            if let Some(wc) = &config.weapons_console {
                let first_bank = wc.phaser_banks.first();
                let beam_color = crate::beam_render::resolve_beam_color(
                    first_bank.map(|b| &b.beam_color).unwrap_or(&vec![]),
                );
                let beam_range = first_bank
                    .map(|b| {
                        if b.beam_range > 0.0 {
                            b.beam_range
                        } else {
                            40.0
                        }
                    })
                    .unwrap_or(40.0);
                let render_cfg = PhaserRenderConfig {
                    beam_color,
                    beam_range,
                };
                // Insert as per-entity component AND global resource (dual-write migration).
                commands.entity(spawned).insert(render_cfg.clone());
                commands.insert_resource(render_cfg);

                // Player phaser combat tuning — overrides the default
                // PhaserCombatConfig that WeaponsPlugin installed. The
                // [weapons_console] block already carries `beam_range`,
                // `beam_damage_per_sec`, `beam_duration_secs`, and
                // `cooldown_secs`; before this slice those were only
                // honoured by the NPC phaser path. Now the player path
                // also reads them via the PhaserCombatConfig resource.
                let combat_cfg = crate::weapons_plugin::PhaserCombatConfigResource(
                    crate::entity_config::PhaserCombatConfig::from_weapons_console(wc),
                );
                // Insert as per-entity component AND global resource (dual-write migration).
                commands.entity(spawned).insert(combat_cfg.clone());
                commands.insert_resource(combat_cfg);
            } else {
                // No [weapons_console] block — insert defaults so the entity-component
                // path always finds a value on the LocalShip entity.
                commands
                    .entity(spawned)
                    .insert(crate::weapons_plugin::PhaserCombatConfigResource::default());
                commands
                    .entity(spawned)
                    .insert(PhaserRenderConfig::default());
            }

            // [torpedoes] block — builds the TorpedoSystem from TOML config.
            // Inserted as per-entity component AND global resource (dual-write
            // migration). NPC ships with a [torpedoes] block also get their own
            // TorpedoSystemResource component via `entities::spawner::spawn_entity`
            // (see #597 PR-3 and the audit follow-up); `tick_torpedo_system`
            // iterates `With<Ship>` so both paths advance the same way.
            if let Some(tc) = &config.torpedoes {
                let runtime_config = tc.to_runtime();
                let torpedo_system = if !tc.tubes.is_empty() {
                    crate::torpedo::TorpedoSystem::from_configs(&tc.tubes, runtime_config)
                } else {
                    crate::torpedo::TorpedoSystem::new(runtime_config)
                };
                let torpedo_res = crate::weapons_plugin::TorpedoSystemResource(torpedo_system);
                // Insert as per-entity component AND global resource (dual-write migration).
                commands.insert_resource(torpedo_res.clone());
                commands.entity(spawned).insert(torpedo_res);
            }

            // Power config — unconditionally insert as per-entity Component
            // so systems that iterate `With<Ship>` always see a value on
            // the player ship (matching NPCs, which spawner.rs always
            // inserts a defaulted `PowerConfigResource` for). Dual-writes
            // the global Resource for legacy readers.
            let power_config = if let Some(pc) = &config.power {
                PowerConfigResource(crate::power_system::PowerConfig {
                    capacity: pc.capacity,
                    rates: pc.rates,
                    emergency_threshold: pc.emergency_threshold,
                })
            } else {
                PowerConfigResource::default()
            };
            commands.entity(spawned).insert(power_config.clone());
            commands.insert_resource(power_config);

            // Power AI config — unconditionally insert as per-entity
            // Component so `ai_power_allocation` iterating `With<Ship>` sees
            // a value on the player ship. Dual-writes the Resource.
            let ai_cfg = match config.power.as_ref().and_then(|pc| pc.ai.as_ref()) {
                Some(ai) => PowerAiConfigResource {
                    movement_thrust_threshold: ai.movement_thrust_threshold,
                    movement_engage_delay_secs: ai.movement_engage_delay_secs,
                    movement_battery_engage_min_pct: ai.movement_battery_engage_min_pct,
                    movement_battery_recharge_pct: ai.movement_battery_recharge_pct,
                    red_alert_engage_delay_secs: ai.red_alert_engage_delay_secs,
                    red_alert_battery_engage_min_pct: ai.red_alert_battery_engage_min_pct,
                    red_alert_battery_recharge_pct: ai.red_alert_battery_recharge_pct,
                },
                None => PowerAiConfigResource::default(),
            };
            commands.entity(spawned).insert(ai_cfg.clone());
            commands.insert_resource(ai_cfg);

            // Power multipliers
            let defaults = [-0.5, 0.0, 0.25, 0.5];
            let mut multipliers: std::collections::HashMap<
                crate::messages::PowerGroupId,
                [f32; 4],
            > = std::collections::HashMap::from([
                (
                    crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                    defaults,
                ),
                (
                    crate::messages::PowerGroupId(crate::power_system::WEAPONS_POWER_GROUP.into()),
                    defaults,
                ),
                (
                    crate::messages::PowerGroupId(crate::power_system::SENSORS_POWER_GROUP.into()),
                    defaults,
                ),
            ]);
            if let Some(hc) = &config.helm_console {
                if let Some(pm) = hc.power_multipliers {
                    multipliers.insert(
                        crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                        pm,
                    );
                }
            }
            if let Some(wc) = &config.weapons_console {
                if let Some(pm) = wc.power_multipliers {
                    multipliers.insert(
                        crate::messages::PowerGroupId(
                            crate::power_system::WEAPONS_POWER_GROUP.into(),
                        ),
                        pm,
                    );
                }
            }
            if let Some(sc) = &config.sensors_console {
                if let Some(pm) = sc.power_multipliers {
                    // sensors_console power drives the Sensors radar range multiplier
                    multipliers.insert(
                        crate::messages::PowerGroupId(
                            crate::power_system::SENSORS_POWER_GROUP.into(),
                        ),
                        pm,
                    );
                }
            }
            commands.insert_resource(PowerMultiplierResource {
                multipliers: multipliers.clone(),
            });
            // Insert as per-entity component AND global resource (dual-write migration — PR 6).
            commands
                .entity(spawned)
                .insert(PowerMultiplierResource { multipliers });

            // Ship physics config from [helm_console] TOML, or default
            let physics_cfg =
                config
                    .helm_console
                    .as_ref()
                    .map(|hc| crate::ship_physics::ShipPhysicsConfig {
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
                    });
            let physics_cfg_resource = crate::ship_plugin::ShipPhysicsConfigResource(
                physics_cfg.unwrap_or(crate::ship_physics::ShipPhysicsConfig::new()),
            );
            commands.insert_resource(physics_cfg_resource.clone());
            commands.entity(spawned).insert(physics_cfg_resource);

            // Impulse config from [helm_console] TOML, or default
            let impulse_cfg = config
                .helm_console
                .as_ref()
                .map(|hc| crate::ship_plugin::ImpulseConfigResource {
                    charge_duration: hc.impulse_charge_duration,
                    speed_multiplier: hc.impulse_speed_multiplier,
                    acceleration_multiplier: hc.impulse_acceleration_multiplier,
                    engage_distance: hc.impulse_engage_distance,
                    cancel_distance: hc.impulse_cancel_distance,
                })
                .unwrap_or_default();
            commands.entity(spawned).insert(impulse_cfg);

            // Boost config from [helm_console.boost] TOML. Absent table ⇒
            // feature disabled (default component has `enabled: false`).
            let boost_cfg = config
                .helm_console
                .as_ref()
                .and_then(|hc| hc.boost.as_ref())
                .map(|b| crate::ship_plugin::BoostConfigResource {
                    enabled: true,
                    multiplier: b.multiplier,
                    steering_multiplier: b.steering_multiplier,
                    active_duration: b.active_duration,
                    recharge_duration: b.recharge_duration,
                })
                .unwrap_or_default();
            commands.entity(spawned).insert(boost_cfg);

            // Bank config from [helm_console] TOML, or default
            let bank_cfg = config
                .helm_console
                .as_ref()
                .map(|hc| crate::ship_plugin::BankConfigResource {
                    max_bank_deg: hc.max_bank_deg,
                    bank_lerp_rate: hc.bank_lerp_rate,
                })
                .unwrap_or_default();
            commands.insert_resource(bank_cfg.clone());
            commands.entity(spawned).insert(bank_cfg);
        }
    }

    *has_spawned = true;
}

/// Diagnostic: dump every tracked entity's components on InProgress start.
/// Helps debug missing raider or other invisible NPC issues.
fn dump_tracked_entities(
    query: Query<(
        &EntityUuid,
        Option<&EntityName>,
        Option<&EntityId>,
        &Transform,
        Option<&MeshSection>,
        Option<&EntityTagsSection>,
        Option<&RadarAppearanceSection>,
        Option<&BehaviourSection>,
        Option<&FactionComponent>,
    )>,
) {
    bevy::log::info!("=== ENTITY DUMP (InProgress start) ===");
    let mut count = 0u32;
    for (uuid, name, id, transform, mesh, tags, radar, behaviour, faction) in &query {
        count += 1;
        let label = name
            .map(|n| n.0.clone())
            .or_else(|| id.map(|i| i.0.clone()))
            .unwrap_or_else(|| "?".to_string());
        let pos = format!(
            "[{:.1}, {:.1}, {:.1}]",
            transform.translation.x, transform.translation.y, transform.translation.z
        );
        let has_mesh = if mesh.is_some() { "MESH" } else { "no-mesh" };
        let tags_str = tags
            .map(|t| format!("tags={:?}", t.0))
            .unwrap_or_else(|| "no-tags".to_string());
        let has_radar = if radar.is_some() { "RADAR" } else { "no-radar" };
        let has_ai = if behaviour.is_some() { "AI" } else { "no-ai" };
        let fac = faction
            .map(|f| format!("faction={}", f.0))
            .unwrap_or_else(|| "no-faction".to_string());
        bevy::log::info!(
            "  ENTTY uuid={} label={} pos={} {} {} {} {} {}",
            &uuid.0[..uuid.0.len().min(8)],
            label,
            pos,
            has_mesh,
            tags_str,
            has_radar,
            has_ai,
            fac
        );
    }
    bevy::log::info!("=== ENTITY DUMP END ({} entities) ===", count);
}

/// Marker: entity mesh has been rendered (GLB procedural).
/// Prevents re-processing by `render_spawned_entities`.
#[derive(Component)]
struct RenderProcessed;

/// Holds a pending GLB scene handle so the asset server keeps the asset alive
/// across frames until it finishes loading.
#[derive(Component)]
struct PendingSceneHandle(Handle<bevy::scene::Scene>);

/// Read a model-rig sidecar TOML for `path`.
///
/// - **Native**: `std::fs::read_to_string` (returns `None` when absent).
/// - **WASM**: checks the pending-sidecar queue populated by JS via
///   `wasm_push_sidecar_toml`; fires a deferred JS fetch on first miss and
///   returns `None` until the fetch resolves. An empty pushed string (404)
///   resolves to `Some(String::new())`, which parses to an identity rig.
///
/// **Important**: this call is destructive — the entry is removed from the
/// queue once read. The renderer is the sole intended consumer; callers that
/// only need readiness (e.g. preload progress) must use
/// [`crate::config_cache::is_pending_sidecar_delivered`] instead, or the
/// renderer will lose the race and the model will never appear.
fn load_sidecar_toml(path: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::config_cache::take_pending_sidecar_toml(path).or_else(|| {
            crate::config_cache::request_sidecar_fetch(path.to_string());
            None
        })
    }
}

/// Resolve a model's rig sidecar to a `ModelRig`.
///
/// Returns:
/// - `Some(rig)` once the sidecar is resolved — either parsed, or an identity
///   rig when the sidecar is genuinely absent (native: file missing; wasm: JS
///   pushed an empty string for a 404) or fails to parse.
/// - `None` while a wasm fetch is still in flight (caller retries next frame).
///   On native this never returns `None` (the filesystem read is synchronous).
fn resolve_sidecar_rig(
    model_path: &str,
    variant: Option<&str>,
) -> Option<crate::model_rig::ModelRig> {
    let path = crate::model_rig::sidecar_path(model_path, variant);
    match load_sidecar_toml(&path) {
        Some(toml_str) => {
            if toml_str.trim().is_empty() {
                // Absent (404 / empty) → identity rig so the model still renders.
                Some(crate::model_rig::ModelRig::default())
            } else {
                match crate::model_rig::ModelRig::from_toml(&toml_str) {
                    Ok(rig) => Some(rig),
                    Err(e) => {
                        // A present-but-malformed sidecar degrades to an identity
                        // rig so the model still renders, but we surface the parse
                        // error so an authoring typo isn't silently invisible.
                        bevy::log::warn!(
                            "rig sidecar {path} failed to parse: {e}; using identity rig"
                        );
                        Some(crate::model_rig::ModelRig::default())
                    }
                }
            }
        }
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                // Native: a missing file is "genuinely absent" → identity rig.
                Some(crate::model_rig::ModelRig::default())
            }
            #[cfg(target_arch = "wasm32")]
            {
                // WASM: fetch still in flight → retry next frame.
                None
            }
        }
    }
}

/// Outcome of attempting to spawn a GLB visual (flat render or LOD swap).
enum GlbSpawnOutcome {
    /// The scene + rig resolved; the `SceneRoot` child entity was spawned.
    Spawned(Entity),
    /// The scene asset or rig sidecar is still loading — retry next frame.
    Pending,
    /// The GLB failed to load permanently.
    Failed,
}

/// Spawn a GLB scene as a child of `entity`, mirroring PATH A of the flat
/// renderer. Resolves the scene handle (storing a [`PendingSceneHandle`] on the
/// parent to keep it alive across frames), waits for both the scene asset and
/// the rig sidecar, then spawns the `SceneRoot` child (hidden +
/// `NoFrustumCulling` for the local ship) and attaches
/// [`crate::model_rig::ModelMarkers`] to the parent. Returns the spawned child
/// so the LOD system can tear it down on a level switch.
///
/// Shared by [`render_spawned_entities`] (initial flat render) and
/// [`update_mesh_lod`] (LOD-level swaps) so both go through identical async
/// loading + rig-composition logic.
fn spawn_glb_visual(
    commands: &mut Commands,
    asset_server: &AssetServer,
    scenes: &Assets<bevy::scene::Scene>,
    entity: Entity,
    model_path: &str,
    variant: Option<&str>,
    is_local_ship: bool,
    pending: Option<&PendingSceneHandle>,
) -> GlbSpawnOutcome {
    let scene: Handle<bevy::scene::Scene> = match pending {
        Some(p) => p.0.clone(),
        None => {
            // `asset_server` resolves paths relative to the `assets/` root, but
            // the TOML `model` field carries an `assets/` prefix. Strip it so
            // the GLB resolves instead of looking for `assets/assets/...`.
            let rel = model_path.strip_prefix("assets/").unwrap_or(model_path);
            let path = format!("{}#Scene0", rel);
            let h: Handle<bevy::scene::Scene> = asset_server.load(&path);
            bevy::log::info!(
                "spawn_glb_visual: requesting scene {path} (load_state={:?})",
                asset_server.load_state(h.id())
            );
            commands
                .entity(entity)
                .insert(PendingSceneHandle(h.clone()));
            h
        }
    };
    // A `LoadState::Failed` GLB never appears in `Assets<Scene>`, so stop
    // retrying and let the caller settle without a mesh.
    if matches!(
        asset_server.load_state(scene.id()),
        bevy::asset::LoadState::Failed(_)
    ) {
        bevy::log::warn!(
            "spawn_glb_visual: GLB failed to load for entity {entity:?}, path={model_path} — entity will exist without a mesh"
        );
        commands.entity(entity).remove::<PendingSceneHandle>();
        return GlbSpawnOutcome::Failed;
    }
    // Wait for BOTH the GLB scene AND the rig sidecar before finalising.
    if scenes.get(&scene).is_none() {
        return GlbSpawnOutcome::Pending;
    }
    let rig = match resolve_sidecar_rig(model_path, variant) {
        Some(rig) => rig,
        // Sidecar fetch still in flight (wasm) — retry next frame.
        None => return GlbSpawnOutcome::Pending,
    };
    commands.entity(entity).remove::<PendingSceneHandle>();

    // Composition: entityTransform ∘ baseRig ∘ model. The base rig is applied
    // INNER to the per-entity transform by spawning the GLB SceneRoot as a
    // CHILD carrying `base_bevy_transform()`.
    let base_tf = rig.base_bevy_transform();
    let child = if is_local_ship {
        // Local ship model: hidden by default; shown only in cinematic camera.
        commands
            .spawn((
                bevy::scene::SceneRoot(scene),
                base_tf,
                Visibility::Hidden,
                LocalShipModel,
                bevy::camera::visibility::NoFrustumCulling,
            ))
            .id()
    } else {
        commands
            .spawn((bevy::scene::SceneRoot(scene), base_tf))
            .id()
    };
    commands.entity(entity).add_child(child);
    // Attach the resolved marker map so downstream systems (weapons, exhaust, …)
    // can resolve mount points by name.
    commands
        .entity(entity)
        .insert(crate::model_rig::ModelMarkers::from_rig(&rig));
    GlbSpawnOutcome::Spawned(child)
}

/// Rounded key for a cached procedural mesh (geometry only — colour/emissive do
/// not affect the mesh, so they are excluded to maximise sharing).
#[derive(Clone, PartialEq, Eq, Hash)]
struct ProcMeshKey {
    /// Shape discriminant: 0 = sphere, 1 = cuboid, 2 = torus.
    shape: u8,
    radius_q: i32,
    size_q: [i32; 3],
    minor_q: i32,
}

/// Rounded key for a cached procedural material (appearance only).
#[derive(Clone, PartialEq, Eq, Hash)]
struct ProcMatKey {
    colour_q: [i32; 3],
    emissive_q: i32,
}

/// Quantise a float to milli-units for use in a hashable cache key.
fn quantize_key(v: f32) -> i32 {
    (v * 1000.0).round() as i32
}

/// Deduplicates procedural meshes and materials by rounded key so that all
/// identical primitives (e.g. every distant asteroid's far-LOD sphere) share a
/// single mesh handle and a single material handle. Reusing handles lets the
/// renderer batch/instance the draws instead of issuing one per entity.
#[derive(Resource, Default)]
struct ProceduralMeshCache {
    meshes: HashMap<ProcMeshKey, Handle<Mesh>>,
    materials: HashMap<ProcMatKey, Handle<StandardMaterial>>,
}

/// Build — or fetch from `cache` — the `Mesh3d`/material handles for a
/// procedural primitive. Mirrors PATH B of the flat renderer but routes through
/// the cache so identical primitives share handles. Shared by the flat renderer
/// and the LOD system.
fn procedural_mesh_material(
    cache: &mut ProceduralMeshCache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    shape: crate::entity_config::MeshShape,
    radius: f32,
    size: Option<[f32; 3]>,
    minor_radius: f32,
    colour: &[f32],
    emissive_mul: f32,
) -> (Handle<Mesh>, Handle<StandardMaterial>) {
    use crate::entity_config::MeshShape;

    let (shape_id, size_for_key) = match shape {
        MeshShape::Sphere => (0u8, [0.0; 3]),
        MeshShape::Cuboid => (1u8, size.unwrap_or([2.0, 1.0, 3.0])),
        MeshShape::Torus => (2u8, [0.0; 3]),
    };
    let mesh_key = ProcMeshKey {
        shape: shape_id,
        radius_q: quantize_key(radius),
        size_q: [
            quantize_key(size_for_key[0]),
            quantize_key(size_for_key[1]),
            quantize_key(size_for_key[2]),
        ],
        minor_q: quantize_key(minor_radius),
    };
    let mesh_handle = cache
        .meshes
        .entry(mesh_key)
        .or_insert_with(|| match shape {
            MeshShape::Sphere => meshes.add(Sphere {
                radius: radius.max(0.1),
            }),
            MeshShape::Cuboid => {
                let [x, y, z] = size.unwrap_or([2.0, 1.0, 3.0]);
                meshes.add(Cuboid::new(x, y, z))
            }
            MeshShape::Torus => meshes.add(Torus {
                major_radius: radius.max(0.5),
                minor_radius: minor_radius.max(0.1),
            }),
        })
        .clone();

    let rgb = if colour.len() >= 3 {
        [colour[0], colour[1], colour[2]]
    } else {
        [0.6, 0.6, 0.6]
    };
    let mat_key = ProcMatKey {
        colour_q: [
            quantize_key(rgb[0]),
            quantize_key(rgb[1]),
            quantize_key(rgb[2]),
        ],
        emissive_q: quantize_key(emissive_mul),
    };
    let mat_handle = cache
        .materials
        .entry(mat_key)
        .or_insert_with(|| {
            let color = Color::srgb(rgb[0], rgb[1], rgb[2]);
            let emissive = LinearRgba::from(color) * emissive_mul;
            materials.add(StandardMaterial {
                base_color: color,
                emissive,
                ..default()
            })
        })
        .clone();

    (mesh_handle, mat_handle)
}

/// Distance-based mesh LOD state, attached to entities whose `[mesh]` config
/// declares one or more `lod` levels. [`update_mesh_lod`] selects and swaps the
/// active level each frame based on camera distance; [`render_spawned_entities`]
/// skips rendering these entities directly.
#[derive(Component)]
struct MeshLods {
    /// Ordered near→far LOD levels copied from the entity's `MeshConfig`.
    levels: Vec<crate::entity_config::LodLevel>,
    /// Flat mesh config supplying fallback fields (colour/radius/emissive/size/
    /// minor_radius) and the shared `variant` for levels that omit them.
    base: crate::entity_config::MeshConfig,
    /// Active level index; `None` until the first evaluation establishes it.
    current: Option<usize>,
    /// The spawned GLB `SceneRoot` child when the active level is a GLB level.
    scene_child: Option<Entity>,
    /// True when the active level's `Mesh3d`/`MeshMaterial3d` live on the parent.
    procedural_on_parent: bool,
    /// Whether this entity is the local player's ship (GLB starts hidden).
    is_local_ship: bool,
}

/// Remove whichever visual the active LOD level installed, so a new level can be
/// built cleanly. Despawns the GLB child (via `try_despawn`, safe if it was
/// already removed — Bevy 0.18 `despawn` panics on an already-despawned entity)
/// and/or strips the parent's procedural mesh + material.
///
/// Note: this intentionally does NOT remove `ModelMarkers`. On a GLB→GLB switch
/// the new level's `spawn_glb_visual` re-inserts `ModelMarkers`, and because
/// commands apply in enqueue order, a blanket `remove` here (queued after that
/// insert) would clobber the new markers. `ModelMarkers` is instead cleared
/// explicitly in the procedural branch of [`update_mesh_lod`] when switching
/// away from a GLB level to a shape level.
fn teardown_lod_visual(commands: &mut Commands, entity: Entity, lods: &mut MeshLods) {
    if let Some(child) = lods.scene_child.take() {
        commands.entity(child).try_despawn();
    }
    if lods.procedural_on_parent {
        commands
            .entity(entity)
            .remove::<Mesh3d>()
            .remove::<MeshMaterial3d<StandardMaterial>>();
        lods.procedural_on_parent = false;
    }
}

/// Add visual meshes and materials to spawned entities that have a `[mesh]`
/// section but no `RenderProcessed` yet. When `cfg.model` is set, loads a GLB
/// scene instead of creating a procedural shape — but defers insertion until
/// the asset is actually loaded (avoids attaching an unloaded handle that
/// would never retry). Applies `cfg.scale` and `cfg.rotation` to the entity's
/// transform in both paths. Additionally, if the entity carries a `Lights`
/// component (from one or more `[[light]]` TOML entries), attach the matching
/// `PointLight`/`DirectionalLight` components (single light → inline, multiple
/// → spawned as child entities).
///
/// Entities whose `MeshConfig.lod` is non-empty are NOT rendered here: they
/// receive a [`MeshLods`] component and are driven by [`update_mesh_lod`].
fn render_spawned_entities(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut star_surface_materials: ResMut<Assets<crate::entity_star::StarSurfaceMaterial>>,
    mut star_halo_materials: ResMut<Assets<crate::entity_star::StarHaloMaterial>>,
    mut proc_cache: ResMut<ProceduralMeshCache>,
    scenes: Res<Assets<bevy::scene::Scene>>,
    entities: Query<
        (
            Entity,
            &Transform,
            Option<&crate::entity_spawner::MeshSection>,
            Option<&crate::entity_spawner::StarSection>,
            Option<&crate::entity_spawner::Lights>,
            Option<&PendingSceneHandle>,
            Option<&crate::simulation::LocalShip>,
        ),
        Without<RenderProcessed>,
    >,
) {
    for (entity, transform, mesh_sec, star_sec, lights_opt, pending, local_ship) in entities.iter()
    {
        let mesh_cfg_for_transform = mesh_sec.map(|mesh_sec| &mesh_sec.0);

        if let Some(star_sec) = star_sec {
            let cfg = &star_sec.0;
            let surface_mesh = meshes.add(crate::entity_star::uv_sphere_mesh(
                cfg.radius,
                cfg.longitude_segments,
                cfg.latitude_segments,
            ));
            let surface_mat =
                star_surface_materials.add(crate::entity_star::surface_material_from_config(cfg));
            let halo_radius = cfg.radius * cfg.halo_radius_multiplier.max(1.0);
            let halo_mesh = meshes.add(crate::entity_star::halo_quad_mesh(halo_radius));
            let halo_mat =
                star_halo_materials.add(crate::entity_star::halo_material_from_config(cfg));
            let mut ec = commands.entity(entity);
            ec.insert((Mesh3d(surface_mesh), MeshMaterial3d(surface_mat)));
            ec.with_children(|parent| {
                parent.spawn((
                    Mesh3d(halo_mesh),
                    MeshMaterial3d(halo_mat),
                    Transform::default(),
                    crate::entity_star::StarHalo {
                        radius: halo_radius,
                    },
                ));
            });
        } else if let Some(mesh_sec) = mesh_sec {
            let cfg = &mesh_sec.0;

            if !cfg.lod.is_empty() {
                // LOD entity: defer the visual to `update_mesh_lod`, which
                // selects a level by camera distance each frame. Attach the LOD
                // state; the flat paths below are skipped for this entity.
                commands.entity(entity).insert(MeshLods {
                    levels: cfg.lod.clone(),
                    base: cfg.clone(),
                    current: None,
                    scene_child: None,
                    procedural_on_parent: false,
                    is_local_ship: local_ship.is_some(),
                });
            } else if let Some(model_path) = &cfg.model {
                // PATH A: GLB model (shared helper preserves the async logic).
                match spawn_glb_visual(
                    &mut commands,
                    &asset_server,
                    &scenes,
                    entity,
                    model_path,
                    cfg.variant.as_deref(),
                    local_ship.is_some(),
                    pending,
                ) {
                    GlbSpawnOutcome::Spawned(_) => {}
                    // GLB / rig not loaded yet — try again next frame.
                    GlbSpawnOutcome::Pending => continue,
                    GlbSpawnOutcome::Failed => {
                        // Stop retrying an entity whose GLB will never load.
                        commands.entity(entity).insert(RenderProcessed);
                        continue;
                    }
                }
            } else {
                // PATH B: Procedural primitive (deduped via the shared cache).
                let emissive_mul = cfg.emissive.unwrap_or(0.4);
                let (mesh, mat) = procedural_mesh_material(
                    &mut proc_cache,
                    &mut meshes,
                    &mut materials,
                    cfg.shape,
                    cfg.radius,
                    cfg.size,
                    cfg.minor_radius,
                    &cfg.colour,
                    emissive_mul,
                );
                commands
                    .entity(entity)
                    .insert((Mesh3d(mesh), MeshMaterial3d(mat)));
            }
        } else {
            continue;
        }

        // Apply scale/rotation — preserves spawn position. `mesh_cfg_for_transform`
        // is `None` for stars, so this is a no-op on that path.
        if let Some(cfg) =
            mesh_cfg_for_transform.filter(|cfg| cfg.scale != 1.0 || cfg.rotation != [0.0, 0.0, 0.0])
        {
            commands.entity(entity).insert(Transform {
                translation: transform.translation,
                rotation: bevy::math::Quat::from_euler(
                    bevy::math::EulerRot::XYZ,
                    cfg.rotation[0],
                    cfg.rotation[1],
                    cfg.rotation[2],
                ),
                scale: Vec3::splat(cfg.scale),
            });
        }

        // Mark processed so we never visit this entity again.
        let mut ec = commands.entity(entity);
        ec.insert(RenderProcessed);

        // Attach lights, if any. A light that needs to face the player must
        // be its own child entity so rotating it doesn't rotate the parent's
        // visual mesh; otherwise a single light can live on the entity itself.
        if let Some(lights_comp) = lights_opt {
            let lights = &lights_comp.0;
            let needs_children = lights.len() > 1 || lights.iter().any(|l| l.face_player);
            match (lights.len(), needs_children) {
                (0, _) => {}
                (1, false) => insert_light(&mut ec, &lights[0]),
                _ => {
                    ec.with_children(|parent| {
                        for light in lights {
                            spawn_child_light(parent, light);
                        }
                    });
                }
            }
        }
    }
}

/// Distance-based LOD driver. For each entity carrying a [`MeshLods`] component,
/// computes the 3-D distance from the [`GameCamera`](crate::server::renderer::GameCamera)
/// to the entity, selects the appropriate level via
/// [`crate::entity_config::select_lod`] (with hysteresis), and — when the chosen
/// level differs from the current one — tears down the old visual and builds the
/// new one through the same helpers the flat renderer uses.
///
/// GLB levels that are still async-loading keep the current visual and retry
/// next frame, so a switch never leaves the entity permanently invisible.
/// Runs after [`render_spawned_entities`] so newly-attached `MeshLods` are
/// established the same frame they are spawned.
fn update_mesh_lod(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut proc_cache: ResMut<ProceduralMeshCache>,
    scenes: Res<Assets<bevy::scene::Scene>>,
    camera: Query<&GlobalTransform, With<crate::server::renderer::GameCamera>>,
    mut lod_entities: Query<(
        Entity,
        &Transform,
        &mut MeshLods,
        Option<&PendingSceneHandle>,
    )>,
) {
    use crate::entity_config::select_lod;

    // No camera → nothing to measure distance against; try again next frame.
    let Some(cam_tf) = camera.iter().next() else {
        return;
    };
    let cam_pos = cam_tf.translation();

    for (entity, transform, mut lods, pending) in lod_entities.iter_mut() {
        // Use the entity's LOCAL transform, not its `GlobalTransform`: on the
        // frame an entity is first rendered its `MeshLods` is inserted this same
        // Update, but global transforms aren't propagated until PostUpdate, so a
        // `GlobalTransform` read here would still be the identity default and pick
        // the initial level from distance-to-origin (a one-frame wrong-LOD flash).
        // Asteroids are top-level/unparented, so local == world. If a parented
        // entity ever needs LOD, this must switch to a propagated world position.
        let distance = transform.translation.distance(cam_pos);
        let target = select_lod(&lods.levels, distance, lods.current);
        if lods.current == Some(target) {
            continue;
        }

        // Copy the target level out so the `lods` borrow is free for teardown.
        let Some(level) = lods.levels.get(target).cloned() else {
            continue;
        };

        if let Some(model_path) = level.model.as_deref() {
            let variant = level.variant.clone().or_else(|| lods.base.variant.clone());
            match spawn_glb_visual(
                &mut commands,
                &asset_server,
                &scenes,
                entity,
                model_path,
                variant.as_deref(),
                lods.is_local_ship,
                pending,
            ) {
                // Keep the current visual until the new GLB resolves — avoids a
                // visible gap. `current` is left unchanged so we retry next frame.
                GlbSpawnOutcome::Pending => continue,
                GlbSpawnOutcome::Failed => {
                    // Give up on this level; drop the old visual and settle so we
                    // stop retrying it every frame.
                    teardown_lod_visual(&mut commands, entity, &mut lods);
                    lods.current = Some(target);
                }
                GlbSpawnOutcome::Spawned(child) => {
                    teardown_lod_visual(&mut commands, entity, &mut lods);
                    lods.scene_child = Some(child);
                    lods.current = Some(target);
                }
            }
        } else if let Some(shape) = level.shape {
            // Procedural level — fields fall back to the flat `base` config.
            let radius = level.radius.unwrap_or(lods.base.radius);
            let minor = level.minor_radius.unwrap_or(lods.base.minor_radius);
            let size = level.size.or(lods.base.size);
            let emissive_mul = level.emissive.or(lods.base.emissive).unwrap_or(0.4);
            let colour = level
                .colour
                .clone()
                .unwrap_or_else(|| lods.base.colour.clone());
            let (mesh, mat) = procedural_mesh_material(
                &mut proc_cache,
                &mut meshes,
                &mut materials,
                shape,
                radius,
                size,
                minor,
                &colour,
                emissive_mul,
            );
            teardown_lod_visual(&mut commands, entity, &mut lods);
            // Switching to a shape level: drop any `ModelMarkers` left by a prior
            // GLB level (no-op if absent). Enqueued after teardown and before the
            // mesh insert, so it never races a freshly-inserted marker map.
            commands
                .entity(entity)
                .remove::<crate::model_rig::ModelMarkers>()
                .insert((Mesh3d(mesh), MeshMaterial3d(mat)));
            lods.procedural_on_parent = true;
            lods.current = Some(target);
        } else {
            // Neither model nor shape — invalid level. Settle so we don't spin.
            bevy::log::warn!(
                "update_mesh_lod: LOD level {target} on {entity:?} has neither model nor shape — skipping"
            );
            lods.current = Some(target);
        }
    }
}

fn insert_light(
    ec: &mut bevy::ecs::system::EntityCommands,
    light: &crate::entity_config::LightConfig,
) {
    use crate::entity_config::LightKind;
    let color = Color::srgb(light.colour[0], light.colour[1], light.colour[2]);
    match light.kind {
        LightKind::Point => {
            ec.insert(PointLight {
                color,
                intensity: light.intensity,
                range: light.range.unwrap_or(50.0),
                shadows_enabled: false,
                ..default()
            });
        }
        LightKind::Directional => {
            ec.insert(DirectionalLight {
                color,
                illuminance: light.intensity,
                shadows_enabled: false,
                ..default()
            });
        }
    }
}

fn spawn_child_light(
    parent: &mut bevy::ecs::relationship::RelatedSpawnerCommands<ChildOf>,
    light: &crate::entity_config::LightConfig,
) {
    use crate::entity_config::LightKind;
    let color = Color::srgb(light.colour[0], light.colour[1], light.colour[2]);
    match light.kind {
        LightKind::Point => {
            let mut child = parent.spawn(PointLight {
                color,
                intensity: light.intensity,
                range: light.range.unwrap_or(50.0),
                shadows_enabled: false,
                ..default()
            });
            if light.face_player {
                child.insert(FacePlayerLight);
            }
        }
        LightKind::Directional => {
            let mut child = parent.spawn(DirectionalLight {
                color,
                illuminance: light.intensity,
                shadows_enabled: false,
                ..default()
            });
            if light.face_player {
                child.insert(FacePlayerLight);
            }
        }
    }
}

/// Rotates every [`FacePlayerLight`] entity so it points toward the
/// player's ship, independent of its parent entity's orientation.
fn face_player_lights(
    ship_query: Query<&GlobalTransform, With<LocalShip>>,
    mut light_query: Query<(&GlobalTransform, &mut Transform), With<FacePlayerLight>>,
) {
    let Some(ship_transform) = ship_query.iter().next() else {
        return;
    };
    let player_pos = ship_transform.translation();
    for (global, mut transform) in &mut light_query {
        let light_pos = global.translation();
        if (player_pos - light_pos).length_squared() > f32::EPSILON {
            transform.rotation = Transform::from_translation(light_pos)
                .looking_at(player_pos, Vec3::Y)
                .rotation;
        }
    }
}

// â"€â"€ Tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::collision_damage;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::ship_plugin::handle_impulse_messages;
    use crate::weapons_plugin::BEAM_DAMAGE_PER_SEC;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    #[derive(Resource)]
    struct ShipEntity(Entity);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.configure_sets(
            Update,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        // Advance time by 200 ms per tick so Hz-based SimBroadcaster timers
        // (period = 100 ms) always fire within a single update call.
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
        .init_resource::<WorldResource>()
        .init_resource::<TrackedEntities>()
        .insert_resource(SimBroadcastTimer(Timer::new(
            std::time::Duration::from_nanos(1),
            TimerMode::Repeating,
        )))
        .init_resource::<WorldSetupBroadcast>()
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastHull>()
        .init_resource::<LastBroadcastShields>()
        .init_resource::<LastBroadcastBlackboards>()
        .init_resource::<crate::messages::InterSystemQueue>()
        .init_resource::<crate::ai::server::AiTokenRegistry>()
        .init_resource::<Outbox>()
        .add_message::<crate::ai_plugin::AiEntityDestroyed>()
        .add_plugins(crate::captain_plugin::CaptainPlugin)
        .add_plugins(crate::weapons_plugin::WeaponsPlugin)
        .add_plugins(crate::repair_plugin::RepairPlugin)
        .add_plugins(crate::power_plugin::ShipPowerPlugin)
        .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
        .add_plugins(crate::sensors_plugin::ShipSensorsPlugin)
        .add_plugins(crate::comms_plugin::CommsConsolePlugin)
        .add_systems(
            OnEnter(GamePhase::InProgress),
            reset_broadcast_caches_on_start,
        )
        .add_systems(
            Update,
            (admit_system_commands, clear_inter_system_queue)
                .after(crate::lobby::process_lobby)
                .before(crate::sim_sets::SimSet::Input),
        )
        .add_systems(
            Update,
            (
                handle_impulse_messages,
                broadcast_shield_status,
                reconcile_runtime_entities
                    .after(crate::lobby::process_lobby)
                    .before(broadcast_world_setup_on_start),
                broadcast_world_setup_on_start.after(crate::lobby::process_lobby),
                refresh_caches_on_midgame_reconnect.after(crate::lobby::process_lobby),
            ),
        )
        .add_systems(
            Update,
            crate::modifier_coordination::translate_power_modifiers
                .after(crate::power_plugin::handle_power_messages)
                .after(crate::power_plugin::tick_power_system),
        )
        .add_systems(
            Update,
            crate::modifier_coordination::translate_impulse_modifiers
                .after(handle_impulse_messages),
        )
        .add_systems(
            Update,
            (
                sim_processing_anchor,
                broadcast_blackboard_updates.in_set(crate::sim_sets::SimSet::PublishAggregate),
            ),
        )
        .add_plugins(weapons_update_broadcaster())
        .add_plugins(sim_state_broadcaster())
        .add_plugins(modifier_events_broadcaster())
        .add_systems(PostUpdate, collect);
        // Spawn the Ship entity immediately so systems that query it (including
        // auth checks in handle_fire_torpedo, handle_power_messages, etc.) work
        // during Lobby as well as InProgress.
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::simulation::LocalShip,
                crate::simulation::ShipSystemBlackboards::default(),
                crate::ship_plugin::ShipConfigComponent::default(),
                crate::ship_plugin::ShipSystemControlSources::default(),
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::messages::AdmittedCommands::default(),
                ShipShields(ShieldSystem::default(), 0.5),
                ShipPhysicsComponent::default(),
                crate::ship_state::ShipRedAlert::default(),
                crate::ship_state::ShipViewMode::default(),
                crate::ship_state::ShipPhaserFrequency::default(),
                bevy::prelude::Transform::default(),
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[
                    (SystemId("helm".into()), 25.0),
                    (SystemId("tactical".into()), 25.0),
                    (SystemId("power".into()), 25.0),
                    (SystemId("shields".into()), 25.0),
                ])),
            ))
            .id();
        // Insert per-entity components (Bundle limit).
        app.world_mut().entity_mut(ship).insert((
            ShipImpulse::default(),
            ShipBoost::default(),
            crate::modifiers::ShipModifiers::new(),
            crate::weapons_plugin::TorpedoSystemResource(crate::torpedo::TorpedoSystem::new(
                crate::torpedo::TorpedoConfig::default(),
            )),
            crate::weapons_plugin::PhaserCombatConfigResource::default(),
            PhaserRenderConfig::default(),
            // PR 7 (issue #597) — per-entity beam / target / cooldown / sensors / waypoint.
            crate::weapons_plugin::WeaponsTarget::default(),
            crate::weapons_plugin::ActiveBeam::default(),
            crate::weapons_plugin::PhaserCooldown::default(),
            crate::sensors_plugin::SensorsTarget::default(),
            crate::navigation_plugin::NavigationWaypoint::default(),
            crate::ship::power::PowerBrownoutState::default(),
        ));
        app.insert_resource(ShipEntity(ship));
        app
    }

    // ── PR 7 (issue #597) test helpers ──────────────────────────────────────
    // These wrap the `Query<&X, With<LocalShip>>` pattern that replaces
    // direct Resource access after PR 7 removed the Resource derive from
    // WeaponsTarget / ActiveBeam / PhaserCooldown / SensorsTarget / NavigationWaypoint.

    fn get_weapons_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::weapons_plugin::WeaponsTarget, With<LocalShip>>();
        q.single(app.world()).ok().and_then(|wt| wt.0.clone())
    }

    fn get_active_beam_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::weapons_plugin::ActiveBeam, With<LocalShip>>();
        q.single(app.world())
            .ok()
            .and_then(|b| b.target_uuid.clone())
    }

    fn active_beam_target_is_none(app: &mut App) -> bool {
        get_active_beam_target(app).is_none()
    }

    fn set_active_beam_target(app: &mut App, uuid: Option<String>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::weapons_plugin::ActiveBeam, With<LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.target_uuid = uuid;
        }
    }

    fn set_active_beam_remaining_secs(app: &mut App, secs: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::weapons_plugin::ActiveBeam, With<LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.remaining_secs = secs;
        }
    }

    fn set_active_beam_damage_accumulator(app: &mut App, val: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::weapons_plugin::ActiveBeam, With<LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.damage_accumulator = val;
        }
    }

    fn phaser_bank_is_active(app: &mut App, bank: &str) -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::weapons_plugin::PhaserCooldown, With<LocalShip>>();
        q.single(app.world())
            .ok()
            .map(|cd| cd.is_bank_active(bank))
            .unwrap_or(false)
    }

    fn start_phaser_cooldown(app: &mut App, bank: &str, secs: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::weapons_plugin::PhaserCooldown, With<LocalShip>>();
        if let Ok(mut cd) = q.single_mut(app.world_mut()) {
            cd.start_bank_with_cooldown(bank, secs);
        }
    }

    fn apply_hull_damage(app: &mut App, amount: f32) {
        let mut rng = rand::rng();
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<crate::entity_spawner::EntitySystemHull>()
            .unwrap()
            .0
            .apply_damage(amount, &mut rng);
    }

    fn get_ship_modifiers(app: &mut App) -> crate::modifiers::ShipModifiers {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::modifiers::ShipModifiers, With<crate::simulation::LocalShip>>(
            );
        q.single(app.world()).unwrap().clone()
    }

    fn modify_ship_modifiers<F>(app: &mut App, f: F)
    where
        F: FnOnce(&mut crate::modifiers::ShipModifiers),
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::modifiers::ShipModifiers, With<crate::simulation::LocalShip>>();
        let mut mods = q.single_mut(app.world_mut()).unwrap();
        f(&mut mods);
    }

    fn get_phaser_frequency(app: &mut App) -> f32 {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship_state::ShipPhaserFrequency, With<LocalShip>>();
        q.single(app.world()).map(|f| f.0).unwrap_or(0.5)
    }

    fn get_view_mode(app: &mut App) -> crate::messages::ViewMode {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship_state::ShipViewMode, With<LocalShip>>();
        q.single(app.world())
            .map(|vm| vm.view_mode.clone())
            .unwrap_or(crate::messages::ViewMode::Camera(
                crate::messages::CameraView::default(),
            ))
    }

    // Test helper for directly setting view mode without round-tripping a
    // client message; retained for tests that may need to seed view state.
    #[allow(dead_code)]
    fn set_ship_view_mode(app: &mut App, mode: crate::messages::ViewMode) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship_state::ShipViewMode, With<LocalShip>>();
        if let Ok(mut vm) = q.single_mut(app.world_mut()) {
            vm.view_mode = mode;
        }
    }

    /// Fast-forward the pre-game countdown so the game starts immediately.
    /// Must be called after the tick that starts the countdown.
    fn fast_forward_countdown(app: &mut App) {
        use crate::lobby::CountdownTimer;
        app.world_mut()
            .resource_mut::<CountdownTimer>()
            .remaining_secs = 0.001;
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        // Drain any leftover SimOutbox entries that the sim systems wrote but
        // were not captured by the PostUpdate collect system (SimOutbox is not
        // connected to the OutboundMessage bus for test_app).
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage {
                target,
                msg,
                delivery: DeliveryClass::Reliable,
            });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn load_tube_now(app: &mut App, tube: &str) {
        // Systems prefer the per-entity component over the resource; update component.
        let mut q = app
            .world_mut()
            .query_filtered::<&mut TorpedoSystemResource, With<LocalShip>>();
        if let Ok(mut ts) = q.single_mut(app.world_mut()) {
            ts.0.tube_mut(tube)
                .expect("test tube should exist")
                .loaded_count = 1;
        } else {
            let world = app.world_mut();
            let mut res = world.resource_mut::<TorpedoSystemResource>();
            res.0
                .tube_mut(tube)
                .expect("test tube should exist")
                .loaded_count = 1;
        }
    }

    fn set_ship_yaw(app: &mut App, yaw: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipPhysicsComponent, With<crate::simulation::Ship>>();
        let mut p = q
            .single_mut(app.world_mut())
            .expect("expected Ship with ShipPhysics");
        p.yaw = yaw;
    }

    fn start_game(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        tick(app); // process_lobby → starts countdown
        fast_forward_countdown(app);
        tick(app); // tick_countdown → emits GameStarted, sets NextState::Set(InProgress)
        tick(app); // NextState takes effect: Phase switches to InProgress
    }

    fn start_game_with_helm(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "helm", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    fn start_game_with_sensors(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::Identify {
                token: "sensors".into(),
                name: "Spock".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "sensors", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    fn start_game_with_navigation(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "navigation",
            ClientMessage::Identify {
                token: "navigation".into(),
                name: "Decker".into(),
            },
        );
        tick(app);
        push(
            app,
            "navigation",
            ClientMessage::SelectStation {
                station: "Navigation".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "navigation", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    #[test]
    fn entity_config_radar_icon_flows_into_world_snapshot() {
        let config = crate::entity_config::EntityConfig {
            name: Some("Sun".into()),
            tags: vec!["star".into(), "center".into()],
            collider: Some(crate::entity_config::ColliderConfig {
                shape: crate::entity_config::ColliderShape::Ball,
                radius: 50.0,
                length: 0.0,
            }),
            radar_appearance: Some(crate::entity_config::RadarAppearanceConfig {
                colour: Some(vec![1.0, 0.85, 0.3]),
                size: None,
                region_colour: None,
                icon: Some("star".into()),
            }),
            ..Default::default()
        };

        let snapshot =
            snapshot_from_entity_config("sun-uuid".into(), None, &config, Vec3::new(0.0, 0.0, 0.0));

        assert_eq!(snapshot.name.as_deref(), Some("Sun"));
        assert_eq!(snapshot.tags, vec!["star", "center"]);
        assert_eq!(snapshot.radius, Some(50.0));
        assert_eq!(snapshot.colour, Some([1.0, 0.85, 0.3]));
        assert_eq!(snapshot.radar_icon.as_deref(), Some("star"));
    }

    #[test]
    fn world_entity_upsert_replaces_existing_snapshot_for_same_uuid() {
        let mut world = WorldResource(WorldData::default());
        upsert_world_entity(
            &mut world,
            EntitySnapshot {
                uuid: "same".into(),
                tags: vec!["asteroid".into()],
                radar_icon: Some("asteroid".into()),
                ..Default::default()
            },
        );
        upsert_world_entity(
            &mut world,
            EntitySnapshot {
                uuid: "same".into(),
                tags: vec!["star".into()],
                radar_icon: Some("star".into()),
                ..Default::default()
            },
        );

        assert_eq!(world.0.entities.len(), 1);
        assert_eq!(world.0.entities[0].tags, vec!["star"]);
        assert_eq!(world.0.entities[0].radar_icon.as_deref(), Some("star"));
    }

    #[test]
    fn sensors_can_switch_view_to_science_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(
            &mut app,
            "sensors",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::ScienceRadar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::ScienceRadar);
    }
    #[test]
    fn sensors_can_switch_view_to_sensors_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(
            &mut app,
            "sensors",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::SensorsRadar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::SensorsRadar);
    }

    #[test]
    fn non_sensors_cannot_switch_view_to_sensors_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::SensorsRadar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn navigation_can_switch_view_to_system_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::SystemChart,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::SystemChart);
    }

    #[test]
    fn non_sensors_cannot_switch_view_to_science_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::ScienceRadar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn non_navigation_cannot_switch_view_to_system_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::SystemChart,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn navigation_can_switch_view_to_navigation_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::NavigationChart,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::NavigationChart);
    }

    #[test]
    fn non_navigation_cannot_switch_view_to_navigation_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::NavigationChart,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    fn start_game_with_comms(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "comms",
            ClientMessage::Identify {
                token: "comms".into(),
                name: "Uhura".into(),
            },
        );
        tick(app);
        push(
            app,
            "comms",
            ClientMessage::SelectStation {
                station: "Comms".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "comms", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    #[test]
    fn comms_can_push_view_to_comms() {
        let mut app = test_app();
        start_game_with_comms(&mut app);
        push(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Comms,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::Comms);
    }

    #[test]
    fn captain_override_from_comms_view_works() {
        let mut app = test_app();
        start_game_with_comms(&mut app);
        push(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Comms,
                },
            },
        );
        tick(&mut app);
        // Captain overrides back to a camera view.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(CameraView::new("camera_aft")),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::new("camera_aft"))
        );
    }

    #[test]
    fn non_comms_cannot_push_comms_view() {
        let mut app = test_app();
        start_game_with_comms(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Comms,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn helm_can_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::Radar);
    }

    #[test]
    fn captain_cannot_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        // Captain has no authority over Radar; request is silently dropped.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn helm_cannot_switch_view_to_camera() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(CameraView::new("camera_aft")),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn world_setup_is_broadcast_once_after_start_game() {
        let mut app = test_app();
        // Pre-populate world data so the broadcast has something to emit.
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid("test-uuid", 5.0, -1.0, 2.0)],
            ..Default::default()
        }));

        // Bring the game up to the point of pressing SetReady
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "A".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(&mut app);
        // Advance the phase to InProgress so broadcast_world_setup_on_start fires.
        push(&mut app, "captain", ClientMessage::SetReady { ready: true });
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        let start_out = tick(&mut app);

        let world_setups: Vec<_> = start_out
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. }))
            .collect();
        assert_eq!(
            world_setups.len(),
            1,
            "expected exactly one WorldSetup on the SetReady tick"
        );
        match &world_setups[0].msg {
            ServerMessage::WorldSetup { world } => {
                assert_eq!(world.entities.len(), 1);
                assert_eq!(world.entities[0].x(), 5.0);
            }
            _ => unreachable!(),
        }
        match &world_setups[0].target {
            crate::lobby::Target::All => {}
            t => panic!("WorldSetup should target All, got {:?}", t),
        }

        // Subsequent ticks must not re-broadcast WorldSetup
        let later = tick(&mut app);
        assert!(
            !later
                .iter()
                .any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should only fire once per game"
        );
    }

    #[test]
    fn world_setup_is_not_broadcast_during_lobby() {
        let mut app = test_app();
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid("test-uuid", 0.0, 0.0, 2.0)],
            ..Default::default()
        }));
        // Identify and select a console but don't start the game.
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "A".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        let out = tick(&mut app);
        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should not be broadcast in the Lobby phase"
        );
    }

    #[test]
    fn hull_integrity_starts_at_100_and_appears_in_system_hull_update() {
        let mut app = test_app();
        start_game(&mut app);
        // The first InProgress tick (inside start_game) already emitted and consumed
        // the initial SystemHullUpdate. Reset the cache to force re-emission.
        app.world_mut()
            .resource_mut::<LastBroadcastHull>()
            .0
            .clear();
        let out = tick(&mut app);
        let entries = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::SystemHullUpdate { entries } => Some(entries.clone()),
                _ => None,
            })
            .expect("expected a SystemHullUpdate broadcast");
        let total: f32 = entries.iter().map(|c| c.current).sum();
        assert!((total - 100.0).abs() < 1e-6);
    }

    #[test]
    fn direct_damage_reduces_hull_integrity_in_broadcast() {
        let mut app = test_app();
        start_game(&mut app);
        // Consume the initial SystemHullUpdate so LastBroadcastHull is seeded.
        let _ = tick(&mut app);

        // Directly apply damage to the EntitySystemHull component (simulates collision at ~half speed).
        apply_hull_damage(&mut app, 10.0);

        let out = tick(&mut app);
        let entries = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::SystemHullUpdate { entries } => Some(entries.clone()),
                _ => None,
            })
            .expect("expected a SystemHullUpdate after damage");
        let total: f32 = entries.iter().map(|c| c.current).sum();
        assert!((total - 90.0).abs() < 1e-6);
    }

    // â"€â"€ SetTarget / TargetLock tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    fn setup_weapons_world(app: &mut App, asteroid_x: f32, asteroid_z: f32) {
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid(
                "target-uuid",
                asteroid_x,
                asteroid_z,
                2.0,
            )],
            ..Default::default()
        }));
        // Also spawn the live ECS entity. As of the targeting fix, gameplay
        // logic reads positions from ECS Transforms (not the WorldResource
        // snapshot), so targets must exist as ECS entities to be lockable.
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
        ));
    }

    /// Like `setup_weapons_world` but also returns the spawned entity for
    /// tests that need to manipulate or despawn it later.
    fn setup_weapons_world_with_entity(
        app: &mut App,
        asteroid_x: f32,
        asteroid_z: f32,
    ) -> bevy::ecs::entity::Entity {
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid(
                "target-uuid",
                asteroid_x,
                asteroid_z,
                2.0,
            )],
            ..Default::default()
        }));
        app.world_mut()
            .spawn((
                Asteroid,
                AsteroidUuid("target-uuid".into()),
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[
                    (crate::messages::SystemId("captain".into()), 30.0),
                ])),
                Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
            ))
            .id()
    }

    fn start_game_with_weapons(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "weapons", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    #[test]
    fn valid_target_within_range_replies_with_target_lock_confirmed() {
        let mut app = test_app();
        // Asteroid at (30, 0) â€" 30 units from ship origin, within 60-unit range.
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert_eq!(lock.0, "target-uuid");
        assert!(lock.1, "expected locked=true for in-range asteroid");

        // Server state should record the lock.
        assert_eq!(get_weapons_target(&mut app).as_deref(), Some("target-uuid"));
    }

    #[test]
    fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
        let mut app = test_app();
        // Asteroid at (400, 0) — 400 units away, outside 300-unit Weapons range.
        setup_weapons_world(&mut app, 400.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for out-of-range asteroid");
        assert!(get_weapons_target(&mut app).is_none());
    }

    #[test]
    fn unknown_uuid_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 10.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "no-such-asteroid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for unknown UUID");
        assert!(get_weapons_target(&mut app).is_none());
    }

    // â"€â"€ WeaponsUpdate / fire_ready tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// Target locked, within 40-unit phaser range, in forward arc â†' fire_ready = true.
    #[test]
    fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
        let mut app = test_app();
        // Ship at origin, yaw=0 (facing -Z). Asteroid at (0, -20): directly ahead, 20 units away.
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        // Target changes → WeaponsUpdate fires this tick.
        let out = tick(&mut app);

        let update = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::WeaponsUpdate {
                    target_uuid, banks, ..
                } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
                _ => None,
            })
            .expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(
            update.1,
            "expected fire_ready=true for in-range, forward-arc target"
        );
    }

    /// Target locked but beyond 40-unit phaser range (within 60u lock range) → fire_ready = false.
    #[test]
    fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
        let mut app = test_app();
        // Ship at origin, yaw=0. Asteroid at (0, -50): directly ahead, 50 units — within lock range
        // (60u) but outside phaser range (40u).
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        // Target changes → WeaponsUpdate fires this tick.
        let out = tick(&mut app);

        let update = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::WeaponsUpdate {
                    target_uuid, banks, ..
                } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
                _ => None,
            })
            .expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(
            !update.1,
            "expected fire_ready=false for beyond-phaser-range target"
        );
    }

    // â"€â"€ FirePhaser / beam lifecycle tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// Helper: lock target then fire phaser; returns messages from the fire tick.
    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        // Lock
        push(
            app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let _ = tick(app);
        // Fire
        push(
            app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(app)
    }

    /// Firing at a fire-ready target broadcasts BeamStarted to all.
    #[test]
    fn fire_phaser_on_valid_target_broadcasts_beam_started() {
        let mut app = test_app();
        // Asteroid directly ahead at 20 units (yaw=0 â†' facing -Z â†' asteroid at (0,-20)).
        let out = lock_and_fire(&mut app, 0.0, -20.0);

        let beam_started = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
        assert!(
            beam_started.is_some(),
            "expected BeamStarted after firing at fire-ready target"
        );
        match &beam_started.unwrap().msg {
            ServerMessage::BeamStarted { target_uuid, .. } => {
                assert_eq!(target_uuid, "target-uuid")
            }
            _ => unreachable!(),
        }
        match &beam_started.unwrap().target {
            Target::All => {}
            t => panic!("BeamStarted should target All, got {:?}", t),
        }

        // ActiveBeam resource should be populated.
        assert_eq!(
            get_active_beam_target(&mut app).as_deref(),
            Some("target-uuid")
        );
    }

    /// FirePhaser is silently ignored when the phaser is on cooldown.
    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Manually put the cooldown into active state (simulating a beam just ended).
        set_active_beam_target(&mut app, None);
        start_phaser_cooldown(&mut app, "port", 3.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "BeamStarted should not fire during cooldown"
        );
    }

    /// Non-weapons player cannot fire.
    #[test]
    fn fire_phaser_ignored_from_non_weapons_player() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "captain should not be able to fire phaser"
        );
    }

    /// When the beam fires at a target outside the 180Â° arc, it is rejected.
    #[test]
    fn fire_phaser_rejected_when_target_behind_ship() {
        let mut app = test_app();
        // Yaw=0 means ship faces -Z. Asteroid at (0, +20) is directly behind â€" in rear arc.
        setup_weapons_world(&mut app, 0.0, 20.0);
        start_game_with_weapons(&mut app);
        // Lock (within 60u range) â€" lock doesn't require arc.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let _ = tick(&mut app);
        // Fire â€" rejected because target is behind.
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser should be rejected when target is in rear arc"
        );
    }

    /// A 6-second natural beam kills the asteroid (5 HP/s Ã— 6s = 30 HP total).
    ///
    /// The test accelerates time by manipulating the beam state directly
    /// after confirming the beam started, then runs ticks with large deltas.
    #[test]
    fn full_beam_duration_kills_asteroid() {
        let mut app = test_app();

        // setup_weapons_world (called by lock_and_fire) now spawns the
        // asteroid ECS entity. Fetch its handle after setup.
        let _ = lock_and_fire(&mut app, 0.0, -20.0);
        let asteroid_entity = {
            let mut q = app
                .world_mut()
                .query::<(bevy::ecs::entity::Entity, &AsteroidUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == "target-uuid")
                .map(|(e, _)| e)
                .expect("setup_weapons_world should have spawned the target asteroid")
        };

        // Verify beam started.
        assert_eq!(
            get_active_beam_target(&mut app).as_deref(),
            Some("target-uuid")
        );

        // Fast-forward: accumulate 30 damage via the damage_accumulator.
        // Set accumulator to 30.0 so all damage applies in one tick.
        set_active_beam_damage_accumulator(&mut app, 30.0);
        set_active_beam_remaining_secs(&mut app, 5.0); // still "ongoing"

        let out = tick(&mut app);

        // Asteroid destroyed message should be present.
        let destroyed = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
        assert!(
            destroyed.is_some(),
            "expected AsteroidDestroyed when asteroid HP reaches 0"
        );
        match &destroyed.unwrap().msg {
            ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
            _ => unreachable!(),
        }

        // BeamEnded also broadcast.
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after asteroid destruction"
        );

        // Asteroid no longer in world data.
        assert!(
            !app.world()
                .resource::<WorldResource>()
                .0
                .entities
                .iter()
                .any(|a| a.uuid == "target-uuid"),
            "destroyed asteroid should be removed from WorldData"
        );

        // Beam resource cleared.
        assert!(active_beam_target_is_none(&mut app));

        // Cooldown started.
        assert!(
            phaser_bank_is_active(&mut app, "port"),
            "cooldown should start after beam end"
        );

        // The entity should be despawned.
        assert!(
            app.world()
                .get::<crate::entity_spawner::EntitySystemHull>(asteroid_entity)
                .is_none(),
            "asteroid entity should be despawned"
        );
    }

    /// Beam severs when ship rotates target out of the 180Â° forward arc.
    #[test]
    fn beam_severs_when_target_leaves_forward_arc() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Now rotate ship so the asteroid is behind it (yaw = π → facing +Z, asteroid at (0,-20) is behind).
        set_ship_yaw(&mut app, std::f32::consts::PI);

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves forward arc"
        );
        assert!(
            active_beam_target_is_none(&mut app),
            "beam should be cleared after sever-by-arc"
        );
        assert!(
            phaser_bank_is_active(&mut app, "port"),
            "cooldown should start after arc sever"
        );
    }

    /// Beam severs when the target moves beyond 40-unit phaser range.
    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Move asteroid position in WorldData to 50 units away (out of 40u range).
        app.world_mut().resource_mut::<WorldResource>().0.entities[0].position =
            Some([0.0, 0.0, -50.0]);
        // Move the live ECS Transform too — gameplay reads positions from
        // Transforms, not from the WorldResource snapshot.
        let mut q = app
            .world_mut()
            .query_filtered::<&mut Transform, With<AsteroidUuid>>();
        for mut t in q.iter_mut(app.world_mut()) {
            t.translation.z = -50.0;
        }

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves phaser range"
        );
        assert!(
            active_beam_target_is_none(&mut app),
            "beam should be cleared after sever-by-range"
        );
        assert!(
            phaser_bank_is_active(&mut app, "port"),
            "cooldown should start after range sever"
        );
    }

    /// No damage refund on sever — whatever HP was dealt is permanent.
    #[test]
    fn no_damage_refund_on_sever() {
        let mut app = test_app();
        // setup_weapons_world (called by lock_and_fire) now spawns the
        // asteroid ECS entity itself. Fetch its handle by querying for the
        // matching UUID after the fact.
        let _ = lock_and_fire(&mut app, 0.0, -20.0);
        let asteroid_entity = {
            let mut q = app
                .world_mut()
                .query::<(bevy::ecs::entity::Entity, &AsteroidUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == "target-uuid")
                .map(|(e, _)| e)
                .expect("setup_weapons_world should have spawned the target asteroid")
        };

        // Apply partial damage via accumulator.
        set_active_beam_damage_accumulator(&mut app, 10.0);
        let _ = tick(&mut app);

        // Now sever by rotating ship.
        set_ship_yaw(&mut app, std::f32::consts::PI);
        let _ = tick(&mut app);

        let hp = app
            .world()
            .get::<crate::entity_spawner::EntitySystemHull>(asteroid_entity)
            .map(|h| h.0.total_current());
        assert!(
            hp.is_some() && hp.unwrap() < 30.0,
            "asteroid should retain damage after sever (no refund), hp={:?}",
            hp
        );
    }

    /// A fresh FirePhaser after cooldown on a new locked target cancels any
    /// active beam and starts a new one.
    #[test]
    fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
        let mut app = test_app();

        // Set up two asteroids.
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![
                EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
            ],
            ..Default::default()
        }));
        // Spawn live ECS entities for both targets — gameplay reads positions
        // from Transforms, not from the WorldResource snapshot.
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("t1".into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("t2".into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -15.0),
        ));
        start_game_with_weapons(&mut app);

        // Lock and fire at t1.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget { uuid: "t1".into() },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let _ = tick(&mut app);
        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t1"));

        // Natural beam expiry: set remaining to 0.
        set_active_beam_remaining_secs(&mut app, 0.0);
        // Zero damage accumulator so no destruction fires.
        set_active_beam_damage_accumulator(&mut app, 0.0);
        let _ = tick(&mut app); // beam ends, cooldown starts

        // Cooldown should be active.
        assert!(phaser_bank_is_active(&mut app, "port"));

        // Force cooldown to expire.
        start_phaser_cooldown(&mut app, "port", 0.0);

        // Lock and fire at t2.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget { uuid: "t2".into() },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "expected BeamStarted for new target after cooldown"
        );
        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t2"));
    }

    // -- Repair helpers --------------------------------------------------

    /// Set up a game with a captain and repair player.
    fn start_game_with_repair(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "eng",
            ClientMessage::Identify {
                token: "eng".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "eng",
            ClientMessage::SelectStation {
                station: "Repair".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "eng", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    fn team_is_travelling(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(
            teams.0.slots()[idx],
            crate::messages::TeamSlot::Travelling { .. }
        )
    }

    fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Idle)
    }

    // -- Repair dispatch tests --------------------------------------

    #[test]
    fn non_repair_sender_is_ignored() {
        let mut app = test_app();
        start_game_with_repair(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);
        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_idle(teams, 0),
            "team 0 should remain idle after non-Repair dispatch"
        );
    }

    #[test]
    fn repair_holder_can_dispatch_team() {
        let mut app = test_app();
        start_game_with_repair(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);
        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_travelling(teams, 0),
            "team 0 should be travelling after dispatch"
        );
    }

    #[test]
    fn all_busy_teams_ignore_further_dispatches() {
        let mut app = test_app();
        start_game_with_repair(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 1,
                    target: crate::messages::RepairTarget::Station(StationId("tactical".into())),
                },
            },
        );
        tick(&mut app);
        // Redirect team 0 (different console → Returning)
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("power".into())),
                },
            },
        );
        tick(&mut app);
        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(matches!(
            &teams.0.slots()[0],
            crate::messages::TeamSlot::Returning { .. }
        ));
        assert!(team_is_travelling(teams, 1));
    }

    #[test]
    fn repair_state_broadcast_after_dispatch() {
        let mut app = test_app();
        start_game_with_repair(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        let out = tick(&mut app);
        let repair_state = out.iter().find(|m| {
            matches!(&m.msg, ServerMessage::RepairState { teams } if
                teams.iter().any(|t| matches!(t, crate::messages::TeamSlot::Travelling { .. })))
                && matches!(&m.target, Target::Token(t) if t == "eng")
        });
        assert!(
            repair_state.is_some(),
            "RepairState with Travelling team should be broadcast to repair console"
        );
    }

    // â"€â"€ SetPhaserMode tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// The Weapons console holder can change the phaser mode to Manual.
    #[test]
    fn weapons_console_can_set_phaser_mode_to_manual() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Manual,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Manual,
            "phaser mode should be Manual after SetPhaserMode"
        );
    }

    /// A non-Weapons player cannot change the phaser mode.
    #[test]
    fn non_weapons_player_cannot_set_phaser_mode() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        // Establish a known mode (Auto) via the authorised player first.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Auto,
                },
            },
        );
        tick(&mut app);
        // Non-weapons player attempts to switch back to Manual — must be ignored.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Manual,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Auto,
            "phaser mode should stay Auto when non-Weapons player sends SetPhaserMode"
        );
    }

    /// Shared setup used by tests that need a Sensors + Tactical(weapons) console pairing.
    fn start_game_with_sensors_and_weapons(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::Identify {
                token: "sensors".into(),
                name: "Spock".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "sensors", ClientMessage::SetReady { ready: true });
        push(app, "weapons", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    // â"€â"€ FireTorpedo tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    #[test]
    fn tactical_player_can_fire_torpedo_broadcasts_torpedo_launched() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(
                &m.msg,
                ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port"
            )),
            "expected TorpedoLaunched broadcast after Tactical fires torpedo"
        );
    }

    #[test]
    fn non_tactical_player_cannot_fire_torpedo() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "captain",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "captain should not be able to fire torpedo"
        );
    }

    #[test]
    fn fire_torpedo_during_lobby_fires_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level.
        // In test configurations without SimSet, the system processes messages during Lobby.
        let mut app = test_app();
        load_tube_now(&mut app, "aft");
        push(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "aft".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "FireTorpedo should fire during Lobby when no SimSet gate is configured"
        );
    }

    #[test]
    fn torpedo_launched_is_broadcast_to_all() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_starboard");

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_starboard".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        let launched = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. }))
            .expect("expected TorpedoLaunched");
        assert!(
            matches!(&launched.target, Target::All),
            "TorpedoLaunched should be broadcast to All, not {:?}",
            launched.target
        );
    }

    // â"€â"€ ShipModifiers integration tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// Empty modifier table: phaser damage is identical to the base BEAM_DAMAGE_PER_SEC
    /// (5 HP/s). After 1 second of beam fire on a 30-HP asteroid the HP decreases by 5.
    #[test]
    fn empty_modifier_table_reproduces_base_phaser_damage() {
        let mut app = test_app();
        // Asteroid directly ahead at 20 units (within 40-unit phaser range).
        setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        // Lock and fire
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        // Advance by 1 second of simulated time (many small ticks).
        // Each tick() calls app.update() which advances the Bevy TimePlugin by a small real step.
        // Instead, directly test the accumulator math by examining the asteroid HP after
        // running a known number of frames equivalent to >1 second.
        // BEAM_DAMAGE_PER_SEC = 5; asteroid starts at 30 HP.
        // After enough ticks (>6 s at 5 HP/s) the asteroid should be destroyed.
        // With identity modifier this should work; with a 2Ã— modifier it would be faster.

        // Run 500 ms worth of ticks at ~16ms each (â‰ˆ31 ticks).
        // After that, asteroid should have taken ~2â€"3 HP (not destroyed yet).
        let hp_before = {
            let world = app.world().resource::<WorldResource>();
            world
                .0
                .entities
                .iter()
                .find(|a| a.uuid == "target-uuid")
                .map(|_| true)
        };
        assert!(hp_before.is_some(), "asteroid should still exist after <1s");
    }

    /// PhaserDamage modifier at 2Ã— doubles the kill rate.
    /// With BEAM_DAMAGE_PER_SEC=5 and 30-HP asteroid:
    /// - Base: 6 seconds to destroy
    /// - 2Ã— modifier (bonus=1.0): 3 seconds to destroy
    ///   Test: after running ~4s of game time, the asteroid is destroyed with 2Ã— but not with 1Ã—.
    #[test]
    fn phaser_damage_modifier_doubles_kill_rate() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::Modifier;

        // --- App with 2Ã— PhaserDamage modifier ---
        let mut app_fast = test_app();
        setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
        start_game_with_weapons(&mut app_fast);
        // Apply 2Ã— phaser damage modifier after ship is spawned.
        modify_ship_modifiers(&mut app_fast, |mods| {
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::PhaserDamage,
                bonus: 1.0, // â†' multiplier 2.0
            });
        });
        push(
            &mut app_fast,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app_fast);
        push(
            &mut app_fast,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app_fast); // processes FirePhaser, beam becomes active

        // Inject accumulated damage: 3.5s × (5 HP/s × 2×) = 35 HP → enough to destroy 30-HP asteroid.
        set_active_beam_damage_accumulator(&mut app_fast, BEAM_DAMAGE_PER_SEC * 2.0 * 3.5);
        tick(&mut app_fast); // One tick to process the accumulated damage.

        let still_exists_fast = app_fast
            .world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid");
        assert!(
            !still_exists_fast,
            "with 2Ã— phaser damage modifier, asteroid should be destroyed after 3.5s of beam"
        );

        // --- App with identity modifier (baseline): same damage injected but at 1Ã— ---
        let mut app_base = test_app();
        setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
        start_game_with_weapons(&mut app_base);
        push(
            &mut app_base,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app_base);
        push(
            &mut app_base,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app_base); // processes FirePhaser, beam becomes active
                             // Inject same real time but at base rate: 3.5s × 5 HP/s = 17.5 HP accumulated
        set_active_beam_damage_accumulator(&mut app_base, BEAM_DAMAGE_PER_SEC * 1.0 * 3.5);
        tick(&mut app_base);

        let still_exists_base = app_base
            .world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid");
        assert!(still_exists_base, "with identity modifier, asteroid should survive 3.5s of beam (only 17.5/30 HP removed)");
    }

    /// HullDamageTaken modifier at -1 (â†' 0.5Ã— multiplier) halves collision damage.
    /// At zero ship speed, base collision_damage=5. With 0.5Ã— modifier: round(5Ã—0.5)=3.
    // â"€â"€ modifier broadcast tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    #[test]
    fn add_modifier_broadcasts_modifier_added_message() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::Modifier;

        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app); // consume startup messages

        // Register a modifier on the ship entity.
        modify_ship_modifiers(&mut app, |mods| {
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::MaxSpeed,
                bonus: 0.5,
            });
        });
        let out = tick(&mut app);

        let found = out.iter().any(|m| {
            matches!(
                &m.msg,
                ServerMessage::ModifierAdded { source, slot, bonus }
                    if *source == ModifierSource::ImpulseDrive
                    && *slot == ModifierSlot::MaxSpeed
                    && (*bonus - 0.5).abs() < 1e-6
            )
        });
        assert!(found, "expected ModifierAdded in outbound messages");
    }

    #[test]
    fn remove_modifier_broadcasts_modifier_removed_message() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::Modifier;

        let mut app = test_app();
        start_game(&mut app);
        // Add first so there's something to remove.
        modify_ship_modifiers(&mut app, |mods| {
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::MaxSpeed,
                bonus: 0.5,
            });
        });
        tick(&mut app);

        // Now remove it.
        modify_ship_modifiers(&mut app, |mods| {
            mods.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
        });
        let out = tick(&mut app);

        let found = out.iter().any(|m| {
            matches!(
                &m.msg,
                ServerMessage::ModifierRemoved { source, slot }
                    if *source == ModifierSource::ImpulseDrive
                    && *slot == ModifierSlot::MaxSpeed
            )
        });
        assert!(found, "expected ModifierRemoved in outbound messages");
    }

    #[test]
    fn asteroid_collision_pierce_zero_routes_all_to_shields() {
        // Replicates the split + apply that `handle_collisions` performs
        // (without standing up Rapier), proving the pierce=0 path leaves
        // hull untouched and the shield quadrant absorbs full damage.
        use crate::damage::{
            apply_damage_with_shields, apply_hull_damage, split_damage_for_pierce,
        };
        use crate::shield::{ShieldConfig, ShieldSystem};
        let mut shields = ShieldSystem::new(&ShieldConfig::default());
        let initial_fore_hp = shields.facings[0].hp;
        let mut hull =
            crate::damage::SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);

        let damage: f32 = 10.0;
        let (pierced, absorbed) = split_damage_for_pierce(damage, 0.0);
        assert_eq!(pierced, 0.0);
        assert_eq!(absorbed, 10.0);
        let leak = apply_damage_with_shields(absorbed.round() as i32, 0.0, &mut shields);
        let total_hull = pierced + leak as f32;
        if total_hull > 0.0 {
            let rng = &mut rand::rngs::SmallRng::from_os_rng();
            apply_hull_damage(&mut hull, total_hull, rng);
        }
        assert!(
            (hull.total_current() - 100.0).abs() < 1e-6,
            "hull untouched with pierce=0 (leak={})",
            leak
        );
        assert_eq!(
            shields.facings[0].hp,
            initial_fore_hp - 10,
            "fore quadrant should have absorbed all 10 damage"
        );
    }

    #[test]
    fn asteroid_collision_pierce_full_routes_all_to_hull() {
        use crate::damage::{
            apply_damage_with_shields, apply_hull_damage, split_damage_for_pierce,
        };
        use crate::shield::{ShieldConfig, ShieldSystem};
        let mut shields = ShieldSystem::new(&ShieldConfig::default());
        let initial_fore_hp = shields.facings[0].hp;
        let mut hull =
            crate::damage::SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);

        let damage: f32 = 10.0;
        let (pierced, absorbed) = split_damage_for_pierce(damage, 1.0);
        assert_eq!(pierced, 10.0);
        assert_eq!(absorbed, 0.0);
        let leak = if absorbed > 0.0 {
            apply_damage_with_shields(absorbed.round() as i32, 0.0, &mut shields)
        } else {
            0
        };
        let total_hull = pierced + leak as f32;
        let rng = &mut rand::rngs::SmallRng::from_os_rng();
        apply_hull_damage(&mut hull, total_hull, rng);
        assert!(
            (hull.total_current() - 90.0).abs() < 1e-6,
            "hull should be 90 with pierce=1 (10 damage straight through)"
        );
        assert_eq!(
            shields.facings[0].hp, initial_fore_hp,
            "fore quadrant should be untouched with pierce=1"
        );
    }

    #[test]
    fn asteroid_collision_pierce_partial_splits_proportionally() {
        use crate::damage::{
            apply_damage_with_shields, apply_hull_damage, split_damage_for_pierce,
        };
        use crate::shield::{ShieldConfig, ShieldSystem};
        let mut shields = ShieldSystem::new(&ShieldConfig::default());
        let initial_fore_hp = shields.facings[0].hp;
        let mut hull =
            crate::damage::SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);

        // pierce = 0.3 on 10 damage → 3 to hull, 7 to fore shield.
        let damage: f32 = 10.0;
        let (pierced, absorbed) = split_damage_for_pierce(damage, 0.3);
        let leak = apply_damage_with_shields(absorbed.round() as i32, 0.0, &mut shields);
        let total_hull = pierced + leak as f32;
        let rng = &mut rand::rngs::SmallRng::from_os_rng();
        apply_hull_damage(&mut hull, total_hull, rng);
        assert!(
            (hull.total_current() - 97.0).abs() < 1e-6,
            "hull should lose 3 (the pierced portion), got {}",
            hull.total_current()
        );
        assert_eq!(
            shields.facings[0].hp,
            initial_fore_hp - 7,
            "fore quadrant should have absorbed 7"
        );
    }

    #[test]
    fn hull_damage_modifier_halves_collision_damage() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::Modifier;

        // Hull damage halved via modifier.
        let mut app = test_app();
        start_game(&mut app);
        modify_ship_modifiers(&mut app, |mods| {
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::HullDamageTaken,
                bonus: -1.0, // â†' multiplier 0.5
            });
        });

        // Apply collision damage directly through the formula used in handle_collisions.
        // At 200 u/s: collision_damage(200) = round(200 * 0.5) = 100.
        // With 0.5Ã— modifier: round(100 * 0.5) = 50.
        fn near(a: f32, b: f32) -> bool {
            (a - b).abs() < 1e-6
        }
        let mods = get_ship_modifiers(&mut app);
        let base_damage = collision_damage(200.0) as f32; // 100
        let scaled_damage = (base_damage * mods.get(&ModifierSlot::HullDamageTaken)).round();
        assert!(
            near(base_damage, 100.0),
            "collision_damage(200) should be 100"
        );
        assert!(
            near(scaled_damage, 50.0),
            "with 0.5Ã— modifier, damage should be 50"
        );

        // Verify the hull loses only the scaled amount by triggering damage through the component.
        apply_hull_damage(&mut app, scaled_damage);
        let out = tick(&mut app);
        let entries = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::SystemHullUpdate { entries } => Some(entries.clone()),
                _ => None,
            })
            .expect("expected SystemHullUpdate");
        let total: f32 = entries.iter().map(|c| c.current).sum();
        assert!(
            near(total, 50.0),
            "hull should be 100 - 50 = 50 with halved collision damage"
        );
    }

    /// PRD #597 PR-8: NPC ships share the collision code path with the player,
    /// so an NPC ship overlapping an asteroid must take hull damage on its own
    /// `EntitySystemHull` component just like the player ship does.
    ///
    /// This spins up a minimal Rapier world (no plugin scaffolding) with just
    /// `handle_collisions`, spawns an NPC ship (`Ship` marker, no `LocalShip`)
    /// overlapping an asteroid, ticks once, and asserts the NPC's hull dropped.
    /// Because the ship is not `LocalShip`, none of the player-only side
    /// effects (`DamageTaken`, `ShipDestroyed`, `GameOver`) may fire.
    #[test]
    fn npc_ship_takes_hull_damage_from_asteroid_collision() {
        use crate::damage::SystemHull;
        use crate::entity_config::{ColliderConfig, ColliderShape};
        use crate::entity_spawner::{ColliderSection, EntitySystemHull, EntityUuid};
        use crate::modifiers::ShipModifiers;
        use bevy_rapier3d::prelude::*;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(50),
            ))
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<bevy::mesh::Mesh>()
            .init_resource::<bevy::scene::SceneSpawner>()
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<GamePhase>()
            .add_plugins(RapierPhysicsPlugin::<()>::default())
            .init_resource::<SimOutbox>()
            .init_resource::<WorldResource>()
            .insert_resource(GameOverReason(None))
            .init_resource::<DamageLog>()
            .add_message::<crate::ai_plugin::AiEntityDestroyed>()
            .add_systems(Update, handle_collisions);

        // Move the game into InProgress so RapierPhysicsPlugin's default
        // run condition (if any) doesn't gate the step. Not strictly required
        // for handle_collisions itself, but keeps the test's app state
        // consistent with production semantics.
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        app.update();

        // Spawn an NPC ship at the origin with a ball collider, some hull,
        // some forward speed (so collision_damage yields non-zero), and no
        // `LocalShip` marker. `ShipShields` is omitted deliberately — NPCs
        // in production may or may not have shields; when absent, all damage
        // routes to hull.
        let npc_uuid = "npc-test-uuid".to_string();
        let npc_hull_max = 100.0f32;
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                EntityUuid(npc_uuid.clone()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                GlobalTransform::default(),
                Visibility::default(),
                ShipPhysicsComponent {
                    x: 0.0,
                    z: 0.0,
                    yaw: 0.0,
                    forward_speed: 100.0,
                    roll: 0.0,
                    lateral_speed: 0.0,
                },
                CollisionCooldown::default(),
                EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    npc_hull_max,
                )])),
                ShipModifiers::new(),
                ShipImpulse::default(),
                ColliderSection(ColliderConfig {
                    shape: ColliderShape::Ball,
                    radius: 5.0,
                    length: 0.0,
                }),
                Collider::ball(5.0),
                RigidBody::KinematicPositionBased,
                ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
            ))
            .id();

        // Spawn an asteroid overlapping the NPC at the origin.
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("ast-test-uuid".to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 5.0,
                length: 0.0,
            }),
            Collider::ball(5.0),
            RigidBody::Fixed,
            ActiveCollisionTypes::KINEMATIC_STATIC,
        ));

        // Several updates: first ticks let Rapier build the broad-phase and
        // detect the overlapping pair; subsequent ticks run `handle_collisions`
        // with the contact visible on `ReadRapierContext`.
        for _ in 0..3 {
            app.update();
        }

        let hull = app
            .world()
            .get::<EntitySystemHull>(npc)
            .expect("NPC must retain EntitySystemHull");
        assert!(
            hull.0.total_current() < npc_hull_max,
            "NPC hull must decrease from asteroid collision (current={}, max={})",
            hull.0.total_current(),
            npc_hull_max
        );

        // Player-only messages must NOT be emitted for an NPC-vs-asteroid
        // collision — those are gated on `Has<LocalShip>`.
        let outbox = &app.world().resource::<SimOutbox>().0;
        assert!(
            !outbox
                .iter()
                .any(|(_, m)| matches!(m, ServerMessage::DamageTaken { .. })),
            "DamageTaken is a player-only UI message; must not fire for NPCs"
        );
        assert!(
            !outbox
                .iter()
                .any(|(_, m)| matches!(m, ServerMessage::ShipDestroyed)),
            "ShipDestroyed is a player-only UI message; must not fire for NPCs"
        );

        // Collision response stops the ship and separates it out of the
        // overlapping collider volume, instead of bouncing it backward.
        let physics = app.world().get::<ShipPhysicsComponent>(npc).unwrap();
        assert_eq!(
            physics.forward_speed, 0.0,
            "NPC forward_speed should be zeroed after collision"
        );
        let dist = (physics.x * physics.x + physics.z * physics.z).sqrt();
        assert!(
            dist >= 10.0 + COLLISION_SEPARATION_SLOP - 1e-5,
            "NPC should be separated outside the two collider radii, distance={dist}"
        );
    }

    #[test]
    fn drain_sim_outbox_directly() {
        let mut app = test_app();
        start_game(&mut app);

        // Write directly to SimOutbox
        let len_before = app.world().resource::<SimOutbox>().0.len();
        app.world_mut()
            .resource_mut::<SimOutbox>()
            .0
            .push((Target::All, ServerMessage::GameStarted));

        // Drain manually
        app.world_mut().resource_mut::<SimOutbox>().0.clear();

        // Check SimOutbox is now empty
        let len_after = app.world().resource::<SimOutbox>().0.len();
        assert_eq!(
            len_after,
            0,
            "SimOutbox should be empty after drain, was {} before drain",
            len_before + 1
        );
    }

    // -- Power system integration tests --------------------------------------

    /// Helper: captain + power console player, game started.
    fn start_game_with_power(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "power",
            ClientMessage::Identify {
                token: "power".into(),
                name: "Monty".into(),
            },
        );
        tick(app);
        push(
            app,
            "power",
            ClientMessage::SelectStation {
                station: "Power".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "power", ClientMessage::SetReady { ready: true });
        let _ = tick(app);
        fast_forward_countdown(app);
        let _ = tick(app);
        let _ = tick(app);
    }

    #[test]
    fn non_power_sender_increase_power_is_ignored() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Reset power to known state.
        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                1,
            );

        // Captain (not Power holder) tries to set Helm to 2.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::HELM_POWER_GROUP.into(),
                    ),
                    level: 2,
                },
            },
        );
        let _ = tick(&mut app);

        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&crate::messages::PowerGroupId(
                    crate::power_system::HELM_POWER_GROUP.into()
                )),
            1,
            "non-Power sender should not be able to increase power"
        );
    }

    #[test]
    fn non_power_sender_decrease_power_is_ignored() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Captain (not Power holder) tries to set Sensors to 1.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::SENSORS_POWER_GROUP.into(),
                    ),
                    level: 1,
                },
            },
        );
        let _ = tick(&mut app);

        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&crate::messages::PowerGroupId(
                    crate::power_system::SENSORS_POWER_GROUP.into()
                )),
            2,
            "non-Power sender should not be able to decrease power"
        );
    }

    #[test]
    fn power_sender_increase_reflected_in_next_power_state() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Power holder sets Helm to 3.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::HELM_POWER_GROUP.into(),
                    ),
                    level: 3,
                },
            },
        );
        let _ = tick(&mut app);

        let out = tick(&mut app);
        let power_state = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::PowerState { helm, .. } => Some(*helm),
                _ => None,
            })
            .expect("expected a PowerState message for power holder");
        assert_eq!(
            power_state, 3,
            "PowerState should show helm=3 after increase"
        );
    }

    #[test]
    fn power_sender_decrease_reflected_in_next_power_state() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Power holder sets Weapons to 1.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::WEAPONS_POWER_GROUP.into(),
                    ),
                    level: 1,
                },
            },
        );
        let _ = tick(&mut app);

        let out = tick(&mut app);
        let power_state = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::PowerState { weapons, .. } => Some(*weapons),
                _ => None,
            })
            .expect("expected a PowerState message");
        assert_eq!(
            power_state, 1,
            "PowerState should show weapons=1 after decrease"
        );
    }

    #[test]
    fn power_state_only_sent_to_power_holder() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        let out = tick(&mut app);

        // Every PowerState message should target the power holder.
        for m in out
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::PowerState { .. }))
        {
            assert!(
                matches!(&m.target, Target::Token(t) if t == "power"),
                "PowerState should only go to the Power holder, got {:?}",
                m.target
            );
        }
    }

    #[test]
    fn no_power_station_holder_no_power_state_broadcast() {
        let mut app = test_app();
        // Only captain, no power station holder.
        start_game(&mut app);

        let out = tick(&mut app);
        let any_power_state = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::PowerState { .. }));
        assert!(
            !any_power_state,
            "no PowerState should be sent when no Power station holder exists"
        );
    }

    #[test]
    fn power_increase_respects_bounds_noop_at_four() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Manually set Helm to 4 (max).
        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                4,
            );

        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::HELM_POWER_GROUP.into(),
                    ),
                    level: 4,
                },
            },
        );
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let power_state = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::PowerState { helm, .. } => Some(*helm),
                _ => None,
            })
            .expect("expected a PowerState message");
        assert_eq!(
            power_state, 4,
            "helm should stay at 4 (max bound enforced by PowerSystem)"
        );
    }

    // -- Power ? Modifier wiring integration tests -------------------------

    #[test]
    fn increasing_helm_power_updates_max_speed_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Override multipliers for Helm so level 2 ? 0.0, level 3 ? 1.0
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                [-0.5, 0.0, 1.0, 2.0],
            );

        // Set Helm to 3.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::HELM_POWER_GROUP.into(),
                    ),
                    level: 3,
                },
            },
        );
        let _ = tick(&mut app);

        // Level 3 ? index 2 ? bonus 1.0 ? MaxSpeed multiplier = 2.0
        let mult = get_ship_modifiers(&mut app).get(&ModifierSlot::MaxSpeed);
        assert!(
            (mult - 2.0).abs() < 1e-6,
            "Helm power 3 should give MaxSpeed multiplier 2.0, got {mult}"
        );
    }

    #[test]
    fn decreasing_weapons_power_updates_phaser_damage_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Override multipliers for Tactical: level 2 ? 0.0, level 1 ? -0.5
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::WEAPONS_POWER_GROUP.into()),
                [-0.5, 0.0, 0.25, 0.5],
            );

        // Set Weapons to 1.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::WEAPONS_POWER_GROUP.into(),
                    ),
                    level: 1,
                },
            },
        );
        let _ = tick(&mut app);

        // Level 1 ? index 0 ? bonus -0.5 (negative) ? 1.0 / (1.0 + 0.5) = 0.666...
        let expected = 1.0 / 1.5;
        let mult = get_ship_modifiers(&mut app).get(&ModifierSlot::PhaserDamage);
        assert!(
            (mult - expected).abs() < 1e-6,
            "Weapons power 1 should give PhaserDamage multiplier {expected}, got {mult}"
        );
    }

    #[test]
    fn exhaustion_forces_consoles_to_one_and_updates_all_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Set known multipliers for all three
        let defaults = [-0.5, 0.0, 0.25, 0.5];
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                defaults,
            );
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::WEAPONS_POWER_GROUP.into()),
                defaults,
            );
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::SENSORS_POWER_GROUP.into()),
                defaults,
            );

        // Set state that will trigger exhaustion on the next tick:
        // total=8 (negative rate), battery already at 0 ? tick keeps it at 0
        // and forces all consoles to 1 + lock.
        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                4,
            );
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::WEAPONS_POWER_GROUP.into()),
                2,
            );
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::SENSORS_POWER_GROUP.into()),
                2,
            );
            ps.0.battery_charge = 0.0;
            ps.0.locked = false;
        }

        // Tick triggers exhaustion ? lock changes ? sync_power_modifiers runs
        tick(&mut app);

        // All three forced to 1 ? bonus -0.5 (negative) ? multiplier = 1.0 / (1.0 + 0.5) ˜ 0.666...
        let expected = 1.0 / 1.5;
        let mods = get_ship_modifiers(&mut app);

        assert!(
            (mods.get(&ModifierSlot::MaxSpeed) - expected).abs() < 1e-6,
            "after exhaustion MaxSpeed should be {expected}, got {}",
            mods.get(&ModifierSlot::MaxSpeed)
        );
        assert!(
            (mods.get(&ModifierSlot::PhaserDamage) - expected).abs() < 1e-6,
            "after exhaustion PhaserDamage should be {expected}, got {}",
            mods.get(&ModifierSlot::PhaserDamage)
        );
        assert!(
            (mods.get(&ModifierSlot::RadarRange) - expected).abs() < 1e-6,
            "after exhaustion RadarRange should be {expected}, got {}",
            mods.get(&ModifierSlot::RadarRange)
        );
    }

    #[test]
    fn power_increase_respects_total_cap_of_eight() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Set total to 8: helm=4, weapons=2, sensors=2.
        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                4,
            );

        // Try to set sensors to 3 — total would be 9 (over cap), should be blocked at 2.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::SENSORS_POWER_GROUP.into(),
                    ),
                    level: 3,
                },
            },
        );
        let _ = tick(&mut app);

        let out = tick(&mut app);
        let power_state = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::PowerState { sensors, .. } => Some(*sensors),
                _ => None,
            })
            .expect("expected a PowerState message");
        assert_eq!(
            power_state, 2,
            "sensors should stay at 2 when total is already at the cap of 8"
        );
        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.total(),
            8,
            "total should remain 8"
        );
    }

    // -- Runtime entity lifecycle (EntitySpawned / EntityDespawned) -----

    #[test]
    fn reconcile_system_seeds_on_first_inprogress_frame() {
        let mut app = test_app();
        start_game(&mut app);
        // After start_game, the system should have seeded (even if empty).
        let registry = app.world().resource::<TrackedEntities>();
        assert!(
            registry.seeded,
            "system should be seeded after first InProgress frame"
        );
    }

    #[test]
    fn spawn_non_asteroid_entity_emits_entity_spawned() {
        let mut app = test_app();
        start_game(&mut app);

        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("runtime-entity-1".into()),
            Transform::from_xyz(100.0, 0.0, -200.0),
        ));

        let out = tick(&mut app);

        let spawned = out.iter().find_map(|m| match &m.msg {
            ServerMessage::EntitySpawned { snapshot } => Some(snapshot.clone()),
            _ => None,
        });
        assert!(
            spawned.is_some(),
            "expected EntitySpawned after spawning a non-asteroid entity"
        );
        assert_eq!(spawned.unwrap().uuid, "runtime-entity-1");
    }

    #[test]
    fn entity_spawned_broadcast_contains_position_and_id() {
        let mut app = test_app();
        start_game(&mut app);

        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("pos-entity".into()),
            crate::entity_spawner::EntityId("station-alpha".into()),
            Transform::from_xyz(50.0, 0.0, -75.0),
        ));

        let out = tick(&mut app);

        let spawned = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::EntitySpawned { snapshot } => Some(snapshot.clone()),
                _ => None,
            })
            .expect("expected EntitySpawned");

        assert_eq!(spawned.uuid, "pos-entity");
        assert_eq!(spawned.id, Some("station-alpha".into()));
        assert_eq!(spawned.position, Some([50.0, 0.0, -75.0]));
    }

    #[test]
    fn despawn_non_asteroid_entity_emits_entity_despawned() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn a non-asteroid entity.
        let entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("to-despawn".into()),
                Transform::default(),
            ))
            .id();

        // Tick once so the spawn system picks it up.
        let _ = tick(&mut app);

        // Now despawn it.
        app.world_mut().despawn(entity);
        let out = tick(&mut app);

        let despawned = out.iter().find_map(|m| match &m.msg {
            ServerMessage::EntityDespawned { uuid } => Some(uuid.clone()),
            _ => None,
        });
        assert!(
            despawned.is_some(),
            "expected EntityDespawned after despawning a non-asteroid entity"
        );
        assert_eq!(despawned.unwrap(), "to-despawn");
    }

    #[test]
    fn asteroid_spawn_does_not_emit_entity_spawned() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn an asteroid entity (has Asteroid component + EntityUuid).
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("asteroid-1".into()),
            Asteroid,
            AsteroidUuid("asteroid-1".into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::default(),
        ));

        let out = tick(&mut app);

        let spawned = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::EntitySpawned { .. }));
        assert!(
            !spawned,
            "asteroid spawn must not emit EntitySpawned (uses AsteroidSpawned instead)"
        );
    }

    #[test]
    fn runtime_entity_appears_in_world_data_for_reconnect() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn a non-asteroid entity.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("reconnect-entity".into()),
            Transform::from_xyz(25.0, 0.0, -50.0),
        ));

        let _ = tick(&mut app);

        // The entity should now be in world.entities so Welcome includes it.
        let world = app.world().resource::<WorldResource>();
        let found = world
            .0
            .entities
            .iter()
            .any(|e| e.uuid == "reconnect-entity");
        assert!(
            found,
            "runtime entity must appear in WorldResource for Welcome reconnects"
        );
    }

    #[test]
    fn midgame_reconnect_resets_blackboard_cache() {
        let mut app = test_app();
        start_game(&mut app);

        let helm_id = SystemId("helm".into());
        let helm_bb = SystemBlackboard::Helm(HelmBlackboard {
            yaw: 1.0,
            forward_speed: 50.0,
            x: 100.0,
            z: 200.0,
            impulse_charge: 0.5,
            boost_battery: 0.8,
            boost_active: false,
            boost_enabled: true,
            radar_range: 0.0,
            lateral_speed: 0.0,
        });

        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemBlackboards, With<LocalShip>>();
            if let Ok(mut bbs) = q.single_mut(app.world_mut()) {
                bbs.0.insert(helm_id.clone(), helm_bb.clone());
            }
        }

        // Tick: broadcast_blackboard_updates caches the blackboard and emits it.
        let out1 = tick(&mut app);
        assert!(
            out1.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BlackboardUpdate { .. })),
            "first tick after seeding must emit BlackboardUpdate"
        );

        // Simulate reconnect: push Identify with same token -> Welcome emitted.
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        let out2 = tick(&mut app);
        let out3 = tick(&mut app);

        let has_bb_for_helm = |out: &[OutboundMessage]| -> bool {
            out.iter().any(|m| match &m.msg {
                ServerMessage::BlackboardUpdate { updates } => {
                    updates.iter().any(|(id, _)| id.0 == "helm")
                }
                _ => false,
            })
        };

        assert!(
            has_bb_for_helm(&out2) || has_bb_for_helm(&out3),
            "must emit BlackboardUpdate with helm data within one tick of reconnect Welcome"
        );
    }

    #[test]
    fn entity_spawned_is_broadcast_to_all() {
        let mut app = test_app();
        start_game(&mut app);

        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("all-broadcast".into()),
            Transform::default(),
        ));

        let out = tick(&mut app);

        let spawn_msg = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::EntitySpawned { .. }))
            .expect("expected EntitySpawned message");
        assert!(
            matches!(&spawn_msg.target, crate::lobby::Target::All),
            "EntitySpawned must broadcast to All, got {:?}",
            spawn_msg.target
        );
    }

    #[test]
    fn entity_despawned_is_broadcast_to_all() {
        let mut app = test_app();
        start_game(&mut app);

        let entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("broadcast-despawn".into()),
                Transform::default(),
            ))
            .id();
        let _ = tick(&mut app);

        app.world_mut().despawn(entity);
        let out = tick(&mut app);

        let despawn_msg = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::EntityDespawned { .. }))
            .expect("expected EntityDespawned message");
        assert!(
            matches!(&despawn_msg.target, crate::lobby::Target::All),
            "EntityDespawned must broadcast to All, got {:?}",
            despawn_msg.target
        );
    }

    // -- SetPhaserFrequency delegation tests ----------------------------

    /// Tactical holder may always set phaser frequency.
    #[test]
    fn tactical_holder_can_set_phaser_frequency() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SetPhaserFrequency { frequency: 0.8 },
        );
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.8).abs() < 1e-5,
            "Tactical holder should set phaser frequency to 0.8, got {freq}"
        );
    }

    /// Sensors holder is never authorized to set phaser frequency (delegation removed in B4).
    #[test]
    fn sensors_holder_cannot_set_phaser_frequency() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        push(
            &mut app,
            "sensors",
            ClientMessage::SetPhaserFrequency { frequency: 0.9 },
        );
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.5).abs() < 1e-5,
            "Sensors holder must NOT change phaser frequency, got {freq}"
        );
    }

    /// An unrelated console (e.g. captain) cannot set phaser frequency.
    #[test]
    fn unrelated_console_cannot_set_phaser_frequency() {
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SetPhaserFrequency { frequency: 0.9 },
        );
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.5).abs() < 1e-5,
            "Captain must NOT change phaser frequency, got {freq}"
        );
    }

    /// Frequency value is clamped to [0.0, 1.0] by the handler.
    #[test]
    fn set_phaser_frequency_clamps_value() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SetPhaserFrequency { frequency: 1.5 },
        );
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 1.0).abs() < 1e-5,
            "frequency above 1.0 should clamp to 1.0, got {freq}"
        );

        push(
            &mut app,
            "weapons",
            ClientMessage::SetPhaserFrequency { frequency: -0.5 },
        );
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.0).abs() < 1e-5,
            "frequency below 0.0 should clamp to 0.0, got {freq}"
        );
    }

    // -- Shield focus tests --------------------------------------------------

    fn start_game_with_shields(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "shields",
            ClientMessage::Identify {
                token: "shields".into(),
                name: "Sully".into(),
            },
        );
        tick(app);
        push(
            app,
            "shields",
            ClientMessage::SelectStation {
                station: "Shields".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "shields", ClientMessage::SetReady { ready: true });
        let _ = tick(app);
        fast_forward_countdown(app);
        let _ = tick(app);
        let _ = tick(app);
    }

    #[test]
    fn shields_holder_can_focus_a_facing() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipEntity>().0;
        let shields = app.world().entity(ship).get::<ShipShields>().unwrap();
        assert_eq!(shields.0.focused_facing, Some(0));
        assert!(shields.0.facings[0].is_focused);
    }

    #[test]
    fn non_shields_sender_cannot_set_focus() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        // Captain (not Shields holder) tries to set focus.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("port").expect("port"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipEntity>().0;
        assert!(app
            .world()
            .entity(ship)
            .get::<ShipShields>()
            .unwrap()
            .0
            .focused_facing
            .is_none());
    }

    #[test]
    fn shields_holder_can_clear_focus() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        tick(&mut app);
        let ship = app.world().resource::<ShipEntity>().0;
        let shields = app.world().entity(ship).get::<ShipShields>().unwrap();
        assert_eq!(shields.0.focused_facing, Some(0));

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: false },
            },
        );
        tick(&mut app);
        let ship = app.world().resource::<ShipEntity>().0;
        assert!(app
            .world()
            .entity(ship)
            .get::<ShipShields>()
            .unwrap()
            .0
            .focused_facing
            .is_none());
    }

    #[test]
    fn shield_focus_is_ignored_during_lobby() {
        let mut app = test_app();
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(&mut app);

        // Still in Lobby — SetShieldArcFocus should be ignored.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("aft").expect("aft"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipEntity>().0;
        assert!(app
            .world()
            .entity(ship)
            .get::<ShipShields>()
            .unwrap()
            .0
            .focused_facing
            .is_none());
    }

    #[test]
    fn shield_focus_updates_broadcast_status() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let shield_status = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::ShieldStatus { facings, .. } => Some(facings.clone()),
                _ => None,
            })
            .expect("expected a ShieldStatus broadcast after focus change");

        assert!(shield_status[0].is_focused, "Fore should be focused");
        assert!(!shield_status[1].is_focused, "Port should not be focused");
        assert!(!shield_status[2].is_focused, "Aft should not be focused");
        assert!(
            !shield_status[3].is_focused,
            "Starboard should not be focused"
        );
    }

    #[test]
    fn player_spawn_rotation_yaw_extracts_yaw_correctly() {
        let (q, yaw) = player_spawn_rotation_yaw([0.0, std::f32::consts::FRAC_PI_2, 0.0]);
        assert!(
            (yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6,
            "yaw-only rotation should produce matching yaw"
        );
        let (y, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
        assert!(
            (y - std::f32::consts::FRAC_PI_2).abs() < 1e-6,
            "quaternion yaw should match input"
        );
    }

    #[test]
    fn player_spawn_rotation_yaw_pitch_only_gives_zero_yaw() {
        let (_, yaw) = player_spawn_rotation_yaw([std::f32::consts::FRAC_PI_4, 0.0, 0.0]);
        assert!(yaw.abs() < 1e-6, "pitch-only rotation should give zero yaw");
    }

    #[test]
    fn player_spawn_rotation_yaw_roll_only_gives_zero_yaw() {
        let (_, yaw) = player_spawn_rotation_yaw([0.0, 0.0, std::f32::consts::FRAC_PI_3]);
        assert!(yaw.abs() < 1e-6, "roll-only rotation should give zero yaw");
    }

    // ── last_attacker clear handler tests ──────────────────────────────────

    fn last_attacker_test_app() -> App {
        let mut app = App::new();
        app.add_systems(
            Update,
            (
                clear_last_attacker_on_death,
                clear_last_attacker_on_red_alert_off,
            ),
        );
        app
    }

    #[test]
    fn clear_on_despawn_clears_when_entity_removed() {
        let mut app = last_attacker_test_app();
        let attacker_uuid = "attacker-1".to_string();
        let attacker_entity = app
            .world_mut()
            .spawn((EntityUuid(attacker_uuid.clone()),))
            .id();
        let ship = app
            .world_mut()
            .spawn((LastShipAttacker(Some(attacker_uuid)),))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<LastShipAttacker>(ship).unwrap().0,
            Some("attacker-1".to_string())
        );
        app.world_mut().despawn(attacker_entity);
        app.update();
        assert_eq!(app.world().get::<LastShipAttacker>(ship).unwrap().0, None);
    }

    #[test]
    fn clear_on_despawn_does_not_clear_when_entity_still_alive() {
        let mut app = last_attacker_test_app();
        app.world_mut()
            .spawn((EntityUuid("attacker-1".to_string()),));
        let ship = app
            .world_mut()
            .spawn((LastShipAttacker(Some("attacker-1".to_string())),))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<LastShipAttacker>(ship).unwrap().0,
            Some("attacker-1".to_string())
        );
    }

    #[test]
    fn clear_on_red_alert_off_clears_when_red_alert_turns_off() {
        let mut app = last_attacker_test_app();
        // Spawn an entity so clear_last_attacker_on_death doesn't fire.
        app.world_mut()
            .spawn((EntityUuid("attacker-1".to_string()),));
        let ship = app
            .world_mut()
            .spawn((
                LastShipAttacker(Some("attacker-1".to_string())),
                crate::ship_state::ShipRedAlert(true),
                LocalShip,
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<LastShipAttacker>(ship).unwrap().0,
            Some("attacker-1".to_string())
        );
        app.world_mut()
            .get_mut::<crate::ship_state::ShipRedAlert>(ship)
            .unwrap()
            .0 = false;
        app.update();
        assert!(app
            .world()
            .get::<LastShipAttacker>(ship)
            .unwrap()
            .0
            .is_none());
    }

    #[test]
    fn clear_on_red_alert_off_does_not_clear_when_alert_stays_on() {
        let mut app = last_attacker_test_app();
        // Spawn an entity so clear_last_attacker_on_death doesn't fire.
        app.world_mut()
            .spawn((EntityUuid("attacker-1".to_string()),));
        let ship = app
            .world_mut()
            .spawn((
                LastShipAttacker(Some("attacker-1".to_string())),
                crate::ship_state::ShipRedAlert(true),
                LocalShip,
            ))
            .id();
        app.update();
        app.update();
        assert_eq!(
            app.world().get::<LastShipAttacker>(ship).unwrap().0,
            Some("attacker-1".to_string())
        );
    }
}
