/// Bevy plugin: AI controller lifecycle — attaches `AiController` components
/// to entities that declare a `[behaviour]` block, mints synthetic
/// `ai:<entity_uuid>` session tokens, and ticks controllers during
/// `InProgress` phase.
///
/// Compiled only for the `server` feature (same gate as `simulation.rs`).
use bevy::prelude::*;
use std::collections::HashMap;

use crate::ai::{AiController, AiTickOutput, WorldView};
use crate::entity_spawner::{BehaviourSection, EntityUuid};
use crate::lobby::CurrentPhase;
use crate::messages::GamePhase;

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

/// Bevy component wrapping an `AiController` (the pure state machine).
/// Also carries the entity UUID so the despawn handler can unregister
/// the synthetic token without querying a potentially-absent UUID component.
#[derive(Component)]
pub struct AiControllerComponent {
    pub controller: AiController,
    pub entity_uuid: String,
}

// ── Plugin ───────────────────────────────────────────────────────────────────

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiTokenRegistry>();
        app.add_systems(
            Update,
            (
                attach_controllers_on_spawn,
                tick_ai_controllers,
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
        let mut controller = AiController::new(
            [pos.x, pos.y, pos.z],
            time.elapsed_secs_f64(),
        );
        controller.current_state = initial_state;
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
    mut query: Query<(&mut AiControllerComponent, &mut Transform)>,
    time: Res<Time>,
    map_config: Option<Res<crate::map_config::MapConfig>>,
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

    let dt = time.delta_secs();
    let sim_time = time.elapsed_secs_f64();

    for (mut ctrl, mut transform) in &mut query {
        let pos = transform.translation;
        let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;

        let world_view = WorldView {
            sim_time,
            entity_pos: [pos.x, pos.y, pos.z],
            entity_yaw: yaw,
            anchors: anchors.clone(),
        };

        let output: AiTickOutput = crate::ai::tick(&ctrl.controller, &world_view);

        // Apply blackboard update
        if let Some(new_bb) = output.new_blackboard {
            ctrl.controller.blackboard = new_bb;
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

    use crate::entity_config::BehaviourConfig;
    use crate::entity_spawner::EntityUuid;
    use crate::lobby::{CurrentPhase, LobbyPlugin};
    use crate::messages::GamePhase;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(AiPlugin);
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
}
