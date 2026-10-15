//! Paid recovery progress only. Finite quantities stay with existing endpoints.
use phoenix_model::messages::SystemId;
pub use phoenix_model::recovery::*;
use serde::{Deserialize, Serialize};

pub const RECOVERY_KIND: &str = "resource-recovery";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryOutcome {
    pub tick: u64,
    pub offered: i64,
    pub moved: i64,
    pub settled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryState {
    pub owner: SystemId,
    pub enabled: bool,
    pub target: Option<String>,
    pub resource: Option<String>,
    pub destination_store: Option<String>,
    pub fraction: f64,
    pub last_tick: u64,
    pub last_outcome: Option<RecoveryOutcome>,
}

impl RecoveryState {
    pub fn new(profile: &RecoveryToolConfig) -> Result<Self, String> {
        profile.validate()?;
        Ok(Self {
            owner: profile.system.clone(),
            enabled: false,
            target: None,
            resource: None,
            destination_store: None,
            fraction: 0.0,
            last_tick: 0,
            last_outcome: None,
        })
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn select(
        &mut self,
        profile: &RecoveryToolConfig,
        target: Option<&str>,
        resource: Option<&str>,
        destination_store: Option<&str>,
    ) -> Result<(), String> {
        profile.validate()?;
        validate_selection(profile, target, resource, destination_store)?;
        if self.owner != profile.system {
            return Err("recovery owner differs from fitted System".into());
        }
        let changed = self.target.as_deref() != target
            || self.resource.as_deref() != resource
            || self.destination_store.as_deref() != destination_store;
        if changed {
            self.fraction = 0.0;
        }
        self.target = target.map(str::to_owned);
        self.resource = resource.map(str::to_owned);
        self.destination_store = destination_store.map(str::to_owned);
        Ok(())
    }

    /// The caller first establishes actual range, permission, source supply,
    /// protected floors and destination room, then stages energy with this state.
    /// Sub-unit carry represents paid work, never a spendable material stock.
    pub fn advance<F>(
        &mut self,
        profile: &RecoveryToolConfig,
        tick: u64,
        dt: f32,
        quantity_limit: i64,
        mut work: F,
    ) -> Result<i64, String>
    where
        F: FnMut(&SystemId, f32) -> Result<f32, String>,
    {
        self.validate(profile, tick.max(self.last_tick))?;
        if !dt.is_finite() || dt < 0.0 || quantity_limit < 0 {
            return Err("recovery requires finite nonnegative time and quantity limit".into());
        }
        if tick <= self.last_tick {
            return Ok(0);
        }
        let mut next = self.clone();
        next.last_tick = tick;
        let mut offered = 0;
        if self.enabled && self.target.is_some() && quantity_limit > 0 && dt > 0.0 {
            let rate = f64::from(profile.units_per_sec);
            let seconds =
                f64::from(dt).min(((quantity_limit as f64 - self.fraction) / rate).max(0.0));
            let mut work_seconds = seconds as f32;
            // Round down when the machinery's f32 time would otherwise buy
            // more than the preflighted finite quantity can accept.
            if f64::from(work_seconds) > seconds {
                work_seconds = f32::from_bits(work_seconds.to_bits() - 1);
            }
            let seconds = work_seconds;
            let possible = self.fraction + rate * f64::from(seconds);
            if !possible.is_finite() || possible >= i64::MAX as f64 {
                return Err("recovery work exceeds bounded integer quantity".into());
            }
            if seconds > 0.0 {
                let funded = work(&self.owner, seconds)?;
                if !funded.is_finite() || !(0.0..=1.0).contains(&funded) {
                    return Err(
                        "recovery funded work fraction must be finite and within 0..1".into(),
                    );
                }
                let produced = self.fraction + rate * f64::from(seconds) * f64::from(funded);
                offered = (produced.floor() as i64).min(quantity_limit);
                next.fraction = produced.fract();
            }
        }
        next.last_outcome = Some(RecoveryOutcome {
            tick,
            offered,
            moved: 0,
            settled: offered == 0,
        });
        *self = next;
        Ok(offered)
    }

    /// Record actual conservative endpoint movement after the caller's staged
    /// transaction. Repeating the same receipt is harmless; rewrites refuse.
    pub fn record_receipt(&mut self, tick: u64, moved: i64) -> Result<(), String> {
        let receipt = self
            .last_outcome
            .as_mut()
            .ok_or("recovery receipt has no work offer")?;
        if tick != self.last_tick
            || tick != receipt.tick
            || moved < 0
            || moved > receipt.offered
            || (receipt.settled && moved != receipt.moved)
        {
            return Err("recovery receipt differs from its actual offered work".into());
        }
        receipt.moved = moved;
        receipt.settled = true;
        Ok(())
    }

    pub fn validate(&self, profile: &RecoveryToolConfig, snapshot_tick: u64) -> Result<(), String> {
        profile.validate()?;
        self.validate_settlement()?;
        validate_selection(
            profile,
            self.target.as_deref(),
            self.resource.as_deref(),
            self.destination_store.as_deref(),
        )?;
        if self.owner != profile.system
            || !self.fraction.is_finite()
            || !(0.0..1.0).contains(&self.fraction)
            || self.last_tick > snapshot_tick
            || (self.target.is_none() && self.fraction != 0.0)
        {
            return Err("recovery continuation owner, fraction or receipt is invalid".into());
        }
        Ok(())
    }

    /// The receipt queue is ephemeral and must settle before a tick boundary.
    pub fn validate_settlement(&self) -> Result<(), String> {
        if (self.last_tick == 0) != self.last_outcome.is_none()
            || self.last_outcome.as_ref().is_some_and(|r| {
                r.tick != self.last_tick
                    || r.offered < 0
                    || r.moved < 0
                    || r.moved > r.offered
                    || !r.settled
            })
        {
            Err("recovery work has an invalid or unsettled transfer receipt".into())
        } else {
            Ok(())
        }
    }
}

fn validate_selection(
    profile: &RecoveryToolConfig,
    target: Option<&str>,
    resource: Option<&str>,
    destination: Option<&str>,
) -> Result<(), String> {
    match (target, resource, destination) {
        (None, None, None) => Ok(()),
        (Some(t), Some(r), Some(d)) if !t.trim().is_empty() && !d.trim().is_empty() && profile.resources.iter().any(|supported| supported == r) => Ok(()),
        _ => Err("recovery selection requires a target, supported resource and destination, or a complete clear".into()),
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
