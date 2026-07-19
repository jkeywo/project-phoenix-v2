//! `phoenix-headless` — run the simulation without a browser.
//!
//! All the work lives in `project_phoenix::headless` so it can be unit-tested;
//! this is just argv in, report out, exit code out.
//!
//! Exit codes: 0 clean, 1 build/IO failure, 2 bad arguments, 3 the run ended in
//! `GamePhase::GameOver` and `--fail-on-game-over` was given.

// `required-features` in Cargo.toml gates on features, not targets, so
// `--features headless --target wasm32-unknown-unknown` would otherwise try to
// build this against a `crate::headless` that does not exist there.
#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("phoenix-headless is a native binary; it has no wasm32 build.");
}

#[cfg(not(target_arch = "wasm32"))]
use project_phoenix::headless::{
    build_headless_app, build_report, parse_args, run, ParseOutcome, HELP,
};

#[cfg(not(target_arch = "wasm32"))]
fn main() {
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

    let mut app = match build_headless_app(&args) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("phoenix-headless: {e}");
            std::process::exit(1);
        }
    };

    let started = std::time::Instant::now();
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, started.elapsed().as_secs_f64());

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
