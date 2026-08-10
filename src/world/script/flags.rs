//! The sugared `flags` script type (issue #979, Rhai milestone M1).
//!
//! `FlagStore` stays the only mutable world state a script can touch, reached
//! through a `flags` custom type that reads like a struct:
//!
//! ```rhai
//! fn on_armed(ctx) {
//!     ctx.flags.score += 50;      // read-modify-write a counter
//!     ctx.flags.alarm = 1;        // set a boolean (non-zero == true)
//! }
//! ```
//!
//! `flags.score` is not a fixed property — Rhai resolves an unknown property on
//! a custom type by falling back to an **indexer**, so a single registered
//! indexer get/set serves every flag name (`chaining.rs`: "Try an indexer if
//! property does not exist").
//!
//! # Scratch overlay
//!
//! Writes do not touch the real store during a call; they land in a per-call
//! **overlay** on top of a snapshot of the live [`FlagStore`]. That is what
//! makes read-after-write correct *within one call*: `ctx.flags.score` after a
//! `+= 50` reads back the 50, not the pre-call value. When the call succeeds the
//! host [`drain`](Flags::drain)s the overlay into the effect buffer as
//! `ActionCmd::MutateFlag { mutation: SetValue(..) }` commands — the applier
//! writes them to the store exactly as a `SetWorldFlagValue` trigger would. The
//! overlay is a `BTreeMap`, so the drain order is sorted and deterministic; on
//! the failure path the overlay is dropped whole with the rest of the call's
//! effects (settled decision 10).
//!
//! Draining collapses a read-modify-write to an absolute `SetValue`: the overlay
//! stores the final computed value, which is both what read-after-write needs
//! and what keeps two calls in a tick consistent (each snapshots the live store
//! the applier has already advanced). `parent:`-prefixed cross-layer names are
//! not specially resolved in M1 — no shipped world authors scripts yet — so a
//! name drains verbatim to the base layer.
//!
//! [`FlagStore`]: crate::world::flags::FlagStore

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rhai::{Engine, ImmutableString};

use crate::world::dispatch::{ActionCmd, FlagMutation};
use crate::world::flags::FlagStore;

/// The per-call flag view: a snapshot of the live store plus a scratch overlay
/// of pending writes.
#[derive(Clone, Default)]
struct FlagOverlay {
    /// Snapshot of the live store at call start. Reads of an un-overlaid name
    /// fall through to here.
    base: FlagStore,
    /// Pending writes, keyed by flag name. Sorted iteration → deterministic
    /// drain order.
    overlay: BTreeMap<String, i64>,
}

/// The `flags` custom type handed to a script call.
///
/// Cloneable and interior-mutable (like [`EffectSink`](super::effects::EffectSink)):
/// the clone that goes into the context map and the clone the host retains share
/// one overlay, so the host observes every write after the call returns.
#[derive(Clone, Default)]
pub struct Flags(Arc<Mutex<FlagOverlay>>);

impl Flags {
    /// A fresh view over a snapshot of `base` for one call.
    pub fn new(base: &FlagStore) -> Self {
        Self(Arc::new(Mutex::new(FlagOverlay {
            base: base.clone(),
            overlay: BTreeMap::new(),
        })))
    }

    /// Counter view of `name`: the overlay value if written this call, else the
    /// snapshot value (unset names read `0`).
    fn get(&self, name: &str) -> i64 {
        let inner = self.0.lock().expect("flags overlay lock");
        inner
            .overlay
            .get(name)
            .copied()
            .unwrap_or_else(|| inner.base.counter(name))
    }

    /// Write `value` to `name` in the overlay.
    fn set(&self, name: &str, value: i64) {
        self.0
            .lock()
            .expect("flags overlay lock")
            .overlay
            .insert(name.to_string(), value);
    }

    /// Drain the overlay into `ActionCmd`s, in sorted name order.
    ///
    /// Each pending write becomes an absolute `MutateFlag { SetValue }` at base
    /// scope. Called by the host on the success path only.
    pub fn drain(&self) -> Vec<ActionCmd> {
        let inner = self.0.lock().expect("flags overlay lock");
        inner
            .overlay
            .iter()
            .map(|(name, &value)| ActionCmd::MutateFlag {
                target_layer: None,
                name: name.clone(),
                mutation: FlagMutation::SetValue(value),
            })
            .collect()
    }
}

/// Register the `flags` custom type and its indexer on a runtime engine.
///
/// The indexer serves both `flags.name` (via Rhai's property→indexer fallback)
/// and explicit `flags["name"]` syntax.
pub fn register_flags(engine: &mut Engine) {
    engine.register_type_with_name::<Flags>("Flags");
    engine
        .register_indexer_get(|flags: &mut Flags, key: ImmutableString| -> i64 { flags.get(&key) });
    engine.register_indexer_set(|flags: &mut Flags, key: ImmutableString, value: i64| {
        flags.set(&key, value);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::script::engine::runtime_engine;
    use rhai::{Dynamic, Map};

    /// Compile `source`, call `fn_name` with a `flags` view over `base`, and
    /// return the drained flag commands.
    fn run(source: &str, fn_name: &str, base: FlagStore) -> (Flags, Vec<ActionCmd>) {
        let engine = runtime_engine();
        let ast = engine.compile(source).expect("compiles");
        let flags = Flags::new(&base);
        let mut ctx = Map::new();
        ctx.insert("flags".into(), Dynamic::from(flags.clone()));
        let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).expect("calls");
        let cmds = flags.drain();
        (flags, cmds)
    }

    #[test]
    fn read_after_write_sees_the_written_value_within_one_call() {
        // `a` is incremented twice, then `b` is set FROM `a` — so `b` only ends
        // up 12 if the second read of `a` saw the first two writes.
        let (_flags, cmds) = run(
            r#"fn on_x(ctx) {
                ctx.flags.a += 5;
                ctx.flags.a += 7;
                ctx.flags.b = ctx.flags.a;
            }"#,
            "on_x",
            FlagStore::new(),
        );
        assert_eq!(
            cmds,
            vec![
                ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "a".to_string(),
                    mutation: FlagMutation::SetValue(12),
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
    fn reads_fall_through_to_the_base_snapshot() {
        let mut base = FlagStore::new();
        base.set_flag_value("score", 100);
        // `score` starts at 100 (from the snapshot); += 50 -> 150.
        let (_flags, cmds) = run(r#"fn on_x(ctx) { ctx.flags.score += 50; }"#, "on_x", base);
        assert_eq!(
            cmds,
            vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "score".to_string(),
                mutation: FlagMutation::SetValue(150),
            }]
        );
    }

    #[test]
    fn no_writes_drains_to_nothing() {
        // Reading a flag must not create an overlay entry.
        let (_flags, cmds) = run(
            r#"fn on_x(ctx) { let seen = ctx.flags.absent; }"#,
            "on_x",
            FlagStore::new(),
        );
        assert!(cmds.is_empty());
    }

    #[test]
    fn indexer_syntax_also_works() {
        let (_flags, cmds) = run(
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
}
