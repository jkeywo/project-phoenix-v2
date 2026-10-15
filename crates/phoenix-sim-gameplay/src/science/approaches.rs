//! Conditional closest approaches from delivered observations only. This input
//! port has no World, projectile, routing, intent or target-speed access.
use phoenix_model::sensing_tracks::{ObservedApproach, SensorTrackSnapshot};

pub fn derive(
    records: &[SensorTrackSnapshot],
    position: [f32; 2],
    velocity: [f32; 2],
    horizon_secs: f32,
) -> Vec<ObservedApproach> {
    if !position.into_iter().chain(velocity).all(f32::is_finite)
        || !horizon_secs.is_finite()
        || horizon_secs <= 0.0
    {
        return Vec::new();
    }
    let mut unique = std::collections::BTreeMap::<&str, Option<&SensorTrackSnapshot>>::new();
    for record in records {
        unique
            .entry(&record.contact_id)
            .and_modify(|held| {
                if held.is_some_and(|held| held != record) {
                    *held = None;
                }
            })
            .or_insert(Some(record));
    }
    let mut result = Vec::new();
    for record in unique.into_values().flatten() {
        if record.contact_id.is_empty()
            || !matches!(
                record.classification.as_deref(),
                Some("torpedo" | "missile")
            )
            || !record.position.iter().all(|value| value.is_finite())
            || !record.position_error.is_finite()
            || record.position_error < 0.0
        {
            continue;
        }
        let Some(motion) = record
            .velocity
            .filter(|motion| motion.iter().all(|v| v.is_finite()))
        else {
            continue;
        };
        // Without actual sample-error metadata, do not invent a precision.
        if record.samples.is_empty()
            || record.samples.iter().any(|sample| {
                !sample.velocity_error.is_finite()
                    || sample.velocity_error < 0.0
                    || !sample.drift_error_per_sec.is_finite()
                    || sample.drift_error_per_sec < 0.0
            })
        {
            continue;
        }
        let r = [
            f64::from(record.position[0]) - f64::from(position[0]),
            f64::from(record.position[1]) - f64::from(position[1]),
        ];
        let v = [
            f64::from(motion[0]) - f64::from(velocity[0]),
            f64::from(motion[1]) - f64::from(velocity[1]),
        ];
        let closing = r[0] * v[0] + r[1] * v[1];
        let speed_squared = v[0] * v[0] + v[1] * v[1];
        if closing >= 0.0 || speed_squared <= 0.0 {
            continue;
        }
        let seconds = (-closing / speed_squared).min(f64::from(horizon_secs));
        let own = [
            f64::from(position[0]) + f64::from(velocity[0]) * seconds,
            f64::from(position[1]) + f64::from(velocity[1]) * seconds,
        ];
        let contact = [
            f64::from(record.position[0]) + f64::from(motion[0]) * seconds,
            f64::from(record.position[1]) + f64::from(motion[1]) * seconds,
        ];
        let separation = ((r[0] + v[0] * seconds).powi(2) + (r[1] + v[1] * seconds).powi(2)).sqrt();
        let widening = record
            .samples
            .iter()
            .map(|sample| f64::from(sample.velocity_error) + f64::from(sample.drift_error_per_sec))
            .fold(0.0_f64, f64::max);
        let own_position = own.map(|value| value as f32);
        let contact_position = contact.map(|value| value as f32);
        // A wire-space center can move when large coordinates are narrowed.
        // Enclose that displacement as well as the measured uncertainty.
        let rounding = (own[0] - f64::from(own_position[0]))
            .hypot(own[1] - f64::from(own_position[1]))
            + (contact[0] - f64::from(contact_position[0]))
                .hypot(contact[1] - f64::from(contact_position[1]));
        let error = f64::from(record.position_error) + widening * seconds + rounding;
        let bounds = [(separation - error).max(0.0), separation + error];
        if own
            .into_iter()
            .chain(contact)
            .chain(bounds)
            .chain([error, seconds])
            .any(|value| !value.is_finite() || value.abs() > f64::from(f32::MAX))
        {
            continue;
        }
        let upper = |value: f64| {
            let narrowed = value as f32;
            if f64::from(narrowed) < value {
                narrowed.next_up()
            } else {
                narrowed
            }
        };
        let lower = bounds[0] as f32;
        let lower = if f64::from(lower) > bounds[0] {
            lower.next_down()
        } else {
            lower
        };
        let position_error = upper(error);
        let upper_separation = upper(bounds[1]);
        if !position_error.is_finite() || !upper_separation.is_finite() {
            continue;
        }
        result.push(ObservedApproach {
            contact: record.contact_id.clone(),
            observed_tick: record.observed_tick,
            predicted: record.predicted,
            seconds: seconds as f32,
            own_position,
            contact_position,
            position_error,
            separation: [lower.max(0.0), upper_separation],
        });
    }
    result.sort_by(|a, b| {
        a.seconds
            .total_cmp(&b.seconds)
            .then(a.contact.cmp(&b.contact))
    });
    result
}

#[cfg(test)]
#[path = "approaches_tests.rs"]
mod tests;
