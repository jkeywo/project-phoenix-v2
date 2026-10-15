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

#[cfg(test)]
#[path = "protection_tests.rs"]
mod tests;
