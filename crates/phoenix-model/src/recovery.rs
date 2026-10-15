//! Closed recovery tools and bindings to existing finite source stocks.
use crate::{logistics::ResourceEndpoint, messages::SystemId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryToolConfig {
    pub system: SystemId,
    pub range: f32,
    pub units_per_sec: f32,
    pub resources: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAvailability {
    Always,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverableSourceConfig {
    pub availability: RecoveryAvailability,
    pub resources: BTreeMap<String, ResourceEndpoint>,
}

impl RecoveryToolConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.system.0.trim().is_empty()
            || !self.range.is_finite()
            || self.range <= 0.0
            || !self.units_per_sec.is_finite()
            || self.units_per_sec <= 0.0
            || self.resources.is_empty()
            || self.resources.iter().any(|r| r.trim().is_empty())
            || self.resources.iter().collect::<BTreeSet<_>>().len() != self.resources.len()
        {
            return Err(
                "recovery tool requires a System, positive finite range/rate and unique resources"
                    .into(),
            );
        }
        Ok(())
    }
}

impl RecoverableSourceConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.resources.is_empty() {
            return Err("recoverable source requires actual resource endpoint bindings".into());
        }
        let mut endpoints = BTreeSet::new();
        for (resource, endpoint) in &self.resources {
            endpoint.validate_resource(resource)?;
            if !endpoints.insert(endpoint) {
                return Err("recoverable resources cannot alias one actual endpoint".into());
            }
        }
        Ok(())
    }
}
