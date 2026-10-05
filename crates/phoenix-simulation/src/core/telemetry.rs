//! Optional report telemetry, independent of authoritative collision history.
//!
//! # Why this is here and not under `headless`
//!
//! It used to live in `headless::report`, beside the collector systems that
//! fill it and the report builder that consumes it. That was the right home
//! while the only reader was the headless exit summary. It stopped being the
//! right home when `crate::sim_digest` — the canonical authoritative-state
//! digest (issue #901) — became a *cross-target* artifact (issue #904): the
//! digest then folded this resource's collision attribution, so a digest module
//! that has to compile for `wasm32` cannot name a type that only exists in a
//! native, `headless`-featured build.
//!
//! Moving the type alone did not install a collector on browser hosts. #1316
//! gives authoritative attribution its own shared fixed-tick accumulator in
//! [`super::collision_history`]. This report remains optional: its frame-time
//! reader, message counts and timeline cannot affect the simulation digest.
//!
//! `headless::report` re-exports this type, so every existing
//! `headless::report::RunTelemetry` path still resolves.

use bevy::{ecs::message::MessageCursor, prelude::Resource};
use std::collections::BTreeMap;

use crate::core::balance::{BalanceEvent, StampedBalanceEvent};
use crate::core::narrative::StampedNarrativeEvent;

/// Accumulates everything the exit summary needs, tick by tick.
///
/// No longer keeps its own tick counter (issue #895 re-review): a headless
/// run's `--hz` frame rate and the world's `[global] sim_tick_hz` are
/// independent, so a per-`update()` counter folds however many logical sim
/// ticks a frame ran (2 at `--hz 30` against the shipped `sim_tick_hz = 60`)
/// into one stamp. `Res<SimTick>` (`crate::sim_tick`) is the real counter
/// every other tick-keyed artifact already keys on — read it directly at the
/// call sites in `headless::report` instead.
#[derive(Resource, Default)]
pub struct RunTelemetry {
    /// Report-only balance reader position. Restore resets this cursor without
    /// deleting shared events or changing the independent authoritative reader.
    pub balance_cursor: MessageCursor<BalanceEvent>,
    /// Count of each `ServerMessage` variant seen, keyed by variant name.
    /// `BTreeMap` so the report is byte-identical across runs.
    pub message_counts: BTreeMap<String, u64>,
    /// One JSON line per outbound message. Only populated for
    /// `ReportFormat::Ndjson` — at 10 Hz a minute of play is a lot of lines.
    pub stream: Vec<String>,
    pub capture_stream: bool,
    /// Every balance event the run produced, stamped at collection time.
    /// Always captured — unlike `stream` this is bounded by combat, not by
    /// broadcast rate, and the per-ship ledgers are built from it.
    pub balance_events: Vec<StampedBalanceEvent>,
    /// Every authored mission-timeline event the run produced (issue #1338),
    /// stamped with its monotonic sequence, fixed sim tick and derived time at
    /// collection. Always captured, like `balance_events` and for the same
    /// reason: it is bounded by what the scenario authored, not by broadcast
    /// rate, and the report's timeline projection is built from it.
    ///
    /// **Nothing authoritative reads this.** No fold stage walks report
    /// telemetry — see `crate::core::narrative`'s determinism note.
    /// Adding one here would make an after-action surface part of the
    /// authoritative digest, which is exactly what issue #1338's third
    /// acceptance criterion forbids.
    pub narrative_events: Vec<StampedNarrativeEvent>,
    /// uuid → raw `EntityName`, snapshotted as events arrive. Recorded here
    /// rather than looked up at report time because a destroyed NPC is gone
    /// from the world long before the summary is built. Stored verbatim — for
    /// TOML entities that is a strings.csv key, not display text.
    pub entity_names: BTreeMap<String, String>,
    /// uuid → faction uuid (as a string), snapshotted the same way and for the
    /// same reason as `entity_names` (#843): a ship that died mid-run is gone
    /// from the ECS, but the exit report still needs its side to bucket its
    /// damage ledger. Absent for factionless ships.
    pub entity_factions: BTreeMap<String, String>,
}
