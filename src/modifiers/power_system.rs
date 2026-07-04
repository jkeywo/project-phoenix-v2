use crate::messages::PowerGroupId;
use std::collections::HashMap;

pub const HELM_POWER_GROUP: &str = "helm";
pub const WEAPONS_POWER_GROUP: &str = "weapons";
pub const SENSORS_POWER_GROUP: &str = "sensors";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerAllocationError {
    UnknownGroup(PowerGroupId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerReadState {
    pub allocations: Vec<(PowerGroupId, u8)>,
    pub battery_charge: f32,
    pub locked: bool,
}

impl PowerReadState {
    pub fn level_for_group(&self, group: &PowerGroupId) -> Option<u8> {
        self.allocations
            .iter()
            .find(|(id, _)| id == group)
            .map(|(_, level)| *level)
    }
}

pub struct Channel1Read<'a> {
    state: &'a PowerReadState,
}

impl<'a> Channel1Read<'a> {
    pub fn new(state: &'a PowerReadState) -> Self {
        Self { state }
    }

    pub fn power_level(&self, group: &PowerGroupId) -> Option<u8> {
        self.state.level_for_group(group)
    }
}

/// The stable canonical order of the built-in power groups. The publisher
/// walks this order to build wire snapshots. Extending the list requires
/// touching the wire format and every filter path, so keep it minimal.
pub const POWER_GROUP_ORDER: &[&str] =
    &[HELM_POWER_GROUP, WEAPONS_POWER_GROUP, SENSORS_POWER_GROUP];

/// Pure `PowerSystem` state — keyed by [`PowerGroupId`] after issue #617.
///
/// The three canonical groups (`helm`, `weapons`, `sensors`) are seeded at
/// construction so tests can rely on `level_for` returning `Some(2)` without
/// first calling `set_group_allocation`. Additional groups can be added by
/// TOML-driven config in future PRs.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerSystem {
    /// Per-group allocation level. Values are clamped to `[1, 4]` by the
    /// setter API; direct construction should preserve that invariant.
    groups: HashMap<PowerGroupId, u8>,
    /// Insertion order of `groups`; walked by publishers so wire output is
    /// deterministic even when the HashMap iteration order isn't.
    order: Vec<PowerGroupId>,
    pub battery_charge: f32,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerConfig {
    pub capacity: f32,
    pub rates: [f32; 6],
    pub emergency_threshold: f32,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            capacity: 100.0,
            rates: [6.0, 5.0, 4.0, 2.0, -2.0, -6.0],
            emergency_threshold: 25.0,
        }
    }
}

impl Default for PowerSystem {
    fn default() -> Self {
        Self::seeded_with_defaults(100.0)
    }
}

impl PowerSystem {
    pub fn new(config: &PowerConfig) -> Self {
        Self::seeded_with_defaults(config.capacity)
    }

    /// Internal helper: construct a PowerSystem with the three canonical
    /// groups pre-seeded at level 2 and the requested battery charge.
    fn seeded_with_defaults(battery_charge: f32) -> Self {
        let mut groups = HashMap::with_capacity(3);
        let mut order = Vec::with_capacity(3);
        for &name in POWER_GROUP_ORDER {
            let id = PowerGroupId(name.to_string());
            groups.insert(id.clone(), 2u8);
            order.push(id);
        }
        Self {
            groups,
            order,
            battery_charge,
            locked: false,
        }
    }

    /// Total allocation across all groups.
    pub fn total(&self) -> u8 {
        self.groups.values().copied().sum()
    }

    /// Current level for the given power group. Returns `0` for groups the
    /// system does not know about (matches the historical
    /// `power_level_for_console` fallback for non-powered consoles).
    pub fn level_for(&self, group: &PowerGroupId) -> u8 {
        self.groups.get(group).copied().unwrap_or(0)
    }

    /// True if the system tracks the given power group.
    pub fn has_group(&self, group: &PowerGroupId) -> bool {
        self.groups.contains_key(group)
    }

    /// Insertion-ordered iteration over `(&PowerGroupId, level)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&PowerGroupId, u8)> {
        self.order
            .iter()
            .map(move |id| (id, *self.groups.get(id).unwrap_or(&0)))
    }

    pub fn read_state(&self) -> PowerReadState {
        PowerReadState {
            allocations: self
                .order
                .iter()
                .map(|id| (id.clone(), *self.groups.get(id).unwrap_or(&0)))
                .collect(),
            battery_charge: self.battery_charge,
            locked: self.locked,
        }
    }

    /// Set the allocation for a specific power group to `level`, clamped to
    /// `[1, 4]`. Delta is applied one step at a time via `increase` /
    /// `decrease` so the `total() <= 8` and `locked` invariants are honoured.
    pub fn set_group_allocation(
        &mut self,
        group: &PowerGroupId,
        level: u8,
    ) -> Result<(), PowerAllocationError> {
        if !self.groups.contains_key(group) {
            return Err(PowerAllocationError::UnknownGroup(group.clone()));
        }
        let current = self.level_for(group);
        let target_level = level.clamp(1, 4);
        if target_level > current {
            for _ in 0..(target_level - current) {
                self.increase(group);
            }
        } else if target_level < current {
            for _ in 0..(current - target_level) {
                self.decrease(group);
            }
        }
        Ok(())
    }

    /// Increase the allocation for `group` by 1. Clamped to `4` per group and
    /// to `8` for the total. No-op when the system is locked.
    pub fn increase(&mut self, group: &PowerGroupId) {
        if self.locked || self.total() >= 8 {
            return;
        }
        if let Some(v) = self.groups.get_mut(group) {
            if *v < 4 {
                *v += 1;
            }
        }
    }

    /// Decrease the allocation for `group` by 1. Clamped to `1` per group.
    /// No-op when the system is locked.
    pub fn decrease(&mut self, group: &PowerGroupId) {
        if self.locked {
            return;
        }
        if let Some(v) = self.groups.get_mut(group) {
            if *v > 1 {
                *v -= 1;
            }
        }
    }

    /// Advance the simulation by `dt` seconds. Updates battery charge based on the
    /// current total allocation, handles exhaustion (forced to 1 + lock), and
    /// recovery (unlock at emergency threshold).
    ///
    /// Returns `true` if the `locked` state changed this tick.
    pub fn tick(&mut self, dt: f32, config: &PowerConfig) -> bool {
        let prev_locked = self.locked;
        let total = self.total().clamp(3, 8) as usize;
        let rate = config.rates[total - 3];
        self.battery_charge = (self.battery_charge + rate * dt).clamp(0.0, config.capacity);

        if self.battery_charge <= 0.0 {
            for v in self.groups.values_mut() {
                *v = 1;
            }
            self.locked = true;
        } else if self.locked && self.battery_charge >= config.emergency_threshold {
            self.locked = false;
        }

        self.locked != prev_locked
    }
}

/// Free function preserving the historical `power_level_for_console`
/// signature but keyed on `PowerGroupId`. Returns `0` for unknown groups.
pub fn power_level_for_group(ps: &PowerSystem, group: &PowerGroupId) -> u8 {
    ps.level_for(group)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]

    use super::*;

    fn helm() -> PowerGroupId {
        PowerGroupId(HELM_POWER_GROUP.into())
    }
    fn weapons() -> PowerGroupId {
        PowerGroupId(WEAPONS_POWER_GROUP.into())
    }
    fn sensors() -> PowerGroupId {
        PowerGroupId(SENSORS_POWER_GROUP.into())
    }

    #[test]
    fn defaults() {
        let ps = PowerSystem::default();
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&weapons()), 2);
        assert_eq!(ps.level_for(&sensors()), 2);
        assert_eq!(ps.battery_charge, 100.0);
        assert!(!ps.locked);
    }

    #[test]
    fn increase_helm() {
        let mut ps = PowerSystem::default();
        ps.increase(&helm());
        assert_eq!(ps.level_for(&helm()), 3);
    }

    #[test]
    fn increase_weapons() {
        let mut ps = PowerSystem::default();
        ps.increase(&weapons());
        assert_eq!(ps.level_for(&weapons()), 3);
    }

    #[test]
    fn increase_sensors() {
        let mut ps = PowerSystem::default();
        ps.increase(&sensors());
        assert_eq!(ps.level_for(&sensors()), 3);
    }

    #[test]
    fn increase_at_four_is_noop() {
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap();
        ps.increase(&helm());
        assert_eq!(ps.level_for(&helm()), 4);
    }

    #[test]
    fn increase_at_total_cap_eight_is_noop() {
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 3).unwrap();
        ps.set_group_allocation(&weapons(), 3).unwrap();
        // sensors is 2 → total = 8
        assert_eq!(ps.total(), 8);
        ps.increase(&sensors());
        assert_eq!(ps.level_for(&sensors()), 2);
    }

    #[test]
    fn increase_when_locked_is_noop() {
        let mut ps = PowerSystem::default();
        ps.locked = true;
        // Force helm down to 1 first (locked blocks decrease too, so seed
        // via a fresh system then toggle locked).
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 1).unwrap();
        ps.locked = true;
        ps.increase(&helm());
        assert_eq!(ps.level_for(&helm()), 1);
    }

    #[test]
    fn decrease_helm() {
        let mut ps = PowerSystem::default();
        ps.decrease(&helm());
        assert_eq!(ps.level_for(&helm()), 1);
    }

    #[test]
    fn decrease_weapons() {
        let mut ps = PowerSystem::default();
        ps.decrease(&weapons());
        assert_eq!(ps.level_for(&weapons()), 1);
    }

    #[test]
    fn decrease_sensors() {
        let mut ps = PowerSystem::default();
        ps.decrease(&sensors());
        assert_eq!(ps.level_for(&sensors()), 1);
    }

    #[test]
    fn decrease_at_one_is_noop() {
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 1).unwrap();
        ps.decrease(&helm());
        assert_eq!(ps.level_for(&helm()), 1);
    }

    #[test]
    fn decrease_when_locked_is_noop() {
        let mut ps = PowerSystem::default();
        ps.locked = true;
        ps.decrease(&helm());
        assert_eq!(ps.level_for(&helm()), 2);
    }

    // ── tick ──────────────────────────────────────────────────────────────

    #[test]
    fn tick_discharges_above_base() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap();
        // total = 8 → rate = -6.0/s
        ps.battery_charge = 100.0;
        ps.tick(1.0, &config);
        assert!((ps.battery_charge - 94.0).abs() < 0.001);
    }

    #[test]
    fn tick_recharges_below_base() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // total = 6 → rate = 2.0/s
        ps.battery_charge = 50.0;
        ps.tick(1.0, &config);
        assert!((ps.battery_charge - 52.0).abs() < 0.001);
    }

    #[test]
    fn tick_cannot_overcharge_beyond_capacity() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // total = 3 → rate = 6.0/s
        ps.set_group_allocation(&helm(), 1).unwrap();
        ps.set_group_allocation(&weapons(), 1).unwrap();
        ps.set_group_allocation(&sensors(), 1).unwrap();
        ps.battery_charge = 99.0;
        ps.tick(1.0, &config);
        assert!((ps.battery_charge - 100.0).abs() < 0.001);
    }

    #[test]
    fn exhaustion_forces_consoles_to_one_and_locks() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // total = 8 → rate = -6.0/s; drain battery completely
        ps.set_group_allocation(&helm(), 4).unwrap();
        ps.battery_charge = 5.0;
        ps.tick(1.0, &config);
        assert_eq!(ps.level_for(&helm()), 1);
        assert_eq!(ps.level_for(&weapons()), 1);
        assert_eq!(ps.level_for(&sensors()), 1);
        assert!(ps.locked);
    }

    #[test]
    fn recovery_unlocks_at_threshold() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 1).unwrap();
        ps.set_group_allocation(&weapons(), 1).unwrap();
        ps.set_group_allocation(&sensors(), 1).unwrap();
        ps.locked = true;
        ps.battery_charge = 10.0;
        // total = 3 → rate = 6.0/s; charge from 10 to 16 (still below 25)
        ps.tick(1.0, &config);
        assert!(ps.locked);
        // charge from 16 to 22
        ps.tick(1.0, &config);
        assert!(ps.locked);
        // charge from 22 to 28 → crosses 25
        ps.tick(1.0, &config);
        assert!(!ps.locked);
    }

    #[test]
    fn locked_blocks_increase_and_decrease() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // drain battery to force lock
        ps.set_group_allocation(&helm(), 4).unwrap();
        ps.battery_charge = 5.0;
        ps.tick(1.0, &config);
        assert!(ps.locked);

        // Now all are 1; try to increase — should be no-op
        ps.increase(&helm());
        assert_eq!(ps.level_for(&helm()), 1);
        ps.decrease(&helm());
        assert_eq!(ps.level_for(&helm()), 1);
    }

    #[test]
    fn tick_returns_true_when_lock_changes() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap();
        ps.battery_charge = 5.0;
        // tick that exhausts the battery → locked changes from false to true
        assert!(ps.tick(1.0, &config));
        // next tick: locked stays true
        assert!(!ps.tick(1.0, &config));
    }

    // ── configurable constructor ──────────────────────────────────────────

    #[test]
    fn custom_config() {
        let config = PowerConfig {
            capacity: 50.0,
            rates: [1.0, 1.0, 1.0, -1.0, -2.0, -3.0],
            emergency_threshold: 10.0,
        };
        let ps = PowerSystem::new(&config);
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&weapons()), 2);
        assert_eq!(ps.level_for(&sensors()), 2);
        assert!((ps.battery_charge - 50.0).abs() < 0.001);
        assert!(!ps.locked);
    }

    #[test]
    fn increase_sensors_increases_radar_power_slot() {
        let mut ps = PowerSystem::default();
        ps.increase(&sensors());
        assert_eq!(
            ps.level_for(&sensors()),
            3,
            "Sensors group should drive radar power"
        );
    }

    #[test]
    fn increase_on_unknown_group_is_noop() {
        let mut ps = PowerSystem::default();
        // Neither shields nor navigation are seeded as power groups.
        ps.increase(&PowerGroupId("shields".into()));
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&weapons()), 2);
        assert_eq!(ps.level_for(&sensors()), 2);

        ps.increase(&PowerGroupId("navigation".into()));
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&weapons()), 2);
        assert_eq!(ps.level_for(&sensors()), 2);
    }

    #[test]
    fn set_group_allocation_updates_named_group() {
        let mut ps = PowerSystem::default();

        ps.set_group_allocation(&weapons(), 3).unwrap();

        assert_eq!(ps.level_for(&weapons()), 3);
        assert_eq!(ps.level_for(&helm()), 2);
        assert_eq!(ps.level_for(&sensors()), 2);
    }

    #[test]
    fn set_group_allocation_rejects_unknown_group() {
        let mut ps = PowerSystem::default();
        let group = PowerGroupId("life-support".into());

        assert_eq!(
            ps.set_group_allocation(&group, 3),
            Err(PowerAllocationError::UnknownGroup(group))
        );
        assert_eq!(ps.total(), 6);
    }

    #[test]
    fn channel_1_read_exposes_power_without_mutation_access() {
        let mut ps = PowerSystem::default();
        ps.set_group_allocation(&helm(), 4).unwrap();
        let state = ps.read_state();
        let channel_1 = Channel1Read::new(&state);

        assert_eq!(channel_1.power_level(&helm()), Some(4));
        assert_eq!(channel_1.power_level(&PowerGroupId("unknown".into())), None);
        assert_eq!(ps.level_for(&helm()), 4);
    }
}
