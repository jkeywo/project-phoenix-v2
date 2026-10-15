use super::*;
use crate::world::activities::{
    ActivityRequest, SituationCondition, SituationConfig, SituationDefinition, SituationInstances,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decomposition {
    /// Stable relative primitive node path -> ordinary situation config.
    pub leaves: BTreeMap<String, SituationConfig>,
    pub definitions: BTreeMap<String, SituationDefinition>,
    pub dependencies: Vec<Dependency>,
    pub forecasts: BTreeMap<String, super::temporal::ActivityForecast>,
}
#[derive(Clone, Debug)]
pub struct PlanSelection {
    pub choice: PlanChoice,
    pub graph: Decomposition,
    /// Immutable capability/source explanations, not live physical facts.
    pub refused_methods: Vec<String>,
}
fn evaluate(argument: &Argument, arguments: &BTreeMap<String, Value>) -> Result<Value, String> {
    match argument {
        Argument::Literal { value } => {
            super::compile::validate_value(value)?;
            Ok(value.clone())
        }
        Argument::Parameter { name } => arguments
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unbound parameter {name:?}")),
    }
}
fn instantiate(
    template: &RequestTemplate,
    args: &BTreeMap<String, Value>,
) -> Result<ActivityRequest, String> {
    macro_rules! get {
        ($term:expr,$kind:ident) => {
            match evaluate($term, args)? {
                Value::$kind(value) => value,
                _ => return Err("instantiated task argument type mismatch".into()),
            }
        };
    }
    Ok(match template {
        RequestTemplate::Communication {
            actor,
            owner,
            recipient,
            channel,
            request,
        } => ActivityRequest::Communication {
            actor: get!(actor, Actor),
            owner: get!(owner, System),
            recipient: get!(recipient, Actor),
            channel: channel.clone(),
            request: *request,
        },
        RequestTemplate::Repair {
            actor,
            owner,
            team,
            request,
            target,
            goal,
        } => ActivityRequest::Repair {
            actor: get!(actor, Actor),
            owner: get!(owner, System),
            team: u32::from(get!(team, Team)),
            request: get!(request, RepairRequest),
            target: get!(target, RepairTarget),
            goal: get!(goal, RepairGoal),
        },
        RequestTemplate::Delivery {
            actor,
            owner,
            source,
            destination,
            commodity,
            amount,
        } => ActivityRequest::Delivery {
            actor: get!(actor, Actor),
            owner: get!(owner, System),
            source: get!(source, Store),
            destination: get!(destination, Store),
            commodity: get!(commodity, Commodity),
            amount: get!(amount, Quantity),
        },
    })
}
fn root_actor(arguments: &BTreeMap<String, Value>) -> Result<&str, String> {
    let mut actors = arguments.values().filter_map(|value| match value {
        Value::Actor(actor) => Some(actor.as_str()),
        _ => None,
    });
    let actor = actors
        .next()
        .ok_or("first maintenance plan requires one actor argument")?;
    if actors.next().is_some() {
        return Err("first maintenance plan supports one actor argument".into());
    }
    Ok(actor)
}
fn compatible(condition: &SituationCondition, request: &ActivityRequest) -> bool {
    match condition {
        SituationCondition::ScenarioApplied { .. } => true,
        SituationCondition::CommunicationRead {}
        | SituationCondition::CommunicationReported {}
        | SituationCondition::CommunicationDeclined {} => {
            matches!(request, ActivityRequest::Communication { .. })
        }
        SituationCondition::Accepted => true,
        SituationCondition::Achieved | SituationCondition::Settled { .. } => {
            matches!(request, ActivityRequest::Repair { .. })
        }
        SituationCondition::Received | SituationCondition::Returned => {
            matches!(request, ActivityRequest::Delivery { .. })
        }
    }
}
fn path(value: &str) -> bool {
    value.len() <= 1024 && value.split('/').all(super::compile::identifier)
}

impl CompiledPlanning {
    pub fn actor_reference(&self) -> Result<&str, String> {
        root_actor(&self.config.arguments)
    }

    pub fn choose(
        &self,
        actor_uuid: &str,
        tick: u64,
        supports: impl FnMut(&ActivityRequest) -> Result<(), String>,
    ) -> Result<PlanSelection, String> {
        self.choose_as(actor_uuid, tick, PlanExecution::Primary, supports)
    }
    fn choose_as(
        &self,
        actor_uuid: &str,
        tick: u64,
        execution: PlanExecution,
        mut supports: impl FnMut(&ActivityRequest) -> Result<(), String>,
    ) -> Result<PlanSelection, String> {
        let required_commitment = self.commitment_for_execution(&execution)?;
        if actor_uuid.trim().is_empty()
            || actor_uuid.len() > 256
            || actor_uuid.chars().any(char::is_control)
        {
            return Err("plan requires its actual accepted actor UUID".into());
        }
        let mut methods = BTreeMap::new();
        let mut refused_methods = Vec::new();
        let mut budget = MAX_EXPANDED_NODES;
        let root = self.resolve(None, &self.config.task)?;
        let graph = self.expand(
            &root,
            &self.config.arguments,
            "root",
            &execution,
            &mut methods,
            None,
            &mut supports,
            &mut refused_methods,
            &mut budget,
        )?;
        self.validate_graph(&graph, tick)?;
        Ok(PlanSelection {
            choice: PlanChoice {
                id: self.config.id.clone(),
                required_commitment,
                execution,
                root_arguments: self.config.arguments.clone(),
                chosen_tick: tick,
                actor_uuid: actor_uuid.into(),
                released: BTreeMap::new(),
                methods,
                refused_methods: refused_methods.clone(),
                temporal: None,
                temporal_reconsiderations: Vec::new(),
            },
            graph,
            refused_methods,
        })
    }
    /// Reconstruct the exact saved alternative, never choose again from current
    /// damage, resources, equipment or command availability.
    pub fn derive(&self, choice: &PlanChoice) -> Result<Decomposition, String> {
        if !choice.execution.is_primary()
            || choice.id != self.config.id
            || choice.required_commitment != self.commitment_for_execution(&choice.execution)?
            || choice.root_arguments != self.config.arguments
            || choice.actor_uuid.trim().is_empty()
            || choice.actor_uuid.len() > 256
            || choice.actor_uuid.chars().any(char::is_control)
            || choice.refused_methods.len() > MAX_EXPANDED_NODES
            || choice.refused_methods.iter().any(|reason| {
                reason.is_empty()
                    || reason.chars().count() > MAX_EXPLANATION_CHARS
                    || reason.chars().any(char::is_control)
            })
        {
            return Err("saved plan root/actor mismatch".into());
        }
        let root = self.resolve(None, &self.config.task)?;
        let mut methods = BTreeMap::new();
        let mut budget = MAX_EXPANDED_NODES;
        let graph = self.expand(
            &root,
            &choice.root_arguments,
            "root",
            &choice.execution,
            &mut methods,
            Some(&choice.methods),
            &mut |_| Ok(()),
            &mut Vec::new(),
            &mut budget,
        )?;
        if methods != choice.methods {
            return Err("saved plan contains missing or surplus method choices".into());
        }
        self.validate_graph(&graph, choice.chosen_tick)?;
        Ok(graph)
    }
    #[allow(clippy::too_many_arguments)]
    fn expand(
        &self,
        task_id: &TaskIdentity,
        args: &BTreeMap<String, Value>,
        node: &str,
        execution: &PlanExecution,
        choices: &mut BTreeMap<String, MethodChoice>,
        saved: Option<&BTreeMap<String, MethodChoice>>,
        supports: &mut impl FnMut(&ActivityRequest) -> Result<(), String>,
        refused: &mut Vec<String>,
        budget: &mut usize,
    ) -> Result<Decomposition, String> {
        if *budget == 0 || !path(node) {
            return Err("HTN expansion bound exhausted".into());
        }
        *budget -= 1;
        let task = self.task(task_id);
        match task {
            Task::Primitive { .. } => {
                let graph = self.catalog.expand_resolved_primitive(
                    task_id,
                    args,
                    &execution.namespace(&self.config.id),
                    node,
                    root_actor(&self.config.arguments)?,
                )?;
                for config in graph.leaves.values() {
                    supports(&config.request)?;
                }
                Ok(graph)
            }

            Task::Compound { methods, .. } => {
                let selected = saved
                    .map(|saved| saved.get(node).ok_or("missing saved compound method"))
                    .transpose()?;
                if selected.is_some_and(|row| &row.task != task_id) {
                    return Err("saved compound task identity mismatch".into());
                }
                let mut last_error = None;
                for method in methods {
                    if selected.is_some_and(|row| row.method != method.id) {
                        continue;
                    }
                    let mut candidate_choices = choices.clone();
                    candidate_choices.insert(
                        node.into(),
                        MethodChoice {
                            task: task_id.clone(),
                            method: method.id.clone(),
                        },
                    );
                    let mut graph = Decomposition::default();
                    let mut error = None;
                    for step in &method.steps {
                        let arguments = step
                            .arguments
                            .iter()
                            .map(|(name, value)| Ok((name.clone(), evaluate(value, args)?)))
                            .collect::<Result<BTreeMap<_, _>, String>>()?;
                        let id = self.resolve(Some(&task_id.library_path), &step.task)?;
                        match self.expand(
                            &id,
                            &arguments,
                            &format!("{node}/{}", step.id),
                            execution,
                            &mut candidate_choices,
                            saved,
                            supports,
                            refused,
                            budget,
                        ) {
                            Ok(child) => {
                                graph.leaves.extend(child.leaves);
                                graph.definitions.extend(child.definitions);
                                graph.dependencies.extend(child.dependencies);
                                graph.forecasts.extend(child.forecasts);
                                if graph.leaves.len() > MAX_LEAVES {
                                    error = Some("HTN physical leaf bound exhausted".into());
                                    break;
                                }
                            }
                            Err(reason) => {
                                error = Some(reason);
                                break;
                            }
                        }
                    }
                    if error.is_none() {
                        for edge in &method.edges {
                            graph.dependencies.push(Dependency {
                                from: format!("{node}/{}", edge.from),
                                to: format!("{node}/{}", edge.to),
                                when: edge.when.clone(),
                            });
                        }
                        error = self.validate_graph(&graph, 0).err();
                    }
                    if let Some(error) = error {
                        if refused.len() < MAX_EXPANDED_NODES {
                            refused.push(
                                format!(
                                    "{node} {}::{} method {}: {}",
                                    task_id.library_path,
                                    task_id.task,
                                    method.id,
                                    error
                                        .chars()
                                        .filter(|ch| !ch.is_control())
                                        .take(MAX_EXPLANATION_CHARS)
                                        .collect::<String>()
                                )
                                .chars()
                                .filter(|ch| !ch.is_control())
                                .take(MAX_EXPLANATION_CHARS)
                                .collect(),
                            );
                        }
                        if saved.is_some() {
                            return Err(error);
                        }
                        last_error = Some(error);
                        continue;
                    }
                    *choices = candidate_choices;
                    return Ok(graph);
                }
                Err(format!(
                    "no compatible method for {node} {}::{}: {}",
                    task_id.library_path,
                    task_id.task,
                    last_error.as_deref().unwrap_or("unknown saved method")
                ))
            }
        }
    }
    fn validate_graph(&self, graph: &Decomposition, tick: u64) -> Result<(), String> {
        if self.config.temporal.is_some() && graph.forecasts.len() != graph.leaves.len() {
            return Err(
                "temporal planning requires forecasts on every selected primitive leaf".into(),
            );
        }
        if graph.leaves.is_empty()
            || graph.leaves.len() > MAX_LEAVES
            || graph.dependencies.len() > MAX_EXPANDED_NODES
        {
            return Err("invalid expanded HTN graph bounds".into());
        }
        // Ephemeral validation only: this uses the SAME disjoint binding and
        // Scenario checks, and creates no live job or physical reservation.
        let mut candidate = SituationInstances::default();
        for config in graph.leaves.values() {
            candidate.arm(config, &graph.definitions[&config.definition], tick)?;
        }
        for (index, edge) in graph.dependencies.iter().enumerate() {
            if edge.from == edge.to
                || !graph.leaves.contains_key(&edge.from)
                || !graph.leaves.contains_key(&edge.to)
                || graph.dependencies[..index].contains(edge)
                || !compatible(&edge.when, &graph.leaves[&edge.from].request)
                || matches!(&edge.when, SituationCondition::ScenarioApplied { state }
                    if !graph.definitions[&graph.leaves[&edge.from].definition].state.iter()
                        .any(|row| &row.id == state && row.call.is_some()))
            {
                return Err("invalid HTN primitive dependency reference/outcome".into());
            }
        }
        let mut remaining: BTreeSet<_> = graph.leaves.keys().cloned().collect();
        while !remaining.is_empty() {
            let ready: Vec<_> = remaining
                .iter()
                .filter(|node| {
                    !graph
                        .dependencies
                        .iter()
                        .any(|edge| &edge.to == *node && remaining.contains(&edge.from))
                })
                .cloned()
                .collect();
            if ready.is_empty() {
                return Err("cyclic HTN dependencies".into());
            }
            for node in ready {
                remaining.remove(&node);
            }
        }
        Ok(())
    }
}

impl DomainCatalog {
    /// Expand a real explicitly imported primitive through the same typed
    /// template and Scenario validation used by authored HTN leaves.
    pub fn expand_operator(
        &self,
        reference: &str,
        arguments: &BTreeMap<String, Value>,
        namespace: &str,
        node: &str,
        actor_reference: &str,
    ) -> Result<Decomposition, String> {
        let task = self.resolve(None, reference)?;
        self.expand_resolved_primitive(&task, arguments, namespace, node, actor_reference)
    }
    fn expand_resolved_primitive(
        &self,
        task_id: &TaskIdentity,
        args: &BTreeMap<String, Value>,
        namespace: &str,
        node: &str,
        actor_reference: &str,
    ) -> Result<Decomposition, String> {
        if !path(namespace) || !path(node) {
            return Err("invalid generated operator namespace".into());
        }
        let task = self.task(task_id);
        if args.len() != task.parameters().len()
            || task.parameters().iter().any(|parameter| {
                args.get(&parameter.name).is_none_or(|value| {
                    value.kind() != parameter.kind || super::compile::validate_value(value).is_err()
                })
            })
        {
            return Err("generated operator parameter set/type mismatch".into());
        }
        let Task::Primitive {
            definition,
            request,
            forecast,
            ..
        } = task
        else {
            return Err("generated operator must name an exported primitive".into());
        };
        let request = instantiate(request, args)?;
        if request.actor() != actor_reference {
            return Err("first maintenance leaves must bind the root actor exactly".into());
        }
        let mut definition = self.libraries[&task_id.library_path]
            .definitions
            .iter()
            .find(|row| &row.id == definition)
            .expect("compiled Scenario definition")
            .clone();
        if definition
            .state
            .iter()
            .any(|state| !compatible(&state.when, &request))
        {
            return Err(
                "primitive Scenario condition does not match its physical outcome family".into(),
            );
        }
        definition.id = format!(
            "{namespace}/{}/{definition_id}",
            task_id.library_path,
            definition_id = definition.id
        );
        let config = SituationConfig {
            id: format!("{namespace}/{node}"),
            definition: definition.id.clone(),
            request,
        };
        Ok(Decomposition {
            leaves: BTreeMap::from([(node.into(), config)]),
            definitions: BTreeMap::from([(definition.id.clone(), definition)]),
            dependencies: Vec::new(),
            forecasts: forecast
                .as_ref()
                .map(|value| BTreeMap::from([(node.into(), value.clone())]))
                .unwrap_or_default(),
        })
    }
}
