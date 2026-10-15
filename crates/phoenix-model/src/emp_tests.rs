use super::*;

fn policy() -> EmpResistanceConfig {
    EmpResistanceConfig {
        resistance_per_stack: 1.0,
        max_stacks: 4,
        decay_secs: 20.0,
        bindings: vec![DamageBinding::Operation],
    }
}
fn clock(tick: u64) -> EmpClock {
    EmpClock {
        tick,
        period_nanos: 1_000_000_000,
    }
}
fn payload(duration_secs: f32) -> EmpPayload {
    EmpPayload { duration_secs }
}

#[test]
fn refresh_uses_old_resistance_keeps_longer_remaining_effect_and_decays_without_repair() {
    let policy = policy();
    let first = ComponentEmpState::apply(None, &policy, payload(10.0), clock(0)).unwrap();
    assert_eq!(first.effect_until_tick, 10);
    assert_eq!(first.next_decay_tick, 30);
    let second = ComponentEmpState::apply(Some(&first), &policy, payload(10.0), clock(1)).unwrap();
    assert_eq!(second.effect_until_tick, 10);
    assert_eq!(second.resistance_stacks, 2);
    let third = ComponentEmpState::apply(Some(&second), &policy, payload(30.0), clock(2)).unwrap();
    assert_eq!(third.effect_until_tick, 12);
    assert_eq!(third.next_decay_tick, 32);
    assert_eq!(third.factor_at(11), 0.0);
    assert_eq!(third.factor_at(12), 1.0);
    assert_eq!(third.stacks_at(31), 3);
    assert_eq!(third.stacks_at(32), 2);
    assert_eq!(third.stacks_at(52), 1);
    assert_eq!(third.stacks_at(72), 0);
}

#[test]
fn same_tick_hits_are_ordered_and_positive_duration_never_becomes_immunity() {
    let mut state = None;
    let policy = policy();
    for _ in 0..20 {
        state = Some(
            ComponentEmpState::apply(state.as_ref(), &policy, payload(1e-30), clock(5)).unwrap(),
        );
    }
    let state = state.unwrap();
    assert_eq!(state.effect_until_tick, 6);
    assert_eq!(state.resistance_stacks, 4);
    let long_then_short = ComponentEmpState::apply(
        Some(&ComponentEmpState::apply(None, &policy, payload(8.0), clock(0)).unwrap()),
        &policy,
        payload(4.0),
        clock(0),
    )
    .unwrap();
    let short_then_long = ComponentEmpState::apply(
        Some(&ComponentEmpState::apply(None, &policy, payload(4.0), clock(0)).unwrap()),
        &policy,
        payload(8.0),
        clock(0),
    )
    .unwrap();
    assert_eq!(long_then_short.effect_until_tick, 8);
    assert_eq!(short_then_long.effect_until_tick, 4);
}

#[test]
fn stowed_transferred_and_json_copies_expire_from_causal_ticks_without_a_ticking_inventory() {
    let original = ComponentEmpState::apply(None, &policy(), payload(5.0), clock(100)).unwrap();
    let stored: ComponentEmpState =
        serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(original, stored);
    assert_eq!(stored.factor_at(104), 0.0);
    assert_eq!(stored.factor_at(105), 1.0);
    assert_eq!(stored.stacks_at(125), 0);
    assert_eq!(stored, original); // Reading time does not mutate another owner.
    stored.validate(&policy(), clock(200)).unwrap();
}

#[test]
fn malformed_profiles_future_frontiers_and_overflow_refuse_without_rewriting_old_state() {
    let policy = policy();
    let state = ComponentEmpState::apply(None, &policy, payload(5.0), clock(1)).unwrap();
    let before = state.clone();
    assert!(ComponentEmpState::apply(Some(&state), &policy, payload(5.0), clock(0)).is_err());
    assert!(
        ComponentEmpState::apply(Some(&state), &policy, payload(5.0), clock(u64::MAX - 2)).is_err()
    );
    assert!(ComponentEmpState::apply(Some(&state), &policy, payload(f32::MAX), clock(2)).is_err());
    assert_eq!(state, before);
    for changed in [
        ComponentEmpState {
            next_decay_tick: 6,
            ..state.clone()
        },
        ComponentEmpState {
            resistance_stacks: 5,
            ..state.clone()
        },
        ComponentEmpState {
            decay_ticks: 0,
            ..state.clone()
        },
        ComponentEmpState {
            effect_until_tick: 0,
            ..state.clone()
        },
    ] {
        assert!(changed.validate(&policy, clock(10)).is_err());
    }
    for changed in [
        EmpResistanceConfig {
            resistance_per_stack: 0.0,
            ..policy.clone()
        },
        EmpResistanceConfig {
            max_stacks: 0,
            ..policy.clone()
        },
        EmpResistanceConfig {
            decay_secs: f32::INFINITY,
            ..policy.clone()
        },
        EmpResistanceConfig {
            bindings: vec![DamageBinding::Operation; 2],
            ..policy.clone()
        },
    ] {
        assert!(changed.validate().is_err());
    }
}
