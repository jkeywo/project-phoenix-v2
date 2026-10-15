//! Plan release commits in the same World owner as existing situation arming.
use super::*;
use crate::world::activities::{ActivityRequest, SituationConfig, SituationInstances};
use std::collections::BTreeSet;

impl Decomposition {
    fn ready(&self, node: &str, instances: &SituationInstances) -> bool {
        self.dependencies
            .iter()
            .filter(|edge| edge.to == node)
            .all(|edge| {
                instances
                    .instances
                    .get(&self.leaves[&edge.from].id)
                    .is_some_and(|source| edge.when.satisfied_tick(source).is_some())
            })
    }
    fn bound(&self, node: &str, choice: &PlanChoice) -> SituationConfig {
        let mut config = self.leaves[node].clone();
        match &mut config.request {
            ActivityRequest::Repair { actor, .. }
            | ActivityRequest::Delivery { actor, .. }
            | ActivityRequest::Communication { actor, .. } => *actor = choice.actor_uuid.clone(),
        }
        config
    }
    pub fn validate_history(
        &self,
        choice: &PlanChoice,
        instances: &SituationInstances,
        now: u64,
    ) -> Result<(), String> {
        let namespace = choice.execution.namespace(&choice.id);
        if !super::compile::identifier(&choice.id)
            || matches!(&choice.execution, PlanExecution::Optional {source,ordinal} if source!=&choice.id || !(1..=8).contains(ordinal))
            || (!choice.execution.is_primary()
                && (!choice.root_arguments.is_empty()
                    || !choice.methods.is_empty()
                    || !choice.refused_methods.is_empty()
                    || choice.temporal.is_some()
                    || !choice.temporal_reconsiderations.is_empty()))
            || self.leaves.iter().any(|(node, config)| {
                config.id != format!("{namespace}/{node}")
                    || self
                        .definitions
                        .get(&config.definition)
                        .is_none_or(|definition| definition.id != config.definition)
            })
            || self.definitions.iter().any(|(id, definition)| {
                id != &definition.id
                    || !id.starts_with(&format!("{namespace}/"))
                    || !self.leaves.values().any(|config| &config.definition == id)
            })
        {
            return Err("decomposition and frozen execution identity disagree".into());
        }
        if choice.chosen_tick > now
            || choice.released.len() > MAX_LEAVES
            || choice
                .temporal_reconsiderations
                .iter()
                .any(|row| row.tick > now)
        {
            return Err("invalid plan choice/release clock or bound".into());
        }
        self.validate_dispositions(choice, instances, now)?;
        let mut expected = BTreeSet::new();
        for (node, tick) in &choice.released {
            if !self.leaves.contains_key(node) || *tick < choice.chosen_tick || *tick > now {
                return Err("invalid released HTN node/tick".into());
            }
            if choice.effective_temporal().is_some_and(|saved| {
                saved
                    .nodes()
                    .and_then(|nodes| nodes.get(node))
                    .is_none_or(|interval| *tick < interval.start_tick)
            }) {
                return Err("HTN leaf released before its committed forecast start".into());
            }
            let bound = self.bound(node, choice);
            expected.insert(bound.id.clone());
            let actual = instances
                .instances
                .get(&bound.id)
                .ok_or("released HTN situation is missing")?;
            if actual.request != bound.request
                || actual.definition != bound.definition
                || actual.armed_tick != *tick
            {
                return Err("released HTN situation binding/frontier mismatch".into());
            }
            for edge in self.dependencies.iter().filter(|edge| &edge.to == node) {
                if !choice.released.contains_key(&edge.from) {
                    return Err("released HTN task lacks its released predecessor".into());
                }
                let original = instances
                    .instances
                    .get(&self.leaves[&edge.from].id)
                    .and_then(|source| edge.when.satisfied_tick(source))
                    .ok_or("released HTN task lacks acknowledged predecessor evidence")?;
                if original > *tick {
                    return Err(
                        "HTN task was released before its original prerequisite evidence".into(),
                    );
                }
            }
        }
        let prefix = format!("{}/", choice.execution.namespace(&choice.id));
        if instances
            .instances
            .keys()
            .any(|id| id.starts_with(&prefix) && !expected.contains(id))
        {
            return Err("surplus unreleased HTN situation".into());
        }
        Ok(())
    }
}
impl CompiledPlanning {
    pub fn validate_history(
        &self,
        choice: &PlanChoice,
        instances: &SituationInstances,
        now: u64,
    ) -> Result<Decomposition, String> {
        let graph = self.derive(choice)?;
        self.validate_schedule(choice, &graph)?;
        graph.validate_history(choice, instances, now)?;
        Ok(graph)
    }
    /// Call only for this actual accepted actor. A missing actor does not call
    /// release; historical releases are retained and can never silently rearm.
    /// Both candidate owners commit together, or neither changes.
    pub fn release(
        &self,
        choice: &mut PlanChoice,
        instances: &mut SituationInstances,
        actor_uuid: &str,
        now: u64,
    ) -> Result<Vec<String>, String> {
        if actor_uuid != choice.actor_uuid {
            return Err("HTN release actor is not its accepted physical binding".into());
        }
        let graph = self.validate_history(choice, instances, now)?;
        graph.release(choice, instances, actor_uuid, now)
    }
    /// Actual released and unaccepted requests only. Publication decides its
    /// exact actor/System/feed visibility; this function grants no authority.
    pub fn ready_requests(
        &self,
        choice: &PlanChoice,
        instances: &SituationInstances,
        now: u64,
    ) -> Result<Vec<SituationConfig>, String> {
        let graph = self.validate_history(choice, instances, now)?;
        Ok(choice
            .released
            .keys()
            .filter(|node| {
                !choice
                    .node_dispositions
                    .get(*node)
                    .is_some_and(PlanNodeDisposition::withheld)
            })
            .filter_map(|node| {
                let config = graph.bound(node, choice);
                instances
                    .instances
                    .get(&config.id)
                    .filter(|instance| instance.accepted.is_none())
                    .map(|_| config)
            })
            .collect())
    }
}

impl Decomposition {
    /// Shared release transaction for source-derived authored or generated
    /// decompositions; all leaves use the same Situation admission owner.
    pub fn release(
        &self,
        choice: &mut PlanChoice,
        instances: &mut SituationInstances,
        actor_uuid: &str,
        now: u64,
    ) -> Result<Vec<String>, String> {
        if actor_uuid != choice.actor_uuid {
            return Err("release actor mismatch".into());
        }
        self.validate_history(choice, instances, now)?;
        let mut next_choice = choice.clone();
        let mut next_instances = instances.clone();
        let mut released = Vec::new();
        for node in self.leaves.keys() {
            if next_choice.released.contains_key(node)
                || choice
                    .node_dispositions
                    .get(node)
                    .is_some_and(PlanNodeDisposition::withheld)
                || !self.ready(node, &next_instances)
            {
                continue;
            }
            if next_choice.effective_temporal().is_some_and(|saved| {
                saved
                    .nodes()
                    .is_none_or(|nodes| now < nodes[node].start_tick)
            }) {
                continue;
            }
            let config = self.bound(node, choice);
            next_instances.arm(&config, &self.definitions[&config.definition], now)?;
            next_choice.released.insert(node.clone(), now);
            released.push(config.id);
        }
        self.validate_history(&next_choice, &next_instances, now)?;
        *choice = next_choice;
        *instances = next_instances;
        Ok(released)
    }
    pub fn ready_requests(
        &self,
        choice: &PlanChoice,
        instances: &SituationInstances,
        now: u64,
    ) -> Result<Vec<SituationConfig>, String> {
        self.validate_history(choice, instances, now)?;
        Ok(choice
            .released
            .keys()
            .filter(|node| {
                !choice
                    .node_dispositions
                    .get(*node)
                    .is_some_and(PlanNodeDisposition::withheld)
            })
            .filter_map(|node| {
                let config = self.bound(node, choice);
                instances
                    .instances
                    .get(&config.id)
                    .filter(|instance| instance.accepted.is_none())
                    .map(|_| config)
            })
            .collect())
    }
    /// Accepted future intent is information only. A missing instance is not
    /// an assignment; no physical admission or reservation happens here.
    pub fn intent_requests(
        &self,
        choice: &PlanChoice,
        instances: &SituationInstances,
        now: u64,
    ) -> Result<Vec<PlanIntentRequest>, String> {
        use crate::world::activities::SituationCondition as Condition;
        use phoenix_model::activity::{PlannedPrerequisite, PlannedPrerequisiteKind as Kind};
        self.validate_history(choice, instances, now)?;
        if choice
            .effective_temporal()
            .is_some_and(|schedule| schedule.nodes().is_none())
        {
            return Ok(Vec::new());
        }
        Ok(self
            .leaves
            .keys()
            .filter_map(|node| {
                let config = self.bound(node, choice);
                if instances
                    .instances
                    .get(&config.id)
                    .is_some_and(|row| row.accepted.is_some())
                {
                    return None;
                }
                let prerequisites = self
                    .dependencies
                    .iter()
                    .filter(|edge| &edge.to == node)
                    .map(|edge| {
                        let predecessor = &self.leaves[&edge.from].id;
                        PlannedPrerequisite {
                            id: predecessor.clone(),
                            condition: match &edge.when {
                                Condition::Accepted => Kind::Accepted,
                                Condition::Achieved => Kind::Achieved,
                                Condition::Settled { outcome } => {
                                    Kind::Settled { outcome: *outcome }
                                }
                                Condition::Received => Kind::Received,
                                Condition::Returned => Kind::Returned,
                                Condition::CommunicationRead {} => Kind::CommunicationRead,
                                Condition::CommunicationReported {} => Kind::CommunicationReported,
                                Condition::CommunicationDeclined {} => Kind::CommunicationDeclined,
                                Condition::ScenarioApplied { state } => Kind::ScenarioApplied {
                                    state: state.clone(),
                                },
                            },
                            satisfied: instances
                                .instances
                                .get(predecessor)
                                .is_some_and(|row| edge.when.satisfied_tick(row).is_some()),
                        }
                    })
                    .collect();
                Some(PlanIntentRequest {
                    config,
                    chosen_tick: choice.chosen_tick,
                    released_tick: choice.released.get(node).copied(),
                    prerequisites,
                    disposition: choice.node_dispositions.get(node).map(|row|phoenix_model::activity::PlannedRepairDisposition{
                        kind:match row.latest.reason {
                            PlanNodeReason::PartsUnavailable=>phoenix_model::activity::PlannedRepairDispositionKind::PartsUnavailable,
                            PlanNodeReason::RouteUnavailable=>phoenix_model::activity::PlannedRepairDispositionKind::RouteUnavailable,
                            PlanNodeReason::WorkInProgress=>phoenix_model::activity::PlannedRepairDispositionKind::WorkInProgress,
                            PlanNodeReason::Available=>phoenix_model::activity::PlannedRepairDispositionKind::Available,
                            PlanNodeReason::RestoredExternally=>phoenix_model::activity::PlannedRepairDispositionKind::RestoredExternally,
                        },tick:row.latest.tick().expect("validated receipt"),consequence_applied:row.consequence.as_ref().map(|receipt|receipt.result==crate::world::activities::ScenarioCallResult::Applied),
                    }),
                })
            })
            .collect())
    }
    /// Achievement does not free an assignment; every released physical owner
    /// must have reported terminal settlement and all callbacks must be closed.
    pub fn settled_tick(
        &self,
        choice: &PlanChoice,
        instances: &SituationInstances,
        now: u64,
    ) -> Result<Option<u64>, String> {
        self.validate_history(choice, instances, now)?;
        let mut latest = choice.chosen_tick;
        for node in self.leaves.keys() {
            if let Some(row) = choice
                .node_dispositions
                .get(node)
                .filter(|row| row.latest.terminal())
            {
                let Some(receipt) = row.consequence.as_ref().filter(|receipt| {
                    receipt.result == crate::world::activities::ScenarioCallResult::Applied
                }) else {
                    return Ok(None);
                };
                latest = latest.max(receipt.tick);
                continue;
            }
            let Some(instance) = instances.instances.get(&self.leaves[node].id) else {
                return Ok(None);
            };
            let Some(settled) = &instance.settled else {
                return Ok(None);
            };
            if instance.state.is_some() {
                return Ok(None);
            }
            latest = latest.max(settled.tick).max(
                instance
                    .activation_ticks
                    .values()
                    .copied()
                    .max()
                    .unwrap_or(0),
            );
        }
        Ok(Some(latest))
    }
}
