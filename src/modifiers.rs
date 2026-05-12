use std::collections::HashMap;
use serde::{Deserialize, Serialize};
#[cfg(feature = "server")]
use bevy::prelude::Resource;
pub use crate::messages::{ModifierSlot, ModifierSource};

impl ModifierSlot {
    pub const COUNT: usize = 6;

    /// Maps each slot to a fixed index for use in the cache array.
    pub fn index(&self) -> usize {
        match self {
            ModifierSlot::MaxSpeed => 0,
            ModifierSlot::MaxYawRate => 1,
            ModifierSlot::RadarRange => 2,
            ModifierSlot::PhaserDamage => 3,
            ModifierSlot::HullDamageTaken => 4,
            ModifierSlot::RepairRate => 5,
        }
    }

    fn all() -> [ModifierSlot; Self::COUNT] {
        [
            ModifierSlot::MaxSpeed,
            ModifierSlot::MaxYawRate,
            ModifierSlot::RadarRange,
            ModifierSlot::PhaserDamage,
            ModifierSlot::HullDamageTaken,
            ModifierSlot::RepairRate,
        ]
    }
}

/// A single modifier entry: which source, which slot, and the bonus magnitude.
///
/// Positive bonus: buff (e.g. `+0.5` on `MaxSpeed` → multiplier 1.5×).
/// Negative bonus: debuff (e.g. `-0.5` on `HullDamageTaken` → multiplier ≈ 0.67×).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Modifier {
    pub source: ModifierSource,
    pub slot: ModifierSlot,
    /// Additive bonus. Positive = buff, negative = debuff.
    pub bonus: f32,
}

/// All active modifiers for a ship, plus an eagerly-maintained multiplier cache.
///
/// Identity: `(source, slot)` pair. Re-adding the same source+slot replaces the
/// previous entry. Different sources on the same slot stack additively.
///
/// Cache formula per slot:
/// - `sum = Σ bonus` for all entries on that slot
/// - if `sum >= 0` → multiplier = `1.0 + sum`
/// - if `sum < 0`  → multiplier = `1.0 / (1.0 + |sum|)`
#[cfg_attr(feature = "server", derive(Resource))]
#[derive(Clone, Debug)]
pub struct ShipModifiers {
    /// Sparse table: `(source, slot) → bonus`.
    table: HashMap<(ModifierSource, ModifierSlot), f32>,
    /// Pre-computed multipliers, indexed by `ModifierSlot::index()`.
    cache: [f32; ModifierSlot::COUNT],
}

impl ShipModifiers {
    /// Creates an empty modifier set. All multipliers default to `1.0`.
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
            cache: [1.0; ModifierSlot::COUNT],
        }
    }

    /// Inserts or replaces the modifier for the given `(source, slot)` pair,
    /// then rebuilds the cache.
    pub fn add_or_update(&mut self, modifier: Modifier) {
        let key = (modifier.source, modifier.slot);
        self.table.insert(key, modifier.bonus);
        self.rebuild_cache();
    }

    /// Removes the modifier for the given `(source, slot)` pair (no-op if absent),
    /// then rebuilds the cache.
    pub fn remove(&mut self, source: &ModifierSource, slot: &ModifierSlot) {
        let key = (source.clone(), slot.clone());
        self.table.remove(&key);
        self.rebuild_cache();
    }

    /// Returns the computed multiplier for `slot`.
    pub fn get(&self, slot: &ModifierSlot) -> f32 {
        self.cache[slot.index()]
    }

    fn rebuild_cache(&mut self) {
        for slot in ModifierSlot::all() {
            let sum: f32 = self
                .table
                .iter()
                .filter(|((_, s), _)| s == &slot)
                .map(|(_, &bonus)| bonus)
                .sum();
            self.cache[slot.index()] = if sum >= 0.0 {
                1.0 + sum
            } else {
                1.0 / (1.0 + sum.abs())
            };
        }
    }
}

impl Default for ShipModifiers {
    fn default() -> Self {
        Self::new()
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::Console;

    fn ms(source: ModifierSource, slot: ModifierSlot, bonus: f32) -> Modifier {
        Modifier { source, slot, bonus }
    }

    // ── 1. Empty table → 1.0 for every slot ──────────────────────────────────

    #[test]
    fn empty_table_returns_identity_for_all_slots() {
        let mods = ShipModifiers::new();
        for slot in ModifierSlot::all() {
            assert_eq!(mods.get(&slot), 1.0, "expected 1.0 for {slot:?}");
        }
    }

    // ── 2. Single positive bonus ──────────────────────────────────────────────

    #[test]
    fn single_positive_bonus_gives_one_plus_bonus() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(ModifierSource::ImpulseDrive, ModifierSlot::MaxSpeed, 0.5));
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    // ── 3. Single negative bonus (penalty) ───────────────────────────────────

    #[test]
    fn single_penalty_gives_one_over_one_plus_abs() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(ModifierSource::ImpulseDrive, ModifierSlot::HullDamageTaken, -0.5));
        let expected = 1.0 / 1.5;
        assert!((mods.get(&ModifierSlot::HullDamageTaken) - expected).abs() < 1e-6);
    }

    // ── 4. Multiple bonuses on the same slot stack additively ─────────────────

    #[test]
    fn multiple_sources_same_slot_stack_additively() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(ModifierSource::ImpulseDrive, ModifierSlot::MaxSpeed, 0.3));
        mods.add_or_update(ms(
            ModifierSource::RegionEffect { region_id: "nebula".into() },
            ModifierSlot::MaxSpeed,
            0.2,
        ));
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    // ── 5. Mixed positive + negative bonuses on same slot ────────────────────

    #[test]
    fn mixed_bonuses_sum_before_formula() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(ModifierSource::ImpulseDrive, ModifierSlot::MaxSpeed, 1.0));
        mods.add_or_update(ms(
            ModifierSource::RegionEffect { region_id: "drag".into() },
            ModifierSlot::MaxSpeed,
            -0.5,
        ));
        // sum = 0.5 → multiplier = 1.5
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    // ── 6. Re-adding same source+slot replaces (not stacks) ──────────────────

    #[test]
    fn readding_same_source_slot_replaces() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(ModifierSource::ImpulseDrive, ModifierSlot::MaxSpeed, 0.5));
        mods.add_or_update(ms(ModifierSource::ImpulseDrive, ModifierSlot::MaxSpeed, 0.1));
        // Only 0.1 should remain, not 0.6
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.1).abs() < 1e-6);
    }

    // ── 7. Remove restores to identity ───────────────────────────────────────

    #[test]
    fn remove_existing_modifier_restores_identity() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(ModifierSource::ImpulseDrive, ModifierSlot::RadarRange, 0.5));
        mods.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::RadarRange);
        assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
    }

    // ── 8. Remove unknown entry is a no-op ───────────────────────────────────

    #[test]
    fn remove_unknown_is_noop() {
        let mut mods = ShipModifiers::new();
        // Should not panic
        mods.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
    }

    // ── 9. Modifiers on different slots don't bleed into each other ───────────

    #[test]
    fn slot_isolation() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(ModifierSource::ImpulseDrive, ModifierSlot::MaxSpeed, 2.0));
        // All other slots should still be 1.0
        assert_eq!(mods.get(&ModifierSlot::PhaserDamage), 1.0);
        assert_eq!(mods.get(&ModifierSlot::RepairRate), 1.0);
        assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
    }

    // ── 10. Region IDs stack as distinct sources ──────────────────────────────

    #[test]
    fn different_region_ids_stack_as_distinct_sources() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::RegionEffect { region_id: "alpha".into() },
            ModifierSlot::MaxSpeed,
            0.2,
        ));
        mods.add_or_update(ms(
            ModifierSource::RegionEffect { region_id: "beta".into() },
            ModifierSlot::MaxSpeed,
            0.3,
        ));
        // sum = 0.5 → 1.5
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    // ── 11. Console source stacks per-console ────────────────────────────────

    #[test]
    fn console_source_uses_console_variant() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::Console(Console::Science),
            ModifierSlot::RadarRange,
            1.0,
        ));
        assert!((mods.get(&ModifierSlot::RadarRange) - 2.0).abs() < 1e-6);
    }
}
