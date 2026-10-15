//! Finite capability allocation over explicit own System priorities. No live
//! exposure, resource reserve, controller or saved assignment frontier lives here.
use crate::{
    core::messages::SystemId,
    ship::composition::{validate_system_priorities, MAX_SYSTEM_PRIORITIES},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalProtectionConfig {
    pub capacity: u32,
    pub strength: f32,
    /// Actual compatible receiver System -> whole allocation cost.
    pub systems: BTreeMap<SystemId, u32>,
}
impl SignalProtectionConfig {
    /// Actual equipment-kind/owner eligibility is checked by the accepted graph
    /// compiler. This pure leaf checks finite capability and intent shape only.
    pub fn validate(&self) -> Result<(), String> {
        if self.capacity == 0
            || !self.strength.is_finite()
            || self.strength <= 0.0
            || self.systems.is_empty()
            || self.systems.len() > MAX_SYSTEM_PRIORITIES
            || self
                .systems
                .iter()
                .any(|(id, cost)| id.0.trim().is_empty() || *cost == 0 || *cost > self.capacity)
        {
            return Err("signal protection requires finite positive strength/capacity and bounded eligible System costs".into());
        }
        Ok(())
    }
    pub fn validate_priorities(&self, priorities: &[SystemId]) -> Result<(), String> {
        self.validate()?;
        validate_system_priorities(priorities)?;
        if priorities.iter().any(|id| !self.systems.contains_key(id)) {
            return Err("protection priority names an ineligible actual System".into());
        }
        Ok(())
    }
    /// Whole costs in authored/user order; no hidden exposure participates.
    /// A cost which does not fit never blocks a later smaller eligible cost.
    pub fn allocate(&self, priorities: &[SystemId]) -> Result<Vec<SystemId>, String> {
        self.validate_priorities(priorities)?;
        let mut remaining = self.capacity;
        Ok(priorities
            .iter()
            .filter_map(|id| {
                let cost = self.systems[id];
                if cost > remaining {
                    return None;
                }
                remaining -= cost;
                Some(id.clone())
            })
            .collect())
    }
    pub fn paid_suppression(&self, fraction: f32) -> Result<f64, String> {
        self.validate()?;
        if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
            return Err("protection work fraction must be finite and within 0..=1".into());
        }
        Ok(f64::from(self.strength) * f64::from(fraction))
    }
}
/// Equipment suppression cannot weaken natural denial, terrain or physical
/// damage. Caller sums only current paid grants for the allocated actual owner.
pub fn receiver_factor(exposure: f64, suppression: f64) -> Result<f32, String> {
    if !exposure.is_finite() || exposure < 0.0 || !suppression.is_finite() || suppression < 0.0 {
        return Err("equipment exposure/suppression must be finite nonnegative".into());
    }
    Ok((1.0 / (1.0 + (exposure - suppression).max(0.0))) as f32)
}

/// Only the actual owning System's own local intent is offered to its ordinary
/// controller. Priorities remain authored/user intent, never a hidden selector.
pub fn bindings(enabled: &str) -> crate::ai::state_tree::SupportedBindings {
    crate::ai::state_tree::SupportedBindings {
        facts: ["enabled".into(), "has_priorities".into()].into(),
        channels: [enabled.into()].into(),
    }
}
/// Accepted capability eligibility comes from actual fitted receiver equipment,
/// never a family name, current interference or a second runtime receiver map.
pub fn validate_equipment(
    entity: &crate::entities::config::EntityConfig,
    system: &crate::ship::config::SystemInstanceConfig,
) -> Result<(), String> {
    crate::ship::composition::validate_composition(system)?;
    let binding = crate::ship::composition::protection_composition(&system.components)
        .ok_or("missing actual signal protection component")?;
    let ship = entity
        .ship_config
        .as_ref()
        .ok_or("protection requires an actual owning ship graph")?;
    for receiver in binding.config.systems.keys() {
        if ship.system(receiver).is_none() {
            return Err("protection names an unknown own System".into());
        }
        let radio = entity
            .comms
            .as_ref()
            .and_then(|comms| comms.radio.as_ref())
            .is_some_and(|radio| &radio.system == receiver);
        let sensors = entity.sensing.as_ref().is_some_and(|sensing| {
            sensing
                .broad
                .iter()
                .any(|profile| &profile.system == receiver)
                || sensing
                    .focused
                    .iter()
                    .any(|profile| &profile.system == receiver)
                || sensing
                    .passive
                    .iter()
                    .any(|profile| &profile.system == receiver)
        });
        if !radio && !sensors {
            return Err(
                "protection names a System without supported actual receiver equipment".into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "protection_tests.rs"]
mod tests;
