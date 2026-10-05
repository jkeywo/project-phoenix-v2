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

use crate::core::messages::SystemControlPayload;

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
    let summary = config
        .system(&effective_target)
        .is_some_and(|s| s.kind == crate::ship::system_registry::REPAIR_KIND)
        && matches!(
            payload,
            SystemControlPayload::SetRepairPriority { .. }
                | SystemControlPayload::SetRepairTargetPriority { .. }
        );

    use phoenix_sim_contracts::authority::{
        authorize_command, CommandAuthorization, CommandClaimant, CommandTargetPolicy,
    };
    let claimant = if token.starts_with("ai:") {
        CommandClaimant::Ai
    } else if token == crate::console_bridge::LOCAL_CONSOLE_TOKEN {
        CommandClaimant::LocalConsole
    } else {
        CommandClaimant::Crew {
            spectator: sessions.0.is_spectator(token),
            afk: sessions.0.is_afk(token),
            registered: sessions
                .0
                .players()
                .iter()
                .any(|player| player.token == token),
            holds_station: station_for_system(config, hosts, &effective_target)
                .map(|station| sessions.0.holder_for_station(&station) == Some(token)),
        }
    };
    match authorize_command(
        claimant,
        CommandTargetPolicy {
            control: policy,
            summary,
            debug_route: crate::command_admission::debug_route::admits_debug_command(
                &effective_target,
                payload,
            ),
        },
    ) {
        CommandAuthorization::Allowed => true,
        CommandAuthorization::Denied => false,
        CommandAuthorization::UnknownSystem => {
            warn!(target: crate::logging::LogCat::Admit.target(),
                "unknown system id {:?} — denying", effective_target.0);
            false
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;

pub use phoenix_sim_gameplay::command_admission::policy::*;
