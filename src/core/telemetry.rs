//! Run telemetry accumulator — the resource a run's exit summary and its
//! canonical digest both read.
//!
//! # Why this is here and not under `headless`
//!
//! It used to live in `headless::report`, beside the collector systems that
//! fill it and the report builder that consumes it. That was the right home
//! while the only reader was the headless exit summary. It stopped being the
//! right home when `crate::sim_digest` — the canonical authoritative-state
//! digest (issue #901) — became a *cross-target* artifact (issue #904): the
//! digest folds this resource's collision attribution, so a digest module
//! that has to compile for `wasm32` cannot name a type that only exists in a
//! native, `headless`-featured build.
//!
//! The alternative considered and rejected was a `cfg`-conditional
//! `fold_collisions` — real on native, a fixed "absent" marker on wasm. That
//! would have made the fold itself a function of the target, which is
//! precisely the property a cross-target digest exists to deny. Moving the
//! *type* (25 lines of plain fields) keeps one fold with one definition on
//! every target; the collector systems and the report builder stay in
//! `headless::report`, which is still where they belong.
//!
//! `headless::report` re-exports this type, so every existing
//! `headless::report::RunTelemetry` path still resolves.

use bevy::prelude::Resource;
use std::collections::BTreeMap;

use crate::core::balance::StampedBalanceEvent;

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
