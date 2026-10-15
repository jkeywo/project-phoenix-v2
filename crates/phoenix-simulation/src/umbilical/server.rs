//! The Bevy adapter for the transfer umbilical (issue #1160).
//!
//! Gathers the live world into the plain values the pure sibling
//! [`crate::umbilical::flow`] takes — both docked ends' resolved resource stocks, the
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
//! one. Each planned move queues one ResourceTransfer. Its transaction drain
//! commits both endpoints or neither, accounting for reservations and tube-held
//! ammunition. Capacity changes retain the infrastructure adjustment and counter
//! publication boundary; magazine changes use their existing owner.

use crate::ai::standing_operation::OperationFacts;
use crate::ship::compartment_damage::{operation_disabled, ShipDamage};
use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
use crate::core::messages::{
    PowerGroupId, SystemAffinity, SystemBlackboard, SystemControlPayload, SystemId,
    UmbilicalBlackboard,
};
use crate::core::task_lifecycle::{
    physical_work_reports, PhysicalWork, PriorActivation, TaskLifecycleRequest, TaskSlot,
    TaskTerminalReason, TASK_VERB_UMBILICAL_FLOW,
};
use crate::dock::DockControl;
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::{EntitySystemHull, EntityUuid};
use crate::infrastructure::InfrastructureCondition;
use crate::ship::power::ShipPowerSystem;
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
/// partner without the endpoint). Running intent and fractional carry are
/// folded and snapshotted. `carry` is the sub-unit remainder the flow arithmetic meters a
/// per-second rate through; `last_refusal` is why the flow last stopped; and
/// `operator_level`/`partner_level` are this tick's ledger readings for the
/// console. The fractional carry is authoritative continuation; refusal and level readings
/// are projections the next tick re-derives — exactly as the dock leaves its `available_target`/`undock_target`
/// out.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TransferUmbilical {
    pub selected_store: Option<String>,
    pub port_maintenance: bool,
    pub port_order: crate::resources::ports::PortOrderState,
    pub port_flow: Option<crate::resources::ports::PortOrderState>,
    /// Derived terms and permission for this physically coupled provider.
    pub port_view: Option<phoenix_model::ports_ui::PortServiceView>,
    pub selected_direction: UmbilicalDirection,
    pub selected_resource: String,
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
    /// rate. Reset on start/stop; authoritative continuation, folded and saved.
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
            selected_store: None,
            port_maintenance: false,
            port_order: Default::default(),
            port_flow: None,
            port_view: None,
            selected_direction: config.direction,
            selected_resource: config.capacity.clone(),
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
    /// `TractorSaveState`. Fractional carry is retained; refusal and level projections are
    /// recomputed on the first resumed tick.
    pub fn save_state(&self) -> UmbilicalSaveState {
        UmbilicalSaveState {
            selected_store: self.selected_store.clone(),
            port_maintenance: self.port_maintenance,
            port_order: self.port_order.clone(),
            port_flow: self.port_flow.clone(),
            selected_direction: self.selected_direction,
            selected_resource: self.selected_resource.clone(),
            running: self.running,
            carry: self.carry,
        }
    }

    /// Reseed the running intent from a restored snapshot, onto an umbilical that
    /// already carries its authored config and resolved power group from the
    /// fresh spawn. Fractional carry is restored. Last refusal and level projections are
    /// rederived by the next tick from the resumed world.
    pub fn restore(&mut self, save: &UmbilicalSaveState) {
        self.selected_store = save.selected_store.clone();
        self.port_maintenance = save.port_maintenance;
        self.port_order = save.port_order.clone();
        self.port_flow = save.port_flow.clone();
        self.port_view = None;
        self.selected_direction = save.selected_direction;
        self.running = save.running;
        self.selected_resource = save.selected_resource.clone();
        self.carry = save.carry;
        self.last_refusal = None;
        self.operator_level = None;
        self.partner_level = None;
        self.activation_target = None;
    }
}

/// The snapshot-carried half of a [`TransferUmbilical`] (issue #1160): the
/// running intent and fractional flow carry.
///
/// `Default` is the idle umbilical — not running — which is what a hull that
/// authored an umbilical and never started it captures, so a resume of such a
/// ship restores byte-identically and folds the same number.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UmbilicalSaveState {
    pub selected_store: Option<String>,
    pub port_maintenance: bool,
    pub port_order: crate::resources::ports::PortOrderState,
    pub port_flow: Option<crate::resources::ports::PortOrderState>,
    pub selected_direction: UmbilicalDirection,
    pub selected_resource: String,
    #[serde(default)]
    pub running: bool,
    pub carry: f32,
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
        app.register_admitted_consumer(
            ConsumerMatcher::exact(
                crate::ship::system_registry::UMBILICAL_KIND,
                UMBILICAL_SYSTEM_ID,
            )
            .with_feedback(
                crate::command_admission::FeedbackAddress::MatcherSpelling,
                &[
                    crate::core::messages::SystemControlPayloadDiscriminants::StartTransfer,
                    crate::core::messages::SystemControlPayloadDiscriminants::StopTransfer,
                    crate::core::messages::SystemControlPayloadDiscriminants::SetTransferResource,
                    crate::core::messages::SystemControlPayloadDiscriminants::SetTransferDirection,
                    crate::core::messages::SystemControlPayloadDiscriminants::SetTransferStore,
                    crate::core::messages::SystemControlPayloadDiscriminants::SetPortMaintenance,
                    crate::core::messages::SystemControlPayloadDiscriminants::StartPortOrder,
                    crate::core::messages::SystemControlPayloadDiscriminants::StopPortOrder,
                ],
            ),
        )
        .add_message::<PendingUmbilicalActionFeedback>();
        app.add_systems(
            FixedUpdate,
            (
                // Backfill Engineering umbilical AI (issue #1162): on the shared
                // AI cadence (rule 7), emitting StartTransfer / StopTransfer
                // BEFORE `handle_umbilical_commands` consumes the tick.
                operate_umbilical_ai
                    .in_set(crate::sim_sets::FixedStep::OperateUmbilicalAi)
                    .in_set(crate::sim_sets::SimSet::Input)
                    .run_if(crate::ai::cadence::ai_tick_ready)
                    .before(handle_umbilical_commands),
                handle_umbilical_commands
                    .in_set(crate::sim_sets::FixedStep::HandleUmbilicalCommands)
                    .in_set(crate::sim_sets::SimSet::Input),
                // Decide transfers after Dock and the infrastructure adjustments
                // establish this tick's actual capacities. The shared resource
                // settlement owner applies the queued transfer later this phase.
                tick_umbilical
                    .in_set(crate::sim_sets::FixedStep::TickUmbilical)
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(crate::dock::server::tick_dock)
                    .after(crate::infrastructure::tick_infrastructure_condition),
                finish_umbilical_action_feedback
                    .in_set(crate::sim_sets::FixedStep::FinishUmbilicalActionFeedback)
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(tick_umbilical),
                publish_umbilical_blackboard
                    .in_set(crate::sim_sets::FixedStep::PublishUmbilicalBlackboard)
                    .in_set(crate::sim_sets::SimSet::Publish),
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
    factions: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    providers: Query<(
        &EntityUuid,
        &crate::resources::ports_server::CompiledPortService,
        &crate::resources::ports_server::PortProviderState,
        Option<&crate::entities::spawner::FactionComponent>,
    )>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut ships: Query<(
        Entity,
        &crate::core::messages::AdmittedCommands,
        &mut TransferUmbilical,
        Option<&EntityUuid>,
        Option<&DockControl>,
        Option<&crate::resources::logistics_server::CompiledShipLogistics>,
        Option<&crate::entities::spawner::FactionComponent>,
    )>,
    mut pending: Option<ResMut<Messages<PendingUmbilicalActionFeedback>>>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    for (entity, admitted, mut umbilical, uuid, dock, stores, faction) in ships.iter_mut() {
        for cmd in admitted.for_target(UMBILICAL_SYSTEM_ID) {
            match &cmd.payload {
                SystemControlPayload::SetTransferStore { store } => {
                    let permitted = !umbilical.running
                        && store.as_ref().is_none_or(|id| {
                            stores.is_some_and(|profile| {
                                profile.0.config().stores.iter().any(|entry| {
                                    &entry.id == id
                                        && entry.role == phoenix_model::logistics::StoreRole::Main
                                        && entry.resource == umbilical.selected_resource
                                })
                            })
                        });
                    if permitted {
                        umbilical.selected_store = store.clone();
                        umbilical.carry = 0.0;
                        umbilical.port_flow = None;
                    }
                    crate::command_admission::finish_admitted_action_feedback(
                        &mut outbound,
                        cmd,
                        if permitted {
                            crate::core::messages::ActionFeedbackOutcome::Applied
                        } else {
                            crate::core::messages::ActionFeedbackOutcome::Refused
                        },
                    );
                }
                SystemControlPayload::SetPortMaintenance { enabled } => {
                    if umbilical.port_maintenance != *enabled {
                        umbilical.port_flow = None;
                    }
                    umbilical.port_maintenance = *enabled;
                    crate::command_admission::finish_admitted_action_feedback(
                        &mut outbound,
                        cmd,
                        crate::core::messages::ActionFeedbackOutcome::Applied,
                    );
                }
                SystemControlPayload::StartPortOrder {
                    partner_uuid,
                    visit,
                    lines,
                } => {
                    let candidate = providers.iter().find(|(id, ..)| id.0 == *partner_uuid);
                    let permitted = !umbilical.running
                        && dock.and_then(DockControl::docked_partner)
                            == Some(partner_uuid.as_str())
                        && umbilical.port_view.as_ref().is_some_and(|view| {
                            view.eligible
                                && view.partner_uuid == *partner_uuid
                                && view.visit == *visit
                        });
                    let next = candidate.and_then(|(_, profile, ledger, provider_faction)| {
                        let (Some(faction), Some(provider_faction), Some(uuid)) =
                            (faction, provider_faction, uuid)
                        else {
                            return None;
                        };
                        if !permitted
                            || !profile.0.eligible_factions.contains(&faction.0)
                            || factions.as_ref().is_some_and(|registry| {
                                crate::ai::faction::is_enemy(
                                    Some(faction.0),
                                    Some(provider_faction.0),
                                    &registry.0,
                                ) || crate::ai::faction::is_enemy(
                                    Some(provider_faction.0),
                                    Some(faction.0),
                                    &registry.0,
                                )
                            })
                        {
                            return None;
                        }
                        crate::resources::ports_server::approved_order(
                            &profile.0,
                            &ledger.0,
                            &uuid.0,
                            partner_uuid,
                            *visit,
                            lines.clone(),
                            stores,
                            &umbilical.port_order,
                        )
                        .ok()
                    });
                    let approved = next.is_some();
                    if let Some(next) = next {
                        umbilical.port_order = next;
                        umbilical.running = true;
                        umbilical.carry = 0.0;
                    }
                    crate::command_admission::finish_admitted_action_feedback(
                        &mut outbound,
                        cmd,
                        if approved {
                            crate::core::messages::ActionFeedbackOutcome::Applied
                        } else {
                            crate::core::messages::ActionFeedbackOutcome::Refused
                        },
                    );
                }
                SystemControlPayload::StopPortOrder => {
                    umbilical.port_order.stop();
                    umbilical.running = false;
                    crate::command_admission::finish_admitted_action_feedback(
                        &mut outbound,
                        cmd,
                        crate::core::messages::ActionFeedbackOutcome::Applied,
                    );
                }
                SystemControlPayload::SetTransferDirection { direction } => {
                    let permitted = !umbilical.running;
                    if permitted && umbilical.selected_direction != *direction {
                        umbilical.selected_direction = *direction;
                        umbilical.port_flow = None;
                        umbilical.carry = 0.0;
                        umbilical.last_refusal = None;
                        umbilical.operator_level = None;
                        umbilical.partner_level = None;
                    }
                    crate::command_admission::finish_admitted_action_feedback(
                        &mut outbound,
                        cmd,
                        if permitted {
                            crate::core::messages::ActionFeedbackOutcome::Applied
                        } else {
                            crate::core::messages::ActionFeedbackOutcome::Refused
                        },
                    );
                }
                SystemControlPayload::SetTransferResource { resource } => {
                    let permitted = umbilical.config.permits_resource(resource);
                    if permitted && umbilical.selected_resource != *resource {
                        umbilical.selected_resource = resource.clone();
                        umbilical.selected_store = None;
                        umbilical.port_flow = None;
                        umbilical.carry = 0.0;
                        umbilical.last_refusal = None;
                    }
                    crate::command_admission::finish_admitted_action_feedback(
                        &mut outbound,
                        cmd,
                        if permitted {
                            crate::core::messages::ActionFeedbackOutcome::Applied
                        } else {
                            crate::core::messages::ActionFeedbackOutcome::Refused
                        },
                    );
                }
                SystemControlPayload::StartTransfer => {
                    let manual = dock
                        .and_then(DockControl::docked_partner)
                        .and_then(|partner| providers.iter().find(|(id, ..)| id.0 == partner))
                        .filter(|(_, profile, ..)| {
                            profile.0.policy == phoenix_model::ports::PortServicePolicy::ManualOffer
                        });
                    if manual.is_some_and(|(provider, _, ledger, _)| {
                        !umbilical.port_order.active
                            || umbilical.port_order.partner.as_deref() != Some(provider.0.as_str())
                            || !uuid.is_some_and(|uuid| {
                                ledger.0.visitors.get(&uuid.0).is_some_and(|visit| {
                                    Some(visit.visit) == umbilical.port_order.visit
                                })
                            })
                    }) {
                        crate::command_admission::finish_admitted_action_feedback(
                            &mut outbound,
                            cmd,
                            crate::core::messages::ActionFeedbackOutcome::Refused,
                        );
                        continue;
                    }
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
        // The same individual Umbilical AI may maintain observed Main goals
        // at a free allied port. It never originates manual trade approval.
        let port = config
            .and_then(|config| {
                config
                    .0
                    .observations_for_system(&umbilical_system_id(), &blackboards.0)
                    .get(&umbilical_system_id())
            })
            .and_then(|board| match board {
                SystemBlackboard::Umbilical(view) => view.port.as_ref(),
                _ => None,
            });
        let transfer_active = match config.and_then(|config| {
            config
                .0
                .observations_for_system(&umbilical_system_id(), &blackboards.0)
                .get(&crate::ship::system_registry::viewscreen_system_id())
        }) {
            Some(SystemBlackboard::Viewscreen(vbb)) => crate::objectives::top_operate_directive(
                &vbb.scored_objectives,
                SystemAffinity::Engineering,
                |d| crate::objectives::transfer_directive_target(d).is_some(),
            )
            .is_some(),
            _ => false,
        };

        let maintenance = crate::umbilical::flow::port_maintenance_change(
            crate::umbilical::flow::PortMaintenanceFacts {
                explicit_transfer: transfer_active,
                running: umbilical.running,
                maintaining: umbilical.port_maintenance,
                eligible_free_port: port.is_some_and(|port| {
                    port.policy == phoenix_model::ports::PortServicePolicy::FreeAlliedReplenishment
                        && port.eligible
                        && !port.stores.is_empty()
                }),
            },
        );
        if let Some(enabled) = maintenance {
            emit_ai_command(
                uuid,
                umbilical_system_id(),
                SystemControlPayload::SetPortMaintenance { enabled },
                sources,
                &sessions,
                config,
                &mut admitted,
            );
        }
        let decision = crate::ai::standing_operation::decide(OperationFacts {
            order_present: transfer_active,
            ready: dock.docked_partner().is_some(),
            active: umbilical.running,
            owned: host_running,
        });
        if decision.claim {
            commands.entity(entity).insert(UmbilicalAiRunning);
        }
        let payload = if decision.withdraw {
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
            decision
                .start
                .then_some(SystemControlPayload::StartTransfer)
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
    queue: Option<crate::resources::server::ResourceTransfer>,
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
/// and the reason is retained; on a flow the two `CapacityAdjustment`s are
/// queued for the infrastructure tick. The operator's own level and the partner's
/// are projected onto the component every tick, running or not, so the console
/// shows both ends' levels the moment a berth is in reach.
#[allow(clippy::type_complexity)]
pub fn tick_umbilical(
    physical_q: Query<&ShipDamage>,
    mut power_q: Query<&mut ShipPowerSystem>,
    // The capacity-adjustment queue, extracted off `WorldContentRuntime` (issue
    // #1223) and owned by `InfrastructurePlugin`; this pusher feeds it. `Option`
    // so a reduced test app that runs the umbilical systems without
    // `InfrastructurePlugin` is a no-op rather than a panic.
    transfer_queue: Option<ResMut<EffectQueue<crate::resources::server::ResourceTransfer>>>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    time: Res<Time>,
    mut set: ParamSet<(
        // Operator rows: everything the verdict needs off the umbilical's own hull.
        Query<(
            Entity,
            &EntityUuid,
            &TransferUmbilical,
            &DockControl,
            Option<&EntitySystemHull>,
            Option<&ShipDamage>,
            Option<&crate::resources::logistics_server::CompiledShipLogistics>,
        )>,
        // Every entity's capacity ledger, to resolve both docked ends.
        Query<(
            &EntityUuid,
            Option<&InfrastructureCondition>,
            Option<&crate::console::weapons::torpedo::TorpedoSystemResource>,
            Option<&crate::resources::server::ResourceServices>,
        )>,
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
        .filter(|(_, _, tool, ..)| tool.port_view.is_none())
        .map(|(entity, uuid, umbilical, dock, hull, damage, stores)| {
            let power_level = u8::MAX;
            let disabled = operation_disabled(damage, hull, &umbilical_system_id());
            OperatorRow {
                entity,
                uuid: uuid.0.clone(),
                running: umbilical.running,
                partner: dock.docked_partner().map(|s| s.to_string()),
                power_level,
                disabled,
                config: {
                    let mut config = umbilical.config.clone();
                    config.capacity = umbilical.selected_resource.clone();
                    config.direction = umbilical.selected_direction;
                    // The explicit provider describes the original resource only.
                    // Other authored choices use their exact named service.
                    if config.capacity != umbilical.config.capacity {
                        config.link = Default::default();
                    }
                    if let Some(store) = umbilical.selected_store.as_ref().and_then(|id| {
                        stores.and_then(|profile| {
                            profile.0.config().stores.iter().find(|store| {
                                store.id == *id
                                    && store.role == phoenix_model::logistics::StoreRole::Main
                                    && store.resource == config.capacity
                            })
                        })
                    }) {
                        config.link = crate::resources::service::ResourceLink {
                            service: None,
                            provider: Some(store.endpoint.clone()),
                        };
                    }
                    config
                },
                carry: umbilical.carry,
                activation_target: umbilical.activation_target.clone(),
            }
        })
        .collect();
    if rows.is_empty() {
        return;
    }

    // Consumer-owned binding at the operator; the remote end uses its exact
    // authored resource default. Neither binding selects another dock partner.
    let empty_services = std::collections::BTreeMap::new();
    let read_end = |conditions: &Query<(
        &EntityUuid,
        Option<&InfrastructureCondition>,
        Option<&crate::console::weapons::torpedo::TorpedoSystemResource>,
        Option<&crate::resources::server::ResourceServices>,
    )>,
                    uuid: &str,
                    resource: &str,
                    link: &crate::resources::service::ResourceLink|
     -> Option<(CapacityEnd, crate::resources::service::ResourceEndpoint)> {
        let (_, infrastructure, torpedoes, services) =
            conditions.iter().find(|(id, _, _, _)| id.0 == uuid)?;
        let endpoint = crate::resources::service::resolve_endpoint(
            resource,
            services.map(|s| &s.0).unwrap_or(&empty_services),
            link,
        )
        .ok()?;
        let (stock, held) =
            crate::resources::server::endpoint_stock(&endpoint, infrastructure, torpedoes)?;
        Some((
            CapacityEnd {
                level: stock.available(),
                headroom: stock.headroom().saturating_sub(held),
            },
            endpoint,
        ))
    };

    let mut outcomes: Vec<Outcome> = Vec::with_capacity(rows.len());
    {
        let conditions = set.p1();
        for row in &rows {
            let operator_binding = read_end(
                &conditions,
                &row.uuid,
                &row.config.capacity,
                &row.config.link,
            );
            let operator_end = operator_binding.as_ref().map(|(end, _)| *end);
            let partner_binding = row.partner.as_deref().and_then(|p| {
                read_end(
                    &conditions,
                    p,
                    &row.config.capacity,
                    &crate::resources::service::ResourceLink::default(),
                )
            });
            let partner_end = partner_binding.as_ref().map(|(end, _)| *end);
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

            let mut ctx = FlowContext {
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
            if matches!(
                plan_flow(&row.config, &ends, &ctx),
                FlowVerdict::Flowing { .. }
            ) {
                let available = match (&operator_end, &partner_end, row.config.direction) {
                    (Some(own), Some(remote), UmbilicalDirection::Deliver) => {
                        own.level.min(remote.headroom)
                    }
                    (Some(own), Some(remote), UmbilicalDirection::Collect) => {
                        remote.level.min(own.headroom)
                    }
                    _ => 0,
                };
                if available > 0 && dt > 0.0 {
                    let activity =
                        ((available as f32 - row.carry).max(0.0) / (row.config.rate * dt)).min(1.0);
                    let fraction = power_q.get_mut(row.entity).ok().map_or(0.0, |mut power| {
                        power
                            .0
                            .work_continuous(&umbilical_system_id(), dt, activity)
                            .map_or(0.0, |grant| grant.fraction)
                    });
                    // Exhaustion pauses movement and preserves standing intent
                    // and already funded fractional flow.
                    let efficiency = physical_q.get(row.entity).map_or(1.0, |damage| {
                        damage.factor(
                            &umbilical_system_id(),
                            phoenix_model::damage::DamageBinding::Operation,
                        )
                    });
                    ctx.dt *= activity * fraction * efficiency;
                } else {
                    ctx.dt = 0.0;
                }
            }
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
                    for report in physical_work_reports(
                        flow_slot(&row.uuid),
                        PriorActivation {
                            active: row.activation_target.is_some(),
                            target: row.activation_target.as_deref(),
                        },
                        PhysicalWork::Refused {
                            target: row.partner.as_deref(),
                            reason: terminal_reason_for(refusal),
                        },
                    ) {
                        push_lifecycle(lifecycle.as_deref_mut(), report);
                    }
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
                    partner_delta: _,
                    carry,
                } => {
                    let worked = operator_delta != 0 || carry > row.carry;
                    // Flowing always implies a docked partner (the `docked` gate
                    // above is the only way to reach here), so `row.partner` is
                    // always `Some`. Open the activation the moment it starts
                    // moving anything, and re-open it if the partner ever
                    // changed under a live flow — unreachable today (a partner
                    // change requires an undock, which refuses the flow first),
                    // but kept symmetric with the tractor's own re-designation
                    // handling rather than assumed away.
                    if worked {
                        for report in physical_work_reports(
                            flow_slot(&row.uuid),
                            PriorActivation {
                                active: row.activation_target.is_some(),
                                target: row.activation_target.as_deref(),
                            },
                            PhysicalWork::Formed {
                                target: row.partner.as_deref(),
                                follow_subject: true,
                            },
                        ) {
                            push_lifecycle(lifecycle.as_deref_mut(), report);
                        }
                    }
                    // A move only reaches the queue when it is non-zero and there
                    // is a partner to move it to — a depleted source or a full
                    // destination keeps the flow running but queues nothing.
                    let queue = match (
                        &row.partner,
                        &operator_binding,
                        &partner_binding,
                        operator_delta,
                    ) {
                        (
                            Some(partner),
                            Some((_, own_endpoint)),
                            Some((_, remote_endpoint)),
                            delta,
                        ) if delta != 0 => {
                            let (source, source_endpoint, destination, destination_endpoint) =
                                if delta < 0 {
                                    (
                                        row.uuid.clone(),
                                        own_endpoint.clone(),
                                        partner.clone(),
                                        remote_endpoint.clone(),
                                    )
                                } else {
                                    (
                                        partner.clone(),
                                        remote_endpoint.clone(),
                                        row.uuid.clone(),
                                        own_endpoint.clone(),
                                    )
                                };
                            Some(crate::resources::server::ResourceTransfer {
                                operator: row.uuid.clone(),
                                source,
                                source_endpoint,
                                destination,
                                destination_endpoint,
                                amount: delta.abs(),
                                receipt: None,
                            })
                        }
                        _ => None,
                    };
                    outcomes.push(Outcome {
                        entity: row.entity,
                        running: true,
                        carry,
                        last_refusal: None,
                        operator_level,
                        partner_level,
                        activation_target: if worked {
                            row.partner.clone()
                        } else {
                            row.activation_target.clone()
                        },
                        queue,
                    });
                }
            }
        }
    }

    // Collect the capacity moves in UUID order, so two hosts queue identically —
    // the walk-order rule the infrastructure and operations ticks both keep.
    let transfers: Vec<_> = outcomes
        .iter()
        .filter_map(|out| out.queue.clone())
        .collect();

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
    if transfers.is_empty() {
        return;
    }
    let Some(mut transfer_queue) = transfer_queue else {
        return;
    };
    transfer_queue.0.extend(transfers);
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
            selected_store: umbilical.selected_store.clone(),
            port_maintenance: umbilical.port_maintenance,
            port: umbilical.port_view.clone(),
            selected_resource: umbilical.selected_resource.clone(),
            allowed_resources: umbilical.config.resource_choices(),
            capacity: umbilical.config.capacity.clone(),
            rate: umbilical.config.rate,
            direction: match umbilical.selected_direction {
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
#[path = "server_tests.rs"]
mod tests;
