//! Bind recovery declarations to actual installed machinery and finite owners.
use super::{logistics::StoreRole, logistics_profile::compile_entity_logistics, recovery::*};
use crate::{core::messages::SystemBlackboard, entities::config::EntityConfig};

pub fn compile_entity_recovery(
    config: &EntityConfig,
) -> Result<Option<RecoveryToolConfig>, String> {
    let fitted: Vec<_> = config
        .ship_config
        .as_ref()
        .into_iter()
        .flat_map(|ship| &ship.systems)
        .filter(|system| system.kind == RECOVERY_KIND)
        .collect();
    let Some(profile) = &config.recovery else {
        return if fitted.is_empty() {
            Ok(None)
        } else {
            Err("A fitted resource-recovery System requires a complete [recovery] profile".into())
        };
    };
    profile.validate()?;
    if fitted.len() != 1 || fitted[0].id != profile.system {
        return Err("Recovery requires exactly one fitted resource-recovery System matching recovery.system".into());
    }
    let ship = config
        .ship_config
        .as_ref()
        .ok_or("recovery needs installed Systems")?;
    let logistics = compile_entity_logistics(config)?
        .ok_or("recovery needs actual managed destination stocks")?;
    let room = logistics
        .graph()
        .compartment_for_system(&profile.system)
        .ok_or("recovery System needs a physical compartment")?;
    if !logistics.config().stores.iter().any(|store| {
        store.role == StoreRole::Main
            && &store.compartment == room
            && profile.resources.contains(&store.resource)
    }) {
        return Err(
            "Recovery requires a compatible Main destination store in its own compartment".into(),
        );
    }
    if !config
        .energy
        .as_ref()
        .is_some_and(|energy| energy.consumers.iter().any(|c| c.system == profile.system))
    {
        return Err("Recovery requires an actual local-energy consumer".into());
    }
    let inventory = logistics.handling().system.clone();
    let mut boards = std::collections::HashMap::from([
        (
            inventory.clone(),
            SystemBlackboard::Logistics(Default::default()),
        ),
        (
            profile.system.clone(),
            SystemBlackboard::Recovery(Default::default()),
        ),
    ]);
    for (publication, kind) in crate::ship::system_registry::fixed_publication_kinds() {
        if kind == phoenix_model::console::ConsoleObservationKind::SensorRadar
            && ship
                .systems
                .iter()
                .any(|s| s.kind == crate::ship::system_registry::SENSOR_RADAR_KIND)
        {
            boards.insert(
                crate::core::messages::SystemId(publication),
                SystemBlackboard::SensorRadar(Default::default()),
            );
        } else if kind == phoenix_model::console::ConsoleObservationKind::TacticalRadar
            && ship
                .systems
                .iter()
                .any(|s| s.kind == crate::ship::system_registry::TACTICAL_RADAR_KIND)
        {
            boards.insert(
                crate::core::messages::SystemId(publication),
                SystemBlackboard::TacticalRadar(Default::default()),
            );
        }
    }
    let permitted = ship.observations_for_system(&profile.system, &boards);
    if !matches!(
        permitted.get(&inventory),
        Some(SystemBlackboard::Logistics(_))
    ) || !matches!(
        permitted.get(&profile.system),
        Some(SystemBlackboard::Recovery(_))
    ) || !(permitted.permits_kind(phoenix_model::console::ConsoleObservationKind::SensorRadar)
        || permitted.permits_kind(phoenix_model::console::ConsoleObservationKind::TacticalRadar))
    {
        return Err("Recovery needs its own typed Recovery view plus permitted Logistics and SensorRadar or TacticalRadar feeds; co-hosting grants no information".into());
    }
    Ok(Some(profile.clone()))
}

pub fn validate_recoverable_source(config: &EntityConfig) -> Result<(), String> {
    let Some(source) = &config.recoverable else {
        return Ok(());
    };
    source.validate()?;
    if source.availability == RecoveryAvailability::Disabled
        && (config.damage.is_none()
            || (config.behaviour.is_none() && !config.is_static_point_defence()))
    {
        return Err(
            "Disabled recoverable sources require the ordinary physical disablement owner".into(),
        );
    }
    for (resource, endpoint) in &source.resources {
        if let Some(store) = config.logistics.as_ref().and_then(|profile| {
            profile
                .stores
                .iter()
                .find(|store| &store.endpoint == endpoint && &store.resource != resource)
        }) {
            return Err(format!("Recoverable resource {resource:?} conflicts with managed store {:?} containing {:?}",
                    store.id, store.resource));
        }
        match endpoint {
            phoenix_model::logistics::ResourceEndpoint::Capacity { id } => {
                if !config
                    .infrastructure
                    .as_ref()
                    .is_some_and(|inventory| inventory.capacities.iter().any(|c| &c.id == id))
                {
                    return Err(format!(
                        "Recoverable source names absent finite capacity {id:?}"
                    ));
                }
            }
            phoenix_model::logistics::ResourceEndpoint::TorpedoMagazine {}
                if config.torpedoes.is_none() =>
            {
                return Err("Recoverable source names a missing actual torpedo magazine".into())
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "recovery_profile_tests.rs"]
mod tests;
