//! Local-reserve work at an authored capability boundary. Missing equipment is
//! unavailable, rather than an implicit source of free work.
use crate::modifiers::power_system::PowerSystem;

pub fn continuous_for_kind(
    power: Option<&mut PowerSystem>,
    kind: &str,
    dt: f32,
    activity: f32,
) -> f32 {
    let Some(power) = power else { return 0.0 };
    let Some(id) = power
        .ship_config()
        .systems
        .iter()
        .find(|system| system.kind == kind)
        .map(|system| system.id.clone())
    else {
        return 0.0;
    };
    power
        .work_continuous(&id, dt, activity)
        .map_or(0.0, |grant| grant.fraction)
}

#[cfg(test)]
#[path = "energy_work_tests.rs"]
mod tests;
