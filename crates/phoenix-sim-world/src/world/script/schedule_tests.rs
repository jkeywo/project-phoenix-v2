use super::*;

fn clock(tick: u64, elapsed: f32, hz: f32) -> SchedClock {
    SchedClock {
        tick,
        elapsed_secs: elapsed,
        tick_hz: hz,
    }
}

#[test]
fn seconds_to_ticks_rounds_at_the_authored_rate() {
    assert_eq!(seconds_to_ticks(5, 60.0), 300);
    assert_eq!(seconds_to_ticks(1, 30.0), 30);
    // Non-positive delays fire on the next tick.
    assert_eq!(seconds_to_ticks(0, 60.0), 0);
    assert_eq!(seconds_to_ticks(-5, 60.0), 0);
}

#[test]
fn pending_callbacks_round_trip_through_serialization() {
    // The `(tick, script_path, fn_name)` key is the serialisable deferred-work
    // record: a save must reload the identical queue.
    let mut queue = PendingCallbacks::new();
    queue.push(ScheduledCall {
        fire_tick: 300,
        script_path: "world.toml#script.setup".to_string(),
        fn_name: "anon$41a691411dc30a5e".to_string(),
        origin_layer: None,
    });
    queue.push(ScheduledCall {
        fire_tick: 42,
        script_path: "combat.rhai".to_string(),
        fn_name: "on_reinforce".to_string(),
        origin_layer: Some("worlds/reinforcements.toml".to_string()),
    });

    let json = serde_json::to_string(&queue).expect("serialises");
    let restored: PendingCallbacks = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(restored, queue, "the pending queue must round-trip exactly");

    let legacy = r#"[{"fire_tick":7,"script_path":"old.rhai","fn_name":"f"}]"#;
    let restored: PendingCallbacks =
        serde_json::from_str(legacy).expect("pre-#1045 calls remain readable");
    assert_eq!(restored.0[0].origin_layer, None);
}

#[test]
fn scoped_drain_stamps_callbacks_and_delayed_effects_with_the_same_owner() {
    let sink = ScheduleSink::new();
    sink.push(Deferred::Effect {
        delay_secs: 1,
        action: Box::new(TriggerAction::CompleteObjective { id: "x".into() }),
    });
    sink.push(Deferred::Callback {
        delay_secs: 1,
        fn_name: "later".into(),
    });

    let (delayed, callbacks) =
        sink.drain_scoped(&clock(10, 2.0, 60.0), "shared.rhai", Some("worlds/a.toml"));
    assert_eq!(delayed[0].origin_layer.as_deref(), Some("worlds/a.toml"));
    assert_eq!(callbacks[0].origin_layer.as_deref(), Some("worlds/a.toml"));
}

#[test]
fn sink_drain_stamps_absolute_fire_times() {
    // A delayed effect converts seconds→elapsed; a callback converts
    // seconds→tick, both against the clock, and the callback is attributed to
    // the draining unit's path.
    let sink = ScheduleSink::new();
    sink.push(Deferred::Effect {
        delay_secs: 10,
        action: Box::new(TriggerAction::CompleteObjective {
            id: "later".to_string(),
        }),
    });
    sink.push(Deferred::Callback {
        delay_secs: 5,
        fn_name: "anon$abc".to_string(),
    });
    assert_eq!(sink.len(), 2);

    let (delayed, callbacks) = sink.drain(&clock(300, 5.0, 60.0), "combat.rhai");
    assert!(sink.is_empty(), "drain empties the buffer");

    assert_eq!(delayed.len(), 1);
    assert_eq!(delayed[0].fire_at_elapsed, 15.0, "elapsed 5 + delay 10");
    assert!(delayed[0].origin_layer.is_none());

    assert_eq!(
        callbacks,
        vec![ScheduledCall {
            fire_tick: 300 + 5 * 60,
            script_path: "combat.rhai".to_string(),
            fn_name: "anon$abc".to_string(),
            origin_layer: None,
        }]
    );
}

/// A DELAYED destroy buffers the identical `TriggerAction` the immediate verb
/// does, stamped with its fire time (issue #1033, AC6).
///
/// The AC is "with no new machinery", and this is what that cashes out to: the
/// action reaches `pending_delayed_actions` as an ordinary `DelayedAction`, so
/// `tick_delayed_actions` resolves it through the same `dispatch_action` and
/// applies the same whole `DispatchResult` — chaining included. Nothing in the
/// deferred path knows a destroy is different from a spawn.
#[test]
fn a_delayed_destroy_entity_defers_the_same_action() {
    use crate::world::script::engine::runtime_engine;
    use rhai::{Dynamic, Map};

    let engine = runtime_engine();
    let ast = engine
        .compile(r#"fn on_x(ctx) { ctx.schedule.in_seconds(8).destroy_entity("skyhook"); }"#)
        .expect("compiles");
    let sink = ScheduleSink::new();
    let mut ctx = Map::new();
    ctx.insert("schedule".into(), Dynamic::from(sink.clone()));
    let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("the call runs");

    let (delayed, callbacks) = sink.drain(&clock(0, 2.0, 60.0), "t.rhai");
    assert!(callbacks.is_empty(), "a delayed effect is not a callback");
    assert_eq!(delayed.len(), 1);
    assert_eq!(
        delayed[0].fire_at_elapsed, 10.0,
        "elapsed 2 + delay 8, the seconds→elapsed conversion every delayed \
             effect shares"
    );
    assert_eq!(
        delayed[0].action,
        TriggerAction::DestroyEntity {
            entity: "skyhook".to_string(),
        },
        "byte-identical to what `ctx.effects.destroy_entity` buffers — both \
             build it through `destroy_entity_action`"
    );
}

/// The delayed `game_over` overload that DECLARES an outcome (issue #984).
///
/// `combat_test`'s victory window is a declarative `game_over` carrying both
/// `outcome = "victory"` and `delay_secs = 5.0` on the same action, so
/// without this the conversion could only defer an UNDECLARED end and the
/// balance classifier would read a scripted victory as a draw. Validated
/// through the same `Outcome::parse` as the immediate form, so a typo raises
/// and the call's whole buffer is discarded rather than a bad end deferred.
#[test]
fn a_delayed_game_over_can_declare_its_outcome() {
    use crate::world::script::engine::runtime_engine;
    use rhai::{Dynamic, Map};

    fn deferred(source: &str) -> Result<Vec<DelayedAction>, String> {
        let engine = runtime_engine();
        let ast = engine.compile(source).expect("compiles");
        let sink = ScheduleSink::new();
        let mut ctx = Map::new();
        ctx.insert("schedule".into(), Dynamic::from(sink.clone()));
        vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx)
            .map(|_| sink.drain(&clock(0, 0.0, 60.0), "t.rhai").0)
            .map_err(|e| e.to_string())
    }

    let delayed =
        deferred(r#"fn on_x(ctx) { ctx.schedule.in_seconds(5).game_over("msg", "victory"); }"#)
            .expect("the call runs");
    assert_eq!(delayed.len(), 1);
    assert_eq!(delayed[0].fire_at_elapsed, 5.0);
    assert_eq!(
        delayed[0].action,
        TriggerAction::GameOver {
            message: Some("msg".to_string()),
            outcome: Some(crate::core::balance::Outcome::Victory),
        }
    );

    // The one-arg form still defers an UNDECLARED end.
    let undeclared = deferred(r#"fn on_x(ctx) { ctx.schedule.in_seconds(5).game_over("msg"); }"#)
        .expect("the call runs");
    assert_eq!(
        undeclared[0].action,
        TriggerAction::GameOver {
            message: Some("msg".to_string()),
            outcome: None,
        }
    );

    // And a bad outcome raises rather than deferring a nonsense end.
    assert!(
        deferred(r#"fn on_x(ctx) { ctx.schedule.in_seconds(5).game_over("m", "victni"); }"#)
            .is_err(),
        "an unparseable outcome must raise, as it does on the immediate form"
    );
}

#[test]
fn drain_due_splits_by_tick_preserving_order() {
    let mut queue = PendingCallbacks::new();
    for (fire, name) in [(10, "a"), (300, "b"), (20, "c"), (5, "d")] {
        queue.push(ScheduledCall {
            fire_tick: fire,
            script_path: "s.rhai".to_string(),
            fn_name: name.to_string(),
            origin_layer: None,
        });
    }
    let due = queue.drain_due(20);
    let due_names: Vec<&str> = due.iter().map(|c| c.fn_name.as_str()).collect();
    // `now >= fire`, original order preserved.
    assert_eq!(due_names, vec!["a", "c", "d"]);
    let pending_names: Vec<&str> = queue.0.iter().map(|c| c.fn_name.as_str()).collect();
    assert_eq!(pending_names, vec!["b"]);
}

#[test]
fn retract_removes_one_equal_call_and_leaves_its_twin() {
    // The deadline re-keying primitive (issue #1024). Equal keys are legal —
    // two deadlines may share a handler and a tick — so a retraction takes
    // exactly one.
    let call = |fire: u64, name: &str| ScheduledCall {
        fire_tick: fire,
        script_path: "s.rhai".to_string(),
        fn_name: name.to_string(),
        origin_layer: None,
    };
    let mut queue = PendingCallbacks::new();
    queue.push(call(300, "shared"));
    queue.push(call(300, "shared"));
    queue.push(call(600, "other"));

    assert!(
        queue.retract(&call(300, "shared")),
        "the first equal one goes"
    );
    assert_eq!(queue.len(), 2);
    assert!(queue.retract(&call(300, "shared")), "and so does its twin");
    assert_eq!(queue.len(), 1);
    assert!(
        !queue.retract(&call(300, "shared")),
        "a third retraction finds nothing and says so"
    );
    assert_eq!(
        queue.drain_due(600).len(),
        1,
        "the unrelated call is untouched"
    );
}

#[test]
fn budget_call_cap_trips_and_drops_the_rest() {
    let mut budget = TickBudget::new();
    for _ in 0..MAX_CALLS_PER_TICK {
        assert!(budget.admit_call(), "calls under the cap are admitted");
    }
    assert!(
        !budget.tripped(),
        "reaching the cap exactly is still admitted"
    );
    assert!(
        !budget.admit_call(),
        "the call over the cap is dropped and trips the budget"
    );
    assert!(budget.tripped());
    assert!(
        !budget.admit_call(),
        "a tripped budget drops every later call"
    );
}

#[test]
fn can_admit_agrees_with_admit_call_at_every_step() {
    // The pre-flight predicate and the gate must never disagree — including
    // on the call that REACHES the cap, which `tripped()` alone gets wrong.
    let mut budget = TickBudget::new();
    for _ in 0..MAX_CALLS_PER_TICK + 2 {
        let predicted = budget.can_admit();
        assert_eq!(
            predicted,
            budget.admit_call(),
            "can_admit must predict admit_call exactly"
        );
    }
    // And the specific case the pre-flight used to miss: at the cap, the
    // budget has NOT tripped yet, but the next call will be refused.
    let mut budget = TickBudget::new();
    for _ in 0..MAX_CALLS_PER_TICK {
        assert!(budget.admit_call());
    }
    assert!(!budget.tripped(), "reaching the cap exactly does not trip");
    assert!(
        !budget.can_admit(),
        "but the next call is already refused — what `tripped()` could not see"
    );
}

#[test]
fn budget_op_aggregate_trips_across_calls() {
    let mut budget = TickBudget::new();
    // Two calls just under half the aggregate: fine.
    budget.admit_call();
    budget.charge_ops(MAX_OPS_PER_TICK / 2 - 1);
    assert!(!budget.tripped());
    budget.admit_call();
    budget.charge_ops(MAX_OPS_PER_TICK / 2 - 1);
    assert!(!budget.tripped(), "still under the aggregate");
    // The call that crosses the aggregate trips it.
    budget.admit_call();
    budget.charge_ops(2);
    assert!(budget.tripped());
    assert!(
        !budget.admit_call(),
        "once the op aggregate trips, remaining calls are dropped"
    );
}

#[test]
fn budget_trip_point_is_deterministic() {
    // Same op sequence → same trip point on any peer.
    let run = || {
        let mut b = TickBudget::new();
        let mut admitted = 0u32;
        for _ in 0..10 {
            if b.admit_call() {
                admitted += 1;
                b.charge_ops(MAX_OPS_PER_TICK / 4);
            }
        }
        (admitted, b.tripped())
    };
    assert_eq!(
        run(),
        run(),
        "the trip is a pure function of the op sequence"
    );
}
