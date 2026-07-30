//! Headless simulation runner.
//!
//! Runs the game with no window, no renderer and nobody connected, at a fixed
//! timestep, as fast as the CPU allows. With zero sessions every station
//! backfills to AI (see `spawn_game_start_entities` in `server_app`), so the
//! player ship flies itself — which is what makes an unattended run meaningful.
//!
//! Entry point is the `phoenix-headless` binary; everything here lives in the
//! library so it can be unit-tested the way the rest of the crate is.

pub mod app;
pub mod args;
pub mod duel;
pub mod perf;
pub mod report;

pub use app::{build_headless_app, run, run_sampled, BuildError};
pub use args::{parse_args, HeadlessArgs, ParseOutcome, ReportFormat, HELP};
pub use duel::{apply_duel_sides, resolve_template, DuelError};
pub use perf::{baseline_path, load_baseline, TickSampler};
pub use report::{build_report, RunReport};
