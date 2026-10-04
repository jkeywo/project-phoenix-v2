//! Command-line parsing for `phoenix-headless`.
//!
//! Native-only Clap declarations provide syntax and help; domain validation
//! remains a non-exiting pure function over an argument iterator.

use crate::headless::duel::MAX_SIDE;
use crate::logging::{parse_log_entities, parse_log_spec, LogFilterConfig};

/// Default frame rate the harness drives the app at. Matches the 60 Hz rAF
/// rate of the browser host AND the default `[global] sim_tick_hz`, so by
/// default each `update()` advances exactly one logical sim tick and headless
/// traces line up with what a player would have seen.
pub const DEFAULT_HZ: f64 = 60.0;

const DEFAULT_WORLD: &str = "assets/worlds/default.toml";
/// World `--side-a`/`--side-b` imply when `--world` is absent.
///
/// The slot seam those flags drive only exists in this world, so defaulting to
/// `default.toml` meant `--side-a cruiser --side-b destroyer` loaded a world
/// with nothing to generate into and ran a combat-free 300s draw that reads like
/// a real balance result. An explicit `--world` still wins — a user may have
/// authored their own duel-shaped world — and a world carrying no
/// `duel::SLOT_MARKER` is now rejected by `duel::apply_duel_sides` rather than
/// silently ignored.
const DUEL_WORLD: &str = "assets/worlds/duel.toml";
const DEFAULT_SHIP: &str = "assets/entities/alliance_cruiser.toml";
/// The scenario a capture is filed under when `--perf-scenario` is absent.
/// Named for the setup, not the run: only captures of the same scenario are
/// comparable, and `perf/baselines/<scenario>.ron` is where its expectations
/// live.
const DEFAULT_PERF_SCENARIO: &str = "headless-default";

use clap::{CommandFactory, Parser};

#[derive(Parser)]
#[command(
    name = "phoenix-headless",
    args_override_self = true,
    about = "Run the simulation without a window or renderer, on AI Backfill at a fixed step"
)]
struct Cli {
    #[arg(
        long,
        help = "World TOML to load    [default: assets/worlds/default.toml]"
    )]
    world: Option<String>,
    #[arg(
        long,
        help = "Player ship template  [default: assets/entities/alliance_cruiser.toml]"
    )]
    ship: Option<String>,
    #[arg(
        long,
        help = "Comma-separated ship classes for side A (max 5). The first is the player ship; the rest are NPC escorts. Mutually exclusive with --ship."
    )]
    side_a: Option<String>,
    #[arg(
        long,
        help = "Comma-separated ship classes for side B (max 5), all NPCs hostile to side A. Names resolve in order: alliance_<name>.toml, then <name>.toml (both under assets/entities/), then <name> as a literal path. e.g. --side-a cruiser --side-b destroyer Either flag defaults --world to the duel harness (assets/worlds/duel.toml) — the only world carrying the `// duel:slots` marker, below which the side_a_*/ side_b_* slot drivers are regenerated. An explicit --world still wins, but a world with no such marker is rejected rather than silently run as-is. Without either flag the world's own authored roster runs untouched."
    )]
    side_b: Option<String>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Frame rate the harness drives the app at, in frames per sim-second [default: 60]. Since issue #895 the SIMULATION advances on the world's [global] sim_tick_hz (default 60) inside Bevy's fixed loop, so this flag chooses how much virtual time each update() advances, not how often the sim thinks — any --hz covers the same logical ticks per sim-second — and since issue #896 that includes rapier, which steps once per logical tick too.",
        value_parser = |value: &str| parse_positive_f64(value, "--hz")
    )]
    hz: Option<f64>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Frame period; mutually exclusive with --hz",
        value_parser = |value: &str| parse_positive_f64(value, "--dt")
    )]
    dt: Option<f64>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Stop after N frames. Named before issue #895, when a frame and a sim tick were the same thing; it counts update() calls, so the LOGICAL ticks a run covers are N x sim_tick_hz / --hz.",
        value_parser = whole_number
    )]
    ticks: Option<u64>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Stop after N seconds of simulated time (If neither is given, the run stops at 60 sim-seconds.)",
        value_parser = |value: &str| parse_positive_f64(value, "--sim-seconds")
    )]
    sim_seconds: Option<f64>,
    #[arg(
        long,
        help = "Category levels, e.g. 'info,ai=debug,admit=trace' Categories: ai helm weapons shields damage power sensors comms repair nav captain lobby admit world regions physics broadcast assets config Levels: off error warn info debug trace"
    )]
    log: Option<String>,
    #[arg(
        long,
        help = "Only log events for these entities, by display name. Comma-separated; matched exactly, then case-insensitively as a substring. e.g. 'Ironveil,Ashrender'"
    )]
    log_entity: Option<String>,
    #[arg(
        long,
        help = "Write the exit summary here instead of stdout ('-' for stdout)"
    )]
    report: Option<String>,
    #[arg(
        long,
        help = "'json' (exit summary only) or 'ndjson' (also stream every outbound message, one JSON object per line) [default: json]"
    )]
    report_format: Option<String>,
    #[arg(long, help = "Exit non-zero if the run ends in GamePhase::GameOver")]
    fail_on_game_over: bool,
    #[arg(
        long,
        help = "Sample per-tick and whole-run wall time and write the capture JSON here ('-' for stdout). Absent, no measurement is collected at all. Sampling brackets the harness loop from outside, so a measured run steps identically to an unmeasured one. FixedUpdate system spans add observed per-phase intervals, not inclusive phase wall time. Coverage and unattributed work are retained in <PATH>.phases.json (stderr with '-')."
    )]
    perf_capture: Option<String>,
    #[arg(
        long,
        help = "Scenario the capture is filed under, and the baseline it is compared against, at perf/baselines/<N>.ron [default: headless-default]. A missing baseline is not an error: the capture is still written. Comparison is warnings-only and never changes the exit code."
    )]
    perf_scenario: Option<String>,
    #[arg(
        long,
        help = "Write a replay artifact here: the seed, the world, hull, length and pacing, the command log the run accepted, and the digests it passed through. Requires --seed — an artifact with an OS-drawn seed names a run nothing can re-derive."
    )]
    record: Option<String>,
    #[arg(
        long,
        help = "Replay an artifact and report whether the second run reproduced the first. Everything the run needs comes from the ARTIFACT, so --world, --ship, --side-a, --side-b, --seed, --ticks, --sim-seconds, --dt and --hz are rejected alongside it rather than accepted and quietly ignored. Exit code 4 when the replay diverges, and the message names the tick window it first disagreed in rather than merely that it disagreed. --log/--log-entity and --report/--report-format are NOT rejected, but they are inert here: a replay prints a verdict, not the ordinary run report or per-tick log output those flags shape, so giving them changes nothing about what --replay does."
    )]
    replay: Option<String>,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Sample an authoritative-state digest every N LOGICAL ticks [default: 0 = off, and off costs nothing: no digest is computed at all]. The samples are what turn 'these two runs differ' into 'they agreed at tick 240 and disagreed by tick 250'. A --replay run samples at the interval its artifact recorded, so it compares like with like; this flag chooses the interval a --record run writes down.",
        value_parser = whole_number
    )]
    digest_every: Option<u64>,
    #[arg(
        long,
        help = "Measure console input-to-feedback latency and report the per-action p50/p75/max under 'console_latency' [default: off, and off costs nothing: no wall-clock reading is taken at all]. A headless run has no console client, so the only segment it can measure is the simulation's own admission-to-broadcast service window — the slice of a player's round trip the host is answerable for. Implied by --perf-capture, which files the same samples under the 'sim.console_ack' perf metric."
    )]
    console_latency: bool,
    #[arg(
        long,
        help = "Use Bevy's SingleThreaded executor for fixed schedules and their shared StateTransition, plus a one-thread task pool. A fixed timestep alone is not reproducibility."
    )]
    deterministic: bool,
    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Master seed for the simulation RNG (u64). Implies --deterministic. Every RNG site — damage distribution, region effects, entity UUIDs — derives its own stream from this, so two runs with the same seed produce byte-identical reports. Byte-identical includes the timing fields: a --seed run reports wall_seconds, ticks_per_second and speedup_vs_realtime as 0, because those are measured off the host clock and would otherwise be the only lines that differ between two identical --seed runs. Only --seed gets this, because only --seed pins the scheduler: zeroed timings mean 'this run is replayable'. World-TOML-seeded and unseeded runs report the real figures. Precedence: --seed, then the world TOML's [global] seed, then a seed drawn from the OS. The resolved seed and its source are always in the report, so you can replay any run by feeding that seed back in as --seed. Only --seed pins the scheduler, so a run that took its seed from the world TOML or the OS was not itself reproducible: replaying it with --seed is a single-threaded re-run of the same scenario, and may not match what you saw. CAVEAT: the contract is same binary, same machine. Floating-point differences across CPUs or compiler versions can still diverge.",
        value_parser = whole_number
    )]
    seed: Option<u64>,
}

/// Help generated from the same declarations used for parsing.
pub fn help() -> String {
    Cli::command().render_long_help().to_string()
}

/// How the run reports itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ReportFormat {
    /// A single JSON summary object at exit.
    #[default]
    Json,
    /// The summary, plus one JSON object per outbound message as it happens.
    Ndjson,
}

/// Fully-resolved headless configuration.
#[derive(Clone, Debug)]
pub struct HeadlessArgs {
    pub world_path: String,
    pub ship_path: String,
    /// Frame period in seconds — the virtual time one `update()` advances.
    /// Since issue #895 this is NOT the simulation's step: the sim steps at the
    /// world's `[global] sim_tick_hz` inside Bevy's fixed loop.
    pub dt: f64,
    /// FRAME count at which the run stops (`--ticks`, named before #895 split
    /// frames from logical ticks). Always resolved — `--sim-seconds` is
    /// converted here so the run loop only ever counts frames.
    pub max_ticks: u64,
    pub log: LogFilterConfig,
    /// The raw `--log` spec, forwarded to `LogPlugin`'s own `EnvFilter` so
    /// bevy-internal events roughly agree with our categories.
    pub log_spec: String,
    pub report_path: Option<String>,
    pub report_format: ReportFormat,
    pub fail_on_game_over: bool,
    /// Select serial fixed executors and a one-thread pool. Implied by `seed`, which needs a
    /// fixed system execution order to be worth anything.
    pub deterministic: bool,
    /// Master RNG seed from `--seed`. `None` falls through to the world TOML's
    /// `[global] seed`, then to a seed drawn from the OS — resolved in
    /// `headless::app`, which is where the world config is in scope.
    pub seed: Option<u64>,
    /// Side-A ship list from `--side-a` (issue #844). Empty when the flag is
    /// absent — a plain `--world` run leaves the duel transform off. `side_a[0]`
    /// is the player ship; `side_a[1..]` fill NPC escort slots. Resolved to
    /// template paths and applied in `headless::app`.
    pub side_a: Vec<String>,
    /// Side-B ship list from `--side-b` (issue #844). Empty when absent. All
    /// entries fill NPC slots on the enemy side.
    pub side_b: Vec<String>,
    /// Where to write the performance capture from `--perf-capture` (issue
    /// #868). `None` leaves the harness-loop collector off entirely, so an
    /// ordinary run pays nothing for measurement it did not ask for.
    pub perf_capture_path: Option<String>,
    /// Scenario name a capture is filed under, and the baseline file it is
    /// compared against. Measurement is only meaningful between runs of the
    /// *same* scenario, so this names the setup rather than the run.
    pub perf_scenario: String,
    /// Where to write the replay artifact from `--record` (issue #901).
    /// `None` leaves the recording path off entirely, so an ordinary run is
    /// byte-for-byte the run it always was. Requires `--seed`: an artifact
    /// whose seed came from the OS names a run nothing can re-derive.
    pub record_path: Option<String>,
    /// Replay artifact to consume from `--replay` (issue #901). When set, the
    /// run's world, hull, length, pacing and seed all come from the ARTIFACT,
    /// not from this argument list — which is why the flags that would set
    /// them are rejected alongside it.
    pub replay_path: Option<String>,
    /// Sample an authoritative-state digest every N logical ticks
    /// (`--digest-every`, issue #901). `0` — the default — is off, and costs
    /// nothing: no digest is computed at all.
    pub digest_every: u64,
    /// Measure console input-to-feedback latency (`--console-latency`, issue
    /// #1169). Off by default, and off costs nothing: the simulation takes no
    /// wall-clock reading at all, exactly as `--digest-every 0` computes no
    /// digest.
    ///
    /// **Implied by `--perf-capture`**, because a run that was asked to measure
    /// performance wants the metric the #868 budget compares
    /// (`sim.console_ack`), and requiring two flags to get one number is how a
    /// CI job silently stops producing it.
    pub console_latency: bool,
}

impl Default for HeadlessArgs {
    fn default() -> Self {
        Self {
            world_path: DEFAULT_WORLD.to_string(),
            ship_path: DEFAULT_SHIP.to_string(),
            dt: 1.0 / DEFAULT_HZ,
            max_ticks: ticks_for_sim_seconds(60.0, 1.0 / DEFAULT_HZ),
            log: LogFilterConfig::default(),
            log_spec: String::new(),
            report_path: None,
            report_format: ReportFormat::default(),
            fail_on_game_over: false,
            deterministic: false,
            seed: None,
            side_a: Vec::new(),
            side_b: Vec::new(),
            perf_capture_path: None,
            perf_scenario: DEFAULT_PERF_SCENARIO.to_string(),
            record_path: None,
            replay_path: None,
            digest_every: 0,
            console_latency: false,
        }
    }
}

impl HeadlessArgs {
    /// Simulated seconds this run covers.
    ///
    /// One less than `max_ticks` steps of `dt`: Bevy's first `update()`
    /// establishes the time baseline and reports a zero delta, so N ticks
    /// advance the clock by (N-1)·dt. [`ticks_for_sim_seconds`] inverts this.
    pub fn sim_seconds(&self) -> f64 {
        self.max_ticks.saturating_sub(1) as f64 * self.dt
    }

    pub fn hz(&self) -> f64 {
        1.0 / self.dt
    }
}

/// Outcome of parsing. `--help` is not an error, so it gets its own variant.
#[derive(Debug)]
pub enum ParseOutcome {
    Run(Box<HeadlessArgs>),
    Help,
}

/// Parse an argument list (excluding argv[0]).
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<ParseOutcome, String> {
    let raw = match Cli::try_parse_from(std::iter::once("phoenix-headless".to_string()).chain(args))
    {
        Ok(raw) => raw,
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
            return Ok(ParseOutcome::Help)
        }
        Err(error) => return Err(error.to_string()),
    };
    let world_given = raw.world.is_some();
    let ship_given = raw.ship.is_some();
    let mut out = HeadlessArgs::default();
    if let Some(world) = raw.world {
        out.world_path = world;
    }
    if let Some(ship) = raw.ship {
        out.ship_path = ship;
    }
    out.side_a = raw
        .side_a
        .as_deref()
        .map(parse_ship_list)
        .unwrap_or_default();
    out.side_b = raw
        .side_b
        .as_deref()
        .map(parse_ship_list)
        .unwrap_or_default();
    let hz = raw.hz;
    let dt = raw.dt;
    let sim_seconds = raw.sim_seconds;
    let ticks = raw.ticks;
    if let Some(spec) = raw.log {
        out.log = parse_log_spec(&spec).map_err(|e| e.to_string())?;
        out.log_spec = spec;
    }
    let log_entities = raw.log_entity;
    out.report_path = raw.report;
    if let Some(v) = raw.report_format {
        out.report_format = match v.to_lowercase().as_str() {
            "json" => ReportFormat::Json,
            "ndjson" => ReportFormat::Ndjson,
            other => {
                return Err(format!(
                    "--report-format expects 'json' or 'ndjson', got {other:?}"
                ))
            }
        };
    }
    out.fail_on_game_over = raw.fail_on_game_over;
    out.perf_capture_path = raw.perf_capture;
    if let Some(v) = raw.perf_scenario {
        if v.trim().is_empty() {
            return Err("--perf-scenario expects a name".into());
        }
        out.perf_scenario = v;
    }
    out.record_path = raw.record;
    out.replay_path = raw.replay;
    if let Some(interval) = raw.digest_every {
        out.digest_every = interval;
    }
    out.console_latency = raw.console_latency;
    out.deterministic = raw.deterministic;
    out.seed = raw.seed;

    if hz.is_some() && dt.is_some() {
        return Err("--hz and --dt both set the timestep; give one or the other".into());
    }
    if ticks.is_some() && sim_seconds.is_some() {
        return Err(
            "--ticks and --sim-seconds both set the run length; give one or the other".into(),
        );
    }

    // `--side-a` sets the player ship from its first entry, so it collides with
    // an explicit `--ship`; give one or the other. `--side-b` alone is fine.
    if ship_given && !out.side_a.is_empty() {
        return Err(
            "--ship and --side-a both choose the player ship; give one or the other".into(),
        );
    }
    // The duel flags only mean anything in a world that authors duel slots, so
    // asking for sides is asking for the duel harness unless the user named a
    // world themselves. Derived after the loop so the flags are
    // order-independent, the same idiom `--seed`/`--deterministic` uses below.
    if !world_given && (!out.side_a.is_empty() || !out.side_b.is_empty()) {
        out.world_path = DUEL_WORLD.to_string();
    }
    // Reject an over-long side here, so a bad roster fails at argument time
    // rather than deep in the world transform. The transform re-checks (it is
    // the pure authority) — this is the CLI-facing early error.
    for (side, list) in [("a", &out.side_a), ("b", &out.side_b)] {
        if list.len() > MAX_SIDE {
            return Err(format!(
                "--side-{side} lists {} ships; the maximum is {MAX_SIDE} per side",
                list.len()
            ));
        }
    }

    out.dt = match (hz, dt) {
        (Some(hz), _) => 1.0 / hz,
        (_, Some(dt)) => dt,
        _ => 1.0 / DEFAULT_HZ,
    };

    out.max_ticks = match (ticks, sim_seconds) {
        (Some(t), _) => t,
        (_, Some(s)) => ticks_for_sim_seconds(s, out.dt),
        _ => ticks_for_sim_seconds(60.0, out.dt),
    };

    // Derived after the loop so `--seed` and `--deterministic` are
    // order-independent, the same idiom `--hz`/`--dt` uses above. A seed with a
    // varying system execution order is not reproducible, so asking for one
    // implies the other; `--deterministic` alone remains meaningful.
    if out.seed.is_some() {
        out.deterministic = true;
    }

    // Replay/record validation (issue #901). Derived after the loop for the
    // same reason every other cross-flag rule here is: the flags stay
    // order-independent.
    if out.record_path.is_some() && out.replay_path.is_some() {
        return Err(
            "--record writes an artifact and --replay consumes one; give one or the other".into(),
        );
    }
    // A recording without a seed produces a file that LOOKS replayable and is
    // not — the second run would re-draw every stream from the OS. Rejected at
    // argument time rather than at write time, so the failure costs a
    // millisecond instead of a whole run.
    // A perf capture wants every metric the #868 budget knows how to compare,
    // and `sim.console_ack` (issue #1169) only exists if the run measured it.
    // Implied rather than required so a CI perf job that predates the metric
    // starts producing it without being edited — the one way a budget can stop
    // being checked without anyone deciding to stop checking it.
    if out.perf_capture_path.is_some() {
        out.console_latency = true;
    }
    if out.record_path.is_some() && out.seed.is_none() {
        return Err(
            "--record needs --seed: without one the recorded run cannot be reproduced".into(),
        );
    }
    // A recording run is driven through `PhoenixSim`, which the harness-loop
    // perf collector does not bracket. Rejected rather than accepted and
    // silently unmeasured — a capture file that quietly never appears is worse
    // than a flag that says no.
    if out.record_path.is_some() && out.perf_capture_path.is_some() {
        return Err(
            "--record and --perf-capture cannot be given together: a recording run is driven \
             through the replay simulation, which the harness-loop sampler does not measure"
                .into(),
        );
    }
    // A replay takes its whole setup from the artifact. Accepting a flag that
    // would set the same thing and then ignoring it is how a replay silently
    // runs a different scenario from the one it is verifying.
    //
    // `--ticks`/`--sim-seconds`/`--dt`/`--hz` belong on this list for exactly
    // the same reason `--world`/`--ship`/`--seed`/`--side-a`/`--side-b` do:
    // `ReplayArtifact::replay_args` sources `max_ticks` and `dt` from the
    // artifact alone (see that function), so any of these four silently did
    // nothing under `--replay` rather than erroring — a run that pacing looked
    // like it had asked for a different length or rate and had not.
    if out.replay_path.is_some() {
        for (flag, given) in [
            ("--world", world_given),
            ("--ship", ship_given),
            ("--seed", out.seed.is_some()),
            ("--side-a", !out.side_a.is_empty()),
            ("--side-b", !out.side_b.is_empty()),
            ("--ticks", ticks.is_some()),
            ("--sim-seconds", sim_seconds.is_some()),
            ("--dt", dt.is_some()),
            ("--hz", hz.is_some()),
        ] {
            if given {
                return Err(format!(
                    "{flag} cannot be given with --replay: a replay runs the world, hull, \
                     length, pacing and seed the artifact recorded"
                ));
            }
        }
    }

    // Applied after `--log` so the two flags are order-independent: setting the
    // spec replaces the whole config, which would otherwise drop the filter.
    if let Some(names) = log_entities {
        out.log.entity_filter = parse_log_entities(&names);
    }

    Ok(ParseOutcome::Run(Box::new(out)))
}

/// Ticks needed to advance the simulation clock by `seconds` at `dt`.
///
/// Rounds up so the requested span is never under-run, then adds the one
/// zero-delta tick Bevy spends establishing its time baseline — see
/// [`HeadlessArgs::sim_seconds`].
pub fn ticks_for_sim_seconds(seconds: f64, dt: f64) -> u64 {
    (seconds / dt).ceil() as u64 + 1
}

/// Split a `--side-a`/`--side-b` comma list into ship names, trimming
/// whitespace and dropping empty entries (so `a, ,b` and a trailing comma are
/// forgiving).
fn parse_ship_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

fn whole_number(value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("expects a whole number, got {value:?}"))
}

fn parse_positive_f64(s: &str, flag: &str) -> Result<f64, String> {
    let v: f64 = s
        .parse()
        .map_err(|_| format!("{flag} expects a number, got {s:?}"))?;
    if !(v.is_finite() && v > 0.0) {
        return Err(format!("{flag} expects a positive number, got {s:?}"));
    }
    Ok(v)
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod tests;
