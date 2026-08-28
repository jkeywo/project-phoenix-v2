//! Does the game actually fit through the relay? (issue #1113)
//!
//! `assets/join/join-codes.toml` authors `max_relay_frame_bytes`, and both ends
//! of the WebSocket game relay refuse anything larger — the service at
//! `worker-rendezvous/src/relay.js`, the phone at `gui/rendezvous-relay.js`, the
//! native host at `src/native_host/relay_transport.rs`. Until this file existed
//! the bound's justification was the word "comfortably" in a TOML comment, and
//! it was wrong: the number was a quarter of the DataChannel ceiling the same
//! payloads already have to clear, and it named the wrong payload class as the
//! one that approaches it.
//!
//! So this measures. It boots the SHIPPED worlds through the ordinary headless
//! simulation, drains the real outbound bus, encodes with the real codec, and
//! asserts the largest frame of each delivery class fits the authored bound.
//! Nothing here fabricates a payload: every byte counted is a byte the wire
//! would carry.
//!
//! It is an integration test rather than an inline module for the reason
//! `tests/headless_runner.rs` states at length — building a headless app
//! populates the process-global native template cache.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

mod common;

use bevy::prelude::*;
use project_phoenix::core::codec::{JsonCodec, MessageCodec};
use project_phoenix::core::messages::{DeliveryClass, ServerMessage};
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::lobby::OutboundMessage;

/// The authored ceiling, read from the table a designer edits rather than
/// copied into this file. A tuning edit that lowered it below what the game
/// emits has to fail here, which it cannot do against a hardcoded twin.
fn authored_max_relay_frame_bytes() -> usize {
    let text = std::fs::read_to_string("assets/join/join-codes.toml")
        .expect("assets/join/join-codes.toml is shipped");
    // Read the assignment rather than deserialising the whole table: the file
    // is mostly commentary and a deny-list, and a scan for the one key cannot
    // break on an unrelated schema change.
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == "max_relay_frame_bytes")
        .and_then(|(_, value)| value.split('#').next().unwrap_or("").trim().parse().ok())
        .expect("[limits] max_relay_frame_bytes is authored in assets/join/join-codes.toml")
}

/// The largest encoded payload of each delivery class a run of `world` emits.
struct FrameBudget {
    reliable: (usize, String),
    snapshot: (usize, String),
}

/// Collector for the outbound bus, so the run's real traffic can be measured
/// rather than a reconstruction of it.
#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut outbox: ResMut<Outbox>) {
    for m in reader.read() {
        outbox.0.push(m.clone());
    }
}

/// Boot `world`, run it into the mission, and measure what it put on the wire.
///
/// 600 ticks at the default 60 Hz is ten seconds of simulated mission: past the
/// lobby auto-start, past the one-off `WorldSetup`, and well into the per-tick
/// snapshot cadence with the asteroid lifecycle running.
fn budget_for(world: &str) -> FrameBudget {
    let args = HeadlessArgs {
        world_path: world.into(),
        max_ticks: 600,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).expect("a shipped world should build a headless app");
    app.init_resource::<Outbox>();
    app.add_systems(Last, collect);
    run(&mut app, args.max_ticks);

    let mut budget = FrameBudget {
        reliable: (0, String::new()),
        snapshot: (0, String::new()),
    };
    for entry in &app.world().resource::<Outbox>().0 {
        let encoded = JsonCodec
            .encode_server(&entry.msg)
            .expect("every outbound message encodes");
        // The same measurement the relay makes: encoded UTF-8 bytes.
        let bytes = encoded.len();
        let name = variant_name(&entry.msg);
        let slot = match entry.delivery {
            DeliveryClass::Reliable => &mut budget.reliable,
            DeliveryClass::Snapshot => &mut budget.snapshot,
        };
        if bytes > slot.0 {
            *slot = (bytes, name);
        }
    }
    budget
}

/// The variant name, for a failure message that says WHICH payload class blew
/// the budget — the whole practical value of this test on the day it fails.
fn variant_name(msg: &ServerMessage) -> String {
    format!("{msg:?}")
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("ServerMessage")
        .to_string()
}

/// The worlds worth measuring, and why each earns its place.
///
/// `falling_skyway` authors the most `[[entity]]` blocks of any shipped world
/// (17), so it is the biggest `WorldSetup` the game can produce — the class the
/// TOML comment used to reason about. `combat_test` is the asteroid-heavy one,
/// so it is the biggest per-tick snapshot — the class that ACTUALLY approaches
/// the bound, and the one the comment did not mention.
const MEASURED_WORLDS: [&str; 2] = [
    "assets/worlds/falling_skyway.toml",
    "assets/worlds/combat_test.toml",
];

#[test]
fn every_shipped_frame_fits_the_authored_relay_ceiling() {
    let ceiling = authored_max_relay_frame_bytes();
    for world in MEASURED_WORLDS {
        let budget = budget_for(world);
        assert!(
            budget.reliable.0 > 0,
            "{world} emitted no reliable traffic at all — the run never reached the mission, \
             so this test measured nothing"
        );
        assert!(
            budget.reliable.0 <= ceiling,
            "{world}: the largest reliable frame is {} bytes ({}) and the relay carries at most \
             {ceiling}. A reliable frame over the ceiling is not shed — it FAILS THE LINK on \
             both hosts — so this is a mission that works on a direct link and cannot be played \
             at all on a restrictive network.",
            budget.reliable.0,
            budget.reliable.1,
        );
        assert!(
            budget.snapshot.0 <= ceiling,
            "{world}: the largest snapshot frame is {} bytes ({}) and the relay carries at most \
             {ceiling}. Snapshot frames over the ceiling are shed, so a relayed player would see \
             a frozen picture with a healthy-looking readout.",
            budget.snapshot.0,
            budget.snapshot.1,
        );
        eprintln!(
            "{world}: largest reliable {} bytes ({}), largest snapshot {} bytes ({}), ceiling {ceiling}",
            budget.reliable.0, budget.reliable.1, budget.snapshot.0, budget.snapshot.1,
        );
    }
}

#[test]
fn the_relay_ceiling_is_not_below_the_datachannel_one() {
    // The point of the number, stated as an assertion rather than as a comment.
    // A DataChannel's SDP-negotiated `max-message-size` is 262144 bytes between
    // two Chromiums — documented in gui/rendezvous-transport.js's send() — and a
    // relay ceiling below it means a payload that crosses a good network cannot
    // cross a bad one. That is a mission that works direct and fails relayed:
    // exactly the failure the fallback exists to prevent, arrived at from
    // inside it.
    const DATACHANNEL_MAX_MESSAGE_SIZE: usize = 262_144;
    assert!(
        authored_max_relay_frame_bytes() >= DATACHANNEL_MAX_MESSAGE_SIZE,
        "max_relay_frame_bytes is below the DataChannel ceiling the same payloads already fit"
    );
}
