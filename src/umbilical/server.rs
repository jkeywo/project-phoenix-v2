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
use crate::damage::DamageTier;
use crate::dock::DockControl;
use crate::entities::spawner::{EntitySystemHull, EntityUuid};
use crate::infrastructure::{CapacityAdjustment, InfrastructureCondition};
use crate::messages::{
    PowerGroupId, SystemAffinity, SystemBlackboard, SystemControlPayload, SystemId,
    UmbilicalBlackboard,
};
use crate::ship::power::{power_level_for, ShipPowerSystem};
use crate::system_registry::{umbilical_system_id, UMBILICAL_SYSTEM_ID};
use crate::umbilical::flow::{
    plan_flow, CapacityEnd, FlowContext, FlowEnds, FlowVerdict, UmbilicalConfig,
    UmbilicalDirection, UmbilicalRefusal,
};
use crate::world::server::WorldContentRuntime;

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
        app.register_admitted_consumer(ConsumerMatcher::exact(UMBILICAL_SYSTEM_ID));
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
    mut ships: Query<(&crate::messages::AdmittedCommands, &mut TransferUmbilical)>,
) {
    for (admitted, mut umbilical) in ships.iter_mut() {
        for cmd in admitted.for_target(UMBILICAL_SYSTEM_ID) {
            match &cmd.payload {
                SystemControlPayload::StartTransfer if !umbilical.running => {
                    umbilical.running = true;
                    // Clear a stale refusal on a fresh start; the tick will
                    // repopulate it if this start cannot flow.
                    umbilical.last_refusal = None;
                    umbilical.carry = 0.0;
                }
                SystemControlPayload::StopTransfer => {
                    umbilical.running = false;
                    umbilical.last_refusal = None;
                    umbilical.carry = 0.0;
                }
                _ => {}
            }
        }
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
    mut ships: Query<(
        Entity,
        Option<&EntityUuid>,
        &crate::ship_plugin::ShipSystemControlSources,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        &TransferUmbilical,
        &DockControl,
        &crate::server_app::ShipSystemBlackboards,
        Has<UmbilicalAiRunning>,
        &mut crate::messages::AdmittedCommands,
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
            .get(&crate::system_registry::viewscreen_system_id())
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
    runtime: Option<ResMut<WorldContentRuntime>>,
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
            // move nothing.
            if !row.running {
                outcomes.push(Outcome {
                    entity: row.entity,
                    running: false,
                    carry: 0.0,
                    last_refusal: None, // retained on the component by leaving it; see apply
                    operator_level,
                    partner_level,
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
                FlowVerdict::Refused(refusal) => outcomes.push(Outcome {
                    entity: row.entity,
                    running: false,
                    carry: 0.0,
                    last_refusal: Some(refusal),
                    operator_level,
                    partner_level,
                    queue: None,
                }),
                FlowVerdict::Flowing {
                    operator_delta,
                    partner_delta,
                    carry,
                } => {
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
    // umbilicals are idle never marks `WorldContentRuntime` changed on a quiet
    // tick.
    if adjustments.is_empty() {
        return;
    }
    let Some(mut runtime) = runtime else {
        return;
    };
    runtime.pending_capacity_adjustments.extend(adjustments);
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
    }
}
