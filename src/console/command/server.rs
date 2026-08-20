//! Bevy wiring for the Command console (issue #1107).

use bevy::prelude::*;
use std::collections::HashMap;

use crate::messages::{
    AdmittedCommands, CommandBlackboard, CommandStanceOption, StationId, SystemBlackboard,
    SystemControlPayload, SystemId,
};
use crate::ship::command_stance;
use crate::ship::config::{ShipConfig, StanceKind, StationConfig};
use crate::ship::control_source::{ControlSource, ControlSourceResolver};
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};

/// One ship's current Command stance selections, keyed by directed Station id
/// (issue #1107).
///
/// EMPTY is the load-bearing default: a Station with no entry is undirected and
/// its AI hosts track the ship's own Red Alert exactly as before this issue, so
/// a hull nobody commands stays byte-identical. A human Command operator's
/// explicit pick — and only that — lands an entry here.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ShipStationStances(pub HashMap<StationId, String>);

pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::system_registry::COMMAND_SYSTEM_ID,
        ));
        crate::ai::cadence::register_ai_cadence(app);
        app.add_systems(
            FixedUpdate,
            (
                // A human Command operator's stance pick lands first…
                handle_set_station_stance.in_set(crate::sim_sets::SimSet::Input),
                // …then the alert-level neutral-to-neutral switch runs, so a
                // stored neutral follows an alert change the same tick the
                // captain raises it (issue #1107 criterion 5).
                apply_alert_change_to_stances
                    .in_set(crate::sim_sets::SimSet::Input)
                    .after(handle_set_station_stance),
                // The AI Command operator: resets a human's non-persistent order
                // to neutral when no human holds Command. Gated on the shared AI
                // cadence, like the Captain AI.
                operate_command_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .after(apply_alert_change_to_stances)
                    .run_if(crate::ai::cadence::ai_snapshot_ready),
                publish_command_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

// ── Config helpers (pure) ─────────────────────────────────────────────────────

/// The one authored Command station on a hull, if any: an auxiliary station that
/// names a `command_target`.
pub fn command_station(config: &ShipConfig) -> Option<&StationConfig> {
    config
        .stations
        .iter()
        .find(|station| station.command_target.is_some())
}

/// Whether a Station is currently AI-controlled — the only state in which
/// Command lists and applies stances (criterion 2).
///
/// A Station is AI-controlled when every one of its owned fine Systems (the
/// `human_seeking` visitors excluded — they are other Stations' surfaces) reads
/// `ControlSource::Ai`. A Station that owns no such System is treated as
/// AI-controlled: nobody is operating a fine System there.
pub fn station_is_ai_controlled(
    config: &ShipConfig,
    control_sources: &ControlSourceResolver,
    station: &StationId,
) -> bool {
    config
        .systems
        .iter()
        .filter(|system| system.station.as_ref() == Some(station) && !system.human_seeking)
        .all(|system| control_sources.source_for(&system.id) == ControlSource::Ai)
}

/// The high-alert posture a directed weapons Station's Command stance seeds for
/// the weapons AI hosts (issue #1107) — the seam that carries the migrated Red
/// Alert branch onto the neutral-stance path.
///
/// `None` means "no Command stance in force" and the caller falls back to the
/// ship's own Red Alert, byte-identical to the pre-#1107 fire gate. `Some(high)`
/// forces the posture the operator selected. Only returns `Some` when the
/// weapons Station is AI-controlled AND carries an explicit stored selection.
pub fn weapons_station_stance_high_alert(
    stances: Option<&ShipStationStances>,
    config: &ShipConfig,
    control_sources: &ControlSourceResolver,
    red_alert: bool,
) -> Option<bool> {
    let stances = stances?;
    let weapons_station = config.weapons_station()?;
    if !station_is_ai_controlled(config, control_sources, &weapons_station) {
        return None;
    }
    let selected = stances.0.get(&weapons_station)?;
    let catalogue = &config.station(&weapons_station)?.stances;
    Some(command_stance::effective_high_alert(
        catalogue,
        Some(selected.as_str()),
        red_alert,
    ))
}

// ── Input handlers ────────────────────────────────────────────────────────────

/// Apply admitted `SetStationStance` orders (issue #1107).
///
/// Admission has already authorised the sender against the Command station's
/// live host (Captain, normally) via `station_for_system`. Here the order is
/// validated against CONTENT: this hull's Command station must direct the named
/// Station, that Station must be AI-controlled right now, and the stance id must
/// be one the target authored (Command "does not invent orders outside the
/// authored vocabulary"). A rejected order is a silent no-op.
fn handle_set_station_stance(
    mut ships: Query<
        (
            &AdmittedCommands,
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &mut ShipStationStances,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (admitted, ship_config, control_sources, mut stances) in ships.iter_mut() {
        let config = &ship_config.0;
        let Some(command) = command_station(config) else {
            continue;
        };
        let target = command.command_target.as_ref();
        for cmd in admitted.for_target(crate::system_registry::COMMAND_SYSTEM_ID) {
            let SystemControlPayload::SetStationStance { station, stance } = &cmd.payload else {
                continue;
            };
            // Command directs exactly the authored target Station.
            if target != Some(station) {
                continue;
            }
            // Only while that Station is AI-controlled.
            if !station_is_ai_controlled(config, &control_sources.0, station) {
                continue;
            }
            // Only an authored stance.
            let Some(target_station) = config.station(station) else {
                continue;
            };
            if !command_stance::is_selectable(&target_station.stances, stance) {
                continue;
            }
            stances.0.insert(station.clone(), stance.clone());
        }
    }
}

/// Switch a stored neutral selection to the other neutral when the ship's alert
/// level changes, never overwriting a standard stance (issue #1107 criterion 5).
///
/// `Changed<ShipRedAlert>` fires on insertion too, so the sync is applied once
/// at spawn and then only on a real alert transition. Only STORED entries are
/// touched: an undirected Station has none, and it keeps tracking the alert
/// through the absent-selection default rather than through a stored value.
fn apply_alert_change_to_stances(
    mut ships: Query<
        (
            &ShipConfigComponent,
            &crate::ship_state::ShipRedAlert,
            &mut ShipStationStances,
        ),
        (
            With<crate::server_app::Ship>,
            Changed<crate::ship_state::ShipRedAlert>,
        ),
    >,
) {
    for (ship_config, red_alert, mut stances) in ships.iter_mut() {
        let config = &ship_config.0;
        if stances.0.is_empty() {
            continue;
        }
        let mut updates: Vec<(StationId, String)> = Vec::new();
        for (station, current) in stances.0.iter() {
            let Some(target) = config.station(station) else {
                continue;
            };
            if let Some(next) = command_stance::selection_after_alert_change(
                &target.stances,
                Some(current.as_str()),
                red_alert.0,
            ) {
                if &next != current {
                    updates.push((station.clone(), next));
                }
            }
        }
        for (station, next) in updates {
            stances.0.insert(station, next);
        }
    }
}

/// The AI Command operator (issue #1107).
///
/// When no human holds Command — its coarse system reads `ControlSource::Ai` —
/// a human's non-persistent standard order is reset to the alert-appropriate
/// neutral so an old aggressive order does not resume behind the handoff, while
/// a `persist_behind_human` order and the two neutrals are kept. AI Command uses
/// the SAME authored catalogue and the SAME stored-selection path a human does;
/// it never writes an order the human vocabulary does not contain.
fn operate_command_ai(
    mut ships: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &crate::ship_state::ShipRedAlert,
            &mut ShipStationStances,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (ship_config, control_sources, red_alert, mut stances) in ships.iter_mut() {
        let config = &ship_config.0;
        let Some(command) = command_station(config) else {
            continue;
        };
        // Only when Command itself is AI-operated.
        if control_sources
            .0
            .source_for(&crate::system_registry::command_system_id())
            != ControlSource::Ai
        {
            continue;
        }
        let Some(target) = command.command_target.clone() else {
            continue;
        };
        let Some(target_station) = config.station(&target) else {
            continue;
        };
        let current = stances.0.get(&target).cloned();
        let resolved = command_stance::selection_after_human_lost(
            &target_station.stances,
            current.as_deref(),
            red_alert.0,
        );
        match resolved {
            // A neutral outcome is the tracking default — clear the stored entry
            // so the AI-controlled Station stays byte-identical to a hull that
            // was never commanded.
            Some(next)
                if command_stance::stance_by_id(&target_station.stances, &next).is_some_and(
                    |s| {
                        matches!(
                            s.kind,
                            StanceKind::NormalAlertNeutral | StanceKind::HighAlertNeutral
                        )
                    },
                ) =>
            {
                // `remove` is a no-op when nothing is stored, so the entry ends
                // up absent either way — byte-identical to a never-commanded hull.
                stances.0.remove(&target);
            }
            Some(next) => {
                // Re-inserting an equal value leaves the map unchanged, so an
                // unconditional insert is equivalent to the guarded one.
                stances.0.insert(target.clone(), next);
            }
            None => {}
        }
    }
}

// ── Blackboard publish ─────────────────────────────────────────────────────────

/// Publish the Command console readout for every ship carrying a Command station
/// (issue #1107).
fn publish_command_blackboard(
    mut ships: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &crate::ship_state::ShipRedAlert,
            Option<&ShipStationStances>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (ship_config, control_sources, red_alert, stances, mut bbs) in ships.iter_mut() {
        let config = &ship_config.0;
        let Some(command) = command_station(config) else {
            continue;
        };
        let Some(target) = command.command_target.clone() else {
            continue;
        };
        let Some(target_station) = config.station(&target) else {
            continue;
        };

        let directed_station_ai = station_is_ai_controlled(config, &control_sources.0, &target);
        let command_auto = control_sources
            .0
            .source_for(&crate::system_registry::command_system_id())
            == ControlSource::Ai;

        // The stance in force: an explicit stored selection, else the alert
        // level's neutral (the tracking default the AI hosts see).
        let selected_stance = stances
            .and_then(|s| s.0.get(&target).cloned())
            .or_else(|| {
                command_stance::neutral_stance_for_alert(&target_station.stances, red_alert.0)
                    .map(str::to_string)
            })
            .unwrap_or_default();

        let options: Vec<CommandStanceOption> = target_station
            .stances
            .iter()
            .map(|stance| CommandStanceOption {
                id: stance.id.clone(),
                label: stance.label.clone(),
                kind: stance_kind_wire(stance.kind).to_string(),
                high_alert: stance.high_alert,
            })
            .collect();

        let bb = CommandBlackboard {
            command_system_id: crate::system_registry::command_system_id(),
            directed_station: target.clone(),
            directed_station_name: target_station.name.clone(),
            directed_station_ai,
            command_auto,
            selected_stance,
            stances: options,
        };
        bbs.0.insert(
            SystemId(crate::system_registry::COMMAND_SYSTEM_ID.to_string()),
            SystemBlackboard::Command(bb),
        );
    }
}

/// The wire string for a stance kind — the snake_case the TOML authors and the
/// console groups on.
fn stance_kind_wire(kind: StanceKind) -> &'static str {
    match kind {
        StanceKind::Standard => "standard",
        StanceKind::NormalAlertNeutral => "normal_alert_neutral",
        StanceKind::HighAlertNeutral => "high_alert_neutral",
    }
}

#[cfg(test)]
mod tests;
