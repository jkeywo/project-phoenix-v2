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

use crate::debug::payload::{
    ScenarioCommitment, ScenarioDeadline, ScenarioDelayedAction, ScenarioDossierEntry,
    ScenarioFlag, ScenarioObjective, ScenarioStatePayload, ScenarioTrigger,
};
use crate::objectives::ObjectiveManager;
use crate::world::config::{TriggerAction, TriggerCondition};
use crate::world::flags::{CmpOp, FactContext, FlagStore, Operand, Predicate};
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

/// Project the scenario runtime + objective manager into the wire payload
/// (issue #1148). Pure and Bevy-agnostic: the whole surface is testable without
/// an `App`.
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
        .map(|state| {
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
/// neither still runs (it publishes an empty, version-stamped payload).
pub fn publish_scenario_state(
    runtime: Option<Res<WorldContentRuntime>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    mut capture: ResMut<ScenarioStateCapture>,
) {
    let payload = match (runtime.as_deref(), objectives.as_deref()) {
        (Some(runtime), Some(objectives)) => collect_scenario_state(runtime, &objectives.0),
        (Some(runtime), None) => collect_scenario_state(runtime, &ObjectiveManager::default()),
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
}
