//! Clock fixture shared by simulation and application integration tests.
use bevy::prelude::*;
/// Discard whatever overstep `Time<Fixed>` has accumulated so far.
///
/// A fixture that re-paces the fixed clock — a new timestep, a hand-rolled
/// `TimeUpdateStrategy`, or both — must clear this first, or overstep
/// computed under the OLD pacing leaks into the new one. `test_app()` (via
/// `drive_one_fixed_step_per_update`) preloads one `TEST_TICK` (200 ms) of
/// overstep before the fixture's first `app.update()` ever runs; a fixture
/// that re-paces to a finer timestep before that first update — as the
/// shared-AI-tick throttle tests do, swapping the coarse `TEST_TICK` pacing
/// for a fine 10 ms one before driving any frames — inherits that stale
/// 200 ms untouched if it skips this call, and bursts however many of the
/// new, finer steps fit inside it the moment the first frame runs.
pub fn discard_stale_overstep(app: &mut App) {
    let mut fixed = app.world_mut().resource_mut::<Time<Fixed>>();
    let stale = fixed.overstep();
    fixed.discard_overstep(stale);
}

/// Drive exactly one fixed simulation step per `app.update()` (issue #895).
///
/// The production sim lives in `FixedUpdate`, so a fixture has to step the
/// fixed clock or nothing runs. This pins `Time<Fixed>`'s timestep to
/// `period` and drives the app with `TimeUpdateStrategy::FixedTimesteps(1)`,
/// which re-reads the CURRENT `Time<Fixed>` timestep every frame and always
/// contributes exactly one timestep's worth of delta to the accumulator —
/// unlike `ManualDuration`, which bakes a `Duration` in at insert time and
/// would silently drift if a later call re-paced the timestep without also
/// refreshing the strategy resource. Since this function always keeps the
/// two in lockstep anyway, `FixedTimesteps(1)` costs nothing extra and is
/// the more defensive of the two.
///
/// Bevy's very first `app.update()` ever run on a fresh `App` reports a ZERO
/// delta on `Time<Real>` — there is no previous update instant to diff
/// against, by design (`bevy_time::real::Time::<Real>::update_with_instant`:
/// "delta() and elapsed() will report zero on the first update"). Every
/// `TimeUpdateStrategy` variant routes through that same first-call path, so
/// switching strategies cannot dodge it: only an app that has never had
/// `update()` called needs a manual preload to make its first frame still
/// run a step. This function detects that case (`Time<Real>::last_update()`
/// is `None`) and preloads the accumulator with one `period` only then — the
/// caller's first `update()` is therefore a real tick, matching the
/// one-tick-per-update semantics every fixture in this crate is written
/// against, with no extra warm-up frame that would run `Startup` before the
/// test has finished arranging its world.
///
/// Safe to call again with a different `period` to re-pace an app that has
/// already taken at least one `update()`: the stale overstep is discarded
/// (see `discard_stale_overstep`) and, because `update()` has already run at
/// least once, the fresh-app preload above does NOT fire — preloading here
/// too would double-count against the delta the very next frame already
/// contributes, running two fixed steps instead of one. It is NOT safe to
/// call this a second time on a fresh app that still has not taken its first
/// `update()` — the discard makes repeated pre-update calls idempotent, but
/// that path exists to be idempotent, not to model a genuine mid-test
/// re-pace.
pub fn drive_one_fixed_step_per_update(app: &mut App, period: std::time::Duration) {
    // `Time<Virtual>` clamps each frame's delta to its `max_delta` (250 ms by
    // default) before the fixed accumulator sees it — a fixture pacing itself
    // in whole seconds would silently under-feed the fixed clock and run a
    // step only every fourth update.
    {
        let mut virt = app.world_mut().resource_mut::<Time<Virtual>>();
        if period > virt.max_delta() {
            virt.set_max_delta(period);
        }
    }
    let fresh = app.world().resource::<Time<Real>>().last_update().is_none();
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);
    discard_stale_overstep(app);
    if fresh {
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .accumulate_overstep(period);
    }
    app.insert_resource(bevy::time::TimeUpdateStrategy::FixedTimesteps(1));
}
