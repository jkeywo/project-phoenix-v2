//! A native host that boots with **no world** and loads one at runtime
//! (issue #1326).
//!
//! An *integration* test, not an inline `mod tests`, for the reason
//! `tests/native_host_sim.rs` is one: building a native host populates the
//! process-global native entity-template cache (`config_cache::
//! insert_native_config`), and inside the lib test binary that would leak into
//! thousands of unrelated unit tests. Anything calling `insert_native_config`
//! belongs here (AGENTS.md).
//!
//! It runs against the REPOSITORY'S OWN content — `assets/scenarios.toml` and
//! the worlds it lists — because the claim worth pinning is that the shipped
//! catalogue really can be picked from and really does boot.
//!
//! Nothing here opens a window. Selection arrives the way it arrives in
//! production: as `ClientMessage::SelectScenario` / `SelectPlayerShip` on the
//! ordinary transport seam, or written straight onto `Messages<InboundMessage>`
//! where the test is not about the transport.

use bevy::prelude::*;

use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::core::messages::{ClientMessage, GamePhase, ServerMessage};
use project_phoenix::delivery::serve::ManifestSource;
use project_phoenix::entities::template_preload::TemplatePreload;
use project_phoenix::lobby::handler::Target;
use project_phoenix::lobby::scenario_arbiter;
use project_phoenix::lobby::server::InboundMessage;
use project_phoenix::native_host::transport::{LoopbackHandle, NativeTransportLink};
use project_phoenix::native_host::world_load::{LobbyScenarioCatalog, LobbySelection};
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig,
};
use project_phoenix::world::config::WorldConfig;
use project_phoenix::world::manifest::ScenarioCatalog;

/// The base manifest a host publishes with no `--manifest`.
const MANIFEST: &str = "assets/scenarios.toml";
/// The flagship scenario, and the one the curated public catalogue publishes.
const SCENARIO: &str = "combat_test";
/// A fixed seed, so two hosts of one world are comparable.
const SEED: u64 = 20260894;

/// Populate the process-global template cache from the repository's own tree.
/// Idempotent (a map keyed by template path), and cargo runs this file as its
/// own process, so nothing outside it sees the result.
fn preload() -> TemplatePreload {
    preload_content_templates(".").expect("the repository's own content preloads")
}

/// The merged catalogue a `--lobby` host publishes, built exactly as
/// `phoenix-host` builds it.
fn catalog() -> ScenarioCatalog {
    ManifestSource::read(".", MANIFEST)
        .expect("the repository's own scenario manifest reads")
        .merged_catalog()
        .catalog
}

/// A world-less host over the repository's own catalogue.
fn lobby_config() -> NativeHostConfig {
    let mut cfg = NativeHostConfig::lobby(catalog());
    cfg.seed = Some(SEED);
    cfg.surface = NativeRenderSurface::Contract;
    cfg
}

/// Pump `app` for `frames` frames of fixed virtual time, exactly as
/// `tests/native_host_sim.rs` does.
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

/// The scenario the tests pick, and the first hull its catalogue entry offers —
/// read off the catalogue rather than pinned here, so a manifest edit moves the
/// test rather than breaking it.
fn pick() -> (String, String) {
    let catalog = catalog();
    let entry = scenario_arbiter::find_scenario(&catalog, SCENARIO)
        .unwrap_or_else(|| panic!("{MANIFEST} must publish `{SCENARIO}`"));
    let hull = entry
        .ships
        .first()
        .unwrap_or_else(|| panic!("`{SCENARIO}` must offer at least one hull"));
    (entry.id.clone(), hull.template_path.clone())
}

/// Write the two selection messages straight onto the inbound bus, as a
/// participant's would arrive.
fn select(app: &mut App, token: &str, scenario_id: &str, template_path: &str) {
    for msg in [
        ClientMessage::SelectScenario {
            scenario_id: scenario_id.to_string(),
        },
        ClientMessage::SelectPlayerShip {
            template_path: template_path.to_string(),
        },
    ] {
        app.world_mut().write_message(InboundMessage {
            token: token.to_string(),
            msg,
        });
    }
}

/// Every `EntityUuid` in the world, sorted — the identity half of what
/// `sim_digest::fold_entity_namespace` folds, and the thing a runtime load must
/// not change.
fn entity_uuids(app: &mut App) -> Vec<String> {
    let mut ids: Vec<String> = app
        .world_mut()
        .query::<&project_phoenix::entities::spawner::EntityUuid>()
        .iter(app.world())
        .map(|uuid| uuid.0.clone())
        .collect();
    ids.sort();
    ids
}

#[test]
fn a_host_with_no_world_boots_into_an_empty_lobby_holding_the_catalogue() {
    // Acceptance criterion 1. Before #1326 there was no way to ask for an
    // authoritative host without naming a world; `NativeHostConfig::new` was the
    // only constructor and `world_path` was a required `String`.
    let preload = preload();
    let mut app = build_native_host_app(&lobby_config(), &preload)
        .expect("a world-less native host assembles");
    pump(&mut app, 8);

    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "a host with no world waits in the lobby"
    );
    assert!(
        app.world().get_resource::<WorldConfig>().is_none(),
        "nothing has been ingested yet"
    );
    let published = app.world().resource::<LobbyScenarioCatalog>();
    assert!(
        !published.0.scenarios.is_empty(),
        "the merged catalogue is available to pick from"
    );
    assert!(
        scenario_arbiter::find_scenario(&published.0, SCENARIO).is_some(),
        "the shipped catalogue publishes `{SCENARIO}`"
    );
    // And no world means no world entities — the empty lobby is genuinely empty
    // rather than half-populated.
    assert!(
        entity_uuids(&mut app).is_empty(),
        "an empty lobby spawns no simulation entities"
    );
}

#[test]
fn a_selection_pair_loads_the_world_and_the_mission_starts() {
    // Acceptance criterion 2, at its most direct: the two arbitrated messages,
    // then a running mission over the world they named.
    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = true;
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");
    pump(&mut app, 4);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "--solo must NOT start a mission before there is a world to fly"
    );

    let (scenario_id, hull) = pick();
    select(&mut app, "phone-1", &scenario_id, &hull);
    pump(&mut app, 120);

    let world_config = app
        .world()
        .get_resource::<WorldConfig>()
        .expect("the selected world was ingested");
    assert!(
        !world_config.available_ships.is_empty(),
        "the ingested world is the authored one, not an empty stand-in"
    );
    assert_eq!(
        app.world()
            .resource::<project_phoenix::lobby::SelectedShipResource>()
            .0,
        project_phoenix::entities::include_resolve::canonical_template_path(&hull),
        "the arbitrated hull is the one the host flies"
    );
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "the mission starts once a world has been chosen"
    );
    let local_ships = app
        .world_mut()
        .query::<&project_phoenix::server_app::LocalShip>()
        .iter(app.world())
        .count();
    assert_eq!(local_ships, 1, "the player's hull is in the world");
    assert!(
        !entity_uuids(&mut app).is_empty(),
        "the world's own entities spawned"
    );
}

#[test]
fn the_chosen_hulls_own_configuration_reaches_the_client_config_not_the_default() {
    // The config-cache trap, on the runtime path. `update_session_with_config`
    // recomputes the station roster only while it is EMPTY — and a world-less
    // lobby has already run it once at `Startup`, filling both the roster and
    // the client config from `load_ship_config_from_disk`'s battleship fallback.
    // Without the reset `load_selected_world` performs, a runtime-loaded host
    // would fly the chosen hull behind the wrong ship's stations and a Default
    // `ShipClientConfig`: a plausible mission with the wrong numbers.
    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = true;
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");
    pump(&mut app, 4);

    let (scenario_id, hull) = pick();
    select(&mut app, "phone-1", &scenario_id, &hull);
    pump(&mut app, 60);

    let authored = project_phoenix::entities::include_resolve::load_entity_config(&hull)
        .expect("the selected hull parses")
        .helm_console
        .as_ref()
        .map(|hc| hc.effective_radar_range())
        .unwrap_or_default();
    assert!(
        authored > 0.0,
        "{hull} must author a helm radar range for this test to mean anything"
    );
    let live = app
        .world()
        .resource::<project_phoenix::lobby::server::ShipClientConfigResource>()
        .0
        .clone();
    assert_eq!(
        live.helm_radar_range, authored,
        "the runtime-loaded hull's authored range must reach the client config"
    );
    assert_ne!(
        live.helm_radar_range,
        project_phoenix::core::messages::ShipClientConfig::default().helm_radar_range,
        "{hull}'s authored range coincides with the default, so this test cannot \
         tell a re-read config from a stale one — pick another hull"
    );

    // The station roster is the chosen hull's, not the disk fallback's.
    let stations = app
        .world()
        .resource::<project_phoenix::lobby::stations_config::ShipStations>();
    let expected = project_phoenix::entities::include_resolve::load_entity_config(&hull)
        .expect("the selected hull parses")
        .ship_config
        .expect("the selected hull authors [[station]] blocks")
        .stations
        .len();
    assert_eq!(
        stations.stations.len(),
        expected,
        "the lobby's station roster belongs to the hull that was chosen"
    );
}

#[test]
fn a_runtime_load_mints_the_same_world_entity_ids_as_a_boot_load() {
    // The determinism claim, as an observable rather than an argument.
    //
    // `WorldIdMint` stamps every id with the tick it was minted on, and
    // `begin_tick` resets the per-namespace sequences only when the tick MOVES.
    // A boot ingest spawns from `Startup`, where the mint is still at its
    // default (tick 0); a runtime ingest happens at tick N. Without
    // `world_load::park_mint` the same authored world would therefore produce a
    // different set of uuids depending on how long the operator spent choosing
    // it — and those uuids are folded into the authoritative digest BY NAME
    // (`sim_digest::fold_entity_namespace` folds the rendered id), key a
    // snapshot's entity matching, and in a fleet have to agree across hosts.
    let preload = preload();
    let (scenario_id, hull) = pick();
    let world = scenario_arbiter::world_path_for(
        &catalog(),
        &project_phoenix::lobby::scenario_arbiter::ScenarioSelection {
            scenario_id: Some(scenario_id.clone()),
            template_path: None,
        },
    )
    .expect("the picked scenario names a world")
    .to_string();

    // The boot path: `--world`, ingested before the `App` exists.
    let mut boot_cfg = NativeHostConfig::new(world);
    boot_cfg.seed = Some(SEED);
    boot_cfg.ship_path = Some(hull.clone());
    boot_cfg.surface = NativeRenderSurface::Contract;
    let mut booted = build_native_host_app(&boot_cfg, &preload).expect("the boot host assembles");
    pump(&mut booted, 4);

    // The runtime path: the same world and hull, chosen from the lobby.
    let mut runtime =
        build_native_host_app(&lobby_config(), &preload).expect("a world-less host assembles");
    pump(&mut runtime, 4);
    select(&mut runtime, "phone-1", &scenario_id, &hull);
    pump(&mut runtime, 4);

    let booted_ids = entity_uuids(&mut booted);
    assert!(
        !booted_ids.is_empty(),
        "the boot host spawned the world's entities"
    );
    assert_eq!(
        entity_uuids(&mut runtime),
        booted_ids,
        "a world loaded at runtime must mint exactly the ids the same world \
         loaded at boot mints — the mint is parked at tick 0 across the spawn \
         pass precisely so the uuids do not carry the operator's reaction time"
    );
}

#[test]
fn an_unlisted_scenario_is_refused_and_the_host_stays_in_the_lobby() {
    // First-valid-wins means a request that fails catalogue validation locks
    // nothing — `gui/scenario-arbiter.js` returns 'rejected' and `server.html`
    // drops it. A native host must not instead try to read a world path it
    // invented.
    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = true;
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");
    pump(&mut app, 4);

    select(
        &mut app,
        "phone-1",
        "not_a_scenario",
        "assets/entities/alliance_cruiser.toml",
    );
    pump(&mut app, 30);

    assert!(
        app.world().get_resource::<WorldConfig>().is_none(),
        "an unlisted scenario ingests nothing"
    );
    assert_eq!(
        app.world().resource::<LobbySelection>().0,
        project_phoenix::lobby::scenario_arbiter::ScenarioSelection::default(),
        "a rejected request locks nothing"
    );
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "and the host is still waiting for a valid pick"
    );

    // A valid pair after the bad one still works — the host was not poisoned.
    let (scenario_id, hull) = pick();
    select(&mut app, "phone-2", &scenario_id, &hull);
    pump(&mut app, 120);
    assert!(
        app.world().get_resource::<WorldConfig>().is_some(),
        "the next valid selection loads normally"
    );
}

#[test]
fn the_first_valid_selection_wins_over_a_later_one() {
    // The whole arbitration rule, end to end: two participants pick different
    // scenarios in the same tick and the host loads the first one's.
    let catalog = catalog();
    if catalog.scenarios.len() < 2 {
        // Nothing to arbitrate between; the pure rule is covered by
        // `lobby::scenario_arbiter`'s own tests either way.
        return;
    }
    let preload = preload();
    let mut app =
        build_native_host_app(&lobby_config(), &preload).expect("a world-less host assembles");
    pump(&mut app, 4);

    let first = catalog.scenarios[0].clone();
    let second = catalog.scenarios[1].clone();
    for (token, scenario) in [("phone-1", &first), ("phone-2", &second)] {
        app.world_mut().write_message(InboundMessage {
            token: token.to_string(),
            msg: ClientMessage::SelectScenario {
                scenario_id: scenario.id.clone(),
            },
        });
    }
    pump(&mut app, 8);

    assert_eq!(
        app.world()
            .resource::<LobbySelection>()
            .0
            .scenario_id
            .as_deref(),
        Some(first.id.as_str()),
        "the first valid request locks the scenario; the second is ignored"
    );
}

#[test]
fn a_participant_picks_the_scenario_and_readies_the_mission_through_the_ordinary_path() {
    // Acceptance criterion 2's other half: not `--solo`, but the ordinary
    // lobby -> ready -> start flow, driven entirely by messages a phone sends,
    // arriving through the transport seam a phone's really arrive through.
    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = false;
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");

    let handle = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(handle.transport()));

    const TOKEN: &str = "3f1a6c2e-0a11-4b3c-9d55-000000000042";
    handle.send(
        TOKEN,
        ClientMessage::Identify {
            token: TOKEN.to_string(),
            name: "Ada".to_string(),
        },
    );
    pump(&mut app, 8);

    // A participant arriving before a world exists is handed the catalogue, the
    // way `server.html`'s `sendCatalogTo` hands it to a fresh datachannel.
    let dispatched = handle.drain_outbound();
    let catalogued = dispatched.iter().any(|(target, msg, _)| {
        matches!(msg, ServerMessage::ScenarioCatalog { .. })
            && target == &Target::Token(TOKEN.to_string())
    });
    assert!(
        catalogued,
        "a participant that identifies into a world-less lobby is sent the catalogue"
    );

    // They pick, through the transport rather than the bus directly.
    let (scenario_id, hull) = pick();
    handle.send(TOKEN, ClientMessage::SelectScenario { scenario_id });
    handle.send(
        TOKEN,
        ClientMessage::SelectPlayerShip {
            template_path: hull.clone(),
        },
    );
    pump(&mut app, 30);
    assert!(
        app.world().get_resource::<WorldConfig>().is_some(),
        "the selection arriving over the transport loads the world"
    );
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "loading a world does not start the mission — readying does"
    );

    // Now the ordinary ready path: claim a station, ready up, and the collective
    // auto-start takes the host into the mission.
    let station = app
        .world()
        .resource::<project_phoenix::lobby::stations_config::ShipStations>()
        .stations
        .first()
        .expect("the chosen hull authors stations")
        .id
        .clone();
    handle.send(
        TOKEN,
        ClientMessage::SelectStation {
            station: station.0.clone(),
        },
    );
    pump(&mut app, 8);
    assert_eq!(
        app.world()
            .resource::<project_phoenix::lobby::Sessions>()
            .0
            .players()
            .iter()
            .find(|p| p.token == TOKEN)
            .and_then(|p| p.station.clone())
            .map(|s| s.0),
        Some(station.0.clone()),
        "the participant holds the station they claimed on the runtime-loaded hull"
    );

    handle.send(TOKEN, ClientMessage::SetReady { ready: true });
    // Long enough for the pre-game countdown `tick_countdown` runs.
    pump(&mut app, 60 * 15);

    // `Loading`, not `InProgress`, and that is the ordinary path rather than a
    // shortfall of it: collective `SetReady` auto-start takes a host to
    // `GamePhase::Loading`, and the last hop is `asset_preload::
    // auto_transition_from_loading`, which waits for the viewscreen's GLBs to
    // finish streaming. `NativeRenderSurface::Contract` stands up no wgpu device
    // (this runner has no GPU — see `tests/native_viewscreen_render.rs`, which
    // is `#[ignore]`d for exactly that), so that gate never opens here on ANY
    // native host, world-at-boot or world-at-runtime. `--solo` reaches
    // `InProgress` in the tests above because `solo_auto_start` deliberately
    // does not wait for the preloader.
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Loading,
        "the collective-ready auto-start leaves the lobby for the mission"
    );
}

#[test]
fn an_explicit_hull_outranks_the_lobbys_pick() {
    // `--ship` given without `--world` (issue #1326 makes that combination
    // legal) keeps the meaning it has with one: an explicit hull wins, the way
    // `?ship=` does in the browser.
    let catalog = catalog();
    let entry = scenario_arbiter::find_scenario(&catalog, SCENARIO)
        .unwrap_or_else(|| panic!("{MANIFEST} must publish `{SCENARIO}`"));
    if entry.ships.len() < 2 {
        return; // nothing to outrank
    }
    let picked = entry.ships[0].template_path.clone();
    let pinned = entry.ships[1].template_path.clone();

    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = true;
    cfg.ship_path = Some(pinned.clone());
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");
    pump(&mut app, 4);
    select(&mut app, "phone-1", &entry.id, &picked);
    pump(&mut app, 60);

    assert_eq!(
        app.world()
            .resource::<project_phoenix::lobby::SelectedShipResource>()
            .0,
        project_phoenix::entities::include_resolve::canonical_template_path(&pinned),
        "--ship outranks the arbitrated hull"
    );
}
