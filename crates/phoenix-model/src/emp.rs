//! Temporary component interference. Absolute causal tick frontiers also age
//! while the actual component is in a locker or carried by an existing job.
use crate::damage::DamageBinding;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmpPayload {
    pub duration_secs: f32,
}
impl EmpPayload {
    pub fn validate(&self) -> Result<(), String> {
        if !self.duration_secs.is_finite() || self.duration_secs <= 0.0 {
            return Err("EMP requires positive finite duration".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmpResistanceConfig {
    pub resistance_per_stack: f32,
    pub max_stacks: u16,
    pub decay_secs: f32,
    pub bindings: Vec<DamageBinding>,
}
impl EmpResistanceConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.resistance_per_stack.is_finite()
            || self.resistance_per_stack <= 0.0
            || self.max_stacks == 0
            || !self.decay_secs.is_finite()
            || self.decay_secs <= 0.0
            || self.bindings.is_empty()
            || self
                .bindings
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.bindings.len()
        {
            return Err("EMP resistance requires positive finite reduction/decay, bounded stacks and distinct component bindings".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmpClock {
    pub tick: u64,
    pub period_nanos: u64,
}
impl EmpClock {
    pub fn validate(&self, tick: u64) -> Result<(), String> {
        if self.period_nanos == 0 || self.tick > tick {
            return Err("invalid EMP logical clock".into());
        }
        Ok(())
    }
    pub fn ticks(&self, seconds: f64) -> Result<u64, String> {
        if self.period_nanos == 0 || !seconds.is_finite() || seconds <= 0.0 {
            return Err("EMP duration requires a positive fixed period and finite duration".into());
        }
        let ticks = (seconds * 1_000_000_000.0 / self.period_nanos as f64)
            .ceil()
            .max(1.0);
        // The f64 representation of u64::MAX rounds up to 2^64. Refuse that
        // endpoint rather than accepting a saturating float-to-int conversion.
        if !ticks.is_finite() || ticks >= u64::MAX as f64 {
            return Err("EMP duration exceeds the supported logical frontier".into());
        }
        Ok(ticks as u64)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentEmpState {
    pub applied_tick: u64,
    pub effect_until_tick: u64,
    pub resistance_stacks: u16,
    pub next_decay_tick: u64,
    pub decay_ticks: u64,
}
impl ComponentEmpState {
    /// Shape validation can be used for portable hardware without consulting
    /// world truth. Full continuation additionally binds the fixed clock.
    pub fn validate_shape(&self, policy: &EmpResistanceConfig) -> Result<(), String> {
        policy.validate()?;
        if self.effect_until_tick <= self.applied_tick
            || self.decay_ticks == 0
            || self.resistance_stacks == 0
            || self.resistance_stacks > policy.max_stacks
            || self
                .effect_until_tick
                .checked_add(self.decay_ticks)
                .is_none_or(|earliest| self.next_decay_tick < earliest)
        {
            return Err("invalid temporary component EMP or resistance frontier".into());
        }
        Ok(())
    }
    pub fn validate(&self, policy: &EmpResistanceConfig, clock: EmpClock) -> Result<(), String> {
        self.validate_shape(policy)?;
        clock.validate(clock.tick)?;
        if self.applied_tick > clock.tick
            || self.decay_ticks != clock.ticks(f64::from(policy.decay_secs))?
        {
            return Err("EMP component frontier disagrees with the causal fixed clock".into());
        }
        Ok(())
    }
    pub fn factor_at(&self, tick: u64) -> f32 {
        if self.applied_tick <= tick && tick < self.effect_until_tick {
            0.0
        } else {
            1.0
        }
    }
    pub fn stacks_at(&self, tick: u64) -> u16 {
        if tick < self.next_decay_tick {
            return self.resistance_stacks;
        }
        if self.decay_ticks == 0 {
            return 0;
        }
        let lost = ((tick - self.next_decay_tick) / self.decay_ticks).saturating_add(1);
        self.resistance_stacks
            .saturating_sub(lost.min(u64::from(u16::MAX)) as u16)
    }
    /// Old resistance adjusts this application; the new stack only affects
    /// later hits. Positive supported EMP always lasts at least one tick.
    pub fn apply(
        previous: Option<&Self>,
        policy: &EmpResistanceConfig,
        payload: EmpPayload,
        clock: EmpClock,
    ) -> Result<Self, String> {
        policy.validate()?;
        payload.validate()?;
        clock.validate(clock.tick)?;
        if let Some(previous) = previous {
            previous.validate(policy, clock)?;
        }
        let stacks = previous.map_or(0, |old| old.stacks_at(clock.tick));
        let adjusted = f64::from(payload.duration_secs)
            / (1.0 + f64::from(stacks) * f64::from(policy.resistance_per_stack));
        let proposed = clock
            .tick
            .checked_add(clock.ticks(adjusted)?)
            .ok_or("EMP expiry overflows logical time")?;
        let old_active = previous.filter(|old| old.effect_until_tick > clock.tick);
        let effect_until_tick =
            old_active.map_or(proposed, |old| old.effect_until_tick.max(proposed));
        let decay_ticks = clock.ticks(f64::from(policy.decay_secs))?;
        let decay_start = effect_until_tick
            .checked_add(decay_ticks)
            .ok_or("EMP resistance expiry overflows logical time")?;
        let next = Self {
            applied_tick: clock.tick,
            effect_until_tick,
            resistance_stacks: stacks.saturating_add(1).min(policy.max_stacks),
            next_decay_tick: previous
                .filter(|_| stacks > 0)
                .map_or(decay_start, |old| old.next_decay_tick.max(decay_start)),
            decay_ticks,
        };
        next.validate(policy, clock)?;
        Ok(next)
    }
}

#[cfg(test)]
#[path = "emp_tests.rs"]
mod tests;
