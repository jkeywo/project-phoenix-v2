//! Immutable supported fitting compilation and one actual portable-equipment owner.
use crate::core::messages::SystemId;
use crate::entities::config::EntityConfig;
use phoenix_model::damage::ComponentState;
pub use phoenix_model::fitting::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[path = "fitting_compile.rs"]
mod fitting_compile;
#[path = "fitting_state.rs"]
mod fitting_state;

#[derive(Clone, Debug)]
pub struct CompiledFitting {
    config: FittingConfig,
    base: std::sync::Arc<EntityConfig>,
    compartments: BTreeMap<String, String>,
}

/// Installed equipment carries no parallel physical condition. Desired choices
/// are requests only; real assemblies and physical work still determine supply.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FittingState {
    pub assemblies: BTreeMap<String, EquipmentAssembly>,
    pub desired: BTreeMap<String, String>,
}

impl CompiledFitting {
    pub fn config(&self) -> &FittingConfig {
        &self.config
    }
    pub fn slot(&self, id: &str) -> Result<&FittingSlotConfig, String> {
        self.config
            .slots
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("unknown fitting slot {id}"))
    }
    pub fn equipment(&self, id: &str) -> Result<&EquipmentDefinition, String> {
        self.config
            .equipment
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("unknown equipment definition {id}"))
    }
    pub fn locker(&self, id: &str) -> Result<&EquipmentLockerConfig, String> {
        self.config
            .lockers
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("unknown equipment locker {id}"))
    }
    pub fn compartment(&self, slot: &str) -> Result<&str, String> {
        self.compartments
            .get(slot)
            .map(String::as_str)
            .ok_or_else(|| format!("unknown fitting slot {slot}"))
    }
    pub fn compatible(&self, slot: &str, definition: &str) -> Result<(), String> {
        let slot = self.slot(slot)?;
        let equipment = self.equipment(definition)?;
        if equipment.hardware.family() != slot.family
            || equipment.interface != slot.interface
            || equipment.size > slot.size
        {
            return Err(format!(
                "equipment {} is incompatible with fitting slot {}",
                equipment.id, slot.id
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "fitting_tests.rs"]
mod tests;
