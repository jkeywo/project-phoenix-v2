//! Derived defeat transitions, separate from physical damage and destruction.
use super::components::ShipConfigComponent;
use super::{physical_damage::ShipDamage, power::ShipPowerSystem};
use crate::ai::server::AiEntityDisabled;
use crate::entities::spawner::EntityUuid;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Persist this frontier with its equipped owner. Restoring a disabled actor
/// must not publish a second entry edge. This is no machinery/controller state.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisablementObservation {
    pub disabled: bool,
}

pub fn publish_ship_disablement(
    mut ships: Query<(
        &EntityUuid,
        &ShipConfigComponent,
        &ShipDamage,
        Option<&ShipPowerSystem>,
        &mut DisablementObservation,
    )>,
    mut events: MessageWriter<AiEntityDisabled>,
) {
    // Event order is visible to script handlers; never use ECS archetype order.
    let mut ordered: Vec<_> = ships.iter_mut().collect();
    ordered.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    for (uuid, config, damage, power, mut previous) in ordered {
        let disabled = damage.neutralized(&config.0, power);
        if previous.disabled != disabled {
            previous.disabled = disabled;
            events.write(AiEntityDisabled {
                entity_uuid: uuid.0.clone(),
                disabled,
            });
        }
    }
}
