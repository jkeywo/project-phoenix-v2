use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::damage::ConsoleHull;
use crate::lobby::{InboundMessage, LobbyOutbox, OutboundMessage, Sessions, Target, WorldResource};
use crate::messages::{
    ClientMessage, Console, EntitySnapshot, GamePhase, ServerMessage, ShieldFacingStatus,
    SystemControlPayload,
};
use crate::shield::ShieldSystem;
use rand::SeedableRng as _;

use crate::damage::{apply_damage_with_shields, apply_hull_damage, collision_damage};
use crate::debug_overlay::{DamageLog, DamageLogEntry};
use crate::shield::attacker_bearing_relative;
use crate::ship_state::ShipState;
use bevy_rapier3d::prelude::ReadRapierContext;

use crate::entity_spawner::{
    AsteroidFieldSection, BehaviourSection, EntityId, EntityName, EntityTagsSection, EntityUuid,
    FactionComponent, MeshSection, RadarAppearanceSection, RegionShapeSection,
};
use crate::impulse::ImpulseState;
use crate::messages::ModifierSlot;
use crate::modifiers::ShipModifiers;
use crate::world::server::ObjectiveManagerRes;
use std::collections::HashMap;

// â"€â"€ Beam constants â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
pub use crate::weapons_plugin::{
    weapons_update_broadcaster, ActiveBeam, AsteroidDestroyedVfx, CurrentPhaserMode,
    LastWeaponsUpdate, PhaserCooldown, PhaserRenderConfig, TorpedoSystemResource, WeaponsTarget,
};

pub use crate::repair_plugin::{repair_state_broadcaster, ShipRepairTeams};

pub use crate::power_plugin::{
    power_state_broadcaster, PowerAiConfigResource, PowerConfigResource, PowerMultiplierResource,
    ShipPowerSystem,
};

// â"€â"€ Marker Components â"€â"€â"€â"€â"€â"€â"€â"€
#[derive(Component)]
pub struct Ship;

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

/// Per-console hull tracker for the player ship. Tracked as a Bevy resource so
/// systems can read/write it independently of `ShipState`.
#[derive(Resource)]
pub struct ShipHullIntegrity(pub ConsoleHull);

/// The ship's shield system. Damage from collisions is routed through shields
/// first; only overflow passes through to the hull.
#[derive(Resource)]
pub struct ShipShields(pub ShieldSystem);

/// The ship's impulse drive state. Cancelled automatically when hull damage is taken.
#[derive(Resource)]
pub struct ShipImpulse(pub ImpulseState);

/// The ship's boost drive battery state. Toggle/partial-drain model; only
/// active when the ship's TOML enables it (see `BoostConfigResource`).
#[derive(Resource, Default)]
pub struct ShipBoost(pub crate::boost::BoostState);

/// Set to `true` by phaser/torpedo fire systems when a weapon actually fires
/// this tick. Reset to `false` at the start of each tick by `update_combat_activity`.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WeaponFiredThisTick(pub bool);

/// Carries the reason string when the game ends. Set to `Some(reason)` before
/// transitioning to `GamePhase::GameOver`. The `OnEnter(GameOver)` system reads
/// this resource and broadcasts the reason to all clients.
#[derive(Resource, Default)]
pub struct GameOverReason(pub Option<String>);

/// Prevents `handle_collisions` from applying damage every frame while the
/// ship is in contact. After damage is applied once, a 1-second cooldown
/// suppresses further hits until the ship clears the obstacle.
#[derive(Resource, Default)]
struct CollisionCooldown {
    remaining_secs: f32,
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

/// Last-broadcast positions for non-asteroid entities (NPCs, stations).
/// Keyed by UUID string; value is (translation, yaw). Used by the
/// sim_state_broadcaster to skip sending position/yaw when unchanged.
#[derive(Resource, Default)]
pub struct LastBroadcastEntityPositions(
    pub std::collections::HashMap<String, (bevy::math::Vec3, f32)>,
);

/// Last-broadcast per-console hull state. When the hull changes, a
/// `ConsoleHullUpdate` event message is emitted and this cache is updated.
#[derive(Resource, Default)]
pub struct LastBroadcastHull(pub Vec<crate::messages::ConsoleHullStatus>);

/// Last-broadcast shield facings. Used to suppress the per-tick `ShieldStatus`
/// broadcast to all players when nothing has changed.
#[derive(Resource, Default)]
pub struct LastBroadcastShields(pub Vec<crate::messages::ShieldFacingStatus>);

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

/// Current-tick per-system blackboards. Each system writes its own entry
/// during `SimSet::Publish`. Snapshotted into `FrozenBlackboards` before
/// the next tick's `SimSet::Input` so cross-system reads in Physics/Damage/
/// Modifiers always see last tick's values (deterministic, one-tick lag).
#[derive(Resource, Default, Clone)]
pub struct SystemBlackboards(
    pub std::collections::HashMap<crate::messages::SystemId, crate::messages::SystemBlackboard>,
);

/// Frozen snapshot of the previous tick's `SystemBlackboards`. Updated by
/// `snapshot_blackboards` which runs before `SimSet::Input`. Cross-system
/// readers in Physics/Damage/Modifiers should read this, never the live
/// `SystemBlackboards`, to guarantee determinism.
#[derive(Resource, Default, Clone)]
pub struct FrozenBlackboards(
    pub std::collections::HashMap<crate::messages::SystemId, crate::messages::SystemBlackboard>,
);

/// Last-broadcast blackboard state per system. The `broadcast_blackboard_updates`
/// system compares `SystemBlackboards` against this and only emits a
/// `BlackboardUpdate` for systems whose blackboard has changed.
#[derive(Resource, Default)]
pub struct LastBroadcastBlackboards(
    pub std::collections::HashMap<crate::messages::SystemId, crate::messages::SystemBlackboard>,
);

// â"€â"€ Plugin â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
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
    .insert_resource(ShipState::new())
    .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
        (Console::Helm, 25.0),
        (Console::Tactical, 25.0),
        (Console::Power, 25.0),
        (Console::Shields, 25.0),
        (Console::Core, 50.0),
    ])))
    .insert_resource(ShipShields(ShieldSystem::default()))
    .insert_resource(ShipImpulse(ImpulseState::new()))
    .insert_resource(ShipBoost(crate::boost::BoostState::new()))
    .init_resource::<WeaponFiredThisTick>()
    .insert_resource(crate::config_cache::FactionRegistryResource(
        crate::config_cache::get_faction_registry(),
    ))
    .init_resource::<WorldResource>()
    .init_resource::<WorldSetupBroadcast>()
    .init_resource::<CollisionCooldown>()
    .init_resource::<TrackedEntities>()
    .init_resource::<SimOutbox>()
    .init_resource::<LastBroadcastEntityPositions>()
    .init_resource::<LastBroadcastHull>()
    .init_resource::<LastBroadcastShields>()
    .init_resource::<SystemBlackboards>()
    .init_resource::<FrozenBlackboards>()
    .init_resource::<LastBroadcastBlackboards>()
    .init_resource::<crate::messages::AdmittedCommands>()
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
            spawn_game_start_entities,
            dump_tracked_entities,
        )
            .chain(),
    )
    .add_systems(Update, render_spawned_entities)
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
        snapshot_blackboards
            .after(crate::lobby::process_lobby)
            .before(crate::sim_sets::SimSet::Input)
            .run_if(in_state(GamePhase::InProgress)),
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
        broadcast_blackboard_updates.in_set(crate::sim_sets::SimSet::Broadcast),
    )
    .add_systems(
        Update,
        refresh_caches_on_midgame_reconnect
            .after(crate::lobby::process_lobby)
            .before(crate::sim_sets::SimSet::Broadcast),
    )
    .add_systems(
        Update,
        (
            handle_set_sensors_target.in_set(crate::sim_sets::SimSet::Input),
            tick_shields.in_set(crate::sim_sets::SimSet::Physics),
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
        publish_viewscreen_blackboard.in_set(crate::sim_sets::SimSet::PublishAggregate),
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
        let asteroid_states: Vec<crate::messages::EntityStateSnapshot> = {
            let mut q = world.query::<(
                &AsteroidUuid,
                Option<&crate::entity_spawner::EntityConsoleHull>,
                Option<&crate::entity_spawner::EntityShield>,
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
                    let shield_fraction = shield_comp.map(|s| s.fraction());
                    // Omit entry entirely when there is nothing to update.
                    if hull_fraction.is_none() && shield_fraction.is_none() {
                        return None;
                    }
                    Some(crate::messages::EntityStateSnapshot {
                        uuid: uuid.0.clone(),
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
        // we can drop the ECS borrow before mutating the LastBroadcastEntityPositions
        // resource.
        type NpcRaw = (String, bevy::math::Vec3, f32, Option<f32>, Option<f32>);
        let npc_raw: Vec<NpcRaw> = {
            let mut q = world.query_filtered::<(
                &Transform,
                &EntityUuid,
                Option<&crate::entity_spawner::EntityConsoleHull>,
                Option<&crate::entity_spawner::EntityShield>,
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
                    let shield_fraction = shield_comp.map(|s| s.fraction());
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

        // Compare against last-broadcast positions; only include position/yaw
        // when the entity actually moved (delta > ~1 cm).
        const POS_THRESHOLD_SQ: f32 = 0.0001; // 0.01 world-unit radius
        const YAW_THRESHOLD: f32 = 0.001; // ~0.057 degrees
        let npc_states: Vec<crate::messages::EntityStateSnapshot> = {
            let mut last = world.resource_mut::<LastBroadcastEntityPositions>();
            npc_raw
                .into_iter()
                .map(|(uuid, pos, yaw, hull_fraction, shield_fraction)| {
                    let moved = match last.0.get(&uuid) {
                        Some(&(prev_pos, prev_yaw)) => {
                            (pos - prev_pos).length_squared() > POS_THRESHOLD_SQ
                                || (yaw - prev_yaw).abs() > YAW_THRESHOLD
                        }
                        None => true,
                    };
                    if moved {
                        last.0.insert(uuid.clone(), (pos, yaw));
                    }
                    crate::messages::EntityStateSnapshot {
                        uuid,
                        position: if moved {
                            Some([pos.x, pos.y, pos.z])
                        } else {
                            None
                        },
                        yaw: if moved { Some(yaw) } else { None },
                        hull_fraction,
                        shield_fraction,
                        flags: vec![],
                        shields: None,
                        warp_out_remaining_secs: None,
                    }
                })
                .collect()
        };

        let entity_states: Vec<_> = asteroid_states.into_iter().chain(npc_states).collect();

        // ── Emit ConsoleHullUpdate only when hull HP changed.
        {
            let hull = world.resource::<ShipHullIntegrity>();
            let current_hull: Vec<crate::messages::ConsoleHullStatus> = hull
                .0
                .entries()
                .iter()
                .map(|(c, cur, max)| crate::messages::ConsoleHullStatus {
                    console: c.clone(),
                    current: *cur,
                    max_hp: *max,
                })
                .collect();
            let hull_changed = world.resource::<LastBroadcastHull>().0 != current_hull;
            if hull_changed {
                let entries = current_hull.clone();
                world.resource_mut::<LastBroadcastHull>().0 = current_hull;
                world
                    .resource_mut::<SimOutbox>()
                    .0
                    .push((Target::All, ServerMessage::ConsoleHullUpdate { entries }));
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
pub fn modifier_events_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::OnEvent, |world: &mut World| {
        use crate::modifiers::ModifierEvent;
        let events: Vec<_> = {
            let mut modifiers = world.resource_mut::<crate::modifiers::ShipModifiers>();
            std::mem::take(&mut modifiers.pending_events)
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
            world.write_message(OutboundMessage { target, msg });
        }
        vec![]
    })
}

// -- Systems -------------------------------------------------------------------

fn publish_viewscreen_blackboard(
    ship: Option<Res<ShipState>>,
    hull: Option<Res<ShipHullIntegrity>>,
    activity: Option<Res<crate::ship::combat_activity::RecentCombatActivity>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    mut blackboards: ResMut<SystemBlackboards>,
) {
    use crate::messages::{SystemBlackboard, SystemId, ViewscreenBlackboard};
    use crate::objectives::WorldConditions;
    use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

    let red_alert = ship.as_ref().map(|s| s.red_alert()).unwrap_or(false);
    let hull_integrity_pct = if let Some(h) = &hull {
        let max = h.0.total_max();
        let cur = h.0.total_current();
        if max > 0.0 { (cur / max * 100.0).clamp(0.0, 100.0) } else { 100.0 }
    } else {
        100.0
    };
    let last_damage_taken_secs = activity.as_ref().and_then(|a| a.last_damage_taken);
    let last_weapon_fired_secs = activity.as_ref().and_then(|a| a.last_weapon_fired);

    let conditions = WorldConditions {
        red_alert,
        hull_fraction: hull_integrity_pct / 100.0,
    };
    let scored_objectives = objectives
        .as_ref()
        .map(|o| o.0.scored_pool(&conditions))
        .unwrap_or_default();

    let bb = ViewscreenBlackboard {
        red_alert,
        hull_integrity_pct,
        last_damage_taken_secs,
        last_weapon_fired_secs,
        scored_objectives,
    };

    blackboards.0.insert(
        SystemId(VIEWSCREEN_SYSTEM_ID.to_string()),
        SystemBlackboard::Viewscreen(bb),
    );
}

fn handle_set_sensors_target(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<&crate::ship_plugin::ShipConfigComponent, With<Ship>>,
    mut outbox: ResMut<SimOutbox>,
) {
    let Ok(ship_config) = ship_query.single() else { return; };
    for ev in reader.read() {
        let ClientMessage::ControlSystem { target, payload } = &ev.msg else {
            continue;
        };
        if target.0 != crate::system_registry::SENSORS_SYSTEM_ID {
            continue;
        }
        let SystemControlPayload::SetScienceTarget { uuid } = payload else {
            continue;
        };

        // Only the Sensors console holder may broadcast a target suggestion.
        if sessions.0.console_holder(&Console::Sensors, &ship_config.0) != Some(ev.token.as_str()) {
            continue;
        }

        // Only broadcast if there is a Tactical console player to receive it.
        let Some(tactical_token) = sessions
            .0
            .console_holder(&Console::Tactical, &ship_config.0)
        else {
            continue;
        };

        outbox.0.push((
            Target::Token(tactical_token.to_string()),
            ServerMessage::SensorsTargetSuggestion { uuid: uuid.clone() },
        ));
    }
}

fn handle_collisions(
    time: Res<Time>,
    context: ReadRapierContext,
    ship_query: Query<Entity, With<Ship>>,
    asteroid_query: Query<
        (&Transform, &AsteroidUuid, Option<&AsteroidShieldPierce>),
        With<Asteroid>,
    >,
    mut ship: ResMut<ShipState>,
    mut impulse: ResMut<ShipImpulse>,
    mut hull: ResMut<ShipHullIntegrity>,
    mut shields: ResMut<ShipShields>,
    mut cooldown: ResMut<CollisionCooldown>,
    modifiers: Res<ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut game_over_reason: ResMut<GameOverReason>,
    mut damage_log: ResMut<DamageLog>,
) {
    let dt = time.delta_secs();
    cooldown.remaining_secs = (cooldown.remaining_secs - dt).max(0.0);

    let Ok(ctx) = context.single() else { return };
    let Ok(ship_entity) = ship_query.single() else {
        return;
    };

    let contact = ctx.contact_pairs_with(ship_entity).next().and_then(|pair| {
        if pair.collider1() == Some(ship_entity) {
            pair.collider2()
        } else {
            pair.collider1()
        }
    });

    if contact.is_some() {
        if cooldown.remaining_secs > 0.0 {
            return;
        }
        impulse.0.cancel_charge();
        let speed_at_impact = ship.forward_speed;
        ship.forward_speed = -0.5 * speed_at_impact;
        let damage = collision_damage(speed_at_impact) as f32
            * modifiers.get(&ModifierSlot::HullDamageTaken);

        let bearing = contact
            .and_then(|attacker_entity| {
                asteroid_query.get(attacker_entity).ok().map(|(t, _, _)| {
                    attacker_bearing_relative(
                        t.translation.x,
                        t.translation.z,
                        ship.x,
                        ship.z,
                        ship.yaw,
                    )
                })
            })
            .unwrap_or(0.0);

        let source_label = contact
            .and_then(|attacker_entity| {
                asteroid_query
                    .get(attacker_entity)
                    .ok()
                    .map(|(_, uuid, _)| format!("asteroid:{}", uuid.0))
            })
            .unwrap_or_else(|| "collision".to_string());

        // Resolve the colliding asteroid's `shield_pierce` (missing → 0.0,
        // matching pre-#414 behaviour where all collision damage was first
        // absorbed by shields).
        let shield_pierce = contact
            .and_then(|attacker_entity| {
                asteroid_query
                    .get(attacker_entity)
                    .ok()
                    .and_then(|(_, _, sp)| sp.map(|c| c.0))
            })
            .unwrap_or(0.0);

        let arc_idx = shields.0.facing_index_for_bearing(bearing);
        let arc_label = shields.0.facings.get(arc_idx).map(|f| f.label.clone());

        damage_log.push(DamageLogEntry {
            source: source_label,
            shield_arc: arc_label,
            amount: damage,
        });

        // Split impact damage by the asteroid's `shield_pierce`: the
        // pierced fraction goes straight to hull; the absorbed fraction
        // is mitigated by the facing shield quadrant (any leak adds to
        // hull damage).
        let (pierced, absorbed) = crate::damage::split_damage_for_pierce(damage, shield_pierce);
        let mut total_hull = pierced;
        let mut shield_amount = 0.0;
        if absorbed > 0.0 {
            let leak = apply_damage_with_shields(absorbed.round() as i32, bearing, &mut shields.0);
            shield_amount = (absorbed - leak as f32).max(0.0);
            total_hull += leak as f32;
        }

        if total_hull > 0.0 {
            let rng = &mut rand::rngs::SmallRng::from_os_rng();
            let (hull_applied, ship_destroyed) = apply_hull_damage(&mut hull.0, total_hull, rng);
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
        } else {
            outbox.0.push((
                Target::All,
                ServerMessage::DamageTaken {
                    hull: 0.0,
                    shield: shield_amount,
                },
            ));
        }
        cooldown.remaining_secs = 1.0;
    }
}
/// Tick shield regen and offline timers each frame.
fn tick_shields(time: Res<Time>, mut shields: ResMut<ShipShields>) {
    shields.0.tick(time.delta_secs());
}

/// Broadcast `ShieldStatus` at 10 Hz.
/// Sends to all players only when shield state changed; always sends to the
/// Shields console holder so their panel stays smooth during regeneration.
fn broadcast_shield_status(
    time: Res<Time>,
    mut timer: ResMut<SimBroadcastTimer>,
    shields: Res<ShipShields>,
    mut outbox: ResMut<SimOutbox>,
    sessions: Res<Sessions>,
    ship_query: Query<&crate::ship_plugin::ShipConfigComponent, With<Ship>>,
    mut last: ResMut<LastBroadcastShields>,
) {
    let Ok(ship_config) = ship_query.single() else { return; };
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
        })
        .collect();

    if facings != last.0 {
        // State changed — broadcast to everyone.
        last.0 = facings.clone();
        outbox
            .0
            .push((Target::All, ServerMessage::ShieldStatus { facings }));
    } else if let Some(token) = sessions.0.console_holder(&Console::Shields, &ship_config.0) {
        // Nothing changed but the Shields holder still gets a periodic refresh
        // so regenerating HP stays smooth on their panel.
        outbox.0.push((
            Target::Token(token.to_string()),
            ServerMessage::ShieldStatus { facings },
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
fn reset_broadcast_caches_on_start(
    mut hull: ResMut<LastBroadcastHull>,
    mut shields: ResMut<LastBroadcastShields>,
    mut positions: ResMut<LastBroadcastEntityPositions>,
    mut weapons: ResMut<LastWeaponsUpdate>,
    mut last_bb: ResMut<LastBroadcastBlackboards>,
) {
    *hull = LastBroadcastHull::default();
    *shields = LastBroadcastShields::default();
    *positions = LastBroadcastEntityPositions::default();
    *weapons = LastWeaponsUpdate::default();
    *last_bb = LastBroadcastBlackboards::default();
}

/// Snapshot `SystemBlackboards` into `FrozenBlackboards` before each tick's
/// `SimSet::Input`. Cross-system reads during Physics/Damage/Modifiers must
/// read `FrozenBlackboards`; they will see last tick's published values
/// (uniform one-tick lag, guaranteed determinism).
fn snapshot_blackboards(
    blackboards: Res<SystemBlackboards>,
    mut frozen: ResMut<FrozenBlackboards>,
) {
    if blackboards.is_changed() {
        frozen.0 = blackboards.0.clone();
    }
}

/// Public alias of `snapshot_blackboards` for unit tests that need to
/// verify the one-tick-lag determinism invariant in isolation.
pub fn snapshot_blackboards_for_test(
    blackboards: Res<SystemBlackboards>,
    frozen: ResMut<FrozenBlackboards>,
) {
    snapshot_blackboards(blackboards, frozen);
}

/// Emit `BlackboardUpdate` for any system whose blackboard has changed since
/// the last broadcast. Runs in `SimSet::Broadcast` so it sees the current
/// tick's fully-published blackboards.
pub fn broadcast_blackboard_updates(
    blackboards: Res<SystemBlackboards>,
    mut last: ResMut<LastBroadcastBlackboards>,
    mut outbox: ResMut<SimOutbox>,
) {
    let updates: Vec<(crate::messages::SystemId, crate::messages::SystemBlackboard)> = blackboards
        .0
        .iter()
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
        app.init_resource::<crate::messages::AdmittedCommands>()
            .init_resource::<crate::messages::InterSystemQueue>()
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
    mut admitted: ResMut<crate::messages::AdmittedCommands>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
        ),
        With<Ship>,
    >,
) {
    admitted.0.clear();
    let Ok((ship_config, control_sources)) = ship_query.single() else {
        return;
    };
    for ev in reader.read() {
        let ClientMessage::ControlSystem { target, payload } = &ev.msg else {
            continue;
        };
        if is_command_authorized(&ev.token, target, payload, control_sources, &sessions, ship_config) {
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

/// Returns the `Console` that controls `target`, used for seat-holder checks.
fn console_for_system(target: &crate::messages::SystemId) -> Option<Console> {
    use crate::system_registry::*;
    match target.0.as_str() {
        CAPTAIN_SYSTEM_ID | RED_ALERT_SYSTEM_ID => Some(Console::CaptainChair),
        HELM_SYSTEM_ID => Some(Console::Helm),
        TACTICAL_SYSTEM_ID => Some(Console::Tactical),
        POWER_SYSTEM_ID => Some(Console::Power),
        SENSORS_SYSTEM_ID => Some(Console::Sensors),
        NAVIGATION_SYSTEM_ID => Some(Console::Navigation),
        SHIELDS_SYSTEM_ID => Some(Console::Shields),
        COMMS_SYSTEM_ID => Some(Console::Comms),
        REPAIR_SYSTEM_ID => Some(Console::Repair),
        _ => None,
    }
}

fn is_command_authorized(
    token: &str,
    target: &crate::messages::SystemId,
    payload: &SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    ship_config: &crate::ship_plugin::ShipConfigComponent,
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

    // Human network token: must hold the console for the target system.
    match console_for_system(&effective_target) {
        Some(console) => sessions.0.console_holder(&console, &ship_config.0) == Some(token),
        None => true, // Unknown system: conservative allow.
    }
}

/// When a player reconnects mid-game (Identify during InProgress), `process_lobby`
/// queues a Welcome into LobbyOutbox. Detect this and clear hull/shields/weapons
/// caches so the next 10 Hz tick sends a full state update to all players
/// (including the reconnecting player).
fn refresh_caches_on_midgame_reconnect(
    lobby_outbox: Res<LobbyOutbox>,
    state: Res<State<GamePhase>>,
    mut hull: ResMut<LastBroadcastHull>,
    mut shields: ResMut<LastBroadcastShields>,
    mut weapons: ResMut<LastWeaponsUpdate>,
) {
    if *state.get() != GamePhase::InProgress {
        return;
    }
    let has_welcome = lobby_outbox
        .0
        .iter()
        .any(|(_, msg)| matches!(msg, ServerMessage::Welcome { .. }));
    if has_welcome {
        *hull = LastBroadcastHull::default();
        *shields = LastBroadcastShields::default();
        *weapons = LastWeaponsUpdate::default();
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
    if config.shields.is_some() {
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
            Option<&crate::entity_spawner::EntityConsoleHull>,
            Option<&crate::entity_spawner::EntityTarget>,
            Option<&crate::entity_spawner::EntityShield>,
        ),
        Without<Asteroid>,
    >,
    mut outbox: ResMut<SimOutbox>,
    objectives: Option<Res<ObjectiveManagerRes>>,
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
                let shield_fraction = shield_comp.map(|s| s.fraction());
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
                let shield_fraction = shield_comp.map(|s| s.fraction());
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
// When no `WorldConfig` is loaded (native unit tests, hardcoded fallback)
// this is a no-op; `world::server::setup_fallback_world` covers that case.
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

/// Spawn entities with `spawn_on = GameStart` (e.g. player ship) when the
/// game transitions to InProgress. Registered in `OnEnter(GamePhase::InProgress)`.
fn spawn_game_start_entities(
    mut commands: Commands,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ship_state: ResMut<crate::ship_state::ShipState>,
    mut pending_ship_config: Option<ResMut<crate::ship_plugin::PendingShipConfig>>,
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

        let spawned = crate::entity_spawner::spawn_entity(
            &mut commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );

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
            commands.entity(spawned)
                .insert(Ship)
                .insert(ship_config)
                .insert(crate::ship_plugin::ShipSystemControlSources::default())
                .insert(crate::ship_plugin::ActiveStationRatings::default())
                .insert(crate::ship_plugin::CoordinationQueue::default());
            ship_spawned = true;

            // Seed authoritative ship position from the world TOML so that
            // `sync_ship_position` (ShipState → Transform) doesn't snap the
            // ship back to (0,0) on the first Physics tick.
            ship_state.x = pos.x;
            ship_state.z = pos.z;

            // Ship-specific resource setup
            if let Some(hc) = &config.hull {
                let entries: Vec<(Console, f32)> = hc
                    .console_hull
                    .iter()
                    .map(|e| (e.console.clone(), e.max_hp))
                    .collect();
                let hull = if entries.is_empty() {
                    // Legacy fallback: single "virtual" console with hull_integrity HP.
                    ConsoleHull::from_config(&[(Console::Helm, hc.hull_integrity)])
                } else {
                    ConsoleHull::from_config(&entries)
                };
                commands.insert_resource(ShipHullIntegrity(hull));
                // [repair] block — overrides default RepairTimings if present.
                // Absent block keeps the same defaults the hardcoded constants
                // used to provide (5.0s travel, 0.5 HP/s repair rate).
                let repair = config.repair.as_ref();
                let team_count = repair
                    .map(|rc| rc.repair_team_count as usize)
                    .filter(|&n| n > 0)
                    .unwrap_or(2);
                let timings = repair.map(|rc| rc.to_runtime()).unwrap_or_default();
                commands.insert_resource(ShipRepairTeams(
                    crate::repair_teams::RepairTeams::new_with_timings(team_count, timings),
                ));
            } else {
                commands.insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
                    (Console::Helm, 25.0),
                    (Console::Tactical, 25.0),
                    (Console::Power, 25.0),
                    (Console::Shields, 25.0),
                ])));
            }

            // Apply shield focus config + base shield-system values from TOML if present.
            // The `[shields_console.base]` sub-block, when present, overrides
            // the four hardcoded defaults in `ShieldConfig::default()`
            // (num_facings, max_hp, regen_per_sec, offline_duration).
            // When absent we keep the historical defaults (4 quadrants,
            // 100 HP, 5 HP/s regen, 10 s offline).
            if let Some(sc) = &config.shields_console {
                let shield_config = sc.base.as_ref().map(|b| b.to_runtime()).unwrap_or_default();
                let mut shields = ShipShields(ShieldSystem::new(&shield_config));
                shields.0.focus_config = crate::shield::ShieldFocusConfig {
                    bonus_max_hp: sc.focus_bonus_max_hp,
                    bonus_regen: sc.focus_bonus_regen,
                    penalty_max_hp: sc.focus_penalty_max_hp,
                    penalty_regen: sc.focus_penalty_regen,
                    decay_rate: sc.focus_decay_rate,
                };
                commands.insert_resource(shields);
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
                commands.insert_resource(PhaserRenderConfig {
                    beam_color,
                    beam_range,
                });

                // Player phaser combat tuning — overrides the default
                // PhaserCombatConfig that WeaponsPlugin installed. The
                // [weapons_console] block already carries `beam_range`,
                // `beam_damage_per_sec`, `beam_duration_secs`, and
                // `cooldown_secs`; before this slice those were only
                // honoured by the NPC phaser path. Now the player path
                // also reads them via the PhaserCombatConfig resource.
                commands.insert_resource(crate::weapons_plugin::PhaserCombatConfigResource(
                    crate::entity_config::PhaserCombatConfig::from_weapons_console(wc),
                ));
            }

            // [torpedoes] block — overrides the default TorpedoSystemResource
            // that `WeaponsPlugin` initialised. Absent block keeps defaults
            // (so NPC ships without `[torpedoes]` are unaffected).
            //
            // TODO: NPC ships that declare their own `[torpedoes]` block will
            // parse it (it lands in `EntityConfig.torpedoes`) but it is
            // silently ignored here because this override lives inside the
            // "first ship entity == player ship" branch. When per-entity NPC
            // torpedo systems land, lift this into `entity_spawner::spawn_entity`
            // as a per-entity component instead of a single shared resource.
            if let Some(tc) = &config.torpedoes {
                let runtime_config = tc.to_runtime();
                let torpedo_system = if !tc.tubes.is_empty() {
                    crate::torpedo::TorpedoSystem::from_configs(&tc.tubes, runtime_config)
                } else {
                    crate::torpedo::TorpedoSystem::new(runtime_config)
                };
                commands
                    .insert_resource(crate::weapons_plugin::TorpedoSystemResource(torpedo_system));
            }

            if let Some(pc) = &config.power {
                commands.insert_resource(PowerConfigResource(crate::power_system::PowerConfig {
                    capacity: pc.capacity,
                    rates: pc.rates,
                    emergency_threshold: pc.emergency_threshold,
                }));
                if let Some(ai) = &pc.ai {
                    commands.insert_resource(PowerAiConfigResource {
                        weapons_battery_floor: ai.weapons_battery_floor,
                        shields_battery_floor: ai.shields_battery_floor,
                        helm_battery_floor: ai.helm_battery_floor,
                        helm_throttle_threshold: ai.helm_throttle_threshold,
                    });
                }
            }

            // Power multipliers
            let defaults = [-0.5, 0.0, 0.25, 0.5];
            let mut multipliers: std::collections::HashMap<Console, [f32; 4]> =
                std::collections::HashMap::from([
                    (Console::Helm, defaults),
                    (Console::Tactical, defaults),
                    (Console::Sensors, defaults),
                ]);
            if let Some(hc) = &config.helm_console {
                if let Some(pm) = hc.power_multipliers {
                    multipliers.insert(Console::Helm, pm);
                }
            }
            if let Some(wc) = &config.weapons_console {
                if let Some(pm) = wc.power_multipliers {
                    multipliers.insert(Console::Tactical, pm);
                }
            }
            if let Some(sc) = &config.sensors_console {
                if let Some(pm) = sc.power_multipliers {
                    // sensors_console power drives the Sensors radar range multiplier
                    multipliers.insert(Console::Sensors, pm);
                }
            }
            commands.insert_resource(PowerMultiplierResource { multipliers });

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
                    });
            commands.insert_resource(crate::ship_plugin::ShipPhysicsConfigResource(
                physics_cfg.unwrap_or(crate::ship_physics::ShipPhysicsConfig::new()),
            ));

            // Impulse config from [helm_console] TOML, or default
            let impulse_cfg =
                config
                    .helm_console
                    .as_ref()
                    .map(|hc| crate::ship_plugin::ImpulseConfigResource {
                        charge_duration: hc.impulse_charge_duration,
                        speed_multiplier: hc.impulse_speed_multiplier,
                        acceleration_multiplier: hc.impulse_acceleration_multiplier,
                    });
            commands.insert_resource(impulse_cfg.unwrap_or_default());

            // Boost config from [helm_console.boost] TOML. Absent table ⇒
            // feature disabled (default resource has `enabled: false`).
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
                });
            commands.insert_resource(boost_cfg.unwrap_or_default());

            // Bank config from [helm_console] TOML, or default
            let bank_cfg =
                config
                    .helm_console
                    .as_ref()
                    .map(|hc| crate::ship_plugin::BankConfigResource {
                        max_bank_deg: hc.max_bank_deg,
                        bank_lerp_rate: hc.bank_lerp_rate,
                    });
            commands.insert_resource(bank_cfg.unwrap_or_default());
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

/// Add visual meshes and materials to spawned entities that have a `[mesh]`
/// section but no `RenderProcessed` yet. When `cfg.model` is set, loads a GLB
/// scene instead of creating a procedural shape — but defers insertion until
/// the asset is actually loaded (avoids attaching an unloaded handle that
/// would never retry). Applies `cfg.scale` and `cfg.rotation` to the entity's
/// transform in both paths. Additionally, if the entity carries a `Lights`
/// component (from one or more `[[light]]` TOML entries), attach the matching
/// `PointLight`/`DirectionalLight` components (single light → inline, multiple
/// → spawned as child entities).
fn render_spawned_entities(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut star_surface_materials: ResMut<Assets<crate::entity_star::StarSurfaceMaterial>>,
    mut star_halo_materials: ResMut<Assets<crate::entity_star::StarHaloMaterial>>,
    scenes: Res<Assets<bevy::scene::Scene>>,
    entities: Query<
        (
            Entity,
            &Transform,
            Option<&crate::entity_spawner::MeshSection>,
            Option<&crate::entity_spawner::StarSection>,
            Option<&crate::entity_spawner::Lights>,
            Option<&PendingSceneHandle>,
        ),
        Without<RenderProcessed>,
    >,
) {
    use crate::entity_config::MeshShape;

    for (entity, transform, mesh_sec, star_sec, lights_opt, pending) in entities.iter() {
        let mesh_cfg_for_transform = mesh_sec.map(|mesh_sec| &mesh_sec.0);
        let mut ec = commands.entity(entity);
        let mut rendered = false;

        if let Some(star_sec) = star_sec {
            let cfg = &star_sec.0;
            let surface_mesh = meshes.add(crate::entity_star::uv_sphere_mesh(
                cfg.radius,
                cfg.longitude_segments,
                cfg.latitude_segments,
            ));
            let surface_mat =
                star_surface_materials.add(crate::entity_star::surface_material_from_config(cfg));
            ec.insert((Mesh3d(surface_mesh), MeshMaterial3d(surface_mat)));

            let halo_radius = cfg.radius * cfg.halo_radius_multiplier.max(1.0);
            let halo_mesh = meshes.add(crate::entity_star::halo_quad_mesh(halo_radius));
            let halo_mat =
                star_halo_materials.add(crate::entity_star::halo_material_from_config(cfg));
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

            if let Some(model_path) = &cfg.model {
                // PATH A: GLB model. When `no_render` is true we only need the
                // sidecar (for ModelMarkers); we skip loading the GLB entirely.
                if cfg.no_render {
                    // Sidecar-only path: resolve once, attach markers, done.
                    let rig = match resolve_sidecar_rig(model_path, cfg.variant.as_deref()) {
                        Some(rig) => rig,
                        None => {
                            // Sidecar fetch still in flight (wasm) — retry next frame.
                            continue;
                        }
                    };
                    ec.insert(crate::model_rig::ModelMarkers(rig.markers.clone()));
                    rendered = true;
                } else {
                    // PATH A (render): load the GLB scene and its sidecar. Issue the
                    // load once and store the handle so the asset server keeps the
                    // asset alive. On subsequent frames the same strong handle is
                    // retrieved via `pending`, and we check readiness against it.
                    let scene: Handle<bevy::scene::Scene> = match pending {
                        Some(p) => p.0.clone(),
                        None => {
                            // `asset_server` resolves paths relative to the `assets/`
                            // root, but the TOML `model` field carries an `assets/`
                            // prefix (matching the `template_path` convention, which is
                            // read via `std::fs` relative to the cwd). Strip it so the
                            // GLB resolves correctly instead of looking for
                            // `assets/assets/models/...` and silently failing to load —
                            // which leaves the entity unrendered and invisible.
                            let rel = model_path.strip_prefix("assets/").unwrap_or(model_path);
                            let path = format!("{}#Scene0", rel);
                            let h: Handle<bevy::scene::Scene> = asset_server.load(&path);
                            // Diagnostic: distinguish prefetch hits (asset already in
                            // path-cache, will arrive quickly) from cold loads (first
                            // request for this path, network round-trip pending).
                            bevy::log::info!(
                            "render_spawned_entities: requesting scene {path} (load_state={:?})",
                            asset_server.load_state(h.id())
                        );
                            ec.insert(PendingSceneHandle(h.clone()));
                            h
                        }
                    };
                    // Hard-fail surface: a `LoadState::Failed` GLB will never appear in
                    // `Assets<Scene>`, so the `scenes.get(...).is_some()` check would
                    // spin forever. Mark such entities `RenderProcessed` so we stop
                    // retrying them every frame, and warn once per entity.
                    if matches!(
                        asset_server.load_state(scene.id()),
                        bevy::asset::LoadState::Failed(_)
                    ) {
                        bevy::log::warn!(
                        "render_spawned_entities: GLB failed to load for entity {entity:?}, path={model_path} — entity will exist without a mesh"
                    );
                        ec.remove::<PendingSceneHandle>();
                        ec.insert(RenderProcessed);
                        continue;
                    }
                    // Wait for BOTH the GLB scene AND the rig sidecar before finalising.
                    // The sidecar resolves to an identity rig when genuinely absent, so
                    // models without a sidecar still render (visually unchanged).
                    if scenes.get(&scene).is_some() {
                        let rig = match resolve_sidecar_rig(model_path, cfg.variant.as_deref()) {
                            Some(rig) => rig,
                            None => {
                                // Sidecar fetch still in flight (wasm) — retry next frame.
                                continue;
                            }
                        };

                        ec.remove::<PendingSceneHandle>();

                        // Composition: entityTransform ∘ baseRig ∘ model. The base rig
                        // is applied INNER to the per-entity transform by spawning the
                        // GLB SceneRoot as a CHILD carrying `base_bevy_transform()`,
                        // while the per-entity Transform (spawn position + per-entity
                        // scale/rotation) stays on the parent below. Spawning the scene
                        // on a child (instead of the entity) also keeps the base rig's
                        // non-uniform scale from composing badly with the per-entity
                        // rotation on a single Transform.
                        let base_tf = rig.base_bevy_transform();
                        let scene_for_child = scene.clone();
                        ec.with_children(|parent| {
                            parent.spawn((bevy::scene::SceneRoot(scene_for_child), base_tf));
                        });

                        // Attach the resolved marker map so downstream systems (weapons,
                        // exhaust, …) can resolve mount points by name.
                        ec.insert(crate::model_rig::ModelMarkers(rig.markers.clone()));

                        rendered = true;
                    }
                } // end else (render path)
            } else {
                // PATH B: Procedural primitive.
                let color = if cfg.colour.len() >= 3 {
                    Color::srgb(cfg.colour[0], cfg.colour[1], cfg.colour[2])
                } else {
                    Color::srgb(0.6, 0.6, 0.6)
                };

                let emissive_mul = cfg.emissive.unwrap_or(0.4);
                let emissive = LinearRgba::from(color) * emissive_mul;

                let mesh = match cfg.shape {
                    MeshShape::Sphere => meshes.add(Sphere {
                        radius: cfg.radius.max(0.1),
                    }),
                    MeshShape::Cuboid => {
                        let [x, y, z] = cfg.size.unwrap_or([2.0, 1.0, 3.0]);
                        meshes.add(Cuboid::new(x, y, z))
                    }
                    MeshShape::Torus => meshes.add(Torus {
                        major_radius: cfg.radius.max(0.5),
                        minor_radius: cfg.minor_radius.max(0.1),
                    }),
                };

                let mat = materials.add(StandardMaterial {
                    base_color: color,
                    emissive,
                    ..default()
                });

                ec.insert((Mesh3d(mesh), MeshMaterial3d(mat)));
                rendered = true;
            }

            if !rendered {
                // GLB not loaded yet — try again next frame.
                continue;
            }

            // Apply scale/rotation to both paths — preserves spawn position.
            if let Some(cfg) = mesh_cfg_for_transform
                .filter(|cfg| cfg.scale != 1.0 || cfg.rotation != [0.0, 0.0, 0.0])
            {
                ec.insert(Transform {
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
        } else {
            continue;
        }

        // Mark processed so we never visit this entity again.
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
    ship_query: Query<&GlobalTransform, With<Ship>>,
    mut light_query: Query<(&GlobalTransform, &mut Transform), With<FacePlayerLight>>,
) {
    let Ok(ship_transform) = ship_query.single() else {
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
        .insert_resource(ShipState::new())
        .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
            (Console::Helm, 25.0),
            (Console::Tactical, 25.0),
            (Console::Power, 25.0),
            (Console::Shields, 25.0),
        ])))
        .insert_resource(ShipShields(ShieldSystem::default()))
        .insert_resource(ShipImpulse(ImpulseState::new()))
        .init_resource::<WorldResource>()
        .insert_resource(crate::modifiers::ShipModifiers::new())
        .init_resource::<TrackedEntities>()
        .insert_resource(SimBroadcastTimer(Timer::new(
            std::time::Duration::from_nanos(1),
            TimerMode::Repeating,
        )))
        .init_resource::<WorldSetupBroadcast>()
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<LastBroadcastHull>()
        .init_resource::<LastBroadcastShields>()
        .init_resource::<SystemBlackboards>()
        .init_resource::<FrozenBlackboards>()
        .init_resource::<LastBroadcastBlackboards>()
        .init_resource::<crate::messages::AdmittedCommands>()
        .init_resource::<crate::messages::InterSystemQueue>()
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
                handle_set_sensors_target,
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
        .add_systems(Update, sim_processing_anchor)
        .add_plugins(weapons_update_broadcaster())
        .add_plugins(sim_state_broadcaster())
        .add_plugins(modifier_events_broadcaster())
        .add_systems(PostUpdate, collect);
        // Spawn the Ship entity immediately so systems that query it (including
        // auth checks in handle_fire_torpedo, handle_power_messages, etc.) work
        // during Lobby as well as InProgress.
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
        ));
        app
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
            out.push(OutboundMessage { target, msg });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn load_tube_now(app: &mut App, tube: &str) {
        app.world_mut()
            .resource_mut::<TorpedoSystemResource>()
            .0
            .tube_mut(tube)
            .expect("test tube should exist")
            .load_state = crate::torpedo::TubeLoadState::Loaded;
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
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        tick(app); // process_lobby → sets NextState::Set(InProgress)
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
                station: "Captain's Chair".into(),
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
                station: "Captain's Chair".into(),
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
                station: "Captain's Chair".into(),
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
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::ScienceRadar
        );
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
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::SensorsRadar
        );
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
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
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
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::SystemChart
        );
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
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
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
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
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
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::NavigationChart
        );
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
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
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
                station: "Captain's Chair".into(),
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
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Comms
        );
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
                    mode: ViewMode::Camera(ViewDirection::Aft),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Aft)
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
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
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
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Radar
        );
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
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
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
                    mode: ViewMode::Camera(ViewDirection::Aft),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
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
                station: "Captain's Chair".into(),
            },
        );
        tick(&mut app);
        // Simulate OnEnter(InProgress) having run by inserting ShipHullIntegrity.
        // The test explicitly advances phase to InProgress so broadcast_world_setup_on_start fires.
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
                station: "Captain's Chair".into(),
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
    fn hull_integrity_starts_at_100_and_appears_in_console_hull_update() {
        let mut app = test_app();
        start_game(&mut app);
        // The first InProgress tick (inside start_game) already emitted and consumed
        // the initial ConsoleHullUpdate. Reset the cache to force re-emission.
        app.world_mut()
            .resource_mut::<LastBroadcastHull>()
            .0
            .clear();
        let out = tick(&mut app);
        let entries = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::ConsoleHullUpdate { entries } => Some(entries.clone()),
                _ => None,
            })
            .expect("expected a ConsoleHullUpdate broadcast");
        let total: f32 = entries.iter().map(|c| c.current).sum();
        assert!((total - 100.0).abs() < 1e-6);
    }

    #[test]
    fn direct_damage_reduces_hull_integrity_in_broadcast() {
        let mut app = test_app();
        start_game(&mut app);
        // Consume the initial ConsoleHullUpdate so LastBroadcastHull is seeded.
        let _ = tick(&mut app);

        // Directly apply damage to the resource (simulates collision at ~half speed).
        {
            let mut rng = rand::rng();
            app.world_mut()
                .resource_mut::<ShipHullIntegrity>()
                .0
                .apply_damage(10.0, &mut rng);
        }

        let out = tick(&mut app);
        let entries = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::ConsoleHullUpdate { entries } => Some(entries.clone()),
                _ => None,
            })
            .expect("expected a ConsoleHullUpdate after damage");
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
            crate::entity_spawner::EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
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
                crate::entity_spawner::EntityConsoleHull(crate::damage::ConsoleHull::from_config(
                    &[(crate::messages::Console::CaptainChair, 30.0)],
                )),
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
                station: "Captain's Chair".into(),
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
        assert_eq!(
            app.world().resource::<WeaponsTarget>().0.as_deref(),
            Some("target-uuid")
        );
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
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
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
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
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
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );
    }

    /// FirePhaser is silently ignored when the phaser is on cooldown.
    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Manually put the cooldown into active state (simulating a beam just ended).
        app.world_mut().resource_mut::<ActiveBeam>().target_uuid = None;
        app.world_mut()
            .resource_mut::<PhaserCooldown>()
            .start_bank_with_cooldown("port", 3.0);

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
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );

        // Fast-forward: accumulate 30 damage via the damage_accumulator.
        // Set accumulator to 30.0 so all damage applies in one tick.
        {
            let mut b = app.world_mut().resource_mut::<ActiveBeam>();
            b.damage_accumulator = 30.0;
            b.remaining_secs = 5.0; // still "ongoing"
        }

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
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none());

        // Cooldown started.
        assert!(
            app.world()
                .resource::<PhaserCooldown>()
                .is_bank_active("port"),
            "cooldown should start after beam end"
        );

        // The entity should be despawned.
        assert!(
            app.world()
                .get::<crate::entity_spawner::EntityConsoleHull>(asteroid_entity)
                .is_none(),
            "asteroid entity should be despawned"
        );
    }

    /// Beam severs when ship rotates target out of the 180Â° forward arc.
    #[test]
    fn beam_severs_when_target_leaves_forward_arc() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Now rotate ship so the asteroid is behind it (yaw = Ï€ â†' facing +Z, asteroid at (0,-20) is behind).
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves forward arc"
        );
        assert!(
            app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-arc"
        );
        assert!(
            app.world()
                .resource::<PhaserCooldown>()
                .is_bank_active("port"),
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
            app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-range"
        );
        assert!(
            app.world()
                .resource::<PhaserCooldown>()
                .is_bank_active("port"),
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
        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 10.0;
        let _ = tick(&mut app);

        // Now sever by rotating ship.
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;
        let _ = tick(&mut app);

        let hp = app
            .world()
            .get::<crate::entity_spawner::EntityConsoleHull>(asteroid_entity)
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
            crate::entity_spawner::EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
                30.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("t2".into()),
            crate::entity_spawner::EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
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
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("t1")
        );

        // Natural beam expiry: set remaining to 0.
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 0.0;
        // Zero damage accumulator so no destruction fires.
        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 0.0;
        let _ = tick(&mut app); // beam ends, cooldown starts

        // Cooldown should be active.
        assert!(app
            .world()
            .resource::<PhaserCooldown>()
            .is_bank_active("port"));

        // Force cooldown to expire.
        app.world_mut()
            .resource_mut::<PhaserCooldown>()
            .start_bank_with_cooldown("port", 0.0);

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
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("t2")
        );
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
                station: "Captain's Chair".into(),
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
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
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
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
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
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 1,
                console: Console::Tactical,
            },
        );
        tick(&mut app);
        // Redirect team 0 (different console → Returning)
        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Power,
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
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
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

    // â"€â"€ SetSensorsTarget / SensorsTargetSuggestion tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

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
                station: "Captain's Chair".into(),
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
    }

    #[test]
    fn sensors_set_sensors_target_broadcasts_sensors_target_suggestion_to_tactical() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);

        push(
            &mut app,
            "sensors",
            ClientMessage::ControlSystem {
                target: crate::system_registry::sensors_system_id(),
                payload: SystemControlPayload::SetScienceTarget {
                    uuid: "asteroid-99".into(),
                },
            },
        );
        let out = tick(&mut app);

        let suggestion = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::SensorsTargetSuggestion { uuid } => Some(uuid.clone()),
                _ => None,
            })
            .expect("expected a SensorsTargetSuggestion message");
        assert_eq!(suggestion, "asteroid-99");

        // Must be targeted to Tactical console player only.
        let suggestion_msg = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::SensorsTargetSuggestion { .. }))
            .unwrap();
        assert!(
            matches!(&suggestion_msg.target, Target::Token(t) if t == "weapons"),
            "SensorsTargetSuggestion should be sent only to Tactical console"
        );
    }

    #[test]
    fn non_sensors_player_cannot_send_sensors_target() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::sensors_system_id(),
                payload: SystemControlPayload::SetScienceTarget {
                    uuid: "asteroid-99".into(),
                },
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::SensorsTargetSuggestion { .. })),
            "non-Sensors player should not be able to send SensorsTargetSuggestion"
        );
    }

    #[test]
    fn set_sensors_target_ignored_in_lobby() {
        let mut app = test_app();
        push(
            &mut app,
            "sensors",
            ClientMessage::Identify {
                token: "sensors".into(),
                name: "Spock".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "sensors",
            ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
        );
        tick(&mut app);

        push(
            &mut app,
            "sensors",
            ClientMessage::ControlSystem {
                target: crate::system_registry::sensors_system_id(),
                payload: SystemControlPayload::SetScienceTarget {
                    uuid: "asteroid-99".into(),
                },
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::SensorsTargetSuggestion { .. })),
            "SetSensorsTarget should be ignored during Lobby phase"
        );
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
        use crate::modifiers::{Modifier, ShipModifiers};

        // --- App with 2Ã— PhaserDamage modifier ---
        let mut app_fast = test_app();
        setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
        // Apply 2Ã— phaser damage modifier before game starts.
        {
            let mut mods = app_fast.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::PhaserDamage,
                bonus: 1.0, // â†' multiplier 2.0
            });
        }
        start_game_with_weapons(&mut app_fast);
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

        // Inject accumulated damage: 3.5s Ã— (5 HP/s Ã— 2Ã—) = 35 HP â†' enough to destroy 30-HP asteroid.
        {
            let mut beam = app_fast.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 2.0 * 3.5;
        }
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
                             // Inject same real time but at base rate: 3.5s Ã— 5 HP/s = 17.5 HP accumulated
        {
            let mut beam = app_base.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 1.0 * 3.5;
        }
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
        use crate::modifiers::{Modifier, ShipModifiers};

        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app); // consume startup messages

        // Register a modifier on the live resource.
        {
            let mut mods = app.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::MaxSpeed,
                bonus: 0.5,
            });
        }
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
        use crate::modifiers::{Modifier, ShipModifiers};

        let mut app = test_app();
        start_game(&mut app);
        // Add first so there's something to remove.
        {
            let mut mods = app.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::MaxSpeed,
                bonus: 0.5,
            });
        }
        tick(&mut app);

        // Now remove it.
        {
            let mut mods = app.world_mut().resource_mut::<ShipModifiers>();
            mods.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
        }
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
        let mut hull = crate::damage::ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)]);

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
        let mut hull = crate::damage::ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)]);

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
        let mut hull = crate::damage::ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)]);

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
        use crate::modifiers::{Modifier, ShipModifiers};

        // Hull damage halved via modifier.
        let mut app = test_app();
        start_game(&mut app);
        {
            let mut mods = app.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::HullDamageTaken,
                bonus: -1.0, // â†' multiplier 0.5
            });
        }

        // Apply collision damage directly through the formula used in handle_collisions.
        // At 200 u/s: collision_damage(200) = round(200 * 0.5) = 100.
        // With 0.5Ã— modifier: round(100 * 0.5) = 50.
        fn near(a: f32, b: f32) -> bool {
            (a - b).abs() < 1e-6
        }
        let mods = app.world().resource::<ShipModifiers>().clone();
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

        // Verify the hull loses only the scaled amount by triggering damage through the resource.
        {
            let mut rng = rand::rng();
            app.world_mut()
                .resource_mut::<ShipHullIntegrity>()
                .0
                .apply_damage(scaled_damage, &mut rng);
        }
        let out = tick(&mut app);
        let entries = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::ConsoleHullUpdate { entries } => Some(entries.clone()),
                _ => None,
            })
            .expect("expected ConsoleHullUpdate");
        let total: f32 = entries.iter().map(|c| c.current).sum();
        assert!(
            near(total, 50.0),
            "hull should be 100 - 50 = 50 with halved collision damage"
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
                station: "Captain's Chair".into(),
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
    }

    #[test]
    fn non_power_sender_increase_power_is_ignored() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Reset power to known state.
        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 1;

        // Captain (not Power holder) tries to set Helm to 2.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_system_id(),
                payload: crate::messages::SystemControlPayload::SetPower {
                    target: Console::Helm,
                    level: 2,
                },
            },
        );
        let _ = tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.helm,
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
                target: crate::system_registry::power_system_id(),
                payload: crate::messages::SystemControlPayload::SetPower {
                    target: Console::Sensors,
                    level: 1,
                },
            },
        );
        let _ = tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.sensors,
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
                target: crate::system_registry::power_system_id(),
                payload: crate::messages::SystemControlPayload::SetPower {
                    target: Console::Helm,
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
                target: crate::system_registry::power_system_id(),
                payload: crate::messages::SystemControlPayload::SetPower {
                    target: Console::Tactical,
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
    fn no_power_console_holder_no_power_state_broadcast() {
        let mut app = test_app();
        // Only captain, no power console holder.
        start_game(&mut app);

        let out = tick(&mut app);
        let any_power_state = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::PowerState { .. }));
        assert!(
            !any_power_state,
            "no PowerState should be sent when no Power console holder exists"
        );
    }

    #[test]
    fn power_increase_respects_bounds_noop_at_four() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Manually set Helm to 4 (max).
        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 4;

        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_system_id(),
                payload: crate::messages::SystemControlPayload::SetPower {
                    target: Console::Helm,
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
            .insert(Console::Helm, [-0.5, 0.0, 1.0, 2.0]);

        // Set Helm to 3.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_system_id(),
                payload: crate::messages::SystemControlPayload::SetPower {
                    target: Console::Helm,
                    level: 3,
                },
            },
        );
        let _ = tick(&mut app);

        // Level 3 ? index 2 ? bonus 1.0 ? MaxSpeed multiplier = 2.0
        let mult = app
            .world()
            .resource::<ShipModifiers>()
            .get(&ModifierSlot::MaxSpeed);
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
            .insert(Console::Tactical, [-0.5, 0.0, 0.25, 0.5]);

        // Set Weapons to 1.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_system_id(),
                payload: crate::messages::SystemControlPayload::SetPower {
                    target: Console::Tactical,
                    level: 1,
                },
            },
        );
        let _ = tick(&mut app);

        // Level 1 ? index 0 ? bonus -0.5 (negative) ? 1.0 / (1.0 + 0.5) = 0.666...
        let expected = 1.0 / 1.5;
        let mult = app
            .world()
            .resource::<ShipModifiers>()
            .get(&ModifierSlot::PhaserDamage);
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
            .insert(Console::Helm, defaults);
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(Console::Tactical, defaults);
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(Console::Sensors, defaults);

        // Set state that will trigger exhaustion on the next tick:
        // total=8 (negative rate), battery already at 0 ? tick keeps it at 0
        // and forces all consoles to 1 + lock.
        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            ps.0.helm = 4;
            ps.0.weapons = 2;
            ps.0.sensors = 2;
            ps.0.battery_charge = 0.0;
            ps.0.locked = false;
        }

        // Tick triggers exhaustion ? lock changes ? sync_power_modifiers runs
        tick(&mut app);

        // All three forced to 1 ? bonus -0.5 (negative) ? multiplier = 1.0 / (1.0 + 0.5) ˜ 0.666...
        let expected = 1.0 / 1.5;
        let mods = app.world().resource::<ShipModifiers>();

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
        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 4;

        // Try to set sensors to 3 — total would be 9 (over cap), should be blocked at 2.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_system_id(),
                payload: crate::messages::SystemControlPayload::SetPower {
                    target: Console::Sensors,
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
            crate::entity_spawner::EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
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
        let freq = app.world().resource::<ShipState>().phaser_frequency;
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
        let freq = app.world().resource::<ShipState>().phaser_frequency;
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
        let freq = app.world().resource::<ShipState>().phaser_frequency;
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
        let freq = app.world().resource::<ShipState>().phaser_frequency;
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
        let freq = app.world().resource::<ShipState>().phaser_frequency;
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
                station: "Captain's Chair".into(),
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
    }

    #[test]
    fn shields_holder_can_focus_a_facing() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shields_system_id(),
                payload: SystemControlPayload::SetShieldFocus {
                    facing: Some(ViewDirection::Fore),
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipShields>().0.focused_facing,
            Some(0)
        );
        assert!(app.world().resource::<ShipShields>().0.facings[0].is_focused);
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
                target: crate::system_registry::shields_system_id(),
                payload: SystemControlPayload::SetShieldFocus {
                    facing: Some(ViewDirection::Port),
                },
            },
        );
        tick(&mut app);

        assert!(app
            .world()
            .resource::<ShipShields>()
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
                target: crate::system_registry::shields_system_id(),
                payload: SystemControlPayload::SetShieldFocus {
                    facing: Some(ViewDirection::Fore),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipShields>().0.focused_facing,
            Some(0)
        );

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shields_system_id(),
                payload: SystemControlPayload::SetShieldFocus { facing: None },
            },
        );
        tick(&mut app);
        assert!(app
            .world()
            .resource::<ShipShields>()
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
                station: "Captain's Chair".into(),
            },
        );
        tick(&mut app);

        // Still in Lobby — SetShieldFocus should be ignored.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shields_system_id(),
                payload: SystemControlPayload::SetShieldFocus {
                    facing: Some(ViewDirection::Aft),
                },
            },
        );
        tick(&mut app);

        assert!(app
            .world()
            .resource::<ShipShields>()
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
                target: crate::system_registry::shields_system_id(),
                payload: SystemControlPayload::SetShieldFocus {
                    facing: Some(ViewDirection::Fore),
                },
            },
        );
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let shield_status = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::ShieldStatus { facings } => Some(facings.clone()),
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
}
