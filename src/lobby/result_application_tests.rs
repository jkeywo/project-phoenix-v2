//! Registered LobbyPlugin flows, with and without the LocalShip projection.
use super::{
    tests::{push, test_app, tick},
    *,
};
use crate::core::messages::{StationId, SystemId};
use crate::ship::{config::ShipConfig, control_source::ControlSource};

fn fixture(loaded: bool) -> (App, Option<Entity>) {
    let config: ShipConfig = toml::from_str(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = "Flight"
rank = "Crew"
[[station.rating]]
name = "Std"
automated_systems = []
[[station.rating]]
name = "Assist"
automated_systems = ["drive"]
[[system]]
id = "drive"
kind = "helm_thrust"
station = "helm"
"#,
    )
    .unwrap();
    let mut app = test_app();
    app.insert_resource(PendingShipConfig(config.clone()));
    let ship = loaded.then(|| {
        let (sources, ratings) = rating::seed_boot_ratings(&config, |_| "Backfill".into());
        app.world_mut()
            .spawn((
                crate::server_app::LocalShip,
                ShipConfigComponent(config),
                ShipSystemControlSources(sources),
                ActiveStationRatings(ratings),
            ))
            .id()
    });
    tick(&mut app);
    (app, ship)
}

fn identify(app: &mut App, token: &str) -> Vec<OutboundMessage> {
    push(
        app,
        token,
        ClientMessage::Identify {
            token: token.into(),
            name: token.into(),
        },
    );
    tick(app)
}

fn command(app: &mut App, token: &str, message: ClientMessage) -> Vec<OutboundMessage> {
    push(app, token, message);
    tick(app)
}

fn assert_projection(app: &App, ship: Option<Entity>, rating: &str, source: ControlSource) {
    if let Some(ship) = ship {
        let ship = app.world().entity(ship);
        assert_eq!(
            ship.get::<ActiveStationRatings>().unwrap().0[&StationId("helm".into())],
            rating
        );
        assert_eq!(
            ship.get::<ShipSystemControlSources>()
                .unwrap()
                .0
                .source_for(&SystemId("drive".into())),
            source
        );
    }
}

#[test]
fn complete_result_preserves_pending_welcome_and_countdown_without_a_ship() {
    for loaded in [false, true] {
        let (mut app, _) = fixture(loaded);
        identify(&mut app, "crew");
        command(
            &mut app,
            "crew",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        command(
            &mut app,
            "crew",
            ClientMessage::SetStationRating {
                rating_name: "Assist".into(),
            },
        );
        let welcome = identify(&mut app, "reader");
        let ratings = welcome
            .iter()
            .filter_map(|m| match &m.msg {
                ServerMessage::Welcome {
                    station_ratings, ..
                } => Some(station_ratings),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(ratings.len(), 1, "one evaluation sends one Welcome");
        assert_eq!(
            ratings[0][&StationId("helm".into())],
            if loaded { "Backfill" } else { "Assist" }
        );
        assert_eq!(
            welcome
                .iter()
                .filter(|m| matches!(m.msg, ServerMessage::ShipManual { .. }))
                .count(),
            1
        );
        // Spectators do not hold up readiness; the ordinary handler decides.
        command(
            &mut app,
            "reader",
            ClientMessage::SetSpectator { spectator: true },
        );
        let ready = command(&mut app, "crew", ClientMessage::SetReady { ready: true });
        assert!(ready.iter().any(|m| matches!(
            m.msg,
            ServerMessage::GameStartCountdown { remaining_secs: 5 }
        )));
        assert!(app.world().resource::<CountdownTimer>().remaining_secs > 0.0);
        let unready = command(&mut app, "crew", ClientMessage::SetReady { ready: false });
        assert_eq!(app.world().resource::<CountdownTimer>().remaining_secs, 0.0);
        let cancelled = unready
            .iter()
            .position(|m| {
                matches!(
                    m.msg,
                    ServerMessage::GameStartCountdown { remaining_secs: 0 }
                )
            })
            .unwrap();
        let changed = unready
            .iter()
            .position(|m| matches!(m.msg, ServerMessage::ReadyChanged { ready: false, .. }))
            .unwrap();
        assert!(
            cancelled < changed,
            "countdown cancellation precedes the handler's outbound messages"
        );
    }
}

#[test]
fn complete_result_claim_afk_release_and_spectator_update_all_loaded_projections() {
    for loaded in [false, true] {
        let (mut app, ship) = fixture(loaded);
        identify(&mut app, "crew");
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        tick(&mut app);
        command(
            &mut app,
            "crew",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        assert_projection(&app, ship, "Backfill", ControlSource::Ai);
        command(&mut app, "crew", ClientMessage::SetReady { ready: true });
        assert_projection(&app, ship, "Std", ControlSource::Human);
        command(&mut app, "crew", ClientMessage::SetAfk { afk: true });
        assert_projection(&app, ship, "Backfill", ControlSource::Ai);
        let resumed = command(&mut app, "crew", ClientMessage::SetAfk { afk: false });
        let expected = if loaded { "Std" } else { "Backfill" };
        assert!(resumed.iter().any(|m| matches!(&m.msg, ServerMessage::RatingChanged { rating_name, .. } if rating_name == expected)));
        assert_projection(&app, ship, "Std", ControlSource::Human);
        for message in [
            ClientMessage::ReleaseStation,
            ClientMessage::SetSpectator { spectator: true },
        ] {
            command(&mut app, "crew", message);
            assert!(app
                .world()
                .resource::<Sessions>()
                .0
                .station_for_token("crew")
                .is_none());
            assert_projection(&app, ship, "Backfill", ControlSource::Ai);
            command(
                &mut app,
                "crew",
                ClientMessage::SetSpectator { spectator: false },
            );
            command(
                &mut app,
                "crew",
                ClientMessage::SelectStation {
                    station: "Helm".into(),
                },
            );
            command(&mut app, "crew", ClientMessage::SetReady { ready: true });
        }
        command(
            &mut app,
            crate::console_bridge::LOCAL_CONSOLE_TOKEN,
            ClientMessage::ReturnToLobby,
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<State<GamePhase>>().get(),
            &GamePhase::Lobby
        );
    }
}

#[test]
fn complete_result_disconnect_precedes_same_tick_identify_and_reconnect_yields_to_a_new_holder() {
    for loaded in [false, true] {
        let (mut app, ship) = fixture(loaded);
        identify(&mut app, "crew");
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        tick(&mut app);
        command(
            &mut app,
            "crew",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        command(&mut app, "crew", ClientMessage::SetReady { ready: true });
        app.world_mut()
            .resource_mut::<Messages<PlayerDisconnected>>()
            .write(PlayerDisconnected {
                token: "crew".into(),
            });
        let rejoined = identify(&mut app, "crew");
        let left = rejoined
            .iter()
            .position(|m| matches!(m.msg, ServerMessage::PlayerLeft { .. }))
            .unwrap();
        let welcome = rejoined
            .iter()
            .position(|m| matches!(m.msg, ServerMessage::Welcome { .. }))
            .unwrap();
        assert!(left < welcome);
        assert_projection(&app, ship, "Std", ControlSource::Human);
        assert!(
            app.world()
                .resource::<Sessions>()
                .0
                .players()
                .iter()
                .find(|p| p.token == "crew")
                .unwrap()
                .connected
        );
        app.world_mut()
            .resource_mut::<Messages<PlayerDisconnected>>()
            .write(PlayerDisconnected {
                token: "crew".into(),
            });
        tick(&mut app);
        identify(&mut app, "replacement");
        command(
            &mut app,
            "replacement",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        command(
            &mut app,
            "replacement",
            ClientMessage::SetReady { ready: true },
        );
        identify(&mut app, "crew");
        let sessions = &app.world().resource::<Sessions>().0;
        assert!(sessions.station_for_token("crew").is_none());
        assert_eq!(
            sessions.station_for_token("replacement"),
            Some(&StationId("helm".into()))
        );
        assert_projection(&app, ship, "Std", ControlSource::Human);
    }
}
