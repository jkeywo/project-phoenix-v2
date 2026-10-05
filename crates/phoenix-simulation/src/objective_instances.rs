//! Explicitly shared Objective instances (issue #1523).
//!
//! Legacy Objectives remain in `objectives::ObjectiveManager`; this module is
//! the additive multi-ship contract. Recipients are ship slots, not subject
//! targets, and every mutation addresses `(objective_id, instance_id)`.

pub mod control;

use std::collections::{BTreeMap, BTreeSet};

use crate::core::messages::ObjectiveStatus;
use bevy::prelude::*;

#[derive(Clone, Debug)]
pub struct ObjectiveCommandOrigin {
    pub source: Option<String>,
    pub line: Option<usize>,
    pub tick: u64,
}

impl ObjectiveCommandOrigin {
    fn refused(&self, world: &mut World, message: String) {
        crate::recipients::report(
            world,
            crate::recipients::RecipientDiagnostic {
                tick: self.tick,
                source: self.source.clone(),
                line: self.line,
                action: "objective-instance".into(),
                message,
            },
        );
    }
}

fn activation_refusal_message(reason: &ActivationRefusal) -> String {
    match reason {
        ActivationRefusal::EmptyRecipients => "Choose at least one Objective recipient".into(),
        ActivationRefusal::UnknownShipSlot(slot) => format!("Unknown Objective ship slot '{slot}'"),
        ActivationRefusal::UnknownFaction(faction) => format!("Unknown Objective faction '{faction}'"),
        ActivationRefusal::StaticShipSlotConflict { objective_id, slot_id, instance_ids } =>
            format!("Objective '{objective_id}' gives ship slot '{slot_id}' equal-specificity assignments in instances {}. Change one recipient selector", instance_ids.join(", ")),
        ActivationRefusal::Conflict(conflict) =>
            format!("Objective '{}' gives ship '{}' equal-specificity assignments in instances {}. Change one recipient selector", conflict.objective_id, conflict.ship_id, conflict.instance_ids.join(", ")),
    }
}

/// Deterministic live fleet projection used by every Objective-instance
/// lifecycle mutation. Authored slot identity is never reconstructed from
/// transport `HostSlot` ordering.
pub fn player_ship_memberships(world: &mut World) -> Vec<PlayerShipMembership> {
    crate::recipients::prepare_in_world(world).fleet
}

pub fn reconcile_memberships(world: &mut World) {
    let fleet = player_ship_memberships(world);
    let refused = {
        let mut manager = world.resource_mut::<crate::world::server::ObjectiveInstanceManagerRes>();
        manager
            .0
            .reconcile(&fleet)
            .err()
            .map(|conflict| (conflict, manager.0.accepted_fleet().to_vec()))
    };
    if let Some((conflict, accepted)) = refused {
        // The manager refused the candidate without changing its history. A
        // live faction edit must likewise leave the ship on its old faction;
        // otherwise combat and Objectives would disagree about membership.
        // Use an exclusive system so the rollback lands before any later
        // fixed-step reader, rather than through deferred Commands.
        let restore: BTreeMap<_, _> = accepted
            .iter()
            .map(|member| {
                let faction = world
                    .get_resource::<crate::entities::config_cache::FactionRegistryResource>()
                    .and_then(|registry| registry.uuid_by_name(&member.faction));
                (member.ship_id.as_str(), faction)
            })
            .collect();
        let mut query = world.query_filtered::<(Entity, &crate::entities::spawner::EntityUuid), With<crate::server_app::Ship>>();
        let entities: Vec<_> = query
            .iter(world)
            .filter_map(|(entity, uuid)| {
                restore
                    .get(uuid.0.as_str())
                    .map(|faction| (entity, *faction))
            })
            .collect();
        for (entity, faction) in entities {
            if let Some(faction) = faction {
                world
                    .entity_mut(entity)
                    .insert(crate::entities::spawner::FactionComponent(faction));
            } else {
                world
                    .entity_mut(entity)
                    .remove::<crate::entities::spawner::FactionComponent>();
            }
        }
        let message = activation_refusal_message(&ActivationRefusal::Conflict(conflict.clone()));
        ObjectiveCommandOrigin {
            source: None,
            line: None,
            tick: world
                .get_resource::<crate::sim_tick::SimTick>()
                .map_or(0, |tick| tick.0),
        }
        .refused(world, message);
        bevy::log::warn!(
            "Objective instance reconciliation refused: objective '{}' is ambiguous for ship '{}' across {:?}",
            conflict.objective_id,
            conflict.ship_id,
            conflict.instance_ids
        );
    }
}

/// Apply the instance-only objective commands queued by the shared world
/// dispatcher. Legacy commands never enter this function and therefore retain
/// their existing reducer byte-for-byte.
pub fn apply_command(
    world: &mut World,
    command: crate::world::dispatch::ActionCmd,
    origin: &ObjectiveCommandOrigin,
) {
    let prepared = crate::recipients::prepare_in_world(world);
    let applied = world.resource_scope(
        |world, mut instances: Mut<crate::world::server::ObjectiveInstanceManagerRes>| {
            let mut objectives = world.resource_mut::<crate::world::server::ObjectiveManagerRes>();
            control::apply_instance_command(
                command,
                &mut instances.0,
                &mut objectives.0,
                &prepared.fleet,
                &prepared.catalog.ship_slots,
                &prepared.catalog.factions,
            )
        },
    );
    match applied {
        Err(reason) => {
            bevy::log::warn!("Objective instance mutation refused: {}", reason.message());
            origin.refused(world, reason.message());
        }
        Ok(applied) => {
            let targets = applied
                .transition
                .as_ref()
                .and_then(|(id, _)| {
                    world
                        .resource::<crate::world::server::ObjectiveManagerRes>()
                        .0
                        .targets(id)
                })
                .unwrap_or_default()
                .to_vec();
            control::publish_instance_apply(
                &applied,
                &targets,
                None,
                world
                    .get_resource_mut::<crate::world::server::WorldLayerMap>()
                    .as_deref_mut(),
            );
            control::publish_instance_apply(
                &applied,
                &targets,
                world
                    .get_resource_mut::<Messages<crate::core::balance::BalanceEvent>>()
                    .as_deref_mut(),
                None,
            );
        }
    }
}

#[cfg(test)]
#[path = "objective_instances_tests.rs"]
mod tests;

pub use phoenix_sim_world::objective_instances::*;
