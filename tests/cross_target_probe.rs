//! Issue #904's native half: the seeded probe run, the pin the browser is
//! compared against, and the mutation proof that the comparison works.
//!
//! # Why this is its own test binary
//!
//! The same reason `tests/entity_id_minting.rs`, `tests/rng_determinism.rs`
//! and `tests/registration_order_determinism.rs` are. Bevy's task pools are
//! **process-global** and created by whichever app in the process builds
//! first, so a claim about system ordering made under a scheduler some other
//! test file chose is not a claim about anything. Cargo gives each integration
//! test file its own process; that is the isolation this needs.
//!
//! Deliberately NOT gated on the `headless` feature. The probe world builds
//! from Rust literals with no filesystem, so it compiles and runs on the bare
//! `cargo test` CI already runs — which means the pin cannot rot behind a
//! feature flag nobody passes. That was a real failure mode: three determinism
//! suites in this directory had never run in CI at all until #915's gate pass
//! found them (see the comments in `.github/workflows/ci.yml`).
//!
//! # The re-bless procedure
//!
//! `tests/fixtures/cross-target-ledger.json` is the single place the pinned
//! digests live. Both this file and `tests/smoke/cross-target-determinism.
//! spec.js` read it; neither carries a copy. To move it after a *deliberate*
//! change — a widened fold, a changed probe world, a `libm` upgrade, a new
//! tick count:
//!
//! ```text
//! PHOENIX_BLESS_CROSS_TARGET_LEDGER=1 cargo test --test cross_target_probe
//! git diff tests/fixtures/cross-target-ledger.json
//! ```
//!
//! Then **verify the browser agrees before committing** — the whole value of
//! the pin is that it is cross-target, and re-blessing from native alone
//! reduces it to a native self-consistency check:
//!
//! ```text
//! TRUNK_BUILD_RELEASE=true trunk build --release && node scripts/build-client.mjs
//! cd tests/smoke && npx playwright test cross-target-determinism.spec.js
//! ```
//!
//! If the spec fails after a native-only re-bless, the two targets genuinely
//! disagree and that is the bug this slice exists to surface. Do not re-bless
//! from the browser's numbers to make it pass.

#![cfg(not(target_arch = "wasm32"))]

use project_phoenix::cross_target_probe::{
    build_probe_app, probe_end_tick, run_probe, ProbeConfig, ProbeReport, BURSTY_PACING,
    CHECKPOINT_INTERVAL, EVEN_PACING, PROBE_SEED, PROBE_TICKS,
};
use project_phoenix::entities::spawner::{EntitySystemHull, EntityUuid};
use project_phoenix::sim_digest::DigestLedger;
use project_phoenix::world_id::{IdNamespace, WorldId};

const LEDGER_PATH: &str = "tests/fixtures/cross-target-ledger.json";
const BLESS_ENV: &str = "PHOENIX_BLESS_CROSS_TARGET_LEDGER";

/// Rebuild a [`DigestLedger`] from a report, so the report the browser sends
/// and the ledger a native run produces can be compared through
/// `DigestLedger::first_divergence` — the comparator that names a tick.
fn ledger_from_report(report: &ProbeReport) -> DigestLedger {
    let mut ledger = DigestLedger::new(report.interval);
    for checkpoint in &report.checkpoints {
        let digest = u64::from_str_radix(&checkpoint.digest, 16).unwrap_or_else(|e| {
            panic!("checkpoint digest {:?} is not hex: {e}", checkpoint.digest)
        });
        ledger.record(checkpoint.tick, digest);
    }
    ledger.final_digest = u64::from_str_radix(&report.final_digest, 16)
        .unwrap_or_else(|e| panic!("final digest {:?} is not hex: {e}", report.final_digest));
    ledger
}

/// AC5, and the half of AC1 about delay: injecting delay changes *timing*,
/// never outcomes.
///
/// One instance advances one logical tick per `App::update()`; the other
/// stalls and catches up on a `1, 2, 3, 4, 2` cycle. Same seed, same command
/// log, and every checkpoint plus the final state must be bit-identical.
///
/// This is the native companion #895 proved for frame pacing on the headless
/// runner (`tests/headless_runner.rs`'s `--hz` invariance cases); what is new
/// here is that it runs on the *same* world the browser drives, so a pass here
/// and a pass in the smoke spec compose into the cross-target claim rather
/// than being two unrelated facts.
#[test]
fn injected_delay_changes_timing_and_nothing_else() {
    let even = run_probe(&ProbeConfig::paced(EVEN_PACING));
    let bursty = run_probe(&ProbeConfig::paced(BURSTY_PACING));

    assert!(
        even.checkpoints.len() > 1,
        "precondition: the run must have sampled more than one checkpoint, or \
         the equality below is nearly vacuous. Got {:?}",
        even.checkpoints
    );
    assert_eq!(
        even.checkpoints.len(),
        bursty.checkpoints.len(),
        "the two pacings sampled different checkpoint sets — every pacing \
         cycle must divide CHECKPOINT_INTERVAL so both land on every sampling \
         tick, or the comparison silently narrows to an intersection"
    );

    if let Some(divergence) = even.first_divergence(&bursty) {
        panic!(
            "even and bursty pacing diverged, so delay injection changed the \
             outcome and not merely the timing: {divergence}"
        );
    }
}

/// The run lands exactly on the pinned tick under both pacings.
///
/// Worth its own assertion: a bursty cycle that overshot `PROBE_TICKS` would
/// have the two instances comparing states taken one tick apart, and the
/// equality above would then be either a false pass or a false failure
/// depending on whether that tick happened to change anything.
#[test]
fn every_pacing_stops_on_the_same_tick() {
    for (label, pacing) in [("even", EVEN_PACING), ("bursty", BURSTY_PACING)] {
        let ledger = run_probe(&ProbeConfig::paced(pacing));
        assert_eq!(
            probe_end_tick(&ledger),
            PROBE_TICKS,
            "{label} pacing finished on tick {} instead of {PROBE_TICKS} — a \
             pacing cycle no longer divides the run length exactly",
            probe_end_tick(&ledger)
        );
    }
}

/// AC4: the comparison is proven by mutation.
///
/// A single ULP is added to one ship's forward speed at a chosen tick — the
/// smallest cross-instance difference an `f32` can carry. The comparison must
/// catch it, and must locate it at the first checkpoint at or after the tick
/// it was injected on, not merely report that the two runs ended differently.
///
/// Reverting is the *absence* of the knob: the unmutated pair is compared
/// again at the end of this test, which is the "revert, confirm equality
/// restored" half of the AC as an assertion rather than as a note in a commit
/// message.
#[test]
fn a_one_ulp_mutation_is_caught_at_the_tick_it_happens() {
    let clean = run_probe(&ProbeConfig::paced(EVEN_PACING));

    for mutate_at in [37_u64, 100, 205] {
        let mutated = run_probe(&ProbeConfig {
            mutate_at: Some(mutate_at),
            ..ProbeConfig::paced(EVEN_PACING)
        });

        let divergence = clean.first_divergence(&mutated).unwrap_or_else(|| {
            panic!(
                "a one-ULP difference injected at tick {mutate_at} was not \
                 caught at all — the comparison has a tolerance in it, or the \
                 fold is not reading the perturbed field"
            )
        });

        // The first checkpoint at or after the injection. Not the injection
        // tick itself unless it happens to be a sampling tick: nothing is
        // sampled in between, and claiming otherwise would be claiming a
        // resolution the ledger does not have.
        let expected = mutate_at.div_ceil(CHECKPOINT_INTERVAL) * CHECKPOINT_INTERVAL;
        assert!(
            !divergence.at_end,
            "the mutation at tick {mutate_at} was only noticed by comparing \
             final states ({divergence}) — it should have been located at a \
             checkpoint"
        );
        assert_eq!(
            divergence.tick, expected,
            "a mutation injected at tick {mutate_at} was reported at tick {} \
             rather than at the first sampled tick after it ({expected}): \
             {divergence}",
            divergence.tick
        );
    }

    // Revert: no knob, equality restored.
    let reverted = run_probe(&ProbeConfig::paced(EVEN_PACING));
    assert_eq!(
        clean.first_divergence(&reverted),
        None,
        "with the mutation removed the two runs must agree again"
    );
}

/// The pin, and the fixture's blessing path.
///
/// `tests/fixtures/cross-target-ledger.json` is the ONE place the expected
/// digests live; `tests/smoke/cross-target-determinism.spec.js` reads the same
/// file rather than carrying a second copy that could drift. This test is what
/// keeps the file honest against native, and the smoke spec is what keeps it
/// honest against wasm.
///
/// See this file's module docs for the full re-bless procedure — in short,
/// `PHOENIX_BLESS_CROSS_TARGET_LEDGER=1 cargo test --test cross_target_probe`,
/// then rebuild `dist/` and run the smoke spec before committing.
#[test]
fn the_committed_ledger_matches_this_build() {
    let cfg = ProbeConfig::paced(EVEN_PACING);
    let ledger = run_probe(&cfg);
    let report = ProbeReport::from_ledger(&ledger, &cfg, "even");

    if std::env::var(BLESS_ENV).is_ok() {
        let json = serde_json::to_string_pretty(&report).expect("report serialises");
        std::fs::create_dir_all("tests/fixtures").expect("fixtures dir");
        std::fs::write(LEDGER_PATH, format!("{json}\n")).expect("write ledger");
        eprintln!("re-blessed {LEDGER_PATH} — now verify the browser agrees before committing");
        return;
    }

    let raw = std::fs::read_to_string(LEDGER_PATH).unwrap_or_else(|e| {
        panic!(
            "could not read {LEDGER_PATH}: {e}. If the fixture is genuinely \
             missing, re-create it with {BLESS_ENV}=1 cargo test --test \
             cross_target_probe — and read this file's module docs first."
        )
    });
    let pinned: ProbeReport =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{LEDGER_PATH} is not a report: {e}"));

    assert_eq!(
        (pinned.seed, pinned.ticks, pinned.interval),
        (PROBE_SEED, PROBE_TICKS, CHECKPOINT_INTERVAL),
        "the committed ledger describes a different run shape than this build \
         asks for — re-bless it rather than comparing two different runs"
    );

    if let Some(divergence) = ledger_from_report(&pinned).first_divergence(&ledger) {
        panic!(
            "this build no longer reproduces the pinned cross-target ledger: \
             {divergence}\n\nIf the change to the fold, the probe world or \
             simmath was deliberate, re-bless with {BLESS_ENV}=1 and then \
             confirm the browser still agrees (see the module docs). If it was \
             not, the simulation has stopped being reproducible on native, \
             which is the regression this file exists to catch."
        );
    }
}

/// Preconditions: the probe world is not a comparison of two empty sets.
///
/// Every equality above would pass on a world where nothing spawned, nothing
/// moved and nothing collided. These are the assertions that say it did.
#[test]
fn the_probe_world_actually_simulates_something() {
    let cfg = ProbeConfig::paced(EVEN_PACING);
    let mut app = build_probe_app(&cfg);
    let period = std::time::Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    for _ in 0..PROBE_TICKS {
        app.update();
    }

    let ids: Vec<String> = {
        let mut q = app.world_mut().query::<&EntityUuid>();
        q.iter(app.world()).map(|u| u.0.clone()).collect()
    };
    assert!(
        ids.len() >= 7,
        "the world should hold five start-up ships plus two mid-run mints; got {}",
        ids.len()
    );
    for id in &ids {
        let parsed = WorldId::parse(id)
            .unwrap_or_else(|| panic!("probe ship id {id:?} is not a tick-scoped mint"));
        assert_eq!(
            parsed.namespace,
            IdNamespace::Entity,
            "probe ships must mint in the Entity namespace ({id})"
        );
    }
    assert!(
        ids.iter()
            .any(|id| WorldId::parse(id).is_some_and(|w| w.tick > 0)),
        "no ship was minted after tick 0 — the mid-run spawns did not run, so \
         the fold never had to order a late arrival against an early one"
    );

    let damaged = {
        let mut q = app.world_mut().query::<&EntitySystemHull>();
        q.iter(app.world())
            .any(|hull| hull.0.total_current() < hull.0.total_max())
    };
    assert!(
        damaged,
        "no ship took hull damage over {PROBE_TICKS} ticks — the ships never \
         collided, so rapier's broadphase and the seeded damage distribution \
         contributed nothing to the digest and the cross-target claim is \
         narrower than it reads"
    );
}

/// The digest reads the run, rather than being a constant that would agree
/// with anything. Without this, every equality above is vacuous.
#[test]
fn a_different_seed_moves_the_digest() {
    let pinned = run_probe(&ProbeConfig::paced(EVEN_PACING));
    let other = run_probe(&ProbeConfig {
        seed: PROBE_SEED + 1,
        ..ProbeConfig::paced(EVEN_PACING)
    });
    assert!(
        pinned.first_divergence(&other).is_some(),
        "a different seed reached an identical ledger — the fold is not \
         reading the seeded run at all"
    );
}
