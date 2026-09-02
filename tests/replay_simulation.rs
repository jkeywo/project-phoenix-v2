//! Issue #901's acceptance: the phoenix simulation **is** a
//! `vellum_replay::Simulation`, a recorded run replays to the same digest, and
//! a corrupted one says which tick it stopped agreeing on.
//!
//! # Why this is its own test binary
//!
//! The same reason `tests/command_log_replay.rs` and `tests/rng_determinism.rs`
//! are, and it is not a style preference. `--deterministic` pins the scheduler
//! by handing `TaskPoolPlugin` a one-thread `TaskPoolOptions`, but Bevy's task
//! pools are **process-global** and created by whichever app in the process
//! builds first. Dropped into `tests/headless_runner.rs`, these seeded builds
//! would join a race with forty-odd other tests over who fixes the pool, and
//! the loser is whichever combat-chaotic probe then runs under a scheduler it
//! was not blessed against.
//!
//! Cargo gives every integration-test file its own process, which is what makes
//! a digest-equality claim mean what it says.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::{IntoScheduleConfigs, Resource};
use project_phoenix::command_admission::log::{CommandOrder, HostSlot, LoggedCommand, ShipKey};
use project_phoenix::core::messages::{GamePhase, StationId, SystemControlPayload, SystemId};
use project_phoenix::gm_action::{
    GmAction, GmActionGrant, GmActionId, GmActionJournal, GmActionOrder, GmActionOutcome,
    GmActionRefusalReason,
};
use project_phoenix::headless::replay::{
    drive_run, drive_run_with_gm_actions, PhoenixSim, ReplayError,
};
use project_phoenix::headless::{verify_artifact, HeadlessArgs, ReplayArtifact};
use project_phoenix::lockstep::{
    FleetGm, FleetLockstep, FleetRoster, FleetShip, LockstepSession, PendingHostLoss,
};

/// How often the runs here sample a digest, in logical ticks.
///
/// Small enough that a 260-frame run takes several samples — the whole point is
/// that a divergence lands in a *window*, and a run with one checkpoint has no
/// window to land in.
const CHECKPOINT_EVERY: u64 = 25;

/// The scenario's fixed inputs.
///
/// `patrol.toml` for the same reason `tests/command_log_replay.rs` uses it: its
/// backfilled player flies a deterministic non-contact course, so a quiet
/// scenario keeps the comparison about the log and the digest rather than about
/// a chaotic pursuit.
fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 260,
        seed: Some(901_2026),
        deterministic: true,
        ..Default::default()
    }
}

fn command(tick: u64, target: SystemId, payload: SystemControlPayload) -> LoggedCommand {
    LoggedCommand {
        tick,
        // The fleet order a recording run stamped (issue #1116). A synthetic
        // script has no fleet, so it carries the solo order every lone host
        // mints — which is what the pre-#1116 arrival counter was.
        order: CommandOrder::default(),
        // The recording run fills this in from admission's own routing; what a
        // driver supplies is a credential derived from the TARGET (see
        // `replay::replay_token_for`), so the key a script carries is not what
        // routes it.
        ship: ShipKey::default(),
        target,
        payload,
    }
}

/// Well past the auto-start countdown, and spread so the record has to get both
/// the across-tick order and the within-tick order right. Two commands share
/// tick 182, exactly as `tests/command_log_replay.rs`'s script does.
fn script() -> Vec<LoggedCommand> {
    let helm_thrust = project_phoenix::ship::system_registry::helm_thrust_system_id();
    let red_alert = SystemId("red-alert".into());
    vec![
        command(
            150,
            red_alert.clone(),
            SystemControlPayload::SetRedAlert { active: true },
        ),
        command(
            170,
            helm_thrust.clone(),
            SystemControlPayload::SetThrust { value: -1.0 },
        ),
        command(
            182,
            helm_thrust.clone(),
            SystemControlPayload::SetThrust { value: 1.0 },
        ),
        command(
            182,
            red_alert,
            SystemControlPayload::SetRedAlert { active: false },
        ),
        command(
            215,
            helm_thrust,
            SystemControlPayload::SetThrust { value: -0.5 },
        ),
    ]
}

fn gm_grant(sequence: u64, apply_tick: u64, correlation: &str, active: bool) -> GmActionGrant {
    let from = HostSlot(2);
    GmActionGrant {
        from,
        sequenced_by: HostSlot(1),
        operator_id: "gm-replay".into(),
        correlation: GmActionId::new(correlation).expect("valid correlation"),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(from, sequence),
        action: GmAction::SetSessionPaused { active },
    }
}

fn station_grant(
    from: HostSlot,
    operator_id: &str,
    sequence: u64,
    apply_tick: u64,
    correlation: &str,
    action: GmAction,
) -> GmActionGrant {
    GmActionGrant {
        from,
        sequenced_by: HostSlot(1),
        operator_id: operator_id.into(),
        correlation: GmActionId::new(correlation).expect("valid correlation"),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(from, sequence),
        action,
    }
}

fn gm_journal(grants: impl IntoIterator<Item = GmActionGrant>) -> GmActionJournal {
    let mut journal = GmActionJournal::default();
    for grant in grants {
        journal.insert(grant).expect("canonical GM fixture");
    }
    journal
        .restore_applied_frontier(journal.len())
        .expect("the fixture applies every supplied grant");
    journal
}

#[derive(Resource, Clone)]
struct SeededLossGmActions(GmActionJournal);

#[derive(Resource, Clone)]
struct SeededSlotRecovery {
    slot: HostSlot,
    boundary: u64,
    applied: bool,
}

fn seed_loss_gm_actions(
    seed: bevy::prelude::Res<SeededLossGmActions>,
    mut journal: bevy::prelude::ResMut<GmActionJournal>,
) {
    *journal = seed.0.clone();
}

/// Test-only driver for the production ordering used by slot recovery: due GM
/// actions run first at the boundary, then the canonical generation advances
/// and `rejoin` clears only the barrier's transient departed bit.
fn apply_seeded_slot_recovery(world: &mut bevy::prelude::World) {
    let Some(seed) = world.get_resource::<SeededSlotRecovery>() else {
        return;
    };
    let tick = world
        .get_resource::<project_phoenix::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    if seed.applied || tick < seed.boundary {
        return;
    }
    let slot = seed.slot;
    let boundary = seed.boundary;
    world.resource_mut::<SeededSlotRecovery>().applied = true;
    world
        .resource_mut::<GmActionJournal>()
        .record_slot_recovery(slot, boundary)
        .expect("the seeded canonical recovery advances one generation");
    let mut session = world.resource_mut::<FleetLockstep>();
    session.rejoin(slot, boundary);
    // The replacement's first genuine post-restore watermark is what resumes
    // the production barrier. This fixture has no browser mesh transport, so
    // drive that canonical observation explicitly after rejoin.
    session.observe(slot, boundary.saturating_add(10_000));
}

/// Record a run under `script`, and capture it as an artifact.
fn record() -> ReplayArtifact {
    let args = args();
    let script = script();
    let mut sim =
        drive_run(&args, &script, CHECKPOINT_EVERY).expect("the recording run should drive");
    let log = sim.recorded_log();
    assert_eq!(
        log.len(),
        script.len(),
        "precondition: every scripted command must have crossed the production \
         admission boundary and been accepted. A refused one is silently absent \
         from the log, and every claim below would then be about a shorter run."
    );
    assert!(
        log.entries().iter().all(|e| e.ship.is_named()),
        "precondition: admission must have resolved every entry to a ship by \
         uuid — that key is the whole of what makes an entry re-routable"
    );
    let gm_actions = sim.recorded_gm_actions();
    let final_tick = sim.tick();
    ReplayArtifact::capture(&args, log, gm_actions, final_tick, sim.seal())
        .expect("a seeded run captures")
}

/// AC1 + the contract: `vellum_replay`'s own checks, run against the REAL
/// simulation rather than a toy.
///
/// `replay_is_deterministic` is the floor — the same script on the same seed
/// must land on the same digest. `rejection_is_pure` is the one neither this
/// repository nor the two games vellum came from were testing against the
/// *digest*: a refusal that quietly consumed a random draw would leave the
/// world looking untouched while every later draw shifted, and no log records
/// how many illegal things were tried. `refusals_stay_out_of_the_log` closes
/// the third rule.
///
/// The rejected command is one stamped for a tick the clock has already left
/// behind — the sole rejection [`PhoenixSim`] has, and the one it decides
/// before anything steps, submits or draws.
#[test]
fn the_real_simulation_keeps_the_replay_contract() {
    let args = args();
    let script = script();
    let expected = script.len();
    let rejected = command(
        1,
        SystemId("red-alert".into()),
        SystemControlPayload::SetRedAlert { active: true },
    );

    vellum_replay::contract::check_all(
        || PhoenixSim::new_tailless(&args, expected).expect("the contract fixture should build"),
        &script,
        &rejected,
    );
}

/// Issue #902 AC3, made literally true rather than argued from the digest:
/// a refused command must not move a single [`SimRng`] stream, checked
/// against the raw [`SimRngState`] itself rather than through the folded
/// digest that already includes it (see `headless::digest`'s module docs —
/// `SimRngState` folds via `digest_postcard` as part of the run-scope
/// preamble). The digest equality `rejection_is_pure` already asserts is
/// sufficient to catch a stray draw, but this test pins the SPECIFIC claim
/// the issue names — RNG stream positions, not merely "some hash of
/// everything" — so a reviewer does not have to trust the fold to believe it.
#[test]
fn a_refused_command_leaves_every_rng_stream_position_untouched() {
    use project_phoenix::sim_rng::SimRng;
    use vellum_replay::Simulation;

    let args = args();
    let script = script();
    let mut sim = PhoenixSim::new_tailless(&args, script.len()).expect("the fixture should build");
    vellum_replay::replay_into(&mut sim, &script).expect("the setup script should apply");

    let before = sim
        .app_mut()
        .world()
        .get_resource::<SimRng>()
        .expect("a seeded run carries SimRng")
        .state();

    // The sole rejection PhoenixSim has: a tick the clock has already left
    // behind, decided before anything steps, submits, or draws.
    let rejected = command(
        1,
        SystemId("red-alert".into()),
        SystemControlPayload::SetRedAlert { active: true },
    );
    let outcome = sim.apply(&rejected);
    assert!(
        outcome.is_err(),
        "the command given as `rejected` was accepted, so this check proves \
         nothing"
    );

    let after = sim
        .app_mut()
        .world()
        .get_resource::<SimRng>()
        .expect("SimRng must still be present after a refusal")
        .state();

    assert_eq!(
        before, after,
        "a refused command moved at least one SimStream's position — a draw \
         happened on a path that was not allowed to happen at all"
    );
}

/// AC5 + AC7's happy half: a run writes an artifact, a second run consumes it,
/// and every checkpoint plus the final digest agree.
///
/// The control at the end is what stops this being a tautology. A run of the
/// same seed with NO commands must reach a *different* digest — otherwise the
/// injected commands changed nothing observable, and the equality above would
/// have proved only what `tests/rng_determinism.rs` already proves.
#[test]
fn a_recorded_run_replays_to_the_same_digest() {
    let recorded = record();
    assert!(
        recorded.ledger.checkpoints.len() > 4,
        "precondition: the run must have taken several samples, or there is no \
         window for a divergence to land in. Got {:?}",
        recorded.ledger.checkpoints
    );

    assert_eq!(
        verify_artifact(&recorded).expect("the artifact should replay"),
        None,
        "the same seed replayed with the same command log must pass through \
         every recorded checkpoint and finish on the same digest"
    );

    // And the artifact survives a round trip through the file format, because
    // the run that consumes it is a different process from the one that wrote
    // it.
    let text = recorded.to_ron().expect("serialises");
    let reread = ReplayArtifact::from_ron(&text).expect("parses");
    assert_eq!(reread, recorded);

    let control = {
        let args = args();
        let mut sim = drive_run(&args, &[], CHECKPOINT_EVERY).expect("the control run drives");
        sim.seal()
    };
    assert_ne!(
        control.final_digest,
        recorded.ledger.final_digest,
        "the same seed with NO commands reached the same digest as one with {} \
         commands — the commands changed nothing the digest can see, so the \
         equality above is not a replay",
        recorded.log.len()
    );
}

/// Issue #1292: typed GM input is part of the replay script, not a UI-side
/// mutation. Pause and Resume share one frozen logical boundary (wall frames
/// spent paused are deliberately absent), retain their explicit terminal
/// outcomes, and reproduce the recording's digest from the artifact alone.
#[test]
fn attributed_pause_and_resume_replay_through_the_canonical_journal() {
    let args = args();
    let gm_actions = gm_journal([
        gm_grant(1, 120, "pause-replay", true),
        gm_grant(2, 120, "resume-replay", false),
    ]);
    let mut sim = drive_run_with_gm_actions(&args, &[], &gm_actions, CHECKPOINT_EVERY)
        .expect("the GM recording should drive");

    let outcomes: Vec<_> = sim
        .app_mut()
        .world()
        .resource::<project_phoenix::gm_action::GmActionLog>()
        .entries()
        .iter()
        .map(|entry| entry.outcome)
        .collect();
    assert_eq!(
        outcomes,
        [GmActionOutcome::Applied, GmActionOutcome::Applied]
    );

    let recorded_actions = sim.recorded_gm_actions();
    let final_tick = sim.tick();
    let artifact = ReplayArtifact::capture(
        &args,
        sim.recorded_log(),
        recorded_actions,
        final_tick,
        sim.seal(),
    )
    .expect("the GM run captures");

    assert_eq!(artifact.gm_actions, gm_actions);
    assert_eq!(
        verify_artifact(&artifact).expect("the GM artifact should replay"),
        None,
        "the same seed and canonical GM grants must reproduce the recording"
    );
}

/// Issue #1299's seeded production proof: two equal GM operators take the same
/// authentic Backfill Helm in canonical owner order, one issues the existing
/// Helm thrust command, and their releases remove only their own membership.
/// A second seeded production run must reproduce the terminal facts, command
/// activity, physical effect and digest exactly; the portable artifact then
/// independently verifies the same digest ledger.
#[test]
fn station_takeover_command_equal_gm_order_and_release_replay_identically() {
    fn local_ship_uuid(sim: &mut PhoenixSim) -> String {
        let world = sim.app_mut().world_mut();
        let mut query = world.query::<(
            &project_phoenix::entities::spawner::EntityUuid,
            Option<&project_phoenix::server_app::LocalShip>,
        )>();
        query
            .iter(world)
            .find_map(|(uuid, local)| local.is_some().then(|| uuid.0.clone()))
            .expect("the seeded production world spawns one local player ship")
    }

    fn effects(
        sim: &mut PhoenixSim,
        ship_uuid: &str,
    ) -> (
        Vec<project_phoenix::gm_action::LoggedGmAction>,
        Vec<project_phoenix::gm_puppet::StationPuppetActivityEntry>,
        project_phoenix::ship::state::ShipPhysics,
        u64,
    ) {
        let log = sim
            .app_mut()
            .world()
            .resource::<project_phoenix::gm_action::GmActionLog>()
            .entries()
            .to_vec();
        let activity = sim
            .app_mut()
            .world()
            .resource::<project_phoenix::gm_puppet::StationPuppetActivity>()
            .entries()
            .to_vec();
        let physics = {
            let world = sim.app_mut().world_mut();
            let mut query = world.query::<(
                &project_phoenix::entities::spawner::EntityUuid,
                &project_phoenix::ship::state::ShipPhysics,
            )>();
            *query
                .iter(world)
                .find(|(uuid, _)| uuid.0 == ship_uuid)
                .expect("the commanded ship remains in the production world")
                .1
        };
        let digest = sim.seal().final_digest;
        (log, activity, physics, digest)
    }

    let args = args();
    let mut discovery = drive_run(&args, &[], 0).expect("seeded identity probe drives");
    let ship_uuid = local_ship_uuid(&mut discovery);
    let ship = ShipKey(ship_uuid.clone());
    let helm = StationId("helm".into());
    let target = project_phoenix::gm_puppet::StationPuppetTarget::new(ship.clone(), helm.clone());
    let command_payload =
        project_phoenix::core::codec::canonical_system_command(&SystemControlPayload::SetThrust {
            value: 0.85,
        })
        .expect("the authentic Helm command has canonical wire bytes");

    let mut planned = GmActionJournal::default();
    for grant in [
        station_grant(
            HostSlot(3),
            "gm-b",
            1,
            120,
            "gm-b-take",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: true,
            },
        ),
        station_grant(
            HostSlot(2),
            "gm-a",
            2,
            120,
            "gm-a-take",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: true,
            },
        ),
        station_grant(
            HostSlot(3),
            "gm-b",
            3,
            145,
            "gm-b-thrust",
            GmAction::IssueStationCommand {
                ship: ship.clone(),
                station: helm.clone(),
                target: SystemId("helm-thrust".into()),
                payload: command_payload,
            },
        ),
        station_grant(
            HostSlot(3),
            "gm-b",
            4,
            180,
            "gm-b-release",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: false,
            },
        ),
        station_grant(
            HostSlot(2),
            "gm-a",
            5,
            205,
            "gm-a-release",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: false,
            },
        ),
    ] {
        planned.insert(grant).expect("canonical production action");
    }
    planned
        .restore_applied_frontier(planned.len())
        .expect("the recording plans every supplied action");

    let mut recorded = drive_run_with_gm_actions(&args, &[], &planned, CHECKPOINT_EVERY)
        .expect("the Station GM recording should drive");
    let recorded_actions = recorded.recorded_gm_actions();
    let recorded_effects = effects(&mut recorded, &ship_uuid);
    assert_eq!(
        recorded_effects
            .0
            .iter()
            .map(|entry| (entry.operator_id.as_str(), entry.outcome, entry.order))
            .collect::<Vec<_>>(),
        vec![
            (
                "gm-b",
                GmActionOutcome::Applied,
                Some(GmActionOrder::new(HostSlot(3), 1))
            ),
            (
                "gm-a",
                GmActionOutcome::Applied,
                Some(GmActionOrder::new(HostSlot(2), 2))
            ),
            (
                "gm-b",
                GmActionOutcome::Applied,
                Some(GmActionOrder::new(HostSlot(3), 3))
            ),
            (
                "gm-b",
                GmActionOutcome::Applied,
                Some(GmActionOrder::new(HostSlot(3), 4))
            ),
            (
                "gm-a",
                GmActionOutcome::Applied,
                Some(GmActionOrder::new(HostSlot(2), 5))
            ),
        ],
        "actual outcomes retain the owner's equal-GM order exactly",
    );
    assert_eq!(
        recorded_effects
            .1
            .iter()
            .map(|entry| (entry.operator_id.as_str(), entry.action.as_str()))
            .collect::<Vec<_>>(),
        vec![("gm-b", "SetThrust")],
        "the authentic command crosses production admission exactly once",
    );
    assert!(!recorded
        .app_mut()
        .world()
        .resource::<project_phoenix::gm_puppet::StationPuppets>()
        .is_active(&target));

    let mut replayed = drive_run_with_gm_actions(&args, &[], &recorded_actions, CHECKPOINT_EVERY)
        .expect("the recorded Station actions replay through production");
    let replayed_effects = effects(&mut replayed, &ship_uuid);
    assert_eq!(replayed_effects, recorded_effects);

    let final_tick = recorded.tick();
    let artifact = ReplayArtifact::capture(
        &args,
        recorded.recorded_log(),
        recorded_actions,
        final_tick,
        recorded.seal(),
    )
    .expect("the Station GM run captures");
    assert_eq!(
        verify_artifact(&artifact).expect("the Station GM artifact replays"),
        None,
    );

    let control_digest = {
        let mut control = drive_run(&args, &[], 0).expect("the no-GM control drives");
        control.seal().final_digest
    };
    assert_ne!(
        control_digest, recorded_effects.3,
        "takeover and authentic thrust changed authoritative production state",
    );
}

/// Cycle-2 regression for issue #1299: a sequenced grant can outlive the GM
/// transport that proposed it. The agreed loss boundary, not browser-local
/// connection state, decides whether it may still mutate the Station.
///
/// Both operators take Helm on the loss boundary in owner order, then gm-a
/// releases before FixedUpdate applies its loss, leaving equal gm-b in control.
/// A later canonical slot recovery clears the barrier's departed flag. The old
/// queued takeover and authentic Helm command must nevertheless remain durable
/// refusals, while the replacement incarnation's takeover, command and release
/// apply. The captured artifact repeats those effects and digest from the
/// journal's generation history alone.
#[test]
fn departed_gm_future_station_grants_are_seeded_deterministic_refusals() {
    fn local_ship_uuid(sim: &mut PhoenixSim) -> String {
        let world = sim.app_mut().world_mut();
        let mut query = world.query::<(
            &project_phoenix::entities::spawner::EntityUuid,
            Option<&project_phoenix::server_app::LocalShip>,
        )>();
        query
            .iter(world)
            .find_map(|(uuid, local)| local.is_some().then(|| uuid.0.clone()))
            .expect("the seeded production world spawns one local player ship")
    }

    let args = args();
    let mut discovery = drive_run(&args, &[], 0).expect("seeded identity probe drives");
    let ship_uuid = local_ship_uuid(&mut discovery);
    let ship = ShipKey(ship_uuid);
    let helm = StationId("helm".into());
    let target = project_phoenix::gm_puppet::StationPuppetTarget::new(ship.clone(), helm.clone());
    let gm_a_slot = HostSlot(2);
    let gm_b_slot = HostSlot(3);
    let loss_tick = 120;
    let recovery_tick = 130;
    let command_payload =
        project_phoenix::core::codec::canonical_system_command(&SystemControlPayload::SetThrust {
            value: 0.65,
        })
        .expect("the authentic Helm command has canonical wire bytes");

    let mut post_recovery_take = station_grant(
        gm_a_slot,
        "gm-a",
        6,
        recovery_tick + 20,
        "gm-a-recovered-take",
        GmAction::SetStationPuppet {
            ship: ship.clone(),
            station: helm.clone(),
            active: true,
        },
    );
    post_recovery_take.recovery_generation = 1;
    let mut post_recovery_command = station_grant(
        gm_a_slot,
        "gm-a",
        7,
        recovery_tick + 25,
        "gm-a-recovered-thrust",
        GmAction::IssueStationCommand {
            ship: ship.clone(),
            station: helm.clone(),
            target: SystemId("helm-thrust".into()),
            payload: command_payload.clone(),
        },
    );
    post_recovery_command.recovery_generation = 1;
    let mut post_recovery_release = station_grant(
        gm_a_slot,
        "gm-a",
        8,
        recovery_tick + 30,
        "gm-a-recovered-release",
        GmAction::SetStationPuppet {
            ship: ship.clone(),
            station: helm.clone(),
            active: false,
        },
    );
    post_recovery_release.recovery_generation = 1;

    let mut planned = GmActionJournal::default();
    for grant in [
        station_grant(
            gm_a_slot,
            "gm-a",
            1,
            loss_tick,
            "gm-a-boundary-take",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: true,
            },
        ),
        station_grant(
            gm_b_slot,
            "gm-b",
            2,
            loss_tick,
            "gm-b-boundary-take",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: true,
            },
        ),
        station_grant(
            gm_a_slot,
            "gm-a",
            3,
            loss_tick,
            "gm-a-boundary-release",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: false,
            },
        ),
        station_grant(
            gm_a_slot,
            "gm-a",
            4,
            loss_tick + 20,
            "gm-a-stale-take",
            GmAction::SetStationPuppet {
                ship: ship.clone(),
                station: helm.clone(),
                active: true,
            },
        ),
        station_grant(
            gm_a_slot,
            "gm-a",
            5,
            loss_tick + 25,
            "gm-a-stale-thrust",
            GmAction::IssueStationCommand {
                ship: ship.clone(),
                station: helm.clone(),
                target: SystemId("helm-thrust".into()),
                payload: command_payload,
            },
        ),
        post_recovery_take,
        post_recovery_command,
        post_recovery_release,
    ] {
        planned.insert(grant).expect("canonical production action");
    }

    let ship_host = HostSlot(1);
    let roster = FleetRoster::with_participants_and_gms(
        vec![FleetShip::new(ship_host)],
        vec![ship_host, gm_a_slot, gm_b_slot],
        vec![
            FleetGm {
                host: gm_a_slot,
                operator_id: "gm-a".into(),
            },
            FleetGm {
                host: gm_b_slot,
                operator_id: "gm-b".into(),
            },
        ],
        ship_host,
        ship_host,
    )
    .expect("one ship and two equal GMs form a valid frozen roster");
    let mut session = LockstepSession::new(ship_host, [gm_a_slot, gm_b_slot], 0);
    session.observe(gm_a_slot, loss_tick - 1);
    session.observe(gm_b_slot, args.max_ticks + 10);
    session.depart(gm_a_slot);
    let mut pending_loss = PendingHostLoss::default();
    assert!(pending_loss.observe(gm_a_slot, loss_tick));

    let run = || {
        let mut sim = PhoenixSim::new(&args, 0, CHECKPOINT_EVERY)
            .expect("the seeded loss production run builds");
        sim.app_mut()
            .insert_resource(roster.clone())
            .insert_resource(FleetLockstep(session.clone()))
            .insert_resource(pending_loss.clone())
            .insert_resource(SeededSlotRecovery {
                slot: gm_a_slot,
                boundary: recovery_tick,
                applied: false,
            })
            .insert_resource(SeededLossGmActions(planned.clone()))
            .add_systems(
                bevy::prelude::OnEnter(GamePhase::InProgress),
                seed_loss_gm_actions.after(project_phoenix::gm_action::reset),
            )
            .add_systems(
                bevy::prelude::PreUpdate,
                apply_seeded_slot_recovery.after(project_phoenix::gm_action::apply_due_actions),
            );
        vellum_replay::replay_into(&mut sim, &[]).expect("the seeded loss production run drives");

        let journal = sim.recorded_gm_actions();
        let operators = sim
            .app_mut()
            .world()
            .resource::<project_phoenix::gm_puppet::StationPuppets>()
            .operators(&target)
            .to_vec();
        let activity = sim
            .app_mut()
            .world()
            .resource::<project_phoenix::gm_puppet::StationPuppetActivity>()
            .entries()
            .to_vec();
        let losses = sim
            .app_mut()
            .world()
            .resource::<PendingHostLoss>()
            .records()
            .to_vec();
        let final_tick = sim.tick();
        let command_log = sim.recorded_log();
        let ledger = sim.seal();
        let digest = ledger.final_digest;
        let artifact =
            ReplayArtifact::capture(&args, command_log, journal.clone(), final_tick, ledger)
                .expect("the loss/rejoin generation run captures");
        (journal, operators, activity, losses, digest, artifact)
    };

    let first = run();
    assert_eq!(
        first
            .0
            .applied_results()
            .iter()
            .map(|entry| (entry.outcome, entry.reason))
            .collect::<Vec<_>>(),
        vec![
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::NotGameMaster),
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::NotGameMaster),
            ),
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None),
        ],
        "same-boundary actions retain their order; old-incarnation grants stay \
         refused after rejoin and genuinely recovered grants apply",
    );
    assert_eq!(first.1, ["gm-b"]);
    assert_eq!(
        first
            .2
            .iter()
            .map(|entry| (entry.operator_id.as_str(), entry.action.as_str()))
            .collect::<Vec<_>>(),
        [("gm-a", "SetThrust")],
        "only the recovered incarnation's authentic command reaches activity",
    );
    assert_eq!(
        first.3,
        [project_phoenix::lockstep::HostLossRecord {
            slot: gm_a_slot,
            tick: loss_tick,
        }],
    );
    assert_eq!(
        first.0.recovery_generations(),
        [project_phoenix::gm_action::GmSlotRecoveryGeneration {
            slot: gm_a_slot,
            boundary_tick: recovery_tick,
            generation: 1,
        }],
    );
    assert_eq!(
        verify_artifact(&first.5).expect("the loss/rejoin artifact replays"),
        None,
        "the recovery generation, effects and digest replay from the artifact",
    );

    let second = run();
    assert_eq!(second.0, first.0);
    assert_eq!(second.1, first.1);
    assert_eq!(second.2, first.2);
    assert_eq!(second.3, first.3);
    assert_eq!(
        second.4, first.4,
        "the seeded loss run must be deterministic"
    );
}

#[test]
fn an_adopted_restore_pause_resumes_through_the_real_build_and_artifact_path() {
    let mut args = args();
    args.max_ticks = 40;
    let mut gm_actions = GmActionJournal::default();
    gm_actions.adopt_initial_pause(true);
    gm_actions
        .insert(gm_grant(1, 0, "restored-resume", false))
        .unwrap();
    gm_actions.restore_applied_frontier(1).unwrap();

    let mut sim = drive_run_with_gm_actions(&args, &[], &gm_actions, CHECKPOINT_EVERY)
        .expect("the restored session should resume through PhoenixSim::build");
    assert!(
        sim.tick() > 0,
        "the typed Resume must reopen the fixed clock"
    );
    assert_eq!(
        sim.app_mut()
            .world()
            .resource::<project_phoenix::gm_action::GmActionLog>()
            .entries()[0]
            .outcome,
        GmActionOutcome::Applied
    );
    let final_tick = sim.tick();
    let artifact = ReplayArtifact::capture(
        &args,
        sim.recorded_log(),
        sim.recorded_gm_actions(),
        final_tick,
        sim.seal(),
    )
    .expect("the restored run captures");
    assert_eq!(verify_artifact(&artifact).unwrap(), None);
}

#[test]
fn replay_cannot_skip_an_interior_pause_on_a_multi_step_frame() {
    let mut args = args();
    args.max_ticks = 20;
    args.dt = 5.0 / 60.0;
    let gm_actions = gm_journal([gm_grant(1, 3, "interior-pause", true)]);
    let mut sim = drive_run_with_gm_actions(&args, &[], &gm_actions, 0)
        .expect("the multi-step recording should stop on its typed boundary");
    assert_eq!(
        sim.tick(),
        3,
        "the fixed catch-up loop must not cross an interior Pause boundary"
    );
    assert_eq!(sim.recorded_gm_actions().applied_grants(), 1);
    let final_tick = sim.tick();
    let artifact = ReplayArtifact::capture(
        &args,
        sim.recorded_log(),
        sim.recorded_gm_actions(),
        final_tick,
        sim.seal(),
    )
    .expect("the stopped run captures");
    assert_eq!(verify_artifact(&artifact).unwrap(), None);
}

/// A recording that ends paused spends its remaining wall-frame allowance at
/// one logical tick. Artifact replay therefore terminates at that recorded
/// logical boundary after consuming the pause, instead of waiting for a tick
/// that can never occur or treating the paused frames as simulation steps.
#[test]
fn a_replay_that_ends_paused_terminates_at_the_recorded_logical_tick() {
    let args = args();
    let gm_actions = gm_journal([gm_grant(1, 120, "terminal-pause", true)]);
    let mut sim = drive_run_with_gm_actions(&args, &[], &gm_actions, CHECKPOINT_EVERY)
        .expect("the paused recording should drive");
    let final_tick = sim.tick();
    assert_eq!(
        final_tick, 120,
        "the boundary-120 pause applies before tick 120 can be left behind"
    );

    let artifact = ReplayArtifact::capture(
        &args,
        sim.recorded_log(),
        sim.recorded_gm_actions(),
        final_tick,
        sim.seal(),
    )
    .expect("the paused run captures");

    assert_eq!(
        verify_artifact(&artifact).expect("the paused artifact should terminate"),
        None
    );
}

/// AC7: a deliberately corrupted replay reports the FIRST divergent tick, not
/// merely that the run diverged.
///
/// The corruption is a payload change on a command in the middle of the script:
/// the log still applies cleanly (nothing about it is out of order), so the
/// driver has no rejection to raise and the only thing that can catch this is
/// the digest. That is the case the checkpoints exist for.
#[test]
fn a_corrupted_payload_is_located_to_a_tick_window() {
    let recorded = record();
    let last = recorded.log.entries().len() - 1;
    let corrupt_tick = recorded.log.entries()[last].tick;

    let mut corrupted = recorded.clone();
    let mut entries: Vec<LoggedCommand> = corrupted.log.entries().to_vec();
    // The last command asks for a half-astern throttle; the corrupted log asks
    // for near-full ahead — same tick, same system, same credential, one field
    // different.
    //
    // The LAST command, and that is worth saying out loud rather than leaving
    // as an arbitrary index. Corrupting an *earlier* one in this scenario does
    // not diverge, and the reason is a property of the scenario rather than of
    // the digest: `patrol.toml` puts the player ship on AI Backfill flying a
    // waypoint route, and a waypoint follower is a closed loop. Nudge its
    // throttle or its alert state forty ticks from the end of the run and it
    // steers back onto the route, and by the next checkpoint the two runs
    // genuinely agree again — the divergence is not missed, it is repaired. A
    // corruption inside the last sampling window is the one this run cannot
    // recover from before it is measured.
    entries[last].payload = SystemControlPayload::SetThrust { value: 0.9 };
    corrupted.log = rebuild_log(&entries);

    let divergence = verify_artifact(&corrupted)
        .expect("a corrupted payload still applies — it is the digest that catches it")
        .expect("the corrupted run must not reproduce the recording");

    assert!(
        divergence.tick >= corrupt_tick,
        "the divergence was reported at tick {}, BEFORE the corrupted command's \
         own tick {corrupt_tick} — a digest that disagrees before the input \
         that caused it is measuring something other than this run",
        divergence.tick
    );
    assert!(
        divergence.tick - corrupt_tick <= CHECKPOINT_EVERY,
        "the divergence was reported at tick {} for a command on tick \
         {corrupt_tick}, which is more than one {CHECKPOINT_EVERY}-tick sampling \
         window away. The point of periodic sampling is that the answer is a \
         window, not the whole run.",
        divergence.tick
    );
    assert_eq!(
        divergence.after,
        recorded
            .ledger
            .checkpoints
            .iter()
            .map(|c| c.tick)
            .rfind(|t| *t < divergence.tick),
        "the window's lower edge must be the last checkpoint the two runs \
         agreed on"
    );
}

/// AC7's other half: a log whose ticks go backwards is refused by name, at the
/// command's own index — the driver's answer where a digest is not needed.
#[test]
fn a_reordered_log_names_the_command_that_broke_it() {
    let recorded = record();
    let mut corrupted = recorded.clone();
    let mut entries: Vec<LoggedCommand> = corrupted.log.entries().to_vec();
    // The last command is stamped for a tick long past; the replay clock has
    // already left it behind by the time it arrives.
    let last = entries.len() - 1;
    entries[last].tick = 1;
    corrupted.log = rebuild_log(&entries);

    match verify_artifact(&corrupted) {
        Err(ReplayError::Refused { at_command, why }) => {
            assert_eq!(
                at_command, last,
                "the refusal must name the command, not the run"
            );
            assert!(
                why.contains("out of order"),
                "the refusal should say what was wrong with it; got {why:?}"
            );
        }
        other => panic!("a backwards tick must be refused, got {other:?}"),
    }
}

/// AC6: `0` disables periodic hashing, and disabling it costs nothing — no
/// checkpoint is recorded, and the run still reports where it finished.
#[test]
fn a_zero_interval_samples_nothing_but_still_reports_the_ending() {
    let args = args();
    let mut sampled = drive_run(&args, &[], CHECKPOINT_EVERY).expect("drives");
    let sampled = sampled.seal();
    let mut unsampled = drive_run(&args, &[], 0).expect("drives");
    let unsampled = unsampled.seal();

    assert!(
        unsampled.checkpoints.is_empty(),
        "a zero interval must sample nothing at all"
    );
    assert!(!sampled.checkpoints.is_empty(), "the control must sample");
    assert_eq!(
        sampled.final_digest, unsampled.final_digest,
        "sampling must not perturb the run it measures — the digest reads state \
         rather than drawing from it, and a sampled run must land exactly where \
         an unsampled one does"
    );
    assert_eq!(
        unsampled.first_divergence(&sampled),
        None,
        "two ledgers that share no checkpoint ticks and agree on the ending \
         have not diverged"
    );
}

/// The duel harness's own inputs — a non-empty roster on both sides, so the
/// artifact actually exercises `side_a`/`side_b` rather than replaying an
/// ordinary `--world`/`--ship` run that happens to route through the same
/// code (issue #901 review, finding 1: a v1 artifact had nowhere to put these
/// at all).
fn duel_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/duel.toml".into(),
        // `side_a[0]` is the player ship; `side_a[1..]` and `side_b[..]` are
        // NPC escorts/enemies `duel.toml`'s slots fill.
        side_a: vec!["cruiser".into(), "courier".into()],
        side_b: vec!["destroyer".into()],
        max_ticks: 200,
        seed: Some(901_2027),
        deterministic: true,
        ..Default::default()
    }
}

/// AC1/AC5 over a DUEL config: a run whose `side_a`/`side_b` rosters are
/// non-empty records an artifact that carries them, and a replay of that
/// artifact reproduces the recording — including the duel transform itself
/// (`apply_duel_sides`), which only runs at all when `HeadlessArgs::side_a`/
/// `side_b` reach `build_headless_app`. A v1 artifact (before this review)
/// had nowhere to carry these two fields, so replaying a duel recording
/// silently ran `duel.toml` UNTRANSFORMED — since issue #984 that means the
/// world's own authored default roster (a 5v5 of couriers against destroyers),
/// and before it an empty arena with every slot deleted. Either way it is not
/// the roster that was actually recorded.
#[test]
fn a_duel_recording_replays_with_its_side_rosters_intact() {
    let args = duel_args();
    let mut sim = drive_run(&args, &[], CHECKPOINT_EVERY).expect("the duel run should drive");
    let log = sim.recorded_log();
    let gm_actions = sim.recorded_gm_actions();
    let final_tick = sim.tick();
    let recorded = ReplayArtifact::capture(&args, log, gm_actions, final_tick, sim.seal())
        .expect("a seeded run captures");

    assert_eq!(recorded.side_a, args.side_a, "the roster must round-trip");
    assert_eq!(recorded.side_b, args.side_b);

    // The artifact's own replay_args must still name the duel rosters — this
    // is what `build_headless_app` reads to re-run `apply_duel_sides`.
    let replay_args = recorded.replay_args();
    assert_eq!(replay_args.side_a, args.side_a);
    assert_eq!(replay_args.side_b, args.side_b);

    assert_eq!(
        verify_artifact(&recorded).expect("the duel artifact should replay"),
        None,
        "a duel recording with non-empty side rosters must reproduce exactly, \
         including the escort/enemy slots the rosters fill"
    );

    // And the artifact — rosters included — survives the file round trip a
    // real `--record`/`--replay` pair takes.
    let text = recorded.to_ron().expect("serialises");
    let reread = ReplayArtifact::from_ron(&text).expect("parses");
    assert_eq!(reread, recorded);
    assert_eq!(reread.side_a, args.side_a);
    assert_eq!(reread.side_b, args.side_b);
}

/// Build a `CommandLog` from a mutated entry list.
///
/// `CommandLog` deliberately has no public constructor from entries: recording
/// goes through `stamp_accepted_command`, which cannot record without also
/// queueing, and production code must keep it that way. A test that wants a
/// *corrupted* log therefore builds one the only way anything outside the
/// process can — through the serialised form, which is exactly the route a
/// tampered-with artifact would take to reach a replay in the first place.
fn rebuild_log(entries: &[LoggedCommand]) -> project_phoenix::command_admission::CommandLog {
    let text = ron::ser::to_string(&Wrapper {
        entries: entries.to_vec(),
    })
    .expect("entries serialise");
    ron::from_str(&text).expect("a CommandLog is exactly its entries")
}

/// The `CommandLog` wire shape, named locally so this file can build one
/// without the crate exposing a constructor production code must not have.
#[derive(serde::Serialize)]
struct Wrapper {
    entries: Vec<LoggedCommand>,
}
