use super::*;
fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}
fn buffer() -> Buffer {
    let mut buffer = Buffer::default();
    buffer.kinds.insert(1, Kind::Schedule("FixedUpdate".into()));
    buffer.kinds.insert(2, Kind::System("outer".into()));
    buffer.kinds.insert(3, Kind::System("inner".into()));
    buffer.kinds.insert(4, Kind::Auxiliary);
    buffer
}
#[test]
fn duplicate_or_composed_names_are_not_guessed_into_a_phase() {
    let snapshot = ScheduleObservation {
        systems: vec![
            ("one".into(), vec!["Input".into()]),
            ("duplicate".into(), vec!["Input".into()]),
            ("duplicate".into(), vec!["Physics".into()]),
            ("same-phase".into(), vec!["Input".into()]),
            ("same-phase".into(), vec!["Input".into()]),
            ("multi-set".into(), vec!["Input".into(), "Physics".into()]),
            ("unowned".into(), vec![]),
        ],
        ambiguities: vec![],
    };
    let map = attribution(&snapshot);
    assert_eq!(map["one"], Some(SimSet::Input));
    for name in ["duplicate", "same-phase", "multi-set", "unowned"] {
        assert_eq!(map[name], None);
    }
    assert!(!map.contains_key("composed-child-not-in-schedule"));
}
#[test]
fn paired_worker_and_nested_spans_retain_real_invocation_boundaries() {
    let main = std::thread::current().id();
    let worker = std::thread::spawn(|| std::thread::current().id())
        .join()
        .unwrap();
    let mut buffer = buffer();
    buffer.enter(1, main, at(0));
    buffer.enter(2, worker, at(1));
    buffer.enter(3, worker, at(2));
    buffer.exit(3, worker, at(3));
    buffer.exit(2, worker, at(4));
    buffer.enter(4, main, at(5));
    buffer.enter(3, main, at(6));
    buffer.exit(3, main, at(7));
    buffer.exit(4, main, at(8));
    buffer.exit(1, main, at(10));
    assert!(buffer.error.is_none());
    let result = &buffer.completed[0];
    assert_eq!(
        result.interval,
        Interval {
            start: at(0),
            end: at(10)
        }
    );
    assert_eq!(result.systems.len(), 3);
    assert!(result.systems[0].unattributed);
    assert!(!result.systems[1].unattributed);
    assert!(result.systems[2].unattributed);
}
#[test]
fn the_outer_fixed_main_system_is_not_a_nested_fixed_update_system() {
    let thread = std::thread::current().id();
    let mut buffer = buffer();
    buffer.register(5, Kind::System("fixed-main-runner".into()));
    buffer.enter(5, thread, at(0));
    buffer.enter(1, thread, at(1));
    buffer.enter(2, thread, at(2));
    buffer.exit(2, thread, at(3));
    buffer.exit(1, thread, at(4));
    buffer.exit(5, thread, at(5));
    assert!(buffer.error.is_none());
    assert!(!buffer.completed[0].systems[0].unattributed);
}

#[test]
fn a_temporary_duplicate_span_cannot_be_attributed_after_it_closes() {
    let thread = std::thread::current().id();
    let mut buffer = buffer();
    buffer.register(6, Kind::System("duplicate".into()));
    buffer.register(7, Kind::System("duplicate".into()));
    buffer.enter(1, thread, at(0));
    buffer.enter(7, thread, at(1));
    buffer.exit(7, thread, at(2));
    buffer.close(7);
    buffer.exit(1, thread, at(3));
    assert!(buffer.completed[0].systems[0].ambiguous);
    assert_eq!(buffer.named_spans["duplicate"], 1);
}

#[test]
fn unbalanced_spans_fail_instead_of_inventing_complete_samples() {
    let thread = std::thread::current().id();
    let mut buffer = buffer();
    buffer.enter(1, thread, at(0));
    buffer.enter(2, thread, at(1));
    buffer.exit(1, thread, at(2));
    assert!(buffer.error.is_some());
    assert!(buffer.completed.is_empty());
}
