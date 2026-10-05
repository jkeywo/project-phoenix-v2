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
pub use phoenix_sim_contracts::sim_tick::*;

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

#[cfg(test)]
#[path = "sim_tick_tests.rs"]
mod tests;
