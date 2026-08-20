//! Bevy wiring for the Command console (issue #1107).

use bevy::prelude::*;
use std::collections::HashMap;

use crate::messages::{
    AdmittedCommands, CommandBlackboard, CommandStanceOption, StationId, SystemBlackboard,
    SystemControlPayload, SystemId,
};
use crate::ship::command_stance;
use crate::ship::config::{ShipConfig, StanceKind, StationConfig, StationStanceConfig};
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

/// Per-ship edge-detection memory for the persist-behind-human trigger
/// (issue #1108): the last-observed AI/human control state of each Command
/// directed Station (`true` == that Station was AI-controlled last tick).
///
/// # Why this is transient scratch and is NOT folded into the sim digest
///
/// It records a value that is itself a pure function of already-authoritative
/// state — `station_is_ai_controlled`, derived each tick from the ship's
/// control sources (which the digest already excludes as `derived`). The
/// AUTHORITATIVE outcome of the Human→AI edge — resuming a persistent stance or
/// clearing a transient one — lands in [`ShipStationStances`], which IS folded.
/// This map only remembers *when the last transition was* so the decision fires
/// once, on the edge, rather than every tick. A host that never saw the prior
/// state (a fresh spawn) records the current state as its FIRST observation and
/// fires no edge — the deliberate first-observation no-op below.
///
/// The exclusion is safe because the digest is only ever compared in full
/// replay from tick 0 (headless, within one process), never across a snapshot
/// boundary: `snapshot.rs` persists NEITHER this scratch NOR
/// [`ShipStationStances`] today, so every host reconstructs both by replaying
/// the same ticks over the same control sources. WARNING: if a future change
/// starts persisting [`ShipStationStances`] in a snapshot (so a restored host
/// keeps stored stances), it MUST also persist or reseed this scratch —
/// otherwise a Human→AI transition on the first post-restore tick would fire on
/// a continuous host (`was_ai == Some(false)`) but be swallowed as a first
/// observation (`was_ai == None`) on the restored host, diverging the resolved
/// stance.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct LastDirectedControl(pub HashMap<StationId, bool>);

pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::system_registry::COMMAND_SYSTEM_ID,
        ));
        // The ONE shared AI decision cadence (issues #889, #895): re-armed here
        // because an uncrewed Command seat now decides through ordinary AI
        // (issue #1109). Idempotent — every plugin registering a gated system
        // calls it.
        crate::ai::cadence::register_ai_cadence(app);
        app.add_systems(
            FixedUpdate,
            (
                // A stored id no longer in the authored catalogue is dropped
                // FIRST, so the input handlers below never act on a stale
                // selection (issue #1108 criterion 4).
                reconcile_station_stances.in_set(crate::sim_sets::SimSet::Input),
                // An uncrewed Command seat chooses a stance through ordinary AI
                // (issue #1109). It runs AFTER the catalogue reconcile (so it
                // never picks a stale id) and BEFORE the applier (so its emitted
                // order lands the same tick). It emits an admitted order rather
                // than writing ShipStationStances itself, so #1108's Human→AI
                // reconcile (below, after the applier) still has the final word
                // on a handoff tick. Cadence-gated like every other AI operator.
                operate_command_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .after(reconcile_station_stances)
                    .before(handle_set_station_stance)
                    .run_if(crate::ai::cadence::ai_snapshot_ready),
                // A human Command operator's stance pick lands next…
                handle_set_station_stance
                    .in_set(crate::sim_sets::SimSet::Input)
                    .after(reconcile_station_stances),
                // …then the alert-level neutral-to-neutral switch runs, so a
                // stored neutral follows an alert change the same tick the
                // captain raises it (issue #1107 criterion 5).
                apply_alert_change_to_stances
                    .in_set(crate::sim_sets::SimSet::Input)
                    .after(handle_set_station_stance),
                // The persist-behind-human trigger (issue #1108): when the
                // directed target Station transitions Human→AI, a persistent
                // stance resumes and a transient one falls back to the current
                // alert-neutral. Runs EVERY tick (not cadence-gated) so no
                // control-source edge is missed.
                reconcile_directed_target_control
                    .in_set(crate::sim_sets::SimSet::Input)
                    .after(apply_alert_change_to_stances),
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

/// Drop any stored selection whose stance has left the directed Station's
/// authored catalogue (issue #1108 criterion 4).
///
/// A stance id no longer authored — a hull change, or (forward-looking, #1110)
/// an objective stance whose objective ended — is removed here, so the Station
/// falls back to the alert-neutral tracking default and the removal is visible
/// on the next blackboard publish. Membership is decided through the one
/// `command_stance` catalogue seam, never an ad-hoc check. A station id that no
/// longer names any station at all is dropped too.
fn reconcile_station_stances(
    mut ships: Query<
        (&ShipConfigComponent, &mut ShipStationStances),
        With<crate::server_app::Ship>,
    >,
) {
    for (ship_config, mut stances) in ships.iter_mut() {
        // Read-only probe first: taking `&mut` does not mark the component
        // changed until it is deref-mutated, so an already-clean map (the
        // overwhelming common case) triggers no spurious change detection.
        if stances.0.is_empty() {
            continue;
        }
        let config = &ship_config.0;
        let stale: Vec<StationId> = stances
            .0
            .iter()
            .filter(|(station, stance)| match config.station(station) {
                Some(target) => {
                    command_stance::reconcile_selection(&target.stances, stance).is_none()
                }
                None => true,
            })
            .map(|(station, _)| station.clone())
            .collect();
        for station in stale {
            stances.0.remove(&station);
        }
    }
}

/// The persist-behind-human trigger (issue #1108).
///
/// Carries an authored Command stance across a human's control of the DIRECTED
/// Station without constraining that human. While a human holds the target,
/// `station_is_ai_controlled` is false: the stored order is dormant and the
/// human sees it only as advice (`publish_command_blackboard` +
/// `withCommandAdvice`), keeping full ordinary authority. The instant the
/// target returns to AI — the Human→AI edge — `selection_after_human_lost`
/// decides per the stance's authored `persist_behind_human`: a persistent
/// standard order RESUMES its stored id; a transient one (and the neutrals)
/// falls back to the current alert-neutral, which clears the entry.
///
/// Keys on the TARGET Station's control state, not the Command seat's: this is
/// about the person crewing the directed Station, and it is the same whether a
/// human or the ship's AI currently holds Command. Edge detection uses
/// [`LastDirectedControl`]; a first observation records state and fires nothing.
fn reconcile_directed_target_control(
    mut ships: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &crate::ship_state::ShipRedAlert,
            &mut ShipStationStances,
            &mut LastDirectedControl,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (ship_config, control_sources, red_alert, mut stances, mut last) in ships.iter_mut() {
        let config = &ship_config.0;
        let Some(command) = command_station(config) else {
            continue;
        };
        let Some(target) = command.command_target.clone() else {
            continue;
        };
        let now_ai = station_is_ai_controlled(config, &control_sources.0, &target);
        let was_ai = last.0.get(&target).copied();

        // Human→AI edge only: a first observation (`None`) or any other
        // transition just records state below.
        if was_ai == Some(false) && now_ai {
            if let Some(target_station) = config.station(&target) {
                let current = stances.0.get(&target).cloned();
                let resolved = command_stance::selection_after_human_lost(
                    &target_station.stances,
                    current.as_deref(),
                    red_alert.0,
                );
                store_resolved_selection(&mut stances, &target, &target_station.stances, resolved);
            }
        }

        // Record the current observation for next tick's edge detection. Guard
        // the write so an unchanged state marks nothing dirty.
        if was_ai != Some(now_ai) {
            last.0.insert(target.clone(), now_ai);
        }
    }
}

/// Apply a `selection_after_*` outcome to the stored map for `target`.
///
/// A neutral outcome is the tracking default, so the entry is CLEARED — an
/// absent entry is byte-identical to a never-commanded hull, and folds to the
/// same digest. Any other (standard) stance is stored. `None` leaves the map
/// untouched.
fn store_resolved_selection(
    stances: &mut ShipStationStances,
    target: &StationId,
    catalogue: &[StationStanceConfig],
    resolved: Option<String>,
) {
    match resolved {
        Some(next)
            if command_stance::stance_by_id(catalogue, &next).is_some_and(|s| {
                matches!(
                    s.kind,
                    StanceKind::NormalAlertNeutral | StanceKind::HighAlertNeutral
                )
            }) =>
        {
            stances.0.remove(target);
        }
        Some(next) => {
            stances.0.insert(target.clone(), next);
        }
        None => {}
    }
}

// ── AI Command operator (issue #1109) ──────────────────────────────────────────

/// Operate an uncrewed Command seat through ordinary AI (issue #1109).
///
/// When no human hosts Command — its coarse system reads `operate_ai` — the
/// ship's AI directs the target Station from EXACTLY the authored stance
/// catalogue a human uses, and applies its choice through the SAME authoritative
/// path: it emits an admitted `SetStationStance` (via
/// [`emit_command_ai_command`]) targeting `command_system_id()`, so admission
/// validates the `ai:` token against the Command system's `operate_ai` policy
/// and the shared [`handle_set_station_stance`] applier lands it through its
/// three content gates. This is why AC2/AC3 come for free: the AI can neither
/// invent a stance nor bypass admission — it goes through the same seam a human
/// order does. It never writes [`ShipStationStances`] directly.
///
/// The choice is [`command_stance::select_stance`]: a pure, tick-derived
/// function of the ship's own Red Alert and the authored catalogue, with the
/// high-alert posture named by the catalogue's `ai_engaged` flag (never a
/// hard-coded id). Emission is guarded on change against the stance currently in
/// force to avoid admission spam; `SetStationStance` is idempotent so
/// correctness does not depend on the guard.
fn operate_command_ai(
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &crate::ship_state::ShipRedAlert,
            &ShipStationStances,
            &mut AdmittedCommands,
            Option<&crate::entity_spawner::EntityUuid>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (ship_config, control_sources, red_alert, stances, mut admitted, entity_uuid) in
        ships.iter_mut()
    {
        let config = &ship_config.0;
        let Some(command) = command_station(config) else {
            continue;
        };
        // Only when the Command SEAT itself is AI-operated. This is the
        // uncrewed-Command gate (a) — distinct from the DIRECTED station's
        // control, which the applier checks as gate (b).
        if !control_sources
            .0
            .policy_for(&crate::system_registry::command_system_id())
            .operate_ai
        {
            continue;
        }
        let Some(target) = command.command_target.clone() else {
            continue;
        };
        // Nothing to direct while a human holds the target Station — the applier
        // would reject the order anyway; skipping here avoids admission spam.
        if !station_is_ai_controlled(config, &control_sources.0, &target) {
            continue;
        }
        let Some(target_station) = config.station(&target) else {
            continue;
        };
        let Some(chosen) = command_stance::select_stance(
            &target_station.stances,
            command_stance::CommandKnowledge {
                red_alert: red_alert.0,
            },
        ) else {
            continue;
        };
        // The stance currently in force: an explicit stored selection, else the
        // alert level's neutral (the same tracking default the console shows).
        let in_force = stances.0.get(&target).cloned().or_else(|| {
            command_stance::neutral_stance_for_alert(&target_station.stances, red_alert.0)
                .map(str::to_string)
        });
        if in_force.as_deref() == Some(chosen.as_str()) {
            continue;
        }
        emit_command_ai_command(
            entity_uuid,
            SystemControlPayload::SetStationStance {
                station: target.clone(),
                stance: chosen,
            },
            control_sources,
            &sessions,
            Some(ship_config),
            &mut admitted,
        );
    }
}

/// Emit an admitted Command AI order targeting the Command system through the
/// shared [`crate::command_admission::ai_emit::emit_ai_command`] seam, using this
/// ship's own `ai:<uuid>` token (mirrors `emit_captain_ai_command`).
fn emit_command_ai_command(
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    payload: SystemControlPayload,
    sources: &ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    ship_config: Option<&ShipConfigComponent>,
    admitted: &mut AdmittedCommands,
) -> bool {
    crate::command_admission::ai_emit::emit_ai_command(
        entity_uuid,
        crate::system_registry::command_system_id(),
        payload,
        sources,
        sessions,
        ship_config,
        admitted,
    )
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
