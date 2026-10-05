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

#[test]
fn state_breakpoint_holds_at_the_completed_tick_and_step_does_not_retrigger() {
    use crate::workshop::test_protocol::{TestBreakpoint, TestBreakpointCondition};
    let mut app = app();
    app.world_mut()
        .insert_resource(crate::world::server::WorldContentRuntime::default());
    app.world_mut()
        .resource_mut::<super::super::test_breakpoint::TestBreakpointState>()
        .configured = Some(TestBreakpoint {
        layer: None,
        condition: TestBreakpointCondition::Flag {
            name: "arrived".into(),
            value: true,
        },
    });
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("arrived");
    app.world_mut().run_schedule(FixedLast);
    assert_eq!(app.world().resource::<crate::sim_tick::SimTick>().0, 1);
    assert!(app.world().resource::<TestClock>().paused);
    assert_eq!(
        app.world()
            .resource::<super::super::test_breakpoint::TestBreakpointState>()
            .hit
            .as_ref()
            .unwrap()
            .tick,
        1
    );
    request(&mut app, TestControl::Step {});
    assert_eq!(app.world().resource::<crate::sim_tick::SimTick>().0, 2);
    assert!(app
        .world()
        .resource::<super::super::test_breakpoint::TestBreakpointState>()
        .hit
        .is_none());
}

#[test]
fn composed_extra_world_breakpoint_reads_the_loaded_layer_store() {
    use crate::workshop::test_protocol::{TestBreakpoint, TestBreakpointCondition};
    let mut app = app();
    app.world_mut()
        .insert_resource(crate::world::server::WorldContentRuntime::default());
    let mut layer = crate::world::server::WorldRuntime {
        is_active: true,
        ..Default::default()
    };
    layer.flags.set_flag("layer_arrived");
    let mut layers = crate::world::server::WorldLayerMap::default();
    layers
        .0
        .insert("assets/worlds/test-layer.toml".into(), layer);
    app.world_mut().insert_resource(layers);
    app.world_mut()
        .resource_mut::<super::super::test_breakpoint::TestBreakpointState>()
        .configured = Some(TestBreakpoint {
        layer: Some("assets/worlds/test-layer.toml".into()),
        condition: TestBreakpointCondition::Flag {
            name: "layer_arrived".into(),
            value: true,
        },
    });

    app.world_mut().run_schedule(FixedLast);

    let hit = app
        .world()
        .resource::<super::super::test_breakpoint::TestBreakpointState>()
        .hit
        .as_ref()
        .expect("the loaded extra-world Flag holds Test");
    assert_eq!(hit.tick, 1);
    assert_eq!(
        hit.breakpoint.layer.as_deref(),
        Some("assets/worlds/test-layer.toml")
    );
}
