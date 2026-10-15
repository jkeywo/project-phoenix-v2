//! Validate installed and portable component expiry against the complete saved
//! roster/header before any live entity, global, or assembly write.
use super::*;

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let installed = row.physical_damage.as_ref().is_some_and(|state| {
            state
                .components
                .values()
                .any(|components| components.values().any(|component| component.emp.is_some()))
        });
        let portable = row.fitting.as_ref().is_some_and(|state| {
            state.equipment.assemblies.values().any(|item| {
                item.condition
                    .as_ref()
                    .is_some_and(|condition| condition.emp.is_some())
            })
        });
        if !installed && !portable {
            continue;
        }
        let result = (|| {
            if !world.contains_resource::<crate::sim_tick::SimTick>() {
                return Err("EMP continuation requires its logical tick".into());
            }
            let clock = crate::ship::emp::clock(world, snapshot.tick)?;
            let live = world
                .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
                .and_then(|mut query| {
                    query
                        .iter(world)
                        .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
                });
            let definition = if let Some(entity) = live {
                world
                    .get::<crate::ship::compartment_damage::ShipDamage>(entity)
                    .map(|damage| damage.definition.clone())
                    .ok_or("EMP has no actual physical component owner")?
            } else {
                let config = row
                    .spawn
                    .as_ref()
                    .and_then(|origin| {
                        origin.resolve(
                            &crate::entities::loader::WasmTemplateLoader,
                            &mut Vec::new(),
                        )
                    })
                    .ok_or("cannot resolve saved EMP component owner")?;
                crate::ship::compartment_damage::ShipDamage::from_config(&config)?
                    .ok_or("saved EMP has no physical receiver")?
                    .definition
            };
            if let Some(state) = &row.physical_damage {
                state.validate_emp_at(&definition, clock)?;
            }
            if let Some(state) = &row.fitting {
                for item in state.equipment.assemblies.values() {
                    if let Some(emp) = item
                        .condition
                        .as_ref()
                        .and_then(|condition| condition.emp.as_ref())
                    {
                        emp.validate(clock)?;
                    }
                }
            }
            Ok::<_, String>(())
        })();
        result.map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "emp_tests.rs"]
mod tests;
