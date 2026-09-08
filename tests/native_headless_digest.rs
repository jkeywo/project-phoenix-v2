//! The native↔headless authoritative equivalence check (issue #1121,
//! acceptance criterion 5, content half).
//!
//! # Why this is its own test binary
//!
//! The same reason `tests/archetype_order_determinism.rs`,
//! `tests/rng_determinism.rs`, `tests/registration_order_determinism.rs` and
//! `tests/snapshot_resume.rs` are, and it is not a style preference: pinning the
//! scheduler means handing `TaskPoolPlugin` a one-thread `TaskPoolOptions`, and
//! **Bevy's task pools are process-global and fixed by whichever app in the
//! process builds first**. A digest-equality claim made in a binary shared with
//! other tests is a claim about whoever won that race. Both apps below ask for
//! the single-threaded pool and nothing else in this binary builds an app, so
//! the pool the digest is taken on is genuinely the pinned one.
//!
//! `BootPlan::single_threaded` is what carries that request: `deterministic` on
//! [`NativeHostConfig`] and `deterministic` on `HeadlessArgs` both reach it.
//! Neither default is `true` — `HeadlessArgs::default()` leaves it `false` and
//! only `parse_args`' `--seed`/`--deterministic` rule sets it — so both sides
//! set it explicitly here.
//!
//! # What the comparison proves, and what it does not
//!
//! The native host composes the simulation with `render: true`, which registers
//! a pile of presentation state headless never sees — `RenderInterp`,
//! `ProceduralMeshCache`, `RenderTuning`, `AssetPreloadResource`, the star and
//! planet renderers, the viewscreen radar and the reference grid. Every one of
//! those is declared `Presentation` or `DeferredFold` in the authoritative
//! census. If any of them were to touch authoritative state, this digest would
//! move. Same world, same seed, same frame clock, byte-identical answer.
//!
//! **Scope, stated rather than implied.** The default test runs under
//! [`NativeRenderSurface::Contract`], which composes `core_plugins` plus the
//! renderer's *contract* — no `DefaultPlugins`, no wgpu, no `RendererPlugin`, no
//! glTF loader. So it covers "the `render: true` SIMULATION plugins do not fold
//! into the digest" and not "the wgpu render stack does not fold into it": the
//! real stack inserts main-world components of its own at extract
//! (`bevy_render`'s `entity_sync_system` and its `RenderEntity`), which the
//! Contract composition cannot observe. The second half is
//! [`the_wgpu_render_stack_does_not_move_the_authoritative_digest_either`], the
//! same 240-frame comparison under [`NativeRenderSurface::Offscreen`] — a real
//! wgpu device — `#[ignore]`d because CI has no GPU, for the same reason
//! `tests/native_viewscreen_render.rs` is:
//!
//! ```text
//! cargo test --features headless --test native_headless_digest -- --ignored --nocapture
//! ```
//!
//! # Why headless and not the browser
//!
//! `src/cross_target_probe.rs` already pins native↔wasm equivalence for the
//! simulation crate, tick by tick, against a committed ledger, and it does so
//! from Rust literals with no filesystem precisely so the two targets are
//! comparable. Adding a boot profile does not change what that probe tests, and
//! re-blessing its ledger to accommodate a native host would be exactly the move
//! its own header forbids.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::core::telemetry::RunTelemetry;
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig,
};
use project_phoenix::sim_digest::world_digest;

/// The flagship scenario, and the one the curated public catalogue publishes.
const WORLD: &str = "assets/worlds/combat_test.toml";
/// A fixed seed, so a digest comparison compares a simulation rather than two
/// draws from the OS.
const SEED: u64 = 20260894;
/// Long enough that the mission has started, ships have closed and the belts
/// have streamed — a comparison over an empty lobby would agree trivially.
const FRAMES: u64 = 240;

/// Pump `app` for `frames` frames of fixed virtual time, exactly as the headless
/// harness does — `ManualDuration` makes every clock advance by `dt` per
/// `update()` regardless of wall clock, which is what lets two apps be compared
/// at all.
fn pump(app: &mut App, frames: u64) {
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    ));
    app.finish();
    app.cleanup();
    for _ in 0..frames {
        app.update();
    }
}

/// A solo, pinned native host over `WORLD` on `surface`.
fn native_config(surface: NativeRenderSurface) -> NativeHostConfig {
    let mut cfg = NativeHostConfig::new(WORLD);
    cfg.seed = Some(SEED);
    cfg.solo = true;
    cfg.surface = surface;
    // The whole reason this file exists: without it both sides run on the
    // multithreaded default pool and the equality below is a race.
    cfg.deterministic = true;
    cfg
}

/// Build both apps, run them the same number of frames, and assert the digests
/// agree. Parameterised by surface so the Contract claim and the wgpu claim are
/// the same comparison rather than two similar ones.
fn assert_native_and_headless_agree(surface: NativeRenderSurface) {
    let preload = preload_content_templates(".").expect("the repository's own content preloads");

    let cfg = native_config(surface);
    let mut native = build_native_host_app(&cfg, &preload).expect("the native host assembles");
    // The native host picks the world's first `available_ships` entry; name the
    // same hull to headless so the two are flying the same ship.
    let ship = native
        .world()
        .resource::<project_phoenix::lobby::SelectedShipResource>()
        .0
        .clone();

    // Collision attribution belongs to ordinary shared boot on both hosts;
    // an optional headless report is no longer prerequisite scaffolding.
    assert!(!native.world().contains_resource::<RunTelemetry>());

    pump(&mut native, FRAMES);

    let mut headless = build_headless_app(&HeadlessArgs {
        world_path: WORLD.to_string(),
        ship_path: ship,
        seed: Some(SEED),
        max_ticks: FRAMES,
        // `HeadlessArgs::default()` leaves this `false` — the `--seed implies
        // --deterministic` rule lives in `parse_args`, which a struct literal
        // bypasses — so a comparison built this way has to say it.
        deterministic: true,
        ..Default::default()
    })
    .expect("the headless app assembles");
    assert!(headless.world().contains_resource::<RunTelemetry>());
    // `build_headless_app` already installs `ManualDuration` at the same `dt`
    // this uses; re-inserting it is a no-op that keeps the two loops identical.
    pump(&mut headless, FRAMES);

    assert!(
        native
            .world()
            .resource::<project_phoenix::sim_tick::SimTick>()
            .0
            > 0,
        "the native host must actually have simulated, or this comparison is \
         between two worlds at tick zero"
    );
    assert_eq!(
        world_digest(native.world()),
        world_digest(headless.world()),
        "a rendered native host ({surface:?}) and a headless run of the same \
         world and seed must reach the same authoritative state — the \
         presentation plugins `render: true` adds are declared \
         Presentation/DeferredFold and must not fold into the digest"
    );
}

#[test]
fn a_native_host_and_a_headless_run_agree_on_the_authoritative_digest() {
    assert_native_and_headless_agree(NativeRenderSurface::Contract);
}

/// The same comparison with a **real wgpu device** behind it (issue #1121's fix
/// round).
///
/// The Contract composition above cannot see anything the shipped render stack
/// does to the main world: no `DefaultPlugins`, so no `RenderPlugin`, no
/// extract schedule, and none of the main-world components `bevy_render` adds
/// there (`RenderEntity`, inserted by `entity_sync_system` at every frame's
/// extract). Those are exactly the things a "presentation must not fold into the
/// digest" claim ought to be tested against, and the only composition that has
/// them is one with a GPU under it.
///
/// `Offscreen` rather than `Window`: same wgpu device, same render graph, same
/// `RendererPlugin`, minus the one piece — a winit surface — that cannot be
/// automated. `capture-billboard` and `tune-lods` already drive native wgpu this
/// way.
#[test]
#[ignore = "needs a real GPU adapter; CI has no Windows runner and no display. \
            Run: cargo test --features headless --test native_headless_digest -- --ignored --nocapture"]
fn the_wgpu_render_stack_does_not_move_the_authoritative_digest_either() {
    assert_native_and_headless_agree(NativeRenderSurface::Offscreen);
}
