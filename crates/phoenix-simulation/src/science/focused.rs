//! Continuation for the existing focused scan owner. Acquisition is still
//! performed by `science::server`; these records never query hidden truth.
use super::server::{FocusedSaveState, FocusedScanRecord};
use crate::science::scan::ScanReading;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusedWork {
    pub aim: phoenix_model::focused_enquiry::FocusedAim,
    pub intrusive: bool,
    pub paid_started_tick: Option<u64>,
    pub subject: String,
    pub presentation_id: String,
    pub progress: f32,
    pub work_secs: f32,
    pub started_tick: u64,
    pub last_valid_tick: u64,
    pub last_tick: u64,
    pub waiting: String,
}
impl FocusedWork {
    pub fn key(&self) -> String {
        Self::key_for(&self.subject, self.aim, self.intrusive)
    }
    pub fn key_for(
        subject: &str,
        aim: phoenix_model::focused_enquiry::FocusedAim,
        intrusive: bool,
    ) -> String {
        format!(
            "{}:{}:{}:{}",
            subject.len(),
            subject,
            aim.as_str(),
            intrusive
        )
    }
}

impl FocusedScanRecord {
    pub(crate) fn validate_state(&self, saved: &FocusedSaveState, tick: u64) -> Result<(), String> {
        let policy = &self.policy;
        validate(saved, usize::from(policy.history_limit), tick)?;
        for work in saved.focused.iter().chain(saved.unfinished.values()) {
            if policy
                .aim_cost(work.aim, work.intrusive)
                .is_none_or(|(secs, _)| secs != work.work_secs)
            {
                return Err("focused work names an unsupported equipment question".into());
            }
        }
        if saved.history.iter().any(|reading| {
            policy.aim_cost(reading.aim, reading.intrusive).is_none()
                || !self.config.bands.iter().any(|band| {
                    band.id == reading.band
                        && band.label == reading.band_label
                        && reading
                            .condition_step
                            .is_none_or(|step| band.condition_step == step)
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

pub(crate) fn validate(saved: &FocusedSaveState, limit: usize, tick: u64) -> Result<(), String> {
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
            || !valid_reading_shape(reading)
        {
            return Err("invalid recorded scan sample frontier".into());
        }
        previous = reading.taken_at_tick;
        if let Some(forecast) = &reading.forecast {
            forecast.validate(
                reading.taken_at_tick,
                reading.condition_fraction,
                &reading.capacities,
                &reading.quantities,
            )?;
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
            .any(|(key, work)| key != &work.key())
        || saved
            .focused
            .as_ref()
            .is_some_and(|work| saved.unfinished.contains_key(&work.key()))
    {
        return Err("focused enquiry ownership or retained-work bound is invalid".into());
    }
    for work in saved.focused.iter().chain(saved.unfinished.values()) {
        if work.subject.is_empty()
            || work.presentation_id.is_empty()
            || work.presentation_id.len() > 512
            || work.presentation_id.chars().any(char::is_control)
            || !work.work_secs.is_finite()
            || work.work_secs <= 0.0
            || !work.progress.is_finite()
            || !(0.0..=work.work_secs).contains(&work.progress)
            || work.started_tick > work.last_valid_tick
            || work.last_valid_tick > work.last_tick
            || work.last_tick > tick
            || work
                .paid_started_tick
                .is_some_and(|paid| paid < work.started_tick || paid > work.last_valid_tick)
            || (work.progress > 0.0 && work.paid_started_tick.is_none())
        {
            return Err("focused scan continuation disagrees with accepted work".into());
        }
    }
    Ok(())
}

fn valid_reading_shape(reading: &ScanReading) -> bool {
    use phoenix_model::focused_enquiry::{FocusedAim, ScanQuantityKind};
    if reading.aim == FocusedAim::General {
        return reading.quantities.is_empty()
            && reading
                .condition_fraction
                .is_some_and(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            && reading
                .condition_step
                .is_some_and(|value| value.is_finite() && value > 0.0)
            && reading
                .mass
                .is_some_and(|value| value.is_finite() && value > 0.0);
    }
    let mut keys = BTreeSet::new();
    reading.condition_fraction.is_none()
        && reading.condition_step.is_none()
        && reading.mass.is_none()
        && reading.flags.is_empty()
        && reading.capacities.is_empty()
        && reading.debris.is_none()
        && reading.mass_class.is_empty()
        && reading.mass_class_label.is_empty()
        && !reading.quantities.is_empty()
        && reading.quantities.iter().all(|row| {
            row.validate().is_ok()
                && keys.insert((row.kind, &row.label))
                && matches!(
                    (reading.aim, row.kind),
                    (FocusedAim::Environment, ScanQuantityKind::FieldExposure)
                        | (FocusedAim::Resources, ScanQuantityKind::Capacity)
                        | (FocusedAim::Anomaly, ScanQuantityKind::EffectSignature)
                )
        })
}

#[cfg(test)]
#[path = "focused_tests.rs"]
mod tests;
