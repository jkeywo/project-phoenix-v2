//! Transaction-local use of existing stocks and weapon operating parameters.
//! The actual executor stages Power plus these same stock owners together.
use crate::resources::Stock;
pub use phoenix_model::weapon_operation::{BeamOperation, WeaponRoundFeed};

/// Reserve one actual barrel's rounds before paying work. Claim collisions are
/// rejected even when amounts match; another owner's reservation is not ammo.
pub fn claim_rounds(
    stock: &mut Stock,
    feed: &WeaponRoundFeed,
    claim: &str,
) -> Result<bool, String> {
    feed.validate()?;
    stock.validate()?;
    if claim.trim().is_empty() || stock.reserved.contains_key(claim) {
        return Err("weapon round claim must be new and nonempty".into());
    }
    if stock.available() < feed.amount {
        return Ok(false);
    }
    stock.reserve(claim, feed.amount)?;
    Ok(true)
}

/// Complete the already validated claim within the same staged transaction.
/// A failed Power grant releases it; successful work consumes exactly it.
pub fn finish_rounds(
    stock: &mut Stock,
    feed: &WeaponRoundFeed,
    claim: &str,
    paid: bool,
) -> Result<bool, String> {
    feed.validate()?;
    stock.validate()?;
    if stock.reserved.get(claim) != Some(&feed.amount) {
        return Err("weapon round settlement must match its actual claim".into());
    }
    if paid {
        stock.consume(claim);
    } else {
        stock.release(claim);
    }
    Ok(paid)
}

/// No infinity sentinel or second beam timer: the existing active slot freezes
/// its operation, and only fixed beams spend their remaining duration.
pub fn beam_work_seconds(
    operation: BeamOperation,
    remaining_secs: f32,
    dt: f32,
) -> Result<f32, String> {
    if !remaining_secs.is_finite() || remaining_secs < 0.0 || !dt.is_finite() || dt < 0.0 {
        return Err("beam work requires finite nonnegative remaining time and timestep".into());
    }
    Ok(match operation {
        BeamOperation::Fixed => remaining_secs.min(dt),
        BeamOperation::Sustained => dt,
    })
}

/// Actual world XZ velocity impulse, opposite the emitted projectile bearing.
/// Retains existing authored recoil units; the caller projects onto ship axes.
pub fn recoil_delta(heading: f32, impulse: f32) -> Result<[f32; 2], String> {
    if !heading.is_finite() || !impulse.is_finite() || impulse < 0.0 {
        return Err("recoil requires a finite heading and nonnegative impulse".into());
    }
    Ok([
        -crate::simmath::sin(heading) * impulse,
        crate::simmath::cos(heading) * impulse,
    ])
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
