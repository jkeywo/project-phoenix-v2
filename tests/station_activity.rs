//! Station-activity tracker integration + determinism guard (issue #1145,
//! PRD #1144).
//!
//! Two claims the pure unit tests in `debug::station_activity` cannot make:
//!
//! 1. **The tap works end-to-end.** Driving `record_station_activity` over a
//!    ship with mixed control sources produces a payload whose per-station,
//!    per-source counts match the admitted commands — the human-vs-AI split the
//!    surface exists for, re-derived from the authoritative control-source
//!    resolver rather than from any smuggled command identity.
//!
//! 2. **Enabling capture never moves the digest.** Two seeded headless runs of
//!    the same world — one with the station-activity flag on, one off — fold to a
//!    byte-identical authoritative-state digest. Capture is a read-only
//!    projection; the counters are always-on in both runs and inert to the fold.
//!    This follows the seeded-headless prior art in `tests/rng_determinism.rs`.

use bevy::prelude::*;

use project_phoenix::core::messages::{
    AdmittedCommand, AdmittedCommands, RepairTarget, SystemControlPayload, SystemId,
};
use project_phoenix::debug::station_activity::record_station_activity;
use project_phoenix::debug::{
    DebugStationActivityEnabled, StationActivityCapture, StationActivityTracker,
};
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::ship::config::ShipConfig;
use project_phoenix::ship::control_source::{ControlSource, ControlSourceResolver};
use project_phoenix::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use project_phoenix::sim_digest::world_digest;
use project_phoenix::sim_tick::SimTick;

// ── The control-source split, end-to-end through the tap ─────────────────────

/// A two-station hull: `helm` and `weapons` each own one system, so
/// `station_for_system` resolves each command's target to a distinct station.
/// Both systems use `repair_control` as their kind because the tracker cares
/// only about the target→station mapping and the resolver's source, never the
/// payload — the kind just has to be a declared one.
fn two_station_config() -> ShipConfig {
    ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = "Steering."
rank = "Ltn."

[[station]]
id = "weapons"
name = "Tactical"
description = "Guns."
rank = "Ltn."

[[system]]
id = "helm"
kind = "repair_control"
station = "helm"

[[system]]
id = "weapons"
kind = "repair_control"
station = "weapons"
"#,
        &["repair_control"],
    )
    .expect("two-station config parses")
}

fn admitted(target: &str) -> AdmittedCommand {
    AdmittedCommand {
        target: SystemId(target.into()),
        // Payload is irrelevant to activity counting — the tracker keys on the
        // target's station and the system's control source, never the payload.
        payload: SystemControlPayload::DispatchRepairTeam {
            team_idx: 0,
            target: RepairTarget::Core,
        },
        response_token: None,
    }
}

/// Given admitted commands on a ship whose `helm` is human-crewed and `weapons`
/// is AI-crewed, the payload counts each command under the right station AND the
/// right source — the split re-derived from the authoritative resolver.
#[test]
fn admitted_commands_are_counted_by_station_and_control_source() {
    let mut resolver = ControlSourceResolver::new();
    resolver.set(SystemId("helm".into()), ControlSource::Human);
    resolver.set(SystemId("weapons".into()), ControlSource::Ai);

    let commands = AdmittedCommands(vec![
        admitted("helm"),    // human
        admitted("helm"),    // human
        admitted("weapons"), // ai
    ]);

    let mut app = App::new();
    app.insert_resource(SimTick(5));
    app.init_resource::<StationActivityTracker>();
    app.world_mut().spawn((
        commands,
        ShipSystemControlSources(resolver),
        ShipConfigComponent(two_station_config()),
    ));
    app.add_systems(Update, record_station_activity);
    app.update();

    let payload = app.world().resource::<StationActivityTracker>().report();

    // Exactly one bucket (default one-tick buckets, tick 5), opened at tick 5.
    assert_eq!(
        payload.buckets.len(),
        1,
        "one bucket for the one recorded tick"
    );
    let bucket = &payload.buckets[0];
    assert_eq!(bucket.start_tick, 5);
    assert_eq!(
        bucket.stations.len(),
        2,
        "two distinct stations were worked"
    );

    // Sorted by station id: helm before weapons.
    let helm = &bucket.stations[0];
    assert_eq!(helm.station, "helm");
    assert_eq!(helm.human, 2, "two human helm commands");
    assert_eq!(helm.ai, 0, "helm is human-crewed, no AI work");

    let weapons = &bucket.stations[1];
    assert_eq!(weapons.station, "weapons");
    assert_eq!(weapons.ai, 1, "one AI weapons command");
    assert_eq!(weapons.human, 0, "weapons is AI-crewed, no human work");
}

// ── Determinism: enabling capture never moves the digest ─────────────────────

/// A fixed seed so both runs walk the identical RNG stream — the whole point.
const SEED: u64 = 0x5741_4354_4956_4954; // "WACTIVIT"
/// Long enough to reach `InProgress` and let the AI crew admit commands, short
/// enough to keep the test quick.
const TICKS: u64 = 600;

/// Build and run one seeded headless run, optionally with station-activity
/// capture enabled. Returns the final authoritative-state digest and the
/// captured JSON (if any).
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
        // flag-gated JSON publish on for every InProgress tick of this run.
        app.insert_resource(DebugStationActivityEnabled(true));
    }
    run(&mut app, TICKS);
    let digest = world_digest(app.world());
    let captured = app.world().resource::<StationActivityCapture>().0.clone();
    (digest, captured)
}

/// The AC guard: a seeded sweep produces byte-identical digests with capture on
/// and off. Enabling debug output is a read-only projection that cannot perturb
/// the simulation or its digest.
#[test]
fn enabling_capture_leaves_the_seeded_digest_identical() {
    let (digest_off, captured_off) = run_once(false);
    let (digest_on, captured_on) = run_once(true);

    assert_eq!(
        digest_off, digest_on,
        "enabling station-activity capture moved the authoritative-state digest \
         — capture must be a read-only projection off authoritative state"
    );

    // The gating is real, not vacuous: capture off writes nothing, capture on
    // writes a versioned payload. Without this, the digest equality above could
    // pass simply because the publish never ran.
    assert!(
        captured_off.is_none(),
        "capture disabled must publish nothing"
    );
    let json = captured_on.expect("capture enabled must publish a payload");
    assert!(
        json.contains("\"schema_version\":1"),
        "the captured payload must carry the schema version; got: {json}"
    );
    assert!(
        json.contains("\"buckets\""),
        "the captured payload must carry the bucket series; got: {json}"
    );
}
