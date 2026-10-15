//! Whole-candidate guided flight validation before any restore writes.
use super::{PhoenixSnapshot, WeaponState};
use std::collections::BTreeSet;

fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

pub(super) fn validate_weapon_state(stored: &WeaponState) -> Result<(), String> {
    for round in &stored.torpedoes_in_flight {
        round
            .profile
            .validate_active(round.lifespan_remaining, round.durability_remaining)?;
        if let Some(observation) = &round.last_guidance {
            observation.validate(u64::MAX)?;
            if round.target_uuid.as_deref() != Some(observation.target_uuid.as_str())
                || match observation.source {
                    phoenix_model::guided_guidance::GuidanceSource::Seeker => {
                        round.profile.seeker.is_none()
                    }
                    phoenix_model::guided_guidance::GuidanceSource::Console { .. } => {
                        round.profile.receiver.is_none()
                    }
                }
            {
                return Err(
                    "guidance observation has no matching supported round intent/equipment".into(),
                );
            }
        }
        if !identity(&round.uuid)
            || !identity(&round.tube_id)
            || round
                .target_uuid
                .as_deref()
                .is_some_and(|value| !identity(value))
            || round
                .source_uuid
                .as_deref()
                .is_some_and(|value| !identity(value))
            || round.position.iter().any(|value| !value.is_finite())
            || !round.heading.is_finite()
            || !round.pitch.is_finite()
            || round.pitch.abs() > std::f32::consts::FRAC_PI_2
            || !round.strike_damage_bonus.is_finite()
            || round.strike_damage_bonus < -1.0
        {
            return Err("invalid guided round identity, pose or paid strike modifier".into());
        }
    }
    let mut tubes = BTreeSet::new();
    for burst in &stored.bursts {
        burst.profile.validate()?;
        if !identity(&burst.tube_id)
            || !tubes.insert(&burst.tube_id)
            || burst.pending == 0
            || !burst.burst_interval_secs.is_finite()
            || burst.burst_interval_secs <= 0.0
            || !burst.timer.is_finite()
            || burst.timer > burst.burst_interval_secs
            || burst.launch.iter().any(|value| !value.is_finite())
            || !burst.launch_heading.is_finite()
            || burst
                .target_uuid
                .as_deref()
                .is_some_and(|value| !identity(value))
            || burst
                .source_uuid
                .as_deref()
                .is_some_and(|value| !identity(value))
            || burst
                .barrel_origins
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
            || burst.next_shot_index == 0
            || burst.next_shot_index.checked_add(burst.pending).is_none()
            || (!burst.barrel_origins.is_empty()
                && burst
                    .barrel_sequence
                    .iter()
                    .any(|(barrel, _)| *barrel as usize >= burst.barrel_origins.len()))
        {
            return Err("invalid guided burst identity, frozen cadence, origin or cursor".into());
        }
    }
    Ok(())
}

pub(super) fn validate_roster(snapshot: &PhoenixSnapshot) -> Result<(), (String, String)> {
    let mut projectile_ids: BTreeSet<_> = snapshot
        .entities
        .iter()
        .map(|row| &row.uuid)
        .chain(snapshot.asteroids.iter().map(|row| &row.uuid))
        .collect();
    for row in &snapshot.entities {
        let Some(weapons) = &row.weapons else {
            continue;
        };
        validate_weapon_state(weapons).map_err(|reason| (row.uuid.clone(), reason))?;
        for round in &weapons.torpedoes_in_flight {
            if let Some(observation) = &round.last_guidance {
                observation
                    .validate(snapshot.tick)
                    .map_err(|reason| (row.uuid.clone(), reason))?;
            }
        }
        for id in weapons
            .torpedoes_in_flight
            .iter()
            .map(|round| &round.uuid)
            .chain(
                weapons
                    .blasters
                    .iter()
                    .flat_map(|bank| bank.in_flight.iter().map(|round| &round.id)),
            )
        {
            if !identity(id) || !projectile_ids.insert(id) {
                return Err((
                    row.uuid.clone(),
                    "guided/bolt projectile identities must be unique across the complete roster"
                        .into(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "guided_rounds_tests.rs"]
mod tests;
