//! Private Test input. This is a choice of initial hypothetical equipment,
//! never a machinery command, assembly identity or installation request.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_REQUEST_BYTES: usize = 32_768;
pub const CRUISER: &str = "assets/entities/alliance_cruiser.toml";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Workload {
    SustainedEngagement,
    BurstRecovery,
}
impl Workload {
    pub fn world(self) -> &'static str {
        match self {
            Self::SustainedEngagement => "assets/worlds/fitting_rehearsal_sustained.toml",
            Self::BurstRecovery => "assets/worlds/fitting_rehearsal_burst.toml",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RehearsalRequest {
    pub version: u8,
    pub plan: FittingPlan,
    pub workload: Workload,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FittingPlan {
    pub version: u8,
    pub hull_path: String,
    pub content_id: String,
    pub scope: String,
    #[serde(deserialize_with = "unique_map")]
    pub sections: BTreeMap<String, PlanSection>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanSection {
    pub explicit: bool,
    #[serde(deserialize_with = "unique_map")]
    pub choices: BTreeMap<String, String>,
}

fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Unique<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Unique<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object with unique bounded keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, T>()? {
                if out.len() >= 64 || !identifier(&key) || out.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate, invalid or excessive fitting keys",
                    ));
                }
            }
            Ok(out)
        }
    }
    deserializer.deserialize_map(Unique(std::marker::PhantomData))
}

fn identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl RehearsalRequest {
    /// Revalidate after serde too: native in-process callers need the same gate.
    pub fn validate(&self, selection: &super::test_protocol::TestSelection) -> Result<(), String> {
        let plan = &self.plan;
        if self.version != 1
            || plan.version != 1
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_REQUEST_BYTES
            || selection.ship != CRUISER
            || plan.hull_path != selection.ship
            || selection.world != self.workload.world()
            || selection.slot.as_deref() != Some("rehearsal")
            || plan.content_id.len() != 16
            || !plan
                .content_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || plan.sections.is_empty()
            || plan.sections.len() > 8
            || (plan.scope != "*" && !identifier(&plan.scope))
            || plan.sections.iter().any(|(station, section)| {
                !identifier(station)
                    || section.choices.is_empty()
                    || section
                        .choices
                        .iter()
                        .any(|(slot, def)| !identifier(slot) || !identifier(def))
            })
            || plan
                .sections
                .values()
                .map(|s| s.choices.len())
                .sum::<usize>()
                > 64
        {
            return Err("Unsupported or malformed fitting rehearsal request".into());
        }
        Ok(())
    }

    /// Normalize against the actual include-resolved accepted planner context.
    /// A Station favourite leaves other Stations at their authored defaults.
    pub fn choices(
        &self,
        context: &phoenix_model::system_status::FittingPlannerContext,
    ) -> Result<BTreeMap<String, String>, String> {
        let plan = &self.plan;
        if context.hull_path != plan.hull_path || context.content_id != plan.content_id {
            return Err("Rehearsal favourite belongs to a different captured hull".into());
        }
        let homes: BTreeSet<_> = context
            .slots
            .iter()
            .map(|s| s.home_station.as_str())
            .collect();
        let expected: BTreeSet<_> = if plan.scope == "*" {
            homes.clone()
        } else if homes.contains(plan.scope.as_str()) {
            BTreeSet::from([plan.scope.as_str()])
        } else {
            return Err("Unknown favourite Station".into());
        };
        if plan
            .sections
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != expected
        {
            return Err("Favourite must contain exactly its offered Station sections".into());
        }
        let mut choices = BTreeMap::new();
        for (station, section) in &plan.sections {
            let slots: Vec<_> = context
                .slots
                .iter()
                .filter(|s| &s.home_station == station)
                .collect();
            if section
                .choices
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                != slots
                    .iter()
                    .map(|s| s.slot.as_str())
                    .collect::<BTreeSet<_>>()
            {
                return Err("Favourite must contain exactly its offered fitting slots".into());
            }
        }
        for slot in &context.slots {
            let choice = plan
                .sections
                .get(&slot.home_station)
                .and_then(|section| section.choices.get(&slot.slot))
                .unwrap_or(&slot.default_equipment);
            if !slot.compatible_equipment.contains(choice) {
                return Err("Favourite equipment is not compatible with its actual slot".into());
            }
            choices.insert(slot.slot.clone(), choice.clone());
        }
        Ok(choices)
    }
}
