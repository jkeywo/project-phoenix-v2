use crate::core::messages::FlagKind;
pub use crate::core::messages::{ModifierSlot, ModifierSource};
use bevy::prelude::Component;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

// ── Integer modifier system ───────────────────────────────────────────────────

/// Which integer attribute a modifier affects. Server-internal only; not in
/// `messages.rs` because integer modifier values are never sent to clients.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    pub const COUNT: usize = 9;

    /// Maps each slot to a fixed index for use in the cache array.
    pub fn index(&self) -> usize {
        match self {
            ModifierSlot::MaxSpeed => 0,
            ModifierSlot::MaxYawRate => 1,
            ModifierSlot::RadarRange => 2,
            ModifierSlot::PhaserDamage => 3,
            ModifierSlot::HullDamageTaken => 4,
            ModifierSlot::RepairRate => 5,
            ModifierSlot::HelmRadarRange => 6,
            ModifierSlot::SensorRadarRange => 7,
            ModifierSlot::ShieldRegen => 8,
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
            ModifierSlot::HelmRadarRange,
            ModifierSlot::SensorRadarRange,
            ModifierSlot::ShieldRegen,
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
/// Per-entity `Component` storage on each ship (PR 6 migration, PRD #597;
/// the legacy `Resource` fallback was removed in issue #606). Every ship —
/// player and NPC — carries its own instance.
#[derive(Component, Clone, Debug)]
pub struct ShipModifiers {
    /// Sparse table: `(source, slot) → bonus`.
    ///
    /// A `BTreeMap`, and this is load-bearing rather than a taste call
    /// (issue #965). [`Self::rebuild_cache`] walks this table adding `f32`
    /// bonuses. IEEE-754 `f32` addition is commutative — only associativity
    /// fails — and `0.0 + b` is exact, so a slot with exactly two producers
    /// computes `(0.0 + b1) + b2` and `(0.0 + b2) + b1` to the same bits
    /// regardless of which one the walk visits first. Divergence needs
    /// three or more producers stacked on one slot, at which point how the
    /// running sum parenthesises depends on the order they arrived — and
    /// under a `HashMap`, that order came from `RandomState`, whose key is
    /// drawn per process (and in fact per map).
    ///
    /// The named slots are not equally exposed. `HullDamageTaken` has no
    /// code producer at all today. `RadarRange` has exactly one baseline
    /// producer — `apply_radar_damage_modifiers`'s per-slot `SystemDamage`
    /// entry — plus one more for every `RadarDampening` region a ship is
    /// standing in. Only `MaxSpeed` reaches three producers in ordinary
    /// play: helm power, the impulse drive, and a region's `SlowZone`
    /// thrust modifier all write it, so the failure needs a world that
    /// authors a slow zone (or a scripted `apply_modifier` trigger stacking
    /// a third source onto some slot) — a scenario with neither never
    /// exercised this defect, however its table happened to hash. Once a
    /// slot's producers do stack three deep, a ULP does not stay a ULP: it
    /// steers a helm a hair differently, which lands a shot differently,
    /// which draws the seeded RNG a different number of times. Ordering by
    /// key makes the walk a property of WHICH modifiers are held, never of
    /// where they hashed.
    ///
    /// The same ordering is what makes [`Self::clear_source`] queue its
    /// `ModifierEvent::Removed`s in a fixed sequence, and those become
    /// outbound messages.
    table: BTreeMap<(ModifierSource, ModifierSlot), f32>,
    /// Pre-computed multipliers, indexed by `ModifierSlot::index()`.
    cache: [f32; ModifierSlot::COUNT],
    /// Pending broadcast events. Drained each frame by `broadcast_modifier_events`.
    pub pending_events: Vec<ModifierEvent>,
    /// Boolean flags keyed by `FlagKind`, each backed by a set of sources.
    /// A flag is set iff its source-set is non-empty.
    ///
    /// Ordered for the same reason as `table`, though the stake is smaller:
    /// the aggregation here is a non-empty check rather than a sum, so no
    /// arithmetic depends on it, but [`Self::flags`] hands the key set out as
    /// a `Vec` and a caller that put that on the wire would inherit whatever
    /// order the map felt like. Ordering it costs nothing at these sizes and
    /// removes the trap.
    flags: BTreeMap<FlagKind, BTreeSet<ModifierSource>>,
    /// Sparse table for integer modifiers: `(source, slot) → bonus`.
    ///
    /// A `HashMap` deliberately, unlike `table` above. `i32` addition IS
    /// associative and exact, so [`Self::rebuild_int_cache`]'s sum is the same
    /// number in any order, and nothing else iterates this table into an
    /// order-sensitive output — `format_debug` sorts its own rendering. That
    /// leaves point lookups, which is what a hash map is for.
    int_table: HashMap<(ModifierSource, IntModifierSlot), i32>,
    /// Pre-computed sums for integer slots, indexed by `IntModifierSlot::index()`.
    int_cache: [i32; IntModifierSlot::COUNT],
}

impl ShipModifiers {
    /// Creates an empty modifier set. All multipliers default to `1.0`; all
    /// integer sums default to `0`.
    pub fn new() -> Self {
        Self {
            table: BTreeMap::new(),
            cache: [1.0; ModifierSlot::COUNT],
            pending_events: Vec::new(),
            flags: BTreeMap::new(),
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

    /// Recomputes every float slot's multiplier from `table`.
    ///
    /// One ordered walk, accumulating into a per-slot array, rather than the
    /// nine filtered walks this used to be. Each slot's bonuses are therefore
    /// still added in exactly the order the table yields them — the same
    /// sequence the old per-slot filter saw — so this is not a change of
    /// arithmetic, only of how many times the table is traversed. That matters
    /// because this runs on a hot path: `apply_power_modifiers` re-applies
    /// every power group's bonus each fixed tick, so `add_or_update` and hence
    /// this rebuild fire several times per ship per tick. Nine passes over the
    /// whole table became one, which more than pays for the ordered map's
    /// `O(log n)` lookups (n is a handful of entries per ship — power groups,
    /// the impulse drive, damaged systems, and whichever regions the ship is
    /// standing in — so the whole table lives inside a single B-tree node and
    /// the walk is a linear scan of contiguous memory).
    fn rebuild_cache(&mut self) {
        let mut sums = [0.0_f32; ModifierSlot::COUNT];
        for ((_, slot), bonus) in self.table.iter() {
            sums[slot.index()] += *bonus;
        }
        for slot in ModifierSlot::all() {
            let sum = sums[slot.index()];
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
    /// Projects the current state of all three modifier systems (flags, float
    /// modifiers, integer modifiers) into the structured debug payload the
    /// observability pipeline carries (issue #1150, PRD #1144).
    ///
    /// Replaces the pre-formatted `format_debug` text stream the legacy modifier
    /// overlay emitted: the three sections are the same, but each is now a list
    /// of typed entries the dock renders rather than a block of text. Every
    /// section is sorted by name and each entry's contributions by rendered
    /// source, so two hosts folding the same state serialise byte-identical JSON
    /// (payload convention 4). Owns the private-field access the projection
    /// needs, exactly as `format_debug` did.
    pub fn debug_payload(&self) -> crate::debug::payload::ModifierDebugPayload {
        use crate::debug::payload::{
            FloatContribution, FloatModifierEntry, IntContribution, IntModifierEntry,
            ModifierDebugPayload, ModifierFlagEntry, DEBUG_SCHEMA_VERSION,
        };

        // Flags: sorted by flag name, each flag's sources sorted.
        let mut flags: Vec<ModifierFlagEntry> = self
            .flags
            .iter()
            .map(|(flag, sources)| {
                let mut srcs: Vec<String> = sources.iter().map(format_source).collect();
                srcs.sort();
                ModifierFlagEntry {
                    flag: format!("{flag:?}"),
                    sources: srcs,
                }
            })
            .collect();
        flags.sort_by(|a, b| a.flag.cmp(&b.flag));

        // Float modifiers: one entry per non-empty slot, sorted by slot name.
        let mut float_modifiers: Vec<FloatModifierEntry> = ModifierSlot::all()
            .iter()
            .filter_map(|slot| {
                let mut contributions: Vec<FloatContribution> = self
                    .table
                    .iter()
                    .filter(|((_, s), _)| s == slot)
                    .map(|((src, _), bonus)| FloatContribution {
                        source: format_source(src),
                        bonus: *bonus,
                    })
                    .collect();
                if contributions.is_empty() {
                    return None;
                }
                contributions.sort_by(|a, b| a.source.cmp(&b.source));
                Some(FloatModifierEntry {
                    slot: format!("{slot:?}"),
                    multiplier: self.get(slot),
                    contributions,
                })
            })
            .collect();
        float_modifiers.sort_by(|a, b| a.slot.cmp(&b.slot));

        // Integer modifiers: one entry per non-empty slot, sorted by slot name.
        let mut int_modifiers: Vec<IntModifierEntry> = IntModifierSlot::all()
            .iter()
            .filter_map(|slot| {
                let mut contributions: Vec<IntContribution> = self
                    .int_table
                    .iter()
                    .filter(|((_, s), _)| s == slot)
                    .map(|((src, _), bonus)| IntContribution {
                        source: format_source(src),
                        bonus: *bonus,
                    })
                    .collect();
                if contributions.is_empty() {
                    return None;
                }
                contributions.sort_by(|a, b| a.source.cmp(&b.source));
                Some(IntModifierEntry {
                    slot: format!("{slot:?}"),
                    sum: self.get_int(slot),
                    contributions,
                })
            })
            .collect();
        int_modifiers.sort_by(|a, b| a.slot.cmp(&b.slot));

        ModifierDebugPayload {
            schema_version: DEBUG_SCHEMA_VERSION,
            flags,
            float_modifiers,
            int_modifiers,
        }
    }
}

/// Formats a `ModifierSource` as a human-readable string for the debug overlay.
fn format_source(source: &ModifierSource) -> String {
    match source {
        ModifierSource::ImpulseDrive => "ImpulseDrive".to_string(),
        ModifierSource::RegionEffect { uuid } => format!("Region({})", &uuid.to_string()[..8]),
        ModifierSource::World { id, tag } => format!("World({id}/{tag})"),
        ModifierSource::PowerGroup(g) => format!("PowerGroup({})", g.0),
        ModifierSource::SystemDamage(sid) => format!("SystemDamage({})", sid.0),
        ModifierSource::TractorLoad => "TractorLoad".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── 11. PowerGroup source stacks per-group ───────────────────────────────

    #[test]
    fn power_group_source_uses_group_id() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::PowerGroup(crate::core::messages::PowerGroupId("sensors".into())),
            ModifierSlot::RadarRange,
            1.0,
        ));
        assert!((mods.get(&ModifierSlot::RadarRange) - 2.0).abs() < 1e-6);
    }

    // ── Determinism guard (issue #965) ────────────────────────────────────

    /// Number of independent `ShipModifiers` instances each producer set is
    /// summed by. Every one draws its own `RandomState`, so under a hashed
    /// table these are 24 different walks of the same keys.
    const GUARD_INSTANCES: usize = 24;
    /// Number of distinct producer sets the guard tries. One set is not
    /// enough: whether a given set's ULP disagreement survives the
    /// `1.0 + sum` rounding into the published multiplier depends on where
    /// the sum lands in its binade, and not every set shows it.
    /// Re-measured directly against the pre-fix code (swap `table` back to a
    /// `HashMap`, rerun the two guard tests below, restore the `BTreeMap`):
    /// `same_producers_publish_identical_bits_in_every_instance` (the
    /// `1.0 + sum` branch) saw 42 of these 48 sets disagree, and
    /// `negative_sum_slots_publish_identical_bits_in_every_instance` (the
    /// `1.0 / (1.0 + |sum|)` branch) saw 36 of 48. Both figures move a
    /// little from run to run — the split depends on the process's random
    /// hash seed, same as the bug itself — but in every run the large
    /// majority of sets disagree, which is why 48 sets is enough to make the
    /// guard reliable without being slow.
    const GUARD_SETS: usize = 48;

    /// Bonus for producer `j` of set `set_idx` — a spread of ordinary modifier
    /// magnitudes in `[-0.5, 0.8)`, none of them exactly representable in
    /// binary (the `/101` sees to that), so their partial sums round.
    /// Arithmetic rather than a literal table so the guard covers a family of
    /// value shapes instead of one lucky tuple.
    fn guard_bonus(set_idx: usize, j: usize) -> f32 {
        (((set_idx * 8 + j) * 37 % 101) as f32 / 101.0) * 1.3 - 0.5
    }

    /// Every `ShipModifiers` holding the SAME producers must publish the SAME
    /// BITS for a slot, whatever order those producers arrived in and whichever
    /// instance holds them.
    ///
    /// This is the standing guard against unordered float accumulation coming
    /// back into the modifier cache (issue #965). It deliberately is not "the
    /// multiplier equals a constant": the defect only shows under a *different
    /// iteration order*, and any one instance's order is fixed, so a
    /// single-instance assertion would have passed throughout the bug's life.
    ///
    /// It gets those different orders honestly. `HashMap::new()` draws a fresh
    /// `RandomState` per instance — std seeds a thread-local key once and bumps
    /// it on every construction — so a hashed table walks the same eight keys in
    /// a different order in each of these instances. That is the same class of
    /// disagreement two *processes* see from the per-process seed, which is the
    /// one this test cannot itself create and the one that was diverging seeded
    /// runs. Insertion order is rotated too, so a table that ordered by
    /// insertion rather than by key would also be caught.
    ///
    /// This guard exists alongside `tests/rng_determinism.rs`'s
    /// `two_runs_with_the_same_seed_produce_byte_identical_reports` — not
    /// because that integration guard is structurally unable to see this
    /// class of bug. It is not: `HashMap::new()` reseeds per MAP, not only
    /// per process, so that guard's two sequential app builds already
    /// construct their `ShipModifiers` tables from fresh, independently
    /// seeded `RandomState`s and are just as capable of diverging. It has
    /// stayed green through this defect's whole life because its world,
    /// `rng_coverage.toml`, declares exactly one region and that region's
    /// only effect is a bare `damage_zone`, which `apply_region_effects`
    /// does not turn into a modifier at all — so no slot in that run ever
    /// picked up the three-or-more producers this defect needs, regardless
    /// of table order. This unit guard earns its place for a different
    /// reason: it is fast, it is targeted at the one function that matters,
    /// and it names the invariant directly instead of hoping a full
    /// simulation's damage numbers happen to move.
    ///
    /// Comparing `to_bits()` rather than an epsilon is the point: a single ULP
    /// is the whole issue, because a ULP compounds chaotically across a 600 s
    /// simulation.
    #[test]
    fn same_producers_publish_identical_bits_in_every_instance() {
        let mut split_sets: Vec<(usize, Vec<f32>)> = Vec::new();

        for set_idx in 0..GUARD_SETS {
            let mut published: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
            for rotation in 0..GUARD_INSTANCES {
                let mut mods = ShipModifiers::new();
                for k in 0..8 {
                    let j = (k + rotation) % 8;
                    mods.add_or_update(ms(
                        ModifierSource::RegionEffect {
                            uuid: uuid::Uuid::from_u128(j as u128 + 1),
                        },
                        ModifierSlot::MaxSpeed,
                        guard_bonus(set_idx, j),
                    ));
                }
                published.insert(mods.get(&ModifierSlot::MaxSpeed).to_bits());
            }
            if published.len() > 1 {
                split_sets.push((
                    set_idx,
                    published.iter().map(|b| f32::from_bits(*b)).collect(),
                ));
            }
        }

        assert!(
            split_sets.is_empty(),
            "{} of {GUARD_SETS} producer sets published more than one multiplier \
             across {GUARD_INSTANCES} instances holding identical modifiers — the \
             modifier cache is accumulating f32 over an unordered collection again, \
             so two processes running the same seed will diverge. Offenders: {:?}",
            split_sets.len(),
            split_sets
        );
    }

    /// The same guard for the reciprocal branch of the cache formula: a slot
    /// whose producers sum negative goes through `1.0 / (1.0 + |sum|)`, a
    /// different rounding path from `1.0 + sum`, and `HullDamageTaken` is one
    /// of the real slots that lands there.
    #[test]
    fn negative_sum_slots_publish_identical_bits_in_every_instance() {
        let mut split = 0usize;
        for set_idx in 0..GUARD_SETS {
            let mut published: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
            for rotation in 0..GUARD_INSTANCES {
                let mut mods = ShipModifiers::new();
                for k in 0..8 {
                    let j = (k + rotation) % 8;
                    mods.add_or_update(ms(
                        ModifierSource::RegionEffect {
                            uuid: uuid::Uuid::from_u128(j as u128 + 1),
                        },
                        ModifierSlot::HullDamageTaken,
                        // Shifted negative so every set sums below zero.
                        guard_bonus(set_idx, j) - 0.35,
                    ));
                }
                published.insert(mods.get(&ModifierSlot::HullDamageTaken).to_bits());
            }
            if published.len() > 1 {
                split += 1;
            }
        }
        assert_eq!(
            split, 0,
            "{split} of {GUARD_SETS} negative-sum producer sets published more than \
             one multiplier across instances holding identical modifiers"
        );
    }

    /// `clear_source` walks the table to decide which `ModifierEvent::Removed`
    /// to queue, and those events reach the wire. The sequence must not depend
    /// on which instance queued them.
    #[test]
    fn clear_source_queues_removals_in_the_same_order_in_every_instance() {
        let slots = ModifierSlot::all();
        let mut sequences: std::collections::BTreeSet<Vec<String>> = Default::default();
        for _ in 0..GUARD_INSTANCES {
            let mut mods = ShipModifiers::new();
            for (i, slot) in slots.iter().enumerate() {
                mods.add_or_update(ms(
                    ModifierSource::ImpulseDrive,
                    slot.clone(),
                    0.1 * (i as f32 + 1.0),
                ));
            }
            mods.pending_events.clear();
            mods.clear_source(&ModifierSource::ImpulseDrive);
            sequences.insert(
                mods.pending_events
                    .iter()
                    .map(|e| format!("{e:?}"))
                    .collect(),
            );
        }
        assert_eq!(
            sequences.len(),
            1,
            "clear_source queued its removals in {} different orders — that \
             sequence becomes an outbound message stream, so two processes \
             running the same seed emit different bytes",
            sequences.len()
        );
    }

    // ── Flag API tests ─────────────────────────────────────────────────────

    use crate::core::messages::FlagKind;

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

    // ── debug_payload tests (issue #1150) ──────────────────────────────────
    //
    // The structured projection that replaced the `format_debug` text stream.
    // Asserts the same facts the old text tests did, now as typed payload data
    // the observability dock renders.

    #[test]
    fn debug_payload_empty_has_no_entries_in_any_section() {
        let mods = ShipModifiers::new();
        let p = mods.debug_payload();
        assert_eq!(
            p.schema_version,
            crate::debug::payload::DEBUG_SCHEMA_VERSION
        );
        assert!(p.flags.is_empty(), "no flags on a fresh ship");
        assert!(p.float_modifiers.is_empty(), "no float modifiers");
        assert!(p.int_modifiers.is_empty(), "no int modifiers");
    }

    #[test]
    fn debug_payload_shows_active_flag_with_source() {
        let mut mods = ShipModifiers::new();
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
        let p = mods.debug_payload();
        assert_eq!(p.flags.len(), 1);
        assert_eq!(p.flags[0].flag, "CommsJammed", "flag name");
        assert_eq!(p.flags[0].sources, vec!["ImpulseDrive".to_string()]);
        // Only a flag was set.
        assert!(p.float_modifiers.is_empty());
        assert!(p.int_modifiers.is_empty());
    }

    #[test]
    fn debug_payload_shows_float_modifier_with_multiplier_and_source() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.5,
        ));
        let p = mods.debug_payload();
        assert_eq!(p.float_modifiers.len(), 1);
        let entry = &p.float_modifiers[0];
        assert_eq!(entry.slot, "MaxSpeed", "slot name");
        assert!(
            (entry.multiplier - 1.5).abs() < f32::EPSILON,
            "+0.5 bonus → 1.5× multiplier, got {}",
            entry.multiplier
        );
        assert_eq!(entry.contributions.len(), 1);
        assert_eq!(entry.contributions[0].source, "ImpulseDrive");
        assert!((entry.contributions[0].bonus - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn debug_payload_shows_int_modifier_with_sum_and_source() {
        let mut mods = ShipModifiers::new();
        mods.add_or_update_int(IntModifier {
            source: ModifierSource::ImpulseDrive,
            slot: IntModifierSlot::RepairTeams,
            bonus: 2,
        });
        let p = mods.debug_payload();
        assert_eq!(p.int_modifiers.len(), 1);
        let entry = &p.int_modifiers[0];
        assert_eq!(entry.slot, "RepairTeams", "int slot name");
        assert_eq!(entry.sum, 2, "summed total");
        assert_eq!(entry.contributions.len(), 1);
        assert_eq!(entry.contributions[0].source, "ImpulseDrive");
        assert_eq!(entry.contributions[0].bonus, 2);
    }

    #[test]
    fn debug_payload_omits_empty_float_and_int_slots() {
        let mut mods = ShipModifiers::new();
        // Only a flag — no float or int modifiers.
        mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::SensorBlind);
        let p = mods.debug_payload();
        assert_eq!(p.flags.len(), 1);
        assert!(
            p.float_modifiers.is_empty(),
            "no float slot has a producer, so none is listed"
        );
        assert!(
            p.int_modifiers.is_empty(),
            "no int slot has a producer, so none is listed"
        );
    }

    #[test]
    fn debug_payload_sorts_float_contributions_by_source() {
        let mut mods = ShipModifiers::new();
        // Two sources on one slot; expect them sorted by rendered source name.
        mods.add_or_update(ms(
            ModifierSource::ImpulseDrive,
            ModifierSlot::MaxSpeed,
            0.2,
        ));
        mods.add_or_update(ms(
            ModifierSource::TractorLoad,
            ModifierSlot::MaxSpeed,
            -0.1,
        ));
        let p = mods.debug_payload();
        assert_eq!(p.float_modifiers.len(), 1);
        let sources: Vec<&str> = p.float_modifiers[0]
            .contributions
            .iter()
            .map(|c| c.source.as_str())
            .collect();
        assert_eq!(
            sources,
            vec!["ImpulseDrive", "TractorLoad"],
            "contributions must be sorted for deterministic JSON"
        );
    }
}
