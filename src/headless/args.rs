//! Command-line parsing for `phoenix-headless`.
//!
//! Hand-rolled rather than `clap`. The crate is a `cdylib` whose primary target
//! is `wasm32-unknown-unknown`, so every dependency has to be
//! `cfg(not(wasm32))`-gated or it lands in the shipped `.wasm`; combined with
//! `lto = true` / `codegen-units = 1`, a ~10-crate argument parser is a real
//! cost for eight flags. Keeping it a pure function over an iterator also makes
//! it directly unit-testable, matching how the rest of this crate is tested.

use crate::logging::{parse_log_entities, parse_log_spec, LogFilterConfig};

/// Default simulation rate. Matches the 60 Hz the browser host effectively runs
/// at, so headless traces line up with what a player would have seen.
pub const DEFAULT_HZ: f64 = 60.0;

const DEFAULT_WORLD: &str = "assets/worlds/default.toml";
const DEFAULT_SHIP: &str = "assets/entities/alliance_cruiser.toml";

pub const HELP: &str = "\
phoenix-headless — run the simulation with no window, no renderer, and the
player ship on AI backfill. Time advances at a fixed step as fast as the CPU
allows, so a run is wall-clock independent.

USAGE:
    phoenix-headless [OPTIONS]

WORLD
    --world <PATH>        World TOML to load    [default: assets/worlds/default.toml]
    --ship <PATH>         Player ship template  [default: assets/entities/alliance_cruiser.toml]

TIME
    --hz <N>              Simulation rate in ticks per sim-second [default: 60]
                          Do not go below 30: the AI helm clamps its integration
                          step to 1/30s, so slower rates under-integrate and the
                          run stops matching real play. 30/60/144 agree closely.
    --dt <SECONDS>        Fixed timestep; mutually exclusive with --hz
    --ticks <N>           Stop after N ticks
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

DETERMINISM
    --deterministic       Pin the scheduler to one thread, so system execution
                          order is fixed run to run. A fixed timestep alone gives
                          wall-clock independence, not reproducibility.
                          CAVEAT: damage distribution and region effects still
                          seed from the OS (5 `SmallRng::from_os_rng()` sites).
                          Runs that take damage may still diverge; there is no
                          --seed flag because there is nothing yet to seed.

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
    /// Fixed timestep in seconds.
    pub dt: f64,
    /// Tick count at which the run stops. Always resolved — `--sim-seconds` is
    /// converted here so the run loop only ever counts ticks.
    pub max_ticks: u64,
    pub log: LogFilterConfig,
    /// The raw `--log` spec, forwarded to `LogPlugin`'s own `EnvFilter` so
    /// bevy-internal events roughly agree with our categories.
    pub log_spec: String,
    pub report_path: Option<String>,
    pub report_format: ReportFormat,
    pub fail_on_game_over: bool,
    /// Pin the scheduler to a single thread. Does *not* pin the RNG — see the
    /// caveat in [`HELP`].
    pub deterministic: bool,
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

    while let Some(arg) = it.next() {
        let mut value = || -> Result<String, String> {
            it.next().ok_or_else(|| format!("{arg} requires a value"))
        };
        match arg.as_str() {
            "-h" | "--help" => return Ok(ParseOutcome::Help),
            "--world" => out.world_path = value()?,
            "--ship" => out.ship_path = value()?,
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
            "--deterministic" => out.deterministic = true,
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
}
