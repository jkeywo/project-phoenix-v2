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
    steps: u8,
    stepping: bool,
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
            .init_resource::<TestClock>()
            .init_resource::<crate::gm_action::SimulationPaused>()
            .add_systems(
                First,
                apply_controls
                    .after(crate::sim_tick::reconcile_fixed_timestep)
                    .before(TimeSystems),
            )
            .add_systems(Last, finish_step);
    }
}

pub(crate) fn apply_controls(
    mut requests: ResMut<TestControls>,
    mut clock: ResMut<TestClock>,
    mut strategy: ResMut<TimeUpdateStrategy>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut fixed_time: ResMut<Time<Fixed>>,
    mut paused: ResMut<crate::gm_action::SimulationPaused>,
    mut exit: MessageWriter<AppExit>,
    mut windows: Query<&mut Window>,
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
            }
            TestControl::Step {} if clock.paused => {
                clock.steps = clock.steps.saturating_add(1).min(8);
            }
            TestControl::Rate { multiplier } if matches!(multiplier, 1 | 2 | 4 | 8) => {
                clock.multiplier = multiplier;
            }
            TestControl::Stop {} => {
                exit.write(AppExit::Success);
            }
            TestControl::Visibility { visible } => {
                for mut window in &mut windows {
                    window.visible = visible;
                }
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

pub(crate) fn finish_step(
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
mod tests {
    use super::*;
    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        crate::sim_tick::register_sim_tick(&mut app);
        app.add_plugins(TestClockPlugin);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(std::time::Duration::from_millis(10));
        app.update();
        app
    }
    fn request(app: &mut App, request: TestControl) {
        app.world_mut()
            .resource_mut::<TestControls>()
            .0
            .push_back(request);
        app.update();
    }
    #[test]
    fn pause_holds_every_fixed_schedule_and_each_step_advances_exactly_one_tick() {
        let mut app = app();
        request(&mut app, TestControl::Pause {});
        let before = app.world().resource::<crate::sim_tick::SimTick>().0;
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(app.world().resource::<crate::sim_tick::SimTick>().0, before);
        // Emulate a leftover partial frame from accelerated play.
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .accumulate_overstep(std::time::Duration::from_millis(9));
        request(&mut app, TestControl::Step {});
        assert_eq!(
            app.world().resource::<crate::sim_tick::SimTick>().0,
            before + 1
        );
        assert!(
            app.world()
                .resource::<crate::gm_action::SimulationPaused>()
                .0
        );
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        app.update();
        assert_eq!(
            app.world().resource::<crate::sim_tick::SimTick>().0,
            before + 1
        );
        request(&mut app, TestControl::Step {});
        assert_eq!(
            app.world().resource::<crate::sim_tick::SimTick>().0,
            before + 2
        );
    }
    #[test]
    fn rates_are_finite_and_resuming_discards_queued_steps() {
        let mut app = app();
        request(&mut app, TestControl::Rate { multiplier: 4 });
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            4.0
        );
        request(&mut app, TestControl::Rate { multiplier: 255 });
        assert_eq!(app.world().resource::<TestClock>().multiplier, 4);
        request(&mut app, TestControl::Pause {});
        app.world_mut()
            .resource_mut::<TestControls>()
            .0
            .extend([TestControl::Step {}, TestControl::Resume {}]);
        app.update();
        assert!(!app.world().resource::<TestClock>().paused);
        assert_eq!(app.world().resource::<TestClock>().steps, 0);
        assert!(
            !app.world()
                .resource::<crate::gm_action::SimulationPaused>()
                .0
        );
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            4.0
        );
    }
}
