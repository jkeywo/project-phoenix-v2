use super::*;

fn config(arc: f32) -> TurretConfig {
    TurretConfig {
        facing_deg: 0.0,
        traverse_arc_deg: arc,
        traverse_deg_per_sec: 90.0,
        aim_tolerance_deg: 1.0,
    }
}

#[test]
fn paid_traverse_is_finite_disabled_and_unfunded_ticks_freeze_without_backlog() {
    let config = config(270.0);
    let mut state = TurretState::default();
    assert_eq!(
        state
            .advance(&config, 0, 1.0, true, Some(90.0), 0.25)
            .unwrap()
            .moved_deg,
        22.5
    );
    assert_eq!(state.offset_deg, 22.5);
    state
        .advance(&config, 1, 20.0, false, Some(90.0), 1.0)
        .unwrap();
    state
        .advance(&config, 2, 20.0, true, Some(90.0), 0.0)
        .unwrap();
    assert_eq!(state.offset_deg, 22.5);
    let mut resumed: TurretState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    for tick in 3..20 {
        assert_eq!(
            state
                .advance(&config, tick, 0.1, true, Some(90.0), 0.5)
                .unwrap(),
            resumed
                .advance(&config, tick, 0.1, true, Some(90.0), 0.5)
                .unwrap()
        );
        assert_eq!(state, resumed);
    }
    assert!(state.ready(&config, 90.0));
    let before = state.clone();
    assert_eq!(
        state
            .advance(&config, 19, 100.0, true, Some(-90.0), 1.0)
            .unwrap()
            .moved_deg,
        0.0
    );
    assert_eq!(state, before);
}

#[test]
fn limited_sector_cannot_take_the_shortcut_across_its_forbidden_back() {
    let mut limited = TurretState {
        offset_deg: 130.0,
        last_tick: Some(0),
    };
    limited
        .advance(&config(300.0), 1, 1.0, true, Some(-130.0), 1.0)
        .unwrap();
    assert_eq!(limited.offset_deg, 40.0);
    let mut full = TurretState {
        offset_deg: 130.0,
        last_tick: Some(0),
    };
    full.advance(&config(360.0), 1, 1.0, true, Some(-130.0), 1.0)
        .unwrap();
    assert_eq!(full.offset_deg, -140.0);
    full.advance(&config(360.0), 2, 1.0, true, Some(-130.0), 1.0)
        .unwrap();
    assert!(full.ready(&config(360.0), -130.0));
}

#[test]
fn unreachable_targets_stop_at_limit_and_no_target_returns_home_through_paid_motion() {
    let config = config(90.0);
    let mut state = TurretState::default();
    state
        .advance(&config, 0, 10.0, true, Some(120.0), 1.0)
        .unwrap();
    assert_eq!(state.offset_deg, 45.0);
    assert!(!state.ready(&config, 120.0));
    state.advance(&config, 1, 0.25, true, None, 1.0).unwrap();
    assert_eq!(state.offset_deg, 22.5);
    state.advance(&config, 2, 0.25, true, None, 1.0).unwrap();
    assert_eq!(state.offset_deg, 0.0);
}

#[test]
fn finite_aim_adds_readiness_to_the_existing_static_range_and_sector() {
    let config = config(180.0);
    let mut state = TurretState::default();
    let mount = super::super::mount::MountConfig {
        facing_deg: 0.0,
        fire_arc_deg: 180.0,
        range: 100.0,
    };
    let target = [50.0, 0.0];
    assert!(
        !state
            .geometry(&config, &mount, target, [0.0, 0.0], 0.0)
            .in_arc
    );
    state
        .advance(&config, 0, 1.0, true, Some(90.0), 1.0)
        .unwrap();
    assert!(
        state
            .geometry(&config, &mount, target, [0.0, 0.0], 0.0)
            .in_arc
    );
    assert!(
        !state
            .geometry(&config, &mount, [101.0, 0.0], [0.0, 0.0], 0.0)
            .in_range
    );
    assert!(
        !state
            .geometry(&config, &mount, [0.0, 50.0], [0.0, 0.0], 0.0)
            .in_arc
    );
}

#[test]
fn malformed_config_state_and_steps_refuse_without_moving_or_advancing_frontier() {
    let config = config(90.0);
    let mut state = TurretState::default();
    for bad in [f32::NAN, f32::INFINITY, -1.0] {
        let before = state.clone();
        assert!(state
            .advance(&config, 1, bad, true, Some(0.0), 1.0)
            .is_err());
        assert_eq!(state, before);
    }
    for bad in [f32::NAN, 1.1, -0.1] {
        assert!(state
            .advance(&config, 1, 1.0, true, Some(0.0), bad)
            .is_err());
    }
    assert!(state
        .advance(&config, 1, 1.0, true, Some(f32::INFINITY), 1.0)
        .is_err());
    state.last_tick = Some(10);
    let before = state.clone();
    assert!(state.advance(&config, 9, 1.0, true, None, 1.0).is_err());
    assert_eq!(state, before);
    for invalid in [
        TurretState {
            offset_deg: 46.0,
            last_tick: Some(0),
        },
        TurretState {
            offset_deg: 1.0,
            last_tick: None,
        },
        TurretState {
            offset_deg: f64::NAN,
            last_tick: Some(0),
        },
    ] {
        assert!(invalid.validate(&config, 10).is_err());
    }
    for invalid in [
        TurretConfig {
            traverse_deg_per_sec: 0.0,
            ..config.clone()
        },
        TurretConfig {
            traverse_arc_deg: 361.0,
            ..config.clone()
        },
        TurretConfig {
            aim_tolerance_deg: 46.0,
            ..config.clone()
        },
        TurretConfig {
            facing_deg: f32::INFINITY,
            ..config.clone()
        },
    ] {
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn tiny_paid_intervals_accumulate_real_motion_without_a_second_work_counter() {
    let config = config(360.0);
    let mut state = TurretState::default();
    for tick in 0..100 {
        state
            .advance(&config, tick, 1e-6, true, Some(90.0), 0.5)
            .unwrap();
    }
    assert!((state.offset_deg - 0.0045).abs() < 1e-9);
    state.validate(&config, 100).unwrap();
}
