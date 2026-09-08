//! Desired native station screens outlive a missing display or a failed view.
//! The physical layout still reconciles hardware normally; this private intent
//! keeps the station reserved and leaves the host an actionable Off/move row.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::core::messages::StationId;
use crate::lobby::{InboundMessage, Sessions};

use super::bridge_display::BridgeLayoutResource;
use super::bridge_profile::MonitorIdentity;
use super::panes::PaneBusResource;

#[derive(Resource, Default)]
pub struct ConsoleAssignments(pub HashMap<StationId, MonitorIdentity>);

/// Runs after the display follower has opened consoles and before its fault
/// reconciler may remove an unusable physical seat.
pub fn remember_console_assignments(
    layout: Option<Res<BridgeLayoutResource>>,
    mut assignments: ResMut<ConsoleAssignments>,
) {
    let Some(layout) = layout else { return };
    for monitor in layout.layout.monitors() {
        for station in layout.layout.stations_on(monitor) {
            if assignments.0.get(station) != Some(monitor) {
                assignments.0.insert(station.clone(), monitor.clone());
            }
        }
    }
}

/// Keep logical reservations even after a view closes. No client can create
/// these entries: only a host layout action or an authored native profile can.
pub fn sync_console_assignments(
    layout: Option<Res<BridgeLayoutResource>>,
    bus: Option<Res<PaneBusResource>>,
    sessions: Option<ResMut<Sessions>>,
    mut assignments: ResMut<ConsoleAssignments>,
    mut inbound: ResMut<Messages<InboundMessage>>,
) {
    let (Some(layout), Some(bus), Some(mut sessions)) = (layout, bus, sessions) else {
        return;
    };
    let mut stale: Vec<_> = assignments
        .0
        .keys()
        .filter(|station| !layout.layout.roster().contains(station))
        .cloned()
        .collect();
    stale.sort_by(|a, b| a.0.cmp(&b.0));
    for station in stale {
        assignments.0.remove(&station);
        bus.0.release_console(&station.0);
    }
    let bound = bus.0.console_assignments();
    sessions.0.set_native_station_assignments(bound.clone());
    // A return to the lobby clears ordinary tenure. The native screen retains
    // its assignment and reclaims it through the ordinary typed handler.
    for (token, station) in bound {
        if sessions
            .0
            .players()
            .iter()
            .any(|p| p.token == token && p.connected)
            && sessions.0.station_for_token(&token) != Some(&station)
            // The initial auto-claim dispatcher runs immediately before this
            // return-to-lobby repair. Both routes can observe the same newly
            // registered session before FixedUpdate handles its first claim.
            && !inbound.iter_current_update_messages().any(|message| {
                message.token == token
                    && matches!(&message.msg,
                        crate::core::messages::ClientMessage::SelectStation { station: claimed }
                            if claimed == &station.0)
            })
        {
            inbound.write(InboundMessage {
                token,
                msg: crate::core::messages::ClientMessage::SelectStation { station: station.0 },
            });
        }
    }
}
