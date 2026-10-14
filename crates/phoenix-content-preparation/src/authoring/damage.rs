//! Closed physical authoring vocabulary, derived from the actual model enums.
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkshopDamageSchema {
    pub damage_types: Vec<String>,
    pub bindings: Vec<String>,
    pub outcomes: Vec<String>,
}

pub fn schema() -> WorkshopDamageSchema {
    use phoenix_model::damage::{DamageBinding, DamageType, RemedyOutcome};
    let name = |value: serde_json::Value| value.as_str().expect("closed enum name").to_owned();
    WorkshopDamageSchema {
        damage_types: [DamageType::Physical, DamageType::Laser]
            .into_iter()
            .map(|kind| name(serde_json::to_value(kind).expect("damage type")))
            .collect(),
        bindings: [
            DamageBinding::Operation,
            DamageBinding::Intake,
            DamageBinding::Cooling,
        ]
        .into_iter()
        .map(|kind| name(serde_json::to_value(kind).expect("damage binding")))
        .collect(),
        outcomes: [
            RemedyOutcome::Structural,
            RemedyOutcome::Full,
            RemedyOutcome::Patch { factor: 0.5 },
        ]
        .into_iter()
        .map(|kind| name(serde_json::to_value(kind).expect("repair outcome")["kind"].clone()))
        .collect(),
    }
}

#[cfg(test)]
#[path = "damage_tests.rs"]
mod tests;
