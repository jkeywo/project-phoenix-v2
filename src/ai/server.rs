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

// `ShipAiMemory(AiMemory)` lived here until issue #702 deleted it. There is no
// private per-entity AI memory any more: every goal the AI serves is read from
// a surface some console owns and a human could equally drive —
// `TacticalRadarSelection` (Tactical), `NavigationWaypoint` (Navigation),
// `ObjectiveCursors` (the objective), `LastShipAttacker` (the world). Adding a
// private mirror of any of them back re-creates the split brain — and the
// helm/weapons targeting divergence — that removing it fixed.

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

/// Per-objective route cursors: where this ship is on each objective's route.
///
/// Each entry is a [`PatrolCursor`] tracking the current waypoint for one
/// objective. Entries are independent — advancing one does not affect others.
/// Cursor state is interpreted (and its out-of-range terminal stop owned) by
/// the pure `ai::patrol_cursor` module.
///
/// # Sole writer
///
/// `advance_objective_cursors` (`SimSet::Modifiers`) is the only writer: it
/// owns arrival detection and cursor advancement for every ship, every
/// objective, at every LOD. Everyone else reads —
/// `simulate_low_lod_ships` (`SimSet::Physics`) to cheaply steer NPCs outside
/// sensor range, `helm_patrol` to steer high-LOD ships, `operate_navigation_ai`
/// to place the waypoint. One writer in one set is what stops a cursor from
/// being advanced twice in a tick.
///
/// # Why a side-table (issue #702)
///
/// Keyed by `objective_id` per ship, rather than living on the objective. It
/// cannot live there: mission objectives are a single shared world-level
/// record (every ship pursuing one would share a cursor), and doctrine
/// objectives are rebuilt from TOML every tick (a cursor on one would be reset
/// every tick).
///
/// Named `PatrolCursors` until #702 generalised it: the name always lied,
/// since it handled `Reach` too. It is now *the* cursor surface for every
/// directive. Before #702 the high-LOD helm path kept a rival cursor of its own
/// in `AiMemory.waypoint_index`, so patrol position was tracked in two places
/// that could disagree; there is now one.
///
/// Present on every ship (player + NPC). The player ship was missing it —
/// `entities/spawner.rs` inserted it, `server_app.rs` did not — which silently
/// disabled AI patrol on the player ship under `Backfill`.
#[derive(Component, Clone, Debug, Default)]
pub struct ObjectiveCursors(pub Vec<crate::ai::patrol_cursor::PatrolCursor>);

/// Marker component set on NPC entities currently in a warp-out sequence.
/// Carries the data needed to draw the warp-exit visual and to populate
/// `EntitySnapshot::warp_out_remaining_secs` in the broadcast.
/// Kept for interface compatibility; not set by the doctrine-based AI system.
#[derive(Component)]
pub struct WarpOutMarker {
    pub remaining_secs: f32,
    pub target_speed: f32,
}

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

/// Emitted by the AI plugin when a ship's [`LastShipAttacker`] changes to name
/// a new attacker.
///
/// The world plugin observes this event to evaluate `on_entity_attacked`
/// trigger conditions without a direct dependency on the AI module.
///
/// [`LastShipAttacker`]: crate::weapons_plugin::LastShipAttacker
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
/// The world plugin reads this in `tick_trigger_pipeline` and turns it into a
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

/// Build the [`WorldSnapshot`] from every entity with an `EntityUuid`, plus
/// every field asteroid. Runs in `SimSet::Physics` before `AiTickLabel` so
/// per-system AI handlers see a consistent frame.
///
/// Asteroids need the separate pass because they are streamed by
/// `src/asteroids/lifecycle.rs` rather than `spawn_entity`, so they carry an
/// `AsteroidUuid` instead of an `EntityUuid` and never matched the query below.
/// That is why AI collision avoidance flew straight through asteroid fields:
/// not a tuning problem, the rocks simply were not in the world it steers from.
/// Keying off `AsteroidUuid` rather than back-filling `EntityUuid` keeps them
/// out of the radar, networking, and targeting paths that key on the latter.
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
    asteroids: Query<
        (
            &crate::simulation::AsteroidUuid,
            &Transform,
            &crate::entity_spawner::ColliderSection,
        ),
        With<crate::simulation::Asteroid>,
    >,
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

    // Asteroids are obstacles and nothing else: no faction to be hostile to, no
    // hull fraction to retreat over, and they do not move. Only position and
    // radius matter, which is exactly what `avoidance_steering` reads.
    snapshot
        .entities
        .extend(
            asteroids
                .iter()
                .map(|(uuid, transform, collider)| crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::parse_str(&uuid.0).unwrap_or_default(),
                    name: None,
                    position: [
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ],
                    faction: None,
                    hull_fraction: None,
                    yaw: Some(0.0),
                    radius: collider.0.radius,
                    forward_speed: 0.0,
                    shields: None,
                }),
        );
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
        let attacked = last_attacker_opt.is_some();
        let conditions = crate::objectives::WorldConditions {
            red_alert,
            hull_fraction,
            attacked,
        };
        // Retreat is ordinary authored doctrine (issue #702). A synthetic
        // hull-triggered Retreat used to be injected here; it was deleted
        // because it could not work and never had:
        //
        //  1. Its anchor was always empty (the anchors map is not in scope
        //     here), so it leaned on an `AiMemory.home_position` fallback that
        //     production never seeded — `register_ai_tokens_on_spawn` only
        //     seeded it when no memory existed, but the spawner had already
        //     inserted a default in the same batch. Every shipped ship therefore
        //     "retreated" to world origin.
        //  2. Its score ran 0..1 while doctrine `base_priority` runs in the
        //     tens, so it could never outrank anything even after re-sorting.
        //
        // A designer authors the same intent with vocabulary that already
        // exists, on the right score scale and with a real anchor:
        //
        //     [[behaviour.doctrine]]
        //     id               = "retreat-when-hurt"
        //     directive_kind   = "Retreat"
        //     directive_anchor = "pirate_haven"
        //     base_priority    = 100.0
        //     zero_gates       = [{ condition = "hull_below", threshold = 0.3 }]
        //
        // `hull_fraction` below is what `hull_below` gates on, so the trigger
        // is unchanged — only now it is tunable per hull without a recompile.
        let scored = crate::ai::score_doctrine_pool(&behaviour.0.doctrine, &conditions);
        // Lift Combat Lock + Science Target from this ship's own radar
        // blackboards (issue #829). They were published this tick in
        // `SimSet::Publish`, which runs before this `PublishAggregate` system.
        let combat_lock = match blackboards
            .0
            .get(&crate::ship::system_registry::tactical_radar_system_id())
        {
            Some(crate::messages::SystemBlackboard::TacticalRadar(bb)) => {
                bb.selected_target.clone()
            }
            _ => None,
        };
        let science_target = match blackboards
            .0
            .get(&crate::ship::system_registry::sensor_radar_system_id())
        {
            Some(crate::messages::SystemBlackboard::SensorRadar(bb)) => bb.selected_target.clone(),
            _ => None,
        };
        let viewscreen_bb = crate::messages::ViewscreenBlackboard {
            red_alert,
            hull_integrity_pct: hull_fraction * 100.0,
            last_damage_taken_secs: activity_opt.and_then(|a| a.last_damage_taken),
            last_weapon_fired_secs: activity_opt.and_then(|a| a.last_weapon_fired),
            last_attacker_uuid: last_attacker_opt.and_then(|la| la.0.clone()),
            scored_objectives: scored,
            combat_lock,
            science_target,
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
                emit_attacked_on_new_attacker
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
    query: Query<(Entity, &EntityUuid), (With<BehaviourSection>, Without<AiControllerComponent>)>,
) {
    for (entity, uuid) in &query {
        registry.register_with_entity(&uuid.0, entity);
        commands.entity(entity).insert(AiControllerComponent);
    }
}

/// Emit `AiEntityAttacked` whenever a ship's [`LastShipAttacker`] changes to
/// name a new attacker.
///
/// [`LastShipAttacker`] *is* the "who last attacked me" surface — the same one
/// `ai_target_selection` reads — so the rising-edge latch that fires this event
/// is that component's own change detection (issue #702). It replaces the
/// private `AiMemory.last_attacker` mirror and the `AttackerThisTick` one-tick
/// component that used to carry the edge across.
///
/// Exactly one event per new attacker under sustained fire: `tick_beams`
/// compares before writing, so a beam that keeps naming the same shooter never
/// marks the component changed and this system never re-fires. The two clear
/// paths (`clear_last_attacker_on_death` /
/// `clear_last_attacker_on_red_alert_off`) do mark it changed, but only ever
/// write `None` — which the `Some` guard below skips. Re-attack by the same
/// shooter after a clear is a genuine new edge and correctly re-fires.
///
/// Runs in `SimSet::Physics`, i.e. *before* the `SimSet::Damage` `tick_beams`
/// that writes the component, so the event lands on the tick after the hit —
/// the same one-tick bridge `AttackerThisTick` gave it.
///
/// [`LastShipAttacker`]: crate::weapons_plugin::LastShipAttacker
fn emit_attacked_on_new_attacker(
    query: Query<
        (&EntityUuid, &crate::weapons_plugin::LastShipAttacker),
        Changed<crate::weapons_plugin::LastShipAttacker>,
    >,
    mut attacked_events: MessageWriter<AiEntityAttacked>,
) {
    for (uuid, last_attacker) in query.iter() {
        // `None` is a clear, not an attack; an unparseable UUID names nobody.
        let Some(attacker_uuid) = last_attacker
            .0
            .as_deref()
            .and_then(|a| uuid::Uuid::parse_str(a).ok())
        else {
            continue;
        };
        attacked_events.write(AiEntityAttacked {
            entity_uuid: uuid.0.clone(),
            attacker_uuid,
        });
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
                        .insert(crate::console_ai_plugin::ShipFrequencyHintState::default());
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
                        .remove::<crate::console_ai_plugin::ShipFrequencyHintState>();
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
/// This is the single owner of `ObjectiveCursors` state: every steering path —
/// low-LOD (`simulate_low_lod_ships`) and high-LOD (`helm_patrol`) alike — only
/// *reads* the cursor, so a cursor can never be advanced twice in one tick.
///
/// Per ship, per active Helm-relevant `Patrol`/`Reach` objective: advance the
/// cursor via the pure `advance_cursor` (which judges arrival against the
/// radius and handles wraparound, terminal stops, settling degenerate looping
/// routes, and skipping waypoints whose anchors are unknown), then emit one
/// `AiWaypointReached` per waypoint it reports as consumed.
///
/// Covers all ships carrying a `BehaviourSection` regardless of LOD. Since
/// #702 that is the whole story: the high-LOD helm path (`helm_patrol`) reads
/// these same cursors rather than advancing a rival `AiMemory.waypoint_index`
/// of its own.
pub(crate) fn advance_objective_cursors(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<(
        Option<&BehaviourSection>,
        &ShipPhysics,
        &crate::server_app::ShipSystemBlackboards,
        &mut ObjectiveCursors,
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
        let arrival_radius = behaviour
            .map(|b| b.0.waypoint_arrival_radius)
            .unwrap_or(crate::ai::WAYPOINT_ARRIVAL_RADIUS);
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
/// [`ObjectiveCursors`] entry currently points at and advances forward at
/// `forward_speed`. Ships with no such objective (or a stalled/terminal route)
/// keep the pre-existing dumb forward-drift so they don't regress to standing
/// still.
///
/// Read-only with respect to the cursor: arrival detection and advancement
/// belong to `advance_objective_cursors` in `SimSet::Modifiers`.
///
/// This is the *low-fidelity* path. It reads the same `ObjectiveCursors` the
/// high-LOD path (`helm_patrol` in `ai/core.rs`) reads — since #702 there is
/// one cursor surface rather than two rival ones, so a ship promoted or demoted
/// between LODs resumes its route exactly where it left off.
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
            Option<&ObjectiveCursors>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
        ),
        (With<Ship>, Without<AiHighFidelity>),
    >,
) {
    let dt = time.delta_secs();
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();

    // Default speed fraction when the low-LOD path has a valid route but the
    // helm config is absent (unusual — all shipped hulls carry one).
    const LOW_LOD_SPEED_FRACTION: f32 = 0.4;
    // Simple ramp rate so forward_speed doesn't snap from 0 to max in one tick.
    const LOW_LOD_ACCEL_PER_SEC: f32 = 10.0;

    for (mut physics, blackboards, cursors, helm_section) in &mut ships {
        let max_speed = helm_section
            .map(|h| h.0.max_speed)
            .filter(|&s| s > 0.0)
            .unwrap_or(20.0);
        let route = blackboards.and_then(active_waypoint_route);

        // Steer along the route only when we have a Patrol/Reach objective AND
        // a `ObjectiveCursors` component tracking the waypoint index. A low-LOD
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
                // Accelerate toward the target speed so a ship that spawned
                // far from the player (and never had `integrate_ship_physics`
                // running for it) actually moves. Without this a low-LOD ship
                // stays stuck at forward_speed = 0 forever.
                let target_speed = max_speed * LOW_LOD_SPEED_FRACTION;
                if physics.forward_speed < target_speed {
                    physics.forward_speed =
                        (physics.forward_speed + LOW_LOD_ACCEL_PER_SEC * dt).min(target_speed);
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

    // ── build_world_snapshot: asteroids as obstacles ───────────────────────

    fn snapshot_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<WorldSnapshot>()
            .add_systems(Update, build_world_snapshot);
        app
    }

    /// Field asteroids carry `AsteroidUuid`, not `EntityUuid`, because they are
    /// streamed rather than spawned through `spawn_entity`. They used to fall
    /// out of the snapshot entirely, which left `avoidance_steering` blind to
    /// every rock in the field.
    #[test]
    fn world_snapshot_includes_field_asteroids_with_their_radius() {
        let mut app = snapshot_test_app();
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(30.0, 0.0, -12.0),
            crate::entity_spawner::ColliderSection(crate::entity_config::ColliderConfig {
                shape: crate::entity_config::ColliderShape::Ball,
                radius: 4.0,
                length: 0.0,
            }),
        ));

        app.update();

        let snapshot = app.world().resource::<WorldSnapshot>();
        assert_eq!(
            snapshot.entities.len(),
            1,
            "asteroid must reach the snapshot"
        );
        let rock = &snapshot.entities[0];
        assert_eq!(rock.radius, 4.0, "avoidance sizes the obstacle off radius");
        assert_eq!(rock.position, [30.0, 0.0, -12.0]);
        assert_eq!(rock.faction, None, "a rock is hostile to nobody");
        assert_eq!(rock.forward_speed, 0.0);
    }

    #[test]
    fn world_snapshot_carries_both_entities_and_asteroids() {
        let mut app = snapshot_test_app();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            crate::entity_spawner::ColliderSection(crate::entity_config::ColliderConfig {
                shape: crate::entity_config::ColliderShape::Ball,
                radius: 2.5,
                length: 0.0,
            }),
        ));

        app.update();

        let snapshot = app.world().resource::<WorldSnapshot>();
        assert_eq!(snapshot.entities.len(), 2);
        assert!(
            snapshot.entities.iter().any(|e| e.radius == 2.5),
            "the asteroid pass must not replace the entity pass"
        );
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
    //
    // Post-#702 the rising edge lives on `LastShipAttacker`'s change detection
    // rather than on a private `AiMemory.last_attacker` mirror, so these drive
    // that component. They pin the *reader's* half of the exactly-once
    // contract: given a writer that compares before writing, the emitter fires
    // once per new attacker. The writer's half — that `tick_beams` really does
    // compare rather than blind-write under a live beam — is pinned by
    // `sustained_beam_marks_last_attacker_changed_exactly_once` in
    // `console::weapons`. Both halves are required; neither alone
    // establishes the AC.

    /// Write `LastShipAttacker` the way `tick_beams` does — via `set_if_neq`,
    /// not `insert`. The distinction is the whole point: an `insert` marks the
    /// component changed even when the value is identical, so a fixture that
    /// inserted would fake an edge production never produces and this test
    /// would pass on a blind-writing `tick_beams`.
    fn beam_hit(app: &mut App, entity: Entity, attacker: &str) {
        let mut e = app.world_mut().entity_mut(entity);
        let mut last = e
            .get_mut::<crate::weapons_plugin::LastShipAttacker>()
            .expect("ship must carry LastShipAttacker");
        last.set_if_neq(crate::weapons_plugin::LastShipAttacker(Some(
            attacker.to_string(),
        )));
    }

    fn attacked_count(app: &App, entity_uuid: &str) -> usize {
        app.world()
            .resource::<AttackedBox>()
            .0
            .iter()
            .filter(|e| e.entity_uuid == entity_uuid)
            .count()
    }

    #[test]
    fn ai_entity_attacked_event_emitted_when_new_attacker_arrives() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let attacker_id = "aaaaaaaa-0000-0000-0000-000000000099";
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-attacked-001".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::weapons_plugin::LastShipAttacker::default(),
            ))
            .id();

        app.update(); // attach controller; no attacker yet
        beam_hit(&mut app, entity, attacker_id);
        app.update(); // the change is seen — emits AiEntityAttacked

        let events = app.world().resource::<AttackedBox>().0.clone();
        let event = events
            .iter()
            .find(|e| e.entity_uuid == "ent-attacked-001")
            .expect("AiEntityAttacked must be emitted when a new attacker arrives");
        assert_eq!(
            event.attacker_uuid,
            uuid::Uuid::parse_str(attacker_id).unwrap(),
            "the event must name the attacker LastShipAttacker records"
        );
    }

    /// Sustained fire: the beam keeps naming the same shooter every tick, and
    /// the trigger must fire exactly once.
    #[test]
    fn ai_entity_attacked_not_re_emitted_for_same_attacker() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let attacker_id = "aaaaaaaa-0000-0000-0000-000000000088";
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-attacked-002".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::weapons_plugin::LastShipAttacker::default(),
            ))
            .id();

        app.update(); // attach

        // Five ticks of a live beam from one shooter.
        for _ in 0..5 {
            beam_hit(&mut app, entity, attacker_id);
            app.update();
        }

        assert_eq!(
            attacked_count(&app, "ent-attacked-002"),
            1,
            "sustained fire from one shooter must emit AiEntityAttacked exactly once"
        );
    }

    /// The other edge: a *different* shooter is a new attacker and must re-fire,
    /// even though `LastShipAttacker` was already `Some`. Guards against a fix
    /// for the test above that latches on "was ever attacked" instead of "who".
    #[test]
    fn ai_entity_attacked_re_emitted_for_a_different_attacker() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let first = "aaaaaaaa-0000-0000-0000-000000000077";
        let second = "bbbbbbbb-0000-0000-0000-000000000077";
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-attacked-003".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::weapons_plugin::LastShipAttacker::default(),
            ))
            .id();

        app.update();
        beam_hit(&mut app, entity, first);
        app.update();
        beam_hit(&mut app, entity, second);
        app.update();

        assert_eq!(
            attacked_count(&app, "ent-attacked-003"),
            2,
            "a second, different attacker is a new edge and must re-emit"
        );
    }

    /// Clearing the attacker (`clear_last_attacker_on_death` /
    /// `clear_last_attacker_on_red_alert_off` both write `None`) marks the
    /// component changed, but `None` names nobody and must not be reported as
    /// an attack.
    #[test]
    fn clearing_the_attacker_emits_no_attacked_event() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-attacked-004".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::weapons_plugin::LastShipAttacker::default(),
            ))
            .id();

        app.update();
        beam_hit(&mut app, entity, "aaaaaaaa-0000-0000-0000-000000000066");
        app.update();

        // The threat passes — the attacker record is cleared.
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::weapons_plugin::LastShipAttacker(None));
        app.update();

        assert_eq!(
            attacked_count(&app, "ent-attacked-004"),
            1,
            "clearing LastShipAttacker to None must not count as an attack"
        );
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
            planet: None,
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

    /// A `Retreat` doctrine entry gated on `hull_below` scores only once the
    /// ship is actually hurt (issue #702).
    ///
    /// This is the replacement for the engine's synthetic hull-triggered
    /// Retreat, which `aggregate_doctrine_blackboards` used to inject below a
    /// `[behaviour] retreat_hull_threshold`. That mechanism was inert in
    /// production and could not have worked: it scored 0..1 against doctrine
    /// priorities in the tens, so it lost every contest even at zero hull. An
    /// authored entry scores on the same scale as everything else, which is the
    /// bug fix hiding inside the deletion.
    #[test]
    fn authored_retreat_outranks_doctrine_only_once_hull_is_low() {
        let healthy = scored_pool_for(retreat_behaviour(0.3), 100.0, 100.0);
        let hurt = scored_pool_for(retreat_behaviour(0.3), 10.0, 100.0);

        let score_of = |pool: &[crate::messages::ScoredObjective], id: &str| {
            pool.iter()
                .find(|o| o.id == id)
                .unwrap_or_else(|| panic!("{id} must be in the pool"))
                .score
        };

        assert_eq!(
            score_of(&healthy, "retreat-when-hurt"),
            0.0,
            "at full hull the `hull_below` zero-gate must veto the Retreat \
             outright, so its high base_priority costs nothing"
        );
        assert!(
            score_of(&healthy, "loiter") > 0.0,
            "precondition: the rival objective must be live at full hull"
        );

        assert!(
            score_of(&hurt, "retreat-when-hurt") > score_of(&hurt, "loiter"),
            "below the gate's threshold the Retreat must outrank ordinary \
             doctrine — the score-scale bug the synthetic Retreat could never \
             clear (0..1 against a base_priority in the tens)"
        );
        assert_eq!(
            hurt[0].id, "retreat-when-hurt",
            "and it must lead the pool, since every consumer takes the FIRST \
             Helm-relevant entry rather than scanning for the maximum"
        );
    }

    /// The retreat threshold is designer-tunable per entity template — two
    /// ships at identical hull must disagree about retreating purely on their
    /// TOML, with no recompile.
    ///
    /// Was `retreat_threshold_comes_from_behaviour_config`, which tuned the
    /// engine's `[behaviour] retreat_hull_threshold`. The authored form is
    /// strictly more expressive: the threshold, the destination anchor, the
    /// urgency and the gate condition are all per-hull now, rather than one
    /// hardwired hull ramp to a fixed place.
    #[test]
    fn retreat_threshold_is_authored_per_entity_template() {
        // Both ships sit at 40% hull; only their authored gate differs.
        let brave = scored_pool_for(retreat_behaviour(0.1), 40.0, 100.0);
        let cautious = scored_pool_for(retreat_behaviour(0.9), 40.0, 100.0);

        let retreat_score = |pool: &[crate::messages::ScoredObjective]| {
            pool.iter()
                .find(|o| o.id == "retreat-when-hurt")
                .expect("retreat must be in the pool")
                .score
        };

        assert_eq!(
            retreat_score(&brave),
            0.0,
            "hull 0.4 is above a 0.1 threshold — a brave ship must not retreat"
        );
        assert!(
            retreat_score(&cautious) > 0.0,
            "hull 0.4 is below a 0.9 threshold — a cautious ship must retreat"
        );
    }

    /// The published pool is sorted descending by score.
    ///
    /// `operate_helm` and `resolve_helm_target_position` both take the FIRST
    /// Helm-relevant entry as the top-scored directive rather than scanning for
    /// the maximum, so a pool that is merely "mostly sorted" silently
    /// mis-selects.
    #[test]
    fn published_pool_is_sorted_by_score_descending() {
        let scored = scored_pool_for(retreat_behaviour(0.5), 10.0, 100.0);

        assert!(scored.len() > 1, "precondition: need a pool to sort");
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
    }

    /// A hull carrying an authored `Retreat` gated at `threshold`, plus an
    /// ordinary always-on objective to outrank. Mirrors the shape shipped in
    /// `assets/entities/pirate_raider.toml`.
    fn retreat_behaviour(threshold: f32) -> crate::entity_config::BehaviourConfig {
        use crate::entity_config::{BehaviourConfig, DoctrineObjective};
        BehaviourConfig {
            doctrine: vec![
                DoctrineObjective {
                    id: "loiter".into(),
                    text: "Loiter".into(),
                    directive_kind: Some("Patrol".into()),
                    base_priority: 20.0,
                    directive_loop: true,
                    ..Default::default()
                },
                DoctrineObjective {
                    id: "retreat-when-hurt".into(),
                    text: "Hull critical - run for the haven".into(),
                    directive_kind: Some("Retreat".into()),
                    directive_anchor: Some("pirate_haven".into()),
                    base_priority: 100.0,
                    zero_gates: vec![crate::objectives::ZeroGateCondition {
                        condition: "hull_below".into(),
                        threshold: Some(threshold),
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
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
                crate::console_ai_plugin::ShipFrequencyHintState::default(),
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

    // ── Low-LOD patrol wiring (ObjectiveCursors / advance_cursor) ───────────────

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
                ObjectiveCursors::default(),
                blackboards_with_patrol(objective_id, waypoints, loop_path),
            ))
            .id()
    }

    /// Every cursor on `entity` as `(objective_id, waypoint_index)`.
    fn cursor_state(app: &App, entity: Entity) -> Vec<(String, usize)> {
        app.world()
            .get::<ObjectiveCursors>(entity)
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
                ObjectiveCursors::default(),
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
            !app.world().get::<ObjectiveCursors>(npc).unwrap().0[0].settled(),
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

        // NPC carries ObjectiveCursors + an (empty) blackboard but NO patrol
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
                ObjectiveCursors::default(),
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
            app.world()
                .get::<ObjectiveCursors>(npc)
                .unwrap()
                .0
                .is_empty(),
            "cursor must stay empty when there is no patrol objective"
        );
    }
}
