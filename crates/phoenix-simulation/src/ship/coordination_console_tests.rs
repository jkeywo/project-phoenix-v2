use super::*;
use crate::core::messages::{ConsoleId, SystemId};
use crate::ship::components::ConsoleHosts;
use phoenix_model::console::ConsoleHostSnapshot;

fn fixture(source: ControlSource) -> (App, Entity) {
    let config = toml::from_str(
        r#"
[[station]]
id="first"
name="First"
description=""
rank=""
[[station]]
id="second"
name="Second"
description=""
rank=""
[[console]]
id="portable"
name="Portable repair"
screen="gui/repair-console.html"
human_seeking=true
host_order=["first", "second"]
[[system]]
id="repair"
kind="repair"
console="portable"
"#,
    )
    .unwrap();
    let mut app = App::new();
    app.insert_resource(SimTick(0));
    app.insert_resource(Sessions(crate::lobby::session::SessionManager::new()));
    for seat in ["first", "second"] {
        let mut sessions = app.world_mut().resource_mut::<Sessions>();
        sessions.0.register(seat.into(), seat.into()).unwrap();
        sessions.0.set_station(seat, Some(StationId(seat.into())));
    }
    app.add_message::<AiChatterEvent>();
    app.add_message::<DeliveredCoordination>();
    app.add_message::<OrderedCoordinationPopup>();
    app.add_systems(Update, process_coordination_lag);
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(SystemId("repair".into()), source);
    let entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            LocalShip,
            ShipConfigComponent(config),
            ShipSystemControlSources(sources),
            CoordinationQueue(coordination::CoordinationLagQueue::new()),
            ConsoleHosts(vec![ConsoleHostSnapshot {
                console: ConsoleId("portable".into()),
                host: Some(StationId("first".into())),
                rating: "Std".into(),
            }]),
        ))
        .id();
    (app, entity)
}

fn enqueue(app: &mut App, entity: Entity, origin: ControlSource) {
    app.world_mut()
        .get_mut::<CoordinationQueue>(entity)
        .unwrap()
        .0
        .enqueue(QueuedCoordination {
            sender_origin: origin,
            address: CoordinationAddress::Console(ConsoleId("portable".into())),
            payload: CoordinationPayload::Advisory {
                message: "test.advisory".into(),
            },
            presentation: crate::core::messages::CoordinationPresentation::new(
                "test.title",
                "test.body",
            ),
            sender_label: "test.sender".into(),
            due_tick: 1,
        });
}

#[test]
fn delayed_console_popup_follows_current_host_not_enqueue_host() {
    let (mut app, ship) = fixture(ControlSource::Human);
    enqueue(&mut app, ship, ControlSource::Ai);
    app.update();
    assert_eq!(
        app.world()
            .resource::<Messages<OrderedCoordinationPopup>>()
            .len(),
        0
    );
    app.world_mut().get_mut::<ConsoleHosts>(ship).unwrap().0[0].host =
        Some(StationId("second".into()));
    app.world_mut().resource_mut::<SimTick>().0 = 1;
    app.update();
    let popups: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<OrderedCoordinationPopup>>()
        .drain()
        .collect();
    assert_eq!(popups.len(), 1);
    assert_eq!(popups[0].token, "second");
    assert!(
        matches!(&popups[0].message, crate::core::messages::ServerMessage::CoordinationPopup { to_label, .. } if to_label == "Portable repair")
    );
}

#[test]
fn unhosted_detached_console_still_receives_normal_ai_delivery() {
    let (mut app, ship) = fixture(ControlSource::Ai);
    app.world_mut().get_mut::<ConsoleHosts>(ship).unwrap().0[0].host = None;
    enqueue(&mut app, ship, ControlSource::Human);
    app.world_mut().resource_mut::<SimTick>().0 = 1;
    app.update();
    let delivered: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<DeliveredCoordination>>()
        .drain()
        .collect();
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        delivered[0].address,
        CoordinationAddress::Console(ConsoleId("portable".into()))
    );
    assert_eq!(delivered[0].delivery, CoordinationDelivery::Ai);
    assert_eq!(
        app.world()
            .resource::<Messages<OrderedCoordinationPopup>>()
            .len(),
        0
    );
}

#[test]
fn console_human_to_human_coordination_remains_suppressed() {
    let (mut app, ship) = fixture(ControlSource::Human);
    enqueue(&mut app, ship, ControlSource::Human);
    app.world_mut().resource_mut::<SimTick>().0 = 1;
    app.update();
    assert_eq!(
        app.world()
            .resource::<Messages<OrderedCoordinationPopup>>()
            .len(),
        0
    );
    assert_eq!(
        app.world()
            .resource::<Messages<DeliveredCoordination>>()
            .len(),
        0
    );
}

#[test]
fn explicit_station_policy_keeps_home_owned_movable_systems() {
    let (mut app, ship) = fixture(ControlSource::Human);
    app.world_mut()
        .get_mut::<ShipConfigComponent>(ship)
        .unwrap()
        .0
        .consoles[0]
        .home_station = Some(StationId("first".into()));
    let config = app.world().get::<ShipConfigComponent>(ship).unwrap();
    let sources = app.world().get::<ShipSystemControlSources>(ship).unwrap();
    assert_eq!(
        station_delivery_policy(&config.0, sources, &StationId("first".into())).1,
        ControlSource::Human
    );
    assert_eq!(
        station_delivery_policy(&config.0, sources, &StationId("second".into())).1,
        ControlSource::Offline
    );
    assert_eq!(
        address_delivery_policy(
            &config.0,
            sources,
            &CoordinationAddress::Console(ConsoleId("portable".into()))
        )
        .1,
        ControlSource::Human
    );
}

#[test]
fn detached_radar_without_owned_weapons_uses_its_own_ai_policy() {
    let (mut app, ship) = fixture(ControlSource::Ai);
    let radar = crate::ship::system_registry::tactical_radar_system_id();
    app.world_mut()
        .get_mut::<ShipConfigComponent>(ship)
        .unwrap()
        .0
        .systems[0]
        .id = radar.clone();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(radar, ControlSource::Ai);
    let config = app.world().get::<ShipConfigComponent>(ship).unwrap();
    let sources = app.world().get::<ShipSystemControlSources>(ship).unwrap();
    assert_eq!(
        address_delivery_policy(
            &config.0,
            sources,
            &CoordinationAddress::Console(ConsoleId("portable".into()))
        )
        .1,
        ControlSource::Ai
    );
}

#[test]
fn detached_steering_without_thrust_keeps_its_own_ai_policy() {
    let (mut app, ship) = fixture(ControlSource::Ai);
    let steering = crate::ship::system_registry::helm_steering_system_id();
    app.world_mut()
        .get_mut::<ShipConfigComponent>(ship)
        .unwrap()
        .0
        .systems[0]
        .id = steering.clone();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(steering, ControlSource::Ai);
    let config = app.world().get::<ShipConfigComponent>(ship).unwrap();
    let sources = app.world().get::<ShipSystemControlSources>(ship).unwrap();
    assert_eq!(
        address_delivery_policy(
            &config.0,
            sources,
            &CoordinationAddress::Console(ConsoleId("portable".into()))
        )
        .1,
        ControlSource::Ai
    );
}
