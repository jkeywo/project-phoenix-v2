//! Current equipment observations, derived directly from existing owners.
use crate::core::messages::SystemId;
use crate::ship::control_source::{ControlSource, ControlSourceResolver};
use phoenix_model::system_status::{Availability, StatusReasonCode, SystemStatus};
use std::collections::BTreeMap;

pub(crate) fn project(
    config: &crate::ship::config::ShipConfig,
    hull: Option<&crate::ship::damage::SystemHull>,
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
            let damage = entry.and_then(|_| hull.map(|h| h.tier_for(&system.id)));
            let source = sources.source_for(&system.id);
            let mut status = SystemStatus::observe(
                entry
                    .map(|e| e.display_name.clone())
                    .unwrap_or_else(|| system.id.0.clone()),
                damage,
                sources.is_offline(&system.id) || source == ControlSource::Offline,
                source == ControlSource::Ai,
            );
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
mod tests {
    use super::*;

    fn cruiser() -> (
        crate::ship::config::ShipConfig,
        crate::ship::power::ShipPowerSystem,
    ) {
        let authored = crate::entities::include_resolve::load_entity_config(
            "assets/entities/alliance_cruiser.toml",
        )
        .unwrap();
        let config = authored.ship_config.unwrap();
        let power = crate::modifiers::power_system::PowerSystem::from_energy(
            authored.energy.unwrap(),
            config.clone(),
        )
        .unwrap();
        (config, crate::ship::power::ShipPowerSystem(power))
    }

    #[test]
    fn parent_intake_cut_keeps_charged_equipment_degraded_until_thermal_trip() {
        let (config, mut power) = cruiser();
        let id = SystemId("dock".into());
        let group = config.system(&id).unwrap().power_group.as_ref().unwrap();
        let parent = power
            .0
            .config()
            .groups
            .iter()
            .find(|entry| &entry.id == group)
            .unwrap()
            .parent
            .clone()
            .unwrap();
        power.0.network_mut().groups.get_mut(&parent).unwrap().cut = true;
        assert!(!power.0.network().groups[group].cut);
        let sources = ControlSourceResolver::new();
        let status = &project(&config, None, &sources, None, Some(&power), None)[&id];
        assert_eq!(status.availability, Availability::Degraded);
        assert_eq!(
            status
                .reasons
                .iter()
                .map(|reason| reason.code)
                .collect::<Vec<_>>(),
            vec![StatusReasonCode::IntakeCut]
        );
        let trip_heat = power
            .0
            .config()
            .consumers
            .iter()
            .find(|profile| profile.system == id)
            .unwrap()
            .reserve
            .trip_heat;
        let reserve = power.0.network_mut().consumers.get_mut(&id).unwrap();
        reserve.thermal_stopped = true;
        reserve.heat = trip_heat;
        let status = &project(&config, None, &sources, None, Some(&power), None)[&id];
        assert_eq!(status.availability, Availability::Unavailable);
        assert!(status
            .reasons
            .iter()
            .any(|reason| reason.code == StatusReasonCode::Overheated));
        assert!(status
            .reasons
            .iter()
            .any(|reason| reason.code == StatusReasonCode::IntakeCut));
        assert!(!status
            .reasons
            .iter()
            .any(|reason| reason.code == StatusReasonCode::Unpowered));
    }

    #[test]
    fn work_costs_distinguish_free_discrete_and_partial_mixed_capabilities() {
        let (config, base) = cruiser();
        let id = SystemId("dock".into());
        let sources = ControlSourceResolver::new();
        for (continuous, discrete, energy, expected) in [
            (0.0, 0.0, 0.0, Availability::Normal),
            (0.0, 2.0, 1.0, Availability::Unavailable),
            (1.0, 2.0, 1.0, Availability::Degraded),
        ] {
            let mut hardware = base.0.config().clone();
            let profile = hardware
                .consumers
                .iter_mut()
                .find(|profile| profile.system == id)
                .unwrap();
            profile.continuous_cost_per_sec = continuous;
            profile.discrete_cost = discrete;
            profile.initial_energy = energy;
            let power = crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::from_energy(hardware, config.clone())
                    .unwrap(),
            );
            let status = &project(&config, None, &sources, None, Some(&power), None)[&id];
            assert_eq!(status.availability, expected);
            assert_eq!(
                status
                    .reasons
                    .iter()
                    .any(|reason| reason.code == StatusReasonCode::Unpowered),
                expected != Availability::Normal
            );
        }
    }
    #[test]
    fn status_visibility_follows_exact_console_host_and_revokes_stale_host() {
        use crate::core::messages::StationId;
        use phoenix_model::{console::ConsoleHostSnapshot, identity::ConsoleId};
        let id = SystemId("passive".into());
        let console = ConsoleId("navigation".into());
        let owners = BTreeMap::from([(id.clone(), console.clone())]);
        let mut hosts = vec![ConsoleHostSnapshot {
            console,
            host: Some(StationId("tactical".into())),
            rating: "Std".into(),
        }];
        assert!(visible_to(
            &id,
            &owners,
            &hosts,
            Some(&StationId("tactical".into()))
        ));
        assert!(!visible_to(
            &id,
            &owners,
            &hosts,
            Some(&StationId("navigation".into()))
        ));
        assert!(!visible_to(&id, &owners, &hosts, None));
        hosts[0].host = Some(StationId("captain".into()));
        assert!(!visible_to(
            &id,
            &owners,
            &hosts,
            Some(&StationId("tactical".into()))
        ));
        assert!(visible_to(
            &id,
            &owners,
            &hosts,
            Some(&StationId("captain".into()))
        ));
    }
    #[test]
    fn dock_current_facts_override_historical_refusal_and_keep_multiple_reasons() {
        let (config, mut power) = cruiser();
        let id = SystemId("dock".into());
        let mut dock = crate::dock::server::DockControl::from_system(config.system(&id).unwrap());
        let mut sources = ControlSourceResolver::new();
        sources.set(id.clone(), ControlSource::Ai);
        let mut hull = crate::ship::damage::SystemHull::from_config(&[(id.clone(), 100.0)]);
        hull.set_hp(&id, 0.0);
        let statuses = project(&config, Some(&hull), &sources, Some(&dock), None, None);
        let status = &statuses[&id];
        assert_eq!(status.availability, Availability::Unavailable);
        assert!(status.delegated);
        assert_eq!(
            status
                .reasons
                .iter()
                .map(|reason| reason.code)
                .collect::<Vec<_>>(),
            vec![
                StatusReasonCode::Destroyed,
                StatusReasonCode::Unpowered,
                StatusReasonCode::NoDockMarkers
            ]
        );
        assert_eq!(status.reasons[1].component_id.as_deref(), Some("power"));
        assert_eq!(status.reasons[2].component_id.as_deref(), Some("mating"));
        hull.set_hp(&id, 100.0);
        dock.last_refusal = Some(crate::dock::mating::DockRefusal::Unpowered);
        let markers = crate::dock::server::DockMarkers {
            markers: vec![crate::dock::mating::DockMarker {
                position: bevy::prelude::Vec3::ZERO,
                direction: bevy::prelude::Vec3::NEG_Z,
            }],
        };
        let status = &project(
            &config,
            Some(&hull),
            &sources,
            Some(&dock),
            Some(&power),
            Some(&markers),
        )[&id];
        assert_eq!(status.availability, Availability::Normal);
        assert!(status.reasons.is_empty());
        power.0.network_mut().consumers.get_mut(&id).unwrap().energy = 0.0;
        let status = &project(
            &config,
            Some(&hull),
            &sources,
            Some(&dock),
            Some(&power),
            Some(&markers),
        )[&id];
        assert_eq!(status.availability, Availability::Unavailable);
        assert_eq!(status.reasons[0].code, StatusReasonCode::Unpowered);
        assert!(status.delegated);
    }
}
