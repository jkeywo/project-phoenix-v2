use super::*;

/// A depot with one capacity and one `transfer_capable` threshold that
/// fails below 40 % and, with the default 0.05 band, returns at 45 %.
fn depot() -> InfrastructureConfig {
    InfrastructureConfig {
        condition_max: 100.0,
        capacities: vec![CapacityConfig {
            label: None,
            id: "transfer_throughput".to_string(),
            amount: 40,
            ceiling: None,
        }],
        thresholds: vec![ThresholdConfig {
            label: None,
            flag: "depot_transfer_capable".to_string(),
            capacity: None,
            fails_below: 0.4,
            restores_above: None,
        }],
        ..Default::default()
    }
}

// ── AC1: an omitted table changes nothing; an authored one parses ──

#[test]
fn an_authored_table_takes_the_documented_defaults_for_everything_it_omits() {
    let parsed: InfrastructureConfig = toml::from_str("").expect("an empty table is legal");
    assert_eq!(
        parsed,
        InfrastructureConfig::default(),
        "serde's defaults and the hand-written Default impl must agree — two copies of \
             these numbers could only drift"
    );
    assert_eq!(
        parsed.condition_max, 100.0,
        "the condition ceiling falls back to the documented parse default"
    );
    assert!(
        parsed.thresholds.is_empty() && parsed.capacities.is_empty(),
        "a table that declares neither thresholds nor capacities declares neither"
    );
}

#[test]
fn the_authored_vocabulary_round_trips_through_toml() {
    let authored = r#"
condition_max = 200.0
condition = 150.0
decay_per_sec = 0.5
hull_damage_share = 0.25
hysteresis = 0.1
publish = false

[[capacity]]
id = "berths"
amount = 12

[[threshold]]
flag = "docking_capable"
capacity = "berths"
fails_below = 0.3
restores_above = 0.6
"#;
    let parsed: InfrastructureConfig = toml::from_str(authored).expect("the vocabulary parses");
    assert_eq!(parsed.condition_max, 200.0);
    assert_eq!(parsed.condition, Some(150.0));
    assert_eq!(parsed.decay_per_sec, 0.5);
    assert_eq!(parsed.hull_damage_share, 0.25);
    assert_eq!(parsed.hysteresis, 0.1);
    assert!(!parsed.publish);
    assert_eq!(
        parsed.capacities.len(),
        1,
        "[[capacity]] is a repeated block"
    );
    assert_eq!(
        parsed.thresholds.len(),
        1,
        "[[threshold]] is a repeated block"
    );
    assert_eq!(parsed.thresholds[0].capacity.as_deref(), Some("berths"));
    parsed.validate().expect("the authored table is valid");
}

#[test]
fn an_omitted_threshold_capacity_keeps_condition_backing() {
    let parsed: ThresholdConfig = toml::from_str(
        r#"
flag = "load_bearing"
fails_below = 0.4
"#,
    )
    .expect("the original threshold vocabulary still parses");
    assert_eq!(parsed.capacity, None);
    assert_eq!(ThresholdConfig::default().capacity, None);
}

#[test]
fn an_unknown_key_is_a_parse_error_rather_than_a_silently_ignored_typo() {
    let err = toml::from_str::<InfrastructureConfig>("conditon_max = 50.0")
        .expect_err("a misspelt key must not be swallowed");
    assert!(
        err.to_string().contains("conditon_max"),
        "the error must name the offending key, got {err}"
    );
}

// ── AC2: thresholds flip in BOTH directions, with a dead band ──

#[test]
fn a_threshold_falls_on_the_way_down_and_returns_on_the_way_up() {
    let mut state = InfrastructureState::from_config(&depot());
    assert_eq!(
        state.flag("depot_transfer_capable"),
        Some(true),
        "an intact depot starts capable"
    );

    let falling = state.degrade(65.0);
    assert_eq!(
        falling,
        vec![FlagChange {
            flag: "depot_transfer_capable".to_string(),
            raised: false
        }],
        "dropping to 35 % crosses fails_below (40 %) and reports the flag falling"
    );
    assert_eq!(state.flag("depot_transfer_capable"), Some(false));

    let rising = state.repair(15.0);
    assert_eq!(
        rising,
        vec![FlagChange {
            flag: "depot_transfer_capable".to_string(),
            raised: true
        }],
        "climbing to 50 % clears restores_above (45 %) and reports the flag returning"
    );
    assert_eq!(state.flag("depot_transfer_capable"), Some(true));
}

#[test]
fn a_condition_parked_inside_the_hysteresis_band_reports_no_transition() {
    let mut state = InfrastructureState::from_config(&depot());
    state.degrade(65.0);
    assert_eq!(state.flag("depot_transfer_capable"), Some(false));

    // 35 % → 42 %: past fails_below, short of restores_above. This is the
    // exact value a hovering structure sits at, and it must be silent.
    let changes = state.repair(7.0);
    assert!(
        changes.is_empty(),
        "a value inside the dead band must report nothing, got {changes:?}"
    );
    assert_eq!(
        state.flag("depot_transfer_capable"),
        Some(false),
        "…and must stay down until the restore point is actually reached"
    );
}

#[test]
fn a_point_of_damage_and_a_point_of_repair_on_the_boundary_do_not_chatter() {
    let mut state = InfrastructureState::from_config(&depot());
    for lap in 0..8 {
        // Park the depot back on the failure line, capable, for each lap.
        state.set_condition(100.0);
        state.set_condition(40.0);
        assert_eq!(
            state.flag("depot_transfer_capable"),
            Some(true),
            "lap {lap}: sitting exactly on fails_below is still capable — the flag falls \
                 BELOW it"
        );
        let down = state.degrade(1.0);
        assert_eq!(
            down.len(),
            1,
            "lap {lap}: the first point below the line is a genuine crossing"
        );
        let up = state.repair(1.0);
        assert!(
            up.is_empty(),
            "lap {lap}: putting that one point back lands in the dead band, so the flag \
                 stays down instead of flapping once per tick, got {up:?}"
        );
    }
}

#[test]
fn an_authored_restore_point_overrides_the_default_band() {
    let mut config = depot();
    config.thresholds[0].restores_above = Some(0.9);
    let mut state = InfrastructureState::from_config(&config);
    state.degrade(70.0);
    assert_eq!(state.flag("depot_transfer_capable"), Some(false));
    assert!(
        state.repair(50.0).is_empty(),
        "80 % is above the default band but below the authored 90 % restore point"
    );
    assert_eq!(
        state.repair(10.0),
        vec![FlagChange {
            flag: "depot_transfer_capable".to_string(),
            raised: true
        }],
        "…and 90 % is the authored restore point exactly, so the flag returns there"
    );
}

#[test]
fn a_capacity_backed_threshold_flips_from_the_capacity_track() {
    let config = InfrastructureConfig {
        capacities: vec![CapacityConfig {
            id: "reserve_fuel".to_string(),
            amount: 0,
            ceiling: Some(100),
            label: None,
        }],
        thresholds: vec![ThresholdConfig {
            flag: "transfer_primed".to_string(),
            capacity: Some("reserve_fuel".to_string()),
            fails_below: 0.4,
            restores_above: Some(0.5),
            label: None,
        }],
        ..Default::default()
    };
    config
        .validate()
        .expect("a threshold may read a capacity on the same infrastructure track");
    let mut state = InfrastructureState::from_config(&config);
    assert_eq!(state.flag("transfer_primed"), Some(false));

    let (level, short) = state
        .adjust_capacity("reserve_fuel", 49)
        .expect("the capacity exists");
    assert_eq!(level, 49);
    assert!(
        short.is_empty(),
        "49 % remains below the authored 50 % restore point"
    );

    let (_, raised) = state
        .adjust_capacity("reserve_fuel", 1)
        .expect("the capacity exists");
    assert_eq!(
        raised,
        vec![FlagChange {
            flag: "transfer_primed".to_string(),
            raised: true,
        }],
        "filling the receiving ledger to its authored line raises the operational edge"
    );

    let (_, fell) = state
        .adjust_capacity("reserve_fuel", -11)
        .expect("the capacity exists");
    assert_eq!(
        fell,
        vec![FlagChange {
            flag: "transfer_primed".to_string(),
            raised: false,
        }],
        "draining below the failure line clears the same target-side flag"
    );
}

#[test]
fn a_capacity_backed_threshold_must_name_a_capacity_on_the_same_entity() {
    let config = InfrastructureConfig {
        thresholds: vec![ThresholdConfig {
            flag: "transfer_primed".to_string(),
            capacity: Some("missing".to_string()),
            fails_below: 0.5,
            restores_above: Some(0.5),
            label: None,
        }],
        ..Default::default()
    };
    let err = config
        .validate()
        .expect_err("a target-local reading cannot silently bind a global counter");
    assert!(
        err.contains("declares no [[infrastructure.capacity]]"),
        "the authoring error must explain the missing local capacity, got {err}"
    );
}

#[test]
fn a_capacity_backed_threshold_rejects_an_empty_source_id() {
    let config = InfrastructureConfig {
        thresholds: vec![ThresholdConfig {
            flag: "transfer_primed".to_string(),
            capacity: Some("   ".to_string()),
            fails_below: 0.5,
            restores_above: Some(0.5),
            label: None,
        }],
        ..Default::default()
    };
    let err = config
        .validate()
        .expect_err("an empty source cannot identify a target-local capacity");
    assert!(
        err.contains("capacity must be a non-empty id"),
        "the validation error must name the capacity source, got {err}"
    );
}

#[test]
fn a_structure_that_starts_degraded_starts_with_its_flag_already_down() {
    let mut config = depot();
    config.condition = Some(10.0);
    let state = InfrastructureState::from_config(&config);
    assert_eq!(
        state.flag("depot_transfer_capable"),
        Some(false),
        "flags are level-evaluated once at construction, so a mission opening on a wrecked \
             skyhook opens with the flag down rather than flipping it on tick one"
    );
    assert_eq!(
        state.initial_flags(),
        vec![FlagChange {
            flag: "depot_transfer_capable".to_string(),
            raised: false
        }],
        "…and the starting set is phrased as changes so the caller mirrors it through the \
             same path it mirrors later edges"
    );
}

// ── AC4: repair raises condition, incrementally ──

#[test]
fn a_repair_can_arrive_in_arbitrarily_small_increments() {
    let mut config = depot();
    config.condition = Some(30.0);
    let mut state = InfrastructureState::from_config(&config);
    assert_eq!(
        state.flag("depot_transfer_capable"),
        Some(false),
        "precondition: 30 % starts below the 40 % failure point"
    );
    let mut crossings = 0;
    for _ in 0..400 {
        // A timed operation's per-tick slice: 0.05 points at a time.
        crossings += state.repair(0.05).len();
    }
    // Within a float's worth of 50: four hundred `f32` additions of 0.05 do
    // not land on the number a single `+ 20.0` would, and the tolerance
    // says so rather than pretending otherwise.
    assert!(
        (state.condition() - 50.0).abs() < 0.01,
        "four hundred slices of 0.05 must add up to the 20 points a single jump would, \
             got {}",
        state.condition()
    );
    assert_eq!(
        crossings, 1,
        "the crossing is reported exactly once, on the slice that carries the condition \
             over the 45 % restore point — not once per slice above it, and not never"
    );
}

#[test]
fn condition_is_clamped_at_both_ends() {
    let mut state = InfrastructureState::from_config(&depot());
    state.repair(1_000.0);
    assert_eq!(
        state.condition(),
        100.0,
        "a repair cannot exceed the ceiling"
    );
    state.degrade(1_000.0);
    assert_eq!(
        state.condition(),
        0.0,
        "damage cannot drive condition negative"
    );
    assert_eq!(state.condition_fraction(), 0.0);
}

#[test]
fn a_degrade_refuses_a_negative_amount_instead_of_repairing() {
    let mut state = InfrastructureState::from_config(&depot());
    assert!(state.degrade(-50.0).is_empty());
    assert_eq!(
        state.condition(),
        100.0,
        "a negative degrade is ignored — the sign convention is in the method name, and a \
             caller that got it wrong must not silently heal the structure"
    );
    assert!(state.repair(-50.0).is_empty());
    assert_eq!(state.condition(), 100.0);
}

// ── AC3-adjacent: damage drives condition down ──

#[test]
fn the_first_hull_observation_only_records_the_baseline() {
    let mut state = InfrastructureState::from_config(&depot());
    assert!(state.observe_hull(200.0).is_empty());
    assert_eq!(
        state.condition(),
        100.0,
        "a structure that spawns with 200 hull has not just taken 200 points of damage"
    );
}

#[test]
fn a_hull_drop_is_booked_as_condition_damage_at_the_authored_share() {
    let mut config = depot();
    config.hull_damage_share = 0.5;
    let mut state = InfrastructureState::from_config(&config);
    state.observe_hull(200.0);
    assert!(
        state.observe_hull(120.0).is_empty(),
        "80 hull → 40 condition"
    );
    assert_eq!(
        state.condition(),
        60.0,
        "the authored share is what converts hull points into condition points"
    );
    // 60 more hull points off is 30 more condition points: 30/100 = 30 %,
    // the first reading strictly below the authored 40 %.
    let changes = state.observe_hull(60.0);
    assert_eq!(
        changes,
        vec![FlagChange {
            flag: "depot_transfer_capable".to_string(),
            raised: false
        }],
        "…and a hit that takes condition through a threshold reports the crossing, so the \
             damage path and the script path flip the same flag by the same rule"
    );
}

#[test]
fn a_share_of_zero_leaves_condition_entirely_to_the_scenario() {
    let mut config = depot();
    config.hull_damage_share = 0.0;
    let mut state = InfrastructureState::from_config(&config);
    state.observe_hull(200.0);
    state.observe_hull(1.0);
    assert_eq!(
        state.condition(),
        100.0,
        "a structure whose condition is purely script-driven authors share = 0 and is not \
             quietly degraded by the hull it happens to also carry"
    );
}

#[test]
fn a_hull_that_climbs_does_not_repair_condition() {
    let mut state = InfrastructureState::from_config(&depot());
    state.observe_hull(100.0);
    state.observe_hull(60.0);
    assert_eq!(state.condition(), 60.0);
    assert!(state.observe_hull(100.0).is_empty());
    assert_eq!(
        state.condition(),
        60.0,
        "condition is its own track: only the repair hooks raise it, so a hull restored by \
             some other means does not silently un-degrade the structure"
    );
}

// ── AC5: capacity is readable without hard-coding the number ──

#[test]
fn a_consumer_can_ask_a_depot_how_much_it_moves() {
    let state = InfrastructureState::from_config(&depot());
    assert_eq!(
        state.capacity("transfer_throughput"),
        Some(40),
        "the authored number is the answer"
    );
    assert_eq!(
        state.capacity("berths"),
        None,
        "…and a capacity the structure never declared reads as absent rather than zero, so \
             a consumer can tell 'holds nobody' apart from 'does not do berths'"
    );
}

#[test]
fn capacity_does_not_move_when_condition_does() {
    let mut state = InfrastructureState::from_config(&depot());
    state.degrade(90.0);
    assert_eq!(
        state.capacity("transfer_throughput"),
        Some(40),
        "capacity is an authored property of the structure. A scenario that wants a \
             battered depot to move less reads the condition and decides that itself, rather \
             than having an implicit curve applied underneath it."
    );
}

// ── AC6: nothing here is a hidden-truth field ──

#[test]
fn every_readable_field_is_one_the_scenario_authored() {
    let config = depot();
    let state = InfrastructureState::from_config(&config);
    assert_eq!(state.condition(), config.condition_max);
    assert_eq!(
        state.flags(),
        vec![("depot_transfer_capable", true)],
        "the flag set is exactly the authored thresholds"
    );
    assert_eq!(state.capacities().len(), config.capacities.len());
    assert!(
        state.publishes(),
        "publication is authored, defaulting to on; a scenario that wants a structure's \
             condition off every console says so"
    );
}

// ── Validation ──

#[test]
fn validation_rejects_the_tables_that_cannot_mean_anything() {
    let cases: Vec<(InfrastructureConfig, &str)> = vec![
        (
            InfrastructureConfig {
                condition_max: 0.0,
                ..Default::default()
            },
            "condition_max",
        ),
        (
            InfrastructureConfig {
                condition: Some(500.0),
                ..Default::default()
            },
            "condition",
        ),
        (
            InfrastructureConfig {
                decay_per_sec: -1.0,
                ..Default::default()
            },
            "decay_per_sec",
        ),
        (
            InfrastructureConfig {
                hull_damage_share: -1.0,
                ..Default::default()
            },
            "hull_damage_share",
        ),
        (
            InfrastructureConfig {
                thresholds: vec![ThresholdConfig {
                    label: None,
                    flag: "a".to_string(),
                    capacity: None,
                    fails_below: 40.0,
                    restores_above: None,
                }],
                ..Default::default()
            },
            "FRACTION",
        ),
        (
            InfrastructureConfig {
                thresholds: vec![ThresholdConfig {
                    label: None,
                    flag: "a".to_string(),
                    capacity: None,
                    fails_below: 0.5,
                    restores_above: Some(0.2),
                }],
                ..Default::default()
            },
            "inverts",
        ),
        (
            InfrastructureConfig {
                thresholds: vec![
                    ThresholdConfig {
                        label: None,
                        flag: "a".to_string(),
                        capacity: None,
                        fails_below: 0.5,
                        restores_above: None,
                    },
                    ThresholdConfig {
                        label: None,
                        flag: "a".to_string(),
                        capacity: None,
                        fails_below: 0.2,
                        restores_above: None,
                    },
                ],
                ..Default::default()
            },
            "twice",
        ),
        (
            InfrastructureConfig {
                capacities: vec![
                    CapacityConfig {
                        label: None,
                        id: "a".to_string(),
                        amount: 1,
                        ceiling: None,
                    },
                    CapacityConfig {
                        label: None,
                        id: "a".to_string(),
                        amount: 2,
                        ceiling: None,
                    },
                ],
                ..Default::default()
            },
            "twice",
        ),
    ];
    for (config, expected) in cases {
        let err = config
            .validate()
            .expect_err("this table is not authorable and must be refused");
        assert!(
            err.contains(expected),
            "the refusal must name what is wrong ({expected}), got {err}"
        );
    }
}

#[test]
fn the_shipped_shape_of_a_valid_table_passes_validation() {
    depot()
        .validate()
        .expect("the exemplar depot is authorable");
    InfrastructureConfig::default()
        .validate()
        .expect("an empty [infrastructure] table is authorable");
}

// ── Serde round-trip (the snapshot payload leans on this) ──

#[test]
fn live_state_round_trips_through_serde_with_its_flag_edges_intact() {
    let mut state = InfrastructureState::from_config(&depot());
    state.degrade(65.0);
    let bytes = toml::to_string(&state).expect("the live track serialises");
    let restored: InfrastructureState = toml::from_str(&bytes).expect("…and comes back");
    assert_eq!(
        restored, state,
        "a resumed structure must remember its condition AND which flags are currently \
             down — restoring the number alone would re-fire the crossing on the next tick"
    );
    assert_eq!(restored.flag("depot_transfer_capable"), Some(false));
}

#[test]
fn capacity_threshold_source_and_held_state_survive_a_snapshot_round_trip() {
    let config = InfrastructureConfig {
        capacities: vec![CapacityConfig {
            id: "reserve_fuel".to_string(),
            amount: 0,
            ceiling: Some(100),
            label: None,
        }],
        thresholds: vec![ThresholdConfig {
            flag: "transfer_primed".to_string(),
            capacity: Some("reserve_fuel".to_string()),
            fails_below: 0.4,
            restores_above: Some(0.5),
            label: None,
        }],
        ..Default::default()
    };
    let mut state = InfrastructureState::from_config(&config);
    state
        .adjust_capacity("reserve_fuel", 50)
        .expect("the capacity exists");

    let bytes = toml::to_string(&state).expect("the live track serialises");
    let mut restored: InfrastructureState =
        toml::from_str(&bytes).expect("the capacity-backed threshold comes back");
    assert_eq!(restored, state);
    assert_eq!(
        restored.thresholds()[0].capacity.as_deref(),
        Some("reserve_fuel")
    );
    assert_eq!(restored.flag("transfer_primed"), Some(true));
    assert_eq!(
            restored
                .adjust_capacity("reserve_fuel", -11)
                .expect("the restored capacity remains addressable")
                .1,
            vec![FlagChange {
                flag: "transfer_primed".to_string(),
                raised: false,
            }],
            "after resume, the threshold must still read the capacity rather than the unchanged condition track"
        );
}
