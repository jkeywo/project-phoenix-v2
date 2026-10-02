use super::*;
use crate::world::script::effects::BufferedEffect;
use crate::world::script::engine::runtime_engine;
use rhai::Dynamic;

const TICK: u64 = 600;

/// Run `source`'s `on_x` and return the commands it emitted in authored
/// order. `flags` shares the one sink exactly as the live host wires it,
/// which is what lets a test assert an append lands BETWEEN a handler's own
/// flag writes.
fn run(source: &str) -> Vec<ActionCmd> {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let sink = EffectSink::new();
    let mut ctx = Map::new();
    ctx.insert(
        "flags".into(),
        Dynamic::from(crate::world::script::flags::Flags::new(
            &crate::world::flags::FlagStore::default(),
            sink.clone(),
        )),
    );
    ctx.insert(
        "dossier".into(),
        Dynamic::from(Dossier::new(sink.clone(), TICK, &EvidenceLog::default())),
    );
    // This vocabulary communicates entirely through the effect buffer, so a
    // handler's return value is nothing to read — unlike a dialogue node fn.
    let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("runs");
    sink.take()
        .into_iter()
        .map(|e| match e {
            BufferedEffect::Cmd(cmd) => cmd,
            other => panic!("expected a resolved command, got {other:?}"),
        })
        .collect()
}

fn err(source: &str) -> String {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let mut ctx = Map::new();
    ctx.insert(
        "dossier".into(),
        Dynamic::from(Dossier::new(
            EffectSink::new(),
            TICK,
            &EvidenceLog::default(),
        )),
    );
    let e = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx)
        .expect_err("this call should raise");
    format!("{e}")
}

/// Run `source`'s `on_x` against a log the crew already hold, returning what
/// it evaluated to — the shape a dialogue node fn gating an option takes.
fn ask(source: &str, base: &EvidenceLog) -> Dynamic {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let mut ctx = Map::new();
    ctx.insert(
        "dossier".into(),
        Dynamic::from(Dossier::new(EffectSink::new(), TICK, base)),
    );
    vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("runs")
}

fn gathered() -> EvidenceLog {
    let mut log = EvidenceLog::default();
    log.append(
        "depot-b",
        "world.probe.evidence.maintenance_file",
        EvidenceProvenance::Records,
        120,
    );
    log
}

/// **AC1/AC2.** One append buffers one command carrying the subject NAME,
/// the text id, the typed provenance and the call's tick.
#[test]
fn an_append_buffers_the_subject_name_the_text_the_provenance_and_the_tick() {
    let cmds = run(r#"fn on_x(ctx) {
                 ctx.dossier.append(#{
                     subject: "skyway_hook",
                     text: "world.probe.evidence.fracture",
                     provenance: "scan",
                 });
               }"#);
    assert_eq!(
        cmds,
        vec![ActionCmd::RecordDossierEvidence {
            subject: "skyway_hook".into(),
            text: "world.probe.evidence.fracture".into(),
            provenance: EvidenceProvenance::Scan,
            gathered_at_tick: TICK,
        }],
        "the NAME is buffered unresolved — the applier holds name_to_uuid"
    );
}

/// Every provenance the vocabulary names is reachable from script under its
/// own spelling, so a new one cannot ship with no way to author it.
#[test]
fn every_provenance_is_authorable_under_its_own_name() {
    for provenance in EvidenceProvenance::ALL {
        let cmds = run(&format!(
            r#"fn on_x(ctx) {{
                     ctx.dossier.append(#{{ subject: "s", text: "t", provenance: "{}" }});
                   }}"#,
            provenance.as_str()
        ));
        match &cmds[..] {
            [ActionCmd::RecordDossierEvidence {
                provenance: got, ..
            }] => {
                assert_eq!(*got, provenance)
            }
            other => panic!("expected one append, got {other:?}"),
        }
    }
}

/// The ordering property (issue #981 hazard 2) applied to an append: a
/// finding lands where the author put it, between the call's own flag
/// writes, because it rides the same one buffer.
#[test]
fn an_append_is_emitted_in_authored_order_beside_the_calls_flag_writes() {
    let cmds = run(r#"fn on_x(ctx) {
                 ctx.flags.increment("before", 1);
                 ctx.dossier.append(#{ subject: "s", text: "t", provenance: "records" });
                 ctx.flags.increment("after", 1);
               }"#);
    let shape: Vec<&str> = cmds
        .iter()
        .map(|c| match c {
            ActionCmd::MutateFlag { name, .. } => name.as_str(),
            ActionCmd::RecordDossierEvidence { text, .. } => text.as_str(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(shape, vec!["before", "t", "after"]);
}

/// Two appends in one call are two commands, in order. The store settles
/// whether the second is a duplicate — this boundary does not second-guess
/// it, exactly as the commitments surface hands both mutations over.
#[test]
fn two_appends_in_one_call_buffer_two_commands_in_order() {
    let cmds = run(r#"fn on_x(ctx) {
                 ctx.dossier.append(#{ subject: "s", text: "a", provenance: "scan" });
                 ctx.dossier.append(#{ subject: "s", text: "a", provenance: "scan" });
               }"#);
    assert_eq!(cmds.len(), 2);
}

/// **The provenance gate.** A name outside the vocabulary raises, naming
/// what was expected — never a silent default onto one of the four.
#[test]
fn an_unknown_provenance_raises_and_says_what_was_expected() {
    let message = err(r#"fn on_x(ctx) {
                 ctx.dossier.append(#{ subject: "s", text: "t", provenance: "hearsay" });
               }"#);
    assert!(message.contains("hearsay"), "{message}");
    assert!(
        message.contains("scan, dialogue, records, briefing"),
        "{message}"
    );
}

// ── The read (issue #1036): a tree branches on the crew's own file ───────

/// **#1036's evidence branch.** The same node fn, asked twice: an option
/// that exists only because the crew went and looked.
#[test]
fn a_node_fn_reads_whether_the_crew_have_gathered_a_finding() {
    let source = r#"fn on_x(ctx) {
                 ctx.dossier.holds(#{ text: "world.probe.evidence.maintenance_file" })
               }"#;
    assert!(
        !ask(source, &EvidenceLog::default())
            .as_bool()
            .expect("a bool"),
        "a crew who never looked hold nothing"
    );
    assert!(ask(source, &gathered()).as_bool().expect("a bool"));
}

/// The optional narrowing: "do they know it" and "do they know it from a
/// records comparison" are different questions, and a finding learned
/// another way answers only the first.
#[test]
fn a_provenance_narrows_the_read_without_being_required() {
    let by_records = r#"fn on_x(ctx) {
                 ctx.dossier.holds(#{
                     text: "world.probe.evidence.maintenance_file",
                     provenance: "records",
                 })
               }"#;
    let by_scan = r#"fn on_x(ctx) {
                 ctx.dossier.holds(#{
                     text: "world.probe.evidence.maintenance_file",
                     provenance: "scan",
                 })
               }"#;
    assert!(ask(by_records, &gathered()).as_bool().expect("a bool"));
    assert!(
        !ask(by_scan, &gathered()).as_bool().expect("a bool"),
        "the crew have the file, but not off a sensor"
    );
}

/// The snapshot is the log as it stood at CALL START, so an append made
/// earlier in the same handler is not visible to a later read — the append
/// is buffered and resolved by the applier a step later, like every other
/// name-resolving effect.
#[test]
fn a_read_after_an_append_in_the_same_call_does_not_see_it() {
    let engine = runtime_engine();
    let ast = engine
        .compile(
            r#"fn on_x(ctx) {
                     ctx.dossier.append(#{
                         subject: "depot-b",
                         text: "world.probe.evidence.fresh",
                         provenance: "scan",
                     });
                     ctx.dossier.holds(#{ text: "world.probe.evidence.fresh" })
                   }"#,
        )
        .expect("compiles");
    let sink = EffectSink::new();
    let mut ctx = Map::new();
    ctx.insert(
        "dossier".into(),
        Dynamic::from(Dossier::new(sink.clone(), TICK, &EvidenceLog::default())),
    );
    let value = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("runs");
    assert!(!value.as_bool().expect("a bool"));
    assert_eq!(sink.take().len(), 1, "the append itself still buffered");
}

/// A `holds` that raises is the same kind of authoring error an `append`
/// that raises is: a mistyped provenance must never read as a quietly-false
/// branch that hides an option the crew earned.
#[test]
fn a_malformed_read_raises_rather_than_answering_false() {
    for (source, want) in [
        (
            r#"fn on_x(ctx) { ctx.dossier.holds(#{ provenance: "scan" }); }"#,
            "`text`",
        ),
        (
            r#"fn on_x(ctx) { ctx.dossier.holds(#{ text: "t", provenance: "hearsay" }); }"#,
            "hearsay",
        ),
    ] {
        let message = err(source);
        assert!(message.contains(want), "{message}");
    }
}

#[test]
fn an_append_missing_a_required_field_raises() {
    for (source, want) in [
        (
            r#"fn on_x(ctx) { ctx.dossier.append(#{ text: "t", provenance: "scan" }); }"#,
            "`subject`",
        ),
        (
            r#"fn on_x(ctx) { ctx.dossier.append(#{ subject: "s", provenance: "scan" }); }"#,
            "`text`",
        ),
        (
            r#"fn on_x(ctx) { ctx.dossier.append(#{ subject: "s", text: "t" }); }"#,
            "`provenance`",
        ),
    ] {
        let message = err(source);
        assert!(
            message.contains(want),
            "the raise names the missing field {want}: {message}"
        );
    }
}
