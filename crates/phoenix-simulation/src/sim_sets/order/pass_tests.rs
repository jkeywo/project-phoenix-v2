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
    assert!(matches!(
        schedule.initialize(&mut world),
        Err(ScheduleBuildError::FlatDependencySort(_))
    ));
}
