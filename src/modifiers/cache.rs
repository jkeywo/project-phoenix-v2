use crate::flag_kind::FlagKind;
pub use crate::messages::{ModifierSlot, ModifierSource};
use bevy::prelude::{Component, Resource};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ── Integer modifier system ───────────────────────────────────────────────────

/// Which integer attribute a modifier affects. Server-internal only; not in
/// `messages.rs` because integer modifier values are never sent to clients.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IntModifierSlot {
    /// Additional repair teams granted to a ship.
    RepairTeams,
}

impl IntModifierSlot {
    /// Total number of slots; must be updated when new variants are added.
    pub const COUNT: usize = 1;

    /// Maps each slot to a fixed array index.
    pub fn index(&self) -> usize {
        match self {
            IntModifierSlot::RepairTeams => 0,
        }
    }

    fn all() -> [IntModifierSlot; Self::COUNT] {
        [IntModifierSlot::RepairTeams]
    }
}

/// A single integer modifier entry.
///
/// The `(source, slot)` pair is the identity key. Applying the same pair twice
/// replaces the existing entry rather than stacking.
#[derive(Clone, Debug, PartialEq)]
pub struct IntModifier {
    pub source: ModifierSource,
    pub slot: IntModifierSlot,
    /// Additive bonus applied to the slot's running total.
    pub bonus: i32,
}

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

/// An event queued inside `ShipModifiers` when a modifier is added/updated or removed.
/// Drained by the simulation broadcast system to emit `OutboundMessage`s.
#[derive(Clone, Debug)]
pub enum ModifierEvent {
    Added {
        source: ModifierSource,
        slot: ModifierSlot,
        bonus: f32,
    },
    Removed {
        source: ModifierSource,
        slot: ModifierSlot,
    },
}

/// All active modifiers for a ship, plus an eagerly-maintained multiplier cache.
///
/// Identity: `(source, slot)` pair. Re-adding the same source+slot replaces the
/// previous entry. Different sources on the same slot stack additively.
///
/// Cache formula per float slot:
/// - `sum = Σ bonus` for all entries on that slot
/// - if `sum >= 0` → multiplier = `1.0 + sum`
/// - if `sum < 0`  → multiplier = `1.0 / (1.0 + |sum|)`
///
/// Cache formula per int slot: straight sum of all active bonuses.
///
/// Dual-derives `Resource` (legacy global fallback used by some tests and by
/// the coordinator until PR 7 completes) and `Component` (per-entity storage
/// on each ship — PR 6 migration, see PRD #597).
#[derive(Resource, Component, Clone, Debug)]
pub struct ShipModifiers {
    /// Sparse table: `(source, slot) → bonus`.
    table: HashMap<(ModifierSource, ModifierSlot), f32>,
    /// Pre-computed multipliers, indexed by `ModifierSlot::index()`.
    cache: [f32; ModifierSlot::COUNT],
    /// Pending broadcast events. Drained each frame by `broadcast_modifier_events`.
    pub pending_events: Vec<ModifierEvent>,
    /// Boolean flags keyed by `FlagKind`, each backed by a set of sources.
    /// A flag is set iff its source-set is non-empty.
    flags: HashMap<FlagKind, HashSet<ModifierSource>>,
    /// Sparse table for integer modifiers: `(source, slot) → bonus`.
    int_table: HashMap<(ModifierSource, IntModifierSlot), i32>,
    /// Pre-computed sums for integer slots, indexed by `IntModifierSlot::index()`.
    int_cache: [i32; IntModifierSlot::COUNT],
}

impl ShipModifiers {
    /// Creates an empty modifier set. All multipliers default to `1.0`; all
    /// integer sums default to `0`.
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
            cache: [1.0; ModifierSlot::COUNT],
            pending_events: Vec::new(),
            flags: HashMap::new(),
            int_table: HashMap::new(),
            int_cache: [0; IntModifierSlot::COUNT],
        }
    }

    /// Inserts or replaces the modifier for the given `(source, slot)` pair,
    /// then rebuilds the cache. Only queues a broadcast `ModifierEvent::Added`
    /// when the entry is new or its bonus actually changed — callers like
    /// `translate_power_modifiers` re-apply the current power level every
    /// simulation tick, and without this check that flooded every connected
    /// client with a `ModifierAdded` message per tick regardless of whether
    /// anything changed, starving the render loop (e.g. stalling the lobby's
    /// Ready/Leave buttons for a mid-game station claimant).
    pub fn add_or_update(&mut self, modifier: Modifier) {
        let key = (modifier.source.clone(), modifier.slot.clone());
        let unchanged = self.table.get(&key) == Some(&modifier.bonus);
        if !unchanged {
            self.pending_events.push(ModifierEvent::Added {
                source: modifier.source.clone(),
                slot: modifier.slot.clone(),
                bonus: modifier.bonus,
            });
        }
        self.table.insert(key, modifier.bonus);
        self.rebuild_cache();
    }

    /// Removes the modifier for the given `(source, slot)` pair (no-op if absent),
    /// then rebuilds the cache.
    pub fn remove(&mut self, source: &ModifierSource, slot: &ModifierSlot) {
        let key = (source.clone(), slot.clone());
        if self.table.remove(&key).is_some() {
            self.pending_events.push(ModifierEvent::Removed {
                source: source.clone(),
                slot: slot.clone(),
            });
        }
        self.rebuild_cache();
    }

    /// Returns the computed multiplier for `slot`.
    pub fn get(&self, slot: &ModifierSlot) -> f32 {
        self.cache[slot.index()]
    }

    /// Adds `source` to the set for `flag`. Idempotent — adding the same
    /// `(source, flag)` twice has no additional effect.
    pub fn add_flag(&mut self, source: ModifierSource, flag: FlagKind) {
        self.flags.entry(flag).or_default().insert(source);
    }

    /// Removes `source` from the set for `flag`. No-op if `source` was not
    /// present. If the last source is removed, the flag becomes unset.
    pub fn remove_flag(&mut self, source: ModifierSource, flag: FlagKind) {
        if let Some(sources) = self.flags.get_mut(&flag) {
            sources.remove(&source);
            if sources.is_empty() {
                self.flags.remove(&flag);
            }
        }
    }

    /// Returns `true` iff at least one source has set `flag`.
    pub fn has_flag(&self, flag: &FlagKind) -> bool {
        self.flags.contains_key(flag)
    }

    /// Returns all `FlagKind` values that are currently set (i.e. have at
    /// least one source).
    pub fn flags(&self) -> Vec<FlagKind> {
        self.flags.keys().cloned().collect()
    }

    /// Removes ALL modifiers and flags originating from `source`.
    /// Pushes a `ModifierEvent::Removed` for each modifier removed.
    /// Rebuilds the cache after removal.
    pub fn clear_source(&mut self, source: &ModifierSource) {
        let keys: Vec<(ModifierSource, ModifierSlot)> = self
            .table
            .keys()
            .filter(|(s, _)| s == source)
            .cloned()
            .collect();

        for (src, slot) in keys {
            self.table.remove(&(src.clone(), slot.clone()));
            self.pending_events
                .push(ModifierEvent::Removed { source: src, slot });
        }

        let flag_kinds: Vec<FlagKind> = self.flags.keys().cloned().collect();
        for flag in &flag_kinds {
            if let Some(sources) = self.flags.get_mut(flag) {
                sources.remove(source);
                if sources.is_empty() {
                    self.flags.remove(flag);
                }
            }
        }

        self.rebuild_cache();
    }

    // ── Integer modifier API ──────────────────────────────────────────────────

    /// Inserts or replaces the integer modifier for the given `(source, slot)`
    /// pair, then rebuilds the integer cache.
    pub fn add_or_update_int(&mut self, modifier: IntModifier) {
        let key = (modifier.source, modifier.slot);
        self.int_table.insert(key, modifier.bonus);
        self.rebuild_int_cache();
    }

    /// Removes the integer modifier for the given `(source, slot)` pair (no-op
    /// if absent), then rebuilds the integer cache.
    pub fn remove_int(&mut self, source: &ModifierSource, slot: &IntModifierSlot) {
        let key = (source.clone(), slot.clone());
        self.int_table.remove(&key);
        self.rebuild_int_cache();
    }

    /// Returns the computed sum for `slot` (straight sum of all active bonuses).
    pub fn get_int(&self, slot: &IntModifierSlot) -> i32 {
        self.int_cache[slot.index()]
    }

    fn rebuild_int_cache(&mut self) {
        for slot in IntModifierSlot::all() {
            let sum: i32 = self
                .int_table
                .iter()
                .filter(|((_, s), _)| s == &slot)
                .map(|(_, &bonus)| bonus)
                .sum();
            self.int_cache[slot.index()] = sum;
        }
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

impl ShipModifiers {
    /// Returns a pre-formatted text block summarising the current state of all
    /// three modifier systems (flags, float modifiers, integer modifiers).
    ///
    /// Output has three labelled sections — `[Flags]`, `[Float Modifiers]`,
    /// `[Int Modifiers]` — each listing active entries with their value and
    /// source(s), or `(none)` if the section is empty.
    pub fn format_debug(&self) -> String {
        let mut out = String::new();

        // ── Flags ────────────────────────────────────────────────────────────
        out.push_str("[Flags]\n");
        let flag_entries: Vec<(&FlagKind, &HashSet<ModifierSource>)> = {
            let mut v: Vec<_> = self.flags.iter().collect();
            v.sort_by_key(|(f, _)| format!("{f:?}"));
            v
        };
        if flag_entries.is_empty() {
            out.push_str("(none)\n");
        } else {
            for (flag, sources) in flag_entries {
                let mut srcs: Vec<String> = sources.iter().map(format_source).collect();
                srcs.sort();
                out.push_str(&format!("{:?}  ← {}\n", flag, srcs.join(", ")));
            }
        }

        out.push('\n');

        // ── Float Modifiers ──────────────────────────────────────────────────
        out.push_str("[Float Modifiers]\n");
        // Group by slot, collect active entries.
        let mut float_by_slot: Vec<(ModifierSlot, f32, Vec<String>)> = ModifierSlot::all()
            .iter()
            .filter_map(|slot| {
                let entries: Vec<(&(ModifierSource, ModifierSlot), &f32)> =
                    self.table.iter().filter(|((_, s), _)| s == slot).collect();
                if entries.is_empty() {
                    None
                } else {
                    let multiplier = self.get(slot);
                    let mut detail: Vec<String> = entries
                        .iter()
                        .map(|((src, _), bonus)| format!("{} ({:+.2})", format_source(src), bonus))
                        .collect();
                    detail.sort();
                    Some((slot.clone(), multiplier, detail))
                }
            })
            .collect();
        float_by_slot.sort_by_key(|(s, _, _)| format!("{s:?}"));
        if float_by_slot.is_empty() {
            out.push_str("(none)\n");
        } else {
            for (slot, mult, detail) in float_by_slot {
                out.push_str(&format!(
                    "{:?}  ×{:.2}  ← {}\n",
                    slot,
                    mult,
                    detail.join(", ")
                ));
            }
        }

        out.push('\n');

        // ── Int Modifiers ────────────────────────────────────────────────────
        out.push_str("[Int Modifiers]\n");
        let mut int_by_slot: Vec<(IntModifierSlot, i32, Vec<String>)> = IntModifierSlot::all()
            .iter()
            .filter_map(|slot| {
                let entries: Vec<(&(ModifierSource, IntModifierSlot), &i32)> = self
                    .int_table
                    .iter()
                    .filter(|((_, s), _)| s == slot)
                    .collect();
                if entries.is_empty() {
                    None
                } else {
                    let sum = self.get_int(slot);
                    let mut detail: Vec<String> = entries
                        .iter()
                        .map(|((src, _), bonus)| format!("{} ({:+})", format_source(src), bonus))
                        .collect();
                    detail.sort();
                    Some((slot.clone(), sum, detail))
                }
            })
            .collect();
        int_by_slot.sort_by_key(|(s, _, _)| format!("{s:?}"));
        if int_by_slot.is_empty() {
            out.push_str("(none)\n");
        } else {
            for (slot, sum, detail) in int_by_slot {
                out.push_str(&format!("{:?}  {:+}  ← {}\n", slot, sum, detail.join(", ")));
            }
        }

        out
    }
}

/// Formats a `ModifierSource` as a human-readable string for the debug overlay.
fn format_source(source: &ModifierSource) -> String {
    match source {
        ModifierSource::Console(c) => format!("Console({c:?})"),
        ModifierSource::ImpulseDrive => "ImpulseDrive".to_string(),
        ModifierSource::RegionEffect { uuid } => format!("Region({})", &uuid.to_string()[..8]),
        ModifierSource::World { id, tag } => format!("World({id}/{tag})"),
        ModifierSource::PowerGroup(g) => format!("PowerGroup({})", g.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::Console;

    fn ms(source: ModifierSource, slot: ModifierSlot, bonus: f32) -> Modifier {
        Modifier {
            source,
            slot,
            bonus,
        }
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
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    // ── 3. Single negative bonus (penalty) ───────────────────────────────────

    #[test]
    fn single_penalty_gives_one_over_one_plus_abs() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::HullDamageTaken,
            -0.5,
        ));
        let expected = 1.0 / 1.5;
        assert!((mods.get(&ModifierSlot::HullDamageTaken) - expected).abs() < 1e-6);
    }

    // ── 4. Multiple bonuses on the same slot stack additively ─────────────────

    #[test]
    fn multiple_sources_same_slot_stack_additively() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.3,
        ));
        mods.add_or_update(ms(
            ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(1),
            },
            ModifierSlot::MaxSpeed,
            0.2,
        ));
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    // ── 5. Mixed positive + negative bonuses on same slot ────────────────────

    #[test]
    fn mixed_bonuses_sum_before_formula() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            1.0,
        ));
        mods.add_or_update(ms(
            ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(2),
            },
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
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.1,
        ));
        // Only 0.1 should remain, not 0.6
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.1).abs() < 1e-6);
    }

    // ── 7. Remove restores to identity ───────────────────────────────────────

    #[test]
    fn remove_existing_modifier_restores_identity() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::RadarRange,
            0.5,
        ));
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
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            2.0,
        ));
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
            ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(3),
            },
            ModifierSlot::MaxSpeed,
            0.2,
        ));
        mods.add_or_update(ms(
            ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(4),
            },
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
            ModifierSource::Console(Console::Sensors),
            ModifierSlot::RadarRange,
            1.0,
        ));
        assert!((mods.get(&ModifierSlot::RadarRange) - 2.0).abs() < 1e-6);
    }

    // ── Flag API tests ─────────────────────────────────────────────────────

    use crate::flag_kind::FlagKind;

    #[test]
    fn single_source_adds_flag() {
        let mut mods = ShipModifiers::new();
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        assert!(mods.has_flag(&FlagKind::CommsJammed));
    }

    #[test]
    fn multiple_sources_or_aggregate() {
        let mut mods = ShipModifiers::new();
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        mods.add_flag(
            ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(1),
            },
            FlagKind::CommsJammed,
        );
        assert!(mods.has_flag(&FlagKind::CommsJammed));
    }

    #[test]
    fn removing_one_source_leaves_flag_set_when_multiple_sources() {
        let mut mods = ShipModifiers::new();
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        mods.add_flag(
            ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(1),
            },
            FlagKind::CommsJammed,
        );
        mods.remove_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        assert!(
            mods.has_flag(&FlagKind::CommsJammed),
            "flag should remain because 2nd source still exists"
        );
    }

    #[test]
    fn removing_last_source_clears_flag() {
        let mut mods = ShipModifiers::new();
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::SensorBlind);
        mods.remove_flag(ModifierSource::ImpulseDrive, FlagKind::SensorBlind);
        assert!(!mods.has_flag(&FlagKind::SensorBlind));
    }

    #[test]
    fn idempotent_add_does_not_duplicate() {
        let mut mods = ShipModifiers::new();
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        assert!(mods.has_flag(&FlagKind::CommsJammed));
    }

    #[test]
    fn removing_unknown_source_is_noop() {
        let mut mods = ShipModifiers::new();
        mods.remove_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        assert!(!mods.has_flag(&FlagKind::CommsJammed));
    }

    #[test]
    fn flag_storage_independent_from_numeric_modifiers() {
        let mut mods = ShipModifiers::new();
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        // Flag should still be set
        assert!(mods.has_flag(&FlagKind::CommsJammed));
        // Modifier value should be unaffected
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn flags_returns_all_set_flags() {
        let mut mods = ShipModifiers::new();
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        mods.add_flag(
            ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(1),
            },
            FlagKind::SensorBlind,
        );
        let result = mods.flags();
        assert!(result.contains(&FlagKind::CommsJammed));
        assert!(result.contains(&FlagKind::SensorBlind));
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn flags_empty_when_no_flags_set() {
        let mods = ShipModifiers::new();
        assert!(mods.flags().is_empty());
    }

    // ── clear_source tests ────────────────────────────────────────────────

    #[test]
    fn clear_source_removes_all_modifiers_and_flags_for_source() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::RadarRange,
            0.3,
        ));
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        mods.clear_source(&ModifierSource::ImpulseDrive);
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
        assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
        assert!(!mods.has_flag(&FlagKind::CommsJammed));
    }

    #[test]
    fn clear_source_does_not_affect_other_sources() {
        let mut mods = ShipModifiers::new();
        let region = ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(10),
        };
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        mods.add_or_update(ms(region.clone(), ModifierSlot::MaxSpeed, 0.3));
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        mods.add_flag(region.clone(), FlagKind::SensorBlind);
        mods.clear_source(&region);
        // ImpulseDrive modifiers and flags should survive
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
        assert!(mods.has_flag(&FlagKind::CommsJammed));
        // Region modifiers and flags should be gone
        assert!(!mods.has_flag(&FlagKind::SensorBlind));
    }

    #[test]
    fn clear_source_on_unknown_source_is_noop() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        mods.clear_source(&ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(99),
        });
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    // ── IntModifierSlot ───────────────────────────────────────────────────────

    #[test]
    fn int_modifier_slot_count_is_correct() {
        // COUNT must equal the number of variants; currently 1 (RepairTeams).
        assert_eq!(IntModifierSlot::COUNT, 1);
    }

    #[test]
    fn repair_teams_slot_index_is_zero() {
        assert_eq!(IntModifierSlot::RepairTeams.index(), 0);
    }

    // ── add_or_update_int / get_int ───────────────────────────────────────────

    #[test]
    fn get_int_returns_zero_with_no_modifiers() {
        let mods = ShipModifiers::new();
        assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 0);
    }

    #[test]
    fn add_or_update_int_accumulates_bonuses_from_distinct_sources() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::ImpulseDrive,
            slot: IntModifierSlot::RepairTeams,
            bonus: 2,
        });
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(1),
            },
            slot: IntModifierSlot::RepairTeams,
            bonus: 3,
        });
        assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 5);
    }

    #[test]
    fn same_source_slot_pair_replaces_rather_than_stacks() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::ImpulseDrive,
            slot: IntModifierSlot::RepairTeams,
            bonus: 10,
        });
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::ImpulseDrive,
            slot: IntModifierSlot::RepairTeams,
            bonus: 1,
        });
        // Only the latest bonus (1) should survive
        assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 1);
    }

    // ── remove_int ────────────────────────────────────────────────────────────

    #[test]
    fn remove_int_removes_correct_entry_and_updates_cache() {
        let mut mods = ShipModifiers::new();
        let region = ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(5),
        };
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::ImpulseDrive,
            slot: IntModifierSlot::RepairTeams,
            bonus: 2,
        });
        mods.add_or_update_int(IntModifier {
            source: region.clone(),
            slot: IntModifierSlot::RepairTeams,
            bonus: 3,
        });
        mods.remove_int(&region, &IntModifierSlot::RepairTeams);
        assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 2);
    }

    #[test]
    fn remove_int_unknown_entry_is_noop() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::ImpulseDrive,
            slot: IntModifierSlot::RepairTeams,
            bonus: 4,
        });
        mods.remove_int(
            &ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(99),
            },
            &IntModifierSlot::RepairTeams,
        );
        assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 4);
    }

    #[test]
    fn remove_int_all_sources_returns_zero() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::ImpulseDrive,
            slot: IntModifierSlot::RepairTeams,
            bonus: 7,
        });
        mods.remove_int(&ModifierSource::ImpulseDrive, &IntModifierSlot::RepairTeams);
        assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 0);
    }

    #[test]
    fn int_modifiers_are_independent_from_float_modifiers() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::ImpulseDrive,
            slot: IntModifierSlot::RepairTeams,
            bonus: 3,
        });
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 3);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn clear_source_on_region_exit_cleans_up_all_region_effects() {
        let mut mods = ShipModifiers::new();
        let region_uuid = uuid::Uuid::from_u128(42);
        let region = ModifierSource::RegionEffect { uuid: region_uuid };
        mods.add_or_update(ms(region.clone(), ModifierSlot::MaxSpeed, -0.2));
        mods.add_or_update(ms(region.clone(), ModifierSlot::PhaserDamage, 0.5));
        mods.add_flag(region.clone(), FlagKind::CommsJammed);
        mods.add_flag(region.clone(), FlagKind::SensorBlind);
        mods.clear_source(&region);
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
        assert_eq!(mods.get(&ModifierSlot::PhaserDamage), 1.0);
        assert!(!mods.has_flag(&FlagKind::CommsJammed));
        assert!(!mods.has_flag(&FlagKind::SensorBlind));
    }

    #[test]
    fn clear_source_pushes_removed_events() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::RadarRange,
            0.3,
        ));
        mods.pending_events.clear();
        mods.clear_source(&ModifierSource::ImpulseDrive);
        assert_eq!(mods.pending_events.len(), 2);
        assert!(mods
            .pending_events
            .iter()
            .all(|e| matches!(e, ModifierEvent::Removed { .. })));
    }

    // ── format_debug tests ────────────────────────────────────────────────

    #[test]
    fn format_debug_empty_shows_none_in_all_sections() {
        let mods = ShipModifiers::new();
        let s = mods.format_debug();
        assert!(s.contains("[Flags]"), "missing [Flags] header");
        assert!(
            s.contains("[Float Modifiers]"),
            "missing [Float Modifiers] header"
        );
        assert!(
            s.contains("[Int Modifiers]"),
            "missing [Int Modifiers] header"
        );
        // All three sections should show (none)
        assert_eq!(
            s.matches("(none)").count(),
            3,
            "expected (none) in all three sections"
        );
    }

    #[test]
    fn format_debug_shows_active_flag_with_source() {
        let mut mods = ShipModifiers::new();
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        let s = mods.format_debug();
        assert!(s.contains("CommsJammed"), "missing flag name");
        assert!(s.contains("ImpulseDrive"), "missing source name");
        // Only Float and Int sections should be (none)
        assert_eq!(
            s.matches("(none)").count(),
            2,
            "expected (none) for float and int sections only"
        );
    }

    #[test]
    fn format_debug_shows_float_modifier_with_multiplier_and_source() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        let s = mods.format_debug();
        assert!(s.contains("MaxSpeed"), "missing slot name");
        assert!(s.contains("×1.50"), "missing multiplier");
        assert!(s.contains("ImpulseDrive"), "missing source");
        assert!(s.contains("+0.50"), "missing bonus detail");
    }

    #[test]
    fn format_debug_shows_int_modifier_with_sum_and_source() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::ImpulseDrive,
            slot: IntModifierSlot::RepairTeams,
            bonus: 2,
        });
        let s = mods.format_debug();
        assert!(s.contains("RepairTeams"), "missing int slot name");
        assert!(s.contains("+2"), "missing sum");
        assert!(s.contains("ImpulseDrive"), "missing source");
    }

    #[test]
    fn format_debug_empty_float_and_int_slots_not_shown() {
        let mut mods = ShipModifiers::new();
        // Only a flag — no float or int modifiers
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::SensorBlind);
        let s = mods.format_debug();
        // Float and Int sections should both say (none)
        let lines: Vec<&str> = s.lines().collect();
        let float_idx = lines
            .iter()
            .position(|l| l.contains("[Float Modifiers]"))
            .unwrap();
        let int_idx = lines
            .iter()
            .position(|l| l.contains("[Int Modifiers]"))
            .unwrap();
        assert_eq!(
            lines[float_idx + 1],
            "(none)",
            "float section should be (none)"
        );
        assert_eq!(lines[int_idx + 1], "(none)", "int section should be (none)");
    }
}
