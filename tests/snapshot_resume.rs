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
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::messages::{GamePhase, ServerMessage};
use project_phoenix::server_app::{GameOverReason, SimOutbox};
use project_phoenix::sim_digest::world_digest;
use project_phoenix::snapshot::{
    capture, load_from, ready_to_restore, restore, run_for, save_to, versions, LoadRefusal,
    PhoenixSnapshot, SavedGame, SIMULATION_RULES, SNAPSHOT_FORMAT,
};
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
    use project_phoenix::entity_spawner::EntityUuid;
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
        if ready_to_restore(app.world(), snapshot) {
            return app;
        }
        app.update();
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
        .query::<&project_phoenix::simulation::AsteroidUuid>()
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
/// which reaches the ledger through `FsTemplateLoader` resolving `--ship` —
/// since `duel.toml`'s NPC slots became script (issue #984, M6) they are no
/// longer in `world_config.entities`, so `content_ledger::
/// eager_record_world_entities` does not see them (tracked as its own issue).
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
    outcome: project_phoenix::balance::Outcome,
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
        project_phoenix::balance::Outcome::Defeat,
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
        Some(project_phoenix::balance::Outcome::Defeat),
        "the restored GameOverReason's outcome should have survived — \
         on_game_over_enter only takes the reason half"
    );

    let outbox = &resumed.world().resource::<SimOutbox>().0;
    let reasons: Vec<&str> = outbox
        .iter()
        .filter_map(|(_, msg)| match msg {
            ServerMessage::GameOver { reason } => Some(reason.as_str()),
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
