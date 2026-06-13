use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RegionEffectKind {
    DamageZone {
        dps: f32,
        shield_pierce: f32,
    },
    SlowZone {
        thrust_modifier: Option<f32>,
        yaw_rate_modifier: Option<f32>,
    },
    BlocksImpulse,
    RadarDampening {
        multiplier: f32,
    },
    CommsJam,
    SensorBlind,
    NebulaFog {
        color: [f32; 3],
        density: f32,
    },
}

// ── Effect config types for TOML entity templates ─────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageZoneEffect {
    #[serde(alias = "dps")]
    pub damage_per_second: f32,
    /// Fraction of damage that bypasses shields and goes straight to the
    /// hull. Clamped to `[0.0, 1.0]` at apply time. Default `0.0` — all
    /// damage is mitigated by the facing shield quadrant.
    #[serde(default)]
    pub shield_pierce: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlowZoneEffect {
    pub thrust_modifier: Option<f32>,
    pub yaw_rate_modifier: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlocksImpulseEffect {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadarDampeningEffect {
    #[serde(alias = "multiplier")]
    pub range_modifier: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommsJamEffect {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SensorBlindEffect {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NebulaFogEffect {
    /// RGB fog/cloud colour in linear 0–1 range.
    pub color: [f32; 3],
    /// Exponential fog density. Higher = thicker. Typical range: 0.002–0.02.
    pub density: f32,
}

/// TOML-deserializable effects block.
///
/// Each field corresponds to an optional `[effects.*]` sub-table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct RegionEffectsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage_zone: Option<DamageZoneEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow_zone: Option<SlowZoneEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks_impulse: Option<BlocksImpulseEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar_dampening: Option<RadarDampeningEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "comms_jam")]
    pub comms_jammed: Option<CommsJamEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_blind: Option<SensorBlindEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nebula_fog: Option<NebulaFogEffect>,
}

impl RegionEffectsConfig {
    /// Returns `true` when no effect sub-tables are present.
    pub fn is_empty(&self) -> bool {
        self.damage_zone.is_none()
            && self.slow_zone.is_none()
            && self.blocks_impulse.is_none()
            && self.radar_dampening.is_none()
            && self.comms_jammed.is_none()
            && self.sensor_blind.is_none()
            && self.nebula_fog.is_none()
    }

    /// Convert to a `Vec<RegionEffectKind>` for runtime use.
    pub fn to_kinds(&self) -> Vec<RegionEffectKind> {
        let mut kinds = Vec::new();
        if let Some(z) = &self.damage_zone {
            kinds.push(RegionEffectKind::DamageZone {
                dps: z.damage_per_second,
                shield_pierce: z.shield_pierce,
            });
        }
        if let Some(z) = &self.slow_zone {
            kinds.push(RegionEffectKind::SlowZone {
                thrust_modifier: z.thrust_modifier,
                yaw_rate_modifier: z.yaw_rate_modifier,
            });
        }
        if self.blocks_impulse.is_some() {
            kinds.push(RegionEffectKind::BlocksImpulse);
        }
        if let Some(r) = &self.radar_dampening {
            kinds.push(RegionEffectKind::RadarDampening {
                multiplier: r.range_modifier,
            });
        }
        if self.comms_jammed.is_some() {
            kinds.push(RegionEffectKind::CommsJam);
        }
        if self.sensor_blind.is_some() {
            kinds.push(RegionEffectKind::SensorBlind);
        }
        if let Some(n) = &self.nebula_fog {
            kinds.push(RegionEffectKind::NebulaFog {
                color: n.color,
                density: n.density,
            });
        }
        kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(effect: RegionEffectKind) {
        let json = serde_json::to_string(&effect).unwrap();
        let decoded: RegionEffectKind = serde_json::from_str(&json).unwrap();
        assert_eq!(effect, decoded);
    }

    #[test]
    fn serde_round_trip_damage_zone() {
        round_trip(RegionEffectKind::DamageZone {
            dps: 15.0,
            shield_pierce: 0.0,
        });
    }

    #[test]
    fn serde_round_trip_slow_zone() {
        round_trip(RegionEffectKind::SlowZone {
            thrust_modifier: Some(0.5),
            yaw_rate_modifier: Some(-0.3),
        });
        round_trip(RegionEffectKind::SlowZone {
            thrust_modifier: Some(0.5),
            yaw_rate_modifier: None,
        });
        round_trip(RegionEffectKind::SlowZone {
            thrust_modifier: None,
            yaw_rate_modifier: Some(-0.3),
        });
        round_trip(RegionEffectKind::SlowZone {
            thrust_modifier: None,
            yaw_rate_modifier: None,
        });
    }

    #[test]
    fn serde_round_trip_blocks_impulse() {
        round_trip(RegionEffectKind::BlocksImpulse);
    }

    #[test]
    fn serde_round_trip_radar_dampening() {
        round_trip(RegionEffectKind::RadarDampening { multiplier: 0.3 });
    }

    #[test]
    fn serde_round_trip_comms_jam() {
        round_trip(RegionEffectKind::CommsJam);
    }

    #[test]
    fn serde_round_trip_sensor_blind() {
        round_trip(RegionEffectKind::SensorBlind);
    }

    #[test]
    fn serde_round_trip_nebula_fog() {
        round_trip(RegionEffectKind::NebulaFog {
            color: [0.25, 0.08, 0.32],
            density: 0.008,
        });
        round_trip(RegionEffectKind::NebulaFog {
            color: [0.5, 0.1, 0.2],
            density: 0.015,
        });
    }

    #[test]
    fn serde_round_trip_negative_values() {
        round_trip(RegionEffectKind::DamageZone {
            dps: -5.0,
            shield_pierce: 0.0,
        });
        round_trip(RegionEffectKind::SlowZone {
            thrust_modifier: Some(-1.0),
            yaw_rate_modifier: None,
        });
    }

    #[test]
    fn serde_round_trip_zero_values() {
        round_trip(RegionEffectKind::DamageZone {
            dps: 0.0,
            shield_pierce: 0.0,
        });
        round_trip(RegionEffectKind::RadarDampening { multiplier: 0.0 });
    }

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
}
