//! The Rhai comms dialogue front-end (issue #982, milestone M4).
//!
//! A scripted comms thread authors its dialogue tree as ordinary named
//! functions instead of the nested `[[comms.response.follow_up…]]` TOML tables
//! (and their eight-segment localization keys). One function is one dialogue
//! node:
//!
//! ```rhai
//! // The root node: `[[comms]] script = "hail_axiom"` names this fn.
//! fn hail_axiom(ctx) {
//!     #{ message: "Axiom Station, go ahead.", responses: [
//!         #{ text: "Acknowledge", on_pick: "on_ack" },
//!         #{ text: "Decline",     on_pick: "on_decline", important: true },
//!     ] }
//! }
//!
//! // A response's `on_pick` names the next node fn. It runs effects and then
//! // returns the FOLLOW-UP node (or `()` for a terminal response).
//! fn on_ack(ctx) {
//!     ctx.effects.complete_objective("reach_axiom");
//!     #{ message: "Docking clamps released.", responses: [] }   // a follow-up
//! }
//! fn on_decline(ctx) {
//!     ctx.effects.fail_objective("reach_axiom");                // terminal (returns ())
//! }
//! ```
//!
//! Entering a node and picking a response are the **same operation** — call a
//! fn, collect the effects it buffered, and read the `#{message, responses}` map
//! it returned. A root node fn buffers no effects and returns the node to show; a
//! response `on_pick` fn buffers the picked response's effects and returns the
//! follow-up node (or `()`). Both go through [`enter_node`].
//!
//! # One evaluator, two front-ends (settled decision 8)
//!
//! Response actions route through the **existing**
//! [`ActionCmd`](crate::world::dispatch::ActionCmd) boundary: an `on_pick` fn's
//! `ctx.effects.*` / `ctx.flags.*` calls push the *same* `ActionCmd`s the
//! declarative `[[comms.response.action]]` array produces via
//! [`dispatch_action`](crate::world::dispatch::dispatch_action). A scripted
//! thread and its TOML equivalent therefore emit an identical `ActionCmd`
//! sequence for the same player choices — the migration guard the tests below
//! assert (mirroring the M2 trigger parity test).
//!
//! # The one front-end (M4 dormant, M6 wired, M7 alone)
//!
//! This landed dormant in M4 — front-end, validation and parity test — beside a
//! live declarative path, the way M2 landed scripted triggers without touching
//! the trigger evaluator. M6 (#984) wired it into `handle_respond_to_message`
//! beside the declarative arm; M7 (#985) deleted that arm and the `[[comms]]`
//! parser behind it. A thread is opened by a script calling
//! `ctx.effects.open_comms(#{...})`, materialised by
//! [`open_scripted_comms_threads`](crate::comms::scripted::open_scripted_comms_threads),
//! and every node fn name it reaches is cross-reference validated at load
//! ([`validate_on_pick_fns`](super::validate::validate_on_pick_fns)).

use rhai::{Dynamic, Map};

use crate::comms::content::{CommsDialogueNode, CommsResponse};
use crate::world::flags::FlagStore;
use crate::world::script::engine::RuntimeHost;
use crate::world::script::schedule::{CallEffects, SchedClock, TickBudget};

/// One response option in a scripted dialogue node — the script analogue of a
/// [`CommsResponse`](crate::comms::content::CommsResponse).
///
/// `on_pick` names the fn to run when the player picks this response; that fn
/// supplies the response's effects and returns the follow-up node (or `()`), so
/// the branching lives in fn-to-fn references rather than nested TOML tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptDialogueResponse {
    /// Player-facing button text (authored inline, exactly like
    /// [`CommsResponse::text`](crate::comms::content::CommsResponse)).
    pub text: String,
    /// Name of the fn to call when this response is picked.
    pub on_pick: String,
    /// Whether the client confirms before submitting this response (the script
    /// analogue of [`CommsResponse::important`](crate::comms::content::CommsResponse)).
    /// Defaults to `false` when the response map omits it.
    pub important: bool,
}

/// A scripted dialogue node materialized from a node fn's return map — the
/// script analogue of a [`CommsDialogueNode`](crate::comms::content::CommsDialogueNode).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptDialogueNode {
    /// The message body to inject into the inbox.
    pub message: String,
    /// The response options offered on this node.
    pub responses: Vec<ScriptDialogueResponse>,
}

/// Read a string field out of a Rhai map, cloning it to an owned `String`.
fn take_string(map: &Map, key: &str) -> Option<String> {
    map.get(key).and_then(|d| d.clone().into_string().ok())
}

/// Materialize the `Dynamic` a dialogue-node or `on_pick` fn returned.
///
/// A `#{message, responses}` map becomes `Some(node)`; a `()` return (a terminal
/// response with no follow-up) becomes `None`. Any other shape — a script that
/// compiled but returned the wrong thing (a bare string, a number, a map missing
/// `message`) — is an `Err` with an authoring-facing message. This is a shape
/// error, distinct from a script *error* (a raise or a tripped limit), which
/// [`RuntimeHost::call_dialogue`] already handles under the failure policy.
pub fn read_dialogue_node(value: Dynamic) -> Result<Option<ScriptDialogueNode>, String> {
    if value.is_unit() {
        return Ok(None);
    }
    let map = value.try_cast::<Map>().ok_or_else(|| {
        "a dialogue node fn must return a #{message, responses} map or () for a terminal response"
            .to_string()
    })?;
    let message = take_string(&map, "message")
        .ok_or_else(|| "dialogue node map is missing a string `message` field".to_string())?;
    let responses = match map.get("responses") {
        None => Vec::new(),
        Some(d) => {
            let arr = d.clone().into_array().map_err(|actual| {
                format!("dialogue node `responses` must be an array, got {actual}")
            })?;
            let mut out = Vec::with_capacity(arr.len());
            for (i, r) in arr.into_iter().enumerate() {
                out.push(read_response(r, i)?);
            }
            out
        }
    };
    Ok(Some(ScriptDialogueNode { message, responses }))
}

/// Materialize one entry of a node's `responses` array.
fn read_response(value: Dynamic, index: usize) -> Result<ScriptDialogueResponse, String> {
    let map = value
        .try_cast::<Map>()
        .ok_or_else(|| format!("dialogue response[{index}] must be a #{{text, on_pick}} map"))?;
    let text = take_string(&map, "text")
        .ok_or_else(|| format!("dialogue response[{index}] is missing a string `text` field"))?;
    let on_pick = take_string(&map, "on_pick")
        .ok_or_else(|| format!("dialogue response[{index}] is missing a string `on_pick` field"))?;
    // `important` is optional and defaults to false, matching the TOML front-end.
    let important = map
        .get("important")
        .and_then(|d| d.as_bool().ok())
        .unwrap_or(false);
    Ok(ScriptDialogueResponse {
        text,
        on_pick,
        important,
    })
}

/// Project a materialized script node onto the wire dialogue shape, returning
/// the [`CommsDialogueNode`] to show and the parallel `on_pick` fn names
/// (issue #984).
///
/// The ONE place a script node meets the comms wire vocabulary, and pure —
/// no Bevy, no host, no clock (AGENTS.md rule 10) — so both the open path
/// (`comms::scripted::open_scripted_comms_threads`) and the response path
/// (`console::comms::server::handle_respond_to_message`) project identically.
///
/// The projected node carries `actions: []` and `follow_up: None` **by
/// construction**: a scripted response's effects and its follow-up both come
/// from calling `on_pick`, never from the node. That is what keeps every
/// existing reader of `ActiveDialogue::current_node` — the wire projection
/// ([`response_views`](crate::comms::content::response_views)), the AI
/// response policy's `responses.len()` / `.important` inputs, the handler's
/// bounds check — working unchanged on a scripted thread. `speaker` is `None`
/// for the same reason a scripted node has no `trigger`: who is calling is
/// metadata on the OPEN (`OpenCommsRequest::display_name`), not on the node.
///
/// The returned `Vec<String>` is index-parallel to the node's `responses`, so
/// the index a player submits addresses both the button they pressed and the fn
/// that answers it.
pub fn project_node(node: &ScriptDialogueNode) -> (CommsDialogueNode, Vec<String>) {
    let mut responses = Vec::with_capacity(node.responses.len());
    let mut on_pick = Vec::with_capacity(node.responses.len());
    for r in &node.responses {
        responses.push(CommsResponse {
            text: r.text.clone(),
            important: r.important,
        });
        on_pick.push(r.on_pick.clone());
    }
    (
        CommsDialogueNode {
            body: node.message.clone(),
            responses,
        },
        on_pick,
    )
}

/// Why [`enter_node`] produced no node — the three ways short of "the fn ran and
/// returned a well-shaped node or `()`" (issue #984).
///
/// Distinct variants because the consumers owe the player different things: a
/// pick that produced nothing must be REFUSED visibly (the control flashes red)
/// rather than recorded as though the thread ended, and the log line has to name
/// which of these happened or an authoring typo reads as a budget problem.
#[derive(Debug)]
pub enum EnterError {
    /// The fn RAN — buffering effects and flag writes, which are handed back
    /// here — and then returned a value that is neither a `#{message, responses}`
    /// map nor `()`.
    ///
    /// The effects come back because the call SUCCEEDED. Settled decision 10
    /// discards a call's buffers whole on a script ERROR, and the reason it can:
    /// the buffers were never drained, so nothing was half-applied. A shape error
    /// is the other side of that line — the fn completed, its buffers drained,
    /// and only the *return value* is malformed. Dropping a completed
    /// `complete_objective` because the author also mistyped the follow-up map
    /// would silently un-apply work the script really did.
    Shape {
        /// Everything the call produced before its malformed return.
        ///
        /// Boxed so this variant does not make `enter_node`'s whole `Result`
        /// large for every caller on the success path (clippy's
        /// `result_large_err`). `CallEffects` is five `Vec`s and grows a field
        /// each time script gains a new kind of buffered work — it grew a fifth
        /// for `deadline_changes` in issue #1024 — so the box is what stops that
        /// growth being paid by the path that never errors.
        effects: Box<CallEffects>,
        /// The authoring-facing shape complaint.
        message: String,
    },
    /// `fn_name` is not defined in this unit, so the call was never ATTEMPTED.
    ///
    /// An unresolvable `on_pick` reaches here rather than the host's failure
    /// policy (which would panic in dev on the resulting `CallError` and, in
    /// release, return an empty result the caller could not tell from a terminal
    /// response). The load-time lint
    /// ([`validate_on_pick_fns`](super::validate::validate_on_pick_fns)) is what
    /// stops an authored typo reaching here at all; this is the runtime backstop
    /// for the dynamically-built names that lint cannot see.
    Unresolved,
    /// The call was attempted but produced nothing: the tick's [`TickBudget`]
    /// refused it, or it raised and settled decision 10 discarded its buffers
    /// whole.
    Refused,
}

impl std::fmt::Display for EnterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnterError::Shape { message, .. } => write!(f, "{message}"),
            EnterError::Unresolved => write!(f, "names no function defined in this script unit"),
            EnterError::Refused => write!(
                f,
                "did not run: the tick's script budget refused it, or it raised"
            ),
        }
    }
}

/// Enter a dialogue node: run the fn named `fn_name` and materialize both
/// everything it produced ([`CallEffects`] — immediate effects in authored
/// order, delayed effects, deferred callbacks) and the node it returned.
///
/// This is the single operation behind BOTH "show the root node" (call the root
/// fn; it buffers no effects and returns the node to display) and "pick response
/// *i*" (call `responses[i].on_pick`; it buffers that response's effects and
/// returns the follow-up node, or `None` for a terminal response). The returned
/// `commands` are exactly what the declarative TOML path would dispatch for the
/// same choice — see the module docs.
///
/// `budget` is the caller's per-tick [`TickBudget`] (the live path threads the
/// tick's shared one, so a dialogue call counts against the same aggregate caps
/// every other script call does) and `clock` stamps any deferred work the fn
/// scheduled — a delayed comms reply is authored as
/// `ctx.schedule.after(n, |ctx| …)`, so both are load-bearing rather than
/// ceremonial.
///
/// `Ok` means the fn ran and returned a node (`Some`) or ended the thread
/// (`None`). Every other outcome is an [`EnterError`] naming which one — see its
/// docs for why the three are not collapsed, and why
/// [`EnterError::Shape`] still carries the call's effects.
pub fn enter_node(
    host: &RuntimeHost,
    budget: &mut TickBudget,
    clock: &SchedClock,
    ast: &rhai::AST,
    path: &str,
    fn_name: &str,
    base_flags: &FlagStore,
    base_deadlines: &crate::world::deadlines::DeadlineTable,
    base_commitments: &crate::world::commitments::CommitmentLedger,
) -> Result<(CallEffects, Option<ScriptDialogueNode>), EnterError> {
    // Resolve the name against the unit BEFORE calling. `call_fn` reports a
    // missing function as an ordinary `CallError`, which the host's failure
    // policy turns into a dev panic mid-mission and a release no-op — neither of
    // which a caller can distinguish from a terminal response. A name that
    // resolves to nothing is authoring data being wrong, not the script failing,
    // so it is answered here as a refusal the player can see.
    if !ast.iter_functions().any(|f| f.name == fn_name) {
        return Err(EnterError::Unresolved);
    }
    let Some((effects, value)) = host.call_dialogue(
        budget,
        clock,
        ast,
        path,
        fn_name,
        base_flags,
        base_deadlines,
        base_commitments,
        Map::new(),
    ) else {
        return Err(EnterError::Refused);
    };
    match read_dialogue_node(value) {
        Ok(node) => Ok((effects, node)),
        Err(message) => Err(EnterError::Shape {
            effects: Box::new(effects),
            message,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::dispatch::ActionCmd;
    use crate::world::script::effects::BufferedEffect;
    use crate::world::script::engine::RuntimeHost;
    use crate::world::script::load::compile_scripts;
    use vellum_script::ScriptSource;

    const PATH: &str = "w.toml#script.axiom";

    /// Enter a node with a fresh budget and the zero clock — the unit-test shape.
    /// The live path (the M7 collapse) threads the tick's shared budget and its
    /// real clock instead; nothing here schedules deferred work.
    fn enter(
        host: &RuntimeHost,
        ast: &rhai::AST,
        fn_name: &str,
        flags: &FlagStore,
    ) -> Result<(CallEffects, Option<ScriptDialogueNode>), EnterError> {
        let mut budget = TickBudget::new();
        enter_node(
            host,
            &mut budget,
            &SchedClock::ZERO,
            ast,
            PATH,
            fn_name,
            flags,
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
        )
    }

    /// The `ActionCmd`s a dialogue call produced, for comparison against the
    /// declarative dispatch. Panics on a name-resolving [`BufferedEffect::Action`]:
    /// these fixtures author only resolved verbs, so an `Action` here would mean
    /// the parity comparison had silently changed shape.
    fn cmds(effects: &CallEffects) -> Vec<ActionCmd> {
        effects
            .commands
            .iter()
            .map(|e| match e {
                BufferedEffect::Cmd(cmd) => cmd.clone(),
                BufferedEffect::Action(action) => {
                    panic!("parity fixture buffered a name-resolving action: {action:?}")
                }
            })
            .collect()
    }

    /// Compile one inline script unit and return `(compiled asts keyed by PATH)`.
    fn compile(source: &str) -> rhai::AST {
        let compiled = compile_scripts(&[ScriptSource {
            path: PATH.to_string(),
            source: source.to_string(),
        }]);
        assert!(
            compiled.findings.is_empty(),
            "unexpected findings: {:?}",
            compiled.findings
        );
        compiled.asts.get(PATH).expect("compiled ast").clone()
    }

    // ── read_dialogue_node materialization ────────────────────────────────────

    #[test]
    fn a_node_fn_returns_a_message_and_responses() {
        let ast = compile(
            r#"
            fn root(ctx) {
                #{ message: "Go ahead.", responses: [
                    #{ text: "Yes", on_pick: "on_yes" },
                    #{ text: "No",  on_pick: "on_no", important: true },
                ] }
            }
            fn on_yes(ctx) { }
            fn on_no(ctx) { }
            "#,
        );
        let host = RuntimeHost::new();
        let (effects, node) = enter(&host, &ast, "root", &FlagStore::new()).unwrap();
        assert!(
            effects.commands.is_empty(),
            "a root node fn buffers no effects"
        );
        let node = node.expect("root returns a node");
        assert_eq!(node.message, "Go ahead.");
        assert_eq!(
            node.responses,
            vec![
                ScriptDialogueResponse {
                    text: "Yes".into(),
                    on_pick: "on_yes".into(),
                    important: false,
                },
                ScriptDialogueResponse {
                    text: "No".into(),
                    on_pick: "on_no".into(),
                    important: true,
                },
            ]
        );
    }

    #[test]
    fn a_terminal_response_fn_returns_none() {
        let ast = compile(r#"fn done(ctx) { ctx.effects.complete_objective("obj"); }"#);
        let host = RuntimeHost::new();
        let (effects, node) = enter(&host, &ast, "done", &FlagStore::new()).unwrap();
        assert_eq!(
            cmds(&effects),
            vec![ActionCmd::CompleteObjective { id: "obj".into() }]
        );
        assert!(node.is_none(), "a fn returning () is a terminal response");
    }

    #[test]
    fn a_node_with_no_responses_materializes_empty() {
        let ast = compile(r#"fn root(ctx) { #{ message: "One-way broadcast.", responses: [] } }"#);
        let host = RuntimeHost::new();
        let (_e, node) = enter(&host, &ast, "root", &FlagStore::new()).unwrap();
        let node = node.expect("returns a node");
        assert_eq!(node.message, "One-way broadcast.");
        assert!(node.responses.is_empty());
    }

    #[test]
    fn a_wrongly_shaped_return_is_a_shape_error_not_a_panic() {
        // A fn that compiled but returned the wrong thing surfaces as an Err, not
        // a panic — it is an authoring shape error, not a script runtime error.
        let ast = compile(r#"fn root(ctx) { 42 }"#);
        let host = RuntimeHost::new();
        let err = enter(&host, &ast, "root", &FlagStore::new()).unwrap_err();
        assert!(matches!(err, EnterError::Shape { .. }), "{err:?}");
        assert!(err.to_string().contains("message"), "{err}");
    }

    /// A shape error must NOT un-apply work the fn really did: the call
    /// succeeded and its buffers drained, so its effects come back alongside the
    /// complaint. (Settled decision 10 discards a call's buffers whole on a
    /// script ERROR — it can, because they were never drained. A malformed
    /// return AFTER a successful call is the other side of that line.)
    #[test]
    fn a_shape_error_still_returns_the_effects_the_call_produced() {
        let ast = compile(
            r#"fn root(ctx) {
                ctx.effects.complete_objective("reach_axiom");
                "not a node map"
            }"#,
        );
        let host = RuntimeHost::new();
        let err = enter(&host, &ast, "root", &FlagStore::new()).unwrap_err();
        let EnterError::Shape { effects, .. } = err else {
            panic!("expected a shape error, got {err:?}");
        };
        assert_eq!(
            cmds(&effects),
            vec![ActionCmd::CompleteObjective {
                id: "reach_axiom".into()
            }],
            "the completed objective must survive the malformed return"
        );
    }

    /// An unresolvable `on_pick` is answered as a refusal, not as a `CallError`
    /// (which the failure policy would turn into a dev panic mid-mission and a
    /// release result indistinguishable from a terminal response).
    #[test]
    fn a_fn_name_that_resolves_to_nothing_is_unresolved_not_a_panic() {
        let ast = compile(r#"fn root(ctx) { #{ message: "hi", responses: [] } }"#);
        let host = RuntimeHost::new();
        let err = enter(&host, &ast, "no_such_fn", &FlagStore::new()).unwrap_err();
        assert!(matches!(err, EnterError::Unresolved), "{err:?}");
    }

    /// A budget-refused call is `Refused`, NOT a terminal `Ok((empty, None))` —
    /// the distinction the player's rejection feedback rides on.
    #[test]
    fn a_budget_refused_call_is_refused_not_terminal() {
        let ast = compile(r#"fn root(ctx) { }"#);
        let host = RuntimeHost::new();
        let mut budget = TickBudget::new();
        for _ in 0..crate::world::script::MAX_CALLS_PER_TICK {
            budget.admit_call();
        }
        let err = enter_node(
            &host,
            &mut budget,
            &SchedClock::ZERO,
            &ast,
            PATH,
            "root",
            &FlagStore::new(),
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
        )
        .unwrap_err();
        assert!(matches!(err, EnterError::Refused), "{err:?}");
    }

    // ── the ActionCmd boundary a dialogue's effects reach (issue #982) ────────
    //
    // These three tests were PARITY tests: each ran a scripted thread and its
    // declarative `[[comms]]` twin and asserted the two emitted an identical
    // `ActionCmd` sequence for the same player choices — the migration guard for
    // "one evaluator, two front-ends". Issue #985 deleted the second front-end,
    // so what survives is the half that pinned behaviour rather than
    // equality-to-itself: the concrete `ActionCmd` sequence each `on_pick`
    // produces, on the same boundary `dispatch_action` writes to.

    /// One node, two terminal responses, one `ActionCmd` each.
    #[test]
    fn each_response_fn_emits_its_own_action_cmds() {
        let ast = compile(
            r#"
            fn hail_axiom(ctx) {
                #{ message: "Axiom Station, go ahead.", responses: [
                    #{ text: "Acknowledge", on_pick: "on_ack" },
                    #{ text: "Decline",     on_pick: "on_decline" },
                ] }
            }
            fn on_ack(ctx)     { ctx.effects.complete_objective("reach_axiom"); }
            fn on_decline(ctx) { ctx.effects.fail_objective("reach_axiom"); }
            "#,
        );
        let host = RuntimeHost::new();
        let flags = FlagStore::new();

        let (root_effects, root) = enter(&host, &ast, "hail_axiom", &flags).unwrap();
        assert!(
            root_effects.commands.is_empty(),
            "entering a root node buffers nothing; only a pick does"
        );
        let root = root.expect("root returns a node");
        assert_eq!(root.responses.len(), 2);

        let (ack, follow) = enter(&host, &ast, &root.responses[0].on_pick, &flags).unwrap();
        assert!(follow.is_none(), "response 0 is terminal");
        assert_eq!(
            cmds(&ack),
            vec![ActionCmd::CompleteObjective {
                id: "reach_axiom".into()
            }]
        );

        let (decline, follow) = enter(&host, &ast, &root.responses[1].on_pick, &flags).unwrap();
        assert!(follow.is_none(), "response 1 is terminal");
        assert_eq!(
            cmds(&decline),
            vec![ActionCmd::FailObjective {
                id: "reach_axiom".into()
            }]
        );
    }

    /// A FOLLOW-UP hop: picking a response buffers that response's effects AND
    /// returns the next node, whose own response buffers its own.
    #[test]
    fn a_follow_up_hop_carries_its_own_action_cmds() {
        let ast = compile(
            r#"
            fn root(ctx) {
                #{ message: "Stand by.", responses: [
                    #{ text: "Wait", on_pick: "on_wait" },
                ] }
            }
            fn on_wait(ctx) {
                ctx.effects.complete_objective("waited");
                #{ message: "Patched through.", responses: [
                    #{ text: "Confirm", on_pick: "on_confirm" },
                ] }
            }
            fn on_confirm(ctx) { ctx.effects.fail_objective("aborted"); }
            "#,
        );
        let host = RuntimeHost::new();
        let flags = FlagStore::new();

        // Pick the root's only response → effects + a follow-up node.
        let (wait_effects, follow) = enter(&host, &ast, "on_wait", &flags).unwrap();
        let follow = follow.expect("on_wait returns a follow-up node");
        assert_eq!(follow.message, "Patched through.");
        assert_eq!(follow.responses.len(), 1);
        assert_eq!(
            cmds(&wait_effects),
            vec![ActionCmd::CompleteObjective {
                id: "waited".into()
            }]
        );

        // Pick the follow-up's response → its effects, and the thread ends.
        let (confirm_effects, tail) =
            enter(&host, &ast, &follow.responses[0].on_pick, &flags).unwrap();
        assert!(tail.is_none());
        assert_eq!(
            cmds(&confirm_effects),
            vec![ActionCmd::FailObjective {
                id: "aborted".into()
            }]
        );
    }

    /// A response `on_pick` fn can also compose world flags, and those route
    /// through the same `ActionCmd::MutateFlag` boundary every other flag write
    /// reaches (base layer, no `parent:` walk).
    #[test]
    fn a_response_fns_flag_writes_reach_the_mutate_flag_boundary() {
        use crate::world::dispatch::FlagMutation;
        let ast = compile(
            r#"
            fn on_pick(ctx) {
                ctx.flags.armed = 1;
                ctx.flags.increment("score", 50);
            }
            "#,
        );
        let host = RuntimeHost::new();
        let (effects, _) = enter(&host, &ast, "on_pick", &FlagStore::new()).unwrap();
        assert_eq!(
            cmds(&effects),
            vec![
                ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "armed".into(),
                    mutation: FlagMutation::SetValue(1),
                },
                ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "score".into(),
                    mutation: FlagMutation::Increment(50),
                },
            ]
        );
    }
}
