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

/// Derive one ephemeral view of physical links. Authored limits and operator
/// requests remain unchanged; repaired machinery resumes those same requests.
fn hardware_effects(
    power: &crate::modifiers::power_system::PowerSystem,
    physical: Option<&crate::ship::compartment_damage::ShipDamage>,
    sources: Option<&crate::ship_plugin::ShipSystemControlSources>,
) -> crate::resources::energy_network::EnergyHardwareEffects {
    use crate::resources::energy_network::{EnergyHardwareEffects, EnergyLinkEffects};
    use phoenix_model::damage::DamageBinding;
    let factor = |id: &crate::core::messages::SystemId, binding| {
        physical.map_or(1.0, |damage| damage.factor(id, binding))
    };
    let mut effects = EnergyHardwareEffects::default();
    for system in power
        .ship_config()
        .systems
        .iter()
        .filter(|system| system.kind == crate::ship::system_registry::POWER_REACTOR_KIND)
    {
        effects.reactor_operation = effects.reactor_operation.min(
            if sources.is_some_and(|sources| sources.0.is_offline(&system.id)) {
                0.0
            } else {
                factor(&system.id, DamageBinding::Operation)
            },
        );
        effects.reactor.intake = effects
            .reactor
            .intake
            .min(factor(&system.id, DamageBinding::Intake));
        effects.reactor.cooling = effects
            .reactor
            .cooling
            .min(factor(&system.id, DamageBinding::Cooling));
    }
    effects.consumers = power
        .config()
        .consumers
        .iter()
        .map(|consumer| {
            (
                consumer.system.clone(),
                EnergyLinkEffects {
                    intake: factor(&consumer.system, DamageBinding::Intake),
                    cooling: factor(&consumer.system, DamageBinding::Cooling),
                },
            )
        })
        .collect();
    effects
}

/// A queued stock change is part of the transaction view. World applies it once
/// and publishes threshold edges through its existing capacity owner.
pub fn tick_power_system(
    time: Res<Time>,
    rng: crate::sim_rng::LiveStream<'_, { crate::sim_rng::SimStream::PowerFlowCuts as usize }>,
    mut adjustments: ResMut<crate::effect_queue::EffectQueue<CapacityAdjustment>>,
    mut ships: Query<
        (
            Entity,
            &EntityUuid,
            &mut ShipPowerSystem,
            &ResourceServices,
            Option<&InfrastructureCondition>,
            Option<&crate::ship_plugin::ShipSystemControlSources>,
            Option<&mut PowerBrownoutState>,
            Option<&crate::ship::compartment_damage::ShipDamage>,
            Option<&crate::resources::fitting_server::CompiledShipFitting>,
            Option<&crate::resources::fitting_server::ShipFitting>,
            Option<&crate::entities::craft::ProbeCustody>,
            Has<bevy::ecs::entity_disabling::Disabled>,
        ),
        Allow<bevy::ecs::entity_disabling::Disabled>,
    >,
) {
    let mut order: Vec<_> = ships
        .iter_mut()
        .map(|(entity, uuid, ..)| (uuid.0.clone(), entity))
        .collect();
    order.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, entity) in order {
        let Ok((
            _,
            uuid,
            mut power,
            services,
            infrastructure,
            sources,
            brownout,
            physical,
            fitting_profile,
            fitting_state,
            custody,
            disabled,
        )) = ships.get_mut(entity)
        else {
            continue;
        };
        if custody.is_some_and(|custody| {
            matches!(
                &custody.0,
                phoenix_model::craft::CraftLocation::Carried { .. }
            )
        }) {
            // Same energy owner ages actual reserves and thermal state while
            // stored. No active offer, draw, routed credit or scratch survives.
            let _ = power.0.tick_stored_energy(time.delta_secs());
            continue;
        }
        if disabled {
            continue;
        }
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
        let mut effects = hardware_effects(&candidate, physical, sources);
        if fitting_profile
            .zip(fitting_state)
            .is_some_and(|(profile, state)| {
                candidate
                    .ship_config()
                    .systems
                    .iter()
                    .filter(|system| {
                        system.kind == crate::ship::system_registry::POWER_REACTOR_KIND
                    })
                    .any(|system| {
                        crate::resources::fitting_server::unavailable(profile, state, &system.id)
                    })
            })
        {
            effects.reactor_operation = 0.0;
        }
        let result = crate::sim_rng::with_live_stream(rng.as_deref(), |generator| {
            candidate.tick_energy_with_effects(
                time.delta_secs(),
                &bindings,
                &stocks,
                &effects,
                &mut |count| generator.below(count as u32) as usize,
            )
        });
        let Ok(result) = result else {
            continue;
        };
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
        for kind in [
            crate::ship::system_registry::HELM_RADAR_KIND,
            crate::ship::system_registry::TACTICAL_RADAR_KIND,
            crate::ship::system_registry::SENSOR_RADAR_KIND,
            crate::ship::system_registry::NAVIGATION_KIND,
        ] {
            let ids: Vec<_> = candidate
                .ship_config()
                .systems
                .iter()
                .filter(|system| system.kind == kind)
                .map(|system| system.id.clone())
                .collect();
            for id in ids {
                // Disabled machinery performs no work; a restored setting can resume.
                let activity = if sources.is_some_and(|sources| sources.0.is_offline(&id)) {
                    0.0
                } else {
                    1.0
                };
                let _ = candidate.work_continuous(&id, time.delta_secs(), activity);
            }
        }
        power.0 = candidate;
    }
}

#[cfg(test)]
#[path = "energy_runtime_effects_tests.rs"]
mod effects_tests;
