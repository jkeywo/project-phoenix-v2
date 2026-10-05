use super::*;
use crate::world::commitments::CommitmentState;
use crate::world::script::effects::BufferedEffect;
use crate::world::script::engine::runtime_engine;
use rhai::Dynamic;

const TICK: u64 = 600;

fn ledger() -> CommitmentLedger {
    let mut ledger = CommitmentLedger::default();
    ledger
        .record("safe_passage", "committee", "t.passage", "w.passage", 120)
        .expect("seeds");
    ledger
}

/// Run `source`'s `on_x` against a snapshot of `base`, returning the
/// buffered mutations, the commands it emitted in authored order, and what
/// it returned.
///
/// `flags` shares the one sink, exactly as the live host wires it — which is
/// what lets a test assert that a resolution's campaign flag lands *between*
/// the handler's own flag writes rather than after them.
fn run(source: &str, base: &CommitmentLedger) -> (Vec<CommitmentChange>, Vec<ActionCmd>, Dynamic) {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let sink = EffectSink::new();
    let commitments = Commitments::new(base, sink.clone(), TICK);
    let mut ctx = Map::new();
    ctx.insert(
        "flags".into(),
        Dynamic::from(crate::world::script::flags::Flags::new(
            &crate::world::flags::FlagStore::default(),
            sink.clone(),
        )),
    );
    ctx.insert("commitments".into(), Dynamic::from(commitments.clone()));
    let value = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("runs");
    let cmds = sink
        .take()
        .into_iter()
        .map(|e| match e {
            BufferedEffect::Cmd(cmd) => cmd,
            other => panic!("expected a resolved command, got {other:?}"),
        })
        .collect();
    (commitments.take_changes(), cmds, value)
}

// ── AC4: script records and resolves through the host surface ────────────

#[test]
fn a_dialogue_pick_records_a_promise_with_its_party_and_terms() {
    let (changes, cmds, _) = run(
        r#"fn on_x(ctx) {
                 ctx.commitments.record(#{
                     id: "surface_records",
                     made_to: "committee",
                     terms: "t.records",
                     resolves_when: "w.records",
                 });
               }"#,
        &ledger(),
    );
    assert_eq!(
        changes,
        vec![CommitmentChange {
            id: "surface_records".into(),
            mutation: CommitmentMutation::Record {
                made_to: "committee".into(),
                terms: "t.records".into(),
                resolves_when: "w.records".into(),
            },
        }]
    );
    assert!(
        cmds.is_empty(),
        "MAKING a promise writes no campaign flag — only resolving one does"
    );
}

#[test]
fn keeping_a_promise_emits_its_campaign_flag_as_an_ordinary_flag_write() {
    let (changes, cmds, _) = run(
        r#"fn on_x(ctx) { ctx.commitments.keep("safe_passage"); }"#,
        &ledger(),
    );
    assert_eq!(
        changes,
        vec![CommitmentChange {
            id: "safe_passage".into(),
            mutation: CommitmentMutation::Resolve {
                outcome: CommitmentOutcome::Kept
            },
        }]
    );
    assert_eq!(
        cmds,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "commitment.safe_passage.kept".into(),
            mutation: FlagMutation::SetValue(1),
        }],
        "the consequence of a promise is a world flag like any other, so an \
             on_flag_set trigger chains without this vocabulary knowing triggers exist"
    );
}

#[test]
fn breaking_a_promise_emits_the_other_flag() {
    let (_, cmds, _) = run(
        r#"fn on_x(ctx) { ctx.commitments.break_promise("safe_passage"); }"#,
        &ledger(),
    );
    assert_eq!(
        cmds,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "commitment.safe_passage.broken".into(),
            mutation: FlagMutation::SetValue(1),
        }]
    );
}

#[test]
fn a_campaign_flag_is_emitted_in_authored_order_beside_the_calls_other_writes() {
    // The interleaving property `Flags` establishes (issue #981 hazard 2),
    // applied to a resolution: a handler that resolves a promise and then
    // sets a flag of its own emits them in that order.
    let (_, cmds, _) = run(
        r#"fn on_x(ctx) {
                 ctx.flags.increment("before", 1);
                 ctx.commitments.keep("safe_passage");
                 ctx.flags.increment("after", 1);
               }"#,
        &ledger(),
    );
    let names: Vec<&str> = cmds
        .iter()
        .map(|c| match c {
            ActionCmd::MutateFlag { name, .. } => name.as_str(),
            other => panic!("expected a flag write, got {other:?}"),
        })
        .collect();
    assert_eq!(
        names,
        vec!["before", "commitment.safe_passage.kept", "after"],
        "the resolution's flag sits where the author put it"
    );
}

// ── AC5's mechanism: state is readable, so an option can be gated on it ──

#[test]
fn a_node_fn_reads_the_state_it_gates_an_option_on() {
    let source = r#"fn on_x(ctx) { ctx.commitments.state("safe_passage") }"#;
    let (_, _, value) = run(source, &ledger());
    assert_eq!(value.into_string().expect("a string"), "open");

    let (_, _, value) = run(source, &CommitmentLedger::default());
    assert_eq!(
        value.into_string().expect("a string"),
        "unknown",
        "a promise that was never made reads as unknown, not as broken"
    );

    let mut kept = ledger();
    kept.resolve("safe_passage", CommitmentOutcome::Kept, 300);
    let (_, _, value) = run(source, &kept);
    assert_eq!(value.into_string().expect("a string"), "kept");
}

#[test]
fn a_read_after_a_resolve_sees_the_settled_promise_within_the_same_call() {
    let (_, _, value) = run(
        r#"fn on_x(ctx) {
                 ctx.commitments.keep("safe_passage");
                 ctx.commitments.state("safe_passage")
               }"#,
        &ledger(),
    );
    assert_eq!(value.into_string().expect("a string"), "kept");

    let (_, _, value) = run(
        r#"fn on_x(ctx) {
                 ctx.commitments.record(#{ id: "new_one", made_to: "p", terms: "t" });
                 ctx.commitments.state("new_one")
               }"#,
        &ledger(),
    );
    assert_eq!(
        value.into_string().expect("a string"),
        "open",
        "a promise made earlier in this handler is already on the books for the \
             rest of it"
    );
}

#[test]
fn resolving_the_same_promise_twice_in_one_call_writes_one_flag() {
    let (changes, cmds, _) = run(
        r#"fn on_x(ctx) {
                 ctx.commitments.keep("safe_passage");
                 ctx.commitments.keep("safe_passage");
               }"#,
        &ledger(),
    );
    assert_eq!(
        cmds.len(),
        1,
        "the snapshot's no-op decides the emission, so the second keep writes nothing"
    );
    assert_eq!(
        changes.len(),
        2,
        "both mutations are still handed to the host — the live ledger reaches \
             the same no-op the snapshot did"
    );
}

// ── AC1: duplicates are an error, and raising drops the whole call ───────

#[test]
fn recording_a_duplicate_id_raises() {
    let engine = runtime_engine();
    let ast = engine
        .compile(
            r#"fn on_x(ctx) {
                     ctx.commitments.record(#{ id: "safe_passage", made_to: "p", terms: "t" });
                   }"#,
        )
        .expect("compiles");
    let sink = EffectSink::new();
    let commitments = Commitments::new(&ledger(), sink.clone(), TICK);
    let mut ctx = Map::new();
    ctx.insert("commitments".into(), Dynamic::from(commitments.clone()));
    let err = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx)
        .expect_err("a duplicate id is an error, not an overwrite");
    assert!(
        format!("{err}").contains("already on the books"),
        "the raise names the problem to the author: {err}"
    );
}

#[test]
fn a_record_missing_a_required_field_raises() {
    let engine = runtime_engine();
    for (source, want) in [
        (
            r#"fn on_x(ctx) { ctx.commitments.record(#{ made_to: "p", terms: "t" }); }"#,
            "`id`",
        ),
        (
            r#"fn on_x(ctx) { ctx.commitments.record(#{ id: "x", terms: "t" }); }"#,
            "`made_to`",
        ),
        (
            r#"fn on_x(ctx) { ctx.commitments.record(#{ id: "x", made_to: "p" }); }"#,
            "`terms`",
        ),
    ] {
        let ast = engine.compile(source).expect("compiles");
        let commitments = Commitments::new(&ledger(), EffectSink::new(), TICK);
        let mut ctx = Map::new();
        ctx.insert("commitments".into(), Dynamic::from(commitments));
        let err = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx)
            .expect_err("a malformed record map raises");
        assert!(
            format!("{err}").contains(want),
            "the raise names the missing field {want}: {err}"
        );
    }
}

#[test]
fn the_live_ledger_is_untouched_by_a_call() {
    // The call mutates its own snapshot; only the adapter replaying the
    // drained changes moves the real ledger.
    let live = ledger();
    let (changes, _, _) = run(
        r#"fn on_x(ctx) { ctx.commitments.keep("safe_passage"); }"#,
        &live,
    );
    assert_eq!(
        live.get("safe_passage").expect("still there").state,
        CommitmentState::Open,
        "a script call never writes the live ledger directly"
    );
    assert_eq!(changes.len(), 1);
}

#[test]
fn taking_the_changes_twice_yields_them_once() {
    let commitments = Commitments::new(&ledger(), EffectSink::new(), TICK);
    commitments
        .resolve("safe_passage", CommitmentOutcome::Kept)
        .expect("resolves");
    assert_eq!(commitments.take_changes().len(), 1);
    assert!(
        commitments.take_changes().is_empty(),
        "a drained buffer cannot replay its mutations"
    );
}
