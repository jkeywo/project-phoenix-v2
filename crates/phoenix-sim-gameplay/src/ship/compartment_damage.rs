//! One pure owner of compartment structure and mechanism-bound faults.
//! Producers supply their actual incident and RNG stream; repair owners supply
//! completed, paid remedies. No controller, stock mirror or second ship HP pool.
use super::config::ShipConfig;
pub use phoenix_model::damage::*;
use phoenix_model::interior::{CompartmentConfig, InteriorConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use vellum_rng::Pcg32;

const TYPES: [DamageType; 2] = [DamageType::Physical, DamageType::Laser];
fn id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn source(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
fn resource_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}
fn finite(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}
fn unit(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
fn typed(values: &BTreeMap<DamageType, f32>) -> bool {
    TYPES
        .iter()
        .all(|kind| values.get(kind).is_some_and(|v| finite(*v)))
}

#[derive(Clone, Debug)]
pub struct DamageDefinition {
    pub interior: InteriorConfig,
    pub config: DamageConfig,
    pub components:
        BTreeMap<phoenix_model::messages::SystemId, BTreeMap<String, DamageComponentConfig>>,
    rooms: BTreeMap<String, CompartmentConfig>,
    decks: Vec<(String, f64)>,
}
impl DamageDefinition {
    pub fn compile(
        interior: &InteriorConfig,
        config: &DamageConfig,
        ship: &ShipConfig,
    ) -> Result<Self, String> {
        if let Some(emp) = &config.emp {
            emp.validate()?;
        }
        let mut decks = BTreeMap::new();
        for deck in &interior.decks {
            if !id(&deck.id)
                || !finite(deck.damage_weight)
                || deck.damage_weight == 0.0
                || decks
                    .insert(deck.id.clone(), deck.damage_weight as f64)
                    .is_some()
            {
                return Err("damage decks require unique IDs and positive finite weights".into());
            }
        }
        let mut rooms = BTreeMap::new();
        for room in &interior.compartments {
            if !id(&room.id)
                || !decks.contains_key(&room.deck)
                || ![
                    room.x,
                    room.z,
                    room.width,
                    room.depth,
                    room.max_hp,
                    room.recovery_fraction,
                ]
                .iter()
                .all(|v| v.is_finite())
                || room.width <= 0.0
                || room.depth <= 0.0
                || room.max_hp <= 0.0
                || room.recovery_fraction <= 0.0
                || room.recovery_fraction >= 1.0
                || !config.materials.contains_key(&room.material)
                || !matches!(
                    config
                        .remedies
                        .get(&room.structural_remedy)
                        .map(|r| &r.outcome),
                    Some(RemedyOutcome::Structural)
                )
                || rooms.insert(room.id.clone(), room.clone()).is_some()
            {
                return Err(format!(
                    "invalid structural compartment '{}' or missing material/remedy",
                    room.id
                ));
            }
        }
        if rooms.is_empty()
            || decks.is_empty()
            || decks
                .keys()
                .any(|deck| !rooms.values().any(|room| &room.deck == deck))
        {
            return Err("each damage deck requires physical compartments".into());
        }
        let values: Vec<_> = rooms.values().collect();
        for (i, a) in values.iter().enumerate() {
            for b in values.iter().skip(i + 1) {
                if a.deck == b.deck
                    && (a.x as f64).max(b.x as f64)
                        < (a.x as f64 + a.width as f64).min(b.x as f64 + b.width as f64)
                    && (a.z as f64).max(b.z as f64)
                        < (a.z as f64 + a.depth as f64).min(b.z as f64 + b.depth as f64)
                {
                    return Err(format!(
                        "damage compartments '{}' and '{}' overlap on deck '{}'",
                        a.id, b.id, a.deck
                    ));
                }
            }
        }
        for (name, material) in &config.materials {
            if !id(name) || !typed(&material.resistance) || !unit(material.residual_fraction) {
                return Err(format!("invalid damage material '{name}'"));
            }
        }
        for (name, remedy) in &config.remedies {
            let outcome = match remedy.outcome {
                RemedyOutcome::Structural => remedy
                    .structural_hp
                    .is_some_and(|hp| finite(hp) && hp > 0.0),
                RemedyOutcome::Full => remedy.structural_hp.is_none(),
                RemedyOutcome::Patch { factor } => {
                    remedy.structural_hp.is_none() && unit(factor) && factor > 0.0 && factor < 1.0
                }
            };
            if !id(name)
                || !finite(remedy.work_secs)
                || remedy.work_secs == 0.0
                || !unit(remedy.manual_work_fraction)
                || remedy.manual_work_fraction == 0.0
                || !outcome
                || remedy
                    .requirements
                    .iter()
                    .any(|r| !resource_id(&r.resource) || !resource_id(&r.service) || r.amount <= 0)
            {
                return Err(format!("invalid damage remedy '{name}'"));
            }
        }
        let systems: BTreeSet<_> = ship
            .systems
            .iter()
            .map(|system| system.id.clone())
            .collect();
        if systems.len() != ship.systems.len()
            || interior.placements.keys().cloned().collect::<BTreeSet<_>>() != systems
            || interior
                .placements
                .values()
                .any(|room| !rooms.contains_key(room))
        {
            return Err("damage placements must name every installed System exactly once".into());
        }
        let mut components = BTreeMap::new();
        for system in &ship.systems {
            let mut definitions = BTreeMap::new();
            for component in &system.damage_components {
                if !id(&component.id)
                    || !typed(&component.susceptibility)
                    || !finite(component.fault_dose_threshold)
                    || component.fault_dose_threshold == 0.0
                    || !unit(component.degraded_factor)
                    || component.degraded_factor == 0.0
                    || component.degraded_factor == 1.0
                    || !matches!(
                        config
                            .remedies
                            .get(&component.full_remedy)
                            .map(|r| &r.outcome),
                        Some(RemedyOutcome::Full)
                    )
                    || component.patch_remedy.as_ref().is_some_and(|r| {
                        !matches!(
                            config.remedies.get(r).map(|r| &r.outcome),
                            Some(RemedyOutcome::Patch { .. })
                        )
                    })
                    || definitions
                        .insert(component.id.clone(), component.clone())
                        .is_some()
                {
                    return Err(format!(
                        "invalid damage component '{}:{}' or unsupported remedy",
                        system.id.0, component.id
                    ));
                }
            }
            components.insert(system.id.clone(), definitions);
        }
        Ok(Self {
            interior: interior.clone(),
            config: config.clone(),
            components,
            rooms,
            decks: decks.into_iter().collect(),
        })
    }
    pub fn sample_hit(&self, rng: &mut Pcg32) -> HitSample {
        let total: f64 = self.decks.iter().map(|(_, weight)| weight).sum();
        let mut draw = super::damage::unit_f32(rng) as f64 * total;
        let mut selected = &self.decks[self.decks.len() - 1].0;
        for (deck, weight) in &self.decks {
            if draw < *weight {
                selected = deck;
                break;
            }
            draw -= weight;
        }
        HitSample {
            deck: selected.clone(),
            profile_fraction: super::damage::unit_f32(rng),
        }
    }
    pub fn component(&self, target: &ComponentRef) -> Option<&DamageComponentConfig> {
        self.components.get(&target.system)?.get(&target.component)
    }
    pub fn compartment(&self, id: &str) -> Option<&CompartmentConfig> {
        self.rooms.get(id)
    }
    pub fn component_room(&self, target: &ComponentRef) -> Option<&str> {
        self.component(target)?;
        self.interior
            .placements
            .get(&target.system)
            .map(String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageState {
    pub compartments: BTreeMap<String, CompartmentState>,
    pub components: BTreeMap<phoenix_model::messages::SystemId, BTreeMap<String, ComponentState>>,
    pub catastrophic: bool,
}
impl DamageState {
    pub fn new(definition: &DamageDefinition) -> Self {
        Self {
            compartments: definition
                .rooms
                .iter()
                .map(|(id, room)| {
                    (
                        id.clone(),
                        CompartmentState {
                            hp: room.max_hp as f64,
                            disabled: false,
                        },
                    )
                })
                .collect(),
            components: definition
                .components
                .iter()
                .map(|(system, components)| {
                    (
                        system.clone(),
                        components
                            .keys()
                            .map(|id| {
                                (
                                    id.clone(),
                                    ComponentState {
                                        emp: None,
                                        condition: ComponentCondition::Healthy,
                                        dose: 0.0,
                                        last_source: None,
                                    },
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
            catastrophic: false,
        }
    }
    pub fn validate(&self, definition: &DamageDefinition) -> Result<(), String> {
        if self.compartments.keys().ne(definition.rooms.keys())
            || self.components.keys().ne(definition.components.keys())
        {
            return Err("damage state identities do not match fitted equipment/layout".into());
        }
        for (id, state) in &self.compartments {
            let room = &definition.rooms[id];
            let threshold = room.max_hp as f64 * room.recovery_fraction as f64;
            if !state.hp.is_finite()
                || state.hp < 0.0
                || state.hp > room.max_hp as f64
                || (state.hp == 0.0 && !state.disabled)
                || (state.hp >= threshold && state.disabled)
            {
                return Err(format!("invalid structural state '{id}'"));
            }
        }
        for (system, states) in &self.components {
            let definitions = &definition.components[system];
            if states.keys().ne(definitions.keys()) {
                return Err(
                    "damage component state identities do not match fitted equipment".into(),
                );
            }
            for (id, state) in states {
                let config = &definitions[id];
                if let Some(emp) = &state.emp {
                    if !emp.policy.bindings.contains(&config.binding) {
                        return Err("EMP state names an unsupported component binding".into());
                    }
                    emp.validate_shape()?;
                }
                let valid_condition = match state.condition {
                    ComponentCondition::Patched { factor } => config
                        .patch_remedy
                        .as_ref()
                        .and_then(|recipe| definition.config.remedies.get(recipe))
                        .is_some_and(|recipe| {
                            matches!(recipe.outcome,
                            RemedyOutcome::Patch { factor: authored } if authored == factor)
                        }),
                    _ => true,
                };
                if !valid_condition
                    || !state.dose.is_finite()
                    || state.dose < 0.0
                    || state.dose >= config.fault_dose_threshold as f64
                    || state.last_source.as_ref().is_some_and(|s| !source(s))
                    || (state.last_source.is_none()
                        && (state.dose > 0.0 || state.condition != ComponentCondition::Healthy))
                {
                    return Err(format!("invalid component state '{}:{id}'", system.0));
                }
            }
        }
        Ok(())
    }
    pub fn restore(&mut self, definition: &DamageDefinition, saved: &Self) -> Result<(), String> {
        saved.validate(definition)?;
        *self = saved.clone();
        Ok(())
    }
    pub fn structural_fraction(&self, definition: &DamageDefinition) -> f32 {
        let max: f64 = definition.rooms.values().map(|r| r.max_hp as f64).sum();
        (self.compartments.values().map(|r| r.hp).sum::<f64>() / max) as f32
    }
    pub fn effect_factor(
        &self,
        definition: &DamageDefinition,
        system: &phoenix_model::messages::SystemId,
        binding: DamageBinding,
    ) -> f32 {
        self.effect_factor_at(definition, system, binding, None)
    }
    pub fn effect_factor_at(
        &self,
        definition: &DamageDefinition,
        system: &phoenix_model::messages::SystemId,
        binding: DamageBinding,
        clock: Option<phoenix_model::emp::EmpClock>,
    ) -> f32 {
        self.binding_factor(definition, system, binding, true, clock)
    }
    pub fn physical_factor(
        &self,
        definition: &DamageDefinition,
        system: &phoenix_model::messages::SystemId,
        binding: DamageBinding,
    ) -> f32 {
        self.binding_factor(definition, system, binding, false, None)
    }
    fn binding_factor(
        &self,
        definition: &DamageDefinition,
        system: &phoenix_model::messages::SystemId,
        binding: DamageBinding,
        include_emp: bool,
        clock: Option<phoenix_model::emp::EmpClock>,
    ) -> f32 {
        if self.catastrophic
            || definition
                .interior
                .placements
                .get(system)
                .and_then(|id| self.compartments.get(id))
                .is_none_or(|room| room.disabled)
        {
            return 0.0;
        }
        let Some(states) = self.components.get(system) else {
            return 0.0;
        };
        states
            .iter()
            .filter(|(id, _)| definition.components[system][*id].binding == binding)
            .fold(1.0, |factor, (id, state)| {
                let physical = match state.condition {
                    ComponentCondition::Healthy => 1.0,
                    ComponentCondition::Damaged => {
                        definition.components[system][id].degraded_factor
                    }
                    ComponentCondition::Destroyed => 0.0,
                    ComponentCondition::Patched { factor } => factor,
                };
                let temporary = if include_emp {
                    state.emp.as_ref().map_or(1.0, |emp| {
                        clock.map_or(0.0, |clock| emp.factor_at(clock.tick))
                    })
                } else {
                    1.0
                };
                factor.min(physical * temporary)
            })
    }
    pub fn validate_emp_at(
        &self,
        definition: &DamageDefinition,
        clock: phoenix_model::emp::EmpClock,
    ) -> Result<(), String> {
        self.validate(definition)?;
        clock.validate(clock.tick)?;
        for components in self.components.values() {
            for component in components.values() {
                if let Some(emp) = &component.emp {
                    emp.validate(clock)?;
                }
            }
        }
        Ok(())
    }
    pub fn emp_failed_components(
        &self,
        system: &phoenix_model::messages::SystemId,
        clock: Option<phoenix_model::emp::EmpClock>,
    ) -> Vec<String> {
        self.components
            .get(system)
            .into_iter()
            .flat_map(|components| components.iter())
            .filter(|(_, state)| {
                state
                    .emp
                    .as_ref()
                    .is_some_and(|emp| clock.is_none_or(|clock| emp.factor_at(clock.tick) == 0.0))
            })
            .map(|(id, _)| id.clone())
            .collect()
    }
    pub fn restore_structure(
        &mut self,
        definition: &DamageDefinition,
        compartment: &str,
        amount: f32,
    ) -> Result<f32, String> {
        if !finite(amount) {
            return Err("structural restoration requires finite nonnegative work".into());
        }
        self.validate(definition)?;
        let room = definition
            .rooms
            .get(compartment)
            .ok_or("unknown structural repair target")?;
        let state = self
            .compartments
            .get_mut(compartment)
            .expect("validated state");
        let before = state.hp;
        state.hp = (state.hp + amount as f64).min(room.max_hp as f64);
        if state.hp >= room.max_hp as f64 * room.recovery_fraction as f64 {
            state.disabled = false;
        }
        Ok((state.hp - before) as f32)
    }
    /// Called only after the ordinary repair owner completes and pays its stage.
    pub fn remedy_component(
        &mut self,
        definition: &DamageDefinition,
        target: &ComponentRef,
        remedy: &str,
    ) -> Result<bool, String> {
        self.validate(definition)?;
        let config = definition
            .component(target)
            .ok_or("unknown component repair target")?;
        let state = self
            .components
            .get_mut(&target.system)
            .and_then(|c| c.get_mut(&target.component))
            .expect("validated target");
        if state.condition == ComponentCondition::Destroyed && !config.field_replaceable {
            return Err("component is beyond field restoration".into());
        }
        let next = if remedy == config.full_remedy {
            ComponentCondition::Healthy
        } else if config.patch_remedy.as_deref() == Some(remedy) {
            match definition.config.remedies[remedy].outcome {
                RemedyOutcome::Patch { factor } => ComponentCondition::Patched { factor },
                _ => unreachable!("compiled patch"),
            }
        } else {
            return Err("remedy is not supported by this component".into());
        };
        if state.condition == ComponentCondition::Healthy || state.condition == next {
            return Ok(false);
        }
        let changed = state.condition != next;
        state.condition = next;
        state.dose = 0.0;
        if state.condition == ComponentCondition::Healthy {
            state.last_source = None;
        }
        Ok(changed)
    }
    pub fn apply_hit(
        &mut self,
        definition: &DamageDefinition,
        hit: &DamageHit,
        rng: &mut Pcg32,
    ) -> Result<DamageOutcome, String> {
        self.apply_hit_at(definition, hit, rng, None)
    }
    pub fn apply_hit_at(
        &mut self,
        definition: &DamageDefinition,
        hit: &DamageHit,
        rng: &mut Pcg32,
        clock: Option<phoenix_model::emp::EmpClock>,
    ) -> Result<DamageOutcome, String> {
        self.validate(definition)?;
        hit.profile.validate()?;
        // Precompute every eligible application before the first structural
        // write or RNG sample, so an invalid/overflowing causal frontier cannot
        // leave a half-applied impact. Only actually deposited rooms commit it.
        let mut emp_applications = BTreeMap::new();
        if let (Some(payload), Some(policy)) = (hit.profile.emp, &definition.config.emp) {
            let clock = clock.ok_or("EMP impact requires the current fixed-step clock")?;
            for (system, components) in &definition.components {
                for (id, config) in components {
                    if policy.bindings.contains(&config.binding) {
                        let old = self.components[system][id].emp.as_ref();
                        emp_applications.insert(
                            ComponentRef {
                                system: system.clone(),
                                component: id.clone(),
                            },
                            phoenix_model::emp::ComponentEmpState::apply(
                                old, policy, payload, clock,
                            )?,
                        );
                    }
                }
            }
        }
        let norm = libm::sqrt(hit.incoming.iter().map(|v| (*v as f64) * (*v as f64)).sum());
        if !hit.incoming.iter().all(|v| v.is_finite())
            || norm == 0.0
            || !finite(hit.profile.penetration)
            || !hit.payload.is_finite()
            || hit.payload <= 0.0
            || !source(&hit.source)
            || hit
                .aim
                .as_ref()
                .is_some_and(|aim| !definition.rooms.contains_key(aim))
        {
            return Err("invalid physical damage incident".into());
        }
        if hit.sample.as_ref().is_some_and(|sample| {
            !definition.decks.iter().any(|(id, _)| id == &sample.deck)
                || !sample.profile_fraction.is_finite()
                || !(0.0..1.0).contains(&sample.profile_fraction)
        }) {
            return Err("invalid retained damage hit sample".into());
        }
        let sample = hit
            .sample
            .clone()
            .unwrap_or_else(|| definition.sample_hit(rng));
        let direction = [hit.incoming[0] as f64 / norm, hit.incoming[1] as f64 / norm];
        let normal = [-direction[1], direction[0]];
        let deck_rooms: Vec<_> = definition
            .rooms
            .values()
            .filter(|room| room.deck == sample.deck)
            .collect();
        let offset = if let Some(aim) = &hit.aim {
            let room = &definition.rooms[aim];
            normal[0] * (room.x as f64 + room.width as f64 * 0.5)
                + normal[1] * (room.z as f64 + room.depth as f64 * 0.5)
        } else {
            profile_offset(&deck_rooms, normal, sample.profile_fraction as f64)
        };
        let origin = [normal[0] * offset, normal[1] * offset];
        let mut intersections: Vec<_> = deck_rooms
            .into_iter()
            .filter_map(|room| {
                intersect(room, origin, direction).map(|(entry, exit)| (entry, exit, room))
            })
            .collect();
        intersections.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.id.cmp(&b.2.id)));
        let initial_penetration = hit.profile.penetration as f64;
        let mut penetration = initial_penetration;
        let mut payload = hit.payload;
        let mut result = DamageOutcome {
            sample,
            deposits: Vec::new(),
            changed_components: Vec::new(),
            structural_loss: 0.0,
            escaped_payload: hit.payload,
            remaining_penetration: penetration,
        };
        for (entry, exit, room) in intersections {
            if payload <= 0.0 {
                break;
            }
            let material = &definition.config.materials[&room.material];
            let state = self
                .compartments
                .get_mut(&room.id)
                .expect("compiled compartment");
            let resistance = material.resistance[&hit.profile.damage_type] as f64
                * (material.residual_fraction as f64
                    + (1.0 - material.residual_fraction as f64) * state.hp / room.max_hp as f64);
            let spent = penetration.min(resistance * (exit - entry));
            let deposit = if initial_penetration == 0.0 {
                payload
            } else {
                payload.min(hit.payload * (spent / initial_penetration))
            };
            penetration = (penetration - spent).max(0.0);
            payload = (payload - deposit).max(0.0);
            if deposit == 0.0 {
                continue;
            }
            let loss = state.hp.min(deposit);
            state.hp = (state.hp - loss).max(0.0);
            if state.hp == 0.0 {
                state.disabled = true;
            }
            self.deposit_dose(
                definition,
                &room.id,
                hit.profile.damage_type,
                deposit,
                &hit.source,
                &mut result.changed_components,
            );
            for (target, emp) in &emp_applications {
                if definition.interior.placements[&target.system] == room.id {
                    self.components
                        .get_mut(&target.system)
                        .expect("compiled component owner")
                        .get_mut(&target.component)
                        .expect("compiled component")
                        .emp = Some(emp.clone());
                }
            }
            result.deposits.push(CompartmentDeposit {
                compartment: room.id.clone(),
                payload: deposit,
                structural_loss: loss,
                penetration_spent: spent,
            });
            result.structural_loss += loss;
        }
        result.escaped_payload = payload;
        result.remaining_penetration = penetration;
        result.changed_components.sort();
        result.changed_components.dedup();
        Ok(result)
    }
    fn deposit_dose(
        &mut self,
        definition: &DamageDefinition,
        room: &str,
        kind: DamageType,
        amount: f64,
        source: &str,
        changed: &mut Vec<ComponentRef>,
    ) {
        let mut eligible = Vec::new();
        for (system, components) in &definition.components {
            for (id, config) in components {
                if definition.interior.placements[system] == room
                    && config.susceptibility[&kind] > 0.0
                {
                    eligible.push((
                        ComponentRef {
                            system: system.clone(),
                            component: id.clone(),
                        },
                        config.susceptibility[&kind] as f64,
                    ));
                }
            }
        }
        let total: f64 = eligible.iter().map(|(_, weight)| weight).sum();
        for (target, weight) in eligible {
            let config = definition.component(&target).expect("selected component");
            let state = self
                .components
                .get_mut(&target.system)
                .and_then(|c| c.get_mut(&target.component))
                .expect("compiled state");
            let threshold = config.fault_dose_threshold as f64;
            let dose = state.dose + amount * (weight / total);
            let crossings = libm::floor(dose / threshold);
            state.dose = dose % threshold;
            state.last_source = Some(source.to_owned());
            if crossings >= 1.0 && state.condition != ComponentCondition::Destroyed {
                state.condition = match state.condition {
                    ComponentCondition::Healthy if crossings < 2.0 => ComponentCondition::Damaged,
                    _ => ComponentCondition::Destroyed,
                };
                changed.push(target);
            }
        }
    }
}
fn profile_offset(rooms: &[&CompartmentConfig], normal: [f64; 2], fraction: f64) -> f64 {
    let mut intervals: Vec<_> = rooms
        .iter()
        .map(|room| {
            let values = [
                normal[0] * room.x as f64 + normal[1] * room.z as f64,
                normal[0] * (room.x as f64 + room.width as f64) + normal[1] * room.z as f64,
                normal[0] * room.x as f64 + normal[1] * (room.z as f64 + room.depth as f64),
                normal[0] * (room.x as f64 + room.width as f64)
                    + normal[1] * (room.z as f64 + room.depth as f64),
            ];
            (
                values.iter().copied().fold(f64::INFINITY, f64::min),
                values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            )
        })
        .collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut union: Vec<(f64, f64)> = Vec::new();
    for (low, high) in intervals {
        if let Some(last) = union.last_mut().filter(|last| low <= last.1) {
            last.1 = last.1.max(high);
        } else {
            union.push((low, high));
        }
    }
    let mut distance = union.iter().map(|(a, b)| b - a).sum::<f64>() * fraction;
    for (low, high) in &union {
        if distance < high - low {
            return low + distance;
        }
        distance -= high - low;
    }
    union.last().expect("compiled deck").1
}
fn intersect(
    room: &CompartmentConfig,
    origin: [f64; 2],
    direction: [f64; 2],
) -> Option<(f64, f64)> {
    let mut entry = f64::NEG_INFINITY;
    let mut exit = f64::INFINITY;
    for (axis, low, size) in [(0, room.x, room.width), (1, room.z, room.depth)] {
        let low = low as f64;
        let high = low + size as f64;
        if direction[axis] == 0.0 {
            if origin[axis] < low || origin[axis] >= high {
                return None;
            }
        } else {
            let a = (low - origin[axis]) / direction[axis];
            let b = (high - origin[axis]) / direction[axis];
            entry = entry.max(a.min(b));
            exit = exit.min(a.max(b));
        }
    }
    (exit > entry).then_some((entry, exit))
}

#[cfg(test)]
#[path = "compartment_damage_tests.rs"]
mod tests;

#[path = "compartment_damage_internal.rs"]
mod internal;
