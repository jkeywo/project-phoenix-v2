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
            ActivityRequest::Repair { actor, .. } | ActivityRequest::Delivery { actor, .. } => {
                *actor = choice.actor_uuid.clone()
            }
        }
        config
    }
    fn validate_history(
        &self,
        choice: &PlanChoice,
        instances: &SituationInstances,
        now: u64,
    ) -> Result<(), String> {
        if choice.chosen_tick > now || choice.released.len() > MAX_LEAVES {
            return Err("invalid plan choice/release clock or bound".into());
        }
        let mut expected = BTreeSet::new();
        for (node, tick) in &choice.released {
            if !self.leaves.contains_key(node) || *tick < choice.chosen_tick || *tick > now {
                return Err("invalid released HTN node/tick".into());
            }
            if choice.temporal.as_ref().is_some_and(|saved| {
                saved
                    .nodes
                    .get(node)
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
        let prefix = format!("htn/{}/", choice.id);
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
        let mut next_choice = choice.clone();
        let mut next_instances = instances.clone();
        let mut released = Vec::new();
        for node in graph.leaves.keys() {
            if next_choice.released.contains_key(node) || !graph.ready(node, &next_instances) {
                continue;
            }
            if next_choice
                .temporal
                .as_ref()
                .is_some_and(|saved| now < saved.nodes[node].start_tick)
            {
                continue;
            }
            let config = graph.bound(node, choice);
            next_instances.arm(&config, &graph.definitions[&config.definition], now)?;
            next_choice.released.insert(node.clone(), now);
            released.push(config.id);
        }
        graph.validate_history(&next_choice, &next_instances, now)?;
        *choice = next_choice;
        *instances = next_instances;
        Ok(released)
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
