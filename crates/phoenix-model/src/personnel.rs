//! Accepted finite personnel attached to existing physical Repair teams.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DutyCompetency {
    MechanicalRepair,
    MedicalCare,
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
    AtCompartment {
        compartment: String,
    },
    /// Location is read through the one actual team navigation cursor.
    WithRepairTeam {
        team_idx: u32,
    },
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyTeamView {
    pub id: String,
    pub label: String,
    pub team_idx: u32,
    pub competencies: Vec<DutyCompetency>,
    pub officer: Option<String>,
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
    pub condition: PersonnelCondition,
    pub treatment_teams: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub treatment: Option<OfficerTreatmentView>,
    pub id: String,
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
