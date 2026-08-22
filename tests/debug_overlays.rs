//! The four migrated legacy debug overlays — integration + determinism guard
//! (issue #1150, PRD #1144).
//!
//! Two claims the per-surface pure unit tests (in `debug::damage`,
//! `debug::entities`, `debug::inspector` and `modifiers::cache`) cannot make:
//!
//! 1. **The publish path works end-to-end and is flag-gated.** Driving
//!    `publish_damage_debug` over a populated `DamageLog` produces a captured
//!    JSON payload carrying the recorded facts when the flag is on, and writes
//!    nothing when the flag is absent — the "one debug system, structured JSON,
//!    flag-gated" contract every migrated surface follows.
//!
//! 2. **Enabling any of the four surfaces never moves the digest.** Two seeded
//!    headless runs of the same world — one with all four overlay flags on, one
//!    off — fold to a byte-identical authoritative-state digest. Each surface is
//!    a read-only projection off authoritative/presentation state, so enabling
//!    the JSON publish is inert to the fold. This mirrors the station-activity
//!    guard in `tests/station_activity.rs`.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::debug::damage::publish_damage_debug;
use project_phoenix::debug::{
    DamageDebugCapture, EntityBehaviorCapture, EntityInspectorCapture, ModifierDebugCapture,
};
use project_phoenix::debug_overlay::{
    DamageLog, DamageLogEntry, DebugDamageEnabled, DebugEntitiesEnabled,
    DebugEntityInspectorEnabled, DebugOverlayEnabled,
};
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::sim_digest::world_digest;

// ── The damage surface, end-to-end through the publish system ────────────────

/// With the damage flag on, the publish system projects the ring buffer into a
/// versioned JSON payload carrying each event's source, arc and amount.
#[test]
fn damage_publish_captures_the_log_as_json_when_enabled() {
    let mut app = App::new();
    app.init_resource::<DamageDebugCapture>();
    app.insert_resource(DebugDamageEnabled(true));
    let mut log = DamageLog::default();
    log.push(DamageLogEntry {
        source: "asteroid-7".into(),
        shield_arc: Some("Fore".into()),
        amount: 8.0,
    });
    app.insert_resource(log);
    app.add_systems(Update, publish_damage_debug);
    app.update();

    let json = app
        .world()
        .resource::<DamageDebugCapture>()
        .0
        .clone()
        .expect("an enabled damage surface must publish a payload");
    assert!(
        json.contains("\"schema_version\":1"),
        "payload must carry the schema version; got {json}"
    );
    assert!(
        json.contains("asteroid-7"),
        "the source must survive; got {json}"
    );
    assert!(
        json.contains("\"shield_arc\":\"Fore\""),
        "the shield arc must survive; got {json}"
    );
    assert!(
        json.contains("\"amount\":8.0"),
        "the amount must survive; got {json}"
    );
}

/// With no `DebugDamageEnabled` resource present (the headless default), the
/// publish system short-circuits and writes nothing — the gating is real.
#[test]
fn damage_publish_writes_nothing_when_the_flag_is_absent() {
    let mut app = App::new();
    app.init_resource::<DamageDebugCapture>();
    app.insert_resource(DamageLog::default());
    app.add_systems(Update, publish_damage_debug);
    app.update();

    assert!(
        app.world().resource::<DamageDebugCapture>().0.is_none(),
        "a surface whose flag is absent must publish nothing"
    );
}

// ── Determinism: enabling the four surfaces never moves the digest ───────────

/// A fixed seed so both runs walk the identical RNG stream.
const SEED: u64 = 0x4f56_4552_4c41_5953; // "OVERLAYS"
/// Long enough to reach `InProgress` and take damage / spawn NPCs, short enough
/// to stay quick.
const TICKS: u64 = 600;

/// One seeded headless run, optionally with all four legacy-overlay flags on.
/// Returns the final authoritative-state digest and whether each surface's
/// capture was written.
fn run_once(overlays_enabled: bool) -> (u64, [bool; 4]) {
    let args = HeadlessArgs {
        seed: Some(SEED),
        deterministic: true,
        max_ticks: TICKS,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).expect("headless app should build");
    if overlays_enabled {
        // These flags are only DECLARED on a headless run; inserting them here
        // turns the four flag-gated publishes on for every frame of this run.
        app.insert_resource(DebugOverlayEnabled(true));
        app.insert_resource(DebugDamageEnabled(true));
        app.insert_resource(DebugEntitiesEnabled(true));
        app.insert_resource(DebugEntityInspectorEnabled(true));
    }
    run(&mut app, TICKS);
    let digest = world_digest(app.world());
    let captured = [
        app.world().resource::<ModifierDebugCapture>().0.is_some(),
        app.world().resource::<DamageDebugCapture>().0.is_some(),
        app.world().resource::<EntityBehaviorCapture>().0.is_some(),
        app.world().resource::<EntityInspectorCapture>().0.is_some(),
    ];
    (digest, captured)
}

/// The AC guard: a seeded sweep produces byte-identical digests with the four
/// overlays on and off. Enabling structured debug output is a read-only
/// projection that cannot perturb the simulation or its digest.
#[test]
fn enabling_the_legacy_overlays_leaves_the_seeded_digest_identical() {
    let (digest_off, captured_off) = run_once(false);
    let (digest_on, captured_on) = run_once(true);

    assert_eq!(
        digest_off, digest_on,
        "enabling the migrated debug overlays moved the authoritative-state digest \
         — each must be a read-only projection off authoritative state"
    );

    // The gating is real, not vacuous: off writes nothing, on writes every
    // surface. Without this the digest equality could pass simply because the
    // publishes never ran.
    assert_eq!(
        captured_off, [false; 4],
        "every surface must publish nothing while its flag is absent"
    );
    assert_eq!(
        captured_on, [true; 4],
        "every surface must publish a payload while its flag is on"
    );
}
