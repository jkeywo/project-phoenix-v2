//! The single AI decision cadence (issue #889).
//!
//! Before this module the AI's decision cadence was fragmented three ways:
//! the six per-axis helm systems ran on an `AiHelmTickTimer` at the
//! TOML-authored `[global] ai_helm_tick_hz`; Captain and Sensors ran on a
//! *hardcoded* 10 Hz `AiSnapshotTimer` gated inside the system body by an
//! `Option<Res<_>>` that fell back to evaluating every tick when absent; and
//! seven further deciders (shield focus, power allocation, torpedo auto-fire,
//! torpedo load, frequency hint, phaser auto-fire, blaster auto-fire, AI target
//! selection) had no gate at all. Because `SimSet` is configured in Bevy's
//! `Update`, "no gate" means **once per rendered frame** — decisions taken at
//! display refresh rate over a `WorldSnapshot` rebuilt on an unrelated clock.
//!
//! There is now exactly ONE timer. [`AiTickTimer`] runs at the authored
//! `[global] ai_tick_hz` (the old `ai_helm_tick_hz` key remains a serde alias,
//! so every shipped world TOML keeps working) and sets [`AiTickReady`]. The
//! slower snapshot cadence is **derived** from it as an integer multiple —
//! `ai_tick_hz / ai_snapshot_hz` base ticks per snapshot tick — rather than
//! being a second, independently-drifting `Timer`. A non-integer relationship
//! between the two authored rates is rejected at world load
//! (`world::config::parse_world`), so the two AI clocks can never be
//! commensurate only by luck.
//!
//! # Why a latch resource rather than `Timer` in a run condition
//! `run_if` conditions must take read-only parameters, so the timer is advanced
//! by a dedicated system ([`tick_ai_cadence`]) that writes the two boolean
//! latches, which the conditions then read. The tick system is registered in
//! Bevy's `Last` schedule: every gated system lives in `Update`, so `Last`
//! guarantees the flag is consumed before it is re-armed **without** needing an
//! explicit `.after()` edge against each of the fifteen gated systems (which
//! would silently degrade to an empty constraint in any fixture that does not
//! register them all).
//!
//! # Free-run on the first update
//! Both latches initialise to `true` so the very first `Update` always decides,
//! before the timer has had a chance to fire. This mirrors the pre-#889
//! behaviour of both `AiHelmTickReady` and `AiSnapshotReady`.

use bevy::prelude::*;

/// The single repeating timer behind every AI decision gate.
///
/// Period is the authored `[global] ai_tick_hz` (serde default 30 Hz). The
/// resource is created at plugin build, before any `WorldConfig` exists, so
/// [`tick_ai_cadence`] reconciles the period against the loaded world config on
/// each frame (a cheap duration-equality check that only writes when the
/// authored rate differs).
#[derive(Resource)]
pub struct AiTickTimer(pub Timer);

impl Default for AiTickTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(
            1.0 / crate::entity_config::GlobalConfig::default().ai_tick_hz,
            TimerMode::Repeating,
        ))
    }
}

/// Boolean latch set each frame by [`tick_ai_cadence`]: `true` on base-cadence
/// ticks, `false` on every other rendered frame. Read by [`ai_tick_ready`].
#[derive(Resource)]
pub struct AiTickReady(pub bool);

/// Boolean latch for the DERIVED slower cadence — `true` on every
/// `ai_tick_hz / ai_snapshot_hz`-th base tick. Read by [`ai_snapshot_ready`].
///
/// Gates the world-snapshot / doctrine-aggregation rebuild and the two policy
/// hosts that have always run on that slower clock (Captain, Sensors).
#[derive(Resource)]
pub struct AiSnapshotReady(pub bool);

/// How many base ticks have elapsed since the last snapshot tick. Private to
/// this module: it is the derivation of [`AiSnapshotReady`] from
/// [`AiTickReady`], not a second clock.
#[derive(Resource, Default)]
pub struct AiSnapshotPhase(u32);

/// Advance [`AiTickTimer`] and set both latches.
///
/// Registered in `Last` by [`register_ai_cadence`], so it always runs after
/// every gated `Update` system: the flag is consumed before it is re-armed.
/// Also reconciles the timer period against the TOML-authored
/// `[global] ai_tick_hz` once `WorldConfig` exists — the timer resource is
/// created at plugin build, before the world TOML has been parsed.
pub fn tick_ai_cadence(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut timer: ResMut<AiTickTimer>,
    mut ready: ResMut<AiTickReady>,
    mut phase: ResMut<AiSnapshotPhase>,
    mut snapshot_ready: ResMut<AiSnapshotReady>,
) {
    let mut snapshot_every = crate::entity_config::GlobalConfig::default().snapshot_every_ticks();
    if let Some(wc) = world_config.as_deref() {
        let hz = wc.global.ai_tick_hz;
        if hz > 0.0 {
            let configured = std::time::Duration::from_secs_f32(1.0 / hz);
            if timer.0.duration() != configured {
                timer.0.set_duration(configured);
            }
        }
        snapshot_every = wc.global.snapshot_every_ticks();
    }

    let fired = timer.0.tick(time.delta()).just_finished();
    ready.0 = fired;
    if fired {
        // The FIRST base tick is also a snapshot tick, then every
        // `snapshot_every`-th one after it.
        snapshot_ready.0 = phase.0 == 0;
        phase.0 = (phase.0 + 1) % snapshot_every.max(1);
    } else {
        snapshot_ready.0 = false;
    }
}

/// Read-only run condition: the shared AI base cadence.
pub fn ai_tick_ready(ready: Res<AiTickReady>) -> bool {
    ready.0
}

/// Read-only run condition: the derived slower snapshot cadence.
pub fn ai_snapshot_ready(ready: Res<AiSnapshotReady>) -> bool {
    ready.0
}

/// Install the shared cadence resources and the one system that advances them.
///
/// Idempotent, and deliberately a plain function rather than a `Plugin`: every
/// plugin that registers a gated system calls it, and duplicate registration of
/// [`tick_ai_cadence`] would advance the timer once per calling plugin.
pub fn register_ai_cadence(app: &mut App) {
    if app.world().contains_resource::<AiTickTimer>() {
        return;
    }
    app.init_resource::<AiTickTimer>()
        .insert_resource(AiTickReady(true))
        .init_resource::<AiSnapshotPhase>()
        .insert_resource(AiSnapshotReady(true))
        .add_systems(Last, tick_ai_cadence);
}

/// Re-arm both latches so the next `app.update()` is an AI decision tick.
///
/// Test-only. Fixtures that assert on decision CONTENT drive several updates
/// without advancing wall-clock time past the 33.3 ms period; they call this to
/// tick the latch by hand rather than relying on an evaluate-every-frame
/// fallback that production never takes. Fixtures that assert on CADENCE drive
/// `Time` instead and must not call this.
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

    fn cadence_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        register_ai_cadence(&mut app);
        app
    }

    /// One base tick per authored period, and the derived snapshot latch fires
    /// on exactly every third of them at the shipped 30 Hz / 10 Hz pair.
    #[test]
    fn snapshot_latch_is_an_integer_multiple_of_the_base_latch() {
        let mut app = cadence_app();
        // 40 ms per frame is longer than the 33.3 ms base period, so every
        // frame is a base tick and the phase counter is the only thing
        // separating base ticks from snapshot ticks.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(40),
        ));

        // Count over exactly nine BASE ticks, however many frames that takes —
        // the first frame carries a zero delta, so a fixed frame count would be
        // measuring the warm-up rather than the phase.
        let mut base = 0usize;
        let mut snapshot = 0usize;
        let mut frames = 0usize;
        while base < 9 {
            app.update();
            frames += 1;
            assert!(frames < 50, "the base latch never fired at 40 ms per frame");
            if app.world().resource::<AiTickReady>().0 {
                base += 1;
                if app.world().resource::<AiSnapshotReady>().0 {
                    snapshot += 1;
                }
            }
        }

        assert_eq!(
            snapshot, 3,
            "the snapshot cadence is DERIVED as every third base tick \
             (30 Hz / 10 Hz), not a second independent timer"
        );
    }

    /// The base cadence is TOML-authored, not hardcoded: an authored
    /// `[global] ai_tick_hz` must reconfigure the shared timer.
    #[test]
    fn base_rate_is_read_from_world_config() {
        let mut app = cadence_app();
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.global.ai_tick_hz = 100.0;
        app.insert_resource(cfg);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(11),
        ));

        let mut base = 0usize;
        for _ in 0..8 {
            app.update();
            if app.world().resource::<AiTickReady>().0 {
                base += 1;
            }
        }

        // 7 of 8, not 8 of 8: the first frame's delta is zero. At the default
        // 30 Hz this same drive fires at most twice, so the margin is wide.
        assert!(
            base >= 7,
            "with [global] ai_tick_hz = 100 the 10 ms period fires on every \
             11 ms frame — {base} of 8 means the authored rate was never applied"
        );
    }

    /// The frame-rate decoupling that is the whole point of the latch: at a
    /// frame period well under the authored tick period, most frames must NOT
    /// be decision ticks.
    #[test]
    fn base_latch_throttles_frames_shorter_than_the_period() {
        let mut app = cadence_app();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(10),
        ));

        let mut base = 0usize;
        for _ in 0..12 {
            app.update();
            if app.world().resource::<AiTickReady>().0 {
                base += 1;
            }
        }

        assert!(
            base > 0,
            "precondition: 12 frames x 10 ms spans several 33.3 ms periods"
        );
        assert!(
            base <= 6,
            "at 10 ms per frame — what a 60 Hz rAF-driven host does — the \
             33.3 ms base cadence must fire on at most half the frames; \
             {base} of 12 means the throttle is gone"
        );
    }
}
