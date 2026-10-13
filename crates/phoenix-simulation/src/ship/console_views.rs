//! Console working views use existing command admission, replay and mesh lanes.
use crate::core::messages::{ClientMessage, SystemControlPayload, SystemId};
use crate::lobby::{InboundMessage, Sessions};
use crate::ship::components::{ConsoleHosts, ConsoleWorkingViews, ShipConfigComponent};
use bevy::prelude::*;

pub const ASSIGN_CONSOLE_VIEW: &str = "assign-console-view";

/// Validate the crew sender before minting a host-owned replicated assignment.
/// The internal assignment has no directly claimable System, so a phone cannot
/// bypass this check by forging its ControlSystem envelope.
pub fn admit_console_view_requests(
    mut inbound: ParamSet<(MessageReader<InboundMessage>, MessageWriter<InboundMessage>)>,
    sessions: Res<Sessions>,
    ships: Query<(&ShipConfigComponent, &ConsoleHosts), With<crate::server_app::LocalShip>>,
) {
    let messages: Vec<_> = inbound
        .p0()
        .read()
        .filter_map(|event| {
            let ClientMessage::SetConsoleView { console, view } = &event.msg else {
                return None;
            };
            Some((event.token.clone(), console.clone(), view.clone()))
        })
        .collect();
    let Ok((config, hosts)) = ships.single() else {
        return;
    };
    for (token, id, view) in messages {
        if !view.is_valid()
            || sessions.0.is_afk(&token)
            || sessions.0.is_spectator(&token)
            || !sessions
                .0
                .players()
                .iter()
                .any(|player| player.token == token && player.connected)
        {
            continue;
        }
        if config.0.console(&id).is_none() {
            continue;
        }
        // Read the same derived placement as machinery admission. In a fleet it
        // reflects frozen crew occupancy, which live Sessions must not replace.
        let host = hosts
            .0
            .iter()
            .find(|assignment| assignment.console == id)
            .and_then(|assignment| assignment.host.as_ref());
        if host.is_none_or(|host| sessions.0.holder_for_station(host) != Some(token.as_str())) {
            continue;
        }
        inbound.p1().write(InboundMessage {
            token: crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
            msg: ClientMessage::ControlSystem {
                target: SystemId(ASSIGN_CONSOLE_VIEW.into()),
                payload: SystemControlPayload::AssignConsoleView { console: id, view },
            },
        });
    }
}

pub fn apply_console_views(
    mut ships: Query<(
        &crate::core::messages::AdmittedCommands,
        &ShipConfigComponent,
        &mut ConsoleWorkingViews,
    )>,
) {
    for (commands, config, mut views) in &mut ships {
        for command in commands.for_target(ASSIGN_CONSOLE_VIEW) {
            let SystemControlPayload::AssignConsoleView { console, view } = &command.payload else {
                continue;
            };
            if config.0.console(console).is_some() && view.is_valid() {
                views.0.insert(console.clone(), view.clone());
            }
        }
    }
}

#[cfg(test)]
#[path = "console_views_tests.rs"]
mod tests;
