use super::*;
use crate::core::messages::LatencySurface;
use crate::perf::profile;

#[test]
fn every_retained_host_sample_reaches_the_recorder() {
    let mut tracker = ConsoleLatencyTracker::default();
    tracker.record_host("FirePhaser", 1.0);
    tracker.record_host("FirePhaser", 3.0);
    tracker.record_host("SetThrottle", 2.0);

    let mut recorder = Recorder::new();
    sample_console_latency(&mut recorder, &tracker);
    let capture = recorder.finish("test-scenario", profile("test"));

    let summary = &capture.summaries[CONSOLE_ACK_METRIC].summary;
    assert_eq!(summary.count, 3, "one perf sample per retained sample");
    assert_eq!(summary.max, 3.0);
}

/// A run that measured nothing must contribute NO metric — a zeroed one
/// would read as a budget that passed.
#[test]
fn an_unmeasured_run_contributes_no_metric() {
    let mut recorder = Recorder::new();
    sample_console_latency(&mut recorder, &ConsoleLatencyTracker::default());
    assert!(recorder.is_empty());
}

/// Client-reported samples must never reach the budget: they measure a
/// player's network, which no checkout controls — and since the #1169 review
/// they are additionally a *perceived-feedback proxy* bounded below by the
/// host's broadcast cadence, which would make them meaningless as a
/// processing budget even over a perfect link.
#[test]
fn client_measured_samples_stay_out_of_the_budget() {
    let mut tracker = ConsoleLatencyTracker::default();
    tracker.record_client(
        LatencySurface::PhoneConsole,
        &crate::core::messages::ConsoleLatencySample {
            action: "fire_phaser".into(),
            input_to_send_ms: 5.0,
            send_to_ack_ms: 500.0,
        },
    );

    let mut recorder = Recorder::new();
    sample_console_latency(&mut recorder, &tracker);
    assert!(
        recorder.is_empty(),
        "a phone's round trip is not a budget this repository can be held to"
    );
}
