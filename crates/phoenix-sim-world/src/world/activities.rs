//! Opt-in Scenario activity bindings and evidence; physical owners stay elsewhere.
use phoenix_model::{
    activity::*,
    damage::{ComponentCondition, RepairWorkTarget},
    identity::SystemId,
    job::{JobId, JobOutcome},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SITUATIONS: usize = 32;
pub const MAX_SCENARIO_STATES: usize = 32;

pub use phoenix_model::activity::RepairGoal;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityRequest {
    Repair {
        actor: String,
        owner: SystemId,
        team: u32,
        request: phoenix_model::messages::RepairTarget,
        target: RepairWorkTarget,
        goal: RepairGoal,
    },
    Delivery {
        actor: String,
        owner: SystemId,
        source: String,
        destination: String,
        commodity: String,
        amount: i64,
    },
}
impl ActivityRequest {
    pub fn actor(&self) -> &str {
        match self {
            Self::Repair { actor, .. } | Self::Delivery { actor, .. } => actor,
        }
    }
    pub fn owner(&self) -> &SystemId {
        match self {
            Self::Repair { owner, .. } | Self::Delivery { owner, .. } => owner,
        }
    }
    fn validate(&self) -> Result<(), String> {
        if self.actor().trim().is_empty() || self.owner().0.trim().is_empty() {
            return Err("activity requires actual actor/System".into());
        }
        match self {
            Self::Repair {
                request: phoenix_model::messages::RepairTarget::Station(id),
                ..
            } if id.0.trim().is_empty() => {
                return Err("repair request names an empty Station".into())
            }
            Self::Repair {
                request: phoenix_model::messages::RepairTarget::Console(id),
                ..
            } if id.0.trim().is_empty() => {
                return Err("repair request names an empty Console".into())
            }
            Self::Repair {
                request: phoenix_model::messages::RepairTarget::System(id),
                ..
            } if id.0.trim().is_empty() => {
                return Err("repair request names an empty System".into())
            }
            Self::Repair {
                request: phoenix_model::messages::RepairTarget::External,
                ..
            } => return Err("first activity repair requires an internal admitted request".into()),
            Self::Repair {
                request: phoenix_model::messages::RepairTarget::Work(exact),
                target,
                ..
            } if exact != target => {
                return Err("exact admitted repair request disagrees with its named goal".into())
            }
            Self::Repair { target, goal, .. } => match target {
                RepairWorkTarget::Compartment { compartment }
                    if !compartment.trim().is_empty() && *goal == RepairGoal::Structure => {}
                RepairWorkTarget::Component { system, component }
                    if !system.0.trim().is_empty()
                        && !component.trim().is_empty()
                        && *goal != RepairGoal::Structure => {}
                _ => return Err("invalid activity Repair target".into()),
            },
            Self::Delivery {
                source,
                destination,
                commodity,
                amount,
                ..
            } => {
                if source.trim().is_empty()
                    || destination.trim().is_empty()
                    || source == destination
                    || commodity.trim().is_empty()
                    || *amount <= 0
                {
                    return Err("invalid activity delivery".into());
                }
            }
        }
        Ok(())
    }
    fn conflicts(&self, other: &Self) -> bool {
        if self.actor() != other.actor() || self.owner() != other.owner() {
            return false;
        }
        match (self, other) {
            (
                Self::Repair {
                    team: a, target: x, ..
                },
                Self::Repair {
                    team: b, target: y, ..
                },
            ) => a == b || x == y,
            (
                Self::Delivery {
                    source: a,
                    destination: b,
                    ..
                },
                Self::Delivery {
                    source: x,
                    destination: y,
                    ..
                },
            ) => a == x && b == y,
            _ => false,
        }
    }
    fn matches(&self, outcome: &ActivityOutcome) -> bool {
        match (self, outcome) {
            (
                Self::Repair { target, goal, .. },
                ActivityOutcome::RepairAchieved {
                    target: actual,
                    result,
                    ..
                },
            ) => {
                target == actual
                    && match (goal, result) {
                        (RepairGoal::Structure, RepairAchievement::Structure { .. }) => true,
                        (
                            RepairGoal::Patch,
                            RepairAchievement::Component {
                                after: ComponentCondition::Patched { .. },
                                ..
                            },
                        ) => true,
                        (
                            RepairGoal::Full,
                            RepairAchievement::Component {
                                after: ComponentCondition::Healthy,
                                ..
                            },
                        ) => true,
                        _ => false,
                    }
            }
            (Self::Repair { .. }, ActivityOutcome::RepairSettled { .. }) => true,
            (
                Self::Delivery {
                    source,
                    destination,
                    commodity,
                    amount,
                    ..
                },
                ActivityOutcome::Delivery {
                    source: a,
                    destination: b,
                    commodity: c,
                    amount: n,
                    returned,
                    ..
                },
            ) => {
                source == a
                    && c == commodity
                    && n == amount
                    && if *returned {
                        b == source
                    } else {
                        b == destination
                    }
            }
            _ => false,
        }
    }
}

/// Scenario conditions consume only this instance's acknowledged historical facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SituationCondition {
    Accepted,
    Achieved,
    Settled { outcome: JobOutcome },
    Received,
    Returned,
}
impl SituationCondition {
    /// Original acknowledged evidence clock, shared by Scenario activation and
    /// HTN release. No live condition, job slot or stock polling is involved.
    pub fn satisfied_tick(&self, instance: &SituationInstance) -> Option<u64> {
        match self {
            Self::Accepted => instance.accepted.as_ref().and(instance.accepted_tick),
            Self::Achieved => instance.achieved.as_ref().filter(|fact| matches!(fact.outcome, ActivityOutcome::RepairAchieved { .. })).map(|fact| fact.tick),
            Self::Settled { outcome } => instance.settled.as_ref().filter(|fact| matches!(&fact.outcome, ActivityOutcome::RepairSettled { outcome: actual } if actual == outcome)).map(|fact| fact.tick),
            Self::Received => instance.settled.as_ref().filter(|fact| matches!(fact.outcome, ActivityOutcome::Delivery { returned: false, .. })).map(|fact| fact.tick),
            Self::Returned => instance.settled.as_ref().filter(|fact| matches!(fact.outcome, ActivityOutcome::Delivery { returned: true, .. })).map(|fact| fact.tick),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioCall {
    pub script: String,
    pub function: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SituationState {
    pub id: String,
    pub when: SituationCondition,
    #[serde(default)]
    pub call: Option<ScenarioCall>,
    #[serde(default)]
    pub next: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SituationDefinition {
    pub id: String,
    pub initial: String,
    pub state: Vec<SituationState>,
}
impl SituationDefinition {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty()
            || self.state.is_empty()
            || self.state.len() > MAX_SCENARIO_STATES
        {
            return Err("invalid Scenario definition size/identity".into());
        }
        let mut ids = BTreeSet::new();
        for state in &self.state {
            if state.id.trim().is_empty()
                || !ids.insert(&state.id)
                || state.call.as_ref().is_some_and(|call| {
                    call.script.trim().is_empty() || call.function.trim().is_empty()
                })
            {
                return Err("invalid Scenario state/call".into());
            }
        }
        if !ids.contains(&self.initial)
            || self
                .state
                .iter()
                .any(|state| state.next.as_ref().is_some_and(|next| !ids.contains(next)))
        {
            return Err("unknown Scenario transition".into());
        }
        // This first closed lifecycle has no loops: one activation per state.
        let mut visited = BTreeSet::new();
        let mut cursor = Some(&self.initial);
        while let Some(id) = cursor {
            if !visited.insert(id) {
                return Err("cyclic first Scenario lifecycle".into());
            }
            cursor = self
                .state
                .iter()
                .find(|state| &state.id == id)
                .and_then(|state| state.next.as_ref());
        }
        if visited.len() != self.state.len() {
            return Err("unreachable Scenario state".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SituationConfig {
    pub id: String,
    pub definition: String,
    pub request: ActivityRequest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioCallResult {
    Applied,
    Refused,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SituationInstance {
    pub definition: String,
    pub request: ActivityRequest,
    pub armed_tick: u64,
    pub accepted: Option<ActivityJob>,
    pub accepted_tick: Option<u64>,
    pub achieved: Option<ActivityFact>,
    pub settled: Option<ActivityFact>,
    pub state: Option<String>,
    /// Each state is activated once through the existing script-effect boundary.
    pub activations: BTreeMap<String, ScenarioCallResult>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SituationInstances {
    pub instances: BTreeMap<String, SituationInstance>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioActivation {
    pub instance: String,
    pub state: String,
    pub call: Option<ScenarioCall>,
}

impl SituationInstances {
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }
    pub fn arm(
        &mut self,
        config: &SituationConfig,
        definition: &SituationDefinition,
        tick: u64,
    ) -> Result<(), String> {
        definition.validate()?;
        config.request.validate()?;
        if config.id.trim().is_empty()
            || config.definition != definition.id
            || self.instances.contains_key(&config.id)
            || self.instances.len() >= MAX_SITUATIONS
            || self
                .instances
                .values()
                .any(|instance| instance.request.conflicts(&config.request))
        {
            return Err("activity binding identity/capacity/conflict".into());
        }
        self.instances.insert(
            config.id.clone(),
            SituationInstance {
                definition: definition.id.clone(),
                request: config.request.clone(),
                armed_tick: tick,
                accepted: None,
                accepted_tick: None,
                achieved: None,
                settled: None,
                state: Some(definition.initial.clone()),
                activations: BTreeMap::new(),
            },
        );
        Ok(())
    }
    fn accept(
        &mut self,
        actor: &str,
        owner: &SystemId,
        job: JobId,
        tick: u64,
        matches: impl Fn(&ActivityRequest) -> bool,
    ) -> Result<Option<String>, String> {
        if job.0 == 0 {
            return Err("invalid accepted activity JobId".into());
        }
        let scope = ActivityJob {
            actor: actor.into(),
            owner: owner.clone(),
            job,
        };
        let candidates: Vec<_> = self
            .instances
            .iter()
            .filter(|(_, instance)| {
                instance.accepted.is_none()
                    && instance.request.actor() == actor
                    && instance.request.owner() == owner
                    && matches(&instance.request)
            })
            .map(|(id, _)| id.clone())
            .collect();
        if candidates.len() > 1
            || self
                .instances
                .values()
                .any(|instance| instance.accepted.as_ref() == Some(&scope))
        {
            return Err("ambiguous or already bound physical job".into());
        }
        let Some(id) = candidates.first() else {
            return Ok(None);
        };
        let instance = self.instances.get_mut(id).expect("candidate exists");
        if tick < instance.armed_tick {
            return Err("acceptance precedes arming".into());
        }
        instance.accepted = Some(scope);
        instance.accepted_tick = Some(tick);
        Ok(Some(id.clone()))
    }
    pub fn accept_repair(
        &mut self,
        actor: &str,
        owner: &SystemId,
        team: u32,
        request: &phoenix_model::messages::RepairTarget,
        job: JobId,
        tick: u64,
    ) -> Result<Option<String>, String> {
        self.accept(actor,owner,job,tick,|binding|matches!(binding,ActivityRequest::Repair {team:expected,request:actual,..} if *expected==team&&actual==request))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn accept_delivery(
        &mut self,
        actor: &str,
        owner: &SystemId,
        source: &str,
        destination: &str,
        commodity: &str,
        amount: i64,
        job: JobId,
        tick: u64,
    ) -> Result<Option<String>, String> {
        self.accept(actor,owner,job,tick,|request|matches!(request,ActivityRequest::Delivery {source:a,destination:b,commodity:c,amount:n,..} if a==source&&b==destination&&c==commodity&&*n==amount))
    }
    /// Select only a bound, still-reserved evidence slot. Pending events are
    /// checked by the adapter before enqueueing; no later physical polling.
    pub fn fact(
        &self,
        scope: &ActivityJob,
        tick: u64,
        outcome: ActivityOutcome,
    ) -> Option<ActivityFact> {
        let (id, instance) = self
            .instances
            .iter()
            .find(|(_, instance)| instance.accepted.as_ref() == Some(scope))?;
        if !instance.request.matches(&outcome) {
            return None;
        }
        let achieved = matches!(outcome, ActivityOutcome::RepairAchieved { .. });
        if if achieved {
            instance.achieved.is_some()
        } else {
            instance.settled.is_some()
        } {
            return None;
        }
        let fact = ActivityFact {
            instance: id.clone(),
            scope: scope.clone(),
            tick,
            outcome,
        };
        fact.validate(tick).ok()?;
        Some(fact)
    }
    /// Returns true only for a matching fact now safely held by its instance.
    pub fn acknowledge(&mut self, fact: &ActivityFact, now: u64) -> Result<bool, String> {
        fact.validate(now)?;
        let Some(instance) = self.instances.get_mut(&fact.instance) else {
            return Err("unbound activity evidence".into());
        };
        if instance.accepted.as_ref() != Some(&fact.scope)
            || instance.accepted_tick.is_none_or(|tick| tick > fact.tick)
            || !instance.request.matches(&fact.outcome)
        {
            return Err("activity evidence binding mismatch".into());
        }
        let impossible_order = match &fact.outcome {
            ActivityOutcome::RepairAchieved { .. } => instance
                .settled
                .as_ref()
                .is_some_and(|settled| fact.tick > settled.tick),
            ActivityOutcome::RepairSettled { .. } => instance
                .achieved
                .as_ref()
                .is_some_and(|achieved| achieved.tick > fact.tick),
            _ => false,
        };
        if impossible_order {
            return Err("repair achievement occurs after assignment settlement".into());
        }
        let slot = if matches!(fact.outcome, ActivityOutcome::RepairAchieved { .. }) {
            &mut instance.achieved
        } else {
            &mut instance.settled
        };
        if let Some(previous) = slot {
            if previous != fact {
                return Err("conflicting activity evidence".into());
            }
        } else {
            *slot = Some(fact.clone());
        }
        Ok(true)
    }
    pub fn ready(&self, definitions: &[SituationDefinition]) -> Vec<ScenarioActivation> {
        self.instances
            .iter()
            .filter_map(|(id, instance)| {
                let state = instance.state.as_ref()?;
                if instance.activations.contains_key(state) {
                    return None;
                }
                let state = definitions
                    .iter()
                    .find(|definition| definition.id == instance.definition)?
                    .state
                    .iter()
                    .find(|row| &row.id == state)?;
                let ready = state.when.satisfied_tick(instance).is_some();
                ready.then(|| ScenarioActivation {
                    instance: id.clone(),
                    state: state.id.clone(),
                    call: state.call.clone(),
                })
            })
            .collect()
    }
    pub fn finish_activation(
        &mut self,
        activation: &ScenarioActivation,
        result: ScenarioCallResult,
        definitions: &[SituationDefinition],
    ) -> Result<(), String> {
        if !self.ready(definitions).contains(activation) {
            return Err("stale or repeated Scenario activation".into());
        }
        let instance = self
            .instances
            .get_mut(&activation.instance)
            .expect("ready instance");
        let state = definitions
            .iter()
            .find(|row| row.id == instance.definition)
            .expect("ready definition")
            .state
            .iter()
            .find(|row| row.id == activation.state)
            .expect("ready state");
        instance.activations.insert(state.id.clone(), result);
        instance.state = if result == ScenarioCallResult::Applied {
            state.next.clone()
        } else {
            None
        };
        Ok(())
    }
    pub fn validate(&self, definitions: &[SituationDefinition], now: u64) -> Result<(), String> {
        if self.instances.len() > MAX_SITUATIONS {
            return Err("too many situation instances".into());
        }
        for definition in definitions {
            definition.validate()?;
        }
        let mut scopes = BTreeSet::new();
        for (id, instance) in &self.instances {
            instance.request.validate()?;
            let definition = definitions
                .iter()
                .find(|row| row.id == instance.definition)
                .ok_or("unknown situation definition")?;
            if id.trim().is_empty()
                || instance.armed_tick > now
                || instance.accepted.is_some() != instance.accepted_tick.is_some()
                || instance
                    .accepted_tick
                    .is_some_and(|tick| tick < instance.armed_tick || tick > now)
            {
                return Err("invalid activity acceptance frontier".into());
            }
            if let Some(scope) = &instance.accepted {
                if scope.actor != instance.request.actor()
                    || &scope.owner != instance.request.owner()
                    || scope.job.0 == 0
                    || !scopes.insert(scope)
                {
                    return Err("invalid/duplicate activity job binding".into());
                }
            }
            for fact in [&instance.achieved, &instance.settled]
                .into_iter()
                .flatten()
            {
                fact.validate(now)?;
                if fact.instance != *id
                    || instance.accepted.as_ref() != Some(&fact.scope)
                    || instance.accepted_tick.is_none_or(|tick| tick > fact.tick)
                    || !instance.request.matches(&fact.outcome)
                {
                    return Err("invalid held activity evidence".into());
                }
            }
            if instance
                .achieved
                .as_ref()
                .is_some_and(|fact| !matches!(fact.outcome, ActivityOutcome::RepairAchieved { .. }))
                || instance.settled.as_ref().is_some_and(|fact| {
                    matches!(fact.outcome, ActivityOutcome::RepairAchieved { .. })
                })
            {
                return Err("invalid activity evidence slot".into());
            }
            if instance
                .achieved
                .as_ref()
                .zip(instance.settled.as_ref())
                .is_some_and(|(achieved, settled)| achieved.tick > settled.tick)
            {
                return Err("repair achievement occurs after assignment settlement".into());
            }
            // Reconstruct only legal progression from consumed facts and the
            // persisted per-state call result; never inspect current machinery.
            let mut expected = Some(definition.initial.clone());
            let mut seen = BTreeSet::new();
            while let Some(current) = expected.clone() {
                let Some(result) = instance.activations.get(&current) else {
                    break;
                };
                seen.insert(current.clone());
                let state = definition
                    .state
                    .iter()
                    .find(|row| row.id == current)
                    .expect("validated graph");
                let mut projection = instance.clone();
                projection.state = Some(current.clone());
                projection.activations.remove(&current);
                let check = Self {
                    instances: BTreeMap::from([(id.clone(), projection)]),
                };
                if check.ready(definitions).is_empty() {
                    return Err("activation lacks its completion evidence".into());
                }
                expected = if *result == ScenarioCallResult::Applied {
                    state.next.clone()
                } else {
                    None
                };
            }
            if instance.state != expected || seen.len() != instance.activations.len() {
                return Err("invalid Scenario activation frontier".into());
            }
        }
        let values: Vec<_> = self.instances.values().collect();
        for (index, left) in values.iter().enumerate() {
            if values
                .iter()
                .skip(index + 1)
                .any(|right| left.request.conflicts(&right.request))
            {
                return Err("overlapping situation requests".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "activities_tests.rs"]
mod tests;
