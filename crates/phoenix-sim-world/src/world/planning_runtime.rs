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
            if next_choice.released.contains_key(node) || !self.ready(node, &next_instances) {
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
    /// Achievement does not free an assignment; every released physical owner
    /// must have reported terminal settlement and all callbacks must be closed.
    pub fn settled_tick(
        &self,
        choice: &PlanChoice,
        instances: &SituationInstances,
        now: u64,
    ) -> Result<Option<u64>, String> {
        self.validate_history(choice, instances, now)?;
        if choice.released.len() != self.leaves.len() {
            return Ok(None);
        }
        let mut latest = choice.chosen_tick;
        for node in self.leaves.keys() {
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
