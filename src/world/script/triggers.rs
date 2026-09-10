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
mod tests {
    use crate::world::config::{Trigger, TriggerCondition};
    use crate::world::script::load::compile_scripts;
    use vellum_script::ScriptSource;

    /// Compile a single inline script unit and return the triggers its top level
    /// authored, in registration order.
    fn script_triggers(source: &str) -> Vec<crate::world::script::engine::ScriptTrigger> {
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: source.to_string(),
        }]);
        assert!(
            compiled.findings.is_empty(),
            "unexpected findings: {:?}",
            compiled.findings
        );
        compiled.script_triggers
    }

    /// Assert a registration fn builds exactly the expected `Trigger`, and that
    /// it recorded the handler name.
    ///
    /// The expectation used to be computed by parsing the equivalent
    /// `[[trigger]] script = "h"` block — the "one evaluator, two front-ends"
    /// structural-equality guarantee. Issue #985 deleted that second front-end,
    /// so the expectation is written out instead, which is the stronger
    /// assertion: it pins the condition each registration builds rather than
    /// comparing two parsers against each other.
    fn assert_builds(script_call: &str, expected: TriggerCondition) {
        assert_builds_with(
            script_call,
            crate::world::config::scripted_trigger(expected),
        );
    }

    /// [`assert_builds`] for a registration that also sets a lifecycle field
    /// (`.when(…)`, `.repeat()`, …), where the whole `Trigger` is the claim.
    fn assert_builds_with(script_call: &str, expected: Trigger) {
        let regs = script_triggers(&format!("{script_call};\nfn h(ctx) {{ }}"));
        assert_eq!(regs.len(), 1, "exactly one trigger from `{script_call}`");
        assert_eq!(regs[0].handler, "h");
        assert_eq!(
            regs[0].trigger, expected,
            "script `{script_call}` must build the expected Trigger"
        );
    }

    // ── one test per TriggerCondition variant (all 11) ────────────────────────

    #[test]
    fn on_destroyed_builds_its_condition() {
        assert_builds(
            r#"on_destroyed("raider", "h")"#,
            TriggerCondition::OnDestroyed {
                entity_name: "raider".into(),
            },
        );
        // Spot-check the condition through the public accessor too.
        let regs = script_triggers(r#"on_destroyed("raider", "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnDestroyed {
                entity_name: "raider".into()
            }
        );
    }

    #[test]
    fn on_all_destroyed_default_gate_builds_its_condition() {
        assert_builds(
            r#"on_all_destroyed("wave_1", "h")"#,
            TriggerCondition::OnAllDestroyed {
                group: "wave_1".into(),
                after_secs: 0.0,
            },
        );
    }

    #[test]
    fn on_all_destroyed_with_gate_builds_its_condition() {
        assert_builds(
            r#"on_all_destroyed("wave_1", 5, "h")"#,
            TriggerCondition::OnAllDestroyed {
                group: "wave_1".into(),
                after_secs: 5.0,
            },
        );
        let regs = script_triggers(r#"on_all_destroyed("wave_1", 5, "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnAllDestroyed {
                group: "wave_1".into(),
                after_secs: 5.0
            }
        );
    }

    #[test]
    fn on_attacked_builds_its_condition() {
        assert_builds(
            r#"on_attacked("escort", "h")"#,
            TriggerCondition::OnAttacked {
                entity_name: "escort".into(),
            },
        );
    }

    #[test]
    fn on_timer_builds_its_condition() {
        assert_builds(
            r#"on_timer(45, "h")"#,
            TriggerCondition::OnTimer { after_secs: 45.0 },
        );
        let regs = script_triggers(r#"on_timer(45, "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnTimer { after_secs: 45.0 }
        );
    }

    #[test]
    fn on_hailed_builds_its_condition() {
        assert_builds(
            r#"on_hailed("relay", "h")"#,
            TriggerCondition::OnHailed {
                entity_name: "relay".into(),
            },
        );
    }

    #[test]
    fn on_flag_set_builds_its_condition() {
        assert_builds(
            r#"on_flag_set("armed", "h")"#,
            TriggerCondition::OnFlagSet {
                name: "armed".into(),
            },
        );
    }

    #[test]
    fn on_flag_cleared_builds_its_condition() {
        assert_builds(
            r#"on_flag_cleared("armed", "h")"#,
            TriggerCondition::OnFlagCleared {
                name: "armed".into(),
            },
        );
    }

    #[test]
    fn on_world_loaded_builds_its_condition() {
        assert_builds(r#"on_world_loaded("h")"#, TriggerCondition::OnWorldLoaded);
        let regs = script_triggers(r#"on_world_loaded("h"); fn h(ctx) { }"#);
        assert_eq!(regs[0].trigger.condition, TriggerCondition::OnWorldLoaded);
    }

    #[test]
    fn on_entered_region_builds_its_condition() {
        assert_builds(
            r#"on_entered_region("nebula", "h")"#,
            TriggerCondition::OnEnteredRegion {
                entity_name: "nebula".into(),
            },
        );
    }

    #[test]
    fn on_exited_region_builds_its_condition() {
        assert_builds(
            r#"on_exited_region("nebula", "h")"#,
            TriggerCondition::OnExitedRegion {
                entity_name: "nebula".into(),
            },
        );
    }

    #[test]
    fn on_waypoint_reached_any_builds_its_condition() {
        assert_builds(
            r#"on_waypoint_reached("courier", "h")"#,
            TriggerCondition::OnWaypointReached {
                entity_name: "courier".into(),
                waypoint: None,
            },
        );
        let regs = script_triggers(r#"on_waypoint_reached("courier", "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnWaypointReached {
                entity_name: "courier".into(),
                waypoint: None
            }
        );
    }

    #[test]
    fn on_waypoint_reached_specific_builds_its_condition() {
        assert_builds(
            r#"on_waypoint_reached("courier", "beacon_3", "h")"#,
            TriggerCondition::OnWaypointReached {
                entity_name: "courier".into(),
                waypoint: Some("beacon_3".into()),
            },
        );
        let regs =
            script_triggers(r#"on_waypoint_reached("courier", "beacon_3", "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnWaypointReached {
                entity_name: "courier".into(),
                waypoint: Some("beacon_3".into())
            }
        );
    }

    #[test]
    fn on_hull_below_builds_its_condition() {
        // The one condition with a FRACTIONAL field, so the one registration
        // taking a `flt(…)` marker rather than an INT (issue #984).
        assert_builds(
            r#"on_hull_below("station", flt("0.75"), "h")"#,
            TriggerCondition::OnHullBelow {
                entity_name: "station".into(),
                threshold: 0.75,
            },
        );
        let regs = script_triggers(r#"on_hull_below("station", flt("0.5"), "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnHullBelow {
                entity_name: "station".into(),
                threshold: 0.5
            }
        );
    }

    #[test]
    fn on_hull_below_rejects_a_threshold_outside_the_authored_range() {
        // Mirrors the declarative front-end's `(0, 1]` check, so the two
        // front-ends refuse the same content.
        for bad in ["0.0", "1.5", "-0.25"] {
            let compiled = compile_scripts(&[ScriptSource {
                path: "w.toml#script.setup".to_string(),
                source: format!("on_hull_below(\"s\", flt(\"{bad}\"), \"h\"); fn h(ctx) {{ }}"),
            }]);
            assert!(
                !compiled.findings.is_empty(),
                "threshold {bad} must be refused at load"
            );
        }
    }

    // ── the trigger-level `when` modifier (issue #984) ────────────────────────

    #[test]
    fn when_builds_the_same_predicate_the_declarative_field_does() {
        let mut expected =
            crate::world::config::scripted_trigger(TriggerCondition::OnAllDestroyed {
                group: "hostiles".into(),
                after_secs: 0.0,
            });
        expected.when = Some(
            crate::world::flags::parse_predicate("counter(waves_spawned) >= 8")
                .expect("the predicate parses"),
        );
        assert_builds_with(
            r#"on_all_destroyed("hostiles", "h").when("counter(waves_spawned) >= 8")"#,
            expected,
        );
    }

    #[test]
    fn when_applies_to_the_registration_it_is_chained_onto() {
        // Two registrations, one guarded: the modifier must land on ITS OWN
        // trigger, which is what the returned handle is for.
        let regs = script_triggers(
            r#"
            on_world_loaded("a");
            on_destroyed("x", "b").when("flag(armed)");
            on_timer(5, "c");
            fn a(ctx) { }
            fn b(ctx) { }
            fn c(ctx) { }
            "#,
        );
        let guarded: Vec<&str> = regs
            .iter()
            .filter(|r| r.trigger.when.is_some())
            .map(|r| r.handler.as_str())
            .collect();
        assert_eq!(guarded, vec!["b"]);
    }

    #[test]
    fn when_rejects_a_malformed_predicate_and_a_world_history_atom() {
        for bad in [
            // Not a predicate at all.
            "counter(",
            // A bounded-history window, which a WORLD expression cannot fold —
            // refused by the same `reject_world_history` the declarative `when =`
            // field runs (issue #890).
            "history(hull_below, 5) >= 1",
        ] {
            let compiled = compile_scripts(&[ScriptSource {
                path: "w.toml#script.setup".to_string(),
                source: format!("on_world_loaded(\"h\").when(\"{bad}\"); fn h(ctx) {{ }}"),
            }]);
            assert!(
                !compiled.findings.is_empty(),
                "predicate `{bad}` must be refused at load"
            );
        }
    }

    /// `.repeat()` marks exactly the registration it is chained onto, and it
    /// composes with `.when(…)` in either order — the two are orthogonal
    /// modifiers of one `Trigger`, not a sequence.
    #[test]
    fn repeat_marks_the_registration_it_is_chained_onto_and_composes_with_when() {
        let regs = script_triggers(
            r#"
            on_hailed("x", "a");
            on_hailed("x", "b").repeat();
            on_hailed("x", "c").repeat().when("flag(armed)");
            on_hailed("x", "d").when("flag(armed)").repeat();
            fn a(ctx) { }
            fn b(ctx) { }
            fn c(ctx) { }
            fn d(ctx) { }
            "#,
        );
        let repeating: Vec<&str> = regs
            .iter()
            .filter(|r| r.trigger.repeat)
            .map(|r| r.handler.as_str())
            .collect();
        assert_eq!(repeating, vec!["b", "c", "d"]);
        let guarded: Vec<&str> = regs
            .iter()
            .filter(|r| r.trigger.when.is_some())
            .map(|r| r.handler.as_str())
            .collect();
        assert_eq!(
            guarded,
            vec!["c", "d"],
            "both modifiers hand the handle straight back, so the two compose in \
             EITHER order and both land on the same registration — the guide, the \
             spec and this front-end's own summaries all promise that, and `d` is \
             the order the promise used to break in"
        );
        assert_eq!(
            regs.iter()
                .find(|r| r.handler == "c")
                .expect("the third registration")
                .trigger
                .cooldown_secs,
            None,
            "no cooldown twin is exposed on this front-end: an event-driven condition \
             fires once per occurrence and has nothing for a minimum spacing to \
             suppress"
        );
    }

    // ── the front-end also records the handler and defaults the lifecycle ─────

    #[test]
    fn a_scripted_trigger_defaults_every_lifecycle_field() {
        let regs = script_triggers(r#"on_destroyed("x", "h"); fn h(ctx) { }"#);
        let t = &regs[0].trigger;
        assert_eq!(t.when, None);
        assert_eq!(t.id, None);
        assert!(!t.repeat);
        assert_eq!(t.cooldown_secs, None);
    }

    #[test]
    fn multiple_registrations_are_collected_in_order() {
        let regs = script_triggers(
            r#"
            on_world_loaded("a");
            on_destroyed("x", "b");
            on_timer(10, "c");
            fn a(ctx) { }
            fn b(ctx) { }
            fn c(ctx) { }
            "#,
        );
        let handlers: Vec<&str> = regs.iter().map(|r| r.handler.as_str()).collect();
        assert_eq!(handlers, vec!["a", "b", "c"]);
    }

    // ── a scripted trigger's effects come from its handler (issue #980) ──────

    /// A scripted world fires its trigger through the shared evaluator and then
    /// runs the handler on the runtime host, landing on the one `ActionCmd`
    /// boundary. Neither step touches `tick_trigger_pipeline`.
    ///
    /// This was the strongest migration guard while there were TWO front-ends: it
    /// built the same trigger declaratively, dispatched its `[[trigger.action]]`
    /// array, and asserted the two `ActionCmd` sequences were identical. Issue
    /// #985 deleted the declarative half — and with it `FiredTrigger::actions`,
    /// which is why the "a fired trigger carries no action list" assertion below
    /// is now a statement about the type rather than about a scripted trigger in
    /// particular. What survives is the concrete expected sequence, which is what
    /// pinned behaviour rather than equality-to-itself.
    #[test]
    fn a_scripted_trigger_fires_and_its_handler_emits_the_action_cmds() {
        use crate::world::content::{evaluate_triggers, TriggerState, WorldEvent};
        use crate::world::dispatch::ActionCmd;
        use crate::world::flags::FlagStore;
        use crate::world::script::engine::RuntimeHost;
        use rhai::Map;
        use std::collections::{HashMap, HashSet};

        // Shared event stream: the entity "raider" is destroyed.
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".to_string(), "uuid-raider".to_string());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-raider".to_string(),
        }];

        let path = "w.toml#script.setup";
        let compiled = compile_scripts(&[ScriptSource {
            path: path.to_string(),
            source: r#"
                on_destroyed("raider", "on_raider_dead");
                fn on_raider_dead(ctx) {
                    ctx.effects.complete_objective("obj-x");
                    ctx.effects.fail_objective("obj-y");
                }
            "#
            .to_string(),
        }]);
        assert!(compiled.findings.is_empty(), "{:?}", compiled.findings);
        assert_eq!(compiled.script_triggers.len(), 1);
        let st = compiled.script_triggers[0].clone();

        let mut states = vec![TriggerState {
            trigger: st.trigger.clone(),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);

        let host = RuntimeHost::new();
        let ast = compiled.asts.get(path).expect("compiled ast");
        let cmds = host.call_immediate(ast, path, &st.handler, &FlagStore::new(), Map::new());

        assert_eq!(
            cmds,
            vec![
                ActionCmd::CompleteObjective {
                    id: "obj-x".to_string()
                },
                ActionCmd::FailObjective {
                    id: "obj-y".to_string()
                },
            ]
        );
    }

    // ── gm_event: the manual-only GM shorthand (issue #1301) ─────────────────

    #[test]
    fn gm_event_builds_a_manual_trigger_with_an_implied_fire_control() {
        let mut expected = crate::world::config::scripted_trigger(TriggerCondition::Manual);
        expected.id = Some("breach_alarm".into());
        expected.gm_controls = Some(crate::world::config::GmEventControls::fire_only(
            "breach_alarm".into(),
            "world.gm.event.breach_alarm".into(),
        ));
        assert_builds_with(
            r#"gm_event("breach_alarm", "world.gm.event.breach_alarm", "h")"#,
            expected,
        );
    }

    #[test]
    fn a_gm_event_is_one_shot_until_repeatable_says_otherwise() {
        let once = script_triggers(r#"gm_event("a", "world.gm.event.a", "h"); fn h(ctx) { }"#);
        assert!(!once[0].trigger.repeat, "gm_event is one-shot by default");

        let reusable = script_triggers(
            r#"gm_event("a", "world.gm.event.a", "h").repeatable(); fn h(ctx) { }"#,
        );
        assert!(
            reusable[0].trigger.repeat,
            "repeatable() makes it a reusable quick tool"
        );
        // `repeatable()` sets the SAME lifecycle field `.repeat()` does, and the
        // two compose with `.when(…)` in either order.
        let chained = script_triggers(
            r#"gm_event("a", "world.gm.event.a", "h").repeatable().when("flag(ready)");
               fn h(ctx) { }"#,
        );
        assert!(chained[0].trigger.repeat);
        assert!(chained[0].trigger.when.is_some());
        let reversed = script_triggers(
            r#"gm_event("a", "world.gm.event.a", "h").when("flag(ready)").repeatable();
               fn h(ctx) { }"#,
        );
        assert_eq!(chained[0].trigger, reversed[0].trigger);
    }

    /// A malformed id or label is a LOAD-TIME finding, not an event that
    /// silently never appears in the mission panel.
    #[test]
    fn a_malformed_gm_event_identity_is_a_blocking_finding() {
        for source in [
            r#"gm_event("", "world.gm.event.a", "h"); fn h(ctx) { }"#,
            r#"gm_event("a::b", "world.gm.event.a", "h"); fn h(ctx) { }"#,
            r#"gm_event("a b", "world.gm.event.a", "h"); fn h(ctx) { }"#,
            r#"gm_event("a", "", "h"); fn h(ctx) { }"#,
        ] {
            let compiled = compile_scripts(&[ScriptSource {
                path: "w.toml#script.setup".to_string(),
                source: source.to_string(),
            }]);
            assert!(
                compiled
                    .findings
                    .iter()
                    .any(|finding| finding.severity == crate::world::validate::Severity::Error),
                "`{source}` must be refused at load: {:?}",
                compiled.findings
            );
            assert!(
                compiled.script_triggers.is_empty(),
                "`{source}` must not register a half-built event"
            );
        }
    }

    // ── gm_controls: GM operability on an ORDINARY event (issue #1302) ────────

    /// The identity rule is ONE rule: the same four malformed shapes
    /// `gm_event` refuses are refused on this surface too, and the trigger the
    /// registration already pushed is left carrying NO control set — the
    /// half-built event the atomic activation gate then blocks the world over.
    #[test]
    fn a_malformed_gm_controls_identity_is_a_blocking_finding() {
        for source in [
            r#"on_world_loaded("h").gm_controls("", "world.gm.event.a"); fn h(ctx) { }"#,
            r#"on_world_loaded("h").gm_controls("a::b", "world.gm.event.a"); fn h(ctx) { }"#,
            r#"on_world_loaded("h").gm_controls("a b", "world.gm.event.a"); fn h(ctx) { }"#,
            r#"on_world_loaded("h").gm_controls("a", ""); fn h(ctx) { }"#,
        ] {
            let compiled = compile_scripts(&[ScriptSource {
                path: "w.toml#script.setup".to_string(),
                source: source.to_string(),
            }]);
            assert!(
                compiled
                    .findings
                    .iter()
                    .any(|finding| finding.severity == crate::world::validate::Severity::Error),
                "`{source}` must be refused at load: {:?}",
                compiled.findings
            );
            assert!(
                compiled
                    .script_triggers
                    .iter()
                    .all(|st| st.trigger.gm_controls.is_none()),
                "`{source}` must not register a half-built control set"
            );
        }
    }

    /// The declaration keeps the automatic condition and adds exactly the Fire
    /// lever — the same control set `gm_event` implies, under an authored id.
    #[test]
    fn gm_controls_keeps_the_condition_and_adds_an_authored_fire_control() {
        let mut expected = crate::world::config::scripted_trigger(TriggerCondition::OnHullBelow {
            entity_name: "courier".into(),
            threshold: 0.4,
        });
        expected.id = Some("breach_alarm".into());
        expected.gm_controls = Some(crate::world::config::GmEventControls::fire_only(
            "breach_alarm".into(),
            "world.gm.event.breach_alarm".into(),
        ));
        assert_builds_with(
            r#"on_hull_below("courier", flt("0.4"), "h")
                   .gm_controls("breach_alarm", "world.gm.event.breach_alarm")"#,
            expected,
        );

        let controls = script_triggers(
            r#"on_hull_below("courier", flt("0.4"), "h")
                   .gm_controls("breach_alarm", "world.gm.event.breach_alarm");
               fn h(ctx) { }"#,
        )[0]
        .trigger
        .gm_controls
        .clone()
        .expect("a control set");
        assert!(controls.fire, "Fire is what this declaration turns on");
        assert!(
            !controls.pause && !controls.skip,
            "Pause (#1303) and Skip (#1304) are not declared here"
        );
    }

    /// It is a trigger-level modifier like `.when(…)` and `.repeat()`, so the
    /// three compose in any order and say the same sentence.
    #[test]
    fn gm_controls_composes_with_the_other_trigger_modifiers_in_any_order() {
        let one = script_triggers(
            r#"on_flag_set("alarm", "h")
                   .gm_controls("evac", "world.gm.event.evac").repeat().when("flag(ready)");
               fn h(ctx) { }"#,
        );
        let other = script_triggers(
            r#"on_flag_set("alarm", "h")
                   .when("flag(ready)").repeat().gm_controls("evac", "world.gm.event.evac");
               fn h(ctx) { }"#,
        );
        assert_eq!(one[0].trigger, other[0].trigger);
        assert!(one[0].trigger.repeat);
        assert!(one[0].trigger.when.is_some());
        assert_eq!(
            one[0].trigger.condition,
            TriggerCondition::OnFlagSet {
                name: "alarm".into()
            },
            "the automatic condition is untouched by the declaration"
        );
    }

    /// Issue #1303: `.pauseable()` adds the Pause lever to a control set either
    /// surface declared, composes with every other modifier in any order, and
    /// changes nothing else about the trigger.
    #[test]
    fn pauseable_adds_the_pause_lever_to_either_authoring_surface() {
        for source in [
            r#"on_destroyed("courier", "h")
                   .gm_controls("evac", "world.gm.event.evac").pauseable();
               fn h(ctx) { }"#,
            r#"gm_event("evac", "world.gm.event.evac", "h").pauseable();
               fn h(ctx) { }"#,
        ] {
            let controls = script_triggers(source)[0]
                .trigger
                .gm_controls
                .clone()
                .expect("a control set");
            assert!(controls.pause, "`{source}` declares the Pause lever");
            assert!(controls.fire, "and leaves the Fire lever alone");
            assert!(!controls.skip, "Skip (#1304) is still not declared");
            assert_eq!(controls.id, "evac");
        }

        // Order-independent, like every other trigger-level modifier, and a
        // second `.pauseable()` is idempotent rather than a load-time error:
        // unlike a second control set it cannot re-identify the event.
        let one = script_triggers(
            r#"on_flag_set("alarm", "h")
                   .gm_controls("evac", "world.gm.event.evac").pauseable().repeat()
                   .when("flag(ready)");
               fn h(ctx) { }"#,
        );
        let other = script_triggers(
            r#"on_flag_set("alarm", "h")
                   .when("flag(ready)").repeat()
                   .gm_controls("evac", "world.gm.event.evac").pauseable().pauseable();
               fn h(ctx) { }"#,
        );
        assert_eq!(one[0].trigger, other[0].trigger);
        assert!(one[0].trigger.repeat && one[0].trigger.when.is_some());
        assert_eq!(
            one[0].trigger.condition,
            TriggerCondition::OnFlagSet {
                name: "alarm".into()
            },
            "the automatic condition is untouched by the declaration"
        );
    }

    /// `.pauseable()` on a trigger that declares no control set is a load-time
    /// error: there is no id to address it by, no label to render and no
    /// mission-panel row to hang a toggle on, so it can only be a mistake.
    #[test]
    fn pauseable_without_a_control_set_is_a_blocking_finding() {
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: r#"on_world_loaded("h").pauseable(); fn h(ctx) { }"#.to_string(),
        }]);
        assert!(
            compiled
                .findings
                .iter()
                .any(|finding| finding.severity == crate::world::validate::Severity::Error),
            "a pauseable() with no gm_controls must be refused at load: {:?}",
            compiled.findings
        );
        assert!(compiled
            .script_triggers
            .iter()
            .all(|st| st.trigger.gm_controls.is_none()));
    }

    /// One trigger, one control set. Declaring twice is a load-time error
    /// rather than a last-writer-wins overwrite that would publish a panel row
    /// under an id the author did not expect.
    #[test]
    fn a_second_gm_controls_declaration_on_one_trigger_is_a_blocking_finding() {
        for source in [
            r#"on_world_loaded("h").gm_controls("a", "world.gm.event.a")
                   .gm_controls("b", "world.gm.event.b"); fn h(ctx) { }"#,
            // `gm_event` already implied Fire under its OWN authored id.
            r#"gm_event("a", "world.gm.event.a", "h")
                   .gm_controls("b", "world.gm.event.b"); fn h(ctx) { }"#,
        ] {
            let compiled = compile_scripts(&[ScriptSource {
                path: "w.toml#script.setup".to_string(),
                source: source.to_string(),
            }]);
            assert!(
                compiled
                    .findings
                    .iter()
                    .any(|finding| finding.severity == crate::world::validate::Severity::Error),
                "`{source}` must be refused at load: {:?}",
                compiled.findings
            );
        }

        // The FIRST declaration survives untouched — the refusal is not a
        // half-applied overwrite.
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: r#"on_world_loaded("h").gm_controls("a", "world.gm.event.a")
                           .gm_controls("b", "world.gm.event.b"); fn h(ctx) { }"#
                .to_string(),
        }]);
        assert_eq!(
            compiled.script_triggers[0]
                .trigger
                .gm_controls
                .as_ref()
                .map(|c| c.id.as_str()),
            Some("a"),
        );
    }

    // -- skip(): the Skip-next lever (issue #1304) ----------------------------

    /// `.skip()` turns on exactly one field of the control set the line before
    /// it declared, and leaves the condition, the lifecycle and Fire alone.
    #[test]
    fn skip_adds_one_lever_to_the_declaration_before_it() {
        let controls = script_triggers(
            r#"on_destroyed("courier", "h")
                   .gm_controls("evac", "world.gm.event.evac").skip();
               fn h(ctx) { }"#,
        )[0]
        .trigger
        .gm_controls
        .clone()
        .expect("a control set");
        assert!(controls.skip, "Skip is what this modifier turns on");
        assert!(controls.fire, "and it leaves the declared Fire alone");
        assert!(!controls.pause, "Pause (#1303) is not declared here");

        let mut expected = crate::world::config::scripted_trigger(TriggerCondition::OnDestroyed {
            entity_name: "courier".into(),
        });
        expected.id = Some("evac".into());
        let mut with_skip = crate::world::config::GmEventControls::fire_only(
            "evac".into(),
            "world.gm.event.evac".into(),
        );
        with_skip.skip = true;
        expected.gm_controls = Some(with_skip);
        assert_builds_with(
            r#"on_destroyed("courier", "h")
                   .gm_controls("evac", "world.gm.event.evac").skip()"#,
            expected,
        );
    }

    /// It composes with the other trigger modifiers in any order, exactly as
    /// they compose with each other -- with the ONE ordering constraint the
    /// lever's meaning imposes: it must follow the declaration it belongs to.
    #[test]
    fn skip_composes_with_the_other_modifiers_and_needs_its_declaration_first() {
        let one = script_triggers(
            r#"on_flag_set("alarm", "h")
                   .gm_controls("evac", "world.gm.event.evac").skip().repeat();
               fn h(ctx) { }"#,
        );
        let other = script_triggers(
            r#"on_flag_set("alarm", "h")
                   .repeat().gm_controls("evac", "world.gm.event.evac").skip();
               fn h(ctx) { }"#,
        );
        assert_eq!(one[0].trigger, other[0].trigger);
        assert!(one[0].trigger.repeat);
        assert!(one[0]
            .trigger
            .gm_controls
            .as_ref()
            .is_some_and(|controls| controls.skip));

        // A lever with no declaration to belong to names no event at all, so it
        // is a load-time error rather than a trigger with a silent Skip.
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: r#"on_flag_set("alarm", "h").skip(); fn h(ctx) { }"#.to_string(),
        }]);
        assert!(
            compiled
                .findings
                .iter()
                .any(|finding| finding.severity == crate::world::validate::Severity::Error),
            "a Skip with no gm_controls must be refused at load: {:?}",
            compiled.findings
        );
    }

    /// A manual `gm_event` has no automatic occurrence, so a Skip on it could
    /// never be consumed: a mission-panel button an operator can press for ever
    /// with no possible effect. Refused at load rather than shipped inert.
    #[test]
    fn skip_on_a_manual_gm_event_is_a_blocking_finding() {
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: r#"gm_event("a", "world.gm.event.a", "h").skip(); fn h(ctx) { }"#.to_string(),
        }]);
        assert!(
            compiled
                .findings
                .iter()
                .any(|finding| finding.severity == crate::world::validate::Severity::Error),
            "a Skip on a manual event must be refused at load: {:?}",
            compiled.findings
        );
        assert!(
            compiled.script_triggers.iter().all(|st| st
                .trigger
                .gm_controls
                .as_ref()
                .is_none_or(|controls| !controls.skip)),
            "and must not register a half-built lever"
        );
    }
}
