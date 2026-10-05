use super::*;

/// Every live region effect kind maps onto an authorable name, and the map
/// is one-to-one (issue #1026, relocated with the vocabulary in #1166). A
/// `match` on `RegionEffectKind`, so a new hazard will not compile until it
/// is authorable; the pinned pairs stop the map being made total by
/// pointing two kinds at one name.
#[test]
fn every_live_region_effect_maps_onto_an_authorable_name() {
    let pairs = [
        (
            RegionEffectKind::DamageZone {
                dps: 1.0,
                shield_pierce: 0.0,
            },
            RegionEffectName::DamageZone,
        ),
        (
            RegionEffectKind::SlowZone {
                thrust_modifier: None,
                yaw_rate_modifier: None,
            },
            RegionEffectName::SlowZone,
        ),
        (
            RegionEffectKind::BlocksImpulse,
            RegionEffectName::BlocksImpulse,
        ),
        (
            RegionEffectKind::RadarDampening { multiplier: 0.5 },
            RegionEffectName::RadarDampening,
        ),
        (RegionEffectKind::CommsJam, RegionEffectName::CommsJam),
        (RegionEffectKind::SensorBlind, RegionEffectName::SensorBlind),
        (
            RegionEffectKind::NebulaFog {
                color: [0.0; 3],
                density: 0.01,
            },
            RegionEffectName::NebulaFog,
        ),
    ];
    assert_eq!(
        pairs.len(),
        RegionEffectName::ALL.len(),
        "every authorable name is reachable from a live region effect, and vice versa — a \
             hazard band nobody can name is a hazard nothing can be told about"
    );
    for (kind, name) in pairs {
        assert_eq!(region_effect_name(&kind), name);
    }
}

// ── RegionEffectKind serde round-trips live in crates/phoenix-simulation/src/core/codec.rs ──────
// (moved there as part of issue #524 to enforce the codec-only JSON rule)

// ── RegionEffectsConfig tests ─────────────────────────────────

#[test]
fn effects_config_default_is_empty() {
    let cfg = RegionEffectsConfig::default();
    assert!(cfg.is_empty());
    assert!(cfg.to_kinds().is_empty());
}

#[test]
fn effects_config_damage_zone() {
    let cfg = RegionEffectsConfig {
        damage_zone: Some(DamageZoneEffect {
            damage_per_second: 15.0,
            shield_pierce: 0.0,
        }),
        ..Default::default()
    };
    assert!(!cfg.is_empty());
    let kinds = cfg.to_kinds();
    assert_eq!(kinds.len(), 1);
    assert_eq!(
        kinds[0],
        RegionEffectKind::DamageZone {
            dps: 15.0,
            shield_pierce: 0.0
        }
    );
}

#[test]
fn effects_config_to_kinds_aggregates_all() {
    let cfg = RegionEffectsConfig {
        damage_zone: Some(DamageZoneEffect {
            damage_per_second: 10.0,
            shield_pierce: 0.0,
        }),
        slow_zone: Some(SlowZoneEffect {
            thrust_modifier: Some(0.5),
            yaw_rate_modifier: Some(-0.3),
        }),
        blocks_impulse: Some(BlocksImpulseEffect {}),
        radar_dampening: Some(RadarDampeningEffect {
            range_modifier: 0.3,
        }),
        comms_jammed: Some(CommsJamEffect {}),
        sensor_blind: Some(SensorBlindEffect {}),
        nebula_fog: Some(NebulaFogEffect {
            color: [0.25, 0.08, 0.32],
            density: 0.008,
        }),
    };
    let kinds = cfg.to_kinds();
    assert_eq!(kinds.len(), 7);
    assert_eq!(
        kinds[6],
        RegionEffectKind::NebulaFog {
            color: [0.25, 0.08, 0.32],
            density: 0.008
        }
    );
}

#[test]
fn effects_config_toml_old_dps_key_still_parses_via_alias() {
    let toml_str = r#"
[effects]
[effects.damage_zone]
dps = 8.0
"#;
    #[derive(Deserialize)]
    struct Wrap {
        effects: RegionEffectsConfig,
    }
    let wrap: Wrap = toml::from_str(toml_str).unwrap();
    assert_eq!(wrap.effects.damage_zone.unwrap().damage_per_second, 8.0);
}

#[test]
fn effects_config_toml_round_trip_comms_jammed() {
    let toml_str = r#"
[effects]
[effects.comms_jammed]
"#;
    #[derive(Deserialize)]
    struct Wrap {
        effects: RegionEffectsConfig,
    }
    let wrap: Wrap = toml::from_str(toml_str).unwrap();
    assert!(wrap.effects.comms_jammed.is_some());
}

#[test]
fn effects_config_toml_old_comms_jam_key_still_parses_via_alias() {
    let toml_str = r#"
[effects]
[effects.comms_jam]
"#;
    #[derive(Deserialize)]
    struct Wrap {
        effects: RegionEffectsConfig,
    }
    let wrap: Wrap = toml::from_str(toml_str).unwrap();
    assert!(wrap.effects.comms_jammed.is_some());
}

#[test]
fn effects_config_toml_range_modifier_key_parses() {
    let toml_str = r#"
[effects]
[effects.radar_dampening]
range_modifier = 0.4
"#;
    #[derive(Deserialize)]
    struct Wrap {
        effects: RegionEffectsConfig,
    }
    let wrap: Wrap = toml::from_str(toml_str).unwrap();
    assert_eq!(wrap.effects.radar_dampening.unwrap().range_modifier, 0.4);
}

#[test]
fn effects_config_toml_nebula_fog_parses() {
    let toml_str = r#"
[effects]
[effects.nebula_fog]
color = [0.25, 0.08, 0.32]
density = 0.008
"#;
    #[derive(Deserialize)]
    struct Wrap {
        effects: RegionEffectsConfig,
    }
    let wrap: Wrap = toml::from_str(toml_str).unwrap();
    let fog = wrap.effects.nebula_fog.unwrap();
    assert_eq!(fog.color, [0.25, 0.08, 0.32]);
    assert!((fog.density - 0.008).abs() < 1e-6);
}

#[test]
fn effects_config_toml_old_multiplier_key_still_parses_via_alias() {
    let toml_str = r#"
[effects]
[effects.radar_dampening]
multiplier = 0.4
"#;
    #[derive(Deserialize)]
    struct Wrap {
        effects: RegionEffectsConfig,
    }
    let wrap: Wrap = toml::from_str(toml_str).unwrap();
    assert_eq!(wrap.effects.radar_dampening.unwrap().range_modifier, 0.4);
}

// ── The sign of a dampening bonus ─────────────────────────────────────

#[test]
fn a_negative_range_modifier_dampens_and_a_positive_one_does_not() {
    assert!(RadarDampeningEffect {
        range_modifier: -1.0
    }
    .dampens());
    assert!(!RadarDampeningEffect {
        range_modifier: 0.5
    }
    .dampens());
    // Zero is a modifier that modifies nothing, which on this effect is an
    // authoring mistake rather than a neutral default.
    assert!(!RadarDampeningEffect {
        range_modifier: 0.0
    }
    .dampens());
}

// ── The sign of a slow-zone bonus ─────────────────────────────────────

#[test]
fn negative_slow_zone_bonuses_slow_and_positive_ones_do_not() {
    let slow = |thrust, yaw| {
        SlowZoneEffect {
            thrust_modifier: thrust,
            yaw_rate_modifier: yaw,
        }
        .slows()
    };

    assert!(slow(Some(-1.0), Some(-0.5)));
    assert!(slow(Some(-1.0), None));
    assert!(slow(None, Some(-0.5)));

    // The shipped shape of the defect: numbers that READ as multipliers.
    assert!(!slow(Some(0.5), Some(0.6)));
    assert!(!slow(Some(0.6), Some(0.7)));
    // One good axis does not excuse the other.
    assert!(!slow(Some(-1.0), Some(0.6)));
    assert!(!slow(Some(0.5), Some(-0.6)));
    // Zero on a present axis modifies nothing, which on this axis is a
    // mistake rather than a default.
    assert!(!slow(Some(0.0), None));

    // …but a slow zone that authors NEITHER number is the operations
    // presence marker, and has no sign to get wrong.
    assert!(slow(None, None));
}
