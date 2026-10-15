use super::*;

#[test]
fn achievement_cannot_follow_terminal_settlement_in_either_delivery_order() {
    let (mut first, definitions) = prepare();
    let mut work = achieved(&first);
    work.tick = 8;
    let settled = first
        .fact(
            &work.scope,
            5,
            ActivityOutcome::RepairSettled {
                outcome: JobOutcome::Cancelled,
            },
        )
        .unwrap();
    let mut second = first.clone();
    first.acknowledge(&settled, 8).unwrap();
    let before = first.clone();
    assert!(first.acknowledge(&work, 8).is_err());
    assert_eq!(first, before);
    second.acknowledge(&work, 8).unwrap();
    let before = second.clone();
    assert!(second.acknowledge(&settled, 8).is_err());
    assert_eq!(second, before);
    first.instances.get_mut("repair-one").unwrap().achieved = Some(work.clone());
    assert!(first.validate(&definitions, 8).is_err());
    work.tick = 5;
    second.instances.get_mut("repair-one").unwrap().achieved = Some(work);
    second.acknowledge(&settled, 8).unwrap();
    second.validate(&definitions, 8).unwrap();
}

fn definition(id: &str, conditions: Vec<SituationCondition>) -> SituationDefinition {
    SituationDefinition {
        id: id.into(),
        initial: "state-0".into(),
        state: conditions
            .iter()
            .enumerate()
            .map(|(i, when)| SituationState {
                id: format!("state-{i}"),
                when: when.clone(),
                call: Some(ScenarioCall {
                    script: "activities.rhai".into(),
                    function: format!("effect_{i}"),
                }),
                next: (i + 1 < conditions.len()).then(|| format!("state-{}", i + 1)),
            })
            .collect(),
    }
}
fn repair() -> ActivityRequest {
    ActivityRequest::Repair {
        actor: "ship".into(),
        owner: SystemId("repair".into()),
        team: 0,
        request: phoenix_model::messages::RepairTarget::Work(RepairWorkTarget::Compartment {
            compartment: "engine-room".into(),
        }),
        target: RepairWorkTarget::Compartment {
            compartment: "engine-room".into(),
        },
        goal: RepairGoal::Structure,
    }
}
fn delivery() -> ActivityRequest {
    ActivityRequest::Delivery {
        actor: "ship".into(),
        owner: SystemId("handling".into()),
        source: "main".into(),
        destination: "local".into(),
        commodity: "parts".into(),
        amount: 4,
    }
}
fn prepare() -> (SituationInstances, Vec<SituationDefinition>) {
    let definitions = vec![
        definition(
            "restore",
            vec![
                SituationCondition::Achieved,
                SituationCondition::Settled {
                    outcome: JobOutcome::Cancelled,
                },
            ],
        ),
        definition("supply", vec![SituationCondition::Received]),
    ];
    let mut instances = SituationInstances::default();
    instances
        .arm(
            &SituationConfig {
                id: "repair-one".into(),
                definition: "restore".into(),
                request: repair(),
            },
            &definitions[0],
            2,
        )
        .unwrap();
    instances
        .arm(
            &SituationConfig {
                id: "delivery-one".into(),
                definition: "supply".into(),
                request: delivery(),
            },
            &definitions[1],
            2,
        )
        .unwrap();
    let ActivityRequest::Repair { target, .. } = repair() else {
        unreachable!()
    };
    assert_eq!(
        instances
            .accept_repair(
                "ship",
                &SystemId("repair".into()),
                0,
                &phoenix_model::messages::RepairTarget::Work(target),
                JobId(1),
                3
            )
            .unwrap()
            .as_deref(),
        Some("repair-one")
    );
    assert_eq!(
        instances
            .accept_delivery(
                "ship",
                &SystemId("handling".into()),
                "main",
                "local",
                "parts",
                4,
                JobId(1),
                3
            )
            .unwrap()
            .as_deref(),
        Some("delivery-one")
    );
    (instances, definitions)
}
fn achieved(instances: &SituationInstances) -> ActivityFact {
    let ActivityRequest::Repair { target, .. } = repair() else {
        unreachable!()
    };
    instances
        .fact(
            &ActivityJob {
                actor: "ship".into(),
                owner: SystemId("repair".into()),
                job: JobId(1),
            },
            5,
            ActivityOutcome::RepairAchieved {
                stage: 0,
                target,
                remedy: "structure".into(),
                result: RepairAchievement::Structure { restored_hp: 5.0 },
            },
        )
        .unwrap()
}

#[test]
fn two_explicit_instances_do_not_cross_credit_same_job_numbers_or_replay_effects() {
    let (mut instances, definitions) = prepare();
    let fact = achieved(&instances);
    instances.acknowledge(&fact, 5).unwrap();
    instances.acknowledge(&fact, 5).unwrap();
    let ready = instances.ready(&definitions);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].instance, "repair-one");
    instances
        .finish_activation(&ready[0], ScenarioCallResult::Applied, 5, &definitions)
        .unwrap();
    assert!(instances
        .finish_activation(&ready[0], ScenarioCallResult::Applied, 5, &definitions)
        .is_err());
    assert!(
        instances.ready(&definitions).is_empty(),
        "achieved work does not settle team return"
    );
    let settled = instances
        .fact(
            &fact.scope,
            8,
            ActivityOutcome::RepairSettled {
                outcome: JobOutcome::Cancelled,
            },
        )
        .unwrap();
    instances.acknowledge(&settled, 8).unwrap();
    assert!(instances.instances["repair-one"].achieved.is_some());
    let received = instances
        .fact(
            &ActivityJob {
                actor: "ship".into(),
                owner: SystemId("handling".into()),
                job: JobId(1),
            },
            8,
            ActivityOutcome::Delivery {
                source: "main".into(),
                destination: "local".into(),
                commodity: "parts".into(),
                amount: 4,
                returned: false,
                outcome: JobOutcome::Completed,
            },
        )
        .unwrap();
    instances.acknowledge(&received, 8).unwrap();
    let ready = instances.ready(&definitions);
    assert_eq!(ready.len(), 2);
    for activation in ready {
        instances
            .finish_activation(&activation, ScenarioCallResult::Applied, 8, &definitions)
            .unwrap();
    }
    instances.validate(&definitions, 8).unwrap();
    let saved = serde_json::to_vec(&instances).unwrap();
    let resumed: SituationInstances = serde_json::from_slice(&saved).unwrap();
    resumed.validate(&definitions, 8).unwrap();
    assert!(resumed.ready(&definitions).is_empty());
    assert_eq!(instances, resumed);
}

#[test]
fn return_cannot_satisfy_receive_and_refused_script_activation_is_not_reentered() {
    let (mut instances, definitions) = prepare();
    let returned = instances
        .fact(
            &ActivityJob {
                actor: "ship".into(),
                owner: SystemId("handling".into()),
                job: JobId(1),
            },
            5,
            ActivityOutcome::Delivery {
                source: "main".into(),
                destination: "main".into(),
                commodity: "parts".into(),
                amount: 4,
                returned: true,
                outcome: JobOutcome::Cancelled,
            },
        )
        .unwrap();
    instances.acknowledge(&returned, 5).unwrap();
    assert!(instances.ready(&definitions).is_empty());
    let fact = achieved(&instances);
    instances.acknowledge(&fact, 5).unwrap();
    let activation = instances.ready(&definitions).remove(0);
    instances
        .finish_activation(&activation, ScenarioCallResult::Refused, 5, &definitions)
        .unwrap();
    assert!(instances.ready(&definitions).is_empty());
    let resumed: SituationInstances =
        serde_json::from_slice(&serde_json::to_vec(&instances).unwrap()).unwrap();
    resumed.validate(&definitions, 5).unwrap();
    assert!(resumed.ready(&definitions).is_empty());
}

#[test]
fn malformed_binding_evidence_and_activation_frontiers_refuse_without_mutation() {
    let (instances, definitions) = prepare();
    let fact = achieved(&instances);
    for variant in 0..5 {
        let mut invalid = instances.clone();
        let row = invalid.instances.get_mut("repair-one").unwrap();
        match variant {
            0 => row.accepted_tick = Some(9),
            1 => {
                let mut forged = fact.clone();
                forged.scope.owner = SystemId("handling".into());
                row.achieved = Some(forged);
            }
            2 => {
                row.activations
                    .insert("state-0".into(), ScenarioCallResult::Applied);
                row.state = Some("state-1".into());
            }
            3 => row.state = None,
            _ => {
                row.achieved = Some(ActivityFact {
                    tick: 9,
                    ..fact.clone()
                })
            }
        }
        assert!(
            invalid.validate(&definitions, 8).is_err(),
            "variant {variant}"
        );
    }
    let mut live = instances.clone();
    let before = live.clone();
    assert!(live
        .arm(
            &SituationConfig {
                id: "overlap".into(),
                definition: "restore".into(),
                request: repair()
            },
            &definitions[0],
            3
        )
        .is_err());
    assert_eq!(live, before);
    assert!(live
        .acknowledge(
            &ActivityFact {
                instance: "delivery-one".into(),
                ..fact
            },
            5
        )
        .is_err());
    assert_eq!(live, before);
}

#[test]
fn accepted_world_source_can_reuse_a_definition_but_rejects_overlapping_or_invalid_requests() {
    #[derive(serde::Serialize)]
    struct Source {
        entity: Vec<crate::world::config::WorldEntity>,
        situation_definition: Vec<SituationDefinition>,
        situation: Vec<SituationConfig>,
    }
    let definition = definition("supply", vec![SituationCondition::Received]);
    let first = SituationConfig {
        id: "a".into(),
        definition: "supply".into(),
        request: delivery(),
    };
    let mut second = first.clone();
    second.id = "b".into();
    if let ActivityRequest::Delivery { actor, .. } = &mut second.request {
        *actor = "other-ship".into();
    }
    let mut source = Source {
        entity: ["ship", "other-ship"]
            .into_iter()
            .map(|name| crate::world::config::WorldEntity {
                name: Some(name.into()),
                template_path: "assets/entities/alliance_cruiser.toml".into(),
                ..Default::default()
            })
            .collect(),
        situation_definition: vec![definition],
        situation: vec![first, second],
    };
    let accepted = crate::world::config::parse_world(&toml::to_string(&source).unwrap()).unwrap();
    assert_eq!(accepted.situations.len(), 2);
    assert_eq!(accepted.situation_definitions.len(), 1);
    let mut unsupported = Source {
        entity: source.entity.clone(),
        situation_definition: source.situation_definition.clone(),
        situation: source.situation.clone(),
    };
    if let ActivityRequest::Delivery { actor, .. } = &mut unsupported.situation[0].request {
        *actor = "dynamic-later".into();
    }
    let error =
        crate::world::config::parse_world(&toml::to_string(&unsupported).unwrap()).unwrap_err();
    assert!(error.contains("dynamic/layer actor lifecycle"));
    source.situation[1].request = source.situation[0].request.clone();
    assert!(crate::world::config::parse_world(&toml::to_string(&source).unwrap()).is_err());
    source.situation[1].request = repair();
    source.situation[1].definition = "absent".into();
    assert!(crate::world::config::parse_world(&toml::to_string(&source).unwrap()).is_err());
    source.situation.pop();
    source.situation[0].request = repair();
    if let ActivityRequest::Repair { request, .. } = &mut source.situation[0].request {
        *request = phoenix_model::messages::RepairTarget::External;
    }
    assert!(crate::world::config::parse_world(&toml::to_string(&source).unwrap()).is_err());
}

#[test]
fn actual_callback_receipt_has_its_own_clock_and_refusal_never_grants_permission() {
    let condition = SituationCondition::ScenarioApplied {
        state: "state-0".into(),
    };
    let (mut instances, mut definitions) = prepare();
    definitions[0] = definition(
        "restore",
        vec![SituationCondition::Achieved, condition.clone()],
    );
    let fact = achieved(&instances);
    instances.acknowledge(&fact, 5).unwrap();
    let ready = instances.ready(&definitions).remove(0);
    assert_eq!(
        condition.satisfied_tick(&instances.instances["repair-one"]),
        None
    );
    let before = instances.clone();
    assert!(instances
        .finish_activation(&ready, ScenarioCallResult::Applied, 4, &definitions)
        .is_err());
    assert_eq!(
        instances, before,
        "backdated application does not mutate the frontier"
    );
    let mut refused = instances.clone();
    refused
        .finish_activation(&ready, ScenarioCallResult::Refused, 7, &definitions)
        .unwrap();
    assert_eq!(
        condition.satisfied_tick(&refused.instances["repair-one"]),
        None
    );
    refused.validate(&definitions, 7).unwrap();
    instances
        .finish_activation(&ready, ScenarioCallResult::Applied, 9, &definitions)
        .unwrap();
    assert_eq!(
        condition.satisfied_tick(&instances.instances["repair-one"]),
        Some(9)
    );
    let next = instances.ready(&definitions).remove(0);
    let before = instances.clone();
    assert!(instances
        .finish_activation(&next, ScenarioCallResult::Applied, 8, &definitions)
        .is_err());
    assert_eq!(instances, before);
    instances
        .finish_activation(&next, ScenarioCallResult::Applied, 10, &definitions)
        .unwrap();
    let decoded: SituationInstances =
        serde_json::from_slice(&serde_json::to_vec(&instances).unwrap()).unwrap();
    for tick in 10..370 {
        decoded.validate(&definitions, tick).unwrap();
    }
    for variant in 0..5 {
        let mut forged = decoded.clone();
        let row = forged.instances.get_mut("repair-one").unwrap();
        match variant {
            0 => {
                row.activation_ticks.remove("state-0");
            }
            1 => {
                row.activation_ticks.insert("extra".into(), 9);
            }
            2 => {
                row.activation_ticks.insert("state-0".into(), 4);
            }
            3 => {
                row.activation_ticks.insert("state-1".into(), 8);
            }
            _ => {
                row.activation_ticks.insert("state-1".into(), 11);
            }
        }
        assert!(
            forged.validate(&definitions, 10).is_err(),
            "forged application variant {variant}"
        );
    }
    let mut invalid = definitions[0].clone();
    invalid.state[0].when = condition;
    assert!(
        invalid.validate().is_err(),
        "a callback cannot grant itself permission"
    );
    let mut invalid = definitions[0].clone();
    invalid.state[0].call = None;
    assert!(
        invalid.validate().is_err(),
        "permission references an explicit authored callback"
    );
}

#[test]
fn retained_assignment_can_repeat_only_strictly_after_actual_terminal_settlement() {
    let (mut instances, definitions) = prepare();
    let original = instances.instances["repair-one"].clone();
    let config = SituationConfig {
        id: "repair-two".into(),
        definition: original.definition.clone(),
        request: original.request.clone(),
    };
    let definition = definitions
        .iter()
        .find(|row| row.id == config.definition)
        .unwrap();
    let work = achieved(&instances);
    instances.acknowledge(&work, work.tick).unwrap();
    assert!(
        instances.arm(&config, definition, 10).is_err(),
        "achievement does not release the returning team"
    );
    let terminal = instances
        .fact(
            original.accepted.as_ref().unwrap(),
            10,
            ActivityOutcome::RepairSettled {
                outcome: JobOutcome::Cancelled,
            },
        )
        .unwrap();
    instances.acknowledge(&terminal, 10).unwrap();
    assert!(
        instances.arm(&config, definition, 10).is_err(),
        "same-tick repeat overlaps terminal settlement"
    );
    instances.arm(&config, definition, 11).unwrap();
    instances.validate(&definitions, 11).unwrap();
    let mut bad = instances.clone();
    bad.instances.get_mut("repair-two").unwrap().armed_tick = 10;
    assert!(
        bad.validate(&definitions, 11).is_err(),
        "whole-history validation enforces the same strict frontier"
    );
    assert!(
        instances
            .accept_repair(
                "ship",
                &SystemId("repair".into()),
                0,
                &phoenix_model::messages::RepairTarget::Work(RepairWorkTarget::Compartment {
                    compartment: "engine-room".into()
                }),
                original.accepted.as_ref().unwrap().physical_job().unwrap(),
                11
            )
            .is_err(),
        "retained JobId never becomes a new execution identity"
    );
}
