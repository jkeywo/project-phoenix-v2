//! Declared port terms bind to existing finite endpoint owners.
use super::service::{resolve_endpoint, ResourceEndpoint, ResourceLink};
use crate::entities::config::EntityConfig;

pub fn validate_entity_port(config: &EntityConfig) -> Result<(), String> {
    let Some(profile) = &config.port_service else {
        return Ok(());
    };
    profile.validate()?;
    let composed_dock = config.ship_config.as_ref().is_some_and(|ship| ship.systems.iter().any(|system|
        system.kind == crate::ship::system_registry::DOCK_KIND
            && crate::ship::composition::dock_composition(&system.components).is_some()));
    if config.dock.is_none() && !composed_dock {
        return Err("Port services require an actual docking capability".into());
    }
    let mut identities = std::collections::BTreeMap::new();
    for resource in profile.offers.iter().flat_map(|offer| {
        std::iter::once(offer.resource.as_str())
            .chain(offer.costs.iter().map(|cost| cost.resource.as_str()))
    }) {
        let endpoint = resolve_endpoint(
            resource,
            &config.resource_services,
            &ResourceLink::default(),
        )?;
        if identities
            .insert(endpoint.clone(), resource)
            .is_some_and(|previous| previous != resource)
        {
            return Err(
                "Port resources cannot relabel one actual endpoint as different commodities".into(),
            );
        }
        if config.logistics.as_ref().is_some_and(|profile| {
            profile
                .stores
                .iter()
                .any(|store| store.endpoint == endpoint && store.resource != resource)
        }) {
            return Err(format!(
                "Port resource {resource:?} conflicts with the actual managed store commodity"
            ));
        }
        match endpoint {
            ResourceEndpoint::Capacity { id }
                if !config.infrastructure.as_ref().is_some_and(|inventory| {
                    inventory
                        .capacities
                        .iter()
                        .any(|capacity| capacity.id == id)
                }) =>
            {
                return Err(format!(
                    "Port resource {resource:?} requires an actual finite capacity {id:?}"
                ));
            }
            ResourceEndpoint::TorpedoMagazine {} if config.torpedoes.is_none() => {
                return Err("Port resource requires an actual torpedo magazine".into());
            }
            _ => {}
        }
    }
    Ok(())
}
