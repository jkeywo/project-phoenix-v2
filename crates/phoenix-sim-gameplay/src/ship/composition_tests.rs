use super::*;
use crate::ship::config::SystemInstanceConfig;

fn dock() -> SystemInstanceConfig {
    let entity = crate::test_fixtures::load_entity_config("assets/entities/dock_probe.toml")
        .expect("shipped Dock probe composes and validates");
    entity
        .ship_config
        .unwrap()
        .systems
        .into_iter()
        .find(|s| s.kind == "dock")
        .unwrap()
}

#[test]
fn authored_dock_resolves_typed_components_and_roundtrips() {
    let system = dock();
    validate_composition(&system).unwrap();
    let encoded = toml::to_string(&system).unwrap();
    assert_eq!(
        toml::from_str::<SystemInstanceConfig>(&encoded).unwrap(),
        system
    );
    assert_eq!(
        dock_composition(&system.components).unwrap().power_input,
        "power"
    );
}

#[test]
fn channel_writes_are_typed_and_do_not_create_unknown_intent() {
    let system = dock();
    let mut channels = SystemCommandChannels::from_definitions(&system.channels);
    assert!(!channels.boolean("enabled"));
    assert_eq!(channels.target("target"), None);
    channels
        .set("enabled", CommandChannelValue::Boolean(true))
        .unwrap();
    channels
        .set(
            "target",
            CommandChannelValue::EntityTarget(Some("berth".into())),
        )
        .unwrap();
    let before = channels.clone();
    assert!(channels
        .set("target", CommandChannelValue::Boolean(false))
        .is_err());
    assert!(channels
        .set("unknown", CommandChannelValue::Boolean(true))
        .is_err());
    assert_eq!(channels, before);
    channels
        .set("enabled", CommandChannelValue::Boolean(false))
        .unwrap();
    assert!(!channels.boolean("enabled"));
    assert_eq!(channels.target("target"), Some("berth"));
    let json = serde_json::to_string(&channels).unwrap();
    assert_eq!(
        serde_json::from_str::<SystemCommandChannels>(&json).unwrap(),
        channels
    );
}

#[test]
fn mounted_weapon_and_receiver_share_one_supported_turret_and_existing_owner_intent() {
    let mut system = dock();
    system.components = vec![
        SystemComponentConfig::PowerInput { id: "power".into() },
        SystemComponentConfig::Turret {
            id: "turret".into(),
            power_input: "power".into(),
            config: phoenix_model::turret::TurretConfig {
                facing_deg: 0.0,
                traverse_arc_deg: 270.0,
                traverse_deg_per_sec: 30.0,
                aim_tolerance_deg: 1.0,
            },
        },
    ];
    system.channels.clear();
    for kind in [
        "phaser_bank",
        "blaster_bank",
        "torpedo_tube",
        "passive_receiver",
    ] {
        system.kind = kind.into();
        validate_composition(&system).unwrap();
        let decoded: SystemInstanceConfig =
            toml::from_str(&toml::to_string(&system).unwrap()).unwrap();
        assert_eq!(decoded, system);
        assert_eq!(
            turret_composition(&system.components).unwrap().power_input,
            "power"
        );
    }
    let good = system.clone();
    system.components.push(system.components[1].clone());
    assert!(validate_composition(&system).is_err());
    system = good.clone();
    system.channels.push(CommandChannelConfig {
        name: "unused-target".into(),
        kind: CommandChannelKind::EntityTarget,
        initial: None,
    });
    assert!(validate_composition(&system).is_err());
    system = good.clone();
    system.components.remove(0);
    assert!(validate_composition(&system).is_err());
    system = good.clone();
    system.kind = "repair".into();
    assert!(validate_composition(&system).is_err());
    system = good;
    system.components.clear();
    system.channels.clear();
    validate_composition(&system).unwrap();
}

#[test]
fn invalid_component_and_channel_bindings_are_rejected() {
    let original = dock();
    let mut invalid = original.clone();
    invalid.components.push(invalid.components[0].clone());
    assert!(validate_composition(&invalid)
        .unwrap_err()
        .contains("duplicate component"));
    let mut invalid = original.clone();
    invalid.channels[0].kind = CommandChannelKind::EntityTarget;
    assert!(validate_composition(&invalid)
        .unwrap_err()
        .contains("channel bindings"));
    let mut invalid = original.clone();
    invalid
        .components
        .retain(|c| !matches!(c, SystemComponentConfig::PowerInput { .. }));
    assert!(validate_composition(&invalid)
        .unwrap_err()
        .contains("missing power_input"));
    let mut invalid = original.clone();
    invalid.power_group = None;
    assert!(validate_composition(&invalid)
        .unwrap_err()
        .contains("power_group"));
    let mut invalid = original;
    invalid.components.clear();
    assert!(validate_composition(&invalid)
        .unwrap_err()
        .contains("increment 2 format"));
}

#[test]
fn component_identity_and_channels_are_local_to_each_system() {
    let first = dock();
    let mut second = first.clone();
    second.id = crate::core::messages::SystemId("second-dock".into());
    validate_composition(&first).unwrap();
    validate_composition(&second).unwrap();
    let mut first_intent = SystemCommandChannels::from_definitions(&first.channels);
    let second_intent = SystemCommandChannels::from_definitions(&second.channels);
    first_intent
        .set("enabled", CommandChannelValue::Boolean(true))
        .unwrap();
    assert!(!second_intent.boolean("enabled"));
}

#[test]
fn active_legacy_tuning_has_an_actionable_conversion_error() {
    let resolved =
        crate::test_fixtures::resolve_from_disk("assets/entities/dock_probe.toml").unwrap();
    let source = toml::to_string(&resolved.value).unwrap();
    let system = dock();
    let legacy = toml::to_string(&dock_composition(&system.components).unwrap().config).unwrap();
    let error =
        crate::entities::config::EntityConfig::from_toml(&format!("{source}\n[dock]\n{legacy}"))
            .unwrap_err()
            .to_string();
    assert!(
        error.contains("remove the competing top-level [dock]"),
        "{error}"
    );
}

#[test]
fn non_finite_mechanical_values_are_refused_before_spawn() {
    let mut system = dock();
    if let SystemComponentConfig::DockMating { config, .. } = &mut system.components[1] {
        config.approach_speed = f32::INFINITY;
    }
    assert!(validate_composition(&system)
        .unwrap_err()
        .contains("finite"));
}

#[test]
fn dock_tree_declaration_and_channel_bindings_are_checked_at_load() {
    use crate::entities::config::EntityConfig;
    let resolved =
        crate::test_fixtures::resolve_from_disk("assets/entities/dock_probe.toml").unwrap();
    let source = toml::to_string(&resolved.value).unwrap();
    let mut missing: toml::Value = toml::from_str(&source).unwrap();
    let systems = missing["system"].as_array_mut().unwrap();
    let dock = systems
        .iter_mut()
        .find(|s| s["kind"].as_str() == Some("dock"))
        .unwrap();
    dock["config"].as_table_mut().unwrap().remove("ai");
    let error = EntityConfig::from_toml(&toml::to_string(&missing).unwrap())
        .unwrap_err()
        .to_string();
    assert!(error.contains("requires config.ai"), "{error}");
    assert!(error.contains("original game version"), "{error}");

    let mut renamed: toml::Value = toml::from_str(&source).unwrap();
    let dock = renamed["system"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|s| s["kind"].as_str() == Some("dock"))
        .unwrap();
    dock["channel"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["name"].as_str() == Some("enabled"))
        .unwrap()["name"] = "hold".into();
    dock["component"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["kind"].as_str() == Some("dock_mating"))
        .unwrap()["enabled_channel"] = "hold".into();
    let error = EntityConfig::from_toml(&toml::to_string(&renamed).unwrap())
        .unwrap_err()
        .to_string();
    assert!(error.contains("config.ai"), "{error}");
    // An explicit idle definition is valid and is not replaced by built-in behavior.
    let dock = renamed["system"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|s| s["kind"].as_str() == Some("dock"))
        .unwrap();
    dock["config"]["ai"] = toml::from_str::<toml::Value>(
        r#"root = "idle"
[[tree]]
id = "idle"
initial = "idle"
[[tree.state]]
id = "idle"
"#,
    )
    .unwrap();
    EntityConfig::from_toml(&toml::to_string(&renamed).unwrap()).unwrap();
}

#[test]
fn typed_authored_initial_values_create_only_the_canonical_intent_and_do_not_rewrite_saved_intent()
{
    let mut system = dock();
    for channel in &mut system.channels {
        channel.initial = Some(match channel.kind {
            CommandChannelKind::Boolean => CommandChannelValue::Boolean(true),
            CommandChannelKind::EntityTarget => {
                CommandChannelValue::EntityTarget(Some("known-berth".into()))
            }
            CommandChannelKind::SystemPriority => unreachable!(),
        });
    }
    validate_composition(&system).unwrap();
    let source = toml::to_string(&system).unwrap();
    let decoded: SystemInstanceConfig = toml::from_str(&source).unwrap();
    assert_eq!(decoded, system);
    let mut intent = SystemCommandChannels::from_definitions(&decoded.channels);
    assert!(intent.boolean("enabled"));
    assert_eq!(intent.target("target"), Some("known-berth"));
    intent
        .set("enabled", CommandChannelValue::Boolean(false))
        .unwrap();
    let saved = serde_json::to_vec(&intent).unwrap();
    let restored: SystemCommandChannels = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored, intent);
    assert!(!restored.boolean("enabled"));
    assert!(SystemCommandChannels::from_definitions(&decoded.channels).boolean("enabled"));
    let mut malformed = system.clone();
    malformed.channels[0].initial = Some(match malformed.channels[0].kind {
        CommandChannelKind::Boolean => CommandChannelValue::EntityTarget(None),
        _ => CommandChannelValue::Boolean(false),
    });
    assert!(validate_composition(&malformed).is_err());
    assert!(serde_json::from_value::<CommandChannelValue>(
        serde_json::json!({"kind":"boolean","value":true,"ignored":true})
    )
    .is_err());
}
#[test]
fn system_priority_channel_is_bounded_typed_and_transactional() {
    use crate::core::messages::SystemId;
    let priorities = vec![SystemId("comms".into()), SystemId("sensors".into())];
    let definition = CommandChannelConfig {
        name: "priorities".into(),
        kind: CommandChannelKind::SystemPriority,
        initial: Some(CommandChannelValue::SystemPriority(priorities.clone())),
    };
    definition.validate().unwrap();
    assert_eq!(
        toml::from_str::<CommandChannelConfig>(&toml::to_string(&definition).unwrap()).unwrap(),
        definition
    );
    let mut intent = SystemCommandChannels::from_definitions(&[definition]);
    assert_eq!(intent.priorities("priorities"), priorities);
    let before = intent.clone();
    for value in [
        CommandChannelValue::Boolean(true),
        CommandChannelValue::SystemPriority(vec![SystemId("comms".into()); 2]),
        CommandChannelValue::SystemPriority(vec![SystemId("comms".into()); 65]),
    ] {
        assert!(intent.set("priorities", value).is_err());
        assert_eq!(intent, before);
    }
    let plain = CommandChannelConfig {
        name: "priorities".into(),
        kind: CommandChannelKind::SystemPriority,
        initial: None,
    };
    assert!(SystemCommandChannels::from_definitions(&[plain])
        .priorities("priorities")
        .is_empty());
}

#[test]
fn actual_bay_composition_uses_one_distinct_paid_owner_and_closed_component_links() {
    let mut system = dock();
    system.kind = super::super::system_registry::CRAFT_BAY_KIND.into();
    system.id = crate::core::messages::SystemId("actual-probe-bay".into());
    system.channels.clear();
    system.components = vec![
        SystemComponentConfig::PowerInput {
            id: "intake".into(),
        },
        SystemComponentConfig::CraftBay {
            id: "bay".into(),
            power_input: "intake".into(),
            config: phoenix_model::craft::BayConfig {
                compartment: "probe-room".into(),
                capacity_space: 2,
                launch_work_secs: 2.0,
                recovery_work_secs: 3.0,
                recovery_range: 8.0,
                maximum_parent_speed: 0.1,
                release_offset: [0.0, 0.0, -3.0],
            },
        },
    ];
    validate_composition(&system).unwrap();
    let encoded = toml::to_string(&system).unwrap();
    assert_eq!(
        toml::from_str::<SystemInstanceConfig>(&encoded).unwrap(),
        system
    );
    for fault in 0..4 {
        let mut invalid = system.clone();
        match fault {
            0 => invalid.id.0 = "x".repeat(129),
            1 => {
                invalid.components.remove(0);
            }
            2 => invalid.power_group = None,
            _ => invalid.components.push(SystemComponentConfig::PowerInput {
                id: "unused".into(),
            }),
        };
        assert!(validate_composition(&invalid).is_err(), "case {fault}");
    }
}
