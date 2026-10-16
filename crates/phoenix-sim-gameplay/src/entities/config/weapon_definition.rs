//! Supported authored weapon composition. Compilation creates existing
//! strategy parameters, never another authoritative weapon lifecycle.
use super::*;

pub type WeaponMountConfig = crate::weapons::mount::MountConfig;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponDefinition {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "phoenix_model::weapon_operation::deserialize_target_filter"
    )]
    pub targets: Option<Vec<phoenix_model::tags::EntityTag>>,

    pub id: String,
    /// Existing bank/tube identity, independently namespaced by delivery
    /// family. Defaults to the authored definition id.
    #[serde(default)]
    pub strategy_id: Option<String>,
    pub mount: WeaponMountConfig,
    pub impact: phoenix_model::damage::DamageProfile,
    pub delivery: WeaponDeliveryConfig,
    #[serde(default)]
    pub marker: Option<String>,
    #[serde(default)]
    pub barrels: Vec<String>,
    #[serde(default)]
    pub pattern: crate::weapons::pattern::BarrelPattern,
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WeaponDeliveryConfig {
    Beam {
        #[serde(default)]
        operation: phoenix_model::weapon_operation::BeamOperation,
        auto_arc_deg: f32,
        damage_per_sec: f32,
        duration_secs: f32,
        cooldown_secs: f32,
        #[serde(default)]
        cycle_jitter: f32,
        #[serde(default)]
        color: Vec<f32>,
        #[serde(default)]
        shield_pierce: f32,
    },
    Bolt {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rounds: Option<phoenix_model::weapon_operation::WeaponRoundFeed>,
        volley_count: u32,
        volley_interval_secs: f32,
        cooldown_secs: f32,
        projectile_speed: f32,
        collision_radius: f32,
        visual_scale: f32,
        damage: i32,
        #[serde(default)]
        charge_time_secs: f32,
        #[serde(default)]
        shield_pierce: f32,
        #[serde(default)]
        recoil_impulse: f32,
        #[serde(default)]
        screenshake_magnitude: f32,
    },
    /// Guided payload and flight tuning remain shared in `[torpedoes]`.
    Guided {
        #[serde(default = "super::weapons::default_load_priority")]
        load_priority: u16,
        #[serde(default)]
        load_time: Option<f32>,
        #[serde(default = "one_round")]
        volley_max: u32,
        #[serde(default)]
        ai_target_count: Option<u32>,
    },
}

fn one_round() -> u32 {
    1
}

fn nonnegative(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{name} must be finite and nonnegative"));
    }
    Ok(())
}

fn positive(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{name} must be finite and positive"));
    }
    Ok(())
}

fn pierce(value: f32) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err("shield_pierce must be finite and in [0, 1]".into());
    }
    Ok(())
}

impl WeaponDefinition {
    fn validate(&self, torpedoes: Option<&TorpedoesConfig>) -> Result<(), String> {
        if self.id.trim().is_empty() || self.id.trim() != self.id {
            return Err("weapon id must be nonempty without surrounding whitespace".into());
        }
        if self
            .strategy_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty() || id.trim() != id)
        {
            return Err("strategy_id must be nonempty without surrounding whitespace".into());
        }
        crate::weapons::operation::validate_target_filter(self.targets.as_deref())?;
        self.mount.validate()?;
        self.impact.validate()?;
        if self
            .marker
            .as_ref()
            .is_some_and(|marker| marker.trim().is_empty())
            || self.barrels.iter().any(|barrel| barrel.trim().is_empty())
        {
            return Err("marker and barrel names must be nonempty".into());
        }
        crate::weapons::pattern::validate_barrel_pattern(
            &format!("weapon '{}'", self.id),
            self.barrels.len().max(1),
            &self.pattern,
        )?;
        match &self.delivery {
            WeaponDeliveryConfig::Beam {
                operation,
                auto_arc_deg,
                damage_per_sec,
                duration_secs,
                cooldown_secs,
                cycle_jitter,
                color,
                shield_pierce,
            } => {
                if !self.barrels.is_empty() || !self.pattern.is_empty() {
                    return Err("beam delivery supports one marker; barrel patterns require bolt or guided delivery".into());
                }
                if !(0.0 < *auto_arc_deg && *auto_arc_deg <= self.mount.fire_arc_deg) {
                    return Err("auto_arc_deg must be in (0, mount.fire_arc_deg]".into());
                }
                positive("damage_per_sec", *damage_per_sec)?;
                positive("duration_secs", *duration_secs)?;
                // Existing beams use zero as a default sentinel, so accepting
                // an explicit zero here would silently substitute a different cycle.
                positive("cooldown_secs", *cooldown_secs)?;
                if !cycle_jitter.is_finite() || !(0.0..1.0).contains(cycle_jitter) {
                    return Err("cycle_jitter must be finite and in [0, 1)".into());
                }
                if *operation == phoenix_model::weapon_operation::BeamOperation::Sustained
                    && *cycle_jitter != 0.0
                {
                    return Err("sustained beam operation has no fixed-cycle jitter".into());
                }
                if !color.is_empty()
                    && (color.len() != 4
                        || color
                            .iter()
                            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)))
                {
                    return Err("color must be empty or four finite RGBA values in [0, 1]".into());
                }
                pierce(*shield_pierce)?;
            }
            WeaponDeliveryConfig::Bolt {
                rounds,
                volley_count,
                volley_interval_secs,
                cooldown_secs,
                projectile_speed,
                collision_radius,
                visual_scale,
                damage,
                charge_time_secs,
                shield_pierce,
                recoil_impulse,
                screenshake_magnitude,
            } => {
                if let Some(rounds) = rounds {
                    rounds.validate()?;
                }
                if (*volley_count == 0 && self.pattern.is_empty()) || *damage < 0 {
                    return Err(
                        "bolt requires a nonempty firing cycle and nonnegative damage".into(),
                    );
                }
                // Zero is the shipped simultaneous-volley cadence; the
                // schedule still contains its authored finite shot count.
                nonnegative("volley_interval_secs", *volley_interval_secs)?;
                nonnegative("cooldown_secs", *cooldown_secs)?;
                positive("projectile_speed", *projectile_speed)?;
                positive("collision_radius", *collision_radius)?;
                positive("visual_scale", *visual_scale)?;
                nonnegative("charge_time_secs", *charge_time_secs)?;
                nonnegative("recoil_impulse", *recoil_impulse)?;
                nonnegative("screenshake_magnitude", *screenshake_magnitude)?;
                pierce(*shield_pierce)?;
            }
            WeaponDeliveryConfig::Guided {
                load_time,
                volley_max,
                ai_target_count,
                load_priority: _,
            } => {
                let tuning = torpedoes
                    .ok_or("guided delivery requires [torpedoes] payload/flight tuning")?;
                positive("torpedoes.speed", tuning.speed)?;
                positive("torpedoes.lifespan", tuning.lifespan)?;
                let range = tuning.speed * tuning.lifespan;
                if !range.is_finite() || (self.mount.range - range).abs() > range * 1e-6 {
                    return Err("guided mount.range must equal torpedoes.speed * torpedoes.lifespan; per-tube flight tuning is not supported".into());
                }
                if *volley_max == 0 || ai_target_count.is_some_and(|count| count > *volley_max) {
                    return Err(
                        "guided volley_max must be positive and ai_target_count must fit".into(),
                    );
                }
                positive("load_time", load_time.unwrap_or(tuning.load_time))?;
                positive("torpedoes.burst_interval_secs", tuning.burst_interval_secs)?;
                nonnegative(
                    "torpedoes.turn_rate_deg_per_sec",
                    tuning.turn_rate_deg_per_sec,
                )?;
                positive("torpedoes.detonation_radius", tuning.detonation_radius)?;
                if tuning.damage_hull < 0 || tuning.damage_shields < 0 {
                    return Err("guided payload damage must be nonnegative".into());
                }
                pierce(tuning.shield_pierce)?;
            }
        }
        Ok(())
    }

    fn compile(self) -> Result<(&'static str, &'static str, toml::Value), String> {
        let mount = self.mount;
        let id = self.strategy_id.unwrap_or(self.id);
        let (parent, field, value) = match self.delivery {
            WeaponDeliveryConfig::Beam {
                operation,
                auto_arc_deg,
                damage_per_sec,
                duration_secs,
                cooldown_secs,
                cycle_jitter,
                color,
                shield_pierce,
            } => {
                let bank = PhaserBankConfig {
                    targets: self.targets,
                    operation,
                    impact: self.impact,
                    id,
                    facing_deg: mount.facing_deg,
                    fire_arc_deg: mount.fire_arc_deg,
                    auto_arc_deg,
                    beam_range: mount.range,
                    beam_damage_per_sec: damage_per_sec,
                    beam_duration_secs: duration_secs,
                    cooldown_secs,
                    cycle_jitter,
                    beam_color: color,
                    shield_pierce: Some(shield_pierce),
                    marker: self.marker,
                    ai: self.ai,
                };
                (
                    "weapons_console",
                    "phaser_banks",
                    toml::Value::try_from(bank),
                )
            }
            WeaponDeliveryConfig::Bolt {
                rounds,
                volley_count,
                volley_interval_secs,
                cooldown_secs,
                projectile_speed,
                collision_radius,
                visual_scale,
                damage,
                charge_time_secs,
                shield_pierce,
                recoil_impulse,
                screenshake_magnitude,
            } => {
                let bank = BlasterBankConfig {
                    targets: self.targets,
                    rounds,
                    impact: self.impact,
                    id,
                    facing_deg: mount.facing_deg,
                    fire_arc_deg: mount.fire_arc_deg,
                    volley_count,
                    volley_interval_secs,
                    cooldown_secs,
                    charge_time_secs,
                    projectile_speed,
                    collision_radius,
                    visual_scale,
                    damage,
                    shield_pierce,
                    recoil_impulse,
                    screenshake_magnitude,
                    marker: self.marker,
                    barrels: self.barrels,
                    pattern: self.pattern,
                    range: mount.range,
                    ai: self.ai,
                };
                (
                    "weapons_console",
                    "blaster_banks",
                    toml::Value::try_from(bank),
                )
            }
            WeaponDeliveryConfig::Guided {
                load_priority,
                load_time,
                volley_max,
                ai_target_count,
            } => {
                let tube = TorpedoTubeConfig {
                    load_priority,
                    targets: self.targets,
                    impact: self.impact,
                    id,
                    facing_deg: mount.facing_deg,
                    fire_arc_deg: mount.fire_arc_deg,
                    load_time,
                    marker: self.marker,
                    barrels: self.barrels,
                    pattern: self.pattern,
                    volley_max,
                    ai_target_count,
                    ai: self.ai,
                };
                ("torpedoes", "tubes", toml::Value::try_from(tube))
            }
        };
        Ok((parent, field, value.map_err(|error| error.to_string())?))
    }
}

/// Bind finite rounds to a real local capacity owned by this actual bank.
/// Inventory and Logistics retain the stock; the weapon gets no count mirror.
pub(super) fn validate_round_feeds(config: &EntityConfig) -> Result<(), String> {
    let Some(weapons) = &config.weapons_console else {
        return Ok(());
    };
    for bank in &weapons.phaser_banks {
        crate::weapons::operation::validate_target_filter(bank.targets.as_deref())?;
        if bank.operation == phoenix_model::weapon_operation::BeamOperation::Sustained
            && bank.cycle_jitter != 0.0
        {
            return Err("sustained beam operation has no fixed-cycle jitter".into());
        }
    }
    for bank in &weapons.blaster_banks {
        crate::weapons::operation::validate_target_filter(bank.targets.as_deref())?;
        let Some(feed) = &bank.rounds else {
            continue;
        };
        feed.validate()?;
        let owner = crate::ship::system_registry::blaster_bank_system_id(&bank.id)
            .ok_or("round feed has no actual bank System")?;
        if !config.ship_config.as_ref().is_some_and(|ship| {
            ship.systems.iter().any(|system| {
                system.id == owner && system.kind == crate::ship::system_registry::BLASTER_BANK_KIND
            })
        }) {
            return Err("weapon rounds require their actual fitted bolt bank System".into());
        }
        let store = config
            .logistics
            .as_ref()
            .and_then(|profile| profile.stores.iter().find(|store| store.id == feed.store))
            .ok_or("weapon round feed requires an actual managed local store")?;
        let room = config
            .interior
            .as_ref()
            .and_then(|layout| layout.placements.get(&owner))
            .ok_or("round feed bank needs an actual compartment")?;
        if store.role != phoenix_model::logistics::StoreRole::Local
            || store.system.as_ref() != Some(&owner)
            || &store.compartment != room
        {
            return Err(
                "weapon rounds require the bank's own local store in its physical compartment"
                    .into(),
            );
        }
        let phoenix_model::logistics::ResourceEndpoint::Capacity { id } = &store.endpoint else {
            return Err("bolt rounds require an actual capacity endpoint".into());
        };
        let capacity = config
            .infrastructure
            .as_ref()
            .and_then(|infrastructure| {
                infrastructure
                    .capacities
                    .iter()
                    .find(|capacity| &capacity.id == id)
            })
            .ok_or("weapon round capacity is absent")?;
        if capacity.ceiling.unwrap_or(capacity.amount) < feed.amount {
            return Err("weapon rounds per barrel exceed actual capacity".into());
        }
    }
    Ok(())
}

pub(super) fn compile_weapons(value: &mut toml::Value) -> Result<(), String> {
    let root = value.as_table_mut().ok_or("entity must be a table")?;
    let Some(authored) = root.remove("weapon") else {
        return Ok(());
    };
    let definitions = authored
        .as_array()
        .ok_or("[[weapon]] must be an array of tables")?;
    let tuning = root
        .get("torpedoes")
        .cloned()
        .map(toml::Value::try_into::<TorpedoesConfig>)
        .transpose()
        .map_err(|error| error.to_string())?;
    let mut seen = std::collections::BTreeSet::new();
    for authored in definitions {
        let definition: WeaponDefinition = authored
            .clone()
            .try_into()
            .map_err(|error: toml::de::Error| error.to_string())?;
        if !seen.insert(definition.id.clone()) {
            return Err(format!("duplicate generic weapon id '{}'", definition.id));
        }
        definition
            .validate(tuning.as_ref())
            .map_err(|error| format!("weapon '{}': {error}", definition.id))?;
        let id = definition
            .strategy_id
            .clone()
            .unwrap_or_else(|| definition.id.clone());
        let (parent, field, compiled) = definition.compile()?;
        let section = root
            .entry(parent)
            .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("[{parent}] must be a table"))?;
        let list = section
            .entry(field)
            .or_insert_with(|| toml::Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("{parent}.{field} must be an array"))?;
        if list
            .iter()
            .any(|entry| entry.get("id").and_then(toml::Value::as_str) == Some(id.as_str()))
        {
            return Err(format!("weapon '{id}' is declared in both [[weapon]] and [[{parent}.{field}]]; keep one definition"));
        }
        list.push(compiled);
    }
    Ok(())
}

#[cfg(test)]
#[path = "weapon_definition_tests.rs"]
mod tests;
