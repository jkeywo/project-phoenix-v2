use super::*;

#[test]
fn a_new_window_is_empty_and_not_full() {
    let w = BoundedHistory::new(3);
    assert!(w.is_empty());
    assert!(!w.is_full());
    assert_eq!(w.len(), 0);
    assert_eq!(w.capacity(), 3);
    assert_eq!(w.min(), None);
    assert_eq!(w.max(), None);
    assert_eq!(w.last(), None);
}

#[test]
fn it_fills_to_capacity_and_then_stops_growing() {
    let mut w = BoundedHistory::new(3);
    for n in 0..100 {
        w.push(n as f64);
        assert!(w.len() <= 3, "the window must never exceed its capacity");
    }
    assert!(w.is_full());
    assert_eq!(w.len(), 3);
    // The retained samples are the LAST three, oldest first.
    assert_eq!(w.iter().collect::<Vec<_>>(), vec![97.0, 98.0, 99.0]);
    assert_eq!(w.last(), Some(99.0));
}

#[test]
fn the_oldest_sample_is_the_one_evicted() {
    let mut w = BoundedHistory::new(2);
    w.push(1.0);
    w.push(2.0);
    assert_eq!(w.min(), Some(1.0));
    w.push(5.0);
    assert_eq!(
        w.min(),
        Some(2.0),
        "1.0 aged out of the window, so it must stop counting"
    );
    assert_eq!(w.max(), Some(5.0));
}

/// The property a running aggregate cannot provide: one bad sample stops
/// mattering once it ages out. A running minimum would be stuck at 1.0.
#[test]
fn a_stale_bad_sample_ages_out_instead_of_poisoning_the_window() {
    let mut w = BoundedHistory::new(3);
    w.push(1.0);
    w.push(10.0);
    w.push(10.0);
    assert!(
        !w.all_at_least(5.0),
        "the bad sample is still in the window"
    );
    w.push(10.0);
    assert!(
        w.all_at_least(5.0),
        "it has aged out; the window is now clean"
    );
}

#[test]
fn all_at_least_is_false_until_the_window_is_full() {
    let mut w = BoundedHistory::new(3);
    w.push(100.0);
    assert!(
        !w.all_at_least(5.0),
        "one good sample is not a maintained distance"
    );
    w.push(100.0);
    assert!(!w.all_at_least(5.0));
    w.push(100.0);
    assert!(w.all_at_least(5.0));
}

#[test]
fn all_at_least_is_inclusive_at_the_threshold() {
    let mut w = BoundedHistory::new(2);
    w.push(5.0);
    w.push(5.0);
    assert!(w.all_at_least(5.0));
    assert!(!w.all_at_least(5.000_001));
}

#[test]
fn a_zero_capacity_window_retains_nothing_and_never_answers_held() {
    let mut w = BoundedHistory::new(0);
    w.push(100.0);
    w.push(100.0);
    assert!(w.is_empty());
    assert!(!w.is_full());
    assert!(!w.all_at_least(0.0));
}

#[test]
fn clearing_drops_the_samples_but_keeps_the_capacity() {
    let mut w = BoundedHistory::new(2);
    w.push(1.0);
    w.push(2.0);
    assert!(w.is_full());
    w.clear();
    assert!(w.is_empty());
    assert_eq!(w.capacity(), 2);
    assert!(!w.all_at_least(0.0), "a cleared window is not a full one");
}

#[test]
fn shrinking_the_capacity_discards_the_oldest_samples() {
    let mut w = BoundedHistory::new(4);
    for n in 1..=4 {
        w.push(n as f64);
    }
    w.set_capacity(2);
    assert_eq!(w.iter().collect::<Vec<_>>(), vec![3.0, 4.0]);
    assert!(w.is_full());
}

#[test]
fn growing_the_capacity_keeps_what_is_there_but_un_fulls_the_window() {
    let mut w = BoundedHistory::new(2);
    w.push(1.0);
    w.push(2.0);
    assert!(w.is_full());
    w.set_capacity(4);
    assert_eq!(w.len(), 2);
    assert!(
        !w.is_full(),
        "a grown window needs new samples before it is full again"
    );
    assert_eq!(w.iter().collect::<Vec<_>>(), vec![1.0, 2.0]);
}

#[test]
fn re_authoring_the_same_capacity_does_not_reset_the_window() {
    let mut w = BoundedHistory::new(3);
    for n in 0..3 {
        w.push(n as f64);
    }
    for _ in 0..10 {
        w.set_capacity(3);
    }
    assert!(
        w.is_full(),
        "an idempotent re-author must not clear the history"
    );
    assert_eq!(w.len(), 3);
}

#[test]
fn net_change_is_the_newest_sample_minus_the_oldest() {
    let mut w = BoundedHistory::new(3);
    w.push(10.0);
    w.push(20.0);
    w.push(40.0);
    assert_eq!(w.net_change(), Some(30.0));
    // A reading that goes the OTHER way is a negative net change, not an
    // absolute distance: the sign is the whole point of a trend.
    w.push(5.0);
    assert_eq!(w.net_change(), Some(-15.0), "20.0 is now the oldest sample");
}

#[test]
fn net_change_is_none_until_the_window_is_full() {
    let mut w = BoundedHistory::new(3);
    w.push(0.0);
    assert_eq!(
        w.net_change(),
        None,
        "a one-sample window has no span to measure across"
    );
    w.push(100.0);
    assert_eq!(
        w.net_change(),
        None,
        "two samples is still a SHORTER span than the authored three: answering \
             here would report progress over a window nobody authored"
    );
    w.push(200.0);
    assert_eq!(w.net_change(), Some(200.0));
}

/// The same property `all_at_least` has: a cleared window is not a full one,
/// so the trend goes unavailable until it has been re-earned.
#[test]
fn clearing_makes_the_net_change_unavailable_again() {
    let mut w = BoundedHistory::new(2);
    w.push(1.0);
    w.push(9.0);
    assert_eq!(w.net_change(), Some(8.0));
    w.clear();
    assert_eq!(w.net_change(), None);
}

#[test]
fn a_zero_capacity_window_never_reports_a_net_change() {
    let mut w = BoundedHistory::new(0);
    w.push(1.0);
    w.push(2.0);
    assert_eq!(w.net_change(), None);
}

#[test]
fn a_flat_window_reports_no_progress_rather_than_nothing() {
    let mut w = BoundedHistory::new(3);
    for _ in 0..3 {
        w.push(50.0);
    }
    assert_eq!(
        w.net_change(),
        Some(0.0),
        "a reading that has not moved is a MEASURED zero, not an absent answer"
    );
}

#[test]
fn setting_the_capacity_to_zero_empties_the_window() {
    let mut w = BoundedHistory::new(3);
    w.push(1.0);
    w.set_capacity(0);
    assert!(w.is_empty());
    assert!(!w.is_full());
}

// ── BoundedRing<T> (issue #1151) ──────────────────────────────────────────
//
// The generic ring the trigger-fire recorder keeps per trigger. The same
// bound-is-the-point properties `BoundedHistory` has, over an arbitrary
// record type (here `&'static str`, standing in for a fire record).

#[test]
fn a_ring_fills_to_capacity_then_evicts_the_oldest() {
    let mut r: BoundedRing<&str> = BoundedRing::new(2);
    assert!(r.is_empty());
    r.push("a");
    r.push("b");
    assert!(r.is_full());
    assert_eq!(r.len(), 2);
    r.push("c");
    assert_eq!(r.len(), 2, "the ring must never exceed its capacity");
    assert_eq!(
        r.iter().copied().collect::<Vec<_>>(),
        vec!["b", "c"],
        "the oldest record aged out; the last `capacity` remain, oldest first"
    );
    assert_eq!(r.last(), Some(&"c"));
}

#[test]
fn a_zero_capacity_ring_retains_nothing() {
    let mut r: BoundedRing<&str> = BoundedRing::new(0);
    r.push("a");
    r.push("b");
    assert!(r.is_empty());
    assert!(!r.is_full());
    assert_eq!(r.last(), None);
}

#[test]
fn shrinking_a_ring_discards_the_oldest_records() {
    let mut r: BoundedRing<i32> = BoundedRing::new(4);
    for n in 1..=4 {
        r.push(n);
    }
    r.set_capacity(2);
    assert_eq!(r.iter().copied().collect::<Vec<_>>(), vec![3, 4]);
    assert!(r.is_full());
}

#[test]
fn re_authoring_the_same_ring_capacity_does_not_reset_it() {
    let mut r: BoundedRing<i32> = BoundedRing::new(3);
    for n in 0..3 {
        r.push(n);
    }
    for _ in 0..10 {
        r.set_capacity(3);
    }
    assert_eq!(
        r.len(),
        3,
        "an idempotent re-author must not clear the ring"
    );
}

#[test]
fn clearing_a_ring_drops_records_but_keeps_capacity() {
    let mut r: BoundedRing<i32> = BoundedRing::new(2);
    r.push(1);
    r.push(2);
    r.clear();
    assert!(r.is_empty());
    assert_eq!(r.capacity(), 2);
}

#[test]
fn a_default_ring_retains_nothing_until_a_capacity_is_authored() {
    // The recorder builds records with an explicit capacity; a `default()`
    // ring is the degenerate zero-length one, matching `BoundedHistory`.
    let mut r: BoundedRing<i32> = BoundedRing::default();
    assert_eq!(r.capacity(), 0);
    r.push(1);
    assert!(r.is_empty());
    r.set_capacity(2);
    r.push(1);
    r.push(2);
    assert_eq!(r.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
}
