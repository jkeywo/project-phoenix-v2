//! Resolve authored stores against their real installed endpoint owners.
use super::logistics::{CompiledLogistics, EndpointDefinition, EndpointStock, ResourceEndpoint};
use crate::entities::config::EntityConfig;
use std::collections::BTreeMap;

pub fn compile_entity_logistics(
    config: &EntityConfig,
) -> Result<Option<CompiledLogistics>, String> {
    let local_publications = config
        .logistics
        .as_ref()
        .into_iter()
        .flat_map(|profile| &profile.stores)
        .filter(|store| store.role == phoenix_model::logistics::StoreRole::Local)
        .filter_map(|store| store.system.as_ref())
        .map(phoenix_model::logistics_ui::stock_publication)
        .collect::<std::collections::BTreeSet<_>>();
    if config.ship_config.as_ref().is_some_and(|ship| {
        ship.consoles.iter().any(|console| {
            console.feeds.iter().any(|feed| {
                feed.kind == phoenix_model::console::ConsoleObservationKind::Stock
                    && !local_publications.contains(&feed.source)
            })
        })
    }) {
        return Err("Stock feeds require a configured managed local store owner; remove dangling stock/<SystemId> bindings".into());
    }
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
    if ship
        .systems
        .iter()
        .filter(|system| system.kind == super::logistics::LOGISTICS_KIND)
        .count()
        != 1
    {
        return Err("This build supports exactly one logistics System per [logistics] profile; remove additional unowned handlers".into());
    }
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
        if store.role == phoenix_model::logistics::StoreRole::Local {
            let owner = store.system.as_ref().expect("compiled local owner");
            let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
                .expect("built-in registry");
            if ship
                .system(owner)
                .and_then(|system| registry.descriptor(&system.kind))
                .is_none_or(|kind| !kind.accepts_admitted_commands())
            {
                return Err("a local stock goal needs a commandable owning System".into());
            }
            let publication = phoenix_model::logistics_ui::stock_publication(owner);
            let boards = std::collections::HashMap::from([(
                publication.clone(),
                crate::core::messages::SystemBlackboard::Stock(
                    phoenix_model::logistics_ui::SystemStockBlackboard {
                        system: owner.clone(),
                        stores: Vec::new(),
                    },
                ),
            )]);
            for receiver in [owner, &profile.handling().system] {
                if ship
                    .observations_for_system(receiver, &boards)
                    .get(&publication)
                    .is_none()
                {
                    return Err(format!("System {} needs an explicit typed Stock feed from {} for local goals; co-hosted Consoles do not share knowledge", receiver.0, publication.0));
                }
            }
        }
    }
    profile.validate_stocks(&stocks)?;
    Ok(Some(profile))
}
