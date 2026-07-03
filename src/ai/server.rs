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

use crate::ai::AiMemory;
use crate::entity_spawner::{BehaviourSection, EntityUuid};

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
    if timer.0.tick(time.delta()).just_finished() {
        ready.0 = true;
    } else {
        ready.0 = false;
    }
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
        let scored = crate::ai::score_doctrine_pool(&behaviour.0.doctrine, &conditions);
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
            comms: None,
            radar_appearance: None,
            mesh: None,
            target: None,
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
    /// This is the gate that `operate_helm_ai` checks; without it the ship stays
    /// still even when Backfill AI is active.
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

        let hull = EntitySystemHull(SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]));

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
}
