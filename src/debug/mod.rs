//! Structured debug observability (PRD #1144).
//!
//! One pipeline for making the running simulation inspectable: every debug
//! surface is a `serde` struct ([`payload`]) produced by a read-only `collect`
//! projection off authoritative state, encoded to JSON in
//! [`crate::core::codec`], and consumed identically by the host-page debug dock,
//! the headless report, and (later) the GM console Live Inspector. This is the
//! home the four legacy text overlays in [`crate::debug_overlay`] migrate onto,
//! and the schema the later PRD #1144 slices extend — read [`payload`] for the
//! conventions they follow.
//!
//! This first slice ([`station_activity`], issue #1145) drives station activity
//! end-to-end: an always-on tracker at command admission, the shared payload
//! schema, a debug flag + WASM bridge getter, and the dock chart — establishing
//! the schema and transport pattern the rest of the pipeline reuses.
//!
//! # Determinism and demo builds
//!
//! Capture is a read-only projection off authoritative state, so enabling it
//! never moves the #894 digest. The counters are always-on on every target; only
//! the JSON publish is flag-gated. Demo builds keep cfg-stripping the debug
//! *surface* (the WASM toggle export and its client route) exactly as the legacy
//! overlays do; the counters, being invisible and inert to the sim, stay.

pub mod payload;
pub mod station_activity;

use bevy::prelude::*;

pub use payload::{
    ActivitySource, StationActivityBucket, StationActivityEntry, StationActivityPayload,
    DEBUG_SCHEMA_VERSION,
};
pub use station_activity::{
    DebugStationActivityEnabled, StationActivityCapture, StationActivityTracker,
};

/// Wires the always-on debug counters and their flag-gated JSON publish into the
/// simulation app on every target (issue #1145).
///
/// Added by `server_app::add_simulation_plugins_with`, so the browser host, the
/// headless runner and the native host all get the same counters and the same
/// capture path. The three resources it owns are declared
/// `StateClass::Presentation` (read-only diagnostic surfaces nothing in the
/// fixed tick reads), which is what keeps the authoritative-state enumeration
/// guard green without folding them into the digest.
pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};

        app.init_resource::<StationActivityTracker>()
            .init_resource::<DebugStationActivityEnabled>()
            .init_resource::<StationActivityCapture>();

        // The observability surfaces are read-only projections nothing in the
        // fixed tick reads back, so they are digest EXCLUSIONS, not authoritative
        // state — see `crate::authoritative` and the enumeration guard.
        app.declare_state::<StationActivityTracker>(
            StateClass::Presentation,
            "debug-station-activity",
        )
        .declare_state::<DebugStationActivityEnabled>(
            StateClass::Presentation,
            "debug-station-activity",
        )
        .declare_state::<StationActivityCapture>(
            StateClass::Presentation,
            "debug-station-activity",
        );

        // Counters: always-on, after the whole tick's admission has run (the
        // same window the unrouted-command lint observes), gated only on there
        // being a run in progress.
        app.add_systems(
            FixedUpdate,
            station_activity::record_station_activity
                .after(crate::sim_sets::SimSet::Broadcast)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress)),
        );

        // Publish: flag-gated, after the counters have taken this tick's tally.
        app.add_systems(
            FixedUpdate,
            station_activity::publish_station_activity
                .after(station_activity::record_station_activity)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress))
                .run_if(|flag: Res<DebugStationActivityEnabled>| flag.0),
        );
    }
}
