//! The native windowed authoritative host, end to end (issue #1121).
//!
//! An *integration* test, not an inline `mod tests`, for the reason
//! `tests/headless_runner.rs` and `tests/native_host.rs` are: building a native
//! host populates the process-global native entity-template cache
//! (`config_cache::insert_native_config`), and inside the lib test binary that
//! would leak into thousands of unrelated unit tests. Anything calling
//! `insert_native_config` belongs here (AGENTS.md).
//!
//! It runs against the REPOSITORY'S OWN content — `assets/worlds/combat_test.toml`
//! and the hulls that world offers — because the claims worth pinning are about
//! shipped content actually booting.
//!
//! # What is NOT here, and why
//!
//! Issue #1121's acceptance criterion "browser clients join, claim Stations,
//! ready and play through the same protocol, admission and projection
//! contracts" is **deferred to issue #1112**, the Phoenix WebRTC transport that
//! replaces PeerJS. PeerJS is browser JavaScript: it cannot run in a native
//! process at all, which is why #1121 is filed as blocked by #1112. So the
//! Windows end-to-end test with a real browser participant is deferred with it.
//!
//! What is testable now, and is tested below, is everything that criterion
//! rests on: a participant entering through the transport **seam**
//! (`native_host::transport`) is admitted, welcomed, given a station and
//! projected to by exactly the code a phone goes through — the transport is the
//! only missing link, not the contracts. See
//! `a_participant_joins_and_claims_a_station_through_the_transport_seam`.
//!
//! The native *render* proof is `tests/native_viewscreen_render.rs`, which
//! needs a GPU and is `#[ignore]`d for that reason.

use bevy::prelude::*;

use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::core::messages::{ClientMessage, GamePhase, ServerMessage};
use project_phoenix::entities::template_preload::TemplatePreload;
use project_phoenix::lobby::handler::Target;
use project_phoenix::native_host::transport::{LoopbackHandle, NativeTransportLink};
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig, NativeHostError,
};

/// The flagship scenario, and the one the curated public catalogue publishes.
const WORLD: &str = "assets/worlds/combat_test.toml";
/// A fixed seed, so a digest comparison compares a simulation rather than two
/// draws from the OS.
const SEED: u64 = 20260894;

/// Populate the process-global template cache from the repository's own tree.
///
/// Every test here needs it and it is idempotent (the cache is a map keyed by
/// template path), so each test simply calls it; cargo runs this file as its
/// own process, so nothing outside it sees the result.
fn preload() -> TemplatePreload {
    preload_content_templates(".").expect("the repository's own content preloads")
}

/// A solo native host over `WORLD`: no window (this runner has no GPU), the
/// mission started with nobody connected, every station on `Backfill`.
fn solo_config() -> NativeHostConfig {
    let mut cfg = NativeHostConfig::new(WORLD);
    cfg.seed = Some(SEED);
    cfg.solo = true;
    cfg.surface = NativeRenderSurface::Contract;
    cfg
}

/// Pump `app` for `frames` frames of fixed virtual time, exactly as the
/// headless harness does — `ManualDuration` makes every clock advance by `dt`
/// per `update()` regardless of wall clock, which is what lets two apps be
/// compared at all.
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

#[test]
fn one_executable_loads_ordinary_phoenix_content_and_runs_the_mission() {
    // Acceptance criterion 1, at its most direct: the repository's own shipped
    // world and hull, through the ordinary loaders, into a running mission.
    let preload = preload();
    let mut app =
        build_native_host_app(&solo_config(), &preload).expect("the native host assembles");
    pump(&mut app, 120);

    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "a solo native host starts the mission with nobody connected"
    );
    let local_ships = app
        .world_mut()
        .query::<&project_phoenix::server_app::LocalShip>()
        .iter(app.world())
        .count();
    assert_eq!(local_ships, 1, "the player's hull is in the world");
    // And the simulation really advanced, rather than sitting on tick zero.
    assert!(
        app.world().resource::<project_phoenix::sim_tick::SimTick>().0 > 0,
        "the fixed logical tick advanced"
    );
}

#[test]
fn the_chosen_hulls_own_configuration_reaches_the_client_config_not_the_default() {
    // The config-cache trap issue #1121 closes, asserted through the observable
    // it corrupts. `lobby::server::update_session_with_config` reads the
    // selected hull straight out of the native template cache with NO
    // filesystem fallback; on a miss it silently keeps a DEFAULT
    // `ShipClientConfig` — default helm radar range, default impulse-charge
    // duration — and the mission runs on looking entirely plausible.
    //
    // So: assert the resource carries the HULL's authored numbers, read from
    // the hull's own TOML rather than pinned here, and assert they are not
    // simply the defaults (which would make the test pass on the bug).
    let preload = preload();
    let mut cfg = solo_config();
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");
    // `update_session_with_config` — the reader with no filesystem fallback —
    // runs at `Startup`, so the claim is about a host that has actually run.
    pump(&mut app, 4);

    let selected = app
        .world()
        .resource::<project_phoenix::lobby::SelectedShipResource>()
        .0
        .clone();
    let hull = project_phoenix::entities::include_resolve::load_entity_config(&selected)
        .expect("the selected hull parses");
    let authored_range = hull
        .helm_console
        .as_ref()
        .map(|hc| hc.effective_radar_range())
        .unwrap_or_default();
    assert!(
        authored_range > 0.0,
        "{selected} must author a helm radar range for this test to mean anything"
    );

    let live = app
        .world()
        .resource::<project_phoenix::lobby::server::ShipClientConfigResource>()
        .0
        .clone();
    assert_eq!(
        live.helm_radar_range, authored_range,
        "the hull's authored helm radar range must reach the client config; the \
         default would mean the native template cache was never populated"
    );
    assert_ne!(
        live.helm_radar_range,
        project_phoenix::core::messages::ShipClientConfig::default().helm_radar_range,
        "{selected}'s authored range coincides with the default, so this test \
         cannot tell a populated cache from an empty one — pick another hull"
    );

    // And the same thing said structurally: naming a hull explicitly is honoured.
    cfg.ship_path = Some(selected);
    let _ = build_native_host_app(&cfg, &preload).expect("an explicit hull assembles");
}

#[test]
fn a_participant_joins_and_claims_a_station_through_the_transport_seam() {
    // Acceptance criterion 3's contracts, minus the transport itself (issue
    // #1112, deferred — see this file's header). A participant arriving through
    // the seam is an ordinary session token going through the ordinary
    // `Identify` handler, is answered by the ordinary lobby broadcast, and is
    // projected to through `Audience`/`SessionManager::holder_for_station` — no
    // native-only path, no privilege, nothing to bypass.
    let preload = preload();
    let mut cfg = solo_config();
    // NOT solo: the point is that a participant readies the mission, exactly as
    // one does against a browser host.
    cfg.solo = false;
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");

    let handle = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(handle.transport()));

    const TOKEN: &str = "3f1a6c2e-0a11-4b3c-9d55-000000000001";
    handle.send(
        TOKEN,
        ClientMessage::Identify {
            token: TOKEN.to_string(),
            name: "Ada".to_string(),
        },
    );
    pump(&mut app, 8);

    let dispatched = handle.drain_outbound();
    let welcomed = dispatched.iter().any(|(target, msg, _)| {
        matches!(msg, ServerMessage::Welcome { .. }) && target == &Target::Token(TOKEN.to_string())
    });
    assert!(
        welcomed,
        "the participant must be Welcomed on their own token: {:?}",
        dispatched
            .iter()
            .map(|(t, m, _)| (t.clone(), std::mem::discriminant(m)))
            .collect::<Vec<_>>()
    );

    // They now hold a session, which is what every audience projection resolves
    // through — the same one a phone gets.
    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    assert!(
        sessions.0.players().iter().any(|p| p.token == TOKEN),
        "an ordinary session token registers a session"
    );
}

#[test]
fn the_host_operators_reserved_token_cannot_join_through_the_seam() {
    // `__local_console__` skips the station-tenure branch of
    // `is_command_authorized` entirely and carries host mission-abort
    // authority. The browser refuses it at its own PeerJS ingress as well as
    // inside `handle_identify`; the native ingress must do the same, or the
    // in-process path issue #1122 builds on is a hole the network path is not.
    let preload = preload();
    let mut cfg = solo_config();
    cfg.solo = false;
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");

    let handle = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(handle.transport()));
    const RESERVED: &str = "__local_console__";
    handle.send(
        RESERVED,
        ClientMessage::Identify {
            token: RESERVED.to_string(),
            name: "impostor".to_string(),
        },
    );
    pump(&mut app, 8);

    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    assert!(
        !sessions.0.players().iter().any(|p| p.token == RESERVED),
        "a reserved token must never become a session"
    );
    assert!(
        !handle
            .drain_outbound()
            .iter()
            .any(|(_, msg, _)| matches!(msg, ServerMessage::Welcome { .. })),
        "and must never be welcomed"
    );
}

#[test]
fn a_content_tree_with_no_templates_is_refused_rather_than_silently_defaulted() {
    // The loud half of the config-cache fix. A preload that caches nothing is
    // the worst possible way to report a wrong `--content-dir`: every
    // cache-only reader would answer `Default` and the host would run a
    // plausible mission with the wrong numbers. This is deterministic
    // regardless of what the rest of this file has already put in the
    // process-global cache, because it is the WALK that fails, before the cache
    // is consulted at all.
    let empty = std::env::temp_dir().join("phoenix-native-host-empty-content");
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(empty.join("assets/entities")).expect("fixture tree");
    let err = preload_content_templates(&empty.to_string_lossy())
        .expect_err("an empty content tree must not preload successfully");
    assert!(
        matches!(err, NativeHostError::Content(_)),
        "expected a content error, got {err:?}"
    );
    assert!(
        err.to_string().contains("no entity templates"),
        "the refusal must say what was wrong: {err}"
    );

    // A directory that is not there at all is refused the same way.
    let missing = empty.join("nope");
    assert!(preload_content_templates(&missing.to_string_lossy()).is_err());

    let _ = std::fs::remove_dir_all(&empty);
}

/// The native↔headless authoritative equivalence check (acceptance criterion 5).
///
/// Gated on `headless` because that is where the other half lives; CI runs
/// `cargo test --workspace --features headless`, so it runs there.
///
/// What it proves is precise and worth stating: the native host composes the
/// simulation with `render: true`, which registers a pile of presentation state
/// headless never sees — `RenderInterp`, `ProceduralMeshCache`, `RenderTuning`,
/// `AssetPreloadResource`, the star and planet renderers, the viewscreen radar
/// and the reference grid. Every one of those is declared `Presentation` or
/// `DeferredFold` in the authoritative census. If any of them were to touch
/// authoritative state, this digest would move. Same world, same seed, same
/// frame clock, byte-identical answer.
///
/// It is deliberately a comparison against **headless** rather than against the
/// browser: `src/cross_target_probe.rs` already pins native↔wasm equivalence
/// for the simulation crate, tick by tick, against a committed ledger, and it
/// does so from Rust literals with no filesystem precisely so the two targets
/// are comparable. Adding a boot profile does not change what that probe tests,
/// and re-blessing its ledger to accommodate a native host would be exactly the
/// move its own header forbids.
#[cfg(feature = "headless")]
#[test]
fn a_native_host_and_a_headless_run_agree_on_the_authoritative_digest() {
    use project_phoenix::core::telemetry::RunTelemetry;
    use project_phoenix::headless::report::{collect_balance_events, collect_outbound};
    use project_phoenix::headless::{build_headless_app, HeadlessArgs};
    use project_phoenix::sim_digest::world_digest;

    const FRAMES: u64 = 240;
    let preload = preload();

    // The native host picks the world's first `available_ships` entry; name the
    // same hull to headless so the two are flying the same ship.
    let cfg = solo_config();
    let mut native = build_native_host_app(&cfg, &preload).expect("the native host assembles");
    let ship = native
        .world()
        .resource::<project_phoenix::lobby::SelectedShipResource>()
        .0
        .clone();

    // The one piece of scaffolding this comparison needs, and it is the
    // OBSERVER rather than the simulation. `sim_digest::fold_collisions` reads
    // `RunTelemetry` — the batch runner's collision tracer — and deliberately
    // folds "absent" and "present but empty" as different numbers. A live host
    // produces no run report and so carries no `RunTelemetry`; headless always
    // does. Installing the same collector on both sides is what makes the two
    // digests comparable at all, and it adds nothing to `FixedUpdate`: both
    // systems run in `Last` and only read.
    native.insert_resource(RunTelemetry::default()).add_systems(
        Last,
        (collect_outbound, collect_balance_events).chain(),
    );

    pump(&mut native, FRAMES);

    let mut headless = build_headless_app(&HeadlessArgs {
        world_path: WORLD.to_string(),
        ship_path: ship,
        seed: Some(SEED),
        max_ticks: FRAMES,
        ..Default::default()
    })
    .expect("the headless app assembles");
    // `build_headless_app` already installs `ManualDuration` at the same `dt`
    // this uses; re-inserting it is a no-op that keeps the two loops identical.
    pump(&mut headless, FRAMES);

    assert_eq!(
        world_digest(native.world()),
        world_digest(headless.world()),
        "a rendered native host and a headless run of the same world and seed \
         must reach the same authoritative state — the presentation plugins \
         `render: true` adds are declared Presentation/DeferredFold and must \
         not fold into the digest"
    );
}
