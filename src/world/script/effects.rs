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
//! One verb does NOT go through that buffer: `open_comms` (issue #984), which
//! asks the comms module to open a scripted dialogue thread. Its
//! [`OpenCommsRequest`] is comms vocabulary, not the world/entity vocabulary the
//! applier dispatches, so it buffers separately and is materialised by a later
//! comms system — see [`EffectSink`].
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
//! A FRACTIONAL leaf an int cannot express (`target_speed = 0.9`, a modifier
//! `weight = 1.5`) is authored through the `flt("…")` marker: it carries a parsed
//! `f64` as OPAQUE DATA (a [`RealLit`], never arithmetic), so `no_float`
//! determinism is preserved AND the value is byte-identical to the declarative
//! `0.9` — Rust's `f64` `FromStr` and toml's float parse agree on canonical
//! decimals, which the parity tests pin. `map_f32` and `dynamic_to_toml` accept a
//! `RealLit` wherever they already accept an int.
//!
//! An `overrides` leaf that targets a genuine INTEGER `EntityConfig` field
//! (issue #1048) is the mirror-image problem: `dynamic_to_toml`'s ambient
//! numeric-leaf rule renders a bare int as a toml FLOAT, because that is what
//! the overwhelming majority of `EntityConfig` numeric fields are. A script
//! that needs the rare int-target field instead says so explicitly with the
//! `int(3)` marker (an [`IntLit`]), the same escape-hatch shape `flt("…")`
//! already uses for the opposite rare case.
//!
//! [`ActionCmd`]: crate::world::dispatch::ActionCmd
//! [`TriggerAction`]: crate::world::config::TriggerAction
//! [`engine::RuntimeHost`]: crate::world::script::engine::RuntimeHost
//! [`flags`]: crate::world::script::flags

use std::sync::{Arc, Mutex};

use rhai::{Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Position};

use crate::comms::content::OpenCommsRequest;
use crate::world::config::{
    parse_action_entry, RawActionEntry, RawCommandStance, RawModifier, RawZeroGate, TriggerAction,
};
use crate::world::dispatch::{ActionCmd, FlagMutation};
use crate::world::script::registry::{host_fn, HostRegistry};

/// A fractional literal carried as OPAQUE DATA, the `no_float`-safe fractional-leaf
/// marker (issue #984, Rhai M6 follow-on).
///
/// Rhai is `no_float`, so an author cannot write `0.9` and the API is otherwise
/// integer-only. `flt("0.9")` parses the string ONCE, at map-build time, into this
/// wrapper; the `f64` is then only ever *read* (by [`dynamic_to_toml`] and
/// [`map_f32`]) and never arithmetic'd in script, so determinism is untouched — a
/// converted world does no float math, it merely transports a constant the toml
/// crate would have parsed identically. Rust's `f64::from_str` and toml's float
/// parse agree on canonical decimals, so `flt("0.9")` produces the SAME `f64` a
/// declarative `0.9` does; the override / utility parity tests pin it.
#[derive(Clone, Debug)]
pub struct RealLit(pub f64);

/// An integer literal carried as an explicit INTEGER-target marker, the
/// `no_float`-safe mirror of [`RealLit`] (issue #1048).
///
/// # Why a marker, and not schema-aware conversion
///
/// `dynamic_to_toml` cannot tell, from a bare Rhai int alone, whether the
/// author means "the target field is a float, and `200` is shorthand for
/// `200.0`" (the overwhelming common case — `range`, `base_priority`,
/// `target_speed`, …) or "the target field is a genuine integer, and `3`
/// must render as a toml INTEGER" (`repair.repair_team_count`, `volley_count`,
/// …). Resolving that honestly would mean knowing `EntityConfig`'s declared
/// field type before rendering the leaf — real schema awareness, which this
/// codebase has no reflection story for short of either (a) parsing serde's
/// derive output at compile time (nothing here does that, and bolting it on
/// for one conversion function is a disproportionate lift), or (b) a
/// hand-maintained field-name → type table, which is the option the #1048
/// review explicitly rejected: it rots the moment a new integer field is
/// added to `EntityConfig` and nothing forces the table to be told.
///
/// So the marker: `int(3)` says, at the ONE leaf that needs it, "this is an
/// integer target" — mirroring `flt("0.9")`'s "this is a fractional value"
/// for the opposite rare case. Both are opt-in escape hatches over one
/// ambient default (bare int ⇒ float), so the vastly more common float-target
/// override needs no annotation at all, exactly as today.
///
/// # Why a plain `i64`, not a parsed string like `flt`
///
/// `flt` parses a STRING because Rhai's `no_float` build has no float literal
/// at all — there is no other way to get an `f64` into a script. An integer
/// target has no such gap: Rhai's native int type already IS the value an
/// integer field wants, so `int(3)` wraps the `i64` directly. `int("3")`
/// would add a parse step this marker has no reason to pay for.
#[derive(Clone, Debug)]
pub struct IntLit(pub i64);

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
///
/// A SECOND buffer holds the call's [`OpenCommsRequest`]s (issue #984). They are
/// deliberately not `BufferedEffect`s: an open is comms vocabulary, materialised
/// by a later comms system, while the ordered buffer is the world/entity
/// `ActionCmd`/`TriggerAction` vocabulary the applier dispatches (the #816
/// split). The cost is that an open is not interleaved with flag writes in
/// authored order — unobservable, since nothing reads the thread within the
/// call, and the same trade `delayed`/`callbacks` already make. Both buffers are
/// drained together on the success path and dropped whole on the failure path,
/// so a raising call discards its opens exactly as it discards its effects
/// (settled decision 10).
#[derive(Clone, Default)]
pub struct EffectSink {
    effects: Arc<Mutex<Vec<BufferedEffect>>>,
    opens: Arc<Mutex<Vec<OpenCommsRequest>>>,
}

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
        self.effects
            .lock()
            .expect("effect sink lock")
            .push(BufferedEffect::Cmd(cmd));
    }

    /// Push one unresolved declarative action (as [`BufferedEffect::Action`]),
    /// onto the SAME ordered buffer as [`push`](Self::push) so a name-resolving
    /// effect keeps its authored position relative to flag writes and command
    /// effects. The applier resolves it through `dispatch_action` (issue #984, M6).
    pub(crate) fn push_action(&self, action: TriggerAction) {
        self.effects
            .lock()
            .expect("effect sink lock")
            .push(BufferedEffect::Action(action));
    }

    /// Push one comms-thread open onto the SECOND buffer (issue #984). The
    /// request arrives without its `script_path` — the sink cannot know which
    /// unit is running — and [`take_opens`](Self::take_opens) stamps it.
    pub(crate) fn push_open(&self, open: OpenCommsRequest) {
        self.opens.lock().expect("effect sink lock").push(open);
    }

    /// Drain the buffer, leaving it empty. Called by the host on the success
    /// path only — on the failure path the buffer is dropped whole, which is
    /// how "discard the call's effects" (settled decision 10) is enforced.
    pub fn take(&self) -> Vec<BufferedEffect> {
        std::mem::take(&mut self.effects.lock().expect("effect sink lock"))
    }

    /// Drain the comms-open buffer, stamping every request with the running
    /// unit's `script_path` — the same host-boundary stamping
    /// [`ScheduleSink::drain`](super::schedule::ScheduleSink::drain) applies to a
    /// callback's path, and for the same reason (a short or anonymous fn name is
    /// not unique across units). Success path only, like [`take`](Self::take).
    pub fn take_opens(&self, script_path: &str) -> Vec<OpenCommsRequest> {
        std::mem::take(&mut *self.opens.lock().expect("effect sink lock"))
            .into_iter()
            .map(|open| OpenCommsRequest {
                script_path: script_path.to_string(),
                ..open
            })
            .collect()
    }

    /// Number of buffered effects (test/introspection helper).
    pub fn len(&self) -> usize {
        self.effects.lock().expect("effect sink lock").len()
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
/// Register the `no_float`-safe fractional-leaf marker on an engine.
///
/// `flt("0.9")` parses the string into a [`RealLit`] its readers unwrap; the
/// parse happens ONCE, at map-build time, and the `f64` is never arithmetic'd in
/// script — so a fractional constant reaches the declarative boundary with
/// determinism intact. A bad string raises (discarding the call, settled
/// decision 10), exactly as a malformed declarative float fails the world load.
///
/// Registered on BOTH engines: the runtime engine reads it inside handler maps
/// (`target_speed: flt("0.9")`), and the LOADING engine needs it because a
/// trigger registration can carry a fractional condition field —
/// `on_hull_below(entity, flt("0.75"), handler)` is authored at a unit's top
/// level, which only the loading engine ever runs. One marker, one spelling,
/// wherever a fraction has to be said.
pub fn register_real_lit(engine: &mut Engine) {
    engine.register_type_with_name::<RealLit>("RealLit");
    engine.register_fn(
        "flt",
        |s: ImmutableString| -> Result<RealLit, Box<EvalAltResult>> {
            s.parse::<f64>()
                .map(RealLit)
                .map_err(|e| raise(format!("flt(\"{s}\"): not a real number: {e}")))
        },
    );
}

/// Register the `no_float`-safe INTEGER-target marker on an engine (issue
/// #1048). See [`IntLit`] for why this exists and why it wraps an `i64`
/// directly rather than parsing a string the way `flt` does.
///
/// Runtime engine only, unlike [`register_real_lit`]: `int(…)` is only ever
/// meaningful inside a `spawn_entity` `overrides` map, which is `Effects`
/// vocabulary (registered by [`register_effects`], runtime-only) — nothing
/// authored at a unit's top level (the loading engine's whole surface) ever
/// reaches `dynamic_to_toml`.
pub fn register_int_lit(engine: &mut Engine) {
    engine.register_type_with_name::<IntLit>("IntLit");
    engine.register_fn("int", |i: i64| IntLit(i));
}

pub(crate) fn register_effects(engine: &mut HostRegistry) {
    engine.register_type_with_name::<EffectSink>("Effects");

    // The `flt(…)` / `int(…)` markers are not editor-exposed, so bare
    // registrations.
    register_real_lit(engine.engine_mut());
    register_int_lit(engine.engine_mut());

    host_fn!(
        engine,
        "complete_objective",
        receiver = "effects",
        category = "effect",
        params = ["id"],
        summary = "Mark the objective complete.",
        |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::CompleteObjective { id: id.to_string() });
        },
    );
    host_fn!(
        engine,
        "fail_objective",
        receiver = "effects",
        category = "effect",
        params = ["id"],
        summary = "Mark the objective failed.",
        |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::FailObjective { id: id.to_string() });
        },
    );
    host_fn!(
        engine,
        "reset_trigger",
        receiver = "effects",
        category = "effect",
        params = ["id"],
        summary = "Re-arm a fired trigger by id.",
        |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::ResetTrigger { id: id.to_string() });
        },
    );
    host_fn!(
        engine,
        "load_world",
        receiver = "effects",
        category = "effect",
        params = ["path"],
        summary = "Load the world layer at `path`.",
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
    host_fn!(
        engine,
        "unload_world",
        receiver = "effects",
        category = "effect",
        params = ["path"],
        summary = "Unload the world layer at `path`.",
        |sink: &mut EffectSink, path: ImmutableString| {
            sink.push(ActionCmd::UnloadWorld {
                path: path.to_string(),
            });
        },
    );
    host_fn!(
        engine,
        "game_over",
        receiver = "effects",
        category = "effect",
        params = ["reason"],
        summary = "End the game with a reason string.",
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
                phase: crate::core::messages::GamePhase::GameOver,
            });
        },
    );
    // The outcome-DECLARING overload is not a separate editor entry — one
    // descriptor per callable name — so a bare registration.
    engine.register_fn(
        "game_over",
        |sink: &mut EffectSink,
         reason: ImmutableString,
         outcome: ImmutableString|
         -> Result<(), Box<EvalAltResult>> {
            // The outcome-DECLARING end (#843). The outcome is validated through
            // the SAME `crate::core::balance::Outcome::parse` the declarative `game_over`
            // action uses, so a scripted typo (`"victni"`) raises a Rhai error —
            // discarding this call's effects (settled decision 10) — exactly as a
            // bad `outcome = "…"` fails the declarative world load. Only
            // `victory`/`defeat` are accepted (`Outcome` has no `Draw`). Emits the
            // same two commands as the 1-arg form, reason first, differing only in
            // `outcome: Some(_)`.
            let outcome = crate::core::balance::Outcome::parse(&outcome)
                .map_err(|e| raise(format!("game_over: {e}")))?;
            sink.push(ActionCmd::SetGameOverReason {
                reason: reason.to_string(),
                outcome: Some(outcome),
            });
            sink.push(ActionCmd::SetNextState {
                phase: crate::core::messages::GamePhase::GameOver,
            });
            Ok(())
        },
    );
    // Infrastructure condition hooks (issue #1025). Two verbs rather than one
    // signed one: the sign convention lives in the name, so a scenario cannot
    // repair a skyhook by getting a minus sign wrong. Each takes whole
    // condition POINTS, with a `flt("…")` overload for the fractional slice a
    // timed operation applies per tick — the same `no_float` boundary
    // `on_hull_below(entity, flt("0.75"), …)` uses.
    //
    // Both buffer a resolved `ActionCmd` carrying the entity NAME; the applier
    // resolves it and queues the delta for `tick_infrastructure_condition`,
    // which is where every operational-flag edge is detected and mirrored.
    host_fn!(
        engine,
        "repair_infrastructure",
        receiver = "effects",
        category = "effect",
        params = ["entity", "points"],
        summary = "Raise the named structure's infrastructure condition by whole \
                  points, or by a `flt(\"…\")` slice. No delayed form — a timed \
                  repair applies a slice per tick.",
        |sink: &mut EffectSink, entity: ImmutableString, points: i64| {
            sink.push(ActionCmd::AdjustInfrastructureCondition {
                entity: entity.to_string(),
                delta: points as f32,
            });
        },
    );
    // The fractional `flt(…)` overload shares the one editor entry above.
    engine.register_fn(
        "repair_infrastructure",
        |sink: &mut EffectSink, entity: ImmutableString, points: RealLit| {
            sink.push(ActionCmd::AdjustInfrastructureCondition {
                entity: entity.to_string(),
                delta: points.0 as f32,
            });
        },
    );
    host_fn!(
        engine,
        "damage_infrastructure",
        receiver = "effects",
        category = "effect",
        params = ["entity", "points"],
        summary = "Lower the named structure's infrastructure condition by whole \
                  points, or by a `flt(\"…\")` slice.",
        |sink: &mut EffectSink, entity: ImmutableString, points: i64| {
            sink.push(ActionCmd::AdjustInfrastructureCondition {
                entity: entity.to_string(),
                delta: -(points as f32),
            });
        },
    );
    // The fractional `flt(…)` overload shares the one editor entry above.
    engine.register_fn(
        "damage_infrastructure",
        |sink: &mut EffectSink, entity: ImmutableString, points: RealLit| {
            sink.push(ActionCmd::AdjustInfrastructureCondition {
                entity: entity.to_string(),
                delta: -(points.0 as f32),
            });
        },
    );
    // Infrastructure CAPACITY (issue #1042). The third door onto a structure's
    // published numbers, beside the two condition verbs above and the
    // `transfer` operation below, and the only one a scenario can aim at a
    // quantity it worked out for itself.
    //
    // ONE signed verb rather than the spend/return pair the condition hooks
    // are, which is the opposite call and deliberately so: a condition move has
    // a different FICTION on each side — repairing and damaging are two things a
    // crew do — where a capacity move has one, "this published count moves by
    // this much". Splitting it would force a scenario publishing a computed
    // value to branch on the sign of its own arithmetic before it could name the
    // verb it wanted.
    //
    // Whole units, and no `flt` overload, because a capacity is a count and the
    // world counter it mirrors onto is an `i64` (`CapacityConfig::amount` says
    // so at length).
    engine.register_fn(
        "adjust_capacity",
        |sink: &mut EffectSink, entity: ImmutableString, capacity: ImmutableString, delta: i64| {
            sink.push(ActionCmd::AdjustInfrastructureCapacity {
                entity: entity.to_string(),
                capacity: capacity.to_string(),
                delta,
            });
        },
    );
    // Civilian order hooks (issue #1028). Four verbs rather than one taking a
    // verb string, for the reason the two infrastructure verbs are two: the
    // vocabulary lives in the name, so a scenario cannot divert a hauler onto a
    // lane by misspelling an anchor and having it read as an anchor anyway.
    // `divert` is split by destination for exactly that reason — a single
    // `order_divert(entity, "depot_run")` could not tell a route id from an
    // anchor name, and guessing is how a mistyped lane becomes a silent no-op.
    //
    // Each buffers a resolved `ActionCmd` carrying the civilian's NAME; the
    // applier resolves it and queues the order for `tick_civilian_traffic`,
    // which is where the acknowledgement delay and the authored disposition are
    // applied. A scripted order is a request, not a remote control: a civilian
    // whose disposition refuses `divert` refuses a scripted divert too.
    host_fn!(
        engine,
        "order_hold",
        receiver = "effects",
        category = "effect",
        params = ["entity"],
        summary = "Order the named civilian to stop where it is. A request, not a \
                  remote control: it is answered after the hull's authored \
                  acknowledgement delay and may be refused.",
        |sink: &mut EffectSink, entity: ImmutableString| {
            sink.push(ActionCmd::OrderCivilian {
                entity: entity.to_string(),
                order: crate::civilian::CivilianOrder::Hold,
            });
        },
    );
    host_fn!(
        engine,
        "order_divert_route",
        receiver = "effects",
        category = "effect",
        params = ["entity", "route"],
        summary = "Order the named civilian onto another authored `[[route]]`, by \
                  route id. Refusable.",
        |sink: &mut EffectSink, entity: ImmutableString, route: ImmutableString| {
            sink.push(ActionCmd::OrderCivilian {
                entity: entity.to_string(),
                order: crate::civilian::CivilianOrder::divert_to_route(route.to_string()),
            });
        },
    );
    host_fn!(
        engine,
        "order_divert_anchor",
        receiver = "effects",
        category = "effect",
        params = ["entity", "anchor"],
        summary = "Order the named civilian to make for a single world anchor, by \
                  anchor name. Refusable.",
        |sink: &mut EffectSink, entity: ImmutableString, anchor: ImmutableString| {
            sink.push(ActionCmd::OrderCivilian {
                entity: entity.to_string(),
                order: crate::civilian::CivilianOrder::divert_to_anchor(anchor.to_string()),
            });
        },
    );
    host_fn!(
        engine,
        "order_dock",
        receiver = "effects",
        category = "effect",
        params = ["entity", "structure"],
        summary = "Order the named civilian to proceed to and berth at the named \
                  structure. Refusable, and lands in `non_compliant` if the \
                  structure is not there.",
        |sink: &mut EffectSink, entity: ImmutableString, structure: ImmutableString| {
            sink.push(ActionCmd::OrderCivilian {
                entity: entity.to_string(),
                order: crate::civilian::CivilianOrder::dock_at(structure.to_string()),
            });
        },
    );
    // Labour dispute hooks (issue #1035). Three verbs rather than one setter,
    // for the reason `repair_infrastructure` and `damage_infrastructure` are
    // two: the direction lives in the name, so a scenario cannot end a strike by
    // getting a boolean the wrong way round.
    //
    // Each pushes TWO commands, in this order: the register move, then the
    // mirror flag. The flag is an ordinary `MutateFlag` on the same ordered
    // buffer `ctx.flags.x = 1` uses, so `apply_script_commands` previews its
    // transition and an `on_flag_cleared("workforce.<id>.on_strike", …)` trigger
    // authored by the negotiation slice chains off a settlement — through
    // machinery that was already there, and without this vocabulary knowing
    // triggers exist.
    //
    // A settlement that changes nothing still writes its flag to the value it
    // already held: `preview_mutation` sees no transition and emits no event, so
    // the idempotent case costs a redundant write and never a spurious chain.
    for (name, mutation, flag_value) in [
        (
            "call_strike",
            crate::world::workforce::WorkforceMutation::CallStrike,
            1,
        ),
        (
            "settle_strike",
            crate::world::workforce::WorkforceMutation::Settle,
            0,
        ),
    ] {
        engine.register_fn(name, move |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::SetWorkforceState {
                id: id.to_string(),
                mutation,
            });
            sink.push(ActionCmd::MutateFlag {
                target_layer: None,
                name: crate::world::workforce::strike_flag(&id),
                mutation: FlagMutation::SetValue(flag_value),
            });
        });
    }
    // The tactical restraint lever, from the scenario's side (issue #1041).
    // Two verbs rather than one setter, for the reason the strike hooks above
    // are two and `repair_infrastructure`/`damage_infrastructure` are two: the
    // direction lives in the name, so a scenario cannot arm a ship it meant to
    // silence by getting a boolean the wrong way round.
    //
    // ONE command each, unlike the strike hooks: the mirror flag
    // (`weapons_hold.<name>`) is written off the authoritative component by
    // `mirror_weapons_hold_flags`, because a weapons hold has a second author —
    // the ship's own captain — and a flag pushed here would have covered the
    // scenario's orders and silently missed the crew's.
    for (name, held) in [("hold_fire", true), ("release_fire", false)] {
        engine.register_fn(
            name,
            move |sink: &mut EffectSink, entity: ImmutableString| {
                sink.push(ActionCmd::SetWeaponsHold {
                    entity: entity.to_string(),
                    held,
                });
            },
        );
    }
    engine.register_fn(
        "set_workforce_disposition",
        |sink: &mut EffectSink, id: ImmutableString, value: i64| {
            // Clamped here as well as in the register, because the mirror flag
            // is written from THIS value and a script that asked for 9,000 must
            // not leave the flag saying 9,000 while the record says 100.
            let value = value.clamp(
                crate::world::workforce::DISPOSITION_MIN,
                crate::world::workforce::DISPOSITION_MAX,
            );
            sink.push(ActionCmd::SetWorkforceState {
                id: id.to_string(),
                mutation: crate::world::workforce::WorkforceMutation::SetDisposition(value),
            });
            sink.push(ActionCmd::MutateFlag {
                target_layer: None,
                name: crate::world::workforce::disposition_flag(&id),
                mutation: FlagMutation::SetValue(value),
            });
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
        "destroy_entity",
        |sink: &mut EffectSink, entity: ImmutableString| -> Result<(), Box<EvalAltResult>> {
            // The counterpart to `spawn_entity` (issue #1033), and buffered for the
            // SAME reason `add_faction_enemy` is: the name→uuid map lives on
            // `WorldContentRuntime`, not at this host-fn boundary, so resolution is
            // deferred to the applier's `dispatch_action` → `dispatch_destroy_entity`.
            //
            // That deferral is what makes a scripted destruction chain. The pure
            // dispatcher pushes `WorldEvent::Destroyed` onto `DispatchResult::new_events`
            // beside the `ActionCmd::DestroyEntity`, and `apply_script_commands` feeds
            // the WHOLE result to `apply_dispatch_result` — whose `events_out` is
            // `tick_trigger_pipeline`'s `next_events`. So an `on_destroyed` handler and
            // an `on_all_destroyed` group both fire on the next chaining pass of the
            // SAME tick, exactly as they do for a combat kill. Emitting the despawn
            // from here instead would kill the entity and chain nothing.
            //
            // `parse_action_entry`, not a hand-built variant: the required-`entity`
            // check is then the declarative one rather than a second copy of it.
            let action = destroy_entity_action(&entity).map_err(raise)?;
            sink.push_action(action);
            Ok(())
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
    host_fn!(
        engine,
        "open_comms",
        receiver = "effects",
        category = "effect",
        params = ["spec"],
        summary = "Open a scripted comms thread: `#{from, node_fn, display_name?, \
                  thread_id?, urgent?}`. No delayed form — defer it with \
                  `schedule.after`.",
        |sink: &mut EffectSink, spec: Map| -> Result<(), Box<EvalAltResult>> {
            // Comms vocabulary, so it buffers onto the sink's SECOND buffer
            // rather than the ordered `ActionCmd`/`TriggerAction` one (see
            // `EffectSink`). Read with the same map idiom as `add_objective` /
            // `spawn_entity`: a missing required key raises, discarding the call.
            let open = open_comms_request(&spec).map_err(raise)?;
            sink.push_open(open);
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
///
/// `pub(super)` because the sibling `commitments` (issue #1029) and `dossier`
/// (issue #1031) vocabularies raise on the same terms — a malformed map, a
/// duplicate id, an unknown provenance — and must produce the same kind of
/// error: two spellings of "raise" would be two failure policies.
pub(super) fn raise(message: String) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(message.into(), Position::NONE))
}

/// Read an optional string field out of a script map (mirrors the comms map
/// readers). `None` when the key is absent or not a string.
pub(super) fn map_str(spec: &Map, key: &str) -> Option<String> {
    spec.get(key).and_then(|d| d.clone().into_string().ok())
}

/// Read an optional bool field. `None` when absent or not a bool.
fn map_bool(spec: &Map, key: &str) -> Option<bool> {
    spec.get(key).and_then(|d| d.as_bool().ok())
}

/// Read a KNOWN-`f32` scalar (`base_priority`, a modifier `weight` / `threshold`).
/// `no_float`: authored as an INT (`80` → `80.0`, the seconds→elapsed rule) OR — for
/// a fractional value an int cannot express — as a [`RealLit`] via `flt("0.9")`,
/// unwrapped `.0 as f32`. Both routes land on the SAME `f32` the declarative float
/// parses to: an int through `as f32`, and a `flt("0.9")` through the identical
/// `f64` → `f32` narrowing the toml `f32` deserializer applies to `0.9`. `None`
/// when the key is absent (or present but neither an int nor a `RealLit`).
fn map_f32(spec: &Map, key: &str) -> Option<f32> {
    let d = spec.get(key)?;
    if let Some(real) = d.clone().try_cast::<RealLit>() {
        return Some(real.0 as f32);
    }
    d.as_int().ok().map(|i| i as f32)
}

/// Read an optional `#{ … }` of runtime values to interpolate into a text id's
/// `{placeholder}` tokens (see `messages::TEXT_PARAMS_SUFFIX`).
///
/// Collected into a `BTreeMap` so the wire encoding is key-ordered and the same
/// authored call always produces the same bytes — a `HashMap` here would make
/// the payload's encoding depend on hash order.
///
/// Values are rendered to `String` at this seam rather than carried as a typed
/// union, because interpolation is textual substitution and the client has no
/// use for the distinction. A script authors an INT (this engine is built
/// `no_float`, so a computed figure arrives as `14`), a string, or a bool;
/// anything else — a map, an array, a unit — is an authoring error and raises,
/// discarding the call rather than rendering Rhai's debug form into crew-facing
/// copy.
pub(super) fn map_text_params(
    spec: &Map,
    key: &str,
) -> Result<Option<std::collections::BTreeMap<String, String>>, String> {
    let Some(d) = spec.get(key) else {
        return Ok(None);
    };
    let map = d
        .clone()
        .try_cast::<Map>()
        .ok_or_else(|| format!("`{key}` must be a #{{ name: value }} map"))?;
    let mut out = std::collections::BTreeMap::new();
    for (name, value) in map {
        let type_name = value.type_name();
        let rendered = if value.is_string() {
            value
                .into_string()
                .map_err(|actual| format!("`{key}.{name}` must be a string, got {actual}"))?
        } else if let Ok(i) = value.as_int() {
            i.to_string()
        } else if let Ok(b) = value.as_bool() {
            b.to_string()
        } else {
            return Err(format!(
                "`{key}.{name}` must be a string, an integer or a bool, got {type_name}"
            ));
        };
        out.insert(name.to_string(), rendered);
    }
    Ok(Some(out))
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
/// array. [`RealLit`] support is deliberately omitted: no shipped world authors a
/// fractional transform, and a stray `flt()` here raises loudly rather than
/// silently diverging — add the branch only when a world actually needs it.
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

/// Read an `add_objective` `modifiers` array — `[#{ condition, threshold?, weight }]`
/// — into `Vec<RawModifier>`, so the SHARED `parse_utility_config` builds the exact
/// same `UtilityConfig` the declarative twin does; nothing utility-specific is
/// re-implemented here. `weight` is required and `threshold` optional, both read
/// through [`map_f32`] so each is authored as `flt("…")` or an int and lands on the
/// byte-identical `f32` the declarative `weight = …` / `threshold = …` parses to.
/// `Ok(None)` when absent; `Err` — which discards the whole call (settled decision
/// 10) — when present but not an array of well-formed maps, rather than silently
/// dropping a modifier the declarative path would have parsed.
fn map_modifiers(spec: &Map) -> Result<Option<Vec<RawModifier>>, String> {
    let Some(d) = spec.get("modifiers") else {
        return Ok(None);
    };
    let arr = d
        .clone()
        .into_array()
        .map_err(|actual| format!("`modifiers` must be an array of maps, got {actual}"))?;
    let mut out = Vec::with_capacity(arr.len());
    for (i, el) in arr.into_iter().enumerate() {
        let ty = el.type_name();
        let m = el
            .try_cast::<Map>()
            .ok_or_else(|| format!("`modifiers`[{i}] must be a map, got {ty}"))?;
        let condition = map_str(&m, "condition")
            .ok_or_else(|| format!("`modifiers`[{i}] requires a string `condition`"))?;
        let weight = map_f32(&m, "weight")
            .ok_or_else(|| format!("`modifiers`[{i}] requires a `weight` (`flt(\"…\")` or int)"))?;
        out.push(RawModifier {
            condition,
            threshold: map_f32(&m, "threshold"),
            weight,
        });
    }
    Ok(Some(out))
}

/// Read an `add_objective` `zero_gates` array — `[#{ condition, threshold? }]` — into
/// `Vec<RawZeroGate>`, the veto twin of [`map_modifiers`] (no `weight`). Same reuse,
/// same `flt`-or-int `threshold`, same discard-on-malformed policy.
fn map_zero_gates(spec: &Map) -> Result<Option<Vec<RawZeroGate>>, String> {
    let Some(d) = spec.get("zero_gates") else {
        return Ok(None);
    };
    let arr = d
        .clone()
        .into_array()
        .map_err(|actual| format!("`zero_gates` must be an array of maps, got {actual}"))?;
    let mut out = Vec::with_capacity(arr.len());
    for (i, el) in arr.into_iter().enumerate() {
        let ty = el.type_name();
        let m = el
            .try_cast::<Map>()
            .ok_or_else(|| format!("`zero_gates`[{i}] must be a map, got {ty}"))?;
        let condition = map_str(&m, "condition")
            .ok_or_else(|| format!("`zero_gates`[{i}] requires a string `condition`"))?;
        out.push(RawZeroGate {
            condition,
            threshold: map_f32(&m, "threshold"),
        });
    }
    Ok(Some(out))
}

/// Convert a `spawn_entity` `overrides` subtree (`#{ … }`) into the `toml::Value`
/// the declarative `overrides` field carries.
///
/// Every BARE numeric leaf renders as a toml FLOAT, NOT an integer: the whole
/// script API is `no_float`, so an author writes `range: 200` (an INT), but
/// `EntityConfig`'s numeric fields are overwhelmingly floats and a declarative
/// `overrides` authors them as floats — so `200` must become `200.0` to
/// deserialize into the same config `overrides = { range = 200.0 }` produces.
/// Recurses through arrays and nested maps (the `behaviour.doctrine`
/// array-of-maps, the radar string arrays). Pinned by the `range: 200 ≡
/// range = 200.0` structural-parity assertion.
///
/// A leaf that targets a genuine INTEGER `EntityConfig` field
/// (`repair.repair_team_count`, `volley_count`, …) is the one case the bare-int
/// default gets wrong: rendered as a float it would become `field = 3.0`, fail
/// to deserialize into an integer field, and `dispatch_spawn_entity` would drop
/// the WHOLE override (keeping the template) — see its `override_failures`
/// doc. Issue #1048 closed this with an explicit marker, [`IntLit`] /
/// `int(3)`, checked below: the same opt-in escape hatch [`RealLit`] /
/// `flt("…")` already is for the opposite rare case (a fractional VALUE a bare
/// int cannot express at all). A hand-maintained field-name → type table was
/// considered and rejected (see [`IntLit`]'s doc) — it would let this function
/// guess the right rendering for an UNMARKED leaf, but the table rots silently
/// the moment `EntityConfig` grows an integer field nobody remembers to add to
/// it, whereas a missing `int(…)` marker fails LOUDLY (a deserialize error,
/// now reported through `override_failures` rather than a bare warning).
fn dynamic_to_toml(value: &Dynamic) -> Result<toml::Value, String> {
    // Bool before int: the two are distinct `Dynamic` types, so the order is for
    // total coverage, not disambiguation.
    if let Ok(b) = value.as_bool() {
        return Ok(toml::Value::Boolean(b));
    }
    if let Some(int_lit) = value.clone().try_cast::<IntLit>() {
        // An `int(3)` marker (issue #1048): the `no_float`-safe INTEGER-target
        // leaf. Overrides the ambient bare-int-as-float default below, so it must
        // be checked first — a `value.as_int()` on a boxed custom type like this
        // one always fails anyway, but checking here keeps the two markers
        // textually adjacent to the bare-int branch they both modify.
        return Ok(toml::Value::Integer(int_lit.0));
    }
    if let Ok(i) = value.as_int() {
        return Ok(toml::Value::Float(i as f64));
    }
    if let Some(real) = value.clone().try_cast::<RealLit>() {
        // A `flt("0.9")` marker: the `no_float`-safe fractional leaf. Renders as the
        // SAME toml FLOAT the declarative `0.9` parses to (`f64::from_str` and toml's
        // float parse agree on canonical decimals), so a converted override's
        // `toml::Value` is byte-identical to its declarative twin's.
        return Ok(toml::Value::Float(real.0));
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
/// The full utility config is read here: `base_priority` (a `flt`-or-int `f32`),
/// plus the `modifiers` and `zero_gates` arrays via [`map_modifiers`] /
/// [`map_zero_gates`]. They are set on the `RawActionEntry` so the SHARED
/// `parse_utility_config` (run inside `parse_action_entry`) builds the byte-identical
/// `UtilityConfig` the declarative twin does — the scripted and TOML `add_objective`
/// are two front-ends over one parser, not two implementations kept in sync.
fn add_objective_action(spec: &Map) -> Result<TriggerAction, String> {
    let raw = RawActionEntry {
        kind: "add_objective".to_string(),
        id: map_str(spec, "id"),
        text: map_str(spec, "text"),
        text_params: map_text_params(spec, "text_params")?,
        mandatory: map_bool(spec, "mandatory"),
        targets: map_string_array(spec, "targets")?,
        target: map_str(spec, "target"),
        directive_kind: map_str(spec, "directive_kind"),
        directive_anchors: map_string_array(spec, "directive_anchors")?,
        directive_loop: map_bool(spec, "directive_loop"),
        directive_anchor: map_str(spec, "directive_anchor"),
        base_priority: map_f32(spec, "base_priority"),
        source: map_str(spec, "source"),
        modifiers: map_modifiers(spec)?,
        zero_gates: map_zero_gates(spec)?,
        command_stance: map_command_stance(spec)?,
        ..Default::default()
    };
    parse_action_entry(&raw)
}

/// Read an optional `command_stance` `#{ … }` map (issue #1110) into a
/// [`RawCommandStance`], so a scripted `add_objective` contributes an
/// objective-specific stance through the SAME `parse_command_stance` seam the
/// declarative TOML twin uses — one validator, not two.
///
/// `station`, `id` and `kind` are required (a stance with no id cannot be
/// selected, no kind cannot be resolved, and no station has nothing to lend to);
/// the remaining posture flags default exactly as `#[serde(default)]` does on the
/// declarative side. `Ok(None)` when absent; `Err` — discarding the whole call
/// (settled decision 10) — when present but malformed.
fn map_command_stance(spec: &Map) -> Result<Option<RawCommandStance>, String> {
    let Some(d) = spec.get("command_stance") else {
        return Ok(None);
    };
    let map = d
        .clone()
        .try_cast::<Map>()
        .ok_or_else(|| "`command_stance` must be a #{ … } map".to_string())?;
    let station = map_str(&map, "station")
        .ok_or_else(|| "`command_stance` requires a string `station`".to_string())?;
    let id =
        map_str(&map, "id").ok_or_else(|| "`command_stance` requires a string `id`".to_string())?;
    let kind_str = map_str(&map, "kind")
        .ok_or_else(|| "`command_stance` requires a string `kind`".to_string())?;
    let kind = parse_stance_kind(&kind_str)?;
    let stance = crate::ship::config::StationStanceConfig {
        id,
        label: map_str(&map, "label").unwrap_or_default(),
        kind,
        high_alert: map_bool(&map, "high_alert").unwrap_or(false),
        persist_behind_human: map_bool(&map, "persist_behind_human").unwrap_or(false),
        ai_engaged: map_bool(&map, "ai_engaged").unwrap_or(false),
    };
    Ok(Some(RawCommandStance { station, stance }))
}

/// Map a `command_stance` `kind` string to the [`StanceKind`] the declarative
/// serde path resolves the same token to. The three snake_case spellings match
/// `#[serde(rename_all = "snake_case")]` on the enum; an unknown one raises.
fn parse_stance_kind(s: &str) -> Result<crate::ship::config::StanceKind, String> {
    use crate::ship::config::StanceKind;
    match s {
        "standard" => Ok(StanceKind::Standard),
        "normal_alert_neutral" => Ok(StanceKind::NormalAlertNeutral),
        "high_alert_neutral" => Ok(StanceKind::HighAlertNeutral),
        other => Err(format!(
            "`command_stance.kind` must be one of standard, normal_alert_neutral, \
             high_alert_neutral; got '{other}'"
        )),
    }
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

/// Build a `TriggerAction::DestroyEntity` from a script entity name, reusing the
/// declarative `parse_action_entry` for the required-field check (issue #1033).
///
/// A bare string rather than a `#{ … }` map, matching `add_faction_enemy`: the
/// action carries exactly one field, and a map would invent an authoring shape the
/// declarative twin does not have. It still routes through `parse_action_entry`,
/// so "which field is required, and what does its absence say" has one owner —
/// unreachable from Rhai (the arity is the check) but true by construction rather
/// than by a comment claiming so.
///
/// `pub(super)` because the deferred twin — `ctx.schedule.in_seconds(n)
/// .destroy_entity(…)` — must buffer the byte-identical `TriggerAction`, and two
/// spellings of "build the destroy action" would be two chances to diverge.
pub(super) fn destroy_entity_action(entity: &str) -> Result<TriggerAction, String> {
    let raw = RawActionEntry {
        kind: "destroy_entity".to_string(),
        entity: Some(entity.to_string()),
        ..Default::default()
    };
    parse_action_entry(&raw)
}

/// Build an [`OpenCommsRequest`] from an `open_comms` script map.
///
/// ```rhai
/// ctx.effects.open_comms(#{
///     from: "axiom",                 // required: sender ref id -> name_to_uuid
///     node_fn: "hail_axiom",         // required: the root dialogue node fn
///     display_name: "Axiom Control", // optional
///     thread_id: "aphelion",         // optional: joins an existing thread
///     urgent: true,                  // optional, default false
/// });
/// ```
///
/// `node_fn`, not `fn`: `fn` is a Rhai KEYWORD, and the map-literal parser
/// accepts only an identifier or a string as a property name — so `#{ fn: … }`
/// is a parse error rather than a key this could read (pinned by
/// `fn_is_not_usable_as_a_map_key`). One spelling, so there is nothing to keep in
/// step.
///
/// A missing required key raises, discarding the whole call (settled decision
/// 10), exactly as a malformed `add_objective` / `spawn_entity` map does. Unknown
/// keys are ignored, matching those two.
fn open_comms_request(spec: &Map) -> Result<OpenCommsRequest, String> {
    let from = map_str(spec, "from")
        .ok_or_else(|| "open_comms requires a string `from` (the sender ref id)".to_string())?;
    let root_fn = map_str(spec, "node_fn").ok_or_else(|| {
        "open_comms requires a string `node_fn` (the root dialogue node fn)".to_string()
    })?;
    Ok(OpenCommsRequest {
        from,
        root_fn,
        display_name: map_str(spec, "display_name"),
        thread_id: map_str(spec, "thread_id"),
        urgent: map_bool(spec, "urgent").unwrap_or(false),
        // Stamped by `EffectSink::take_opens` at the host boundary.
        script_path: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// Compile and call, returning BOTH drained buffers: the ordered effects and
    /// the comms opens (stamped with the unit path, as the host does).
    fn run_with_opens(source: &str, fn_name: &str) -> (Vec<BufferedEffect>, Vec<OpenCommsRequest>) {
        let engine = runtime_engine();
        let ast = engine.compile(source).expect("compiles");
        let sink = EffectSink::new();
        let ctx = make_ctx(&sink);
        let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).expect("calls");
        (sink.take(), sink.take_opens("t.rhai"))
    }

    /// The single `TriggerAction` serde produces for one action table, built
    /// independently of this module's map extraction — the independent source of
    /// truth for the M6 structural-parity assertions.
    ///
    /// It went through `parse_world` and a `[[trigger]]` wrapper until issue #985
    /// deleted that container. The TABLE is what mattered and the table survives:
    /// `RawActionEntry` is the same struct the script host populates, and
    /// `parse_action_entry` the same shared rule, so this still reaches the
    /// parity target by the route that is not the one under test.
    fn toml_action(action_body: &str) -> TriggerAction {
        let raw: crate::world::config::RawActionEntry =
            toml::from_str(action_body).expect("the action table parses");
        crate::world::config::parse_action_entry(&raw).expect("the action parses")
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
                    phase: crate::core::messages::GamePhase::GameOver,
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
                    outcome: Some(crate::core::balance::Outcome::Victory),
                },
                ActionCmd::SetNextState {
                    phase: crate::core::messages::GamePhase::GameOver,
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
                outcome: Some(crate::core::balance::Outcome::Defeat),
            }
        );
    }

    /// A scripted typo raises (discarding the call's effects, settled decision
    /// 10) exactly as a bad declarative `outcome = "…"` fails the world load —
    /// both route through `crate::core::balance::Outcome::parse`.
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

    // ── destroy_entity (issue #1033) ─────────────────────────────────────────

    /// `destroy_entity(name)` buffers the DECLARATIVE `DestroyEntity` — the entity
    /// NAME, unresolved — identical to the TOML action before UUID resolution, the
    /// same assertion `add_faction_enemy_matches_toml` makes about its twin.
    #[test]
    fn destroy_entity_matches_toml() {
        let effs = run_buffered(
            r#"fn f(ctx) { ctx.effects.destroy_entity("skyhook"); }"#,
            "f",
        );
        let toml = toml_action("type = \"destroy_entity\"\nentity = \"skyhook\"");
        assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
        assert_eq!(
            effs,
            vec![BufferedEffect::Action(TriggerAction::DestroyEntity {
                entity: "skyhook".to_string(),
            })]
        );
    }

    /// The load-bearing shape claim, stated where it can fail: a destroy buffers as
    /// an `Action`, NOT a `Cmd`.
    ///
    /// This is the whole architecture in one assertion. A `Cmd` is applied
    /// directly and would despawn the entity while chaining nothing; an `Action` is
    /// resolved through `dispatch_destroy_entity`, which pushes
    /// `WorldEvent::Destroyed` onto `new_events` beside the command — and that
    /// event is what makes `on_destroyed` / `on_all_destroyed` fire off a scripted
    /// removal. A refactor that "simplified" this into a resolved command would
    /// pass every other test in this module and silently break chaining.
    #[test]
    fn destroy_entity_buffers_an_action_not_a_resolved_command() {
        let effs = run_buffered(
            r#"fn f(ctx) { ctx.effects.destroy_entity("skyhook"); }"#,
            "f",
        );
        assert!(
            matches!(effs.as_slice(), [BufferedEffect::Action(_)]),
            "a destroy must defer name resolution to dispatch — a resolved Cmd \
             would despawn without chaining a Destroyed event, got {effs:?}"
        );
    }

    /// A destroy keeps its authored position among the other effects, so a handler
    /// that raises a flag, destroys a structure and completes an objective applies
    /// them in that order.
    #[test]
    fn destroy_entity_interleaves_in_authored_order() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.complete_objective("first");
                ctx.effects.destroy_entity("skyhook");
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
                BufferedEffect::Action(TriggerAction::DestroyEntity {
                    entity: "skyhook".to_string(),
                }),
                BufferedEffect::Cmd(ActionCmd::FailObjective {
                    id: "last".to_string()
                }),
            ]
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

    /// `add_objective` now READS `modifiers`/`zero_gates` (the utility-config
    /// milestone): the script arrays-of-maps, with `flt("…")` fractional thresholds
    /// and weights, build the identical `TriggerAction::AddObjective` — same
    /// `UtilityConfig` — the declarative twin parses through the shared
    /// `parse_utility_config`. This is the parity the FRACTIONAL test-infra worlds
    /// ride on: `no_float` Rhai could not author `weight = 1.25` before `flt`.
    #[test]
    fn add_objective_reads_modifiers_and_zero_gates_matching_toml() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "obj-utility",
                    text: "world.obj.text",
                    base_priority: flt("50.5"),
                    modifiers: [
                        #{ condition: "enemy_near", threshold: flt("0.5"), weight: flt("2.0") },
                        #{ condition: "low_hull", weight: flt("1.25") },
                    ],
                    zero_gates: [
                        #{ condition: "shields_down" },
                        #{ condition: "power_low", threshold: flt("0.2") },
                    ],
                });
            }"#,
            "f",
        );
        let toml = toml_action(
            "type = \"add_objective\"\n\
             id = \"obj-utility\"\n\
             text = \"world.obj.text\"\n\
             base_priority = 50.5\n\
             modifiers = [{ condition = \"enemy_near\", threshold = 0.5, weight = 2.0 }, { condition = \"low_hull\", weight = 1.25 }]\n\
             zero_gates = [{ condition = \"shields_down\" }, { condition = \"power_low\", threshold = 0.2 }]",
        );
        assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
    }

    /// A `modifier` missing its required `weight` RAISES (discarding the call,
    /// settled decision 10) — the same loud failure a declarative modifier without a
    /// `weight` gives, rather than a silently degraded `UtilityConfig`.
    #[test]
    fn add_objective_rejects_a_modifier_without_a_weight() {
        let err = run_result(
            r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "o", text: "t",
                    modifiers: [#{ condition: "enemy_near" }],
                });
            }"#,
            "f",
        )
        .expect_err("a weightless modifier must raise");
        assert!(err.to_string().contains("weight"), "{err}");
    }

    /// Issue #1110: a scripted `command_stance` map and the declarative
    /// `command_stance` table build the BYTE-IDENTICAL `TriggerAction::AddObjective`
    /// — same target Station id, same `StationStanceConfig` — because both run
    /// through the one `parse_command_stance` seam. Two front-ends, one parser.
    #[test]
    fn add_objective_command_stance_matches_toml() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "obj-escort",
                    text: "world.obj.text",
                    command_stance: #{
                        station: "tactical",
                        id: "objective-escort",
                        kind: "standard",
                        high_alert: true,
                        persist_behind_human: true,
                    },
                });
            }"#,
            "f",
        );
        let toml = toml_action(
            "type = \"add_objective\"\n\
             id = \"obj-escort\"\n\
             text = \"world.obj.text\"\n\
             command_stance = { station = \"tactical\", id = \"objective-escort\", kind = \"standard\", high_alert = true, persist_behind_human = true }",
        );
        assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
    }

    /// A `command_stance` with no `station` RAISES (discarding the call, settled
    /// decision 10): there is no target Station to lend the stance to. The same
    /// loud failure the declarative `parse_command_stance` gives a blank station.
    #[test]
    fn add_objective_rejects_a_command_stance_without_a_station() {
        let err = run_result(
            r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "o", text: "t",
                    command_stance: #{ id: "x", kind: "standard" },
                });
            }"#,
            "f",
        )
        .expect_err("a stationless command_stance must raise");
        assert!(err.to_string().contains("station"), "{err}");
    }

    // ── Feature A: the `flt("…")` fractional-data marker (no_float-safe) ───────

    /// The sharpest `flt` parity: a `flt("0.9")` override leaf must produce the
    /// IDENTICAL `toml::Value` the declarative `target_speed = 0.9` carries — the
    /// byte-identity a converted FRACTIONAL world depends on (`f64::from_str` ≡ toml's
    /// float parse for canonical decimals). This is why a fractional constant can be
    /// transported through `no_float` script as opaque data with no determinism loss.
    #[test]
    fn flt_override_leaf_matches_declarative_float() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        helm_console: #{
                            target_speed: flt("0.9"),
                        },
                    },
                });
            }"#,
            "f",
        );
        let toml = toml_action(
            "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship.toml\"\n\
             name = \"x\"\n\
             position = [0.0, 0.0, 0.0]\n\
             overrides = { helm_console = { target_speed = 0.9 } }",
        );
        assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

        // Pin the conversion concretely: the `flt("0.9")` leaf is the identical toml
        // FLOAT `0.9` — so a regression in the `RealLit` branch is unmistakable.
        let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
            panic!("expected a spawn action, got {:?}", effs[0]);
        };
        let speed = overrides
            .as_ref()
            .and_then(|o| o.get("helm_console"))
            .and_then(|h| h.get("target_speed"))
            .expect("override target_speed present");
        assert_eq!(speed, &toml::Value::Float(0.9));
    }

    /// The doctrine-ARRAY shape the converted worlds actually author (issue #984
    /// review advisory): a `flt` leaf nested Array→Map deep —
    /// `overrides.behaviour.doctrine = [ { … target_speed = 0.9 … } ]` — must
    /// still equal the declarative twin. `flt_override_leaf_matches_declarative_float`
    /// pins Map→Map recursion; this pins the Array→Map→RealLit path the probe/duel
    /// spawn overrides (and combat_test's waves) ride through `dynamic_to_toml`.
    #[test]
    fn flt_inside_a_doctrine_array_override_matches_declarative() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        behaviour: #{
                            doctrine: [
                                #{
                                    id: "kill",
                                    base_priority: 80,
                                    target_speed: flt("0.9"),
                                    maintain_range: 25,
                                },
                            ],
                        },
                    },
                });
            }"#,
            "f",
        );
        let toml = toml_action(
            "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship.toml\"\n\
             name = \"x\"\n\
             position = [0.0, 0.0, 0.0]\n\
             overrides = { behaviour = { doctrine = [ { id = \"kill\", base_priority = 80.0, target_speed = 0.9, maintain_range = 25.0 } ] } }",
        );
        assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

        // Pin the nested leaf concretely, as the flat-leaf test does.
        let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
            panic!("expected a spawn action, got {:?}", effs[0]);
        };
        let speed = overrides
            .as_ref()
            .and_then(|o| o.get("behaviour"))
            .and_then(|b| b.get("doctrine"))
            .and_then(|d| d.get(0))
            .and_then(|row| row.get("target_speed"))
            .expect("doctrine[0].target_speed present");
        assert_eq!(speed, &toml::Value::Float(0.9));
    }

    /// An unparseable `flt("…")` RAISES (discarding the call, settled decision 10),
    /// exactly as a malformed declarative float fails the world load. The parse
    /// happens once, at map-build time, so the raise pre-empts the effect entirely.
    #[test]
    fn flt_rejects_an_unparseable_string() {
        let err = run_result(
            r#"fn f(ctx) { ctx.effects.add_objective(#{ id: "o", text: "t", base_priority: flt("xyz") }); }"#,
            "f",
        )
        .expect_err("an unparseable flt must raise");
        assert!(err.to_string().contains("flt"), "{err}");
    }

    // ── Feature B: the `int(…)` integer-target marker (issue #1048) ───────────

    /// The mirror of `flt_override_leaf_matches_declarative_float`: an `int(3)`
    /// override leaf must produce the IDENTICAL `toml::Value` — a toml INTEGER,
    /// not the ambient float default — the declarative
    /// `repair.repair_team_count = 3` carries, AND that value must actually
    /// deserialize into the genuine `u32` `EntityConfig` field it targets
    /// (`entities::config::RepairConfig::repair_team_count`). That last step is
    /// the whole point of #1048: before the marker existed, this same leaf
    /// rendered as a toml FLOAT and could not deserialize into an integer field
    /// at all.
    #[test]
    fn int_override_leaf_matches_declarative_integer_and_deserializes() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        repair: #{
                            repair_team_count: int(3),
                        },
                    },
                });
            }"#,
            "f",
        );
        let toml = toml_action(
            "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship.toml\"\n\
             name = \"x\"\n\
             position = [0.0, 0.0, 0.0]\n\
             overrides = { repair = { repair_team_count = 3 } }",
        );
        assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

        // Pin the conversion concretely: an `int(3)` leaf is the toml INTEGER
        // `3`, not the ambient toml FLOAT a bare `3` would render as.
        let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
            panic!("expected a spawn action, got {:?}", effs[0]);
        };
        let count = overrides
            .as_ref()
            .and_then(|o| o.get("repair"))
            .and_then(|r| r.get("repair_team_count"))
            .expect("override repair_team_count present");
        assert_eq!(count, &toml::Value::Integer(3));

        // The crux of #1048: the override actually deserializes into the
        // genuine integer field. Before the fix this `try_into` would fail
        // (`invalid type: floating point`3`, expected u32`).
        let repair: crate::entities::config::RepairConfig = overrides
            .as_ref()
            .and_then(|o| o.get("repair"))
            .cloned()
            .expect("repair override present")
            .try_into()
            .expect("an int(3) leaf must deserialize into RepairConfig's genuine u32 field");
        assert_eq!(repair.repair_team_count, 3);
    }

    /// The control, mirroring `spawn_entity_override_without_a_tombstone_still_applies`'s
    /// role for the tombstone test: an UNMARKED int on the SAME integer-target
    /// field still renders as the ambient toml FLOAT, exactly as before #1048.
    /// The marker is opt-in, not a new schema-aware default — pinned here so a
    /// regression that made `dynamic_to_toml` "smart" about `repair_team_count`
    /// specifically (the hand-maintained field table issue #1048 explicitly
    /// rejected) would fail this test, not silently pass it.
    #[test]
    fn a_bare_int_on_the_same_integer_field_still_renders_as_the_ambient_float() {
        let effs = run_buffered(
            r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        repair: #{
                            repair_team_count: 3,
                        },
                    },
                });
            }"#,
            "f",
        );
        let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
            panic!("expected a spawn action, got {:?}", effs[0]);
        };
        let count = overrides
            .as_ref()
            .and_then(|o| o.get("repair"))
            .and_then(|r| r.get("repair_team_count"))
            .expect("override repair_team_count present");
        assert_eq!(
            count,
            &toml::Value::Float(3.0),
            "an unmarked int must still render as the ambient float default"
        );
        // And, unmarked, it does NOT deserialize into the integer field — the
        // authoring mistake `int(…)` exists to let an author avoid.
        let repair_result: Result<crate::entities::config::RepairConfig, _> = overrides
            .as_ref()
            .and_then(|o| o.get("repair"))
            .cloned()
            .expect("repair override present")
            .try_into();
        assert!(
            repair_result.is_err(),
            "a float leaf must NOT silently coerce into the integer field"
        );
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

    /// A handler may DELEGATE to a helper fn in the same unit, and the helper's
    /// effects land in the CALLER's buffer, in authored order (issue #984, M6
    /// duel harness).
    ///
    /// This is what lets a world author one parameterised spawn body and call it
    /// from N one-line handlers instead of repeating the body N times: `ctx` is
    /// copied into the callee (Rhai passes maps by value), but the `effects`
    /// handle inside it is an [`EffectSink`] — `Arc<Mutex<_>>` — so every copy
    /// pushes onto the ONE buffer the host drains. `duel.toml`'s generated slot
    /// drivers ride on this; nothing else shipped did, so it is pinned here.
    #[test]
    fn a_helper_fn_shares_the_callers_effect_buffer() {
        let effs = run_buffered(
            r#"
            fn spawn_slot(ctx, name, template) {
                ctx.effects.spawn_entity(#{
                    template_path: template,
                    name: name,
                    position: [0, 0, 0],
                });
            }

            fn f(ctx) {
                ctx.effects.complete_objective("before");
                spawn_slot(ctx, "side_b_1", "assets/entities/ship.toml");
                ctx.effects.complete_objective("after");
            }"#,
            "f",
        );
        let spawned = toml_action(
            "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship.toml\"\n\
             name = \"side_b_1\"\n\
             position = [0.0, 0.0, 0.0]",
        );
        assert_eq!(
            effs,
            vec![
                BufferedEffect::Cmd(ActionCmd::CompleteObjective {
                    id: "before".to_string()
                }),
                BufferedEffect::Action(spawned),
                BufferedEffect::Cmd(ActionCmd::CompleteObjective {
                    id: "after".to_string()
                }),
            ],
            "a helper fn's effects must interleave in the caller's buffer"
        );
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

    // ── open_comms (issue #984) ──────────────────────────────────────────────

    /// The full authored form buffers one request with every optional carried,
    /// and the drain stamps the running unit's path (the script never names it).
    #[test]
    fn open_comms_buffers_the_request_with_its_metadata() {
        let (effs, opens) = run_with_opens(
            r#"fn f(ctx) {
                ctx.effects.open_comms(#{
                    from: "axiom",
                    node_fn: "hail_axiom",
                    display_name: "Axiom Control",
                    thread_id: "aphelion",
                    urgent: true,
                });
            }"#,
            "f",
        );
        assert!(
            effs.is_empty(),
            "an open is not an ActionCmd/TriggerAction effect"
        );
        assert_eq!(
            opens,
            vec![OpenCommsRequest {
                from: "axiom".to_string(),
                root_fn: "hail_axiom".to_string(),
                display_name: Some("Axiom Control".to_string()),
                thread_id: Some("aphelion".to_string()),
                urgent: true,
                script_path: "t.rhai".to_string(),
            }]
        );
    }

    /// Only `from` and `node_fn` are required; the rest default the way the
    /// declarative `[[comms]]` template's optional fields do.
    #[test]
    fn open_comms_defaults_every_optional_key() {
        let (_e, opens) = run_with_opens(
            r#"fn f(ctx) { ctx.effects.open_comms(#{ from: "axiom", node_fn: "hail" }); }"#,
            "f",
        );
        assert_eq!(
            opens,
            vec![OpenCommsRequest {
                from: "axiom".to_string(),
                root_fn: "hail".to_string(),
                display_name: None,
                thread_id: None,
                urgent: false,
                script_path: "t.rhai".to_string(),
            }]
        );
    }

    /// A missing required key raises, discarding the whole call (settled decision
    /// 10) — including the effects authored before it.
    #[test]
    fn open_comms_raises_on_a_missing_required_key() {
        for (spec, wanted) in [
            (r#"#{ node_fn: "hail" }"#, "from"),
            (r#"#{ from: "axiom" }"#, "node_fn"),
        ] {
            let src = format!(
                "fn f(ctx) {{ ctx.effects.complete_objective(\"before\"); \
                 ctx.effects.open_comms({spec}); }}"
            );
            let err = run_result(&src, "f").expect_err("a missing required key must raise");
            assert!(
                err.to_string().contains(wanted),
                "the error should name `{wanted}`: {err}"
            );
        }
    }

    /// `node_fn`, not `fn`: `fn` is a Rhai keyword and the map-literal parser
    /// takes only an identifier or a string as a property name, so `#{ fn: … }`
    /// never even compiles. Pinned so the naming choice is evidence, not taste.
    #[test]
    fn fn_is_not_usable_as_a_map_key() {
        let engine = runtime_engine();
        assert!(
            engine
                .compile(r#"fn f(ctx) { ctx.effects.open_comms(#{ from: "a", fn: "b" }); }"#)
                .is_err(),
            "`fn` as a bare map key must be a parse error (hence `node_fn`)"
        );
    }

    /// Opens ride a SECOND buffer, so they cannot perturb the authored order of
    /// the `Cmd`/`Action` sequence the applier dispatches — the one ordering
    /// guarantee flag writes and name-resolving effects depend on.
    #[test]
    fn open_comms_does_not_disturb_authored_effect_order() {
        let (effs, opens) = run_with_opens(
            r#"fn f(ctx) {
                ctx.effects.complete_objective("first");
                ctx.effects.open_comms(#{ from: "axiom", node_fn: "hail" });
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
            ],
            "the ordered buffer must read exactly as it does without the open"
        );
        assert_eq!(opens.len(), 1);
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
            template_loader: &crate::entities::loader::WasmTemplateLoader,
        };
        dispatch_action(action, &ctx).commands
    }
}
