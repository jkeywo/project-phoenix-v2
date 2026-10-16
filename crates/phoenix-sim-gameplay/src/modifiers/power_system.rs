//! Compatibility Power identity over the single local-energy implementation.
//! Legacy rungs are read/write translations of incoming flow, never simulation
//! stores, output multipliers or an exhaustion lock. Production constructs this
//! owner from validated authored energy hardware; fixture conveniences create
//! explicit hardware using the same processor.
use crate::core::messages::{PowerGroupId, SystemId};
use crate::entities::config::{
    EnergyConfigSection, EnergyConsumerConfig, EnergyGroupConfig, EnergyReactorConfig,
};
use crate::resources::energy::EnergyConfig;
use crate::resources::energy_network::{
    CoolingUsage, EnergyAllocationMode, EnergyCoolantBindings, EnergyNetworkResult,
    EnergyNetworkState,
};
use crate::resources::service::{ResourceEndpoint, ResourceLink};
use crate::ship::config::{PowerGroupConfig, ShipConfig, SystemInstanceConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const HELM_POWER_GROUP: &str = "helm";
pub const SHIELDS_POWER_GROUP: &str = "shields";
pub const POWER_GROUP_ORDER: &[&str] =
    &[HELM_POWER_GROUP, WEAPONS_POWER_GROUP, SHIELDS_POWER_GROUP];
pub const GROUP_LEVEL_MIN: u8 = crate::ship::config::default_min_power_level();
pub const GROUP_LEVEL_MAX: u8 = crate::ship::config::default_max_power_level();

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerState {
    pub network: EnergyNetworkState,
    #[serde(default)]
    pub coolant_credit: BTreeMap<String, f64>,
    pub weapons_restrained: bool,
    pub last_reactor_energy_delta_per_sec: f32,
    pub last_intake_per_sec: BTreeMap<SystemId, f32>,
    pub last_work_fraction: BTreeMap<SystemId, f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike_boost: Option<crate::modifiers::strike_reserve::StrikeBoost>,
}
impl Default for PowerState {
    fn default() -> Self {
        PowerSystem::default().capture_continuation()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerAllocationError {
    UnknownGroup(PowerGroupId),
    InvalidRequest(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingGroupPower {
    pub uuid: String,
    pub held: bool,
}

/// Legacy payload projection. None of these fields is independently mutable
/// simulation state; consumers of output multipliers are retired in increment 8.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerReadState {
    pub allocations: Vec<(PowerGroupId, u8)>,
    pub battery_charge: f32,
    pub locked: bool,
}
impl PowerReadState {
    pub fn level_for_group(&self, group: &PowerGroupId) -> Option<u8> {
        self.allocations
            .iter()
            .find(|(id, _)| id == group)
            .map(|(_, level)| *level)
    }
}
pub struct Channel1Read<'a> {
    state: &'a PowerReadState,
}
impl<'a> Channel1Read<'a> {
    pub fn new(state: &'a PowerReadState) -> Self {
        Self { state }
    }
    pub fn power_level(&self, group: &PowerGroupId) -> Option<u8> {
        self.state.level_for_group(group)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredPowerGroup {
    pub id: PowerGroupId,
    pub level: u8,
    pub floor: u8,
}
impl AuthoredPowerGroup {
    pub fn at_default_floor(id: PowerGroupId, level: u8) -> Self {
        Self {
            id,
            level,
            floor: GROUP_LEVEL_MIN,
        }
    }
}

/// Read-only legacy syntax retained for compiler fixtures and explicit weapon
/// overdrive configuration. Its rates and threshold never drive new physics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrikeReserveConfig {
    #[serde(default)]
    pub ai_enable_at: Option<f32>,
    pub group: String,
    pub units_per_level: f32,
    #[serde(default)]
    pub weapons: BTreeMap<String, crate::modifiers::strike_reserve::StrikeWeaponConfig>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PowerConfig {
    pub strike_reserve: Option<StrikeReserveConfig>,
    pub capacity: f32,
    pub rates: [f32; 6],
    pub sustainable_total: u8,
    pub max_commanded_total: u8,
    pub emergency_threshold: f32,
}
impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            strike_reserve: None,
            capacity: 100.0,
            rates: [6.0, 5.0, 4.0, 2.0, -2.0, -6.0],
            sustainable_total: 6,
            max_commanded_total: 8,
            emergency_threshold: 25.0,
        }
    }
}
impl PowerConfig {
    pub fn minimum_rated_total(&self) -> u8 {
        self.max_commanded_total
            .saturating_sub(self.rates.len().saturating_sub(1) as u8)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerSystem {
    hardware: std::sync::Arc<EnergyConfigSection>,
    ship: std::sync::Arc<ShipConfig>,
    network: EnergyNetworkState,
    coolant_credit: BTreeMap<String, f64>,
    weapons_restrained: bool,
    last_reactor_energy_delta_per_sec: f32,
    last_intake_per_sec: BTreeMap<SystemId, f32>,
    last_work_fraction: BTreeMap<SystemId, f32>,
    /// Derived receipts for this fixed interval; cleared before its Input work.
    /// Observers measure actual expenditure, never persistent control intent.
    work_emissions: BTreeMap<SystemId, f64>,
    strike_config: Option<StrikeReserveConfig>,
    strike_boost: Option<crate::modifiers::strike_reserve::StrikeBoost>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PowerTickResult {
    pub energy: EnergyNetworkResult,
    /// Already withdrawn fractional coolant remains in coolant_credit. Apply
    /// this integer plan against the actual provider Stocks after staging.
    pub coolant_withdrawals: BTreeMap<String, i64>,
}

impl PowerSystem {
    pub fn from_energy(hardware: EnergyConfigSection, ship: ShipConfig) -> Result<Self, String> {
        let network = EnergyNetworkState::new(&hardware, &ship)?;
        let last_intake_per_sec = network
            .consumers
            .keys()
            .map(|id| (id.clone(), 0.0))
            .collect();
        let last_work_fraction = network
            .consumers
            .keys()
            .map(|id| (id.clone(), 0.0))
            .collect();
        Ok(Self {
            hardware: std::sync::Arc::new(hardware),
            ship: std::sync::Arc::new(ship),
            network,
            coolant_credit: BTreeMap::new(),
            weapons_restrained: false,
            last_reactor_energy_delta_per_sec: 0.0,
            last_work_fraction,
            work_emissions: BTreeMap::new(),
            last_intake_per_sec,
            strike_config: None,
            strike_boost: None,
        })
    }
    pub fn config(&self) -> &EnergyConfigSection {
        &self.hardware
    }
    /// Replace supported hardware tuning without re-seeding any physical state.
    /// Slot topology, buffers and service links belong to the hull. Fitting can
    /// change operating costs/output, but cannot create charge or lose coolant.
    pub fn reconfigure_energy(&mut self, hardware: EnergyConfigSection) -> Result<(), String> {
        if [
            hardware.reactor.sustainable_output_per_sec,
            hardware.reactor.max_output_per_sec,
            hardware.reactor.ramp_per_sec,
            hardware.reactor.reserve.charge_heat_per_unit,
            hardware.reactor.reserve.discharge_heat_per_unit,
        ]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
            || hardware.reactor.ramp_per_sec <= 0.0
            || hardware.reactor.sustainable_output_per_sec > hardware.reactor.max_output_per_sec
            || hardware.consumers.iter().any(|consumer| {
                !consumer.continuous_cost_per_sec.is_finite()
                    || consumer.continuous_cost_per_sec < 0.0
            })
        {
            return Err("refit operating coefficients must be finite and physically valid".into());
        }
        let mut fixed = hardware.clone();
        fixed.reactor.sustainable_output_per_sec = self.hardware.reactor.sustainable_output_per_sec;
        fixed.reactor.max_output_per_sec = self.hardware.reactor.max_output_per_sec;
        fixed.reactor.ramp_per_sec = self.hardware.reactor.ramp_per_sec;
        fixed.reactor.reserve.charge_heat_per_unit =
            self.hardware.reactor.reserve.charge_heat_per_unit;
        fixed.reactor.reserve.discharge_heat_per_unit =
            self.hardware.reactor.reserve.discharge_heat_per_unit;
        for consumer in &mut fixed.consumers {
            let old = self
                .hardware
                .consumers
                .iter()
                .find(|old| old.system == consumer.system)
                .ok_or("refit cannot add an energy consumer")?;
            consumer.continuous_cost_per_sec = old.continuous_cost_per_sec;
        }
        if &fixed != self.hardware.as_ref() {
            return Err(
                "refit cannot change fixed buffers, links, initial state or allocation topology"
                    .into(),
            );
        }
        let saved = self.capture_continuation();
        let mut candidate = self.clone();
        candidate.hardware = std::sync::Arc::new(hardware);
        candidate.validate_saved(&saved)?;
        *self = candidate;
        Ok(())
    }
    pub fn ship_config(&self) -> &ShipConfig {
        &self.ship
    }
    pub fn network(&self) -> &EnergyNetworkState {
        &self.network
    }
    pub fn network_mut(&mut self) -> &mut EnergyNetworkState {
        &mut self.network
    }
    pub fn set_generation_enabled(&mut self, enabled: bool) {
        self.network.generation_enabled = enabled;
    }
    pub fn battery_charge(&self) -> f32 {
        self.network.reactor.energy
    }
    pub fn capacity(&self) -> f32 {
        self.hardware.reactor.reserve.capacity
    }
    pub fn coolant_credit(&self) -> &BTreeMap<String, f64> {
        &self.coolant_credit
    }
    pub fn last_intake_per_sec(&self, system: &SystemId) -> Option<f32> {
        self.last_intake_per_sec.get(system).copied()
    }
    pub fn reactor_energy_rate(&self) -> f32 {
        self.last_reactor_energy_delta_per_sec
    }
    pub fn set_flow(&mut self, group: &PowerGroupId, flow: f32) -> Result<(), String> {
        let cooling = self
            .network
            .groups
            .get(group)
            .ok_or("unknown allocation group")?
            .desired_cooling_per_sec;
        self.network.set_group(&self.hardware, group, flow, cooling)
    }
    pub fn flow_for(&self, group: &PowerGroupId) -> Option<f32> {
        self.network
            .groups
            .get(group)
            .map(|group| group.desired_flow_per_sec)
    }
    pub fn cooling_for(&self, group: &PowerGroupId) -> Option<f32> {
        self.network
            .groups
            .get(group)
            .map(|group| group.desired_cooling_per_sec)
    }
    pub fn set_cooling(&mut self, group: &PowerGroupId, cooling: f32) -> Result<(), String> {
        let flow = self.flow_for(group).ok_or("unknown allocation group")?;
        self.network.set_group(&self.hardware, group, flow, cooling)
    }
    pub fn set_output(&mut self, output: f32) -> Result<(), String> {
        self.network.set_output(&self.hardware, output)
    }
    pub fn set_reactor_cooling(&mut self, cooling: f32) -> Result<(), String> {
        self.network.set_reactor_cooling(&self.hardware, cooling)
    }
    pub fn set_priority(&mut self, group: &PowerGroupId, priority: u16) -> Result<(), String> {
        self.network.set_priority(group, priority)
    }
    pub fn set_mode(&mut self, mode: EnergyAllocationMode) {
        self.network.mode = mode;
    }
    pub fn set_cooling_usage(&mut self, usage: CoolingUsage) {
        self.network.cooling_usage = usage;
    }
    pub fn set_fine_flow(&mut self, system: &SystemId, flow: Option<f32>) -> Result<(), String> {
        self.network
            .set_fine_flow(&self.hardware, &self.ship, system, flow)
    }
    pub fn spend_continuous(
        &mut self,
        system: &SystemId,
        energy: f32,
    ) -> Result<crate::resources::energy::EnergyGrant, String> {
        let grant = self
            .network
            .spend_continuous(&self.hardware, system, energy)?;
        *self.work_emissions.entry(system.clone()).or_default() += f64::from(grant.granted);
        Ok(grant)
    }
    pub fn try_spend_discrete(&mut self, system: &SystemId) -> Result<bool, String> {
        self.try_work_discrete_count(system, 1)
    }

    /// Grant actual work before advancing an actuator or producing its effect.
    pub fn work_continuous(
        &mut self,
        system: &SystemId,
        dt: f32,
        activity: f32,
    ) -> Result<crate::resources::energy::EnergyGrant, String> {
        if !dt.is_finite() || dt < 0.0 || !activity.is_finite() || activity < 0.0 {
            return Err("energy work requires finite nonnegative time and activity".into());
        }
        let rate = self
            .hardware
            .consumers
            .iter()
            .find(|consumer| &consumer.system == system)
            .ok_or("unknown energy consumer")?
            .continuous_cost_per_sec;
        let grant = self.spend_continuous(system, rate * dt * activity)?;
        self.last_work_fraction.insert(
            system.clone(),
            if activity > 0.0 { grant.fraction } else { 0.0 },
        );
        Ok(grant)
    }

    /// A simultaneous firing step commits every projectile's energy or none.
    pub fn try_work_discrete_count(
        &mut self,
        system: &SystemId,
        count: usize,
    ) -> Result<bool, String> {
        let consumer = self
            .hardware
            .consumers
            .iter()
            .find(|consumer| &consumer.system == system)
            .ok_or("unknown energy consumer")?;
        let amount = consumer.discrete_cost * count as f32;
        self.try_work_discrete_amount(system, amount)
    }

    /// Debit an immutable accepted work requirement through the actual current
    /// consumer's reserve and thermal policy. This creates no separate credit.
    pub fn try_work_discrete_amount(
        &mut self,
        system: &SystemId,
        amount: f32,
    ) -> Result<bool, String> {
        if !amount.is_finite() || amount < 0.0 {
            return Err("discrete work requires a finite nonnegative amount".into());
        }
        let consumer = self
            .hardware
            .consumers
            .iter()
            .find(|consumer| &consumer.system == system)
            .ok_or("unknown energy consumer")?;
        let paid = self
            .network
            .consumers
            .get_mut(system)
            .ok_or("unknown consumer state")?
            .try_spend_discrete(&consumer.reserve, amount)?;
        if paid {
            *self.work_emissions.entry(system.clone()).or_default() += f64::from(amount);
        }
        Ok(paid)
    }

    pub fn try_work_discrete(&mut self, system: &SystemId) -> Result<bool, String> {
        self.try_work_discrete_count(system, 1)
    }
    pub fn last_work_fraction(&self, system: &SystemId) -> Option<f32> {
        self.last_work_fraction.get(system).copied()
    }
    pub fn emitted_work(&self, system: &SystemId) -> f64 {
        self.work_emissions.get(system).copied().unwrap_or(0.0)
    }

    /// Begin one complete work interval before its first machinery payment.
    /// Energy integration may occur between Input and later work, so it must
    /// not erase these derived receipts. Direct Gameplay drivers call this
    /// explicitly; Simulation calls it once in FixedFirst for every ship.
    pub fn begin_work_interval(&mut self) {
        self.work_emissions.clear();
    }

    /// A recovered physical delivery shares this interval's intake limit with
    /// reactor delivery and earlier recoveries. Canonical reserve/heat are the
    /// only credited state; failed or saturated deliveries consume no source.
    pub fn accept_recovered_energy(
        &mut self,
        system: &SystemId,
        dt: f32,
        offered: f32,
    ) -> Result<f32, String> {
        self.accept_recovered_energy_with_intake_factor(system, dt, offered, 1.0)
    }

    pub fn accept_recovered_energy_with_intake_factor(
        &mut self,
        system: &SystemId,
        dt: f32,
        offered: f32,
        intake_factor: f32,
    ) -> Result<f32, String> {
        if !intake_factor.is_finite() || !(0.0..=1.0).contains(&intake_factor) {
            return Err("recovery intake effectiveness must be finite in [0,1]".into());
        }
        if !dt.is_finite() || dt <= 0.0 || !offered.is_finite() || offered < 0.0 {
            return Err(
                "recovered energy requires positive time and finite nonnegative charge".into(),
            );
        }
        let consumer = self
            .hardware
            .consumers
            .iter()
            .find(|consumer| &consumer.system == system)
            .ok_or("unknown recovery destination")?;
        let state = self
            .network
            .consumers
            .get_mut(system)
            .ok_or("missing recovery destination state")?;
        if state.thermal_stopped {
            return Ok(0.0);
        }
        let previous = self.last_intake_per_sec.get(system).copied().unwrap_or(0.0);
        let budget = (((consumer.reserve.max_intake_per_sec as f64 * intake_factor as f64
            - previous as f64)
            .max(0.0)
            * dt as f64)
            .min(f32::MAX as f64)) as f32;
        let accepted = state.accept_charge(&consumer.reserve, offered, budget)?;
        self.last_intake_per_sec.insert(
            system.clone(),
            (previous as f64 + accepted as f64 / dt as f64)
                .min(consumer.reserve.max_intake_per_sec as f64) as f32,
        );
        Ok(accepted)
    }
    pub fn set_weapons_restrained(&mut self, restrained: bool) {
        self.weapons_restrained = restrained;
    }
    pub fn weapons_restrained(&self) -> bool {
        self.weapons_restrained
    }

    pub fn capture_continuation(&self) -> PowerState {
        PowerState {
            network: self.network.clone(),
            coolant_credit: self.coolant_credit.clone(),
            weapons_restrained: self.weapons_restrained,
            last_reactor_energy_delta_per_sec: self.last_reactor_energy_delta_per_sec,
            last_intake_per_sec: self.last_intake_per_sec.clone(),
            last_work_fraction: self.last_work_fraction.clone(),
            strike_boost: self.strike_boost.clone(),
        }
    }
    fn validate_saved(&self, saved: &PowerState) -> Result<(), String> {
        saved.network.validate_against(&self.hardware, &self.ship)?;
        self.validate_observations(
            saved.last_reactor_energy_delta_per_sec,
            &saved.last_intake_per_sec,
            &saved.last_work_fraction,
            &saved.coolant_credit,
            saved.strike_boost.is_some(),
        )
    }

    // Runtime validation borrows observations; capture is reserved for actual
    // persistence. The network transaction validates its own pre/post state.
    fn validate_current_observations(&self) -> Result<(), String> {
        self.validate_observations(
            self.last_reactor_energy_delta_per_sec,
            &self.last_intake_per_sec,
            &self.last_work_fraction,
            &self.coolant_credit,
            self.strike_boost.is_some(),
        )
    }

    fn validate_observations(
        &self,
        last_reactor_energy_delta_per_sec: f32,
        last_intake_per_sec: &BTreeMap<SystemId, f32>,
        last_work_fraction: &BTreeMap<SystemId, f32>,
        coolant_credit: &BTreeMap<String, f64>,
        has_strike_boost: bool,
    ) -> Result<(), String> {
        if !last_reactor_energy_delta_per_sec.is_finite()
            || last_intake_per_sec.keys().ne(self.network.consumers.keys())
            || last_intake_per_sec
                .values()
                .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err("invalid saved energy observations".into());
        }
        if last_work_fraction.keys().ne(self.network.consumers.keys())
            || last_work_fraction
                .values()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("invalid saved work observations".into());
        }
        if coolant_credit.iter().any(|(id, value)| {
            id.trim().is_empty() || !value.is_finite() || !(0.0..1.0).contains(value)
        }) {
            return Err("invalid saved coolant credit".into());
        }
        if has_strike_boost != self.strike_config.is_some() {
            return Err("saved strike control does not match authored equipment".into());
        }
        Ok(())
    }
    pub fn validate_continuation(
        &self,
        saved: &PowerState,
        capacity_ids: &BTreeSet<String>,
    ) -> Result<(), String> {
        self.validate_saved(saved)?;
        if saved
            .coolant_credit
            .keys()
            .any(|id| !capacity_ids.contains(id))
        {
            return Err("saved coolant credit references unknown actual capacity".into());
        }
        Ok(())
    }
    pub fn restore_continuation(&mut self, saved: &PowerState) -> Result<(), String> {
        self.validate_saved(saved)?;
        self.work_emissions.clear();
        self.network = saved.network.clone();
        self.coolant_credit = saved.coolant_credit.clone();
        self.weapons_restrained = saved.weapons_restrained;
        self.last_reactor_energy_delta_per_sec = saved.last_reactor_energy_delta_per_sec;
        self.last_intake_per_sec = saved.last_intake_per_sec.clone();
        self.last_work_fraction = saved.last_work_fraction.clone();
        self.strike_boost = saved.strike_boost.clone();
        Ok(())
    }
    pub fn restore_continuation_with_capacities(
        &mut self,
        saved: &PowerState,
        capacity_ids: &BTreeSet<String>,
    ) -> Result<(), String> {
        self.validate_continuation(saved, capacity_ids)?;
        self.restore_continuation(saved)
    }

    /// Passive aging of the same physical reserves while a probe is carried.
    /// No generator, routing, coolant draw, work or operator-intent rewrite.
    /// The existing Simulation energy tick selects this mode once per interval.
    pub fn tick_stored_energy(&mut self, dt: f32) -> Result<(), String> {
        if !dt.is_finite() || dt < 0.0 {
            return Err("stored energy interval must be finite and nonnegative".into());
        }
        let mut next = self.clone();
        let before = next.battery_charge();
        let passive = || crate::resources::energy::EnergyStep {
            dt,
            offered_energy: 0.0,
            demands: &[],
            cooling_rate: 0.0,
            coolant_available: 0.0,
        };
        next.network
            .reactor
            .step(&next.hardware.reactor.reserve, passive())?;
        for consumer in &next.hardware.consumers {
            next.network
                .consumers
                .get_mut(&consumer.system)
                .ok_or("stored energy lost an actual consumer reserve")?
                .step(&consumer.reserve, passive())?;
        }
        next.network.output_actual_per_sec = 0.0;
        for group in next.network.groups.values_mut() {
            group.effective_flow_per_sec = 0.0;
            group.effective_cooling_per_sec = 0.0;
        }
        next.last_intake_per_sec
            .values_mut()
            .for_each(|value| *value = 0.0);
        next.last_work_fraction
            .values_mut()
            .for_each(|value| *value = 0.0);
        next.work_emissions.clear();
        if dt > 0.0 {
            next.last_reactor_energy_delta_per_sec = (next.battery_charge() - before) / dt;
        }
        next.network.validate_against(&next.hardware, &next.ship)?;
        next.validate_current_observations()?;
        *self = next;
        Ok(())
    }

    /// Integrate incoming energy within the current work interval. This is
    /// not a receipt boundary; call `begin_work_interval` before any work.
    pub fn tick_energy(
        &mut self,
        dt: f32,
        bindings: &EnergyCoolantBindings,
        available_stock: &BTreeMap<String, i64>,
        choose: &mut impl FnMut(usize) -> usize,
    ) -> Result<PowerTickResult, String> {
        self.tick_energy_with_effects(
            dt,
            bindings,
            available_stock,
            &crate::resources::energy_network::EnergyHardwareEffects::default(),
            choose,
        )
    }

    pub fn tick_energy_with_effects(
        &mut self,
        dt: f32,
        bindings: &EnergyCoolantBindings,
        available_stock: &BTreeMap<String, i64>,
        effects: &crate::resources::energy_network::EnergyHardwareEffects,
        choose: &mut impl FnMut(usize) -> usize,
    ) -> Result<PowerTickResult, String> {
        if available_stock.values().any(|value| *value < 0) {
            return Err("coolant stocks cannot be negative".into());
        }
        self.validate_current_observations()?;
        if self
            .coolant_credit
            .keys()
            .any(|id| !available_stock.contains_key(id))
        {
            return Err("saved coolant credit references unknown actual capacity".into());
        }
        let mut budgets = BTreeMap::new();
        for (id, available) in available_stock {
            let amount = *available as f64 + self.coolant_credit.get(id).copied().unwrap_or(0.0);
            let mut continuous = amount as f32;
            if continuous as f64 > amount && continuous > 0.0 {
                continuous = f32::from_bits(continuous.to_bits() - 1);
            }
            budgets.insert(id.clone(), continuous);
        }
        let mut next = self.clone();
        let before = next.battery_charge();
        let energy = next.network.tick_with_effects_in_place(
            &next.hardware,
            &next.ship,
            dt,
            bindings,
            &budgets,
            effects,
            choose,
        )?;
        let mut withdrawals = BTreeMap::new();
        for (id, used) in &energy.coolant_used {
            let old = next.coolant_credit.get(id).copied().unwrap_or(0.0);
            let needed = (*used as f64 - old).max(0.0);
            let units = needed.ceil();
            let available = available_stock
                .get(id)
                .copied()
                .ok_or("cooling used an unknown provider")?;
            if !units.is_finite() || units >= i64::MAX as f64 || units > available as f64 {
                return Err("actual coolant stock cannot fund resolved cooling".into());
            }
            let credit = old + units - *used as f64;
            if !credit.is_finite() || credit < 0.0 || credit > 1.0 {
                return Err("coolant accounting escaped its unit bounds".into());
            }
            // Conservative IEEE rounding of sub-precision expenditure never
            // credits an entire extra stock unit or creates coolant.
            let credit = credit.min(f64::from_bits(1.0f64.to_bits() - 1));
            next.coolant_credit.insert(id.clone(), credit);
            if units > 0.0 {
                withdrawals.insert(id.clone(), units as i64);
            }
        }
        if dt > 0.0 {
            next.last_reactor_energy_delta_per_sec = (next.battery_charge() - before) / dt;
            next.last_intake_per_sec = energy
                .consumer_reports
                .iter()
                .map(|(id, report)| (id.clone(), report.accepted_energy / dt))
                .collect();
        }
        next.validate_current_observations()?;
        *self = next;
        Ok(PowerTickResult {
            energy,
            coolant_withdrawals: withdrawals,
        })
    }

    pub fn with_strike_reserve(
        mut self,
        config: Option<StrikeReserveConfig>,
    ) -> Result<Self, String> {
        if let Some(value) = &config {
            if !value.units_per_level.is_finite()
                || value.units_per_level <= 0.0
                || value.ai_enable_at.is_some_and(|threshold| {
                    !threshold.is_finite() || threshold < 0.0 || threshold > self.capacity()
                })
                || value
                    .weapons
                    .values()
                    .any(|weapon| !weapon.valid() || weapon.cost > self.capacity())
            {
                return Err("invalid authored strike overdrive".into());
            }
        }
        self.strike_boost = config.as_ref().map(|_| Default::default());
        self.strike_config = config;
        Ok(self)
    }
    pub fn strike_boost(&self) -> Option<&crate::modifiers::strike_reserve::StrikeBoost> {
        self.strike_boost.as_ref()
    }
    pub fn set_strike_boost(&mut self, enabled: bool) -> bool {
        if let Some(boost) = self.strike_boost.as_mut() {
            boost.set(enabled);
            true
        } else {
            false
        }
    }
    pub fn fire_strike_weapon(&mut self, system_id: &str) -> f32 {
        if !self
            .strike_boost
            .as_ref()
            .is_some_and(|boost| boost.enabled)
        {
            return 1.0;
        }
        let Some(weapon) = self
            .strike_config
            .as_ref()
            .and_then(|config| config.weapons.get(system_id))
            .cloned()
        else {
            return 1.0;
        };
        if !matches!(
            self.network
                .reactor
                .try_spend_discrete(&self.hardware.reactor.reserve, weapon.cost),
            Ok(true)
        ) {
            if let Some(boost) = self.strike_boost.as_mut() {
                boost.enabled = false;
                boost.depleted = true;
            }
            return 1.0;
        }
        if self.network.reactor.thermal_stopped {
            self.network.output_actual_per_sec = 0.0;
        }
        weapon.damage_multiplier
    }
    pub fn strike_read(
        &self,
        _legacy: &PowerConfig,
    ) -> Option<crate::core::messages::StrikeReserveBlackboard> {
        self.strike_boost
            .as_ref()
            .map(|boost| crate::core::messages::StrikeReserveBlackboard {
                charge: self.battery_charge(),
                capacity: self.capacity(),
                charging: self.reactor_energy_rate() > 0.0,
                enabled: boost.enabled,
                depleted: boost.depleted,
            })
    }

    // Temporary payload/screen translations, removed with the old rung UI at
    // this increment's exit. They never limit supply or amplify System work.
    pub fn level_for(&self, id: &PowerGroupId) -> u8 {
        let Some(group) = self.hardware.groups.iter().find(|group| &group.id == id) else {
            return 0;
        };
        let flow = self.flow_for(id).unwrap_or(0.0);
        if group.max_flow_per_sec == 0.0 {
            0
        } else {
            ((flow / group.max_flow_per_sec) * GROUP_LEVEL_MAX as f32).round() as u8
        }
    }
    pub fn commanded_level_for(&self, id: &PowerGroupId) -> u8 {
        self.level_for(id)
    }
    pub fn has_group(&self, id: &PowerGroupId) -> bool {
        self.network.groups.contains_key(id)
    }
    pub fn floor_for(&self, id: &PowerGroupId) -> u8 {
        self.ship
            .power_groups
            .get(id)
            .map_or(GROUP_LEVEL_MIN, |group| {
                group.min_level.min(GROUP_LEVEL_MAX)
            })
    }
    pub fn is_group_cold(&self, id: &PowerGroupId) -> bool {
        self.flow_for(id) == Some(0.0)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&PowerGroupId, u8)> {
        self.hardware
            .groups
            .iter()
            .map(|group| (&group.id, self.level_for(&group.id)))
    }
    pub fn total(&self) -> u8 {
        self.iter()
            .fold(0u8, |total, (_, level)| total.saturating_add(level))
    }
    pub fn commanded_total(&self) -> u8 {
        self.total()
    }
    pub fn max_commanded_total(&self) -> u8 {
        self.iter()
            .fold(0u8, |total, _| total.saturating_add(GROUP_LEVEL_MAX))
    }
    pub fn locked(&self) -> bool {
        false
    }
    pub fn set_group_allocation(
        &mut self,
        id: &PowerGroupId,
        level: u8,
    ) -> Result<(), PowerAllocationError> {
        let maximum = self
            .hardware
            .groups
            .iter()
            .find(|group| &group.id == id)
            .ok_or_else(|| PowerAllocationError::UnknownGroup(id.clone()))?
            .max_flow_per_sec;
        let level = level.clamp(self.floor_for(id), GROUP_LEVEL_MAX);
        self.set_flow(id, maximum * level as f32 / GROUP_LEVEL_MAX as f32)
            .map_err(PowerAllocationError::InvalidRequest)
    }
    pub fn increase(&mut self, id: &PowerGroupId) {
        let _ = self.set_group_allocation(id, self.level_for(id).saturating_add(1));
    }
    pub fn decrease(&mut self, id: &PowerGroupId) {
        let _ = self.set_group_allocation(id, self.level_for(id).saturating_sub(1));
    }
    pub fn battery_rate(&self, _legacy: &PowerConfig) -> f32 {
        self.reactor_energy_rate()
    }
    pub fn is_draining(&self, _legacy: &PowerConfig) -> bool {
        self.reactor_energy_rate() < 0.0
    }
    pub fn is_charging(&self, _legacy: &PowerConfig) -> bool {
        self.reactor_energy_rate() > 0.0
    }
    pub fn read_state(&self) -> PowerReadState {
        PowerReadState {
            allocations: self.iter().map(|(id, level)| (id.clone(), level)).collect(),
            battery_charge: self.battery_charge(),
            locked: false,
        }
    }

    /// Rust fixture convenience. Production entity spawning uses from_energy.
    pub fn new(config: &PowerConfig) -> Self {
        Self::from_authored_groups(config, &[])
    }
    pub fn from_authored_groups(config: &PowerConfig, authored: &[AuthoredPowerGroup]) -> Self {
        assert!(
            config.capacity.is_finite() && config.capacity > 0.0,
            "fixture capacity must be positive and finite"
        );
        let fallback: Vec<_> = POWER_GROUP_ORDER
            .iter()
            .map(|id| AuthoredPowerGroup::at_default_floor(PowerGroupId((*id).into()), 2))
            .collect();
        let authored = if authored.is_empty() {
            &fallback
        } else {
            authored
        };
        let mut seen = BTreeSet::new();
        let authored: Vec<_> = authored
            .iter()
            .filter(|group| seen.insert(group.id.clone()))
            .collect();
        let link = ResourceLink {
            service: None,
            provider: Some(ResourceEndpoint::Capacity {
                id: "fixture-coolant".into(),
            }),
        };
        let reserve = EnergyConfig {
            capacity: config.capacity,
            max_intake_per_sec: 4.0,
            passive_loss_at_full_per_sec: 0.1,
            charge_heat_per_unit: 0.1,
            discharge_heat_per_unit: 0.1,
            passive_cooling_per_sec: 0.1,
            coolant_heat_per_unit: 10.0,
            trip_heat: 100.0,
            recover_heat: 50.0,
        };
        let groups: Vec<_> = authored
            .iter()
            .map(|group| EnergyGroupConfig {
                id: group.id.clone(),
                parent: None,
                max_flow_per_sec: 4.0,
                default_flow_per_sec: group
                    .level
                    .clamp(group.floor.min(GROUP_LEVEL_MAX), GROUP_LEVEL_MAX)
                    as f32,
                max_cooling_per_sec: 1.0,
                default_cooling_per_sec: 0.1,
                priority: 0,
                restore_delay_secs: 0.5,
            })
            .collect();
        let consumers: Vec<_> = authored
            .iter()
            .map(|group| EnergyConsumerConfig {
                system: SystemId(format!("fixture-{}", group.id.0)),
                initial_energy: config.capacity * 0.5,
                initial_heat: 0.0,
                continuous_cost_per_sec: 1.0,
                discrete_cost: 1.0,
                reserve,
                coolant: link.clone(),
            })
            .collect();
        let ship = ShipConfig {
            consoles: vec![],
            stations: vec![],
            coordination_lag_secs: 0.0,
            systems: authored
                .iter()
                .zip(&consumers)
                .map(|(group, consumer)| SystemInstanceConfig {
                    damage_components: Vec::new(),
                    id: consumer.system.clone(),
                    kind: "fixture-energy-consumer".into(),
                    console: None,
                    ai_only: true,
                    power_group: Some(group.id.clone()),
                    marker: None,
                    config: None,
                    components: vec![],
                    channels: vec![],
                })
                .collect(),
            power_groups: authored
                .iter()
                .map(|group| {
                    (
                        group.id.clone(),
                        PowerGroupConfig {
                            label: group.id.0.clone(),
                            default_level: group
                                .level
                                .clamp(group.floor.min(GROUP_LEVEL_MAX), GROUP_LEVEL_MAX),
                            min_level: group.floor.min(GROUP_LEVEL_MAX),
                            max_level: GROUP_LEVEL_MAX,
                        },
                    )
                })
                .collect::<HashMap<_, _>>(),
        };
        let sustainable = config.sustainable_total.max(1) as f32;
        let hardware = EnergyConfigSection {
            reactor: EnergyReactorConfig {
                sustainable_output_per_sec: sustainable,
                max_output_per_sec: (config.max_commanded_total as f32).max(sustainable),
                ramp_per_sec: sustainable,
                initial_output_per_sec: sustainable,
                initial_energy: config.capacity,
                initial_heat: 0.0,
                max_cooling_per_sec: 1.0,
                default_cooling_per_sec: 0.1,
                reserve: EnergyConfig {
                    max_intake_per_sec: (config.max_commanded_total as f32).max(sustainable),
                    ..reserve
                },
                coolant: link,
            },
            consumers,
            groups,
        };
        Self::from_energy(hardware, ship)
            .and_then(|power| power.with_strike_reserve(config.strike_reserve.clone()))
            .expect("explicit fixture hardware must validate")
    }
}
impl Default for PowerSystem {
    fn default() -> Self {
        Self::new(&PowerConfig::default())
    }
}

pub fn power_level_for_group(power: &PowerSystem, group: &PowerGroupId) -> u8 {
    power.level_for(group)
}
#[derive(Clone, Debug, PartialEq)]
pub struct AllocationBid {
    pub group: PowerGroupId,
    pub want: u8,
    pub max_level: u8,
    pub floor: u8,
    pub rule_priority: i32,
}
/// Legacy policy output translation only. Scarcity is resolved by the new
/// physical allocation policy; no old shared rung budget or cold-weapon gate.
pub fn plan_allocation(power: &PowerSystem, bids: &[AllocationBid]) -> Vec<(PowerGroupId, u8)> {
    let mut ordered: Vec<_> = bids
        .iter()
        .filter(|bid| power.has_group(&bid.group))
        .collect();
    ordered.sort_by_key(|bid| std::cmp::Reverse(bid.rule_priority));
    let mut seen = BTreeSet::new();
    ordered
        .into_iter()
        .filter(|bid| seen.insert(bid.group.clone()))
        .filter_map(|bid| {
            let maximum = bid.max_level.min(GROUP_LEVEL_MAX);
            let desired = bid.want.clamp(bid.floor.min(maximum), maximum);
            (desired != power.commanded_level_for(&bid.group)).then(|| (bid.group.clone(), desired))
        })
        .collect()
}

pub use phoenix_sim_contracts::power::*;
#[cfg(test)]
#[path = "power_system_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "power_stored_tests.rs"]
mod stored_tests;
