//! Immutable support for actual ordinary requests; no job or resource state.
use crate::{
    resources::logistics::CompiledLogistics,
    ship::{compartment_damage::DamageDefinition, config::ShipConfig},
};
use phoenix_model::{
    activity::RepairGoal,
    damage::{ComponentRef, RepairWorkTarget},
    messages::{RepairGroup, RepairTarget, SystemId},
};

pub fn resolve_repair_visit(
    target: &RepairTarget,
    config: &ShipConfig,
    definition: &DamageDefinition,
) -> Option<SystemId> {
    let group = match target {
        RepairTarget::Work(RepairWorkTarget::Component { system, component }) => {
            definition.component(&ComponentRef {
                system: system.clone(),
                component: component.clone(),
            })?;
            return Some(system.clone());
        }
        RepairTarget::Work(RepairWorkTarget::Compartment { compartment }) => {
            definition.compartment(compartment)?;
            return definition
                .interior
                .placements
                .iter()
                .find(|(_, room)| *room == compartment)
                .map(|(system, _)| system.clone())
                .or_else(|| definition.components.keys().next().cloned());
        }
        RepairTarget::System(system) => {
            return definition
                .components
                .contains_key(system)
                .then(|| system.clone())
        }
        RepairTarget::Station(id) => RepairGroup::Station(id.clone()),
        RepairTarget::Console(id) => RepairGroup::Console(id.clone()),
        RepairTarget::Core => RepairGroup::Core,
        RepairTarget::External => return None,
    };
    definition
        .components
        .keys()
        .find(|system| crate::modifiers::repair_teams::repair_group(system, config) == group)
        .cloned()
}

/// Borrowed authored arguments, never a second persisted request or actor.
pub struct RepairBinding<'a> {
    pub owner: &'a SystemId,
    pub team: u32,
    pub request: &'a RepairTarget,
    pub target: &'a RepairWorkTarget,
    pub goal: RepairGoal,
}
pub fn repair_supported(
    binding: RepairBinding<'_>,
    config: &ShipConfig,
    definition: &DamageDefinition,
    team_owner: &SystemId,
    count: usize,
) -> bool {
    if binding.owner != team_owner
        || u8::try_from(binding.team).is_err()
        || binding.team as usize >= count
        || config
            .system(binding.owner)
            .is_none_or(|system| system.kind != crate::ship::system_registry::REPAIR_KIND)
    {
        return false;
    }
    let supported = match (binding.target, binding.goal) {
        (RepairWorkTarget::Compartment { compartment }, RepairGoal::Structure) => {
            definition.compartment(compartment).is_some()
        }
        (RepairWorkTarget::Component { system, component }, goal) => definition
            .component(&ComponentRef {
                system: system.clone(),
                component: component.clone(),
            })
            .is_some_and(|part| match goal {
                RepairGoal::Full => definition.config.remedies.contains_key(&part.full_remedy),
                RepairGoal::Patch => part
                    .patch_remedy
                    .as_ref()
                    .is_some_and(|name| definition.config.remedies.contains_key(name)),
                RepairGoal::Structure => false,
            }),
        _ => false,
    };
    if !supported {
        return false;
    }
    let Some(selected) = resolve_repair_visit(binding.request, config, definition) else {
        return false;
    };
    if let RepairTarget::Work(exact) = binding.request {
        return exact == binding.target;
    }
    let room = match binding.target {
        RepairWorkTarget::Compartment { compartment } => definition
            .compartment(compartment)
            .map(|row| row.id.as_str()),
        RepairWorkTarget::Component { system, component } => {
            definition.component_room(&ComponentRef {
                system: system.clone(),
                component: component.clone(),
            })
        }
    };
    room.is_some_and(|room| {
        definition
            .interior
            .placements
            .get(&selected)
            .is_some_and(|selected| selected == room)
    })
}

pub fn delivery_supported(
    profile: &CompiledLogistics,
    owner: &SystemId,
    source: &str,
    destination: &str,
    commodity: &str,
    amount: i64,
) -> bool {
    if owner != &profile.handling().system || amount <= 0 || amount > profile.handling().max_batch {
        return false;
    }
    profile
        .store(source)
        .ok()
        .zip(profile.store(destination).ok())
        .is_some_and(|(from, to)| {
            from.role == phoenix_model::logistics::StoreRole::Main
                && to.role == phoenix_model::logistics::StoreRole::Local
                && to.source.as_deref() == Some(source)
                && from.resource == commodity
                && to.resource == commodity
        })
}

#[cfg(test)]
#[path = "activity_bindings_tests.rs"]
mod tests;

/// Immutable equipment support, shared by source preparation and live arming.
/// Current contact knowledge, radio paths and energy are ordinary runtime gates.
pub fn communication_supported(
    owner: &SystemId,
    channel: &str,
    config: &ShipConfig,
    radio: &phoenix_model::comms_channel::CommsRadioConfig,
) -> bool {
    &radio.system == owner
        && radio.channels.iter().any(|row| row.id == channel)
        && config
            .system(owner)
            .is_some_and(|system| system.kind == crate::ship::system_registry::COMMS_KIND)
}

/// Immutable actual external Security equipment; observations remain a live gate.
pub fn acquisition_supported(
    owner: &SystemId,
    team: u32,
    ship: &ShipConfig,
    security: &crate::security::SecurityConfig,
) -> bool {
    owner == &crate::ship::system_registry::security_system_id()
        && team < u32::from(security.team_count)
        && security.local.is_none()
        && ship
            .system(owner)
            .is_some_and(|system| system.kind == crate::ship::system_registry::SECURITY_KIND)
}
