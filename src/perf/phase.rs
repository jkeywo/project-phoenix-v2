//! Pure reduction for #1400's externally observed FixedUpdate intervals.
//!
//! This is pure reduction, not a clock reader. No App, system or Resource
//! lives here. The harness-owned producer pairs real spans and must
//! resolve their phase membership without guessing and pass complete invocations.
//! Neither an execution union nor its envelope is inclusive SimSet wall time.

use crate::sim_sets::SimSet;
use std::collections::BTreeSet;
use std::time::Duration;
use vellum_perf::{Recorder, Unit};

const PHASES: [SimSet; 7] = [
    SimSet::Input,
    SimSet::Physics,
    SimSet::Damage,
    SimSet::Modifiers,
    SimSet::Publish,
    SimSet::PublishAggregate,
    SimSet::Broadcast,
];
pub const FIXED_ELAPSED: &str = "sim.fixed.elapsed";
pub const UNATTRIBUTED_OBSERVED: &str = "sim.fixed.unattributed.observed";
pub const UNOBSERVED: &str = "sim.fixed.unobserved";

/// Relative timestamps from one producer's monotonic capture epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub start: Duration,
    pub end: Duration,
}

/// None preserves unclassified/ambiguous work instead of assigning a phase by
/// name or dropping it. Duplicate/nested intervals are safe for union duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedSystem {
    pub interval: Interval,
    pub phase: Option<SimSet>,
}

/// One actual FixedUpdate invocation, not one app.update() or simulation tick
/// inferred from a frame count. The external producer supplies unique IDs.
#[derive(Clone, Debug)]
pub struct FixedInvocation {
    pub id: u64,
    pub interval: Interval,
    pub systems: Vec<ObservedSystem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Union of observed intervals: overlapping parallel work counts once.
    pub observed: Duration,
    /// First entry to last exit, including gaps between observed systems.
    pub envelope: Duration,
    /// Number of supplied intervals, preserving nested/duplicate multiplicity.
    pub intervals: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseTiming {
    pub phase: SimSet,
    pub timing: Timing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReducedInvocation {
    pub id: u64,
    pub elapsed: Duration,
    /// Only observed phases, in canonical SimSet order. Missing is not zero.
    pub phases: Vec<PhaseTiming>,
    pub unattributed: Option<Timing>,
    /// Invocation time outside the union of ALL observed system intervals.
    /// Includes scheduler/condition/deferred work and any producer coverage gaps.
    /// It must not be presented as pure executor overhead.
    pub unobserved: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReductionError {
    DuplicateInvocation(u64),
    ReversedInvocation(u64),
    InvalidSystemInterval { invocation: u64, index: usize },
}

impl std::fmt::Display for ReductionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid external phase timing sample: {self:?}")
    }
}
impl std::error::Error for ReductionError {}

pub fn observed_metric(phase: SimSet) -> &'static str {
    match phase {
        SimSet::Input => "sim.phase.input.observed",
        SimSet::Physics => "sim.phase.physics.observed",
        SimSet::Damage => "sim.phase.damage.observed",
        SimSet::Modifiers => "sim.phase.modifiers.observed",
        SimSet::Publish => "sim.phase.publish.observed",
        SimSet::PublishAggregate => "sim.phase.publish_aggregate.observed",
        SimSet::Broadcast => "sim.phase.broadcast.observed",
    }
}

fn timing(mut intervals: Vec<Interval>) -> Option<Timing> {
    intervals.sort_by_key(|interval| (interval.start, interval.end));
    let first = *intervals.first()?;
    let envelope_end = intervals.iter().map(|interval| interval.end).max().unwrap();
    let mut current = first;
    let mut observed = Duration::ZERO;
    for interval in intervals.iter().skip(1) {
        if interval.start <= current.end {
            current.end = current.end.max(interval.end);
        } else {
            observed += current.end - current.start;
            current = *interval;
        }
    }
    observed += current.end - current.start;
    Some(Timing {
        observed,
        envelope: envelope_end - first.start,
        intervals: intervals.len(),
    })
}

/// Validate and reduce a batch without reading a clock or mutating anything.
/// Every interval must fit its actual invocation; never clip malformed evidence.
/// Duplicate invocation IDs in a batch are refused, not silently double-sampled.
/// Across batches, consumption/uniqueness remains the external producer's duty.
pub fn reduce(invocations: &[FixedInvocation]) -> Result<Vec<ReducedInvocation>, ReductionError> {
    let mut ids = BTreeSet::new();
    let mut reduced = Vec::with_capacity(invocations.len());
    for invocation in invocations {
        if !ids.insert(invocation.id) {
            return Err(ReductionError::DuplicateInvocation(invocation.id));
        }
        if invocation.interval.end < invocation.interval.start {
            return Err(ReductionError::ReversedInvocation(invocation.id));
        }
        for (index, system) in invocation.systems.iter().enumerate() {
            if system.interval.end < system.interval.start
                || system.interval.start < invocation.interval.start
                || system.interval.end > invocation.interval.end
            {
                return Err(ReductionError::InvalidSystemInterval {
                    invocation: invocation.id,
                    index,
                });
            }
        }
        let intervals_for = |phase| {
            invocation
                .systems
                .iter()
                .filter(|system| system.phase == phase)
                .map(|system| system.interval)
                .collect()
        };
        let phases = PHASES
            .into_iter()
            .filter_map(|phase| {
                timing(intervals_for(Some(phase))).map(|timing| PhaseTiming { phase, timing })
            })
            .collect();
        let elapsed = invocation.interval.end - invocation.interval.start;
        let all = timing(
            invocation
                .systems
                .iter()
                .map(|system| system.interval)
                .collect(),
        );
        reduced.push(ReducedInvocation {
            id: invocation.id,
            elapsed,
            phases,
            unattributed: timing(intervals_for(None)),
            unobserved: elapsed - all.map_or(Duration::ZERO, |timing| timing.observed),
        });
    }
    Ok(reduced)
}

/// Adapter to the existing harness-owned Recorder. Validation is atomic for a
/// batch: a later malformed invocation cannot leave earlier samples recorded.
/// Return coverage/envelope details for the producer's evidence report; they
/// cannot be reconstructed from the Recorder's aggregate percentiles.
pub fn sample_phase_timings(
    recorder: &mut Recorder,
    invocations: &[FixedInvocation],
) -> Result<Vec<ReducedInvocation>, ReductionError> {
    let reduced = reduce(invocations)?;
    for invocation in &reduced {
        let mut sample = |metric: &str, duration: Duration| {
            recorder.sample(metric, Unit::Millis, duration.as_secs_f64() * 1000.0);
        };
        sample(FIXED_ELAPSED, invocation.elapsed);
        sample(UNOBSERVED, invocation.unobserved);
        for phase in &invocation.phases {
            sample(observed_metric(phase.phase), phase.timing.observed);
        }
        if let Some(unattributed) = invocation.unattributed {
            sample(UNATTRIBUTED_OBSERVED, unattributed.observed);
        }
    }
    Ok(reduced)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(start: u64, end: u64) -> Interval {
        Interval {
            start: Duration::from_millis(start),
            end: Duration::from_millis(end),
        }
    }
    fn system(phase: Option<SimSet>, start: u64, end: u64) -> ObservedSystem {
        ObservedSystem {
            phase,
            interval: interval(start, end),
        }
    }
    fn invocation(systems: Vec<ObservedSystem>) -> FixedInvocation {
        FixedInvocation {
            id: 4,
            interval: interval(0, 20),
            systems,
        }
    }

    #[test]
    fn serial_phases_preserve_gaps_and_missing_is_not_zero() {
        let result = reduce(&[invocation(vec![
            system(Some(SimSet::Input), 1, 3),
            system(Some(SimSet::Input), 5, 7),
            system(Some(SimSet::Physics), 8, 11),
        ])])
        .unwrap()
        .remove(0);
        assert_eq!(result.elapsed, Duration::from_millis(20));
        assert_eq!(result.phases.len(), 2);
        assert_eq!(
            result.phases[0],
            PhaseTiming {
                phase: SimSet::Input,
                timing: Timing {
                    observed: Duration::from_millis(4),
                    envelope: Duration::from_millis(6),
                    intervals: 2
                }
            }
        );
        assert_eq!(result.phases[1].phase, SimSet::Physics);
        assert_eq!(result.unobserved, Duration::from_millis(13));
        assert_eq!(result.unattributed, None);
    }

    #[test]
    fn parallel_nested_duplicate_and_unsorted_intervals_count_time_once() {
        let input = invocation(vec![
            system(Some(SimSet::Input), 7, 12),
            system(Some(SimSet::Input), 2, 9),
            system(Some(SimSet::Input), 3, 4),
            system(Some(SimSet::Input), 2, 9),
        ]);
        let result = reduce(std::slice::from_ref(&input)).unwrap();
        let observed = result[0].phases[0].timing;
        assert_eq!(observed.observed, Duration::from_millis(10));
        assert_eq!(observed.envelope, Duration::from_millis(10));
        assert_eq!(observed.intervals, 4);
        let mut reversed = input;
        reversed.systems.reverse();
        assert_eq!(reduce(&[reversed]).unwrap(), result);
    }

    #[test]
    fn ambiguous_work_stays_unattributed_and_coverage_uses_a_global_union() {
        let result = reduce(&[invocation(vec![
            system(Some(SimSet::Input), 2, 8),
            system(None, 4, 10),
        ])])
        .unwrap()
        .remove(0);
        assert_eq!(
            result.unattributed.unwrap().observed,
            Duration::from_millis(6)
        );
        assert_eq!(result.unobserved, Duration::from_millis(12));
        assert_eq!(result.phases.len(), 1);
    }

    #[test]
    fn separate_fixed_invocations_produce_separate_samples_even_in_one_frame() {
        let mut second = invocation(vec![system(Some(SimSet::Input), 22, 26)]);
        second.id = 5;
        second.interval = interval(21, 41);
        let mut recorder = Recorder::new();
        sample_phase_timings(
            &mut recorder,
            &[invocation(vec![system(Some(SimSet::Input), 1, 3)]), second],
        )
        .unwrap();
        let capture = recorder.finish("fabricated", crate::perf::profile("fabricated"));
        let summary = &capture.summaries[observed_metric(SimSet::Input)].summary;
        assert_eq!(summary.count, 2);
        assert_eq!(summary.max, 4.0);
        assert!(!capture
            .summaries
            .contains_key(observed_metric(SimSet::Damage)));
        assert_eq!(capture.summaries[FIXED_ELAPSED].summary.count, 2);
    }

    #[test]
    fn no_capture_is_empty_but_an_observed_empty_invocation_keeps_its_coverage_gap() {
        let mut recorder = Recorder::new();
        assert!(sample_phase_timings(&mut recorder, &[]).unwrap().is_empty());
        assert!(recorder.is_empty());
        let result = sample_phase_timings(&mut recorder, &[invocation(vec![])]).unwrap();
        assert!(result[0].phases.is_empty());
        assert_eq!(result[0].unobserved, Duration::from_millis(20));
    }

    #[test]
    fn invalid_or_duplicate_invocations_never_partially_modify_the_recorder() {
        let valid = invocation(vec![system(Some(SimSet::Input), 1, 2)]);
        let mut bad = valid.clone();
        bad.id = 5;
        for malformed in [interval(3, 2), interval(0, 21)] {
            bad.systems[0].interval = malformed;
            let mut recorder = Recorder::new();
            assert_eq!(
                sample_phase_timings(&mut recorder, &[valid.clone(), bad.clone()]),
                Err(ReductionError::InvalidSystemInterval {
                    invocation: 5,
                    index: 0
                })
            );
            assert!(recorder.is_empty());
        }
        assert_eq!(
            reduce(&[valid.clone(), valid]),
            Err(ReductionError::DuplicateInvocation(4))
        );
        bad.interval = interval(20, 1);
        assert_eq!(reduce(&[bad]), Err(ReductionError::ReversedInvocation(5)));
    }
}
