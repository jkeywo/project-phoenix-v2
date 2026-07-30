//! `phoenix-perf` — asset budgets, and the capture-versus-baseline report.
//!
//! One binary for the two halves of #868 that need no simulation: extracting
//! the static asset inventory, and comparing any capture (from here, from the
//! headless harness, or pulled out of a browser session) against its committed
//! baseline.
//!
//! Exit codes: 0 whatever the verdict, 1 IO/parse failure, 2 bad arguments.
//! The verdict deliberately does not reach the exit code — the comparison
//! reports, it does not gate.

#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("phoenix-perf is a native binary; it has no wasm32 build.");
}

#[cfg(not(target_arch = "wasm32"))]
const HELP: &str = "\
phoenix-perf — asset budgets and baseline comparison.

USAGE:
    phoenix-perf assets [--root <DIR>] [--capture <PATH>]
    phoenix-perf report --capture <PATH> [--scenario <NAME>]

assets      Walk assets/models and assets/entities and write a capture of the
            shipped byte counts and LOD coverage. No simulation runs, so the
            same checkout always produces the same numbers.
    --root <DIR>       Repository root to walk [default: .]
    --capture <PATH>   Where to write the capture JSON ('-' for stdout)

report      Compare a capture against perf/baselines/<scenario>.ron and render
            the findings. Warnings-first: the exit code is 0 whatever the
            verdict, because measurement informs optimisation before it gates.
    --capture <PATH>   Capture JSON to read ('-' for stdin)
    --scenario <NAME>  Baseline to compare against [default: the capture's own
                       scenario name]

    -h, --help  Show this help
";

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return;
    }

    let result = match args[0].as_str() {
        "assets" => assets(&args[1..]),
        "report" => report(&args[1..]),
        other => Err(format!("unknown command {other:?} (try --help)")),
    };

    if let Err(e) = result {
        eprintln!("phoenix-perf: {e}");
        std::process::exit(2);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn flag(args: &[String], name: &str) -> Result<Option<String>, String> {
    match args.iter().position(|a| a == name) {
        None => Ok(None),
        Some(i) => args
            .get(i + 1)
            .cloned()
            .map(Some)
            .ok_or_else(|| format!("{name} requires a value")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn reject_unknown(args: &[String], known: &[&str]) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        if !known.contains(&args[i].as_str()) {
            return Err(format!("unknown argument {:?} (try --help)", args[i]));
        }
        i += 2;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn assets(args: &[String]) -> Result<(), String> {
    reject_unknown(args, &["--root", "--capture"])?;
    let root = flag(args, "--root")?.unwrap_or_else(|| ".".to_string());
    let out = flag(args, "--capture")?.unwrap_or_else(|| "-".to_string());

    let found = project_phoenix::perf::assets::inventory(std::path::Path::new(&root))
        .map_err(|e| e.to_string())?;
    let capture = project_phoenix::perf::assets::capture(
        &found,
        project_phoenix::perf::profile(project_phoenix::perf::assets::RUNTIME),
    );
    write_out(&out, &capture.to_json())
}

#[cfg(not(target_arch = "wasm32"))]
fn report(args: &[String]) -> Result<(), String> {
    reject_unknown(args, &["--capture", "--scenario"])?;
    let path = flag(args, "--capture")?.ok_or("report requires --capture")?;

    let json = if path == "-" {
        let mut buffer = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)
            .map_err(|e| format!("could not read capture from stdin: {e}"))?;
        buffer
    } else {
        std::fs::read_to_string(&path).map_err(|e| format!("could not read {path:?}: {e}"))?
    };
    let capture: vellum_perf::Capture =
        serde_json::from_str(&json).map_err(|e| format!("malformed capture {path:?}: {e}"))?;

    let scenario = flag(args, "--scenario")?.unwrap_or_else(|| capture.scenario.clone());
    let baseline_file = project_phoenix::perf::baseline_path(&scenario);

    match project_phoenix::perf::load_baseline(std::path::Path::new(&baseline_file)) {
        Ok(None) => {
            // Not an error: a scenario is measured before anyone has an
            // opinion about its numbers.
            eprintln!("phoenix-perf: no baseline at {baseline_file}; nothing to compare");
            Ok(())
        }
        Ok(Some(baseline)) => {
            let (_findings, rendered) = project_phoenix::perf::report(&capture, &baseline);
            println!("{rendered}");
            Ok(())
        }
        // A malformed baseline is a broken contract, not a slow build. Exit 1
        // rather than 2: the arguments were fine, the repository is not.
        Err(e) => {
            eprintln!("phoenix-perf: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_out(path: &str, contents: &str) -> Result<(), String> {
    match path {
        "-" => {
            println!("{contents}");
            Ok(())
        }
        path => std::fs::write(path, format!("{contents}\n"))
            .map_err(|e| format!("could not write {path:?}: {e}")),
    }
}
