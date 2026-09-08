//! Live coverage and measured/unmeasured state proof, isolated because pools
//! and the tracing subscriber are process-global. No duration is asserted.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::headless::{
    build_headless_app, build_headless_app_with_external_logging, run, run_sampled_with_phases,
    HeadlessArgs,
};
use project_phoenix::perf::{
    self,
    phase::observed_metric,
    phase_trace::{observe_schedule, PhaseProfiler},
    tick::TickSampler,
};
use project_phoenix::sim_sets::SimSet;
use project_phoenix::sim_tick::SimTick;
const MODE: &str = "PHOENIX_PHASE_PROOF_CHILD";
const PREFIX: &str = "PHOENIX_PHASE_PROOF=";

#[test]
fn external_profiling_preserves_state_and_census_and_captures_the_default_executor() {
    let mut reports = Vec::new();
    for mode in ["ordinary", "profiled", "default-coverage"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "phase_proof_child", "--ignored", "--nocapture"])
            .env(MODE, mode)
            .env("RUST_LOG", "error")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{mode}: {}\n{stdout}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let rows: Vec<_> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix(PREFIX))
            .collect();
        assert_eq!(rows.len(), 1, "child must run and report exactly once");
        let report: serde_json::Value = serde_json::from_str(rows[0]).unwrap();
        assert_eq!(report["mode"], mode);
        println!(
            "{mode}: tick={} digest={} pool={} intervals={}",
            report["tick"], report["digest"], report["pool"], report["intervals"]
        );
        reports.push(report);
    }
    // The default-pool determinism proof is #1400 slice4, not this collector's
    // responsibility: parity here uses the same explicit pinned configuration.
    for field in ["tick", "digest", "seed", "schedule"] {
        assert_eq!(
            reports[0][field], reports[1][field],
            "profiling changed {field}"
        );
    }
    assert_binary_capture();
}

fn assert_binary_capture() {
    let capture_path =
        std::env::temp_dir().join(format!("phoenix-phase-{}.json", uuid::Uuid::new_v4()));
    let coverage_path = std::path::PathBuf::from(format!("{}.phases.json", capture_path.display()));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_phoenix-headless"))
        .args([
            "--world",
            "assets/worlds/combat_test.toml",
            "--ship",
            "assets/entities/alliance_destroyer.toml",
            "--seed",
            "20260894",
            "--ticks",
            "12",
            "--perf-scenario",
            "phase-proof",
        ])
        .arg("--perf-capture")
        .arg(&capture_path)
        .env("RUST_LOG", "error")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "real perf-capture command: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let capture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&capture_path).unwrap()).unwrap();
    assert!(
        capture["summaries"].get("sim.fixed.elapsed").is_some(),
        "the actual binary must write its phase metric"
    );
    let coverage: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&coverage_path).unwrap()).unwrap();
    assert!(coverage["invocations"]
        .as_u64()
        .is_some_and(|count| count > 0));
    assert!(coverage["samples"]
        .as_array()
        .is_some_and(|samples| !samples.is_empty()));
    std::fs::remove_file(capture_path).unwrap();
    std::fs::remove_file(coverage_path).unwrap();
}

#[test]
#[ignore = "launched by the parent in a fresh process for each observer mode"]
fn phase_proof_child() {
    let mode = std::env::var(MODE).unwrap();
    assert!(["ordinary", "profiled", "default-coverage"].contains(&mode.as_str()));
    let measured = mode != "ordinary";
    let args = HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        seed: Some(20260894),
        deterministic: mode != "default-coverage",
        max_ticks: 240,
        dt: 1.0 / 60.0,
        // Existing --perf-capture's console-latency pipeline is present on BOTH
        // sides; this proof adds no redundant measurement of that window.
        console_latency: true,
        ..Default::default()
    };
    let mut profiler = measured.then(|| PhaseProfiler::install("").unwrap());
    let mut app = if measured {
        build_headless_app_with_external_logging(&args)
    } else {
        build_headless_app(&args)
    }
    .unwrap();
    let seed = app
        .world()
        .resource::<project_phoenix::sim_rng::SimRng>()
        .seed();
    assert_eq!(seed, 20260894);
    let pool = bevy::tasks::ComputeTaskPool::get().thread_num();
    let mut intervals = 0;
    if let Some(profiler) = &mut profiler {
        let mut sampler = TickSampler::new();
        assert_eq!(
            run_sampled_with_phases(&mut app, 240, &mut sampler, profiler).unwrap(),
            240
        );
        assert!(profiler.coverage().invocations > 0);
        intervals = profiler.coverage().observed_intervals;
        assert!(intervals > 0);
        assert!(profiler
            .coverage()
            .schedule
            .as_ref()
            .unwrap()
            .systems
            .iter()
            .all(|(name, _)| !name.contains("Enable the debug feature")));
        let capture = sampler.finish("phase-proof", perf::profile("headless-native"));
        for phase in [
            SimSet::Input,
            SimSet::Physics,
            SimSet::Damage,
            SimSet::Modifiers,
            SimSet::Publish,
            SimSet::PublishAggregate,
            SimSet::Broadcast,
        ] {
            assert!(capture.summaries.get(observed_metric(phase)).is_some_and(|metric| metric.summary.count > 0),
                "actual {phase:?} coverage missing despite error-only formatting; unknown names: {:?}", profiler.coverage().unattributed_names);
        }
    } else {
        assert_eq!(run(&mut app, 240), 240);
    }
    if mode == "default-coverage" {
        assert!(
            pool > 1,
            "this machine's default compute pool cannot establish worker coverage"
        );
        assert_eq!(
            app.get_schedule(FixedUpdate).unwrap().get_executor_kind(),
            bevy::ecs::schedule::ExecutorKind::MultiThreaded
        );
    }
    let tick = app.world().resource::<SimTick>().0;
    assert!(tick > 0);
    assert_eq!(
        app.world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get(),
        &project_phoenix::core::messages::GamePhase::InProgress
    );
    let report = serde_json::json!({ "mode": mode, "tick": tick, "seed": seed, "pool": pool,
        "intervals": intervals, "digest": project_phoenix::sim_digest::world_digest(app.world()),
        "schedule": observe_schedule(&app).unwrap() });
    println!("\n{PREFIX}{report}");
}
