use bevy::prelude::*;
use crate::client::lobby_state::{LobbyState, LocalPlayerToken};
use crate::client::sim_state::ClientSimState;
use crate::client::lobby_plugin::ClientLobbyPlugin;
use crate::client::captain_plugin::CaptainConsolePlugin;
use crate::client::helm_plugin::HelmConsolePlugin;
use crate::shared::messages::{ClientMessage, ServerMessage};

#[derive(Message, Clone, Debug)]
pub struct InboundServerMessage(pub ServerMessage);

#[derive(Message, Clone, Debug)]
pub struct OutboundClientMessage(pub ClientMessage);

pub struct ClientAppPlugin;

impl Plugin for ClientAppPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LobbyState>()
            .init_resource::<ClientSimState>()
            .init_resource::<LocalPlayerToken>()
            .add_message::<InboundServerMessage>()
            .add_message::<OutboundClientMessage>()
            .add_systems(Update, apply_inbound_messages)
            .add_plugins((ClientLobbyPlugin, CaptainConsolePlugin, HelmConsolePlugin));
    }
}

fn apply_inbound_messages(
    mut reader: MessageReader<InboundServerMessage>,
    mut lobby: ResMut<LobbyState>,
    mut sim: ResMut<ClientSimState>,
) {
    for ev in reader.read() {
        lobby.apply(&ev.0);
        sim.apply(&ev.0);
    }
}
