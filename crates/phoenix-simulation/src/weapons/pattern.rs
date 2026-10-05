//! Shared timed multi-barrel attack pattern types (issue #765).
//!
//! A *barrel pattern* is a timed schedule that decides which authored barrel
//! markers fire, and when, over the course of one volley. It is deliberately
//! weapon-family-agnostic: blasters wire it now (issue #765) and torpedoes
//! reuse the SAME types in issue #766, so the schema and its validation live
//! here rather than inside either weapon module.
//!
//! Semantics:
//!   - A [`BarrelPatternStep`] fires every barrel index in `barrels`
//!     SIMULTANEOUSLY at time `offset_secs` after the volley begins.
//!   - Successive steps at increasing offsets produce ALTERNATING fire.
//!
//! This module is pure (Bevy-free) and carries no gameplay defaults — every
//! number originates in TOML.

use serde::{Deserialize, Serialize};

/// One timed step of a [`BarrelPattern`].
///
/// `barrels` lists zero-based indices into the owner's authored barrel-marker
/// list. All listed barrels fire together at `offset_secs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BarrelPatternStep {
    /// Zero-based barrel indices that fire on this step, simultaneously.
    pub barrels: Vec<u32>,
    /// Seconds after the volley begins that this step fires. Successive steps
    /// with increasing offsets alternate; a single step with several barrels
    /// fires them together.
    #[serde(default)]
    pub offset_secs: f32,
}

/// An ordered, timed sequence of barrel-firing steps for one volley.
pub type BarrelPattern = Vec<BarrelPatternStep>;

/// Validate a barrel list + timed pattern for one weapon instance.
///
/// `barrel_count` is the number of authored barrels (callers pass `1` for the
/// implicit single-barrel/backward-compat case). `owner` is a human label used
/// in error strings (e.g. `blaster bank 'heavy-fore'`).
///
/// Rejects:
///   - a step with an empty `barrels` list,
///   - a barrel index `>= barrel_count`,
///   - a negative `offset_secs`,
///   - an empty pattern when more than one barrel is declared (under-specified:
///     with multiple barrels the author must say which fire when).
///
/// An empty pattern with `barrel_count <= 1` is accepted — it means "no pattern
/// authored", i.e. the uniform single-barrel volley behaviour.
pub fn validate_barrel_pattern(
    owner: &str,
    barrel_count: usize,
    pattern: &[BarrelPatternStep],
) -> Result<(), String> {
    if pattern.is_empty() {
        if barrel_count > 1 {
            return Err(format!(
                "{owner} declares {barrel_count} barrels but no firing pattern; \
                 multiple barrels require a pattern to say which fire when"
            ));
        }
        return Ok(());
    }
    for (i, step) in pattern.iter().enumerate() {
        if step.barrels.is_empty() {
            return Err(format!(
                "{owner} pattern step {i} fires no barrels (empty `barrels`)"
            ));
        }
        if !step.offset_secs.is_finite() || step.offset_secs < 0.0 {
            return Err(format!(
                "{owner} pattern step {i} has offset_secs={} (must be finite and >= 0)",
                step.offset_secs
            ));
        }
        for &b in &step.barrels {
            if b as usize >= barrel_count {
                return Err(format!(
                    "{owner} pattern step {i} references barrel index {b} \
                     but only {barrel_count} barrel(s) are declared"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "pattern_tests.rs"]
mod tests;
