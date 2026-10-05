// Pure Rust module for the helm boost drive.
// No Bevy — fully unit-testable on native.

/// Default speed/acceleration multiplier applied while boost is engaged.
/// Used as a fallback when the TOML omits a value.
pub const BOOST_MULTIPLIER: f32 = 3.0;

/// Default steering multiplier applied while boost is engaged.
/// A value of 1.0 preserves the normal yaw rate.
pub const BOOST_STEERING_MULTIPLIER: f32 = 1.0;

/// Default time in seconds a full battery lasts while boost is engaged.
pub const BOOST_ACTIVE_DURATION: f32 = 4.0;

/// Default time in seconds for an empty battery to recharge to full.
pub const BOOST_RECHARGE_DURATION: f32 = 20.0;

/// Boost drive battery state.
///
/// Toggle/partial-drain model: engaging drains the battery over
/// `active_duration`; disengaging lets it recharge over `recharge_duration`.
/// The drive can be re-engaged with a partial battery and auto-disengages when
/// the battery hits empty.
#[derive(Debug, Clone, Copy)]
pub struct BoostState {
    /// Whether the boost drive is currently engaged.
    pub active: bool,
    /// Battery charge: 0.0 (empty) to 1.0 (full).
    pub battery: f32,
}

impl Default for BoostState {
    fn default() -> Self {
        Self::new()
    }
}

impl BoostState {
    /// Create a new boost state: idle with a full battery.
    pub fn new() -> Self {
        Self {
            active: false,
            battery: 1.0,
        }
    }

    /// Toggle engagement. Engages only if there is charge left; disengaging is
    /// always allowed.
    pub fn toggle(&mut self) {
        if self.active {
            self.active = false;
        } else if self.battery > 0.0 {
            self.active = true;
        }
    }

    /// Explicitly engage the boost drive. No-op when battery is empty.
    pub fn activate(&mut self) {
        if self.battery > 0.0 {
            self.active = true;
        }
    }

    /// Explicitly disengage the boost drive.
    pub fn deactivate(&mut self) {
        self.active = false;
    }

    /// Advance the boost drive by `dt` seconds.
    ///
    /// While active the battery drains over `active_duration` and the drive
    /// auto-disengages when empty. While idle the battery recharges over
    /// `recharge_duration`, clamped to full. Non-positive durations fall back
    /// to the module constants instead of dividing by zero.
    pub fn tick(&mut self, dt: f32, active_duration: f32, recharge_duration: f32) {
        self.tick_with_drain_factor(dt, active_duration, recharge_duration, 1.0);
    }

    /// Advance the boost drive with active drain scaled by drive demand.
    ///
    /// `drain_factor` is normally `abs(thrust) + abs(steering)`, so boost does
    /// not drain while the helm is idle, drains at the base rate with either
    /// full thrust or full steering, and drains at double rate when both are at
    /// full deflection.
    pub fn tick_with_drain_factor(
        &mut self,
        dt: f32,
        active_duration: f32,
        recharge_duration: f32,
        drain_factor: f32,
    ) {
        let active_dur = if active_duration > 0.0 {
            active_duration
        } else {
            BOOST_ACTIVE_DURATION
        };
        let recharge_dur = if recharge_duration > 0.0 {
            recharge_duration
        } else {
            BOOST_RECHARGE_DURATION
        };

        if self.active {
            let drain_factor = drain_factor.max(0.0);
            self.battery = (self.battery - (dt / active_dur) * drain_factor).max(0.0);
            if self.battery <= 0.0 {
                self.active = false;
            }
        } else {
            self.battery = (self.battery + dt / recharge_dur).min(1.0);
        }
    }

    /// Returns true when the boost drive is engaged.
    pub fn is_active(&self) -> bool {
        self.active
    }
}

#[cfg(test)]
#[path = "boost_tests.rs"]
mod tests;
