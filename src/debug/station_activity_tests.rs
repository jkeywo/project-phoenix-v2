use super::*;

fn station(id: &str) -> StationId {
    StationId(id.into())
}

/// A command admitted at tick T lands in bucket `T / bucket_ticks`, and the
/// bucket's `start_tick` is that ordinal times the length.
#[test]
fn commands_land_in_the_bucket_their_tick_falls_in() {
    let mut tracker = StationActivityTracker::new(10, DEFAULT_MAX_BUCKETS);
    tracker.record(3, &station("helm"), ActivitySource::Human);
    tracker.record(9, &station("helm"), ActivitySource::Human);
    // Same bucket [0, 10): both counted together.
    let payload = tracker.report();
    assert_eq!(payload.buckets.len(), 1);
    assert_eq!(payload.buckets[0].start_tick, 0);
    assert_eq!(payload.buckets[0].stations[0].human, 2);
}

/// Crossing a bucket boundary finalises the old bucket and opens a new one,
/// each carrying its own start tick.
#[test]
fn crossing_a_boundary_opens_a_new_bucket() {
    let mut tracker = StationActivityTracker::new(10, DEFAULT_MAX_BUCKETS);
    tracker.record(5, &station("helm"), ActivitySource::Human); // bucket 0
    tracker.record(15, &station("helm"), ActivitySource::Human); // bucket 1
    let payload = tracker.report();
    assert_eq!(payload.buckets.len(), 2, "one bucket per crossed boundary");
    assert_eq!(payload.buckets[0].start_tick, 0);
    assert_eq!(payload.buckets[1].start_tick, 10);
    assert_eq!(payload.buckets[0].stations[0].human, 1);
    assert_eq!(payload.buckets[1].stations[0].human, 1);
}

/// A quiet stretch shows as empty buckets rather than collapsing the axis.
#[test]
fn a_quiet_gap_fills_empty_buckets() {
    let mut tracker = StationActivityTracker::new(10, DEFAULT_MAX_BUCKETS);
    tracker.record(5, &station("helm"), ActivitySource::Human); // bucket 0
    tracker.begin_tick(35); // jump to bucket 3, no command
    let payload = tracker.report();
    // Buckets 0 (one command), 1 (empty), 2 (empty), 3 (current, empty).
    assert_eq!(payload.buckets.len(), 4);
    assert_eq!(payload.buckets[0].start_tick, 0);
    assert_eq!(payload.buckets[1].start_tick, 10);
    assert_eq!(payload.buckets[1].stations.len(), 0, "gap bucket is empty");
    assert_eq!(payload.buckets[3].start_tick, 30);
}

/// The configured bucket size changes which tick opens a new bucket.
#[test]
fn configurable_bucket_size_changes_boundaries() {
    // 2 s buckets at 30 Hz = 60 ticks each.
    let mut tracker = StationActivityTracker::default();
    tracker.configure(2.0, 30.0);
    assert_eq!(tracker.bucket_ticks(), 60);

    tracker.record(59, &station("helm"), ActivitySource::Human); // bucket 0
    tracker.record(60, &station("helm"), ActivitySource::Human); // bucket 1
    let payload = tracker.report();
    assert_eq!(payload.bucket_ticks, 60);
    assert_eq!(payload.bucket_secs, 2.0);
    assert_eq!(payload.buckets.len(), 2);
}

/// The whole point: commands split by control source, per station.
#[test]
fn counts_split_by_station_and_source() {
    let mut tracker = StationActivityTracker::new(100, DEFAULT_MAX_BUCKETS);
    tracker.record(1, &station("helm"), ActivitySource::Human);
    tracker.record(1, &station("helm"), ActivitySource::Human);
    tracker.record(1, &station("helm"), ActivitySource::Ai);
    tracker.record(1, &station("weapons"), ActivitySource::Ai);

    let payload = tracker.report();
    assert_eq!(payload.buckets.len(), 1);
    let stations = &payload.buckets[0].stations;
    assert_eq!(stations.len(), 2, "two distinct stations");
    // Sorted by station id: "helm" before "weapons".
    assert_eq!(stations[0].station, "helm");
    assert_eq!(stations[0].human, 2);
    assert_eq!(stations[0].ai, 1);
    assert_eq!(stations[1].station, "weapons");
    assert_eq!(stations[1].ai, 1);
    assert_eq!(stations[1].human, 0);
}

/// The retention window bounds how many completed buckets are kept.
#[test]
fn completed_buckets_are_bounded_by_the_window() {
    let mut tracker = StationActivityTracker::new(1, 4);
    for tick in 0..20u64 {
        tracker.record(tick, &station("helm"), ActivitySource::Human);
    }
    let payload = tracker.report();
    // 4 completed + 1 current = at most 5.
    assert!(
        payload.buckets.len() <= 5,
        "window must bound retained buckets, got {}",
        payload.buckets.len()
    );
    // The series ends at the most recent tick.
    assert_eq!(payload.buckets.last().unwrap().start_tick, 19);
}

/// Re-authoring the bucket size to the same value is a no-op that never
/// resets the running series.
#[test]
fn reconfiguring_to_the_same_size_preserves_the_series() {
    let mut tracker = StationActivityTracker::default();
    tracker.configure(15.0, 60.0);
    tracker.record(10, &station("helm"), ActivitySource::Human);
    tracker.configure(15.0, 60.0); // same → no-op
    tracker.record(20, &station("helm"), ActivitySource::Human);
    let payload = tracker.report();
    assert_eq!(payload.buckets[0].stations[0].human, 2, "series survived");
}
