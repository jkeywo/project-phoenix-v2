//! Controller reading behaviour over held semantic text. No processing resource,
//! command emission or knowledge acquisition lives here.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextReading {
    pub key: String,
    pub required_ticks: u32,
    pub completed_ticks: u32,
    pub last_tick: Option<u64>,
}
impl TextReading {
    pub fn validate(&self, tick: u64) -> Result<(), String> {
        if self.key.is_empty()
            || self.key.len() > 4096
            || self.key.chars().any(char::is_control)
            || self.completed_ticks > self.required_ticks
            || self.last_tick.is_none_or(|last| last > tick)
        {
            return Err("invalid AI semantic reading frontier".into());
        }
        Ok(())
    }

    /// Called once per logical tick, including paused ticks. A gap never earns
    /// work: it may have contained a human handoff or unavailable information.
    pub fn observe(
        held: &mut Option<Self>,
        key: &str,
        required_ticks: u32,
        tick: u64,
        eligible: bool,
    ) -> Result<bool, String> {
        if key.is_empty() || key.len() > 4096 || key.chars().any(char::is_control) {
            return Err("invalid AI reading key".into());
        }
        if let Some(prior) = held.as_ref() {
            prior.validate(tick)?;
            if prior.last_tick == Some(tick) {
                return Ok(prior.key == key
                    && prior.required_ticks == required_ticks
                    && prior.completed_ticks == required_ticks
                    && eligible);
            }
        }
        let same = held
            .as_ref()
            .is_some_and(|old| old.key == key && old.required_ticks == required_ticks);
        if !same {
            *held = Some(Self {
                key: key.into(),
                required_ticks,
                completed_ticks: 0,
                last_tick: Some(tick),
            });
        }
        let current = held.as_mut().expect("selected reading");
        if eligible && same && current.last_tick.and_then(|last| last.checked_add(1)) == Some(tick)
        {
            current.completed_ticks = current
                .completed_ticks
                .saturating_add(1)
                .min(required_ticks);
        }
        current.last_tick = Some(tick);
        Ok(eligible && current.completed_ticks == required_ticks)
    }

    pub fn ready(&self, key: &str, tick: u64) -> bool {
        self.key == key
            && self.last_tick == Some(tick)
            && self.completed_ticks == self.required_ticks
    }
}

#[cfg(test)]
#[path = "text_reading_tests.rs"]
mod tests;
