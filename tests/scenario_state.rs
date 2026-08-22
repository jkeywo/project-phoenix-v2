//! Scenario-state projector integration + determinism guard (issue #1148,
//! PRD #1144).
//!
//! Two claims the pure unit tests in `debug::scenario` cannot make:
//!
//! 1. **The publish runs end-to-end.** Driving a real headless run with the
//!    scenario-state flag on produces a versioned payload carrying every surface
//!    the projector collects — the transport from the world content runtime,
//!    through the codec seam, to the target-agnostic capture sink works on the
//!    headless target.
//!
//! 2. **Enabling capture never moves the digest.** Two seeded headless runs of
//!    the same world — one with the scenario-state flag on, one off — fold to a
//!    byte-identical authoritative-state digest. The projection is read-only off
//!    authoritative state and its capture is `StateClass::Presentation`, so it is
//!    inert to the fold. This follows the seeded-headless prior art in
//!    `tests/rng_determinism.rs` and `tests/station_activity.rs`.
//!
//! The projector's *content* — that a given authored world state produces the
//! expected flags/objectives/triggers/queues/commitments/dossiers — is asserted
//! by the pure unit tests in `src/debug/scenario.rs`, which build the authoritative
//! state directly rather than through a whole app.

use project_phoenix::debug::{DebugScenarioStateEnabled, ScenarioStateCapture};
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::sim_digest::world_digest;
use project_phoenix::sim_tick::SimTick;

/// A fixed seed so both runs walk the identical RNG stream — the whole point.
const SEED: u64 = 0x5343_454E_4152_494F; // "SCENARIO"
/// Long enough to reach `InProgress` and run the trigger pipeline, short enough
/// to keep the test quick.
const TICKS: u64 = 600;

/// Build and run one seeded headless run, optionally with scenario-state capture
/// enabled. Returns the final authoritative-state digest and the captured JSON
/// (if any).
fn run_once(capture_enabled: bool) -> (u64, Option<String>) {
    let args = HeadlessArgs {
        seed: Some(SEED),
        deterministic: true,
        max_ticks: TICKS,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).expect("headless app should build");
    if capture_enabled {
        // Overrides the default `false` `DebugPlugin` installed, turning the
        // flag-gated JSON projection on for every InProgress tick of this run.
        app.insert_resource(DebugScenarioStateEnabled(true));
    }
    run(&mut app, TICKS);
    let _ = app.world().resource::<SimTick>().0;
    let digest = world_digest(app.world());
    let captured = app.world().resource::<ScenarioStateCapture>().0.clone();
    (digest, captured)
}

/// The AC guard: a seeded sweep produces byte-identical digests with capture on
/// and off. Enabling the scenario-state surface is a read-only projection that
/// cannot perturb the simulation or its digest.
#[test]
fn enabling_capture_leaves_the_seeded_digest_identical() {
    let (digest_off, captured_off) = run_once(false);
    let (digest_on, captured_on) = run_once(true);

    assert_eq!(
        digest_off, digest_on,
        "enabling scenario-state capture moved the authoritative-state digest — \
         capture must be a read-only projection off authoritative state"
    );

    // The gating is real, not vacuous: capture off writes nothing, capture on
    // writes a versioned payload. Without this, the digest equality above could
    // pass simply because the projection never ran.
    assert!(
        captured_off.is_none(),
        "capture disabled must publish nothing"
    );
    let json = captured_on.expect("capture enabled must publish a payload");
    assert!(
        json.contains("\"schema_version\":1"),
        "the captured payload must carry the schema version; got: {json}"
    );
    // Every top-level surface is present (the fields are non-skipped), so the
    // panel and the report always find them even when a surface is empty.
    for key in [
        "\"flags\"",
        "\"objectives\"",
        "\"triggers\"",
        "\"delayed_actions\"",
        "\"deadlines\"",
        "\"commitments\"",
        "\"dossier\"",
    ] {
        assert!(
            json.contains(key),
            "the captured payload must carry {key}; got: {json}"
        );
    }
}
