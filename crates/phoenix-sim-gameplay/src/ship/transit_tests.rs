use super::*;
fn order(now: u64, request: Option<(&str, u64)>) -> TransitOrder {
    TransitOrder {
        destination: TransitDestination::Local {
            point: [10.0, 2.0, 0.0],
            course_generation: 7,
        },
        original_world: TransitWorldBinding::Base,
        origin: [0.0, 2.0, 0.0],
        profile: AcceptedLocalJumpProfile {
            config: LocalJumpConfig {
                preparation_work_secs: 2.0,
                max_distance: 50.0,
                scatter_radius: 1.0,
                gravity_response: 1.0,
                world_travel: false,
                fuel: TransitFuel {
                    store: "local-fuel".into(),
                    amount: 2,
                },
                reading_ticks: 120,
            },
            preparation_energy_per_sec: 1.0,
            commit_energy: 3.0,
        },
        accepted_tick: now,
        request: request.map(|(id, revision)| TransitRequestKey {
            objective_id: id.into(),
            revision,
        }),
    }
}
fn funded(work: TransitWork, _: &TransitOrder) -> Result<TransitGrant, String> {
    Ok(match work {
        TransitWork::Prepare { seconds } => TransitGrant::Preparation { seconds },
        TransitWork::Commit => TransitGrant::Commitment { paid: true },
    })
}
fn placement(order: &TransitOrder, _: [f32; 3]) -> Option<TransitArrivalSolution> {
    TransitArrivalSolution::resolve(order, [0.0; 3], [1.0, 0.0, 0.0], vec![]).ok()
}
#[test]
fn paid_preparation_pauses_commit_waits_and_arrival_is_once_after_json() {
    let mut state = TransitState::default();
    state.begin(order(10, Some(("request", 1))), 10).unwrap();
    assert_eq!(
        state
            .advance(10, 1.0, [0.0, 2.0, 0.0], true, true, 0.5, funded, placement)
            .unwrap(),
        TransitAdvance::Prepared
    );
    assert_eq!(state.worked_secs(), 0.5);
    let mut callback_count = 0;
    let mut no_work = |_: TransitWork, _: &TransitOrder| {
        callback_count += 1;
        panic!("unavailable/duplicate ticks cannot request work")
    };
    assert_eq!(
        state
            .advance(
                11,
                1.0,
                [0.0, 2.0, 0.0],
                true,
                false,
                1.0,
                &mut no_work,
                placement
            )
            .unwrap(),
        TransitAdvance::Waiting(TransitWait::Unavailable)
    );
    assert_eq!(
        state
            .advance(
                11,
                1.0,
                [0.0, 2.0, 0.0],
                true,
                true,
                1.0,
                &mut no_work,
                placement
            )
            .unwrap(),
        TransitAdvance::Unchanged
    );
    assert_eq!(callback_count, 0);
    assert_eq!(state.worked_secs(), 0.5);
    let mut commits = 0;
    assert_eq!(
        state
            .advance(
                12,
                1.5,
                [0.0, 2.0, 0.0],
                true,
                true,
                1.0,
                |request, _| Ok(match request {
                    TransitWork::Prepare { seconds } => TransitGrant::Preparation { seconds },
                    TransitWork::Commit => {
                        commits += 1;
                        TransitGrant::Commitment { paid: false }
                    }
                }),
                placement
            )
            .unwrap(),
        TransitAdvance::Waiting(TransitWait::Unfunded)
    );
    assert_eq!(commits, 1);
    assert_eq!(state.worked_secs(), 2.0);
    let mut prepared_again = false;
    assert_eq!(
        state
            .advance(
                13,
                1.0,
                [0.0, 2.0, 0.0],
                true,
                true,
                1.0,
                |request, _| {
                    prepared_again |= matches!(request, TransitWork::Prepare { .. });
                    funded(request, &order(0, None))
                },
                placement
            )
            .unwrap(),
        TransitAdvance::Committed
    );
    assert!(
        !prepared_again,
        "a waiting commit cannot spend preparation twice"
    );
    let mut resumed: TransitState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    resumed.validate(13).unwrap();
    assert!(resumed.cancel(13).is_err());
    for tick in 14..374 {
        let live = state
            .advance(
                tick,
                1.0,
                [0.0, 2.0, 0.0],
                true,
                false,
                0.0,
                |_, _| panic!("paid commitment cannot pay again"),
                |_, _| panic!("arrival cannot re-scatter"),
            )
            .unwrap();
        let restored = resumed
            .advance(
                tick,
                1.0,
                [0.0, 2.0, 0.0],
                true,
                false,
                0.0,
                |_, _| panic!("restored commitment cannot pay again"),
                |_, _| panic!("restored arrival cannot re-scatter"),
            )
            .unwrap();
        assert_eq!(live, restored);
        assert_eq!(state, resumed);
        assert_eq!(matches!(live, TransitAdvance::Arrived { .. }), tick == 14);
    }
}
#[test]
fn two_standing_requests_cannot_alternate_old_revisions_after_restore() {
    let mut state = TransitState::default();
    for (tick, id) in [(1, "first"), (2, "second")] {
        state.begin(order(tick, Some((id, 4))), tick).unwrap();
        state.cancel(tick).unwrap();
    }
    let mut state: TransitState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    let before = state.clone();
    for (id, revision) in [("first", 4), ("second", 4), ("first", 1), ("second", 3)] {
        let mut changed = order(3, Some((id, revision)));
        changed.destination = TransitDestination::Local {
            point: [20.0, 2.0, 0.0],
            course_generation: 8,
        };
        assert!(state.begin(changed, 3).is_err());
        assert_eq!(state, before);
    }
    state.begin(order(3, Some(("first", 5))), 3).unwrap();
    state.cancel(3).unwrap();
    state.begin(order(4, None), 4).unwrap();
    assert_eq!(
        state.handled.len(),
        2,
        "manual Begin creates no mission receipt"
    );
}
#[test]
fn technical_bound_refuses_before_acceptance_without_eviction_or_blocking_manual_work() {
    let mut state = TransitState::default();
    for i in 0..MAX_HANDLED_TRANSIT_OBJECTIVES {
        let id = format!("objective-{i}");
        state
            .begin(order(i as u64, Some((&id, 1))), i as u64)
            .unwrap();
        state.cancel(i as u64).unwrap();
    }
    let now = MAX_HANDLED_TRANSIT_OBJECTIVES as u64;
    let before = state.clone();
    assert!(state
        .begin(order(now, Some(("new-distinct", 1))), now)
        .is_err());
    assert_eq!(state, before);
    state
        .begin(order(now, Some(("objective-0", 2))), now)
        .unwrap();
    state.cancel(now).unwrap();
    state.begin(order(now, None), now).unwrap();
    assert_eq!(state.handled.len(), MAX_HANDLED_TRANSIT_OBJECTIVES);
}
#[test]
fn missing_physics_range_and_invalid_frontiers_never_call_commit_or_apply_arrival() {
    let mut state = TransitState::default();
    state.begin(order(1, None), 1).unwrap();
    assert_eq!(
        state
            .advance(
                1,
                2.0,
                [100.0, 2.0, 0.0],
                true,
                true,
                1.0,
                |request, _| match request {
                    TransitWork::Prepare { seconds } => Ok(TransitGrant::Preparation { seconds }),
                    TransitWork::Commit => panic!("out-of-range order cannot commit"),
                },
                |_, _| panic!("out-of-range order cannot resolve arrival")
            )
            .unwrap(),
        TransitAdvance::Waiting(TransitWait::OutOfRange)
    );
    assert_eq!(
        state
            .advance(
                2,
                1.0,
                [0.0, 2.0, 0.0],
                false,
                true,
                1.0,
                |_, _| panic!("missing owner cannot pay"),
                placement
            )
            .unwrap(),
        TransitAdvance::Waiting(TransitWait::Unavailable)
    );
    assert_eq!(
        state
            .advance(3, 1.0, [0.0, 2.0, 0.0], true, true, 1.0, funded, placement)
            .unwrap(),
        TransitAdvance::Committed
    );
    assert_eq!(
        state
            .advance(
                4,
                1.0,
                [0.0, 2.0, 0.0],
                false,
                false,
                0.0,
                |_, _| panic!("commit already paid"),
                placement
            )
            .unwrap(),
        TransitAdvance::Waiting(TransitWait::Unavailable)
    );
    let before = state.clone();
    assert!(state
        .advance(3, 1.0, [0.0, 2.0, 0.0], true, true, 1.0, funded, placement)
        .is_err());
    assert_eq!(state, before);
    for variant in 0..4 {
        let mut bad = before.clone();
        match variant {
            0 => bad.last_tick = Some(6),
            1 => {
                if let TransitPhase::Committed { arrival, .. } = &mut bad.phase {
                    arrival.point[1] = f32::INFINITY
                }
            }
            2 => {
                if let TransitPhase::Committed { commit_tick, .. } = &mut bad.phase {
                    *commit_tick = 0
                }
            }
            _ => {
                if let TransitPhase::Committed { order, .. } = &mut bad.phase {
                    order.profile.config.fuel.amount = 0
                }
            }
        }
        assert!(bad.validate(5).is_err(), "variant {variant}");
    }
}
#[test]
fn frozen_profile_and_accepted_goal_do_not_follow_later_authoring_or_course_changes() {
    let mut state = TransitState::default();
    let accepted = order(1, Some(("order", 1)));
    let original = accepted.clone();
    state.begin(accepted, 1).unwrap();
    let mut later = original.clone();
    later.profile.config.fuel.amount = 1;
    later.profile.config.preparation_work_secs = 0.1;
    later.destination = TransitDestination::Local {
        point: [20.0, 2.0, 0.0],
        course_generation: 99,
    };
    assert!(state.begin(later, 1).is_err());
    assert_eq!(state.phase.order(), Some(&original));
    let before = state.clone();
    assert!(state
        .advance(
            1,
            1.0,
            [0.0, 2.0, 0.0],
            true,
            true,
            1.0,
            |_, _| Ok(TransitGrant::Preparation {
                seconds: f64::INFINITY
            }),
            placement
        )
        .is_err());
    assert_eq!(
        state, before,
        "bad staged receipts cannot mutate causal work"
    );
}

#[test]
fn forged_same_tick_arrival_refuses_even_with_finite_paid_shape() {
    let mut state = TransitState::default();
    state.begin(order(1, None), 1).unwrap();
    assert_eq!(
        state
            .advance(1, 2.0, [0.0, 2.0, 0.0], true, true, 1.0, funded, placement)
            .unwrap(),
        TransitAdvance::Committed
    );
    let TransitPhase::Committed {
        order,
        commit_tick,
        arrival,
    } = state.phase.clone()
    else {
        panic!("actual paid commit")
    };
    state.phase = TransitPhase::Arrived {
        order,
        commit_tick,
        arrival,
        arrival_tick: commit_tick,
    };
    assert!(state.validate(1).is_err());
}

#[test]
fn original_scatter_and_static_gravity_justify_saved_point_without_a_final_scatter_only_cap() {
    let order = order(1, None);
    let body = TransitGravityBody {
        body_id: "static-body".into(),
        position: [10.0, 2.0, 0.0],
        policy: ArrivalGravityConfig {
            exclusion_radius: 5.0,
        },
    };
    let receipt =
        TransitArrivalSolution::resolve(&order, [0.0; 3], [1.0, 0.0, 0.0], vec![body]).unwrap();
    assert_eq!(receipt.point, [15.0, 2.0, 0.0]);
    receipt.validate(&order).unwrap();
    assert!(
        distance(receipt.point, order.destination.point())
            > f64::from(order.profile.config.scatter_radius)
    );
    let mut forged = receipt.clone();
    forged.point = [5000.0, 2.0, 0.0];
    assert!(forged.validate(&order).is_err());
    assert!(
        TransitArrivalSolution::resolve(&order, [2.0, 0.0, 0.0], [1.0, 0.0, 0.0], vec![]).is_err()
    );
    let mut state = TransitState::default();
    state.begin(order.clone(), 1).unwrap();
    assert_eq!(
        state
            .advance(
                1,
                2.0,
                [0.0, 2.0, 0.0],
                true,
                true,
                1.0,
                |request, _| match request {
                    TransitWork::Prepare { seconds } => Ok(TransitGrant::Preparation { seconds }),
                    TransitWork::Commit => panic!("unjustified placement cannot debit commit"),
                },
                |_, _| Some(forged.clone())
            )
            .unwrap(),
        TransitAdvance::Waiting(TransitWait::NoPlacement)
    );
    assert_eq!(
        state
            .advance(2, 1.0, [0.0, 2.0, 0.0], true, true, 1.0, funded, |_, _| {
                Some(receipt.clone())
            })
            .unwrap(),
        TransitAdvance::Committed
    );
    if let TransitPhase::Committed { arrival, .. } = &mut state.phase {
        arrival.point = [5000.0, 2.0, 0.0];
    }
    assert!(state.validate(2).is_err());
}

#[test]
fn stable_body_pass_refuses_reentry_into_an_earlier_exclusion_before_any_commit() {
    let mut order = order(1, None);
    order.profile.config.max_distance = 200.0;
    order.destination = TransitDestination::Local {
        point: [75.0, 2.0, 0.0],
        course_generation: 1,
    };
    let body = |id: &str, x| TransitGravityBody {
        body_id: id.into(),
        position: [x, 2.0, 0.0],
        policy: ArrivalGravityConfig {
            exclusion_radius: 100.0,
        },
    };
    let overlap = vec![body("a", 0.0), body("b", 150.0)];
    assert!(
        TransitArrivalSolution::resolve(&order, [0.0; 3], [1.0, 0.0, 0.0], overlap.clone())
            .is_err()
    );
    let positive = TransitArrivalSolution::resolve(
        &order,
        [0.0; 3],
        [1.0, 0.0, 0.0],
        vec![body("b", 300.0), body("a", 0.0)],
    )
    .unwrap();
    assert_eq!(positive.point, [100.0, 2.0, 0.0]);
    positive.validate(&order).unwrap();
    let mut state = TransitState::default();
    state.begin(order, 1).unwrap();
    assert_eq!(
        state
            .advance(
                1,
                2.0,
                [0.0, 2.0, 0.0],
                true,
                true,
                1.0,
                |action, _| match action {
                    TransitWork::Prepare { seconds } => Ok(TransitGrant::Preparation { seconds }),
                    TransitWork::Commit => panic!("invalid overlap cannot debit fuel or Power"),
                },
                |order, _| TransitArrivalSolution::resolve(
                    order,
                    [0.0; 3],
                    [1.0, 0.0, 0.0],
                    overlap.clone()
                )
                .ok()
            )
            .unwrap(),
        TransitAdvance::Waiting(TransitWait::NoPlacement)
    );
}

#[test]
fn drive_response_scales_original_gravity_without_mutating_body_evidence() {
    let mut selected = order(1, None);
    let body = TransitGravityBody {
        body_id: "fixed-body".into(),
        position: selected.destination.point(),
        policy: ArrivalGravityConfig {
            exclusion_radius: 8.0,
        },
    };
    for (response, expected_x) in [(1.0, 18.0), (0.25, 12.0), (0.0, 10.0), (2.0, 26.0)] {
        selected.profile.config.gravity_response = response;
        let receipt = TransitArrivalSolution::resolve(
            &selected,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            vec![body.clone()],
        )
        .unwrap();
        assert_eq!(receipt.point, [expected_x, 2.0, 0.0]);
        assert_eq!(receipt.gravity, vec![body.clone()]);
        receipt.validate(&selected).unwrap();
        let mut forged = selected.clone();
        forged.profile.config.gravity_response = response + 1.0;
        assert!(receipt.validate(&forged).is_err());
        let resumed: TransitArrivalSolution =
            serde_json::from_slice(&serde_json::to_vec(&receipt).unwrap()).unwrap();
        resumed.validate(&selected).unwrap();
    }
}
