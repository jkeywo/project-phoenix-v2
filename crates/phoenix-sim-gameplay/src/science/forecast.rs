//! Evidence-only forecasts. This input port cannot reach world state, private
//! entity associations, scenario scripts or unobserved model parameters.
use phoenix_model::messages::ScanReadingSnapshot;
use phoenix_model::sensing_forecast::{
    ForecastQuantity, ForecastRange, ScanForecast, ScanForecastPolicy,
};
use std::collections::BTreeMap;

fn condition(reading: &ScanReadingSnapshot) -> Option<(f64, f64)> {
    let value = f64::from(reading.condition_fraction);
    let step = f64::from(reading.condition_step);
    if !value.is_finite() || !(0.0..=1.0).contains(&value) || !step.is_finite() || step <= 0.0 {
        return None;
    }
    Some(((value - step / 2.0).max(0.0), (value + step / 2.0).min(1.0)))
}

pub fn derive(
    history: &[ScanReadingSnapshot],
    current: &ScanReadingSnapshot,
    policy: &ScanForecastPolicy,
    tick_secs: f64,
) -> Option<ScanForecast> {
    if policy.validate().is_err()
        || !tick_secs.is_finite()
        || tick_secs <= 0.0
        || current.subject_uuid.is_empty()
    {
        return None;
    }
    let previous_tick = history
        .iter()
        .filter(|row| {
            row.subject_uuid == current.subject_uuid && row.taken_at_tick < current.taken_at_tick
        })
        .map(|row| row.taken_at_tick)
        .max()?;
    let mut candidates = history.iter().filter(|row| {
        row.subject_uuid == current.subject_uuid && row.taken_at_tick == previous_tick
    });
    let previous = candidates.next()?;
    // Conflicting records at one sample time are uncertainty, not a free choice
    // of whichever history ordering happens to give the most useful slope.
    if candidates.any(|row| {
        row.condition_fraction != previous.condition_fraction
            || row.condition_step != previous.condition_step
            || row.capacities != previous.capacities
    }) {
        return None;
    }
    let elapsed = (current.taken_at_tick - previous_tick) as f64 * tick_secs;
    let ratio = f64::from(policy.horizon_secs) / elapsed;
    if !elapsed.is_finite() || elapsed <= 0.0 || !ratio.is_finite() {
        return None;
    }
    let (old_low, old_high) = condition(previous)?;
    let (new_low, new_high) = condition(current)?;
    let mut ranges = vec![ForecastRange {
        quantity: ForecastQuantity::Condition,
        lower: (new_low + (new_low - old_high) * ratio).clamp(0.0, 1.0),
        upper: (new_high + (new_high - old_low) * ratio).clamp(0.0, 1.0),
    }];
    // Labels are the observed quantity identity, not an internal inventory id.
    // Duplicate labels are ambiguous and are not extrapolated.
    let capacities = |reading: &ScanReadingSnapshot| {
        let mut values = BTreeMap::new();
        for (label, amount) in &reading.capacities {
            values
                .entry(label.clone())
                .and_modify(|held| *held = None)
                .or_insert(Some(*amount));
        }
        values
    };
    let old = capacities(previous);
    for (label, amount) in capacities(current) {
        if label.is_empty() {
            continue;
        }
        if let (Some(new), Some(Some(old))) = (amount, old.get(&label)) {
            // Outside binary64's exact integer range, neighbouring discrete
            // observations must not collapse into a falsely constant flow.
            const MAX_EXACT_INTEGER: i128 = 1_i128 << 53;
            let change = i128::from(new) - i128::from(*old);
            if [i128::from(new), i128::from(*old), change]
                .into_iter()
                .any(|value| value.abs() > MAX_EXACT_INTEGER)
            {
                continue;
            }
            // Stored resource amounts are discrete observed levels. The exact
            // recent net change is conditional, not a predicted consumption job.
            let value = new as f64 + change as f64 * ratio;
            if value.is_finite() {
                ranges.push(ForecastRange {
                    quantity: ForecastQuantity::Capacity { label },
                    lower: value,
                    upper: value,
                });
            }
        }
    }
    Some(ScanForecast {
        sample_ticks: [previous_tick, current.taken_at_tick],
        sample_interval_secs: elapsed,
        horizon_secs: policy.horizon_secs,
        assumptions: vec![
            "component.scan.forecast.linear".into(),
            "component.scan.forecast.quantisation".into(),
        ],
        ranges,
    })
}

#[cfg(test)]
#[path = "forecast_tests.rs"]
mod tests;
