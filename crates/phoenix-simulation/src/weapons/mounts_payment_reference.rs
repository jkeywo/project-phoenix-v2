use super::*;
#[allow(clippy::too_many_arguments)]
pub(super) fn advance_system_reference(
    profiles: &MountProfiles,
    states: &mut ShipMounts,
    owner: &SystemId,
    tick: u64,
    dt: f32,
    enabled: bool,
    desired_deg: Option<f32>,
    factor: f32,
    power: &mut crate::ship::power::ShipPowerSystem,
) -> Result<(), String> {
    let config = profiles.0.get(owner).ok_or("unknown mounted System")?;
    let state = states.0.get(owner).ok_or("missing physical turret state")?;
    state.validate(config, tick)?;
    if !dt.is_finite() || dt < 0.0 || !factor.is_finite() || !(0.0..=1.0).contains(&factor) {
        return Err("invalid turret duration or actual operation fraction".into());
    }
    let movement = state.movement_deg(config, desired_deg)?.abs();
    if state.last_tick == Some(tick) {
        return Ok(());
    }
    let offered = (movement / f64::from(config.traverse_deg_per_sec)).min(f64::from(dt)) as f32;
    let mut next = state.clone();
    if !enabled || factor == 0.0 || offered == 0.0 {
        next.advance(config, tick, 0.0, false, desired_deg, 0.0)?;
        states.0.insert(owner.clone(), next);
        return Ok(());
    }
    let mut paid = power.0.clone();
    let grant = paid.work_continuous(owner, offered, 1.0)?;
    next.advance(
        config,
        tick,
        offered,
        true,
        desired_deg,
        grant.fraction * factor,
    )?;
    power.0 = paid;
    states.0.insert(owner.clone(), next);
    Ok(())
}
