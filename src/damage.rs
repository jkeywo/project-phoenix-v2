/// Compute collision damage to the ship hull.
///
/// Formula: `floor(clamp(5 + (forward_speed / max_speed) * 10, 5, 15))`
///
/// - At zero speed the damage is always 5.
/// - At full speed (forward_speed == max_speed) the damage is 15.
/// - Clamped to the range [5, 15] regardless of input.
pub fn collision_damage(forward_speed: f32, max_speed: f32) -> i32 {
    let raw = 5.0 + (forward_speed / max_speed) * 10.0;
    raw.clamp(5.0, 15.0).floor() as i32
}

/// Hull Integrity tracker. Starts at 100 and is clamped to [0, 100].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HullIntegrity {
    hp: i32,
}

impl HullIntegrity {
    pub fn new() -> Self {
        Self { hp: 100 }
    }

    pub fn current(&self) -> i32 {
        self.hp
    }

    /// Apply damage. Clamps HP floor to 0.
    pub fn apply_damage(&mut self, amount: i32) {
        self.hp = (self.hp - amount).max(0);
    }

    /// Restore HP. Clamps HP ceiling to 100.
    pub fn restore(&mut self, amount: i32) {
        self.hp = (self.hp + amount).min(100);
    }
    
    /// Create a new HullIntegrity with a specific HP value.
    pub fn with_hp(hp: i32) -> Self {
        Self { hp: hp.clamp(0, 100) }
    }
}

impl Default for HullIntegrity {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── collision_damage formula ──────────────────────────────────────────

    #[test]
    fn zero_speed_gives_minimum_damage() {
        assert_eq!(collision_damage(0.0, 50.0), 5);
    }

    #[test]
    fn max_speed_gives_maximum_damage() {
        assert_eq!(collision_damage(50.0, 50.0), 15);
    }

    #[test]
    fn mid_speed_floors_to_integer() {
        // At half max speed: 5 + 0.5 * 10 = 10.0 → 10
        assert_eq!(collision_damage(25.0, 50.0), 10);
    }

    #[test]
    fn one_third_speed_floors_correctly() {
        // 5 + (1/3) * 10 = 5 + 3.333… = 8.333… → floors to 8
        let speed = 50.0_f32 / 3.0;
        assert_eq!(collision_damage(speed, 50.0), 8);
    }

    #[test]
    fn above_max_speed_is_clamped_to_15() {
        assert_eq!(collision_damage(100.0, 50.0), 15);
    }

    #[test]
    fn negative_speed_is_clamped_to_5() {
        assert_eq!(collision_damage(-10.0, 50.0), 5);
    }

    // ── HullIntegrity ─────────────────────────────────────────────────────

    #[test]
    fn starts_at_100() {
        assert_eq!(HullIntegrity::new().current(), 100);
    }

    #[test]
    fn damage_reduces_hp() {
        let mut h = HullIntegrity::new();
        h.apply_damage(10);
        assert_eq!(h.current(), 90);
    }

    #[test]
    fn hp_cannot_go_below_zero() {
        let mut h = HullIntegrity::new();
        h.apply_damage(200);
        assert_eq!(h.current(), 0);
    }

    #[test]
    fn multiple_damage_events_accumulate() {
        let mut h = HullIntegrity::new();
        h.apply_damage(5);
        h.apply_damage(10);
        assert_eq!(h.current(), 85);
    }

    #[test]
    fn restore_increases_hp() {
        let mut h = HullIntegrity::new();
        h.apply_damage(20);
        h.restore(5);
        assert_eq!(h.current(), 85);
    }

    #[test]
    fn restore_clamps_at_100() {
        let mut h = HullIntegrity::new();
        h.restore(50);
        assert_eq!(h.current(), 100);
    }
}
