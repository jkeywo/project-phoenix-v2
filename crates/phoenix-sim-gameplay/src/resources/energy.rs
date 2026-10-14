//! Continuous local reserves and thermal accounting shared by consumers and
//! reactor capacitors. No controllers, ECS ownership or parallel stock ledger.
//!
//! A step applies passive loss, actual headroom-limited intake, requested work,
//! passive heat rejection, then coolant cooling. Work can free headroom for the
//! next intake; a speculative demand never draws energy from the provider.
//! Demand precedence is explicit priority followed by stable identity, never
//! caller iteration order. Immediate operations use the same accounting for
//! owners that already resolve their actual work in a separate engine phase.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyConfig {
    pub capacity: f32,
    pub max_intake_per_sec: f32,
    /// Linear fullness-dependent loss. An empty reserve loses no energy.
    pub passive_loss_at_full_per_sec: f32,
    pub charge_heat_per_unit: f32,
    pub discharge_heat_per_unit: f32,
    pub passive_cooling_per_sec: f32,
    /// Actual heat removed by one unit of expended coolant.
    pub coolant_heat_per_unit: f32,
    pub trip_heat: f32,
    pub recover_heat: f32,
}

fn finite_nonnegative(name: &str, value: f32) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and nonnegative"))
    }
}

fn finite_positive(name: &str, value: f32) -> Result<(), String> {
    finite_nonnegative(name, value)?;
    if value == 0.0 {
        return Err(format!("{name} must be positive"));
    }
    Ok(())
}

/// Widen intermediate products so finite inputs cannot silently overflow before
/// bounds are applied. Authoritative stored state and all reported values remain
/// finite f32 values. An unrepresentable outcome refuses the entire transaction.
fn finite_result(name: &str, value: f64) -> Result<f32, String> {
    if value.is_finite() && value >= 0.0 && value <= f32::MAX as f64 {
        Ok(value as f32)
    } else {
        Err(format!("{name} exceeds the supported finite range"))
    }
}

impl EnergyConfig {
    pub fn validate(&self) -> Result<(), String> {
        finite_positive("capacity", self.capacity)?;
        finite_positive("max_intake_per_sec", self.max_intake_per_sec)?;
        finite_positive("coolant_heat_per_unit", self.coolant_heat_per_unit)?;
        finite_positive("trip_heat", self.trip_heat)?;
        for (name, value) in [
            (
                "passive_loss_at_full_per_sec",
                self.passive_loss_at_full_per_sec,
            ),
            ("charge_heat_per_unit", self.charge_heat_per_unit),
            ("discharge_heat_per_unit", self.discharge_heat_per_unit),
            ("passive_cooling_per_sec", self.passive_cooling_per_sec),
            ("recover_heat", self.recover_heat),
        ] {
            finite_nonnegative(name, value)?;
        }
        if self.recover_heat >= self.trip_heat {
            return Err("recover_heat must be lower than trip_heat".into());
        }
        Ok(())
    }
}

/// No enabled/target/order fields: protective availability never overwrites
/// the owning System's persistent intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyState {
    pub energy: f32,
    pub heat: f32,
    pub thermal_stopped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnergyDemandKind {
    Continuous,
    Discrete,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnergyDemand {
    pub id: String,
    pub priority: i32,
    pub kind: EnergyDemandKind,
    /// Energy requested for this step, rather than a rate.
    pub amount: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnergyGrant {
    pub requested: f32,
    pub granted: f32,
    pub fraction: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnergyDemandGrant {
    pub id: String,
    pub grant: EnergyGrant,
}

pub struct EnergyStep<'a> {
    pub dt: f32,
    /// Already resolved source offer, in energy units. Acceptance also respects
    /// this System's own intake limit and actual current headroom.
    pub offered_energy: f32,
    pub demands: &'a [EnergyDemand],
    /// Allocated active cooling capacity in heat units per second.
    pub cooling_rate: f32,
    /// Caller-provided spendable coolant, including any fractional accounting.
    /// The processor reports expenditure; the caller owns the real stock.
    pub coolant_available: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnergyStepResult {
    pub accepted_energy: f32,
    pub rejected_energy: f32,
    pub passive_loss: f32,
    pub grants: Vec<EnergyDemandGrant>,
    pub work_energy: f32,
    pub heat_generated: f32,
    pub passive_heat_removed: f32,
    pub coolant_heat_removed: f32,
    pub coolant_used: f32,
    pub thermal_stopped: bool,
}

impl EnergyState {
    pub fn new(config: &EnergyConfig, energy: f32, heat: f32) -> Result<Self, String> {
        let mut state = Self {
            energy,
            heat,
            thermal_stopped: false,
        };
        state.trip_if_hot(config);
        state.validate_against(config)?;
        Ok(state)
    }

    /// Decode/preflight validates against the authored configuration before
    /// application. Serde alone cannot know the equipment's capacity.
    pub fn validate_against(&self, config: &EnergyConfig) -> Result<(), String> {
        config.validate()?;
        finite_nonnegative("energy", self.energy)?;
        finite_nonnegative("heat", self.heat)?;
        if self.energy > config.capacity {
            return Err("energy exceeds authored capacity".into());
        }
        if self.heat >= config.trip_heat && !self.thermal_stopped {
            return Err("thermal stop is missing above the trip threshold".into());
        }
        Ok(())
    }

    fn trip_if_hot(&mut self, config: &EnergyConfig) {
        if self.heat >= config.trip_heat {
            self.thermal_stopped = true;
        }
    }

    fn add_depletion_heat(&mut self, config: &EnergyConfig, amount: f32) -> Result<(), String> {
        self.heat = finite_result(
            "heat",
            self.heat as f64 + amount as f64 * config.discharge_heat_per_unit as f64,
        )?;
        self.trip_if_hot(config);
        Ok(())
    }

    fn spend(
        &mut self,
        config: &EnergyConfig,
        requested: f32,
        kind: EnergyDemandKind,
    ) -> Result<EnergyGrant, String> {
        let granted = if self.thermal_stopped {
            0.0
        } else {
            match kind {
                EnergyDemandKind::Continuous => requested.min(self.energy),
                EnergyDemandKind::Discrete if requested <= self.energy => requested,
                EnergyDemandKind::Discrete => 0.0,
            }
        };
        self.energy -= granted;
        self.add_depletion_heat(config, granted)?;
        Ok(EnergyGrant {
            requested,
            granted,
            // A zero-cost operation is affordable unless thermal protection
            // blocks operation. No division by zero enters a forecast or gate.
            fraction: if requested == 0.0 {
                if self.thermal_stopped {
                    0.0
                } else {
                    1.0
                }
            } else {
                granted / requested
            },
        })
    }

    pub fn spend_continuous(
        &mut self,
        config: &EnergyConfig,
        requested_energy: f32,
    ) -> Result<EnergyGrant, String> {
        self.validate_against(config)?;
        finite_nonnegative("requested_energy", requested_energy)?;
        let mut next = *self;
        let grant = next.spend(config, requested_energy, EnergyDemandKind::Continuous)?;
        *self = next;
        Ok(grant)
    }

    /// Does not consume a partial cost, add heat, or reset any intent while
    /// waiting. Callers consume ammunition/produce effects only after `true`.
    pub fn try_spend_discrete(&mut self, config: &EnergyConfig, cost: f32) -> Result<bool, String> {
        self.validate_against(config)?;
        finite_nonnegative("cost", cost)?;
        if self.thermal_stopped || cost > self.energy {
            return Ok(false);
        }
        let mut next = *self;
        next.spend(config, cost, EnergyDemandKind::Discrete)?;
        *self = next;
        Ok(true)
    }

    /// Credit an actual physical delivery using the caller's remaining intake
    /// budget. This does not advance time, repeat passive loss or provide cooling.
    /// The provider removes only the returned accepted quantity.
    pub fn accept_charge(
        &mut self,
        config: &EnergyConfig,
        offered: f32,
        available_intake: f32,
    ) -> Result<f32, String> {
        self.validate_against(config)?;
        finite_nonnegative("offered charge", offered)?;
        finite_nonnegative("available intake", available_intake)?;
        let offered_limit = offered
            .min(available_intake)
            .min((config.capacity - self.energy).max(0.0));
        let mut next = *self;
        let exact = (next.energy as f64 + offered_limit as f64).min(config.capacity as f64);
        next.energy = finite_result("energy", exact)?;
        if next.energy as f64 > exact {
            next.energy = f32::from_bits(next.energy.to_bits() - 1);
        }
        // Credit rounds down and provider debit rounds up. Sub-ulp deliveries
        // are refused instead of silently creating charge in a full reservoir.
        let credited = next.energy as f64 - self.energy as f64;
        let mut accepted = credited as f32;
        if (accepted as f64) < credited {
            accepted = f32::from_bits(accepted.to_bits() + 1);
        }
        next.heat = finite_result(
            "heat",
            next.heat as f64 + accepted as f64 * config.charge_heat_per_unit as f64,
        )?;
        next.trip_if_hot(config);
        next.validate_against(config)?;
        *self = next;
        Ok(accepted)
    }

    /// Entire step commits atomically; malformed offers, requests or impossible
    /// finite results cannot leave a partly changed reserve or thermal state.
    pub fn step(
        &mut self,
        config: &EnergyConfig,
        input: EnergyStep<'_>,
    ) -> Result<EnergyStepResult, String> {
        self.validate_against(config)?;
        for (name, value) in [
            ("dt", input.dt),
            ("offered_energy", input.offered_energy),
            ("cooling_rate", input.cooling_rate),
            ("coolant_available", input.coolant_available),
        ] {
            finite_nonnegative(name, value)?;
        }
        let mut ids = BTreeSet::new();
        for demand in input.demands {
            if demand.id.trim().is_empty() || !ids.insert(demand.id.as_str()) {
                return Err("energy demand identities must be nonempty and unique".into());
            }
            finite_nonnegative("demand.amount", demand.amount)?;
        }
        let mut ordered: Vec<_> = input.demands.iter().collect();
        ordered.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        let mut next = *self;
        let mut result = EnergyStepResult::default();
        result.passive_loss = finite_result(
            "passive loss",
            (next.energy as f64 / config.capacity as f64
                * config.passive_loss_at_full_per_sec as f64
                * input.dt as f64)
                .min(next.energy as f64),
        )?;
        next.energy -= result.passive_loss;
        next.add_depletion_heat(config, result.passive_loss)?;
        result.accepted_energy = finite_result(
            "accepted energy",
            (input.offered_energy as f64)
                .min(config.max_intake_per_sec as f64 * input.dt as f64)
                .min((config.capacity - next.energy) as f64),
        )?;
        result.rejected_energy = input.offered_energy - result.accepted_energy;
        next.energy = finite_result(
            "energy",
            (next.energy as f64 + result.accepted_energy as f64).min(config.capacity as f64),
        )?;
        next.heat = finite_result(
            "heat",
            next.heat as f64 + result.accepted_energy as f64 * config.charge_heat_per_unit as f64,
        )?;
        next.trip_if_hot(config);
        let mut work_energy = 0.0f64;
        for demand in ordered {
            let grant = next.spend(config, demand.amount, demand.kind)?;
            work_energy += grant.granted as f64;
            result.grants.push(EnergyDemandGrant {
                id: demand.id.clone(),
                grant,
            });
        }
        result.work_energy = finite_result("work energy", work_energy)?;
        result.heat_generated = finite_result(
            "heat generated",
            result.accepted_energy as f64 * config.charge_heat_per_unit as f64
                + (result.passive_loss as f64 + work_energy)
                    * config.discharge_heat_per_unit as f64,
        )?;
        result.passive_heat_removed = finite_result(
            "passive heat removed",
            (config.passive_cooling_per_sec as f64 * input.dt as f64).min(next.heat as f64),
        )?;
        next.heat -= result.passive_heat_removed;
        result.coolant_heat_removed = finite_result(
            "coolant heat removed",
            (input.cooling_rate as f64 * input.dt as f64)
                .min(input.coolant_available as f64 * config.coolant_heat_per_unit as f64)
                .min(next.heat as f64),
        )?;
        result.coolant_used = (result.coolant_heat_removed / config.coolant_heat_per_unit)
            .min(input.coolant_available);
        next.heat -= result.coolant_heat_removed;
        if next.thermal_stopped && next.heat <= config.recover_heat {
            next.thermal_stopped = false;
        }
        result.thermal_stopped = next.thermal_stopped;
        next.validate_against(config)?;
        *self = next;
        Ok(result)
    }
}

#[cfg(test)]
#[path = "energy_tests.rs"]
mod tests;
