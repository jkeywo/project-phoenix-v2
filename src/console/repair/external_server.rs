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
//! # The one external-commitment source
//!
//! A team held against a NAMED target the crew designate from the repair
//! console is unavailable to the hull's own damage-control sweep: the human
//! dispatch router and the repair AI both read
//! [`ExternalRepairDispatch::committed_repair_teams`] off the same idle pool, so
//! a team sent to an ally cannot be undercut by whichever path did not know
//! about it. This was one of two external-commitment sources until #1166 (S12)
//! dissolved the operations coordinator; it is now the only one.

use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::console::repair::external::{
    dispatch_status, ExternalRepairConfig, ExternalRepairRefusal,
};
use crate::console::weapons::beam::TacticalRadarSelection;
use crate::core::messages::{
    AdmittedCommands, SystemAffinity, SystemBlackboard, SystemControlPayload,
};
use crate::core::task_lifecycle::{
    TaskLifecycleRequest, TaskSlot, TaskTerminalReason, TASK_VERB_EXTERNAL_REPAIR,
};
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::EntityUuid;
use crate::infrastructure::condition::ConditionAdjustment;
use crate::infrastructure::InfrastructureCondition;
use crate::ship::damage::DamageTier;
use crate::ship::system_registry::{repair_system_id, REPAIR_SYSTEM_ID};
use crate::world::server::WorldContentRuntime;

use super::server::{RepairRequestQueue, ShipRepairTeams};

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
    /// team never moves in the [`crate::modifiers::repair_teams::RepairTeams`] readout — it
    /// is still `Idle`, simply spoken for.
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
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut set: ParamSet<(
        // Gather: everything the dispatch verdict needs off each operator.
        Query<(
            Entity,
            &AdmittedCommands,
            &ExternalRepairDispatch,
            Option<&TacticalRadarSelection>,
            &Transform,
            Option<&ShipRepairTeams>,
        )>,
        // Every entity's position, to resolve the designated target's separation.
        Query<(&EntityUuid, &Transform)>,
        // Apply the verdict.
        Query<(&mut ExternalRepairDispatch, Option<&EntityUuid>)>,
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
            |(entity, admitted, dispatch, selection, transform, teams)| {
                let mut request = None;
                for cmd in admitted.for_target(REPAIR_SYSTEM_ID) {
                    match &cmd.payload {
                        SystemControlPayload::DispatchExternalRepair => {
                            // The team-availability answer: this dispatch is now
                            // the only external claim on the idle pool, so "is a
                            // team free" is exactly `free_team_indices(0)`.
                            let has_free_team = teams
                                .map(|t| !t.0.free_team_indices(0).is_empty())
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
        let Ok((mut dispatch, uuid)) = dispatches.get_mut(*entity) else {
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
                        // The activation opens at COMMIT, not at every tick a
                        // team is still out there working (issue #1345) — the
                        // same dispatch-and-claim shape Security's team
                        // assignment uses. A dispatch onto a FRESH target
                        // while a team is already abroad still needs to
                        // restart through the registry (it closes the old
                        // activation as `Restarted` and opens this one), but a
                        // re-dispatch onto the SAME target the ship is already
                        // working (a stale-UI double tap) changes nothing in
                        // the sim and must not report a phantom restart —
                        // `SystemControlPayload::Dock if !dock.engaged`
                        // (dock/server.rs) and `StartTransfer if
                        // !umbilical.running` (umbilical/server.rs) guard the
                        // same no-op at their own commit sites.
                        if dispatch.dispatched_target.as_deref() != lock.as_deref() {
                            push_lifecycle(
                                lifecycle.as_deref_mut(),
                                uuid,
                                TaskLifecycleRequest::Start {
                                    slot: dispatch_slot(uuid),
                                    target: lock.clone(),
                                },
                            );
                        }
                        dispatch.dispatched_target = lock.clone();
                        dispatch.last_refusal = None;
                    }
                    Err(refusal) => {
                        // A refused dispatch sends nobody: the target is left
                        // untouched and the reason is retained for the console.
                        // No lifecycle report — nothing was ever committed, the
                        // same "a refusal at dispatch time opens nothing" rule
                        // Security's own dispatch keeps.
                        dispatch.last_refusal = Some(refusal);
                    }
                }
            }
            Request::Recall => {
                // Report the cancel BEFORE clearing the claim, so a recall of an
                // idle dispatch (a stale-UI double tap) reports nothing at all
                // (issue #1345).
                if dispatch.dispatched_target.is_some() {
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        uuid,
                        TaskLifecycleRequest::End {
                            slot: dispatch_slot(uuid),
                            reason: TaskTerminalReason::Released,
                        },
                    );
                }
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
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut set: ParamSet<(
        Query<(Entity, &ExternalRepairDispatch, &Transform)>,
        Query<(&EntityUuid, &Transform)>,
        Query<(&mut ExternalRepairDispatch, Option<&EntityUuid>)>,
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
            let Ok((mut dispatch, uuid)) = dispatches.get_mut(row.entity) else {
                continue;
            };
            dispatch.dispatched_target = None;
            dispatch.last_refusal = Some(refusal);
            // This system only ever CLOSES a dispatch (it never opens one — that
            // is `handle_external_repair_commands`' job at commit time), so every
            // row it walks was live and the terminal is unconditional (issue
            // #1345). `refusal` is always `OutOfRange` here — the range
            // maintenance calls the pure verdict with a team already claimed and
            // a target already present, so only drift (or the target vanishing,
            // which reads identically) can fail it — but mapped through the same
            // table `handle_external_repair_commands` would use for hygiene.
            push_lifecycle(
                lifecycle.as_deref_mut(),
                uuid,
                TaskLifecycleRequest::End {
                    slot: dispatch_slot(uuid),
                    reason: terminal_reason_for(refusal),
                },
            );
        }
    }
}

/// The lifecycle slot a hull's external repair-team dispatch occupies (issue
/// #1345) — its uuid, the repair system, and the external-repair verb.
fn dispatch_slot(uuid: Option<&EntityUuid>) -> TaskSlot {
    TaskSlot::new(
        uuid.map(|u| u.0.clone()).unwrap_or_default(),
        REPAIR_SYSTEM_ID,
        TASK_VERB_EXTERNAL_REPAIR,
    )
}

/// Queue one lifecycle report, if there is a queue and the hull has a uuid to be
/// identified by (issue #1345). Mirrors `tractor::server::push_lifecycle`.
fn push_lifecycle(
    queue: Option<&mut EffectQueue<TaskLifecycleRequest>>,
    uuid: Option<&EntityUuid>,
    request: TaskLifecycleRequest,
) {
    if uuid.is_none() {
        return;
    }
    if let Some(queue) = queue {
        queue.0.push(request);
    }
}

/// The terminal reason a dropped external-repair dispatch reports, from the
/// refusal the pure dispatch verdict returned (issue #1345).
///
/// Only [`ExternalRepairRefusal::OutOfRange`] is reachable from `tick_external_
/// repair` (the only site that ever CLOSES a dispatch); `NoFreeTeam` and
/// `NoTarget` are refusals `handle_external_repair_commands` shows the console
/// at dispatch time and never turns into a lifecycle event, because nothing was
/// ever committed for them to end. Mapped exhaustively anyway, matching the
/// dock's and tractor's own defensive style.
fn terminal_reason_for(refusal: ExternalRepairRefusal) -> TaskTerminalReason {
    match refusal {
        ExternalRepairRefusal::NoFreeTeam => TaskTerminalReason::NotCapable,
        ExternalRepairRefusal::NoTarget => TaskTerminalReason::NoSuchTarget,
        ExternalRepairRefusal::OutOfRange => TaskTerminalReason::OutOfRange,
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
    // The condition-adjustment queue, extracted off `WorldContentRuntime` (issue
    // #1223) and owned by `InfrastructurePlugin`; this pusher feeds it. `Option`
    // so a reduced test app that runs the repair systems without
    // `InfrastructurePlugin` is a no-op rather than a panic — the same
    // defensiveness the former `Option<ResMut<WorldContentRuntime>>` had.
    condition_queue: Option<ResMut<EffectQueue<ConditionAdjustment>>>,
    time: Option<Res<Time>>,
    operators: Query<&ExternalRepairDispatch>,
    targets: Query<(&EntityUuid, &InfrastructureCondition)>,
) {
    // Collect from read-only queries first, so a world with no live dispatch
    // never takes the queue mutably and marks it changed on a quiet tick.
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
    // infrastructure and tractor ticks all keep.
    adjustments.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    let Some(mut condition_queue) = condition_queue else {
        return;
    };
    condition_queue.0.extend(adjustments);
}

/// Marks a ship whose external repair team the backfill host DISPATCHED to serve
/// a `FieldRepair` directive (issue #1162). Inserted on dispatch, removed on
/// recall; the host recalls only while it is present, so it never recalls a team
/// a console dispatched on the same AI-operated system. Not folded/snapshotted:
/// re-adopted from the still-present directive on resume.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct ExternalRepairAiDispatched;

/// Teams to hold back from a backfill field-repair dispatch (issue #1162): the
/// standing-dispatch commitment PLUS one reserved per outstanding CRITICAL
/// (Disabled/Destroyed) local repair. Pure (AGENTS.md rule 10) so the "without
/// starving critical repairs" policy is unit-testable Bevy-free.
///
/// Folded into the SAME [`crate::modifiers::repair_teams::RepairTeams::free_team_indices`]
/// availability answer the standing external dispatch already eats from — it
/// only makes the host MORE conservative than the applier's own `has_free_team`
/// gate, so an AI decision to dispatch is always one the applier admits.
fn dispatch_committed_teams(external_committed: u8, critical_local_repairs: u8) -> u8 {
    external_committed.saturating_add(critical_local_repairs)
}

/// The count of outstanding CRITICAL (Disabled/Destroyed) local repair requests
/// on `queue` — one team is reserved per such request (issue #1162).
fn critical_local_repairs(queue: Option<&RepairRequestQueue>) -> u8 {
    queue
        .map(|q| {
            q.entries
                .iter()
                .filter(|e| e.tier >= DamageTier::Disabled)
                .count() as u8
        })
        .unwrap_or(0)
}

/// Resolve a directive's named target to the UUID the ship's combat lock carries
/// (issue #1162): a world entity NAME through the runtime map, or the value
/// itself when it is already a UUID (or the runtime is absent).
fn resolve_field_repair_target(name: &str, runtime: Option<&WorldContentRuntime>) -> String {
    runtime
        .and_then(|rt| rt.name_to_uuid.get(name).cloned())
        .unwrap_or_else(|| name.to_string())
}

/// Backfill Repair external-dispatch AI (issue #1162).
///
/// On an active `FieldRepair` directive (Repair affinity) naming a target the
/// ship has LOCKED, dispatch a team abroad — WITHOUT starving its own hull's
/// critical repairs; with no such directive active, recall a team still working
/// abroad. The concrete command is exactly the `DispatchExternalRepair`/
/// `RecallExternalRepair` a human at the repair console emits, sent through the
/// SAME `emit_ai_command` seam so `handle_external_repair_commands` never learns
/// who spoke (AGENTS.md rule 6).
///
/// The lock is reached upstream by `ai_target_selection`'s `objective-operate`
/// source (D2), and the dispatch applier reads that same lock, so a human and an
/// AI dispatch of the same designated ally admit the byte-identical command.
///
/// "Without starving critical repairs" is the POLICY half this host adds atop
/// the symmetric command: it reserves one idle team for each of the hull's own
/// outstanding CRITICAL (Disabled/Destroyed) repair requests, folding that
/// reserve into the SAME shared free-team availability answer
/// (`RepairTeams::free_team_indices`) the operations commitment and any standing
/// external dispatch already eat from. It never dispatches a team the local
/// damage-control sweep needs to bring a knocked-out system back. Because the
/// reserve only makes the host MORE conservative than the applier's own
/// `has_free_team` check, an AI decision to dispatch is always one the applier
/// admits. Decides ONLY on the shared AI cadence (rule 7).
#[allow(clippy::type_complexity)]
pub fn operate_external_repair_ai(
    mut commands: Commands,
    sessions: Res<crate::lobby::Sessions>,
    runtime: Option<Res<WorldContentRuntime>>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut ships: Query<(
        Entity,
        Option<&EntityUuid>,
        &crate::ship_plugin::ShipSystemControlSources,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        &ExternalRepairDispatch,
        Option<&TacticalRadarSelection>,
        Option<&ShipRepairTeams>,
        Option<&RepairRequestQueue>,
        &crate::server_app::ShipSystemBlackboards,
        Has<ExternalRepairAiDispatched>,
        &mut AdmittedCommands,
    )>,
) {
    for (
        entity,
        uuid,
        sources,
        config,
        dispatch,
        lock,
        teams,
        repair_queue,
        blackboards,
        host_dispatched,
        mut admitted,
    ) in ships.iter_mut()
    {
        if !sources.0.policy_for(&repair_system_id()).operate_ai {
            continue;
        }
        let directive_target: Option<String> = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(vbb)) => crate::objectives::top_operate_directive(
                &vbb.scored_objectives,
                SystemAffinity::Repair,
                |d| crate::objectives::field_repair_directive_target(d).is_some(),
            )
            .and_then(crate::objectives::field_repair_directive_target)
            .map(str::to_string),
            _ => None,
        };

        let payload = match directive_target {
            Some(name) => {
                let resolved = resolve_field_repair_target(&name, runtime.as_deref());
                let locked_on_target = lock
                    .and_then(|l| l.0.as_deref())
                    .is_some_and(|locked| locked == resolved);
                // The one availability answer (AGENTS.md rule 6): the standing
                // external dispatch commitment, PLUS this host's own reserve of
                // one team per outstanding critical local repair — so helping an
                // ally never leaves a knocked-out system of our own unswept.
                let committed = dispatch_committed_teams(
                    dispatch.committed_repair_teams(),
                    critical_local_repairs(repair_queue),
                );
                let has_free_team = teams
                    .map(|t| !t.0.free_team_indices(committed).is_empty())
                    .unwrap_or(false);
                // Dispatch once the ordered ally is locked and a team is free
                // beyond every other claim. Idempotent — a team already working
                // this target emits nothing.
                let already_here = dispatch.dispatched_target.as_deref() == Some(resolved.as_str());
                let emit = (locked_on_target && has_free_team && !already_here)
                    .then_some(SystemControlPayload::DispatchExternalRepair);
                // Claim the dispatch as host-driven while a team is out under this
                // order (fresh send, or one already working the target).
                if (emit.is_some() || already_here) && !host_dispatched {
                    commands.entity(entity).insert(ExternalRepairAiDispatched);
                }
                emit
            }
            // No field-repair order: recall a team THIS HOST sent — never one a
            // console dispatched on the same AI-operated system.
            None => {
                if dispatch.dispatched_target.is_some() && host_dispatched {
                    commands
                        .entity(entity)
                        .remove::<ExternalRepairAiDispatched>();
                    // The scenario closing the task, not the operator changing
                    // their mind (issue #1345). Reported HERE, ahead of the
                    // `RecallExternalRepair` this emits and this system's own
                    // ordering ahead of `handle_external_repair_commands`, so the
                    // withdrawn order is the terminal the timeline records.
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        uuid,
                        TaskLifecycleRequest::End {
                            slot: dispatch_slot(uuid),
                            reason: TaskTerminalReason::OrderWithdrawn,
                        },
                    );
                    Some(SystemControlPayload::RecallExternalRepair)
                } else {
                    None
                }
            }
        };

        if let Some(payload) = payload {
            emit_ai_command(
                uuid,
                repair_system_id(),
                payload,
                sources,
                &sessions,
                config,
                &mut admitted,
            );
        }
    }
}

/// Register the external repair-dispatch systems (issue #1161). Called from
/// `RepairPlugin::build`.
pub fn register_external_repair(app: &mut App) {
    // Gated AI decider (issue #1162); `register_ai_cadence` is idempotent, and
    // `RepairPlugin` (this fn's caller) already installs it for `operate_repair_ai`.
    crate::ai::cadence::register_ai_cadence(app);
    // Authoritative-state exclusion declaration (issue #1221, Track 3 step C9).
    // `ExternalRepairAiDispatched` is the DERIVED "I am driving this external
    // repair" marker — re-derived every AI tick from the still-folded operate
    // directive plus the folded external-repair-dispatch state, never a second
    // copy of either, so a lost marker self-heals within one AI tick. Declared
    // here at its owning site, replacing the `EXCLUSIONS` const in
    // `tests/authoritative_state_enumeration.rs`; inert to the digest.
    {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<ExternalRepairAiDispatched>(
            StateClass::Derived,
            "external-repair-dispatch-state",
        );
    }
    app.add_systems(
        FixedUpdate,
        (
            // Backfill Repair external-dispatch AI (issue #1162): on the shared
            // AI cadence (rule 7), emitting Dispatch / Recall BEFORE
            // `handle_external_repair_commands` consumes the tick.
            operate_external_repair_ai
                .in_set(crate::sim_sets::SimSet::Input)
                .run_if(crate::ai::cadence::ai_tick_ready)
                .before(handle_external_repair_commands),
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

    fn critical_entry() -> crate::console::repair::server::RepairQueueEntry {
        crate::console::repair::server::RepairQueueEntry {
            station_id: "engineering".into(),
            station_label: "engineering".into(),
            tier: DamageTier::Disabled,
            deficit: 0.9,
        }
    }

    /// The "without starving critical repairs" reserve (issue #1162): one team is
    /// held back per outstanding CRITICAL (Disabled/Destroyed) local repair, and
    /// a merely-Damaged one reserves nothing.
    #[test]
    fn critical_local_repairs_reserves_one_team_each_and_damaged_reserves_none() {
        use crate::console::repair::server::RepairRequestQueue;
        assert_eq!(critical_local_repairs(None), 0);
        assert_eq!(
            critical_local_repairs(Some(&RepairRequestQueue { entries: vec![] })),
            0
        );
        // A merely-Damaged (non-critical) request reserves nothing.
        let damaged = crate::console::repair::server::RepairQueueEntry {
            tier: DamageTier::Damaged,
            ..critical_entry()
        };
        assert_eq!(
            critical_local_repairs(Some(&RepairRequestQueue {
                entries: vec![damaged],
            })),
            0
        );
        // Two Disabled requests reserve two teams.
        assert_eq!(
            critical_local_repairs(Some(&RepairRequestQueue {
                entries: vec![critical_entry(), critical_entry()],
            })),
            2
        );
    }

    /// The reserve folds into the SAME `free_team_indices` answer: a one-team
    /// hull with a critical local repair outstanding has NO team free to
    /// dispatch, but frees it the moment the local critical repair clears.
    #[test]
    fn a_one_team_hull_reserves_its_last_team_for_a_critical_local_repair() {
        use crate::modifiers::repair_teams::RepairTeams;
        let teams = RepairTeams::new(1);

        // A critical local repair reserves the one team → none free to dispatch.
        let committed = dispatch_committed_teams(0, 1);
        assert!(
            teams.free_team_indices(committed).is_empty(),
            "the last team must be reserved for a critical local repair"
        );

        // With no critical local repair, the one team is dispatchable.
        let committed = dispatch_committed_teams(0, 0);
        assert!(
            !teams.free_team_indices(committed).is_empty(),
            "with the local sweep clear the free team is available to help the ally"
        );
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

    // ── The task lifecycle (issue #1345) ─────────────────────────────────────

    use crate::modifiers::repair_teams::RepairTeams;
    use crate::server_app::ShipSystemBlackboards;

    const OPERATOR: &str = "operator-1";
    const ALLY: &str = "ally-1";

    fn app_with(target_at: Option<Vec3>, free_teams: usize) -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
        app.add_systems(
            Update,
            (handle_external_repair_commands, tick_external_repair).chain(),
        );
        let operator = app
            .world_mut()
            .spawn((
                EntityUuid(OPERATOR.into()),
                Transform::from_translation(Vec3::ZERO),
                TacticalRadarSelection(Some(ALLY.into())),
                ShipRepairTeams(RepairTeams::new(free_teams)),
                ExternalRepairDispatch::new(ExternalRepairConfig {
                    range: 600.0,
                    repair_rate: 8.0,
                }),
                AdmittedCommands::default(),
                ShipSystemBlackboards::default(),
            ))
            .id();
        if let Some(position) = target_at {
            app.world_mut().spawn((
                EntityUuid(ALLY.into()),
                Transform::from_translation(position),
            ));
        }
        (app, operator)
    }

    fn admit_dispatch(app: &mut App, operator: Entity) {
        app.world_mut()
            .entity_mut(operator)
            .get_mut::<AdmittedCommands>()
            .unwrap()
            .0
            .push(crate::core::messages::AdmittedCommand {
                target: repair_system_id(),
                payload: SystemControlPayload::DispatchExternalRepair,
                response_token: None,
            });
    }

    fn admit_recall(app: &mut App, operator: Entity) {
        app.world_mut()
            .entity_mut(operator)
            .get_mut::<AdmittedCommands>()
            .unwrap()
            .0
            .push(crate::core::messages::AdmittedCommand {
                target: repair_system_id(),
                payload: SystemControlPayload::RecallExternalRepair,
                response_token: None,
            });
    }

    fn drain_lifecycle(app: &mut App) -> Vec<TaskLifecycleRequest> {
        std::mem::take(
            &mut app
                .world_mut()
                .resource_mut::<EffectQueue<TaskLifecycleRequest>>()
                .0,
        )
    }

    fn dispatch_slot_for_test() -> TaskSlot {
        TaskSlot::new(OPERATOR, REPAIR_SYSTEM_ID, TASK_VERB_EXTERNAL_REPAIR)
    }

    /// A dispatch that commits opens exactly one activation, named for the
    /// designated ally.
    #[test]
    fn a_committed_dispatch_opens_one_activation() {
        let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 1);
        admit_dispatch(&mut app, operator);
        app.update();

        assert_eq!(
            app.world()
                .entity(operator)
                .get::<ExternalRepairDispatch>()
                .unwrap()
                .dispatched_target
                .as_deref(),
            Some(ALLY)
        );
        assert_eq!(
            drain_lifecycle(&mut app),
            vec![TaskLifecycleRequest::Start {
                slot: dispatch_slot_for_test(),
                target: Some(ALLY.into()),
            }]
        );
    }

    /// A dispatch refused at commit time (no free team) opens nothing — the
    /// same "a refusal at dispatch time opens nothing" rule Security's own
    /// dispatch keeps, since no team was ever actually claimed.
    #[test]
    fn a_dispatch_refused_at_commit_time_opens_nothing() {
        let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 0);
        admit_dispatch(&mut app, operator);
        app.update();

        assert!(app
            .world()
            .entity(operator)
            .get::<ExternalRepairDispatch>()
            .unwrap()
            .dispatched_target
            .is_none());
        assert!(
            drain_lifecycle(&mut app).is_empty(),
            "nothing was ever committed for a refused dispatch to end"
        );
    }

    /// A recall reports the cancel before clearing the claim, and a recall of
    /// an idle dispatch (a stale-UI double tap) reports nothing at all.
    #[test]
    fn a_recall_reports_released_exactly_once_and_an_idle_recall_reports_nothing() {
        let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 1);
        admit_dispatch(&mut app, operator);
        app.update();
        drain_lifecycle(&mut app);

        admit_recall(&mut app, operator);
        app.update();
        assert_eq!(
            drain_lifecycle(&mut app),
            vec![TaskLifecycleRequest::End {
                slot: dispatch_slot_for_test(),
                reason: TaskTerminalReason::Released,
            }]
        );

        // A second recall of the now-idle dispatch is a no-op.
        admit_recall(&mut app, operator);
        app.update();
        assert!(drain_lifecycle(&mut app).is_empty());
    }

    /// A repeat `DispatchExternalRepair` onto the SAME locked target is a sim
    /// no-op (the commit re-assigns the identical value) and must drain an
    /// empty lifecycle queue rather than reporting a phantom restart. A
    /// dispatch onto a DIFFERENT target is a genuine change of claim and
    /// still pushes a fresh `Start` (issue #1345).
    #[test]
    fn a_repeat_dispatch_onto_the_same_target_reports_nothing_but_a_new_target_still_starts() {
        const ALLY_TWO: &str = "ally-2";

        let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 1);
        admit_dispatch(&mut app, operator);
        app.update();
        assert_eq!(
            drain_lifecycle(&mut app),
            vec![TaskLifecycleRequest::Start {
                slot: dispatch_slot_for_test(),
                target: Some(ALLY.into()),
            }]
        );

        // A stale-UI double tap on the same lock: nothing changed in the sim,
        // so nothing should be reported.
        admit_dispatch(&mut app, operator);
        app.update();
        assert!(
            drain_lifecycle(&mut app).is_empty(),
            "a repeat dispatch onto the same target must not report a phantom restart"
        );
        assert_eq!(
            app.world()
                .entity(operator)
                .get::<ExternalRepairDispatch>()
                .unwrap()
                .dispatched_target
                .as_deref(),
            Some(ALLY)
        );

        // Re-lock onto a second ally: a genuinely different target still
        // starts a fresh activation (the old one is closed as `Restarted` by
        // the narrative emitter's own registry, downstream of this queue).
        app.world_mut().spawn((
            EntityUuid(ALLY_TWO.into()),
            Transform::from_translation(Vec3::new(100.0, 0.0, 0.0)),
        ));
        app.world_mut()
            .entity_mut(operator)
            .get_mut::<TacticalRadarSelection>()
            .unwrap()
            .0 = Some(ALLY_TWO.into());
        admit_dispatch(&mut app, operator);
        app.update();
        assert_eq!(
            drain_lifecycle(&mut app),
            vec![TaskLifecycleRequest::Start {
                slot: dispatch_slot_for_test(),
                target: Some(ALLY_TWO.into()),
            }]
        );
        assert_eq!(
            app.world()
                .entity(operator)
                .get::<ExternalRepairDispatch>()
                .unwrap()
                .dispatched_target
                .as_deref(),
            Some(ALLY_TWO)
        );
    }

    /// A target that drifts past the authored range ends a live dispatch with
    /// the mapped reason — `tick_external_repair` only ever CLOSES a dispatch,
    /// so this never opens a fresh activation of its own.
    #[test]
    fn a_target_that_drifts_out_of_range_ends_the_live_dispatch() {
        let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 1);
        admit_dispatch(&mut app, operator);
        app.update();
        drain_lifecycle(&mut app);

        let target = app
            .world_mut()
            .query::<(Entity, &EntityUuid)>()
            .iter(app.world())
            .find(|(_, uuid)| uuid.0 == ALLY)
            .map(|(e, _)| e)
            .unwrap();
        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_xyz(9000.0, 0.0, 0.0));
        app.update();

        assert_eq!(
            drain_lifecycle(&mut app),
            vec![TaskLifecycleRequest::End {
                slot: dispatch_slot_for_test(),
                reason: TaskTerminalReason::OutOfRange,
            }]
        );
        assert!(app
            .world()
            .entity(operator)
            .get::<ExternalRepairDispatch>()
            .unwrap()
            .dispatched_target
            .is_none());
    }
}
