//! Closed emitter equipment and generic command-channel ownership.
use crate::{
    ai::state_tree::SupportedBindings,
    ship::composition::{CommandChannelKind, CommandChannelValue, SystemCommandChannels},
    weapons::mount::MountConfig,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IlluminationConfig {
    pub mount: MountConfig,
    pub link_range: f32,
    pub minimum_transmission: f32,
    /// Existing AI text-reading frontier; no additional timer owner.
    pub reading_ticks: u32,
}
impl IlluminationConfig {
    pub fn validate(&self) -> Result<(), String> {
        self.mount.validate()?;
        if !self.link_range.is_finite()
            || self.link_range <= 0.0
            || !self.minimum_transmission.is_finite()
            || self.minimum_transmission <= 0.0
            || self.minimum_transmission > 1.0
        {
            return Err(
                "illumination requires finite positive link range and transmission in (0,1]".into(),
            );
        }
        Ok(())
    }
}

/// Exactly the authored channels, including their declared types. No duplicate
/// enabled/target fields are maintained by GUI, mount, controller or snapshot.
pub fn validate_channels(
    channels: &SystemCommandChannels,
    definitions: &[crate::ship::composition::CommandChannelConfig],
) -> Result<(), String> {
    if channels.0.len() != definitions.len() {
        return Err("illumination channel roster differs from authored equipment".into());
    }
    for channel in definitions {
        match (&channel.kind, channels.0.get(&channel.name)) {
            (CommandChannelKind::Boolean, Some(CommandChannelValue::Boolean(_))) => {}
            (CommandChannelKind::EntityTarget, Some(CommandChannelValue::EntityTarget(target)))
                if target.as_ref().is_none_or(|target| {
                    !target.trim().is_empty()
                        && target.len() <= 1024
                        && !target.chars().any(char::is_control)
                }) => {}
            _ => return Err("illumination channel has unknown or invalid typed intent".into()),
        }
    }
    Ok(())
}

/// Boolean target output means use the one permitted request resolved by this
/// host. Workshop trees cannot construct private identities or arbitrary code.
pub fn bindings(enabled: &str, target: &str) -> SupportedBindings {
    crate::ship::composition::enabled_target_bindings(enabled, target)
}

#[cfg(test)]
#[path = "illumination_tests.rs"]
mod tests;
