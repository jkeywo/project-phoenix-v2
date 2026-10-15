use super::*;
use std::collections::BTreeMap;

fn instrument_profile() -> SensingConfig {
    SensingConfig {
        focused: Some(FocusedConfig {
            system: crate::core::messages::SystemId("sensors".into()),
            work_secs: 2.0,
            history_limit: 64,
            unfinished_retention_secs: 0.0,
        }),
        broad: Some(BroadConfig {
            system: crate::core::messages::SystemId("sensor-radar".into()),
            sample_interval_secs: 0.5,
            identity_secs: 1.0,
            classification_secs: 2.0,
            assessment_secs: 4.0,
            position_error: 1.0,
            drift_error_per_sec: 0.5,
            track_retention_secs: 60.0,
        }),
        passive: None,
    }
}
#[test]
fn actual_broad_and_focused_instruments_use_distinct_owned_physical_reserves() {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    validate_sensing(&instrument_profile(), &config).unwrap();
    for mode in 0..5 {
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
            _ => bad
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
        }
        assert!(
            validate_sensing(&instrument_profile(), &bad).is_err(),
            "mode {mode}"
        );
    }
}
#[test]
fn optional_roles_do_not_create_free_instruments_or_merge_shared_identity() {
    let mut profile = instrument_profile();
    profile.broad = None;
    profile.validate().unwrap();
    profile.focused = None;
    assert!(profile.validate().is_err());
    profile = instrument_profile();
    profile.broad.as_mut().unwrap().system = profile.focused.as_ref().unwrap().system.clone();
    assert!(profile.validate().is_err());
}
#[test]
fn malformed_numeric_and_extension_profiles_refuse_before_any_runtime_state() {
    for mode in 0..5 {
        let mut value = instrument_profile();
        match mode {
            0 => value.focused.as_mut().unwrap().work_secs = 0.0,
            1 => value.focused.as_mut().unwrap().history_limit = 0,
            2 => value.broad.as_mut().unwrap().classification_secs = 0.5,
            3 => value.broad.as_mut().unwrap().position_error = f32::NAN,
            _ => value.broad.as_mut().unwrap().track_retention_secs = -1.0,
        }
        assert!(value.validate().is_err());
    }
    let mut value = serde_json::to_value(instrument_profile()).unwrap();
    value["focused"]["extension"] = serde_json::json!("read-private-world");
    assert!(serde_json::from_value::<SensingConfig>(value).is_err());
    assert_eq!(
        FocusedConfig::legacy(crate::core::messages::SystemId("sensors".into())).work_secs,
        1.0
    );
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
    let mut receiver = entity
        .ship_config
        .as_ref()
        .unwrap()
        .system(&crate::core::messages::SystemId("sensors".into()))
        .unwrap()
        .clone();
    receiver.id = id.clone();
    receiver.kind = PASSIVE_RECEIVER_KIND.into();
    // Serialise through TOML so optional state-tree fields keep their native shape.
    let tree = toml::to_string(&crate::science::sensing_ai::initial_tree()).unwrap();
    receiver.config = Some(toml::Value::Table(toml::map::Map::from_iter([(
        "ai".into(),
        toml::from_str(&tree).unwrap(),
    )])));
    entity.ship_config.as_mut().unwrap().systems.push(receiver);
    let mut consumer = entity
        .energy
        .as_ref()
        .unwrap()
        .consumers
        .iter()
        .find(|consumer| consumer.system.0 == "sensors")
        .unwrap()
        .clone();
    consumer.system = id.clone();
    entity.energy.as_mut().unwrap().consumers.push(consumer);
    entity
        .interior
        .as_mut()
        .unwrap()
        .placements
        .insert(id.clone(), "bridge".into());
    let profile = SensingConfig {
        focused: None,
        broad: None,
        passive: Some(PassiveConfig {
            system: id.clone(),
            mount: SensorMountConfig {
                facing_deg: 0.0,
                arc_deg: 360.0,
                range: 100.0,
            },
            sample_interval_secs: 0.5,
            min_signal_strength: 0.01,
            bearing_error_rad: 0.1,
        }),
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
    bad.passive.as_mut().unwrap().bearing_error_rad = 4.0;
    assert!(bad.validate().is_err());
    let mut bad = profile.clone();
    bad.passive.as_mut().unwrap().min_signal_strength = f32::NAN;
    assert!(bad.validate().is_err());
}
