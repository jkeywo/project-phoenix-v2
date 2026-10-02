use crate::core::messages::SystemId;
use crate::weapons::shield::ShieldSystem;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use vellum_rng::Pcg32;

/// A uniform `f32` in `[0, 1)` from one 32-bit draw.
///
/// `Pcg32` deliberately implements no `rand` traits, so there is no
/// `random::<f32>()` to call and the conversion is written here, once, where
/// the weighted damage roll below can be read against it (issue #897).
///
/// An `f32` has a 24-bit significand, so 24 bits is exactly what can be kept
/// without rounding: `>> 8` leaves an integer in `0..2^24`, and multiplying by
/// `2^-24` maps it onto the 2^24 evenly spaced representable multiples of
/// `2^-24` across `[0, 1)`. Both operands are powers of two and the numerator
/// is exactly representable, so every product is exact — no rounding, no
/// platform variation, and `1.0` is unreachable.
///
/// `pub(crate)` since issue #929, so the beam-cycle jitter draws its uniform the
/// same way rather than growing a second conversion with its own rounding. That last part is load-bearing
/// for the callers: they scale by a total and walk the systems subtracting, and
/// a draw of exactly `1.0` would fall off the end of the list every time.
///
/// The *high* 24 bits are the ones kept. PCG's XSH-RR output permutation makes
/// the whole word equally good, so this is a convention rather than a
/// correction — but it is the convention the sequence is now recorded against.
pub(crate) fn unit_f32(rng: &mut Pcg32) -> f32 {
    (rng.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
}

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
    /// `debuff_magnitude` in the `[[hull.system_hull]]` TOML block.
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

/// Apply hull damage via `SystemHull`.
///
/// Takes the final hull damage amount (after shields), distributes it randomly
/// across systems, and returns:
///
/// - `hull_damage_applied`: what was actually absorbed by systems
/// - `ship_destroyed`: true when all systems have reached 0 HP after this hit
pub fn apply_hull_damage(hull: &mut SystemHull, amount: f32, rng: &mut Pcg32) -> (f32, bool) {
    apply_hull_damage_within(hull, None, amount, rng)
}

/// [`apply_hull_damage`] restricted to the Systems named by `allow` (issue
/// #1311); `None` is the whole hull and is exactly [`apply_hull_damage`].
///
/// `ship_destroyed` still asks the WHOLE hull, not the scope: a Station hit
/// that empties the last living Systems ends the ship, and a scoped hit that
/// leaves other Systems alive has not. Destruction is a fact about the entity,
/// which is why it is measured where it was before this scope existed.
pub fn apply_hull_damage_within(
    hull: &mut SystemHull,
    allow: Option<&[SystemId]>,
    amount: f32,
    rng: &mut Pcg32,
) -> (f32, bool) {
    let before = hull.total_current();
    hull.apply_damage_within(allow, amount, rng);
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

// ── SystemHull ────────────────────────────────────────────────────────────────

/// One entry in [`SystemHull`]: per-system HP + tier thresholds + display name.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemHullEntry {
    /// Current hull HP for this system.
    pub current: f32,
    /// Maximum hull HP for this system.
    pub max: f32,
    /// Tier thresholds for damage-tier derivation.
    pub tier_config: ConsoleTierConfig,
    /// Human-readable name for UI display. Falls back to the raw
    /// `SystemId` string when no `display_name` was supplied via TOML.
    pub display_name: String,
}

/// Per-system hull tracker keyed by [`SystemId`] (parent issue #516,
/// sub-issue #617).
///
/// Stores `(SystemId, entry)` pairs plus a parallel insertion-order `order`
/// vec so iteration is deterministic (a bare HashMap would randomise iteration
/// and break deterministic damage distribution — see `ShipArcHull` for the
/// same pattern).
///
/// Damage is distributed randomly across entries that still have HP, spilling
/// to further random entries when a system reaches 0. Repair targets a
/// specific system by [`SystemId`].
///
/// Pure struct — `ship/damage.rs` is Bevy-free per AGENTS.md rule 9.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemHull {
    /// A `BTreeMap`, so `values()` yields a stable order. Float addition is not
    /// associative and `HashMap` iteration order follows `RandomState`'s
    /// per-process seed, so the hull totals below differed in their last bits
    /// between two runs of the same seeded binary (observed: 0.6000061 vs
    /// 0.5999756). Those totals feed damage-tier thresholds, so the drift did
    /// not stay cosmetic — seeded duels diverged into different outcomes.
    /// Ordering here rather than looking each id up through `order` keeps the
    /// sums a single walk, with no per-entry lookup.
    entries: std::collections::BTreeMap<SystemId, SystemHullEntry>,
    /// Ordered list of system ids (matches TOML insertion order).
    order: Vec<SystemId>,
}

impl SystemHull {
    /// Build from a list of `(SystemId, max_hp)` pairs using default tier
    /// thresholds and derived display names. All systems start at full HP.
    pub fn from_config(config: &[(SystemId, f32)]) -> Self {
        let mut order = Vec::with_capacity(config.len());
        let mut entries = std::collections::BTreeMap::new();
        for (sid, max) in config {
            let display_name = sid.0.clone();
            if !entries.contains_key(sid) {
                order.push(sid.clone());
            }
            entries.insert(
                sid.clone(),
                SystemHullEntry {
                    current: *max,
                    max: *max,
                    tier_config: ConsoleTierConfig::default(),
                    display_name,
                },
            );
        }
        Self { entries, order }
    }

    /// Build from a list of `(SystemId, max_hp, tier_config)` triples.
    /// Display names default to the raw `SystemId` string.
    pub fn from_config_with_tiers(config: &[(SystemId, f32, ConsoleTierConfig)]) -> Self {
        let mut order = Vec::with_capacity(config.len());
        let mut entries = std::collections::BTreeMap::new();
        for (sid, max, tc) in config {
            let display_name = sid.0.clone();
            if !entries.contains_key(sid) {
                order.push(sid.clone());
            }
            entries.insert(
                sid.clone(),
                SystemHullEntry {
                    current: *max,
                    max: *max,
                    tier_config: *tc,
                    display_name,
                },
            );
        }
        Self { entries, order }
    }

    /// Build from a list of `(SystemId, display_name, max_hp, tier_config)`
    /// quadruples — the spawner main path uses this to preserve TOML-supplied
    /// display names.
    pub fn from_config_with_display_names(
        config: Vec<(SystemId, String, f32, ConsoleTierConfig)>,
    ) -> Self {
        let mut order = Vec::with_capacity(config.len());
        let mut entries = std::collections::BTreeMap::new();
        for (sid, display_name, max, tc) in config {
            if !entries.contains_key(&sid) {
                order.push(sid.clone());
            }
            entries.insert(
                sid,
                SystemHullEntry {
                    current: max,
                    max,
                    tier_config: tc,
                    display_name,
                },
            );
        }
        Self { entries, order }
    }

    /// True when the tracker has no systems.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Return the `DamageTier` for the given system.
    ///
    /// - `Destroyed`: `current == 0`
    /// - `Disabled`: `current/max < disabled_threshold_pct`
    /// - `Damaged`: `current/max < damaged_threshold_pct`
    /// - `Operational`: otherwise
    ///
    /// Returns `Operational` for systems not tracked by this hull.
    pub fn tier_for(&self, sid: &SystemId) -> DamageTier {
        let Some(entry) = self.entries.get(sid) else {
            return DamageTier::Operational;
        };
        if entry.current == 0.0 {
            return DamageTier::Destroyed;
        }
        let ratio = if entry.max > 0.0 {
            entry.current / entry.max
        } else {
            0.0
        };
        if ratio < entry.tier_config.disabled_threshold_pct {
            DamageTier::Disabled
        } else if ratio < entry.tier_config.damaged_threshold_pct {
            DamageTier::Damaged
        } else {
            DamageTier::Operational
        }
    }

    /// Apply `amount` of damage distributed across systems above 0 HP,
    /// weighted by their remaining HP (a system with more HP is proportionally
    /// more likely to absorb the next hit). Damage spills to further weighted
    /// selections when a system is exhausted. Systems already at 0 HP are
    /// never targeted.
    pub fn apply_damage(&mut self, amount: f32, rng: &mut Pcg32) {
        self.apply_damage_within(None, amount, rng);
    }

    /// [`Self::apply_damage`] restricted to `allow`, the Systems a scoped hit
    /// is permitted to touch (issue #1311). `None` is the whole hull and is
    /// EXACTLY [`Self::apply_damage`] — one walk, one weighting, one spill
    /// loop, so a Station-scoped GM hit and a beam hit choose their systems by
    /// the same rule rather than by two rules that could drift.
    ///
    /// A system outside `allow` is invisible to every step: it contributes no
    /// weight, cannot be chosen, cannot be the float-precision fallback, and
    /// cannot absorb spill. That is the whole of "Station scope never affects a
    /// System outside its authored ownership, and System scope never spills to
    /// siblings" — the restriction lives in the distribution itself rather than
    /// in a caller that clamps afterwards.
    pub fn apply_damage_within(
        &mut self,
        allow: Option<&[SystemId]>,
        mut amount: f32,
        rng: &mut Pcg32,
    ) {
        let in_scope = |id: &SystemId| allow.is_none_or(|allow| allow.contains(id));
        while amount > 0.0 {
            let total: f32 = self
                .order
                .iter()
                .filter(|id| in_scope(id))
                .filter_map(|id| self.entries.get(id))
                .filter(|entry| entry.current > 0.0)
                .map(|entry| entry.current)
                .sum();
            if total == 0.0 {
                break;
            }
            // Weighted selection: generate r in [0, total), subtract each
            // system's HP in order; choose the first one that drives r
            // negative.
            let mut r = unit_f32(rng) * total;
            let mut chosen_id: Option<SystemId> = None;
            for id in self.order.iter().filter(|id| in_scope(id)) {
                let entry = self
                    .entries
                    .get(id)
                    .expect("SystemHull invariant: order and entries agree");
                if entry.current <= 0.0 {
                    continue;
                }
                r -= entry.current;
                if r < 0.0 {
                    chosen_id = Some(id.clone());
                    break;
                }
            }
            // Float-precision safety: fall back to the last available system.
            let idx = chosen_id.unwrap_or_else(|| {
                self.order
                    .iter()
                    .rev()
                    .filter(|id| in_scope(id))
                    .find(|id| self.entries.get(*id).is_some_and(|e| e.current > 0.0))
                    .cloned()
                    .expect("total > 0.0 implies at least one live entry in scope")
            });
            let entry = self
                .entries
                .get_mut(&idx)
                .expect("SystemHull invariant: order and entries agree");
            let absorbed = amount.min(entry.current);
            entry.current -= absorbed;
            amount -= absorbed;
        }
    }

    /// Iterate `(SystemId, current, max)` triples in TOML declaration order.
    /// Kept as `(&SystemId, f32, f32)` so callers don't have to reach into
    /// `SystemHullEntry` for the two most common fields.
    pub fn entries(&self) -> impl Iterator<Item = (&SystemId, f32, f32)> {
        self.order.iter().map(move |id| {
            let entry = self
                .entries
                .get(id)
                .expect("SystemHull invariant: order and entries agree");
            (id, entry.current, entry.max)
        })
    }

    /// Iterate the full `(SystemId, &SystemHullEntry)` view when callers need
    /// the display name / tier config.
    pub fn iter(&self) -> impl Iterator<Item = (&SystemId, &SystemHullEntry)> {
        self.order.iter().map(move |id| {
            let entry = self
                .entries
                .get(id)
                .expect("SystemHull invariant: order and entries agree");
            (id, entry)
        })
    }

    /// Look up a full entry by SystemId.
    pub fn get(&self, sid: &SystemId) -> Option<&SystemHullEntry> {
        self.entries.get(sid)
    }

    /// Restore `amount` HP to a specific system, clamped to its max.
    /// Systems not present in the map are silently ignored.
    pub fn restore(&mut self, sid: &SystemId, amount: f32) {
        if let Some(entry) = self.entries.get_mut(sid) {
            entry.current = (entry.current + amount).min(entry.max);
        }
    }

    /// Sum of current HP across all systems.
    pub fn total_current(&self) -> f32 {
        self.entries.values().map(|e| e.current).sum()
    }

    /// Sum of current HP across the Systems named by `allow` (issue #1311),
    /// walked in declaration order rather than map order so a scoped total is
    /// the same last bit on every peer — the reason `entries` is a `BTreeMap`
    /// at all, applied to a subset.
    pub fn total_current_within(&self, allow: Option<&[SystemId]>) -> f32 {
        match allow {
            None => self.total_current(),
            Some(allow) => self
                .order
                .iter()
                .filter(|id| allow.contains(id))
                .filter_map(|id| self.entries.get(id))
                .map(|entry| entry.current)
                .sum(),
        }
    }

    /// Sum of max HP across the Systems named by `allow` (issue #1311).
    pub fn total_max_within(&self, allow: Option<&[SystemId]>) -> f32 {
        match allow {
            None => self.total_max(),
            Some(allow) => self
                .order
                .iter()
                .filter(|id| allow.contains(id))
                .filter_map(|id| self.entries.get(id))
                .map(|entry| entry.max)
                .sum(),
        }
    }

    /// Distribute `amount` of healing across systems below their max HP,
    /// weighted by their MISSING HP, spilling to further weighted selections
    /// when one fills up. Returns the amount actually restored.
    ///
    /// The exact mirror of [`Self::apply_damage`] (issue #1310): same walk over
    /// `order`, same `unit_f32` draw against a running weight total, same
    /// spill-until-exhausted loop, with `max - current` in place of `current`.
    /// A whole-entity heal has to choose systems the same way a whole-entity
    /// hit does, or one mechanic would answer "which system" with two rules.
    ///
    /// A system at 0 HP has the largest possible headroom and is therefore the
    /// most likely to be picked: [`Self::restore`] is already documented as the
    /// one route back out of `Destroyed`, and this is that route applied to the
    /// whole hull rather than to one named system.
    ///
    /// Callers that must report what they discarded clamp against
    /// [`Self::total_missing`] first; this function simply stops when there is
    /// nothing left to fill.
    pub fn restore_distributed(&mut self, amount: f32, rng: &mut Pcg32) -> f32 {
        self.restore_distributed_within(None, amount, rng)
    }

    /// [`Self::restore_distributed`] restricted to `allow` (issue #1311), the
    /// exact mirror of [`Self::apply_damage_within`] and restricted for its
    /// reason: healing mirrors scope so a Station repair cannot quietly refill
    /// another Station's systems.
    pub fn restore_distributed_within(
        &mut self,
        allow: Option<&[SystemId]>,
        mut amount: f32,
        rng: &mut Pcg32,
    ) -> f32 {
        let in_scope = |id: &SystemId| allow.is_none_or(|allow| allow.contains(id));
        let mut restored = 0.0f32;
        while amount > 0.0 {
            let total: f32 = self
                .order
                .iter()
                .filter(|id| in_scope(id))
                .filter_map(|id| self.entries.get(id))
                .map(|entry| (entry.max - entry.current).max(0.0))
                .sum();
            if total == 0.0 {
                break;
            }
            let mut r = unit_f32(rng) * total;
            let mut chosen_id: Option<SystemId> = None;
            for id in self.order.iter().filter(|id| in_scope(id)) {
                let entry = self
                    .entries
                    .get(id)
                    .expect("SystemHull invariant: order and entries agree");
                let missing = (entry.max - entry.current).max(0.0);
                if missing <= 0.0 {
                    continue;
                }
                r -= missing;
                if r < 0.0 {
                    chosen_id = Some(id.clone());
                    break;
                }
            }
            // Float-precision safety: fall back to the last system with room,
            // exactly as the damage walk falls back to the last live one.
            let idx = chosen_id.unwrap_or_else(|| {
                self.order
                    .iter()
                    .rev()
                    .filter(|id| in_scope(id))
                    .find(|id| {
                        self.entries
                            .get(*id)
                            .is_some_and(|e| (e.max - e.current) > 0.0)
                    })
                    .cloned()
                    .expect("total > 0.0 implies at least one entry with headroom in scope")
            });
            let entry = self
                .entries
                .get_mut(&idx)
                .expect("SystemHull invariant: order and entries agree");
            let filled = amount.min((entry.max - entry.current).max(0.0));
            entry.current += filled;
            amount -= filled;
            restored += filled;
        }
        restored
    }

    /// Sum of headroom (`max - current`) across all systems — what a heal can
    /// absorb before the rest of it is discarded (issue #1310).
    pub fn total_missing(&self) -> f32 {
        self.entries
            .values()
            .map(|e| (e.max - e.current).max(0.0))
            .sum()
    }

    /// Sum of max HP across all systems.
    pub fn total_max(&self) -> f32 {
        self.entries.values().map(|e| e.max).sum()
    }

    /// True only when every system is at 0 HP.
    pub fn is_destroyed(&self) -> bool {
        !self.entries.is_empty() && self.entries.values().all(|e| e.current == 0.0)
    }

    /// Current HP for a specific system. Returns `None` if not tracked.
    pub fn current_for(&self, sid: &SystemId) -> Option<f32> {
        self.entries.get(sid).map(|e| e.current)
    }

    /// Return the active debuff magnitude for the given system.
    ///
    /// - `Operational` or `Destroyed` → `0.0` (fully operational or fully
    ///   offline; no partial debuff applies).
    /// - `Damaged` or `Disabled` → `tier_config.debuff_magnitude` from the
    ///   per-system TOML configuration.
    ///
    /// Returns `0.0` for systems not tracked by this hull.
    pub fn debuff_magnitude_for(&self, sid: &SystemId) -> f32 {
        let Some(entry) = self.entries.get(sid) else {
            return 0.0;
        };
        match self.tier_for(sid) {
            DamageTier::Operational | DamageTier::Destroyed => 0.0,
            DamageTier::Damaged | DamageTier::Disabled => entry.tier_config.debuff_magnitude,
        }
    }

    /// Returns `true` if the given system is at its maximum HP (or not
    /// tracked).
    pub fn is_at_max(&self, sid: &SystemId) -> bool {
        match self.entries.get(sid) {
            Some(entry) => entry.current >= entry.max,
            None => true, // not tracked → treat as full
        }
    }

    /// Directly set the current HP for a given system. No-op if the system
    /// is not tracked. Clamps to `[0.0, max_hp]`.
    ///
    /// Used in tests to set specific damage states without applying random
    /// hull damage.
    pub fn set_hp(&mut self, sid: &SystemId, new_hp: f32) {
        if let Some(entry) = self.entries.get_mut(sid) {
            entry.current = new_hp.clamp(0.0, entry.max);
        }
    }
}

// ── ShipArcHull (issue #514) ──────────────────────────────────────────────────

/// One entry in [`ShipArcHull`]: per-arc hull HP + tier thresholds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcHullEntry {
    /// Current hull HP for this arc.
    pub current: f32,
    /// Maximum hull HP for this arc.
    pub max: f32,
    /// Tier thresholds for damage-tier derivation.
    pub tier_config: ConsoleTierConfig,
}

/// Per-arc hull tracker (issue #514).
///
/// Parallels [`SystemHull`] but keyed by arc id (string) instead of
/// [`SystemId`]. Each shield arc declared via a `[[shield_arc]]` block in
/// ship TOML gets a corresponding entry here. Damage is distributed
/// proportionally to total hull damage on the ship (the arc HP pool tracks
/// total ship hull damage, matching how per-system hull entries behave).
///
/// `sync_console_damage_tiers` iterates this component alongside
/// [`SystemHull`], deriving `SystemId("shield-arc-<id>")` offline states from
/// each entry's tier.
///
/// Skipped on NPCs — NPCs use scalar `hull_integrity` and do not declare
/// per-arc `[[hull.system_hull]]` entries (mirrors how #512 skipped
/// per-bank/tube hull on NPCs).
///
/// Pure struct — `ship/damage.rs` is Bevy-free per AGENTS.md rule 9.
/// The Bevy `Component` wrapper lives in `entities/spawner.rs` as
/// `ShipArcHullComponent`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShipArcHull {
    entries: HashMap<String, ArcHullEntry>,
    /// Ordered list of arc ids (matches TOML insertion order) — HashMap
    /// alone would randomise iteration and break deterministic damage
    /// distribution.
    order: Vec<String>,
}

impl ShipArcHull {
    /// Build from a list of `(arc_id, ArcHullEntry)` pairs. Preserves the
    /// caller's order for deterministic iteration.
    pub fn from_entries(entries: Vec<(String, ArcHullEntry)>) -> Self {
        let mut order = Vec::with_capacity(entries.len());
        let mut map = HashMap::with_capacity(entries.len());
        for (id, entry) in entries {
            if !map.contains_key(&id) {
                order.push(id.clone());
            }
            map.insert(id, entry);
        }
        Self {
            entries: map,
            order,
        }
    }

    /// True when the tracker has no arcs (NPCs, empty TOMLs).
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Iterate `(arc_id, ArcHullEntry)` pairs in TOML declaration order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ArcHullEntry)> {
        self.order.iter().map(|id| {
            let entry = self
                .entries
                .get(id)
                .expect("ShipArcHull invariant: order and entries agree");
            (id.as_str(), entry)
        })
    }

    /// Look up an entry by arc id.
    pub fn get(&self, arc_id: &str) -> Option<&ArcHullEntry> {
        self.entries.get(arc_id)
    }

    /// Return the current [`DamageTier`] for the given arc.
    ///
    /// Returns `Operational` for arcs not tracked by this hull (mirrors
    /// [`SystemHull::tier_for`]).
    pub fn tier_for(&self, arc_id: &str) -> DamageTier {
        let Some(entry) = self.entries.get(arc_id) else {
            return DamageTier::Operational;
        };
        if entry.current == 0.0 {
            return DamageTier::Destroyed;
        }
        let ratio = if entry.max > 0.0 {
            entry.current / entry.max
        } else {
            0.0
        };
        if ratio < entry.tier_config.disabled_threshold_pct {
            DamageTier::Disabled
        } else if ratio < entry.tier_config.damaged_threshold_pct {
            DamageTier::Damaged
        } else {
            DamageTier::Operational
        }
    }

    /// Apply `amount` of damage distributed across arcs above 0 HP, weighted
    /// by remaining HP — same policy as [`SystemHull::apply_damage`]. Damage
    /// spills to further weighted selections when an arc is exhausted.
    pub fn apply_damage(&mut self, mut amount: f32, rng: &mut Pcg32) {
        while amount > 0.0 {
            let total: f32 = self
                .order
                .iter()
                .filter_map(|id| self.entries.get(id))
                .filter(|entry| entry.current > 0.0)
                .map(|entry| entry.current)
                .sum();
            if total == 0.0 {
                break;
            }
            let mut r = unit_f32(rng) * total;
            let mut chosen_id: Option<String> = None;
            for id in &self.order {
                let entry = self
                    .entries
                    .get(id)
                    .expect("ShipArcHull invariant: order and entries agree");
                if entry.current <= 0.0 {
                    continue;
                }
                r -= entry.current;
                if r < 0.0 {
                    chosen_id = Some(id.clone());
                    break;
                }
            }
            let idx = chosen_id.unwrap_or_else(|| {
                self.order
                    .iter()
                    .rev()
                    .find(|id| {
                        self.entries
                            .get(id.as_str())
                            .is_some_and(|e| e.current > 0.0)
                    })
                    .cloned()
                    .expect("total > 0.0 implies at least one live entry")
            });
            let entry = self
                .entries
                .get_mut(&idx)
                .expect("ShipArcHull invariant: order and entries agree");
            let absorbed = amount.min(entry.current);
            entry.current -= absorbed;
            amount -= absorbed;
        }
    }

    /// Restore `amount` HP to a specific arc, clamped to its max. Arcs not
    /// present are silently ignored.
    pub fn restore(&mut self, arc_id: &str, amount: f32) {
        if let Some(entry) = self.entries.get_mut(arc_id) {
            entry.current = (entry.current + amount).min(entry.max);
        }
    }

    /// Directly set the current HP for a given arc. No-op if the arc is not
    /// tracked. Clamps to `[0.0, max]`. Test helper.
    pub fn set_hp(&mut self, arc_id: &str, new_hp: f32) {
        if let Some(entry) = self.entries.get_mut(arc_id) {
            entry.current = new_hp.clamp(0.0, entry.max);
        }
    }
}

#[cfg(test)]
#[path = "damage_tests.rs"]
mod tests;
