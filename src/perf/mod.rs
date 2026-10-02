//! Performance measurement (issue #868).
//!
//! The measurement *contract* — series, summaries, provenance, baselines,
//! comparison, rendering — is `vellum-perf`, and phoenix is its second
//! consumer. What lives here is the part the crate excludes by charter: the
//! collectors, and where the baseline files live.
//!
//! Collectors, one contract:
//!
//! - [`tick`] — the headless harness loop, native.
//! - [`native_frames`] — opt-in native App cadence and fixed-update catch-up.
//! - [`assets`] — the shipped asset inventory, native, no run required.
//! - [`mesh`] — the mesh interior read through Bevy's own loader, native.
//! - [`browser`] — boot, preload and frame timing in the browser host, wasm.
//! - [`console`] — the simulation's admission→broadcast service window (issue
//!   #1169), native. The odd one out: it takes no measurement of its own, it
//!   bridges samples the PRD #1144 debug pipeline already holds into a
//!   `Recorder`, so one run cannot report two different numbers for one thing.
//!
//! [`baseline`] is not a collector: recording a baseline
//! *from* a capture, so the numbers a runner is held to are the numbers that
//! runner produced.
//!
//! Collection stays out of the simulation. Every collector here samples from
//! outside authoritative state, so a measured run and an unmeasured run
//! produce the same simulation.
//!
//! Values are benchmark evidence, not assertions. Nothing in the test suite
//! asserts on a duration; the tests cover the pure machinery around them.
//!
//! # Recording a baseline (issue #905, revised for the move-fast demo phase)
//!
//! Baselines are recorded on the machine that compares against them, and the CI
//! runner commits them: a scenario with no committed baseline yet is
//! bootstrapped by the perf job, which records the missing file from its own
//! capture and commits it on a push to `main` (with `[skip ci]`, so the
//! baseline commit does not start another run). An *existing* baseline is never
//! moved by CI — a baseline that changes is a budget decision, so it still
//! moves only through a deliberate `adopt` that lands in a reviewable diff:
//!
//! ```text
//! gh run download <run-id> -n perf-capture -D target/perf-artifact
//! cargo run --release --features perf --bin phoenix-perf -- \
//!     adopt --artifact target/perf-artifact
//! git diff perf/baselines
//! ```
//!
//! Adoption records the measurement and leaves the judgement alone: an
//! existing expectation keeps its statistic and tolerances and only its
//! `expected` moves. See [`baseline`].
//!
//! # When measurement gates (issue #905, revised for the demo phase)
//!
//! Whether a scenario's [`vellum_perf::Verdict::Fail`] *can* become a build
//! gate honestly still turns on both of these being true of it:
//!
//! 1. **Its metrics are a function of the checkout, not of the host.** Bytes on
//!    disk, LOD ladder depth, triangle counts and texture dimensions are the
//!    same on every machine that reads the same commit, so a drift is a real
//!    change to what a player downloads or what a GPU is handed. Wall-clock
//!    metrics are not: a shared runner's neighbours move them.
//! 2. **Its baseline was recorded by a machine whose measurement the comparing
//!    runner reproduces.** For a metric that passes (1) that is every machine,
//!    and the runner's own captures are the proof. For a wall-clock metric it
//!    is only a machine of the same class the baseline was recorded on.
//!
//! But *where* gating happens has moved. During the move-fast demo phase:
//!
//! - **The regular CI perf job (`.github/workflows/ci.yml`) gates on nothing.**
//!   Every scenario is warnings-first there; adverse drift from a baseline is
//!   filed as a GitHub issue to fix later, never a red build and never a silent
//!   re-record. The job stays out of `deploy`'s `needs`, as before.
//! - **The manual demo deploy (`.github/workflows/deploy-demo.yml`) is the
//!   gate.** It runs `phoenix-perf report --gate` on all four scenarios —
//!   including the wall-clock ones — and a fail blocks the demo deploy before
//!   it ships. A demo deploy is human-dispatched, so a wall-clock gate that
//!   fires on runner noise costs a re-dispatch rather than a disabled gate;
//!   that is the trade the demo accepts to ship a build whose budgets were
//!   actually checked. Its browser and headless captures are taken the same way
//!   CI takes them (native release binary; the smoke perf spec under WebDriver)
//!   so the comparison against the runner-recorded baseline stays valid.
//!
//! `Verdict::Incomparable` gates wherever `Fail` does — a metric that vanished
//! from a capture is a broken contract, and passing it is how a budget stops
//! being enforced without anyone deciding to stop enforcing it. `Warn` never
//! gates; the whole tolerance design assumes a warning is read — now, filed —
//! rather than obeyed.
//!
//! The mechanism is `phoenix-perf report --gate`, which is off unless asked
//! for; `deploy-demo.yml` is the workflow that asks.

#[cfg(not(target_arch = "wasm32"))]
pub mod assets;
#[cfg(not(target_arch = "wasm32"))]
pub mod baseline;
#[cfg(target_arch = "wasm32")]
pub mod browser;
#[cfg(not(target_arch = "wasm32"))]
pub mod console;
#[cfg(not(target_arch = "wasm32"))]
pub mod mesh;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod native_frames;
/// Pure phase-interval reduction for the external headless producer (#1400).
#[cfg(not(target_arch = "wasm32"))]
pub mod phase;
/// Worker-visible span collection owned by the headless harness, never the App.
#[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
pub mod phase_trace;
#[cfg(not(target_arch = "wasm32"))]
pub mod tick;

use vellum_perf::Profile;

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use vellum_perf::{Baseline, Capture, Finding};

/// Where committed baselines live. One file per scenario, RON, reviewable in
/// a diff — the crate owns the types, this repository owns the layout.
pub const BASELINE_DIR: &str = "perf/baselines";

/// Which side of an expectation is adverse for a metric.
///
/// `vellum-perf` intentionally compares by absolute drift. Phoenix adds this
/// policy at its boundary because a smaller download or faster frame is an
/// improvement, while a deeper LOD ladder is improved by growing. Unknown
/// metrics retain vellum's symmetric comparison until their meaning is made
/// explicit here.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetricDirection {
    LowerIsBetter,
    HigherIsBetter,
    Symmetric,
}

#[cfg(not(target_arch = "wasm32"))]
fn metric_direction(metric: &str) -> MetricDirection {
    match metric {
        "sim.tick"
        | "sim.run"
        // Console input-to-feedback (issue #1169): a longer wait between a
        // player's tap and the simulation's answer is unambiguously worse.
        | "sim.console_ack"
        | "browser.boot"
        | "browser.preload"
        | "browser.frame"
        | "assets.glb.bytes"
        | "assets.glb.total.bytes"
        | "assets.glb.without_lod"
        | "assets.mesh.triangles"
        | "assets.mesh.triangles.total"
        | "assets.texture.count"
        | "assets.texture.pixels" => MetricDirection::LowerIsBetter,
        "assets.lod.levels" => MetricDirection::HigherIsBetter,
        _ => MetricDirection::Symmetric,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn normalize_direction(findings: &mut [Finding]) {
    for finding in findings {
        // Incomparability is a broken measurement contract, not drift, and
        // must never be normalized into a pass.
        if finding.verdict == vellum_perf::Verdict::Incomparable {
            continue;
        }
        let favourable = match metric_direction(&finding.metric) {
            MetricDirection::LowerIsBetter => finding.got <= finding.expected,
            MetricDirection::HigherIsBetter => finding.got >= finding.expected,
            MetricDirection::Symmetric => false,
        };
        if favourable {
            finding.verdict = vellum_perf::Verdict::Pass;
        }
    }
}

/// Build provenance for a capture, so two captures are only compared when
/// they are comparable.
///
/// `device` and `rev` come from the environment because only the environment
/// knows them: CI sets `GITHUB_SHA` and identifies its runner image, and a
/// developer's desktop is not a runner. An unset value stays empty rather
/// than guessing — the contract is that provenance is honest, not complete.
#[cfg(not(target_arch = "wasm32"))]
pub fn profile(runtime: &str) -> Profile {
    Profile {
        runtime: runtime.to_string(),
        build: build_flavour().to_string(),
        device: std::env::var("PHOENIX_PERF_DEVICE")
            .or_else(|_| std::env::var("RUNNER_OS"))
            .unwrap_or_default(),
        rev: std::env::var("GITHUB_SHA").unwrap_or_default(),
    }
}

/// The wasm build has no environment to read, so provenance the host knows
/// (the page's own build stamp, the runner) is supplied by the caller.
#[cfg(target_arch = "wasm32")]
pub fn profile(runtime: &str) -> Profile {
    Profile {
        runtime: runtime.to_string(),
        build: build_flavour().to_string(),
        device: String::new(),
        rev: String::new(),
    }
}

fn build_flavour() -> &'static str {
    if cfg!(debug_assertions) {
        "dev"
    } else {
        "release"
    }
}

/// The committed baseline path for one scenario.
#[cfg(not(target_arch = "wasm32"))]
pub fn baseline_path(scenario: &str) -> String {
    format!("{BASELINE_DIR}/{scenario}.ron")
}

/// Errors reading a baseline. A missing file is not one of them — see
/// [`load_baseline`].
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub enum BaselineError {
    Io(String, std::io::Error),
    // Boxed: RON's spanned error carries its whole error enum plus a position,
    // and an unboxed variant makes every Ok result pay for the failure case.
    Parse(String, Box<ron::error::SpannedError>),
}

#[cfg(not(target_arch = "wasm32"))]
impl std::fmt::Display for BaselineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BaselineError::Io(path, e) => write!(f, "could not read baseline {path:?}: {e}"),
            BaselineError::Parse(path, e) => write!(f, "malformed baseline {path:?}: {e}"),
        }
    }
}

/// Read a baseline, or `None` when the scenario has no committed baseline yet.
///
/// A scenario measured before anyone has an opinion about its numbers is the
/// normal first state, not an error — the same reasoning that makes an
/// unbaselined metric produce no finding.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_baseline(path: &Path) -> Result<Option<Baseline>, BaselineError> {
    let display = path.display().to_string();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(BaselineError::Io(display, e)),
    };
    ron::from_str(&text)
        .map(Some)
        .map_err(|e| BaselineError::Parse(display, Box::new(e)))
}

/// The comparison report, warnings-first.
///
/// Returns the findings and their rendering, and leaves the decision about
/// exit codes to the caller — #868 is explicit that measurement informs
/// optimisation before it gates correctness. The module documentation above
/// records which scenarios have since earned a gate, and why the rest have
/// not.
#[cfg(not(target_arch = "wasm32"))]
pub fn report(capture: &Capture, baseline: &Baseline) -> (Vec<Finding>, String) {
    let mut findings = vellum_perf::compare(capture, baseline);
    normalize_direction(&mut findings);
    let rendered = vellum_perf::render(&findings);
    (findings, rendered)
}

/// Whether these findings should fail a build, *for a caller that asked to
/// gate*. Nothing here decides to ask.
///
/// `Incomparable` gates alongside `Fail`: a baselined metric missing from the
/// capture, or arriving in the wrong unit, means the budget was not checked at
/// all — and letting that through is how a gate stops gating without anyone
/// deciding to stop. `Warn` never gates; the whole tolerance design assumes a
/// warning is read rather than obeyed.
#[cfg(not(target_arch = "wasm32"))]
pub fn gates(findings: &[Finding]) -> bool {
    matches!(
        vellum_perf::worst(findings),
        vellum_perf::Verdict::Fail | vellum_perf::Verdict::Incomparable
    )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "tests.rs"]
mod tests;
