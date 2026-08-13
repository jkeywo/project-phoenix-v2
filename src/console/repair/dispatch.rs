//! Host repair router — the dispatch seam for `DispatchRepairTeam`.
//!
//! This module is the single file named by the PASM entity
//! `host-repair-router`. It exists as its own module (issue #736) so the
//! router's dependency on the host command-admission seam is a real, observed
//! code edge (the `use crate::command_admission::…;` below) rather than a
//! claim made only in prose.
//!
//! The router never inspects who sent a command. Every command it reads has
//! already been validated by [`crate::command_admission::admit_system_commands`]
//! and placed into `AdmittedCommands`; [`repair_dispatch_system`] pins that
//! ordering explicitly with `.after(AdmissionSet)`.

use bevy::prelude::*;

use crate::command_admission::AdmissionSet;
use crate::messages::{AdmittedCommands, RepairTarget, SystemControlPayload, SystemId};
use crate::ship::system_registry::REPAIR_SYSTEM_ID;

use super::server::ShipRepairTeams;

/// The repair dispatch handler, scheduled so it can only ever observe a
/// fully-populated `AdmittedCommands`.
///
/// Runs in `SimSet::Physics`, `.after(operate_repair_ai)` (issue #830). The AI
/// operator emits its `DispatchRepairTeam` into `AdmittedCommands` in Physics;
/// admission clears `AdmittedCommands` once per tick *before* `SimSet::Input`,
/// so the applier must run after the AI emit in the same set for a same-tick
/// AI dispatch to land. Human dispatches admitted before Input survive to
/// Physics unchanged. The explicit `.after(AdmissionSet)` keeps the router's
/// dependency on the admission seam a real, observed code edge (the `use`
/// above) even though `SimSet::Physics` already runs downstream of it.
pub fn register_repair_dispatch(app: &mut App) {
    use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
    // Admitted-command consumer (issue #833): `handle_dispatch_repair_team`
    // reads the `repair` system's admitted commands.
    app.register_admitted_consumer(ConsumerMatcher::exact(REPAIR_SYSTEM_ID));
    app.add_systems(
        FixedUpdate,
        (
            handle_dispatch_repair_team,
            handle_set_repair_priority,
            handle_set_repair_target_priority,
        )
            .in_set(crate::sim_sets::SimSet::Physics)
            .after(super::server::operate_repair_ai)
            .after(AdmissionSet),
    );
}

/// Handle `DispatchRepairTeam` messages from the Repair console.
///
/// Reads `ClientMessage::ControlSystem { target: "repair", payload:
/// DispatchRepairTeam { .. } }` messages from `AdmittedCommands`. Admission
/// upstream (`admit_system_commands` / `is_command_authorized`) has already
/// checked:
///
/// 1. `ControlSourceResolver::policy_for(&repair_system_id()).accept_human_input`
///    (rejects when the system is under AI control)
/// 2. Sender holds the `repair` station.
///
/// `RepairTarget::Core` dispatches to `SystemId("core")`, the repair bucket for
/// ownerless ship-wide systems.
///
/// Per-entity (issue #830): iterates every `Ship` (player + NPC) and applies
/// each ship's own admitted `DispatchRepairTeam` commands to its own
/// `ShipRepairTeams` component. The global `ShipRepairTeams` Resource and its
/// dual-write are gone — a same-source admitted command lands on exactly the
/// ship that owns it, whether a human Engineering console or an AI operator's
/// `ai:<uuid>` emission produced it.
pub fn handle_dispatch_repair_team(
    mut ship_query: Query<
        (
            &AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
            &mut ShipRepairTeams,
            Option<&crate::entity_spawner::EntitySystemHull>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (admitted, ship_config, mut teams, hull_opt) in ship_query.iter_mut() {
        // Look up a human-readable display name for a SystemId. Prefer the
        // ship's `EntitySystemHull` entry (populated from TOML with the
        // designer-authored display name), and fall back to the raw SystemId
        // string when the hull has no entry for that id.
        let hull_ref = hull_opt.map(|h| &h.0);
        let display_name_for = |sid: &SystemId| -> String {
            if let Some(hull) = hull_ref {
                if let Some(entry) = hull.get(sid) {
                    return entry.display_name.clone();
                }
            }
            sid.0.clone()
        };

        for cmd in admitted.for_target(REPAIR_SYSTEM_ID) {
            if let SystemControlPayload::DispatchRepairTeam {
                team_idx,
                target: repair_target,
            } = &cmd.payload
            {
                // `None` ⇒ the order names no work: a station with no damaged
                // owned system whose own name is a hull row (see
                // `resolve_repair_target`). Skip the command and leave the slot
                // exactly as it was — an Idle team stays Idle, a working team is
                // not recalled — which is the same nothing-happens a dispatch to
                // an undamaged station has always produced.
                let Some(sid) = resolve_repair_target(repair_target, ship_config, hull_ref) else {
                    continue;
                };
                let display = display_name_for(&sid);
                teams.0.dispatch(*team_idx as usize, sid, display);
            }
        }
    }
}

/// Handle `SetRepairPriority` messages from the Repair console.
///
/// Reads `ClientMessage::ControlSystem { target: "repair", payload:
/// SetRepairPriority { team_idx, priority } }` messages from
/// `AdmittedCommands`. Admission upstream has already validated that the
/// sender holds the repair station. The priority only applies when the team
/// is in `Repairing` state (gated by `RepairTeams::set_priority`).
pub fn handle_set_repair_priority(
    mut ship_query: Query<(&AdmittedCommands, &mut ShipRepairTeams), With<crate::server_app::Ship>>,
) {
    for (admitted, mut teams) in ship_query.iter_mut() {
        for cmd in admitted.for_target(REPAIR_SYSTEM_ID) {
            if let SystemControlPayload::SetRepairPriority { team_idx, priority } = &cmd.payload {
                teams.0.set_priority(*team_idx as usize, *priority);
            }
        }
    }
}

/// Handle `SetRepairTargetPriority` messages from the Repair console's
/// damaged-systems list (issue #1015).
///
/// Reads `ClientMessage::ControlSystem { target: "repair", payload:
/// SetRepairTargetPriority { system_id } }` from `AdmittedCommands`. Admission
/// upstream has already checked exactly what it checks for `SetRepairPriority`
/// — same target system, therefore same station-ownership and same
/// `accept_human_input` gate; the payload variant is not part of that decision
/// (`command_admission::policy::is_command_authorized` turns on the target).
///
/// The whole point of this handler over [`handle_set_repair_priority`] is that
/// the TEAM and SYSTEM are resolved here, from the ship's own hull and
/// config, rather than trusted from the client — see the payload's doc for
/// why the console cannot compute them correctly and
/// `RepairTeams::prioritise_system` for the resolution rules. A tap naming a
/// system no on-site team can reach is a silent no-op, like a dispatch to an
/// undamaged station.
///
/// The hull and config are `Option` for the same reason they are elsewhere on
/// the repair path: a ship spawned without them has no group membership to
/// resolve a tap against, and simply ignores it.
pub fn handle_set_repair_target_priority(
    mut ship_query: Query<
        (
            &AdmittedCommands,
            &mut ShipRepairTeams,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (admitted, mut teams, hull_opt, config_opt) in ship_query.iter_mut() {
        let (Some(hull), Some(config)) = (hull_opt, config_opt) else {
            continue;
        };
        for cmd in admitted.for_target(REPAIR_SYSTEM_ID) {
            if let SystemControlPayload::SetRepairTargetPriority { system_id } = &cmd.payload {
                teams.0.prioritise_system(system_id, &hull.0, &config.0);
            }
        }
    }
}

/// Resolve a station-level repair order to a concrete hull system, or `None`
/// when the order names nothing a team could be sent to.
///
/// The repair console deliberately addresses stations, while repair teams heal
/// a single `SystemHull` entry. Prefer the most damaged repairable system the
/// ship configuration assigns to that station. The station-id fallback keeps
/// older/coarse hull layouts working until they migrate to fine system hulls.
///
/// Only the FIRST system is resolved here. Since issue #1013 an arrived team
/// sweeps the rest of the station itself (`RepairTeams::tick`), so this picks
/// where the walk starts, not the whole job.
///
/// # The fallback and the sweep gate are a complementary PAIR
///
/// The `SystemId(station_id)` fallback below is a STATION NAME, and it is
/// produced ONLY for a name the hull does not track as a row. That is the exact
/// complement of `repair_teams::sweep_from`'s gate, which runs a sweep only from
/// an arrival the hull DOES track: between them, a dispatch that fell back to a
/// station name always bounces home exactly as it did before the sweep existed,
/// and never gets bucketed as ownerless and walks off to repair `core`.
///
/// The `is_none()` guard is what makes that a guarantee rather than a
/// coincidence of the shipped hulls. A station name CAN collide with a hull
/// row: `alliance_cruiser` authors a `science` `[[hull.system_hull]]` with no
/// `[[system]]` behind it (so the row is ownerless, bucketed under `core`) AND a
/// `science` STATION whose three systems carry no hull rows of their own — so
/// that station can never produce a repairable system here and the fallback is
/// the only path out. Emitting `SystemId("science")` would pass `sweep_from`'s
/// hull-row gate and let the team sweep the ownerless bucket. Refusing to
/// dispatch is the honest answer: the station has no damaged owned system, so
/// there is no work at it to send anyone to.
///
/// Destroyed systems (0 HP) were excluded until #1013 — a team could not lift
/// the latch, so sending it to the worst system on the station would have been
/// sending it to the one system it could not touch. The sweep repairs them now,
/// and a destroyed system's damage fraction is 1.0, so it naturally sorts to the
/// front of the ranking below: the team walks to the worst thing first.
fn resolve_repair_target(
    target: &RepairTarget,
    ship_config: &crate::ship_plugin::ShipConfigComponent,
    hull: Option<&crate::damage::SystemHull>,
) -> Option<SystemId> {
    match target {
        RepairTarget::Core => Some(SystemId("core".into())),
        RepairTarget::Station(station_id) => {
            let Some(hull) = hull else {
                // No hull at all ⇒ no row can collide, and the coarse layout
                // this fallback exists for is the only thing left to name.
                return Some(SystemId(station_id.0.clone()));
            };

            let best = ship_config
                .0
                .systems_for_station(station_id)
                .filter_map(|system| {
                    let entry = hull.get(&system.id)?;
                    if entry.current >= entry.max {
                        return None;
                    }
                    let damage_fraction = 1.0 - entry.current / entry.max;
                    Some((system.id.clone(), damage_fraction))
                })
                .max_by(|left, right| left.1.total_cmp(&right.1))
                .map(|(system_id, _)| system_id);
            best.or_else(|| {
                let fallback = SystemId(station_id.0.clone());
                hull.get(&fallback).is_none().then_some(fallback)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::SystemHull;
    use crate::messages::StationId;
    use crate::ship::config::{ShipConfig, SystemInstanceConfig};

    fn system(id: &str, station: Option<&str>) -> SystemInstanceConfig {
        SystemInstanceConfig {
            id: SystemId(id.into()),
            kind: "generic".into(),
            station: station.map(|s| StationId(s.into())),
            ai_only: false,
            human_seeking: false,
            power_group: None,
            marker: None,
            config: None,
        }
    }

    fn config(systems: Vec<SystemInstanceConfig>) -> crate::ship_plugin::ShipConfigComponent {
        crate::ship_plugin::ShipConfigComponent(ShipConfig {
            stations: vec![],
            systems,
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        })
    }

    /// The `alliance_cruiser` collision: `science` is BOTH a station and an
    /// OWNERLESS hull row (a `[[hull.system_hull]]` with no `[[system]]` behind
    /// it, so it lives in the `core` sweep bucket), while the three systems the
    /// `science` station owns carry no hull rows at all.
    ///
    /// A dispatch to that station therefore finds no repairable owned system and
    /// falls through to the station-name fallback — where emitting
    /// `SystemId("science")` would hand `repair_teams::sweep_from` a real hull
    /// row, pass its gate, and let the team sweep the OWNERLESS bucket instead of
    /// bouncing. `None` is the honest answer: nothing the station owns is
    /// damaged, so there is no work to send a team to.
    #[test]
    fn station_name_colliding_with_a_hull_row_resolves_to_no_dispatch() {
        let cfg = config(vec![
            system("sensor-array", Some("science")),
            system("sensor-probe", Some("science")),
            system("sensor-lab", Some("science")),
        ]);
        // The hull tracks the ownerless `science` row and the ship-wide `core`
        // row — and NONE of the science station's own systems.
        let hull = SystemHull::from_config(&[
            (SystemId("core".into()), 100.0_f32),
            (SystemId("science".into()), 58.0),
        ]);

        assert_eq!(
            resolve_repair_target(
                &RepairTarget::Station(StationId("science".into())),
                &cfg,
                Some(&hull),
            ),
            None,
            "a station whose own name is a hull row must not fall back to it"
        );
    }

    /// The complementary half: a station name the hull does NOT track still
    /// falls back, so the pre-#1013 bounce behaviour for coarse hull layouts is
    /// untouched. `repair_teams::sweep_from` rejects exactly these arrivals.
    #[test]
    fn station_name_that_is_not_a_hull_row_still_falls_back() {
        let cfg = config(vec![system("helm-engine-port", Some("helm"))]);
        // `helm-engine-port` is at full HP, so no owned system is repairable.
        let hull = SystemHull::from_config(&[(SystemId("helm-engine-port".into()), 100.0_f32)]);

        assert_eq!(
            resolve_repair_target(
                &RepairTarget::Station(StationId("helm".into())),
                &cfg,
                Some(&hull),
            ),
            Some(SystemId("helm".into())),
            "an untracked station name is still the coarse-layout fallback"
        );
    }

    /// A damaged owned system is picked as before — the collision guard only
    /// governs the fallback arm.
    #[test]
    fn a_damaged_owned_system_still_wins_over_the_fallback() {
        let cfg = config(vec![system("science-scope", Some("science"))]);
        let mut hull = SystemHull::from_config(&[
            (SystemId("science".into()), 58.0_f32),
            (SystemId("science-scope".into()), 40.0),
        ]);
        hull.set_hp(&SystemId("science-scope".into()), 10.0);

        assert_eq!(
            resolve_repair_target(
                &RepairTarget::Station(StationId("science".into())),
                &cfg,
                Some(&hull),
            ),
            Some(SystemId("science-scope".into())),
        );
    }
}
