use super::*;
use phoenix_model::logistics::ResourceEndpoint;
use std::collections::BTreeMap;

fn profile() -> RecoveryToolConfig {
    RecoveryToolConfig {
        system: SystemId("recovery".into()),
        range: 300.0,
        units_per_sec: 4.0,
        resources: vec!["alloy".into(), "torpedoes".into()],
    }
}
fn selected(profile: &RecoveryToolConfig) -> RecoveryState {
    let mut state = RecoveryState::new(profile).unwrap();
    state
        .select(profile, Some("source"), Some("alloy"), Some("hold-alloy"))
        .unwrap();
    state.set_enabled(true);
    state
}

#[test]
fn no_supply_space_selection_or_enable_buys_no_work() {
    let profile = profile();
    let mut state = RecoveryState::new(&profile).unwrap();
    assert!(!state.enabled);
    assert_eq!(
        state
            .advance(&profile, 1, 1.0, 4, |_, _| panic!("disabled"))
            .unwrap(),
        0
    );
    state.set_enabled(true);
    assert_eq!(
        state
            .advance(&profile, 2, 1.0, 4, |_, _| panic!("no selection"))
            .unwrap(),
        0
    );
    state
        .select(&profile, Some("source"), Some("alloy"), Some("hold-alloy"))
        .unwrap();
    assert_eq!(
        state
            .advance(&profile, 3, 1.0, 0, |_, _| panic!(
                "no actual supply or receiving room"
            ))
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .advance(&profile, 4, 0.0, 4, |_, _| panic!("no elapsed work"))
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .advance(&profile, 5, 1.0, 4, |_, seconds| {
                assert_eq!(seconds, 1.0);
                Ok(0.0)
            })
            .unwrap(),
        0
    );
    assert_eq!(state.fraction, 0.0);
    state.validate(&profile, 5).unwrap();
}

#[test]
fn actual_limit_bounds_paid_seconds_and_receipt_is_explicit_exact_and_idempotent() {
    let profile = profile();
    let mut state = selected(&profile);
    let mut work_seconds = 0.0;
    let offered = state
        .advance(&profile, 1, 99.0, 2, |owner, seconds| {
            assert_eq!(owner, &profile.system);
            work_seconds += seconds;
            Ok(1.0)
        })
        .unwrap();
    assert_eq!(offered, 2);
    assert_eq!(work_seconds, 0.5);
    assert!(
        state.validate(&profile, 1).is_err(),
        "pending movement cannot pass a save barrier"
    );
    let pending = state.clone();
    for (tick, moved) in [(0, 1), (1, -1), (1, 3)] {
        assert!(state.record_receipt(tick, moved).is_err());
        assert_eq!(state, pending);
    }
    state.record_receipt(1, 0).unwrap();
    state.record_receipt(1, 0).unwrap();
    assert!(
        state.record_receipt(1, 1).is_err(),
        "even a zero settlement is final"
    );
    state.validate(&profile, 1).unwrap();
    assert_eq!(
        state
            .advance(&profile, 1, 99.0, 2, |_, _| panic!("tick replay"))
            .unwrap(),
        0
    );
    assert_eq!(state.last_outcome.as_ref().unwrap().moved, 0);
}

#[test]
fn fractional_paid_work_survives_disable_json_and_missing_actual_supply() {
    let mut profile = profile();
    profile.units_per_sec = 0.5;
    let mut state = selected(&profile);
    for tick in 1..=2 {
        assert_eq!(
            state
                .advance(&profile, tick, 1.0, 4, |_, _| Ok(0.5))
                .unwrap(),
            0
        );
    }
    assert_eq!(state.fraction, 0.5);
    let mut restored: RecoveryState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    for cursor in [&mut state, &mut restored] {
        cursor.set_enabled(false);
        assert_eq!(
            cursor
                .advance(&profile, 3, 9.0, 4, |_, _| panic!("disabled work"))
                .unwrap(),
            0
        );
        cursor.set_enabled(true);
        assert_eq!(
            cursor
                .advance(&profile, 4, 9.0, 0, |_, _| panic!("no supply"))
                .unwrap(),
            0
        );
        assert_eq!(cursor.fraction, 0.5);
        assert_eq!(
            cursor.advance(&profile, 5, 1.0, 4, |_, _| Ok(1.0)).unwrap(),
            1
        );
        cursor.record_receipt(5, 1).unwrap();
        cursor.validate(&profile, 5).unwrap();
    }
    assert_eq!(state, restored);
    assert_eq!(state.fraction, 0.0);
}

#[test]
fn selection_changes_discard_only_paid_fraction_and_invalid_changes_are_atomic() {
    let mut profile = profile();
    profile.units_per_sec = 0.25;
    let mut state = selected(&profile);
    state.advance(&profile, 1, 1.0, 4, |_, _| Ok(1.0)).unwrap();
    assert_eq!(state.fraction, 0.25);
    state
        .select(&profile, Some("source"), Some("alloy"), Some("hold-alloy"))
        .unwrap();
    assert_eq!(state.fraction, 0.25);
    let before = state.clone();
    for (target, resource, dest) in [
        (Some(""), Some("alloy"), Some("hold-alloy")),
        (Some("source"), Some("unknown"), Some("hold-alloy")),
        (None, Some("alloy"), None),
    ] {
        assert!(state.select(&profile, target, resource, dest).is_err());
        assert_eq!(state, before);
    }
    state
        .select(
            &profile,
            Some("other-source"),
            Some("alloy"),
            Some("hold-alloy"),
        )
        .unwrap();
    assert_eq!(state.fraction, 0.0);
    state.advance(&profile, 2, 1.0, 4, |_, _| Ok(1.0)).unwrap();
    state
        .select(
            &profile,
            Some("other-source"),
            Some("torpedoes"),
            Some("hold-ammunition"),
        )
        .unwrap();
    assert_eq!(state.fraction, 0.0);
    state.select(&profile, None, None, None).unwrap();
    state.validate(&profile, 2).unwrap();
}

#[test]
fn malformed_time_power_bounds_and_continuation_refuse_without_state_commit() {
    let profile = profile();
    let mut state = selected(&profile);
    let before = state.clone();
    for (dt, limit) in [(f32::NAN, 1), (f32::INFINITY, 1), (-1.0, 1), (1.0, -1)] {
        assert!(state
            .advance(&profile, 1, dt, limit, |_, _| panic!("invalid work"))
            .is_err());
        assert_eq!(state, before);
    }
    for fraction in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(state
            .advance(&profile, 1, 1.0, 4, |_, _| Ok(fraction))
            .is_err());
        assert_eq!(state, before);
    }
    assert!(state
        .advance(&profile, 1, 1.0, 4, |_, _| Err("work refused".into()))
        .is_err());
    assert_eq!(state, before);
    let mut enormous = profile.clone();
    enormous.units_per_sec = 2.0;
    assert!(state
        .advance(&enormous, 1, f32::MAX, i64::MAX, |_, _| panic!(
            "integer overflow"
        ))
        .is_err());
    assert_eq!(state, before);
    for mode in 0..6 {
        let mut bad = before.clone();
        match mode {
            0 => bad.owner = SystemId("other".into()),
            1 => bad.fraction = f64::NAN,
            2 => bad.fraction = 1.0,
            3 => bad.resource = None,
            4 => bad.last_tick = 2,
            _ => {
                bad.last_outcome = Some(RecoveryOutcome {
                    tick: 1,
                    offered: 1,
                    moved: 2,
                    settled: true,
                })
            }
        }
        assert!(bad.validate(&profile, 1).is_err());
    }
}

#[test]
fn nonrepresentable_limit_duration_never_overbuys_or_loses_its_paid_fraction() {
    let mut profile = profile();
    profile.units_per_sec = 3.0;
    let mut state = selected(&profile);
    let mut moved = 0;
    let mut paid_seconds = 0.0f64;
    for tick in 1..=8 {
        let offered = state
            .advance(&profile, tick, 1.0, 1 - moved, |_, seconds| {
                paid_seconds += f64::from(seconds);
                Ok(1.0)
            })
            .unwrap();
        state.record_receipt(tick, offered).unwrap();
        moved += offered;
    }
    assert_eq!(moved, 1);
    assert!(paid_seconds <= 1.0 / 3.0);
    assert_eq!(state.fraction, 0.0);
}

#[test]
fn closed_tool_and_source_config_validate_real_endpoint_shapes() {
    let valid = profile();
    valid.validate().unwrap();
    for mode in 0..5 {
        let mut bad = valid.clone();
        match mode {
            0 => bad.system = SystemId(" ".into()),
            1 => bad.range = 0.0,
            2 => bad.units_per_sec = f32::NAN,
            3 => bad.resources.clear(),
            _ => bad.resources.push("alloy".into()),
        }
        assert!(bad.validate().is_err());
    }
    let mut source = RecoverableSourceConfig {
        availability: RecoveryAvailability::Disabled,
        resources: BTreeMap::from([(
            "alloy".into(),
            ResourceEndpoint::Capacity { id: "stock".into() },
        )]),
    };
    source.validate().unwrap();
    source.resources.insert(
        "parts".into(),
        ResourceEndpoint::Capacity { id: "stock".into() },
    );
    assert!(source.validate().is_err());
    source.resources = BTreeMap::from([("alloy".into(), ResourceEndpoint::TorpedoMagazine {})]);
    assert!(source.validate().is_err());
    assert!(serde_json::from_value::<RecoveryToolConfig>(serde_json::json!({"system":"recovery","range":1,"units_per_sec":1,"resources":["alloy"],"script":"extra"})).is_err());
    assert!(serde_json::from_value::<RecoverableSourceConfig>(
        serde_json::json!({"availability":"mining","resources":{}})
    )
    .is_err());
}

#[test]
fn ordinary_tree_has_only_closed_permitted_recovery_inputs() {
    let bindings = crate::resources::recovery_ai::bindings();
    let program =
        crate::ai::state_tree::compile(&crate::resources::recovery_ai::initial_tree(), &bindings)
            .unwrap();
    let mut memory = crate::ai::state_tree::TreeRuntime::default();
    for (tick, available) in [(1, false), (2, true), (3, false)] {
        let step = crate::ai::state_tree::step(
            &program,
            &BTreeMap::from([
                ("enabled".into(), true),
                ("recovery_available".into(), available),
            ]),
            &BTreeMap::from([("recover".into(), false)]),
            &mut memory,
            tick,
        )
        .unwrap();
        assert_eq!(step.intents.get("recover"), Some(&available));
    }
    assert_eq!(bindings.facts.len(), 2);
    assert_eq!(bindings.channels.len(), 1);
}
