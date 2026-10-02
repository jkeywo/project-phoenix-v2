//! Fixed-tick Force Start admission for the private native GM surface.
//!
//! The surface queues intent only. This module rechecks its authority at the
//! tick that forwards an accepted request to the existing host start applier.
//! Results use the ordinary attributed activity stream, including refusals.

use bevy::prelude::*;
use serde::Serialize;

use super::{NativeGmLifecycle, NativeGmSurface};
use crate::core::messages::GamePhase;
use crate::gm_action::{NativeGmAuthority, NATIVE_GM_OPERATOR_ID};
use crate::gm_roster::GmRoster;
use crate::lobby::start_policy::{
    evaluate_start_policy, ReadinessTally, StartGrantReason, StartGrantResult, StartGrantStatus,
    StartPolicyDecision, StartPolicyReason, StartTrigger,
};
use crate::lobby::{FleetManagedLobby, Sessions, StartGrantResults};
use crate::server::bridge::PendingForceStart;

/// One coalesced request and one presentation result, retained across surface
/// rebuilds and role changes. Sequence numbers belong to this host lifetime.
#[derive(Resource, Default)]
pub struct NativeGmStartRequests {
    pending: bool,
    sequence: u64,
    last_result: Option<StartGrantResult>,
}

impl NativeGmStartRequests {
    /// Returns false when an identical request is already waiting for a tick.
    pub fn request(&mut self) -> bool {
        !std::mem::replace(&mut self.pending, true)
    }

    pub fn last_result(&self) -> Option<&StartGrantResult> {
        self.last_result.as_ref()
    }

    pub fn record_result(&mut self, result: StartGrantResult) {
        self.last_result = Some(result);
    }
}

/// Private metadata for the shared readiness presentation. Invalid aggregate
/// counts remain explicit rather than displaying an overflowed ready total.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct NativeGmReadinessTotals {
    pub crew: ReadinessTally,
    pub gms: ReadinessTally,
    pub participants: Option<ReadinessTally>,
    pub connected_total: u64,
    pub ready_total: u64,
}

pub fn readiness_totals(crew: ReadinessTally, gms: &GmRoster) -> NativeGmReadinessTotals {
    let gm_tally = gms.readiness_tally();
    NativeGmReadinessTotals {
        crew,
        gms: gm_tally,
        participants: ReadinessTally::try_new(crew.connected, crew.ready)
            .and_then(|crew| crew.checked_add(gm_tally))
            .ok(),
        connected_total: u64::from(crew.connected) + u64::from(gm_tally.connected),
        ready_total: u64::from(crew.ready) + u64::from(gm_tally.ready),
    }
}

pub struct NativeGmStartPlugin;

impl Plugin for NativeGmStartPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<NativeGmStartRequests>(StateClass::Timer, "native-local-gm-workspace")
            .init_resource::<NativeGmStartRequests>()
            .add_systems(
                FixedUpdate,
                admit_force_start
                    .after(crate::native_host::world_load::NativeWorldLoadSet)
                    .before(crate::server::bridge::apply_force_start),
            );
    }
}

/// Exclusive so all checks and the forwarding latch share one world boundary;
/// no surface-state reader can be separated from its recorded outcome by a
/// different Bevy system. The existing applier still owns preload/phase writes.
pub(crate) fn admit_force_start(world: &mut World) {
    let mut requests = world.resource_mut::<NativeGmStartRequests>();
    if !std::mem::take(&mut requests.pending) {
        return;
    }
    let sequence = requests.sequence.checked_add(1);
    if let Some(sequence) = sequence {
        requests.sequence = sequence;
    }
    let (status, reason) = if sequence.is_none() {
        (
            StartGrantStatus::Refused,
            Some(StartGrantReason::InvalidGrant),
        )
    } else {
        admission(world, force_operator_id(world))
    };
    if status == StartGrantStatus::Applied {
        world.resource_mut::<PendingForceStart>().0 = true;
    }
    let result = StartGrantResult {
        tick: world.resource::<crate::sim_tick::SimTick>().0,
        status,
        operator_id: Some(force_operator_id(world).to_owned()),
        reason,
        grant_id: sequence.map(|sequence| format!("start-{sequence}")),
    };
    world.resource_mut::<NativeGmStartRequests>().last_result = Some(result.clone());
    world.resource_mut::<StartGrantResults>().push(result);
}

fn force_operator_id(world: &World) -> &str {
    world
        .get_resource::<crate::lockstep::FleetRoster>()
        .and_then(|roster| roster.gm_operator(roster.local()))
        .unwrap_or(NATIVE_GM_OPERATOR_ID)
}

fn admission(world: &World, operator_id: &str) -> (StartGrantStatus, Option<StartGrantReason>) {
    let refuse = |reason| (StartGrantStatus::Refused, Some(reason));
    let Some(managed) = world.get_resource::<FleetManagedLobby>() else {
        return refuse(StartGrantReason::UnauthorizedGrant);
    };
    // A malformed native/fleet composition must not use the standalone latch
    // to evade the fleet's canonical grant and ordering owner.
    let standalone_gm = world
        .get_resource::<crate::native_host::session_role::NativeSessionRoleState>()
        .is_some_and(|role| {
            role.committed()
                && role.role()
                    == crate::native_host::session_role::NativeSessionRole::StandaloneGameMaster
        });
    if managed.enabled
        || world.contains_resource::<crate::lockstep::FleetLockstep>()
        || world
            .get_resource::<crate::lockstep::FleetRoster>()
            .is_some_and(|roster| !roster.is_solo() || (!standalone_gm && !roster.gms().is_empty()))
    {
        return refuse(StartGrantReason::UnauthorizedGrant);
    }
    if !world
        .get_resource::<NativeGmLifecycle>()
        .is_some_and(|state| state.enabled)
    {
        return refuse(StartGrantReason::UnauthorizedGrant);
    }
    let live = world
        .get_resource::<NativeGmSurface>()
        .is_some_and(|surface| surface.bridge.live() && !surface.bridge.failed());
    let placed = world
        .get_resource::<super::super::bridge_display::BridgeLayoutResource>()
        .is_some_and(|layout| {
            let monitor = if standalone_gm {
                Some(layout.layout.viewscreen())
            } else {
                layout.layout.game_master_monitor()
            };
            monitor.is_some_and(|monitor| {
                layout
                    .monitors
                    .iter()
                    .any(|present| &present.identity == monitor)
            })
        });
    if !live
        || !placed
        || !world
            .get_resource::<NativeGmAuthority>()
            .is_some_and(|authority| authority.connected)
    {
        return refuse(StartGrantReason::GmNotConnected);
    }
    if !world
        .get_resource::<State<GamePhase>>()
        .is_some_and(|phase| phase.get() == &GamePhase::Lobby)
        || world.get_resource::<NextState<GamePhase>>().is_some_and(
            |next| matches!(next, NextState::Pending(phase) if phase != &GamePhase::Lobby),
        )
    {
        return (
            StartGrantStatus::NoOp,
            Some(StartGrantReason::AlreadyStarted),
        );
    }
    let Some(gms) = world.get_resource::<GmRoster>() else {
        return refuse(StartGrantReason::GmNotConnected);
    };
    let crew = world
        .get_resource::<Sessions>()
        .map(|sessions| sessions.0.readiness_tally())
        .unwrap_or_default();
    let valid = (managed.validation_passed || standalone_gm)
        && world.contains_resource::<crate::world::config::WorldConfig>();
    match evaluate_start_policy([crew], gms, valid, StartTrigger::Forced { operator_id }) {
        StartPolicyDecision::Start { .. } => (StartGrantStatus::Applied, None),
        StartPolicyDecision::Wait { reason } | StartPolicyDecision::Refused { reason } => {
            refuse(match reason {
                StartPolicyReason::ValidationFailed => StartGrantReason::ValidationFailed,
                StartPolicyReason::GmNotConnected => StartGrantReason::GmNotConnected,
                StartPolicyReason::InvalidReadiness => StartGrantReason::InvalidGrant,
                StartPolicyReason::NoParticipants | StartPolicyReason::NotReady => {
                    StartGrantReason::ReadinessChanged
                }
            })
        }
    }
}

#[cfg(test)]
#[path = "start_tests.rs"]
mod tests;
