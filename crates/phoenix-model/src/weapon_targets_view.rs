//! Display-only assignment references. Unavailable never changes retained intent to Hold.
use crate::weapon_targets::{WeaponIndividualAssignment, WeaponTargetAssignment};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum WeaponTargetReference {
    Hold,
    Target { contact: String },
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponGroupAssignmentView {
    pub group: String,
    pub target: WeaponTargetReference,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponTargetAssignmentView {
    pub group: Option<WeaponGroupAssignmentView>,
    pub individual: Option<WeaponTargetReference>,
}
impl From<&WeaponTargetAssignment> for WeaponTargetAssignmentView {
    fn from(intent: &WeaponTargetAssignment) -> Self {
        Self {
            group: intent
                .group
                .as_ref()
                .map(|group| WeaponGroupAssignmentView {
                    group: group.group.clone(),
                    target: group
                        .target
                        .as_ref()
                        .map_or(WeaponTargetReference::Hold, |contact| {
                            WeaponTargetReference::Target {
                                contact: contact.clone(),
                            }
                        }),
                }),
            individual: intent
                .individual
                .as_ref()
                .map(|individual| match individual {
                    WeaponIndividualAssignment::Hold => WeaponTargetReference::Hold,
                    WeaponIndividualAssignment::Target { contact } => {
                        WeaponTargetReference::Target {
                            contact: contact.clone(),
                        }
                    }
                }),
        }
    }
}
