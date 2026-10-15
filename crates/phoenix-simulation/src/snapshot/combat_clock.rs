//! Validate every historical combat timestamp before any candidate state writes.
use super::*;

pub(super) fn validate(world: &World, snapshot: &PhoenixSnapshot) -> Result<(), (String, String)> {
    let fixed = world.get_resource::<Time<Fixed>>();
    let period = fixed.map(|time| time.timestep().as_secs_f64());
    let logical = world.contains_resource::<SimTick>();
    let now = phoenix_sim_contracts::logical_clock::sample(
        logical.then_some(snapshot.tick),
        period,
        phoenix_sim_contracts::logical_clock::from_world(world),
    ) as f32;
    for row in &snapshot.entities {
        let Some(state) = &row.recent_combat else {
            continue;
        };
        let fail = |reason: &str| Err((row.uuid.clone(), reason.to_string()));
        if !state.prev_hull.is_finite() || state.prev_hull < 0.0 {
            return fail("combat history requires finite nonnegative previous hull");
        }
        let timestamps = [
            state.last_damage_taken_secs,
            state.last_hostile_fire_taken_secs,
            state.last_weapon_fired_secs,
        ];
        if timestamps.iter().all(Option::is_none) {
            continue;
        }
        if logical && !period.is_some_and(|period| period.is_finite() && period > 0.0) {
            return fail("combat history requires a finite positive fixed clock");
        }
        if !now.is_finite()
            || timestamps
                .into_iter()
                .flatten()
                .any(|timestamp| !timestamp.is_finite() || timestamp < 0.0 || timestamp > now)
        {
            return fail("combat timestamp must be finite, nonnegative and no later than the saved logical clock");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "combat_clock_tests.rs"]
mod tests;
