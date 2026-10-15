//! Remove the existing public roster entry at the actual lifecycle boundary.
//! Visibility is sampled before GM knowledge cleanup; no recipient cache is kept.
use super::{LocalShip, SimOutbox, TrackedEntities};
use crate::core::messages::ServerMessage;
use crate::entities::spawner::EntityUuid;
use crate::lobby::{Target, WorldResource};
use crate::world::server::WorldContentRuntime;
use bevy::ecs::system::{SystemParam, SystemState};
use bevy::prelude::*;

#[derive(SystemParam)]
pub(crate) struct EntityRemoval<'w, 's> {
    world: Option<ResMut<'w, WorldResource>>,
    tracked: Option<ResMut<'w, TrackedEntities>>,
    observer: Query<'w, 's, &'static EntityUuid, With<LocalShip>>,
    content: Option<Res<'w, WorldContentRuntime>>,
}

impl EntityRemoval<'_, '_> {
    /// Existing kill sites retain their own damage, VFX and terminal events.
    /// This joins only their roster removal and reconcile suppression.
    pub(crate) fn remove(&mut self, uuid: &str, outbox: Option<&mut SimOutbox>) {
        let visible_chart = self.world.as_ref().is_some_and(|world| {
            visible_chart_record(
                world,
                uuid,
                self.observer
                    .single()
                    .ok()
                    .map(|observer| observer.0.as_str()),
                self.content.as_deref(),
            )
        });
        if let Some(world) = self.world.as_mut() {
            world.0.entities.retain(|entity| entity.uuid != uuid);
            if visible_chart {
                if let Some(outbox) = outbox {
                    outbox.push_reliable((
                        Target::All,
                        ServerMessage::WorldSetup {
                            world: world.0.clone(),
                        },
                    ));
                }
            }
        }
        if let Some(tracked) = self.tracked.as_mut() {
            tracked.forget(uuid);
        }
    }
}

pub(crate) fn visible_chart_record(
    world: &WorldResource,
    uuid: &str,
    observer: Option<&str>,
    content: Option<&WorldContentRuntime>,
) -> bool {
    world.0.entities.iter().any(|entity| {
        entity.uuid == uuid
            && entity
                .tags
                .iter()
                .any(|tag| tag == crate::console_delivery::CHARTED_TAG)
    }) && !observer.is_some_and(|observer| {
        content.is_some_and(|content| {
            crate::gm::operations::contact::mode(&content.contact_overrides, observer, uuid)
                == crate::gm::operations::contact::ContactMode::Conceal
        })
    })
}

/// Ordinary scripted and GM removal share this exclusive-World entry point.
/// Run before clearing observations about the actor being removed.
pub(crate) fn remove_scripted_entity_record(world: &mut World, uuid: &str) {
    let mut state = SystemState::<(EntityRemoval, Option<ResMut<SimOutbox>>)>::new(world);
    {
        let (mut removal, mut outbox) = state.get_mut(world);
        // Match reconciliation's existing terminal publication: only a previously
        // reported entity has a client-side lifecycle to close.
        if removal
            .tracked
            .as_ref()
            .is_some_and(|tracked| tracked.reported.contains(uuid))
        {
            if let Some(outbox) = outbox.as_mut() {
                outbox.push_reliable((
                    Target::All,
                    ServerMessage::EntityDespawned { uuid: uuid.into() },
                ));
            }
        }
        removal.remove(uuid, outbox.as_deref_mut());
    }
    // This path used to leave removal to reconciliation, which also pruned its
    // two delta caches. Preserve that behavior when forgetting eagerly now.
    if let Some(mut positions) = world.get_resource_mut::<super::LastBroadcastEntityPositions>() {
        positions.0.remove(uuid);
    }
    if let Some(mut health) = world.get_resource_mut::<super::LastBroadcastEntityHealth>() {
        health.0.remove(uuid);
    }
}

#[cfg(test)]
#[path = "entity_removal_tests.rs"]
mod tests;
