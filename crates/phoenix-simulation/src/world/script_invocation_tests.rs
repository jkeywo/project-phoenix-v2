use super::*;
use crate::world::dispatch::{ActionCmd, FlagMutation};
use crate::world::script::effects::BufferedEffect;
use crate::world::script::schedule::SchedClock;
use crate::world::server::WorldRuntime;

const PATH: &str = "invocation.rhai";

fn runtime(source: &str) -> WorldScriptRuntime {
    crate::world::script::init_hashing_seed();
    let mut runtime = WorldScriptRuntime::empty();
    runtime
        .asts
        .insert(PATH.into(), runtime.host.engine().compile(source).unwrap());
    runtime
}

fn context<'a>(function: &'a str, tick: u64, origin: Option<&str>) -> ScriptCallContext<'a> {
    ScriptCallContext {
        log_ctx: "invocation test",
        clock: SchedClock {
            tick,
            elapsed_secs: 5.0,
            tick_hz: 10.0,
        },
        mission_clock_anchored: true,
        origin_layer: origin.map(str::to_owned),
        entity_name: None,
        script_path: PATH,
        function,
    }
}

fn seen(effects: &CallEffects) -> i64 {
    effects
        .commands
        .iter()
        .find_map(|effect| match effect {
            BufferedEffect::Cmd(ActionCmd::MutateFlag {
                name,
                mutation: FlagMutation::SetValue(value),
                ..
            }) if name == "seen" => Some(*value),
            _ => None,
        })
        .expect("handler emitted its observed value")
}

#[test]
fn mixed_invocations_share_the_call_cap_and_renew_once_on_the_next_tick() {
    let mut runtime = runtime("fn run(ctx) { }");
    let content = WorldContentRuntime::default();
    let call = context("run", 40, None);
    for index in 0..crate::world::script::MAX_CALLS_PER_TICK {
        // This is also what separate adapters do when entering the same tick.
        runtime.prepare_invocation_tick(40);
        if index % 2 == 0 {
            assert!(
                runtime
                    .invoke_effects(&call, &content, None)
                    .unwrap()
                    .completed
            );
        } else {
            assert!(
                runtime
                    .invoke_dialogue(&call, &content, None)
                    .unwrap()
                    .0
                    .completed
            );
        }
    }
    let calls = runtime.budget.calls_used();
    let ops = runtime.budget.ops_used();
    assert!(
        !runtime.budget.tripped(),
        "reaching the call cap alone does not trip"
    );
    assert!(matches!(
        runtime.invoke_dialogue(&call, &content, None),
        Err(DialogueInvocationError::BudgetUnavailable)
    ));
    assert_eq!(runtime.budget.calls_used(), calls);
    assert_eq!(runtime.budget.ops_used(), ops);
    assert!(
        !runtime.budget.tripped(),
        "Comms preflight spends no attempt"
    );
    assert!(
        !runtime
            .invoke_effects(&call, &content, None)
            .unwrap()
            .completed
    );
    assert!(
        runtime.budget.tripped(),
        "ordinary invocation retains admit_call semantics"
    );

    let next = context("run", 41, None);
    assert!(
        runtime
            .invoke_dialogue(&next, &content, None)
            .unwrap()
            .0
            .completed
    );
    runtime.prepare_invocation_tick(41);
    assert!(
        runtime
            .invoke_effects(&next, &content, None)
            .unwrap()
            .completed
    );
    for _ in 2..crate::world::script::MAX_CALLS_PER_TICK {
        assert!(
            runtime
                .invoke_effects(&next, &content, None)
                .unwrap()
                .completed
        );
    }
    assert!(matches!(
        runtime.invoke_dialogue(&next, &content, None),
        Err(DialogueInvocationError::BudgetUnavailable)
    ));
}

#[test]
fn aggregate_budget_is_shared_and_precedes_dialogue_unit_and_function_resolution() {
    let mut runtime = runtime("fn run(ctx) { }");
    let content = WorldContentRuntime::default();
    runtime.prepare_invocation_tick(20);
    runtime
        .budget
        .charge_ops(crate::world::script::MAX_OPS_PER_TICK);
    let mut missing = context("absent", 20, None);
    missing.script_path = "absent.rhai";
    assert!(matches!(
        runtime.invoke_dialogue(&missing, &content, None),
        Err(DialogueInvocationError::BudgetUnavailable)
    ));
    missing.script_path = PATH;
    assert!(matches!(
        runtime.invoke_dialogue(&missing, &content, None),
        Err(DialogueInvocationError::BudgetUnavailable)
    ));
    assert!(
        !runtime
            .invoke_effects(&context("run", 20, None), &content, None)
            .unwrap()
            .completed
    );

    missing.clock.tick = 21;
    missing.script_path = "absent.rhai";
    assert!(matches!(
        runtime.invoke_dialogue(&missing, &content, None),
        Err(DialogueInvocationError::MissingUnit)
    ));
    assert!(runtime.invoke_effects(&missing, &content, None).is_err());
    missing.script_path = PATH;
    assert!(matches!(
        runtime.invoke_dialogue(&missing, &content, None),
        Err(DialogueInvocationError::Node(EnterError::Unresolved))
    ));
    let ready = context("run", 21, None);
    for _ in 0..crate::world::script::MAX_CALLS_PER_TICK {
        assert!(
            runtime
                .invoke_effects(&ready, &content, None)
                .unwrap()
                .completed,
            "unresolved calls must leave the full call allowance"
        );
    }
    assert!(
        !runtime
            .invoke_effects(&ready, &content, None)
            .unwrap()
            .completed
    );
}

#[test]
fn each_invocation_reads_fresh_layer_state_and_captures_its_owner_and_clock() {
    let mut runtime = runtime(
        r#"
        fn read(ctx) {
            ctx.flags.seen = ctx.flags.value;
            ctx.schedule.after(2, |ctx| { });
            #{ message: "test.message", responses: [] }
        }
        fn read_parent(ctx) { ctx.flags.seen = ctx.flags["parent:value"]; }
    "#,
    );
    let mut content = WorldContentRuntime::default();
    content.flags.set_flag_value("value", 9);
    let mut layers = WorldLayerMap::default();
    let mut parent = WorldRuntime::default();
    parent.flags.set_flag_value("value", 3);
    let mut child = WorldRuntime {
        loader_path: Some("parent".into()),
        ..Default::default()
    };
    child.flags.set_flag_value("value", 7);
    layers.0.insert("parent".into(), parent);
    layers.0.insert("child".into(), child);
    let first = runtime
        .invoke_effects(&context("read", 40, Some("child")), &content, Some(&layers))
        .unwrap();
    assert_eq!(seen(&first), 7);
    assert_eq!(first.callbacks[0].origin_layer.as_deref(), Some("child"));
    assert_eq!(first.callbacks[0].fire_tick, 60);
    layers
        .0
        .get_mut("child")
        .unwrap()
        .flags
        .set_flag_value("value", 8);
    let (fresh, _) = runtime
        .invoke_dialogue(&context("read", 40, Some("child")), &content, Some(&layers))
        .unwrap();
    assert_eq!(seen(&fresh), 8);
    let (parent, _) = runtime
        .invoke_dialogue(
            &context("read", 40, Some("parent")),
            &content,
            Some(&layers),
        )
        .unwrap();
    assert_eq!(seen(&parent), 3);
    assert_eq!(parent.callbacks[0].origin_layer.as_deref(), Some("parent"));
    let root = runtime
        .invoke_effects(&context("read", 40, None), &content, Some(&layers))
        .unwrap();
    assert_eq!(seen(&root), 9);
    assert_eq!(root.callbacks[0].origin_layer, None);
    // Unqualified reads stay local; an explicit parent: hop follows the loader.
    layers.0.get_mut("child").unwrap().flags = Default::default();
    assert_eq!(
        seen(
            &runtime
                .invoke_effects(&context("read", 40, Some("child")), &content, Some(&layers))
                .unwrap()
        ),
        0
    );
    assert_eq!(
        seen(
            &runtime
                .invoke_effects(
                    &context("read_parent", 40, Some("child")),
                    &content,
                    Some(&layers),
                )
                .unwrap()
        ),
        3
    );
}

#[test]
fn malformed_dialogue_retains_completed_effects_and_restore_always_resets_the_budget() {
    let mut runtime = runtime(r#"fn malformed(ctx) { ctx.flags.seen = 4; 123 } fn clean(ctx) { }"#);
    let content = WorldContentRuntime::default();
    let call = context("malformed", 40, None);
    let Err(DialogueInvocationError::Node(EnterError::Shape { effects, .. })) =
        runtime.invoke_dialogue(&call, &content, None)
    else {
        panic!("successful malformed dialogue must retain its effects");
    };
    assert!(effects.completed);
    assert_eq!(seen(&effects), 4);
    // Bootstrap may already have spent the captured tick's whole budget.
    let mut spent = TickBudget::new();
    spent.charge_ops(crate::world::script::MAX_OPS_PER_TICK);
    runtime.seed_invocation_budget(40, spent.clone());
    let clean = context("clean", 40, None);
    assert!(matches!(
        runtime.invoke_dialogue(&clean, &content, None),
        Err(DialogueInvocationError::BudgetUnavailable)
    ));
    runtime.reset_invocation_budget(40);
    assert!(
        runtime
            .invoke_dialogue(&clean, &content, None)
            .unwrap()
            .0
            .completed
    );
    runtime.seed_invocation_budget(40, spent);
    runtime.reset_invocation_budget(10);
    assert!(
        runtime
            .invoke_effects(&context("clean", 10, None), &content, None)
            .unwrap()
            .completed
    );
}

#[test]
fn failed_invocations_keep_engine_failure_policy_and_do_not_leak_effects() {
    let source = r#"
        fn fail(ctx) { ctx.flags.seen = 4; throw "failed invocation"; }
        fn clean(ctx) { }
    "#;
    let content = WorldContentRuntime::default();
    for dialogue in [false, true] {
        let mut runtime = runtime(source);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if dialogue {
                assert!(matches!(
                    runtime.invoke_dialogue(&context("fail", 40, None), &content, None),
                    Err(DialogueInvocationError::Node(EnterError::Refused))
                ));
            } else {
                let effects = runtime
                    .invoke_effects(&context("fail", 40, None), &content, None)
                    .unwrap();
                assert!(!effects.completed);
                assert!(effects.commands.is_empty());
            }
        }));
        assert_eq!(result.is_err(), cfg!(debug_assertions));
        let clean = runtime
            .invoke_effects(&context("clean", 40, None), &content, None)
            .unwrap();
        assert!(clean.completed);
        assert!(
            clean.commands.is_empty(),
            "failed buffers do not reach the next call"
        );
        for _ in 2..crate::world::script::MAX_CALLS_PER_TICK {
            assert!(
                runtime
                    .invoke_effects(&context("clean", 40, None), &content, None)
                    .unwrap()
                    .completed
            );
        }
        assert!(
            !runtime
                .invoke_effects(&context("clean", 40, None), &content, None)
                .unwrap()
                .completed,
            "the failed call still spent its attempt"
        );
    }
}
