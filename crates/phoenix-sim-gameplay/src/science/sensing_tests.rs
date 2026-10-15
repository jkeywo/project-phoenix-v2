use super::*;
use std::collections::BTreeMap;

#[test]
fn explicit_actuator_binding_selects_one_real_owner_without_merging_observations() {
    let mut config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let profile = config.sensing.clone().unwrap();
    validate_sensing(&profile, &config).unwrap();
    let mut missing = profile.clone();
    missing.control.science_target = None;
    assert!(validate_sensing(&missing, &config)
        .unwrap_err()
        .contains("science_target"));
    let mut wrong = profile.clone();
    wrong.control.combat_lock = Some(crate::core::messages::SystemId("sensors".into()));
    assert!(validate_sensing(&wrong, &config).is_err());
    let ship = config.ship_config.as_mut().unwrap();
    let mut second = ship
        .system(&crate::core::messages::SystemId("sensors".into()))
        .unwrap()
        .clone();
    second.id = crate::core::messages::SystemId("focused-two".into());
    ship.systems.push(second);
    let mut consumer = config
        .energy
        .as_ref()
        .unwrap()
        .consumers
        .iter()
        .find(|row| row.system.0 == "sensors")
        .unwrap()
        .clone();
    consumer.system = crate::core::messages::SystemId("focused-two".into());
    config.energy.as_mut().unwrap().consumers.push(consumer);
    let room = config.interior.as_ref().unwrap().placements
        [&crate::core::messages::SystemId("sensors".into())]
        .clone();
    config
        .interior
        .as_mut()
        .unwrap()
        .placements
        .insert(crate::core::messages::SystemId("focused-two".into()), room);
    let mut multiple = profile;
    let mut focused = multiple.focused[0].clone();
    focused.system = crate::core::messages::SystemId("focused-two".into());
    multiple.focused.push(focused);
    validate_sensing(&multiple, &config).unwrap();
    multiple.control.science_target = Some(crate::core::messages::SystemId("focused-two".into()));
    validate_sensing(&multiple, &config).unwrap();
    assert_eq!(multiple.focused.len(), 2);
}

#[test]
fn configured_producer_and_canonical_typed_scan_feed_are_required_together() {
    let mut config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    validate_scan_feeds(&config).unwrap();
    config.sensing.as_mut().unwrap().focused.clear();
    assert!(validate_scan_feeds(&config).is_err());
}

#[test]
fn focused_and_targeting_closed_trees_keep_policy_outputs_and_private_frames() {
    use crate::ai::state_tree::{compile, step, TreeRuntime};
    let program = compile(
        &crate::science::sensing_ai::focused_tree(),
        &crate::science::sensing_ai::focused_bindings(),
    )
    .unwrap();
    let mut runtime = TreeRuntime::default();
    for (tick, requested) in [(1, false), (2, true), (3, false)] {
        let result = step(
            &program,
            &BTreeMap::from([
                ("can_operate".into(), true),
                ("request_available".into(), requested),
                ("acquiring".into(), false),
                ("has_target".into(), requested),
            ]),
            &BTreeMap::from([
                ("designate".into(), false),
                ("enquire".into(), false),
                ("cancel".into(), false),
            ]),
            &mut runtime,
            tick,
        )
        .unwrap();
        assert_eq!(result.intents.get("enquire"), Some(&requested));
        assert_eq!(result.intents.get("designate"), Some(&true));
        assert_eq!(result.intents.get("cancel"), Some(&false));
    }
    compile(
        &crate::science::sensing_ai::targeting_tree(),
        &crate::science::sensing_ai::targeting_bindings(),
    )
    .unwrap();
}
#[test]
fn all_fitted_instruments_derive_their_actual_independent_coverage() {
    for hull in [
        "alliance_battleship",
        "alliance_courier",
        "alliance_cruiser",
        "alliance_destroyer",
        "dynasty_player_cruiser",
        "ship_harrow_cruiser",
        "ship_harrow_destroyer",
        "ship_harrow_patrol",
        "ship_harrow_tug",
        "ship_harrow_warhawk",
        "ship_requiem_courier",
        "station_axiom",
        "manual_trade_port",
    ] {
        let config =
            crate::test_fixtures::load_entity_config(&format!("assets/entities/{hull}.toml"))
                .unwrap();
        let profile = config.sensing.as_ref().unwrap();
        validate_sensing(profile, &config).unwrap();
        let ship = config.ship_config.as_ref().unwrap();
        let actual = ship
            .systems
            .iter()
            .filter(|s| {
                [HELM_RADAR_KIND, TACTICAL_RADAR_KIND, SENSOR_RADAR_KIND].contains(&s.kind.as_str())
            })
            .count();
        assert_eq!(actual, profile.broad.len(), "{hull}");
        for instrument in &profile.broad {
            assert!(broad_radar(&instrument.system, &config).unwrap().range > 0.0);
        }
    }
    let mut config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let profile = config.sensing.clone().unwrap();
    config.helm_console.as_mut().unwrap().radar = None;
    assert!(validate_sensing(&profile, &config).is_err());
    let mut wrong = profile;
    wrong.broad[0].system = crate::core::messages::SystemId("repair".into());
    assert!(validate_sensing(&wrong, &config).is_err());
}

fn instrument_profile() -> SensingConfig {
    SensingConfig {
        sharing: Vec::new(),
        control: SensingControlBindings {
            combat_lock: Some(crate::core::messages::SystemId("tactical-radar".into())),
            science_target: Some(crate::core::messages::SystemId("sensors".into())),
        },
        focused: vec![FocusedConfig {
            reading_ticks: 0,
            default_aim: Default::default(),
            aims: Vec::new(),
            system: crate::core::messages::SystemId("sensors".into()),
            work_secs: 2.0,
            history_limit: 64,
            unfinished_retention_secs: 0.0,
            forecast: None,
        }],
        broad: vec![BroadConfig {
            reading_ticks: 0,
            system: crate::core::messages::SystemId("sensor-radar".into()),
            sample_interval_secs: 0.5,
            identity_secs: 1.0,
            classification_secs: 2.0,
            assessment_secs: 4.0,
            position_error: 1.0,
            drift_error_per_sec: 0.5,
            track_retention_secs: 60.0,
        }],
        passive: vec![],
    }
}
#[test]
fn actual_broad_and_focused_instruments_use_distinct_owned_physical_reserves() {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let mut profile = instrument_profile();
    profile.passive = config
        .sensing
        .as_ref()
        .map(|profile| profile.passive.clone())
        .unwrap_or_default();
    validate_sensing(&profile, &config).unwrap();
    for mode in 0..6 {
        let mut bad = config.clone();
        match mode {
            0 => {
                bad.ship_config
                    .as_mut()
                    .unwrap()
                    .systems
                    .iter_mut()
                    .find(|system| system.id.0 == "sensors")
                    .unwrap()
                    .kind = SENSOR_RADAR_KIND.into()
            }
            1 => bad
                .energy
                .as_mut()
                .unwrap()
                .consumers
                .retain(|consumer| consumer.system.0 != "sensors"),
            2 => bad
                .interior
                .as_mut()
                .unwrap()
                .placements
                .retain(|id, _| id.0 != "sensors"),
            3 => {
                bad.ship_config
                    .as_mut()
                    .unwrap()
                    .systems
                    .iter_mut()
                    .find(|system| system.id.0 == "sensors")
                    .unwrap()
                    .console = None
            }
            4 => bad
                .ship_config
                .as_mut()
                .unwrap()
                .systems
                .iter_mut()
                .find(|system| system.id.0 == "sensors")
                .unwrap()
                .damage_components
                .retain(|component| {
                    component.binding != phoenix_model::damage::DamageBinding::Operation
                }),
            _ => bad.sensors_console = None,
        }
        assert!(validate_sensing(&profile, &bad).is_err(), "mode {mode}");
    }
}
#[test]
fn optional_roles_do_not_create_free_instruments_or_merge_shared_identity() {
    let mut profile = instrument_profile();
    profile.broad.clear();
    profile.validate().unwrap();
    profile.focused.clear();
    assert!(profile.validate().is_err());
    profile = instrument_profile();
    profile.broad.first_mut().unwrap().system = profile.focused.first().unwrap().system.clone();
    assert!(profile.validate().is_err());
}
#[test]
fn malformed_numeric_and_extension_profiles_refuse_before_any_runtime_state() {
    for mode in 0..5 {
        let mut value = instrument_profile();
        match mode {
            0 => value.focused.first_mut().unwrap().work_secs = 0.0,
            1 => value.focused.first_mut().unwrap().history_limit = 0,
            2 => value.broad.first_mut().unwrap().classification_secs = 0.5,
            3 => value.broad.first_mut().unwrap().position_error = f32::NAN,
            _ => value.broad.first_mut().unwrap().track_retention_secs = -1.0,
        }
        assert!(value.validate().is_err());
    }
    let mut value = serde_json::to_value(instrument_profile()).unwrap();
    value["focused"][0]["extension"] = serde_json::json!("read-private-world");
    assert!(serde_json::from_value::<SensingConfig>(value).is_err());
}
#[test]
fn passive_mount_delegates_exact_existing_weapon_geometry_and_refuses_invalid_arcs() {
    let geometry = SensorMountConfig {
        facing_deg: 90.0,
        arc_deg: 90.0,
        range: 100.0,
    };
    geometry.validate().unwrap();
    let adapter = mount(&geometry);
    let existing = crate::weapons::mount::MountConfig {
        facing_deg: 90.0,
        fire_arc_deg: 90.0,
        range: 100.0,
    };
    for (x, z, yaw) in [
        (1.0, 0.0, 0.0),
        (-1.0, 0.0, 0.0),
        (0.0, -100.0, 1.0),
        (200.0, 0.0, 0.0),
    ] {
        assert_eq!(
            adapter.geometry(x, z, 0.0, 0.0, yaw),
            existing.geometry(x, z, 0.0, 0.0, yaw)
        );
    }
    for arc in [0.0, 361.0, f32::NAN] {
        let mut invalid = geometry.clone();
        invalid.arc_deg = arc;
        assert!(invalid.validate().is_err());
    }
}
#[test]
fn ordinary_passive_tree_reads_only_local_machinery_and_writes_its_own_intent() {
    let bindings = crate::science::sensing_ai::bindings();
    let program =
        crate::ai::state_tree::compile(&crate::science::sensing_ai::initial_tree(), &bindings)
            .unwrap();
    let mut memory = crate::ai::state_tree::TreeRuntime::default();
    for (tick, available) in [(1, false), (2, true), (3, false)] {
        let step = crate::ai::state_tree::step(
            &program,
            &BTreeMap::from([("enabled".into(), true), ("can_operate".into(), available)]),
            &BTreeMap::from([("operate".into(), false)]),
            &mut memory,
            tick,
        )
        .unwrap();
        assert_eq!(step.intents.get("operate"), Some(&available));
    }
    assert_eq!(bindings.facts.len(), 2);
    assert_eq!(bindings.channels.len(), 1);
    let source = serde_json::to_string(&crate::science::sensing_ai::initial_tree())
        .unwrap()
        .replace("can_operate", "hidden_emitter_uuid");
    let bad = serde_json::from_str(&source).unwrap();
    assert!(crate::ai::state_tree::compile(&bad, &bindings).is_err());
}

#[test]
fn passive_receiver_needs_own_hardware_and_closed_program_without_target_truth() {
    let mut entity =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let id = crate::core::messages::SystemId("passive-receiver".into());
    assert_eq!(
        entity
            .ship_config
            .as_ref()
            .unwrap()
            .systems
            .iter()
            .filter(|system| system.id == id)
            .count(),
        1
    );
    assert_eq!(
        entity
            .energy
            .as_ref()
            .unwrap()
            .consumers
            .iter()
            .filter(|consumer| consumer.system == id)
            .count(),
        1
    );
    let profile = SensingConfig {
        sharing: Vec::new(),
        control: Default::default(),
        focused: vec![],
        broad: vec![],
        passive: vec![PassiveConfig {
            system: id.clone(),
            mount: SensorMountConfig {
                facing_deg: 0.0,
                arc_deg: 360.0,
                range: 100.0,
            },
            sample_interval_secs: 0.5,
            min_signal_strength: 0.01,
            bearing_error_rad: 0.1,
        }],
    };
    validate_sensing(&profile, &entity).unwrap();
    entity
        .ship_config
        .as_mut()
        .unwrap()
        .systems
        .iter_mut()
        .find(|system| system.id == id)
        .unwrap()
        .config = None;
    assert!(validate_sensing(&profile, &entity)
        .unwrap_err()
        .contains("state tree"));
    let mut bad = profile.clone();
    bad.passive.first_mut().unwrap().bearing_error_rad = 4.0;
    assert!(bad.validate().is_err());
    let mut bad = profile.clone();
    bad.passive.first_mut().unwrap().min_signal_strength = f32::NAN;
    assert!(bad.validate().is_err());
}
