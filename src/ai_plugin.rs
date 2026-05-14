/// Bevy plugin: AI controller lifecycle — attaches `AiController` components
/// to entities that declare a `[behaviour]` block, mints synthetic
/// `ai:<entity_uuid>` session tokens, and ticks controllers during
/// `InProgress` phase.
///
/// Compiled only for the `server` feature (same gate as `simulation.rs`).
use bevy::prelude::*;
use std::collections::HashMap;

use crate::ai::{AiController, AiTickOutput, WorldView, WorldEntity};
use crate::entity_spawner::{BehaviourSection, EntityUuid};
use crate::lobby::CurrentPhase;
use crate::messages::GamePhase;
#[cfg(target_arch = "wasm32")]
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
        self.by_entity.get(entity_uuid).map(|s| s.as_str()).unwrap_or("")
    }

    /// Register a synthetic token AND record the Bevy `Entity` for
    /// despawn-time unregistration. Idempotent.
    pub fn register_with_entity(&mut self, entity_uuid: &str, entity: Entity) {
        self.register(entity_uuid);
        self.by_bevy_entity.insert(entity, entity_uuid.to_string());
    }

    /// Unregister by entity UUID; silently does nothing if not present.
    pub fn unregister(&mut self, entity_uuid: &str) {
        if let Some(token) = self.by_entity.remove(entity_uuid) {
            self.by_token.remove(&token);
        }
    }

    /// Unregister by Bevy `Entity`; used by the despawn handler when the
    /// UUID component is no longer accessible.
    pub fn unregister_by_bevy_entity(&mut self, entity: Entity) {
        if let Some(uuid) = self.by_bevy_entity.remove(&entity) {
            self.unregister(&uuid);
        }
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

// ── AiControllerComponent ────────────────────────────────────────────────────

/// Marker component wrapping an `AiController` (the pure state machine).
/// Also carries the entity UUID so the despawn handler can unregister
/// the synthetic token without querying a potentially-absent UUID component.
#[derive(Component)]
pub struct AiControllerComponent {
    pub controller: AiController,
    pub entity_uuid: String,
}

/// Marker component set on NPC entities currently in the `WarpingOut` AI state.
/// Carries the data needed to draw the warp-exit visual and to populate
/// `EntitySnapshot::warp_out_remaining_secs` in the broadcast.
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

/// Emitted by the AI plugin when an NPC entity's `on_attacked` condition fires
/// (first hit per state-entry, i.e. while `on_attacked_armed` is true).
///
/// The scenario plugin observes this event to evaluate `on_entity_attacked`
/// trigger conditions without a direct dependency on the AI module.
#[derive(Message, Clone, Debug)]
pub struct AiEntityAttacked {
    pub entity_uuid: String,
    pub attacker_uuid: uuid::Uuid,
}

/// Emitted by the AI plugin when an NPC entity's hull reaches ≤ 0.0.
///
/// The scenario plugin observes this event to evaluate `on_entity_destroyed`
/// trigger conditions without a direct dependency on the AI module.
#[derive(Message, Clone, Debug)]
pub struct AiEntityDestroyed {
    pub entity_uuid: String,
}

// ── Plugin ───────────────────────────────────────────────────────────────────

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiTokenRegistry>();
        app.add_message::<AiEntityAttacked>();
        app.add_message::<AiEntityDestroyed>();
        app.add_systems(
            Update,
            (
                attach_controllers_on_spawn,
                tick_ai_controllers,
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
    query: Query<(Entity, &EntityUuid, &Transform, &BehaviourSection), Without<AiControllerComponent>>,
    time: Res<Time>,
) {
    for (entity, uuid, transform, behaviour) in &query {
        let pos = transform.translation;
        let initial_state = crate::ai::build_initial_state(&behaviour.0);
        let initial_state_name = behaviour.0.initial_state.clone();
        let mut controller = AiController::new(
            [pos.x, pos.y, pos.z],
            time.elapsed_secs_f64(),
        );
        controller.current_state = initial_state;
        controller.current_state_name = initial_state_name;
        registry.register_with_entity(&uuid.0, entity);
        commands
            .entity(entity)
            .insert(AiControllerComponent {
                controller,
                entity_uuid: uuid.0.clone(),
            });
    }
}

/// Tick AI controllers, but only during `InProgress` phase.
fn tick_ai_controllers(
    phase: Res<CurrentPhase>,
    mut commands: Commands,
    mut query: Query<(Entity, &mut AiControllerComponent, &mut Transform, &BehaviourSection, Option<&AttackerThisTick>)>,
    time: Res<Time>,
    map_config: Option<Res<crate::map_config::MapConfig>>,
    #[cfg(target_arch = "wasm32")]
    faction_registry: Option<Res<FactionRegistryResource>>,
    entity_query: Query<(&EntityUuid, &Transform), Without<AiControllerComponent>>,
    mut attacked_events: MessageWriter<AiEntityAttacked>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    // Build anchor map once (shared across all controllers this tick)
    let anchors: HashMap<String, [f32; 3]> = if let Some(ref mc) = map_config {
        mc.anchors.iter().filter_map(|(name, pos)| {
            if pos.len() >= 3 {
                Some((name.clone(), [pos[0], pos[1], pos[2]]))
            } else if pos.len() == 2 {
                Some((name.clone(), [pos[0], 0.0, pos[1]]))
            } else {
                None
            }
        }).collect()
    } else {
        HashMap::new()
    };

    // Collect world entities from all non-AI entities (approximate: no faction yet)
    let world_entities: Vec<WorldEntity> = entity_query.iter().map(|(uid, t)| {
        WorldEntity {
            uuid: uuid::Uuid::parse_str(&uid.0).unwrap_or_default(),
            position: [t.translation.x, t.translation.y, t.translation.z],
            faction: None,
            shields: None,
            hull_fraction: None,
            yaw: None,
        }
    }).collect();

    let empty_registry = crate::faction::FactionRegistry::new();
    #[cfg(target_arch = "wasm32")]
    let actual_registry: Option<&crate::faction::FactionRegistry> =
        faction_registry.as_ref().map(|r| &r.0);
    #[cfg(not(target_arch = "wasm32"))]
    let actual_registry: Option<&crate::faction::FactionRegistry> = None;

    let dt = time.delta_secs();
    let sim_time = time.elapsed_secs_f64();

    for (entity, mut ctrl, mut transform, behaviour, attacker_comp) in &mut query {
        let pos = transform.translation;
        let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;

        // Read attacker from component (set externally by simulation / tests).
        let attacker_this_tick = attacker_comp.map(|a| a.0);

        let world_view = WorldView {
            sim_time,
            entity_pos: [pos.x, pos.y, pos.z],
            entity_yaw: yaw,
            anchors: anchors.clone(),
            entities: world_entities.clone(),
            attacker_this_tick,
            self_faction: None,       // TODO: populate from entity config faction field
            entity_phaser_ready: false,
            entity_weapons_range: None,
            torpedo_tube_ready: None,
            self_hull_fraction: None,     // TODO: populate from NPC hull component when added
            scenario_unloaded: false,     // TODO: set when owning scenario begins unloading
        };

        let registry = actual_registry.unwrap_or(&empty_registry);
        let output: AiTickOutput = crate::ai::tick(&ctrl.controller, &world_view, &behaviour.0, registry);

        // Emit AiEntityAttacked when the on_attacked condition just fired.
        // We detect this by checking: attacker present AND on_attacked_armed was true
        // before the tick (i.e. the new blackboard has on_attacked_armed == false).
        if let Some(attacker_uuid) = attacker_this_tick {
            let was_armed = ctrl.controller.blackboard.on_attacked_armed;
            let now_disarmed = output.new_blackboard.as_ref()
                .map(|bb| !bb.on_attacked_armed)
                .unwrap_or(false);
            if was_armed && now_disarmed {
                attacked_events.write(AiEntityAttacked {
                    entity_uuid: ctrl.entity_uuid.clone(),
                    attacker_uuid,
                });
            }
        }

        // Apply blackboard update
        if let Some(new_bb) = output.new_blackboard {
            ctrl.controller.blackboard = new_bb;
        }
        // Update state name when state changes
        if output.new_state != ctrl.controller.current_state {
            // Find the config entry matching the new state kind to get its name
            ctrl.controller.current_state_name = behaviour.0.transition.iter()
                .find(|t| build_state_by_name_matches(&output.new_state, &behaviour.0, &t.to))
                .map(|t| t.to.clone())
                .unwrap_or_else(|| output.new_state.kind_name().to_string());
        }
        ctrl.controller.current_state = output.new_state;

        // Apply the first Helm input to the entity's Transform.
        for input in &output.inputs {
            if let crate::ai::AiInput::Helm { thrust, steering } = input {
                let physics_state = crate::ship_physics::ShipPhysicsState {
                    x: pos.x,
                    z: pos.z,
                    yaw,
                    forward_speed: 0.0,
                };
                let physics_input = crate::ship_physics::ShipPhysicsInput {
                    thrust: *thrust,
                    steering: *steering,
                };
                let result = crate::ship_physics::compute_physics(
                    physics_state,
                    physics_input,
                    dt,
                    &crate::ship_physics::ShipPhysicsConfig::new(),
                );
                transform.translation.x = result.x;
                transform.translation.z = result.z;
                transform.rotation = Quat::from_rotation_y(result.yaw);
                break;
            }
        }

        // Handle self-despawn (e.g. WarpingOut timer expired).
        if output.despawn {
            commands.entity(entity).despawn();
            continue;
        }

        // Update WarpOutMarker: insert/update when warping out, remove otherwise.
        match &ctrl.controller.current_state {
            crate::ai::AiState::WarpingOut { duration_secs, target_speed } => {
                let elapsed = sim_time as f32 - ctrl.controller.blackboard.state_entered_at as f32;
                let remaining = (duration_secs - elapsed).max(0.0);
                commands.entity(entity).insert(WarpOutMarker {
                    remaining_secs: remaining,
                    target_speed: *target_speed,
                });
            }
            _ => {
                commands.entity(entity).remove::<WarpOutMarker>();
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

/// Helper: check if a state built from `name` in `behaviour` matches `new_state`.
fn build_state_by_name_matches(
    new_state: &crate::ai::AiState,
    behaviour: &crate::entity_config::BehaviourConfig,
    name: &str,
) -> bool {
    let built = crate::ai::build_initial_state(&crate::entity_config::BehaviourConfig {
        initial_state: name.to_string(),
        state: behaviour.state.clone(),
        transition: vec![],
    });
    &built == new_state
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

    // ── AiTokenRegistry unit tests ────────────────────────────────────────

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

    // ── Bevy integration tests ────────────────────────────────────────────

    use crate::entity_config::{BehaviourConfig, StateConfig};
    use crate::entity_spawner::EntityUuid;
    use crate::lobby::{CurrentPhase, LobbyPlugin};
    use crate::messages::GamePhase;

    #[derive(Resource, Default)]
    struct AttackedBox(Vec<AiEntityAttacked>);
    #[derive(Resource, Default)]
    struct DestroyedBox(Vec<AiEntityDestroyed>);

    fn collect_attacked(mut r: MessageReader<AiEntityAttacked>, mut b: ResMut<AttackedBox>) {
        for e in r.read() { b.0.push(e.clone()); }
    }
    fn collect_destroyed(mut r: MessageReader<AiEntityDestroyed>, mut b: ResMut<DestroyedBox>) {
        for e in r.read() { b.0.push(e.clone()); }
    }

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(AiPlugin)
            .init_resource::<AttackedBox>()
            .init_resource::<DestroyedBox>()
            .add_systems(PostUpdate, (collect_attacked, collect_destroyed));
        app
    }

    fn spawn_behaviour_entity(app: &mut App, uuid: &str) -> Entity {
        let entity = app.world_mut().spawn((
            Transform::from_xyz(1.0, 0.0, 2.0),
            EntityUuid(uuid.to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".to_string(),
                state: vec![],
                transition: vec![],
            }),
        )).id();
        entity
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
    fn controller_starts_in_idle_state() {
        let mut app = build_test_app();
        let entity = spawn_behaviour_entity(&mut app, "ent-002");
        app.update();
        let ctrl = app.world().get::<AiControllerComponent>(entity).unwrap();
        assert_eq!(ctrl.controller.current_state, crate::ai::AiState::Idle);
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
    fn blackboard_seeded_with_spawn_position() {
        let mut app = build_test_app();
        let entity = app.world_mut().spawn((
            Transform::from_xyz(10.0, 0.0, -5.0),
            EntityUuid("ent-004".to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".to_string(),
                state: vec![],
                transition: vec![],
            }),
        )).id();
        app.update();
        let ctrl = app.world().get::<AiControllerComponent>(entity).unwrap();
        let home = ctrl.controller.blackboard.home_position;
        assert!((home[0] - 10.0).abs() < 0.001, "home x must be spawn x");
        assert!((home[2] - -5.0).abs() < 0.001, "home z must be spawn z");
    }

    #[test]
    fn idle_controller_produces_no_inputs_in_progress_phase() {
        let mut app = build_test_app();
        // Set InProgress phase
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        let entity = spawn_behaviour_entity(&mut app, "ent-005");
        app.update();
        // Controller stays idle - we verify it's still Idle
        let ctrl = app.world().get::<AiControllerComponent>(entity).unwrap();
        assert_eq!(ctrl.controller.current_state, crate::ai::AiState::Idle);
    }

    #[test]
    fn idle_controller_does_not_change_state_in_lobby_phase() {
        let mut app = build_test_app();
        // Default phase is Lobby
        let entity = spawn_behaviour_entity(&mut app, "ent-006");
        app.update();
        let ctrl = app.world().get::<AiControllerComponent>(entity).unwrap();
        assert_eq!(ctrl.controller.current_state, crate::ai::AiState::Idle);
    }

    #[test]
    fn token_unregistered_after_entity_despawn() {
        let mut app = build_test_app();
        let entity = spawn_behaviour_entity(&mut app, "ent-007");
        app.update();
        // Verify registered
        assert!(app.world().resource::<AiTokenRegistry>().contains_entity("ent-007"));
        // Despawn
        app.world_mut().despawn(entity);
        app.update();
        assert!(
            !app.world().resource::<AiTokenRegistry>().contains_entity("ent-007"),
            "token must be unregistered after despawn"
        );
    }

    // ── AiEntityAttacked event ────────────────────────────────────────────

    fn build_on_attacked_behaviour() -> BehaviourConfig {
        BehaviourConfig {
            initial_state: "idle".into(),
            state: vec![StateConfig {
                name: "chase".into(),
                kind: "pursuing".into(),
                waypoints: vec![],
                loop_path: false,
                target_speed: 0.8,
                maintain_range: 0.0,
                duration_secs: 0.0,
            }],
            transition: vec![crate::ai::TransitionConfig {
                from: crate::ai::StringOrVec::Single("idle".into()),
                to: "chase".into(),
                condition: "on_attacked".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
        }
    }

    #[test]
    fn ai_entity_attacked_event_emitted_when_attacker_set_and_on_attacked_fires() {
        let mut app = build_test_app();
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;

        // Spawn entity with on_attacked transition and an attacker component.
        let attacker_id = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000099").unwrap();
        let entity = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-attacked-001".to_string()),
            BehaviourSection(build_on_attacked_behaviour()),
            AttackerThisTick(attacker_id),
        )).id();

        app.update(); // attach controller + first tick
        app.update(); // tick fires transition

        let events = app.world().resource::<AttackedBox>().0.clone();

        assert!(
            events.iter().any(|e| e.entity_uuid == "ent-attacked-001"),
            "AiEntityAttacked must be emitted when on_attacked fires"
        );
    }

    // ── AiEntityDestroyed event ───────────────────────────────────────────

    #[test]
    fn ai_entity_destroyed_event_emitted_when_hull_reaches_zero() {
        let mut app = build_test_app();
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;

        let entity = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-hull-001".to_string()),
            NpcHullFraction(1.0),
        )).id();
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

        let entity = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-hull-002".to_string()),
            NpcHullFraction(0.5),
        )).id();
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
}
