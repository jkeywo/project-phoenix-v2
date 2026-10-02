use super::*;

const PHONE: LatencySurface = LatencySurface::PhoneConsole;

fn client_sample(action: &str, send: f32, ack: f32) -> ConsoleLatencySample {
    ConsoleLatencySample {
        action: action.into(),
        input_to_send_ms: send,
        send_to_ack_ms: ack,
    }
}

fn expiry(action: &str, count: u32) -> ConsoleLatencyExpiry {
    ConsoleLatencyExpiry {
        action: action.into(),
        count,
    }
}

/// Nearest-rank, matching `vellum-perf`: with four samples the median is the
/// 2nd and p75 the 3rd.
#[test]
fn percentiles_are_nearest_rank() {
    let sorted = [10.0f32, 20.0, 30.0, 40.0];
    assert_eq!(percentile(&sorted, 0.50), 20.0);
    assert_eq!(percentile(&sorted, 0.75), 30.0);
    assert_eq!(percentile(&sorted, 1.0), 40.0);
}

#[test]
fn a_single_sample_is_its_own_p50_p75_and_max() {
    let mut tracker = ConsoleLatencyTracker::default();
    tracker.record_client(PHONE, &client_sample("fire_phaser", 4.0, 60.0));
    let payload = tracker.report();
    assert_eq!(payload.actions.len(), 1);
    let entry = &payload.actions[0];
    let end_to_end = entry.input_to_ack.clone().expect("client surface has it");
    assert_eq!(end_to_end.p50_ms, 64.0);
    assert_eq!(end_to_end.p75_ms, 64.0);
    assert_eq!(end_to_end.max_ms, 64.0);
}

/// The end-to-end figure is summarised from per-sample sums, so it is not
/// the sum of the two segment summaries when the segments peak on different
/// samples.
#[test]
fn end_to_end_summarises_sums_rather_than_summing_summaries() {
    let mut tracker = ConsoleLatencyTracker::default();
    // Sample A: slow send, fast round trip. Sample B: the reverse.
    tracker.record_client(PHONE, &client_sample("set_impulse", 30.0, 10.0));
    tracker.record_client(PHONE, &client_sample("set_impulse", 1.0, 50.0));
    let entry = &tracker.report().actions[0];
    let end_to_end = entry.input_to_ack.clone().expect("client surface has it");
    // Sums are 40 and 51; the max of the sums is 51, NOT max(30) + max(50).
    assert_eq!(end_to_end.max_ms, 51.0);
}

/// The host's own series is not reachable from the client fold at all: the
/// surface is a parameter the drain supplies, and it never supplies
/// `SimHost`. This pins the guard that would otherwise be the only thing
/// between a peer and the series a CI budget compares.
#[test]
fn a_client_fold_can_never_write_the_sim_host_surface() {
    let mut tracker = ConsoleLatencyTracker::default();
    tracker.record_client(
        LatencySurface::SimHost,
        &client_sample("FirePhaser", 0.0, 0.0),
    );
    tracker.record_client_expiry(LatencySurface::SimHost, &expiry("FirePhaser", 5));
    assert!(tracker.is_empty(), "SimHost samples are the host's alone");
}

/// Garbage in is dropped, not folded: a negative or non-finite duration is a
/// broken measurement and would drag every percentile with it.
#[test]
fn non_finite_and_negative_durations_are_refused() {
    let mut tracker = ConsoleLatencyTracker::default();
    for (send, ack) in [(-1.0, 5.0), (5.0, f32::NAN), (f32::INFINITY, 5.0)] {
        tracker.record_client(PHONE, &client_sample("x", send, ack));
    }
    assert!(tracker.is_empty());
}

#[test]
fn the_window_bounds_retained_samples() {
    let mut tracker = ConsoleLatencyTracker::new(4);
    for i in 0..20 {
        tracker.record_host("FirePhaser", i as f32);
    }
    let entry = &tracker.report().actions[0];
    let summary = entry
        .admit_to_broadcast
        .clone()
        .expect("host surface has it");
    assert_eq!(summary.count, 4, "only the window is retained");
    assert_eq!(summary.max_ms, 19.0, "and it is the RECENT window");
}

/// The action map is bounded, because a client chooses the label.
#[test]
fn distinct_action_labels_are_capped() {
    let mut tracker = ConsoleLatencyTracker::default();
    for i in 0..(MAX_TRACKED_ACTIONS + 50) {
        tracker.record_client(PHONE, &client_sample(&format!("action_{i}"), 1.0, 1.0));
    }
    assert_eq!(tracker.report().actions.len(), MAX_TRACKED_ACTIONS);
}

/// The cap is PER SURFACE (issue #1169 review, C3). A client that fills its
/// own budget with junk must not be able to starve the host's own rows —
/// the series a CI budget compares — which a single global cap allowed.
#[test]
fn a_flooded_client_surface_cannot_starve_the_host_surface() {
    let mut tracker = ConsoleLatencyTracker::default();
    for i in 0..(MAX_TRACKED_ACTIONS * 2) {
        tracker.record_client(PHONE, &client_sample(&format!("junk_{i}"), 1.0, 1.0));
    }
    // The host's own tap still opens its series afterwards.
    tracker.record_host("FirePhaser", 3.0);
    let payload = tracker.report();
    assert!(
        payload
            .actions
            .iter()
            .any(|e| e.surface == LatencySurface::SimHost && e.action == "FirePhaser"),
        "a flooded phone surface must not consume the host's budget"
    );
    assert_eq!(
        payload
            .actions
            .iter()
            .filter(|e| e.surface == PHONE)
            .count(),
        MAX_TRACKED_ACTIONS,
        "the client surface is still bounded by its own cap"
    );
}

/// Switching measurement on empties the tracker: it matches the client
/// meters' own reset, and it is the recovery path for a surface whose budget
/// a misbehaving client filled.
#[test]
fn clearing_frees_a_flooded_surface() {
    let mut tracker = ConsoleLatencyTracker::default();
    for i in 0..(MAX_TRACKED_ACTIONS * 2) {
        tracker.record_client(PHONE, &client_sample(&format!("junk_{i}"), 1.0, 1.0));
    }
    tracker.clear();
    assert!(tracker.is_empty());
    tracker.record_client(PHONE, &client_sample("fire_phaser", 1.0, 2.0));
    assert_eq!(tracker.report().actions.len(), 1);
}

#[test]
fn a_long_action_label_is_trimmed_on_a_character_boundary() {
    let long = "e\u{0301}".repeat(200);
    let trimmed = label_of(&long);
    assert!(trimmed.len() <= MAX_ACTION_LABEL + 2, "trimmed to the cap");
    assert!(
        trimmed.chars().all(|c| c == 'e' || c == '\u{0301}'),
        "no split character"
    );
}

/// A host entry carries only the host segment, and a client entry only the
/// client segments — the payload must never invent the other side.
#[test]
fn each_surface_carries_only_the_segments_it_can_observe() {
    let mut tracker = ConsoleLatencyTracker::default();
    tracker.record_host("FirePhaser", 3.0);
    tracker.record_client(PHONE, &client_sample("fire_phaser", 1.0, 2.0));
    let payload = tracker.report();

    let host = payload
        .actions
        .iter()
        .find(|e| e.surface == LatencySurface::SimHost)
        .expect("host entry");
    assert!(host.admit_to_broadcast.is_some());
    assert!(host.input_to_send.is_none());
    assert!(host.send_to_ack.is_none());
    assert!(host.input_to_ack.is_none());
    assert_eq!(host.expired, 0, "the host's window always closes");

    let phone = payload
        .actions
        .iter()
        .find(|e| e.surface == PHONE)
        .expect("phone entry");
    assert!(phone.admit_to_broadcast.is_none());
    assert!(phone.input_to_send.is_some());
}

/// An outage is COUNTED, not swallowed (issue #1169 review, C1). Without
/// this, an action whose surface never answers produces no record at all and
/// a dead link reads as a quiet one.
#[test]
fn unanswered_actions_are_counted_beside_the_distributions() {
    let mut tracker = ConsoleLatencyTracker::default();
    tracker.record_client(PHONE, &client_sample("fire_phaser", 1.0, 40.0));
    tracker.record_client_expiry(PHONE, &expiry("fire_phaser", 3));
    tracker.record_client_expiry(PHONE, &expiry("fire_phaser", 2));

    let entry = &tracker.report().actions[0];
    assert_eq!(entry.expired, 5, "expiries accumulate across reports");
    assert!(
        entry.send_to_ack.is_some(),
        "the distribution still describes the actions that DID get through"
    );
}

/// An action that has never once been answered exists only as an outage
/// count — which is exactly the case worth surfacing.
#[test]
fn an_action_that_never_answers_still_appears() {
    let mut tracker = ConsoleLatencyTracker::default();
    tracker.record_client_expiry(PHONE, &expiry("set_impulse", 7));
    let entry = &tracker.report().actions[0];
    assert_eq!(entry.action, "set_impulse");
    assert_eq!(entry.expired, 7);
    assert_eq!(entry.count, 0);
    assert!(entry.send_to_ack.is_none(), "nothing was ever measured");
}

#[test]
fn host_samples_yield_every_retained_raw_value() {
    let mut tracker = ConsoleLatencyTracker::default();
    tracker.record_host("FirePhaser", 1.0);
    tracker.record_host("FirePhaser", 2.0);
    tracker.record_host("SetThrottle", 5.0);
    let mut got: Vec<(String, f32)> = tracker
        .host_samples()
        .map(|(a, ms)| (a.to_string(), ms))
        .collect();
    got.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    assert_eq!(
        got,
        vec![
            ("FirePhaser".to_string(), 1.0),
            ("FirePhaser".to_string(), 2.0),
            ("SetThrottle".to_string(), 5.0),
        ]
    );
}

/// The publish throttle is derived from the authored tick rate and is always
/// a whole number of ticks — the projection must not run every tick inside
/// the window `sim.tick` measures (issue #1169 review, C4).
#[test]
fn the_publish_interval_is_whole_ticks_at_the_authored_rate() {
    assert_eq!(publish_interval_ticks(60.0), 15, "60 Hz / 4 Hz");
    assert_eq!(publish_interval_ticks(30.0), 8, "30 Hz / 4 Hz, rounded");
    assert_eq!(publish_interval_ticks(4.0), 1, "never below one tick");
    assert_eq!(publish_interval_ticks(1.0), 1);
    assert_eq!(
        publish_interval_ticks(f32::NAN),
        1,
        "degenerate input is safe"
    );
}
