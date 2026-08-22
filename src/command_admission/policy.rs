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

pub fn is_command_authorized(
    token: &str,
    target: &crate::core::messages::SystemId,
    payload: &SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    config: &crate::ship::config::ShipConfig,
    hosts: Option<&crate::ship_plugin::HumanSeekingHosts>,
) -> bool {
    // Viewscreen SetView: authority derives from the view mode's source system.
    let effective_target = if target.0 == crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID {
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
    if sessions.0.is_spectator(token) {
        return false;
    }

    if !policy.accept_human_input {
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
mod tests {
    use super::*;
    use crate::core::messages::{RepairTarget, SystemId};
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
            None,
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
            None,
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
            None,
        ));
        assert!(!is_command_authorized(
            "ai:backfill",
            &SystemId("repair".into()),
            &dispatch(),
            &sources,
            &sessions_with_repair_holder("t1"),
            &config(),
            None,
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
            None,
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
            None,
        ));
        assert!(is_command_authorized(
            "ai:backfill",
            &SystemId("repair".into()),
            &dispatch(),
            &sources,
            &sessions_with_repair_holder("t1"),
            &config(),
            None,
        ));
    }

    // ── Shield-arc station resolution ─────────────────────────────────────
    //
    // `station_for_system` must resolve `shield-arc-*` to the station that
    // owns THIS ship's shields, not a hardcoded "shields" — the destroyer's
    // arcs live on "engineering", the cruiser's on "science", the courier's
    // on "captain". Only the battleship has a literal "shields" station.

    /// A hull whose `kind = "shields"` system is owned by the engineering
    /// station, with synthesised per-arc entries mirroring what
    /// `EntityConfig::from_toml_in_mode` appends.
    fn config_with_shields_on_engineering() -> crate::ship::config::ShipConfig {
        crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "engineering"
name = "Engineering"
description = "Shields and power."
rank = "Ltn."

[[system]]
id = "shields-system"
kind = "shields"
station = "engineering"

[[system]]
id = "shield-arc-fore"
kind = "shield_arc"
station = "engineering"

[[system]]
id = "shield-arc-aft"
kind = "shield_arc"
station = "engineering"
"#,
            &["shields", "shield_arc"],
        )
        .unwrap()
    }

    /// A hull whose arcs are ownerless AI-only systems, as synthesised for an
    /// NPC ship with no `kind = "shields"` system and no "shields" station.
    fn config_with_ownerless_arcs() -> crate::ship::config::ShipConfig {
        crate::ship::config::ShipConfig::from_toml(
            r#"
[[system]]
id = "shield-arc-all"
kind = "shield_arc"
ai_only = true
"#,
            &["shield_arc"],
        )
        .unwrap()
    }

    #[test]
    fn station_for_system_resolves_shield_arcs_to_the_ships_owning_station() {
        assert_eq!(
            station_for_system(
                &config_with_shields_on_engineering(),
                None,
                &SystemId("shield-arc-fore".into()),
            ),
            Some(StationId("engineering".into())),
            "a hull whose shields live on engineering must authorise that station's holder"
        );
        assert_eq!(
            station_for_system(
                &config_with_shields_on_engineering(),
                None,
                &SystemId("shield-arc-aft".into()),
            ),
            Some(StationId("engineering".into())),
        );
    }

    #[test]
    fn station_for_system_resolves_ownerless_arcs_to_none() {
        assert_eq!(
            station_for_system(
                &config_with_ownerless_arcs(),
                None,
                &SystemId("shield-arc-all".into())
            ),
            None,
            "an NPC's ownerless arc must not resolve to a human-held station"
        );
    }

    #[test]
    fn station_for_system_falls_back_to_literal_shields_station_when_config_has_no_arc() {
        let empty = crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "shields"
name = "Shields"
description = "Defensive grid."
rank = "Ltn."

[[system]]
id = "repair"
kind = "repair_control"
station = "shields"
"#,
            &["repair_control"],
        )
        .unwrap();
        assert_eq!(
            station_for_system(&empty, None, &SystemId("shield-arc-fore".into())),
            Some(StationId("shields".into())),
            "legacy fixtures with no synthesised arcs keep the historical shields-station mapping"
        );
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
            None,
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
            &SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into()),
            &toggle_god_mode(),
            &sources(ControlSource::Human, false),
            &sessions_with_repair_holder("t1"),
            &config(),
            None,
        ));
    }

    /// A connected player's session token now reaches God Mode through the
    /// phone settings menu's Debug/Cheat route (issue #940) — but only in a
    /// build that has that route. In the demo build
    /// `debug_route::admits_debug_command` is compiled to a constant `false`
    /// and the token falls through to the pre-#940 behaviour:
    /// `station_for_system` returns `None` for `god-mode` (no ship TOML
    /// declares it) and the unknown-system deny path refuses it.
    #[test]
    fn toggle_god_mode_follows_the_debug_route_for_a_remote_human_token() {
        assert_eq!(
            is_command_authorized(
                "t1",
                &SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into()),
                &toggle_god_mode(),
                &sources(ControlSource::Human, false),
                &sessions_with_repair_holder("t1"),
                &config(),
                None,
            ),
            !crate::build_flags::is_demo_cfg(),
        );
    }

    /// The route is not a blanket "remote tokens may touch `god-mode`": an
    /// unregistered token — one that never completed `Identify`, so it is in no
    /// session — is refused even in a dev build. Registration is the whole of
    /// the identity claim being made, and it has to be a real one.
    #[test]
    fn toggle_god_mode_is_rejected_for_a_token_with_no_session() {
        assert!(!is_command_authorized(
            "never-identified",
            &SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into()),
            &toggle_god_mode(),
            &sources(ControlSource::Human, false),
            &sessions_with_repair_holder("t1"),
            &config(),
            None,
        ));
    }

    /// The route widens exactly one (target, payload) pair. A connected player
    /// who does not hold the repair station still cannot dispatch repair teams
    /// — #940 must not have turned admission into "any registered token, any
    /// ownerless-looking command".
    #[test]
    fn the_debug_route_does_not_widen_ordinary_station_commands() {
        assert!(!is_command_authorized(
            "intruder",
            &SystemId("repair".into()),
            &dispatch(),
            &sources(ControlSource::Human, false),
            &{
                let mut sm = crate::lobby::session::SessionManager::new();
                sm.register("t1".into(), "Engineer".into()).unwrap();
                sm.set_station("t1", Some(StationId("repair".into())));
                // `intruder` is a registered player too — the deny below is
                // station tenure, not an unknown token.
                sm.register("intruder".into(), "Nosy".into()).unwrap();
                crate::lobby::Sessions(sm)
            },
            &config(),
            None,
        ));
    }

    /// An `ai:`-prefixed token is denied without any god-mode-specific check:
    /// the default (unregistered) policy has `operate_ai: false`.
    #[test]
    fn toggle_god_mode_is_rejected_for_an_ai_token() {
        assert!(!is_command_authorized(
            "ai:backfill",
            &SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into()),
            &toggle_god_mode(),
            &sources(ControlSource::Human, false),
            &sessions_with_repair_holder("t1"),
            &config(),
            None,
        ));
    }

    // ── Spectator admission (issue #1105, AC3) ───────────────────────────────

    /// A registered spectator holds a repair session but no station. Sessions
    /// helper: register the token, mark it a spectator (which vacates any seat).
    fn sessions_with_spectator(token: &str) -> crate::lobby::Sessions {
        let mut sm = crate::lobby::session::SessionManager::new();
        sm.register(token.into(), "Watcher".into()).unwrap();
        sm.set_spectator(token, true);
        crate::lobby::Sessions(sm)
    }

    /// AC3: a spectator is refused an ordinary station-owned command. (It would
    /// be denied by station tenure anyway — a spectator holds no seat — but the
    /// explicit early rejection is what makes it robust and un-bypassable.)
    #[test]
    fn spectator_is_refused_a_station_command() {
        assert!(!is_command_authorized(
            "spec",
            &SystemId("repair".into()),
            &dispatch(),
            &sources(ControlSource::Human, false),
            &sessions_with_spectator("spec"),
            &config(),
            None,
        ));
    }

    /// AC3, the load-bearing case: a spectator is refused the ownerless
    /// Debug/God-Mode route. That route admits ANY registered player, and a
    /// spectator IS registered — so only the explicit spectator rejection closes
    /// it. In a demo build the route is compiled out and refusal is trivially
    /// true; in a dev build the refusal is entirely down to the spectator gate.
    #[test]
    fn spectator_is_refused_the_debug_god_mode_route() {
        assert!(!is_command_authorized(
            "spec",
            &SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into()),
            &toggle_god_mode(),
            &sources(ControlSource::Human, false),
            &sessions_with_spectator("spec"),
            &config(),
            None,
        ));
    }

    /// The companion that proves the previous test is about the spectator gate,
    /// not a closed route: the SAME token, registered but NOT a spectator, IS
    /// admitted by the debug route in a dev build.
    #[test]
    fn a_non_spectator_registered_token_still_reaches_the_debug_route() {
        let sessions = {
            let mut sm = crate::lobby::session::SessionManager::new();
            sm.register("player".into(), "Player".into()).unwrap();
            crate::lobby::Sessions(sm)
        };
        assert_eq!(
            is_command_authorized(
                "player",
                &SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into()),
                &toggle_god_mode(),
                &sources(ControlSource::Human, false),
                &sessions,
                &config(),
                None,
            ),
            !crate::build_flags::is_demo_cfg(),
        );
    }

    // ── Command station SetStationStance admission (issue #1107, AC6/AC3) ─────
    //
    // The Command station (`auxiliary = true`, `human_seeking = true`) owns the
    // `command` system but is authored on no fixed seat — it is hosted, this
    // tick, wherever `resolve_human_seeking_hosts` put it (its `host_order`
    // starts with the Captain). So a `SetStationStance` order must be admitted
    // for whichever token holds THAT host station and refused for everyone else,
    // resolved through the same `HumanSeekingHosts` seam #984 built and the same
    // `is_command_authorized` public path every other console command takes —
    // never by pushing an `AdmittedCommand` past the gate. `command`'s
    // ownership resolves through step 0 (the live seek), so this exercises the
    // real authorization boundary AC3 depends on.

    /// A hull with a Captain seat and the auxiliary, human-seeking Command
    /// station that owns the `command` system. `command_target` is deliberately
    /// omitted so no stance catalogue is required — admission turns on station
    /// tenure, not on the stance content this fixture is not about.
    fn command_config() -> crate::ship::config::ShipConfig {
        crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."

[[station]]
id = "command"
name = "Command"
description = "Direct an AI station."
rank = "Cpt."
auxiliary = true
human_seeking = true
host_order = ["captain"]
visiting_rating = "Std"

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "command"
kind = "command"
station = "command"
"#,
            &["command"],
        )
        .unwrap()
    }

    /// The live seek result production writes every tick: the auxiliary Command
    /// station is hosted on the Captain seat, so its `command` system's
    /// authoritative station is `captain`, NOT the `command` station it authors.
    fn command_hosted_on_captain() -> crate::ship_plugin::HumanSeekingHosts {
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            crate::ship::system_registry::command_system_id(),
            StationId("captain".into()),
        );
        crate::ship_plugin::HumanSeekingHosts(map)
    }

    /// `command` under the given control source. Human is the shape a Captain
    /// actively directing the board produces; Ai is the backfilled shape.
    fn command_sources(source: ControlSource) -> ShipSystemControlSources {
        let mut resolver = ControlSourceResolver::new();
        resolver.set(crate::ship::system_registry::command_system_id(), source);
        ShipSystemControlSources(resolver)
    }

    fn set_stance() -> SystemControlPayload {
        SystemControlPayload::SetStationStance {
            station: StationId("tactical".into()),
            stance: "hold".into(),
        }
    }

    fn sessions_with_captain(token: &str) -> crate::lobby::Sessions {
        let mut sm = crate::lobby::session::SessionManager::new();
        sm.register(token.into(), "Kirk".into()).unwrap();
        sm.set_station(token, Some(StationId("captain".into())));
        crate::lobby::Sessions(sm)
    }

    /// AC3/AC6 happy path: the token holding the resolved Command host station
    /// (the Captain) is admitted to direct the board.
    #[test]
    fn set_station_stance_is_admitted_for_the_resolved_command_host() {
        assert!(is_command_authorized(
            "captain-token",
            &crate::ship::system_registry::command_system_id(),
            &set_stance(),
            &command_sources(ControlSource::Human),
            &sessions_with_captain("captain-token"),
            &command_config(),
            Some(&command_hosted_on_captain()),
        ));
    }

    /// AC6 rejection: a registered player who holds no station that hosts
    /// Command cannot issue the order, even though the target and payload are
    /// well-formed. The deny is station tenure — the seek put Command on the
    /// Captain, and this token is not the Captain.
    #[test]
    fn set_station_stance_is_refused_for_a_token_without_the_command_host() {
        let sessions = {
            let mut sm = crate::lobby::session::SessionManager::new();
            sm.register("captain-token".into(), "Kirk".into()).unwrap();
            sm.set_station("captain-token", Some(StationId("captain".into())));
            // A connected player seated on Tactical — a real session, so the
            // deny below is tenure of the Command host, not an unknown token.
            sm.register("intruder".into(), "Nosy".into()).unwrap();
            sm.set_station("intruder", Some(StationId("tactical".into())));
            crate::lobby::Sessions(sm)
        };
        assert!(!is_command_authorized(
            "intruder",
            &crate::ship::system_registry::command_system_id(),
            &set_stance(),
            &command_sources(ControlSource::Human),
            &sessions,
            &command_config(),
            Some(&command_hosted_on_captain()),
        ));
    }

    /// AGENTS.md rule 6 — identity is stripped past admission and humans/AI are
    /// symmetric: when the Command system is backfilled to AI, the `ai:` token
    /// is admitted on the same terms the human host was, with no origin-specific
    /// branch. This mirrors `dispatch_to_an_ai_controlled_system_rejects_humans_and_admits_ai`.
    #[test]
    fn set_station_stance_admits_ai_and_refuses_humans_while_command_is_ai() {
        let sources = command_sources(ControlSource::Ai);
        assert!(!is_command_authorized(
            "captain-token",
            &crate::ship::system_registry::command_system_id(),
            &set_stance(),
            &sources,
            &sessions_with_captain("captain-token"),
            &command_config(),
            Some(&command_hosted_on_captain()),
        ));
        assert!(is_command_authorized(
            "ai:command",
            &crate::ship::system_registry::command_system_id(),
            &set_stance(),
            &sources,
            &sessions_with_captain("captain-token"),
            &command_config(),
            Some(&command_hosted_on_captain()),
        ));
    }

    // ── AFK stale-command refusal (issue #1104, AC5) ─────────────────────────
    //
    // AFK adds NO new admission branch. Entering AFK delegates the holder's own
    // Station to AI (Backfill → every owned system `ControlSource::Ai`), and it
    // relocates human-seeking Stations off the AFK holder via the live
    // `HumanSeekingHosts` seam. Both stale-command refusals fall out of the two
    // gates `is_command_authorized` already applies: `!accept_human_input` and
    // station tenure. These pin that they do.

    /// AC5, refusal 1: the AFK player's OWN direct-station command is refused
    /// because entering AFK delegated that station to AI, so `accept_human_input`
    /// is false — the very first human gate `is_command_authorized` checks.
    #[test]
    fn an_afk_delegated_holders_own_command_is_refused_via_accept_human_input() {
        let sessions = {
            let mut sm = crate::lobby::session::SessionManager::new();
            sm.register("t1".into(), "Engineer".into()).unwrap();
            sm.set_station("t1", Some(StationId("repair".into())));
            sm.set_afk("t1", true); // AFK delegated the seat to AI…
            crate::lobby::Sessions(sm)
        };
        assert!(
            !is_command_authorized(
                "t1",
                &SystemId("repair".into()),
                &dispatch(),
                &sources(ControlSource::Ai, false), // …so its system runs on AI.
                &sessions,
                &config(),
                None,
            ),
            "an AFK holder's own command is refused because the seat is AI"
        );
        // The companion that proves the deny is the delegation, not the token:
        // the SAME holder, seat still Human (not AFK-delegated), is admitted.
        assert!(is_command_authorized(
            "t1",
            &SystemId("repair".into()),
            &dispatch(),
            &sources(ControlSource::Human, false),
            &sessions,
            &config(),
            None,
        ));
    }

    /// AC5, refusal 2: after an AFK holder's visiting Station re-resolves to a
    /// new host, the STALE prior host — no longer holding the station the live
    /// `HumanSeekingHosts` seam puts the system on — is refused by the ordinary
    /// tenure check. Here `command` has relocated off the now-AFK Captain onto
    /// Tactical; the Captain's stale order is refused because the live host is
    /// Tactical, held by someone else.
    #[test]
    fn a_stale_prior_visiting_host_is_refused_via_tenure_after_afk_relocation() {
        let hosts = {
            let mut map = std::collections::BTreeMap::new();
            map.insert(
                crate::ship::system_registry::command_system_id(),
                StationId("tactical".into()),
            );
            crate::ship_plugin::HumanSeekingHosts(map)
        };
        let sessions = {
            let mut sm = crate::lobby::session::SessionManager::new();
            // The relocated (present) Tactical officer is the live Command host.
            sm.register("tac".into(), "Sulu".into()).unwrap();
            sm.set_station("tac", Some(StationId("tactical".into())));
            // The prior host stepped AFK but still holds Captain — its order is
            // stale now that the seek has moved Command to Tactical.
            sm.register("cap".into(), "Kirk".into()).unwrap();
            sm.set_station("cap", Some(StationId("captain".into())));
            sm.set_afk("cap", true);
            crate::lobby::Sessions(sm)
        };
        assert!(
            !is_command_authorized(
                "cap",
                &crate::ship::system_registry::command_system_id(),
                &set_stance(),
                &command_sources(ControlSource::Human),
                &sessions,
                &command_config(),
                Some(&hosts),
            ),
            "the stale prior host is refused — the live host is Tactical"
        );
        // And the relocated host IS admitted, proving the seek moved authority.
        assert!(is_command_authorized(
            "tac",
            &crate::ship::system_registry::command_system_id(),
            &set_stance(),
            &command_sources(ControlSource::Human),
            &sessions,
            &command_config(),
            Some(&hosts),
        ));
    }
}
