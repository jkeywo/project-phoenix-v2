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
        Update,
        handle_dispatch_repair_team
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
                let sid = resolve_repair_target(repair_target, ship_config, hull_ref);
                let display = display_name_for(&sid);
                teams.0.dispatch(*team_idx as usize, sid, display);
            }
        }
    }
}

/// Resolve a station-level repair order to a concrete hull system.
///
/// The repair console deliberately addresses stations, while repair teams heal
/// a single `SystemHull` entry. Prefer the most damaged repairable system the
/// ship configuration assigns to that station. The station-id fallback keeps
/// older/coarse hull layouts working until they migrate to fine system hulls.
fn resolve_repair_target(
    target: &RepairTarget,
    ship_config: &crate::ship_plugin::ShipConfigComponent,
    hull: Option<&crate::damage::SystemHull>,
) -> SystemId {
    match target {
        RepairTarget::Core => SystemId("core".into()),
        RepairTarget::Station(station_id) => {
            let Some(hull) = hull else {
                return SystemId(station_id.0.clone());
            };

            ship_config
                .0
                .systems_for_station(station_id)
                .filter_map(|system| {
                    let entry = hull.get(&system.id)?;
                    if entry.current <= 0.0 || entry.current >= entry.max {
                        return None;
                    }
                    let damage_fraction = 1.0 - entry.current / entry.max;
                    Some((system.id.clone(), damage_fraction))
                })
                .max_by(|left, right| left.1.total_cmp(&right.1))
                .map(|(system_id, _)| system_id)
                .unwrap_or_else(|| SystemId(station_id.0.clone()))
        }
    }
}
