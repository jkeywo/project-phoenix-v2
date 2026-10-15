//! Accepted finite personnel attached to existing physical Repair teams.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Stable identity within the single retained roster, independent of current holder.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonRef {
    pub origin: String,
    pub officer: String,
}
impl PersonRef {
    pub fn new(origin: impl Into<String>, officer: impl Into<String>) -> Self {
        Self {
            origin: origin.into(),
            officer: officer.into(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.origin.trim().is_empty()
            || self.origin.trim() != self.origin
            || self.origin.len() > 128
            || self.origin.chars().any(char::is_control)
            || !valid_id(&self.officer)
        {
            return Err(
                "personnel reference requires an actual origin identity and accepted officer ID"
                    .into(),
            );
        }
        Ok(())
    }
}
/// One canonical compound-key map, encoded as sorted rows in JSON and postcard.
/// Duplicate/unsorted rows are refused rather than silently replacing a request.
pub mod order_rows {
    use super::*;
    use serde::{Deserializer, Serializer};
    use std::collections::BTreeMap;
    pub fn serialize<S: Serializer>(
        value: &BTreeMap<PersonRef, PersonnelDestinationRequest>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<PersonRef, PersonnelDestinationRequest>, D::Error> {
        let rows = Vec::<(PersonRef, PersonnelDestinationRequest)>::deserialize(deserializer)?;
        if rows.windows(2).any(|pair| pair[0].0 >= pair[1].0)
            || rows.iter().any(|(person, _)| person.validate().is_err())
        {
            return Err(serde::de::Error::custom(
                "personnel request rows require unique sorted accepted references",
            ));
        }
        Ok(rows.into_iter().collect())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DutyCompetency {
    MechanicalRepair,
    MedicalCare,
    PersonnelTransport,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelConfig {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::guided_guidance::deserialize_optional_profile"
    )]
    pub care: Option<CareConfig>,
    #[serde(default, rename = "repair_team")]
    pub repair_teams: Vec<DutyTeamConfig>,
    #[serde(default, rename = "officer")]
    pub officers: Vec<DutyOfficerConfig>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyTeamConfig {
    pub id: String,
    pub label: String,
    /// Binds an existing finite Repair team; never creates an additional team.
    pub team_idx: u32,
    pub competencies: Vec<DutyCompetency>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyOfficerConfig {
    #[serde(default)]
    pub initial_condition: PersonnelCondition,
    pub id: String,
    pub label: String,
    pub home_compartment: String,
    pub competencies: Vec<DutyCompetency>,
    /// Applies only to actual compatible internal physical remedy progress.
    pub repair_work_multiplier: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonnelCondition {
    #[default]
    Fit,
    Minor,
    Severe,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CareConfig {
    pub system: crate::identity::SystemId,
    pub minor_work_secs: f32,
    pub severe_work_secs: f32,
    #[serde(rename = "requirement")]
    pub requirements: Vec<crate::damage::PartRequirement>,
}
impl CareConfig {
    pub fn work_secs(&self, condition: PersonnelCondition) -> Option<f32> {
        match condition {
            PersonnelCondition::Fit => None,
            PersonnelCondition::Minor => Some(self.minor_work_secs),
            PersonnelCondition::Severe => Some(self.severe_work_secs),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut resources = BTreeSet::new();
        if self.system.0.trim().is_empty()
            || ![self.minor_work_secs, self.severe_work_secs]
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            || self.requirements.is_empty()
            || self.requirements.iter().any(|part| {
                part.resource.trim().is_empty()
                    || part.service.trim().is_empty()
                    || part.amount <= 0
                    || !resources.insert(&part.resource)
            })
        {
            return Err("care requires actual equipment, positive finite work and unique finite medication requirements".into());
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
fn valid_competencies(values: &[DutyCompetency]) -> bool {
    !values.is_empty() && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
impl PersonnelConfig {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(care) = &self.care {
            care.validate()?;
        }
        let mut ids = BTreeSet::new();
        let mut slots = BTreeSet::new();
        for team in &self.repair_teams {
            if !valid_id(&team.id)
                || team.label.trim().is_empty()
                || !ids.insert(&team.id)
                || !slots.insert(team.team_idx)
                || !valid_competencies(&team.competencies)
            {
                return Err(
                    "personnel requires unique actual Repair team bindings and competencies".into(),
                );
            }
        }
        let mut officers = BTreeSet::new();
        for officer in &self.officers {
            if !valid_id(&officer.id)
                || officer.label.trim().is_empty()
                || !valid_id(&officer.home_compartment)
                || !officers.insert(&officer.id)
                || !valid_competencies(&officer.competencies)
                || !officer.repair_work_multiplier.is_finite()
                || officer.repair_work_multiplier < 1.0
            {
                return Err(
                    "personnel requires unique finite officers with supported repair expertise"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfficerLocation {
    /// The one real escort team cursor, not a second passenger position.
    WithTransfer {
        actor: String,
        team_idx: u32,
        job: crate::job::JobId,
    },
    AboardCraft {
        craft: String,
        berth: String,
    },
    /// Terminal physical loss; retaining identity never keeps its ship alive.
    Lost {
        actor: String,
    },
    AtCompartment {
        actor: String,
        compartment: String,
    },
    /// Location is read through the one actual team navigation cursor.
    WithRepairTeam {
        actor: String,
        team_idx: u32,
    },
}

/// Destination intent is not physical custody or a capacity reservation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PersonnelDestination {
    Home,
    Craft { craft: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelDestinationRequest {
    pub destination: PersonnelDestination,
    pub requested_tick: u64,
}

/// The personnel-relevant accepted recipe of a runtime origin. World placement
/// and display names do not alter its finite roster or interior bindings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelSource {
    /// Actual Entity WorldId receipt, never an independently minted identity.
    pub birth_tick: u64,
    pub birth_sequence: u64,
    pub template_path: String,
    pub overrides: Option<toml::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyOfficerState {
    /// Required continuation field; older state needs its original game version.
    pub condition: PersonnelCondition,
    pub location: OfficerLocation,
    pub changed_tick: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelView {
    pub teams: Vec<DutyTeamView>,
    pub officers: Vec<DutyOfficerView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried_destinations: Vec<PersonnelCraftChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelCraftChoice {
    pub craft: String,
    pub name: Option<String>,
    pub free_berths: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonnelTransferStage {
    Collecting,
    Escorting,
    Returning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficerTransferView {
    pub team_idx: u32,
    pub stage: PersonnelTransferStage,
    pub completed: bool,
    pub blocked: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyTeamView {
    pub id: String,
    pub label: String,
    pub team_idx: u32,
    pub competencies: Vec<DutyCompetency>,
    pub officer: Option<PersonRef>,
    pub active_work_multiplier: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficerTreatmentView {
    pub team_idx: u32,
    pub phase: crate::job::JobPhase,
    pub progress: u16,
    pub waiting_medication: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyOfficerView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_request: Option<PersonnelDestinationRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transfer_teams: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<OfficerTransferView>,
    pub condition: PersonnelCondition,
    pub treatment_teams: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub treatment: Option<OfficerTreatmentView>,
    pub person: PersonRef,
    pub label: String,
    pub competencies: Vec<DutyCompetency>,
    pub location: OfficerLocation,
    pub changed_tick: u64,
    pub repair_work_multiplier: f32,
    pub available_teams: Vec<u32>,
    pub can_release: bool,
}

#[cfg(test)]
#[path = "personnel_tests.rs"]
mod tests;
