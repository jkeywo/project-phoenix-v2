//! Clock controls for an explicitly disposable offline Test runtime. This
//! plugin is never installed in a Live host. Fixed schedules remain the
//! ordinary simulation schedules; stepping only feeds their existing clock.
pub use crate::workshop::test_protocol::TestControl;
use bevy::{
    prelude::*,
    time::{TimeSystems, TimeUpdateStrategy},
};
use std::collections::VecDeque;

/// Requests arrive solely over the disposable runtime's private control channel. The queue
/// is bounded before entering the app, and neither source nor ECS edits exist
/// in this vocabulary.
#[derive(Resource, Default)]
pub struct TestControls(pub VecDeque<TestControl>);

/// This run is disposable. Lifecycle capture must stay absent even if a target
/// accidentally registers a Live save consumer beside the shared simulation.
#[derive(Resource)]
pub struct DisposableTest;

#[derive(Resource, Debug)]
pub struct TestClock {
    pub paused: bool,
    pub multiplier: u8,
    pub(crate) steps: u8,
    pub(crate) stepping: bool,
    last_frame: Option<bevy::platform::time::Instant>,
}
impl Default for TestClock {
    fn default() -> Self {
        Self {
            paused: false,
            multiplier: 1,
            steps: 0,
            stepping: false,
            last_frame: None,
        }
    }
}

pub struct TestClockPlugin;
impl Plugin for TestClockPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<TestControls>(StateClass::Timer, "gm-milestone-integrated-workshop")
            .declare_state::<TestClock>(StateClass::Timer, "gm-milestone-integrated-workshop")
            .declare_state::<DisposableTest>(
                StateClass::Derived,
                "gm-milestone-integrated-workshop",
            )
            .insert_resource(DisposableTest)
            .init_resource::<TestControls>()
            .declare_state::<crate::presentation_contracts::TestWindowVisibility>(
                StateClass::TestInfra,
                "gm-milestone-integrated-workshop",
            )
            .init_resource::<crate::presentation_contracts::TestWindowVisibility>()
            .init_resource::<TestClock>()
            .init_resource::<super::test_breakpoint::TestBreakpointState>()
            .init_resource::<super::test_trace::TestTrace>()
            .init_resource::<crate::gm_action::SimulationPaused>()
            .add_systems(
                First,
                apply_controls
                    .after(crate::sim_tick::reconcile_fixed_timestep)
                    .before(TimeSystems),
            )
            .add_systems(
                FixedLast,
                super::test_breakpoint::evaluate_breakpoint
                    .after(crate::sim_tick::advance_sim_tick),
            )
            .add_systems(Last, finish_step);
        super::test_view::install(app);
    }
}

pub fn apply_controls(
    mut requests: ResMut<TestControls>,
    mut clock: ResMut<TestClock>,
    mut strategy: ResMut<TimeUpdateStrategy>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut fixed_time: ResMut<Time<Fixed>>,
    mut paused: ResMut<crate::gm_action::SimulationPaused>,
    mut exit: MessageWriter<AppExit>,
    mut visibility: ResMut<crate::presentation_contracts::TestWindowVisibility>,
    mut requested_view: ResMut<super::test_view::TestViewState>,
    mut breakpoint: ResMut<super::test_breakpoint::TestBreakpointState>,
) {
    // Keep this local wall-clock cursor moving while held. Always feed manual
    // durations: switching a manually stepped Real clock back to Automatic
    // would otherwise repay the entire paused interval as simulation time.
    let now = bevy::platform::time::Instant::now();
    let elapsed = clock
        .last_frame
        .replace(now)
        .map_or(std::time::Duration::ZERO, |previous| {
            now.saturating_duration_since(previous)
        });
    for request in requests.0.drain(..) {
        match request {
            TestControl::Pause {} => {
                clock.paused = true;
                clock.steps = 0;
            }
            TestControl::Resume {} => {
                clock.paused = false;
                clock.steps = 0;
                breakpoint.release();
            }
            TestControl::Step {} if clock.paused => {
                clock.steps = clock.steps.saturating_add(1).min(8);
                breakpoint.release();
            }
            TestControl::Rate { multiplier } if matches!(multiplier, 1 | 2 | 4 | 8) => {
                clock.multiplier = multiplier;
            }
            TestControl::Stop {} => {
                exit.write(AppExit::Success);
            }
            TestControl::Visibility { visible } => {
                visibility.0 = Some(visible);
            }
            // Recorded here, applied by `test_view::apply_test_view`, which has
            // the Commands this system deliberately does not.
            TestControl::View { view } => {
                requested_view.requested = view;
            }
            _ => {}
        }
    }
    clock.stepping = clock.paused && clock.steps > 0;
    paused.0 = clock.paused && !clock.stepping;
    if clock.paused {
        // A partial rendered frame must not make Step run more (or less) than
        // one authored fixed tick, including after acceleration and Pause.
        let remainder = fixed_time.overstep();
        fixed_time.discard_overstep(remainder);
        virtual_time.set_relative_speed(1.0);
        if clock.stepping {
            clock.steps -= 1;
            virtual_time.unpause();
            *strategy = TimeUpdateStrategy::ManualDuration(fixed_time.timestep());
        } else {
            virtual_time.pause();
            *strategy = TimeUpdateStrategy::ManualDuration(std::time::Duration::ZERO);
        }
    } else {
        virtual_time.unpause();
        virtual_time.set_relative_speed(f32::from(clock.multiplier));
        *strategy = TimeUpdateStrategy::ManualDuration(elapsed);
    }
}

pub fn finish_step(
    mut clock: ResMut<TestClock>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut fixed_time: ResMut<Time<Fixed>>,
    mut paused: ResMut<crate::gm_action::SimulationPaused>,
) {
    if clock.stepping {
        clock.stepping = false;
        virtual_time.pause();
        // The visible status reports the completed tick as held immediately;
        // neither the next rendered frame nor a second click is owed a tick.
        virtual_time.advance_by(std::time::Duration::ZERO);
        let remainder = fixed_time.overstep();
        fixed_time.discard_overstep(remainder);
        paused.0 = true;
    }
}

#[cfg(test)]
#[path = "test_clock_tests.rs"]
mod tests;
