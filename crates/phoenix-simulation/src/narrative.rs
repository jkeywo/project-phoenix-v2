//! The narrative-event emitters (issue #1338, PRD #1337).
//!
//! [`crate::core::narrative`] owns the vocabulary — what a mission-timeline
//! event IS. This module owns the two systems that produce them (the third
//! producer, the Comms pair, lives with the Comms console it reads), and it
//! lives at the crate root rather than under `core` because both read scenario
//! state (`world::server`) that `core` must not depend on.
//!
//! # How each producer observes what it reports
//!
//! [`crate::core::balance::BalanceEvent`] is written *at* its chokepoints, and
//! that is right for combat: a hit has exactly one site, and the site knows the
//! attacker. Objective, deadline and marked-entity outcomes are not like that.
//!
//! * An Objective can be completed from a trigger action, a Rhai effect, a
//!   comms response, or the Helm AI satisfying its own directive
//!   (`ship::helm_ai`) — four call sites today, and the count has only ever
//!   gone up. Threading a message writer through all of them would put the
//!   timeline's completeness at the mercy of the next one added. So the
//!   *manager* logs instead: every one of those sites goes through
//!   [`crate::objectives::ObjectiveManager`]'s four mutators, each of which
//!   appends an [`crate::objectives::ObjectiveTransition`], and
//!   [`emit_scenario_narrative`] drains that log once a tick.
//! * The systems that own those sites are at Bevy's 16-parameter limit already,
//!   which is why `ScriptedCommsAux` / `CommsRespondAux` exist at all.
//!
//! A per-tick LOG rather than a per-tick diff, because a status field can only
//! ever report the state a tick ended in: an objective posted and completed
//! inside one tick is one field and two beats, and a diff would report only the
//! completion (issue #1338 review). The diff survives as a *backstop* — it
//! still runs, and reports anything whose status moved without going through
//! the manager's own API — and the `Local` it diffs against is deliberately NOT
//! a resource: it is derived, non-authoritative, and keeping it out of the world
//! keeps it out of the #894 census and the snapshot both.
//!
//! Deadlines have no such API to log through — a `DeadlineRecord`'s `state` is
//! set by the world's own tick — so that half stays a pure diff over the
//! ordered `DeadlineTable::records`, and a deadline cannot fire twice in a tick.
//!
//! Authored beats and authored entity outcomes DO have one true chokepoint —
//! the script boundary — so they ride the #1223 effect-queue pattern instead:
//! `world::server::apply_dispatch_result` pushes a
//! [`NarrativeRequest`](crate::core::narrative::NarrativeRequest) and
//! [`emit_authored_and_marked_entity_narrative`] turns it into an event. That
//! same queue carries the scripted-removal signal, which is why the authored
//! queue and the marked-entity lifecycle are ONE system: deciding whether a
//! scripted removal deserves a death beat needs the mark memory, the authored
//! outcomes and the combat deaths in the same place.
//!
//! # Determinism
//!
//! Every emitter reads state the fixed tick already decided and writes only
//! `Messages<NarrativeEvent>`, which nothing authoritative reads. Iteration
//! order is fixed at every site: the objective log and the effect queue are
//! drained front-to-back, the deadline diff walks an ordered `Vec`, and the
//! marked entity pass sorts its query results by authored id before emitting.
//! No wall clock, no RNG, no `HashMap` walk.

use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::core::balance::BalanceEvent;
use crate::core::computer_message::{ActiveComputerMessage, ComputerMessageRequest};
use crate::core::messages::ObjectiveStatus;
use crate::core::messages::{GamePhase, SystemId};
use crate::core::narrative::{
    NarrativeActor, NarrativeEvent, NarrativeKind, NarrativeMark, NarrativeRequest, NarrativeValue,
};
use crate::core::task_lifecycle::{
    repeats_event, start_event, terminal_event, TaskActivation, TaskLifecycleRequest,
    TaskLifecycles, TaskSlot, TaskTerminalReason,
};
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::EntityUuid;
use crate::objectives::{ObjectiveTransition, ObjectiveTransitionKind};
use crate::ship::components::{HumanSeekingHosts, ShipConfigComponent};
use crate::sim_tick::SimTick;
use crate::world::config::WorldConfig;
use crate::world::deadlines::DeadlineState;
use crate::world::script::schedule::SchedClock;
use crate::world::server::{ObjectiveManagerRes, WorldContentRuntime};

/// The narrative kind an objective's status maps to when it is first seen, or
/// when it changes. `Active` is only a *transition* to report on the tick the
/// objective appears — an objective that stays Active reports nothing.
fn kind_for_status(status: &ObjectiveStatus) -> NarrativeKind {
    match status {
        ObjectiveStatus::Active => NarrativeKind::ObjectivePosted,
        ObjectiveStatus::Completed => NarrativeKind::ObjectiveCompleted,
        ObjectiveStatus::Failed => NarrativeKind::ObjectiveFailed,
    }
}

/// The narrative kind one logged transition reports, or `None` when the
/// transition is bookkeeping rather than story.
///
/// A REMOVED objective (a layer unloading its own, `ObjectiveManager::remove`)
/// is the only such case: nothing happened in the story, the scenario simply
/// took its bookkeeping back. It still has to be *seen* here, so the diff
/// backstop's view drops it.
fn kind_for_transition(kind: ObjectiveTransitionKind) -> Option<NarrativeKind> {
    match kind {
        ObjectiveTransitionKind::Posted => Some(NarrativeKind::ObjectivePosted),
        ObjectiveTransitionKind::Completed => Some(NarrativeKind::ObjectiveCompleted),
        ObjectiveTransitionKind::Failed => Some(NarrativeKind::ObjectiveFailed),
        ObjectiveTransitionKind::Removed => None,
    }
}

/// The status an objective is left in by one logged transition, or `None` when
/// the record is gone.
fn status_after(kind: ObjectiveTransitionKind) -> Option<ObjectiveStatus> {
    match kind {
        ObjectiveTransitionKind::Posted => Some(ObjectiveStatus::Active),
        ObjectiveTransitionKind::Completed => Some(ObjectiveStatus::Completed),
        ObjectiveTransitionKind::Failed => Some(ObjectiveStatus::Failed),
        ObjectiveTransitionKind::Removed => None,
    }
}

/// One objective beat, carrying the objective's `strings.csv` text id verbatim —
/// AGENTS.md rule 11 and PRD #1337's "String Ids, not prose".
fn objective_event(
    kind: NarrativeKind,
    id: &str,
    text: &str,
    mandatory: bool,
    targets: &[String],
) -> NarrativeEvent {
    let mut event = NarrativeEvent::new(kind, id)
        .text("text", text)
        .detail("mandatory", NarrativeValue::Flag(mandatory));
    if let Some(first) = targets.first() {
        event = event.to_target(first.clone());
    }
    event
}

/// Emit Objective and deadline transitions.
///
/// One system for both because they are the same shape and the same argument:
/// ordered authored records whose *status* is the story. Each `Local` map is
/// this system's private previous-tick view; neither is world state.
///
/// Objectives are read from [`crate::objectives::ObjectiveManager`]'s own
/// transition log, drained here, so every authored transition produces its own
/// event however many of them share a tick — an objective posted and completed
/// on one tick reports both, in the order the tick made them. See this module's
/// docs for why the log rather than the diff is primary.
///
/// The diff over `sorted_snapshots` still runs afterwards, as a BACKSTOP: it
/// reports anything whose status moved without passing through the manager's
/// API (a wholesale resource replacement, a future direct mutation), and its
/// rebuilt view is what drops removed objectives.
///
/// Deadlines have no equivalent API to log through, so that half is the pure
/// diff it always was.
pub fn emit_scenario_narrative(
    objectives: Option<ResMut<ObjectiveManagerRes>>,
    objective_instances: Option<ResMut<crate::world::server::ObjectiveInstanceManagerRes>>,
    runtime: Option<Res<WorldContentRuntime>>,
    mut seen_objectives: Local<BTreeMap<String, ObjectiveStatus>>,
    mut seen_deadlines: Local<BTreeMap<String, DeadlineState>>,
    mut out: MessageWriter<NarrativeEvent>,
) {
    let mut instanced_definitions = BTreeSet::new();
    if let Some(mut instances) = objective_instances {
        instanced_definitions.extend(
            instances
                .0
                .records()
                .iter()
                .map(|record| record.spec.key.objective_id.clone()),
        );
        let transitions = instances.bypass_change_detection().0.drain_transitions();
        for transition in transitions {
            let definition = objectives.as_ref().and_then(|objectives| {
                objectives
                    .0
                    .sorted_snapshots()
                    .into_iter()
                    .find(|row| row.id == transition.key.objective_id)
            });
            let Some(definition) = definition else {
                continue;
            };
            let kind = kind_for_status(&transition.status);
            let mut event = objective_event(
                kind,
                &format!(
                    "{}::{}",
                    transition.key.objective_id, transition.key.instance_id
                ),
                &definition.text,
                definition.mandatory,
                &definition.targets,
            )
            .detail(
                "objective_id",
                NarrativeValue::Text(transition.key.objective_id),
            )
            .detail(
                "instance_id",
                NarrativeValue::Text(transition.key.instance_id),
            );
            if transition.status == ObjectiveStatus::Completed {
                event = event.detail(
                    "completion_members",
                    NarrativeValue::TextList(transition.completion_members),
                );
            }
            out.write(event);
        }
    }
    if let Some(mut objectives) = objectives {
        // Restoration is history, not a new fictional event. Rebase before
        // draining the log: genuine transitions since the restore still follow
        // that baseline and are reported normally on this same continuation.
        if let Some(restored) = objectives
            .bypass_change_detection()
            .0
            .take_restored_statuses()
        {
            *seen_objectives = restored;
        }
        // ── Primary: the authored log, in the order the tick made it ──────────
        //
        // `bypass_change_detection` because draining the log changes nothing any
        // other reader can see — the objective set itself is untouched — and a
        // recorder that flagged the authoritative resource as mutated every
        // single tick would make `Changed<ObjectiveManagerRes>` useless to
        // whoever wants it next.
        let transitions: Vec<ObjectiveTransition> =
            objectives.bypass_change_detection().0.drain_transitions();
        for transition in transitions {
            if !instanced_definitions.contains(&transition.id) {
                if let Some(kind) = kind_for_transition(transition.kind) {
                    out.write(objective_event(
                        kind,
                        &transition.id,
                        &transition.text,
                        transition.mandatory,
                        &transition.targets,
                    ));
                }
            }
            // Keep the backstop's view in step, so a transition reported here is
            // not reported a second time by the diff below.
            match status_after(transition.kind) {
                Some(status) => {
                    seen_objectives.insert(transition.id, status);
                }
                None => {
                    seen_objectives.remove(&transition.id);
                }
            }
        }

        // ── Backstop: the diff, for state that moved outside the API ──────────
        //
        // `sorted_snapshots` is mandatory-first then authored order — a total
        // order that does not depend on when each objective was added.
        let snapshots = objectives.0.sorted_snapshots();
        let mut live: BTreeMap<String, ObjectiveStatus> = BTreeMap::new();
        for snap in &snapshots {
            live.insert(snap.id.clone(), snap.status.clone());
            let changed = match seen_objectives.get(&snap.id) {
                None => true,
                Some(prev) => prev != &snap.status,
            };
            if !changed {
                continue;
            }
            if instanced_definitions.contains(&snap.id) {
                continue;
            }
            out.write(objective_event(
                kind_for_status(&snap.status),
                &snap.id,
                &snap.text,
                snap.mandatory,
                &snap.targets,
            ));
        }
        // Rebuild rather than merge, so a removed objective leaves the view.
        *seen_objectives = live;
    }

    if let Some(runtime) = runtime.as_deref() {
        let mut live: BTreeMap<String, DeadlineState> = BTreeMap::new();
        for record in &runtime.deadlines.records {
            let key = record.presentation_id();
            live.insert(key.clone(), record.state);
            let previously = seen_deadlines.get(&key).copied();
            // Only ONE deadline transition is a story beat: firing. Arming is
            // the world loading, and a cancellation is the scenario deciding
            // the beat will not happen — the authored `narrative_beat` call is
            // where an author says that mattered.
            if record.state == DeadlineState::Fired && previously != Some(DeadlineState::Fired) {
                out.write(
                    NarrativeEvent::new(NarrativeKind::DeadlineFired, key.clone())
                        .text("label", record.label.clone())
                        .detail("due_tick", NarrativeValue::Int(record.due_tick as i64)),
                );
            }
        }
        *seen_deadlines = live;
    }
}

/// Emit marked-entity spawn and death, and drain the authored request queue.
///
/// # The three inputs, and why they are one system
///
/// * `known` is uuid → authored narrative id, and it is what makes death
///   reportable at all: by the time anything can observe a destroyed ship it has
///   been despawned, so the mark has to have been remembered while it was alive.
/// * `fates` is the set of authored ids whose outcome this run has already
///   recorded — an authored `escaped`/`rescued`/…, or a death already emitted.
/// * The queue carries both the author's own declarations
///   ([`NarrativeRequest::Authored`]) and the scripted-removal signal
///   ([`NarrativeRequest::ScriptedRemoval`]).
///
/// Deciding whether a scripted removal deserves a death beat needs all three,
/// so they live together rather than in a system each. Both `Local`s are
/// `Local`s rather than resources for the reason [`emit_scenario_narrative`]'s
/// are — see this module's docs.
///
/// # The two death paths
///
/// A combat kill rides `BalanceEvent::EntityDestroyed`: that event is already
/// emitted exactly once per death, at the kill site, carrying the killer
/// credit. Reading it here is not "inferring a story event from a shot" — the
/// gate is the authored [`NarrativeMark`], and an unmarked hull dying produces
/// nothing.
///
/// A SCRIPTED removal (`ctx.effects.destroy_entity(name)`) writes no balance
/// event, and must not: an authorial removal is not a combat kill, and putting
/// one in the ledger would make a rescue-by-despawn count as a destruction in
/// the AAR. So the applier queues a [`NarrativeRequest::ScriptedRemoval`]
/// instead, and this system turns it into a death beat ONLY when the marked
/// entity has no recorded fate — an author who removed the hull to say
/// "rescued" says so with `ctx.effects.narrative_outcome(..)` before or on the
/// same tick, and that statement stands alone. Everything else stays silent:
/// unmarked hulls, and entities whose fate is already on the timeline.
///
/// The queue is drained in full every tick (`std::mem::take`), front to back,
/// so it is structurally empty at the fold point — the `ClearedAtFold` contract
/// every [`EffectQueue`] carries.
pub fn emit_authored_and_marked_entity_narrative(
    marks: Query<(&EntityUuid, &NarrativeMark)>,
    queue: Option<ResMut<EffectQueue<NarrativeRequest>>>,
    mut known: Local<BTreeMap<String, String>>,
    mut fates: Local<BTreeSet<String>>,
    mut balance: MessageReader<BalanceEvent>,
    mut out: MessageWriter<NarrativeEvent>,
) {
    // Newly marked entities, sorted by authored id so the emission order does
    // not depend on archetype layout.
    let mut fresh: Vec<(String, String)> = marks
        .iter()
        .filter(|(uuid, _)| !known.contains_key(&uuid.0))
        .map(|(uuid, mark)| (mark.0.clone(), uuid.0.clone()))
        .collect();
    fresh.sort();
    for (narrative_id, uuid) in fresh {
        known.insert(uuid.clone(), narrative_id.clone());
        out.write(
            NarrativeEvent::new(NarrativeKind::MarkedEntitySpawned, narrative_id)
                .from_actor(NarrativeActor::entity(uuid)),
        );
    }

    for event in balance.read() {
        let BalanceEvent::EntityDestroyed { victim, killer } = event else {
            continue;
        };
        let Some(narrative_id) = known.get(victim).cloned() else {
            continue;
        };
        // A real death is always reported, whatever else the author said — but
        // it is recorded as this entity's fate, so a scripted removal that
        // tidies the wreck away afterwards does not report a second death.
        fates.insert(narrative_id.clone());
        let mut ev = NarrativeEvent::new(NarrativeKind::MarkedEntityDestroyed, narrative_id)
            .from_actor(NarrativeActor::entity(victim.clone()));
        if let Some(killer) = killer {
            ev = ev.to_target(killer.clone());
        }
        out.write(ev);
    }

    let Some(mut queue) = queue else {
        return;
    };
    if queue.0.is_empty() {
        return;
    }
    let batch = std::mem::take(&mut queue.0);
    // Pass one: every authored outcome in this batch counts, wherever it sits in
    // it. That is what makes the contract "before OR on the same tick as the
    // destroy" true regardless of the order a handler happened to make the two
    // calls in — the tick is the unit, not the queue index.
    for request in &batch {
        if let NarrativeRequest::Authored { kind, id, .. } = request {
            if kind.is_marked_entity_outcome() {
                fates.insert(id.clone());
            }
        }
    }
    // Pass two: emit, in queue order.
    for request in batch {
        match request {
            NarrativeRequest::Authored {
                kind,
                id,
                entity_uuid,
            } => {
                let mut event = NarrativeEvent::new(kind, id);
                if let Some(uuid) = entity_uuid {
                    event = event.from_actor(NarrativeActor::entity(uuid));
                }
                out.write(event);
            }
            NarrativeRequest::ScriptedRemoval { entity_uuid } => {
                // Marking is the gate, exactly as it is for a combat kill: an
                // unmarked hull a script removes produces nothing.
                let Some(narrative_id) = known.get(&entity_uuid).cloned() else {
                    continue;
                };
                // `insert` returns false when the fate was already recorded —
                // an authored outcome (this tick or any earlier one), a combat
                // death, or a previous removal of the same hull. Either way the
                // story has already said what became of it.
                if !fates.insert(narrative_id.clone()) {
                    continue;
                }
                out.write(
                    NarrativeEvent::new(NarrativeKind::MarkedEntityDestroyed, narrative_id)
                        .from_actor(NarrativeActor::entity(entity_uuid)),
                );
            }
        }
    }
}

/// Apply this tick's `show_message(..)` requests to the authoritative
/// [`ActiveComputerMessage`], check its simulation-time expiry, and emit the
/// `shown`/`superseded`/`expired` narrative trio (issue #1342).
///
/// One system for the whole state machine, for the same reason
/// [`emit_authored_and_marked_entity_narrative`] combines its three inputs:
/// deciding whether a `show` superseded a prior message needs the resource's
/// state at the moment it changes, and only the system holding the `ResMut`
/// can report that honestly. The queue is drained in full
/// (`std::mem::take`), front to back, so a scenario that (mis)authors two
/// `show_message` calls in one tick still produces a coherent
/// shown/superseded pair for each.
///
/// Expiry is checked AFTER the queue drains, on whatever is `current` once
/// every request this tick has applied — so a message shown this same tick
/// can never be reported expired on the tick it was shown (its `expires_tick`
/// is always at least one tick out; see
/// [`crate::core::computer_message::ActiveComputerMessage::show`]).
pub fn tick_computer_message(
    mut active: ResMut<ActiveComputerMessage>,
    queue: Option<ResMut<EffectQueue<ComputerMessageRequest>>>,
    sim_tick: Res<SimTick>,
    world_config: Option<Res<WorldConfig>>,
    mut out: MessageWriter<NarrativeEvent>,
) {
    let now_tick = sim_tick.0;
    let tick_hz = world_config
        .as_deref()
        .map_or(SchedClock::ZERO.tick_hz, |wc| wc.global.sim_tick_hz);

    if let Some(mut queue) = queue {
        if !queue.0.is_empty() {
            for request in std::mem::take(&mut queue.0) {
                let new_id = request.id.clone();
                if let Some(superseded) = active.show(&request, now_tick, tick_hz) {
                    out.write(
                        NarrativeEvent::new(NarrativeKind::ComputerMessageCleared, superseded.id)
                            .text("reason", "superseded")
                            .text("superseded_by", new_id.clone()),
                    );
                }
                let mut event = NarrativeEvent::new(NarrativeKind::ComputerMessagePosted, new_id)
                    .text("text", request.text)
                    .text("severity", request.severity.as_str())
                    .detail("duration_secs", NarrativeValue::Int(request.duration_secs));
                if let Some(station) = &request.station {
                    event = event.text("station", station.0.clone());
                }
                out.write(event);
            }
        }
    }

    if let Some(expired_id) = active.expire_if_due(now_tick) {
        out.write(
            NarrativeEvent::new(NarrativeKind::ComputerMessageCleared, expired_id)
                .text("reason", "expired"),
        );
    }
}

/// Unconditionally clear the active ship's-computer message. Registered on
/// `OnEnter(GamePhase::GameOver)` and `OnEnter(GamePhase::Lobby)` (issue
/// #1342): the two transitions the issue names, neither of which is itself a
/// narrative beat (see [`ActiveComputerMessage::clear`]'s doc for why).
pub fn clear_active_computer_message(mut active: ResMut<ActiveComputerMessage>) {
    active.clear();
}

// ── The continuous-task lifecycle (issue #1341) ──────────────────────────────

/// Turn the queued task-lifecycle reports into their narrative beats, holding
/// the one-start-one-terminal contract (issue #1341).
///
/// # Why the sites report and this system decides
///
/// The three reporting sites — `tractor::server::handle_tractor_commands`,
/// `tractor::server::tick_tractor`, `science::server::tick_scans`, and the AI
/// host `tractor::server::operate_tractor_ai` — each know one fact: work began,
/// or work ended for this reason. None of them knows the activation's ordinal,
/// the station the system belongs to, whether the subject is still in the world,
/// or whether some other site has already reported the same ending. Putting
/// those four decisions here means a later mechanic (#1345, #1346, #1348, #1350)
/// reports the same two facts and inherits all four.
///
/// # The five decisions
///
/// 1. **Ordinal.** [`TaskLifecycles::begin`] mints it per slot, so a repeat of
///    the same work is a new key rather than a second event on the old one.
/// 2. **Station.** Resolved through `command_admission::policy::
///    station_for_system` off the operator's own config, so the beat names the
///    station a human WOULD be sitting at, human or AI (AGENTS.md rule 6). An
///    unresolvable station stays `None` rather than being the system id cast to
///    a station — the rule the Comms beat keeps.
/// 3. **A vanished subject.** A hold whose target was destroyed reports itself
///    as out of range or as having lost its lock, because that is all
///    `hold_status` can see. Any reason [`TaskTerminalReason::masks_target_loss`]
///    admits is upgraded to [`TaskTerminalReason::TargetDestroyed`] when the
///    subject really has left the world — and a still-live activation whose
///    subject vanished is closed here even if no site reports it at all, which
///    is what covers a scripted removal.
/// 4. **Exactly one terminal.** A terminal report for a slot with no live
///    activation is dropped. That is what lets `operate_tractor_ai` report an
///    `OrderWithdrawn` and `handle_tractor_commands` report the `Released` it
///    caused, on the same tick, and have the timeline record the truer of the
///    two — the first one through the queue, which is the AI host, because it is
///    ordered before the command handler.
/// 5. **A poll is bounded, not erased.** An AI host serving a standing objective
///    re-issues its request every authored snapshot until the objective is
///    satisfied — the Sensors host's Scan says so in as many words — so an order
///    that can never be fulfilled is refused again on every cadence. An
///    instantaneous activation that repeats, exactly, the failure its slot last
///    recorded does not get its own pair of beats (see
///    [`TaskLifecycles::repeats_last_failure`]); it is COUNTED against the
///    retained terminal instead, and the count is written once — as a
///    `task_repeated` census beat carrying that terminal's key — when the
///    standing failure ends or the run does. That keeps a standing unfulfillable
///    order at O(1) beats rather than a rate, while leaving the repeats
///    identifiable, which is what AC3 asks for: this rule cannot tell an AI
///    cadence retry from an engineer pressing Engage a second time, so it must
///    not make either of them vanish. The moment anything about it changes — the
///    subject, the reason, or the fact that it now works — it is a full beat
///    again, and the repeats it had accumulated are reported first.
///
/// # Determinism
///
/// The drained batch is STABLY sorted by slot before it is read, so the
/// timeline's order does not depend on the archetype order the reporting
/// queries happened to walk — while the order of reports WITHIN a slot, which
/// is the part that carries meaning, is preserved exactly. The registry is a
/// `BTreeMap`, so every sweep walks in slot order. No clock, no RNG.
pub fn emit_task_lifecycle_narrative(
    queue: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    lifecycles: Option<ResMut<TaskLifecycles>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    phase: Option<Res<State<GamePhase>>>,
    operators: Query<(
        &EntityUuid,
        Option<&ShipConfigComponent>,
        Option<&HumanSeekingHosts>,
    )>,
    mut out: MessageWriter<NarrativeEvent>,
) {
    let Some(mut lifecycles) = lifecycles else {
        return;
    };
    let batch: Vec<TaskLifecycleRequest> = match queue {
        Some(mut queue) if !queue.0.is_empty() => std::mem::take(&mut queue.0),
        _ => Vec::new(),
    };
    let mission_over = phase.is_some_and(|p| *p.get() == GamePhase::GameOver);
    // A run that ends with a standing order still being polled has repeats to
    // report even though nothing is live and nothing was queued, so the quiet-
    // tick early-out has to let that one case through.
    let flush_pending_repeats = mission_over && lifecycles.has_pending_repeats();
    if batch.is_empty() && lifecycles.is_empty() && !flush_pending_repeats {
        return;
    }
    let now = tick.map(|t| t.0).unwrap_or(0);

    let mut batch = batch;
    // Stable: cross-slot order becomes the slot's own total order, intra-slot
    // order stays the order the tick produced it in. See the determinism note.
    batch.sort_by(|a, b| slot_of(a).cmp(slot_of(b)));

    let mut index = 0usize;
    while index < batch.len() {
        let request = batch[index].clone();
        index += 1;
        match request {
            TaskLifecycleRequest::Start { slot, target } => {
                // An INSTANTANEOUS activation — a start and its own terminal,
                // adjacent on one slot, both minted by this tick — that repeats
                // the failure the slot last recorded gets no beats of its own
                // and spends no ordinal. It is COUNTED against the retained
                // terminal instead, so a standing unfulfillable order costs the
                // timeline one pair plus one census beat rather than a rate —
                // and no attempt goes unrecorded, which is the half of AC3 a
                // silent drop would lose. See
                // `TaskLifecycles::repeats_last_failure`.
                if let Some(TaskLifecycleRequest::End {
                    slot: next_slot,
                    reason,
                }) = batch.get(index)
                {
                    if *next_slot == slot
                        && lifecycles.repeats_last_failure(&slot, target.as_deref(), *reason)
                    {
                        lifecycles.note_repeat(&slot);
                        index += 1;
                        continue;
                    }
                }
                // Anything else on this slot ENDS the standing failure it was
                // repeating, so the attempts folded into that terminal are
                // reported before the new activation opens — and before the
                // restart below, whose own terminal would otherwise overwrite
                // the count.
                write_repeats(&mut out, &mut lifecycles, &slot);
                // A start on a slot that already holds one closes the old
                // activation first, so the restart is visible as a restart and
                // the replaced key still gets its single terminal event.
                if let Some(previous) = lifecycles.end(&slot) {
                    write_terminal(
                        &mut out,
                        &mut lifecycles,
                        &previous,
                        TaskTerminalReason::Restarted,
                        now,
                    );
                }
                let station = station_for(&operators, &slot);
                let activation = lifecycles.begin(slot, target, station, now);
                out.write(start_event(&activation));
            }
            TaskLifecycleRequest::End { slot, reason } => {
                // The dedupe: no live activation, no event.
                let Some(activation) = lifecycles.end(&slot) else {
                    continue;
                };
                let reason = if reason.masks_target_loss() && subject_gone(&operators, &activation)
                {
                    TaskTerminalReason::TargetDestroyed
                } else {
                    reason
                };
                write_terminal(&mut out, &mut lifecycles, &activation, reason, now);
            }
        }
    }

    // A subject that left the world ends the work whether or not the owning
    // system got as far as saying so this tick.
    let vanished: Vec<TaskSlot> = lifecycles
        .active()
        .filter(|activation| subject_gone(&operators, activation))
        .map(|activation| activation.key.slot.clone())
        .collect();
    for slot in vanished {
        if let Some(activation) = lifecycles.end(&slot) {
            write_terminal(
                &mut out,
                &mut lifecycles,
                &activation,
                TaskTerminalReason::TargetDestroyed,
                now,
            );
        }
    }

    // And the mission ending ends everything still running under it. The
    // headless report boundary (`headless::report::finalize_task_lifecycles`)
    // covers the OTHER way a run stops — simply reaching `max_ticks`, where no
    // further tick runs for this system to observe anything on.
    if mission_over {
        // The polled orders' counts first — they describe attempts made DURING
        // the run — then the terminals for what the ending itself stopped.
        for (activation, reason, repeats) in lifecycles.take_all_repeats() {
            out.write(repeats_event(&activation, reason, repeats));
        }
        for activation in lifecycles.take_all() {
            write_terminal(
                &mut out,
                &mut lifecycles,
                &activation,
                TaskTerminalReason::MissionEnded,
                now,
            );
        }
    }
}

/// Report the attempts coalesced into `slot`'s retained terminal, if any, as one
/// census beat carrying that terminal's key (issue #1341).
///
/// The retained terminal itself stays recorded, so a further identical attempt
/// still coalesces: what is drained is the count, not the memory of what the
/// slot last did.
fn write_repeats(
    out: &mut MessageWriter<NarrativeEvent>,
    lifecycles: &mut TaskLifecycles,
    slot: &TaskSlot,
) {
    if let Some((activation, reason, repeats)) = lifecycles.take_repeats(slot) {
        out.write(repeats_event(&activation, reason, repeats));
    }
}

/// Write one terminal beat and remember it on its slot.
///
/// Every terminal goes through here rather than through [`terminal_event`]
/// directly, so the "what did this slot last do" the coalescing rule reads is
/// the whole truth about the slot and not just the half some workflow reported.
fn write_terminal(
    out: &mut MessageWriter<NarrativeEvent>,
    lifecycles: &mut TaskLifecycles,
    activation: &TaskActivation,
    reason: TaskTerminalReason,
    now: u64,
) {
    lifecycles.record_terminal(activation, reason);
    out.write(terminal_event(activation, reason, now));
}

/// The slot a queued request addresses — the sort key that makes the batch
/// order independent of query iteration order.
fn slot_of(request: &TaskLifecycleRequest) -> &TaskSlot {
    match request {
        TaskLifecycleRequest::Start { slot, .. } => slot,
        TaskLifecycleRequest::End { slot, .. } => slot,
    }
}

/// The station the operating system belongs to on the operator's own hull, or
/// `None` when the hull carries no config or the system resolves to no station.
fn station_for(
    operators: &Query<(
        &EntityUuid,
        Option<&ShipConfigComponent>,
        Option<&HumanSeekingHosts>,
    )>,
    slot: &TaskSlot,
) -> Option<String> {
    let (_, config, hosts) = operators
        .iter()
        .find(|(uuid, _, _)| uuid.0 == slot.operator)?;
    let config = config?;
    crate::command_admission::policy::station_for_system(
        &config.0,
        hosts,
        &SystemId(slot.system.clone()),
    )
    .map(|station| station.0)
}

/// Whether this activation's subject has left the world.
///
/// A task with no subject can never lose one. A world with no uuid'd entities at
/// all is a reduced fixture that is not simulating entities, and reports nothing
/// gone — otherwise every activation in such a fixture would be closed as though
/// its subject had been destroyed.
fn subject_gone(
    operators: &Query<(
        &EntityUuid,
        Option<&ShipConfigComponent>,
        Option<&HumanSeekingHosts>,
    )>,
    activation: &TaskActivation,
) -> bool {
    let Some(target) = activation.key.target.as_deref() else {
        return false;
    };
    let mut any = false;
    for (uuid, _, _) in operators.iter() {
        any = true;
        if uuid.0 == target {
            return false;
        }
    }
    any
}

#[cfg(test)]
#[path = "narrative_tests.rs"]
mod tests;
