//! Authoritative local energy routing. The caller supplies immutable authored
//! equipment, resolved stock bindings and the simulation's deterministic RNG.
//! No controller, second battery, ECS store or stock copy lives here.
use super::energy::{
    EnergyDemand, EnergyDemandKind, EnergyGrant, EnergyState, EnergyStep, EnergyStepResult,
};
use crate::core::messages::{PowerGroupId, SystemId};
use crate::entities::config::{EnergyConfigSection, EnergyConsumerConfig};
use crate::ship::config::ShipConfig;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

#[path = "energy_topology.rs"]
mod topology;
pub(crate) use topology::CompiledEnergyDefinition;

pub use phoenix_model::energy::{CoolingUsage, EnergyAllocationMode};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyGroupState {
    pub desired_flow_per_sec: f32,
    pub desired_cooling_per_sec: f32,
    pub priority: u16,
    pub effective_flow_per_sec: f32,
    pub effective_cooling_per_sec: f32,
    pub cut: bool,
    /// Time for which this cut's entire request has continuously fit available
    /// headroom. An infeasible tick resets it; already supplied groups stay put.
    pub restore_ready_secs: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyNetworkState {
    pub reactor: EnergyState,
    /// New generation only; stored reserve export and cooling remain ordinary.
    pub generation_enabled: bool,
    pub output_target_per_sec: f32,
    pub output_actual_per_sec: f32,
    pub reactor_cooling_per_sec: f32,
    pub mode: EnergyAllocationMode,
    pub cooling_usage: CoolingUsage,
    pub groups: BTreeMap<PowerGroupId, EnergyGroupState>,
    pub consumers: BTreeMap<SystemId, EnergyState>,
    pub fine_flows: BTreeMap<SystemId, f32>,
}

/// Prepared by the content/runtime owner through the ordinary service resolver.
/// A binding identifies actual capacity, never a cloned supply inventory.
pub struct EnergyCoolantBindings {
    pub reactor: String,
    pub consumers: BTreeMap<SystemId, String>,
}

/// Transient machinery effectiveness for one tick. Never persisted or written
/// into authored hardware; omitted equipment remains intact for pure fixtures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnergyLinkEffects {
    pub intake: f32,
    pub cooling: f32,
}
impl Default for EnergyLinkEffects {
    fn default() -> Self {
        Self {
            intake: 1.0,
            cooling: 1.0,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct EnergyHardwareEffects {
    pub reactor_operation: f32,
    pub reactor: EnergyLinkEffects,
    pub consumers: BTreeMap<SystemId, EnergyLinkEffects>,
}
impl Default for EnergyHardwareEffects {
    fn default() -> Self {
        Self {
            reactor_operation: 1.0,
            reactor: EnergyLinkEffects::default(),
            consumers: BTreeMap::new(),
        }
    }
}
impl EnergyHardwareEffects {
    fn validate(
        &self,
        config: &EnergyConfigSection,
        definition: Option<&CompiledEnergyDefinition>,
    ) -> Result<(), String> {
        for value in std::iter::once(self.reactor_operation)
            .chain([self.reactor.intake, self.reactor.cooling])
            .chain(
                self.consumers
                    .values()
                    .flat_map(|effects| [effects.intake, effects.cooling]),
            )
        {
            bounded("hardware effectiveness", value, 1.0)?;
        }
        if self.consumers.keys().any(|id| {
            definition.map_or_else(
                || {
                    !config
                        .consumers
                        .iter()
                        .any(|consumer| &consumer.system == id)
                },
                |definition| !definition.consumer_indices.contains_key(id),
            )
        }) {
            return Err("hardware effects name an uninstalled energy consumer".into());
        }
        Ok(())
    }
    fn consumer(&self, id: &SystemId) -> EnergyLinkEffects {
        self.consumers.get(id).copied().unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnergyNetworkResult {
    pub generation_accepted: f32,
    pub generation_rejected: f32,
    pub delivered_energy: f32,
    /// Conservative source rounding, bounded to one representable energy unit
    /// at the aggregate's magnitude; never manufactures consumer energy.
    pub rounding_energy_loss: f32,
    pub coolant_used: BTreeMap<String, f32>,
    pub consumer_reports: BTreeMap<SystemId, EnergyStepResult>,
    pub cut_groups: Vec<PowerGroupId>,
    pub restored_groups: Vec<PowerGroupId>,
}

fn nonnegative(name: &str, value: f32) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and nonnegative"))
    }
}

/// Never round an available budget upward while converting wider accounting to
/// stored f32 machinery units. Zero is the smallest supported spendable amount.
fn budget(value: f64) -> f32 {
    let result = value.max(0.0).min(f32::MAX as f64) as f32;
    if result as f64 > value && result > 0.0 {
        f32::from_bits(result.to_bits() - 1)
    } else {
        result
    }
}

fn bounded(name: &str, value: f32, maximum: f32) -> Result<(), String> {
    nonnegative(name, value)?;
    nonnegative("maximum", maximum)?;
    if value > maximum {
        Err(format!("{name} exceeds authored maximum"))
    } else {
        Ok(())
    }
}

impl EnergyNetworkState {
    pub fn new(config: &EnergyConfigSection, ship: &ShipConfig) -> Result<Self, String> {
        let reactor = EnergyState::new(
            &config.reactor.reserve,
            config.reactor.initial_energy,
            config.reactor.initial_heat,
        )?;
        let state = Self {
            reactor,
            generation_enabled: true,
            output_target_per_sec: config.reactor.initial_output_per_sec,
            output_actual_per_sec: if reactor.thermal_stopped {
                0.0
            } else {
                config.reactor.initial_output_per_sec
            },
            reactor_cooling_per_sec: config.reactor.default_cooling_per_sec,
            mode: EnergyAllocationMode::Manual,
            cooling_usage: CoolingUsage::Low,
            groups: config
                .groups
                .iter()
                .map(|group| {
                    (
                        group.id.clone(),
                        EnergyGroupState {
                            desired_flow_per_sec: group.default_flow_per_sec,
                            desired_cooling_per_sec: group.default_cooling_per_sec,
                            priority: group.priority,
                            effective_flow_per_sec: 0.0,
                            effective_cooling_per_sec: 0.0,
                            cut: false,
                            restore_ready_secs: 0.0,
                        },
                    )
                })
                .collect(),
            consumers: config
                .consumers
                .iter()
                .map(|consumer| {
                    Ok((
                        consumer.system.clone(),
                        EnergyState::new(
                            &consumer.reserve,
                            consumer.initial_energy,
                            consumer.initial_heat,
                        )?,
                    ))
                })
                .collect::<Result<_, String>>()?,
            fine_flows: BTreeMap::new(),
        };
        state.validate_against(config, ship)?;
        Ok(state)
    }

    pub fn validate_against(
        &self,
        config: &EnergyConfigSection,
        ship: &ShipConfig,
    ) -> Result<(), String> {
        self.reactor.validate_against(&config.reactor.reserve)?;
        bounded(
            "output target",
            self.output_target_per_sec,
            config.reactor.max_output_per_sec,
        )?;
        bounded(
            "output actual",
            self.output_actual_per_sec,
            config.reactor.max_output_per_sec,
        )?;
        bounded(
            "reactor cooling",
            self.reactor_cooling_per_sec,
            config.reactor.max_cooling_per_sec,
        )?;
        if self.reactor.thermal_stopped && self.output_actual_per_sec != 0.0 {
            return Err("thermally stopped reactor cannot report actual output".into());
        }
        nonnegative(
            "sustainable output",
            config.reactor.sustainable_output_per_sec,
        )?;
        nonnegative("reactor ramp", config.reactor.ramp_per_sec)?;
        if config.reactor.ramp_per_sec <= 0.0
            || config.reactor.sustainable_output_per_sec > config.reactor.max_output_per_sec
        {
            return Err("invalid reactor generation or ramp".into());
        }
        let group_ids: BTreeSet<_> = config.groups.iter().map(|group| &group.id).collect();
        let consumer_ids: BTreeSet<_> = config
            .consumers
            .iter()
            .map(|consumer| &consumer.system)
            .collect();
        if group_ids.len() != config.groups.len()
            || consumer_ids.len() != config.consumers.len()
            || self.groups.keys().ne(group_ids.iter().copied())
            || self.consumers.keys().ne(consumer_ids.iter().copied())
        {
            return Err("energy state does not match authored group/consumer identities".into());
        }
        for group in &config.groups {
            if !ship.power_groups.contains_key(&group.id) {
                return Err("energy group is not declared on ship".into());
            }
            nonnegative("restore delay", group.restore_delay_secs)?;
            let state = &self.groups[&group.id];
            bounded(
                "desired flow",
                state.desired_flow_per_sec,
                group.max_flow_per_sec,
            )?;
            bounded(
                "effective flow",
                state.effective_flow_per_sec,
                group.max_flow_per_sec,
            )?;
            bounded(
                "desired cooling",
                state.desired_cooling_per_sec,
                group.max_cooling_per_sec,
            )?;
            bounded(
                "effective cooling",
                state.effective_cooling_per_sec,
                group.max_cooling_per_sec,
            )?;
            bounded(
                "restore readiness",
                state.restore_ready_secs,
                group.restore_delay_secs,
            )?;
            if state.cut && state.effective_flow_per_sec != 0.0 {
                return Err("cut group cannot have effective incoming flow".into());
            }
            let children: Vec<_> = config
                .groups
                .iter()
                .filter(|child| child.parent.as_ref() == Some(&group.id))
                .collect();
            let child_flow = children
                .iter()
                .map(|child| self.groups[&child.id].effective_flow_per_sec as f64)
                .sum::<f64>();
            let child_cooling = children
                .iter()
                .map(|child| self.groups[&child.id].effective_cooling_per_sec as f64)
                .sum::<f64>();
            if child_flow > state.effective_flow_per_sec as f64
                || child_cooling > state.effective_cooling_per_sec as f64
            {
                return Err("saved child allocations exceed parent budget".into());
            }
            let mut seen = BTreeSet::from([&group.id]);
            let mut parent = group.parent.as_ref();
            while let Some(id) = parent {
                if !seen.insert(id) {
                    return Err("energy group hierarchy contains a cycle".into());
                }
                parent = config
                    .groups
                    .iter()
                    .find(|candidate| &candidate.id == id)
                    .ok_or("energy group has unknown parent")?
                    .parent
                    .as_ref();
            }
        }
        for consumer in &config.consumers {
            self.consumers[&consumer.system].validate_against(&consumer.reserve)?;
            let system = ship
                .systems
                .iter()
                .find(|system| system.id == consumer.system)
                .ok_or("energy consumer System is not declared")?;
            if system
                .power_group
                .as_ref()
                .is_some_and(|group| !self.groups.contains_key(group))
            {
                return Err("consumer has unknown allocation group".into());
            }
            nonnegative("continuous cost", consumer.continuous_cost_per_sec)?;
            bounded(
                "discrete cost",
                consumer.discrete_cost,
                consumer.reserve.capacity,
            )?;
        }
        for (system, flow) in &self.fine_flows {
            let consumer = config
                .consumers
                .iter()
                .find(|consumer| &consumer.system == system)
                .ok_or("fine flow has unknown consumer")?;
            bounded("fine flow", *flow, consumer.reserve.max_intake_per_sec)?;
            let group_id = ship
                .systems
                .iter()
                .find(|instance| &instance.id == system)
                .and_then(|instance| instance.power_group.as_ref())
                .ok_or("fine flow consumer has no allocation group")?;
            let group = config
                .groups
                .iter()
                .find(|group| &group.id == group_id)
                .ok_or("fine flow has unknown group")?;
            bounded("fine flow", *flow, group.max_flow_per_sec)?;
        }
        Ok(())
    }

    pub fn set_output(&mut self, config: &EnergyConfigSection, output: f32) -> Result<(), String> {
        bounded("output target", output, config.reactor.max_output_per_sec)?;
        self.output_target_per_sec = output;
        Ok(())
    }
    pub fn set_reactor_cooling(
        &mut self,
        config: &EnergyConfigSection,
        cooling: f32,
    ) -> Result<(), String> {
        bounded(
            "reactor cooling",
            cooling,
            config.reactor.max_cooling_per_sec,
        )?;
        self.reactor_cooling_per_sec = cooling;
        Ok(())
    }
    pub fn set_group(
        &mut self,
        config: &EnergyConfigSection,
        id: &PowerGroupId,
        flow: f32,
        cooling: f32,
    ) -> Result<(), String> {
        let group = config
            .groups
            .iter()
            .find(|group| &group.id == id)
            .ok_or("unknown allocation group")?;
        bounded("desired flow", flow, group.max_flow_per_sec)?;
        bounded("desired cooling", cooling, group.max_cooling_per_sec)?;
        let state = self
            .groups
            .get_mut(id)
            .ok_or("unknown allocation group state")?;
        if state.desired_flow_per_sec != flow {
            state.restore_ready_secs = 0.0;
        }
        state.desired_flow_per_sec = flow;
        state.desired_cooling_per_sec = cooling;
        Ok(())
    }
    pub fn set_priority(&mut self, id: &PowerGroupId, priority: u16) -> Result<(), String> {
        self.groups
            .get_mut(id)
            .ok_or("unknown allocation group")?
            .priority = priority;
        Ok(())
    }
    pub fn set_fine_flow(
        &mut self,
        config: &EnergyConfigSection,
        ship: &ShipConfig,
        id: &SystemId,
        flow: Option<f32>,
    ) -> Result<(), String> {
        if !self.consumers.contains_key(id) {
            return Err("unknown energy consumer".into());
        }
        let mut next = self.clone();
        match flow {
            Some(value) => {
                next.fine_flows.insert(id.clone(), value);
            }
            None => {
                next.fine_flows.remove(id);
            }
        }
        next.validate_against(config, ship)?;
        *self = next;
        Ok(())
    }
    pub fn spend_continuous(
        &mut self,
        config: &EnergyConfigSection,
        id: &SystemId,
        requested_energy: f32,
    ) -> Result<EnergyGrant, String> {
        let consumer = config
            .consumers
            .iter()
            .find(|consumer| &consumer.system == id)
            .ok_or("unknown energy consumer")?;
        self.consumers
            .get_mut(id)
            .ok_or("unknown consumer state")?
            .spend_continuous(&consumer.reserve, requested_energy)
    }
    pub fn try_spend_discrete(
        &mut self,
        config: &EnergyConfigSection,
        id: &SystemId,
    ) -> Result<bool, String> {
        let consumer = config
            .consumers
            .iter()
            .find(|consumer| &consumer.system == id)
            .ok_or("unknown energy consumer")?;
        self.consumers
            .get_mut(id)
            .ok_or("unknown consumer state")?
            .try_spend_discrete(&consumer.reserve, consumer.discrete_cost)
    }

    /// Inputs are read once; the caller debits reported coolant from real
    /// stocks and advances its saved fractional remainder after success.
    pub fn tick(
        &mut self,
        config: &EnergyConfigSection,
        ship: &ShipConfig,
        dt: f32,
        bindings: &EnergyCoolantBindings,
        coolant: &BTreeMap<String, f32>,
        choose: &mut impl FnMut(usize) -> usize,
    ) -> Result<EnergyNetworkResult, String> {
        self.tick_with_effects(
            config,
            ship,
            dt,
            bindings,
            coolant,
            &EnergyHardwareEffects::default(),
            choose,
        )
    }

    // The processor receives authored hardware, routing, finite stock and one transient effect view.
    #[allow(clippy::too_many_arguments)]
    pub fn tick_with_effects(
        &mut self,
        config: &EnergyConfigSection,
        ship: &ShipConfig,
        dt: f32,
        bindings: &EnergyCoolantBindings,
        coolant: &BTreeMap<String, f32>,
        effects: &EnergyHardwareEffects,
        choose: &mut impl FnMut(usize) -> usize,
    ) -> Result<EnergyNetworkResult, String> {
        let mut next = self.clone();
        let result =
            next.tick_with_effects_in_place(config, ship, dt, bindings, coolant, effects, choose)?;
        *self = next;
        Ok(result)
    }

    /// Integrate a disposable transaction candidate owned by PowerSystem.
    /// A failure may leave this candidate modified; callers must discard it.
    /// Public network callers retain the atomic clone/commit wrapper above.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tick_with_effects_in_place(
        &mut self,
        config: &EnergyConfigSection,
        ship: &ShipConfig,
        dt: f32,
        bindings: &EnergyCoolantBindings,
        coolant: &BTreeMap<String, f32>,
        effects: &EnergyHardwareEffects,
        choose: &mut impl FnMut(usize) -> usize,
    ) -> Result<EnergyNetworkResult, String> {
        self.tick_inner(config, ship, None, dt, bindings, coolant, effects, choose)
    }

    /// Only Power can retain an immutable definition; the supplied definition
    /// itself owns the exact config/ship used for every index in this call.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tick_with_definition_in_place(
        &mut self,
        definition: &CompiledEnergyDefinition,
        dt: f32,
        bindings: &EnergyCoolantBindings,
        coolant: &BTreeMap<String, f32>,
        effects: &EnergyHardwareEffects,
        choose: &mut impl FnMut(usize) -> usize,
    ) -> Result<EnergyNetworkResult, String> {
        self.tick_inner(
            definition.config(),
            definition.ship(),
            Some(definition),
            dt,
            bindings,
            coolant,
            effects,
            choose,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn tick_inner(
        &mut self,
        config: &EnergyConfigSection,
        ship: &ShipConfig,
        definition: Option<&CompiledEnergyDefinition>,
        dt: f32,
        bindings: &EnergyCoolantBindings,
        coolant: &BTreeMap<String, f32>,
        effects: &EnergyHardwareEffects,
        choose: &mut impl FnMut(usize) -> usize,
    ) -> Result<EnergyNetworkResult, String> {
        effects.validate(config, definition)?;
        match definition {
            Some(definition) => definition.validate_state(self)?,
            None => self.validate_against(config, ship)?,
        }
        nonnegative("dt", dt)?;
        if bindings.reactor.trim().is_empty()
            || bindings.consumers.keys().ne(self.consumers.keys())
            || bindings
                .consumers
                .values()
                .any(|value| value.trim().is_empty())
        {
            return Err("coolant bindings do not match equipment".into());
        }
        for value in coolant.values() {
            nonnegative("coolant budget", *value)?;
        }
        if !coolant.contains_key(&bindings.reactor)
            || bindings
                .consumers
                .values()
                .any(|id| !coolant.contains_key(id))
        {
            return Err("coolant binding has no provided budget".into());
        }
        if dt == 0.0 {
            return Ok(EnergyNetworkResult::default());
        }
        let mut result = EnergyNetworkResult::default();
        let ramp = config.reactor.ramp_per_sec as f64 * dt as f64;
        let actual = self.output_actual_per_sec as f64;
        let target = self.output_target_per_sec as f64;
        self.output_actual_per_sec = if self.reactor.thermal_stopped {
            0.0
        } else if target > actual {
            (actual + ramp).min(target) as f32
        } else {
            (actual - ramp).max(target) as f32
        };
        self.output_actual_per_sec = self
            .output_actual_per_sec
            .min(config.reactor.max_output_per_sec * effects.reactor_operation);
        let generation = if !self.generation_enabled || self.reactor.thermal_stopped {
            0.0
        } else {
            budget(
                self.output_actual_per_sec
                    .min(config.reactor.sustainable_output_per_sec * effects.reactor_operation)
                    as f64
                    * effects.reactor.intake as f64
                    * dt as f64,
            )
        };
        let mut supply_preview = self.reactor;
        supply_preview.step(
            &config.reactor.reserve,
            EnergyStep {
                dt,
                offered_energy: generation,
                demands: &[],
                cooling_rate: 0.0,
                coolant_available: 0.0,
            },
        )?;
        let available_rate = if supply_preview.thermal_stopped {
            0.0
        } else {
            budget(
                (supply_preview.energy as f64 / dt as f64).min(self.output_actual_per_sec as f64),
            )
        };
        self.route_groups(config, definition, available_rate, dt, choose, &mut result)?;
        let mut routed = BTreeMap::new();
        let mut cooling_rates = BTreeMap::new();
        for group in config.groups.iter().filter(|group| group.parent.is_none()) {
            self.route_consumers(
                config,
                ship,
                definition,
                &group.id,
                &mut routed,
                &mut cooling_rates,
            )?;
        }
        let mut remaining_coolant: BTreeMap<_, _> = coolant
            .iter()
            .map(|(id, value)| (id.clone(), *value as f64))
            .collect();
        let mut coolant_spent = BTreeMap::<String, f64>::new();
        let mut remaining_supply = supply_preview.energy as f64;
        let mut delivered = 0.0f64;
        // Priority is an authored allocation policy, not an engine privilege.
        let mut consumers: Vec<_> = config
            .consumers
            .iter()
            .map(|consumer| {
                (
                    consumer_priority(config, ship, definition, &self.groups, consumer),
                    consumer,
                )
            })
            .collect();
        consumers.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.system.cmp(&b.1.system)));
        for (_, consumer) in consumers {
            let offer = budget(
                (routed.get(&consumer.system).copied().unwrap_or(0.0) as f64
                    * effects.consumer(&consumer.system).intake as f64
                    * dt as f64)
                    .min(remaining_supply),
            );
            let provider = &bindings.consumers[&consumer.system];
            let report = self
                .consumers
                .get_mut(&consumer.system)
                .expect("validated consumer")
                .step(
                    &consumer.reserve,
                    EnergyStep {
                        dt,
                        offered_energy: offer,
                        demands: &[],
                        cooling_rate: cooling_rates.get(&consumer.system).copied().unwrap_or(0.0)
                            * effects.consumer(&consumer.system).cooling,
                        coolant_available: budget(remaining_coolant[provider]),
                    },
                )?;
            remaining_supply -= report.accepted_energy as f64;
            delivered += report.accepted_energy as f64;
            if let Some(value) = remaining_coolant.get_mut(provider) {
                *value -= report.coolant_used as f64;
            }
            *coolant_spent.entry(provider.clone()).or_default() += report.coolant_used as f64;
            result
                .consumer_reports
                .insert(consumer.system.clone(), report);
        }
        // Exactly one reactor step accounts generation, reserve loss, aggregate
        // accepted exports and cooling. The earlier preview never commits.
        let lower_export = budget(delivered);
        let export = if (lower_export as f64) < delivered {
            f32::from_bits(lower_export.to_bits() + 1)
        } else {
            lower_export
        };
        result.rounding_energy_loss = (export as f64 - delivered) as f32;
        let work = [EnergyDemand {
            id: "distribution-export".into(),
            priority: 0,
            kind: EnergyDemandKind::Continuous,
            amount: export,
        }];
        let reactor_cooling = match self.mode {
            EnergyAllocationMode::Manual => self.reactor_cooling_per_sec,
            EnergyAllocationMode::Minimal => match self.cooling_usage {
                CoolingUsage::None => 0.0,
                CoolingUsage::Low => config.reactor.default_cooling_per_sec,
                CoolingUsage::High => config.reactor.max_cooling_per_sec,
            },
        };
        let reactor_report = self.reactor.step(
            &config.reactor.reserve,
            EnergyStep {
                dt,
                offered_energy: generation,
                demands: &work,
                cooling_rate: reactor_cooling * effects.reactor.cooling,
                coolant_available: budget(remaining_coolant[&bindings.reactor]),
            },
        )?;
        if reactor_report.work_energy != export {
            return Err("source cannot supply accepted consumer energy".into());
        }
        *coolant_spent.entry(bindings.reactor.clone()).or_default() +=
            reactor_report.coolant_used as f64;
        // Sum actual shared-provider reports in wide arithmetic, then round
        // the debit upward. Rounding down would manufacture fractional stock
        // credit in the integer-inventory adapter over repeated cooling ticks.
        for (id, amount) in coolant_spent {
            let lower = budget(amount);
            let debit = if (lower as f64) < amount {
                f32::from_bits(lower.to_bits() + 1)
            } else {
                lower
            };
            if !debit.is_finite() || debit > coolant[&id] {
                return Err("aggregate cooling exceeds its provider budget".into());
            }
            result.coolant_used.insert(id, debit);
        }
        result.generation_accepted = reactor_report.accepted_energy;
        result.generation_rejected = reactor_report.rejected_energy;
        result.delivered_energy = export;
        if self.reactor.thermal_stopped {
            self.output_actual_per_sec = 0.0;
        }
        match definition {
            Some(definition) => definition.validate_state(self)?,
            None => self.validate_against(config, ship)?,
        }
        Ok(result)
    }

    fn route_groups(
        &mut self,
        config: &EnergyConfigSection,
        definition: Option<&CompiledEnergyDefinition>,
        available_rate: f32,
        dt: f32,
        choose: &mut impl FnMut(usize) -> usize,
        result: &mut EnergyNetworkResult,
    ) -> Result<(), String> {
        let roots = definition.map_or_else(
            || {
                Cow::Owned(
                    config
                        .groups
                        .iter()
                        .filter(|group| group.parent.is_none())
                        .map(|group| group.id.clone())
                        .collect(),
                )
            },
            |definition| Cow::Borrowed(definition.roots.as_slice()),
        );
        self.route_siblings(
            config,
            definition,
            &roots,
            available_rate,
            dt,
            choose,
            result,
        )?;
        for root in roots.iter() {
            self.route_children(config, definition, root, dt, choose, result)?;
        }
        Ok(())
    }
    fn route_children(
        &mut self,
        config: &EnergyConfigSection,
        definition: Option<&CompiledEnergyDefinition>,
        parent: &PowerGroupId,
        dt: f32,
        choose: &mut impl FnMut(usize) -> usize,
        result: &mut EnergyNetworkResult,
    ) -> Result<(), String> {
        let children = child_ids(config, parent, definition);
        let available = self.groups[parent].effective_flow_per_sec;
        self.route_siblings(config, definition, &children, available, dt, choose, result)?;
        let mut cooling_left = self.groups[parent].effective_cooling_per_sec as f64;
        let mut ordered = children.to_vec();
        ordered.sort_by(|a, b| {
            self.groups[a]
                .priority
                .cmp(&self.groups[b].priority)
                .then_with(|| a.cmp(b))
        });
        for child in &ordered {
            let state = self.groups.get_mut(child).expect("validated group");
            state.effective_cooling_per_sec =
                budget((state.effective_cooling_per_sec as f64).min(cooling_left));
            cooling_left -= state.effective_cooling_per_sec as f64;
        }
        for child in children.iter() {
            self.route_children(config, definition, child, dt, choose, result)?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn route_siblings(
        &mut self,
        config: &EnergyConfigSection,
        definition: Option<&CompiledEnergyDefinition>,
        ids: &[PowerGroupId],
        available: f32,
        dt: f32,
        choose: &mut impl FnMut(usize) -> usize,
        result: &mut EnergyNetworkResult,
    ) -> Result<(), String> {
        let requested = |state: &EnergyGroupState| state.desired_flow_per_sec as f64;
        if self.mode == EnergyAllocationMode::Minimal {
            let mut remaining = available as f64;
            let mut ordered = ids.to_vec();
            ordered.sort_by(|a, b| {
                self.groups[a]
                    .priority
                    .cmp(&self.groups[b].priority)
                    .then_with(|| a.cmp(b))
            });
            for id in ordered {
                let state = self.groups.get_mut(&id).expect("validated group");
                state.effective_flow_per_sec = budget(requested(state).min(remaining));
                remaining -= state.effective_flow_per_sec as f64;
                state.cut = false;
                state.restore_ready_secs = 0.0;
            }
        } else {
            let mut served = ids
                .iter()
                .filter(|id| !self.groups[*id].cut)
                .map(|id| requested(&self.groups[id]))
                .sum::<f64>();
            while served > available as f64 {
                let eligible: Vec<_> = ids
                    .iter()
                    .filter(|id| {
                        !self.groups[*id].cut && self.groups[*id].desired_flow_per_sec > 0.0
                    })
                    .collect();
                let id = eligible
                    .get(choose(eligible.len()))
                    .ok_or("cut chooser returned an invalid index")?;
                let state = self.groups.get_mut(*id).expect("validated group");
                state.cut = true;
                state.restore_ready_secs = 0.0;
                result.cut_groups.push((*id).clone());
                served = ids
                    .iter()
                    .filter(|id| !self.groups[*id].cut)
                    .map(|id| requested(&self.groups[id]))
                    .sum::<f64>();
            }
            for id in ids {
                let state = self.groups.get_mut(id).expect("validated group");
                if state.cut {
                    let delay = group_definition(config, id, definition).restore_delay_secs;
                    state.restore_ready_secs = if served + requested(state) <= available as f64 {
                        (state.restore_ready_secs as f64 + dt as f64).min(delay as f64) as f32
                    } else {
                        0.0
                    };
                }
            }
            loop {
                let feasible: Vec<_> = ids
                    .iter()
                    .filter(|id| {
                        let state = &self.groups[*id];
                        let delay = group_definition(config, id, definition).restore_delay_secs;
                        state.cut
                            && state.restore_ready_secs >= delay
                            && served + requested(state) <= available as f64
                    })
                    .collect();
                if feasible.is_empty() {
                    break;
                }
                let id = feasible
                    .get(choose(feasible.len()))
                    .ok_or("restore chooser returned an invalid index")?;
                let state = self.groups.get_mut(*id).expect("validated group");
                state.cut = false;
                state.restore_ready_secs = 0.0;
                served += requested(state);
                result.restored_groups.push((*id).clone());
            }
            for id in ids {
                let state = self.groups.get_mut(id).expect("validated group");
                if state.cut && served + requested(state) > available as f64 {
                    state.restore_ready_secs = 0.0;
                }
                state.effective_flow_per_sec = if state.cut {
                    0.0
                } else {
                    state.desired_flow_per_sec
                };
            }
        }
        for id in ids {
            let group = group_definition(config, id, definition);
            let state = self.groups.get_mut(id).expect("validated group");
            state.effective_cooling_per_sec = match self.mode {
                EnergyAllocationMode::Manual => state.desired_cooling_per_sec,
                EnergyAllocationMode::Minimal => match self.cooling_usage {
                    CoolingUsage::None => 0.0,
                    CoolingUsage::Low => group.default_cooling_per_sec,
                    CoolingUsage::High => group.max_cooling_per_sec,
                },
            };
        }
        Ok(())
    }

    fn route_consumers(
        &self,
        config: &EnergyConfigSection,
        ship: &ShipConfig,
        definition: Option<&CompiledEnergyDefinition>,
        group: &PowerGroupId,
        flows: &mut BTreeMap<SystemId, f32>,
        cooling: &mut BTreeMap<SystemId, f32>,
    ) -> Result<(), String> {
        let children = child_ids(config, group, definition);
        let mut flow_left = (self.groups[group].effective_flow_per_sec as f64
            - children
                .iter()
                .map(|child| self.groups[child].effective_flow_per_sec as f64)
                .sum::<f64>())
        .max(0.0);
        let mut cooling_left = (self.groups[group].effective_cooling_per_sec as f64
            - children
                .iter()
                .map(|child| self.groups[child].effective_cooling_per_sec as f64)
                .sum::<f64>())
        .max(0.0);
        let consumers: Vec<_> = if let Some(definition) = definition {
            definition.group_consumers[definition.group_indices[group]]
                .iter()
                .map(|index| &config.consumers[*index])
                .collect()
        } else {
            let mut consumers: Vec<_> = config
                .consumers
                .iter()
                .filter(|consumer| {
                    ship.systems.iter().any(|system| {
                        system.id == consumer.system && system.power_group.as_ref() == Some(group)
                    })
                })
                .collect();
            consumers.sort_by(|a, b| a.system.cmp(&b.system));
            consumers
        };
        // Explicit fine requests reserve first; unavailable acceptance is not
        // reassigned after routing. Coarse adaptive weights reflect stored
        // reserve deficit and authored supported operating demand.
        for consumer in &consumers {
            if self.mode == EnergyAllocationMode::Manual {
                if let Some(request) = self.fine_flows.get(&consumer.system) {
                    if *request as f64 <= flow_left {
                        flows.insert(consumer.system.clone(), *request);
                        flow_left -= *request as f64;
                    } else {
                        flows.insert(consumer.system.clone(), 0.0);
                    }
                }
            }
        }
        let adaptive: Vec<_> = consumers
            .iter()
            .filter(|consumer| !flows.contains_key(&consumer.system))
            .collect();
        let weights: Vec<_> = adaptive
            .iter()
            .map(|consumer| {
                let state = &self.consumers[&consumer.system];
                let deficit =
                    (consumer.reserve.capacity - state.energy).max(0.0) / consumer.reserve.capacity;
                (consumer.continuous_cost_per_sec + consumer.reserve.max_intake_per_sec * deficit)
                    .min(consumer.reserve.max_intake_per_sec) as f64
            })
            .collect();
        let total = weights.iter().sum::<f64>();
        let assigned_budget = flow_left;
        for (consumer, weight) in adaptive.into_iter().zip(weights) {
            let grant = if total > 0.0 {
                budget(
                    (assigned_budget * weight / total)
                        .min(flow_left)
                        .min(consumer.reserve.max_intake_per_sec as f64),
                )
            } else {
                0.0
            };
            flows.insert(consumer.system.clone(), grant);
            flow_left -= grant as f64;
        }
        let thermal_total = consumers
            .iter()
            .map(|consumer| self.consumers[&consumer.system].heat as f64)
            .sum::<f64>();
        let cooling_budget = cooling_left;
        for consumer in consumers {
            let heat = self.consumers[&consumer.system].heat as f64;
            let grant = if thermal_total > 0.0 {
                budget((cooling_budget * heat / thermal_total).min(cooling_left))
            } else {
                0.0
            };
            cooling.insert(consumer.system.clone(), grant);
            cooling_left -= grant as f64;
        }
        for child in children.iter() {
            self.route_consumers(config, ship, definition, child, flows, cooling)?;
        }
        Ok(())
    }
}

fn child_ids<'a>(
    config: &'a EnergyConfigSection,
    parent: &PowerGroupId,
    definition: Option<&'a CompiledEnergyDefinition>,
) -> Cow<'a, [PowerGroupId]> {
    definition.map_or_else(
        || {
            Cow::Owned(
                config
                    .groups
                    .iter()
                    .filter(|group| group.parent.as_ref() == Some(parent))
                    .map(|group| group.id.clone())
                    .collect(),
            )
        },
        |definition| {
            Cow::Borrowed(definition.children[definition.group_indices[parent]].as_slice())
        },
    )
}

fn group_definition<'a>(
    config: &'a EnergyConfigSection,
    id: &PowerGroupId,
    definition: Option<&CompiledEnergyDefinition>,
) -> &'a crate::entities::config::EnergyGroupConfig {
    match definition {
        Some(definition) => &config.groups[definition.group_indices[id]],
        None => config
            .groups
            .iter()
            .find(|group| &group.id == id)
            .expect("validated group"),
    }
}

/// Sibling list rank is meaningful within its parent. Resolve the whole path
/// so a low-ranked branch cannot take scarce stock ahead of a preferred root.
/// A directly mounted parent consumer sorts before its descendants; its route
/// already contains only the budget left after those child allocations.
fn consumer_priority<'a>(
    config: &'a EnergyConfigSection,
    ship: &ShipConfig,
    definition: Option<&'a CompiledEnergyDefinition>,
    groups: &BTreeMap<PowerGroupId, EnergyGroupState>,
    consumer: &EnergyConsumerConfig,
) -> Vec<(u16, &'a PowerGroupId)> {
    if let Some(definition) = definition {
        return definition.consumer_paths[definition.consumer_indices[&consumer.system]]
            .iter()
            .map(|index| {
                let id = &config.groups[*index].id;
                (groups[id].priority, id)
            })
            .collect();
    }
    let mut current = ship
        .systems
        .iter()
        .find(|system| system.id == consumer.system)
        .and_then(|system| system.power_group.as_ref())
        .and_then(|id| config.groups.iter().find(|group| &group.id == id));
    let mut path = Vec::new();
    while let Some(group) = current {
        path.push((groups[&group.id].priority, &group.id));
        current = group
            .parent
            .as_ref()
            .and_then(|id| config.groups.iter().find(|parent| &parent.id == id));
    }
    path.reverse();
    path
}

#[cfg(test)]
#[path = "energy_network_tests.rs"]
mod tests;
