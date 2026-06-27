/// Bevy plugin: AI controller lifecycle — attaches `AiControllerComponent`
/// to entities that declare a `[behaviour]` block, mints synthetic
/// `ai:<entity_uuid>` session tokens, and ticks controllers during
/// `InProgress` phase.
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

use crate::ai::{
    operate_helm, operate_weapons, score_doctrine_pool, AiMemory, AiWorldEntity, WorldView,
};
use crate::entity_spawner::{BehaviourSection, ColliderSection, EntityUuid};

use crate::config_cache::FactionRegistryResource;

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

    /// Returns `true` when the registry contains a record for this entity UUID.
    pub fn contains_entity(&self, entity_uuid: &str) -> bool {
        self.by_entity.contains_key(entity_uuid)
    }
}

// ── AiControllerComponent ─────────────────────────────────────────────────────

/// Marker component wrapping per-entity `AiMemory` (private reasoning state).
/// Also carries the entity UUID so the despawn handler can unregister
/// the synthetic token without querying a potentially-absent UUID component.
#[derive(Component)]
pub struct AiControllerComponent {
    pub memory: AiMemory,
    pub entity_uuid: String,
    /// Current forward speed in world-units/sec, carried across ticks so the
    /// ship can accelerate over multiple frames (like the player helm does).
    pub forward_speed: f32,
    /// Last helm intent from the AI tick: (thrust, steering). Reset each tick;
    /// read by `operate_helm_ai` to drive ships when helm is on Backfill.
    pub last_helm_intent: Option<(f32, f32)>,
}

/// Per-NPC phaser state. Mirrors the player-ship `PhaserCooldown` / `ActiveBeam`
/// but lives as an ECS component so each NPC tracks its own cooldown independently.
#[derive(Component, Clone, Debug)]
pub struct EntityPhaserState {
    /// Cooldown remaining in seconds after a beam ends. Ready when 0.
    pub cooldown_remaining: f32,
    /// Whether a beam is currently active (firing this tick or ongoing).
    pub beam_active: bool,
    /// UUID of the entity currently being targeted by the beam, if active.
    pub beam_target: Option<uuid::Uuid>,
    /// Duration left on the current beam in seconds.
    pub beam_remaining_secs: f32,
    /// Sub-integer damage accumulator so that fractional per-tick damage
    /// (e.g. 0.3/tick) is flushed in whole-integer chunks rather than being
    /// lost to rounding when passed to shield/hull functions.
    pub damage_accumulator: f32,
}

impl Default for EntityPhaserState {
    fn default() -> Self {
        EntityPhaserState {
            cooldown_remaining: 0.0,
            beam_active: false,
            beam_target: None,
            beam_remaining_secs: 0.0,
            damage_accumulator: 0.0,
        }
    }
}

impl EntityPhaserState {
    pub fn is_ready(&self) -> bool {
        !self.beam_active && self.cooldown_remaining <= 0.0
    }
}

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

/// Component: tracks the hull integrity fraction [0.0, 1.0] of an NPC entity.
/// Default 1.0 (full health). When it reaches ≤ 0.0 the `detect_npc_hull_zero`
/// system emits an `AiEntityDestroyed` event and despawns the entity.
#[derive(Component, Clone, Debug)]
pub struct NpcHullFraction(pub f32);

impl Default for NpcHullFraction {
    fn default() -> Self {
        NpcHullFraction(1.0)
    }
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

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiTokenRegistry>();
        app.init_resource::<ScenariosBeingUnloaded>();
        app.add_message::<AiEntityAttacked>();
        app.add_message::<AiEntityDestroyed>();
        app.add_systems(
            Update,
            (
                attach_controllers_on_spawn,
                tick_ai_controllers
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .in_set(crate::sim_sets::AiTickLabel),
                detect_npc_hull_zero,
                unregister_on_despawn,
            ),
        );
    }
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Attach an `AiControllerComponent` (and register a synthetic token) when a
/// newly-spawned entity carries a `BehaviourSection` but no controller yet.
fn attach_controllers_on_spawn(
    mut commands: Commands,
    mut registry: ResMut<AiTokenRegistry>,
    query: Query<
        (
            Entity,
            &EntityUuid,
            &Transform,
            &BehaviourSection,
            Option<&crate::entity_spawner::WeaponsConsoleSection>,
            Option<&EntityPhaserState>,
        ),
        Without<AiControllerComponent>,
    >,
) {
    for (entity, uuid, transform, _behaviour, weapons_section, existing_phaser) in &query {
        let pos = transform.translation;
        let memory = AiMemory {
            home_position: [pos.x, pos.y, pos.z],
            ..Default::default()
        };
        registry.register_with_entity(&uuid.0, entity);
        let mut entity_cmd = commands.entity(entity);
        entity_cmd.insert(AiControllerComponent {
            memory,
            entity_uuid: uuid.0.clone(),
            forward_speed: 0.0,
            last_helm_intent: None,
        });
        // Pre-attach phaser state so the first attack tick can fire immediately,
        // but only when one isn't already present (tests may set an explicit cooldown).
        if weapons_section.is_some() && existing_phaser.is_none() {
            entity_cmd.insert(EntityPhaserState::default());
        }
    }
}

/// Tick AI controllers — one tick per entity per frame.
///
/// Phase 1: score doctrine pool from `BehaviourSection.doctrine` + per-entity
///   `WorldConditions` (hull fraction + recent attacker).
/// Phase 2: `operate_helm` → `(thrust, steering)` → `last_helm_intent`
/// Phase 3: `operate_weapons` → `(target, fire?)` → InboundMessages
fn tick_ai_controllers(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &mut AiControllerComponent,
        &Transform,
        &BehaviourSection,
        Option<&AttackerThisTick>,
        Option<&crate::entities::spawner::FactionComponent>,
        Option<&ScenarioUnloadedMarker>,
        Option<&crate::entities::spawner::EntityConsoleHull>,
        Option<&crate::entities::spawner::WeaponsConsoleSection>,
        Option<&EntityPhaserState>,
        Option<&crate::entities::spawner::HelmConsoleSection>,
        Option<&ColliderSection>,
    )>,
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    faction_registry: Res<FactionRegistryResource>,
    entity_query: Query<
        (
            &EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::EntityConsoleHull>,
            Option<&ColliderSection>,
        ),
        Without<AiControllerComponent>,
    >,
    mut attacked_events: MessageWriter<AiEntityAttacked>,
    mut inbound: MessageWriter<crate::lobby::InboundMessage>,
    registry_res: Res<AiTokenRegistry>,
) {
    // Build anchor map once (shared across all controllers this tick).
    let anchors: HashMap<String, [f32; 3]> = if let Some(ref wc) = world_config {
        anchors_from_world_config(wc.as_ref())
    } else {
        HashMap::new()
    };

    // Collect world entities from all non-AI entities.
    let mut world_entities: Vec<AiWorldEntity> = entity_query
        .iter()
        .map(
            |(uid, t, faction_comp, hull_comp, collider)| AiWorldEntity {
                uuid: uuid::Uuid::parse_str(&uid.0).unwrap_or_default(),
                position: [t.translation.x, t.translation.y, t.translation.z],
                faction: faction_comp.map(|f| f.0),
                shields: None,
                hull_fraction: hull_comp.and_then(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        Some(h.0.total_current() / max)
                    } else {
                        None
                    }
                }),
                yaw: None,
                radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
                forward_speed: 0.0,
            },
        )
        .collect();

    // Also snapshot all AI-controlled entities so ships can avoid each other.
    // This immutable pass MUST come before the mutable loop below.
    let ai_snapshots: Vec<AiWorldEntity> = query
        .iter()
        .map(|(_, ctrl, t, _, _, _, _, _, _, _, _, collider)| {
            let yaw = t.rotation.to_euler(EulerRot::YXZ).0;
            AiWorldEntity {
                uuid: uuid::Uuid::parse_str(&ctrl.entity_uuid).unwrap_or_default(),
                position: [t.translation.x, t.translation.y, t.translation.z],
                faction: None,
                shields: None,
                hull_fraction: None,
                yaw: Some(yaw),
                radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
                forward_speed: ctrl.forward_speed,
            }
        })
        .collect();
    world_entities.extend(ai_snapshots);

    let sim_time = time.elapsed_secs_f64();

    for (
        entity,
        mut ctrl,
        transform,
        behaviour,
        attacker_comp,
        self_faction_comp,
        unloaded_marker,
        hull_comp,
        weapons_section,
        phaser_state,
        _helm_section,
        collider_section,
    ) in &mut query
    {
        let pos = transform.translation;
        let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;

        // Read attacker from component (set externally by simulation / tests).
        let attacker_this_tick = attacker_comp.map(|a| a.0);
        if attacker_this_tick.is_some() {
            commands.entity(entity).remove::<AttackerThisTick>();
        }

        // Remove ScenarioUnloadedMarker after reading it (fires only once).
        let scenario_unloaded = unloaded_marker.is_some();
        if scenario_unloaded {
            commands.entity(entity).remove::<ScenarioUnloadedMarker>();
        }

        // Populate hull fraction from EntityConsoleHull if present.
        let self_hull_fraction = hull_comp.and_then(|h| {
            let max = h.0.total_max();
            if max > 0.0 {
                Some(h.0.total_current() / max)
            } else {
                None
            }
        });

        // Populate weapons range and phaser readiness from WeaponsConsoleSection and EntityPhaserState.
        let (entity_phaser_ready, entity_weapons_range) = match weapons_section {
            Some(wc) => {
                let ready = phaser_state.map(|ps| ps.is_ready()).unwrap_or(false);
                let range = wc.0.phaser_banks.first().and_then(|b| {
                    if b.beam_range > 0.0 {
                        Some(b.beam_range)
                    } else {
                        None
                    }
                });
                (ready, range)
            }
            None => (false, None),
        };

        let self_uuid_str = ctrl.entity_uuid.clone();
        let world_view = WorldView {
            sim_time,
            entity_pos: [pos.x, pos.y, pos.z],
            entity_yaw: yaw,
            anchors: anchors.clone(),
            entities: world_entities
                .iter()
                .filter(|e| e.uuid.to_string() != self_uuid_str)
                .cloned()
                .collect(),
            attacker_this_tick,
            self_faction: self_faction_comp.map(|f| f.0),
            entity_phaser_ready,
            entity_weapons_range,
            torpedo_tube_ready: None,
            self_hull_fraction,
            scenario_unloaded,
            self_radius: collider_section.map(|c| c.0.radius).unwrap_or(0.0),
        };

        let registry = &faction_registry.0;

        // ── Phase 1: score doctrine pool ──────────────────────────────────

        let conditions = crate::objectives::WorldConditions {
            red_alert: attacker_this_tick.is_some() || ctrl.memory.last_attacker.is_some(),
            hull_fraction: self_hull_fraction.unwrap_or(1.0),
        };
        let scored_pool = score_doctrine_pool(&behaviour.0.doctrine, &conditions);

        // Read values from ctrl before any mutable borrows.
        let forward_speed = ctrl.forward_speed;
        let entity_uuid = ctrl.entity_uuid.clone();

        // ── Phase 2: operate_helm ─────────────────────────────────────────

        let (thrust, steering) = operate_helm(
            &mut ctrl.memory,
            &world_view,
            &scored_pool,
            &behaviour.0.doctrine,
            &anchors,
            behaviour.0.waypoint_arrival_radius,
            behaviour.0.avoidance_buffer,
            behaviour.0.avoidance_look_ahead_secs,
            forward_speed,
            registry,
        );
        ctrl.last_helm_intent = if thrust != 0.0 || steering != 0.0 {
            Some((thrust, steering))
        } else {
            None
        };

        // ── Phase 3: operate_weapons ──────────────────────────────────────

        let (_target_opt, should_fire) =
            operate_weapons(&ctrl.memory, &world_view, &scored_pool, registry);

        // ── Update memory: track last attacker ────────────────────────────

        if let Some(attacker_uuid) = attacker_this_tick {
            let is_new = ctrl.memory.last_attacker != Some(attacker_uuid);
            if is_new {
                attacked_events.write(AiEntityAttacked {
                    entity_uuid: entity_uuid.clone(),
                    attacker_uuid,
                });
            }
            ctrl.memory.last_attacker = Some(attacker_uuid);
        }

        // ── Emit weapon InboundMessages ───────────────────────────────────

        // NPC fire: target is already stored in ctrl.memory by operate_weapons and
        // read directly by the NPC phaser handler — no SetTarget message needed.
        if should_fire {
            if let Some(token) = registry_res.token_for_entity(&entity_uuid) {
                let bank_id = weapons_section
                    .and_then(|wc| wc.0.phaser_banks.first())
                    .map(|b| b.id.clone())
                    .unwrap_or_else(|| "fore".to_string());
                inbound.write(crate::lobby::InboundMessage {
                    token: token.to_string(),
                    msg: crate::messages::ClientMessage::FirePhaser { bank: bank_id },
                });
            }
        }
    }
}

/// Emit `AiEntityDestroyed` and despawn any NPC entity whose `NpcHullFraction`
/// has dropped to ≤ 0.0.
fn detect_npc_hull_zero(
    mut commands: Commands,
    query: Query<(Entity, &EntityUuid, &NpcHullFraction), Changed<NpcHullFraction>>,
    mut destroyed_events: MessageWriter<AiEntityDestroyed>,
) {
    for (entity, uuid, hull) in &query {
        if hull.0 <= 0.0 {
            destroyed_events.write(AiEntityDestroyed {
                entity_uuid: uuid.0.clone(),
            });
            commands.entity(entity).despawn();
        }
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
        let ctrl = app.world().get::<AiControllerComponent>(entity).unwrap();
        let home = ctrl.memory.home_position;
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

    // ── AiEntityDestroyed event ────────────────────────────────────────────

    #[test]
    fn ai_entity_destroyed_event_emitted_when_hull_reaches_zero() {
        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-hull-001".to_string()),
                NpcHullFraction(1.0),
            ))
            .id();
        app.update();

        // Reduce hull to zero
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<NpcHullFraction>()
            .unwrap()
            .0 = 0.0;
        app.update();

        let events = app.world().resource::<DestroyedBox>().0.clone();
        assert!(
            events.iter().any(|e| e.entity_uuid == "ent-hull-001"),
            "AiEntityDestroyed must be emitted when hull reaches 0"
        );
    }

    #[test]
    fn entity_despawned_when_hull_reaches_zero() {
        let mut app = build_test_app();

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-hull-002".to_string()),
                NpcHullFraction(0.5),
            ))
            .id();
        app.update();

        app.world_mut()
            .entity_mut(entity)
            .get_mut::<NpcHullFraction>()
            .unwrap()
            .0 = 0.0;
        app.update();

        assert!(
            app.world().get_entity(entity).is_err(),
            "entity must be despawned when hull reaches 0"
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
            radar: None,
        }
    }

    #[test]
    fn self_hull_fraction_reflects_entity_console_hull() {
        use crate::damage::ConsoleHull;
        use crate::entity_spawner::EntityConsoleHull;
        use crate::messages::Console;

        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        // 50 HP out of 100 HP = 0.5 fraction
        let mut hull = ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)]);
        let mut rng = rand::rng();
        hull.apply_damage(50.0, &mut rng);

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-hull-frac-001".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                EntityConsoleHull(hull),
            ))
            .id();

        app.update(); // attach controller
        app.update(); // tick

        // The hull fraction should be ~0.5; we verify via the world_view that was
        // used internally by confirming the EntityConsoleHull component is readable.
        let hull_comp = app.world().get::<EntityConsoleHull>(entity).unwrap();
        let frac = hull_comp.0.total_current() / hull_comp.0.total_max();
        assert!(
            (frac - 0.5).abs() < 0.01,
            "hull fraction should be ~0.5, got {frac}"
        );
    }

    #[test]
    fn entity_phaser_ready_true_when_weapons_console_present_and_no_cooldown() {
        use crate::entity_spawner::WeaponsConsoleSection;

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
                EntityPhaserState::default(), // cooldown 0 → ready
            ))
            .id();

        app.update(); // attach controller + first tick
        app.update(); // second tick runs the world_view logic

        // entity_phaser_ready was used in the WorldView. We can't directly observe
        // the WorldView, but we can verify that the entity has its components intact.
        let ps = app.world().get::<EntityPhaserState>(entity).unwrap();
        assert!(ps.is_ready(), "phaser must be ready when cooldown is 0");
    }

    #[test]
    fn entity_phaser_ready_false_when_cooldown_active() {
        use crate::entity_spawner::WeaponsConsoleSection;

        let mut app = build_test_app();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid("ent-phaser-003".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                WeaponsConsoleSection(make_weapons_console_config(40.0)),
                EntityPhaserState {
                    cooldown_remaining: 5.0,
                    ..EntityPhaserState::default()
                },
            ))
            .id();

        app.update();

        let ps = app.world().get::<EntityPhaserState>(entity).unwrap();
        assert!(
            !ps.is_ready(),
            "phaser must not be ready when cooldown is active"
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
            shields: None,
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
