//! Actual dock visits, permitted service terms and one Umbilical work owner.
use crate::core::messages::SystemBlackboard;
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
    /// Legacy: completed work offer. Equipment-bound provider: upper limit for
    /// the current next work step, paid atomically by the settlement owner.
    pub lots: i64,
    pub tick: u64,
    pub ordered: bool,
    pub inbound: bool,
    pub maintain_goal: bool,
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<CompiledPortService>(
        StateClass::Derived,
        "authorised-port-stock-target-replenishment",
    )
    .declare_state::<PortProviderState>(
        StateClass::Folded,
        "authorised-port-stock-target-replenishment",
    )
    .init_resource::<crate::effect_queue::EffectQueue<PortTransfer>>()
    .declare_state::<crate::effect_queue::EffectQueue<PortTransfer>>(
        StateClass::ClearedAtFold,
        "authorised-port-stock-target-replenishment",
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

/// Shared immutable binding/visit validation for Human admission and the
/// explicitly trusted World action. This does not infer approval from tokens.
#[allow(clippy::too_many_arguments)]
pub(crate) fn approved_order(
    profile: &PortServiceConfig,
    ledger: &PortVisitState,
    visitor: &str,
    provider: &str,
    visit: u64,
    lines: Vec<phoenix_model::ports::PortOrderLine>,
    stores: Option<&CompiledShipLogistics>,
    previous: &PortOrderState,
) -> Result<PortOrderState, String> {
    if profile.policy != PortServicePolicy::ManualOffer {
        return Err("explicit port orders require manual terms".into());
    }
    let stores = stores.ok_or("manual trade requires actual Main destination and cost stores")?;
    for line in &lines {
        let offer = profile.offer(&line.offer).ok_or("unknown port offer")?;
        if !stores.0.config().stores.iter().any(|store| {
            store.id == line.store
                && store.role == phoenix_model::logistics::StoreRole::Main
                && store.resource == offer.resource
        }) || !offer.costs.iter().all(|cost| {
            stores.0.config().stores.iter().any(|store| {
                store.role == phoenix_model::logistics::StoreRole::Main
                    && store.resource == cost.resource
            })
        }) || line.lots > ledger.remaining(profile, visitor, visit, &line.offer)?
        {
            return Err(
                "port order requires current quota and actual compatible Main stock bindings"
                    .into(),
            );
        }
    }
    let mut next = previous.clone();
    next.approve(profile, provider, visit, lines)?;
    Ok(next)
}

/// Only explicitly trusted World actions call this at their ordinary effect
/// flush. Client controls
/// first pass ordinary System/Console admission and share approved_order above.
pub fn approve_port_order(
    world: &mut World,
    visitor: &str,
    provider: &str,
    visit: u64,
    lines: Vec<phoenix_model::ports::PortOrderLine>,
) -> Result<(), String> {
    let ship = entity_for(world, visitor).ok_or("port visitor is missing")?;
    let facility = entity_for(world, provider).ok_or("port provider is missing")?;
    if world
        .get::<crate::dock::DockControl>(ship)
        .and_then(crate::dock::DockControl::docked_partner)
        != Some(provider)
    {
        return Err("port order belongs to a different physical partner".into());
    }
    let profile = &world
        .get::<CompiledPortService>(facility)
        .ok_or("provider has no trade terms")?
        .0;
    if !eligible(world, facility, ship, profile) {
        return Err("visitor lacks current port permission".into());
    }
    let tool = world
        .get::<TransferUmbilical>(ship)
        .ok_or("visitor has no Umbilical")?;
    if tool.running {
        return Err("stop ordinary transfer before approving a trade order".into());
    }
    let next = approved_order(
        profile,
        &world
            .get::<PortProviderState>(facility)
            .ok_or("missing visit ledger")?
            .0,
        visitor,
        provider,
        visit,
        lines,
        world.get::<CompiledShipLogistics>(ship),
        &tool.port_order,
    )?;
    let mut tool = world.get_mut::<TransferUmbilical>(ship).unwrap();
    tool.port_order = next;
    tool.running = true;
    tool.carry = 0.0;
    Ok(())
}

pub(crate) fn entity_for(world: &mut World, uuid: &str) -> Option<Entity> {
    world
        .query::<(Entity, &EntityUuid)>()
        .iter(world)
        .find(|(_, id)| id.0 == uuid)
        .map(|(entity, _)| entity)
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
    world: &mut World,
    provider: Entity,
    visitor: Entity,
    offer: &phoenix_model::ports::PortOfferConfig,
    destination: &ResourceEndpoint,
) -> i64 {
    let (Some(operator), Some(provider_id)) = (
        world.get::<EntityUuid>(visitor).map(|id| id.0.clone()),
        world.get::<EntityUuid>(provider).map(|id| id.0.clone()),
    ) else {
        return 0;
    };
    let Some(visit) = world
        .get::<PortProviderState>(provider)
        .and_then(|state| state.0.visitors.get(&operator))
        .map(|visit| visit.visit)
    else {
        return 0;
    };
    let store = world
        .get::<CompiledShipLogistics>(visitor)
        .and_then(|profile| {
            profile.0.config().stores.iter().find(|store| {
                store.endpoint == *destination
                    && store.resource == offer.resource
                    && store.role == phoenix_model::logistics::StoreRole::Main
            })
        })
        .map(|store| store.id.clone());
    let ordered = world
        .get::<CompiledPortService>(provider)
        .is_some_and(|profile| profile.0.policy == PortServicePolicy::ManualOffer);
    let request = PortTransfer {
        operator,
        provider: provider_id,
        visit,
        offer: offer.id.clone(),
        destination: destination.clone(),
        store,
        lots: offer.quota,
        tick: world.resource::<crate::sim_tick::SimTick>().0,
        ordered,
        inbound: false,
        maintain_goal: false,
    };
    crate::resources::server::preview_port_exchange(world, &request).unwrap_or(0)
}

fn prepare(world: &mut World) {
    let tick = world.resource::<crate::sim_tick::SimTick>().0;
    let dt = world.resource::<Time>().delta_secs();
    let mut providers: Vec<_> = world
        .query::<(Entity, &EntityUuid, &CompiledPortService)>()
        .iter(world)
        .map(|(entity, uuid, profile)| (uuid.0.clone(), entity, profile.0.clone()))
        .collect();
    providers.sort_by(|a, b| a.0.cmp(&b.0));
    for (uuid, entity, profile) in &providers {
        let visitors: BTreeSet<_> = world
            .query::<(&EntityUuid, &crate::dock::DockControl)>()
            .iter(world)
            .filter(|(_, dock)| dock.docked_partner() == Some(uuid.as_str()))
            .map(|(id, _)| id.0.clone())
            .collect();
        world
            .get_mut::<PortProviderState>(*entity)
            .expect("provider state installed")
            .0
            .refresh(profile, tick, &visitors)
            .expect("actual visits use the fixed-tick frontier");
    }
    let mut operators: Vec<_> = world
        .query::<(
            Entity,
            &EntityUuid,
            &TransferUmbilical,
            &crate::dock::DockControl,
        )>()
        .iter(world)
        .map(|(entity, id, tool, dock)| {
            (
                id.0.clone(),
                entity,
                tool.clone(),
                dock.docked_partner().map(str::to_owned),
            )
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
                current.port_order = PortOrderState::default();
                current.port_flow = None;
            }
            continue;
        };
        let ledger = world.get::<PortProviderState>(*provider).unwrap().0.clone();
        let Some(visit) = ledger.visitors.get(&uuid).map(|visit| visit.visit) else {
            continue;
        };
        if tool
            .port_order
            .partner
            .as_ref()
            .is_some_and(|partner| partner != partner_uuid)
            || tool.port_order.visit.is_some_and(|saved| saved != visit)
        {
            tool.port_order = PortOrderState::default();
        }
        if tool.port_flow.as_ref().is_some_and(|flow| {
            flow.partner.as_ref() != Some(partner_uuid) || flow.visit != Some(visit)
        }) {
            tool.port_flow = None;
        }
        let allowed = eligible(world, *provider, entity, profile);
        let mut stores = permitted_stores(world, entity);
        stores.sort_by(|a, b| (a.priority, &a.id).cmp(&(b.priority, &b.id)));
        let offers = profile
            .offers
            .iter()
            .map(|offer| {
                let destinations: Vec<_> = stores
                    .iter()
                    .filter(|store| store.resource == offer.resource)
                    .filter_map(|store| actual_store(world, entity, &store.id, &offer.resource))
                    .collect();
                PortOfferView {
                    id: offer.id.clone(),
                    resource: offer.resource.clone(),
                    unit_amount: offer.unit_amount,
                    quota_remaining: ledger
                        .remaining(profile, &uuid, visit, &offer.id)
                        .unwrap_or(0),
                    costs: offer.costs.clone(),
                    available_lots: if allowed {
                        destinations
                            .iter()
                            .map(|destination| {
                                affordable(world, *provider, entity, offer, destination)
                            })
                            .max()
                            .unwrap_or(0)
                    } else {
                        0
                    },
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
                        if !tool.port_view.as_ref()?.offers.iter().any(|visible| {
                            visible.id == offer.id
                                && visible.quota_remaining > 0
                                && visible.available_lots > 0
                        }) || store.headroom < offer.unit_amount
                        {
                            return None;
                        }
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
                    let efficiency = world
                        .get::<crate::ship::compartment_damage::ShipDamage>(entity)
                        .map_or(1.0, |damage| {
                            damage.factor(&system, phoenix_model::damage::DamageBinding::Operation)
                        });
                    if disabled {
                        blocked = Some("ports.blocked.disabled");
                    } else if profile.system.is_some() {
                        if limit > 0 {
                            request = Some(PortTransfer {
                                operator: uuid.clone(),
                                provider: partner_uuid.clone(),
                                visit,
                                offer: offer_id,
                                destination,
                                store,
                                lots: limit,
                                tick,
                                ordered,
                                inbound,
                                maintain_goal: !ordered && tool.port_maintenance,
                            });
                        } else {
                            blocked = Some("ports.blocked.no_stock");
                        }
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
                                if plan
                                    .approve(
                                        profile,
                                        partner_uuid,
                                        visit,
                                        vec![phoenix_model::ports::PortOrderLine {
                                            offer: offer_id.clone(),
                                            store: selected_store,
                                            lots: limit.min(offer.quota),
                                        }],
                                    )
                                    .is_err()
                                {
                                    blocked = Some("ports.blocked.quota_exhausted");
                                }
                            }
                            plan
                        };
                        if limit > 0 && plan.partner.is_some() {
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
                                        .map(|grant| grant.fraction * efficiency)
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
                                        maintain_goal: !ordered && tool.port_maintenance,
                                    });
                                }
                            }
                        } else if blocked.is_none() {
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
            if tool.port_order.partner.is_some() || blocked.is_some() {
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
        if entity_for(world, &request.provider)
            .and_then(|entity| world.get::<CompiledPortService>(entity))
            .is_some_and(|profile| profile.0.system.is_some())
        {
            let result = super::ports_equipment::settle(world, &request);
            super::ports_equipment::update_view(world, &request, result.err());
            continue;
        }
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

#[cfg(test)]
#[path = "ports_server_tests.rs"]
mod tests;
