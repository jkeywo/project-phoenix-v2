//! Common metadata for permitted recorded facts; no truth lookup or extra store.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservationSubject {
    #[default]
    Unknown,
    Entity { uuid: String },
    System { id: SystemId },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    #[default]
    Unknown,
    Scan,
    Evidence,
    SensorsReport,
    SystemReport,
    RepairTeam,
    RepairDiagnosis,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationProvenance {
    pub kind: ObservationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemId>,
    /// Existing public String Id, never a GM policy, identity or hidden cause.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// Precision/evidence limits, not invented probability or verified truth.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservationUncertainty {
    #[default]
    Unspecified,
    Qualitative,
    Quantized { field: String, step: f32 },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ObservationMetadata {
    pub subject: ObservationSubject,
    pub provenance: ObservationProvenance,
    /// None means unknown sample time, never a fresh observation at tick zero.
    pub observed_tick: Option<u64>,
    /// Receipt does not renew the sample time when a report is forwarded.
    pub received_tick: Option<u64>,
    pub uncertainty: ObservationUncertainty,
}

impl ObservationMetadata {
    pub fn at(
        tick: u64,
        subject: ObservationSubject,
        provenance: ObservationProvenance,
        uncertainty: ObservationUncertainty,
    ) -> Self {
        Self { subject, provenance, observed_tick: Some(tick), received_tick: None, uncertainty }
    }

    pub fn received(mut self, tick: u64) -> Self {
        self.received_tick = Some(tick);
        self
    }

    pub fn age_ticks(&self, now: u64) -> Option<u64> {
        self.observed_tick.map(|observed| now.saturating_sub(observed))
    }

    pub fn is_valid(&self) -> bool {
        match &self.uncertainty {
            ObservationUncertainty::Quantized { field, step } => !field.is_empty() && step.is_finite() && *step > 0.0,
            _ => true,
        }
    }
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
