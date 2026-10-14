//! Simulation joins Gameplay energy with World-owned finite coolant endpoints.
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::entities::spawner::EntityUuid;
use crate::infrastructure::{CapacityAdjustment, InfrastructureCondition};
use crate::resources::server::ResourceServices;
use crate::resources::service::{resolve_endpoint, ResourceEndpoint};
use crate::ship::power::{PowerBrownoutState, ShipPowerSystem};

pub fn coolant_bindings(
    power: &crate::modifiers::power_system::PowerSystem,
    services: &ResourceServices,
) -> Result<crate::resources::energy_network::EnergyCoolantBindings, String> {
    let resolve = |link: &crate::resources::service::ResourceLink| -> Result<String, String> {
        match resolve_endpoint("coolant", &services.0, link)? {
            ResourceEndpoint::Capacity { id } => Ok(id),
            ResourceEndpoint::TorpedoMagazine {} => {
                Err("coolant requires a physical capacity endpoint".into())
            }
        }
    };
    Ok(crate::resources::energy_network::EnergyCoolantBindings {
        reactor: resolve(&power.config().reactor.coolant)?,
        consumers: power
            .config()
            .consumers
            .iter()
            .map(|consumer| Ok((consumer.system.clone(), resolve(&consumer.coolant)?)))
            .collect::<Result<_, String>>()?,
    })
}

/// A queued stock change is part of the transaction view. World applies it once
/// and publishes threshold edges through its existing capacity owner.
pub fn tick_power_system(
    time: Res<Time>,
    rng: crate::sim_rng::LiveStream<'_, { crate::sim_rng::SimStream::PowerFlowCuts as usize }>,
    mut adjustments: ResMut<crate::effect_queue::EffectQueue<CapacityAdjustment>>,
    mut ships: Query<(
        Entity,
        &EntityUuid,
        &mut ShipPowerSystem,
        &ResourceServices,
        Option<&InfrastructureCondition>,
        Option<&crate::ship_plugin::ShipSystemControlSources>,
        Option<&mut PowerBrownoutState>,
    )>,
) {
    let mut order: Vec<_> = ships
        .iter_mut()
        .map(|(entity, uuid, ..)| (uuid.0.clone(), entity))
        .collect();
    order.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, entity) in order {
        let Ok((_, uuid, mut power, services, infrastructure, sources, brownout)) =
            ships.get_mut(entity)
        else {
            continue;
        };
        let Some(infrastructure) = infrastructure else {
            continue;
        };
        let Ok(bindings) = coolant_bindings(&power.0, services) else {
            continue;
        };
        let ids: BTreeSet<_> = std::iter::once(bindings.reactor.clone())
            .chain(bindings.consumers.values().cloned())
            .collect();
        let stocks: Option<BTreeMap<_, _>> = ids
            .iter()
            .map(|id| {
                let mut stock = infrastructure.0.capacity_stock(id)?.clone();
                for change in adjustments
                    .0
                    .iter()
                    .filter(|change| change.uuid == uuid.0 && &change.capacity == id)
                {
                    stock.adjust(change.delta);
                }
                Some((id.clone(), stock.available()))
            })
            .collect();
        let Some(stocks) = stocks else {
            continue;
        };
        let before: BTreeSet<_> = power
            .0
            .network()
            .groups
            .iter()
            .filter(|(_, group)| group.cut)
            .map(|(id, _)| id.clone())
            .collect();
        let mut candidate = power.0.clone();
        let requested_output = candidate.network().output_target_per_sec;
        if sources.is_some_and(|sources| {
            sources
                .0
                .is_offline(&crate::ship::system_registry::power_reactor_system_id())
        }) {
            candidate.network_mut().output_actual_per_sec = 0.0;
            candidate.network_mut().output_target_per_sec = 0.0;
        }
        let result = crate::sim_rng::with_live_stream(rng.as_deref(), |generator| {
            candidate.tick_energy(time.delta_secs(), &bindings, &stocks, &mut |count| {
                generator.below(count as u32) as usize
            })
        });
        let Ok(result) = result else {
            continue;
        };
        candidate.network_mut().output_target_per_sec = requested_output;
        for (id, amount) in result.coolant_withdrawals {
            if amount > 0 {
                adjustments.0.push(CapacityAdjustment {
                    uuid: uuid.0.clone(),
                    capacity: id,
                    delta: -amount,
                });
            }
        }
        let after: BTreeSet<_> = candidate
            .network()
            .groups
            .iter()
            .filter(|(_, group)| group.cut)
            .map(|(id, _)| id.clone())
            .collect();
        if let Some(mut state) = brownout {
            state.locked_changed |= before != after;
        }
        power.0 = candidate;
    }
}
