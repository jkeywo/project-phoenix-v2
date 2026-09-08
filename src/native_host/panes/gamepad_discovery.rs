//! Retain the native startup inventory until Bevy has materialized its pads.
//!
//! Gilrs enumerates already-connected devices in PreStartup. The ordinary input
//! consumer runs in PreUpdate; panes may not exist for several more frames. Keep
//! that inventory independently of message-buffer lifetime, and repair only a
//! known connected entity missing its Gamepad component. Hot-plug disconnects
//! remove entries immediately. This never creates a second backend or input path.
use bevy::input::gamepad::{GamepadConnection, GamepadConnectionEvent};
use bevy::input::InputSystems;
use bevy::prelude::*;
use std::collections::BTreeMap;

const MAX_REPLAYS: usize = 3;

#[derive(Resource, Default)]
struct ConnectedInventory(BTreeMap<Entity, (GamepadConnection, usize)>);

pub(super) struct GamepadDiscoveryPlugin;

impl Plugin for GamepadDiscoveryPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<ConnectedInventory>(
            StateClass::Timer,
            "native-controller-preferences-and-assignment",
        )
        .init_resource::<ConnectedInventory>()
        .add_systems(
            Startup,
            remember_connections.run_if(resource_exists::<Messages<GamepadConnectionEvent>>),
        )
        .add_systems(
            PreUpdate,
            (remember_connections, reconcile_connections)
                .chain()
                .after(InputSystems)
                .run_if(resource_exists::<Messages<GamepadConnectionEvent>>),
        );
    }
}

fn remember_connections(
    mut events: MessageReader<GamepadConnectionEvent>,
    mut inventory: ResMut<ConnectedInventory>,
) {
    for event in events.read() {
        match &event.connection {
            GamepadConnection::Connected { .. } => {
                inventory
                    .0
                    .entry(event.gamepad)
                    .or_insert_with(|| (event.connection.clone(), 0));
            }
            GamepadConnection::Disconnected => {
                inventory.0.remove(&event.gamepad);
            }
        }
    }
}

fn reconcile_connections(
    entities: Query<Option<&Gamepad>>,
    mut inventory: ResMut<ConnectedInventory>,
    mut events: MessageWriter<GamepadConnectionEvent>,
) {
    inventory.0.retain(|entity, (connection, attempts)| {
        match entities.get(*entity) {
            Ok(Some(_)) => {}
            Ok(None) if *attempts < MAX_REPLAYS => {
                events.write(GamepadConnectionEvent::new(*entity, connection.clone()));
                *attempts += 1;
            }
            Ok(None) => {}
            Err(_) => return false,
        }
        true
    });
}

#[cfg(test)]
mod tests {
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
}
