//! Continuation for the existing focused scan owner. Acquisition is still
//! performed by `science::server`; these records never query hidden truth.
use super::server::{FocusedSaveState, FocusedScanRecord};
use crate::science::scan::ScanReading;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusedWork {
    pub subject: String,
    pub presentation_id: String,
    pub progress: f32,
    pub work_secs: f32,
    pub started_tick: u64,
    pub last_valid_tick: u64,
    pub last_tick: u64,
    pub waiting: String,
}

impl FocusedScanRecord {
    pub(crate) fn validate_state(&self, saved: &FocusedSaveState, tick: u64) -> Result<(), String> {
        let policy = &self.policy;
        validate(
            saved,
            usize::from(policy.history_limit),
            policy.work_secs,
            tick,
        )?;
        if saved.history.iter().any(|reading| {
            !self.config.bands.iter().any(|band| {
                band.id == reading.band
                    && band.label == reading.band_label
                    && band.condition_step == reading.condition_step
            })
        }) {
            return Err("recorded scan names an unsupported instrument fidelity".into());
        }
        if saved
            .history
            .iter()
            .filter_map(|reading| reading.forecast.as_ref())
            .any(|forecast| {
                policy
                    .forecast
                    .as_ref()
                    .is_none_or(|policy| policy.horizon_secs != forecast.horizon_secs)
            })
        {
            return Err("recorded forecast disagrees with accepted instrument policy".into());
        }
        Ok(())
    }
    /// The latest screen is a projection of this owner's retained readings.
    /// A refusal may clear that screen without erasing earlier evidence.
    pub(crate) fn remember(&mut self, mut reading: ScanReading, limit: usize, tick_secs: f64) {
        reading.forecast = self.policy.forecast.as_ref().and_then(|policy| {
            let history: Vec<_> = self
                .history
                .iter()
                .map(crate::core::messages::scan_reading_snapshot_from_reading)
                .collect();
            phoenix_sim_gameplay::science::forecast::derive(
                &history,
                &crate::core::messages::scan_reading_snapshot_from_reading(&reading),
                policy,
                tick_secs,
            )
        });
        if !self.history.iter().any(|held| held == &reading) {
            self.history.push(reading.clone());
            if self.history.len() > limit {
                self.history.drain(..self.history.len() - limit);
            }
        }
        self.last = Some(reading);
        self.refusal = None;
    }
}

pub(crate) fn validate(
    saved: &FocusedSaveState,
    limit: usize,
    work_secs: f32,
    tick: u64,
) -> Result<(), String> {
    if saved.history.len() > limit || saved.last.is_some() && saved.refusal.is_some() {
        return Err("scan history or latest projection exceeds its accepted profile".into());
    }
    let mut samples = BTreeSet::new();
    let mut previous = 0;
    for reading in &saved.history {
        if reading.subject_uuid.is_empty()
            || reading.contact_id.as_ref().is_none_or(|id| {
                id.is_empty() || id.len() > 512 || id.chars().any(char::is_control)
            })
            || reading.taken_at_tick > tick
            || reading.taken_at_tick < previous
            || !samples.insert((reading.subject_uuid.clone(), reading.taken_at_tick))
            || !reading.condition_fraction.is_finite()
            || !(0.0..=1.0).contains(&reading.condition_fraction)
            || !reading.condition_step.is_finite()
            || reading.condition_step <= 0.0
            || !reading.mass.is_finite()
            || reading.mass <= 0.0
        {
            return Err("invalid recorded scan sample frontier".into());
        }
        previous = reading.taken_at_tick;
        if let Some(forecast) = &reading.forecast {
            forecast.validate(reading.taken_at_tick, &reading.capacities)?;
        }
    }
    if saved
        .last
        .as_ref()
        .is_some_and(|last| saved.history.last() != Some(last))
    {
        return Err("latest scan disagrees with its canonical retained sample".into());
    }
    if saved.unfinished.len() > limit
        || saved
            .unfinished
            .iter()
            .any(|(key, work)| key != &work.subject)
        || saved
            .focused
            .as_ref()
            .is_some_and(|work| saved.unfinished.contains_key(&work.subject))
    {
        return Err("focused enquiry ownership or retained-work bound is invalid".into());
    }
    for work in saved.focused.iter().chain(saved.unfinished.values()) {
        if work.subject.is_empty()
            || work.presentation_id.is_empty()
            || work.presentation_id.len() > 512
            || work.presentation_id.chars().any(char::is_control)
            || work.work_secs != work_secs
            || !work.progress.is_finite()
            || !(0.0..=work_secs).contains(&work.progress)
            || work.started_tick > work.last_valid_tick
            || work.last_valid_tick > work.last_tick
            || work.last_tick > tick
        {
            return Err("focused scan continuation disagrees with accepted work".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "focused_tests.rs"]
mod tests;
