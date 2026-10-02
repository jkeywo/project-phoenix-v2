//! The Rhai trigger front-end (issue #980, milestone M2).
//!
//! One registered *loading-engine* host function per
//! [`TriggerCondition`](crate::world::config::TriggerCondition) variant. A unit's
//! top level calls them to author triggers in script:
//!
//! ```rhai
//! on_destroyed("raider", "on_raider_dead");
//! on_all_destroyed("wave_1", 5, "wave_one_cleared");
//! on_world_loaded("arm_the_rivalry");
//! ```
//!
//! Each call builds the *same* [`Trigger`](crate::world::config::Trigger) struct
//! the TOML `[[trigger]]` front-end builds — through the one shared
//! [`scripted_trigger`](crate::world::config::scripted_trigger) constructor — and
//! records the named handler fn that supplies its effects. The built triggers
//! feed the existing evaluator ([`crate::world::content`]) and pipeline
//! (`tick_trigger_pipeline`) exactly as TOML-authored ones do: **one evaluator,
//! two front-ends** (settled decision 5). Nothing here executes a handler — that
//! is the runtime host's job ([`super::engine::RuntimeHost`]); the loading engine
//! only *collects* what a top level authored (`Engine::run_ast` never runs fn
//! bodies), so a handler's effect calls compile here and resolve at runtime.
//!
//! # Integer-only surface (`no_float`)
//!
//! The whole script API is integer-only. The two conditions with a float field —
//! `OnTimer { after_secs }` and `OnAllDestroyed { after_secs }` — take an `INT`
//! (seconds) at the host-fn boundary and convert to `f32` there, exactly as the
//! roadmap's "floats convert at the host-fn boundary" rule requires. Authored
//! `after_secs` in TOML are whole seconds in every shipped world, so a scripted
//! trigger and its TOML equivalent build the identical `f32`.

use std::sync::{Arc, Mutex};

use rhai::{EvalAltResult, ImmutableString, Position};

use crate::world::config::{
    reject_world_history, scripted_trigger, GmEventControls, TriggerCondition,
};
use crate::world::script::effects::RealLit;
use crate::world::script::engine::{BuilderState, ScriptTrigger};
use crate::world::script::registry::{host_fn, HostRegistry};

/// A handle to the trigger a registration fn just authored, returned so
/// TRIGGER-LEVEL fields — the ones that are neither the condition nor the
/// handler — can be chained onto it:
///
/// ```rhai
/// on_all_destroyed("hostiles", "on_victory").when("counter(waves_spawned) >= 8");
/// ```
///
/// It is an index into the running unit's `script_triggers`, not a borrow, so
/// the builder state lock is taken once per call and nothing outlives it. A
/// registration used as a statement simply discards it, which is why every
/// existing `on_*(…);` line is unaffected.
#[derive(Clone, Copy, Debug)]
pub struct TriggerHandle {
    index: usize,
}

/// Record one script-authored trigger against the unit currently running, and
/// hand back a [`TriggerHandle`] to it.
fn push_trigger(
    state: &Mutex<BuilderState>,
    condition: TriggerCondition,
    handler: &str,
) -> TriggerHandle {
    let mut s = state.lock().expect("builder state lock");
    let source_path = s.current_path.clone();
    s.script_triggers.push(ScriptTrigger {
        trigger: scripted_trigger(condition),
        handler: handler.to_string(),
        source_path,
    });
    TriggerHandle {
        index: s.script_triggers.len() - 1,
    }
}

/// Build a Rhai load-time error from a builder message.
fn raise(message: String) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(message.into(), Position::NONE))
}

/// Register the typed trigger-builder host functions on a loading engine.
///
/// Called once from [`super::engine::loading_engine`]. Every closure captures a
/// clone of the shared [`BuilderState`] handle, so a top-level call attributes
/// its trigger to whichever unit the loader is currently running.
pub(crate) fn register_trigger_builders(
    engine: &mut HostRegistry,
    state: Arc<Mutex<BuilderState>>,
) {
    // 1. OnDestroyed { entity_name }
    let s = state.clone();
    host_fn!(
        engine,
        "on_destroyed",
        receiver = "",
        category = "trigger",
        params = ["entity", "handler"],
        summary = "Fire when the named entity is destroyed.",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnDestroyed {
                    entity_name: entity.to_string(),
                },
                &handler,
            )
        },
    );

    // 2. OnAllDestroyed { group, after_secs } — no-gate (after_secs = 0.0) …
    let s = state.clone();
    host_fn!(
        engine,
        "on_all_destroyed",
        receiver = "",
        category = "trigger",
        params = ["group", "handler"],
        summary = "Fire when every entity in a group is destroyed. Optional \
                  middle arg `after_secs` gates the fire.",
        move |group: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnAllDestroyed {
                    group: group.to_string(),
                    after_secs: 0.0,
                },
                &handler,
            )
        },
    );
    // … and the gated form (integer seconds → f32 at the boundary).
    let s = state.clone();
    engine.register_fn(
        "on_all_destroyed",
        move |group: ImmutableString, after_secs: i64, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnAllDestroyed {
                    group: group.to_string(),
                    after_secs: after_secs as f32,
                },
                &handler,
            )
        },
    );

    // 3. OnAttacked { entity_name }
    let s = state.clone();
    host_fn!(
        engine,
        "on_attacked",
        receiver = "",
        category = "trigger",
        params = ["entity", "handler"],
        summary = "Fire when the named entity is attacked.",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnAttacked {
                    entity_name: entity.to_string(),
                },
                &handler,
            )
        },
    );

    // 4. OnTimer { after_secs } — integer seconds → f32 at the boundary.
    let s = state.clone();
    host_fn!(
        engine,
        "on_timer",
        receiver = "",
        category = "trigger",
        params = ["after_secs", "handler"],
        summary = "Fire once, `after_secs` seconds after the world loads.",
        move |after_secs: i64, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnTimer {
                    after_secs: after_secs as f32,
                },
                &handler,
            )
        },
    );

    // 5. OnHailed { entity_name }
    let s = state.clone();
    host_fn!(
        engine,
        "on_hailed",
        receiver = "",
        category = "trigger",
        params = ["entity", "handler"],
        summary = "Fire when the named entity is hailed over comms.",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnHailed {
                    entity_name: entity.to_string(),
                },
                &handler,
            )
        },
    );

    // 6. OnFlagSet { name }
    let s = state.clone();
    host_fn!(
        engine,
        "on_flag_set",
        receiver = "",
        category = "trigger",
        params = ["name", "handler"],
        summary = "Fire when the named flag transitions to set.",
        move |name: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnFlagSet {
                    name: name.to_string(),
                },
                &handler,
            )
        },
    );

    // 7. OnFlagCleared { name }
    let s = state.clone();
    host_fn!(
        engine,
        "on_flag_cleared",
        receiver = "",
        category = "trigger",
        params = ["name", "handler"],
        summary = "Fire when the named flag transitions to cleared.",
        move |name: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnFlagCleared {
                    name: name.to_string(),
                },
                &handler,
            )
        },
    );

    // 8. OnWorldLoaded (no condition params)
    let s = state.clone();
    host_fn!(
        engine,
        "on_world_loaded",
        receiver = "",
        category = "trigger",
        params = ["handler"],
        summary = "Fire once when this world finishes loading.",
        move |handler: ImmutableString| {
            push_trigger(&s, TriggerCondition::OnWorldLoaded, &handler)
        },
    );

    // 9. OnEnteredRegion { entity_name }
    let s = state.clone();
    host_fn!(
        engine,
        "on_entered_region",
        receiver = "",
        category = "trigger",
        params = ["entity", "handler"],
        summary = "Fire when the named entity enters a region.",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnEnteredRegion {
                    entity_name: entity.to_string(),
                },
                &handler,
            )
        },
    );

    // 10. OnExitedRegion { entity_name }
    let s = state.clone();
    host_fn!(
        engine,
        "on_exited_region",
        receiver = "",
        category = "trigger",
        params = ["entity", "handler"],
        summary = "Fire when the named entity exits a region.",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnExitedRegion {
                    entity_name: entity.to_string(),
                },
                &handler,
            )
        },
    );

    // 11. OnWaypointReached { entity_name, waypoint } — any-waypoint form …
    let s = state.clone();
    host_fn!(
        engine,
        "on_waypoint_reached",
        receiver = "",
        category = "trigger",
        params = ["entity", "handler"],
        summary = "Fire when the named entity reaches a waypoint. Optional middle \
                  arg `waypoint` pins a specific anchor.",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnWaypointReached {
                    entity_name: entity.to_string(),
                    waypoint: None,
                },
                &handler,
            )
        },
    );
    // … and the specific-anchor form.
    let s = state.clone();
    engine.register_fn(
        "on_waypoint_reached",
        move |entity: ImmutableString, waypoint: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnWaypointReached {
                    entity_name: entity.to_string(),
                    waypoint: Some(waypoint.to_string()),
                },
                &handler,
            )
        },
    );

    // 12. OnHullBelow { entity_name, threshold } — the one condition whose own
    // field is a FRACTION, so it is the one registration that takes a `flt(…)`
    // marker rather than an INT (issue #984, the combat_test conversion). The
    // threshold is a hull fraction in (0, 1]: a whole number could only ever be
    // 1, so this is the boundary where the integer-only rule genuinely runs out
    // and the same `RealLit` the effect maps use carries the value instead.
    // Range validation mirrors the declarative front-end's, and a rejection is a
    // load-time finding exactly as a bad `threshold = …` fails the world parse.
    let s = state.clone();
    host_fn!(
        engine,
        "on_hull_below",
        receiver = "",
        category = "trigger",
        params = ["entity", "threshold", "handler"],
        summary = "Fire when the named entity's hull fraction crosses DOWN through \
                  `threshold`, a fraction in (0, 1] written `flt(\"0.75\")`.",
        move |entity: ImmutableString,
              threshold: RealLit,
              handler: ImmutableString|
              -> Result<TriggerHandle, Box<EvalAltResult>> {
            let threshold = threshold.0 as f32;
            if !(threshold > 0.0 && threshold <= 1.0) {
                return Err(raise(format!(
                    "on_hull_below threshold must be in (0, 1], got {threshold}"
                )));
            }
            Ok(push_trigger(
                &s,
                TriggerCondition::OnHullBelow {
                    entity_name: entity.to_string(),
                    threshold,
                },
                &handler,
            ))
        },
    );

    // 13. The manual-only GM authoring shorthand (issue #1301, PRD #930 M2).
    //
    // `gm_event("breach_alarm", "world.gm.event.breach_alarm", "on_breach")`
    // authors an event with NO automatic condition at all: the only thing that
    // can cause it is a GM pressing Fire in the mission panel. That is why it
    // does not take a condition and why it needs no separate `gm_controls`
    // declaration — a manual event that could not be fired would be an event
    // nothing could ever cause, so Fire is implied rather than authored.
    //
    // It is one-shot by default, exactly like every other registration here, and
    // `.repeatable()` below makes it a reusable quick tool. `.when(…)` composes
    // too, with its ordinary meaning: a false reading suppresses the firing
    // without consuming the Fire, so the event lands when the predicate holds.
    //
    // The `label` is a String Table id, never English, because the mission panel
    // renders it to an operator. The id is the author's stable handle; it is
    // qualified by the layer that authored it at read time
    // (`crate::gm_event::qualified_event_id`), so two layers may each author
    // `breach_alarm` without colliding. Both are validated HERE, at the
    // authoring boundary, so a malformed one is a load-time finding that blocks
    // activation rather than an event that silently never appears.
    let s = state.clone();
    host_fn!(
        engine,
        "gm_event",
        receiver = "",
        category = "trigger",
        params = ["id", "label", "handler"],
        summary = "Author a GM-only manual event: no automatic condition, an \
                  implied Fire control in the GM mission panel, and a String \
                  Table `label`. One-shot unless `.repeatable()`.",
        move |id: ImmutableString,
              label: ImmutableString,
              handler: ImmutableString|
              -> Result<TriggerHandle, Box<EvalAltResult>> {
            GmEventControls::validate_authored(&id, &label).map_err(raise)?;
            let handle = push_trigger(&s, TriggerCondition::Manual, &handler);
            let mut st = s.lock().expect("builder state lock");
            let trigger = &mut st.script_triggers[handle.index].trigger;
            // The authored id doubles as the ordinary `Trigger::id`, so a
            // scenario can `reset_trigger` a spent GM event and the balance
            // feed attributes its fire by the same name the GM pressed.
            trigger.id = Some(id.to_string());
            trigger.gm_controls = Some(GmEventControls::fire_only(
                id.to_string(),
                label.to_string(),
            ));
            Ok(handle)
        },
    );

    // The GM-event lifecycle modifier (issue #1301).
    //
    // Spelled `repeatable()` rather than reusing `repeat()` because it says the
    // thing the GM contract says — "a reusable quick tool" — at the one call
    // site an operator's mental model is the subject. It sets the SAME
    // `Trigger::repeat` field, so there is one lifecycle policy and not two, and
    // it is registered on `TriggerHandle` like every other modifier so the two
    // spellings compose with `.when(…)` in either order.
    let s = state.clone();
    host_fn!(
        engine,
        "repeatable",
        receiver = "trigger",
        category = "trigger",
        params = [],
        summary = "Make a `gm_event` a reusable GM quick tool instead of a \
                  one-shot: `gm_event(id, label, h).repeatable()`. The same \
                  lifecycle field `.repeat()` sets.",
        move |handle: &mut TriggerHandle| -> Result<TriggerHandle, Box<EvalAltResult>> {
            let mut st = s.lock().expect("builder state lock");
            let index = handle.index;
            match st.script_triggers.get_mut(index) {
                Some(t) => {
                    t.trigger.repeat = true;
                    Ok(*handle)
                }
                // Unreachable through the front-end, exactly as in `when` below.
                None => Err(raise(format!(
                    "repeatable(): trigger handle {index} names no registered trigger"
                ))),
            }
        },
    );

    // 14. The GM operability declaration on an ORDINARY event (issue #1302).
    //
    // `on_destroyed("courier", "on_lost").gm_controls("courier_lost",
    // "world.gm.event.courier_lost")` keeps its automatic condition and ALSO
    // becomes something a Game Master can reach for in the mission panel.
    //
    // It is a trigger-level modifier, not a parameter of thirteen registration
    // fns, for exactly the reason `.when(…)` is: GM operability is orthogonal
    // to every condition, and thirteen `_gm` overloads would say one thing
    // thirteen times. Nothing downstream of here knows which surface authored
    // the control set — `state_event_id`, `controllable_events`,
    // `fireable_index`, `manual_fire_is_still_live` and every eligibility gate
    // in `fire_manual_trigger` read `Trigger::gm_controls` and the lifecycle
    // fields, never the condition — so a Fire on an automatic event bypasses
    // that condition and runs the ordinary handler through the ordinary
    // lifecycle by construction rather than by a second code path. The one
    // deliberate read is `fire_manual_trigger` naming the condition's SUBJECT
    // on the `FiredTrigger` it returns, so the record of a GM fire is
    // indistinguishable from the record of the occurrence it stood in for; see
    // the module docs on `crate::gm_event`.
    //
    // Fire is the ONLY lever it declares. #1303's Pause and #1304's Skip arrive
    // as sibling modifiers on this same handle rather than as extra parameters
    // here, matching how `.repeat()` and `.when()` are siblings: an author who
    // wants them says so, and a signature change two issues from now is not a
    // breaking edit to every world that already declares a control set.
    //
    // Declaring twice is a load-time error rather than a last-writer-wins
    // overwrite. `gm_event(…).gm_controls(…)` is the interesting case: the
    // shorthand already implied Fire under a DIFFERENT authored id, so silently
    // keeping one of the two would publish a mission-panel row under an id the
    // author did not expect and leave the other unaddressable.
    let s = state.clone();
    host_fn!(
        engine,
        "gm_controls",
        receiver = "trigger",
        category = "trigger",
        params = ["id", "label"],
        summary = "Make the ORDINARY registration just authored GM-operable: \
                  `on_destroyed(e, \"h\").gm_controls(\"courier_lost\", \
                  \"world.gm.event.courier_lost\")` adds a Fire control to the GM \
                  mission panel under a stable id and a String Table label, \
                  leaving the automatic condition untouched. Chains with \
                  `.when(…)` and `.repeat()`, in any order.",
        move |handle: &mut TriggerHandle,
              id: ImmutableString,
              label: ImmutableString|
              -> Result<TriggerHandle, Box<EvalAltResult>> {
            GmEventControls::validate_authored(&id, &label).map_err(raise)?;
            let mut st = s.lock().expect("builder state lock");
            let index = handle.index;
            match st.script_triggers.get_mut(index) {
                Some(t) if t.trigger.gm_controls.is_some() => Err(raise(format!(
                    "gm_controls(\"{id}\", …): this registration already declares \
                     GM controls (id '{}'); a trigger has one control set, and \
                     `gm_event` already implies Fire",
                    t.trigger
                        .gm_controls
                        .as_ref()
                        .map(|c| c.id.as_str())
                        .unwrap_or_default(),
                ))),
                Some(t) => {
                    // The authored id doubles as the ordinary `Trigger::id`,
                    // exactly as `gm_event` sets it: a scenario can
                    // `reset_trigger` a spent GM-controlled event, and the
                    // balance feed attributes its fire — automatic or GM — by
                    // the one name the operator pressed.
                    t.trigger.id = Some(id.to_string());
                    t.trigger.gm_controls = Some(GmEventControls::fire_only(
                        id.to_string(),
                        label.to_string(),
                    ));
                    Ok(*handle)
                }
                // Unreachable through the front-end, exactly as in `when` below.
                None => Err(raise(format!(
                    "gm_controls(): trigger handle {index} names no registered trigger"
                ))),
            }
        },
    );

    // 15. The Pause lever on an authored GM-operable event (issue #1303).
    //
    // `on_destroyed("courier", "evac").gm_controls("evac", "world.gm.event.evac").pauseable()`
    // — a SIBLING modifier on the same handle, exactly as
    // `gm-t2-event-control-contract` said Pause would arrive, rather than a
    // third parameter of `gm_controls`. A world that already declares a control
    // set is untouched by this issue landing, and an author who wants the lever
    // says so in one word.
    //
    // It requires a control set to already exist, and that guard is the
    // load-time half of "only an event declaring Pause exposes the toggle": a
    // `.pauseable()` on a trigger with no `gm_event`/`gm_controls` declaration
    // has no id to address, no label to render and no mission-panel row to hang
    // a toggle on, so it can only be an authoring mistake. The guard is the
    // exact inverse of `gm_controls`'s duplicate check above.
    //
    // Declaring it TWICE is deliberately fine, unlike a second `gm_controls`.
    // The difference is what the second declaration could change: a second
    // control set silently re-identifies the event under an id the author did
    // not choose, while a second `.pauseable()` sets the same bool to the same
    // value. `repeatable()` is idempotent for the same reason and this matches
    // it.
    let s = state.clone();
    host_fn!(
        engine,
        "pauseable",
        receiver = "trigger",
        category = "trigger",
        params = [],
        summary = "Add the persistent GM Pause lever to the control set this \
                  registration already declares: \
                  `on_destroyed(e, \"h\").gm_controls(\"id\", \"l\").pauseable()`. \
                  While a GM has the event paused its automatic condition is \
                  not evaluated and captures no missed edge; Fire, if \
                  declared, still works. Chains with the other modifiers in \
                  any order.",
        move |handle: &mut TriggerHandle| -> Result<TriggerHandle, Box<EvalAltResult>> {
            let mut st = s.lock().expect("builder state lock");
            let index = handle.index;
            match st.script_triggers.get_mut(index) {
                Some(t) => match t.trigger.gm_controls.as_mut() {
                    Some(controls) => {
                        controls.pause = true;
                        Ok(*handle)
                    }
                    None => Err(raise(
                        "pauseable(): this registration declares no GM \
                         controls; call gm_event(id, label, handler) or \
                         .gm_controls(id, label) first"
                            .to_string(),
                    )),
                },
                // Unreachable through the front-end, exactly as in `when` below.
                None => Err(raise(format!(
                    "pauseable(): trigger handle {index} names no registered trigger"
                ))),
            }
        },
    );

    // 16. The Skip-next lever on a GM-operable ORDINARY event (issue #1304).
    //
    // `on_destroyed("courier", "on_lost").gm_controls("courier_lost", "…").skip()`
    // adds the third lever to the control set the line before it declared. It
    // is a sibling modifier rather than a `gm_controls` parameter for exactly
    // the reason `.when(…)` and `.repeat()` are siblings of their conditions:
    // an author who wants a lever says so, and a signature change would be a
    // breaking edit to every world that already declares a control set.
    //
    // What it authorises: a GM may ARM the next matching occurrence so it
    // advances the ordinary lifecycle — a once-only event is spent, a repeating
    // one enters its ordinary cooldown — WITHOUT running the handler. The crew
    // see nothing, which is the point; the record of what a GM did lives on the
    // GM's own feed.
    //
    // Two load-time errors rather than a silently inert declaration:
    //
    // * no control set to attach it to. `.skip()` names a lever OF a
    //   declaration, so `on_destroyed(e, "h").skip()` is not "a trigger with a
    //   Skip" but a trigger with no GM identity at all — nothing a mission
    //   panel could list and nothing a GM action could name. (This is the one
    //   ordering constraint among the trigger modifiers; `.when(…)` and
    //   `.repeat()` set trigger-level fields that exist unconditionally.)
    // * a `TriggerCondition::Manual` condition — the `gm_event(…)` shorthand.
    //   A manual event has no automatic occurrence, so its Skip could never be
    //   consumed. Shared with the load-time validation pass through
    //   `GmEventControls::validate_skip_condition` so the two cannot drift.
    let s = state.clone();
    host_fn!(
        engine,
        "skip",
        receiver = "trigger",
        category = "trigger",
        params = [],
        summary = "Add the Skip-next lever to the GM control set just declared: \
                  `on_destroyed(e, \"h\").gm_controls(id, label).skip()` lets a \
                  Game Master arm the NEXT matching occurrence to advance the \
                  ordinary lifecycle without running the handler. Needs a \
                  preceding `gm_controls(...)` and an automatic condition.",
        move |handle: &mut TriggerHandle| -> Result<TriggerHandle, Box<EvalAltResult>> {
            let mut st = s.lock().expect("builder state lock");
            let index = handle.index;
            match st.script_triggers.get_mut(index) {
                Some(t) if t.trigger.gm_controls.is_none() => Err(raise(
                    "skip(): this registration declares no GM controls; write \
                     `.gm_controls(id, label).skip()` so the lever has an event \
                     identity to belong to"
                        .to_string(),
                )),
                Some(t) => {
                    GmEventControls::validate_skip_condition(&t.trigger.condition)
                        .map_err(|message| raise(format!("skip(): {message}")))?;
                    t.trigger.gm_controls.as_mut().expect("checked above").skip = true;
                    Ok(*handle)
                }
                // Unreachable through the front-end, exactly as in `when` below.
                None => Err(raise(format!(
                    "skip(): trigger handle {index} names no registered trigger"
                ))),
            }
        },
    );

    // 17. The GM-attention band of a declared beat (issue #1434).
    //
    // `on_timer(45, "h").gm_controls("wave_2", "…").attention_band("background")`
    // — a sibling modifier for `.pauseable()`'s reason, and it says one thing:
    // where this beat sits in the Game Master's own attention queue while it is
    // eligible. Urgent, Attention (the default) or Background, and nothing
    // else; an invented word is a load-time error naming the three, not a beat
    // that silently lands in a band the author did not choose.
    //
    // What it is NOT: it does not change when the beat fires, what its handler
    // does, whether the crew see anything, or what any other operator's desk
    // shows. It is triage metadata on one private, advisory, peer-local list —
    // which is exactly why an author may set it and why setting it can never be
    // a route into the simulation.
    //
    // It requires a control set to already exist, the same guard `.pauseable()`
    // has and for the same reason: a band with no event identity has no row to
    // belong to. Declaring it twice is deliberately fine — the second word can
    // only change this beat's own band, unlike a second `gm_controls`, which
    // would re-identify the event.
    let s = state.clone();
    host_fn!(
        engine,
        "attention_band",
        receiver = "trigger",
        category = "trigger",
        params = ["band"],
        summary = "Set where this beat sits in a Game Master's attention queue \
                  while it is eligible: `.gm_controls(id, label)\
                  .attention_band(\"background\")`. One of \"urgent\", \
                  \"attention\" (the default) or \"background\". Advisory \
                  triage only: it changes nothing about when the beat fires or \
                  what the crew see. Needs a preceding gm_event(...) or \
                  .gm_controls(...).",
        move |handle: &mut TriggerHandle,
              band: ImmutableString|
              -> Result<TriggerHandle, Box<EvalAltResult>> {
            GmEventControls::validate_attention_band(&band)
                .map_err(|message| raise(format!("attention_band(): {message}")))?;
            let mut st = s.lock().expect("builder state lock");
            let index = handle.index;
            match st.script_triggers.get_mut(index) {
                Some(t) => match t.trigger.gm_controls.as_mut() {
                    Some(controls) => {
                        controls.attention_band = Some(band.to_string());
                        Ok(*handle)
                    }
                    None => Err(raise(
                        "attention_band(): this registration declares no GM \
                         controls; call gm_event(id, label, handler) or \
                         .gm_controls(id, label) first"
                            .to_string(),
                    )),
                },
                // Unreachable through the front-end, exactly as in `when` below.
                None => Err(raise(format!(
                    "attention_band(): trigger handle {index} names no registered trigger"
                ))),
            }
        },
    );

    // The trigger-LEVEL predicate gate, chained onto whichever registration just
    // ran: `on_all_destroyed("hostiles", "h").when("counter(waves) >= 8")`.
    //
    // It is a modifier rather than an argument because it is orthogonal to every
    // condition — the declarative front-end spells it as a sibling field of
    // `condition`, not part of it — and eleven `_when` overloads would say the
    // same thing eleven times.
    //
    // The semantics that make it worth having, and not expressible as an `if` at
    // the top of the handler: a `when` that reads false suppresses the firing
    // WITHOUT consuming the trigger (`evaluate_triggers_with_flags` `continue`s
    // before `state.fired = true`), so the trigger stays armed for a later
    // moment when the predicate holds. An in-handler guard cannot do that — the
    // condition already fired, and the trigger is spent.
    //
    // Parsed through the SAME `parse_predicate` the declarative `when =` field
    // uses, and refused the same bounded-history atoms, so the two front-ends
    // build the identical `Predicate`.
    //
    // Hands the handle BACK rather than unit, for the reason `repeat` below
    // does: the two trigger-level modifiers are orthogonal fields of one
    // `Trigger`, so an author writing them in the other order
    // (`on_hailed(e, h).when("flag(x)").repeat()`) is saying the same sentence
    // and must not meet a function-not-found error on a Rhai unit value.
    let s = state.clone();
    host_fn!(
        engine,
        "when",
        receiver = "trigger",
        category = "trigger",
        params = ["predicate"],
        summary = "Gate the registration just authored on a flag predicate: \
                  `on_all_destroyed(g, h).when(\"counter(x) >= 8\")`. A false \
                  reading suppresses the firing WITHOUT consuming the trigger. \
                  Chains with `.repeat()`, in either order.",
        move |handle: &mut TriggerHandle,
              predicate: ImmutableString|
              -> Result<TriggerHandle, Box<EvalAltResult>> {
            let pred = crate::world::flags::parse_predicate(&predicate)
                .map_err(|e| raise(format!("Trigger 'when' predicate parse error: {e}")))?;
            reject_world_history(&pred, "Trigger 'when' predicate").map_err(raise)?;
            let mut st = s.lock().expect("builder state lock");
            let index = handle.index;
            match st.script_triggers.get_mut(index) {
                Some(t) => {
                    t.trigger.when = Some(pred);
                    Ok(*handle)
                }
                // Unreachable through the front-end: a handle is only ever minted
                // by `push_trigger`, and nothing removes from `script_triggers`.
                None => Err(raise(format!(
                    "when(): trigger handle {index} names no registered trigger"
                ))),
            }
        },
    );

    // The trigger-LEVEL lifecycle policy, the second sibling field of
    // `condition` the declarative front-end already spells (`repeat = true`,
    // issue #751) and the script front-end could not.
    //
    // Why a scenario needs it, and why a `when` cannot stand in: `when` keeps a
    // registration ARMED across a false reading, but a registration that has
    // actually fired is spent forever. So an author who wants a recurring event
    // — a hail on a channel a crew may open, clear and open again — was reduced
    // to registering the same handler once per reachable state and hoping the
    // partition covered every one of them. It cannot: any state a pick leaves
    // unchanged is a state whose one registration is already consumed, and the
    // channel goes dead with the situation it answers still open (#1349).
    //
    // No cooldown twin is offered. The conditions worth repeating here are
    // EVENT-driven (a hail, an attack, a flag transition), so one condition
    // occurrence is one firing and there is nothing for a minimum spacing to
    // suppress; `cooldown_secs` stays a declarative-only field until an author
    // has a level-triggered repeat that needs it.
    //
    // Returns the handle so the two modifiers compose in either order —
    // `on_hailed(e, h).repeat().when("counter(x) < 1")` reads as the sentence it
    // is.
    let s = state.clone();
    host_fn!(
        engine,
        "repeat",
        receiver = "trigger",
        category = "trigger",
        params = [],
        summary = "Make the registration just authored repeatable: \
                  `on_hailed(e, h).repeat()` fires every time its condition \
                  occurs instead of once. Chains with `.when(…)`.",
        move |handle: &mut TriggerHandle| -> Result<TriggerHandle, Box<EvalAltResult>> {
            let mut st = s.lock().expect("builder state lock");
            let index = handle.index;
            match st.script_triggers.get_mut(index) {
                Some(t) => {
                    t.trigger.repeat = true;
                    Ok(*handle)
                }
                // Unreachable through the front-end, exactly as in `when` above.
                None => Err(raise(format!(
                    "repeat(): trigger handle {index} names no registered trigger"
                ))),
            }
        },
    );

    engine.register_type_with_name::<TriggerHandle>("Trigger");
}

#[cfg(test)]
#[path = "triggers_tests.rs"]
mod tests;
