// Pure Rust module implementing lateral thrust physics.
// No Bevy or Rapier — pure computation, simulation layer applies results.

/// Lateral thrust tuning constants.
#[derive(Debug, Clone, Copy)]
pub struct LateralThrustConfig {
    /// Maximum lateral speed in world units per second.
    pub max_lateral_speed: f32,
    /// Lateral acceleration in world units per second squared.
    pub lateral_acceleration: f32,
}

impl Default for LateralThrustConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl LateralThrustConfig {
    pub fn new() -> Self {
        Self {
            max_lateral_speed: 15.0,
            lateral_acceleration: 15.0,
        }
    }
}

/// Compute the new lateral speed given current lateral speed, input (-1..1), dt, and config.
///
/// The input is a desired lateral thrust fraction (-1.0 = full port, 1.0 = full starboard).
/// Speed is driven toward `input * max_lateral_speed` at the configured acceleration rate.
/// When input is near zero, lateral speed decelerates toward 0.
pub fn compute_lateral_speed(
    current_speed: f32,
    input: f32,
    dt: f32,
    config: &LateralThrustConfig,
) -> f32 {
    let input = input.clamp(-1.0, 1.0);
    let max_spd = config.max_lateral_speed;

    if input.abs() > f32::EPSILON {
        let target = input * max_spd;
        let diff = target - current_speed;
        let step = config.lateral_acceleration * dt;
        let delta = if diff.abs() <= step {
            diff
        } else {
            step.copysign(diff)
        };
        (current_speed + delta).clamp(-max_spd, max_spd)
    } else {
        let decel = config.lateral_acceleration * dt;
        if current_speed > 0.0 {
            (current_speed - decel).max(0.0)
        } else if current_speed < 0.0 {
            (current_speed + decel).min(0.0)
        } else {
            0.0
        }
    }
}

/// Compute the lateral displacement (delta X and delta Z) given a yaw angle,
/// lateral speed, and delta time.
///
/// Lateral thrust pushes the ship perpendicular to its heading:
/// - Positive lateral speed displaces the ship to the ship's right (starboard).
/// - Negative lateral speed displaces the ship to the ship's left (port).
///
/// Returns `(delta_x, delta_z)` in world space.
pub fn lateral_displacement(yaw: f32, lateral_speed: f32, dt: f32) -> (f32, f32) {
    // Right vector from yaw: perpendicular to forward direction
    // Forward is (sin(yaw), -cos(yaw)), so right is (cos(yaw), sin(yaw))
    let right_x = yaw.cos();
    let right_z = yaw.sin();
    (right_x * lateral_speed * dt, right_z * lateral_speed * dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> LateralThrustConfig {
        LateralThrustConfig::new()
    }

    #[test]
    fn zero_input_decelerates_to_zero() {
        let speed = compute_lateral_speed(10.0, 0.0, 1.0, &default_config());
        assert!((speed - 0.0).abs() < f32::EPSILON, "expected 0, got {speed}");
    }

    #[test]
    fn full_input_approaches_max_speed() {
        let speed = compute_lateral_speed(0.0, 1.0, 5.0, &default_config());
        assert!(speed >= default_config().max_lateral_speed - 0.1);
    }

    #[test]
    fn negative_input_approaches_negative_max() {
        let speed = compute_lateral_speed(0.0, -1.0, 5.0, &default_config());
        assert!(speed <= -default_config().max_lateral_speed + 0.1);
    }

    #[test]
    fn speed_capped_at_max() {
        let speed = compute_lateral_speed(0.0, 1.0, 10.0, &default_config());
        assert!(speed <= default_config().max_lateral_speed);
    }

    #[test]
    fn speed_clamped_to_negative_max() {
        let speed = compute_lateral_speed(0.0, -1.0, 10.0, &default_config());
        assert!(speed >= -default_config().max_lateral_speed);
    }

    #[test]
    fn lateral_displacement_positive_for_starboard() {
        // Yaw = 0 → facing -Z; right is +X.
        let (dx, dz) = lateral_displacement(0.0, 10.0, 1.0);
        assert!(dx > 0.0, "expected positive X displacement, got {dx}");
        assert!((dz).abs() < 0.001, "expected minimal Z displacement, got {dz}");
    }

    #[test]
    fn lateral_displacement_negative_for_port() {
        // Yaw = 0 → facing -Z; left is -X.
        let (dx, dz) = lateral_displacement(0.0, -10.0, 1.0);
        assert!(dx < 0.0, "expected negative X displacement, got {dx}");
        assert!((dz).abs() < 0.001, "expected minimal Z displacement, got {dz}");
    }

    #[test]
    fn lateral_displacement_rotates_with_yaw() {
        // Yaw = PI/2 → facing +X; right is +Z.
        let (dx, dz) = lateral_displacement(std::f32::consts::FRAC_PI_2, 10.0, 1.0);
        assert!((dx).abs() < 0.001, "expected minimal X displacement, got {dx}");
        assert!(dz > 0.0, "expected positive Z displacement, got {dz}");
    }
}
