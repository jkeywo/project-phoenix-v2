//! `phoenix-headless` — run the simulation without a browser.
//!
//! All the work lives in `project_phoenix::headless` so it can be unit-tested;
//! this is just argv in, report out, exit code out.
//!
//! Exit codes: 0 clean, 1 build/IO failure, 2 bad arguments, 3 the run ended in
//! `GamePhase::GameOver` and `--fail-on-game-over` was given, 4 a `--replay`
//! run did not reproduce the artifact it was given (issue #901).

// `required-features` in Cargo.toml gates on features, not targets, so
// `--features headless --target wasm32-unknown-unknown` would otherwise try to
// build this against a `crate::headless` that does not exist there.
#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("phoenix-headless is a native binary; it has no wasm32 build.");
}

#[cfg(not(target_arch = "wasm32"))]
use project_phoenix::headless::{
    build_headless_app, build_report, parse_args, replay::drive_run, run_sampled, HeadlessArgs,
    ParseOutcome, ReplayArtifact, HELP,
};
#[cfg(not(target_arch = "wasm32"))]
use project_phoenix::perf::{self, tick::TickSampler};

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // Fix Rhai's global hashing seed before anything can build a script engine
    // (issue #979): the headless runner must seed identically to every browser
    // peer, or a `(tick, script_path, fn_name)` key recorded on one would not
    // resolve on the other. Idempotent; see `world::script::init_hashing_seed`.
    project_phoenix::world::script::init_hashing_seed();

    let args = match parse_args(std::env::args().skip(1)) {
        Ok(ParseOutcome::Help) => {
            print!("{HELP}");
            return;
        }
        Ok(ParseOutcome::Run(args)) => *args,
        Err(e) => {
            eprintln!("phoenix-headless: {e}");
            eprintln!("try --help");
            std::process::exit(2);
        }
    };

    // `--replay` is a whole different job from running the simulation for its
    // own sake: it takes its setup from the artifact, produces a verdict rather
    // than a report, and has its own exit code. It returns rather than falling
    // through.
    if let Some(path) = args.replay_path.as_deref() {
        replay(path);
        return;
    }

    // `--record` drives the run through `PhoenixSim` (the `vellum_replay::
    // Simulation` this binary's replay path consumes) rather than through
    // `run_sampled`, so a recording and a replay walk the same code and a
    // divergence can never be an artifact of the two being driven differently.
    // Without `--record` nothing changes: the shipped run is the run it always
    // was, `--perf-capture` included.
    let started = std::time::Instant::now();
    let mut sampler = None;
    let mut app = if args.record_path.is_some() {
        match record(&args) {
            Ok(app) => app,
            Err(e) => {
                eprintln!("phoenix-headless: {e}");
                std::process::exit(1);
            }
        }
    } else {
        let mut app = match build_headless_app(&args) {
            Ok(app) => app,
            Err(e) => {
                eprintln!("phoenix-headless: {e}");
                std::process::exit(1);
            }
        };
        // The collector only exists when it was asked for, so an ordinary run
        // is byte-for-byte the run it always was.
        sampler = args.perf_capture_path.as_ref().map(|_| TickSampler::new());
        run_sampled(&mut app, args.max_ticks, sampler.as_mut());
        app
    };
    let report = build_report(&mut app, &args, started.elapsed().as_secs_f64());

    // Measurement is written and compared before the report's exit code is
    // decided: a run that ends in GameOver still measured something, and
    // throwing the evidence away because of the exit path would be the one
    // case where the numbers are most interesting.
    if let (Some(sampler), Some(path)) = (sampler, args.perf_capture_path.as_deref()) {
        let capture = sampler.finish(&args.perf_scenario, perf::profile(perf::tick::RUNTIME));
        let json = capture.to_json();
        match path {
            "-" => println!("{json}"),
            path => {
                if let Err(e) = std::fs::write(path, format!("{json}\n")) {
                    eprintln!("phoenix-headless: could not write capture to {path:?}: {e}");
                    std::process::exit(1);
                }
            }
        }

        let baseline_file = perf::baseline_path(&args.perf_scenario);
        match perf::load_baseline(std::path::Path::new(&baseline_file)) {
            // No baseline yet is the normal first state for a new scenario.
            Ok(None) => eprintln!("phoenix-headless: no baseline at {baseline_file}; capture only"),
            Ok(Some(baseline)) => {
                let (_findings, rendered) = perf::report(&capture, &baseline);
                eprintln!("{rendered}");
            }
            // A malformed baseline is a broken contract, not a slow build.
            // Say so loudly and still leave the exit code to the run.
            Err(e) => eprintln!("phoenix-headless: {e}"),
        }
    }

    let json = report.to_json();
    match args.report_path.as_deref() {
        None | Some("-") => println!("{json}"),
        Some(path) => {
            if let Err(e) = std::fs::write(path, format!("{json}\n")) {
                eprintln!("phoenix-headless: could not write report to {path:?}: {e}");
                std::process::exit(1);
            }
        }
    }

    if args.fail_on_game_over && report.ended_in_game_over() {
        eprintln!(
            "phoenix-headless: run ended in GameOver{}",
            match &report.game_over_reason {
                Some(r) => format!(": {r}"),
                None => String::new(),
            }
        );
        std::process::exit(3);
    }
}

/// Drive a `--record` run and write its replay artifact, handing the app back
/// so the ordinary exit report is still produced.
///
/// The artifact is written BEFORE the report, and before any exit-code
/// decision: a run that ends in `GameOver` still recorded a replayable run, and
/// throwing that away because of the exit path would discard the artifact in
/// exactly the case it is most worth having. Same argument the perf capture
/// above makes for itself.
#[cfg(not(target_arch = "wasm32"))]
fn record(args: &HeadlessArgs) -> Result<bevy::app::App, Box<dyn std::error::Error>> {
    // No script: an unattended headless run has no human input of its own, and
    // the AI's in-process emissions deliberately never cross the boundary the
    // log records — a replay re-derives them from the seed.
    let mut sim = drive_run(args, &[], args.digest_every)?;
    let log = sim.recorded_log();
    let ledger = sim.seal();
    let path = args
        .record_path
        .as_deref()
        .expect("record() is only called when --record was given");
    let artifact = ReplayArtifact::capture(args, log, ledger)?;
    artifact.write(path)?;
    eprintln!(
        "phoenix-headless: wrote replay artifact to {path:?} — seed {}, {} command(s), \
         {} checkpoint(s), final digest {:#018x}{}",
        artifact.seed,
        artifact.log.len(),
        artifact.ledger.checkpoints.len(),
        artifact.ledger.final_digest,
        refused_suffix(artifact.ledger.refused),
    );
    Ok(sim.into_app())
}

/// Render "N command(s) refused" when non-zero, empty otherwise — so an
/// ordinary run's message is unchanged and a refusal is named rather than
/// silently folded into the digest (issue #901 review).
#[cfg(not(target_arch = "wasm32"))]
fn refused_suffix(refused: u64) -> String {
    if refused == 0 {
        String::new()
    } else {
        format!(", {refused} command(s) submitted but refused by admission")
    }
}

/// Replay an artifact and report the verdict. Exits rather than returning: a
/// replay produces a verdict, not a run report.
///
/// Drives `replay_artifact` directly rather than the `verify_artifact`
/// convenience (which discards the replayed ledger once it has compared it):
/// this needs the replayed ledger's own `refused` count, not only whether the
/// two ledgers first disagree, so a refusal that changed between the
/// recording and this replay is named even when the two ledgers otherwise
/// happen to agree.
#[cfg(not(target_arch = "wasm32"))]
fn replay(path: &str) {
    let artifact = match ReplayArtifact::read(path) {
        Ok(artifact) => artifact,
        Err(e) => {
            eprintln!("phoenix-headless: {e}");
            std::process::exit(1);
        }
    };
    let replayed =
        match project_phoenix::headless::replay_artifact(&artifact, artifact.ledger.interval) {
            Ok(replayed) => replayed,
            Err(e) => {
                eprintln!("phoenix-headless: {e}");
                std::process::exit(1);
            }
        };
    if replayed.refused != artifact.ledger.refused {
        eprintln!(
            "phoenix-headless: refusal count changed — {} command(s) were admitted when this \
             was recorded, {} are admitted now. A command that no longer admits is a real \
             divergence even if the digests below still happen to agree.",
            artifact.ledger.refused, replayed.refused
        );
    }
    match artifact.ledger.first_divergence(&replayed) {
        None => println!(
            "replay reproduced the recording: seed {}, {} command(s), {} checkpoint(s) all \
             agreed, final digest {:#018x}{}",
            artifact.seed,
            artifact.log.len(),
            artifact.ledger.checkpoints.len(),
            artifact.ledger.final_digest,
            refused_suffix(replayed.refused),
        ),
        Some(divergence) => {
            eprintln!("phoenix-headless: replay did NOT reproduce the recording");
            eprintln!("  {divergence}");
            if artifact.ledger.interval == 0 {
                eprintln!(
                    "  This artifact recorded no periodic checkpoints, so the answer is the \
                     whole run. Re-record with --digest-every <N> to localise it to a tick \
                     window."
                );
            }
            std::process::exit(4);
        }
    }
}
