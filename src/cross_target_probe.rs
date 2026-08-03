//! The seeded cross-target determinism probe (issue #904).
//!
//! # What this proves, and why it is not two native processes
//!
//! PRD #849's user story asks for "two instances under artificial delay". Two
//! *native* instances is the one configuration that cannot detect the
//! divergence most likely to occur, because both would share whatever the
//! native build's physics and libm routing happen to be. The claim that
//! matters for P2P lockstep is native ↔ **wasm**: the browser host is one of
//! the two peers, and it is compiled for a different target, with a different
//! backend, a different threading model, and — before issues #908/#909 — a
//! different libm.
//!
//! This module is the single seeded world both targets drive. The native side
//! is `tests/cross_target_probe.rs`; the browser side is
//! [`wasm_cross_target_probe`] at the bottom of this file, reached from
//! `tests/smoke/cross-target-determinism.spec.ts`. Both fold their state
//! through `crate::sim_digest::world_digest` — the canonical #901 digest, the
//! same function a headless run folds, not a probe-local approximation — and
//! both produce a [`DigestLedger`], so comparing them is
//! `DigestLedger::first_divergence`, which names the first disagreeing tick
//! rather than merely reporting that two numbers differ.
//!
//! # Why a purpose-built world and not `combat_test.toml`
//!
//! Because the browser cannot build one. `headless::app::build_headless_app`
//! reads the world TOML, every `assets/entities/*.toml`, the ship hull and the
//! composition closure straight off the filesystem; there is no filesystem in
//! a wasm page, and the in-page equivalent (the JS preload feeding
//! `config_cache`) produces the *live* app, which carries a renderer, a lobby,
//! sessions and a scenario — none of which a native headless run has. Driving
//! that would compare two different simulations and call the difference a
//! divergence.
//!
//! So the probe builds the smallest world that still exercises the machinery
//! the cross-target risk actually lives in, from Rust literals alone with no
//! asset I/O on either side:
//!
//! * **Rapier's broadphase and narrowphase.** The bodies are
//!   `KinematicPositionBased` with ball colliders, exactly as production ships
//!   are, and [`probe_resolve_contacts`] picks its contact the way
//!   `server_app::handle_collisions` does — lowest world id among pairs with
//!   an active contact, never "whatever the narrow phase yielded first". This
//!   is the surface `Cargo.toml`'s dropped `parallel` feature (issue #896) was
//!   about: a parallel broadphase reorders float accumulation, so a native
//!   build that quietly got it back would agree with another native build and
//!   disagree with the browser. That regression fails *here*.
//! * **`crate::simmath`.** Every transcendental the steering does routes
//!   through the shared pure-Rust libm (issues #908/#909). `simmath_vectors`
//!   already proves those functions agree in isolation; this proves they still
//!   agree after 240 ticks of feeding each other.
//! * **`SimRng`** (`Pcg32`, issue #897) — drawn from on collision damage
//!   distribution, so a divergent *draw count* moves the digest on the tick it
//!   happens.
//! * **`WorldIdMint`** (issue #907) — ships are minted mid-run, so the fold's
//!   sort key is populated rather than defaulted and a divergent *spawn count*
//!   is caught the tick it happens.
//! * **The fixed tick** (issue #895) — `SimTick` advances in `FixedLast`, and
//!   the whole probe simulation lives in `FixedUpdate`.
//!
//! # The artificial delay
//!
//! [`ProbeConfig::pacing`] is how many logical ticks of virtual time one
//! `App::update()` advances, cycled. [`EVEN_PACING`] is one tick per update —
//! the smooth peer. [`BURSTY_PACING`] is `1, 2, 3, 4, 2` — a peer whose frames
//! stall and then catch up, which is what "injected delay" looks like to a
//! simulation that steps on a fixed clock. Both cycles sum to a divisor of
//! [`CHECKPOINT_INTERVAL`], so every checkpoint tick falls on a frame boundary
//! under *both* pacings and the two ledgers have every checkpoint in common
//! rather than a sparse intersection.
//!
//! The native test drives both pacings; the browser drives [`BURSTY_PACING`].
//! The comparison therefore spans pacing *and* target at once, which is the
//! whole of AC1/AC2/AC5: same seed, same command log, different delay,
//! different instruction set, identical digests.
//!
//! # The command log
//!
//! [`COMMAND_LOG`] is a fixed `(tick, ship index, throttle, turn)` schedule,
//! applied in `FixedUpdate` on the tick whose number it names — never on a
//! frame. Both instances consume the identical log. It is deliberately not a
//! transport: nothing here sends, receives, joins, or recovers (AC6). It is
//! the "same command log" half of AC1 and nothing more.
//!
//! # Re-blessing the pinned ledger
//!
//! The pinned digests live in **one place**: `tests/fixtures/
//! cross-target-ledger.json`, written by the native test and read by both the
//! native test and the smoke spec. See that test's
//! `the_committed_ledger_matches_this_build` for the exact procedure. Any
//! change to the fold, to this world, to the tick count, or to `simmath`'s
//! output moves those numbers; that is a deliberate, reviewed re-bless, not a
//! failure to paper over.

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::core::messages::{GamePhase, SystemId};
use crate::damage::SystemHull;
use crate::entity_spawner::{EntitySystemHull, EntityUuid};
use crate::ship::state::{ShipPhysics, ShipRedAlert};
use crate::sim_digest::{world_digest, DigestLedger};
use crate::sim_rng::{SeedSource, SimRng, SimStream};
use crate::sim_tick::{sim_tick_period, SimTick};
use crate::world_id::{mint_id_with, IdNamespace, WorldIdMint};

// ── The pinned shape of the run ──────────────────────────────────────────────

/// The tick rate the probe world runs at. Fixed here rather than read from a
/// `WorldConfig`, because the probe has no world TOML on either target.
pub const PROBE_HZ: f32 = 60.0;

/// The seed both instances consume (AC1).
pub const PROBE_SEED: u64 = 904_849;

/// How many logical ticks the probe runs.
///
/// 240 = four seconds of simulation. Long enough that the ships have completed
/// a full approach, collided, taken RNG-distributed hull damage and had two
/// mid-run mints land; short enough that the in-page run stays far inside a
/// Playwright timeout — see the timing note in the smoke spec. Every pacing
/// cycle divides it exactly, so both instances stop on the same tick rather
/// than one overshooting.
pub const PROBE_TICKS: u64 = 240;

/// Sample a digest every N ticks. 240/12 = 20 checkpoints, which is a small
/// enough committed fixture to read in a diff and dense enough that a reported
/// divergence names a 12-tick window rather than the whole run.
pub const CHECKPOINT_INTERVAL: u64 = 12;

/// One logical tick per `App::update()` — the peer whose frames are on time.
pub const EVEN_PACING: &[u32] = &[1];

/// A peer under injected delay: stall, then catch up. Cycles to 12 ticks, so
/// it lands on every [`CHECKPOINT_INTERVAL`] boundary that [`EVEN_PACING`]
/// does.
pub const BURSTY_PACING: &[u32] = &[1, 2, 3, 4, 2];

/// How many ships the world starts with. Mints bring it to
/// `INITIAL_SHIPS + SPAWN_TICKS.len()`.
const INITIAL_SHIPS: usize = 5;

/// Ticks on which one more ship is minted into the world, exercising the
/// tick-scoped mint (issue #907) mid-run rather than only at start-up.
const SPAWN_TICKS: [u64; 2] = [61, 137];

/// The scripted command log both instances consume:
/// `(tick, ship index, throttle, turn rate)`.
///
/// Deliberately irregular — same-tick pairs, a lone command, a command aimed
/// at a ship that has not been minted yet on the earliest ticks — so that "the
/// log was applied in the same order on the same tick" is a real claim rather
/// than a property of a uniform schedule.
const COMMAND_LOG: &[(u64, usize, f32, f32)] = &[
    (5, 0, 34.0, 0.35),
    (5, 3, 21.0, -0.6),
    (17, 1, 47.5, 0.125),
    (29, 2, 12.25, 1.05),
    (29, 0, 55.0, -0.25),
    (48, 4, 30.75, 0.5),
    (66, 5, 40.0, -0.875),
    (83, 1, 8.5, 1.5),
    (110, 3, 62.0, 0.0),
    (141, 6, 25.0, -1.25),
    (166, 2, 51.25, 0.75),
    (199, 0, 18.0, -1.75),
];

// ── Configuration ────────────────────────────────────────────────────────────

/// One instance's run parameters.
#[derive(Clone, Debug)]
pub struct ProbeConfig {
    pub seed: u64,
    pub ticks: u64,
    pub checkpoint_interval: u64,
    /// Ticks of virtual time each `App::update()` advances, cycled. See the
    /// module docs on the artificial delay.
    pub pacing: &'static [u32],
    /// The mutation knob (AC4). `Some(tick)` perturbs one ship's forward speed
    /// by a single ULP at the start of that tick — the smallest cross-instance
    /// difference expressible in `f32`. Always `None` in the pinned run; the
    /// native test sets it to prove the comparison actually catches a
    /// divergence, and catches it at the right tick.
    pub mutate_at: Option<u64>,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            seed: PROBE_SEED,
            ticks: PROBE_TICKS,
            checkpoint_interval: CHECKPOINT_INTERVAL,
            pacing: EVEN_PACING,
            mutate_at: None,
        }
    }
}

impl ProbeConfig {
    /// The pinned run, at the given pacing.
    pub fn paced(pacing: &'static [u32]) -> Self {
        Self {
            pacing,
            ..Self::default()
        }
    }
}

// ── Probe-local resources and sets ───────────────────────────────────────────

/// Which slot in the world a ship occupies. The command log keys on this, and
/// so does the steering phase, so neither depends on spawn order in the
/// archetypes.
#[derive(Component, Clone, Copy, Debug)]
struct ProbeSlot(usize);

/// The ULP perturbation knob, as a resource so the driving loop can set it
/// without rebuilding the app.
#[derive(Resource, Clone, Copy, Debug, Default)]
struct ProbeMutation(Option<u64>);

/// The probe's own schedule ordering. Mirrors production's relationship to
/// rapier: our motion runs before `PhysicsSet::SyncBackend` reads the
/// transforms, and our damage runs after `PhysicsSet::Writeback`.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ProbeSet {
    /// Command log, mid-run mints, and the mutation knob.
    Input,
    /// Steering and integration — writes `Transform` and `ShipPhysics`.
    Motion,
    /// Contact resolution and hull damage — reads rapier's writeback.
    Damage,
}

// ── World construction ───────────────────────────────────────────────────────

/// The hull every probe ship carries. Four systems, so `fold_hull` walks a
/// real insertion order and RNG-distributed damage has somewhere to spill.
fn probe_hull() -> SystemHull {
    SystemHull::from_config(&[
        (SystemId("helm".into()), 40.0),
        (SystemId("weapons".into()), 30.0),
        (SystemId("shields".into()), 25.0),
        (SystemId("engineering".into()), 35.0),
    ])
}

/// Spawn one ship in `slot`, minting its id from the tick-scoped mint.
///
/// `commands` rather than direct world access so the start-up spawns and the
/// mid-run spawns go through the identical path — a mid-run mint that used a
/// different code path would be proving something else.
fn spawn_probe_ship(commands: &mut Commands, mint: Option<&WorldIdMint>, slot: usize) {
    // Positions and headings come from `simmath` over the slot index, so the
    // *initial conditions themselves* are a cross-target claim rather than a
    // table of literals both targets trivially agree on.
    let angle = slot as f32 * 1.256_637_1;
    let radius = 70.0 + slot as f32 * 6.5;
    let (sin, cos) = crate::simmath::sin_cos(angle);
    let x = cos * radius;
    let z = sin * radius;
    // Point every ship at the origin, so they converge on the hazard and
    // actually collide. Yaw 0 faces -Z (the `ShipPhysics` convention), so the
    // heading that walks *towards* the origin from `(x, z)` is `atan2(x, z)`,
    // which `probe_steer` then integrates as `(-sin yaw, -cos yaw)`.
    let yaw = crate::simmath::atan2(x, z);

    commands.spawn((
        EntityUuid(mint_id_with(mint, IdNamespace::Entity)),
        ProbeSlot(slot),
        Transform::from_xyz(x, 0.0, z),
        GlobalTransform::default(),
        Visibility::default(),
        ShipPhysics {
            x,
            y: 0.0,
            z,
            yaw,
            forward_speed: 24.0 + slot as f32 * 3.0,
            roll: 0.0,
            lateral_speed: 0.0,
            vertical_speed: 0.0,
        },
        ShipRedAlert(false),
        EntitySystemHull(probe_hull()),
        Collider::ball(6.0),
        RigidBody::KinematicPositionBased,
        ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
    ));
}

/// The uuid of the hazard at the origin.
///
/// A version-4 literal, deliberately: an `AsteroidUuid` is coordinate-derived
/// rather than minted (see `world_id`'s module docs and
/// `sim_digest::FoldKey`), so it keys as `(0, 0)` and sorts on the raw string.
/// Carrying one here means the probe folds the **asteroid namespace** as well
/// as the entity namespace, and exercises the string-tiebreak arm of the sort
/// that a ships-only world would leave untouched.
const HAZARD_UUID: &str = "904a5704-0000-4000-8000-00000000ha21";

/// A fixed obstacle at the origin, wide enough that every converging ship
/// flies into it.
///
/// The probe needs contacts to be *certain*, not merely likely. Five ships
/// converging on a point do pass near each other, but whether any two are
/// within a collider diameter on the same tick is an emergent property of the
/// command log — and a probe whose physics coverage can be switched off by
/// retuning a throttle value is not a guard. The hazard makes rapier's
/// broadphase, the lowest-id contact pick and the seeded damage distribution
/// unconditional.
fn spawn_probe_hazard(commands: &mut Commands) {
    commands.spawn((
        crate::server_app::AsteroidUuid(HAZARD_UUID.to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        GlobalTransform::default(),
        Visibility::default(),
        EntitySystemHull(SystemHull::from_config(&[(SystemId("mass".into()), 500.0)])),
        Collider::ball(24.0),
        RigidBody::Fixed,
        ActiveCollisionTypes::KINEMATIC_STATIC,
    ));
}

fn probe_startup(mut commands: Commands, mint: Option<Res<WorldIdMint>>) {
    spawn_probe_hazard(&mut commands);
    for slot in 0..INITIAL_SHIPS {
        spawn_probe_ship(&mut commands, mint.as_deref(), slot);
    }
}

// ── Simulation systems ───────────────────────────────────────────────────────

/// Apply the scripted command log for this tick, in the log's own order.
///
/// Reads `Res<SimTick>` rather than counting frames: on a bursty frame that
/// runs four steps, a frame-counted log would fire all four ticks' commands at
/// once and the two pacings would stop being the same run.
fn probe_apply_command_log(tick: Res<SimTick>, mut ships: Query<(&ProbeSlot, &mut ShipPhysics)>) {
    let now = tick.0;
    for (at, slot, throttle, turn) in COMMAND_LOG.iter().copied() {
        if at != now {
            continue;
        }
        // A command for a slot that does not exist yet is dropped, not
        // deferred — the log is a fixed script, and both instances drop the
        // identical entries on the identical ticks.
        for (ship_slot, mut physics) in ships.iter_mut() {
            if ship_slot.0 == slot {
                physics.forward_speed = throttle;
                physics.yaw = crate::simmath::atan2(
                    crate::simmath::sin(physics.yaw + turn),
                    crate::simmath::cos(physics.yaw + turn),
                );
            }
        }
    }
}

/// Mint the mid-run ships on their scheduled ticks.
fn probe_spawn_scheduled(
    tick: Res<SimTick>,
    mut commands: Commands,
    mint: Option<Res<WorldIdMint>>,
    existing: Query<&ProbeSlot>,
) {
    let Some(index) = SPAWN_TICKS.iter().position(|t| *t == tick.0) else {
        return;
    };
    let slot = INITIAL_SHIPS + index;
    if existing.iter().any(|s| s.0 == slot) {
        return;
    }
    spawn_probe_ship(&mut commands, mint.as_deref(), slot);
}

/// The mutation knob (AC4): one ULP on one ship's forward speed, once.
///
/// A whole unit would be a caricature. One ULP is the smallest difference two
/// instances could possibly have, so catching it is evidence the comparison
/// has no tolerance hiding inside it — the #901 fold quantises nothing, and
/// this is what asserts that end to end.
fn probe_mutate(
    tick: Res<SimTick>,
    mutation: Res<ProbeMutation>,
    mut ships: Query<(&ProbeSlot, &mut ShipPhysics)>,
) {
    if mutation.0 != Some(tick.0) {
        return;
    }
    for (slot, mut physics) in ships.iter_mut() {
        if slot.0 == 0 {
            physics.forward_speed = f32::from_bits(physics.forward_speed.to_bits() + 1);
        }
    }
}

/// Steer and integrate every ship, in world-id order.
///
/// The sort is the same discipline `handle_collisions` follows and for the
/// same reason: `Query::iter_mut` walks archetypes, which is an artefact of
/// spawn/despawn history rather than anything the simulation authored. Here it
/// would not change the result (each ship's update reads only itself), but a
/// probe that relies on "it happens not to matter" is not a probe of ordering
/// discipline, and this world grows a mid-run spawn precisely to disturb
/// archetype order.
fn probe_steer(
    tick: Res<SimTick>,
    mut ships: Query<(&EntityUuid, &ProbeSlot, &mut ShipPhysics, &mut Transform)>,
) {
    let dt = 1.0 / PROBE_HZ;
    let mut order: Vec<(String, usize)> = ships
        .iter()
        .map(|(uuid, slot, _, _)| (uuid.0.clone(), slot.0))
        .collect();
    order.sort();

    for (_, slot) in order {
        for (_, ship_slot, mut physics, mut transform) in ships.iter_mut() {
            if ship_slot.0 != slot {
                continue;
            }
            // A slow yaw oscillation on top of whatever the command log last
            // set, so the trajectories curve and the contact set changes over
            // the run instead of being decided in the first tick.
            let wobble = crate::simmath::sin(
                (tick.0 as f32) * 0.031_25 + slot as f32 * 0.618_034, // golden-ratio offset: no two ships in phase
            ) * 0.02;
            physics.yaw += wobble * dt * PROBE_HZ * 0.05;
            physics.roll = crate::simmath::sin(physics.yaw) * 0.25;

            let (sin_yaw, cos_yaw) = crate::simmath::sin_cos(physics.yaw);
            // Yaw 0 faces -Z, matching `ShipPhysics`' documented convention.
            let step = physics.forward_speed * dt;
            physics.x += -sin_yaw * step;
            physics.z += -cos_yaw * step;
            physics.vertical_speed = crate::simmath::cos(physics.yaw) * 0.5;
            physics.y += physics.vertical_speed * dt;

            transform.translation = Vec3::new(physics.x, physics.y, physics.z);
            transform.rotation = Quat::from_rotation_y(physics.yaw);
            break;
        }
    }
}

/// Resolve one contact per ship per tick and distribute the hull damage.
///
/// The contact *choice* mirrors `server_app::handle_collisions` exactly:
/// filter to pairs with an active contact, then take the lowest world id.
/// Taking whatever the narrow phase yields first is the failure mode issue
/// #896 fixed, and a probe that took the easy path would agree with a build
/// that had regressed.
fn probe_resolve_contacts(
    context: ReadRapierContext,
    sim_rng: Option<Res<SimRng>>,
    bodies: Query<(
        Entity,
        Option<&EntityUuid>,
        Option<&crate::server_app::AsteroidUuid>,
    )>,
    mut ships: Query<(
        Entity,
        &EntityUuid,
        &mut EntitySystemHull,
        &mut ShipRedAlert,
    )>,
) {
    let Ok(ctx) = context.single() else {
        return;
    };

    // Every collidable body's world id, snapshotted before the mutable walk so
    // the lowest-id pick can compare ids without holding a second borrow.
    // Ships and the hazard alike: the contact partner is not necessarily a
    // ship, and picking "the lowest id among ships" would silently mean
    // "ignore the rock" — which is not what production does.
    let ids: Vec<(Entity, String)> = bodies
        .iter()
        .filter_map(|(entity, ship, rock)| {
            ship.map(|u| u.0.clone())
                .or_else(|| rock.map(|u| u.0.clone()))
                .map(|id| (entity, id))
        })
        .collect();

    // World-id order for the *outer* walk too: each resolved contact draws
    // from the shared `CollisionDamage` stream, so which ship resolves first
    // decides every later ship's numbers. `Query::iter` walks archetypes,
    // which the mid-run spawns deliberately disturb.
    let mut order: Vec<(String, Entity)> = ships
        .iter()
        .map(|(entity, uuid, _, _)| (uuid.0.clone(), entity))
        .collect();
    order.sort();

    for (_, entity) in order {
        let contact = ctx
            .contact_pairs_with(entity)
            // `contact_pairs_with` yields every pair whose *bounding volumes*
            // overlap, not only the ones actually touching — the same filter
            // `handle_collisions` applies, and for the same reason: a nearer
            // AABB must not out-rank a body the ship is genuinely inside.
            .filter(|pair| pair.has_any_active_contact())
            .filter_map(|pair| {
                if pair.collider1() == Some(entity) {
                    pair.collider2()
                } else {
                    pair.collider1()
                }
            })
            .filter_map(|candidate| {
                ids.iter()
                    .find(|(e, _)| *e == candidate)
                    .map(|(_, id)| id.clone())
            })
            .min();
        if contact.is_none() {
            continue;
        }

        let Ok((_, _, mut hull, mut alert)) = ships.get_mut(entity) else {
            continue;
        };
        alert.0 = true;
        crate::sim_rng::with_stream(sim_rng.as_deref(), SimStream::CollisionDamage, |rng| {
            hull.0.apply_damage(0.9, rng);
        });
    }
}

// ── App construction ─────────────────────────────────────────────────────────

/// Build the probe world. Identical on every target: no filesystem, no assets,
/// no JS host, no renderer.
pub fn build_probe_app(cfg: &ProbeConfig) -> App {
    let period = sim_tick_period(PROBE_HZ);
    let mut app = App::new();

    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<bevy::mesh::Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>()
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>();

    // The logical tick and the mint that is scoped to it (issues #895/#907).
    app.insert_resource(Time::<Fixed>::from_duration(period));
    crate::sim_tick::register_sim_tick(&mut app);

    // Physics on the tick, at the tick's own rate (issue #896). Same
    // construction `server_app::register_physics` uses, from the same
    // `sim_tick_period` — never a second, independent `1.0 / hz`.
    app.insert_resource(TimestepMode::Fixed {
        dt: period.as_secs_f32(),
        substeps: 1,
    })
    .add_plugins(RapierPhysicsPlugin::<()>::default().in_fixed_schedule());

    app.insert_resource(SimRng::new(cfg.seed, SeedSource::Cli));
    app.insert_resource(ProbeMutation(cfg.mutate_at));

    app.configure_sets(
        FixedUpdate,
        (ProbeSet::Input, ProbeSet::Motion, ProbeSet::Damage).chain(),
    )
    .configure_sets(
        FixedUpdate,
        (
            PhysicsSet::SyncBackend.after(ProbeSet::Motion),
            PhysicsSet::Writeback.before(ProbeSet::Damage),
        ),
    );

    app.add_systems(Startup, probe_startup);
    app.add_systems(
        FixedUpdate,
        (probe_apply_command_log, probe_spawn_scheduled, probe_mutate)
            .chain()
            .in_set(ProbeSet::Input),
    );
    app.add_systems(FixedUpdate, probe_steer.in_set(ProbeSet::Motion));
    app.add_systems(FixedUpdate, probe_resolve_contacts.in_set(ProbeSet::Damage));

    app
}

// ── Driving ──────────────────────────────────────────────────────────────────

/// Run the probe and return its checkpoint ledger.
///
/// The pacing loop is the artificial delay: each `App::update()` advances the
/// virtual clock by `k` whole tick periods, so Bevy's fixed loop runs exactly
/// `k` steps. `Duration * u32` is exact integer nanosecond arithmetic, so the
/// accumulator never drifts a step early or late — the hazard
/// `sim_tick_period`'s own doc comment warns about.
///
/// A digest is folded only *between* `update()` calls, which is the
/// `RenderInterp` bracket the #901 record requires, and only on ticks the
/// ledger samples. Every pacing cycle divides [`CHECKPOINT_INTERVAL`], so
/// every sampling tick is a frame boundary under every pacing — no checkpoint
/// is skipped merely because a burst stepped past it.
pub fn run_probe(cfg: &ProbeConfig) -> DigestLedger {
    let period = sim_tick_period(PROBE_HZ);
    let mut app = build_probe_app(cfg);
    let mut ledger = DigestLedger::new(cfg.checkpoint_interval);

    let mut frame = 0usize;
    loop {
        let steps = cfg.pacing[frame % cfg.pacing.len()];
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            period * steps,
        ));
        app.update();
        frame += 1;

        let tick = app.world().resource::<SimTick>().0;
        if ledger.samples(tick) {
            let digest = world_digest(app.world());
            ledger.record(tick, digest);
        }
        if tick >= cfg.ticks {
            break;
        }
    }

    ledger.final_digest = world_digest(app.world());
    ledger
}

/// The tick the probe actually finished on, for the caller that wants to
/// assert the pacing landed exactly rather than overshot.
pub fn probe_end_tick(ledger: &DigestLedger) -> u64 {
    ledger.checkpoints.last().map_or(0, |c| c.tick)
}

// ── The wire shape both targets speak ────────────────────────────────────────

/// One sampled checkpoint, as it crosses the target boundary.
///
/// The digest is a **hex string**, never a JSON number. A `u64` digest
/// routinely exceeds `Number.MAX_SAFE_INTEGER`, and `JSON.parse` would round
/// it silently — a comparison that agrees because both sides lost the same low
/// bits is worse than no comparison at all.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReportCheckpoint {
    pub tick: u64,
    pub digest: String,
}

/// One instance's run, in the shape the committed fixture and the wasm export
/// both use. One definition, so a fixture the native test writes and a JSON
/// the browser returns are field-for-field comparable rather than two hand-
/// rolled encodings that could drift.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProbeReport {
    pub seed: u64,
    pub ticks: u64,
    pub interval: u64,
    /// `"even"` or `"bursty"` — which artificial delay this instance ran
    /// under. Recorded so a reader can see at a glance that the pinned side
    /// and the browser side were *not* paced alike, which is the point.
    pub pacing: String,
    pub checkpoints: Vec<ReportCheckpoint>,
    pub final_digest: String,
}

impl ProbeReport {
    pub fn from_ledger(ledger: &DigestLedger, cfg: &ProbeConfig, pacing: &str) -> Self {
        Self {
            seed: cfg.seed,
            ticks: cfg.ticks,
            interval: cfg.checkpoint_interval,
            pacing: pacing.to_string(),
            checkpoints: ledger
                .checkpoints
                .iter()
                .map(|c| ReportCheckpoint {
                    tick: c.tick,
                    digest: format!("{:016x}", c.digest),
                })
                .collect(),
            final_digest: format!("{:016x}", ledger.final_digest),
        }
    }
}

// ── WASM export ──────────────────────────────────────────────────────────────
// The browser-side half of the proof. `server.html` promotes this onto
// `window` through its export allowlist — an explicit list, not an automatic
// re-export — and `tests/smoke/cross-target-determinism.spec.ts` calls it.
//
// Deliberately NOT feature-gated, for the same reason `wasm_simmath_battery`
// is not (see `simmath_vectors.rs`): the claim is about the *deployed*
// artifact, and an export behind a test-only feature would only ever prove a
// binary nobody serves.
//
// It runs [`BURSTY_PACING`] because the native pin is recorded under
// [`EVEN_PACING`] — the comparison has to span pacing as well as target, or it
// is only half of AC1.
//
// Blocking for `PROBE_TICKS` steps on the browser's main thread is accepted:
// this is an automation entry point, called by a smoke spec on a page that has
// nothing else to do. See the spec for the measured wall time.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn wasm_cross_target_probe() -> String {
    let cfg = ProbeConfig::paced(BURSTY_PACING);
    let ledger = run_probe(&cfg);
    let report = ProbeReport::from_ledger(&ledger, &cfg, "bursty");
    serde_json::to_string(&report)
        .unwrap_or_else(|e| format!("{{\"error\":\"probe report would not serialise: {e}\"}}"))
}
