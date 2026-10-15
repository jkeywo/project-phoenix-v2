//! Source-known orders plus current Console choices. No remote flight query.
use super::{
    guidance_link::{GuidanceContext, ShipGuidanceKnowledge},
    TorpedoSystemResource,
};
use crate::{
    core::messages::SystemId,
    entities::spawner::EntityUuid,
    science::broad::{BroadTracks, SensingControls},
    server_app::ShipSystemBlackboards,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_model::guided_orders::{GuidanceControlsView, GuidanceTargetOption};

pub(crate) fn project(
    world: &World,
    entity: Entity,
    owner: &SystemId,
) -> Option<GuidanceControlsView> {
    let knowledge = world.get::<ShipGuidanceKnowledge>(entity)?;
    let config = world.get::<ShipConfigComponent>(entity)?;
    if config.0.system(owner)?.kind != crate::ship::system_registry::TORPEDO_TUBE_KIND {
        return None;
    }
    let source = world.get::<TorpedoSystemResource>(entity);
    let tube = source.and_then(|source| {
        source.0.tubes.iter().find(|tube| {
            crate::ship::system_registry::torpedo_tube_system_id(&tube.id).as_ref() == Some(owner)
        })
    });
    let ready = tube.is_some()
        && source.is_some_and(|source| source.0.config.receiver.is_some())
        && world
            .get::<ShipSystemControlSources>(entity)
            .is_some_and(|sources| sources.0.policy_for(owner).accept_human_input)
        && world.get::<ShipDamage>(entity).is_none_or(|damage| {
            damage.factor(owner, phoenix_model::damage::DamageBinding::Operation) > 0.0
        })
        && world
            .get::<ShipPowerSystem>(entity)
            .is_some_and(|power| matches!(power.0.clone().try_work_discrete(owner), Ok(true)));
    let rounds = phoenix_sim_gameplay::weapons::guided_orders::project(&knowledge.0, owner, ready);
    let mut result = GuidanceControlsView {
        rounds,
        targets: Vec::new(),
    };
    let Some((observer, boards, tracks, tick)) = world
        .get::<EntityUuid>(entity)
        .zip(world.get::<ShipSystemBlackboards>(entity))
        .zip(world.get::<BroadTracks>(entity))
        .zip(world.get_resource::<crate::sim_tick::SimTick>())
        .map(|(((a, b), c), d)| (a, b, c, d.0))
    else {
        return Some(result);
    };
    let scope = GuidanceContext {
        observer,
        config,
        boards,
        tracks,
        controls: world.get::<SensingControls>(entity),
        tick,
        runtime: world.get_resource::<crate::world::server::WorldContentRuntime>(),
    };
    let permitted = config.0.observations_for_system(owner, &boards.0);
    let mut ids = std::collections::BTreeSet::new();
    for record in boards
        .0
        .keys()
        .filter_map(|key| permitted.get(key))
        .filter_map(crate::science::contact_routing::observations)
        .flatten()
    {
        if !ids.insert(record.contact_id.clone()) {
            continue;
        }
        let Some(reading) = scope.sample(owner, &record.contact_id) else {
            continue;
        };
        let tags: Vec<_> = record
            .classification
            .as_deref()
            .and_then(phoenix_model::tags::EntityTag::from_str)
            .into_iter()
            .collect();
        // Exact authorised route membership is an actuator domain check only.
        // No foreign location, classification, health or survival is projected.
        let ordinary = world
            .try_query::<&EntityUuid>()
            .is_some_and(|mut query| query.iter(world).any(|id| id.0 == reading.target_uuid))
            || world
                .try_query::<&crate::server_app::AsteroidUuid>()
                .is_some_and(|mut query| query.iter(world).any(|id| id.0 == reading.target_uuid));
        if tube.is_some_and(|tube| {
            crate::weapons::operation::target_allowed(tube.targets.as_deref(), &tags, ordinary)
        }) {
            result.targets.push(GuidanceTargetOption {
                contact: record.contact_id.clone(),
                name: record
                    .identity
                    .as_ref()
                    .map(|identity| identity.name.clone()),
            });
        }
    }
    result.targets.sort_by(|a, b| a.contact.cmp(&b.contact));
    Some(result)
}
