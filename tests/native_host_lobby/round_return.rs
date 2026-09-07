//! A retained native world returns its existing crew to a usable lobby.
use super::*;
use project_phoenix::core::messages::DeliveryClass;
use project_phoenix::lobby::{stations_config::ShipStations, SelectedShipResource, Sessions};

const CREW: [&str; 2] = [
    "3f1a6c2e-0a11-4b3c-9d55-000000000071",
    "3f1a6c2e-0a11-4b3c-9d55-000000000072",
];

fn retained_host(deferred: bool) -> (App, LoopbackHandle, Vec<String>) {
    let (_, hull) = pick();
    let mut cfg = if deferred {
        lobby_config()
    } else {
        NativeHostConfig::new("assets/worlds/combat_test.toml")
    };
    cfg.ship_path = Some(hull);
    cfg.surface = NativeRenderSurface::Contract;
    cfg.seed = Some(SEED);
    let mut app = build_native_host_app(&cfg, &preload()).expect("native host assembles");
    let handle = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(handle.transport()));
    pump(&mut app, 4);
    assert!(
        handle
            .drain_outbound()
            .iter()
            .all(|(_, message, _)| { !matches!(message, ServerMessage::Welcome { .. }) }),
        "initial Lobby entry is not a round return"
    );
    for (index, token) in CREW.iter().enumerate() {
        handle.send(
            *token,
            ClientMessage::Identify {
                token: (*token).into(),
                name: format!("Crew {index}"),
            },
        );
    }
    pump(&mut app, 4);
    if deferred {
        handle.send(
            CREW[0],
            ClientMessage::SelectScenario {
                scenario_id: SCENARIO.into(),
            },
        );
        pump(&mut app, 8);
    }
    assert!(app.world().contains_resource::<WorldConfig>());
    let seats: Vec<_> = app
        .world()
        .resource::<ShipStations>()
        .stations
        .iter()
        .filter(|station| !station.auxiliary)
        .take(2)
        .map(|station| station.id.0.clone())
        .collect();
    assert_eq!(seats.len(), 2);
    for (token, station) in CREW.iter().zip(&seats) {
        handle.send(
            *token,
            ClientMessage::SelectStation {
                station: station.clone(),
            },
        );
    }
    pump(&mut app, 4);
    for token in CREW {
        handle.send(token, ClientMessage::SetReady { ready: true });
    }
    pump(&mut app, 4);
    assert!(app
        .world()
        .resource::<Sessions>()
        .0
        .players()
        .iter()
        .all(|p| p.ready));
    // Contract has no GPU preloader. Enter the real phase without replacing
    // any registered lifecycle/transport system or simulating a reconnect.
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    pump(&mut app, 2);
    handle.drain_outbound();
    (app, handle, seats)
}

fn assert_retained_return(deferred: bool, abort_in_play: bool) {
    let (mut app, handle, seats) = retained_host(deferred);
    if abort_in_play {
        handle.send(CREW[0], ClientMessage::ReturnToLobby);
        pump(&mut app, 2);
        assert_eq!(
            app.world().resource::<State<GamePhase>>().get(),
            &GamePhase::InProgress
        );
        assert!(
            handle
                .drain_outbound()
                .iter()
                .all(|(_, message, _)| !matches!(
                    message,
                    ServerMessage::ReturnedToLobby | ServerMessage::Welcome { .. }
                )),
            "a refused participant abort publishes no lobby completion"
        );
    }
    if !abort_in_play {
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::GameOver);
        pump(&mut app, 2);
        handle.drain_outbound();
    }
    let identities = world_name_to_uuid(&app);
    let selected_ship = app.world().resource::<SelectedShipResource>().0.clone();
    let stations = app.world().resource::<ShipStations>().clone();
    let client_config = app
        .world()
        .resource::<project_phoenix::lobby::server::ShipClientConfigResource>()
        .0
        .clone();
    if abort_in_play {
        // The host-lobby bridge writes directly to the inbound bus. Its
        // reserved identity must never be accepted through a crew transport.
        app.world_mut().write_message(InboundMessage {
            token: project_phoenix::console_bridge::LOCAL_CONSOLE_TOKEN.into(),
            msg: ClientMessage::ReturnToLobby,
        });
    } else {
        handle.send(CREW[0], ClientMessage::ReturnToLobby);
    }
    pump(&mut app, 4);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby
    );
    let delivered = handle.drain_outbound();
    let returned = delivered
        .iter()
        .position(|(_, message, _)| matches!(message, ServerMessage::ReturnedToLobby))
        .expect("accepted return reaches the crew");
    let welcomes: Vec<_> = delivered
        .iter()
        .enumerate()
        .filter(|(_, (_, message, _))| matches!(message, ServerMessage::Welcome { .. }))
        .collect();
    assert_eq!(
        welcomes.len(),
        1,
        "one completion projection per accepted phase edge"
    );
    let (welcome_index, (target, welcome, delivery)) = welcomes[0];
    assert!(
        welcome_index > returned,
        "selection completion follows ReturnedToLobby"
    );
    assert_eq!(
        *target,
        Target::All,
        "phones and panes receive the same completion"
    );
    assert_eq!(*delivery, DeliveryClass::Reliable);
    assert!(delivered[..=welcome_index]
        .iter()
        .all(|(_, _, delivery)| *delivery == DeliveryClass::Reliable));
    let ServerMessage::Welcome {
        state,
        ship_stations,
        ship_config,
        ..
    } = welcome
    else {
        unreachable!()
    };
    assert_eq!(state.phase, GamePhase::Lobby);
    assert_eq!(*ship_stations, stations);
    assert_eq!(*ship_config, client_config);
    for token in CREW {
        let player = state
            .players
            .iter()
            .find(|p| p.token == token)
            .expect("same participant identity");
        assert!(player.connected);
        assert!(player.station.is_none());
        assert!(!player.ready);
        assert!(delivered[..returned].iter().any(|(_, message, _)| matches!(message, ServerMessage::ReadyChanged { token: changed, ready: false } if changed == token)));
        assert!(delivered[..returned].iter().any(|(_, message, _)| matches!(message, ServerMessage::StationAssigned { token: changed, station: None, .. } if changed == token)));
    }
    assert_eq!(
        world_name_to_uuid(&app),
        identities,
        "return does not rematerialize the world"
    );
    assert_eq!(
        app.world().resource::<SelectedShipResource>().0,
        selected_ship
    );
    assert!(delivered[welcome_index + 1..]
        .iter()
        .any(|(_, message, _)| matches!(message, ServerMessage::ShipManual { .. })));
    pump(&mut app, 4);
    assert!(
        handle
            .drain_outbound()
            .iter()
            .all(|(_, message, _)| !matches!(message, ServerMessage::Welcome { .. })),
        "idle Lobby does not repeat Welcome"
    );

    // Existing identities reclaim directly from the now-usable lobby; neither
    // an Identify nor a scenario selection is sent after ReturnToLobby.
    handle.send(
        CREW[0],
        ClientMessage::SelectStation {
            station: seats[1].clone(),
        },
    );
    handle.send(
        CREW[1],
        ClientMessage::SelectStation {
            station: seats[0].clone(),
        },
    );
    pump(&mut app, 4);
    handle.send(CREW[0], ClientMessage::SetReady { ready: true });
    pump(&mut app, 4);
    let sessions = &app.world().resource::<Sessions>().0;
    assert_eq!(sessions.players().len(), 2);
    for (token, station) in [(CREW[0], &seats[1]), (CREW[1], &seats[0])] {
        let player = sessions
            .players()
            .iter()
            .find(|p| p.token == token)
            .unwrap();
        assert_eq!(player.station.as_ref().map(|s| &s.0), Some(station));
        assert_eq!(player.ready, token == CREW[0]);
    }
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby,
        "claim and Ready are accepted before launch"
    );
    let claimed = handle.drain_outbound();
    assert!(claimed.iter().any(|(_, message, _)| matches!(message, ServerMessage::ReadyChanged { token, ready: true } if token == CREW[0])));
}

#[test]
fn retained_world_return_rewelcomes_existing_crew_before_they_reclaim_and_ready() {
    assert_retained_return(false, false);
}

#[test]
fn selected_lobby_return_rewelcomes_existing_crew_before_they_reclaim_and_ready() {
    assert_retained_return(true, false);
}

#[test]
fn retained_world_host_abort_also_rewelcomes_existing_crew() {
    assert_retained_return(false, true);
}
