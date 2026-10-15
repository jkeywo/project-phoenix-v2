//! Authored prepared groups and the public assignment view. Commands still
//! address each individual weapon System through ordinary admission.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponGroupConfig {
    pub id: String,
    pub label: String,
    pub members: Vec<SystemId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponGroupAssignment {
    pub group: String,
    /// Public contact handle. None is an explicit hold, not primary fallback.
    pub target: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum WeaponIndividualAssignment {
    Hold,
    Target { contact: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponTargetAssignment {
    pub group: Option<WeaponGroupAssignment>,
    /// Absence follows the base assignment. Explicit Hold survives JSON.
    pub individual: Option<WeaponIndividualAssignment>,
    /// A public current-contact fallback. It never overrides an explicit order
    /// or a primary target already admitted as eligible by the owning weapon.
    pub automatic: Option<String>,
}

impl WeaponTargetAssignment {
    pub fn target<'a>(&'a self, primary: Option<&'a str>) -> Option<&'a str> {
        match &self.individual {
            Some(WeaponIndividualAssignment::Hold) => None,
            Some(WeaponIndividualAssignment::Target { contact }) => Some(contact),
            None => self.group.as_ref().map_or_else(
                || primary.or(self.automatic.as_deref()),
                |group| group.target.as_deref(),
            ),
        }
    }

    pub fn follows_primary(&self) -> bool {
        !self.has_explicit_order()
    }

    pub fn has_explicit_order(&self) -> bool {
        self.individual.is_some() || self.group.is_some()
    }

    pub fn is_empty(&self) -> bool {
        !self.has_explicit_order() && self.automatic.is_none()
    }
}
