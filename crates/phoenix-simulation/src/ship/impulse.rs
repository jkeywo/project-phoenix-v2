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

/// Default steering multiplier applied while impulse is active.
/// 0.0 = no steering, 0.1 = harsh but possible, 1.0 = full steering.
/// Ships can override this via `[helm_capability.impulse] steering_multiplier`
/// in their entity TOML.
pub const IMPULSE_STEERING_MULTIPLIER_DEFAULT: f32 = 0.1;

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
    /// - `steering` is scaled by `steering_multiplier` (harsh but not zero)
    ///
    /// Returns `(effective_max_speed, effective_steering)`.
    pub fn apply_to_physics(
        &self,
        base_max_speed: f32,
        steering: f32,
        speed_multiplier: f32,
        steering_multiplier: f32,
    ) -> (f32, f32) {
        if self.is_active() {
            let mult = if speed_multiplier > 0.0 {
                speed_multiplier
            } else {
                IMPULSE_SPEED_MULTIPLIER
            };
            (base_max_speed * mult, steering * steering_multiplier)
        } else {
            (base_max_speed, steering)
        }
    }
}

#[cfg(test)]
#[path = "impulse_tests.rs"]
mod tests;
