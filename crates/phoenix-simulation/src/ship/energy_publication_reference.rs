//! Project the canonical energy network into ordinary Console observations.
use crate::core::messages::*;
use crate::ship::power::{PowerConfigResource, ShipPowerSystem};
use bevy::prelude::*;
use phoenix_model::energy::*;

pub(super) fn publish_power_blackboard_reference(
    mut ships: Query<(
        &ShipPowerSystem,
        &crate::ship_plugin::ShipConfigComponent,
        Option<&crate::ship_plugin::ShipSystemControlSources>,
        Option<&PowerConfigResource>,
        &crate::resources::server::ResourceServices,
        Option<&crate::infrastructure::InfrastructureCondition>,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
) {
    for (power, ship, sources, legacy, services, infrastructure, mut boards) in &mut ships {
        let p = &power.0;
        let config = p.config();
        let network = p.network();
        let r = &config.reactor;
        let mut coolant = Vec::new();
        if let Ok(bindings) = super::super::energy_runtime::coolant_bindings(p, services) {
            let ids: std::collections::BTreeSet<_> = std::iter::once(bindings.reactor)
                .chain(bindings.consumers.into_values())
                .collect();
            for id in ids {
                coolant.push(CoolantReading {
                    available: infrastructure
                        .and_then(|state| state.0.capacity_stock(&id))
                        .map(|stock| stock.available())
                        .unwrap_or(0),
                    credit: p.coolant_credit().get(&id).copied().unwrap_or(0.0),
                    provider: id,
                });
            }
        }
        let energy = PowerEnergyBlackboard {
            reactor: ReactorEnergyReading {
                generation_enabled: network.generation_enabled,
                energy: network.reactor.energy,
                capacity: r.reserve.capacity,
                heat: network.reactor.heat,
                trip_heat: r.reserve.trip_heat,
                recover_heat: r.reserve.recover_heat,
                thermal_stopped: network.reactor.thermal_stopped,
                output_target_per_sec: network.output_target_per_sec,
                output_actual_per_sec: network.output_actual_per_sec,
                sustainable_output_per_sec: r.sustainable_output_per_sec,
                max_output_per_sec: r.max_output_per_sec,
                cooling_per_sec: network.reactor_cooling_per_sec,
                max_cooling_per_sec: r.max_cooling_per_sec,
            },
            mode: network.mode,
            cooling_usage: network.cooling_usage,
            groups: config
                .groups
                .iter()
                .map(|group| {
                    let state = &network.groups[&group.id];
                    EnergyGroupReading {
                        id: group.id.0.clone(),
                        label: ship
                            .0
                            .power_groups
                            .get(&group.id)
                            .map(|group| group.label.clone())
                            .unwrap_or_else(|| group.id.0.clone()),
                        parent: group.parent.as_ref().map(|parent| parent.0.clone()),
                        desired_flow_per_sec: state.desired_flow_per_sec,
                        effective_flow_per_sec: state.effective_flow_per_sec,
                        max_flow_per_sec: group.max_flow_per_sec,
                        desired_cooling_per_sec: state.desired_cooling_per_sec,
                        effective_cooling_per_sec: state.effective_cooling_per_sec,
                        max_cooling_per_sec: group.max_cooling_per_sec,
                        priority: state.priority,
                        cut: state.cut,
                    }
                })
                .collect(),
            consumers: config
                .consumers
                .iter()
                .map(|consumer| {
                    let state = &network.consumers[&consumer.system];
                    ConsumerEnergyReading {
                        system: consumer.system.clone(),
                        group: ship
                            .0
                            .systems
                            .iter()
                            .find(|system| system.id == consumer.system)
                            .and_then(|system| system.power_group.clone()),
                        energy: state.energy,
                        capacity: consumer.reserve.capacity,
                        heat: state.heat,
                        trip_heat: consumer.reserve.trip_heat,
                        recover_heat: consumer.reserve.recover_heat,
                        thermal_stopped: state.thermal_stopped,
                        intake_per_sec: p.last_intake_per_sec(&consumer.system).unwrap_or(0.0),
                        max_intake_per_sec: consumer.reserve.max_intake_per_sec,
                        flow_override_per_sec: network.fine_flows.get(&consumer.system).copied(),
                    }
                })
                .collect(),
            coolant,
        };
        let bb = PowerBlackboard {
            energy: Some(energy),
            strike_reserve: legacy.and_then(|legacy| p.strike_read(&legacy.0)),
            groups: Vec::new(),
            total: p.total(),
            total_max: p.max_commanded_total(),
            battery_charge: p.battery_charge(),
            battery_max: p.capacity(),
            draining: p.reactor_energy_rate() < 0.0,
            charging: p.reactor_energy_rate() > 0.0,
            locked: false,
        };
        let reactor = crate::ship::system_registry::power_reactor_system_id();
        let battery = crate::ship::system_registry::power_battery_system_id();
        let online = |id: &SystemId| !sources.is_some_and(|sources| sources.0.is_offline(id));
        boards.0.insert(
            SystemId(crate::ship::system_registry::POWER_SYSTEM_ID.into()),
            SystemBlackboard::Power(bb),
        );
        boards.0.insert(
            reactor.clone(),
            SystemBlackboard::PowerReactor(PowerReactorBlackboard {
                total_allocation: p.total(),
                max_allocation: p.max_commanded_total(),
                is_online: online(&reactor),
                draining: p.reactor_energy_rate() < 0.0,
            }),
        );
        boards.0.insert(
            battery.clone(),
            SystemBlackboard::PowerBattery(PowerBatteryBlackboard {
                charge: p.battery_charge(),
                capacity: p.capacity(),
                is_online: online(&battery),
                emergency_threshold: 0.0,
            }),
        );
    }
}
