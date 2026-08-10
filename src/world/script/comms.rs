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
//! # Dormant in M4 (the collapse is deferred to M7)
//!
//! Nothing wires this into the live `handle_hail` / `handle_respond_to_message`
//! path yet: a `[[comms]] script = "fn"` block parses into a metadata-only
//! [`ScriptedCommsTemplate`](crate::world::config::ScriptedCommsTemplate) held
//! apart from `WorldConfig::comms`, and the root fn name is cross-reference
//! validated at load ([`validate_toml_script_comms`](super::validate::validate_toml_script_comms)).
//! Unifying the two response-dispatch paths — routing both TOML and script comms
//! responses through one shared applier — is the M7 (teardown) collapse. Landing
//! this front-end dormant, front-end + validation + parity test, matches how M2
//! landed the scripted-trigger front-end without touching the trigger evaluator.

use rhai::{Dynamic, Map};

use crate::world::dispatch::ActionCmd;
use crate::world::flags::FlagStore;
use crate::world::script::engine::RuntimeHost;

/// One response option in a scripted dialogue node — the script analogue of a
/// [`CommsResponse`](crate::world::config::CommsResponse).
///
/// `on_pick` names the fn to run when the player picks this response; that fn
/// supplies the response's effects and returns the follow-up node (or `()`), so
/// the branching lives in fn-to-fn references rather than nested TOML tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptDialogueResponse {
    /// Player-facing button text (authored inline, exactly like
    /// [`CommsResponse::text`](crate::world::config::CommsResponse)).
    pub text: String,
    /// Name of the fn to call when this response is picked.
    pub on_pick: String,
    /// Whether the client confirms before submitting this response (the script
    /// analogue of [`CommsResponse::important`](crate::world::config::CommsResponse)).
    /// Defaults to `false` when the response map omits it.
    pub important: bool,
}

/// A scripted dialogue node materialized from a node fn's return map — the
/// script analogue of a [`CommsDialogueNode`](crate::world::config::CommsDialogueNode).
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

/// Enter a dialogue node: run the fn named `fn_name` and materialize both the
/// effects it buffered (the M1 `ActionCmd` boundary) and the node it returned.
///
/// This is the single operation behind BOTH "show the root node" (call the root
/// fn; it buffers no effects and returns the node to display) and "pick response
/// *i*" (call `responses[i].on_pick`; it buffers that response's effects and
/// returns the follow-up node, or `None` for a terminal response). The returned
/// `commands` are exactly what the declarative TOML path would dispatch for the
/// same choice — see the module docs.
pub fn enter_node(
    host: &RuntimeHost,
    ast: &rhai::AST,
    path: &str,
    fn_name: &str,
    base_flags: &FlagStore,
) -> Result<(Vec<ActionCmd>, Option<ScriptDialogueNode>), String> {
    let (commands, value) = host.call_dialogue(ast, path, fn_name, base_flags, Map::new());
    let node = read_dialogue_node(value)?;
    Ok((commands, node))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::config::parse_world;
    use crate::world::dispatch::{dispatch_action, DispatchContext};
    use crate::world::script::engine::RuntimeHost;
    use crate::world::script::load::compile_scripts;
    use std::collections::HashMap;
    use vellum_script::ScriptSource;

    const PATH: &str = "w.toml#script.axiom";

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

    /// A minimal context-free dispatch context — the objective actions the parity
    /// tests use resolve nothing, so only the required borrows are populated.
    /// Mirrors the M2 trigger parity test's context.
    fn dispatch_toml(actions: &[crate::world::config::TriggerAction]) -> Vec<ActionCmd> {
        let empty_names: HashMap<String, String> = HashMap::new();
        let base_flags = FlagStore::new();
        let layers = HashMap::new();
        let anchors = HashMap::new();
        let uuid = || "uuid".to_string();
        let ctx = DispatchContext {
            origin_layer: None,
            entity_name: None,
            name_to_uuid: &empty_names,
            base_flags: &base_flags,
            layers: &layers,
            base_anchors: &anchors,
            factions: None,
            uuid_source: &uuid,
            template_loader: &crate::entity_loader::WasmTemplateLoader,
        };
        actions
            .iter()
            .flat_map(|a| dispatch_action(a, &ctx).commands)
            .collect()
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
        let (effects, node) = enter_node(&host, &ast, PATH, "root", &FlagStore::new()).unwrap();
        assert!(effects.is_empty(), "a root node fn buffers no effects");
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
        let (effects, node) = enter_node(&host, &ast, PATH, "done", &FlagStore::new()).unwrap();
        assert_eq!(
            effects,
            vec![ActionCmd::CompleteObjective { id: "obj".into() }]
        );
        assert!(node.is_none(), "a fn returning () is a terminal response");
    }

    #[test]
    fn a_node_with_no_responses_materializes_empty() {
        let ast = compile(r#"fn root(ctx) { #{ message: "One-way broadcast.", responses: [] } }"#);
        let host = RuntimeHost::new();
        let (_e, node) = enter_node(&host, &ast, PATH, "root", &FlagStore::new()).unwrap();
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
        let err = enter_node(&host, &ast, PATH, "root", &FlagStore::new()).unwrap_err();
        assert!(err.contains("message"), "{err}");
    }

    // ── parity: scripted thread == TOML thread, same ActionCmds (issue #982) ──

    /// The strongest migration guard (acceptance criterion): a scripted dialogue
    /// thread and its declarative TOML equivalent produce the identical
    /// `ActionCmd` sequence for the same player choices. The scripted path runs
    /// the picked response's `on_pick` fn on the runtime host and collects its
    /// buffered effects; the TOML path dispatches the same response's
    /// `[[response.action]]` list through `dispatch_action`. Both land on the one
    /// `ActionCmd` boundary.
    #[test]
    fn scripted_and_toml_comms_emit_identical_action_cmds_per_choice() {
        // ---- Scripted front-end: the whole dialogue tree in Rhai. ----
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

        let (root_effects, root) = enter_node(&host, &ast, PATH, "hail_axiom", &flags).unwrap();
        assert!(root_effects.is_empty());
        let root = root.expect("root returns a node");
        assert_eq!(root.responses.len(), 2);

        // ---- Declarative TOML front-end: the same thread, tables inline. ----
        let cfg = parse_world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            message = "Axiom Station, go ahead."

            [[comms.response]]
            text = "Acknowledge"
            [[comms.response.action]]
            type = "complete_objective"
            id = "reach_axiom"

            [[comms.response]]
            text = "Decline"
            [[comms.response.action]]
            type = "fail_objective"
            id = "reach_axiom"
            "#,
        )
        .expect("world parses");
        // A TOML comms block (not scripted) still lands in `comms`.
        assert_eq!(cfg.comms.len(), 1);
        assert!(cfg.scripted_comms.is_empty());
        let toml_node = &cfg.comms[0].node;
        assert_eq!(toml_node.responses.len(), 2);

        // For EACH choice, the scripted `on_pick` effects == the TOML response
        // actions dispatched.
        for i in 0..2 {
            let (scripted_cmds, follow) =
                enter_node(&host, &ast, PATH, &root.responses[i].on_pick, &flags).unwrap();
            assert!(follow.is_none(), "response {i} is terminal");
            let toml_cmds = dispatch_toml(&toml_node.responses[i].actions);
            assert_eq!(
                scripted_cmds, toml_cmds,
                "choice {i}: scripted and TOML must emit identical ActionCmds"
            );
        }

        // And the concrete expected sequences, so this pins behaviour not just
        // equality-to-itself.
        let (ack, _) = enter_node(&host, &ast, PATH, "on_ack", &flags).unwrap();
        assert_eq!(
            ack,
            vec![ActionCmd::CompleteObjective {
                id: "reach_axiom".into()
            }]
        );
        let (decline, _) = enter_node(&host, &ast, PATH, "on_decline", &flags).unwrap();
        assert_eq!(
            decline,
            vec![ActionCmd::FailObjective {
                id: "reach_axiom".into()
            }]
        );
    }

    /// Parity across a FOLLOW-UP hop: picking a response advances to the next
    /// node (script: the `on_pick` fn returns it; TOML: `response.follow_up`),
    /// and a choice on that follow-up node again emits identical `ActionCmd`s.
    #[test]
    fn scripted_and_toml_follow_up_hop_emit_identical_action_cmds() {
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
        let (wait_cmds, follow) = enter_node(&host, &ast, PATH, "on_wait", &flags).unwrap();
        let follow = follow.expect("on_wait returns a follow-up node");
        assert_eq!(follow.message, "Patched through.");
        assert_eq!(follow.responses.len(), 1);

        // Pick the follow-up's response → its effects.
        let (confirm_cmds, tail) =
            enter_node(&host, &ast, PATH, &follow.responses[0].on_pick, &flags).unwrap();
        assert!(tail.is_none());

        // ---- TOML equivalent: nested response.follow_up. ----
        let cfg = parse_world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            message = "Stand by."

            [[comms.response]]
            text = "Wait"
            [[comms.response.action]]
            type = "complete_objective"
            id = "waited"

            [comms.response.follow_up]
            message = "Patched through."
            [[comms.response.follow_up.response]]
            text = "Confirm"
            [[comms.response.follow_up.response.action]]
            type = "fail_objective"
            id = "aborted"
            "#,
        )
        .expect("world parses");
        let root_resp = &cfg.comms[0].node.responses[0];
        assert_eq!(wait_cmds, dispatch_toml(&root_resp.actions));
        let follow_resp = &root_resp
            .follow_up
            .as_ref()
            .expect("toml follow-up")
            .responses[0];
        assert_eq!(confirm_cmds, dispatch_toml(&follow_resp.actions));

        // Concrete sequences.
        assert_eq!(
            wait_cmds,
            vec![ActionCmd::CompleteObjective {
                id: "waited".into()
            }]
        );
        assert_eq!(
            confirm_cmds,
            vec![ActionCmd::FailObjective {
                id: "aborted".into()
            }]
        );
    }

    /// A response `on_pick` fn can also compose world flags, and those route
    /// through the same `ActionCmd::MutateFlag` boundary a TOML
    /// `set_flag_value` / `increment_flag` action produces.
    #[test]
    fn scripted_response_flag_writes_match_toml_flag_actions() {
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
        let (cmds, _) = enter_node(&host, &ast, PATH, "on_pick", &FlagStore::new()).unwrap();
        assert_eq!(
            cmds,
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

        // The TOML equivalent (`set_flag_value` + `increment_flag`) dispatches the
        // same two MutateFlag commands (base layer, no `parent:` walk).
        let cfg = parse_world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            message = "x"
            [[comms.response]]
            text = "Arm"
            [[comms.response.action]]
            type = "set_flag_value"
            name = "armed"
            value = 1
            [[comms.response.action]]
            type = "increment_flag"
            name = "score"
            by = 50
            "#,
        )
        .expect("world parses");
        let toml_cmds = dispatch_toml(&cfg.comms[0].node.responses[0].actions);
        assert_eq!(cmds, toml_cmds);
    }
}
