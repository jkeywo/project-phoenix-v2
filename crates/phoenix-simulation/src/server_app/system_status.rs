//! Current equipment observations, derived directly from existing owners.
use crate::core::messages::SystemId;
use crate::ship::control_source::{ControlSource, ControlSourceResolver};
use phoenix_model::system_status::{Availability, StatusReasonCode, SystemStatus};
use std::collections::BTreeMap;

pub(crate) fn project(
    config: &crate::ship::config::ShipConfig,
    hull: Option<&crate::ship::damage::SystemHull>,
    physical: Option<&crate::ship::compartment_damage::ShipDamage>,
    sources: &ControlSourceResolver,
    dock: Option<&crate::dock::server::DockControl>,
    power: Option<&crate::ship::power::ShipPowerSystem>,
    markers: Option<&crate::dock::server::DockMarkers>,
) -> BTreeMap<SystemId, SystemStatus> {
    config
        .systems
        .iter()
        .map(|system| {
            let entry = hull.and_then(|h| h.get(&system.id));
            let damage = physical
                .map(|physical| physical.operating_tier(&system.id))
                .or_else(|| entry.and_then(|_| hull.map(|h| h.tier_for(&system.id))));
            let source = sources.source_for(&system.id);
            let mut status = SystemStatus::observe(
                entry
                    .map(|e| e.display_name.clone())
                    .unwrap_or_else(|| system.id.0.clone()),
                damage,
                sources.is_offline(&system.id) || source == ControlSource::Offline,
                source == ControlSource::Ai,
            );
            if let Some(physical) = physical {
                for component in physical
                    .state
                    .emp_failed_components(&system.id, physical.emp_clock)
                {
                    if physical.definition.components[&system.id][&component].binding
                        == phoenix_model::damage::DamageBinding::Operation
                    {
                        status.unavailable(StatusReasonCode::Emp, Some(component));
                    } else {
                        if status.availability == Availability::Normal {
                            status.availability = Availability::Degraded;
                        }
                        status
                            .reasons
                            .push(phoenix_model::system_status::StatusReason {
                                code: StatusReasonCode::Emp,
                                component_id: Some(component),
                            });
                    }
                }
            }
            let supply = system
                .components
                .iter()
                .find_map(|component| match component {
                    crate::ship::composition::SystemComponentConfig::PowerInput { id } => {
                        Some(id.clone())
                    }
                    _ => None,
                });
            let needs_energy = crate::entities::config::requires_energy_profile(&system.kind);
            let mut has_hardware = false;
            if let Some(power) = power {
                let reactor_view = matches!(
                    system.kind.as_str(),
                    crate::ship::system_registry::POWER_REACTOR_KIND
                        | crate::ship::system_registry::POWER_BATTERY_KIND
                );
                let profile = power
                    .0
                    .config()
                    .consumers
                    .iter()
                    .find(|profile| profile.system == system.id);
                let state = if reactor_view {
                    Some(&power.0.network().reactor)
                } else {
                    power.0.network().consumers.get(&system.id)
                };
                if let Some(state) = state {
                    has_hardware = reactor_view || profile.is_some();
                    if status.availability == Availability::Unknown {
                        status.availability = Availability::Normal;
                    }
                    if state.thermal_stopped {
                        status.unavailable(StatusReasonCode::Overheated, None);
                    }
                    if let Some(profile) = profile {
                        let continuous = profile.continuous_cost_per_sec > 0.0;
                        let discrete = profile.discrete_cost > 0.0;
                        if state.energy <= 0.0 && (continuous || discrete) {
                            status.unavailable(StatusReasonCode::Unpowered, supply.clone());
                        } else if discrete && state.energy < profile.discrete_cost {
                            if continuous && state.energy > 0.0 {
                                if status.availability == Availability::Normal {
                                    status.availability = Availability::Degraded;
                                }
                                status
                                    .reasons
                                    .push(phoenix_model::system_status::StatusReason {
                                        code: StatusReasonCode::Unpowered,
                                        component_id: supply.clone(),
                                    });
                            } else {
                                status.unavailable(StatusReasonCode::Unpowered, supply.clone());
                            }
                        }
                    }
                    let mut group = system.power_group.as_ref();
                    let mut intake_cut = false;
                    // Authored ancestry is validated before spawn. Bound this
                    // read as well so status remains safe for diagnostic fixtures.
                    for _ in 0..power.0.config().groups.len() {
                        let Some(id) = group else { break };
                        if power
                            .0
                            .network()
                            .groups
                            .get(id)
                            .is_some_and(|state| state.cut)
                        {
                            intake_cut = true;
                            break;
                        }
                        group = power
                            .0
                            .config()
                            .groups
                            .iter()
                            .find(|group| &group.id == id)
                            .and_then(|group| group.parent.as_ref());
                    }
                    if intake_cut {
                        if status.availability == Availability::Normal {
                            status.availability = Availability::Degraded;
                        }
                        status
                            .reasons
                            .push(phoenix_model::system_status::StatusReason {
                                code: StatusReasonCode::IntakeCut,
                                component_id: None,
                            });
                    }
                }
            }
            if needs_energy && !has_hardware {
                status.unavailable(StatusReasonCode::Unpowered, supply);
            }
            if dock.is_some_and(|dock| dock.system_id == system.id) {
                if status.availability == Availability::Unknown {
                    status.availability = Availability::Normal;
                }
                if markers.is_none_or(|markers| markers.is_empty()) {
                    let component =
                        system
                            .components
                            .iter()
                            .find_map(|component| match component {
                                crate::ship::composition::SystemComponentConfig::DockMating {
                                    id,
                                    ..
                                } => Some(id.clone()),
                                _ => None,
                            });
                    status.unavailable(StatusReasonCode::NoDockMarkers, component);
                }
            }
            (system.id.clone(), status)
        })
        .collect()
}

/// Refine equipment availability with actual funded shield operation.
pub(crate) fn apply_shields(
    status: &mut BTreeMap<SystemId, SystemStatus>,
    shields: Option<&crate::ship::shields::ShipShields>,
) {
    let Some(shields) = shields else {
        return;
    };
    for facing in &shields.0.facings {
        let Some(id) = crate::ship::system_registry::shield_arc_system_id(&facing.id) else {
            continue;
        };
        let Some(status) = status.get_mut(&id) else {
            continue;
        };
        if status.availability == Availability::Unavailable {
            continue;
        }
        if !facing.raised || !facing.is_online() {
            status.unavailable(StatusReasonCode::Offline, None);
        } else if facing.protection_fraction <= 0.0 {
            status.unavailable(StatusReasonCode::Unpowered, None);
        } else if facing.protection_fraction < 1.0 {
            status.availability = Availability::Degraded;
            status
                .reasons
                .push(phoenix_model::system_status::StatusReason {
                    code: StatusReasonCode::Unpowered,
                    component_id: None,
                });
        }
    }
}

/// Detailed status follows live Console tenure, never an identically named seat.
pub(crate) fn visible_to(
    id: &SystemId,
    owners: &BTreeMap<SystemId, phoenix_model::identity::ConsoleId>,
    hosts: &[phoenix_model::console::ConsoleHostSnapshot],
    station: Option<&crate::core::messages::StationId>,
) -> bool {
    owners.get(id).is_some_and(|owner| {
        hosts.iter().any(|assignment| {
            &assignment.console == owner
                && assignment.host.is_some()
                && assignment.host.as_ref() == station
        })
    })
}

#[cfg(test)]
#[path = "system_status_tests.rs"]
mod tests;
