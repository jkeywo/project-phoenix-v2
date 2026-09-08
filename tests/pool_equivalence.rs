//! #1400 slice 4's first proof: two default-pool runs and one explicitly
//! deterministic run reach the same authoritative state. Each App has a fresh
//! process because Bevy task pools are process-global. No scheduling or world
//! mutation is introduced by this observer.
//!
//! Requires slice 3's real fixed-family executor pin. This is not the remaining
//! default-pool archetype/order/registration perturbations or resume proof.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::app::FixedMain;
use bevy::ecs::schedule::{ExecutorKind, ScheduleLabel};
use bevy::prelude::*;
use project_phoenix::core::messages::GamePhase;
use project_phoenix::headless::{build_headless_app, parse_args, ParseOutcome};
use project_phoenix::server_app::LocalShip;
use project_phoenix::sim_digest::world_digest;
use project_phoenix::sim_tick::SimTick;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Exactly the mission, seed and 240-frame ManualDuration boundary used by
// native_headless_digest.rs. Its native side chooses the world's first hull;
// resolve that same choice without constructing a second App in this process.
const WORLD: &str = "assets/worlds/combat_test.toml";
const SEED: u64 = 20260894;
const FRAMES: u64 = 240;
const ROLE_ENV: &str = "PHOENIX_POOL_PROOF_CHILD";
const REPORT_PREFIX: &str = "PHOENIX_POOL_PROOF_RESULT=";

#[derive(Debug, Serialize, Deserialize)]
struct Observation {
    role: String,
    world: String,
    ship: String,
    seed: u64,
    frames: u64,
    sim_tick: u64,
    phase: GamePhase,
    digest: u64,
    ticks: Vec<(u64, u64)>,
    compute_threads: usize,
    executors: BTreeMap<String, Option<String>>,
}

#[test]
fn default_pool_twice_and_pinned_executor_reach_the_same_digest() {
    // The parent never builds an App or initializes a pool. Run children
    // sequentially so this proof does not become a CPU-contention benchmark.
    let observations: Vec<Observation> = ["default-1", "default-2", "pinned"]
        .into_iter()
        .map(|role| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "pool_proof_child", "--ignored", "--nocapture"])
                .env(ROLE_ENV, role)
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .expect("start a fresh copy of this test executable");
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "{role} exited {}:\n{stdout}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            let reports: Vec<_> = stdout
                .lines()
                .filter_map(|line| line.strip_prefix(REPORT_PREFIX))
                .collect();
            assert_eq!(
                reports.len(),
                1,
                "{role}: exactly one real child report required: {stdout}"
            );
            let observed: Observation = serde_json::from_str(reports[0]).unwrap();
            assert_eq!(observed.role, role);
            println!("{REPORT_PREFIX}{}", reports[0]);
            observed
        })
        .collect();

    for other in &observations[1..] {
        let first = &observations[0];
        assert_eq!(first.world, other.world);
        assert_eq!(first.ship, other.ship);
        assert_eq!(first.seed, other.seed);
        assert_eq!(first.frames, other.frames);
        assert_eq!(
            first.sim_tick, other.sim_tick,
            "compare the same completed fixed-tick boundary"
        );
        assert_eq!(first.phase, other.phase);
        assert_eq!(
            first.digest, other.digest,
            "same mission and boundary must agree across fresh default/pinned runs:\n{observations:#?}"
        );
        assert_eq!(first.ticks, other.ticks, "every completed tick must agree");
    }
}

#[test]
#[ignore = "parent launches each role in a fresh process; do not invoke without its role"]
fn pool_proof_child() {
    let role = std::env::var(ROLE_ENV).expect("parent supplies the child role");
    let pinned = match role.as_str() {
        "default-1" | "default-2" => false,
        "pinned" => true,
        _ => panic!("unknown pool proof role {role}"),
    };
    let source: toml::Value = toml::from_str(&std::fs::read_to_string(WORLD).unwrap()).unwrap();
    let ship = source["available_ships"][0]["template_path"]
        .as_str()
        .expect("Combat Test offers its first hull")
        .to_string();
    let mut cli = vec![
        "--world".to_string(),
        WORLD.to_string(),
        "--ship".to_string(),
        ship.clone(),
        "--ticks".to_string(),
        FRAMES.to_string(),
    ];
    if pinned {
        cli.extend(["--deterministic".into(), "--seed".into(), SEED.to_string()]);
    }
    let ParseOutcome::Run(mut args) = parse_args(cli).unwrap() else {
        panic!("the real CLI must choose a run")
    };
    assert_eq!(args.deterministic, pinned);
    // --seed intentionally implies deterministic in the real CLI. Set only
    // the programmatic seed for the default roles; never accidentally test
    // three pinned Apps while claiming default-pool equivalence.
    args.seed = Some(SEED);
    args.dt = 1.0 / 60.0;
    let mut app = build_headless_app(&args).expect("ordinary headless boot");
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    ));
    app.finish();
    app.cleanup();

    let actual_seed = app
        .world()
        .resource::<project_phoenix::sim_rng::SimRng>()
        .seed();
    assert_eq!(
        actual_seed, SEED,
        "{role}: boot must apply the requested mission seed"
    );
    let compute_threads = bevy::tasks::ComputeTaskPool::get().thread_num();
    if pinned {
        assert_eq!(compute_threads, 1, "{role}: real deterministic pool");
    } else {
        assert!(compute_threads > 1,
            "{role}: default ComputeTaskPool has {compute_threads} worker(s); this machine cannot establish multithreaded equivalence. Use a runner whose ordinary default pool has more than one worker; do not override or silently pin the pool.");
    }
    // Read existing schedules only: do not add probes to materialize empty
    // schedules and thereby change the production graph in this comparison.
    let fixed = [
        FixedFirst.intern(),
        FixedPreUpdate.intern(),
        FixedUpdate.intern(),
        FixedPostUpdate.intern(),
        FixedLast.intern(),
        bevy::state::state::StateTransition.intern(),
    ];
    let mut executors = BTreeMap::new();
    for label in fixed {
        let kind = app
            .get_schedule(label)
            .map(|schedule| schedule.get_executor_kind());
        if let Some(kind) = kind {
            assert_eq!(
                kind,
                if pinned {
                    ExecutorKind::SingleThreaded
                } else {
                    ExecutorKind::default()
                },
                "{role}: {label:?}; pinned execution requires #1400 slice 3"
            );
        }
        executors.insert(format!("{label:?}"), kind.map(|kind| format!("{kind:?}")));
    }
    assert_eq!(
        app.get_schedule(FixedUpdate).unwrap().get_executor_kind(),
        if pinned {
            ExecutorKind::SingleThreaded
        } else {
            ExecutorKind::MultiThreaded
        }
    );
    let main_kind = app.get_schedule(FixedMain).unwrap().get_executor_kind();
    assert_eq!(main_kind, ExecutorKind::SingleThreaded);
    executors.insert("FixedMain".into(), Some(format!("{main_kind:?}")));

    // Do not use the early-GameOver run helper: the prior equivalence test
    // pumps exactly 240 frames, even if a future scenario ends early.
    let mut ticks = Vec::new();
    for _ in 0..FRAMES {
        app.update();
        ticks.push((
            app.world().resource::<SimTick>().0,
            world_digest(app.world()),
        ));
    }
    let sim_tick = app.world().resource::<SimTick>().0;
    assert!(
        sim_tick > 0,
        "a tick-zero comparison is not a mission proof"
    );
    let phase = app.world().resource::<State<GamePhase>>().get().clone();
    assert!(
        matches!(phase, GamePhase::InProgress | GamePhase::GameOver),
        "the scenario must have left the lobby/loading phases"
    );
    let mut local_ships = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
    assert_eq!(
        local_ships.iter(app.world()).count(),
        1,
        "the ordinary player ship must have spawned"
    );
    let observation = Observation {
        role,
        world: WORLD.into(),
        ship,
        seed: actual_seed,
        frames: FRAMES,
        sim_tick,
        phase,
        digest: world_digest(app.world()),
        ticks,
        compute_threads,
        executors,
    };
    // Start a new line even under libtest's test-name prefix. The parent
    // requires one report, so a zero-match child cannot masquerade as a pass.
    println!(
        "\n{REPORT_PREFIX}{}",
        serde_json::to_string(&observation).unwrap()
    );
}
