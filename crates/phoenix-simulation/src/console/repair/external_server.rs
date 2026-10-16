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
//! dispatch router and the repair AI both ask
//! `ShipRepairTeams::external_team` which slot is spoken for, so a team
//! sent to an ally cannot be undercut by whichever path did not know about it.
//! Until #1386 that answer was a COUNT and every reader truncated the idle list
//! to guess which slot it meant; naming the team is what lets a seat choose one.
//! This was one of two external-commitment sources until #1166 (S12) dissolved
//! the operations coordinator; it is now the only one.

use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::console::repair::external::{
    dispatch_status, named_dispatch_status, ExternalRepairConfig, ExternalRepairRefusal,
};
use crate::console::weapons::beam::TacticalRadarSelection;
use crate::core::messages::{
    AdmittedCommands, RepairTarget, SystemAffinity, SystemBlackboard, SystemControlPayload,
    TeamSlot,
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

/// Authored external-repair terms and transient feedback. Team commitments
/// live exclusively in ShipRepairTeams, alongside internal assignments.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ExternalRepairDispatch {
    pub config: ExternalRepairConfig,
    pub last_refusal: Option<ExternalRepairRefusal>,
}
impl ExternalRepairDispatch {
    pub fn new(config: ExternalRepairConfig) -> Self {
        Self {
            config,
            last_refusal: None,
        }
    }
}

// ── The dispatch / recall commands ───────────────────────────────────────────

/// Take this tick's field-repair dispatch commands for the repair system and
/// decide, at dispatch time, whether a team may cross over (issues #1161,
/// #1386).
///
/// # Two dispatch verbs, one commit site
///
/// `DispatchExternalRepair` is FIELDLESS and means "send somebody": the server
/// picks the lowest free team. It is the AI / operate-directive vocabulary
/// (#1162), and it is what today's implicit rule always was, now written down.
/// `DispatchRepairTeam { team_idx, target: External }` NAMES its team and is
/// what a seat sends from that team's own card (#1386) — the same shape a
/// station dispatch takes, so choosing a destination is one decision on the
/// console whichever side of the hull it lands on. Both resolve HERE, against
/// the same lock, the same reach and the same claim, so neither can express
/// something the other cannot see.
///
/// `RecallExternalRepair` stays for the directive path. The named recall,
/// `RecallRepairTeam`, is `super::dispatch::handle_recall_repair_team`'s — one
/// verb for a team whatever it is doing.
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
            Option<&ExternalRepairDispatch>,
            Option<&TacticalRadarSelection>,
            &Transform,
            Option<&ShipRepairTeams>,
        )>,
        // Every entity's position, to resolve the designated target's separation.
        Query<(&EntityUuid, &Transform)>,
        // Apply the verdict.
        Query<(
            &mut ExternalRepairDispatch,
            &mut ShipRepairTeams,
            Option<&EntityUuid>,
        )>,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    // One request per operator this tick — the latest command wins, the same
    // latest-wins policy the tractor and helm axes take, so a stale-UI double
    // tap is idempotent.
    enum Request {
        Dispatch {
            command: crate::core::messages::AdmittedCommand,
            lock: Option<String>,
            operator_pos: Vec3,
            range: f32,
            /// `Some(idx)` for a NAMED dispatch, `None` for the fieldless verb.
            /// This is what selects which pure verdict answers below — a named
            /// order can be refused two ways the fieldless one structurally
            /// cannot (issue #1386).
            named: Option<u8>,
            /// The slot that would actually cross over: the named one, or (for
            /// the fieldless verb) the team already abroad re-pointing at a new
            /// lock, else the lowest free team. `None` only when the fieldless
            /// verb found nobody, which the verdict reads as `NoFreeTeam`.
            chosen: Option<u8>,
            /// Whether the NAMED slot is `Idle` — false for a busy team and for
            /// a slot this hull does not have.
            team_is_idle: bool,
            /// Whoever currently holds this ship's one external claim.
            abroad: Option<u8>,
        },
        Recall {
            command: crate::core::messages::AdmittedCommand,
        },
        Unavailable {
            command: crate::core::messages::AdmittedCommand,
        },
    }
    impl Request {
        fn command(&self) -> &crate::core::messages::AdmittedCommand {
            match self {
                Self::Dispatch { command, .. }
                | Self::Recall { command }
                | Self::Unavailable { command } => command,
            }
        }
    }
    let requests: Vec<(Entity, Request)> = set
        .p0()
        .iter()
        .filter_map(
            |(entity, admitted, dispatch, selection, transform, teams)| {
                let mut request = None;
                // Whoever holds this ship's one external claim right now — the
                // team the availability answer excludes, and the team a named
                // dispatch of ANOTHER slot is refused against (issue #1386).
                let abroad = teams.and_then(|t| t.0.external_team()).map(|idx| idx as u8);
                for cmd in admitted.for_target(REPAIR_SYSTEM_ID) {
                    // The three field-repair verbs this system answers, read off
                    // the payload once. `Send(None)` is the fieldless dispatch
                    // (the server picks the team); `Send(Some(idx))` names its
                    // slot (issue #1386); everything else on the repair system
                    // belongs to another applier and is not this system's to
                    // report on.
                    enum Verb {
                        Send(Option<u8>),
                        BringHome,
                    }
                    let verb = match &cmd.payload {
                        SystemControlPayload::DispatchExternalRepair => Verb::Send(None),
                        SystemControlPayload::DispatchRepairTeam {
                            team_idx,
                            target: RepairTarget::External,
                        } => Verb::Send(Some(*team_idx)),
                        SystemControlPayload::RecallExternalRepair => Verb::BringHome,
                        _ => continue,
                    };
                    let next = match (verb, dispatch) {
                        // Correlated external commands are protocol-allowlisted
                        // for the Repair owner. A hull that omits the optional
                        // capability must therefore still terminate the promise
                        // explicitly.
                        (_, None) => Request::Unavailable {
                            command: cmd.clone(),
                        },
                        (Verb::BringHome, Some(_)) => Request::Recall {
                            command: cmd.clone(),
                        },
                        (Verb::Send(named), Some(dispatch)) => {
                            // The team-availability answer, asked the one way
                            // (rule 6): the idle pool minus whoever is already
                            // abroad. A fieldless re-dispatch onto a fresh lock
                            // keeps the team already out there rather than
                            // spending a second one — the claim stays single.
                            let free_team = teams.and_then(|t| {
                                t.0.free_team_indices(abroad.map(usize::from))
                                    .first()
                                    .map(|&idx| idx as u8)
                            });
                            let chosen = named.or(abroad).or(free_team);
                            // Only a NAMED order can name a busy slot; the
                            // fieldless verb picked a free one by construction
                            // and reports `NoFreeTeam` when it could not.
                            let team_is_idle = named
                                .and_then(|idx| teams.map(|t| (t, idx)))
                                .is_some_and(|(t, idx)| {
                                    matches!(
                                        t.0.slots().get(usize::from(idx)),
                                        Some(TeamSlot::Idle)
                                    )
                                });
                            Request::Dispatch {
                                command: cmd.clone(),
                                lock: selection.and_then(|s| s.0.clone()),
                                operator_pos: transform.translation,
                                range: dispatch.config.range,
                                named,
                                chosen,
                                team_is_idle,
                                abroad,
                            }
                        }
                    };
                    if let Some(previous) = request.replace(next) {
                        crate::command_admission::finish_admitted_action_feedback(
                            &mut outbound,
                            previous.command(),
                            crate::core::messages::ActionFeedbackOutcome::Refused,
                        );
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
                Request::Recall { .. } | Request::Unavailable { .. } => None,
            })
            .collect()
    };

    // Apply.
    let mut dispatches = set.p2();
    for ((entity, request), separation) in requests.iter().zip(separations) {
        let Ok((mut dispatch, mut teams, uuid)) = dispatches.get_mut(*entity) else {
            crate::command_admission::finish_admitted_action_feedback(
                &mut outbound,
                request.command(),
                crate::core::messages::ActionFeedbackOutcome::Refused,
            );
            continue;
        };
        match request {
            Request::Dispatch {
                lock,
                range,
                named,
                chosen,
                team_is_idle,
                abroad,
                ..
            } => {
                // A named order answers through the named verdict, which reports
                // the two staleness cases the fieldless one cannot have (issue
                // #1386); the fieldless verb keeps the availability question it
                // has always asked, now answered by whether a team was found.
                let verdict = match named {
                    Some(team_idx) => named_dispatch_status(
                        *team_is_idle,
                        *abroad,
                        *team_idx,
                        lock.as_deref(),
                        separation,
                        *range,
                    ),
                    None => dispatch_status(chosen.is_some(), lock.as_deref(), separation, *range),
                };
                match verdict {
                    Ok(()) => {
                        let changed = teams.0.external_target() != lock.as_deref();
                        let committed = chosen.zip(lock.as_ref()).is_some_and(|(team, target)| {
                            teams.0.claim_external(team as usize, target.clone())
                        });
                        if !committed {
                            dispatch.last_refusal = Some(ExternalRepairRefusal::TeamBusy);
                            crate::command_admission::finish_admitted_action_feedback(
                                &mut outbound,
                                request.command(),
                                crate::core::messages::ActionFeedbackOutcome::Refused,
                            );
                            continue;
                        }
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
                        if changed {
                            push_lifecycle(
                                lifecycle.as_deref_mut(),
                                uuid,
                                TaskLifecycleRequest::Start {
                                    slot: dispatch_slot(uuid),
                                    target: lock.clone(),
                                },
                            );
                        }
                        dispatch.last_refusal = None;
                        crate::command_admission::finish_admitted_action_feedback(
                            &mut outbound,
                            request.command(),
                            crate::core::messages::ActionFeedbackOutcome::Applied,
                        );
                    }
                    Err(refusal) => {
                        // A refused dispatch sends nobody: the target is left
                        // untouched and the reason is retained for the console.
                        // No lifecycle report — nothing was ever committed, the
                        // same "a refusal at dispatch time opens nothing" rule
                        // Security's own dispatch keeps.
                        dispatch.last_refusal = Some(refusal);
                        crate::command_admission::finish_admitted_action_feedback(
                            &mut outbound,
                            request.command(),
                            crate::core::messages::ActionFeedbackOutcome::Refused,
                        );
                    }
                }
            }
            Request::Recall { .. } => {
                // Report the cancel BEFORE clearing the claim, so a recall of an
                // idle dispatch (a stale-UI double tap) reports nothing at all
                // (issue #1345).
                if teams.0.external_target().is_some() {
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
                teams.0.release_external();
                dispatch.last_refusal = None;
                crate::command_admission::finish_admitted_action_feedback(
                    &mut outbound,
                    request.command(),
                    crate::core::messages::ActionFeedbackOutcome::Applied,
                );
            }
            Request::Unavailable { .. } => {
                // The mutable-component lookup above always handles this arm.
                // Keep it exhaustive in case the gather/apply query changes.
                crate::command_admission::finish_admitted_action_feedback(
                    &mut outbound,
                    request.command(),
                    crate::core::messages::ActionFeedbackOutcome::Refused,
                );
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
        Query<(
            Entity,
            &ExternalRepairDispatch,
            &ShipRepairTeams,
            &Transform,
        )>,
        Query<(&EntityUuid, &Transform)>,
        Query<(
            &mut ExternalRepairDispatch,
            &mut ShipRepairTeams,
            Option<&EntityUuid>,
        )>,
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
        .filter_map(|(entity, dispatch, teams, transform)| {
            teams.0.external_target().map(|target| Row {
                entity,
                target: target.to_string(),
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
            let Ok((mut dispatch, mut teams, uuid)) = dispatches.get_mut(row.entity) else {
                continue;
            };
            teams.0.release_external();
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
///
/// `pub(crate)` since issue #1386: `super::dispatch::handle_recall_repair_team`
/// closes this same activation when a named recall releases the claim, and two
/// spellings of one slot identity is exactly the drift that would let a timeline
/// carry an End nothing ever opened.
pub(crate) fn dispatch_slot(uuid: Option<&EntityUuid>) -> TaskSlot {
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
        // Named-dispatch refusals (issue #1386), unreachable here for the same
        // reason the two above are: they are commit-time answers to an order
        // that claimed nothing, so there is no activation for them to end.
        ExternalRepairRefusal::TeamBusy | ExternalRepairRefusal::AlreadyAbroad => {
            TaskTerminalReason::NotCapable
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
#[allow(clippy::type_complexity)]
pub fn apply_external_repair(
    mut condition_queue: Option<ResMut<EffectQueue<ConditionAdjustment>>>,
    time: Option<Res<Time>>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
    mut balance: Option<ResMut<Messages<crate::core::balance::BalanceEvent>>>,
    mut set: ParamSet<(
        Query<(
            Entity,
            Option<&EntityUuid>,
            &ExternalRepairDispatch,
            &mut ShipRepairTeams,
            Option<&mut crate::ship::power::ShipPowerSystem>,
            Option<&mut InfrastructureCondition>,
            Option<&crate::resources::server::ResourceServices>,
            Option<&crate::ship::compartment_damage::ShipDamage>,
            Option<&crate::ship_plugin::ShipSystemControlSources>,
        )>,
        Query<(
            Entity,
            &EntityUuid,
            Option<&mut crate::ship::compartment_damage::ShipDamage>,
            Option<&InfrastructureCondition>,
        )>,
        Query<(&EntityUuid, &ShipRepairTeams)>,
        Query<(
            &EntityUuid,
            &crate::resources::equipment_recovery_server::ShipEquipmentRecovery,
        )>,
    )>,
) {
    use crate::jobs::repair::RepairStockAction;
    use phoenix_model::damage::{DamageBinding, RepairWorkTarget};
    use std::collections::{BTreeMap, BTreeSet};
    let dt = time.map_or(0.0, |time| time.delta_secs());
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    struct Row {
        entity: Entity,
        uuid: Option<EntityUuid>,
        config: ExternalRepairConfig,
        teams: crate::jobs::repair::RepairTeams,
        power: Option<crate::ship::power::ShipPowerSystem>,
        inventory: Option<InfrastructureCondition>,
        services: Option<crate::resources::server::ResourceServices>,
        operation: f32,
        tools_available: bool,
    }
    // Clone only this synchronous transaction's source owners. No second store
    // survives the call, and source + target work commits in stable UUID order.
    let mut rows: Vec<_> = set
        .p0()
        .iter_mut()
        .filter_map(
            |(entity, uuid, dispatch, teams, power, inventory, services, damage, sources)| {
                // A released/replaced Job may still own a reserved physical stage.
                // Keep that row until the ordinary cleanup releases its claim;
                // other internal/refit/returning jobs have no effect in this pass.
                if teams.0.external_target().is_none()
                    && teams.0.job_state().external_physical_work.is_empty()
                {
                    return None;
                }
                Some(Row {
                    entity,
                    uuid: uuid.cloned(),
                    config: dispatch.config.clone(),
                    teams: teams.0.clone(),
                    power: power.map(|power| power.clone()),
                    inventory: inventory.map(|stock| stock.clone()),
                    services: services.cloned(),
                    operation: damage.map_or(1.0, |damage| {
                        damage.factor(teams.0.owner(), DamageBinding::Operation)
                    }),
                    tools_available: sources
                        .is_none_or(|sources| !sources.0.is_offline(teams.0.owner())),
                })
            },
        )
        .collect();
    // No work or stale claims means there is no transaction, lifecycle edge,
    // clock or target mutation to apply. Avoid rebuilding the world claim view.
    if rows.is_empty() {
        return;
    }
    rows.sort_by(|a, b| {
        a.uuid
            .as_ref()
            .map(|id| &id.0)
            .cmp(&b.uuid.as_ref().map(|id| &id.0))
            .then(a.entity.index().cmp(&b.entity.index()))
    });
    let physical_targets: BTreeSet<_> = set
        .p1()
        .iter()
        .filter(|(_, _, damage, _)| damage.as_ref().is_some_and(|damage| !damage.catastrophic()))
        .map(|(_, uuid, _, _)| uuid.0.clone())
        .collect();
    // Claims are reconstructed from the SAME saved jobs, including target-local
    // teams. There is no mutable global claim registry or shared crew pool.
    let mut claims: BTreeMap<(String, RepairWorkTarget), BTreeSet<(String, u64)>> = BTreeMap::new();
    let refit_components: BTreeMap<_, Vec<_>> = set
        .p1()
        .iter()
        .flat_map(|(_, uuid, damage, _)| {
            damage
                .as_ref()
                .into_iter()
                .flat_map(move |damage| {
                    damage
                        .definition
                        .components
                        .iter()
                        .map(move |(system, components)| {
                            (
                                (uuid.0.clone(), system.clone()),
                                components
                                    .keys()
                                    .map(|component| RepairWorkTarget::Component {
                                        system: system.clone(),
                                        component: component.clone(),
                                    })
                                    .collect(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for (source, teams) in set.p2().iter() {
        let saved = teams.0.job_state();
        for job in saved
            .teams
            .iter()
            .flatten()
            .filter(|job| job.outcome.is_none())
        {
            if let crate::jobs::repair::RepairActivity::Refit { target, phase, .. } = &job.activity
            {
                if !matches!(phase, crate::core::messages::TeamSlot::Returning { .. }) {
                    for component in refit_components
                        .get(&(source.0.clone(), target.clone()))
                        .into_iter()
                        .flatten()
                    {
                        claims
                            .entry((source.0.clone(), component.clone()))
                            .or_default()
                            .insert((source.0.clone(), job.id.0));
                    }
                }
            }
        }
        for (id, work) in &saved.physical_work {
            let active = saved
                .teams
                .iter()
                .flatten()
                .any(|job| job.id.0 == *id && job.outcome.is_none());
            if active {
                if let Some(target) = &work.target {
                    claims
                        .entry((source.0.clone(), target.clone()))
                        .or_default()
                        .insert((source.0.clone(), *id));
                }
            }
        }
        for (id, work) in &saved.external_physical_work {
            let active=saved.teams.iter().flatten().any(|job|job.id.0==*id && job.outcome.is_none() &&
                matches!(&job.activity,crate::jobs::repair::RepairActivity::External{target} if target==&work.target_uuid));
            if active {
                if let Some(target) = &work.target {
                    claims
                        .entry((work.target_uuid.clone(), target.clone()))
                        .or_default()
                        .insert((source.0.clone(), *id));
                }
            }
        }
    }
    for (actor, recovery) in set.p3().iter() {
        if let Some(job) = recovery.job.as_ref().filter(|job| job.outcome.is_none()) {
            claims
                .entry((
                    job.activity.target_uuid.clone(),
                    RepairWorkTarget::Component {
                        system: job.activity.source_system.clone(),
                        component: job.activity.source_component.clone(),
                    },
                ))
                .or_default()
                .insert((actor.0.clone(), job.id.0));
        }
    }
    let mut adjustments = Vec::new();
    for mut row in rows {
        row.teams.cleanup_external_physical(
            |uuid| physical_targets.contains(uuid),
            |claim| {
                if let Some(inventory) = &mut row.inventory {
                    inventory.0.release_repair_claim(claim);
                }
            },
        );
        let before_target = row.teams.external_target().map(str::to_owned);
        let mut flags = Vec::new();
        let mut terminal_reason = TaskTerminalReason::Completed;
        if let Some(target_uuid) = before_target.as_deref() {
            let target_entity = set
                .p1()
                .iter()
                .find(|(_, uuid, _, _)| uuid.0 == target_uuid)
                .map(|(entity, _, _, _)| entity);
            if let Some(target_entity) = target_entity {
                let mut targets = set.p1();
                if let Ok((_, _, damage, condition)) = targets.get_mut(target_entity) {
                    if let Some(mut damage) = damage {
                        // Physical self-repair must use internal routes, not an
                        // external dispatch shortcut around compartment access.
                        if row.uuid.as_ref().is_some_and(|uuid| uuid.0 == target_uuid)
                            || damage.catastrophic()
                        {
                            terminal_reason = if damage.catastrophic() {
                                TaskTerminalReason::TargetDestroyed
                            } else {
                                TaskTerminalReason::NotCapable
                            };
                            row.teams.release_external();
                        } else if let Some(source) = &row.uuid {
                            let job_id = row
                                .teams
                                .external_team()
                                .and_then(|team| {
                                    row.teams.job_state().teams[team]
                                        .as_ref()
                                        .map(|job| job.id.0)
                                })
                                .expect("external job");
                            let owner = (source.0.clone(), job_id);
                            let blocked: BTreeSet<_> = claims
                                .iter()
                                .filter(|((uuid, _), owners)| {
                                    uuid == target_uuid
                                        && owners.iter().any(|other| other != &owner)
                                })
                                .map(|((_, target), _)| target.clone())
                                .collect();
                            let before = damage.structural_totals().0;
                            let crate::ship::compartment_damage::ShipDamage {
                                definition,
                                state,
                                ..
                            } = &mut *damage;
                            row.teams.tick_external_physical(
                                dt,
                                target_uuid,
                                definition,
                                state,
                                &blocked,
                                |action, claim, recipe| {
                                    if action == RepairStockAction::Release {
                                        if let Some(stock) = &mut row.inventory {
                                            stock.0.release_repair_claim(claim);
                                        }
                                        return true;
                                    }
                                    let Some(recipe) = recipe else {
                                        return false;
                                    };
                                    if recipe.requirements.is_empty() {
                                        return true;
                                    }
                                    let (Some(stock), Some(services)) =
                                        (&mut row.inventory, &row.services)
                                    else {
                                        return false;
                                    };
                                    match stock.0.repair_parts(action, claim, recipe, &services.0) {
                                        Ok(changes) => {
                                            flags.extend(changes);
                                            true
                                        }
                                        Err(_) => false,
                                    }
                                },
                                |owner, work_dt| {
                                    if !row.tools_available || row.operation <= 0.0 {
                                        return 0.0;
                                    }
                                    row.power.as_mut().map_or(0.0, |power| {
                                        power
                                            .0
                                            .work_continuous(owner, work_dt, 1.0)
                                            .map_or(0.0, |grant| grant.fraction * row.operation)
                                    })
                                },
                            );
                            let restored = damage.structural_totals().0 - before;
                            if restored > 0.0 {
                                if let Some(events) = balance.as_mut() {
                                    events.write(
                                        crate::core::balance::BalanceEvent::RepairApplied {
                                            ship: target_uuid.into(),
                                            hp: restored as f32,
                                        },
                                    );
                                }
                            }
                            // Replace this operator's logical claim with its
                            // newly selected stage before another crew is served.
                            for owners in claims.values_mut() {
                                owners.remove(&owner);
                            }
                            if let Some(work) =
                                row.teams.job_state().external_physical_work.get(&job_id)
                            {
                                if let Some(target) = &work.target {
                                    claims
                                        .entry((target_uuid.into(), target.clone()))
                                        .or_default()
                                        .insert(owner);
                                }
                            }
                        }
                    } else if let Some(condition) = condition {
                        // Non-equipped legacy condition tracks keep their one
                        // existing adjustment owner; physical actors never heal it.
                        if row.teams.step_external()
                            && condition.0.condition() < condition.0.condition_max()
                            && row.config.repair_rate > 0.0
                        {
                            let work_dt = dt.min(
                                (condition.0.condition_max() - condition.0.condition())
                                    / row.config.repair_rate,
                            );
                            let funded = if row.tools_available && row.operation > 0.0 {
                                row.power.as_mut().map_or(0.0, |power| {
                                    power
                                        .0
                                        .work_continuous(row.teams.owner(), work_dt, 1.0)
                                        .map_or(0.0, |grant| grant.fraction * row.operation)
                                })
                            } else {
                                0.0
                            };
                            let delta = row.config.repair_rate * work_dt * funded;
                            if delta > 0.0 {
                                adjustments.push(ConditionAdjustment {
                                    uuid: target_uuid.into(),
                                    delta,
                                });
                            }
                        }
                    }
                }
            }
        }
        // Invalid external physical targets close the same lifecycle identity
        // as ordinary recall/range maintenance. Completed stages stay aboard.
        if before_target.is_some() && row.teams.external_target().is_none() {
            push_lifecycle(
                lifecycle.as_deref_mut(),
                row.uuid.as_ref(),
                TaskLifecycleRequest::End {
                    slot: dispatch_slot(row.uuid.as_ref()),
                    reason: terminal_reason,
                },
            );
        }
        row.teams.cleanup_external_physical(
            |uuid| physical_targets.contains(uuid),
            |claim| {
                if let Some(inventory) = &mut row.inventory {
                    inventory.0.release_repair_claim(claim);
                }
            },
        );
        if let (Some(runtime), Some(source)) = (runtime.as_deref_mut(), row.uuid.as_ref()) {
            crate::infrastructure::server::mirror_flags(runtime, &flags, &source.0, &log);
        }
        if let Ok((_, _, _, mut teams, power, inventory, _, _, _)) = set.p0().get_mut(row.entity) {
            teams.0 = row.teams;
            if let (Some(mut current), Some(next)) = (power, row.power) {
                *current = next;
            }
            if let (Some(mut current), Some(next)) = (inventory, row.inventory) {
                *current = next;
            }
        }
    }
    adjustments.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    if let Some(queue) = condition_queue.as_mut() {
        queue.0.extend(adjustments);
    }
}

/// Marks a ship whose external repair team the backfill host DISPATCHED to serve
/// a `FieldRepair` directive (issue #1162). Inserted on dispatch, removed on
/// recall; the host recalls only while it is present, so it never recalls a team
/// a console dispatched on the same AI-operated system. Not folded/snapshotted:
/// re-adopted from the still-present directive on resume.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct ExternalRepairAiDispatched;

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
/// (`RepairTeams::free_team_indices_reserving`) the standing external claim
/// already eats from. It never dispatches a team the local damage-control sweep
/// needs to bring a knocked-out system back. The claim NAMES its team (issue
/// #1386) while the reserve stays a count, which is the honest shape of each: one
/// says who is out, the other says how many to keep spare. Because the
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
        _dispatch,
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
        let directive_target: Option<String> = match config.and_then(|config| {
            config
                .0
                .observations_for_system(&repair_system_id(), &blackboards.0)
                .get(&crate::ship::system_registry::viewscreen_system_id())
        }) {
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
                let has_free_team = teams
                    .map(|t| {
                        !t.0.free_team_indices_reserving(
                            t.0.external_team(),
                            critical_local_repairs(repair_queue),
                        )
                        .is_empty()
                    })
                    .unwrap_or(false);
                // Dispatch once the ordered ally is locked and a team is free
                // beyond every other claim. Idempotent — a team already working
                // this target emits nothing.
                let already_here =
                    teams.and_then(|t| t.0.external_target()) == Some(resolved.as_str());
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
                if teams.and_then(|t| t.0.external_target()).is_some() && host_dispatched {
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
                .in_set(crate::sim_sets::FixedStep::OperateExternalRepairAi)
                .in_set(crate::sim_sets::SimSet::Input)
                .run_if(crate::ai::cadence::ai_tick_ready)
                .before(handle_external_repair_commands),
            handle_external_repair_commands
                .in_set(crate::sim_sets::FixedStep::HandleExternalRepairCommands)
                .in_set(crate::sim_sets::SimSet::Input),
            tick_external_repair
                .in_set(crate::sim_sets::FixedStep::TickExternalRepair)
                .in_set(crate::sim_sets::SimSet::Modifiers)
                // Coordination projects the physical onsite view first; external
                // job maintenance and its gated effect then advance the shared owner.
                .after(crate::ship::coordination_systems::process_coordination_lag),
            apply_external_repair
                .in_set(crate::sim_sets::FixedStep::ApplyExternalRepair)
                .in_set(crate::sim_sets::SimSet::Modifiers)
                .after(tick_external_repair)
                .before(crate::infrastructure::tick_infrastructure_condition),
        ),
    );
}

#[cfg(test)]
#[path = "external_server_tests.rs"]
mod tests;
