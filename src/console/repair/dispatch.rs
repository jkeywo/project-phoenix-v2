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
/// `SimSet::Input` already runs after [`AdmissionSet`]; the explicit
/// `.after(AdmissionSet)` states the dependency at the point of use so it
/// survives any future re-ordering of the sim sets.
pub fn register_repair_dispatch(app: &mut App) {
    app.add_systems(
        Update,
        handle_dispatch_repair_team
            .in_set(crate::sim_sets::SimSet::Input)
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
/// After PR 6 (PRD #597): prefers the per-entity `ShipRepairTeams` component
/// on the LocalShip entity; falls back to the global `ShipRepairTeams` resource
/// for tests. Dual-writes to the Resource so legacy Resource-based readers
/// stay in sync.
pub fn handle_dispatch_repair_team(
    mut ship_query: Query<
        (
            &AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
            Option<&mut ShipRepairTeams>,
            Option<&crate::entity_spawner::EntitySystemHull>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    teams_res: Option<ResMut<ShipRepairTeams>>,
) {
    let Some((admitted, ship_config, mut teams_comp, hull_opt)) = ship_query.iter_mut().next()
    else {
        return;
    };
    let mut teams_res = teams_res;

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

    // Collect all dispatches into a batch first, then apply once — avoids the
    // closure-captures-borrow tangle when routing between Component and Resource.
    let mut pending: Vec<(usize, SystemId, String)> = Vec::new();

    for cmd in admitted.for_target(REPAIR_SYSTEM_ID) {
        if let SystemControlPayload::DispatchRepairTeam {
            team_idx,
            target: repair_target,
        } = &cmd.payload
        {
            let sid = resolve_repair_target(repair_target, ship_config, hull_ref);
            let display = display_name_for(&sid);
            pending.push((*team_idx as usize, sid, display));
        }
    }

    if pending.is_empty() {
        return;
    }

    // Apply to whichever backing store is available; dual-write when both.
    for (idx, sid, display) in pending {
        if let Some(t) = teams_comp.as_deref_mut() {
            t.0.dispatch(idx, sid.clone(), display.clone());
        }
        if let Some(r) = teams_res.as_deref_mut() {
            r.0.dispatch(idx, sid, display);
        }
    }
    // Keep Resource in sync with per-entity component (Resource is dual-written
    // above; but if only the Component was updated we snapshot the Component
    // into the Resource so legacy Resource-based readers see the latest state).
    if let (Some(t), Some(r)) = (teams_comp.as_deref(), teams_res.as_deref_mut()) {
        r.0 = t.0.clone();
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
