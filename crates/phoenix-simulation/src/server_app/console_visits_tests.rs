use super::*;
use crate::core::messages::ObjectiveStatus;
use phoenix_model::console::ConsoleHostSnapshot;

fn fixture() -> (Sessions, crate::ship::config::ShipConfig) {
    let config = toml::from_str(
        r#"
[[station]]
id="home"
name="Home"
description=""
rank=""
[[station.rating]]
name="Std"
automated_systems=[]
[[station]]
id="visitor"
name="Visitor"
description=""
rank=""
[[station.rating]]
name="Std"
automated_systems=[]
[[console]]
id="ordinary"
screen="gui/captain-console.html"
home_station="home"
[[console]]
id="detached"
screen="gui/navigation-console.html"
human_seeking=true
host_order=["home","visitor"]
visiting_rating="Std"
[[console.rating]]
name="Std"
automated_systems=[]
"#,
    )
    .unwrap();
    let mut sessions = Sessions(crate::lobby::session::SessionManager::new());
    for (token, seat) in [("first", "home"), ("second", "visitor")] {
        sessions.0.register(token.into(), token.into()).unwrap();
        sessions.0.set_station(token, Some(StationId(seat.into())));
    }
    (sessions, config)
}

#[test]
fn detached_console_visits_follow_live_host_authority() {
    let (mut sessions, config) = fixture();
    let id = ConsoleId("detached".into());
    let mut hosts = hosts();
    assert!(may_visit(&sessions, "first", &config, &id, &hosts));
    assert!(!may_visit(&sessions, "second", &config, &id, &hosts));
    sessions.0.set_afk("first", true);
    assert!(!may_visit(&sessions, "first", &config, &id, &hosts));
    // Frozen/current host remains home despite a live AFK change.
    assert!(!may_visit(&sessions, "second", &config, &id, &hosts));
    hosts.0[1].host = Some(StationId("visitor".into()));
    assert!(may_visit(&sessions, "second", &config, &id, &hosts));
    sessions.0.set_afk("first", false);
    assert!(may_visit(&sessions, "second", &config, &id, &hosts));
    assert!(!may_visit(&sessions, "first", &config, &id, &hosts));
    hosts.0[1].host = Some(StationId("home".into()));
    assert!(!may_visit(&sessions, "second", &config, &id, &hosts));
    assert!(!may_visit(&sessions, "unknown", &config, &id, &hosts));
    assert!(!may_visit(
        &sessions,
        "first",
        &config,
        &ConsoleId("unknown".into()),
        &hosts
    ));
}

#[test]
fn visiting_detached_console_clears_only_its_unread_and_preserves_critical() {
    let (sessions, config) = fixture();
    let detached = AttentionOwner::Console(ConsoleId("detached".into()));
    let ordinary = AttentionOwner::Console(ConsoleId("ordinary".into()));
    let mut app = App::new();
    app.add_message::<InboundMessage>();
    app.insert_resource(sessions);
    app.init_resource::<StationImportanceRes>();
    app.world_mut().spawn((
        crate::server_app::LocalShip,
        ShipConfigComponent(config),
        hosts(),
    ));
    app.add_systems(Update, drain_station_visited);
    app.world_mut()
        .resource_mut::<StationImportanceRes>()
        .0
        .ingest(
            vec![
                ("one".into(), detached.clone(), ObjectiveStatus::Completed),
                ("two".into(), ordinary.clone(), ObjectiveStatus::Completed),
            ],
            vec![detached.clone()],
        );
    app.world_mut().write_message(InboundMessage {
        token: "second".into(),
        msg: ClientMessage::ConsoleVisited {
            console: ConsoleId("detached".into()),
        },
    });
    app.update();
    assert!(
        app.world()
            .resource::<StationImportanceRes>()
            .0
            .flags_of(&detached)
            .unread
    );
    app.world_mut().write_message(InboundMessage {
        token: "first".into(),
        msg: ClientMessage::ConsoleVisited {
            console: ConsoleId("detached".into()),
        },
    });
    app.update();
    let importance = &app.world().resource::<StationImportanceRes>().0;
    assert!(!importance.flags_of(&detached).unread);
    assert!(importance.flags_of(&detached).critical);
    assert!(importance.flags_of(&ordinary).unread);
    app.world_mut().write_message(InboundMessage {
        token: "first".into(),
        msg: ClientMessage::StationVisited {
            station: StationId("home".into()),
        },
    });
    app.update();
    assert!(
        !app.world()
            .resource::<StationImportanceRes>()
            .0
            .flags_of(&ordinary)
            .unread
    );
}

fn hosts() -> ConsoleHosts {
    ConsoleHosts(
        ["ordinary", "detached"]
            .into_iter()
            .map(|id| ConsoleHostSnapshot {
                console: ConsoleId(id.into()),
                host: Some(StationId("home".into())),
                rating: "Std".into(),
            })
            .collect(),
    )
}
