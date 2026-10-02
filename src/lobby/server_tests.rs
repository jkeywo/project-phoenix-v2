use super::*;

include!("outbox_access_tests.rs");

fn local_gm(app: &mut App, connected: bool, ready: bool) {
    app.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
            id: "native-gm".into(),
            name: "GM".into(),
            connected,
            ready,
        }])
        .unwrap(),
    );
}

#[test]
fn local_gm_readiness_starts_and_cancels_the_shared_countdown() {
    let mut app = test_app();
    app.insert_resource(crate::world::config::WorldConfig::default());
    local_gm(&mut app, true, false);
    push(
        &mut app,
        "crew",
        ClientMessage::Identify {
            token: "crew".into(),
            name: "Ada".into(),
        },
    );
    tick(&mut app);
    push(&mut app, "crew", ClientMessage::SetReady { ready: true });
    tick(&mut app);
    assert_eq!(app.world().resource::<CountdownTimer>().remaining_secs, 0.0);
    local_gm(&mut app, true, true);
    let out = tick(&mut app);
    assert!(out.iter().any(|m| matches!(
        m.msg,
        ServerMessage::GameStartCountdown { remaining_secs: 5 }
    )));
    local_gm(&mut app, false, true);
    let out = tick(&mut app);
    assert_eq!(app.world().resource::<CountdownTimer>().remaining_secs, 0.0);
    assert!(out.iter().any(|m| matches!(
        m.msg,
        ServerMessage::GameStartCountdown { remaining_secs: 0 }
    )));
    local_gm(&mut app, true, false);
    tick(&mut app);
    assert_eq!(app.world().resource::<CountdownTimer>().remaining_secs, 0.0);
}

#[test]
fn local_gm_can_ready_an_ai_ship_only_after_a_world_is_selected() {
    let mut app = test_app();
    local_gm(&mut app, true, true);
    tick(&mut app);
    assert_eq!(app.world().resource::<CountdownTimer>().remaining_secs, 0.0);
    app.insert_resource(crate::world::config::WorldConfig::default());
    tick(&mut app);
    assert!(app.world().resource::<CountdownTimer>().remaining_secs > 0.0);
    // Several fixed ticks can run before the queued phase transition.
    // A completed countdown must not restart within that same frame.
    app.world_mut()
        .resource_mut::<CountdownTimer>()
        .remaining_secs = 0.0;
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.world_mut().run_schedule(FixedUpdate);
    assert_eq!(app.world().resource::<CountdownTimer>().remaining_secs, 0.0);
}

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut outbox: ResMut<Outbox>) {
    for ev in reader.read() {
        outbox.0.push(ev.clone());
    }
}

pub(super) fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(LobbyPlugin)
        .add_plugins(lobby_outbox_broadcaster())
        .add_plugins(bevy::time::TimePlugin)
        .init_resource::<Outbox>()
        .add_systems(PostUpdate, collect);
    crate::sim_tick::register_sim_tick(&mut app);
    // One fixed step per update (issue #895): the lobby runs on the
    // logical tick, and each 1 s harness tick advances it once — the
    // countdown tests count whole seconds per tick.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_secs_f32(1.0),
    );
    app
}

pub(super) fn push(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}

pub(super) fn tick(app: &mut App) -> Vec<OutboundMessage> {
    app.update();
    let msgs = app.world().resource::<Outbox>().0.clone();
    app.world_mut().resource_mut::<Outbox>().0.clear();
    msgs
}

#[test]
fn client_system_kind_projection_preserves_arbitrary_authored_instance_ids() {
    use crate::core::messages::SystemId;
    use crate::ship::config::SystemInstanceConfig;

    let system = |id: &str, kind: &str| SystemInstanceConfig {
        id: SystemId(id.into()),
        kind: kind.into(),
        station: None,
        ai_only: false,
        human_seeking: false,
        seek_order: vec![],
        power_group: None,
        marker: None,
        config: None,
    };
    let projected = project_system_kinds(&[
        system("port-flight-vector", "helm_steering"),
        system("berthing-clamps", "dock"),
        system("pulse-reservoir-seven", "helm_boost"),
    ]);

    assert_eq!(
        projected,
        std::collections::HashMap::from([
            (
                "port-flight-vector".to_string(),
                "helm_steering".to_string()
            ),
            ("berthing-clamps".to_string(), "dock".to_string()),
            (
                "pulse-reservoir-seven".to_string(),
                "helm_boost".to_string()
            ),
        ])
    );
}

#[test]
fn identify_arrives_via_inbound_message_and_welcome_is_sent_via_outbound() {
    let mut app = test_app();
    push(
        &mut app,
        "peer-id",
        ClientMessage::Identify {
            token: "t1".into(),
            name: "Alice".into(),
        },
    );
    let out = tick(&mut app);
    assert!(out
        .iter()
        .any(|m| matches!(&m.msg, ServerMessage::Welcome { .. })));
}

#[test]
fn identify_welcome_projects_the_separate_gm_resource() {
    let mut app = test_app();
    app.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: true,
            ready: false,
        }])
        .unwrap(),
    );
    push(
        &mut app,
        "peer-id",
        ClientMessage::Identify {
            token: "t1".into(),
            name: "Alice".into(),
        },
    );

    let out = tick(&mut app);
    let gms = out.iter().find_map(|outbound| match &outbound.msg {
        ServerMessage::Welcome { state, gms, .. } => {
            assert_eq!(state.players.len(), 1);
            Some(gms)
        }
        _ => None,
    });
    assert_eq!(gms.unwrap()[0].id, "gm-1");
    assert_eq!(app.world().resource::<Sessions>().0.players().len(), 1);
}

#[test]
fn select_station_works_during_in_progress_phase() {
    use crate::core::messages::StationId;
    use crate::lobby::stations_config::stations_from_ship_config;
    use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
    use std::collections::HashMap;

    let mut app = test_app();

    // Phase starts at Lobby by default.
    // Add a ship with station config before startup so
    // update_session_with_config sees non-empty stations.
    let ship_config = ShipConfig {
        stations: vec![
            StationConfig {
                id: StationId("helm".into()),
                name: "Helm".into(),
                description: "Helm station".into(),
                rank: "Crew".into(),
                short_code: "H".into(),
                ratings: vec![StationRatingConfig {
                    detailed_systems: None,
                    name: "Std".into(),
                    automated_systems: vec![],
                    ai_tuning: None,
                }],
                console: None,
                manual_overview: None,
                tutorials: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
                command_target: None,
                stances: vec![],
            },
            StationConfig {
                id: StationId("tactical".into()),
                name: "Tactical".into(),
                description: "Tactical station".into(),
                rank: "Crew".into(),
                short_code: "T".into(),
                ratings: vec![StationRatingConfig {
                    detailed_systems: None,
                    name: "Std".into(),
                    automated_systems: vec![],
                    ai_tuning: None,
                }],
                console: None,
                manual_overview: None,
                tutorials: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
                command_target: None,
                stances: vec![],
            },
        ],
        systems: vec![],
        power_groups: HashMap::new(),
        coordination_lag_secs: 2.0,
    };
    app.world_mut()
        .insert_resource(stations_from_ship_config(&ship_config));
    app.world_mut()
        .insert_resource(ShipClientConfigResource::default());

    // Verify stations are populated
    {
        let stations = app.world().resource::<ShipStations>();
        assert!(
            !stations.stations.is_empty(),
            "ShipStations must be non-empty"
        );
        assert_eq!(stations.stations.len(), 2, "expected 2 stations");
    }

    // Register two players in lobby first.
    // The peer ID (first arg to push) is the session token sent by the bridge,
    // and the Identify message body carries the same token for registration.
    push(
        &mut app,
        "t1",
        ClientMessage::Identify {
            token: "t1".into(),
            name: "Player1".into(),
        },
    );
    push(
        &mut app,
        "t2",
        ClientMessage::Identify {
            token: "t2".into(),
            name: "Player2".into(),
        },
    );
    tick(&mut app);

    // Player1 claims Helm in lobby
    push(
        &mut app,
        "t1",
        ClientMessage::SelectStation {
            station: "Helm".into(),
        },
    );
    let out = tick(&mut app);
    assert!(out.iter().any(|m| {
        matches!(&m.msg, ServerMessage::StationAssigned { token, station, .. }
                if token == "t1" && station == &Some("Helm".into()))
    }));

    // Start the game — both ready triggers countdown
    push(&mut app, "t1", ClientMessage::SetReady { ready: true });
    push(&mut app, "t2", ClientMessage::SetReady { ready: true });
    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::GameStartCountdown { .. })),
        "ready should start countdown"
    );

    // Fast-forward the countdown by advancing the timer directly.
    use crate::lobby::CountdownTimer;
    app.world_mut()
        .resource_mut::<CountdownTimer>()
        .remaining_secs = 0.001;
    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::GameStarted)),
        "countdown expiry must emit GameStarted"
    );

    // Now in InProgress: Player2 claims Tactical (was unclaimed)
    push(
        &mut app,
        "t2",
        ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
    );
    let out = tick(&mut app);
    assert!(
        out.iter().any(|m| {
            matches!(&m.msg, ServerMessage::StationAssigned { token, station, .. }
                    if token == "t2" && station == &Some("Tactical".into()))
        }),
        "SelectStation should work during InProgress phase"
    );
}

#[test]
fn release_station_works_during_in_progress_phase() {
    use crate::core::messages::StationId;
    use crate::lobby::stations_config::stations_from_ship_config;
    use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
    use std::collections::HashMap;

    let mut app = test_app();

    // Phase starts at Lobby by default.
    let ship_config = ShipConfig {
        stations: vec![StationConfig {
            id: StationId("helm".into()),
            name: "Helm".into(),
            description: "Helm station".into(),
            rank: "Crew".into(),
            short_code: "H".into(),
            ratings: vec![StationRatingConfig {
                detailed_systems: None,
                name: "Std".into(),
                automated_systems: vec![],
                ai_tuning: None,
            }],
            console: None,
            manual_overview: None,
            tutorials: vec![],
            human_seeking: false,
            host_order: vec![],
            visiting_rating: None,
            auxiliary: false,
            command_target: None,
            stances: vec![],
        }],
        systems: vec![],
        power_groups: HashMap::new(),
        coordination_lag_secs: 2.0,
    };
    app.world_mut()
        .insert_resource(stations_from_ship_config(&ship_config));
    app.world_mut()
        .insert_resource(ShipClientConfigResource::default());

    // Register player and claim station in lobby.
    // The peer ID (first arg to push) is the session token sent by the bridge.
    push(
        &mut app,
        "t1",
        ClientMessage::Identify {
            token: "t1".into(),
            name: "Player1".into(),
        },
    );
    tick(&mut app);

    push(
        &mut app,
        "t1",
        ClientMessage::SelectStation {
            station: "Helm".into(),
        },
    );
    tick(&mut app);

    // Start the game — single player ready triggers countdown
    push(&mut app, "t1", ClientMessage::SetReady { ready: true });
    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::GameStartCountdown { .. })),
        "ready should start countdown"
    );

    // Fast-forward the countdown by advancing the timer directly.
    use crate::lobby::CountdownTimer;
    app.world_mut()
        .resource_mut::<CountdownTimer>()
        .remaining_secs = 0.001;
    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::GameStarted)),
        "countdown expiry must emit GameStarted"
    );

    // Now in InProgress: Player1 releases Helm
    push(&mut app, "t1", ClientMessage::ReleaseStation);
    let out = tick(&mut app);
    assert!(
        out.iter().any(|m| {
            matches!(&m.msg, ServerMessage::StationAssigned { token, station, .. }
                    if token == "t1" && station.is_none())
        }),
        "ReleaseStation should work during InProgress phase"
    );
}

#[test]
fn selected_ship_resource_populates_ship_stations_via_update_session() {
    use crate::core::messages::StationId;
    use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
    use std::collections::HashMap;

    let mut app = test_app();

    // Insert PendingShipConfig so update_session_with_config uses it.
    // ShipStations starts empty (init_resource in LobbyPlugin).
    let ship_config = ShipConfig {
        stations: vec![
            StationConfig {
                id: StationId("helm".into()),
                name: "Helm".into(),
                description: "Helm station".into(),
                rank: "Crew".into(),
                short_code: "H".into(),
                ratings: vec![StationRatingConfig {
                    detailed_systems: None,
                    name: "Std".into(),
                    automated_systems: vec![],
                    ai_tuning: None,
                }],
                console: None,
                manual_overview: None,
                tutorials: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
                command_target: None,
                stances: vec![],
            },
            StationConfig {
                id: StationId("tactical".into()),
                name: "Tactical".into(),
                description: "Tactical station".into(),
                rank: "Crew".into(),
                short_code: "T".into(),
                ratings: vec![StationRatingConfig {
                    detailed_systems: None,
                    name: "Std".into(),
                    automated_systems: vec![],
                    ai_tuning: None,
                }],
                console: None,
                manual_overview: None,
                tutorials: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
                command_target: None,
                stances: vec![],
            },
        ],
        systems: vec![],
        power_groups: HashMap::new(),
        coordination_lag_secs: 2.0,
    };
    app.world_mut()
        .insert_resource(crate::ship_plugin::PendingShipConfig(ship_config.clone()));

    // First update runs Startup systems including update_session_with_config
    app.update();

    // Assert stations were populated from PendingShipConfig
    let stations = app.world().resource::<ShipStations>();
    assert_eq!(stations.stations.len(), 2);
    assert_eq!(stations.stations[0].id.0, "helm");
    assert_eq!(stations.stations[0].name, "Helm");
    assert_eq!(stations.stations[1].id.0, "tactical");
    assert_eq!(stations.stations[1].name, "Tactical");
}

/// One station, enough to tell "the roster was built" from "it was not".
#[cfg(test)]
fn one_station_ship_config() -> crate::ship::config::ShipConfig {
    use crate::core::messages::StationId;
    use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
    ShipConfig {
        stations: vec![StationConfig {
            id: StationId("helm".into()),
            name: "Helm".into(),
            description: "Helm station".into(),
            rank: "Crew".into(),
            short_code: "H".into(),
            ratings: vec![StationRatingConfig {
                detailed_systems: None,
                name: "Std".into(),
                automated_systems: vec![],
                ai_tuning: None,
            }],
            console: None,
            manual_overview: None,
            tutorials: vec![],
            human_seeking: false,
            host_order: vec![],
            visiting_rating: None,
            auxiliary: false,
            command_target: None,
            stances: vec![],
        }],
        systems: vec![],
        power_groups: std::collections::HashMap::new(),
        coordination_lag_secs: 2.0,
    }
}

/// A STANDALONE browser game master owns a ship, and its ship gets stations.
///
/// The landing's Host as GM route picks a World and a hull like any host, so
/// this boot arrives with a `PendingShipConfig` — and its ship needs the same
/// station roster every other ship gets, because "crew on AI backfill" is a
/// rating on a station, not the absence of one. Before this, the marker alone
/// short-circuited the system and the standalone GM's ship had no stations at
/// all.
#[test]
fn browser_gm_with_a_selected_hull_still_gets_its_stations() {
    let mut app = test_app();
    app.world_mut()
        .insert_resource(crate::gm_projection::BrowserGameMaster);
    app.world_mut()
        .insert_resource(crate::ship_plugin::PendingShipConfig(
            one_station_ship_config(),
        ));

    app.update();

    let stations = app.world().resource::<ShipStations>();
    assert_eq!(stations.stations.len(), 1);
    assert_eq!(stations.stations[0].id.0, "helm");
}

/// A JOINED browser game master owns no ship, and must not be given one.
///
/// The other half of the pair: no `PendingShipConfig`, no
/// `SelectedShipResource`, so nothing was ever selected — and the native
/// filesystem fallback below the guard must not answer for it with whatever
/// hull happens to be on disk.
#[test]
fn browser_gm_with_no_selected_hull_gets_no_stations() {
    let mut app = test_app();
    app.world_mut()
        .insert_resource(crate::gm_projection::BrowserGameMaster);

    app.update();

    assert!(app.world().resource::<ShipStations>().stations.is_empty());
}

// ── #773: system_extras extraction from real hull assets ──────────────────

/// Parse a real hull TOML into an `EntityConfig`, run the full manual
/// pipeline (`build_manual_system_extras` + `build_ship_manual`), and return
/// the resulting manual alongside the config it was built from. This is the
/// integration path the pure `ship::manual` module can't cover on its own
/// (it never sees `EntityConfig`).
fn manual_from_hull(
    path: &str,
) -> (
    crate::entities::config::EntityConfig,
    crate::ship::manual::ShipManualWire,
) {
    // Through the include resolver (issue #906) — the same document the
    // runtime hull load produces, composed or not.
    let config = crate::entities::include_resolve::load_entity_config(path)
        .unwrap_or_else(|e| panic!("parse {path}: {e}"));
    let topology = config
        .ship_config
        .clone()
        .expect("hull declares a ship_config");
    let extras = build_manual_system_extras(&config);
    let registry = crate::ship::manual::ManualProviderRegistry::with_shipped_providers();
    let manual = crate::ship::manual::build_ship_manual(&topology, &registry, &extras);
    (config, manual)
}

#[test]
fn shared_client_config_projector_preserves_complete_non_default_authored_values() {
    let (config, _) = manual_from_hull("assets/entities/alliance_cruiser.toml");
    let projected = project_ship_client_config(&config);

    assert_eq!(projected.helm_radar_range, 93.75);
    assert_eq!(projected.helm_radar_shows[0], "player");
    assert_eq!(projected.sensors_radar_range, 300.0);
    assert_eq!(
        projected.sensors_radar_selects,
        ["ship", "station", "planet"]
    );
    assert_eq!(projected.nav_chart_range, 800.0);
    assert_eq!(
        projected.nav_chart_selects,
        ["station", "planet", "star", "region"]
    );
    assert_eq!(projected.hostile_arc_color, [1.0, 0.3, 0.3, 0.07]);
    assert_eq!(
        projected
            .phaser_banks
            .iter()
            .map(|bank| (bank.id.as_str(), bank.facing_deg, bank.fire_arc_deg))
            .collect::<Vec<_>>(),
        [("fore", 0.0, 270.0), ("aft", 180.0, 270.0)]
    );
    assert_eq!(
        projected
            .torpedo_tubes
            .iter()
            .map(|tube| (tube.id.as_str(), tube.facing_deg, tube.fire_arc_deg))
            .collect::<Vec<_>>(),
        [
            ("fore_port", 0.0, 90.0),
            ("fore_starboard", 0.0, 90.0),
            ("aft", 180.0, 90.0),
        ]
    );
    assert_eq!(projected.class.as_deref(), Some("cruiser"));
    assert_eq!(projected.hull_id.as_deref(), Some("AEV-1864"));
    assert_eq!(projected.power_rating, Some(90));
    assert_eq!(
        projected.ship_css.as_deref(),
        Some("gui/themes/cruiser.css")
    );
    assert!(
        projected
            .station_tutorials
            .get("helm")
            .is_some_and(|tutorials| tutorials.iter().any(|entry| entry.id == "helm-welcome")),
        "the complete ordinary Welcome config carries authored tutorials"
    );

    let topology = config.ship_config.as_ref().expect("cruiser topology");
    let expected_gaps = topology
        .stations
        .iter()
        .map(|station| {
            (
                station.id.0.clone(),
                crate::ship::eligibility::projected_assist_gaps(station, topology),
            )
        })
        .filter(|(_, gaps)| !gaps.is_empty())
        .collect::<std::collections::HashMap<_, _>>();
    assert!(
        !expected_gaps.is_empty(),
        "fixture must exercise assist gaps"
    );
    assert_eq!(projected.station_assist_gaps, expected_gaps);
    assert_eq!(projected.station_systems["helm"], projected.helm_systems);
    assert_eq!(projected.system_kinds["helm-thrust"], "helm_thrust");
}

fn find_metric(
    manual: &crate::ship::manual::ShipManualWire,
    kind: &str,
    code: &str,
) -> Option<f64> {
    manual
        .stations
        .iter()
        .flat_map(|s| &s.sections)
        .find(|sec| sec.kind == kind)
        .and_then(|sec| sec.metrics.iter().find(|m| m.code == code))
        .map(|m| m.value)
}

#[test]
fn real_hulls_produce_different_manual_values() {
    let (cruiser_cfg, cruiser) = manual_from_hull("assets/entities/alliance_cruiser.toml");
    let (courier_cfg, courier) = manual_from_hull("assets/entities/alliance_courier.toml");

    // Reactor capacity reflects each hull's own authored [power] capacity.
    let cruiser_cap = find_metric(
        &cruiser,
        crate::ship::system_registry::POWER_REACTOR_KIND,
        "capacity",
    );
    let courier_cap = find_metric(
        &courier,
        crate::ship::system_registry::POWER_REACTOR_KIND,
        "capacity",
    );
    assert_eq!(
        cruiser_cap,
        cruiser_cfg.power.as_ref().map(|p| p.capacity as f64)
    );
    assert_eq!(
        courier_cap,
        courier_cfg.power.as_ref().map(|p| p.capacity as f64)
    );
    assert_ne!(
        cruiser_cap, courier_cap,
        "manual content must change with ship configuration (AC2)"
    );

    // Comms range likewise reflects each hull's own authored [comms] range.
    let cruiser_comms = find_metric(&cruiser, crate::ship::system_registry::COMMS_KIND, "range");
    let courier_comms = find_metric(&courier, crate::ship::system_registry::COMMS_KIND, "range");
    assert_eq!(
        cruiser_comms,
        cruiser_cfg.comms.as_ref().map(|c| c.range as f64)
    );
    assert_eq!(
        courier_comms,
        courier_cfg.comms.as_ref().map(|c| c.range as f64)
    );
    assert_ne!(
        cruiser_comms, courier_comms,
        "manual content must change with ship configuration (AC2)"
    );
}

#[test]
fn cruiser_manual_covers_weapons_helm_and_sensors_from_authored_config() {
    let (cfg, cruiser) = manual_from_hull("assets/entities/alliance_cruiser.toml");

    // Phaser bank beam range reflects the authored config, not a pinned number.
    let authored_beam_range = cfg
        .weapons_console
        .as_ref()
        .and_then(|w| w.phaser_banks.first())
        .map(|b| b.beam_range as f64);
    assert!(authored_beam_range.is_some(), "hull authors a phaser bank");
    assert_eq!(
        find_metric(
            &cruiser,
            crate::ship::system_registry::PHASER_BANK_KIND,
            "beam_range"
        ),
        authored_beam_range
    );
    // Torpedo magazine capacity and tube count reflect the authored [torpedoes] block.
    let torpedoes = cfg.torpedoes.as_ref().expect("hull authors torpedoes");
    assert_eq!(
        find_metric(
            &cruiser,
            crate::ship::system_registry::TORPEDO_MAGAZINE_KIND,
            "capacity"
        ),
        Some(torpedoes.count as f64)
    );
    assert_eq!(
        find_metric(
            &cruiser,
            crate::ship::system_registry::TORPEDO_MAGAZINE_KIND,
            "tubes"
        ),
        Some(torpedoes.tubes.len() as f64)
    );
    // Sensors long-range radar range reflects the authored [sensors_console].
    assert_eq!(
        find_metric(
            &cruiser,
            crate::ship::system_registry::SENSORS_KIND,
            "range"
        ),
        cfg.sensors_console
            .as_ref()
            .map(|s| s.long_range_radar.range as f64)
    );

    // Helm movement mode: no `[helm_capability]` authored ⇒ effective planar.
    let helm = cruiser
        .stations
        .iter()
        .flat_map(|s| &s.sections)
        .find(|sec| sec.kind == crate::ship::system_registry::HELM_THRUST_KIND)
        .expect("helm section present");
    assert_eq!(
        helm.capabilities
            .iter()
            .find(|c| c.code == "movement_mode")
            .map(|c| c.value_code.as_str()),
        Some("planar")
    );
    // And the authored [helm_console] max speed is reflected, not a pinned number.
    assert_eq!(
        helm.metrics
            .iter()
            .find(|m| m.code == "max_speed")
            .map(|m| m.value),
        cfg.helm_console.as_ref().map(|h| h.max_speed as f64)
    );
}

#[test]
fn courier_manual_covers_its_blaster_bank() {
    // The courier carries a blaster, not torpedoes — proving the blaster
    // provider is fed from real authored config.
    let (cfg, courier) = manual_from_hull("assets/entities/alliance_courier.toml");
    let authored_range = cfg
        .weapons_console
        .as_ref()
        .and_then(|w| w.blaster_banks.first())
        .map(|b| b.range as f64);
    assert!(authored_range.is_some(), "hull authors a blaster bank");
    assert_eq!(
        find_metric(
            &courier,
            crate::ship::system_registry::BLASTER_BANK_KIND,
            "range"
        ),
        authored_range
    );
}

fn automatic_grant(sequence: u64) -> crate::lobby::start_policy::StartGrant {
    crate::lobby::start_policy::StartGrant {
        id: format!("start-{sequence}"),
        mode: crate::lobby::start_policy::StartGrantMode::Automatic,
        operator_id: None,
        apply_tick: 0,
    }
}

fn forced_grant(sequence: u64, operator_id: &str) -> crate::lobby::start_policy::StartGrant {
    crate::lobby::start_policy::StartGrant {
        id: format!("start-{sequence}"),
        mode: crate::lobby::start_policy::StartGrantMode::Forced,
        operator_id: Some(operator_id.into()),
        apply_tick: 0,
    }
}

fn enable_managed_lobby(app: &mut App, validation_passed: bool) {
    let mut managed = app.world_mut().resource_mut::<FleetManagedLobby>();
    managed.set_enabled(true);
    managed.validation_passed = validation_passed;
}

fn install_two_participant_fleet(app: &mut App, local: u32, owner: u32, delay: u64) {
    use crate::command_admission::HostSlot;

    app.init_resource::<crate::command_admission::log::PendingCommands>();
    crate::lockstep::register_lockstep(app);
    let participants = vec![HostSlot(1), HostSlot(2)];
    let roster = crate::lockstep::FleetRoster::with_participants(
        vec![crate::lockstep::FleetShip::new(HostSlot(1))],
        participants.clone(),
        HostSlot(local),
        HostSlot(owner),
    )
    .unwrap();
    app.insert_resource(roster);
    app.insert_resource(crate::lockstep::FleetLockstep(
        crate::lockstep::LockstepSession::new_at(HostSlot(local), participants, delay, 0).unwrap(),
    ));
    for peer in [HostSlot(1), HostSlot(2)] {
        if peer != HostSlot(local) {
            app.world_mut()
                .resource_mut::<crate::lockstep::FleetLockstep>()
                .observe(peer, u64::MAX);
        }
    }
}

fn install_gm_owner_fleet(app: &mut App, local: u32, delay: u64) {
    use crate::command_admission::HostSlot;

    app.init_resource::<crate::command_admission::log::PendingCommands>();
    crate::lockstep::register_lockstep(app);
    let participants = vec![HostSlot(1), HostSlot(2), HostSlot(3)];
    let roster = crate::lockstep::FleetRoster::with_participants(
        vec![
            crate::lockstep::FleetShip::new(HostSlot(2)),
            crate::lockstep::FleetShip::new(HostSlot(3)),
        ],
        participants.clone(),
        HostSlot(local),
        HostSlot(1),
    )
    .unwrap();
    app.insert_resource(roster);
    let mut session =
        crate::lockstep::LockstepSession::new_at(HostSlot(local), participants, delay, 0).unwrap();
    // Keep every surviving ship peer ahead of this narrow start-boundary
    // fixture. The GM owner's opening watermark remains exactly `delay`, so
    // its loss is agreed at the same tick the embedded grant names.
    for survivor in [HostSlot(2), HostSlot(3)] {
        if survivor != HostSlot(local) {
            session.observe(survivor, u64::MAX);
        }
    }
    app.insert_resource(crate::lockstep::FleetLockstep(session));
}

fn take_start_results(app: &mut App) -> Vec<crate::lobby::start_policy::StartGrantResult> {
    app.world_mut()
        .resource_mut::<StartGrantResults>()
        .drain()
        .collect()
}

#[test]
fn managed_automatic_grant_is_the_readiness_boundary_for_an_empty_ship_host() {
    let mut app = test_app();
    enable_managed_lobby(&mut app, true);
    assert_eq!(
        app.world().resource::<Sessions>().0.readiness_tally(),
        crate::lobby::start_policy::ReadinessTally::default(),
        "this peer must not invent a local participant"
    );
    assert!(app
        .world_mut()
        .resource_mut::<PendingStartGrants>()
        .try_push(automatic_grant(1)));

    let source_tick = app.world().resource::<crate::sim_tick::SimTick>().0;
    let out = tick(&mut app);
    assert!(out
        .iter()
        .any(|message| matches!(message.msg, ServerMessage::GameStarted)));
    let result = take_start_results(&mut app).pop().unwrap();
    assert_eq!(
        result.status,
        crate::lobby::start_policy::StartGrantStatus::Applied
    );
    assert_eq!(result.tick, source_tick);
    assert_eq!(
        app.world().resource::<crate::sim_tick::SimTick>().0,
        source_tick + 1,
        "the result must not inherit PostUpdate's continuation tick"
    );
}

#[test]
fn managed_gm_only_automatic_grant_applies_without_local_crew() {
    let mut app = test_app();
    enable_managed_lobby(&mut app, true);
    app.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: true,
            ready: true,
        }])
        .unwrap(),
    );
    assert!(app
        .world_mut()
        .resource_mut::<PendingStartGrants>()
        .try_push(automatic_grant(1)));

    let out = tick(&mut app);
    assert!(out
        .iter()
        .any(|message| matches!(message.msg, ServerMessage::GameStarted)));
    assert_eq!(
        take_start_results(&mut app)[0].status,
        crate::lobby::start_policy::StartGrantStatus::Applied
    );
}

#[cfg(feature = "server")]
#[test]
fn managed_grant_enters_the_same_phase_for_different_local_preload_states() {
    fn peer_with_preload(complete: bool) -> App {
        let mut app = test_app();
        enable_managed_lobby(&mut app, true);
        let mut preload = crate::server::asset_preload::AssetPreloadResource::default();
        preload.started = true;
        preload.complete = complete;
        app.insert_resource(preload);
        assert!(app
            .world_mut()
            .resource_mut::<PendingStartGrants>()
            .try_push(automatic_grant(1)));
        app
    }

    let mut still_loading_assets = peer_with_preload(false);
    let mut completed_assets = peer_with_preload(true);

    for app in [&mut still_loading_assets, &mut completed_assets] {
        tick(app);
        assert_eq!(
            take_start_results(app)[0].status,
            crate::lobby::start_policy::StartGrantStatus::Applied
        );
        // Apply the NextState scheduled on the fixed tick above.
        tick(app);
        assert_eq!(
            app.world().resource::<State<GamePhase>>().get(),
            &GamePhase::InProgress
        );
    }
}

#[test]
fn managed_force_grant_is_immutable_across_a_late_gm_disconnect() {
    let mut app = test_app();
    enable_managed_lobby(&mut app, true);
    app.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: true,
            ready: false,
        }])
        .unwrap(),
    );
    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .register("crew-1".into(), "Alice".into())
        .unwrap();
    assert!(app
        .world_mut()
        .resource_mut::<PendingStartGrants>()
        .try_push(forced_grant(1, "gm-1")));

    // The owner authenticated and attributed the force before emitting the
    // grant. A disconnect observed by only this peer after that decision
    // must not make it diverge from peers that already applied the grant.
    app.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: false,
            ready: false,
        }])
        .unwrap(),
    );
    tick(&mut app);
    assert_eq!(
        take_start_results(&mut app)[0].status,
        crate::lobby::start_policy::StartGrantStatus::Applied
    );
}

#[test]
fn managed_validation_refuses_auto_and_force() {
    let mut app = test_app();
    enable_managed_lobby(&mut app, false);
    app.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: true,
            ready: true,
        }])
        .unwrap(),
    );
    {
        let mut pending = app.world_mut().resource_mut::<PendingStartGrants>();
        assert!(pending.try_push(automatic_grant(1)));
        assert!(pending.try_push(forced_grant(2, "gm-1")));
    }
    tick(&mut app);
    let results = take_start_results(&mut app);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| {
        result.status == crate::lobby::start_policy::StartGrantStatus::Refused
            && result.reason == Some(crate::lobby::start_policy::StartGrantReason::ValidationFailed)
    }));
}

#[test]
fn canonical_grant_does_not_reread_validation_at_apply_tick() {
    let mut app = test_app();
    enable_managed_lobby(&mut app, true);
    install_two_participant_fleet(&mut app, 1, 1, 2);
    assert!(app
        .world_mut()
        .resource_mut::<PendingStartGrants>()
        .try_push(automatic_grant(1)));

    tick(&mut app);
    assert!(app
        .world()
        .resource::<StartGrantTracker>()
        .canonical
        .is_some());
    app.world_mut()
        .resource_mut::<FleetManagedLobby>()
        .validation_passed = false;

    for _ in 0..3 {
        tick(&mut app);
    }
    let results = take_start_results(&mut app);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].status,
        crate::lobby::start_policy::StartGrantStatus::Applied,
        "validation was frozen when the owner sealed the canonical grant"
    );
}

#[test]
fn member_with_failed_validation_refuses_owner_grant_before_barrier_opens() {
    use crate::command_admission::HostSlot;

    let mut app = test_app();
    enable_managed_lobby(&mut app, false);
    install_two_participant_fleet(&mut app, 2, 1, 2);
    let mut grant = automatic_grant(1);
    grant.apply_tick = 3;
    app.world_mut()
        .resource_mut::<crate::lockstep::MeshInbox>()
        .push_from(
            crate::lockstep::MeshFrame::Tick(crate::lockstep::TickFrame {
                from: HostSlot(1),
                tick: 0,
                ready_through: 2,
                commands: Vec::new(),
                start_grant: Some(grant),
            }),
            crate::lockstep::MeshOrigin::Peer(HostSlot(1)),
        );

    tick(&mut app);
    assert!(app
        .world()
        .resource::<StartGrantTracker>()
        .is_failed_closed());
    assert!(app.world().resource::<PendingStartGrants>().0.is_empty());
    let results = take_start_results(&mut app);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].reason,
        Some(crate::lobby::start_policy::StartGrantReason::ValidationFailed)
    );
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby
    );
}

#[test]
fn duplicate_grant_applies_once_and_reports_a_no_op() {
    let mut app = test_app();
    enable_managed_lobby(&mut app, true);
    {
        let mut pending = app.world_mut().resource_mut::<PendingStartGrants>();
        assert!(pending.try_push(automatic_grant(1)));
        assert!(pending.try_push(automatic_grant(1)));
    }
    let out = tick(&mut app);
    assert_eq!(
        out.iter()
            .filter(|message| matches!(message.msg, ServerMessage::GameStarted))
            .count(),
        1
    );
    let results = take_start_results(&mut app);
    assert_eq!(
        results
            .iter()
            .map(|result| result.status)
            .collect::<Vec<_>>(),
        vec![
            crate::lobby::start_policy::StartGrantStatus::Applied,
            crate::lobby::start_policy::StartGrantStatus::NoOp
        ]
    );
}

#[test]
fn local_owner_cannot_propose_a_preselected_nonzero_apply_tick() {
    let mut app = test_app();
    enable_managed_lobby(&mut app, true);
    install_two_participant_fleet(&mut app, 1, 1, 2);

    let mut forged = automatic_grant(1);
    forged.apply_tick = 99;
    assert!(app
        .world_mut()
        .resource_mut::<PendingStartGrants>()
        .try_push(forged));

    tick(&mut app);
    let results = take_start_results(&mut app);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].status,
        crate::lobby::start_policy::StartGrantStatus::Refused
    );
    assert_eq!(
        results[0].reason,
        Some(crate::lobby::start_policy::StartGrantReason::UnsafeApplyTick)
    );
    assert!(app
        .world()
        .resource::<crate::lockstep::MeshOutbox>()
        .pending_frames()
        .iter()
        .all(|frame| !matches!(
            frame,
            crate::lockstep::MeshFrame::Tick(tick) if tick.start_grant.is_some()
        )));
}

#[test]
fn technical_gm_departure_does_not_strand_an_embedded_start_boundary() {
    use crate::command_admission::HostSlot;

    let mut owner = test_app();
    enable_managed_lobby(&mut owner, true);
    install_gm_owner_fleet(&mut owner, 1, 2);
    assert!(owner
        .world_mut()
        .resource_mut::<PendingStartGrants>()
        .try_push(automatic_grant(1)));
    tick(&mut owner);
    let bearing = owner
        .world_mut()
        .resource_mut::<crate::lockstep::MeshOutbox>()
        .drain()
        .into_iter()
        .find(|frame| {
            matches!(
                frame,
                crate::lockstep::MeshFrame::Tick(tick) if tick.start_grant.is_some()
            )
        })
        .expect("the technical owner embeds its assigned boundary in a TickFrame");
    let apply_tick = match &bearing {
        crate::lockstep::MeshFrame::Tick(tick) => {
            assert_eq!(tick.from, HostSlot(1));
            tick.start_grant.as_ref().unwrap().apply_tick
        }
        _ => unreachable!(),
    };
    assert_eq!(apply_tick, 3);

    let mut survivors = [test_app(), test_app()];
    for (app, local) in survivors.iter_mut().zip([2, 3]) {
        enable_managed_lobby(app, true);
        install_gm_owner_fleet(app, local, 2);
        let mut inbox = app.world_mut().resource_mut::<crate::lockstep::MeshInbox>();
        // Stronger than the reliable transport's ordinary ordering: even if
        // the socket-close observation reaches Rust before the final owner
        // frame, the roster still authenticates the frozen owner and the
        // departed wait-set cannot strand its immutable decision.
        inbox.push_from(
            crate::lockstep::MeshFrame::HostLoss(crate::lockstep::HostLossFrame {
                from: HostSlot(1),
                lost: HostSlot(1),
                tick: 0,
            }),
            crate::lockstep::MeshOrigin::LocalObservation,
        );
        inbox.push_from(
            bearing.clone(),
            crate::lockstep::MeshOrigin::Peer(HostSlot(1)),
        );
    }

    for app in &mut survivors {
        tick(app);
        let fleet = app.world().resource::<crate::lockstep::FleetLockstep>();
        assert!(fleet.has_departed(HostSlot(1)));
        assert_eq!(fleet.watermark_of(HostSlot(1)), None);
        assert!(fleet.peers().all(|peer| peer != HostSlot(1)));
        assert_eq!(
            app.world()
                .resource::<StartGrantTracker>()
                .canonical
                .as_ref()
                .unwrap()
                .apply_tick,
            apply_tick
        );
        assert_eq!(
            app.world().resource::<State<GamePhase>>().get(),
            &GamePhase::Lobby
        );
        for _ in 0..3 {
            tick(app);
        }
        let results = take_start_results(app);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].status,
            crate::lobby::start_policy::StartGrantStatus::Applied
        );
        tick(app);
        assert_eq!(
            app.world().resource::<State<GamePhase>>().get(),
            &GamePhase::InProgress
        );
    }
    assert_eq!(
        survivors[0]
            .world()
            .resource::<crate::sim_tick::SimTick>()
            .0,
        survivors[1]
            .world()
            .resource::<crate::sim_tick::SimTick>()
            .0
    );
}

#[test]
fn departed_owner_start_boundary_never_revives_commands_or_survives_owner_transfer() {
    use crate::command_admission::{CommandOrder, HostSlot, ShipKey};
    use crate::core::messages::{SystemControlPayload, SystemId};
    use crate::lockstep::{FleetLockstep, FleetRoster, MeshFrame, MeshInbox, MeshOrigin};

    // The live case proves this same command passes ownership/admission;
    // departure alone must fence it while preserving the start decision.
    for (departed, transferred) in [(false, false), (true, false), (true, true)] {
        let mut app = test_app();
        enable_managed_lobby(&mut app, true);
        install_two_participant_fleet(&mut app, 2, 1, 2);
        let ship = "00000000-0000-8000-8000-000000000001";
        app.world_mut().spawn((
            crate::lockstep::FleetSlotOf(HostSlot(1)),
            crate::entities::spawner::EntityUuid(ship.into()),
        ));
        if departed {
            app.world_mut()
                .resource_mut::<FleetLockstep>()
                .depart(HostSlot(1));
        }
        if transferred {
            assert!(app
                .world_mut()
                .resource_mut::<FleetRoster>()
                .transfer_owner(HostSlot(1), HostSlot(2)));
        }
        let mut grant = automatic_grant(1);
        grant.apply_tick = 3;
        app.world_mut().resource_mut::<MeshInbox>().push_from(
            MeshFrame::Tick(crate::lockstep::TickFrame {
                from: HostSlot(1),
                tick: 0,
                ready_through: 2,
                commands: vec![crate::lockstep::MeshCommand {
                    tick: 10,
                    order: CommandOrder::new(HostSlot(1), 1),
                    ship: ShipKey(ship.into()),
                    target: SystemId("helm_thrust".into()),
                    payload: SystemControlPayload::SetThrust { value: 0.5 },
                }],
                start_grant: Some(grant.clone()),
            }),
            MeshOrigin::Peer(HostSlot(1)),
        );
        tick(&mut app);
        let tracker = app.world().resource::<StartGrantTracker>();
        assert_eq!(tracker.canonical.as_ref(), (!transferred).then_some(&grant));
        assert!(
            !tracker.is_failed_closed(),
            "a former owner cannot poison the successor"
        );
        assert_eq!(
            app.world()
                .resource::<crate::command_admission::log::PendingCommands>()
                .len(),
            usize::from(!departed)
        );
        if departed {
            let fleet = app.world().resource::<FleetLockstep>();
            assert!(fleet.has_departed(HostSlot(1)));
            assert_eq!(fleet.watermark_of(HostSlot(1)), None);
        }
    }
}

#[test]
fn managed_lobby_never_arms_the_legacy_local_countdown() {
    let mut app = test_app();
    enable_managed_lobby(&mut app, true);
    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .register("t1".into(), "Alice".into())
        .unwrap();
    push(&mut app, "t1", ClientMessage::SetReady { ready: true });

    let out = tick(&mut app);
    assert_eq!(app.world().resource::<CountdownTimer>().remaining_secs, 0.0);
    assert!(!out.iter().any(|message| matches!(
        message.msg,
        ServerMessage::GameStartCountdown { remaining_secs } if remaining_secs > 0
    )));
}

#[test]
fn same_frame_teardown_and_reopen_resets_start_id_generation() {
    let mut inputs = VecDeque::from([
        FleetLobbyInput::Managed(false),
        FleetLobbyInput::Managed(true),
        FleetLobbyInput::Validation(true),
        FleetLobbyInput::Grant(automatic_grant(1)),
    ]);
    let mut managed = FleetManagedLobby {
        enabled: true,
        validation_passed: true,
    };
    let mut grants = PendingStartGrants::default();
    let mut tracker = StartGrantTracker {
        last_sequence: 1,
        ..Default::default()
    };
    let mut results = StartGrantResults::default();

    assert!(apply_fleet_lobby_inputs(
        &mut inputs,
        &mut managed,
        &mut grants,
        &mut tracker,
        &mut results,
    ));
    assert!(inputs.is_empty());
    assert!(managed.enabled);
    assert!(managed.validation_passed);
    assert_eq!(tracker.last_sequence, 0);
    assert_eq!(grants.pop_front().unwrap().id, "start-1");
}
