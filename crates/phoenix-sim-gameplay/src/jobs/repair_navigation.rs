//! Physical adaptation of the retained repair phase/work executor.
use crate::core::messages::TeamSlot;
use crate::modifiers::repair_teams::RepairPhases;
use crate::ship::interior::{InteriorGraph, TeamLocation, TeamNavigation};
use std::collections::BTreeSet;

pub(super) fn prepare(
    graph: &InteriorGraph,
    teams: &mut [TeamNavigation],
    phases: &mut RepairPhases,
    dt: f32,
    enabled: &mut [bool],
    blocked: &BTreeSet<String>,
) {
    let mut slots = phases.slots().to_vec();
    guard_slots(graph, teams, &mut slots);
    for (index, (slot, team)) in slots.iter_mut().zip(teams).enumerate() {
        if !enabled.get(index).copied().unwrap_or(false) {
            // Pause at a safe endpoint, never inside a committed crossing where
            // the access interlock would otherwise wait indefinitely.
            if let TeamLocation::Crossing { to, .. } = team.location.clone() {
                graph.advance(team, &to, dt, blocked);
            }
            continue;
        }
        match slot {
            TeamSlot::Travelling {
                system_id, elapsed, ..
            } => {
                let destination = system_id
                    .as_ref()
                    .and_then(|id| graph.compartment_for_system(id));
                let arrived =
                    destination.is_some_and(|room| graph.advance(team, room, dt, blocked));
                if arrived {
                    *elapsed = phases.timings().travel_duration;
                } else {
                    enabled[index] = false;
                    *elapsed = 0.0;
                }
            }
            TeamSlot::Returning { remaining, .. } => {
                if graph.advance(team, &graph.config.team_homes[index], dt, blocked) {
                    *remaining = 0.0;
                } else {
                    enabled[index] = false;
                    *remaining = phases.timings().travel_duration;
                }
            }
            _ => {}
        }
    }
    phases.restore_slots(&slots);
}

/// A sweep can choose a different System without moving its team. Convert that
/// choice to travel before publication, work grants or on-site knowledge.
pub(super) fn guard_arrival(
    graph: &InteriorGraph,
    teams: &[TeamNavigation],
    phases: &mut RepairPhases,
) {
    let mut slots = phases.slots().to_vec();
    guard_slots(graph, teams, &mut slots);
    phases.restore_slots(&slots);
}
fn guard_slots(graph: &InteriorGraph, teams: &[TeamNavigation], slots: &mut [TeamSlot]) {
    for (slot, team) in slots.iter_mut().zip(teams) {
        if let TeamSlot::Repairing {
            system_id: Some(id),
            display_name,
            priority,
            ..
        } = slot
        {
            let room = graph.compartment_for_system(id);
            let arrived = matches!(&team.location,TeamLocation::Compartment {compartment} if Some(compartment)==room);
            if !arrived {
                *slot = TeamSlot::Travelling {
                    system_id: Some(id.clone()),
                    display_name: display_name.clone(),
                    elapsed: 0.0,
                    priority: *priority,
                };
            }
        }
    }
}
