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
//! | `input_to_send` (①) | the client's | phone console |
//! | `send_to_ack` (②+③+④) | the client's | phone console |
//! | `input_to_ack` (sum) | the client's | phone console |
//! | `admit_to_broadcast` (③) | the host's | browser host, native, headless |
//!
//! The client half is measured in `gui/console-latency.js` and arrives here as
//! already-differenced durations via [`ClientMessage::ReportConsoleLatency`].
//! This module owns the host half and the folding of both into one payload.
//!
//! # Two things this deliberately does not claim
//!
//! **Pixels.** `send_to_ack` ends when the issuing console's surface is handed
//! fresh server-derived state, which is the last moment any of this code can
//! observe. The compositor is past that line.
//!
//! **Per-command service time, on the client segments.** A client cannot see
//! which broadcast its command caused, so `send_to_ack` measures the wait until
//! the surface next received server state — a *perceived-feedback proxy*,
//! bounded below by the host's broadcast cadence (`SimState` alone dirties
//! several consoles at 10 Hz). That is the right number for the ~100 ms polish
//! bar, which is about what a player perceives, and the WRONG number for
//! detecting a per-command processing regression. Per-command truth lives in
//! `admit_to_broadcast` and nowhere else. (Issue #1169 review, finding C1.)
//!
//! There is no browser-host client series: the host page mounts no console
//! surface that receives state, so it has no acknowledgement to measure — see
//! [`LatencySurface`] for the full reasoning and what would bring it back.
//!
//! # Outages are counted, not swallowed
//!
//! A client action its surface never answers yields no duration at all, so an
//! outage would otherwise read — in the distributions alone — exactly like a
//! quiet, healthy link. The client counts those expiries per action and reports
//! them; [`ActionLatencyEntry::expired`] carries the count beside the
//! distributions it qualifies.
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
    ConsoleLatencyExpiry, ConsoleLatencySample, LatencySurface, SystemControlPayloadDiscriminants,
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

/// How many distinct action series the tracker will open **per surface**.
///
/// A bound on untrusted input as much as on memory: a client surface's action
/// label is a STRING the client chose, so an unbounded map would be a
/// client-controlled allocation. Once a surface's cap is reached, samples for
/// unseen actions on THAT surface are dropped rather than evicting a live one.
///
/// **Per surface, not global** (issue #1169 review, finding C3). A single global
/// cap made the surfaces share one budget, so two batches naming 96 junk actions
/// permanently starved the host's own `SimHost` rows — the series a CI budget
/// compares — with no recovery short of a page reload. Each surface now has its
/// own budget, and `SimHost` is reachable by no wire message at all, so a client
/// can exhaust only the space its own reports occupy.
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

/// The four segments one (surface, action) pair can carry, plus its
/// unanswered-action count.
#[derive(Clone, Debug, Default)]
struct ActionSeries {
    input_to_send: Series,
    send_to_ack: Series,
    input_to_ack: Series,
    admit_to_broadcast: Series,
    /// Actions of this kind the client raised that its surface never answered.
    /// Cumulative rather than windowed: an outage is a fact about the run, and a
    /// rolling window would let it age out of sight while it was still happening.
    expired: u32,
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

    /// Forget every series.
    ///
    /// Called when the flag is switched ON (issue #1169 review, finding C3), for
    /// two reasons that point the same way. It matches the client meters, which
    /// discard their own in-flight work on every enable change — without it the
    /// two halves disagree, and a re-enable republishes a window measured under
    /// conditions nobody is looking at any more. And it is the recovery path for
    /// a surface whose action budget a misbehaving client filled: switching the
    /// surface off and on again empties it, rather than the reload the first
    /// implementation required.
    pub fn clear(&mut self) {
        self.series.clear();
    }

    /// Fold one client-measured sample in, against the surface the HOST assigned.
    ///
    /// `surface` is a parameter and not a field of `sample` on purpose: a client
    /// does not name its own surface. It is derived from the fact that the report
    /// arrived over a session at all, so a peer cannot file against a series it
    /// does not own — and `SimHost`, the series a CI budget compares, is
    /// reachable by no wire message whatsoever (the drain passes
    /// [`LatencySurface::PhoneConsole`] and nothing else).
    ///
    /// The remaining fields are still screened: the label is trimmed to
    /// [`MAX_ACTION_LABEL`] and the durations must be finite and non-negative. A
    /// rejected sample is dropped silently; this is a diagnostic surface, and a
    /// client that mis-reports gets no data rather than a complaint on the sim
    /// thread.
    pub fn record_client(&mut self, surface: LatencySurface, sample: &ConsoleLatencySample) {
        // Defence in depth rather than an assertion: the wire carries no surface
        // field and the only caller passes a constant, so reaching this arm is a
        // programming error — but the series behind it is the one a CI budget
        // compares against a baseline, and a silent refusal is the right failure
        // mode for a diagnostic. Pinned by
        // `tests::a_client_fold_can_never_write_the_sim_host_surface`.
        if surface == LatencySurface::SimHost {
            return;
        }
        let (Some(send), Some(ack)) = (
            sanitise_ms(sample.input_to_send_ms),
            sanitise_ms(sample.send_to_ack_ms),
        ) else {
            return;
        };
        let window = self.window;
        let Some(entry) = self.entry(surface, &sample.action) else {
            return;
        };
        entry.input_to_send.push(send, window);
        entry.send_to_ack.push(ack, window);
        // The end-to-end figure is summarised from the per-sample SUM, not
        // assembled from the two summaries: p75 of a sum is not the sum of the
        // p75s, and the ~100 ms bar is a claim about the whole trip.
        entry.input_to_ack.push(send + ack, window);
    }

    /// Fold one client-reported outage count in (issue #1169 review, C1).
    ///
    /// Opens a series for an action that has produced no successful sample at
    /// all, which is exactly the case worth seeing: an action whose surface has
    /// never once answered appears here and nowhere else.
    pub fn record_client_expiry(&mut self, surface: LatencySurface, expiry: &ConsoleLatencyExpiry) {
        if surface == LatencySurface::SimHost || expiry.count == 0 {
            return;
        }
        let Some(entry) = self.entry(surface, &expiry.action) else {
            return;
        };
        entry.expired = entry.expired.saturating_add(expiry.count);
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
                expired: series.expired,
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

    /// The series for one (surface, action), opening it if that SURFACE has room.
    ///
    /// `None` once this surface holds [`MAX_TRACKED_ACTIONS`] distinct actions
    /// and this is not one of them. The budget is counted per surface, so a
    /// client filling its own cannot displace another surface's series — see
    /// [`MAX_TRACKED_ACTIONS`] for the starvation this closes.
    fn entry(&mut self, surface: LatencySurface, action: &str) -> Option<&mut ActionSeries> {
        let label = label_of(action);
        let key = (surface, label);
        if !self.series.contains_key(&key) && self.surface_len(surface) >= MAX_TRACKED_ACTIONS {
            return None;
        }
        Some(self.series.entry(key).or_default())
    }

    /// How many distinct actions one surface currently holds.
    ///
    /// A range scan over the `BTreeMap`, so it walks only that surface's block
    /// rather than the whole map — the keys are `(surface, action)` and
    /// `LatencySurface` is `Ord`, so one surface's entries are contiguous.
    fn surface_len(&self, surface: LatencySurface) -> usize {
        self.series
            .range((surface, String::new())..)
            .take_while(|((s, _), _)| *s == surface)
            .count()
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

impl crate::debug::catalogue::DebugSurfaceState for DebugConsoleLatencyEnabled {
    fn is_enabled(&self) -> bool {
        self.0
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.0 = enabled;
    }
}

/// Module-owned adapter for the console-latency Debug Surface.
pub const DEBUG_CONSOLE_LATENCY_ADAPTER: crate::debug::catalogue::DebugSurfaceAdapter =
    crate::debug::catalogue::DebugSurfaceAdapter::for_resource::<DebugConsoleLatencyEnabled>(
        crate::core::debug_surface::DebugSurface::ConsoleLatency,
    );

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
/// `None` outside a measured tick, and that is enforced rather than merely
/// documented: [`record_admit_to_broadcast`] TAKES the stamp when it closes the
/// window, so a tick whose stamp system did not run (the flag went off between
/// the two, or a fixture registered only the recorder) finds `None` and records
/// nothing instead of differencing against a stale instant from some earlier
/// tick.
///
/// Wall-clock, therefore `StateClass::Presentation` and never folded — see the
/// module docs.
#[derive(Resource, Default, Debug)]
pub struct TickAdmissionInstant(pub Option<Instant>);

/// How often the flag-gated JSON publish runs, in Hz of simulation time.
///
/// The publish clones every label, sorts up to [`DEFAULT_WINDOW`] floats per
/// series and encodes JSON. At the shipped 30–60 Hz tick that was happening
/// every tick, INSIDE the window `sim.tick` measures — so a `--perf-capture` run
/// was charging the metric for the cost of producing the metric (issue #1169
/// review, finding C4). 4 Hz is far above what a dock chart or a run report can
/// use and far below the tick rate, so the projection stops distorting the thing
/// it is projecting.
const PUBLISH_HZ: f32 = 4.0;

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
/// Closing the window CONSUMES the stamp, so no later tick can difference
/// against it — see [`TickAdmissionInstant`].
pub fn record_admit_to_broadcast(
    mut stamp: ResMut<TickAdmissionInstant>,
    ships: Query<&crate::core::messages::AdmittedCommands>,
    mut tracker: ResMut<ConsoleLatencyTracker>,
) {
    let Some(started) = stamp.0.take() else {
        return;
    };
    let elapsed_ms = (started.elapsed().as_secs_f64() * 1000.0) as f32;
    for admitted in ships.iter() {
        for command in admitted.0.iter() {
            // `&'static str` straight out of the discriminant enum, not
            // `format!("{:?}")`: this runs once per admitted command inside the
            // window `sim.tick` measures, and an allocation there is the
            // measurement charging for itself (issue #1169 review, C4).
            let action: &'static str =
                SystemControlPayloadDiscriminants::from(&command.payload).into();
            tracker.record_host(action, elapsed_ms);
        }
    }
}

/// Project the windows to JSON when capture is enabled (flag-gated, throttled).
///
/// Read-only: it never touches an authoritative resource, so its running or not
/// cannot move the digest. On the browser host it also feeds the WASM bridge
/// thread-local the dock panel reads; every target keeps the JSON in
/// [`ConsoleLatencyCapture`].
///
/// Throttled to [`PUBLISH_HZ`] of simulation time rather than run every tick.
/// The interval is derived from the authored `sim_tick_hz` and rounded to whole
/// ticks, so it is integer and independent of frame pacing — the same shape
/// `StationActivityTracker::configure` uses. `SimTick` and `WorldConfig` are
/// `Option` so a bare-`App` fixture that registered neither still publishes
/// (every tick, from tick 0).
pub fn publish_console_latency(
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    tracker: Res<ConsoleLatencyTracker>,
    mut capture: ResMut<ConsoleLatencyCapture>,
) {
    if let (Some(tick), Some(wc)) = (sim_tick.as_deref(), world_config.as_deref()) {
        let interval = publish_interval_ticks(wc.global.sim_tick_hz);
        if tick.0 % interval != 0 {
            return;
        }
    }

    let payload = tracker.report();
    let json = crate::core::codec::encode_console_latency(&payload);

    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_console_latency_string(json.clone());

    capture.0 = Some(json);
}

/// Whole ticks between publishes at the authored tick rate, always `>= 1`.
fn publish_interval_ticks(sim_tick_hz: f32) -> u64 {
    let ticks = (sim_tick_hz / PUBLISH_HZ) as f64;
    if ticks.is_finite() && ticks >= 1.0 {
        ticks.round() as u64
    } else {
        1
    }
}

/// Empty the tracker whenever measurement is switched ON (issue #1169 review,
/// finding C3).
///
/// Two problems, one fix. The client meters discard their in-flight work on
/// every enable change, so without this the two halves disagreed and a re-enable
/// republished a window measured under conditions nobody is looking at any more.
/// And a surface whose action budget a misbehaving client had filled had no
/// recovery short of a page reload; now switching the surface off and on again
/// is the recovery.
///
/// Runs in `PreUpdate` on the change edge only — `Res::is_changed` covers both
/// the drain that flips it from a phone and the host cog's absolute set.
pub fn clear_console_latency_on_enable(
    flag: Res<DebugConsoleLatencyEnabled>,
    mut tracker: ResMut<ConsoleLatencyTracker>,
    mut capture: ResMut<ConsoleLatencyCapture>,
) {
    if !flag.is_changed() || !flag.0 {
        return;
    }
    tracker.clear();
    // The last published JSON described the window just cleared; leaving it
    // would show a stale table until the next publish lands.
    capture.0 = None;
    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_console_latency_string(String::new());
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
/// # The surface is assigned here, not read off the wire
///
/// Every report that reaches this drain arrived over a client session, so its
/// surface is [`LatencySurface::PhoneConsole`] and the message carries no field
/// to say otherwise. That is what makes forgery structurally impossible rather
/// than merely refused: there is no other surface a client could name, and the
/// host's own `SimHost` series has no wire route at all.
///
/// **Not compiled into a demo build**, and neither is the message it reads.
#[cfg(not(phoenix_demo_build))]
pub fn drain_console_latency_reports(
    mut reader: MessageReader<crate::lobby::InboundMessage>,
    mut tracker: ResMut<ConsoleLatencyTracker>,
) {
    use crate::core::messages::{ClientMessage, MAX_CONSOLE_LATENCY_SAMPLES};
    for ev in reader.read() {
        let ClientMessage::ReportConsoleLatency { samples, expired } = &ev.msg else {
            continue;
        };
        for sample in samples.iter().take(MAX_CONSOLE_LATENCY_SAMPLES) {
            tracker.record_client(LatencySurface::PhoneConsole, sample);
        }
        for outage in expired.iter().take(MAX_CONSOLE_LATENCY_SAMPLES) {
            tracker.record_client_expiry(LatencySurface::PhoneConsole, outage);
        }
    }
}

#[cfg(test)]
mod tests {
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
}
