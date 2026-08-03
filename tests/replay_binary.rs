//! Issue #901 review: the binary-path evidence the tests over `PhoenixSim`
//! directly cannot give.
//!
//! `tests/replay_simulation.rs` proves the *library* keeps the replay
//! contract — `drive_run`/`replay_artifact` reproduce each other's digests.
//! It never runs `phoenix-headless` itself, so it cannot catch a bug in the
//! bin's own argument wiring (`record()`/`replay()` in
//! `src/bin/phoenix_headless.rs`) or in the CLI-facing exit codes
//! (`--replay`'s documented exit 4 on divergence). This file closes that gap
//! by invoking the built binary directly, the way a user actually would:
//! `--record` then `--replay`, checking the process exit code and the
//! printed verdict rather than a library return value.
//!
//! It also closes the "record-run report equivalence" claim the module doc on
//! `src/headless/replay.rs` makes in prose (a recording run is driven through
//! `PhoenixSim` rather than `run_sampled`, and is claimed to still produce the
//! same exit report) but that nothing before this file asserted on: a plain
//! run and a `--record` run of the identical seeded scenario must print
//! byte-identical reports, because `--seed` zeroes every timing field that
//! could otherwise make two runs of the same input differ.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use std::process::Command;

/// The compiled `phoenix-headless` binary, built by Cargo before this test
/// runs because it is this crate's own `[[bin]]` target (Cargo sets
/// `CARGO_BIN_EXE_<name>` for exactly this reason).
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_phoenix-headless")
}

/// A scratch path under the OS temp dir, unique to this test process so
/// parallel test runs never collide.
fn scratch_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "phoenix-headless-test-{}-{name}",
        std::process::id()
    ))
}

/// The scenario's fixed inputs — small and quiet, so the test is fast and the
/// comparison is about the plumbing rather than about combat chaos. Mirrors
/// `tests/replay_simulation.rs`'s own `args()` scenario (`patrol.toml`) at a
/// much shorter length, since this file is checking CLI wiring, not digest
/// coverage over a long run.
const WORLD: &str = "assets/worlds/patrol.toml";
const SHIP: &str = "assets/entities/alliance_cruiser.toml";
const SEED: &str = "901955";
const TICKS: &str = "50";

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("phoenix-headless should spawn")
}

/// The binary path, end to end: `--record` writes an artifact, `--replay`
/// consumes it, and the verdict is a clean exit 0 that says every checkpoint
/// (and the digest) agreed — the same claim `tests/replay_simulation.rs`
/// makes against the library, now made against the shipped binary a user
/// actually runs.
#[test]
fn record_then_replay_reproduces_and_exits_clean() {
    let artifact = scratch_path("record-then-replay.ron");
    let artifact_str = artifact.to_string_lossy().to_string();

    let record = run(&[
        "--world",
        WORLD,
        "--ship",
        SHIP,
        "--seed",
        SEED,
        "--ticks",
        TICKS,
        "--digest-every",
        "10",
        "--record",
        &artifact_str,
        "--report",
        "-",
    ]);
    assert!(
        record.status.success(),
        "recording should exit clean; stderr: {}",
        String::from_utf8_lossy(&record.stderr)
    );
    assert!(
        artifact.exists(),
        "--record must have written the artifact file"
    );

    let replay = run(&["--replay", &artifact_str]);
    let stdout = String::from_utf8_lossy(&replay.stdout);
    let stderr = String::from_utf8_lossy(&replay.stderr);
    assert!(
        replay.status.success(),
        "a replay of its own recording must exit 0 (exit 4 is reserved for a \
         genuine divergence); stdout: {stdout:?}, stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("reproduced the recording") && stdout.contains("all"),
        "the verdict must say the replay reproduced the recording; got {stdout:?}"
    );

    let _ = std::fs::remove_file(&artifact);
}

/// The other half of AC7 at the binary level: `--replay` on an artifact whose
/// world the recording never matches must exit 4, not 0 — the documented exit
/// code contract in `src/bin/phoenix_headless.rs`'s own header comment.
///
/// The corruption here is simpler than `tests/replay_simulation.rs`'s
/// payload-tamper (which needs library access to `LoggedCommand`): editing the
/// artifact's own recorded seed on disk is available from outside the crate,
/// and a different seed draws a different RNG stream from tick zero, which
/// this quiet non-contact scenario has no way to converge back from.
#[test]
fn a_tampered_artifact_exits_four_on_replay() {
    let artifact = scratch_path("tampered.ron");
    let artifact_str = artifact.to_string_lossy().to_string();

    let record = run(&[
        "--world",
        WORLD,
        "--ship",
        SHIP,
        "--seed",
        SEED,
        "--ticks",
        TICKS,
        "--digest-every",
        "10",
        "--record",
        &artifact_str,
        "--report",
        "-",
    ]);
    assert!(record.status.success());

    let original = std::fs::read_to_string(&artifact).expect("artifact readable");
    // Flip the recorded seed only — every other field (world, ship, ticks,
    // dt, log, ledger) stays exactly what the honest recording wrote.
    let tampered = original.replacen(&format!("seed: {SEED}"), "seed: 424242", 1);
    assert_ne!(
        tampered, original,
        "precondition: the seed field must actually be found and replaced"
    );
    std::fs::write(&artifact, tampered).expect("can overwrite the scratch artifact");

    let replay = run(&["--replay", &artifact_str]);
    assert_eq!(
        replay.status.code(),
        Some(4),
        "a replay that does not reproduce its artifact must exit 4; stdout: {:?}, stderr: {:?}",
        String::from_utf8_lossy(&replay.stdout),
        String::from_utf8_lossy(&replay.stderr)
    );

    let _ = std::fs::remove_file(&artifact);
}

/// The "recording and plain runs take the same path" claim, made byte-level:
/// a `--record` run (driven through `PhoenixSim`) must print the exact same
/// exit report as a plain run (driven through `run_sampled`) of the identical
/// seeded scenario — this is the evidence for the module doc's claim in
/// `src/headless/replay.rs` that a recording is not a different simulation
/// from an ordinary run, merely the same one with its inputs written down.
/// `--seed` is what makes this a byte-for-byte claim rather than a fuzzy one:
/// it zeroes `wall_seconds`/`ticks_per_second`/`speedup_vs_realtime`, the only
/// fields two runs of the same seeded scenario could otherwise differ on.
#[test]
fn a_record_runs_report_is_byte_identical_to_a_plain_runs() {
    let artifact = scratch_path("report-equivalence.ron");
    let artifact_str = artifact.to_string_lossy().to_string();

    let recorded = run(&[
        "--world",
        WORLD,
        "--ship",
        SHIP,
        "--seed",
        SEED,
        "--ticks",
        TICKS,
        "--record",
        &artifact_str,
        "--report",
        "-",
    ]);
    assert!(recorded.status.success());

    let plain = run(&[
        "--world", WORLD, "--ship", SHIP, "--seed", SEED, "--ticks", TICKS, "--report", "-",
    ]);
    assert!(plain.status.success());

    assert_eq!(
        recorded.stdout,
        plain.stdout,
        "a --record run and a plain run of the identical seeded scenario must \
         print byte-identical reports — recorded: {:?}, plain: {:?}",
        String::from_utf8_lossy(&recorded.stdout),
        String::from_utf8_lossy(&plain.stdout)
    );

    let _ = std::fs::remove_file(&artifact);
}
