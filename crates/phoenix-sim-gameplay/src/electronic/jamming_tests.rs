use super::*;
fn profile() -> JammingConfig {
    JammingConfig {
        mount: MountConfig {
            facing_deg: 0.0,
            fire_arc_deg: 60.0,
            range: 100.0,
        },
        falloff: FieldFalloff::Linear,
        strength: 8.0,
        maximum_sample_age_secs: 2.0,
        reading_ticks: 30,
    }
}
#[test]
fn shared_physical_cone_falloff_path_and_paid_fraction_apply_exactly_once() {
    let p = profile();
    assert_eq!(
        exposure(&p, [0.0; 3], 0.0, [0.0, 0.0, -50.0], 0.5, 0.25),
        0.5
    );
    assert_eq!(
        exposure(&p, [0.0; 3], 0.0, [0.0; 3], 1.0, 1.0),
        8.0,
        "self is not immune"
    );
    for outside in [[0.0, 0.0, 50.0], [0.0, 50.0, 0.0], [0.0, 0.0, -101.0]] {
        assert_eq!(exposure(&p, [0.0; 3], 0.0, outside, 1.0, 1.0), 0.0);
    }
    assert_eq!(
        exposure(&p, [0.0; 3], 0.0, [0.0, 0.0, -50.0], 0.0, 1.0),
        0.0
    );
    assert_eq!(
        exposure(&p, [0.0; 3], 0.0, [0.0, 0.0, -50.0], 1.0, 0.0),
        0.0
    );
    let mut edge = p;
    edge.falloff = FieldFalloff::Step;
    edge.mount.fire_arc_deg = 90.0;
    assert!(exposure(&edge, [0.0; 3], 0.0, [1.0, 0.0, -1.0], 1.0, 1.0) > 0.0);
    assert!(exposure(&edge, [0.0; 3], 0.0, [0.0, 0.0, -100.0], 1.0, 1.0) > 0.0);
    assert_eq!(
        exposure(&edge, [0.0; 3], 0.0, [1.01, 0.0, -1.0], 1.0, 1.0),
        0.0
    );
    let mut all = p;
    all.mount.fire_arc_deg = 360.0;
    assert!(exposure(&all, [0.0; 3], 0.0, [0.0, 50.0, 0.0], 1.0, 1.0) > 0.0);
}
#[test]
fn original_sample_time_and_finite_authored_boundaries_are_not_receipt_time() {
    let p = profile();
    assert!(p.sample_fresh(100, 160, 1.0 / 30.0));
    assert!(!p.sample_fresh(100, 161, 1.0 / 30.0));
    assert!(!p.sample_fresh(161, 160, 1.0 / 30.0));
    let mut extreme = p;
    extreme.strength = f32::MAX;
    extreme.mount.range = f32::MAX;
    extreme.mount.fire_arc_deg = 360.0;
    assert!(exposure(&extreme, [0.0; 3], 0.0, [f32::MAX, 0.0, 0.0], 1.0, 1.0).is_finite());
    for invalid in [f32::NAN, 0.0, -1.0, f32::INFINITY] {
        let mut bad = p;
        bad.strength = invalid;
        assert!(bad.validate().is_err());
        assert_eq!(exposure(&bad, [0.0; 3], 0.0, [0.0; 3], 1.0, 1.0), 0.0);
    }
    let value = serde_json::to_value(p).unwrap();
    let mut unknown = value.clone();
    unknown["victim_count"] = 1.into();
    assert!(serde_json::from_value::<JammingConfig>(unknown).is_err());
    assert_eq!(serde_json::from_value::<JammingConfig>(value).unwrap(), p);
}

#[test]
fn actual_jamming_composition_shares_typed_intent_input_and_single_optional_mount() {
    use crate::ship::composition::{
        jamming_composition, validate_composition, SystemComponentConfig,
    };
    let system:crate::ship::config::SystemInstanceConfig=serde_json::from_value(serde_json::json!({
        "id":"comms-jammer", "kind":"jamming", "console":"comms", "power_group":"electronics",
        "channel":[{"name":"active","kind":"boolean"},{"name":"subject","kind":"entity_target"}],
        "component":[{"id":"input","kind":"power_input"},{"id":"emitter","kind":"jamming_emitter","power_input":"input","enabled_channel":"active","target_channel":"subject",
            "config":{"mount":{"facing_deg":0,"fire_arc_deg":60,"range":100},"falloff":"linear","strength":8,"maximum_sample_age_secs":2,"reading_ticks":30}}]
    })).unwrap();
    validate_composition(&system).unwrap();
    let binding = jamming_composition(&system.components).unwrap();
    assert_eq!(binding.enabled_channel, "active");
    assert_eq!(
        bindings(binding.enabled_channel, binding.target_channel).channels,
        ["active".to_owned(), "subject".to_owned()]
            .into_iter()
            .collect()
    );
    let restored = toml::from_str::<crate::ship::config::SystemInstanceConfig>(
        &toml::to_string(&system).unwrap(),
    )
    .unwrap();
    assert_eq!(restored, system);
    let mut mounted = system.clone();
    mounted.components.push(SystemComponentConfig::Turret {
        id: "mount".into(),
        power_input: "input".into(),
        config: phoenix_model::turret::TurretConfig {
            facing_deg: 0.0,
            traverse_arc_deg: 270.0,
            traverse_deg_per_sec: 30.0,
            aim_tolerance_deg: 2.0,
        },
    });
    validate_composition(&mounted).unwrap();
    for fault in 0..5 {
        let mut bad = system.clone();
        match fault {
            0 => bad.channels[1].kind = crate::ship::composition::CommandChannelKind::Boolean,
            1 => bad.components.push(bad.components[1].clone()),
            2 => bad.power_group = None,
            3 => {
                if let SystemComponentConfig::JammingEmitter { power_input, .. } =
                    &mut bad.components[1]
                {
                    *power_input = "missing".into()
                }
            }
            _ => bad.kind = crate::ship::system_registry::ILLUMINATION_KIND.into(),
        }
        assert!(validate_composition(&bad).is_err(), "fault{fault}");
    }
}
