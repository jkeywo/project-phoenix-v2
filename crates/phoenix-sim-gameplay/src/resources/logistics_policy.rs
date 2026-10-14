use super::*;

/// Observation-only destination policy. Callers supply permitted published
/// quantities; this helper cannot inspect inventories, routes or another Console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplenishmentObservation {
    pub store: String,
    /// Physical, incoming and held quantity summed by the publication adapter.
    pub occupied: i64,
    pub target: i64,
    pub priority: u16,
}

/// Rank eligible desired stocks, independent of source availability. Lower
/// priority wins, then lower fullness relative to target, then stable store ID.
pub fn ordered_replenishment_candidates(
    observations: &[ReplenishmentObservation],
    handling: &HandlingConfig,
) -> Result<Vec<ReplenishmentObservation>, String> {
    if handling.max_batch <= 0
        || !handling.reorder_fraction.is_finite()
        || !(0.0..=1.0).contains(&handling.reorder_fraction)
    {
        return Err(
            "replenishment policy requires a positive batch and finite reorder fraction in 0..1"
                .into(),
        );
    }
    let mut ids = BTreeSet::new();
    let mut eligible = Vec::new();
    for observation in observations {
        if !named(&observation.store)
            || !ids.insert(&observation.store)
            || observation.occupied < 0
            || observation.target < 0
        {
            return Err(
                "replenishment observations require unique store IDs and nonnegative quantities"
                    .into(),
            );
        }
        if observation.target <= observation.occupied {
            continue;
        }
        let deficit = observation.target - observation.occupied;
        if deficit >= handling.max_batch
            || observation.occupied
                <= reorder_threshold(observation.target, handling.reorder_fraction)
        {
            eligible.push(observation.clone());
        }
    }
    eligible.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| {
                (i128::from(a.occupied) * i128::from(b.target))
                    .cmp(&(i128::from(b.occupied) * i128::from(a.target)))
            })
            .then_with(|| a.store.cmp(&b.store))
    });
    Ok(eligible)
}

/// Exact floor(target * the authored f32 rational). Casting a large i64 target
/// to f32 can round a threshold across several real stock units. The product
/// fits u128 (63 quantity bits + 24 significand bits), even for i64::MAX.
fn reorder_threshold(target: i64, fraction: f32) -> i64 {
    let bits = fraction.to_bits();
    let exponent = (bits >> 23) & 0xff;
    let significand = if exponent == 0 {
        bits & 0x7fffff
    } else {
        (bits & 0x7fffff) | (1 << 23)
    };
    let shift = if exponent == 0 { 149 } else { 150 - exponent };
    let product = (target as u128) * u128::from(significand);
    if shift >= 128 {
        0
    } else {
        (product >> shift) as i64
    }
}

#[cfg(test)]
#[path = "logistics_policy_tests.rs"]
mod tests;
