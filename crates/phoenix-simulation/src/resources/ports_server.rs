//! Actual dock visits, permitted service terms and one Umbilical work owner.
use crate::core::messages::{SystemBlackboard, SystemId};
use crate::entities::spawner::{EntityUuid, FactionComponent};
use crate::resources::logistics_server::{CompiledShipLogistics, ShipLogistics};
use crate::resources::ports::{
    PortOrderState, PortServiceConfig, PortServicePolicy, PortVisitState,
};
use crate::resources::server::{endpoint_stock, source_available, ResourceServices};
use crate::resources::service::{resolve_endpoint, ResourceEndpoint, ResourceLink};
use crate::umbilical::TransferUmbilical;
use bevy::prelude::*;
use phoenix_model::ports_ui::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Component, Clone, Debug)]
pub struct CompiledPortService(pub PortServiceConfig);
#[derive(Component, Clone, Debug, Default)]
pub struct PortProviderState(pub PortVisitState);

/// Current-tick request only. Quantities and permissions are rechecked by the
/// shared settlement owner; this contains no authoritative inventory.
#[derive(Clone, Debug)]
pub struct PortTransfer {
    pub operator: String,
    pub provider: String,
    pub visit: u64,
    pub offer: String,
    pub destination: ResourceEndpoint,
    pub store: Option<String>,
    pub lots: i64,
    pub tick: u64,
    pub ordered: bool,
    pub inbound: bool,
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<CompiledPortService>(
        StateClass::Derived,
        "automatic-logistics-and-production",
    )
    .declare_state::<PortProviderState>(StateClass::Folded, "automatic-logistics-and-production")
    .init_resource::<crate::effect_queue::EffectQueue<PortTransfer>>()
    .declare_state::<crate::effect_queue::EffectQueue<PortTransfer>>(
        StateClass::ClearedAtFold,
        "automatic-logistics-and-production",
    );
    app.add_systems(
        FixedUpdate,
        prepare
            .in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::infrastructure::tick_infrastructure_condition)
            .after(crate::dock::server::tick_dock)
            .before(crate::umbilical::tick_umbilical),
    );
    app.add_systems(
        FixedUpdate,
        settle
            .in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::resources::server::tick_resource_transfers)
            .before(crate::resources::logistics_server::tick),
    );
}

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(profile) = &config.port_service {
        commands.insert((
            CompiledPortService(profile.clone()),
            PortProviderState::default(),
        ));
    }
}

pub(crate) fn entity_for(world: &World, uuid: &str) -> Option<Entity> {
    world
        .iter_entities()
        .find(|entity| entity.get::<EntityUuid>().is_some_and(|id| id.0 == uuid))
        .map(|entity| entity.id())
}

pub(crate) fn eligible(
    world: &World,
    provider: Entity,
    visitor: Entity,
    profile: &PortServiceConfig,
) -> bool {
    let Some(visitor_faction) = world.get::<FactionComponent>(visitor) else {
        return false;
    };
    if !profile.eligible_factions.contains(&visitor_faction.0) {
        return false;
    }
    let Some(provider_faction) = world.get::<FactionComponent>(provider) else {
        return false;
    };
    !world
        .get_resource::<crate::entities::config_cache::FactionRegistryResource>()
        .is_some_and(|registry| {
            crate::ai::faction::is_enemy(
                Some(provider_faction.0),
                Some(visitor_faction.0),
                &registry.0,
            ) || crate::ai::faction::is_enemy(
                Some(visitor_faction.0),
                Some(provider_faction.0),
                &registry.0,
            )
        })
}

fn stock(
    world: &World,
    entity: Entity,
    endpoint: &ResourceEndpoint,
) -> Option<(crate::resources::Stock, i64)> {
    endpoint_stock(
        endpoint,
        world.get::<crate::infrastructure::InfrastructureCondition>(entity),
        world.get::<crate::console::weapons::torpedo::TorpedoSystemResource>(entity),
    )
}

pub(crate) fn endpoint(
    world: &World,
    entity: Entity,
    resource: &str,
) -> Result<ResourceEndpoint, String> {
    let empty = BTreeMap::new();
    resolve_endpoint(
        resource,
        world
            .get::<ResourceServices>(entity)
            .map(|services| &services.0)
            .unwrap_or(&empty),
        &ResourceLink::default(),
    )
}

fn available(world: &World, entity: Entity, endpoint: &ResourceEndpoint) -> i64 {
    stock(world, entity, endpoint).map_or(0, |(stock, _)| {
        source_available(
            &stock,
            endpoint,
            world.get::<CompiledShipLogistics>(entity),
            world.get::<ShipLogistics>(entity),
        )
    })
}

fn permitted_stores(
    world: &World,
    entity: Entity,
) -> Vec<phoenix_model::logistics_ui::LogisticsStoreView> {
    let (Some(config), Some(boards)) = (
        world.get::<crate::ship_plugin::ShipConfigComponent>(entity),
        world.get::<crate::server_app::ShipSystemBlackboards>(entity),
    ) else {
        return Vec::new();
    };
    let views = config.0.observations_for_system(
        &crate::ship::system_registry::umbilical_system_id(),
        &boards.0,
    );
    let Some(profile) = world.get::<CompiledShipLogistics>(entity) else {
        return Vec::new();
    };
    match views.get(&profile.0.handling().system) {
        Some(SystemBlackboard::Logistics(view)) => view
            .stores
            .iter()
            .filter(|store| store.role == "main")
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

fn actual_store(
    world: &World,
    entity: Entity,
    id: &str,
    resource: &str,
) -> Option<ResourceEndpoint> {
    world
        .get::<CompiledShipLogistics>(entity)?
        .0
        .config()
        .stores
        .iter()
        .find(|store| {
            store.id == id
                && store.resource == resource
                && store.role == phoenix_model::logistics::StoreRole::Main
        })
        .map(|store| store.endpoint.clone())
}

fn receiving_room(world: &World, entity: Entity, endpoint: &ResourceEndpoint) -> i64 {
    let Some((stock, held)) = stock(world, entity, endpoint) else {
        return 0;
    };
    let profile = world.get::<CompiledShipLogistics>(entity);
    let stocks = profile
        .map(|profile| {
            crate::resources::logistics_server::endpoint_view(
                &profile.0,
                world.get::<crate::infrastructure::InfrastructureCondition>(entity),
                world.get::<crate::console::weapons::torpedo::TorpedoSystemResource>(entity),
            )
        })
        .transpose();
    let Ok(stocks) = stocks else { return 0 };
    crate::resources::logistics_server::receiving_room(
        profile,
        endpoint,
        &stocks.unwrap_or_default(),
    )
    .unwrap_or(0)
    .min(stock.headroom().saturating_sub(held))
}

/// All independent pre-existing costs must be affordable. Incoming goods never
/// finance their own barter, and protected Main floors remain unavailable.
fn affordable(
    world: &World,
    provider: Entity,
    visitor: Entity,
    offer: &phoenix_model::ports::PortOfferConfig,
    destination: &ResourceEndpoint,
) -> i64 {
    let Ok(source) = endpoint(world, provider, &offer.resource) else {
        return 0;
    };
    let mut lots = available(world, provider, &source) / offer.unit_amount;
    lots = lots.min(receiving_room(world, visitor, destination) / offer.unit_amount);
    for cost in &offer.costs {
        let Some(profile) = world.get::<CompiledShipLogistics>(visitor) else {
            return 0;
        };
        let Some(store) = profile.0.config().stores.iter().find(|store| {
            store.role == phoenix_model::logistics::StoreRole::Main
                && store.resource == cost.resource
        }) else {
            return 0;
        };
        let Ok(remote) = endpoint(world, provider, &cost.resource) else {
            return 0;
        };
        lots = lots
            .min(available(world, visitor, &store.endpoint) / cost.amount)
            .min(receiving_room(world, provider, &remote) / cost.amount);
    }
    lots.max(0)
}

fn prepare(world: &mut World) {
    let tick = world.resource::<crate::sim_tick::SimTick>().0;
    let dt = world.resource::<Time>().delta_secs();
    let mut providers: Vec<_> = world
        .iter_entities()
        .filter_map(|entity| {
            Some((
                entity.get::<EntityUuid>()?.0.clone(),
                entity.id(),
                entity.get::<CompiledPortService>()?.0.clone(),
            ))
        })
        .collect();
    providers.sort_by(|a, b| a.0.cmp(&b.0));
    for (uuid, entity, profile) in &providers {
        let visitors: BTreeSet<_> = world
            .iter_entities()
            .filter_map(|visitor| {
                (visitor.get::<crate::dock::DockControl>()?.docked_partner() == Some(uuid.as_str()))
                    .then(|| visitor.get::<EntityUuid>().map(|id| id.0.clone()))
                    .flatten()
            })
            .collect();
        world
            .get_mut::<PortProviderState>(*entity)
            .expect("provider state installed")
            .0
            .refresh(profile, tick, &visitors)
            .expect("actual visits use the fixed-tick frontier");
    }
    let mut operators: Vec<_> = world
        .iter_entities()
        .filter_map(|entity| {
            Some((
                entity.get::<EntityUuid>()?.0.clone(),
                entity.id(),
                entity.get::<TransferUmbilical>()?.clone(),
                entity
                    .get::<crate::dock::DockControl>()?
                    .docked_partner()
                    .map(str::to_owned),
            ))
        })
        .collect();
    operators.sort_by(|a, b| a.0.cmp(&b.0));
    for (uuid, entity, mut tool, partner) in operators {
        tool.port_view = None;
        let Some((partner_uuid, provider, profile)) = providers
            .iter()
            .find(|(id, ..)| Some(id) == partner.as_ref())
        else {
            if let Some(mut current) = world.get_mut::<TransferUmbilical>(entity) {
                current.port_view = None;
            }
            continue;
        };
        let ledger = world.get::<PortProviderState>(*provider).unwrap().0.clone();
        let Some(visit) = ledger.visitors.get(&uuid).map(|visit| visit.visit) else {
            continue;
        };
        let allowed = eligible(world, *provider, entity, profile);
        let stores = permitted_stores(world, entity);
        let offers = profile
            .offers
            .iter()
            .map(|offer| {
                let destination = stores
                    .iter()
                    .find(|store| store.resource == offer.resource)
                    .and_then(|store| actual_store(world, entity, &store.id, &offer.resource));
                PortOfferView {
                    id: offer.id.clone(),
                    resource: offer.resource.clone(),
                    unit_amount: offer.unit_amount,
                    quota_remaining: ledger
                        .remaining(profile, &uuid, visit, &offer.id)
                        .unwrap_or(0),
                    costs: offer.costs.clone(),
                    available_lots: destination.as_ref().map_or(0, |destination| {
                        affordable(world, *provider, entity, offer, destination)
                    }),
                }
            })
            .collect();
        tool.port_view = Some(PortServiceView {
            partner_uuid: partner_uuid.clone(),
            visit,
            policy: profile.policy,
            eligible: allowed,
            offers,
            stores: stores.clone(),
            order: None,
        });
        let mut request = None;
        let mut blocked = None;
        if !allowed {
            blocked = Some("ports.blocked.ineligible");
        } else if tool.running || tool.port_maintenance {
            let ordered = profile.policy == PortServicePolicy::ManualOffer;
            let choice = if ordered {
                if tool.running
                    && tool.port_order.active
                    && tool.port_order.partner.as_ref() == Some(partner_uuid)
                    && tool.port_order.visit == Some(visit)
                {
                    tool.port_order.current_line().map(|(_, line)| {
                        (
                            line.offer.clone(),
                            Some(line.store.clone()),
                            line.requested_lots - line.delivered_lots,
                            false,
                        )
                    })
                } else {
                    blocked = Some("ports.blocked.no_order");
                    None
                }
            } else if tool.port_maintenance {
                stores
                    .iter()
                    .filter_map(|store| {
                        let offer = profile
                            .offers
                            .iter()
                            .find(|offer| offer.resource == store.resource)?;
                        let deficit = store
                            .target
                            .saturating_sub(
                                store
                                    .level
                                    .saturating_add(store.incoming)
                                    .saturating_add(store.held),
                            )
                            .max(0);
                        (deficit >= offer.unit_amount).then(|| {
                            (
                                offer.id.clone(),
                                Some(store.id.clone()),
                                deficit / offer.unit_amount,
                                false,
                            )
                        })
                    })
                    .next()
            } else {
                profile
                    .offers
                    .iter()
                    .find(|offer| offer.resource == tool.selected_resource)
                    .map(|offer| {
                        (
                            offer.id.clone(),
                            tool.selected_store.clone(),
                            i64::MAX / offer.unit_amount,
                            tool.selected_direction
                                == crate::umbilical::flow::UmbilicalDirection::Deliver,
                        )
                    })
            };
            if let Some((offer_id, store, demand, inbound)) = choice {
                let offer = profile.offer(&offer_id).unwrap();
                let destination = store
                    .as_ref()
                    .and_then(|id| actual_store(world, entity, id, &offer.resource))
                    .or_else(|| {
                        (!ordered && !tool.port_maintenance && store.is_none())
                            .then(|| endpoint(world, entity, &offer.resource).ok())
                            .flatten()
                    });
                if let Some(destination) = destination {
                    let quota = ledger
                        .remaining(profile, &uuid, visit, &offer_id)
                        .unwrap_or(0);
                    let limit = if inbound {
                        endpoint(world, *provider, &offer.resource).map_or(0, |remote| {
                            (available(world, entity, &destination) / offer.unit_amount)
                                .min(receiving_room(world, *provider, &remote) / offer.unit_amount)
                        })
                    } else {
                        affordable(world, *provider, entity, offer, &destination).min(quota)
                    }
                    .min(demand);
                    let system = crate::ship::system_registry::umbilical_system_id();
                    let disabled = !world
                        .get::<crate::ship_plugin::ShipSystemControlSources>(entity)
                        .is_some_and(|control| {
                            control.0.entries().any(|(id, _)| id == &system)
                                && control.0.policy_for(&system).coordinate
                        })
                        || crate::ship::compartment_damage::operation_disabled(
                            world.get::<crate::ship::compartment_damage::ShipDamage>(entity),
                            world.get::<crate::entities::spawner::EntitySystemHull>(entity),
                            &system,
                        );
                    let mut power = world
                        .get::<crate::ship::power::ShipPowerSystem>(entity)
                        .cloned();
                    if disabled {
                        blocked = Some("ports.blocked.disabled");
                    } else if let Some(ref mut power) = power {
                        let mut plan = if ordered {
                            tool.port_order.clone()
                        } else {
                            let selected_store =
                                store.clone().unwrap_or_else(|| "named-service".into());
                            let mut plan = tool
                                .port_flow
                                .clone()
                                .filter(|plan| {
                                    plan.partner.as_ref() == Some(partner_uuid)
                                        && plan.visit == Some(visit)
                                        && plan.active
                                        && plan.current_line().is_some_and(|(_, line)| {
                                            line.offer == offer_id && line.store == selected_store
                                        })
                                })
                                .unwrap_or_default();
                            if plan.partner.is_none() && limit > 0 {
                                let _ = plan.approve(
                                    profile,
                                    partner_uuid,
                                    visit,
                                    vec![phoenix_model::ports::PortOrderLine {
                                        offer: offer_id.clone(),
                                        store: selected_store,
                                        lots: limit.min(offer.quota),
                                    }],
                                );
                            }
                            plan
                        };
                        if limit > 0 {
                            if let Ok(lots) = plan.advance(
                                profile,
                                &crate::ship::system_registry::umbilical_system_id(),
                                tick,
                                dt,
                                tool.config.rate,
                                limit,
                                |system, seconds| {
                                    power
                                        .0
                                        .work_continuous(system, seconds, 1.0)
                                        .map(|grant| grant.fraction)
                                        .map_err(|error| error.to_string())
                                },
                            ) {
                                if ordered {
                                    tool.port_order = plan;
                                } else {
                                    tool.port_flow = Some(plan);
                                }
                                if let Some(mut actual) =
                                    world.get_mut::<crate::ship::power::ShipPowerSystem>(entity)
                                {
                                    *actual = power.clone();
                                }
                                if lots > 0 {
                                    request = Some(PortTransfer {
                                        operator: uuid.clone(),
                                        provider: partner_uuid.clone(),
                                        visit,
                                        offer: offer_id,
                                        destination,
                                        store,
                                        lots,
                                        tick,
                                        ordered,
                                        inbound,
                                    });
                                }
                            }
                        } else {
                            blocked = Some(if quota == 0 && !inbound {
                                "ports.blocked.quota_exhausted"
                            } else {
                                "ports.blocked.no_stock"
                            });
                        }
                    } else {
                        blocked = Some("ports.blocked.unpowered");
                    }
                } else {
                    blocked = Some("ports.blocked.invalid_store");
                }
            } else if blocked.is_none() {
                blocked = Some("ports.blocked.no_demand");
            }
        }
        if let Some(view) = &mut tool.port_view {
            if tool.port_order.partner.is_some() {
                view.order = Some(PortOrderView {
                    active: tool.port_order.active,
                    lines: tool
                        .port_order
                        .lines
                        .iter()
                        .map(|line| PortOrderLineView {
                            offer: line.offer.clone(),
                            store: line.store.clone(),
                            requested_lots: line.requested_lots,
                            delivered_lots: line.delivered_lots,
                        })
                        .collect(),
                    blocked: blocked.map(str::to_owned),
                });
            }
        }
        *world.get_mut::<TransferUmbilical>(entity).unwrap() = tool;
        if let Some(request) = request {
            world
                .resource_mut::<crate::effect_queue::EffectQueue<PortTransfer>>()
                .0
                .push(request);
        }
    }
}

fn settle(world: &mut World) {
    let mut requests = std::mem::take(
        &mut world
            .resource_mut::<crate::effect_queue::EffectQueue<PortTransfer>>()
            .0,
    );
    requests.sort_by(|a, b| {
        (&a.operator, &a.provider, &a.offer).cmp(&(&b.operator, &b.provider, &b.offer))
    });
    for request in requests {
        let moved = crate::resources::server::settle_port_exchange(world, &request).unwrap_or(0);
        if let Some(entity) = entity_for(world, &request.operator) {
            let mut tool = world.get_mut::<TransferUmbilical>(entity).unwrap();
            if request.ordered {
                tool.port_order
                    .record_receipt(request.tick, moved)
                    .expect("actual settled lot receipt matches its one work offer");
            } else {
                tool.port_flow
                    .as_mut()
                    .expect("free flow owns its work frontier")
                    .record_receipt(request.tick, moved)
                    .expect("actual free-flow receipt matches paid work");
            }
            let lines = tool
                .port_order
                .lines
                .iter()
                .map(|line| PortOrderLineView {
                    offer: line.offer.clone(),
                    store: line.store.clone(),
                    requested_lots: line.requested_lots,
                    delivered_lots: line.delivered_lots,
                })
                .collect();
            let active = tool.port_order.active;
            if let Some(order) = tool.port_view.as_mut().and_then(|view| view.order.as_mut()) {
                order.active = active;
                order.lines = lines;
            }
        }
    }
}
