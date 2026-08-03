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

use crate::messages::{StationId, SystemControlPayload};

/// Maps a `SystemId` to the `StationId` whose holder is authoritative for
/// that system's admission. Returns `None` for systems with no owning
/// station (either ship-wide or unknown), signalling a deny at the
/// caller.
///
/// Lookup order:
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
pub fn station_for_system(
    config: &crate::ship::config::ShipConfig,
    target: &crate::messages::SystemId,
) -> Option<StationId> {
    // Step 1: shield-arc prefix (arcs are not in `config.systems`).
    if target.0.starts_with("shield-arc-") {
        return Some(StationId("shields".into()));
    }
    // Step 2: direct system lookup.
    if let Some(system) = config.system(target) {
        return system.station.clone();
    }
    // Step 3: unknown.
    None
}

pub fn is_command_authorized(
    token: &str,
    target: &crate::messages::SystemId,
    payload: &SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    config: &crate::ship::config::ShipConfig,
) -> bool {
    // Viewscreen SetView: authority derives from the view mode's source system.
    let effective_target = if target.0 == crate::system_registry::VIEWSCREEN_SYSTEM_ID {
        if let SystemControlPayload::SetView { mode } = payload {
            crate::ship::viewscreen::source_system_for_view_mode(mode)
        } else {
            target.clone()
        }
    } else {
        target.clone()
    };

    let policy = control_sources.0.policy_for(&effective_target);

    if token.starts_with("ai:") {
        return policy.operate_ai;
    }
    if token == crate::console_bridge::LOCAL_CONSOLE_TOKEN {
        return policy.accept_human_input;
    }
    if !policy.accept_human_input {
        return false;
    }

    // Human network token: must hold the station for the target system.
    match station_for_system(config, &effective_target) {
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
mod tests {
    use super::*;
    use crate::messages::{RepairTarget, SystemId};
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    use crate::ship_plugin::ShipSystemControlSources;

    /// A minimal hull with a repair station that owns one repair system, so
    /// `station_for_system` can resolve `repair` → the `repair` station.
    fn config() -> crate::ship::config::ShipConfig {
        crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "repair"
name = "Engineering"
description = "Damage control."
rank = "Ltn."

[[system]]
id = "repair"
kind = "repair_control"
station = "repair"
"#,
            &["repair_control"],
        )
        .unwrap()
    }

    fn sessions_with_repair_holder(token: &str) -> crate::lobby::Sessions {
        let mut sm = crate::lobby::session::SessionManager::new();
        sm.register(token.into(), "Engineer".into()).unwrap();
        sm.set_station(token, Some(StationId("repair".into())));
        crate::lobby::Sessions(sm)
    }

    fn dispatch() -> SystemControlPayload {
        SystemControlPayload::DispatchRepairTeam {
            team_idx: 0,
            target: RepairTarget::Core,
        }
    }

    fn sources(source: ControlSource, offline: bool) -> ShipSystemControlSources {
        let mut resolver = ControlSourceResolver::new();
        resolver.set(SystemId("repair".into()), source);
        if offline {
            resolver.set_offline(SystemId("repair".into()), true);
        }
        ShipSystemControlSources(resolver)
    }

    /// The happy path the Repair console takes: the station holder's typed
    /// dispatch is admitted.
    #[test]
    fn repair_station_holder_is_admitted() {
        assert!(is_command_authorized(
            "t1",
            &SystemId("repair".into()),
            &dispatch(),
            &sources(ControlSource::Human, false),
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
    }

    /// Rejection class 1 — unauthorised: a connected player who does not hold
    /// the repair station cannot dispatch repair teams.
    #[test]
    fn dispatch_from_a_non_repair_token_is_rejected() {
        assert!(!is_command_authorized(
            "intruder",
            &SystemId("repair".into()),
            &dispatch(),
            &sources(ControlSource::Human, false),
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
    }

    /// Rejection class 2 — unavailable: a system explicitly rated Offline
    /// accepts neither human nor AI input.
    #[test]
    fn dispatch_to_an_offline_rated_system_is_rejected() {
        let sources = sources(ControlSource::Offline, false);
        assert!(!is_command_authorized(
            "t1",
            &SystemId("repair".into()),
            &dispatch(),
            &sources,
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
        assert!(!is_command_authorized(
            "ai:backfill",
            &SystemId("repair".into()),
            &dispatch(),
            &sources,
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
    }

    /// Rejection class 3 — damaged: `offline_systems` (driven by damage tier)
    /// overrides the station rating, so even the rightful holder is denied.
    #[test]
    fn dispatch_to_a_damaged_system_is_rejected() {
        assert!(!is_command_authorized(
            "t1",
            &SystemId("repair".into()),
            &dispatch(),
            &sources(ControlSource::Human, true),
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
    }

    /// Rejection class 4 — AI-controlled: while the system is under AI
    /// control, human input is refused and the AI token is admitted instead.
    #[test]
    fn dispatch_to_an_ai_controlled_system_rejects_humans_and_admits_ai() {
        let sources = sources(ControlSource::Ai, false);
        assert!(!is_command_authorized(
            "t1",
            &SystemId("repair".into()),
            &dispatch(),
            &sources,
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
        assert!(is_command_authorized(
            "ai:backfill",
            &SystemId("repair".into()),
            &dispatch(),
            &sources,
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
    }

    /// An unknown system id is denied outright — the router never sees it.
    #[test]
    fn dispatch_to_an_unknown_system_is_rejected() {
        assert!(!is_command_authorized(
            "t1",
            &SystemId("no-such-system".into()),
            &dispatch(),
            &sources(ControlSource::Human, false),
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
    }

    // ── God Mode authority (issue #900) ─────────────────────────────────────
    //
    // `god-mode` (`GOD_MODE_SYSTEM_ID`) is deliberately declared by no ship
    // TOML — `config()` above has no `[[system]] id = "god-mode"` block — so
    // these tests exercise the exact three authority branches
    // `is_command_authorized` offers, with no god-mode-specific code added to
    // reach them (AGENTS.md constraint 6: no origin branch beyond ordinary
    // token authority).

    fn toggle_god_mode() -> SystemControlPayload {
        SystemControlPayload::ToggleGodMode
    }

    /// The host console is the only token this is ever admitted for: the
    /// `LOCAL_CONSOLE_TOKEN` branch checks `policy.accept_human_input`
    /// only, never station tenure, and `ControlSourceResolver::policy_for`
    /// defaults an unregistered `SystemId` to `ControlSource::Human`
    /// (`accept_human_input: true`).
    #[test]
    fn toggle_god_mode_is_admitted_for_the_local_console_token() {
        assert!(is_command_authorized(
            crate::console_bridge::LOCAL_CONSOLE_TOKEN,
            &SystemId(crate::system_registry::GOD_MODE_SYSTEM_ID.into()),
            &toggle_god_mode(),
            &sources(ControlSource::Human, false),
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
    }

    /// A connected player's session token is denied: `station_for_system`
    /// returns `None` for `god-mode` (no ship TOML declares it), which is the
    /// "unknown system" deny path every remote human token hits.
    #[test]
    fn toggle_god_mode_is_rejected_for_a_remote_human_token() {
        assert!(!is_command_authorized(
            "t1",
            &SystemId(crate::system_registry::GOD_MODE_SYSTEM_ID.into()),
            &toggle_god_mode(),
            &sources(ControlSource::Human, false),
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
    }

    /// An `ai:`-prefixed token is denied without any god-mode-specific check:
    /// the default (unregistered) policy has `operate_ai: false`.
    #[test]
    fn toggle_god_mode_is_rejected_for_an_ai_token() {
        assert!(!is_command_authorized(
            "ai:backfill",
            &SystemId(crate::system_registry::GOD_MODE_SYSTEM_ID.into()),
            &toggle_god_mode(),
            &sources(ControlSource::Human, false),
            &sessions_with_repair_holder("t1"),
            &config(),
        ));
    }
}
