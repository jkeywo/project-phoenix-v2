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
/// `PreUpdate`, i.e. once per rendered frame. Since issue #1121's fix round
/// every *production* `NextState<GamePhase>` writer — the JS bridge's
/// force-start (`server::bridge::apply_force_start`), the asset preloader
/// (`server::asset_preload::auto_transition_from_loading`), headless' and
/// the native host's auto-start (`headless::app::headless_auto_start`,
/// `native_host::solo_auto_start`) and the lobby countdown
/// (`lobby::server::tick_countdown`) — writes from `FixedUpdate` instead, so
/// this frame-level site no longer has a production writer to serve. It stays
/// registered anyway, for bare-`App` fixtures and test drivers that write the
/// phase from a frame schedule (e.g. `tests/headless_runner.rs` setting
/// `NextState<GamePhase>` directly rather than going through a fixed system)
/// — this adds a SECOND run site rather than moving the first.
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
    // Issue #907: the id mint is scoped to this counter, so it is registered
    // here rather than beside the other resources — the tick and the thing
    // that is tick-scoped cannot get out of step if they are wired together.
    // `FixedFirst`, so every sim system in the step mints against the index of
    // the step it is running in (see `world_id::sync_world_id_mint`).
    app.init_resource::<crate::world_id::WorldIdMint>()
        .init_resource::<crate::world_id::EntityMint>()
        .init_resource::<crate::world_id::AsteroidMint>()
        .init_resource::<crate::world_id::MessageMint>()
        .init_resource::<crate::world_id::ProjectileMint>()
        .add_systems(FixedFirst, crate::world_id::sync_world_id_mint);
}

#[cfg(test)]
#[path = "sim_tick_tests.rs"]
mod tests;
