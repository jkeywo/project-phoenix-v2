use crate::core::messages::SystemId;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlSourceResolver {
    /// Derived from the owning hull. No Station, GM availability switch or
    /// reconnect may grant gameplay control over a wreck.
    destroyed: bool,
    sources: HashMap<SystemId, ControlSource>,
    /// Systems that are offline due to damage (Disabled/Destroyed tier).
    ///
    /// When a system is in this set, `policy_for` returns the offline policy
    /// regardless of the `ControlSource` value in `sources`. The set is driven
    /// by `sync_console_damage_tiers` through [`Self::set_offline`] and is
    /// additive: damage overrides the station rating until the console is
    /// repaired.
    offline_systems: HashSet<SystemId>,
    /// Independent GM availability latch. Damage repair and rating changes
    /// cannot clear it; restoring it never changes hull HP or damage tiers.
    gm_disabled_systems: std::collections::BTreeSet<SystemId>,
}

impl ControlSourceResolver {
    pub fn set_destroyed(&mut self, destroyed: bool) {
        self.destroyed = destroyed;
    }
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, system_id: SystemId, source: ControlSource) {
        self.sources.insert(system_id, source);
    }

    pub fn source_for(&self, system_id: &SystemId) -> ControlSource {
        self.sources.get(system_id).copied().unwrap_or_default()
    }

    /// Mark `system_id` as damage-offline (`offline == true`) or restore it
    /// (`offline == false`).
    ///
    /// Offline is additive: while set, `policy_for` returns the offline policy
    /// regardless of the station-rating `ControlSource`, until repair clears it.
    pub fn set_offline(&mut self, system_id: SystemId, offline: bool) {
        if offline {
            self.offline_systems.insert(system_id);
        } else {
            self.offline_systems.remove(&system_id);
        }
    }

    /// True when damage or an explicit GM latch prevents operation.
    pub fn is_offline(&self, system_id: &SystemId) -> bool {
        self.destroyed || self.offline_systems.contains(system_id) || self.is_gm_disabled(system_id)
    }

    pub fn is_gm_disabled(&self, system_id: &SystemId) -> bool {
        self.gm_disabled_systems.contains(system_id)
    }

    /// Absolute state setting; returns whether the independent latch changed.
    pub fn set_gm_disabled(&mut self, system_id: SystemId, disabled: bool) -> bool {
        if disabled {
            self.gm_disabled_systems.insert(system_id)
        } else {
            self.gm_disabled_systems.remove(&system_id)
        }
    }

    pub fn gm_disabled_entries(&self) -> impl Iterator<Item = &SystemId> {
        self.gm_disabled_systems.iter()
    }

    pub fn replace_gm_disabled_systems(&mut self, ids: impl IntoIterator<Item = SystemId>) {
        self.gm_disabled_systems = ids.into_iter().collect();
    }

    /// Replace the complete damage-offline set.
    ///
    /// Snapshot restore uses this instead of replaying damage transitions: the
    /// resolver is read by command admission before the next damage-sync pass,
    /// so inheriting even one bootstrap entry (or omitting one captured entry)
    /// changes which commands are accepted on the first continuation tick.
    pub fn replace_offline_systems(&mut self, system_ids: impl IntoIterator<Item = SystemId>) {
        self.offline_systems.clear();
        self.offline_systems.extend(system_ids);
    }

    pub fn offline_entries(&self) -> impl Iterator<Item = &SystemId> {
        self.offline_systems.iter()
    }

    /// Return the effective `ControlTickPolicy` for `system_id`.
    ///
    /// If the system is in `offline_systems` (damage-driven), the offline policy
    /// is returned unconditionally, overriding any `ControlSource` value.
    pub fn policy_for(&self, system_id: &SystemId) -> ControlTickPolicy {
        if self.is_offline(system_id) {
            return control_tick_policy(ControlSource::Offline);
        }
        control_tick_policy(self.source_for(system_id))
    }

    pub fn entries(&self) -> impl Iterator<Item = (&SystemId, &ControlSource)> {
        self.sources.iter()
    }
}

#[cfg(test)]
#[path = "control_source_tests.rs"]
mod tests;

pub use phoenix_sim_contracts::control::*;
