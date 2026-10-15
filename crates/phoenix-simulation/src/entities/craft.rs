//! Actual physical probe custody and bay jobs. No identity registry or craft copy.
use super::{config::EntityConfig, spawner::EntityUuid};
use bevy::prelude::*;
use phoenix_model::{
    craft::{CraftLocation, ProbeProfile},
    identity::SystemId,
};
use phoenix_sim_gameplay::ship::craft::BayState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Component, Clone, Debug)]
pub struct PhysicalProbe(pub ProbeProfile);
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeCustody(pub CraftLocation);
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipBayJobs(pub BTreeMap<SystemId, BayState>);
/// Exclusive whole-roster restore is not a gameplay loss of the bootstrap parent.
#[derive(Resource, Default)]
pub(crate) struct RestoreGuard(pub bool);

pub(crate) fn install(config: &EntityConfig, position: Vec3, commands: &mut EntityCommands) {
    if let Some(ship) = &config.ship_config {
        let bays: BTreeMap<_, _> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::CRAFT_BAY_KIND)
            .map(|system| (system.id.clone(), BayState::default()))
            .collect();
        if !bays.is_empty() {
            commands.insert(ShipBayJobs(bays));
        }
    }
    if let Some(profile) = &config.probe {
        commands.insert((
            PhysicalProbe(profile.clone()),
            ProbeCustody::default(),
            crate::ship::state::ShipPhysics {
                x: position.x,
                y: position.y,
                z: position.z,
                ..Default::default()
            },
        ));
    }
}

pub(crate) fn register(app: &mut App) {
    use crate::{
        command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer},
        core::messages::SystemControlPayloadDiscriminants as Payload,
        sim_sets::{FixedStep, SimSet},
    };
    crate::ai::cadence::register_ai_cadence(app);
    for (payload, feedback) in [
        (Payload::LaunchCraft, &[Payload::LaunchCraft][..]),
        (Payload::RecoverCraft, &[Payload::RecoverCraft][..]),
        (
            Payload::CancelCraftHandling,
            &[Payload::CancelCraftHandling][..],
        ),
    ] {
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(crate::ship::system_registry::CRAFT_BAY_KIND, payload)
                .with_feedback(FeedbackAddress::DeclaredKindOnly, feedback),
        );
    }
    app.add_systems(
        FixedUpdate,
        ai::operate
            .in_set(SimSet::Physics)
            .in_set(FixedStep::OperateCraftBayAi)
            .run_if(crate::ai::cadence::ai_tick_ready)
            .run_if(any_with_component::<ShipBayJobs>),
    )
    .add_systems(
        FixedUpdate,
        runtime::handle
            .in_set(SimSet::Physics)
            .in_set(FixedStep::HandleCraftBayCommands)
            .run_if(any_with_component::<ShipBayJobs>),
    )
    .add_systems(
        FixedUpdate,
        runtime::commit
            .in_set(SimSet::Physics)
            .in_set(FixedStep::CommitCraftHandling)
            .run_if(any_with_component::<ShipBayJobs>),
    )
    .add_systems(
        FixedUpdate,
        runtime::tick
            .in_set(SimSet::Modifiers)
            .in_set(FixedStep::TickCraftHandling)
            .run_if(any_with_component::<ShipBayJobs>),
    )
    .add_systems(
        FixedUpdate,
        view::publish
            .in_set(SimSet::Publish)
            .in_set(FixedStep::PublishCraftBay),
    );
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<PhysicalProbe>(StateClass::Derived, "physical-probe-profile")
        .declare_state::<ProbeCustody>(StateClass::Folded, "physical-probe-custody")
        .declare_state::<ShipBayJobs>(StateClass::Folded, "actual-bay-jobs")
        .declare_state::<RestoreGuard>(StateClass::Cache, "craft-restore-guard");
    app.world_mut()
        .register_component_hooks::<EntityUuid>()
        .on_despawn(parent_removed);
}

fn parent_removed(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    if world
        .get_resource::<RestoreGuard>()
        .is_some_and(|guard| guard.0)
    {
        return;
    }
    let Some(uuid) = world
        .get::<EntityUuid>(context.entity)
        .map(|uuid| uuid.0.clone())
    else {
        return;
    };
    world.commands().queue(move |world: &mut World| {
        if world
            .get_resource::<RestoreGuard>()
            .is_some_and(|guard| guard.0)
        {
            return;
        }
        let now = world
            .get_resource::<crate::sim_tick::SimTick>()
            .map_or(0, |tick| tick.0);
        let mut jobs = world
            .query_filtered::<&mut ShipBayJobs, Allow<bevy::ecs::entity_disabling::Disabled>>();
        for mut bays in jobs.iter_mut(world) {
            for state in bays.0.values_mut() {
                state.fail_removed(&uuid, now);
            }
        }
        remove_carried_with_parent(world, &uuid);
    });
}
/// Shared actual despawn hook reaches scripted/GM, physical and layer removals.
/// The same deferred removal transaction drains actual contained bodies before
/// the next publication/checkpoint boundary. Deployed probes are independent.
fn remove_carried_with_parent(world: &mut World, parent: &str) {
    use bevy::ecs::entity_disabling::Disabled;
    let mut query = world.query_filtered::<(Entity, &EntityUuid, &ProbeCustody), Allow<Disabled>>();
    let mut lost: Vec<_> = query
        .iter(world)
        .filter_map(|(entity, uuid, custody)| {
            matches!(&custody.0,CraftLocation::Carried{parent_uuid,..} if parent_uuid==parent)
                .then_some((uuid.0.clone(), entity))
        })
        .collect();
    lost.sort_by(|a, b| a.0.cmp(&b.0));
    for (uuid, entity) in lost {
        if world.get_entity(entity).is_err() {
            continue;
        }
        if let Some(mut events) =
            world.get_resource_mut::<Messages<crate::ai::server::AiEntityDestroyed>>()
        {
            events.write(crate::ai::server::AiEntityDestroyed {
                entity_uuid: uuid.clone(),
            });
        } else if let Some(mut runtime) =
            world.get_resource_mut::<crate::world::server::WorldContentRuntime>()
        {
            runtime
                .pending_world_events
                .push(crate::world::content::WorldEvent::Destroyed { uuid: uuid.clone() });
        }
        let _ = crate::gm::operations::despawn::remove_entity(world, entity);
    }
}

pub(crate) fn restore_guard(world: &mut World, value: bool) -> bool {
    let previous = world
        .get_resource::<RestoreGuard>()
        .is_some_and(|guard| guard.0);
    world.insert_resource(RestoreGuard(value));
    previous
}

#[path = "craft_presence.rs"]
mod presence;
pub(crate) use presence::{restore_presence, set_carried, set_deployed};

#[path = "craft_ai.rs"]
mod ai;
#[path = "craft_bootstrap.rs"]
mod bootstrap;
#[path = "craft_runtime.rs"]
mod runtime;
#[path = "craft_view.rs"]
mod view;
pub(crate) use bootstrap::{initial_game_start_location, initial_location, stage_initial};
pub(crate) use runtime::context as bay_context;
pub(crate) use view::project;

#[cfg(test)]
#[path = "craft_tests.rs"]
mod tests;
