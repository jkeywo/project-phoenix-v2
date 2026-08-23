//! Scenario-state projection — a read-only debug surface on the structured
//! observability pipeline (issue #1148, PRD #1144).
//!
//! # What this is
//!
//! A flag-gated projection of the running scenario's working state — the flags
//! table, the objective table, the pending triggers with their eligibility, the
//! delayed-action and deadline queues, the commitments board and the comms
//! dossier — off the authoritative [`WorldContentRuntime`] and the objective
//! manager. It answers a scenario author's "why didn't the story beat fire?"
//! without reading the opaque snapshot save or the one-way sim digest hash.
//!
//! Rhai runtime internals — handler registrations, op budgets — are deliberately
//! NOT projected (PRD #1144 out-of-scope): they are the engine's business, not
//! the scenario's working state.
//!
//! # Determinism
//!
//! [`collect_scenario_state`] is a pure read off authoritative resources and
//! [`publish_scenario_state`] takes them by `Res` (never `ResMut`), so no
//! reading here touches a folded resource or its change-detection tick. The
//! surface's own state — [`ScenarioStateCapture`] and
//! [`DebugScenarioStateEnabled`] — is declared `StateClass::Presentation` at
//! `DebugPlugin::build`, so `world_digest` never folds it. Enabling capture
//! therefore leaves a seeded digest byte-identical, proven by
//! `tests/scenario_state.rs`.
//!
//! # The transport recipe this copies
//!
//! It follows the station-activity slice (#1145) exactly: one payload struct in
//! [`crate::debug::payload`], one `encode_*` in [`crate::core::codec`], a
//! [`DebugFlag`](crate::core::messages::DebugFlag) variant and its bridge
//! plumbing, and a JSON-driven dock renderer. The only shape difference from the
//! station-activity tracker is that there are no always-on counters to feed:
//! scenario state is authoritative already, so the whole surface is the
//! flag-gated publish.

use bevy::prelude::*;

use crate::bounded_history::BoundedRing;
use crate::debug::payload::{
    PredicateValue, ScenarioCommitment, ScenarioDeadline, ScenarioDelayedAction,
    ScenarioDossierEntry, ScenarioFlag, ScenarioObjective, ScenarioStatePayload, ScenarioTrigger,
    TriggerFire,
};
use crate::objectives::ObjectiveManager;
use crate::world::config::{TriggerAction, TriggerCondition, WorldConfig};
use crate::world::flags::{
    counter_in_chain, flag_in_chain, CmpOp, FactContext, FlagStore, Operand, Predicate,
};
use crate::world::server::{ObjectiveManagerRes, WorldContentRuntime};

/// Whether the scenario-state debug output is being rendered (issue #1148).
///
/// Gates only the JSON *publish*: unlike the station-activity tracker there are
/// no counters behind it, because scenario state is authoritative already. Read
/// back in `ServerMessage::DebugState`; flipped from the host cog's Debug tab
/// (`wasm_toggle_scenario_state`) and from a connected phone
/// (`DebugFlag::ScenarioState`).
#[derive(Resource, Default, Debug)]
pub struct DebugScenarioStateEnabled(pub bool);

/// The latest scenario-state JSON, when capture is enabled (issue #1148).
///
/// The target-agnostic sink, exactly like `StationActivityCapture`: on the
/// browser host the publish system ALSO writes the WASM bridge thread-local the
/// dock reads, but every target keeps the JSON here so the headless report path
/// and the determinism guard can read it without a browser. `None` until the
/// first publish; never folded into the digest.
#[derive(Resource, Default, Debug)]
pub struct ScenarioStateCapture(pub Option<String>);

/// Ring-depth fallback when no world config authors one (issue #1151). The only
/// sanctioned hardcoded copy is the config serde default
/// (`[global] trigger_fire_history_depth`, AGENTS.md #11); this mirrors it for a
/// bare-`App` fixture that registered no `WorldConfig`.
const DEFAULT_FIRE_HISTORY_DEPTH: usize = 16;

/// The value recorded for a predicate atom that carries no flag-store reading in
/// the world-trigger context — an AI-policy `fact` / `history` atom a world
/// `when` gate never uses. The extractor stays total (it can quote any
/// predicate) without inventing a reading the world context does not have.
const FIRE_VALUE_UNAVAILABLE: &str = "n/a";

/// The bounded per-trigger record of recent trigger fires with the predicate
/// values observed at each (issue #1151).
///
/// # A read-only projection, never authoritative state
///
/// Declared `StateClass::Presentation` at `DebugPlugin::build`, exactly like the
/// scenario capture and the station-activity counters, so `world_digest` never
/// folds it. [`record_trigger_fires`] reads the authoritative
/// [`WorldContentRuntime`] by `Res` (never `ResMut`) and writes ONLY here, so
/// recording fire history cannot move the sim or its #894 digest whether capture
/// is on or off — proven by `tests/scenario_state.rs`.
///
/// # How a fire is detected without touching authoritative state
///
/// The recorder cannot live in `TriggerState` (that is folded into the digest),
/// so it detects fires by DIFFING each trigger's authoritative
/// `last_fired_elapsed` against the value it saw last tick: a change to a new
/// `Some(..)` is a new fire (once-only `None→Some`, or a `repeat` re-fire whose
/// elapsed advanced). A trigger reset (`Some→None`) is not a fire. The
/// `observed` guard means a trigger that fired BEFORE capture began is not
/// mis-recorded as firing on the first captured tick.
///
/// The records are parallel to `WorldContentRuntime.trigger_states` by index —
/// the same order [`collect_scenario_state`] emits triggers in — so
/// [`Self::fire_history`] keys straight off the collection index. Grown in place
/// when the roster extends (a layer's triggers append) and rebuilt whenever the
/// table is RESHAPED.
///
/// # Why length is not enough (issue #1045)
///
/// It reconciled on length alone, which was sound while the table only ever grew
/// and was cleared whole. Script-in-layers made a removal from the MIDDLE of it
/// possible: unload one layer and load another between two samples and the count
/// can come back identical while every index past the removal now names a
/// different trigger — so the ring built for the trigger that used to be at index
/// 4 would go on collecting index 4's fires, and the AAR would attribute them to
/// the wrong scenario beat. `WorldContentRuntime::trigger_table_generation` moves
/// on every reshape, so this rebuilds when it does.
#[derive(Resource, Debug, Default)]
pub struct TriggerFireRecorder {
    /// Per-trigger fire record, indexed like `trigger_states`.
    per_trigger: Vec<TriggerFireRecord>,
    /// The ring depth last applied, so a retuned config re-caps existing rings.
    depth: usize,
    /// The `trigger_table_generation` these records were built against. A change
    /// means the table was reshaped and every index may now mean something else.
    generation: u64,
}

/// One trigger's fire ring plus the fire-detection baseline.
#[derive(Debug)]
struct TriggerFireRecord {
    ring: BoundedRing<TriggerFire>,
    /// The `last_fired_elapsed` seen last tick, to detect a new fire.
    last_seen_fired_at: Option<f32>,
    /// `false` until this trigger has been observed once. Guards against
    /// recording a pre-capture fire on the first captured tick.
    observed: bool,
}

impl TriggerFireRecord {
    fn new(depth: usize) -> Self {
        Self {
            ring: BoundedRing::new(depth),
            last_seen_fired_at: None,
            observed: false,
        }
    }
}

impl TriggerFireRecorder {
    /// Reconcile the per-trigger records with the live roster, then record any
    /// new fires this tick. Pure w.r.t. `runtime` (takes `&`), so it is unit
    /// testable without an `App`.
    fn sync_and_record(&mut self, runtime: &WorldContentRuntime, depth: usize) {
        let n = runtime.trigger_states.len();

        // Roster reconciliation. A RESHAPE (any merge or layer retraction, which
        // is what moves the generation) invalidates every index, so rebuild fresh;
        // otherwise grow in place, so an appended layer's triggers do not cost the
        // existing rows their history. The length check stays as the belt to that
        // brace, covering any shrink a future writer makes without the counter.
        if self.generation != runtime.trigger_table_generation || self.per_trigger.len() > n {
            self.per_trigger.clear();
            self.generation = runtime.trigger_table_generation;
        }
        while self.per_trigger.len() < n {
            self.per_trigger.push(TriggerFireRecord::new(depth));
        }

        // Re-cap on a retuned depth (a no-op on the common unchanged tick).
        if self.depth != depth {
            for rec in &mut self.per_trigger {
                rec.ring.set_capacity(depth);
            }
            self.depth = depth;
        }

        let chain: [&FlagStore; 1] = [&runtime.flags];
        for (idx, state) in runtime.trigger_states.iter().enumerate() {
            let rec = &mut self.per_trigger[idx];
            let cur = state.last_fired_elapsed;

            // First observation seeds the baseline without recording: we cannot
            // tell whether an already-fired trigger fired before or during
            // capture, so we do not attribute a fire to this tick.
            if !rec.observed {
                rec.observed = true;
                rec.last_seen_fired_at = cur;
                continue;
            }

            let is_new_fire = cur.is_some() && cur != rec.last_seen_fired_at;
            rec.last_seen_fired_at = cur;
            if !is_new_fire {
                continue;
            }

            let mut predicate_values = Vec::new();
            condition_atom_values(&state.trigger.condition, &chain, &mut predicate_values);
            if let Some(pred) = &state.trigger.when {
                predicate_atom_values(pred, &chain, &mut predicate_values);
            }
            rec.ring.push(TriggerFire {
                fired_secs: cur.unwrap_or(0.0),
                predicate_values,
            });
        }
    }

    /// The recorded fire history for the trigger at collection index `idx`,
    /// oldest first. Empty for a trigger with no ring (out of range) or no fires.
    fn fire_history(&self, idx: usize) -> Vec<TriggerFire> {
        self.per_trigger
            .get(idx)
            .map(|rec| rec.ring.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// Record any trigger fires this tick into the bounded per-trigger rings
/// (issue #1151). Gated on the scenario-state debug flag, ordered after
/// `SimSet::Broadcast` (the trigger pipeline's end-of-tick state) and before
/// [`publish_scenario_state`], which reads the rings it fills.
///
/// Read-only w.r.t. every folded resource: it borrows [`WorldContentRuntime`]
/// and [`WorldConfig`] by `Res` and writes only the `Presentation`-class
/// recorder, so its running or not cannot move the digest. The resources are
/// `Option` so a bare-`App` fixture that registered neither still runs (a no-op).
pub fn record_trigger_fires(
    runtime: Option<Res<WorldContentRuntime>>,
    world_config: Option<Res<WorldConfig>>,
    mut recorder: ResMut<TriggerFireRecorder>,
) {
    let Some(runtime) = runtime.as_deref() else {
        return;
    };
    let depth = world_config
        .as_deref()
        .map(|wc| wc.global.trigger_fire_history_depth as usize)
        .unwrap_or(DEFAULT_FIRE_HISTORY_DEPTH);
    recorder.sync_and_record(runtime, depth);
}

/// Collect the flag-store atoms of a predicate with their observed values,
/// left-to-right in tree order (deterministic), against `chain` (issue #1151).
///
/// Only the flag-family atoms a world `when` gate uses carry a reading here;
/// `fact` / `history` atoms are AI-policy grammar and record as
/// [`FIRE_VALUE_UNAVAILABLE`] so the extractor stays total.
fn predicate_atom_values(pred: &Predicate, chain: &[&FlagStore], out: &mut Vec<PredicateValue>) {
    match pred {
        Predicate::Flag { name } => out.push(PredicateValue {
            atom: format!("flag({name})"),
            value: flag_in_chain(chain, name).to_string(),
        }),
        Predicate::Counter { name, .. } => out.push(PredicateValue {
            atom: format!("counter({name})"),
            value: counter_in_chain(chain, name).to_string(),
        }),
        Predicate::Bool(b) => out.push(PredicateValue {
            atom: b.to_string(),
            value: b.to_string(),
        }),
        Predicate::Not(inner) => predicate_atom_values(inner, chain, out),
        Predicate::And(a, b) | Predicate::Or(a, b) => {
            predicate_atom_values(a, chain, out);
            predicate_atom_values(b, chain, out);
        }
        Predicate::Fact { .. } | Predicate::History { .. } => out.push(PredicateValue {
            atom: render_predicate(pred),
            value: FIRE_VALUE_UNAVAILABLE.to_string(),
        }),
    }
}

/// Collect the flag-store atom of a flag-referencing `condition`, if any, with
/// its observed value (issue #1151).
///
/// Event / entity / timer conditions carry no flag-store reading — their timing
/// is the fire record's `fired_secs` — so only `on_flag_set` / `on_flag_cleared`
/// contribute an atom, and it reads back in the same `flag(name)` vocabulary the
/// `when` atoms use.
fn condition_atom_values(
    condition: &TriggerCondition,
    chain: &[&FlagStore],
    out: &mut Vec<PredicateValue>,
) {
    match condition {
        TriggerCondition::OnFlagSet { name } | TriggerCondition::OnFlagCleared { name } => {
            out.push(PredicateValue {
                atom: format!("flag({name})"),
                value: flag_in_chain(chain, name).to_string(),
            });
        }
        _ => {}
    }
}

/// Project the scenario runtime + objective manager into the wire payload
/// (issue #1148). Pure and Bevy-agnostic: the whole surface is testable without
/// an `App`.
///
/// The public two-argument form carries an EMPTY fire history for every trigger;
/// [`collect_scenario_state_with_fires`] is the form the flag-gated publish and
/// the headless report use to fold in the recorded fires (issue #1151).
///
/// Flags are sorted by name; every other collection is emitted in its authored
/// / insertion order, which the underlying stores already keep deterministic
/// (`trigger_states`, `pending_delayed_actions`, `DeadlineTable::records`,
/// `CommitmentLedger::records` and `EvidenceLog::entries` are all `Vec`s the
/// world file / the run's own history orders). The result is byte-identical JSON
/// for identical state.
///
/// Trigger eligibility (`when_holds`) is evaluated against the base world flag
/// store. Sub-world layer flag chains are not walked here: PRD #342 flattened
/// runtime state to one world per session, and #985 removed the only authoring
/// path that produced layer-owned triggers, so the base store IS the chain for
/// every live trigger.
pub fn collect_scenario_state(
    runtime: &WorldContentRuntime,
    objectives: &ObjectiveManager,
) -> ScenarioStatePayload {
    collect_scenario_state_with_fires(runtime, objectives, &TriggerFireRecorder::default())
}

/// Project the scenario runtime + objective manager into the wire payload,
/// folding in each trigger's recorded fire history from `recorder` (issue #1151).
///
/// Identical to [`collect_scenario_state`] but for the per-trigger
/// [`ScenarioTrigger::fire_history`]: the recorder's rings are parallel to
/// `trigger_states` by index, the same order this collects them in, so the fire
/// history keys straight off the enumeration index.
pub fn collect_scenario_state_with_fires(
    runtime: &WorldContentRuntime,
    objectives: &ObjectiveManager,
    recorder: &TriggerFireRecorder,
) -> ScenarioStatePayload {
    let mut payload = ScenarioStatePayload::empty();

    // Flags — sorted by name for a deterministic table.
    let mut flags: Vec<ScenarioFlag> = runtime
        .flags
        .iter()
        .map(|(name, value)| ScenarioFlag {
            name: name.to_string(),
            value,
        })
        .collect();
    flags.sort_by(|a, b| a.name.cmp(&b.name));
    payload.flags = flags;

    // Objectives — the manager's own mandatory-first order.
    payload.objectives = objectives
        .debug_views()
        .map(|o| ScenarioObjective {
            id: o.id.to_string(),
            status: o.status.clone(),
            mandatory: o.mandatory,
            base_priority: o.base_priority,
            directive: o.directive.clone(),
        })
        .collect();

    // Triggers — pending state + eligibility. The flag chain is the base store
    // (see the fn docs).
    let chain: [&FlagStore; 1] = [&runtime.flags];
    payload.triggers = runtime
        .trigger_states
        .iter()
        .enumerate()
        .map(|(idx, state)| {
            let when_holds = match &state.trigger.when {
                Some(pred) => pred.evaluate(&chain),
                None => true,
            };
            ScenarioTrigger {
                id: state.trigger.id.clone(),
                condition: render_condition(&state.trigger.condition),
                when: state.trigger.when.as_ref().map(render_predicate),
                repeat: state.trigger.repeat,
                fired: state.fired,
                // Once-only: armed while it has not fired. Repeat: always armed.
                pending: state.trigger.repeat || !state.fired,
                when_holds,
                last_fired_secs: state.last_fired_elapsed,
                fire_history: recorder.fire_history(idx),
            }
        })
        .collect();

    // Delayed-action queue — dispatch order.
    payload.delayed_actions = runtime
        .pending_delayed_actions
        .iter()
        .map(|pda| ScenarioDelayedAction {
            action: render_action(&pda.action),
            entity: pda.entity_name.clone(),
            fire_at_secs: pda.fire_at_elapsed,
        })
        .collect();

    // Deadline queue — authored order.
    payload.deadlines = runtime
        .deadlines
        .records
        .iter()
        .map(|d| ScenarioDeadline {
            id: d.id.clone(),
            label: d.label.clone(),
            visible: d.visible,
            due_tick: d.due_tick,
            state: d.state.as_str().to_string(),
        })
        .collect();

    // Commitments board — oldest first.
    payload.commitments = runtime
        .commitments
        .records
        .iter()
        .map(|c| ScenarioCommitment {
            id: c.id.clone(),
            made_to: c.made_to.clone(),
            terms: c.terms.clone(),
            resolves_when: c.resolves_when.clone(),
            state: c.state.as_str().to_string(),
            made_at_tick: c.made_at_tick,
            resolved_at_tick: c.resolved_at_tick,
        })
        .collect();

    // Comms dossier — oldest finding first.
    payload.dossier = runtime
        .evidence
        .entries
        .iter()
        .map(|e| ScenarioDossierEntry {
            subject_uuid: e.subject_uuid.clone(),
            text: e.text.clone(),
            provenance: e.provenance.as_str().to_string(),
            gathered_at_tick: e.gathered_at_tick,
        })
        .collect();

    payload
}

/// Project the scenario state to JSON when capture is enabled (flag-gated).
///
/// Read-only: it borrows the runtime and objective manager by `Res` and writes
/// only the presentation-class capture (and, on the browser host, the WASM
/// bridge thread-local the dock reads), so its running or not cannot move the
/// digest.
///
/// The runtime resources are `Option` so a bare-`App` fixture that registered
/// neither still runs (it publishes an empty, version-stamped payload). The
/// [`TriggerFireRecorder`] is folded in so each trigger carries its recorded
/// fire history (issue #1151); [`record_trigger_fires`] runs before this in the
/// same tick, so the rings it reads are current.
pub fn publish_scenario_state(
    runtime: Option<Res<WorldContentRuntime>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    recorder: Res<TriggerFireRecorder>,
    mut capture: ResMut<ScenarioStateCapture>,
) {
    let payload = match (runtime.as_deref(), objectives.as_deref()) {
        (Some(runtime), Some(objectives)) => {
            collect_scenario_state_with_fires(runtime, &objectives.0, &recorder)
        }
        (Some(runtime), None) => {
            collect_scenario_state_with_fires(runtime, &ObjectiveManager::default(), &recorder)
        }
        // No world loaded — an empty, version-stamped payload.
        (None, _) => ScenarioStatePayload::empty(),
    };
    let json = crate::core::codec::encode_scenario_state(&payload);

    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_scenario_state_string(json.clone());

    capture.0 = Some(json);
}

// ── Renderers ───────────────────────────────────────────────────────────────
//
// `TriggerCondition`, `TriggerAction` and `Predicate` are not `serde` types, so
// each is rendered to a stable compact string. The vocabulary mirrors the world
// TOML / script keywords, so the string an author reads back is the one they
// wrote — and it is the vocabulary #1151's per-fire records quote predicate
// values against.

/// Render a `SimTick`/seconds float compactly and deterministically: whole
/// numbers drop the fraction, so `on_timer(after_secs=30)` rather than `30.0`.
/// Rust's float formatting is deterministic, so two hosts render an identical
/// value identically.
fn render_f32(value: f32) -> String {
    if value.fract() == 0.0 && value.is_finite() {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Render a trigger's firing condition to a stable compact string.
pub fn render_condition(condition: &TriggerCondition) -> String {
    match condition {
        TriggerCondition::OnDestroyed { entity_name } => format!("on_destroyed({entity_name})"),
        TriggerCondition::OnAllDestroyed { group, after_secs } => {
            format!(
                "on_all_destroyed({group}, after_secs={})",
                render_f32(*after_secs)
            )
        }
        TriggerCondition::OnAttacked { entity_name } => format!("on_attacked({entity_name})"),
        TriggerCondition::OnHullBelow {
            entity_name,
            threshold,
        } => format!(
            "on_hull_below({entity_name}, threshold={})",
            render_f32(*threshold)
        ),
        TriggerCondition::OnTimer { after_secs } => {
            format!("on_timer(after_secs={})", render_f32(*after_secs))
        }
        TriggerCondition::OnHailed { entity_name } => format!("on_hailed({entity_name})"),
        TriggerCondition::OnFlagSet { name } => format!("on_flag_set({name})"),
        TriggerCondition::OnFlagCleared { name } => format!("on_flag_cleared({name})"),
        TriggerCondition::OnWorldLoaded => "on_world_loaded".to_string(),
        TriggerCondition::OnEnteredRegion { entity_name } => {
            format!("on_entered_region({entity_name})")
        }
        TriggerCondition::OnExitedRegion { entity_name } => {
            format!("on_exited_region({entity_name})")
        }
        TriggerCondition::OnWaypointReached {
            entity_name,
            waypoint,
        } => match waypoint {
            Some(wp) => format!("on_waypoint_reached({entity_name}, waypoint={wp})"),
            None => format!("on_waypoint_reached({entity_name})"),
        },
    }
}

/// Render a trigger action to a stable `kind(args)` string. Exhaustive on
/// purpose: a new [`TriggerAction`] variant is a compile error here until it is
/// given a rendering, rather than silently vanishing from the queue view.
pub fn render_action(action: &TriggerAction) -> String {
    match action {
        TriggerAction::AddObjective { id, .. } => format!("add_objective({id})"),
        TriggerAction::CompleteObjective { id } => format!("complete_objective({id})"),
        TriggerAction::FailObjective { id } => format!("fail_objective({id})"),
        TriggerAction::SetAiState { entity, state, .. } => {
            format!("set_ai_state({entity}, {state})")
        }
        TriggerAction::ApplyModifier { entity, tag, .. } => {
            format!("apply_modifier({entity}, {tag})")
        }
        TriggerAction::RemoveModifier { entity, tag, .. } => {
            format!("remove_modifier({entity}, {tag})")
        }
        TriggerAction::ApplyFlag { entity, tag, .. } => format!("apply_flag({entity}, {tag})"),
        TriggerAction::RemoveFlag { entity, tag, .. } => format!("remove_flag({entity}, {tag})"),
        TriggerAction::ApplyIntModifier { entity, tag, .. } => {
            format!("apply_int_modifier({entity}, {tag})")
        }
        TriggerAction::RemoveIntModifier { entity, tag, .. } => {
            format!("remove_int_modifier({entity}, {tag})")
        }
        TriggerAction::GameOver { .. } => "game_over".to_string(),
        TriggerAction::LoadWorld { path } => format!("load_world({path})"),
        TriggerAction::UnloadWorld { path } => format!("unload_world({path})"),
        TriggerAction::SetWorldFlag { name } => format!("set_world_flag({name})"),
        TriggerAction::ClearWorldFlag { name } => format!("clear_world_flag({name})"),
        TriggerAction::IncrementWorldFlag { name, by } => {
            format!("increment_world_flag({name}, by={by})")
        }
        TriggerAction::SetWorldFlagValue { name, value } => {
            format!("set_world_flag_value({name}, value={value})")
        }
        TriggerAction::SpawnEntity { name, .. } => format!("spawn_entity({name})"),
        TriggerAction::DestroyEntity { entity } => format!("destroy_entity({entity})"),
        TriggerAction::AddFactionEnemy { faction, enemy } => {
            format!("add_faction_enemy({faction}, {enemy})")
        }
        TriggerAction::RemoveFactionEnemy { faction, enemy } => {
            format!("remove_faction_enemy({faction}, {enemy})")
        }
        TriggerAction::ResetTrigger { id } => format!("reset_trigger({id})"),
    }
}

/// Render a comparison operator the way the predicate grammar spells it.
fn render_cmp(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Ge => ">=",
        CmpOp::Gt => ">",
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Le => "<=",
        CmpOp::Lt => "<",
    }
}

/// Render a predicate operand back the way an author typed it.
fn render_operand(operand: &Operand) -> String {
    match operand {
        Operand::Number(n) => {
            if n.fract() == 0.0 && n.is_finite() {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            }
        }
        Operand::Param(name) => format!("param({name})"),
    }
}

/// The keyword prefix a `fact(...)`-family atom reads under.
fn fact_prefix(context: FactContext) -> &'static str {
    match context {
        FactContext::SelfCtx => "fact",
        FactContext::Candidate => "candidate_fact",
        FactContext::Target => "target_fact",
        FactContext::Memory => "memory",
        FactContext::StateTime => "state_time",
    }
}

/// Render a parsed predicate to a stable expression string.
///
/// World-trigger `when` gates are almost always `flag(...)` / `counter(...)`
/// atoms composed with `and`/`or`/`not`; the fact / history atoms are AI-policy
/// grammar that a world `when` does not use, but they are rendered too so the
/// function is total and #1151 can quote any predicate's atoms.
pub fn render_predicate(pred: &Predicate) -> String {
    match pred {
        Predicate::Flag { name } => format!("flag({name})"),
        Predicate::Counter { name, op, rhs } => {
            format!("counter({name}) {} {rhs}", render_cmp(*op))
        }
        Predicate::Fact {
            context,
            name,
            op,
            rhs,
        } => {
            let lhs = match context {
                // `state_time` takes no argument (its `name` is a fixed literal).
                FactContext::StateTime => "state_time".to_string(),
                other => format!("{}({name})", fact_prefix(*other)),
            };
            format!("{lhs} {} {}", render_cmp(*op), render_operand(rhs))
        }
        Predicate::History {
            reducer,
            window,
            op,
            rhs,
        } => format!(
            "history({}, {}, {}) {} {}",
            reducer.name(),
            window.fact,
            render_operand(&window.ticks),
            render_cmp(*op),
            render_operand(rhs),
        ),
        Predicate::Bool(b) => b.to_string(),
        Predicate::Not(inner) => format!("!({})", render_predicate(inner)),
        Predicate::And(a, b) => {
            format!("({} and {})", render_predicate(a), render_predicate(b))
        }
        Predicate::Or(a, b) => format!("({} or {})", render_predicate(a), render_predicate(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{AiDirective, ObjectiveStatus};
    use crate::dossier::evidence::EvidenceProvenance;
    use crate::world::commitments::CommitmentOutcome;
    use crate::world::config::Trigger;
    use crate::world::content::TriggerState;
    use crate::world::deadlines::{Deadline, DeadlineHandler};
    use crate::world::delayed::DelayedAction;
    use crate::world::flags::parse_predicate;

    const HZ: f32 = 60.0;

    fn trigger_state(
        id: Option<&str>,
        condition: TriggerCondition,
        when: Option<&str>,
        repeat: bool,
        fired: bool,
    ) -> TriggerState {
        TriggerState {
            trigger: Trigger {
                condition,
                when: when.map(|w| parse_predicate(w).expect("test predicate parses")),
                id: id.map(str::to_string),
                repeat,
                cooldown_secs: None,
            },
            fired,
            origin_layer: None,
            seen_destroyed: Default::default(),
            last_fired_elapsed: None,
        }
    }

    /// Given an authored flag store, the payload contains every SET flag, sorted
    /// by name, and nothing for an unset one.
    #[test]
    fn flags_are_projected_sorted_and_only_when_set() {
        let mut runtime = WorldContentRuntime::default();
        runtime.flags.set_flag("zeta");
        runtime.flags.set_flag_value("alpha", 5);
        runtime.flags.set_flag_value("cleared", 0); // removed by the store

        let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());

        let names: Vec<&str> = payload.flags.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta"], "sorted, cleared flag absent");
        assert_eq!(payload.flags[0].value, 5);
        assert_eq!(payload.flags[1].value, 1);
    }

    /// Objectives carry id, status, base priority and directive, mandatory
    /// first — the id/status/score/directive the AC names.
    #[test]
    fn objectives_carry_status_priority_and_directive() {
        let mut manager = ObjectiveManager::default();
        // Optional first, then a mandatory — so the mandatory-first ordering is
        // actually exercised rather than trivially satisfied.
        manager.add_full(
            "scan",
            "objective.scan",
            false,
            vec![],
            AiDirective::None,
            crate::objectives::UtilityConfig {
                base_priority: 2.0,
                ..Default::default()
            },
            Default::default(),
        );
        manager.add_full(
            "kill",
            "objective.kill",
            true,
            vec![],
            AiDirective::Destroy {
                target: "raider".into(),
            },
            crate::objectives::UtilityConfig {
                base_priority: 7.0,
                ..Default::default()
            },
            Default::default(),
        );
        manager.complete("scan");

        let runtime = WorldContentRuntime::default();
        let payload = collect_scenario_state(&runtime, &manager);

        assert_eq!(payload.objectives.len(), 2);
        // Mandatory first.
        let kill = &payload.objectives[0];
        assert_eq!(kill.id, "kill");
        assert_eq!(kill.status, ObjectiveStatus::Active);
        assert!(kill.mandatory);
        assert_eq!(kill.base_priority, 7.0);
        assert_eq!(
            kill.directive,
            AiDirective::Destroy {
                target: "raider".into()
            }
        );
        let scan = &payload.objectives[1];
        assert_eq!(scan.id, "scan");
        assert_eq!(scan.status, ObjectiveStatus::Completed);
        assert!(!scan.mandatory);
        assert_eq!(scan.base_priority, 2.0);
        assert_eq!(scan.directive, AiDirective::None);
    }

    /// A trigger whose `when` predicate references an unset flag is pending but
    /// not eligible; setting the flag makes `when_holds` true. This is the
    /// "armed vs waiting" evidence the surface exists for.
    #[test]
    fn triggers_report_pending_and_eligibility() {
        let mut runtime = WorldContentRuntime::default();
        runtime.trigger_states.push(trigger_state(
            Some("beat"),
            TriggerCondition::OnTimer { after_secs: 30.0 },
            Some("flag(ready)"),
            false,
            false,
        ));

        // Flag unset: armed but waiting on its gate.
        let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
        let trig = &payload.triggers[0];
        assert_eq!(trig.id.as_deref(), Some("beat"));
        assert_eq!(trig.condition, "on_timer(after_secs=30)");
        assert_eq!(trig.when.as_deref(), Some("flag(ready)"));
        assert!(trig.pending, "unfired once-only trigger is armed");
        assert!(!trig.fired);
        assert!(!trig.when_holds, "gate flag is unset");

        // Set the flag: now eligible.
        runtime.flags.set_flag("ready");
        let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
        assert!(payload.triggers[0].when_holds, "gate flag now holds");
    }

    /// A fired once-only trigger is no longer pending; a repeat trigger stays
    /// armed after firing. A gateless trigger is always eligible.
    #[test]
    fn pending_reflects_lifecycle_and_gateless_is_eligible() {
        let mut runtime = WorldContentRuntime::default();
        runtime.trigger_states.push(trigger_state(
            None,
            TriggerCondition::OnWorldLoaded,
            None,
            false,
            true, // fired once-only
        ));
        runtime.trigger_states.push(trigger_state(
            Some("repeater"),
            TriggerCondition::OnFlagSet {
                name: "tick".into(),
            },
            None,
            true,
            true, // fired but repeatable
        ));

        let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
        assert!(!payload.triggers[0].pending, "fired once-only is spent");
        assert!(payload.triggers[0].when_holds, "no gate is always eligible");
        assert!(payload.triggers[1].pending, "a repeat trigger re-arms");
    }

    /// The delayed-action queue is projected with the action rendered and its
    /// fire time.
    #[test]
    fn delayed_actions_are_projected() {
        let mut runtime = WorldContentRuntime::default();
        runtime.pending_delayed_actions.push(DelayedAction {
            action: TriggerAction::SetWorldFlag {
                name: "reinforce".into(),
            },
            origin_layer: None,
            entity_name: Some("carrier".into()),
            fire_at_elapsed: 45.5,
        });

        let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
        assert_eq!(payload.delayed_actions.len(), 1);
        assert_eq!(
            payload.delayed_actions[0].action,
            "set_world_flag(reinforce)"
        );
        assert_eq!(
            payload.delayed_actions[0].entity.as_deref(),
            Some("carrier")
        );
        assert_eq!(payload.delayed_actions[0].fire_at_secs, 45.5);
    }

    /// Armed deadlines are projected with their state; a cancelled one reads
    /// `cancelled`.
    #[test]
    fn deadlines_are_projected_with_state() {
        let mut runtime = WorldContentRuntime::default();
        let authored = [
            Deadline {
                id: "window".into(),
                label: "world.deadline.window".into(),
                due_secs: 600,
                visible: true,
            },
            Deadline {
                id: "quiet".into(),
                label: String::new(),
                due_secs: 120,
                visible: false,
            },
        ];
        let handlers = [
            DeadlineHandler {
                deadline_id: "window".into(),
                handler: "on_window".into(),
                source_path: "w.rhai".into(),
            },
            DeadlineHandler {
                deadline_id: "quiet".into(),
                handler: "on_quiet".into(),
                source_path: "w.rhai".into(),
            },
        ];
        runtime.deadlines.arm(&authored, &handlers, 0, HZ);

        let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
        assert_eq!(payload.deadlines.len(), 2);
        let window = &payload.deadlines[0];
        assert_eq!(window.id, "window");
        assert_eq!(window.label, "world.deadline.window");
        assert!(window.visible);
        assert_eq!(window.due_tick, 600 * HZ as u64);
        assert_eq!(window.state, "pending");
        assert_eq!(payload.deadlines[1].label, "", "no label authored");
    }

    /// The commitments board projects open and resolved promises with their
    /// state and provenance ticks.
    #[test]
    fn commitments_are_projected() {
        let mut runtime = WorldContentRuntime::default();
        runtime
            .commitments
            .record(
                "passage",
                "strike_committee",
                "terms.passage",
                "resolves.passage",
                10,
            )
            .expect("record");
        runtime
            .commitments
            .record("aid", "colony", "terms.aid", "", 20)
            .expect("record");
        runtime
            .commitments
            .resolve("passage", CommitmentOutcome::Kept, 30);

        let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
        assert_eq!(payload.commitments.len(), 2);
        let passage = &payload.commitments[0];
        assert_eq!(passage.id, "passage");
        assert_eq!(passage.made_to, "strike_committee");
        assert_eq!(passage.terms, "terms.passage");
        assert_eq!(passage.state, "kept");
        assert_eq!(passage.made_at_tick, 10);
        assert_eq!(passage.resolved_at_tick, Some(30));
        let aid = &payload.commitments[1];
        assert_eq!(aid.state, "open");
        assert_eq!(aid.resolved_at_tick, None);
    }

    /// The comms dossier projects each finding with its provenance label.
    #[test]
    fn dossier_findings_are_projected() {
        let mut runtime = WorldContentRuntime::default();
        runtime.evidence.append(
            "uuid-1",
            "evidence.forged_manifest",
            EvidenceProvenance::Records,
            40,
        );
        runtime.evidence.append(
            "uuid-2",
            "evidence.scan_anomaly",
            EvidenceProvenance::Scan,
            50,
        );

        let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
        assert_eq!(payload.dossier.len(), 2);
        assert_eq!(payload.dossier[0].subject_uuid, "uuid-1");
        assert_eq!(payload.dossier[0].text, "evidence.forged_manifest");
        assert_eq!(payload.dossier[0].provenance, "records");
        assert_eq!(payload.dossier[0].gathered_at_tick, 40);
        assert_eq!(payload.dossier[1].provenance, "scan");
    }

    /// An empty runtime projects a version-stamped, empty payload.
    #[test]
    fn empty_runtime_projects_version_stamped_empty() {
        let payload = collect_scenario_state(
            &WorldContentRuntime::default(),
            &ObjectiveManager::default(),
        );
        assert_eq!(
            payload.schema_version,
            crate::debug::payload::DEBUG_SCHEMA_VERSION
        );
        assert!(payload.flags.is_empty());
        assert!(payload.objectives.is_empty());
        assert!(payload.triggers.is_empty());
    }

    // ── Renderers ────────────────────────────────────────────────────────────

    #[test]
    fn conditions_render_to_stable_strings() {
        assert_eq!(
            render_condition(&TriggerCondition::OnDestroyed {
                entity_name: "raider".into()
            }),
            "on_destroyed(raider)"
        );
        assert_eq!(
            render_condition(&TriggerCondition::OnAllDestroyed {
                group: "wing".into(),
                after_secs: 5.0
            }),
            "on_all_destroyed(wing, after_secs=5)"
        );
        assert_eq!(
            render_condition(&TriggerCondition::OnHullBelow {
                entity_name: "boss".into(),
                threshold: 0.25
            }),
            "on_hull_below(boss, threshold=0.25)"
        );
        assert_eq!(
            render_condition(&TriggerCondition::OnWaypointReached {
                entity_name: "convoy".into(),
                waypoint: Some("alpha".into())
            }),
            "on_waypoint_reached(convoy, waypoint=alpha)"
        );
        assert_eq!(
            render_condition(&TriggerCondition::OnWorldLoaded),
            "on_world_loaded"
        );
    }

    #[test]
    fn predicates_render_to_stable_strings() {
        for (src, expected) in [
            ("flag(ready)", "flag(ready)"),
            ("counter(kills) >= 3", "counter(kills) >= 3"),
            ("flag(a) and counter(b) < 2", "(flag(a) and counter(b) < 2)"),
            ("not flag(x)", "!(flag(x))"),
        ] {
            let pred = parse_predicate(src).expect("parses");
            assert_eq!(render_predicate(&pred), expected, "for source {src:?}");
        }
    }

    // ── Trigger fire history recorder (issue #1151) ───────────────────────────

    /// Push a `TriggerState` onto the runtime and return its index, so a test can
    /// mutate its fire fields to simulate the authoritative trigger pipeline.
    fn push_trigger(
        runtime: &mut WorldContentRuntime,
        id: Option<&str>,
        condition: TriggerCondition,
        when: Option<&str>,
        repeat: bool,
    ) -> usize {
        runtime
            .trigger_states
            .push(trigger_state(id, condition, when, repeat, false));
        runtime.trigger_states.len() - 1
    }

    /// Simulate the authoritative fire of the trigger at `idx`: the same two
    /// fields `evaluate_single_trigger` sets when a trigger fires.
    fn simulate_fire(runtime: &mut WorldContentRuntime, idx: usize, elapsed: f32) {
        runtime.trigger_states[idx].fired = true;
        runtime.trigger_states[idx].last_fired_elapsed = Some(elapsed);
    }

    const DEPTH: usize = 4;

    /// A fire records its fire time and the observed values of its `when` gate's
    /// atoms — the evidence an author reconstructs "why did this beat fire" from.
    #[test]
    fn a_fire_records_its_time_and_when_atom_values() {
        let mut runtime = WorldContentRuntime::default();
        let idx = push_trigger(
            &mut runtime,
            Some("beat"),
            TriggerCondition::OnTimer { after_secs: 30.0 },
            Some("flag(ready) and counter(kills) >= 3"),
            false,
        );
        let mut recorder = TriggerFireRecorder::default();

        // First observation seeds the baseline; the trigger has not fired yet.
        recorder.sync_and_record(&runtime, DEPTH);
        assert!(recorder.fire_history(idx).is_empty(), "no fire yet");

        // The pipeline fires it at t=32.5 with the gate flags holding.
        runtime.flags.set_flag("ready");
        runtime.flags.set_flag_value("kills", 5);
        simulate_fire(&mut runtime, idx, 32.5);
        recorder.sync_and_record(&runtime, DEPTH);

        let history = recorder.fire_history(idx);
        assert_eq!(history.len(), 1, "one fire recorded");
        assert_eq!(history[0].fired_secs, 32.5);
        // Atom values quote the `when` vocabulary the surface already renders.
        let values: Vec<(&str, &str)> = history[0]
            .predicate_values
            .iter()
            .map(|v| (v.atom.as_str(), v.value.as_str()))
            .collect();
        assert!(
            values.contains(&("flag(ready)", "true")),
            "the gate flag's observed value, got {values:?}"
        );
        assert!(
            values.contains(&("counter(kills)", "5")),
            "the gate counter's observed reading, got {values:?}"
        );
    }

    /// A flag-referencing `condition` contributes its atom too — the "condition"
    /// half of "the atoms in its condition/when that made it fire".
    #[test]
    fn a_flag_condition_atom_is_recorded() {
        let mut runtime = WorldContentRuntime::default();
        let idx = push_trigger(
            &mut runtime,
            Some("on_alarm"),
            TriggerCondition::OnFlagSet {
                name: "alarm".into(),
            },
            None,
            false,
        );
        let mut recorder = TriggerFireRecorder::default();
        recorder.sync_and_record(&runtime, DEPTH);

        runtime.flags.set_flag("alarm");
        simulate_fire(&mut runtime, idx, 10.0);
        recorder.sync_and_record(&runtime, DEPTH);

        let history = recorder.fire_history(idx);
        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0].predicate_values,
            vec![PredicateValue {
                atom: "flag(alarm)".into(),
                value: "true".into(),
            }],
            "the condition's flag reads back in the flag(name) vocabulary"
        );
    }

    /// The per-trigger ring is bounded: firing past its depth keeps the most
    /// recent `depth` fires and evicts the oldest.
    #[test]
    fn the_ring_caps_at_the_authored_depth() {
        let mut runtime = WorldContentRuntime::default();
        let idx = push_trigger(
            &mut runtime,
            Some("beacon"),
            TriggerCondition::OnTimer { after_secs: 1.0 },
            None,
            true, // repeat, so it can fire many times
        );
        let mut recorder = TriggerFireRecorder::default();
        recorder.sync_and_record(&runtime, 2); // depth 2

        for elapsed in [10.0_f32, 20.0, 30.0, 40.0] {
            simulate_fire(&mut runtime, idx, elapsed);
            recorder.sync_and_record(&runtime, 2);
        }

        let history = recorder.fire_history(idx);
        assert_eq!(history.len(), 2, "ring bounded at depth 2");
        let times: Vec<f32> = history.iter().map(|f| f.fired_secs).collect();
        assert_eq!(
            times,
            vec![30.0, 40.0],
            "the two most recent fires, oldest first"
        );
    }

    /// A trigger that fired BEFORE capture began is not mis-recorded on the first
    /// captured tick: the first observation only seeds the baseline.
    #[test]
    fn a_pre_capture_fire_is_not_recorded() {
        let mut runtime = WorldContentRuntime::default();
        let idx = push_trigger(
            &mut runtime,
            Some("already"),
            TriggerCondition::OnWorldLoaded,
            None,
            false,
        );
        // It fired at t=5 before the recorder ever saw it.
        simulate_fire(&mut runtime, idx, 5.0);

        let mut recorder = TriggerFireRecorder::default();
        recorder.sync_and_record(&runtime, DEPTH);
        assert!(
            recorder.fire_history(idx).is_empty(),
            "a fire before capture must not be attributed to the first tick"
        );
    }

    /// A `ResetTrigger` (last_fired_elapsed Some→None) is not a fire, and a
    /// subsequent genuine re-fire after the reset IS recorded.
    #[test]
    fn a_reset_is_not_a_fire_but_the_next_fire_is() {
        let mut runtime = WorldContentRuntime::default();
        let idx = push_trigger(
            &mut runtime,
            Some("resettable"),
            TriggerCondition::OnWorldLoaded,
            None,
            false,
        );
        let mut recorder = TriggerFireRecorder::default();
        recorder.sync_and_record(&runtime, DEPTH);

        // Fire, then reset (what `reset_triggers_by_id` does to the fire fields).
        simulate_fire(&mut runtime, idx, 5.0);
        recorder.sync_and_record(&runtime, DEPTH);
        runtime.trigger_states[idx].fired = false;
        runtime.trigger_states[idx].last_fired_elapsed = None;
        recorder.sync_and_record(&runtime, DEPTH);
        assert_eq!(
            recorder.fire_history(idx).len(),
            1,
            "the reset is not a fire"
        );

        // A fresh fire after the reset is recorded.
        simulate_fire(&mut runtime, idx, 12.0);
        recorder.sync_and_record(&runtime, DEPTH);
        let history = recorder.fire_history(idx);
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].fired_secs, 12.0);
    }

    /// The recorded fire history flows into the collected payload at the matching
    /// trigger index — the additive `fire_history` field the dock reads.
    #[test]
    fn fire_history_lands_on_the_collected_trigger() {
        let mut runtime = WorldContentRuntime::default();
        let idx = push_trigger(
            &mut runtime,
            Some("beat"),
            TriggerCondition::OnTimer { after_secs: 30.0 },
            Some("flag(ready)"),
            false,
        );
        let mut recorder = TriggerFireRecorder::default();
        recorder.sync_and_record(&runtime, DEPTH);
        runtime.flags.set_flag("ready");
        simulate_fire(&mut runtime, idx, 30.0);
        recorder.sync_and_record(&runtime, DEPTH);

        let payload =
            collect_scenario_state_with_fires(&runtime, &ObjectiveManager::default(), &recorder);
        assert_eq!(payload.triggers[idx].fire_history.len(), 1);
        assert_eq!(payload.triggers[idx].fire_history[0].fired_secs, 30.0);
        // The public two-arg form carries an empty fire history (no recorder).
        let plain = collect_scenario_state(&runtime, &ObjectiveManager::default());
        assert!(plain.triggers[idx].fire_history.is_empty());
    }

    /// When the roster shrinks (a world reload), the recorder rebuilds rather than
    /// mis-aligning old rings onto new triggers.
    #[test]
    fn a_shrinking_roster_rebuilds_the_records() {
        let mut runtime = WorldContentRuntime::default();
        let a = push_trigger(
            &mut runtime,
            Some("a"),
            TriggerCondition::OnWorldLoaded,
            None,
            true,
        );
        let _b = push_trigger(
            &mut runtime,
            Some("b"),
            TriggerCondition::OnWorldLoaded,
            None,
            true,
        );
        let mut recorder = TriggerFireRecorder::default();
        recorder.sync_and_record(&runtime, DEPTH);
        simulate_fire(&mut runtime, a, 5.0);
        recorder.sync_and_record(&runtime, DEPTH);
        assert_eq!(recorder.fire_history(a).len(), 1);

        // A reload leaves a single trigger; the recorder must not carry the old
        // ring onto it.
        runtime.trigger_states.truncate(1);
        recorder.sync_and_record(&runtime, DEPTH);
        assert!(
            recorder.fire_history(0).is_empty(),
            "the rebuilt record starts empty and re-seeds its baseline"
        );
    }
}
