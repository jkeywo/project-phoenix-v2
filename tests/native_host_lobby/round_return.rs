//! A retained native world returns its existing crew to a usable lobby.
use super::*;
use project_phoenix::core::codec::{JsonCodec, MessageCodec};
use project_phoenix::core::messages::DeliveryClass;
use project_phoenix::lobby::{stations_config::ShipStations, SelectedShipResource, Sessions};
use project_phoenix::native_host::panes::{identity::PaneIdentity, PaneBus};
use project_phoenix::native_host::transport::PairedTransport;

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

fn assert_game_over_pane_reconnect(deferred: bool) {
    let (mut app, handle, seats) = retained_host(deferred);
    let bus = PaneBus::default();
    let pane = bus.open(PaneIdentity::adopt(CREW[1], "Moved crew").unwrap());
    bus.mark_live(pane);
    app.insert_resource(NativeTransportLink::new(PairedTransport::new(
        handle.transport(),
        bus.transport(),
    )));
    bus.submit(
        pane,
        ClientMessage::Identify {
            token: CREW[1].into(),
            name: "Moved crew".into(),
        },
    )
    .unwrap();
    pump(&mut app, 4);
    bus.take_outbound(pane);
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::GameOver);
    pump(&mut app, 2);
    bus.take_outbound(pane);

    // The real move/resize path closes the old document and recreates it on
    // the same identity. Its replacement page identifies while the ending is
    // still visible, not after ReturnToLobby releases the station controls.
    bus.close(pane);
    let (recreated, _) = bus.recreate(pane).expect("closed pane recreates");
    bus.mark_live(recreated);
    assert_ne!(recreated, pane);
    assert_eq!(bus.token_of(recreated).as_deref(), Some(CREW[1]));
    pump(&mut app, 4);
    assert!(
        !app.world()
            .resource::<Sessions>()
            .0
            .players()
            .iter()
            .find(|p| p.token == CREW[1])
            .unwrap()
            .connected
    );
    bus.submit(
        recreated,
        ClientMessage::Identify {
            token: CREW[1].into(),
            name: "Moved crew".into(),
        },
    )
    .unwrap();
    // Several real frame/fixed updates expose a phase-gated reader that loses
    // this one-time Identify, rather than keeping it until the return frame.
    pump(&mut app, 8);
    let terminal_messages: Vec<_> = bus
        .take_outbound(recreated)
        .into_iter()
        .map(|pending| JsonCodec.decode_server(&pending.json).unwrap())
        .collect();

    // Identity can reconnect at the ending; station actions remain gated.
    bus.submit(recreated, ClientMessage::ReleaseStation)
        .unwrap();
    bus.submit(recreated, ClientMessage::SetReady { ready: true })
        .unwrap();
    pump(&mut app, 4);
    let player = app
        .world()
        .resource::<Sessions>()
        .0
        .players()
        .iter()
        .find(|p| p.token == CREW[1])
        .unwrap();
    assert_eq!(player.station.as_ref().map(|id| &id.0), Some(&seats[1]));
    assert!(!player.ready);
    bus.take_outbound(recreated);

    handle.send(CREW[0], ClientMessage::ReturnToLobby);
    pump(&mut app, 4);
    let returned_messages: Vec<_> = bus
        .take_outbound(recreated)
        .into_iter()
        .map(|pending| JsonCodec.decode_server(&pending.json).unwrap())
        .collect();
    assert!(returned_messages
        .iter()
        .any(|msg| matches!(msg, ServerMessage::ReturnedToLobby)));
    bus.submit(
        recreated,
        ClientMessage::SelectStation {
            station: seats[1].clone(),
        },
    )
    .unwrap();
    pump(&mut app, 4);
    let claimed: Vec<_> = bus
        .take_outbound(recreated)
        .into_iter()
        .map(|pending| JsonCodec.decode_server(&pending.json).unwrap())
        .collect();
    assert!(
        claimed.iter().any(|msg| matches!(msg,
            ServerMessage::StationAssigned { token, station_id: Some(id), .. }
            if token == CREW[1] && id.0 == seats[1]
        )),
        "the actual pane receives the positive claim assignment"
    );
    let player = app
        .world()
        .resource::<Sessions>()
        .0
        .players()
        .iter()
        .find(|p| p.token == CREW[1])
        .unwrap();
    assert!(player.connected,
        "a positive StationAssigned is insufficient: disconnected holders remain CLAIM in the client roster");
    assert_eq!(player.station.as_ref().map(|id| &id.0), Some(&seats[1]));
    assert_eq!(
        app.world().resource::<Sessions>().0.holder_for_station(
            &project_phoenix::core::messages::StationId(seats[1].clone())
        ),
        Some(CREW[1])
    );
    for (messages, phase, station) in [
        (&terminal_messages, GamePhase::GameOver, Some(&seats[1])),
        (&returned_messages, GamePhase::Lobby, None),
    ] {
        let state = messages
            .iter()
            .find_map(|msg| match msg {
                ServerMessage::Welcome { state, .. } if state.phase == phase => Some(state),
                _ => None,
            })
            .expect("the recreated pane receives its current lifecycle Welcome");
        let player = state.players.iter().find(|p| p.token == CREW[1]).unwrap();
        assert!(
            player.connected,
            "the actual client projection must include a connected holder"
        );
        assert_eq!(player.station.as_ref().map(|id| &id.0), station);
        assert!(!player.ready);
    }
    bus.submit(recreated, ClientMessage::SetReady { ready: true })
        .unwrap();
    pump(&mut app, 4);
    assert!(bus.take_outbound(recreated).iter().any(|pending| matches!(
        JsonCodec.decode_server(&pending.json).unwrap(),
        ServerMessage::ReadyChanged { token, ready: true } if token == CREW[1]
    )));
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby
    );
    assert_eq!(
        bus.open_pane_for_name("Moved crew"),
        Some(recreated),
        "return, claim and Ready require no second page recreation or Identify"
    );
}

#[test]
fn retained_world_pane_recreated_during_game_over_can_reclaim_after_return() {
    assert_game_over_pane_reconnect(false);
}

#[test]
fn selected_lobby_pane_recreated_during_game_over_can_reclaim_after_return() {
    assert_game_over_pane_reconnect(true);
}
