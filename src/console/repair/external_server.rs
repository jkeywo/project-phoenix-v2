//! The Bevy adapter for external repair-team dispatch (issue #1161).
//!
//! Gathers the live world into scalars, hands them to the pure sibling
//! [`crate::console::repair::external`], and applies what comes back — the
//! per-ship [`ExternalRepairDispatch`] component, the fixed-tick systems that
//! take the dispatch/recall commands, decide whether a team may cross over,
//! bring it home when the range is lost, and pay the target's own condition
//! track while a team works there. Nothing here decides eligibility itself:
//! rule 10, the same split the tractor keeps between `coupling` and `server`.
//!
//! # The parallel-to-operations decision (keep the old path live)
//!
//! An external field-repair *operation* (#1026/#1027) still holds teams back as
//! a count against a scripted verb; this slice adds a SECOND external-commitment
//! source that holds a team against a NAMED target the crew designate from the
//! repair console, and works that target's condition directly. Both sources are
//! ADDITIVE: the human dispatch router and the repair AI each add
//! [`ExternalRepairDispatch::committed_repair_teams`] to the operations
//! commitment, so a hull can be running a scripted stabilise AND have sent a
//! team to an ally at once, and both eat from the same idle pool. The operations
//! path is untouched — #1166 (S12) retires it, not this slice.

use bevy::prelude::*;

use crate::console::repair::external::{
    dispatch_status, ExternalRepairConfig, ExternalRepairRefusal,
};
use crate::console::weapons::beam::TacticalRadarSelection;
use crate::entities::spawner::EntityUuid;
use crate::infrastructure::condition::ConditionAdjustment;
use crate::infrastructure::InfrastructureCondition;
use crate::messages::{AdmittedCommands, SystemControlPayload};
use crate::ship::system_registry::REPAIR_SYSTEM_ID;
use crate::world::server::WorldContentRuntime;

use super::server::ShipRepairTeams;

/// One ship's external repair-dispatch state (issue #1161): the authored reach
/// and rate, and which target a team is currently working abroad.
///
/// Inserted at spawn only on a hull that authored a `[repair.external_dispatch]`
/// table AND declares repair teams — a hull with neither carries no component
/// and is byte-identical in every way to one built before this existed
/// (AGENTS.md rule 11).
///
/// `dispatched_target` is what a team is actually working this tick,
/// `Some(target-uuid)` while a dispatch is live and `None` when the team is home.
/// It commits immediately at dispatch time — unlike the tractor's engage/hold
/// split, a dispatched team is a resource CLAIM, not a beam re-evaluated every
/// tick: the free-team gate is a dispatch-time check, and once a team is over
/// there only drifting past the range (or an explicit recall) brings it home.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ExternalRepairDispatch {
    /// The authored dispatch terms — the reach and the repair rate.
    pub config: ExternalRepairConfig,
    /// The target-uuid a team is working abroad this tick, or `None` when the
    /// team is home. Present only while a dispatch is live.
    pub dispatched_target: Option<String>,
    /// Why the last dispatch could not form, or why a live dispatch was brought
    /// home — the reason the console shows, retained until the operator
    /// dispatches or recalls again. `None` when idle or working cleanly.
    pub last_refusal: Option<ExternalRepairRefusal>,
}

impl ExternalRepairDispatch {
    /// A fresh, idle dispatch record carrying its authored terms.
    pub fn new(config: ExternalRepairConfig) -> Self {
        Self {
            config,
            dispatched_target: None,
            last_refusal: None,
        }
    }

    /// How many of this ship's repair teams are held back by an external
    /// dispatch (issue #1161) — **the count this slice adds to the internal-
    /// sweep availability answer.**
    ///
    /// One team per live dispatch (a ship designates one target at a time).
    /// Derived from the live `dispatched_target` rather than stored, which is
    /// what makes "returned on recall or drift" true by construction: a team
    /// brought home commits nothing, so there is no release step to forget. The
    /// team never moves in the [`crate::repair_teams::RepairTeams`] readout — it
    /// is still `Idle`, simply spoken for — exactly as the operations commitment
    /// holds one.
    pub fn committed_repair_teams(&self) -> u8 {
        u8::from(self.dispatched_target.is_some())
    }

    /// The persistable half — the dispatched target — for the snapshot payload
    /// (issue #1161). The authored config rides the template and is re-derived
    /// on spawn, so it is deliberately not here, exactly as `TractorSaveState`
    /// leaves the coupling terms out.
    pub fn save_state(&self) -> ExternalRepairSaveState {
        ExternalRepairSaveState {
            dispatched_target: self.dispatched_target.clone(),
        }
    }

    /// Reseed the dispatched target from a restored snapshot (issue #1161), onto
    /// a record that already carries its authored config from the fresh spawn.
    ///
    /// The last refusal is deliberately NOT restored: it is a projection the
    /// next tick re-derives (a resumed dispatch that comes back out of range
    /// refuses again on its first tick), so carrying a stale one would show the
    /// crew a reason for a condition that no longer holds.
    pub fn restore(&mut self, save: &ExternalRepairSaveState) {
        self.dispatched_target = save.dispatched_target.clone();
        self.last_refusal = None;
    }
}

/// The snapshot-carried half of an [`ExternalRepairDispatch`] (issue #1161): the
/// dispatched target, and nothing else.
///
/// `Default` is the idle record — no team abroad — which is what a hull that
/// authored external dispatch and never used it captures, so a resume of such a
/// ship restores byte-identically and folds the same number.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExternalRepairSaveState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_target: Option<String>,
}

// ── The dispatch / recall commands ───────────────────────────────────────────

/// Take this tick's `DispatchExternalRepair` / `RecallExternalRepair` commands
/// for the repair system and decide, at dispatch time, whether a team may cross
/// over (issue #1161).
///
/// Runs in `SimSet::Input`, so `dispatched_target` is set before the internal
/// dispatch router and the repair AI read the committed count in
/// `SimSet::Physics` — a team sent abroad this tick is withdrawn from the same
/// tick's internal sweep.
///
/// The full pure [`dispatch_status`] is checked HERE, at dispatch time, because
/// the free-team gate is a resource claim: once a team is over there it must not
/// be recalled merely because the hull later put its other teams on internal
/// jobs. The lighter per-tick range maintenance is [`tick_external_repair`]'s.
///
/// Human and AI reach this identically: admission has already decided who may
/// speak and stripped the source, so nothing here asks who sent the command
/// (AGENTS.md rule 6). The AI host proper is #1162; this slice makes the command
/// admissible from the repair console and reads the same availability answer the
/// AI dispatcher does.
pub fn handle_external_repair_commands(
    mut set: ParamSet<(
        // Gather: everything the dispatch verdict needs off each operator.
        Query<(
            Entity,
            &AdmittedCommands,
            &ExternalRepairDispatch,
            Option<&TacticalRadarSelection>,
            &Transform,
            Option<&ShipRepairTeams>,
            Option<&crate::operations::ShipOperations>,
        )>,
        // Every entity's position, to resolve the designated target's separation.
        Query<(&EntityUuid, &Transform)>,
        // Apply the verdict.
        Query<&mut ExternalRepairDispatch>,
    )>,
) {
    // One request per operator this tick — the latest command wins, the same
    // latest-wins policy the tractor and helm axes take, so a stale-UI double
    // tap is idempotent.
    enum Request {
        Dispatch {
            lock: Option<String>,
            operator_pos: Vec3,
            has_free_team: bool,
            range: f32,
        },
        Recall,
    }
    let requests: Vec<(Entity, Request)> = set
        .p0()
        .iter()
        .filter_map(
            |(entity, admitted, dispatch, selection, transform, teams, operations)| {
                let mut request = None;
                for cmd in admitted.for_target(REPAIR_SYSTEM_ID) {
                    match &cmd.payload {
                        SystemControlPayload::DispatchExternalRepair => {
                            // The team-availability answer, EXCLUDING this
                            // component's own would-be dispatch: the operations
                            // commitment is the only other external source today, so
                            // "is a team free beyond what a scripted operation
                            // holds" is exactly `free_team_indices(ops_committed)`.
                            let ops_committed =
                                operations.map(|o| o.committed_repair_teams()).unwrap_or(0);
                            let has_free_team = teams
                                .map(|t| !t.0.free_team_indices(ops_committed).is_empty())
                                .unwrap_or(false);
                            request = Some(Request::Dispatch {
                                lock: selection.and_then(|s| s.0.clone()),
                                operator_pos: transform.translation,
                                has_free_team,
                                range: dispatch.config.range,
                            });
                        }
                        SystemControlPayload::RecallExternalRepair => {
                            request = Some(Request::Recall);
                        }
                        _ => {}
                    }
                }
                request.map(|r| (entity, r))
            },
        )
        .collect();
    if requests.is_empty() {
        return;
    }

    // Resolve each dispatch's separation to its designated target, once, from
    // the transform query. `None` when the lock names an entity that no longer
    // exists, which the verdict reads as out of range.
    let separations: Vec<Option<f32>> = {
        let transforms = set.p1();
        requests
            .iter()
            .map(|(_, request)| match request {
                Request::Dispatch {
                    lock, operator_pos, ..
                } => {
                    let lock = lock.as_deref()?;
                    let target = transforms
                        .iter()
                        .find(|(uuid, _)| uuid.0 == lock)
                        .map(|(_, t)| t.translation)?;
                    Some(operator_pos.distance(target))
                }
                Request::Recall => None,
            })
            .collect()
    };

    // Apply.
    let mut dispatches = set.p2();
    for ((entity, request), separation) in requests.iter().zip(separations) {
        let Ok(mut dispatch) = dispatches.get_mut(*entity) else {
            continue;
        };
        match request {
            Request::Dispatch {
                lock,
                has_free_team,
                range,
                ..
            } => {
                match dispatch_status(*has_free_team, lock.as_deref(), separation, *range) {
                    Ok(()) => {
                        dispatch.dispatched_target = lock.clone();
                        dispatch.last_refusal = None;
                    }
                    Err(refusal) => {
                        // A refused dispatch sends nobody: the target is left
                        // untouched and the reason is retained for the console.
                        dispatch.last_refusal = Some(refusal);
                    }
                }
            }
            Request::Recall => {
                // Recall brings the team home and stops the work, leaving what it
                // already did on the target. A deliberate recall is not a
                // refusal, so the reason clears.
                dispatch.dispatched_target = None;
                dispatch.last_refusal = None;
            }
        }
    }
}

// ── The range maintenance ────────────────────────────────────────────────────

/// Bring home every dispatched team whose target has drifted out of the authored
/// range (issue #1161).
///
/// Runs in `SimSet::Modifiers`, after the operators have moved. For each ship
/// working a target abroad, re-run the pure verdict with the team already
/// claimed (`has_free_team = true`, the captured target present) so the only
/// thing that can drop the dispatch is the range: a target that has drifted past
/// `range`, or vanished, ends the work and records the reason. A recall leaves
/// the work already done on the target — this system stops queuing new condition
/// the instant `dispatched_target` clears, exactly as releasing a tractor stops
/// its arrest.
pub fn tick_external_repair(
    mut set: ParamSet<(
        Query<(Entity, &ExternalRepairDispatch, &Transform)>,
        Query<(&EntityUuid, &Transform)>,
        Query<&mut ExternalRepairDispatch>,
    )>,
) {
    struct Row {
        entity: Entity,
        target: String,
        operator_pos: Vec3,
        range: f32,
    }
    let rows: Vec<Row> = set
        .p0()
        .iter()
        .filter_map(|(entity, dispatch, transform)| {
            dispatch.dispatched_target.as_ref().map(|target| Row {
                entity,
                target: target.clone(),
                operator_pos: transform.translation,
                range: dispatch.config.range,
            })
        })
        .collect();
    if rows.is_empty() {
        return;
    }

    let separations: Vec<Option<f32>> = {
        let transforms = set.p1();
        rows.iter()
            .map(|row| {
                let target = transforms
                    .iter()
                    .find(|(uuid, _)| uuid.0 == row.target)
                    .map(|(_, t)| t.translation)?;
                Some(row.operator_pos.distance(target))
            })
            .collect()
    };

    let mut dispatches = set.p2();
    for (row, separation) in rows.iter().zip(separations) {
        if let Err(refusal) =
            dispatch_status(true, Some(row.target.as_str()), separation, row.range)
        {
            let Ok(mut dispatch) = dispatches.get_mut(row.entity) else {
                continue;
            };
            dispatch.dispatched_target = None;
            dispatch.last_refusal = Some(refusal);
        }
    }
}

// ── The work ─────────────────────────────────────────────────────────────────

/// Pay each dispatched team's target its authored repair rate this tick (issue
/// #1161).
///
/// For every ship working a coupled target that carries an infrastructure
/// condition track, queue a [`ConditionAdjustment`] of `repair_rate * dt` on the
/// target's OWN track. Ordered after [`tick_external_repair`] (so a team brought
/// home this tick banks nothing) and BEFORE `tick_infrastructure_condition` (so
/// the adjustment lands the same tick it was earned), the exact ordering
/// `arrest_held_declines` keeps.
///
/// The delta is purely additive — the team is repairing, not arresting a
/// decline — so it composes with a tractor's arrest on the same target: both
/// push onto the one condition queue and `tick_infrastructure_condition` sums
/// them. Going through the queue rather than onto the component is what keeps the
/// repaired condition crossing the target's OWN authored thresholds, by the one
/// system that owns the flag edges.
pub fn apply_external_repair(
    runtime: Option<ResMut<WorldContentRuntime>>,
    time: Option<Res<Time>>,
    operators: Query<&ExternalRepairDispatch>,
    targets: Query<(&EntityUuid, &InfrastructureCondition)>,
) {
    // Collect from read-only queries first, so a world with no live dispatch
    // never takes `WorldContentRuntime` mutably and marks it changed on a quiet
    // tick.
    let dt = time.map(|t| t.delta_secs()).unwrap_or(0.0);
    let mut adjustments: Vec<ConditionAdjustment> = operators
        .iter()
        .filter_map(|dispatch| {
            let target_uuid = dispatch.dispatched_target.as_ref()?;
            // Only a target that carries a condition track can be worked; a
            // dispatch to something without one holds the team but banks nothing.
            targets.iter().find(|(uuid, _)| &uuid.0 == target_uuid)?;
            let delta = dispatch.config.repair_rate * dt;
            if delta == 0.0 {
                return None;
            }
            Some(ConditionAdjustment {
                uuid: target_uuid.clone(),
                delta,
            })
        })
        .collect();
    if adjustments.is_empty() {
        return;
    }
    // UUID order so two hosts queue identically — the walk-order rule the
    // infrastructure, operations and tractor ticks all keep.
    adjustments.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    let Some(mut runtime) = runtime else {
        return;
    };
    runtime.pending_condition_adjustments.extend(adjustments);
}

/// Register the external repair-dispatch systems (issue #1161). Called from
/// `RepairPlugin::build`.
pub fn register_external_repair(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        (
            handle_external_repair_commands.in_set(crate::sim_sets::SimSet::Input),
            tick_external_repair.in_set(crate::sim_sets::SimSet::Modifiers),
            apply_external_repair
                .in_set(crate::sim_sets::SimSet::Modifiers)
                .after(tick_external_repair)
                .before(crate::infrastructure::tick_infrastructure_condition),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> ExternalRepairDispatch {
        ExternalRepairDispatch::new(ExternalRepairConfig {
            range: 600.0,
            repair_rate: 8.0,
        })
    }

    #[test]
    fn a_live_dispatch_commits_one_team_and_an_idle_record_commits_none() {
        let mut r = record();
        assert_eq!(r.committed_repair_teams(), 0);
        r.dispatched_target = Some("ally-1".into());
        assert_eq!(r.committed_repair_teams(), 1);
    }

    #[test]
    fn save_state_carries_the_dispatched_target_only() {
        let mut r = record();
        r.dispatched_target = Some("ally-1".into());
        r.last_refusal = Some(ExternalRepairRefusal::OutOfRange);
        let save = r.save_state();
        assert_eq!(save.dispatched_target.as_deref(), Some("ally-1"));
    }

    #[test]
    fn an_idle_record_saves_as_default() {
        assert_eq!(record().save_state(), ExternalRepairSaveState::default());
    }

    #[test]
    fn restore_reseeds_the_target_and_clears_any_stale_refusal() {
        let mut r = record();
        r.last_refusal = Some(ExternalRepairRefusal::NoFreeTeam);
        r.restore(&ExternalRepairSaveState {
            dispatched_target: Some("ally-2".into()),
        });
        assert_eq!(r.dispatched_target.as_deref(), Some("ally-2"));
        assert!(r.last_refusal.is_none());
    }
}
