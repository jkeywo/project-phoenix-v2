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
mod tests {
    use super::*;

    #[test]
    fn new_starts_idle_with_full_battery() {
        let s = BoostState::new();
        assert!(!s.active);
        assert!((s.battery - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn toggle_engages_when_battery_available() {
        let mut s = BoostState::new();
        s.toggle();
        assert!(s.is_active());
    }

    #[test]
    fn toggle_disengages_when_active() {
        let mut s = BoostState::new();
        s.toggle();
        s.toggle();
        assert!(!s.is_active());
    }

    #[test]
    fn toggle_does_not_engage_on_empty_battery() {
        let mut s = BoostState::new();
        s.battery = 0.0;
        s.toggle();
        assert!(!s.is_active(), "must not engage with an empty battery");
    }

    #[test]
    fn tick_drains_battery_while_active() {
        let mut s = BoostState::new();
        s.toggle();
        s.tick(
            BOOST_ACTIVE_DURATION / 2.0,
            BOOST_ACTIVE_DURATION,
            BOOST_RECHARGE_DURATION,
        );
        assert!((s.battery - 0.5).abs() < 0.001);
        assert!(s.is_active());
    }

    #[test]
    fn tick_auto_disengages_when_battery_empties() {
        let mut s = BoostState::new();
        s.toggle();
        s.tick(
            BOOST_ACTIVE_DURATION,
            BOOST_ACTIVE_DURATION,
            BOOST_RECHARGE_DURATION,
        );
        assert!((s.battery).abs() < f32::EPSILON);
        assert!(!s.is_active(), "should auto-disengage at empty");
    }

    #[test]
    fn battery_never_goes_negative() {
        let mut s = BoostState::new();
        s.toggle();
        s.tick(
            BOOST_ACTIVE_DURATION * 5.0,
            BOOST_ACTIVE_DURATION,
            BOOST_RECHARGE_DURATION,
        );
        assert!(s.battery >= 0.0);
        assert!((s.battery).abs() < f32::EPSILON);
    }

    #[test]
    fn tick_recharges_battery_while_idle() {
        let mut s = BoostState::new();
        s.battery = 0.0;
        s.tick(
            BOOST_RECHARGE_DURATION / 2.0,
            BOOST_ACTIVE_DURATION,
            BOOST_RECHARGE_DURATION,
        );
        assert!((s.battery - 0.5).abs() < 0.001);
        assert!(!s.is_active());
    }

    #[test]
    fn recharge_clamps_at_full() {
        let mut s = BoostState::new();
        s.battery = 0.0;
        s.tick(
            BOOST_RECHARGE_DURATION * 5.0,
            BOOST_ACTIVE_DURATION,
            BOOST_RECHARGE_DURATION,
        );
        assert!((s.battery - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn can_reengage_with_partial_battery() {
        let mut s = BoostState::new();
        s.toggle();
        s.tick(
            BOOST_ACTIVE_DURATION / 2.0,
            BOOST_ACTIVE_DURATION,
            BOOST_RECHARGE_DURATION,
        );
        s.toggle(); // disengage at 50%
        assert!(!s.is_active());
        s.toggle(); // re-engage on partial battery
        assert!(s.is_active());
    }

    #[test]
    fn non_positive_durations_fall_back_to_consts() {
        let mut s = BoostState::new();
        s.toggle();
        // active_duration = 0 → falls back to BOOST_ACTIVE_DURATION
        s.tick(BOOST_ACTIVE_DURATION / 2.0, 0.0, 0.0);
        assert!(
            (s.battery - 0.5).abs() < 0.001,
            "expected fallback drain rate"
        );

        s.toggle(); // idle
        s.tick(BOOST_RECHARGE_DURATION / 2.0, 0.0, 0.0);
        // 0.5 + 0.5 = 1.0
        assert!(
            (s.battery - 1.0).abs() < 0.001,
            "expected fallback recharge rate"
        );
    }

    #[test]
    fn active_boost_does_not_drain_when_demand_is_zero() {
        let mut s = BoostState::new();
        s.toggle();
        s.tick_with_drain_factor(
            BOOST_ACTIVE_DURATION,
            BOOST_ACTIVE_DURATION,
            BOOST_RECHARGE_DURATION,
            0.0,
        );
        assert!((s.battery - 1.0).abs() < f32::EPSILON);
        assert!(s.is_active());
    }

    #[test]
    fn active_boost_drains_twice_as_fast_at_full_thrust_and_steering() {
        let mut s = BoostState::new();
        s.toggle();
        s.tick_with_drain_factor(
            BOOST_ACTIVE_DURATION / 4.0,
            BOOST_ACTIVE_DURATION,
            BOOST_RECHARGE_DURATION,
            2.0,
        );
        assert!((s.battery - 0.5).abs() < 0.001);
        assert!(s.is_active());
    }
}
