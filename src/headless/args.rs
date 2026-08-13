//! Command-line parsing for `phoenix-headless`.
//!
//! Hand-rolled rather than `clap`. The crate is a `cdylib` whose primary target
//! is `wasm32-unknown-unknown`, so every dependency has to be
//! `cfg(not(wasm32))`-gated or it lands in the shipped `.wasm`; combined with
//! `lto = true` / `codegen-units = 1`, a ~10-crate argument parser is a real
//! cost for eight flags. Keeping it a pure function over an iterator also makes
//! it directly unit-testable, matching how the rest of this crate is tested.

use crate::headless::duel::MAX_SIDE;
use crate::logging::{parse_log_entities, parse_log_spec, LogFilterConfig};
use crate::server_app::{RegistrationOrder, RegistrationProbes};

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

pub const HELP: &str = "\
phoenix-headless — run the simulation with no window, no renderer, and the
player ship on AI backfill. Time advances at a fixed step as fast as the CPU
allows, so a run is wall-clock independent.

USAGE:
    phoenix-headless [OPTIONS]

WORLD
    --world <PATH>        World TOML to load    [default: assets/worlds/default.toml]
    --ship <PATH>         Player ship template  [default: assets/entities/alliance_cruiser.toml]

DUEL (assets/worlds/duel.toml)
    --side-a <LIST>       Comma-separated ship classes for side A (max 5). The
                          first is the player ship; the rest are NPC escorts.
                          Mutually exclusive with --ship.
    --side-b <LIST>       Comma-separated ship classes for side B (max 5), all
                          NPCs hostile to side A.
                          Names resolve in order: alliance_<name>.toml, then
                          <name>.toml (both under assets/entities/), then <name>
                          as a literal path. e.g. --side-a cruiser --side-b destroyer
                          Either flag defaults --world to the duel harness
                          (assets/worlds/duel.toml) — the only world carrying the
                          `// duel:slots` marker, below which the side_a_*/
                          side_b_* slot drivers are regenerated. An explicit
                          --world still wins, but a world with no such marker is
                          rejected rather than silently run as-is. Without either
                          flag the world's own authored roster runs untouched.

TIME
    --hz <N>              Frame rate the harness drives the app at, in frames
                          per sim-second [default: 60]. Since issue #895 the
                          SIMULATION advances on the world's [global]
                          sim_tick_hz (default 60) inside Bevy's fixed loop,
                          so this flag chooses how much virtual time each
                          update() advances, not how often the sim thinks —
                          any --hz covers the same logical ticks per
                          sim-second — and since issue #896 that includes
                          rapier, which steps once per logical tick too.
    --dt <SECONDS>        Frame period; mutually exclusive with --hz
    --ticks <N>           Stop after N frames. Named before issue #895, when a
                          frame and a sim tick were the same thing; it counts
                          update() calls, so the LOGICAL ticks a run covers are
                          N x sim_tick_hz / --hz.
    --sim-seconds <N>     Stop after N seconds of simulated time
                          (If neither is given, the run stops at 60 sim-seconds.)

LOGGING
    --log <SPEC>          Category levels, e.g. 'info,ai=debug,admit=trace'
                          Categories: ai helm weapons shields damage power sensors
                          comms repair nav captain lobby admit world regions
                          physics broadcast assets config
                          Levels: off error warn info debug trace
    --log-entity <NAMES>  Only log events for these entities, by display name.
                          Comma-separated; matched exactly, then case-insensitively
                          as a substring. e.g. 'Ironveil,Ashrender'

OUTPUT
    --report <PATH>       Write the exit summary here instead of stdout ('-' for stdout)
    --report-format <F>   'json' (exit summary only) or 'ndjson' (also stream every
                          outbound message, one JSON object per line) [default: json]
    --fail-on-game-over   Exit non-zero if the run ends in GamePhase::GameOver

PERFORMANCE
    --perf-capture <PATH> Sample per-tick and whole-run wall time and write the
                          capture JSON here ('-' for stdout). Absent, no
                          measurement is collected at all. Sampling brackets the
                          harness loop from outside, so a measured run steps
                          identically to an unmeasured one.
    --perf-scenario <N>   Scenario the capture is filed under, and the baseline
                          it is compared against, at perf/baselines/<N>.ron
                          [default: headless-default]. A missing baseline is not an
                          error: the capture is still written. Comparison is
                          warnings-only and never changes the exit code.

DETERMINISM
    --deterministic       Pin the scheduler to one thread, so system execution
                          order is fixed run to run. A fixed timestep alone gives
                          wall-clock independence, not reproducibility.
    --seed <N>            Master seed for the simulation RNG (u64). Implies
                          --deterministic. Every RNG site — damage distribution,
                          region effects, entity UUIDs — derives its own stream
                          from this, so two runs with the same seed produce
                          byte-identical reports.
                          Byte-identical includes the timing fields: a --seed run
                          reports wall_seconds, ticks_per_second and
                          speedup_vs_realtime as 0, because those are measured
                          off the host clock and would otherwise be the only
                          lines that differ between two identical --seed runs.
                          Only --seed gets this, because only --seed pins the
                          scheduler: zeroed timings mean 'this run is
                          replayable'. World-TOML-seeded and unseeded runs
                          report the real figures.
                          Precedence: --seed, then the world TOML's
                          [global] seed, then a seed drawn from the OS. The
                          resolved seed and its source are always in the report,
                          so you can replay any run by feeding that seed back in
                          as --seed. Only --seed pins the scheduler, so a run
                          that took its seed from the world TOML or the OS was
                          not itself reproducible: replaying it with --seed is a
                          single-threaded re-run of the same scenario, and may
                          not match what you saw.
                          CAVEAT: the contract is same binary, same machine.
                          Floating-point differences across CPUs or compiler
                          versions can still diverge.

REPLAY (issue #901)
    --record <PATH>       Write a replay artifact here: the seed, the world,
                          hull, length and pacing, the command log the run
                          accepted, and the digests it passed through. Requires
                          --seed — an artifact with an OS-drawn seed names a run
                          nothing can re-derive.
    --replay <PATH>       Replay an artifact and report whether the second run
                          reproduced the first. Everything the run needs comes
                          from the ARTIFACT, so --world, --ship, --side-a,
                          --side-b, --seed, --ticks, --sim-seconds, --dt and
                          --hz are rejected alongside it rather than accepted
                          and quietly ignored. Exit code 4 when the replay
                          diverges, and the message names the tick window it
                          first disagreed in rather than merely that it
                          disagreed.
                          --log/--log-entity and --report/--report-format are
                          NOT rejected, but they are inert here: a replay
                          prints a verdict, not the ordinary run report or
                          per-tick log output those flags shape, so giving them
                          changes nothing about what --replay does.
    --digest-every <N>    Sample an authoritative-state digest every N LOGICAL
                          ticks [default: 0 = off, and off costs nothing: no
                          digest is computed at all]. The samples are what turn
                          'these two runs differ' into 'they agreed at tick 240
                          and disagreed by tick 250'. A --replay run samples at
                          the interval its artifact recorded, so it compares
                          like with like; this flag chooses the interval a
                          --record run writes down.

    -h, --help            Show this help
";

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
    /// Pin the scheduler to a single thread. Implied by `seed`, which needs a
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
    /// Register the physics plugin last instead of first
    /// (`SimPluginOptions::physics_last`, issue #896).
    ///
    /// **Not a command-line flag** and never parsed from one — no run a user
    /// can ask for changes this. It exists so a test can build the same
    /// colliding scenario with the contributing systems registered in either
    /// order and require the two to reach the same state.
    pub physics_last: bool,
    /// Which order to register the `SimSet`-chain plugins in
    /// (`SimPluginOptions::registration_order`, issue #899). **Not a
    /// command-line flag**, like `physics_last` above — the sole caller is
    /// `tests/registration_order_determinism.rs`.
    pub registration_order: RegistrationOrder,
    /// Extra mutation-proof probes to fold into the shuffled group
    /// (`SimPluginOptions::extra_registration_probes`, issue #899). `None` in
    /// every real run.
    pub extra_registration_probes: Option<RegistrationProbes>,
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
            physics_last: false,
            registration_order: RegistrationOrder::Canonical,
            extra_registration_probes: None,
            record_path: None,
            replay_path: None,
            digest_every: 0,
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
    let mut out = HeadlessArgs::default();
    let mut it = args.into_iter().peekable();

    // Deferred so `--hz`/`--dt` can be given in any order relative to
    // `--ticks`/`--sim-seconds`, and so we can reject contradictory pairs.
    let mut hz: Option<f64> = None;
    let mut dt: Option<f64> = None;
    let mut ticks: Option<u64> = None;
    let mut sim_seconds: Option<f64> = None;
    let mut log_entities: Option<String> = None;
    // Whether `--ship` was given explicitly, so it can be rejected alongside
    // `--side-a` (which sets the player ship from its first entry).
    let mut ship_given = false;
    // Whether `--world` was given explicitly, so `--side-a`/`--side-b` can
    // default it to the duel harness without overriding a deliberate choice.
    let mut world_given = false;

    while let Some(arg) = it.next() {
        let mut value = || -> Result<String, String> {
            it.next().ok_or_else(|| format!("{arg} requires a value"))
        };
        match arg.as_str() {
            "-h" | "--help" => return Ok(ParseOutcome::Help),
            "--world" => {
                out.world_path = value()?;
                world_given = true;
            }
            "--ship" => {
                out.ship_path = value()?;
                ship_given = true;
            }
            "--side-a" => out.side_a = parse_ship_list(&value()?),
            "--side-b" => out.side_b = parse_ship_list(&value()?),
            "--hz" => hz = Some(parse_positive_f64(&value()?, "--hz")?),
            "--dt" => dt = Some(parse_positive_f64(&value()?, "--dt")?),
            "--ticks" => {
                let v = value()?;
                ticks = Some(
                    v.parse()
                        .map_err(|_| format!("--ticks expects a whole number, got {v:?}"))?,
                );
            }
            "--sim-seconds" => sim_seconds = Some(parse_positive_f64(&value()?, "--sim-seconds")?),
            "--log" => {
                let spec = value()?;
                out.log = parse_log_spec(&spec).map_err(|e| e.to_string())?;
                out.log_spec = spec;
            }
            "--log-entity" => log_entities = Some(value()?),
            "--report" => out.report_path = Some(value()?),
            "--report-format" => {
                let v = value()?;
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
            "--fail-on-game-over" => out.fail_on_game_over = true,
            "--perf-capture" => out.perf_capture_path = Some(value()?),
            "--perf-scenario" => {
                let v = value()?;
                if v.trim().is_empty() {
                    return Err("--perf-scenario expects a name".into());
                }
                out.perf_scenario = v;
            }
            "--record" => out.record_path = Some(value()?),
            "--replay" => out.replay_path = Some(value()?),
            "--digest-every" => {
                let v = value()?;
                out.digest_every = v.parse().map_err(|_| {
                    format!("--digest-every expects a whole number of ticks, got {v:?}")
                })?;
            }
            "--deterministic" => out.deterministic = true,
            "--seed" => {
                let v = value()?;
                out.seed = Some(
                    v.parse()
                        .map_err(|_| format!("--seed expects a whole number, got {v:?}"))?,
                );
            }
            other => return Err(format!("unknown argument {other:?} (try --help)")),
        }
    }

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
mod tests {
    use super::*;
    use crate::logging::{LevelFilter, LogCat};

    fn parse(args: &[&str]) -> HeadlessArgs {
        match parse_args(args.iter().map(|s| s.to_string())).unwrap() {
            ParseOutcome::Run(a) => *a,
            ParseOutcome::Help => panic!("expected Run, got Help"),
        }
    }

    fn err(args: &[&str]) -> String {
        parse_args(args.iter().map(|s| s.to_string())).unwrap_err()
    }

    #[test]
    fn defaults_are_sixty_hz_for_sixty_seconds() {
        let a = parse(&[]);
        assert_eq!(a.dt, 1.0 / 60.0);
        // 3600 stepping ticks plus the zero-delta baseline tick.
        assert_eq!(a.max_ticks, 3601);
        assert!((a.sim_seconds() - 60.0).abs() < 1e-9);
        assert!(a.world_path.ends_with("default.toml"));
    }

    /// `sim_seconds()` must invert `ticks_for_sim_seconds` — this is the
    /// contract the run loop and the report both lean on.
    #[test]
    fn tick_count_and_sim_seconds_round_trip() {
        for (secs, hz) in [(60.0, 60.0), (10.0, 30.0), (1.0, 144.0), (0.5, 20.0)] {
            let a = parse(&["--sim-seconds", &secs.to_string(), "--hz", &hz.to_string()]);
            assert!(
                (a.sim_seconds() - secs).abs() < 1e-9,
                "{secs}s at {hz}Hz round-tripped to {}",
                a.sim_seconds()
            );
        }
    }

    #[test]
    fn help_short_circuits_before_other_arguments() {
        assert!(matches!(
            parse_args(["--help".to_string(), "--nonsense".to_string()]).unwrap(),
            ParseOutcome::Help
        ));
    }

    #[test]
    fn hz_sets_the_timestep() {
        let a = parse(&["--hz", "120"]);
        assert!((a.dt - 1.0 / 120.0).abs() < f64::EPSILON);
        assert!((a.hz() - 120.0).abs() < 1e-9);
    }

    #[test]
    fn dt_is_an_alternative_to_hz() {
        let a = parse(&["--dt", "0.05"]);
        assert_eq!(a.dt, 0.05);
    }

    #[test]
    fn hz_and_dt_together_are_rejected() {
        assert!(err(&["--hz", "60", "--dt", "0.01"]).contains("give one or the other"));
    }

    /// The run loop only counts ticks, so `--sim-seconds` must resolve against
    /// whatever `--hz` ends up being — including when `--hz` comes afterwards.
    #[test]
    fn sim_seconds_resolves_against_hz_in_either_order() {
        assert_eq!(parse(&["--sim-seconds", "10", "--hz", "30"]).max_ticks, 301);
        assert_eq!(parse(&["--hz", "30", "--sim-seconds", "10"]).max_ticks, 301);
    }

    #[test]
    fn sim_seconds_rounds_up_so_the_span_is_never_short() {
        // 10s at 3Hz is 30 stepping ticks exactly; 10.1s needs 31. Both then
        // gain the baseline tick.
        assert_eq!(parse(&["--hz", "3", "--sim-seconds", "10"]).max_ticks, 31);
        assert_eq!(parse(&["--hz", "3", "--sim-seconds", "10.1"]).max_ticks, 32);
    }

    #[test]
    fn ticks_and_sim_seconds_together_are_rejected() {
        assert!(err(&["--ticks", "10", "--sim-seconds", "10"]).contains("give one or the other"));
    }

    #[test]
    fn log_spec_is_parsed_and_retained_verbatim() {
        let a = parse(&["--log", "info,ai=debug"]);
        assert_eq!(a.log.default_level, LevelFilter::Info);
        assert_eq!(a.log.per_cat[&LogCat::Ai], LevelFilter::Debug);
        assert_eq!(a.log_spec, "info,ai=debug");
    }

    /// `--log` replaces the whole config, so it must not clobber an entity
    /// filter given before it.
    #[test]
    fn log_and_log_entity_are_order_independent() {
        for args in [
            ["--log", "ai=debug", "--log-entity", "Ironveil"],
            ["--log-entity", "Ironveil", "--log", "ai=debug"],
        ] {
            let a = parse(&args);
            assert_eq!(a.log.per_cat[&LogCat::Ai], LevelFilter::Debug);
            let f = a.log.entity_filter.as_ref().expect("entity filter dropped");
            assert_eq!(f.names, vec!["Ironveil"]);
        }
    }

    #[test]
    fn deterministic_is_off_by_default() {
        assert!(!parse(&[]).deterministic);
        assert!(parse(&["--deterministic"]).deterministic);
    }

    #[test]
    fn seed_is_unset_by_default_and_implies_deterministic() {
        assert_eq!(parse(&[]).seed, None);
        assert!(!parse(&[]).deterministic);

        for args in [
            ["--seed", "9001", "--hz", "30"],
            ["--hz", "30", "--seed", "9001"],
        ] {
            let a = parse(&args);
            assert_eq!(a.seed, Some(9001));
            assert!(a.deterministic, "--seed must imply --deterministic");
        }
    }

    #[test]
    fn a_non_numeric_seed_is_rejected() {
        assert!(err(&["--seed", "lucky"]).contains("whole number"));
        assert!(err(&["--seed", "-1"]).contains("whole number"));
        assert!(err(&["--seed"]).contains("requires a value"));
    }

    #[test]
    fn report_format_is_case_insensitive_and_validated() {
        assert_eq!(
            parse(&["--report-format", "NDJSON"]).report_format,
            ReportFormat::Ndjson
        );
        assert!(err(&["--report-format", "yaml"]).contains("json"));
    }

    #[test]
    fn bad_log_spec_surfaces_the_parser_error() {
        assert!(err(&["--log", "warpcore=debug"]).contains("warpcore"));
    }

    #[test]
    fn unknown_flags_and_missing_values_are_errors() {
        assert!(err(&["--warp"]).contains("unknown argument"));
        assert!(err(&["--world"]).contains("requires a value"));
    }

    #[test]
    fn non_positive_rates_are_rejected() {
        assert!(err(&["--hz", "0"]).contains("positive"));
        assert!(err(&["--dt", "-1"]).contains("positive"));
        assert!(err(&["--hz", "fast"]).contains("expects a number"));
    }

    // ── --side-a / --side-b (issue #844) ────────────────────────────────────

    #[test]
    fn sides_default_to_empty() {
        let a = parse(&[]);
        assert!(a.side_a.is_empty());
        assert!(a.side_b.is_empty());
    }

    #[test]
    fn side_a_is_a_comma_split_list() {
        let a = parse(&["--side-a", "cruiser,courier"]);
        assert_eq!(a.side_a, vec!["cruiser", "courier"]);
        assert!(a.side_b.is_empty());
    }

    #[test]
    fn side_b_is_a_comma_split_list() {
        let a = parse(&["--side-b", "destroyer"]);
        assert_eq!(a.side_b, vec!["destroyer"]);
        assert!(a.side_a.is_empty());
    }

    /// Forgiving splitting: whitespace trimmed, empty entries dropped.
    #[test]
    fn side_list_trims_and_drops_empties() {
        let a = parse(&["--side-a", " cruiser , , courier ,"]);
        assert_eq!(a.side_a, vec!["cruiser", "courier"]);
    }

    #[test]
    fn a_side_longer_than_five_is_rejected() {
        assert!(err(&["--side-a", "a,b,c,d,e,f"]).contains("maximum is 5"));
        assert!(err(&["--side-b", "a,b,c,d,e,f"]).contains("maximum is 5"));
        // Exactly five is allowed.
        assert_eq!(parse(&["--side-a", "a,b,c,d,e"]).side_a.len(), 5);
    }

    /// `--side-a` sets the player ship, so it collides with an explicit
    /// `--ship`. `--side-b` alone does not.
    #[test]
    fn ship_and_side_a_together_are_rejected() {
        assert!(err(&[
            "--ship",
            "assets/entities/alliance_cruiser.toml",
            "--side-a",
            "courier"
        ])
        .contains("give one or the other"));
        // --ship with only --side-b is fine (side B is all NPCs).
        let a = parse(&[
            "--ship",
            "assets/entities/alliance_cruiser.toml",
            "--side-b",
            "destroyer",
        ]);
        assert_eq!(a.side_b, vec!["destroyer"]);
    }

    /// The trap this closes: `--side-a cruiser --side-b destroyer` with no
    /// `--world` used to load `default.toml`, which authors none of the slots
    /// those flags fill — the run was a combat-free draw that looked like a
    /// balance finding. Either flag alone is enough to imply the harness.
    #[test]
    fn sides_without_an_explicit_world_default_to_the_duel_harness() {
        assert_eq!(
            parse(&["--side-a", "cruiser", "--side-b", "destroyer"]).world_path,
            DUEL_WORLD
        );
        assert_eq!(parse(&["--side-b", "destroyer"]).world_path, DUEL_WORLD);
        // No sides → the plain default is untouched.
        assert_eq!(parse(&[]).world_path, DEFAULT_WORLD);
    }

    /// An explicit `--world` still wins: a user may have authored their own
    /// duel-shaped world. Order-independent, like `--seed`/`--deterministic`.
    #[test]
    fn an_explicit_world_wins_over_the_duel_default() {
        assert_eq!(
            parse(&[
                "--side-a",
                "cruiser",
                "--world",
                "assets/worlds/combat_test.toml"
            ])
            .world_path,
            "assets/worlds/combat_test.toml"
        );
        assert_eq!(
            parse(&[
                "--world",
                "assets/worlds/combat_test.toml",
                "--side-b",
                "destroyer"
            ])
            .world_path,
            "assets/worlds/combat_test.toml"
        );
    }

    // ── Replay flags (issue #901) ────────────────────────────────────────────

    #[test]
    fn replay_flags_default_to_off() {
        let a = parse(&[]);
        assert_eq!(a.record_path, None);
        assert_eq!(a.replay_path, None);
        assert_eq!(
            a.digest_every, 0,
            "periodic hashing must be off unless asked for, so a run that did not ask for it pays nothing"
        );
    }

    #[test]
    fn record_and_digest_every_parse() {
        let a = parse(&[
            "--record",
            "run.ron",
            "--seed",
            "7",
            "--digest-every",
            "120",
        ]);
        assert_eq!(a.record_path.as_deref(), Some("run.ron"));
        assert_eq!(a.digest_every, 120);
        assert_eq!(a.seed, Some(7));
    }

    /// An artifact whose seed came from the OS names a run nothing can
    /// re-derive, so it must fail at argument time rather than after the run.
    #[test]
    fn recording_without_a_seed_is_refused() {
        assert!(err(&["--record", "run.ron"]).contains("--seed"));
    }

    #[test]
    fn recording_and_replaying_at_once_is_refused() {
        assert!(
            err(&["--record", "a.ron", "--replay", "b.ron", "--seed", "1"])
                .contains("one or the other")
        );
    }

    /// A replay takes its whole setup from the artifact. A flag that would set
    /// the same thing is rejected rather than accepted and quietly ignored.
    #[test]
    fn a_replay_refuses_the_flags_the_artifact_already_decides() {
        for extra in [
            vec!["--world", "assets/worlds/patrol.toml"],
            vec!["--ship", "assets/entities/alliance_cruiser.toml"],
            vec!["--seed", "3"],
            vec!["--side-a", "cruiser"],
            vec!["--side-b", "destroyer"],
            vec!["--ticks", "100"],
            vec!["--sim-seconds", "10"],
            vec!["--dt", "0.02"],
            vec!["--hz", "30"],
        ] {
            let mut argv = vec!["--replay", "run.ron"];
            argv.extend(extra.iter());
            let message = err(&argv);
            assert!(
                message.contains("--replay"),
                "{extra:?} should be refused alongside --replay; got {message:?}"
            );
        }
    }

    #[test]
    fn recording_and_perf_capture_together_are_refused() {
        assert!(err(&[
            "--record",
            "run.ron",
            "--seed",
            "1",
            "--perf-capture",
            "cap.json"
        ])
        .contains("--perf-capture"));
    }

    #[test]
    fn digest_every_rejects_a_non_number() {
        assert!(err(&["--digest-every", "often"]).contains("--digest-every"));
    }
}
