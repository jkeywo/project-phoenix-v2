//! Preserve existing emitted projectile vectors when their launcher goes.
//! A carrier is not a ship, controller, launcher or second projectile store.
use crate::console::weapons::{blaster::BlasterSystemResource, TorpedoSystemResource};
use crate::weapons::blaster::{BlasterBankConfig, BlasterSystem};
use crate::weapons::torpedo::{Torpedo, TorpedoConfig, TorpedoSystem};
use bevy::prelude::*;

#[derive(Component, Debug)]
pub struct WeaponFlightCarrier;

/// Exclusive roster replacement must discard bootstrap flights rather than
/// rehome them as new physical work. This transient guard never enters saves.
#[derive(Resource, Default)]
pub(crate) struct RestoreGuard(pub bool);

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SettleCarriers;

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<WeaponFlightCarrier>(StateClass::Folded, "weapon-flight-carrier-state");
    app.declare_state::<RestoreGuard>(StateClass::Cache, "weapon-flight-restore-guard");
    app.add_systems(
        FixedUpdate,
        settle_carriers
            .in_set(crate::sim_sets::SimSet::Modifiers)
            .in_set(SettleCarriers),
    );
    app.world_mut()
        .register_component_hooks::<TorpedoSystemResource>()
        .on_replace(rehome);
    app.world_mut()
        .register_component_hooks::<BlasterSystemResource>()
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

fn enabled(world: &bevy::ecs::world::DeferredWorld) -> bool {
    !world
        .get_resource::<RestoreGuard>()
        .is_some_and(|guard| guard.0)
        && world
            .get_resource::<State<crate::core::messages::GamePhase>>()
            .is_none_or(|phase| *phase.get() == crate::core::messages::GamePhase::InProgress)
}

/// A source removal/replacement is one flight rehome transaction, regardless
/// of which component hook Bevy invokes first. The next hook sees empty vectors.
fn rehome(mut world: bevy::ecs::world::DeferredWorld, context: bevy::ecs::lifecycle::HookContext) {
    if !enabled(&world) {
        return;
    }
    let flights = world
        .get_mut::<TorpedoSystemResource>(context.entity)
        .map(|mut owner| std::mem::take(&mut owner.0.in_flight))
        .unwrap_or_default();
    let banks: Vec<_> = world
        .get_mut::<BlasterSystemResource>(context.entity)
        .map(|mut owner| {
            owner
                .0
                .iter_mut()
                .filter_map(|bank| {
                    let flights = std::mem::take(&mut bank.in_flight);
                    if flights.is_empty() {
                        return None;
                    }
                    let mut carried = flight_bank(bank.config.id.clone());
                    carried.in_flight = flights;
                    Some(carried)
                })
                .collect()
        })
        .unwrap_or_default();
    if flights.is_empty() && banks.is_empty() {
        return;
    }
    let source = context.entity;
    world.commands().queue(move |world: &mut World| {
        // Old physical records win over copies inside replacement equipment.
        if let Some(mut replacement) = world.get_mut::<TorpedoSystemResource>(source) {
            replacement
                .0
                .in_flight
                .retain(|round| !flights.iter().any(|old| old.uuid == round.uuid));
        }
        if let Some(mut replacement) = world.get_mut::<BlasterSystemResource>(source) {
            for bank in &mut replacement.0 {
                bank.in_flight.retain(|round| {
                    !banks
                        .iter()
                        .any(|old| old.in_flight.iter().any(|previous| previous.id == round.id))
                });
            }
        }
        // Multiple replacements can precede the deferred flush. A later hook
        // may therefore see copies reintroduced by an earlier replacement.
        // Already-committed actual carriers retain the earliest physical record;
        // these ID sets are derived scratch, never another ownership registry.
        let (guided_ids, bolt_ids) = {
            let mut query = world.query_filtered::<(
                Option<&TorpedoSystemResource>,
                Option<&BlasterSystemResource>,
            ), With<WeaponFlightCarrier>>();
            let mut guided_ids = std::collections::BTreeSet::new();
            let mut bolt_ids = std::collections::BTreeSet::new();
            for (guided, bolts) in query.iter(world) {
                if let Some(owner) = guided {
                    guided_ids.extend(owner.0.in_flight.iter().map(|round| round.uuid.clone()));
                }
                if let Some(owner) = bolts {
                    bolt_ids.extend(
                        owner
                            .0
                            .iter()
                            .flat_map(|bank| bank.in_flight.iter().map(|round| round.id.clone())),
                    );
                }
            }
            (guided_ids, bolt_ids)
        };
        let mut flights = flights;
        flights.retain(|round| !guided_ids.contains(&round.uuid));
        let mut banks = banks;
        for bank in &mut banks {
            bank.in_flight.retain(|round| !bolt_ids.contains(&round.id));
        }
        banks.retain(|bank| !bank.in_flight.is_empty());
        if flights.is_empty() && banks.is_empty() {
            return;
        }
        let entity = spawn(world, flights);
        if !banks.is_empty() {
            world
                .entity_mut(entity)
                .insert(BlasterSystemResource(banks));
        }
    });
}

pub(crate) fn flight_bank(id: String) -> BlasterSystem {
    BlasterSystem::new(BlasterBankConfig {
        id,
        volley_count: 0,
        rounds: None,
        ..Default::default()
    })
}

pub(crate) fn seed_banks(world: &mut World, entity: Entity, ids: &[String]) {
    if !ids.is_empty() {
        world.entity_mut(entity).insert(BlasterSystemResource(
            ids.iter().cloned().map(flight_bank).collect(),
        ));
    }
}

/// Finish the actual Physics/Damage interval before any continuation is taken.
/// Flight-only banks retain no inactive machinery that a later tick must run.
pub(crate) fn settle_carriers(
    mut carriers: Query<
        (
            Entity,
            Option<&TorpedoSystemResource>,
            Option<&mut BlasterSystemResource>,
        ),
        With<WeaponFlightCarrier>,
    >,
    mut commands: Commands,
) {
    for (entity, guided, mut bolts) in &mut carriers {
        if let Some(banks) = &mut bolts {
            banks.0.retain(|bank| !bank.in_flight.is_empty());
        }
        if guided.is_none_or(|owner| owner.0.in_flight.is_empty())
            && bolts.is_none_or(|owner| owner.0.is_empty())
        {
            commands.entity(entity).despawn();
        }
    }
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
            WeaponFlightCarrier,
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
#[path = "flight_carrier_tests.rs"]
mod tests;
