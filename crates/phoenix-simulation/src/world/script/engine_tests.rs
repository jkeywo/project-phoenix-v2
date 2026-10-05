use super::*;
use crate::world::config::TriggerAction;
use crate::world::dispatch::{ActionCmd, FlagMutation};

/// A clock five minutes and 300 ticks in, at 60 Hz — enough offset that a
/// stamped fire time is visibly `now + delay`.
fn clock() -> SchedClock {
    SchedClock {
        tick: 300,
        elapsed_secs: 5.0,
        tick_hz: 60.0,
    }
}

#[test]
fn runtime_engine_builds_and_runs_a_trivial_fn() {
    let host = RuntimeHost::new();
    let ast = host.engine().compile("fn noop(ctx) { }").expect("compiles");
    let cmds = host.call_immediate(&ast, "t.rhai", "noop", &FlagStore::new(), Map::new());
    assert!(cmds.is_empty());
}

#[test]
fn effects_and_flag_writes_emit_in_authored_order() {
    // Issue #981 hazard 2: a flag write authored BEFORE an effect must emit
    // before it, not after every effect. The M1 host appended all flag
    // writes last, so this interleaving would have failed.
    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile(
            r#"fn on_x(ctx) {
                    ctx.flags.armed = 1;
                    ctx.effects.complete_objective("obj1");
                    ctx.flags.increment("score", 50);
                    ctx.effects.fail_objective("obj2");
                }"#,
        )
        .expect("compiles");
    let cmds = host.call_immediate(&ast, "t.rhai", "on_x", &FlagStore::new(), Map::new());
    assert_eq!(
        cmds,
        vec![
            ActionCmd::MutateFlag {
                target_layer: None,
                name: "armed".to_string(),
                mutation: FlagMutation::SetValue(1),
            },
            ActionCmd::CompleteObjective {
                id: "obj1".to_string()
            },
            ActionCmd::MutateFlag {
                target_layer: None,
                name: "score".to_string(),
                mutation: FlagMutation::Increment(50),
            },
            ActionCmd::FailObjective {
                id: "obj2".to_string()
            },
        ]
    );
}

/// Issue #984: a comms dialogue fn shares the one runtime engine, so it CAN
/// call a name-resolving verb (`add_faction_enemy`). The dialogue entry point
/// now returns the full `CallEffects`, so that `Action` SURVIVES in authored
/// order for the live applier to dispatch — it is no longer warn-dropped
/// (which silently lost an authored effect).
#[test]
fn dialogue_path_keeps_a_name_resolving_effect_for_the_live_applier() {
    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile(
            r#"fn node(ctx) {
                    ctx.effects.complete_objective("a");
                    ctx.effects.add_faction_enemy("Harrow", "Alliance");
                    #{}
                }"#,
        )
        .expect("compiles");
    let (effects, _node, _ops) = host
        .try_call_returning(
            &SchedClock::ZERO,
            &ast,
            "t.rhai",
            "node",
            &[FlagStore::new()],
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
            &crate::dossier::evidence::EvidenceLog::default(),
            Map::new(),
        )
        .expect("the dialogue call must not error");
    assert_eq!(
        effects.commands,
        vec![
            BufferedEffect::Cmd(ActionCmd::CompleteObjective {
                id: "a".to_string()
            }),
            BufferedEffect::Action(TriggerAction::AddFactionEnemy {
                faction: "Harrow".to_string(),
                enemy: "Alliance".to_string(),
            }),
        ],
        "a dialogue fn's name-resolving effect must reach the applier, in \
             authored order alongside the resolved ones"
    );
}

/// Issue #984: a dialogue fn's `ctx.schedule.after(..)` used to be dropped on
/// the floor (the entry point never drained the schedule sink), which is what
/// a DELAYED scripted comms reply is authored as. It must now surface as a
/// stamped callback like any other call's.
#[test]
fn dialogue_path_keeps_scheduled_callbacks() {
    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile(
            r#"fn on_pick(ctx) {
                    ctx.schedule.after(5, |ctx| { ctx.effects.complete_objective("later"); });
                    ctx.schedule.in_seconds(10).fail_objective("timeout");
                    #{ message: "Stand by.", responses: [] }
                }"#,
        )
        .expect("compiles");
    let clk = clock();
    let (effects, node, _ops) = host
        .try_call_returning(
            &clk,
            &ast,
            "t.rhai",
            "on_pick",
            &[FlagStore::new()],
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
            &crate::dossier::evidence::EvidenceLog::default(),
            Map::new(),
        )
        .expect("the dialogue call must not error");
    assert!(!node.is_unit(), "the node map still comes back");
    assert_eq!(effects.callbacks.len(), 1, "the deferred callback survives");
    assert_eq!(effects.callbacks[0].fire_tick, 300 + 5 * 60);
    assert_eq!(effects.callbacks[0].script_path, "t.rhai");
    assert_eq!(effects.delayed.len(), 1, "the delayed effect survives too");
    assert_eq!(effects.delayed[0].fire_at_elapsed, 15.0);
}

/// Issue #984: dialogue calls join the tick's SHARED budget (they used to be
/// entirely unbudgeted). A refused call is dropped whole — no effects, no
/// node — exactly as `call` drops a refused handler.
#[test]
fn dialogue_call_charges_and_obeys_the_shared_budget() {
    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile(
            r#"fn node(ctx) {
                    ctx.effects.complete_objective("a");
                    #{ message: "Go ahead.", responses: [] }
                }"#,
        )
        .expect("compiles");
    let mut budget = TickBudget::new();
    let (effects, node) = host
        .call_dialogue(
            &mut budget,
            &SchedClock::ZERO,
            &ast,
            "t.rhai",
            "node",
            &[FlagStore::new()],
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
            &crate::dossier::evidence::EvidenceLog::default(),
            Map::new(),
        )
        .expect("an admitted call runs");
    assert_eq!(effects.commands.len(), 1);
    assert!(!node.is_unit());
    assert_eq!(budget.calls_used(), 1, "a dialogue call takes a call slot");
    assert!(budget.ops_used() > 0, "and charges its operations");

    // Exhaust the call cap, then a dialogue call is refused cleanly: `None`,
    // which is what distinguishes it from a terminal fn returning `()`.
    for _ in 0..crate::world::script::MAX_CALLS_PER_TICK {
        budget.admit_call();
    }
    assert!(
        host.call_dialogue(
            &mut budget,
            &SchedClock::ZERO,
            &ast,
            "t.rhai",
            "node",
            &[FlagStore::new()],
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
            &crate::dossier::evidence::EvidenceLog::default(),
            Map::new(),
        )
        .is_none(),
        "a dialogue call over the cap is dropped whole and reported as such"
    );
    assert!(budget.tripped(), "the refusal leaves the budget tripped");
}

#[test]
fn in_seconds_stamps_a_delayed_effect() {
    // `in_seconds(n).<verb>(…)` buffers a delayed effect that surfaces as a
    // `DelayedAction` ready for `pending_delayed_actions`, with the delay
    // converted to elapsed seconds at the host boundary.
    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile(
            r#"fn on_x(ctx) {
                    ctx.effects.complete_objective("now");
                    ctx.schedule.in_seconds(10).complete_objective("later");
                }"#,
        )
        .expect("compiles");
    let mut budget = TickBudget::new();
    let clk = clock();
    let effects = host.call(
        &mut budget,
        &clk,
        &ast,
        "t.rhai",
        "on_x",
        &[FlagStore::new()],
        &crate::world::deadlines::DeadlineTable::default(),
        &crate::world::commitments::CommitmentLedger::default(),
        &crate::dossier::evidence::EvidenceLog::default(),
        Map::new(),
    );
    // The immediate effect applies now; the delayed one is deferred. The
    // immediate buffer holds `BufferedEffect`s (a `Cmd` here).
    assert_eq!(
        effects.commands,
        vec![BufferedEffect::Cmd(ActionCmd::CompleteObjective {
            id: "now".to_string()
        })]
    );
    assert_eq!(effects.delayed.len(), 1);
    let d = &effects.delayed[0];
    assert_eq!(
        d.action,
        TriggerAction::CompleteObjective {
            id: "later".to_string()
        }
    );
    assert_eq!(d.fire_at_elapsed, 15.0, "elapsed 5s + 10s delay");
    assert!(d.origin_layer.is_none());
    assert!(effects.callbacks.is_empty());
}

#[test]
fn after_schedules_a_named_callback_from_an_anonymous_closure() {
    // `after(n, |ctx| …)` records the closure's generated `anon$…` name as a
    // serialisable `(fire_tick, script_path, fn_name)` callback, and that
    // name resolves back to a callable function on the same AST.
    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile(
            r#"fn on_x(ctx) {
                    ctx.schedule.after(5, |ctx| { ctx.effects.complete_objective("deferred"); });
                }"#,
        )
        .expect("compiles");
    let mut budget = TickBudget::new();
    let clk = clock();
    let effects = host.call(
        &mut budget,
        &clk,
        &ast,
        "t.rhai",
        "on_x",
        &[FlagStore::new()],
        &crate::world::deadlines::DeadlineTable::default(),
        &crate::world::commitments::CommitmentLedger::default(),
        &crate::dossier::evidence::EvidenceLog::default(),
        Map::new(),
    );
    assert_eq!(effects.callbacks.len(), 1);
    let cb = &effects.callbacks[0];
    assert_eq!(cb.fire_tick, 300 + 5 * 60, "tick 300 + 5s at 60 Hz");
    assert_eq!(cb.script_path, "t.rhai");
    assert!(
        cb.fn_name.starts_with("anon$"),
        "an anonymous closure lifts to a generated name, got '{}'",
        cb.fn_name
    );

    // The lifted name is callable: invoking it produces the deferred effect.
    let resolved = host.call_immediate(&ast, "t.rhai", &cb.fn_name, &FlagStore::new(), Map::new());
    assert_eq!(
        resolved,
        vec![ActionCmd::CompleteObjective {
            id: "deferred".to_string()
        }]
    );
}

#[test]
fn anonymous_closure_name_is_stable_across_hosts() {
    // The fixed hashing seed makes the generated name reproducible: two
    // independent hosts schedule the identical callback name (the basis for
    // serialising deferred work).
    let source = r#"fn on_x(ctx) {
            ctx.schedule.after(5, |ctx| { ctx.effects.complete_objective("deferred"); });
        }"#;
    let schedule_once = || {
        let host = RuntimeHost::new();
        let ast = host.engine().compile(source).expect("compiles");
        let mut budget = TickBudget::new();
        host.call(
            &mut budget,
            &clock(),
            &ast,
            "t.rhai",
            "on_x",
            &[FlagStore::new()],
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
            &crate::dossier::evidence::EvidenceLog::default(),
            Map::new(),
        )
        .callbacks
    };
    assert_eq!(
        schedule_once(),
        schedule_once(),
        "same seed + same script → identical schedule"
    );
}

#[test]
fn schedule_is_deterministic_across_runs() {
    // Same seed + same script → identical immediate, delayed and callback
    // schedules on two independent runs.
    let source = r#"fn on_x(ctx) {
            ctx.effects.complete_objective("now");
            ctx.schedule.in_seconds(3).fail_objective("soon");
            ctx.schedule.after(7, |ctx| { ctx.effects.reset_trigger("t"); });
        }"#;
    let run = || {
        let host = RuntimeHost::new();
        let ast = host.engine().compile(source).expect("compiles");
        let mut budget = TickBudget::new();
        let e = host.call(
            &mut budget,
            &clock(),
            &ast,
            "t.rhai",
            "on_x",
            &[FlagStore::new()],
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
            &crate::dossier::evidence::EvidenceLog::default(),
            Map::new(),
        );
        // Reduce to comparable, `PartialEq` parts (DelayedAction is not Eq).
        let delayed: Vec<(TriggerAction, f32)> = e
            .delayed
            .iter()
            .map(|d| (d.action.clone(), d.fire_at_elapsed))
            .collect();
        (e.commands, delayed, e.callbacks)
    };
    assert_eq!(run(), run());
}

#[test]
fn budget_drops_calls_once_the_call_cap_trips() {
    // A tripped budget drops a call whole: no effects, nothing scheduled.
    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile(r#"fn on_x(ctx) { ctx.effects.complete_objective("x"); }"#)
        .expect("compiles");
    let mut budget = TickBudget::new();
    // Exhaust the call cap.
    for _ in 0..crate::world::script::MAX_CALLS_PER_TICK {
        budget.admit_call();
    }
    assert!(!budget.tripped());
    let effects = host.call(
        &mut budget,
        &SchedClock::ZERO,
        &ast,
        "t.rhai",
        "on_x",
        &[FlagStore::new()],
        &crate::world::deadlines::DeadlineTable::default(),
        &crate::world::commitments::CommitmentLedger::default(),
        &crate::dossier::evidence::EvidenceLog::default(),
        Map::new(),
    );
    assert!(
        effects.commands.is_empty() && effects.delayed.is_empty() && effects.callbacks.is_empty(),
        "a call over the cap is dropped whole"
    );
    assert!(budget.tripped());
}

#[test]
fn budget_charges_operations_across_calls() {
    // Each real call charges a positive, deterministic op count to the tick
    // budget, so a busy tick converges toward the aggregate.
    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile(r#"fn on_x(ctx) { ctx.effects.complete_objective("x"); }"#)
        .expect("compiles");
    let mut budget = TickBudget::new();
    host.call(
        &mut budget,
        &SchedClock::ZERO,
        &ast,
        "t.rhai",
        "on_x",
        &[FlagStore::new()],
        &crate::world::deadlines::DeadlineTable::default(),
        &crate::world::commitments::CommitmentLedger::default(),
        &crate::dossier::evidence::EvidenceLog::default(),
        Map::new(),
    );
    let after_one = budget.ops_used();
    assert!(after_one > 0, "a real call charges operations");
    host.call(
        &mut budget,
        &SchedClock::ZERO,
        &ast,
        "t.rhai",
        "on_x",
        &[FlagStore::new()],
        &crate::world::deadlines::DeadlineTable::default(),
        &crate::world::commitments::CommitmentLedger::default(),
        &crate::dossier::evidence::EvidenceLog::default(),
        Map::new(),
    );
    assert!(
        budget.ops_used() > after_one,
        "a second call adds to the tick aggregate"
    );
}

#[test]
fn try_call_returns_err_on_a_runaway_script() {
    let host = RuntimeHost::new();
    // An infinite loop trips the per-call operation limit.
    let ast = host
        .engine()
        .compile("fn boom(ctx) { let i = 0; loop { i += 1; } }")
        .expect("compiles");
    let err = host
        .try_call(
            &SchedClock::ZERO,
            &ast,
            "scenario.rhai",
            "boom",
            &[FlagStore::new()],
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
            &crate::dossier::evidence::EvidenceLog::default(),
            Map::new(),
        )
        .expect_err("a runaway must be refused");
    // The failure names the file (vellum's `CallError::Runtime`).
    assert!(err.to_string().contains("scenario.rhai"), "{err}");
}

#[test]
fn try_call_discards_effects_on_error() {
    let host = RuntimeHost::new();
    // Pushes one effect, then trips the op limit — the partial buffer must
    // not come back.
    let ast = host
        .engine()
        .compile(
            r#"fn boom(ctx) {
                    ctx.effects.complete_objective("obj1");
                    let i = 0; loop { i += 1; }
                }"#,
        )
        .expect("compiles");
    assert!(host
        .try_call(
            &SchedClock::ZERO,
            &ast,
            "t.rhai",
            "boom",
            &[FlagStore::new()],
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
            &crate::dossier::evidence::EvidenceLog::default(),
            Map::new()
        )
        .is_err());
}

#[test]
fn failed_script_call_discards_all_six_effect_buffers() {
    use crate::world::deadlines::{Deadline, DeadlineHandler, DeadlineTable};

    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile(
            r#"
            fn boom(ctx) {
                ctx.effects.complete_objective("immediate");
                ctx.schedule.in_seconds(7).complete_objective("delayed");
                ctx.schedule.after(8, |ctx| { ctx.flags.callback = 1; });
                ctx.effects.open_comms(#{ from: "axiom", node_fn: "clean" });
                ctx.deadlines.slip("window", 5);
                ctx.commitments.record(#{ id: "promise", made_to: "axiom", terms: "test.terms" });
                throw "abort the complete call";
            }
            fn clean(ctx) { }
        "#,
        )
        .unwrap();
    let mut deadlines = DeadlineTable::default();
    let _ = deadlines.arm(
        &[Deadline {
            id: "window".into(),
            due_secs: 100,
            ..Default::default()
        }],
        &[DeadlineHandler {
            deadline_id: "window".into(),
            handler: "clean".into(),
            source_path: "t.rhai".into(),
        }],
        0,
        SchedClock::ZERO.tick_hz,
    );
    let commitments = crate::world::commitments::CommitmentLedger::default();
    let evidence = crate::dossier::evidence::EvidenceLog::default();
    let flags = [FlagStore::new()];
    let failed = host.try_call(
        &SchedClock::ZERO,
        &ast,
        "t.rhai",
        "boom",
        &flags,
        &deadlines,
        &commitments,
        &evidence,
        Map::new(),
    );
    assert!(
        failed.is_err(),
        "a raising call exposes no effects to commit"
    );
    assert_eq!(deadlines.get("window").unwrap().due_tick, 6000);
    assert!(commitments.get("promise").is_none());

    let (effects, _) = host
        .try_call(
            &SchedClock::ZERO,
            &ast,
            "t.rhai",
            "clean",
            &flags,
            &deadlines,
            &commitments,
            &evidence,
            Map::new(),
        )
        .unwrap();
    assert!(effects.commands.is_empty());
    assert!(effects.delayed.is_empty());
    assert!(effects.callbacks.is_empty());
    assert!(effects.comms_opens.is_empty());
    assert!(effects.deadline_changes.is_empty());
    assert!(
        effects.commitment_changes.is_empty(),
        "no failed work leaks to a later call"
    );
}

#[test]
#[should_panic(expected = "script error")]
fn call_panics_in_dev_on_a_script_error() {
    // `cargo test` runs with `debug_assertions`, so `call` takes the panic
    // arm of settled decision 10.
    let host = RuntimeHost::new();
    let ast = host
        .engine()
        .compile("fn boom(ctx) { let i = 0; loop { i += 1; } }")
        .expect("compiles");
    let _ = host.call_immediate(&ast, "t.rhai", "boom", &FlagStore::new(), Map::new());
}

#[test]
fn loading_engine_collects_registrations() {
    let state = Arc::new(Mutex::new(BuilderState {
        current_path: "world.toml#script.setup".to_string(),
        ..Default::default()
    }));
    let engine = loading_engine(state.clone());
    let ast = engine
        .compile(r#"on("flag_set:armed", "handle_armed");"#)
        .expect("compiles");
    engine.run_ast(&ast).expect("top level runs");
    drop(engine);
    let regs = Arc::try_unwrap(state)
        .map(|m| m.into_inner().expect("lock").registrations)
        .expect("engine dropped, sole owner");
    assert_eq!(
        regs,
        vec![Registration {
            event: "flag_set:armed".to_string(),
            handler: "handle_armed".to_string(),
            source_path: "world.toml#script.setup".to_string(),
        }]
    );
}
