//! Bounded scratch maintenance search over real imported primitive operators.
//! This owner forecasts useful work; it never reserves resources or mints jobs.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceArgument {
    Actor,
    RepairOwner,
    HandlingOwner,
    Team,
    RepairRequest,
    RepairTarget,
    RepairGoal,
    SourceStore,
    DestinationStore,
    Commodity,
    Quantity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceOperator {
    pub task: String,
    pub arguments: BTreeMap<String, MaintenanceArgument>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceSearchConfig {
    pub id: String,
    pub actor: String,
    #[serde(rename = "import")]
    pub imports: Vec<DomainImport>,
    pub repair: MaintenanceOperator,
    pub delivery: MaintenanceOperator,
    pub repair_owner: phoenix_model::identity::SystemId,
    pub handling_owner: phoenix_model::identity::SystemId,
    pub team: u8,
    pub compartments: Vec<String>,
    pub source_store: String,
    pub destination_store: String,
    pub commodity: String,
    pub offer_script: String,
    pub offer_function: String,
    /// Explicit authored consequence for a restored, never-accepted final leaf.
    pub changed_work: crate::world::activities::ScenarioCall,
    pub terms: String,
    pub resolves_when: String,
}
/// Frozen observations from the actual owner's finite maintenance work view.
/// Historical validation checks their original shape and accepted recipe;
/// it never substitutes current damage or stock for this discovery evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceOpportunityInput {
    pub compartment: String,
    pub hp: f64,
    pub max_hp: f64,
    pub remedy_hp: f64,
    pub required_parts: i64,
    pub local_available: i64,
    pub local_headroom: i64,
    pub source_available_above_floor: i64,
    pub known: bool,
    pub report: Option<phoenix_model::observation::ObservationMetadata>,
    pub repair_idle: bool,
    pub handling_idle: bool,
    pub route_available: bool,
    pub outstanding_supply: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorInvocation {
    pub task: String,
    pub arguments: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceSearchResult {
    pub steps: Vec<OperatorInvocation>,
    pub expansions: u16,
}
#[derive(Clone, Debug)]
pub struct CompiledMaintenanceSearch {
    pub config: MaintenanceSearchConfig,
    pub catalog: DomainCatalog,
}
impl CompiledMaintenanceSearch {
    pub fn load(
        config: MaintenanceSearchConfig,
        read: impl FnMut(&str) -> Option<String>,
    ) -> Result<(Self, Vec<DomainSource>), String> {
        if !super::compile::identifier(&config.id)
            || config.compartments.is_empty()
            || config.compartments.len() > 16
            || config
                .compartments
                .iter()
                .any(|id| !super::compile::identifier(id))
            || config.compartments.iter().collect::<BTreeSet<_>>().len()
                != config.compartments.len()
            || !super::compile::identifier(&config.source_store)
            || !super::compile::identifier(&config.destination_store)
            || config.source_store == config.destination_store
            || !super::compile::identifier(&config.commodity)
            || !super::compile::identifier(&config.offer_function)
            || !super::compile::identifier(&config.changed_work.function)
            || [
                config.actor.as_str(),
                config.offer_script.as_str(),
                config.changed_work.script.as_str(),
                config.terms.as_str(),
                config.resolves_when.as_str(),
            ]
            .iter()
            .any(|value| {
                value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control)
            })
        {
            return Err("invalid bounded maintenance discovery source".into());
        }
        let (catalog, sources) = DomainCatalog::load(config.imports.clone(), read)?;
        let compiled = Self { config, catalog };
        // Every role and parameter is checked by the same actual primitive
        // expander. There is no synthetic compound/root configuration.
        for compartment in &compiled.config.compartments {
            for required_parts in [1, 2] {
                let sample = MaintenanceOpportunityInput {
                    compartment: compartment.clone(),
                    hp: 1.0,
                    max_hp: 2.0,
                    remedy_hp: 1.0,
                    required_parts,
                    local_available: 0,
                    local_headroom: 1,
                    source_available_above_floor: 1,
                    known: true,
                    report: Some(
                        phoenix_model::observation::ObservationMetadata::at(
                            0,
                            phoenix_model::observation::ObservationSubject::System {
                                id: compiled.config.repair_owner.clone(),
                            },
                            phoenix_model::observation::ObservationProvenance {
                                kind: phoenix_model::observation::ObservationSource::RepairTeam,
                                system: Some(compiled.config.repair_owner.clone()),
                                label: String::new(),
                            },
                            phoenix_model::observation::ObservationUncertainty::Qualitative,
                        )
                        .received(0),
                    ),
                    repair_idle: true,
                    handling_idle: true,
                    route_available: true,
                    outstanding_supply: false,
                };
                for (operator, expected) in [
                    (
                        &compiled.config.repair,
                        crate::world::activities::ActivityRequest::Repair {
                            actor: compiled.config.actor.clone(),
                            owner: compiled.config.repair_owner.clone(),
                            team: u32::from(compiled.config.team),
                            request: phoenix_model::messages::RepairTarget::Work(
                                phoenix_model::damage::RepairWorkTarget::Compartment {
                                    compartment: sample.compartment.clone(),
                                },
                            ),
                            target: phoenix_model::damage::RepairWorkTarget::Compartment {
                                compartment: sample.compartment.clone(),
                            },
                            goal: phoenix_model::activity::RepairGoal::Structure,
                        },
                    ),
                    (
                        &compiled.config.delivery,
                        crate::world::activities::ActivityRequest::Delivery {
                            actor: compiled.config.actor.clone(),
                            owner: compiled.config.handling_owner.clone(),
                            source: compiled.config.source_store.clone(),
                            destination: compiled.config.destination_store.clone(),
                            commodity: compiled.config.commodity.clone(),
                            amount: required_parts,
                        },
                    ),
                ] {
                    let step = compiled.invocation(operator, &sample)?;
                    let graph = compiled.catalog.expand_operator(
                        &step.task,
                        &step.arguments,
                        "source-check",
                        "operator",
                        &compiled.config.actor,
                    )?;
                    if graph.leaves["operator"].request != expected {
                        return Err(
                    "maintenance role bindings do not instantiate their declared actual operator"
                        .into(),
                );
                    }
                }
            }
        }
        Ok((compiled, sources))
    }
    fn invocation(
        &self,
        operator: &MaintenanceOperator,
        input: &MaintenanceOpportunityInput,
    ) -> Result<OperatorInvocation, String> {
        let target = phoenix_model::damage::RepairWorkTarget::Compartment {
            compartment: input.compartment.clone(),
        };
        let arguments = operator
            .arguments
            .iter()
            .map(|(name, role)| {
                use MaintenanceArgument as Role;
                let value = match role {
                    Role::Actor => Value::Actor(self.config.actor.clone()),
                    Role::RepairOwner => Value::System(self.config.repair_owner.clone()),
                    Role::HandlingOwner => Value::System(self.config.handling_owner.clone()),
                    Role::Team => Value::Team(self.config.team),
                    Role::RepairRequest => Value::RepairRequest(
                        phoenix_model::messages::RepairTarget::Work(target.clone()),
                    ),
                    Role::RepairTarget => Value::RepairTarget(target.clone()),
                    Role::RepairGoal => {
                        Value::RepairGoal(phoenix_model::activity::RepairGoal::Structure)
                    }
                    Role::SourceStore => Value::Store(self.config.source_store.clone()),
                    Role::DestinationStore => Value::Store(self.config.destination_store.clone()),
                    Role::Commodity => Value::Commodity(self.config.commodity.clone()),
                    Role::Quantity => Value::Quantity(
                        input.required_parts - input.local_available.min(input.required_parts),
                    ),
                };
                Ok((name.clone(), value))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        Ok(OperatorInvocation {
            task: operator.task.clone(),
            arguments,
        })
    }
    pub fn validate_input(&self, input: &MaintenanceOpportunityInput) -> Result<(), String> {
        if input.known != input.report.is_some()
            || input.report.as_ref().is_some_and(|report| {
                !report.is_valid()
                    || report
                        .observed_tick
                        .zip(report.received_tick)
                        .is_none_or(|(observed, received)| observed > received)
            })
            || !self.config.compartments.contains(&input.compartment)
            || [input.hp, input.max_hp, input.remedy_hp]
                .iter()
                .any(|value| !value.is_finite())
            || input.hp < 0.0
            || input.max_hp <= 0.0
            || input.hp > input.max_hp
            || input.remedy_hp <= 0.0
            || input.required_parts <= 0
            || [
                input.local_available,
                input.local_headroom,
                input.source_available_above_floor,
            ]
            .iter()
            .any(|value| *value < 0)
        {
            return Err("invalid original maintenance discovery evidence".into());
        }
        Ok(())
    }
    /// Breadth-first search with fixed order, cycle detection and explicit
    /// expansion/depth bounds. The successful path must improve actual HP.
    pub fn search(
        &self,
        input: &MaintenanceOpportunityInput,
    ) -> Result<Option<MaintenanceSearchResult>, String> {
        self.validate_input(input)?;
        if !input.known
            || !input.route_available
            || !input.repair_idle
            || input.outstanding_supply
            || input.hp >= input.max_hp
        {
            return Ok(None);
        }
        let initial = (
            input.hp.to_bits(),
            input.local_available,
            input.source_available_above_floor,
        );
        let mut queue = VecDeque::from([(initial, Vec::new())]);
        let mut seen = BTreeSet::from([initial]);
        let mut expansions = 0u16;
        while let Some(((hp, local, source), steps)) = queue.pop_front() {
            if expansions >= 256 {
                break;
            }
            expansions += 1;
            if f64::from_bits(hp) > input.hp {
                return Ok(Some(MaintenanceSearchResult { steps, expansions }));
            }
            if steps.len() >= 8 {
                continue;
            }
            // Fixed supported operator order; effects exist only in scratch.
            let mut successors = Vec::new();
            if local >= input.required_parts {
                let next_hp = (f64::from_bits(hp) + input.remedy_hp)
                    .min(input.max_hp)
                    .to_bits();
                successors.push((
                    (next_hp, local - input.required_parts, source),
                    self.invocation(&self.config.repair, input)?,
                ));
            }
            let missing = input.required_parts - local.min(input.required_parts);
            if missing > 0
                && input.handling_idle
                && input.route_available
                && missing <= source
                && missing <= input.local_headroom
            {
                successors.push((
                    (hp, local + missing, source - missing),
                    self.invocation(&self.config.delivery, input)?,
                ));
            }
            for (state, step) in successors {
                if seen.insert(state) {
                    let mut path = steps.clone();
                    path.push(step);
                    queue.push_back((state, path));
                }
            }
        }
        Ok(None)
    }
    pub fn derive(
        &self,
        reference: &phoenix_model::comms_request::OpportunityRef,
        input: &MaintenanceOpportunityInput,
        result: &MaintenanceSearchResult,
    ) -> Result<Decomposition, String> {
        reference.validate()?;
        if reference.source != self.config.id || self.search(input)?.as_ref() != Some(result) {
            return Err("optional work differs from its original bounded search".into());
        }
        let namespace = format!("htn-optional/{}/{}", reference.source, reference.ordinal);
        let mut graph = Decomposition::default();
        for (index, step) in result.steps.iter().enumerate() {
            let node = format!("step-{index}");
            let child = self.catalog.expand_operator(
                &step.task,
                &step.arguments,
                &namespace,
                &node,
                &self.config.actor,
            )?;
            if index > 0 {
                let previous = format!("step-{}", index - 1);
                let condition = match &graph.leaves[&previous].request {
                    crate::world::activities::ActivityRequest::Delivery { .. } => {
                        crate::world::activities::SituationCondition::Received
                    }
                    _ => return Err("unsupported generated maintenance predecessor".into()),
                };
                graph.dependencies.push(Dependency {
                    from: previous,
                    to: node.clone(),
                    when: condition,
                });
            }
            graph.leaves.extend(child.leaves);
            graph.definitions.extend(child.definitions);
            graph.forecasts.extend(child.forecasts);
        }
        Ok(graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const WORLD: &str = include_str!("../../../../tests/fixtures/optional-maintenance-world.toml");
    const DOMAIN: &str =
        include_str!("../../../../tests/fixtures/maintenance-opportunity-domain.toml");
    fn fixture() -> CompiledMaintenanceSearch {
        let config = crate::world::config::parse_world(WORLD)
            .unwrap()
            .opportunity_search
            .unwrap();
        CompiledMaintenanceSearch::load(config, |path| {
            (path == "tests/fixtures/maintenance-opportunity-domain.toml").then(|| DOMAIN.into())
        })
        .unwrap()
        .0
    }
    fn input() -> MaintenanceOpportunityInput {
        MaintenanceOpportunityInput {
            compartment: "engineering".into(),
            hp: 80.0,
            max_hp: 100.0,
            remedy_hp: 10.0,
            required_parts: 1,
            local_available: 0,
            local_headroom: 8,
            source_available_above_floor: 20,
            known: true,
            report: Some(
                phoenix_model::observation::ObservationMetadata::at(
                    0,
                    phoenix_model::observation::ObservationSubject::System {
                        id: phoenix_model::identity::SystemId("repair".into()),
                    },
                    phoenix_model::observation::ObservationProvenance {
                        kind: phoenix_model::observation::ObservationSource::RepairTeam,
                        system: Some(phoenix_model::identity::SystemId("repair".into())),
                        label: String::new(),
                    },
                    phoenix_model::observation::ObservationUncertainty::Qualitative,
                )
                .received(0),
            ),
            repair_idle: true,
            handling_idle: true,
            route_available: true,
            outstanding_supply: false,
        }
    }
    #[test]
    fn real_operator_search_selects_funded_delivery_then_repair_or_direct_repair_without_writes() {
        let source = fixture();
        let mut evidence = input();
        let original = evidence.clone();
        let selected = source.search(&evidence).unwrap().unwrap();
        assert_eq!(selected.steps.len(), 2);
        assert_eq!(selected.expansions, 3);
        assert!(selected.steps[0].task.ends_with("deliver-parts"));
        assert!(selected.steps[1].task.ends_with("restore-room"));
        assert_eq!(evidence, original);
        evidence.local_available = 1;
        let selected = source.search(&evidence).unwrap().unwrap();
        assert_eq!(selected.steps.len(), 1);
        assert_eq!(selected.expansions, 2);
        assert!(selected.steps[0].task.ends_with("restore-room"));
        for fault in 0..7 {
            let mut changed = input();
            match fault {
                0 => changed.outstanding_supply = true,
                1 => changed.repair_idle = false,
                2 => {
                    changed.known = false;
                    changed.report = None;
                }
                3 => changed.source_available_above_floor = 0,
                4 => changed.local_headroom = 0,
                5 => changed.route_available = false,
                _ => changed.handling_idle = false,
            };
            assert!(source.search(&changed).unwrap().is_none(),"unfunded, unknown, inaccessible or already owned work {fault} creates no candidate");
        }
    }
    #[test]
    fn generated_frontier_rejects_changed_source_ordinal_and_operator_history_before_mutation() {
        let source = fixture();
        let input = input();
        let result = source.search(&input).unwrap().unwrap();
        let reference = phoenix_model::comms_request::OpportunityRef {
            source: source.config.id.clone(),
            ordinal: 1,
            actor: "actual-crew".into(),
        };
        let graph = source.derive(&reference, &input, &result).unwrap();
        let mut choice = PlanChoice {
            id: source.config.id.clone(),
            execution: PlanExecution::Optional {
                source: source.config.id.clone(),
                ordinal: 1,
            },
            required_commitment: Some("opportunity.maintenance.1".into()),
            root_arguments: BTreeMap::new(),
            chosen_tick: 10,
            actor_uuid: reference.actor.clone(),
            released: BTreeMap::new(),
            methods: BTreeMap::new(),
            refused_methods: Vec::new(),
            temporal: None,
            temporal_reconsiderations: Vec::new(),
            node_dispositions: BTreeMap::new(),
        };
        let mut instances = crate::world::activities::SituationInstances::default();
        graph
            .release(&mut choice, &mut instances, &reference.actor, 10)
            .unwrap();
        assert_eq!(instances.instances.len(), 1);
        let before_intent = (choice.clone(), instances.clone());
        let orders = graph.intent_requests(&choice, &instances, 10).unwrap();
        let future = orders
            .iter()
            .find(|row| {
                matches!(
                    row.config.request,
                    crate::world::activities::ActivityRequest::Repair { .. }
                )
            })
            .unwrap();
        assert!(future.released_tick.is_none());
        assert_eq!(future.config.request.actor(), reference.actor);
        assert_eq!(future.prerequisites.len(), 1);
        assert_eq!(
            future.prerequisites[0].condition,
            phoenix_model::activity::PlannedPrerequisiteKind::Received
        );
        assert!(!future.prerequisites[0].satisfied);
        assert!(!instances.instances.contains_key(&future.config.id));
        assert_eq!(
            before_intent,
            (choice.clone(), instances.clone()),
            "intent creates no admission or reservation"
        );
        for fault in 0..4 {
            let mut forged = choice.clone();
            match fault {
                0 => {
                    forged.execution = PlanExecution::Optional {
                        source: source.config.id.clone(),
                        ordinal: 2,
                    }
                }
                1 => {
                    forged.execution = PlanExecution::Optional {
                        source: "foreign".into(),
                        ordinal: 1,
                    }
                }
                2 => {
                    forged
                        .root_arguments
                        .insert("actor".into(), Value::Actor("foreign".into()));
                }
                _ => forged.execution = PlanExecution::Primary,
            };
            let before = instances.clone();
            let before_choice = forged.clone();
            assert!(graph
                .release(&mut forged, &mut instances, &reference.actor, 11)
                .is_err());
            assert_eq!(instances, before);
            assert_eq!(forged, before_choice);
        }
        let mut forged = result.clone();
        forged.steps[0]
            .arguments
            .insert("amount".into(), Value::Quantity(2));
        assert!(source.derive(&reference, &input, &forged).is_err());
    }
}
