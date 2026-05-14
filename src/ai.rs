/// Pure AI controller module — no Bevy imports.
///
/// Owns the `AiController` struct, the fixed five-slot `Blackboard`,
/// `AiInput`, `AiTickOutput`, the pure `tick` function, and the
/// `should_emit` edge-emission filter.
use uuid::Uuid;
use serde::{Deserialize, Serialize};

// ── AiState ───────────────────────────────────────────────────────────────────

/// The set of states an AI controller can be in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AiState {
    Idle,
}

impl Default for AiState {
    fn default() -> Self {
        AiState::Idle
    }
}

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
/// Intentionally minimal for this scaffold slice — future slices will extend it.
#[derive(Debug, Clone, Default)]
pub struct WorldView {
    /// Current simulation time in seconds.
    pub sim_time: f64,
}

// ── AiTickOutput ─────────────────────────────────────────────────────────────

/// The output of a single AI tick.
#[derive(Debug, Clone, PartialEq)]
pub struct AiTickOutput {
    /// The state the controller should transition to (may be the same state).
    pub new_state: AiState,
    /// Input actions to inject into the simulation this tick.
    pub inputs: Vec<AiInput>,
}

// ── tick ──────────────────────────────────────────────────────────────────────

/// Advance an AI controller by one tick.
///
/// Pure function: takes a controller and a world view, returns the new state
/// and any inputs to emit. The caller is responsible for applying `new_state`
/// back to the controller.
pub fn tick(controller: &AiController, _world_view: &WorldView) -> AiTickOutput {
    match controller.current_state {
        AiState::Idle => AiTickOutput {
            new_state: AiState::Idle,
            inputs: vec![],
        },
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
}
