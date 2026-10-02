use super::*;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((bevy::input::InputPlugin, GamepadDiscoveryPlugin));
    app
}

fn connection(entity: Entity) -> GamepadConnectionEvent {
    GamepadConnectionEvent::new(
        entity,
        GamepadConnection::Connected {
            name: "Already connected controller".into(),
            vendor_id: Some(123),
            product_id: Some(456),
        },
    )
}

#[test]
fn retained_startup_inventory_repairs_a_connection_message_lost_before_input() {
    let mut app = app();
    let entity = app.world_mut().spawn_empty().id();
    app.world_mut().write_message(connection(entity));
    app.world_mut().run_schedule(Startup);
    app.world_mut()
        .resource_mut::<Messages<GamepadConnectionEvent>>()
        .clear();
    app.update();
    app.update();
    let pad = app.world().get::<Gamepad>(entity).unwrap();
    assert_eq!(pad.vendor_id(), Some(123));
    assert_eq!(pad.product_id(), Some(456));
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(app.world().resource::<ConnectedInventory>().0[&entity].1, 1);
}

#[test]
fn hotplug_disconnect_is_never_resurrected_and_normal_startup_needs_no_replay() {
    let mut app = app();
    let entity = app.world_mut().spawn_empty().id();
    app.world_mut().write_message(connection(entity));
    app.update();
    assert!(app.world().get::<Gamepad>(entity).is_some());
    assert_eq!(app.world().resource::<ConnectedInventory>().0[&entity].1, 0);
    app.world_mut().write_message(GamepadConnectionEvent::new(
        entity,
        GamepadConnection::Disconnected,
    ));
    for _ in 0..5 {
        app.update();
    }
    assert!(app.world().get::<Gamepad>(entity).is_none());
    assert!(!app
        .world()
        .resource::<ConnectedInventory>()
        .0
        .contains_key(&entity));
}
