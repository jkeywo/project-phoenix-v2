//! Physical work is a specialisation of the existing team assignment. Phase,
//! crew commitment, navigation and private workflow keep their existing owners.
use super::*;
use crate::ship::compartment_damage::{DamageDefinition, DamageState};
use phoenix_model::damage::{ComponentCondition, ComponentRef, RemedyConfig, RepairWorkTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairStockAction {
    Check,
    Reserve,
    Consume,
    Release,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalRepairWork {
    pub system: SystemId,
    pub target: Option<RepairWorkTarget>,
    pub recipe: Option<String>,
    pub progress: f32,
    pub stage: u64,
    pub reserved: bool,
    pub waiting_parts: bool,
    pub requested_target: Option<RepairWorkTarget>,
}
impl PhysicalRepairWork {
    fn new(system: SystemId) -> Self {
        Self {
            system,
            target: None,
            recipe: None,
            progress: 0.0,
            stage: 0,
            reserved: false,
            waiting_parts: false,
            requested_target: None,
        }
    }
    pub fn claim(&self, id: u64) -> String {
        format!("repair/{id}/{}", self.stage)
    }
}

/// Transient results returned only from this committed owner invocation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RepairTickResult {
    pub outcomes: Vec<(
        phoenix_model::job::JobId,
        phoenix_model::activity::ActivityOutcome,
    )>,
}

impl RepairTeams {
    /// Compatibility adapter: acceptance still comes from the same operation.
    pub fn dispatch_physical(
        &mut self,
        team: usize,
        system: SystemId,
        target: Option<RepairWorkTarget>,
        label: String,
    ) -> bool {
        self.dispatch_physical_job(team, system, target, label)
            .is_some()
    }
    pub fn dispatch_physical_job(
        &mut self,
        team: usize,
        system: SystemId,
        target: Option<RepairWorkTarget>,
        label: String,
    ) -> Option<phoenix_model::job::JobId> {
        if self.external_team() == Some(team) || (self.is_refit_team(team) || self.is_treatment_team(team)) || self.refit_claims_system(&system)
            || target.as_ref().is_some_and(|target| matches!(target, RepairWorkTarget::Component { system, .. } if self.refit_claims_system(system))) {
            return None;
        }
        let current = self
            .state
            .jobs
            .teams
            .get(team)
            .and_then(Option::as_ref)
            .map(|job| job.id.0);
        if target.as_ref().is_some_and(|target| {
            self.state.physical_work.iter().any(|(id, work)| {
                Some(*id) != current
                    && (work.target.as_ref() == Some(target)
                        || work.requested_target.as_ref() == Some(target))
            })
        }) {
            return None;
        }
        let redirected = if target.is_some() {
            if let Some(job) = self
                .state
                .jobs
                .teams
                .get_mut(team)
                .and_then(Option::as_mut)
                .filter(|job| job.outcome.is_none())
            {
                if let RepairActivity::Internal { phase, cancelled } = &mut job.activity {
                    *phase = TeamSlot::Travelling {
                        system_id: Some(system.clone()),
                        display_name: Some(label.clone()),
                        elapsed: 0.0,
                        priority: None,
                    };
                    *cancelled = false;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        if redirected {
            self.refresh();
        } else {
            self.dispatch(team, system.clone(), label);
        }
        if let Some(job) = self
            .state
            .jobs
            .teams
            .get(team)
            .and_then(Option::as_ref)
            .filter(|job| {
                job.outcome.is_none() && matches!(job.activity, RepairActivity::Internal { .. })
            })
        {
            self.state
                .physical_work
                .entry(job.id.0)
                .or_insert_with(|| PhysicalRepairWork::new(system))
                .requested_target = target;
            return Some(job.id);
        }
        None
    }
    pub fn install_damage_definition(&mut self, definition: DamageDefinition) {
        self.damage_definition = Some(definition);
    }
    pub(super) fn validate_physical_work(&self, saved: &RepairJobState) -> Result<(), String> {
        let mut targets = BTreeSet::new();
        for (id, work) in &saved.physical_work {
            let definition = self
                .damage_definition
                .as_ref()
                .ok_or("physical work without damage definition")?;
            let job = saved
                .teams
                .iter()
                .flatten()
                .find(|job| job.id.0 == *id)
                .ok_or("physical work has no team assignment")?;
            if !matches!(job.activity, RepairActivity::Internal { .. })
                || job.outcome.is_some()
                || !work.progress.is_finite()
                || work.progress < 0.0
                || (work.progress > 0.0 && !work.reserved)
                || (work.reserved != work.recipe.is_some())
                || work.stage == u64::MAX
                || !definition.components.contains_key(&work.system)
            {
                return Err("invalid physical repair continuation".into());
            }
            if let Some(recipe) = &work.recipe {
                let target = work
                    .target
                    .as_ref()
                    .ok_or("reserved repair has no target")?;
                if !recipes(definition, target).contains(recipe) {
                    return Err("repair recipe is incompatible with its target".into());
                }
                let recipe = definition
                    .config
                    .remedies
                    .get(recipe)
                    .ok_or("unknown repair recipe")?;
                if work.progress > recipe.work_secs {
                    return Err("repair progress exceeds stage".into());
                }
            }
            if let Some(target) = &work.target {
                validate_target(definition, target)?;
            }
            let local_targets: BTreeSet<_> = work
                .target
                .iter()
                .chain(work.requested_target.iter())
                .collect();
            for target in local_targets {
                if !targets.insert(target) {
                    return Err("multiple teams claim the same physical repair task".into());
                }
            }
            if let Some(target) = &work.requested_target {
                validate_target(definition, target)?;
            }
            if let Some(RepairWorkTarget::Component { system, component }) = &work.target {
                let home = definition
                    .interior
                    .placements
                    .get(&work.system)
                    .map(String::as_str);
                if definition.component_room(&ComponentRef {
                    system: system.clone(),
                    component: component.clone(),
                }) != home
                {
                    return Err(
                        "repair component is outside the assigned equipment compartment".into(),
                    );
                }
            }
        }
        Ok(())
    }

    /// Work orders are broad until a team arrives and diagnoses the equipment.
    /// This does not confer remote access to hidden component conditions.
    pub fn tick_physical(
        &mut self,
        dt: f32,
        state: &mut DamageState,
        blocked: &BTreeSet<String>,
        stock: impl FnMut(RepairStockAction, &str, &RemedyConfig) -> bool,
        tools: impl FnMut(&SystemId, f32) -> f32,
    ) {
        self.tick_physical_with_claims(dt, state, blocked, &BTreeSet::new(), stock, tools);
    }
    pub fn tick_physical_with_claims(
        &mut self,
        dt: f32,
        state: &mut DamageState,
        blocked: &BTreeSet<String>,
        foreign_claims: &BTreeSet<RepairWorkTarget>,
        stock: impl FnMut(RepairStockAction, &str, &RemedyConfig) -> bool,
        tools: impl FnMut(&SystemId, f32) -> f32,
    ) {
        let _ = self.tick_physical_with_expertise(
            dt,
            state,
            blocked,
            foreign_claims,
            &[],
            stock,
            tools,
        );
    }
    /// Expertise changes only on-site remedy progress. Real navigation, parts
    /// and the existing per-interval tool grant keep their original clocks.
    #[allow(clippy::too_many_arguments)]
    pub fn tick_physical_with_expertise(
        &mut self,
        dt: f32,
        state: &mut DamageState,
        blocked: &BTreeSet<String>,
        foreign_claims: &BTreeSet<RepairWorkTarget>,
        work_multipliers: &[f32],
        mut stock: impl FnMut(RepairStockAction, &str, &RemedyConfig) -> bool,
        mut tools: impl FnMut(&SystemId, f32) -> f32,
    ) -> RepairTickResult {
        let mut evidence = RepairTickResult::default();
        let live_jobs: BTreeSet<_> = self
            .state
            .jobs
            .teams
            .iter()
            .flatten()
            .filter(|job| job.outcome.is_none())
            .map(|job| job.id)
            .collect();
        if !dt.is_finite() || dt <= 0.0 || state.catastrophic {
            return evidence;
        }
        let Some(definition) = self.damage_definition.clone() else {
            return evidence;
        };
        let Some(graph) = self.layout.as_ref() else {
            return evidence;
        };
        let mut enabled: Vec<_> = self
            .state
            .teams
            .iter_mut()
            .map(|row| {
                row.as_mut()
                    .filter(|job| {
                        job.outcome.is_none()
                            && matches!(job.activity, RepairActivity::Internal { .. })
                    })
                    .is_some_and(|job| evaluate(&self.program, job))
            })
            .collect();
        let mut phases = self.phases();
        let Some(navigation) = self.state.navigation.as_mut() else {
            return evidence;
        };
        let mut physical_slots = phases.slots().to_vec();
        for (index, (slot, team)) in physical_slots.iter_mut().zip(navigation).enumerate() {
            if self.state.jobs.teams[index].as_ref().is_some_and(|job| {
                matches!(
                    job.activity,
                    RepairActivity::Refit { .. } | RepairActivity::Treatment { .. }
                ) && job.outcome.is_none()
            }) {
                continue;
            }
            use crate::ship::interior::TeamLocation;
            if !enabled[index] {
                if let TeamLocation::Crossing { to, .. } = team.location.clone() {
                    graph.advance(team, &to, dt, blocked);
                }
                continue;
            }
            let work = self.state.jobs.teams[index]
                .as_ref()
                .and_then(|job| self.state.physical_work.get(&job.id.0));
            let exact_room = work
                .and_then(|work| {
                    work.requested_target.as_ref().or_else(|| {
                        let current = match slot {
                            TeamSlot::Travelling { system_id, .. }
                            | TeamSlot::Repairing { system_id, .. } => system_id.as_ref(),
                            _ => None,
                        };
                        (current == Some(&work.system))
                            .then_some(work.target.as_ref())
                            .flatten()
                    })
                })
                .and_then(|target| match target {
                    RepairWorkTarget::Compartment { compartment } => Some(compartment.as_str()),
                    RepairWorkTarget::Component { system, component } => {
                        definition.component_room(&ComponentRef {
                            system: system.clone(),
                            component: component.clone(),
                        })
                    }
                });
            match slot {
                TeamSlot::Travelling {
                    system_id, elapsed, ..
                } => {
                    let destination = exact_room.or_else(|| {
                        system_id
                            .as_ref()
                            .and_then(|id| graph.compartment_for_system(id))
                            .map(String::as_str)
                    });
                    if destination.is_some_and(|room| graph.advance(team, room, dt, blocked)) {
                        *elapsed = self.timings.travel_duration;
                    } else {
                        *elapsed = 0.0;
                        enabled[index] = false;
                    }
                }
                TeamSlot::Returning { remaining, .. } => {
                    if graph.advance(team, &graph.config.team_homes[index], dt, blocked) {
                        *remaining = 0.0;
                    } else {
                        *remaining = self.timings.travel_duration;
                        enabled[index] = false;
                    }
                }
                _ => {}
            }
        }
        phases.restore_slots(&physical_slots);
        let mut slots = phases.slots().to_vec();
        let mut claimed: BTreeSet<_> = self
            .state
            .physical_work
            .values()
            .filter_map(|w| w.target.clone())
            .collect();
        claimed.extend(foreign_claims.iter().cloned());
        // Fitting owns the replaceable hardware; another on-site repair team
        // may work in the room, but cannot diagnose or repair that hardware.
        for (system, components) in &definition.components {
            if self.refit_claims_system(system) {
                claimed.extend(
                    components
                        .keys()
                        .map(|component| RepairWorkTarget::Component {
                            system: system.clone(),
                            component: component.clone(),
                        }),
                );
            }
        }
        for (team, slot) in slots.iter_mut().enumerate() {
            let Some(job) = self.state.jobs.teams[team]
                .as_ref()
                .filter(|j| j.outcome.is_none())
            else {
                continue;
            };
            if !matches!(job.activity, RepairActivity::Internal { .. }) {
                continue;
            }
            let id = job.id.0;
            let current = match slot {
                TeamSlot::Travelling { system_id, .. } | TeamSlot::Repairing { system_id, .. } => {
                    system_id.clone()
                }
                _ => None,
            };
            let stale = self
                .state
                .physical_work
                .get(&id)
                .is_some_and(|work| current.as_ref() != Some(&work.system));
            if stale {
                if let Some(work) = self.state.physical_work.remove(&id) {
                    release(&work, id, &definition, &mut stock);
                    if let Some(target) = &work.target {
                        claimed.remove(target);
                    }
                    if let Some(system) = &current {
                        let mut next = PhysicalRepairWork::new(system.clone());
                        next.stage = work.stage.saturating_add(1);
                        next.requested_target = work.requested_target;
                        self.state.physical_work.insert(id, next);
                    }
                }
            }
            if !enabled[team] {
                continue;
            }
            match slot.clone() {
                TeamSlot::Returning { remaining, .. } if remaining <= 0.0 => *slot = TeamSlot::Idle,
                TeamSlot::Travelling {
                    system_id,
                    display_name,
                    priority,
                    elapsed,
                } if elapsed >= self.timings.travel_duration => {
                    *slot = TeamSlot::Repairing {
                        system_id,
                        display_name,
                        priority,
                        priority_system_id: None,
                    };
                    // Travel permission cannot authorize work in a different
                    // workflow phase. Re-evaluate apply_repair next step.
                    continue;
                }
                _ => {}
            }
            let TeamSlot::Repairing {
                system_id: Some(system),
                display_name,
                ..
            } = slot.clone()
            else {
                continue;
            };
            let work = self
                .state
                .physical_work
                .entry(id)
                .or_insert_with(|| PhysicalRepairWork::new(system.clone()));
            if work
                .requested_target
                .as_ref()
                .or(work.target.as_ref())
                .is_some_and(|target| foreign_claims.contains(target))
            {
                // Pending intent does not preempt a foreign crew's active work.
                // It may wait, but cannot reserve, progress or consume this task.
                continue;
            }
            if let Some(requested) = work.requested_target.take() {
                if work.target.as_ref() != Some(&requested) {
                    release(work, id, &definition, &mut stock);
                    if let Some(target) = &work.target {
                        claimed.remove(target);
                    }
                    work.target = Some(requested);
                    work.recipe = None;
                    work.progress = 0.0;
                    work.reserved = false;
                    work.stage = work.stage.saturating_add(1);
                    claimed.insert(work.target.clone().expect("requested target"));
                }
            }
            if let Some(target) = &work.target {
                if !needs_work(&definition, state, target) {
                    release(work, id, &definition, &mut stock);
                    claimed.remove(target);
                    work.target = None;
                    work.recipe = None;
                    work.progress = 0.0;
                    work.reserved = false;
                }
            }
            if work.target.is_none() {
                let room =
                    match &self.state.navigation.as_ref().expect("physical teams")[team].location {
                        crate::ship::interior::TeamLocation::Compartment { compartment } => {
                            Some(compartment.as_str())
                        }
                        _ => None,
                    };
                let mut candidates = Vec::new();
                if let Some(room) = room {
                    candidates.push(RepairWorkTarget::Compartment {
                        compartment: room.into(),
                    });
                    // On-site diagnosis covers the physical room. Other rooms
                    // still require an explicit journey and ordinary reports.
                    for (sid, components) in &definition.components {
                        for component in components.keys() {
                            let reference = ComponentRef {
                                system: sid.clone(),
                                component: component.clone(),
                            };
                            if definition.component_room(&reference) == Some(room) {
                                candidates.push(RepairWorkTarget::Component {
                                    system: sid.clone(),
                                    component: component.clone(),
                                });
                            }
                        }
                    }
                }
                work.target = candidates.into_iter().find(|target| {
                    !claimed.contains(target)
                        && !foreign_claims.contains(target)
                        && needs_work(&definition, state, target)
                        && (!is_patched(state, target)
                            || recipes(&definition, target)
                                .first()
                                .and_then(|name| definition.config.remedies.get(name))
                                .is_some_and(|recipe| {
                                    stock(RepairStockAction::Check, &work.claim(id), recipe)
                                }))
                });
                if let Some(target) = &work.target {
                    claimed.insert(target.clone());
                } else {
                    *slot = TeamSlot::Returning {
                        remaining: self.timings.travel_duration,
                        system_id: Some(system),
                        display_name,
                        queued_system_id: None,
                        queued_display_name: None,
                    };
                    continue;
                }
            }
            let target_room = work
                .target
                .as_ref()
                .and_then(|target| target_room(&definition, target));
            if !matches!(&self.state.navigation.as_ref().expect("physical navigation")[team].location,
                crate::ship::interior::TeamLocation::Compartment{compartment} if Some(compartment.as_str())==target_room)
            {
                // A malformed order cannot spend parts or repair remotely.
                *slot = TeamSlot::Travelling {
                    system_id: Some(system),
                    display_name,
                    elapsed: 0.0,
                    priority: None,
                };
                continue;
            }
            if work.recipe.is_none() {
                let target = work.target.as_ref().expect("diagnosed target");
                let mut choices = recipes(&definition, target);
                if is_patched(state, target) {
                    choices.truncate(1);
                }
                // Prefer full restoration. A patch is selected only when the
                // full recipe cannot currently be funded, never repeatedly.
                work.recipe = choices.into_iter().find(|name| {
                    definition.config.remedies.get(name).is_some_and(|recipe| {
                        stock(RepairStockAction::Check, &work.claim(id), recipe)
                    })
                });
                work.waiting_parts = work.recipe.is_none();
            }
            let Some(name) = work.recipe.as_ref() else {
                continue;
            };
            let recipe = &definition.config.remedies[name];
            let claim = work.claim(id);
            if !work.reserved {
                work.reserved = stock(RepairStockAction::Reserve, &claim, recipe);
                work.waiting_parts = !work.reserved;
                if !work.reserved {
                    work.recipe = None;
                    continue;
                }
            }
            let grant = tools(&self.state.jobs.owner, dt).clamp(0.0, 1.0);
            let rate = recipe.manual_work_fraction + (1.0 - recipe.manual_work_fraction) * grant;
            let expertise = work_multipliers
                .get(team)
                .copied()
                .filter(|value| value.is_finite() && *value >= 1.0)
                .unwrap_or(1.0);
            work.progress = if expertise == 1.0 {
                (work.progress + dt * rate).min(recipe.work_secs)
            } else {
                ((work.progress as f64 + dt as f64 * rate as f64 * expertise as f64)
                    .min(recipe.work_secs as f64)) as f32
            };
            if work.progress >= recipe.work_secs {
                let mut restored = state.clone();
                let target = work
                    .target
                    .as_ref()
                    .expect("reserved work has target")
                    .clone();
                let restoration = match work.target.as_ref().expect("reserved work has target") {
                    RepairWorkTarget::Compartment { compartment } => restored
                        .restore_structure(
                            &definition,
                            compartment,
                            recipe.structural_hp.unwrap_or(0.0),
                        )
                        .map(|_| phoenix_model::activity::RepairAchievement::Structure {
                            restored_hp: restored.compartments[compartment].hp
                                - state.compartments[compartment].hp,
                        }),
                    RepairWorkTarget::Component { system, component } => restored
                        .remedy_component(
                            &definition,
                            &ComponentRef {
                                system: system.clone(),
                                component: component.clone(),
                            },
                            name,
                        )
                        .map(|_| phoenix_model::activity::RepairAchievement::Component {
                            before: state.components[system][component].condition.clone(),
                            after: restored.components[system][component].condition.clone(),
                        }),
                };
                if restoration.is_err() || !stock(RepairStockAction::Consume, &claim, recipe) {
                    continue;
                }
                let result = restoration.expect("checked restoration");
                let changed = match &result {
                    phoenix_model::activity::RepairAchievement::Structure { restored_hp } => {
                        *restored_hp > 0.0
                    }
                    phoenix_model::activity::RepairAchievement::Component { before, after } => {
                        before != after
                    }
                };
                if changed {
                    evidence.outcomes.push((
                        phoenix_model::job::JobId(id),
                        phoenix_model::activity::ActivityOutcome::RepairAchieved {
                            stage: work.stage,
                            target,
                            remedy: name.clone(),
                            result,
                        },
                    ));
                }
                *state = restored;
                if matches!(
                    recipe.outcome,
                    phoenix_model::damage::RemedyOutcome::Patch { .. }
                ) {
                    if let Some(target) = work.target.take() {
                        claimed.remove(&target);
                    }
                    *slot = TeamSlot::Returning {
                        remaining: self.timings.travel_duration,
                        system_id: Some(system),
                        display_name,
                        queued_system_id: None,
                        queued_display_name: None,
                    };
                }
                work.reserved = false;
                work.progress = 0.0;
                work.recipe = None;
                work.stage = work.stage.saturating_add(1);
            }
        }
        phases.restore_slots(&slots);
        self.commit_phases(&phases);
        for job in self.state.jobs.teams.iter().flatten() {
            if live_jobs.contains(&job.id) {
                if let Some(outcome) = job.outcome {
                    evidence.outcomes.push((
                        job.id,
                        phoenix_model::activity::ActivityOutcome::RepairSettled { outcome },
                    ));
                }
            }
        }
        let finished: Vec<_> = self
            .state
            .physical_work
            .keys()
            .copied()
            .filter(|id| {
                !self
                    .state
                    .jobs
                    .teams
                    .iter()
                    .flatten()
                    .any(|job| job.id.0 == *id && job.outcome.is_none())
            })
            .collect();
        for id in finished {
            if let Some(work) = self.state.physical_work.remove(&id) {
                release(&work, id, &definition, &mut stock);
            }
        }
        evidence
    }
}
fn target_room<'a>(
    definition: &'a DamageDefinition,
    target: &'a RepairWorkTarget,
) -> Option<&'a str> {
    match target {
        RepairWorkTarget::Compartment { compartment } => definition
            .compartment(compartment)
            .map(|room| room.id.as_str()),
        RepairWorkTarget::Component { system, component } => {
            definition.component_room(&ComponentRef {
                system: system.clone(),
                component: component.clone(),
            })
        }
    }
}
pub(super) fn is_patched(state: &DamageState, target: &RepairWorkTarget) -> bool {
    match target {
        RepairWorkTarget::Component { system, component } => state
            .components
            .get(system)
            .and_then(|rows| rows.get(component))
            .is_some_and(|row| matches!(row.condition, ComponentCondition::Patched { .. })),
        _ => false,
    }
}
pub(super) fn validate_target(
    definition: &DamageDefinition,
    target: &RepairWorkTarget,
) -> Result<(), String> {
    match target {
        RepairWorkTarget::Compartment { compartment }
            if definition.compartment(compartment).is_some() =>
        {
            Ok(())
        }
        RepairWorkTarget::Component { system, component }
            if definition
                .component(&ComponentRef {
                    system: system.clone(),
                    component: component.clone(),
                })
                .is_some() =>
        {
            Ok(())
        }
        _ => Err("repair target is absent from physical definition".into()),
    }
}
pub(super) fn needs_work(
    definition: &DamageDefinition,
    state: &DamageState,
    target: &RepairWorkTarget,
) -> bool {
    match target {
        RepairWorkTarget::Compartment { compartment } => definition
            .compartment(compartment)
            .zip(state.compartments.get(compartment))
            .is_some_and(|(config, state)| state.hp < f64::from(config.max_hp)),
        RepairWorkTarget::Component { system, component } => state
            .components
            .get(system)
            .and_then(|rows| rows.get(component))
            .is_some_and(|row| {
                matches!(
                    row.condition,
                    ComponentCondition::Damaged
                        | ComponentCondition::Destroyed
                        | ComponentCondition::Patched { .. }
                ) && definition
                    .component(&ComponentRef {
                        system: system.clone(),
                        component: component.clone(),
                    })
                    .is_some_and(|config| {
                        !matches!(row.condition, ComponentCondition::Destroyed)
                            || config.field_replaceable
                    })
            }),
    }
}
pub(super) fn recipes(definition: &DamageDefinition, target: &RepairWorkTarget) -> Vec<String> {
    match target {
        RepairWorkTarget::Compartment { compartment } => definition
            .compartment(compartment)
            .map(|c| vec![c.structural_remedy.clone()])
            .unwrap_or_default(),
        RepairWorkTarget::Component { system, component } => definition
            .component(&ComponentRef {
                system: system.clone(),
                component: component.clone(),
            })
            .map(|c| {
                std::iter::once(c.full_remedy.clone())
                    .chain(c.patch_remedy.clone())
                    .collect()
            })
            .unwrap_or_default(),
    }
}
fn release(
    work: &PhysicalRepairWork,
    id: u64,
    definition: &DamageDefinition,
    stock: &mut impl FnMut(RepairStockAction, &str, &RemedyConfig) -> bool,
) {
    if work.reserved {
        if let Some(recipe) = work
            .recipe
            .as_ref()
            .and_then(|name| definition.config.remedies.get(name))
        {
            stock(RepairStockAction::Release, &work.claim(id), recipe);
        }
    }
}

#[cfg(test)]
#[path = "repair_physical_tests.rs"]
mod tests;
