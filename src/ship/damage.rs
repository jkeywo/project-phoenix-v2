use crate::messages::Console;
use crate::shield::ShieldSystem;
use rand::Rng;
use serde::{Deserialize, Serialize};

// ── DamageTier ────────────────────────────────────────────────────────────────

/// HP-derived damage tier for a single console.
///
/// Tiers are computed from `current_hp / max_hp` against configurable
/// thresholds, except `Destroyed` which latches at exactly 0 HP.
///
/// | Tier        | Condition                                  |
/// |-------------|--------------------------------------------|
/// | Operational | `current / max >= damaged_threshold_pct`   |
/// | Damaged     | `disabled_threshold_pct <= ratio < damaged_threshold_pct` |
/// | Disabled    | `0 < ratio < disabled_threshold_pct`       |
/// | Destroyed   | `current == 0`                             |
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DamageTier {
    Operational,
    Damaged,
    Disabled,
    /// HP reached exactly 0. Unrepairable until `restore()` is called.
    Destroyed,
}

// ── ConsoleTierConfig ─────────────────────────────────────────────────────────

/// Per-console threshold configuration for damage tier derivation.
///
/// Fields are HP-fraction values in `[0.0, 1.0]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConsoleTierConfig {
    /// HP fraction below which the console enters the `Damaged` tier.
    /// Default: `0.75` (below 75 % → Damaged).
    pub damaged_threshold_pct: f32,
    /// HP fraction below which the console enters the `Disabled` tier.
    /// Default: `0.25` (below 25 % → Disabled).
    pub disabled_threshold_pct: f32,
    /// Performance reduction applied when the console is in the `Damaged` or
    /// `Disabled` tier (e.g. `0.15` = 15 % reduction). Sourced from
    /// `debuff_magnitude` in the `[[hull.console_hull]]` TOML block.
    /// Default: `0.15`.
    pub debuff_magnitude: f32,
}

impl Default for ConsoleTierConfig {
    fn default() -> Self {
        Self {
            damaged_threshold_pct: 0.75,
            disabled_threshold_pct: 0.25,
            debuff_magnitude: 0.15,
        }
    }
}

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

/// Apply hull damage via `ConsoleHull`.
///
/// Takes the final hull damage amount (after shields), distributes it randomly
/// across consoles, and returns:
///
/// - `hull_damage_applied`: what was actually absorbed by consoles
/// - `ship_destroyed`: true when all consoles have reached 0 HP after this hit
pub fn apply_hull_damage(
    hull: &mut ConsoleHull,
    amount: f32,
    rng: &mut impl rand::Rng,
) -> (f32, bool) {
    let before = hull.total_current();
    hull.apply_damage(amount, rng);
    let hull_damage = before - hull.total_current();
    let ship_destroyed = hull.is_destroyed();
    (hull_damage, ship_destroyed)
}

/// Split an incoming damage amount into a pierced portion (bypasses shields,
/// goes straight to hull) and an absorbed portion (routes through the shield
/// quadrant pipeline).
///
/// `shield_pierce` is clamped defensively to `[0.0, 1.0]`. NaN is treated as
/// `0.0` (no pierce — fully shielded). Values outside the range do not panic.
///
/// Returns `(pierced, absorbed)` such that `pierced + absorbed == damage`
/// (modulo float precision) and both are non-negative when `damage >= 0`.
pub fn split_damage_for_pierce(damage: f32, shield_pierce: f32) -> (f32, f32) {
    let pierce = if shield_pierce.is_nan() {
        0.0
    } else {
        shield_pierce.clamp(0.0, 1.0)
    };
    let pierced = damage * pierce;
    let absorbed = damage * (1.0 - pierce);
    (pierced, absorbed)
}

/// Compute collision damage proportional to absolute speed.
///
/// Formula: `round(|forward_speed| * 0.5)`
///
/// - At zero speed the damage is 0.
/// - At full impulse (~250 u/s) the damage is ~125.
pub fn collision_damage(forward_speed: f32) -> i32 {
    (forward_speed.abs() * 0.5).round() as i32
}

// ── ConsoleHull ───────────────────────────────────────────────────────────────

/// Per-console hull tracker.
///
/// Stores `(console, current_hp, max_hp)` entries plus per-console tier
/// thresholds. Damage is distributed randomly across consoles that still have
/// HP, spilling to further random consoles when a console reaches 0. Repair
/// targets a specific console.
#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleHull {
    entries: Vec<(Console, f32, f32)>,
    /// Parallel vec of tier thresholds, one entry per `entries` element.
    tier_configs: Vec<ConsoleTierConfig>,
}

impl ConsoleHull {
    /// Build from a list of `(console, max_hp)` pairs using default tier
    /// thresholds. All consoles start at full HP.
    pub fn from_config(config: &[(Console, f32)]) -> Self {
        Self {
            entries: config
                .iter()
                .map(|(c, max)| (c.clone(), *max, *max))
                .collect(),
            tier_configs: config.iter().map(|_| ConsoleTierConfig::default()).collect(),
        }
    }

    /// Build from a list of `(console, max_hp, tier_config)` triples.
    /// All consoles start at full HP.
    pub fn from_config_with_tiers(config: &[(Console, f32, ConsoleTierConfig)]) -> Self {
        Self {
            entries: config
                .iter()
                .map(|(c, max, _)| (c.clone(), *max, *max))
                .collect(),
            tier_configs: config.iter().map(|(_, _, tc)| *tc).collect(),
        }
    }

    /// Return the `DamageTier` for the given console.
    ///
    /// - `Destroyed`: `current == 0`
    /// - `Disabled`: `current/max < disabled_threshold_pct`
    /// - `Damaged`: `current/max < damaged_threshold_pct`
    /// - `Operational`: otherwise
    ///
    /// Returns `Operational` for consoles not tracked by this hull.
    pub fn tier_for(&self, console: Console) -> DamageTier {
        let idx = self.entries.iter().position(|(c, _, _)| *c == console);
        let Some(idx) = idx else {
            return DamageTier::Operational;
        };
        let (_, cur, max) = self.entries[idx];
        let tier_cfg = &self.tier_configs[idx];

        if cur == 0.0 {
            return DamageTier::Destroyed;
        }
        let ratio = if max > 0.0 { cur / max } else { 0.0 };
        if ratio < tier_cfg.disabled_threshold_pct {
            DamageTier::Disabled
        } else if ratio < tier_cfg.damaged_threshold_pct {
            DamageTier::Damaged
        } else {
            DamageTier::Operational
        }
    }

    /// Apply `amount` of damage distributed across consoles above 0 HP, weighted
    /// by their remaining HP (a console with more HP is proportionally more likely
    /// to absorb the next hit). Damage spills to further weighted selections when
    /// a console is exhausted. Consoles already at 0 HP are never targeted.
    pub fn apply_damage(&mut self, mut amount: f32, rng: &mut impl Rng) {
        while amount > 0.0 {
            let total: f32 = self
                .entries
                .iter()
                .filter(|(_, cur, _)| *cur > 0.0)
                .map(|(_, cur, _)| *cur)
                .sum();
            if total == 0.0 {
                break;
            }
            // Weighted selection: generate r in [0, total), subtract each
            // console's HP in order; choose the first one that drives r negative.
            let mut r = rng.random::<f32>() * total;
            let mut chosen = None;
            for (i, (_, cur, _)) in self.entries.iter().enumerate() {
                if *cur <= 0.0 {
                    continue;
                }
                r -= cur;
                if r < 0.0 {
                    chosen = Some(i);
                    break;
                }
            }
            // Float-precision safety: fall back to the last available console.
            let idx = chosen.unwrap_or_else(|| {
                self.entries
                    .iter()
                    .enumerate()
                    .rfind(|(_, (_, cur, _))| *cur > 0.0)
                    .unwrap()
                    .0
            });
            let (_, cur, _) = &mut self.entries[idx];
            let absorbed = amount.min(*cur);
            *cur -= absorbed;
            amount -= absorbed;
        }
    }

    /// Read-only view of all `(console, current_hp, max_hp)` entries.
    pub fn entries(&self) -> &[(Console, f32, f32)] {
        &self.entries
    }

    /// Restore `amount` HP to a specific console, clamped to its max.
    /// Consoles not present in the map are silently ignored.
    pub fn restore(&mut self, console: Console, amount: f32) {
        for (c, cur, max) in &mut self.entries {
            if *c == console {
                *cur = (*cur + amount).min(*max);
                return;
            }
        }
    }

    /// Sum of current HP across all consoles.
    pub fn total_current(&self) -> f32 {
        self.entries.iter().map(|(_, cur, _)| cur).sum()
    }

    /// Sum of max HP across all consoles.
    pub fn total_max(&self) -> f32 {
        self.entries.iter().map(|(_, _, max)| max).sum()
    }

    /// True only when every console is at 0 HP.
    pub fn is_destroyed(&self) -> bool {
        self.entries.iter().all(|(_, cur, _)| *cur == 0.0)
    }

    /// Current HP for a specific console. Returns `None` if not tracked.
    pub fn current_for(&self, console: Console) -> Option<f32> {
        self.entries
            .iter()
            .find(|(c, _, _)| *c == console)
            .map(|(_, cur, _)| *cur)
    }

    /// Restore `amount` HP to the first console that is below its max HP.
    /// Useful for repair systems that don't yet target a specific console.
    /// Returns the console that was restored, or `None` if all are at max.
    pub fn restore_any_damaged(&mut self, amount: f32) -> Option<Console> {
        for (c, cur, max) in &mut self.entries {
            if *cur < *max {
                *cur = (*cur + amount).min(*max);
                return Some(c.clone());
            }
        }
        None
    }

    /// Return the active debuff magnitude for the given console.
    ///
    /// - `Operational` or `Destroyed` → `0.0` (fully operational or fully offline;
    ///   no partial debuff applies).
    /// - `Damaged` or `Disabled` → `tier_config.debuff_magnitude` from the
    ///   per-console TOML configuration.
    ///
    /// Returns `0.0` for consoles not tracked by this hull.
    pub fn debuff_magnitude_for(&self, console: Console) -> f32 {
        let idx = self.entries.iter().position(|(c, _, _)| *c == console);
        let Some(idx) = idx else {
            return 0.0;
        };
        match self.tier_for(console) {
            DamageTier::Operational | DamageTier::Destroyed => 0.0,
            DamageTier::Damaged | DamageTier::Disabled => self.tier_configs[idx].debuff_magnitude,
        }
    }

    /// Returns `true` if the given console is at its maximum HP (or not tracked).
    pub fn is_at_max(&self, console: &Console) -> bool {
        match self.entries.iter().find(|(c, _, _)| c == console) {
            Some((_, cur, max)) => *cur >= *max,
            None => true, // not tracked → treat as full
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── split_damage_for_pierce helper ────────────────────────────────────

    #[test]
    fn pierce_zero_routes_all_damage_to_shields() {
        let (pierced, absorbed) = split_damage_for_pierce(10.0, 0.0);
        assert!((pierced - 0.0).abs() < 1e-6);
        assert!((absorbed - 10.0).abs() < 1e-6);
    }

    #[test]
    fn pierce_one_routes_all_damage_to_hull() {
        let (pierced, absorbed) = split_damage_for_pierce(10.0, 1.0);
        assert!((pierced - 10.0).abs() < 1e-6);
        assert!((absorbed - 0.0).abs() < 1e-6);
    }

    #[test]
    fn pierce_fractional_splits_proportionally() {
        let (pierced, absorbed) = split_damage_for_pierce(10.0, 0.3);
        assert!((pierced - 3.0).abs() < 1e-6, "pierced={}", pierced);
        assert!((absorbed - 7.0).abs() < 1e-6, "absorbed={}", absorbed);
    }

    #[test]
    fn pierce_above_one_clamps_to_one() {
        let (pierced, absorbed) = split_damage_for_pierce(10.0, 2.5);
        assert!((pierced - 10.0).abs() < 1e-6);
        assert!((absorbed - 0.0).abs() < 1e-6);
    }

    #[test]
    fn pierce_below_zero_clamps_to_zero() {
        let (pierced, absorbed) = split_damage_for_pierce(10.0, -0.5);
        assert!((pierced - 0.0).abs() < 1e-6);
        assert!((absorbed - 10.0).abs() < 1e-6);
    }

    #[test]
    fn pierce_nan_treated_as_zero_no_panic() {
        let (pierced, absorbed) = split_damage_for_pierce(10.0, f32::NAN);
        assert!((pierced - 0.0).abs() < 1e-6);
        assert!((absorbed - 10.0).abs() < 1e-6);
    }

    #[test]
    fn pierce_infinity_clamps_to_one() {
        let (pierced, absorbed) = split_damage_for_pierce(10.0, f32::INFINITY);
        assert!((pierced - 10.0).abs() < 1e-6);
        assert!((absorbed - 0.0).abs() < 1e-6);
    }

    #[test]
    fn pierce_negative_infinity_clamps_to_zero() {
        let (pierced, absorbed) = split_damage_for_pierce(10.0, f32::NEG_INFINITY);
        assert!((pierced - 0.0).abs() < 1e-6);
        assert!((absorbed - 10.0).abs() < 1e-6);
    }

    // ── collision_damage formula ──────────────────────────────────────────

    #[test]
    fn zero_speed_gives_zero_damage() {
        assert_eq!(collision_damage(0.0), 0);
    }

    #[test]
    fn full_impulse_gives_125_damage() {
        // 250 u/s * 0.5 = 125
        assert_eq!(collision_damage(250.0), 125);
    }

    #[test]
    fn half_impulse_rounds_correctly() {
        assert_eq!(collision_damage(125.0), 63);
    }

    #[test]
    fn negative_speed_uses_absolute_value() {
        assert_eq!(collision_damage(-100.0), 50);
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    // ── apply_hull_damage helper ────────────────────────────────────────────

    fn single_console_hull(hp: f32) -> ConsoleHull {
        ConsoleHull::from_config(&[(Console::Helm, hp)])
    }

    #[test]
    fn apply_hull_damage_zero_damage_no_change() {
        let mut hull = single_console_hull(100.0);
        let mut rng = rand::rng();
        let (applied, _destroyed) = crate::damage::apply_hull_damage(&mut hull, 0.0, &mut rng);
        assert_eq!(applied, 0.0);
        assert!((hull.total_current() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn apply_hull_damage_fractional_accumulates() {
        let mut hull = single_console_hull(100.0);
        let mut rng = rand::rng();
        let (applied, _destroyed) = crate::damage::apply_hull_damage(&mut hull, 3.5, &mut rng);
        assert!((applied - 3.5).abs() < 1e-6, "applied={}", applied);
        assert!((hull.total_current() - 96.5).abs() < 1e-6);
    }

    #[test]
    fn apply_hull_damage_also_applies_to_hull() {
        let mut hull = single_console_hull(100.0);
        let mut rng = rand::rng();
        apply_hull_damage(&mut hull, 10.0, &mut rng);
        assert!((hull.total_current() - 90.0).abs() < 1e-6);
    }

    #[test]
    fn station_hull_can_be_initialised_with_custom_hp_and_absorbs_damage() {
        let mut station_hull = single_console_hull(80.0);
        let mut rng = rand::rng();
        let (applied, _destroyed) = apply_hull_damage(&mut station_hull, 30.0, &mut rng);
        assert!((applied - 30.0).abs() < 1e-6, "applied={}", applied);
        assert!((station_hull.total_current() - 50.0).abs() < 1e-6);
    }

    #[test]
    fn station_hull_reaches_zero_on_destruction() {
        let mut station_hull = single_console_hull(50.0);
        let mut rng = rand::rng();
        apply_hull_damage(&mut station_hull, 100.0, &mut rng);
        assert_eq!(
            station_hull.total_current(),
            0.0,
            "station hull should reach zero"
        );
    }

    #[test]
    fn apply_hull_damage_returns_ship_destroyed_false_when_hp_remains() {
        let mut hull = single_console_hull(100.0);
        let mut rng = rand::rng();
        let (_applied, destroyed) = apply_hull_damage(&mut hull, 10.0, &mut rng);
        assert!(!destroyed, "ship should not be destroyed when HP remains");
    }

    #[test]
    fn apply_hull_damage_returns_ship_destroyed_true_when_all_consoles_at_zero() {
        let mut hull = single_console_hull(20.0);
        let mut rng = rand::rng();
        let (_applied, destroyed) = apply_hull_damage(&mut hull, 100.0, &mut rng);
        assert!(
            destroyed,
            "ship should be destroyed when all consoles reach 0"
        );
    }

    #[test]
    fn apply_hull_damage_spillover_fires_destroyed_after_second_console_wiped() {
        let mut hull = ConsoleHull::from_config(&[(Console::Helm, 5.0), (Console::Tactical, 10.0)]);
        let mut rng = rand::rng();
        let (_applied, destroyed) = apply_hull_damage(&mut hull, 20.0, &mut rng);
        assert!(
            destroyed,
            "spillover should destroy the ship after both consoles reach 0"
        );
        assert!(hull.is_destroyed());
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
        let config = ShieldConfig {
            max_hp: 50,
            ..Default::default()
        };
        let mut shields = ShieldSystem::new(&config);
        // 60 damage, fore shield has 50 → 10 overflow to hull
        let hull_damage = apply_damage_with_shields(60, 0.0, &mut shields);
        assert_eq!(hull_damage, 10);
    }

    #[test]
    fn offline_shield_passes_all_damage_to_hull() {
        use crate::shield::{ShieldConfig, ShieldSystem};
        let config = ShieldConfig {
            max_hp: 50,
            ..Default::default()
        };
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

    // ── ConsoleHull ──────────────────────────────────────────────────────────

    fn four_console_hull() -> ConsoleHull {
        ConsoleHull::from_config(&[
            (Console::Helm, 25.0),
            (Console::Tactical, 25.0),
            (Console::Power, 25.0),
            (Console::Shields, 25.0),
        ])
    }

    // Cycle 1: aggregates are correct at full HP
    #[test]
    fn console_hull_total_current_and_max_at_start() {
        let hull = four_console_hull();
        assert!(near(hull.total_current(), 100.0));
        assert!(near(hull.total_max(), 100.0));
    }

    // Cycle 2: not destroyed when HP remains
    #[test]
    fn console_hull_not_destroyed_when_hp_remains() {
        let hull = four_console_hull();
        assert!(!hull.is_destroyed());
    }

    // Cycle 3: is_destroyed only when all consoles at 0
    #[test]
    fn console_hull_is_destroyed_only_when_all_at_zero() {
        let mut hull = four_console_hull();
        let mut rng = rand::rng();
        hull.apply_damage(1000.0, &mut rng); // wipe everything
        assert!(hull.is_destroyed());
    }

    // Cycle 4: apply_damage reduces total_current
    #[test]
    fn apply_damage_reduces_total_hp() {
        let mut hull = four_console_hull();
        let mut rng = rand::rng();
        hull.apply_damage(10.0, &mut rng);
        assert!(near(hull.total_current(), 90.0));
    }

    // Cycle 5: damage never targets consoles at 0 HP (spillover)
    #[test]
    fn apply_damage_skips_depleted_consoles() {
        // Build hull with one console at very low HP so it depletes first.
        // Use a seeded RNG to control which console is chosen.
        let mut hull =
            ConsoleHull::from_config(&[(Console::Helm, 5.0), (Console::Tactical, 100.0)]);
        let mut rng = rand::rng();
        // Apply 110 damage — should wipe both consoles (5 + 105 spill to Tactical)
        hull.apply_damage(110.0, &mut rng);
        assert!(hull.is_destroyed(), "all consoles should be at 0");
        assert!(near(hull.total_current(), 0.0));
    }

    // Cycle 6: restore heals only the specified console
    #[test]
    fn restore_heals_only_targeted_console() {
        let mut hull = four_console_hull();
        let mut rng = rand::rng();
        hull.apply_damage(100.0, &mut rng); // wipe all
        hull.restore(Console::Helm, 10.0);
        // Only Helm should have HP restored
        assert!(near(hull.current_for(Console::Helm).unwrap(), 10.0));
        assert!(near(hull.current_for(Console::Tactical).unwrap(), 0.0));
        assert!(near(hull.current_for(Console::Power).unwrap(), 0.0));
        assert!(near(hull.current_for(Console::Shields).unwrap(), 0.0));
    }

    // Cycle 7: restore is clamped to max HP
    #[test]
    fn restore_is_clamped_to_max() {
        let mut hull = four_console_hull();
        hull.restore(Console::Helm, 50.0); // already at 25, restore 50 → capped at 25
        assert!(near(hull.current_for(Console::Helm).unwrap(), 25.0));
        assert!(near(hull.total_current(), 100.0));
    }

    // Cycle 8: from_config with default ship values
    #[test]
    fn default_ship_config_four_consoles_at_25hp() {
        let hull = four_console_hull();
        assert!(near(hull.current_for(Console::Helm).unwrap(), 25.0));
        assert!(near(hull.current_for(Console::Tactical).unwrap(), 25.0));
        assert!(near(hull.current_for(Console::Power).unwrap(), 25.0));
        assert!(near(hull.current_for(Console::Shields).unwrap(), 25.0));
    }

    // Cycle 9: restore on unknown console is a no-op
    #[test]
    fn weighted_selection_favours_higher_hp_console() {
        // Tactical has 99× more HP than Helm, so it should absorb ~99% of hits.
        let mut hull = ConsoleHull::from_config(&[(Console::Helm, 1.0), (Console::Tactical, 99.0)]);
        let mut rng = rand::rng();
        let mut tactical_hits = 0u32;
        let trials = 10_000;
        for _ in 0..trials {
            let before_tactical = hull.current_for(Console::Tactical).unwrap();
            hull.apply_damage(0.001, &mut rng); // tiny damage to record which was chosen
            let after_tactical = hull.current_for(Console::Tactical).unwrap();
            if after_tactical < before_tactical {
                tactical_hits += 1;
            }
        }
        // Expect ~99% of hits on Tactical; allow generous margin due to HP drift.
        let fraction = tactical_hits as f32 / trials as f32;
        assert!(
            fraction > 0.90,
            "Tactical should absorb >90% of hits, got {:.1}%",
            fraction * 100.0
        );
    }

    #[test]
    fn restore_on_unknown_console_is_noop() {
        let mut hull = four_console_hull();
        let before = hull.total_current();
        hull.restore(Console::Navigation, 10.0); // not in the map
        assert!(near(hull.total_current(), before));
    }

    // ── DamageTier tests ──────────────────────────────────────────────────────

    #[test]
    fn tier_is_operational_at_full_hp() {
        let hull = ConsoleHull::from_config(&[(Console::Helm, 25.0)]);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Operational);
    }

    #[test]
    fn tier_is_damaged_below_damaged_threshold() {
        // Default damaged_threshold = 0.75. 74% of 100 → Damaged.
        let mut hull = ConsoleHull::from_config(&[(Console::Helm, 100.0)]);
        let mut rng = rand::rng();
        // Directly set HP to 74 by restoring after wiping.
        hull.apply_damage(100.0, &mut rng); // wipe to 0
        hull.restore(Console::Helm, 74.0);  // 74/100 = 0.74 < 0.75 → Damaged
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Damaged);
    }

    #[test]
    fn tier_is_disabled_below_disabled_threshold() {
        // Default disabled_threshold = 0.25. 24% of 100 → Disabled.
        let mut hull = ConsoleHull::from_config(&[(Console::Helm, 100.0)]);
        let mut rng = rand::rng();
        hull.apply_damage(100.0, &mut rng); // wipe to 0
        hull.restore(Console::Helm, 24.0);  // 24/100 = 0.24 < 0.25 → Disabled
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Disabled);
    }

    #[test]
    fn tier_is_destroyed_at_zero_hp() {
        let mut hull = ConsoleHull::from_config(&[(Console::Helm, 25.0)]);
        let mut rng = rand::rng();
        hull.apply_damage(100.0, &mut rng);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Destroyed);
    }

    #[test]
    fn tier_thresholds_are_configurable() {
        // Custom: damaged at 50%, disabled at 10%.
        let cfg = ConsoleTierConfig {
            damaged_threshold_pct: 0.50,
            disabled_threshold_pct: 0.10,
            debuff_magnitude: 0.15,
        };
        let mut hull = ConsoleHull::from_config_with_tiers(&[(Console::Helm, 100.0, cfg)]);
        let mut rng = rand::rng();

        // 60% → still Operational (above 50% threshold).
        hull.apply_damage(100.0, &mut rng);
        hull.restore(Console::Helm, 60.0);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Operational);

        // 40% → Damaged (below 50%, above 10%).
        hull.apply_damage(100.0, &mut rng);
        hull.restore(Console::Helm, 40.0);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Damaged);

        // 9% → Disabled (below 10%).
        hull.apply_damage(100.0, &mut rng);
        hull.restore(Console::Helm, 9.0);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Disabled);
    }

    #[test]
    fn tier_transitions_correctly_with_damage() {
        // Track the tier as the console takes progressive damage.
        let mut hull = ConsoleHull::from_config(&[(Console::Helm, 100.0)]);
        let mut rng = rand::rng();

        // Full HP → Operational.
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Operational);

        // 80% → still Operational (default damaged_threshold = 0.75).
        hull.apply_damage(100.0, &mut rng);
        hull.restore(Console::Helm, 80.0);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Operational);

        // 50% → Damaged.
        hull.apply_damage(100.0, &mut rng);
        hull.restore(Console::Helm, 50.0);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Damaged);

        // 10% → Disabled (below 0.25).
        hull.apply_damage(100.0, &mut rng);
        hull.restore(Console::Helm, 10.0);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Disabled);

        // 0% → Destroyed.
        hull.apply_damage(100.0, &mut rng);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Destroyed);

        // Repaired back to 50% → Damaged again (tier latches only at 0).
        hull.restore(Console::Helm, 50.0);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Damaged);

        // Fully repaired → Operational.
        hull.restore(Console::Helm, 100.0);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Operational);
    }

    // ── debuff_magnitude_for tests ────────────────────────────────────────────

    #[test]
    fn debuff_magnitude_for_operational_console_returns_zero() {
        // Full HP → Operational → no debuff.
        let hull = ConsoleHull::from_config(&[(Console::Helm, 100.0)]);
        assert!(
            (hull.debuff_magnitude_for(Console::Helm) - 0.0).abs() < 1e-6,
            "Operational console should have 0.0 debuff magnitude"
        );
    }

    #[test]
    fn debuff_magnitude_for_damaged_console_returns_config_value() {
        // 50% HP → Damaged tier → returns tier_config.debuff_magnitude (default 0.15).
        let mut hull = ConsoleHull::from_config(&[(Console::Helm, 100.0)]);
        let mut rng = rand::rng();
        hull.apply_damage(100.0, &mut rng);
        hull.restore(Console::Helm, 50.0); // 50% < 75% threshold → Damaged
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Damaged);
        let debuff = hull.debuff_magnitude_for(Console::Helm);
        assert!(
            (debuff - 0.15).abs() < 1e-6,
            "Damaged console should return default debuff_magnitude 0.15, got {debuff}"
        );
    }

    #[test]
    fn debuff_magnitude_for_damaged_console_respects_custom_config() {
        // Custom debuff_magnitude of 0.30 in tier config.
        let cfg = ConsoleTierConfig {
            damaged_threshold_pct: 0.75,
            disabled_threshold_pct: 0.25,
            debuff_magnitude: 0.30,
        };
        let mut hull = ConsoleHull::from_config_with_tiers(&[(Console::Helm, 100.0, cfg)]);
        let mut rng = rand::rng();
        hull.apply_damage(100.0, &mut rng);
        hull.restore(Console::Helm, 50.0); // 50% → Damaged
        let debuff = hull.debuff_magnitude_for(Console::Helm);
        assert!(
            (debuff - 0.30).abs() < 1e-6,
            "Damaged console should return custom debuff_magnitude 0.30, got {debuff}"
        );
    }

    #[test]
    fn debuff_magnitude_for_destroyed_console_returns_zero() {
        // 0 HP → Destroyed → no partial debuff (fully offline).
        let mut hull = ConsoleHull::from_config(&[(Console::Helm, 100.0)]);
        let mut rng = rand::rng();
        hull.apply_damage(100.0, &mut rng);
        assert_eq!(hull.tier_for(Console::Helm), DamageTier::Destroyed);
        let debuff = hull.debuff_magnitude_for(Console::Helm);
        assert!(
            (debuff - 0.0).abs() < 1e-6,
            "Destroyed console should have 0.0 debuff magnitude, got {debuff}"
        );
    }
}
