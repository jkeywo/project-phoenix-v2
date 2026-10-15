//! Accepted instrument composition. Geometry delegates to actual weapon mounts.
use crate::entities::config::EntityConfig;
use crate::ship::system_registry::{
    HELM_RADAR_KIND, PASSIVE_RECEIVER_KIND, SENSORS_KIND, SENSOR_RADAR_KIND, TACTICAL_RADAR_KIND,
};
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
    let ship = entity
        .ship_config
        .as_ref()
        .ok_or("sensing requires actual System composition")?;
    for sharing in &profile.sharing {
        if ship.system(&sharing.system).is_none_or(|system| {
            !matches!(
                system.kind.as_str(),
                SENSOR_RADAR_KIND | TACTICAL_RADAR_KIND
            )
        }) {
            return Err(
                "sharing currently supports actual Sensors and Tactical radar owners".into(),
            );
        }
    }
    for (binding, kind, label) in [
        (
            &profile.control.combat_lock,
            TACTICAL_RADAR_KIND,
            "combat_lock",
        ),
        (
            &profile.control.science_target,
            SENSORS_KIND,
            "science_target",
        ),
    ] {
        if let Some(owner) = binding {
            if ship.system(owner).is_none_or(|system| {
                system.kind != kind
                    || system
                        .console
                        .as_ref()
                        .is_none_or(|console| !ship.consoles.iter().any(|row| &row.id == console))
            }) {
                return Err(format!(
                    "sensing.control.{label} must bind an actual owned {kind} System"
                ));
            }
        } else if !profile.broad.is_empty() && ship.systems.iter().any(|system| system.kind == kind)
        {
            return Err(format!(
                "fitted broad sensing requires explicit sensing.control.{label} actuator ownership"
            ));
        }
    }
    for owner in profile
        .focused
        .iter()
        .map(|row| &row.system)
        .chain(profile.control.science_target.iter())
        .chain(profile.control.combat_lock.iter())
    {
        let system = ship
            .system(owner)
            .ok_or("sensing controller owner missing")?;
        let authored = system
            .config
            .as_ref()
            .and_then(|config| config.get("ai"))
            .ok_or_else(|| {
                format!(
                    "sensing controller {} requires an explicit config.ai tree",
                    owner.0
                )
            })?;
        let tree: crate::ai::state_tree::AiTreeConfig = authored
            .clone()
            .try_into()
            .map_err(|error| format!("invalid sensing controller: {error}"))?;
        let bindings = if system.kind == TACTICAL_RADAR_KIND {
            crate::science::sensing_ai::targeting_bindings()
        } else {
            crate::science::sensing_ai::focused_bindings()
        };
        crate::ai::state_tree::compile(&tree, &bindings).map_err(|error| error.to_string())?;
    }
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
                .iter()
                .all(|config| config.system != system.id)
    }) {
        return Err("every fitted passive receiver requires its actual sensing profile".into());
    }
    for (id, kind) in profile
        .focused
        .iter()
        .map(|config| (&config.system, SENSORS_KIND))
        .chain(profile.broad.iter().map(|config| (&config.system, "broad")))
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
        if (kind != "broad" && system.kind != kind)
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
        if kind == "broad" {
            broad_radar(id, entity)?;
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
    if !profile.focused.is_empty() && entity.scan.is_none() {
        return Err("focused sensing requires actual ScanConfig derivation bands".into());
    }
    Ok(())
}
/// ShipConfig validates the dynamic publication grammar; the complete entity
/// requires an actual configured producer rather than a command-only Sensors node.
pub fn validate_scan_feeds(entity: &EntityConfig) -> Result<(), String> {
    let Some(ship) = &entity.ship_config else {
        return Ok(());
    };
    for feed in ship
        .consoles
        .iter()
        .flat_map(|console| &console.feeds)
        .filter(|feed| feed.kind == phoenix_model::console::ConsoleObservationKind::Scan)
    {
        let owner = scan_owner_from_blackboard_key(&feed.source)
            .ok_or("Scan feed requires a qualified actual instrument publication")?;
        if entity.scan.is_none()
            || !entity.sensing.as_ref().is_some_and(|profile| {
                profile
                    .focused
                    .iter()
                    .any(|instrument| instrument.system == owner)
            })
        {
            return Err(format!(
                "Scan feed {} has no actual configured focused instrument",
                feed.source.0
            ));
        }
    }
    Ok(())
}
/// Derive immutable coverage from the actual fitted radar's existing capability.
/// No AiProfile reach or co-hosted Console is evidence of equipment.
pub fn broad_radar(
    system: &phoenix_model::identity::SystemId,
    entity: &EntityConfig,
) -> Result<crate::radar_config::RadarConfig, String> {
    let kind = &entity
        .ship_config
        .as_ref()
        .and_then(|ship| ship.system(system))
        .ok_or("broad instrument is not installed")?
        .kind;
    let radar = match kind.as_str() {
        HELM_RADAR_KIND => entity
            .helm_console
            .as_ref()
            .and_then(|console| console.radar.as_ref()),
        TACTICAL_RADAR_KIND => entity
            .weapons_console
            .as_ref()
            .and_then(|console| console.radar.as_ref()),
        SENSOR_RADAR_KIND => entity
            .sensors_console
            .as_ref()
            .map(|console| &console.long_range_radar),
        _ => return Err(
            "broad instruments require actual HelmRadar, TacticalRadar or SensorRadar equipment"
                .into(),
        ),
    }
    .ok_or_else(|| {
        format!(
            "broad instrument {} requires its actual {kind} radar coverage",
            system.0
        )
    })?;
    if !radar.range.is_finite() || radar.range <= 0.0 {
        return Err(format!(
            "broad instrument {} requires finite positive actual radar coverage",
            system.0
        ));
    }
    Ok(radar.clone())
}
#[cfg(test)]
#[path = "sensing_tests.rs"]
mod tests;
