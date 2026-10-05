//! The `commitments` script vocabulary (issue #1029).
//!
//! One handle on the runtime engine, read/write like
//! [`Deadlines`](super::deadlines::Deadlines), reached from any script call —
//! and, most of all, from a dialogue node fn and its `on_pick`:
//!
//! ```rhai
//! // The negotiation beat. Picking this is giving your word.
//! fn on_promise_passage(ctx) {
//!     ctx.commitments.record(#{
//!         id:            "safe_passage",
//!         made_to:       "skyway_strike_committee",
//!         terms:         "world.thin_margin.commitment.safe_passage.terms",
//!         resolves_when: "world.thin_margin.commitment.safe_passage.resolves",
//!     });
//!     #{ message: "...", responses: [] }
//! }
//!
//! // A later node. The option EXISTS only because the promise is on the books.
//! fn committee_calls_back(ctx) {
//!     let responses = [ #{ text: "Nothing to report.", on_pick: "on_stall" } ];
//!     if ctx.commitments.state("safe_passage") == "open" {
//!         responses.push(#{ text: "Your people are through.", on_pick: "on_honour" });
//!     }
//!     #{ message: "...", responses: responses }
//! }
//!
//! fn on_honour(ctx) { ctx.commitments.keep("safe_passage"); }
//! ```
//!
//! # Gating is ordinary control flow, and that is the design
//!
//! There is no `when:` field on a dialogue response and this slice does not add
//! one. A [`ScriptDialogueResponse`](super::comms::ScriptDialogueResponse) is a
//! `text` and an `on_pick`, and a node fn is a *function* — so "this option
//! appears only if the captain promised" is an `if` around a `push`, in the
//! language the author is already writing. A declarative predicate would need
//! its own expression grammar, its own evaluator and its own load-time
//! validation to say something Rhai already says in one line.
//!
//! # Why `break_promise` and not `break`
//!
//! `break` is a Rhai keyword (`rhai::tokenizer`: `("break", Token::Break)`), so
//! `ctx.commitments.break("x")` is a parse error, not a method call. The verb
//! pair is therefore `keep` / `break_promise`. The asymmetry is deliberate and
//! is preferred to renaming *both* halves away from the domain words the ledger
//! itself uses — [`CommitmentState`](crate::world::commitments::CommitmentState)
//! reads `kept` and `broken`, and a script that says `keep` should be settling
//! the thing that reads `kept`.
//!
//! # The campaign flag rides the ordered effect buffer
//!
//! Resolving does two separate things, and only one of them is new machinery:
//!
//! * The **ledger move** buffers a [`CommitmentChange`] for the host to replay,
//!   exactly as a deadline slip does.
//! * The **campaign flag** — `commitment.<id>.kept` / `.broken` — is pushed onto
//!   the call's shared [`EffectSink`] as an ordinary
//!   [`ActionCmd::MutateFlag`], the same command `ctx.flags.x = 1` emits. It is
//!   applied by the applier that already exists, `push_flag_transition` emits
//!   its `FlagSet` on the boolean flip, and an `on_flag_set` trigger authored
//!   against it chains. Nothing about the consequence of a promise is special.
//!
//! Both are dropped whole on the failure path with the rest of the call's
//! buffers (settled decision 10), so a handler that raises after resolving a
//! promise neither moves the ledger nor sets the flag.
//!
//! # Duplicates raise
//!
//! `record` on an id already on the books raises, per issue #1029's AC1 — and
//! raising is what makes that honest, because it drops the call's whole buffer
//! rather than leaving half a negotiation applied. A scenario that can reach the
//! same promise twice guards it with
//! `ctx.commitments.state("id") == "unknown"`.

use std::sync::{Arc, Mutex};

use rhai::{EvalAltResult, ImmutableString, Map};

use crate::world::commitments::{
    CommitmentChange, CommitmentLedger, CommitmentMutation, CommitmentOutcome,
};
use crate::world::dispatch::{ActionCmd, FlagMutation};
use crate::world::script::effects::{map_str, raise, EffectSink};
use crate::world::script::registry::HostRegistry;

/// The `commitments` custom type handed to a script call.
///
/// Cloneable and interior-mutable like [`Flags`](super::flags::Flags): the clone
/// in the context map and the clone the host retains share one snapshot and one
/// change buffer, so the host observes every mutation the script authored, and a
/// `state` read *after* a `keep` in the same call sees the promise settled.
///
/// `now_tick` comes from the call's [`SchedClock`](super::schedule::SchedClock)
/// — the same clock a deferred effect is stamped against — so a promise is
/// stamped with the tick the handler actually ran on.
#[derive(Clone)]
pub struct Commitments {
    /// A snapshot of the live ledger, mutated in place for read-after-write.
    /// Discarded when the call ends; the real ledger is moved by the adapter
    /// replaying `changes`.
    snapshot: Arc<Mutex<CommitmentLedger>>,
    /// The mutations, in authored order, for the host to drain.
    changes: Arc<Mutex<Vec<CommitmentChange>>>,
    /// The one ordered command buffer shared with the call's effects — where a
    /// resolution's campaign flag is emitted, in authored order, interleaved
    /// with everything else the handler did.
    sink: EffectSink,
    now_tick: u64,
}

impl Commitments {
    /// A fresh per-call view over a snapshot of `base`, stamping at `now_tick`
    /// and emitting campaign flags onto the call's shared `sink`.
    pub fn new(base: &CommitmentLedger, sink: EffectSink, now_tick: u64) -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(base.clone())),
            changes: Arc::new(Mutex::new(Vec::new())),
            sink,
            now_tick,
        }
    }

    /// `"open"` / `"kept"` / `"broken"`, or `"unknown"` for a promise this run
    /// never made — see
    /// [`CommitmentLedger::state_of`](crate::world::commitments::CommitmentLedger::state_of).
    fn state(&self, id: &str) -> String {
        self.snapshot
            .lock()
            .expect("commitment snapshot lock")
            .state_of(id)
            .to_string()
    }

    /// Write a promise onto the books. Raises on a missing field or on an id
    /// already recorded.
    fn record(&self, spec: &Map) -> Result<(), Box<EvalAltResult>> {
        let id = map_str(spec, "id")
            .ok_or_else(|| raise("commitments.record requires a string `id`".to_string()))?;
        let made_to = map_str(spec, "made_to").ok_or_else(|| {
            raise(
                "commitments.record requires a string `made_to` (the party the promise \
                 is made to)"
                    .to_string(),
            )
        })?;
        let terms = map_str(spec, "terms").ok_or_else(|| {
            raise(
                "commitments.record requires a string `terms` (a strings.csv id, never \
                 English)"
                    .to_string(),
            )
        })?;
        // `resolves_when` is optional in the same sense a deadline's `label` is:
        // a promise the mission keeps entirely to itself owes the crew no
        // statement of what would settle it.
        let resolves_when = map_str(spec, "resolves_when").unwrap_or_default();
        self.push(
            &id,
            CommitmentMutation::Record {
                made_to,
                terms,
                resolves_when,
            },
        )
    }

    /// Settle `id` one way or the other, emitting its campaign flag.
    fn resolve(&self, id: &str, outcome: CommitmentOutcome) -> Result<(), Box<EvalAltResult>> {
        self.push(id, CommitmentMutation::Resolve { outcome })
    }

    /// Record a mutation, apply it to the snapshot so the rest of this call
    /// reads what it just wrote, and emit any campaign flag it asks for.
    ///
    /// The snapshot's answer is what decides whether a flag is emitted, so a
    /// second `keep` of an already-kept promise writes nothing — the same
    /// no-op the live ledger will reach when the adapter replays it.
    fn push(&self, id: &str, mutation: CommitmentMutation) -> Result<(), Box<EvalAltResult>> {
        let change = CommitmentChange {
            id: id.to_string(),
            mutation,
        };
        let flag = self
            .snapshot
            .lock()
            .expect("commitment snapshot lock")
            .apply(&change, self.now_tick)
            .map_err(|dup| raise(dup.to_string()))?;
        if let Some(name) = flag {
            self.sink.push(ActionCmd::MutateFlag {
                target_layer: None,
                name,
                mutation: FlagMutation::SetValue(1),
            });
        }
        self.changes
            .lock()
            .expect("commitment changes lock")
            .push(change);
        Ok(())
    }

    /// Drain the buffered mutations. Called by the host on the success path
    /// only — on the failure path the buffer is dropped whole with the rest of
    /// the call's effects.
    pub fn take_changes(&self) -> Vec<CommitmentChange> {
        std::mem::take(&mut *self.changes.lock().expect("commitment changes lock"))
    }
}

/// Register the runtime `commitments` vocabulary on a runtime engine.
///
/// `record` takes a map rather than four positional strings, matching
/// `ctx.effects.open_comms(#{…})`: the fields are named at the call site, which
/// is what keeps `made_to` and `terms` from being silently swapped by an author
/// who is reading their own scenario rather than this file.
pub(crate) fn register_commitments(engine: &mut HostRegistry) {
    engine.register_type_with_name::<Commitments>("Commitments");

    engine.register_fn(
        "state",
        |c: &mut Commitments, id: ImmutableString| -> String { c.state(&id) },
    );
    engine.register_fn(
        "record",
        |c: &mut Commitments, spec: Map| -> Result<(), Box<EvalAltResult>> { c.record(&spec) },
    );
    engine.register_fn(
        "keep",
        |c: &mut Commitments, id: ImmutableString| -> Result<(), Box<EvalAltResult>> {
            c.resolve(&id, CommitmentOutcome::Kept)
        },
    );
    // Not `break` — see the module docs; it is a Rhai keyword.
    engine.register_fn(
        "break_promise",
        |c: &mut Commitments, id: ImmutableString| -> Result<(), Box<EvalAltResult>> {
            c.resolve(&id, CommitmentOutcome::Broken)
        },
    );
}

#[cfg(test)]
#[path = "commitments_tests.rs"]
mod tests;
