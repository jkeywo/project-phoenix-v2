//! `phoenix-perf` — asset budgets, and the capture-versus-baseline report.
//!
//! One binary for the halves of #868 and #905 that need no simulation:
//! extracting the static asset inventory, reading the mesh interior through
//! Bevy's own loader, comparing any capture (from here, from the headless
//! harness, or pulled out of a browser session) against its committed
//! baseline, and recording a baseline back from a capture.
//!
//! Exit codes: 0 whatever the verdict, 1 IO/parse failure, 2 bad arguments,
//! 3 a gated regression. The verdict only reaches the exit code when `--gate`
//! asks it to — see the gating decision in `crates/phoenix-simulation/src/perf/mod.rs`.

#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("phoenix-perf is a native binary; it has no wasm32 build.");
}

#[cfg(not(target_arch = "wasm32"))]
use clap::{CommandFactory, Parser};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Parser, Debug)]
#[command(
    name = "phoenix-perf",
    args_override_self = true,
    about = "Measure asset budgets and compare captures with committed baselines",
    after_help = "Exit codes: 0 success or ungated verdict, 1 malformed baseline, 2 bad arguments or tool failure, 3 gated regression. Captures use '-' for stdin or stdout; baseline output uses '-' for stdout."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<PerfCommand>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Subcommand, Debug)]
enum PerfCommand {
    /// Inventory shipped bytes and LOD coverage without running a simulation
    #[command(args_override_self = true)]
    Assets(MeasureArgs),
    /// Measure mesh triangles and textures through Bevy's headless asset loader
    #[command(args_override_self = true)]
    Mesh(MeasureArgs),
    /// Compare with perf/baselines/<scenario>.ron; warnings-only unless gated
    #[command(args_override_self = true)]
    Report(ReportArgs),
    /// Record expectations while preserving existing statistics and tolerances
    #[command(args_override_self = true)]
    Adopt(AdoptArgs),
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Args, Debug)]
struct MeasureArgs {
    /// Repository root
    #[arg(long, default_value = ".")]
    root: String,
    /// Capture JSON destination ('-' for stdout)
    #[arg(long, default_value = "-")]
    capture: String,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Args, Debug)]
struct ReportArgs {
    /// Capture JSON to read ('-' for stdin)
    #[arg(long)]
    capture: String,
    /// Baseline name (default: capture's scenario)
    #[arg(long)]
    scenario: Option<String>,
    /// Exit 3 on a failed or incomparable finding for a gating scenario
    #[arg(long)]
    gate: bool,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Args, Debug)]
#[group(skip)]
#[command(group(clap::ArgGroup::new("source").required(true).multiple(false)))]
struct AdoptArgs {
    /// Capture JSON to adopt ('-' for stdin)
    #[arg(long, group = "source")]
    capture: Option<String>,
    /// Downloaded CI artifact directory containing captures
    #[arg(long, group = "source")]
    artifact: Option<String>,
    /// Baseline output (default: perf/baselines/<scenario>.ron; '-' for stdout)
    #[arg(long)]
    out: Option<String>,
    /// Artifact baseline directory (default: perf/baselines)
    #[arg(long)]
    out_dir: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            std::process::exit(code);
        }
    };
    let result = match cli.command {
        Some(PerfCommand::Assets(args)) => assets(args),
        Some(PerfCommand::Mesh(args)) => mesh(args),
        Some(PerfCommand::Report(args)) => report(args),
        Some(PerfCommand::Adopt(args)) => adopt(args),
        None => {
            print!("{}", Cli::command().render_long_help());
            return;
        }
    };
    if let Err(error) = result {
        eprintln!("phoenix-perf: {error}");
        std::process::exit(2);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn assets(args: MeasureArgs) -> Result<(), String> {
    let MeasureArgs { root, capture: out } = args;

    let found = project_phoenix::perf::assets::inventory(std::path::Path::new(&root))
        .map_err(|e| e.to_string())?;
    let capture = project_phoenix::perf::assets::capture(
        &found,
        project_phoenix::perf::profile(project_phoenix::perf::assets::RUNTIME),
    );
    write_out(&out, &capture.to_json())
}

/// The mesh interior, through Bevy's loader (issue #905).
#[cfg(not(target_arch = "wasm32"))]
fn mesh(args: MeasureArgs) -> Result<(), String> {
    let MeasureArgs { root, capture: out } = args;

    let found = project_phoenix::perf::mesh::measure(std::path::Path::new(&root))
        .map_err(|e| e.to_string())?;
    let capture = project_phoenix::perf::mesh::capture(
        &found,
        project_phoenix::perf::profile(project_phoenix::perf::mesh::RUNTIME),
    );
    write_out(&out, &capture.to_json())
}

#[cfg(not(target_arch = "wasm32"))]
fn report(args: ReportArgs) -> Result<(), String> {
    let ReportArgs {
        capture: path,
        scenario,
        gate,
    } = args;
    let capture = read_capture(&path)?;

    let scenario = scenario.unwrap_or_else(|| capture.scenario.clone());
    let baseline_file = project_phoenix::perf::baseline_path(&scenario);

    match project_phoenix::perf::load_baseline(std::path::Path::new(&baseline_file)) {
        Ok(None) => {
            // Not an error: a scenario is measured before anyone has an
            // opinion about its numbers. Not a gate either — a gate asked for
            // against a baseline nobody has written yet would fail every
            // build for a scenario with no budget.
            eprintln!("phoenix-perf: no baseline at {baseline_file}; nothing to compare");
            Ok(())
        }
        Ok(Some(baseline)) => {
            let (findings, rendered) = project_phoenix::perf::report(&capture, &baseline);
            println!("{rendered}");
            if gate && project_phoenix::perf::gates(&findings) {
                eprintln!(
                    "phoenix-perf: {scenario} is a gating scenario and its budget was not met \
                     (see crates/phoenix-simulation/src/perf/mod.rs for which scenarios gate, and why)"
                );
                std::process::exit(3);
            }
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

/// Record baselines from captures (issue #905).
#[cfg(not(target_arch = "wasm32"))]
fn adopt(args: AdoptArgs) -> Result<(), String> {
    match (args.capture, args.artifact) {
        (Some(path), None) => {
            let capture = read_capture(&path)?;
            let out = args
                .out
                .unwrap_or_else(|| project_phoenix::perf::baseline_path(&capture.scenario));
            adopt_one(&capture, &out)
        }
        (None, Some(dir)) => {
            let out_dir = args
                .out_dir
                .unwrap_or_else(|| project_phoenix::perf::BASELINE_DIR.to_string());
            adopt_artifact(&dir, &out_dir)
        }
        _ => unreachable!("Clap requires exactly one adoption source"),
    }
}

/// Adopt every capture in a downloaded CI artifact directory.
///
/// A JSON file that is not a capture is skipped rather than fatal: the same
/// artifact carries run reports and rendered baselines, and an adoption that
/// died on the first of those would be unusable against the thing CI actually
/// uploads.
#[cfg(not(target_arch = "wasm32"))]
fn adopt_artifact(dir: &str, out_dir: &str) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("could not list {dir:?}: {e}"))?;
    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort();

    std::fs::create_dir_all(out_dir).map_err(|e| format!("could not create {out_dir:?}: {e}"))?;

    let mut adopted = 0;
    for path in &paths {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let Ok(capture) = serde_json::from_str::<vellum_perf::Capture>(&text) else {
            continue;
        };
        // A capture with no scenario name has nowhere to be filed.
        if capture.scenario.is_empty() {
            eprintln!(
                "phoenix-perf: {} names no scenario; skipped",
                path.display()
            );
            continue;
        }
        let out = format!("{out_dir}/{}.ron", capture.scenario);
        adopt_one(&capture, &out)?;
        adopted += 1;
    }
    if adopted == 0 {
        return Err(format!("no captures found in {dir:?}"));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn adopt_one(capture: &vellum_perf::Capture, out: &str) -> Result<(), String> {
    // Read the file being replaced, so its statistics, tolerances and prose
    // survive. A destination that does not exist yet is the first recording.
    let existing_text = if out == "-" {
        None
    } else {
        std::fs::read_to_string(out).ok()
    };
    let existing = existing_text
        .as_deref()
        .and_then(|text| ron::from_str::<vellum_perf::Baseline>(text).ok());

    for metric in project_phoenix::perf::baseline::unmeasured(capture, existing.as_ref()) {
        eprintln!(
            "phoenix-perf: {out}: {metric:?} is expected but was not measured; kept unchanged"
        );
    }

    let baseline = project_phoenix::perf::baseline::adopt(capture, existing.as_ref());
    let rendered = project_phoenix::perf::baseline::render(
        &baseline,
        &capture.profile,
        existing_text.as_deref(),
    );
    if out != "-" {
        eprintln!(
            "phoenix-perf: recorded {} expectation(s) for {:?} into {out}",
            baseline.expectations.len(),
            baseline.scenario
        );
    }
    write_raw(out, &rendered)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_capture(path: &str) -> Result<vellum_perf::Capture, String> {
    let json = if path == "-" {
        let mut buffer = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)
            .map_err(|e| format!("could not read capture from stdin: {e}"))?;
        buffer
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("could not read {path:?}: {e}"))?
    };
    serde_json::from_str(&json).map_err(|e| format!("malformed capture {path:?}: {e}"))
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

/// `write_out` without the added newline — a rendered baseline already ends in
/// one, and a second would move in every diff that touched the file.
#[cfg(not(target_arch = "wasm32"))]
fn write_raw(path: &str, contents: &str) -> Result<(), String> {
    match path {
        "-" => {
            print!("{contents}");
            Ok(())
        }
        path => {
            std::fs::write(path, contents).map_err(|e| format!("could not write {path:?}: {e}"))
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "tests/phoenix_perf_cli_tests.rs"]
mod cli_tests;
