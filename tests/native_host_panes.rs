//! Local Station panes as ordinary logical clients (issue #1122).
//!
//! An *integration* test for the reason `tests/native_host_sim.rs` is: building
//! a native host populates the process-global native entity-template cache
//! (`config_cache::insert_native_config`), and inside the lib test binary that
//! would leak into thousands of unrelated unit tests (AGENTS.md).
//!
//! # What is here
//!
//! The half of issue #1122 that does **not** need an SDK, a GPU or a window —
//! which is deliberately the half the acceptance criteria are about. A pane's
//! `PaneBus` is the whole of how it talks to the simulation, and it is drivable
//! from a test with no document at all, so:
//!
//! * a pane joins, claims a Station and readies through exactly the contracts a
//!   phone uses (criterion 3's contract half — its *console surface* half is
//!   `tests/native_host_pane_ultralight.rs`, `#[ignore]`d because it needs the
//!   SDK);
//! * a pane's identity cannot be `LOCAL_CONSOLE_TOKEN`, and cannot be another
//!   pane's (criterion 4);
//! * a pane is admitted for its own Station's systems and refused for another's,
//!   by the ordinary `command_admission::policy` (criterion 4);
//! * a pane receives its own audience's projections and no others (criterion 4);
//! * a pane participant and a *transport* participant operate different Stations
//!   on the same ship concurrently (criterion 5, contract half — see below).
//!
//! # The deferral, and why it is exactly issue #1121's
//!
//! Criterion 5 asks for a **native and a browser** participant on different
//! Stations at once. The browser half of that is `#1112`, the Phoenix WebRTC
//! transport that replaces PeerJS: PeerJS is browser JavaScript and cannot run
//! in a native process, which is why #1121 deferred "browser clients join the
//! native host" to it and built the seam instead.
//!
//! So the pairing proved below is a pane participant and a
//! [`LoopbackTransport`] participant — the second being *literally the seam a
//! network transport plugs into*, entering by the same `TransportEvent` a
//! browser client's decoded message will. What is deferred is the socket, not
//! the contract, and the contract is what "operate different Stations
//! concurrently" is a claim about.

use bevy::prelude::*;

use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::core::messages::{ClientMessage, GamePhase, ServerMessage, SystemId};
use project_phoenix::delivery::args::ClientSource;
use project_phoenix::delivery::http::parse_request;
use project_phoenix::delivery::serve::{load_content, route, HostedDocuments, PeerOrigin, Route};
use project_phoenix::entities::template_preload::TemplatePreload;
use project_phoenix::lobby::handler::Target;
use project_phoenix::native_host::panes::identity::PaneIdentity;
use project_phoenix::native_host::panes::{LocalPanes, PaneId};
use project_phoenix::native_host::transport::{
    LoopbackHandle, NativeTransport, NativeTransportLink, PairedTransport, TransportDispatch,
};
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig,
};
use project_phoenix::ship::config::ShipConfig;

/// The flagship scenario, and the one the curated public catalogue publishes.
const WORLD: &str = "assets/worlds/combat_test.toml";
/// A fixed seed, so nothing here is a draw from the OS.
const SEED: u64 = 20260894;

fn preload() -> TemplatePreload {
    preload_content_templates(".").expect("the repository's own content preloads")
}

/// A native host over `WORLD` that waits in the lobby — the mode a crewed
/// session runs in, and the only one in which claiming a Station means anything.
fn crewed_config() -> NativeHostConfig {
    let mut cfg = NativeHostConfig::new(WORLD);
    cfg.seed = Some(SEED);
    cfg.solo = false;
    cfg.surface = NativeRenderSurface::Contract;
    cfg
}

/// Pump `app` for `frames` frames of fixed virtual time, as the headless
/// harness does.
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

/// Pump until the lobby has let go of the mission, or give up.
///
/// Collective `SetReady` does not start a mission on the spot: it starts the
/// lobby's countdown, which runs on the fixed clock, and the countdown's
/// `pending_phase` is `Loading` — the asset preloader owns the last step to
/// `InProgress`. So "everyone readied" and "the mission has left the lobby" are
/// seconds of simulated time apart, and a test that asserted the second
/// immediately after the first would be asserting the countdown does not exist.
fn pump_until_out_of_lobby(app: &mut App, max_frames: u64) -> GamePhase {
    for _ in 0..max_frames {
        pump(app, 1);
        let phase = app.world().resource::<State<GamePhase>>().get().clone();
        if phase != GamePhase::Lobby {
            return phase;
        }
    }
    app.world().resource::<State<GamePhase>>().get().clone()
}

/// The player hull's own station roster, read off the resource boot inserted.
fn ship_config(app: &App) -> ShipConfig {
    app.world()
        .resource::<project_phoenix::ship_plugin::PendingShipConfig>()
        .0
        .clone()
}

/// Two stations that each own at least one system, so a command aimed at one is
/// a command the other's holder must not be allowed to issue.
///
/// Chosen relationally rather than pinned by name: a designer renaming a station
/// on the destroyer should not have to edit this file.
fn two_stations_with_systems(config: &ShipConfig) -> (String, SystemId, String, SystemId) {
    let mut owned: Vec<(String, SystemId)> = Vec::new();
    for station in &config.stations {
        if let Some(system) = config
            .systems
            .iter()
            .find(|s| s.station.as_ref() == Some(&station.id))
        {
            owned.push((station.id.0.clone(), system.id.clone()));
        }
    }
    assert!(
        owned.len() >= 2,
        "this hull must have two stations owning systems for the concurrency claim to mean \
         anything; it has {}",
        owned.len()
    );
    let (a_station, a_system) = owned[0].clone();
    let (b_station, b_system) = owned[1].clone();
    (a_station, a_system, b_station, b_system)
}

/// Whether the ordinary admission policy would let `token` drive `system`.
///
/// The real `command_admission::policy::is_command_authorized`, against the live
/// session manager and the live ship — not a re-statement of its rules.
fn admits(app: &mut App, token: &str, system: &SystemId) -> bool {
    // Any ordinary payload: the policy's verdict here turns on station tenure,
    // not on what is being asked for. Deliberately not one the debug route
    // recognises, which would admit any registered player and prove nothing.
    let payload =
        project_phoenix::core::messages::SystemControlPayload::SetRedAlert { active: false };
    // `LocalShip` matters: NPC hulls carry their own `ShipConfigComponent` and
    // `ShipSystemControlSources`, and asking the wrong ship about a system it
    // does not have answers "unknown system — denying", which would make this
    // helper return false for reasons that have nothing to do with tenure.
    let mut ships = app.world_mut().query_filtered::<(
        &project_phoenix::ship_plugin::ShipConfigComponent,
        &project_phoenix::ship_plugin::ShipSystemControlSources,
    ), With<project_phoenix::server_app::LocalShip>>();
    let (config, sources) = ships
        .iter(app.world())
        .next()
        .map(|(c, s)| (c.0.clone(), s.0.clone()))
        .expect("the player's ship exists");
    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    project_phoenix::command_admission::policy::is_command_authorized(
        token,
        system,
        &payload,
        &project_phoenix::ship_plugin::ShipSystemControlSources(sources),
        sessions,
        &config,
        None,
    )
}

/// The three messages a participant sends to take a seat, **one per pumped
/// batch**.
///
/// Deliberately not all three at once. The lobby's per-variant systems share one
/// `FixedUpdate` set and are not chained, so three messages that arrive in the
/// same frame may be handled in any order — and a `SelectStation` handled before
/// its `Identify` is silently ignored, because the session it would seat does not
/// exist yet. A real participant never does that: `Identify` goes on connect and
/// the seat is a later click. Pumping between them is what makes this test model
/// a participant rather than a batch.
fn join_claim_ready(
    app: &mut App,
    send: &mut dyn FnMut(ClientMessage),
    token: &str,
    name: &str,
    station: &str,
) {
    send(ClientMessage::Identify {
        token: token.to_string(),
        name: name.to_string(),
    });
    pump(app, 4);
    send(ClientMessage::SelectStation {
        station: station.to_string(),
    });
    pump(app, 4);
    send(ClientMessage::SetReady { ready: true });
    pump(app, 4);
}

#[test]
fn a_pane_joins_claims_a_station_and_readies_through_the_ordinary_contracts() {
    // Acceptance criterion 3's contract half, and the premise of criterion 4:
    // there is no pane-shaped path through the lobby. A pane sends `Identify`,
    // `SelectStation` and `SetReady`, is answered by the ordinary broadcasts,
    // and holds its seat in the ordinary `SessionManager`.
    let preload = preload();
    let mut cfg = crewed_config();
    let panes = LocalPanes::open(&["Ada".to_string()], "127.0.0.1:0");
    let bus = panes.bus.clone();
    let pane = panes.opened[0].id;
    let token = panes.opened[0].identity.token().to_string();
    cfg.panes = Some(panes);
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");

    let config = ship_config(&app);
    let (station, _, _, _) = two_stations_with_systems(&config);

    bus.mark_live(pane);
    let mut send = |msg: ClientMessage| bus.submit(pane, msg).expect("the pane may say this");
    join_claim_ready(&mut app, &mut send, &token, "Ada", &station);
    let phase = pump_until_out_of_lobby(&mut app, 900);

    // The lobby answered on this pane's own token, exactly as it answers a
    // phone: a Welcome, and a seat.
    let dispatched = bus.take_outbound(pane);
    assert!(
        dispatched.iter().any(|d| d.json.contains("\"Welcome\"")),
        "the pane must be Welcomed: {:?}",
        dispatched
            .iter()
            .map(|d| &d.json[..40.min(d.json.len())])
            .collect::<Vec<_>>()
    );
    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    let player = sessions
        .0
        .players()
        .iter()
        .find(|p| p.token == token)
        .expect("an ordinary session token registers a session");
    assert_eq!(
        player.station.as_ref().map(|s| s.0.clone()),
        Some(station.clone()),
        "the pane holds the Station it claimed"
    );
    assert!(player.ready, "and readied");

    // One ready participant is the whole crew, so the collective auto-start
    // fired and the lobby handed the mission on. The LAST step —
    // `Loading` → `InProgress` — is the asset preloader's, and this host has no
    // real asset loaders (`NativeRenderSurface::Contract` composes the render
    // *contract*, not the renderer), so asserting it here would be asserting
    // something about the GPU. Issue #1121's `tests/native_viewscreen_render.rs`
    // owns that claim; what this one owns is that a pane's readiness is what
    // moved the lobby at all.
    assert_ne!(
        phase,
        GamePhase::Lobby,
        "collective SetReady auto-start is what starts a crewed mission, pane or phone"
    );
}

#[test]
fn a_panes_identity_can_never_be_the_host_operators_token() {
    // The single most important design point in this issue.
    // `console_bridge::LOCAL_CONSOLE_TOKEN` takes branch 2 of
    // `is_command_authorized` — `policy.accept_human_input`, with NO station
    // tenure check at all — and carries `ReturnToLobbyAuthority::Host`, which
    // can abort a mission that is still running. A pane built around it would
    // satisfy every other criterion and violate the one that matters.
    //
    // Three gates, and this asserts all three, because they close different
    // holes: minting, the pane bus, and the #1121 transport seam.
    let reserved = project_phoenix::console_bridge::LOCAL_CONSOLE_TOKEN;

    // 1. A pane cannot be CONFIGURED with it.
    assert!(
        PaneIdentity::adopt(reserved, "impostor").is_err(),
        "the host operator's token is not a participant identity"
    );

    let preload = preload();
    let mut cfg = crewed_config();
    let panes = LocalPanes::open(&["Ada".to_string()], "127.0.0.1:0");
    let bus = panes.bus.clone();
    let pane = panes.opened[0].id;
    cfg.panes = Some(panes);
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");

    // 2. A pane's PAGE cannot present it — `Identify` carries a token in its
    //    body, and `handle_identify` uses that one rather than the envelope's.
    bus.mark_live(pane);
    assert!(
        bus.submit(
            pane,
            ClientMessage::Identify {
                token: reserved.to_string(),
                name: "impostor".to_string(),
            },
        )
        .is_err(),
        "a pane may only identify as itself"
    );

    // 3. And even if something reached the seam with it, the seam refuses it.
    let loopback = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(loopback.transport()));
    loopback.send(
        reserved,
        ClientMessage::Identify {
            token: reserved.to_string(),
            name: "impostor".to_string(),
        },
    );
    pump(&mut app, 8);
    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    assert!(
        !sessions.0.players().iter().any(|p| p.token == reserved),
        "a reserved token must never become a session"
    );
}

#[test]
fn a_pane_cannot_read_another_panes_audience_projection() {
    // Criterion 4's projection half. Two panes on two Stations: every
    // `Audience::Holding*` has already resolved through
    // `SessionManager::holder_for_station` into a `Target::Token` by the time a
    // transport sees it, and the pane bus hands it to that pane and no other.
    //
    // The failure this rules out is a real temptation: an in-process transport
    // that broadcast every dispatch to every pane, on the reasoning that they
    // are all in one process anyway, would hand every station's private
    // projection to every pane and would do it silently.
    let preload = preload();
    let mut cfg = crewed_config();
    let panes = LocalPanes::open(&["Ada".to_string(), "Grace".to_string()], "127.0.0.1:0");
    let bus = panes.bus.clone();
    let (ada, grace) = (panes.opened[0].id, panes.opened[1].id);
    let ada_token = panes.opened[0].identity.token().to_string();
    let grace_token = panes.opened[1].identity.token().to_string();
    cfg.panes = Some(panes);
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");

    let config = ship_config(&app);
    let (station_a, _, station_b, _) = two_stations_with_systems(&config);
    bus.mark_live(ada);
    bus.mark_live(grace);
    let send = |id: PaneId, msg: ClientMessage| bus.submit(id, msg).expect("allowed");
    send(
        ada,
        ClientMessage::Identify {
            token: ada_token.clone(),
            name: "Ada".to_string(),
        },
    );
    send(
        grace,
        ClientMessage::Identify {
            token: grace_token.clone(),
            name: "Grace".to_string(),
        },
    );
    pump(&mut app, 4);
    send(
        ada,
        ClientMessage::SelectStation {
            station: station_a.clone(),
        },
    );
    send(
        grace,
        ClientMessage::SelectStation {
            station: station_b.clone(),
        },
    );
    pump(&mut app, 4);
    let _ = bus.take_outbound(ada);
    let _ = bus.take_outbound(grace);

    // A projection addressed to Ada's token — which is what an
    // `Audience::Holding(station_a)` becomes.
    let mut transport = bus.transport();
    transport.dispatch(TransportDispatch {
        target: &Target::Token(ada_token.clone()),
        msg: &ServerMessage::GameStarted,
        delivery: project_phoenix::core::messages::DeliveryClass::Reliable,
    });
    assert_eq!(bus.take_outbound(ada).len(), 1, "Ada's own projection");
    assert!(
        bus.take_outbound(grace).is_empty(),
        "Grace must not receive a projection addressed to Ada's Station"
    );

    // And the sessions really are on different Stations, so the audiences above
    // are genuinely distinct rather than accidentally equal.
    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    let held: Vec<Option<String>> = [&ada_token, &grace_token]
        .iter()
        .map(|t| {
            sessions
                .0
                .players()
                .iter()
                .find(|p| &&p.token == t)
                .and_then(|p| p.station.as_ref().map(|s| s.0.clone()))
        })
        .collect();
    assert_eq!(held, vec![Some(station_a), Some(station_b)]);
}

#[test]
fn a_pane_and_a_transport_participant_operate_different_stations_on_the_same_ship() {
    // Criterion 5, contract half. The pane arrives through `PaneTransport`; the
    // other participant arrives through `LoopbackTransport`, which is literally
    // the seam a network transport plugs into — the same `TransportEvent`, the
    // same `drain_native_inbound`, the same reserved-token gate. What is
    // deferred to issue #1112 is the socket, not the contract.
    //
    // The host runs `--solo`, so the mission is already under way with every
    // Station on `Backfill` and the player's ship in the world. Two participants
    // then arrive and take seats mid-mission, which is (a) the case
    // `handle_select_station_system`'s `InProgress` gate exists for, and (b) the
    // only way to reach a *running* simulation in a test binary with no GPU: a
    // crewed start hands off through `Loading`, and the asset preloader's last
    // step needs real asset loaders this profile does not compose.
    let preload = preload();
    let mut cfg = crewed_config();
    cfg.solo = true;
    let panes = LocalPanes::open(&["Ada".to_string()], "127.0.0.1:0");
    let bus = panes.bus.clone();
    let pane = panes.opened[0].id;
    let pane_token = panes.opened[0].identity.token().to_string();
    cfg.panes = Some(panes);
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");

    // One host, two transports. Issue #1112 owns the production version of this
    // composition (a network transport beside the panes); what it needs from
    // here is that the seam takes both without either seeing the other's
    // traffic.
    let network = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(PairedTransport::new(
        bus.transport(),
        network.transport(),
    )));

    let config = ship_config(&app);
    let (pane_station, pane_system, net_station, net_system) = two_stations_with_systems(&config);
    const NET_TOKEN: &str = "3f1a6c2e-0a11-4b3c-9d55-0000000000ff";

    // The solo mission is already running before either participant speaks.
    pump(&mut app, 8);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "a solo host is flying the mission before anybody joins"
    );

    bus.mark_live(pane);
    {
        let mut send = |msg: ClientMessage| bus.submit(pane, msg).expect("allowed");
        join_claim_ready(&mut app, &mut send, &pane_token, "Ada", &pane_station);
    }
    {
        let mut send = |msg: ClientMessage| network.send(NET_TOKEN, msg);
        join_claim_ready(&mut app, &mut send, NET_TOKEN, "Grace", &net_station);
    }
    pump(&mut app, 8);

    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    let seat = |token: &str| {
        sessions
            .0
            .players()
            .iter()
            .find(|p| p.token == token)
            .and_then(|p| p.station.as_ref().map(|s| s.0.clone()))
    };
    assert_eq!(
        seat(&pane_token),
        Some(pane_station.clone()),
        "the pane participant took its Station on the running ship"
    );
    assert_eq!(
        seat(NET_TOKEN),
        Some(net_station.clone()),
        "and the transport participant took a different one, concurrently"
    );

    // Each drives its own Station's systems and neither drives the other's —
    // the ordinary policy, applied to two participants who reached it by two
    // different transports.
    assert!(admits(&mut app, &pane_token, &pane_system));
    assert!(!admits(&mut app, &pane_token, &net_system));
    assert!(admits(&mut app, NET_TOKEN, &net_system));
    assert!(!admits(&mut app, NET_TOKEN, &pane_system));

    // And their projections stayed apart across the two transports: the
    // network participant's Welcome never reached the pane's queue.
    let pane_traffic = bus.take_outbound(pane);
    assert!(
        pane_traffic
            .iter()
            .all(|d| !d.json.contains(NET_TOKEN) || !d.json.contains("\"Welcome\"")),
        "a pane must not receive another participant's private Welcome"
    );
    assert!(
        network
            .drain_outbound()
            .iter()
            .any(
                |(target, msg, _)| matches!(msg, ServerMessage::Welcome { .. })
                    && target == &Target::Token(NET_TOKEN.to_string())
            ),
        "the transport participant was Welcomed on its own token"
    );
}

#[test]
fn a_panes_identity_is_in_its_url_and_never_in_the_body_the_host_serves() {
    // The finding, end to end and through the real router. `phoenix-host` binds
    // 0.0.0.0:8080 by default with no TLS and no authentication — that is what
    // PRD #855 built, because the audience is phones on a LAN. A live
    // participant's session token in a body served on that listener would be a
    // seat on the bridge available to anyone on the network.
    //
    // Three separate defences, asserted here as a chain rather than
    // individually: the identity is in the URL fragment (which a browser never
    // transmits), the path carries a per-pane nonce (so it cannot be
    // enumerated), and the document is served only to a loopback peer.
    let panes = LocalPanes::open(&["Ada".to_string()], "127.0.0.1:8080");
    let documents = HostedDocuments::default();
    let token = panes.opened[0].identity.token().to_string();
    let page = "<html><head></head><body></body></html>";
    panes
        .publish(page, &documents)
        .expect("the client page becomes a pane document");

    let path = panes
        .bus
        .document_path(panes.opened[0].id)
        .expect("the pane's document is published somewhere");
    let url = panes.urls()[0].clone();
    let (address, fragment) = url.split_once('#').expect("a pane URL carries a fragment");

    // 1. The identity is in the fragment and in nothing else.
    assert!(fragment.contains(&token.replace('_', "%5F")));
    assert!(fragment.contains("Ada"));
    assert!(
        !address.contains(&token),
        "the token must not be in the part of the URL a browser transmits: {address}"
    );
    let served = documents.get(&path).expect("the document is published");
    assert!(
        !served.contains(&token),
        "a pane's session token must never appear in a served body"
    );
    assert!(!served.contains("Ada"), "nor the name it joins under");

    // 2. The path is not `/client/pane-0.html` and cannot be guessed from the
    //    pane's number.
    assert!(path.starts_with("/client/pane-0-") && path.ends_with(".html"));
    assert!(
        documents.get("/client/pane-0.html").is_none(),
        "the enumerable path must not resolve"
    );

    // 3. And the real router hands it out only to this machine.
    let fx_dir = std::env::temp_dir().join("phoenix-pane-serve-gate");
    let _ = std::fs::remove_dir_all(&fx_dir);
    std::fs::create_dir_all(fx_dir.join("assets/worlds")).unwrap();
    std::fs::write(
        fx_dir.join("assets/scenarios.toml"),
        "[content]\nid = \"phoenix-base\"\nepoch = 1\n",
    )
    .unwrap();
    let content = load_content(&fx_dir.to_string_lossy(), "assets/scenarios.toml")
        .expect("the fixture manifest loads");
    let client = ClientSource::Bundled {
        dir: "dist".to_string(),
    };
    let req = parse_request(&format!("GET {path} HTTP/1.1\r\n")).unwrap();
    assert!(
        matches!(
            route(&req, &content, &client, &documents, PeerOrigin::Loopback),
            Route::Document { .. }
        ),
        "the pane's own view, from this machine, is served"
    );
    assert!(
        !matches!(
            route(&req, &content, &client, &documents, PeerOrigin::Remote),
            Route::Document { .. }
        ),
        "the same request from the LAN is not"
    );

    // 4. And a closed pane's path stops resolving at all — the pane's id is
    //    never reissued, so there is nothing this document could ever be for
    //    again.
    panes.bus.close(panes.opened[0].id);
    assert_eq!(panes.bus.document_path(panes.opened[0].id), None);
    assert!(documents.get(&path).is_none());
    assert!(
        !matches!(
            route(&req, &content, &client, &documents, PeerOrigin::Loopback),
            Route::Document { .. }
        ),
        "a closed pane's document must stop being served, even to this machine"
    );
    let _ = std::fs::remove_dir_all(&fx_dir);
}

#[test]
fn a_pane_that_closes_hands_the_lobby_the_disconnect_a_dropped_phone_would() {
    // Criterion 4 again, at the other end of the lifecycle: a pane that goes
    // away is a participant that dropped. The station keeps its holder and flips
    // to Backfill (AGENTS.md rule 5) — it is not vacated, and nothing about the
    // pane's departure is a native-only path.
    let preload = preload();
    let mut cfg = crewed_config();
    let panes = LocalPanes::open(&["Ada".to_string()], "127.0.0.1:0");
    let bus = panes.bus.clone();
    let pane = panes.opened[0].id;
    let token = panes.opened[0].identity.token().to_string();
    cfg.panes = Some(panes);
    let mut app = build_native_host_app(&cfg, &preload).expect("the native host assembles");

    let config = ship_config(&app);
    let (station, _, _, _) = two_stations_with_systems(&config);
    bus.mark_live(pane);
    {
        let mut send = |msg: ClientMessage| bus.submit(pane, msg).expect("allowed");
        join_claim_ready(&mut app, &mut send, &token, "Ada", &station);
    }

    bus.close(pane);
    pump(&mut app, 8);

    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    let player = sessions
        .0
        .players()
        .iter()
        .find(|p| p.token == token)
        .expect("a dropped participant keeps its session");
    assert!(!player.connected, "the pane's participant is disconnected");
    assert_eq!(
        player.station.as_ref().map(|s| s.0.clone()),
        Some(station),
        "and keeps the Station it held, which is what flips it to Backfill"
    );
}
