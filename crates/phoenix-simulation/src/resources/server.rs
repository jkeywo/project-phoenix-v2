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
