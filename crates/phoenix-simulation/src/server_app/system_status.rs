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
            if let Some(power) = power {
                let state = if system.kind == crate::ship::system_registry::POWER_REACTOR_KIND {
                    Some(&power.0.network().reactor)
                } else {
                    power.0.network().consumers.get(&system.id)
                };
                if let Some(state) = state {
                    if status.availability == Availability::Unknown {
                        status.availability = Availability::Normal;
                    }
                    if state.thermal_stopped {
                        status.unavailable(StatusReasonCode::Overheated, None);
                    }
                    if state.energy <= 0.0
                        && system.kind != crate::ship::system_registry::POWER_REACTOR_KIND
                    {
                        status.unavailable(StatusReasonCode::Unpowered, None);
                    }
                    if system
                        .power_group
                        .as_ref()
                        .and_then(|group| power.0.network().groups.get(group))
                        .is_some_and(|group| group.cut)
                    {
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
        let config: crate::ship::config::ShipConfig = toml::from_str(
            r#"
[[console]]
id = "equipment"
[[system]]
id = "dock"
kind = "dock"
console = "equipment"
ai_only = true
[[system.component]]
kind = "power_input"
id = "supply"
[[system.component]]
kind = "dock_mating"
id = "coupler"
enabled_channel = "enabled"
target_channel = "target"
power_input = "supply"
[system.component.config]
range = 200.0
engage_distance = 400.0
approach_speed = 60.0
mate_tolerance = 4.0
undock_clear_distance = 120.0
min_power_level = 2
"#,
        )
        .unwrap();
        let id = SystemId("dock".into());
        let cfg = crate::dock::mating::DockConfig {
            range: 200.0,
            engage_distance: 400.0,
            approach_speed: 60.0,
            mate_tolerance: 4.0,
            undock_clear_distance: 120.0,
            min_power_level: 2,
        };
        let mut dock = crate::dock::server::DockControl::new(
            id.clone(),
            cfg,
            crate::core::messages::PowerGroupId("dock".into()),
        );
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
        assert_eq!(status.reasons[1].component_id.as_deref(), Some("supply"));
        assert_eq!(status.reasons[2].component_id.as_deref(), Some("coupler"));
        hull.set_hp(&id, 100.0);
        dock.config.min_power_level = 0;
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
            None,
            Some(&markers),
        )[&id];
        assert_eq!(status.availability, Availability::Normal);
        assert!(status.reasons.is_empty());
    }
}
