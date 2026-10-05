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
        let mut disabled = [false; ModifierSlot::COUNT];
        for ((source, slot), bonus) in self.table.iter() {
            disabled[slot.index()] |= matches!(source, ModifierSource::SystemDisabled(_));
            sums[slot.index()] += *bonus;
        }
        for slot in ModifierSlot::all() {
            let sum = sums[slot.index()];
            self.cache[slot.index()] = if disabled[slot.index()] {
                0.0
            } else if sum >= 0.0 {
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
        ModifierSource::SystemDisabled(sid) => format!("SystemDisabled({})", sid.0),
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
