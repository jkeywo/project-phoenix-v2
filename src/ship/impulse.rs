// Pure Rust module for impulse drive mechanics.
// No Bevy — fully unit-testable on native.

/// Duration in seconds to charge up impulse drive.
pub const IMPULSE_CHARGE_DURATION: f32 = 3.0;

/// Speed multiplier applied during impulse.
pub const IMPULSE_SPEED_MULTIPLIER: f32 = 10.0;

/// Acceleration multiplier applied to the ship's base acceleration while
/// the impulse drive is active. The autopilot runs at full thrust during
/// the Active phase, and this boost lets it ramp to the boosted top speed
/// quickly without rewriting the steady-state acceleration curve.
pub const IMPULSE_ACCELERATION_MULTIPLIER: f32 = 5.0;

/// State of the impulse drive.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ImpulsePhase {
    /// Idle — not charging, not active.
    #[default]
    Idle,
    /// Charging — progress from 0.0 to 1.0.
    Charging,
    /// Active — impulse drive is engaged.
    Active,
}

/// Impulse drive state.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImpulseState {
    /// Current phase of the impulse drive.
    pub phase: ImpulsePhase,
    /// Charge progress: 0.0 (empty) to 1.0 (full).
    pub charge_progress: f32,
}

impl ImpulseState {
    /// Create a new, idle impulse state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin charging the impulse drive. No-op if already charging or active.
    pub fn start_charge(&mut self) {
        if self.phase == ImpulsePhase::Idle {
            self.phase = ImpulsePhase::Charging;
        }
    }

    /// Cancel charging or deactivate impulse drive. Returns to Idle.
    pub fn cancel_charge(&mut self) {
        self.phase = ImpulsePhase::Idle;
        self.charge_progress = 0.0;
    }

    /// Advance the impulse drive by `dt` seconds.
    /// When charge reaches 1.0, transitions to Active.
    /// `charge_duration` is the total time in seconds to fully charge.
    pub fn tick(&mut self, dt: f32, charge_duration: f32) {
        if self.phase == ImpulsePhase::Charging {
            let duration = if charge_duration > 0.0 {
                charge_duration
            } else {
                IMPULSE_CHARGE_DURATION
            };
            self.charge_progress = (self.charge_progress + dt / duration).min(1.0);
            if self.charge_progress >= 1.0 {
                self.phase = ImpulsePhase::Active;
            }
        }
    }

    /// Returns true when the impulse drive is active (engaged).
    pub fn is_active(&self) -> bool {
        self.phase == ImpulsePhase::Active
    }

    /// Apply impulse modifiers to physics inputs.
    ///
    /// During impulse:
    /// - `max_speed` is multiplied by `speed_multiplier`
    /// - `steering` input is forced to 0.0 (ignored)
    ///
    /// Returns `(effective_max_speed, effective_steering)`.
    pub fn apply_to_physics(
        &self,
        base_max_speed: f32,
        steering: f32,
        speed_multiplier: f32,
    ) -> (f32, f32) {
        if self.is_active() {
            let mult = if speed_multiplier > 0.0 {
                speed_multiplier
            } else {
                IMPULSE_SPEED_MULTIPLIER
            };
            (base_max_speed * mult, 0.0)
        } else {
            (base_max_speed, steering)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- struct + basic API ---

    #[test]
    fn start_charge_transitions_idle_to_charging() {
        let mut s = ImpulseState::new();
        s.start_charge();
        assert_eq!(s.phase, ImpulsePhase::Charging);
    }

    #[test]
    fn start_charge_is_noop_when_already_charging() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(1.0, IMPULSE_CHARGE_DURATION); // partial progress
        let progress_before = s.charge_progress;
        s.start_charge(); // should not reset progress
        assert_eq!(s.phase, ImpulsePhase::Charging);
        assert!((s.charge_progress - progress_before).abs() < f32::EPSILON);
    }

    #[test]
    fn start_charge_is_noop_when_active() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION); // fully charged → Active
        assert!(s.is_active());
        s.start_charge(); // should stay Active
        assert_eq!(s.phase, ImpulsePhase::Active);
    }

    #[test]
    fn cancel_charge_returns_to_idle_and_resets_progress() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(1.0, IMPULSE_CHARGE_DURATION);
        s.cancel_charge();
        assert_eq!(s.phase, ImpulsePhase::Idle);
        assert!((s.charge_progress).abs() < f32::EPSILON);
    }

    #[test]
    fn cancel_from_active_returns_to_idle() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        assert!(s.is_active());
        s.cancel_charge();
        assert_eq!(s.phase, ImpulsePhase::Idle);
    }

    // --- tick / charge progress ---

    #[test]
    fn tick_while_idle_does_nothing() {
        let mut s = ImpulseState::new();
        s.tick(10.0, IMPULSE_CHARGE_DURATION);
        assert_eq!(s.phase, ImpulsePhase::Idle);
        assert!((s.charge_progress).abs() < f32::EPSILON);
    }

    #[test]
    fn tick_advances_charge_progress() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION / 2.0, IMPULSE_CHARGE_DURATION);
        assert!((s.charge_progress - 0.5).abs() < 0.001);
        assert_eq!(s.phase, ImpulsePhase::Charging);
    }

    #[test]
    fn tick_to_full_charge_transitions_to_active() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        assert_eq!(s.phase, ImpulsePhase::Active);
        assert!((s.charge_progress - 1.0).abs() < f32::EPSILON);
        assert!(s.is_active());
    }

    #[test]
    fn charge_progress_capped_at_one() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION * 5.0, IMPULSE_CHARGE_DURATION);
        assert!((s.charge_progress - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn custom_charge_duration_charges_at_configured_rate() {
        let custom_duration = 6.0; // twice the default
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(3.0, custom_duration); // half of custom duration → 50%
        assert!((s.charge_progress - 0.5).abs() < 0.001);
        assert_eq!(s.phase, ImpulsePhase::Charging);
        s.tick(3.0, custom_duration); // full custom duration → Active
        assert_eq!(s.phase, ImpulsePhase::Active);
    }

    // --- physics modifiers ---

    #[test]
    fn apply_to_physics_returns_base_values_when_idle() {
        let s = ImpulseState::new();
        let (max_speed, steering) = s.apply_to_physics(25.0, 0.8, IMPULSE_SPEED_MULTIPLIER);
        assert!((max_speed - 25.0).abs() < f32::EPSILON);
        assert!((steering - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn apply_to_physics_returns_base_values_when_charging() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(1.0, IMPULSE_CHARGE_DURATION);
        let (max_speed, steering) = s.apply_to_physics(25.0, 0.8, IMPULSE_SPEED_MULTIPLIER);
        assert!((max_speed - 25.0).abs() < f32::EPSILON);
        assert!((steering - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn active_impulse_applies_10x_speed_multiplier() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        let (max_speed, _) = s.apply_to_physics(25.0, 0.0, IMPULSE_SPEED_MULTIPLIER);
        assert!((max_speed - 25.0 * IMPULSE_SPEED_MULTIPLIER).abs() < f32::EPSILON);
    }

    #[test]
    fn active_impulse_zeroes_steering_input() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        let (_, steering) = s.apply_to_physics(25.0, 0.9, IMPULSE_SPEED_MULTIPLIER);
        assert!((steering).abs() < f32::EPSILON);
    }

    #[test]
    fn custom_speed_multiplier_applied_when_active() {
        let custom_mult = 5.0;
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        let (max_speed, _) = s.apply_to_physics(25.0, 0.0, custom_mult);
        assert!((max_speed - 25.0 * custom_mult).abs() < f32::EPSILON);
    }
}
