//! Project the canonical energy network into ordinary Console observations.
use crate::core::messages::*;
use crate::ship::power::{PowerConfigResource, ShipPowerSystem};
use bevy::prelude::*;
use phoenix_model::energy::*;

pub fn publish_power_blackboard(
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
        let key = SystemId(crate::ship::system_registry::POWER_SYSTEM_ID.into());
        let slot = boards
            .0
            .entry(key)
            .or_insert_with(|| SystemBlackboard::Power(PowerBlackboard::default()));
        if !matches!(slot, SystemBlackboard::Power(_)) {
            *slot = SystemBlackboard::Power(PowerBlackboard::default());
        }
        let SystemBlackboard::Power(previous) = slot else {
            unreachable!()
        };
        let previous_energy = previous.energy.take().unwrap_or_default();
        let mut coolant = previous_energy.coolant;
        let mut coolant_count = 0;
        if let Ok(bindings) = super::energy_runtime::coolant_bindings(p, services) {
            let ids: std::collections::BTreeSet<_> = std::iter::once(bindings.reactor)
                .chain(bindings.consumers.into_values())
                .collect();
            for id in ids {
                let old = coolant.get_mut(coolant_count);
                let reading = CoolantReading {
                    available: infrastructure
                        .and_then(|state| state.0.capacity_stock(&id))
                        .map(|stock| stock.available())
                        .unwrap_or(0),
                    credit: p.coolant_credit().get(&id).copied().unwrap_or(0.0),
                    provider: reuse_text(old.map(|row| &mut row.provider), &id),
                };
                replace_row(&mut coolant, coolant_count, reading);
                coolant_count += 1;
            }
        }
        coolant.truncate(coolant_count);
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
            groups: refresh_rows(
                previous_energy.groups,
                config.groups.iter(),
                |group, mut old| {
                    let state = &network.groups[&group.id];
                    EnergyGroupReading {
                        id: reuse_text(old.as_deref_mut().map(|row| &mut row.id), &group.id.0),
                        label: reuse_text(
                            old.as_deref_mut().map(|row| &mut row.label),
                            ship.0
                                .power_groups
                                .get(&group.id)
                                .map(|group| group.label.as_str())
                                .unwrap_or(&group.id.0),
                        ),
                        parent: reuse_optional_text(
                            old.as_deref_mut().map(|row| &mut row.parent),
                            group.parent.as_ref().map(|parent| parent.0.as_str()),
                        ),
                        desired_flow_per_sec: state.desired_flow_per_sec,
                        effective_flow_per_sec: state.effective_flow_per_sec,
                        max_flow_per_sec: group.max_flow_per_sec,
                        desired_cooling_per_sec: state.desired_cooling_per_sec,
                        effective_cooling_per_sec: state.effective_cooling_per_sec,
                        max_cooling_per_sec: group.max_cooling_per_sec,
                        priority: state.priority,
                        cut: state.cut,
                    }
                },
            ),
            consumers: refresh_rows(
                previous_energy.consumers,
                config.consumers.iter(),
                |consumer, mut old| {
                    let state = &network.consumers[&consumer.system];
                    ConsumerEnergyReading {
                        system: SystemId(reuse_text(
                            old.as_deref_mut().map(|row| &mut row.system.0),
                            &consumer.system.0,
                        )),
                        group: {
                            let source = ship
                                .0
                                .systems
                                .iter()
                                .find(|system| system.id == consumer.system)
                                .and_then(|system| system.power_group.as_ref());
                            let mut prior = old
                                .as_deref_mut()
                                .and_then(|row| row.group.take())
                                .map(|id| id.0);
                            reuse_optional_text(Some(&mut prior), source.map(|id| id.0.as_str()))
                                .map(PowerGroupId)
                        },
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
                },
            ),
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
        *previous = bb;
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

// Reuse storage only: every row and every scalar is projected anew from this call's inputs.
fn replace_row<T>(rows: &mut Vec<T>, index: usize, row: T) {
    if index < rows.len() {
        rows[index] = row;
    } else {
        rows.push(row);
    }
}
fn refresh_rows<T, I: IntoIterator>(
    mut rows: Vec<T>,
    input: I,
    mut project: impl FnMut(I::Item, Option<&mut T>) -> T,
) -> Vec<T> {
    let mut count = 0;
    for item in input {
        let row = project(item, rows.get_mut(count));
        replace_row(&mut rows, count, row);
        count += 1;
    }
    rows.truncate(count);
    rows
}
fn reuse_text(old: Option<&mut String>, source: &str) -> String {
    let mut text = old.map(std::mem::take).unwrap_or_default();
    text.clear();
    text.push_str(source);
    text
}
fn reuse_optional_text(old: Option<&mut Option<String>>, source: Option<&str>) -> Option<String> {
    let mut text = old.and_then(Option::take);
    source.map(|source| reuse_text(text.as_mut(), source))
}
#[cfg(test)]
#[path = "energy_publication_reference.rs"]
mod reference;
#[cfg(test)]
#[path = "energy_publication_tests.rs"]
mod tests;
