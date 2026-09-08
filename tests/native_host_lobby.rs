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
use project_phoenix::native_host::host_lobby::{pump_host_lobby, LocalHostLobby};
use project_phoenix::native_host::panes::RecordingSurface;
use project_phoenix::native_host::transport::{LoopbackHandle, NativeTransportLink};
use project_phoenix::native_host::world_load::{
    LobbyScenarioCatalog, LobbySelection, RuntimeWorldLoad,
};
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

#[path = "native_host_lobby/materialization.rs"]
mod materialization;

#[path = "native_host_lobby/round_return.rs"]
mod round_return;

#[path = "native_host_lobby/display_roster.rs"]
mod display_roster;

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

/// Every `EntityUuid` in the world paired with **which entity holds it**,
/// sorted — the identity half of what `sim_digest::fold_entity_namespace`
/// folds, and the thing a runtime load must not change.
///
/// The pairing is the whole point. A bare sorted list of uuids is a multiset and
/// says nothing about *whose* id each one is, so the hazard the
/// `setup_world < spawn_world_entities` pin exists to stop — the two spawn
/// passes swapping places, which permutes ids between the anonymous stars and
/// the named entities without changing the set — walks straight past it, while
/// the authoritative digest (which folds each id against that entity's own
/// physics and hull) diverges.
///
/// The identity is the instance's `EntityId`, else the template's display
/// `EntityName`, else the marker below; the spawn position disambiguates the
/// several entities that share one template. Neither moves in `GamePhase::Lobby`
/// — the simulation sets are gated on `InProgress` — so it is a stable key on
/// both hosts.
///
/// Two residual blind spots, stated rather than papered over. Two entities that
/// are BOTH anonymous (no `EntityId`, no `EntityName`) and at IDENTICAL
/// coordinates collapse onto one `<unnamed>@x,y,z` key, so a swap confined to
/// that pair would not be seen — not a live gap in `combat_test`, whose
/// anonymous spawns are stars and planets at distinct positions, but a real one
/// in a world that authors coincident anonymous entities. And an `EntityUuid`
/// holder with no `Transform` is excluded from this half entirely (the query
/// requires one); named entities are covered by [`world_name_to_uuid`] below
/// regardless of whether they carry a transform.
fn entity_identities(app: &mut App) -> Vec<(String, String)> {
    use project_phoenix::entities::spawner::{EntityId, EntityName, EntityUuid};
    let mut rows: Vec<(String, String)> = app
        .world_mut()
        .query::<(
            &EntityUuid,
            Option<&EntityId>,
            Option<&EntityName>,
            &Transform,
        )>()
        .iter(app.world())
        .map(|(uuid, id, name, transform)| {
            let who = id
                .map(|i| i.0.clone())
                .or_else(|| name.map(|n| n.0.clone()))
                .unwrap_or_else(|| "<unnamed>".to_string());
            let at = transform.translation;
            (
                format!("{who}@{:.3},{:.3},{:.3}", at.x, at.y, at.z),
                uuid.0.clone(),
            )
        })
        .collect();
    rows.sort();
    rows
}

/// The world's own `name -> uuid` map, sorted — the second half of the identity
/// claim, and the one the scenario runtime actually resolves through.
///
/// `spawn_world_entities` writes it, so it is empty until that system has run
/// and it re-keys the moment the mint hands out different sequence numbers.
/// `world::dispatch`, `comms::scripted` and `civilian::server` all resolve an
/// authored name to a live uuid through it, so two hosts of one mission
/// disagreeing here is two hosts targeting different entities from the same
/// script line.
fn world_name_to_uuid(app: &App) -> Vec<(String, String)> {
    let Some(config) = app.world().get_resource::<WorldConfig>() else {
        return Vec::new();
    };
    let mut pairs: Vec<(String, String)> = config
        .name_to_uuid
        .iter()
        .map(|(name, uuid)| (name.clone(), uuid.clone()))
        .collect();
    pairs.sort();
    pairs
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
        entity_identities(&mut app).is_empty(),
        "an empty lobby spawns no simulation entities"
    );
}

#[test]
fn game_over_reaches_the_native_transport_and_final_hud_after_simulation_stops() {
    use project_phoenix::console_bridge::HudStateChanged;
    use project_phoenix::core::balance::Outcome;
    use project_phoenix::core::messages::DeliveryClass;
    use project_phoenix::core::report::{MissionReport, ReportRow, ReportRowState};
    use project_phoenix::server::ViewscreenBorderPlugin;
    use project_phoenix::server_app::GameOverReason;

    let mut cfg = lobby_config();
    cfg.solo = true;
    let mut app = build_native_host_app(&cfg, &preload()).expect("native host assembles");
    // Contract omits the GPU and its presentation plugins. Install the actual
    // HUD plugin, including its OnEnter ordering, without opening a window.
    app.add_plugins(ViewscreenBorderPlugin);
    let transport = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(transport.transport()));
    pump(&mut app, 4);
    let (scenario, hull) = pick();
    select(&mut app, "phase-operator", &scenario, &hull);
    pump(&mut app, 120);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress
    );

    const TOKEN: &str = "3f1a6c2e-0a11-4b3c-9d55-000000000043";
    transport.send(
        TOKEN,
        ClientMessage::Identify {
            token: TOKEN.into(),
            name: "Ending witness".into(),
        },
    );
    pump(&mut app, 2);
    assert!(
        transport
            .drain_outbound()
            .iter()
            .any(|(target, message, _)| {
                *target == Target::Token(TOKEN.into())
                    && matches!(message, ServerMessage::Welcome { .. })
            }),
        "the crew transport is connected before the terminal transition"
    );
    let mut hud_cursor = app
        .world()
        .resource::<Messages<HudStateChanged>>()
        .get_cursor_current();

    const REASON: &str = "server.game_over.ship_destroyed";
    app.world_mut()
        .insert_resource(GameOverReason(Some(REASON.into()), Some(Outcome::Defeat)));
    app.world_mut()
        .resource_mut::<MissionReport>()
        .set_row(ReportRow {
            id: "lyra".into(),
            heading_id: "world.falling_skyway.report.lyra.heading".into(),
            outcome_id: "world.falling_skyway.report.lyra.lost".into(),
            state: ReportRowState::Lost,
            score: -6,
        });
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::GameOver);
    pump(&mut app, 1);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::GameOver
    );

    let outbound = transport.drain_outbound();
    let endings: Vec<_> = outbound
        .iter()
        .filter(|(_, message, _)| matches!(message, ServerMessage::GameOver { .. }))
        .collect();
    assert_eq!(
        endings.len(),
        1,
        "the phase entry reaches the real frame-driven transport exactly once"
    );
    assert_eq!(endings[0].0, Target::All);
    assert_eq!(endings[0].2, DeliveryClass::Reliable);
    let ServerMessage::GameOver {
        reason,
        outcome,
        report,
    } = &endings[0].1
    else {
        unreachable!()
    };
    assert_eq!(reason, REASON);
    assert_eq!(outcome.as_deref(), Some("defeat"));
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].id, "lyra");
    assert_eq!(report[0].state, "lost");
    let hud: Vec<_> = hud_cursor
        .read(app.world().resource::<Messages<HudStateChanged>>())
        .collect();
    assert_eq!(hud.len(), 1, "GameOver publishes one final HUD snapshot");
    assert!(
        hud[0]
            .json
            .contains(&format!("\"game_over_message\":\"{REASON}\"")),
        "the registered HUD captures the nonempty reason before its consumption: {}",
        hud[0].json
    );
    let latch = app.world().resource::<GameOverReason>();
    assert_eq!(
        latch.0, None,
        "preserve the reason's existing consumption/digest contract"
    );
    assert_eq!(latch.1, Some(Outcome::Defeat));
    assert_eq!(app.world().resource::<MissionReport>().total(), -6);

    pump(&mut app, 3);
    assert!(
        !transport
            .drain_outbound()
            .iter()
            .any(|(_, message, _)| matches!(message, ServerMessage::GameOver { .. })),
        "later GameOver frames must not repeat the ending"
    );
    assert_eq!(
        hud_cursor
            .read(app.world().resource::<Messages<HudStateChanged>>())
            .count(),
        0,
        "later frames must not replace the final HUD with an empty reason"
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
        !entity_identities(&mut app).is_empty(),
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

    // The same claim on the viewscreen's own presentation: the four radar
    // widgets `Startup` spawned against the world-less lobby's fallback hull
    // were taken down and re-spawned against the chosen one, so there are still
    // exactly four of them and their ranges are this hull's.
    let widgets: Vec<f32> = app
        .world_mut()
        .query::<(
            &project_phoenix::gui::ConsoleRadar,
            &project_phoenix::gui::GenericRadarWidget,
        )>()
        .iter(app.world())
        .map(|(_, radar)| radar.range)
        .collect();
    assert_eq!(
        widgets.len(),
        4,
        "the runtime load re-seats the viewscreen radar widgets rather than \
         stacking a second set on top of them"
    );
    assert!(
        widgets.contains(&authored),
        "the helm widget carries the chosen hull's authored range ({authored}), \
         not the fallback cruiser's: {widgets:?}"
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

    let booted_ids = entity_identities(&mut booted);
    assert!(
        !booted_ids.is_empty(),
        "the boot host spawned the world's entities"
    );
    assert_eq!(
        entity_identities(&mut runtime),
        booted_ids,
        "a world loaded at runtime must mint exactly the ids the same world \
         loaded at boot mints, AND mint each one to the same entity — the mint \
         is parked at tick 0 across the spawn pass precisely so the uuids do \
         not carry the operator's reaction time, and the ids are folded into \
         the authoritative digest against the entity that holds them"
    );

    // The same claim through the map the scenario runtime resolves names with.
    // It is written by `spawn_world_entities` alone, so it re-keys the instant
    // the `setup_world < spawn_world_entities` pin flips and the anonymous
    // spawn pass takes the sequence numbers the named one was minting from.
    let booted_names = world_name_to_uuid(&booted);
    assert!(
        !booted_names.is_empty(),
        "the picked world must author at least one named [[entity]] for this \
         half of the test to mean anything"
    );
    assert_eq!(
        world_name_to_uuid(&runtime),
        booted_names,
        "the world's name -> uuid map must be identical on both paths — it is \
         what `world::dispatch`, `comms::scripted` and `civilian::server` \
         resolve an authored name through, so a disagreement here is two hosts \
         of one mission targeting different entities from the same script line"
    );

    use bevy::ecs::system::RunSystemOnce;
    use project_phoenix::sim_rng::{with_live_stream, LiveStream, SimRng, SimStream};
    fn live_draw(rng: LiveStream<{ SimStream::BeamCycleJitter as usize }>) -> u32 {
        with_live_stream(rng.as_deref(), |stream| stream.next_u32())
    }
    assert_eq!(booted.world().resource::<SimRng>().seed(), SEED);
    assert_eq!(runtime.world().resource::<SimRng>().seed(), SEED);
    let before = booted.world().resource::<SimRng>().state();
    assert_eq!(runtime.world().resource::<SimRng>().state(), before);
    let reference = SimRng::from_state(before).unwrap();
    let expected = reference.stream(SimStream::BeamCycleJitter).next_u32();
    assert_eq!(
        booted.world_mut().run_system_once(live_draw).unwrap(),
        expected
    );
    assert_eq!(
        runtime.world_mut().run_system_once(live_draw).unwrap(),
        expected,
        "runtime load must rebind the live generator after the lobby schedules initialized"
    );
    assert_eq!(
        booted.world().resource::<SimRng>().state(),
        reference.state()
    );
    assert_eq!(
        runtime.world().resource::<SimRng>().state(),
        reference.state()
    );
}

#[test]
fn a_load_that_fails_after_the_ingest_leaves_a_pickable_lobby() {
    // The failure the runtime path has that the boot path does not: `--world`
    // reports an unusable hull at the prompt and the process exits, but here the
    // ingest has already inserted `WorldConfig` + `PreCompiledScripts` by the
    // time `install_world_selection` refuses. Left behind, they take
    // `world_load::awaiting_world` false — a host holding a world it never
    // spawned, deaf to every later pick. `--ship` naming a template the preload
    // never cached is the reachable way to provoke exactly that ordering.
    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = true;
    cfg.ship_path = Some("assets/entities/not_a_hull_at_all.toml".to_string());
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");
    pump(&mut app, 4);

    let (scenario_id, hull) = pick();
    select(&mut app, "phone-1", &scenario_id, &hull);
    pump(&mut app, 30);

    assert!(
        app.world().get_resource::<WorldConfig>().is_none(),
        "a refused hull unwinds the ingest rather than half-loading the world"
    );
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "and the host is still in the lobby"
    );
    // This failure point is BETWEEN the two freezes: `ingest_world` has frozen
    // the content ledger over the refused world's file set, and
    // `install_world_selection` refuses the hull before it re-records and
    // freezes again. Neither of the two ingest-time failure classes below
    // (unreadable file, malformed file) reaches a frozen ledger at all, so
    // without this assertion the middle point is unpinned and only
    // `a_hull_with_no_station_blocks_leaves_a_pickable_lobby` (the latest point,
    // past BOTH freezes) covers the unwind's ledger reset.
    assert!(
        !project_phoenix::content_ledger::is_frozen(),
        "a refused hull must unfreeze the ledger too — `frozen_or_live` is what \
         `snapshot::versions` answers a fleet peer's content check with, and \
         what a save is bound to, so a ledger left frozen over the refused \
         world answers for a world this host does not have"
    );

    // Now a hull that IS cached: the same host loads normally, which is the
    // whole point of unwinding rather than latching.
    app.world_mut()
        .resource_mut::<project_phoenix::native_host::world_load::LobbyBootSettings>()
        .ship_path = None;
    select(&mut app, "phone-2", &scenario_id, &hull);
    pump(&mut app, 120);
    assert!(
        app.world().get_resource::<WorldConfig>().is_some(),
        "the next selection loads normally"
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
        matches!(msg, ServerMessage::ScenarioCatalog(..))
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

    // The load is not silent. This phone identified BEFORE the pick, so the
    // `Welcome` it holds was built against `load_ship_config_from_disk`'s
    // battleship fallback roster and a Default `ShipClientConfig` — a hull this
    // host is not flying. The load has just replaced both, and nothing else on
    // this path would say so: `gui/lobby-state.js`'s fully-locked-catalogue
    // branch resumes the lobby on whatever `Welcome` it already has, because
    // that branch was written for issue #756's round-2 world REUSE, where the
    // roster genuinely has not moved. A native runtime load is round ONE, and
    // the browser's round one re-welcomes for free (its Bevy app, and therefore
    // its `handle_identify`, does not exist until after `wasm_init`).
    //
    // So the drained outbound must carry a fresh `Welcome`, and its roster must
    // be the CHOSEN hull's. Everything below claims a seat from THAT roster
    // rather than from the host's own `ShipStations` resource — reading the
    // resource is how a test walks straight past this bug, because the resource
    // is right and only the client's copy is wrong.
    let dispatched = handle.drain_outbound();
    let (welcome_stations, welcome_config) = dispatched
        .iter()
        .rev()
        .find_map(|(_, msg, _)| match msg {
            ServerMessage::Welcome {
                ship_stations,
                ship_config,
                ..
            } => Some((ship_stations.clone(), ship_config.clone())),
            _ => None,
        })
        .expect(
            "a runtime world load must re-Welcome every connected participant — \
             a phone welcomed before the pick is holding the fallback hull's roster",
        );

    let authored = project_phoenix::entities::include_resolve::load_entity_config(&hull)
        .expect("the selected hull parses");
    assert_eq!(
        welcome_stations
            .stations
            .iter()
            .map(|s| s.id.0.clone())
            .collect::<Vec<_>>(),
        authored
            .ship_config
            .as_ref()
            .expect("the selected hull authors [[station]] blocks")
            .stations
            .iter()
            .map(|s| s.id.0.clone())
            .collect::<Vec<_>>(),
        "the re-Welcome carries the chosen hull's station roster, not the \
         world-less lobby's `load_ship_config_from_disk` battleship fallback"
    );
    // And the client config with it. The world-less lobby's fallback here is a
    // different hull again — `update_session_with_config` reads the literal
    // `alliance_cruiser` path when no `SelectedShipResource` has been installed
    // — so this is an equality against the CHOSEN hull rather than a mere
    // "not the Default", which the cruiser's own numbers would also satisfy.
    let expected_range = authored
        .helm_console
        .as_ref()
        .map(|hc| hc.effective_radar_range())
        .unwrap_or_default();
    assert!(
        expected_range > 0.0,
        "{hull} must author a helm radar range for this assertion to mean anything"
    );
    assert_eq!(
        welcome_config.helm_radar_range, expected_range,
        "the re-Welcome carries the chosen hull's authored client config"
    );

    // Now the ordinary ready path: claim a station from the roster the client
    // was actually told about, ready up, and the collective auto-start takes the
    // host into the mission. `handle_select_station` validates against the REAL
    // roster, so a claim from a phantom roster would be silently ignored.
    let station = welcome_stations
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
fn a_runtime_load_clears_the_ready_flags_the_world_less_lobby_collected() {
    // The fourth of `handle_return_to_lobby`'s four lines, and the one with
    // teeth. `republish_loaded_world` wipes the seats because the roster is
    // being replaced wholesale — but `SessionManager::all_ready` does not look
    // at seats at all: it asks only whether every connected non-spectator is
    // ready. So a ready flag set against the pre-load fallback roster survives
    // a seat-only wipe, and the very next `SetReady` from anyone else starts a
    // mission with a crew holding no stations.
    //
    // The shipped phone client cannot reach that state today (it readies from a
    // console it has already claimed, and a runtime load is round one), which
    // is exactly why this is a test rather than a bug report: what is being
    // pinned is the PARITY claim `republish_loaded_world` makes with
    // `handle_return_to_lobby`, and parity means all four lines plus the
    // per-player `ReadyChanged`.
    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = false;
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");

    let handle = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(handle.transport()));

    const TOKEN: &str = "3f1a6c2e-0a11-4b3c-9d55-000000000756";
    handle.send(
        TOKEN,
        ClientMessage::Identify {
            token: TOKEN.to_string(),
            name: "Ada".to_string(),
        },
    );
    pump(&mut app, 8);

    // The flag is set on the session directly rather than through a `SetReady`
    // message, because the handler would ALSO start the five-second auto-start
    // countdown and this test is about the flag outliving the load, not about
    // that countdown. The state reached is identical either way — one bool on
    // one `Player`.
    app.world_mut()
        .resource_mut::<project_phoenix::lobby::Sessions>()
        .0
        .set_ready(TOKEN, true);
    let ready_before = app
        .world()
        .resource::<project_phoenix::lobby::Sessions>()
        .0
        .players()
        .iter()
        .find(|p| p.token == TOKEN)
        .map(|p| p.ready);
    assert_eq!(
        ready_before,
        Some(true),
        "the participant is genuinely ready before the world lands, or this \
         test proves nothing"
    );
    let _ = handle.drain_outbound(); // everything from before the pick

    let (scenario_id, hull) = pick();
    handle.send(TOKEN, ClientMessage::SelectScenario { scenario_id });
    handle.send(
        TOKEN,
        ClientMessage::SelectPlayerShip {
            template_path: hull,
        },
    );
    pump(&mut app, 60);
    assert!(
        app.world().get_resource::<WorldConfig>().is_some(),
        "the selection loaded the world"
    );

    assert_eq!(
        app.world()
            .resource::<project_phoenix::lobby::Sessions>()
            .0
            .players()
            .iter()
            .find(|p| p.token == TOKEN)
            .map(|p| p.ready),
        Some(false),
        "a runtime world load must reset the ready flags with the seats — \
         `all_ready` ignores seats, so a flag left set against the pre-load \
         fallback roster can start a mission with a seatless crew"
    );
    assert!(
        !app.world()
            .resource::<project_phoenix::lobby::Sessions>()
            .0
            .all_ready(),
        "and the host is therefore not sitting one message away from starting"
    );

    // And the clients are told, per player, exactly as `handle_return_to_lobby`
    // tells them. The re-`Welcome` carries the cleared flag in its roster too,
    // but `ReadyChanged` is the arm `gui/lobby-state.js` raises
    // `REDUCER_EFFECTS.READY_CHANGED` from, which is what a console's own ready
    // control redraws off.
    let dispatched = handle.drain_outbound();
    assert!(
        dispatched.iter().any(|(_, msg, _)| matches!(
            msg,
            ServerMessage::ReadyChanged { token, ready: false } if token.as_str() == TOKEN
        )),
        "the load must broadcast `ReadyChanged {{ ready: false }}` for every \
         player on the roster, the way `handle_return_to_lobby` does. \
         `ReadyChanged` messages actually seen: {:?}",
        dispatched
            .iter()
            .filter_map(|(_, msg, _)| match msg {
                ServerMessage::ReadyChanged { token, ready } => Some((token.clone(), *ready)),
                _ => None,
            })
            .collect::<Vec<_>>()
    );
}

/// A catalogue that publishes `world` under a synthetic scenario id, alongside
/// every real entry — so a test can drive a doomed pick and then a good one
/// through the ordinary arbiter.
fn catalog_plus(id: &str, world: &str, ships: &[String]) -> ScenarioCatalog {
    let mut catalog = catalog();
    catalog
        .scenarios
        .push(project_phoenix::world::manifest::ScenarioCatalogEntry {
            id: id.to_string(),
            world: world.to_string(),
            label: None,
            description: None,
            ships: ships
                .iter()
                .map(|path| project_phoenix::world::config::AvailableShipEntry {
                    template_path: path.clone(),
                    label: None,
                })
                .collect(),
            origin: None,
        });
    catalog
}

/// Drive `app`'s lobby with a doomed pick and then a good one, asserting the
/// host is genuinely retryable in between.
///
/// The shared body of the failure-class tests below: what every one of them
/// claims is the same claim — a refused world leaves a clean lobby, not a host
/// wedged holding half a world.
///
/// **Read the three callers as one graded set, not three independent tests.**
/// Only the last of them exercises the whole of `unwind_failed_load`:
///
///  * `a_world_file_that_cannot_be_read…` and `a_malformed_world_file…` are two
///    different ERROR CLASSES arriving at the SAME call site and the same point
///    in `boot::ingest_world` — after its ledger reset, before its freeze. So
///    neither of them can fail if the unwind's `content_ledger::reset` is
///    deleted: at that point there is nothing frozen and nothing inserted. They
///    are kept as a pair anyway because they pin the two *refusal* shapes a
///    participant can provoke from the catalogue, and a regression that turned
///    one into a panic rather than a refusal would show up in exactly one of
///    them. They do not pin the unwind.
///  * The unwind's ledger reset is pinned at the two later points instead:
///    between the freezes by `a_load_that_fails_after_the_ingest_leaves_a_
///    pickable_lobby`, and past both by
///    `a_hull_with_no_station_blocks_leaves_a_pickable_lobby`.
fn a_refused_pick_then_a_good_one(app: &mut App, bad_id: &str, bad_hull: &str) {
    select(app, "phone-1", bad_id, bad_hull);
    pump(app, 30);

    assert!(
        app.world().get_resource::<WorldConfig>().is_none(),
        "a refused world must not leave a `WorldConfig` behind — with one \
         present `awaiting_world` is false and the lobby is deaf to every later \
         pick"
    );
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "the host stays in the lobby"
    );
    assert_eq!(
        app.world().resource::<LobbySelection>().0,
        project_phoenix::lobby::scenario_arbiter::ScenarioSelection::default(),
        "the refused selection is released so another participant can pick"
    );
    assert!(
        !project_phoenix::content_ledger::is_frozen(),
        "the content ledger must not stay frozen over a world this host does \
         not have — `frozen_or_live` is what `snapshot::versions` answers a \
         fleet peer's content check with, and what a save is bound to"
    );

    // And the whole point of unwinding rather than latching: the next pick works.
    let (scenario_id, hull) = pick();
    select(app, "phone-2", &scenario_id, &hull);
    pump(app, 120);
    assert!(
        app.world().get_resource::<WorldConfig>().is_some(),
        "a good selection after the refused one loads normally"
    );
    assert!(
        project_phoenix::content_ledger::is_frozen(),
        "and the good load freezes the ledger over ITS content"
    );
}

#[test]
fn a_world_file_that_cannot_be_read_leaves_a_pickable_lobby() {
    // The first failure class the runtime path has and the boot path does not:
    // `--world` naming an unreadable file dies at the prompt, but here the pick
    // came from a participant and there is a lobby to go back to. The refusal
    // happens inside `boot::ingest_world`, AFTER it has reset the content ledger
    // and BEFORE it freezes one — the earliest of the three failure points.
    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = true;
    cfg.catalog = catalog_plus(
        "unreadable_world",
        "assets/worlds/__no_such_world_exists__.toml",
        &[pick().1],
    );
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");
    pump(&mut app, 4);
    a_refused_pick_then_a_good_one(&mut app, "unreadable_world", &pick().1);
}

#[test]
fn a_malformed_world_file_leaves_a_pickable_lobby() {
    // The second: the file reads and does not parse. Written to the OS temp
    // directory rather than into `assets/worlds/` so the repository's own
    // catalogue, which every other test in this file reads, is untouched —
    // `world::load::FsReader` is `std::fs::read_to_string`, so an absolute path
    // is as good as a relative one.
    let broken = std::env::temp_dir().join("phoenix_1326_malformed_world.toml");
    std::fs::write(&broken, "[global\nthis is not toml = = =\n")
        .expect("the temp directory is writable");

    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = true;
    cfg.catalog = catalog_plus("malformed_world", &broken.to_string_lossy(), &[pick().1]);
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");
    pump(&mut app, 4);
    a_refused_pick_then_a_good_one(&mut app, "malformed_world", &pick().1);

    let _ = std::fs::remove_file(&broken);
}

#[test]
fn a_hull_with_no_station_blocks_leaves_a_pickable_lobby() {
    // The third, and the latest of the three: the world ingests cleanly and the
    // HULL is refused. By then `ingest_world` has inserted `WorldConfig` +
    // `PreCompiledScripts` and frozen the ledger, and `install_world_selection`
    // has re-recorded the hull and frozen it AGAIN — all before the
    // `[[station]]` check that fails. Everything that unwinding has to undo is
    // in place at this point and nowhere else, which is what makes this the
    // failure worth pinning.
    //
    // `assets/entities/planet_earth.toml` is real content the picked world
    // itself declares — so the preload has cached it and it reaches that check,
    // rather than being turned away by the template-cache gate before it.
    const NO_STATIONS: &str = "assets/entities/planet_earth.toml";
    assert!(
        project_phoenix::entities::include_resolve::load_entity_config(NO_STATIONS)
            .expect("the planet template parses")
            .ship_config
            .is_none(),
        "{NO_STATIONS} must author no [[station]] blocks for this test to \
         provoke the failure it names"
    );

    let world = scenario_arbiter::find_scenario(&catalog(), SCENARIO)
        .expect("the shipped catalogue publishes the flagship scenario")
        .world
        .clone();
    let preload = preload();
    let mut cfg = lobby_config();
    cfg.solo = true;
    cfg.catalog = catalog_plus("stationless_hull", &world, &[NO_STATIONS.to_string()]);
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");
    pump(&mut app, 4);
    a_refused_pick_then_a_good_one(&mut app, "stationless_hull", NO_STATIONS);
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

    let handle = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(handle.transport()));
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

    // And the catalogue every phone folds has to SAY so. This is the one
    // combination where the two answers differ — a pinned hull and an
    // arbitrated one that is not it — so it is the only place the reported
    // `locked_ship` can lie, and the lie is the exact one the field exists to
    // prevent: the host flying `pinned` while every picker shows `picked` as
    // the settled choice. Asserting `SelectedShipResource` alone walks straight
    // past it, because that resource is right and only the wire is wrong.
    let locked_ship = handle
        .drain_outbound()
        .into_iter()
        .filter_map(|(_, msg, _)| match msg {
            ServerMessage::ScenarioCatalog(catalog) => Some(catalog.locked_ship),
            _ => None,
        })
        .next_back()
        .expect("the lobby publishes its catalogue");
    assert_eq!(
        locked_ship.as_deref(),
        Some(pinned.as_str()),
        "the broadcast catalogue must report the PINNED hull as locked — it is \
         the hull this host is flying. Reporting the arbitrated `{picked}` \
         instead is a picker showing a choice the host has already overruled"
    );
}

#[test]
fn a_pinned_hull_completes_the_selection_on_the_scenario_lock_alone() {
    // `--lobby --ship X` is the scripted-run combination, and before this a
    // participant still had to send a `SelectPlayerShip` the host would then
    // discard — so a script that pinned its hull and picked only a scenario sat
    // in the lobby forever waiting for a message whose content could not matter.
    // A pinned hull now SATISFIES the hull half.
    let preload = preload();
    let (scenario_id, hull) = pick();
    let mut cfg = lobby_config();
    cfg.solo = true;
    cfg.ship_path = Some(hull.clone());
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");

    let handle = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(handle.transport()));
    pump(&mut app, 4);

    // The scenario, and ONLY the scenario.
    app.world_mut().write_message(InboundMessage {
        token: "phone-1".to_string(),
        msg: ClientMessage::SelectScenario { scenario_id },
    });
    pump(&mut app, 60);

    assert!(
        app.world().get_resource::<WorldConfig>().is_some(),
        "a scenario lock alone completes the pick when --ship already named the hull"
    );
    assert_eq!(
        app.world()
            .resource::<project_phoenix::lobby::SelectedShipResource>()
            .0,
        project_phoenix::entities::include_resolve::canonical_template_path(&hull),
        "and the hull flown is the pinned one"
    );

    // The catalogue the phones fold says so too, so their picker shows the
    // decision that has already been made rather than a choice this host would
    // overrule. `gui/lobby-state.js` reads both fields being set as "selection
    // done" and leaves the picker on it.
    let locked = handle
        .drain_outbound()
        .into_iter()
        .filter_map(|(_, msg, _)| match msg {
            ServerMessage::ScenarioCatalog(catalog) => {
                Some((catalog.locked_scenario, catalog.locked_ship))
            }
            _ => None,
        })
        .next_back()
        .expect("the lobby publishes its catalogue");
    assert_eq!(
        locked.1.as_deref(),
        Some(hull.as_str()),
        "the published catalogue reports the pinned hull as locked"
    );
    assert!(
        locked.0.is_some(),
        "and the arbitrated scenario alongside it"
    );
}

// ── Picking from the viewscreen itself (issue #1328) ────────────────────────
//
// Everything above drives the arbiter the way a PHONE reaches it. These drive it
// the way the operator standing in front of the viewscreen does: through the
// host-lobby bridge, over a `RecordingSurface` standing in for the Ultralight
// view, so the whole round trip — push the catalogue, click, load the world —
// runs headless in ordinary `cargo test` rather than only on a Windows machine
// with a GPU.

/// A world-less host with a lobby surface, and the surface's own side of it.
fn lobby_host_with_surface(cfg: NativeHostConfig) -> (App, LocalHostLobby, RecordingSurface) {
    let preload = preload();
    let lobby = LocalHostLobby::open("127.0.0.1:8080");
    let mut cfg = cfg;
    cfg.host_lobby = Some(lobby.clone());
    let app = build_native_host_app(&cfg, &preload).expect("a host with a lobby surface assembles");
    (app, lobby, RecordingSurface::ready())
}

/// One frame of the surface loop the Ultralight display host runs in `Update`:
/// hand the page everything pending, take back everything it queued.
fn pump_surface(lobby: &LocalHostLobby, surface: &mut RecordingSurface) -> Vec<String> {
    let before = surface.pushed.len();
    pump_host_lobby(&lobby.bridge, surface);
    surface.pushed[before..].to_vec()
}

/// The newest scenario-panel payload the surface was handed, decoded.
fn scenario_pushes(pushed: &[String]) -> Vec<serde_json::Value> {
    pushed
        .iter()
        .filter_map(|script| {
            let rest = script.strip_prefix("window.__phoenixHostLobbyScenario('")?;
            let body = rest.strip_suffix("')")?;
            // `bridge::push_call` escapes single quotes and backslashes for the
            // JavaScript string literal it builds; undo exactly that.
            serde_json::from_str(&body.replace("\\'", "'").replace("\\\\", "\\")).ok()
        })
        .collect()
}

#[test]
fn a_lobby_host_puts_its_catalogue_on_the_viewscreen_without_being_asked() {
    // Acceptance criterion 1. The surface sends no `Identify`, so it gets no
    // greeting the way a phone does — the picker has to arrive because the host
    // has one to offer, or a `--lobby` host opens on a blank screen (which is
    // exactly what #1325 + #1326 composed to before this issue).
    let (mut app, lobby, mut surface) = lobby_host_with_surface(lobby_config());
    pump(&mut app, 4);
    let pushed = pump_surface(&lobby, &mut surface);

    let payloads = scenario_pushes(&pushed);
    let panel = payloads
        .last()
        .expect("the picker's state reaches the surface");
    assert_eq!(panel["locked"], serde_json::Value::Bool(false));
    assert_eq!(panel["locked_scenario"], serde_json::Value::Null);
    let ids: Vec<&str> = panel["scenarios"]
        .as_array()
        .expect("the payload carries the catalogue")
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert!(
        ids.contains(&SCENARIO),
        "the shipped catalogue reaches the viewscreen: {ids:?}"
    );

    // And it says nothing more until something moves — every push is a
    // synchronous evaluate_script on the thread `FixedUpdate` runs `SimSet` on.
    let quiet = pump_surface(&lobby, &mut surface);
    assert!(
        scenario_pushes(&quiet).is_empty(),
        "an untouched picker costs the simulation nothing"
    );
}

#[test]
fn picking_from_the_viewscreen_loads_the_world_and_closes_the_picker() {
    // Acceptance criterion 1's second half, end to end through the seam this
    // issue built: two clicks on the surface become two records, the records
    // become the same two `ClientMessage`s a phone sends, the arbiter accepts
    // them, and the world lands.
    let (mut app, lobby, mut surface) = lobby_host_with_surface(lobby_config());
    pump(&mut app, 4);
    pump_surface(&lobby, &mut surface);

    let (scenario_id, hull) = pick();
    surface.queue_record(format!(
        r#"{{"kind":"select_scenario","scenario_id":"{scenario_id}"}}"#
    ));
    pump_surface(&lobby, &mut surface);
    pump(&mut app, 4);

    // The scenario is locked and the surface has been told so — which is what
    // moves its picker on to the hull stage.
    let after_scenario = scenario_pushes(&pump_surface(&lobby, &mut surface));
    let panel = after_scenario
        .last()
        .expect("locking the scenario re-publishes the picker");
    assert_eq!(panel["locked_scenario"], serde_json::json!(scenario_id));
    assert_eq!(panel["locked"], serde_json::Value::Bool(false));

    surface.queue_record(format!(
        r#"{{"kind":"select_ship","template_path":"{hull}"}}"#
    ));
    pump_surface(&lobby, &mut surface);
    pump(&mut app, 60);

    assert!(
        app.world().get_resource::<WorldConfig>().is_some(),
        "a pick made on the viewscreen loads the world, exactly as a phone's does"
    );
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "loading a world does not start the mission — readying or launching does"
    );

    // …and the picker closes. `locked` is `scenarioCatalogView`'s third
    // argument, and it is what takes the full-screen panel off the viewscreen
    // so the crew lobby underneath is visible.
    let closed = scenario_pushes(&pump_surface(&lobby, &mut surface));
    let panel = closed
        .last()
        .expect("the world landing re-publishes the picker one last time");
    assert_eq!(panel["locked"], serde_json::Value::Bool(true));
    assert_eq!(panel["locked_ship"], serde_json::json!(hull));
}

#[test]
fn a_pick_the_catalogue_does_not_offer_leaves_the_picker_exactly_where_it_was() {
    // The refusal, rendered the way the host page renders one: not at all.
    // `arbiterSelectScenario` returns before its render on any non-accepted
    // outcome, so the panel keeps showing the stage that is actually true — and
    // so does this one, because a refused pick moves no state and therefore
    // publishes nothing. What is NOT silent is the operator log, which
    // `drain_scenario_selection` writes at warn level on both hosts.
    let (mut app, lobby, mut surface) = lobby_host_with_surface(lobby_config());
    pump(&mut app, 4);
    pump_surface(&lobby, &mut surface);

    surface.queue_record(r#"{"kind":"select_scenario","scenario_id":"no_such_world"}"#);
    pump_surface(&lobby, &mut surface);
    pump(&mut app, 8);

    assert!(
        scenario_pushes(&pump_surface(&lobby, &mut surface)).is_empty(),
        "a refused pick repaints nothing, exactly as it repaints nothing on the web"
    );
    assert!(
        app.world().get_resource::<WorldConfig>().is_none(),
        "and it certainly does not load a world"
    );
    assert_eq!(
        app.world()
            .resource::<LobbySelection>()
            .0
            .scenario_id
            .as_deref(),
        None,
        "the arbiter's lock is untouched, so the panel's next render is the scenario stage"
    );

    // The picker is still usable: a good pick after a refused one still lands.
    let (scenario_id, hull) = pick();
    surface.queue_record(format!(
        r#"{{"kind":"select_scenario","scenario_id":"{scenario_id}"}}"#
    ));
    surface.queue_record(format!(
        r#"{{"kind":"select_ship","template_path":"{hull}"}}"#
    ));
    pump_surface(&lobby, &mut surface);
    pump(&mut app, 60);
    assert!(app.world().get_resource::<WorldConfig>().is_some());
}

#[test]
fn a_record_this_bridge_does_not_speak_is_dropped_rather_than_admitted() {
    // The surface holds no session token, so a `ClientMessage` envelope on this
    // queue would be a participant nobody admitted. It is refused at the decode,
    // before anything reads it as a command.
    let (mut app, lobby, mut surface) = lobby_host_with_surface(lobby_config());
    pump(&mut app, 4);
    pump_surface(&lobby, &mut surface);

    surface.queue_record(r#"{"type":"SetReady","data":{"ready":true}}"#);
    surface.queue_record(r#"{"kind":"launch_everything"}"#);
    pump_surface(&lobby, &mut surface);
    pump(&mut app, 8);

    assert!(app.world().get_resource::<WorldConfig>().is_none());
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby
    );
}

#[test]
fn the_viewscreens_launch_control_starts_a_crewless_mission() {
    // Acceptance criterion 3, and the reason `apply_force_start` stopped being
    // wasm-only: a lobby whose crew are all on phones has to be launchable from
    // the viewscreen in front of the operator. No `--solo` here — this is the
    // ordinary crewed mode, launched by hand.
    let (mut app, lobby, mut surface) = lobby_host_with_surface(lobby_config());
    pump(&mut app, 4);
    pump_surface(&lobby, &mut surface);

    // Pressing it before there is a world is refused rather than remembered:
    // starting a mission over no world would spawn the game-start entities into
    // an empty `World`.
    surface.queue_record(r#"{"kind":"force_start"}"#);
    pump_surface(&lobby, &mut surface);
    pump(&mut app, 8);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "there is nothing to fly yet"
    );

    let (scenario_id, hull) = pick();
    surface.queue_record(format!(
        r#"{{"kind":"select_scenario","scenario_id":"{scenario_id}"}}"#
    ));
    surface.queue_record(format!(
        r#"{{"kind":"select_ship","template_path":"{hull}"}}"#
    ));
    pump_surface(&lobby, &mut surface);
    pump(&mut app, 60);
    assert!(app.world().get_resource::<WorldConfig>().is_some());
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "and a world alone still does not start a mission"
    );

    surface.queue_record(r#"{"kind":"force_start"}"#);
    pump_surface(&lobby, &mut surface);
    pump(&mut app, 600);
    // The EXACT state, not `!= Lobby`: that spelling also passes on a host that
    // starts a mission and then parks on the loading screen forever, which is
    // the one failure this acceptance criterion is here to catch.
    //
    // `Loading` and not `InProgress` because **this binary cannot reach
    // `InProgress`, and the reason is the harness rather than the host.** The
    // way out is `asset_preload::auto_transition_from_loading`, which IS
    // registered here (`build_native_host_app` passes `render: true`) and does
    // run — but it waits on `AssetPreloadResource::complete`, and that gate
    // wants every radar icon decoded into an `Image`. A `cargo test` binary
    // stands up no image or GLB loader, so those handles fail instead of
    // landing and the gate is unsatisfiable at any frame count; 600 frames here
    // rather than 30 so "not enough time" is not the explanation either.
    //
    // The second assertion pins that reason, so a harness that later does load
    // assets fails loudly and earns the stronger claim rather than leaving a
    // weakened one to be discovered.
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Loading,
        "the AI-launch control on the viewscreen starts the mission: the host \
         leaves Lobby for the loading phase"
    );
    let preload = app
        .world()
        .resource::<project_phoenix::server::asset_preload::AssetPreloadResource>();
    assert!(
        !preload.complete && preload.ready_count < preload.total_count,
        "the only thing between this host and InProgress is the asset gate this \
         test binary cannot satisfy ({}/{} ready). If this now passes, the \
         harness loads assets — assert GamePhase::InProgress above instead",
        preload.ready_count,
        preload.total_count
    );
}

#[test]
fn a_world_host_never_shows_the_picker_and_a_pinned_hull_never_offers_a_choice() {
    // The flag matrix, as the ACs state it. `--world` decides the scenario, so
    // the scenario stage is skipped: a `--world` host holds no
    // `LobbyScenarioCatalog` and therefore never publishes a picker at all,
    // which is what leaves the panel at the `display: none` the document was
    // assembled with.
    let (_, world_hull) = pick();
    let mut cfg = NativeHostConfig::new("assets/worlds/combat_test.toml");
    cfg.seed = Some(SEED);
    cfg.surface = NativeRenderSurface::Contract;
    let (mut app, lobby, mut surface) = lobby_host_with_surface(cfg);
    pump(&mut app, 8);
    let pushed = pump_surface(&lobby, &mut surface);
    assert!(
        scenario_pushes(&pushed).is_empty(),
        "--world skips the scenario stage: there is nothing to pick"
    );
    assert!(
        app.world().get_resource::<WorldConfig>().is_some(),
        "and the world it named is already ingested"
    );
    // The lobby itself still reaches the surface, so the viewscreen is not blank.
    assert!(
        pushed
            .iter()
            .any(|s| s.starts_with("window.__phoenixHostLobbyApply(")),
        "the crew lobby is still fed: {pushed:?}"
    );

    // `--world --ship` skips both — same absence, plus the hull the flag named.
    let mut cfg = NativeHostConfig::new("assets/worlds/combat_test.toml");
    cfg.seed = Some(SEED);
    cfg.surface = NativeRenderSurface::Contract;
    cfg.ship_path = Some(world_hull.clone());
    let (mut app, lobby, mut surface) = lobby_host_with_surface(cfg);
    pump(&mut app, 8);
    assert!(scenario_pushes(&pump_surface(&lobby, &mut surface)).is_empty());
    assert_eq!(
        app.world()
            .resource::<project_phoenix::lobby::SelectedShipResource>()
            .0,
        project_phoenix::entities::include_resolve::canonical_template_path(&world_hull),
    );
}

#[test]
fn a_pinned_hull_reaches_the_viewscreen_as_a_decision_already_made() {
    // `--lobby --ship X`: the scenario stage still runs, and the hull stage does
    // not, because the flag has already answered it. The surface has to be told
    // the SAME thing every phone is told — `locked_ship` is the pinned hull —
    // or the viewscreen would offer a choice this host has already overruled.
    let (scenario_id, hull) = pick();
    let mut cfg = lobby_config();
    cfg.ship_path = Some(hull.clone());
    let (mut app, lobby, mut surface) = lobby_host_with_surface(cfg);
    pump(&mut app, 4);

    let opening = scenario_pushes(&pump_surface(&lobby, &mut surface));
    let panel = opening
        .last()
        .expect("the picker's state reaches the surface");
    assert_eq!(
        panel["locked_ship"],
        serde_json::json!(hull),
        "a pinned hull is reported as the locked one, so the picker shows a settled choice"
    );
    assert_eq!(
        panel["locked"],
        serde_json::Value::Bool(false),
        "the scenario stage still has to be picked"
    );

    // And the scenario alone completes it, exactly as it does for a phone.
    surface.queue_record(format!(
        r#"{{"kind":"select_scenario","scenario_id":"{scenario_id}"}}"#
    ));
    pump_surface(&lobby, &mut surface);
    pump(&mut app, 60);
    assert!(app.world().get_resource::<WorldConfig>().is_some());
}

// ── a pane name and a station id are one namespace (issue #1331) ─────────────
//
// `install_world_selection` is the one place a participant pane name and the
// hull's station ids can be compared: the names are fixed at the prompt, and the
// roster is known at boot on the `--world` path and at the pick on the `--lobby`
// one. Both paths reach it, so both are driven here.
//
// The `--pane` half of the guard landed with the round-2 fix. What it missed is
// that a pane name reaches the bus by a SECOND route: a `--profile`'s
// `[[display.pane]]` with a `label` and no `station` key. That slot is a
// participant pane in every way that matters here — it stays in the runtime
// watcher's `pane_labels`, and the adapter lays it out as a rectangle on a
// Station window — so an authored `label = "helm"` on a hull with a `helm`
// station is the same collision, reached through a file instead of a flag.

/// The first station id `hull` authors — read off the content rather than
/// pinned, so a hull edit moves this test rather than breaking it.
fn first_station_of(hull: &str) -> String {
    project_phoenix::entities::include_resolve::load_entity_config(hull)
        .expect("the hull's template parses")
        .ship_config
        .expect("a playable hull authors [[station]] blocks")
        .stations
        .first()
        .expect("and at least one of them")
        .id
        .0
        .clone()
}

/// A validated `--profile`: the viewscreen on one monitor, and a Station monitor
/// carrying one PARTICIPANT pane named `label` (no `station` key).
fn profile_naming_a_participant(
    label: &str,
) -> project_phoenix::native_host::bridge_profile::ValidatedProfile {
    use project_phoenix::native_host::bridge_profile::{
        BridgeProfile, DisplayEntry, PaneSlot, PROFILE_VERSION, ROLE_STATION, ROLE_VIEWSCREEN,
    };
    BridgeProfile {
        version: PROFILE_VERSION,
        displays: vec![
            DisplayEntry {
                id: "DELL U2720Q@3840x2160".to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            },
            DisplayEntry {
                id: "BenQ EX@1920x1080".to_string(),
                role: ROLE_STATION.to_string(),
                split: None,
                panes: vec![PaneSlot::for_participant(label)],
            },
        ],
        touch: Vec::new(),
        media: Vec::new(),
    }
    .validate()
    .expect("a viewscreen and a one-participant Station display are a lawful profile")
}

#[test]
fn a_profile_pane_named_for_a_station_is_refused_on_the_world_path() {
    // `--world --profile`: the roster is known before the `App` exists, so the
    // refusal happens where every other bad launch argument does — at the
    // prompt, before a window or a listener is up.
    let preload = preload();
    let (scenario_id, hull) = pick();
    let world = scenario_arbiter::find_scenario(&catalog(), &scenario_id)
        .expect("the picked scenario is in the catalogue")
        .world
        .clone();
    let station = first_station_of(&hull);

    let mut cfg = NativeHostConfig::new(world);
    cfg.seed = Some(SEED);
    cfg.surface = NativeRenderSurface::Contract;
    cfg.ship_path = Some(hull.clone());
    cfg.bridge_profile = Some(profile_naming_a_participant(&station));

    let refusal = match build_native_host_app(&cfg, &preload) {
        Ok(_) => panic!("a profile pane named for a station on this hull is refused"),
        Err(e) => e.to_string(),
    };
    assert!(
        refusal.contains(&format!("{station:?}")),
        "the refusal names the colliding pane: {refusal}"
    );
    assert!(
        refusal.contains("one namespace"),
        "and says why the two cannot both exist: {refusal}"
    );
    assert!(
        refusal.contains(&format!("{station}-crew")),
        "and what to do instead: {refusal}"
    );

    // The same profile with a pane named for a PERSON is untouched — this must
    // refuse a collision, not `--profile` participants.
    cfg.bridge_profile = Some(profile_naming_a_participant("Ada"));
    assert!(
        build_native_host_app(&cfg, &preload).is_ok(),
        "an ordinary authored participant pane still boots"
    );
}

#[test]
fn a_profile_pane_named_for_a_station_is_refused_on_the_lobby_path() {
    // `--lobby --profile`: the roster is not known until a scenario and hull are
    // picked, so the same guard has to fire frames into a running host — through
    // the same `install_world_selection`, with `unwind_failed_load` putting the
    // lobby back to genuinely world-less afterwards.
    //
    // The refusal reaches the operator LOG and nothing else here: the catalogue
    // is re-published with nothing locked, which is what a phone sees. That
    // asymmetry with the `--world` path above is documented on
    // `world_load::unwind_failed_load` rather than papered over.
    let preload = preload();
    let (scenario_id, hull) = pick();
    let station = first_station_of(&hull);

    let mut cfg = lobby_config();
    cfg.solo = true;
    cfg.bridge_profile = Some(profile_naming_a_participant(&station));
    let mut app = build_native_host_app(&cfg, &preload).expect("a world-less host assembles");
    pump(&mut app, 4);

    select(&mut app, "phone-1", &scenario_id, &hull);
    pump(&mut app, 30);

    assert!(
        app.world().get_resource::<WorldConfig>().is_none(),
        "the pick is refused, and the ingest it had already done is unwound"
    );
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "the host stays in the lobby rather than flying a bridge whose pane \
         names and station ids collide"
    );
    assert_eq!(
        app.world().resource::<LobbySelection>().0,
        project_phoenix::lobby::scenario_arbiter::ScenarioSelection::default(),
        "and the selection is released"
    );
    assert!(
        !project_phoenix::content_ledger::is_frozen(),
        "`PaneShadowsStation` is a failure point past `ingest_world`'s freeze, \
         so the unwind has to unfreeze the ledger as it does for an uncached hull"
    );

    // The same host with the collision removed loads that very pick — the
    // refusal is about the NAME, not about the profile or the scenario.
    app.world_mut()
        .resource_mut::<project_phoenix::native_host::app::AuthoredPaneLabels>()
        .0 = vec!["Ada".to_string()];
    select(&mut app, "phone-2", &scenario_id, &hull);
    pump(&mut app, 120);
    assert!(
        app.world().get_resource::<WorldConfig>().is_some(),
        "a lobby that refused one pick is still a lobby"
    );
}
