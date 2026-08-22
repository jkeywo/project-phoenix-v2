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
/// The canonical authoritative-state digest (issue #901).
///
/// An alias rather than a module of its own: the implementation moved out to
/// `crate::sim_digest` (issue #904) so the wasm build — which has no
/// `headless` module at all — folds through the identical code a native run
/// does. Every existing `headless::digest::…` path still resolves.
pub use crate::sim_digest as digest;
pub mod duel;
pub mod fingerprint;
pub mod replay;
pub mod report;

pub use app::{
    build_headless_app, build_headless_app_with, run, run_sampled, BuildError,
    SimRegistrationOverrides,
};
pub use args::{parse_args, HeadlessArgs, ParseOutcome, ReportFormat, HELP};
pub use digest::{state_digest, world_digest, DigestLedger, Divergence, FoldKey, Namespace};
pub use duel::{apply_duel_sides, resolve_template, DuelError, DuelTemplateLoader};
pub use fingerprint::{fingerprint, RunFingerprint};
pub use replay::{
    replay_artifact, verify_artifact, ArtifactError, PhoenixSim, ReplayArtifact, ReplayError,
};
pub use report::{build_report, RunReport};
