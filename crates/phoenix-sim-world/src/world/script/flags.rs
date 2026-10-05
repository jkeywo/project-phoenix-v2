//! The sugared `flags` script type (issue #979 M1; issue #981 M3 hazard fixes).
//!
//! `FlagStore` stays the only mutable world state a script can touch, reached
//! through a `flags` custom type that reads like a struct for absolute writes and
//! takes an explicit method for increments:
//!
//! ```rhai
//! fn on_armed(ctx) {
//!     ctx.flags.increment("score", 50);   // compose a counter (drains Increment)
//!     ctx.flags.alarm = 1;                 // set a boolean, absolute (drains SetValue)
//! }
//! ```
//!
//! `flags.alarm` is not a fixed property — Rhai resolves an unknown property on
//! a custom type by falling back to an **indexer**, so a single registered
//! indexer get/set serves every flag name for reads and absolute assignment
//! (`chaining.rs`: "Try an indexer if property does not exist").
//!
//! # Increment vs. absolute, and why `+=` is not it (issue #981 hazard 1)
//!
//! An M1 `flags.x += 50` drained as an **absolute** `SetValue(final)`, which
//! clobbers a concurrent TOML `increment_flag` applied out of order — the +N and
//! the scripted write race, and one order silently loses the +N. The fix is to
//! drain an authored increment as [`FlagMutation::Increment`] so the two
//! *compose* regardless of order.
//!
//! That distinction cannot be recovered from `+=`: Rhai desugars `flags.x += n`
//! on an indexer to *get-then-set*, so the custom type only ever sees a set of
//! the final value — indistinguishable from `flags.x = final`
//! (`rhai/src/eval/chaining.rs`, the `Property … op=` arm falls back to
//! `call_indexer_get` then `call_indexer_set`). So increments take an explicit
//! verb, [`increment`](Flags::increment) (`flags.increment("x", n)`), which is
//! unambiguous; `flags.x = v` stays absolute. `+=` on a flag is **not** a safe
//! increment: it still parses, but it silently degrades to an absolute
//! `SetValue` and so re-introduces exactly the clobber hazard above — do not use
//! it for counters. Until a load-time lint rejects it (tracked follow-up), the
//! only guard is this documentation: an increment must be said as an increment.
//!
//! # Scratch overlay + ordered emission (issue #981 hazard 2)
//!
//! Writes do not touch the real store during a call. Two things happen per write:
//!
//! * The new value lands in a per-call **overlay** over a snapshot of the live
//!   [`FlagStore`], so read-after-write is correct *within one call*
//!   (`flags.score` after an `increment` reads back the new value).
//! * The corresponding `ActionCmd::MutateFlag` is pushed **immediately** onto the
//!   *shared* [`EffectSink`](super::effects::EffectSink) the call's effects use —
//!   so a flag write authored *before* an effect is emitted before it, preserving
//!   authored interleaving. (M1 appended all flag writes after all effects, which
//!   an immediate/scheduled effect that reads the flag would then observe in the
//!   wrong order.) On the failure path the sink is dropped whole with the rest of
//!   the call's effects (settled decision 10), so nothing is applied.
//!
//! # Layer scope (issue #1045)
//!
//! A name is resolved against the LAYER CHAIN of the handler that wrote it, on
//! both sides: [`Flags::with_chain`] snapshots that chain for reads, and
//! `world::server::scope_scripted_flag_write` resolves the emitted
//! `MutateFlag`'s target the same way. Unprefixed means "my own scope" and does
//! not fall outward; each `parent:` steps one layer out; past the root reads `0`
//! and drops the write. That is exactly what `flag(..)` in a `when` predicate
//! already did, so a handler and its own trigger conditions cannot disagree.
//! A base-world handler has a one-entry chain, so everything above collapses to
//! the base store it always used.
//!
//! [`FlagStore`]: crate::world::flags::FlagStore
//! [`FlagMutation::Increment`]: crate::world::dispatch::FlagMutation::Increment

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rhai::ImmutableString;

use crate::world::dispatch::{ActionCmd, FlagMutation};
use crate::world::flags::FlagStore;
use crate::world::script::effects::EffectSink;
use crate::world::script::registry::{host_fn, HostRegistry};

/// The per-call flag view: a snapshot of the live store CHAIN plus a scratch
/// overlay giving read-after-write.
#[derive(Clone, Default)]
struct FlagOverlay {
    /// Snapshot of the layered store chain at call start, innermost first and
    /// terminating at the base world — the same walk `layered_flag_chain` builds
    /// for a `when` predicate. Reads of an un-overlaid name fall through to here.
    ///
    /// A chain rather than one store since issue #1045: a layer's handler writes
    /// into its OWN store (`scope_scripted_flag_write`), so it has to read from
    /// there too or `ctx.flags.armed = 1` in one handler would be invisible to
    /// `if ctx.flags.armed == 1` in the next. A base-origin handler gets a
    /// one-entry chain and behaves exactly as it always did.
    chain: Vec<FlagStore>,
    /// Concrete post-write values, for read-after-write within the call. Not
    /// drained — mutations are emitted eagerly onto the shared effect sink.
    ///
    /// Keyed by the name AS AUTHORED, `parent:` prefixes and all, which is what
    /// the write emits and what a later read in the same call spells — so
    /// read-after-write matches whatever scope the write resolved to.
    overlay: BTreeMap<String, i64>,
}

/// The `flags` custom type handed to a script call.
///
/// Cloneable and interior-mutable (like [`EffectSink`]): the clone in the context
/// map and the clone the host retains share one overlay. Holds a clone of the
/// call's shared effect sink so a flag mutation is emitted in authored order,
/// interleaved with effects (issue #981 hazard 2).
#[derive(Clone)]
pub struct Flags {
    overlay: Arc<Mutex<FlagOverlay>>,
    /// The one ordered command buffer shared with the call's effects.
    sink: EffectSink,
}

impl Flags {
    /// A fresh view over a snapshot of `base` alone for one call — the base-world
    /// shape, and the one every `#[cfg(test)]` fixture wants.
    pub fn new(base: &FlagStore, sink: EffectSink) -> Self {
        Self::with_chain(std::slice::from_ref(base), sink)
    }

    /// A fresh view over a snapshot of a layered store `chain` (innermost first),
    /// emitting mutations onto the call's shared effect `sink`.
    ///
    /// The production constructor since issue #1045: `RuntimeHost::call` hands
    /// down the chain of the layer whose handler is running, so a read resolves
    /// through the same walk its `when` predicates and its writes do.
    pub fn with_chain(chain: &[FlagStore], sink: EffectSink) -> Self {
        Self {
            overlay: Arc::new(Mutex::new(FlagOverlay {
                chain: chain.to_vec(),
                overlay: BTreeMap::new(),
            })),
            sink,
        }
    }

    /// Counter view of `name`: the overlay value if written this call, else the
    /// chain value (unset names, and names that walk past the chain's root, read
    /// `0`).
    fn get(&self, name: &str) -> i64 {
        let inner = self.overlay.lock().expect("flags overlay lock");
        inner
            .overlay
            .get(name)
            .copied()
            .unwrap_or_else(|| crate::world::flags::counter_in_owned_chain(&inner.chain, name))
    }

    /// Absolute write `flags.name = value`. Updates the overlay and emits an
    /// absolute `MutateFlag { SetValue }` in authored order.
    fn set(&self, name: &str, value: i64) {
        self.overlay
            .lock()
            .expect("flags overlay lock")
            .overlay
            .insert(name.to_string(), value);
        self.sink.push(ActionCmd::MutateFlag {
            target_layer: None,
            name: name.to_string(),
            mutation: FlagMutation::SetValue(value),
        });
    }

    /// Increment `flags.name` by `by` (may be negative). Updates the overlay
    /// (saturating, mirroring [`FlagStore::increment_flag`]) for read-after-write
    /// and emits a *relative* `MutateFlag { Increment(by) }` so a concurrent
    /// increment on the same flag composes rather than clobbers.
    ///
    /// [`FlagStore::increment_flag`]: crate::world::flags::FlagStore::increment_flag
    fn increment(&self, name: &str, by: i64) {
        {
            let mut inner = self.overlay.lock().expect("flags overlay lock");
            let before =
                inner.overlay.get(name).copied().unwrap_or_else(|| {
                    crate::world::flags::counter_in_owned_chain(&inner.chain, name)
                });
            inner
                .overlay
                .insert(name.to_string(), before.saturating_add(by));
        }
        self.sink.push(ActionCmd::MutateFlag {
            target_layer: None,
            name: name.to_string(),
            mutation: FlagMutation::Increment(by),
        });
    }
}

/// Register the `flags` custom type on a runtime engine.
///
/// The indexer serves reads (`flags.name` / `flags["name"]`) and absolute writes
/// (`flags.name = v`); `flags.increment(name, by)` is the composable-increment
/// verb (issue #981 hazard 1).
pub fn register_flags(engine: &mut HostRegistry) {
    engine.register_type_with_name::<Flags>("Flags");
    engine
        .register_indexer_get(|flags: &mut Flags, key: ImmutableString| -> i64 { flags.get(&key) });
    engine.register_indexer_set(|flags: &mut Flags, key: ImmutableString, value: i64| {
        flags.set(&key, value);
    });
    host_fn!(
        engine,
        "increment",
        receiver = "flags",
        category = "flag",
        params = ["name", "by"],
        summary = "Composably add `by` to a counter flag. Use over `flags.x += n`.",
        |flags: &mut Flags, name: ImmutableString, by: i64| {
            flags.increment(&name, by);
        },
    );
}

#[cfg(test)]
#[path = "flags_tests.rs"]
mod tests;
