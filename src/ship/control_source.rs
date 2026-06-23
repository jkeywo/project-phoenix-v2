use crate::messages::SystemId;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlSource {
    #[default]
    Human,
    Ai,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlTickPolicy {
    pub accept_human_input: bool,
    pub operate_ai: bool,
    pub coordinate: bool,
}

pub fn control_tick_policy(source: ControlSource) -> ControlTickPolicy {
    ControlTickPolicy {
        accept_human_input: source == ControlSource::Human,
        operate_ai: source == ControlSource::Ai,
        coordinate: true,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlSourceResolver {
    sources: HashMap<SystemId, ControlSource>,
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

    pub fn policy_for(&self, system_id: &SystemId) -> ControlTickPolicy {
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
}
