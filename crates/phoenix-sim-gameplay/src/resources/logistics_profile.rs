//! Resolve authored stores against their real installed endpoint owners.
use super::logistics::{CompiledLogistics, EndpointDefinition, EndpointStock, ResourceEndpoint};
use crate::entities::config::EntityConfig;
use std::collections::BTreeMap;

pub fn compile_entity_logistics(
    config: &EntityConfig,
) -> Result<Option<CompiledLogistics>, String> {
    let Some(authored) = &config.logistics else {
        if config.ship_config.as_ref().is_some_and(|ship| {
            ship.systems
                .iter()
                .any(|s| s.kind == super::logistics::LOGISTICS_KIND)
        }) {
            return Err(
                "A fitted logistics System requires an explicit [logistics] profile".into(),
            );
        }
        return Ok(None);
    };
    let ship = config
        .ship_config
        .as_ref()
        .ok_or("logistics requires installed Systems")?;
    let layout = config
        .interior
        .as_ref()
        .ok_or("logistics requires compartment routes")?;
    let graph = crate::ship::interior::InteriorGraph::compile(
        layout,
        ship,
        config
            .repair
            .as_ref()
            .map_or(0, |repair| repair.repair_team_count as usize),
    )?;
    let mut definitions = BTreeMap::new();
    let mut stocks = BTreeMap::new();
    if let Some(infrastructure) = &config.infrastructure {
        let state = crate::infrastructure::InfrastructureState::from_config(infrastructure);
        for capacity in &infrastructure.capacities {
            let endpoint = ResourceEndpoint::Capacity {
                id: capacity.id.clone(),
            };
            let stock = state
                .capacity_stock(&capacity.id)
                .ok_or("configured capacity has no stock")?
                .clone();
            definitions.insert(
                endpoint.clone(),
                EndpointDefinition {
                    ceiling: stock.ceiling,
                },
            );
            stocks.insert(endpoint, EndpointStock { stock, held: 0 });
        }
    }
    if let Some(torpedoes) = &config.torpedoes {
        let system = crate::weapons::torpedo::TorpedoSystem::from_configs(
            &torpedoes.tubes,
            torpedoes.to_runtime(),
        );
        let held = system.magazine.headroom() - system.magazine_headroom();
        definitions.insert(
            ResourceEndpoint::TorpedoMagazine {},
            EndpointDefinition {
                ceiling: system.magazine.ceiling,
            },
        );
        stocks.insert(
            ResourceEndpoint::TorpedoMagazine {},
            EndpointStock {
                stock: system.magazine,
                held,
            },
        );
    }
    let profile = CompiledLogistics::compile(authored, ship, &graph, &definitions)?;
    if !config.energy.as_ref().is_some_and(|energy| {
        energy
            .consumers
            .iter()
            .any(|consumer| consumer.system == profile.handling().system)
    }) {
        return Err("logistics handling requires an actual local-energy consumer".into());
    }
    for store in &authored.stores {
        if store.endpoint == (ResourceEndpoint::TorpedoMagazine {})
            && store.system.as_ref().is_none_or(|id| {
                ship.system(id)
                    .is_none_or(|s| s.kind != crate::ship::system_registry::TORPEDO_MAGAZINE_KIND)
            })
        {
            return Err(
                "the managed torpedo magazine requires its actual TorpedoMagazine System owner"
                    .into(),
            );
        }
        if store.role == phoenix_model::logistics::StoreRole::Local
            && store.endpoint == (ResourceEndpoint::TorpedoMagazine {})
        {
            let publication = store.system.as_ref().expect("compiled local owner");
            let boards = std::collections::HashMap::from([(
                publication.clone(),
                crate::core::messages::SystemBlackboard::TorpedoMagazine(Default::default()),
            )]);
            if !ship
                .observations_for_system(&profile.handling().system, &boards)
                .get(publication)
                .is_some()
            {
                return Err("Logistics needs an explicit typed TorpedoMagazine feed for its local demand; co-hosted Consoles do not share knowledge".into());
            }
        }
    }
    profile.validate_stocks(&stocks)?;
    Ok(Some(profile))
}
