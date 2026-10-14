use super::*;

#[test]
fn recovered_charge_uses_remaining_intake_and_preserves_passive_state() {
    let config = config();
    let mut state = EnergyState::new(&config, 96.0, 3.0).unwrap();
    assert_eq!(state.accept_charge(&config, 20.0, 2.0).unwrap(), 2.0);
    assert_eq!(state.energy, 98.0);
    assert_eq!(state.heat, 7.0);
    assert_eq!(state.accept_charge(&config, 20.0, 10.0).unwrap(), 2.0);
    assert_eq!(state.energy, 100.0);
    let full = state;
    assert_eq!(state.accept_charge(&config, 1.0, 10.0).unwrap(), 0.0);
    assert_eq!(state, full);
    assert!(state.accept_charge(&config, f32::NAN, 10.0).is_err());
    assert_eq!(state, full);
}

#[test]
fn tiny_recovery_cannot_round_into_free_stored_energy() {
    let config = config();
    let mut state = EnergyState::new(&config, 90.0, 0.0).unwrap();
    for offer in [f32::EPSILON, 0.00001, 0.1, 0.3, 0.7] {
        let before = state.energy;
        let debit = state.accept_charge(&config, offer, offer).unwrap();
        let credit = state.energy as f64 - before as f64;
        assert!(credit <= debit as f64);
        assert!(debit <= offer);
    }
}

fn config() -> EnergyConfig {
    EnergyConfig {
        capacity: 100.0,
        max_intake_per_sec: 10.0,
        passive_loss_at_full_per_sec: 2.0,
        charge_heat_per_unit: 2.0,
        discharge_heat_per_unit: 3.0,
        passive_cooling_per_sec: 0.0,
        coolant_heat_per_unit: 4.0,
        trip_heat: 1_000.0,
        recover_heat: 500.0,
    }
}

fn tick<'a>(demands: &'a [EnergyDemand]) -> EnergyStep<'a> {
    EnergyStep {
        dt: 1.0,
        offered_energy: 0.0,
        demands,
        cooling_rate: 0.0,
        coolant_available: 0.0,
    }
}

fn demand(id: &str, priority: i32, kind: EnergyDemandKind, amount: f32) -> EnergyDemand {
    EnergyDemand {
        id: id.into(),
        priority,
        kind,
        amount,
    }
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
}

#[test]
fn full_reserve_is_more_expensive_and_empty_reserve_has_no_passive_loss() {
    let cfg = config();
    for (energy, expected_loss) in [(100.0, 2.0), (50.0, 1.0), (0.0, 0.0)] {
        let mut state = EnergyState::new(&cfg, energy, 0.0).unwrap();
        let result = state.step(&cfg, tick(&[])).unwrap();
        close(result.passive_loss, expected_loss);
        close(state.energy, energy - expected_loss);
        close(state.heat, expected_loss * cfg.discharge_heat_per_unit);
    }
}

#[test]
fn actual_headroom_and_intake_limit_conserve_source_offer() {
    let cfg = config();
    let mut state = EnergyState::new(&cfg, 99.0, 0.0).unwrap();
    let result = state
        .step(
            &cfg,
            EnergyStep {
                offered_energy: 30.0,
                ..tick(&[])
            },
        )
        .unwrap();
    close(result.passive_loss, 1.98);
    close(result.accepted_energy, 2.98);
    close(result.accepted_energy + result.rejected_energy, 30.0);
    close(state.energy, 100.0);
    close(state.heat, 1.98 * 3.0 + 2.98 * 2.0);
    let mut empty = EnergyState::default();
    let result = empty
        .step(
            &cfg,
            EnergyStep {
                offered_energy: 30.0,
                ..tick(&[])
            },
        )
        .unwrap();
    assert_eq!(result.accepted_energy, 10.0);
    assert_eq!(result.rejected_energy, 20.0);
}

#[test]
fn attempted_work_does_not_invent_receiving_headroom_or_spend_rejected_supply() {
    let cfg = EnergyConfig {
        passive_loss_at_full_per_sec: 0.0,
        ..config()
    };
    let mut state = EnergyState::new(&cfg, 100.0, 0.0).unwrap();
    let work = [demand("beam", 0, EnergyDemandKind::Continuous, 10.0)];
    let result = state
        .step(
            &cfg,
            EnergyStep {
                offered_energy: 10.0,
                ..tick(&work)
            },
        )
        .unwrap();
    assert_eq!(result.accepted_energy, 0.0);
    assert_eq!(result.rejected_energy, 10.0);
    assert_eq!(state.energy, 90.0);
    assert_eq!(result.work_energy, 10.0);
}

#[test]
fn gross_charging_and_depletion_heat_survive_an_unchanged_net_energy_level() {
    let cfg = EnergyConfig {
        passive_loss_at_full_per_sec: 0.0,
        ..config()
    };
    let mut state = EnergyState::new(&cfg, 50.0, 0.0).unwrap();
    let work = [demand("engine", 0, EnergyDemandKind::Continuous, 10.0)];
    let result = state
        .step(
            &cfg,
            EnergyStep {
                offered_energy: 10.0,
                ..tick(&work)
            },
        )
        .unwrap();
    assert_eq!(state.energy, 50.0);
    assert_eq!(result.heat_generated, 50.0);
    assert_eq!(state.heat, 50.0);
}

#[test]
fn continuous_work_scales_and_discrete_work_never_partly_consumes() {
    let cfg = config();
    let mut state = EnergyState::new(&cfg, 6.0, 0.0).unwrap();
    let before = state;
    assert!(!state.try_spend_discrete(&cfg, 10.0).unwrap());
    assert_eq!(state, before);
    let result = state.spend_continuous(&cfg, 10.0).unwrap();
    assert_eq!(
        result,
        EnergyGrant {
            requested: 10.0,
            granted: 6.0,
            fraction: 0.6
        }
    );
    assert_eq!(state.energy, 0.0);
    assert_eq!(state.heat, 18.0);
    assert!(!state.thermal_stopped);
    assert_eq!(state.spend_continuous(&cfg, 2.0).unwrap().fraction, 0.0);
    assert_eq!(state.spend_continuous(&cfg, 0.0).unwrap().fraction, 1.0);
}

#[test]
fn zero_intake_does_not_stop_stored_energy_or_create_a_restart_lock() {
    let cfg = EnergyConfig {
        passive_loss_at_full_per_sec: 0.0,
        ..config()
    };
    let mut state = EnergyState::new(&cfg, 8.0, 0.0).unwrap();
    let work = [demand("launcher", 0, EnergyDemandKind::Discrete, 8.0)];
    assert_eq!(
        state.step(&cfg, tick(&work)).unwrap().grants[0]
            .grant
            .fraction,
        1.0
    );
    assert_eq!(state.energy, 0.0);
    assert!(!state.try_spend_discrete(&cfg, 1.0).unwrap());
    state
        .step(
            &cfg,
            EnergyStep {
                offered_energy: 1.0,
                ..tick(&[])
            },
        )
        .unwrap();
    assert!(state.try_spend_discrete(&cfg, 1.0).unwrap());
}

#[test]
fn explicit_priority_and_identity_determine_grants_instead_of_input_order() {
    let cfg = EnergyConfig {
        passive_loss_at_full_per_sec: 0.0,
        ..config()
    };
    let work = [
        demand("z", 10, EnergyDemandKind::Discrete, 4.0),
        demand("b", 5, EnergyDemandKind::Continuous, 5.0),
        demand("a", 5, EnergyDemandKind::Discrete, 4.0),
    ];
    let mut forward = EnergyState::new(&cfg, 10.0, 0.0).unwrap();
    let mut backward = forward;
    let a = forward.step(&cfg, tick(&work)).unwrap();
    let reversed: Vec<_> = work.iter().cloned().rev().collect();
    let b = backward.step(&cfg, tick(&reversed)).unwrap();
    assert_eq!(a, b);
    assert_eq!(forward, backward);
    assert_eq!(
        a.grants
            .iter()
            .map(|result| result.id.as_str())
            .collect::<Vec<_>>(),
        ["z", "a", "b"]
    );
    assert_eq!(
        a.grants
            .iter()
            .map(|result| result.grant.granted)
            .collect::<Vec<_>>(),
        [4.0, 4.0, 2.0]
    );
    assert_eq!(forward.energy, 0.0);
    assert_eq!(a.work_energy, 10.0);
}

#[test]
fn cooling_consumes_only_actual_heat_removed_after_passive_rejection() {
    let cfg = EnergyConfig {
        passive_cooling_per_sec: 2.0,
        ..config()
    };
    let mut state = EnergyState::new(&cfg, 0.0, 10.0).unwrap();
    let result = state
        .step(
            &cfg,
            EnergyStep {
                cooling_rate: 100.0,
                coolant_available: 20.0,
                ..tick(&[])
            },
        )
        .unwrap();
    assert_eq!(result.passive_heat_removed, 2.0);
    assert_eq!(result.coolant_heat_removed, 8.0);
    assert_eq!(result.coolant_used, 2.0);
    assert_eq!(state.heat, 0.0);
    let result = state
        .step(
            &cfg,
            EnergyStep {
                cooling_rate: 100.0,
                coolant_available: 20.0,
                ..tick(&[])
            },
        )
        .unwrap();
    assert_eq!(result.coolant_used, 0.0);
    assert_eq!(result.coolant_heat_removed, 0.0);
}

#[test]
fn finite_coolant_budget_and_allocated_cooling_capacity_both_limit_removal() {
    let cfg = config();
    let original = EnergyState::new(&cfg, 0.0, 100.0).unwrap();
    for (rate, budget, expected_heat, expected_units) in [
        (100.0, 0.5, 2.0, 0.5),
        (5.0, 100.0, 5.0, 1.25),
        (100.0, 0.0, 0.0, 0.0),
    ] {
        let mut state = original;
        let result = state
            .step(
                &cfg,
                EnergyStep {
                    cooling_rate: rate,
                    coolant_available: budget,
                    ..tick(&[])
                },
            )
            .unwrap();
        assert_eq!(result.coolant_heat_removed, expected_heat);
        assert_eq!(result.coolant_used, expected_units);
        assert_eq!(state.heat, original.heat - expected_heat);
    }
}

#[test]
fn thermal_protection_trips_then_recovers_only_at_the_lower_threshold() {
    let cfg = EnergyConfig {
        trip_heat: 20.0,
        recover_heat: 5.0,
        ..config()
    };
    let mut state = EnergyState::new(&cfg, 20.0, 0.0).unwrap();
    assert!(state.try_spend_discrete(&cfg, 8.0).unwrap());
    assert!(state.thermal_stopped);
    let stopped = state;
    assert!(!state.try_spend_discrete(&cfg, 1.0).unwrap());
    assert_eq!(state.spend_continuous(&cfg, 2.0).unwrap().granted, 0.0);
    assert_eq!(state, stopped);
    state
        .step(
            &cfg,
            EnergyStep {
                dt: 0.0,
                ..tick(&[])
            },
        )
        .unwrap();
    assert!(state.thermal_stopped);
    state
        .step(
            &cfg,
            EnergyStep {
                cooling_rate: 100.0,
                coolant_available: 3.0,
                ..tick(&[])
            },
        )
        .unwrap();
    assert!(state.thermal_stopped);
    state
        .step(
            &cfg,
            EnergyStep {
                cooling_rate: 100.0,
                coolant_available: 10.0,
                ..tick(&[])
            },
        )
        .unwrap();
    assert!(!state.thermal_stopped);
    assert!(state.try_spend_discrete(&cfg, 1.0).unwrap());
}

#[test]
fn charging_and_passive_loss_can_trip_protection_without_any_requested_work() {
    let charge_cfg = EnergyConfig {
        trip_heat: 10.0,
        recover_heat: 5.0,
        ..config()
    };
    let mut charging = EnergyState::default();
    charging
        .step(
            &charge_cfg,
            EnergyStep {
                offered_energy: 10.0,
                ..tick(&[])
            },
        )
        .unwrap();
    assert!(charging.thermal_stopped);
    let loss_cfg = EnergyConfig {
        trip_heat: 5.0,
        recover_heat: 2.0,
        ..config()
    };
    let mut leaking = EnergyState::new(&loss_cfg, 100.0, 0.0).unwrap();
    leaking.step(&loss_cfg, tick(&[])).unwrap();
    assert!(leaking.thermal_stopped);
}

#[test]
fn config_state_and_inputs_reject_invalid_numbers_before_mutation() {
    let cfg = config();
    for bad in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(EnergyConfig {
            capacity: bad,
            ..cfg
        }
        .validate()
        .is_err());
        assert!(EnergyConfig {
            charge_heat_per_unit: bad,
            ..cfg
        }
        .validate()
        .is_err());
        assert!(EnergyConfig {
            max_intake_per_sec: bad,
            ..cfg
        }
        .validate()
        .is_err());
        assert!(EnergyState::new(&cfg, bad, 0.0).is_err());
        assert!(EnergyState::new(&cfg, 0.0, bad).is_err());
        let mut state = EnergyState::new(&cfg, 10.0, 10.0).unwrap();
        let before = state;
        assert!(state
            .step(
                &cfg,
                EnergyStep {
                    dt: bad,
                    ..tick(&[])
                }
            )
            .is_err());
        assert!(state
            .step(
                &cfg,
                EnergyStep {
                    offered_energy: bad,
                    ..tick(&[])
                }
            )
            .is_err());
        assert!(state
            .step(
                &cfg,
                EnergyStep {
                    cooling_rate: bad,
                    ..tick(&[])
                }
            )
            .is_err());
        assert!(state
            .step(
                &cfg,
                EnergyStep {
                    coolant_available: bad,
                    ..tick(&[])
                }
            )
            .is_err());
        assert!(state.spend_continuous(&cfg, bad).is_err());
        assert!(state.try_spend_discrete(&cfg, bad).is_err());
        assert_eq!(state, before);
    }
    assert!(EnergyConfig {
        recover_heat: cfg.trip_heat,
        ..cfg
    }
    .validate()
    .is_err());
    assert!(EnergyState::new(&cfg, cfg.capacity + 1.0, 0.0).is_err());
    assert!(EnergyState {
        energy: 0.0,
        heat: cfg.trip_heat,
        thermal_stopped: false
    }
    .validate_against(&cfg)
    .is_err());
    assert!(
        EnergyState::new(&cfg, 0.0, cfg.trip_heat)
            .unwrap()
            .thermal_stopped
    );
}

#[test]
fn duplicate_or_invalid_work_and_unrepresentable_heat_roll_back_the_step() {
    let cfg = config();
    let mut state = EnergyState::new(&cfg, 100.0, 0.0).unwrap();
    let before = state;
    let duplicates = [
        demand("same", 0, EnergyDemandKind::Continuous, 5.0),
        demand("same", 1, EnergyDemandKind::Discrete, 1.0),
    ];
    assert!(state.step(&cfg, tick(&duplicates)).is_err());
    let invalid = [demand("bad", 0, EnergyDemandKind::Continuous, f32::NAN)];
    assert!(state.step(&cfg, tick(&invalid)).is_err());
    let impossible = EnergyConfig {
        discharge_heat_per_unit: f32::MAX,
        ..cfg
    };
    assert!(state.try_spend_discrete(&impossible, 10.0).is_err());
    assert_eq!(state, before);
}

#[test]
fn reserve_and_thermal_continuation_round_trip_and_replay_identically() {
    let cfg = config();
    let work = [demand("engine", 0, EnergyDemandKind::Continuous, 4.0)];
    let mut state = EnergyState::new(&cfg, 50.0, 0.0).unwrap();
    state
        .step(
            &cfg,
            EnergyStep {
                dt: 0.25,
                offered_energy: 1.0,
                ..tick(&work)
            },
        )
        .unwrap();
    let saved = toml::to_string(&state).unwrap();
    let mut restored: EnergyState = toml::from_str(&saved).unwrap();
    restored.validate_against(&cfg).unwrap();
    for _ in 0..25 {
        let result = state
            .step(
                &cfg,
                EnergyStep {
                    dt: 0.1,
                    offered_energy: 0.5,
                    cooling_rate: 10.0,
                    coolant_available: 0.25,
                    ..tick(&work)
                },
            )
            .unwrap();
        let resumed = restored
            .step(
                &cfg,
                EnergyStep {
                    dt: 0.1,
                    offered_energy: 0.5,
                    cooling_rate: 10.0,
                    coolant_available: 0.25,
                    ..tick(&work)
                },
            )
            .unwrap();
        assert_eq!(state, restored);
        assert_eq!(result, resumed);
    }
}
