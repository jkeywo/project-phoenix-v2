use crate::messages::ShieldFacingStatus;
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
/// Pure AI controller module — no Bevy imports.
///
/// Owns the `AiController` struct, the fixed five-slot `Blackboard`,
/// `AiInput`, `AiTickOutput`, the pure `tick` function, and the
/// `should_emit` edge-emission filter.
use uuid::Uuid;

// ── AiState ───────────────────────────────────────────────────────────────────

/// The set of states an AI controller can be in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum AiState {
    #[default]
    Idle,
    /// Navigate between a list of map-anchor waypoints in order.
    Patrolling {
        /// Ordered anchor names to visit.
        waypoints: Vec<String>,
        /// If true, loop back to the first waypoint after the last.
        loop_path: bool,
        /// Desired thrust fraction [0, 1].
        target_speed: f32,
    },
    /// Steer toward the blackboard `target` at `target_speed`.
    /// No-op (emits nothing) when `target` is `None`.
    Pursuing {
        /// Desired thrust fraction [0, 1].
        target_speed: f32,
    },
    /// Close to within `maintain_range` of the target and hold station there,
    /// stripping shields with phasers and landing torpedoes on depleted facings.
    Attacking {
        /// Distance to maintain from target (world units). Thrust = 0 when inside.
        maintain_range: f32,
        /// Desired thrust fraction [0, 1] when outside `maintain_range`.
        target_speed: f32,
    },
    /// Flee from `last_attacker` at `target_speed`, steering 180° away.
    /// Falls back to hold-heading when `last_attacker` is absent.
    Fleeing {
        /// Desired thrust fraction [0, 1].
        target_speed: f32,
    },
    /// Hold current heading at `target_speed` for `duration_secs`, then self-despawn.
    WarpingOut {
        /// Seconds to maintain heading before despawning.
        duration_secs: f32,
        /// Desired thrust fraction [0, 1].
        target_speed: f32,
    },
}

impl AiState {
    /// Returns the canonical kind name for this state variant.
    /// Used to match against `StateConfig.kind` when building states.
    pub fn kind_name(&self) -> &'static str {
        match self {
            AiState::Idle => "idle",
            AiState::Patrolling { .. } => "patrolling",
            AiState::Pursuing { .. } => "pursuing",
            AiState::Attacking { .. } => "attacking",
            AiState::Fleeing { .. } => "fleeing",
            AiState::WarpingOut { .. } => "warping_out",
        }
    }
}

// ── StringOrVec ───────────────────────────────────────────────────────────────

/// A Serde helper that accepts either a single string or a list of strings.
///
/// Used by `TransitionConfig.from` so that TOML files can write either
/// `from = "patrol"` or `from = ["patrol", "idle"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrVec {
    Single(String),
    Multi(Vec<String>),
}

impl StringOrVec {
    /// Returns `true` if this value contains `s`.
    pub fn contains(&self, s: &str) -> bool {
        match self {
            StringOrVec::Single(v) => v == s,
            StringOrVec::Multi(v) => v.iter().any(|x| x == s),
        }
    }
}

// ── TransitionConfig ──────────────────────────────────────────────────────────

/// A single declarative transition rule.
///
/// Transitions are evaluated in declaration order; the first whose `from`
/// matches the current state name AND whose `condition` evaluates to `true`
/// fires and causes a state change to `to`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionConfig {
    /// State name(s) this transition may fire from (matches `current_state_name`).
    pub from: StringOrVec,
    /// Name of the state to transition to.
    pub to: String,
    /// Condition kind: `"enemy_in_range"`, `"on_attacked"`, `"target_destroyed"`,
    /// `"in_weapons_range"`, `"hull_below"`, `"on_timer"`, or `"on_scenario_unloaded"`.
    pub condition: String,
    /// Radius parameter for `enemy_in_range` (ignored by other conditions).
    #[serde(default)]
    pub radius: Option<f32>,
    /// Hull fraction threshold for `hull_below` (fires when `self_hull_fraction < threshold`).
    #[serde(default)]
    pub threshold: Option<f32>,
    /// Elapsed seconds for `on_timer` (fires when `sim_time - state_entered_at >= seconds`).
    #[serde(default)]
    pub seconds: Option<f32>,
}

// ── Constants ─────────────────────────────────────────────────────────────────

/// Arrival radius in world units — closer than this counts as "reached waypoint".
/// Used as the serde default for `BehaviourConfig::waypoint_arrival_radius`.
pub const WAYPOINT_ARRIVAL_RADIUS: f32 = 20.0;
/// Angular deadband for `steer_toward`: within this angle, steering = 0.
pub const PATROL_DEADBAND_RAD: f32 = 0.05;
/// Angular error at which steering saturates to ±1.
pub const PATROL_FULL_STEER_RAD: f32 = PI / 4.0;
/// Extra clearance (world units) added on top of the sum of radii for both
/// target-approach offsetting and predictive collision avoidance.
/// Used as the serde default for `BehaviourConfig::avoidance_buffer`.
pub const AVOIDANCE_BUFFER: f32 = 5.0;
/// Look-ahead horizon (seconds) for predictive collision avoidance.
/// Used as the serde default for `BehaviourConfig::avoidance_look_ahead_secs`.
pub const AVOIDANCE_LOOK_AHEAD_SECS: f32 = 3.0;

// ── AiInput ───────────────────────────────────────────────────────────────────

/// The set of actions an AI controller can emit, mirroring the client
/// message vocabulary so the simulation can process them identically.
#[derive(Debug, Clone, PartialEq)]
pub enum AiInput {
    Helm {
        thrust: f32,
        steering: f32,
    },
    SetTarget {
        uuid: Uuid,
    },
    FirePhaser,
    FireTorpedo {
        tube: String,
    },
    Hail {
        target_uuid: Uuid,
    },
    RespondToMessage {
        message_id: String,
        response_index: usize,
    },
}

// ── Blackboard ────────────────────────────────────────────────────────────────

/// Fixed five-slot blackboard per AI controller.
#[derive(Debug, Clone, PartialEq)]
pub struct Blackboard {
    /// UUID of the current attack/follow target.
    pub target: Option<Uuid>,
    /// UUID of the entity that last damaged this controller's entity.
    pub last_attacker: Option<Uuid>,
    /// World-space spawn position, used as the "home" retreat anchor.
    pub home_position: [f32; 3],
    /// Index into a waypoint list (for patrol behaviours).
    pub waypoint_index: usize,
    /// Simulation time (seconds) when the current state was entered.
    pub state_entered_at: f64,
    /// `true` = the `on_attacked` condition is available to fire once in this
    /// state entry.  Set to `false` when `on_attacked` fires; reset to `true`
    /// when a non-`on_attacked` transition enters a new state.
    pub on_attacked_armed: bool,
}

impl Default for Blackboard {
    fn default() -> Self {
        Blackboard {
            target: None,
            last_attacker: None,
            home_position: [0.0, 0.0, 0.0],
            waypoint_index: 0,
            state_entered_at: 0.0,
            on_attacked_armed: true,
        }
    }
}

// ── AiController ─────────────────────────────────────────────────────────────

/// Per-entity AI controller state.
#[derive(Debug, Clone, PartialEq)]
pub struct AiController {
    pub current_state: AiState,
    /// The state name from `StateConfig.name` for the currently active state.
    /// Used to match `TransitionConfig.from` entries.  Defaults to `"idle"`.
    pub current_state_name: String,
    pub blackboard: Blackboard,
    /// Per-entity nav tuning, sourced from `[behaviour]` TOML fields.
    pub waypoint_arrival_radius: f32,
    pub avoidance_buffer: f32,
    pub avoidance_look_ahead_secs: f32,
}

impl AiController {
    /// Create a new controller in `Idle` state with the given spawn position
    /// and the simulation time at construction.
    pub fn new(home_position: [f32; 3], state_entered_at: f64) -> Self {
        Self {
            current_state: AiState::Idle,
            current_state_name: "idle".to_string(),
            blackboard: Blackboard {
                home_position,
                state_entered_at,
                ..Default::default()
            },
            waypoint_arrival_radius: WAYPOINT_ARRIVAL_RADIUS,
            avoidance_buffer: AVOIDANCE_BUFFER,
            avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        }
    }
}

// ── WorldView ─────────────────────────────────────────────────────────────────

/// A visible entity in the AI's world view.
#[derive(Debug, Clone, Default)]
pub struct AiWorldEntity {
    /// Stable UUID of the entity.
    pub uuid: Uuid,
    /// World-space position [x, y, z].
    pub position: [f32; 3],
    /// Faction UUID, if any.
    pub faction: Option<Uuid>,
    /// Four-quadrant shield state (from the entity broadcast), if the entity has shields.
    pub shields: Option<Vec<ShieldFacingStatus>>,
    /// Hull integrity fraction [0, 1], if known.
    pub hull_fraction: Option<f32>,
    /// Yaw in radians (Y-up, forward = -Z at yaw 0), if known.
    pub yaw: Option<f32>,
    /// Physical radius of the entity (world units) used for collision avoidance.
    pub radius: f32,
    /// Current forward speed of the entity (world units/s) used for predictive avoidance.
    pub forward_speed: f32,
}

/// A read-only snapshot of world state visible to the AI.
#[derive(Debug, Clone, Default)]
pub struct WorldView {
    /// Current simulation time in seconds.
    pub sim_time: f64,
    /// Current entity position [x, y, z].
    pub entity_pos: [f32; 3],
    /// Current entity yaw in radians (Y-up, forward = -Z at yaw 0).
    pub entity_yaw: f32,
    /// Named map anchors: name → [x, y, z].
    pub anchors: std::collections::HashMap<String, [f32; 3]>,
    /// All other entities currently visible to this AI.
    pub entities: Vec<AiWorldEntity>,
    /// UUID of an entity that attacked this entity during this tick, if any.
    pub attacker_this_tick: Option<Uuid>,
    /// Faction of this AI entity itself (used by `enemy_in_range`).
    pub self_faction: Option<Uuid>,
    /// Effective beam/weapons range of this AI entity (world units), if it has weapons.
    /// Used by the `in_weapons_range` transition condition and `Attacking` state.
    pub entity_weapons_range: Option<f32>,
    /// `true` when the AI entity's phasers are ready to fire this tick.
    pub entity_phaser_ready: bool,
    /// Name of the first ready torpedo tube, if any. `None` = no tubes loaded.
    pub torpedo_tube_ready: Option<String>,
    /// Hull integrity fraction [0, 1] of the AI entity itself (for `hull_below` condition).
    pub self_hull_fraction: Option<f32>,
    /// Set to `true` when the current scenario is being unloaded.
    /// Causes `on_scenario_unloaded` transitions to fire.
    pub scenario_unloaded: bool,
    /// Physical radius of this AI entity (world units), used for collision avoidance.
    pub self_radius: f32,
}

// ── AiTickOutput ─────────────────────────────────────────────────────────────

/// The output of a single AI tick.
#[derive(Debug, Clone, PartialEq)]
pub struct AiTickOutput {
    /// The state the controller should transition to (may be the same state).
    pub new_state: AiState,
    /// The name of `new_state` from `StateConfig.name`.
    /// `None` = no state name change (stay in current state).
    /// `Some(name)` = transition fired; caller should update `current_state_name`.
    pub new_state_name: Option<String>,
    /// Input actions to inject into the simulation this tick.
    pub inputs: Vec<AiInput>,
    /// Updated blackboard to write back to the controller. `None` = unchanged.
    pub new_blackboard: Option<Blackboard>,
    /// When `true`, the entity should be despawned this tick (end of `warping_out`).
    pub despawn: bool,
}

impl AiTickOutput {
    fn idle() -> Self {
        Self {
            new_state: AiState::Idle,
            new_state_name: None,
            inputs: vec![],
            new_blackboard: None,
            despawn: false,
        }
    }
}

// ── steer_toward ─────────────────────────────────────────────────────────────

/// Proportional steering toward a direction, with deadband and saturation.
///
/// - `yaw`: current entity yaw in radians (forward = `(sin(yaw), -cos(yaw))` in XZ).
/// - `target_dir`: 2-element [dx, dz] unit vector pointing at target.
/// - `deadband_rad`: angular error below which steering = 0.
/// - `full_steer_rad`: angular error at which steering saturates to ±1.
///
/// Returns a steering value in [-1, 1].
pub fn steer_toward(yaw: f32, target_dir: [f32; 2], deadband_rad: f32, full_steer_rad: f32) -> f32 {
    // Ship forward in XZ: (sin(yaw), -cos(yaw))
    let fwd_x = yaw.sin();
    let fwd_z = -yaw.cos();

    // Signed angle from forward to target: use 2D cross product for sign, dot for cosine.
    // cross = fwd × target = fwd_x * target_z - fwd_z * target_x  (positive = target is to the right)
    let cross = fwd_x * target_dir[1] - fwd_z * target_dir[0];
    let dot = fwd_x * target_dir[0] + fwd_z * target_dir[1];
    let angle = cross.atan2(dot); // radians in (-π, π]

    if angle.abs() < deadband_rad {
        return 0.0;
    }

    // Proportional, clamped to [-1, 1]
    let ratio = angle / full_steer_rad;
    ratio.clamp(-1.0, 1.0)
}

// ── Collision avoidance helpers ───────────────────────────────────────────────

/// Returns the nav point the AI should steer toward when approaching `target_pos`.
///
/// Instead of flying into the target's center the AI aims for a point on the
/// boundary sphere at distance `min_dist` from `target_pos`, measured along the
/// current approach vector.  When the AI is already inside that boundary the
/// returned point is the nearest boundary point (so the AI steers *away*).
///
/// `min_dist` should be `self_radius + target_radius + AVOIDANCE_BUFFER`.
fn offset_approach_target(self_pos: [f32; 3], target_pos: [f32; 3], min_dist: f32) -> [f32; 3] {
    let dx = self_pos[0] - target_pos[0];
    let dz = self_pos[2] - target_pos[2];
    let dist = (dx * dx + dz * dz).sqrt();
    if dist < 1.0 {
        // Directly on top — return a point directly ahead of self; caller handles it.
        return target_pos;
    }
    // Unit vector from target toward self.
    let inv_dist = 1.0 / dist;
    let ux = dx * inv_dist;
    let uz = dz * inv_dist;
    // Nav point = target center + unit_toward_self * min_dist
    [
        target_pos[0] + ux * min_dist,
        target_pos[1],
        target_pos[2] + uz * min_dist,
    ]
}

/// Computes a proportional avoidance steering correction in `[-1, 1]`.
///
/// For each entity in `world_entities` (excluding `excluded_uuid`) the function
/// projects both the AI's position and the entity's position forward by
/// `AVOIDANCE_LOOK_AHEAD_SECS` using their respective headings and speeds.
/// If the projected separation is less than the combined avoidance radius
/// (`self_radius + entity.radius + AVOIDANCE_BUFFER`) the entity is a threat.
///
/// The direction of the correction is determined by the signed lateral offset of
/// the threat's projected position relative to the AI's heading (using a 2-D
/// cross product):
/// - Threat is to the **right** → steer **left** (negative)
/// - Threat is to the **left** → steer **right** (positive)
///
/// The avoidance contribution from each threat is proportional to penetration
/// depth:
///
/// ```text
/// threat_fraction = 1 − (projected_dist / avoidance_radius)   ∈ (0, 1]
/// ```
///
/// Contributions are summed (preserving sign) and clamped to `[-1, 1]`.
/// Returns `0.0` when no threats are detected.
fn avoidance_steering(
    self_pos: [f32; 3],
    self_yaw: f32,
    self_speed: f32,
    self_radius: f32,
    excluded_uuid: Uuid,
    world_entities: &[AiWorldEntity],
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
) -> f32 {
    let fwd_x = self_yaw.sin();
    let fwd_z = -self_yaw.cos();
    let proj_self_x = self_pos[0] + fwd_x * self_speed * avoidance_look_ahead_secs;
    let proj_self_z = self_pos[2] + fwd_z * self_speed * avoidance_look_ahead_secs;

    let mut total_avoidance: f32 = 0.0;

    for entity in world_entities {
        if entity.uuid == excluded_uuid {
            continue;
        }
        let avoidance_radius = self_radius + entity.radius + avoidance_buffer;

        // Project the other entity forward using its yaw and forward_speed.
        let (ent_proj_x, ent_proj_z) = if let Some(ent_yaw) = entity.yaw {
            let ent_fwd_x = ent_yaw.sin();
            let ent_fwd_z = -ent_yaw.cos();
            (
                entity.position[0] + ent_fwd_x * entity.forward_speed * avoidance_look_ahead_secs,
                entity.position[2] + ent_fwd_z * entity.forward_speed * avoidance_look_ahead_secs,
            )
        } else {
            // Static entity — use current position.
            (entity.position[0], entity.position[2])
        };

        let ddx = proj_self_x - ent_proj_x;
        let ddz = proj_self_z - ent_proj_z;
        let proj_dist = (ddx * ddx + ddz * ddz).sqrt();

        if proj_dist < avoidance_radius {
            // Proportional: deepest penetration → strongest correction.
            let threat_fraction = 1.0 - (proj_dist / avoidance_radius);

            // Determine which side of the AI's heading the threat's projected
            // position falls on using the 2-D cross product:
            //   fwd × to_threat = fwd_x * to_threat_z - fwd_z * to_threat_x
            // Positive → threat is to the right → steer left (subtract).
            // Negative → threat is to the left  → steer right (add).
            let to_x = ent_proj_x - proj_self_x;
            let to_z = ent_proj_z - proj_self_z;
            let cross = fwd_x * to_z - fwd_z * to_x;
            // cross > 0 means threat is to the right; we steer left (negative).
            let sign = if cross >= 0.0 { -1.0_f32 } else { 1.0_f32 };
            total_avoidance += sign * threat_fraction;
        }
    }

    total_avoidance.clamp(-1.0, 1.0)
}

/// Advance an AI controller by one tick.
///
/// Pure function: takes a controller, a world view, the behaviour config (for
/// transition rules and state parameter lookup), and the faction registry (for
/// `enemy_in_range` condition). Returns the new state and any inputs to emit.
///
/// The caller is responsible for applying `new_state`, `new_blackboard`, and
/// `current_state_name` back to the controller.
pub fn tick(
    controller: &AiController,
    world_view: &WorldView,
    behaviour: &crate::entity_config::BehaviourConfig,
    faction_registry: &crate::faction::FactionRegistry,
) -> AiTickOutput {
    // Evaluate transitions first (declaration order); first match fires.
    if let Some(transition_output) =
        evaluate_transitions(controller, world_view, behaviour, faction_registry)
    {
        return transition_output;
    }

    match &controller.current_state {
        AiState::Idle => AiTickOutput::idle(),
        AiState::Patrolling {
            waypoints,
            loop_path,
            target_speed,
        } => tick_patrolling(controller, world_view, waypoints, *loop_path, *target_speed),
        AiState::Pursuing { target_speed } => tick_pursuing(controller, world_view, *target_speed),
        AiState::Attacking {
            maintain_range,
            target_speed,
        } => tick_attacking(controller, world_view, *maintain_range, *target_speed),
        AiState::Fleeing { target_speed } => tick_fleeing(controller, world_view, *target_speed),
        AiState::WarpingOut {
            duration_secs,
            target_speed,
        } => tick_warping_out(controller, world_view, *duration_secs, *target_speed),
    }
}

/// Evaluate all transitions in declaration order; return the first that fires.
fn tick_pursuing(
    controller: &AiController,
    world_view: &WorldView,
    target_speed: f32,
) -> AiTickOutput {
    // No target → no-op
    let target_uuid = match controller.blackboard.target {
        Some(uuid) => uuid,
        None => {
            return AiTickOutput {
                new_state: controller.current_state.clone(),
                new_state_name: None,
                inputs: vec![],
                new_blackboard: None,
                despawn: false,
            }
        }
    };

    // Find target entity in world view
    let target_entity = match world_view.entities.iter().find(|e| e.uuid == target_uuid) {
        Some(e) => e,
        None => {
            return AiTickOutput {
                new_state: controller.current_state.clone(),
                new_state_name: None,
                inputs: vec![],
                new_blackboard: None,
                despawn: false,
            }
        }
    };

    let pos = world_view.entity_pos;
    let dx = target_entity.position[0] - pos[0];
    let dz = target_entity.position[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    if dist < 1.0 {
        return AiTickOutput {
            new_state: controller.current_state.clone(),
            new_state_name: None,
            inputs: vec![AiInput::Helm {
                thrust: 0.0,
                steering: 0.0,
            }],
            new_blackboard: None,
            despawn: false,
        };
    }

    // Approach to the boundary of the target rather than its center.
    let min_dist = world_view.self_radius + target_entity.radius + controller.avoidance_buffer;
    let nav_point = offset_approach_target(pos, target_entity.position, min_dist);
    let ndx = nav_point[0] - pos[0];
    let ndz = nav_point[2] - pos[2];
    let nav_dist = (ndx * ndx + ndz * ndz).sqrt();

    let inv_dist = 1.0 / nav_dist.max(0.001);
    let dir = [ndx * inv_dist, ndz * inv_dist];
    let base_steering = steer_toward(
        world_view.entity_yaw,
        dir,
        PATROL_DEADBAND_RAD,
        PATROL_FULL_STEER_RAD,
    );

    // Blend in proportional avoidance from all nearby entities.
    let avoid = avoidance_steering(
        pos,
        world_view.entity_yaw,
        target_speed.abs(),
        world_view.self_radius,
        target_uuid,
        &world_view.entities,
        controller.avoidance_buffer,
        controller.avoidance_look_ahead_secs,
    );
    let steering = (base_steering + avoid).clamp(-1.0, 1.0);

    AiTickOutput {
        new_state: controller.current_state.clone(),
        new_state_name: None,
        inputs: vec![AiInput::Helm {
            thrust: target_speed,
            steering,
        }],
        new_blackboard: None,
        despawn: false,
    }
}

// ── facing_quadrant_depleted ──────────────────────────────────────────────────

/// Returns the shield label for the quadrant of `target` facing the AI's position.
///
/// Quadrant is determined by projecting `(ai_pos - target_pos)` into the
/// target's local frame:
/// - `atan2(dot_right, dot_fwd)` in `[-π/4, π/4]` → "Fore"
/// - in `[3π/4, π] ∪ [-π, -3π/4]` → "Aft"
/// - in `[π/4, 3π/4]` → "Starboard"
/// - in `[-3π/4, -π/4]` → "Port"
fn facing_quadrant_label(ai_pos: [f32; 3], target_pos: [f32; 3], target_yaw: f32) -> &'static str {
    let dx = ai_pos[0] - target_pos[0];
    let dz = ai_pos[2] - target_pos[2];
    // Target's local forward (XZ): (-sin(yaw), -cos(yaw))
    let fwd_x = -target_yaw.sin();
    let fwd_z = -target_yaw.cos();
    // Target's local right (starboard), CCW-perpendicular of forward in XZ: (cos(yaw), -sin(yaw))
    let right_x = target_yaw.cos();
    let right_z = -target_yaw.sin();
    let dot_fwd = dx * fwd_x + dz * fwd_z;
    let dot_right = dx * right_x + dz * right_z;
    let angle = dot_right.atan2(dot_fwd); // in (-π, π]
    let quarter = PI / 4.0;
    let three_quarter = 3.0 * PI / 4.0;
    if angle >= -quarter && angle < quarter {
        "Fore"
    } else if angle >= three_quarter || angle < -three_quarter {
        "Aft"
    } else if angle >= quarter {
        "Starboard"
    } else {
        "Port"
    }
}

/// Returns `true` if the given shield facing is depleted (offline or HP ≤ 0).
fn shield_facing_depleted(f: &ShieldFacingStatus) -> bool {
    !f.online || f.hp <= 0
}

/// Returns `true` when the AI should fire a torpedo at the target.
///
/// Torpedoes fire when:
/// - A tube is ready (`torpedo_tube_ready` is `Some`), AND
/// - The target has no shield data (treat as always-down), OR
///   the facing quadrant's shield is depleted.
fn should_fire_torpedo(world_view: &WorldView, target: &AiWorldEntity) -> bool {
    if world_view.torpedo_tube_ready.is_none() {
        return false;
    }
    match &target.shields {
        None => true, // no shield data → treat as always-down
        Some(facings) if facings.is_empty() => true,
        Some(facings) => {
            let target_yaw = target.yaw.unwrap_or(0.0);
            let label = facing_quadrant_label(world_view.entity_pos, target.position, target_yaw);
            facings
                .iter()
                .find(|f| f.label == label)
                .map(shield_facing_depleted)
                .unwrap_or(true) // quadrant not found → treat as down
        }
    }
}

fn tick_attacking(
    controller: &AiController,
    world_view: &WorldView,
    maintain_range: f32,
    target_speed: f32,
) -> AiTickOutput {
    let target_uuid = match controller.blackboard.target {
        Some(uuid) => uuid,
        None => {
            return AiTickOutput {
                new_state: controller.current_state.clone(),
                new_state_name: None,
                inputs: vec![],
                new_blackboard: None,
                despawn: false,
            }
        }
    };

    let target_entity = match world_view.entities.iter().find(|e| e.uuid == target_uuid) {
        Some(e) => e,
        None => {
            return AiTickOutput {
                new_state: controller.current_state.clone(),
                new_state_name: None,
                inputs: vec![],
                new_blackboard: None,
                despawn: false,
            }
        }
    };

    let pos = world_view.entity_pos;
    let dx = target_entity.position[0] - pos[0];
    let dz = target_entity.position[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    // Minimum approach distance accounts for physical radii so the AI never
    // tries to fly into the target's hull.
    let min_dist = world_view.self_radius + target_entity.radius + controller.avoidance_buffer;
    let effective_range = maintain_range.max(min_dist);

    // Steer toward target always.
    let (steering, thrust) = if dist < 1.0 {
        (0.0_f32, 0.0_f32)
    } else {
        let nav_point = offset_approach_target(pos, target_entity.position, min_dist);
        let ndx = nav_point[0] - pos[0];
        let ndz = nav_point[2] - pos[2];
        let nav_dist = (ndx * ndx + ndz * ndz).sqrt();
        let inv_dist = 1.0 / nav_dist.max(0.001);
        let dir = [ndx * inv_dist, ndz * inv_dist];
        let base_s = steer_toward(
            world_view.entity_yaw,
            dir,
            PATROL_DEADBAND_RAD,
            PATROL_FULL_STEER_RAD,
        );
        let avoid = avoidance_steering(
            pos,
            world_view.entity_yaw,
            target_speed.abs(),
            world_view.self_radius,
            target_uuid,
            &world_view.entities,
            controller.avoidance_buffer,
            controller.avoidance_look_ahead_secs,
        );
        let s = (base_s + avoid).clamp(-1.0, 1.0);
        // Thrust: 0 inside effective orbit range, target_speed outside; never reverses.
        let t = if dist <= effective_range {
            0.0
        } else {
            target_speed
        };
        (s, t)
    };

    let mut inputs = vec![AiInput::Helm { thrust, steering }];

    // Fire phasers when in beam range and ready.
    let beam_range = world_view
        .entity_weapons_range
        .unwrap_or(crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE);
    if dist <= beam_range && world_view.entity_phaser_ready {
        inputs.push(AiInput::SetTarget { uuid: target_uuid });
        inputs.push(AiInput::FirePhaser);
    }

    // Fire torpedo when facing quadrant is depleted (or target has no shields).
    if should_fire_torpedo(world_view, target_entity) {
        let tube = world_view.torpedo_tube_ready.clone().unwrap(); // safe: checked in should_fire_torpedo
        inputs.push(AiInput::FireTorpedo { tube });
    }

    AiTickOutput {
        new_state: controller.current_state.clone(),
        new_state_name: None,
        inputs,
        new_blackboard: None,
        despawn: false,
    }
}

/// Flee at `target_speed`, steering 180° away from `last_attacker`.
/// Falls back to hold-heading (steering = 0) when `last_attacker` is absent
/// or not found in the world view.
fn tick_fleeing(
    controller: &AiController,
    world_view: &WorldView,
    target_speed: f32,
) -> AiTickOutput {
    let base_steering = if let Some(attacker_uuid) = controller.blackboard.last_attacker {
        if let Some(attacker) = world_view.entities.iter().find(|e| e.uuid == attacker_uuid) {
            let pos = world_view.entity_pos;
            let dx = pos[0] - attacker.position[0]; // direction AWAY from attacker
            let dz = pos[2] - attacker.position[2];
            let dist = (dx * dx + dz * dz).sqrt();
            if dist > 1.0 {
                let inv_dist = 1.0 / dist;
                let away_dir = [dx * inv_dist, dz * inv_dist];
                steer_toward(
                    world_view.entity_yaw,
                    away_dir,
                    PATROL_DEADBAND_RAD,
                    PATROL_FULL_STEER_RAD,
                )
            } else {
                0.0 // effectively on top of attacker, hold heading
            }
        } else {
            0.0 // attacker not in world view, hold heading
        }
    } else {
        0.0 // no last_attacker, hold heading
    };

    let pos = world_view.entity_pos;
    let avoid = avoidance_steering(
        pos,
        world_view.entity_yaw,
        target_speed.abs(),
        world_view.self_radius,
        controller.blackboard.last_attacker.unwrap_or(Uuid::nil()),
        &world_view.entities,
        controller.avoidance_buffer,
        controller.avoidance_look_ahead_secs,
    );
    let steering = (base_steering + avoid).clamp(-1.0, 1.0);

    AiTickOutput {
        new_state: controller.current_state.clone(),
        new_state_name: None,
        inputs: vec![AiInput::Helm {
            thrust: target_speed,
            steering,
        }],
        new_blackboard: None,
        despawn: false,
    }
}

/// Hold current heading at `target_speed` for `duration_secs`, then self-despawn.
fn tick_warping_out(
    controller: &AiController,
    world_view: &WorldView,
    duration_secs: f32,
    target_speed: f32,
) -> AiTickOutput {
    let elapsed = (world_view.sim_time - controller.blackboard.state_entered_at) as f32;
    let should_despawn = elapsed >= duration_secs;

    AiTickOutput {
        new_state: controller.current_state.clone(),
        new_state_name: None,
        inputs: vec![AiInput::Helm {
            thrust: target_speed,
            steering: 0.0,
        }],
        new_blackboard: None,
        despawn: should_despawn,
    }
}

/// Evaluate all transitions in declaration order; return the first that fires.
///
/// When a transition fires, the returned `AiTickOutput` carries the new state
/// (built from `behaviour.state`) and a blackboard snapshot with:
/// - `state_entered_at` set to `world_view.sim_time`
/// - `on_attacked_armed` reset to `true` for non-`on_attacked` transitions
///   (so `on_attacked` can fire in the new state), or `false` for
///   `on_attacked`-triggered transitions (preventing an immediate re-fire).
/// - `target` / `last_attacker` populated for `on_attacked` and `enemy_in_range`.
/// - `target` cleared for `target_destroyed`.
fn evaluate_transitions(
    controller: &AiController,
    world_view: &WorldView,
    behaviour: &crate::entity_config::BehaviourConfig,
    faction_registry: &crate::faction::FactionRegistry,
) -> Option<AiTickOutput> {
    for transition in &behaviour.transition {
        if !transition.from.contains(&controller.current_state_name) {
            continue;
        }

        let mut new_bb = controller.blackboard.clone();
        new_bb.state_entered_at = world_view.sim_time;

        let fires = match transition.condition.as_str() {
            "on_attacked"
                if world_view.attacker_this_tick.is_some()
                    && controller.blackboard.on_attacked_armed =>
            {
                new_bb.target = world_view.attacker_this_tick;
                new_bb.last_attacker = world_view.attacker_this_tick;
                new_bb.on_attacked_armed = false; // suppress until next non-on_attacked entry
                true
            }
            "enemy_in_range" => {
                let radius = transition.radius.unwrap_or(100.0);
                let pos = world_view.entity_pos;
                let first_hostile = world_view.entities.iter().find(|e| {
                    let dx = e.position[0] - pos[0];
                    let dz = e.position[2] - pos[2];
                    let dist_sq = dx * dx + dz * dz;
                    dist_sq <= radius * radius
                        && crate::faction::is_enemy(
                            world_view.self_faction,
                            e.faction,
                            faction_registry,
                        )
                });
                if let Some(hostile) = first_hostile {
                    new_bb.target = Some(hostile.uuid);
                    new_bb.on_attacked_armed = true;
                    true
                } else {
                    false
                }
            }
            "target_destroyed" => {
                match controller.blackboard.target {
                    Some(target_uuid) => {
                        let present = world_view.entities.iter().any(|e| e.uuid == target_uuid);
                        if !present {
                            new_bb.target = None;
                            new_bb.on_attacked_armed = true;
                            true
                        } else {
                            false
                        }
                    }
                    None => {
                        // No target — treat as already destroyed so the AI
                        // transitions out of attack/pursue states.
                        new_bb.target = None;
                        new_bb.on_attacked_armed = true;
                        true
                    }
                }
            }
            "in_weapons_range" => {
                // Fires when the blackboard target is within the entity's weapons range.
                let weapons_range = world_view
                    .entity_weapons_range
                    .unwrap_or(crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE);
                if let Some(target_uuid) = controller.blackboard.target {
                    let pos = world_view.entity_pos;
                    let in_range = world_view.entities.iter().any(|e| {
                        if e.uuid != target_uuid {
                            return false;
                        }
                        let dx = e.position[0] - pos[0];
                        let dz = e.position[2] - pos[2];
                        dx * dx + dz * dz <= weapons_range * weapons_range
                    });
                    if in_range {
                        new_bb.on_attacked_armed = true;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            "hull_below" => {
                let threshold = transition.threshold.unwrap_or(0.5);
                match world_view.self_hull_fraction {
                    Some(frac) if frac < threshold => {
                        new_bb.on_attacked_armed = true;
                        true
                    }
                    _ => false,
                }
            }
            "on_timer" => {
                let seconds = transition.seconds.unwrap_or(f32::MAX) as f64;
                let elapsed = world_view.sim_time - controller.blackboard.state_entered_at;
                if elapsed >= seconds {
                    new_bb.on_attacked_armed = true;
                    true
                } else {
                    false
                }
            }
            "on_scenario_unloaded" if world_view.scenario_unloaded => {
                new_bb.on_attacked_armed = true;
                true
            }
            _ => false,
        };

        if fires {
            let new_state = build_state_by_name(behaviour, &transition.to);
            return Some(AiTickOutput {
                new_state,
                new_state_name: Some(transition.to.clone()),
                inputs: vec![],
                new_blackboard: Some(new_bb),
                despawn: false,
            });
        }
    }
    None
}

/// Build an `AiState` by looking up a state name in `behaviour.state`.
/// Falls back to `Idle` for `"idle"` or unknown names.
fn build_state_by_name(behaviour: &crate::entity_config::BehaviourConfig, name: &str) -> AiState {
    if name == "idle" {
        return AiState::Idle;
    }
    if let Some(sc) = behaviour.state.iter().find(|s| s.name == name) {
        match sc.kind.as_str() {
            "patrolling" => AiState::Patrolling {
                waypoints: sc.waypoints.clone(),
                loop_path: sc.loop_path,
                target_speed: sc.target_speed,
            },
            "pursuing" => AiState::Pursuing {
                target_speed: sc.target_speed,
            },
            "attacking" => AiState::Attacking {
                maintain_range: sc.maintain_range,
                target_speed: sc.target_speed,
            },
            "fleeing" => AiState::Fleeing {
                target_speed: sc.target_speed,
            },
            "warping_out" => AiState::WarpingOut {
                duration_secs: sc.duration_secs,
                target_speed: sc.target_speed,
            },
            _ => AiState::Idle,
        }
    } else {
        AiState::Idle
    }
}

fn tick_patrolling(
    controller: &AiController,
    world_view: &WorldView,
    waypoints: &[String],
    loop_path: bool,
    target_speed: f32,
) -> AiTickOutput {
    // No waypoints → stay idle
    if waypoints.is_empty() {
        return AiTickOutput::idle();
    }

    let idx = controller.blackboard.waypoint_index;

    // If we've exhausted waypoints and not looping, stop
    if idx >= waypoints.len() {
        return AiTickOutput {
            new_state: controller.current_state.clone(),
            new_state_name: None,
            inputs: vec![AiInput::Helm {
                thrust: 0.0,
                steering: 0.0,
            }],
            new_blackboard: None,
            despawn: false,
        };
    }

    let waypoint_name = &waypoints[idx];
    let anchor = match world_view.anchors.get(waypoint_name) {
        Some(a) => *a,
        None => {
            // Unknown anchor — emit nothing, stay put
            return AiTickOutput {
                new_state: controller.current_state.clone(),
                new_state_name: None,
                inputs: vec![AiInput::Helm {
                    thrust: 0.0,
                    steering: 0.0,
                }],
                new_blackboard: None,
                despawn: false,
            };
        }
    };

    let pos = world_view.entity_pos;
    let dx = anchor[0] - pos[0];
    let dz = anchor[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    // Check arrival
    if dist < controller.waypoint_arrival_radius {
        let next_idx = idx + 1;
        let (new_idx, still_patrolling) = if next_idx >= waypoints.len() {
            if loop_path {
                (0, true)
            } else {
                (next_idx, false) // idx >= len → stop
            }
        } else {
            (next_idx, true)
        };

        let mut new_bb = controller.blackboard.clone();
        new_bb.waypoint_index = new_idx;

        if !still_patrolling {
            return AiTickOutput {
                new_state: controller.current_state.clone(),
                new_state_name: None,
                inputs: vec![AiInput::Helm {
                    thrust: 0.0,
                    steering: 0.0,
                }],
                new_blackboard: Some(new_bb),
                despawn: false,
            };
        }

        return AiTickOutput {
            new_state: controller.current_state.clone(),
            new_state_name: None,
            inputs: vec![AiInput::Helm {
                thrust: target_speed,
                steering: 0.0,
            }],
            new_blackboard: Some(new_bb),
            despawn: false,
        };
    }

    // Steer toward waypoint, blended with collision avoidance.
    let inv_dist = 1.0 / dist;
    let dir = [dx * inv_dist, dz * inv_dist];
    let base_steering = steer_toward(
        world_view.entity_yaw,
        dir,
        PATROL_DEADBAND_RAD,
        PATROL_FULL_STEER_RAD,
    );
    let avoid = avoidance_steering(
        pos,
        world_view.entity_yaw,
        target_speed.abs(),
        world_view.self_radius,
        Uuid::nil(), // no target to exclude — all entities considered
        &world_view.entities,
        controller.avoidance_buffer,
        controller.avoidance_look_ahead_secs,
    );
    let steering = (base_steering + avoid).clamp(-1.0, 1.0);

    AiTickOutput {
        new_state: controller.current_state.clone(),
        new_state_name: None,
        inputs: vec![AiInput::Helm {
            thrust: target_speed,
            steering,
        }],
        new_blackboard: None,
        despawn: false,
    }
}

// ── build_initial_state ───────────────────────────────────────────────────────

/// Build the initial `AiState` from a `BehaviourConfig`.
///
/// Looks up the `initial_state` name in `config.state`; if a matching
/// `StateConfig` entry is found, builds the typed state from it.  Falls back
/// to `Idle` when the name is `"idle"` or there is no matching entry.
pub fn build_initial_state(config: &crate::entity_config::BehaviourConfig) -> AiState {
    build_state_by_name(config, &config.initial_state)
}

// ── should_emit ───────────────────────────────────────────────────────────────

/// Returns `true` if the change from `last` to `current` is significant enough
/// to warrant emitting a new input message.
///
/// Used to suppress redundant `HelmInput` messages when the joystick hasn't
/// moved more than `epsilon`.
pub fn should_emit(last: f32, current: f32, epsilon: f32) -> bool {
    (current - last).abs() > epsilon
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::{BehaviourConfig, StateConfig};
    use crate::faction::{FactionConfig, FactionRegistry};

    // ── Test helpers ──────────────────────────────────────────────────────

    fn empty_behaviour() -> BehaviourConfig {
        BehaviourConfig {
            initial_state: "idle".into(),
            state: vec![],
            transition: vec![],
            waypoint_arrival_radius: WAYPOINT_ARRIVAL_RADIUS,
            avoidance_buffer: AVOIDANCE_BUFFER,
            avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        }
    }

    fn empty_registry() -> FactionRegistry {
        FactionRegistry::new()
    }

    fn do_tick(controller: &AiController, world: &WorldView) -> AiTickOutput {
        tick(controller, world, &empty_behaviour(), &empty_registry())
    }

    fn do_tick_with(
        controller: &AiController,
        world: &WorldView,
        behaviour: &BehaviourConfig,
        registry: &FactionRegistry,
    ) -> AiTickOutput {
        tick(controller, world, behaviour, registry)
    }

    /// Construct a minimal `AiWorldEntity` with only required fields, rest defaulted.
    fn make_world_entity(uuid: Uuid, position: [f32; 3], faction: Option<Uuid>) -> AiWorldEntity {
        AiWorldEntity {
            uuid,
            position,
            faction,
            shields: None,
            hull_fraction: None,
            yaw: None,
            radius: 0.0,
            forward_speed: 0.0,
        }
    }
    fn fed_uuid() -> Uuid {
        Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap()
    }
    fn pirate_uuid() -> Uuid {
        Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap()
    }

    fn mutual_hostile_registry() -> FactionRegistry {
        let mut reg = FactionRegistry::new();
        reg.insert(FactionConfig {
            uuid: fed_uuid(),
            name: "Fed".into(),
            enemies: vec![pirate_uuid()],
        });
        reg.insert(FactionConfig {
            uuid: pirate_uuid(),
            name: "Pirate".into(),
            enemies: vec![fed_uuid()],
        });
        reg
    }

    // ── Tracer bullet: idle state emits nothing ────────────────────────────

    #[test]
    fn tick_idle_returns_no_inputs() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        let world = WorldView::default();
        let output = do_tick(&controller, &world);
        assert!(output.inputs.is_empty(), "idle must emit no inputs");
    }

    #[test]
    fn tick_idle_returns_idle_state() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        let world = WorldView::default();
        let output = do_tick(&controller, &world);
        assert_eq!(output.new_state, AiState::Idle);
    }

    // ── Blackboard initial seeding ─────────────────────────────────────────

    #[test]
    fn new_controller_seeds_home_position() {
        let pos = [10.0, 0.0, -5.0];
        let controller = AiController::new(pos, 42.5);
        assert_eq!(controller.blackboard.home_position, pos);
    }

    #[test]
    fn new_controller_seeds_state_entered_at() {
        let controller = AiController::new([0.0, 0.0, 0.0], 99.9);
        assert!((controller.blackboard.state_entered_at - 99.9).abs() < 1e-9);
    }

    #[test]
    fn new_controller_target_is_none() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        assert!(controller.blackboard.target.is_none());
    }

    #[test]
    fn new_controller_last_attacker_is_none() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        assert!(controller.blackboard.last_attacker.is_none());
    }

    #[test]
    fn new_controller_waypoint_index_is_zero() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        assert_eq!(controller.blackboard.waypoint_index, 0);
    }

    #[test]
    fn new_controller_starts_in_idle_state() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        assert_eq!(controller.current_state, AiState::Idle);
    }

    #[test]
    fn new_controller_on_attacked_armed_is_true() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        assert!(controller.blackboard.on_attacked_armed);
    }

    #[test]
    fn new_controller_state_name_is_idle() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        assert_eq!(controller.current_state_name, "idle");
    }

    // ── should_emit epsilon semantics ──────────────────────────────────────

    #[test]
    fn should_emit_returns_false_when_within_epsilon() {
        assert!(!should_emit(0.5, 0.5 + 0.001, 0.01));
    }

    #[test]
    fn should_emit_returns_true_when_outside_epsilon() {
        assert!(should_emit(0.0, 0.5, 0.01));
    }

    #[test]
    fn should_emit_returns_false_when_equal() {
        assert!(!should_emit(0.3, 0.3, 0.0001));
    }

    #[test]
    fn should_emit_returns_true_when_negative_delta_exceeds_epsilon() {
        assert!(should_emit(0.5, 0.0, 0.01));
    }

    #[test]
    fn should_emit_with_zero_epsilon_returns_false_only_when_equal() {
        assert!(!should_emit(0.5, 0.5, 0.0));
        assert!(should_emit(0.5, 0.5001, 0.0));
    }

    #[test]
    fn should_emit_at_exact_epsilon_boundary_returns_false() {
        // |current - last| == epsilon → NOT greater → returns false
        assert!(!should_emit(0.0, 0.1, 0.1));
    }

    #[test]
    fn should_emit_just_over_epsilon_boundary_returns_true() {
        assert!(should_emit(0.0, 0.10001, 0.1));
    }

    // ── steer_toward ──────────────────────────────────────────────────────

    #[test]
    fn steer_toward_aligned_returns_zero_within_deadband() {
        // Facing straight ahead (yaw=0), target straight ahead [0, -1] (−Z)
        let steering = steer_toward(0.0, [0.0, -1.0], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
        assert!(
            steering.abs() < 1e-6,
            "aligned: steering must be 0, got {steering}"
        );
    }

    #[test]
    fn steer_toward_90deg_right_saturates_to_positive_one() {
        // Facing forward (yaw=0, forward=[0,-1]), target to the right [1, 0]
        let steering = steer_toward(0.0, [1.0, 0.0], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
        assert!(
            (steering - 1.0).abs() < 1e-5,
            "90deg right: steering must be 1.0, got {steering}"
        );
    }

    #[test]
    fn steer_toward_90deg_left_saturates_to_negative_one() {
        // Target to the left [-1, 0]
        let steering = steer_toward(0.0, [-1.0, 0.0], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
        assert!(
            (steering + 1.0).abs() < 1e-5,
            "90deg left: steering must be -1.0, got {steering}"
        );
    }

    #[test]
    fn steer_toward_proportional_mid_error() {
        // Half of full_steer_rad to the right → steering ~= 0.5
        let half = PATROL_FULL_STEER_RAD / 2.0;
        let dir_x = half.sin();
        let dir_z = -half.cos();
        let dir_len = (dir_x * dir_x + dir_z * dir_z).sqrt();
        let dir = [dir_x / dir_len, dir_z / dir_len];
        let steering = steer_toward(0.0, dir, 0.0, PATROL_FULL_STEER_RAD);
        assert!(
            (steering - 0.5).abs() < 0.02,
            "half-angle: steering ~= 0.5, got {steering}"
        );
    }

    #[test]
    fn steer_toward_pi_wraparound_saturates() {
        // Target directly behind: [0, 1] (i.e. +Z), angle = ±π
        // Should saturate to ±1 (sign depends on cross product, but magnitude = 1)
        let steering = steer_toward(0.0, [0.0, 1.0], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
        assert!(
            steering.abs() > 0.99,
            "behind: steering magnitude must be ≈1, got {steering}"
        );
    }

    // ── tick_patrolling ───────────────────────────────────────────────────

    fn patrolling_controller(
        waypoints: Vec<String>,
        loop_path: bool,
        target_speed: f32,
    ) -> AiController {
        let mut c = AiController::new([0.0, 0.0, 0.0], 0.0);
        c.current_state = AiState::Patrolling {
            waypoints,
            loop_path,
            target_speed,
        };
        c.current_state_name = "patrol".to_string();
        c
    }

    fn two_waypoint_view(entity_pos: [f32; 3], entity_yaw: f32) -> WorldView {
        let mut anchors = std::collections::HashMap::new();
        anchors.insert("wp1".to_string(), [100.0, 0.0, 0.0]);
        anchors.insert("wp2".to_string(), [-100.0, 0.0, 0.0]);
        WorldView {
            entity_pos,
            entity_yaw,
            anchors,
            ..Default::default()
        }
    }

    #[test]
    fn patrolling_emits_thrust_equal_to_target_speed() {
        let ctrl = patrolling_controller(vec!["wp1".into()], false, 0.7);
        let view = two_waypoint_view([0.0, 0.0, 0.0], 0.0);
        let output = do_tick(&ctrl, &view);
        match output.inputs.first() {
            Some(AiInput::Helm { thrust, .. }) => {
                assert!(
                    (*thrust - 0.7).abs() < 1e-5,
                    "thrust must equal target_speed, got {thrust}"
                );
            }
            _ => panic!("expected Helm input"),
        }
    }

    #[test]
    fn patrolling_steers_toward_waypoint() {
        // Entity at origin, facing forward (yaw=0, forward = [0,-1]).
        // wp1 is at [100, 0, 0] which is to the right → positive steering.
        let ctrl = patrolling_controller(vec!["wp1".into()], false, 0.5);
        let view = two_waypoint_view([0.0, 0.0, 0.0], 0.0);
        let output = do_tick(&ctrl, &view);
        match output.inputs.first() {
            Some(AiInput::Helm { steering, .. }) => {
                assert!(
                    *steering > 0.0,
                    "should steer right toward wp1, got {steering}"
                );
            }
            _ => panic!("expected Helm input"),
        }
    }

    #[test]
    fn patrolling_advances_waypoint_index_on_arrival() {
        // Entity at wp1 position (within arrival radius)
        let ctrl = patrolling_controller(vec!["wp1".into(), "wp2".into()], false, 0.5);
        let view = two_waypoint_view([100.0, 0.0, 0.0], 0.0); // at wp1
        let output = do_tick(&ctrl, &view);
        let new_bb = output
            .new_blackboard
            .expect("new_blackboard must be set on arrival");
        assert_eq!(new_bb.waypoint_index, 1, "must advance to next waypoint");
    }

    #[test]
    fn patrolling_loops_when_last_waypoint_reached_and_loop_path_true() {
        let mut ctrl = patrolling_controller(vec!["wp1".into(), "wp2".into()], true, 0.5);
        ctrl.blackboard.waypoint_index = 1; // already at wp2 index
        let view = two_waypoint_view([-100.0, 0.0, 0.0], 0.0); // at wp2
        let output = do_tick(&ctrl, &view);
        let new_bb = output.new_blackboard.expect("new_blackboard must be set");
        assert_eq!(new_bb.waypoint_index, 0, "must loop back to waypoint 0");
    }

    #[test]
    fn patrolling_stops_when_last_waypoint_reached_and_not_looping() {
        let mut ctrl = patrolling_controller(vec!["wp1".into(), "wp2".into()], false, 0.5);
        ctrl.blackboard.waypoint_index = 1; // at last waypoint
        let view = two_waypoint_view([-100.0, 0.0, 0.0], 0.0); // at wp2
        let output = do_tick(&ctrl, &view);
        let new_bb = output.new_blackboard.expect("new_blackboard must be set");
        assert_eq!(
            new_bb.waypoint_index, 2,
            "index advances past end to signal stop"
        );
        // Next tick: idx >= len → emit thrust 0
        let mut ctrl2 = ctrl.clone();
        ctrl2.blackboard.waypoint_index = 2;
        let output2 = do_tick(&ctrl2, &view);
        match output2.inputs.first() {
            Some(AiInput::Helm { thrust, .. }) => {
                assert!(
                    (*thrust).abs() < 1e-5,
                    "must stop (thrust=0) after last waypoint"
                );
            }
            _ => panic!("expected Helm input"),
        }
    }

    // ── build_initial_state ────────────────────────────────────────────────

    #[test]
    fn build_initial_state_idle_name_returns_idle() {
        let config = empty_behaviour();
        assert_eq!(build_initial_state(&config), AiState::Idle);
    }

    #[test]
    fn build_initial_state_patrolling_builds_patrolling_variant() {
        let config = BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![StateConfig {
                name: "patrol".into(),
                kind: "patrolling".into(),
                waypoints: vec!["wp1".into(), "wp2".into()],
                loop_path: true,
                target_speed: 0.6,
                maintain_range: 0.0,
                duration_secs: 0.0,
            }],
            transition: vec![],
            waypoint_arrival_radius: WAYPOINT_ARRIVAL_RADIUS,
            avoidance_buffer: AVOIDANCE_BUFFER,
            avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        };
        let state = build_initial_state(&config);
        assert_eq!(
            state,
            AiState::Patrolling {
                waypoints: vec!["wp1".into(), "wp2".into()],
                loop_path: true,
                target_speed: 0.6,
            }
        );
    }

    #[test]
    fn build_initial_state_unknown_name_returns_idle() {
        let config = BehaviourConfig {
            initial_state: "attack".into(),
            state: vec![],
            transition: vec![],
            waypoint_arrival_radius: WAYPOINT_ARRIVAL_RADIUS,
            avoidance_buffer: AVOIDANCE_BUFFER,
            avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        };
        assert_eq!(build_initial_state(&config), AiState::Idle);
    }

    #[test]
    fn build_initial_state_pursuing_builds_pursuing_variant() {
        let config = BehaviourConfig {
            initial_state: "chase".into(),
            state: vec![StateConfig {
                name: "chase".into(),
                kind: "pursuing".into(),
                waypoints: vec![],
                loop_path: false,
                target_speed: 0.9,
                maintain_range: 0.0,
                duration_secs: 0.0,
            }],
            transition: vec![],
            ..Default::default()
        };
        let state = build_initial_state(&config);
        assert_eq!(state, AiState::Pursuing { target_speed: 0.9 });
    }

    // ── tick_pursuing ─────────────────────────────────────────────────────

    fn pursuing_controller(target: Option<Uuid>, target_speed: f32) -> AiController {
        let mut c = AiController::new([0.0, 0.0, 0.0], 0.0);
        c.current_state = AiState::Pursuing { target_speed };
        c.current_state_name = "chase".to_string();
        c.blackboard.target = target;
        c
    }

    fn world_with_entity(
        entity_uuid: Uuid,
        entity_pos: [f32; 3],
        entity_faction: Option<Uuid>,
    ) -> WorldView {
        WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![make_world_entity(entity_uuid, entity_pos, entity_faction)],
            ..Default::default()
        }
    }

    #[test]
    fn pursuing_no_ops_when_target_is_none() {
        let ctrl = pursuing_controller(None, 0.8);
        let world = WorldView::default();
        let output = do_tick(&ctrl, &world);
        assert!(
            output.inputs.is_empty(),
            "pursuing with no target must emit no inputs"
        );
    }

    #[test]
    fn pursuing_no_ops_when_target_not_in_world() {
        let ctrl = pursuing_controller(Some(Uuid::new_v4()), 0.8);
        let world = WorldView::default(); // no entities
        let output = do_tick(&ctrl, &world);
        assert!(
            output.inputs.is_empty(),
            "pursuing must emit nothing when target not in world"
        );
    }

    #[test]
    fn pursuing_emits_helm_with_target_speed_when_target_set() {
        let target_id = Uuid::new_v4();
        let ctrl = pursuing_controller(Some(target_id), 0.75);
        let world = world_with_entity(target_id, [100.0, 0.0, 0.0], None);
        let output = do_tick(&ctrl, &world);
        match output.inputs.first() {
            Some(AiInput::Helm { thrust, .. }) => {
                assert!(
                    (*thrust - 0.75).abs() < 1e-5,
                    "thrust must equal target_speed, got {thrust}"
                );
            }
            _ => panic!("expected Helm input, got {:?}", output.inputs),
        }
    }

    #[test]
    fn pursuing_steers_toward_target() {
        // Entity at origin, facing forward (yaw=0, forward=[0,-1]).
        // Target is to the right at [100,0,0] → positive steering.
        let target_id = Uuid::new_v4();
        let ctrl = pursuing_controller(Some(target_id), 0.5);
        let world = world_with_entity(target_id, [100.0, 0.0, 0.0], None);
        let output = do_tick(&ctrl, &world);
        match output.inputs.first() {
            Some(AiInput::Helm { steering, .. }) => {
                assert!(
                    *steering > 0.0,
                    "must steer right toward target, got {steering}"
                );
            }
            _ => panic!("expected Helm input"),
        }
    }

    // ── StringOrVec ───────────────────────────────────────────────────────

    #[test]
    fn string_or_vec_single_contains_match() {
        let sov = StringOrVec::Single("patrol".into());
        assert!(sov.contains("patrol"));
        assert!(!sov.contains("chase"));
    }

    #[test]
    fn string_or_vec_multi_contains_any_match() {
        let sov = StringOrVec::Multi(vec!["patrol".into(), "idle".into()]);
        assert!(sov.contains("patrol"));
        assert!(sov.contains("idle"));
        assert!(!sov.contains("chase"));
    }

    // ── Transition: on_attacked ────────────────────────────────────────────

    fn patrol_to_chase_on_attacked() -> BehaviourConfig {
        BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("patrol".into()),
                to: "chase".into(),
                condition: "on_attacked".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        }
    }

    fn patrolling_ctrl_named(name: &str) -> AiController {
        let mut c = AiController::new([0.0, 0.0, 0.0], 0.0);
        c.current_state = AiState::Patrolling {
            waypoints: vec![],
            loop_path: false,
            target_speed: 0.5,
        };
        c.current_state_name = name.to_string();
        c
    }

    #[test]
    fn on_attacked_fires_when_attacker_present() {
        let attacker_id = Uuid::new_v4();
        let ctrl = patrolling_ctrl_named("patrol");
        let world = WorldView {
            attacker_this_tick: Some(attacker_id),
            ..Default::default()
        };
        let behaviour = patrol_to_chase_on_attacked();
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_eq!(
            output.new_state,
            AiState::Pursuing { target_speed: 0.8 },
            "on_attacked should trigger transition to pursuing"
        );
        let new_bb = output.new_blackboard.expect("new_blackboard must be set");
        assert_eq!(
            new_bb.target,
            Some(attacker_id),
            "target must be set to attacker"
        );
        assert_eq!(
            new_bb.last_attacker,
            Some(attacker_id),
            "last_attacker must be set"
        );
    }

    #[test]
    fn on_attacked_does_not_fire_without_attacker() {
        let ctrl = patrolling_ctrl_named("patrol");
        let world = WorldView::default(); // no attacker
        let behaviour = patrol_to_chase_on_attacked();
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        // No transition: should remain in patrolling (empty waypoints → idle-like)
        assert_ne!(output.new_state, AiState::Pursuing { target_speed: 0.8 });
    }

    #[test]
    fn on_attacked_fires_once_then_suppressed_by_armed_flag() {
        let attacker_id = Uuid::new_v4();
        let mut ctrl = patrolling_ctrl_named("patrol");
        ctrl.blackboard.on_attacked_armed = false; // already fired once
        let world = WorldView {
            attacker_this_tick: Some(attacker_id),
            ..Default::default()
        };
        let behaviour = patrol_to_chase_on_attacked();
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        // Transition must NOT fire because armed = false
        assert_ne!(
            output.new_state,
            AiState::Pursuing { target_speed: 0.8 },
            "on_attacked must be suppressed when on_attacked_armed = false"
        );
    }

    #[test]
    fn on_attacked_new_blackboard_has_armed_false() {
        // When on_attacked fires, new state's on_attacked_armed must be false
        // (prevents immediate re-fire in same state entry).
        let attacker_id = Uuid::new_v4();
        let ctrl = patrolling_ctrl_named("patrol");
        let world = WorldView {
            attacker_this_tick: Some(attacker_id),
            ..Default::default()
        };
        let behaviour = patrol_to_chase_on_attacked();
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        let new_bb = output.new_blackboard.expect("new_blackboard set");
        assert!(
            !new_bb.on_attacked_armed,
            "on_attacked_armed must be false after firing"
        );
    }

    #[test]
    fn on_attacked_rearms_via_non_on_attacked_transition() {
        // Simulate: patrol → (enemy_in_range) → chase
        // The resulting blackboard should have on_attacked_armed = true.
        let enemy_id = Uuid::new_v4();
        let mut ctrl = patrolling_ctrl_named("patrol");
        ctrl.blackboard.on_attacked_armed = false; // was fired in patrol

        let behaviour = BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("patrol".into()),
                to: "chase".into(),
                condition: "enemy_in_range".into(),
                radius: Some(500.0),
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        };

        let registry = mutual_hostile_registry();
        // Add a hostile entity within range
        let world = WorldView {
            self_faction: Some(fed_uuid()),
            entities: vec![AiWorldEntity {
                uuid: enemy_id,
                position: [10.0, 0.0, 0.0],
                faction: Some(pirate_uuid()),
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 0.0,
                forward_speed: 0.0,
            }],
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &registry);
        let new_bb = output.new_blackboard.expect("new_blackboard set");
        assert!(
            new_bb.on_attacked_armed,
            "enemy_in_range transition must re-arm on_attacked"
        );
    }

    // ── Transition: enemy_in_range ────────────────────────────────────────

    #[test]
    fn enemy_in_range_picks_first_hostile_and_sets_target() {
        let pirate_id = Uuid::new_v4();
        let ctrl = patrolling_ctrl_named("patrol");
        let behaviour = BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("patrol".into()),
                to: "chase".into(),
                condition: "enemy_in_range".into(),
                radius: Some(200.0),
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        };
        let registry = mutual_hostile_registry();
        let world = WorldView {
            self_faction: Some(fed_uuid()),
            entities: vec![AiWorldEntity {
                uuid: pirate_id,
                position: [50.0, 0.0, 0.0],
                faction: Some(pirate_uuid()),
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 0.0,
                forward_speed: 0.0,
            }],
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &registry);
        assert_eq!(
            output.new_state,
            AiState::Pursuing { target_speed: 0.8 },
            "enemy_in_range must trigger transition to pursuing"
        );
        let new_bb = output.new_blackboard.expect("new_blackboard set");
        assert_eq!(
            new_bb.target,
            Some(pirate_id),
            "target must be set to the hostile entity"
        );
    }

    #[test]
    fn enemy_in_range_ignores_friendly_entities() {
        let friendly_id = Uuid::new_v4();
        let ctrl = patrolling_ctrl_named("patrol");
        let behaviour = BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("patrol".into()),
                to: "chase".into(),
                condition: "enemy_in_range".into(),
                radius: Some(200.0),
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        };
        let registry = mutual_hostile_registry();
        // Friendly = same faction as self
        let world = WorldView {
            self_faction: Some(fed_uuid()),
            entities: vec![AiWorldEntity {
                uuid: friendly_id,
                position: [50.0, 0.0, 0.0],
                faction: Some(fed_uuid()),
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 0.0,
                forward_speed: 0.0,
            }],
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &registry);
        assert_ne!(
            output.new_state,
            AiState::Pursuing { target_speed: 0.8 },
            "enemy_in_range must not fire for friendly entities"
        );
    }

    #[test]
    fn enemy_in_range_ignores_entities_outside_radius() {
        let pirate_id = Uuid::new_v4();
        let ctrl = patrolling_ctrl_named("patrol");
        let behaviour = BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("patrol".into()),
                to: "chase".into(),
                condition: "enemy_in_range".into(),
                radius: Some(50.0), // small radius
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        };
        let registry = mutual_hostile_registry();
        let world = WorldView {
            self_faction: Some(fed_uuid()),
            entities: vec![AiWorldEntity {
                uuid: pirate_id,
                position: [200.0, 0.0, 0.0],
                faction: Some(pirate_uuid()),
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 0.0,
                forward_speed: 0.0,
            }],
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &registry);
        assert_ne!(
            output.new_state,
            AiState::Pursuing { target_speed: 0.8 },
            "enemy_in_range must not fire for entities outside radius"
        );
    }

    // ── Transition: target_destroyed ──────────────────────────────────────

    fn chase_behaviour_with_target_destroyed_to_patrol() -> BehaviourConfig {
        BehaviourConfig {
            initial_state: "chase".into(),
            state: vec![
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("chase".into()),
                to: "patrol".into(),
                condition: "target_destroyed".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn target_destroyed_fires_when_target_not_in_world() {
        let target_id = Uuid::new_v4();
        let mut ctrl = pursuing_controller(Some(target_id), 0.8);
        ctrl.current_state_name = "chase".to_string();
        let world = WorldView::default(); // target not present
        let behaviour = chase_behaviour_with_target_destroyed_to_patrol();
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_eq!(
            output.new_state,
            AiState::Patrolling {
                waypoints: vec![],
                loop_path: false,
                target_speed: 0.5
            },
            "target_destroyed must transition to patrol"
        );
        let new_bb = output.new_blackboard.expect("new_blackboard set");
        assert!(
            new_bb.target.is_none(),
            "target must be cleared after target_destroyed"
        );
    }

    #[test]
    fn target_destroyed_does_not_fire_when_target_present() {
        let target_id = Uuid::new_v4();
        let mut ctrl = pursuing_controller(Some(target_id), 0.8);
        ctrl.current_state_name = "chase".to_string();
        let world = WorldView {
            entities: vec![AiWorldEntity {
                uuid: target_id,
                position: [100.0, 0.0, 0.0],
                faction: None,
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 0.0,
                forward_speed: 0.0,
            }],
            ..Default::default()
        };
        let behaviour = chase_behaviour_with_target_destroyed_to_patrol();
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_ne!(
            output.new_state,
            AiState::Patrolling {
                waypoints: vec![],
                loop_path: false,
                target_speed: 0.5
            },
            "target_destroyed must not fire when target is still alive"
        );
    }

    #[test]
    fn target_destroyed_fires_when_target_is_none() {
        // No target set → condition fires (target is effectively destroyed/missing)
        let mut ctrl = pursuing_controller(None, 0.8);
        ctrl.current_state_name = "chase".to_string();
        let world = WorldView::default();
        let behaviour = chase_behaviour_with_target_destroyed_to_patrol();
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_eq!(
            output.new_state,
            AiState::Patrolling {
                waypoints: vec![],
                loop_path: false,
                target_speed: 0.5
            },
            "target_destroyed must fire when target is None"
        );
    }

    // ── Transition ordering ────────────────────────────────────────────────

    #[test]
    fn transition_ordering_first_match_fires() {
        // Two transitions from "patrol": first to "stateA", second to "stateB".
        // enemy_in_range fires; on_attacked would also fire but enemy_in_range is first.
        let attacker_id = Uuid::new_v4();
        let ctrl = patrolling_ctrl_named("patrol");
        let registry = mutual_hostile_registry();
        let behaviour = BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "stateA".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.6,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "stateB".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.9,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![
                TransitionConfig {
                    from: StringOrVec::Single("patrol".into()),
                    to: "stateA".into(),
                    condition: "enemy_in_range".into(),
                    radius: Some(500.0),
                    threshold: None,
                    seconds: None,
                },
                TransitionConfig {
                    from: StringOrVec::Single("patrol".into()),
                    to: "stateB".into(),
                    condition: "on_attacked".into(),
                    radius: None,
                    threshold: None,
                    seconds: None,
                },
            ],
            ..Default::default()
        };
        // Both conditions are true: enemy in range AND attacker present
        let world = WorldView {
            self_faction: Some(fed_uuid()),
            attacker_this_tick: Some(attacker_id),
            entities: vec![AiWorldEntity {
                uuid: attacker_id,
                position: [10.0, 0.0, 0.0],
                faction: Some(pirate_uuid()),
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 0.0,
                forward_speed: 0.0,
            }],
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &registry);
        // First transition (enemy_in_range → stateA) must fire
        assert_eq!(
            output.new_state,
            AiState::Pursuing { target_speed: 0.6 },
            "first matching transition (enemy_in_range → stateA) must fire"
        );
    }

    // ── Transition: from as string or list ────────────────────────────────

    #[test]
    fn transition_from_string_fires_when_state_matches() {
        let attacker_id = Uuid::new_v4();
        let ctrl = patrolling_ctrl_named("patrol");
        let behaviour = BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("patrol".into()), // single string
                to: "chase".into(),
                condition: "on_attacked".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        };
        let world = WorldView {
            attacker_this_tick: Some(attacker_id),
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_eq!(
            output.new_state,
            AiState::Pursuing { target_speed: 0.8 },
            "from as single string must match current state name"
        );
    }

    #[test]
    fn transition_from_list_fires_when_state_matches_any() {
        let attacker_id = Uuid::new_v4();
        let ctrl = patrolling_ctrl_named("patrol");
        let behaviour = BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "idle_wait".into(),
                    kind: "idle".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.0,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Multi(vec!["patrol".into(), "idle_wait".into()]), // list
                to: "chase".into(),
                condition: "on_attacked".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        };
        let world = WorldView {
            attacker_this_tick: Some(attacker_id),
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_eq!(
            output.new_state,
            AiState::Pursuing { target_speed: 0.8 },
            "from as list must fire when current state is any listed name"
        );
    }

    #[test]
    fn transition_from_list_does_not_fire_when_state_not_in_list() {
        let attacker_id = Uuid::new_v4();
        // Controller in "chase" which is NOT in the from list
        let mut ctrl = pursuing_controller(None, 0.8);
        ctrl.current_state_name = "chase".to_string();
        let behaviour = BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![
                StateConfig {
                    name: "patrol".into(),
                    kind: "patrolling".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "idle_wait".into(),
                    kind: "idle".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.0,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Multi(vec!["patrol".into(), "idle_wait".into()]),
                to: "chase".into(),
                condition: "on_attacked".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        };
        let world = WorldView {
            attacker_this_tick: Some(attacker_id),
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        // "chase" is not in the from list → no transition should have fired
        assert!(
            output.new_state_name.is_none(),
            "transition must not fire when current state is not in from list"
        );
    }

    // ── Attacking state ────────────────────────────────────────────────────

    fn make_attacking_controller(pos: [f32; 3], _yaw: f32, target: Uuid) -> AiController {
        let mut ctrl = AiController::new(pos, 0.0);
        ctrl.current_state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        ctrl.current_state_name = "attack".into();
        ctrl.blackboard.target = Some(target);
        ctrl
    }

    #[test]
    fn attacking_kind_name_is_attacking() {
        let state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        assert_eq!(state.kind_name(), "attacking");
    }

    #[test]
    fn build_state_by_name_attacking_builds_attacking_variant() {
        let config = BehaviourConfig {
            initial_state: "attack".into(),
            state: vec![StateConfig {
                name: "attack".into(),
                kind: "attacking".into(),
                waypoints: vec![],
                loop_path: false,
                target_speed: 0.8,
                maintain_range: 30.0,
                duration_secs: 0.0,
            }],
            transition: vec![],
            ..Default::default()
        };
        let state = build_initial_state(&config);
        assert_eq!(
            state,
            AiState::Attacking {
                maintain_range: 30.0,
                target_speed: 0.8
            }
        );
    }

    #[test]
    fn tick_attacking_no_target_emits_no_inputs() {
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.current_state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        // No target in blackboard
        let world = WorldView::default();
        let output = do_tick(&ctrl, &world);
        assert!(output.inputs.is_empty(), "no target → no inputs");
    }

    #[test]
    fn tick_attacking_target_not_in_world_emits_no_inputs() {
        let target_id = Uuid::new_v4();
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.current_state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        ctrl.blackboard.target = Some(target_id);
        // Target not in world entities
        let world = WorldView::default();
        let output = do_tick(&ctrl, &world);
        assert!(output.inputs.is_empty(), "absent target → no inputs");
    }

    #[test]
    fn tick_attacking_outside_maintain_range_thrusts() {
        let target_id = Uuid::new_v4();
        // AI at origin, target 50 units away — outside maintain_range=30
        let ctrl = make_attacking_controller([0.0, 0.0, 0.0], 0.0, target_id);
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![make_world_entity(target_id, [0.0, 0.0, -50.0], None)],
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        let helm = output
            .inputs
            .iter()
            .find(|i| matches!(i, AiInput::Helm { .. }));
        if let Some(AiInput::Helm { thrust, .. }) = helm {
            assert!(
                *thrust > 0.0,
                "should thrust when outside maintain_range, got {thrust}"
            );
        } else {
            panic!("no Helm input emitted");
        }
    }

    #[test]
    fn tick_attacking_inside_maintain_range_zero_thrust() {
        let target_id = Uuid::new_v4();
        // AI at origin, target 10 units away — inside maintain_range=30
        let ctrl = make_attacking_controller([0.0, 0.0, 0.0], 0.0, target_id);
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![make_world_entity(target_id, [0.0, 0.0, -10.0], None)],
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        let helm = output
            .inputs
            .iter()
            .find(|i| matches!(i, AiInput::Helm { .. }));
        if let Some(AiInput::Helm { thrust, .. }) = helm {
            assert_eq!(*thrust, 0.0, "should not thrust when inside maintain_range");
        } else {
            panic!("no Helm input emitted");
        }
    }

    #[test]
    fn tick_attacking_steers_toward_target() {
        let target_id = Uuid::new_v4();
        // AI facing +Z (yaw = π), target is to the right (+X). Expect positive steering.
        let ctrl = make_attacking_controller([0.0, 0.0, 0.0], std::f32::consts::PI, target_id);
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: std::f32::consts::PI,
            entities: vec![make_world_entity(target_id, [50.0, 0.0, 0.0], None)],
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        let helm = output
            .inputs
            .iter()
            .find(|i| matches!(i, AiInput::Helm { .. }));
        if let Some(AiInput::Helm { steering, .. }) = helm {
            assert!(*steering != 0.0, "should steer toward off-axis target");
        } else {
            panic!("no Helm input emitted");
        }
    }

    #[test]
    fn tick_attacking_fires_phaser_when_in_range_and_ready() {
        let target_id = Uuid::new_v4();
        let mut ctrl = make_attacking_controller([0.0, 0.0, 0.0], 0.0, target_id);
        ctrl.current_state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        // Target within weapons range
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![make_world_entity(target_id, [0.0, 0.0, -20.0], None)],
            entity_weapons_range: Some(40.0),
            entity_phaser_ready: true,
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        let fires_phaser = output
            .inputs
            .iter()
            .any(|i| matches!(i, AiInput::FirePhaser));
        assert!(fires_phaser, "should fire phaser when in range and ready");
    }

    #[test]
    fn tick_attacking_does_not_fire_phaser_when_not_ready() {
        let target_id = Uuid::new_v4();
        let ctrl = make_attacking_controller([0.0, 0.0, 0.0], 0.0, target_id);
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![make_world_entity(target_id, [0.0, 0.0, -20.0], None)],
            entity_weapons_range: Some(40.0),
            entity_phaser_ready: false,
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        let fires_phaser = output
            .inputs
            .iter()
            .any(|i| matches!(i, AiInput::FirePhaser));
        assert!(!fires_phaser, "should not fire phaser when not ready");
    }

    #[test]
    fn tick_attacking_does_not_fire_phaser_when_out_of_range() {
        let target_id = Uuid::new_v4();
        let ctrl = make_attacking_controller([0.0, 0.0, 0.0], 0.0, target_id);
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![make_world_entity(target_id, [0.0, 0.0, -60.0], None)],
            entity_weapons_range: Some(40.0),
            entity_phaser_ready: true,
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        let fires_phaser = output
            .inputs
            .iter()
            .any(|i| matches!(i, AiInput::FirePhaser));
        assert!(!fires_phaser, "should not fire phaser when out of range");
    }

    #[test]
    fn tick_attacking_fires_torpedo_when_facing_shield_depleted_and_tube_ready() {
        let target_id = Uuid::new_v4();
        let ctrl = make_attacking_controller([0.0, 0.0, 0.0], 0.0, target_id);
        // AI at origin. Target at [0,0,-20] with yaw=PI (facing +Z).
        // AI is at +Z relative to target → AI is in the "Fore" quadrant → "Fore" shield depleted.
        let target = AiWorldEntity {
            uuid: target_id,
            position: [0.0, 0.0, -20.0],
            faction: None,
            yaw: Some(std::f32::consts::PI),
            shields: Some(vec![
                ShieldFacingStatus {
                    label: "Fore".into(),
                    hp: 0,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
                ShieldFacingStatus {
                    label: "Aft".into(),
                    hp: 100,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
                ShieldFacingStatus {
                    label: "Port".into(),
                    hp: 100,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
                ShieldFacingStatus {
                    label: "Starboard".into(),
                    hp: 100,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
            ]),
            hull_fraction: Some(1.0),
            ..Default::default()
        };
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![target],
            torpedo_tube_ready: Some("tube_1".into()),
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        let fires_torp = output
            .inputs
            .iter()
            .any(|i| matches!(i, AiInput::FireTorpedo { .. }));
        assert!(
            fires_torp,
            "should fire torpedo when facing shield is depleted and tube ready"
        );
    }

    #[test]
    fn tick_attacking_does_not_fire_torpedo_when_facing_shield_intact() {
        let target_id = Uuid::new_v4();
        let ctrl = make_attacking_controller([0.0, 0.0, 0.0], 0.0, target_id);
        let target = AiWorldEntity {
            uuid: target_id,
            position: [0.0, 0.0, -20.0],
            faction: None,
            yaw: Some(0.0),
            shields: Some(vec![
                ShieldFacingStatus {
                    label: "Fore".into(),
                    hp: 100,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
                ShieldFacingStatus {
                    label: "Aft".into(),
                    hp: 100,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
                ShieldFacingStatus {
                    label: "Port".into(),
                    hp: 100,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
                ShieldFacingStatus {
                    label: "Starboard".into(),
                    hp: 100,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
            ]),
            hull_fraction: Some(1.0),
            ..Default::default()
        };
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![target],
            torpedo_tube_ready: Some("tube_1".into()),
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        let fires_torp = output
            .inputs
            .iter()
            .any(|i| matches!(i, AiInput::FireTorpedo { .. }));
        assert!(
            !fires_torp,
            "should not fire torpedo when facing shield is intact"
        );
    }

    #[test]
    fn tick_attacking_does_not_fire_torpedo_when_no_tube_ready() {
        let target_id = Uuid::new_v4();
        let ctrl = make_attacking_controller([0.0, 0.0, 0.0], 0.0, target_id);
        let target = AiWorldEntity {
            uuid: target_id,
            position: [0.0, 0.0, -20.0],
            faction: None,
            yaw: Some(0.0),
            shields: Some(vec![ShieldFacingStatus {
                label: "Fore".into(),
                hp: 0,
                max_hp: 100,
                online: true,
                offline_remaining: 0.0,
                is_focused: false,
            }]),
            hull_fraction: Some(1.0),
            ..Default::default()
        };
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![target],
            torpedo_tube_ready: None,
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        let fires_torp = output
            .inputs
            .iter()
            .any(|i| matches!(i, AiInput::FireTorpedo { .. }));
        assert!(!fires_torp, "should not fire torpedo when no tube is ready");
    }

    #[test]
    fn in_weapons_range_transition_fires_when_target_in_range() {
        let target_id = Uuid::new_v4();
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.current_state = AiState::Pursuing { target_speed: 0.8 };
        ctrl.current_state_name = "chase".into();
        ctrl.blackboard.target = Some(target_id);

        let behaviour = BehaviourConfig {
            initial_state: "chase".into(),
            state: vec![
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "attack".into(),
                    kind: "attacking".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 30.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("chase".into()),
                to: "attack".into(),
                condition: "in_weapons_range".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        };
        // entity_weapons_range=40, target at 20 — should be in range
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_weapons_range: Some(40.0),
            entities: vec![make_world_entity(target_id, [0.0, 0.0, -20.0], None)],
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_eq!(
            output.new_state,
            AiState::Attacking {
                maintain_range: 30.0,
                target_speed: 0.8
            },
            "in_weapons_range should transition to attacking"
        );
    }

    #[test]
    fn in_weapons_range_transition_does_not_fire_when_target_out_of_range() {
        let target_id = Uuid::new_v4();
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.current_state = AiState::Pursuing { target_speed: 0.8 };
        ctrl.current_state_name = "chase".into();
        ctrl.blackboard.target = Some(target_id);

        let behaviour = BehaviourConfig {
            initial_state: "chase".into(),
            state: vec![
                StateConfig {
                    name: "chase".into(),
                    kind: "pursuing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "attack".into(),
                    kind: "attacking".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 30.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("chase".into()),
                to: "attack".into(),
                condition: "in_weapons_range".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        };
        // entity_weapons_range=40, target at 100 — out of range
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_weapons_range: Some(40.0),
            entities: vec![make_world_entity(target_id, [0.0, 0.0, -100.0], None)],
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert!(
            output.new_state_name.is_none(),
            "in_weapons_range must not fire when target is out of range"
        );
    }

    // ── Fleeing state ─────────────────────────────────────────────────────

    fn fleeing_controller(last_attacker: Option<Uuid>, target_speed: f32) -> AiController {
        let mut c = AiController::new([0.0, 0.0, 0.0], 0.0);
        c.current_state = AiState::Fleeing { target_speed };
        c.current_state_name = "fleeing".to_string();
        c.blackboard.last_attacker = last_attacker;
        c
    }

    #[test]
    fn fleeing_steers_away_from_last_attacker() {
        // Entity at origin facing forward (yaw=0, forward=[0,-1]).
        // Attacker is behind at [0,0,10] (+Z direction).
        // Away direction is [0,0,-1] (−Z), which is straight ahead → steering ≈ 0.
        // Let's put attacker to the left at [-50,0,0]. Away is [1,0,0] (right) → positive steering.
        let attacker_id = Uuid::new_v4();
        let ctrl = fleeing_controller(Some(attacker_id), 0.6);
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            entities: vec![make_world_entity(attacker_id, [-50.0, 0.0, 0.0], None)],
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        match output.inputs.first() {
            Some(AiInput::Helm { steering, thrust }) => {
                assert!(
                    *steering > 0.0,
                    "fleeing with attacker to left must steer right, got {steering}"
                );
                assert!(
                    (*thrust - 0.6).abs() < 1e-5,
                    "thrust must equal target_speed, got {thrust}"
                );
            }
            _ => panic!("expected Helm input, got {:?}", output.inputs),
        }
    }

    #[test]
    fn fleeing_holds_heading_when_no_last_attacker() {
        let ctrl = fleeing_controller(None, 0.7);
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.5,
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        match output.inputs.first() {
            Some(AiInput::Helm { steering, thrust }) => {
                assert_eq!(
                    *steering, 0.0,
                    "fleeing with no attacker must not steer, got {steering}"
                );
                assert!(
                    (*thrust - 0.7).abs() < 1e-5,
                    "thrust must equal target_speed, got {thrust}"
                );
            }
            _ => panic!("expected Helm input, got {:?}", output.inputs),
        }
    }

    #[test]
    fn fleeing_holds_heading_when_attacker_not_in_world() {
        let attacker_id = Uuid::new_v4();
        let ctrl = fleeing_controller(Some(attacker_id), 0.5);
        let world = WorldView::default(); // attacker not present
        let output = do_tick(&ctrl, &world);
        match output.inputs.first() {
            Some(AiInput::Helm { steering, .. }) => {
                assert_eq!(
                    *steering, 0.0,
                    "fleeing with absent attacker must hold heading, got {steering}"
                );
            }
            _ => panic!("expected Helm input, got {:?}", output.inputs),
        }
    }

    // ── WarpingOut state ──────────────────────────────────────────────────

    fn warping_out_controller(state_entered_at: f64, duration_secs: f32) -> AiController {
        let mut c = AiController::new([0.0, 0.0, 0.0], state_entered_at);
        c.current_state = AiState::WarpingOut {
            duration_secs,
            target_speed: 0.8,
        };
        c.current_state_name = "warping_out".to_string();
        c.blackboard.state_entered_at = state_entered_at;
        c
    }

    #[test]
    fn warping_out_holds_heading_zero_steering() {
        let ctrl = warping_out_controller(0.0, 5.0);
        let world = WorldView {
            sim_time: 1.0,
            entity_yaw: 1.2,
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        match output.inputs.first() {
            Some(AiInput::Helm { steering, thrust }) => {
                assert_eq!(*steering, 0.0, "warping_out must not steer");
                assert!(
                    (*thrust - 0.8).abs() < 1e-5,
                    "thrust must equal target_speed"
                );
            }
            _ => panic!("expected Helm input"),
        }
    }

    #[test]
    fn warping_out_does_not_despawn_before_duration() {
        let ctrl = warping_out_controller(0.0, 5.0);
        let world = WorldView {
            sim_time: 3.0,
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        assert!(!output.despawn, "must not despawn before duration expires");
    }

    #[test]
    fn warping_out_despawns_when_duration_elapsed() {
        let ctrl = warping_out_controller(0.0, 5.0);
        let world = WorldView {
            sim_time: 5.0,
            ..Default::default()
        };
        let output = do_tick(&ctrl, &world);
        assert!(output.despawn, "must despawn when duration has elapsed");
    }

    #[test]
    fn warping_out_despawns_when_duration_exceeded() {
        let ctrl = warping_out_controller(10.0, 3.0);
        let world = WorldView {
            sim_time: 14.0,
            ..Default::default()
        }; // 4s > 3s
        let output = do_tick(&ctrl, &world);
        assert!(output.despawn, "must despawn when duration is exceeded");
    }

    // ── Condition: hull_below ─────────────────────────────────────────────

    fn make_hull_below_behaviour(threshold: f32) -> BehaviourConfig {
        BehaviourConfig {
            initial_state: "attack".into(),
            state: vec![
                StateConfig {
                    name: "attack".into(),
                    kind: "attacking".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 30.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "flee".into(),
                    kind: "fleeing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.9,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("attack".into()),
                to: "flee".into(),
                condition: "hull_below".into(),
                radius: None,
                threshold: Some(threshold),
                seconds: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn hull_below_fires_when_hull_fraction_below_threshold() {
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.current_state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        ctrl.current_state_name = "attack".into();
        let behaviour = make_hull_below_behaviour(0.3);
        let world = WorldView {
            self_hull_fraction: Some(0.25),
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_eq!(
            output.new_state,
            AiState::Fleeing { target_speed: 0.9 },
            "hull_below must fire when hull < threshold"
        );
    }

    #[test]
    fn hull_below_does_not_fire_when_hull_at_threshold() {
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.current_state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        ctrl.current_state_name = "attack".into();
        let behaviour = make_hull_below_behaviour(0.3);
        let world = WorldView {
            self_hull_fraction: Some(0.3),
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_ne!(
            output.new_state,
            AiState::Fleeing { target_speed: 0.9 },
            "hull_below must not fire when hull == threshold"
        );
    }

    #[test]
    fn hull_below_does_not_fire_when_hull_above_threshold() {
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.current_state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        ctrl.current_state_name = "attack".into();
        let behaviour = make_hull_below_behaviour(0.3);
        let world = WorldView {
            self_hull_fraction: Some(0.8),
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_ne!(
            output.new_state,
            AiState::Fleeing { target_speed: 0.9 },
            "hull_below must not fire when hull > threshold"
        );
    }

    // ── Condition: on_timer ───────────────────────────────────────────────

    fn make_on_timer_behaviour(seconds: f32) -> BehaviourConfig {
        BehaviourConfig {
            initial_state: "flee".into(),
            state: vec![
                StateConfig {
                    name: "flee".into(),
                    kind: "fleeing".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 0.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "warp".into(),
                    kind: "warping_out".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 4.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("flee".into()),
                to: "warp".into(),
                condition: "on_timer".into(),
                radius: None,
                threshold: None,
                seconds: Some(seconds),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn on_timer_fires_after_elapsed_seconds() {
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 10.0);
        ctrl.current_state = AiState::Fleeing { target_speed: 0.8 };
        ctrl.current_state_name = "flee".into();
        ctrl.blackboard.state_entered_at = 10.0;
        let behaviour = make_on_timer_behaviour(5.0);
        let world = WorldView {
            sim_time: 15.5,
            ..Default::default()
        }; // 5.5s elapsed > 5.0s
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_eq!(
            output.new_state,
            AiState::WarpingOut {
                duration_secs: 4.0,
                target_speed: 0.5
            },
            "on_timer must fire after elapsed >= seconds"
        );
    }

    #[test]
    fn on_timer_does_not_fire_before_elapsed_seconds() {
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 10.0);
        ctrl.current_state = AiState::Fleeing { target_speed: 0.8 };
        ctrl.current_state_name = "flee".into();
        ctrl.blackboard.state_entered_at = 10.0;
        let behaviour = make_on_timer_behaviour(5.0);
        let world = WorldView {
            sim_time: 14.0,
            ..Default::default()
        }; // 4.0s < 5.0s
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_ne!(
            output.new_state,
            AiState::WarpingOut {
                duration_secs: 4.0,
                target_speed: 0.5
            },
            "on_timer must not fire before elapsed < seconds"
        );
    }

    // ── Condition: on_scenario_unloaded ───────────────────────────────────

    fn make_on_scenario_unloaded_behaviour() -> BehaviourConfig {
        BehaviourConfig {
            initial_state: "attack".into(),
            state: vec![
                StateConfig {
                    name: "attack".into(),
                    kind: "attacking".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.8,
                    maintain_range: 30.0,
                    duration_secs: 0.0,
                },
                StateConfig {
                    name: "warp".into(),
                    kind: "warping_out".into(),
                    waypoints: vec![],
                    loop_path: false,
                    target_speed: 0.5,
                    maintain_range: 0.0,
                    duration_secs: 3.0,
                },
            ],
            transition: vec![TransitionConfig {
                from: StringOrVec::Single("attack".into()),
                to: "warp".into(),
                condition: "on_scenario_unloaded".into(),
                radius: None,
                threshold: None,
                seconds: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn on_scenario_unloaded_fires_when_signal_set() {
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.current_state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        ctrl.current_state_name = "attack".into();
        let behaviour = make_on_scenario_unloaded_behaviour();
        let world = WorldView {
            scenario_unloaded: true,
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_eq!(
            output.new_state,
            AiState::WarpingOut {
                duration_secs: 3.0,
                target_speed: 0.5
            },
            "on_scenario_unloaded must fire when scenario_unloaded=true"
        );
    }

    #[test]
    fn on_scenario_unloaded_does_not_fire_normally() {
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.current_state = AiState::Attacking {
            maintain_range: 30.0,
            target_speed: 0.8,
        };
        ctrl.current_state_name = "attack".into();
        let behaviour = make_on_scenario_unloaded_behaviour();
        let world = WorldView {
            scenario_unloaded: false,
            ..Default::default()
        };
        let output = do_tick_with(&ctrl, &world, &behaviour, &empty_registry());
        assert_ne!(
            output.new_state,
            AiState::WarpingOut {
                duration_secs: 3.0,
                target_speed: 0.5
            },
            "on_scenario_unloaded must not fire when scenario_unloaded=false"
        );
    }

    // ── despawn flag ──────────────────────────────────────────────────────

    #[test]
    fn idle_tick_does_not_set_despawn() {
        let ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        let world = WorldView::default();
        let output = do_tick(&ctrl, &world);
        assert!(!output.despawn, "idle tick must not set despawn");
    }
}
