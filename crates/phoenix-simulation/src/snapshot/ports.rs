//! Port grants are validated against the complete candidate physical roster.
use super::*;
use crate::resources::logistics_server::CompiledShipLogistics;
use crate::resources::ports::{PortOrderState, PortServicePolicy};
use crate::resources::ports_server::{CompiledPortService, PortProviderState};
use crate::umbilical::TransferUmbilical;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn assert_settled(world: &World) {
    assert!(world.get_resource::<crate::effect_queue::EffectQueue<crate::resources::ports_server::PortTransfer>>().is_none_or(|queue| queue.0.is_empty()), "port transfers must settle before snapshot capture or deterministic fold");
}

/// Immutable definitions and component presence only. All mutable frontier,
/// dock and stock readings below come from candidate rows, never bootstrap state.
pub(super) fn seed_shape(source: &World, entity: Entity, target: &mut EntityWorldMut<'_>) {
    if let Some(value) = source.get::<CompiledPortService>(entity) {
        target.insert(value.clone());
    }
    if source.get::<PortProviderState>(entity).is_some() {
        target.insert(PortProviderState::default());
    }
    if let Some(value) = source.get::<TransferUmbilical>(entity) {
        target.insert(TransferUmbilical::new(
            value.config.clone(),
            value.power_group.clone(),
        ));
    }
    if let Some(value) = source.get::<crate::dock::DockControl>(entity) {
        target.insert(value.clone());
    }
    if let Some(value) = source.get::<CompiledShipLogistics>(entity) {
        target.insert(value.clone());
    }
}

pub(super) fn validate_candidate(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let entities: BTreeMap<_, _> = world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
        .map(|mut query| {
            query
                .iter(world)
                .map(|(entity, uuid)| (uuid.0.clone(), entity))
                .collect()
        })
        .unwrap_or_default();
    let rows: BTreeMap<_, _> = snapshot
        .entities
        .iter()
        .map(|row| (row.uuid.as_str(), row))
        .collect();
    let mut partners = BTreeMap::new();
    for row in &snapshot.entities {
        let Some(entity) = entities.get(&row.uuid).copied() else {
            continue;
        };
        let Some(dock) = world.get::<crate::dock::DockControl>(entity) else {
            continue;
        };
        if let Some(saved) = &row.dock {
            let mut candidate = dock.clone();
            candidate.restore(saved);
            if let Some(partner) = candidate.docked_partner() {
                partners.insert(row.uuid.as_str(), partner.to_owned());
            }
        }
    }
    for row in &snapshot.entities {
        let Some(entity) = entities.get(&row.uuid).copied() else {
            continue;
        };
        let validate = || -> Result<(), String> {
            match (world.get::<CompiledPortService>(entity), world.get::<PortProviderState>(entity), &row.port_provider) {
                (Some(profile), Some(_), Some(state)) => {
                    state.validate(&profile.0, snapshot.tick)?;
                    let expected: BTreeSet<_> = partners.iter().filter(|(_, partner)| partner.as_str() == row.uuid).map(|(visitor, _)| (*visitor).to_owned()).collect();
                    let actual: BTreeSet<_> = state.visitors.keys().cloned().collect();
                    if actual != expected { return Err("port provider visit membership disagrees with saved physical docking".into()); }
                },
                (None, None, None) => {},
                _ => return Err("saved port provider is missing its authored definition or required quota owner".into()),
            }
            let Some(tool) = world.get::<TransferUmbilical>(entity) else {
                if row.umbilical.is_some() {
                    return Err("saved port tool has no fitted Umbilical owner".into());
                }
                return Ok(());
            };
            let saved = row
                .umbilical
                .as_ref()
                .ok_or("fitted Umbilical is missing its required port and transfer continuation")?;
            if let Some(store) = &saved.selected_store {
                destination(world, entity, store, &saved.selected_resource)?;
            }
            // Defaults do not need a provider; maintenance/store are standing
            // local intent and deliberately survive an ended physical visit.
            let manual = saved.port_order != PortOrderState::default();
            if !manual && saved.port_flow.is_none() {
                return Ok(());
            }
            let partner = partners
                .get(row.uuid.as_str())
                .ok_or("saved port work has no physical docked partner")?;
            let provider_row = rows
                .get(partner.as_str())
                .ok_or("saved port partner is absent from the candidate roster")?;
            let provider_entity = entities
                .get(partner)
                .copied()
                .ok_or("saved port partner could not be materialized")?;
            let profile = world
                .get::<CompiledPortService>(provider_entity)
                .ok_or("saved port partner has no authored service terms")?;
            let ledger = provider_row
                .port_provider
                .as_ref()
                .ok_or("saved port partner has no quota ledger")?;
            let visit = ledger
                .visitors
                .get(&row.uuid)
                .ok_or("saved port work lacks this visitor's physical grant")?;
            if manual {
                if profile.0.policy != PortServicePolicy::ManualOffer {
                    return Err("manual port order belongs to a different provider policy".into());
                }
                validate_order(
                    world,
                    entity,
                    &saved.port_order,
                    &profile.0,
                    partner,
                    visit.visit,
                    snapshot.tick,
                    true,
                )?;
                for line in &saved.port_order.lines {
                    if line.delivered_lots
                        > *visit
                            .delivered
                            .get(&line.offer)
                            .ok_or("port order references an absent visit offer")?
                    {
                        return Err(
                            "saved manual order progress exceeds provider's actual settled quota"
                                .into(),
                        );
                    }
                }
            }
            if let Some(flow) = &saved.port_flow {
                if profile.0.policy != PortServicePolicy::FreeAlliedReplenishment {
                    return Err("free port work belongs to a different provider policy".into());
                }
                validate_order(
                    world,
                    entity,
                    flow,
                    &profile.0,
                    partner,
                    visit.visit,
                    snapshot.tick,
                    false,
                )?;
                if saved.port_maintenance
                    || saved.selected_direction == crate::umbilical::UmbilicalDirection::Collect
                {
                    for line in &flow.lines {
                        if line.delivered_lots
                            > *visit
                                .delivered
                                .get(&line.offer)
                                .ok_or("free port meter references an absent visit offer")?
                        {
                            return Err("saved free receiving progress exceeds provider's actual settled quota".into());
                        }
                    }
                }
                if flow.lines.len() != 1 {
                    return Err("free port meter must have one actual transfer line".into());
                }
                if !saved.port_maintenance {
                    let line = &flow.lines[0];
                    let expected_store = saved.selected_store.as_deref().unwrap_or("named-service");
                    if line.store != expected_store
                        || profile
                            .0
                            .offer(&line.offer)
                            .is_none_or(|offer| offer.resource != saved.selected_resource)
                    {
                        return Err("saved free port meter disagrees with selected transfer resource or endpoint".into());
                    }
                }
            }
            // Authored resource selection is checked by the ordinary owner.
            // No incoming stock or receipt is synthesised during preflight.
            if !tool.config.permits_resource(&saved.selected_resource) {
                return Err("saved port tool selects an unsupported resource".into());
            }
            Ok(())
        };
        validate().map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

fn destination(world: &World, entity: Entity, id: &str, resource: &str) -> Result<(), String> {
    let profile = world
        .get::<CompiledShipLogistics>(entity)
        .ok_or("saved port destination has no managed stock owner")?;
    let store = profile.0.store(id)?;
    if store.role != phoenix_model::logistics::StoreRole::Main || store.resource != resource {
        return Err("saved port destination is not an actual compatible Main store".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_order(
    world: &World,
    entity: Entity,
    order: &PortOrderState,
    profile: &crate::resources::ports::PortServiceConfig,
    partner: &str,
    visit: u64,
    tick: u64,
    manual: bool,
) -> Result<(), String> {
    order.validate(profile, tick)?;
    if order.partner.as_deref() != Some(partner) || order.visit != Some(visit) {
        return Err("saved port work belongs to an ended or different physical visit".into());
    }
    for line in &order.lines {
        let offer = profile
            .offer(&line.offer)
            .ok_or("saved port work references unknown offer terms")?;
        if manual {
            let local = world
                .get::<CompiledShipLogistics>(entity)
                .ok_or("saved barter has no actual Main cost owner")?;
            for cost in &offer.costs {
                if !local.0.config().stores.iter().any(|store| {
                    store.role == phoenix_model::logistics::StoreRole::Main
                        && store.resource == cost.resource
                }) {
                    return Err(
                        "saved barter cost has no actual compatible Main source store".into(),
                    );
                }
            }
        }
        // Free legacy service transfer has no explicit Main-store selection;
        // its meter names the existing named endpoint with a private sentinel.
        if manual || line.store != "named-service" {
            destination(world, entity, &line.store, &offer.resource)?;
        }
    }
    Ok(())
}
