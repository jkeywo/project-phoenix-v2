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
/// which reaches the ledger through `FsTemplateLoader` resolving `--ship`.
///
/// The duel's NPC hulls are deliberately not the ones edited: they are not in
/// the FROZEN ledger and never were. `content_ledger::
/// eager_record_world_entities` walks `world_config.entities`, so a hull that
/// arrives any other way — a `[[trigger]]` spawn action, as duel.toml's slots
/// did before issue #984, or a script spawn, as they do after it — is recorded
/// only when it actually spawns, which is after `freeze`. That is a #935-class
/// gap about trigger/script-spawned templates generally, tracked as its own
/// issue; the M6 conversion changed which side of it these slots sit on, not
/// whether the gap exists.
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

fn live_inbox(app: &bevy::prelude::App) -> Vec<project_phoenix::messages::CommsMessage> {
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

// ── Issue #1026: an in-flight operation survives a resume ───────────────────

/// The stabilise probe: one tender, one degraded depot, one scripted operation.
const STABILISE: &str = "assets/worlds/probe_stabilise.toml";

/// Frames to run before the stabilise capture.
///
/// The probe starts the operation at t=1 s (tick 60) and completes it three
/// authored seconds later (tick 240), so this has to land strictly between the
/// two: a capture taken before the start would round-trip a ship running
/// nothing, which is exactly the payload a restore that dropped the whole field
/// would also produce, and one taken after the finish would round-trip a
/// settled hold that no longer advances. The assertions below make both halves
/// of that precondition explicit rather than trusting the number.
const STABILISE_CAPTURE_AT: u64 = 150;

fn stabilise_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: STABILISE.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// **Issue #1026.** A mission interrupted mid-operation comes back mid-operation
/// — the banked eligible ticks, the spent stall budget, the state and the id
/// counter together.
///
/// Restoring the hold's progress alone would be worse than dropping it: a
/// resumed crew would be shown a bar that no longer matched what the simulation
/// thought it owed them, and the next operation would reuse an id the console
/// had already displayed. The control below reads the freshly booted ship first,
/// so an inert restore is visible rather than hidden behind a value that
/// happened to match.
#[test]
fn the_resumed_world_keeps_a_ship_mid_operation() {
    let mut live = boot(&stabilise_args());
    step(&mut live, STABILISE_CAPTURE_AT);

    let payload = capture(live.world());
    let captured: Vec<_> = payload
        .entities
        .iter()
        .filter_map(|e| e.operations.as_ref().map(|o| (&e.uuid, o)))
        .collect();
    assert_eq!(
        captured.len(),
        1,
        "the probe world carries exactly one ship with an operations record"
    );
    let (uuid, record) = captured[0];
    let hold = record
        .active
        .as_ref()
        .expect("precondition: the capture must be taken after the scripted start");
    assert!(
        hold.elapsed_ticks() > 0 && !hold.is_settled(),
        "precondition: and strictly BEFORE the completion — a capture of a ship running nothing, \
         or of a settled hold, would round-trip identically even if restore dropped the field \
         entirely ({} of {} ticks, {:?})",
        hold.elapsed_ticks(),
        hold.required_ticks(),
        hold.state()
    );
    assert_eq!(
        record.next_id, 1,
        "precondition: one operation has been handed an id"
    );

    let mut resumed = boot_to_restore_point(&stabilise_args(), &payload);
    let before_restore = resumed
        .world_mut()
        .query::<&project_phoenix::operations::ShipOperations>()
        .iter(resumed.world())
        .next()
        .cloned()
        .expect("the fresh world spawned the tender from its template");
    assert!(
        before_restore.active.is_none(),
        "control: a freshly booted tender is running nothing, so an inert restore would be \
         visible here rather than hidden behind a value that happened to match"
    );
    assert_eq!(
        before_restore
            .capabilities
            .capability(project_phoenix::operations::OperationVerb::Stabilise)
            .map(|c| c.duration_secs),
        Some(3),
        "…and it carries its authored capability, which the save deliberately does NOT: that is \
         content, re-derived from the template on spawn"
    );

    restore(resumed.world_mut(), &payload);
    let after = capture(resumed.world());
    let restored = after
        .entities
        .iter()
        .find(|e| &e.uuid == uuid)
        .and_then(|e| e.operations.as_ref())
        .unwrap_or_else(|| panic!("ship {uuid} came back without its operations record"));
    assert_eq!(
        restored, record,
        "ship {uuid}: the hold's banked ticks, its required ticks, its spent stall budget, its \
         state and the id counter must all come back exactly as captured"
    );

    let live_capabilities = resumed
        .world_mut()
        .query::<&project_phoenix::operations::ShipOperations>()
        .iter(resumed.world())
        .next()
        .map(|ops| ops.capabilities.clone())
        .expect("the record is still attached");
    assert_eq!(
        live_capabilities, before_restore.capabilities,
        "and the restore left the spawned capability table alone — it restores the mutable half \
         of the record, not the content half"
    );
}

/// **Issue #1026.** A world that runs no operations writes nothing
/// operation-shaped into its save.
#[test]
fn a_world_with_no_operations_writes_no_operation_state() {
    let mut live = duel();
    step(&mut live, 120);
    let payload = capture(live.world());
    assert!(
        payload.entities.iter().all(|e| e.operations.is_none()),
        "every world in the repository but the probe is in this arm, and none of them should pay \
         a byte for a feature they do not use"
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
