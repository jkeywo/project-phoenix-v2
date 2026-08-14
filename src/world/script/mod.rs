//! Rhai scenario-scripting host seam (issue #979, Rhai milestone M1).
//!
//! Phoenix embeds the fleet's deterministic Rhai sandbox (`vellum-script`) so
//! scenario logic can be authored as script while TOML keeps the declarative
//! spine. Following vellum's charter, the *sandbox profile* and the
//! context-in/effects-out *call seam* come from `vellum-script` unchanged; the
//! *vocabulary* lives here:
//!
//! * [`engine`] — the two engines (last-aeon's pattern). A **loading** engine
//!   registers builder host-fns and runs each unit's top level once
//!   (`Engine::run_ast`) to collect registrations; a **runtime** engine (from
//!   `quiet_sandbox`) never re-runs top level and only calls retained
//!   functions (`CallFnOptions::eval_ast(false)`, via `vellum_script::call_fn`).
//! * [`effects`] — the per-call effect buffer. Host functions push the
//!   *existing* [`ActionCmd`](crate::world::dispatch::ActionCmd)s; script gets
//!   no new effect vocabulary of its own.
//! * [`flags`] — a sugared `flags` custom type over
//!   [`FlagStore`](crate::world::flags::FlagStore). Use `flags.increment("score",
//!   50)` for a composable counter (emits `FlagMutation::Increment`, so a script
//!   increment and a concurrent TOML `increment_flag` on the same flag in the
//!   same tick compose in either order) and `flags.x = v` for an absolute set
//!   (`FlagMutation::SetValue`). A scratch overlay keeps read-after-write within
//!   one call correct; each mutation is pushed onto the call's single shared
//!   effect sink *at the point it is authored*, so flag writes and effects emit
//!   in authored order. Note: `flags.x += n` still parses but silently degrades
//!   to an absolute `SetValue` (Rhai desugars `+=` on an indexer to get-then-set
//!   before the custom type is consulted, so it cannot be made composable) — do
//!   not use it for counters; reach for `flags.increment` instead.
//! * [`schedule`] — the `schedule` custom type (issue #981, M3): `in_seconds(n)`
//!   stamps a delay onto a buffered effect and routes it through the *existing*
//!   [`DelayedAction`](crate::world::delayed::DelayedAction) queue, and
//!   `after(n, |ctx| …)` defers a callback as a serializable
//!   `(fire_tick, script_path, fn_name)` record. Per-tick operation and call
//!   budgets are fixed engine-safety limits enforced deterministically.
//! * [`load`] — resolves `script = "…"` sibling `.rhai` files and lifts inline
//!   `[script.*]` TOML blocks to virtual paths (`world.toml#script.on_x`), so
//!   there is one loader, one AST map, and one span-offset mapping. ASTs land
//!   in a `BTreeMap<String, AST>` in sorted path order and feed
//!   `vellum_script::content_hash`.
//! * [`triggers`] — the Rhai trigger front-end (issue #980, M2): one loading-
//!   engine host fn per `TriggerCondition` variant (`on_destroyed`, `on_timer`,
//!   …), each building the *same* [`Trigger`](crate::world::config::Trigger) the
//!   TOML `[[trigger]]` front-end builds. One evaluator, two front-ends.
//! * [`comms`] — the Rhai comms dialogue front-end (issue #982, M4): a dialogue
//!   node is a named fn returning `#{message, responses:[#{text, on_pick}]}`, and
//!   a response's `on_pick` names the next node fn — so follow-ups are fn-to-fn
//!   references, not nested `[[comms.response.follow_up…]]` tables. Response
//!   effects route through the *same* [`ActionCmd`](crate::world::dispatch::ActionCmd)
//!   boundary the TOML `[[comms.response.action]]` array does, so the two
//!   front-ends emit identical command sequences. Dormant in M4 — the live
//!   `handle_respond_to_message` collapse is deferred to M7.
//! * [`deadlines`] — the named-deadline vocabulary (issue #1024): a loading-engine
//!   `on_deadline("id", "fn")` declaration pairing an authored `[[deadline]]`
//!   block with the fn it runs, and a runtime `ctx.deadlines` handle that reads a
//!   deadline's remaining time and state and buffers `slip`/`cancel`. A deadline's
//!   firing rides the **existing** `pending_callbacks` queue — see
//!   [`crate::world::deadlines`] for why that is the whole design.
//! * [`validate`] — a cross-reference pass over `AST::iter_functions()` proving
//!   every registered handler name (and every TOML `script = "fn"` reference, for
//!   both `[[trigger]]` and `[[comms]]`) resolves at load, emitting the existing
//!   [`WorldFinding`](crate::world::validate::WorldFinding)s so the atomic
//!   activation gate keeps working.
//!
//! # Determinism constraints (inherited, not negotiated)
//!
//! * **Fixed hashing seed.** Rhai names every anonymous function `anon$<hex>`,
//!   hashed with a seed that defaults to OS randomness *per process*. Two runs
//!   of the same binary would therefore name the same closure differently, and
//!   serialized deferred work would resolve to nothing after a reload. The M0
//!   spike (`rhai-anonymous-function-naming`) proved that
//!   [`init_hashing_seed`] fixes this — but only if it runs before *any* engine
//!   is constructed. `set_hashing_seed` silently no-ops once a hash has been
//!   taken, so this must be genuinely first: it is wired into every phoenix
//!   startup path (`wasm_init`, `phoenix-headless`) and is also called
//!   defensively at the head of each engine constructor here.
//! * **Integer-only arithmetic** (`no_float`, inherited from vellum). The whole
//!   script API is integer-only; any float conversion (a TOML `after_secs =
//!   600.0`, say) happens at the host-fn boundary — but M1 has no scheduling,
//!   so no float ever reaches script.
//! * **Fixed operation budgets** ([`MAX_OPS_PER_CALL`], [`MAX_OPS_PER_TICK`],
//!   [`MAX_CALLS_PER_TICK`]). These are engine *safety limits*, not tunable
//!   gameplay data, so they are deliberately hardcoded constants (allowed under
//!   AGENTS.md rule 11). M1 wires the per-call limit; the per-tick aggregate and
//!   call cap are defined now and enforced in M3 once a tick loop exists to hook
//!   (see their TODOs).
//!
//! # Failure mode (settled decision 10)
//!
//! A script that raises or trips a limit **panics in dev** (`debug_assertions`)
//! and, in release, has that one call's effects **discarded whole**, is logged,
//! and the game continues. See [`engine::RuntimeHost::call`].

pub mod authoring;
/// The `commitments` read/write vocabulary (issue #1029): record a promise,
/// settle it kept or broken, and read its state back to gate a dialogue option.
pub mod commitments;
pub mod comms;
/// The `deadlines` script vocabulary (issue #1024): `on_deadline` on the
/// loading engine, `ctx.deadlines.remaining/state/slip/cancel` on the runtime one.
pub mod deadlines;
/// The `dossier` write vocabulary (issue #1031): `ctx.dossier.append(…)` writes
/// one finding, with the provenance that says how the crew learned it, onto a
/// subject's file.
pub mod dossier;
pub mod effects;
pub mod engine;
/// Test-only harness for driving a `[script]`-authored world's handlers, shared
/// by the four modules whose shipped-world tests lost their declarative subject
/// to the conversion (issue #984).
#[cfg(test)]
pub mod fixture;
pub mod flags;
pub mod load;
pub mod schedule;
pub mod triggers;
pub mod validate;

use std::sync::Once;

/// The fixed Rhai hashing seed for every phoenix process.
///
/// The exact value is arbitrary — any fixed `[u64; 4]` gives stable anonymous
/// function names — but it **must be identical on every peer and in the
/// headless runner**, because a serialized `(tick, script_path, fn_name)` key
/// only resolves against the seed it was recorded under. This matches the value
/// the M0 spike measured (`rhai-anonymous-function-naming`), so the `anon$`
/// names it recorded stay reproducible.
pub const HASHING_SEED: [u64; 4] = [1, 2, 3, 4];

/// Per-call operation budget. Overrides the vellum sandbox's 5,000,000 default
/// (far too generous for a per-tick call) via `Engine::set_max_operations`.
///
/// From the M0 spike (`rhai-script-operation-budget`): ~900x the heaviest real
/// handler measured, bounding one runaway script to ~1.55 ms natively. An
/// engine safety limit, not gameplay data.
pub const MAX_OPS_PER_CALL: u64 = 50_000;

/// Per-tick aggregate operation budget, summed across every script call in a
/// tick. A circuit breaker: when it trips, the tick's remaining script work is
/// dropped and the tick completes (every peer sums the same operations in the
/// same order, so every peer trips on the same tick).
///
/// Enforced by [`schedule::TickBudget`] (issue #981, M3): the host charges each
/// call's measured operation count (an `on_progress` high-water counter) to a
/// per-tick budget, which refuses the tick's remaining calls once this aggregate
/// is reached.
pub const MAX_OPS_PER_TICK: u64 = 200_000;

/// Per-tick script call cap. Operations do not price the ~2 µs of fixed
/// per-`call_fn` overhead, so this closes the "thousands of tiny handlers" hole
/// the operation budget alone leaves open (M0 spike).
///
/// Enforced alongside [`MAX_OPS_PER_TICK`] by [`schedule::TickBudget`] (issue
/// #981, M3): [`admit_call`](schedule::TickBudget::admit_call) refuses a call once
/// this many have run in the tick.
pub const MAX_CALLS_PER_TICK: u32 = 512;

static SEED_INIT: Once = Once::new();

/// Fix Rhai's global hashing seed. **Must run before any `Engine` is
/// constructed** — see the module docs.
///
/// Idempotent: the [`Once`] guard means the underlying `set_hashing_seed` runs
/// exactly once per process, so calling this from several startup paths (and
/// defensively from the engine constructors) is safe and cheap. The `Result`
/// from `set_hashing_seed` is intentionally ignored — it errors only if the
/// seed was already set, which the guard already prevents on our side.
pub fn init_hashing_seed() {
    SEED_INIT.call_once(|| {
        let _ = rhai::config::hashing::set_hashing_seed(Some(HASHING_SEED));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_hashing_seed_is_idempotent() {
        // Calling it repeatedly must not panic (the `Once` guard swallows the
        // second `set_hashing_seed`, which would otherwise return `Err`).
        init_hashing_seed();
        init_hashing_seed();
        init_hashing_seed();
    }

    #[test]
    fn budgets_are_ordered_safety_limits() {
        // Sanity on the fixed constants: a per-tick aggregate must admit more
        // than a single per-call runaway, and the call cap is positive.
        const { assert!(MAX_OPS_PER_TICK > MAX_OPS_PER_CALL) };
        const { assert!(MAX_CALLS_PER_TICK > 0) };
    }
}
