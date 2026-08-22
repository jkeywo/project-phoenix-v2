//! Guard tests for the boot seam (issue #1217).
//!
//! The headline is the three-profile registration-parity test: the whole point of
//! the module is that Headless, BrowserHost and BrowserAutomation cannot drift on
//! what the renderer (real or surrogate) owes the simulation, so a test builds all
//! three and asserts they land on the same four-asset/three-message floor — and
//! that only BrowserHost took the real render-stack path.
//!
//! Everything runs off an in-memory world fixture, so no filesystem, GPU, browser
//! or window is involved and the tests are native-`cargo test` clean.

use super::{build, BootError, BootPlan, BootProfile, RenderStackApplied, RenderSurrogateApplied};
use bevy::prelude::*;

use crate::console_bridge::{AiChatterEvent, HudStateChanged, LobbyStateChanged};
use crate::world::load::MemoryReader;
use crate::world::script::load::ScriptResolver;
use crate::world::server::PreCompiledScripts;

/// A resolver that serves no sibling scripts — the fixtures author their scripts
/// inline in a `[script]` block, which never reaches a resolver.
struct NoScriptResolver;

impl ScriptResolver for NoScriptResolver {
    fn read(&self, _path: &str) -> Option<String> {
        None
    }
}

/// The world path every fixture uses.
const WORLD_PATH: &str = "boot_test_world.toml";

/// A minimal, entity-free world that validates clean and carries no scripts.
const CLEAN_WORLD: &str = "[global]\nseed = 1\n";

/// A world whose inline `[script]` block is valid TOML but invalid Rhai, so the
/// compile produces an erroring finding (the activation gate's trigger) rather than
/// a `LoadError`.
const BROKEN_SCRIPT_WORLD: &str =
    "[global]\nseed = 1\n[script]\non_alpha = \"fn on_alpha(ctx) { let x = ; }\"\n";

/// A fresh [`BootPlan`] for `profile` over `world` — fresh because the boxed reader
/// is consumed by [`build`], so each build needs its own.
fn plan_with(profile: BootProfile, world: &str) -> BootPlan {
    BootPlan {
        profile,
        log_filter: "warn".to_string(),
        world_path: WORLD_PATH.to_string(),
        reader: Box::new(MemoryReader::new([(WORLD_PATH, world)])),
        script_resolver: Box::new(NoScriptResolver),
        single_threaded: false,
        raw_transform: None,
    }
}

/// A fresh plan for `profile` over the clean world.
fn plan_for(profile: BootProfile) -> BootPlan {
    plan_with(profile, CLEAN_WORLD)
}

const PROFILES: [(BootProfile, &str); 3] = [
    (BootProfile::Headless, "headless"),
    (BootProfile::BrowserHost, "browser-host"),
    (BootProfile::BrowserAutomation, "browser-automation"),
];

/// Assert the four render assets and three bridge messages are all registered.
fn assert_render_contract(app: &App, label: &str) {
    let w = app.world();
    assert!(
        w.contains_resource::<Assets<Shader>>(),
        "{label}: Shader asset type must be registered"
    );
    assert!(
        w.contains_resource::<Assets<Image>>(),
        "{label}: Image asset type must be registered"
    );
    assert!(
        w.contains_resource::<Assets<Mesh>>(),
        "{label}: Mesh asset type must be registered"
    );
    assert!(
        w.contains_resource::<Assets<StandardMaterial>>(),
        "{label}: StandardMaterial asset type must be registered"
    );
    assert!(
        w.contains_resource::<Messages<HudStateChanged>>(),
        "{label}: HudStateChanged message must be registered"
    );
    assert!(
        w.contains_resource::<Messages<LobbyStateChanged>>(),
        "{label}: LobbyStateChanged message must be registered"
    );
    assert!(
        w.contains_resource::<Messages<AiChatterEvent>>(),
        "{label}: AiChatterEvent message must be registered"
    );
}

#[test]
fn all_three_profiles_register_the_same_asset_and_message_floor() {
    for (profile, label) in PROFILES {
        let app = build(plan_for(profile)).unwrap_or_else(|e| panic!("{label} build failed: {e}"));
        assert_render_contract(&app, label);
    }
    crate::content_ledger::reset();
}

#[test]
fn the_render_stack_is_taken_only_for_the_browser_host() {
    let headless = build(plan_for(BootProfile::Headless)).expect("headless build");
    let host = build(plan_for(BootProfile::BrowserHost)).expect("browser-host build");
    let automation =
        build(plan_for(BootProfile::BrowserAutomation)).expect("browser-automation build");

    // BrowserHost drove the render stack and NOT the surrogate.
    assert!(
        host.world().contains_resource::<RenderStackApplied>(),
        "BrowserHost must take the render-stack path"
    );
    assert!(
        !host.world().contains_resource::<RenderSurrogateApplied>(),
        "BrowserHost must not also take the surrogate path"
    );

    // The two renderer-less profiles took the surrogate and NOT the stack.
    for (app, label) in [(&headless, "headless"), (&automation, "browser-automation")] {
        assert!(
            app.world().contains_resource::<RenderSurrogateApplied>(),
            "{label} must take the render-surrogate path"
        );
        assert!(
            !app.world().contains_resource::<RenderStackApplied>(),
            "{label} must not build the render stack"
        );
    }
    crate::content_ledger::reset();
}

#[test]
fn a_broken_world_aborts_headless_but_only_blocks_activation_for_the_browser() {
    // Headless is authoritative: a world whose scripts do not compile aborts the
    // build outright, so it activates zero content.
    let err = build(plan_with(BootProfile::Headless, BROKEN_SCRIPT_WORLD))
        .expect_err("headless must abort on a broken world");
    assert!(
        matches!(err, BootError::WorldInvalid(_)),
        "expected WorldInvalid, got {err:?}"
    );

    // A browser host keeps booting: the broken scripts are carried through as a
    // resource so the downstream WorldPlugin gate can refuse to activate them,
    // rather than the build failing here.
    for profile in [BootProfile::BrowserHost, BootProfile::BrowserAutomation] {
        let app = build(plan_with(profile, BROKEN_SCRIPT_WORLD))
            .unwrap_or_else(|e| panic!("{profile:?} must boot a broken world: {e}"));
        assert!(
            app.world().contains_resource::<PreCompiledScripts>(),
            "{profile:?} must carry the compiled (broken) scripts for the downstream gate"
        );
    }
    crate::content_ledger::reset();
}

#[test]
fn an_unreadable_world_is_a_load_error_for_every_profile() {
    for (profile, label) in PROFILES {
        // A plan whose reader carries nothing at the requested path.
        let plan = BootPlan {
            profile,
            log_filter: "warn".to_string(),
            world_path: WORLD_PATH.to_string(),
            reader: Box::new(MemoryReader::new(std::iter::empty::<(String, String)>())),
            script_resolver: Box::new(NoScriptResolver),
            single_threaded: false,
            raw_transform: None,
        };
        let err = build(plan).expect_err("a missing world must be a load error");
        assert!(
            matches!(err, BootError::WorldLoad(_)),
            "{label}: expected WorldLoad, got {err:?}"
        );
    }
    crate::content_ledger::reset();
}
