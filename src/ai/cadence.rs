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
        .add_systems(
            FixedLast,
            tick_ai_cadence.after(crate::sim_tick::advance_sim_tick),
        );
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
mod tests {
    use super::*;

    /// A cadence app whose `ManualDuration` equals its fixed timestep, so
    /// every `update()` after the zero-delta baseline frame runs exactly one
    /// fixed step.
    fn cadence_app(period_ms: u64) -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        register_ai_cadence(&mut app);
        let period = std::time::Duration::from_millis(period_ms);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
        // Establish the time baseline: the first update carries a zero delta
        // and runs no fixed step, so it would otherwise read the init-true
        // latches as a counted decision.
        app.update();
        app
    }

    /// A `WorldConfig` authoring the given rates — the SHIPPED derivation arm,
    /// which no fixture without a world config ever exercises (#889's lesson).
    fn world_config(
        sim_hz: f32,
        ai_hz: f32,
        snapshot_hz: f32,
    ) -> crate::world::config::WorldConfig {
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.global.sim_tick_hz = sim_hz;
        cfg.global.ai_tick_hz = ai_hz;
        cfg.global.ai_snapshot_hz = snapshot_hz;
        cfg
    }

    /// Drive `steps` fixed steps and record `(base, snapshot)` latch counts.
    fn count_latches(app: &mut App, steps: usize) -> (usize, usize) {
        let mut base = 0;
        let mut snapshot = 0;
        for _ in 0..steps {
            app.update();
            if app.world().resource::<AiTickReady>().0 {
                base += 1;
                if app.world().resource::<AiSnapshotReady>().0 {
                    snapshot += 1;
                }
            }
        }
        (base, snapshot)
    }

    /// The shipped default rates (60/30/10): every second sim tick is an AI
    /// tick, every sixth is a snapshot tick, and the snapshot latch only ever
    /// arms alongside the base latch — both derived from the tick count alone.
    #[test]
    fn cadence_is_derived_from_the_tick_count_at_the_shipped_rates() {
        let mut app = cadence_app(10);
        app.insert_resource(world_config(60.0, 30.0, 10.0));

        // 12 steps: `count_latches` reads each latch AFTER `app.update()`
        // returns, and `tick_ai_cadence` (`FixedLast`, `.after(advance_sim_tick)`)
        // computes it from the POST-increment tick — the module's
        // consume-before-rearm scheduling pre-arms the NEXT step's latch, not
        // the step that just ran. So the Nth `update()` observes the latch
        // keyed to tick N, not N-1: base fires on ticks 2,4,6,8,10,12 → 6;
        // snapshot fires on 6,12 → 2.
        let (base, snapshot) = count_latches(&mut app, 12);
        assert_eq!(
            base, 6,
            "at 60/30 Hz every second sim tick is an AI decision tick"
        );
        assert_eq!(
            snapshot, 2,
            "the snapshot cadence is DERIVED as every third AI tick \
             (30 Hz / 10 Hz), not a second independent clock"
        );
    }

    /// The base cadence is TOML-authored, not hardcoded: an authored
    /// `[global] ai_tick_hz` equal to the sim rate makes every step decide.
    #[test]
    fn base_rate_is_read_from_world_config() {
        let mut app = cadence_app(10);
        app.insert_resource(world_config(100.0, 100.0, 100.0));

        let (base, _) = count_latches(&mut app, 8);
        assert_eq!(
            base, 8,
            "with sim_tick_hz == ai_tick_hz every fixed step must be a \
             decision tick — fewer means the authored rate was never applied"
        );
    }

    /// The frame-rate decoupling that is the whole point: on frames whose
    /// accumulated time never reaches the timestep, no fixed step runs, so no
    /// new decision tick can be minted — however many frames the host renders.
    #[test]
    fn frames_without_a_fixed_step_mint_no_decision_ticks() {
        let mut app = cadence_app(30);
        app.insert_resource(world_config(60.0, 30.0, 10.0));
        // Reconfigure the drive to a third of the timestep: what a 90 Hz
        // rAF-driven host does against a ~33 ms tick.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(10),
        ));

        let mut steps = 0usize;
        for _ in 0..12 {
            let tick_before = app.world().resource::<crate::sim_tick::SimTick>().0;
            let latch_before = app.world().resource::<AiTickReady>().0;
            app.update();
            let tick_after = app.world().resource::<crate::sim_tick::SimTick>().0;
            if tick_after == tick_before {
                // No fixed step ran this frame: the latch must be exactly what
                // the last step left it — a rendered frame can never re-arm.
                assert_eq!(
                    app.world().resource::<AiTickReady>().0,
                    latch_before,
                    "a frame with no fixed step re-armed the AI latch"
                );
            } else {
                steps += (tick_after - tick_before) as usize;
            }
        }
        assert!(
            (3..=5).contains(&steps),
            "12 frames x 10 ms against a 30 ms timestep is 4 steps (±1 for \
             rounding); got {steps} — the fixed loop is not throttling"
        );
    }

    /// Without a `WorldConfig`, every fixed step decides — on BOTH latches.
    /// The documented fixture arm, pinned so a change here is a deliberate one:
    /// a snapshot divisor borrowed from the shipped defaults would put every
    /// bare-`App` fixture's Captain and Sensors on a 3-step cadence they were
    /// never written against (pre-#895 their wall-clock timer fired every
    /// update).
    #[test]
    fn without_a_world_config_every_step_is_a_decision_tick() {
        let mut app = cadence_app(10);
        let (base, snapshot) = count_latches(&mut app, 6);
        assert_eq!(base, 6);
        assert_eq!(
            snapshot, 6,
            "the fixture arm must arm the SNAPSHOT latch every step too"
        );
    }
}
