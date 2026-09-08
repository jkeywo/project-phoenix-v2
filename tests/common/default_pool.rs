//! Fresh-process default-pool arms for existing determinism guards (#1400).
//! No production pool settings or schedules are changed by this observer.
use bevy::ecs::schedule::ExecutorKind;
use bevy::prelude::{App, FixedUpdate, State};
use project_phoenix::core::messages::GamePhase;
use project_phoenix::sim_digest::world_digest;
use project_phoenix::sim_rng::SimRng;
use project_phoenix::sim_tick::SimTick;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const CHILD_ENV: &str = "PHOENIX_DEFAULT_PERTURBATION_CHILD";
const PREFIX: &str = "PHOENIX_DEFAULT_PERTURBATION_RESULT=";

#[derive(Debug, Serialize, Deserialize)]
struct Observation {
    guard: String,
    process: u32,
    world: String,
    seed: u64,
    tick: u64,
    digest: u64,
    compute_threads: usize,
    executor: String,
}

/// Existing tests retain their pinned configuration in the ordinary process.
/// Only an exact-test child launched below selects the ordinary default pool.
pub fn deterministic() -> bool {
    std::env::var_os(CHILD_ENV).is_none()
}

/// Observe a completed App without changing it; inert in the pinned process.
pub fn observe(app: &App, world: &str, seed: u64) {
    let Ok(guard) = std::env::var(CHILD_ENV) else {
        return;
    };
    let compute_threads = bevy::tasks::ComputeTaskPool::get().thread_num();
    assert!(compute_threads > 1,
        "{guard}: ordinary ComputeTaskPool has {compute_threads} worker(s); this runner cannot prove multithreaded determinism. Do not override the allocation or silently accept a pinned pool.");
    let executor = app
        .get_schedule(FixedUpdate)
        .expect("ordinary fixed schedule")
        .get_executor_kind();
    assert_eq!(
        executor,
        ExecutorKind::MultiThreaded,
        "{guard}: the default arm must use the ordinary multi-threaded executor"
    );
    assert_eq!(
        app.world().resource::<SimRng>().seed(),
        seed,
        "{guard}: actual mission seed"
    );
    let tick = app.world().resource::<SimTick>().0;
    assert!(tick > 0, "{guard}: a tick-zero App is not a mission proof");
    assert!(
        matches!(
            app.world().resource::<State<GamePhase>>().get(),
            GamePhase::InProgress | GamePhase::GameOver
        ),
        "{guard}: the mission must have left loading/lobby"
    );
    let report = Observation {
        guard,
        process: std::process::id(),
        world: world.into(),
        seed,
        tick,
        digest: world_digest(app.world()),
        compute_threads,
        executor: format!("{executor:?}"),
    };
    println!("\n{PREFIX}{}", serde_json::to_string(&report).unwrap());
}

/// Re-run exact existing tests sequentially, each in a fresh process. Global
/// pools initialized by pinned tests in the parent therefore cannot leak in.
/// The report count prevents an exact filter matching zero tests from passing.
pub fn run_guards(guards: &[(&str, usize)]) {
    assert!(
        deterministic(),
        "a child must run an original guard, never recurse into the parent"
    );
    for &(guard, reports_expected) in guards {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", guard, "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, guard)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("start the exact guard in a fresh process");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{guard} exited {}:\n{stdout}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let reports: Vec<Observation> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix(PREFIX))
            .map(|line| serde_json::from_str(line).expect("structured default-pool observation"))
            .collect();
        assert_eq!(
            reports.len(),
            reports_expected,
            "{guard}: the child must finish the expected Apps, not match zero tests: {stdout}"
        );
        let mut boundaries = BTreeMap::<(String, u64), (u64, u64, usize)>::new();
        for report in &reports {
            assert_eq!(report.guard, guard);
            assert_ne!(
                report.process,
                std::process::id(),
                "a fresh process is required"
            );
            assert!(report.compute_threads > 1);
            assert_eq!(report.executor, "MultiThreaded");
            let first = boundaries
                .entry((report.world.clone(), report.seed))
                .or_insert((report.tick, report.digest, 0));
            assert_eq!(
                first.0, report.tick,
                "{guard}: compare the same completed tick boundary"
            );
            assert_eq!(
                first.1, report.digest,
                "{guard}: authoritative digest differs at the same seed/boundary:\n{reports:#?}"
            );
            first.2 += 1;
            println!("{PREFIX}{}", serde_json::to_string(report).unwrap());
        }
        assert!(!boundaries.is_empty());
        assert!(
            boundaries.values().all(|(_, _, count)| *count >= 2),
            "{guard}: every scenario/seed needs an actual comparison"
        );
    }
}
