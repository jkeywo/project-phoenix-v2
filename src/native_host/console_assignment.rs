//! Logical native Station assignments, independent of their current surfaces.
//!
//! Host actions, selected-class adoption and display reconciliation share the
//! reservation and claim lifecycle here. The layout law still owns physical
//! placement; PaneBus owns identity and ordinary Sessions own Admission.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::core::messages::{ClientMessage, StationId};
use crate::lobby::session::SessionManager;
use crate::lobby::{InboundMessage, Sessions};

use super::bridge_display::BridgeLayoutResource;
use super::bridge_layout::{BridgeLayout, LayoutAction};
use super::bridge_profile::MonitorIdentity;
use super::host_lobby::LayoutNotice;
use super::panes::{transport::PaneBus, PaneBusResource, PaneId};

/// Desired screens survive monitor loss and exhausted surface recovery.
/// Only assignment transitions can mutate this storage.
#[derive(Resource, Default)]
pub struct ConsoleAssignments {
    desired: HashMap<StationId, MonitorIdentity>,
}

impl ConsoleAssignments {
    pub fn monitor_for(&self, station: &StationId) -> Option<&MonitorIdentity> {
        self.desired.get(station)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&StationId, &MonitorIdentity)> {
        self.desired.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.desired.is_empty()
    }
}

/// Claim waiting must not change ConsoleAssignments' change tick: the lobby
/// watches desired assignments to decide when its screen rows need repainting.
#[derive(Resource, Default)]
pub struct PendingConsoleClaims(Vec<PendingConsoleClaim>);

struct PendingConsoleClaim {
    token: String,
    station: StationId,
    waited: u32,
}

/// About ten seconds at 60 fps, preserving the native initial-claim bound.
const PENDING_CLAIM_MAX_FRAMES: u32 = 600;

impl PendingConsoleClaims {
    fn cancel(&mut self, station: &StationId) {
        self.0.retain(|claim| &claim.station != station);
    }
}

fn publish_reservations(bus: Option<&PaneBus>, sessions: Option<&mut SessionManager>) {
    if let (Some(bus), Some(sessions)) = (bus, sessions) {
        sessions.set_native_station_assignments(bus.console_assignments());
    }
}

/// Apply an operator's layout action and its logical assignment consequences
/// together. Call inline in PreUpdate so Admission sees reservations before the
/// next fixed tick. The bool reports an Off that removed remembered intent,
/// including when the physical screen was already unavailable.
///
/// Non-Station actions still pass through the same pure layout law. Their
/// source-specific guards (such as the GM role freeze) belong to the caller.
pub(crate) fn apply_host_action(
    layout: &BridgeLayout,
    action: &LayoutAction,
    assignments: Option<&mut ConsoleAssignments>,
    claims: Option<&mut PendingConsoleClaims>,
    bus: Option<&PaneBus>,
    sessions: Option<&mut SessionManager>,
) -> Result<(BridgeLayout, bool), Box<LayoutNotice>> {
    if let LayoutAction::AssignStation { station, .. } = action {
        if let Some(holder) = sessions.as_ref().and_then(|sessions| {
            sessions.players().iter().find(|player| {
                player.connected
                    && player.station.as_ref() == Some(station)
                    && sessions.native_station_for_token(&player.token) != Some(station)
            })
        }) {
            return Err(Box::new(LayoutNotice::StationHeld {
                station: station.clone(),
                holder: holder.name.clone(),
            }));
        }
    }
    let next = layout.apply(action).map_err(LayoutNotice::Refused)?;
    let mut released = false;
    match action {
        LayoutAction::AssignStation { station, monitor } => {
            if let Some(assignments) = assignments {
                assignments.desired.insert(station.clone(), monitor.clone());
            }
            if let Some(bus) = bus {
                bus.reserve_console(&station.0);
            }
        }
        LayoutAction::UnassignStation { station } => {
            if let Some(assignments) = assignments {
                released = assignments.desired.remove(station).is_some();
            }
            if let Some(claims) = claims {
                claims.cancel(station);
            }
            if let Some(bus) = bus {
                bus.release_console(&station.0);
            }
        }
        _ => {}
    }
    publish_reservations(bus, sessions);
    Ok((next, released))
}

/// A selected class replaces desired assignments, including absent monitors.
/// Reserve immediately: adoption follows the display pass, but the next fixed
/// tick must already refuse competing claims before its consoles are opened.
pub(crate) fn adopt_assignments(
    roster: &[StationId],
    desired: HashMap<StationId, MonitorIdentity>,
    assignments: Option<&mut ConsoleAssignments>,
    claims: Option<&mut PendingConsoleClaims>,
    bus: Option<&PaneBus>,
    sessions: Option<&mut SessionManager>,
) {
    if let Some(claims) = claims {
        claims
            .0
            .retain(|claim| desired.contains_key(&claim.station));
    }
    if let Some(bus) = bus {
        for (_, station) in bus.console_assignments() {
            if !desired.contains_key(&station) {
                bus.release_console(&station.0);
            }
        }
        for station in roster {
            if desired.contains_key(station) {
                bus.reserve_console(&station.0);
            }
        }
    }
    publish_reservations(bus, sessions);
    if let Some(assignments) = assignments {
        assignments.desired = desired;
    }
}

/// Open only after the display adapter has prepared the Station's surface.
/// Identity creation and the deferred initial claim are one operation.
pub(crate) fn open_console(
    bus: &PaneBus,
    claims: Option<&mut PendingConsoleClaims>,
    station: &StationId,
) -> (PaneId, String) {
    let opened = bus.open_console(&station.0);
    if let (Some(claims), Some(token)) = (claims, bus.token_of(opened.0)) {
        claims.0.push(PendingConsoleClaim {
            token,
            station: station.clone(),
            waited: 0,
        });
    }
    opened
}

/// A physical close cancels its pending claim but keeps the reservation.
/// Host Off and selected-class adoption are the logical release operations.
pub(crate) fn close_console(
    bus: &PaneBus,
    claims: Option<&mut PendingConsoleClaims>,
    station: &StationId,
) -> Option<PaneId> {
    if let Some(claims) = claims {
        claims.cancel(station);
    }
    let pane = bus.open_pane_for_name(&station.0)?;
    bus.close(pane);
    Some(pane)
}

/// Runs after opening consoles and before recovery may surrender physical seats.
/// Compare before mutating so an unchanged bridge does not republish its rows.
pub fn remember_console_assignments(
    layout: Option<Res<BridgeLayoutResource>>,
    mut assignments: ResMut<ConsoleAssignments>,
) {
    let Some(layout) = layout else { return };
    for monitor in layout.layout.monitors() {
        for station in layout.layout.stations_on(monitor) {
            if assignments.monitor_for(station) != Some(monitor) {
                assignments.desired.insert(station.clone(), monitor.clone());
            }
        }
    }
}

fn connected(sessions: &SessionManager, token: &str) -> bool {
    sessions
        .players()
        .iter()
        .any(|p| p.connected && p.token == token)
}

fn emit_claim(inbound: &mut Messages<InboundMessage>, token: &str, station: &StationId) {
    if !inbound.iter_current_update_messages().any(|message| {
        message.token == token
            && matches!(&message.msg, ClientMessage::SelectStation { station: claimed }
                if claimed == &station.0)
    }) {
        inbound.write(InboundMessage {
            token: token.to_owned(),
            msg: ClientMessage::SelectStation {
                station: station.0.clone(),
            },
        });
    }
}

/// Initial claims wait for Identify, with the original bounded frame counter.
/// Unlike return-to-lobby repair, this sends even if tenure already matches.
pub(crate) fn apply_pending_console_claims(
    claims: Option<ResMut<PendingConsoleClaims>>,
    layout: Option<Res<BridgeLayoutResource>>,
    sessions: Option<Res<Sessions>>,
    mut inbound: ResMut<Messages<InboundMessage>>,
) {
    let (Some(mut claims), Some(sessions)) = (claims, sessions) else {
        return;
    };
    if claims.0.is_empty() {
        return;
    }
    claims.0.retain_mut(|claim| {
        // The display follower defers its close sweep during an empty-monitor
        // frame. The selected roster can already have changed, so discard an
        // obsolete claim before the reservation-pruning system runs.
        if layout
            .as_ref()
            .is_some_and(|layout| !layout.layout.roster().contains(&claim.station))
        {
            return false;
        }
        if connected(&sessions.0, &claim.token) {
            emit_claim(&mut inbound, &claim.token, &claim.station);
            return false;
        }
        claim.waited += 1;
        claim.waited < PENDING_CLAIM_MAX_FRAMES
    });
}

/// Prune Stations that the selected hull removed, publish the Admission mirror,
/// and restore tenure after returning to the lobby. Hardware loss alone never
/// releases an assignment. Initial dispatch runs immediately before this system;
/// both policies share one emission rule without producing duplicate commands.
pub fn sync_console_assignments(
    layout: Option<Res<BridgeLayoutResource>>,
    bus: Option<Res<PaneBusResource>>,
    sessions: Option<ResMut<Sessions>>,
    mut assignments: ResMut<ConsoleAssignments>,
    mut claims: Option<ResMut<PendingConsoleClaims>>,
    mut inbound: ResMut<Messages<InboundMessage>>,
) {
    let (Some(layout), Some(bus), Some(mut sessions)) = (layout, bus, sessions) else {
        return;
    };
    let mut stale: Vec<_> = assignments
        .iter()
        .map(|(station, _)| station)
        .filter(|station| !layout.layout.roster().contains(station))
        .cloned()
        .collect();
    stale.sort_by(|a, b| a.0.cmp(&b.0));
    for station in stale {
        assignments.desired.remove(&station);
        if let Some(claims) = claims.as_mut() {
            claims.cancel(&station);
        }
        bus.0.release_console(&station.0);
    }
    let bound = bus.0.console_assignments();
    sessions.0.set_native_station_assignments(bound.clone());
    for (token, station) in bound {
        if connected(&sessions.0, &token) && sessions.0.station_for_token(&token) != Some(&station)
        {
            emit_claim(&mut inbound, &token, &station);
        }
    }
}

#[cfg(test)]
#[path = "console_assignment_tests.rs"]
mod tests;
