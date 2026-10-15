//! Supported System mechanisms and their shared command intent. No ECS ownership.
use crate::dock::DockConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandChannelKind {
    Boolean,
    EntityTarget,
    SystemPriority,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandChannelConfig {
    pub name: String,
    pub kind: CommandChannelKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<CommandChannelValue>,
}

/// Shared closed output vocabulary for the two actual independently targeted
/// emitters. It contains no target IDs or global target selection.
pub fn enabled_target_bindings(
    enabled: &str,
    target: &str,
) -> crate::ai::state_tree::SupportedBindings {
    crate::ai::state_tree::SupportedBindings {
        facts: [
            "enabled",
            "has_target",
            "target_available",
            "request_available",
        ]
        .map(str::to_owned)
        .into(),
        channels: [enabled.to_owned(), target.to_owned()].into(),
    }
}

/// The supported palette grows with real migrated consumers, not arbitrary scripts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SystemComponentConfig {
    CraftBay {
        id: String,
        power_input: String,
        config: phoenix_model::craft::BayConfig,
    },
    TransitDrive {
        id: String,
        power_input: String,
        navigation_source: crate::core::messages::SystemId,
        config: phoenix_model::transit::LocalJumpConfig,
    },
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
    IdentityEmitter {
        id: String,
        enabled_channel: String,
        power_input: String,
        config: crate::electronic::identity::IdentityEmitterConfig,
    },
    SignalProtection {
        id: String,
        enabled_channel: String,
        priorities_channel: String,
        power_input: String,
        config: crate::electronic::protection::SignalProtectionConfig,
    },
    JammingEmitter {
        id: String,
        enabled_channel: String,
        target_channel: String,
        power_input: String,
        config: crate::electronic::jamming::JammingConfig,
    },
    IlluminationEmitter {
        id: String,
        enabled_channel: String,
        target_channel: String,
        power_input: String,
        config: crate::weapons::illumination::IlluminationConfig,
    },
    Turret {
        id: String,
        power_input: String,
        config: phoenix_model::turret::TurretConfig,
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
            | Self::TransitDrive { id, .. }
            | Self::CraftBay { id, .. }
            | Self::IlluminationEmitter { id, .. }
            | Self::JammingEmitter { id, .. }
            | Self::SignalProtection { id, .. }
            | Self::IdentityEmitter { id, .. }
            | Self::PowerInput { id }
            | Self::Turret { id, .. }
            | Self::AccessBoundary { id, .. } => id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CommandChannelValue {
    Boolean(bool),
    EntityTarget(Option<String>),
    SystemPriority(Vec<crate::core::messages::SystemId>),
}
/// Technical bound on one intent record, not equipment or message capacity.
pub const MAX_SYSTEM_PRIORITIES: usize = 64;
pub fn validate_system_priorities(
    values: &[crate::core::messages::SystemId],
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    if values.len() > MAX_SYSTEM_PRIORITIES
        || values
            .iter()
            .any(|id| id.0.trim().is_empty() || !seen.insert(id))
    {
        return Err(
            "System priorities require a bounded distinct list of actual System IDs".into(),
        );
    }
    Ok(())
}
impl CommandChannelValue {
    pub fn kind(&self) -> CommandChannelKind {
        match self {
            Self::Boolean(_) => CommandChannelKind::Boolean,
            Self::EntityTarget(_) => CommandChannelKind::EntityTarget,
            Self::SystemPriority(_) => CommandChannelKind::SystemPriority,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Boolean(_) => Ok(()),
            Self::SystemPriority(values) => validate_system_priorities(values),
            Self::EntityTarget(target)
                if target.as_ref().is_none_or(|value| {
                    !value.trim().is_empty()
                        && value.len() <= 128
                        && !value.chars().any(char::is_control)
                }) =>
            {
                Ok(())
            }
            Self::EntityTarget(_) => {
                Err("entity_target requires a bounded nonempty reference or None".into())
            }
        }
    }
}
impl CommandChannelConfig {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(value) = &self.initial {
            if value.kind() != self.kind {
                return Err("channel initial value has the wrong declared kind".into());
            }
            value.validate()?;
        }
        Ok(())
    }
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
                        d.initial.clone().unwrap_or_else(|| match d.kind {
                            CommandChannelKind::Boolean => CommandChannelValue::Boolean(false),
                            CommandChannelKind::EntityTarget => {
                                CommandChannelValue::EntityTarget(None)
                            }
                            CommandChannelKind::SystemPriority => {
                                CommandChannelValue::SystemPriority(Vec::new())
                            }
                        }),
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
    pub fn priorities(&self, name: &str) -> &[crate::core::messages::SystemId] {
        match self.0.get(name) {
            Some(CommandChannelValue::SystemPriority(values)) => values,
            _ => &[],
        }
    }
    pub fn set(&mut self, name: &str, value: CommandChannelValue) -> Result<(), String> {
        if let CommandChannelValue::SystemPriority(values) = &value {
            validate_system_priorities(values)?;
        }
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

pub struct IlluminationComposition<'a> {
    pub component_id: &'a str,
    pub enabled_channel: &'a str,
    pub target_channel: &'a str,
    pub power_input: &'a str,
    pub config: &'a crate::weapons::illumination::IlluminationConfig,
}
pub fn illumination_composition(
    components: &[SystemComponentConfig],
) -> Option<IlluminationComposition<'_>> {
    components.iter().find_map(|component| match component {
        SystemComponentConfig::IlluminationEmitter {
            id,
            enabled_channel,
            target_channel,
            power_input,
            config,
        } => Some(IlluminationComposition {
            component_id: id,
            enabled_channel,
            target_channel,
            power_input,
            config,
        }),
        _ => None,
    })
}
pub struct IdentityComposition<'a> {
    pub enabled_channel: &'a str,
    pub power_input: &'a str,
    pub config: &'a crate::electronic::identity::IdentityEmitterConfig,
}
pub fn identity_composition(
    components: &[SystemComponentConfig],
) -> Option<IdentityComposition<'_>> {
    components.iter().find_map(|component| match component {
        SystemComponentConfig::IdentityEmitter {
            enabled_channel,
            power_input,
            config,
            ..
        } => Some(IdentityComposition {
            enabled_channel,
            power_input,
            config,
        }),
        _ => None,
    })
}
fn validate_identity(system: &super::config::SystemInstanceConfig) -> Result<(), String> {
    let binding = identity_composition(&system.components)
        .ok_or("identity emitter requires its supported component")?;
    binding.config.validate()?;
    let mut ids = BTreeSet::new();
    if system.components.len()!=2 || system.power_group.is_none() || system.channels.len()!=1
        || system.channels[0].name!=binding.enabled_channel || system.channels[0].kind!=CommandChannelKind::Boolean
        || !valid_id(binding.enabled_channel)
        || system.components.iter().any(|component|!valid_id(component.id()) || !ids.insert(component.id()))
        || !system.components.iter().any(|component|matches!(component,SystemComponentConfig::PowerInput{id} if id==binding.power_input)) {
        return Err("identity emitter requires one distinct actual power_input, a power group and its exact Boolean enabled channel".into());
    }
    Ok(())
}
pub struct ProtectionComposition<'a> {
    pub enabled_channel: &'a str,
    pub priorities_channel: &'a str,
    pub config: &'a crate::electronic::protection::SignalProtectionConfig,
}
pub fn protection_composition(
    components: &[SystemComponentConfig],
) -> Option<ProtectionComposition<'_>> {
    components.iter().find_map(|component| match component {
        SystemComponentConfig::SignalProtection {
            enabled_channel,
            priorities_channel,
            config,
            ..
        } => Some(ProtectionComposition {
            enabled_channel,
            priorities_channel,
            config,
        }),
        _ => None,
    })
}
fn validate_protection(system: &super::config::SystemInstanceConfig) -> Result<(), String> {
    let ([SystemComponentConfig::PowerInput { id: input }, SystemComponentConfig::SignalProtection {
        id,
        enabled_channel,
        priorities_channel,
        power_input,
        config,
    }]
    | [SystemComponentConfig::SignalProtection {
        id,
        enabled_channel,
        priorities_channel,
        power_input,
        config,
    }, SystemComponentConfig::PowerInput { id: input }]) = system.components.as_slice()
    else {
        return Err(
            "signal protection requires exactly one supported component and actual power_input"
                .into(),
        );
    };
    config.validate()?;
    if !valid_id(id)
        || !valid_id(input)
        || id == input
        || power_input != input
        || system.power_group.is_none()
        || system.channels.len() != 2
        || enabled_channel == priorities_channel
        || system
            .channels
            .iter()
            .any(|channel| !valid_id(&channel.name))
        || !system.channels.iter().any(|channel| {
            &channel.name == enabled_channel && channel.kind == CommandChannelKind::Boolean
        })
        || !system.channels.iter().any(|channel| {
            &channel.name == priorities_channel
                && channel.kind == CommandChannelKind::SystemPriority
        })
    {
        return Err("signal protection requires exact enabled/priorities channels and a bound actual power input".into());
    }
    if let Some(CommandChannelValue::SystemPriority(priorities)) = system
        .channels
        .iter()
        .find(|channel| &channel.name == priorities_channel)
        .and_then(|channel| channel.initial.as_ref())
    {
        config.validate_priorities(priorities)?;
    }
    Ok(())
}

pub struct JammingComposition<'a> {
    pub component_id: &'a str,
    pub enabled_channel: &'a str,
    pub target_channel: &'a str,
    pub power_input: &'a str,
    pub config: &'a crate::electronic::jamming::JammingConfig,
}
pub fn jamming_composition(components: &[SystemComponentConfig]) -> Option<JammingComposition<'_>> {
    components.iter().find_map(|component| match component {
        SystemComponentConfig::JammingEmitter {
            id,
            enabled_channel,
            target_channel,
            power_input,
            config,
        } => Some(JammingComposition {
            component_id: id,
            enabled_channel,
            target_channel,
            power_input,
            config,
        }),
        _ => None,
    })
}
fn validate_jamming(system: &super::config::SystemInstanceConfig) -> Result<(), String> {
    let binding = jamming_composition(&system.components)
        .ok_or("jamming requires one supported emitter component")?;
    binding.config.validate()?;
    validate_mounted_emitter(
        system,
        binding.component_id,
        binding.enabled_channel,
        binding.target_channel,
        binding.power_input,
        "jamming",
    )
}
fn validate_illumination(system: &super::config::SystemInstanceConfig) -> Result<(), String> {
    let binding = illumination_composition(&system.components)
        .ok_or("illumination requires one supported emitter component")?;
    binding.config.validate()?;
    validate_mounted_emitter(
        system,
        binding.component_id,
        binding.enabled_channel,
        binding.target_channel,
        binding.power_input,
        "illumination",
    )
}
/// Both actual emitters share the same exact typed intent and input/turret wiring.
fn validate_mounted_emitter(
    system: &super::config::SystemInstanceConfig,
    component_id: &str,
    enabled_channel: &str,
    target_channel: &str,
    power_input: &str,
    label: &str,
) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    if system.power_group.is_none()
        || system
            .components
            .iter()
            .any(|component| !valid_id(component.id()) || !ids.insert(component.id()))
    {
        return Err(format!(
            "{label} requires unique component identities and an actual power group"
        ));
    }
    let mut channels = BTreeMap::new();
    for channel in &system.channels {
        if !valid_id(&channel.name)
            || channels
                .insert(channel.name.as_str(), &channel.kind)
                .is_some()
        {
            return Err(format!("{label} requires unique valid generic channels"));
        }
    }
    if channels.len() != 2
        || channels.get(enabled_channel) != Some(&&CommandChannelKind::Boolean)
        || channels.get(target_channel) != Some(&&CommandChannelKind::EntityTarget)
    {
        return Err(format!(
            "{label} requires its exact declared enabled/target channels"
        ));
    }
    let mut emitters = 0;
    let mut inputs = 0;
    let mut turrets = 0;
    for component in &system.components {
        match component {
            component if component.id() == component_id => emitters += 1,
            SystemComponentConfig::PowerInput { id } if id == power_input => inputs += 1,
            SystemComponentConfig::Turret {
                power_input: input,
                config,
                ..
            } if input == power_input => {
                config.validate()?;
                turrets += 1;
            }
            _ => {
                return Err(format!(
                    "{label} contains an unused or incorrectly connected component"
                ))
            }
        }
    }
    if emitters != 1 || inputs != 1 || turrets > 1 {
        return Err(format!(
            "{label} supports one emitter, one power input and at most one shared turret"
        ));
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub struct TurretComposition<'a> {
    pub component_id: &'a str,
    pub power_input: &'a str,
    pub config: &'a phoenix_model::turret::TurretConfig,
}
pub fn turret_composition(components: &[SystemComponentConfig]) -> Option<TurretComposition<'_>> {
    components.iter().find_map(|component| match component {
        SystemComponentConfig::Turret {
            id,
            power_input,
            config,
        } => Some(TurretComposition {
            component_id: id,
            power_input,
            config,
        }),
        _ => None,
    })
}

fn validate_turret(system: &super::config::SystemInstanceConfig) -> Result<(), String> {
    let turret = turret_composition(&system.components)
        .ok_or("mounted System requires one turret component")?;
    turret.config.validate()?;
    let mut ids = BTreeSet::new();
    if system
        .components
        .iter()
        .any(|component| !valid_id(component.id()) || !ids.insert(component.id()))
    {
        return Err("turret System has invalid or duplicate component ids".into());
    }
    if system.components.len() != 2 || !system.components.iter().any(|component| matches!(component, SystemComponentConfig::PowerInput { id } if id == turret.power_input)) {
        return Err("mounted System requires exactly one turret and its referenced power_input".into());
    }
    // Target and availability are borrowed from the one actual capability
    // controller. No extra target/on-off latch or unread generic channels.
    if !system.channels.is_empty() {
        return Err("turret uses the owning capability's existing intent; no additional channels are supported".into());
    }
    if system.power_group.is_none() {
        return Err("turret power_input requires the System power_group binding".into());
    }
    Ok(())
}

pub fn validate_composition(system: &super::config::SystemInstanceConfig) -> Result<(), String> {
    for channel in &system.channels {
        channel.validate()?;
    }
    if system.kind == super::system_registry::CRAFT_BAY_KIND {
        let binding = craft_bay_composition(&system.components)
            .ok_or("Craft Bay requires exactly one craft_bay and its power_input")?;
        binding.config.validate()?;
        let mut ids = BTreeSet::new();
        if system.id.0.trim().is_empty()
            || system.id.0.len() > 128
            || system.id.0.chars().any(char::is_control)
            || system.components.len() != 2
            || system.power_group.is_none()
            || !system.channels.is_empty()
            || system
                .components
                .iter()
                .any(|component| !valid_id(component.id()) || !ids.insert(component.id()))
            || !system.components.iter().any(|component| {
                matches!(component,
                SystemComponentConfig::PowerInput { id } if id == binding.power_input)
            })
        {
            return Err("Craft Bay requires one actual bounded owner, distinct craft_bay/power_input, a power group and no unused channels".into());
        }
        return Ok(());
    }
    if system.kind == super::system_registry::IDENTITY_EMITTER_KIND {
        return validate_identity(system);
    }
    if system.kind == super::system_registry::SIGNAL_PROTECTION_KIND {
        return validate_protection(system);
    }
    if system.kind == super::system_registry::JAMMING_KIND {
        return validate_jamming(system);
    }
    if system.kind == super::system_registry::TRANSIT_DRIVE_KIND {
        let binding = transit_composition(&system.components)
            .ok_or("Transit requires exactly one transit_drive component and its power_input")?;
        binding.config.validate()?;
        let mut ids = BTreeSet::new();
        if system.components.len() != 2
            || system.power_group.is_none()
            || !system.channels.is_empty()
            || system
                .components
                .iter()
                .any(|component| !valid_id(component.id()) || !ids.insert(component.id()))
            || !system.components.iter().any(|component| {
                matches!(component,
                SystemComponentConfig::PowerInput { id } if id == binding.power_input)
            })
        {
            return Err("Transit requires one drive and its distinct power_input, a power group, and no additional channels".into());
        }
        return Ok(());
    }
    if system.kind == super::system_registry::ILLUMINATION_KIND {
        return validate_illumination(system);
    }
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
        if matches!(
            system.kind.as_str(),
            super::system_registry::PHASER_BANK_KIND
                | super::system_registry::BLASTER_BANK_KIND
                | super::system_registry::TORPEDO_TUBE_KIND
                | super::system_registry::PASSIVE_RECEIVER_KIND
        ) && (!system.components.is_empty() || !system.channels.is_empty())
        {
            return validate_turret(system);
        }
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

pub struct CraftBayComposition<'a> {
    pub component_id: &'a str,
    pub power_input: &'a str,
    pub config: &'a phoenix_model::craft::BayConfig,
}
pub fn craft_bay_composition(
    components: &[SystemComponentConfig],
) -> Option<CraftBayComposition<'_>> {
    components.iter().find_map(|component| match component {
        SystemComponentConfig::CraftBay {
            id,
            power_input,
            config,
        } => Some(CraftBayComposition {
            component_id: id,
            power_input,
            config,
        }),
        _ => None,
    })
}

pub struct TransitComposition<'a> {
    pub component_id: &'a str,
    pub power_input: &'a str,
    pub navigation_source: &'a crate::core::messages::SystemId,
    pub config: &'a phoenix_model::transit::LocalJumpConfig,
}
pub fn transit_composition(components: &[SystemComponentConfig]) -> Option<TransitComposition<'_>> {
    components.iter().find_map(|component| match component {
        SystemComponentConfig::TransitDrive {
            id,
            power_input,
            navigation_source,
            config,
        } => Some(TransitComposition {
            component_id: id,
            power_input,
            navigation_source,
            config,
        }),
        _ => None,
    })
}

#[cfg(test)]
#[path = "composition_tests.rs"]
mod tests;
