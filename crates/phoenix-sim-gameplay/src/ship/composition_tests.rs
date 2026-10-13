use super::*;
use crate::ship::config::SystemInstanceConfig;

fn dock() -> SystemInstanceConfig {
    let entity: crate::entities::config::EntityConfig =
        crate::entities::config::EntityConfig::from_toml(include_str!(
            "../../../../assets/entities/dock_probe.toml"
        ))
        .unwrap();
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
    let source = include_str!("../../../../assets/entities/dock_probe.toml");
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
