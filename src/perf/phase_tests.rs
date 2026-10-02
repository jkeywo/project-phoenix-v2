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
