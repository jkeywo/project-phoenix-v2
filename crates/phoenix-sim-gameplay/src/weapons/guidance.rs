//! Finite measured guidance inputs; physical collision candidates are unrelated.
use phoenix_model::guided_guidance::{
    GuidanceObservation, GuidanceReceiverProfile, GuidanceSource, SeekerProfile,
};

/// Actual instrument geometry uses widened differences; neither a caller's
/// target roster nor a target UUID grants a measurement outside this cone.
#[allow(clippy::too_many_arguments)]
pub fn seeker_observation(
    profile: &SeekerProfile,
    round_position: [f32; 3],
    heading: f32,
    pitch: f32,
    target_uuid: &str,
    target_position: [f32; 3],
    transmission: f32,
    sampled_tick: u64,
) -> Option<GuidanceObservation> {
    if profile.validate().is_err()
        || !heading.is_finite()
        || !pitch.is_finite()
        || !round_position
            .iter()
            .chain(target_position.iter())
            .all(|value| value.is_finite())
        || !transmission.is_finite()
        || transmission < profile.minimum_transmission
        || transmission > 1.0
    {
        return None;
    }
    if !super::mount::contains_3d(
        round_position,
        heading,
        pitch,
        target_position,
        profile.range,
        profile.fire_arc_deg,
    ) {
        return None;
    }
    let result = GuidanceObservation {
        target_uuid: target_uuid.into(),
        position: target_position,
        sampled_tick,
        source: GuidanceSource::Seeker,
    };
    result.validate(sampled_tick).ok()?;
    Some(result)
}

/// Receipt freshness always derives from the original sample and fixed period,
/// never from arrival time. Keeping an already accepted point is separate memory.
pub fn receiver_accepts(
    profile: &GuidanceReceiverProfile,
    receiver_position: [f32; 3],
    transmitter_position: [f32; 3],
    transmission: f32,
    observation: &GuidanceObservation,
    tick: u64,
    fixed_secs: f64,
) -> bool {
    link_reaches(
        profile,
        receiver_position,
        transmitter_position,
        transmission,
    ) && observation.validate(tick).is_ok()
        && fixed_secs.is_finite()
        && fixed_secs > 0.0
        && tick.saturating_sub(observation.sampled_tick) as f64 * fixed_secs
            <= f64::from(profile.maximum_sample_age_secs)
}

/// Physical equipment link only; a stop/acknowledgement carries no target sample.
pub fn link_reaches(
    profile: &GuidanceReceiverProfile,
    receiver_position: [f32; 3],
    transmitter_position: [f32; 3],
    transmission: f32,
) -> bool {
    if profile.validate().is_err()
        || !receiver_position
            .iter()
            .chain(transmitter_position.iter())
            .all(|value| value.is_finite())
        || !transmission.is_finite()
        || transmission < profile.minimum_transmission
        || transmission > 1.0
    {
        return false;
    }
    let delta = glam::DVec3::from_array(receiver_position.map(f64::from))
        - glam::DVec3::from_array(transmitter_position.map(f64::from));
    delta.length() <= f64::from(profile.range)
}

#[cfg(test)]
#[path = "guidance_tests.rs"]
mod tests;
