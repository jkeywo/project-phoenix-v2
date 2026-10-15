use super::*;

/// A permission/service label alone is not actual powered facility machinery.
pub fn validate_refit_facility(config: &EntityConfig) -> Result<(), String> {
    let Some(facility) = &config.refit_facility else {
        return Ok(());
    };
    let ship = config
        .ship_config
        .as_ref()
        .ok_or("refit facility requires actual owned Systems")?;
    let system = ship
        .system(&facility.system)
        .ok_or("refit facility System is absent")?;
    if system.kind != crate::ship::system_registry::DOCK_KIND
        || !system
            .console
            .as_ref()
            .is_some_and(|id| ship.consoles.iter().any(|console| &console.id == id))
        || crate::ship::composition::dock_composition(&system.components).is_none()
    {
        return Err(
            "refit facility requires an actual Dock System with its Console and mating capability"
                .into(),
        );
    }
    if !config
        .energy
        .as_ref()
        .is_some_and(|energy| energy.consumers.iter().any(|c| c.system == facility.system))
    {
        return Err("refit facility Dock requires its actual local energy reserve".into());
    }
    if config.port_service.is_none() {
        return Err("refit facility requires an explicit port policy and eligible factions; no default permission".into());
    }
    if !config
        .interior
        .as_ref()
        .is_some_and(|layout| layout.placements.contains_key(&facility.system))
        || config.damage.is_none()
    {
        return Err("refit facility requires actual physical placement and condition".into());
    }
    Ok(())
}
