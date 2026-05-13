use crate::shield::ShieldSystem;

/// Apply `amount` of damage from bearing `bearing_relative` (radians) to the
/// ship, routing it through `shields` first. Returns the amount of hull damage
/// that leaked through (0 if shields absorbed everything). Does NOT apply the
/// damage to the hull — callers must call `apply_hull_damage` for that.
pub fn apply_damage_with_shields(
    amount: i32,
    bearing_relative: f32,
    shields: &mut ShieldSystem,
) -> i32 {
    shields.apply_damage(amount, bearing_relative)
}

/// Single code path for applying hull damage.
///
/// Takes the final hull damage amount (after shields), applies it to the hull,
/// tracks cumulative damage for breakdown bucket crossings, and returns:
///
/// - `hull_damage_applied`: what was actually subtracted from hull (may be less
///   than `amount` if hull hits 0)
/// - `new_cumulative_damage`: the updated cumulative total
/// - `breakdown_count`: how many complete 10-HP buckets were crossed
pub fn apply_hull_damage(
    hull: &mut HullIntegrity,
    amount: f32,
    cumulative_damage: f32,
) -> (f32, f32, u32) {
    let before = hull.current();
    hull.apply_damage(amount);
    let hull_damage = before - hull.current();
    let new_cumulative = cumulative_damage + hull_damage;
    let breakdown_count = crate::breakdown::breakdowns_from_damage(cumulative_damage, new_cumulative);
    (hull_damage, new_cumulative, breakdown_count)
}

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
#[derive(Clone, Debug, PartialEq)]
pub struct HullIntegrity {
    hp: f32,
}

impl HullIntegrity {
    pub fn new() -> Self {
        Self { hp: 100.0 }
    }

    pub fn current(&self) -> f32 {
        self.hp
    }

    /// Apply damage. Clamps HP floor to 0.
    pub fn apply_damage(&mut self, amount: f32) {
        self.hp = (self.hp - amount).max(0.0);
    }

    /// Restore HP. Clamps HP ceiling to 100.
    pub fn restore(&mut self, amount: f32) {
        self.hp = (self.hp + amount).min(100.0);
    }
    
    /// Create a new HullIntegrity with a specific HP value.
    pub fn with_hp(hp: f32) -> Self {
        Self { hp: hp.clamp(0.0, 100.0) }
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

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn starts_at_100() {
        assert!(near(HullIntegrity::new().current(), 100.0));
    }

    #[test]
    fn damage_reduces_hp() {
        let mut h = HullIntegrity::new();
        h.apply_damage(10.0);
        assert!(near(h.current(), 90.0));
    }

    #[test]
    fn hp_cannot_go_below_zero() {
        let mut h = HullIntegrity::new();
        h.apply_damage(200.0);
        assert!(near(h.current(), 0.0));
    }

    #[test]
    fn multiple_damage_events_accumulate() {
        let mut h = HullIntegrity::new();
        h.apply_damage(5.0);
        h.apply_damage(10.0);
        assert!(near(h.current(), 85.0));
    }

    #[test]
    fn restore_increases_hp() {
        let mut h = HullIntegrity::new();
        h.apply_damage(20.0);
        h.restore(5.0);
        assert!(near(h.current(), 85.0));
    }

    #[test]
    fn restore_clamps_at_100() {
        let mut h = HullIntegrity::new();
        h.restore(50.0);
        assert!(near(h.current(), 100.0));
    }

    // ── apply_hull_damage helper ────────────────────────────────────────────

    #[test]
    fn apply_hull_damage_zero_damage_no_change() {
        let mut hull = HullIntegrity::new();
        let (applied, cumulative, breakdowns) = crate::damage::apply_hull_damage(&mut hull, 0.0, 0.0);
        assert_eq!(applied, 0.0);
        assert_eq!(cumulative, 0.0);
        assert_eq!(breakdowns, 0);
        assert_eq!(hull.current(), 100.0);
    }

    #[test]
    fn apply_hull_damage_fractional_accumulates() {
        let mut hull = HullIntegrity::new();
        let (applied, cumulative, breakdowns) = crate::damage::apply_hull_damage(&mut hull, 3.5, 0.0);
        assert!((applied - 3.5).abs() < 1e-6, "applied={}", applied);
        assert!((cumulative - 3.5).abs() < 1e-6, "cumulative={}", cumulative);
        assert_eq!(breakdowns, 0);
        assert!((hull.current() - 96.5).abs() < 1e-6);
    }

    #[test]
    fn apply_hull_damage_fractional_crosses_10hp_bucket() {
        let mut hull = HullIntegrity::new();
        // First tick: 5.5 damage
        let (applied, cumulative, breakdowns) = crate::damage::apply_hull_damage(&mut hull, 5.5, 0.0);
        assert!((applied - 5.5).abs() < 1e-6);
        assert_eq!(breakdowns, 0);
        // Second tick: 5.5 damage = 11 total → crosses 10
        let (_, cumulative2, breakdowns2) = crate::damage::apply_hull_damage(&mut hull, 5.5, cumulative);
        assert!((cumulative2 - 11.0).abs() < 1e-6);
        assert_eq!(breakdowns2, 1);
        assert!((hull.current() - 89.0).abs() < 1e-6);
    }

    #[test]
    fn apply_hull_damage_crosses_two_buckets() {
        let mut hull = HullIntegrity::new();
        let (applied, cumulative, breakdowns) = crate::damage::apply_hull_damage(&mut hull, 25.0, 0.0);
        assert!((applied - 25.0).abs() < 1e-6);
        assert!((cumulative - 25.0).abs() < 1e-6);
        assert_eq!(breakdowns, 2);
    }

    #[test]
    fn apply_hull_damage_also_applies_to_hull() {
        // The helper should actually modify the hull, not just return numbers.
        let mut hull = HullIntegrity::new();
        apply_hull_damage(&mut hull, 10.0, 0.0);
        assert_eq!(hull.current(), 90.0);
    }

    #[test]
    fn hull_integrity_with_fractional_damage() {
        let mut h = HullIntegrity::new();
        h.apply_damage(3.5);
        assert!((h.current() - 96.5).abs() < 1e-6);
    }

    #[test]
    fn hull_integrity_restore_with_fractional() {
        let mut h = HullIntegrity::new();
        h.apply_damage(10.5);
        assert!((h.current() - 89.5).abs() < 1e-6);
        h.restore(3.5);
        assert!((h.current() - 93.0).abs() < 1e-6);
    }

    #[test]
    fn hull_integrity_fractional_capped_at_zero() {
        let mut h = HullIntegrity::new();
        h.apply_damage(200.5);
        assert_eq!(h.current(), 0.0);
    }

    #[test]
    fn hull_integrity_with_hp_fractional() {
        let h = HullIntegrity::with_hp(75.5);
        assert!((h.current() - 75.5).abs() < 1e-6);
    }

    // ── apply_damage_with_shields ─────────────────────────────────────────────

    #[test]
    fn shield_absorbs_damage_hull_unchanged() {
        let mut shields = crate::shield::ShieldSystem::default(); // 4 facings, 100 hp each
        let hull_damage = apply_damage_with_shields(20, 0.0, &mut shields);
        // Fore shield absorbs all 20; hull unchanged (no hull ref passed).
        assert_eq!(hull_damage, 0);
        assert_eq!(shields.facings[0].hp, 80);
    }

    #[test]
    fn depleted_shield_passes_overflow_to_hull() {
        use crate::shield::{ShieldConfig, ShieldSystem};
        let config = ShieldConfig { max_hp: 50, ..Default::default() };
        let mut shields = ShieldSystem::new(&config);
        // 60 damage, fore shield has 50 → 10 overflow to hull
        let hull_damage = apply_damage_with_shields(60, 0.0, &mut shields);
        assert_eq!(hull_damage, 10);
    }

    #[test]
    fn offline_shield_passes_all_damage_to_hull() {
        use crate::shield::{ShieldConfig, ShieldSystem};
        let config = ShieldConfig { max_hp: 50, ..Default::default() };
        let mut shields = ShieldSystem::new(&config);
        // Deplete fore shield (goes offline)
        apply_damage_with_shields(50, 0.0, &mut shields);
        // Now fore is offline; any further hit at bearing 0 goes straight to hull
        let hull_damage = apply_damage_with_shields(15, 0.0, &mut shields);
        assert_eq!(hull_damage, 15);
    }

    #[test]
    fn damage_routed_to_correct_facing_not_hull() {
        let mut shields = crate::shield::ShieldSystem::default();
        // Hit from the port side (bearing -π/2)
        let hull_damage = apply_damage_with_shields(10, -std::f32::consts::FRAC_PI_2, &mut shields);
        assert_eq!(hull_damage, 0);
        assert_eq!(shields.facings[1].hp, 90); // Port
        assert_eq!(shields.facings[0].hp, 100); // Fore untouched
    }
}
