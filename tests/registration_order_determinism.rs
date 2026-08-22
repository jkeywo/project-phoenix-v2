//! The standing regression guard for simulation-system registration order
//! (issue #899, the ordering half of the parent's determinism ask, #849).
//!
//! # What this proves
//!
//! `SimSet` is a chained set (`Input -> Physics -> Damage -> Modifiers ->
//! Publish -> PublishAggregate -> Broadcast`), but ordering *within* one set
//! is Bevy's to choose unless a system declares an explicit edge against its
//! neighbour. A new system added to a set without one inherits whatever order
//! `add_simulation_plugins_with`'s own `add_plugins` calls happened to run
//! in — stable on one machine, not guaranteed anywhere else, and exactly the
//! kind of thing that silently invalidates P2P lockstep (#854) the day it
//! stops holding.
//!
//! `SimPluginOptions::registration_order` (`src/server_app.rs`) is the knob
//! this guard turns: `RegistrationOrder::Shuffled(seed)` deterministically
//! permutes the order the 13 `SimSet`-chain plugins register in, while
//! leaving every `configure_sets` edge on `SimSet` itself — and every
//! plugin's own internal `.after`/`.before` wiring — untouched. Those edges
//! are exactly what SHOULD pin behaviour; this guard is the check that they
//! do, on every system that matters, not just the ones a human remembered to
//! test.
//!
//! # Why this is its own test binary
//!
//! Same reason as `tests/rng_determinism.rs`: `--deterministic` (which
//! `HeadlessArgs::deterministic` selects) pins Bevy's `TaskPoolPlugin` to a
//! single thread, but task pools are process-global and initialised by
//! whichever app builds first. Sharing a binary with other headless tests
//! means inheriting a multi-threaded pool a neighbour already created, which
//! is precisely the kind of nondeterminism this guard exists to rule out of
//! the SIMULATION — it must not creep in from the test harness instead.
//! Cargo gives every integration-test file its own process, which is what
//! keeps `--deterministic` meaning what it says. Do not add unrelated tests
//! here.
//!
//! # Scope
//!
//! Ordering only. The parent issue's other half — wall-clock and RNG gating —
//! is #903's slice, not this one; `tests/rng_determinism.rs` already covers
//! RNG-site determinism on its own axis (seed reproducibility), independent
//! of registration order.
//!
//! A second guard, `every_simulation_set_system_is_registered_under_fixed_update`
//! (issue #1182), closes the *schedule* half of the same family: every `SimSet`
//! member must live in `FixedUpdate` (the deterministic fixed clock) and never
//! in `Update`/`PostUpdate` (the frame-paced virtual clock) — the `Res<Time>`
//! trap ~62 systems would otherwise fall into. It too reads the real headless
//! app's built schedule graphs, walks `SimSet` set-membership across every
//! schedule, and enumerates the sim-adjacent fixed-family exceptions census-
//! style. Both guards share this binary because both assert schedule
//! structure against the real `--deterministic` app and want its task-pool
//! isolation.
//!
//! # What stays out of the shuffle, and why that is fine
//!
//! `add_simulation_plugins_with` registers a handful of `FixedUpdate`
//! residents outside the 13-plugin group this guard permutes. Each has its
//! own, already-explicit reason to sit where it does, so leaving them fixed
//! does not reopen the ambiguity this guard closes:
//!
//! - `register_admission_seam` (`.before(SimSet::Input)`) — explicitly
//!   ordered against reconcile/admission by registration order ON PURPOSE
//!   (see the comment above its call site in `server_app.rs`); shuffling it
//!   would defeat a DIFFERENT, already-settled tie-break, not test this one.
//! - `emit_phase_change_balance_events` (`server_app.rs`) — a single global
//!   reader/emitter with nothing in `FixedUpdate` racing it for the same
//!   state.
//! - `drain_lobby_outbox` (`lobby/server.rs`) — network-boundary drain, not a
//!   `SimSet` participant.
//! - `advance_sim_tick` — runs in `FixedLast`, after every `SimSet` stage.
//! - `StateTransition` — a Bevy-internal schedule, not one this crate
//!   registers systems into directly.
//!
//! The 13 plugins this guard DOES permute are exactly the ones that were
//! previously a flat, unexplained chain of `.add_plugins` calls in
//! `add_simulation_plugins_with` (issue #899 broke that chain out into
//! `SIM_SET_PLUGIN_REGISTRARS` so it could be reordered at all): `RegionPlugin`,
//! `ConsoleAiPlugin`, `AiPlugin`, `CaptainPlugin`, `HelmPlugin`, `ShipPlugin`,
//! `WeaponsPlugin`, `RepairPlugin`, `ShipPowerPlugin`, `ShipShieldsPlugin`,
//! `ShipSensorsPlugin`, `NavigationPlugin`, `CommsConsolePlugin`.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

mod common;

use bevy::ecs::schedule::{NodeId, Schedules, SystemKey, SystemSetKey};
use bevy::prelude::*;
use common::SimFixture;
use project_phoenix::headless::fingerprint::{fingerprint, RunFingerprint};
use project_phoenix::headless::HeadlessArgs;
use project_phoenix::server_app::{RegistrationOrder, RegistrationProbes};
use project_phoenix::sim_sets::SimSet;
use project_phoenix::sim_tick::SimTick;
use std::collections::{HashMap, HashSet};

/// `rng_coverage.toml` (issue #837): two NPCs in weapons range, an asteroid
/// field the player flies through, and a radiation zone — beam, blaster,
/// torpedo, collision and region damage all fire inside the window below.
/// Reused here (rather than a quieter world) so the fingerprint is actually
/// exercising the AI, weapons, repair, power, shields, sensors, navigation
/// and comms plugins this guard shuffles, not just idling them.
const WORLD: &str = "assets/worlds/rng_coverage.toml";

/// Long enough that the belt has been entered and at least one weapon has
/// fired (see `a_colliding_world_reaches_the_same_state_at_wildly_different_frame_rates`
/// in `tests/headless_runner.rs`, which uses the same world and a comparable
/// window); short enough that the guard stays fast.
const TICKS: u64 = 300;

const SEED: u64 = 20260899;

fn build_and_run(
    registration_order: RegistrationOrder,
    extra_registration_probes: Option<RegistrationProbes>,
) -> App {
    let args = HeadlessArgs {
        world_path: WORLD.into(),
        max_ticks: TICKS,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    };
    SimFixture::new(args)
        .registration_order(registration_order)
        .extra_registration_probes(extra_registration_probes)
        .build_and_run()
}

fn fingerprint_with_order(registration_order: RegistrationOrder) -> RunFingerprint {
    let mut app = build_and_run(registration_order, None);
    fingerprint(&mut app)
}

/// Issue #899's headline acceptance criterion: the same seed and the same
/// commands must reach the same digest regardless of the order simulation
/// systems were registered in.
///
/// Two runs of the identical seeded world — one with the 13 `SimSet`-chain
/// plugins registered in their canonical order, one with a deterministic
/// shuffle of it — are compared bit for bit via [`RunFingerprint`] (tick
/// count, every RNG stream's position, every ship's physics and hull state,
/// and the full ordered collision-attribution list). Run against two
/// different shuffle seeds so a single accidentally-order-preserving
/// permutation cannot pass this vacuously.
#[test]
fn the_same_seed_reaches_the_same_state_with_registration_order_shuffled() {
    let canonical = fingerprint_with_order(RegistrationOrder::Canonical);
    let shuffled_a = fingerprint_with_order(RegistrationOrder::Shuffled(1));
    let shuffled_b = fingerprint_with_order(RegistrationOrder::Shuffled(0xC0FFEE));

    assert!(
        !canonical.ships.is_empty(),
        "precondition: the fingerprint covers no ship — an empty slice would \
         make the comparison below vacuous"
    );
    assert!(
        !canonical.collisions.is_empty(),
        "precondition: no collision was recorded in {TICKS} ticks of {WORLD} — \
         this would degrade into a scenario with nothing for physics-adjacent \
         ordering to disturb. Ships: {:?}",
        canonical.ships
    );

    assert_eq!(
        canonical, shuffled_a,
        "the run diverged when the SimSet-chain plugins were registered in a \
         SHUFFLED order (seed 1) — some system in FixedUpdate is ordered \
         against a neighbour in the same SimSet only by registration order \
         (Bevy's --deterministic tie-break), not by an explicit \
         .after()/.before() edge. Add the missing edge in the system that \
         moved, rather than depending on where `add_simulation_plugins_with` \
         happens to register its plugin."
    );
    assert_eq!(
        canonical, shuffled_b,
        "the run diverged when the SimSet-chain plugins were registered in a \
         SHUFFLED order (seed 0xC0FFEE) — some system in FixedUpdate is \
         ordered against a neighbour in the same SimSet only by registration \
         order (Bevy's --deterministic tie-break), not by an explicit \
         .after()/.before() edge. Add the missing edge in the system that \
         moved, rather than depending on where `add_simulation_plugins_with` \
         happens to register its plugin."
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Mutation proof (issue #899, AC-2)
// ─────────────────────────────────────────────────────────────────────────

/// A value only this tick's own [`order_probe_write`] should be able to
/// produce, so [`OrderProbeLog`] reveals whether the read saw a fresh write
/// or a stale one from the tick before.
#[derive(Resource, Default)]
struct OrderProbeState(f32);

/// One entry per tick `order_probe_read` ran, in tick order.
#[derive(Resource, Default)]
struct OrderProbeLog(Vec<f32>);

/// Deliberately order-dependent: writes a value derived from the current
/// tick, with NO `.after`/`.before` against [`order_probe_read`] — the exact
/// shape the guard above exists to catch. In [`SimSet::Modifiers`] alongside
/// three real production systems (`translate_power_modifiers` and friends)
/// that already coexist there with no edges between each other either.
fn order_probe_write(mut state: ResMut<OrderProbeState>, tick: Res<SimTick>) {
    state.0 = tick.0 as f32 * 10.0 + 1.0;
}

/// Reads whatever [`order_probe_write`] most recently left behind. If it ran
/// first this tick, that is THIS tick's value (fresh); if it ran second,
/// that is last tick's leftover (stale) — the two registration orders below
/// are provably distinguishable by the resulting log.
fn order_probe_read(state: Res<OrderProbeState>, mut log: ResMut<OrderProbeLog>) {
    log.0.push(state.0);
}

fn register_probe_write(app: &mut App) {
    app.init_resource::<OrderProbeState>()
        .init_resource::<OrderProbeLog>()
        .add_systems(
            FixedUpdate,
            order_probe_write.in_set(project_phoenix::sim_sets::SimSet::Modifiers),
        );
}

fn register_probe_read(app: &mut App) {
    app.init_resource::<OrderProbeState>()
        .init_resource::<OrderProbeLog>()
        .add_systems(
            FixedUpdate,
            order_probe_read.in_set(project_phoenix::sim_sets::SimSet::Modifiers),
        );
}

/// Issue #899, AC-2: the guard above is proven by mutation, not just
/// asserted to work.
///
/// `SimPluginOptions::extra_registration_probes` folds
/// [`order_probe_write`]/[`order_probe_read`] into the exact same
/// registration-order machinery `register_sim_set_plugins` uses for the real
/// 13 plugins (`src/server_app.rs`) — the pair is appended to the shuffled
/// group and registered through the identical `fn(&mut App)` mechanism, so
/// this is not a parallel, hand-rolled ordering path. The two calls below
/// register the identical pair of systems in the two possible relative
/// orders (the one hand-picked permutation this pair can be in, rather than
/// searching for a `Shuffled` seed that happens to transpose two specific
/// array slots): write-then-read, and read-then-write.
///
/// A system pair like this is precisely what the guard above is watching
/// for: two systems in the same `SimSet` stage with no edge between them,
/// where one's output depends on whether it ran before or after the other.
/// If such a pair ever shipped among the REAL 13 plugins, the guard's
/// canonical-vs-shuffled comparison would fail exactly the way the assertion
/// below does — and disappear the moment the offending system gained an
/// explicit edge (equivalently: the moment it stopped mattering which of the
/// two registrations below is used). That "remove it, the divergence goes
/// away" half of the proof is what the always-on guard test already
/// demonstrates for the production plugin set, byte-identically, every run.
///
/// `#[ignore]`d because it exists to document and demonstrate the guard's
/// discriminating power, not to run as a standing regression check — run it
/// explicitly with `cargo test --features headless --test
/// registration_order_determinism -- --ignored`.
#[test]
#[ignore = "issue #899 mutation proof — demonstrates the guard's discriminating power on demand, not a standing regression check"]
fn an_order_dependent_system_pair_produces_different_results_when_flipped() {
    const PROBE_TICKS: u64 = 5;

    let probe_args = || HeadlessArgs {
        world_path: WORLD.into(),
        max_ticks: PROBE_TICKS,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    };

    let write_then_read = {
        let app = SimFixture::new(probe_args())
            .registration_order(RegistrationOrder::Canonical)
            .extra_registration_probes(Some((register_probe_write, register_probe_read)))
            .build_and_run();
        app.world().resource::<OrderProbeLog>().0.clone()
    };

    let read_then_write = {
        let app = SimFixture::new(probe_args())
            .registration_order(RegistrationOrder::Canonical)
            // Same pair, registered in the OPPOSITE order — the only change.
            .extra_registration_probes(Some((register_probe_read, register_probe_write)))
            .build_and_run();
        app.world().resource::<OrderProbeLog>().0.clone()
    };

    assert!(
        !write_then_read.is_empty(),
        "precondition: the probe never logged anything — GamePhase never \
         reached InProgress inside {PROBE_TICKS} ticks, so SimSet::Modifiers \
         never ran"
    );
    assert_eq!(
        write_then_read.len(),
        read_then_write.len(),
        "precondition: both runs cover the same seed, ticks and world, so the \
         probe must log the same NUMBER of times in each — a different count \
         here means the flip changed more than the two probes' relative \
         order"
    );
    assert!(
        write_then_read.iter().all(|&v| v != 0.0),
        "precondition: write-then-read should see a fresh, nonzero value every \
         tick: {write_then_read:?}"
    );
    assert_ne!(
        write_then_read, read_then_write,
        "flipping the registration order of two systems in the same SimSet, \
         with no edge between them, did NOT change the outcome — the probe \
         pair failed to demonstrate the property this guard exists to catch"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// FixedUpdate-membership guard (issue #1182)
// ─────────────────────────────────────────────────────────────────────────

/// The seven `SimSet`-chain variants (`src/sim_sets.rs`). Every system that is
/// a member of one of these must run in `FixedUpdate`: there `Res<Time>` is the
/// deterministic fixed clock, whereas in `Update`/`PostUpdate` the very same
/// `Res<Time>` is silently the frame-paced virtual clock. That substitution is
/// invisible at the type level (`Res<Time>` in both) — it is the `Res<Time>`
/// trap the ~62 time-reading sim systems live one misregistration away from.
const SIM_SETS: [SimSet; 7] = [
    SimSet::Input,
    SimSet::Physics,
    SimSet::Damage,
    SimSet::Modifiers,
    SimSet::Publish,
    SimSet::PublishAggregate,
    SimSet::Broadcast,
];

/// The genuine exceptions: `SimSet`-member systems that are permitted to run
/// OUTSIDE `FixedUpdate`, census-style — one `(system-name-substring, schedule)`
/// per entry, each earning its place.
///
/// EMPTY as of issue #1182. Every one of the ~168 `SimSet` members in the real
/// headless app resolves into `FixedUpdate`; there is no legitimate reason for a
/// system that reads the simulation clock through set membership to run on any
/// other. A future system that must be a `SimSet` member yet run elsewhere would
/// be added here WITH its justification, turning an intentional design choice
/// into a reviewed line of code rather than a silent guard failure; until then,
/// any `SimSet` member outside `FixedUpdate` is the bug this guard exists to
/// catch.
const ALLOWED_SIMSET_MEMBERS_OUTSIDE_FIXED_UPDATE: [(&str, &str); 0] = [];

/// The sim-ADJACENT systems that deliberately live outside `FixedUpdate`,
/// census-style: `(system-name-substring, expected-schedule, reason)`.
///
/// None of these is a `SimSet` member, so the membership guard never flags them
/// — they are enumerated here so the boundary is explicit and self-checking
/// (`sim_adjacent_infrastructure_sits_in_the_fixed_timestep_family` asserts each
/// one is present in its named schedule AND is not a `SimSet` member). Each runs
/// on the deterministic fixed clock anyway (`FixedFirst`/`FixedLast` are inside
/// the fixed-timestep loop; `First` only reconfigures the timestep before it),
/// so none is exposed to the trap above.
const FIXED_FAMILY_INFRASTRUCTURE: [(&str, &str, &str); 3] = [
    (
        "sim_tick::advance_sim_tick",
        "FixedLast",
        "advances the SimTick counter once per fixed step, after the whole \
         SimSet chain has run; kept out of any SimSet so a step counts exactly \
         once (see register_sim_tick)",
    ),
    (
        "world_id::sync_world_id_mint",
        "FixedFirst",
        "resets the tick-scoped WorldIdMint at the head of each fixed step so \
         every sim system in the step mints against that step's index; inside \
         the fixed loop, not a SimSet member",
    ),
    (
        "sim_tick::reconcile_fixed_timestep",
        "First",
        "reconciles Time<Fixed>'s period from the loaded WorldConfig before the \
         fixed loop runs; touches Time only to reconfigure the timestep and runs \
         no simulation logic",
    ),
];

/// A brief headless run — enough frames to build the FixedUpdate/Update/… graphs
/// the guards read; the schedule structure they assert is fixed at registration,
/// not something the length of the run changes.
fn schedule_probe_app() -> App {
    let args = HeadlessArgs {
        world_path: WORLD.into(),
        max_ticks: 5,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    };
    SimFixture::new(args).build_and_run()
}

/// Every `(schedule-label, SimSet-variant, system-name)` triple for which the
/// system is a member of that `SimSet` variant, across EVERY schedule in the
/// app — not just `FixedUpdate`. This is the census the guards filter.
///
/// Membership is read straight from each schedule's hierarchy graph: an
/// `.in_set(SimSet::X)` registration adds a `Set(X) -> member` edge there
/// (Bevy's `ScheduleGraph`), so the set's transitive out-edges ARE its members,
/// nested subsets included. A system misregistered into `Update.in_set(..)`
/// therefore shows up here tagged `Update`, which is exactly what makes it
/// catchable.
fn simset_members_by_schedule(app: &App) -> Vec<(String, String, String)> {
    let schedules = app.world().resource::<Schedules>();
    let mut members = Vec::new();

    for (label, schedule) in schedules.iter() {
        let label_str = format!("{label:?}");
        let graph = schedule.graph();

        // SystemKey -> name. After `build_schedule` the systems are moved out of
        // `graph.systems` into the executable, so resolve from `Schedule::systems()`
        // when the schedule has run and fall back to the unbuilt container otherwise
        // (state schedules like `OnEnter(..)` may never have run).
        let mut names: HashMap<SystemKey, String> = HashMap::new();
        if let Ok(built) = schedule.systems() {
            for (key, sys) in built {
                names.insert(key, sys.name().as_string());
            }
        } else {
            for (key, sys, _conditions) in graph.systems.iter() {
                names.insert(key, sys.name().as_string());
            }
        }

        // The `SimSet` variants that exist as set nodes in THIS schedule.
        let simset_nodes: Vec<(SystemSetKey, String)> = SIM_SETS
            .iter()
            .filter_map(|ss| {
                graph
                    .system_sets
                    .get_key(ss.intern())
                    .map(|key| (key, format!("{ss:?}")))
            })
            .collect();
        if simset_nodes.is_empty() {
            continue;
        }

        // Hierarchy adjacency: edge (parent-set -> child member).
        let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for (parent, child) in graph.hierarchy().graph().all_edges() {
            children.entry(parent).or_default().push(child);
        }

        for (set_key, set_name) in simset_nodes {
            let mut stack = vec![NodeId::Set(set_key)];
            let mut seen: HashSet<NodeId> = HashSet::new();
            while let Some(node) = stack.pop() {
                if !seen.insert(node) {
                    continue;
                }
                if let Some(cs) = children.get(&node) {
                    stack.extend(cs.iter().copied());
                }
                if let Some(system_key) = node.as_system() {
                    let name = names
                        .get(&system_key)
                        .cloned()
                        .unwrap_or_else(|| format!("<unresolved {system_key:?}>"));
                    members.push((label_str.clone(), set_name.clone(), name));
                }
            }
        }
    }

    members
}

/// Issue #1182's headline acceptance criterion: every simulation-set system is
/// registered under `FixedUpdate`, closing the `Res<Time>` trap for good.
///
/// Reads the REAL headless app's built schedules, walks `SimSet` membership
/// across all of them, and fails if any member resolves into a schedule other
/// than `FixedUpdate` (minus the census allow-list, currently empty). Prior art
/// is the shuffle guard above and the physics-last ordering test: both assert
/// schedule structure against the real app rather than a hand-rolled fixture.
#[test]
fn every_simulation_set_system_is_registered_under_fixed_update() {
    let app = schedule_probe_app();
    let members = simset_members_by_schedule(&app);

    // Precondition: the whole sim chain really is being enumerated, so an
    // all-clear cannot be a vacuous pass from an empty walk.
    let fixed_update_members: HashSet<&String> = members
        .iter()
        .filter(|(schedule, _, _)| schedule == "FixedUpdate")
        .map(|(_, _, system)| system)
        .collect();
    assert!(
        fixed_update_members.len() >= 150,
        "precondition: expected the SimSet chain's ~168 members under \
         FixedUpdate, found only {} — the membership walk is not seeing the \
         real app's simulation systems",
        fixed_update_members.len()
    );

    let offenders: Vec<&(String, String, String)> = members
        .iter()
        .filter(|(schedule, _, _)| schedule != "FixedUpdate")
        .filter(|(schedule, _, system)| {
            !ALLOWED_SIMSET_MEMBERS_OUTSIDE_FIXED_UPDATE.iter().any(
                |(allowed_sys, allowed_sched)| {
                    system.contains(allowed_sys) && schedule == allowed_sched
                },
            )
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "these SimSet members are registered OUTSIDE FixedUpdate, where \
         `Res<Time>` silently becomes the frame-paced virtual clock instead of \
         the deterministic fixed clock (issue #1182): {offenders:?}. Move each \
         back to `FixedUpdate`, or — if it genuinely must run elsewhere — add it \
         to `ALLOWED_SIMSET_MEMBERS_OUTSIDE_FIXED_UPDATE` with a justification."
    );
}

/// A no-op standing in for a simulation system whose author reached for the
/// wrong schedule. Used only by the negative test below.
fn misregistered_marker_system() {}

/// Issue #1182, the deliberate-misregistration proof: registering a `SimSet`
/// member in `Update` makes the guard fire, naming the offender.
///
/// This is the schedule-guard analogue of the shuffle guard's mutation proof
/// (issue #899 AC-2) above. It takes
/// the real headless app, injects `misregistered_marker_system` into `Update`
/// as a `SimSet::Physics` member — the exact mistake #1182 exists to catch —
/// and asserts the SAME offender computation the guard above uses now flags it.
/// Remove the misregistration (as the assertion's inverse shows) and the
/// offender list is empty again: the guard is load-bearing, not decorative.
#[test]
fn the_membership_guard_fires_on_a_sim_system_misregistered_outside_fixed_update() {
    let ticks = 5u64;
    let args = HeadlessArgs {
        world_path: WORLD.into(),
        max_ticks: ticks,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    };
    let mut app = SimFixture::new(args).build();
    // The misregistration: a SimSet member in Update rather than FixedUpdate.
    app.add_systems(Update, misregistered_marker_system.in_set(SimSet::Physics));
    project_phoenix::headless::run(&mut app, ticks);

    let members = simset_members_by_schedule(&app);

    // The membership walk must SEE the misregistered system, tagged Update/Physics.
    let seen = members.iter().any(|(schedule, set, system)| {
        schedule == "Update" && set == "Physics" && system.contains("misregistered_marker_system")
    });
    assert!(
        seen,
        "the membership walk did not observe a SimSet::Physics member injected \
         into Update — the guard would be blind to a real misregistration. \
         Members outside FixedUpdate: {:?}",
        members
            .iter()
            .filter(|(s, _, _)| s != "FixedUpdate")
            .collect::<Vec<_>>()
    );

    // ...and the guard's offender computation must reject it (fail-on-removal-of-
    // protection: were the marker instead a real production system, the guard
    // above would panic exactly here).
    let offenders: Vec<&(String, String, String)> = members
        .iter()
        .filter(|(schedule, _, _)| schedule != "FixedUpdate")
        .filter(|(schedule, _, system)| {
            !ALLOWED_SIMSET_MEMBERS_OUTSIDE_FIXED_UPDATE.iter().any(
                |(allowed_sys, allowed_sched)| {
                    system.contains(allowed_sys) && schedule == allowed_sched
                },
            )
        })
        .collect();
    assert!(
        offenders
            .iter()
            .any(|(_, _, system)| system.contains("misregistered_marker_system")),
        "offender computation missed the injected misregistration: {offenders:?}"
    );
}

/// Issue #1182, the census's live half: the sim-adjacent systems enumerated in
/// `FIXED_FAMILY_INFRASTRUCTURE` really do sit in the fixed-timestep-family
/// schedule each claims, and none of them is secretly a `SimSet` member.
///
/// This keeps the allow-list honest. If `advance_sim_tick` were ever moved, or
/// quietly folded into a `SimSet`, the census would be stale — and a stale
/// census is how an exception silently stops being the thing it was justified
/// as. Asserting it turns the comment into a check.
#[test]
fn sim_adjacent_infrastructure_sits_in_the_fixed_timestep_family() {
    let app = schedule_probe_app();
    let schedules = app.world().resource::<Schedules>();

    // schedule-label -> the system names it contains.
    let mut by_schedule: HashMap<String, Vec<String>> = HashMap::new();
    for (label, schedule) in schedules.iter() {
        let entry = by_schedule.entry(format!("{label:?}")).or_default();
        if let Ok(built) = schedule.systems() {
            for (_key, sys) in built {
                entry.push(sys.name().as_string());
            }
        } else {
            for (_key, sys, _conditions) in schedule.graph().systems.iter() {
                entry.push(sys.name().as_string());
            }
        }
    }

    for (system, expected_schedule, _reason) in FIXED_FAMILY_INFRASTRUCTURE {
        let present = by_schedule
            .get(expected_schedule)
            .is_some_and(|names| names.iter().any(|n| n.contains(system)));
        assert!(
            present,
            "census drift: `{system}` is enumerated as living in the \
             `{expected_schedule}` schedule, but it was not found there. Update \
             FIXED_FAMILY_INFRASTRUCTURE (issue #1182)."
        );
    }

    // None of them may be a SimSet member — that is the whole premise for their
    // being exempt from the FixedUpdate obligation.
    let members = simset_members_by_schedule(&app);
    for (system, _expected_schedule, _reason) in FIXED_FAMILY_INFRASTRUCTURE {
        assert!(
            !members.iter().any(|(_, _, m)| m.contains(system)),
            "census contradiction: `{system}` is listed as a non-SimSet \
             fixed-family system, but it IS a SimSet member — it must then \
             either run in FixedUpdate or leave FIXED_FAMILY_INFRASTRUCTURE."
        );
    }
}
