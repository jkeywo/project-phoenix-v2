use super::*;
use crate::world::activities::*;
use phoenix_model::{
    activity::{ActivityJob, ActivityOutcome, RepairAchievement},
    damage::RepairWorkTarget,
    job::JobId,
    messages::{RepairTarget, SystemId},
};
use std::collections::BTreeMap;
const LIB: &str = "assets/worlds/domains/maintenance.toml";
#[path = "planning_temporal_tests.rs"]
mod temporal_tests;
fn literal(value: Value) -> Argument {
    Argument::Literal { value }
}
fn actor() -> Argument {
    Argument::Parameter {
        name: "actor".into(),
    }
}
fn parameters() -> Vec<Parameter> {
    vec![Parameter {
        name: "actor".into(),
        kind: ParameterKind::Actor,
    }]
}
fn room() -> RepairWorkTarget {
    RepairWorkTarget::Compartment {
        compartment: "engineering".into(),
    }
}
fn library(ordered: bool) -> DomainLibrary {
    let repair = |name: &str, team| Task::Primitive {
        forecast: None,
        id: name.into(),
        parameters: parameters(),
        definition: "repair".into(),
        request: RequestTemplate::Repair {
            actor: actor(),
            owner: literal(Value::System(SystemId("repair".into()))),
            team: literal(Value::Team(team)),
            request: literal(Value::RepairRequest(RepairTarget::Work(room()))),
            target: literal(Value::RepairTarget(room())),
            goal: literal(Value::RepairGoal(
                phoenix_model::activity::RepairGoal::Structure,
            )),
        },
    };
    let method = |id: &str, task: &str| Method {
        id: id.into(),
        steps: vec![
            TaskCall {
                id: "repair".into(),
                task: task.into(),
                arguments: BTreeMap::from([("actor".into(), actor())]),
            },
            TaskCall {
                id: "supply".into(),
                task: "deliver".into(),
                arguments: BTreeMap::from([("actor".into(), actor())]),
            },
        ],
        edges: if ordered {
            vec![Dependency {
                from: "repair".into(),
                to: "supply".into(),
                when: SituationCondition::Achieved,
            }]
        } else {
            vec![]
        },
    };
    DomainLibrary {
        domain: DomainHeader {
            format: DOMAIN_FORMAT,
            id: "maintenance".into(),
            exports: vec!["maintain".into()],
            requires: vec![
                OperatorFamily::RepairInternal,
                OperatorFamily::DeliveryLocal,
            ],
        },
        imports: vec![],
        tasks: vec![
            Task::Compound {
                id: "maintain".into(),
                parameters: parameters(),
                methods: vec![
                    method("two-team", "repair-second"),
                    method("one-team", "repair-first"),
                ],
            },
            repair("repair-second", 1),
            repair("repair-first", 0),
            Task::Primitive {
                forecast: None,
                id: "deliver".into(),
                parameters: parameters(),
                definition: "delivery".into(),
                request: RequestTemplate::Delivery {
                    actor: actor(),
                    owner: literal(Value::System(SystemId("logistics".into()))),
                    source: literal(Value::Store("hold".into())),
                    destination: literal(Value::Store("local".into())),
                    commodity: literal(Value::Commodity("parts".into())),
                    amount: literal(Value::Quantity(2)),
                },
            },
        ],
        definitions: vec![
            SituationDefinition {
                id: "repair".into(),
                initial: "done".into(),
                state: vec![SituationState {
                    id: "done".into(),
                    when: SituationCondition::Achieved,
                    call: None,
                    next: None,
                }],
            },
            SituationDefinition {
                id: "delivery".into(),
                initial: "done".into(),
                state: vec![SituationState {
                    id: "done".into(),
                    when: SituationCondition::Received,
                    call: None,
                    next: None,
                }],
            },
        ],
    }
}
fn config() -> PlanningConfig {
    PlanningConfig {
        required_commitment: None,
        temporal: None,
        id: "maintenance".into(),
        task: "care::maintain".into(),
        arguments: BTreeMap::from([("actor".into(), Value::Actor("crew".into()))]),
        imports: vec![DomainImport {
            path: LIB.into(),
            namespace: "care".into(),
            names: vec!["maintain".into()],
        }],
    }
}
fn compile(library: DomainLibrary) -> CompiledPlanning {
    CompiledPlanning::load(config(), |path| {
        (path == LIB).then(|| toml::to_string(&library).unwrap())
    })
    .unwrap()
    .0
}
fn choose(ordered: bool) -> (CompiledPlanning, PlanSelection) {
    let compiled = compile(library(ordered));
    let selected = compiled
        .choose("actual-crew", 10, |request| match request {
            ActivityRequest::Repair { team: 1, .. } => {
                Err("actual accepted vessel has only team0".into())
            }
            _ => Ok(()),
        })
        .unwrap();
    (compiled, selected)
}
#[test]
fn explicit_imported_private_closure_and_typed_capability_alternative_form_a_real_partial_order() {
    let (compiled, mut selected) = choose(true);
    assert_eq!(selected.choice.methods["root"].method, "one-team");
    assert_eq!(selected.refused_methods.len(), 1);
    assert_eq!(selected.choice.refused_methods, selected.refused_methods);
    assert!(selected.refused_methods[0].contains("only team0"));
    assert_eq!(selected.graph.leaves.len(), 2);
    assert_eq!(selected.graph.dependencies.len(), 1);
    let mut instances = SituationInstances::default();
    assert_eq!(
        compiled
            .release(&mut selected.choice, &mut instances, "actual-crew", 11)
            .unwrap(),
        vec!["htn/maintenance/root/repair"]
    );
    let source = instances.instances["htn/maintenance/root/repair"].clone();
    assert_eq!(source.request.actor(), "actual-crew");
    assert!(compiled
        .release(&mut selected.choice, &mut instances, "actual-crew", 12)
        .unwrap()
        .is_empty());
    assert_eq!(
        instances
            .accept_repair(
                "actual-crew",
                &SystemId("repair".into()),
                0,
                &RepairTarget::Work(room()),
                JobId(1),
                12
            )
            .unwrap(),
        Some("htn/maintenance/root/repair".into())
    );
    assert!(
        compiled
            .release(&mut selected.choice, &mut instances, "actual-crew", 13)
            .unwrap()
            .is_empty(),
        "acceptance is not achievement"
    );
    let scope = ActivityJob {
        actor: "actual-crew".into(),
        owner: SystemId("repair".into()),
        job: JobId(1),
    };
    let fact = instances
        .fact(
            &scope,
            14,
            ActivityOutcome::RepairAchieved {
                stage: 0,
                target: room(),
                remedy: "structural".into(),
                result: RepairAchievement::Structure { restored_hp: 1.0 },
            },
        )
        .unwrap();
    // A produced-but-unacknowledged fact cannot release dependent work.
    assert!(compiled
        .release(&mut selected.choice, &mut instances, "actual-crew", 14)
        .unwrap()
        .is_empty());
    instances.acknowledge(&fact, 14).unwrap();
    assert_eq!(
        compiled
            .release(&mut selected.choice, &mut instances, "actual-crew", 14)
            .unwrap(),
        vec!["htn/maintenance/root/supply"]
    );
    assert_eq!(selected.choice.released["root/supply"], 14);
    let (parallel, mut unordered) = choose(false);
    let mut concurrent = SituationInstances::default();
    assert_eq!(
        parallel
            .release(&mut unordered.choice, &mut concurrent, "actual-crew", 11)
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn release_and_history_are_atomic_and_json_replays_the_exact_method_without_reselection() {
    let (compiled, mut selected) = choose(false);
    let mut instances = SituationInstances::default();
    compiled
        .release(&mut selected.choice, &mut instances, "actual-crew", 11)
        .unwrap();
    let original = (selected.choice.clone(), instances.clone());
    let (choice, history): (PlanChoice, SituationInstances) =
        serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(compiled.derive(&choice).unwrap(), selected.graph);
    for now in 11..371 {
        compiled.validate_history(&choice, &history, now).unwrap();
        assert_eq!((&choice, &history), (&original.0, &original.1));
    }
    let mut forged = history.clone();
    forged.instances.remove("htn/maintenance/root/repair");
    assert!(
        compiled.validate_history(&choice, &forged, 12).is_err(),
        "removed actor cannot erase an already released situation"
    );
    let before = (selected.choice.clone(), instances.clone());
    assert!(compiled
        .release(&mut selected.choice, &mut instances, "replacement-crew", 12)
        .is_err());
    assert_eq!((selected.choice.clone(), instances.clone()), before);
    let mut forged = choice.clone();
    forged.methods.get_mut("root").unwrap().method = "not-authored".into();
    assert!(compiled.derive(&forged).is_err());
    let (ordered, mut selected) = choose(true);
    let mut instances = SituationInstances::default();
    ordered
        .release(&mut selected.choice, &mut instances, "actual-crew", 11)
        .unwrap();
    // A fabricated release record cannot substitute for the evidence edge.
    selected.choice.released.insert("root/supply".into(), 11);
    assert!(ordered
        .validate_history(&selected.choice, &instances, 12)
        .is_err());
}
#[test]
fn invalid_export_types_recursion_and_closed_nested_requests_refuse_before_selection() {
    assert!(serde_json::from_value::<Value>(
        serde_json::json!({"kind":"repair_request","value":{"type":"Core","ignored":true}})
    )
    .is_err());
    assert!(
        serde_json::from_value::<Value>(serde_json::json!({"kind":"team","value":256})).is_err()
    );
    for fault in 0..5 {
        let mut config = config();
        let mut library = library(true);
        match fault {
            0 => config.imports[0].names.push("repair-first".into()),
            1 => config.imports[0].path = "../outside.toml".into(),
            2 => config
                .arguments
                .insert("actor".into(), Value::Quantity(2))
                .map(|_| ())
                .unwrap(),
            3 => {
                if let Task::Compound { methods, .. } = &mut library.tasks[0] {
                    methods[0].steps[0].task = "maintain".into();
                }
            }
            _ => {
                if let Task::Compound { methods, .. } = &mut library.tasks[0] {
                    methods[0].steps[0].arguments.insert(
                        "actor".into(),
                        literal(Value::System(SystemId("repair".into()))),
                    );
                }
            }
        }
        assert!(
            CompiledPlanning::load(config, |path| (path == LIB)
                .then(|| toml::to_string(&library).unwrap()))
            .is_err(),
            "fault{fault}"
        );
    }
    let mut library = library(true);
    if let Task::Compound { methods, .. } = &mut library.tasks[0] {
        methods[1].edges.push(Dependency {
            from: "supply".into(),
            to: "repair".into(),
            when: SituationCondition::Received,
        });
    }
    let compiled = compile(library);
    assert!(
        compiled
            .choose("actual-crew", 0, |request| {
                if matches!(request, ActivityRequest::Repair { team: 1, .. }) {
                    Err("no second team".into())
                } else {
                    Ok(())
                }
            })
            .is_err(),
        "a supported primitive set does not make a cyclic method usable"
    );
}

#[test]
fn original_capability_explanations_are_bounded_and_survive_choice_json() {
    let compiled = compile(library(true));
    let selected = compiled
        .choose("actual-crew", 10, |request| match request {
            ActivityRequest::Repair { team: 1, .. } => Err(format!(
                "{}\nnot a current damage fact",
                "\u{e9}".repeat(900)
            )),
            _ => Ok(()),
        })
        .unwrap();
    assert_eq!(selected.choice.refused_methods.len(), 1);
    assert_eq!(
        selected.choice.refused_methods[0].chars().count(),
        MAX_EXPLANATION_CHARS
    );
    assert!(!selected.choice.refused_methods[0]
        .chars()
        .any(char::is_control));
    let decoded: PlanChoice =
        serde_json::from_slice(&serde_json::to_vec(&selected.choice).unwrap()).unwrap();
    assert_eq!(decoded, selected.choice);
    let mut forged = decoded.clone();
    forged
        .refused_methods
        .push("x".repeat(MAX_EXPLANATION_CHARS + 1));
    assert!(compiled.derive(&forged).is_err());
    assert!(compiled.derive(&decoded).is_ok());
}

#[test]
fn dependency_waits_for_actual_authored_callback_not_just_preceding_fact() {
    let mut source = library(true);
    source.definitions[0].state[0].call = Some(ScenarioCall {
        script: "assets/worlds/permission.toml#script.setup".into(),
        function: "grant".into(),
    });
    for task in &mut source.tasks {
        if let Task::Compound { methods, .. } = task {
            for method in methods {
                method.edges[0].when = SituationCondition::ScenarioApplied {
                    state: "done".into(),
                };
            }
        }
    }
    let compiled = compile(source.clone());
    let selected = compiled.choose("actual-crew", 10, |_| Ok(())).unwrap();
    let definitions = selected
        .graph
        .definitions
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let mut choice = selected.choice;
    let mut instances = SituationInstances::default();
    compiled
        .release(&mut choice, &mut instances, "actual-crew", 11)
        .unwrap();
    instances
        .accept_repair(
            "actual-crew",
            &SystemId("repair".into()),
            1,
            &RepairTarget::Work(room()),
            JobId(1),
            12,
        )
        .unwrap();
    let fact = instances
        .fact(
            &ActivityJob {
                actor: "actual-crew".into(),
                owner: SystemId("repair".into()),
                job: JobId(1),
            },
            14,
            ActivityOutcome::RepairAchieved {
                stage: 0,
                target: room(),
                remedy: "structure".into(),
                result: RepairAchievement::Structure { restored_hp: 1.0 },
            },
        )
        .unwrap();
    instances.acknowledge(&fact, 14).unwrap();
    assert!(
        compiled
            .release(&mut choice, &mut instances, "actual-crew", 15)
            .unwrap()
            .is_empty(),
        "achieved work is not an applied Scenario callback"
    );
    let activation = instances.ready(&definitions).remove(0);
    let mut refused = instances.clone();
    refused
        .finish_activation(&activation, ScenarioCallResult::Refused, 18, &definitions)
        .unwrap();
    let mut refused_choice = choice.clone();
    assert!(compiled
        .release(&mut refused_choice, &mut refused, "actual-crew", 19)
        .unwrap()
        .is_empty());
    instances
        .finish_activation(&activation, ScenarioCallResult::Applied, 20, &definitions)
        .unwrap();
    assert_eq!(
        compiled
            .release(&mut choice, &mut instances, "actual-crew", 21)
            .unwrap(),
        vec!["htn/maintenance/root/supply"]
    );
    let decoded: PlanChoice =
        serde_json::from_slice(&serde_json::to_vec(&choice).unwrap()).unwrap();
    for now in 21..381 {
        compiled
            .validate_history(&decoded, &instances, now)
            .unwrap();
    }
    let mut forged = decoded.clone();
    forged.released.insert("root/supply".into(), 19);
    assert!(
        compiled.validate_history(&forged, &instances, 21).is_err(),
        "a release cannot predate its actual permission"
    );
    source.definitions[0].state[0].call = None;
    assert!(
        compile(source)
            .choose("actual-crew", 10, |_| Ok(()))
            .is_err(),
        "dependency must name an explicit callback"
    );
}

#[test]
fn required_promise_gates_only_first_choice_and_retains_its_original_open_interval() {
    use crate::world::commitments::{CommitmentLedger, CommitmentOutcome};
    let mut root = config();
    root.required_commitment = Some("assistance".into());
    let compiled = CompiledPlanning::load(root, |path| {
        (path == LIB).then(|| toml::to_string(&library(true)).unwrap())
    })
    .unwrap()
    .0;
    let mut ledger = CommitmentLedger::default();
    assert!(!compiled.commitment_ready(&ledger, 10).unwrap());
    assert!(compiled.dormant_commitment_allowed(&ledger, 10).unwrap());
    ledger
        .record("assistance", "maintenance-team", "terms", "resolution", 10)
        .unwrap();
    assert!(compiled.commitment_ready(&ledger, 10).unwrap());
    assert!(
        compiled.dormant_commitment_allowed(&ledger, 10).unwrap(),
        "same-tick script acceptance awaits collection"
    );
    assert!(
        !compiled.dormant_commitment_allowed(&ledger, 11).unwrap(),
        "past open promise cannot erase the root"
    );
    let choice = compiled
        .choose("actual-crew", 11, |_| Ok(()))
        .unwrap()
        .choice;
    assert_eq!(choice.required_commitment.as_deref(), Some("assistance"));
    compiled
        .validate_required_commitment(&choice, &ledger, 11)
        .unwrap();
    let original = choice.clone();
    ledger
        .resolve("assistance", CommitmentOutcome::Kept, 20)
        .unwrap();
    assert!(!compiled.commitment_ready(&ledger, 20).unwrap());
    assert!(
        !compiled.dormant_commitment_allowed(&ledger, 20).unwrap(),
        "later resolution cannot launder lost accepted history"
    );
    for now in 20..380 {
        compiled
            .validate_required_commitment(&choice, &ledger, now)
            .unwrap();
    }
    assert_eq!(
        choice, original,
        "resolution has no physical cancellation or deadline effect"
    );
    for fault in 0..6 {
        let mut bad = ledger.clone();
        match fault {
            0 => bad.records.clear(),
            1 => {
                bad.records.push(bad.records[0].clone());
            }
            2 => bad.records[0].made_at_tick = 12,
            3 => bad.records[0].resolved_at_tick = Some(10),
            4 => bad.records[0].resolved_at_tick = None,
            _ => bad.records[0].resolved_at_tick = Some(31),
        }
        assert!(
            compiled
                .validate_required_commitment(&choice, &bad, 30)
                .is_err(),
            "malformed promise {fault}"
        );
    }
    let mut instantaneous = CommitmentLedger::default();
    instantaneous
        .record("assistance", "party", "terms", "resolution", 10)
        .unwrap();
    instantaneous
        .resolve("assistance", CommitmentOutcome::Broken, 10)
        .unwrap();
    assert!(compiled
        .dormant_commitment_allowed(&instantaneous, 30)
        .unwrap());
    let mut forged = choice;
    forged.required_commitment = None;
    assert!(compiled.derive(&forged).is_err());
}
