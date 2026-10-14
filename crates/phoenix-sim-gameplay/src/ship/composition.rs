//! Supported System mechanisms and their shared command intent. No ECS ownership.
use crate::dock::DockConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandChannelKind {
    Boolean,
    EntityTarget,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandChannelConfig {
    pub name: String,
    pub kind: CommandChannelKind,
}

/// The supported palette grows with real migrated consumers, not arbitrary scripts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SystemComponentConfig {
    DockMating {
        id: String,
        enabled_channel: String,
        target_channel: String,
        power_input: String,
        config: DockConfig,
    },
    PowerInput {
        id: String,
    },
    AccessBoundary {
        id: String,
        sealed_channel: String,
        transition_secs: f32,
    },
}
impl SystemComponentConfig {
    pub fn id(&self) -> &str {
        match self {
            Self::DockMating { id, .. }
            | Self::PowerInput { id }
            | Self::AccessBoundary { id, .. } => id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CommandChannelValue {
    Boolean(bool),
    EntityTarget(Option<String>),
}

/// One canonical intent store per composed System; components read the same values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemCommandChannels(pub BTreeMap<String, CommandChannelValue>);
impl SystemCommandChannels {
    pub fn from_definitions(definitions: &[CommandChannelConfig]) -> Self {
        Self(
            definitions
                .iter()
                .map(|d| {
                    (
                        d.name.clone(),
                        match d.kind {
                            CommandChannelKind::Boolean => CommandChannelValue::Boolean(false),
                            CommandChannelKind::EntityTarget => {
                                CommandChannelValue::EntityTarget(None)
                            }
                        },
                    )
                })
                .collect(),
        )
    }
    pub fn boolean(&self, name: &str) -> bool {
        matches!(self.0.get(name), Some(CommandChannelValue::Boolean(true)))
    }
    pub fn target(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Some(CommandChannelValue::EntityTarget(value)) => value.as_deref(),
            _ => None,
        }
    }
    pub fn set(&mut self, name: &str, value: CommandChannelValue) -> Result<(), String> {
        let previous = self
            .0
            .get_mut(name)
            .ok_or_else(|| format!("unknown command channel '{name}'"))?;
        if std::mem::discriminant(previous) != std::mem::discriminant(&value) {
            return Err(format!("command channel '{name}' has the wrong value type"));
        }
        *previous = value;
        Ok(())
    }
}

/// Borrowed resolved bindings, with no second copy of authored state.
pub struct DockComposition<'a> {
    pub component_id: &'a str,
    pub enabled_channel: &'a str,
    pub target_channel: &'a str,
    pub power_input: &'a str,
    pub config: &'a DockConfig,
}

pub fn dock_composition(components: &[SystemComponentConfig]) -> Option<DockComposition<'_>> {
    components.iter().find_map(|component| match component {
        SystemComponentConfig::DockMating {
            id,
            enabled_channel,
            target_channel,
            power_input,
            config,
        } => Some(DockComposition {
            component_id: id,
            enabled_channel,
            target_channel,
            power_input,
            config,
        }),
        _ => None,
    })
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn validate_composition(system: &super::config::SystemInstanceConfig) -> Result<(), String> {
    if system.kind == super::system_registry::ACCESS_GATE_KIND {
        let [SystemComponentConfig::AccessBoundary {
            id,
            sealed_channel,
            transition_secs,
        }] = system.components.as_slice()
        else {
            return Err("Access gate requires exactly one access_boundary component".into());
        };
        if !valid_id(id) || !transition_secs.is_finite() || *transition_secs <= 0.0 {
            return Err(
                "Access boundary requires a valid component id and positive finite transition_secs"
                    .into(),
            );
        }
        if system.channels.len() != 1
            || system.channels[0].name != *sealed_channel
            || system.channels[0].kind != CommandChannelKind::Boolean
        {
            return Err("Access boundary requires its declared boolean sealed channel".into());
        }
        return Ok(());
    }
    if system.kind != super::system_registry::DOCK_KIND {
        if !system.components.is_empty() || !system.channels.is_empty() {
            return Err(format!(
                "System kind '{}' does not yet support component composition",
                system.kind
            ));
        }
        return Ok(());
    }
    if system.components.is_empty() {
        return Err("Dock requires explicit dock_mating and power_input components; move active [dock] tuning into [[system.component]] (increment 2 format)".into());
    }
    let mut ids = BTreeSet::new();
    for component in &system.components {
        if !valid_id(component.id()) || !ids.insert(component.id()) {
            return Err(format!(
                "invalid or duplicate component id '{}'",
                component.id()
            ));
        }
    }
    let mut channels = BTreeMap::new();
    for channel in &system.channels {
        if !valid_id(&channel.name)
            || channels
                .insert(channel.name.as_str(), &channel.kind)
                .is_some()
        {
            return Err(format!(
                "invalid or duplicate command channel '{}'",
                channel.name
            ));
        }
    }
    let count = system
        .components
        .iter()
        .filter(|c| matches!(c, SystemComponentConfig::DockMating { .. }))
        .count();
    if count != 1 {
        return Err("Dock supports exactly one dock_mating component".into());
    }
    let dock = dock_composition(&system.components).expect("one checked mating component");
    dock.config.validate()?;
    // A non-finite value can otherwise survive the legacy positive-distance test.
    if [
        dock.config.range,
        dock.config.engage_distance,
        dock.config.approach_speed,
        dock.config.mate_tolerance,
        dock.config.undock_clear_distance,
    ]
    .iter()
    .any(|v| !v.is_finite())
    {
        return Err("Dock component distances must be finite".into());
    }
    if channels.get(dock.enabled_channel) != Some(&&CommandChannelKind::Boolean)
        || channels.get(dock.target_channel) != Some(&&CommandChannelKind::EntityTarget)
    {
        return Err(
            "Dock channel bindings require a declared boolean engagement and entity_target latch"
                .into(),
        );
    }
    if channels.len() != 2 {
        return Err("Dock declares only channels read by its supported components".into());
    }
    if !system
        .components
        .iter()
        .any(|c| matches!(c, SystemComponentConfig::PowerInput { id } if id == dock.power_input))
    {
        return Err(format!(
            "Dock references missing power_input component '{}'",
            dock.power_input
        ));
    }
    if system.components.len() != 2 {
        return Err("Dock must not contain unused components".into());
    }
    if system.power_group.is_none() {
        return Err("Dock power_input requires the System power_group binding".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "composition_tests.rs"]
mod tests;
