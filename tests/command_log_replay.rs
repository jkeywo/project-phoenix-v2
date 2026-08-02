//! The acceptance issue #898 actually owes: **a seed plus its command log
//! reproduce the run**, at this slice's scope.
//!
//! # Why this is its own test binary
//!
//! The same reason `tests/rng_determinism.rs` is, and it is not a style
//! preference. `--deterministic` pins the scheduler by handing `TaskPoolPlugin`
//! a one-thread `TaskPoolOptions`, but Bevy's task pools are **process-global**
//! and created by whichever app in the process builds first. Dropped into
//! `tests/headless_runner.rs`, this file's three seeded app builds join a race
//! with forty-odd other tests over who fixes the pool — and the loser is
//! whichever combat-chaotic duel probe then runs under a scheduler it was not
//! blessed against. Observed, not theorised: adding these runs to that binary
//! turned `world_spawned_alliance_hull_returns_fire_and_the_duel_resolves` and
//! `balance_logging_systems_run_with_an_enabled_filter_and_the_duel_resolves`
//! red while leaving this test green.
//!
//! Cargo gives every integration-test file its own process, which is what makes
//! a byte-equality claim mean what it says. Keep this file to the one test.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::command_admission::ai_emit::AI_BACKFILL_TOKEN;
use project_phoenix::command_admission::{CommandLog, ShipKey};
use project_phoenix::entity_spawner::EntityUuid;
use project_phoenix::headless::{build_headless_app, build_report, HeadlessArgs};
use project_phoenix::lobby::InboundMessage;
use project_phoenix::messages::{ClientMessage, SystemControlPayload, SystemId};
use project_phoenix::server_app::LocalShip;
use project_phoenix::sim_tick::SimTick;
use std::collections::VecDeque;

/// One command to push across the boundary, and the tick to push it on.
type Injection = (u64, SystemId, SystemControlPayload);

/// What one run leaves behind.
struct Run {
    json: String,
    log: CommandLog,
    /// The `LocalShip`'s uuid — the key every entry should carry.
    local_uuid: String,
}

/// The scenario's fixed inputs.
///
/// `patrol.toml` for the same reason
/// `the_simulation_reaches_the_same_state_at_wildly_different_frame_rates` uses
/// it: its backfilled player flies a deterministic non-contact course, so rapier
/// — still frame-driven until #896 — never feeds a collision back into ship
/// state. Both runs here are driven at identical frame pacing, so rapier is not
/// strictly a hazard; the real reason is that a quiet scenario keeps the
/// comparison about the command log rather than about a chaotic pursuit.
fn replay_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 260,
        seed: Some(898_2026),
        deterministic: true,
        ..Default::default()
    }
}

/// Drive a whole run, pushing each injection through `InboundMessage` on the
/// tick it names.
///
/// The queue is drained on `tick >= injection.tick` rather than `==` because a
/// frame can run zero fixed steps (the first frame establishes the time
/// baseline and runs none), so the same `SimTick` value can be observed at the
/// top of two consecutive frames. Popping makes each injection happen exactly
/// once, whichever frame catches it.
///
/// The commands go in under `AI_BACKFILL_TOKEN` because a headless run has
/// nobody connected, so that is the token the shipped authority check accepts;
/// the point being made is about the *boundary*, not about who was on the far
/// side of it. Re-injection uses it too, which is exactly the seam a `ShipKey`
/// records: the log names the destination *ship*, and the harness — like #901's
/// driver — supplies a credential that routes there. The sender's session token
/// was deliberately never written down (see `command_admission::log`).
fn drive(mut queue: VecDeque<Injection>) -> Run {
    let args = replay_args();
    let mut app = build_headless_app(&args).expect("app should build");
    app.finish();
    app.cleanup();
    for _ in 0..args.max_ticks {
        let tick = app.world().resource::<SimTick>().0;
        while queue.front().is_some_and(|(at, _, _)| *at <= tick) {
            let (_, target, payload) = queue.pop_front().expect("just checked the front");
            app.world_mut()
                .resource_mut::<Messages<InboundMessage>>()
                .write(InboundMessage {
                    token: AI_BACKFILL_TOKEN.into(),
                    msg: ClientMessage::ControlSystem { target, payload },
                });
        }
        app.update();
    }
    assert!(
        queue.is_empty(),
        "the run ended before every injection was pushed — max_ticks is too \
         low for the ticks the log names"
    );

    let local_uuid = {
        let mut q = app
            .world_mut()
            .query_filtered::<&EntityUuid, With<LocalShip>>();
        q.iter(app.world())
            .next()
            .map(|u| u.0.clone())
            .expect("the player ship carries an EntityUuid")
    };
    let log = app.world().resource::<CommandLog>().clone();
    // `wall_seconds` of 0.0 for the same reason `rng_determinism.rs` passes it:
    // the derived timing fields are measurements, and no two runs could ever
    // match byte for byte on those.
    let json = build_report(&mut app, &args, 0.0).to_json();
    Run {
        json,
        log,
        local_uuid,
    }
}

/// Run once with commands pushed across the production boundary and keep the
/// log. Run the same seed again from scratch, re-injecting each `LoggedCommand`
/// on the tick it was recorded for — and nothing else. The two runs must produce
/// byte-identical `build_report` output.
///
/// # What this does and does not claim
///
/// It is not the replay *driver* — that is #901, and it needs the snapshot
/// boundary, a way to drive the app from a file, and divergence reporting. What
/// is proved here is the property the driver will rest on and cannot repair:
/// that the log is a *sufficient* record of a run's input. Everything the second
/// run does differently, it does because of the log; everything else it
/// re-derives from the seed. If the log were missing a command, carrying the
/// wrong tick, or reordering two commands within a tick, the reports would
/// diverge.
///
/// The third run is what stops the first two being a tautology. With no commands
/// at all the same seed must reach a *different* report — otherwise the injected
/// commands never mattered, and two identical runs of an untouched simulation
/// would have proved only what `rng_determinism.rs` already proves.
#[test]
fn a_seed_and_its_command_log_reproduce_the_run() {
    let helm_thrust = project_phoenix::ship::system_registry::helm_thrust_system_id();
    let red_alert = SystemId("red-alert".into());
    // Well past the auto-start countdown, and spread so the record has to get
    // both the across-tick order and the within-tick order right. Two commands
    // share tick 182.
    let script: VecDeque<Injection> = VecDeque::from(vec![
        (
            150,
            red_alert.clone(),
            SystemControlPayload::SetRedAlert { active: true },
        ),
        (
            170,
            helm_thrust.clone(),
            SystemControlPayload::SetThrust { value: -1.0 },
        ),
        (
            182,
            helm_thrust.clone(),
            SystemControlPayload::SetThrust { value: 1.0 },
        ),
        (
            182,
            red_alert.clone(),
            SystemControlPayload::SetRedAlert { active: false },
        ),
        (
            215,
            helm_thrust.clone(),
            SystemControlPayload::SetThrust { value: -0.5 },
        ),
    ]);

    let recorded = drive(script.clone());
    assert_eq!(
        recorded.log.len(),
        script.len(),
        "precondition: every injected command must have crossed the boundary \
         and been accepted — a refused one is silently absent from the log, and \
         the replay below would then be re-injecting a shorter script"
    );
    assert!(
        recorded
            .log
            .entries()
            .iter()
            .all(|e| e.ship == ShipKey(recorded.local_uuid.clone())),
        "every entry must name the ship admission routed it to, by uuid — that \
         key is the whole of what makes an entry re-routable. Got: {:?}",
        recorded.log.entries()
    );
    assert!(
        !recorded.json.is_empty() && recorded.log.ticks_are_monotonic(),
        "precondition: a usable report and an in-order log"
    );

    // The replay: the ONLY input is what the log recorded.
    let replayed = drive(
        recorded
            .log
            .entries()
            .iter()
            .map(|e| (e.tick, e.target.clone(), e.payload.clone()))
            .collect(),
    );
    assert_eq!(
        recorded.json, replayed.json,
        "the same seed replayed with the same command log must reach a \
         byte-identical end state — this is what 'log + seed reproduces the \
         run' means at this slice's scope"
    );
    assert_eq!(
        replayed.log.entries(),
        recorded.log.entries(),
        "and the replay must record the same log it was driven from, or the log \
         is not a fixed point and a second replay would drift"
    );

    // The control: without the commands the same seed must land somewhere else,
    // or the comparison above proves nothing.
    let untouched = drive(VecDeque::new());
    assert!(
        untouched.log.is_empty(),
        "the control run injected nothing, so it must record nothing"
    );
    assert_ne!(
        recorded.json,
        untouched.json,
        "the same seed with NO commands produced the same report as one with {} \
         commands — the injected commands changed nothing observable, so the \
         equality above is a tautology rather than a replay",
        script.len()
    );
}
