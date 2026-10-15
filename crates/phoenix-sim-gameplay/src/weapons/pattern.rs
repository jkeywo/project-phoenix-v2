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
#[serde(deny_unknown_fields)]
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

/// Resolve a timed volley. Equal offsets retain authored step and barrel order.
/// With no authored pattern, the strategy's uniform count/cadence is retained.
pub fn firing_schedule(
    pattern: &[BarrelPatternStep],
    uniform_count: u32,
    uniform_interval_secs: f32,
) -> Vec<(Vec<u32>, f32)> {
    if !pattern.is_empty() {
        let mut steps: Vec<(Vec<u32>, f32)> = pattern
            .iter()
            .map(|s| (s.barrels.clone(), s.offset_secs.max(0.0)))
            .collect();
        steps.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        steps
    } else {
        (0..uniform_count)
            .map(|i| (vec![0u32], i as f32 * uniform_interval_secs))
            .collect()
    }
}

/// Resolve origins for an ammunition-counted burst. Offset order determines
/// barrel order, never round count or cadence. The tube owns both of those.
/// The step number is one-based; implicit single-origin fire reports step zero.
pub fn barrel_sequence(pattern: &[BarrelPatternStep]) -> Vec<(u32, u32)> {
    if pattern.is_empty() {
        return vec![(0, 0)];
    }
    let mut steps: Vec<&BarrelPatternStep> = pattern.iter().collect();
    steps.sort_by(|a, b| {
        a.offset_secs
            .partial_cmp(&b.offset_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seq = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        for &b in &step.barrels {
            seq.push((b, (i + 1) as u32));
        }
    }
    if seq.is_empty() {
        seq.push((0, 0));
    }
    seq
}

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
        if step
            .barrels
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != step.barrels.len()
        {
            return Err(format!(
                "{owner} pattern step {i} lists the same physical barrel more than once"
            ));
        }
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
