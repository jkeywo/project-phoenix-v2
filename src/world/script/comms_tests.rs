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
        &crate::dossier::evidence::EvidenceLog::default(),
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
                ai: Default::default(),
            },
            ScriptDialogueResponse {
                text: "No".into(),
                on_pick: "on_no".into(),
                important: true,
                ai: Default::default(),
            },
        ]
    );
}

// ── Backfill choice metadata (issue #1343) ───────────────────────────────

/// The authoring seam for a decision an absent officer can be trusted with:
/// two whole numbers per response, straight through to the wire shape the
/// picker reads.
#[test]
fn a_response_can_author_a_backfill_weight_and_delay() {
    let ast = compile(
        r#"
            fn root(ctx) {
                #{ message: "Who gets the corridor?", responses: [
                    #{ text: "Stand by", on_pick: "on_hold",  ai_weight: 0, ai_delay_seconds: 5 },
                    #{ text: "Lift",     on_pick: "on_lift",  ai_weight: 1, ai_delay_seconds: 5, important: true },
                    #{ text: "Deny",     on_pick: "on_deny",  ai_weight: 1, ai_delay_seconds: 5, important: true },
                ] }
            }
            fn on_hold(ctx) { }
            fn on_lift(ctx) { }
            fn on_deny(ctx) { }
            "#,
    );
    let host = RuntimeHost::new();
    let (_e, node) = enter(&host, &ast, "root", &FlagStore::new()).unwrap();
    let node = node.expect("root returns a node");

    let (wire, _on_pick) = project_node(&node);
    assert_eq!(
        wire.responses
            .iter()
            .map(|r| (r.ai.weight, r.ai.delay_seconds))
            .collect::<Vec<_>>(),
        vec![(Some(0), Some(5)), (Some(1), Some(5)), (Some(1), Some(5))]
    );
    assert!(crate::comms::ai_choice::node_authors_ai_choice(
        &wire.responses
    ));
    assert_eq!(
        crate::comms::ai_choice::weighted_pool(&wire.responses, true)
            .iter()
            .map(|e| e.index)
            .collect::<Vec<_>>(),
        vec![1, 2],
        "the zero-weight stand-by must not be reachable by an unmanned console"
    );
}

/// A response that authors neither field materialises exactly as it always
/// did — the property that keeps every other conversation in every world on
/// the legacy first-response policy.
#[test]
fn a_response_without_backfill_metadata_authors_none() {
    let ast = compile(
        r#"
            fn root(ctx) { #{ message: "m", responses: [ #{ text: "Ack", on_pick: "on_ack" } ] } }
            fn on_ack(ctx) { }
            "#,
    );
    let host = RuntimeHost::new();
    let (_e, node) = enter(&host, &ast, "root", &FlagStore::new()).unwrap();
    let node = node.expect("root returns a node");
    assert_eq!(node.responses[0].ai, Default::default());
    let (wire, _) = project_node(&node);
    assert!(!crate::comms::ai_choice::node_authors_ai_choice(
        &wire.responses
    ));
}

/// A negative weight is an author saying "not this one", not an author
/// saying "no metadata" — the two produce different mechanisms, so the
/// clamp has to keep the field PRESENT.
#[test]
fn a_negative_weight_clamps_to_forbidden_rather_than_absent() {
    let ast = compile(
        r#"
            fn root(ctx) { #{ message: "m", responses: [ #{ text: "No", on_pick: "on_no", ai_weight: -3 } ] } }
            fn on_no(ctx) { }
            "#,
    );
    let host = RuntimeHost::new();
    let (_e, node) = enter(&host, &ast, "root", &FlagStore::new()).unwrap();
    let node = node.expect("root returns a node");
    assert_eq!(node.responses[0].ai.weight, Some(0));
}

// ── Parameterised bodies ──────────────────────────────────────────────────

/// The authoring seam for a computed figure: an optional `params` key on the
/// node map, read off whatever the script has to hand (here the flag store,
/// which is where `falling_skyway` banks the window ledger).
#[test]
fn a_node_fn_can_attach_params_to_its_message() {
    let ast = compile(
        r#"
            fn root(ctx) {
                #{
                    message: "world.skyway.comms.window_opens_short",
                    params: #{
                        available: ctx.flags.supply,
                        claimed: ctx.flags.demand,
                        note: "manifest",
                    },
                    responses: [],
                }
            }
            "#,
    );
    let mut flags = FlagStore::new();
    flags.set_flag_value("supply", 38);
    flags.set_flag_value("demand", 52);
    let host = RuntimeHost::new();
    let (_e, node) = enter(&host, &ast, "root", &flags).unwrap();
    let node = node.expect("root returns a node");

    // Integers render to strings at this seam: interpolation is textual
    // substitution and the client has no use for the distinction.
    assert_eq!(node.params["available"], "38");
    assert_eq!(node.params["claimed"], "52");
    assert_eq!(node.params["note"], "manifest");

    // And they reach the wire shape unchanged.
    let (wire, _on_pick) = project_node(&node);
    assert_eq!(wire.body, "world.skyway.comms.window_opens_short");
    assert_eq!(wire.body_params, node.params);
}

/// A node that names no figure materialises exactly as it always did — the
/// property that keeps every existing thread's payload byte-identical.
#[test]
fn a_node_fn_without_params_materializes_an_empty_table() {
    let ast = compile(r#"fn root(ctx) { #{ message: "m", responses: [] } }"#);
    let host = RuntimeHost::new();
    let (_e, node) = enter(&host, &ast, "root", &FlagStore::new()).unwrap();
    let node = node.expect("root returns a node");
    assert!(node.params.is_empty());
    assert!(project_node(&node).0.body_params.is_empty());
}

/// A params value the client could not render is an authoring error rather
/// than a silently-stringified debug form in crew-facing copy.
#[test]
fn a_params_value_that_is_not_a_scalar_raises() {
    let ast =
        compile(r#"fn root(ctx) { #{ message: "m", params: #{ bad: [1, 2] }, responses: [] } }"#);
    let host = RuntimeHost::new();
    let err = enter(&host, &ast, "root", &FlagStore::new())
        .expect_err("a non-scalar param is a shape error");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("params.bad"),
        "the error must name the offending key, got {rendered}"
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
        &crate::dossier::evidence::EvidenceLog::default(),
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
    let (confirm_effects, tail) = enter(&host, &ast, &follow.responses[0].on_pick, &flags).unwrap();
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
