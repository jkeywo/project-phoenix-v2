//! Issue #1086's acceptance: the authoritative-state digest folds the
//! **scenario's own state**, so two hosts that diverge only in a flag, a
//! scheduled callback, a trigger latch, a comms thread or a gathered finding
//! disagree loudly — and say which tick they stopped agreeing on.
//!
//! # What this proves that the unit tests cannot
//!
//! `src/sim_digest_tests.rs` builds each folded surface by hand in a bare
//! `World` and asserts the fold moves. That is the right shape for "this field
//! is in the fold", and the wrong shape for the two claims this issue is
//! actually about:
//!
//! 1. **Two identically-driven instances agree, tick by tick.** Not just at the
//!    end: each run samples its digest into a [`DigestLedger`] on a fixed
//!    interval, and the two ledgers are compared with `first_divergence`, which
//!    pairs by tick. A widened fold that quietly depended on a `HashMap`'s
//!    iteration order, or on when a resource happened to be inserted, passes an
//!    end-state comparison roughly half the time and fails this one.
//! 2. **An injected scenario-state divergence is DETECTED, and located.** One
//!    instance is perturbed mid-run in a way that touches nothing but scenario
//!    state; the ledger comparison must name a sampled tick after the injection
//!    and a last-agreed tick before it. Before this issue every one of these
//!    injections was invisible to `world_digest` — which is the hole
//!    `p2p-design-deltas.yaml`'s `p2p-delta-scenario-state-must-ride-the-join`
//!    names, and the reason #1118's cross-peer hash exchange could not see a
//!    scenario fork at all.
//!
//! # Why this world
//!
//! `assets/worlds/probe_evidence.toml` is the smallest shipped world that
//! reaches every folded surface at once inside a short run: it authors scripted
//! triggers and world flags, arms a named deadline (so `PendingCallbacks` is
//! genuinely non-empty rather than trivially so), writes a dossier finding from
//! a survey, and opens a scripted comms thread its own Backfill officer answers.
//! A world that exercised none of them would satisfy every equality below
//! vacuously, which is what [`the_scenario_surfaces_are_actually_populated`]
//! exists to rule out.
//!
//! # Why this is its own test binary
//!
//! The reason `tests/entity_id_minting.rs`, `tests/rng_determinism.rs` and
//! `tests/snapshot_resume.rs` are: `--deterministic` pins the scheduler by
//! handing `TaskPoolPlugin` a one-thread `TaskPoolOptions`, and Bevy's task
//! pools are process-global, fixed by whichever app builds first. A claim about
//! two instances agreeing, made under a pool somebody else chose, is not a claim
//! about anything.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::App;
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::sim_digest::{world_digest, DigestLedger};
use project_phoenix::sim_tick::SimTick;
use project_phoenix::world::server::{WorldContentRuntime, WorldScriptRuntime};

/// See the module docs for why this world and not `combat_test`.
const WORLD: &str = "assets/worlds/probe_evidence.toml";

/// The world's own player hull — the Comms response policy that answers the
/// foreman is authored as an override on this entry, so a different hull leaves
/// the dialogue unanswered and the comms surface half-dead.
const SHIP: &str = "assets/entities/alliance_destroyer.toml";

/// One fixed seed for every run in this file: two instances only agree if they
/// walk the identical RNG stream, and that is a precondition of the claim rather
/// than part of it.
const SEED: u64 = 0x1086_D19E_5700_0001;

/// Frames each instance runs. Past the survey finding (t = 3 s), past the
/// foreman thread opening (t = 5 s) and past the officer answering it, so the
/// comms and dossier surfaces are populated well before the run ends.
const FRAMES: u64 = 480;

/// Ticks between digest samples. Small enough that the injection below is
/// bracketed by two checkpoints a few ticks apart, so a located divergence is a
/// narrow window rather than a shrug.
const INTERVAL: u64 = 30;

/// The frame an injection is applied on — after the scenario has real state to
/// perturb, and far enough from the end that several checkpoints follow it.
const INJECT_AT: u64 = 300;

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: SHIP.into(),
        max_ticks: FRAMES,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn boot() -> App {
    let mut app = build_headless_app(&args()).expect("the probe world builds");
    // `headless::run_sampled` does this before its loop; a driver stepping the
    // app by hand has to do it too, or `Startup` never runs.
    app.finish();
    app.cleanup();
    app
}

/// A perturbation of **scenario state only**, applied to one instance mid-run.
///
/// Every variant writes a resource the scenario owns and nothing else: no
/// entity moves, no hull changes, no generator is drawn from. Before issue
/// #1086 all five were invisible to `world_digest`.
#[derive(Clone, Copy, Debug)]
enum Injection {
    /// A world flag nothing in the scenario reads — the narrowest possible
    /// scenario fork, and the one a `when = "…"` predicate would act on.
    Flag,
    /// A scripted callback queued for a tick the run never reaches, so the
    /// queue's CONTENTS differ while nothing else does.
    Callback,
    /// A trigger's single-shot latch, flipped as a layer's own state would be.
    TriggerLatch,
    /// A message in one crew's inbox and not the other's — the heart of
    /// `CommsState`, and the thing a Comms officer would answer.
    ///
    /// The inbox rather than `CommsRuntime::open_hails`, which was tried first
    /// and is the wrong lever for this test: `update_comms_range_flags` prunes a
    /// hail whose target is not a live in-range contact, so a synthetic entry is
    /// gone inside the same tick and reaches no sample. That is the simulation
    /// working, not the fold missing it.
    InboxMessage,
    /// A finding one crew has and the other does not.
    Evidence,
}

impl Injection {
    fn apply(self, app: &mut App) {
        match self {
            Self::Flag => {
                app.world_mut()
                    .resource_mut::<WorldContentRuntime>()
                    .flags
                    .set_flag_value("injected_divergence", 7);
            }
            Self::Callback => {
                let mut script = app
                    .world_mut()
                    .get_resource_mut::<WorldScriptRuntime>()
                    .expect("the probe world authors scripts");
                script
                    .pending_callbacks
                    .push(project_phoenix::world::script::schedule::ScheduledCall {
                        // Never due inside this run, so the entry perturbs the
                        // QUEUE without perturbing what the scenario does.
                        fire_tick: u64::MAX,
                        script_path: "injected#script.divergence".into(),
                        fn_name: "never_called".into(),
                        origin_layer: None,
                    });
            }
            Self::TriggerLatch => {
                let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
                let last = runtime
                    .trigger_states
                    .last_mut()
                    .expect("the probe world authors triggers");
                last.fired = !last.fired;
            }
            Self::InboxMessage => {
                let message = project_phoenix::core::messages::CommsMessage::injected(
                    "injected-message".into(),
                    "injected-sender".into(),
                    "Injected Sender".into(),
                    "world.injected.comms.body".into(),
                    std::collections::BTreeMap::new(),
                    Vec::new(),
                    "injected-thread".into(),
                    true,
                    false,
                );
                app.world_mut()
                    .resource_mut::<project_phoenix::comms::server::CommsInboxRes>()
                    .0
                    .inject(message);
            }
            Self::Evidence => {
                app.world_mut()
                    .resource_mut::<WorldContentRuntime>()
                    .evidence
                    .append(
                        "injected-subject",
                        "world.injected.evidence",
                        project_phoenix::dossier::EvidenceProvenance::Records,
                        1,
                    );
            }
        }
    }
}

/// Apply `injection`, if there is one, on the frame it belongs on.
///
/// A free function rather than an `if` inside the loop so the clean run and the
/// forked run walk byte-identical driver code up to the injection frame.
fn apply_injection(injection: Option<Injection>, frame: u64, app: &mut App) {
    if frame != INJECT_AT {
        return;
    }
    let Some(injection) = injection else {
        return;
    };
    injection.apply(app);
}

/// Run one instance, sampling its digest into a ledger every [`INTERVAL`] ticks.
///
/// The sample is taken between `App::update()` calls — the `RenderInterp`
/// bracket `sim_digest`'s module docs require — so no fold ever observes a
/// half-committed tick.
fn run_ledger(injection: Option<Injection>) -> DigestLedger {
    let mut app = boot();
    let mut ledger = DigestLedger::new(INTERVAL);
    for frame in 0..FRAMES {
        apply_injection(injection, frame, &mut app);
        app.update();
        let tick = app.world().resource::<SimTick>().0;
        if ledger.samples(tick) {
            ledger.record(tick, world_digest(app.world()));
        }
    }
    ledger.final_digest = world_digest(app.world());
    ledger
}

/// The non-vacuity guard for everything below: the world this file drives
/// actually reaches every scenario surface the fold widened over.
///
/// Without it, "two instances agree" and "an injection is caught" could both
/// pass over a scenario that never set a flag, never queued a callback and never
/// opened a thread — which is exactly the state the fold was blind to before,
/// dressed up as a passing test.
#[test]
fn the_scenario_surfaces_are_actually_populated() {
    let mut app = boot();
    for _ in 0..FRAMES {
        app.update();
    }

    let runtime = app.world().resource::<WorldContentRuntime>();
    assert!(
        runtime.flags.iter().next().is_some(),
        "precondition: the run must have set at least one world flag"
    );
    assert!(
        !runtime.trigger_states.is_empty(),
        "precondition: the run must carry a live trigger table"
    );
    assert!(
        runtime.trigger_states.iter().any(|state| state.fired),
        "precondition: at least one trigger must have latched, or the latch fold \
         is being asserted over a table of untouched rows"
    );
    assert!(
        !runtime.evidence.is_empty(),
        "precondition: the crew must have found something out"
    );
    assert!(
        !runtime.deadlines.records.is_empty() && runtime.deadlines.armed,
        "precondition: the world must arm a named deadline, which is what puts a \
         real entry on the pending-callback queue"
    );

    let comms = app
        .world()
        .resource::<project_phoenix::comms::server::CommsInboxRes>();
    assert!(
        !comms.0.is_empty(),
        "precondition: the scripted comms thread must have opened, or the comms \
         fold is being asserted over an empty inbox"
    );
}

/// AC4, and the harness's own floor: two instances driven identically fold to
/// byte-identical digests at every shared checkpoint AND at the end.
///
/// Compared through `DigestLedger::first_divergence` rather than as two final
/// numbers, because a fold that leaked a `HashMap`'s iteration order can still
/// land on the same final digest by luck, and cannot land on the same digest at
/// sixteen consecutive checkpoints twice running.
#[test]
fn two_instances_fold_the_same_scenario_state() {
    let a = run_ledger(None);
    let b = run_ledger(None);

    assert!(
        a.checkpoints.len() > 5,
        "precondition: the run must sample several checkpoints; got {}",
        a.checkpoints.len()
    );
    assert_eq!(
        a.first_divergence(&b),
        None,
        "two seeded instances of the same world must agree at every sampled tick"
    );
    assert_eq!(
        a.final_digest, b.final_digest,
        "and on the state they finish in"
    );
}

/// AC1 + AC3: each injected scenario-state fork is detected, and the ledger
/// names the window it happened in.
///
/// One test body over every variant rather than five near-identical ones — the
/// claim is identical in each case and only the perturbation differs, so a
/// table makes the *set* that is covered legible and adding one is a line.
#[test]
fn an_injected_scenario_divergence_is_caught_and_located() {
    let clean = run_ledger(None);

    for injection in [
        Injection::Flag,
        Injection::Callback,
        Injection::TriggerLatch,
        Injection::InboxMessage,
        Injection::Evidence,
    ] {
        let forked = run_ledger(Some(injection));
        let divergence = clean
            .first_divergence(&forked)
            .unwrap_or_else(|| panic!("{injection:?} went undetected by the digest"));

        assert!(
            !divergence.at_end,
            "{injection:?} must disagree at a SAMPLED tick — an end-only mismatch \
             means the fold noticed the consequences and not the state; got \
             {divergence}"
        );
        // The injection is applied BEFORE the frame's `update()`, so the tick
        // that frame produces is the first one that can disagree — and it must
        // be caught by the next sample after that, not eventually.
        assert!(
            (INJECT_AT..=INJECT_AT + INTERVAL).contains(&divergence.tick),
            "{injection:?} must be caught within one sampling interval of the \
             tick it was applied on; got {divergence}"
        );
        assert!(
            divergence.after.is_some_and(|agreed| agreed < INJECT_AT),
            "{injection:?} must have a last-agreed checkpoint BEFORE the \
             injection, or the window it names is not a window; got {divergence}"
        );
    }
}
