/// Pure AI controller module — no Bevy imports.
///
/// Owns the `AiController` struct, the fixed five-slot `Blackboard`,
/// `AiInput`, `AiTickOutput`, the pure `tick` function, and the
/// `should_emit` edge-emission filter.
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

// ── AiState ───────────────────────────────────────────────────────────────────

/// The set of states an AI controller can be in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AiState {
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
}

impl Default for AiState {
    fn default() -> Self {
        AiState::Idle
    }
}

// ── Constants ─────────────────────────────────────────────────────────────────

/// Arrival radius in world units — closer than this counts as "reached waypoint".
pub const WAYPOINT_ARRIVAL_RADIUS: f32 = 20.0;
/// Angular deadband for `steer_toward`: within this angle, steering = 0.
pub const PATROL_DEADBAND_RAD: f32 = 0.05;
/// Angular error at which steering saturates to ±1.
pub const PATROL_FULL_STEER_RAD: f32 = PI / 4.0;

// ── AiInput ───────────────────────────────────────────────────────────────────

/// The set of actions an AI controller can emit, mirroring the client
/// message vocabulary so the simulation can process them identically.
#[derive(Debug, Clone, PartialEq)]
pub enum AiInput {
    Helm { thrust: f32, steering: f32 },
    SetTarget { uuid: Uuid },
    FirePhaser,
    FireTorpedo { tube: String },
    Hail { target_uuid: Uuid },
    RespondToMessage { message_id: String, response_index: usize },
}

// ── Blackboard ────────────────────────────────────────────────────────────────

/// Fixed five-slot blackboard per AI controller.
#[derive(Debug, Clone, Default, PartialEq)]
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
}

// ── AiController ─────────────────────────────────────────────────────────────

/// Per-entity AI controller state.
#[derive(Debug, Clone, PartialEq)]
pub struct AiController {
    pub current_state: AiState,
    pub blackboard: Blackboard,
}

impl AiController {
    /// Create a new controller in `Idle` state with the given spawn position
    /// and the simulation time at construction.
    pub fn new(home_position: [f32; 3], state_entered_at: f64) -> Self {
        Self {
            current_state: AiState::Idle,
            blackboard: Blackboard {
                home_position,
                state_entered_at,
                ..Default::default()
            },
        }
    }
}

// ── WorldView ─────────────────────────────────────────────────────────────────

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
}

// ── AiTickOutput ─────────────────────────────────────────────────────────────

/// The output of a single AI tick.
#[derive(Debug, Clone, PartialEq)]
pub struct AiTickOutput {
    /// The state the controller should transition to (may be the same state).
    pub new_state: AiState,
    /// Input actions to inject into the simulation this tick.
    pub inputs: Vec<AiInput>,
    /// Updated blackboard to write back to the controller. `None` = unchanged.
    pub new_blackboard: Option<Blackboard>,
}

impl AiTickOutput {
    fn idle() -> Self {
        Self { new_state: AiState::Idle, inputs: vec![], new_blackboard: None }
    }
}

// ── steer_toward ─────────────────────────────────────────────────────────────

/// Proportional steering toward a direction, with deadband and saturation.
///
/// - `yaw`: current entity yaw in radians (forward = `-sin(yaw), -cos(yaw)` in XZ).
/// - `target_dir`: 2-element [dx, dz] unit vector pointing at target.
/// - `deadband_rad`: angular error below which steering = 0.
/// - `full_steer_rad`: angular error at which steering saturates to ±1.
///
/// Returns a steering value in [-1, 1].
pub fn steer_toward(yaw: f32, target_dir: [f32; 2], deadband_rad: f32, full_steer_rad: f32) -> f32 {
    // Ship forward in XZ: (-sin(yaw), -cos(yaw))
    let fwd_x = -yaw.sin();
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

// ── tick ──────────────────────────────────────────────────────────────────────

/// Advance an AI controller by one tick.
///
/// Pure function: takes a controller and a world view, returns the new state
/// and any inputs to emit. The caller is responsible for applying `new_state`
/// and `new_blackboard` back to the controller.
pub fn tick(controller: &AiController, world_view: &WorldView) -> AiTickOutput {
    match &controller.current_state {
        AiState::Idle => AiTickOutput::idle(),
        AiState::Patrolling { waypoints, loop_path, target_speed } => {
            tick_patrolling(controller, world_view, waypoints, *loop_path, *target_speed)
        }
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
            inputs: vec![AiInput::Helm { thrust: 0.0, steering: 0.0 }],
            new_blackboard: None,
        };
    }

    let waypoint_name = &waypoints[idx];
    let anchor = match world_view.anchors.get(waypoint_name) {
        Some(a) => *a,
        None => {
            // Unknown anchor — emit nothing, stay put
            return AiTickOutput {
                new_state: controller.current_state.clone(),
                inputs: vec![AiInput::Helm { thrust: 0.0, steering: 0.0 }],
                new_blackboard: None,
            };
        }
    };

    let pos = world_view.entity_pos;
    let dx = anchor[0] - pos[0];
    let dz = anchor[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    // Check arrival
    if dist < WAYPOINT_ARRIVAL_RADIUS {
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
                inputs: vec![AiInput::Helm { thrust: 0.0, steering: 0.0 }],
                new_blackboard: Some(new_bb),
            };
        }

        return AiTickOutput {
            new_state: controller.current_state.clone(),
            inputs: vec![AiInput::Helm { thrust: target_speed, steering: 0.0 }],
            new_blackboard: Some(new_bb),
        };
    }

    // Steer toward waypoint
    let inv_dist = 1.0 / dist;
    let dir = [dx * inv_dist, dz * inv_dist];
    let steering = steer_toward(world_view.entity_yaw, dir, PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);

    AiTickOutput {
        new_state: controller.current_state.clone(),
        inputs: vec![AiInput::Helm { thrust: target_speed, steering }],
        new_blackboard: None,
    }
}

// ── build_initial_state ───────────────────────────────────────────────────────

/// Build the initial `AiState` from a `BehaviourConfig`.
///
/// Looks up the `initial_state` name in `config.state`; if a matching
/// `StateConfig` entry is found, builds the typed state from it.  Falls back
/// to `Idle` when the name is `"idle"` or there is no matching entry.
pub fn build_initial_state(config: &crate::entity_config::BehaviourConfig) -> AiState {
    let name = &config.initial_state;
    if name == "idle" {
        return AiState::Idle;
    }
    if let Some(sc) = config.state.iter().find(|s| &s.name == name) {
        match sc.kind.as_str() {
            "patrolling" => AiState::Patrolling {
                waypoints: sc.waypoints.clone(),
                loop_path: sc.loop_path,
                target_speed: sc.target_speed,
            },
            _ => AiState::Idle,
        }
    } else {
        AiState::Idle
    }
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

    // ── Tracer bullet: idle state emits nothing ────────────────────────────

    #[test]
    fn tick_idle_returns_no_inputs() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        let world = WorldView::default();
        let output = tick(&controller, &world);
        assert!(output.inputs.is_empty(), "idle must emit no inputs");
    }

    #[test]
    fn tick_idle_returns_idle_state() {
        let controller = AiController::new([0.0, 0.0, 0.0], 0.0);
        let world = WorldView::default();
        let output = tick(&controller, &world);
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
        assert!(steering.abs() < 1e-6, "aligned: steering must be 0, got {steering}");
    }

    #[test]
    fn steer_toward_90deg_right_saturates_to_positive_one() {
        // Facing forward (yaw=0, forward=[0,-1]), target to the right [1, 0]
        let steering = steer_toward(0.0, [1.0, 0.0], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
        assert!((steering - 1.0).abs() < 1e-5, "90deg right: steering must be 1.0, got {steering}");
    }

    #[test]
    fn steer_toward_90deg_left_saturates_to_negative_one() {
        // Target to the left [-1, 0]
        let steering = steer_toward(0.0, [-1.0, 0.0], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
        assert!((steering + 1.0).abs() < 1e-5, "90deg left: steering must be -1.0, got {steering}");
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
        assert!((steering - 0.5).abs() < 0.02, "half-angle: steering ~= 0.5, got {steering}");
    }

    #[test]
    fn steer_toward_pi_wraparound_saturates() {
        // Target directly behind: [0, 1] (i.e. +Z), angle = ±π
        // Should saturate to ±1 (sign depends on cross product, but magnitude = 1)
        let steering = steer_toward(0.0, [0.0, 1.0], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
        assert!(steering.abs() > 0.99, "behind: steering magnitude must be ≈1, got {steering}");
    }

    // ── tick_patrolling ───────────────────────────────────────────────────

    fn patrolling_controller(waypoints: Vec<String>, loop_path: bool, target_speed: f32) -> AiController {
        let mut c = AiController::new([0.0, 0.0, 0.0], 0.0);
        c.current_state = AiState::Patrolling { waypoints, loop_path, target_speed };
        c
    }

    fn two_waypoint_view(entity_pos: [f32; 3], entity_yaw: f32) -> WorldView {
        let mut anchors = std::collections::HashMap::new();
        anchors.insert("wp1".to_string(), [100.0, 0.0, 0.0]);
        anchors.insert("wp2".to_string(), [-100.0, 0.0, 0.0]);
        WorldView { entity_pos, entity_yaw, anchors, ..Default::default() }
    }

    #[test]
    fn patrolling_emits_thrust_equal_to_target_speed() {
        let ctrl = patrolling_controller(vec!["wp1".into()], false, 0.7);
        let view = two_waypoint_view([0.0, 0.0, 0.0], 0.0);
        let output = tick(&ctrl, &view);
        match output.inputs.first() {
            Some(AiInput::Helm { thrust, .. }) => {
                assert!((*thrust - 0.7).abs() < 1e-5, "thrust must equal target_speed, got {thrust}");
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
        let output = tick(&ctrl, &view);
        match output.inputs.first() {
            Some(AiInput::Helm { steering, .. }) => {
                assert!(*steering > 0.0, "should steer right toward wp1, got {steering}");
            }
            _ => panic!("expected Helm input"),
        }
    }

    #[test]
    fn patrolling_advances_waypoint_index_on_arrival() {
        // Entity at wp1 position (within arrival radius)
        let ctrl = patrolling_controller(vec!["wp1".into(), "wp2".into()], false, 0.5);
        let view = two_waypoint_view([100.0, 0.0, 0.0], 0.0); // at wp1
        let output = tick(&ctrl, &view);
        let new_bb = output.new_blackboard.expect("new_blackboard must be set on arrival");
        assert_eq!(new_bb.waypoint_index, 1, "must advance to next waypoint");
    }

    #[test]
    fn patrolling_loops_when_last_waypoint_reached_and_loop_path_true() {
        let mut ctrl = patrolling_controller(vec!["wp1".into(), "wp2".into()], true, 0.5);
        ctrl.blackboard.waypoint_index = 1; // already at wp2 index
        let view = two_waypoint_view([-100.0, 0.0, 0.0], 0.0); // at wp2
        let output = tick(&ctrl, &view);
        let new_bb = output.new_blackboard.expect("new_blackboard must be set");
        assert_eq!(new_bb.waypoint_index, 0, "must loop back to waypoint 0");
    }

    #[test]
    fn patrolling_stops_when_last_waypoint_reached_and_not_looping() {
        let mut ctrl = patrolling_controller(vec!["wp1".into(), "wp2".into()], false, 0.5);
        ctrl.blackboard.waypoint_index = 1; // at last waypoint
        let view = two_waypoint_view([-100.0, 0.0, 0.0], 0.0); // at wp2
        let output = tick(&ctrl, &view);
        let new_bb = output.new_blackboard.expect("new_blackboard must be set");
        assert_eq!(new_bb.waypoint_index, 2, "index advances past end to signal stop");
        // Next tick: idx >= len → emit thrust 0
        let mut ctrl2 = ctrl.clone();
        ctrl2.blackboard.waypoint_index = 2;
        let output2 = tick(&ctrl2, &view);
        match output2.inputs.first() {
            Some(AiInput::Helm { thrust, .. }) => {
                assert!((*thrust).abs() < 1e-5, "must stop (thrust=0) after last waypoint");
            }
            _ => panic!("expected Helm input"),
        }
    }

    // ── build_initial_state ────────────────────────────────────────────────

    #[test]
    fn build_initial_state_idle_name_returns_idle() {
        let config = crate::entity_config::BehaviourConfig {
            initial_state: "idle".into(),
            state: vec![],
            transition: vec![],
        };
        assert_eq!(build_initial_state(&config), AiState::Idle);
    }

    #[test]
    fn build_initial_state_patrolling_builds_patrolling_variant() {
        use crate::entity_config::StateConfig;
        let config = crate::entity_config::BehaviourConfig {
            initial_state: "patrol".into(),
            state: vec![StateConfig {
                name: "patrol".into(),
                kind: "patrolling".into(),
                waypoints: vec!["wp1".into(), "wp2".into()],
                loop_path: true,
                target_speed: 0.6,
            }],
            transition: vec![],
        };
        let state = build_initial_state(&config);
        assert_eq!(state, AiState::Patrolling {
            waypoints: vec!["wp1".into(), "wp2".into()],
            loop_path: true,
            target_speed: 0.6,
        });
    }

    #[test]
    fn build_initial_state_unknown_name_returns_idle() {
        let config = crate::entity_config::BehaviourConfig {
            initial_state: "attack".into(),
            state: vec![],
            transition: vec![],
        };
        assert_eq!(build_initial_state(&config), AiState::Idle);
    }
}
