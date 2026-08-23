//! Console input-to-feedback latency (issue #1169, PRD #1144).
//!
//! # What this measures, and what nothing can
//!
//! "Tap to response" is not one number and it is not measurable from one place.
//! A console action crosses four boundaries before the player sees an answer:
//!
//! ```text
//!   input event ──▶ transport hand-off ──▶ host admission ──▶ broadcast ──▶ issuing surface refreshed ──▶ pixels
//!               ①                      ②                  ③             ④                            (unobservable)
//! ```
//!
//! ② and ④ cross a device boundary. A phone's clock and the host's clock have
//! no defined relationship, so subtracting one device's stamp from the other's
//! measures their disagreement as much as the latency. Everything here is
//! therefore built out of **single-clock** measurements — the same device stamps
//! both ends of every number — and the segments are reported separately rather
//! than glued into a fabricated total:
//!
//! | segment | clock | paths it exists on |
//! |---|---|---|
//! | `input_to_send` (①) | the client's | browser host, phone |
//! | `send_to_ack` (②+③+④) | the client's | browser host, phone |
//! | `input_to_ack` (sum) | the client's | browser host, phone |
//! | `admit_to_broadcast` (③) | the host's | browser host, native, headless |
//!
//! The client half is measured in `gui/console-latency.js` and arrives here as
//! already-differenced durations via [`ClientMessage::ReportConsoleLatency`].
//! This module owns the host half and the folding of both into one payload.
//!
//! **Pixels are deliberately not claimed.** `send_to_ack` ends when the issuing
//! console's surface is handed fresh server-derived state, which is the last
//! moment any of this code can observe. The compositor is past that line, so no
//! number here pretends to include it.
//!
//! # The host segment: `admit_to_broadcast`
//!
//! Wall time from the tick's command admission ([`AdmissionSet`]) to the end of
//! the tick's `SimSet::Broadcast` — the simulation's own service window,
//! attributed to every action admitted in that tick. It is the slice of the
//! client's `send_to_ack` the host is answerable for; the remainder is transport
//! and client work, which this side cannot see.
//!
//! It is a WINDOW, not a per-command queue time, and the doc comment on
//! [`record_admit_to_broadcast`] says exactly what that costs. It is also the
//! only segment a headless run has — there is no client — which is precisely
//! why it is the one wired to the #868 perf budget (`crate::perf::console`):
//! it is the segment a CI machine can reproduce.
//!
//! # Determinism
//!
//! `Instant::now()` is non-deterministic by construction, so nothing it produces
//! may enter authoritative state, the #894 digest, or anything a replay
//! re-derives. Three things keep that true:
//!
//! 1. Every resource here is declared `StateClass::Presentation` at
//!    `DebugPlugin::build`, so `world_digest` never folds it.
//! 2. Nothing in the fixed tick reads these resources back. They are written and
//!    then only projected.
//! 3. The clock is not even READ unless [`DebugConsoleLatencyEnabled`] is on —
//!    unlike the always-on station-activity counters beside it. That is the PRD
//!    requirement that measurement off the flag costs nothing, and it makes the
//!    determinism A/B in `tests/console_latency.rs` a real experiment: flag off
//!    takes no stamps at all, and the seeded digest is byte-identical either way.

use bevy::platform::time::Instant;
use bevy::prelude::*;
use std::collections::BTreeMap;
use std::collections::VecDeque;

use crate::core::messages::{
    ConsoleLatencySample, LatencySurface, SystemControlPayloadDiscriminants,
};
use crate::debug::payload::{
    ActionLatencyEntry, ConsoleLatencyPayload, LatencySummary, DEBUG_SCHEMA_VERSION,
};

/// How many samples one (surface, action) series retains.
///
/// A rolling window, not a whole-run history: the surface answers "how does this
/// feel *now*", and a run that has been going for an hour should not have its
/// current tail hidden under a warm-up spike. Presentation-only — nothing in the
/// fixed tick reads it.
pub const DEFAULT_WINDOW: usize = 256;

/// How many distinct (surface, action) series the tracker will open.
///
/// A bound on untrusted input as much as on memory: the action label for a
/// client surface is a STRING chosen by the client, so an unbounded map would be
/// a client-controlled allocation. Once the cap is reached, samples for unseen
/// actions are dropped rather than evicting a live series.
pub const MAX_TRACKED_ACTIONS: usize = 96;

/// Longest action label accepted from a client. Every real one is far shorter
/// (`set_navigation_waypoint` is 24 characters); this only stops a pathological
/// sender from filing a novel under an action name.
const MAX_ACTION_LABEL: usize = 48;

/// One segment's bounded sample window, oldest first.
#[derive(Clone, Debug, Default)]
struct Series {
    samples: VecDeque<f32>,
}

impl Series {
    fn push(&mut self, value: f32, window: usize) {
        self.samples.push_back(value);
        while self.samples.len() > window {
            self.samples.pop_front();
        }
    }

    fn len(&self) -> usize {
        self.samples.len()
    }

    /// The distribution, or `None` when this segment took no samples — which is
    /// how the payload distinguishes "not measured on this path" from "0 ms".
    fn summary(&self) -> Option<LatencySummary> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<f32> = self.samples.iter().copied().collect();
        // `total_cmp` rather than `partial_cmp().unwrap()`: every value is
        // already screened finite by `sanitise_ms`, but a total order costs
        // nothing and cannot panic if that screen is ever loosened.
        sorted.sort_by(|a, b| a.total_cmp(b));
        Some(LatencySummary {
            count: sorted.len() as u32,
            p50_ms: percentile(&sorted, 0.50),
            p75_ms: percentile(&sorted, 0.75),
            max_ms: *sorted.last().expect("non-empty by the guard above"),
        })
    }
}

/// Nearest-rank percentile over an ascending slice, `p` in `0.0..=1.0`.
///
/// The same definition `vellum-perf::summarize` uses, so a p50 in the debug
/// payload and a p50 in a perf capture of the same run are the same statistic
/// rather than two conventions that happen to share a name.
fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p * sorted.len() as f32).ceil().max(1.0) as usize;
    sorted[(rank - 1).min(sorted.len() - 1)]
}

/// The four segments one (surface, action) pair can carry.
#[derive(Clone, Debug, Default)]
struct ActionSeries {
    input_to_send: Series,
    send_to_ack: Series,
    input_to_ack: Series,
    admit_to_broadcast: Series,
}

/// The bounded per-(surface, action) latency windows (issue #1169).
///
/// Pure and Bevy-agnostic apart from the `Resource` derive, like
/// [`crate::debug::station_activity::StationActivityTracker`]: [`Self::record_client`],
/// [`Self::record_host`] and [`Self::report`] are the whole interface and are
/// unit-testable without an `App`. `world_digest` never folds this — it is
/// declared `StateClass::Presentation` at `DebugPlugin::build`.
#[derive(Resource, Clone, Debug)]
pub struct ConsoleLatencyTracker {
    window: usize,
    series: BTreeMap<(LatencySurface, String), ActionSeries>,
}

impl Default for ConsoleLatencyTracker {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW)
    }
}

impl ConsoleLatencyTracker {
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            series: BTreeMap::new(),
        }
    }

    /// Whether any sample has been taken. The publish uses this to stay silent
    /// until there is something to say.
    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }

    /// Fold one client-measured sample in.
    ///
    /// Every field is screened before it is kept: the label is trimmed to
    /// [`MAX_ACTION_LABEL`], the durations must be finite and non-negative, and
    /// a `SimHost` surface is refused outright — that surface is the host's own
    /// and a client claiming it would forge the one number CI compares against a
    /// baseline. A rejected sample is dropped silently; this is a diagnostic
    /// surface, and a client that mis-reports gets no data rather than a
    /// complaint on the sim thread.
    pub fn record_client(&mut self, sample: &ConsoleLatencySample) {
        if sample.surface == LatencySurface::SimHost {
            return;
        }
        let (Some(send), Some(ack)) = (
            sanitise_ms(sample.input_to_send_ms),
            sanitise_ms(sample.send_to_ack_ms),
        ) else {
            return;
        };
        let window = self.window;
        let Some(entry) = self.entry(sample.surface, &sample.action) else {
            return;
        };
        entry.input_to_send.push(send, window);
        entry.send_to_ack.push(ack, window);
        // The end-to-end figure is summarised from the per-sample SUM, not
        // assembled from the two summaries: p75 of a sum is not the sum of the
        // p75s, and the ~100 ms bar is a claim about the whole trip.
        entry.input_to_ack.push(send + ack, window);
    }

    /// Fold one host-measured `admit_to_broadcast` sample in.
    ///
    /// `action` is a `SystemControlPayload` variant name — the host's own
    /// vocabulary, never a client's string, which is why it needs no screening
    /// beyond the shared duration check.
    pub fn record_host(&mut self, action: &str, ms: f32) {
        let Some(ms) = sanitise_ms(ms) else {
            return;
        };
        let window = self.window;
        let Some(entry) = self.entry(LatencySurface::SimHost, action) else {
            return;
        };
        entry.admit_to_broadcast.push(ms, window);
    }

    /// The read-only projection: every series as a wire payload, sorted by
    /// `(surface, action)` — the `BTreeMap`'s own order, so the JSON is
    /// deterministic without a separate sort (payload convention 4).
    pub fn report(&self) -> ConsoleLatencyPayload {
        let actions = self
            .series
            .iter()
            .map(|((surface, action), series)| ActionLatencyEntry {
                surface: *surface,
                action: action.clone(),
                count: series
                    .input_to_ack
                    .len()
                    .max(series.admit_to_broadcast.len()) as u32,
                input_to_send: series.input_to_send.summary(),
                send_to_ack: series.send_to_ack.summary(),
                input_to_ack: series.input_to_ack.summary(),
                admit_to_broadcast: series.admit_to_broadcast.summary(),
            })
            .collect();
        ConsoleLatencyPayload {
            schema_version: DEBUG_SCHEMA_VERSION,
            actions,
        }
    }

    /// Every retained `admit_to_broadcast` sample, action by action.
    ///
    /// The raw values rather than the summary, because the #868 collector feeds
    /// them to `vellum_perf::Recorder` one at a time and lets the crate compute
    /// its own statistics — two percentile implementations over one dataset is
    /// how a capture and a report quietly stop agreeing.
    pub fn host_samples(&self) -> impl Iterator<Item = (&str, f32)> + '_ {
        self.series
            .iter()
            .filter(|((surface, _), _)| *surface == LatencySurface::SimHost)
            .flat_map(|((_, action), series)| {
                series
                    .admit_to_broadcast
                    .samples
                    .iter()
                    .map(move |ms| (action.as_str(), *ms))
            })
    }

    /// The series for one (surface, action), opening it if there is room.
    /// `None` once [`MAX_TRACKED_ACTIONS`] distinct series exist and this is not
    /// one of them.
    fn entry(&mut self, surface: LatencySurface, action: &str) -> Option<&mut ActionSeries> {
        let label = label_of(action);
        let key = (surface, label);
        if !self.series.contains_key(&key) && self.series.len() >= MAX_TRACKED_ACTIONS {
            return None;
        }
        Some(self.series.entry(key).or_default())
    }
}

/// A duration worth keeping, or `None`.
///
/// Rejects NaN, infinities and negatives — a negative "elapsed" means the
/// client's two stamps came from different time origins, which is a broken
/// measurement rather than a fast one, and letting it through would drag a p50
/// below zero.
fn sanitise_ms(ms: f32) -> Option<f32> {
    (ms.is_finite() && ms >= 0.0).then_some(ms)
}

/// Trim an action label to the accepted length, on a character boundary.
fn label_of(action: &str) -> String {
    if action.len() <= MAX_ACTION_LABEL {
        return action.to_string();
    }
    action
        .char_indices()
        .take_while(|(i, _)| *i <= MAX_ACTION_LABEL)
        .map(|(_, c)| c)
        .collect()
}

/// Whether console-latency measurement is running (issue #1169).
///
/// Unlike every other flag on this pipeline it gates MEASUREMENT, not only
/// rendering: with it off no `Instant::now()` is called, no sample is taken, and
/// a connected client does not measure or report either (it learns the flag from
/// `ServerMessage::DebugState`). See the module docs.
#[derive(Resource, Default, Debug)]
pub struct DebugConsoleLatencyEnabled(pub bool);

/// The latest console-latency JSON, when capture is enabled (issue #1169).
///
/// The target-agnostic sink, exactly like
/// [`crate::debug::StationActivityCapture`]: on the browser host the publish
/// ALSO writes the WASM bridge thread-local the dock reads, but every target
/// keeps the JSON here so the headless path and the determinism guard can read
/// it without a browser. `None` until the first publish.
#[derive(Resource, Default, Debug)]
pub struct ConsoleLatencyCapture(pub Option<String>);

/// The instant this tick's command admission finished (issue #1169).
///
/// `None` outside a measured tick. Wall-clock, therefore
/// `StateClass::Presentation` and never folded — see the module docs.
#[derive(Resource, Default, Debug)]
pub struct TickAdmissionInstant(pub Option<Instant>);

/// Stamp the moment this tick's admission finished (flag-gated).
///
/// Ordered `.after(AdmissionSet).before(SimSet::Input)`: after the tick's
/// commands exist, and strictly before the sim consumes them, so the window this
/// opens is the simulation's own service window and nothing else. Writes one
/// `Presentation` resource and reads nothing, so it can neither observe nor
/// perturb the tick it brackets.
pub fn stamp_admission_instant(mut stamp: ResMut<TickAdmissionInstant>) {
    stamp.0 = Some(Instant::now());
}

/// Close the window opened by [`stamp_admission_instant`] and attribute it to
/// every action served this tick (flag-gated).
///
/// Runs `.after(SimSet::Broadcast)` — the same end-of-tick window
/// `record_station_activity` and the unrouted-command lint use, and the only one
/// where `AdmittedCommands` holds the tick's FULL set: network-admitted commands
/// land before `SimSet::Input`, in-process AI emissions land during
/// `SimSet::Physics`.
///
/// # What the number is, precisely
///
/// One sample per action present at end of tick, all carrying the same value:
/// the tick's admission→broadcast wall time. It is a WINDOW attributed per
/// action, not a per-command queue time — for a command an in-process AI
/// operator emitted mid-tick, the window is an upper bound on that command's own
/// service. What the per-action split still tells you is *which* actions are
/// served in expensive ticks, which is the question a budget regression asks.
///
/// The stamp is taken as late as possible and read as early as possible in this
/// system so the two clock reads bracket the schedule rather than the query.
pub fn record_admit_to_broadcast(
    stamp: Res<TickAdmissionInstant>,
    ships: Query<&crate::core::messages::AdmittedCommands>,
    mut tracker: ResMut<ConsoleLatencyTracker>,
) {
    let Some(started) = stamp.0 else {
        return;
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    for admitted in ships.iter() {
        for command in admitted.0.iter() {
            let action = SystemControlPayloadDiscriminants::from(&command.payload);
            tracker.record_host(&format!("{action:?}"), elapsed_ms as f32);
        }
    }
}

/// Project the windows to JSON when capture is enabled (flag-gated).
///
/// Read-only: it never touches an authoritative resource, so its running or not
/// cannot move the digest. On the browser host it also feeds the WASM bridge
/// thread-local the dock panel reads; every target keeps the JSON in
/// [`ConsoleLatencyCapture`].
pub fn publish_console_latency(
    tracker: Res<ConsoleLatencyTracker>,
    mut capture: ResMut<ConsoleLatencyCapture>,
) {
    let payload = tracker.report();
    let json = crate::core::codec::encode_console_latency(&payload);

    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_console_latency_string(json.clone());

    capture.0 = Some(json);
}

/// Fold connected clients' own latency reports in (issue #1169).
///
/// Reads raw [`crate::lobby::InboundMessage`] rather than `AdmittedCommands`,
/// like its `ToggleDebugFlag` sibling and for the same reason: this is a session
/// diagnostic that changes no simulation outcome, so it must not enter the
/// command log a replay re-derives. There is no authority check because there is
/// no authority to check — a sample grants nothing, reaches nothing but a
/// `Presentation` resource, and every value it carries is screened by
/// [`ConsoleLatencyTracker::record_client`].
///
/// **Not compiled into a demo build**, and neither is the message it reads.
#[cfg(not(phoenix_demo_build))]
pub fn drain_console_latency_reports(
    mut reader: MessageReader<crate::lobby::InboundMessage>,
    mut tracker: ResMut<ConsoleLatencyTracker>,
) {
    use crate::core::messages::{ClientMessage, MAX_CONSOLE_LATENCY_SAMPLES};
    for ev in reader.read() {
        let ClientMessage::ReportConsoleLatency { samples } = &ev.msg else {
            continue;
        };
        for sample in samples.iter().take(MAX_CONSOLE_LATENCY_SAMPLES) {
            tracker.record_client(sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_sample(
        action: &str,
        surface: LatencySurface,
        send: f32,
        ack: f32,
    ) -> ConsoleLatencySample {
        ConsoleLatencySample {
            action: action.into(),
            surface,
            input_to_send_ms: send,
            send_to_ack_ms: ack,
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
        tracker.record_client(&client_sample(
            "fire_phaser",
            LatencySurface::PhoneConsole,
            4.0,
            60.0,
        ));
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
        tracker.record_client(&client_sample(
            "set_impulse",
            LatencySurface::PhoneConsole,
            30.0,
            10.0,
        ));
        tracker.record_client(&client_sample(
            "set_impulse",
            LatencySurface::PhoneConsole,
            1.0,
            50.0,
        ));
        let entry = &tracker.report().actions[0];
        let end_to_end = entry.input_to_ack.clone().expect("client surface has it");
        // Sums are 40 and 51; the max of the sums is 51, NOT max(30) + max(50).
        assert_eq!(end_to_end.max_ms, 51.0);
    }

    /// The two client surfaces are separate series: a phone's WebRTC round trip
    /// must never be averaged into the host page's in-process one.
    #[test]
    fn the_two_client_surfaces_are_kept_apart() {
        let mut tracker = ConsoleLatencyTracker::default();
        tracker.record_client(&client_sample(
            "fire_phaser",
            LatencySurface::BrowserHost,
            1.0,
            2.0,
        ));
        tracker.record_client(&client_sample(
            "fire_phaser",
            LatencySurface::PhoneConsole,
            1.0,
            90.0,
        ));
        let payload = tracker.report();
        assert_eq!(payload.actions.len(), 2, "one series per surface");
        assert_eq!(payload.actions[0].surface, LatencySurface::BrowserHost);
        assert_eq!(payload.actions[1].surface, LatencySurface::PhoneConsole);
    }

    /// A client may not file samples under the host's own surface — that is the
    /// series a CI budget compares against a baseline.
    #[test]
    fn a_client_cannot_report_against_the_sim_host_surface() {
        let mut tracker = ConsoleLatencyTracker::default();
        tracker.record_client(&client_sample(
            "FirePhaser",
            LatencySurface::SimHost,
            0.0,
            0.0,
        ));
        assert!(tracker.is_empty(), "SimHost samples are the host's alone");
    }

    /// Garbage in is dropped, not folded: a negative or non-finite duration is a
    /// broken measurement and would drag every percentile with it.
    #[test]
    fn non_finite_and_negative_durations_are_refused() {
        let mut tracker = ConsoleLatencyTracker::default();
        for (send, ack) in [(-1.0, 5.0), (5.0, f32::NAN), (f32::INFINITY, 5.0)] {
            tracker.record_client(&client_sample("x", LatencySurface::PhoneConsole, send, ack));
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
            tracker.record_client(&client_sample(
                &format!("action_{i}"),
                LatencySurface::PhoneConsole,
                1.0,
                1.0,
            ));
        }
        assert_eq!(tracker.report().actions.len(), MAX_TRACKED_ACTIONS);
    }

    #[test]
    fn a_long_action_label_is_trimmed_on_a_character_boundary() {
        let long = "é".repeat(200);
        let trimmed = label_of(&long);
        assert!(trimmed.len() <= MAX_ACTION_LABEL + 2, "trimmed to the cap");
        assert!(trimmed.chars().all(|c| c == 'é'), "no split character");
    }

    /// A host entry carries only the host segment, and a client entry only the
    /// client segments — the payload must never invent the other side.
    #[test]
    fn each_surface_carries_only_the_segments_it_can_observe() {
        let mut tracker = ConsoleLatencyTracker::default();
        tracker.record_host("FirePhaser", 3.0);
        tracker.record_client(&client_sample(
            "fire_phaser",
            LatencySurface::PhoneConsole,
            1.0,
            2.0,
        ));
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

        let phone = payload
            .actions
            .iter()
            .find(|e| e.surface == LatencySurface::PhoneConsole)
            .expect("phone entry");
        assert!(phone.admit_to_broadcast.is_none());
        assert!(phone.input_to_send.is_some());
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
}
