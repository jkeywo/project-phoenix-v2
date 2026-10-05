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

use rhai::{EvalAltResult, Map};

use crate::dossier::evidence::{EvidenceLog, EvidenceProvenance};
use crate::world::dispatch::ActionCmd;
use crate::world::script::effects::{map_str, raise, EffectSink};
use crate::world::script::registry::{host_fn, HostRegistry};

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
pub(crate) fn register_dossier(engine: &mut HostRegistry) {
    engine.register_type_with_name::<Dossier>("Dossier");
    host_fn!(
        engine,
        "append",
        receiver = "dossier",
        category = "effect",
        params = ["spec"],
        summary = "Write one finding onto a subject's dossier: \
                  `#{ subject, text, provenance }`, where `subject` is an \
                  `[[entity]] id`, `text` is a strings.csv id and `provenance` is \
                  one of scan / dialogue / records / briefing. Appending the same \
                  finding twice keeps the first stamp; an unknown subject is a \
                  warned no-op.",
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
#[path = "dossier_tests.rs"]
mod tests;
