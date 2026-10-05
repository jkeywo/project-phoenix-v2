//! Fixed-step timing and identity registration shared by simulation branches.
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

/// Convert an integer-seconds delay to a whole number of sim ticks at `hz`.
///
/// Rounds to the nearest tick, so a scenario authored in seconds lands on a tick
/// boundary the same way every peer computes it (identical `hz`, identical
/// rounding). A non-positive delay fires on the next tick (`0`), matching the
/// delayed-action queue's boundary-inclusive `fire_at`.
pub fn seconds_to_ticks(secs: i64, hz: f32) -> u64 {
    if secs <= 0 {
        return 0;
    }
    ((secs as f64) * (hz as f64)).round().max(0.0) as u64
}
