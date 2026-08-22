//! Station-activity tracking — the first tracer-bullet slice of the debug
//! observability pipeline (issue #1145, PRD #1144).
//!
//! # What this is
//!
//! An always-on set of counters that bucket *admitted commands* per station,
//! per configurable time chunk, split by control source (human / AI / offline).
//! It is the evidence crew-control and Backfill tuning need: how busy each
//! station is, and whether a human is working it or Backfill is carrying it.
//!
//! # Where it taps, and why it is not inside `admit_system_commands`
//!
//! The counters read each ship's [`AdmittedCommands`] once per tick, AFTER the
//! whole tick's admission has run (`.after(SimSet::Broadcast)`, the same window
//! `warn_unrouted_admitted_commands` observes). That is deliberate and it is the
//! only correct tap for a human-vs-AI split:
//!
//! - Network commands (human tokens and `ai:` Backfill tokens) are admitted by
//!   [`crate::command_admission::admit_system_commands`] before `SimSet::Input`.
//! - **In-process AI decisions bypass that system entirely** — the per-axis helm
//!   AI, the weapons deciders, and every other console/system AI operator emit
//!   through [`crate::command_admission::ai_emit::emit_ai_command`] straight into
//!   their ship's `AdmittedCommands` during `SimSet::Physics`.
//!
//! `AdmittedCommands` is the ONE buffer both halves land in, so reading it at
//! end-of-tick is the only place that sees a station's full workload. Tapping
//! inside `admit_system_commands` would miss every AI emission — which is
//! exactly the "Backfill carrying a station" signal this surface exists to show.
//!
//! An `AdmittedCommand` deliberately carries no source identity (AGENTS.md
//! constraint 6: nothing downstream of admission may branch on human-vs-AI). The
//! control source is re-derived here from the ship's own authoritative
//! [`ControlSourceResolver`] — a read-only projection, not a behavioural branch:
//! it routes no command and changes no simulation outcome, so the symmetry
//! constraint is untouched.
//!
//! # Determinism
//!
//! The counters run on every target (browser host, headless, native), always,
//! whatever the debug flag says — so enabling capture cannot introduce them.
//! Their state lives in [`StationActivityTracker`], a resource `world_digest`
//! never folds; the read-only [`StationActivityTracker::report`] projection and
//! its flag-gated JSON publish touch no authoritative state. Enabling debug
//! output therefore leaves a seeded digest byte-identical — proven by
//! `tests/station_activity.rs`.

use bevy::prelude::*;
use std::collections::{BTreeMap, VecDeque};

use crate::core::messages::{StationId, SystemId};
use crate::debug::payload::{
    ActivitySource, StationActivityBucket, StationActivityEntry, StationActivityPayload,
    DEBUG_SCHEMA_VERSION,
};

/// How many completed buckets the rolling chart window retains. A presentation
/// bound on the debug surface, not a gameplay value: it decides how much history
/// the chart can draw, nothing the fixed tick computes.
pub const DEFAULT_MAX_BUCKETS: usize = 32;

/// One time bucket's per-station tallies, keyed for deterministic ordering.
#[derive(Clone, Debug)]
struct Bucket {
    /// `start_tick / bucket_ticks` — the bucket's ordinal on the time axis.
    index: u64,
    /// The `SimTick` at which this bucket opened.
    start_tick: u64,
    /// Per-station tallies, keyed by the station id STRING (not `StationId`,
    /// which is not `Ord`). A `BTreeMap` so `report` emits stations in a stable
    /// (sorted) order without a separate sort.
    stations: BTreeMap<String, StationActivityEntry>,
}

impl Bucket {
    fn new(index: u64, start_tick: u64) -> Self {
        Self {
            index,
            start_tick,
            stations: BTreeMap::new(),
        }
    }

    fn to_payload(&self) -> StationActivityBucket {
        StationActivityBucket {
            start_tick: self.start_tick,
            stations: self.stations.values().cloned().collect(),
        }
    }
}

/// The always-on station-activity counters (issue #1145).
///
/// Pure and Bevy-agnostic apart from the `Resource` derive: [`Self::record`],
/// [`Self::begin_tick`], [`Self::configure`] and [`Self::report`] are the whole
/// interface, testable without an `App`. `world_digest` never folds this — it is
/// declared `StateClass::Presentation` at `DebugPlugin::build`.
#[derive(Resource, Clone, Debug)]
pub struct StationActivityTracker {
    /// Bucket length in `SimTick`s (always `>= 1`).
    bucket_ticks: u64,
    /// The authored bucket length in simulation seconds, carried into the
    /// payload for the chart's axis label.
    bucket_secs: f32,
    /// How many completed buckets to retain.
    max_buckets: usize,
    /// The in-progress bucket, if any.
    current: Option<Bucket>,
    /// Finished buckets, oldest first, bounded to `max_buckets`.
    completed: VecDeque<Bucket>,
}

impl Default for StationActivityTracker {
    fn default() -> Self {
        // A degenerate one-tick bucket until `configure` learns the authored
        // `[global] station_activity_bucket_secs` and `sim_tick_hz` — the same
        // "safe default until authored data arrives" shape `BoundedHistory` uses.
        Self::new(1, DEFAULT_MAX_BUCKETS)
    }
}

impl StationActivityTracker {
    /// A tracker with an explicit bucket length (ticks) and retention window.
    /// Used directly by the pure unit tests; the Bevy path constructs the
    /// default and calls [`Self::configure`].
    pub fn new(bucket_ticks: u64, max_buckets: usize) -> Self {
        Self {
            bucket_ticks: bucket_ticks.max(1),
            bucket_secs: 0.0,
            max_buckets,
            current: None,
            completed: VecDeque::new(),
        }
    }

    /// The bucket length currently in effect, in `SimTick`s.
    pub fn bucket_ticks(&self) -> u64 {
        self.bucket_ticks
    }

    /// Apply the authored bucket length. Converts `bucket_secs` at `sim_tick_hz`
    /// to an integer tick count (`>= 1`), so bucketing stays integer and
    /// deterministic regardless of frame pacing.
    ///
    /// Idempotent when the derived tick count is unchanged — safe to call every
    /// tick. A genuine change (e.g. the first configured tick, moving off the
    /// degenerate default) restarts the series, because buckets measured under
    /// one length cannot be reinterpreted under another.
    pub fn configure(&mut self, bucket_secs: f32, sim_tick_hz: f32) {
        let ticks = {
            let product = bucket_secs as f64 * sim_tick_hz as f64;
            if product.is_finite() && product >= 1.0 {
                product.round() as u64
            } else {
                1
            }
        };
        if ticks != self.bucket_ticks {
            self.bucket_ticks = ticks;
            self.current = None;
            self.completed.clear();
        }
        self.bucket_secs = bucket_secs;
    }

    /// Advance the time axis to `now_tick`, opening the bucket it falls in and
    /// finalising any the run has moved past. Records nothing — call it every
    /// tick so a quiet stretch still shows as empty buckets on the chart.
    pub fn begin_tick(&mut self, now_tick: u64) {
        self.ensure_bucket(now_tick);
    }

    /// Count one admitted command against `station` from `source`, in the bucket
    /// `now_tick` falls in.
    pub fn record(&mut self, now_tick: u64, station: &StationId, source: ActivitySource) {
        self.ensure_bucket(now_tick);
        if let Some(current) = self.current.as_mut() {
            let entry = current.stations.entry(station.0.clone()).or_default();
            entry.station = station.0.clone();
            entry.record(source);
        }
    }

    /// The read-only projection: the bounded time series as a wire payload.
    pub fn report(&self) -> StationActivityPayload {
        let mut buckets = Vec::with_capacity(self.completed.len() + 1);
        for bucket in &self.completed {
            buckets.push(bucket.to_payload());
        }
        if let Some(current) = &self.current {
            buckets.push(current.to_payload());
        }
        StationActivityPayload {
            schema_version: DEBUG_SCHEMA_VERSION,
            bucket_ticks: self.bucket_ticks,
            bucket_secs: self.bucket_secs,
            buckets,
        }
    }

    /// Open the bucket `now_tick` belongs to, finalising the current one (and
    /// filling any wholly-empty gap buckets, capped by `max_buckets`) when the
    /// run has advanced past it.
    fn ensure_bucket(&mut self, now_tick: u64) {
        let ticks = self.bucket_ticks.max(1);
        let index = now_tick / ticks;
        match &self.current {
            None => {
                self.current = Some(Bucket::new(index, index * ticks));
            }
            Some(current) if index == current.index => {}
            Some(current) if index > current.index => {
                let gap = index - current.index; // >= 1
                let finished = self.current.take().expect("current is Some in this arm");
                self.push_completed(finished);
                // The wholly-empty buckets between the one just finished and the
                // new one, so a quiet stretch reads as zeros on the chart. Capped
                // at `max_buckets`: a huge quiet gap needs no more empties than
                // the window can show.
                let empties = (gap - 1).min(self.max_buckets as u64);
                for k in 0..empties {
                    let ei = index - empties + k;
                    self.push_completed(Bucket::new(ei, ei * ticks));
                }
                self.current = Some(Bucket::new(index, index * ticks));
            }
            // `index < current.index`: the tick went backwards, which a
            // monotonic `SimTick` never does. Ignore rather than rewrite history.
            Some(_) => {}
        }
    }

    fn push_completed(&mut self, bucket: Bucket) {
        self.completed.push_back(bucket);
        while self.completed.len() > self.max_buckets {
            self.completed.pop_front();
        }
    }
}

/// Whether the station-activity debug output is being rendered (issue #1145).
///
/// The flag gates only the JSON *publish*; the counters in
/// [`StationActivityTracker`] run whatever this says. Flipped from the host
/// cog's Debug tab (`wasm_toggle_station_activity`) and from a connected phone
/// (`DebugFlag::StationActivity`), read back in `ServerMessage::DebugState`.
#[derive(Resource, Default, Debug)]
pub struct DebugStationActivityEnabled(pub bool);

/// The latest station-activity JSON, when capture is enabled (issue #1145).
///
/// The target-agnostic sink: on the browser host the publish system ALSO writes
/// the WASM bridge thread-local the dock reads, but every target keeps the JSON
/// here so the headless report path (a later PRD #1144 slice) and the
/// determinism guard can read it without a browser. `None` until the first
/// publish; never folded into the digest.
#[derive(Resource, Default, Debug)]
pub struct StationActivityCapture(pub Option<String>);

/// Count this tick's admitted commands into the tracker (always-on).
///
/// Runs after the whole tick's admission (`.after(SimSet::Broadcast)`), reading
/// the unified `AdmittedCommands` so it sees both network-admitted and
/// in-process AI-emitted commands — see the module docs for why that is the only
/// correct tap for a human-vs-AI split. Read-only w.r.t. every folded resource.
///
/// `WorldConfig` and `SimTick` are `Option` so a bare-`App` fixture that
/// registered neither still runs (degenerate one-tick buckets from tick 0).
pub fn record_station_activity(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    ships: Query<(
        &crate::core::messages::AdmittedCommands,
        &crate::ship_plugin::ShipSystemControlSources,
        &crate::ship_plugin::ShipConfigComponent,
        Option<&crate::ship_plugin::HumanSeekingHosts>,
    )>,
    mut tracker: ResMut<StationActivityTracker>,
) {
    let now = sim_tick.map_or(0, |t| t.0);

    // Keep the bucket length in step with the authored config every tick — a
    // no-op once set (see `configure`), and it means a run that retunes between
    // rounds picks the new length up without a dedicated apply system.
    if let Some(wc) = world_config.as_deref() {
        tracker.configure(
            wc.global.station_activity_bucket_secs,
            wc.global.sim_tick_hz,
        );
    }

    // Advance the time axis even when no command was admitted, so a quiet
    // stretch draws as empty buckets rather than vanishing from the chart.
    tracker.begin_tick(now);

    for (admitted, sources, config, hosts) in ships.iter() {
        for command in admitted.0.iter() {
            let target: &SystemId = &command.target;
            let Some(station) =
                crate::command_admission::station_for_system(&config.0, hosts, target)
            else {
                // Ownerless systems (e.g. the host-only god-mode route) have no
                // station and are not station activity — skip them.
                continue;
            };
            let source = ActivitySource::from(sources.0.source_for(target));
            tracker.record(now, &station, source);
        }
    }
}

/// Project the counters to JSON when capture is enabled (flag-gated).
///
/// Read-only: it never touches an authoritative resource, so its running or not
/// cannot move the digest. On the browser host it also feeds the WASM bridge
/// thread-local the dock chart reads; every target keeps the JSON in
/// [`StationActivityCapture`].
pub fn publish_station_activity(
    tracker: Res<StationActivityTracker>,
    mut capture: ResMut<StationActivityCapture>,
) {
    let payload = tracker.report();
    let json = crate::core::codec::encode_station_activity(&payload);

    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_station_activity_string(json.clone());

    capture.0 = Some(json);
}

#[cfg(test)]
mod tests {
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
}
