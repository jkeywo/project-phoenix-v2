//! The logical simulation tick (issue #895, PRD #849).
//!
//! The simulation advances in Bevy's `FixedMain` loop: `SimSet` is configured
//! in `FixedUpdate` (see `server_app::add_simulation_plugins_with`), so every
//! sim system runs zero or more times per rendered frame, once per fixed step,
//! on a clock two hosts can agree on. [`SimTick`] is the count of those steps —
//! the number every future lockstep artifact (command stamps, digests, replay
//! logs) keys on.
//!
//! # Rate
//! The step rate is the TOML-authored `[global] sim_tick_hz` (serde default
//! 60 Hz — the rate the browser host effectively ran at when the sim was
//! frame-driven, and headless' `DEFAULT_HZ`). [`reconcile_fixed_timestep`]
//! applies the authored rate to `Time<Fixed>` once a `WorldConfig` exists;
//! apps with no world config (bare-`App` fixtures) keep whatever timestep
//! their harness set, so a fixture can drive one step per `update()` without
//! fighting a reconciler.
//!
//! # Relation to the AI cadence
//! `ai::cadence` derives the AI decision tick from this counter as a whole
//! number of sim ticks (`sim_tick_hz / ai_tick_hz`, validated at world load),
//! replacing the wall-clock `Timer` it used while the sim was frame-driven.
//!
//! # Phase transitions are tick-timed too
//! Bevy runs its `StateTransition` schedule once per FRAME (after `PreUpdate`),
//! but every `NextState<GamePhase>` writer now lives in `FixedUpdate` — the
//! lobby countdown, the game-over setters in the weapon, world and region
//! modules. Left frame-timed, a K-step frame would run its remaining K−1 steps
//! under the stale phase, so the number of ticks before a transition (and the
//! tick `OnEnter` spawns land on) would vary with frame pacing. [`register_sim_tick`]
//! therefore also inserts `StateTransition` into the `FixedMainScheduleOrder`
//! right after `FixedUpdate` — see [`register_fixed_state_transition`].

use bevy::prelude::*;

/// Monotonic count of fixed simulation steps.
///
/// Advanced by [`advance_sim_tick`] in `FixedLast`, so inside the fixed
/// schedules it reads the 0-based index of the step currently executing, and
/// outside them (frame-driven schedules, tests, the JS bridge) it reads the
/// number of completed steps.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimTick(pub u64);

/// Advance [`SimTick`] at the end of every fixed step.
///
/// `FixedLast` rather than `FixedFirst` so that consumers *within* a step see
/// the index of that step (step 0 reads 0), matching the latch semantics in
/// `ai::cadence` where the very first step is a decision tick.
pub fn advance_sim_tick(mut tick: ResMut<SimTick>) {
    tick.0 = tick.0.wrapping_add(1);
}

/// The `Duration` of one sim tick at `hz`.
///
/// The one conversion every driver must share: `Time<Fixed>`'s accumulator
/// works in integer nanoseconds, so a headless/test harness that wants
/// "exactly one step per `update()`" must feed `TimeUpdateStrategy` the
/// *identical* `Duration` this produces — two call sites rounding
/// `1.0 / hz` separately can land one nanosecond apart and skip a step.
pub fn sim_tick_period(hz: f32) -> std::time::Duration {
    std::time::Duration::from_secs_f64(1.0 / hz as f64)
}

/// Apply the TOML-authored `[global] sim_tick_hz` to `Time<Fixed>`.
///
/// Registered in `First` (before `RunFixedMainLoop`) by
/// `add_simulation_plugins_with`, so the first frame's steps already run at
/// the authored rate: headless inserts `WorldConfig` before the app is built,
/// and the browser host inserts it during `Startup`, both of which precede the
/// first `First`. Runs every frame because the world config can be replaced at
/// runtime (scenario load), but only writes when the authored rate differs —
/// the same reconcile shape `ai::cadence` used for its timer while it was
/// wall-clock-driven.
///
/// Deliberately a no-op without a `WorldConfig`: bare-`App` fixtures configure
/// `Time<Fixed>` themselves and must not be fought back to the default.
///
/// # Rapier rides the same clock (issue #896)
/// Since #896 rapier's `PhysicsSet` chain runs inside `FixedUpdate` and advances
/// by a fixed `dt` of its own. One authored rate, two clocks that have to agree:
/// if a `WorldConfig` retuned `Time<Fixed>` and left rapier's `dt` alone,
/// physics would keep integrating at the shipped 60 Hz while the simulation
/// around it stepped at the authored rate, so a collision's speed — and the
/// damage it deals — would come out of a world moving at the wrong speed. Both
/// clocks are therefore set here, from the one `sim_tick_period`.
///
/// `Option<ResMut<TimestepMode>>` because bare-`App` fixtures never add
/// `RapierPhysicsPlugin`, and this system must not fail Bevy's parameter
/// validation in an app that has no physics at all.
pub fn reconcile_fixed_timestep(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut fixed: ResMut<Time<Fixed>>,
    mut physics: Option<ResMut<bevy_rapier3d::prelude::TimestepMode>>,
) {
    let Some(wc) = world_config else {
        return;
    };
    let hz = wc.global.sim_tick_hz;
    if !(hz.is_finite() && hz > 0.0) {
        return;
    }
    let configured = sim_tick_period(hz);
    if fixed.timestep() != configured {
        fixed.set_timestep(configured);
    }

    // Rapier takes seconds as `f32`, so `dt` comes from the same `configured`
    // `Duration` set on `Time<Fixed>` two lines up (`.as_secs_f32()`), not a
    // second, independent `1.0 / hz` division. `register_physics`
    // (`server_app.rs`) derives its own rapier `dt` the identical way, from
    // the same `sim_tick_period` call, so both clocks agree to the same
    // rounding rather than merely to the same formula. `as_secs_f32` is still
    // a lossy f64→f32 cast — rapier's own `f32` dt forces that — so this is
    // not bit-identical to `Time<Fixed>`'s f64 accumulator, only reciprocal
    // with `register_physics`'s rapier dt.
    if let Some(mode) = physics.as_mut() {
        let wanted = bevy_rapier3d::prelude::TimestepMode::Fixed {
            dt: configured.as_secs_f32(),
            substeps: 1,
        };
        if **mode != wanted {
            **mode = wanted;
        }
    }
}

/// Run Bevy's `StateTransition` schedule inside the fixed loop, immediately
/// after `FixedUpdate`.
///
/// `StatesPlugin` registers `StateTransition` in the `MainScheduleOrder` after
/// `PreUpdate`, i.e. once per rendered frame. That is the right place for the
/// writers that are still frame-driven (the JS bridge's force-start, the asset
/// preloader, headless' auto-start), and it stays registered there — this adds
/// a SECOND run site rather than moving the first.
///
/// The second site is what makes a phase change deterministic. Every in-game
/// `NextState<GamePhase>` writer runs in `FixedUpdate`, so on a frame that
/// steps the fixed loop K times a frame-only transition would leave the
/// remaining K−1 steps running under the stale phase: how many ticks elapse
/// before `GamePhase::GameOver` takes hold, and which tick `OnEnter` spawns
/// land on, would then depend on frame pacing. Running the schedule after
/// `FixedUpdate` puts the transition — and its `OnExit`/`OnTransition`/`OnEnter`
/// schedules — on a tick boundary: the step that sets `NextState` is the step
/// that applies it, and `in_state`-gated fixed systems see the new phase from
/// the very next step, whatever the frame rate.
///
/// Running the schedule twice per frame is safe: `apply_state_transition`
/// returns early unless `NextState` is set, and the `OnEnter`/`OnExit` runners
/// read the `StateTransitionEvent` stream through a cursor, so whichever site
/// applies the change is the only one that runs the enter/exit schedules.
///
/// Idempotent, so the fixture apps that register the tick through several
/// plugins do not stack duplicate labels.
pub fn register_fixed_state_transition(app: &mut App) {
    use bevy::ecs::schedule::ScheduleLabel;
    let label = bevy::state::state::StateTransition.intern();
    let mut order = app
        .world_mut()
        .get_resource_or_init::<bevy::app::FixedMainScheduleOrder>();
    if order.labels.contains(&label) {
        return;
    }
    order.insert_after(FixedUpdate, bevy::state::state::StateTransition);
}

/// Install the tick counter and its advance system. Idempotent, and a plain
/// function rather than a `Plugin` for the same reason as
/// `ai::cadence::register_ai_cadence`: several plugins depend on it, and a
/// duplicate registration of [`advance_sim_tick`] would count each step twice.
///
/// Also puts state transitions on the tick — an app whose simulation runs in
/// the fixed loop must have its phase changes land there too, or the two
/// disagree on a multi-step frame ([`register_fixed_state_transition`]).
pub fn register_sim_tick(app: &mut App) {
    if app.world().contains_resource::<SimTick>() {
        return;
    }
    register_fixed_state_transition(app);
    app.init_resource::<SimTick>()
        .add_systems(FixedLast, advance_sim_tick);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One counted tick per fixed step, none on frames whose accumulated time
    /// stays under the timestep, and catch-up frames count every step they run.
    #[test]
    fn sim_tick_counts_fixed_steps_not_frames() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        register_sim_tick(&mut app);

        let period = std::time::Duration::from_millis(10);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);

        // Frame 0 establishes the time baseline (zero delta): no step.
        app.update();
        assert_eq!(app.world().resource::<SimTick>().0, 0);

        // A frame of exactly half a period: still no step.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period / 2));
        app.update();
        assert_eq!(
            app.world().resource::<SimTick>().0,
            0,
            "a frame shorter than the timestep must not advance the logical tick"
        );

        // The second half arrives: exactly one step.
        app.update();
        assert_eq!(app.world().resource::<SimTick>().0, 1);

        // A long frame of three periods: three catch-up steps in one frame.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 3));
        app.update();
        assert_eq!(
            app.world().resource::<SimTick>().0,
            4,
            "a frame spanning several periods must run (and count) every step"
        );
    }

    /// The reconciler applies the authored `[global] sim_tick_hz`, and leaves
    /// apps without a `WorldConfig` alone.
    #[test]
    fn fixed_timestep_follows_the_authored_rate() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.add_systems(First, reconcile_fixed_timestep);

        // No WorldConfig: whatever the harness set stands.
        let harness_period = std::time::Duration::from_millis(200);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(harness_period);
        app.update();
        assert_eq!(
            app.world().resource::<Time<Fixed>>().timestep(),
            harness_period,
            "without a WorldConfig the reconciler must not touch the timestep"
        );

        // With one: the authored rate wins.
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.global.sim_tick_hz = 100.0;
        app.insert_resource(cfg);
        app.update();
        assert_eq!(
            app.world().resource::<Time<Fixed>>().timestep(),
            sim_tick_period(100.0),
            "an authored sim_tick_hz must reconfigure Time<Fixed>"
        );
    }

    /// A `GamePhase` change written from `FixedUpdate` lands on a TICK
    /// boundary, and the same schedule crosses it identically whether the host
    /// runs one fixed step per frame or four.
    ///
    /// This is the frame-pacing hole [`register_fixed_state_transition`]
    /// closes. With `StateTransition` left frame-only, a four-step frame
    /// applies the change at the START of the NEXT frame, so the three steps
    /// after the writer still run under the stale phase — the number of ticks
    /// spent in the old phase, and the tick `OnEnter` spawns land on, would
    /// then be a function of the frame rate.
    #[test]
    fn phase_transitions_land_on_the_same_tick_whatever_the_frame_pacing() {
        use crate::messages::GamePhase;

        /// The step the writer flips the phase on, and the total steps driven.
        const SWITCH_ON: u64 = 5;
        const STEPS: u64 = 12;

        #[derive(Resource, Default, Debug, PartialEq, Eq)]
        struct Crossing {
            /// The tick `OnEnter(InProgress)` observed.
            entered_on: Option<u64>,
            /// Fixed steps that ran under `in_state(InProgress)`.
            steps_in_progress: u64,
        }

        fn flip_on_the_fifth_tick(tick: Res<SimTick>, mut next: ResMut<NextState<GamePhase>>) {
            if tick.0 == SWITCH_ON {
                next.set(GamePhase::InProgress);
            }
        }

        fn record_enter(tick: Res<SimTick>, mut crossing: ResMut<Crossing>) {
            crossing.entered_on.get_or_insert(tick.0);
        }

        fn count_in_progress_steps(mut crossing: ResMut<Crossing>) {
            crossing.steps_in_progress += 1;
        }

        fn cross(ticks_per_frame: u32) -> Crossing {
            let period = std::time::Duration::from_millis(10);
            let mut app = App::new();
            app.add_plugins(bevy::time::TimePlugin)
                .add_plugins(bevy::state::app::StatesPlugin)
                .init_state::<GamePhase>()
                .init_resource::<Crossing>();
            register_sim_tick(&mut app);
            app.world_mut()
                .resource_mut::<Time<Fixed>>()
                .set_timestep(period);
            app.add_systems(FixedUpdate, flip_on_the_fifth_tick)
                .add_systems(
                    FixedUpdate,
                    count_in_progress_steps.run_if(in_state(GamePhase::InProgress)),
                )
                .add_systems(OnEnter(GamePhase::InProgress), record_enter);
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                period * ticks_per_frame,
            ));
            // The first update carries a zero delta and runs no step.
            app.update();
            for _ in 0..(STEPS / ticks_per_frame as u64) {
                app.update();
            }
            assert_eq!(
                app.world().resource::<SimTick>().0,
                STEPS,
                "precondition: {ticks_per_frame} tick(s) per frame must still \
                 cover {STEPS} steps"
            );
            std::mem::take(&mut *app.world_mut().resource_mut::<Crossing>())
        }

        let per_tick = cross(1);
        let per_four = cross(4);

        assert_eq!(
            per_tick.entered_on,
            Some(SWITCH_ON),
            "OnEnter must run inside the step that wrote NextState"
        );
        // Steps 6..=11 run under the new phase; step 5 wrote the change after
        // its own gated systems had already been skipped.
        assert_eq!(per_tick.steps_in_progress, STEPS - SWITCH_ON - 1);
        assert_eq!(
            per_tick, per_four,
            "a four-step frame must cross the phase boundary on the SAME \
             logical tick as a one-step frame — a difference means the \
             transition is still frame-timed and the steps after the writer \
             ran under the stale phase"
        );
    }
}
