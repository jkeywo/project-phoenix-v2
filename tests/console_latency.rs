//! Console input-to-feedback latency: end-to-end tap and determinism guard
//! (issue #1169, PRD #1144).
//!
//! Three claims the pure unit tests in `debug::console_latency` cannot make:
//!
//! 1. **The host tap works end-to-end.** Running the two bracketing systems over
//!    a ship with admitted commands produces a payload whose `SimHost` entries
//!    are keyed by the `SystemControlPayload` VARIANT — the action — and carry
//!    the `admit_to_broadcast` segment and nothing else.
//!
//! 2. **Measurement off costs nothing observable, and on changes no outcome.**
//!    Two seeded headless runs of the same world — one measuring, one not — fold
//!    to a byte-identical authoritative-state digest. This is a stronger claim
//!    than the station-activity guard beside it: there the counters run either
//!    way and only the publish is gated, so "on" and "off" were always doing the
//!    same work. Here the whole measurement is gated, so "off" genuinely takes
//!    no `Instant::now()` at all — and the digests must still match.
//!
//! 3. **The gating is not vacuous.** The measuring run really does produce a
//!    versioned payload with per-action distributions; without that, claim 2
//!    could pass simply because nothing ever ran.
//!
//! Follows the seeded-headless prior art in `tests/station_activity.rs` and
//! `tests/rng_determinism.rs`.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::core::messages::{
    AdmittedCommand, AdmittedCommands, LatencySurface, RepairTarget, SystemControlPayload, SystemId,
};
use project_phoenix::debug::console_latency::{record_admit_to_broadcast, stamp_admission_instant};
use project_phoenix::debug::{ConsoleLatencyCapture, ConsoleLatencyTracker, TickAdmissionInstant};
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::sim_digest::world_digest;

// ── The host tap, end-to-end ─────────────────────────────────────────────────

fn admitted(payload: SystemControlPayload) -> AdmittedCommand {
    AdmittedCommand {
        target: SystemId("repair".into()),
        payload,
        response_token: None,
    }
}

/// The two bracketing systems, run in order, file one sample per admitted
/// command under that command's ACTION — the payload variant name, never the
/// carried data (which would make every distinct value its own action).
#[test]
fn the_host_window_is_attributed_per_action() {
    let mut app = App::new();
    app.init_resource::<ConsoleLatencyTracker>();
    app.init_resource::<TickAdmissionInstant>();
    app.world_mut().spawn(AdmittedCommands(vec![
        admitted(SystemControlPayload::DispatchRepairTeam {
            team_idx: 0,
            target: RepairTarget::Core,
        }),
        admitted(SystemControlPayload::DispatchRepairTeam {
            // A different operand: still the same ACTION, so it must land in
            // the same series rather than opening a second one.
            team_idx: 1,
            target: RepairTarget::Core,
        }),
        admitted(SystemControlPayload::SetRepairPriority {
            team_idx: 0,
            priority: 2,
        }),
    ]));
    app.add_systems(
        Update,
        (stamp_admission_instant, record_admit_to_broadcast).chain(),
    );
    app.update();

    let payload = app.world().resource::<ConsoleLatencyTracker>().report();
    assert_eq!(
        payload.actions.len(),
        2,
        "two distinct actions, three commands: {payload:?}"
    );
    for entry in &payload.actions {
        assert_eq!(
            entry.surface,
            LatencySurface::SimHost,
            "the host measures only its own surface"
        );
        let segment = entry
            .admit_to_broadcast
            .as_ref()
            .expect("the host segment is the one it can observe");
        assert!(
            segment.p50_ms >= 0.0 && segment.max_ms >= segment.p50_ms,
            "a distribution must be ordered and non-negative: {segment:?}"
        );
        // The host never sees a client's input event, so it must not claim to.
        assert!(entry.input_to_send.is_none());
        assert!(entry.send_to_ack.is_none());
        assert!(entry.input_to_ack.is_none());
    }

    let dispatch = payload
        .actions
        .iter()
        .find(|e| e.action == "DispatchRepairTeam")
        .expect("keyed by the payload variant name");
    assert_eq!(
        dispatch
            .admit_to_broadcast
            .as_ref()
            .expect("host segment")
            .count,
        2,
        "both dispatches are the same action despite different operands"
    );
}

/// Without the opening stamp there is no window, so the recorder files nothing
/// rather than inventing a duration from an unset clock. This is what makes the
/// `run_if` gate on the pair safe: skipping the stamp cannot leave the recorder
/// producing garbage.
#[test]
fn no_admission_stamp_means_no_sample() {
    let mut app = App::new();
    app.init_resource::<ConsoleLatencyTracker>();
    app.init_resource::<TickAdmissionInstant>();
    app.world_mut().spawn(AdmittedCommands(vec![admitted(
        SystemControlPayload::SetRepairPriority {
            team_idx: 0,
            priority: 1,
        },
    )]));
    app.add_systems(Update, record_admit_to_broadcast);
    app.update();

    assert!(
        app.world().resource::<ConsoleLatencyTracker>().is_empty(),
        "a recorder with no open window must record nothing"
    );
}

// ── Determinism: measuring never moves the digest ────────────────────────────

/// A fixed seed so both runs walk the identical RNG stream — the whole point.
const SEED: u64 = 0x4C41_5445_4E43_5900; // "LATENCY\0"
/// Long enough to reach `InProgress` and let the AI crew admit commands, short
/// enough to keep the test quick. Matches `tests/station_activity.rs`.
const TICKS: u64 = 600;

/// Build and run one seeded headless run, optionally measuring console latency.
/// Returns the final authoritative-state digest, the flag-gated capture JSON,
/// and the run report's own copy of the payload.
fn run_once(
    measure: bool,
) -> (
    u64,
    Option<String>,
    project_phoenix::debug::ConsoleLatencyPayload,
) {
    let args = HeadlessArgs {
        seed: Some(SEED),
        deterministic: true,
        max_ticks: TICKS,
        console_latency: measure,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).expect("headless app should build");
    run(&mut app, TICKS);
    let digest = world_digest(app.world());
    let captured = app.world().resource::<ConsoleLatencyCapture>().0.clone();
    let payload = app.world().resource::<ConsoleLatencyTracker>().report();
    (digest, captured, payload)
}

/// The AC guard: a seeded sweep produces byte-identical digests with measurement
/// on and off. Every stamp is wall-clock — the most non-deterministic input
/// there is — so this is the test that proves it never reaches authoritative
/// state.
#[test]
fn measuring_leaves_the_seeded_digest_identical() {
    let (digest_off, captured_off, payload_off) = run_once(false);
    let (digest_on, captured_on, payload_on) = run_once(true);

    assert_eq!(
        digest_off, digest_on,
        "measuring console latency moved the authoritative-state digest — a \
         wall-clock reading must never enter authoritative state"
    );

    // The gating is real, not vacuous. Off: no clock read, no sample, no
    // payload. On: a versioned payload with the host's own segment.
    assert!(
        captured_off.is_none(),
        "measurement disabled must publish nothing"
    );
    assert!(
        payload_off.actions.is_empty(),
        "measurement disabled must take no samples at all, not merely withhold \
         the publish: {payload_off:?}"
    );

    let json = captured_on.expect("measurement enabled must publish a payload");
    assert!(
        json.contains("\"schema_version\":1"),
        "the captured payload must carry the schema version; got: {json}"
    );
    assert!(
        !payload_on.actions.is_empty(),
        "a 600-tick AI-crewed run admits commands, so it must measure some"
    );
    for entry in &payload_on.actions {
        assert_eq!(
            entry.surface,
            LatencySurface::SimHost,
            "a headless run has no console client, so only the host segment exists"
        );
        assert!(
            entry.admit_to_broadcast.is_some(),
            "the host segment is the one a headless run can measure"
        );
    }
}
