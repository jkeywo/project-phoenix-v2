use crate::core::messages::SystemId;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlSource {
    #[default]
    Human,
    Ai,
    /// Explicit offline marker. A system with this source behaves as if it were
    /// in the `offline_systems` set: both `accept_human_input` and `operate_ai`
    /// return `false`. Set by the station-rating system when a rating marks a
    /// system as explicitly offline (distinct from damage-driven offline).
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlTickPolicy {
    pub accept_human_input: bool,
    pub operate_ai: bool,
    pub coordinate: bool,
}

pub fn control_tick_policy(source: ControlSource) -> ControlTickPolicy {
    match source {
        ControlSource::Human => ControlTickPolicy {
            accept_human_input: true,
            operate_ai: false,
            coordinate: true,
        },
        ControlSource::Ai => ControlTickPolicy {
            accept_human_input: false,
            operate_ai: true,
            coordinate: true,
        },
        ControlSource::Offline => ControlTickPolicy {
            accept_human_input: false,
            operate_ai: false,
            coordinate: false,
        },
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlSourceResolver {
    sources: HashMap<SystemId, ControlSource>,
    /// Systems that are offline due to damage (Disabled/Destroyed tier).
    ///
    /// When a system is in this set, `policy_for` returns the offline policy
    /// regardless of the `ControlSource` value in `sources`. The set is driven
    /// by `sync_console_damage_tiers` through [`Self::set_offline`] and is
    /// additive: damage overrides the station rating until the console is
    /// repaired.
    offline_systems: HashSet<SystemId>,
}

impl ControlSourceResolver {
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

    /// True when `system_id` is currently damage-offline.
    pub fn is_offline(&self, system_id: &SystemId) -> bool {
        self.offline_systems.contains(system_id)
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
        if self.offline_systems.contains(system_id) {
            return control_tick_policy(ControlSource::Offline);
        }
        control_tick_policy(self.source_for(system_id))
    }

    pub fn entries(&self) -> impl Iterator<Item = (&SystemId, &ControlSource)> {
        self.sources.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_source_accepts_human_suppresses_ai_and_coordinates() {
        assert_eq!(
            control_tick_policy(ControlSource::Human),
            ControlTickPolicy {
                accept_human_input: true,
                operate_ai: false,
                coordinate: true,
            }
        );
    }

    #[test]
    fn ai_source_suppresses_human_operates_ai_and_coordinates() {
        assert_eq!(
            control_tick_policy(ControlSource::Ai),
            ControlTickPolicy {
                accept_human_input: false,
                operate_ai: true,
                coordinate: true,
            }
        );
    }

    #[test]
    fn resolver_defaults_to_human_per_instance() {
        let resolver = ControlSourceResolver::new();

        assert_eq!(
            resolver.source_for(&SystemId("helm".into())),
            ControlSource::Human
        );
    }

    #[test]
    fn resolver_selects_source_per_system_instance() {
        let mut resolver = ControlSourceResolver::new();
        let helm = SystemId("helm".into());
        let red_alert = SystemId("red-alert".into());

        resolver.set(helm.clone(), ControlSource::Ai);

        assert_eq!(resolver.source_for(&helm), ControlSource::Ai);
        assert_eq!(resolver.source_for(&red_alert), ControlSource::Human);
        assert_eq!(
            resolver.policy_for(&helm),
            ControlTickPolicy {
                accept_human_input: false,
                operate_ai: true,
                coordinate: true,
            }
        );
    }

    #[test]
    fn offline_returns_false_false_false_policy() {
        assert_eq!(
            control_tick_policy(ControlSource::Offline),
            ControlTickPolicy {
                accept_human_input: false,
                operate_ai: false,
                coordinate: false,
            }
        );
    }

    #[test]
    fn offline_systems_gate_overrides_human_and_ai() {
        let mut resolver = ControlSourceResolver::new();
        let helm = SystemId("helm".into());
        let tactical = SystemId("tactical".into());

        // helm is Human-controlled, tactical is Ai-controlled.
        resolver.set(helm.clone(), ControlSource::Human);
        resolver.set(tactical.clone(), ControlSource::Ai);

        // Mark both as offline via damage.
        resolver.set_offline(helm.clone(), true);
        resolver.set_offline(tactical.clone(), true);
        assert!(resolver.is_offline(&helm));
        assert!(resolver.is_offline(&tactical));

        // Both must return the offline policy, regardless of ControlSource.
        let offline_policy = ControlTickPolicy {
            accept_human_input: false,
            operate_ai: false,
            coordinate: false,
        };
        assert_eq!(resolver.policy_for(&helm), offline_policy);
        assert_eq!(resolver.policy_for(&tactical), offline_policy);
    }

    #[test]
    fn restore_from_offline_set_restores_original_policy() {
        let mut resolver = ControlSourceResolver::new();
        let helm = SystemId("helm".into());

        // Human-controlled, then marked offline.
        resolver.set(helm.clone(), ControlSource::Human);
        resolver.set_offline(helm.clone(), true);

        // Offline.
        assert_eq!(
            resolver.policy_for(&helm),
            ControlTickPolicy {
                accept_human_input: false,
                operate_ai: false,
                coordinate: false,
            }
        );

        // Repair: remove from offline set.
        resolver.set_offline(helm.clone(), false);
        assert!(!resolver.is_offline(&helm));

        // Human policy restored.
        assert_eq!(
            resolver.policy_for(&helm),
            ControlTickPolicy {
                accept_human_input: true,
                operate_ai: false,
                coordinate: true,
            }
        );
    }

    #[test]
    fn replacing_offline_systems_clears_bootstrap_entries() {
        let mut resolver = ControlSourceResolver::new();
        let bootstrap = SystemId("bootstrap".into());
        let captured = SystemId("captured".into());
        resolver.set_offline(bootstrap.clone(), true);

        resolver.replace_offline_systems([captured.clone()]);

        assert!(!resolver.is_offline(&bootstrap));
        assert!(resolver.is_offline(&captured));
        assert_eq!(resolver.offline_entries().count(), 1);
    }
}
