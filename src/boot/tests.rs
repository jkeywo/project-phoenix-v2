//! Guard tests for the boot seam (issue #1217; fourth profile #1121).
//!
//! The headline is the profile registration-parity test: the whole point of
//! the module is that Headless, BrowserHost, BrowserAutomation and NativeHost
//! cannot drift on what the renderer (real or surrogate) owes the simulation, so
//! a test builds all four and asserts they land on the same
//! four-asset/three-message floor — and that only the two render-stack profiles
//! took the real render-stack path.
//!
//! Everything runs off an in-memory world fixture, so no filesystem, GPU, browser
//! or window is involved and the tests are native-`cargo test` clean. NativeHost
//! is built with [`NativeRenderSurface::Contract`] for that reason, exactly as
//! BrowserHost is composed here without wgpu: a GPU-less runner cannot stand up
//! `RenderPlugin`, and what these tests assert is the *inventory*, not the
//! device. The native render path's own proof is
//! `tests/native_viewscreen_render.rs`, which draws a frame on a real GPU.

use super::{
    build, BootError, BootPlan, BootProfile, NativeRenderSurface, RenderStackApplied,
    RenderSurrogateApplied, WorldIngest,
};
use bevy::prelude::*;

use crate::console_bridge::{AiChatterEvent, HudStateChanged, LobbyStateChanged};
use crate::world::load::MemoryReader;
use crate::world::script::load::ScriptResolver;
use crate::world::server::PreCompiledScripts;

/// A resolver that serves no sibling scripts — fixtures using it author scripts
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
const CHILD_PATH: &str = "boot_child.toml";
const ROOT_WITH_CHILD: &str = "extra_worlds = [\"boot_child.toml\"]\n[global]\nseed = 1\n";

/// A fresh [`BootPlan`] for `profile` over `world` — fresh because the boxed reader
/// is consumed by [`build`], so each build needs its own.
fn plan_with(profile: BootProfile, world: &str) -> BootPlan {
    BootPlan {
        profile,
        world_ingest: WorldIngest::FromReader,
        log_filter: "warn".to_string(),
        world_path: WORLD_PATH.to_string(),
        reader: Box::new(MemoryReader::new([(WORLD_PATH, world)])),
        script_resolver: Box::new(NoScriptResolver),
        single_threaded: false,
        raw_transform: None,
        native_surface: NativeRenderSurface::Contract,
    }
}

/// A fresh plan for `profile` over the clean world.
fn plan_for(profile: BootProfile) -> BootPlan {
    plan_with(profile, CLEAN_WORLD)
}

fn plan_with_child(profile: BootProfile, child: &str) -> BootPlan {
    BootPlan {
        profile,
        world_ingest: WorldIngest::FromReader,
        log_filter: "warn".to_string(),
        world_path: WORLD_PATH.to_string(),
        reader: Box::new(MemoryReader::new([
            (WORLD_PATH, ROOT_WITH_CHILD),
            (CHILD_PATH, child),
        ])),
        script_resolver: Box::new(NoScriptResolver),
        single_threaded: false,
        raw_transform: None,
        native_surface: NativeRenderSurface::Contract,
    }
}

const PROFILES: [(BootProfile, &str); 4] = [
    (BootProfile::Headless, "headless"),
    (BootProfile::BrowserHost, "browser-host"),
    (BootProfile::BrowserAutomation, "browser-automation"),
    (BootProfile::NativeHost, "native-host"),
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
fn all_four_profiles_register_the_same_asset_and_message_floor() {
    for (profile, label) in PROFILES {
        let app = build(plan_for(profile)).unwrap_or_else(|e| panic!("{label} build failed: {e}"));
        assert_render_contract(&app, label);
    }
    crate::content_ledger::reset();
}

#[test]
fn the_render_stack_is_taken_only_by_the_profiles_that_name_a_renderer() {
    let headless = build(plan_for(BootProfile::Headless)).expect("headless build");
    let host = build(plan_for(BootProfile::BrowserHost)).expect("browser-host build");
    let automation =
        build(plan_for(BootProfile::BrowserAutomation)).expect("browser-automation build");
    let native = build(plan_for(BootProfile::NativeHost)).expect("native-host build");

    // The two render-stack profiles drove it and NOT the surrogate. That the
    // native one composed the *contract* here (this runner has no GPU) is the
    // `NativeRenderSurface` axis, not the profile axis: the profile still names
    // a renderer, which is what the marker records.
    for (app, label) in [(&host, "browser-host"), (&native, "native-host")] {
        assert!(
            app.world().contains_resource::<RenderStackApplied>(),
            "{label} must take the render-stack path"
        );
        assert!(
            !app.world().contains_resource::<RenderSurrogateApplied>(),
            "{label} must not also take the surrogate path"
        );
    }

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
fn a_native_host_refuses_to_boot_a_world_whose_templates_are_not_in_the_native_cache() {
    // The trap issue #1121 closes. `boot::build` runs no template preload of
    // its own — the adapters do — and every cache-only reader reads it with
    // NO filesystem fallback, the worst being
    // `lobby::server::update_session_with_config`: on a miss it silently keeps a
    // DEFAULT `ShipClientConfig` (default helm radar range, default
    // impulse-charge duration, default hostile-arc colour) and the mission runs
    // on, looking plausible, with nothing in the log.
    //
    // The hull below exists on disk, so the world composes and validates
    // cleanly; what it is not is *cached*. Nothing in the lib test binary
    // populates the native cache with an `assets/entities/…` key (AGENTS.md
    // confines `insert_native_config` to integration tests, and the handful of
    // unit tests that do call it use `fixture/…` keys), so this is the genuine
    // configless boot.
    const HULL: &str = "assets/entities/alliance_destroyer.toml";
    let world = format!("[global]\nseed = 1\n\n[[available_ships]]\ntemplate_path = \"{HULL}\"\n");

    // Every other profile boots it: they read a different cache (the browser's
    // JS preload) or populate this one themselves before boot is called.
    for (profile, label) in PROFILES {
        let result = build(plan_with(profile, &world));
        match profile {
            BootProfile::NativeHost => {
                let err = result.expect_err("a native host must refuse a configless boot");
                assert!(
                    matches!(err, BootError::NativeTemplatesMissing(_)),
                    "expected NativeTemplatesMissing, got {err:?}"
                );
                assert!(
                    err.to_string().contains(HULL),
                    "the refusal must name the template that is missing: {err}"
                );
            }
            _ => {
                assert!(
                    result.is_ok(),
                    "{label} must be unaffected by the native template cache"
                );
            }
        }
    }
    crate::content_ledger::reset();
}

#[test]
fn the_native_template_gate_covers_a_hull_declared_only_by_a_static_child() {
    // The gate's set must be the COMPOSED world, not just its root.
    // `ingest_world` eager-records the declared entities of the root AND of
    // every `extra_worlds` child, so boot itself already treats a child's
    // templates as part of this world's declared content — and the cache-only
    // readers cannot tell which file declared the hull they are about to answer
    // `Default` for.
    //
    // The root below declares nothing; only the child names the hull, and the
    // hull is real on disk (so the composition validates) but is not in the
    // native cache.
    const HULL: &str = "assets/entities/alliance_cruiser.toml";
    let child = format!("[global]\nseed = 2\n\n[[available_ships]]\ntemplate_path = \"{HULL}\"\n");

    let err = build(plan_with_child(BootProfile::NativeHost, &child))
        .expect_err("a hull declared only by a static child must still be gated");
    assert!(
        matches!(err, BootError::NativeTemplatesMissing(_)),
        "expected NativeTemplatesMissing, got {err:?}"
    );
    assert!(
        err.to_string().contains(HULL),
        "the refusal must name the child's template: {err}"
    );

    // And the other three profiles are unaffected, exactly as they are for a
    // root-declared one.
    for profile in [
        BootProfile::Headless,
        BootProfile::BrowserHost,
        BootProfile::BrowserAutomation,
    ] {
        assert!(
            build(plan_with_child(profile, &child)).is_ok(),
            "{profile:?} must be unaffected by the native template cache"
        );
    }
    crate::content_ledger::reset();
}

#[test]
fn a_broken_world_aborts_headless_but_only_blocks_activation_for_the_browser() {
    // Headless is authoritative: a world whose scripts do not compile aborts the
    // build outright, so it activates zero content. The native host takes the
    // same side, for the same reason from a different direction: it is launched
    // from a command line naming its world, so failing at the prompt beats
    // opening a window onto a lobby that can never start.
    for profile in [BootProfile::Headless, BootProfile::NativeHost] {
        let err = build(plan_with(profile, BROKEN_SCRIPT_WORLD))
            .err()
            .unwrap_or_else(|| panic!("{profile:?} must abort on a broken world"));
        assert!(
            matches!(err, BootError::WorldInvalid(_)),
            "{profile:?}: expected WorldInvalid, got {err:?}"
        );
    }

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
fn a_broken_static_child_script_is_rejected_before_every_profile_can_drop_it() {
    for (profile, label) in PROFILES {
        let err = build(plan_with_child(profile, BROKEN_SCRIPT_WORLD))
            .expect_err("a broken static child has no downstream resource owner");
        assert!(
            matches!(err, BootError::WorldInvalid(_)),
            "{label}: expected WorldInvalid, got {err:?}"
        );
        assert!(
            err.to_string().contains("extra_world[0] scripts"),
            "{label}: the rejection must identify the child script set: {err}"
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
            world_ingest: WorldIngest::FromReader,
            log_filter: "warn".to_string(),
            world_path: WORLD_PATH.to_string(),
            reader: Box::new(MemoryReader::new(std::iter::empty::<(String, String)>())),
            script_resolver: Box::new(NoScriptResolver),
            single_threaded: false,
            raw_transform: None,
            native_surface: NativeRenderSurface::Contract,
        };
        let err = build(plan).expect_err("a missing world must be a load error");
        assert!(
            matches!(err, BootError::WorldLoad(_)),
            "{label}: expected WorldLoad, got {err:?}"
        );
    }
    crate::content_ledger::reset();
}

#[test]
fn host_preloaded_ingest_neither_reads_the_reader_nor_inserts_the_world() {
    use crate::world::config::WorldConfig;

    // A reader carrying nothing: under `FromReader` this is the unreadable-world
    // load error above. `HostPreloaded` must not consult it at all — the host
    // (the browser's JS preload) already ingested the world — so the build
    // succeeds, and boot inserts NEITHER the `WorldConfig` nor the
    // `PreCompiledScripts` (the browser's `WorldPlugin` Startup systems own both).
    let plan = BootPlan {
        profile: BootProfile::BrowserAutomation,
        world_ingest: WorldIngest::HostPreloaded,
        log_filter: "warn".to_string(),
        world_path: "unused-under-host-preloaded.toml".to_string(),
        reader: Box::new(MemoryReader::new(std::iter::empty::<(String, String)>())),
        script_resolver: Box::new(NoScriptResolver),
        single_threaded: false,
        raw_transform: None,
        native_surface: NativeRenderSurface::Contract,
    };
    let app = build(plan).expect("HostPreloaded must build without reading the world");
    assert!(
        !app.world().contains_resource::<WorldConfig>(),
        "HostPreloaded must not insert a WorldConfig — the browser's Startup does"
    );
    assert!(
        !app.world().contains_resource::<PreCompiledScripts>(),
        "HostPreloaded must not insert PreCompiledScripts"
    );
    crate::content_ledger::reset();
}
