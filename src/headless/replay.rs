//! `vellum_replay::Simulation` for the phoenix headless app, plus the artifact
//! a recorded run writes and the divergence sampling a replay reads back
//! (issue #901).
//!
//! # What this is, and what `command_admission::log::CommandLogReplay` is not
//!
//! `CommandLogReplay` (issue #898) models the command log's *ordering*
//! contract: it is a clock, a counter and a rolling fingerprint of the accepted
//! sequence. It is not the phoenix simulation and says so.
//!
//! [`PhoenixSim`] is. It owns a real headless `App`, `apply` pushes a
//! [`LoggedCommand`] across the **production** admission boundary — the same
//! `InboundMessage` → `admit_system_commands` → `AdmittedCommands` path a
//! browser client's message takes — and [`digest`](PhoenixSim::digest) is the
//! canonical authoritative-state digest of `headless::digest`, folded exactly
//! as issue #894's record decided. Nothing here re-implements a simulation
//! step; the whole of `apply` is "advance the app to the tick, then hand the
//! command to the boundary and let the boundary do what it always does".
//!
//! # How the trait's shape maps onto a fixed-tick simulation
//!
//! `vellum_replay` has no notion of a tick — deliberately, per #898's Cargo
//! note. The mapping:
//!
//! * **`apply`** advances to the command's tick and then submits it. It submits
//!   *without* stepping afterwards, so two commands stamped for the same tick
//!   both reach the boundary before that tick runs — which is the within-tick
//!   ordering the log records and a replay must reproduce.
//! * **`is_over`** is `GamePhase::GameOver` **or** the configured run length
//!   being spent. The second half is what lets `replay_into` stop cleanly when
//!   a log outlives the run it is being replayed into, rather than the sim
//!   inventing a rejection for it.
//! * **`needs_continuation`/`continue_step`** drive the *tail* — every tick
//!   after the last command, up to the run length. `needs_continuation` is
//!   false until every command in the log has been applied, which is why
//!   [`PhoenixSim::new`] is told how long the log is. That is not a leak: this
//!   type is built *from* the replay artifact, so the log's length is something
//!   it already knows. Without the guard, `replay_into`'s pump-before-each-apply
//!   would run the whole tail before the first command was ever submitted.
//! * **Rejection is pure.** The one rejection is a tick that goes backwards,
//!   and it is decided before anything steps, submits, or draws — so a refused
//!   command leaves the digest bit-identical. `vellum_replay::contract::
//!   rejection_is_pure` checks that against this real implementation.
//!
//! # Divergence sampling
//!
//! Sampling lives inside [`PhoenixSim::continue_step`] (strictly: inside the
//! one private `step` both `continue_step` and `apply`'s advance loop call), so
//! it needs no driver callback and a run that does not ask for it pays nothing
//! — `DigestLedger::samples` is `false` for every tick when the interval is
//! `0`, and no digest is computed at all.
//!
//! What that buys is the difference between "these two runs differ" and "these
//! two runs agreed through tick 240 and disagreed by tick 250". Only the second
//! is a window somebody can read a log over.
//!
//! # A recorded run and a replayed run take the same path
//!
//! Both are `replay_into` over a `PhoenixSim`. A recording run is driven with
//! the commands its harness wants injected (a headless run with nobody
//! connected has no human input of its own, and the in-process AI emissions of
//! `emit_ai_command` are deliberately *not* logged — a replay re-derives them
//! from the seed); it then reads the `CommandLog` the app itself recorded and
//! writes that, with its ledger, as the artifact. A replay run is driven with
//! the artifact's log. One code path, so a divergence can never be an artifact
//! of the two runs being driven differently.
//!
//! # An attended run's limitation — stated in advance, not discovered later
//!
//! Everything above is true of a headless run because a headless run has
//! nobody connected: every station is Backfill from the start, and Backfill
//! replaying into Backfill is the same script twice. That stops being true the
//! moment this machinery is pointed at a run with a human at a station (a
//! browser host, once #862's snapshot boundary lets one be recorded at all).
//!
//! A session held by a human is not in the log's own vocabulary — the log
//! records the *commands* the network boundary admitted, never *who held
//! which station when*. So a replay of an attended recording rebuilds the
//! world with every station on Backfill (there is no session state to say
//! otherwise), and the AI now emits decisions for the station the human held
//! that the recording never asked for. Replaying the human's own logged
//! commands on top of that does not fix it: the AI's in-process emissions for
//! that station are exactly the class of thing this module deliberately does
//! not log (see above — they are meant to be *re-derived*, not replayed
//! twice), so an attended run diverges from its own recording BY DESIGN, not
//! by a bug in this module, until the snapshot boundary (#862) carries enough
//! session/station-holder state for a replay to know which stations to leave
//! off Backfill. Nothing here can fix that from this side of the boundary;
//! it is recorded so a reviewer hitting this limitation does not spend time
//! re-discovering it.
//!
//! # Refusals are named, not swallowed
//!
//! A command `apply` submits across the boundary can still be refused by
//! `command_admission::is_command_authorized` — the driver has no way to ask
//! in advance, and previously that refusal was silent: the run continued, the
//! digest read whatever state resulted, and nothing said a command that used
//! to be admitted no longer was. [`DigestLedger::refused`] closes that: it is
//! submitted-minus-admitted, computed once at [`PhoenixSim::seal`] from the
//! same `CommandLog` a recording writes down. `phoenix-headless --record` and
//! `--replay` both print it, and a mismatch between a recorded and a replayed
//! ledger's `refused` count is exactly the case above — a command still in the
//! script but no longer accepted by the boundary it crosses.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::command_admission::ai_emit::AI_BACKFILL_TOKEN;
use crate::command_admission::log::{LoggedCommand, ReplayRejection};
use crate::command_admission::CommandLog;
use crate::console_bridge::LOCAL_CONSOLE_TOKEN;
use crate::headless::args::HeadlessArgs;
use crate::headless::digest::{state_digest, DigestLedger, Divergence};
use crate::lobby::InboundMessage;
use crate::messages::{ClientMessage, GamePhase, SystemId};
use crate::sim_tick::SimTick;

/// The credential a re-injected command is submitted under.
///
/// The log deliberately never carries one — a session token is a bearer
/// credential, and `command_admission::log` says why it stays behind. So a
/// driver has to pick a token from the *target* alone, and this is the whole
/// of that choice: it does not read the entry's [`ShipKey`] at all.
///
/// **What this does NOT do, stated plainly because the name invites the wrong
/// reading:** this is not "route the command to `entry.ship`". The driver
/// picks a credential from `target` alone — `GOD_MODE_SYSTEM_ID` gets
/// `LOCAL_CONSOLE_TOKEN`, everything else gets `AI_BACKFILL_TOKEN` — and
/// `admit_system_commands`' own routing rule then sends an `AI_BACKFILL_TOKEN`
/// command to the `LocalShip` unconditionally (an unregistered `ai:` token
/// always falls back there; see `command_admission::mod`'s routing doc). The
/// entry's `ShipKey` is carried on [`LoggedCommand`] and asserted on in the
/// tests below, but nothing in this replay path reads it to choose a
/// destination — today every production log entry names the `LocalShip`
/// anyway (a headless run has exactly one ship whose commands cross the
/// network boundary), so the gap is latent rather than observed. Routing a
/// replayed command to an arbitrary named ship — reading `ShipKey` back out
/// and resolving *it* to a credential/destination, rather than hard-coding the
/// one-ship assumption here — is issue #854's work (per-ship P2P routing),
/// not this one's.
///
/// God Mode (issue #900) is authorized only for the host console; everything
/// else in an unattended run is driven by Backfill AI, which answers to
/// `AI_BACKFILL_TOKEN`. If a future target needs a third credential, this is
/// the one function that has to learn about it.
pub fn replay_token_for(target: &SystemId) -> &'static str {
    if target.0 == crate::system_registry::GOD_MODE_SYSTEM_ID {
        LOCAL_CONSOLE_TOKEN
    } else {
        AI_BACKFILL_TOKEN
    }
}

/// A headless phoenix run, driven by a command log.
pub struct PhoenixSim {
    app: App,
    /// Frames (`App::update()` calls) this run is allowed, mirroring
    /// `headless::run`'s `max_ticks`. Frames rather than logical ticks because
    /// that is what `--ticks` has always counted; the *sampling* cadence below
    /// is in logical ticks, which is what a digest is meaningful against.
    max_frames: u64,
    frames: u64,
    /// How many commands the log being replayed holds. See the module docs for
    /// why the tail needs this.
    expected_commands: usize,
    applied: usize,
    /// How many commands actually reached the production admission boundary
    /// (the `Messages<InboundMessage>` write in `apply`'s submit branch) —
    /// distinct from `applied`, which also counts a command `apply` handled by
    /// deciding the run had already ended before it. See [`Self::refused`].
    submitted: usize,
    /// Whether to drive the tail after the last command. True for every real
    /// run; see [`PhoenixSim::new_tailless`] for the one case that wants it off.
    tail: bool,
    ledger: DigestLedger,
}

impl PhoenixSim {
    /// Build a run from `args`, ready to be driven by a log of `commands` long,
    /// sampling a digest every `checkpoint_every` logical ticks (`0` = off).
    pub fn new(
        args: &HeadlessArgs,
        expected_commands: usize,
        checkpoint_every: u64,
    ) -> Result<Self, crate::headless::BuildError> {
        Self::build(args, expected_commands, checkpoint_every, true)
    }

    /// A run that stops the moment its last command has been submitted, rather
    /// than driving the tail out to the configured length.
    ///
    /// This exists for exactly one caller: `vellum_replay::contract::check_all`.
    /// Its `refusals_stay_out_of_the_log` check compares a simulation driven
    /// through `Log::apply` — which submits commands and never pumps — against
    /// one driven through `replay_into`, which pumps between commands and again
    /// at the end. For a turn-based game those two land in the same state. For a
    /// fixed-tick simulation they do not: the second has simulated the whole
    /// tail and the first has not, so their digests differ for a reason that is
    /// a property of vellum's two entry points rather than of phoenix keeping or
    /// breaking the contract.
    ///
    /// Turning the tail off makes the two comparable without weakening anything
    /// the contract actually asserts: the same script still has to reach the
    /// same digest twice, a refusal still has to leave the digest untouched, and
    /// a refused command still has to stay out of the log.
    pub fn new_tailless(
        args: &HeadlessArgs,
        expected_commands: usize,
    ) -> Result<Self, crate::headless::BuildError> {
        Self::build(args, expected_commands, 0, false)
    }

    fn build(
        args: &HeadlessArgs,
        expected_commands: usize,
        checkpoint_every: u64,
        tail: bool,
    ) -> Result<Self, crate::headless::BuildError> {
        let mut app = crate::headless::build_headless_app(args)?;
        // `headless::run_sampled` does this before its loop; a driver that
        // steps the app by hand has to do it too, or `Startup` never runs.
        app.finish();
        app.cleanup();
        Ok(Self {
            app,
            max_frames: args.max_ticks,
            frames: 0,
            expected_commands,
            applied: 0,
            submitted: 0,
            tail,
            ledger: DigestLedger::new(checkpoint_every),
        })
    }

    /// The current logical tick.
    pub fn tick(&self) -> u64 {
        self.app
            .world()
            .get_resource::<SimTick>()
            .map_or(0, |t| t.0)
    }

    /// The log the app itself recorded — what a recording run writes down.
    pub fn recorded_log(&self) -> CommandLog {
        self.app
            .world()
            .get_resource::<CommandLog>()
            .cloned()
            .unwrap_or_default()
    }

    /// How many submitted commands never made it into the `CommandLog` —
    /// i.e. the authority gate refused them (issue #901 review).
    ///
    /// Cheap and honest rather than exact bookkeeping per command: `submitted`
    /// is a plain counter incremented at the one site this type ever writes to
    /// `Messages<InboundMessage>`, and `recorded_log().len()` is the same
    /// `CommandLog` a recording run writes down — the same observable
    /// `--record` already surfaces via [`Self::recorded_log`], read a second
    /// time rather than re-derived. The alternative — walking
    /// `AdmittedCommands` per ship right
    /// after admission to count what landed — would need a system hook this
    /// driver does not otherwise have any reason to add, for the same number.
    ///
    /// This can only ever undercount refusals from OUTSIDE this driver's own
    /// script (there are none: a headless run's only inbound traffic is what
    /// `apply` submits), and it correctly reads `0` for a run that submitted
    /// nothing, which is why it is safe to call before every command has been
    /// applied as well as after.
    fn refused(&self) -> u64 {
        (self.submitted.saturating_sub(self.recorded_log().len())) as u64
    }

    /// The sampled digests, with the final digest filled in.
    ///
    /// Sealing computes the final digest once, at the point the caller says the
    /// run is finished — not on every step, and not lazily somewhere a later
    /// mutation could invalidate it.
    pub fn seal(&mut self) -> DigestLedger {
        self.ledger.final_digest = state_digest(&self.app);
        self.ledger.refused = self.refused();
        self.ledger.clone()
    }

    /// [`seal`](Self::seal), consuming the run.
    pub fn into_ledger(mut self) -> DigestLedger {
        self.seal()
    }

    /// Borrow the app — for tests that want to look at end state directly.
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// Take the app back, so a driven run can still produce the ordinary exit
    /// report through `headless::build_report`.
    pub fn into_app(self) -> App {
        self.app
    }

    /// One frame, plus the periodic digest sample.
    ///
    /// The single place the app is stepped, so there is exactly one place
    /// sampling can be attached and no path that advances without being seen.
    fn step(&mut self) {
        self.app.update();
        self.frames += 1;
        // The `0 = off` branch: no digest is computed, so an unsampled run
        // steps identically to one built before this module existed.
        let tick = self.tick();
        if self.ledger.samples(tick) {
            let digest = state_digest(&self.app);
            self.ledger.record(tick, digest);
        }
    }

    /// Frames left in the configured run.
    fn frames_left(&self) -> bool {
        self.frames < self.max_frames
    }
}

impl vellum_replay::Simulation for PhoenixSim {
    type Command = LoggedCommand;
    type Rejection = ReplayRejection;

    /// Advance to the command's tick, then push it across the production
    /// admission boundary.
    ///
    /// The one rejection is decided FIRST, before anything advances or is
    /// submitted, so a refusal leaves the state — and therefore the digest —
    /// exactly as it was.
    ///
    /// The advance is `while tick < command.tick`, not `!=`: a frame can run
    /// zero fixed steps (Bevy's first `update()` establishes the time baseline
    /// and steps nothing) or several (when `--hz` runs slower than
    /// `sim_tick_hz`), so the loop stops at the first tick that has *reached*
    /// the stamp, which is the same rule the recorder's own injection queue
    /// drains on.
    fn apply(&mut self, command: &LoggedCommand) -> Result<(), ReplayRejection> {
        let clock = self.tick();
        if command.tick < clock {
            return Err(ReplayRejection::TickWentBackwards {
                stamped: command.tick,
                clock,
            });
        }

        while self.tick() < command.tick && self.frames_left() && !is_game_over(&self.app) {
            self.step();
        }

        // A log that outlives the run it is being replayed into: the run is
        // over, so `replay_into` stops on the next iteration. Not a rejection —
        // the command was never wrong, the run was simply shorter.
        if !self.frames_left() || is_game_over(&self.app) {
            self.applied += 1;
            return Ok(());
        }

        // Submitted, not stepped. Two commands stamped for the same tick both
        // land before that tick runs, in log order — the within-tick ordering
        // the log records.
        self.app
            .world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: replay_token_for(&command.target).into(),
                msg: ClientMessage::ControlSystem {
                    target: command.target.clone(),
                    payload: command.payload.clone(),
                },
            });
        self.applied += 1;
        self.submitted += 1;
        Ok(())
    }

    fn is_over(&self) -> bool {
        is_game_over(&self.app) || self.frames >= self.max_frames
    }

    /// The canonical authoritative-state digest of `headless::digest`, folded
    /// per issue #894's record. `&self` is why that module walks the world
    /// through `World::try_query` rather than `World::query`.
    fn digest(&self) -> u64 {
        state_digest(&self.app)
    }

    /// True only once every command has been applied — see the module docs.
    /// Then it drives the tail to the run's final tick.
    fn needs_continuation(&self) -> bool {
        self.tail && self.applied >= self.expected_commands && self.frames_left() && !self.is_over()
    }

    fn continue_step(&mut self) {
        self.step();
    }
}

fn is_game_over(app: &App) -> bool {
    app.world()
        .get_resource::<State<GamePhase>>()
        .is_some_and(|phase| phase.get() == &GamePhase::GameOver)
}

// ── The replay artifact ──────────────────────────────────────────────────────

/// Bumped whenever the artifact's own shape changes. A replay refuses an
/// artifact it does not recognise rather than reading it wrongly — the same
/// argument `vellum_digest::ShareCodec` makes for versioning a prefix.
///
/// `2` (from `1`): added `side_a`/`side_b`. A version-1 artifact recorded a
/// duel run's ship rosters nowhere — `world_path` named `duel.toml`, but the
/// CLI ship lists that filled its slots were never captured, so replaying one
/// silently ran the *unfilled* duel world (every slot deleted, no escorts, no
/// enemies) rather than the roster that was actually recorded. `from_ron`
/// refuses a `1` outright rather than defaulting the new fields to empty,
/// because an empty `side_a`/`side_b` is indistinguishable from a real
/// `--world`-only run with no duel sides, and reading it that way would
/// silently replay the wrong scenario instead of refusing to.
pub const ARTIFACT_VERSION: u32 = 2;

/// Everything a second run needs to reproduce the first: the run's setup, the
/// commands it accepted, and the digests it passed through.
///
/// RON rather than JSON because AGENTS.md constraint 1 keeps `serde_json` in
/// `codec.rs`, and `ron` is already a native dependency (the perf baselines
/// read it). The artifact is native-only anyway — `headless` does not exist
/// under wasm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayArtifact {
    pub version: u32,
    /// The master seed. A replay run is `--seed <this>` and nothing else; every
    /// other RNG position is re-derived.
    pub seed: u64,
    pub world_path: String,
    pub ship_path: String,
    /// `--side-a`/`--side-b` ship rosters (issue #844), captured so a duel
    /// run's replay fills the same slots the recording did. Empty for a plain
    /// `--world`/`--ship` run, exactly like [`HeadlessArgs::side_a`]/
    /// [`HeadlessArgs::side_b`]. Added in [`ARTIFACT_VERSION`] 2 — see that
    /// constant for what a version-1 artifact was missing without it.
    pub side_a: Vec<String>,
    pub side_b: Vec<String>,
    /// Frame count and frame period, so a replay drives the app at exactly the
    /// pacing the recording did.
    pub max_ticks: u64,
    pub dt: f64,
    /// Everything the network boundary admitted, in apply order.
    pub log: CommandLog,
    /// The sampled digests and the final one.
    pub ledger: DigestLedger,
}

/// Why an artifact could not be read or used.
#[derive(Debug)]
pub enum ArtifactError {
    Io(String),
    Parse(String),
    /// Written by a build whose artifact shape this one does not know.
    Version {
        found: u32,
        expected: u32,
    },
    /// The recording run had no seed, so nothing can reproduce it.
    Unseeded,
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtifactError::Io(e) => write!(f, "replay artifact io error: {e}"),
            ArtifactError::Parse(e) => write!(f, "replay artifact did not parse: {e}"),
            ArtifactError::Version { found, expected } => write!(
                f,
                "replay artifact is version {found}; this build writes and reads \
                 version {expected}. Re-record it rather than replaying it wrongly."
            ),
            ArtifactError::Unseeded => write!(
                f,
                "a replay artifact needs a seed, and this run has none. Record \
                 with --seed: without one the second run re-draws every stream \
                 from the OS and reproduces nothing."
            ),
        }
    }
}

impl std::error::Error for ArtifactError {}

impl ReplayArtifact {
    /// Capture what a finished recording run leaves behind.
    ///
    /// `args.seed` is required: an artifact whose seed came from the OS names a
    /// run nothing can re-derive, and writing one would be a file that looks
    /// replayable and is not.
    pub fn capture(
        args: &HeadlessArgs,
        log: CommandLog,
        ledger: DigestLedger,
    ) -> Result<Self, ArtifactError> {
        Ok(Self {
            version: ARTIFACT_VERSION,
            seed: args.seed.ok_or(ArtifactError::Unseeded)?,
            world_path: args.world_path.clone(),
            ship_path: args.ship_path.clone(),
            side_a: args.side_a.clone(),
            side_b: args.side_b.clone(),
            max_ticks: args.max_ticks,
            dt: args.dt,
            log,
            ledger,
        })
    }

    /// The arguments a replay of this artifact must run under.
    ///
    /// Derived from the artifact rather than from the replaying process's own
    /// command line, so a replay cannot silently run a different world, hull,
    /// length or pacing from the recording. `deterministic` is implied by the
    /// seed exactly as `parse_args` implies it.
    pub fn replay_args(&self) -> HeadlessArgs {
        HeadlessArgs {
            world_path: self.world_path.clone(),
            ship_path: self.ship_path.clone(),
            side_a: self.side_a.clone(),
            side_b: self.side_b.clone(),
            max_ticks: self.max_ticks,
            dt: self.dt,
            seed: Some(self.seed),
            deterministic: true,
            ..Default::default()
        }
    }

    pub fn to_ron(&self) -> Result<String, ArtifactError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| ArtifactError::Parse(e.to_string()))
    }

    pub fn from_ron(text: &str) -> Result<Self, ArtifactError> {
        // Read the version FIRST, against a probe shape that names only that
        // one field, before attempting to deserialise the rest of a shape this
        // build might not know — the same "versioned prefix" argument
        // `vellum_digest::ShareCodec` makes, applied to a whole-document RON
        // artifact rather than a byte prefix. Without this, a version-1
        // artifact (missing `side_a`/`side_b`, added in version 2) would fail
        // deserialising into the CURRENT shape with a generic parse error
        // instead of the specific, actionable "re-record it" message
        // `ArtifactError::Version` gives.
        #[derive(Deserialize)]
        struct ArtifactVersion {
            version: u32,
        }
        let ArtifactVersion { version } =
            ron::from_str(text).map_err(|e| ArtifactError::Parse(e.to_string()))?;
        if version != ARTIFACT_VERSION {
            return Err(ArtifactError::Version {
                found: version,
                expected: ARTIFACT_VERSION,
            });
        }
        let artifact: Self =
            ron::from_str(text).map_err(|e| ArtifactError::Parse(e.to_string()))?;
        Ok(artifact)
    }

    pub fn write(&self, path: &str) -> Result<(), ArtifactError> {
        std::fs::write(path, format!("{}\n", self.to_ron()?))
            .map_err(|e| ArtifactError::Io(format!("{path:?}: {e}")))
    }

    pub fn read(path: &str) -> Result<Self, ArtifactError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ArtifactError::Io(format!("{path:?}: {e}")))?;
        Self::from_ron(&text)
    }
}

// ── The drivers ──────────────────────────────────────────────────────────────

/// Anything that stopped a record or a replay.
#[derive(Debug)]
pub enum ReplayError {
    Build(String),
    /// A command the simulation refused. Names the index into the log, so the
    /// failure names the command rather than the run.
    Refused {
        at_command: usize,
        why: String,
    },
    Artifact(String),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Build(e) => write!(f, "{e}"),
            ReplayError::Refused { at_command, why } => {
                write!(f, "replay diverged at command #{at_command}: {why}")
            }
            ReplayError::Artifact(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReplayError {}

impl From<crate::headless::BuildError> for ReplayError {
    fn from(e: crate::headless::BuildError) -> Self {
        ReplayError::Build(e.to_string())
    }
}

impl From<ArtifactError> for ReplayError {
    fn from(e: ArtifactError) -> Self {
        ReplayError::Artifact(e.to_string())
    }
}

/// Drive a run under `args`, injecting `script` through the production boundary,
/// and capture what it leaves behind.
///
/// `script` is normally empty: an unattended headless run has no human input,
/// and the AI's own emissions never cross the boundary the log records. It is a
/// parameter because a *test* needs a run whose log is not empty, and driving
/// the recording through the same `replay_into` the replay uses is what makes
/// the comparison mean something.
pub fn drive_run(
    args: &HeadlessArgs,
    script: &[LoggedCommand],
    checkpoint_every: u64,
) -> Result<PhoenixSim, ReplayError> {
    let mut sim = PhoenixSim::new(args, script.len(), checkpoint_every)?;
    vellum_replay::replay_into(&mut sim, script).map_err(|fault| ReplayError::Refused {
        at_command: fault.at_command,
        why: fault.rejection.to_string(),
    })?;
    Ok(sim)
}

/// Replay an artifact: rebuild the run from its own recorded setup, drive it
/// with its own log, and return the ledger the second run produced.
pub fn replay_artifact(
    artifact: &ReplayArtifact,
    checkpoint_every: u64,
) -> Result<DigestLedger, ReplayError> {
    let args = artifact.replay_args();
    let commands = artifact.log.entries();
    let mut sim = PhoenixSim::new(&args, commands.len(), checkpoint_every)?;
    vellum_replay::replay_into(&mut sim, commands).map_err(|fault| ReplayError::Refused {
        at_command: fault.at_command,
        why: fault.rejection.to_string(),
    })?;
    Ok(sim.into_ledger())
}

/// Replay an artifact and say whether — and where — it stopped reproducing.
///
/// `Ok(None)` is a clean replay. `Ok(Some(_))` is the located answer: the tick
/// window the two runs first disagreed in.
pub fn verify_artifact(artifact: &ReplayArtifact) -> Result<Option<Divergence>, ReplayError> {
    let replayed = replay_artifact(artifact, artifact.ledger.interval)?;
    Ok(artifact.ledger.first_divergence(&replayed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_admission::ShipKey;
    use crate::messages::SystemControlPayload;

    fn artifact() -> ReplayArtifact {
        let mut ledger = DigestLedger::new(50);
        ledger.record(50, 0xaaaa);
        ledger.final_digest = 0xbbbb;
        ReplayArtifact {
            version: ARTIFACT_VERSION,
            seed: 901,
            world_path: "assets/worlds/patrol.toml".into(),
            ship_path: "assets/entities/alliance_cruiser.toml".into(),
            side_a: Vec::new(),
            side_b: Vec::new(),
            max_ticks: 260,
            dt: 1.0 / 60.0,
            log: CommandLog::default(),
            ledger,
        }
    }

    #[test]
    fn an_artifact_round_trips_through_ron() {
        let original = artifact();
        let text = original.to_ron().expect("serialises");
        assert_eq!(ReplayArtifact::from_ron(&text).expect("parses"), original);
    }

    /// A shape this build does not know must be refused, not read wrongly —
    /// this is also the guard against a real version-1 artifact silently
    /// replaying a duel run with its side lists dropped on the floor (see
    /// `ARTIFACT_VERSION`'s own doc).
    #[test]
    fn a_future_artifact_version_is_refused() {
        let mut future = artifact();
        future.version = ARTIFACT_VERSION + 1;
        let text = future.to_ron().expect("serialises");
        assert!(matches!(
            ReplayArtifact::from_ron(&text),
            Err(ArtifactError::Version { .. })
        ));
    }

    /// A version-1 artifact (no `side_a`/`side_b`) must be refused rather than
    /// read with the new fields silently defaulted to empty — an empty roster
    /// is indistinguishable from a genuine no-duel run, so defaulting would
    /// silently replay the wrong scenario.
    #[test]
    fn a_version_one_artifact_is_refused_not_defaulted() {
        // Built from the wire shape directly: a real v1 file never had
        // `side_a`/`side_b` keys at all.
        #[derive(serde::Serialize)]
        struct ArtifactV1 {
            version: u32,
            seed: u64,
            world_path: String,
            ship_path: String,
            max_ticks: u64,
            dt: f64,
            log: CommandLog,
            ledger: DigestLedger,
        }
        let v1 = ArtifactV1 {
            version: 1,
            seed: 901,
            world_path: "assets/worlds/duel.toml".into(),
            ship_path: "assets/entities/alliance_cruiser.toml".into(),
            max_ticks: 260,
            dt: 1.0 / 60.0,
            log: CommandLog::default(),
            ledger: DigestLedger::new(0),
        };
        let text =
            ron::ser::to_string_pretty(&v1, ron::ser::PrettyConfig::default()).expect("serialises");
        match ReplayArtifact::from_ron(&text) {
            Err(ArtifactError::Version { found, expected }) => {
                assert_eq!(found, 1);
                assert_eq!(expected, ARTIFACT_VERSION);
            }
            other => panic!("a v1 artifact must be refused by version, got {other:?}"),
        }
    }

    /// An unseeded run names something nothing can re-derive, so it must not
    /// produce a file that looks replayable.
    #[test]
    fn an_unseeded_run_cannot_be_captured() {
        let args = HeadlessArgs {
            seed: None,
            ..Default::default()
        };
        assert!(matches!(
            ReplayArtifact::capture(&args, CommandLog::default(), DigestLedger::new(0)),
            Err(ArtifactError::Unseeded)
        ));
    }

    /// A replay must run the recording's world, hull, length and pacing — never
    /// the replaying process's own defaults.
    #[test]
    fn replay_args_come_from_the_artifact_not_the_process() {
        let args = artifact().replay_args();
        assert_eq!(args.world_path, "assets/worlds/patrol.toml");
        assert_eq!(args.max_ticks, 260);
        assert_eq!(args.seed, Some(901));
        assert!(args.deterministic, "a seed implies a pinned scheduler");
    }

    #[test]
    fn god_mode_is_the_one_target_that_needs_the_host_console_token() {
        let god = SystemId(crate::system_registry::GOD_MODE_SYSTEM_ID.into());
        assert_eq!(replay_token_for(&god), LOCAL_CONSOLE_TOKEN);
        assert_eq!(
            replay_token_for(&SystemId("red-alert".into())),
            AI_BACKFILL_TOKEN
        );
    }

    /// The log entry shape the driver consumes, kept honest against the type.
    #[test]
    fn a_logged_command_carries_everything_a_replay_needs_to_route_it() {
        let entry = LoggedCommand {
            tick: 7,
            ship: ShipKey("uuid-1".into()),
            target: SystemId("red-alert".into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
        };
        assert!(entry.ship.is_named());
        assert_eq!(replay_token_for(&entry.target), AI_BACKFILL_TOKEN);
    }
}
