use super::*;
use phoenix_model::{
    console::{ConsoleHostSnapshot, ConsoleWorkingView},
    identity::ConsoleId,
};

fn app() -> App {
    let config = toml::from_str(
        r#"
[[station]]
id = "home"
name = "Home"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []
[[station]]
id = "visitor"
name = "Visitor"
description = ""
rank = ""
[[console]]
id = "navigation"
home_station = "home"
screen = "navigation.html"
human_seeking = true
host_order = ["visitor"]
visiting_rating = "Std"
"#,
    )
    .unwrap();
    let mut app = App::new();
    app.add_message::<InboundMessage>();
    let mut sessions = Sessions(crate::lobby::session::SessionManager::new());
    for (token, seat) in [("first", "home"), ("second", "visitor")] {
        sessions.0.register(token.into(), token.into()).unwrap();
        sessions
            .0
            .set_station(token, Some(crate::core::messages::StationId(seat.into())));
    }
    app.insert_resource(sessions);
    app.world_mut().spawn((
        crate::server_app::LocalShip,
        ShipConfigComponent(config),
        ConsoleHosts(vec![ConsoleHostSnapshot {
            console: ConsoleId("navigation".into()),
            host: Some(crate::core::messages::StationId("home".into())),
            rating: "Std".into(),
        }]),
    ));
    app.add_systems(Update, admit_console_view_requests);
    app
}

fn request(app: &mut App, token: &str) -> usize {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .clear();
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg: ClientMessage::SetConsoleView {
            console: ConsoleId("navigation".into()),
            view: ConsoleWorkingView {
                comms_thread: Some("fleet".into()),
                ..Default::default()
            },
        },
    });
    app.update();
    app.world()
        .resource::<Messages<InboundMessage>>()
        .get_cursor()
        .read(app.world().resource::<Messages<InboundMessage>>())
        .filter(|event| matches!(event.msg, ClientMessage::ControlSystem { .. }))
        .count()
}

#[test]
fn console_views_reject_previous_host_after_handoff_and_restore_home_preference() {
    let mut app = app();
    assert_eq!(request(&mut app, "first"), 1);
    assert_eq!(request(&mut app, "second"), 0);
    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .set_afk("first", true);
    assert_eq!(request(&mut app, "first"), 0);
    // Live AFK does not move a Console while the authoritative placement still
    // names home (including a frozen fleet roster).
    assert_eq!(request(&mut app, "second"), 0);
    set_host(&mut app, Some("visitor"));
    assert_eq!(request(&mut app, "second"), 1);
    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .set_afk("first", false);
    assert_eq!(request(&mut app, "second"), 1);
    assert_eq!(request(&mut app, "first"), 0);
    set_host(&mut app, Some("home"));
    assert_eq!(request(&mut app, "second"), 0);
    assert_eq!(request(&mut app, "first"), 1);
    set_host(&mut app, None);
    assert_eq!(request(&mut app, "first"), 0);
}

fn set_host(app: &mut App, station: Option<&str>) {
    let world = app.world_mut();
    let mut query = world.query::<&mut ConsoleHosts>();
    query.single_mut(world).unwrap().0[0].host =
        station.map(|station| crate::core::messages::StationId(station.into()));
}
