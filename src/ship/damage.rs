use crate::messages::SystemId;
use crate::shield::ShieldSystem;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Apply hull damage via `SystemHull`.
///
/// Takes the final hull damage amount (after shields), distributes it randomly
/// across systems, and returns:
///
/// - `hull_damage_applied`: what was actually absorbed by systems
/// - `ship_destroyed`: true when all systems have reached 0 HP after this hit
pub fn apply_hull_damage(
    hull: &mut SystemHull,
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
/// sub-issue #617). Successor of the retired `ConsoleHull` type.
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
    entries: HashMap<SystemId, SystemHullEntry>,
    /// Ordered list of system ids (matches TOML insertion order).
    order: Vec<SystemId>,
}

impl SystemHull {
    /// Build from a list of `(SystemId, max_hp)` pairs using default tier
    /// thresholds and derived display names. All systems start at full HP.
    pub fn from_config(config: &[(SystemId, f32)]) -> Self {
        let mut order = Vec::with_capacity(config.len());
        let mut entries = HashMap::with_capacity(config.len());
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
        let mut entries = HashMap::with_capacity(config.len());
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
        let mut entries = HashMap::with_capacity(config.len());
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
    pub fn apply_damage(&mut self, mut amount: f32, rng: &mut impl Rng) {
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
            // Weighted selection: generate r in [0, total), subtract each
            // system's HP in order; choose the first one that drives r
            // negative.
            let mut r = rng.random::<f32>() * total;
            let mut chosen_id: Option<SystemId> = None;
            for id in &self.order {
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
                    .find(|id| {
                        self.entries
                            .get(*id)
                            .is_some_and(|e| e.current > 0.0)
                    })
                    .cloned()
                    .expect("total > 0.0 implies at least one live entry")
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

    /// Restore `amount` HP to the first system that is below its max HP.
    /// Useful for repair systems that don't yet target a specific system.
    /// Returns the `SystemId` that was restored, or `None` if all are at max.
    pub fn restore_any_damaged(&mut self, amount: f32) -> Option<SystemId> {
        for sid in &self.order {
            let entry = self
                .entries
                .get_mut(sid)
                .expect("SystemHull invariant: order and entries agree");
            if entry.current < entry.max {
                entry.current = (entry.current + amount).min(entry.max);
                return Some(sid.clone());
            }
        }
        None
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
/// per-arc `[[hull.console_hull]]` entries (mirrors how #512 skipped
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
    pub fn apply_damage(&mut self, mut amount: f32, rng: &mut impl Rng) {
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
            let mut r = rng.random::<f32>() * total;
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
        let mut hull = SystemHull::from_config(&[(sid("helm"), 5.0), (sid("tactical"), 10.0)]);
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
            SystemHull::from_config(&[(sid("helm"), 5.0), (sid("tactical"), 100.0)]);
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
        hull.restore(&sid("helm"), 10.0);
        // Only Helm should have HP restored
        assert!(near(hull.current_for(&sid("helm")).unwrap(), 10.0));
        assert!(near(hull.current_for(&sid("tactical")).unwrap(), 0.0));
        assert!(near(hull.current_for(&sid("power")).unwrap(), 0.0));
        assert!(near(hull.current_for(&sid("shields")).unwrap(), 0.0));
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
        let mut rng = rand::rng();
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
        let mut rng = rand::rng();
        // Directly set HP to 74 by restoring after wiping.
        hull.apply_damage(100.0, &mut rng); // wipe to 0
        hull.restore(&sid("helm"), 74.0); // 74/100 = 0.74 < 0.75 → Damaged
        assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Damaged);
    }

    #[test]
    fn tier_is_disabled_below_disabled_threshold() {
        // Default disabled_threshold = 0.25. 24% of 100 → Disabled.
        let mut hull = SystemHull::from_config(&[(sid("helm"), 100.0)]);
        let mut rng = rand::rng();
        hull.apply_damage(100.0, &mut rng); // wipe to 0
        hull.restore(&sid("helm"), 24.0); // 24/100 = 0.24 < 0.25 → Disabled
        assert_eq!(hull.tier_for(&sid("helm")), DamageTier::Disabled);
    }

    #[test]
    fn tier_is_destroyed_at_zero_hp() {
        let mut hull = SystemHull::from_config(&[(sid("helm"), 25.0)]);
        let mut rng = rand::rng();
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
        let mut rng = rand::rng();

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
        let mut rng = rand::rng();

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
        let mut rng = rand::rng();
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
        let mut rng = rand::rng();
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
        let mut rng = rand::rng();
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
        let mut rng = rand::rng();
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
        let mut rng = rand::rng();
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
}
