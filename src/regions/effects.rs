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

/// A region's effect on the radar horizon of every ship standing in it.
///
/// # `range_modifier` is a signed BONUS, not a multiplier
///
/// The field name is `range_modifier` and the serde alias is `multiplier`, and
/// the alias is the historical trap: the value is neither. It is added to
/// [`crate::messages::ModifierSlot::RadarRange`] by
/// `modifiers::coordination::apply_region_effects`, and the slot's cache
/// (`modifiers::cache::ShipModifiers::rebuild_cache`) turns the SUM of every
/// bonus on the slot into the multiplier the radar actually uses, via PRD
/// #117's two-sided formula:
///
/// ```text
/// bonus >= 0  ->  multiplier = 1 + bonus
/// bonus <  0  ->  multiplier = 1 / (1 + |bonus|)
/// ```
///
/// So a DAMPENING region authors a NEGATIVE number, and the value that gives a
/// wanted multiplier `m` (for `0 < m < 1`) is `-(1/m - 1)`: −1.0 halves the
/// horizon, −1.5 takes it to two fifths, −2.0 to a third.
///
/// A POSITIVE `range_modifier` on an effect called *dampening* therefore does
/// the opposite of what it reads like — it lets a ship see FURTHER inside the
/// hazard than outside it. Two of the three shipped region templates authored
/// exactly that (`region_kaleth_nebula.toml` at `0.4`, `region_storm_band.toml`
/// at `0.5`, both evidently meaning "reduce the radar to 40%/50%") until the
/// fix that added this doc comment; the defect was found in #1037, whose own
/// `region_radiation_band.toml` documents the formula and authors `-2.0`
/// deliberately.
///
/// There is no load-time validation surface for region effects to warn from —
/// see `shipped_assets::every_shipped_radar_dampening_actually_dampens` below,
/// which is the CI-side guard that replaced the warning that would have needed
/// one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadarDampeningEffect {
    #[serde(alias = "multiplier")]
    pub range_modifier: f32,
}

impl RadarDampeningEffect {
    /// True when this effect actually reduces the radar horizon — i.e. when the
    /// authored bonus resolves to a multiplier below 1.0.
    ///
    /// `0.0` is not dampening either: it is a modifier that changes nothing,
    /// which on an effect whose entire job is to change something is an
    /// authoring mistake rather than a neutral default.
    pub fn dampens(&self) -> bool {
        self.range_modifier < 0.0
    }
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

    // ── RegionEffectKind serde round-trips live in src/core/codec.rs ──────
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
}

/// Shipped-asset conformance: every region template in `assets/` that claims to
/// dampen radar actually does.
///
/// This reads the real files (native only, like `marker_validate`'s own
/// shipped-asset walk) because the defect this guards against is a DATA defect
/// and no amount of engine testing catches it: the runtime formula was always
/// right, and `regions::server`'s own dampening tests always passed, because
/// they author their own negative bonuses. What shipped were two templates
/// authoring a positive one — a nebula and a storm band that made the radar
/// reach further inside the hazard than outside it, for as long as they existed
/// (found in #1037).
///
/// There is deliberately no load-time warning to go with this. A region effect
/// has no validation surface of its own — the `WorldFinding` collector in
/// `world::validate` walks WORLD references and `entities::marker_validate`
/// walks rig markers, and a third one would mean a new finding type, a new
/// startup call site and a new editor-side twin to keep honest. A `cargo test`
/// gate over `assets/` is strictly harder than a printed warning for everything
/// that ships in this repository; what it does NOT cover is a region template
/// arriving from a MOD PACK, which is the case a real load-time check would
/// buy and the reason to revisit this when one needs it.
#[cfg(test)]
mod shipped_assets {
    use super::*;

    #[test]
    fn every_shipped_radar_dampening_actually_dampens() {
        #[derive(Deserialize)]
        struct Wrap {
            #[serde(default)]
            effects: RegionEffectsConfig,
        }

        let mut checked = 0usize;
        let mut problems: Vec<String> = Vec::new();
        let entries = std::fs::read_dir("assets/entities").expect("assets/entities must exist");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let file = path.to_string_lossy().replace('\\', "/");
            let toml_str = std::fs::read_to_string(&path).expect("entity template readable");
            // Templates that are not regions simply carry no `[effects]` block;
            // one that fails to parse is somebody else's test to fail.
            let Ok(wrap) = toml::from_str::<Wrap>(&toml_str) else {
                continue;
            };
            let Some(dampening) = wrap.effects.radar_dampening else {
                continue;
            };
            checked += 1;
            if !dampening.dampens() {
                let m = dampening.range_modifier;
                let effective = if m >= 0.0 {
                    1.0 + m
                } else {
                    1.0 / (1.0 + m.abs())
                };
                problems.push(format!(
                    "{file}: `[effects.radar_dampening] range_modifier = {m}` is a signed BONUS, \
                     so it resolves to a radar-range multiplier of {effective} — the region makes \
                     the radar reach FURTHER. For a multiplier of `m`, author `-(1/m - 1)`."
                ));
            }
        }
        assert!(
            checked > 0,
            "shipped region templates should author radar dampening"
        );
        assert!(
            problems.is_empty(),
            "a region that dampens radar must author a NEGATIVE range_modifier:\n{}",
            problems.join("\n")
        );
    }
}
