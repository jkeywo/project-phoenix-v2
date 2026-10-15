use super::*;
use crate::ship::composition::CommandChannelConfig;
#[test]
fn finite_illumination_equipment_and_exact_generic_channels_roundtrip() {
    let config = IlluminationConfig {
        mount: MountConfig {
            facing_deg: 0.0,
            fire_arc_deg: 180.0,
            range: 200.0,
        },
        link_range: 150.0,
        minimum_transmission: 0.2,
        reading_ticks: 30,
    };
    config.validate().unwrap();
    let definitions = vec![
        CommandChannelConfig {
            name: "active".into(),
            kind: CommandChannelKind::Boolean,
            initial: None,
        },
        CommandChannelConfig {
            name: "subject".into(),
            kind: CommandChannelKind::EntityTarget,
            initial: None,
        },
    ];
    let mut channels = SystemCommandChannels::from_definitions(&definitions);
    channels
        .set("active", CommandChannelValue::Boolean(true))
        .unwrap();
    channels
        .set(
            "subject",
            CommandChannelValue::EntityTarget(Some("measured-contact".into())),
        )
        .unwrap();
    let restored = serde_json::from_str(&serde_json::to_string(&channels).unwrap()).unwrap();
    validate_channels(&restored, &definitions).unwrap();
    assert_eq!(channels, restored);
    assert!(validate_channels(&channels, &definitions[..1]).is_err());
    channels
        .0
        .insert("subject".into(), CommandChannelValue::Boolean(false));
    assert!(validate_channels(&channels, &definitions).is_err());
    for range in [0.0, -1.0, f32::INFINITY, f32::NAN] {
        assert!(IlluminationConfig {
            link_range: range,
            ..config
        }
        .validate()
        .is_err());
    }
    assert!(IlluminationConfig {
        minimum_transmission: 0.0,
        ..config
    }
    .validate()
    .is_err());
}

#[test]
fn composed_emitter_uses_exact_shared_channel_and_power_bindings_with_optional_turret() {
    let mut system: crate::ship::config::SystemInstanceConfig = serde_json::from_value(serde_json::json!({
        "id":"science-light", "kind":"illumination", "console":"science", "power_group":"sensors",
        "channel":[{"name":"active","kind":"boolean"},{"name":"subject","kind":"entity_target"}],
        "component":[{"id":"input","kind":"power_input"},{"id":"emitter","kind":"illumination_emitter", "power_input":"input", "enabled_channel":"active", "target_channel":"subject",
            "config":{"mount":{"facing_deg":0,"fire_arc_deg":90,"range":200},"link_range":150,"minimum_transmission":0.2,"reading_ticks":30}}]
    })).unwrap();
    crate::ship::composition::validate_composition(&system).unwrap();
    let before = system.clone();
    system
        .components
        .push(crate::ship::composition::SystemComponentConfig::Turret {
            id: "shared-turret".into(),
            power_input: "input".into(),
            config: phoenix_model::turret::TurretConfig {
                facing_deg: 0.0,
                traverse_arc_deg: 270.0,
                traverse_deg_per_sec: 45.0,
                aim_tolerance_deg: 2.0,
            },
        });
    crate::ship::composition::validate_composition(&system).unwrap();
    let restored = toml::from_str::<crate::ship::config::SystemInstanceConfig>(
        &toml::to_string(&system).unwrap(),
    )
    .unwrap();
    assert_eq!(restored, system);
    for fault in 0..4 {
        let mut bad = before.clone();
        match fault {
            0 => bad.channels[1].kind = CommandChannelKind::Boolean,
            1 => bad.components.push(bad.components[1].clone()),
            2 => bad.power_group = None,
            _ => {
                if let crate::ship::composition::SystemComponentConfig::IlluminationEmitter {
                    power_input,
                    ..
                } = &mut bad.components[1]
                {
                    *power_input = "absent".into();
                }
            }
        }
        assert!(crate::ship::composition::validate_composition(&bad).is_err());
    }
}

#[test]
fn shared_authored_guidance_profiles_support_explicit_false_without_granting_true_defaults() {
    let none: crate::entities::config::TorpedoesConfig =
        toml::from_str("seeker = false\nreceiver = false\n").unwrap();
    assert!(none.seeker.is_none() && none.receiver.is_none());
    assert!(toml::from_str::<crate::entities::config::TorpedoesConfig>("receiver = true").is_err());
    assert!(toml::from_str::<crate::entities::config::TorpedoesConfig>("seeker = true").is_err());
    let configured: crate::entities::config::TorpedoesConfig = toml::from_str(
        "[receiver]\nrange=200\nminimum_transmission=0.2\nmaximum_sample_age_secs=2\n",
    )
    .unwrap();
    assert!(!configured.receiver.unwrap().accept_external_illumination);
    let opted_in: crate::entities::config::TorpedoesConfig = toml::from_str("[receiver]\nrange=200\nminimum_transmission=0.2\nmaximum_sample_age_secs=2\naccept_external_illumination=true\n").unwrap();
    assert!(opted_in.receiver.unwrap().accept_external_illumination);
}
