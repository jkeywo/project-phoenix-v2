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
#[path = "gamepad_discovery_tests.rs"]
mod tests;
