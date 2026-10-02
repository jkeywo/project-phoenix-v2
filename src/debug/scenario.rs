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
//! [`DebugSurface`](crate::core::debug_surface::DebugSurface) identity and its bridge
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
/// (the generic Debug Surface setter) and from a connected phone
/// (`DebugSurface::ScenarioState`).
#[derive(Resource, Default, Debug)]
pub struct DebugScenarioStateEnabled(pub bool);

impl crate::debug::catalogue::DebugSurfaceState for DebugScenarioStateEnabled {
    fn is_enabled(&self) -> bool {
        self.0
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.0 = enabled;
    }
}

/// Module-owned adapter for the scenario-state Debug Surface.
pub const DEBUG_SCENARIO_STATE_ADAPTER: crate::debug::catalogue::DebugSurfaceAdapter =
    crate::debug::catalogue::DebugSurfaceAdapter::for_resource::<DebugScenarioStateEnabled>(
        crate::core::debug_surface::DebugSurface::ScenarioState,
    );

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
/// The records are parallel to `WorldContentRuntime.triggers` by index —
/// the same order [`collect_scenario_state`] emits triggers in — so
/// [`Self::fire_history`] keys straight off the collection index. Rebuilt whenever the
/// registry changes shape or restores its continuation.
///
/// # Why length is not enough (issue #1045)
///
/// It reconciled on length alone, which was sound while the table only ever grew
/// and was cleared whole. Script-in-layers made a removal from the MIDDLE of it
/// possible: unload one layer and load another between two samples and the count
/// can come back identical while every index past the removal now names a
/// different trigger — so the ring built for the trigger that used to be at index
/// 4 would go on collecting index 4's fires, and the AAR would attribute them to
/// the wrong scenario beat. `WorldTriggerRegistry::generation` moves
/// on every reshape, so this rebuilds when it does.
#[derive(Resource, Debug, Default)]
pub struct TriggerFireRecorder {
    /// Per-trigger fire record, indexed like `triggers`.
    per_trigger: Vec<TriggerFireRecord>,
    /// The ring depth last applied, so a retuned config re-caps existing rings.
    depth: usize,
    /// The registry generation these records were built against. A change
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
        let n = runtime.triggers.len();

        // Registry generation invalidates index history after any reshape or
        // successful continuation restore. A new recorder also starts empty.
        if self.generation != runtime.triggers.generation() || self.per_trigger.len() > n {
            self.per_trigger.clear();
            self.generation = runtime.triggers.generation();
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
        for (idx, state) in runtime.triggers.iter().enumerate() {
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
/// (`triggers`, `pending_delayed_actions`, `DeadlineTable::records`,
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
/// `triggers` by index, the same order this collects them in, so the fire
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
        .triggers
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
        TriggerCondition::Manual => "gm_event".to_string(),
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
        TriggerAction::Presentation { ship, cue } => format!("presentation({ship}, {cue:?})"),
        TriggerAction::SetContactInformation { ship, change } => {
            format!("set_contact_information({ship}, {})", change.target())
        }
        TriggerAction::SetNpcDoctrine { entity, id } => format!("set_npc_doctrine({entity}, {id})"),
        TriggerAction::AddObjective { id, .. } => format!("add_objective({id})"),
        TriggerAction::AddObjectiveInstance { spec, .. } => format!(
            "add_objective({}, instance={})",
            spec.key.objective_id, spec.key.instance_id
        ),
        TriggerAction::CompleteObjective { id } => format!("complete_objective({id})"),
        TriggerAction::FailObjective { id } => format!("fail_objective({id})"),
        TriggerAction::CompleteObjectiveInstance { key } => format!(
            "complete_objective({}, instance={})",
            key.objective_id, key.instance_id
        ),
        TriggerAction::FailObjectiveInstance { key } => format!(
            "fail_objective({}, instance={})",
            key.objective_id, key.instance_id
        ),
        TriggerAction::SetObjectiveInstanceProgress { key, progress } => format!(
            "set_objective_progress({}, instance={}, progress={progress})",
            key.objective_id, key.instance_id
        ),
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
        TriggerAction::Addressed { .. } => "addressed action".to_string(),
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
#[path = "scenario_tests.rs"]
mod tests;
