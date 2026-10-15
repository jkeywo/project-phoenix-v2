//! Transactional stock movement. Capacity publication remains infrastructure-owned.
use bevy::prelude::*;
use std::collections::BTreeMap;

use crate::console::weapons::torpedo::TorpedoSystemResource;
use crate::core::messages::SystemId;
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::EntityUuid;
use crate::infrastructure::InfrastructureCondition;
use crate::resources::logistics_server::{CompiledShipLogistics, ShipLogistics};
use crate::resources::service::ResourceEndpoint;
use crate::resources::{plan_transfer, Stock};

/// Correlates a producer's current fixed-tick request with its settled outcome.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransferReceiptKey {
    pub system: SystemId,
    pub tick: u64,
}

/// Ephemeral settlement feedback, consumed before the same tick's fold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceTransferReceipt {
    pub operator: String,
    pub key: TransferReceiptKey,
    pub source: String,
    pub destination: String,
    pub requested: i64,
    pub moved: i64,
}

/// Authored, rederived bindings. They never carry a second inventory.
#[derive(Component, Clone, Debug, Default)]
pub struct ResourceServices(pub BTreeMap<String, ResourceEndpoint>);

/// An admitted physical flow's next stock transaction, drained before publication.
#[derive(Clone, Debug)]
pub struct ResourceTransfer {
    pub operator: String,
    pub source: String,
    pub source_endpoint: ResourceEndpoint,
    pub destination: String,
    pub destination_endpoint: ResourceEndpoint,
    pub amount: i64,
    pub receipt: Option<TransferReceiptKey>,
}

pub(crate) fn source_available(
    stock: &Stock,
    endpoint: &ResourceEndpoint,
    profile: Option<&CompiledShipLogistics>,
    state: Option<&ShipLogistics>,
) -> i64 {
    let main = profile.and_then(|profile| {
        profile.0.config().stores.iter().find(|store| {
            store.endpoint == *endpoint && store.role == phoenix_model::logistics::StoreRole::Main
        })
    });
    let Some(store) = main else {
        return stock.available();
    };
    let Some(floor) = state
        .and_then(|state| state.0.floors.get(&store.id))
        .copied()
    else {
        return 0;
    };
    if floor < 0 || floor > stock.ceiling {
        return 0;
    }
    stock
        .available()
        .min(stock.level.saturating_sub(floor).max(0))
}

/// Snapshot the relevant stock for one transaction. Tube-held space stays occupied.
pub fn endpoint_stock(
    endpoint: &ResourceEndpoint,
    infrastructure: Option<&InfrastructureCondition>,
    torpedoes: Option<&TorpedoSystemResource>,
) -> Option<(Stock, i64)> {
    match endpoint {
        ResourceEndpoint::Capacity { id } => infrastructure
            .and_then(|state| state.0.capacity_stock(id))
            .map(|stock| (stock.clone(), 0)),
        ResourceEndpoint::TorpedoMagazine {} => torpedoes.map(|system| {
            let held = system.0.magazine.headroom() - system.0.magazine_headroom();
            (system.0.magazine.clone(), held)
        }),
    }
}

/// Settle after ordinary capacity effects, committing both actual owners before
/// issuing a receipt. No intermediate quantity survives this transaction.
#[allow(clippy::type_complexity)]
pub fn tick_resource_transfers(
    mut transfers: ResMut<EffectQueue<ResourceTransfer>>,
    mut receipts: ResMut<EffectQueue<ResourceTransferReceipt>>,
    mut runtime: ResMut<crate::world::server::WorldContentRuntime>,
    mut endpoints: Query<(
        &EntityUuid,
        Option<&mut InfrastructureCondition>,
        Option<&mut TorpedoSystemResource>,
        Option<&CompiledShipLogistics>,
        Option<&ShipLogistics>,
    )>,
) {
    let mut requests = std::mem::take(&mut transfers.0);
    requests.sort_by(|a, b| {
        (
            &a.operator,
            &a.source,
            &a.source_endpoint,
            &a.destination,
            &a.destination_endpoint,
            &a.receipt,
            a.amount,
        )
            .cmp(&(
                &b.operator,
                &b.source,
                &b.source_endpoint,
                &b.destination,
                &b.destination_endpoint,
                &b.receipt,
                b.amount,
            ))
    });
    for request in requests {
        let moved = (|| -> Result<i64, String> {
            if request.source == request.destination
                && request.source_endpoint == request.destination_endpoint
            {
                return Ok(0);
            }
            let (_, source_inventory, source_torpedoes, source_profile, source_state) = endpoints
                .iter()
                .find(|(id, ..)| id.0 == request.source)
                .ok_or("missing source")?;
            let (mut source, _) =
                endpoint_stock(&request.source_endpoint, source_inventory, source_torpedoes)
                    .ok_or("missing source endpoint")?;
            let available = source_available(
                &source,
                &request.source_endpoint,
                source_profile,
                source_state,
            );
            let (_, inventory, torpedoes, profile, _) = endpoints
                .iter()
                .find(|(id, ..)| id.0 == request.destination)
                .ok_or("missing destination")?;
            let (mut destination, held) =
                endpoint_stock(&request.destination_endpoint, inventory, torpedoes)
                    .ok_or("missing destination endpoint")?;
            let destination_stocks = match profile {
                Some(profile) => crate::resources::logistics_server::endpoint_view(
                    &profile.0, inventory, torpedoes,
                )?,
                None => BTreeMap::new(),
            };
            let room = crate::resources::logistics_server::receiving_room(
                profile,
                &request.destination_endpoint,
                &destination_stocks,
            )?;
            let moved = plan_transfer(
                request.amount,
                available,
                destination.headroom().saturating_sub(held).min(room),
            );
            if moved == 0 {
                return Ok(0);
            }
            if source.adjust(-moved) != -moved || destination.adjust(moved) != moved {
                return Err("endpoint quantities changed during staging".into());
            }
            let mut updates: BTreeMap<String, BTreeMap<ResourceEndpoint, Stock>> = BTreeMap::new();
            updates
                .entry(request.source.clone())
                .or_default()
                .insert(request.source_endpoint.clone(), source);
            updates
                .entry(request.destination.clone())
                .or_default()
                .insert(request.destination_endpoint.clone(), destination);
            let mut staged = Vec::new();
            // Validate every affected owner before writing either. Combining
            // by UUID also handles two different endpoints on the same ship.
            for (uuid, updates) in updates {
                let (_, inventory, torpedoes, profile, _) = endpoints
                    .iter()
                    .find(|(id, ..)| id.0 == uuid)
                    .ok_or("endpoint owner vanished")?;
                let next = crate::infrastructure::server::stage_resource_endpoints(
                    inventory, torpedoes, &updates,
                )?;
                if let Some(profile) = profile {
                    let stocks = crate::resources::logistics_server::endpoint_view(
                        &profile.0,
                        next.0.as_ref(),
                        next.1.as_ref(),
                    )?;
                    profile.0.validate_stocks(&stocks)?;
                }
                staged.push((uuid, next));
            }
            for (uuid, (inventory, torpedoes, flags)) in staged {
                let (_, actual_inventory, actual_torpedoes, _, _) = endpoints
                    .iter_mut()
                    .find(|(id, ..)| id.0 == uuid)
                    .expect("staged owner remains present");
                if let (Some(mut actual), Some(next)) = (actual_inventory, inventory) {
                    *actual = next;
                    for capacity in actual.0.capacities() {
                        runtime.flags.set_flag_value(&capacity.id, capacity.level);
                    }
                }
                if let (Some(mut actual), Some(next)) = (actual_torpedoes, torpedoes) {
                    *actual = next;
                }
                crate::infrastructure::server::mirror_flags(&mut runtime, &flags, &uuid, &None);
            }
            Ok(moved)
        })()
        .unwrap_or(0);
        if let Some(key) = request.receipt {
            receipts.0.push(ResourceTransferReceipt {
                operator: request.operator,
                key,
                source: request.source,
                destination: request.destination,
                requested: request.amount,
                moved,
            });
        }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;

/// One authoritative exchange transaction: all goods/cost endpoints and the
/// provider's visit quota commit together, after ordinary capacity effects.
/// No incoming goods can satisfy a pre-existing cost or bypass a Main floor.
pub(crate) fn settle_port_exchange(
    world: &mut World,
    request: &crate::resources::ports_server::PortTransfer,
) -> Result<i64, String> {
    port_exchange(world, request, true)
}

/// Quote the exact final-owner transaction before buying machinery time.
/// This stages the same finite stocks and quota but performs no live writes.
pub(crate) fn preview_port_exchange(
    world: &mut World,
    request: &crate::resources::ports_server::PortTransfer,
) -> Result<i64, String> {
    port_exchange(world, request, false)
}

fn port_exchange(
    world: &mut World,
    request: &crate::resources::ports_server::PortTransfer,
    commit: bool,
) -> Result<i64, String> {
    if !world.contains_resource::<crate::world::server::WorldContentRuntime>() {
        return Err("port settlement requires the actual infrastructure publication owner".into());
    }
    use crate::resources::ports_server::{
        eligible, endpoint, entity_for, CompiledPortService, PortProviderState,
    };
    let provider = entity_for(world, &request.provider).ok_or("port provider vanished")?;
    let visitor = entity_for(world, &request.operator).ok_or("visitor vanished")?;
    if provider == visitor
        || world
            .get::<crate::dock::DockControl>(visitor)
            .and_then(crate::dock::DockControl::docked_partner)
            != Some(request.provider.as_str())
    {
        return Err("port exchange requires the current physical partner".into());
    }
    let profile = world
        .get::<CompiledPortService>(provider)
        .ok_or("provider terms missing")?
        .0
        .clone();
    if !eligible(world, provider, visitor, &profile) {
        return Err("port visitor is ineligible".into());
    }
    let mut ledger = world
        .get::<PortProviderState>(provider)
        .ok_or("port quota owner missing")?
        .0
        .clone();
    let quota = ledger.remaining(&profile, &request.operator, request.visit, &request.offer)?;
    let offer = profile.offer(&request.offer).ok_or("unknown port offer")?;
    if offer.quota == 0 {
        return Err("zero-quota port offer is disabled in both directions".into());
    }
    if request.inbound
        && profile.policy != phoenix_model::ports::PortServicePolicy::FreeAlliedReplenishment
    {
        return Err("manual port cannot accept legacy purchase flow".into());
    }
    if commit && profile.policy == phoenix_model::ports::PortServicePolicy::ManualOffer {
        let order = &world
            .get::<crate::umbilical::TransferUmbilical>(visitor)
            .ok_or("missing Umbilical")?
            .port_order;
        let (_, line) = order
            .current_line()
            .ok_or("manual trade lacks approved order")?;
        if !request.ordered
            || !order.active
            || order.partner.as_ref() != Some(&request.provider)
            || order.visit != Some(request.visit)
            || line.offer != request.offer
            || Some(&line.store) != request.store.as_ref()
            || request.lots > line.requested_lots - line.delivered_lots
        {
            return Err("manual trade differs from approved visit-bound order".into());
        }
    }
    if let Some(store) = &request.store {
        let profile = world
            .get::<CompiledShipLogistics>(visitor)
            .ok_or("missing receiving store profile")?;
        if !profile.0.config().stores.iter().any(|entry| {
            entry.id == *store
                && entry.role == phoenix_model::logistics::StoreRole::Main
                && entry.resource == offer.resource
                && entry.endpoint == request.destination
        }) {
            return Err("port destination is not the actual compatible Main store".into());
        }
    } else if request.ordered || endpoint(world, visitor, &offer.resource)? != request.destination {
        return Err("legacy port endpoint differs from the named service".into());
    }
    let remote = endpoint(world, provider, &offer.resource)?;
    let (source, source_endpoint, destination, destination_endpoint) = if request.inbound {
        (visitor, request.destination.clone(), provider, remote)
    } else {
        (provider, remote, visitor, request.destination.clone())
    };
    let legs = vec![(
        source,
        source_endpoint,
        destination,
        destination_endpoint,
        offer.unit_amount,
    )];
    let mut cost_plans = Vec::new();
    for cost in &offer.costs {
        let stores = world
            .get::<CompiledShipLogistics>(visitor)
            .ok_or("barter requires actual Main cost stores")?;
        let receiving_compartment = stores
            .0
            .config()
            .stores
            .iter()
            .find(|store| store.endpoint == request.destination)
            .map(|store| store.compartment.as_str());
        let mut local: Vec<_> = stores
            .0
            .config()
            .stores
            .iter()
            .filter(|store| {
                store.role == phoenix_model::logistics::StoreRole::Main
                    && store.resource == cost.resource
            })
            .map(|store| {
                (
                    if store.endpoint == request.destination {
                        0
                    } else if Some(store.compartment.as_str()) == receiving_compartment {
                        1
                    } else {
                        2
                    },
                    store.id.clone(),
                    store.endpoint.clone(),
                )
            })
            .collect();
        local.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        if local.is_empty() {
            return Err("barter cost has no compatible Main store".into());
        }
        let mut sources = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (_, _, endpoint) in local {
            if !seen.insert(endpoint.clone()) {
                continue;
            }
            let (stock, _) = endpoint_stock(
                &endpoint,
                world.get::<InfrastructureCondition>(visitor),
                world.get::<TorpedoSystemResource>(visitor),
            )
            .ok_or("missing actual barter cost endpoint")?;
            let available = source_available(
                &stock,
                &endpoint,
                world.get::<CompiledShipLogistics>(visitor),
                world.get::<ShipLogistics>(visitor),
            );
            sources.push((endpoint, available));
        }
        cost_plans.push((
            endpoint(world, provider, &cost.resource)?,
            cost.amount,
            sources,
        ));
    }
    let mut limit = request.lots.max(0);
    if !request.inbound {
        limit = limit.min(quota);
    }
    if request.maintain_goal {
        let store_id = request
            .store
            .as_ref()
            .ok_or("maintained port flow requires an explicit Main store")?;
        let state = world
            .get::<ShipLogistics>(visitor)
            .ok_or("maintained port flow requires actual stock targets")?;
        let goal = state
            .0
            .goals
            .get(store_id)
            .copied()
            .ok_or("maintained store has no actual goal")?;
        let (stock, held) = endpoint_stock(
            &request.destination,
            world.get::<InfrastructureCondition>(visitor),
            world.get::<TorpedoSystemResource>(visitor),
        )
        .ok_or("maintained destination vanished")?;
        let occupied = stock
            .level
            .saturating_add(stock.incoming_total())
            .saturating_add(held);
        limit = limit.min(goal.saturating_sub(occupied).max(0) / offer.unit_amount);
    }
    for (_, amount, sources) in &cost_plans {
        let total: i128 = sources
            .iter()
            .map(|(_, available)| i128::from(*available))
            .sum();
        limit = limit.min(
            i64::try_from((total / i128::from(*amount)).min(i128::from(i64::MAX)))
                .map_err(|_| "barter availability overflow")?,
        );
    }
    // Each source is checked before any credit. Multiple cost/goods legs using
    // one endpoint share this exact independent available amount.
    let mut requirements: BTreeMap<(Entity, ResourceEndpoint), i64> = BTreeMap::new();
    for (source, endpoint, .., amount) in &legs {
        let total = requirements.entry((*source, endpoint.clone())).or_default();
        *total = total.checked_add(*amount).ok_or("port cost overflow")?;
    }
    for ((entity, endpoint), per_lot) in &requirements {
        let (stock, _) = endpoint_stock(
            endpoint,
            world.get::<InfrastructureCondition>(*entity),
            world.get::<TorpedoSystemResource>(*entity),
        )
        .ok_or("missing actual port source stock")?;
        let available = source_available(
            &stock,
            endpoint,
            world.get::<CompiledShipLogistics>(*entity),
            world.get::<ShipLogistics>(*entity),
        );
        limit = limit.min(available / per_lot);
    }
    // Final shared-space validity is checked on whole owner snapshots. A cost
    // withdrawal can make room for its purchased output, but never for itself.
    let stage = |lots: i64| -> Result<Vec<_>, String> {
        let mut updates: BTreeMap<Entity, BTreeMap<ResourceEndpoint, Stock>> = BTreeMap::new();
        let mut deltas: BTreeMap<(Entity, ResourceEndpoint), i128> = BTreeMap::new();
        for (source, source_endpoint, destination, destination_endpoint, per_lot) in &legs {
            let amount = per_lot
                .checked_mul(lots)
                .ok_or("port lot product overflow")?;
            for (entity, endpoint, delta) in [
                (*source, source_endpoint, -amount),
                (*destination, destination_endpoint, amount),
            ] {
                let total = deltas.entry((entity, endpoint.clone())).or_default();
                *total = total
                    .checked_add(i128::from(delta))
                    .ok_or("port endpoint delta overflow")?;
            }
        }
        for (remote, per_lot, sources) in &cost_plans {
            let mut needed = per_lot.checked_mul(lots).ok_or("barter cost overflow")?;
            for (local, available) in sources {
                let amount = needed.min(*available);
                if amount > 0 {
                    *deltas.entry((visitor, local.clone())).or_default() -= i128::from(amount);
                    *deltas.entry((provider, remote.clone())).or_default() += i128::from(amount);
                    needed -= amount;
                }
            }
            if needed != 0 {
                return Err("barter cost cannot be funded by pre-existing available stock".into());
            }
        }
        for ((entity, endpoint), delta) in deltas {
            let delta = i64::try_from(delta).map_err(|_| "port net endpoint delta overflow")?;
            let (mut stock, _) = endpoint_stock(
                &endpoint,
                world.get::<InfrastructureCondition>(entity),
                world.get::<TorpedoSystemResource>(entity),
            )
            .ok_or("port stock owner missing")?;
            if stock.adjust(delta) != delta {
                return Err("port endpoint cannot fit the complete lot".into());
            }
            updates.entry(entity).or_default().insert(endpoint, stock);
        }
        let mut staged = Vec::new();
        for (entity, updates) in updates {
            let next = crate::infrastructure::server::stage_resource_endpoints(
                world.get::<InfrastructureCondition>(entity),
                world.get::<TorpedoSystemResource>(entity),
                &updates,
            )?;
            if let Some(profile) = world.get::<CompiledShipLogistics>(entity) {
                let stocks = crate::resources::logistics_server::endpoint_view(
                    &profile.0,
                    next.0.as_ref(),
                    next.1.as_ref(),
                )?;
                profile.0.validate_stocks(&stocks)?;
            }
            staged.push((entity, next));
        }
        staged.sort_by(|(a, _), (b, _)| {
            world
                .get::<EntityUuid>(*a)
                .map(|id| &id.0)
                .cmp(&world.get::<EntityUuid>(*b).map(|id| &id.0))
        });
        Ok(staged)
    };
    let (mut low, mut high) = (0, limit);
    while low < high {
        let mid = low + (high - low) / 2 + 1;
        if stage(mid).is_ok() {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    if low == 0 {
        return Ok(0);
    }
    if !commit {
        return Ok(low);
    }
    let staged = stage(low)?;
    if !request.inbound {
        ledger.record_delivery(
            &profile,
            &request.operator,
            request.visit,
            &request.offer,
            low,
        )?;
    }
    for (entity, (inventory, torpedoes, flags)) in staged {
        if let Some(next) = inventory {
            let counters: Vec<_> = next
                .0
                .capacities()
                .iter()
                .map(|capacity| (capacity.id.clone(), capacity.level))
                .collect();
            *world
                .get_mut::<InfrastructureCondition>(entity)
                .ok_or("staged infrastructure owner vanished")? = next;
            let mut runtime = world.resource_mut::<crate::world::server::WorldContentRuntime>();
            for (id, level) in counters {
                runtime.flags.set_flag_value(&id, level);
            }
        }
        if let Some(next) = torpedoes {
            *world
                .get_mut::<TorpedoSystemResource>(entity)
                .ok_or("staged magazine vanished")? = next;
        }
        let uuid = world
            .get::<EntityUuid>(entity)
            .ok_or("staged identity vanished")?
            .0
            .clone();
        crate::infrastructure::server::mirror_flags(
            &mut world.resource_mut::<crate::world::server::WorldContentRuntime>(),
            &flags,
            &uuid,
            &None,
        );
    }
    world
        .get_mut::<PortProviderState>(provider)
        .expect("staged provider remains")
        .0 = ledger;
    Ok(low)
}
