//! External schedule-property proof for #1400 slice 3. Each mode boots in a
//! fresh process because Bevy's task pools belong to the process, not the App.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::app::FixedMain;
use bevy::ecs::schedule::{ExecutorKind, ScheduleLabel};
use bevy::prelude::*;
use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::headless::{build_headless_app, parse_args, ParseOutcome};
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig,
};

const WORLD: &str = "assets/worlds/default.toml";
const MODE: &str = "PHOENIX_EXECUTOR_TEST_MODE";

#[test]
fn fixed_executor_policy_is_observable_in_isolated_boots() {
    for mode in [
        "headless-flag",
        "headless-seed",
        "headless-normal",
        "native-pinned",
        "native-normal",
    ] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "executor_policy_child",
                "--ignored",
                "--nocapture",
            ])
            .env(MODE, mode)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("executor policy observed"),
            "the named child must actually run"
        );
    }
}

fn inert_late_registration() {}

#[test]
#[ignore = "launched by parent with one fresh task-pool process per boot mode"]
fn executor_policy_child() {
    let mode = std::env::var(MODE).expect("parent supplies a boot mode");
    let pinned = match mode.as_str() {
        "headless-flag" | "headless-seed" | "native-pinned" => true,
        "headless-normal" | "native-normal" => false,
        _ => panic!("unknown mode"),
    };
    let mut app = if mode.starts_with("headless-") {
        let mut cli = vec!["--world".into(), WORLD.into()];
        if mode == "headless-flag" {
            cli.push("--deterministic".into());
        }
        if mode == "headless-seed" {
            cli.extend(["--seed".into(), "1400".into()]);
        }
        let ParseOutcome::Run(args) = parse_args(cli).unwrap() else {
            panic!("run expected")
        };
        assert_eq!(args.deterministic, pinned);
        build_headless_app(&args).unwrap()
    } else {
        let preload = preload_content_templates(".").unwrap();
        let mut config = NativeHostConfig::new(WORLD);
        config.surface = NativeRenderSurface::Contract;
        config.deterministic = pinned;
        build_native_host_app(&config, &preload).unwrap()
    };
    let fixed = [
        FixedFirst.intern(),
        FixedPreUpdate.intern(),
        FixedUpdate.intern(),
        FixedPostUpdate.intern(),
        FixedLast.intern(),
        bevy::state::state::StateTransition.intern(),
    ];
    // Adding systems after BootPlan composes must preserve the executor. This
    // also materializes normally empty schedules in the normal-mode app.
    for label in fixed {
        app.add_systems(label, inert_late_registration);
    }
    app.finish();
    app.cleanup();
    for label in fixed {
        assert_eq!(
            app.get_schedule(label).unwrap().get_executor_kind(),
            if pinned {
                ExecutorKind::SingleThreaded
            } else {
                ExecutorKind::default()
            },
            "{mode}: {label:?}"
        );
    }
    // Bevy already makes this facilitator serial. The frame Update executor
    // remains its original default even when the fixed family is pinned.
    assert_eq!(
        app.get_schedule(FixedMain).unwrap().get_executor_kind(),
        ExecutorKind::SingleThreaded
    );
    assert_eq!(
        app.get_schedule(Update).unwrap().get_executor_kind(),
        ExecutorKind::default()
    );
    if pinned {
        assert_eq!(bevy::tasks::ComputeTaskPool::get().thread_num(), 1);
    }
    println!("executor policy observed: {mode}");
}
