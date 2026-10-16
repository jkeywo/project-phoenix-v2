use crate::core::messages::{StationId, SystemControlPayload};
/// Closed crew orders, distinct from powered repair-tool actuation. This does
/// not grant another Console authority or alter the ordinary individual AI.
pub fn is_repair_crew_order(
    config: &crate::ship::config::ShipConfig,
    target: &crate::core::messages::SystemId,
    payload: &SystemControlPayload,
) -> bool {
    config
        .system(target)
        .is_some_and(|system| system.kind == crate::ship::system_registry::REPAIR_KIND)
        && matches!(
            payload,
            SystemControlPayload::DispatchRepairTeam { .. }
                | SystemControlPayload::RecallRepairTeam { .. }
                | SystemControlPayload::SetRepairPriority { .. }
                | SystemControlPayload::SetRepairTargetPriority { .. }
                | SystemControlPayload::DispatchExternalRepair
                | SystemControlPayload::RecallExternalRepair
                | SystemControlPayload::DispatchRefit { .. }
                | SystemControlPayload::CancelRefit { .. }
        )
}

pub fn command_policy(
    config: &crate::ship::config::ShipConfig,
    sources: &crate::ship::control_source::ControlSourceResolver,
    target: &crate::core::messages::SystemId,
    payload: &SystemControlPayload,
) -> crate::ship::control_source::ControlTickPolicy {
    use crate::ship::system_registry::{HELM_STEERING_KIND, HELM_THRUST_KIND, LATERAL_THRUST_KIND};
    if matches!(
        payload,
        SystemControlPayload::SetCommsListeningOrder { .. }
            | SystemControlPayload::ReportCommsAlert
    ) && config
        .system(target)
        .is_none_or(|system| system.kind != crate::ship::system_registry::COMMS_KIND)
    {
        return crate::ship::control_source::control_tick_policy(
            crate::ship::control_source::ControlSource::Offline,
        );
    }
    if let SystemControlPayload::SelectCommsCapture { capture } = payload {
        if capture.is_some_and(|value| {
            value == 0 || value > phoenix_model::comms_capture::MAX_CAPTURE_SEQUENCE
        }) || config
            .system(target)
            .is_none_or(|system| system.kind != crate::ship::system_registry::COMMS_KIND)
        {
            return crate::ship::control_source::control_tick_policy(
                crate::ship::control_source::ControlSource::Offline,
            );
        }
    }
    if matches!(payload, SystemControlPayload::SetEmissionIntent { .. })
        && crate::ship::emission_profile::profile(config)
            .ok()
            .flatten()
            .is_none_or(|(owner, _)| &owner != target)
    {
        return crate::ship::control_source::control_tick_policy(
            crate::ship::control_source::ControlSource::Offline,
        );
    }
    let movement_kind = match payload {
        SystemControlPayload::SetHelmMode { .. }
        | SystemControlPayload::SetFollowNavigation { .. } => {
            Some(&[HELM_THRUST_KIND, HELM_STEERING_KIND, LATERAL_THRUST_KIND][..])
        }
        SystemControlPayload::SetDesiredSpeed { .. }
        | SystemControlPayload::SetMatchSpeed { .. } => Some(&[HELM_THRUST_KIND][..]),
        SystemControlPayload::SetDesiredHeading { .. } => Some(&[HELM_STEERING_KIND][..]),
        _ => None,
    };
    if movement_kind.is_some_and(|kinds| {
        config
            .system(target)
            .is_none_or(|system| !kinds.contains(&system.kind.as_str()))
    }) {
        return crate::ship::control_source::control_tick_policy(
            crate::ship::control_source::ControlSource::Offline,
        );
    }
    if matches!(payload, SystemControlPayload::StartPortOrder { .. }) {
        // Manual trade approval is a human decision. Ordinary AI may execute
        // the resulting order, but cannot originate or enlarge its authority.
        let mut policy = sources.policy_for(target);
        policy.operate_ai = false;
        return policy;
    }
    // A stock target is an administrative request, not powered machinery work.
    // The resource owner still validates the exact fitted store after admission.
    if is_repair_crew_order(config, target, payload)
        || (config.system(target).is_some()
            && matches!(
                payload,
                SystemControlPayload::SetStockTarget { .. }
                    | SystemControlPayload::RequestEquipment { .. }
            ))
    {
        sources.crew_order_policy_for(target)
    } else {
        sources.policy_for(target)
    }
}

/// Resolve authority through the System's Console and current hosting Station.
/// Generated equipment must carry the same explicit ownership as authored gear;
/// an unknown target never acquires authority from its spelling.
pub fn station_for_system(
    config: &crate::ship::config::ShipConfig,
    hosts: Option<&crate::ship_plugin::HumanSeekingHosts>,
    target: &crate::core::messages::SystemId,
) -> Option<StationId> {
    // Step 0: the live seek result wins over the authored station.
    if let Some(host) = hosts.and_then(|h| h.host_for(target)) {
        return Some(host.clone());
    }
    // Step 2: direct system lookup.
    if let Some(system) = config.system(target) {
        return config.system_home_station(system).cloned();
    }
    // Step 3: unknown.
    None
}

/// Payload-aware System target used by every authority path. A viewscreen is
/// only the transport target for `SetView`; authority and availability belong
/// to the authored System that supplies the selected view.
pub fn effective_target_for_command(
    config: &crate::ship::config::ShipConfig,
    target: &crate::core::messages::SystemId,
    payload: &SystemControlPayload,
) -> crate::core::messages::SystemId {
    let is_viewscreen_target = config
        .system(target)
        .is_some_and(|system| system.kind == crate::ship::system_registry::VIEWSCREEN_KIND)
        || (target.0 == crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID
            && !config
                .systems
                .iter()
                .any(|system| system.kind == crate::ship::system_registry::VIEWSCREEN_KIND));
    if !is_viewscreen_target {
        return target.clone();
    }
    let SystemControlPayload::SetView { mode } = payload else {
        return target.clone();
    };
    let source_kind = match mode {
        crate::core::messages::ViewMode::Camera(_) | crate::core::messages::ViewMode::Cinematic => {
            crate::ship::system_registry::CAPTAIN_KIND
        }
        crate::core::messages::ViewMode::Radar => crate::ship::system_registry::HELM_RADAR_KIND,
        crate::core::messages::ViewMode::ScienceRadar
        | crate::core::messages::ViewMode::SensorsRadar => {
            crate::ship::system_registry::SENSORS_KIND
        }
        crate::core::messages::ViewMode::SystemChart
        | crate::core::messages::ViewMode::NavigationChart => {
            crate::ship::system_registry::NAVIGATION_KIND
        }
        crate::core::messages::ViewMode::Comms => crate::ship::system_registry::COMMS_KIND,
    };
    config
        .systems
        .iter()
        .find(|system| system.kind == source_kind)
        .map(|system| system.id.clone())
        .unwrap_or_else(|| crate::ship::viewscreen::source_system_for_view_mode(mode))
}

/// Refusal from the payload-aware Station authority/availability half of
/// System Admission. Authentication is deliberately absent: the caller has
/// already authenticated one GM operator and substitutes the selected Station
/// as authority at this seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StationCommandPolicyFailure {
    SystemOutsideStation,
    SystemUnavailable,
}

/// Validate an authenticated GM Station command against the same effective
/// target and live availability used by ordinary System Admission. This does
/// not require the resolver's source to be Human: a legitimate takeover starts
/// while the Station is Backfill (`Ai`) and substitutes Station authority at
/// this boundary. Damage- and rating-offline Systems remain unavailable.
pub fn authorize_station_command(
    station: &StationId,
    target: &crate::core::messages::SystemId,
    payload: &SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    config: &crate::ship::config::ShipConfig,
    hosts: Option<&crate::ship_plugin::HumanSeekingHosts>,
    console_hosts: Option<&crate::ship_plugin::ConsoleHosts>,
) -> Result<crate::core::messages::SystemId, StationCommandPolicyFailure> {
    if let SystemControlPayload::AssignConsoleView { console, view } = payload {
        let Some(definition) = config.console(console) else {
            return Err(StationCommandPolicyFailure::SystemOutsideStation);
        };
        let live_host = console_hosts
            .and_then(|hosts| hosts.0.iter().find(|host| &host.console == console))
            .and_then(|host| host.host.as_ref());
        let owner = live_host.or(definition.home_station.as_ref());
        return if target.0 == "assign-console-view" && view.is_valid() && owner == Some(station) {
            Ok(target.clone())
        } else {
            Err(StationCommandPolicyFailure::SystemOutsideStation)
        };
    }
    let effective_target = effective_target_for_command(config, target, payload);
    if station_for_system(config, hosts, &effective_target).as_ref() != Some(station) {
        return Err(StationCommandPolicyFailure::SystemOutsideStation);
    }
    if !command_policy(config, &control_sources.0, &effective_target, payload).coordinate {
        return Err(StationCommandPolicyFailure::SystemUnavailable);
    }
    Ok(effective_target)
}
