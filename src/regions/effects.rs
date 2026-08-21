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

/// A region's effect on how fast — and how sharply — a ship standing in it can
/// fly.
///
/// # Both fields are signed BONUSES, not multipliers
///
/// This is the same trap [`RadarDampeningEffect::range_modifier`] carries, on a
/// field pair whose names read even more like multipliers. `thrust_modifier`
/// and `yaw_rate_modifier` are added to [`crate::messages::ModifierSlot::MaxSpeed`]
/// and [`crate::messages::ModifierSlot::MaxYawRate`] by
/// `modifiers::coordination::apply_region_effects`, and each slot's cache
/// (`modifiers::cache::ShipModifiers::rebuild_cache`) turns the SUM of every
/// bonus on the slot into the multiplier the helm actually flies, through PRD
/// #117's two-sided formula:
///
/// ```text
/// bonus >= 0  ->  multiplier = 1 + bonus
/// bonus <  0  ->  multiplier = 1 / (1 + |bonus|)
/// ```
///
/// So a SLOWING region authors NEGATIVE numbers, and the value that gives a
/// wanted multiplier `m` (for `0 < m < 1`) is `-(1/m - 1)`: −1.0 halves the
/// axis, −2/3 takes it to three fifths, −3/7 to seven tenths.
///
/// A POSITIVE number on an effect called a *slow zone* therefore does the
/// opposite of what it reads like — the hazard makes ships FASTER and more
/// agile than they are in clear space. Both shipped bands authored exactly
/// that (`region_storm_band.toml` at `0.5`/`0.6` and
/// `region_radiation_band.toml` at `0.6`/`0.7`, evidently meaning "reduce
/// thrust to 50%/60%" and "to 60%/70%") until the fix that added this doc
/// comment, so a storm front sped a ship up by half and a radiation front by
/// three fifths. It is the same defect the radar-dampening sign fix corrected
/// on the neighbouring field, found in #1037 and fixed there first.
///
/// # A field-free slow zone is not a mistake
///
/// `[effects.slow_zone]` with NEITHER field authored is legitimate and shipped
/// deliberately: it is the presence marker an operation's
/// `[[operations.capability.interrupt]]` names with
/// `region_effect = "slow_zone"`, and the rate that stretches the work lives on
/// the CAPABILITY. A band that only wants to make external work take longer
/// authors no numbers here and has no sign to get wrong. That path is a
/// separate, correctly-signed mechanism (`rate_percent`, where 50 means half
/// rate) and nothing here touches it.
///
/// See `shipped_assets::every_shipped_slow_zone_actually_slows` below for the
/// CI-side guard, and
/// `regions::server::every_shipped_slow_zone_slows_the_ship_that_enters_it` for
/// the runtime twin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlowZoneEffect {
    pub thrust_modifier: Option<f32>,
    pub yaw_rate_modifier: Option<f32>,
}

impl SlowZoneEffect {
    /// True when every axis this effect actually authors slows the ship — i.e.
    /// when each present bonus resolves to a multiplier below 1.0.
    ///
    /// Vacuously true for the field-free presence marker described on the type,
    /// and that is the intended reading: a slow zone that authors no numbers has
    /// no sign to get wrong, and rejecting it here would fail the one shape the
    /// operations path depends on.
    ///
    /// `0.0` on a PRESENT axis is not neutral either: it is a modifier that
    /// modifies nothing, on the one axis whose entire job is to change
    /// something, which is an authoring mistake rather than a default.
    pub fn slows(&self) -> bool {
        [self.thrust_modifier, self.yaw_rate_modifier]
            .into_iter()
            .flatten()
            .all(|bonus| bonus < 0.0)
    }
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

/// The authorable name of a region effect (issue #1026, relocated here in
/// #1166 when the operations coordinator that first defined it was dissolved).
///
/// The spellings mirror [`RegionEffectKind`]'s variants, and
/// [`region_effect_name`] maps one to the other. An enum rather than a raw
/// string so a misspelt band is a load error instead of a rule that silently
/// never fires. The science scan reports which of these a structure is standing
/// in; [`region_effect_name`]'s test proves every kind has a name here, so a new
/// hazard cannot ship unauthorable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionEffectName {
    DamageZone,
    SlowZone,
    BlocksImpulse,
    RadarDampening,
    CommsJam,
    SensorBlind,
    NebulaFog,
}

impl RegionEffectName {
    /// Every effect name, in declaration order.
    pub const ALL: &'static [RegionEffectName] = &[
        RegionEffectName::DamageZone,
        RegionEffectName::SlowZone,
        RegionEffectName::BlocksImpulse,
        RegionEffectName::RadarDampening,
        RegionEffectName::CommsJam,
        RegionEffectName::SensorBlind,
        RegionEffectName::NebulaFog,
    ];

    /// The authored spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            RegionEffectName::DamageZone => "damage_zone",
            RegionEffectName::SlowZone => "slow_zone",
            RegionEffectName::BlocksImpulse => "blocks_impulse",
            RegionEffectName::RadarDampening => "radar_dampening",
            RegionEffectName::CommsJam => "comms_jam",
            RegionEffectName::SensorBlind => "sensor_blind",
            RegionEffectName::NebulaFog => "nebula_fog",
        }
    }
}

/// The authorable name of a live region effect (issue #1026, relocated in
/// #1166).
///
/// Total by construction — a new [`RegionEffectKind`] variant will not compile
/// until it has a name, which is the point. A hazard band nobody can name is a
/// hazard nothing can be told about.
pub fn region_effect_name(kind: &RegionEffectKind) -> RegionEffectName {
    match kind {
        RegionEffectKind::DamageZone { .. } => RegionEffectName::DamageZone,
        RegionEffectKind::SlowZone { .. } => RegionEffectName::SlowZone,
        RegionEffectKind::BlocksImpulse => RegionEffectName::BlocksImpulse,
        RegionEffectKind::RadarDampening { .. } => RegionEffectName::RadarDampening,
        RegionEffectKind::CommsJam => RegionEffectName::CommsJam,
        RegionEffectKind::SensorBlind => RegionEffectName::SensorBlind,
        RegionEffectKind::NebulaFog { .. } => RegionEffectName::NebulaFog,
    }
}

#[cfg(test)]
mod tests {
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
}
