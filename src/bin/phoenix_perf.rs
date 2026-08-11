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
//! asks it to — see the gating decision in `src/perf/mod.rs`.

#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("phoenix-perf is a native binary; it has no wasm32 build.");
}

#[cfg(not(target_arch = "wasm32"))]
const HELP: &str = "\
phoenix-perf — asset budgets and baseline comparison.

USAGE:
    phoenix-perf assets [--root <DIR>] [--capture <PATH>]
    phoenix-perf mesh   [--root <DIR>] [--capture <PATH>]
    phoenix-perf report --capture <PATH> [--scenario <NAME>] [--gate]
    phoenix-perf adopt  (--capture <PATH> [--out <PATH>]
                        | --artifact <DIR> [--out-dir <DIR>])

assets      Walk assets/models and assets/entities and write a capture of the
            shipped byte counts and LOD coverage. No simulation runs, so the
            same checkout always produces the same numbers.
    --root <DIR>       Repository root to walk [default: .]
    --capture <PATH>   Where to write the capture JSON ('-' for stdout)

mesh        Resolve entity templates and their rig sidecars, then load every
            runtime-reachable GLB level through Bevy's own asset loader and
            write a capture of the triangle and texture counts it produced.
            Headless — no window, no renderer, no simulation — but the loader
            really runs, so this reads what the engine makes of a file rather
            than a second opinion about its bytes.
    --root <DIR>       Repository root to load from [default: .]
    --capture <PATH>   Where to write the capture JSON ('-' for stdout)

report      Compare a capture against perf/baselines/<scenario>.ron and render
            the findings. Warnings-first by default: the exit code is 0
            whatever the verdict, because measurement informs optimisation
            before it gates.
    --capture <PATH>   Capture JSON to read ('-' for stdin)
    --scenario <NAME>  Baseline to compare against [default: the capture's own
                       scenario name]
    --gate             Exit 3 on a fail or an incomparable finding. Only for
                       scenarios the gating decision in src/perf/mod.rs says
                       have earned it.

adopt       Record a baseline from a capture, so the numbers a runner is held
            to are the numbers that runner produced. An existing baseline's
            statistics and tolerances carry over untouched — only the expected
            values move.
    --capture <PATH>   Capture JSON to adopt ('-' for stdin)
    --out <PATH>       Where to write the baseline ('-' for stdout)
                       [default: perf/baselines/<scenario>.ron]
    --artifact <DIR>   Adopt every capture in a downloaded CI artifact
    --out-dir <DIR>    Where --artifact writes its baselines
                       [default: perf/baselines]

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
        "mesh" => mesh(&args[1..]),
        "report" => report(&args[1..]),
        "adopt" => adopt(&args[1..]),
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

/// Whether a valueless flag is present.
#[cfg(not(target_arch = "wasm32"))]
fn switch(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// Reject anything that is not a known flag.
///
/// `switches` take no value and `known` take one, so the walk has to know
/// which is which — a value-taking flag consumes the token after it, and a
/// switch does not. Getting that wrong would either read a switch's successor
/// as a value or read a value as an unknown argument.
#[cfg(not(target_arch = "wasm32"))]
fn reject_unknown(args: &[String], known: &[&str], switches: &[&str]) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if switches.contains(&arg) {
            i += 1;
        } else if known.contains(&arg) {
            i += 2;
        } else {
            return Err(format!("unknown argument {arg:?} (try --help)"));
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn assets(args: &[String]) -> Result<(), String> {
    reject_unknown(args, &["--root", "--capture"], &[])?;
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

/// The mesh interior, through Bevy's loader (issue #905).
#[cfg(not(target_arch = "wasm32"))]
fn mesh(args: &[String]) -> Result<(), String> {
    reject_unknown(args, &["--root", "--capture"], &[])?;
    let root = flag(args, "--root")?.unwrap_or_else(|| ".".to_string());
    let out = flag(args, "--capture")?.unwrap_or_else(|| "-".to_string());

    let found = project_phoenix::perf::mesh::measure(std::path::Path::new(&root))
        .map_err(|e| e.to_string())?;
    let capture = project_phoenix::perf::mesh::capture(
        &found,
        project_phoenix::perf::profile(project_phoenix::perf::mesh::RUNTIME),
    );
    write_out(&out, &capture.to_json())
}

#[cfg(not(target_arch = "wasm32"))]
fn report(args: &[String]) -> Result<(), String> {
    reject_unknown(args, &["--capture", "--scenario"], &["--gate"])?;
    let path = flag(args, "--capture")?.ok_or("report requires --capture")?;
    let gate = switch(args, "--gate");
    let capture = read_capture(&path)?;

    let scenario = flag(args, "--scenario")?.unwrap_or_else(|| capture.scenario.clone());
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
                     (see src/perf/mod.rs for which scenarios gate, and why)"
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
fn adopt(args: &[String]) -> Result<(), String> {
    reject_unknown(
        args,
        &["--capture", "--out", "--artifact", "--out-dir"],
        &[],
    )?;
    match (flag(args, "--capture")?, flag(args, "--artifact")?) {
        (Some(_), Some(_)) => Err("adopt takes --capture or --artifact, not both".to_string()),
        (None, None) => Err("adopt requires --capture or --artifact".to_string()),
        (Some(path), None) => {
            let capture = read_capture(&path)?;
            let out = flag(args, "--out")?
                .unwrap_or_else(|| project_phoenix::perf::baseline_path(&capture.scenario));
            adopt_one(&capture, &out)
        }
        (None, Some(dir)) => {
            let out_dir = flag(args, "--out-dir")?
                .unwrap_or_else(|| project_phoenix::perf::BASELINE_DIR.to_string());
            adopt_artifact(&dir, &out_dir)
        }
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
