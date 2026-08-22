//! Bevy wiring for the Command console (issue #1107).

use bevy::prelude::*;
use std::collections::HashMap;

use crate::core::messages::{
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
/// The exclusion stays safe across a snapshot boundary because
/// [`ShipStationStances`] — the AUTHORITATIVE half that IS folded — now travels
/// in the payload (`snapshot.rs`, SNAPSHOT_FORMAT 11), and `restore` RESEEDS
/// this scratch from the restored world's own control sources: for the directed
/// Station it records the current `station_is_ai_controlled` reading, so the
/// first post-restore tick is a continuation of the state the resume landed in
/// rather than a first observation (`was_ai == None`) that would swallow a
/// Human→AI edge a continuous host (`was_ai == Some(false)`) fires that tick.
/// The scratch itself is deliberately NOT persisted: the captured session-level
/// human/AI split on the target is not recoverable (control sources are derived
/// from who is at a console, which the snapshot excludes), so the honest reseed
/// is the restored world's current reading, not a stored one.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct LastDirectedControl(pub HashMap<StationId, bool>);

/// The Command stances currently contributed by `Active` scenario objectives
/// (issue #1110), each paired with the target Station it is lent to.
///
/// A thin, tick-refreshed PROJECTION of the authoritative
/// [`ObjectiveManagerRes`](crate::world::server::ObjectiveManagerRes): every
/// Command consumer merges these into a directed Station's EFFECTIVE catalogue at
/// read time via [`command_stance::effective_catalogue`], so an objective's
/// stance is exposed and selectable while the objective is `Active` WITHOUT ever
/// mutating the Station's permanent catalogue (AC1). Empty — the default, and
/// what a bare-`App` fixture or an objective-free world carries — leaves every
/// catalogue byte-identical to its permanent form.
///
/// Rebuilt each tick from the objective manager by
/// [`project_active_objective_stances`] in `SimSet::Input`, BEFORE
/// [`reconcile_station_stances`], so an objective that ended last tick has its
/// contribution gone before the removal reconcile runs (AC3/AC4). Not folded into
/// the sim digest: it is a pure function of the already-authoritative objective
/// state.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct ActiveObjectiveStances(pub Vec<(StationId, StationStanceConfig)>);

/// The objective-contributed stances lent to one target Station this tick.
///
/// The per-consumer helper behind the EFFECTIVE catalogue: it filters the
/// [`ActiveObjectiveStances`] projection down to the entries whose target is
/// `station` (AC2's "the contribution's station equals the directed target"
/// gate). Absent projection or no match → empty, so the effective catalogue
/// collapses to the permanent one.
fn contributed_for(
    active: Option<&ActiveObjectiveStances>,
    station: &StationId,
) -> Vec<StationStanceConfig> {
    active
        .map(|a| {
            a.0.iter()
                .filter(|(target, _)| target == station)
                .map(|(_, stance)| stance.clone())
                .collect()
        })
        .unwrap_or_default()
}

pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::COMMAND_SYSTEM_ID,
        ));
        // The ONE shared AI decision cadence (issues #889, #895): re-armed here
        // because an uncrewed Command seat now decides through ordinary AI
        // (issue #1109). Idempotent — every plugin registering a gated system
        // calls it.
        crate::ai::cadence::register_ai_cadence(app);
        app.init_resource::<ActiveObjectiveStances>();
        // Authoritative-state exclusion declarations (issue #1221, Track 3 step
        // C9). Both are DERIVED scratch around the folded `ShipStationStances`:
        // `LastDirectedControl` remembers each directed Station's last-observed
        // control so the Human->AI handoff fires once on the edge, and
        // `ActiveObjectiveStances` is the per-tick projection of the active
        // objective-contributed stances rebuilt from the (folded) objective
        // manager. Neither is folded; declared here at their owning site,
        // replacing the `EXCLUSIONS` const in
        // `tests/authoritative_state_enumeration.rs`. Inert to the digest.
        {
            use crate::authoritative::{DeclareState, StateClass};
            app.declare_state::<LastDirectedControl>(
                StateClass::Derived,
                "command-station-authority",
            )
            .declare_state::<ActiveObjectiveStances>(
                StateClass::Derived,
                "command-stance-selection-state",
            );
        }
        app.add_systems(
            FixedUpdate,
            (
                // Refresh the objective-contribution projection FIRST, from the
                // objective manager the world dispatch pass updated last tick, so
                // every consumer below (and the removal reconcile in particular)
                // reads this tick's active contributions (issue #1110).
                project_active_objective_stances
                    .in_set(crate::sim_sets::SimSet::Input)
                    .before(reconcile_station_stances),
                // A stored id no longer in the effective catalogue — a hull
                // change, or an objective-contributed stance whose objective just
                // ended (#1110) — is dropped FIRST, so the input handlers below
                // never act on a stale selection (issue #1108 criterion 4).
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

// ── Objective-contribution projection (issue #1110) ────────────────────────────

/// Refresh the [`ActiveObjectiveStances`] projection from the authoritative
/// objective manager each tick (issue #1110).
///
/// Runs in `SimSet::Input` before every Command consumer, so activation,
/// completion, failure and invalidation of an objective all reach the effective
/// catalogue on the tick after the world dispatch pass records them — the same
/// one-tick, frozen-snapshot cadence the rest of the sim reads cross-system state
/// through. `Option<Res>` so a bare-`App` fixture with no world plugin simply
/// projects nothing (the empty default). Guarded on change so an unchanged
/// objective set marks the resource clean and triggers no downstream churn.
fn project_active_objective_stances(
    manager: Option<Res<crate::world::server::ObjectiveManagerRes>>,
    mut projection: ResMut<ActiveObjectiveStances>,
) {
    let next = manager
        .map(|m| m.0.active_station_stances())
        .unwrap_or_default();
    if projection.0 != next {
        projection.0 = next;
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
    active: Option<&ActiveObjectiveStances>,
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
    // Resolve the selection through the EFFECTIVE catalogue (issue #1110): a
    // selected objective-contributed stance seeds its authored `high_alert`
    // posture just as a permanent one does, and vanishes with its objective.
    let permanent = &config.station(&weapons_station)?.stances;
    let contributed = contributed_for(active, &weapons_station);
    let catalogue = command_stance::effective_catalogue(permanent, &contributed);
    Some(command_stance::effective_high_alert(
        &catalogue,
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
    active: Option<Res<ActiveObjectiveStances>>,
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
    let active = active.as_deref();
    for (admitted, ship_config, control_sources, mut stances) in ships.iter_mut() {
        let config = &ship_config.0;
        let Some(command) = command_station(config) else {
            continue;
        };
        let target = command.command_target.as_ref();
        for cmd in admitted.for_target(crate::ship::system_registry::COMMAND_SYSTEM_ID) {
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
            // Only a stance the EFFECTIVE catalogue authors — the permanent
            // catalogue plus any active objective contribution to this target
            // (issue #1110), the same vocabulary the console lists.
            let Some(target_station) = config.station(station) else {
                continue;
            };
            let contributed = contributed_for(active, station);
            let catalogue =
                command_stance::effective_catalogue(&target_station.stances, &contributed);
            if !command_stance::is_selectable(&catalogue, stance) {
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
    active: Option<Res<ActiveObjectiveStances>>,
    mut ships: Query<
        (
            &ShipConfigComponent,
            &crate::ship::state::ShipRedAlert,
            &mut ShipStationStances,
        ),
        (
            With<crate::server_app::Ship>,
            Changed<crate::ship::state::ShipRedAlert>,
        ),
    >,
) {
    let active = active.as_deref();
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
            let contributed = contributed_for(active, station);
            let catalogue = command_stance::effective_catalogue(&target.stances, &contributed);
            if let Some(next) = command_stance::selection_after_alert_change(
                &catalogue,
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
    active: Option<Res<ActiveObjectiveStances>>,
    mut ships: Query<
        (&ShipConfigComponent, &mut ShipStationStances),
        With<crate::server_app::Ship>,
    >,
) {
    let active = active.as_deref();
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
                // Membership is judged against the EFFECTIVE catalogue (issue
                // #1110): while an objective contributes a stance to this target
                // the id is a member and survives; once the objective ends the
                // contribution is gone from the projection, so a selected
                // objective stance drops here and the Station falls back to the
                // alert-neutral tracking default (AC3/AC4).
                Some(target) => {
                    let contributed = contributed_for(active, station);
                    let catalogue =
                        command_stance::effective_catalogue(&target.stances, &contributed);
                    command_stance::reconcile_selection(&catalogue, stance).is_none()
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
    active: Option<Res<ActiveObjectiveStances>>,
    mut ships: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &crate::ship::state::ShipRedAlert,
            &mut ShipStationStances,
            &mut LastDirectedControl,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let active = active.as_deref();
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
                // Resolve the handoff against the EFFECTIVE catalogue (issue
                // #1110) so a persist-behind-human OBJECTIVE stance is carried
                // across the human hold exactly as a permanent one is — and, once
                // its objective ends, is already gone from the catalogue so the
                // handoff resolves to the alert-neutral instead.
                let contributed = contributed_for(active, &target);
                let catalogue =
                    command_stance::effective_catalogue(&target_station.stances, &contributed);
                let current = stances.0.get(&target).cloned();
                let resolved = command_stance::selection_after_human_lost(
                    &catalogue,
                    current.as_deref(),
                    red_alert.0,
                );
                store_resolved_selection(&mut stances, &target, &catalogue, resolved);
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
    active: Option<Res<ActiveObjectiveStances>>,
    mut ships: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &crate::ship::state::ShipRedAlert,
            &ShipStationStances,
            &mut AdmittedCommands,
            Option<&crate::entities::spawner::EntityUuid>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let active = active.as_deref();
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
            .policy_for(&crate::ship::system_registry::command_system_id())
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
        // The AI chooses from the SAME effective catalogue a human sees (issue
        // #1110): permanent stances plus any active objective contribution to
        // this target, so both consoles expose one vocabulary (AC2/AC3 parity).
        let contributed = contributed_for(active, &target);
        let catalogue = command_stance::effective_catalogue(&target_station.stances, &contributed);
        let Some(chosen) = command_stance::select_stance(
            &catalogue,
            command_stance::CommandKnowledge {
                red_alert: red_alert.0,
            },
        ) else {
            continue;
        };
        // The stance currently in force: an explicit stored selection, else the
        // alert level's neutral (the same tracking default the console shows).
        let in_force = stances.0.get(&target).cloned().or_else(|| {
            command_stance::neutral_stance_for_alert(&catalogue, red_alert.0).map(str::to_string)
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
    entity_uuid: Option<&crate::entities::spawner::EntityUuid>,
    payload: SystemControlPayload,
    sources: &ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    ship_config: Option<&ShipConfigComponent>,
    admitted: &mut AdmittedCommands,
) -> bool {
    crate::command_admission::ai_emit::emit_ai_command(
        entity_uuid,
        crate::ship::system_registry::command_system_id(),
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
    active: Option<Res<ActiveObjectiveStances>>,
    mut ships: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &crate::ship::state::ShipRedAlert,
            Option<&ShipStationStances>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let active = active.as_deref();
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
            .source_for(&crate::ship::system_registry::command_system_id())
            == ControlSource::Ai;

        // The EFFECTIVE catalogue this tick: permanent stances plus any active
        // objective contribution to this target (issue #1110). The console lists
        // exactly this, so an objective stance appears while its objective is
        // active and vanishes when it ends — the same vocabulary the AI and the
        // order applier read.
        let contributed = contributed_for(active, &target);
        let catalogue = command_stance::effective_catalogue(&target_station.stances, &contributed);

        // The stance in force: an explicit stored selection, else the alert
        // level's neutral (the tracking default the AI hosts see).
        let selected_stance = stances
            .and_then(|s| s.0.get(&target).cloned())
            .or_else(|| {
                command_stance::neutral_stance_for_alert(&catalogue, red_alert.0)
                    .map(str::to_string)
            })
            .unwrap_or_default();

        let options: Vec<CommandStanceOption> = catalogue
            .iter()
            .map(|stance| CommandStanceOption {
                id: stance.id.clone(),
                label: stance.label.clone(),
                kind: stance_kind_wire(stance.kind).to_string(),
                high_alert: stance.high_alert,
            })
            .collect();

        let bb = CommandBlackboard {
            command_system_id: crate::ship::system_registry::command_system_id(),
            directed_station: target.clone(),
            directed_station_name: target_station.name.clone(),
            directed_station_ai,
            command_auto,
            selected_stance,
            stances: options,
        };
        bbs.0.insert(
            SystemId(crate::ship::system_registry::COMMAND_SYSTEM_ID.to_string()),
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
