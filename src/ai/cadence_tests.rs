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
fn world_config(sim_hz: f32, ai_hz: f32, snapshot_hz: f32) -> crate::world::config::WorldConfig {
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

// ── evaluate_every_ticks (issue #889's PASM-tracked runtime gap) ────────

/// `evaluate_every_ticks <= 1` must reduce to exactly the base cadence:
/// every shipped hull authors no explicit value (the TOML parse default is
/// 1), so this is the pin that keeps shipped behaviour unchanged. Driven
/// against the REAL derived base interval, not a hand-picked number.
#[test]
fn n_equal_one_reduces_to_every_base_tick() {
    let mut app = cadence_app(10);
    app.insert_resource(world_config(60.0, 30.0, 10.0));

    let mut base_arms = 0usize;
    let mut n1_arms = 0usize;
    for _ in 0..12 {
        app.update();
        let tick = app.world().resource::<crate::sim_tick::SimTick>().0;
        let base_interval = app.world().resource::<AiBaseInterval>().0;
        if app.world().resource::<AiTickReady>().0 {
            base_arms += 1;
        }
        if evaluate_every_ticks_ready(tick, base_interval, 1) {
            n1_arms += 1;
        }
    }
    assert_eq!(
        base_arms, n1_arms,
        "evaluate_every_ticks = 1 must decide on exactly the ticks \
             AiTickReady already arms on — no host adopting this check may \
             change behaviour for content that never authors the field"
    );
}

/// A fixture authors `evaluate_every_ticks = 3`: it must decide on exactly
/// every third arm of the shared base latch, never more, never fewer, and
/// the arms it picks must be the 3rd, 6th, 9th, ... — not merely "a third
/// of them" in some other pattern. `base_interval` (2, from 60/30 Hz) keeps
/// the test honest that the multiple stacks ON TOP of the base derivation
/// rather than replacing it: the host's own arm is every 6th SIM tick.
#[test]
fn n_equal_three_decides_on_exactly_every_third_arm() {
    let mut app = cadence_app(10);
    app.insert_resource(world_config(60.0, 30.0, 10.0));

    let mut base_arm_index = 0u64; // 1-based count of AiTickReady arms seen
    let mut due_at_base_arm: Vec<u64> = Vec::new();
    for _ in 0..24 {
        app.update();
        if !app.world().resource::<AiTickReady>().0 {
            continue;
        }
        base_arm_index += 1;
        let tick = app.world().resource::<crate::sim_tick::SimTick>().0;
        let base_interval = app.world().resource::<AiBaseInterval>().0;
        assert_eq!(
            base_interval, 2,
            "60/30 Hz must derive a 2-sim-tick base interval"
        );
        if evaluate_every_ticks_ready(tick, base_interval, 3) {
            due_at_base_arm.push(base_arm_index);
        }
    }
    assert_eq!(
        due_at_base_arm,
        vec![3, 6, 9, 12],
        "evaluate_every_ticks = 3 must decide on exactly every third arm \
             of the shared base latch (12 base arms in 24 sim ticks at a \
             2-tick base interval), not some other fraction of them"
    );
}
