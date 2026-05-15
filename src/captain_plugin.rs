use bevy::prelude::*;

use crate::lobby::{CurrentPhase, InboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};
use crate::ship_state::ShipState;

pub struct CaptainPlugin;

impl Plugin for CaptainPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (handle_toggle_red_alert, handle_set_view));
    }
}

fn handle_toggle_red_alert(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if matches!(ev.msg, ClientMessage::ToggleRedAlert)
            && sessions.0.console_holder(Console::CaptainChair) == Some(ev.token.as_str())
        {
            ship.toggle_red_alert();
        }
    }
}

fn handle_set_view(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if let ClientMessage::SetView { mode } = ev.msg.clone() {
            // Authorization is per-variant: Camera views are the captain's call,
            // Radar is the helm's call. A request from the wrong console is
            // silently ignored.
            let required = match &mode {
                ViewMode::Camera(_) => Console::CaptainChair,
                ViewMode::Radar => Console::Helm,
                ViewMode::ScienceRadar | ViewMode::SensorsRadar => Console::Sensors,
                ViewMode::SystemChart | ViewMode::NavigationChart => Console::Navigation,
                ViewMode::Comms => Console::Comms,
            };
            if sessions.0.console_holder(required) == Some(ev.token.as_str()) {
                ship.view_mode = mode;
            }
        }
    }
}
