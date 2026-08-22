//! The shared structured debug-payload schema (PRD #1144).
//!
//! # This module IS the schema
//!
//! Every debug surface in the observability pipeline — the station-activity
//! chart in this slice, and the scenario-state / AI-state surfaces the later
//! slices (#1147–#1152) add — is a plain `serde` struct produced by a read-only
//! `collect` projection off authoritative state and carried to a consumer as
//! JSON. There are no pre-formatted strings on this pipeline: a text overlay is
//! the *legacy* `debug_overlay` shape this pipeline replaces.
//!
//! ## Conventions the later slices copy
//!
//! 1. **One module, one schema.** Every surface's payload struct lives here in
//!    `crate::debug::payload`, not scattered next to the projector that fills it.
//!    A consumer (the dock, the headless report, the GM console M6 Live
//!    Inspector) imports one module to know the whole wire vocabulary.
//! 2. **Every top-level payload carries [`DEBUG_SCHEMA_VERSION`].** A consumer
//!    reads the version first and refuses or adapts to a mismatch rather than
//!    silently misreading a payload from a newer host. Additive, serde-tolerant
//!    field growth does not bump it; a rename or a removed/retyped field does.
//! 3. **`serde` derives, never hand-rolled JSON.** The one place the structs
//!    become JSON is [`crate::core::codec`] (AGENTS.md Key Constraint 1 confines
//!    `serde_json` there). Each surface adds one `encode_*` there; nothing else
//!    imports `serde_json`.
//! 4. **Deterministic ordering.** Collections are emitted in a stable order
//!    (sorted keys, oldest→newest time series) so two hosts folding the same
//!    state produce byte-identical JSON — the property the GM inspector and any
//!    diff tooling lean on.
//!
//! Nothing in here is authoritative simulation state: these are read-only
//! projections, so producing one can never move the #894 digest (see
//! `crate::debug::station_activity` for the determinism guard).

use serde::{Deserialize, Serialize};

/// The wire-schema version every top-level debug payload stamps.
///
/// See convention 2 in the module docs. Bump on a breaking shape change to any
/// payload in this module; a consumer compares it against the version it was
/// built for.
pub const DEBUG_SCHEMA_VERSION: u32 = 1;

/// Which control source issued the admitted commands counted in a bucket.
///
/// The wire form of [`crate::ship::control_source::ControlSource`], kept as a
/// separate type so the payload schema does not depend on the internals of the
/// ship control model and so the JSON field names are stable regardless of how
/// the simulation spells the source. `Offline` is present for completeness — a
/// command admitted against an offline system cannot happen (admission refuses
/// it), so the count stays zero, but the slot keeps the schema uniform across
/// all three sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivitySource {
    Human,
    Ai,
    Offline,
}

impl From<crate::ship::control_source::ControlSource> for ActivitySource {
    fn from(source: crate::ship::control_source::ControlSource) -> Self {
        use crate::ship::control_source::ControlSource;
        match source {
            ControlSource::Human => ActivitySource::Human,
            ControlSource::Ai => ActivitySource::Ai,
            ControlSource::Offline => ActivitySource::Offline,
        }
    }
}

/// One station's admitted-command tally for one time bucket, split by control
/// source.
///
/// A "few integers per station", per the PRD: the always-on counters cost this
/// much and no more.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationActivityEntry {
    /// The `StationId` string this tally is for.
    pub station: String,
    /// Commands admitted while the station was human-crewed.
    pub human: u32,
    /// Commands admitted while the station was AI-crewed (Backfill or NPC).
    pub ai: u32,
    /// Commands admitted while the station was offline. Always zero in practice
    /// (admission refuses an offline system); kept for schema uniformity.
    pub offline: u32,
}

impl StationActivityEntry {
    /// Add one admitted command from `source` to this entry.
    pub fn record(&mut self, source: ActivitySource) {
        match source {
            ActivitySource::Human => self.human = self.human.saturating_add(1),
            ActivitySource::Ai => self.ai = self.ai.saturating_add(1),
            ActivitySource::Offline => self.offline = self.offline.saturating_add(1),
        }
    }

    /// Total commands across all sources in this entry.
    pub fn total(&self) -> u32 {
        self.human
            .saturating_add(self.ai)
            .saturating_add(self.offline)
    }
}

/// One time bucket: every station's tally over one `bucket_ticks`-long window.
///
/// `stations` is sorted by station id so the JSON is deterministic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationActivityBucket {
    /// The `SimTick` at which this bucket opened (`bucket_index * bucket_ticks`).
    pub start_tick: u64,
    /// Per-station tallies, sorted by [`StationActivityEntry::station`].
    pub stations: Vec<StationActivityEntry>,
}

/// The station-activity surface's whole payload: a bounded time series of
/// per-station, per-source admitted-command counts.
///
/// Produced by `crate::debug::station_activity::StationActivityTracker::report`,
/// encoded to JSON by `crate::core::codec::encode_station_activity`, and read by
/// the dock chart (`gui/station-activity-chart.js`).
///
/// Not `Eq`: `bucket_secs` is an `f32`. `PartialEq` is enough for the tests that
/// compare payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationActivityPayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// The bucket length in `SimTick`s the counters actually used.
    pub bucket_ticks: u64,
    /// The authored bucket length in simulation seconds (for the chart's axis
    /// label). Derived from `[global] station_activity_bucket_secs`.
    pub bucket_secs: f32,
    /// Buckets oldest→newest, ending with the in-progress bucket.
    pub buckets: Vec<StationActivityBucket>,
}

impl Default for StationActivityPayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            bucket_ticks: 0,
            bucket_secs: 0.0,
            buckets: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ship::control_source::ControlSource;

    #[test]
    fn activity_source_maps_from_every_control_source() {
        assert_eq!(
            ActivitySource::from(ControlSource::Human),
            ActivitySource::Human
        );
        assert_eq!(ActivitySource::from(ControlSource::Ai), ActivitySource::Ai);
        assert_eq!(
            ActivitySource::from(ControlSource::Offline),
            ActivitySource::Offline
        );
    }

    #[test]
    fn entry_records_into_the_matching_source_and_totals() {
        let mut entry = StationActivityEntry::default();
        entry.record(ActivitySource::Human);
        entry.record(ActivitySource::Human);
        entry.record(ActivitySource::Ai);
        assert_eq!(entry.human, 2);
        assert_eq!(entry.ai, 1);
        assert_eq!(entry.offline, 0);
        assert_eq!(entry.total(), 3);
    }

    #[test]
    fn default_payload_carries_the_current_schema_version() {
        assert_eq!(
            StationActivityPayload::default().schema_version,
            DEBUG_SCHEMA_VERSION
        );
    }
}
