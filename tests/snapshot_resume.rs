//! Issue #862's acceptance: a bounded session can be saved, stored through
//! `vellum-save`, and resumed into a **freshly constructed app** that stands at
//! the same authoritative state.
//!
//! # Why this is its own test binary
//!
//! The same reason `tests/replay_simulation.rs` is, and it is not a style
//! preference: `--deterministic` pins the scheduler by handing `TaskPoolPlugin`
//! a one-thread `TaskPoolOptions`, but Bevy's task pools are process-global and
//! fixed by whichever app in the process builds first. A digest-equality claim
//! made in a process shared with forty other tests is a claim about whoever won
//! that race.
//!
//! # What is asserted here, and what is deliberately not
//!
//! Asserted: that a capture round-trips through a `Store`, that a fresh app
//! restored from it reproduces the capture's `world_digest` exactly, that the
//! restored world then *steps forward together with* the live one for a
//! measured number of frames, that `vellum_save::verify` agrees, and that a
//! save whose rules moved is refused with vellum's own `Moved` rather than a
//! phoenix status.
//!
//! Not asserted: schema and version *behaviour* — the ordering of the three
//! checks, the byte-compatibility of a snapshotless run, what a tampered
//! payload does. That is `vellum-save`'s own suite, upstream, and a second copy
//! of it here would prove nothing this repository owns.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::{NextState, State};
use project_phoenix::content_ledger;
use project_phoenix::core::messages::{GamePhase, ServerMessage};
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::server_app::{GameOverReason, SimOutbox};
use project_phoenix::sim_digest::world_digest;
use project_phoenix::snapshot::{
    capture, load_from, ready_to_rebuild, ready_to_restore, reconcile_world_layers, restore,
    run_for, save_to, versions, LayerReconcileStatus, LoadRefusal, PhoenixSnapshot, SavedGame,
    SIMULATION_RULES, SNAPSHOT_FORMAT,
};
use project_phoenix::world::script::load::script_ledger_key;
use vellum_save::{FileStore, Moved, Verdict, Versions};

/// The duel arena: a fixed roster spawned at t=0 and no asteroid field at all.
///
/// The narrow world, and it is here first because when it fails the failure is
/// readable — two ships, no streaming, nothing between the capture and the
/// restore but the ships themselves.
const DUEL: &str = "assets/worlds/duel.toml";

/// Combat Test: the acceptance criteria's own world, and the hard one.
///
/// Its two asteroid belts are **streamed** — a rock exists when the player's
/// cell window covers it — so a fresh app bootstrapped at the spawn point has a
/// different rock population than a capture taken after the player has flown
/// somewhere. That is precisely the case a resume has to survive, and
/// [`CAPTURE_AT`] is chosen to be well past the point where the player has
/// moved: a capture taken before the belt had streamed would be asserting the
/// absence of a problem it had arranged not to meet.
const COMBAT_TEST: &str = "assets/worlds/combat_test.toml";

const SEED: u64 = 862_2026;

/// Frames to run before the capture. Well past the auto-start countdown and far
/// enough in that the ships have closed, acquired and started trading fire — a
/// capture of a world at rest would not exercise motion, damage or weapons, and
/// [`assert_capture_is_alive`] is what enforces that rather than trusting it.
const CAPTURE_AT: u64 = 400;

/// Frames the **duel** runs after the restore before the two worlds are
/// allowed to be compared for the last time, and this number is a **measured
/// finding**, not a taste.
///
/// It was 1. The restored world held for a single frame and parted company on
/// the second, and the cause was written down at the time as cold weapon state
/// machines plus the RNG draws that damage application makes. That was wrong,
/// and finding out why took chasing the divergence through four layers. In
/// order:
///
/// 1. `AiPolicyTickClock` — the tick-derived clock every stateful policy
///    measures `state_time` against — was not in the payload, so each ship's
///    restored `entered_at_secs` referred to a clock that had been rewound to
///    the resumed app's own age.
/// 2. `HelmRecoveryHistory` — the bounded range windows behind
///    `fact(safe_distance_held)` and the pressed detector — was not either, and
///    they are an accumulation nothing recomputes.
/// 3. `ShipSystemBlackboards`, the *frozen* cross-system read surface a ship's
///    own helm decides from, was the bootstrap's rather than the capture's.
/// 4. And the one that actually broke it: `build_world_snapshot` runs under
///    `run_if(ai_snapshot_ready)`, a latch that is a pure function of
///    `SimTick` — which the restore had just moved. So the resumed ships spent
///    every tick up to the next cadence arm steering from an *empty* radar
///    view, resolved no target at all, cleared their recovery windows on the
///    resulting target switch, and dropped out of `torpedo_run` and `inbound`
///    into `acquire`. `snapshot::restore` now rebuilds that derivation itself.
///
/// With all four closed — and with the weapon and repair machines the payload
/// gained alongside them — the two worlds are bit-identical for this many
/// frames, which at the duel's `sim_tick_hz` is comfortably past the 60 ticks
/// the review asked for. A frame is not a tick (issue #895), and both worlds
/// are stepped the same way, so the claim is exactly "the same schedule run the
/// same number of times produces the same digest".
const CONTINUE_FOR: u64 = 120;

/// Frames **Combat Test** is held to, and it is 0 rather than 120 — a measured
/// bound with a named cause, not a bound that was tuned until it passed.
///
/// Combat Test's roster is ten ships, and eight of them are wave NPCs standing
/// off at r >= 1000 — outside the player's sensor range, so they carry no
/// `AiHighFidelity` marker and are moved by `ai::server::simulate_low_lod_ships`
/// rather than by the helm path this payload restores. That system steers from
/// the Patrol/Reach directive it reads out of the *republished* Viewscreen
/// blackboard's `scored_objectives`, and the objective evaluator rewrites that
/// list from its own inputs on its own cadence. One frame after the restore, a
/// low-LOD Harrow that the live world was still holding at its patrol throttle
/// (`max_speed * 0.4`, clamped) was ramping toward `max_speed` in the resumed
/// one, because it had no route to clamp against.
///
/// That is the doctrine objective evaluator's per-ship state, not a field this
/// payload forgot: the two halves of it that ARE per-ship components —
/// `ObjectiveCursors` (which leg of the route a patroller is on) and the
/// blackboards themselves — are both restored, and restoring them is what
/// closed the *other* low-LOD divergence this measurement found, a wave ship
/// that came back steering for the far end of a lap it was halfway around.
/// What remains is the evaluator's own re-derivation, and widening the payload
/// to cover it is a different issue's work.
///
/// So what Combat Test asserts here is everything up to and including the
/// instant of restore — AC1's streamed belts rebuilt rock for rock, the digest
/// equal, `vellum_save::verify` satisfied — and the continuation claim is left
/// to the duel, where every ship is high-fidelity and every mover this payload
/// restores is the one actually driving.
const COMBAT_TEST_CONTINUE_FOR: u64 = 0;

fn args(world: &str, ships: (&str, &str)) -> HeadlessArgs {
    HeadlessArgs {
        world_path: world.into(),
        side_a: vec![ships.0.into()],
        side_b: vec![ships.1.into()],
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// Combat Test brings its own waves, so it names a player hull directly rather
/// than through the duel harness — it authors no `side_a_*`/`side_b_*` slots
/// for `--side-a` to fill, and asking for them is a build error.
fn combat_test_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: COMBAT_TEST.into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// The content version this process's most recent `build_headless_app` call
/// froze (issue #935). `build_headless_app` resets the ledger and re-records
/// the whole declared file set on every call, so this always reads the load
/// that is currently active on this test's thread — see
/// `content_ledger`'s module docs for why a *frozen* read, not a live fold,
/// is the one that stays stable while the world goes on to stream.
///
/// `world` is unused now that the digest is the ledger's, not the scenario
/// text's, but is kept so call sites still read as "the versions for THIS
/// world" and a caller comparing two different worlds' calls stays honest
/// about which one it means.
fn current_versions(_world: &str) -> Versions {
    versions(&content_ledger::frozen_or_live())
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("phoenix-snapshot-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn boot(args: &HeadlessArgs) -> bevy::prelude::App {
    let mut app = build_headless_app(args).expect("the world builds");
    // `headless::run_sampled` does this before its loop; a driver stepping the
    // app by hand has to do it too, or `Startup` never runs.
    app.finish();
    app.cleanup();
    app
}

fn duel() -> bevy::prelude::App {
    boot(&args(DUEL, ("cruiser", "destroyer")))
}

fn step(app: &mut bevy::prelude::App, frames: u64) {
    for _ in 0..frames {
        app.update();
    }
}

/// Read every `ShipShields`-bearing entity's per-facing CHARGE out of a world,
/// keyed by `EntityUuid`, in a form two worlds can be compared field-for-field
/// **without** going through `world_digest`.
///
/// The digest deliberately does not fold shield charge — shields stay in the
/// deferred list in `deterministic-simulation.yaml` — so a continuation claim
/// about shields cannot be read off the digest the way the motion/hull claim in
/// `resume_round_trip` is; it has to read the component itself. Each facing is
/// `(arc id, hp, hp_frac, offline_remaining, is_focused)`, the same runtime
/// charge tuple the snapshot layer captures as `WeaponState::shield_charge`.
/// Per-facing runtime charge tuple: `(arc id, hp, hp_frac, offline_remaining, is_focused)`.
type FacingCharge = (String, i32, f32, f32, bool);

fn shield_charge_by_uuid(
    world: &mut bevy::prelude::World,
) -> std::collections::BTreeMap<String, Vec<FacingCharge>> {
    use project_phoenix::entities::spawner::EntityUuid;
    use project_phoenix::server_app::ShipShields;
    let mut query = world.query::<(&EntityUuid, &ShipShields)>();
    query
        .iter(world)
        .map(|(uuid, shields)| {
            (
                uuid.0.clone(),
                shields
                    .0
                    .facings
                    .iter()
                    .map(|f| {
                        (
                            f.id.clone(),
                            f.hp,
                            f.hp_frac(),
                            f.offline_remaining,
                            f.is_focused,
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

/// Bring a fresh app up to the point where the scenario's world exists.
///
/// A fresh app has no ships at tick 0 — the lobby's collective auto-start has
/// to run and the world has to spawn — so "restore into a fresh app" means
/// letting the same scenario bootstrap and then overwriting it, which is what
/// `snapshot::restore` documents. The tick, the generators, the mint and every
/// ship's state are all overwritten afterwards, so what the bootstrap did on
/// its way here does not survive into the resumed run.
fn boot_to_restore_point(args: &HeadlessArgs, snapshot: &PhoenixSnapshot) -> bevy::prelude::App {
    let mut app = boot(args);
    for _ in 0..1_000 {
        // Run Startup before asking reconciliation to queue anything. Calling it
        // against the pre-Startup empty map would race `load_extra_worlds` and
        // put the same startup layer in the queue twice.
        app.update();
        match reconcile_world_layers(app.world_mut(), snapshot) {
            LayerReconcileStatus::Ready if ready_to_restore(app.world(), snapshot) => return app,
            LayerReconcileStatus::Failed(path) => {
                panic!("world-layer reconciliation failed at {path}")
            }
            LayerReconcileStatus::Ready | LayerReconcileStatus::Waiting => {}
        }
    }
    panic!("the fresh app never reached the restore point");
}

/// The liveness guard: refuse to accept a capture of a world at rest.
///
/// Every claim in this file is of the form "the resumed world does what the
/// live one does", and a parked stalemate satisfies all of them trivially — two
/// ships sitting still at full health with cold weapons step forward
/// identically forever, and would go on doing so with half this payload
/// deleted. So the capture is asserted to be a world that is genuinely *doing*
/// something before any equality is read off it.
fn assert_capture_is_alive(payload: &PhoenixSnapshot, world: &str, expect_combat: bool) {
    let moving = payload.entities.iter().any(|e| {
        e.physics
            .is_some_and(|p| p[4] != 0.0 || p[6] != 0.0 || p[7] != 0.0)
    });
    assert!(moving, "[{world}] no captured ship has any velocity");

    // Combat Test's waves close from r >= 1000 and are still inbound at the
    // capture tick, so its liveness is motion and a belt that has streamed
    // rather than exchanged fire. The duel is where the shooting is asserted.
    if expect_combat {
        let damaged = payload.entities.iter().any(|e| {
            e.hull
                .as_ref()
                .is_some_and(|rows| rows.iter().any(|(_, current, max)| current < max))
        });
        assert!(
            damaged,
            "[{world}] no captured ship has taken damage — nothing is shooting"
        );
    }

    let armed = payload.entities.iter().any(|e| {
        e.weapons.as_ref().is_some_and(|w| {
            !w.beams.is_empty()
                || !w.phaser_cooldowns.is_empty()
                || !w.torpedoes_in_flight.is_empty()
                || !w.bursts.is_empty()
                || w.tubes
                    .iter()
                    .any(|t| t.load_phase != 0 || t.loaded_count > 0)
        })
    });
    assert!(
        armed,
        "[{world}] every captured weapon machine is idle — a cold-machine \
         restore would pass this file's assertions without restoring anything"
    );
}

/// Save, store, build a **new** app, resume into it, and stand at the same
/// state — then step both forward together.
///
/// The whole acceptance criterion in one function, parameterised by world so
/// the streamed-belt case and the fixed-roster case are the same claim rather
/// than two similar ones.
fn resume_round_trip(world: &str, args: HeadlessArgs, slot: &str, continue_for: u64) {
    let mut live = boot(&args);
    step(&mut live, CAPTURE_AT);

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());
    assert!(
        !payload.entities.is_empty(),
        "[{world}] the capture should have found the scenario's ships"
    );
    assert_eq!(
        payload.tick,
        live.world()
            .resource::<project_phoenix::sim_tick::SimTick>()
            .0,
        "[{world}] the payload's tick is the world's, not the frame count"
    );
    assert_capture_is_alive(&payload, world, world == DUEL);

    let run = run_for(
        payload.clone(),
        captured_digest,
        SEED,
        world,
        current_versions(world),
    );

    // Storage goes through `vellum_save::Store` and nothing else — phoenix
    // writes no file of its own and parses no envelope of its own.
    let store = FileStore::new(scratch(slot));
    save_to(&store, "autosave", &run).expect("the save is written");
    let reloaded =
        load_from(&store, "autosave", &current_versions(world)).expect("the save reloads");
    assert_eq!(
        reloaded, run,
        "[{world}] the artifact round-trips through RON"
    );

    let stored = reloaded
        .snapshot
        .as_ref()
        .expect("a saved game carries a snapshot");

    // A genuinely fresh construction: a new `App`, a new world, nothing shared
    // with `live` but the scenario and the seed.
    let mut resumed = boot_to_restore_point(&args, &stored.state);
    let report = restore(resumed.world_mut(), &stored.state);
    assert!(
        report.is_complete(),
        "[{world}] every captured row should have found a home: {:?}",
        report.gaps
    );
    assert_eq!(
        report.entities_restored,
        stored.state.entities.len(),
        "[{world}] every captured ship was restored"
    );
    assert_eq!(
        report.asteroids_restored,
        stored.state.asteroids.len(),
        "[{world}] every captured rock was restored — spawned if the fresh \
         app had never streamed it"
    );

    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "[{world}] the resumed world stands exactly where the capture did"
    );

    // And vellum agrees, through its own gate and its own snapshot check —
    // which is the check that matters, because it is the one a host runs.
    let mut sim = SavedGame::new(resumed.world());
    assert_eq!(
        vellum_save::verify(&reloaded, &current_versions(world), &mut sim),
        Verdict::Reproduced
    );

    // Both worlds now step forward from the same state and stay together. This
    // is the assertion the payload was widened for: an equal digest at the
    // instant of restore is a photograph, and a cold state machine is invisible
    // in one.
    for frame in 1..=continue_for {
        live.update();
        resumed.update();
        assert_eq!(
            world_digest(resumed.world()),
            world_digest(live.world()),
            "[{world}] the two worlds diverged {frame} frame(s) after the restore"
        );
    }
}

/// The acceptance criterion on the readable world, continuation and all.
///
/// Issue #997: the two worlds diverged 2 frames after the restore —
/// deterministically, at every opt-level, with byte-identical divergent digests
/// — because the capture did not carry the helm **pass surface**
/// (`HelmPassSurface`). That surface is republished from scratch every AI tick by
/// `ai_policy_state_tick`, but that system runs `.after(helm_motion_planner)`, so
/// the planner reads the surface the *previous* tick left behind. A resumed ship
/// booted it at the bootstrap's value — the fight the fresh app happened to run
/// on its way to the restore point, not the captured one — so its first
/// continuation planner tick selected a different helm leg and steered onto a
/// different bearing, a steering-intent change the digest cannot see until helm
/// integrates it into yaw ~2 ticks later. The surface now travels in the payload
/// (see `EntityState::pass_surface`), alongside the reactor allocation, blaster
/// volley state, sensor lock and arc-bearing seam that the same measurement found
/// were also default/missing in the resumed world.
#[test]
fn a_bounded_duel_resumes_into_a_fresh_app_and_steps_forward_with_it() {
    resume_round_trip(
        DUEL,
        args(DUEL, ("cruiser", "destroyer")),
        "duel",
        CONTINUE_FOR,
    );
}

/// The same acceptance across SEVERAL seeds (issue #1242).
///
/// One seed is one fight, and a resume gap that only bites when a particular
/// manoeuvre happens to span the capture tick will hide behind it — which is
/// exactly how this one hid: the divergence needed the cruiser to be mid
/// combat-orbit, on a non-AI tick, at tick 400. Sweeping the seed moves the
/// capture instant through different points of different fights, so the
/// stale-derived-state class is caught by the suite rather than by whoever next
/// re-tunes a hull.
///
/// Each seed is a full boot + capture + reload + continuation, so the sweep is
/// deliberately short.
#[test]
fn the_bounded_duel_resumes_across_several_seeds() {
    for seed in SWEPT_SEEDS {
        let mut args = args(DUEL, ("cruiser", "destroyer"));
        args.seed = Some(seed);
        resume_round_trip(DUEL, args, &format!("duel-seed-{seed}"), CONTINUE_FOR);
    }
}

/// The seeds [`the_bounded_duel_resumes_across_several_seeds`] sweeps.
///
/// NOT an arbitrary pick, and deliberately not "whichever ones are green":
/// `SEED + 7` is excluded because it FAILS, and it gets its own `#[ignore]`d
/// test below rather than being quietly dropped from this list.
const SWEPT_SEEDS: [u64; 3] = [SEED, SEED + 1, SEED + 31];

/// A resume gap this issue's seed sweep FOUND and did not fix (issue #1242).
///
/// Seed `SEED + 7` diverges ONE frame after the restore, not two — a different
/// signature from the class #1242 closed, which shows at frame 2 because a
/// steering-intent change takes a tick to reach yaw. A frame-1 divergence means
/// something the digest folds directly was already different on the first
/// continuation tick.
///
/// It is pre-existing, not a regression: measured both ways, it fails identically
/// with and without #1242's `PhoenixSnapshot::ai_world` carry. Ignored rather than
/// deleted so whoever works this seam next has the reproducer already written.
#[test]
#[ignore = "pre-existing frame-1 resume divergence found by #1242's seed sweep;             fails identically with and without that fix — needs its own diagnosis"]
fn the_bounded_duel_resumes_on_the_seed_that_still_diverges() {
    let mut args = args(DUEL, ("cruiser", "destroyer"));
    args.seed = Some(SEED + 7);
    resume_round_trip(DUEL, args, "duel-seed-divergent", CONTINUE_FOR);
}

/// **A resume lands mid-cycle and continues the SAME cycle (issue #929).**
///
/// `cycle_jitter` draws one factor when a bank lights and applies it to that
/// cycle's burn and to the cooldown behind it. The cooldown is therefore decided
/// at light time and has to travel: a payload that dropped it would leave the
/// resumed ship serving the AUTHORED rest instead of the drawn one, silently
/// falling back onto the fixed cadence for that cycle — which is the exact
/// de-synchronisation the field exists to create. That is the class of gap
/// issue #1242 was filed for, one component along, so it gets its own check
/// rather than an assumption.
///
/// This is a payload round-trip rather than a full world resume on purpose. The
/// world-level continuation tests above cover the whole fight (issue #1242
/// closed the gap that once kept one ignored); a property that has to hold for
/// EVERY bank on every hull is pinned directly here rather than through
/// whichever manoeuvres one seed happens to reach. What it asserts is the whole of what the field
/// needs: capture sees it, the artifact carries it, restore puts it back
/// unchanged, and a restored slot is distinguishable from a freshly-lit one.
#[test]
fn a_beams_drawn_cooldown_survives_capture_and_restore() {
    use project_phoenix::console::weapons::beam::{ActiveBeam, ActiveBeamSlot};

    let mut app = duel();
    step(&mut app, CAPTURE_AT);

    // Put a bank mid-cycle with a cooldown that could not have come from any
    // authored number in the fleet, so "restored the drawn value" and "re-read
    // the config" are impossible to confuse.
    const DRAWN_COOLDOWN: f32 = 9.75;
    const DRAWN_REMAINING: f32 = 2.5;
    let ship = {
        let mut q = app
            .world_mut()
            .query::<(bevy::prelude::Entity, &ActiveBeam)>();
        q.iter(app.world())
            .map(|(e, _)| e)
            .next()
            .expect("the duel's ships carry ActiveBeam")
    };
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ActiveBeam>()
        .expect("the ship carries ActiveBeam")
        .restore_live_banks([(
            "fore".to_string(),
            ActiveBeamSlot {
                target_uuid: "resume-probe-target".to_string(),
                remaining_secs: DRAWN_REMAINING,
                damage_accumulator: 0.125,
                pending_cooldown_secs: DRAWN_COOLDOWN,
            },
        )]);

    let payload = capture(app.world());
    let row = payload
        .entities
        .iter()
        .filter_map(|e| e.weapons.as_ref())
        .find_map(|w| w.beams.iter().find(|(bank, ..)| bank == "fore"))
        .expect("the capture carries the bank that was mid-cycle");
    assert_eq!(
        (row.2, row.3, row.4),
        (DRAWN_REMAINING, 0.125, DRAWN_COOLDOWN),
        "capture must carry the burn remaining, the sub-tick damage debt AND the \
         cooldown this cycle drew — the third is what a jittered cycle cannot be \
         resumed without"
    );

    // …and back in. A fresh app, restored, must hold the same three numbers.
    let mut resumed = duel();
    step(&mut resumed, 5);
    restore(resumed.world_mut(), &payload);
    let slot = {
        let mut q = resumed.world_mut().query::<&ActiveBeam>();
        q.iter(resumed.world())
            .filter_map(|b| b.live_banks().find(|(bank, _)| bank.as_str() == "fore"))
            .map(|(_, slot)| slot.clone())
            .next()
            .expect("the restored world carries the mid-cycle bank")
    };
    assert_eq!(
        (
            slot.remaining_secs,
            slot.damage_accumulator,
            slot.pending_cooldown_secs
        ),
        (DRAWN_REMAINING, 0.125, DRAWN_COOLDOWN),
        "the resumed bank continues the cycle it was in rather than redrawing or \
         falling back on the authored cooldown"
    );
}

/// The weak-broadside flip latch (issue #929) survives a save and a restore —
/// asserted on the PAYLOAD, because nothing else can see it.
///
/// The sibling above pins a field on a struct the format version names. This
/// one pins three keys in a map, and the reason it is a test rather than a
/// sentence in `SNAPSHOT_FORMAT`'s doc is that policy memory is **not folded
/// into `world_digest`**. Every digest gate in this file — the byte-identical
/// continuations, the cross-target ledger, the same-seed reruns — would stay
/// green on a payload that dropped `broadside_flip` entirely, and the only
/// symptom would be a resumed cruiser circling the wrong way with its beaten
/// arc back in the enemy's guns. That is the #1242 failure mode exactly, and
/// #1242's lesson is that "the state lives somewhere the payload already
/// carries" is a claim about the code, not evidence about the artifact.
///
/// All three keys, because they are one decision split across three slots: the
/// state, the identity of the arc that justified it, and the clock the dwell is
/// measured from. A restore that brought back the flag and lost the arc index
/// would resume a hull that flips on the next tick it re-reads a beaten arc and
/// can never clear, because the arc it names is `0.0` — a real arc, at full
/// health, that never tripped anything.
#[test]
fn a_mid_flip_broadside_survives_a_save_and_restore() {
    // Values no authored number and no live run could produce, so "restored what
    // was captured" and "re-derived something plausible" cannot be confused.
    const FLIPPED: f64 = 1.0;
    const LATCHED_ARC: f64 = 3.0;
    const LATCHED_AT: f64 = 41.5;

    let mut app = duel();
    step(&mut app, CAPTURE_AT);

    let mut payload = capture(app.world());
    let steering = payload
        .entities
        .iter_mut()
        .filter_map(|e| {
            e.control
                .as_mut()
                .and_then(|control| control.helm_policies.as_mut())
        })
        .next()
        .expect(
            "the capture must carry the STEERING policy's memory — index 1 of the \
             (engines, steering, boost) triple",
        );
    steering[1].memory.set("broadside_flip", FLIPPED);
    steering[1].memory.set("broadside_flip_arc", LATCHED_ARC);
    steering[1].memory.set("broadside_flip_since", LATCHED_AT);
    assert_eq!(
        (
            steering[1].memory.get("broadside_flip"),
            steering[1].memory.get("broadside_flip_arc"),
            steering[1].memory.get("broadside_flip_since"),
        ),
        (Some(FLIPPED), Some(LATCHED_ARC), Some(LATCHED_AT)),
        "the artifact must carry the latch, the arc it named and the clock its \
         dwell runs from. No digest covers these, so a payload that dropped them \
         would pass every other gate in this file"
    );

    // …and back in, through a fresh app that has never flipped anything.
    let mut resumed = duel();
    step(&mut resumed, 5);
    let before = capture(resumed.world())
        .entities
        .iter()
        .filter_map(|e| e.control.as_ref())
        .filter_map(|control| control.helm_policies.as_ref())
        .filter_map(|policies| policies[1].memory.get("broadside_flip"))
        .next();
    assert_eq!(
        before, None,
        "precondition: the fresh app has not latched a flip, so the assertion \
         below is a restore and not a coincidence"
    );

    restore(resumed.world_mut(), &payload);
    let restored_payload = capture(resumed.world());
    let memory = restored_payload
        .entities
        .iter()
        .filter_map(|e| e.control.as_ref())
        .filter_map(|control| control.helm_policies.as_ref())
        .map(|policies| &policies[1].memory)
        .find(|memory| memory.get("broadside_flip").is_some())
        .expect("the restored world carries the mid-flip ship");
    assert_eq!(
        (
            memory.get("broadside_flip"),
            memory.get("broadside_flip_arc"),
            memory.get("broadside_flip_since"),
        ),
        (Some(FLIPPED), Some(LATCHED_ARC), Some(LATCHED_AT)),
        "the resumed hull keeps presenting the broadside it had chosen, still \
         knows WHICH arc it is protecting, and serves the remainder of the dwell \
         it had started rather than restarting it"
    );
}

/// The acceptance criterion on **its own world**, streamed belts and all.
///
/// The capture is taken at [`CAPTURE_AT`], long after the player has left the
/// spawn point, so the rocks it names are ones the fresh app — which reaches
/// the restore point in a fraction of that time — has never had in window. A
/// restore that only corrected the rocks it found would be short of exactly
/// those, and the digest assertion inside `resume_round_trip` is what would
/// catch it.
#[test]
fn a_bounded_combat_test_resumes_with_its_streamed_belts_intact() {
    resume_round_trip(
        COMBAT_TEST,
        combat_test_args(),
        "combat-test",
        COMBAT_TEST_CONTINUE_FOR,
    );
}

/// The streamed belt, stated on its own rather than only inside a digest.
///
/// The digest assertion is the stronger claim, but it is one number: when it
/// fails it says "something moved" and not "the fresh app was thirty rocks
/// short". This is the readable half, and it also pins the thing that makes the
/// restore *stay* restored — the streamer's own window, put back so its next
/// tick resumes instead of rebuilding the belt out from under it.
#[test]
fn a_streamed_belt_comes_back_rock_for_rock_and_the_streamer_resumes() {
    let args = combat_test_args();
    let mut live = boot(&args);
    step(&mut live, CAPTURE_AT);
    let payload = capture(live.world());

    let window = payload
        .asteroid_window
        .as_ref()
        .expect("combat_test streams a belt, so it has a window");
    assert!(
        !window.needs_init,
        "the capture is taken after the streamer has initialised"
    );
    assert!(
        !payload.asteroids.is_empty(),
        "combat_test's belts should have streamed rocks in by tick {CAPTURE_AT}"
    );
    assert!(
        payload.asteroids.iter().all(|a| a.config_path.is_some()),
        "every streamed rock carries the config path a restore rebuilds it from"
    );

    let mut resumed = boot_to_restore_point(&args, &payload);
    let before: usize = resumed
        .world_mut()
        .query::<&project_phoenix::server_app::AsteroidUuid>()
        .iter(resumed.world())
        .count();
    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    let after = capture(resumed.world());
    assert_eq!(
        after.asteroids.len(),
        payload.asteroids.len(),
        "every rock came back"
    );
    for (restored, captured) in after.asteroids.iter().zip(&payload.asteroids) {
        assert_eq!(
            restored, captured,
            "rock {} did not come back as it was",
            captured.uuid
        );
    }
    assert_eq!(
        after.asteroid_window, payload.asteroid_window,
        "and the streamer's own window came back with them"
    );
    assert!(
        before != payload.asteroids.len() || report.despawned > 0,
        "this test is only meaningful if the fresh app's belt differed from \
         the capture's ({before} rocks vs {})",
        payload.asteroids.len()
    );

    // The claim the window restore exists for: the streamer's very next tick
    // recognises the restored anchor and leaves the belt alone.
    let restored_rocks = after.asteroids.len();
    resumed.update();
    assert_eq!(
        capture(resumed.world()).asteroids.len(),
        restored_rocks,
        "the next streamer tick rebuilt the belt instead of resuming it"
    );
}

/// The resumed player ship keeps its identity and its state, named field by
/// field rather than only inside a digest.
#[test]
fn the_resumed_player_ship_keeps_its_identity_and_condition() {
    let mut live = duel();
    step(&mut live, CAPTURE_AT);
    let payload = capture(live.world());
    assert_capture_is_alive(&payload, DUEL, true);

    let mut resumed = boot_to_restore_point(&args(DUEL, ("cruiser", "destroyer")), &payload);
    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    let after = capture(resumed.world());
    assert_eq!(
        after.entities, payload.entities,
        "identity, transform, motion, per-system hull, red-alert, control, \
         weapon and repair state all came back"
    );
    assert_eq!(after.tick, payload.tick);
    assert_eq!(after.rng, payload.rng, "the generators resumed mid-stream");
    assert_eq!(after.mint, payload.mint, "the id mint resumed mid-tick");
    assert!(
        after
            .entities
            .iter()
            .any(|e| e.physics.is_some() && e.hull.is_some()),
        "at least one restored ship carries both motion and per-system hull"
    );
}

/// AC2's "weapon/repair state", named rather than left inside a digest that
/// does not fold it.
#[test]
fn the_resumed_ship_keeps_its_weapon_and_repair_state() {
    let mut live = duel();
    step(&mut live, CAPTURE_AT);

    // Damage a shield arc on every ship BEFORE the capture so the round-trip
    // below proves charge is RESTORED, not defaulted back to full — the exact
    // regression issue #997's follow-up fixes (a resumed world measured ~17 HP
    // live against 100 resumed). A full-health capture would round-trip
    // trivially even if restore dropped the charge entirely, so the fixture
    // knocks the fore arc down to a known sub-max hp and focuses it: both the
    // whole-`WeaponState` equality and the explicit shield-charge assertion
    // below then have a non-default value to disagree on if the wire ever
    // forgets it.
    {
        use project_phoenix::server_app::ShipShields;
        let world = live.world_mut();
        let mut q = world.query::<&mut ShipShields>();
        let mut damaged = 0usize;
        for mut shields in q.iter_mut(world) {
            if shields.0.facings.is_empty() {
                continue;
            }
            // Focus arc 0, then damage it. `set_focused_facing` re-derives the
            // focused arc's `max_hp`; it does not touch `hp`, so the write
            // below survives — the same set order `restore_facings` relies on.
            shields.0.set_focused_facing(Some(0));
            shields.0.facings[0].hp = 17;
            damaged += 1;
        }
        assert!(
            damaged > 0,
            "the duel's ships carry shield facings to damage"
        );
    }

    let payload = capture(live.world());
    assert_capture_is_alive(&payload, DUEL, true);

    let mut resumed = boot_to_restore_point(&args(DUEL, ("cruiser", "destroyer")), &payload);
    restore(resumed.world_mut(), &payload);
    let after = capture(resumed.world());

    let captured: Vec<_> = payload
        .entities
        .iter()
        .filter_map(|e| e.weapons.as_ref().map(|w| (&e.uuid, w)))
        .collect();
    assert!(
        !captured.is_empty(),
        "the duel's ships carry weapon state machines"
    );
    for (uuid, weapons) in captured {
        let restored = after
            .entities
            .iter()
            .find(|e| &e.uuid == uuid)
            .and_then(|e| e.weapons.as_ref())
            .unwrap_or_else(|| panic!("ship {uuid} came back without its weapons"));
        assert_eq!(
            restored, weapons,
            "ship {uuid}: live beams, cooldowns, tubes, rounds in flight, \
             per-arc shield hull and shield charge all came back"
        );
    }

    // Shield CHARGE specifically (issue #997 follow-up), named rather than left
    // to ride inside the struct equality above: the capture carries the
    // damaged arc, and every captured facing's charge reappears after the
    // restore instead of defaulting to full.
    let captured_charge: Vec<_> = payload
        .entities
        .iter()
        .filter_map(|e| e.weapons.as_ref().map(|w| (&e.uuid, &w.shield_charge)))
        .filter(|(_, charge)| !charge.is_empty())
        .collect();
    assert!(
        !captured_charge.is_empty(),
        "the duel's ships carry shield charge in the capture"
    );
    assert!(
        captured_charge
            .iter()
            .any(|(_, charge)| charge.iter().any(|(_, hp, ..)| *hp == 17)),
        "the damaged arc (hp=17) was captured — the round-trip is not trivially \
         full-health, so a restore that defaulted shields to full would fail here"
    );
    for (uuid, charge) in captured_charge {
        let restored = after
            .entities
            .iter()
            .find(|e| &e.uuid == uuid)
            .and_then(|e| e.weapons.as_ref())
            .map(|w| &w.shield_charge)
            .unwrap_or_else(|| panic!("ship {uuid} came back without its shield charge"));
        assert_eq!(
            restored, charge,
            "ship {uuid}: shield charge (hp, hp_frac, offline, focus) round-tripped"
        );
    }

    for entity in &payload.entities {
        let Some(repair) = entity.repair.as_ref() else {
            continue;
        };
        let restored = after
            .entities
            .iter()
            .find(|e| e.uuid == entity.uuid)
            .and_then(|e| e.repair.as_ref())
            .unwrap_or_else(|| panic!("ship {} came back without its crew", entity.uuid));
        assert_eq!(
            restored, repair,
            "ship {}: the repair crew is where it was standing",
            entity.uuid
        );
    }
}

/// The shield-charge **continuation** claim, read DIRECTLY off `ShipShields` in
/// both worlds rather than through `world_digest`.
///
/// The digest continuation in `resume_round_trip` cannot make this claim:
/// shields are deferred from the fold (`deterministic-simulation.yaml`), so a
/// resumed ship whose shield charge drifted a frame after the restore would
/// leave that digest untouched. This test steps the live and resumed duel
/// forward together and compares each ship's per-facing charge — hp, the
/// fractional `hp_frac` accumulator, `offline_remaining` and focus — read out of
/// `ShipShields` on both worlds every frame.
///
/// It is what validates BOTH halves of the fix at once. The capture carries a
/// deliberately damaged, focused fore arc, so every ship comes back with a
/// sub-max arc that is actively **regenerating** — and shield regen is scaled by
/// `ShipModifiers` (issue #952's SHIELDS -> `ShieldRegen`), the very cache
/// `restore`'s `rebuild_power_modifiers` step settles from the restored reactor
/// allocation. Drop that step and the resumed ship regens (and, in an
/// actively-engaging duel, takes phaser damage under a wrong `PhaserDamage`) at
/// the wrong intensity for the first tick, and the per-facing charge parts from
/// the live world's within a frame or two — so the test is not vacuous: it fails
/// if `rebuild_power_modifiers` is removed.
#[test]
fn the_resumed_ship_holds_its_shield_charge_step_for_step() {
    let mut live = duel();
    step(&mut live, CAPTURE_AT);

    // Damage + focus a fore arc on every ship so the continuation has live,
    // sub-max charge to diverge on. A full arc with nothing to regen would step
    // forward identically even with `rebuild_power_modifiers` removed; a damaged
    // one regenerates at a modifier-scaled rate, which is exactly the quantity
    // that step exists to get right.
    {
        use project_phoenix::server_app::ShipShields;
        let world = live.world_mut();
        let mut q = world.query::<&mut ShipShields>();
        let mut damaged = 0usize;
        for mut shields in q.iter_mut(world) {
            if shields.0.facings.is_empty() {
                continue;
            }
            // Focus arc 0, then damage it — `set_focused_facing` re-derives the
            // focused arc's `max_hp` but not its `hp`, so the write survives.
            shields.0.set_focused_facing(Some(0));
            shields.0.facings[0].hp = 17;
            damaged += 1;
        }
        assert!(
            damaged > 0,
            "the duel's ships carry shield facings to damage"
        );
    }

    let payload = capture(live.world());
    assert_capture_is_alive(&payload, DUEL, true);
    let damaged_arc_captured = payload
        .entities
        .iter()
        .filter_map(|e| e.weapons.as_ref())
        .any(|w| w.shield_charge.iter().any(|(_, hp, ..)| *hp == 17));
    assert!(
        damaged_arc_captured,
        "the damaged arc (hp=17) rode into the capture — the continuation is \
         not a trivially-full-health one that would track even with the charge \
         dropped"
    );

    let mut resumed = boot_to_restore_point(&args(DUEL, ("cruiser", "destroyer")), &payload);
    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    // At the instant of restore the two worlds' shield charge is equal — the
    // photograph, read off the component rather than the digest.
    let live_charge = shield_charge_by_uuid(live.world_mut());
    let resumed_charge = shield_charge_by_uuid(resumed.world_mut());
    assert!(
        !live_charge.is_empty(),
        "the live duel carries shields to compare"
    );
    assert_eq!(
        resumed_charge, live_charge,
        "shield charge differed at the very instant of restore"
    );

    // And it stays equal step for step. This is the assertion the digest
    // continuation in `resume_round_trip` cannot make, because shields are not
    // in the fold: without `rebuild_power_modifiers` the resumed ship's first
    // continuation tick regens (and takes phaser damage) at the wrong intensity
    // and the charge parts here.
    for frame in 1..=CONTINUE_FOR {
        live.update();
        resumed.update();
        assert_eq!(
            shield_charge_by_uuid(resumed.world_mut()),
            shield_charge_by_uuid(live.world_mut()),
            "shield charge diverged {frame} frame(s) after the restore"
        );
    }
}

/// A save whose simulation moved is refused **before** anything is activated,
/// and the refusal is vellum's own sentence.
///
/// The assertion is deliberately on `Moved` itself and on its rendering, not on
/// a phoenix status string: this repository defines no compatibility validator,
/// so there is no phoenix status to assert on, and that absence is the point.
#[test]
fn a_save_from_other_rules_is_refused_as_moved_not_as_a_phoenix_status() {
    let mut live = duel();
    step(&mut live, 60);
    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let run = run_for(payload, digest, SEED, DUEL, current_versions(DUEL));

    let store = FileStore::new(scratch("moved"));
    save_to(&store, "autosave", &run).expect("the save is written");

    // The build's rules string moves on. Everything else — format, content,
    // bytes — is untouched, so the only reason to refuse is the one being
    // tested.
    let moved_on = Versions::new(
        SNAPSHOT_FORMAT,
        format!("{SIMULATION_RULES}-and-then-some"),
        current_versions(DUEL).content,
    );

    let refusal = load_from(&store, "autosave", &moved_on).expect_err("this build refuses it");
    let Some(moved) = (match &refusal {
        LoadRefusal::Moved(moved) => Some(moved.clone()),
        _ => None,
    }) else {
        panic!("expected a version refusal, got {refusal}");
    };
    assert!(
        matches!(moved, Moved::Rules { .. }),
        "the refusal names the dimension that moved: {moved:?}"
    );
    // The host-visible text is `Moved`'s, verbatim.
    assert_eq!(refusal.to_string(), moved.to_string());
    assert!(
        refusal.to_string().contains(SIMULATION_RULES),
        "the message says which rules the save was recorded under: {refusal}"
    );
}

/// A save from a different scenario's authored data is refused too, on the
/// dimension nobody has to remember to bump.
#[test]
fn a_save_whose_authored_data_changed_is_refused_on_content() {
    let mut live = duel();
    step(&mut live, 60);
    let run = run_for(
        capture(live.world()),
        world_digest(live.world()),
        SEED,
        DUEL,
        current_versions(DUEL),
    );

    let store = FileStore::new(scratch("content"));
    save_to(&store, "autosave", &run).expect("the save is written");

    // Simulate a designer editing the world file: same ledger shape a real
    // load would build (the world path recorded), one file's text moved.
    content_ledger::reset();
    content_ledger::record(DUEL, "# a designer touched the world file\n");
    content_ledger::freeze();
    let edited = versions(&content_ledger::frozen_or_live());
    let refusal = load_from(&store, "autosave", &edited).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Content { .. })),
        "got {refusal}"
    );
    content_ledger::reset();
}

/// Issue #935's own acceptance: a save is refused on content when an ENTITY
/// TEMPLATE's authored text changes, not only when the scenario/world TOML
/// does. This is exactly the case the old `content_digest` (scenario text
/// only) missed — an edit to `assets/entities/*.toml`, a hull config, or a
/// fragment file moved nothing, so `apply_hull` on restore would have trusted
/// the fresh world's authored maxima over the capture's.
///
/// The edit is simulated through the ledger/loader seam rather than editing a
/// real asset on disk: `duel()` already recorded the duel's real declared file
/// set, so re-recording one of those exact paths with different text is what a
/// designer's edit would have produced, without mutating a shipped asset out
/// from under the rest of the suite. The hull edited below is the PLAYER's own,
/// which reaches the ledger through `FsTemplateLoader` resolving `--ship`.
///
/// The duel's NPC hull is deliberately not the one edited: its path is a
/// computed script argument supplied by the duel transform, so no load-time
/// literal scan can put it in the frozen ledger. Issue #1047 now eagerly walks
/// both `world_config.entities` and `CompiledScripts::spawned_templates`; a
/// literal script-spawned hull is covered before freeze, while a computed path
/// is reported once when dispatched and deliberately not folded late (doing so
/// would make the save digest depend on how far the run had progressed). The
/// PLAYER's selected hull still enters through headless's explicit `--ship`
/// resolution and re-freeze, so editing it keeps this older #935 test isolated
/// from the computed-path residual.
#[test]
fn a_save_whose_entity_template_changed_is_refused_on_content() {
    let mut live = duel();
    step(&mut live, 60);
    let baseline = current_versions(DUEL);
    let run = run_for(
        capture(live.world()),
        world_digest(live.world()),
        SEED,
        DUEL,
        baseline.clone(),
    );

    let store = FileStore::new(scratch("content_entity"));
    save_to(&store, "autosave", &run).expect("the save is written");

    // The duel's own roster (see `args`, which resolves "cruiser" through
    // `headless::duel::resolve_template`) spawns `assets/entities/
    // alliance_cruiser.toml`; the real load above recorded it. Re-record it
    // with different text, the way a designer's edit would move the ledger's
    // fold, and reconstruct the ledger's frozen state around that one change.
    let live_ledger = content_ledger::frozen_or_live();
    assert!(
        live_ledger.len() > 1,
        "the duel's real load must have recorded more than the world file \
         itself, or this test proves nothing: {}",
        live_ledger.len()
    );

    content_ledger::reset();
    content_ledger::record(
        DUEL,
        &std::fs::read_to_string(DUEL).expect("world readable"),
    );
    content_ledger::record(
        "assets/entities/alliance_cruiser.toml",
        "# a designer touched the hull\n",
    );
    content_ledger::freeze();
    let edited = versions(&content_ledger::frozen_or_live());
    assert_ne!(
        edited.content, baseline.content,
        "an edited entity template must move the content digest"
    );

    let refusal = load_from(&store, "autosave", &edited).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Content { .. })),
        "got {refusal}"
    );
    content_ledger::reset();
}

/// Issue #1047 end to end, including the static-child edge: a root with no
/// `[[entity]]` loads an `extra_world`, that child names a sibling `.rhai`, and
/// only that sibling literally names the hull. Editing the hull must make
/// `load_from` return vellum's content-moved refusal.
///
/// The reset immediately AFTER each real `world::load` is load-bearing. The
/// composition validator resolves literal spawn templates through
/// `FsTemplateLoader`, which records as a side effect; leaving that record live
/// would let this test pass even if the returned child's compiled set never
/// reached the eager pre-freeze walk. Erasing it, then applying only the returned
/// `LedgerPlan` and explicitly eager-walking root and child, isolates the shipped
/// compatibility path under test.
#[test]
fn a_save_whose_script_only_hull_template_changed_is_refused_on_content() {
    struct Cleanup(std::path::PathBuf);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            content_ledger::reset();
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let dir = scratch("script_only_hull_content");
    std::fs::create_dir_all(&dir).expect("fixture directory is created");
    let _cleanup = Cleanup(dir.clone());
    content_ledger::reset();

    let hull_path = dir.join("late_hull.toml");
    let root_path = dir.join("root.toml");
    let child_path = dir.join("child.toml");
    let script_path = dir.join("child.rhai");
    let hull_ref = hull_path.to_string_lossy().replace('\\', "/");
    let root_ref = root_path.to_string_lossy().into_owned();
    let child_ref = child_path.to_string_lossy().replace('\\', "/");
    std::fs::write(&hull_path, "[hull]\nhull_integrity = 400.0\n")
        .expect("baseline hull is written");
    let root_source = r#"
extra_worlds = ["__CHILD__"]

[global]
seed = 1047
"#
    .replace("__CHILD__", &child_ref);
    std::fs::write(&root_path, root_source).expect("root world is written");
    std::fs::write(
        &child_path,
        "script = \"child.rhai\"\n\n[global]\nseed = 1048\n",
    )
    .expect("child world is written");
    let script_source = r#"
fn spawn_later(ctx) {
    ctx.effects.spawn_entity(#{ template_path: "__HULL__", name: "late-hull" });
}
"#
    .replace("__HULL__", &hull_ref);
    std::fs::write(&script_path, script_source).expect("child sibling script is written");

    let freeze_declared_set = || {
        use project_phoenix::world::load::{load, FsReader, LoadPolicy, LoadRequest};

        let resolver = project_phoenix::entities::config_cache::production_script_resolver();
        let loaded = load(LoadRequest::new(
            root_ref.clone(),
            &FsReader,
            &resolver,
            LoadPolicy::Activate,
        ))
        .expect("the root and scripted child load");
        assert!(loaded.config.entities.is_empty());
        assert_eq!(loaded.children.len(), 1);
        assert!(loaded.children[0].config.entities.is_empty());
        assert!(
            loaded.children[0].scripts.is_some(),
            "the child's sibling set must survive the load"
        );

        // Intentionally first after `load`: erase every record template
        // resolution during validation could have made.
        content_ledger::reset();
        loaded.ledger.apply();
        content_ledger::eager_record_world_entities_with_scripts(
            &loaded.config,
            loaded.scripts.as_ref(),
        );
        for child in &loaded.children {
            content_ledger::eager_record_world_entities_with_scripts(
                &child.config,
                child.scripts.as_ref(),
            );
        }
        content_ledger::freeze();
        assert!(
            content_ledger::frozen_covers(&hull_ref),
            "the explicit child eager walk must cover the sibling-only hull"
        );
        versions(&content_ledger::frozen_or_live())
    };

    let baseline = freeze_declared_set();
    let run = run_for(
        PhoenixSnapshot::default(),
        0,
        SEED,
        &root_ref,
        baseline.clone(),
    );
    let store = FileStore::new(dir.join("save"));
    save_to(&store, "autosave", &run).expect("the fixture save is written");

    std::fs::write(&hull_path, "[hull]\nhull_integrity = 401.0\n")
        .expect("the hull edit is written");
    let edited = freeze_declared_set();
    assert_ne!(
        edited.content, baseline.content,
        "editing the literal script-only hull must move the frozen content digest"
    );

    let refusal = load_from(&store, "autosave", &edited).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Content { .. })),
        "got {refusal}"
    );
}

/// The digest must not depend on the ORDER the loader happened to record
/// files in — same file set, same text, recorded in the opposite order, must
/// fold to the same number on either target.
#[test]
fn content_digest_does_not_depend_on_record_order() {
    content_ledger::reset();
    content_ledger::record("assets/entities/a.toml", "a");
    content_ledger::record("assets/entities/b.toml", "b");
    content_ledger::freeze();
    let forward = versions(&content_ledger::frozen_or_live());

    content_ledger::reset();
    content_ledger::record("assets/entities/b.toml", "b");
    content_ledger::record("assets/entities/a.toml", "a");
    content_ledger::freeze();
    let backward = versions(&content_ledger::frozen_or_live());

    assert_eq!(
        forward.content, backward.content,
        "the same file set recorded in a different order must fold to the same digest"
    );
    content_ledger::reset();
}

/// The ledger must reset between loads rather than accumulate — a fresh
/// `build_headless_app` call must not carry the previous world's files into
/// the new one's digest.
#[test]
fn the_content_ledger_resets_between_loads() {
    content_ledger::reset();
    content_ledger::record("assets/entities/a.toml", "a");
    content_ledger::freeze();
    assert!(!content_ledger::frozen_or_live().is_empty());

    // A real second load: `duel()` runs `build_headless_app`, which resets
    // the ledger before recording anything of its own.
    let _second_load = duel();
    let after_second_load = content_ledger::frozen_or_live();
    assert!(
        !after_second_load.is_empty(),
        "the second load must have recorded its own files"
    );

    // The stale entry from the first "load" must be gone, not folded
    // alongside the real one — proven indirectly: a ledger that still held
    // both would already be caught by the two assertions above having
    // different content than a ledger built from `duel()` alone. Assert that
    // directly too, since content_ledger's fields are private:
    content_ledger::reset();
    content_ledger::record("assets/entities/a.toml", "a");
    let _third_load = duel();
    let via_stale_then_reload = content_ledger::frozen_or_live();
    assert_eq!(
        after_second_load, via_stale_then_reload,
        "a stale entry recorded before a real load must not survive into that \
         load's frozen digest"
    );
}

// ── Scripted scenario progression (issue #864) ───────────────────────────────

/// The scripted fixture world — see `tests/fixtures/worlds/scripted_resume.toml`
/// for why its two events are timed where they are.
const SCRIPTED: &str = "tests/fixtures/worlds/scripted_resume.toml";

/// Frames the scripted world runs before its capture: past the ~tick-60 timer
/// that fires `relay_signal`, and well short of the ~tick-300 callback that
/// handler schedules. The capture therefore sits *between* the two, which is the
/// only window in which either half of this issue is observable.
const SCRIPT_CAPTURE_AT: u64 = 150;

/// Frames both worlds are stepped after the restore — enough to carry them
/// across the callback's fire tick and out the other side.
const SCRIPT_CONTINUE_FOR: u64 = 220;

fn scripted_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: SCRIPTED.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn world_counter(app: &bevy::prelude::App, name: &str) -> i64 {
    app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .flags
        .counter(name)
}

fn queued_callbacks(app: &bevy::prelude::App) -> usize {
    app.world()
        .get_resource::<project_phoenix::world::server::WorldScriptRuntime>()
        .map_or(0, |script| script.pending_callbacks.len())
}

fn phase_of(app: &bevy::prelude::App) -> GamePhase {
    app.world().resource::<State<GamePhase>>().get().clone()
}

/// The scenario state a scripted world resumes from, named rather than left
/// inside a digest that folds none of it.
fn scenario_of(payload: &PhoenixSnapshot) -> &project_phoenix::snapshot::ScenarioState {
    payload
        .scenario
        .as_ref()
        .expect("a world with a content runtime captures its scenario state")
}

/// AC "restoring preserves each pending action's remaining timing and identity",
/// and "resumed actions fire once according to the scenario contract".
///
/// The capture is taken while a scripted `after(4, |ctx| …)` callback is still
/// in flight — scheduled by a timer handler the fresh app has not reached, so
/// the fresh app has no callback of its own for the restore to accidentally
/// agree with. The callback ends the run in a declared victory, which moves
/// `GamePhase`, so the two worlds' digests part company on the fire tick if the
/// queue did not travel.
#[test]
fn a_pending_script_callback_survives_a_resume_and_fires_on_its_own_tick() {
    let mut live = boot(&scripted_args());
    step(&mut live, SCRIPT_CAPTURE_AT);

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());
    let scenario = scenario_of(&payload);

    // The capture is genuinely mid-schedule: one callback queued, and its fire
    // tick is still in the future. A capture taken after the fire would satisfy
    // every assertion below without carrying anything.
    assert_eq!(
        scenario.script_callbacks.len(),
        1,
        "the timer handler should have left exactly one callback queued"
    );
    assert!(
        scenario.script_callbacks[0].fire_tick > payload.tick,
        "the captured callback should still be pending: fire_tick {} vs capture tick {}",
        scenario.script_callbacks[0].fire_tick,
        payload.tick
    );
    assert_eq!(
        world_counter(&live, "beacon_pulses"),
        1,
        "the timer handler should have run exactly once before the capture"
    );
    assert_eq!(
        world_counter(&live, "relief_arrived"),
        0,
        "the deferred callback must NOT have fired before the capture"
    );

    let mut resumed = boot_to_restore_point(&scripted_args(), &payload);
    // The fresh app has reached neither event — so every claim below is about
    // what the payload carried, not about what the bootstrap happened to redo.
    assert_eq!(
        world_counter(&resumed, "beacon_pulses"),
        0,
        "the fresh app should still be short of the 1 s timer at the restore point"
    );
    assert_eq!(
        queued_callbacks(&resumed),
        0,
        "the fresh app has scheduled no callback of its own, so a resumed one \
         can only have come from the save"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        queued_callbacks(&resumed),
        1,
        "the restore should have queued the captured callback"
    );
    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "the resumed world stands exactly where the capture did"
    );

    // Both worlds now step across the callback's fire tick together.
    let mut live_fired_at = None;
    let mut resumed_fired_at = None;
    for frame in 1..=SCRIPT_CONTINUE_FOR {
        live.update();
        resumed.update();
        if live_fired_at.is_none() && phase_of(&live) == GamePhase::GameOver {
            live_fired_at = Some(frame);
        }
        if resumed_fired_at.is_none() && phase_of(&resumed) == GamePhase::GameOver {
            resumed_fired_at = Some(frame);
        }
        assert_eq!(
            world_digest(resumed.world()),
            world_digest(live.world()),
            "the two worlds diverged {frame} frame(s) after the restore"
        );
    }

    assert!(
        live_fired_at.is_some(),
        "the live callback should have fired within {SCRIPT_CONTINUE_FOR} frames \
         of the capture — retune SCRIPT_CONTINUE_FOR if the fixture's delay moved"
    );
    assert_eq!(
        resumed_fired_at, live_fired_at,
        "the resumed callback fired on a different frame than the live one"
    );
    assert_eq!(
        world_counter(&resumed, "relief_arrived"),
        1,
        "the resumed callback fired exactly once"
    );
    assert_eq!(
        world_counter(&live, "relief_arrived"),
        world_counter(&resumed, "relief_arrived"),
        "and the live world agrees on how many times it fired"
    );
}

/// AC "resumed actions fire once rather than duplicating, disappearing, or
/// restarting" — the *already spent* half of it.
///
/// The scripted `on_timer(1, …)` trigger has fired before the save and must not
/// fire again. Nothing about the resumed world makes that automatic: the restore
/// also puts the mission clock back to the capture's ~2.5 s, so a re-armed
/// single-shot latch would find its 1 s threshold already met and fire on the
/// very next tick. `beacon_pulses` reading 2 is what that looks like.
#[test]
fn a_scripted_trigger_that_already_fired_does_not_fire_again_after_a_resume() {
    let mut live = boot(&scripted_args());
    step(&mut live, SCRIPT_CAPTURE_AT);

    let payload = capture(live.world());
    let scenario = scenario_of(&payload);
    assert!(
        scenario.triggers.iter().any(|t| t.fired),
        "the capture should record the timer trigger as fired"
    );
    let captured_elapsed = scenario
        .mission_elapsed_secs
        .expect("a running mission has an anchored clock");
    assert!(
        captured_elapsed > 1.0,
        "the capture should be past the 1 s timer threshold, got {captured_elapsed}"
    );
    assert_eq!(world_counter(&live, "beacon_pulses"), 1);

    let mut resumed = boot_to_restore_point(&scripted_args(), &payload);
    assert_eq!(
        world_counter(&resumed, "beacon_pulses"),
        0,
        "the fresh app has not reached the timer, so a post-restore reading of 1 \
         can only have been restored and a reading of 2 can only be a re-fire"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        world_counter(&resumed, "beacon_pulses"),
        1,
        "the flag counter came back with the rest of the scenario"
    );

    // The mission clock came back too — the thing that makes the re-fire
    // temptation real rather than theoretical.
    let after = capture(resumed.world());
    let resumed_elapsed = scenario_of(&after)
        .mission_elapsed_secs
        .expect("the restore re-anchored the mission clock");
    assert!(
        (resumed_elapsed - captured_elapsed).abs() < 1e-3,
        "the resumed mission clock reads {resumed_elapsed}s, the capture's was \
         {captured_elapsed}s"
    );
    assert_eq!(
        scenario_of(&after).triggers,
        scenario_of(&payload).triggers,
        "every trigger's latch, destroyed-set and cooldown clock round-tripped"
    );

    // And it stays fired: 120 frames of a mission clock reading well past the
    // threshold, with the counter never moving.
    for frame in 1..=120u64 {
        resumed.update();
        assert_eq!(
            world_counter(&resumed, "beacon_pulses"),
            1,
            "the spent timer re-fired {frame} frame(s) after the restore"
        );
    }
}

/// A save whose **scripts** moved is refused, on the same dimension an edited
/// entity template moves — the mirror of
/// `a_save_whose_entity_template_changed_is_refused_on_content`.
///
/// The binding is `WorldScriptRuntime::content_hash`, the compiled script set's
/// own digest, recorded into the content ledger by `load_world_scripts` under
/// `script_ledger_key`. This test proves the binding is really that number and
/// not an accident of the world TOML's text riding along: re-recording the key
/// with the value the live runtime is holding is a *no-op* on the digest, and
/// moving only that one entry is what refuses the save.
#[test]
fn a_save_whose_script_content_changed_is_refused_on_content() {
    let mut live = boot(&scripted_args());
    step(&mut live, 30);

    let compiled_hash = live
        .world()
        .resource::<project_phoenix::world::server::WorldScriptRuntime>()
        .content_hash;
    let baseline = current_versions(SCRIPTED);
    let run = run_for(
        capture(live.world()),
        world_digest(live.world()),
        SEED,
        SCRIPTED,
        baseline.clone(),
    );
    let store = FileStore::new(scratch("script_content"));
    save_to(&store, "autosave", &run).expect("the save is written");

    // Re-freezing the ledger the real load built, untouched, must not move the
    // digest — otherwise the "only the script entry changed" claim below would
    // be measuring whatever 30 frames of stepping happened to record.
    content_ledger::freeze();
    assert_eq!(
        versions(&content_ledger::frozen_or_live()).content,
        baseline.content,
        "stepping the world recorded new content, so this test cannot isolate \
         the script edit"
    );

    // Recording the compiled set's OWN hash under the loader's key changes
    // nothing — which is the proof the load already bound the save to it.
    content_ledger::record_digest(&script_ledger_key(SCRIPTED), compiled_hash);
    content_ledger::freeze();
    assert_eq!(
        versions(&content_ledger::frozen_or_live()).content,
        baseline.content,
        "the world load should already have recorded WorldScriptRuntime's own \
         content_hash under this key"
    );

    // Now move only that entry, the way editing a handler body would.
    content_ledger::record_digest(
        &script_ledger_key(SCRIPTED),
        compiled_hash ^ 0x9e37_79b9_7f4a_7c15,
    );
    content_ledger::freeze();
    let edited = versions(&content_ledger::frozen_or_live());
    assert_ne!(
        edited.content, baseline.content,
        "an edited script set must move the content digest"
    );

    let refusal = load_from(&store, "autosave", &edited).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Content { .. })),
        "got {refusal}"
    );
    content_ledger::reset();
}

/// The script-free fixture (see the file's own header for why it is a fixture).
const SCRIPT_FREE: &str = "tests/fixtures/worlds/declarative_resume.toml";

fn script_free_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: SCRIPT_FREE.into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// A **script-free** world's payload is well-formed and carries nothing
/// script-shaped — the compatibility half of this issue, stated rather than
/// assumed.
///
/// A world with no `[script]` block compiles no `WorldScriptRuntime` at all. Its
/// capture must still carry a mission clock (not a scripting feature) and leave
/// the script-only fields empty, so a world with nothing to progress produces a
/// payload that still round-trips.
///
/// The subject was `combat_test` until issue #984 converted it — the last
/// shipped world to convert — leaving no script-free shipped world to point at.
/// It has been `declarative_resume.toml` since. The trigger-latch half of the
/// claim went with issue #985: a script-free world has no triggers to latch,
/// because the `[[trigger]]` front-end that was its only source is gone.
#[test]
fn a_script_free_world_captures_scenario_state_without_a_script_runtime() {
    let mut live = boot(&script_free_args());
    step(&mut live, 120);
    assert!(
        live.world()
            .get_resource::<project_phoenix::world::server::WorldScriptRuntime>()
            .is_none(),
        "the fixture authors no scripts, so it must compile no script runtime"
    );

    let payload = capture(live.world());
    let scenario = scenario_of(&payload);
    assert!(
        scenario.script_callbacks.is_empty(),
        "a script-free world queues no callbacks"
    );
    assert!(
        scenario.triggers.is_empty(),
        "a script-free world has no triggers at all: the `[[trigger]]` front-end          that was their only source went in issue #985"
    );
    assert!(
        scenario.mission_elapsed_secs.is_some(),
        "a running mission has an anchored clock whether or not it is scripted"
    );

    // And the payload still round-trips through the artifact unchanged.
    let run = run_for(
        payload.clone(),
        world_digest(live.world()),
        SEED,
        SCRIPT_FREE,
        current_versions(SCRIPT_FREE),
    );
    let store = FileStore::new(scratch("script-free"));
    save_to(&store, "autosave", &run).expect("the save is written");
    let reloaded = load_from(&store, "autosave", &current_versions(SCRIPT_FREE))
        .expect("the save reloads")
        .snapshot
        .expect("a saved game carries a snapshot");
    assert_eq!(
        reloaded.state.scenario, payload.scenario,
        "the scenario state round-trips through RON"
    );
}

/// An empty slot is not a failure: a first run has no save.
#[test]
fn a_slot_with_nothing_in_it_is_not_an_error() {
    let store = FileStore::new(scratch("empty"));
    assert_eq!(
        load_from(&store, "autosave", &current_versions(DUEL)),
        Err(LoadRefusal::Empty)
    );
}

/// Force the live world into `GameOver` through the same seam production code
/// uses — write `GameOverReason` then `NextState<GamePhase>`, and let the
/// fixed-schedule `StateTransition` (issue #895) carry it through
/// `OnEnter(GameOver)` on the very next `app.update()`. Deliberately *not*
/// `snapshot::restore`'s direct `State::new` write — that write is exactly the
/// path issue #934 is about, and this helper exists to produce a capture of a
/// world that went through the real thing.
fn force_game_over(
    app: &mut bevy::prelude::App,
    reason: &str,
    outcome: project_phoenix::core::balance::Outcome,
) {
    {
        let world = app.world_mut();
        let mut game_over = world.resource_mut::<GameOverReason>();
        game_over.0 = Some(reason.to_string());
        game_over.1 = Some(outcome);
        world
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::GameOver);
    }
    app.update();
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::GameOver,
        "the forced transition should have landed before capture"
    );
}

/// Issue #934's pin: restoring a captured `GameOver` re-runs the phase's entry
/// effects, not just its phase label.
///
/// `restore_run_scope`'s `State::new(phase)` write (the fix this test is
/// against) bypasses Bevy's `OnEnter`/`OnExit` schedules entirely, so a
/// restored `GameOver` used to come back with the *label* `GameOver` but none
/// of what a live transition into it does — no `ServerMessage::GameOver` for
/// clients, no HUD flip. The fresh app this test restores into is proof the
/// gap was real: `boot_to_restore_point` only waits for the roster
/// (`ready_to_restore`), so it is still sitting in its own `InProgress` — it
/// never made this transition on its own, and nothing but the restore itself
/// can produce the entry effect.
///
/// The reason string is asserted as `""`, not the string this test forces —
/// and that is a finding, not a shortcut. `on_game_over_enter` (`server_app.rs`)
/// reads `GameOverReason` with `.take()`, so the live transition this test
/// forces already consumes the reason into its own one-shot broadcast; by the
/// time `capture` runs (even on the very next line), the resource it reads
/// holds `None`, and every restore downstream inherits that. That consuming
/// design predates #934 and this issue's scope is "the effects never ran at
/// all", not "the reason resource is a `.take()`" — so the assertion below
/// pins what a restore actually reproduces (the broadcast fires) rather than a
/// string this codebase cannot hand it. The outcome half of the same resource
/// is not `.take()`n by anything, and the assertion on it below is what proves
/// the restored `GameOverReason` — not just the phase label — came back.
#[test]
fn a_restored_game_over_reruns_its_entry_effects() {
    let mut live = duel();
    step(&mut live, CAPTURE_AT);
    force_game_over(
        &mut live,
        "hull breach",
        project_phoenix::core::balance::Outcome::Defeat,
    );

    let payload = capture(live.world());
    assert_eq!(
        payload.phase,
        Some(GamePhase::GameOver),
        "the capture should have recorded the forced GameOver phase"
    );
    assert_eq!(
        payload
            .game_over
            .as_ref()
            .map(|(_, outcome)| outcome.clone()),
        Some(Some("defeat".to_string())),
        "the outcome half of GameOverReason is never `.take()`n, so the \
         capture should still carry it even though the reason half is gone"
    );

    let mut resumed = boot_to_restore_point(&args(DUEL, ("cruiser", "destroyer")), &payload);
    assert_eq!(
        resumed.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "the fresh app should still be mid-run, not GameOver, at the restore point"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    assert_eq!(
        resumed.world().resource::<State<GamePhase>>().get(),
        &GamePhase::GameOver,
        "the restored phase label should be GameOver"
    );
    assert_eq!(
        resumed.world().resource::<GameOverReason>().1,
        Some(project_phoenix::core::balance::Outcome::Defeat),
        "the restored GameOverReason's outcome should have survived — \
         on_game_over_enter only takes the reason half"
    );

    let outbox = &resumed.world().resource::<SimOutbox>().0;
    let reasons: Vec<&str> = outbox
        .iter()
        .filter_map(|(_, msg)| match msg {
            ServerMessage::GameOver { reason, .. } => Some(reason.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        vec![""],
        "restoring a captured GameOver should have re-run on_game_over_enter \
         and pushed the ServerMessage::GameOver the fresh app's own game \
         start never emitted (found {} other outbox entries)",
        outbox.len() - reasons.len()
    );
}

/// The other half of #934's fix: a save button is a no-op outside a run, and
/// says so, rather than recording a `Lobby`/`Loading` phase a restore would
/// have nothing meaningful to re-enter.
///
/// This pins `capture`'s own behaviour, not the wasm-only status surface in
/// `server/bridge.rs` that gates the save button on it — that surface is
/// `#[cfg(target_arch = "wasm32")]` and outside this binary's reach, so what
/// is testable from here is the phase `capture` itself records: a `Lobby`
/// world's capture should carry a phase a caller can refuse on.
#[test]
fn a_lobby_capture_is_distinguishable_from_a_run_in_progress() {
    let live = boot(&args(DUEL, ("cruiser", "destroyer")));
    let payload = capture(live.world());
    assert_eq!(
        payload.phase,
        Some(GamePhase::Lobby),
        "a fresh app's capture should record Lobby, the phase the save-button \
         guard (server/bridge.rs) refuses to write a save from"
    );
}

// ── Scripted comms across a resume (issue #984, S8) ──────────────────────────

/// The scripted-comms fixture — see
/// `tests/fixtures/worlds/scripted_comms_resume.toml` for why its one open is
/// timed where it is and why the sender is a real station.
const SCRIPTED_COMMS: &str = "tests/fixtures/worlds/scripted_comms_resume.toml";

/// Frames the comms fixture is allowed to run while hunting for its capture
/// frame. Generous: the open lands ~1 s (~tick 60) in and the hunt stops the
/// instant it sees it, so this is a runaway guard rather than a tuning knob.
const COMMS_HUNT_LIMIT: u64 = 400;

/// Frames both worlds are stepped after the restore — past thread B's open,
/// past both AI answers, and out the other side of the victory they declare.
const COMMS_CONTINUE_FOR: u64 = 180;

fn scripted_comms_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: SCRIPTED_COMMS.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// The comms state a mid-conversation save resumes from, named rather than left
/// inside a digest that folds none of it.
fn comms_of(payload: &PhoenixSnapshot) -> &project_phoenix::snapshot::CommsState {
    payload
        .comms
        .as_ref()
        .expect("a world with a comms runtime captures its comms state")
}

fn live_dialogue_count(app: &bevy::prelude::App) -> usize {
    app.world()
        .resource::<project_phoenix::comms::server::CommsRuntime>()
        .active_dialogues
        .len()
}

fn live_inbox(app: &bevy::prelude::App) -> Vec<project_phoenix::core::messages::CommsMessage> {
    app.world()
        .resource::<project_phoenix::comms::server::CommsInboxRes>()
        .0
        .messages()
}

/// Step until the fixture's thread A is open and stop on that frame.
///
/// A hunt rather than a frame constant, and that is deliberate rather than a
/// convenience: the window this file needs is ONE frame wide by construction
/// (`open_scripted_comms_threads` runs in `SimSet::Physics`, the Backfill Comms
/// AI answers in the next tick's `SimSet::Input`), so a hard-coded number would
/// be a number that happened to work on the machine it was written on. The
/// caller asserts the shape of what it caught.
fn step_to_the_open_thread(app: &mut bevy::prelude::App) -> u64 {
    for frame in 1..=COMMS_HUNT_LIMIT {
        app.update();
        if live_dialogue_count(app) > 0 {
            return frame;
        }
    }
    panic!("the scripted comms fixture never opened a thread in {COMMS_HUNT_LIMIT} frames");
}

/// S8's acceptance: a save taken with a scripted dialogue OPEN comes back
/// answerable, and answering it does the same thing it would have done live.
///
/// The capture is taken on the one frame where thread A is shown and unanswered
/// (see the fixture). The resumed world is then stepped alongside the live one
/// across the Backfill Comms AI's answer, and the claim is read off
/// `world_digest` every frame — which folds no comms state at all, and does not
/// need to: answering mints the follow-up thread's ids from the tick-scoped
/// `WorldIdMint` (whose per-namespace counters the digest DOES fold) and the
/// second thread's `on_pick` ends the run in a declared victory (`GamePhase`,
/// also folded). A resumed world that came back with an empty `active_dialogues`
/// answers nothing, mints nothing and never gets there.
#[test]
fn a_scripted_dialogue_open_at_the_save_is_answerable_after_a_resume() {
    let mut live = boot(&scripted_comms_args());
    let opened_at = step_to_the_open_thread(&mut live);

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());
    let comms = comms_of(&payload);

    // The capture is genuinely mid-conversation. Each of these is a separate
    // way the measurement could have been vacuous.
    assert_eq!(
        comms.dialogues.len(),
        1,
        "the capture should hold exactly one open scripted dialogue \
         (caught on frame {opened_at})"
    );
    let script = &comms.dialogues[0].script;
    assert_eq!(
        script.node_fn, "axiom_root",
        "the open dialogue should be sitting on the thread's root node"
    );
    assert_eq!(
        script.on_pick,
        vec!["on_first_ack".to_string()],
        "the response's on_pick fn travels alongside the button it answers"
    );
    assert_eq!(
        comms.dialogues[0].responses,
        vec![("Acknowledged.".to_string(), false)],
        "the shown response text and its `important` flag travel too"
    );
    assert_eq!(comms.inbox.len(), 1, "one message is seated in the inbox");
    assert!(
        comms.inbox[0].selected_response.is_none(),
        "the captured message must be UNANSWERED, or this test measures nothing"
    );
    assert!(
        comms.inbox[0].is_urgent,
        "the fixture opens the thread urgent, so the flag should have travelled"
    );
    assert_eq!(
        world_counter(&live, "first_answered"),
        0,
        "the AI has not answered yet at the capture"
    );

    let mut resumed = boot_to_restore_point(&scripted_comms_args(), &payload);
    // The fresh app has not reached the ~1 s timer, so everything below can only
    // have come from the payload.
    assert_eq!(
        live_dialogue_count(&resumed),
        0,
        "the fresh app has opened no thread of its own at the restore point"
    );
    assert!(
        live_inbox(&resumed).is_empty(),
        "and its inbox is empty, so a restored message can only be the save's"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        live_dialogue_count(&resumed),
        1,
        "the restore should have seated the captured dialogue"
    );
    assert_eq!(
        live_inbox(&resumed),
        live_inbox(&live),
        "and the inbox it answers, message for message"
    );
    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "the resumed world stands exactly where the capture did"
    );

    // Both worlds now step across the AI's answer, thread B's open, ITS answer,
    // and the victory that answer declares.
    let mut live_answered_at = None;
    let mut resumed_answered_at = None;
    for frame in 1..=COMMS_CONTINUE_FOR {
        live.update();
        resumed.update();
        if live_answered_at.is_none() && world_counter(&live, "first_answered") > 0 {
            live_answered_at = Some(frame);
        }
        if resumed_answered_at.is_none() && world_counter(&resumed, "first_answered") > 0 {
            resumed_answered_at = Some(frame);
        }
        assert_eq!(
            world_digest(resumed.world()),
            world_digest(live.world()),
            "the two worlds diverged {frame} frame(s) after the restore"
        );
    }

    assert!(
        live_answered_at.is_some(),
        "the live Backfill Comms AI should have answered within \
         {COMMS_CONTINUE_FOR} frames of the capture"
    );
    assert_eq!(
        resumed_answered_at, live_answered_at,
        "the resumed dialogue was answered on a different frame than the live one"
    );
    assert_eq!(
        world_counter(&resumed, "first_closed"),
        world_counter(&live, "first_closed"),
        "and the thread ran on through its follow-up, answer for answer"
    );
    assert!(
        world_counter(&live, "first_closed") > 0,
        "the live thread should have reached its closing response, or the \
         digest claim above never crossed anything"
    );
    assert_eq!(
        phase_of(&resumed),
        phase_of(&live),
        "including the victory the last on_pick declares"
    );
}

/// S8's other half: an `open_comms` that was QUEUED but not yet drained at the
/// save fires after the resume.
///
/// The fixture's root node fn opens a second thread while it is itself running
/// inside the drain, and a nested request is re-queued for the next tick rather
/// than entered re-entrantly — so the capture frame catches exactly one request
/// sitting on `pending_comms_opens`. Nothing else produces that state: every
/// other `open_comms` path queues and drains inside one tick.
#[test]
fn a_queued_open_comms_request_survives_a_resume_and_fires() {
    let mut live = boot(&scripted_comms_args());
    step_to_the_open_thread(&mut live);

    let payload = capture(live.world());
    let comms = comms_of(&payload);
    assert_eq!(
        comms.pending_opens.len(),
        1,
        "the root node fn's nested open should be queued and undrained"
    );
    assert_eq!(
        comms.pending_opens[0].root_fn, "axiom_second",
        "and it should name the second thread's root node fn"
    );
    assert_eq!(
        world_counter(&live, "second_thread_opened"),
        0,
        "the queued open must NOT have fired before the capture"
    );

    let mut resumed = boot_to_restore_point(&scripted_comms_args(), &payload);
    assert_eq!(
        world_counter(&resumed, "second_thread_opened"),
        0,
        "the fresh app has queued no open of its own, so a resumed one can only \
         have come from the save"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    // One frame is all it takes: the queue is drained on the next tick.
    resumed.update();
    live.update();
    assert_eq!(
        world_counter(&resumed, "second_thread_opened"),
        1,
        "the restored request should have opened its thread on the very next tick"
    );
    assert_eq!(
        world_counter(&live, "second_thread_opened"),
        world_counter(&resumed, "second_thread_opened"),
        "and the live world agrees it fired exactly once"
    );
    assert_eq!(
        live_inbox(&resumed).len(),
        2,
        "the second thread's message joined the restored inbox"
    );
}

/// A **comms-quiet** world's payload carries the comms state that exists and
/// nothing conversation-shaped — the compatibility half of this slice.
///
/// The duel authors no `[script]`, so it has no threads and no script runtime. Its capture must still produce a `CommsState`
/// (the runtime exists on every world), leave every conversation field empty,
/// and round-trip through RON.
#[test]
fn a_comms_quiet_world_captures_an_empty_comms_state() {
    let mut live = duel();
    step(&mut live, 120);

    let payload = capture(live.world());
    let comms = comms_of(&payload);
    assert!(comms.inbox.is_empty(), "the duel seats no messages");
    assert!(comms.dialogues.is_empty(), "and opens no threads");
    assert!(comms.pending_opens.is_empty(), "and queues no opens");

    let run = run_for(
        payload.clone(),
        world_digest(live.world()),
        SEED,
        DUEL,
        current_versions(DUEL),
    );
    let store = FileStore::new(scratch("comms-quiet"));
    save_to(&store, "autosave", &run).expect("the save is written");
    let reloaded = load_from(&store, "autosave", &current_versions(DUEL))
        .expect("the save reloads")
        .snapshot
        .expect("a saved game carries a snapshot");
    assert_eq!(
        reloaded.state.comms, payload.comms,
        "the comms state round-trips through RON"
    );
}

/// A save written before comms state was recorded is refused on **format**.
///
/// Every field `CommsState` added carries `#[serde(default)]`, so the older
/// payload still parses — which is exactly why the constant had to move. The
/// payload cannot tell "this world had no conversation open" from "this save
/// predates conversations being recorded", and restoring the second one puts a
/// scenario mid-thread into a world with an empty inbox and no dialogue to
/// answer. `Versions::check` is what refuses it, and it names the dimension.
#[test]
fn a_save_written_before_comms_state_is_refused_on_format() {
    let mut live = boot(&scripted_comms_args());
    step_to_the_open_thread(&mut live);

    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let current = current_versions(SCRIPTED_COMMS);

    // Recorded under the PREVIOUS format, everything else untouched, so the only
    // reason to refuse is the one being tested.
    let previous = Versions::new(SNAPSHOT_FORMAT - 1, SIMULATION_RULES, current.content);
    let run = run_for(payload, digest, SEED, SCRIPTED_COMMS, previous);
    let store = FileStore::new(scratch("comms-format"));
    save_to(&store, "autosave", &run).expect("the save is written");

    let refusal = load_from(&store, "autosave", &current).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Format { .. })),
        "the refusal names the dimension that moved: {refusal}"
    );
}

// `uncarried_comms_state_is_reported_as_a_gap_not_dropped_quietly` lived here.
// It drove a hand-built payload carrying `uncarried_dialogues` /
// `uncarried_follow_ups` and asserted the restore REPORTED what it could not
// carry: a declarative node's responses held `TriggerAction`s and a nested
// follow-up tree, and a queued `PendingFollowUp` held a whole node, so neither
// could travel without a serde derive on the authored-config vocabulary.
//
// Issue #985 deleted the front-end that could author either. Every node is now
// built by `project_node` and reduces losslessly by construction, and the
// follow-up queue is gone, so both fields — and the two `RestoreGap` variants
// that reported them — went with it. `SNAPSHOT_FORMAT` moved to 4 for exactly
// that reason: a format-3 payload could still contain them, and reading one
// here would silently drop state this build has nowhere to put.

// ── Named mission deadlines across a resume (issue #1024) ────────────────────

const DEADLINES: &str = "tests/fixtures/worlds/deadline_resume.toml";
const LAYER_DEADLINES: &str = "tests/fixtures/worlds/layer_deadline_resume.toml";
const DYNAMIC_LAYERS: &str = "tests/fixtures/worlds/layer_dynamic_resume.toml";
const LAYER_DEADLINE_PATH: &str = "tests/fixtures/layer_deadline.toml";
const LAYER_NESTED_PARENT_PATH: &str = "tests/fixtures/layer_nested_parent.toml";
const LAYER_NESTED_CHILD_PATH: &str = "tests/fixtures/layer_nested_child.toml";
const LAYER_ENTITY_PATH: &str = "tests/fixtures/layer_entities.toml";

/// Frames the deadline world runs before its capture: past the ~tick-60 timer
/// that slips one deadline and cancels the other, and short of every fire tick.
const DEADLINE_CAPTURE_AT: u64 = 150;

/// Frames both worlds are stepped after the restore — enough to carry them
/// across the slipped deadline's fire tick (~tick 480) and out the other side.
const DEADLINE_CONTINUE_FOR: u64 = 400;

fn deadline_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: DEADLINES.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn layer_deadline_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: LAYER_DEADLINES.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn dynamic_layer_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: DYNAMIC_LAYERS.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn queue_layer_load(app: &mut bevy::prelude::App, path: &str, loader_path: Option<&str>) {
    use project_phoenix::world::server::{PendingWorldLayerChanges, WorldLayerChange};
    app.world_mut()
        .resource_mut::<PendingWorldLayerChanges>()
        .0
        .push(WorldLayerChange::Load {
            path: path.to_string(),
            loader_path: loader_path.map(str::to_string),
        });
}

fn queue_layer_unload(app: &mut bevy::prelude::App, path: &str) {
    use project_phoenix::world::server::{PendingWorldLayerChanges, WorldLayerChange};
    app.world_mut()
        .resource_mut::<PendingWorldLayerChanges>()
        .0
        .push(WorldLayerChange::Unload(path.to_string()));
}

fn layer_is_active(app: &bevy::prelude::App, path: &str) -> bool {
    app.world()
        .resource::<project_phoenix::world::server::WorldLayerMap>()
        .0
        .get(path)
        .is_some_and(|layer| layer.is_active)
}

fn step_until_layers_are_active(app: &mut bevy::prelude::App, paths: &[&str]) {
    for _ in 0..300 {
        if paths.iter().all(|path| layer_is_active(app, path)) {
            return;
        }
        app.update();
    }
    panic!("layers never became active: {paths:?}");
}

fn step_until_layer_is_absent(app: &mut bevy::prelude::App, path: &str) {
    for _ in 0..300 {
        let absent = !app
            .world()
            .resource::<project_phoenix::world::server::WorldLayerMap>()
            .0
            .contains_key(path);
        let settled = app
            .world()
            .resource::<project_phoenix::world::server::PendingWorldLayerChanges>()
            .0
            .is_empty();
        if absent && settled {
            return;
        }
        app.update();
    }
    panic!("layer never unloaded: {path}");
}

fn layer_counter(app: &bevy::prelude::App, path: &str, name: &str) -> i64 {
    app.world()
        .resource::<project_phoenix::world::server::WorldLayerMap>()
        .0
        .get(path)
        .map_or(0, |layer| layer.flags.counter(name))
}

#[test]
fn active_empty_layers_are_captured_in_order_but_failed_sentinels_are_terminal() {
    use project_phoenix::world::server::{PendingWorldLayerChanges, WorldLayerMap, WorldRuntime};

    let parent = "tests/fixtures/active-empty-parent.toml";
    let child = "tests/fixtures/active-empty-child.toml";
    let sentinel = "tests/fixtures/refused-layer.toml";

    let mut source = bevy::prelude::App::new();
    source.insert_resource(WorldLayerMap(std::collections::HashMap::from([
        (
            child.to_string(),
            WorldRuntime {
                is_active: true,
                activation_order: 8,
                loader_path: Some(parent.to_string()),
                ..Default::default()
            },
        ),
        (
            parent.to_string(),
            WorldRuntime {
                is_active: true,
                activation_order: 7,
                ..Default::default()
            },
        ),
        (sentinel.to_string(), WorldRuntime::default()),
    ])));
    let payload = capture(source.world());
    assert_eq!(
        payload
            .layer_flags
            .iter()
            .map(|layer| layer.path.as_str())
            .collect::<Vec<_>>(),
        vec![parent, child],
        "a successful content-empty layer is active, a failed sentinel is not, \
         and HashMap order never reaches the payload"
    );
    assert_eq!(payload.layer_flags[1].loader_path.as_deref(), Some(parent));

    let mut failed_target = bevy::prelude::App::new();
    failed_target.insert_resource(PendingWorldLayerChanges::default());
    failed_target.insert_resource(WorldLayerMap(std::collections::HashMap::from([
        (
            parent.to_string(),
            WorldRuntime {
                is_active: true,
                activation_order: 1,
                ..Default::default()
            },
        ),
        (child.to_string(), WorldRuntime::default()),
    ])));
    assert_eq!(
        reconcile_world_layers(failed_target.world_mut(), &payload),
        LayerReconcileStatus::Failed(child.to_string()),
        "a desired failed sentinel is a named terminal refusal, never a retry loop"
    );
    assert!(
        failed_target
            .world()
            .resource::<PendingWorldLayerChanges>()
            .0
            .is_empty(),
        "terminal failure queues neither an unload nor an identical retry"
    );

    let mut stale_target = bevy::prelude::App::new();
    stale_target.insert_resource(PendingWorldLayerChanges::default());
    stale_target.insert_resource(WorldLayerMap(std::collections::HashMap::from([(
        sentinel.to_string(),
        WorldRuntime::default(),
    )])));
    let empty = PhoenixSnapshot::default();
    assert_eq!(
        reconcile_world_layers(stale_target.world_mut(), &empty),
        LayerReconcileStatus::Waiting
    );
    assert_eq!(
        stale_target
            .world()
            .resource::<PendingWorldLayerChanges>()
            .0
            .len(),
        1,
        "a non-target sentinel is queued for removal so it cannot remain a dedup blocker"
    );
}

// ── Issue #1025: infrastructure condition survives a resume ─────────────────

/// The infrastructure probe: one transfer depot walked down through its
/// operational threshold and back up again on a script clock.
const INFRASTRUCTURE: &str = "assets/worlds/probe_infrastructure.toml";

/// Frames to run before the infrastructure capture.
///
/// The probe damages the depot at t=1 s and repairs it at t=3 s, so this has to
/// land between the two: a capture taken before the damage would round-trip an
/// intact structure with every flag up, which is exactly the payload a restore
/// that dropped the whole field would also produce. The assertions below make
/// the precondition explicit rather than trusting the number.
const INFRASTRUCTURE_CAPTURE_AT: u64 = 150;

fn infrastructure_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: INFRASTRUCTURE.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn live_deadlines(app: &bevy::prelude::App) -> &project_phoenix::world::deadlines::DeadlineTable {
    &app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .deadlines
}

/// A slipped deadline and a cancelled one both survive a resume, and the
/// resumed world fires exactly what the live one fires, on the same tick.
///
/// The capture sits between the mutations and every fire tick. What makes the
/// measurement discriminating is that the fresh app restored into has run its
/// OWN arming — so before the restore it holds `window_opens` at its authored
/// 5 s and `stand_down` still pending, with two calls queued where the capture
/// has one. Every claim below is therefore about what the payload carried, not
/// about what the bootstrap happened to redo.
#[test]
fn a_slipped_and_a_cancelled_deadline_both_survive_a_resume() {
    use project_phoenix::world::deadlines::DeadlineState;

    let mut live = boot(&deadline_args());
    step(&mut live, DEADLINE_CAPTURE_AT);

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());
    let scenario = scenario_of(&payload);

    // The capture is genuinely mid-flight: the slip has happened, the cancel has
    // happened, and nothing has fired.
    let captured_window = scenario
        .deadlines
        .get("window_opens")
        .expect("the slipped deadline is captured")
        .clone();
    assert_eq!(captured_window.state, DeadlineState::Pending);
    assert!(
        captured_window.due_tick > payload.tick,
        "the captured deadline is still owed: due {} vs capture tick {}",
        captured_window.due_tick,
        payload.tick
    );
    assert_eq!(
        scenario.deadlines.get("stand_down").map(|d| d.state),
        Some(DeadlineState::Cancelled),
        "and the cancelled one is captured as cancelled"
    );
    assert_eq!(
        scenario.script_callbacks.len(),
        1,
        "one queued call — the cancelled deadline's was retracted, not merely marked"
    );
    assert_eq!(
        world_counter(&live, "window_fired"),
        0,
        "nothing has fired before the capture"
    );

    let mut resumed = boot_to_restore_point(&deadline_args(), &payload);

    // The bootstrap's own state, before the restore overwrites it. The restore
    // point is reached while the world exists but the mission has not started,
    // so the fresh app has armed NOTHING — every claim below is therefore about
    // what the payload carried, with no bootstrap value for it to agree with by
    // accident.
    assert!(
        live_deadlines(&resumed).records.is_empty()
            && !live_deadlines(&resumed).armed,
        "precondition: the fresh app is short of the mission's first tick, so it          has armed no deadline of its own"
    );
    assert_eq!(
        queued_callbacks(&resumed),
        0,
        "precondition: and has queued no call of its own"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    assert_eq!(
        live_deadlines(&resumed).get("window_opens"),
        Some(&captured_window),
        "the restore takes the captured record whole — slipped due tick, state, \
         and the queued call it is waiting on"
    );
    assert_eq!(
        live_deadlines(&resumed).get("stand_down").map(|d| d.state),
        Some(DeadlineState::Cancelled),
        "a cancelled deadline stays cancelled rather than coming back armed"
    );
    assert!(
        live_deadlines(&resumed).armed,
        "the armed latch travels too, so the arming system does not re-arm over it"
    );
    assert_eq!(
        queued_callbacks(&resumed),
        1,
        "the capture's single queued call is what the resumed world is waiting on"
    );
    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "the resumed world stands exactly where the capture did"
    );

    // Both worlds now step across the slipped deadline's fire tick together.
    // `arm_mission_deadlines` runs again on the resumed world's way through —
    // the mission's first tick is still ahead of it — and the restored `armed`
    // latch is the only thing stopping it arming a second, authored-time copy
    // over the top of the capture's.
    step(&mut live, DEADLINE_CONTINUE_FOR);
    step(&mut resumed, DEADLINE_CONTINUE_FOR);

    assert_eq!(
        live_deadlines(&resumed).records.len(),
        2,
        "the arming system did not re-arm over the restored table"
    );

    assert_eq!(
        world_counter(&live, "window_fired"),
        1,
        "precondition: the live world's slipped deadline fired inside the window"
    );
    assert_eq!(
        world_counter(&resumed, "window_fired"),
        1,
        "and the resumed world fires it exactly once — not twice, not never"
    );
    assert_eq!(world_counter(&live, "stand_down_fired"), 0);
    assert_eq!(
        world_counter(&resumed, "stand_down_fired"),
        0,
        "the cancelled deadline never fires on either side of the resume"
    );
    assert_eq!(
        phase_of(&resumed),
        phase_of(&live),
        "the deadline's own effect — a declared victory — lands on both"
    );
    assert_eq!(
        world_digest(resumed.world()),
        world_digest(live.world()),
        "and the two worlds are still standing in the same place afterwards"
    );
}

/// A supporting world's ownership survives the whole save boundary. The fresh
/// bootstrap is required to finish loading the layer before restore; restore
/// then replaces its freshly armed row/call rather than merging a duplicate.
#[test]
fn a_layer_deadline_resumes_in_its_owner_without_duplicate_arming() {
    let mut live = boot(&layer_deadline_args());
    step(&mut live, 30);
    let payload = capture(live.world());
    let scenario = scenario_of(&payload);

    assert!(payload
        .layer_flags
        .iter()
        .any(|layer| layer.path == LAYER_DEADLINE_PATH));
    assert_eq!(
        scenario
            .deadlines
            .records
            .iter()
            .filter(|record| record.origin_layer.as_deref() == Some(LAYER_DEADLINE_PATH))
            .count(),
        2
    );
    let captured_calls: Vec<_> = scenario
        .script_callbacks
        .iter()
        .filter(|call| call.origin_layer.as_deref() == Some(LAYER_DEADLINE_PATH))
        .cloned()
        .collect();
    assert_eq!(
        captured_calls.len(),
        1,
        "only the future shared deadline remains queued"
    );

    let mut resumed = boot_to_restore_point(&layer_deadline_args(), &payload);
    assert!(resumed
        .world()
        .resource::<project_phoenix::world::server::WorldLayerMap>()
        .0
        .contains_key(LAYER_DEADLINE_PATH));
    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    let restored = resumed
        .world()
        .resource::<project_phoenix::world::server::WorldScriptRuntime>();
    let restored_calls: Vec<_> = restored
        .pending_callbacks
        .0
        .iter()
        .filter(|call| call.origin_layer.as_deref() == Some(LAYER_DEADLINE_PATH))
        .cloned()
        .collect();
    assert_eq!(
        restored_calls, captured_calls,
        "restore replaces bootstrap arming exactly; it never appends a duplicate"
    );

    for _ in 0..200 {
        live.update();
        resumed.update();
    }
    assert_eq!(
        layer_counter(&resumed, LAYER_DEADLINE_PATH, "deadline_fired"),
        1
    );
    assert_eq!(
        layer_counter(&resumed, LAYER_DEADLINE_PATH, "nested_fired"),
        1
    );
    assert_eq!(
        world_counter(&resumed, "deadline_fired_outward"),
        world_counter(&live, "deadline_fired_outward")
    );
    assert_eq!(world_counter(&resumed, "deadline_fired_outward"), 1);
    assert_eq!(world_counter(&resumed, "nested_fired_outward"), 1);
}

#[test]
fn dynamically_loaded_order_ownership_entities_and_deadlines_resume_exactly() {
    use project_phoenix::entities::spawner::EntityUuid;
    use project_phoenix::world::server::{
        PendingWorldLayerChanges, WorldContentRuntime, WorldLayerMap, WorldScriptRuntime,
    };

    let mut live = boot(&dynamic_layer_args());
    step(&mut live, 20);

    // Parent and child both contribute scripted triggers. Their paths sort in
    // the opposite order to their activation, making this a real handler-index
    // order check rather than a set-equality check that happens to pass.
    queue_layer_load(&mut live, LAYER_NESTED_PARENT_PATH, None);
    step_until_layers_are_active(
        &mut live,
        &[LAYER_NESTED_PARENT_PATH, LAYER_NESTED_CHILD_PATH],
    );
    queue_layer_load(
        &mut live,
        LAYER_DEADLINE_PATH,
        Some(LAYER_NESTED_PARENT_PATH),
    );
    step_until_layers_are_active(&mut live, &[LAYER_DEADLINE_PATH]);
    queue_layer_load(&mut live, LAYER_ENTITY_PATH, Some(LAYER_NESTED_PARENT_PATH));
    step_until_layers_are_active(&mut live, &[LAYER_ENTITY_PATH]);
    step(&mut live, 30);

    let payload = capture(live.world());
    assert_eq!(
        payload
            .layer_flags
            .iter()
            .map(|layer| layer.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            LAYER_NESTED_PARENT_PATH,
            LAYER_NESTED_CHILD_PATH,
            LAYER_DEADLINE_PATH,
            LAYER_ENTITY_PATH,
        ],
        "the capture carries activation order, not lexical path order"
    );
    for child in [
        LAYER_NESTED_CHILD_PATH,
        LAYER_DEADLINE_PATH,
        LAYER_ENTITY_PATH,
    ] {
        assert_eq!(
            payload
                .layer_flags
                .iter()
                .find(|layer| layer.path == child)
                .and_then(|layer| layer.loader_path.as_deref()),
            Some(LAYER_NESTED_PARENT_PATH),
            "{child} keeps the layer that owns its parent: flag scope"
        );
    }

    let entity_layer = payload
        .layer_flags
        .iter()
        .find(|layer| layer.path == LAYER_ENTITY_PATH)
        .expect("the entity layer is captured");
    let captured_entity_uuid = entity_layer
        .declared_entity_uuids
        .first()
        .and_then(Option::as_deref)
        .expect("the live declared layer entity has a captured identity")
        .to_string();
    assert!(
        payload
            .entities
            .iter()
            .any(|row| row.uuid == captured_entity_uuid && row.spawn.is_none()),
        "the layer-declared entity has no script SpawnOrigin recipe; topology must recover it"
    );

    let captured_calls: Vec<_> = scenario_of(&payload)
        .script_callbacks
        .iter()
        .filter(|call| call.origin_layer.as_deref() == Some(LAYER_DEADLINE_PATH))
        .cloned()
        .collect();
    assert_eq!(captured_calls.len(), 1, "one future deadline is armed");

    // Drive the public reconciliation seam explicitly so the assertion can see
    // the fresh UUID immediately before it is exchanged for the captured one.
    let mut resumed = boot(&dynamic_layer_args());
    let mut fresh_entity_uuid = None;
    for _ in 0..1_000 {
        resumed.update();
        let all_active = [
            LAYER_NESTED_PARENT_PATH,
            LAYER_NESTED_CHILD_PATH,
            LAYER_DEADLINE_PATH,
            LAYER_ENTITY_PATH,
        ]
        .iter()
        .all(|path| layer_is_active(&resumed, path));
        let settled = resumed
            .world()
            .resource::<PendingWorldLayerChanges>()
            .0
            .is_empty();
        if all_active && settled && fresh_entity_uuid.is_none() {
            let entity = resumed
                .world()
                .resource::<WorldLayerMap>()
                .0
                .get(LAYER_ENTITY_PATH)
                .and_then(|layer| layer.spawned_entities.first())
                .copied()
                .expect("the fresh activation spawned its declared entity");
            fresh_entity_uuid = resumed
                .world()
                .get::<EntityUuid>(entity)
                .map(|uuid| uuid.0.clone());
        }
        match reconcile_world_layers(resumed.world_mut(), &payload) {
            LayerReconcileStatus::Ready if ready_to_restore(resumed.world(), &payload) => break,
            LayerReconcileStatus::Failed(path) => {
                panic!("dynamic topology failed to reconcile at {path}")
            }
            LayerReconcileStatus::Ready | LayerReconcileStatus::Waiting => continue,
        }
    }
    let fresh_entity_uuid = fresh_entity_uuid.expect("the fresh layer entity was observed");
    assert_ne!(
        fresh_entity_uuid, captured_entity_uuid,
        "loading the dynamic layer at another tick must genuinely exercise UUID reconciliation"
    );
    let restored_layer_entity = resumed
        .world()
        .resource::<WorldLayerMap>()
        .0
        .get(LAYER_ENTITY_PATH)
        .and_then(|layer| layer.spawned_entities.first())
        .copied()
        .expect("the reconciled layer still owns its entity");
    assert_eq!(
        resumed
            .world()
            .get::<EntityUuid>(restored_layer_entity)
            .map(|uuid| uuid.0.as_str()),
        Some(captured_entity_uuid.as_str()),
        "identity is repaired before entity-state restore looks up the captured UUID"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    let runtime = resumed.world().resource::<WorldContentRuntime>();
    let scripts = resumed.world().resource::<WorldScriptRuntime>();
    assert_eq!(runtime.trigger_states.len(), scripts.handlers.len());
    for path in [LAYER_NESTED_PARENT_PATH, LAYER_NESTED_CHILD_PATH] {
        assert_eq!(
            runtime
                .trigger_states
                .iter()
                .zip(&scripts.handlers)
                .filter(|(state, handler)| {
                    state.origin_layer.as_deref() == Some(path) && handler.is_some()
                })
                .count(),
            1,
            "{path} has exactly one aligned trigger handler after resume"
        );
    }
    let restored_calls: Vec<_> = scripts
        .pending_callbacks
        .0
        .iter()
        .filter(|call| call.origin_layer.as_deref() == Some(LAYER_DEADLINE_PATH))
        .cloned()
        .collect();
    assert_eq!(
        restored_calls, captured_calls,
        "deadline arming is replaced"
    );

    step(&mut live, 200);
    step(&mut resumed, 200);
    assert_eq!(
        layer_counter(&resumed, LAYER_DEADLINE_PATH, "deadline_fired"),
        1
    );
    assert_eq!(
        layer_counter(&resumed, LAYER_DEADLINE_PATH, "nested_fired"),
        1
    );
    assert_eq!(
        layer_counter(&resumed, LAYER_NESTED_PARENT_PATH, "deadline_fired_outward"),
        1
    );
    assert_eq!(
        layer_counter(&resumed, LAYER_NESTED_PARENT_PATH, "nested_fired_outward"),
        1
    );
    assert_eq!(
        layer_counter(&resumed, LAYER_NESTED_CHILD_PATH, "child_arrived"),
        layer_counter(&live, LAYER_NESTED_CHILD_PATH, "child_arrived"),
        "the child opening handler does not re-arm after trigger-state restore"
    );
}

#[test]
fn a_dynamically_unloaded_startup_layer_stays_absent_after_resume() {
    use project_phoenix::world::server::{WorldContentRuntime, WorldLayerMap, WorldScriptRuntime};

    let mut live = boot(&layer_deadline_args());
    step_until_layers_are_active(&mut live, &[LAYER_DEADLINE_PATH]);
    step(&mut live, 30);
    queue_layer_unload(&mut live, LAYER_DEADLINE_PATH);
    step_until_layer_is_absent(&mut live, LAYER_DEADLINE_PATH);

    let payload = capture(live.world());
    assert!(
        payload.layer_flags.is_empty(),
        "the captured active composition records the runtime unload, not WorldConfig.extra_worlds"
    );
    assert!(scenario_of(&payload)
        .deadlines
        .records
        .iter()
        .all(|record| record.origin_layer.as_deref() != Some(LAYER_DEADLINE_PATH)));
    assert!(scenario_of(&payload)
        .script_callbacks
        .iter()
        .all(|call| call.origin_layer.as_deref() != Some(LAYER_DEADLINE_PATH)));

    // The fresh world loads the startup layer again. Reconciliation must remove
    // that extra before callbacks/deadlines are restored, even though the saved
    // topology is the empty vector.
    let mut resumed = boot_to_restore_point(&layer_deadline_args(), &payload);
    assert!(!resumed
        .world()
        .resource::<WorldLayerMap>()
        .0
        .contains_key(LAYER_DEADLINE_PATH));
    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    let runtime = resumed.world().resource::<WorldContentRuntime>();
    let scripts = resumed.world().resource::<WorldScriptRuntime>();
    assert!(runtime
        .trigger_states
        .iter()
        .all(|state| state.origin_layer.as_deref() != Some(LAYER_DEADLINE_PATH)));
    assert_eq!(runtime.trigger_states.len(), scripts.handlers.len());
    assert!(scripts
        .pending_callbacks
        .0
        .iter()
        .all(|call| call.origin_layer.as_deref() != Some(LAYER_DEADLINE_PATH)));
    assert!(scripts.ast_owners.values().all(|owners| !owners
        .iter()
        .any(|owner| owner.as_deref() == Some(LAYER_DEADLINE_PATH))));

    step(&mut live, 200);
    step(&mut resumed, 200);
    assert_eq!(world_counter(&resumed, "deadline_fired_outward"), 0);
    assert_eq!(world_counter(&resumed, "nested_fired_outward"), 0);
    assert_eq!(
        world_counter(&resumed, "deadline_fired_outward"),
        world_counter(&live, "deadline_fired_outward")
    );
}

/// The deadline table round-trips through the save's RON, and a world that
/// authors no deadline writes nothing deadline-shaped at all.
#[test]
fn the_deadline_table_round_trips_and_a_deadline_free_world_writes_none() {
    let mut live = boot(&deadline_args());
    step(&mut live, DEADLINE_CAPTURE_AT);
    let payload = capture(live.world());

    let run = run_for(
        payload.clone(),
        world_digest(live.world()),
        SEED,
        DEADLINES,
        current_versions(DEADLINES),
    );
    let store = FileStore::new(scratch("deadline-roundtrip"));
    save_to(&store, "autosave", &run).expect("the save is written");
    let reloaded = load_from(&store, "autosave", &current_versions(DEADLINES))
        .expect("the save reloads")
        .snapshot
        .expect("a saved game carries a snapshot");
    assert_eq!(
        scenario_of(&reloaded.state).deadlines,
        scenario_of(&payload).deadlines,
        "every field a run moves — due tick, state, and the queued call — \
         round-trips through RON"
    );

    // The compatibility half: the duel authors no `[[deadline]]`, so its payload
    // is shaped exactly as it was before this slice (the field skips when empty).
    let mut quiet = duel();
    step(&mut quiet, 120);
    let quiet_payload = capture(quiet.world());
    assert!(
        scenario_of(&quiet_payload).deadlines.is_empty(),
        "a world with no authored deadlines captures no deadline state"
    );
}

/// A save written before deadline state was recorded is refused on **format**.
///
/// Every field [`ScenarioState::deadlines`] added carries `#[serde(default)]`,
/// so the older payload still parses — which is exactly why the constant had to
/// move. The payload cannot tell "this world authored no deadlines" from "this
/// save predates deadlines being recorded", and restoring the second one re-arms
/// every deadline the run had cancelled and rewinds every slip. `Versions::check`
/// is what refuses it, and it names the dimension.
#[test]
fn a_save_written_before_deadline_state_is_refused_on_format() {
    let mut live = boot(&deadline_args());
    step(&mut live, DEADLINE_CAPTURE_AT);

    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let current = current_versions(DEADLINES);

    // Recorded under the PREVIOUS format, everything else untouched, so the only
    // reason to refuse is the one being tested.
    let previous = Versions::new(SNAPSHOT_FORMAT - 1, SIMULATION_RULES, current.content);
    let run = run_for(payload, digest, SEED, DEADLINES, previous);
    let store = FileStore::new(scratch("deadline-format"));
    save_to(&store, "autosave", &run).expect("the save is written");

    let refusal = load_from(&store, "autosave", &current).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Format { .. })),
        "the refusal names the dimension that moved: {refusal}"
    );
}

/// **Issue #1025.** A degraded structure comes back degraded — its condition
/// *and* which of its operational flags are currently down.
///
/// The flag half is the one worth having a test for. Restore the number alone
/// and the first tick after a resume re-detects every crossing the mission
/// already spent, re-firing `on_flag_cleared` on a skyhook that failed twenty
/// minutes ago; the flags travel with the condition precisely so it cannot.
#[test]
fn the_resumed_world_keeps_a_structures_condition_and_its_operational_flags() {
    let mut live = boot(&infrastructure_args());
    step(&mut live, INFRASTRUCTURE_CAPTURE_AT);

    let payload = capture(live.world());
    let captured: Vec<_> = payload
        .entities
        .iter()
        .filter_map(|e| e.infrastructure.as_ref().map(|i| (&e.uuid, i)))
        .collect();
    assert_eq!(
        captured.len(),
        1,
        "the probe world carries exactly one structure with a condition track"
    );
    let (uuid, track) = captured[0];
    assert!(
        track.condition() < track.condition_max(),
        "precondition: the capture must be taken after the scripted damage — a capture of an \
         intact depot would round-trip identically even if restore dropped the field entirely \
         (condition {} of {})",
        track.condition(),
        track.condition_max()
    );
    assert_eq!(
        track.flag("depot_transfer_capable"),
        Some(false),
        "precondition: and after the crossing, so the flag under test is DOWN at capture time"
    );

    let mut resumed = boot_to_restore_point(&infrastructure_args(), &payload);
    let before_restore = resumed
        .world_mut()
        .query::<&project_phoenix::infrastructure::InfrastructureCondition>()
        .iter(resumed.world())
        .next()
        .map(|c| c.0.clone())
        .expect("the fresh world spawned the depot from its template");
    assert_eq!(
        before_restore.flag("depot_transfer_capable"),
        Some(true),
        "control: a freshly booted depot is intact and capable, so an inert restore would be \
         visible here rather than hidden behind a value that happened to match"
    );

    restore(resumed.world_mut(), &payload);
    let after = capture(resumed.world());
    let restored = after
        .entities
        .iter()
        .find(|e| &e.uuid == uuid)
        .and_then(|e| e.infrastructure.as_ref())
        .unwrap_or_else(|| panic!("structure {uuid} came back without its condition track"));
    assert_eq!(
        restored, track,
        "structure {uuid}: condition, ceiling, every operational flag and the hull reading the \
         track was last damaged against must all come back exactly as captured"
    );
}

// ── Issue #1028: civilian traffic survives a resume ─────────────────────────

/// The civilian probe: four haulers on two authored lanes, ordered at t = 2 s.
const CIVILIAN: &str = "assets/worlds/probe_civilian_traffic.toml";

/// Frames to run before the civilian capture.
///
/// The probe issues its four orders at t = 2 s and the default disposition
/// answers two seconds later, so this has to land while at least one craft is
/// still MID-NEGOTIATION — received or acknowledged, with a due tick pending.
/// A capture taken before the orders would round-trip four unordered craft,
/// which is exactly the payload a restore that dropped the whole field would
/// also produce. The assertions below make the precondition explicit rather
/// than trusting the number.
const CIVILIAN_CAPTURE_AT: u64 = 130;

fn civilian_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: CIVILIAN.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// **Issue #1028.** A negotiation in progress comes back in progress: the
/// order, where the craft stands with it, and the tick it is due to answer on.
///
/// The due tick is the half worth having a test for. Restore the compliance
/// state without it and a craft frozen mid-acknowledgement either answers on
/// the first tick after a resume or never answers at all — and both look
/// exactly like a save that worked, right up until the crew notices the hauler
/// never turned.
#[test]
fn the_resumed_world_keeps_a_civilians_lane_order_and_place_in_the_negotiation() {
    use project_phoenix::civilian::{CivilianTraffic, ComplianceState};

    let mut live = boot(&civilian_args());
    step(&mut live, CIVILIAN_CAPTURE_AT);

    let payload = capture(live.world());
    let captured: Vec<_> = payload
        .entities
        .iter()
        .filter_map(|e| e.civilian.as_ref().map(|c| (&e.uuid, c)))
        .collect();
    assert_eq!(
        captured.len(),
        4,
        "the probe world carries exactly four craft with traffic state"
    );
    assert!(
        captured
            .iter()
            .any(|(_, state)| state.compliance().is_pending() && state.due_tick() > 0),
        "precondition: the capture must be taken while at least one craft is still \
         answering — a capture of four unordered craft would round-trip identically even \
         if restore dropped the field entirely. Captured: {:?}",
        captured
            .iter()
            .map(|(id, s)| (id, s.compliance(), s.due_tick()))
            .collect::<Vec<_>>()
    );

    let mut resumed = boot_to_restore_point(&civilian_args(), &payload);
    let before_restore: Vec<ComplianceState> = resumed
        .world_mut()
        .query::<&CivilianTraffic>()
        .iter(resumed.world())
        .map(|t| t.0.compliance())
        .collect();
    assert!(
        before_restore
            .iter()
            .all(|c| *c == ComplianceState::Unordered),
        "control: a freshly booted world's traffic is under no orders at all, so an inert \
         restore would be visible here rather than hidden behind a value that happened to \
         match: {before_restore:?}"
    );

    restore(resumed.world_mut(), &payload);
    let after = capture(resumed.world());
    for (uuid, state) in captured {
        let restored = after
            .entities
            .iter()
            .find(|e| &e.uuid == uuid)
            .and_then(|e| e.civilian.as_ref())
            .unwrap_or_else(|| panic!("craft {uuid} came back without its traffic state"));
        assert_eq!(
            restored, state,
            "craft {uuid}: its lane, its leg, its standing order, where it stands with that \
             order and the tick that stage is due on must all come back exactly as captured"
        );
    }
}

// ── The commitments ledger across a resume (issue #1029) ─────────────────────

/// The commitments probe: two promises made to the same party, one kept on a
/// script clock and one left for a deadline to break.
const COMMITMENTS: &str = "assets/worlds/probe_commitments.toml";

/// Frames the commitments world runs before its capture.
///
/// It has to land in the window where the two promises DISAGREE — after
/// `safe_passage` is kept at t=6 s and before the deadline breaks
/// `surface_records` at t=10 s. A capture taken before the keep would round-trip
/// two open promises, which is the same payload a restore that dropped the field
/// entirely would also produce. The assertions below make that precondition
/// explicit rather than trusting the number.
const COMMITMENTS_CAPTURE_AT: u64 = 480;

/// Frames both worlds are stepped after the restore — enough to carry them
/// across the deadline that breaks the second promise and the second dialogue
/// open that reads the first one back.
const COMMITMENTS_CONTINUE_FOR: u64 = 400;

fn commitments_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: COMMITMENTS.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn live_commitments(
    app: &bevy::prelude::App,
) -> &project_phoenix::world::commitments::CommitmentLedger {
    &app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .commitments
}

/// A kept promise and an open one both survive a resume, and the resumed world
/// settles the open one exactly as the live world does.
///
/// The ledger is **not** folded into the simulation digest — it sits with
/// `FlagStore` and `ObjectiveManager` on that side of the line — so digest
/// agreement below is a check that the resume did not disturb the *simulation*,
/// and every claim about the promises themselves is asserted directly. That is
/// deliberate: a test that only compared digests would pass with the ledger
/// dropped on the floor.
#[test]
fn a_kept_promise_and_an_open_one_both_survive_a_resume() {
    use project_phoenix::world::commitments::CommitmentState;

    let mut live = boot(&commitments_args());
    step(&mut live, COMMITMENTS_CAPTURE_AT);

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());
    let scenario = scenario_of(&payload);

    // The capture is genuinely mid-flight: one promise settled, one still owed.
    // Those two states are what make the measurement discriminating.
    let captured_passage = scenario
        .commitments
        .get("safe_passage")
        .expect("the kept promise is captured")
        .clone();
    assert_eq!(
        captured_passage.state,
        CommitmentState::Kept,
        "precondition: the capture is taken AFTER the promise was kept — a capture \
         of two open promises would round-trip identically even if restore dropped \
         the field entirely"
    );
    assert_eq!(
        scenario.commitments.get("surface_records").map(|c| c.state),
        Some(CommitmentState::Open),
        "precondition: and BEFORE the deadline broke the other one"
    );
    assert_eq!(
        captured_passage.made_to, "skyway_strike_committee",
        "the party travels with the promise"
    );
    assert_eq!(
        world_counter(&live, "records_broken_by_deadline"),
        0,
        "nothing has broken a promise before the capture"
    );

    let mut resumed = boot_to_restore_point(&commitments_args(), &payload);

    // The bootstrap's own state, before the restore overwrites it. A promise is
    // only ever written by a script call, so a fresh app short of the mission's
    // first tick has made none — which is what stops any claim below being
    // satisfied by a bootstrap coincidence.
    assert!(
        live_commitments(&resumed).is_empty(),
        "precondition: the fresh app has given nobody its word"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    assert_eq!(
        live_commitments(&resumed).get("safe_passage"),
        Some(&captured_passage),
        "the restore takes the captured promise whole — party, terms, stated \
         resolution condition, state, and both tick stamps"
    );
    assert_eq!(
        live_commitments(&resumed).state_of("surface_records"),
        "open",
        "and the promise still owed comes back owed rather than unmade"
    );
    assert_eq!(
        live_commitments(&resumed).state_of("never_promised"),
        "unknown",
        "a promise the run never made is still unknown after a resume — which is \
         what stops a resumed scenario re-offering a word already given"
    );
    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "the resumed world stands exactly where the capture did"
    );

    // Both worlds now step across the deadline that breaks the open promise.
    step(&mut live, COMMITMENTS_CONTINUE_FOR);
    step(&mut resumed, COMMITMENTS_CONTINUE_FOR);

    assert_eq!(
        live_commitments(&live).state_of("surface_records"),
        "broken",
        "precondition: the live world's deadline settled the open promise"
    );
    assert_eq!(
        live_commitments(&resumed).state_of("surface_records"),
        "broken",
        "and the resumed world's deadline settles it too — the promise it \
         restored was a real one, owed to a real party, waiting on a real clock"
    );
    assert_eq!(
        live_commitments(&resumed).state_of("safe_passage"),
        "kept",
        "while the promise already kept is not re-settled by the resumed run"
    );
    assert_eq!(
        world_counter(&resumed, "commitment.surface_records.broken"),
        world_counter(&live, "commitment.surface_records.broken"),
        "the campaign flag is written exactly once on both sides of the resume"
    );
    assert_eq!(
        world_counter(&resumed, "broken_flag_chained"),
        world_counter(&live, "broken_flag_chained"),
        "and the on_flag_set trigger watching it fired the same number of times"
    );
    assert_eq!(
        world_digest(resumed.world()),
        world_digest(live.world()),
        "and the two worlds are still standing in the same place afterwards"
    );
}

/// The ledger round-trips through the save's RON, and a run that gave nobody
/// its word writes nothing commitment-shaped at all.
#[test]
fn the_commitment_ledger_round_trips_and_a_promise_free_run_writes_none() {
    let mut live = boot(&commitments_args());
    step(&mut live, COMMITMENTS_CAPTURE_AT);
    let payload = capture(live.world());

    let run = run_for(
        payload.clone(),
        world_digest(live.world()),
        SEED,
        COMMITMENTS,
        current_versions(COMMITMENTS),
    );
    let store = FileStore::new(scratch("commitment-roundtrip"));
    save_to(&store, "autosave", &run).expect("the save is written");
    let reloaded = load_from(&store, "autosave", &current_versions(COMMITMENTS))
        .expect("the save reloads")
        .snapshot
        .expect("a saved game carries a snapshot");
    assert_eq!(
        scenario_of(&reloaded.state).commitments,
        scenario_of(&payload).commitments,
        "every field a run writes — party, terms, condition, state and both tick \
         stamps — round-trips through RON"
    );

    // The compatibility half. Unlike every other slice, this one cannot be shown
    // by picking a world that authors no such block, because there IS no block:
    // the duel simply never reaches a beat where anyone gives their word, and
    // that is what an empty ledger means.
    let mut quiet = duel();
    step(&mut quiet, 120);
    let quiet_payload = capture(quiet.world());
    assert!(
        scenario_of(&quiet_payload).commitments.is_empty(),
        "a run that made no promises captures no commitment state"
    );
}

/// A save written before commitment state was recorded is refused on **format**.
///
/// The field carries `#[serde(default)]`, so the older payload still parses —
/// which is exactly why the constant had to move, and why the content digest
/// could not be left to do the job. A promise is a runtime artifact: no world
/// file declares one, so an older save of the *same* world file has the same
/// content digest and nothing else would refuse it. Restoring it resumes a
/// captain who has promised nothing, which is a plausible state rather than an
/// obviously missing one.
#[test]
fn a_save_written_before_commitment_state_is_refused_on_format() {
    let mut live = boot(&commitments_args());
    step(&mut live, COMMITMENTS_CAPTURE_AT);

    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let current = current_versions(COMMITMENTS);

    // Recorded under the PREVIOUS format, everything else untouched, so the only
    // reason to refuse is the one being tested.
    let previous = Versions::new(SNAPSHOT_FORMAT - 1, SIMULATION_RULES, current.content);
    let run = run_for(payload, digest, SEED, COMMITMENTS, previous);
    let store = FileStore::new(scratch("commitment-format"));
    save_to(&store, "autosave", &run).expect("the save is written");

    let refusal = load_from(&store, "autosave", &current).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Format { .. })),
        "the refusal names the dimension that moved: {refusal}"
    );
}

// ── Issue #1035: a settled strike stays settled across a resume ──────────────

/// The strike probe: two sides, two refusals, and a settlement at t=10 s.
const STRIKE: &str = "assets/worlds/probe_strike_min.toml";

/// Frames to run before the strike capture.
///
/// The probe settles `probe_workers` on a t=10 s handler, so this has to land
/// after it: a capture taken while the strike still held would round-trip
/// identically even if the restore dropped the register and let the world re-arm
/// itself from the file. The precondition below asserts the settlement rather
/// than trusting the number.
const STRIKE_CAPTURE_AT: u64 = 700;

fn strike_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: STRIKE.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn live_workforce(
    app: &bevy::prelude::App,
) -> &project_phoenix::world::workforce::WorkforceRegister {
    &app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .workforce
}

/// **Issue #1035.** A strike the crew settled before the save is still settled
/// after it — and the depot that was refusing transfers goes on taking them.
///
/// This is the sharpest thing a resume can get wrong about authored-then-moved
/// state, and it is sharp for the reason the destroy test below is: the fresh
/// app does not start from the save. It boots the same world file first, which
/// ARMS the register from `[[workforce]]` and puts `probe_workers` straight back
/// out on strike, and only then has the capture laid over it. So the resumed
/// world genuinely holds the wrong answer at the moment `restore` is called, and
/// something has to correct it. That something is the `armed` latch travelling
/// in the payload with the records.
///
/// The register is **not** folded into the simulation digest — it sits with
/// `FlagStore` and the deadline table on that side of the line — so digest
/// agreement here checks that the resume did not disturb the *simulation*, and
/// every claim about the dispute is asserted directly.
#[test]
fn a_settled_strike_stays_settled_across_a_resume() {
    let mut live = boot(&strike_args());
    step(&mut live, STRIKE_CAPTURE_AT);

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());
    let scenario = scenario_of(&payload);

    // The capture is genuinely past the settlement, and past the disposition
    // move that came with it. Both are what make the measurement discriminating.
    assert!(
        !scenario.workforce.on_strike("probe_workers"),
        "precondition: the capture is taken AFTER the strike was settled — a capture \
         taken while it held would round-trip identically even if restore dropped \
         the field entirely"
    );
    assert_eq!(
        scenario.workforce.disposition("probe_workers"),
        Some(70),
        "precondition: and after the settlement moved what they make of the crew"
    );
    assert!(
        scenario.workforce.armed,
        "precondition: the latch is in the payload — it is the field that stops the \
         resumed mission re-arming itself"
    );
    assert!(
        !scenario.workforce.on_strike("probe_operator"),
        "the side that never walked out is still at work"
    );

    let mut resumed = boot_to_restore_point(&strike_args(), &payload);

    // The bootstrap's own state, before the restore overwrites it. This is the
    // control: the fresh app has read the same world file and is either still
    // short of arming or holding the OPENING state it authors — never the
    // settled one. So nothing below can be satisfied by a bootstrap
    // coincidence.
    let bootstrap = live_workforce(&resumed).clone();
    assert_ne!(
        bootstrap.disposition("probe_workers"),
        Some(70),
        "precondition: the freshly booted world has not reached the settlement — it is \
         a fresh read of the same `[[workforce]]` table, not a resumed one"
    );
    assert!(
        !bootstrap.armed || bootstrap.on_strike("probe_workers"),
        "precondition: and once it arms, it arms the strike ON"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    assert!(
        !live_workforce(&resumed).on_strike("probe_workers"),
        "the restore takes the register whole: a negotiation the crew already won is \
         not un-won by reloading"
    );
    assert_eq!(
        live_workforce(&resumed).disposition("probe_workers"),
        Some(70),
        "…with the disposition it ended on, not the one the file authored"
    );
    assert_eq!(
        world_counter(&resumed, "workforce.probe_workers.disposition"),
        70,
        "and the mirror flag a script condition reads came back with it, through the \
         flag store that was already in the payload"
    );
    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "the resumed world stands exactly where the capture did"
    );

    // Both worlds step across the second delivery — the one the strike refused
    // the first time. It only lands if the resumed register is really settled.
    step(&mut live, 700);
    step(&mut resumed, 700);

    assert!(
        !live_workforce(&resumed).on_strike("probe_workers"),
        "the resumed mission's own ticks did not re-arm the register"
    );
    assert_eq!(
        world_counter(&resumed, "second_transfer_begun"),
        world_counter(&live, "second_transfer_begun"),
        "both worlds stood the refused delivery up again"
    );
    assert_eq!(
        world_digest(resumed.world()),
        world_digest(live.world()),
        "and it landed the same way on both sides of the resume — the depot's capacity \
         level IS folded, so a resumed strike that had come back on would show here"
    );
}

/// The register round-trips through the save's RON, and a world that declares no
/// dispute writes nothing workforce-shaped at all.
#[test]
fn the_workforce_register_round_trips_and_a_dispute_free_world_writes_none() {
    let mut live = boot(&strike_args());
    step(&mut live, STRIKE_CAPTURE_AT);
    let payload = capture(live.world());

    let run = run_for(
        payload.clone(),
        world_digest(live.world()),
        SEED,
        STRIKE,
        current_versions(STRIKE),
    );
    let store = FileStore::new(scratch("workforce-roundtrip"));
    save_to(&store, "autosave", &run).expect("the save is written");
    let reloaded = load_from(&store, "autosave", &current_versions(STRIKE))
        .expect("the save reloads")
        .snapshot
        .expect("a saved game carries a snapshot");
    assert_eq!(
        scenario_of(&reloaded.state).workforce,
        scenario_of(&payload).workforce,
        "every field a run moves — both sides' status, both dispositions and the armed \
         latch — round-trips through RON"
    );

    // The compatibility half: a world that authors no `[[workforce]]` captures
    // no workforce state, so its payload is byte-identical to one from before
    // this vocabulary existed.
    let mut quiet = duel();
    step(&mut quiet, 120);
    assert!(
        scenario_of(&capture(quiet.world())).workforce.is_empty(),
        "a world with no dispute captures no dispute"
    );
}

/// A save written before workforce state was recorded is refused on **format**.
///
/// The field carries `#[serde(default)]`, so the older payload still parses —
/// which is exactly why the constant had to move. A strike IS authored in the
/// world file, so it looks as though the content digest could stand in; it
/// cannot, because `RawWorld` sets no `deny_unknown_fields`. An older build
/// loads a world authoring `[[workforce]]`, drops the table, and writes a save
/// of the same files with the same content digest and no dispute in it.
///
/// The save this refuses is written at `SNAPSHOT_FORMAT - 1` rather than at a
/// literal, which since #1031 landed beneath this slice means a **format-7**
/// payload: one that carries the evidence log perfectly well and is silent
/// about the dispute. That is the sharp case rather than a weaker one — a
/// payload missing everything is obviously stale, and a payload missing exactly
/// the one table the world is about is not.
#[test]
fn a_save_written_before_workforce_state_is_refused_on_format() {
    let mut live = boot(&strike_args());
    step(&mut live, STRIKE_CAPTURE_AT);

    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let current = current_versions(STRIKE);

    let previous = Versions::new(SNAPSHOT_FORMAT - 1, SIMULATION_RULES, current.content);
    let run = run_for(payload, digest, SEED, STRIKE, previous);
    let store = FileStore::new(scratch("workforce-format"));
    save_to(&store, "autosave", &run).expect("the save is written");

    let refusal = load_from(&store, "autosave", &current).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Format { .. })),
        "the refusal names the dimension that moved: {refusal}"
    );
}

// ── Issue #1033: an entity destroyed before the save stays destroyed ─────────

/// The destroy probe: a skyhook collapsed by script, two storm bands retired.
const DESTROY: &str = "assets/worlds/probe_destroy_chain.toml";

/// Frames to run before the destroy capture.
///
/// The probe collapses the skyhook on a t=5 s deadline, so this has to land
/// after it — a capture taken before would round-trip a world where nothing had
/// been destroyed at all, which is the one payload that cannot fail this test.
/// The precondition below asserts the crossing rather than trusting the number.
const DESTROY_CAPTURE_AT: u64 = 420;

fn destroy_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: DESTROY.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// **Issue #1033.** An entity a script destroyed before the save does not come
/// back when the save is resumed.
///
/// This is the sharpest thing a resume can get wrong about a removal, and it is
/// sharp precisely because a fresh app does not start from the save — it boots
/// the same world file first, which SPAWNS the skyhook again, and then has the
/// capture laid over it. So the resumed world genuinely contains the destroyed
/// structure at the moment `restore` is called, and something has to take it
/// away again. That something is `restore_entities`' surplus sweep: it despawns
/// every uuid the bootstrap produced that the capture does not name.
///
/// The control below reads the freshly booted world BEFORE the restore, so a
/// restore that did nothing at all would be visible here rather than hidden
/// behind a world that happened to look right. Getting this wrong resurrects a
/// collapsed skyhook mid-mission, with the objective it failed still failed —
/// a world that contradicts its own scenario state.
#[test]
fn an_entity_destroyed_before_the_save_does_not_come_back_after_a_resume() {
    const SKYHOOK: &str = "world.probe_destroy.entity.skyhook.name";

    let mut live = boot(&destroy_args());
    step(&mut live, DESTROY_CAPTURE_AT);

    let skyhook_uuid = live
        .world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::entities::spawner::EntityUuid,
        )>()
        .iter(live.world())
        .find(|(name, _)| name.0 == SKYHOOK)
        .map(|(_, uuid)| uuid.0.clone());
    assert!(
        skyhook_uuid.is_none(),
        "precondition: the capture must be taken AFTER the scripted collapse — a \
         capture of an intact world is the one payload this test cannot fail"
    );

    let payload = capture(live.world());
    let captured_uuids: std::collections::BTreeSet<&String> =
        payload.entities.iter().map(|e| &e.uuid).collect();

    // A fresh app, booted from the same world file — which spawns the skyhook.
    let mut resumed = boot_to_restore_point(&destroy_args(), &payload);
    let before_restore = resumed
        .world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::entities::spawner::EntityUuid,
        )>()
        .iter(resumed.world())
        .find(|(name, _)| name.0 == SKYHOOK)
        .map(|(_, uuid)| uuid.0.clone());
    let bootstrapped = before_restore.expect(
        "control: the fresh world must spawn the skyhook from its `[[entity]]` \
         block, or this test proves nothing — the removal would be an artifact of \
         the bootstrap rather than of the restore",
    );
    assert!(
        !captured_uuids.contains(&bootstrapped),
        "control: and the capture must NOT name it, so the surplus sweep is the \
         only thing that can take it away"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(
        report.despawned >= 1,
        "the restore must report the surplus it removed, not silently leave it: \
         {report:?}"
    );

    let after = resumed
        .world_mut()
        .query::<&project_phoenix::entities::spawner::EntityName>()
        .iter(resumed.world())
        .any(|name| name.0 == SKYHOOK);
    assert!(
        !after,
        "a structure destroyed before the save must stay destroyed through the \
         resume — a resurrected skyhook would contradict the objective its \
         collapse already failed"
    );
}

// ── Gathered evidence across a resume (issue #1031) ──────────────────────────

/// The evidence probe: one finding from a survey deadline, a second from a
/// dialogue the ship's own Comms officer answers.
const EVIDENCE: &str = "assets/worlds/probe_evidence.toml";

/// Frames the evidence world runs before its capture.
///
/// It has to land in the window where the two findings DISAGREE — after the
/// survey deadline writes the scan entry at t=3 s and before the foreman is
/// pressed at t=5 s. A capture taken before the survey would round-trip an empty
/// log, which is the same payload a restore that dropped the field entirely
/// would also produce. The assertions below make that precondition explicit
/// rather than trusting the number.
const EVIDENCE_CAPTURE_AT: u64 = 240;

/// Frames both worlds are stepped after the restore — enough to carry them past
/// the dialogue open at t=5 s and the officer's answer a tick or two later.
const EVIDENCE_CONTINUE_FOR: u64 = 200;

fn evidence_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: EVIDENCE.into(),
        // The world's own player-ship hull, because the Comms response policy
        // that answers the foreman is authored as an override ON that entry.
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn live_evidence(app: &bevy::prelude::App) -> &project_phoenix::dossier::EvidenceLog {
    &app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .evidence
}

fn evidence_texts(app: &bevy::prelude::App) -> Vec<String> {
    live_evidence(app)
        .entries
        .iter()
        .map(|e| e.text.clone())
        .collect()
}

/// A finding survives a resume with its provenance and the tick it was learned
/// on, and the resumed run goes on to learn the second one exactly as the live
/// run does.
///
/// The log is **not** folded into the simulation digest — it sits with the
/// commitments ledger and the deadline table on that side of the line — so
/// digest agreement below checks that the resume did not disturb the
/// *simulation*, and every claim about what the crew know is asserted directly.
/// A test that only compared digests would pass with the log dropped on the
/// floor.
#[test]
fn a_gathered_finding_survives_a_resume_and_the_next_one_still_arrives() {
    use project_phoenix::dossier::EvidenceProvenance;

    let mut live = boot(&evidence_args());
    step(&mut live, EVIDENCE_CAPTURE_AT);

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());
    let scenario = scenario_of(&payload);

    // The capture is genuinely mid-flight: the survey is back, the foreman has
    // not talked yet. Those two states are what make the measurement
    // discriminating.
    assert_eq!(
        scenario
            .evidence
            .entries
            .iter()
            .map(|e| (e.text.as_str(), e.provenance))
            .collect::<Vec<_>>(),
        vec![(
            "world.probe_evidence.evidence.stress_fracture",
            EvidenceProvenance::Scan
        )],
        "precondition: the capture is taken AFTER the scan and BEFORE the \
         admission — a capture of an empty log would round-trip identically even \
         if restore dropped the field entirely"
    );
    let captured_scan = scenario.evidence.entries[0].clone();
    assert!(
        captured_scan.gathered_at_tick > 0,
        "the tick the crew learned it travels with the finding"
    );
    assert_eq!(
        world_counter(&live, "foreman_pressed"),
        0,
        "nothing has pressed the foreman before the capture"
    );

    let mut resumed = boot_to_restore_point(&evidence_args(), &payload);

    // The bootstrap's own state, before the restore overwrites it. A finding is
    // only ever written by a script call, so a fresh app short of the mission's
    // first tick has learned nothing — which is what stops any claim below being
    // satisfied by a bootstrap coincidence.
    assert!(
        live_evidence(&resumed).is_empty(),
        "precondition: the fresh app's crew have found nothing out"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    assert_eq!(
        live_evidence(&resumed).entries,
        vec![captured_scan.clone()],
        "the restore takes the finding whole — subject, text, provenance and the \
         tick it was gathered on"
    );
    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "the resumed world stands exactly where the capture did"
    );

    // Both worlds now step past the dialogue the officer answers.
    step(&mut live, EVIDENCE_CONTINUE_FOR);
    step(&mut resumed, EVIDENCE_CONTINUE_FOR);

    assert_eq!(
        evidence_texts(&live),
        vec![
            "world.probe_evidence.evidence.stress_fracture".to_string(),
            "world.probe_evidence.evidence.foreman_admission".to_string(),
        ],
        "precondition: the live world's foreman talked"
    );
    assert_eq!(
        live_evidence(&resumed).entries,
        live_evidence(&live).entries,
        "and the resumed world learned the same second thing, in the same order, \
         at the same tick — with the first finding still stamped when it was \
         actually made rather than re-stamped by the resume"
    );
    assert_eq!(
        live_evidence(&resumed).entries[0],
        captured_scan,
        "the scan entry is untouched by everything that happened after it: the \
         log is append-only"
    );
    assert_eq!(
        world_digest(resumed.world()),
        world_digest(live.world()),
        "and the two worlds are still standing in the same place afterwards"
    );
}

/// The log round-trips through the save's RON, and a run whose crew found
/// nothing out writes nothing evidence-shaped at all.
#[test]
fn the_evidence_log_round_trips_and_an_incurious_run_writes_none() {
    let mut live = boot(&evidence_args());
    step(&mut live, EVIDENCE_CAPTURE_AT);
    let payload = capture(live.world());

    let run = run_for(
        payload.clone(),
        world_digest(live.world()),
        SEED,
        EVIDENCE,
        current_versions(EVIDENCE),
    );
    let store = FileStore::new(scratch("evidence-roundtrip"));
    save_to(&store, "autosave", &run).expect("the save is written");
    let reloaded = load_from(&store, "autosave", &current_versions(EVIDENCE))
        .expect("the save reloads")
        .snapshot
        .expect("a saved game carries a snapshot");
    assert_eq!(
        scenario_of(&reloaded.state).evidence,
        scenario_of(&payload).evidence,
        "every field a finding carries — subject, text, provenance and tick — \
         round-trips through RON"
    );

    // The compatibility half, and like the ledger's it cannot be shown by
    // picking a world that authors no such block, because there IS no block: the
    // duel simply never reaches a beat where the crew learn anything, and that
    // is what an empty log means.
    let mut quiet = duel();
    step(&mut quiet, 120);
    let quiet_payload = capture(quiet.world());
    assert!(
        scenario_of(&quiet_payload).evidence.is_empty(),
        "a run whose crew found nothing out captures no evidence state"
    );
}

/// A save written before evidence state was recorded is refused on **format**.
///
/// The field carries `#[serde(default)]`, so the older payload still parses —
/// which is exactly why the constant had to move. A finding is a runtime
/// artifact: no world file declares one, so an older save of the *same* world
/// file has the same content digest and nothing else would refuse it. Restoring
/// it hands the crew back a blank intelligence file, which is a plausible state
/// rather than an obviously missing one — and a later re-scan would re-stamp the
/// finding at the resumed tick, rewriting when they found out.
#[test]
fn a_save_written_before_evidence_state_is_refused_on_format() {
    let mut live = boot(&evidence_args());
    step(&mut live, EVIDENCE_CAPTURE_AT);

    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let current = current_versions(EVIDENCE);

    // Recorded under the PREVIOUS format, everything else untouched, so the only
    // reason to refuse is the one being tested.
    let previous = Versions::new(SNAPSHOT_FORMAT - 1, SIMULATION_RULES, current.content);
    let run = run_for(payload, digest, SEED, EVIDENCE, previous);
    let store = FileStore::new(scratch("evidence-format"));
    save_to(&store, "autosave", &run).expect("the save is written");

    let refusal = load_from(&store, "autosave", &current).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Format { .. })),
        "the refusal names the dimension that moved: {refusal}"
    );
}

// ── Issue #1032: a sensor reading survives a resume ──────────────────────────

/// The scan probe: one destroyer, one structure failing under its own authored
/// decay, and two contacts it must refuse.
const SCAN: &str = "assets/worlds/probe_scan.toml";

/// Frames to run before the scan capture — far enough in that the game is
/// InProgress and the ship's stations are backfilled.
const SCAN_CAPTURE_AT: u64 = 90;

fn scan_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: SCAN.into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        max_ticks: 600,
        deterministic: true,
        seed: Some(1032),
        ..Default::default()
    }
}

/// Fly the scanner to 80 units off the depot and take a reading through the
/// ordinary admitted path, exactly as `tests/headless_runner.rs` does.
fn take_a_reading(app: &mut bevy::prelude::App) {
    use project_phoenix::core::messages::{ClientMessage, SystemControlPayload};
    use project_phoenix::entities::spawner::{EntityName, EntityUuid};
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::server_app::LocalShip;
    use project_phoenix::ship::state::ShipPhysics;

    let depot = {
        let mut q = app.world_mut().query::<(&EntityName, &EntityUuid)>();
        let found = q
            .iter(app.world())
            .find(|(n, _)| n.0 == "world.probe_scan.entity.skyway_depot.name")
            .map(|(_, uuid)| uuid.0.clone());
        found.expect("the probe world spawns the depot")
    };
    let ship = {
        let mut q = app
            .world_mut()
            .query_filtered::<bevy::prelude::Entity, bevy::prelude::With<LocalShip>>();
        let found = q.iter(app.world()).next();
        found.expect("the probe world spawns a local ship")
    };
    app.world_mut()
        .get_mut::<ShipPhysics>(ship)
        .expect("the local ship is a ship")
        .x = 520.0;
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: "ai:scan-resume-probe".into(),
            msg: ClientMessage::ControlSystem {
                target: project_phoenix::ship::system_registry::sensors_system_id(),
                payload: SystemControlPayload::ScanTarget { uuid: depot },
            },
        });
    step(app, 2);
}

/// **Issue #1032.** A mission saved after the crew surveyed a structure comes
/// back with the survey.
///
/// This is the one piece of scan state a fold cannot recover. Everything else —
/// the depot's condition, the ship's range, the grid's level — is re-derivable
/// from the resumed world, but the READING is what the crew saw when they
/// looked, at the fidelity that moment bought them, and the structure has gone
/// on failing since. A resumed crew handed a blank readout would have to
/// re-survey a thing they had already surveyed, and would get a different
/// answer, which is the whole reason the reading is stored rather than folded.
///
/// The control below reads the freshly booted ship first, so an inert restore is
/// visible rather than hidden behind a value that happened to match.
#[test]
fn the_resumed_world_keeps_the_reading_the_crew_took() {
    let mut live = boot(&scan_args());
    step(&mut live, SCAN_CAPTURE_AT);
    take_a_reading(&mut live);

    let payload = capture(live.world());
    let captured: Vec<_> = payload
        .entities
        .iter()
        .filter_map(|e| e.scan.as_ref().map(|s| (&e.uuid, s)))
        .collect();
    assert_eq!(
        captured.len(),
        1,
        "the probe world carries exactly one ship with a survey suite"
    );
    let (uuid, record) = captured[0];
    let reading = record
        .last
        .as_ref()
        .expect("precondition: the capture must be taken after a scan came back");
    assert_eq!(
        reading.band, "detailed",
        "precondition: and from inside the destroyer's finest authored band, so the \
         restored value is a specific one rather than whatever a default would give"
    );
    assert!(
        reading.condition_fraction > 0.0,
        "precondition: it read a real, non-zero condition off the depot's track"
    );
    assert!(record.refusal.is_none());

    let mut resumed = boot_to_restore_point(&scan_args(), &payload);
    let before_restore = resumed
        .world_mut()
        .query::<&project_phoenix::science::ShipScanRecord>()
        .iter(resumed.world())
        .next()
        .cloned()
        .expect("the fresh world spawned the destroyer from its template");
    assert!(
        before_restore.last.is_none(),
        "control: a freshly booted crew have surveyed nothing, so an inert restore \
         would be visible here rather than hidden behind a value that matched"
    );
    assert_eq!(
        before_restore.config.bands.len(),
        2,
        "…and it carries its authored fidelity ladder, which the save deliberately \
         does NOT: that is content, re-derived from the template on spawn"
    );

    restore(resumed.world_mut(), &payload);
    let after = capture(resumed.world());
    let restored = after
        .entities
        .iter()
        .find(|e| &e.uuid == uuid)
        .and_then(|e| e.scan.as_ref())
        .unwrap_or_else(|| panic!("ship {uuid} came back without its scan record"));
    assert_eq!(
        restored, record,
        "ship {uuid}: the subject, the band, the tick it was taken on, the quantised \
         condition and every labelled row must all come back exactly as captured"
    );

    let live_ladder = resumed
        .world_mut()
        .query::<&project_phoenix::science::ShipScanRecord>()
        .iter(resumed.world())
        .next()
        .map(|record| record.config.clone())
        .expect("the record is still attached");
    assert_eq!(
        live_ladder, before_restore.config,
        "and the restore left the spawned ladder alone — it restores the mutable half \
         of the record, not the content half"
    );
}

/// **Issue #1032.** A world whose hulls carry no survey suite writes nothing
/// scan-shaped into its save.
///
/// A duel of two cruisers, and a cruiser authors no `[scan]` — the destroyer is
/// the only hull in the repository that does. Every world whose hulls are not
/// destroyers is in this arm, and none of them should pay a byte for a feature
/// they do not use.
#[test]
fn a_world_with_no_survey_suite_writes_no_scan_state() {
    let mut live = boot(&args(DUEL, ("cruiser", "cruiser")));
    step(&mut live, 120);
    let payload = capture(live.world());
    assert!(
        payload.entities.iter().all(|e| e.scan.is_none()),
        "a hull with no [scan] table carries no scan record and writes no scan state"
    );
}

// ── Issue #1041: an order to hold fire survives the save ─────────────────────

const RESTRAINT: &str = "assets/worlds/probe_restraint.toml";

/// Frames to run before the restraint capture.
///
/// `probe_restraint.toml` orders its picket to hold at t=4 s and releases it at
/// t=8 s, so a capture has to land inside that window — 360 frames is t=6 s,
/// squarely between the two. The precondition below asserts the hold rather than
/// trusting the number.
const RESTRAINT_CAPTURE_AT: u64 = 360;

fn restraint_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: RESTRAINT.into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// The saved weapons-hold state of every ship in a payload, keyed by uuid.
fn held_ships(payload: &PhoenixSnapshot) -> Vec<(String, bool)> {
    let mut rows: Vec<(String, bool)> = payload
        .entities
        .iter()
        .filter_map(|e| e.weapons_hold.map(|held| (e.uuid.clone(), held)))
        .collect();
    rows.sort();
    rows
}

/// **Issue #1041.** A ship its captain ordered to hold fire comes back holding
/// it.
///
/// The sharp case, and it is sharp for the reason the strike test above is: the
/// fresh app does not start from the save. It boots the same world file, which
/// spawns every hull weapons-free, and only then has the capture laid over it —
/// so the resumed world genuinely holds the wrong answer at the moment `restore`
/// is called. Half a firing posture is not a posture: a save that remembered the
/// alert and forgot the hold would hand the crew back a ship with live guns on
/// the tick a scenario is weighing what they chose.
#[test]
fn an_order_to_hold_fire_survives_a_resume() {
    let mut live = boot(&restraint_args());
    step(&mut live, RESTRAINT_CAPTURE_AT);

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());

    let held = held_ships(&payload);
    assert!(
        held.iter().any(|(_, h)| *h),
        "precondition: the capture is taken INSIDE the hold window — a capture with \
         nothing held would round-trip identically even if restore dropped the field"
    );
    assert!(
        held.iter().any(|(_, h)| !*h),
        "precondition: and something is weapons-free at the same instant, so what \
         round-trips below is a per-ship answer rather than a constant"
    );

    let mut resumed = boot_to_restore_point(&restraint_args(), &payload);
    assert!(
        held_ships(&capture(resumed.world()))
            .iter()
            .all(|(_, h)| !*h),
        "precondition: the freshly booted world has every hull weapons-free — it is a \
         fresh read of the same file, not a resumed one, so nothing below can be \
         satisfied by a bootstrap coincidence"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    assert_eq!(
        held_ships(&capture(resumed.world())),
        held,
        "every ship's posture comes back exactly as it was ordered — the held hull \
         held, the free ones free"
    );
    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "the resumed world stands exactly where the capture did, weapons-hold \
         namespace included — a resume that dropped the order would fold a different \
         number here, because a held ship IS in that namespace"
    );
}

/// A save written before the weapons hold was recorded is refused on **format**.
///
/// The field carries `#[serde(default)]`, so an older payload still parses —
/// which is exactly why the constant had to move. Nothing in a format-8 payload
/// distinguishes a run whose captain had called a hold from one whose captain
/// had not, and the two are different worlds.
#[test]
fn a_save_written_before_the_weapons_hold_is_refused_on_format() {
    let mut live = boot(&restraint_args());
    step(&mut live, RESTRAINT_CAPTURE_AT);

    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let current = current_versions(RESTRAINT);

    let previous = Versions::new(SNAPSHOT_FORMAT - 1, SIMULATION_RULES, current.content);
    let run = run_for(payload, digest, SEED, RESTRAINT, previous);
    let store = FileStore::new(scratch("weapons-hold-format"));
    save_to(&store, "autosave", &run).expect("the save is written");

    let refusal = load_from(&store, "autosave", &current).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Format { .. })),
        "the refusal names the dimension that moved: {refusal}"
    );
}

// ── Issues #1107–#1109: a Command stance survives the save ───────────────────

/// The saved Command stance selections in a payload, flattened to
/// `(uuid, station id, stance id)` and sorted.
fn directed_ships(payload: &PhoenixSnapshot) -> Vec<(String, String, String)> {
    let mut rows: Vec<(String, String, String)> = payload
        .entities
        .iter()
        .flat_map(|e| {
            e.station_stances
                .iter()
                .map(move |(station, stance)| (e.uuid.clone(), station.clone(), stance.clone()))
        })
        .collect();
    rows.sort();
    rows
}

/// **Issues #1107–#1109.** A ship's in-force Command stance comes back exactly
/// as it stood.
///
/// The restraint world's destroyer runs its Command seat through ordinary AI,
/// which at red alert directs the weapons Station onto the authored `ai_engaged`
/// stance — an entry in `ShipStationStances`, which `world_digest` folds. The
/// capture is taken with that stance in force; the fresh app boots the same file
/// and reaches its restore point long before the destroyer raises red alert, so
/// it holds NO stance at the moment `restore` is called. A resume that dropped
/// the map would fold a different number on the tick the digest counts it — the
/// bug this issue fixes.
#[test]
fn a_command_stance_survives_a_resume() {
    let mut live = boot(&restraint_args());
    step(&mut live, RESTRAINT_CAPTURE_AT);

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());

    let directed = directed_ships(&payload);
    assert!(
        !directed.is_empty(),
        "precondition: the capture is taken with a Command stance in force — the \
         destroyer's AI Command seat picks its engaged stance at red alert — so a \
         resume that dropped the map would fold a different number below"
    );

    let mut resumed = boot_to_restore_point(&restraint_args(), &payload);
    assert!(
        directed_ships(&capture(resumed.world())).is_empty(),
        "precondition: the freshly booted world holds no stance — it reaches the \
         restore point before the destroyer raises red alert, so nothing below can be \
         satisfied by a bootstrap coincidence"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);

    assert_eq!(
        directed_ships(&capture(resumed.world())),
        directed,
        "every ship's Command stance comes back exactly as it was directed"
    );
    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "the resumed world stands exactly where the capture did, station-stances \
         namespace included — a resume that dropped the map would fold a different \
         number here, because a directed ship IS in that namespace"
    );
}

/// A save written before Command stances were recorded is refused on **format**.
///
/// The field carries `#[serde(default)]`, so an older payload still parses —
/// which is exactly why the constant had to move. Nothing in a format-10 payload
/// distinguishes a run whose crew had directed a Station from one that had not,
/// and the two fold to different digests.
#[test]
fn a_save_written_before_the_station_stances_is_refused_on_format() {
    let mut live = boot(&restraint_args());
    step(&mut live, RESTRAINT_CAPTURE_AT);

    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let current = current_versions(RESTRAINT);

    let previous = Versions::new(SNAPSHOT_FORMAT - 1, SIMULATION_RULES, current.content);
    let run = run_for(payload, digest, SEED, RESTRAINT, previous);
    let store = FileStore::new(scratch("station-stances-format"));
    save_to(&store, "autosave", &run).expect("the save is written");

    let refusal = load_from(&store, "autosave", &current).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Format { .. })),
        "the refusal names the dimension that moved: {refusal}"
    );
}

// ── Dynamic combat consequences across a resume (issue #863) ─────────────────

/// The reinforcement probe: an authored escort, and two Harrow raiders a script
/// spawns at t=3 s.
const REINFORCE: &str = "assets/worlds/probe_reinforce.toml";

/// Frames the reinforcement world runs before its capture.
///
/// It has to land AFTER the t=3 s spawn, because a capture taken before it names
/// only authored ships — which is the one payload this file's claim cannot fail,
/// since every row would find a bootstrapped hull waiting. The preconditions
/// below assert the crossing rather than trusting the number.
const REINFORCE_CAPTURE_AT: u64 = 360;

fn reinforce_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: REINFORCE.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// Every `EntityUuid` in a world, as a set.
fn uuids(app: &mut bevy::prelude::App) -> std::collections::BTreeSet<String> {
    use project_phoenix::entities::spawner::EntityUuid;
    let mut query = app.world_mut().query::<&EntityUuid>();
    query.iter(app.world()).map(|u| u.0.clone()).collect()
}

/// Every live `AsteroidUuid`, as a set.
fn rock_uuids(app: &mut bevy::prelude::App) -> std::collections::BTreeSet<String> {
    use project_phoenix::asteroids::lifecycle::AsteroidUuid;
    let mut query = app.world_mut().query::<&AsteroidUuid>();
    query.iter(app.world()).map(|u| u.0.clone()).collect()
}

/// The live `name_to_uuid` map a scenario resolves its entity names through.
fn live_name_to_uuid(app: &bevy::prelude::App) -> std::collections::BTreeMap<String, String> {
    app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .name_to_uuid
        .iter()
        .map(|(name, uuid)| (name.clone(), uuid.clone()))
        .collect()
}

/// Bring a fresh app up to the point the **browser's deadline branch** restores
/// at (issue #863): the authored roster is standing, and whatever the capture
/// still names is something only the payload can build.
///
/// The sibling of [`boot_to_restore_point`], and the difference between them is
/// the whole of #863's browser half. That one waits for `ready_to_restore` —
/// every captured ship standing — which is the right thing to want and the wrong
/// thing to wait for forever. This one stops at `ready_to_rebuild`, which is what
/// `drain_snapshot_restore` asks once its patience budget is spent.
///
/// Deliberately does NOT keep stepping afterwards. A deterministic headless app
/// replaying the same scenario would eventually reach the t=3 s spawn by itself
/// and hand the test a bootstrapped roster — which is the coincidence the
/// assertions here exist to rule out.
fn boot_to_rebuild_point(args: &HeadlessArgs, snapshot: &PhoenixSnapshot) -> bevy::prelude::App {
    let mut app = boot(args);
    for _ in 0..1_000 {
        app.update();
        match reconcile_world_layers(app.world_mut(), snapshot) {
            LayerReconcileStatus::Ready if ready_to_rebuild(app.world(), snapshot) => return app,
            LayerReconcileStatus::Failed(path) => {
                panic!("world-layer reconciliation failed at {path}")
            }
            LayerReconcileStatus::Ready | LayerReconcileStatus::Waiting => {}
        }
    }
    panic!("the fresh app never reached the rebuild point");
}

/// Teleport the viewscreen ship, which is what the asteroid streamer's window
/// follows. Written through `ShipPhysics` and `Transform` together for
/// `restore_entities`' reason: the streamer reads the former, everything visual
/// reads the latter, and a ship that moved only one of them is in two places.
fn fly_local_ship_to(app: &mut bevy::prelude::App, x: f32, z: f32) {
    use project_phoenix::server_app::LocalShip;
    use project_phoenix::ship::state::ShipPhysics;
    let mut query = app
        .world_mut()
        .query_filtered::<bevy::prelude::Entity, bevy::prelude::With<LocalShip>>();
    let entities: Vec<bevy::prelude::Entity> = query.iter(app.world()).collect();
    for entity in entities {
        let mut entity_mut = app.world_mut().entity_mut(entity);
        if let Some(mut physics) = entity_mut.get_mut::<ShipPhysics>() {
            physics.x = x;
            physics.z = z;
        }
        if let Some(mut transform) = entity_mut.get_mut::<bevy::prelude::Transform>() {
            transform.translation.x = x;
            transform.translation.z = z;
        }
    }
}

/// **Issue #863, the roster half.** A ship a script spawned mid-run comes back
/// because the SAVE carries it, not because the fresh app happened to replay the
/// run that produced it.
///
/// The control is the whole test. `probe_reinforce` spawns its raiders at t=3 s,
/// and a fresh app reaches its restore point in a fraction of that — so the
/// world the restore is handed genuinely does not contain them, and the
/// assertions below are about what `restore` built rather than about what a
/// bootstrap coincidentally re-derived. Before this issue that state was not
/// reachable at all: `ready_to_restore` waited for every captured uuid, so the
/// fresh app sat there re-simulating until it re-minted the same ids — a resume
/// that was quietly a replay, and one a browser session booting with nobody at
/// the consoles never completes.
///
/// The two raiders are the same `alliance_cruiser` template the escort flies and
/// are made Harrow by an `overrides` block alone, so the faction and tag
/// assertions are what separate "two ships came back" from "the RIGHT two ships
/// came back". Neither travels anywhere else in the payload: a rebuild from the
/// bare template would put two friendly cruisers in the raiders' positions and
/// satisfy a count.
#[test]
fn a_mid_run_spawn_is_rebuilt_by_the_restore_rather_than_replayed_by_the_bootstrap() {
    use project_phoenix::entities::spawner::{
        EntitySpawnOrigin, EntityTagsSection, EntityUuid, FactionComponent,
    };

    const HARROW: &str = "cccccccc-3333-4333-8333-cccccccccccc";

    let mut live = boot(&reinforce_args());
    step(&mut live, REINFORCE_CAPTURE_AT);

    let raiders: std::collections::BTreeMap<String, String> = live_name_to_uuid(&live)
        .into_iter()
        .filter(|(name, _)| name.starts_with("probe_raider_"))
        .collect();
    assert_eq!(
        raiders.len(),
        2,
        "precondition: the capture must be taken AFTER the t=3 s reinforcement — \
         a capture of the authored roster alone is the one payload this test \
         cannot fail"
    );
    let escort = live_name_to_uuid(&live)
        .get("world.probe_reinforce.entity.escort.name")
        .expect("the escort is authored and named")
        .clone();

    let payload = capture(live.world());
    let captured_digest = world_digest(live.world());

    // The payload's own half of the precondition: exactly the two raiders carry
    // a spawn origin, and the authored ships carry none. That asymmetry is what
    // `restore` and `ready_to_restore` both read.
    let with_origin: std::collections::BTreeSet<&String> = payload
        .entities
        .iter()
        .filter(|row| row.spawn.is_some())
        .map(|row| &row.uuid)
        .collect();
    assert_eq!(
        with_origin,
        raiders.values().collect(),
        "only the scripted spawns should carry an origin; the player ship and \
         the authored escort are what any fresh boot puts back by itself"
    );

    // Through the real storage path, not straight from the capture. The origin
    // carries the instance overrides as a dynamic `toml::Value`, and RON is what
    // a save is actually written in — so the round-trip below is where that
    // document is proved to survive being a save rather than merely a struct.
    let run = run_for(
        payload.clone(),
        captured_digest,
        SEED,
        REINFORCE,
        current_versions(REINFORCE),
    );
    let store = FileStore::new(scratch("reinforce"));
    save_to(&store, "autosave", &run).expect("the save is written");
    let reloaded =
        load_from(&store, "autosave", &current_versions(REINFORCE)).expect("the save reloads");
    assert_eq!(reloaded, run, "the artifact round-trips through RON");
    let payload = reloaded
        .snapshot
        .as_ref()
        .expect("a saved game carries a snapshot")
        .state
        .clone();

    let mut resumed = boot_to_rebuild_point(&reinforce_args(), &payload);
    let before = uuids(&mut resumed);
    for uuid in raiders.values() {
        assert!(
            !before.contains(uuid),
            "control: the freshly booted world must NOT have reached its own t=3 s \
             reinforcement, or the restore is not what puts the raiders back"
        );
    }
    assert!(
        before.contains(&escort),
        "control: the AUTHORED escort must be standing already, so the claim \
         below is about mid-run spawns and not about restores in general"
    );
    assert!(
        !ready_to_restore(resumed.world(), &payload),
        "control: and the world is genuinely NOT ready by the waiting predicate — \
         this is the deadline path the browser takes, not the ordinary one"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        report.entities_spawned, 2,
        "the restore reports the two hulls it had to build: {report:?}"
    );
    assert_eq!(
        report.entities_restored,
        payload.entities.len(),
        "and every captured row found a home, built or bootstrapped"
    );

    // The ships themselves — present, at the captured identity, and made of what
    // the overrides said rather than of the bare template.
    for (name, uuid) in &raiders {
        let mut query = resumed.world_mut().query::<(
            &EntityUuid,
            Option<&FactionComponent>,
            Option<&EntityTagsSection>,
            Option<&EntitySpawnOrigin>,
        )>();
        let (_, faction, tags, origin) = query
            .iter(resumed.world())
            .find(|(u, ..)| &u.0 == uuid)
            .unwrap_or_else(|| panic!("{name} must be back in the resumed world"));
        assert_eq!(
            faction.map(|f| f.0.to_string()).as_deref(),
            Some(HARROW),
            "{name} came back Alliance — the override that makes a raider of the \
             cruiser template was not merged, so the resumed world is a different \
             fight from the captured one"
        );
        assert!(
            tags.is_some_and(|t| t.0.iter().any(|tag| tag == "probe_raider")),
            "{name} lost the tags its override wrote; neither tags nor faction \
             travel anywhere else in the payload, which is why they are read here"
        );
        assert!(
            origin.is_some(),
            "{name} must carry its origin again, or this resumed run could be \
             saved once and never resumed a second time"
        );
    }

    // And the scenario can still say which ship it means.
    let resumed_names = live_name_to_uuid(&resumed);
    for (name, uuid) in &raiders {
        assert_eq!(
            resumed_names.get(name),
            Some(uuid),
            "the resumed scenario must resolve `{name}` to the ship the capture \
             named, or every `on_destroyed`/`destroy_entity` that mentions it \
             addresses nobody"
        );
    }
    assert_eq!(
        resumed
            .world()
            .resource::<project_phoenix::world::server::WorldContentRuntime>()
            .entity_groups
            .get("raiders")
            .map(|members| members.len()),
        Some(2),
        "and the group an `on_all_destroyed` is judged against comes back with \
         both of them in it"
    );

    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "a world whose raiders the restore BUILT stands exactly where the capture \
         did — the strong form of the claim, since a built hull is folded into \
         the entity namespace alongside the bootstrapped ones"
    );
}

/// **Issue #863.** The two predicates say different things, and only the second
/// one lets a mid-run spawn through.
///
/// The pair asserted directly rather than through a resume, because the
/// difference between them is a design decision rather than an implementation
/// detail: `ready_to_restore` still waits for every captured ship — a
/// bootstrapped hull is a better hull than a built one — and `ready_to_rebuild`
/// is what the browser's deadline asks instead of giving up. A row with no origin
/// fails both, which is the silent-write failure the waiting predicate has always
/// existed to prevent, untouched.
#[test]
fn only_the_rebuild_predicate_lets_a_mid_run_spawn_through() {
    let mut live = boot(&reinforce_args());
    step(&mut live, REINFORCE_CAPTURE_AT);
    let payload = capture(live.world());

    // A fresh app at the deadline point: the authored roster is standing, the
    // t=3 s raiders are not.
    let mut fresh = boot_to_rebuild_point(&reinforce_args(), &payload);
    let standing = uuids(&mut fresh);
    assert!(
        payload
            .entities
            .iter()
            .any(|row| row.spawn.is_some() && !standing.contains(&row.uuid)),
        "control: a captured row is genuinely absent, which is the only \
         interesting case"
    );
    assert!(
        !ready_to_restore(fresh.world(), &payload),
        "the waiting predicate is still false — it wants the ship itself, not a \
         recipe for one"
    );
    assert!(
        ready_to_rebuild(fresh.world(), &payload),
        "and the rebuild predicate is true, because what is missing is exactly \
         what the payload can build"
    );

    // The same payload with the origins stripped: the raiders are now
    // indistinguishable from authored ships this world simply has not spawned,
    // and neither predicate will have them.
    let mut without_origins = payload.clone();
    for row in &mut without_origins.entities {
        row.spawn = None;
    }
    assert!(
        !ready_to_rebuild(fresh.world(), &without_origins),
        "a captured row that names no live entity and carries no origin is a save \
         this world cannot honour — the resume is abandoned loudly rather than \
         restored over a roster that is short"
    );
}

/// **Issue #863, the streamed-belt half.** A rock shot out of a streamed cell
/// stays shot after the resume — and its cell refills on re-entry exactly as the
/// live simulation refills it.
///
/// Two claims in one run, because they are two halves of one policy (see
/// `snapshot`'s module docs). Destruction inside a streamed cell is recorded by
/// ABSENCE — the rock is not in `PhoenixSnapshot::asteroids` and its window slot
/// is empty — so the first half is that a fresh app which streamed the rock in
/// alive has it taken away again. The second is that this does NOT make the
/// destruction permanent: leaving the cell and returning respawns the rock whole,
/// which is AGENTS.md's Key Constraint 8, and it is asserted on the LIVE world
/// and the RESUMED one together because the point is that a resume gets no
/// second opinion about it.
///
/// The rock's identity is what makes both halves readable: a streamed rock's
/// uuid is a pure function of its lattice cell, so "the same rock" after a
/// re-stream is a claim this test can make by string equality rather than by
/// counting.
#[test]
fn a_destroyed_streamed_rock_stays_destroyed_and_its_cell_refills_on_re_entry() {
    use project_phoenix::asteroids::lifecycle::AsteroidUuid;
    use project_phoenix::entities::spawner::EntitySystemHull;
    use project_phoenix::server_app::LocalShip;
    use project_phoenix::ship::state::ShipPhysics;

    let args = combat_test_args();
    let mut live = boot(&args);
    step(&mut live, CAPTURE_AT);

    // The ship's own position, and the rock nearest it — near enough that a
    // window rebuild re-evaluates its cell, which is what the re-entry leg needs.
    let ship = live
        .world_mut()
        .query_filtered::<&ShipPhysics, bevy::prelude::With<LocalShip>>()
        .iter(live.world())
        .next()
        .copied()
        .expect("combat_test has a viewscreen ship");
    let mut rocks: Vec<(bevy::prelude::Entity, String, f32)> = live
        .world_mut()
        .query::<(
            bevy::prelude::Entity,
            &AsteroidUuid,
            &bevy::prelude::Transform,
        )>()
        .iter(live.world())
        .map(|(e, uuid, t)| {
            let dx = t.translation.x - ship.x;
            let dz = t.translation.z - ship.z;
            (e, uuid.0.clone(), dx * dx + dz * dz)
        })
        .collect();
    assert!(
        !rocks.is_empty(),
        "precondition: combat_test's belts must have streamed by tick {CAPTURE_AT}"
    );
    rocks.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let (victim_entity, victim, _) = rocks[0].clone();

    // Shoot it — the streamer's own `check_destroyed_asteroids` does the rest:
    // the entity despawns and the cell's window slot is cleared.
    {
        let mut entity = live.world_mut().entity_mut(victim_entity);
        let mut hull = entity
            .get_mut::<EntitySystemHull>()
            .expect("a gameplay rock carries a hull");
        let ids: Vec<_> = hull.0.entries().map(|(id, _, _)| id.clone()).collect();
        for id in ids {
            hull.0.set_hp(&id, 0.0);
        }
    }
    step(&mut live, 2);
    assert!(
        !rock_uuids(&mut live).contains(&victim),
        "precondition: the rock must actually be destroyed in the live world"
    );

    let payload = capture(live.world());
    assert!(
        !payload.asteroids.iter().any(|a| a.uuid == victim),
        "and the payload records the destruction the only way a streamed field \
         can — by not naming the rock"
    );

    let mut resumed = boot_to_restore_point(&args, &payload);
    assert!(
        rock_uuids(&mut resumed).contains(&victim),
        "control: the fresh app streams the same cell and spawns the same rock at \
         the same cell-derived uuid, so something has to take it away again"
    );

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert!(
        !rock_uuids(&mut resumed).contains(&victim),
        "a rock destroyed before the save must stay destroyed through the resume"
    );

    // …and the declared policy: leave the cell, come back, and the field is
    // whole again. Asserted on both worlds, because a resume that answered this
    // differently from the live run would be a resumed world that plays
    // differently from the one it resumed.
    for (label, app) in [("live", &mut live), ("resumed", &mut resumed)] {
        assert!(
            !rock_uuids(app).contains(&victim),
            "[{label}] precondition for the re-entry leg: the rock is gone"
        );
        fly_local_ship_to(app, ship.x + 40_000.0, ship.z + 40_000.0);
        step(app, 4);
        fly_local_ship_to(app, ship.x, ship.z);
        step(app, 4);
        assert!(
            rock_uuids(app).contains(&victim),
            "[{label}] leaving the cell and returning must respawn the rock whole, \
             at the same cell-derived uuid — AGENTS.md Key Constraint 8, which a \
             resume does not get a second opinion about"
        );
    }
}

/// A save written before mid-run spawns were recorded is refused on **format**.
///
/// Both new fields carry `#[serde(default)]`, so a format-9 payload still parses
/// — which is exactly why the constant had to move. Nothing in one distinguishes
/// a run that never spawned anything from a run with a whole raid on the board,
/// and restoring the second resumes a fight two ships short with no error
/// anywhere.
#[test]
fn a_save_written_before_the_spawn_record_is_refused_on_format() {
    let mut live = boot(&reinforce_args());
    step(&mut live, REINFORCE_CAPTURE_AT);

    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let current = current_versions(REINFORCE);

    let previous = Versions::new(SNAPSHOT_FORMAT - 1, SIMULATION_RULES, current.content);
    let run = run_for(payload, digest, SEED, REINFORCE, previous);
    let store = FileStore::new(scratch("reinforce-format"));
    save_to(&store, "autosave", &run).expect("the save is written");

    let refusal = load_from(&store, "autosave", &current).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Format { .. })),
        "the refusal names the dimension that moved: {refusal}"
    );
}

// ── Portable saves: export, import, and the two refusals (issue #866) ────────

/// The exported artifact is **the same string** a browser slot holds.
///
/// Not "equivalent", not "round-trips to an equal value" — the same bytes. That
/// is the whole of this issue's no-second-schema claim, and it is checkable
/// because both paths are `save_to` over a `vellum_save::Store`: one writes into
/// a file, one into `localStorage`, one into memory on its way to a download,
/// and the record they carry is built once.
///
/// `FileStore` stands in for the browser here for the reason the rest of this
/// file uses it: `LocalStorage` needs a browser and `backend-fs` is the same
/// trait over the same text.
#[test]
fn an_exported_artifact_is_byte_identical_to_what_a_slot_holds() {
    let mut live = boot(&reinforce_args());
    step(&mut live, REINFORCE_CAPTURE_AT);

    let run = run_for(
        capture(live.world()),
        world_digest(live.world()),
        SEED,
        REINFORCE,
        current_versions(REINFORCE),
    );

    // One `scratch()` call, bound: the helper clears the directory each time it
    // is asked for one, so asking twice would delete the slot between writing
    // and reading it.
    let dir = scratch("export-identity");
    let store = FileStore::new(dir.clone());
    save_to(&store, "autosave", &run).expect("the save is written");
    let on_disk = std::fs::read_to_string(dir.join("autosave.ron"))
        .expect("the backend wrote the slot as one RON file");

    assert_eq!(
        project_phoenix::snapshot::export_artifact(&run).expect("the export is written"),
        on_disk,
        "an exported save and a stored slot are the same record in the same \
         encoding — if these ever differ, a second snapshot schema has been \
         introduced somewhere between them"
    );
}

/// And it is RON *text*, deliberately — the property the issue asks to lean on
/// rather than paper over.
///
/// Asserted at the shallowest level that means anything: the artifact is
/// human-readable, and the two facts a bug report is asked for first — which
/// world, and which tick — are legible in it without a parser.
#[test]
fn an_exported_artifact_is_text_a_human_can_read() {
    let mut live = boot(&reinforce_args());
    step(&mut live, REINFORCE_CAPTURE_AT);
    let tick = live
        .world()
        .resource::<project_phoenix::sim_tick::SimTick>()
        .0;
    let run = run_for(
        capture(live.world()),
        world_digest(live.world()),
        SEED,
        REINFORCE,
        current_versions(REINFORCE),
    );

    let text = project_phoenix::snapshot::export_artifact(&run).expect("the export is written");
    assert!(
        text.contains(REINFORCE),
        "the scenario it was taken in is readable in the file itself"
    );
    assert!(
        text.contains(&tick.to_string()),
        "and so is the tick it was taken at"
    );
}

/// A save exported from one session imports into a **fresh app** and restores.
///
/// The acceptance criterion end to end, and it goes through the transport rather
/// than around it: the payload that reaches `restore` here was serialised, handed
/// out as a file's worth of text, and parsed back by the import gate — not
/// carried in memory from the capture.
#[test]
fn an_exported_save_imports_into_a_fresh_app_and_restores() {
    let mut live = boot(&reinforce_args());
    step(&mut live, REINFORCE_CAPTURE_AT);

    let captured_digest = world_digest(live.world());
    let run = run_for(
        capture(live.world()),
        captured_digest,
        SEED,
        REINFORCE,
        current_versions(REINFORCE),
    );
    let file = project_phoenix::snapshot::export_artifact(&run).expect("the export is written");

    // The world the file names, read WITHOUT the version gate — the step the
    // browser takes first, because it has to load that world before it has a
    // content digest to check the file against.
    assert_eq!(
        project_phoenix::snapshot::peek_artifact_scenario(&file)
            .expect("an intact file names its scenario"),
        REINFORCE,
        "an import knows which world to load from the file itself"
    );

    let imported = project_phoenix::snapshot::import_artifact(&file, &current_versions(REINFORCE))
        .expect("this build can honour its own export");
    assert_eq!(
        imported, run,
        "the imported record is the exported one, field for field"
    );

    let payload = imported
        .snapshot
        .as_ref()
        .expect("a saved game carries a snapshot")
        .state
        .clone();
    let mut resumed = boot_to_rebuild_point(&reinforce_args(), &payload);
    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        world_digest(resumed.world()),
        captured_digest,
        "a session resumed from a FILE stands exactly where the capture did"
    );
}

/// A damaged file is refused as damaged, and an incompatible one as incompatible.
///
/// AC5, stated as the one thing that makes the two messages worth having:
/// they are different **values**, not different wordings of one. A host told
/// "this file is damaged" goes looking for a better copy; a host told "this save
/// is from an older build" does not. Getting the classification wrong sends them
/// to the wrong place, and that is not a thing a shared error string can be
/// careful about.
///
/// Three shapes of damage, because a file arrives damaged in more than one way:
/// truncated mid-record (an interrupted download), replaced by something that is
/// not a save at all, and empty.
#[test]
fn a_damaged_file_and_an_incompatible_one_are_refused_differently() {
    use project_phoenix::snapshot::{import_artifact, LoadRefusal};

    let mut live = boot(&reinforce_args());
    step(&mut live, REINFORCE_CAPTURE_AT);
    let run = run_for(
        capture(live.world()),
        world_digest(live.world()),
        SEED,
        REINFORCE,
        current_versions(REINFORCE),
    );
    let current = current_versions(REINFORCE);
    let intact = project_phoenix::snapshot::export_artifact(&run).expect("the export is written");

    // The control: intact and honourable, so nothing below is refused for being
    // a save at all.
    assert!(import_artifact(&intact, &current).is_ok());

    for (what, damaged) in [
        ("truncated", intact[..intact.len() / 2].to_string()),
        ("not a save", "hello, this is not a save file".to_string()),
        ("empty", String::new()),
    ] {
        assert!(
            matches!(
                import_artifact(&damaged, &current),
                Err(LoadRefusal::Unparsable(_))
            ),
            "[{what}] a file this build cannot parse is DAMAGED, and must not be \
             reported as a version answer — there is no version in it to have moved"
        );
    }

    // Intact, and from a build whose payload shape moved. The refusal names the
    // dimension, which is the whole reason it is `vellum_save::Moved` verbatim.
    let older = Versions::new(SNAPSHOT_FORMAT - 1, SIMULATION_RULES, current.content);
    let stale = project_phoenix::snapshot::export_artifact(&run_for(
        capture(live.world()),
        world_digest(live.world()),
        SEED,
        REINFORCE,
        older,
    ))
    .expect("the export is written");
    let refusal = import_artifact(&stale, &current).expect_err("this build refuses it");
    assert!(
        matches!(refusal, LoadRefusal::Moved(Moved::Format { .. })),
        "an intact save this build cannot honour is INCOMPATIBLE, and the \
         refusal names which dimension moved: {refusal}"
    );

    // And the same file still parses, which is what makes the two classes
    // genuinely different rather than a fallback ordering.
    assert!(
        project_phoenix::snapshot::peek_artifact_scenario(&stale).is_ok(),
        "an incompatible save is a perfectly readable file — it is the BUILD \
         that cannot honour it, and a host told otherwise would go looking for a \
         better copy of a file that is already fine"
    );
}

/// The transfer store is a `Store` like the others, including the parts nothing
/// in this issue uses.
///
/// Small, and worth having: an incomplete `Store` impl would work for export and
/// import (which only ever write once and read once) and be quietly wrong the
/// first time anything asked it for a slot list.
#[test]
fn the_transfer_store_behaves_like_any_other_store() {
    use project_phoenix::snapshot::TransferStore;
    use vellum_save::Store;

    let store = TransferStore::empty();
    assert_eq!(store.read("autosave").expect("infallible"), None);
    assert!(store.slots().expect("infallible").is_empty());

    store.write("autosave", "hello").expect("infallible");
    assert_eq!(
        store.read("autosave").expect("infallible").as_deref(),
        Some("hello")
    );
    assert_eq!(store.slots().expect("infallible"), vec!["autosave"]);

    store.remove("autosave").expect("infallible");
    assert_eq!(store.read("autosave").expect("infallible"), None);
    store.remove("autosave").expect("removing nothing succeeds");

    let holding = TransferStore::holding("autosave", "text".to_string());
    assert_eq!(holding.take("autosave").as_deref(), Some("text"));
    assert_eq!(
        holding.take("autosave"),
        None,
        "taken means taken — an export must not be handed out twice"
    );
}
