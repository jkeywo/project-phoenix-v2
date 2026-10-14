//! Transactional stock movement. Capacity publication remains infrastructure-owned.
use bevy::prelude::*;
use std::collections::BTreeMap;

use crate::console::weapons::torpedo::TorpedoSystemResource;
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::EntityUuid;
use crate::infrastructure::{CapacityAdjustment, InfrastructureCondition};
use crate::resources::service::ResourceEndpoint;
use crate::resources::{plan_transfer, Stock};

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
        ResourceEndpoint::TorpedoMagazine => torpedoes.map(|system| {
            let held = system.0.magazine.headroom() - system.0.magazine_headroom();
            (system.0.magazine.clone(), held)
        }),
    }
}

/// Reserve and commit a whole transfer against the current transaction view.
/// No canonical quantity is changed until both endpoints are available.
pub fn tick_resource_transfers(
    mut transfers: ResMut<EffectQueue<ResourceTransfer>>,
    mut capacities: ResMut<EffectQueue<CapacityAdjustment>>,
    mut endpoints: Query<(
        &EntityUuid,
        Option<&InfrastructureCondition>,
        Option<&mut TorpedoSystemResource>,
    )>,
) {
    let mut requests = std::mem::take(&mut transfers.0);
    if requests.is_empty() {
        return;
    }
    requests.sort_by(|a, b| {
        (
            &a.operator,
            &a.source,
            &a.source_endpoint,
            &a.destination,
            &a.destination_endpoint,
        )
            .cmp(&(
                &b.operator,
                &b.source,
                &b.source_endpoint,
                &b.destination,
                &b.destination_endpoint,
            ))
    });
    let mut projected: BTreeMap<(String, ResourceEndpoint), (Stock, i64)> = BTreeMap::new();
    // Earlier queued script effects have the same order the publication drain uses.
    // This is a temporary transaction view, not another simulation inventory.
    for request in &requests {
        for (uuid, endpoint) in [
            (&request.source, &request.source_endpoint),
            (&request.destination, &request.destination_endpoint),
        ] {
            let key = (uuid.clone(), endpoint.clone());
            if projected.contains_key(&key) {
                continue;
            }
            let Some((_, infrastructure, torpedoes)) =
                endpoints.iter().find(|(id, _, _)| id.0 == *uuid)
            else {
                continue;
            };
            if let Some((mut stock, held)) = endpoint_stock(endpoint, infrastructure, torpedoes) {
                if let ResourceEndpoint::Capacity { id } = endpoint {
                    for delta in capacities
                        .0
                        .iter()
                        .filter(|delta| delta.uuid == *uuid && delta.capacity == *id)
                    {
                        stock.adjust(delta.delta);
                    }
                }
                projected.insert(key, (stock, held));
            }
        }
    }
    for request in requests {
        let source_key = (request.source.clone(), request.source_endpoint.clone());
        let destination_key = (
            request.destination.clone(),
            request.destination_endpoint.clone(),
        );
        if source_key == destination_key {
            continue;
        }
        let (Some((source, _)), Some((destination, held))) =
            (projected.get(&source_key), projected.get(&destination_key))
        else {
            continue;
        };
        let moved = plan_transfer(
            request.amount,
            source.available(),
            destination.headroom().saturating_sub(*held),
        );
        if moved == 0 {
            continue;
        }
        let claim = format!("transfer:{}", request.operator);
        let source = &mut projected.get_mut(&source_key).expect("resolved source").0;
        if source.reserve(&claim, moved).is_err() {
            continue;
        }
        source.consume(&claim);
        projected
            .get_mut(&destination_key)
            .expect("resolved destination")
            .0
            .adjust(moved);
        for (uuid, endpoint, delta) in [
            (&request.source, &request.source_endpoint, -moved),
            (&request.destination, &request.destination_endpoint, moved),
        ] {
            match endpoint {
                ResourceEndpoint::Capacity { id } => capacities.0.push(CapacityAdjustment {
                    uuid: uuid.clone(),
                    capacity: id.clone(),
                    delta,
                }),
                ResourceEndpoint::TorpedoMagazine => {
                    if let Some((_, _, Some(mut system))) =
                        endpoints.iter_mut().find(|(id, _, _)| id.0 == *uuid)
                    {
                        if delta < 0 {
                            system.0.magazine.adjust(delta);
                        } else {
                            system.0.replenish(delta);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
