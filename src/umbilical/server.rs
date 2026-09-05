//! The Bevy adapter for the transfer umbilical (issue #1160).
//!
//! Gathers the live world into the plain values the pure sibling
//! [`crate::umbilical::flow`] takes — both docked ends' capacity ledgers, the
//! umbilical's power level and damage tier, and #1159's docked state — and
//! applies what comes back: the per-ship [`TransferUmbilical`] component, the
//! fixed-tick systems that take the start/stop commands, move an authored
//! capacity per second between the two docked hulls' ledgers, and publish the
//! blackboard. Nothing here decides the arithmetic itself: rule 10, the split
//! the tractor and dock keep between their pure module and their server.
//!
//! # It flows only while docked, and moves the capacity through the queue
//!
//! The umbilical is the third slice of PRD #1143's coupling family, and it
//! *gates on the second*: a flow runs only while the umbilical's own hull is
//! docked ([`crate::dock::DockControl::docked_partner`]), so resupply requires
//! Helm to have achieved the dock first — two seats on two ships, or two seats on
//! one. The capacity itself moves through the SAME queue the arrest (#1158) and
//! the operations `transfer` use — a [`CapacityAdjustment`] on each end drained
//! by [`crate::infrastructure::tick_infrastructure_condition`] — so the moved
//! goods cross the docked partner's own authored ceiling and re-publish onto the
//! counter a scenario predicate reads, rather than being written onto the
//! component behind the one system that owns those numbers.

use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
use crate::core::messages::{
    PowerGroupId, SystemAffinity, SystemBlackboard, SystemControlPayload, SystemId,
    UmbilicalBlackboard,
};
use crate::core::task_lifecycle::{
    TaskLifecycleRequest, TaskSlot, TaskTerminalReason, TASK_VERB_UMBILICAL_FLOW,
};
use crate::dock::DockControl;
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::{EntitySystemHull, EntityUuid};
use crate::infrastructure::{CapacityAdjustment, InfrastructureCondition};
use crate::ship::damage::DamageTier;
use crate::ship::power::{power_level_for, ShipPowerSystem};
use crate::ship::system_registry::{umbilical_system_id, UMBILICAL_SYSTEM_ID};
use crate::umbilical::flow::{
    plan_flow, CapacityEnd, FlowContext, FlowEnds, FlowVerdict, UmbilicalConfig,
    UmbilicalDirection, UmbilicalRefusal,
};

/// One ship's transfer umbilical (issue #1160): the authored flow terms, the
/// power group it draws from, and the live run state.
///
/// Inserted at spawn only on a hull that authored an `[umbilical]` table AND a
/// `kind = "umbilical"` `[[system]]` — a hull with neither carries no component
/// and is byte-identical in every way to one built before this existed
/// (AGENTS.md rule 11).
///
/// `running` is the operator's INTENT to flow — set by `StartTransfer`, cleared
/// by `StopTransfer` and by every interruption (undock, power loss, damage,
/// partner without the capacity). It is the one field that is folded and
/// snapshotted. `carry` is the sub-unit remainder the flow arithmetic meters a
/// per-second rate through; `last_refusal` is why the flow last stopped; and
/// `operator_level`/`partner_level` are this tick's ledger readings for the
/// console. All four are projections the next tick re-derives — not folded, not
/// snapshotted — exactly as the dock leaves its `available_target`/`undock_target`
/// out.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TransferUmbilical {
    /// The authored flow terms — capacity id, rate, direction, minimum power
    /// level.
    pub config: UmbilicalConfig,
    /// The operator's standing intent to flow. Set true by `StartTransfer`,
    /// false by `StopTransfer` and by every interruption.
    pub running: bool,
    /// The power group the umbilical `[[system]]` declared, resolved once at
    /// spawn so the tick never re-walks the authored systems list. The single
    /// authored source is the `[[system]] power_group` field.
    pub power_group: PowerGroupId,
    /// The sub-unit remainder the flow arithmetic carries between ticks so a
    /// per-second rate finer than one unit per tick still moves at the authored
    /// rate. A live projection: reset on start/stop and on restore, never folded
    /// or saved.
    pub carry: f32,
    /// Why the last flow could not start or was stopped — the reason the console
    /// shows, retained until the operator acts again. `None` while the flow is
    /// idle or running cleanly.
    pub last_refusal: Option<UmbilicalRefusal>,
    /// This tick's operator-end level for the console, or `None` when the
    /// operator carries no such capacity. A projection, not saved.
    pub operator_level: Option<i64>,
    /// This tick's partner-end level for the console, or `None` when there is no
    /// docked partner or it carries no such capacity. A projection, not saved.
    pub partner_level: Option<i64>,
    /// The docked-partner uuid a live task-lifecycle activation (issue #1345) is
    /// currently open against, `None` while idle or refused. Set the moment
    /// `tick_umbilical` sees the flow actually move (or fail to, on the very
    /// first attempt), so a later refusal or stop knows whether an activation is
    /// there to close — the umbilical's analogue of the tractor's
    /// `coupled_target`. A projection `tick_umbilical` re-derives every tick, not
    /// the running intent itself: never folded, never saved, and reset on
    /// restore, so a resumed flow simply opens a fresh activation the next time
    /// it moves anything.
    pub activation_target: Option<String>,
}

impl TransferUmbilical {
    /// A fresh, idle umbilical carrying its authored terms and resolved power
    /// group.
    pub fn new(config: UmbilicalConfig, power_group: PowerGroupId) -> Self {
        Self {
            config,
            power_group,
            running: false,
            carry: 0.0,
            last_refusal: None,
            operator_level: None,
            partner_level: None,
            activation_target: None,
        }
    }

    /// The persistable half — the running intent — for the snapshot payload. The
    /// authored config and power group ride the template and are re-derived on
    /// spawn, exactly as the tractor leaves its coupling terms out of
    /// `TractorSaveState`. The carry, the refusal and the level projections are
    /// deliberately left out: all are re-derived on the first resumed tick.
    pub fn save_state(&self) -> UmbilicalSaveState {
        UmbilicalSaveState {
            running: self.running,
        }
    }

    /// Reseed the running intent from a restored snapshot, onto an umbilical that
    /// already carries its authored config and resolved power group from the
    /// fresh spawn. The carry, the last refusal and the level projections are NOT
    /// restored — the next `tick_umbilical` re-derives them from the resumed
    /// world, so a stored one would only ever be stale.
    pub fn restore(&mut self, save: &UmbilicalSaveState) {
        self.running = save.running;
        self.carry = 0.0;
        self.last_refusal = None;
        self.operator_level = None;
        self.partner_level = None;
        self.activation_target = None;
    }
}

/// The snapshot-carried half of a [`TransferUmbilical`] (issue #1160): the
/// running intent, and nothing else.
///
/// `Default` is the idle umbilical — not running — which is what a hull that
/// authored an umbilical and never started it captures, so a resume of such a
/// ship restores byte-identically and folds the same number.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UmbilicalSaveState {
    #[serde(default)]
    pub running: bool,
}

/// Transient correlation carried from the start intent to the same tick's
/// authoritative flow verdict.
#[derive(Message, Clone)]
pub struct PendingUmbilicalActionFeedback {
    entity: Entity,
    command: crate::core::messages::AdmittedCommand,
}

/// Registers the umbilical systems and its admitted-command consumer (issue
/// #1160). Added by `WorldPlugin` alongside `DockPlugin`.
pub struct UmbilicalPlugin;

impl Plugin for UmbilicalPlugin {
    fn build(&self, app: &mut App) {
        // The umbilical `[[system]]` is an admitted-command consumer:
        // `handle_umbilical_commands` reads `StartTransfer` / `StopTransfer` for
        // it, so admission fans those commands into every ship's
        // `AdmittedCommands` each tick and the end-of-frame lint never warns them
        // unrouted.
        // Gated AI decider (issue #1162); `register_ai_cadence` is idempotent.
        crate::ai::cadence::register_ai_cadence(app);
        // Authoritative-state exclusion declaration (issue #1221, Track 3 step C9).
        // `UmbilicalAiRunning` is the DERIVED "I am driving this umbilical" marker —
        // re-derived every AI tick from the still-folded directive plus the folded
        // umbilical-flow state, never a second copy of either, so a lost marker
        // self-heals within one AI tick. Declared here at its owning site,
        // replacing the `EXCLUSIONS` const in
        // `tests/authoritative_state_enumeration.rs`; inert to the digest.
        {
            use crate::authoritative::{DeclareState, StateClass};
            app.declare_state::<UmbilicalAiRunning>(StateClass::Derived, "umbilical-flow-state");
        }
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::UMBILICAL_KIND,
            UMBILICAL_SYSTEM_ID,
        ))
        .add_message::<PendingUmbilicalActionFeedback>();
        app.add_systems(
            FixedUpdate,
            (
                // Backfill Engineering umbilical AI (issue #1162): on the shared
                // AI cadence (rule 7), emitting StartTransfer / StopTransfer
                // BEFORE `handle_umbilical_commands` consumes the tick.
                operate_umbilical_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .run_if(crate::ai::cadence::ai_tick_ready)
                    .before(handle_umbilical_commands),
                handle_umbilical_commands.in_set(crate::sim_sets::SimSet::Input),
                // Move the capacity this tick — after the dock tick that decides
                // the docked state this gates on, and BEFORE the infrastructure
                // tick that applies the queued moves, the same ordering the
                // tractor's arrest keeps so the flow lands the same tick it is
                // decided.
                tick_umbilical
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(crate::dock::server::tick_dock)
                    .before(crate::infrastructure::tick_infrastructure_condition),
                finish_umbilical_action_feedback
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(tick_umbilical),
                publish_umbilical_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

// ── The start / stop commands ────────────────────────────────────────────────

/// Runs in `SimSet::Input` and reads `AdmittedCommands` for the umbilical system
/// (issue #1160). It sets only the operator's INTENT; `tick_umbilical` decides
/// whether the flow actually runs and records any refusal, so a start that
/// cannot flow is refused within the same tick before the blackboard publishes.
///
/// Human and AI reach this identically: admission has already decided who may
/// speak (an Engineering tenure token at the network gate, or #1162's umbilical
/// AI through the same `validate_and_admit` seam) and stripped the source, so
/// nothing here asks who sent the command (AGENTS.md rule 6).
pub fn handle_umbilical_commands(
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut ships: Query<(
        Entity,
        &crate::core::messages::AdmittedCommands,
        &mut TransferUmbilical,
        Option<&EntityUuid>,
    )>,
    mut pending: Option<ResMut<Messages<PendingUmbilicalActionFeedback>>>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    for (entity, admitted, mut umbilical, uuid) in ships.iter_mut() {
        for cmd in admitted.for_target(UMBILICAL_SYSTEM_ID) {
            match &cmd.payload {
                SystemControlPayload::StartTransfer => {
                    if !umbilical.running {
                        umbilical.running = true;
                        // Clear a stale refusal on a fresh start; the tick will
                        // repopulate it if this start cannot flow.
                        umbilical.last_refusal = None;
                        umbilical.carry = 0.0;
                    }
                    if cmd.response_token.is_some() && cmd.feedback_correlation.is_some() {
                        if let Some(messages) = pending.as_deref_mut() {
                            messages.write(PendingUmbilicalActionFeedback {
                                entity,
                                command: cmd.clone(),
                            });
                        }
                    }
                }
                SystemControlPayload::StopTransfer => {
                    // Report the cancel BEFORE clearing intent, so a stop of an
                    // idle umbilical (a stale-UI double tap) reports nothing at
                    // all (issue #1345). Gated on the standing intent, exactly
                    // as the tractor's release is — a stop that never actually
                    // flowed finds nothing live in the registry and this report
                    // is dropped harmlessly.
                    if umbilical.running {
                        if let Some(uuid) = uuid {
                            push_lifecycle(
                                lifecycle.as_deref_mut(),
                                TaskLifecycleRequest::End {
                                    slot: flow_slot(&uuid.0),
                                    reason: TaskTerminalReason::Released,
                                },
                            );
                        }
                    }
                    umbilical.running = false;
                    umbilical.last_refusal = None;
                    umbilical.carry = 0.0;
                    umbilical.activation_target = None;
                    crate::command_admission::finish_admitted_action_feedback(
                        &mut outbound,
                        cmd,
                        crate::core::messages::ActionFeedbackOutcome::Applied,
                    );
                }
                _ => {}
            }
        }
    }
}

/// The lifecycle slot a hull's umbilical flow occupies (issue #1345) — its
/// uuid, the umbilical system, and the flow verb.
fn flow_slot(uuid: &str) -> TaskSlot {
    TaskSlot::new(uuid, UMBILICAL_SYSTEM_ID, TASK_VERB_UMBILICAL_FLOW)
}

/// Queue one lifecycle report, if there is a queue (issue #1345). `Option` so a
/// reduced fixture that runs these systems without the narrative plugin behaves
/// exactly as it did before this existed — the same guard Security's `push_
/// lifecycle` keeps.
fn push_lifecycle(
    queue: Option<&mut EffectQueue<TaskLifecycleRequest>>,
    request: TaskLifecycleRequest,
) {
    if let Some(queue) = queue {
        queue.0.push(request);
    }
}

/// The terminal reason a dropped (or never-formed) flow reports, from the
/// refusal the pure flow module returned (issue #1345).
///
/// `Undocked` is the connection breaking — the dock the flow depends on let go
/// — so it reads as `TargetLost`, the same reason the tractor gives a hold whose
/// lock went away. `Unpowered`/`Disabled` are the flow's own preconditions
/// failing. `NoCapacity` — one or both docked ends declare no capacity under the
/// authored id at all — is a structural incapability of the pairing, never a
/// depleted source (`plan_flow`'s arithmetic clamps a depleted source or a full
/// destination to moving nothing THIS tick without refusing; the flow keeps
/// running), so it reads as `NotCapable` rather than `Completed`.
fn terminal_reason_for(refusal: UmbilicalRefusal) -> TaskTerminalReason {
    match refusal {
        UmbilicalRefusal::Undocked => TaskTerminalReason::TargetLost,
        UmbilicalRefusal::Unpowered => TaskTerminalReason::Unpowered,
        UmbilicalRefusal::Disabled => TaskTerminalReason::Disabled,
        UmbilicalRefusal::NoCapacity => TaskTerminalReason::NotCapable,
    }
}

/// Complete start feedback only after the live dock, capacity, power and damage
/// gates have resolved in `tick_umbilical`.
fn finish_umbilical_action_feedback(
    mut pending: MessageReader<PendingUmbilicalActionFeedback>,
    umbilicals: Query<(&TransferUmbilical, &DockControl, &EntityUuid)>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    for action in pending.read() {
        let applied = umbilicals
            .get(action.entity)
            .is_ok_and(|(umbilical, _, _)| umbilical.running);
        crate::command_admission::finish_admitted_action_feedback(
            &mut outbound,
            &action.command,
            if applied {
                crate::core::messages::ActionFeedbackOutcome::Applied
            } else {
                crate::core::messages::ActionFeedbackOutcome::Refused
            },
        );
    }
}

/// Marks a ship whose umbilical the backfill host is RUNNING to serve a
/// `Transfer` directive (issue #1162). Inserted when the host starts the flow,
/// removed when it stops; the host stops a flow only while it is present, so it
/// never stops a flow a console started on the same AI-operated system. Not
/// folded/snapshotted: re-adopted from the still-present directive on resume.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct UmbilicalAiRunning;

/// Backfill Engineering umbilical AI (issue #1162).
///
/// On an active `Transfer` directive (Engineering affinity) once the hull is
/// DOCKED, start the umbilical; with no such directive active, stop it. The
/// concrete command is exactly the `StartTransfer`/`StopTransfer` a human
/// Engineering officer emits, sent through the SAME `emit_ai_command` seam, so
/// `handle_umbilical_commands` never learns who spoke (AGENTS.md rule 6).
///
/// Gated on the dock the Helm dock host (issue #1162) achieves: resupply is a
/// chain no single seat completes, so the umbilical waits for
/// `DockControl::docked_partner` exactly as a human Engineering officer waits for
/// Helm to call the dock made. Decides ONLY on the shared AI cadence (rule 7),
/// via `run_if(ai_tick_ready)`.
#[allow(clippy::type_complexity)]
pub fn operate_umbilical_ai(
    mut commands: Commands,
    sessions: Res<crate::lobby::Sessions>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut ships: Query<(
        Entity,
        Option<&EntityUuid>,
        &crate::ship_plugin::ShipSystemControlSources,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        &TransferUmbilical,
        &DockControl,
        &crate::server_app::ShipSystemBlackboards,
        Has<UmbilicalAiRunning>,
        &mut crate::core::messages::AdmittedCommands,
    )>,
) {
    for (entity, uuid, sources, config, umbilical, dock, blackboards, host_running, mut admitted) in
        ships.iter_mut()
    {
        if !sources.0.policy_for(&umbilical_system_id()).operate_ai {
            continue;
        }
        let transfer_active = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(vbb)) => crate::objectives::top_operate_directive(
                &vbb.scored_objectives,
                SystemAffinity::Engineering,
                |d| crate::objectives::transfer_directive_target(d).is_some(),
            )
            .is_some(),
            _ => false,
        };

        let payload = if transfer_active {
            // Start once the dock is made (the umbilical only flows while docked),
            // idempotent once running. Claim the flow as host-driven while it runs
            // under this order.
            let emit = (dock.docked_partner().is_some() && !umbilical.running)
                .then_some(SystemControlPayload::StartTransfer);
            if (emit.is_some() || umbilical.running) && !host_running {
                commands.entity(entity).insert(UmbilicalAiRunning);
            }
            emit
        } else if umbilical.running && host_running {
            // No transfer order: stop a flow THIS HOST started — never one a
            // console started on the same AI-operated system.
            commands.entity(entity).remove::<UmbilicalAiRunning>();
            // The scenario closing the task, not the operator changing their
            // mind (issue #1345). Reported HERE, ahead of the `StopTransfer`
            // this emits and this system's own ordering ahead of `handle_
            // umbilical_commands`, so the withdrawn order is the terminal the
            // timeline records.
            if let Some(uuid) = uuid {
                push_lifecycle(
                    lifecycle.as_deref_mut(),
                    TaskLifecycleRequest::End {
                        slot: flow_slot(&uuid.0),
                        reason: TaskTerminalReason::OrderWithdrawn,
                    },
                );
            }
            Some(SystemControlPayload::StopTransfer)
        } else {
            None
        };

        if let Some(payload) = payload {
            emit_ai_command(
                uuid,
                umbilical_system_id(),
                payload,
                sources,
                &sessions,
                config,
                &mut admitted,
            );
        }
    }
}

// ── The flow tick ────────────────────────────────────────────────────────────

/// What one operator's tick decided, applied in the write phase.
struct Outcome {
    entity: Entity,
    running: bool,
    carry: f32,
    last_refusal: Option<UmbilicalRefusal>,
    operator_level: Option<i64>,
    partner_level: Option<i64>,
    /// The task-lifecycle activation's subject once this tick's verdict is
    /// applied (issue #1345) — the mirror of the tractor's `coupled_target`.
    activation_target: Option<String>,
    /// The two capacity moves to queue this tick:
    /// `(capacity_id, operator_uuid, partner_uuid, operator_delta, partner_delta)`.
    /// `None` on a tick that moved nothing.
    queue: Option<(String, String, String, i64, i64)>,
}

/// One operator read once per tick into plain values so the borrow of the world
/// is released before the capacity readings and the write.
struct OperatorRow {
    entity: Entity,
    uuid: String,
    running: bool,
    partner: Option<String>,
    power_level: u8,
    disabled: bool,
    config: UmbilicalConfig,
    carry: f32,
    /// The task-lifecycle activation this slot has open going into this tick
    /// (issue #1345), or `None` while idle or refused.
    activation_target: Option<String>,
}

/// Move each running umbilical's authored capacity between the two docked hulls'
/// ledgers this tick (issue #1160).
///
/// Reads the live world into the scalars the pure [`plan_flow`] takes — the
/// docked partner (#1159's `docked_partner`), the umbilical's power level and
/// damage tier, both docked ends' capacity ledgers for the authored id, and the
/// stored carry — and applies the verdict. On a refusal the running intent clears
/// and the reason is retained; on a flow the two [`CapacityAdjustment`]s are
/// queued for the infrastructure tick. The operator's own level and the partner's
/// are projected onto the component every tick, running or not, so the console
/// shows both ends' levels the moment a berth is in reach.
#[allow(clippy::type_complexity)]
pub fn tick_umbilical(
    // The capacity-adjustment queue, extracted off `WorldContentRuntime` (issue
    // #1223) and owned by `InfrastructurePlugin`; this pusher feeds it. `Option`
    // so a reduced test app that runs the umbilical systems without
    // `InfrastructurePlugin` is a no-op rather than a panic.
    capacity_queue: Option<ResMut<EffectQueue<CapacityAdjustment>>>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    time: Res<Time>,
    mut set: ParamSet<(
        // Operator rows: everything the verdict needs off the umbilical's own hull.
        Query<(
            Entity,
            &EntityUuid,
            &TransferUmbilical,
            &DockControl,
            Option<&ShipPowerSystem>,
            Option<&EntitySystemHull>,
        )>,
        // Every entity's capacity ledger, to resolve both docked ends.
        Query<(&EntityUuid, &InfrastructureCondition)>,
        // Apply the verdict and the projections.
        Query<&mut TransferUmbilical>,
    )>,
) {
    let dt = time.delta_secs();

    // Gather the operator inputs first, so the ledger lookup and the write can
    // each take the world without holding the other's borrow.
    let rows: Vec<OperatorRow> = set
        .p0()
        .iter()
        .map(|(entity, uuid, umbilical, dock, power, hull)| {
            let power_level = power
                .map(|p| power_level_for(&p.0, &umbilical.power_group))
                .unwrap_or(0);
            let disabled = hull
                .map(|h| {
                    matches!(
                        h.0.tier_for(&umbilical_system_id()),
                        DamageTier::Disabled | DamageTier::Destroyed
                    )
                })
                .unwrap_or(false);
            OperatorRow {
                entity,
                uuid: uuid.0.clone(),
                running: umbilical.running,
                partner: dock.docked_partner().map(|s| s.to_string()),
                power_level,
                disabled,
                config: umbilical.config.clone(),
                carry: umbilical.carry,
                activation_target: umbilical.activation_target.clone(),
            }
        })
        .collect();
    if rows.is_empty() {
        return;
    }

    // Resolve both ends' capacity readings for each operator's authored id.
    let read_end = |conditions: &Query<(&EntityUuid, &InfrastructureCondition)>,
                    uuid: &str,
                    capacity: &str|
     -> Option<CapacityEnd> {
        conditions
            .iter()
            .find(|(id, _)| id.0 == uuid)
            .and_then(|(_, condition)| condition.0.capacity_reading(capacity))
            .map(|reading| CapacityEnd {
                level: reading.level,
                headroom: reading.headroom(),
            })
    };

    let mut outcomes: Vec<Outcome> = Vec::with_capacity(rows.len());
    {
        let conditions = set.p1();
        for row in &rows {
            let operator_end = read_end(&conditions, &row.uuid, &row.config.capacity);
            let partner_end = row
                .partner
                .as_deref()
                .and_then(|p| read_end(&conditions, p, &row.config.capacity));
            let operator_level = operator_end.map(|e| e.level);
            let partner_level = partner_end.map(|e| e.level);

            // Not running: keep the projections fresh, keep any retained refusal,
            // move nothing. No activation is open — a stop already closed it in
            // `handle_umbilical_commands`.
            if !row.running {
                outcomes.push(Outcome {
                    entity: row.entity,
                    running: false,
                    carry: 0.0,
                    last_refusal: None, // retained on the component by leaving it; see apply
                    operator_level,
                    partner_level,
                    activation_target: None,
                    queue: None,
                });
                continue;
            }

            let ctx = FlowContext {
                docked: row.partner.is_some(),
                powered: row.power_level >= row.config.min_power_level,
                disabled: row.disabled,
                dt,
                carry: row.carry,
            };
            let ends = FlowEnds {
                operator: operator_end,
                partner: partner_end,
            };
            match plan_flow(&row.config, &ends, &ctx) {
                FlowVerdict::Refused(refusal) => {
                    // The task activation follows the flow actually moving
                    // capacity, never the standing intent (issue #1345) —
                    // mirroring `tick_tractor`'s "the activation follows the
                    // coupling" rule. A start that never once flowed still gets
                    // a whole activation, opened and closed together here, so
                    // the attempt is a beat rather than a silence — the same
                    // "an engage refused before it couples still opens and
                    // closes one activation" the tractor keeps. A flow that WAS
                    // moving capacity and just broke only needs the close.
                    if row.activation_target.is_none() {
                        push_lifecycle(
                            lifecycle.as_deref_mut(),
                            TaskLifecycleRequest::Start {
                                slot: flow_slot(&row.uuid),
                                target: row.partner.clone(),
                            },
                        );
                    }
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        TaskLifecycleRequest::End {
                            slot: flow_slot(&row.uuid),
                            reason: terminal_reason_for(refusal),
                        },
                    );
                    outcomes.push(Outcome {
                        entity: row.entity,
                        running: false,
                        carry: 0.0,
                        last_refusal: Some(refusal),
                        operator_level,
                        partner_level,
                        activation_target: None,
                        queue: None,
                    });
                }
                FlowVerdict::Flowing {
                    operator_delta,
                    partner_delta,
                    carry,
                } => {
                    // Flowing always implies a docked partner (the `docked` gate
                    // above is the only way to reach here), so `row.partner` is
                    // always `Some`. Open the activation the moment it starts
                    // moving anything, and re-open it if the partner ever
                    // changed under a live flow — unreachable today (a partner
                    // change requires an undock, which refuses the flow first),
                    // but kept symmetric with the tractor's own re-designation
                    // handling rather than assumed away.
                    if row.activation_target.as_deref() != row.partner.as_deref() {
                        if row.activation_target.is_some() {
                            push_lifecycle(
                                lifecycle.as_deref_mut(),
                                TaskLifecycleRequest::End {
                                    slot: flow_slot(&row.uuid),
                                    reason: TaskTerminalReason::TargetLost,
                                },
                            );
                        }
                        push_lifecycle(
                            lifecycle.as_deref_mut(),
                            TaskLifecycleRequest::Start {
                                slot: flow_slot(&row.uuid),
                                target: row.partner.clone(),
                            },
                        );
                    }
                    // A move only reaches the queue when it is non-zero and there
                    // is a partner to move it to — a depleted source or a full
                    // destination keeps the flow running but queues nothing.
                    let queue = match (&row.partner, operator_delta) {
                        (Some(partner), delta) if delta != 0 => Some((
                            row.config.capacity.clone(),
                            row.uuid.clone(),
                            partner.clone(),
                            operator_delta,
                            partner_delta,
                        )),
                        _ => None,
                    };
                    outcomes.push(Outcome {
                        entity: row.entity,
                        running: true,
                        carry,
                        last_refusal: None,
                        operator_level,
                        partner_level,
                        activation_target: row.partner.clone(),
                        queue,
                    });
                }
            }
        }
    }

    // Collect the capacity moves in UUID order, so two hosts queue identically —
    // the walk-order rule the infrastructure and operations ticks both keep.
    let mut adjustments: Vec<CapacityAdjustment> = Vec::new();
    for out in &outcomes {
        if let Some((capacity, op_uuid, partner_uuid, op_delta, partner_delta)) = &out.queue {
            adjustments.push(CapacityAdjustment {
                uuid: op_uuid.clone(),
                capacity: capacity.clone(),
                delta: *op_delta,
            });
            adjustments.push(CapacityAdjustment {
                uuid: partner_uuid.clone(),
                capacity: capacity.clone(),
                delta: *partner_delta,
            });
        }
    }
    adjustments.sort_by(|a, b| a.uuid.cmp(&b.uuid).then(a.capacity.cmp(&b.capacity)));

    // Apply the verdicts and projections.
    {
        let mut umbilicals = set.p2();
        for out in &outcomes {
            let Ok(mut umbilical) = umbilicals.get_mut(out.entity) else {
                continue;
            };
            umbilical.running = out.running;
            umbilical.carry = out.carry;
            umbilical.operator_level = out.operator_level;
            umbilical.partner_level = out.partner_level;
            umbilical.activation_target = out.activation_target.clone();
            // A stop-through-refusal records the reason; an idle tick leaves the
            // retained refusal alone (it clears on the next start/stop command).
            if let Some(refusal) = out.last_refusal {
                umbilical.last_refusal = Some(refusal);
            } else if out.running {
                umbilical.last_refusal = None;
            }
        }
    }

    // Queue the moves only when there is something to move, so a world whose
    // umbilicals are idle never marks the capacity queue changed on a quiet tick.
    if adjustments.is_empty() {
        return;
    }
    let Some(mut capacity_queue) = capacity_queue else {
        return;
    };
    capacity_queue.0.extend(adjustments);
}

// ── The wire ─────────────────────────────────────────────────────────────────

/// Publish each umbilical-carrying ship's blackboard under its system id (issue
/// #1160).
///
/// Only ships that carry [`TransferUmbilical`] publish one, so a world whose
/// hulls author no `[umbilical]` puts exactly the payload on the wire it did
/// before this existed. No English crosses: `capacity` is the authored machine id
/// and `refusal` is the pure module's `strings.csv` id.
pub fn publish_umbilical_blackboard(
    mut ships: Query<(
        &TransferUmbilical,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
) {
    let key = umbilical_system_id();
    for (umbilical, mut blackboards) in ships.iter_mut() {
        let blackboard = SystemBlackboard::Umbilical(UmbilicalBlackboard {
            capacity: umbilical.config.capacity.clone(),
            rate: umbilical.config.rate,
            direction: match umbilical.config.direction {
                UmbilicalDirection::Deliver => "deliver".to_string(),
                UmbilicalDirection::Collect => "collect".to_string(),
            },
            running: umbilical.running,
            operator_level: umbilical.operator_level,
            partner_level: umbilical.partner_level,
            refusal: umbilical.last_refusal.map(|r| r.string_id().to_string()),
        });
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key.clone(), blackboard);
        }
    }
}

/// The umbilical's published blackboard channel key — its system id (issue
/// #1160). A convenience mirror of [`umbilical_system_id`] for readers that key
/// off a function, matching `tractor_blackboard_key`.
pub fn umbilical_blackboard_key() -> SystemId {
    umbilical_system_id()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correlated_umbilical_command(
        correlation: &str,
        payload: SystemControlPayload,
    ) -> crate::core::messages::AdmittedCommand {
        crate::core::messages::AdmittedCommand {
            target: umbilical_system_id(),
            payload,
            response_token: Some("engineering-holder".into()),
            feedback_correlation: Some(
                crate::core::messages::ActionCorrelationId::new(correlation)
                    .expect("valid test correlation"),
            ),
        }
    }

    fn capacity(level: i64, ceiling: i64) -> InfrastructureCondition {
        let config = crate::infrastructure::InfrastructureConfig {
            capacities: vec![crate::infrastructure::CapacityConfig {
                id: "reserve_fuel".into(),
                amount: level,
                label: None,
                ceiling: Some(ceiling),
            }],
            ..Default::default()
        };
        InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
            &config,
        ))
    }

    fn umbilical() -> TransferUmbilical {
        TransferUmbilical::new(
            UmbilicalConfig {
                capacity: "reserve_fuel".into(),
                rate: 5.0,
                direction: UmbilicalDirection::Deliver,
                min_power_level: 2,
            },
            PowerGroupId("umbilical".into()),
        )
    }

    #[test]
    fn umbilical_feedback_waits_for_the_authoritative_flow_verdict() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_message::<PendingUmbilicalActionFeedback>()
            .add_message::<crate::lobby::OutboundMessage>()
            .add_systems(
                Update,
                (
                    handle_umbilical_commands,
                    tick_umbilical,
                    finish_umbilical_action_feedback,
                )
                    .chain(),
            );
        let mut dock = DockControl::new(
            crate::ship::system_registry::dock_system_id(),
            crate::dock::DockConfig {
                range: 100.0,
                engage_distance: 100.0,
                approach_speed: 1.0,
                mate_tolerance: 1.0,
                undock_clear_distance: 1.0,
                min_power_level: 1,
            },
            PowerGroupId("dock".into()),
        );
        dock.engaged = true;
        dock.docked = true;
        dock.docking_target = Some("partner-1".into());
        let power_config = crate::modifiers::power_system::PowerConfig::default();
        let power = ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::from_authored_groups(
                &power_config,
                &[
                    crate::modifiers::power_system::AuthoredPowerGroup::at_default_floor(
                        PowerGroupId("umbilical".into()),
                        2,
                    ),
                ],
            ),
        );
        let operator = app
            .world_mut()
            .spawn((
                crate::core::messages::AdmittedCommands(vec![correlated_umbilical_command(
                    "umbilical-applied",
                    SystemControlPayload::StartTransfer,
                )]),
                umbilical(),
                dock,
                power,
                EntityUuid("operator-1".into()),
                capacity(10, 10),
            ))
            .id();
        app.world_mut()
            .spawn((EntityUuid("partner-1".into()), capacity(0, 10)));
        let mut cursor = app
            .world()
            .resource::<Messages<crate::lobby::OutboundMessage>>()
            .get_cursor();
        let feedback_count = |messages: &[crate::lobby::OutboundMessage], correlation, expected| {
            messages
                .iter()
                .filter(|message| {
                    matches!(
                        (&message.target, &message.msg),
                        (
                            crate::lobby::Target::Token(token),
                            crate::core::messages::ServerMessage::ActionFeedback {
                                correlation: actual,
                                outcome,
                            }
                        ) if token == "engineering-holder"
                            && actual.as_str() == correlation
                            && outcome == &expected
                    )
                })
                .count()
        };

        app.update();
        let messages = app
            .world()
            .resource::<Messages<crate::lobby::OutboundMessage>>();
        let first: Vec<_> = cursor.read(messages).cloned().collect();
        assert_eq!(
            feedback_count(
                &first,
                "umbilical-applied",
                crate::core::messages::ActionFeedbackOutcome::Applied,
            ),
            1
        );
        assert!(app
            .world()
            .get::<TransferUmbilical>(operator)
            .is_some_and(|umbilical| umbilical.running));

        app.world_mut()
            .entity_mut(operator)
            .insert(crate::core::messages::AdmittedCommands(vec![
                correlated_umbilical_command("umbilical-stop", SystemControlPayload::StopTransfer),
            ]));
        app.update();
        let messages = app
            .world()
            .resource::<Messages<crate::lobby::OutboundMessage>>();
        let second: Vec<_> = cursor.read(messages).cloned().collect();
        assert_eq!(
            feedback_count(
                &second,
                "umbilical-stop",
                crate::core::messages::ActionFeedbackOutcome::Applied,
            ),
            1
        );

        app.world_mut()
            .get_mut::<DockControl>(operator)
            .expect("operator dock")
            .docked = false;
        app.world_mut()
            .entity_mut(operator)
            .insert(crate::core::messages::AdmittedCommands(vec![
                correlated_umbilical_command(
                    "umbilical-refused",
                    SystemControlPayload::StartTransfer,
                ),
            ]));
        app.update();
        let messages = app
            .world()
            .resource::<Messages<crate::lobby::OutboundMessage>>();
        let third: Vec<_> = cursor.read(messages).cloned().collect();
        assert_eq!(
            feedback_count(
                &third,
                "umbilical-refused",
                crate::core::messages::ActionFeedbackOutcome::Refused,
            ),
            1
        );
    }

    #[test]
    fn save_state_carries_the_running_intent_only() {
        let mut u = umbilical();
        u.running = true;
        u.carry = 0.4;
        u.last_refusal = Some(UmbilicalRefusal::Undocked);
        u.operator_level = Some(50);
        let save = u.save_state();
        assert!(save.running);
    }

    #[test]
    fn an_idle_umbilical_saves_as_default() {
        assert_eq!(umbilical().save_state(), UmbilicalSaveState::default());
    }

    #[test]
    fn restore_reseeds_running_and_clears_the_projections() {
        let mut u = umbilical();
        u.carry = 0.7;
        u.last_refusal = Some(UmbilicalRefusal::Disabled);
        u.operator_level = Some(12);
        u.restore(&UmbilicalSaveState { running: true });
        assert!(u.running);
        assert_eq!(u.carry, 0.0);
        assert_eq!(u.last_refusal, None);
        assert_eq!(u.operator_level, None);
        assert_eq!(u.activation_target, None);
    }

    // ── The task lifecycle (issue #1345) ─────────────────────────────────────

    use crate::dock::mating::DockConfig;
    use crate::infrastructure::condition::{
        CapacityConfig, InfrastructureConfig, InfrastructureState,
    };

    const OPERATOR: &str = "op-1";
    const PARTNER: &str = "partner-1";

    fn dock_config() -> DockConfig {
        DockConfig {
            range: 200.0,
            engage_distance: 400.0,
            approach_speed: 60.0,
            mate_tolerance: 4.0,
            undock_clear_distance: 120.0,
            min_power_level: 1,
        }
    }

    /// A docked control, mated to `PARTNER` (or idle when `docked_to` is
    /// `None`) — the umbilical's own docking terms don't matter to these
    /// tests, only `docked_partner()`'s answer.
    fn dock(docked_to: Option<&str>) -> DockControl {
        let mut d = DockControl::new(
            SystemId(DOCK_SYSTEM_ID_FOR_TEST.into()),
            dock_config(),
            PowerGroupId("dock".into()),
        );
        if let Some(target) = docked_to {
            d.docked = true;
            d.docking_target = Some(target.into());
        }
        d
    }

    const DOCK_SYSTEM_ID_FOR_TEST: &str = "dock";

    fn capacity_config(amount: i64, ceiling: i64) -> InfrastructureConfig {
        InfrastructureConfig {
            capacities: vec![CapacityConfig {
                id: "fuel".into(),
                amount,
                ceiling: Some(ceiling),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn flow_config() -> UmbilicalConfig {
        UmbilicalConfig {
            capacity: "fuel".into(),
            rate: 10.0,
            direction: UmbilicalDirection::Deliver,
            min_power_level: 0,
        }
    }

    /// A docked pair — the operator delivering `fuel` to its partner — plus the
    /// lifecycle queue, ready to run `handle_umbilical_commands` and `tick_
    /// umbilical` chained.
    fn docked_world() -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs(1));
        app.insert_resource(time);
        app.add_systems(Update, (handle_umbilical_commands, tick_umbilical).chain());

        let operator = app
            .world_mut()
            .spawn((
                EntityUuid(OPERATOR.into()),
                umbilical_with(flow_config()),
                dock(Some(PARTNER)),
                InfrastructureCondition(InfrastructureState::from_config(&capacity_config(
                    100, 200,
                ))),
                crate::core::messages::AdmittedCommands::default(),
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid(PARTNER.into()),
            InfrastructureCondition(InfrastructureState::from_config(&capacity_config(0, 500))),
        ));
        (app, operator)
    }

    fn umbilical_with(config: UmbilicalConfig) -> TransferUmbilical {
        TransferUmbilical::new(config, PowerGroupId("umbilical".into()))
    }

    fn admit_start(app: &mut App, operator: Entity) {
        app.world_mut()
            .entity_mut(operator)
            .get_mut::<crate::core::messages::AdmittedCommands>()
            .unwrap()
            .0
            .push(crate::core::messages::AdmittedCommand {
                target: SystemId(UMBILICAL_SYSTEM_ID.into()),
                payload: SystemControlPayload::StartTransfer,
                response_token: None,
                feedback_correlation: None,
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

    fn flow_slot_for_test() -> TaskSlot {
        TaskSlot::new(OPERATOR, UMBILICAL_SYSTEM_ID, TASK_VERB_UMBILICAL_FLOW)
    }

    /// A start that actually moves capacity opens exactly one activation; an
    /// unchanged, still-flowing tick afterwards reports nothing more.
    #[test]
    fn a_flow_that_moves_capacity_opens_one_activation_and_then_falls_silent() {
        let (mut app, operator) = docked_world();
        admit_start(&mut app, operator);
        app.update();

        assert!(
            app.world()
                .entity(operator)
                .get::<TransferUmbilical>()
                .unwrap()
                .running
        );
        assert_eq!(
            drain_lifecycle(&mut app),
            vec![TaskLifecycleRequest::Start {
                slot: flow_slot_for_test(),
                target: Some(PARTNER.into()),
            }]
        );

        app.update();
        assert!(
            drain_lifecycle(&mut app).is_empty(),
            "a flow that simply continues reports nothing more"
        );
    }

    /// `StopTransfer` on a live flow reports the cancel before clearing intent;
    /// `tick_umbilical` — seeing the intent already cleared — reports nothing
    /// further, so the activation gets exactly one terminal.
    #[test]
    fn stop_transfer_reports_released_exactly_once() {
        let (mut app, operator) = docked_world();
        admit_start(&mut app, operator);
        app.update();
        drain_lifecycle(&mut app);

        app.world_mut()
            .entity_mut(operator)
            .get_mut::<crate::core::messages::AdmittedCommands>()
            .unwrap()
            .0
            .push(crate::core::messages::AdmittedCommand {
                target: SystemId(UMBILICAL_SYSTEM_ID.into()),
                payload: SystemControlPayload::StopTransfer,
                response_token: None,
                feedback_correlation: None,
            });
        app.update();

        assert_eq!(
            drain_lifecycle(&mut app),
            vec![TaskLifecycleRequest::End {
                slot: flow_slot_for_test(),
                reason: TaskTerminalReason::Released,
            }]
        );
        assert!(
            !app.world()
                .entity(operator)
                .get::<TransferUmbilical>()
                .unwrap()
                .running
        );
    }

    /// A start refused before it ever flows — nothing is docked to bridge to —
    /// still gets a whole activation, opened and closed together, mirroring the
    /// tractor's "refused before it couples" case.
    #[test]
    fn a_start_refused_before_it_ever_flows_still_opens_and_closes_one_activation() {
        let mut app = App::new();
        app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs(1));
        app.insert_resource(time);
        app.add_systems(Update, (handle_umbilical_commands, tick_umbilical).chain());
        let operator = app
            .world_mut()
            .spawn((
                EntityUuid(OPERATOR.into()),
                umbilical_with(flow_config()),
                dock(None),
                crate::core::messages::AdmittedCommands::default(),
            ))
            .id();
        admit_start(&mut app, operator);
        app.update();

        let slot = flow_slot_for_test();
        assert_eq!(
            drain_lifecycle(&mut app),
            vec![
                TaskLifecycleRequest::Start {
                    slot: slot.clone(),
                    target: None,
                },
                TaskLifecycleRequest::End {
                    slot,
                    reason: TaskTerminalReason::TargetLost,
                },
            ]
        );
    }

    /// A flow that WAS moving capacity and then loses its dock ends the
    /// standing activation with the mapped reason — a close only, no re-opened
    /// start.
    #[test]
    fn a_flow_that_loses_its_dock_ends_the_standing_activation() {
        let (mut app, operator) = docked_world();
        admit_start(&mut app, operator);
        app.update();
        drain_lifecycle(&mut app);

        app.world_mut()
            .entity_mut(operator)
            .get_mut::<DockControl>()
            .unwrap()
            .docked = false;
        app.update();

        assert_eq!(
            drain_lifecycle(&mut app),
            vec![TaskLifecycleRequest::End {
                slot: flow_slot_for_test(),
                reason: TaskTerminalReason::TargetLost,
            }],
            "the standing activation closes; nothing new opens for a flow that was already live"
        );
    }
}
