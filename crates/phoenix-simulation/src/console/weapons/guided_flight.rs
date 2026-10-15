//! Preserve the existing actual guided-flight vector when its launcher goes.
//! A carrier is not a ship, controller, launcher or second projectile store.
use crate::console::weapons::TorpedoSystemResource;
use crate::weapons::torpedo::{Torpedo, TorpedoConfig, TorpedoSystem};
use bevy::prelude::*;

#[derive(Component, Debug)]
pub struct GuidedFlightCarrier;

/// Exclusive roster replacement must discard bootstrap flights rather than
/// rehome them as new physical work. This transient guard never enters saves.
#[derive(Resource, Default)]
pub(crate) struct RestoreGuard(pub bool);

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<GuidedFlightCarrier>(StateClass::Folded, "guided-flight-carrier-state");
    app.declare_state::<RestoreGuard>(StateClass::Cache, "guided-flight-restore-guard");
    app.world_mut()
        .register_component_hooks::<TorpedoSystemResource>()
        .on_replace(rehome);
}

pub(crate) fn empty() -> TorpedoSystemResource {
    TorpedoSystemResource(TorpedoSystem::from_configs(
        &[],
        TorpedoConfig {
            count: 0,
            initial_count: Some(0),
            ..Default::default()
        },
    ))
}

fn rehome(mut world: bevy::ecs::world::DeferredWorld, context: bevy::ecs::lifecycle::HookContext) {
    if world
        .get_resource::<RestoreGuard>()
        .is_some_and(|guard| guard.0)
        || world
            .get_resource::<State<crate::core::messages::GamePhase>>()
            .is_some_and(|phase| *phase.get() != crate::core::messages::GamePhase::InProgress)
    {
        return;
    }
    let Some(mut owner) = world.get_mut::<TorpedoSystemResource>(context.entity) else {
        return;
    };
    let flights = std::mem::take(&mut owner.0.in_flight);
    if flights.is_empty() {
        return;
    }
    let source = context.entity;
    world.commands().queue(move |world: &mut World| {
        // A replacement may contain a clone of its old flights. The old actual
        // vector wins; the new launcher cannot duplicate or upgrade those IDs.
        if let Some(mut replacement) = world.get_mut::<TorpedoSystemResource>(source) {
            replacement
                .0
                .in_flight
                .retain(|round| !flights.iter().any(|old| old.uuid == round.uuid));
        }
        spawn(world, flights);
    });
}

fn spawn(world: &mut World, flights: Vec<Torpedo>) -> Entity {
    let uuid = crate::world_id::mint_id_with(
        world.get_resource::<crate::world_id::WorldIdMint>(),
        crate::world_id::IdNamespace::Entity,
    );
    let entity = spawn_empty(world, uuid);
    world
        .get_mut::<TorpedoSystemResource>(entity)
        .unwrap()
        .0
        .in_flight = flights;
    entity
}

pub(crate) fn spawn_empty(world: &mut World, uuid: String) -> Entity {
    world
        .spawn((
            crate::entities::spawner::EntityUuid(uuid),
            GuidedFlightCarrier,
            empty(),
        ))
        .id()
}

/// Restore only changes this guard around the actual roster replacement. Every
/// refusal occurs before it, and callers restore a preexisting nested value.
pub(crate) fn suppress(world: &mut World, value: bool) -> bool {
    let previous = world
        .get_resource::<RestoreGuard>()
        .is_some_and(|guard| guard.0);
    world.insert_resource(RestoreGuard(value));
    previous
}

#[cfg(test)]
#[path = "guided_flight_tests.rs"]
mod tests;
