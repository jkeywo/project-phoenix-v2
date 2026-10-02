use super::*;

// These tests want *a* generator, not a particular sequence — the
// distribution they assert on holds for any of them. `damage.rs` is one
// of the crate's pure, Bevy-free modules (AGENTS.md #10); `sim_rng` is
// not (it imports `bevy::prelude::Resource` for the `SimRng` resource),
// so borrowing its `unseeded_test_rng` here coupled this module's tests
// to a Bevy-adjacent one. Building the `Pcg32` locally instead keeps that
// layering clean, and a literal (rather than OS-drawn) seed makes these
// fixtures themselves deterministic run to run.
fn test_rng() -> Pcg32 {
    Pcg32::seeded(1337, 0)
}

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

fn sid(s: &str) -> SystemId {
    SystemId(s.into())
}

// ── apply_hull_damage helper ────────────────────────────────────────────

fn single_console_hull(hp: f32) -> SystemHull {
    SystemHull::from_config(&[(sid("helm"), hp)])
}

#[test]
fn apply_hull_damage_zero_damage_no_change() {
    let mut hull = single_console_hull(100.0);
    let mut rng = test_rng();
    let (applied, _destroyed) = crate::ship::damage::apply_hull_damage(&mut hull, 0.0, &mut rng);
    assert_eq!(applied, 0.0);
    assert!((hull.total_current() - 100.0).abs() < 1e-6);
}

#[test]
fn apply_hull_damage_fractional_accumulates() {
    let mut hull = single_console_hull(100.0);
    let mut rng = test_rng();
    let (applied, _destroyed) = crate::ship::damage::apply_hull_damage(&mut hull, 3.5, &mut rng);
    assert!((applied - 3.5).abs() < 1e-6, "applied={}", applied);
    assert!((hull.total_current() - 96.5).abs() < 1e-6);
}

#[test]
fn apply_hull_damage_also_applies_to_hull() {
    let mut hull = single_console_hull(100.0);
    let mut rng = test_rng();
    apply_hull_damage(&mut hull, 10.0, &mut rng);
    assert!((hull.total_current() - 90.0).abs() < 1e-6);
}

#[test]
fn station_hull_can_be_initialised_with_custom_hp_and_absorbs_damage() {
    let mut station_hull = single_console_hull(80.0);
    let mut rng = test_rng();
    let (applied, _destroyed) = apply_hull_damage(&mut station_hull, 30.0, &mut rng);
    assert!((applied - 30.0).abs() < 1e-6, "applied={}", applied);
    assert!((station_hull.total_current() - 50.0).abs() < 1e-6);
}

#[test]
fn station_hull_reaches_zero_on_destruction() {
    let mut station_hull = single_console_hull(50.0);
    let mut rng = test_rng();
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
    let mut rng = test_rng();
    let (_applied, destroyed) = apply_hull_damage(&mut hull, 10.0, &mut rng);
    assert!(!destroyed, "ship should not be destroyed when HP remains");
}

#[test]
fn apply_hull_damage_returns_ship_destroyed_true_when_all_consoles_at_zero() {
    let mut hull = single_console_hull(20.0);
    let mut rng = test_rng();
    let (_applied, destroyed) = apply_hull_damage(&mut hull, 100.0, &mut rng);
    assert!(
        destroyed,
        "ship should be destroyed when all consoles reach 0"
    );
}

#[test]
fn apply_hull_damage_spillover_fires_destroyed_after_second_console_wiped() {
    let mut hull = SystemHull::from_config(&[(sid("helm"), 5.0), (sid("tactical"), 10.0)]);
    let mut rng = test_rng();
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
    let mut shields = crate::weapons::shield::ShieldSystem::default(); // 4 facings, 100 hp each
    let hull_damage = apply_damage_with_shields(20, 0.0, &mut shields);
    // Fore shield absorbs all 20; hull unchanged (no hull ref passed).
    assert_eq!(hull_damage, 0);
    assert_eq!(shields.facings[0].hp, 80);
}

#[test]
fn depleted_shield_passes_overflow_to_hull() {
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
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
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
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
    let mut shields = crate::weapons::shield::ShieldSystem::default();
    // Hit from the port side (bearing -π/2)
    let hull_damage = apply_damage_with_shields(10, -std::f32::consts::FRAC_PI_2, &mut shields);
    assert_eq!(hull_damage, 0);
    assert_eq!(shields.facings[1].hp, 90); // Port
    assert_eq!(shields.facings[0].hp, 100); // Fore untouched
}

// ── SystemHull ───────────────────────────────────────────────────────────

fn four_console_hull() -> SystemHull {
    SystemHull::from_config(&[
        (sid("helm"), 25.0),
        (sid("tactical"), 25.0),
        (sid("power"), 25.0),
        (sid("shields"), 25.0),
    ])
}

// Cycle 1: aggregates are correct at full HP
#[test]
fn system_hull_total_current_and_max_at_start() {
    let hull = four_console_hull();
    assert!(near(hull.total_current(), 100.0));
    assert!(near(hull.total_max(), 100.0));
}

// Cycle 2: not destroyed when HP remains
#[test]
fn system_hull_not_destroyed_when_hp_remains() {
    let hull = four_console_hull();
    assert!(!hull.is_destroyed());
}

// Cycle 3: is_destroyed only when all consoles at 0
#[test]
fn system_hull_is_destroyed_only_when_all_at_zero() {
    let mut hull = four_console_hull();
    let mut rng = test_rng();
    hull.apply_damage(1000.0, &mut rng); // wipe everything
    assert!(hull.is_destroyed());
}

// Cycle 4: apply_damage reduces total_current
#[test]
fn apply_damage_reduces_total_hp() {
    let mut hull = four_console_hull();
    let mut rng = test_rng();
    hull.apply_damage(10.0, &mut rng);
    assert!(near(hull.total_current(), 90.0));
}

// Cycle 5: damage never targets consoles at 0 HP (spillover)
#[test]
fn apply_damage_skips_depleted_consoles() {
    // Build hull with one console at very low HP so it depletes first.
    // Use a seeded RNG to control which console is chosen.
    let mut hull = SystemHull::from_config(&[(sid("helm"), 5.0), (sid("tactical"), 100.0)]);
    let mut rng = test_rng();
    // Apply 110 damage — should wipe both consoles (5 + 105 spill to Tactical)
    hull.apply_damage(110.0, &mut rng);
    assert!(hull.is_destroyed(), "all consoles should be at 0");
    assert!(near(hull.total_current(), 0.0));
}

// Cycle 6: restore heals only the specified console
#[test]
fn restore_heals_only_targeted_console() {
    let mut hull = four_console_hull();
    let mut rng = test_rng();
    hull.apply_damage(100.0, &mut rng); // wipe all
    hull.restore(&sid("helm"), 10.0);
    // Only Helm should have HP restored
    assert!(near(hull.current_for(&sid("helm")).unwrap(), 10.0));
    assert!(near(hull.current_for(&sid("tactical")).unwrap(), 0.0));
    assert!(near(hull.current_for(&sid("power")).unwrap(), 0.0));
    assert!(near(hull.current_for(&sid("shields")).unwrap(), 0.0));
}

/// Issue #1310: whole-hull healing is the mirror of whole-hull damage —
/// it fills every system that has room, in one deterministic walk, and
/// stops when there is nothing left to fill.
#[test]
fn restore_distributed_fills_the_whole_hull_and_stops_at_the_maxima() {
    let mut hull = four_console_hull();
    let mut rng = test_rng();
    hull.apply_damage(100.0, &mut rng); // wipe all four
    assert!(near(hull.total_current(), 0.0));
    assert!(near(hull.total_missing(), 100.0));

    let restored = hull.restore_distributed(60.0, &mut rng);
    assert!(near(restored, 60.0));
    assert!(near(hull.total_current(), 60.0));
    for (_, current, max) in hull.entries() {
        assert!(current <= max, "no system exceeds its own maximum");
    }

    // More than the headroom fills what is left and reports only that.
    let restored = hull.restore_distributed(500.0, &mut rng);
    assert!(near(restored, 40.0));
    assert!(near(hull.total_current(), hull.total_max()));
    assert!(near(hull.total_missing(), 0.0));

    // An undamaged hull absorbs nothing at all.
    assert!(near(hull.restore_distributed(10.0, &mut rng), 0.0));
}

/// Issue #1311: a restricted walk is invisible to every system outside its
/// allow-list — it contributes no weight, cannot be chosen, cannot be the
/// float-precision fallback and cannot absorb spill.
///
/// The amounts here deliberately OVERRUN the allow-list. A hit that fits
/// inside the scope would pass even if the filter applied only to the first
/// draw; only an overrun exercises the spill loop, which is the step that
/// would otherwise walk into a sibling.
#[test]
fn a_scoped_walk_never_leaves_its_allow_list_in_either_direction() {
    let allow = [sid("helm"), sid("tactical")];
    let mut rng = test_rng();

    let mut hull = four_console_hull();
    hull.apply_damage_within(Some(&allow), 500.0, &mut rng);
    assert!(near(hull.current_for(&sid("helm")).unwrap(), 0.0));
    assert!(near(hull.current_for(&sid("tactical")).unwrap(), 0.0));
    assert!(
        near(hull.current_for(&sid("power")).unwrap(), 25.0)
            && near(hull.current_for(&sid("shields")).unwrap(), 25.0),
        "an overrun spills only within the allow-list"
    );
    assert!(!hull.is_destroyed(), "half the hull is still alive");

    let restored = hull.restore_distributed_within(Some(&allow), 500.0, &mut rng);
    assert!(near(restored, 50.0), "healing mirrors the same restriction");
    assert!(near(hull.total_current(), 100.0));

    // Healing outside the allow-list cannot reach the damaged half either.
    hull.set_hp(&sid("power"), 0.0);
    assert!(near(
        hull.restore_distributed_within(Some(&allow), 25.0, &mut rng),
        0.0
    ));
    assert!(near(hull.current_for(&sid("power")).unwrap(), 0.0));

    // A scope of exactly one is the same rule with nowhere to spill.
    let mut hull = four_console_hull();
    hull.apply_damage_within(Some(&[sid("shields")]), 500.0, &mut rng);
    assert!(near(hull.total_current(), 75.0));
    assert!(near(hull.current_for(&sid("shields")).unwrap(), 0.0));

    // `None` IS the unrestricted walk: same seed, same draw, same result.
    let mut scoped = four_console_hull();
    let mut whole = four_console_hull();
    scoped.apply_damage_within(None, 40.0, &mut Pcg32::seeded(99, 0));
    whole.apply_damage(40.0, &mut Pcg32::seeded(99, 0));
    for id in ["helm", "tactical", "power", "shields"] {
        assert!(near(
            scoped.current_for(&sid(id)).unwrap(),
            whole.current_for(&sid(id)).unwrap()
        ));
    }
}

/// Scoped totals sum only the allow-list, which is what a scoped clamp,
/// overflow figure and lethality preview are all measured against.
#[test]
fn scoped_totals_sum_only_the_allow_list() {
    let mut hull = four_console_hull();
    hull.set_hp(&sid("helm"), 5.0);
    let allow = [sid("helm"), sid("tactical")];
    assert!(near(hull.total_current_within(Some(&allow)), 30.0));
    assert!(near(hull.total_max_within(Some(&allow)), 50.0));
    assert!(near(hull.total_current_within(None), hull.total_current()));
    assert!(near(hull.total_max_within(None), hull.total_max()));
    assert!(
        near(hull.total_current_within(Some(&[])), 0.0),
        "an empty allow-list names nothing, so it sums to nothing"
    );
}

/// A destroyed system has the largest headroom, so the mirror walk brings
/// it back — the whole-hull form of `restore`'s documented revival.
#[test]
fn restore_distributed_revives_a_destroyed_system() {
    let mut hull = four_console_hull();
    let mut rng = test_rng();
    hull.set_hp(&sid("helm"), 0.0);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Destroyed);
    hull.restore_distributed(25.0, &mut rng);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Operational);
}

// Cycle 7: restore is clamped to max HP
#[test]
fn restore_is_clamped_to_max() {
    let mut hull = four_console_hull();
    hull.restore(&sid("helm"), 50.0); // already at 25, restore 50 → capped at 25
    assert!(near(hull.current_for(&sid("helm")).unwrap(), 25.0));
    assert!(near(hull.total_current(), 100.0));
}

// Cycle 8: from_config with default ship values
#[test]
fn default_ship_config_four_consoles_at_25hp() {
    let hull = four_console_hull();
    assert!(near(hull.current_for(&sid("helm")).unwrap(), 25.0));
    assert!(near(hull.current_for(&sid("tactical")).unwrap(), 25.0));
    assert!(near(hull.current_for(&sid("power")).unwrap(), 25.0));
    assert!(near(hull.current_for(&sid("shields")).unwrap(), 25.0));
}

// Cycle 9: weighted selection
#[test]
fn weighted_selection_favours_higher_hp_console() {
    // Tactical has 99× more HP than Helm, so it should absorb ~99% of hits.
    let mut hull = SystemHull::from_config(&[(sid("helm"), 1.0), (sid("tactical"), 99.0)]);
    let mut rng = test_rng();
    let mut tactical_hits = 0u32;
    let trials = 10_000;
    for _ in 0..trials {
        let before_tactical = hull.current_for(&sid("tactical")).unwrap();
        hull.apply_damage(0.001, &mut rng); // tiny damage to record which was chosen
        let after_tactical = hull.current_for(&sid("tactical")).unwrap();
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
    hull.restore(&sid("navigation"), 10.0); // not in the map
    assert!(near(hull.total_current(), before));
}

// ── DamageTier tests ──────────────────────────────────────────────────────

#[test]
fn tier_is_operational_at_full_hp() {
    let hull = SystemHull::from_config(&[(sid("helm"), 25.0)]);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Operational);
}

#[test]
fn tier_is_damaged_below_damaged_threshold() {
    // Default damaged_threshold = 0.75. 74% of 100 → Damaged.
    let mut hull = SystemHull::from_config(&[(sid("helm"), 100.0)]);
    let mut rng = test_rng();
    // Directly set HP to 74 by restoring after wiping.
    hull.apply_damage(100.0, &mut rng); // wipe to 0
    hull.restore(&sid("helm"), 74.0); // 74/100 = 0.74 < 0.75 → Damaged
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Damaged);
}

#[test]
fn tier_is_disabled_below_disabled_threshold() {
    // Default disabled_threshold = 0.25. 24% of 100 → Disabled.
    let mut hull = SystemHull::from_config(&[(sid("helm"), 100.0)]);
    let mut rng = test_rng();
    hull.apply_damage(100.0, &mut rng); // wipe to 0
    hull.restore(&sid("helm"), 24.0); // 24/100 = 0.24 < 0.25 → Disabled
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Disabled);
}

#[test]
fn tier_is_destroyed_at_zero_hp() {
    let mut hull = SystemHull::from_config(&[(sid("helm"), 25.0)]);
    let mut rng = test_rng();
    hull.apply_damage(100.0, &mut rng);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Destroyed);
}

#[test]
fn tier_thresholds_are_configurable() {
    // Custom: damaged at 50%, disabled at 10%.
    let cfg = ConsoleTierConfig {
        damaged_threshold_pct: 0.50,
        disabled_threshold_pct: 0.10,
        debuff_magnitude: 0.15,
    };
    let mut hull = SystemHull::from_config_with_tiers(&[(sid("helm"), 100.0, cfg)]);
    let mut rng = test_rng();

    // 60% → still Operational (above 50% threshold).
    hull.apply_damage(100.0, &mut rng);
    hull.restore(&sid("helm"), 60.0);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Operational);

    // 40% → Damaged (below 50%, above 10%).
    hull.apply_damage(100.0, &mut rng);
    hull.restore(&sid("helm"), 40.0);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Damaged);

    // 9% → Disabled (below 10%).
    hull.apply_damage(100.0, &mut rng);
    hull.restore(&sid("helm"), 9.0);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Disabled);
}

#[test]
fn tier_transitions_correctly_with_damage() {
    // Track the tier as the console takes progressive damage.
    let mut hull = SystemHull::from_config(&[(sid("helm"), 100.0)]);
    let mut rng = test_rng();

    // Full HP → Operational.
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Operational);

    // 80% → still Operational (default damaged_threshold = 0.75).
    hull.apply_damage(100.0, &mut rng);
    hull.restore(&sid("helm"), 80.0);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Operational);

    // 50% → Damaged.
    hull.apply_damage(100.0, &mut rng);
    hull.restore(&sid("helm"), 50.0);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Damaged);

    // 10% → Disabled (below 0.25).
    hull.apply_damage(100.0, &mut rng);
    hull.restore(&sid("helm"), 10.0);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Disabled);

    // 0% → Destroyed.
    hull.apply_damage(100.0, &mut rng);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Destroyed);

    // Repaired back to 50% → Damaged again (tier latches only at 0).
    hull.restore(&sid("helm"), 50.0);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Damaged);

    // Fully repaired → Operational.
    hull.restore(&sid("helm"), 100.0);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Operational);
}

// ── debuff_magnitude_for tests ────────────────────────────────────────────

#[test]
fn debuff_magnitude_for_operational_console_returns_zero() {
    // Full HP → Operational → no debuff.
    let hull = SystemHull::from_config(&[(sid("helm"), 100.0)]);
    assert!(
        (hull.debuff_magnitude_for(&sid("helm")) - 0.0).abs() < 1e-6,
        "Operational console should have 0.0 debuff magnitude"
    );
}

#[test]
fn debuff_magnitude_for_damaged_console_returns_config_value() {
    // 50% HP → Damaged tier → returns tier_config.debuff_magnitude (default 0.15).
    let mut hull = SystemHull::from_config(&[(sid("helm"), 100.0)]);
    let mut rng = test_rng();
    hull.apply_damage(100.0, &mut rng);
    hull.restore(&sid("helm"), 50.0); // 50% < 75% threshold → Damaged
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Damaged);
    let debuff = hull.debuff_magnitude_for(&sid("helm"));
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
    let mut hull = SystemHull::from_config_with_tiers(&[(sid("helm"), 100.0, cfg)]);
    let mut rng = test_rng();
    hull.apply_damage(100.0, &mut rng);
    hull.restore(&sid("helm"), 50.0); // 50% → Damaged
    let debuff = hull.debuff_magnitude_for(&sid("helm"));
    assert!(
        (debuff - 0.30).abs() < 1e-6,
        "Damaged console should return custom debuff_magnitude 0.30, got {debuff}"
    );
}

#[test]
fn debuff_magnitude_for_destroyed_console_returns_zero() {
    // 0 HP → Destroyed → no partial debuff (fully offline).
    let mut hull = SystemHull::from_config(&[(sid("helm"), 100.0)]);
    let mut rng = test_rng();
    hull.apply_damage(100.0, &mut rng);
    assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Destroyed);
    let debuff = hull.debuff_magnitude_for(&sid("helm"));
    assert!(
        (debuff - 0.0).abs() < 1e-6,
        "Destroyed console should have 0.0 debuff magnitude, got {debuff}"
    );
}

// ── ShipArcHull tests (issue #514) ────────────────────────────────────────

fn four_arc_hull() -> ShipArcHull {
    let tc = ConsoleTierConfig::default();
    ShipArcHull::from_entries(vec![
        (
            "fore".into(),
            ArcHullEntry {
                current: 6.0,
                max: 6.0,
                tier_config: tc,
            },
        ),
        (
            "port".into(),
            ArcHullEntry {
                current: 6.0,
                max: 6.0,
                tier_config: tc,
            },
        ),
        (
            "aft".into(),
            ArcHullEntry {
                current: 6.0,
                max: 6.0,
                tier_config: tc,
            },
        ),
        (
            "starboard".into(),
            ArcHullEntry {
                current: 7.0,
                max: 7.0,
                tier_config: tc,
            },
        ),
    ])
}

#[test]
fn arc_hull_starts_at_full_and_reports_operational() {
    let hull = four_arc_hull();
    assert_eq!(hull.tier_for("fore"), DamageTier::Operational);
    assert_eq!(hull.tier_for("port"), DamageTier::Operational);
    assert_eq!(hull.tier_for("aft"), DamageTier::Operational);
    assert_eq!(hull.tier_for("starboard"), DamageTier::Operational);
}

#[test]
fn arc_hull_apply_damage_reduces_total_hp() {
    let mut hull = four_arc_hull();
    let mut rng = test_rng();
    let before: f32 = hull.iter().map(|(_, e)| e.current).sum();
    hull.apply_damage(10.0, &mut rng);
    let after: f32 = hull.iter().map(|(_, e)| e.current).sum();
    assert!(
        (before - after - 10.0).abs() < 1e-3,
        "10 hp should have been absorbed"
    );
}

#[test]
fn arc_hull_tier_transitions_correctly_with_damage() {
    let mut hull = ShipArcHull::from_entries(vec![(
        "fore".into(),
        ArcHullEntry {
            current: 100.0,
            max: 100.0,
            tier_config: ConsoleTierConfig::default(),
        },
    )]);
    assert_eq!(hull.tier_for("fore"), DamageTier::Operational);

    hull.set_hp("fore", 60.0); // 0.6 < 0.75 → Damaged
    assert_eq!(hull.tier_for("fore"), DamageTier::Damaged);

    hull.set_hp("fore", 10.0); // 0.10 < 0.25 → Disabled
    assert_eq!(hull.tier_for("fore"), DamageTier::Disabled);

    hull.set_hp("fore", 0.0); // 0 → Destroyed
    assert_eq!(hull.tier_for("fore"), DamageTier::Destroyed);
}

#[test]
fn arc_hull_tier_for_unknown_arc_is_operational() {
    let hull = four_arc_hull();
    assert_eq!(hull.tier_for("nonexistent"), DamageTier::Operational);
}

#[test]
fn arc_hull_restore_is_clamped_to_max() {
    let mut hull = four_arc_hull();
    hull.set_hp("fore", 3.0);
    hull.restore("fore", 100.0);
    assert_eq!(hull.get("fore").unwrap().current, 6.0);
}

#[test]
fn arc_hull_iter_preserves_toml_order() {
    let hull = four_arc_hull();
    let ids: Vec<&str> = hull.iter().map(|(id, _)| id).collect();
    assert_eq!(ids, vec!["fore", "port", "aft", "starboard"]);
}

#[test]
fn arc_hull_apply_damage_favours_higher_hp_arc() {
    // Fore has 1 hp, aft has 99 hp — aft should absorb most tiny hits.
    let tc = ConsoleTierConfig::default();
    let mut hull = ShipArcHull::from_entries(vec![
        (
            "fore".into(),
            ArcHullEntry {
                current: 1.0,
                max: 1.0,
                tier_config: tc,
            },
        ),
        (
            "aft".into(),
            ArcHullEntry {
                current: 99.0,
                max: 99.0,
                tier_config: tc,
            },
        ),
    ]);
    let mut rng = test_rng();
    let mut aft_hits = 0u32;
    let trials = 10_000;
    for _ in 0..trials {
        let before_aft = hull.get("aft").unwrap().current;
        hull.apply_damage(0.001, &mut rng);
        let after_aft = hull.get("aft").unwrap().current;
        if after_aft < before_aft {
            aft_hits += 1;
        }
    }
    let fraction = aft_hits as f32 / trials as f32;
    assert!(
        fraction > 0.90,
        "Aft (99 hp) should absorb >90% of hits, got {:.1}%",
        fraction * 100.0
    );
}
