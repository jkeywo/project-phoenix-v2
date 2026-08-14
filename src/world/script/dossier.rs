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

use rhai::{Engine, EvalAltResult, Map};

use crate::dossier::evidence::EvidenceProvenance;
use crate::world::dispatch::ActionCmd;
use crate::world::script::effects::{map_str, raise, EffectSink};

/// The `dossier` custom type handed to a script call.
///
/// Cloneable like [`Commitments`](super::commitments::Commitments), and for
/// less: it holds no snapshot at all, because nothing on this vocabulary reads.
/// A scenario that wants to branch on what the crew already know branches on a
/// world flag it set beside the append — one line, in the language the author is
/// already writing, and the same answer #1029 gave to `when:` on a dialogue
/// response.
#[derive(Clone)]
pub struct Dossier {
    /// The one ordered command buffer shared with the call's effects and flag
    /// writes, so an append lands where the author put it.
    sink: EffectSink,
    /// The call's clock — see the module docs on why this handle exists.
    now_tick: u64,
}

impl Dossier {
    /// A fresh per-call view emitting onto the call's shared `sink`, stamping at
    /// `now_tick`.
    pub fn new(sink: EffectSink, now_tick: u64) -> Self {
        Self { sink, now_tick }
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
            Dynamic::from(Dossier::new(sink.clone(), TICK)),
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
            Dynamic::from(Dossier::new(EffectSink::new(), TICK)),
        );
        let e = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx)
            .expect_err("this call should raise");
        format!("{e}")
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
