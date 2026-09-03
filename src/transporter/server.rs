//! The Bevy adapter for the rescue transporter (issue #1348, PRD #1337).
//!
//! Gathers the live world into scalars, hands them to the pure sibling
//! [`crate::transporter::coupling`], and applies what comes back — the per-ship
//! [`Transporter`] component, the per-contact [`CivilianRescue`] component, and
//! the fixed-tick systems that take the select/start/stop commands, reveal a
//! contact once Sensors have scanned it, decide whether a transport runs and
//! advance the recovery, run the Engineering backfill, record a casualty when
//! civilians are lost to fire, and publish the blackboard. Nothing here decides
//! eligibility itself: rule 10, the split between the pure `coupling` sibling and
//! this adapter — the exact shape [`crate::tractor::server`] keeps.
//!
//! # An engineering rescue system
//!
//! A transporter is an admission-gated engineering `[[system]]` the crew run
//! against a discovered rescue contact. Selecting a contact, starting the
//! recovery, losing range, losing the contact, cancelling, completing the
//! rescue, or the mission ending all close a running transport's lifecycle with
//! the appropriate terminal state, exactly as the tractor's hold does.

use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::command_admission::ai_emit::emit_ai_command;
use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
use crate::core::messages::{
    PowerGroupId, SystemAffinity, SystemBlackboard, SystemControlPayload, SystemId,
    TransporterBlackboard,
};
use crate::core::task_lifecycle::{
    TaskLifecycleRequest, TaskSlot, TaskTerminalReason, TASK_VERB_TRANSPORT,
};
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::{EntityId, EntityName, EntitySystemHull, EntityUuid};
use crate::science::scan::scanned_flag;
use crate::ship::damage::DamageTier;
use crate::ship::power::{power_level_for, ShipPowerSystem};
use crate::ship::system_registry::{transporter_system_id, TRANSPORTER_SYSTEM_ID};
use crate::transporter::coupling::{
    transport_status, TransportInputs, TransportRefusal, TransporterConfig,
};
use crate::world::server::WorldContentRuntime;

/// One ship's rescue transporter (issue #1348): the authored terms, the power
/// group it draws from, and the live selection/transport state.
///
/// Inserted at spawn only on a hull that authored a `[transporter]` table AND a
/// `kind = "transporter"` `[[system]]` — a hull with neither carries no component
/// and is byte-identical in every way to one built before this existed (AGENTS.md
/// rule 11).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Transporter {
    /// The authored terms — range, per-civilian duration, minimum power level.
    pub config: TransporterConfig,
    /// The power group the transporter `[[system]]` declared, resolved once at
    /// spawn so the runtime never re-reads the authored systems list.
    pub power_group: PowerGroupId,
    /// The contact the operator has selected for rescue, or `None`. Set by
    /// `TransportSelectContact`, and the target `StartTransport` runs against.
    pub selected_contact: Option<String>,
    /// The operator's standing intent to transport. Set true by `StartTransport`,
    /// false by `StopTransport`, a fresh selection, and every interruption.
    pub transporting: bool,
    /// The contact the current lifecycle activation is open against — the
    /// transporter's analogue of the tractor's `coupled_target`. `Some` only
    /// while a transport is actually running, so every transition of it is a
    /// lifecycle beat and nothing else is.
    pub active_contact: Option<String>,
    /// Accrual toward the NEXT civilian, in `[0, 1)`. Advances by
    /// `dt / seconds_per_civilian` each eligible tick; each time it crosses `1.0`
    /// one civilian is recovered.
    pub progress: f32,
    /// Why the last start or transport could not run — the reason the console
    /// shows, retained until the operator acts again. `None` when idle or running
    /// cleanly.
    pub last_refusal: Option<TransportRefusal>,
}

impl Transporter {
    /// A fresh, idle transporter carrying its authored terms and resolved power
    /// group.
    pub fn new(config: TransporterConfig, power_group: PowerGroupId) -> Self {
        Self {
            config,
            power_group,
            selected_contact: None,
            transporting: false,
            active_contact: None,
            progress: 0.0,
            last_refusal: None,
        }
    }
}

/// Marks a ship whose transporter is CURRENTLY driven BY the backfill rescue
/// host, not by a console (issue #1348) — the transporter's analogue of
/// [`crate::tractor::server::TractorAiEngaged`]. The host inserts it when it
/// starts a transport to serve a `Rescue` directive and removes it when the
/// directive is withdrawn; it stops a transport only while this marker is
/// present, so it never undoes one a console started on the same AI-operated
/// system. Not folded and not snapshotted: re-derived from the still-active
/// directive on resume.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct TransporterAiEngaged;

/// The civilians a scannable contact carries, and how many have been recovered
/// (issue #1348).
///
/// Authored on a contact entity via its `[civilian_rescue]` table. `revealed`
/// latches once Sensors complete the scan that discovers the life signs; the
/// transporter refuses an unrevealed contact, which is how "the computer
/// announces life signs and transport availability only after the revealing
/// scan" is enforced. `recovered` is the authoritative count the transporter
/// advances.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct CivilianRescue {
    /// How many civilians the contact carries in total — the authored figure.
    pub count: u32,
    /// How many have been beamed aboard so far. `0` at spawn; the transporter
    /// advances it, and the contact is fully rescued when it reaches `count`.
    pub recovered: u32,
    /// Whether a completed scan has revealed the life signs. Latched true by
    /// [`reveal_civilian_contacts`] and never cleared — being scanned once is a
    /// permanent fact, the same latch [`scanned_flag`] keeps.
    pub revealed: bool,
}

impl CivilianRescue {
    /// A fresh, unrevealed contact carrying its authored civilian count.
    pub fn new(count: u32) -> Self {
        Self {
            count,
            recovered: 0,
            revealed: false,
        }
    }

    /// How many civilians are still aboard, unrecovered.
    pub fn remaining(&self) -> u32 {
        self.count.saturating_sub(self.recovered)
    }
}

/// Last-known civilian counts for contacts, kept so a casualty can still be
/// attributed after the contact is despawned (issue #1348).
///
/// A **cache**, rebuilt each tick from the live [`CivilianRescue`] components:
/// when fire destroys a carrier the entity is gone by the time
/// [`record_civilian_casualties`] reads the destruction event, so the last tick
/// this ledger saw of it is what says how many souls were lost. `recorded`
/// stops one destruction from raising the loss more than once. Nothing in the
/// fixed tick branches on it and neither `sim_digest` nor `snapshot` walks it.
#[derive(Resource, Debug, Clone, Default)]
pub struct CivilianRescueLedger {
    /// uuid → (world id, unrecovered civilians last seen aboard).
    entries: BTreeMap<String, (String, u32)>,
    /// uuids whose loss has already been recorded.
    recorded: BTreeSet<String>,
}

/// Registers the transporter systems and its admitted-command consumer (issue
/// #1348). Added by `WorldPlugin` alongside the tractor's.
pub struct TransporterPlugin;

impl Plugin for TransporterPlugin {
    fn build(&self, app: &mut App) {
        crate::ai::cadence::register_ai_cadence(app);
        app.init_resource::<CivilianRescueLedger>();
        {
            use crate::authoritative::{DeclareState, StateClass};
            // The transporter and the per-contact civilian counts are
            // authoritative simulation state — a resume that lost them would let
            // a rescued contact be rescued again — but this first slice does not
            // yet fold them into `world_digest` or project them into the
            // snapshot, so they are `DeferredFold`, the honest reading of that
            // class (the same one #801's drive states took). The ledger is a
            // rebuilt-each-tick cache and the AI marker is derived.
            app.declare_state::<Transporter>(StateClass::DeferredFold, "transporter-state")
                .declare_state::<CivilianRescue>(StateClass::DeferredFold, "civilian-rescue-state")
                .declare_state::<CivilianRescueLedger>(
                    StateClass::Cache,
                    "digest-exclusion-classes",
                )
                .declare_state::<TransporterAiEngaged>(StateClass::Derived, "transporter-state");
        }
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::TRANSPORTER_KIND,
            TRANSPORTER_SYSTEM_ID,
        ));
        app.add_systems(
            FixedUpdate,
            (
                // Backfill Engineering rescue AI: decides on the shared AI
                // cadence and emits before the command handler consumes the
                // tick's admitted commands.
                operate_transporter_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .run_if(crate::ai::cadence::ai_tick_ready)
                    .before(handle_transporter_commands),
                handle_transporter_commands.in_set(crate::sim_sets::SimSet::Input),
                // Latch discovery once the contact has been scanned, before the
                // verdict reads it.
                reveal_civilian_contacts.in_set(crate::sim_sets::SimSet::Modifiers),
                // Decide whether the transport runs this tick and advance the
                // recovery. Ordered after the reveal so a scan that lands this
                // tick discovers the contact before the verdict reads it.
                tick_transport
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(reveal_civilian_contacts),
                // Attribute a casualty when a carrier is destroyed with civilians
                // still aboard.
                record_civilian_casualties.in_set(crate::sim_sets::SimSet::Publish),
                publish_transporter_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

/// The lifecycle slot a hull's rescue transport occupies (issue #1348) — its
/// uuid, the transporter system, and the transport verb.
fn transport_slot(uuid: Option<&EntityUuid>) -> TaskSlot {
    TaskSlot::new(
        uuid.map(|u| u.0.clone()).unwrap_or_default(),
        TRANSPORTER_SYSTEM_ID,
        TASK_VERB_TRANSPORT,
    )
}

/// Queue one lifecycle report, if there is a queue and the hull has a uuid.
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

/// The terminal reason a dropped transport reports, from the refusal the pure
/// verdict returned (issue #1348).
///
/// A lost SELECTION is the target going away (interrupted); an unrevealed
/// contact is one the work cannot read yet (`Unreadable`); power, damage and
/// range are the task's own preconditions failing (failed). The emitter upgrades
/// the range/lock pair to `TargetDestroyed` when the subject has actually left
/// the world — a distinction the verdict cannot make.
fn terminal_reason_for(refusal: TransportRefusal) -> TaskTerminalReason {
    match refusal {
        TransportRefusal::NoContact => TaskTerminalReason::TargetLost,
        TransportRefusal::NotDiscovered => TaskTerminalReason::Unreadable,
        TransportRefusal::NoCivilians => TaskTerminalReason::Completed,
        TransportRefusal::OutOfRange => TaskTerminalReason::OutOfRange,
        TransportRefusal::Unpowered => TaskTerminalReason::Unpowered,
        TransportRefusal::Disabled => TaskTerminalReason::Disabled,
    }
}

// ── The select / start / stop commands ───────────────────────────────────────

/// Runs in `SimSet::Input` and reads `AdmittedCommands` for the transporter
/// system (issue #1348). It sets only the operator's INTENT; `tick_transport`
/// decides whether a transport actually runs and records any refusal.
///
/// Human and AI reach this identically (AGENTS.md rule 6): admission has already
/// decided who may speak and stripped the source.
///
/// Selecting a fresh contact, or stopping, ends any running transport's
/// activation here (the release is an intent and needs no subject); the START of
/// an activation is minted by `tick_transport`, which is the only place that
/// knows a transport actually began running.
pub fn handle_transporter_commands(
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut ships: Query<(
        &crate::core::messages::AdmittedCommands,
        &mut Transporter,
        Option<&EntityUuid>,
    )>,
) {
    for (admitted, mut transporter, uuid) in ships.iter_mut() {
        for cmd in admitted.for_target(TRANSPORTER_SYSTEM_ID) {
            match &cmd.payload {
                SystemControlPayload::TransportSelectContact { uuid: selected } => {
                    // A fresh selection ends any running transport of the old
                    // contact (the operator turned the beam elsewhere) and resets
                    // the recovery accrual. Selecting the SAME contact again is
                    // idempotent — it does not restart a running transport.
                    if transporter.selected_contact.as_deref() != Some(selected.as_str()) {
                        if transporter.active_contact.is_some() {
                            push_lifecycle(
                                lifecycle.as_deref_mut(),
                                uuid,
                                TaskLifecycleRequest::End {
                                    slot: transport_slot(uuid),
                                    reason: TaskTerminalReason::Released,
                                },
                            );
                        }
                        transporter.selected_contact = Some(selected.clone());
                        transporter.transporting = false;
                        transporter.active_contact = None;
                        transporter.progress = 0.0;
                        transporter.last_refusal = None;
                    }
                }
                SystemControlPayload::StartTransport => {
                    transporter.transporting = true;
                    transporter.last_refusal = None;
                }
                SystemControlPayload::StopTransport => {
                    if transporter.transporting && transporter.active_contact.is_some() {
                        push_lifecycle(
                            lifecycle.as_deref_mut(),
                            uuid,
                            TaskLifecycleRequest::End {
                                slot: transport_slot(uuid),
                                reason: TaskTerminalReason::Released,
                            },
                        );
                    }
                    transporter.transporting = false;
                    transporter.active_contact = None;
                    transporter.last_refusal = None;
                }
                _ => {}
            }
        }
    }
}

// ── Discovery ─────────────────────────────────────────────────────────────────

/// Latch a civilian contact as revealed once Sensors have completed a scan of it
/// (issue #1348).
///
/// Reads the base-world flag store [`scanned_flag`] mirrors a completed reading
/// into (`science::server::tick_scans`), keyed on the contact's authored world
/// `[[entity]] id`. This is the discovery gate: a contact stays `revealed =
/// false` — and so unrescuable — until the crew have actually gone and looked at
/// it, which is the same `scan.<id>.taken` fact an authored `on_flag_set`
/// discovery beat hangs off. A world with no `WorldContentRuntime` (a bare-`App`
/// fixture) reveals nothing here, so a test sets `revealed` directly.
///
/// Walks contacts in uuid order and latches only — never clears — so two hosts
/// reveal identically and a resume that already scanned stays revealed.
pub fn reveal_civilian_contacts(
    runtime: Option<Res<WorldContentRuntime>>,
    mut contacts: Query<(&EntityId, &mut CivilianRescue)>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    let mut rows: Vec<(String, Mut<CivilianRescue>)> = contacts
        .iter_mut()
        .filter(|(_, rescue)| !rescue.revealed)
        .map(|(id, rescue)| (id.0.clone(), rescue))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (entity_id, mut rescue) in rows {
        if runtime.flags.counter(&scanned_flag(&entity_id)) > 0 {
            rescue.revealed = true;
        }
    }
}

// ── The transport verdict + recovery ─────────────────────────────────────────

/// Decide, for every transporting ship, whether the transport runs this tick and
/// advance the recovery (issue #1348).
///
/// Reads the live world into the scalars the pure [`transport_status`] takes —
/// the selected contact's revealed flag and remaining civilians, the separation
/// off both ends' `Transform`, the transporter's power-group level and damage
/// tier — and applies the verdict. On success the recovery accrues at the
/// authored rate and the lifecycle activation follows the running transport; on
/// any refusal the intent drops, the reason is retained, and a running
/// transport's activation ends with the mapped terminal reason.
#[allow(clippy::type_complexity)]
pub fn tick_transport(
    time: Option<Res<Time>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut set: ParamSet<(
        // Operator rows: everything the verdict needs off the operator.
        Query<(
            Entity,
            &Transporter,
            &Transform,
            Option<&ShipPowerSystem>,
            Option<&EntitySystemHull>,
            Option<&EntityUuid>,
        )>,
        // Every candidate contact's position, civilian state and world id.
        Query<(
            &EntityUuid,
            &Transform,
            &mut CivilianRescue,
            Option<&EntityId>,
        )>,
        // Apply the verdict.
        Query<&mut Transporter>,
    )>,
) {
    let dt = time.map(|t| t.delta_secs()).unwrap_or(0.0);

    struct Row {
        entity: Entity,
        uuid: Option<EntityUuid>,
        selected: Option<String>,
        operator_pos: Vec3,
        power_level: u8,
        disabled: bool,
        range: f32,
        min_power_level: u8,
        seconds_per_civilian: f32,
        active_contact: Option<String>,
        progress: f32,
    }
    let rows: Vec<Row> = set
        .p0()
        .iter()
        .filter(|(_, t, _, _, _, _)| t.transporting)
        .map(|(entity, t, transform, power, hull, uuid)| {
            let power_level = power
                .map(|p| power_level_for(&p.0, &t.power_group))
                .unwrap_or(0);
            let disabled = hull
                .map(|h| {
                    matches!(
                        h.0.tier_for(&transporter_system_id()),
                        DamageTier::Disabled | DamageTier::Destroyed
                    )
                })
                .unwrap_or(false);
            Row {
                entity,
                uuid: uuid.cloned(),
                selected: t.selected_contact.clone(),
                operator_pos: transform.translation,
                power_level,
                disabled,
                range: t.config.range,
                min_power_level: t.config.min_power_level,
                seconds_per_civilian: t.config.seconds_per_civilian,
                active_contact: t.active_contact.clone(),
                progress: t.progress,
            }
        })
        .collect();
    if rows.is_empty() {
        return;
    }

    // Resolve each selected contact's live facts (separation, revealed,
    // remaining, world id) from the contact query.
    struct ContactFacts {
        separation: Option<f32>,
        revealed: bool,
        remaining: u32,
        entity_id: Option<String>,
    }
    let contact_facts: Vec<ContactFacts> = {
        let contacts = set.p1();
        rows.iter()
            .map(|row| {
                let Some(sel) = row.selected.as_deref() else {
                    return ContactFacts {
                        separation: None,
                        revealed: false,
                        remaining: 0,
                        entity_id: None,
                    };
                };
                match contacts.iter().find(|(u, _, _, _)| u.0 == sel) {
                    Some((_, transform, rescue, id)) => ContactFacts {
                        separation: Some(row.operator_pos.distance(transform.translation)),
                        revealed: rescue.revealed,
                        remaining: rescue.remaining(),
                        entity_id: id.map(|i| i.0.clone()),
                    },
                    None => ContactFacts {
                        separation: None,
                        revealed: false,
                        remaining: 0,
                        entity_id: None,
                    },
                }
            })
            .collect()
    };

    // Decide each row, collecting the recovery to apply to the contacts and the
    // transporter state to write back.
    struct Applied {
        entity: Entity,
        transporting: bool,
        active_contact: Option<String>,
        progress: f32,
        refusal: Option<TransportRefusal>,
        // (contact uuid, civilians to recover this tick)
        recover: Option<(String, u32)>,
        // The contact's world id, if the rescue completed this tick.
        completed_id: Option<String>,
    }
    let mut applied: Vec<Applied> = Vec::with_capacity(rows.len());
    for (row, facts) in rows.iter().zip(&contact_facts) {
        let inputs = TransportInputs {
            selected: row.selected.as_deref(),
            discovered: facts.revealed,
            remaining: facts.remaining,
            separation: facts.separation,
            range: row.range,
            power_level: row.power_level,
            min_power_level: row.min_power_level,
            disabled: row.disabled,
        };
        match transport_status(&inputs) {
            Ok(()) => {
                let contact = row.selected.clone().expect("Ok implies a selection");
                // Open a fresh activation when the running transport begins, or
                // re-target if the selection moved under a live transport.
                if row.active_contact.as_deref() != Some(contact.as_str()) {
                    if row.active_contact.is_some() {
                        push_lifecycle(
                            lifecycle.as_deref_mut(),
                            row.uuid.as_ref(),
                            TaskLifecycleRequest::End {
                                slot: transport_slot(row.uuid.as_ref()),
                                reason: TaskTerminalReason::TargetLost,
                            },
                        );
                    }
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        row.uuid.as_ref(),
                        TaskLifecycleRequest::Start {
                            slot: transport_slot(row.uuid.as_ref()),
                            target: Some(contact.clone()),
                        },
                    );
                }
                // Accrue and recover. `floor` is IEEE-exact, so two hosts recover
                // the same number of souls on the same tick.
                let mut progress = row.progress + dt / row.seconds_per_civilian;
                let mut recovered_now = 0u32;
                let mut remaining = facts.remaining;
                while progress >= 1.0 && remaining > 0 {
                    progress -= 1.0;
                    remaining -= 1;
                    recovered_now += 1;
                }
                let completed = remaining == 0 && recovered_now > 0;
                if completed {
                    progress = 0.0;
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        row.uuid.as_ref(),
                        TaskLifecycleRequest::End {
                            slot: transport_slot(row.uuid.as_ref()),
                            reason: TaskTerminalReason::Completed,
                        },
                    );
                }
                applied.push(Applied {
                    entity: row.entity,
                    transporting: !completed,
                    active_contact: if completed {
                        None
                    } else {
                        Some(contact.clone())
                    },
                    progress,
                    refusal: None,
                    recover: (recovered_now > 0).then(|| (contact, recovered_now)),
                    completed_id: completed.then(|| facts.entity_id.clone()).flatten(),
                });
            }
            Err(refusal) => {
                // A start refused before anything ran gets a whole activation —
                // a start and its own terminal — for a genuine refusal
                // (NotDiscovered/OutOfRange/Unpowered/Disabled). NoContact and
                // NoCivilians are not tasks that started, so they only set the
                // refusal shown.
                let genuine = !matches!(
                    refusal,
                    TransportRefusal::NoContact | TransportRefusal::NoCivilians
                );
                if genuine {
                    if row.active_contact.is_none() {
                        if let Some(sel) = row.selected.clone() {
                            push_lifecycle(
                                lifecycle.as_deref_mut(),
                                row.uuid.as_ref(),
                                TaskLifecycleRequest::Start {
                                    slot: transport_slot(row.uuid.as_ref()),
                                    target: Some(sel),
                                },
                            );
                        }
                    }
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        row.uuid.as_ref(),
                        TaskLifecycleRequest::End {
                            slot: transport_slot(row.uuid.as_ref()),
                            reason: terminal_reason_for(refusal),
                        },
                    );
                } else if row.active_contact.is_some() {
                    // A running transport that lost its contact/civilians.
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        row.uuid.as_ref(),
                        TaskLifecycleRequest::End {
                            slot: transport_slot(row.uuid.as_ref()),
                            reason: terminal_reason_for(refusal),
                        },
                    );
                }
                applied.push(Applied {
                    entity: row.entity,
                    transporting: false,
                    active_contact: None,
                    progress: 0.0,
                    refusal: Some(refusal),
                    recover: None,
                    completed_id: None,
                });
            }
        }
    }

    // Apply the recovery to the contacts.
    {
        let mut contacts = set.p1();
        for a in &applied {
            if let Some((uuid, n)) = &a.recover {
                if let Some((_, _, mut rescue, _)) =
                    contacts.iter_mut().find(|(u, _, _, _)| &u.0 == uuid)
                {
                    rescue.recovered = (rescue.recovered + n).min(rescue.count);
                }
            }
        }
    }

    // Apply the transporter state.
    {
        let mut transporters = set.p2();
        for a in &applied {
            if let Ok(mut t) = transporters.get_mut(a.entity) {
                t.transporting = a.transporting;
                t.active_contact = a.active_contact.clone();
                t.progress = a.progress;
                t.last_refusal = a.refusal;
            }
        }
    }

    // Raise the "rescue complete" world flag for any contact fully recovered
    // this tick, so an authored `on_flag_set` beat can complete the objective
    // and cue the crew. Mirrors `science::server::mirror_scanned`.
    if let Some(runtime) = runtime.as_deref_mut() {
        let mut completed: Vec<String> = applied
            .iter()
            .filter_map(|a| a.completed_id.clone())
            .collect();
        completed.sort();
        for entity_id in completed {
            let flag = rescue_recovered_flag(&entity_id);
            let (before, after) = runtime.flags.set_flag(&flag);
            if (before != 0) != (after != 0) {
                runtime
                    .pending_world_events
                    .push(crate::world::content::WorldEvent::FlagSet {
                        name: flag,
                        origin_layer: None,
                    });
            }
        }
    }
}

/// The world flag raised when every civilian aboard the contact authored as
/// `[[entity]] id = "<id>"` has been recovered: `rescue.<id>.recovered` (issue
/// #1348).
///
/// A free function rather than an inlined `format!`, for [`scanned_flag`]'s
/// reason: the name is a contract with scenario authors — an
/// `on_flag_set("rescue.<id>.recovered", …)` trigger completes the rescue
/// objective and cues the crew — and a contract stated in two places can be
/// changed in one of them.
pub fn rescue_recovered_flag(entity_id: &str) -> String {
    format!("rescue.{entity_id}.recovered")
}

/// The world flag raised when a contact authored as `[[entity]] id = "<id>"` is
/// destroyed with civilians still aboard: `rescue.<id>.lost` (issue #1348).
///
/// The casualty counterpart of [`rescue_recovered_flag`]: an authored
/// `on_flag_set("rescue.<id>.lost", …)` beat records the loss to the mission
/// report and cues the crew, which is the "human fire records casualty
/// consequences" half of the acceptance criteria — the transporter never writes
/// English, it raises the fact and the scenario authors the consequence.
pub fn rescue_lost_flag(entity_id: &str) -> String {
    format!("rescue.{entity_id}.lost")
}

// ── Backfill rescue AI ───────────────────────────────────────────────────────

/// Backfill Engineering rescue AI (issue #1348).
///
/// On an active `Rescue` directive (Engineering affinity) naming a contact,
/// select it and start the transport; with no such directive active, stop a
/// transport THIS host started. The concrete commands are exactly the ones a
/// human Engineering officer sends, through the SAME `emit_ai_command` →
/// `validate_and_admit` seam (AGENTS.md rule 6).
///
/// Runs INDEPENDENTLY of the tractor's own backfill on the shared Engineering
/// seat: the tractor-versus-rescue priority the mission wants is decided by which
/// objective the scored pool ranks higher — this host serves the top `Rescue`,
/// the tractor host serves the top `Tow`/`Stabilise`/`Escort`, and the seat has
/// one pair of hands, so whichever the scenario scored higher wins. Decides only
/// on the shared AI cadence (rule 7).
#[allow(clippy::type_complexity)]
pub fn operate_transporter_ai(
    mut commands: Commands,
    sessions: Res<crate::lobby::Sessions>,
    runtime: Option<Res<WorldContentRuntime>>,
    mut ships: Query<(
        Entity,
        Option<&EntityUuid>,
        &crate::ship_plugin::ShipSystemControlSources,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        &Transporter,
        &crate::server_app::ShipSystemBlackboards,
        Has<TransporterAiEngaged>,
        &mut crate::core::messages::AdmittedCommands,
    )>,
) {
    for (entity, uuid, sources, config, transporter, blackboards, host_engaged, mut admitted) in
        ships.iter_mut()
    {
        if !sources.0.policy_for(&transporter_system_id()).operate_ai {
            continue;
        }
        let directive_target: Option<String> = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(vbb)) => crate::objectives::top_operate_directive(
                &vbb.scored_objectives,
                SystemAffinity::Engineering,
                |d| crate::objectives::rescue_directive_target(d).is_some(),
            )
            .and_then(crate::objectives::rescue_directive_target)
            .map(str::to_string),
            _ => None,
        };

        // The commands to emit this tick, in order.
        let mut payloads: Vec<SystemControlPayload> = Vec::new();
        match directive_target {
            Some(name) => {
                let resolved = resolve_directive_target(&name, runtime.as_deref());
                let selected_on_target =
                    transporter.selected_contact.as_deref() == Some(resolved.as_str());
                if !selected_on_target {
                    payloads.push(SystemControlPayload::TransportSelectContact {
                        uuid: resolved.clone(),
                    });
                }
                if !transporter.transporting {
                    payloads.push(SystemControlPayload::StartTransport);
                }
                if (!payloads.is_empty() || transporter.transporting) && !host_engaged {
                    commands.entity(entity).insert(TransporterAiEngaged);
                }
            }
            None => {
                if transporter.transporting && host_engaged {
                    commands.entity(entity).remove::<TransporterAiEngaged>();
                    payloads.push(SystemControlPayload::StopTransport);
                }
            }
        }

        for payload in payloads {
            emit_ai_command(
                uuid,
                transporter_system_id(),
                payload,
                sources,
                &sessions,
                config,
                &mut admitted,
            );
        }
    }
}

/// Resolve a directive's named target to a UUID: a world entity NAME through the
/// runtime map, or the value itself when it is already a UUID (or the runtime is
/// absent, as in fixtures).
fn resolve_directive_target(name: &str, runtime: Option<&WorldContentRuntime>) -> String {
    runtime
        .and_then(|rt| rt.name_to_uuid.get(name).cloned())
        .unwrap_or_else(|| name.to_string())
}

// ── Casualty attribution ─────────────────────────────────────────────────────

/// Record a civilian casualty when a carrier is destroyed with civilians still
/// aboard (issue #1348).
///
/// Rebuilds the [`CivilianRescueLedger`] from the live contacts each tick — so a
/// carrier destroyed by fire (its entity despawned by the combat systems this
/// tick) still has its last-known civilian count on record — then reads the
/// destruction events and, for every victim that was a carrier with unrecovered
/// souls, raises the [`rescue_lost_flag`]. The mission report row and the crew
/// cue are authored off that flag, so this system writes no English and no
/// authoritative gameplay branches on the ledger.
///
/// The exclusion of civilian carriers from Tactical Backfill (in
/// `console::weapons::server::ai_target_selection`) means only HUMAN fire reaches
/// here — which is exactly the acceptance criterion: the AI never shoots the
/// civilians, a human still can, and when they do it is recorded.
pub fn record_civilian_casualties(
    mut ledger: ResMut<CivilianRescueLedger>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    mut destroyed: MessageReader<crate::ai::server::AiEntityDestroyed>,
    contacts: Query<(&EntityUuid, Option<&EntityId>, &CivilianRescue)>,
) {
    // Refresh last-known counts. Entries for despawned carriers deliberately
    // linger, so a destruction this tick still finds one.
    for (uuid, id, rescue) in contacts.iter() {
        ledger.entries.insert(
            uuid.0.clone(),
            (
                id.map(|i| i.0.clone()).unwrap_or_default(),
                rescue.remaining(),
            ),
        );
    }

    // Which destroyed victims are unrecorded carriers with souls still aboard.
    let mut casualties: Vec<(String, String)> = Vec::new();
    for event in destroyed.read() {
        let victim = &event.entity_uuid;
        if ledger.recorded.contains(victim) {
            continue;
        }
        if let Some((entity_id, remaining)) = ledger.entries.get(victim) {
            if *remaining > 0 && !entity_id.is_empty() {
                casualties.push((victim.clone(), entity_id.clone()));
            }
        }
    }
    casualties.sort();
    casualties.dedup();

    let Some(runtime) = runtime.as_deref_mut() else {
        // No world runtime (a bare fixture): still mark recorded so the same
        // event is not reconsidered, but there is no flag store to raise into.
        for (victim, _) in casualties {
            ledger.recorded.insert(victim);
        }
        return;
    };
    for (victim, entity_id) in casualties {
        let flag = rescue_lost_flag(&entity_id);
        let (before, after) = runtime.flags.set_flag(&flag);
        if (before != 0) != (after != 0) {
            runtime
                .pending_world_events
                .push(crate::world::content::WorldEvent::FlagSet {
                    name: flag,
                    origin_layer: None,
                });
        }
        ledger.recorded.insert(victim);
    }
}

// ── The wire ─────────────────────────────────────────────────────────────────

/// Publish each transporter-carrying ship's blackboard under its system id
/// (issue #1348).
pub fn publish_transporter_blackboard(
    mut ships: Query<(&Transporter, &mut crate::server_app::ShipSystemBlackboards)>,
    contacts: Query<(&EntityUuid, Option<&EntityName>, &CivilianRescue)>,
) {
    let key = transporter_system_id();
    for (transporter, mut blackboards) in ships.iter_mut() {
        let (name, discovered, remaining) = match transporter.selected_contact.as_deref() {
            Some(sel) => contacts
                .iter()
                .find(|(u, _, _)| u.0 == sel)
                .map(|(_, n, r)| (n.map(|n| n.0.clone()), r.revealed, r.remaining()))
                .unwrap_or((None, false, 0)),
            None => (None, false, 0),
        };
        let blackboard = SystemBlackboard::Transporter(TransporterBlackboard {
            range: transporter.config.range,
            selected_contact: transporter.selected_contact.clone(),
            selected_contact_name: name,
            discovered,
            transporting: transporter.transporting,
            civilians_remaining: remaining,
            progress: transporter.progress,
            refusal: transporter.last_refusal.map(|r| r.string_id().to_string()),
        });
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key.clone(), blackboard);
        }
    }
}

/// The transporter's published blackboard channel key — its system id.
pub fn transporter_blackboard_key() -> SystemId {
    transporter_system_id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{AdmittedCommand, AdmittedCommands};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;

    fn transporter() -> Transporter {
        Transporter::new(
            TransporterConfig {
                range: 500.0,
                seconds_per_civilian: 2.0,
                min_power_level: 0,
            },
            PowerGroupId("helm".into()),
        )
    }

    fn admitted(payload: SystemControlPayload) -> AdmittedCommands {
        AdmittedCommands(vec![AdmittedCommand {
            target: transporter_system_id(),
            payload,
            response_token: None,
        }])
    }

    fn queued(world: &mut World) -> Vec<TaskLifecycleRequest> {
        std::mem::take(&mut world.resource_mut::<EffectQueue<TaskLifecycleRequest>>().0)
    }

    /// One operator with a discovered, in-range contact carrying `count`
    /// civilians, and the lifecycle queue the systems push onto. Time advances
    /// one whole `seconds_per_civilian` per fixed tick, so exactly one civilian
    /// is recovered per `tick_transport`.
    fn rescue_world(count: u32) -> (World, Entity, Entity) {
        let mut world = World::new();
        world.init_resource::<EffectQueue<TaskLifecycleRequest>>();
        // A fixed clock whose delta is exactly one civilian's worth of time.
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs(2));
        world.insert_resource(time);
        let operator = world
            .spawn((
                EntityUuid("uuid-destroyer".into()),
                transporter(),
                Transform::default(),
                admitted(SystemControlPayload::StartTransport),
            ))
            .id();
        let mut rescue = CivilianRescue::new(count);
        rescue.revealed = true;
        let contact = world
            .spawn((
                EntityUuid("uuid-lighter".into()),
                Transform::from_xyz(100.0, 0.0, 0.0),
                rescue,
            ))
            .id();
        (world, operator, contact)
    }

    fn select_and_start(world: &mut World) {
        let op = world_operator(world);
        world
            .entity_mut(op)
            .insert(admitted(SystemControlPayload::TransportSelectContact {
                uuid: "uuid-lighter".into(),
            }));
        world
            .run_system_once(handle_transporter_commands)
            .expect("select applies");
        world
            .entity_mut(op)
            .insert(admitted(SystemControlPayload::StartTransport));
        world
            .run_system_once(handle_transporter_commands)
            .expect("start applies");
    }

    fn world_operator(world: &mut World) -> Entity {
        let mut q = world.query_filtered::<Entity, With<Transporter>>();
        q.iter(world).next().expect("one operator")
    }

    #[test]
    fn a_discovered_in_range_contact_is_recovered_over_ticks_and_opens_one_activation() {
        let (mut world, operator, contact) = rescue_world(2);
        select_and_start(&mut world);
        // Draining the select's own nothing.
        let _ = queued(&mut world);

        // Tick 1: the transport begins running — a Start beat — and recovers the
        // first civilian.
        world
            .run_system_once(tick_transport)
            .expect("verdict applies");
        assert_eq!(
            queued(&mut world),
            vec![TaskLifecycleRequest::Start {
                slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
                target: Some("uuid-lighter".into()),
            }],
        );
        assert_eq!(world.get::<CivilianRescue>(contact).unwrap().recovered, 1);
        assert!(world.get::<Transporter>(operator).unwrap().transporting);

        // Tick 2: the last civilian is recovered and the activation completes.
        world
            .run_system_once(tick_transport)
            .expect("verdict applies");
        assert_eq!(
            queued(&mut world),
            vec![TaskLifecycleRequest::End {
                slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
                reason: TaskTerminalReason::Completed,
            }],
        );
        let rescue = world.get::<CivilianRescue>(contact).unwrap();
        assert_eq!(rescue.recovered, 2);
        assert_eq!(rescue.remaining(), 0);
        assert!(!world.get::<Transporter>(operator).unwrap().transporting);
    }

    #[test]
    fn an_undiscovered_contact_is_refused_not_discovered() {
        let (mut world, operator, contact) = rescue_world(2);
        world.get_mut::<CivilianRescue>(contact).unwrap().revealed = false;
        select_and_start(&mut world);
        let _ = queued(&mut world);

        world
            .run_system_once(tick_transport)
            .expect("verdict applies");
        // A refused start is a beat — a Start and its own terminal.
        assert_eq!(
            queued(&mut world),
            vec![
                TaskLifecycleRequest::Start {
                    slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
                    target: Some("uuid-lighter".into()),
                },
                TaskLifecycleRequest::End {
                    slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
                    reason: TaskTerminalReason::Unreadable,
                },
            ],
        );
        assert_eq!(world.get::<CivilianRescue>(contact).unwrap().recovered, 0);
        assert_eq!(
            world.get::<Transporter>(operator).unwrap().last_refusal,
            Some(TransportRefusal::NotDiscovered)
        );
    }

    #[test]
    fn a_contact_out_of_range_is_refused_out_of_range() {
        let (mut world, operator, contact) = rescue_world(2);
        world
            .entity_mut(contact)
            .insert(Transform::from_xyz(5_000.0, 0.0, 0.0));
        select_and_start(&mut world);
        let _ = queued(&mut world);

        world
            .run_system_once(tick_transport)
            .expect("verdict applies");
        assert_eq!(
            world.get::<Transporter>(operator).unwrap().last_refusal,
            Some(TransportRefusal::OutOfRange)
        );
        assert_eq!(world.get::<CivilianRescue>(contact).unwrap().recovered, 0);
    }

    #[test]
    fn stopping_a_running_transport_reports_a_release() {
        let (mut world, _operator, _contact) = rescue_world(3);
        select_and_start(&mut world);
        let _ = queued(&mut world);
        world.run_system_once(tick_transport).expect("runs");
        let _ = queued(&mut world);

        // Stop it mid-recovery.
        let op = world_operator(&mut world);
        world
            .entity_mut(op)
            .insert(admitted(SystemControlPayload::StopTransport));
        world
            .run_system_once(handle_transporter_commands)
            .expect("stop applies");
        assert_eq!(
            queued(&mut world),
            vec![TaskLifecycleRequest::End {
                slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
                reason: TaskTerminalReason::Released,
            }],
        );
    }

    #[test]
    fn destroying_a_carrier_with_civilians_aboard_raises_the_lost_flag() {
        use crate::ai::server::AiEntityDestroyed;
        use bevy::ecs::message::Messages;

        let mut world = World::new();
        world.init_resource::<CivilianRescueLedger>();
        world.init_resource::<WorldContentRuntime>();
        world.init_resource::<Messages<AiEntityDestroyed>>();
        // A carrier with two of its four civilians still aboard.
        let mut rescue = CivilianRescue::new(4);
        rescue.recovered = 2;
        world.spawn((
            EntityUuid("uuid-lighter".into()),
            EntityId("lighter".into()),
            rescue,
        ));
        // Fire destroys it — the balance/AI death event both combat paths write.
        world
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: "uuid-lighter".into(),
            });

        world
            .run_system_once(record_civilian_casualties)
            .expect("the casualty system runs");

        assert!(
            world
                .resource::<WorldContentRuntime>()
                .flags
                .counter(&rescue_lost_flag("lighter"))
                > 0,
            "a carrier lost with civilians aboard raises rescue.<id>.lost"
        );
    }

    #[test]
    fn destroying_a_fully_recovered_carrier_raises_no_casualty() {
        use crate::ai::server::AiEntityDestroyed;
        use bevy::ecs::message::Messages;

        let mut world = World::new();
        world.init_resource::<CivilianRescueLedger>();
        world.init_resource::<WorldContentRuntime>();
        world.init_resource::<Messages<AiEntityDestroyed>>();
        // Everyone already aboard.
        let mut rescue = CivilianRescue::new(3);
        rescue.recovered = 3;
        world.spawn((
            EntityUuid("uuid-lighter".into()),
            EntityId("lighter".into()),
            rescue,
        ));
        world
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: "uuid-lighter".into(),
            });

        world
            .run_system_once(record_civilian_casualties)
            .expect("the casualty system runs");

        assert_eq!(
            world
                .resource::<WorldContentRuntime>()
                .flags
                .counter(&rescue_lost_flag("lighter")),
            0,
            "an empty hulk is not a casualty — everyone was already recovered"
        );
    }

    #[test]
    fn selecting_a_contact_resets_progress_and_intent() {
        let mut world = World::new();
        world.init_resource::<EffectQueue<TaskLifecycleRequest>>();
        let operator = world
            .spawn((
                EntityUuid("uuid-destroyer".into()),
                transporter(),
                Transform::default(),
                admitted(SystemControlPayload::TransportSelectContact {
                    uuid: "uuid-lighter".into(),
                }),
            ))
            .id();
        world
            .run_system_once(handle_transporter_commands)
            .expect("select applies");
        let t = world.get::<Transporter>(operator).unwrap();
        assert_eq!(t.selected_contact.as_deref(), Some("uuid-lighter"));
        assert!(!t.transporting);
        assert_eq!(t.progress, 0.0);
    }
}
