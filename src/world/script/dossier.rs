//! The `dossier` script vocabulary (issue #1031).
//!
//! One verb, and it is the only way anything is ever written onto a fact sheet
//! that the world did not already imply:
//!
//! ```rhai
//! // A scan handler. The crew pointed something at the skyhook and read it back.
//! fn on_survey_complete(ctx) {
//!     ctx.dossier.append(#{
//!         subject:    "world.thin_margin.entity.skyway_hook.name",
//!         text:       "world.thin_margin.evidence.stress_fracture",
//!         provenance: "scan",
//!     });
//! }
//!
//! // A dialogue on_pick. Testimony: the foreman said it out loud.
//! fn on_press_foreman(ctx) {
//!     ctx.dossier.append(#{
//!         subject:    "world.thin_margin.entity.skyway_hook.name",
//!         text:       "world.thin_margin.evidence.foreman_admission",
//!         provenance: "dialogue",
//!     });
//! }
//! ```
//!
//! # Reading it back (issue #1036)
//!
//! ```rhai
//! // A negotiation node. This option EXISTS only because the crew went and
//! // looked — the tree reads the record itself, not a flag standing in for it.
//! fn committee_terms(ctx) {
//!     let responses = [ #{ text: "…", on_pick: "on_promise_passage" } ];
//!     if ctx.dossier.holds(#{ text: "world.thin_margin.evidence.ladder_b_file" }) {
//!         responses.push(#{ text: "…", on_pick: "on_show_record" });
//!     }
//!     #{ message: "…", responses: responses }
//! }
//! ```
//!
//! `holds` is a **read of state that already exists** — it registers nothing, so
//! it carries none of the census/snapshot obligations an append's entry does.
//! Two properties of it are worth knowing before authoring against it:
//!
//! * It matches on the finding's own `text` id (optionally narrowed to one
//!   `provenance`), and **not** on a subject. Script names a subject by its
//!   `[[entity]] name` while the log keys entries by UUID — the resolution hop
//!   belongs to the applier, which is the one place holding `name_to_uuid` — so a
//!   subject filter here would have to duplicate that map at a boundary that has
//!   no business holding it. A finding's `strings.csv` id already identifies the
//!   finding; what it is filed under is the panel's question, not the tree's.
//! * It reads the log as it stood when the call STARTED, so an append made
//!   earlier in the same handler is not visible to a `holds` after it. The
//!   append is buffered and resolved by the applier a step later, exactly like
//!   every other name-resolving effect; a handler that needs to branch on what
//!   it just wrote already knows it wrote it.
//!
//! This surface deliberately arrived a slice late. #1031 shipped write-only and
//! said a scenario branching on what the crew know should set a world flag
//! beside its append. #1036 is the beat that showed what that costs: the
//! negotiation must light up for *any* evidence route that reaches the same
//! finding — including the ones #1038/#1039 have not written yet — and a mirror
//! flag only lights for the routes that remembered to set it. Reading the record
//! is what makes the branch a property of the crew's file rather than of one
//! author's bookkeeping.
//!
//! # Why this is a handle and not another `ctx.effects` verb
//!
//! An entry is stamped with the tick the crew learned something on, and
//! [`EffectSink`] is a bare buffer with no clock. [`Commitments`] already solved
//! that: a per-call handle carrying the call's `now_tick`, built from the same
//! [`SchedClock`](super::schedule::SchedClock) a deferred effect is stamped
//! against, so a finding is stamped with the tick the handler actually ran on
//! rather than with the tick the applier happened to drain on. `ctx.dossier` is
//! that handle, and it sits beside `ctx.commitments` because the two are the same
//! kind of thing — the run's record of what happened, as opposed to a change to
//! the world.
//!
//! # The mutation still rides the ONE ordered buffer
//!
//! What the handle pushes is an ordinary
//! [`ActionCmd::RecordDossierEvidence`] onto the call's shared [`EffectSink`],
//! exactly as a resolution's campaign flag does — not a second `CallEffects`
//! field. Two things follow, both wanted:
//!
//! * An append keeps its authored position relative to `ctx.flags.*` writes and
//!   every other effect (the #981 ordering hazard), so a handler that appends a
//!   finding and then sets the flag a trigger watches happens in that order.
//! * The **subject name is resolved by the applier**, which is the one place
//!   holding `WorldContentRuntime::name_to_uuid` — the same hop
//!   `repair_infrastructure` and `order_hold` take. Script therefore names a
//!   subject by its `[[entity]] name`, like every other name-resolving verb,
//!   and never handles a UUID.
//!
//! And on the failure path the whole buffer is dropped (settled decision 10), so
//! a handler that raises after appending records nothing.
//!
//! # What raises, and what does not
//!
//! * A missing `subject` / `text`, or a `provenance` outside
//!   [`EvidenceProvenance::ALL`], **raises** — discarding the call. A mistyped
//!   provenance that silently defaulted would put a claim on a sheet under a
//!   source nobody authored, which is the one thing this vocabulary exists to
//!   make impossible.
//! * A subject name no entity in this world answers to is a **warned no-op** at
//!   the applier (issue #1031's AC2), never a panic: the name is resolved a tick
//!   later than it is written, against a world that may have moved on, and a
//!   scenario appending to something that has been destroyed should lose the
//!   entry rather than the run.
//! * Appending the same finding twice is a **silent no-op** in the store — see
//!   [`crate::dossier::evidence`] for why that is not the ledger's raise.
//!
//! [`ActionCmd::RecordDossierEvidence`]: crate::world::dispatch::ActionCmd::RecordDossierEvidence
//! [`Commitments`]: super::commitments::Commitments

use std::sync::Arc;

use rhai::{Engine, EvalAltResult, Map};

use crate::dossier::evidence::{EvidenceLog, EvidenceProvenance};
use crate::world::dispatch::ActionCmd;
use crate::world::script::effects::{map_str, raise, EffectSink};

/// The `dossier` custom type handed to a script call.
///
/// Cloneable like [`Commitments`](super::commitments::Commitments): the clone in
/// the context map and the one the host retains share the same read-only
/// snapshot of the run's findings, so `holds` costs a pointer copy per clone
/// rather than a second walk of the log.
#[derive(Clone)]
pub struct Dossier {
    /// The one ordered command buffer shared with the call's effects and flag
    /// writes, so an append lands where the author put it.
    sink: EffectSink,
    /// The call's clock — see the module docs on why this handle exists.
    now_tick: u64,
    /// What the crew already knew when this call started (issue #1036).
    ///
    /// Shared rather than cloned per `Dossier` clone, and immutable: nothing on
    /// this vocabulary edits a finding, so there is no read-after-write to give
    /// — see the module docs.
    known: Arc<EvidenceLog>,
}

impl Dossier {
    /// A fresh per-call view emitting onto the call's shared `sink`, stamping at
    /// `now_tick`, reading `base` for what the crew already found out.
    pub fn new(sink: EffectSink, now_tick: u64, base: &EvidenceLog) -> Self {
        Self {
            sink,
            now_tick,
            known: Arc::new(base.clone()),
        }
    }

    /// Whether this run has already learned `text` — optionally only through one
    /// `provenance`. Raises on a missing `text` or an unknown provenance, the
    /// same gate [`append`](Self::append) applies.
    fn holds(&self, spec: &Map) -> Result<bool, Box<EvalAltResult>> {
        let text = map_str(spec, "text").ok_or_else(|| {
            raise(
                "dossier.holds requires a string `text` (the strings.csv id of the \
                 finding to look for)"
                    .to_string(),
            )
        })?;
        // Optional, unlike on `append`: "do the crew know this at all" is the
        // ordinary question, and "do they know it from a scan rather than from
        // somebody's word" is the narrower one. A name outside the vocabulary
        // still raises, so a typo is never a silently-always-false branch.
        let provenance = match map_str(spec, "provenance") {
            Some(name) => Some(
                EvidenceProvenance::parse(&name)
                    .map_err(|e| raise(format!("dossier.holds: {e}")))?,
            ),
            None => None,
        };
        Ok(self.known.entries.iter().any(|entry| {
            if entry.text != text {
                return false;
            }
            match provenance {
                Some(wanted) => entry.provenance == wanted,
                None => true,
            }
        }))
    }

    /// Write one finding onto a subject's file. Raises on a missing field or an
    /// unknown provenance.
    fn append(&self, spec: &Map) -> Result<(), Box<EvalAltResult>> {
        let subject = map_str(spec, "subject").ok_or_else(|| {
            raise(
                "dossier.append requires a string `subject` (the [[entity]] name of \
                 the thing this was learned about)"
                    .to_string(),
            )
        })?;
        let text = map_str(spec, "text").ok_or_else(|| {
            raise(
                "dossier.append requires a string `text` (a strings.csv id, never English)"
                    .to_string(),
            )
        })?;
        let provenance = map_str(spec, "provenance").ok_or_else(|| {
            raise(
                "dossier.append requires a string `provenance` (how the crew learned it)"
                    .to_string(),
            )
        })?;
        // Parsed HERE rather than carried as a string and parsed by the applier,
        // for `game_over`'s reason: a typo is a raise the author sees at the beat
        // they wrote, and the buffered command carries a typed provenance nobody
        // downstream has to re-validate.
        let provenance = EvidenceProvenance::parse(&provenance)
            .map_err(|e| raise(format!("dossier.append: {e}")))?;
        self.sink.push(ActionCmd::RecordDossierEvidence {
            subject,
            text,
            provenance,
            gathered_at_tick: self.now_tick,
        });
        Ok(())
    }
}

/// Register the runtime `dossier` vocabulary on a runtime engine.
///
/// `append` takes a map rather than three positional strings, matching
/// `ctx.commitments.record(…)`: the fields are named at the call site, which is
/// what keeps `subject` and `text` from being silently swapped by an author who
/// is reading their own scenario rather than this file.
pub fn register_dossier(engine: &mut Engine) {
    engine.register_type_with_name::<Dossier>("Dossier");
    engine.register_fn(
        "append",
        |d: &mut Dossier, spec: Map| -> Result<(), Box<EvalAltResult>> { d.append(&spec) },
    );
    // The read (issue #1036), a map for `append`'s reason: the one required key
    // is named at the call site, and the optional `provenance` narrows it
    // without a second overload whose argument order an author has to remember.
    engine.register_fn(
        "holds",
        |d: &mut Dossier, spec: Map| -> Result<bool, Box<EvalAltResult>> { d.holds(&spec) },
    );
}

#[cfg(test)]
mod tests {
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
}
