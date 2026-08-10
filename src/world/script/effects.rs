//! The per-call effect buffer (issue #979, Rhai milestone M1; issue #984, M6).
//!
//! Registered runtime host functions push onto a call-scoped [`EffectSink`],
//! which drains into the **existing** dispatch boundary in `world::dispatch`.
//! Script gets no new effect vocabulary: every host function here maps to
//! something the declarative trigger front-end already produces, so the applier
//! (`world::server`) grows no new arm.
//!
//! The buffer holds [`BufferedEffect`]s, of which there are two shapes sharing
//! ONE ordered `Vec` so a flag write, an immediate command effect, and a
//! name-resolving effect all apply in the order the script authored them:
//!
//! * [`BufferedEffect::Cmd`] — a fully-resolved [`ActionCmd`] needing no dispatch
//!   context (the M1 set: `complete_objective`, `game_over`, … — plus the flag
//!   writes [`flags`] pushes). The applier applies it directly.
//! * [`BufferedEffect::Action`] — a declarative [`TriggerAction`] still holding
//!   entity NAMES, buffered for the applier to resolve through the SAME
//!   `dispatch_action` the declarative evaluator runs (issue #984, M6). The three
//!   name-resolving verbs (`add_objective`, `spawn_entity`, `add_faction_enemy`)
//!   need context absent at this host-fn boundary — no `WorldIdMint`,
//!   `FactionRegistry`, `TemplateLoader`, or anchors — so they buffer the
//!   unresolved action rather than a resolved command. Because a converted world
//!   literally re-runs declarative dispatch, its `ActionCmd` sequence AND its
//!   `SpawnEntity` UUID mint order are identical to its TOML twin's — byte-identity
//!   is STRUCTURAL, not a re-implementation kept in sync (this is what a converted
//!   world's authoritative digest, #894, rides on).
//!
//! The sink is an `Arc<Mutex<Vec<BufferedEffect>>>` so it can be registered on the
//! shared runtime engine once and still be a fresh, per-call buffer: the host
//! builds one sink per call, hands a clone into the context map, and — because
//! the handle is reference-counted with interior mutability — the retained
//! clone observes everything the script pushed. See [`engine::RuntimeHost`].
//!
//! The vocabulary is deliberately integer-and-string-only (the API is
//! integer-only, `no_float`): `position` / `base_priority` / an `overrides`
//! numeric leaf are authored as INTs and converted to the declarative `f32` /
//! toml-float at this boundary, the same rule `after_secs`/`in_seconds` use.
//!
//! [`ActionCmd`]: crate::world::dispatch::ActionCmd
//! [`TriggerAction`]: crate::world::config::TriggerAction
//! [`engine::RuntimeHost`]: crate::world::script::engine::RuntimeHost
//! [`flags`]: crate::world::script::flags

use std::sync::{Arc, Mutex};

use rhai::{Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Position};

use crate::world::config::{parse_action_entry, RawActionEntry, TriggerAction};
use crate::world::dispatch::ActionCmd;

/// One buffered runtime effect, in authored order in the shared [`EffectSink`].
///
/// See the module docs for why two shapes share one buffer: a `Cmd` is a
/// resolved [`ActionCmd`] the applier applies directly; an `Action` is an
/// unresolved [`TriggerAction`] the applier resolves through the same
/// `dispatch_action` the declarative front-end uses (so a scripted spawn mints
/// its `EntityUuid` in the same order as its TOML twin — issue #984, M6).
#[derive(Clone, Debug, PartialEq)]
pub enum BufferedEffect {
    /// A resolved command effect (the M1 set + flag writes).
    Cmd(ActionCmd),
    /// A declarative action still holding entity names, resolved at apply time.
    Action(TriggerAction),
}

/// A call-scoped buffer of the [`BufferedEffect`]s a script produced.
///
/// Cloneable and interior-mutable: every clone shares one underlying `Vec`, so
/// the runtime host can register the effect host-fns on the engine once and
/// still collect exactly one call's effects.
#[derive(Clone, Default)]
pub struct EffectSink(Arc<Mutex<Vec<BufferedEffect>>>);

impl EffectSink {
    /// A fresh, empty buffer for one call.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one resolved command onto the buffer (as [`BufferedEffect::Cmd`]).
    ///
    /// `pub(crate)` because [`Flags`](super::flags::Flags) shares this one buffer
    /// so a flag mutation lands in the emitted sequence *at the point the script
    /// authored it*, interleaved with effects, rather than being appended after
    /// them (issue #981 flag-ordering hazard). The M1 command effects push here too.
    pub(crate) fn push(&self, cmd: ActionCmd) {
        self.0
            .lock()
            .expect("effect sink lock")
            .push(BufferedEffect::Cmd(cmd));
    }

    /// Push one unresolved declarative action (as [`BufferedEffect::Action`]),
    /// onto the SAME ordered buffer as [`push`](Self::push) so a name-resolving
    /// effect keeps its authored position relative to flag writes and command
    /// effects. The applier resolves it through `dispatch_action` (issue #984, M6).
    pub(crate) fn push_action(&self, action: TriggerAction) {
        self.0
            .lock()
            .expect("effect sink lock")
            .push(BufferedEffect::Action(action));
    }

    /// Drain the buffer, leaving it empty. Called by the host on the success
    /// path only — on the failure path the buffer is dropped whole, which is
    /// how "discard the call's effects" (settled decision 10) is enforced.
    pub fn take(&self) -> Vec<BufferedEffect> {
        std::mem::take(&mut self.0.lock().expect("effect sink lock"))
    }

    /// Number of buffered effects (test/introspection helper).
    pub fn len(&self) -> usize {
        self.0.lock().expect("effect sink lock").len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Register the effect vocabulary on a runtime engine.
///
/// Each function is a method on the `Effects` custom type, so a script calls
/// them as `ctx.effects.complete_objective("obj1")`. The M1 set pushes a resolved
/// `ActionCmd`; the three M6 name-resolving verbs buffer a declarative
/// `TriggerAction` for the applier to resolve.
pub fn register_effects(engine: &mut Engine) {
    engine.register_type_with_name::<EffectSink>("Effects");

    engine.register_fn(
        "complete_objective",
        |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::CompleteObjective { id: id.to_string() });
        },
    );
    engine.register_fn(
        "fail_objective",
        |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::FailObjective { id: id.to_string() });
        },
    );
    engine.register_fn(
        "reset_trigger",
        |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::ResetTrigger { id: id.to_string() });
        },
    );
    engine.register_fn(
        "load_world",
        |sink: &mut EffectSink, path: ImmutableString| {
            // `loader_path` is `None` here: a script-issued load is authored at base
            // scope in M1 (no sub-world layer origin to thread yet). Mirrors
            // `dispatch_action`'s `LoadWorld` when `origin_layer` is `None`.
            sink.push(ActionCmd::LoadWorld {
                path: path.to_string(),
                loader_path: None,
            });
        },
    );
    engine.register_fn(
        "unload_world",
        |sink: &mut EffectSink, path: ImmutableString| {
            sink.push(ActionCmd::UnloadWorld {
                path: path.to_string(),
            });
        },
    );
    engine.register_fn(
        "game_over",
        |sink: &mut EffectSink, reason: ImmutableString| {
            // Reason first, then the transition — `OnEnter(GamePhase::GameOver)`
            // reads the reason, so the ordering is load-bearing. Mirrors
            // `dispatch_state_action`'s `GameOver` handling. `outcome` is `None`:
            // an undeclared scripted end (the headless classifier defaults it to
            // victory), matching `TriggerAction::GameOver { outcome: None }`.
            sink.push(ActionCmd::SetGameOverReason {
                reason: reason.to_string(),
                outcome: None,
            });
            sink.push(ActionCmd::SetNextState {
                phase: crate::messages::GamePhase::GameOver,
            });
        },
    );
    engine.register_fn(
        "game_over",
        |sink: &mut EffectSink,
         reason: ImmutableString,
         outcome: ImmutableString|
         -> Result<(), Box<EvalAltResult>> {
            // The outcome-DECLARING end (#843). The outcome is validated through
            // the SAME `crate::balance::Outcome::parse` the declarative `game_over`
            // action uses, so a scripted typo (`"victni"`) raises a Rhai error —
            // discarding this call's effects (settled decision 10) — exactly as a
            // bad `outcome = "…"` fails the declarative world load. Only
            // `victory`/`defeat` are accepted (`Outcome` has no `Draw`). Emits the
            // same two commands as the 1-arg form, reason first, differing only in
            // `outcome: Some(_)`.
            let outcome = crate::balance::Outcome::parse(&outcome)
                .map_err(|e| raise(format!("game_over: {e}")))?;
            sink.push(ActionCmd::SetGameOverReason {
                reason: reason.to_string(),
                outcome: Some(outcome),
            });
            sink.push(ActionCmd::SetNextState {
                phase: crate::messages::GamePhase::GameOver,
            });
            Ok(())
        },
    );
    engine.register_fn(
        "add_faction_enemy",
        |sink: &mut EffectSink, faction: ImmutableString, enemy: ImmutableString| {
            // Buffers the DECLARATIVE action carrying faction NAMES: no
            // `FactionRegistry` is in scope at this boundary, so name→UUID
            // resolution is deferred to the applier's `dispatch_action`, exactly
            // as the declarative `add_faction_enemy` action resolves it (#984 M6).
            sink.push_action(TriggerAction::AddFactionEnemy {
                faction: faction.to_string(),
                enemy: enemy.to_string(),
            });
        },
    );
    engine.register_fn(
        "add_objective",
        |sink: &mut EffectSink, spec: Map| -> Result<(), Box<EvalAltResult>> {
            // Read the script map into a `RawActionEntry` and run the SHARED
            // `parse_action_entry`, so directive-kind validation and utility
            // parsing are byte-identical to the declarative `add_objective`; the
            // resulting `TriggerAction::AddObjective` is buffered for the applier
            // to resolve `targets` through the same dispatch (#984 M6).
            let action = add_objective_action(&spec).map_err(raise)?;
            sink.push_action(action);
            Ok(())
        },
    );
    engine.register_fn(
        "spawn_entity",
        |sink: &mut EffectSink, spec: Map| -> Result<(), Box<EvalAltResult>> {
            // Same reuse as `add_objective`: read the map into a `RawActionEntry`,
            // run `parse_action_entry` (which enforces the anchor/position XOR),
            // and buffer `TriggerAction::SpawnEntity`. The applier resolves the
            // anchor, loads the template, and — crucially — mints the `EntityUuid`
            // inside `dispatch_spawn_entity` from the SAME `uuid_source` the
            // declarative path uses, so a converted world mints in the same order
            // (#984 M6). Nothing is minted here.
            let action = spawn_entity_action(&spec).map_err(raise)?;
            sink.push_action(action);
            Ok(())
        },
    );
}

/// Build a Rhai runtime error from a host-fn message. Raising discards the
/// call's whole effect buffer under the failure policy (settled decision 10),
/// so a malformed `add_objective` / `spawn_entity` map or a bad `game_over`
/// outcome drops the call rather than emitting a half-built effect.
fn raise(message: String) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(message.into(), Position::NONE))
}

/// Read an optional string field out of a script map (mirrors the comms map
/// readers). `None` when the key is absent or not a string.
fn map_str(spec: &Map, key: &str) -> Option<String> {
    spec.get(key).and_then(|d| d.clone().into_string().ok())
}

/// Read an optional bool field. `None` when absent or not a bool.
fn map_bool(spec: &Map, key: &str) -> Option<bool> {
    spec.get(key).and_then(|d| d.as_bool().ok())
}

/// Read a KNOWN-`f32` scalar (`base_priority`). `no_float`: authored as an INT,
/// so it is read as an integer and converted at this boundary to the same `f32`
/// the declarative float parses to (`80` → `80.0`) — the seconds→elapsed rule.
fn map_f32(spec: &Map, key: &str) -> Option<f32> {
    spec.get(key)
        .and_then(|d| d.as_int().ok())
        .map(|i| i as f32)
}

/// Read an optional array-of-strings field (`targets` / `groups` /
/// `directive_anchors`). `Ok(None)` when absent; `Err` when present but not an
/// array of strings, so a malformed spec raises rather than silently dropping.
fn map_string_array(spec: &Map, key: &str) -> Result<Option<Vec<String>>, String> {
    let Some(d) = spec.get(key) else {
        return Ok(None);
    };
    let arr = d
        .clone()
        .into_array()
        .map_err(|actual| format!("`{key}` must be an array of strings, got {actual}"))?;
    let mut out = Vec::with_capacity(arr.len());
    for (i, el) in arr.into_iter().enumerate() {
        out.push(
            el.into_string()
                .map_err(|actual| format!("`{key}`[{i}] must be a string, got {actual}"))?,
        );
    }
    Ok(Some(out))
}

/// Read a KNOWN-`[f32; 3]` field (`position` / `rotation` / `scale`). Each
/// coordinate is authored as an INT (`no_float`) and converted here, so a script
/// `[900, 0, -560]` becomes the identical `[f32; 3]` a declarative
/// `[900.0, 0.0, -560.0]` parses to (pinned by the mint / structural parity
/// tests). `Ok(None)` when absent; `Err` when present but not a 3-element integer
/// array.
fn map_f32_array3(spec: &Map, key: &str) -> Result<Option<[f32; 3]>, String> {
    let Some(d) = spec.get(key) else {
        return Ok(None);
    };
    let arr = d
        .clone()
        .into_array()
        .map_err(|actual| format!("`{key}` must be a 3-element array, got {actual}"))?;
    if arr.len() != 3 {
        return Err(format!(
            "`{key}` must have exactly 3 elements, got {}",
            arr.len()
        ));
    }
    let mut out = [0.0f32; 3];
    for (i, el) in arr.into_iter().enumerate() {
        out[i] = el
            .as_int()
            .map_err(|actual| format!("`{key}`[{i}] must be an integer, got {actual}"))?
            as f32;
    }
    Ok(Some(out))
}

/// Convert a `spawn_entity` `overrides` subtree (`#{ … }`) into the `toml::Value`
/// the declarative `overrides` field carries.
///
/// Every numeric leaf renders as a toml FLOAT, NOT an integer: the whole script
/// API is `no_float`, so an author writes `range: 200` (an INT), but
/// `EntityConfig`'s numeric fields are overwhelmingly floats and a declarative
/// `overrides` authors them as floats — so `200` must become `200.0` to
/// deserialize into the same config `overrides = { range = 200.0 }` produces.
/// Recurses through arrays and nested maps (the `behaviour.doctrine`
/// array-of-maps, the radar string arrays). Pinned by the `range: 200 ≡
/// range = 200.0` structural-parity assertion.
///
/// LIMITATION: because every numeric leaf becomes a float, a scripted override
/// CANNOT target a genuine INTEGER `EntityConfig` field — it would render as
/// `field = 3.0`, fail to deserialize into an integer field, and
/// `dispatch_spawn_entity` drops the whole override (keeping the template) with a
/// warning. No shipped or target world overrides an integer field (they touch
/// radar range, doctrine, and faction — all float/string/array); schema-aware
/// int handling is deferred to a follow-up, because `no_float` gives the author
/// no way to disambiguate an int-target from a float-target leaf.
fn dynamic_to_toml(value: &Dynamic) -> Result<toml::Value, String> {
    // Bool before int: the two are distinct `Dynamic` types, so the order is for
    // total coverage, not disambiguation.
    if let Ok(b) = value.as_bool() {
        return Ok(toml::Value::Boolean(b));
    }
    if let Ok(i) = value.as_int() {
        return Ok(toml::Value::Float(i as f64));
    }
    if let Some(s) = value.clone().try_cast::<ImmutableString>() {
        return Ok(toml::Value::String(s.to_string()));
    }
    if let Some(arr) = value.clone().try_cast::<Array>() {
        let mut out = Vec::with_capacity(arr.len());
        for el in &arr {
            out.push(dynamic_to_toml(el)?);
        }
        return Ok(toml::Value::Array(out));
    }
    if let Some(map) = value.clone().try_cast::<Map>() {
        let mut table = toml::map::Map::new();
        for (k, v) in map.iter() {
            table.insert(k.to_string(), dynamic_to_toml(v)?);
        }
        return Ok(toml::Value::Table(table));
    }
    Err(format!(
        "unsupported override value of type '{}'",
        value.type_name()
    ))
}

/// Build a `TriggerAction::AddObjective` from a script `#{ … }` map, reusing the
/// declarative `parse_action_entry` for directive / utility validation.
///
/// `modifiers` / `zero_gates` (utility arrays of maps) are not yet read here.
/// They are REJECTED loudly rather than silently dropped — silently ignoring a
/// key the declarative twin parses would diverge the scripted `UtilityConfig`
/// from the TOML one. The `RawActionEntry`-reuse path makes reading them later
/// free, without touching this builder's shape.
fn add_objective_action(spec: &Map) -> Result<TriggerAction, String> {
    for unsupported in ["modifiers", "zero_gates"] {
        if spec.contains_key(unsupported) {
            return Err(format!(
                "scripted add_objective does not yet support '{unsupported}'; author that \
                 objective declaratively or await the utility-config milestone"
            ));
        }
    }
    let raw = RawActionEntry {
        kind: "add_objective".to_string(),
        id: map_str(spec, "id"),
        text: map_str(spec, "text"),
        mandatory: map_bool(spec, "mandatory"),
        targets: map_string_array(spec, "targets")?,
        target: map_str(spec, "target"),
        directive_kind: map_str(spec, "directive_kind"),
        directive_anchors: map_string_array(spec, "directive_anchors")?,
        directive_loop: map_bool(spec, "directive_loop"),
        directive_anchor: map_str(spec, "directive_anchor"),
        base_priority: map_f32(spec, "base_priority"),
        source: map_str(spec, "source"),
        ..Default::default()
    };
    parse_action_entry(&raw)
}

/// Build a `TriggerAction::SpawnEntity` from a script `#{ … }` map, reusing the
/// declarative `parse_action_entry` for the required-field and anchor/position
/// XOR checks.
fn spawn_entity_action(spec: &Map) -> Result<TriggerAction, String> {
    let raw = RawActionEntry {
        kind: "spawn_entity".to_string(),
        template_path: map_str(spec, "template_path"),
        name: map_str(spec, "name"),
        anchor: map_str(spec, "anchor"),
        position: map_f32_array3(spec, "position")?,
        rotation: map_f32_array3(spec, "rotation")?,
        scale: map_f32_array3(spec, "scale")?,
        groups: map_string_array(spec, "groups")?,
        overrides: match spec.get("overrides") {
            Some(d) => Some(dynamic_to_toml(d)?),
            None => None,
        },
        ..Default::default()
    };
    parse_action_entry(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::config::parse_world;
    use crate::world::script::engine::runtime_engine;
    use crate::world::script::flags::Flags;
    use rhai::{Dynamic, Map};

    /// Build the `#{ effects, flags }` context one call reads. Flags share the one
    /// ordered buffer (issue #981) so a flag write lands in `sink` alongside
    /// effects; these tests write no flags.
    fn make_ctx(sink: &EffectSink) -> Map {
        let mut ctx = Map::new();
        ctx.insert("effects".into(), Dynamic::from(sink.clone()));
        ctx.insert(
            "flags".into(),
            Dynamic::from(Flags::new(
                &crate::world::flags::FlagStore::new(),
                sink.clone(),
            )),
        );
        ctx
    }

    /// Compile `source` on a runtime engine and call `fn_name`, returning the
    /// drained buffer verbatim. A local harness so this module's tests don't
    /// depend on `RuntimeHost`'s failure-mode wrapper.
    fn run_buffered(source: &str, fn_name: &str) -> Vec<BufferedEffect> {
        let engine = runtime_engine();
        let ast = engine.compile(source).expect("compiles");
        let sink = EffectSink::new();
        let ctx = make_ctx(&sink);
        let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).expect("calls");
        sink.take()
    }

    /// Like [`run_buffered`] but for the effect-only (`Cmd`) verbs: unwrap each
    /// buffered effect to its `ActionCmd`. Panics on a name-resolving `Action`, so
    /// a test that uses this on a spawn/objective/faction verb fails loudly.
    fn run(source: &str, fn_name: &str) -> Vec<ActionCmd> {
        run_buffered(source, fn_name)
            .into_iter()
            .map(|e| match e {
                BufferedEffect::Cmd(cmd) => cmd,
                BufferedEffect::Action(a) => {
                    unreachable!("run(): expected only command effects, got {a:?}")
                }
            })
            .collect()
    }

    /// Compile and call, returning the drained buffer on success or the call
    /// error — for the failure-path tests (a raised host fn discards the call).
    fn run_result(
        source: &str,
        fn_name: &str,
    ) -> Result<Vec<BufferedEffect>, vellum_script::CallError> {
        let engine = runtime_engine();
        let ast = engine.compile(source).expect("compiles");
        let sink = EffectSink::new();
        let ctx = make_ctx(&sink);
        vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).map(|_| sink.take())
    }

    /// The single `TriggerAction` the declarative front-end parses for one
    /// `[[trigger.action]]` body, wrapped in a minimal world so serde produces
    /// the `RawActionEntry` independently of this module's map extraction — the
    /// independent source of truth for the M6 structural-parity assertions.
    fn toml_action(action_body: &str) -> TriggerAction {
        let world = format!(
            "[[trigger]]\ncondition = \"on_world_loaded\"\n\n[[trigger.action]]\n{action_body}\n"
        );
        let cfg = parse_world(&world).expect("world parses");
        cfg.triggers[0].actions[0].clone()
    }

    #[test]
    fn complete_objective_drains_to_action_cmd() {
        let cmds = run(
            r#"fn on_x(ctx) { ctx.effects.complete_objective("obj1"); }"#,
            "on_x",
        );
        assert_eq!(
            cmds,
            vec![ActionCmd::CompleteObjective {
                id: "obj1".to_string()
            }]
        );
    }

    #[test]
    fn game_over_emits_reason_then_transition_in_order() {
        let cmds = run(
            r#"fn end(ctx) { ctx.effects.game_over("hull breach"); }"#,
            "end",
        );
        assert_eq!(
            cmds,
            vec![
                ActionCmd::SetGameOverReason {
                    reason: "hull breach".to_string(),
                    outcome: None,
                },
                ActionCmd::SetNextState {
                    phase: crate::messages::GamePhase::GameOver,
                },
            ]
        );
    }

    #[test]
    fn multiple_effects_buffer_in_call_order() {
        let cmds = run(
            r#"fn on_x(ctx) {
                ctx.effects.fail_objective("a");
                ctx.effects.reset_trigger("b");
                ctx.effects.unload_world("w.toml");
            }"#,
            "on_x",
        );
        assert_eq!(
            cmds,
            vec![
                ActionCmd::FailObjective {
                    id: "a".to_string()
                },
                ActionCmd::ResetTrigger {
                    id: "b".to_string()
                },
                ActionCmd::UnloadWorld {
                    path: "w.toml".to_string()
                },
            ]
        );
    }

    #[test]
    fn take_empties_the_buffer() {
        let sink = EffectSink::new();
        sink.push(ActionCmd::CompleteObjective {
            id: "x".to_string(),
        });
        assert_eq!(sink.len(), 1);
        let _ = sink.take();
        assert!(sink.is_empty());
    }

    // ── M6 effects API: the four new verbs (issue #984) ───────────────────────

    /// `game_over(reason, outcome)` emits the outcome-declaring pair, and does so
    /// identically to the declarative `game_over` action dispatched — the outcome
    /// (`victory`) rides through in `Some(_)`, reason first.
    #[test]
    fn game_over_with_outcome_matches_toml() {
        let cmds = run(
            r#"fn end(ctx) { ctx.effects.game_over("world.win", "victory"); }"#,
            "end",
        );
        assert_eq!(
            cmds,
            vec![
                ActionCmd::SetGameOverReason {
                    reason: "world.win".to_string(),
                    outcome: Some(crate::balance::Outcome::Victory),
                },
                ActionCmd::SetNextState {
                    phase: crate::messages::GamePhase::GameOver,
                },
            ]
        );
        // Structural parity: the same two commands the TOML `game_over` action
        // dispatches (`game_over` needs no context, so a bare dispatch suffices).
        assert_eq!(
            cmds,
            dispatch_bare(&toml_action(
                "type = \"game_over\"\nmessage = \"world.win\"\noutcome = \"victory\""
            ))
        );
    }

    /// `defeat` is the other accepted outcome; nothing else is.
    #[test]
    fn game_over_accepts_defeat() {
        let cmds = run(
            r#"fn end(ctx) { ctx.effects.game_over("world.lose", "defeat"); }"#,
            "end",
        );
        assert_eq!(
            cmds[0],
            ActionCmd::SetGameOverReason {
                reason: "world.lose".to_string(),
                outcome: Some(crate::balance::Outcome::Defeat),
            }
        );
    }

    /// A scripted typo raises (discarding the call's effects, settled decision
    /// 10) exactly as a bad declarative `outcome = "…"` fails the world load —
    /// both route through `crate::balance::Outcome::parse`.
    #[test]
    fn game_over_rejects_an_unknown_outcome() {
        let err = run_result(
            r#"fn end(ctx) { ctx.effects.game_over("x", "victni"); }"#,
            "end",
        )
        .expect_err("an unknown outcome must raise");
        assert!(err.to_string().contains("outcome"), "{err}");
    }

    /// `add_faction_enemy(f, e)` buffers the DECLARATIVE `AddFactionEnemy` (faction
    /// names, unresolved) — identical to the TOML action before UUID resolution.
    #[test]
    fn add_faction_enemy_matches_toml() {
        let effs = run_buffered(
            r#"fn f(ctx) { ctx.effects.add_faction_enemy("Harrow", "Federation"); }"#,
            "f",
        );
        let toml = toml_action(
            "type = \"add_faction_enemy\"\nfaction = \"Harrow\"\nenemy = \"Federation\"",
        );
        assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
        assert_eq!(
            effs,
            vec![BufferedEffect::Action(TriggerAction::AddFactionEnemy {
                faction: "Harrow".to_string(),
                enemy: "Federation".to_string(),
            })]
        );
    }

    /// `add_objective(#{…})` reads the script map into the SAME `TriggerAction`
    /// the combat_test-style declarative action parses — directive (`Destroy`) and
    /// utility (`base_priority: 80` INT → `80.0`) included.
    #[test]
    fn add_objective_matches_toml() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "obj-destroy-wave-1",
                    text: "world.combat_test.trigger.action.obj_destroy_wave_1.text",
                    mandatory: true,
                    targets: ["wave_1"],
                    target: "wave_1",
                    directive_kind: "Destroy",
                    base_priority: 80,
                });
            }"#,
            "f",
        );
        let toml = toml_action(
            "type = \"add_objective\"\n\
             id = \"obj-destroy-wave-1\"\n\
             text = \"world.combat_test.trigger.action.obj_destroy_wave_1.text\"\n\
             mandatory = true\n\
             targets = [\"wave_1\"]\n\
             target = \"wave_1\"\n\
             directive_kind = \"Destroy\"\n\
             base_priority = 80.0",
        );
        assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
    }

    /// Issue #984 review: `modifiers`/`zero_gates` are not yet read by the scripted
    /// builder. Supplying one RAISES (settled decision 10) rather than silently
    /// dropping it, which would diverge the scripted `UtilityConfig` from the
    /// declarative twin's (the TOML path DOES parse them).
    #[test]
    fn add_objective_rejects_unsupported_utility_keys() {
        let err = run_result(
            r#"fn f(ctx) { ctx.effects.add_objective(#{ id: "o", text: "t", modifiers: [] }); }"#,
            "f",
        )
        .expect_err("an unsupported utility key must raise");
        assert!(err.to_string().contains("modifiers"), "{err}");
    }

    /// `spawn_entity(#{…})` — the sharpest parity: the Rhai-int `position:
    /// [900, 0, -560]` must produce the identical `[f32; 3]` the declarative float
    /// `[900.0, 0.0, -560.0]` parses to, AND the `overrides` map must convert to
    /// the identical `toml::Value` (numeric leaves as floats) the declarative
    /// `overrides` carries.
    #[test]
    fn spawn_entity_matches_toml_including_int_to_float_and_overrides() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship_harrow_destroyer.toml",
                    name: "wave_1_bonus",
                    position: [900, 0, -560],
                    groups: ["hostiles", "wave_1"],
                    overrides: #{
                        weapons_console: #{
                            radar: #{
                                range: 200,
                                shows: ["player", "ship", "station"],
                            },
                        },
                    },
                });
            }"#,
            "f",
        );
        let toml = toml_action(
            "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship_harrow_destroyer.toml\"\n\
             name = \"wave_1_bonus\"\n\
             position = [900.0, 0.0, -560.0]\n\
             groups = [\"hostiles\", \"wave_1\"]\n\
             overrides = { weapons_console = { radar = { range = 200.0, shows = [\"player\", \"ship\", \"station\"] } } }",
        );
        assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

        // Pin the two conversions concretely so a regression is unmistakable.
        let BufferedEffect::Action(TriggerAction::SpawnEntity {
            position,
            overrides,
            ..
        }) = &effs[0]
        else {
            panic!("expected a spawn action, got {:?}", effs[0]);
        };
        assert_eq!(*position, Some([900.0_f32, 0.0, -560.0]));
        let range = overrides
            .as_ref()
            .and_then(|o| o.get("weapons_console"))
            .and_then(|w| w.get("radar"))
            .and_then(|r| r.get("range"))
            .expect("override range present");
        assert_eq!(
            range,
            &toml::Value::Float(200.0),
            "a no_float INT override leaf must render as a toml FLOAT"
        );
    }

    /// The anchor/position XOR is enforced by the SHARED parser: neither given
    /// raises (discarding the call), exactly as the declarative parse errors.
    #[test]
    fn spawn_entity_requires_anchor_xor_position() {
        let err = run_result(
            r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                });
            }"#,
            "f",
        )
        .expect_err("neither anchor nor position must raise");
        assert!(err.to_string().contains("anchor"), "{err}");
    }

    /// A `Cmd` effect and an `Action` effect keep their authored order in the one
    /// shared buffer — the interleaving guarantee flag writes also rely on.
    #[test]
    fn cmd_and_action_effects_interleave_in_authored_order() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.complete_objective("first");
                ctx.effects.add_faction_enemy("Harrow", "Federation");
                ctx.effects.fail_objective("last");
            }"#,
            "f",
        );
        assert_eq!(
            effs,
            vec![
                BufferedEffect::Cmd(ActionCmd::CompleteObjective {
                    id: "first".to_string()
                }),
                BufferedEffect::Action(TriggerAction::AddFactionEnemy {
                    faction: "Harrow".to_string(),
                    enemy: "Federation".to_string(),
                }),
                BufferedEffect::Cmd(ActionCmd::FailObjective {
                    id: "last".to_string()
                }),
            ]
        );
    }

    /// A minimal context-free dispatch of one action to its `ActionCmd`s, for the
    /// `game_over` structural-parity assertion (which needs no resolution). Mirrors
    /// the comms module's `dispatch_toml`.
    fn dispatch_bare(action: &TriggerAction) -> Vec<ActionCmd> {
        use crate::world::dispatch::{dispatch_action, DispatchContext};
        use std::collections::HashMap;
        let names: HashMap<String, String> = HashMap::new();
        let base_flags = crate::world::flags::FlagStore::new();
        let layers = HashMap::new();
        let anchors = HashMap::new();
        let uuid = || "uuid".to_string();
        let ctx = DispatchContext {
            origin_layer: None,
            entity_name: None,
            name_to_uuid: &names,
            base_flags: &base_flags,
            layers: &layers,
            base_anchors: &anchors,
            factions: None,
            uuid_source: &uuid,
            template_loader: &crate::entity_loader::WasmTemplateLoader,
        };
        dispatch_action(action, &ctx).commands
    }
}
