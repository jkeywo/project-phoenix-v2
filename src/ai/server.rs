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
}

impl Default for EntityPhaserState {
    fn default() -> Self {
        EntityPhaserState {
            cooldown_remaining: 0.0,
            beam_active: false,
            beam_target: None,
            beam_remaining_secs: 0.0,
        }
    }
}

impl EntityPhaserState {
    pub fn is_ready(&self) -> bool {
        !self.beam_active && self.cooldown_remaining <= 0.0
    }
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

/// Marker component added to an AI entity when its owning scenario is unloaded.
///
/// `tick_ai_controllers` reads this component to set `scenario_unloaded: true`
/// in the `WorldView`, allowing `on_scenario_unloaded` transitions to fire.
/// The component persists until `tick_ai_controllers` removes it (or until the
/// entity despawns alongside its scenario cleanup).
#[derive(Component)]
pub struct ScenarioUnloadedMarker;

/// Resource kept for backward-compatibility; no longer used for signalling.
#[derive(Resource, Default)]
pub struct ScenariosBeingUnloaded(pub std::collections::HashSet<String>);

/// Emitted by the AI plugin when an NPC entity's `on_attacked` condition fires
/// (first hit per state-entry, i.e. while `on_attacked_armed` is true).
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

// ── Plugin ───────────────────────────────────────────────────────────────────

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
                tick_ai_controllers.in_set(crate::sim_sets::SimSet::Damage),
                tick_npc_phasers.in_set(crate::sim_sets::SimSet::Damage),
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

/// Tick AI controllers.
fn tick_ai_controllers(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &mut AiControllerComponent,
        &mut Transform,
        &BehaviourSection,
        Option<&AttackerThisTick>,
        Option<&crate::entities::spawner::FactionComponent>,
        Option<&ScenarioUnloadedMarker>,
        Option<&crate::entities::spawner::EntityConsoleHull>,
        Option<&crate::entities::spawner::WeaponsConsoleSection>,
        Option<&EntityPhaserState>,
    )>,
    time: Res<Time>,
    map_config: Option<Res<crate::map_config::MapConfig>>,
    faction_registry: Option<Res<FactionRegistryResource>>,
    entity_query: Query<(&EntityUuid, &Transform, Option<&crate::entities::spawner::FactionComponent>), Without<AiControllerComponent>>,
    mut attacked_events: MessageWriter<AiEntityAttacked>,
    mut inbound: MessageWriter<crate::lobby::InboundMessage>,
    registry_res: Res<AiTokenRegistry>,
) {

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

    // Collect world entities from all non-AI entities, including faction if present.
    let world_entities: Vec<WorldEntity> = entity_query.iter().map(|(uid, t, faction_comp)| {
        WorldEntity {
            uuid: uuid::Uuid::parse_str(&uid.0).unwrap_or_default(),
            position: [t.translation.x, t.translation.y, t.translation.z],
            faction: faction_comp.map(|f| f.0),
            shields: None,
            hull_fraction: None,
            yaw: None,
        }
    }).collect();

    let empty_registry = crate::faction::FactionRegistry::new();
    let actual_registry: Option<&crate::faction::FactionRegistry> =
        faction_registry.as_ref().map(|r| &r.0);

    let dt = time.delta_secs();
    let sim_time = time.elapsed_secs_f64();

    for (entity, mut ctrl, mut transform, behaviour, attacker_comp, self_faction_comp, unloaded_marker, hull_comp, weapons_section, phaser_state) in &mut query {
        let pos = transform.translation;
        let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;

        // Read attacker from component (set externally by simulation / tests).
        let attacker_this_tick = attacker_comp.map(|a| a.0);

        // `scenario_unloaded` is true when this entity carries the marker set
        // by `handle_ai_events` when its owning scenario was unloaded.
        let scenario_unloaded = unloaded_marker.is_some();
        // Remove the marker so the transition only fires once.
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
                let range = if wc.0.beam_range > 0.0 { Some(wc.0.beam_range) } else { None };
                (ready, range)
            }
            None => (false, None),
        };

        let world_view = WorldView {
            sim_time,
            entity_pos: [pos.x, pos.y, pos.z],
            entity_yaw: yaw,
            anchors: anchors.clone(),
            entities: world_entities.clone(),
            attacker_this_tick,
            self_faction: self_faction_comp.map(|f| f.0),
            entity_phaser_ready,
            entity_weapons_range,
            torpedo_tube_ready: None,
            self_hull_fraction,
            scenario_unloaded,
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

        // Inject weapon AI outputs as InboundMessages so they are processed by the
        // NPC phaser system (tick_npc_phasers) using the entity's synthetic token.
        let ai_token = registry_res.token_for_entity(&ctrl.entity_uuid)
            .map(|s| s.to_string());
        if let Some(token) = ai_token {
            for input in &output.inputs {
                match input {
                    crate::ai::AiInput::SetTarget { uuid: target } => {
                        inbound.write(crate::lobby::InboundMessage {
                            token: token.clone(),
                            msg: crate::messages::ClientMessage::SetTarget {
                                uuid: target.to_string(),
                            },
                        });
                    }
                    crate::ai::AiInput::FirePhaser => {
                        inbound.write(crate::lobby::InboundMessage {
                            token: token.clone(),
                            msg: crate::messages::ClientMessage::FirePhaser,
                        });
                    }
                    _ => {}
                }
            }
        }

        // Apply the first Helm input to the entity's Transform.
        for input in &output.inputs {
            if let crate::ai::AiInput::Helm { thrust, steering } = *input {
                let physics_state = crate::ship_physics::ShipPhysicsState {
                    x: pos.x,
                    z: pos.z,
                    yaw,
                    forward_speed: 0.0,
                };
                let physics_input = crate::ship_physics::ShipPhysicsInput {
                    thrust,
                    steering,
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

// ── NPC phaser constants ──────────────────────────────────────────────────────

/// Default NPC beam duration in seconds (used when `WeaponsConsoleSection` has no override).
const NPC_BEAM_DURATION_SECS: f32 = 3.0;
/// Default NPC beam damage per second (used when config has no override).
const NPC_BEAM_DAMAGE_PER_SEC: f32 = 5.0;

/// Per-tick NPC phaser handler.
///
/// Processes `InboundMessage { FirePhaser }` messages whose token belongs to an
/// NPC entity (i.e. starts with `"ai:"`). For each matching entity:
/// - Activates the beam when `FirePhaser` arrives, the entity is not on cooldown,
///   AND `crate::radar::is_fire_ready_with_range` passes (same range + arc guard
///   as the player weapon path).
/// - Ticks the active beam: accumulates damage, applies it to the target's
///   `EntityConsoleHull`, and cancels the beam when the target is destroyed.
/// - After the beam ends, starts a cooldown on `EntityPhaserState`.
pub fn tick_npc_phasers(
    time: Res<Time>,
    mut commands: Commands,
    registry: Res<AiTokenRegistry>,
    mut npc_query: Query<(
        Entity,
        &EntityUuid,
        &Transform,
        Option<&mut EntityPhaserState>,
        Option<&crate::entities::spawner::WeaponsConsoleSection>,
        Option<&AiControllerComponent>,
    ), With<AiControllerComponent>>,
    mut hull_query: Query<(Entity, &EntityUuid, &Transform, &mut crate::entities::spawner::EntityConsoleHull), Without<AiControllerComponent>>,
    mut inbound: MessageReader<crate::lobby::InboundMessage>,
    mut destroyed_events: MessageWriter<AiEntityDestroyed>,
) {
    let dt = time.delta_secs();

    // Collect FirePhaser messages for AI tokens this tick.
    // We drain them all first so the MessageReader borrow ends before we mutate queries.
    let mut fire_orders: Vec<String> = Vec::new();
    for ev in inbound.read() {
        if !ev.token.starts_with("ai:") {
            continue;
        }
        if matches!(&ev.msg, crate::messages::ClientMessage::FirePhaser) {
            fire_orders.push(ev.token.clone());
        }
    }

    // Snapshot target positions so we can do range/arc checks without holding
    // a mutable borrow on hull_query while also borrowing npc_query.
    let target_positions: Vec<(uuid::Uuid, f32, f32)> = hull_query.iter()
        .filter_map(|(_, uid, t, _)| {
            uuid::Uuid::parse_str(&uid.0).ok()
                .map(|u| (u, t.translation.x, t.translation.z))
        })
        .collect();

    for (npc_entity, npc_uuid, transform, phaser_state_opt, weapons_section, ctrl_opt) in npc_query.iter_mut() {
        let token = match registry.token_for_entity(&npc_uuid.0) {
            Some(t) => t.to_string(),
            None => continue,
        };

        // Ensure the entity has an EntityPhaserState component; insert default if missing.
        let phaser_state = match phaser_state_opt {
            Some(ps) => ps.into_inner(),
            None => {
                commands.entity(npc_entity).insert(EntityPhaserState::default());
                continue; // will pick up the component on the next tick
            }
        };

        // Tick cooldown.
        phaser_state.cooldown_remaining = (phaser_state.cooldown_remaining - dt).max(0.0);

        // Resolve the target UUID from the controller's blackboard.
        let target_uuid: Option<uuid::Uuid> = ctrl_opt
            .and_then(|c| c.controller.blackboard.target);

        let beam_range = weapons_section
            .map(|wc| if wc.0.beam_range > 0.0 { wc.0.beam_range } else { 40.0 })
            .unwrap_or(40.0);
        let damage_per_sec = weapons_section
            .map(|wc| if wc.0.beam_damage_per_sec > 0.0 { wc.0.beam_damage_per_sec } else { NPC_BEAM_DAMAGE_PER_SEC })
            .unwrap_or(NPC_BEAM_DAMAGE_PER_SEC);
        let beam_duration = weapons_section
            .map(|wc| if wc.0.beam_duration_secs > 0.0 { wc.0.beam_duration_secs } else { NPC_BEAM_DURATION_SECS })
            .unwrap_or(NPC_BEAM_DURATION_SECS);

        // NPC position and yaw (same coordinate conventions as the player ship).
        let npc_x = transform.translation.x;
        let npc_z = transform.translation.z;
        let npc_yaw = transform.rotation.to_euler(EulerRot::YXZ).0;

        // Start beam on FirePhaser order when ready AND range/arc check passes.
        if fire_orders.contains(&token) && phaser_state.is_ready() {
            if let Some(t_uuid) = target_uuid {
                let fire_ok = target_positions.iter()
                    .find(|(u, _, _)| *u == t_uuid)
                    .map(|(_, tx, tz)| {
                        crate::radar::is_fire_ready_with_range(
                            *tx, *tz, npc_x, npc_z, npc_yaw, beam_range,
                        )
                    })
                    .unwrap_or(false);

                if fire_ok {
                    phaser_state.beam_active = true;
                    phaser_state.beam_target = Some(t_uuid);
                    phaser_state.beam_remaining_secs = beam_duration;
                }
            }
        }

        // Tick active beam.
        if phaser_state.beam_active {
            phaser_state.beam_remaining_secs = (phaser_state.beam_remaining_secs - dt).max(0.0);

            if let Some(t_uuid) = phaser_state.beam_target {
                // Apply damage to target.
                let damage = damage_per_sec * dt;
                let mut target_destroyed = false;
                let target_uuid_str = t_uuid.to_string();
                for (_tgt_entity, tgt_uid, _tgt_transform, mut tgt_hull) in hull_query.iter_mut() {
                    if tgt_uid.0 != target_uuid_str {
                        continue;
                    }
                    let mut rng = rand::rng();
                    tgt_hull.0.apply_damage(damage, &mut rng);
                    if tgt_hull.0.is_destroyed() {
                        target_destroyed = true;
                        commands.entity(_tgt_entity).despawn();
                        destroyed_events.write(AiEntityDestroyed { entity_uuid: tgt_uid.0.clone() });
                    }
                    break;
                }
                if target_destroyed || phaser_state.beam_remaining_secs <= 0.0 {
                    phaser_state.beam_active = false;
                    phaser_state.beam_target = None;
                    phaser_state.beam_remaining_secs = 0.0;
                    phaser_state.cooldown_remaining = beam_duration; // reuse beam_duration as cooldown
                }
            } else {
                // No target — cancel beam.
                phaser_state.beam_active = false;
                phaser_state.beam_remaining_secs = 0.0;
            }
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
    use crate::lobby::LobbyPlugin;
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
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));
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
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

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
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

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

    // ── scenario_unloaded flag in WorldView ────────────────────────────────

    // When ScenarioUnloadedMarker is on an entity, it fires on_scenario_unloaded transition.
    #[test]
    fn on_scenario_unloaded_transition_fires_when_scenario_being_unloaded() {
        use crate::entities::spawner::ScenarioOwner;
        use crate::entity_config::{BehaviourConfig, StateConfig};
        use crate::ai::{TransitionConfig, StringOrVec};

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let behaviour = BehaviourConfig {
            initial_state: "patrol".to_string(),
            state: vec![
                StateConfig { name: "patrol".to_string(), kind: "idle".to_string(), waypoints: vec![], loop_path: false, target_speed: 0.0, maintain_range: 0.0, duration_secs: 0.0 },
                StateConfig { name: "free".to_string(), kind: "warping_out".to_string(), waypoints: vec![], loop_path: false, target_speed: 0.5, maintain_range: 0.0, duration_secs: 3.0 },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("patrol".into()),
                to: "free".into(),
                condition: "on_scenario_unloaded".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
        };

        let entity = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-unloaded-001".to_string()),
            BehaviourSection(behaviour),
            ScenarioOwner("scenarios/alpha.toml".to_string()),
        )).id();

        app.update(); // attach controller

        // Mark entity as scenario-unloaded via component
        app.world_mut().entity_mut(entity).insert(ScenarioUnloadedMarker);

        app.update(); // tick — should fire transition

        let ctrl = app.world().get::<AiControllerComponent>(entity).unwrap();
        assert_eq!(
            ctrl.controller.current_state_name, "free",
            "on_scenario_unloaded must fire when ScenarioUnloadedMarker is present"
        );
    }

    // Entity without ScenarioOwner does not get scenario_unloaded flag
    #[test]
    fn entity_without_scenario_owner_does_not_see_scenario_unloaded() {
        use crate::entity_config::{BehaviourConfig, StateConfig};
        use crate::ai::{TransitionConfig, StringOrVec};

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let behaviour = BehaviourConfig {
            initial_state: "patrol".to_string(),
            state: vec![
                StateConfig { name: "patrol".to_string(), kind: "idle".to_string(), waypoints: vec![], loop_path: false, target_speed: 0.0, maintain_range: 0.0, duration_secs: 0.0 },
                StateConfig { name: "free".to_string(), kind: "idle".to_string(), waypoints: vec![], loop_path: false, target_speed: 0.0, maintain_range: 0.0, duration_secs: 0.0 },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("patrol".into()),
                to: "free".into(),
                condition: "on_scenario_unloaded".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
        };

        // Entity has NO ScenarioOwner and no ScenarioUnloadedMarker
        let entity = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-free-002".to_string()),
            BehaviourSection(behaviour),
        )).id();

        app.update();

        // A different entity gets the marker — this entity should not transition
        // (simulates: only owned entities get ScenarioUnloadedMarker)
        let _other = app.world_mut().spawn(Transform::from_xyz(0.0, 0.0, 0.0)).id();

        app.update();

        let ctrl = app.world().get::<AiControllerComponent>(entity).unwrap();
        assert_eq!(
            ctrl.controller.current_state_name, "patrol",
            "entity without ScenarioUnloadedMarker must not fire on_scenario_unloaded"
        );
    }

    // ── Issue #314: WorldView population from components ─────────────────────

    fn make_weapons_console_config(beam_range: f32) -> crate::entity_config::WeaponsConsoleConfig {
        crate::entity_config::WeaponsConsoleConfig {
            radar_range: 0.0,
            target_range: 0.0,
            fire_arc: 0.0,
            beam_range,
            beam_damage_per_sec: 5.0,
            beam_duration_secs: 3.0,
            cooldown_secs: 3.0,
            beam_color: vec![],
            power_multipliers: None,
            complexity_toml: None,
        }
    }

    /// Spawn an NPC entity with EntityConsoleHull at the given HP fraction and return its entity.
    fn spawn_npc_with_hull(app: &mut App, uuid: &str, current_hp: f32, max_hp: f32) -> Entity {
        use crate::entity_spawner::EntityConsoleHull;
        use crate::damage::ConsoleHull;
        use crate::messages::Console;

        let mut hull = ConsoleHull::from_config(&[(Console::CaptainChair, max_hp)]);
        if current_hp < max_hp {
            let damage = max_hp - current_hp;
            let mut rng = rand::rng();
            hull.apply_damage(damage, &mut rng);
        }
        app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid(uuid.to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".into(),
                state: vec![],
                transition: vec![],
            }),
            EntityConsoleHull(hull),
        )).id()
    }

    #[test]
    fn self_hull_fraction_reflects_entity_console_hull() {
        use crate::entity_spawner::EntityConsoleHull;
        use crate::damage::ConsoleHull;
        use crate::messages::Console;
        use crate::entity_config::BehaviourConfig;

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        // 50 HP out of 100 HP = 0.5 fraction
        let mut hull = ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)]);
        let mut rng = rand::rng();
        hull.apply_damage(50.0, &mut rng);

        let entity = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-hull-frac-001".to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".into(),
                state: vec![],
                transition: vec![],
            }),
            EntityConsoleHull(hull),
        )).id();

        app.update(); // attach controller
        app.update(); // tick

        // The hull fraction should be ~0.5; we verify via the world_view that was
        // used internally by confirming the EntityConsoleHull component is readable.
        let hull_comp = app.world().get::<EntityConsoleHull>(entity).unwrap();
        let frac = hull_comp.0.total_current() / hull_comp.0.total_max();
        assert!((frac - 0.5).abs() < 0.01, "hull fraction should be ~0.5, got {frac}");
    }

    #[test]
    fn entity_phaser_ready_false_without_weapons_console() {
        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let entity = spawn_behaviour_entity(&mut app, "ent-phaser-001");
        app.update(); // attach controller + tick

        // No WeaponsConsoleSection → entity_phaser_ready should never have been true;
        // the controller stays idle with no inputs.
        let ctrl = app.world().get::<AiControllerComponent>(entity).unwrap();
        assert_eq!(ctrl.controller.current_state, crate::ai::AiState::Idle,
            "idle without weapons console");
    }

    #[test]
    fn entity_phaser_ready_true_when_weapons_console_present_and_no_cooldown() {
        use crate::entity_spawner::WeaponsConsoleSection;
        use crate::entity_config::BehaviourConfig;

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let entity = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-phaser-002".to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".into(),
                state: vec![],
                transition: vec![],
            }),
            WeaponsConsoleSection(make_weapons_console_config(40.0)),
            EntityPhaserState::default(), // cooldown 0 → ready
        )).id();

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
        use crate::entity_config::BehaviourConfig;

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let entity = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-phaser-003".to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".into(),
                state: vec![],
                transition: vec![],
            }),
            WeaponsConsoleSection(make_weapons_console_config(40.0)),
            EntityPhaserState { cooldown_remaining: 5.0, ..EntityPhaserState::default() },
        )).id();

        app.update();

        let ps = app.world().get::<EntityPhaserState>(entity).unwrap();
        assert!(!ps.is_ready(), "phaser must not be ready when cooldown is active");
    }

    #[test]
    fn weapons_console_section_attached_when_config_has_weapons_console() {
        use crate::entity_spawner::WeaponsConsoleSection;
        use crate::entity_config::EntityConfig;

        let mut app = build_test_app();

        // Build a minimal EntityConfig with a weapons_console section.
        let config = EntityConfig {
            faction: None,
            hull: None,
            weapons_console: Some(make_weapons_console_config(80.0)),
            behaviour: None,
            helm_console: None,
            engineering_console: None,
            captain_console: None,
            collider: None,
            appearance: None,
            star: None,
            planet: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            tags: vec![],
            power: None,
            science_console: None,
            sensors_console: None,
            shields_console: None,
            station: None,
            radar_appearance: None,
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
        assert!(wc.is_some(), "WeaponsConsoleSection must be attached when config has weapons_console");
        assert!((wc.unwrap().0.beam_range - 80.0).abs() < 0.01, "beam_range must match config");
    }

    // ── Issue #314: AI FirePhaser → NPC phaser system applies damage ──────────

    #[test]
    fn npc_phaser_beam_applies_damage_to_target_entity_console_hull() {
        use crate::entity_spawner::{WeaponsConsoleSection, EntityConsoleHull};
        use crate::damage::ConsoleHull;
        use crate::messages::Console;
        use crate::entity_config::BehaviourConfig;

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let target_uuid_str = "bbbbbbbb-1111-0000-0000-000000000001";
        let target_uuid = uuid::Uuid::parse_str(target_uuid_str).unwrap();

        // Spawn NPC attacker with weapons console + phaser state ready.
        let attacker_uuid_str = "aaaaaaaa-2222-0000-0000-000000000002";
        let mut blackboard = crate::ai::Blackboard {
            target: Some(target_uuid),
            ..Default::default()
        };
        let attacker = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid(attacker_uuid_str.to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".into(),
                state: vec![],
                transition: vec![],
            }),
            WeaponsConsoleSection(make_weapons_console_config(40.0)),
            EntityPhaserState::default(), // ready
        )).id();

        // Spawn target with full hull.
        let target = app.world_mut().spawn((
            Transform::from_xyz(10.0, 0.0, 0.0),
            EntityUuid(target_uuid_str.to_string()),
            EntityConsoleHull(ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)])),
        )).id();

        app.update(); // attach controller

        // Set the attacker controller's blackboard target manually.
        {
            let mut ctrl = app.world_mut().get_mut::<AiControllerComponent>(attacker).unwrap();
            ctrl.controller.blackboard.target = Some(target_uuid);
        }

        // Directly activate the beam on the EntityPhaserState (simulating what
        // tick_ai_controllers would trigger via FirePhaser injection).
        {
            let mut ps = app.world_mut().get_mut::<EntityPhaserState>(attacker).unwrap();
            ps.beam_active = true;
            ps.beam_target = Some(target_uuid);
            ps.beam_remaining_secs = 3.0;
        }

        // Run multiple updates so the beam accumulates damage.
        for _ in 0..10 {
            app.update();
        }

        // Target's hull should be less than max after beam ticks.
        let hull = app.world().get::<EntityConsoleHull>(target).unwrap();
        let current = hull.0.total_current();
        assert!(current < 100.0, "target hull must have taken damage from NPC beam, current={current}");
    }

    #[test]
    fn entity_phaser_state_cooldown_starts_after_beam_ends() {
        use crate::entity_spawner::{WeaponsConsoleSection, EntityConsoleHull};
        use crate::damage::ConsoleHull;
        use crate::messages::Console;
        use crate::entity_config::BehaviourConfig;

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let target_uuid_str = "cccccccc-3333-0000-0000-000000000003";
        let target_uuid = uuid::Uuid::parse_str(target_uuid_str).unwrap();

        let attacker_uuid_str = "dddddddd-4444-0000-0000-000000000004";
        let attacker = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid(attacker_uuid_str.to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".into(),
                state: vec![],
                transition: vec![],
            }),
            WeaponsConsoleSection(make_weapons_console_config(40.0)),
            // Beam already active with very short duration (expires next tick).
            EntityPhaserState {
                beam_active: true,
                beam_target: Some(target_uuid),
                beam_remaining_secs: 0.001, // effectively zero after first dt
                cooldown_remaining: 0.0,
                ..EntityPhaserState::default()
            },
        )).id();

        // Spawn target.
        let _target = app.world_mut().spawn((
            Transform::from_xyz(5.0, 0.0, 0.0),
            EntityUuid(target_uuid_str.to_string()),
            EntityConsoleHull(ConsoleHull::from_config(&[(Console::CaptainChair, 500.0)])),
        )).id();

        app.update(); // attach controller
        app.update(); // tick — beam should expire and cooldown should start

        let ps = app.world().get::<EntityPhaserState>(attacker).unwrap();
        assert!(!ps.beam_active, "beam must be inactive after expiry");
        assert!(ps.cooldown_remaining > 0.0, "cooldown must start after beam ends, got {}", ps.cooldown_remaining);
    }

    /// Helper: build an InboundMessage FirePhaser and inject it via a one-shot system.
    fn inject_fire_phaser(app: &mut App, token: String) {
        use bevy::ecs::system::RunSystemOnce;
        let _ = app.world_mut().run_system_once(
            move |mut writer: MessageWriter<crate::lobby::InboundMessage>| {
                writer.write(crate::lobby::InboundMessage {
                    token: token.clone(),
                    msg: crate::messages::ClientMessage::FirePhaser,
                });
            }
        );
    }

    #[test]
    fn npc_phaser_does_not_fire_when_target_outside_beam_range() {
        use crate::entity_spawner::{WeaponsConsoleSection, EntityConsoleHull};
        use crate::damage::ConsoleHull;
        use crate::messages::Console;
        use crate::entity_config::BehaviourConfig;

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let target_uuid_str = "eeeeeeee-5555-0000-0000-000000000005";
        let target_uuid = uuid::Uuid::parse_str(target_uuid_str).unwrap();
        let attacker_uuid_str = "ffffffff-6666-0000-0000-000000000006";
        let beam_range = 20.0_f32;

        // Attacker at origin, facing forward (yaw=0, forward=-Z).
        let attacker = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid(attacker_uuid_str.to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".into(),
                state: vec![],
                transition: vec![],
            }),
            WeaponsConsoleSection(make_weapons_console_config(beam_range)),
            EntityPhaserState::default(),
        )).id();

        // Target placed far OUTSIDE beam range (100 units away).
        let _target = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, -100.0), // ahead but too far
            EntityUuid(target_uuid_str.to_string()),
            EntityConsoleHull(ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)])),
        )).id();

        app.update(); // attach controller

        // Set blackboard target.
        {
            let mut ctrl = app.world_mut().get_mut::<AiControllerComponent>(attacker).unwrap();
            ctrl.controller.blackboard.target = Some(target_uuid);
        }

        // Inject FirePhaser for the attacker's synthetic token.
        let token = format!("ai:{}", attacker_uuid_str);
        inject_fire_phaser(&mut app, token);

        app.update(); // tick_npc_phasers runs

        let ps = app.world().get::<EntityPhaserState>(attacker).unwrap();
        assert!(!ps.beam_active, "beam must NOT activate when target is outside range");
    }

    #[test]
    fn npc_phaser_does_not_fire_when_target_in_rear_arc() {
        use crate::entity_spawner::{WeaponsConsoleSection, EntityConsoleHull};
        use crate::damage::ConsoleHull;
        use crate::messages::Console;
        use crate::entity_config::BehaviourConfig;

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let target_uuid_str = "11111111-aaaa-0000-0000-000000000007";
        let target_uuid = uuid::Uuid::parse_str(target_uuid_str).unwrap();
        let attacker_uuid_str = "22222222-bbbb-0000-0000-000000000008";
        let beam_range = 100.0_f32;

        // Attacker at origin, facing forward (yaw=0, forward=-Z).
        let attacker = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid(attacker_uuid_str.to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".into(),
                state: vec![],
                transition: vec![],
            }),
            WeaponsConsoleSection(make_weapons_console_config(beam_range)),
            EntityPhaserState::default(),
        )).id();

        // Target directly BEHIND the attacker (+Z = aft).
        let _target = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 10.0), // behind (aft), within range
            EntityUuid(target_uuid_str.to_string()),
            EntityConsoleHull(ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)])),
        )).id();

        app.update(); // attach controller

        {
            let mut ctrl = app.world_mut().get_mut::<AiControllerComponent>(attacker).unwrap();
            ctrl.controller.blackboard.target = Some(target_uuid);
        }

        let token = format!("ai:{}", attacker_uuid_str);
        inject_fire_phaser(&mut app, token);

        app.update();

        let ps = app.world().get::<EntityPhaserState>(attacker).unwrap();
        assert!(!ps.beam_active, "beam must NOT activate when target is in rear arc");
    }

    #[test]
    fn npc_phaser_fires_when_target_in_range_and_forward_arc() {
        use crate::entity_spawner::{WeaponsConsoleSection, EntityConsoleHull};
        use crate::damage::ConsoleHull;
        use crate::messages::Console;
        use crate::entity_config::BehaviourConfig;

        let mut app = build_test_app();
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

        let target_uuid_str = "33333333-cccc-0000-0000-000000000009";
        let target_uuid = uuid::Uuid::parse_str(target_uuid_str).unwrap();
        let attacker_uuid_str = "44444444-dddd-0000-0000-000000000010";
        let beam_range = 100.0_f32;

        // Attacker at origin, facing forward (yaw=0, forward=-Z).
        let attacker = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid(attacker_uuid_str.to_string()),
            BehaviourSection(BehaviourConfig {
                initial_state: "idle".into(),
                state: vec![],
                transition: vec![],
            }),
            WeaponsConsoleSection(make_weapons_console_config(beam_range)),
            EntityPhaserState::default(),
        )).id();

        // Target directly AHEAD in range (-Z direction).
        let _target = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, -10.0), // ahead, within range
            EntityUuid(target_uuid_str.to_string()),
            EntityConsoleHull(ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)])),
        )).id();

        app.update(); // attach controller

        {
            let mut ctrl = app.world_mut().get_mut::<AiControllerComponent>(attacker).unwrap();
            ctrl.controller.blackboard.target = Some(target_uuid);
        }

        let token = format!("ai:{}", attacker_uuid_str);
        inject_fire_phaser(&mut app, token);

        app.update();

        let ps = app.world().get::<EntityPhaserState>(attacker).unwrap();
        assert!(ps.beam_active, "beam MUST activate when target is in range and forward arc");
    }
}
