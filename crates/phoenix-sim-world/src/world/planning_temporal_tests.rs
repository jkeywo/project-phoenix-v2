//! Pure forecasting and real situation-evidence seams; not a host/workload trial.
use super::*;
use crate::world::planning::temporal::*;
use phoenix_model::identity::StationId;

fn forecast() -> ActivityForecast {
    ActivityForecast {
        min_secs: 6,
        max_secs: 10,
        recovery_secs: 2,
        phases: vec![
            AttentionPhase {
                kind: AttentionKind::Active,
                offset_secs: 0,
                duration_secs: 4,
                attention_permille: 1000,
            },
            AttentionPhase {
                kind: AttentionKind::Monitor,
                offset_secs: 4,
                duration_secs: 6,
                attention_permille: 250,
            },
        ],
    }
}
fn timed(ordered: bool, deadline: u32) -> (CompiledPlanning, PlanSelection) {
    let mut config = config();
    config.temporal = Some(TemporalPolicy {
        profiles: vec![
            DifficultyProfile {
                min_operators: 0,
                deadline_secs: deadline,
            },
            DifficultyProfile {
                min_operators: 2,
                deadline_secs: 12,
            },
        ],
    });
    let mut library = library(ordered);
    for task in &mut library.tasks {
        if let Task::Primitive {
            forecast: value, ..
        } = task
        {
            *value = Some(forecast());
        }
    }
    let compiled = CompiledPlanning::load(config, |path| {
        (path == LIB).then(|| toml::to_string(&library).unwrap())
    })
    .unwrap()
    .0;
    let selected = compiled.choose("actual-crew", 100, |_| Ok(())).unwrap();
    (compiled, selected)
}
fn inputs(shared: bool) -> ForecastInput {
    ForecastInput {
        tick_nanos: 100_000_000,
        owners: BTreeMap::from([
            (
                SystemId("repair".into()),
                OwnerInput::Human {
                    station: StationId("engineering".into()),
                    rating: "Manual".into(),
                },
            ),
            (
                SystemId("logistics".into()),
                OwnerInput::Human {
                    station: StationId(if shared { "engineering" } else { "supply" }.into()),
                    rating: "Manual".into(),
                },
            ),
        ]),
    }
}
#[test]
fn independent_operators_overlap_but_cohosted_operators_do_not_hide_overload_in_a_minute() {
    let (compiled, mut selected) = timed(false, 24);
    compiled
        .commit_schedule(&mut selected.choice, inputs(false))
        .unwrap();
    let parallel = selected.choice.temporal.as_ref().unwrap();
    assert_eq!(parallel.profile_index, 1);
    assert!(parallel
        .nodes()
        .unwrap()
        .values()
        .all(|interval| interval.start_tick == 100 && interval.end_tick == 220));
    let buckets = compiled.workload(&selected.choice).unwrap();
    assert_eq!(buckets.len(), 2);
    assert!(buckets
        .iter()
        .all(|row| row.minute == 0 && row.attention_nanos == 5_500_000_000_000));

    let mut shared = compiled
        .choose("actual-crew", 100, |_| Ok(()))
        .unwrap()
        .choice;
    compiled.commit_schedule(&mut shared, inputs(true)).unwrap();
    let serial = shared.temporal.as_ref().unwrap();
    assert_eq!(serial.profile_index, 0);
    assert_eq!(serial.nodes().unwrap()["root/repair"].start_tick, 100);
    assert_eq!(serial.nodes().unwrap()["root/supply"].start_tick, 200);
    assert_eq!(serial.nodes().unwrap()["root/supply"].end_tick, 320);
    assert_eq!(
        compiled.workload(&shared).unwrap()[0].attention_nanos,
        11_000_000_000_000
    );
    // A minute-area average alone would permit both four-second full-attention
    // decisions at the same instant. The exact interval test refused that.
    assert!(
        serial.nodes().unwrap()["root/supply"].start_tick
            > serial.nodes().unwrap()["root/repair"].start_tick
    );
    let (tight, mut selected) = timed(false, 12);
    let before = selected.choice.clone();
    assert!(tight
        .commit_schedule(&mut selected.choice, inputs(true))
        .unwrap_err()
        .contains("infeasible"));
    assert_eq!(
        selected.choice, before,
        "no partial commitment after refusal"
    );
}

#[test]
fn refused_decision_is_historical_and_never_releases_or_launders_invalid_inputs() {
    let (compiled, mut selected) = timed(false, 12);
    compiled
        .record_schedule_decision(&mut selected.choice, inputs(true))
        .unwrap();
    let saved = selected.choice.temporal.as_ref().unwrap();
    assert!(
        matches!(&saved.outcome,TemporalOutcome::Refused{reason} if reason.contains("infeasible"))
    );
    let mut instances = SituationInstances::default();
    assert!(compiled
        .release(&mut selected.choice, &mut instances, "actual-crew", 100)
        .unwrap()
        .is_empty());
    assert!(instances.instances.is_empty());
    let decoded: PlanChoice =
        serde_json::from_slice(&serde_json::to_vec(&selected.choice).unwrap()).unwrap();
    for now in 100..460 {
        compiled
            .validate_history(&decoded, &instances, now)
            .unwrap();
    }
    let mut forged = decoded.clone();
    forged.temporal.as_mut().unwrap().outcome = TemporalOutcome::Refused {
        reason: "forged explanation".into(),
    };
    assert!(compiled.validate_history(&forged, &instances, 460).is_err());
    let mut unavailable = inputs(true);
    unavailable.owners.insert(
        SystemId("logistics".into()),
        OwnerInput::Unavailable {
            reason: UnavailableReason::Offline,
        },
    );
    let decision = compiled
        .schedule_decision(&selected.graph, 100, unavailable)
        .unwrap();
    assert!(
        matches!(decision.outcome,TemporalOutcome::Refused{reason} if reason.contains("unavailable"))
    );
    let mut invalid = inputs(true);
    invalid.tick_nanos = 0;
    assert!(compiled
        .schedule_decision(&selected.graph, 100, invalid)
        .is_err());
    for state in ["committed", "refused"] {
        let mut value = if state == "committed" {
            serde_json::json!({"state":state,"nodes":{}})
        } else {
            serde_json::json!({"state":state,"reason":"bounded"})
        };
        assert!(serde_json::from_value::<TemporalOutcome>(value.clone()).is_ok());
        value["hidden"] = serde_json::json!(true);
        assert!(serde_json::from_value::<TemporalOutcome>(value).is_err());
    }
}
#[test]
fn forecast_elapsed_is_not_acknowledged_work_and_early_work_does_not_release_before_earliest() {
    let (compiled, mut selected) = timed(true, 30);
    compiled
        .commit_schedule(&mut selected.choice, inputs(true))
        .unwrap();
    let mut instances = SituationInstances::default();
    compiled
        .release(&mut selected.choice, &mut instances, "actual-crew", 100)
        .unwrap();
    // The first actual method uses team1. Acceptance stays an ordinary owner fact.
    instances
        .accept_repair(
            "actual-crew",
            &SystemId("repair".into()),
            1,
            &RepairTarget::Work(room()),
            JobId(1),
            101,
        )
        .unwrap();
    let fact = instances
        .fact(
            &ActivityJob {
                actor: "actual-crew".into(),
                owner: SystemId("repair".into()),
                job: JobId(1),
            },
            102,
            ActivityOutcome::RepairAchieved {
                stage: 0,
                target: room(),
                remedy: "structural".into(),
                result: RepairAchievement::Structure { restored_hp: 1.0 },
            },
        )
        .unwrap();
    assert!(compiled
        .release(&mut selected.choice, &mut instances, "actual-crew", 220)
        .unwrap()
        .is_empty());
    instances.acknowledge(&fact, 220).unwrap();
    assert_eq!(
        compiled
            .release(&mut selected.choice, &mut instances, "actual-crew", 220)
            .unwrap(),
        vec!["htn/maintenance/root/supply"]
    );
    // A second plan acknowledges early; actual mission release still waits.
    let mut early = compiled
        .choose("actual-crew", 100, |_| Ok(()))
        .unwrap()
        .choice;
    compiled.commit_schedule(&mut early, inputs(true)).unwrap();
    let mut earlier = SituationInstances::default();
    compiled
        .release(&mut early, &mut earlier, "actual-crew", 100)
        .unwrap();
    earlier
        .accept_repair(
            "actual-crew",
            &SystemId("repair".into()),
            1,
            &RepairTarget::Work(room()),
            JobId(1),
            101,
        )
        .unwrap();
    earlier.acknowledge(&fact, 102).unwrap();
    assert!(compiled
        .release(&mut early, &mut earlier, "actual-crew", 219)
        .unwrap()
        .is_empty());
    assert_eq!(
        compiled
            .release(&mut early, &mut earlier, "actual-crew", 220)
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn original_inputs_and_deadline_survive_json_and_forged_frontiers_never_mutate_history() {
    let (compiled, mut selected) = timed(false, 24);
    compiled
        .commit_schedule(&mut selected.choice, inputs(true))
        .unwrap();
    let mut instances = SituationInstances::default();
    compiled
        .release(&mut selected.choice, &mut instances, "actual-crew", 100)
        .unwrap();
    let original = (selected.choice, instances);
    let (choice, history): (PlanChoice, SituationInstances) =
        serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    for tick in 100..460 {
        compiled.validate_history(&choice, &history, tick).unwrap();
        assert_eq!((&choice, &history), (&original.0, &original.1));
    }
    // No current rating/source input is accepted by history validation: later
    // disconnect/AI handoff cannot revise the accepted forecast or its deadline.
    for fault in 0..6 {
        let mut candidate = choice.clone();
        let saved = candidate.temporal.as_mut().unwrap();
        match fault {
            0 => saved.deadline_tick += 1,
            1 => saved.profile_index = 1,
            2 => {
                match &mut saved.outcome {
                    TemporalOutcome::Committed { nodes } => nodes,
                    _ => unreachable!(),
                }
                .get_mut("root/supply")
                .unwrap()
                .start_tick = 100
            }
            3 => {
                saved.input.owners.remove(&SystemId("repair".into()));
            }
            4 => saved.input.tick_nanos = 0,
            _ => {
                candidate.released.insert("root/supply".into(), 101);
            }
        }
        assert!(
            compiled
                .validate_history(&candidate, &history, 460)
                .is_err(),
            "fault{fault}"
        );
        assert_eq!((&choice, &history), (&original.0, &original.1));
    }
}
#[test]
fn bounds_recovery_and_exact_large_tick_arithmetic_refuse_before_release() {
    for source in ["ai", "simplified"] {
        let valid = serde_json::json!({"source": source});
        assert!(serde_json::from_value::<OwnerInput>(valid).is_ok());
        for key in ["station", "rating"] {
            let mut forged = serde_json::json!({"source": source});
            forged[key] = serde_json::json!("hidden");
            assert!(serde_json::from_value::<OwnerInput>(forged).is_err());
        }
    }
    let mut value = forecast();
    value.phases[1].offset_secs = 3;
    assert!(value.validate().is_err());
    value = forecast();
    value.max_secs = u32::MAX;
    assert!(value.validate().is_err());
    let (compiled, selected) = timed(false, 24);
    let mut late = compiled
        .choose("actual-crew", (1_u64 << 53) + 1, |_| Ok(()))
        .unwrap()
        .choice;
    compiled.commit_schedule(&mut late, inputs(true)).unwrap();
    let saved = late.temporal.as_ref().unwrap();
    assert_eq!(saved.deadline_tick, late.chosen_tick + 240);
    assert_eq!(
        saved.nodes().unwrap()["root/supply"].start_tick,
        late.chosen_tick + 100
    );
    let mut overflow = compiled
        .choose("actual-crew", u64::MAX - 10, |_| Ok(()))
        .unwrap()
        .choice;
    assert!(compiled
        .commit_schedule(&mut overflow, inputs(true))
        .is_err());
    assert!(overflow.temporal.is_none());
    let mut automated = inputs(false);
    for value in automated.owners.values_mut() {
        *value = OwnerInput::Ai {};
    }
    let saved = compiled.schedule(&selected.graph, 100, automated).unwrap();
    assert_eq!(
        saved.profile_index, 0,
        "AI did not invent occupied operators"
    );
}

#[test]
fn reusable_unused_missing_forecast_is_allowed_but_selected_missing_forecast_refuses() {
    let (template, _) = timed(false, 24);
    let mut library = library(false);
    for task in &mut library.tasks {
        if let Task::Primitive {
            id,
            forecast: value,
            ..
        } = task
        {
            *value = (id != "repair-second").then(forecast);
        }
    }
    let load = |library: &DomainLibrary| {
        CompiledPlanning::load(template.config().clone(), |path| {
            (path == LIB).then(|| toml::to_string(library).unwrap())
        })
        .unwrap()
        .0
    };
    let selected = load(&library)
        .choose("actual-crew", 100, |_| Ok(()))
        .unwrap();
    assert_eq!(selected.choice.methods["root"].method, "one-team");
    assert!(selected.refused_methods[0].contains("forecast"));
    for task in &mut library.tasks {
        if let Task::Primitive { id, forecast, .. } = task {
            if id == "repair-first" {
                *forecast = None;
            }
        }
    }
    assert!(load(&library)
        .choose("actual-crew", 100, |_| Ok(()))
        .unwrap_err()
        .contains("forecast"));
}

#[test]
fn changed_actual_input_recovers_refusal_without_new_method_profile_or_deadline() {
    let (compiled, selected) = timed(false, 24);
    let mut choice = selected.choice;
    let mut unavailable = inputs(false);
    for owner in unavailable.owners.values_mut() {
        *owner = OwnerInput::Unavailable {
            reason: UnavailableReason::Unoccupied,
        };
    }
    compiled
        .record_schedule_decision(&mut choice, unavailable.clone())
        .unwrap();
    let original = choice.temporal.clone();
    let methods = choice.methods.clone();
    assert!(!compiled
        .reconsider_schedule(&mut choice, unavailable, 101)
        .unwrap());
    assert!(choice.temporal_reconsiderations.is_empty());
    let mut actual = inputs(false);
    for owner in actual.owners.values_mut() {
        *owner = OwnerInput::Ai {};
    }
    assert!(compiled
        .reconsider_schedule(&mut choice, actual.clone(), 110)
        .unwrap());
    assert_eq!(choice.temporal, original);
    assert_eq!(choice.methods, methods);
    let effective = choice.effective_temporal().unwrap();
    assert!(!effective.refused());
    assert_eq!(
        effective.profile_index,
        original.as_ref().unwrap().profile_index
    );
    assert_eq!(
        effective.deadline_tick,
        original.as_ref().unwrap().deadline_tick
    );
    assert!(effective
        .nodes()
        .unwrap()
        .values()
        .all(|interval| interval.start_tick >= 110));
    let mut history = SituationInstances::default();
    compiled
        .release(&mut choice, &mut history, "actual-crew", 110)
        .unwrap();
    assert!(!choice.released.is_empty());
    let accepted = choice.clone();
    assert!(!compiled
        .reconsider_schedule(&mut choice, inputs(true), 111)
        .unwrap());
    assert_eq!(choice, accepted);
    let decoded: PlanChoice =
        serde_json::from_slice(&serde_json::to_vec(&choice).unwrap()).unwrap();
    for now in 110..470 {
        compiled.validate_history(&decoded, &history, now).unwrap();
    }
    for fault in 0..6 {
        let mut candidate = decoded.clone();
        let revision = &mut candidate.temporal_reconsiderations[0];
        match fault {
            0 => revision.tick = 99,
            1 => revision.decision.deadline_tick += 1,
            2 => revision.decision.profile_index += 1,
            3 => {
                let ReconsiderationCause::CrewControlChanged { owners } = &mut revision.cause;
                owners.clear();
            }
            4 => revision.decision.input = original.as_ref().unwrap().input.clone(),
            _ => {
                candidate.released.values_mut().for_each(|tick| *tick = 109);
            }
        }
        assert!(
            compiled
                .validate_history(&candidate, &history, 470)
                .is_err(),
            "fault{fault}"
        );
        assert_eq!(choice, accepted);
    }
}
#[test]
fn refusal_reconsideration_is_bounded_and_cannot_extend_an_expired_deadline() {
    let (compiled, selected) = timed(false, 24);
    let mut choice = selected.choice;
    let mut input = inputs(false);
    for value in input.owners.values_mut() {
        *value = OwnerInput::Unavailable {
            reason: UnavailableReason::Unoccupied,
        };
    }
    compiled
        .record_schedule_decision(&mut choice, input.clone())
        .unwrap();
    let deadline = choice.temporal.as_ref().unwrap().deadline_tick;
    for i in 0..MAX_RECONSIDERATIONS {
        for value in input.owners.values_mut() {
            *value = OwnerInput::Unavailable {
                reason: if i % 2 == 0 {
                    UnavailableReason::Offline
                } else {
                    UnavailableReason::Unoccupied
                },
            };
        }
        assert!(compiled
            .reconsider_schedule(&mut choice, input.clone(), 101 + i as u64)
            .unwrap());
    }
    let before = choice.clone();
    for value in input.owners.values_mut() {
        *value = OwnerInput::Ai {};
    }
    assert!(!compiled
        .reconsider_schedule(&mut choice, input.clone(), 200)
        .unwrap());
    assert_eq!(choice, before);
    let mut late = compiled
        .choose("actual-crew", 100, |_| Ok(()))
        .unwrap()
        .choice;
    compiled
        .record_schedule_decision(&mut late, before.temporal.as_ref().unwrap().input.clone())
        .unwrap();
    assert!(compiled
        .reconsider_schedule(&mut late, input, deadline + 1)
        .unwrap());
    assert!(late.effective_temporal().unwrap().refused());
    assert_eq!(late.effective_temporal().unwrap().deadline_tick, deadline);
    compiled
        .validate_history(&late, &SituationInstances::default(), deadline + 1)
        .unwrap();
}
