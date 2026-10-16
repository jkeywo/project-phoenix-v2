use super::*;
use bevy::ecs::schedule::{ApplyDeferred, ExecutorKind, Schedule};

#[derive(Resource)]
struct Published;
#[derive(Resource, Default)]
struct Observations(Vec<bool>);

fn publish(mut commands: Commands) {
    commands.insert_resource(Published);
}
fn observe(published: Option<Res<Published>>, mut observations: ResMut<Observations>) {
    observations.0.push(published.is_some());
}
fn declaration() -> DeclaredOrder {
    let mut order = DeclaredOrder::default();
    order.before(
        FixedStep::TickTriggerPipeline,
        FixedStep::TickScriptCallbacks,
    );
    order
}

#[test]
fn execution_edges_preserve_an_existing_deferred_publication() {
    let mut schedule = Schedule::default();
    schedule.set_executor_kind(ExecutorKind::SingleThreaded);
    schedule.add_systems(
        (
            publish.in_set(FixedStep::TickTriggerPipeline),
            observe.in_set(FixedStep::TickScriptCallbacks),
        )
            .chain(),
    );
    schedule.add_build_pass(declaration());
    let mut world = World::new();
    world.init_resource::<Observations>();
    schedule.run(&mut world);
    assert_eq!(world.resource::<Observations>().0, [true]);
    assert!(schedule
        .systems()
        .unwrap()
        .any(|(_, system)| { system.type_id() == std::any::TypeId::of::<ApplyDeferred>() }));
}

#[test]
fn execution_order_does_not_publish_commands_at_a_new_boundary() {
    let mut schedule = Schedule::default();
    schedule.set_executor_kind(ExecutorKind::SingleThreaded);
    schedule.add_systems((
        publish.in_set(FixedStep::TickTriggerPipeline),
        observe.in_set(FixedStep::TickScriptCallbacks),
    ));
    schedule.add_build_pass(declaration());
    let mut world = World::new();
    world.init_resource::<Observations>();
    schedule.run(&mut world);
    assert_eq!(world.resource::<Observations>().0, [false]);
    assert!(
        world.contains_resource::<Published>(),
        "final flush still runs"
    );
    assert!(schedule
        .systems()
        .unwrap()
        .all(|(_, system)| { system.type_id() != std::any::TypeId::of::<ApplyDeferred>() }));
}

#[test]
fn contradictory_declared_execution_order_is_a_schedule_error() {
    let mut schedule = Schedule::default();
    schedule.add_systems((
        publish
            .in_set(FixedStep::TickTriggerPipeline)
            .after(observe),
        observe.in_set(FixedStep::TickScriptCallbacks),
    ));
    schedule.add_build_pass(declaration());
    let mut world = World::new();
    world.init_resource::<Observations>();
    let error = schedule.initialize(&mut world).unwrap_err();
    assert!(matches!(
        &error,
        ScheduleBuildError::FlatDependencySort(DiGraphToposortError::Cycle(cycles))
            if cycles.len() == 1 && cycles[0].len() == 2
    ));
    let message = error.to_string(schedule.graph(), &world);
    assert!(message.contains("publish"), "{message}");
    assert!(message.contains("observe"), "{message}");
}

#[test]
fn security_acquisition_completion_keeps_one_ordered_clock_owner() {
    fn following_owner() {}

    // Exercise the actual plugin registration, rather than reproducing its
    // tuple: the fixed-order singleton check must accept this real schedule.
    let mut app = App::new();
    app.add_plugins(crate::security::server::SecurityPlugin);
    app.add_systems(
        FixedUpdate,
        following_owner
            .in_set(FixedStep::RevealCivilianContacts)
            .in_set(crate::sim_sets::SimSet::Modifiers),
    );
    let mut schedule = app
        .world_mut()
        .resource_mut::<Schedules>()
        .remove(FixedUpdate)
        .unwrap();
    let mut order = DeclaredOrder::default();
    order.before(
        FixedStep::TickSecurityTeams,
        FixedStep::RevealCivilianContacts,
    );
    schedule.add_build_pass(order);
    schedule.initialize(app.world_mut()).unwrap();

    let names: Vec<_> = schedule
        .systems()
        .unwrap()
        .map(|(_, system)| system.name().to_string())
        .collect();
    let position = |suffix: &str| {
        let matches: Vec<_> = names
            .iter()
            .enumerate()
            .filter_map(|(index, name)| name.ends_with(suffix).then_some(index))
            .collect();
        assert_eq!(matches.len(), 1, "one actual {suffix}: {names:?}");
        matches[0]
    };
    let clock = position("::tick_security_teams");
    let completion = position("::commit_ready_acquisitions");
    let following = position("::following_owner");
    assert!(clock < completion && completion < following, "{names:?}");
}
