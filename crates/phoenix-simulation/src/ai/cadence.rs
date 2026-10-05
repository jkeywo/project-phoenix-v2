//! The single AI decision cadence (issues #889, #895).
//!
//! Before this module the AI's decision cadence was fragmented three ways:
//! the six per-axis helm systems ran on an `AiHelmTickTimer` at the
//! TOML-authored `[global] ai_helm_tick_hz`; Captain and Sensors ran on a
//! *hardcoded* 10 Hz `AiSnapshotTimer` gated inside the system body by an
//! `Option<Res<_>>` that fell back to evaluating every tick when absent; and
//! seven further deciders (shield focus, power allocation, torpedo auto-fire,
//! torpedo load, frequency hint, phaser auto-fire, blaster auto-fire, AI target
//! selection) had no gate at all.
//!
//! #889 unified those onto one wall-clock `Timer`; #895 removed the wall clock.
//! The cadence is now DERIVED from the logical simulation tick
//! ([`SimTick`](crate::sim_tick::SimTick)) by counting: every
//! `sim_tick_hz / ai_tick_hz`-th fixed step is an AI decision tick, and every
//! `ai_tick_hz / ai_snapshot_hz`-th of those is a snapshot tick. Both ratios
//! are authored in the world TOML (the old `ai_helm_tick_hz` key remains a
//! serde alias for `ai_tick_hz`, so every shipped world keeps working) and both
//! are validated as positive integers at world load
//! (`world::config::parse_world`), so no clock in the AI stack can drift
//! against the tick two lockstep hosts must agree on. `tick_ai_cadence` reads
//! no `Res<Time>` at all — two hosts that agree on the tick count agree on
//! every AI decision boundary, regardless of their frame rates.
//!
//! # Why a latch resource rather than a modulo in a run condition
//! `run_if` conditions could compute `tick % n == 0` themselves, but `n` comes
//! from the world config, and fifteen conditions each reading two resources
//! and re-deriving the ratio is exactly the drift surface #889 removed. One
//! system ([`tick_ai_cadence`]) writes the two boolean latches; the conditions
//! read one `bool`.
//!
//! # Scheduling: consume-before-rearm
//! Every gated system lives in `FixedUpdate`; [`tick_ai_cadence`] runs in
//! `FixedLast`, after [`advance_sim_tick`](crate::sim_tick::advance_sim_tick)
//! has moved the counter to the next step's index. Within each fixed step the
//! latch is therefore consumed by the gated systems before it is re-armed for
//! the following step — the same guarantee the pre-#895 `Update`/`Last` split
//! provided, now inside the fixed loop.
//!
//! # Free-run on the first step
//! Both latches initialise to `true`, and the modulo agrees (`0 % n == 0`), so
//! the very first fixed step always decides. This mirrors the pre-#889
//! behaviour of both `AiHelmTickReady` and `AiSnapshotReady`.
//!
//! # The no-world fixture arm
//! Without a `WorldConfig` BOTH latches arm on EVERY fixed step. That is the
//! faithful successor to the pre-#895 fixture behaviour — a bare-`App` fixture
//! ticked both the 33 ms base timer and the 100 ms snapshot timer with a 200 ms
//! `ManualDuration`, so both fired on every update — and it is what lets such a
//! harness drive one decision per `update()` without authoring a world. Taking
//! the snapshot divisor from `GlobalConfig::default()` instead would silently
//! put every fixture's Captain and Sensors on a 3-step cadence they were never
//! written for. Per the #889 lesson — a fallback arm every fixture takes leaves
//! the shipped arm untested — the SHIPPED derivation (both authored ratios, via
//! a real `WorldConfig`) is pinned by this module's own tests below, not left
//! to chance.

use bevy::prelude::*;

/// Boolean latch set once per fixed step by [`tick_ai_cadence`]: `true` on AI
/// base-cadence steps, `false` on every other step. Read by [`ai_tick_ready`].
#[derive(Resource)]
pub struct AiTickReady(pub bool);

/// Boolean latch for the DERIVED slower cadence — `true` on every
/// `ai_tick_hz / ai_snapshot_hz`-th base tick. Read by [`ai_snapshot_ready`].
///
/// Gates the world-snapshot / doctrine-aggregation rebuild and the two policy
/// hosts that have always run on that slower clock (Captain, Sensors).
#[derive(Resource)]
pub struct AiSnapshotReady(pub bool);

/// The shared AI base cadence's INTERVAL in raw sim ticks — `sim_tick_hz /
/// ai_tick_hz`, the same `per_ai` [`tick_ai_cadence`] derives [`AiTickReady`]
/// from. Written alongside the two latches so a host whose fine-system
/// authors a per-host `evaluate_every_ticks` multiple (issue #889's
/// PASM-tracked runtime gap — the field was parsed and validated but no host
/// read it) can derive its OWN slower arm — `tick % (base_interval * n) == 0`,
/// see [`evaluate_every_ticks_ready`] — from the tick count alone, with no
/// second derivation of `sim_tick_hz / ai_tick_hz` anywhere else in the crate.
///
/// `1` in the no-world fixture arm, matching [`tick_ai_cadence`]'s own `(1,
/// 1)` fallback: without an authored `WorldConfig` every base tick already
/// decides, so a per-host `n` still divides the SAME tick count a fixture
/// steps by hand.
#[derive(Resource, Default)]
pub struct AiBaseInterval(pub u64);

/// Derive both latches from the logical tick count.
///
/// Registered in `FixedLast`, after
/// [`advance_sim_tick`](crate::sim_tick::advance_sim_tick): the counter then
/// holds the index of the NEXT fixed step, so the latches written here are
/// that step's, and the current step's gated systems (all in `FixedUpdate`)
/// have already consumed theirs. Reads no `Res<Time>` (issue #895 AC): the
/// cadence is a pure function of the tick count and the authored rates.
pub fn tick_ai_cadence(
    tick: Res<crate::sim_tick::SimTick>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ready: ResMut<AiTickReady>,
    mut snapshot_ready: ResMut<AiSnapshotReady>,
    mut base_interval: ResMut<AiBaseInterval>,
) {
    let (per_ai, snapshot_every) = match world_config.as_deref() {
        Some(wc) => (
            wc.global.sim_ticks_per_ai_tick() as u64,
            wc.global.snapshot_every_ticks() as u64,
        ),
        // No authored world (bare-`App` fixtures): every fixed step is both a
        // decision tick and a snapshot tick — see the module docs' "no-world
        // fixture arm" note. Deliberately (1, 1) rather than the shipped
        // default divisor: pre-#895 a fixture's wall-clock snapshot timer
        // fired every update too, and borrowing `GlobalConfig::default()` here
        // would quietly re-cadence every fixture's Captain and Sensors.
        None => (1, 1),
    };
    let per_ai = per_ai.max(1);
    let per_snapshot = (per_ai * snapshot_every).max(1);
    ready.0 = tick.0.is_multiple_of(per_ai);
    snapshot_ready.0 = tick.0.is_multiple_of(per_snapshot);
    base_interval.0 = per_ai;
}

/// Re-derive the cadence latches immediately at a state-transfer boundary.
///
/// Ordinarily [`tick_ai_cadence`] runs in `FixedLast` and leaves the latches
/// armed for the next fixed step. A paused transfer can enter before another
/// fixed step runs, however, while a restored peer necessarily reconstructs
/// its derived state directly from [`SimTick`](crate::sim_tick::SimTick). Run
/// the same derivation on the live side at that boundary so both peers resume
/// with the same next-decision latch. Partial fixture worlds may not have
/// installed every cadence resource, so this keeps the snapshot restore seam's
/// existing best-effort contract.
pub fn rederive_ai_cadence(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    let _ = world.run_system_once(tick_ai_cadence);
}

/// Whether an authored `evaluate_every_ticks = n` multiple (issue #889) is due
/// to decide on THIS tick, given the shared base interval [`tick_ai_cadence`]
/// derives.
///
/// `n <= 1` reduces to "every base-cadence arm" — the shipped default, and
/// exactly the ticks [`AiTickReady`] already arms on, so a host adopting this
/// check changes nothing for any content that never authors the field. Pure
/// function of the raw tick count: no timer, no per-host clock, and the
/// "phase" (which arms count as the 0th, nth, 2nth, ... of this host's own
/// slower cadence) is anchored to `SimTick` 0 — the same anchor `AiTickReady`
/// and `AiSnapshotReady` use — so two hosts authoring the same `n` (or the
/// same host on two lockstep peers) agree on which arms fire without
/// coordinating with each other.
///
/// Callers gate on this INSTEAD OF re-deriving `sim_tick_hz / ai_tick_hz`
/// themselves: `base_interval` already IS that derivation, read from
/// [`AiBaseInterval`] once per tick by every caller, so the arithmetic lives
/// in this one function rather than in each of the hosts that read it.
pub fn evaluate_every_ticks_ready(
    tick: u64,
    base_interval: u64,
    evaluate_every_ticks: u32,
) -> bool {
    let n = (evaluate_every_ticks as u64).max(1);
    let base = base_interval.max(1);
    tick.is_multiple_of(base * n)
}

/// Read-only run condition: the shared AI base cadence.
pub fn ai_tick_ready(ready: Res<AiTickReady>) -> bool {
    ready.0
}

/// Read-only run condition: the derived slower snapshot cadence.
pub fn ai_snapshot_ready(ready: Res<AiSnapshotReady>) -> bool {
    ready.0
}

/// Install the shared cadence resources and the one system that derives them,
/// plus the [`SimTick`](crate::sim_tick::SimTick) counter they derive from.
///
/// Idempotent, and deliberately a plain function rather than a `Plugin`: every
/// plugin that registers a gated system calls it, and a duplicate registration
/// would re-derive the latches once per calling plugin.
pub fn register_ai_cadence(app: &mut App) {
    if app.world().contains_resource::<AiTickReady>() {
        return;
    }
    crate::sim_tick::register_sim_tick(app);
    app.insert_resource(AiTickReady(true))
        .insert_resource(AiSnapshotReady(true))
        .insert_resource(AiBaseInterval(1))
        .add_systems(
            FixedLast,
            tick_ai_cadence.after(crate::sim_tick::advance_sim_tick),
        );
    // Authoritative-state exclusion declarations (issue #1221, Track 3 step C9).
    // The three cadence latches are DERIVED — pure functions of the tick counter
    // (`sim_tick_hz / ai_tick_hz`), whose source `SimTick` is already folded — so
    // they are declared here at their owning site, replacing the `EXCLUSIONS`
    // const in `tests/authoritative_state_enumeration.rs`. Reached under the
    // idempotency guard above, so declared exactly once; inert to the digest.
    {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<AiTickReady>(StateClass::Derived, "ai-policy-tick-scheduler")
            .declare_state::<AiSnapshotReady>(StateClass::Derived, "ai-policy-tick-scheduler")
            .declare_state::<AiBaseInterval>(StateClass::Derived, "ai-policy-tick-scheduler");
    }
}

/// Re-arm both latches so the next `app.update()` is an AI decision tick.
///
/// Test-only. Fixtures that assert on decision CONTENT drive several updates
/// without stepping the fixed clock; they call this to tick the latch by hand
/// rather than relying on an evaluate-every-frame fallback that production
/// never takes. Fixtures that assert on CADENCE drive `Time` instead and must
/// not call this.
#[cfg(test)]
pub fn arm_ai_tick(app: &mut App) {
    if let Some(mut ready) = app.world_mut().get_resource_mut::<AiTickReady>() {
        ready.0 = true;
    }
    if let Some(mut ready) = app.world_mut().get_resource_mut::<AiSnapshotReady>() {
        ready.0 = true;
    }
}

#[cfg(test)]
#[path = "cadence_tests.rs"]
mod tests;
