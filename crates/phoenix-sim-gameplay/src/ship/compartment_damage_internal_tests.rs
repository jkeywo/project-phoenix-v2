use super::*;

#[test]
fn internal_incident_preserves_exact_budget_scope_faults_and_recoverability() {
    let definition = definition();
    let mut state = DamageState::new(&definition);
    let before = state.clone();
    assert!(state
        .apply_internal(
            &definition,
            &["unknown".into()],
            10.0,
            "gm.direct",
            &mut rng()
        )
        .is_err());
    assert_eq!(state, before);
    assert_eq!(
        state
            .apply_internal(
                &definition,
                &["front".into(), "front".into()],
                140.0,
                "gm.direct",
                &mut rng()
            )
            .unwrap(),
        100.0
    );
    assert_eq!(state.compartments["rear"].hp, 100.0);
    assert!(state.compartments["front"].disabled);
    assert_eq!(condition(&state, "engine"), &ComponentCondition::Destroyed);
    assert_eq!(condition(&state, "scanner"), &ComponentCondition::Healthy);
    assert!(!state.catastrophic);
    state.restore_structure(&definition, "front", 25.0).unwrap();
    assert!(!state.compartments["front"].disabled);
    assert_eq!(
        condition(&state, "engine"),
        &ComponentCondition::Destroyed,
        "structural restoration does not erase the fault"
    );
    state.validate(&definition).unwrap();
}

#[test]
fn internal_spill_and_rng_are_independent_of_requested_room_order() {
    let definition = definition();
    let mut left = DamageState::new(&definition);
    let mut right = left.clone();
    assert_eq!(
        left.apply_internal(
            &definition,
            &["front".into(), "rear".into()],
            150.0,
            "gm.direct",
            &mut rng()
        )
        .unwrap(),
        150.0
    );
    right
        .apply_internal(
            &definition,
            &["rear".into(), "front".into()],
            150.0,
            "gm.direct",
            &mut rng(),
        )
        .unwrap();
    assert_eq!(left, right);
    assert_eq!(left.structural_fraction(&definition), 0.25);
    assert!(!left.catastrophic);
}
