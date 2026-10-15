//! Accepted instrument composition. Geometry delegates to actual weapon mounts.
use crate::entities::config::EntityConfig;
use crate::ship::system_registry::{PASSIVE_RECEIVER_KIND, SENSORS_KIND, SENSOR_RADAR_KIND};
pub use phoenix_model::sensing::*;

pub fn mount(config: &SensorMountConfig) -> crate::weapons::mount::MountConfig {
    crate::weapons::mount::MountConfig {
        facing_deg: config.facing_deg,
        fire_arc_deg: config.arc_deg,
        range: config.range,
    }
}
/// No state, resource quantities or observer truth is copied into this profile.
pub fn validate_sensing(profile: &SensingConfig, entity: &EntityConfig) -> Result<(), String> {
    profile.validate()?;
    if profile.broad.is_some() && entity.sensors_console.is_none() {
        return Err("broad sensing requires its actual SensorsConsole radar coverage".into());
    }
    let ship = entity
        .ship_config
        .as_ref()
        .ok_or("sensing requires actual System composition")?;
    let energy = entity
        .energy
        .as_ref()
        .ok_or("sensing requires canonical local energy profiles")?;
    let interior = entity
        .interior
        .as_ref()
        .ok_or("sensing requires physical instrument placement")?;
    if ship.systems.iter().any(|system| {
        system.kind == PASSIVE_RECEIVER_KIND
            && profile
                .passive
                .as_ref()
                .is_none_or(|config| config.system != system.id)
    }) {
        return Err("every fitted passive receiver requires its actual sensing profile".into());
    }
    for (id, kind) in profile
        .focused
        .iter()
        .map(|config| (&config.system, SENSORS_KIND))
        .chain(
            profile
                .broad
                .iter()
                .map(|config| (&config.system, SENSOR_RADAR_KIND)),
        )
        .chain(
            profile
                .passive
                .iter()
                .map(|config| (&config.system, PASSIVE_RECEIVER_KIND)),
        )
    {
        let system = ship
            .system(id)
            .ok_or_else(|| format!("sensing instrument {} is not installed", id.0))?;
        if system.kind != kind
            || system
                .console
                .as_ref()
                .is_none_or(|id| !ship.consoles.iter().any(|console| &console.id == id))
        {
            return Err(format!(
                "sensing instrument {} requires its actual {kind} kind and owning Console",
                id.0
            ));
        }
        if !energy
            .consumers
            .iter()
            .any(|consumer| &consumer.system == id)
            || !interior.placements.contains_key(id)
            || entity.damage.is_none()
            || !system.damage_components.iter().any(|component| {
                component.binding == phoenix_model::damage::DamageBinding::Operation
            })
        {
            return Err(format!(
                "sensing instrument {} requires real reserve, room and Operation hardware",
                id.0
            ));
        }
        if kind == PASSIVE_RECEIVER_KIND {
            let authored = system
                .config
                .as_ref()
                .and_then(|config| config.get("ai"))
                .ok_or("passive receiver requires explicit config.ai state tree")?;
            let tree: crate::ai::state_tree::AiTreeConfig = authored
                .clone()
                .try_into()
                .map_err(|error| format!("invalid passive receiver tree: {error}"))?;
            crate::ai::state_tree::compile(&tree, &crate::science::sensing_ai::bindings())
                .map_err(|error| error.to_string())?;
        }
    }
    if profile.focused.is_some() && entity.scan.is_none() {
        return Err("focused sensing requires actual ScanConfig derivation bands".into());
    }
    Ok(())
}
#[cfg(test)]
#[path = "sensing_tests.rs"]
mod tests;
