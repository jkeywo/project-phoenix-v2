//! The admission *policy* — the pure predicate that decides whether one
//! command from one token may act on one system.
//!
//! Separated from the admission seam (`super`) so the policy question
//! ("is this allowed?") is testable and namable independently of the Bevy
//! system that applies it once per tick. This module is the single file
//! named by the PASM entity `system-command-admission-policy`.
//!
//! No gameplay values live here: station ownership, per-system control
//! sources, and damage-driven availability all arrive from the ship's TOML
//! config and the live simulation state.

use bevy::prelude::warn;

use crate::core::messages::{StationId, SystemControlPayload};

/// Maps a `SystemId` to the `StationId` whose holder is authoritative for
/// that system's admission. Returns `None` for systems with no owning
/// station (either ship-wide or unknown), signalling a deny at the
/// caller.
///
/// Lookup order:
///   0. The live human-seeking host map (issue #984), when the caller has one.
///      A `human_seeking` system is authoritative wherever this tick's seek put
///      it, which is NOT necessarily the station its `[[system]]` block
///      authors — the destroyer's Comms officer may be sitting on `captain`.
///   1. Shield-arc prefix match — arcs are not auto-generated into
///      `ShipConfig.systems` (they're synthesised at the entity-config layer),
///      so they must be matched by prefix.
///   2. Direct system→station from the config's `[[system]]` blocks
///      (handles fine-grained systems and modern coarse systems).
///   3. `None` — truly unknown system id, caller will deny.
///
/// The former station-name fallback (target string matches a station id) was
/// removed in issue #832: since #801/#822 every wire `target` a client emits
/// names a declared `[[system]]` id, so it always resolves at step 1 or 2.
///
/// `hosts` is `Option` because most callers have no ship entity in hand — and
/// because a ship that authors no `human_seeking` system never grows the
/// component. `None` is exactly the pre-#984 behaviour.
///
/// NEVER shortcut this with `StationId(system_id.0)`. `SystemId("comms")` and
/// `StationId("comms")` are different types naming different things that merely
/// coincide on the cruiser and battleship; that coincidence is what hid the
/// destroyer/courier `CommsState` bug for as long as it did.
pub fn station_for_system(
    config: &crate::ship::config::ShipConfig,
    hosts: Option<&crate::ship_plugin::HumanSeekingHosts>,
    target: &crate::core::messages::SystemId,
) -> Option<StationId> {
    // Step 0: the live seek result wins over the authored station.
    if let Some(host) = hosts.and_then(|h| h.host_for(target)) {
        return Some(host.clone());
    }
    // Step 1: shield-arc prefix. Arcs are synthesised into `config.systems`
    // by the entity-config layer (`EntityConfig::from_toml_in_mode`, which
    // appends a `kind = "shield_arc"` entry per `[[shield_arc]]`) carrying
    // the owning station of the ship's `kind = "shields"` system — e.g. the
    // destroyer's arcs live on "engineering". Resolve through the config so
    // the holder of THAT station is authoritative. Ownerless NPC arcs carry
    // `station: None`, which correctly denies humans. Only when the config
    // has no arc entry at all (legacy/test fixtures whose `ShipConfigComponent`
    // predates arc synthesis) fall back to a literal "shields" station.
    if target.0.starts_with("shield-arc-") {
        if let Some(system) = config.system(target) {
            return system.station.clone();
        }
        return Some(StationId("shields".into()));
    }
    // Step 2: direct system lookup.
    if let Some(system) = config.system(target) {
        return system.station.clone();
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
) -> Result<crate::core::messages::SystemId, StationCommandPolicyFailure> {
    let effective_target = effective_target_for_command(config, target, payload);
    if station_for_system(config, hosts, &effective_target).as_ref() != Some(station) {
        return Err(StationCommandPolicyFailure::SystemOutsideStation);
    }
    if !control_sources.0.policy_for(&effective_target).coordinate {
        return Err(StationCommandPolicyFailure::SystemUnavailable);
    }
    Ok(effective_target)
}

pub fn is_command_authorized(
    token: &str,
    target: &crate::core::messages::SystemId,
    payload: &SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    config: &crate::ship::config::ShipConfig,
    hosts: Option<&crate::ship_plugin::HumanSeekingHosts>,
) -> bool {
    let effective_target = effective_target_for_command(config, target, payload);

    let policy = control_sources.0.policy_for(&effective_target);
    // Summary intent never shares an actuator with the AI. Repair priority
    // changes the ordinary sweep's policy input; dispatch/recall remain AI-only.
    let summary = policy.accept_summary_input
        && config
            .system(&effective_target)
            .is_some_and(|s| s.kind == crate::ship::system_registry::REPAIR_KIND)
        && matches!(
            payload,
            SystemControlPayload::SetRepairPriority { .. }
                | SystemControlPayload::SetRepairTargetPriority { .. }
        );

    if token.starts_with("ai:") {
        return policy.operate_ai;
    }
    if token == crate::console_bridge::LOCAL_CONSOLE_TOKEN {
        return policy.accept_human_input || summary;
    }

    // A Spectator (issue #1105) is a registered, connected player with no
    // Station and no simulation authority. Reject every simulation command from
    // one explicitly and up front — this is the robust single gate rather than
    // relying on the *absence* of a seat. A station-owned command is already
    // denied by the `holder_for_station` tenure check below, but the ownerless
    // Debug/God-Mode route further down admits ANY registered player, and a
    // spectator IS a registered player — so without this, a spectator could
    // still fire the God-Mode/debug cheats. Placed after the `ai:` and
    // LOCAL_CONSOLE_TOKEN branches (those tokens are never spectators) and
    // before the debug route it closes.
    if sessions.0.is_spectator(token) || (summary && sessions.0.is_afk(token)) {
        return false;
    }

    if !policy.accept_human_input && !summary {
        return false;
    }

    // The phone client's Debug/Cheat route (issue #940). Ownerless by
    // construction — no station holds `god-mode` — so the tenure check below
    // would deny it, which is what kept cheats host-only until now. The verdict
    // turns on the (target, payload) pair and the sender being a connected
    // player, never on *which* player: a phone that gets here is admitted on
    // the same terms as any other, and downstream sees the ordinary admitted
    // `ToggleGodMode` with its source identity stripped.
    //
    // `crate::command_admission::debug_route` compiles this route out of a demo
    // build, so the check below is a constant `false` there — the gate and the
    // hidden tab disappear together.
    if crate::command_admission::debug_route::admits_debug_command(&effective_target, payload)
        && sessions.0.players().iter().any(|p| p.token == token)
    {
        return true;
    }

    // Human network token: must hold the station for the target system — the
    // station the seek put it on, when it is human-seeking (issue #984).
    match station_for_system(config, hosts, &effective_target) {
        Some(station) => sessions.0.holder_for_station(&station) == Some(token),
        None => {
            // Plain fn, no `LogFilterConfig` in scope — a bare targeted `warn!`
            // rather than growing a parameter for it. See `crate::logging`.
            warn!(
                target: crate::logging::LogCat::Admit.target(),
                "unknown system id {:?} — denying", effective_target.0
            );
            false
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
