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
use crate::sim_tick::SimTick;
use crate::world::config::WorldConfig;
use crate::ship::components::{HumanSeekingHosts, ShipConfigComponent};
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
    runtime: Option<Res<WorldContentRuntime>>,
    mut seen_objectives: Local<BTreeMap<String, ObjectiveStatus>>,
    mut seen_deadlines: Local<BTreeMap<String, DeadlineState>>,
    mut out: MessageWriter<NarrativeEvent>,
) {
    if let Some(mut objectives) = objectives {
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
            if let Some(kind) = kind_for_transition(transition.kind) {
                out.write(objective_event(
                    kind,
                    &transition.id,
                    &transition.text,
                    transition.mandatory,
                    &transition.targets,
                ));
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
mod tests {
    use super::*;
    use bevy::ecs::message::Messages;

    /// Build a bare app carrying just the narrative message and the systems
    /// under test — no simulation plugins, so the assertions are about these
    /// systems and nothing else.
    fn narrative_app() -> App {
        let mut app = App::new();
        app.add_message::<NarrativeEvent>()
            .add_message::<BalanceEvent>();
        app
    }

    /// Drain the events written so far, in write order.
    fn drain(app: &mut App) -> Vec<NarrativeEvent> {
        let mut messages = app.world_mut().resource_mut::<Messages<NarrativeEvent>>();
        messages.drain().collect()
    }

    /// The whole reason the objective emitter reads the manager's log: it
    /// observes a transition whichever call site caused it, and reports each one
    /// once.
    #[test]
    fn objective_posted_then_completed_reports_each_transition_once() {
        let mut app = narrative_app();
        app.insert_resource(ObjectiveManagerRes(Default::default()));
        app.add_systems(Update, emit_scenario_narrative);

        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "world.probe.objective.reach",
            true,
            vec![],
        );
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ObjectivePosted);
        assert_eq!(events[0].id, "reach_axiom");
        assert_eq!(
            events[0].detail.get("text"),
            Some(&NarrativeValue::Text("world.probe.objective.reach".into())),
            "the objective's String Id must pass through verbatim"
        );

        // An unchanged objective is not re-reported.
        app.update();
        assert!(drain(&mut app).is_empty());

        app.world_mut()
            .resource_mut::<ObjectiveManagerRes>()
            .0
            .complete("reach_axiom");
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ObjectiveCompleted);

        app.update();
        assert!(drain(&mut app).is_empty(), "completion must report once");
    }

    /// The reason the manager keeps a transition log at all (issue #1338
    /// review): an objective posted and resolved inside ONE fixed tick is a
    /// single status field and two story beats. A diff of the tick's end state
    /// could only ever report the completion, silently swallowing the posting —
    /// and #1341/#1344 read this timeline as a sequence of transitions.
    #[test]
    fn an_objective_posted_and_completed_in_one_tick_reports_both_in_order() {
        let mut app = narrative_app();
        app.insert_resource(ObjectiveManagerRes(Default::default()));
        app.add_systems(Update, emit_scenario_narrative);
        {
            let mut objectives = app.world_mut().resource_mut::<ObjectiveManagerRes>();
            objectives
                .0
                .add("snap_decision", "world.probe.objective.snap", true, vec![]);
            objectives.0.complete("snap_decision");
        }
        app.update();

        let events = drain(&mut app);
        assert_eq!(
            events.len(),
            2,
            "both transitions are beats, not just the terminal one: {events:?}"
        );
        assert_eq!(events[0].kind, NarrativeKind::ObjectivePosted);
        assert_eq!(events[1].kind, NarrativeKind::ObjectiveCompleted);
        assert!(events.iter().all(|e| e.id == "snap_decision"));
        // The posting carries the objective's own fields, not a placeholder.
        assert_eq!(
            events[0].detail.get("text"),
            Some(&NarrativeValue::Text("world.probe.objective.snap".into()))
        );
        assert_eq!(
            events[0].detail.get("mandatory"),
            Some(&NarrativeValue::Flag(true))
        );

        // And nothing is re-reported on the next tick by the diff backstop.
        app.update();
        assert!(drain(&mut app).is_empty());
    }

    /// The same for a posting the world takes straight back: the objective
    /// existed, so the posting is a beat. The REMOVAL is not (a layer unloading
    /// its own bookkeeping is not a story event), so exactly one event survives —
    /// where a diff would have reported nothing at all.
    #[test]
    fn an_objective_posted_and_removed_in_one_tick_still_reports_the_posting() {
        let mut app = narrative_app();
        app.insert_resource(ObjectiveManagerRes(Default::default()));
        app.add_systems(Update, emit_scenario_narrative);
        {
            let mut objectives = app.world_mut().resource_mut::<ObjectiveManagerRes>();
            objectives
                .0
                .add("layer_local", "world.probe.objective.layer", false, vec![]);
            assert!(objectives.0.remove("layer_local"));
        }
        app.update();

        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ObjectivePosted);
        assert_eq!(events[0].id, "layer_local");

        app.update();
        assert!(
            drain(&mut app).is_empty(),
            "the removed objective must leave the backstop's view"
        );
    }

    /// The diff is still there, and still earns its keep: an objective whose
    /// state arrived without passing through the manager's own API — a world
    /// load inserting a prepared manager, say — is reported by the backstop.
    #[test]
    fn the_diff_backstop_reports_state_that_never_passed_through_the_log() {
        let mut app = narrative_app();
        let mut prepared = crate::objectives::ObjectiveManager::new();
        prepared.add("prior", "world.probe.objective.prior", true, vec![]);
        prepared.complete("prior");
        // Drop the log, so the ONLY thing the emitter can see is the state.
        let _ = prepared.drain_transitions();
        app.insert_resource(ObjectiveManagerRes(prepared));
        app.add_systems(Update, emit_scenario_narrative);
        app.update();

        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ObjectiveCompleted);
        assert_eq!(events[0].id, "prior");
    }

    /// A failed objective is its own beat, distinct from a completed one.
    #[test]
    fn objective_failure_is_its_own_kind() {
        let mut app = narrative_app();
        app.insert_resource(ObjectiveManagerRes(Default::default()));
        app.add_systems(Update, emit_scenario_narrative);
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "hold_the_line",
            "world.probe.objective.hold",
            false,
            vec![],
        );
        app.update();
        drain(&mut app);
        app.world_mut()
            .resource_mut::<ObjectiveManagerRes>()
            .0
            .fail("hold_the_line");
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ObjectiveFailed);
        assert_eq!(
            events[0].detail.get("mandatory"),
            Some(&NarrativeValue::Flag(false))
        );
    }

    /// Firing is the only deadline transition that is a story beat, and it is
    /// reported once.
    #[test]
    fn only_a_fired_deadline_reaches_the_timeline() {
        use crate::world::deadlines::DeadlineRecord;

        let mut app = narrative_app();
        let mut runtime = WorldContentRuntime::default();
        runtime.deadlines.records.push(DeadlineRecord {
            id: "window_opens".into(),
            origin_layer: None,
            label: "world.probe.deadline.window_opens.label".into(),
            visible: true,
            due_tick: 600,
            state: DeadlineState::Pending,
            armed: None,
        });
        runtime.deadlines.records.push(DeadlineRecord {
            id: "called_off".into(),
            origin_layer: None,
            label: "world.probe.deadline.called_off.label".into(),
            visible: false,
            due_tick: 900,
            state: DeadlineState::Pending,
            armed: None,
        });
        app.insert_resource(runtime);
        app.add_systems(Update, emit_scenario_narrative);

        // Pending deadlines say nothing.
        app.update();
        assert!(drain(&mut app).is_empty());

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.deadlines.records[0].state = DeadlineState::Fired;
            runtime.deadlines.records[1].state = DeadlineState::Cancelled;
        }
        app.update();
        let events = drain(&mut app);
        assert_eq!(
            events.len(),
            1,
            "a cancelled deadline is not a beat: {events:?}"
        );
        assert_eq!(events[0].kind, NarrativeKind::DeadlineFired);
        assert_eq!(events[0].id, "window_opens");
        assert_eq!(
            events[0].detail.get("label"),
            Some(&NarrativeValue::Text(
                "world.probe.deadline.window_opens.label".into()
            ))
        );

        app.update();
        assert!(drain(&mut app).is_empty(), "firing must report once");
    }

    /// Marking is the gate: an unmarked hull dying produces nothing, a marked
    /// one produces exactly one spawn beat and one death beat.
    #[test]
    fn only_marked_entities_produce_spawn_and_death_beats() {
        let mut app = narrative_app();
        app.add_systems(Update, emit_authored_and_marked_entity_narrative);

        app.world_mut()
            .spawn((EntityUuid("uuid-lyra".into()), NarrativeMark("lyra".into())));
        app.world_mut().spawn(EntityUuid("uuid-rock".into()));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntitySpawned);
        assert_eq!(events[0].id, "lyra");
        assert_eq!(events[0].source.entity.as_deref(), Some("uuid-lyra"));

        // Spawns report once.
        app.update();
        assert!(drain(&mut app).is_empty());

        app.world_mut()
            .resource_mut::<Messages<BalanceEvent>>()
            .write(BalanceEvent::EntityDestroyed {
                victim: "uuid-rock".into(),
                killer: Some("uuid-lyra".into()),
            });
        app.world_mut()
            .resource_mut::<Messages<BalanceEvent>>()
            .write(BalanceEvent::EntityDestroyed {
                victim: "uuid-lyra".into(),
                killer: None,
            });
        app.update();
        let events = drain(&mut app);
        assert_eq!(
            events.len(),
            1,
            "the unmarked rock's death is not a story beat: {events:?}"
        );
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntityDestroyed);
        assert_eq!(events[0].id, "lyra");
    }

    /// A marked entity that dies after despawning still reports: the mark is
    /// remembered from while it was alive, which is the whole reason the map
    /// exists.
    #[test]
    fn a_despawned_marked_entity_still_reports_its_death() {
        let mut app = narrative_app();
        app.add_systems(Update, emit_authored_and_marked_entity_narrative);
        let entity = app
            .world_mut()
            .spawn((EntityUuid("uuid-wick".into()), NarrativeMark("wick".into())))
            .id();
        app.update();
        drain(&mut app);

        app.world_mut().entity_mut(entity).despawn();
        app.world_mut()
            .resource_mut::<Messages<BalanceEvent>>()
            .write(BalanceEvent::EntityDestroyed {
                victim: "uuid-wick".into(),
                killer: Some("uuid-raider".into()),
            });
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntityDestroyed);
        assert_eq!(events[0].target.as_deref(), Some("uuid-raider"));
    }

    /// The authored queue drains front to back and leaves nothing behind.
    #[test]
    fn authored_requests_drain_in_order_and_empty_the_queue() {
        let mut app = narrative_app();
        app.init_resource::<EffectQueue<NarrativeRequest>>();
        app.add_systems(Update, emit_authored_and_marked_entity_narrative);
        {
            let mut queue = app
                .world_mut()
                .resource_mut::<EffectQueue<NarrativeRequest>>();
            queue.0.push(NarrativeRequest::Authored {
                kind: NarrativeKind::BeatFired,
                id: "storm_hits".into(),
                entity_uuid: None,
            });
            queue.0.push(NarrativeRequest::Authored {
                kind: NarrativeKind::MarkedEntityRescued,
                id: "lyra".into(),
                entity_uuid: Some("uuid-lyra".into()),
            });
        }
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::BeatFired);
        assert_eq!(events[0].id, "storm_hits");
        assert_eq!(events[1].kind, NarrativeKind::MarkedEntityRescued);
        assert_eq!(events[1].source.entity.as_deref(), Some("uuid-lyra"));
        assert!(app
            .world()
            .resource::<EffectQueue<NarrativeRequest>>()
            .0
            .is_empty());
    }

    // ── The scripted removal path (issue #1338 review) ────────────────────────
    //
    // `ctx.effects.destroy_entity(..)` writes no `BalanceEvent` and must not:
    // an authorial removal is not a combat kill. These three fix what the
    // absence of that event used to mean — a marked hull could be removed by a
    // script and leave the timeline with a spawn and no fate.

    /// Spawn a marked hull, run a tick to register the mark, and return the app
    /// with the entity's id.
    fn app_with_marked_hull(uuid: &str, narrative_id: &str) -> (App, Entity) {
        let mut app = narrative_app();
        app.init_resource::<EffectQueue<NarrativeRequest>>();
        app.add_systems(Update, emit_authored_and_marked_entity_narrative);
        let entity = app
            .world_mut()
            .spawn((
                EntityUuid(uuid.to_string()),
                NarrativeMark(narrative_id.to_string()),
            ))
            .id();
        app.update();
        drain(&mut app);
        (app, entity)
    }

    /// Queue one scripted-removal signal, as `ActionCmd::DestroyEntity`'s
    /// applier arm does, and despawn the entity as the same arm does.
    fn scripted_destroy(app: &mut App, uuid: &str, entity: Entity) {
        app.world_mut()
            .resource_mut::<EffectQueue<NarrativeRequest>>()
            .0
            .push(NarrativeRequest::ScriptedRemoval {
                entity_uuid: uuid.to_string(),
            });
        app.world_mut().entity_mut(entity).despawn();
    }

    /// The no-silent-vanish fallback: a script removing a marked hull the author
    /// said nothing else about records its death exactly once — even though no
    /// `BalanceEvent` was written for it.
    #[test]
    fn a_scripted_destroy_of_a_marked_hull_reports_one_death() {
        let (mut app, entity) = app_with_marked_hull("uuid-skyhook", "skyhook");
        scripted_destroy(&mut app, "uuid-skyhook", entity);
        app.update();

        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntityDestroyed);
        assert_eq!(events[0].id, "skyhook");
        assert_eq!(events[0].source.entity.as_deref(), Some("uuid-skyhook"));

        // A second removal of the same hull (a re-fired trigger, a duplicate
        // schedule entry) adds no second death.
        app.world_mut()
            .resource_mut::<EffectQueue<NarrativeRequest>>()
            .0
            .push(NarrativeRequest::ScriptedRemoval {
                entity_uuid: "uuid-skyhook".into(),
            });
        app.update();
        assert!(drain(&mut app).is_empty(), "a hull dies once");
    }

    /// …and the author's own word wins. A `rescued` outcome recorded on the same
    /// tick as the destroy — in EITHER queue order — leaves the rescue standing
    /// alone, with no death invented behind it.
    #[test]
    fn an_authored_outcome_suppresses_the_scripted_death() {
        for outcome_first in [true, false] {
            let (mut app, entity) = app_with_marked_hull("uuid-lyra", "lyra");
            let authored = NarrativeRequest::Authored {
                kind: NarrativeKind::MarkedEntityRescued,
                id: "lyra".into(),
                entity_uuid: Some("uuid-lyra".into()),
            };
            if outcome_first {
                app.world_mut()
                    .resource_mut::<EffectQueue<NarrativeRequest>>()
                    .0
                    .push(authored.clone());
                scripted_destroy(&mut app, "uuid-lyra", entity);
            } else {
                scripted_destroy(&mut app, "uuid-lyra", entity);
                app.world_mut()
                    .resource_mut::<EffectQueue<NarrativeRequest>>()
                    .0
                    .push(authored.clone());
            }
            app.update();

            let events = drain(&mut app);
            assert_eq!(
                events.len(),
                1,
                "outcome_first={outcome_first}: the rescue must stand alone: {events:?}"
            );
            assert_eq!(events[0].kind, NarrativeKind::MarkedEntityRescued);
            assert!(
                !events
                    .iter()
                    .any(|e| e.kind == NarrativeKind::MarkedEntityDestroyed),
                "a rescue-by-despawn must never be recorded as a death"
            );
        }
    }

    /// An outcome authored on an EARLIER tick suppresses it too — the gate is
    /// the run's timeline, not the tick's.
    #[test]
    fn an_outcome_authored_earlier_in_the_run_suppresses_the_scripted_death() {
        let (mut app, entity) = app_with_marked_hull("uuid-lyra", "lyra");
        app.world_mut()
            .resource_mut::<EffectQueue<NarrativeRequest>>()
            .0
            .push(NarrativeRequest::Authored {
                kind: NarrativeKind::MarkedEntityEscaped,
                id: "lyra".into(),
                entity_uuid: Some("uuid-lyra".into()),
            });
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntityEscaped);

        scripted_destroy(&mut app, "uuid-lyra", entity);
        app.update();
        assert!(
            drain(&mut app).is_empty(),
            "the hull's fate was already on the timeline"
        );
    }

    /// Marking is still the gate on this path: the traffic a scenario clears
    /// away with `destroy_entity` produces nothing at all.
    #[test]
    fn a_scripted_destroy_of_an_unmarked_hull_stays_silent() {
        let mut app = narrative_app();
        app.init_resource::<EffectQueue<NarrativeRequest>>();
        app.add_systems(Update, emit_authored_and_marked_entity_narrative);
        let rock = app.world_mut().spawn(EntityUuid("uuid-rock".into())).id();
        app.update();
        assert!(
            drain(&mut app).is_empty(),
            "an unmarked hull has no spawn beat"
        );

        scripted_destroy(&mut app, "uuid-rock", rock);
        app.update();
        assert!(
            drain(&mut app).is_empty(),
            "an unmarked hull a script removes is not a story beat"
        );
    }

    // ── The continuous-task lifecycle (issue #1341) ───────────────────────────
    //
    // The emitter's four decisions — ordinal, station, vanished subject, and the
    // one-terminal rule — exercised against a bare app, so what is being
    // asserted is this system and not the tractor or the scan.

    use crate::core::task_lifecycle::{
        TaskLifecycleRequest, TaskLifecycles, TaskSlot, TaskTerminalReason, TASK_VERB_SCAN,
        TASK_VERB_TRACTOR_HOLD,
    };

    const OPERATOR: &str = "uuid-tender";
    const SUBJECT: &str = "uuid-hulk";

    fn hold_slot() -> TaskSlot {
        TaskSlot::new(OPERATOR, "tractor", TASK_VERB_TRACTOR_HOLD)
    }

    /// A bare app carrying the lifecycle plumbing, the operator hull and the
    /// subject hull, with the emitter as its only system.
    fn lifecycle_app() -> App {
        let mut app = narrative_app();
        app.init_resource::<EffectQueue<TaskLifecycleRequest>>()
            .init_resource::<TaskLifecycles>()
            .init_resource::<crate::sim_tick::SimTick>()
            .add_systems(Update, emit_task_lifecycle_narrative);
        app.world_mut().spawn(EntityUuid(OPERATOR.into()));
        app.world_mut().spawn(EntityUuid(SUBJECT.into()));
        app
    }

    fn push(app: &mut App, request: TaskLifecycleRequest) {
        app.world_mut()
            .resource_mut::<EffectQueue<TaskLifecycleRequest>>()
            .0
            .push(request);
    }

    fn start(slot: TaskSlot, target: &str) -> TaskLifecycleRequest {
        TaskLifecycleRequest::Start {
            slot,
            target: Some(target.to_string()),
        }
    }

    fn end(slot: TaskSlot, reason: TaskTerminalReason) -> TaskLifecycleRequest {
        TaskLifecycleRequest::End { slot, reason }
    }

    /// The reason recorded on a terminal event.
    fn reason_of(event: &NarrativeEvent) -> String {
        match event.detail.get("reason") {
            Some(NarrativeValue::Text(s)) => s.clone(),
            other => panic!("a terminal event must carry its reason, got {other:?}"),
        }
    }

    /// The core contract: one start, one terminal, sharing one key.
    #[test]
    fn an_activation_produces_one_start_and_one_terminal_sharing_a_key() {
        let mut app = lifecycle_app();
        push(&mut app, start(hold_slot(), SUBJECT));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::TaskStarted);
        let key = events[0].id.clone();
        assert_eq!(events[0].source.entity.as_deref(), Some(OPERATOR));
        assert_eq!(events[0].source.system.as_deref(), Some("tractor"));
        assert_eq!(events[0].target.as_deref(), Some(SUBJECT));

        push(&mut app, end(hold_slot(), TaskTerminalReason::Released));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::TaskCancelled);
        assert_eq!(
            events[0].id, key,
            "the terminal beat carries the start's key"
        );
        assert_eq!(reason_of(&events[0]), "released");
    }

    /// The dedupe, which is what makes "exactly one terminal event" true when
    /// two sites report the same ending: `operate_tractor_ai`'s withdrawn order
    /// and the `ReleaseTractor` it causes are one hold ending once, and the
    /// FIRST report — the truer one — is what the timeline records.
    #[test]
    fn a_second_report_of_the_same_ending_adds_nothing() {
        let mut app = lifecycle_app();
        push(&mut app, start(hold_slot(), SUBJECT));
        app.update();
        drain(&mut app);

        push(
            &mut app,
            end(hold_slot(), TaskTerminalReason::OrderWithdrawn),
        );
        push(&mut app, end(hold_slot(), TaskTerminalReason::Released));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "one hold ends once: {events:?}");
        assert_eq!(reason_of(&events[0]), "order_withdrawn");

        // …and a terminal report for a slot that never started says nothing at
        // all, rather than inventing an activation to close.
        push(&mut app, end(hold_slot(), TaskTerminalReason::Released));
        app.update();
        assert!(drain(&mut app).is_empty());
    }

    /// A restart closes the activation it replaces and opens a distinct key, so
    /// both are separately identifiable in the timeline — AC3.
    #[test]
    fn a_restart_closes_the_old_activation_and_mints_a_new_key() {
        let mut app = lifecycle_app();
        push(&mut app, start(hold_slot(), SUBJECT));
        app.update();
        let first_key = drain(&mut app)[0].id.clone();

        push(&mut app, start(hold_slot(), SUBJECT));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::TaskInterrupted);
        assert_eq!(events[0].id, first_key);
        assert_eq!(reason_of(&events[0]), "restarted");
        assert_eq!(events[1].kind, NarrativeKind::TaskStarted);
        assert_ne!(
            events[1].id, first_key,
            "the second hold of the same hull is a SECOND task"
        );
    }

    /// Two slots run at once and neither ends the other — the simultaneous half
    /// of AC3.
    #[test]
    fn simultaneous_tasks_on_one_hull_stay_separate() {
        let mut app = lifecycle_app();
        let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
        push(&mut app, start(hold_slot(), SUBJECT));
        push(&mut app, start(scan_slot.clone(), SUBJECT));
        push(&mut app, end(scan_slot, TaskTerminalReason::Completed));
        app.update();

        let events = drain(&mut app);
        // Stable slot sort: the sensors slot sorts before the tractor slot, and
        // its own start-then-end order is preserved inside it.
        let shape: Vec<(&str, &str)> = events
            .iter()
            .map(|e| (e.kind.as_str(), e.source.system.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("task_started", "sensors"),
                ("task_completed", "sensors"),
                ("task_started", "tractor"),
            ],
            "{events:?}"
        );
        // The hold is untouched by the scan finishing.
        assert_eq!(
            app.world().resource::<TaskLifecycles>().len(),
            1,
            "the tractor hold must still be running"
        );
    }

    /// The subject leaving the world is a target-DESTROYED interruption, even
    /// though the owning system can only see "out of range".
    #[test]
    fn a_vanished_subject_upgrades_the_reason_it_masked() {
        let mut app = lifecycle_app();
        push(&mut app, start(hold_slot(), SUBJECT));
        app.update();
        drain(&mut app);

        let subject = app
            .world_mut()
            .query::<(Entity, &EntityUuid)>()
            .iter(app.world())
            .find(|(_, uuid)| uuid.0 == SUBJECT)
            .map(|(entity, _)| entity)
            .expect("the fixture spawns the subject");
        app.world_mut().entity_mut(subject).despawn();

        push(&mut app, end(hold_slot(), TaskTerminalReason::OutOfRange));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::TaskInterrupted);
        assert_eq!(
            reason_of(&events[0]),
            "target_destroyed",
            "a hull that left the world is not merely distant"
        );
    }

    /// …and it ends the task even when nothing reports it at all — the case a
    /// scripted removal produces, where the owning system may not run again
    /// before the timeline is read.
    #[test]
    fn a_vanished_subject_ends_the_task_unreported() {
        let mut app = lifecycle_app();
        push(&mut app, start(hold_slot(), SUBJECT));
        app.update();
        drain(&mut app);

        let subject = app
            .world_mut()
            .query::<(Entity, &EntityUuid)>()
            .iter(app.world())
            .find(|(_, uuid)| uuid.0 == SUBJECT)
            .map(|(entity, _)| entity)
            .expect("the fixture spawns the subject");
        app.world_mut().entity_mut(subject).despawn();
        app.update();

        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(reason_of(&events[0]), "target_destroyed");
        assert!(app.world().resource::<TaskLifecycles>().is_empty());

        // And it happens once: the sweep closed the activation, so the next tick
        // has nothing left to close.
        app.update();
        assert!(drain(&mut app).is_empty());
    }

    /// The mission ending under a live task closes it, exactly once — the
    /// GameOver half of AC2, which the headless report boundary does NOT cover
    /// (that is the max-ticks half, `headless::report::finalize_task_lifecycles`).
    ///
    /// This branch is the live one on any host that keeps ticking after the run
    /// is decided, so it also has to leave the registry empty: a second tick in
    /// GameOver must not close the same activation again.
    #[test]
    fn the_mission_ending_closes_every_live_task_once() {
        let mut app = lifecycle_app();
        app.insert_resource(State::new(GamePhase::InProgress));
        let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
        push(&mut app, start(hold_slot(), SUBJECT));
        push(&mut app, start(scan_slot, SUBJECT));
        app.update();
        assert_eq!(drain(&mut app).len(), 2);
        assert_eq!(app.world().resource::<TaskLifecycles>().len(), 2);

        // The run is decided, and the schedule keeps running under it.
        app.insert_resource(State::new(GamePhase::GameOver));
        app.update();
        let events = drain(&mut app);
        assert_eq!(
            events.len(),
            2,
            "both live tasks end when the mission does: {events:?}"
        );
        for event in &events {
            assert_eq!(event.kind, NarrativeKind::TaskInterrupted);
            assert_eq!(reason_of(event), "mission_ended");
        }
        // Slot order, never the order they were opened in.
        assert_eq!(
            events
                .iter()
                .map(|e| e.source.system.as_deref().unwrap_or(""))
                .collect::<Vec<_>>(),
            vec!["sensors", "tractor"]
        );
        assert!(
            app.world().resource::<TaskLifecycles>().is_empty(),
            "the sweep must empty the registry, or the task's own terminal is \
             swallowed by the dedupe while the sim keeps ticking"
        );

        // …and a second tick under GameOver adds nothing.
        app.update();
        assert!(drain(&mut app).is_empty());
    }

    /// A standing order that can never be fulfilled is a POLL, not a beat: the
    /// Sensors AI host re-issues its `ScanTarget` on every authored snapshot
    /// while its objective stays unsatisfied, so an out-of-range Scan would
    /// otherwise mint a whole activation per cadence, unbounded, into the
    /// report, the counts and the ndjson stream.
    #[test]
    fn a_repeated_identical_failure_is_recorded_once() {
        let mut app = lifecycle_app();
        let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
        let mut events = Vec::new();
        for _ in 0..200 {
            push(&mut app, start(scan_slot.clone(), SUBJECT));
            push(
                &mut app,
                end(scan_slot.clone(), TaskTerminalReason::OutOfRange),
            );
            app.update();
            events.extend(drain(&mut app));
        }
        assert_eq!(
            events.len(),
            2,
            "200 cadences of one unchanged refusal are one beat: {events:?}"
        );
        assert_eq!(events[0].kind, NarrativeKind::TaskStarted);
        assert_eq!(reason_of(&events[1]), "out_of_range");
        // The suppressed repeats did not spend ordinals either, so the key the
        // reader sees is the first activation's.
        assert!(events[0].id.ends_with("#0"), "{:?}", events[0].id);
        // …but they were COUNTED, not erased: 199 attempts are still waiting to
        // be reported under that same key. Nothing counts in the dark.
        assert!(app
            .world()
            .resource::<TaskLifecycles>()
            .has_pending_repeats());
    }

    /// The other half of that rule, and the half issue #1341's AC3 turns on: the
    /// coalesced attempts are reported, once, under the key of the beat they
    /// were folded into.
    ///
    /// The emitter cannot tell an AI cadence retry from an engineer pressing
    /// Engage a second time on a target that is still out of reach — both are
    /// one slot, one subject, one unchanged refusal, both halves minted by one
    /// tick — so it must bound the repeat without making it invisible.
    #[test]
    fn the_coalesced_repeats_are_reported_once_when_the_standing_failure_ends() {
        let mut app = lifecycle_app();
        let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
        // Drained every iteration: `Messages` is double-buffered, so an event
        // left unread for two updates expires on its own.
        let mut first = Vec::new();
        for _ in 0..12 {
            push(&mut app, start(scan_slot.clone(), SUBJECT));
            push(
                &mut app,
                end(scan_slot.clone(), TaskTerminalReason::OutOfRange),
            );
            app.update();
            first.extend(drain(&mut app));
        }
        assert_eq!(first.len(), 2, "one pair for the standing order");
        let key = first[0].id.clone();

        // The order finally reaches its subject. That ENDS the standing failure,
        // so the eleven suppressed attempts are reported first — under the key
        // they repeated — and the reading that came back is its own activation.
        push(&mut app, start(scan_slot.clone(), SUBJECT));
        push(&mut app, end(scan_slot, TaskTerminalReason::Completed));
        app.update();
        let events = drain(&mut app);
        assert_eq!(
            events
                .iter()
                .map(|e| (e.kind, e.id.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (NarrativeKind::TaskRepeated, key.as_str()),
                (
                    NarrativeKind::TaskStarted,
                    "uuid-tender/sensors/scan/uuid-hulk#1"
                ),
                (
                    NarrativeKind::TaskCompleted,
                    "uuid-tender/sensors/scan/uuid-hulk#1"
                ),
            ],
            "the census beat, then the activation that broke the standing failure"
        );
        assert_eq!(
            events[0].detail.get("repeats"),
            Some(&NarrativeValue::Int(11)),
            "eleven attempts were folded into the recorded refusal"
        );
        assert_eq!(
            events[0].detail.get("attempts"),
            Some(&NarrativeValue::Int(12))
        );
        assert!(
            !app.world()
                .resource::<TaskLifecycles>()
                .has_pending_repeats(),
            "reported once, not once per later beat"
        );
    }

    /// A run that stops with the standing order still being polled has nothing
    /// left to break the failure, so the mission ending is what reports the
    /// count. (`headless::report::finalize_task_lifecycles` covers the other way
    /// a run stops, where no further tick exists at all.)
    #[test]
    fn a_still_polling_order_reports_its_repeats_when_the_mission_ends() {
        let mut app = lifecycle_app();
        app.insert_resource(State::new(GamePhase::InProgress));
        let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
        // Drained every iteration: `Messages` is double-buffered, so an event
        // left unread for two updates expires on its own.
        let mut recorded = Vec::new();
        for _ in 0..5 {
            push(&mut app, start(scan_slot.clone(), SUBJECT));
            push(
                &mut app,
                end(scan_slot.clone(), TaskTerminalReason::OutOfRange),
            );
            app.update();
            recorded.extend(drain(&mut app));
        }
        assert_eq!(recorded.len(), 2);

        app.insert_resource(State::new(GamePhase::GameOver));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "one census beat: {events:?}");
        assert_eq!(events[0].kind, NarrativeKind::TaskRepeated);
        assert_eq!(
            events[0].detail.get("repeats"),
            Some(&NarrativeValue::Int(4))
        );
        // …and the tick after adds nothing, however long the run sits in
        // GameOver.
        app.update();
        assert!(drain(&mut app).is_empty());
    }

    /// …but only while nothing about it changes. A different subject, a
    /// different reason, or the work starting to succeed are all beats again —
    /// and a task that spanned ticks is never a poll, however its predecessor
    /// ended.
    #[test]
    fn a_changed_or_spanning_task_is_still_a_beat() {
        let mut app = lifecycle_app();
        let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
        let refuse = |app: &mut App, target: &str, reason| {
            push(app, start(scan_slot.clone(), target));
            push(app, end(scan_slot.clone(), reason));
            app.update();
        };

        refuse(&mut app, SUBJECT, TaskTerminalReason::OutOfRange);
        assert_eq!(drain(&mut app).len(), 2, "the first refusal is a beat");
        refuse(&mut app, SUBJECT, TaskTerminalReason::OutOfRange);
        assert!(
            drain(&mut app).is_empty(),
            "the unchanged repeat is not a beat of its own"
        );

        // A different subject — which also ENDS the standing failure, so the one
        // suppressed repeat is reported alongside the new pair.
        refuse(&mut app, OPERATOR, TaskTerminalReason::OutOfRange);
        let changed = drain(&mut app);
        assert_eq!(
            changed.iter().map(|e| e.kind).collect::<Vec<_>>(),
            vec![
                NarrativeKind::TaskRepeated,
                NarrativeKind::TaskStarted,
                NarrativeKind::TaskFailed
            ],
            "the census of what was coalesced, then the beat that changed: \
             {changed:?}"
        );
        assert_eq!(
            changed[0].detail.get("repeats"),
            Some(&NarrativeValue::Int(1))
        );
        // A different reason for the same subject.
        refuse(&mut app, SUBJECT, TaskTerminalReason::Unpowered);
        assert_eq!(drain(&mut app).len(), 2);
        // The work succeeding is never coalesced, however often it recurs.
        refuse(&mut app, SUBJECT, TaskTerminalReason::Completed);
        assert_eq!(drain(&mut app).len(), 2);
        refuse(&mut app, SUBJECT, TaskTerminalReason::Completed);
        assert_eq!(drain(&mut app).len(), 2);

        // And a task that spanned ticks is a real activation ending, even when
        // the last thing this slot recorded was the identical failure.
        refuse(&mut app, SUBJECT, TaskTerminalReason::OutOfRange);
        drain(&mut app);
        push(&mut app, start(scan_slot.clone(), SUBJECT));
        app.update();
        assert_eq!(drain(&mut app).len(), 1, "the start of a spanning task");
        push(&mut app, end(scan_slot, TaskTerminalReason::OutOfRange));
        app.update();
        assert_eq!(drain(&mut app).len(), 1, "and its own terminal");
    }

    /// A quiet tick with nothing running writes nothing — the emitter must not
    /// be a per-tick heartbeat.
    #[test]
    fn an_idle_run_produces_no_lifecycle_events() {
        let mut app = lifecycle_app();
        for _ in 0..5 {
            app.update();
        }
        assert!(drain(&mut app).is_empty());
    }

    /// A marked hull that is SHOT and then tidied away by a script reports one
    /// death, not two — the combat event is the real one and it records the
    /// fate.
    #[test]
    fn a_combat_death_suppresses_a_later_scripted_removal() {
        let (mut app, entity) = app_with_marked_hull("uuid-wick", "wick");
        app.world_mut()
            .resource_mut::<Messages<BalanceEvent>>()
            .write(BalanceEvent::EntityDestroyed {
                victim: "uuid-wick".into(),
                killer: Some("uuid-raider".into()),
            });
        scripted_destroy(&mut app, "uuid-wick", entity);
        app.update();

        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "one hull, one death: {events:?}");
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntityDestroyed);
        assert_eq!(
            events[0].target.as_deref(),
            Some("uuid-raider"),
            "the surviving death is the combat one, which carries killer credit"
        );
    }

    // ── The ship's-computer message (issue #1342) ─────────────────────────

    use crate::core::computer_message::ComputerMessageSeverity;
    use crate::core::messages::StationId;

    /// A bare app carrying just what `tick_computer_message` needs — no
    /// `WorldConfig`, so tick_hz falls back to `SchedClock::ZERO`'s 60 Hz.
    fn computer_message_app() -> App {
        let mut app = App::new();
        app.add_message::<NarrativeEvent>()
            .init_resource::<ActiveComputerMessage>()
            .init_resource::<EffectQueue<ComputerMessageRequest>>()
            .init_resource::<SimTick>();
        app.add_systems(Update, tick_computer_message);
        app
    }

    fn push_request(app: &mut App, req: ComputerMessageRequest) {
        app.world_mut()
            .resource_mut::<EffectQueue<ComputerMessageRequest>>()
            .0
            .push(req);
    }

    fn request(id: &str, secs: i64) -> ComputerMessageRequest {
        ComputerMessageRequest {
            id: id.into(),
            text: "world.probe.computer_message.text".into(),
            severity: ComputerMessageSeverity::Advisory,
            duration_secs: secs,
            station: None,
        }
    }

    fn set_tick(app: &mut App, tick: u64) {
        app.world_mut().resource_mut::<SimTick>().0 = tick;
    }

    /// Showing the first message emits exactly one `shown` beat, carrying the
    /// String Id, severity and duration verbatim.
    #[test]
    fn showing_a_message_emits_one_posted_event() {
        let mut app = computer_message_app();
        push_request(&mut app, request("hail_debris", 10));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ComputerMessagePosted);
        assert_eq!(events[0].id, "hail_debris");
        assert_eq!(
            events[0].detail.get("text"),
            Some(&NarrativeValue::Text(
                "world.probe.computer_message.text".into()
            ))
        );
        assert_eq!(
            events[0].detail.get("severity"),
            Some(&NarrativeValue::Text("advisory".into()))
        );
        assert_eq!(
            events[0].detail.get("duration_secs"),
            Some(&NarrativeValue::Int(10))
        );
        assert!(
            app.world()
                .resource::<ActiveComputerMessage>()
                .current
                .is_some(),
            "the state is now authoritatively showing something"
        );
    }

    /// A second message supersedes the first, in one tick: the superseded
    /// event names the OLD id and the reason, and the posted event follows it
    /// for the NEW id — both in one tick, in that order.
    #[test]
    fn a_second_message_supersedes_the_first_in_the_same_tick() {
        let mut app = computer_message_app();
        push_request(&mut app, request("first", 100));
        app.update();
        drain(&mut app);

        push_request(&mut app, request("second", 5));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ComputerMessageCleared);
        assert_eq!(events[0].id, "first");
        assert_eq!(
            events[0].detail.get("reason"),
            Some(&NarrativeValue::Text("superseded".into()))
        );
        assert_eq!(
            events[0].detail.get("superseded_by"),
            Some(&NarrativeValue::Text("second".into()))
        );
        assert_eq!(events[1].kind, NarrativeKind::ComputerMessagePosted);
        assert_eq!(events[1].id, "second");
    }

    /// Expiry is measured in simulation ticks: nothing is reported before the
    /// due tick, and exactly one `expired` event fires on it.
    #[test]
    fn expiry_reports_once_on_its_due_tick() {
        let mut app = computer_message_app();
        push_request(&mut app, request("hail_debris", 10));
        app.update();
        drain(&mut app);

        // One tick early: nothing.
        set_tick(&mut app, 599);
        app.update();
        assert!(drain(&mut app).is_empty());

        // Due: exactly one expired event.
        set_tick(&mut app, 600);
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ComputerMessageCleared);
        assert_eq!(events[0].id, "hail_debris");
        assert_eq!(
            events[0].detail.get("reason"),
            Some(&NarrativeValue::Text("expired".into()))
        );
        assert!(app
            .world()
            .resource::<ActiveComputerMessage>()
            .current
            .is_none());

        // And it does not re-report on a later tick.
        set_tick(&mut app, 700);
        app.update();
        assert!(drain(&mut app).is_empty(), "expiry must report once");
    }

    /// The optional Station cue rides the posted event's detail.
    #[test]
    fn a_station_cue_rides_the_posted_event() {
        let mut app = computer_message_app();
        push_request(
            &mut app,
            ComputerMessageRequest {
                id: "charge_ready".into(),
                text: "world.probe.computer_message.charge".into(),
                severity: ComputerMessageSeverity::Critical,
                duration_secs: 8,
                station: Some(StationId("tactical".into())),
            },
        );
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(
            events[0].detail.get("station"),
            Some(&NarrativeValue::Text("tactical".into()))
        );
        assert_eq!(
            events[0].detail.get("severity"),
            Some(&NarrativeValue::Text("critical".into()))
        );
    }

    /// A tick with no queued request and nothing due is silent.
    #[test]
    fn a_quiet_tick_emits_nothing() {
        let mut app = computer_message_app();
        app.update();
        assert!(drain(&mut app).is_empty());
    }

    /// `clear_active_computer_message` empties the state unconditionally and
    /// is not itself a narrative beat. Modelled on its real registration —
    /// `OnEnter(GamePhase::GameOver)` / `OnEnter(GamePhase::Lobby)` — rather
    /// than chained onto `tick_computer_message`'s own per-tick schedule,
    /// which would clear a message on the very tick it was shown.
    #[test]
    fn clear_active_computer_message_empties_state_silently() {
        let mut active = ActiveComputerMessage::default();
        active.show(&request("mission_wrap", 30), 0, 60.0);
        assert!(active.current.is_some());

        let mut app = App::new();
        app.add_message::<NarrativeEvent>().insert_resource(active);
        app.add_systems(Update, clear_active_computer_message);
        app.update();

        assert!(drain(&mut app).is_empty(), "clearing is not itself a beat");
        assert!(app
            .world()
            .resource::<ActiveComputerMessage>()
            .current
            .is_none());
    }
}
