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

use project_phoenix::command_admission::log::{LoggedCommand, ShipKey};
use project_phoenix::headless::replay::{drive_run, PhoenixSim, ReplayError};
use project_phoenix::headless::{verify_artifact, HeadlessArgs, ReplayArtifact};
use project_phoenix::messages::{SystemControlPayload, SystemId};

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
    ReplayArtifact::capture(&args, log, sim.seal()).expect("a seeded run captures")
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
/// silently ran the UNFILLED `duel.toml` — every NPC slot deleted, no
/// escorts, no enemies — rather than the roster that was actually recorded.
#[test]
fn a_duel_recording_replays_with_its_side_rosters_intact() {
    let args = duel_args();
    let mut sim = drive_run(&args, &[], CHECKPOINT_EVERY).expect("the duel run should drive");
    let log = sim.recorded_log();
    let recorded = ReplayArtifact::capture(&args, log, sim.seal()).expect("a seeded run captures");

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
