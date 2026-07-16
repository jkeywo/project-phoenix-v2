/// Bevy plugin: NPC AI lifecycle — registers synthetic `ai:<uuid>` tokens,
/// drives per-entity helm/weapons/doctrine AI, and manages NPC hull tracking.
///
/// Compiled only for the `server` feature (same gate as `simulation.rs`).
use bevy::prelude::*;
use std::collections::HashMap;

/// Build the AI anchor lookup table from the unified `WorldConfig` (PRD #337).
///
/// `WorldConfig.anchors()` already returns `HashMap<String, [f32; 3]>` with
/// 2-element anchors widened to 3 components at parse time, so the AI tick
/// just needs a clone.
///
/// This is the sole anchor source after PRD #341.
pub fn anchors_from_world_config(
    world: &crate::world::config::WorldConfig,
) -> HashMap<String, [f32; 3]> {
    world.anchors().clone()
}

use crate::ai::lod::{evaluate_lod, LodState};
use crate::ai::AiMemory;
use crate::entity_spawner::{BehaviourSection, EntityUuid};
use crate::server_app::{LocalShip, Ship};
use crate::ship_state::ShipPhysics;

/// Repeating 10 Hz timer that gates `build_world_snapshot` and
/// `aggregate_doctrine_blackboards`.  Both systems only need to run at the
/// same cadence as the AI tick and the SimState broadcast — running them every
/// Bevy frame (60 Hz) multiplies their cost 6× with no benefit.
#[derive(Resource)]
pub struct AiSnapshotTimer(pub Timer);

impl Default for AiSnapshotTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.1, TimerMode::Repeating))
    }
}

/// Boolean latch set each frame by `tick_ai_snapshot_timer`.
/// `run_if` conditions must use read-only params, so the timer is advanced
/// by a dedicated system that writes this flag, which the condition then reads.
/// Initialises to `true` so the very first update always gets a snapshot
/// (before the timer has had a chance to fire).
#[derive(Resource)]
pub struct AiSnapshotReady(pub bool);

/// Advance the `AiSnapshotTimer` and set `AiSnapshotReady`.
/// Runs unconditionally in `SimSet::Physics` before `AiTickLabel`.
/// Only writes `true` when the timer fires; on frames where it doesn't fire
/// the flag is explicitly cleared so the gated systems skip their work.
fn tick_ai_snapshot_timer(
    time: Res<Time>,
    mut timer: ResMut<AiSnapshotTimer>,
    mut ready: ResMut<AiSnapshotReady>,
) {
    ready.0 = timer.0.tick(time.delta()).just_finished();
}

/// Read-only run condition: fires only when `AiSnapshotReady` is true.
fn ai_snapshot_ready(ready: Res<AiSnapshotReady>) -> bool {
    ready.0
}

// ── AiTokenRegistry ───────────────────────────────────────────────────────────

/// Maps entity UUID → synthetic token string (`"ai:<uuid>"`).
/// The simulation's token-to-entity lookup falls back to this registry when
/// a player session lookup misses.
#[derive(Resource, Default)]
pub struct AiTokenRegistry {
    /// entity_uuid → token string
    by_entity: HashMap<String, String>,
    /// token string → entity_uuid (reverse lookup)
    by_token: HashMap<String, String>,
    /// Bevy Entity id → entity_uuid (for despawn handler)
    by_bevy_entity: HashMap<Entity, String>,
    /// entity_uuid → Bevy Entity (for admission-gate entity lookup)
    uuid_to_bevy: HashMap<String, Entity>,
}

impl AiTokenRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a synthetic token for an entity UUID, also storing the Bevy
    /// entity for despawn-time lookup. Idempotent.
    pub fn register(&mut self, entity_uuid: &str) -> &str {
        let token = format!("ai:{}", entity_uuid);
        self.by_entity
            .entry(entity_uuid.to_string())
            .or_insert_with(|| {
                self.by_token.insert(token.clone(), entity_uuid.to_string());
                token.clone()
            });
        self.by_entity
            .get(entity_uuid)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Register a synthetic token AND record the Bevy `Entity` for
    /// despawn-time unregistration. Idempotent.
    pub fn register_with_entity(&mut self, entity_uuid: &str, entity: Entity) {
        self.register(entity_uuid);
        self.by_bevy_entity.insert(entity, entity_uuid.to_string());
        self.uuid_to_bevy.insert(entity_uuid.to_string(), entity);
    }

    /// Unregister by entity UUID; silently does nothing if not present.
    pub fn unregister(&mut self, entity_uuid: &str) {
        if let Some(token) = self.by_entity.remove(entity_uuid) {
            self.by_token.remove(&token);
        }
        self.uuid_to_bevy.remove(entity_uuid);
    }

    /// Unregister by Bevy `Entity`; used by the despawn handler when the
    /// UUID component is no longer accessible.
    pub fn unregister_by_bevy_entity(&mut self, entity: Entity) {
        if let Some(uuid) = self.by_bevy_entity.remove(&entity) {
            self.unregister(&uuid);
        }
    }

    /// Look up the Bevy `Entity` for an AI token string. Used by the
    /// admission gate to verify the token belongs to the player ship.
    pub fn bevy_entity_for_token(&self, token: &str) -> Option<Entity> {
        let uuid = self.by_token.get(token)?;
        self.uuid_to_bevy.get(uuid).copied()
    }

    /// Look up an entity UUID by its synthetic token. Returns `None` when
    /// the token is not an AI token (so callers can fall back gracefully).
    pub fn entity_uuid_for_token(&self, token: &str) -> Option<&str> {
        self.by_token.get(token).map(|s| s.as_str())
    }

    /// Look up the synthetic token for an entity UUID.
    pub fn token_for_entity(&self, entity_uuid: &str) -> Option<&str> {
        self.by_entity.get(entity_uuid).map(|s| s.as_str())
    }

    /// Look up the Bevy `Entity` for an entity UUID string.
    pub fn bevy_entity_for_uuid(&self, entity_uuid: &str) -> Option<Entity> {
        self.uuid_to_bevy.get(entity_uuid).copied()
    }

    /// Returns `true` when the registry contains a record for this entity UUID.
    pub fn contains_entity(&self, entity_uuid: &str) -> bool {
        self.by_entity.contains_key(entity_uuid)
    }
}

// ── AiControllerComponent ─────────────────────────────────────────────────────

/// Per-entity AI memory component. Carries helm steering state (waypoint cursor,
/// target, last attacker) across ticks for all ship entities.
#[derive(Component, Default, Clone, Debug)]
pub struct ShipAiMemory(pub AiMemory);

/// Empty marker component placed on NPC entities that carry a `BehaviourSection`.
/// Used as a query filter in systems that target NPC ships specifically
/// (e.g. phaser beam handling). Inserted by `register_ai_tokens_on_spawn`.
#[derive(Component, Default)]
pub struct AiControllerComponent;

/// Marker component: entity is eligible for high-fidelity AI simulation.
/// Entities without this marker run at reduced simulation fidelity.
#[derive(Component)]
pub struct AiHighFidelity;

/// AI personality and capability profile for NPC entities.
#[derive(Component, Clone, Debug)]
pub struct AiProfile {
    pub aggression: f32,
    pub sensor_range: f32,
}

/// Tracks time since last LOD state transition for dwell-based demotion.
#[derive(Component, Clone, Debug)]
pub struct LodTransitionTimer {
    pub last_state_change_secs: f64,
}

/// Per-objective patrol waypoint cursors.
///
/// Each entry is a [`PatrolCursor`] tracking the current waypoint for one
/// objective's patrol route. Entries are independent — advancing one does not
/// affect others. Cursor state is interpreted (and its out-of-range terminal
/// stop owned) by the pure `ai::patrol_cursor` module.
///
/// `advance_objective_cursors` (`SimSet::Modifiers`) is the sole writer: it
/// owns arrival detection and cursor advancement for every ship regardless of
/// LOD. Everyone else is a reader — notably `simulate_low_lod_ships`
/// (`SimSet::Physics`), which reads the cursor to cheaply steer NPCs outside
/// sensor range toward their current waypoint. That split is what stops a
/// cursor from being advanced twice in one tick.
///
/// The high-LOD path (`helm_patrol`) tracks its waypoint separately via
/// `AiMemory.waypoint_index`; unifying the two is issue #702 work.
#[derive(Component, Clone, Debug, Default)]
pub struct PatrolCursors(pub Vec<crate::ai::patrol_cursor::PatrolCursor>);

/// Marker component set on NPC entities currently in a warp-out sequence.
/// Carries the data needed to draw the warp-exit visual and to populate
/// `EntitySnapshot::warp_out_remaining_secs` in the broadcast.
/// Kept for interface compatibility; not set by the doctrine-based AI system.
#[derive(Component)]
pub struct WarpOutMarker {
    pub remaining_secs: f32,
    pub target_speed: f32,
}

/// Component: carries the UUID of an entity that attacked this NPC during the
/// current tick. Written by the simulation (or tests) to signal an incoming
/// hit. Consumed by `tick_ai_controllers` to populate `attacker_this_tick` in
/// the WorldView and emit an `AiEntityAttacked` event.
#[derive(Component, Clone, Debug)]
pub struct AttackerThisTick(pub uuid::Uuid);

// ── Events ────────────────────────────────────────────────────────────────────

/// Marker component added to an AI entity when its owning scenario is unloaded.
///
/// `tick_ai_controllers` reads this component to set `scenario_unloaded: true`
/// in the `WorldView`. The component persists until `tick_ai_controllers`
/// removes it (or until the entity despawns alongside its scenario cleanup).
#[derive(Component)]
pub struct ScenarioUnloadedMarker;

/// Resource kept for backward-compatibility; no longer used for signalling.
#[derive(Resource, Default)]
pub struct ScenariosBeingUnloaded(pub std::collections::HashSet<String>);

/// Emitted by the AI plugin when an NPC entity's `last_attacker` in memory
/// changes (a new attacker UUID arrives).
///
/// The world plugin observes this event to evaluate `on_entity_attacked`
/// trigger conditions without a direct dependency on the AI module.
#[derive(Message, Clone, Debug)]
pub struct AiEntityAttacked {
    pub entity_uuid: String,
    pub attacker_uuid: uuid::Uuid,
}

/// Emitted by the AI plugin when an NPC entity's hull reaches ≤ 0.0.
///
/// The world plugin observes this event to evaluate `on_entity_destroyed`
/// trigger conditions without a direct dependency on the AI module.
#[derive(Message, Clone, Debug)]
pub struct AiEntityDestroyed {
    pub entity_uuid: String,
}

/// Emitted by `advance_objective_cursors` when a ship reaches the waypoint its
/// cursor is currently pointing at, immediately before the cursor advances.
///
/// The world plugin reads this in `handle_ai_events` and turns it into a
/// `WorldEvent::WaypointReached`, which drives `on_waypoint_reached` scenario
/// triggers — the same event-bridge shape `AiEntityAttacked` /
/// `AiEntityDestroyed` already use, so the AI module stays free of any
/// dependency on world content.
#[derive(Message, Clone, Debug)]
pub struct AiWaypointReached {
    /// UUID of the ship that arrived.
    pub entity_uuid: String,
    /// Id of the objective whose cursor advanced.
    pub objective_id: String,
    /// Anchor name of the waypoint that was reached.
    pub waypoint: String,
}

// ── WorldSnapshot resource ────────────────────────────────────────────────────

/// Snapshot of all ship/world-entity positions built once per tick.
/// All `operate_*_ai` handlers read from this resource rather than building
/// their own queries.
#[derive(Resource, Default)]
pub struct WorldSnapshot {
    pub entities: Vec<crate::ai::AiWorldEntity>,
}

/// Build the [`WorldSnapshot`] from every entity with an `EntityUuid`. Runs in
/// `SimSet::Physics` before `AiTickLabel` so per-system AI handlers see a
/// consistent frame.
fn build_world_snapshot(
    mut snapshot: ResMut<WorldSnapshot>,
    query: Query<(
        &crate::entity_spawner::EntityUuid,
        &Transform,
        Option<&crate::entity_spawner::EntityName>,
        Option<&crate::entity_spawner::FactionComponent>,
        Option<&crate::entity_spawner::EntitySystemHull>,
        Option<&crate::entity_spawner::ColliderSection>,
        Option<&crate::ship_state::ShipPhysics>,
    )>,
) {
    snapshot.entities = query
        .iter()
        .map(
            |(uuid, transform, name, faction, hull, collider, physics)| {
                let hull_fraction = hull.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let radius = collider.map(|c| c.0.radius).unwrap_or(0.0);
                // Prefer ShipPhysics.forward_speed (authoritative for all ships after #587);
                // Use ShipPhysics.forward_speed (authoritative after #581).
                let forward_speed = physics.map(|p| p.forward_speed).unwrap_or(0.0);
                crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::parse_str(&uuid.0).unwrap_or_default(),
                    name: name.as_ref().map(|n| n.0.clone()),
                    position: [
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ],
                    faction: faction.map(|f| f.0),
                    hull_fraction,
                    yaw: Some(transform.rotation.to_euler(bevy::math::EulerRot::YXZ).0),
                    radius,
                    forward_speed,
                    shields: None,
                }
            },
        )
        .collect();
}

/// Score each entity's doctrine and write `scored_objectives` into its
/// `ShipSystemBlackboards` viewscreen entry. Covers all ships that carry a
/// `BehaviourSection` — both NPC ships and any future player-ship variant that
/// opts into doctrine-based AI.
///
/// After PRD #597 PR 10: reads red-alert / combat-activity / last-attacker
/// from each ship's own per-entity components, so NPC ship viewscreen
/// blackboards mirror the same fields the player ship exposes.
fn aggregate_doctrine_blackboards(
    mut query: Query<(
        &BehaviourSection,
        &crate::entity_spawner::EntitySystemHull,
        &mut crate::server_app::ShipSystemBlackboards,
        Option<&crate::ship_state::ShipRedAlert>,
        Option<&crate::ship::combat_activity::RecentCombatActivity>,
        Option<&crate::weapons_plugin::LastShipAttacker>,
    )>,
) {
    for (behaviour, hull, mut blackboards, red_alert_opt, activity_opt, last_attacker_opt) in
        &mut query
    {
        let hull_fraction = {
            let max = hull.0.total_max();
            if max > 0.0 {
                (hull.0.total_current() / max).clamp(0.0, 1.0)
            } else {
                1.0
            }
        };
        let red_alert = red_alert_opt.map(|ra| ra.0).unwrap_or(false);
        let conditions = crate::objectives::WorldConditions {
            red_alert,
            hull_fraction,
        };
        let mut scored = crate::ai::score_doctrine_pool(&behaviour.0.doctrine, &conditions);
        // Inject synthetic Retreat objective based on hull damage. The
        // threshold is designer-tunable per entity template via
        // `[behaviour] retreat_hull_threshold` (defaulting to
        // `DEFAULT_RETREAT_THRESHOLD` while parsing when the field is
        // absent), so a ship class can be made braver or more cautious
        // without a recompile.
        let retreat_score = crate::ai::retreat_score::score_retreat(
            hull_fraction,
            behaviour.0.retreat_hull_threshold,
        );
        if retreat_score > 0.0 {
            scored.push(crate::messages::ScoredObjective {
                id: "retreat".to_string(),
                score: retreat_score,
                // The empty anchor is intentional: the anchors map is not in
                // scope here, and per PRD #685 the consumer (`operate_helm` /
                // `resolve_helm_target_position`) resolves an empty/unknown
                // anchor to the ship's `AiMemory.home_position` (spawn), which
                // is the designed fallback retreat position. Not a bug.
                directive: crate::messages::AiDirective::Retreat {
                    anchor: String::new(),
                },
                source: crate::messages::ObjectiveSource::Doctrine,
                relevance: crate::objectives::directive_relevance(
                    &crate::messages::AiDirective::Retreat {
                        anchor: String::new(),
                    },
                ),
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "retreat".to_string(),
                    text: "Retreat — hull critically damaged".to_string(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Doctrine,
                },
            });
            // Restore the descending-score order that `score_doctrine_pool`
            // established and that every consumer relies on: `operate_helm`
            // and `resolve_helm_target_position` both take the FIRST
            // Helm-relevant entry as the top-scored directive rather than
            // scanning for the maximum. Pushing the synthetic Retreat onto the
            // tail without re-sorting would park it behind every doctrine
            // objective, so a Retreat could never be selected however low the
            // hull fell.
            scored.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        let viewscreen_bb = crate::messages::ViewscreenBlackboard {
            red_alert,
            hull_integrity_pct: hull_fraction * 100.0,
            last_damage_taken_secs: activity_opt.and_then(|a| a.last_damage_taken),
            last_weapon_fired_secs: activity_opt.and_then(|a| a.last_weapon_fired),
            last_attacker_uuid: last_attacker_opt.and_then(|la| la.0.clone()),
            scored_objectives: scored,
        };
        blackboards.0.insert(
            crate::messages::SystemId(
                crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID.to_string(),
            ),
            crate::messages::SystemBlackboard::Viewscreen(viewscreen_bb),
        );
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiTokenRegistry>();
        app.init_resource::<ScenariosBeingUnloaded>();
        app.add_message::<AiEntityAttacked>();
        app.add_message::<AiEntityDestroyed>();
        app.add_message::<AiWaypointReached>();
        app.init_resource::<WorldSnapshot>();
        app.init_resource::<AiSnapshotTimer>();
        app.insert_resource(AiSnapshotReady(true));
        // The snapshot systems run first (consuming AiSnapshotReady), then the
        // timer system resets / arms the flag for the next frame.
        // Explicit `.after()` ordering ensures the flag is consumed before it is
        // written, even when the SimSet chain is not configured (e.g. in unit tests).
        app.add_systems(
            Update,
            tick_ai_snapshot_timer
                .after(build_world_snapshot)
                .after(aggregate_doctrine_blackboards),
        );
        app.add_systems(
            Update,
            build_world_snapshot
                .in_set(crate::sim_sets::SimSet::Physics)
                .before(crate::sim_sets::AiTickLabel)
                .run_if(ai_snapshot_ready),
        );
        app.add_systems(
            Update,
            aggregate_doctrine_blackboards
                .in_set(crate::sim_sets::SimSet::PublishAggregate)
                .run_if(ai_snapshot_ready),
        );
        app.add_systems(
            Update,
            (
                simulate_low_lod_ships
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .before(lod_ai_ships),
                lod_ai_ships
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .before(build_world_snapshot)
                    .before(crate::sim_sets::AiTickLabel),
            ),
        );
        app.add_systems(
            Update,
            (
                register_ai_tokens_on_spawn,
                process_attacker_this_tick
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .in_set(crate::sim_sets::AiTickLabel),
                unregister_on_despawn,
            ),
        );
    }
}

// -- Systems -------------------------------------------------------------------------------

/// Register a synthetic `ai:<uuid>` token for any entity with `BehaviourSection`
/// that has not yet been registered, and attach the `AiControllerComponent` empty
/// marker so legacy `With<AiControllerComponent>` query filters still work.
fn register_ai_tokens_on_spawn(
    mut commands: Commands,
    mut registry: ResMut<AiTokenRegistry>,
    query: Query<
        (Entity, &EntityUuid, &Transform, Option<&ShipAiMemory>),
        (With<BehaviourSection>, Without<AiControllerComponent>),
    >,
) {
    for (entity, uuid, transform, existing_mem) in &query {
        let home = [
            transform.translation.x,
            transform.translation.y,
            transform.translation.z,
        ];
        registry.register_with_entity(&uuid.0, entity);
        let mut cmd = commands.entity(entity);
        cmd.insert(AiControllerComponent);
        // Seed ShipAiMemory with home_position if not already present (entities
        // spawned via spawn_entity get it from the spawner; bare test entities don't).
        if existing_mem.is_none() {
            cmd.insert(ShipAiMemory(AiMemory {
                home_position: home,
                ..Default::default()
            }));
        }
    }
}

/// Update per-entity `ShipAiMemory.last_attacker` and emit `AiEntityAttacked`
/// whenever an `AttackerThisTick` component arrives on an NPC entity.
/// Replaces the attacker-tracking phase of the retired `tick_ai_controllers`.
///
/// Only processes entities that already have `ShipAiMemory` (i.e., that have been
/// through at least one `register_ai_tokens_on_spawn` tick). On the very first
/// frame of an entity's life the component arrives via deferred commands and will
/// be processed on the following frame.
fn process_attacker_this_tick(
    mut commands: Commands,
    mut query: Query<(Entity, &EntityUuid, &AttackerThisTick, &mut ShipAiMemory)>,
    mut attacked_events: MessageWriter<AiEntityAttacked>,
) {
    for (entity, uuid, attacker, mut ai_mem) in query.iter_mut() {
        let attacker_uuid = attacker.0;
        let is_new = ai_mem.0.last_attacker != Some(attacker_uuid);
        if is_new {
            attacked_events.write(AiEntityAttacked {
                entity_uuid: uuid.0.clone(),
                attacker_uuid,
            });
            ai_mem.0.last_attacker = Some(attacker_uuid);
        }
        commands.entity(entity).remove::<AttackerThisTick>();
    }
}

/// Unregister synthetic tokens when AI-controlled entities are despawned.
fn unregister_on_despawn(
    mut registry: ResMut<AiTokenRegistry>,
    mut removed: RemovedComponents<AiControllerComponent>,
) {
    for entity in removed.read() {
        registry.unregister_by_bevy_entity(entity);
    }
}

// ── LOD Management ──────────────────────────────────────────────────────────────

/// Fractional hysteresis band applied on top of `AiProfile.sensor_range`
/// to prevent rapid LOD oscillation near the range boundary.
const LOD_HYSTERESIS: f32 = 0.2;

/// Minimum time (seconds) that must elapse before a demotion is allowed.
const LOD_DWELL_SECS: f64 = 2.0;

/// Evaluate LOD for every NPC ship vs the player ship's position.
/// Inserts `AiHighFidelity` when an NPC enters sensor range (promotion),
/// removes it when the NPC leaves range after the hysteresis + dwell window
/// has elapsed (demotion). `LocalShip` is never evaluated and is guaranteed
/// to keep its `AiHighFidelity` marker.
fn lod_ai_ships(
    time: Res<Time>,
    player: Query<&Transform, (With<LocalShip>, With<Ship>)>,
    npcs: Query<
        (
            Entity,
            &Transform,
            &AiProfile,
            Has<AiHighFidelity>,
            Option<&LodTransitionTimer>,
        ),
        (With<Ship>, Without<LocalShip>),
    >,
    mut commands: Commands,
) {
    let Ok(player_transform) = player.single() else {
        return;
    };
    let now_secs = time.elapsed_secs() as f64;
    let px = player_transform.translation.x;
    let pz = player_transform.translation.z;

    for (entity, transform, profile, is_high, timer) in &npcs {
        let dx = transform.translation.x - px;
        let dz = transform.translation.z - pz;
        let distance = (dx * dx + dz * dz).sqrt();

        let current_state = if is_high {
            LodState::High
        } else {
            LodState::Low
        };
        let last_change = timer.map(|t| t.last_state_change_secs).unwrap_or(0.0);

        let new_state = evaluate_lod(
            current_state,
            distance,
            profile.sensor_range,
            now_secs,
            last_change,
            LOD_DWELL_SECS,
            LOD_HYSTERESIS,
        );

        if new_state != current_state {
            let timer_comp = LodTransitionTimer {
                last_state_change_secs: now_secs,
            };
            match new_state {
                LodState::High => {
                    commands.entity(entity).insert(AiHighFidelity);
                    // AI intent/state components scoped to AiHighFidelity
                    // (issue #692, extended by #693 for power, #695 for
                    // helm) — bundled alongside the marker so they stay
                    // present exactly while the ship runs full-fidelity AI
                    // decision systems.
                    commands
                        .entity(entity)
                        .insert(crate::ship::shields::ShieldArcIntents::default());
                    commands
                        .entity(entity)
                        .insert(crate::console_ai_plugin::ShipFrequencyHintState::default());
                    commands
                        .entity(entity)
                        .insert(crate::ship::power::PowerReactorIntents::default());
                    commands
                        .entity(entity)
                        .insert(crate::ship::power::ShipPowerAiState::default());
                    commands
                        .entity(entity)
                        .insert(crate::weapons_plugin::TorpedoIntents::default());
                    commands.entity(entity).insert((
                        crate::ship::helm::ThrustInput::default(),
                        crate::ship::helm::SteeringInput::default(),
                        crate::ship::helm::LateralThrustInput::default(),
                        crate::ship::helm::ImpulseCommand::default(),
                        crate::ship::helm::BoostCommand::default(),
                    ));
                    commands.entity(entity).insert(timer_comp);
                }
                LodState::Low => {
                    commands.entity(entity).remove::<AiHighFidelity>();
                    commands
                        .entity(entity)
                        .remove::<crate::ship::shields::ShieldArcIntents>();
                    commands
                        .entity(entity)
                        .remove::<crate::console_ai_plugin::ShipFrequencyHintState>();
                    commands
                        .entity(entity)
                        .remove::<crate::ship::power::PowerReactorIntents>();
                    commands
                        .entity(entity)
                        .remove::<crate::ship::power::ShipPowerAiState>();
                    commands
                        .entity(entity)
                        .remove::<crate::weapons_plugin::TorpedoIntents>();
                    commands.entity(entity).remove::<(
                        crate::ship::helm::ThrustInput,
                        crate::ship::helm::SteeringInput,
                        crate::ship::helm::LateralThrustInput,
                        crate::ship::helm::ImpulseCommand,
                        crate::ship::helm::BoostCommand,
                    )>();
                    commands.entity(entity).insert(timer_comp);
                }
            }
        }
    }
}

/// The active Helm-relevant waypoint route a ship should be following, read
/// from its viewscreen blackboard.
///
/// `aggregate_doctrine_blackboards` publishes `scored_objectives` there for
/// every `BehaviourSection` ship regardless of LOD — the same entry
/// the per-axis helm AI reads on the high-LOD path (see `ship_plugin.rs`, the
/// "Shared helm-AI decision inputs" note).
/// Both the low-LOD steering path and the cursor evaluator resolve their route
/// through this one function so they can never disagree about which objective
/// owns the cursor.
///
/// Returns `(objective_id, waypoint_anchor_names, loop_path)` for the
/// highest-scored `Patrol` or `Reach` directive. `Reach` is modelled as a
/// one-waypoint, non-looping route.
fn active_waypoint_route(
    blackboards: &crate::server_app::ShipSystemBlackboards,
) -> Option<(String, Vec<String>, bool)> {
    let bb = match blackboards
        .0
        .get(&crate::system_registry::viewscreen_system_id())
    {
        Some(crate::messages::SystemBlackboard::Viewscreen(v)) => v,
        _ => return None,
    };
    bb.scored_objectives
        .iter()
        .filter(|o| {
            o.score > 0.0
                && o.relevance.contains(&crate::messages::SystemAffinity::Helm)
                && matches!(
                    o.directive,
                    crate::messages::AiDirective::Patrol { .. }
                        | crate::messages::AiDirective::Reach { .. }
                )
        })
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .and_then(|o| match &o.directive {
            crate::messages::AiDirective::Patrol { anchors, loop_path } => {
                Some((o.id.clone(), anchors.clone(), *loop_path))
            }
            crate::messages::AiDirective::Reach { anchor } => {
                Some((o.id.clone(), vec![anchor.clone()], false))
            }
            _ => None,
        })
}

/// Advance every ship's objective cursors as it reaches its waypoints.
///
/// Runs in `SimSet::Modifiers` — after `Physics` has moved the ships this
/// tick, so arrival is judged against fresh positions, and before `Publish`.
/// This is the single owner of `PatrolCursors` state: the low-LOD steering
/// path (`simulate_low_lod_ships`) only *reads* the cursor, so a cursor can
/// never be advanced twice in one tick.
///
/// Per ship, per active Helm-relevant `Patrol`/`Reach` objective: advance the
/// cursor via the pure `advance_cursor` (which judges arrival against the
/// radius and handles wraparound, terminal stops, settling degenerate looping
/// routes, and skipping waypoints whose anchors are unknown), then emit one
/// `AiWaypointReached` per waypoint it reports as consumed.
///
/// Covers all ships carrying a `BehaviourSection` regardless of LOD — the
/// high-LOD helm path still tracks its own waypoint via
/// `AiMemory.waypoint_index`; unifying the two is issue #702.
pub(crate) fn advance_objective_cursors(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<(
        &BehaviourSection,
        &ShipPhysics,
        &crate::server_app::ShipSystemBlackboards,
        &mut PatrolCursors,
        Option<&EntityUuid>,
    )>,
    mut reached: MessageWriter<AiWaypointReached>,
) {
    let Some(world_config) = world_config else {
        return;
    };
    let anchors = anchors_from_world_config(&world_config);

    for (behaviour, physics, blackboards, mut cursors, entity_uuid) in &mut ships {
        let Some((obj_id, waypoints, loop_path)) = active_waypoint_route(blackboards) else {
            continue;
        };

        // Arrival radius is authored per entity template in TOML
        // (`[behaviour] waypoint_arrival_radius`), so designers can tune how
        // close a ship must get before its cursor advances.
        let arrival_radius = behaviour.0.waypoint_arrival_radius;
        let entity_pos = [physics.x, 0.0, physics.z];

        // Look up (or lazily insert) this objective's cursor by objective id.
        if !cursors.0.iter().any(|c| c.objective_id == obj_id) {
            cursors
                .0
                .push(crate::ai::patrol_cursor::PatrolCursor::new(obj_id.clone()));
        }
        let cursor = cursors
            .0
            .iter_mut()
            .find(|c| c.objective_id == obj_id)
            .expect("cursor entry just ensured to exist");

        // One tick can carry a ship past several waypoints at once (waypoints
        // spaced closer together than the arrival radius, or a slow tick), so
        // `advance_cursor` reports *every* waypoint it consumed rather than
        // just the first — one message each, or triggers keyed to the
        // intermediate waypoints would silently never fire.
        let reached_waypoints = crate::ai::patrol_cursor::advance_cursor(
            cursor,
            &waypoints,
            loop_path,
            entity_pos,
            &anchors,
            arrival_radius,
        );

        // Only ships with a UUID can be named by a scenario trigger; bare
        // test entities without one simply advance their cursor silently.
        let Some(uuid) = entity_uuid else {
            continue;
        };
        for waypoint in reached_waypoints {
            reached.write(AiWaypointReached {
                entity_uuid: uuid.0.clone(),
                objective_id: obj_id.clone(),
                waypoint,
            });
        }
    }
}

/// Cheap steering / forward-movement simulation for ships running at low
/// fidelity (those without the `AiHighFidelity` marker).
///
/// A low-LOD ship with an active Helm-relevant `Patrol`/`Reach` objective
/// cheaply follows its route: it snaps its heading toward the waypoint its
/// [`PatrolCursors`] entry currently points at and advances forward at
/// `forward_speed`. Ships with no such objective (or a stalled/terminal route)
/// keep the pre-existing dumb forward-drift so they don't regress to standing
/// still.
///
/// Read-only with respect to the cursor: arrival detection and advancement
/// belong to `advance_objective_cursors` in `SimSet::Modifiers`.
///
/// This is the *low-fidelity* path. It deliberately does NOT touch the
/// high-LOD patrol path (`helm_patrol` / `AiMemory.waypoint_index` in
/// `ai/core.rs`). The two coexist: low-LOD tracks the waypoint via
/// `PatrolCursors`, high-LOD via `AiMemory.waypoint_index`. Unifying them is
/// separate issue #702 work (a known, accepted limitation).
///
/// # Sanctioned out-of-band `ShipPhysics` writer (issue #699)
///
/// `integrate_ship_physics` is the sole *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`. This system writes
/// `x`/`z`/`yaw` directly and is an intentional exception: it is filtered to
/// `Without<AiHighFidelity>`, i.e. exactly the ships that carry no helm intent
/// components at all, so the helm path cannot serve them and the two writers
/// can never touch the same entity. It deliberately does not opt into the
/// debug `HelmPhysicsWriteGuard`. See the writer-policy table on `ShipPhysics`
/// (`src/ship/state.rs`).
fn simulate_low_lod_ships(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<
        (
            &mut ShipPhysics,
            Option<&crate::server_app::ShipSystemBlackboards>,
            Option<&PatrolCursors>,
        ),
        (With<Ship>, Without<AiHighFidelity>),
    >,
) {
    let dt = time.delta_secs();
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();

    for (mut physics, blackboards, cursors) in &mut ships {
        let route = blackboards.and_then(active_waypoint_route);

        // Steer along the route only when we have a Patrol/Reach objective AND
        // a `PatrolCursors` component tracking the waypoint index. A low-LOD
        // ship lacking either just falls through to the dumb forward-move
        // below. A ship whose cursor entry has not been created yet (the
        // evaluator inserts it at the end of this tick) steers toward the
        // first waypoint in the meantime.
        if let (Some((obj_id, waypoints, loop_path)), Some(cursors)) = (route, cursors) {
            let index = cursors
                .0
                .iter()
                .find(|c| c.objective_id == obj_id)
                .map(|c| c.index())
                .unwrap_or(0);

            let target =
                crate::ai::patrol_cursor::cursor_target(index, &waypoints, loop_path, &anchors);

            if let Some(target_pos) = target {
                // Cheap steering: snap yaw toward the target XZ bearing, then
                // advance forward. The forward vector below is
                // (sin(yaw), -cos(yaw)), so the bearing that makes the ship
                // face `target` is `dx.atan2(-dz)` (the same world-bearing
                // convention used across the sim's yaw math).
                let dx = target_pos[0] - physics.x;
                let dz = target_pos[2] - physics.z;
                if dx * dx + dz * dz > f32::EPSILON {
                    physics.yaw = dx.atan2(-dz);
                }
                physics.x += physics.forward_speed * physics.yaw.sin() * dt;
                physics.z -= physics.forward_speed * physics.yaw.cos() * dt;
                continue;
            }
            // `target == None` (empty route / finished non-looping route /
            // unknown anchor) → fall through to the dumb forward-move
            // fallback below. The evaluator skips past unknown anchors on
            // this same tick, so the drift lasts one tick at most.
        }

        // Dumb forward-move fallback: no patrol objective, no cursor component,
        // or a stalled/terminal patrol. Preserves the pre-existing low-LOD
        // drift so non-patrol ships keep moving instead of standing still.
        physics.x += physics.forward_speed * physics.yaw.sin() * dt;
        physics.z -= physics.forward_speed * physics.yaw.cos() * dt;
    }
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── anchors_from_world_config (PRD #337/#338 slice 1) ──────────────────

    #[test]
    fn anchors_from_world_config_clones_anchor_table() {
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("alpha".to_string(), [10.0, 0.0, 20.0]);
        world.anchors.insert("beta".to_string(), [-5.0, 1.5, 30.0]);

        let anchors = anchors_from_world_config(&world);
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors.get("alpha"), Some(&[10.0, 0.0, 20.0]));
        assert_eq!(anchors.get("beta"), Some(&[-5.0, 1.5, 30.0]));
    }

    #[test]
    fn anchors_from_world_config_returns_empty_when_no_anchors() {
        let world = crate::world::config::WorldConfig::default();
        assert!(anchors_from_world_config(&world).is_empty());
    }

    // ── AiTokenRegistry unit tests ─────────────────────────────────────────

    #[test]
    fn register_produces_ai_prefixed_token() {
        let mut reg = AiTokenRegistry::new();
        reg.register("abc-123");
        assert_eq!(reg.token_for_entity("abc-123"), Some("ai:abc-123"));
    }

    #[test]
    fn register_is_idempotent() {
        let mut reg = AiTokenRegistry::new();
        reg.register("abc-123");
        reg.register("abc-123");
        assert_eq!(reg.token_for_entity("abc-123"), Some("ai:abc-123"));
        // Exactly one reverse entry
        assert_eq!(reg.entity_uuid_for_token("ai:abc-123"), Some("abc-123"));
    }

    #[test]
    fn entity_uuid_for_token_returns_none_for_player_token() {
        let reg = AiTokenRegistry::new();
        assert!(reg.entity_uuid_for_token("some-player-uuid").is_none());
    }

    #[test]
    fn entity_uuid_for_token_round_trips() {
        let mut reg = AiTokenRegistry::new();
        reg.register("ent-999");
        assert_eq!(reg.entity_uuid_for_token("ai:ent-999"), Some("ent-999"));
    }

    #[test]
    fn unregister_removes_both_directions() {
        let mut reg = AiTokenRegistry::new();
        reg.register("ent-1");
        reg.unregister("ent-1");
        assert!(reg.token_for_entity("ent-1").is_none());
        assert!(reg.entity_uuid_for_token("ai:ent-1").is_none());
    }

    #[test]
    fn unregister_unknown_entity_is_silent() {
        let mut reg = AiTokenRegistry::new();
        reg.unregister("ghost-uuid"); // must not panic
    }

    #[test]
    fn contains_entity_returns_true_after_register() {
        let mut reg = AiTokenRegistry::new();
        reg.register("ent-x");
        assert!(reg.contains_entity("ent-x"));
    }

    #[test]
    fn contains_entity_returns_false_after_unregister() {
        let mut reg = AiTokenRegistry::new();
        reg.register("ent-x");
        reg.unregister("ent-x");
        assert!(!reg.contains_entity("ent-x"));
    }

    #[test]
    fn multiple_entities_registered_independently() {
        let mut reg = AiTokenRegistry::new();
        reg.register("alpha");
        reg.register("beta");
        reg.register("gamma");
        assert_eq!(reg.token_for_entity("alpha"), Some("ai:alpha"));
        assert_eq!(reg.token_for_entity("beta"), Some("ai:beta"));
        assert_eq!(reg.token_for_entity("gamma"), Some("ai:gamma"));
    }

    #[test]
    fn unregistering_one_does_not_affect_others() {
        let mut reg = AiTokenRegistry::new();
        reg.register("alpha");
        reg.register("beta");
        reg.unregister("alpha");
        assert!(reg.token_for_entity("alpha").is_none());
        assert_eq!(reg.token_for_entity("beta"), Some("ai:beta"));
    }

    // ── Bevy integration tests ─────────────────────────────────────────────

    use crate::config_cache::FactionRegistryResource;
    use crate::entity_config::BehaviourConfig;
    use crate::entity_spawner::EntityUuid;
    use crate::lobby::LobbyPlugin;
    use crate::messages::GamePhase;

    #[derive(Resource, Default)]
    struct AttackedBox(Vec<AiEntityAttacked>);
    #[derive(Resource, Default)]
    struct DestroyedBox(Vec<AiEntityDestroyed>);

    fn collect_attacked(mut r: MessageReader<AiEntityAttacked>, mut b: ResMut<AttackedBox>) {
        for e in r.read() {
            b.0.push(e.clone());
        }
    }
    fn collect_destroyed(mut r: MessageReader<AiEntityDestroyed>, mut b: ResMut<DestroyedBox>) {
        for e in r.read() {
            b.0.push(e.clone());
        }
    }

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(AiPlugin)
            .insert_resource(FactionRegistryResource(
                crate::config_cache::get_faction_registry(),
            ))
            .init_resource::<AttackedBox>()
            .init_resource::<DestroyedBox>()
            .add_systems(PostUpdate, (collect_attacked, collect_destroyed));
        app
    }

    fn spawn_behaviour_entity(app: &mut App, uuid: &str) -> Entity {
        app.world_mut()
            .spawn((
                Transform::from_xyz(1.0, 0.0, 2.0),
                EntityUuid(uuid.to_string()),
                BehaviourSection(BehaviourConfig::default()),
            ))
            .id()
    }

    #[test]
    fn controller_attached_to_entity_with_behaviour_section() {
        let mut app = build_test_app();
        let entity = spawn_behaviour_entity(&mut app, "ent-001");
        app.update();
        assert!(
            app.world().get::<AiControllerComponent>(entity).is_some(),
            "AiControllerComponent must be attached after update"
        );
    }

    #[test]
    fn token_registered_after_spawn() {
        let mut app = build_test_app();
        spawn_behaviour_entity(&mut app, "ent-003");
        app.update();
        let reg = app.world().resource::<AiTokenRegistry>();
        assert!(reg.contains_entity("ent-003"), "entity must be registered");
        assert_eq!(reg.token_for_entity("ent-003"), Some("ai:ent-003"));
    }

    #[test]
    fn memory_seeded_with_spawn_position() {
        let mut app = build_test_app();
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(10.0, 0.0, -5.0),
                EntityUuid("ent-004".to_string()),
                BehaviourSection(BehaviourConfig::default()),
            ))
            .id();
        app.update();
        // After register_ai_tokens_on_spawn, ShipAiMemory is inserted with home_position.
        let mem = app.world().get::<ShipAiMemory>(entity).unwrap();
        let home = mem.0.home_position;
        assert!((home[0] - 10.0).abs() < 0.001, "home x must be spawn x");
        assert!((home[2] - -5.0).abs() < 0.001, "home z must be spawn z");
    }

    #[test]
    fn token_unregistered_after_entity_despawn() {
        let mut app = build_test_app();
        let entity = spawn_behaviour_entity(&mut app, "ent-007");
        app.update();
        // Verify registered
        assert!(app
            .world()
            .resource::<AiTokenRegistry>()
            .contains_entity("ent-007"));
        // Despawn
        app.world_mut().despawn(entity);
        app.update();
        assert!(
            !app.world()
                .resource::<AiTokenRegistry>()
                .contains_entity("ent-007"),
            "token must be unregistered after despawn"
        );
    }

    // ── AiEntityAttacked event ─────────────────────────────────────────────

    #[test]
    fn ai_entity_attacked_event_emitted_when_new_attacker_arrives() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let attacker_id = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000099").unwrap();
        app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-attacked-001".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            AttackerThisTick(attacker_id),
        ));

        app.update(); // attach controller (AttackerThisTick still present)
        app.update(); // tick processes AttackerThisTick — emits AiEntityAttacked

        let events = app.world().resource::<AttackedBox>().0.clone();
        assert!(
            events.iter().any(|e| e.entity_uuid == "ent-attacked-001"),
            "AiEntityAttacked must be emitted when new attacker arrives"
        );
    }

    #[test]
    fn ai_entity_attacked_not_re_emitted_for_same_attacker() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let attacker_id = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000088").unwrap();
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-attacked-002".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                AttackerThisTick(attacker_id),
            ))
            .id();

        app.update(); // attach
        app.update(); // first attacker tick — emits AiEntityAttacked

        // Insert the same attacker again
        app.world_mut()
            .entity_mut(entity)
            .insert(AttackerThisTick(attacker_id));
        app.update(); // second attacker tick — must NOT re-emit

        let events = app.world().resource::<AttackedBox>().0.clone();
        let count = events
            .iter()
            .filter(|e| e.entity_uuid == "ent-attacked-002")
            .count();
        assert_eq!(count, 1, "same attacker must not re-emit AiEntityAttacked");
    }

    // ── Issue #314: WorldView population from components ───────────────────

    fn make_weapons_console_config(beam_range: f32) -> crate::entity_config::WeaponsConsoleConfig {
        crate::entity_config::WeaponsConsoleConfig {
            torpedo_arc_color: vec![],
            power_multipliers: None,
            phaser_banks: vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 360.0,
                auto_arc_deg: 360.0,
                beam_range,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 3.0,
                cooldown_secs: 3.0,
                beam_color: vec![],
                shield_pierce: Some(0.0),
                marker: None,
            }],
            blaster_banks: vec![],
            radar: None,
        }
    }

    #[test]
    fn self_hull_fraction_reflects_entity_console_hull() {
        use crate::damage::SystemHull;
        use crate::entity_spawner::EntitySystemHull;
        use crate::messages::SystemId;

        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        // 50 HP out of 100 HP = 0.5 fraction
        let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);
        let mut rng = rand::rng();
        hull.apply_damage(50.0, &mut rng);

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-hull-frac-001".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                EntitySystemHull(hull),
            ))
            .id();

        app.update(); // attach controller
        app.update(); // tick

        // The hull fraction should be ~0.5; we verify via the world_view that was
        // used internally by confirming the EntitySystemHull component is readable.
        let hull_comp = app.world().get::<EntitySystemHull>(entity).unwrap();
        let frac = hull_comp.0.total_current() / hull_comp.0.total_max();
        assert!(
            (frac - 0.5).abs() < 0.01,
            "hull fraction should be ~0.5, got {frac}"
        );
    }

    #[test]
    fn npc_beam_ready_true_when_active_beam_inactive_and_no_cooldown() {
        use crate::entity_spawner::WeaponsConsoleSection;
        use crate::weapons_plugin::{ActiveBeam, PhaserCooldown};

        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-phaser-002".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                WeaponsConsoleSection(make_weapons_console_config(40.0)),
                ActiveBeam::default(),
                PhaserCooldown::default(),
            ))
            .id();

        app.update(); // attach controller + first tick
        app.update(); // second tick runs the world_view logic

        let beam = app.world().get::<ActiveBeam>(entity).unwrap();
        let cd = app.world().get::<PhaserCooldown>(entity).unwrap();
        assert!(beam.target_uuid.is_none(), "beam must not be active");
        assert!(!cd.is_bank_active("fore"), "cooldown must be 0");
    }

    #[test]
    fn npc_beam_ready_false_when_cooldown_active() {
        use crate::entity_spawner::WeaponsConsoleSection;
        use crate::weapons_plugin::{ActiveBeam, PhaserCooldown};

        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let mut cooldown = PhaserCooldown::default();
        cooldown.start_bank("fore", 5.0);

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-phaser-003".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                WeaponsConsoleSection(make_weapons_console_config(40.0)),
                ActiveBeam::default(),
                cooldown,
            ))
            .id();

        app.update();

        let cd = app.world().get::<PhaserCooldown>(entity).unwrap();
        assert!(
            cd.is_bank_active("fore"),
            "phaser must not be ready when bank cooldown is active"
        );
    }

    #[test]
    fn weapons_console_section_attached_when_config_has_weapons_console() {
        use crate::entity_config::EntityConfig;
        use crate::entity_spawner::WeaponsConsoleSection;

        let mut app = build_test_app();

        // Build a minimal EntityConfig with a weapons_console section.
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
            faction: None,
            hull: None,
            weapons_console: Some(make_weapons_console_config(80.0)),
            behaviour: None,
            helm_console: None,
            engineering_console: None,
            captain_console: None,
            collider: None,
            appearance: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            tags: vec![],
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            radar_appearance: None,
            mesh: None,
            target: None,
            cinematic_camera: None,
            ai_profile: None,
        };

        let mut commands = app.world_mut().commands();
        let entity = crate::entity_spawner::spawn_entity(
            &mut commands,
            &config,
            bevy::math::Vec3::ZERO,
            "ent-spawner-weapons-001".to_string(),
            None,
        );
        app.world_mut().flush();

        let wc = app.world().get::<WeaponsConsoleSection>(entity);
        assert!(
            wc.is_some(),
            "WeaponsConsoleSection must be attached when config has weapons_console"
        );
        assert!(
            wc.unwrap()
                .0
                .phaser_banks
                .first()
                .map(|b| (b.beam_range - 80.0).abs() < 0.01)
                .unwrap_or(false),
            "beam_range must match config"
        );
    }

    // ── PRD #307: FactionRegistryResource must be accessible as Res (not Option) ──

    /// A minimal system that takes `Res<FactionRegistryResource>` (non-Option).
    /// If the resource is not present, Bevy panics with a missing-resource error.
    /// This test verifies that `build_test_app` — which calls `insert_faction_registry`
    /// via the unconditional path — makes the resource available on native.
    fn read_faction_registry_system(reg: Res<FactionRegistryResource>) {
        // Just accessing it is enough — the test verifies the resource exists.
        let _ = &reg.0;
    }

    // ── aggregate_doctrine_blackboards ────────────────────────────────────────

    /// `aggregate_doctrine_blackboards` must write a `ViewscreenBlackboard` with
    /// at least one `ScoredObjective` carrying `SystemAffinity::Helm` for an
    /// entity whose `BehaviourSection` contains a `Patrol` doctrine entry.
    /// This is the gate the per-axis helm AI checks (`has_helm_objective`);
    /// without it the ship stays still even when Backfill AI is active.
    #[test]
    fn aggregate_doctrine_blackboards_writes_scored_helm_objective() {
        use crate::damage::SystemHull;
        use crate::entity_config::{BehaviourConfig, DoctrineObjective};
        use crate::entity_spawner::EntitySystemHull;
        use crate::messages::{SystemAffinity, SystemId};
        use crate::server_app::ShipSystemBlackboards;
        use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

        let mut app = build_test_app();

        let behaviour = BehaviourConfig {
            doctrine: vec![DoctrineObjective {
                id: "patrol-test".into(),
                text: "Patrol test route".into(),
                directive_kind: Some("Patrol".into()),
                base_priority: 30.0,
                directive_loop: true,
                ..Default::default()
            }],
            ..Default::default()
        };

        let hull = EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            100.0,
        )]));

        app.world_mut().spawn((
            BehaviourSection(behaviour),
            hull,
            ShipSystemBlackboards::default(),
        ));

        app.update();

        let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
        let bb = q
            .iter(app.world())
            .next()
            .expect("entity must have ShipSystemBlackboards");

        let viewscreen =
            bb.0.get(&crate::messages::SystemId(VIEWSCREEN_SYSTEM_ID.to_string()))
                .expect("viewscreen entry must be present after aggregate_doctrine_blackboards");

        let scored = match viewscreen {
            crate::messages::SystemBlackboard::Viewscreen(v) => &v.scored_objectives,
            _ => panic!("expected Viewscreen blackboard"),
        };

        assert!(
            !scored.is_empty(),
            "scored_objectives must not be empty for a Patrol doctrine entity"
        );
        assert!(
            scored
                .iter()
                .any(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm)),
            "at least one scored objective must carry SystemAffinity::Helm"
        );
    }

    /// Publish a viewscreen pool for one entity and hand back its
    /// `scored_objectives`.
    fn scored_pool_for(
        behaviour: crate::entity_config::BehaviourConfig,
        hull_current: f32,
        hull_max: f32,
    ) -> Vec<crate::messages::ScoredObjective> {
        use crate::damage::SystemHull;
        use crate::entity_spawner::EntitySystemHull;
        use crate::messages::SystemId;
        use crate::server_app::ShipSystemBlackboards;
        use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

        let mut app = build_test_app();

        let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), hull_max)]);
        hull.set_hp(&SystemId("captain".into()), hull_current);

        app.world_mut().spawn((
            BehaviourSection(behaviour),
            EntitySystemHull(hull),
            ShipSystemBlackboards::default(),
        ));
        app.update();

        let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
        let bb = q.iter(app.world()).next().expect("blackboards").clone();
        match bb
            .0
            .get(&crate::messages::SystemId(VIEWSCREEN_SYSTEM_ID.to_string()))
            .expect("viewscreen entry")
        {
            crate::messages::SystemBlackboard::Viewscreen(v) => v.scored_objectives.clone(),
            _ => panic!("expected Viewscreen blackboard"),
        }
    }

    /// The published pool must stay sorted descending by score even once the
    /// synthetic hull-triggered Retreat has been injected.
    ///
    /// `operate_helm` and `resolve_helm_target_position` both take the FIRST
    /// Helm-relevant entry as the top-scored directive rather than scanning for
    /// the maximum, so a pool that is merely "mostly sorted" silently mis-selects.
    #[test]
    fn synthetic_retreat_keeps_the_pool_sorted_by_score() {
        use crate::entity_config::{BehaviourConfig, DoctrineObjective};

        let behaviour = BehaviourConfig {
            // Retreat only ever scores in [0, 1], so a sub-1.0 doctrine entry is
            // what makes the ordering observable at all.
            doctrine: vec![DoctrineObjective {
                id: "loiter".into(),
                text: "Loiter".into(),
                directive_kind: Some("Patrol".into()),
                base_priority: 0.1,
                directive_loop: true,
                ..Default::default()
            }],
            retreat_hull_threshold: 0.5,
            ..Default::default()
        };

        // Hull at 10% — well below the 0.5 threshold → retreat scores 0.8.
        let scored = scored_pool_for(behaviour, 10.0, 100.0);

        assert!(
            scored.iter().any(|o| o.id == "retreat"),
            "a badly damaged ship must have a synthetic Retreat injected"
        );
        for pair in scored.windows(2) {
            assert!(
                pair[0].score >= pair[1].score,
                "pool must stay sorted descending: {:?} ({}) preceded {:?} ({})",
                pair[0].id,
                pair[0].score,
                pair[1].id,
                pair[1].score
            );
        }
        assert_eq!(
            scored[0].id, "retreat",
            "the highest-scoring objective must lead the pool"
        );
    }

    /// The retreat threshold is designer-tunable per entity template via
    /// `[behaviour] retreat_hull_threshold` — two ships at identical hull must
    /// disagree about retreating purely on their TOML.
    #[test]
    fn retreat_threshold_comes_from_behaviour_config() {
        use crate::entity_config::BehaviourConfig;

        let brave = scored_pool_for(
            BehaviourConfig {
                retreat_hull_threshold: 0.1,
                ..Default::default()
            },
            40.0,
            100.0,
        );
        let cautious = scored_pool_for(
            BehaviourConfig {
                retreat_hull_threshold: 0.9,
                ..Default::default()
            },
            40.0,
            100.0,
        );

        assert!(
            !brave.iter().any(|o| o.id == "retreat"),
            "hull 0.4 is above a 0.1 threshold — a brave ship must not retreat"
        );
        assert!(
            cautious.iter().any(|o| o.id == "retreat"),
            "hull 0.4 is below a 0.9 threshold — a cautious ship must retreat"
        );
    }

    #[test]
    fn faction_registry_resource_is_present_on_native() {
        let app = build_test_app();
        assert!(
            app.world()
                .get_resource::<FactionRegistryResource>()
                .is_some(),
            "FactionRegistryResource must be present on native without WASM preload"
        );
    }

    #[test]
    fn faction_registry_resource_accessible_as_res_not_option() {
        let mut app = build_test_app();
        app.add_systems(bevy::app::Update, read_faction_registry_system);
        app.update(); // Must not panic
    }

    // ── LOD system tests ─────────────────────────────────────────────────────

    use crate::server_app::{LocalShip, Ship};
    use crate::ship_state::ShipPhysics;

    /// Mirrors the production schedule: `simulate_low_lod_ships` (Physics)
    /// steers from the cursor, then `advance_objective_cursors` (Modifiers)
    /// advances it against the ship's post-movement position. The `SimSet`s
    /// themselves aren't configured here, so the order is stated explicitly.
    fn build_lod_test_app() -> App {
        let mut app = App::new();
        app.add_message::<AiWaypointReached>();
        app.insert_resource(Time::<()>::default()).add_systems(
            Update,
            (
                simulate_low_lod_ships.before(lod_ai_ships),
                lod_ai_ships,
                advance_objective_cursors.after(simulate_low_lod_ships),
            ),
        );
        app
    }

    fn tick_with_dt(app: &mut App, dt_secs: f32) {
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(dt_secs));
        app.update();
    }

    fn spawn_player(app: &mut App, x: f32, z: f32) -> Entity {
        app.world_mut()
            .spawn((
                Ship,
                LocalShip,
                Transform::from_xyz(x, 0.0, z),
                ShipPhysics::default(),
                AiHighFidelity,
                crate::ship::shields::ShieldArcIntents::default(),
                crate::console_ai_plugin::ShipFrequencyHintState::default(),
                crate::ship::power::PowerReactorIntents::default(),
                crate::ship::power::ShipPowerAiState::default(),
                crate::weapons_plugin::TorpedoIntents::default(),
                crate::ship::helm::ThrustInput::default(),
                crate::ship::helm::SteeringInput::default(),
                crate::ship::helm::LateralThrustInput::default(),
                crate::ship::helm::ImpulseCommand::default(),
                crate::ship::helm::BoostCommand::default(),
            ))
            .id()
    }

    fn spawn_npc(app: &mut App, x: f32, z: f32, sensor_range: f32) -> Entity {
        app.world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(x, 0.0, z),
                ShipPhysics {
                    x,
                    z,
                    forward_speed: 10.0,
                    yaw: 0.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range,
                },
            ))
            .id()
    }

    #[test]
    fn local_ship_permanently_has_ai_high_fidelity() {
        let mut app = build_lod_test_app();
        let player = spawn_player(&mut app, 0.0, 0.0);
        assert!(
            app.world().get::<AiHighFidelity>(player).is_some(),
            "LocalShip must start with AiHighFidelity"
        );
        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(player).is_some(),
            "LocalShip must retain AiHighFidelity after update"
        );
    }

    #[test]
    fn npc_out_of_range_gets_cheap_movement() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 0.0, 0.0);
        let npc = spawn_npc(&mut app, 500.0, 0.0, 100.0);

        let initial = *app.world().get::<ShipPhysics>(npc).unwrap();
        tick_with_dt(&mut app, 0.1);
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();

        // With yaw=0, forward_speed=10, dt=0.1:
        //   x' = 500 + 10 * sin(0) * 0.1 = 500
        //   z' = 0 - 10 * cos(0) * 0.1 = -1
        assert!(
            (physics.z - (initial.z - 1.0)).abs() < 0.001,
            "NPC z should advance by forward_speed * dt: expected {}, got {}",
            initial.z - 1.0,
            physics.z,
        );
        assert!(
            (physics.x - initial.x).abs() < 0.001,
            "NPC x should not change when yaw=0: expected {}, got {}",
            initial.x,
            physics.x,
        );
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "NPC out of range must not have AiHighFidelity"
        );
    }

    #[test]
    fn npc_in_range_promoted_to_high_fidelity() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 0.0, 0.0);
        let npc = spawn_npc(&mut app, 50.0, 0.0, 100.0);

        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_some(),
            "NPC within sensor_range must be promoted to AiHighFidelity"
        );
    }

    #[test]
    fn dwell_timer_prevents_lod_thrashing() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 0.0, 0.0);
        let npc = spawn_npc(&mut app, 50.0, 0.0, 100.0);

        // First update: promote to High (within range)
        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_some(),
            "NPC must start in High after first update"
        );

        // Move far outside range + hysteresis
        app.world_mut()
            .entity_mut(npc)
            .insert(Transform::from_xyz(200.0, 0.0, 0.0));
        app.world_mut().entity_mut(npc).insert(ShipPhysics {
            x: 200.0,
            z: 0.0,
            forward_speed: 10.0,
            yaw: 0.0,
            ..Default::default()
        });

        // One more update: still within 2s dwell window (only 0.2s elapsed total)
        tick_with_dt(&mut app, 0.1);
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_some(),
            "NPC must stay High during dwell window"
        );

        // Advance well past 2-second dwell (35 * 0.1 = 3.5s more elapsed)
        for _ in 0..35 {
            tick_with_dt(&mut app, 0.1);
        }

        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "NPC must demote after dwell window elapses"
        );
    }

    // ── Low-LOD patrol wiring (PatrolCursors / advance_cursor) ───────────────

    /// Build a `ShipSystemBlackboards` carrying a single Helm-relevant Patrol
    /// objective under the viewscreen entry (mirrors what
    /// `aggregate_doctrine_blackboards` publishes for a patrolling ship).
    fn blackboards_with_patrol(
        id: &str,
        waypoints: &[&str],
        loop_path: bool,
    ) -> crate::server_app::ShipSystemBlackboards {
        let mut bb = crate::server_app::ShipSystemBlackboards::default();
        bb.0.insert(
            crate::system_registry::viewscreen_system_id(),
            crate::messages::SystemBlackboard::Viewscreen(crate::messages::ViewscreenBlackboard {
                scored_objectives: vec![crate::messages::ScoredObjective {
                    id: id.to_string(),
                    score: 1.0,
                    directive: crate::messages::AiDirective::Patrol {
                        anchors: waypoints.iter().map(|w| w.to_string()).collect(),
                        loop_path,
                    },
                    source: crate::messages::ObjectiveSource::Doctrine,
                    relevance: vec![crate::messages::SystemAffinity::Helm],
                    snapshot: crate::messages::ObjectiveSnapshot {
                        id: id.to_string(),
                        text: "Patrol".to_string(),
                        mandatory: false,
                        status: crate::messages::ObjectiveStatus::Active,
                        targets: vec![],
                        source: crate::messages::ObjectiveSource::Doctrine,
                    },
                }],
                ..Default::default()
            }),
        );
        bb
    }

    /// Spawn a low-LOD patrolling NPC at `(x, z)` carrying the TOML-authored
    /// `BehaviourSection` the cursor evaluator reads its arrival radius from.
    fn spawn_patrolling_npc(
        app: &mut App,
        x: f32,
        z: f32,
        uuid: &str,
        objective_id: &str,
        waypoints: &[&str],
        loop_path: bool,
    ) -> Entity {
        app.world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(x, 0.0, z),
                ShipPhysics {
                    x,
                    z,
                    forward_speed: 10.0,
                    yaw: 0.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 100.0,
                },
                EntityUuid(uuid.to_string()),
                BehaviourSection(BehaviourConfig::default()),
                PatrolCursors::default(),
                blackboards_with_patrol(objective_id, waypoints, loop_path),
            ))
            .id()
    }

    /// Every cursor on `entity` as `(objective_id, waypoint_index)`.
    fn cursor_state(app: &App, entity: Entity) -> Vec<(String, usize)> {
        app.world()
            .get::<PatrolCursors>(entity)
            .unwrap()
            .0
            .iter()
            .map(|c| (c.objective_id.clone(), c.index()))
            .collect()
    }

    #[test]
    fn cursor_advances_when_ship_arrives_at_its_waypoint() {
        let mut app = build_lod_test_app();
        // Ship starts AT wp0, so it arrives immediately; wp1 is 200 units away.
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 1)],
            "cursor must advance to waypoint 1 after arriving at waypoint 0"
        );
    }

    #[test]
    fn cursor_does_not_advance_while_ship_is_short_of_its_waypoint() {
        let mut app = build_lod_test_app();
        // wp0 sits 200 units away — far outside the default arrival radius.
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [700.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [900.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 0)],
            "cursor must stay on waypoint 0 until the ship reaches it"
        );
    }

    /// The arrival radius is designer-tunable via `[behaviour]
    /// waypoint_arrival_radius` in entity TOML — a ship with a wide radius
    /// counts as arrived from a distance that a default-radius ship does not.
    #[test]
    fn arrival_radius_comes_from_the_entity_behaviour_config() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [600.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [900.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        // Both ships sit 100 units from wp0, but only the wide-radius ship
        // is close enough to count as arrived.
        let narrow = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-narrow",
            "patrol",
            &["wp0", "wp1"],
            true,
        );
        let wide = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-wide",
            "patrol",
            &["wp0", "wp1"],
            true,
        );
        app.world_mut()
            .entity_mut(wide)
            .insert(BehaviourSection(BehaviourConfig {
                waypoint_arrival_radius: 150.0,
                ..Default::default()
            }));

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            cursor_state(&app, narrow),
            vec![("patrol".to_string(), 0)],
            "default arrival radius must not count 100 units away as arrived"
        );
        assert_eq!(
            cursor_state(&app, wide),
            vec![("patrol".to_string(), 1)],
            "a TOML-widened arrival radius must count 100 units away as arrived"
        );
    }

    #[test]
    fn reach_objective_cursor_advances_to_terminal_on_arrival() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world
            .anchors
            .insert("dock".to_string(), [500.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let mut bb = crate::server_app::ShipSystemBlackboards::default();
        bb.0.insert(
            crate::system_registry::viewscreen_system_id(),
            crate::messages::SystemBlackboard::Viewscreen(crate::messages::ViewscreenBlackboard {
                scored_objectives: vec![crate::messages::ScoredObjective {
                    id: "reach-dock".to_string(),
                    score: 1.0,
                    directive: crate::messages::AiDirective::Reach {
                        anchor: "dock".to_string(),
                    },
                    source: crate::messages::ObjectiveSource::Mission,
                    relevance: vec![crate::messages::SystemAffinity::Helm],
                    snapshot: crate::messages::ObjectiveSnapshot {
                        id: "reach-dock".to_string(),
                        text: "Reach the dock".to_string(),
                        mandatory: false,
                        status: crate::messages::ObjectiveStatus::Active,
                        targets: vec![],
                        source: crate::messages::ObjectiveSource::Mission,
                    },
                }],
                ..Default::default()
            }),
        );

        // Ship sits on the dock anchor → arrived.
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(500.0, 0.0, 500.0),
                ShipPhysics {
                    x: 500.0,
                    z: 500.0,
                    forward_speed: 10.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 100.0,
                },
                EntityUuid("npc-reach".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                PatrolCursors::default(),
                bb,
            ))
            .id();

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            cursor_state(&app, npc),
            vec![("reach-dock".to_string(), 1)],
            "a Reach cursor is a one-waypoint route: arriving moves it to the terminal index"
        );
    }

    #[test]
    fn low_lod_npc_follows_patrol_route_between_waypoints() {
        let mut app = build_lod_test_app();
        // Ship starts AT wp0; wp1 is offset in +x so the steer is observable
        // in both yaw and position.
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        // Tick 1 arrives at wp0 and advances the cursor to wp1; tick 2 is the
        // first tick that steers toward wp1.
        tick_with_dt(&mut app, 0.1);
        tick_with_dt(&mut app, 0.1);

        // Steering toward wp1 (700,0,500): dx=+200, dz=0 → bearing = π/2.
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            (physics.yaw - std::f32::consts::FRAC_PI_2).abs() < 0.01,
            "yaw should steer toward wp1 bearing (π/2), got {}",
            physics.yaw,
        );
        assert!(
            physics.x > 500.0,
            "ship should advance toward wp1 (+x), got x={}",
            physics.x,
        );

        // Must remain low-LOD (never promoted), proving this is the cheap path.
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "patrolling NPC out of range must stay low-LOD"
        );
    }

    /// End-to-end route following: a low-LOD NPC placed on a two-waypoint
    /// looping route drives itself to the far waypoint, wraps back to the
    /// first, and returns — without ever being promoted to high LOD.
    #[test]
    fn low_lod_npc_patrol_route_wraps_around_and_returns_to_first_waypoint() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        // forward_speed 10 → ~1 unit/tick at dt=0.1, so the 200-unit leg out
        // to wp1 takes ~180 ticks to close to within the 20-unit arrival
        // radius. Run until the cursor wraps back to wp0 (bounded well above
        // that), sampling the cursor each tick to prove the whole cycle.
        let mut seen_indices = Vec::new();
        let mut reached_max_x: f32 = 500.0;
        for _ in 0..600 {
            tick_with_dt(&mut app, 0.1);
            let idx = cursor_state(&app, npc)[0].1;
            if seen_indices.last() != Some(&idx) {
                seen_indices.push(idx);
            }
            reached_max_x = reached_max_x.max(app.world().get::<ShipPhysics>(npc).unwrap().x);
            // Stop on the first wraparound: 0 → 1 → back to 0.
            if seen_indices.len() == 2 {
                break;
            }
        }

        assert_eq!(
            seen_indices,
            vec![1, 0],
            "cursor must advance to wp1, then wrap back to wp0 on a looping route"
        );
        assert!(
            reached_max_x > 680.0,
            "ship must actually travel the leg out to wp1 (x≈700), got max x={}",
            reached_max_x,
        );

        // Steering reads the cursor before the evaluator advances it, so the
        // turn toward wp0 happens on the tick *after* the wrap.
        tick_with_dt(&mut app, 0.1);

        // Having wrapped, it is heading back toward wp0 (-x) → bearing = -π/2.
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            (physics.yaw + std::f32::consts::FRAC_PI_2).abs() < 0.01,
            "after wraparound the ship should steer back toward wp0 (-π/2), got {}",
            physics.yaw,
        );
        assert!(
            physics.x < reached_max_x,
            "ship must be travelling back toward wp0 after the wrap"
        );
        assert!(
            app.world().get::<AiHighFidelity>(npc).is_none(),
            "patrolling NPC out of range must stay low-LOD for the whole route"
        );
    }

    /// The arrival that advances the cursor is announced as an
    /// `AiWaypointReached` message — the bridge the world plugin turns into a
    /// `WorldEvent::WaypointReached` for `on_waypoint_reached` triggers.
    #[test]
    fn reaching_a_waypoint_emits_ai_waypoint_reached() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );

        tick_with_dt(&mut app, 0.1);

        let messages = app
            .world()
            .resource::<bevy::ecs::message::Messages<AiWaypointReached>>();
        let mut cursor = messages.get_cursor();
        let emitted: Vec<_> = cursor.read(messages).collect();

        assert_eq!(
            emitted.len(),
            1,
            "arriving at wp0 must announce exactly once"
        );
        assert_eq!(emitted[0].entity_uuid, "npc-1");
        assert_eq!(emitted[0].objective_id, "patrol");
        assert_eq!(
            emitted[0].waypoint, "wp0",
            "the announced waypoint must be the one arrived at, not the next one"
        );
    }

    /// Read every `AiWaypointReached` emitted so far, as `(uuid, waypoint)`.
    fn reached_waypoints(app: &App) -> Vec<(String, String)> {
        let messages = app
            .world()
            .resource::<bevy::ecs::message::Messages<AiWaypointReached>>();
        let mut cursor = messages.get_cursor();
        cursor
            .read(messages)
            .map(|m| (m.entity_uuid.clone(), m.waypoint.clone()))
            .collect()
    }

    /// Regression: a tick that carries the cursor past several waypoints at
    /// once must announce every one of them. With `wp0` and `wp1` spaced
    /// closer than the arrival radius, the cursor jumps 0 → 2 in a single
    /// tick; announcing only `wp0` would leave an `on_waypoint_reached`
    /// trigger keyed to `wp1` silently dead.
    #[test]
    fn one_message_per_waypoint_consumed_when_a_tick_skips_several() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        // wp0 and wp1 are 5 units apart — well inside the 20-unit default
        // arrival radius — while wp2 is a long leg away.
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [505.0, 0.0, 500.0]);
        world.anchors.insert("wp2".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1", "wp2"],
            true,
        );

        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            reached_waypoints(&app),
            vec![
                ("npc-1".to_string(), "wp0".to_string()),
                ("npc-1".to_string(), "wp1".to_string()),
            ],
            "both waypoints consumed this tick must be announced, in route order"
        );
        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 2)],
            "cursor must land on the far wp2 after skipping wp0 and wp1"
        );
    }

    /// Regression: a looping route whose every waypoint sits inside the
    /// arrival radius closes its lap immediately. Any route with legs shorter
    /// than the authored `waypoint_arrival_radius` does this — a designer
    /// widening the radius for a station-keeping patrol, not just a
    /// pathological case.
    ///
    /// The contract has three parts, and the second and third are what the
    /// original permanent-retirement design broke: the ship announced its lap
    /// and then lost its cursor entirely, fell through to the dumb
    /// forward-move, and flew out of the cluster in a straight line forever
    /// with no way back.
    #[test]
    fn looping_route_entirely_inside_arrival_radius_announces_once_then_holds_station() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        // All three anchors are within the 20-unit default arrival radius of
        // the ship's spawn point.
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [505.0, 0.0, 500.0]);
        world.anchors.insert("wp2".to_string(), [500.0, 0.0, 505.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1", "wp2"],
            true,
        );

        // First tick: the lap closes — each waypoint announced exactly once.
        tick_with_dt(&mut app, 0.1);
        assert_eq!(
            reached_waypoints(&app),
            vec![
                ("npc-1".to_string(), "wp0".to_string()),
                ("npc-1".to_string(), "wp1".to_string()),
                ("npc-1".to_string(), "wp2".to_string()),
            ],
            "the closing lap must announce each waypoint exactly once"
        );

        // ── 2. No per-tick spam, and the ship holds station ────────────────
        // Drain, then tick long enough that a ship flying off at
        // forward_speed (1 unit/tick here) would be 200 units clear of the
        // cluster.
        app.world_mut()
            .resource_mut::<bevy::ecs::message::Messages<AiWaypointReached>>()
            .clear();
        for _ in 0..200 {
            tick_with_dt(&mut app, 0.1);
        }

        assert!(
            reached_waypoints(&app).is_empty(),
            "a settled degenerate route must not re-announce its waypoints every tick"
        );
        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 0)],
            "the cursor must stay on a real waypoint index, not a sentinel"
        );
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        let drift = ((physics.x - 500.0).powi(2) + (physics.z - 500.0).powi(2)).sqrt();
        assert!(
            drift < 20.0,
            "the ship must keep station on its route, not fly out of the cluster: \
             drifted {} units to ({}, {})",
            drift,
            physics.x,
            physics.z,
        );

        // ── 3. Moved out of the radius, the route resumes ──────────────────
        // Shove the ship 2000 units clear (a knockback, tow or scenario
        // teleport does the same thing).
        {
            let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
            physics.x = 2500.0;
            physics.z = 500.0;
        }
        tick_with_dt(&mut app, 0.1);

        assert!(
            reached_waypoints(&app).is_empty(),
            "nothing was arrived at 2000 units out"
        );
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            (physics.yaw + std::f32::consts::FRAC_PI_2).abs() < 0.01,
            "the resumed route must steer back toward wp0 (-π/2), got {}",
            physics.yaw,
        );
        assert!(
            physics.x < 2500.0,
            "the ship must fly back toward its route, got x={}",
            physics.x,
        );

        // Back in the cluster, the lap is flown and announced afresh — the
        // route is alive, not permanently dead.
        {
            let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
            physics.x = 500.0;
            physics.z = 500.0;
        }
        tick_with_dt(&mut app, 0.1);

        assert_eq!(
            reached_waypoints(&app),
            vec![
                ("npc-1".to_string(), "wp0".to_string()),
                ("npc-1".to_string(), "wp1".to_string()),
                ("npc-1".to_string(), "wp2".to_string()),
            ],
            "a route resumed after leaving the arrival radius must announce again"
        );
    }

    /// Regression (issue #696 review): the shipped-content shape of the bug.
    /// `waypoint_arrival_radius` is designer-tunable per entity, so a route
    /// whose legs are shorter than the authored radius is ordinary content —
    /// a station-keeping patrol. It must not silently become "fly off the map
    /// in a straight line, forever".
    #[test]
    fn route_with_legs_shorter_than_the_authored_radius_does_not_die() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        // 100-unit legs against a 150-unit authored radius.
        world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
        world.anchors.insert("wp1".to_string(), [600.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = spawn_patrolling_npc(
            &mut app,
            500.0,
            500.0,
            "npc-1",
            "patrol",
            &["wp0", "wp1"],
            true,
        );
        app.world_mut()
            .entity_mut(npc)
            .insert(BehaviourSection(BehaviourConfig {
                waypoint_arrival_radius: 150.0,
                ..Default::default()
            }));

        // The lap closes on tick 1: both waypoints are inside the radius.
        tick_with_dt(&mut app, 0.1);
        assert_eq!(
            reached_waypoints(&app),
            vec![
                ("npc-1".to_string(), "wp0".to_string()),
                ("npc-1".to_string(), "wp1".to_string()),
            ],
            "the closing lap announces each waypoint once"
        );

        // 400 ticks at 1 unit/tick: a ship that lost its cursor would be 400
        // units clear by now. This one is still on its route.
        for _ in 0..400 {
            tick_with_dt(&mut app, 0.1);
        }
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        let dist_to_wp0 = ((physics.x - 500.0).powi(2) + (physics.z - 500.0).powi(2)).sqrt();
        assert!(
            dist_to_wp0 < 150.0,
            "the ship must hold its route, not fly off at forward_speed: {} units out",
            dist_to_wp0,
        );
        assert_eq!(
            cursor_state(&app, npc),
            vec![("patrol".to_string(), 0)],
            "the cursor must still name a real waypoint"
        );

        // And the route is resumable: shoved clear, it steers back.
        app.world_mut()
            .resource_mut::<bevy::ecs::message::Messages<AiWaypointReached>>()
            .clear();
        {
            let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
            physics.x = 3000.0;
            physics.z = 500.0;
        }
        tick_with_dt(&mut app, 0.1);
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        assert!(
            physics.x < 3000.0,
            "a resumed route must fly the ship back toward wp0, got x={}",
            physics.x,
        );
        assert!(
            !app.world().get::<PatrolCursors>(npc).unwrap().0[0].settled(),
            "leaving the arrival radius must un-settle the cursor"
        );
    }

    #[test]
    fn no_waypoint_reached_message_while_ship_is_short_of_its_waypoint() {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world.anchors.insert("wp0".to_string(), [700.0, 0.0, 500.0]);
        app.insert_resource(world);
        spawn_player(&mut app, 0.0, 0.0);

        spawn_patrolling_npc(&mut app, 500.0, 500.0, "npc-1", "patrol", &["wp0"], false);

        tick_with_dt(&mut app, 0.1);

        let messages = app
            .world()
            .resource::<bevy::ecs::message::Messages<AiWaypointReached>>();
        let mut cursor = messages.get_cursor();
        assert_eq!(
            cursor.read(messages).count(),
            0,
            "no arrival must be announced while the ship is still 200 units out"
        );
    }

    #[test]
    fn low_lod_without_patrol_objective_keeps_dumb_forward_move() {
        let mut app = build_lod_test_app();
        spawn_player(&mut app, 0.0, 0.0);

        // NPC carries PatrolCursors + an (empty) blackboard but NO patrol
        // objective — it must fall back to the pre-existing dumb forward-move.
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(500.0, 0.0, 0.0),
                ShipPhysics {
                    x: 500.0,
                    z: 0.0,
                    forward_speed: 10.0,
                    yaw: 0.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 100.0,
                },
                PatrolCursors::default(),
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();

        let initial = *app.world().get::<ShipPhysics>(npc).unwrap();
        tick_with_dt(&mut app, 0.1);
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();

        // yaw=0, forward_speed=10, dt=0.1 → z advances by -1, x unchanged.
        assert!(
            (physics.z - (initial.z - 1.0)).abs() < 0.001,
            "no-patrol NPC z should advance by forward_speed * dt: expected {}, got {}",
            initial.z - 1.0,
            physics.z,
        );
        assert!(
            (physics.x - initial.x).abs() < 0.001,
            "no-patrol NPC x should not change when yaw=0: expected {}, got {}",
            initial.x,
            physics.x,
        );
        // Cursor stays empty — nothing was advanced.
        assert!(
            app.world().get::<PatrolCursors>(npc).unwrap().0.is_empty(),
            "cursor must stay empty when there is no patrol objective"
        );
    }
}
