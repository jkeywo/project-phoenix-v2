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
/// and `yaw_rate_modifier` are added to [`crate::core::messages::ModifierSlot::MaxSpeed`]
/// and [`crate::core::messages::ModifierSlot::MaxYawRate`] by
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
/// # A field-free slow zone is valid but inert
///
/// `[effects.slow_zone]` with NEITHER field authored still parses, but current
/// runtime semantics register no speed or yaw modifier for it. The old
/// operations runner once consumed the table's mere presence as an interruption
/// marker; that consumer retired in issue #1164 with `[operations]`. No shipped
/// region now relies on a field-free slow zone, and the sign guard below skips
/// one because there is no numeric effect to judge.
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
    /// Vacuously true for an inert field-free table: it has no numeric sign to
    /// reject. No current crew-system path consumes that empty table; this method
    /// only answers whether any numbers that are present point in the right
    /// direction.
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
/// [`crate::core::messages::ModifierSlot::RadarRange`] by
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
#[path = "effects_tests.rs"]
mod tests;
