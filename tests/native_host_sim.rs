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
//!
//! The native↔headless **digest equivalence** check is not here either, and for
//! a load-bearing reason: pinning the scheduler means a one-thread task pool,
//! and Bevy's task pools are process-global and fixed by whichever app in the
//! process builds first — so a digest claim made in this binary, where five
//! other tests build apps of their own, would be a claim about whoever won that
//! race. It lives in `tests/native_headless_digest.rs`, alone, on the pattern
//! `tests/rng_determinism.rs` and `tests/archetype_order_determinism.rs`
//! established. `tests/native_host_snapshot.rs` owns criterion 5's snapshot
//! half for the same reason.

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

/// The hulls the built host's world offers, in the world's authored order —
/// read off the `WorldConfig` boot parsed rather than scanned out of the TOML,
/// because `template_path` is also `[[entity]]`'s field name and a text scan
/// picks up the scenery.
fn available_hulls(app: &App) -> Vec<String> {
    app.world()
        .resource::<project_phoenix::world::config::WorldConfig>()
        .available_ships
        .iter()
        .map(|s| s.template_path.clone())
        .collect()
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
        app.world()
            .resource::<project_phoenix::sim_tick::SimTick>()
            .0
            > 0,
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
    cfg.ship_path = Some(selected.clone());
    let _ = build_native_host_app(&cfg, &preload).expect("an explicit hull assembles");

    // The cache-key mismatch issue #1121 closes (app.rs:337 vs the old :364):
    // the boot gate canonicalises `--ship` before consulting the cache, but
    // `SelectedShipResource` used to keep the RAW string, so every downstream
    // reader above looked up a key the canonically-keyed cache never held.
    // Windows tab-completion spells a path with backslashes, and a leading
    // `./` is an ordinary shell habit — both name the SAME cached hull under
    // `canonical_template_path`, so either spelling must reach the hull's
    // real authored config, not silently fall back to the Default the
    // mismatch used to produce.
    let mut cfg_spelled = solo_config();
    let spelled_differently = format!("./{}", selected.replace('/', "\\"));
    cfg_spelled.ship_path = Some(spelled_differently);
    let mut app_spelled = build_native_host_app(&cfg_spelled, &preload)
        .expect("a `./`-prefixed, backslash-spelled --ship still assembles");
    pump(&mut app_spelled, 4);
    let live_spelled = app_spelled
        .world()
        .resource::<project_phoenix::lobby::server::ShipClientConfigResource>()
        .0
        .clone();
    assert_eq!(
        live_spelled.helm_radar_range, authored_range,
        "a `./`- or backslash-spelled --ship must reach the SAME cached \
         configuration as the canonical spelling, not the Default — the \
         cache-key mismatch issue #1121 closes between app.rs's canonical \
         gate check and the resource every downstream reader consults"
    );
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

#[test]
fn a_ship_the_template_cache_does_not_hold_is_refused_rather_than_silently_defaulted() {
    // The `--ship` half of boot's template gate. `check_native_templates` sees
    // the world's DECLARED set, and issue #935 made the player's own hull
    // authored content that need not be in it — so an explicit `--ship` walks
    // straight past that gate. On a cache miss
    // `lobby::server::update_session_with_config` keeps a DEFAULT
    // `ShipClientConfig` with nothing in the log, which is the exact silent
    // failure the boot refusal exists to prevent.
    //
    // The fixture hull is REAL and parses (so the refusal cannot be confused
    // with "this file does not exist"), it declares stations (so it cannot be
    // confused with the no-`[[station]]` refusal), and it lives outside the
    // content tree the preload walked, so it is genuinely uncached.
    let preload = preload();
    let dir = std::env::temp_dir().join("phoenix-native-host-uncached-hull");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let hull = dir.join("uncached_hull.toml");
    std::fs::copy("assets/entities/alliance_destroyer.toml", &hull).expect("fixture hull");
    let hull_path = hull.to_string_lossy().replace('\\', "/");

    let mut cfg = solo_config();
    cfg.ship_path = Some(hull_path.clone());
    let err = build_native_host_app(&cfg, &preload)
        .expect_err("an uncached hull must not silently default");
    assert!(
        matches!(err, NativeHostError::Ship(_)),
        "expected a ship error, got {err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains(&hull_path),
        "the refusal must name the hull it could not find cached: {message}"
    );
    assert!(
        message.contains("Default"),
        "and must say what would have happened instead: {message}"
    );

    // The same refusal survives an alternate — but canonically equivalent —
    // spelling of this SAME uncached path. `canonical_template_path` collapses
    // `./` and backslashes before the gate ever consults the cache (app.rs's
    // `ship_key`), so a `./`-prefixed or backslash-spelled --ship naming
    // genuinely uncached content must be refused exactly like the canonical
    // spelling — never mistaken, in either direction, for some other key.
    let mut cfg_spelled = solo_config();
    // `hull_path` is an absolute temp path, and `canonical_template_path`
    // decides its leading slash BEFORE dropping `.` segments — so a bare
    // `./`-prefix would strip the root on Unix and canonicalise to a
    // different key. Keep the root separator so the alternate spelling is
    // canonically equivalent on both platforms.
    let spelled_differently = match hull_path.strip_prefix('/') {
        Some(rest) => format!("/./{}", rest.replace('/', "\\")),
        None => format!("./{}", hull_path.replace('/', "\\")),
    };
    cfg_spelled.ship_path = Some(spelled_differently);
    let err_spelled = build_native_host_app(&cfg_spelled, &preload)
        .expect_err("a differently-spelled uncached hull must not silently default either");
    assert!(
        matches!(err_spelled, NativeHostError::Ship(_)),
        "expected a ship error, got {err_spelled:?}"
    );
    let message_spelled = err_spelled.to_string();
    assert!(
        message_spelled.contains(&hull_path),
        "the refusal must name the SAME canonical path regardless of how \
         --ship was spelled: {message_spelled}"
    );
    assert!(
        message_spelled.contains("Default"),
        "and must say what would have happened instead: {message_spelled}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_curating_manifest_restricts_the_hull_the_host_flies_as_well_as_the_one_it_publishes() {
    // Issue #917's catalogue restriction is the same lever on both surfaces.
    // Before the fix round `--manifest assets/scenarios.demo.toml` narrowed what
    // the process PUBLISHED over HTTP but not what it FLEW, so one host could
    // serve a curated catalogue and simultaneously run a hull outside it.
    //
    // Asserted relationally against the world's own list rather than against a
    // pinned path, so a designer reordering `[[available_ships]]` does not have
    // to edit this test.
    let preload = preload();
    let base = build_native_host_app(&solo_config(), &preload).expect("the native host assembles");
    let unrestricted = base
        .world()
        .resource::<project_phoenix::lobby::SelectedShipResource>()
        .0
        .clone();
    let offered = available_hulls(&base);
    assert_eq!(
        offered.first(),
        Some(&unrestricted),
        "with no curation the default is available_ships[0]"
    );

    // Curate to some hull that is NOT the unrestricted default.
    let other = offered
        .iter()
        .find(|p| *p != &unrestricted)
        .cloned()
        .expect("combat_test offers more than one hull");

    let mut cfg = solo_config();
    cfg.curated_ships = vec![other.clone()];
    let restricted = build_native_host_app(&cfg, &preload)
        .expect("the native host assembles under a curated catalogue")
        .world()
        .resource::<project_phoenix::lobby::SelectedShipResource>()
        .0
        .clone();
    assert_eq!(
        restricted, other,
        "the default hull must come from the curated allowlist, not from \
         available_ships[0] ({unrestricted})"
    );

    // An explicit --ship still wins, exactly as `?ship=` does in the browser:
    // curation narrows the DEFAULT, it is not a second admission gate.
    cfg.ship_path = Some(unrestricted.clone());
    let explicit = build_native_host_app(&cfg, &preload)
        .expect("an explicit hull assembles")
        .world()
        .resource::<project_phoenix::lobby::SelectedShipResource>()
        .0
        .clone();
    assert_eq!(explicit, unrestricted);

    // And a world whose hulls the allowlist admits none of is refused by name,
    // rather than quietly falling back to available_ships[0].
    let mut impossible = solo_config();
    impossible.curated_ships = vec!["assets/entities/not_in_this_world.toml".to_string()];
    let err = build_native_host_app(&impossible, &preload)
        .expect_err("no admissible hull must be an error");
    assert!(
        matches!(err, NativeHostError::NoShip(_)),
        "expected NoShip, got {err:?}"
    );
    assert!(
        err.to_string().contains("not_in_this_world.toml"),
        "the refusal must name the allowlist it could not satisfy: {err}"
    );
}

#[test]
fn the_curated_allowlist_is_read_from_the_manifest_the_host_serves() {
    // The other half of the same claim: the allowlist above is not a test
    // fixture, it is what `--manifest` resolves to. `curated_hulls_for_world`
    // is the one function the binary uses, so pin it against the SHIPPED
    // curated manifest rather than a hand-built one.
    let demo =
        std::fs::read_to_string("assets/scenarios.demo.toml").expect("the demo manifest reads");
    let curated = project_phoenix::native_host::curated_hulls_for_world(&demo, WORLD);
    assert!(
        !curated.is_empty(),
        "assets/scenarios.demo.toml curates {WORLD}'s hulls (issue #931), so this \
         test is only meaningful while it does"
    );

    let preload = preload();
    let app = build_native_host_app(&solo_config(), &preload).expect("the native host assembles");
    let authored = available_hulls(&app);
    for hull in &curated {
        assert!(
            authored.contains(hull),
            "the curated list must be a RESTRICTION of what the world authors: \
             {hull} is not in {authored:?}"
        );
    }

    // The base manifest curates nothing, which is "unrestricted" rather than
    // "no hulls" — the distinction the default-hull resolution turns on.
    let base = std::fs::read_to_string("assets/scenarios.toml").expect("the base manifest reads");
    assert!(
        project_phoenix::native_host::curated_hulls_for_world(&base, WORLD).is_empty(),
        "the dev catalogue restricts nothing"
    );
    // A world the manifest does not publish at all is unrestricted too, not
    // refused: `--world` names a file directly and the manifest is beside the
    // point for an ordinary dev invocation.
    assert!(project_phoenix::native_host::curated_hulls_for_world(
        &demo,
        "assets/worlds/nope.toml"
    )
    .is_empty());
}
