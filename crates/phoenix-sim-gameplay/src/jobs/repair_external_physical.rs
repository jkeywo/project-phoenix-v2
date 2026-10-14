//! A visited actor's physical remedies, owned by the existing external Job.
use super::physical::{is_patched, needs_work, recipes, validate_target};
use super::*;
use crate::ship::compartment_damage::{DamageDefinition, DamageState};
use phoenix_model::damage::{ComponentRef, PartRequirement, RemedyConfig, RepairWorkTarget};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalPhysicalRepairWork {
    pub target_uuid: String,
    pub target: Option<RepairWorkTarget>,
    pub recipe: Option<String>,
    pub work_secs: Option<f32>,
    pub requirements: Vec<PartRequirement>,
    pub progress: f32,
    pub stage: u64,
    pub reserved: bool,
    pub waiting_parts: bool,
}
impl ExternalPhysicalRepairWork {
    fn new(target_uuid: String) -> Self {
        Self {
            target_uuid,
            target: None,
            recipe: None,
            work_secs: None,
            requirements: Vec::new(),
            progress: 0.0,
            stage: 0,
            reserved: false,
            waiting_parts: false,
        }
    }
    pub fn claim(&self, id: u64) -> String {
        format!("external-repair/{id}/{}", self.stage)
    }
    fn clear_stage(&mut self) {
        self.target = None;
        self.recipe = None;
        self.work_secs = None;
        self.requirements.clear();
        self.progress = 0.0;
        self.reserved = false;
        self.waiting_parts = false;
        self.stage = self.stage.saturating_add(1);
    }
}

impl RepairTeams {
    /// Validate local identity before accepting continuation. A cancelled or
    /// replaced Job may retain a stage solely for deterministic stock release.
    pub(super) fn validate_external_physical_shape(
        &self,
        saved: &RepairJobState,
    ) -> Result<(), String> {
        for (id, work) in &saved.external_physical_work {
            if *id == 0
                || *id >= saved.next_id
                || saved.physical_work.contains_key(id)
                || work.target_uuid.trim().is_empty()
                || !work.progress.is_finite()
                || work.progress < 0.0
                || work.reserved != work.recipe.is_some()
                || work.reserved != work.work_secs.is_some()
                || (work.progress > 0.0 && !work.reserved)
                || (work.reserved && work.target.is_none())
                || work
                    .work_secs
                    .is_some_and(|secs| !secs.is_finite() || secs <= 0.0 || work.progress > secs)
                || (!work.reserved && !work.requirements.is_empty())
                || work.requirements.iter().any(|part| {
                    part.resource.trim().is_empty()
                        || part.service.trim().is_empty()
                        || part.amount <= 0
                })
                || (work.stage == u64::MAX && work.reserved)
            {
                return Err("invalid external physical repair continuation".into());
            }
        }
        Ok(())
    }

    /// Cross-entity preflight: only a still-active matching commitment can
    /// resume work, and its selected stage must match the real target recipe.
    pub fn validate_external_physical_targets(
        &self,
        saved: &RepairJobState,
        definitions: &BTreeMap<String, DamageDefinition>,
        target_roster: &BTreeSet<String>,
    ) -> Result<(), String> {
        self.validate_external_physical_shape(saved)?;
        for (id, work) in &saved.external_physical_work {
            let active=saved.teams.iter().flatten().any(|job| job.id.0==*id && job.outcome.is_none() &&
                matches!(&job.activity,RepairActivity::External{target} if target==&work.target_uuid));
            if !active {
                continue;
            }
            if !target_roster.contains(&work.target_uuid) {
                // A captured delete→maintenance frontier can only release this
                // cached source claim next tick; it cannot resume remote work.
                continue;
            }
            let definition = definitions
                .get(&work.target_uuid)
                .ok_or("active external repair target has no physical definition")?;
            if let Some(target) = &work.target {
                validate_target(definition, target)?;
            }
            if let Some(name) = &work.recipe {
                let target = work.target.as_ref().ok_or("external stage has no target")?;
                let recipe = definition
                    .config
                    .remedies
                    .get(name)
                    .ok_or("unknown external remedy")?;
                if !recipes(definition, target).contains(name)
                    || Some(recipe.work_secs) != work.work_secs
                    || recipe.requirements != work.requirements
                {
                    return Err("external stage recipe does not match target equipment".into());
                }
            }
        }
        Ok(())
    }

    /// Release is independent of target content: recall, range loss, replacement
    /// or disappearance must not strand actual source-stock reservations.
    pub fn cleanup_external_physical(
        &mut self,
        mut target_exists: impl FnMut(&str) -> bool,
        mut release: impl FnMut(&str),
    ) {
        let stale:Vec<_>=self.state.external_physical_work.iter().filter_map(|(id,work)| {
            let active=self.state.teams.iter().flatten().any(|job| job.id.0==*id && job.outcome.is_none() &&
                matches!(&job.activity,RepairActivity::External{target} if target==&work.target_uuid));
            (!active || !target_exists(&work.target_uuid)).then_some(*id)
        }).collect();
        for id in stale {
            if let Some(work) = self.state.external_physical_work.remove(&id) {
                if work.reserved {
                    release(&work.claim(id));
                }
            }
        }
    }

    /// The caller has already established the team's ordinary range/presence.
    /// `claimed` contains other jobs on this actor; no global claim owner exists.
    #[allow(clippy::too_many_arguments)]
    pub fn tick_external_physical(
        &mut self,
        dt: f32,
        target_uuid: &str,
        definition: &DamageDefinition,
        state: &mut DamageState,
        claimed: &BTreeSet<RepairWorkTarget>,
        mut stock: impl FnMut(RepairStockAction, &str, Option<&RemedyConfig>) -> bool,
        mut tools: impl FnMut(&SystemId, f32) -> f32,
    ) {
        if !dt.is_finite()
            || dt <= 0.0
            || state.catastrophic
            || self.external_target() != Some(target_uuid)
        {
            return;
        }
        if !self.step_external() {
            return;
        }
        let owner = self.state.owner.clone();
        let team = self.external_team().expect("active external job");
        let id = self.state.teams[team].as_ref().expect("external team").id.0;
        let work = self
            .state
            .external_physical_work
            .entry(id)
            .or_insert_with(|| ExternalPhysicalRepairWork::new(target_uuid.into()));
        if work.target_uuid != target_uuid || work.stage == u64::MAX {
            return;
        }
        if work.target.as_ref().is_some_and(|target| {
            claimed.contains(target) || !needs_work(definition, state, target)
        }) {
            if work.reserved {
                stock(RepairStockAction::Release, &work.claim(id), None);
            }
            work.clear_stage();
        }
        if work.target.is_none() {
            let mut candidates: Vec<_> = definition
                .interior
                .compartments
                .iter()
                .map(|room| RepairWorkTarget::Compartment {
                    compartment: room.id.clone(),
                })
                .collect();
            candidates.sort();
            for (system, components) in &definition.components {
                for component in components.keys() {
                    candidates.push(RepairWorkTarget::Component {
                        system: system.clone(),
                        component: component.clone(),
                    });
                }
            }
            work.target = candidates.into_iter().find(|target| {
                !claimed.contains(target)
                    && needs_work(definition, state, target)
                    && (!is_patched(state, target)
                        || recipes(definition, target)
                            .first()
                            .and_then(|name| definition.config.remedies.get(name))
                            .is_some_and(|recipe| {
                                stock(RepairStockAction::Check, &work.claim(id), Some(recipe))
                            }))
            });
        }
        let Some(target) = work.target.as_ref() else {
            return;
        };
        if work.recipe.is_none() {
            let mut choices = recipes(definition, target);
            if is_patched(state, target) {
                choices.truncate(1);
            }
            if let Some(name) = choices.into_iter().find(|name| {
                definition.config.remedies.get(name).is_some_and(|recipe| {
                    stock(RepairStockAction::Check, &work.claim(id), Some(recipe))
                })
            }) {
                let recipe = &definition.config.remedies[&name];
                if stock(RepairStockAction::Reserve, &work.claim(id), Some(recipe)) {
                    work.work_secs = Some(recipe.work_secs);
                    work.requirements = recipe.requirements.clone();
                    work.recipe = Some(name);
                    work.reserved = true;
                    work.waiting_parts = false;
                }
            }
            if !work.reserved {
                work.waiting_parts = true;
                return;
            }
        }
        let Some(name) = work.recipe.as_ref() else {
            return;
        };
        let Some(recipe) = definition.config.remedies.get(name) else {
            return;
        };
        // Crew always provide the selected remedy's manual base. Request only
        // the powered acceleration which can contribute to this stage.
        let remaining = (recipe.work_secs - work.progress).max(0.0);
        let manual = (dt * recipe.manual_work_fraction).min(remaining);
        let acceleration = 1.0 - recipe.manual_work_fraction;
        let tool_dt = if acceleration > 0.0 {
            dt.min((remaining - manual) / acceleration)
        } else {
            0.0
        };
        let funded = if tool_dt > 0.0 {
            tools(&owner, tool_dt).clamp(0.0, 1.0)
        } else {
            0.0
        };
        work.progress =
            (work.progress + manual + tool_dt * acceleration * funded).min(recipe.work_secs);
        if work.progress < recipe.work_secs {
            return;
        }
        let mut restored = state.clone();
        let result = match target {
            RepairWorkTarget::Compartment { compartment } => restored
                .restore_structure(definition, compartment, recipe.structural_hp.unwrap_or(0.0))
                .map(|_| ()),
            RepairWorkTarget::Component { system, component } => restored
                .remedy_component(
                    definition,
                    &ComponentRef {
                        system: system.clone(),
                        component: component.clone(),
                    },
                    name,
                )
                .map(|_| ()),
        };
        if result.is_err() || !stock(RepairStockAction::Consume, &work.claim(id), Some(recipe)) {
            return;
        }
        *state = restored;
        // The bounded stage is complete. Existing external commitment remains
        // aboard until ordinary recall/range/order withdrawal; this avoids an AI
        // repeatedly redispatching under a still-active FieldRepair directive.
        work.clear_stage();
    }
}

#[cfg(test)]
#[path = "repair_external_physical_tests.rs"]
mod tests;
