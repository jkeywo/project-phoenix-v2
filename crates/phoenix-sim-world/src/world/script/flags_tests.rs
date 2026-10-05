use super::*;
use crate::world::script::engine::runtime_engine;
use rhai::{Dynamic, Map};

/// Compile `source`, call `fn_name` with a `flags` view over `base` sharing a
/// fresh effect sink, and return the emitted commands in authored order. Flag
/// writes are all `Cmd` effects, so the drained `BufferedEffect`s unwrap to
/// their `ActionCmd`s here.
fn run(source: &str, fn_name: &str, base: FlagStore) -> Vec<ActionCmd> {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let sink = EffectSink::new();
    let flags = Flags::new(&base, sink.clone());
    let mut ctx = Map::new();
    ctx.insert("flags".into(), Dynamic::from(flags));
    let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).expect("calls");
    use crate::world::script::effects::BufferedEffect;
    sink.take()
        .into_iter()
        .map(|e| match e {
            BufferedEffect::Cmd(cmd) => cmd,
            BufferedEffect::Action(a) => {
                unreachable!("flags emit only command effects, got {a:?}")
            }
        })
        .collect()
}

#[test]
fn read_after_write_sees_the_written_value_within_one_call() {
    // `a` is incremented twice, then `b` is set FROM `a` — so `b` only ends
    // up 12 if the second read of `a` saw the first two increments.
    let cmds = run(
        r#"fn on_x(ctx) {
                ctx.flags.increment("a", 5);
                ctx.flags.increment("a", 7);
                ctx.flags.b = ctx.flags.a;
            }"#,
        "on_x",
        FlagStore::new(),
    );
    // Emitted in authored order: two composable increments, then an absolute
    // set of `b` to the read-back value (12).
    assert_eq!(
        cmds,
        vec![
            ActionCmd::MutateFlag {
                target_layer: None,
                name: "a".to_string(),
                mutation: FlagMutation::Increment(5),
            },
            ActionCmd::MutateFlag {
                target_layer: None,
                name: "a".to_string(),
                mutation: FlagMutation::Increment(7),
            },
            ActionCmd::MutateFlag {
                target_layer: None,
                name: "b".to_string(),
                mutation: FlagMutation::SetValue(12),
            },
        ]
    );
}

#[test]
fn increment_emits_a_relative_mutation_reading_the_base_snapshot() {
    let mut base = FlagStore::new();
    base.set_flag_value("score", 100);
    // The increment drains as a RELATIVE Increment(50), not an absolute
    // SetValue(150): that is what lets it compose with a concurrent increment
    // (issue #981 hazard 1). The base snapshot (100) is still what the overlay
    // reads for read-after-write.
    let cmds = run(
        r#"fn on_x(ctx) { ctx.flags.increment("score", 50); }"#,
        "on_x",
        base,
    );
    assert_eq!(
        cmds,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "score".to_string(),
            mutation: FlagMutation::Increment(50),
        }]
    );
}

#[test]
fn absolute_assignment_drains_setvalue() {
    // `flags.x = v` stays absolute.
    let cmds = run(
        r#"fn on_x(ctx) { ctx.flags.armed = 1; }"#,
        "on_x",
        FlagStore::new(),
    );
    assert_eq!(
        cmds,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "armed".to_string(),
            mutation: FlagMutation::SetValue(1),
        }]
    );
}

/// PIN (issue #994): `flags.x += n` on the indexer degrades to an **absolute**
/// `SetValue(final)`, NOT a composable `Increment(n)`.
///
/// Rhai desugars a compound assignment on a custom-type indexer to get-then-set
/// *before* the custom type is consulted, so the host only ever sees a set of
/// the final computed value — physically indistinguishable from `flags.x =
/// final`. That is the exact clobber-prone degradation the load-time lint
/// (`validate::validate_flag_opassign`) now rejects. This test nails the
/// runtime behaviour so a future Rhai upgrade that changed the desugaring, or
/// an attempt to intercept `+=`, breaks here loudly rather than silently
/// altering flag semantics out from under the lint's premise.
#[test]
fn plus_equals_degrades_to_absolute_setvalue_not_increment() {
    let mut base = FlagStore::new();
    base.set_flag_value("x", 10);
    let cmds = run(r#"fn on_x(ctx) { ctx.flags.x += 5; }"#, "on_x", base);
    // The host sees an ABSOLUTE set of the final value (10 + 5 = 15), not a
    // relative Increment(5) — so a concurrent TOML increment would be clobbered.
    assert_eq!(
        cmds,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "x".to_string(),
            mutation: FlagMutation::SetValue(15),
        }]
    );
    assert!(
        !matches!(
            cmds[0],
            ActionCmd::MutateFlag {
                mutation: FlagMutation::Increment(_),
                ..
            }
        ),
        "`+=` must degrade to SetValue; an Increment here would void the lint's premise"
    );
}

#[test]
fn no_writes_emit_nothing() {
    // Reading a flag must not emit a mutation.
    let cmds = run(
        r#"fn on_x(ctx) { let seen = ctx.flags.absent; }"#,
        "on_x",
        FlagStore::new(),
    );
    assert!(cmds.is_empty());
}

#[test]
fn indexer_syntax_also_works() {
    let cmds = run(
        r#"fn on_x(ctx) { ctx.flags["kills"] = 3; }"#,
        "on_x",
        FlagStore::new(),
    );
    assert_eq!(
        cmds,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "kills".to_string(),
            mutation: FlagMutation::SetValue(3),
        }]
    );
}

/// Parity: a scripted increment and a TOML `increment_flag` on the same flag
/// in the same tick COMPOSE — neither clobbers the other, in either order.
/// The scripted mutation being `Increment` (not `SetValue`) is what makes it
/// order-independent (issue #981 hazard 1).
#[test]
fn scripted_increment_and_toml_increment_compose() {
    // The scripted `+5` as it drains from the overlay.
    let cmds = run(
        r#"fn on_x(ctx) { ctx.flags.increment("kills", 5); }"#,
        "on_x",
        FlagStore::new(),
    );
    let scripted = match &cmds[..] {
        [ActionCmd::MutateFlag {
            mutation: FlagMutation::Increment(by),
            ..
        }] => *by,
        other => panic!("expected one Increment, got {other:?}"),
    };
    assert_eq!(scripted, 5);

    // Apply the scripted increment and a TOML increment_flag(+3) both ways.
    // `MutateFlag { Increment }` is applied via `FlagStore::increment_flag`
    // (see `world::server`'s MutateFlag arm), so mirror that here.
    let mut store_a = FlagStore::new();
    store_a.increment_flag("kills", scripted); // script first
    store_a.increment_flag("kills", 3); // TOML second

    let mut store_b = FlagStore::new();
    store_b.increment_flag("kills", 3); // TOML first
    store_b.increment_flag("kills", scripted); // script second

    assert_eq!(store_a.counter("kills"), 8);
    assert_eq!(
        store_a.counter("kills"),
        store_b.counter("kills"),
        "two increments must compose to the same value in either order"
    );

    // Contrast: had the script drained an absolute SetValue(5) as in M1, the
    // TOML +3 would be clobbered in one order (5) and kept in the other (8) —
    // the hazard this fix removes.
    let mut clobber_a = FlagStore::new();
    clobber_a.set_flag_value("kills", 5); // script (absolute) first
    clobber_a.increment_flag("kills", 3);
    let mut clobber_b = FlagStore::new();
    clobber_b.increment_flag("kills", 3); // TOML first
    clobber_b.set_flag_value("kills", 5); // script (absolute) clobbers
    assert_ne!(
        clobber_a.counter("kills"),
        clobber_b.counter("kills"),
        "an absolute set would be order-dependent — proving why Increment matters"
    );
}
